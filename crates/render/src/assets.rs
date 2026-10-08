//! Registration is a load operation. Frames use numeric handles only.
use crate::scene::Span;
pub use crate::shader::{AlphaFunc as AlphaTest, Cull, TexCoordGen as TcGen};
use crate::shader::{AlphaGen, Deform, FogParms, RgbGen, StageBlend, TexMod};
use crate::surface_cache::{IndexedTexture, PaletteLighting};
use crate::world::{SurfaceBinding, SurfaceMaterial, World, WorldId, geometry::WorldGeometry};
use qa_core::primitives::Vec3;
use qa_world::visibility::VisibilityWorld;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ImageId(pub u32);
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MaterialId(pub u32);
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ModelId(pub u32);
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PaletteId(pub u32);
#[derive(Clone, Copy, Debug, PartialEq)]
#[repr(C)]
pub struct Vertex {
    pub position: Vec3,
    pub normal: Vec3,
    pub texcoord: [f32; 2],
    pub lightmap_coord: [f32; 2],
    pub color: [u8; 4],
}
impl Default for Vertex {
    fn default() -> Self {
        Self {
            position: Vec3::default(),
            normal: Vec3([0.0, 0.0, 1.0]),
            texcoord: [0.0; 2],
            lightmap_coord: [0.0; 2],
            color: [255; 4],
        }
    }
}
pub struct Image {
    pub width: u32,
    pub height: u32,
    pub rgba: Box<[u8]>,
    /// Original disk indices/mips for software presentation. The GL view of
    /// the same image is resolved once using the selected load-time palette.
    pub indexed: Option<IndexedTexture>,
}
/// All frame-time texture choices are numeric. Lightmap is resolved from the
/// surface binding, so a script material is shared across atlas pages/worlds.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum StageTexture {
    Image(ImageId),
    Animation {
        images: [ImageId; 8],
        count: u8,
        frequency: f32,
    },
    Lightmap,
}
impl Default for StageTexture {
    fn default() -> Self {
        Self::Image(ImageId(0))
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Wrap {
    #[default]
    Repeat,
    Clamp,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Filter {
    Nearest,
    #[default]
    Linear,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Sampler {
    pub wrap: Wrap,
    pub filter: Filter,
    pub mipmaps: bool,
}
impl Default for Sampler {
    fn default() -> Self {
        Self {
            wrap: Wrap::Repeat,
            filter: Filter::Linear,
            mipmaps: true,
        }
    }
}
/// Native cross-coordinate liquid displacement, expressed in normalized UVs.
/// The load conversion supplies texture dimensions and original texel scale.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct UvWarp {
    pub texel_scale: [f32; 2],
    pub amplitude: [f32; 2],
    pub frequency: f32,
    pub time_scale: f32,
}
/// Native flowing surfaces use a truncating cycle and an explicit cycle-start
/// value. These parameters encode Q2 flow without a game branch in a backend.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Flow {
    pub speed: f32,
    pub amplitude: [f32; 2],
    pub cycle_start: [f32; 2],
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TcMod {
    Script(TexMod),
    Warp(UvWarp),
    Flow(Flow),
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DepthFunc {
    #[default]
    Lequal,
    Equal,
    Always,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Stage {
    pub texture: StageTexture,
    pub sampler: Sampler,
    pub blend: Option<StageBlend>,
    pub rgb_gen: RgbGen,
    pub alpha_gen: AlphaGen,
    pub alpha_test: AlphaTest,
    pub texgen: TcGen,
    pub tcmods: [Option<TcMod>; 4],
    pub depth_func: DepthFunc,
    pub depth_write: bool,
    pub detail: bool,
}
impl Default for Stage {
    fn default() -> Self {
        Self {
            texture: StageTexture::default(),
            sampler: Sampler::default(),
            blend: None,
            rgb_gen: RgbGen::Identity,
            alpha_gen: AlphaGen::Identity,
            alpha_test: AlphaTest::None,
            texgen: TcGen::Texture,
            tcmods: [None; 4],
            depth_func: DepthFunc::Lequal,
            depth_write: true,
            detail: false,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Sky {
    Layered {
        images: [ImageId; 2],
        sphere: crate::sky::LayeredSphere,
    },
    Cube {
        outer_box: Option<[ImageId; 6]>,
        inner_box: Option<[ImageId; 6]>,
        clouds: crate::sky::CloudSphere,
        rotation: Option<crate::sky::Rotation>,
        params: CubeSkyParams,
    },
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SkyDistance {
    Fixed(f32),
    ViewFar(f32),
}
impl SkyDistance {
    pub fn value(self, far: f32) -> f32 {
        match self {
            Self::Fixed(value) => value,
            Self::ViewFar(scale) => far * scale,
        }
    }
}
/// The source adapters select these values at load. A cube renderer has no
/// map/game-family branch: native Q2 and Q3 differ in distance, UV range and depth.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CubeSkyParams {
    pub distance: SkyDistance,
    pub far_depth: bool,
    pub texcoord_range: [f32; 2],
    pub sampler: Sampler,
    pub snap_bounds: bool,
    /// Native software presentations may draw a cube over the complete clear
    /// background and ignore rotation, independently of GL's clipped cube.
    pub cpu_background: bool,
    pub cpu_rotation: bool,
}
impl Default for CubeSkyParams {
    fn default() -> Self {
        Self {
            distance: SkyDistance::ViewFar(1.0 / 1.75),
            far_depth: true,
            texcoord_range: [0.0, 1.0],
            sampler: Sampler {
                wrap: Wrap::Clamp,
                ..Sampler::default()
            },
            snap_bounds: true,
            cpu_background: false,
            cpu_rotation: true,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MaterialSettings {
    pub cull: Cull,
    pub sort: f32,
    pub polygon_offset: bool,
    pub deforms: [Option<Deform>; 3],
    pub time_offset: f32,
    pub clamp_time: Option<f32>,
    pub sky: Option<Sky>,
    pub fog: Option<FogParms>,
    pub surface_flags: u32,
    pub content_flags: u32,
    pub portal: bool,
}
impl Default for MaterialSettings {
    fn default() -> Self {
        Self {
            cull: Cull::Front,
            sort: 3.0,
            polygon_offset: false,
            deforms: [None; 3],
            time_offset: 0.0,
            clamp_time: None,
            sky: None,
            fog: None,
            surface_flags: 0,
            content_flags: 0,
            portal: false,
        }
    }
}
pub struct Material {
    pub name: String,
    pub stages: Box<[Stage]>,
    pub settings: MaterialSettings,
}
pub struct Model {
    pub vertices: Box<[Vertex]>,
    pub indices: Box<[u32]>,
    pub material: MaterialId,
}
pub struct Assets {
    images: Vec<Image>,
    materials: Vec<Material>,
    models: Vec<Model>,
    worlds: Vec<World>,
    palettes: Vec<PaletteLighting>,
}
impl Default for Assets {
    fn default() -> Self {
        Self::load()
    }
}
impl Assets {
    pub fn load() -> Self {
        Self {
            images: vec![Image {
                width: 1,
                height: 1,
                rgba: vec![255; 4].into_boxed_slice(),
                indexed: None,
            }],
            materials: vec![Material {
                name: "*white".into(),
                stages: vec![Stage {
                    rgb_gen: RgbGen::ExactVertex,
                    alpha_gen: AlphaGen::Vertex,
                    ..Stage::default()
                }]
                .into_boxed_slice(),
                settings: MaterialSettings {
                    cull: Cull::None,
                    sort: 0.0,
                    ..MaterialSettings::default()
                },
            }],
            models: vec![Model {
                vertices: Box::new([]),
                indices: Box::new([]),
                material: MaterialId(0),
            }],
            worlds: Vec::new(),
            palettes: Vec::new(),
        }
    }
    pub fn register_image(
        &mut self,
        width: u32,
        height: u32,
        rgba: &[u8],
    ) -> Result<ImageId, &'static str> {
        if width == 0
            || height == 0
            || width > 8192
            || height > 8192
            || rgba.len() != width as usize * height as usize * 4
        {
            return Err("invalid RGBA image dimensions");
        }
        let id = ImageId(u32::try_from(self.images.len()).map_err(|_| "image table full")?);
        self.images.push(Image {
            width,
            height,
            rgba: rgba.into(),
            indexed: None,
        });
        Ok(id)
    }
    pub fn register_palette(
        &mut self,
        palette: PaletteLighting,
    ) -> Result<PaletteId, &'static str> {
        let id = PaletteId(u32::try_from(self.palettes.len()).map_err(|_| "palette table full")?);
        self.palettes.push(palette);
        Ok(id)
    }
    /// A resource can keep distinct original GL and software images (Q2 sky's
    /// TGA and PCX). IndexedTexture validated its own mip sizes at construction.
    pub fn register_rgba_with_indexed(
        &mut self,
        width: u32,
        height: u32,
        rgba: &[u8],
        indexed: IndexedTexture,
    ) -> Result<ImageId, &'static str> {
        if indexed.mip(0).is_none() {
            return Err("missing indexed base mip");
        }
        let id = self.register_image(width, height, rgba)?;
        self.images[id.0 as usize].indexed = Some(indexed);
        Ok(id)
    }
    pub fn palette(&self, id: PaletteId) -> Option<&PaletteLighting> {
        self.palettes.get(id.0 as usize)
    }
    pub fn register_indexed_image(
        &mut self,
        texture: IndexedTexture,
        palette: PaletteId,
    ) -> Result<ImageId, &'static str> {
        self.register_indexed_image_with_mask_color(texture, palette, None)
    }
    pub fn register_indexed_image_with_mask_color(
        &mut self,
        texture: IndexedTexture,
        palette: PaletteId,
        mask_color: Option<[u8; 3]>,
    ) -> Result<ImageId, &'static str> {
        let palette = self.palette(palette).ok_or("invalid image palette")?;
        let base = texture.mip(0).ok_or("missing image base mip")?;
        let mut rgba = Vec::with_capacity(base.indices().len() * 4);
        for &index in base.indices() {
            let mut color = palette.color(index).to_le_bytes();
            if texture.transparent_index() == Some(index) {
                color[3] = 0;
                if let Some(rgb) = mask_color {
                    color[..3].copy_from_slice(&rgb);
                }
            }
            rgba.extend_from_slice(&color);
        }
        let id = ImageId(u32::try_from(self.images.len()).map_err(|_| "image table full")?);
        self.images.push(Image {
            width: base.width,
            height: base.height,
            rgba: rgba.into_boxed_slice(),
            indexed: Some(texture),
        });
        Ok(id)
    }
    pub fn register_material(
        &mut self,
        name: &str,
        stages: &[Stage],
        settings: MaterialSettings,
    ) -> Result<MaterialId, &'static str> {
        if let Some(index) = self
            .materials
            .iter()
            .position(|m| m.name == name && m.stages.as_ref() == stages && m.settings == settings)
        {
            return Ok(MaterialId(index as u32));
        }
        if (stages.is_empty() && settings.sky.is_none() && settings.fog.is_none())
            || stages.len() > 8
            || !settings_valid(settings, self)
            || stages.iter().any(|s| !stage_valid(*s, self))
        {
            return Err("invalid material stages");
        }
        let id =
            MaterialId(u32::try_from(self.materials.len()).map_err(|_| "material table full")?);
        self.materials.push(Material {
            name: name.into(),
            stages: stages.into(),
            settings,
        });
        Ok(id)
    }
    pub fn register_model(
        &mut self,
        vertices: &[Vertex],
        indices: &[u32],
        material: MaterialId,
    ) -> Result<ModelId, &'static str> {
        if self.material(material).is_none()
            || !indices.len().is_multiple_of(3)
            || indices.iter().any(|&i| i as usize >= vertices.len())
            || vertices.iter().any(|v| {
                v.position
                    .0
                    .iter()
                    .chain(v.normal.0.iter())
                    .chain(v.texcoord.iter())
                    .chain(v.lightmap_coord.iter())
                    .any(|f| !f.is_finite())
            })
        {
            return Err("invalid mesh");
        }
        let id = ModelId(u32::try_from(self.models.len()).map_err(|_| "model table full")?);
        self.models.push(Model {
            vertices: vertices.into(),
            indices: indices.into(),
            material,
        });
        Ok(id)
    }
    pub fn image(&self, id: ImageId) -> Option<&Image> {
        self.images.get(id.0 as usize)
    }
    pub fn material(&self, id: MaterialId) -> Option<&Material> {
        self.materials.get(id.0 as usize)
    }
    pub fn model(&self, id: ModelId) -> Option<&Model> {
        self.models.get(id.0 as usize)
    }
    pub fn images(&self) -> &[Image] {
        &self.images
    }
    pub fn models(&self) -> &[Model] {
        &self.models
    }
    pub fn materials(&self) -> &[Material] {
        &self.materials
    }
    pub fn register_world(
        &mut self,
        geometry: WorldGeometry,
        visibility: VisibilityWorld,
        materials: &[MaterialId],
    ) -> Result<WorldId, &'static str> {
        let bindings: Vec<_> = materials
            .iter()
            .map(|&material| SurfaceMaterial {
                material,
                ..SurfaceMaterial::default()
            })
            .collect();
        self.register_world_with_bindings(geometry, visibility, &bindings)
    }
    pub fn register_world_with_bindings(
        &mut self,
        geometry: WorldGeometry,
        visibility: VisibilityWorld,
        materials: &[SurfaceMaterial],
    ) -> Result<WorldId, &'static str> {
        if geometry.surfaces.len() != visibility.surface_count()
            || materials.len() != geometry.surfaces.len()
            || materials.iter().any(|binding| {
                self.material(binding.material).is_none()
                    || self.image(binding.lightmap).is_none()
                    || binding
                        .texture_scale
                        .iter()
                        .any(|value| !value.is_finite() || *value <= 0.0)
            })
        {
            return Err("invalid world surface bindings");
        }
        let id = WorldId(u32::try_from(self.worlds.len()).map_err(|_| "world table full")?);
        let mut indices = Vec::new();
        let mut bindings = Vec::with_capacity(materials.len());
        for (source, surface) in geometry.surfaces.iter().enumerate() {
            let range = surface.indices.indices();
            if source != surface.source_id as usize
                || range.end > geometry.indices.len()
                || !range.len().is_multiple_of(3)
            {
                return Err("invalid world triangle range");
            }
            let first = u32::try_from(indices.len()).map_err(|_| "world mesh full")?;
            indices.extend_from_slice(&geometry.indices[range]);
            bindings.push(SurfaceBinding {
                material: materials[source].material,
                lightmap: materials[source].lightmap,
                texture_scale: materials[source].texture_scale,
                mesh_indices: Span {
                    first,
                    count: surface.indices.count,
                },
            });
        }
        let vertices: Vec<_> = geometry
            .vertices
            .iter()
            .map(|v| Vertex {
                normal: v.normal,
                ..v.vertex
            })
            .collect();
        let mesh = self.register_model(&vertices, &indices, MaterialId(0))?;
        self.worlds.push(World {
            geometry,
            visibility,
            mesh,
            bindings: bindings.into_boxed_slice(),
        });
        Ok(id)
    }
    pub fn world(&self, id: WorldId) -> Option<&World> {
        self.worlds.get(id.0 as usize)
    }
    pub fn worlds(&self) -> &[World] {
        &self.worlds
    }
}

fn finite(values: &[f32]) -> bool {
    values.iter().all(|value| value.is_finite())
}
fn finite_wave(wave: crate::shader::Waveform) -> bool {
    finite(&[wave.base, wave.amplitude, wave.phase, wave.frequency])
}
fn stage_valid(stage: Stage, assets: &Assets) -> bool {
    let texture = match stage.texture {
        StageTexture::Image(id) => assets.image(id).is_some(),
        StageTexture::Lightmap => true,
        StageTexture::Animation {
            images,
            count,
            frequency,
        } => {
            count > 0
                && count <= 8
                && frequency.is_finite()
                && images[..count.min(8) as usize]
                    .iter()
                    .all(|&id| assets.image(id).is_some())
        }
    };
    texture
        && match stage.rgb_gen {
            RgbGen::Const(v) => finite(&v),
            RgbGen::Wave(wave) => finite_wave(wave),
            _ => true,
        }
        && match stage.alpha_gen {
            AlphaGen::Const(v) | AlphaGen::Portal(v) => v.is_finite(),
            AlphaGen::Wave(wave) => finite_wave(wave),
            _ => true,
        }
        && match stage.texgen {
            TcGen::Vector(v) => v.iter().all(|v| finite(v)),
            TcGen::LayeredSky {
                flatten_z,
                projected_scale,
                texture_size,
                scroll_speed,
            } => {
                finite(&[flatten_z, projected_scale, texture_size, scroll_speed])
                    && texture_size > 0.0
            }
            TcGen::CloudSky { radius, height } => {
                finite(&[radius, height]) && radius > 0.0 && height >= 0.0
            }
            _ => true,
        }
        && stage
            .tcmods
            .iter()
            .flatten()
            .all(|modifier| match modifier {
                TcMod::Warp(warp) => {
                    finite(&warp.texel_scale)
                        && finite(&warp.amplitude)
                        && finite(&[warp.frequency, warp.time_scale])
                }
                TcMod::Flow(flow) => {
                    finite(&[flow.speed]) && finite(&flow.amplitude) && finite(&flow.cycle_start)
                }
                TcMod::Script(modifier) => match *modifier {
                    TexMod::Transform { matrix, translate } => {
                        matrix.iter().all(|v| finite(v)) && finite(&translate)
                    }
                    TexMod::Scale(v) | TexMod::Scroll(v) => finite(&v),
                    TexMod::Rotate(v) => v.is_finite(),
                    TexMod::Stretch(wave) => finite_wave(wave),
                    TexMod::Turbulent {
                        base,
                        amplitude,
                        phase,
                        frequency,
                    } => finite(&[base, amplitude, phase, frequency]),
                    TexMod::EntityTranslate => true,
                },
            })
}
fn settings_valid(settings: MaterialSettings, assets: &Assets) -> bool {
    finite(&[settings.sort, settings.time_offset])
        && settings.clamp_time.is_none_or(f32::is_finite)
        && settings.sky.is_none_or(|sky| match sky {
            Sky::Layered { images, sphere } => {
                images.iter().all(|&id| assets.image(id).is_some())
                    && finite(&[
                        sphere.flatten_z,
                        sphere.projected_scale,
                        sphere.texture_size,
                    ])
                    && sphere.texture_size > 0.0
                    && finite(&sphere.scroll_speeds)
            }
            Sky::Cube {
                outer_box,
                inner_box,
                clouds,
                rotation,
                params,
            } => {
                [outer_box, inner_box]
                    .iter()
                    .flatten()
                    .flatten()
                    .all(|&id| assets.image(id).is_some())
                    && finite(&[clouds.radius, clouds.height])
                    && clouds.radius > 0.0
                    && clouds.height >= 0.0
                    && rotation.is_none_or(|rotation| {
                        finite(&rotation.axis.0) && rotation.degrees_per_second.is_finite()
                    })
                    && match params.distance {
                        SkyDistance::Fixed(value) | SkyDistance::ViewFar(value) => {
                            value.is_finite() && value > 0.0
                        }
                    }
                    && finite(&params.texcoord_range)
                    && params.texcoord_range[0] >= 0.0
                    && params.texcoord_range[1] <= 1.0
                    && params.texcoord_range[0] <= params.texcoord_range[1]
            }
        })
        && settings
            .fog
            .is_none_or(|fog| finite(&fog.color) && fog.depth_opaque.is_finite())
        && settings
            .deforms
            .iter()
            .flatten()
            .all(|deform| match *deform {
                Deform::Wave { spread, wave } => spread.is_finite() && finite_wave(wave),
                Deform::Move { vector, wave } => finite(&vector) && finite_wave(wave),
                Deform::Bulge {
                    width,
                    height,
                    speed,
                } => finite(&[width, height, speed]),
                Deform::Normal {
                    amplitude,
                    frequency,
                } => finite(&[amplitude, frequency]),
                _ => true,
            })
}
