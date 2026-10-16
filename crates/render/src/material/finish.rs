use super::world_load::{NamedShaderLighting, WorldMaterialError};
use crate::{
    Assets,
    assets::{DepthFunc, ImageId, MaterialId, MaterialSettings, Sky, Stage, StageTexture},
    shader::{AlphaFunc, AlphaGen, BlendFactor, RgbGen, StageBlend, TexCoordGen, WaveFunction},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MaterialFinishPolicy {
    pub detail_textures: bool,
    pub vertex_light: bool,
    pub fullbright: bool,
}
impl MaterialFinishPolicy {
    pub fn compile(self, ui_fullscreen: bool) -> ShaderCompilePolicy {
        ShaderCompilePolicy {
            finish: self,
            ui_fullscreen,
        }
    }
}
impl Default for MaterialFinishPolicy {
    fn default() -> Self {
        Self {
            detail_textures: true,
            vertex_light: false,
            fullbright: false,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ShaderCompilePolicy {
    pub finish: MaterialFinishPolicy,
    pub ui_fullscreen: bool,
}
impl ShaderCompilePolicy {
    pub fn collapse_vertex(self) -> bool {
        self.finish.vertex_light && !self.ui_fullscreen
    }

    /// Surface registration normalizes its request before native lookup.
    /// Generic named, fog and remap requests retain their own lighting mode.
    pub fn world_request(
        self,
        source: crate::world::geometry::LightSource,
    ) -> crate::world::geometry::LightSource {
        use crate::world::geometry::LightSource;
        let requested = if self.finish.vertex_light {
            LightSource::Vertex
        } else {
            source
        };
        if self.finish.fullbright {
            LightSource::White
        } else {
            requested
        }
    }
}

#[derive(Clone, Copy)]
pub(super) enum BundleImage {
    Image(ImageId),
    Lightmap,
}
#[derive(Clone, Copy)]
pub(super) struct ShaderSlot {
    pub(super) active: bool,
    pub(super) images: [Option<BundleImage>; crate::shader::MAX_ANIMATIONS],
    pub(super) count: usize,
    pub(super) frequency: f32,
    pub(super) stage: Stage,
    pub(super) rgb: Option<RgbGen>,
    pub(super) rgb_wave: crate::shader::Waveform,
    pub(super) alpha_wave: crate::shader::Waveform,
    pub(super) vectors: [[f32; 3]; 2],
    pub(super) tcgen: bool,
    pub(super) texmods: usize,
    pub(super) source: Option<BlendFactor>,
    pub(super) destination: Option<BlendFactor>,
    pub(super) pending_depth: bool,
    pub(super) depth_explicit: bool,
    pub(super) pending_alpha: AlphaFunc,
    pub(super) pending_func: DepthFunc,
}
impl ShaderSlot {
    pub(super) fn new() -> Self {
        let wave = crate::shader::Waveform {
            function: crate::shader::WaveFunction::Sin,
            base: 0.,
            amplitude: 0.,
            phase: 0.,
            frequency: 0.,
        };
        Self {
            active: true,
            images: [None; crate::shader::MAX_ANIMATIONS],
            count: 0,
            frequency: 0.,
            stage: Stage {
                depth_write: false,
                ..Default::default()
            },
            rgb: None,
            rgb_wave: wave,
            alpha_wave: wave,
            vectors: [[0.; 3]; 2],
            tcgen: false,
            texmods: 0,
            source: None,
            destination: None,
            pending_depth: true,
            depth_explicit: false,
            pending_alpha: AlphaFunc::None,
            pending_func: DepthFunc::Lequal,
        }
    }
    pub(super) fn commit(&mut self) -> Result<(), WorldMaterialError> {
        let rgb = self.rgb.unwrap_or(match self.source {
            None | Some(BlendFactor::One | BlendFactor::SourceAlpha) => RgbGen::IdentityLighting,
            _ => RgbGen::Identity,
        });
        self.stage.rgb_gen = rgb;
        self.rgb = Some(rgb);
        self.stage.blend = match (self.source, self.destination) {
            (None, None) => None,
            (Some(BlendFactor::One), Some(BlendFactor::Zero)) => {
                self.pending_depth = true;
                None
            }
            (Some(source), Some(destination)) => Some(StageBlend {
                source,
                destination,
            }),
            _ => return Err(WorldMaterialError::Shader("incomplete native blend state")),
        };
        self.stage.depth_write = self.pending_depth;
        self.stage.alpha_test = self.pending_alpha;
        self.stage.depth_func = self.pending_func;
        if self.stage.alpha_gen == AlphaGen::Entity
            && matches!(rgb, RgbGen::Identity | RgbGen::LightingDiffuse)
        {
            self.stage.alpha_gen = AlphaGen::Skip
        }
        Ok(())
    }

    fn inactive() -> Self {
        Self {
            active: false,
            ..Self::new()
        }
    }

    pub(super) fn from_stage(stage: Stage) -> Self {
        let mut slot = Self::new();
        slot.stage = stage;
        slot.rgb = Some(stage.rgb_gen);
        slot.tcgen = true;
        slot.texmods = stage
            .tcmods
            .iter()
            .rposition(Option::is_some)
            .map_or(0, |index| index + 1);
        match stage.texture {
            StageTexture::Image(image) => slot.images[0] = Some(BundleImage::Image(image)),
            StageTexture::Lightmap => slot.images[0] = Some(BundleImage::Lightmap),
            StageTexture::Animation {
                images,
                count,
                frequency,
            } => {
                for (target, image) in slot.images.iter_mut().zip(images).take(count as usize) {
                    *target = Some(BundleImage::Image(image));
                }
                slot.count = count as usize;
                slot.frequency = frequency;
            }
        }
        slot
    }

    fn resolve_bundle(&mut self, lighting: NamedShaderLighting) {
        self.stage.texture = if self.count == 0 {
            match self.images[0] {
                Some(BundleImage::Image(id)) => StageTexture::Image(id),
                Some(BundleImage::Lightmap) => StageTexture::Lightmap,
                None => return,
            }
        } else {
            let mut images = [ImageId(0); crate::shader::MAX_ANIMATIONS];
            for (target, source) in images.iter_mut().zip(self.images).take(self.count) {
                *target = match source {
                    Some(BundleImage::Image(id)) => id,
                    Some(BundleImage::Lightmap) => lighting.lightmap,
                    None => ImageId(0),
                };
            }
            StageTexture::Animation {
                images,
                count: self.count as u8,
                frequency: self.frequency,
            }
        };
    }

    fn copy_bundle(&mut self, source: Self) {
        self.images = source.images;
        self.count = source.count;
        self.frequency = source.frequency;
        self.tcgen = source.tcgen;
        self.texmods = source.texmods;
        self.vectors = source.vectors;
        self.stage.texture = source.stage.texture;
        self.stage.sampler = source.stage.sampler;
        self.stage.texgen = source.stage.texgen;
        self.stage.tcmods = source.stage.tcmods;
        self.stage.is_lightmap = source.stage.is_lightmap;
    }
}

fn vertex_lighting_collapse(slots: &mut [ShaderSlot], sort: f32, lighting: NamedShaderLighting) {
    if sort == 3.0 {
        let mut best = 0;
        let mut best_rank = i32::MIN;
        for (index, slot) in slots.iter().enumerate().take_while(|(_, slot)| slot.active) {
            let mut rank = 0;
            if slot.stage.is_lightmap {
                rank -= 100;
            }
            if !slot.tcgen || slot.stage.texgen != TexCoordGen::Texture {
                rank -= 5;
            }
            if slot.texmods != 0 {
                rank -= 5;
            }
            if !matches!(
                slot.rgb.unwrap_or(RgbGen::IdentityLighting),
                RgbGen::Identity | RgbGen::IdentityLighting
            ) {
                rank -= 3;
            }
            if rank > best_rank {
                best = index;
                best_rank = rank;
            }
        }
        let bundle = slots[best];
        slots[0].copy_bundle(bundle);
        slots[0].stage.blend = None;
        slots[0].stage.depth_write = true;
        slots[0].stage.rgb_gen = if lighting.source == crate::world::geometry::LightSource::None {
            RgbGen::LightingDiffuse
        } else {
            RgbGen::ExactVertex
        };
        slots[0].stage.alpha_gen = AlphaGen::Skip;
    } else {
        if slots[0].stage.is_lightmap {
            slots[0] = slots[1];
        }
        let rgb = [slots[0].stage.rgb_gen, slots[1].stage.rgb_gen];
        let crossfade = rgb.contains(&RgbGen::OneMinusEntity)
            || matches!(rgb,
                [RgbGen::Wave(a), RgbGen::Wave(b)]
                if matches!((a.function, b.function),
                    (WaveFunction::Sawtooth, WaveFunction::InverseSawtooth)
                    | (WaveFunction::InverseSawtooth, WaveFunction::Sawtooth)));
        if crossfade {
            slots[0].stage.rgb_gen = RgbGen::IdentityLighting;
        }
    }
    for slot in slots.iter_mut().skip(1) {
        if !slot.active {
            break;
        }
        *slot = ShaderSlot::inactive();
    }
}

pub(super) fn finish_material(
    name: &str,
    mut slots: Vec<ShaderSlot>,
    mut settings: MaterialSettings,
    lighting: NamedShaderLighting,
    policy: ShaderCompilePolicy,
    assets: &mut Assets,
) -> Result<MaterialId, WorldMaterialError> {
    if slots.len() > crate::shader::MAX_STAGES {
        return Err(WorldMaterialError::Shader("native shader stage limit"));
    }
    for slot in &mut slots {
        slot.resolve_bundle(lighting);
    }
    slots.resize_with(crate::shader::MAX_STAGES, ShaderSlot::inactive);
    if settings.sky.is_some() {
        settings.sort = 2.0;
    }
    if settings.polygon_offset && settings.sort == 0.0 {
        settings.sort = 4.0;
    }
    let mut has_lightmap = false;
    let mut scan = 0;
    while scan < slots.len() {
        if !slots[scan].active {
            break;
        }
        if slots[scan].images[0].is_none() {
            slots[scan].active = false;
            scan += 1;
            continue;
        }
        if slots[scan].stage.detail && !policy.finish.detail_textures {
            if scan + 1 < slots.len() {
                slots.copy_within(scan + 1.., scan);
                slots[scan + 1] = ShaderSlot::inactive();
            }
            scan += 1;
            continue;
        }
        let first_blended = slots[0].stage.blend.is_some();
        let slot = &mut slots[scan];
        if !slot.tcgen {
            slot.stage.texgen = if slot.stage.is_lightmap {
                TexCoordGen::Lightmap
            } else {
                TexCoordGen::Texture
            };
            slot.tcgen = true;
        }
        has_lightmap |= slot.stage.is_lightmap;
        if first_blended && slot.stage.blend.is_some() && settings.sort == 0.0 {
            settings.sort = if slot.stage.depth_write { 5.0 } else { 9.0 };
        }
        if slot.rgb.is_none() {
            slot.stage.rgb_gen = RgbGen::IdentityLighting;
        }
        if let Some(Sky::Cube { clouds, .. }) = settings.sky
            && slot.stage.texgen == TexCoordGen::Texture
        {
            slot.stage.texgen = TexCoordGen::CloudSky {
                radius: clouds.radius,
                height: clouds.height,
            };
        }
        scan += 1;
    }
    if settings.sort == 0.0 {
        settings.sort = 3.0;
    }
    if scan > 1 && policy.collapse_vertex() {
        vertex_lighting_collapse(&mut slots, settings.sort, lighting);
        scan = 1;
        has_lightmap = false;
    }
    if scan == 0 {
        settings.sort = 7.0;
    }
    let stages = slots
        .iter()
        .take(scan)
        .take_while(|slot| slot.active)
        .map(|slot| slot.stage)
        .collect::<Vec<_>>();
    assets
        .register_native_material(
            name,
            crate::assets::NativeMaterialFinish {
                stages: &stages,
                settings,
                has_lightmap,
            },
        )
        .map_err(WorldMaterialError::Boundary)
}
