//! Quake III presentation: ref entity.
//!
//! Donor provenance: `src/content/q3/presentation/ref-entity.ts`.

use crate::md3::Md3Model;
use crate::md5::Md5AnimationFrame;
use qa_core::math::{vec2, vec3, vec4, Axis, Bounds, Vec2, Vec3, Vec4};
use std::fmt;

// Intra-group imports: sibling modules split from the same flat port.

// ---------------------------------------------------------------------------
// ref-entity.ts
// ---------------------------------------------------------------------------

/// Decoded model payload (`DecodedModel`, minimal mirror).
#[derive(Debug, Clone, PartialEq)]
pub enum Q3DecodedModel {
    /// Quake III MD3.
    Md3(Md3Model),
    /// MD5 with joint names and animation frames.
    Md5(PresentMd5),
    /// Model exposing `bounds` directly.
    Bounded {
        /// Bounds.
        bounds: Bounds,
    },
    /// Brush model referencing world submodels.
    Brush {
        /// World submodel bounds.
        models: Vec<Bounds>,
        /// Selected submodel.
        index: usize,
    },
    /// Framed model exposing `frames[0].bounds`.
    Framed {
        /// Frame bounds.
        frames: Vec<Bounds>,
    },
}

/// MD5 presentation payload.
#[derive(Debug, Clone, PartialEq)]
pub struct PresentMd5 {
    /// Joint names in pose order.
    pub joint_names: Vec<String>,
    /// Animation frames.
    pub frames: Vec<Md5AnimationFrame>,
}

/// Default scene model (`SceneDefaultModel`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SceneDefaultModel {
    /// Always `*default`.
    pub path: String,
}

/// Loaded scene model (`SceneLoadedModel`).
#[derive(Debug, Clone, PartialEq)]
pub struct SceneLoadedModel {
    /// Path.
    pub path: String,
    /// Decoded model.
    pub model: Q3DecodedModel,
    /// Resource.
    pub resource: PresentResource,
}

/// Inline scene model (`SceneInlineModel`).
#[derive(Debug, Clone, PartialEq)]
pub struct SceneInlineModel {
    /// Path.
    pub path: String,
    /// Index.
    pub index: usize,
    /// Geometry.
    pub geometry: PresentWorld,
    /// Resource.
    pub resource: PresentResource,
    /// Bounds.
    pub bounds: Bounds,
}

/// Scene model (`SceneModel`).
#[derive(Debug, Clone, PartialEq)]
pub enum SceneModel {
    /// Default placeholder.
    Default(SceneDefaultModel),
    /// Loaded model.
    Loaded(SceneLoadedModel),
    /// Inline brush model.
    Inline(SceneInlineModel),
}

impl SceneModel {
    /// Default model.
    #[must_use]
    pub fn default_model() -> Self {
        Self::Default(SceneDefaultModel {
            path: "*default".to_string(),
        })
    }

    /// Whether this is the default placeholder.
    #[must_use]
    pub fn is_default(&self) -> bool {
        matches!(self, Self::Default(_))
    }
}

/// Default model constant (`DEFAULT_MODEL`).
#[must_use]
pub fn default_model() -> SceneModel {
    SceneModel::default_model()
}

/// Scene shader (`SceneShader`).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SceneShader {
    /// Name.
    pub name: String,
}

impl SceneShader {
    /// New shader.
    #[must_use]
    pub fn new(name: impl Into<String>) -> Self {
        Self { name: name.into() }
    }
}

/// Scene skin (`SceneSkin`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SceneSkin {
    /// Path.
    pub path: String,
    /// Surfaces.
    pub surfaces: Vec<SkinMapping>,
}

/// Minimum light render flag.
pub const RF_MINLIGHT: i32 = 1;

/// Third-person render flag.
pub const RF_THIRD_PERSON: i32 = 2;

/// First-person render flag.
pub const RF_FIRST_PERSON: i32 = 4;

/// Depth-hack render flag.
pub const RF_DEPTHHACK: i32 = 8;

/// No-shadow render flag.
pub const RF_NOSHADOW: i32 = 64;

/// Lighting-origin render flag.
pub const RF_LIGHTING_ORIGIN: i32 = 128;

/// Shadow-plane render flag.
pub const RF_SHADOW_PLANE: i32 = 256;

/// Wrap-frames render flag.
pub const RF_WRAP_FRAMES: i32 = 512;

/// Shading fields shared by shaded entities (`ShadedEntity`).
#[derive(Debug, Clone, PartialEq)]
pub struct ShadedFields {
    /// Render flags.
    pub render_flags: i32,
    /// Custom shader.
    pub custom_shader: Option<SceneShader>,
    /// Source byte channels, including zero defaults.
    pub shader_rgba: Vec4,
    /// Shader texture coordinate.
    pub shader_tex_coord: Vec2,
    /// Seconds subtracted from the scene shader clock.
    pub shader_time: f32,
}

impl Default for ShadedFields {
    fn default() -> Self {
        Self {
            render_flags: 0,
            custom_shader: None,
            shader_rgba: vec4(0.0, 0.0, 0.0, 0.0),
            shader_tex_coord: vec2(0.0, 0.0),
            shader_time: 0.0,
        }
    }
}

/// Model reference entity (`RefModelEntity`).
#[derive(Debug, Clone, PartialEq)]
pub struct RefModelEntity {
    /// Shading.
    pub shading: ShadedFields,
    /// Model.
    pub model: SceneModel,
    /// Origin.
    pub origin: Vec3,
    /// Old origin.
    pub old_origin: Vec3,
    /// Axis.
    pub axis: Axis,
    /// Non-normalized axes.
    pub non_normalized_axes: bool,
    /// Lighting origin.
    pub lighting_origin: Vec3,
    /// Shadow plane.
    pub shadow_plane: f32,
    /// Frame.
    pub frame: i32,
    /// Old frame.
    pub old_frame: i32,
    /// Back lerp.
    pub back_lerp: f32,
    /// Skin number.
    pub skin_num: i32,
    /// Custom skin.
    pub custom_skin: Option<SceneSkin>,
}

/// Sprite reference entity (`RefSpriteEntity`).
#[derive(Debug, Clone, PartialEq)]
pub struct RefSpriteEntity {
    /// Shading.
    pub shading: ShadedFields,
    /// Origin.
    pub origin: Vec3,
    /// Radius.
    pub radius: f32,
    /// Rotation.
    pub rotation: f32,
}

/// Beam reference entity (`RefBeamEntity`).
#[derive(Debug, Clone, PartialEq)]
pub struct RefBeamEntity {
    /// Shading.
    pub shading: ShadedFields,
    /// Origin.
    pub origin: Vec3,
    /// Old origin.
    pub old_origin: Vec3,
    /// Axis.
    pub axis: Axis,
    /// Radius (used for sprite-fog selection; beam geometry is radius four).
    pub radius: f32,
}

/// Rail-core reference entity (`RefRailCoreEntity`).
#[derive(Debug, Clone, PartialEq)]
pub struct RefRailCoreEntity {
    /// Shading.
    pub shading: ShadedFields,
    /// Origin.
    pub origin: Vec3,
    /// Old origin.
    pub old_origin: Vec3,
    /// Radius.
    pub radius: f32,
}

/// Rail-rings reference entity (`RefRailRingsEntity`).
#[derive(Debug, Clone, PartialEq)]
pub struct RefRailRingsEntity {
    /// Shading.
    pub shading: ShadedFields,
    /// Origin.
    pub origin: Vec3,
    /// Old origin.
    pub old_origin: Vec3,
    /// Radius.
    pub radius: f32,
}

/// Lightning reference entity (`RefLightningEntity`).
#[derive(Debug, Clone, PartialEq)]
pub struct RefLightningEntity {
    /// Shading.
    pub shading: ShadedFields,
    /// Origin.
    pub origin: Vec3,
    /// Old origin.
    pub old_origin: Vec3,
    /// Radius.
    pub radius: f32,
}

/// Portal reference entity (`RefPortalEntity`).
#[derive(Debug, Clone, PartialEq)]
pub struct RefPortalEntity {
    /// Render flags.
    pub render_flags: i32,
    /// Origin.
    pub origin: Vec3,
    /// Old origin.
    pub old_origin: Vec3,
    /// Axis.
    pub axis: Axis,
    /// Frame.
    pub frame: i32,
    /// Old frame.
    pub old_frame: i32,
    /// Skin number.
    pub skin_num: i32,
}

/// Reference entity (`RefEntity`).
#[derive(Debug, Clone, PartialEq)]
#[allow(clippy::large_enum_variant)]
pub enum RefEntity {
    /// Model.
    Model(RefModelEntity),
    /// Sprite.
    Sprite(RefSpriteEntity),
    /// Beam.
    Beam(RefBeamEntity),
    /// Rail core.
    RailCore(RefRailCoreEntity),
    /// Rail rings.
    RailRings(RefRailRingsEntity),
    /// Lightning.
    Lightning(RefLightningEntity),
    /// Portal surface.
    Portal(RefPortalEntity),
}

impl RefEntity {
    /// Entity kind name.
    #[must_use]
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Model(_) => "model",
            Self::Sprite(_) => "sprite",
            Self::Beam(_) => "beam",
            Self::RailCore(_) => "rail-core",
            Self::RailRings(_) => "rail-rings",
            Self::Lightning(_) => "lightning",
            Self::Portal(_) => "portal-surface",
        }
    }

    /// Origin for light placement (all but portal expose `origin`).
    #[must_use]
    pub fn origin(&self) -> Vec3 {
        match self {
            Self::Model(entity) => entity.origin,
            Self::Sprite(entity) => entity.origin,
            Self::Beam(entity) => entity.origin,
            Self::RailCore(entity) => entity.origin,
            Self::RailRings(entity) => entity.origin,
            Self::Lightning(entity) => entity.origin,
            Self::Portal(entity) => entity.origin,
        }
    }
}

/// Polygon vertex (`RefPolyVertex`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RefPolyVertex {
    /// Position.
    pub position: Vec3,
    /// Texture coordinate.
    pub tex_coord: Vec2,
    /// Source `polyVert_t.modulate` byte channels.
    pub color: Vec4,
}

/// Reference polygon (`RefPoly`).
#[derive(Debug, Clone, PartialEq)]
pub struct RefPoly {
    /// Shader.
    pub shader: Option<SceneShader>,
    /// Vertices.
    pub vertices: Vec<RefPolyVertex>,
}

/// Source model handle: typed model or numeric VM handle.
#[derive(Debug, Clone, PartialEq)]
pub enum ModelRef {
    /// Loaded model.
    Loaded(SceneModel),
    /// Numeric handle.
    Handle(i32),
}

/// Source shader handle: typed shader or numeric VM handle.
#[derive(Debug, Clone, PartialEq)]
pub enum ShaderRef {
    /// Loaded shader.
    Loaded(SceneShader),
    /// Numeric handle.
    Handle(i32),
}

/// Source skin handle: typed skin or numeric VM handle.
#[derive(Debug, Clone, PartialEq)]
pub enum SkinRef {
    /// Loaded skin.
    Loaded(SceneSkin),
    /// Numeric handle.
    Handle(i32),
}

/// Source model entity with numeric-capable handles (`SourceHandles`).
#[derive(Debug, Clone, PartialEq)]
pub struct SourceModelEntity {
    /// Render flags.
    pub render_flags: i32,
    /// Custom shader or handle.
    pub custom_shader: Option<ShaderRef>,
    /// Shader RGBA.
    pub shader_rgba: Vec4,
    /// Shader texture coordinate.
    pub shader_tex_coord: Vec2,
    /// Shader time.
    pub shader_time: f32,
    /// Model or handle.
    pub model: ModelRef,
    /// Origin.
    pub origin: Vec3,
    /// Old origin.
    pub old_origin: Vec3,
    /// Axis.
    pub axis: Axis,
    /// Non-normalized axes.
    pub non_normalized_axes: bool,
    /// Lighting origin.
    pub lighting_origin: Vec3,
    /// Shadow plane.
    pub shadow_plane: f32,
    /// Frame.
    pub frame: i32,
    /// Old frame.
    pub old_frame: i32,
    /// Back lerp.
    pub back_lerp: f32,
    /// Skin number.
    pub skin_num: i32,
    /// Custom skin or handle.
    pub custom_skin: Option<SkinRef>,
    /// Radius (poly records).
    pub radius: f32,
    /// Rotation (poly records).
    pub rotation: f32,
}

/// Source polygon record (`SourceRefEntityRecord` with kind `poly`).
#[derive(Debug, Clone, PartialEq)]
pub struct SourcePolyRecord {
    /// Fields.
    pub fields: SourceModelEntity,
}

/// Source reference entity (`SourceRefEntity`).
#[derive(Debug, Clone, PartialEq)]
pub enum SourceRefEntity {
    /// Model with handles.
    Model(SourceModelEntity),
    /// Sprite with handles.
    Sprite(SourceSpriteEntity),
    /// Beam with handles.
    Beam(SourceBeamEntity),
    /// Rail core with handles.
    RailCore(SourceRailEntity),
    /// Rail rings with handles.
    RailRings(SourceRailEntity),
    /// Lightning with handles.
    Lightning(SourceRailEntity),
    /// Portal (no handles).
    Portal(RefPortalEntity),
    /// Full poly record.
    Poly(SourcePolyRecord),
}

/// Source sprite entity with numeric-capable handles.
#[derive(Debug, Clone, PartialEq)]
pub struct SourceSpriteEntity {
    /// Render flags.
    pub render_flags: i32,
    /// Custom shader or handle.
    pub custom_shader: Option<ShaderRef>,
    /// Shader RGBA.
    pub shader_rgba: Vec4,
    /// Shader texture coordinate.
    pub shader_tex_coord: Vec2,
    /// Shader time.
    pub shader_time: f32,
    /// Origin.
    pub origin: Vec3,
    /// Radius.
    pub radius: f32,
    /// Rotation.
    pub rotation: f32,
}

/// Source beam entity with numeric-capable handles.
#[derive(Debug, Clone, PartialEq)]
pub struct SourceBeamEntity {
    /// Render flags.
    pub render_flags: i32,
    /// Custom shader or handle.
    pub custom_shader: Option<ShaderRef>,
    /// Shader RGBA.
    pub shader_rgba: Vec4,
    /// Shader texture coordinate.
    pub shader_tex_coord: Vec2,
    /// Shader time.
    pub shader_time: f32,
    /// Origin.
    pub origin: Vec3,
    /// Old origin.
    pub old_origin: Vec3,
    /// Axis.
    pub axis: Axis,
    /// Radius.
    pub radius: f32,
}

/// Source rail/lightning entity with numeric-capable handles.
#[derive(Debug, Clone, PartialEq)]
pub struct SourceRailEntity {
    /// Render flags.
    pub render_flags: i32,
    /// Custom shader or handle.
    pub custom_shader: Option<ShaderRef>,
    /// Shader RGBA.
    pub shader_rgba: Vec4,
    /// Shader texture coordinate.
    pub shader_tex_coord: Vec2,
    /// Shader time.
    pub shader_time: f32,
    /// Origin.
    pub origin: Vec3,
    /// Old origin.
    pub old_origin: Vec3,
    /// Radius.
    pub radius: f32,
}

/// Admitted reference entity (`Q3AdmittedRefEntity`).
#[derive(Debug, Clone, PartialEq)]
pub enum Q3AdmittedRefEntity {
    /// Typed entity.
    Entity(RefEntity),
    /// Full poly record.
    Poly(SourcePolyRecord),
}

pub(crate) fn zero_vec3() -> Vec3 {
    vec3(0.0, 0.0, 0.0)
}

pub(crate) fn zero_axis() -> Axis {
    [zero_vec3(), zero_vec3(), zero_vec3()]
}

/// Create a model entity (`createModelEntity`).
#[must_use]
pub fn create_model_entity(model: SceneModel) -> RefModelEntity {
    RefModelEntity {
        shading: ShadedFields::default(),
        model,
        origin: zero_vec3(),
        old_origin: zero_vec3(),
        axis: zero_axis(),
        non_normalized_axes: false,
        lighting_origin: zero_vec3(),
        shadow_plane: 0.0,
        frame: 0,
        old_frame: 0,
        back_lerp: 0.0,
        skin_num: 0,
        custom_skin: None,
    }
}

/// Create a sprite entity (`createSpriteEntity`).
#[must_use]
pub fn create_sprite_entity() -> RefSpriteEntity {
    RefSpriteEntity {
        shading: ShadedFields::default(),
        origin: zero_vec3(),
        radius: 0.0,
        rotation: 0.0,
    }
}

/// Create a beam entity (`createBeamEntity`).
#[must_use]
pub fn create_beam_entity() -> RefBeamEntity {
    RefBeamEntity {
        shading: ShadedFields::default(),
        origin: zero_vec3(),
        old_origin: zero_vec3(),
        axis: zero_axis(),
        radius: 0.0,
    }
}

/// Create a rail-core entity (`createRailCoreEntity`).
#[must_use]
pub fn create_rail_core_entity() -> RefRailCoreEntity {
    RefRailCoreEntity {
        shading: ShadedFields::default(),
        origin: zero_vec3(),
        old_origin: zero_vec3(),
        radius: 0.0,
    }
}

/// Create a rail-rings entity (`createRailRingsEntity`).
#[must_use]
pub fn create_rail_rings_entity() -> RefRailRingsEntity {
    RefRailRingsEntity {
        shading: ShadedFields::default(),
        origin: zero_vec3(),
        old_origin: zero_vec3(),
        radius: 0.0,
    }
}

/// Create a lightning entity (`createLightningEntity`).
#[must_use]
pub fn create_lightning_entity() -> RefLightningEntity {
    RefLightningEntity {
        shading: ShadedFields::default(),
        origin: zero_vec3(),
        old_origin: zero_vec3(),
        radius: 0.0,
    }
}

/// Create a portal entity (`createPortalEntity`).
#[must_use]
pub fn create_portal_entity() -> RefPortalEntity {
    RefPortalEntity {
        render_flags: 0,
        origin: zero_vec3(),
        old_origin: zero_vec3(),
        axis: zero_axis(),
        frame: 0,
        old_frame: 0,
        skin_num: 0,
    }
}

/// Copy a reference entity (`copyRefEntity`; resource identity retained).
#[must_use]
pub fn copy_ref_entity(entity: &RefEntity) -> RefEntity {
    entity.clone()
}

/// Copy an admitted reference entity (`copyRefEntity` overload).
#[must_use]
pub fn copy_admitted_ref_entity(entity: &Q3AdmittedRefEntity) -> Q3AdmittedRefEntity {
    entity.clone()
}

/// Copy a source reference entity (`copySourceRefEntity`).
#[must_use]
pub fn copy_source_ref_entity(entity: &SourceRefEntity) -> SourceRefEntity {
    entity.clone()
}

/// Copy a reference polygon (`copyRefPoly`).
#[must_use]
pub fn copy_ref_poly(poly: &RefPoly) -> RefPoly {
    poly.clone()
}

/// Skin surface mapping.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkinMapping {
    /// Surface name.
    pub name: String,
    /// Shader name.
    pub shader: String,
}

/// World handle for inline models (`DecodedWorld`, minimal mirror).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PresentWorld {
    /// Name.
    pub name: String,
}

impl PresentWorld {
    /// New handle.
    #[must_use]
    pub fn new(name: impl Into<String>) -> Self {
        Self { name: name.into() }
    }
}

/// Resolved resource reference (`ResolvedResourceReference`, minimal mirror).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PresentResource {
    /// Path.
    pub path: String,
}

impl PresentResource {
    /// New reference.
    #[must_use]
    pub fn new(path: impl Into<String>) -> Self {
        Self { path: path.into() }
    }
}

/// Failure of a presentation operation (`RangeError`, `CommonError("drop")`,
/// or a plain `Error` in the donors).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PresentError {
    /// Machine-readable class.
    pub kind: PresentErrorKind,
    /// Human-readable message.
    pub message: String,
}

/// Error class.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PresentErrorKind {
    /// Out-of-range input (`RangeError`).
    Range,
    /// Dropped client (`CommonError("drop")`).
    Drop,
    /// Invalid state or bug (`Error`).
    State,
}

impl PresentError {
    /// Range error.
    #[must_use]
    pub fn range(message: impl Into<String>) -> Self {
        Self {
            kind: PresentErrorKind::Range,
            message: message.into(),
        }
    }

    /// Drop error.
    #[must_use]
    pub fn drop(message: impl Into<String>) -> Self {
        Self {
            kind: PresentErrorKind::Drop,
            message: message.into(),
        }
    }

    /// State error.
    #[must_use]
    pub fn state(message: impl Into<String>) -> Self {
        Self {
            kind: PresentErrorKind::State,
            message: message.into(),
        }
    }
}
impl From<crate::q3::presentation::state::PresentClientError> for PresentError {
    /// Bridge the client group error into the scene group error.
    ///
    /// Both port the same donor failures (`RangeError`, `CommonError("drop")`,
    /// `Error`); scene code calling state.ts-owned accessors propagates them
    /// unchanged through this conversion.
    fn from(error: crate::q3::presentation::state::PresentClientError) -> Self {
        use crate::q3::presentation::state::PresentClientError;
        match error {
            PresentClientError::Drop(message) => Self::drop(message),
            PresentClientError::Range(message) => Self::range(message),
            PresentClientError::State(message) => Self::state(message),
        }
    }
}

impl fmt::Display for PresentError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}: {}", self.kind, self.message)
    }
}

impl std::error::Error for PresentError {}

/// Presentation result.
pub type PresentResult<T> = Result<T, PresentError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entity_factories() {
        let model = create_model_entity(default_model());
        assert_eq!(model.shading.shader_rgba, vec4(0.0, 0.0, 0.0, 0.0));
        assert_eq!(RefEntity::Model(model.clone()).kind(), "model");
        assert_eq!(copy_ref_entity(&RefEntity::Model(model)).kind(), "model");
        assert_eq!(create_sprite_entity().radius, 0.0);
        assert_eq!(create_beam_entity().radius, 0.0);
        assert_eq!(create_portal_entity().frame, 0);
        let poly = RefPoly {
            shader: Some(SceneShader::new("s")),
            vertices: vec![RefPolyVertex {
                position: zero_vec3(),
                tex_coord: vec2(0.0, 0.0),
                color: vec4(1.0, 2.0, 3.0, 4.0),
            }],
        };
        assert_eq!(copy_ref_poly(&poly), poly);
    }
}
