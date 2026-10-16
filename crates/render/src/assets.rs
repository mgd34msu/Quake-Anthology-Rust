//! Registration is a load operation. Frames use numeric handles only.
mod admission;
mod animation;
mod retessellation;
pub mod upload;
pub(crate) use admission::AssetAdmission;
pub use admission::AssetCounts;
pub use animation::{FrameTiming, ModelAnimation, TimedGroup};
pub use retessellation::WorldRetessellation;

use crate::scene::Span;
pub use crate::shader::{AlphaFunc as AlphaTest, Cull, TexCoordGen as TcGen};
use crate::shader::{AlphaGen, Deform, FogParms, RgbGen, StageBlend, TexMod};
use crate::surface_cache::{IndexedTexture, PaletteLighting};
use crate::world::{
    PreparedShader, SurfaceBinding, SurfaceMaterial, World, WorldId, WorldPresentation,
    WorldPresentationId, WorldPresentationInput,
    geometry::{LightSource, WorldGeometry},
};
use qa_core::names::{NameTable, NamesError};
pub use qa_core::primitives::ModelId;
use qa_core::primitives::{NameId, Vec3};
use qa_world::visibility::VisibilityWorld;
use std::sync::Arc;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ImageId(pub u32);
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MaterialId(pub u32);
/// Native lookup identity in this Assets name arena, paired with its variant.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ShaderRegistration<S = ShaderBindingId> {
    pub key: NameId,
    pub requested: LightSource,
    pub finished: LightSource,
    pub shader: S,
    pub defaulted: Option<NativeDefaultKind>,
    pub sun: Option<crate::shader::SunParms>,
}
impl<S> ShaderRegistration<S> {
    pub fn map_shader<T>(self, map: impl FnOnce(S) -> T) -> ShaderRegistration<T> {
        ShaderRegistration {
            key: self.key,
            requested: self.requested,
            finished: self.finished,
            shader: map(self.shader),
            defaulted: self.defaulted,
            sun: self.sun,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NativeDefaultKind {
    MissingImage,
    Explicit(ExplicitDefault),
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExplicitDefaultReason {
    Parse,
    ImageMissing,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ExplicitDefault {
    material: MaterialId,
    reason: ExplicitDefaultReason,
}
impl ExplicitDefault {
    pub fn reason(self) -> ExplicitDefaultReason {
        self.reason
    }
}
pub(crate) struct NativeMaterialFinish<'a> {
    pub stages: &'a [Stage],
    pub settings: MaterialSettings,
    pub has_lightmap: bool,
}
/// A shader variant binds compiled stages to its selected lighting resource.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ShaderBindingId(pub u32);
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ShaderBinding {
    pub material: MaterialId,
    pub light_source: LightSource,
    pub lightmap: ImageId,
    pub lightmap_region: Option<crate::lightmap::AtlasRegion>,
    pub lightmap_conversion: Option<crate::lightmap::LightmapConversion>,
    /// Styled samples remain in the referenced world's existing geometry.
    pub styled_surface: Option<(WorldPresentationId, u32)>,
}
impl ShaderBinding {
    pub fn material(material: MaterialId, light_source: LightSource) -> Self {
        Self {
            material,
            light_source,
            lightmap: ImageId(0),
            lightmap_region: None,
            lightmap_conversion: None,
            styled_surface: None,
        }
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PaletteId(pub u32);

pub struct PreparedPaletteTranslucency {
    id: PaletteId,
    table: crate::surface_cache::PreparedTranslucency,
}
#[derive(Clone, Copy, Debug, PartialEq)]
#[repr(C)]
pub struct Vertex {
    pub position: Vec3,
    pub normal: Vec3,
    pub texcoord: [f32; 2],
    pub lightmap_coord: [f32; 2],
    pub color: [u8; 4],
}
impl Vertex {
    pub fn is_finite(self) -> bool {
        self.position.is_finite()
            && self.normal.is_finite()
            && self
                .texcoord
                .iter()
                .chain(self.lightmap_coord.iter())
                .all(|value| value.is_finite())
    }
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
    /// Cold native upload levels. Both consumers may use these numeric mip
    /// resources; the original indexed mips and unprocessed RGBA stay intact.
    pub prepared: Option<upload::PreparedImage>,
    /// Numeric ownership revision, advanced on every successful preparation
    /// replacement. It does not depend on the image's bytes or identity.
    pub preparation_revision: u64,
    /// Native first image registration owns wrap/mip flags. Other registrations
    /// leave this unset and use the material stage's explicit sampler.
    pub native_sampler: Option<Sampler>,
}
impl Image {
    /// Software/native dimensions remain independent of a GL replacement.
    pub fn logical_size(&self) -> [u32; 2] {
        self.indexed
            .as_ref()
            .and_then(|texture| texture.mip(0))
            .map_or([self.width, self.height], |mip| [mip.width, mip.height])
    }
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
/// Native unlit/translucent stages can compensate for an image's cold upload
/// intensity. Keep this float operation after the shared byte-color generator.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TextureIntensity {
    #[default]
    Preserve,
    NeutralizeUpload,
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
    pub texture_intensity: TextureIntensity,
    pub blend: Option<StageBlend>,
    pub rgb_gen: RgbGen,
    pub alpha_gen: AlphaGen,
    pub alpha_test: AlphaTest,
    pub texgen: TcGen,
    pub tcmods: [Option<TcMod>; 4],
    pub depth_func: DepthFunc,
    pub depth_write: bool,
    pub detail: bool,
    pub is_lightmap: bool,
}
impl Default for Stage {
    fn default() -> Self {
        Self {
            texture: StageTexture::default(),
            sampler: Sampler::default(),
            texture_intensity: TextureIntensity::Preserve,
            blend: None,
            rgb_gen: RgbGen::Identity,
            alpha_gen: AlphaGen::Identity,
            alpha_test: AlphaTest::None,
            texgen: TcGen::Texture,
            tcmods: [None; 4],
            depth_func: DepthFunc::Lequal,
            depth_write: true,
            detail: false,
            is_lightmap: false,
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
pub struct SoftwareAlias {
    /// Native software skips an absent authored alias skin; GL retains its
    /// skin-zero or missing-image fallback on the same material.
    pub missing_skin: bool,
    /// Native infrared palette-index substitution, selected by presentation.
    pub infrared_map: Option<&'static [u8; 256]>,
    /// Native fullbright aliases retain their software ambient/shade clamps.
    pub fullbright_light: Option<[f32; 2]>,
    /// Low/high alpha cutoffs select the native one-third/two-thirds map and
    /// depth-write behavior, independently of the GL stage's continuous blend.
    pub alpha_cutoffs: Option<[f32; 2]>,
}
/// Raw indexed tile sampling is independent of the material's GL texmods.
/// Native adapters choose timing precision and flow coefficients at load.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SoftwareTiled {
    pub turbulent: bool,
    pub flow_speed: Option<f64>,
    pub time_f32: bool,
}
impl SoftwareTiled {
    pub(crate) const SIZE: u32 = 64;
    pub(crate) const MINIMA: [i32; 2] = [-8192; 2];
    pub(crate) const EXTENTS: [u32; 2] = [16384; 2];
    pub(crate) const COORDINATE_MASK: i64 = (128 << 16) - 1;
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
    pub software_alias: Option<SoftwareAlias>,
    pub software_tiled: Option<SoftwareTiled>,
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
            software_alias: None,
            software_tiled: None,
        }
    }
}
pub struct Material {
    pub name: NameId,
    pub stages: Box<[Stage]>,
    pub settings: MaterialSettings,
    finish_has_lightmap: bool,
}
impl Material {
    pub fn finish_has_lightmap(&self) -> bool {
        self.finish_has_lightmap
    }
}

pub struct Model {
    pub vertices: Box<[Vertex]>,
    pub indices: Box<[u32]>,
    pub shader: ShaderBindingId,
    pub vertices_per_frame: usize,
    frame_count: u32,
    pub sprite: Option<SpriteGeometry>,
    alias: Option<AliasFrames>,
    animation: Option<Arc<ModelAnimation>>,
}
#[derive(Clone, Copy, Debug)]
pub struct AliasFrame {
    pub scale: Vec3,
    pub translation: Vec3,
}
struct AliasFrames {
    frames: Box<[AliasFrame]>,
    positions: Box<[[u8; 3]]>,
}
#[derive(Clone, Copy)]
struct AliasBlend {
    translation: Vec3,
    scale: [Vec3; 2],
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SpriteGeometry {
    /// Authored left, right, top and bottom offsets from the entity origin.
    pub extents: [f32; 4],
}

/// A resolved frame pair. Both backends validate the entity once, before
/// looking up vertices in the same immutable frame-major model table.
#[derive(Clone, Copy)]
pub struct ModelFrames<'a> {
    model: &'a Model,
    current: Span,
    previous: Span,
    weights: [f32; 2],
    alias: Option<AliasBlend>,
    motion: Vec3,
}
impl Model {
    pub fn frame_count(&self) -> u32 {
        self.frame_count
    }

    pub fn native_frame_count(&self) -> u32 {
        self.animation
            .as_ref()
            .map_or(self.frame_count, |animation| {
                animation.frame_groups().len() as u32
            })
    }

    pub fn native_skin_count(&self) -> u32 {
        self.animation
            .as_ref()
            .map_or(0, |animation| animation.skin_groups().len() as u32)
    }

    pub fn animation(&self) -> Option<&ModelAnimation> {
        self.animation.as_deref()
    }

    pub fn frames(&self, frame: u32, old_frame: u32, back_lerp: f32) -> Option<ModelFrames<'_>> {
        self.frames_at(frame, old_frame, back_lerp, Vec3::default())
    }

    pub fn posed_frames(&self, entity: &crate::SceneEntity) -> Option<ModelFrames<'_>> {
        let motion = entity.previous_origin.map_or(Vec3::default(), |previous| {
            qa_core::math::inverse_transform_point(entity.origin, entity.axes, previous)
        });
        if !motion.is_finite() {
            return None;
        }
        self.frames_at(entity.frame, entity.old_frame, entity.back_lerp, motion)
    }

    fn frames_at(
        &self,
        frame: u32,
        old_frame: u32,
        back_lerp: f32,
        motion: Vec3,
    ) -> Option<ModelFrames<'_>> {
        if frame >= self.frame_count
            || old_frame >= self.frame_count
            || !back_lerp.is_finite()
            || !(0.0..=1.0).contains(&back_lerp)
        {
            return None;
        }
        let back_lerp = if frame == old_frame && self.alias.is_none() && motion == Vec3::default() {
            0.0
        } else {
            back_lerp
        };
        let weights = [back_lerp, 1.0 - back_lerp];
        let alias = self.alias.as_ref().map(|alias| {
            let current = alias.frames[frame as usize];
            let previous = alias.frames[old_frame as usize];
            AliasBlend {
                translation: (previous.translation + motion) * weights[0]
                    + current.translation * weights[1],
                scale: [previous.scale * weights[0], current.scale * weights[1]],
            }
        });
        let count = self.vertices_per_frame as u32;
        Some(ModelFrames {
            model: self,
            current: Span {
                first: frame * count,
                count,
            },
            previous: Span {
                first: old_frame * count,
                count,
            },
            weights,
            alias,
            motion,
        })
    }
}
impl ModelFrames<'_> {
    pub fn len(self) -> usize {
        self.current.count as usize
    }

    pub fn is_empty(self) -> bool {
        self.current.count == 0
    }

    /// One pose path before material deforms: native packed alias arithmetic
    /// or decoded frame blending. UVs/colors stay with the current frame.
    pub fn vertex(self, index: usize) -> Vertex {
        let current = self.current_vertex(index);
        if let (Some(blend), Some(alias)) = (self.alias, self.model.alias.as_ref()) {
            let new = alias.positions[self.current.first as usize + index];
            let old = alias.positions[self.previous.first as usize + index];
            return Vertex {
                // GL_LerpVerts and R_AliasLerpFrames: translation, old packed
                // contribution, then new packed contribution. Keep the order.
                position: Vec3(std::array::from_fn(|axis| {
                    (blend.translation.0[axis] + f32::from(old[axis]) * blend.scale[0].0[axis])
                        + f32::from(new[axis]) * blend.scale[1].0[axis]
                })),
                ..current
            };
        }
        if self.weights[0] == 0.0 {
            return current;
        }
        let previous = self.model.vertices[self.previous.range()][index];
        let position = if self.motion == Vec3::default() {
            previous.position
        } else {
            previous.position + self.motion
        };
        Vertex {
            position: position * self.weights[0] + current.position * self.weights[1],
            normal: qa_core::math::normalized_fast(
                previous.normal * self.weights[0] + current.normal * self.weights[1],
            ),
            ..current
        }
    }

    /// Native alias software lighting selects the new frame's normal even
    /// when its position is blended with the old frame.
    pub(crate) fn current_vertex(self, index: usize) -> Vertex {
        self.model.vertices[self.current.range()][index]
    }

    pub(crate) fn offsets(self) -> [usize; 2] {
        [self.current.first as usize, self.previous.first as usize]
    }

    pub(crate) fn weights(self) -> [f32; 2] {
        self.weights
    }
    pub(crate) fn resolved_vertices(self) -> bool {
        self.alias.is_some() || self.motion != Vec3::default()
    }
}
pub struct Assets {
    names: NameTable,
    image_search_order: crate::material::resources::ImageSearchOrder,
    images: Vec<Image>,
    materials: Vec<Material>,
    shader_bindings: Vec<ShaderBinding>,
    shader_next: Vec<Option<u32>>,
    material_shader_heads: Vec<Option<u32>>,
    shader_capacity: usize,
    models: Vec<Model>,
    /// Cold in-place mesh replacement invalidates both backend preparations.
    model_preparation_revision: u64,
    worlds: Vec<World>,
    presentations: Vec<WorldPresentation>,
    palettes: Vec<PaletteLighting>,
}
impl Assets {
    pub fn load() -> Result<Self, NamesError> {
        Self::load_with_shader_capacity(65_536)
    }
    pub fn load_with_shader_capacity(shader_capacity: usize) -> Result<Self, NamesError> {
        if shader_capacity == 0 || shader_capacity > u32::MAX as usize {
            return Err(NamesError::Capacity);
        }
        let mut shader_bindings = Vec::new();
        shader_bindings
            .try_reserve_exact(shader_capacity)
            .map_err(|_| NamesError::Capacity)?;
        shader_bindings.push(ShaderBinding::material(
            MaterialId(0),
            LightSource::TwoDimensional,
        ));
        let mut shader_next = Vec::new();
        shader_next
            .try_reserve_exact(shader_capacity)
            .map_err(|_| NamesError::Capacity)?;
        shader_next.push(None);
        let names = NameTable::load_reserved([b"*white".as_slice()], 65536, 4 * 1024 * 1024)?;
        let white = names.find_path("*white").ok_or(NamesError::Capacity)?;
        Ok(Self {
            names,
            image_search_order: crate::material::resources::ImageSearchOrder::default(),
            images: vec![Image {
                width: 1,
                height: 1,
                rgba: vec![255; 4].into_boxed_slice(),
                indexed: None,
                prepared: None,
                preparation_revision: 0,
                native_sampler: None,
            }],
            materials: vec![Material {
                name: white,
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
                finish_has_lightmap: false,
            }],
            shader_bindings,
            shader_next,
            material_shader_heads: vec![Some(0)],
            shader_capacity,
            models: vec![Model {
                vertices: Box::new([]),
                indices: Box::new([]),
                shader: ShaderBindingId(0),
                vertices_per_frame: 0,
                frame_count: 1,
                sprite: None,
                alias: None,
                animation: None,
            }],
            model_preparation_revision: 0,
            worlds: Vec::new(),
            presentations: Vec::new(),
            palettes: Vec::new(),
        })
    }
    pub fn intern_name(&mut self, path: &str) -> Result<NameId, &'static str> {
        self.names
            .intern_path(path)
            .map_err(|_| "asset name table full")
    }
    /// Live search policy affects cold misses without replacing existing images.
    pub fn set_image_search_order(&mut self, order: crate::material::resources::ImageSearchOrder) {
        self.image_search_order = order;
    }
    pub(crate) fn image_search_order(&self) -> crate::material::resources::ImageSearchOrder {
        self.image_search_order
    }
    /// Lump names are byte identities, not paths. Their decoder owns native
    /// folded lookup; slash conversion must not merge distinct lump names.
    pub fn intern_resource_name(&mut self, name: &[u8]) -> Result<NameId, &'static str> {
        self.names.intern(name).map_err(|_| "asset name table full")
    }
    pub(crate) fn resource_name(&self, id: NameId) -> Option<&[u8]> {
        self.names.get(id)
    }
    pub fn name(&self, id: NameId) -> Option<&str> {
        std::str::from_utf8(self.names.get(id)?).ok()
    }
    pub fn find_name(&self, path: &str) -> Option<NameId> {
        self.names.find_path(path)
    }
    pub fn find_shader_name(&self, raw: &[u8]) -> Option<NameId> {
        self.names.find_folded(crate::shader::registration_key(raw))
    }
    pub fn register_image(
        &mut self,
        width: u32,
        height: u32,
        rgba: &[u8],
    ) -> Result<ImageId, &'static str> {
        if upload::validate_dimensions(width, height).is_err()
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
            prepared: None,
            preparation_revision: 0,
            native_sampler: None,
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
    pub fn ensure_palette_translucency(&mut self, id: PaletteId) -> Result<(), &'static str> {
        if let Some(prepared) = self.prepare_palette_translucency(id)? {
            self.install_palette_translucency(prepared);
        }
        Ok(())
    }
    pub fn prepare_palette_translucency(
        &self,
        id: PaletteId,
    ) -> Result<Option<PreparedPaletteTranslucency>, &'static str> {
        Ok(self
            .palettes
            .get(id.0 as usize)
            .ok_or("unknown translucency palette")?
            .prepare_translucency()
            .map(|table| PreparedPaletteTranslucency { id, table }))
    }
    /// No palette may be removed/replaced between preparation and commit.
    pub fn install_palette_translucency(&mut self, prepared: PreparedPaletteTranslucency) {
        self.palettes[prepared.id.0 as usize].install_translucency(prepared.table);
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
            prepared: None,
            preparation_revision: 0,
            native_sampler: None,
        });
        Ok(id)
    }
    pub fn prepare_image(
        &mut self,
        id: ImageId,
        params: upload::UploadParams,
    ) -> Result<(), &'static str> {
        self.install_prepared(id, None, params)
    }
    /// Prepare a separate native GL expansion (palette correction or alpha
    /// fringe repair), retaining the raw RGBA and original software indices.
    /// A failed preparation leaves the previous prepared levels intact.
    pub fn prepare_image_with_rgba(
        &mut self,
        id: ImageId,
        rgba: &[u8],
        params: upload::UploadParams,
    ) -> Result<(), &'static str> {
        self.install_prepared(id, Some(rgba), params)
    }
    fn install_prepared(
        &mut self,
        id: ImageId,
        rgba: Option<&[u8]>,
        params: upload::UploadParams,
    ) -> Result<(), &'static str> {
        let image = self
            .images
            .get_mut(id.0 as usize)
            .ok_or("invalid image handle")?;
        let prepared = upload::prepare_rgba(
            image.width,
            image.height,
            rgba.unwrap_or(&image.rgba),
            params,
        )
        .map_err(|_| "invalid image preparation")?;
        let revision = image
            .preparation_revision
            .checked_add(1)
            .ok_or("image preparation revision exhausted")?;
        image.prepared = Some(prepared);
        image.preparation_revision = revision;
        Ok(())
    }
    pub fn set_image_native_sampler(
        &mut self,
        id: ImageId,
        sampler: Sampler,
    ) -> Result<(), &'static str> {
        let image = self
            .images
            .get_mut(id.0 as usize)
            .ok_or("invalid image handle")?;
        image.native_sampler = Some(sampler);
        Ok(())
    }
    pub fn register_material(
        &mut self,
        name: &str,
        stages: &[Stage],
        settings: MaterialSettings,
    ) -> Result<MaterialId, &'static str> {
        self.intern_material(name, stages, settings, None)
    }
    pub(crate) fn register_native_material(
        &mut self,
        name: &str,
        finish: NativeMaterialFinish<'_>,
    ) -> Result<MaterialId, &'static str> {
        self.intern_material(
            name,
            finish.stages,
            finish.settings,
            Some(finish.has_lightmap),
        )
    }
    pub(crate) fn explicit_default(
        &self,
        material: MaterialId,
        reason: ExplicitDefaultReason,
    ) -> Result<NativeDefaultKind, &'static str> {
        self.material(material)
            .ok_or("missing explicit default body")?;
        Ok(NativeDefaultKind::Explicit(ExplicitDefault {
            material,
            reason,
        }))
    }
    fn intern_material(
        &mut self,
        name: &str,
        stages: &[Stage],
        settings: MaterialSettings,
        native_finish: Option<bool>,
    ) -> Result<MaterialId, &'static str> {
        let name = self.intern_name(name)?;
        if (stages.is_empty()
            && settings.sky.is_none()
            && settings.fog.is_none()
            && native_finish.is_none())
            || stages.len() > 8
            || !settings_valid(settings, self)
            || stages.iter().any(|s| !stage_valid(*s, self))
        {
            return Err("invalid material stages");
        }
        let finish_has_lightmap =
            native_finish.unwrap_or_else(|| stages.iter().any(|stage| stage.is_lightmap));
        if let Some(index) = self.materials.iter().position(|m| {
            m.name == name
                && m.stages.as_ref() == stages
                && m.settings == settings
                && m.finish_has_lightmap == finish_has_lightmap
        }) {
            return Ok(MaterialId(index as u32));
        }
        let id =
            MaterialId(u32::try_from(self.materials.len()).map_err(|_| "material table full")?);
        self.material_shader_heads.push(None);
        self.materials.push(Material {
            name,
            stages: stages.into(),
            settings,
            finish_has_lightmap,
        });
        Ok(id)
    }
    pub fn shader_registration(
        &mut self,
        raw: &[u8],
        shader: ShaderBindingId,
    ) -> Result<ShaderRegistration, &'static str> {
        let binding = *self
            .shader_binding(shader)
            .ok_or("invalid registration shader")?;
        self.shader_registration_value(raw, binding, None)
            .map(|registration| registration.map_shader(|_| shader))
    }
    pub(crate) fn shader_registration_value(
        &mut self,
        raw: &[u8],
        shader: ShaderBinding,
        defaulted: Option<NativeDefaultKind>,
    ) -> Result<ShaderRegistration<ShaderBinding>, &'static str> {
        let (requested, finished) = self.shader_metadata(shader, defaulted)?;
        self.intern_resource_name(raw)?;
        let exact = self.intern_resource_name(crate::shader::registration_key(raw))?;
        let key = self
            .names
            .folded(exact)
            .ok_or("missing shader registration key")?;
        Ok(ShaderRegistration {
            key,
            requested,
            finished,
            shader,
            defaulted,
            sun: None,
        })
    }
    fn shader_metadata(
        &self,
        shader: ShaderBinding,
        defaulted: Option<NativeDefaultKind>,
    ) -> Result<(LightSource, LightSource), &'static str> {
        let material = self
            .material(shader.material)
            .ok_or("invalid registration material")?;
        if matches!(defaulted, Some(NativeDefaultKind::MissingImage))
            && (!material.stages.is_empty()
                || material.settings.sort != 7.0
                || material.settings.sky.is_some()
                || material.settings.fog.is_some())
        {
            return Err("invalid native default body");
        }
        if let Some(NativeDefaultKind::Explicit(origin)) = defaulted
            && origin.material != shader.material
        {
            return Err("explicit default body changed");
        }
        let requested = shader.light_source;
        let finished = if matches!(
            requested,
            LightSource::Page(_) | LightSource::ExternalPage(_) | LightSource::Samples
        ) && !material.finish_has_lightmap()
        {
            LightSource::None
        } else {
            requested
        };
        Ok((requested, finished))
    }
    pub fn name_capacity(&self) -> usize {
        self.names.capacity()
    }
    pub fn shader_binding_capacity(&self) -> usize {
        self.shader_capacity
    }
    pub fn shader_bindings(&self) -> &[ShaderBinding] {
        &self.shader_bindings
    }
    pub fn shader_binding(&self, id: ShaderBindingId) -> Option<&ShaderBinding> {
        self.shader_bindings.get(id.0 as usize)
    }
    pub fn register_shader_binding(
        &mut self,
        binding: ShaderBinding,
    ) -> Result<ShaderBindingId, &'static str> {
        if !self.styled_shader_valid(binding) {
            return Err("invalid styled shader binding");
        }
        self.intern_shader_binding(binding)
    }
    pub(crate) fn styled_shader_valid(&self, binding: ShaderBinding) -> bool {
        binding
            .styled_surface
            .is_none_or(|(presentation, surface)| {
                self.surface_shader(presentation, surface)
                    .and_then(|shader| self.shader_binding(shader))
                    .is_some_and(|source| {
                        source.light_source == LightSource::Samples
                            && binding.light_source == source.light_source
                            && binding.lightmap == source.lightmap
                            && binding.lightmap_region == source.lightmap_region
                            && binding.lightmap_conversion == source.lightmap_conversion
                    })
            })
    }
    fn intern_shader_binding(
        &mut self,
        binding: ShaderBinding,
    ) -> Result<ShaderBindingId, &'static str> {
        if !self.shader_binding_valid(binding) {
            return Err("invalid shader binding");
        }
        let head = self.material_shader_heads[binding.material.0 as usize];
        let mut current = head;
        while let Some(index) = current {
            if self.shader_bindings[index as usize] == binding {
                return Ok(ShaderBindingId(index));
            }
            current = self.shader_next[index as usize];
        }
        if self.shader_bindings.len() == self.shader_capacity {
            return Err("shader binding table full");
        }
        let id = ShaderBindingId(self.shader_bindings.len() as u32);
        self.shader_bindings.push(binding);
        self.shader_next.push(head);
        self.material_shader_heads[binding.material.0 as usize] = Some(id.0);
        Ok(id)
    }
    fn shader_binding_valid(&self, binding: ShaderBinding) -> bool {
        self.material(binding.material).is_some()
            && self.image(binding.lightmap).is_some_and(|image| {
                binding.lightmap_region.is_none_or(|region| {
                    region.width != 0
                        && region.height != 0
                        && region
                            .x
                            .checked_add(region.width)
                            .is_some_and(|end| end <= image.width)
                        && region
                            .y
                            .checked_add(region.height)
                            .is_some_and(|end| end <= image.height)
                })
            })
    }
    pub fn register_model(
        &mut self,
        vertices: &[Vertex],
        indices: &[u32],
        shader: ShaderBindingId,
    ) -> Result<ModelId, &'static str> {
        self.register_animated_model(vertices, vertices.len(), indices, shader)
    }

    /// Load all frame-major vertices once. Indices address a single frame.
    pub fn register_animated_model(
        &mut self,
        vertices: &[Vertex],
        vertices_per_frame: usize,
        indices: &[u32],
        shader: ShaderBindingId,
    ) -> Result<ModelId, &'static str> {
        let model = self.prepare_model(vertices, vertices_per_frame, indices, shader)?;
        let id = ModelId(u32::try_from(self.models.len()).map_err(|_| "model table full")?);
        self.models.push(model);
        Ok(id)
    }

    fn prepare_model(
        &self,
        vertices: &[Vertex],
        vertices_per_frame: usize,
        indices: &[u32],
        shader: ShaderBindingId,
    ) -> Result<Model, &'static str> {
        let frame_count = if vertices_per_frame == 0 {
            if !vertices.is_empty() || !indices.is_empty() {
                return Err("invalid mesh frames");
            }
            1
        } else {
            if vertices.is_empty()
                || !vertices.len().is_multiple_of(vertices_per_frame)
                || u32::try_from(vertices.len()).is_err()
            {
                return Err("invalid mesh frames");
            }
            u32::try_from(vertices.len() / vertices_per_frame)
                .map_err(|_| "mesh frame table full")?
        };
        if self.shader_binding(shader).is_none()
            || !indices.len().is_multiple_of(3)
            || indices.iter().any(|&i| i as usize >= vertices_per_frame)
            || vertices.iter().any(|v| !v.is_finite())
        {
            return Err("invalid mesh");
        }
        Ok(Model {
            vertices: vertices.into(),
            indices: indices.into(),
            shader,
            vertices_per_frame,
            frame_count,
            sprite: None,
            alias: None,
            animation: None,
        })
    }
    pub fn register_sprite(
        &mut self,
        geometry: SpriteGeometry,
        shader: ShaderBindingId,
    ) -> Result<ModelId, &'static str> {
        if !geometry.extents.iter().all(|x| x.is_finite())
            || geometry.extents[0] >= geometry.extents[1]
            || geometry.extents[3] >= geometry.extents[2]
            || self.shader_binding(shader).is_none()
        {
            return Err("invalid sprite geometry");
        }
        let id = ModelId(u32::try_from(self.models.len()).map_err(|_| "model table full")?);
        self.models.push(Model {
            vertices: Box::new([]),
            indices: Box::new([]),
            shader,
            vertices_per_frame: 0,
            frame_count: 1,
            sprite: Some(geometry),
            alias: None,
            animation: None,
        });
        Ok(id)
    }

    /// Attach the source's immutable parent-to-pose recipe before publication.
    pub fn set_model_animation(
        &mut self,
        id: ModelId,
        animation: Arc<ModelAnimation>,
    ) -> Result<(), &'static str> {
        let model = self
            .models
            .get_mut(id.0 as usize)
            .ok_or("unknown animated model")?;
        if model.sprite.is_some() || animation.physical_frames() != model.frame_count {
            return Err("model animation frame count mismatch");
        }
        if let Some(existing) = &model.animation {
            return if Arc::ptr_eq(existing, &animation) || existing == &animation {
                Ok(())
            } else {
                Err("model animation already registered")
            };
        }
        model.animation = Some(animation);
        Ok(())
    }

    pub fn set_alias_frames(
        &mut self,
        id: ModelId,
        frames: &[AliasFrame],
        positions: &[[u8; 3]],
    ) -> Result<(), &'static str> {
        let model = self
            .models
            .get_mut(id.0 as usize)
            .ok_or("unknown alias model")?;
        if frames.len() != model.frame_count as usize
            || positions.len() != model.vertices.len()
            || frames
                .iter()
                .any(|frame| !frame.scale.is_finite() || !frame.translation.is_finite())
            || model.sprite.is_some()
        {
            return Err("invalid packed alias frames");
        }
        model.alias = Some(AliasFrames {
            frames: frames.into(),
            positions: positions.into(),
        });
        Ok(())
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
    fn world_materials_valid(&self, materials: &[SurfaceMaterial], surfaces: usize) -> bool {
        materials.len() == surfaces
            && materials.iter().all(|binding| {
                self.shader_binding_valid(binding.shader)
                    && binding
                        .texture_scale
                        .iter()
                        .all(|value| value.is_finite() && *value > 0.0)
            })
    }

    fn register_world_shaders(
        &mut self,
        presentation: WorldPresentationId,
        input: WorldPresentationInput<'_>,
    ) -> Result<
        (
            Vec<crate::shader::reference::ShaderChoice>,
            Box<[ShaderRegistration]>,
        ),
        &'static str,
    > {
        if input
            .native_page0
            .is_some_and(|id| self.image(id).is_none())
        {
            return Err("invalid native lightmap page");
        }
        if input
            .native_default
            .is_some_and(|shader| self.shader_binding(shader).is_none())
        {
            return Err("invalid native default shader");
        }
        for registration in input.registrations {
            let binding = match registration.shader {
                PreparedShader::Surface(index) | PreparedShader::ConvertedSurface(index) => {
                    input
                        .surfaces
                        .get(index as usize)
                        .ok_or("invalid native shader surface")?
                        .shader
                }
                PreparedShader::Binding { shader, surface } => {
                    if surface.is_some_and(|index| index as usize >= input.surfaces.len()) {
                        return Err("invalid native shader surface");
                    }
                    shader
                }
            };
            if self.names.folded(registration.key) != Some(registration.key)
                || !self.shader_binding_valid(binding)
                || !self.styled_shader_valid(binding)
                || self.shader_metadata(binding, registration.defaulted)?
                    != (registration.requested, registration.finished)
            {
                return Err("invalid native shader registration");
            }
        }
        let shaders = input
            .surfaces
            .iter()
            .enumerate()
            .map(|(surface, material)| {
                let mut binding = material.shader;
                if binding.lightmap_conversion.is_some() {
                    binding.styled_surface = Some((presentation, surface as u32));
                }
                self.intern_shader_binding(binding)
            })
            .collect::<Result<Vec<_>, _>>()?;
        let mut choices: Vec<_> = shaders
            .iter()
            .copied()
            .map(crate::shader::reference::ShaderChoice::Direct)
            .collect();
        let registrations = input
            .registrations
            .iter()
            .enumerate()
            .map(|(occurrence, registration)| {
                let (shader, surface, converted) = match registration.shader {
                    PreparedShader::Surface(index) | PreparedShader::ConvertedSurface(index) => (
                        shaders[index as usize],
                        Some(index),
                        matches!(registration.shader, PreparedShader::ConvertedSurface(_)),
                    ),
                    PreparedShader::Binding { shader, surface } => {
                        (self.intern_shader_binding(shader)?, surface, false)
                    }
                };
                if let Some(surface) = surface {
                    let occurrence = crate::shader::reference::ShaderOccurrence(
                        u32::try_from(occurrence).map_err(|_| "shader occurrence capacity")?,
                    );
                    choices[surface as usize] = if converted {
                        crate::shader::reference::ShaderChoice::Converted {
                            binding: shader,
                            occurrence,
                        }
                    } else {
                        crate::shader::reference::ShaderChoice::Occurrence(occurrence)
                    };
                }
                Ok(registration.map_shader(|_| shader))
            })
            .collect::<Result<Box<[_]>, &'static str>>()?;
        Ok((choices, registrations))
    }
    pub(crate) fn append_world_presentation(
        &mut self,
        world: WorldId,
        input: WorldPresentationInput<'_>,
        replaced: Option<WorldPresentationId>,
    ) -> Result<WorldPresentationId, &'static str> {
        let retained = self.counts();
        let result = (|| {
            let existing = self.world(world).ok_or("missing presentation world")?;
            let insert = replaced
                .map(|old| {
                    existing
                        .presentations()
                        .iter()
                        .position(|&id| id == old)
                        .ok_or("missing replaced world presentation")
                })
                .transpose()?
                .unwrap_or(existing.presentations().len());
            if !self.world_materials_valid(input.surfaces, existing.geometry.surfaces.len()) {
                return Err("invalid world surface bindings");
            }
            let id = WorldPresentationId(
                u32::try_from(self.presentations.len())
                    .map_err(|_| "world presentation table full")?,
            );
            let original = self
                .world_presentation(
                    *existing
                        .presentations()
                        .first()
                        .ok_or("world has no presentation")?,
                )
                .ok_or("missing original presentation")?;
            let mesh_ranges: Vec<_> = original
                .bindings
                .iter()
                .map(|binding| binding.mesh_indices)
                .collect();
            let (shaders, registrations) = self.register_world_shaders(id, input)?;
            let presentation = WorldPresentation::bind(
                world,
                input.surfaces,
                mesh_ranges.into_iter(),
                shaders.into_iter(),
                registrations,
                input.native_page0,
                input.native_default,
                self.name_capacity(),
                self.shader_binding_capacity(),
            )?;
            self.presentations.push(presentation);
            self.worlds[world.0 as usize]
                .presentations
                .insert(insert, id);
            Ok(id)
        })();
        if result.is_err() {
            self.discard_registration_suffix(retained)?;
        }
        result
    }

    pub fn register_world(
        &mut self,
        geometry: WorldGeometry,
        visibility: VisibilityWorld,
        variants: &[WorldPresentationInput<'_>],
    ) -> Result<WorldId, &'static str> {
        let retained = self.counts();
        let result = (|| {
            if geometry.surfaces.len() != visibility.surface_count()
                || variants.is_empty()
                || variants.iter().any(|input| {
                    !self.world_materials_valid(input.surfaces, geometry.surfaces.len())
                })
            {
                return Err("invalid world surface bindings");
            }
            let id = WorldId(u32::try_from(self.worlds.len()).map_err(|_| "world table full")?);
            let first = u32::try_from(self.presentations.len())
                .map_err(|_| "world presentation table full")?;
            let count =
                u32::try_from(variants.len()).map_err(|_| "world presentation table full")?;
            first
                .checked_add(count)
                .ok_or("world presentation table full")?;
            let mesh = retessellation::world_mesh(&geometry)?;
            let mut registrations = Vec::with_capacity(variants.len());
            for (index, &input) in variants.iter().enumerate() {
                let (shaders, native_registrations) =
                    self.register_world_shaders(WorldPresentationId(first + index as u32), input)?;
                registrations.push(WorldPresentation::bind(
                    id,
                    input.surfaces,
                    mesh.ranges.iter().copied(),
                    shaders.into_iter(),
                    native_registrations,
                    input.native_page0,
                    input.native_default,
                    self.name_capacity(),
                    self.shader_binding_capacity(),
                )?);
            }
            let mesh = self.register_model(&mesh.vertices, &mesh.indices, ShaderBindingId(0))?;
            self.worlds.push(World {
                geometry,
                visibility: visibility.into(),
                mesh,
                presentations: (first..first + count).map(WorldPresentationId).collect(),
                material_source: None,
                light_grid: None,
                point_lighting: None,
            });
            self.presentations.extend(registrations);
            Ok(id)
        })();
        if result.is_err() {
            self.discard_registration_suffix(retained)?;
        }
        result
    }

    /// Clear file-derived image caches once per retained world owner. The caller
    /// keeps this receipt in its existing cold registration transaction.
    pub fn begin_world_image_reload(
        &mut self,
    ) -> Result<crate::material::world_load::WorldImageReload, &'static str> {
        if self
            .worlds
            .iter()
            .filter_map(|world| world.material_source.as_ref())
            .any(|source| !source.image_reload_ready())
        {
            return Err("world images already borrowed");
        }
        let states = self
            .worlds
            .iter_mut()
            .enumerate()
            .filter_map(|(index, world)| {
                world
                    .material_source
                    .as_mut()
                    .map(|source| (WorldId(index as u32), source.begin_image_reload()))
            })
            .collect();
        Ok(crate::material::world_load::WorldImageReload { states })
    }

    pub(crate) fn restore_world_image_reload(
        &mut self,
        reload: crate::material::world_load::WorldImageReload,
    ) -> Result<(), &'static str> {
        if reload.states.iter().any(|(id, states)| {
            self.worlds
                .get(id.0 as usize)
                .and_then(|world| world.material_source.as_ref())
                .is_none_or(|source| !source.image_reload_matches(states))
        }) {
            return Err("world image reload owner changed");
        }
        for (id, states) in reload.states {
            if let Some(source) = self.worlds[id.0 as usize].material_source.as_mut() {
                source.restore_image_reload(states);
            }
        }
        Ok(())
    }

    pub(crate) fn replay_world_image_reload(
        &mut self,
        vfs: &qa_content::vfs::Vfs,
        reload: &crate::material::world_load::WorldImageReload,
    ) -> Result<(), crate::material::world_load::WorldMaterialError> {
        for (world, states) in &reload.states {
            self.with_world_material_source(*world, |assets, source| {
                source.replay_images(vfs, assets, states)
            })?;
        }
        Ok(())
    }

    pub(crate) fn retain_world_material_source(
        &mut self,
        id: WorldId,
        source: crate::material::world_load::WorldMaterialSource,
    ) -> Result<(), &'static str> {
        let world = self
            .worlds
            .get_mut(id.0 as usize)
            .ok_or("missing source world")?;
        if world.material_source.is_some() {
            return Err("world material source already retained");
        }
        world.material_source = Some(source);
        Ok(())
    }

    pub(crate) fn with_world_material_source<T>(
        &mut self,
        id: WorldId,
        compile: impl FnOnce(
            &mut Self,
            &mut crate::material::world_load::WorldMaterialSource,
        ) -> Result<T, crate::material::world_load::WorldMaterialError>,
    ) -> Result<T, crate::material::world_load::WorldMaterialError> {
        use crate::material::world_load::WorldMaterialError;
        let mut source = self
            .worlds
            .get_mut(id.0 as usize)
            .and_then(|world| world.material_source.take())
            .ok_or(WorldMaterialError::Boundary(
                "world has no retained material source",
            ))?;
        let result = compile(self, &mut source);
        // The compiler contains no external callback. Restore the same cold
        // recipe before any result reaches caller staging/backend admission.
        self.worlds[id.0 as usize].material_source = Some(source);
        result
    }

    /// Attach the caller's presentation policies to existing numeric world
    /// bindings. Published metadata is immutable; repeated registration agrees.
    pub fn register_world_presentation_metadata(
        &mut self,
        world: WorldId,
        entries: &[crate::material::world_load::LoadedWorldPresentation],
    ) -> Result<(), &'static str> {
        let registered = self.world(world).ok_or("missing world")?;
        for (index, entry) in entries.iter().enumerate() {
            if !registered.presentations().contains(&entry.id)
                || self
                    .world_presentation(entry.id)
                    .is_none_or(|presentation| presentation.world() != world)
            {
                return Err("world presentation does not belong to world");
            }
            if self
                .world_presentation_registration(entry.id)
                .is_some_and(|current| current != *entry)
                || entries[..index]
                    .iter()
                    .any(|previous| previous.id == entry.id && previous != entry)
            {
                return Err("world presentation metadata changed");
            }
        }
        for entry in entries {
            self.presentations[entry.id.0 as usize].profile = Some((
                entry.profile,
                entry.defaults,
                entry.lighting,
                entry.compile_policy,
            ));
        }
        Ok(())
    }

    pub fn surface_shader(
        &self,
        presentation: WorldPresentationId,
        surface: u32,
    ) -> Option<ShaderBindingId> {
        self.world_presentation(presentation)?
            .surface_shader(surface)
    }

    pub fn world_presentation_registration(
        &self,
        id: WorldPresentationId,
    ) -> Option<crate::material::world_load::LoadedWorldPresentation> {
        let (profile, defaults, lighting, compile_policy) = self.world_presentation(id)?.profile?;
        Some(crate::material::world_load::LoadedWorldPresentation {
            id,
            profile,
            defaults,
            lighting,
            compile_policy,
        })
    }

    pub fn world(&self, id: WorldId) -> Option<&World> {
        self.worlds.get(id.0 as usize)
    }
    pub fn set_light_grid(
        &mut self,
        id: WorldId,
        grid: Option<crate::lighting::LightGrid>,
    ) -> Result<(), &'static str> {
        self.worlds
            .get_mut(id.0 as usize)
            .ok_or("world light-grid handle")?
            .light_grid = grid;
        Ok(())
    }
    pub fn set_point_lighting(
        &mut self,
        id: WorldId,
        lighting: crate::lighting::PointLighting,
    ) -> Result<(), &'static str> {
        self.worlds
            .get_mut(id.0 as usize)
            .ok_or("world point-light handle")?
            .point_lighting = Some(lighting);
        Ok(())
    }
    pub fn world_presentation(&self, id: WorldPresentationId) -> Option<&WorldPresentation> {
        self.presentations.get(id.0 as usize)
    }
    pub fn world_presentations(&self) -> &[WorldPresentation] {
        &self.presentations
    }
    pub fn surface_binding(
        &self,
        presentation: WorldPresentationId,
        surface: u32,
    ) -> Option<&SurfaceBinding> {
        self.world_presentation(presentation)?
            .bindings()
            .get(surface as usize)
    }
    /// Copied scene references must name the world owning their presentation.
    pub(crate) fn scene_binding(
        &self,
        surface: crate::scene::SurfaceRef,
    ) -> Option<&SurfaceBinding> {
        (self.world_presentation(surface.presentation)?.world() == surface.world).then_some(())?;
        self.surface_binding(surface.presentation, surface.surface)
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
        && settings.software_tiled.is_none_or(|tile| {
            (tile.turbulent || tile.flow_speed.is_some())
                && tile.flow_speed.is_none_or(f64::is_finite)
        })
        && settings.software_alias.is_none_or(|alias| {
            alias
                .fullbright_light
                .is_none_or(|values| finite(&values) && values.iter().all(|&value| value >= 0.0))
                && alias.alpha_cutoffs.is_none_or(|values| {
                    finite(&values)
                        && values[0] >= 0.0
                        && values[0] <= values[1]
                        && values[1] <= 1.0
                })
        })
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
