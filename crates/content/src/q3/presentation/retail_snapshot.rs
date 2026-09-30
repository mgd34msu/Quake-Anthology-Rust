//! Quake III presentation: retail snapshot.
//!
//! Donor provenance: `src/content/q3/presentation/retail-snapshot.ts`.

use qa_core::math::{vec3, vec4, Axis, Vec3, Vec4};

// Intra-group imports: sibling modules split from the same flat port.
use crate::q3::base::shared::entity_state::*;
use crate::q3::base::shared::player_state::*;

// ---------------------------------------------------------------------------
// Snapshot and retail snapshot (retail-snapshot.ts; transport mirror of
// network/q3/server-message.ts Snapshot)
// ---------------------------------------------------------------------------

/// Server snapshot (`Snapshot`).
#[derive(Debug, Clone)]
pub struct Snapshot {
    /// Message number.
    pub message_number: i32,
    /// Server time.
    pub server_time: i32,
    /// Delta number.
    pub delta_number: i32,
    /// Flags.
    pub flags: i32,
    /// Server command number.
    pub server_command_number: i32,
    /// Parse-entity base number.
    pub parse_entities_number: i32,
    /// Area mask.
    pub area_mask: [u8; 32],
    /// Player state.
    pub player_state: PlayerState,
    /// Entities in source append order.
    pub entities: Vec<EntityState>,
}

/// Owned retail snapshot (`RetailSnapshot`).
pub type RetailSnapshot = Snapshot;

/// Deep-copy transport into owned retail state (`retailSnapshot`).
#[must_use]
pub fn retail_snapshot(source: &Snapshot) -> RetailSnapshot {
    source.clone()
}

// ---------------------------------------------------------------------------
// Media handles (SIBLING-MIRROR of the ref-entity/audio handle substance)
// ---------------------------------------------------------------------------

/// Decoded sound handle (`PcmSound`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PcmSound {
    /// Sound id.
    pub id: u32,
}

impl PcmSound {
    /// New sound handle.
    #[must_use]
    pub const fn new(id: u32) -> Self {
        Self { id }
    }
}

/// Registered model handle (`SceneModel`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SceneModel {
    /// Default (missing) model.
    Default,
    /// Loaded model.
    Loaded {
        /// Resource id.
        id: u32,
    },
}

impl SceneModel {
    /// Whether this is the default model.
    #[must_use]
    pub const fn is_default(&self) -> bool {
        matches!(self, Self::Default)
    }

    /// Resource id, if loaded.
    #[must_use]
    pub const fn resource_id(&self) -> Option<u32> {
        match self {
            Self::Default => None,
            Self::Loaded { id } => Some(*id),
        }
    }
}

/// Default model (`DEFAULT_MODEL`).
#[must_use]
pub const fn default_model() -> SceneModel {
    SceneModel::Default
}

/// Skin surface binding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkinSurface {
    /// Surface name.
    pub name: String,
    /// Shader name.
    pub shader: String,
}

/// Registered skin handle (`SceneSkin`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SceneSkin {
    /// Skin id.
    pub id: u32,
    /// Surface bindings.
    pub surfaces: Vec<SkinSurface>,
}

/// Registered shader/picture handle (`SceneShader` / `MaterialPicture`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SceneShader {
    /// Shader id.
    pub id: u32,
    /// Shader name.
    pub name: String,
    /// Renderer material order.
    pub material_order: i32,
}

/// Material picture (same handle substance as `SceneShader` here).
pub type MaterialPicture = SceneShader;

/// Model entity for scene submission (`RefModelEntity`).
#[derive(Debug, Clone, PartialEq)]
pub struct RefModelEntity {
    /// Origin.
    pub origin: Vec3,
    /// Old origin.
    pub old_origin: Vec3,
    /// Lighting origin.
    pub lighting_origin: Vec3,
    /// Axis.
    pub axis: Axis,
    /// Model.
    pub model: SceneModel,
    /// Custom skin.
    pub custom_skin: Option<SceneSkin>,
    /// Custom shader.
    pub custom_shader: Option<SceneShader>,
    /// Shadow plane.
    pub shadow_plane: f32,
    /// Render flags.
    pub render_flags: i32,
    /// Old frame.
    pub old_frame: i32,
    /// Frame.
    pub frame: i32,
    /// Back lerp.
    pub back_lerp: f32,
    /// Shader color.
    pub shader_rgba: Vec4,
    /// Shader texture coordinate.
    pub shader_tex_coord: [f32; 2],
}

/// Sprite entity for scene submission.
#[derive(Debug, Clone, PartialEq)]
pub struct RefSpriteEntity {
    /// Origin.
    pub origin: Vec3,
    /// Custom shader.
    pub custom_shader: Option<SceneShader>,
    /// Radius.
    pub radius: f32,
    /// Render flags.
    pub render_flags: i32,
    /// Shader color.
    pub shader_rgba: Vec4,
}

/// Scene entity (`RefEntity`).
#[derive(Debug, Clone, PartialEq)]
pub enum RefEntity {
    /// Model entity.
    Model(RefModelEntity),
    /// Sprite entity.
    Sprite(RefSpriteEntity),
}

/// New model entity (`createModelEntity`).
#[must_use]
pub fn create_model_entity() -> RefModelEntity {
    create_model_entity_with(default_model())
}

/// New model entity with a model.
#[must_use]
pub fn create_model_entity_with(model: SceneModel) -> RefModelEntity {
    RefModelEntity {
        origin: vec3(0.0, 0.0, 0.0),
        old_origin: vec3(0.0, 0.0, 0.0),
        lighting_origin: vec3(0.0, 0.0, 0.0),
        axis: [vec3(0.0, 0.0, 0.0), vec3(0.0, 0.0, 0.0), vec3(0.0, 0.0, 0.0)],
        model,
        custom_skin: None,
        custom_shader: None,
        shadow_plane: 0.0,
        render_flags: 0,
        old_frame: 0,
        frame: 0,
        back_lerp: 0.0,
        shader_rgba: vec4(255.0, 255.0, 255.0, 255.0),
        shader_tex_coord: [0.0, 0.0],
    }
}

/// New sprite entity (`createSpriteEntity`).
#[must_use]
pub fn create_sprite_entity() -> RefSpriteEntity {
    RefSpriteEntity {
        origin: vec3(0.0, 0.0, 0.0),
        custom_shader: None,
        radius: 0.0,
        render_flags: 0,
        shader_rgba: vec4(255.0, 255.0, 255.0, 255.0),
    }
}

/// Scene polygon vertex (`RefPolyVertex`).
#[derive(Debug, Clone, PartialEq)]
pub struct RefPolyVertex {
    /// Position.
    pub position: Vec3,
    /// Texture coordinate.
    pub tex_coord: [f32; 2],
    /// Lit color.
    pub color: Vec4,
}

/// Scene polygon (`RefPoly`).
#[derive(Debug, Clone, PartialEq)]
pub struct RefPoly {
    /// Shader.
    pub shader: Option<SceneShader>,
    /// Vertices.
    pub vertices: Vec<RefPolyVertex>,
}

/// Dynamic light (`DynamicLight`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DynamicLight {
    /// Origin.
    pub origin: Vec3,
    /// Radius.
    pub radius: f32,
    /// Color.
    pub color: Vec3,
    /// Additive blend.
    pub additive: bool,
}

/// Lighting sample (`LightingSample`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LightingSample {
    /// Ambient light in source byte units.
    pub ambient_light: Vec3,
    /// Directed light in source byte units.
    pub directed_light: Vec3,
    /// Light direction.
    pub light_dir: Vec3,
}

/// View definition (`Refdef`).
#[derive(Debug, Clone, PartialEq)]
pub struct Refdef {
    /// X.
    pub x: i32,
    /// Y.
    pub y: i32,
    /// Width.
    pub width: i32,
    /// Height.
    pub height: i32,
    /// Horizontal field of view.
    pub fov_x: f32,
    /// Vertical field of view.
    pub fov_y: f32,
    /// View origin.
    pub view_origin: Vec3,
    /// View axis.
    pub view_axis: Axis,
    /// Time.
    pub time: i32,
    /// Render flags.
    pub render_flags: i32,
    /// Area mask.
    pub area_mask: [u8; 32],
    /// Render text rows.
    pub text: [String; 8],
}

/// New view definition (`createRefdef`).
#[must_use]
pub fn create_refdef() -> Refdef {
    Refdef {
        x: 0,
        y: 0,
        width: 0,
        height: 0,
        fov_x: 0.0,
        fov_y: 0.0,
        view_origin: vec3(0.0, 0.0, 0.0),
        view_axis: [vec3(0.0, 0.0, 0.0), vec3(0.0, 0.0, 0.0), vec3(0.0, 0.0, 0.0)],
        time: 0,
        render_flags: 0,
        area_mask: [0; 32],
        text: std::array::from_fn(|_| String::new()),
    }
}
