//! Registration is a load operation. Frames use numeric handles only.
mod admission;
pub mod upload;
pub(crate) use admission::AssetAdmission;
pub use admission::AssetCounts;

use crate::scene::Span;
pub use crate::shader::{AlphaFunc as AlphaTest, Cull, TexCoordGen as TcGen};
use crate::shader::{AlphaGen, Deform, FogParms, RgbGen, StageBlend, TexMod};
use crate::surface_cache::{IndexedTexture, PaletteLighting};
use crate::world::{
    SurfaceBinding, SurfaceMaterial, World, WorldId, WorldPresentation, WorldPresentationId,
    geometry::WorldGeometry,
};
use qa_core::names::{NameTable, NamesError};
pub use qa_core::primitives::ModelId;
use qa_core::primitives::{NameId, Vec3};
use qa_world::visibility::VisibilityWorld;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ImageId(pub u32);
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MaterialId(pub u32);
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
}
pub struct Model {
    pub vertices: Box<[Vertex]>,
    pub indices: Box<[u32]>,
    pub material: MaterialId,
    pub vertices_per_frame: usize,
    frame_count: u32,
    pub sprite: Option<SpriteGeometry>,
    alias: Option<AliasFrames>,
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
    images: Vec<Image>,
    materials: Vec<Material>,
    models: Vec<Model>,
    worlds: Vec<World>,
    presentations: Vec<WorldPresentation>,
    palettes: Vec<PaletteLighting>,
}
impl Assets {
    pub fn load() -> Result<Self, NamesError> {
        let names = NameTable::load_reserved([b"*white".as_slice()], 65536, 4 * 1024 * 1024)?;
        let white = names.find_path("*white").ok_or(NamesError::Capacity)?;
        Ok(Self {
            names,
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
            }],
            models: vec![Model {
                vertices: Box::new([]),
                indices: Box::new([]),
                material: MaterialId(0),
                vertices_per_frame: 0,
                frame_count: 1,
                sprite: None,
                alias: None,
            }],
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
    /// Lump names are byte identities, not paths. Their decoder owns native
    /// folded lookup; slash conversion must not merge distinct lump names.
    pub fn intern_resource_name(&mut self, name: &[u8]) -> Result<NameId, &'static str> {
        self.names.intern(name).map_err(|_| "asset name table full")
    }
    pub fn name(&self, id: NameId) -> Option<&str> {
        std::str::from_utf8(self.names.get(id)?).ok()
    }
    pub fn find_name(&self, path: &str) -> Option<NameId> {
        self.names.find_path(path)
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
        let name = self.intern_name(name)?;
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
            name,
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
        self.register_animated_model(vertices, vertices.len(), indices, material)
    }

    /// Load all frame-major vertices once. Indices address a single frame.
    pub fn register_animated_model(
        &mut self,
        vertices: &[Vertex],
        vertices_per_frame: usize,
        indices: &[u32],
        material: MaterialId,
    ) -> Result<ModelId, &'static str> {
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
        if self.material(material).is_none()
            || !indices.len().is_multiple_of(3)
            || indices.iter().any(|&i| i as usize >= vertices_per_frame)
            || vertices.iter().any(|v| !v.is_finite())
        {
            return Err("invalid mesh");
        }
        let id = ModelId(u32::try_from(self.models.len()).map_err(|_| "model table full")?);
        self.models.push(Model {
            vertices: vertices.into(),
            indices: indices.into(),
            material,
            vertices_per_frame,
            frame_count,
            sprite: None,
            alias: None,
        });
        Ok(id)
    }
    pub fn register_sprite(
        &mut self,
        geometry: SpriteGeometry,
        material: MaterialId,
    ) -> Result<ModelId, &'static str> {
        if !geometry.extents.iter().all(|x| x.is_finite())
            || geometry.extents[0] >= geometry.extents[1]
            || geometry.extents[3] >= geometry.extents[2]
            || self.material(material).is_none()
        {
            return Err("invalid sprite geometry");
        }
        let id = ModelId(u32::try_from(self.models.len()).map_err(|_| "model table full")?);
        self.models.push(Model {
            vertices: Box::new([]),
            indices: Box::new([]),
            material,
            vertices_per_frame: 0,
            frame_count: 1,
            sprite: Some(geometry),
            alias: None,
        });
        Ok(id)
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
    pub fn register_world(
        &mut self,
        geometry: WorldGeometry,
        visibility: VisibilityWorld,
        variants: &[&[SurfaceMaterial]],
    ) -> Result<WorldId, &'static str> {
        if geometry.surfaces.len() != visibility.surface_count()
            || variants.is_empty()
            || variants.iter().any(|materials| {
                materials.len() != geometry.surfaces.len()
                    || materials.iter().any(|binding| {
                        self.material(binding.material).is_none()
                            || !self.image(binding.lightmap).is_some_and(|image| {
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
                            || binding
                                .texture_scale
                                .iter()
                                .any(|value| !value.is_finite() || *value <= 0.0)
                    })
            })
        {
            return Err("invalid world surface bindings");
        }
        let id = WorldId(u32::try_from(self.worlds.len()).map_err(|_| "world table full")?);
        let first =
            u32::try_from(self.presentations.len()).map_err(|_| "world presentation table full")?;
        let count = u32::try_from(variants.len()).map_err(|_| "world presentation table full")?;
        first
            .checked_add(count)
            .ok_or("world presentation table full")?;
        let mut indices = Vec::new();
        let mut mesh_ranges = Vec::with_capacity(geometry.surfaces.len());
        for (source, surface) in geometry.surfaces.iter().enumerate() {
            let range = surface.indices.range();
            if source != surface.source_id as usize
                || range.end > geometry.indices.len()
                || !range.len().is_multiple_of(3)
            {
                return Err("invalid world triangle range");
            }
            let first = u32::try_from(indices.len()).map_err(|_| "world mesh full")?;
            indices.extend_from_slice(&geometry.indices[range]);
            mesh_ranges.push(Span {
                first,
                count: surface.indices.count,
            });
        }
        let registrations: Vec<_> = variants
            .iter()
            .map(|materials| {
                let bindings = materials
                    .iter()
                    .zip(&mesh_ranges)
                    .map(|(material, &mesh_indices)| SurfaceBinding {
                        material: material.material,
                        lightmap: material.lightmap,
                        lightmap_region: material.lightmap_region,
                        lightmap_conversion: material.lightmap_conversion,
                        texture_scale: material.texture_scale,
                        mesh_indices,
                    })
                    .collect::<Vec<_>>()
                    .into_boxed_slice();
                WorldPresentation {
                    world: id,
                    bindings,
                }
            })
            .collect();
        let vertices: Vec<_> = geometry.vertices.iter().map(|v| v.vertex).collect();
        let mesh = self.register_model(&vertices, &indices, MaterialId(0))?;
        self.worlds.push(World {
            geometry,
            visibility: visibility.into(),
            mesh,
            presentations: Span { first, count },
            light_grid: None,
            point_lighting: None,
            sun_direction: crate::lighting::default_sun_direction(),
        });
        self.presentations.extend(registrations);
        Ok(id)
    }
    pub fn world(&self, id: WorldId) -> Option<&World> {
        self.worlds.get(id.0 as usize)
    }
    pub fn set_world_sun(&mut self, id: WorldId, direction: Vec3) -> Result<(), &'static str> {
        if !direction.is_finite() {
            return Err("invalid world sun direction");
        }
        self.worlds
            .get_mut(id.0 as usize)
            .ok_or("world sun handle")?
            .sun_direction = direction;
        Ok(())
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
