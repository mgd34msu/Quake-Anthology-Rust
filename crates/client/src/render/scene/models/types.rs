//! Scene-model submission types (donor `src/render/scene/models/types.ts`).
//!
//! Family-independent entity, pose, option, and preparation records shared by
//! the model transform, bounds, preparation, and renderer modules.

use std::cell::RefCell;
use std::collections::HashMap;

use qa_content::common::TimedFrames;
use qa_content::md5::SkeletonJointPose;
use qa_content::q3scene::SceneMd3;
use qa_core::math::{Axis, Bounds, Plane, Vec2, Vec3, Vec4};

use crate::materials::geometry::MaterialGeometry;
use crate::render::error::RenderError;
use crate::render::types::{
    AlphaTest, CullFace, DrawBatch, ImageLevel, ImageSource, Palette, PaletteTransparency, RenderImage,
};
use crate::view::SceneCamera;

use super::replacements::ModelReplacementPolicy;

/// Model-space to world-space transform with per-axis scale.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EntityTransform {
    /// World-space origin.
    pub origin: Vec3,
    /// Model basis (forward, left, up).
    pub axis: Axis,
    /// Per-axis scale.
    pub scale: Vec3,
}

impl EntityTransform {
    /// Identity transform.
    #[must_use]
    pub const fn identity() -> Self {
        Self {
            origin: Vec3 { x: 0.0, y: 0.0, z: 0.0 },
            axis: [
                Vec3 { x: 1.0, y: 0.0, z: 0.0 },
                Vec3 { x: 0.0, y: 1.0, z: 0.0 },
                Vec3 { x: 0.0, y: 0.0, z: 1.0 },
            ],
            scale: Vec3 { x: 1.0, y: 1.0, z: 1.0 },
        }
    }
}

/// Animation pose: interpolated frames or a full skeleton.
#[derive(Debug, Clone, PartialEq)]
pub enum ScenePose {
    /// Interpolated frame pair.
    Frame {
        /// Current frame.
        frame: i32,
        /// Previous frame.
        previous_frame: i32,
        /// Blend toward the previous frame.
        back_lerp: f32,
    },
    /// Full skeleton joint poses.
    Skeleton {
        /// Joint poses.
        joints: Vec<SkeletonJointPose>,
    },
}

/// Source-family render flags.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntityFlags {
    /// Quake flags.
    Q1 {
        /// Flag bits.
        bits: u32,
    },
    /// Quake II flags.
    Q2 {
        /// Flag bits.
        bits: u32,
    },
    /// Quake III flags.
    Q3 {
        /// Flag bits.
        bits: u32,
    },
}

impl EntityFlags {
    /// Raw flag bits.
    #[must_use]
    pub const fn bits(self) -> u32 {
        match self {
            Self::Q1 { bits } | Self::Q2 { bits } | Self::Q3 { bits } => bits,
        }
    }
}

/// Content identity for a scene entity's model.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelResource {
    /// Registry identity.
    pub id: String,
    /// Requested content path.
    pub requested_path: String,
    /// Fingerprint of the source bytes the entity was built from.
    pub digest: u64,
}

/// Enhanced replacement model selected for an alias entity.
#[derive(Debug, Clone, PartialEq)]
pub struct SceneModelReplacement {
    /// Replacement resource identity.
    pub resource: ModelResource,
    /// Replacement model.
    pub model: Box<SceneModel>,
}

/// Decoded scene model, one variant per source family.
#[derive(Debug, Clone, PartialEq)]
pub enum SceneModel {
    /// Quake alias model.
    Q1Mdl {
        /// Parsed model.
        model: qa_content::mdl::MdlModel,
        /// World-aligned local bounds.
        bounds: Bounds,
        /// Model effect flags.
        flags: i32,
        /// Enhanced replacement, when loaded.
        replacement: Option<SceneModelReplacement>,
    },
    /// Quake II alias model.
    Q2Md2 {
        /// Parsed model.
        model: qa_content::md2::Md2Model,
        /// Enhanced replacement, when loaded.
        replacement: Option<SceneModelReplacement>,
    },
    /// Quake III mesh model.
    Q3Md3(SceneMd3),
    /// Quake III skeletal model.
    Q3Md4(qa_content::md4::Md4Model),
    /// Skeletal MD5 model.
    Md5(qa_content::md5::DecodedMd5Model),
    /// Quake sprite.
    Q1Spr(qa_content::spr::SprModel),
    /// Quake II sprite.
    Q2Sp2(qa_content::spr::Sp2Model),
    /// Brush model; world geometry handles its surfaces.
    BrushModel,
}

impl SceneModel {
    /// Animation frame count; brush models report one.
    #[must_use]
    pub fn frame_count(&self) -> usize {
        match self {
            Self::BrushModel => 1,
            Self::Q1Mdl { model, .. } => model.frames.len(),
            Self::Q2Md2 { model, .. } => model.frames.len(),
            Self::Q3Md3(model) => model.frames.len(),
            Self::Q3Md4(model) => model.frames.len(),
            Self::Md5(model) => model.frames.len(),
            Self::Q1Spr(model) => model.frames.len(),
            Self::Q2Sp2(model) => model.frames.len(),
        }
    }
}

/// Named tag attachment of a child entity.
#[derive(Debug, Clone, PartialEq)]
pub struct ModelAttachment {
    /// Tag name on the parent.
    pub tag: String,
    /// Attached child entity.
    pub entity: Box<SceneEntity>,
}

/// One renderable scene entity.
#[derive(Debug, Clone, PartialEq)]
pub struct SceneEntity {
    /// Content identity.
    pub resource: ModelResource,
    /// Decoded model.
    pub model: SceneModel,
    /// Animation pose.
    pub pose: ScenePose,
    /// World transform.
    pub transform: EntityTransform,
    /// Previous-frame origin (beam endpoints, MD2 motion).
    pub previous_origin: Vec3,
    /// Lighting sample origin.
    pub lighting_origin: Vec3,
    /// Entity color, normalized 0..=1.
    pub color: Vec4,
    /// Selected skin.
    pub skin: i32,
    /// Shader clock offset in seconds.
    pub shader_time_seconds: f64,
    /// Source-family flags.
    pub flags: EntityFlags,
    /// Tag attachments.
    pub attachments: Vec<ModelAttachment>,
    /// Client slot, when the entity belongs to an actor.
    pub actor_slot: Option<u32>,
}

/// Attachment tag in model space.
#[derive(Debug, Clone, PartialEq)]
pub struct ModelTag {
    /// Tag name.
    pub name: String,
    /// Tag origin.
    pub origin: Vec3,
    /// Tag basis.
    pub axis: Axis,
}

/// Why a surface fell back to the default image.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DefaultImageReason {
    /// A custom skin omitted this surface.
    MissingSkinSurface,
    /// The surface declares no skin.
    NoSkin,
}

/// Image selected for one prepared surface.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModelImageSelection {
    /// External content image or shader name.
    External {
        /// Content name.
        name: String,
    },
    /// Embedded indexed pixels.
    Indexed {
        /// Cache name.
        name: String,
        /// Width in pixels.
        width: u32,
        /// Height in pixels.
        height: u32,
        /// Indexed pixels.
        pixels: Vec<u8>,
        /// Transparent palette index.
        transparent_index: Option<u8>,
        /// Whether indices 224..=255 stay fullbright.
        fullbright: bool,
    },
    /// Solid white.
    White,
    /// Missing-texture fallback.
    Default {
        /// Fallback reason.
        reason: DefaultImageReason,
    },
}

/// Caller-supplied indexed skin replacing an MDL skin.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexedModelSkin {
    /// Cache name.
    pub name: String,
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// Indexed pixels.
    pub pixels: Vec<u8>,
}

/// Custom skin entry mapping a surface to a shader.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CustomSkinEntry {
    /// Surface name.
    pub name: String,
    /// Replacement shader.
    pub shader: String,
}

/// Beam subdivision options for a model beam entity.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ModelBeamOptions {
    /// Length of one beam segment.
    pub segment_length: f32,
}

/// Player shirt/shorts color indices.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlayerColors {
    /// Shirt color.
    pub top: u8,
    /// Shorts color.
    pub bottom: u8,
}

/// Source renderer fields supplementing a scene entity.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ModelSourceOptions {
    /// Caller-supplied indexed skin.
    pub indexed_skin: Option<IndexedModelSkin>,
    /// Beam subdivision, when the entity renders as a beam.
    pub model_beam: Option<ModelBeamOptions>,
    /// Animation clock base in seconds.
    pub sync_base: f64,
    /// Sprite roll in degrees.
    pub sprite_roll: f32,
    /// Shader overriding every surface.
    pub custom_shader: Option<String>,
    /// Per-surface shader overrides.
    pub custom_skin: Option<Vec<CustomSkinEntry>>,
    /// MD3 LOD choices, selected by projected radius.
    pub q3_lods: Option<Vec<Option<SceneMd3>>>,
    /// LOD distance scale.
    pub lod_scale: Option<f32>,
    /// LOD index bias.
    pub lod_bias: Option<i32>,
    /// Axes carry non-unit length (Q3 view weapons).
    pub non_normalized_axes: bool,
    /// Skip world-model lighting lookups.
    pub no_world_model: bool,
    /// Shader texture-coordinate override.
    pub shader_tex_coord: Option<Vec2>,
    /// Left-hand weapon mode.
    pub left_hand: u8,
    /// Infrared rendering.
    pub infrared: bool,
    /// First-person view model.
    pub view_model: bool,
    /// Planar shadow projection (Q1).
    pub planar_shadow: bool,
    /// Player entity (minimum ambient).
    pub player: bool,
    /// Double alias brightness (Q1).
    pub overbright_models: Option<bool>,
    /// Player translation colors.
    pub player_colors: Option<PlayerColors>,
}

/// Sampled poses keyed by (mesh source, frame, previous, back-lerp bits).
pub type PoseCache = HashMap<(String, i32, i32, u32), Vec<SkeletonJointPose>>;

/// Skinned meshes keyed by (mesh source, mesh index, joints identity).
pub type MeshCache = HashMap<(String, usize, u64), Vec<SkinnedMeshVertex>>;

/// Per-frame MD5 skinning caches shared across a preparation pass.
#[derive(Debug, Default)]
pub struct ModelSkinningFrame {
    /// Sampled poses keyed by (mesh source, frame, previous, back-lerp bits).
    pub poses: RefCell<PoseCache>,
    /// Skinned meshes keyed by (mesh source, mesh index, joints identity).
    pub meshes: RefCell<MeshCache>,
}

/// One skinned MD5 vertex with texture coordinates.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SkinnedMeshVertex {
    /// Position.
    pub position: Vec3,
    /// Normal.
    pub normal: Vec3,
    /// Texture coordinates.
    pub tex_coord: Vec2,
}

/// Purpose of a preparation pass.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreparePurpose {
    /// Visible view geometry.
    View,
    /// Shadow-caster geometry.
    Shadow,
}

/// Shadow-sphere record used by shadow-body retention.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ShadowSphere {
    /// World-space origin.
    pub origin: Vec3,
    /// Radius in world units.
    pub radius: f32,
}

/// Callbacks the preparation pass needs from its owner.
pub trait ModelPreparationHooks {
    /// Source options for an entity.
    fn options(&self, _entity: &SceneEntity) -> ModelSourceOptions {
        ModelSourceOptions::default()
    }

    /// Normalized alias-light modulation for one vertex.
    fn light_vertex(&self, _entity: &SceneEntity, _normal: Vec3, _position: Vec3) -> Option<Vec3> {
        None
    }

    /// Complete source vertex modulation for an entity.
    fn prepare_vertex_lighting(
        &self,
        _entity: &SceneEntity,
        _options: &ModelSourceOptions,
    ) -> Option<ModelVertexLighting> {
        None
    }

    /// Palette byte colors used by Q2 beams.
    fn palette_color(&self, _entity: &SceneEntity, _index: u8) -> Option<Vec3> {
        None
    }

    /// Whether an MD5 shadow body survives envelope culling.
    fn retain_shadow_body(
        &self,
        _entity: &SceneEntity,
        _sphere: &ShadowSphere,
        _images: &[ModelImageSelection],
        _options: &ModelSourceOptions,
    ) -> bool {
        true
    }
}

/// Complete source vertex modulation: normal, position, corner to RGB.
pub type ModelVertexLighting = Box<dyn Fn(Vec3, Vec3, usize) -> Vec3>;

/// Inputs for one model preparation pass.
pub struct ModelPreparationContext<'a> {
    /// Shared skinning caches.
    pub skinning_frame: Option<&'a ModelSkinningFrame>,
    /// Replacement policy.
    pub model_policy: Option<ModelReplacementPolicy>,
    /// Preparation purpose.
    pub purpose: PreparePurpose,
    /// View camera.
    pub camera: SceneCamera,
    /// Animation clock in seconds.
    pub time_seconds: f64,
    /// Cull frustum planes.
    pub frustum: Option<Vec<Plane>>,
    /// Skip frustum culling.
    pub no_cull: bool,
    /// Owner callbacks.
    pub hooks: &'a dyn ModelPreparationHooks,
}

/// One prepared model surface.
#[derive(Debug, Clone, PartialEq)]
pub struct PreparedModelSurface {
    /// Surface index within the entity.
    pub surface_index: usize,
    /// MD3 fog sphere from the selected frame.
    pub fog_sphere: Option<FogSphere>,
    /// Resolved source options.
    pub options: ModelSourceOptions,
    /// Surface name.
    pub name: String,
    /// Prepared entity (replacement-resolved).
    pub entity: SceneEntity,
    /// World transform.
    pub transform: EntityTransform,
    /// Selected image.
    pub image: ModelImageSelection,
    /// Model-space geometry.
    pub local_geometry: MaterialGeometry,
    /// World-space geometry.
    pub geometry: MaterialGeometry,
    /// Depth range.
    pub depth_range: [f32; 2],
    /// Face culling.
    pub cull: CullFace,
    /// Alpha test.
    pub alpha_test: AlphaTest,
    /// Translucent blending.
    pub translucent: bool,
    /// Unlit surface.
    pub unlit: bool,
    /// Mirrored left-hand weapon projection.
    pub mirror_weapon: bool,
}

/// Fog sphere from an MD3 frame record.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FogSphere {
    /// Model-space origin.
    pub local_origin: Vec3,
    /// Radius.
    pub radius: f32,
}

/// Frustum-cull result for a prepared entity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelCull {
    /// Fully inside.
    In,
    /// Partially inside.
    Clip,
    /// Fully outside.
    Out,
}

/// One prepared entity with its surfaces and attachments.
#[derive(Debug, Clone, PartialEq)]
pub struct PreparedModelEntity {
    /// Prepared entity (replacement-resolved).
    pub entity: SceneEntity,
    /// Repaired frame.
    pub frame: usize,
    /// Repaired previous frame.
    pub previous_frame: usize,
    /// A frame index fell back to zero.
    pub frame_fallback: bool,
    /// Selected LOD index.
    pub lod: usize,
    /// World bounds.
    pub bounds: Option<Bounds>,
    /// Cull result.
    pub cull: ModelCull,
    /// First-person model skipped in mirrored views.
    pub personal_model: bool,
    /// Prepared surfaces.
    pub surfaces: Vec<PreparedModelSurface>,
    /// Prepared attachments (or beam segments).
    pub attachments: Vec<PreparedModelEntity>,
    /// Attachment tags missing from the parent.
    pub missing_attachments: Vec<String>,
    /// Q1 model effect flags retained for the effect owner.
    pub model_effect_flags: i32,
}

/// Submission order of a model draw group.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelGroupOrder {
    /// Opaque order.
    Opaque,
    /// Translucent order.
    Translucent,
    /// Compiled shader order.
    Compiled,
}

/// One model draw group: ordered batches from a single surface.
#[derive(Debug, Clone, PartialEq)]
pub struct ModelDrawGroup {
    /// Submission order.
    pub order: ModelGroupOrder,
    /// Ordered batches.
    pub batches: Vec<DrawBatch>,
}

/// Material sink turning prepared surfaces into draw groups.
pub trait ModelGroupContext {
    /// Draw one prepared surface.
    fn draw(&self, surface: &PreparedModelSurface) -> Vec<ModelDrawGroup>;
}

/// Indexed palette plus its content source path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelPalette {
    /// 768 RGB bytes.
    pub colors: Vec<u8>,
    /// Content path the palette was resolved from.
    pub source: String,
}

impl ModelPalette {
    /// Adapt to a renderer palette.
    #[must_use]
    pub fn to_render_palette(&self) -> Palette {
        Palette {
            colors: self.colors.clone(),
            source: self.source.clone(),
        }
    }
}

/// Build an indexed renderer image from an indexed selection.
#[must_use]
pub fn model_image(
    name: &str,
    level: ImageLevel,
    transparent_index: Option<u8>,
    fullbright: bool,
    palette: &ModelPalette,
    translation: Option<Vec<u8>>,
) -> RenderImage {
    let last = if transparent_index == Some(255) { 254 } else { 255 };
    RenderImage::Indexed8 {
        levels: vec![level],
        palette: Palette {
            colors: palette.colors.clone(),
            source: format!("{}:{}", palette.source, name),
        },
        transparency: transparent_index.map_or(PaletteTransparency::Opaque, PaletteTransparency::Index),
        fullbright: fullbright.then_some((224, last)),
        translation,
    }
}

/// Scale a normalized color to byte range.
#[must_use]
pub fn byte_color(color: Vec4) -> Vec4 {
    Vec4 {
        x: color.x * 255.0,
        y: color.y * 255.0,
        z: color.z * 255.0,
        w: color.w * 255.0,
    }
}

/// Clamp a byte-range color into vertex bytes.
#[must_use]
pub fn color_bytes(color: Vec4) -> [u8; 4] {
    [
        color.x.clamp(0.0, 255.0) as u8,
        color.y.clamp(0.0, 255.0) as u8,
        color.z.clamp(0.0, 255.0) as u8,
        color.w.clamp(0.0, 255.0) as u8,
    ]
}

/// Look up a slice element with a labeled range error.
pub fn at<'a, T>(values: &'a [T], index: usize, label: &str) -> Result<&'a T, RenderError> {
    values.get(index).ok_or_else(|| RenderError::BadBatch {
        index,
        detail: format!("missing {label}"),
    })
}

/// Count the frames of timed-frame groups (test helper mirrors donor `frames()`).
pub fn timed_frame_list<T>(value: &TimedFrames<T>) -> Vec<&T> {
    match value {
        TimedFrames::Single(frame) => vec![frame],
        TimedFrames::Group(frames) => frames.iter().map(|item| &item.frame).collect(),
    }
}

/// Display name of an image source (re-exported helper for selections).
#[must_use]
pub fn image_source_name(source: &ImageSource) -> &str {
    source.display_name()
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_core::math::{vec3, vec4};

    #[test]
    fn byte_color_scales_to_255() {
        let scaled = byte_color(vec4(1.0, 0.5, 0.0, 0.25));
        assert_eq!(scaled, vec4(255.0, 127.5, 0.0, 63.75));
        assert_eq!(color_bytes(scaled), [255, 127, 0, 63]);
    }

    #[test]
    fn color_bytes_clamp() {
        assert_eq!(color_bytes(vec4(-1.0, 300.0, 128.0, 255.0)), [0, 255, 128, 255]);
    }

    #[test]
    fn model_image_reports_fullbright_range() {
        let palette = ModelPalette {
            colors: vec![0; 768],
            source: "pics/palette.lmp".to_string(),
        };
        let level = ImageLevel {
            width: 4,
            height: 4,
            pixels: vec![0; 16],
        };
        let image = model_image("skin", level, None, true, &palette, None);
        match image {
            RenderImage::Indexed8 {
                fullbright,
                transparency,
                ..
            } => {
                assert_eq!(fullbright, Some((224, 255)));
                assert_eq!(transparency, PaletteTransparency::Opaque);
            }
            _ => panic!("expected indexed image"),
        }
    }

    #[test]
    fn model_image_trims_fullbright_at_fence_index() {
        let palette = ModelPalette {
            colors: vec![0; 768],
            source: "pics/palette.lmp".to_string(),
        };
        let level = ImageLevel {
            width: 2,
            height: 2,
            pixels: vec![0; 4],
        };
        let image = model_image("spr", level, Some(255), true, &palette, None);
        match image {
            RenderImage::Indexed8 { fullbright, .. } => {
                assert_eq!(fullbright, Some((224, 254)));
            }
            _ => panic!("expected indexed image"),
        }
    }

    #[test]
    fn at_reports_missing_index() {
        let values = [1, 2];
        assert!(at(&values, 1, "thing").is_ok());
        let error = at(&values, 5, "thing").expect_err("must fail");
        assert!(matches!(error, RenderError::BadBatch { index: 5, .. }));
    }

    #[test]
    fn brush_model_counts_one_frame() {
        assert_eq!(SceneModel::BrushModel.frame_count(), 1);
    }

    #[test]
    fn identity_transform_is_unit() {
        let transform = EntityTransform::identity();
        assert_eq!(transform.origin, vec3(0.0, 0.0, 0.0));
        assert_eq!(transform.scale, vec3(1.0, 1.0, 1.0));
    }

    #[test]
    fn entity_flags_report_bits() {
        assert_eq!(EntityFlags::Q2 { bits: 7 }.bits(), 7);
    }
}
