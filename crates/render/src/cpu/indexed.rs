//! Native indexed mesh shading within the shared clip/triangle consumer.
//! Q1 r_alias.c R_AliasSetupLighting/R_AliasTransformAndProjectFinalVerts;
//! Q2 ref_soft/r_alias.c R_AliasSetupLighting/R_AliasTransformFinalVerts.
use crate::assets::{Assets, Image, PaletteId, SoftwareAlias, Wrap};
use crate::scene::CpuPresentation;
use crate::shader::{RgbGen, StageBlend};
use crate::stage::{EntityLighting, PreparedStage};
use crate::surface_cache::{IndexedLighting, IndexedMip, PaletteLighting};
use qa_core::primitives::Vec3;

const LIGHT_MIN: i32 = 5;
const COLOR_BITS: u32 = 6;
const GRADES: f32 = (1u32 << COLOR_BITS) as f32;

#[derive(Clone, Copy)]
struct AliasLight {
    ambient: i32,
    shade: f32,
    direction: Vec3,
}
impl AliasLight {
    /// The presentation adapter supplies its native ambient/shade values and
    /// model-local direction. Q2 lightpoint averaging, clamps and effect flags
    /// are not inferred from the model's file format here.
    fn prepare(lighting: EntityLighting, policy: IndexedLighting) -> Option<Self> {
        if !lighting.direction.is_finite()
            || !lighting
                .ambient
                .iter()
                .chain(lighting.directed.iter())
                .all(|v| v.is_finite())
        {
            return None;
        }
        let ambient = policy
            .scalar(lighting.ambient)
            .clamp(LIGHT_MIN as f32, 255.0) as i32;
        let shade = policy.scalar(lighting.directed).max(0.0) * GRADES;
        if !shade.is_finite() {
            return None;
        }
        Some(Self {
            ambient: ((255 - ambient) << COLOR_BITS).max(LIGHT_MIN),
            shade,
            direction: lighting.direction,
        })
    }

    fn vertex(self, normal: Vec3) -> f32 {
        let cosine = normal.dot(self.direction);
        let mut light = self.ambient;
        if cosine < 0.0 {
            light += (self.shade * cosine) as i32;
            light = light.max(0);
        }
        light as f32
    }
}

#[derive(Clone, Copy)]
pub(super) struct Raster<'a> {
    pub palette: &'a PaletteLighting,
    pub palette_id: PaletteId,
    mip: Option<&'a IndexedMip>,
    flat_color: Option<u8>,
    transparent_index: Option<u8>,
    wrap: Wrap,
    lighting: IndexedLighting,
    alias_light: Option<AliasLight>,
    fullbright: bool,
    blend: bool,
    alpha_cutoffs: Option<[f32; 2]>,
    depth_write: bool,
    infrared_map: Option<&'static [u8; 256]>,
}

impl<'a> Raster<'a> {
    pub fn prepare(
        presentation: CpuPresentation,
        image: &'a Image,
        stage: &PreparedStage,
        software_alias: Option<SoftwareAlias>,
        lighting: Option<EntityLighting>,
        assets: &'a Assets,
    ) -> Result<Option<Self>, &'static str> {
        let CpuPresentation::Indexed {
            palette,
            lighting: policy,
            fullbright,
            ..
        } = presentation
        else {
            return Ok(None);
        };
        let palette_resource = assets.palette(palette).ok_or("missing mesh palette")?;
        let flat_color = if let Some((color_palette, color)) = stage.inputs.palette_color {
            if color_palette != palette {
                return Err("flat color palette mismatch");
            }
            Some(color)
        } else {
            None
        };
        let texture = image.indexed.as_ref();
        if flat_color.is_none() && texture.is_none() {
            return Err("mesh image has no native indices");
        }
        let blend = match stage.stage.blend {
            None => false,
            Some(StageBlend::ALPHA)
                if palette_resource.translucency_map().is_some()
                    || software_alias.is_some_and(|alias| alias.alpha_cutoffs.is_some()) =>
            {
                true
            }
            _ => return Err("unsupported indexed mesh blend"),
        };
        let fullbright_light = software_alias.and_then(|alias| alias.fullbright_light);
        let alias_light = if matches!(stage.stage.rgb_gen, RgbGen::LightingDiffuse)
            || fullbright_light.is_some()
        {
            let mut lighting = lighting.ok_or("missing alias lighting")?;
            if let Some([ambient, shade]) = fullbright_light {
                lighting.ambient = [ambient; 3];
                lighting.directed = [shade; 3];
            }
            Some(AliasLight::prepare(lighting, policy).ok_or("nonfinite alias lighting")?)
        } else {
            None
        };
        Ok(Some(Self {
            palette: palette_resource,
            palette_id: palette,
            mip: texture
                .map(|texture| texture.mip(0).ok_or("missing native mesh mip"))
                .transpose()?,
            flat_color,
            transparent_index: texture.and_then(|texture| texture.transparent_index()),
            wrap: image.native_sampler.unwrap_or(stage.stage.sampler).wrap,
            lighting: policy,
            alias_light,
            fullbright,
            blend,
            alpha_cutoffs: software_alias.and_then(|alias| alias.alpha_cutoffs),
            depth_write: stage.stage.depth_write,
            infrared_map: stage
                .inputs
                .infrared_active()
                .then(|| software_alias.and_then(|alias| alias.infrared_map))
                .flatten(),
        }))
    }

    pub fn light(self, normal: Vec3, color: [u8; 4]) -> f32 {
        if self.infrared_map.is_some() {
            return 0.0;
        }
        if self.fullbright {
            return 0.0;
        }
        if let Some(alias) = self.alias_light {
            return alias.vertex(normal);
        }
        let brightness = self.lighting.scalar([color[0], color[1], color[2]]);
        ((255 - i32::from(brightness)) << COLOR_BITS) as f32
    }

    pub fn sample(
        self,
        coordinates: [f32; 2],
        light: f32,
        alpha: f32,
        destination: u8,
        destination_palette: u32,
    ) -> Option<(u8, bool)> {
        let raw = if let Some(color) = self.flat_color {
            color
        } else {
            let mip = self.mip?;
            let x = super::texel(coordinates[0], mip.width, self.wrap);
            let y = super::texel(coordinates[1], mip.height, self.wrap);
            mip.indices()[y * mip.width as usize + x]
        };
        if self.transparent_index == Some(raw) {
            return None;
        }
        let shade = (light as i32).clamp(0, 63 * 256 + 255) as usize & 0xff00;
        let color = if let Some(map) = self.infrared_map {
            self.palette.colormap()[map[raw as usize] as usize]
        } else if self.flat_color.is_some() {
            raw
        } else {
            self.palette.colormap()[shade + raw as usize]
        };
        if !self.blend {
            return Some((color, self.depth_write));
        }
        composite_color(
            self.palette,
            color,
            (destination_palette == self.palette_id.0).then_some(destination),
            alpha,
            self.alpha_cutoffs,
            self.depth_write,
        )
    }
}

/// One native palette blend operation for meshes and completed materials.
/// A missing destination palette rejects only a blend that actually needs it.
pub(super) fn composite_color(
    palette: &PaletteLighting,
    color: u8,
    destination: Option<u8>,
    alpha: f32,
    cutoffs: Option<[f32; 2]>,
    depth_write: bool,
) -> Option<(u8, bool)> {
    let (source_major, depth_write) = if let Some([low, high]) = cutoffs {
        if alpha > high {
            return Some((color, true));
        }
        let source_major = alpha > low;
        (source_major, source_major)
    } else {
        if alpha >= 1.0 {
            return Some((color, depth_write));
        }
        if alpha <= 0.0 {
            return None;
        }
        (alpha > 0.33, depth_write)
    };
    let destination = destination?;
    let map = palette.translucency_map()?;
    let [major, minor] = if source_major {
        [color, destination]
    } else {
        [destination, color]
    };
    Some((
        map[usize::from(major) * 256 + usize::from(minor)],
        depth_write,
    ))
}

#[cfg(test)]
#[path = "../../tests/cpu_indexed/oracle.rs"]
mod tests;
