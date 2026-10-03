//! World scene assembly from presentations, characters, and bodies.
//!
//! Port of Quake-Anthology-TS `src/app/bootstrap/presentation-scene.ts`
//! (`seatModelVisible`, `ApplicationWorldScene`). Characters, weapons, entities,
//! options, operations, and admission are the ported foundation, client, and math
//! helpers; the model assets (`./assets.ts`, out of scope), held weapons
//! ([`held_weapon`](super::held_weapon), [`native_held_weapon`](super::native_held_weapon)),
//! and body queries ([`component_bodies`](super::component_bodies)) arrive through the
//! [`SceneModelAssets`], [`SceneHeldWeapons`], and [`SceneBodies`] seams, and the
//! model/world renderers through [`SceneRenderer`] and [`SceneWorld`]. The donor's
//! async loads are sync through the host. Documented
//! folds: the shadow plane has no render counterpart (dropped like the sibling media
//! port); entity actor and opacity ride alongside the ported entity; body time and
//! integer fields truncate toward zero; the nine-parameter prepare call bundles into
//! [`ScenePrepareInput`].

use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use qa_client::materials::lighting::Q2LightStyle;
use qa_client::render::scene::material_registrations::{RegisteredSceneMaterial, ShaderRegistration};
use qa_client::render::scene::models::renderer::ModelShadowCaster;
use qa_client::render::scene::models::transform::{attach_scene_entity, model_attachment_tag};
use qa_client::render::scene::models::types::{
    CustomSkinEntry, EntityFlags, EntityTransform, IndexedModelSkin, ModelAttachment, ModelBeamOptions, ModelResource,
    ModelSkinningFrame, ModelSourceOptions, PlayerColors, SceneEntity, SceneModel, ScenePose,
};
use qa_client::render::scene::submissions::{
    create_source_scene_order, scene_model_batches, sequence_draw_group, SceneGroup, SceneGroupOrder, SceneModelGroup,
    SceneOperation, SequencePhase, SourceEntityOrder,
};
use qa_client::render::scene::world::{create_world_surface_admission, InlineModel, PreparedWorldView, WorldViewInput};
use qa_client::render::types::{DrawBatch, RenderOperation, RendererImage, SourceTime as RenderSourceTime};
use qa_client::render::{LightProfile, LightShadow, SceneLight};
use qa_client::view::{CameraClip, ModelTransform as ViewModelTransform, SceneCamera};
use qa_content::contract::{
    same_presentation_owner, ContentDigest, ContentId, GameFamily, ModelTransform, PresentationOwner,
    ResolvedResourceReference,
};
use qa_content::md5::{DecodedMd5Model, Md5Animation, SkinSelection};
use qa_content::q3::foundation::assets::Q3CharacterAssets;
use qa_content::q3::foundation::player_pose::qvm_angle_vectors;
use qa_content::q3::foundation::presentation::{
    ModelSourceOptions as FoundationModelSourceOptions, Q3Attachment, Q3CharacterPass, Q3CharacterPresenter,
    Q3CharacterRenderOptions, Q3CharacterView, Q3ModelPose, Q3PassOptions, Q3PresentedModel, Q3SceneEntity,
};
use qa_core::identity::ActorId;
use qa_core::math::{
    add3, angles_to_axis, length3, normalize3_or_zero, scale3, sub3, vec3, vec4, vector_to_angles, Vec2, Vec3, Vec4,
};
use qa_core::time::SourceTime;
use qa_world::session::WorldSnapshot;
use thiserror::Error;

use super::presentation_state::{
    OwnerLifecycle, Q1PresentationEvent, Q2PresentationEvent, SimulationPresentationEvent, SourcePresentationEvent,
};
use super::q3_selected_weapon::{
    Q3SelectedWeaponError, Q3WeaponAnchor, Q3WeaponAssets, Q3WeaponAttachment, Q3WeaponCharacter, Q3WeaponModelAsset,
    Q3WeaponSource, Q3WeaponView, SelectedQ3WeaponPresenter,
};
use super::weapon_view::{weapon_view_origin, WeaponViewSource};

/// Scene assembly failure, with donor messages.
#[derive(Debug, Error)]
pub enum SceneError {
    /// A brush model has no prepared scene.
    #[error("Brush model {0} has no prepared scene")]
    BrushScene(String),
    /// A grapple cable exceeds the model segment limit.
    #[error("Source grapple cable exceeds the model segment limit")]
    CableLimit,
    /// A selected weapon has no source held model.
    #[error("Selected weapon {0} has no source held model")]
    HeldModel(String),
    /// A source torso has no weapon attachment.
    #[error("Original source torso has no weapon attachment")]
    TorsoAttachment,
    /// A scene host failed.
    #[error("Scene host failed: {0}")]
    Host(String),
    /// Selected weapon failure.
    #[error(transparent)]
    Weapon(#[from] Q3SelectedWeaponError),
    /// Character presentation failure.
    #[error(transparent)]
    Presentation(#[from] qa_content::q3::foundation::presentation::PresentationError),
}

/// Whether a seat sees a source model (donor `seatModelVisible`).
pub fn seat_model_visible(viewer: Option<&ActorId>, actor: &ActorId, view_weapon: bool) -> bool {
    let first_person = viewer.is_some_and(|viewer| viewer == actor);
    if view_weapon {
        first_person
    } else {
        !first_person
    }
}

/// Shader beam (donor `shaderBeam`).
#[derive(Debug, Clone, PartialEq)]
pub struct SceneShaderBeam {
    /// Shader path.
    pub path: String,
    /// Beam end.
    pub end: Vec3,
    /// Beam width.
    pub width: f32,
}

/// Scene flare (donor `SceneFlare`).
#[derive(Debug, Clone, PartialEq)]
pub struct SceneFlare {
    /// Image path.
    pub image: String,
    /// Fade start distance.
    pub fade_start: f64,
    /// Fade end distance.
    pub fade_end: f64,
    /// Flare scale.
    pub scale: f64,
    /// Flare color.
    pub color: Vec3,
    /// Rim color.
    pub rim_color: Option<Vec3>,
    /// Whether the angle locks.
    pub lock_angle: bool,
}

/// Model attachment (donor `modelAttachments` entry).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SceneModelAttachment {
    /// Model path.
    pub path: String,
    /// Tag name.
    pub tag: String,
}

/// Grapple cable (donor `q3GrappleCable`).
#[derive(Debug, Clone, PartialEq)]
pub struct SceneGrappleCable {
    /// Cable owner.
    pub owner: ActorId,
    /// Owner origin.
    pub owner_origin: Vec3,
    /// Owner angles.
    pub owner_angles: Vec3,
    /// View height.
    pub view_height: f32,
    /// Whether offhand.
    pub offhand: bool,
    /// Whether attached.
    pub attached: bool,
    /// Flight model.
    pub flight: String,
    /// Pull model.
    pub pull: String,
    /// Hold model.
    pub hold: String,
    /// Segment length.
    pub segment_length: f32,
}

/// Simulation presentation read by the scene (donor `SimulationPresentation` subset).
#[derive(Debug, Clone, PartialEq)]
pub struct ScenePresentation {
    /// Presenting actor.
    pub actor: ActorId,
    /// Source content.
    pub content: ContentId,
    /// Game family.
    pub family: GameFamily,
    /// Model path.
    pub path: String,
    /// Current frame.
    pub frame: i32,
    /// Previous frame.
    pub old_frame: i32,
    /// Frame blend.
    pub back_lerp: Option<f32>,
    /// Skin.
    pub skin: i32,
    /// Skin path override.
    pub skin_path: Option<String>,
    /// Indexed skin.
    pub indexed_skin: Option<IndexedModelSkin>,
    /// Player colors.
    pub player_colors: Option<PlayerColors>,
    /// Render flags.
    pub render_flags: i32,
    /// World origin.
    pub origin: Vec3,
    /// Previous origin.
    pub previous_origin: Option<Vec3>,
    /// Beam options.
    pub model_beam: Option<ModelBeamOptions>,
    /// Shader beam.
    pub shader_beam: Option<SceneShaderBeam>,
    /// Flare.
    pub flare: Option<SceneFlare>,
    /// Model attachments.
    pub model_attachments: Vec<SceneModelAttachment>,
    /// View-model anchor override.
    pub model_anchor: Option<Q3WeaponAnchor>,
    /// Grapple cable.
    pub grapple_cable: Option<SceneGrappleCable>,
    /// World angles.
    pub angles: Vec3,
    /// Uniform scale.
    pub scale: f32,
    /// Alpha.
    pub alpha: Option<f32>,
    /// Whether visible.
    pub visible: bool,
    /// Whether a view weapon.
    pub view_weapon: bool,
    /// Selected weapon state.
    pub q3_weapon: Option<Q3WeaponView>,
    /// Whether natively held.
    pub native_held_weapon: bool,
    /// Whether a held weapon declaration is present.
    pub held_weapon: bool,
}

/// Held weapon shader pass (donor `QvmHeldWeapon` pass).
#[derive(Debug, Clone, PartialEq)]
pub struct SceneHeldPass {
    /// Shader override.
    pub shader: Option<String>,
    /// Pass color bytes.
    pub color: Vec4,
    /// Shader time.
    pub shader_time: f64,
}

/// Scene entity with the actor and opacity the ported entity cannot hold.
#[derive(Debug, Clone, PartialEq)]
pub struct ScenePassEntity {
    /// Render entity.
    pub entity: SceneEntity,
    /// Presenting actor.
    pub actor: Option<ActorId>,
    /// Opacity.
    pub opacity: f32,
}

/// Held weapon (donor `QvmHeldWeapon` from `./q3-client/qvm.ts`; renamed because the
/// trap capture port owns the donor name).
#[derive(Debug, Clone, PartialEq)]
pub struct SceneHeldWeapon {
    /// Source content.
    pub content: ContentId,
    /// Parent entity.
    pub parent: ScenePassEntity,
    /// Shader passes.
    pub passes: Vec<SceneHeldPass>,
}

/// QVM body part (donor `QvmBodyPart`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SceneBodyPart {
    /// Body.
    Body,
    /// Lower.
    Lower,
    /// Upper.
    Upper,
    /// Head.
    Head,
}

impl SceneBodyPart {
    /// Donor part text.
    fn as_str(&self) -> &'static str {
        match self {
            SceneBodyPart::Body => "body",
            SceneBodyPart::Lower => "lower",
            SceneBodyPart::Upper => "upper",
            SceneBodyPart::Head => "head",
        }
    }
}

/// Body custom shader (donor `SceneShader`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BodyCustomShader {
    /// Shader name.
    pub name: String,
}

/// Body custom skin (donor `SceneSkin` surfaces).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BodyCustomSkin {
    /// Surface overrides.
    pub surfaces: Vec<CustomSkinEntry>,
}

/// Body material pass (donor `BodyMaterial`).
#[derive(Debug, Clone, PartialEq)]
pub struct SceneBodyMaterial {
    /// Custom shader.
    pub custom_shader: Option<BodyCustomShader>,
    /// Custom skin.
    pub custom_skin: Option<BodyCustomSkin>,
    /// Shader color bytes.
    pub shader_rgba: Vec4,
    /// Shader texture coordinates.
    pub shader_tex_coord: Vec2,
    /// Shader time.
    pub shader_time: f64,
    /// Render flags.
    pub render_flags: i32,
    /// Lighting origin.
    pub lighting_origin: Vec3,
    /// Shadow plane (no render counterpart; carried for the host key).
    pub shadow_plane: f32,
    /// Non-normalized axes.
    pub non_normalized_axes: bool,
}

/// Component body part entry (donor `ComponentBody` part).
#[derive(Debug, Clone, PartialEq)]
pub struct SceneBodyPartEntry {
    /// Part.
    pub part: SceneBodyPart,
    /// Whether a base part.
    pub base: bool,
    /// Material passes.
    pub passes: Vec<SceneBodyMaterial>,
}

/// Component body (donor `ComponentBody`).
#[derive(Debug, Clone, PartialEq)]
pub struct SceneBody {
    /// Presenting owner.
    pub owner: qa_content::contract::PresentationOwner,
    /// Body actor.
    pub actor: ActorId,
    /// Source content.
    pub content: ContentId,
    /// Body time in milliseconds.
    pub time_ms: i32,
    /// Parts.
    pub parts: Vec<SceneBodyPartEntry>,
}

/// Prepared primary body (donor `PreparedPrimaryBody`).
#[derive(Debug, Clone, PartialEq)]
pub struct ScenePrimaryBody {
    /// Body actor.
    pub actor: ActorId,
    /// Part.
    pub part: SceneBodyPart,
    /// Source content.
    pub content: ContentId,
    /// Posed entity.
    pub entity: ScenePassEntity,
    /// Whether a base part.
    pub base: bool,
    /// Source options.
    pub options: ModelSourceOptions,
    /// Shader content.
    pub shader_content: ContentId,
    /// Body time in milliseconds.
    pub time_ms: i32,
}

/// Brush model reference (donor `brushScene` cell; a missing scene is the world).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SceneBrushRef {
    /// Brush model number.
    pub model: i32,
    /// Brush scene key, or [`None`] for the world scene.
    pub scene: Option<u64>,
    /// Whether the brush scene is a Quake map.
    pub q1_bsp: bool,
}

/// Loaded scene model (donor `assets.model` result).
pub struct SceneModelAsset {
    /// Model resource.
    pub resource: ModelResource,
    /// Decoded model.
    pub model: SceneModel,
    /// Alias effect flags (donor `modelFlags`).
    pub model_flags: i32,
    /// Brush model reference, when the model is a brush.
    pub brush: Option<SceneBrushRef>,
}

/// Model assets (donor `ApplicationAssets` subset).
pub trait SceneModelAssets {
    /// Load a model.
    fn model(&mut self, content: &ContentId, path: &str) -> Result<SceneModelAsset, SceneError>;
    /// Open a file, or [`None`] when absent.
    fn open(&mut self, content: &ContentId, path: &str) -> Option<Vec<u8>>;
    /// Load a Quake III model, or [`None`] when absent.
    fn q3_model(&mut self, content: &ContentId, path: &str) -> Option<Q3WeaponModelAsset>;
    /// Load a flare texture, or [`None`] when absent.
    fn flare_texture(&mut self, content: &ContentId, path: &str) -> Option<RendererImage>;
    /// Register a shader.
    fn register_shader(&mut self, content: &ContentId, path: &str) -> Result<RegisteredSceneMaterial, SceneError>;
    /// Character appearance content.
    fn appearance_content(&self) -> ContentId;
    /// Material registrations for admission.
    fn material_registrations(&mut self) -> Vec<ShaderRegistration>;
}

/// Model renderer (donor `SceneModelRenderer`).
pub trait SceneRenderer {
    /// Warm the renderer cache.
    fn preload(&mut self, entity: &SceneEntity, options: &ModelSourceOptions) -> Result<(), SceneError>;
    /// Prepare draw groups.
    fn prepare(
        &self,
        entity: &SceneEntity,
        input: &WorldViewInput,
        options: &ModelSourceOptions,
        skinning: &ModelSkinningFrame,
    ) -> Result<Vec<SceneModelGroup>, SceneError>;
    /// Prepare shadow casters.
    fn prepare_shadow_caster(
        &self,
        entity: &SceneEntity,
        input: &WorldViewInput,
        options: &ModelSourceOptions,
        skinning: &ModelSkinningFrame,
        shadow_lights: &[SceneLight],
    ) -> Result<Vec<ModelShadowCaster>, SceneError>;
}

/// Model renderer factory (donor per-content renderer assembly).
pub trait SceneRendererFactory {
    /// Renderer type.
    type Renderer: SceneRenderer;
    /// Build the renderer for content with an optional shader override.
    fn make_renderer(
        &mut self,
        content: &ContentId,
        shader_content: Option<&ContentId>,
    ) -> Result<Self::Renderer, SceneError>;
}

/// Prepared shadows (donor `prepareShadows` result).
pub struct SceneShadowResult {
    /// Fragment lighting.
    pub lighting: qa_client::render::scene::world::Q2FragmentLighting,
    /// Atlas operations.
    pub operations: Vec<RenderOperation>,
}

/// World scene (donor `WorldScene` subset).
pub trait SceneWorld {
    /// Prepare shadows.
    fn prepare_shadows(
        &mut self,
        lights: &[SceneLight],
        input: &WorldViewInput,
        casters: &[ModelShadowCaster],
    ) -> Result<SceneShadowResult, SceneError>;
    /// Prepare world operations.
    fn prepare_world_operations(&mut self, input: &mut WorldViewInput) -> Result<(), SceneError>;
    /// Prepare the view.
    fn prepare_view(&mut self, input: WorldViewInput) -> Result<PreparedWorldView, SceneError>;
    /// Prepare an inline model.
    fn prepare_model(&mut self, model: &InlineModel, input: &WorldViewInput)
        -> Result<Vec<SceneOperation>, SceneError>;
    /// Prepare a brush model.
    #[allow(clippy::too_many_arguments)]
    fn prepare_brush_model(
        &mut self,
        scene: u64,
        model: i32,
        transform: &ModelTransform,
        frame: i32,
        alternate_animation: bool,
        entity_rgba: [u8; 4],
        input: &WorldViewInput,
    ) -> Result<Vec<SceneOperation>, SceneError>;
    /// Prepare a flare batch.
    fn prepare_flare(
        &mut self,
        flare: &SceneFlare,
        origin: Vec3,
        camera: &SceneCamera,
        image: &RendererImage,
        image_path: &str,
    ) -> Result<DrawBatch, SceneError>;
    /// Prepare shader beam batches.
    fn prepare_shader_beam(
        &mut self,
        material: &RegisteredSceneMaterial,
        origin: Vec3,
        end: Vec3,
        width: f32,
        camera_origin: Vec3,
        input: &WorldViewInput,
    ) -> Result<Vec<DrawBatch>, SceneError>;
}

/// Held weapon declaration (donor `declaration` three states).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HeldWeaponDeclaration {
    /// No declaration.
    Absent,
    /// Explicit none.
    None,
    /// A declaration.
    Present,
}

/// Weapon character (donor `frame` character union).
#[derive(Debug, Clone, PartialEq)]
pub enum SceneWeaponCharacter {
    /// Simple origin with optional color and opacity.
    Simple {
        /// Origin.
        origin: Vec3,
        /// Color.
        color: Option<Vec4>,
        /// Opacity.
        opacity: Option<f32>,
    },
    /// Full character view.
    Character(Q3CharacterView),
}

/// Held resolve mode (donor `"shadow"` / `"view"`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HeldResolveMode {
    /// Shadow.
    Shadow,
    /// View.
    View,
}

/// Native held attachment factory (donor `attachment` result).
pub struct HeldAttachmentFactory {
    /// Attach a pass entity.
    pub attach: Box<dyn Fn(&SceneEntity) -> HeldAttachment>,
}

/// Resolved held entity (donor `ResolvedHeldEntity`).
pub struct HeldAttachment {
    /// Resolve the entity for an origin and mode.
    pub resolve: Box<dyn Fn(Vec3, HeldResolveMode) -> Option<SceneEntity>>,
}

/// Held weapons (donor `ForeignHeldWeapons` / `NativeHeldWeapons`).
pub trait SceneHeldWeapons {
    /// Declare an equipped weapon.
    fn declaration(&mut self, equipped: &ScenePresentation) -> HeldWeaponDeclaration;
    /// Frame an equipped weapon.
    fn frame(
        &mut self,
        equipped: &ScenePresentation,
        character: &SceneWeaponCharacter,
    ) -> Result<Vec<Q3CharacterPass>, SceneError>;
    /// Attach a native held weapon.
    fn attachment(&mut self, content: &ContentId, entity: &SceneEntity) -> HeldAttachmentFactory;
}

/// Component bodies (donor `component-bodies.ts` queries).
pub trait SceneBodies {
    /// Whether a base part stays visible.
    fn body_base_visible(&self, bodies: &[SceneBody], actor: &ActorId, part: SceneBodyPart) -> bool;
    /// Material passes for a body part.
    fn body_materials(&self, body: &SceneBody, part: SceneBodyPart) -> Vec<SceneBodyMaterial>;
}

/// Scene prepare input (donor `prepare` parameters).
pub struct ScenePrepareInput<'a> {
    /// Viewing actor.
    pub viewer: Option<&'a ActorId>,
    /// World snapshot.
    pub snapshot: &'a WorldSnapshot,
    /// Presentations.
    pub presentations: &'a [ScenePresentation],
    /// Character views.
    pub characters: &'a [Q3CharacterView],
    /// Field of view.
    pub field_of_view: f32,
    /// Held weapons.
    pub held_weapons: &'a [SceneHeldWeapon],
    /// Whether view weapons stay visible.
    pub view_weapon_visible: bool,
    /// Component bodies.
    pub bodies: &'a [SceneBody],
    /// Primary bodies.
    pub primary_bodies: &'a [ScenePrimaryBody],
}

/// First 64 bits of a `sha256:` digest for model resources.
fn resource_digest(digest: &ContentDigest) -> u64 {
    let hex = digest.as_str().strip_prefix("sha256:").unwrap_or(digest.as_str());
    u64::from_str_radix(hex.get(..16).unwrap_or(""), 16).unwrap_or(0)
}

/// Render resource for a resolved reference.
fn model_resource(resource: &ResolvedResourceReference) -> ModelResource {
    ModelResource {
        id: resource.id.to_string(),
        requested_path: resource.requested_path.clone(),
        digest: resource_digest(&resource.digest),
    }
}

/// Render pose for a presented pose.
fn scene_pose(pose: &Q3ModelPose) -> ScenePose {
    match pose {
        Q3ModelPose::Frame {
            frame,
            previous_frame,
            back_lerp,
        } => ScenePose::Frame {
            frame: *frame,
            previous_frame: *previous_frame,
            back_lerp: *back_lerp,
        },
        Q3ModelPose::Skeleton { joints } => ScenePose::Skeleton { joints: joints.clone() },
    }
}

/// Render model for a presented model.
///
/// The foundation mirror carries MD5 joints and frames only; meshes stay
/// loader-side and arrive as an empty mesh list.
fn scene_model(model: &Q3PresentedModel) -> SceneModel {
    match model {
        Q3PresentedModel::Md3(scene) => SceneModel::Q3Md3(scene.clone()),
        Q3PresentedModel::Md5 { joints, frames } => SceneModel::Md5(DecodedMd5Model {
            mesh_source: String::new(),
            mesh_command_line: String::new(),
            mesh_joints: joints.clone(),
            bind_pose: Vec::new(),
            animation: Md5Animation {
                source: String::new(),
                command_line: String::new(),
                frame_rate: 0,
                hierarchy: Vec::new(),
                base_frame: Vec::new(),
                frames: frames.clone(),
                scale_source: None,
                diagnostics: Vec::new(),
            },
            joints: Vec::new(),
            meshes: Vec::new(),
            frame_rate: 0,
            frames: frames.clone(),
            skin_selection: SkinSelection::MeshShaders,
        }),
    }
}

/// Render attachment for a presented attachment.
fn scene_attachment(attachment: &Q3Attachment) -> ModelAttachment {
    ModelAttachment {
        tag: attachment.tag.clone(),
        entity: Box::new(scene_entity(&attachment.entity)),
    }
}

/// Render entity for a presented entity (donor identity; the ported shapes split Q3
/// content from render state, and the shadow plane has no render counterpart).
fn scene_entity(entity: &Q3SceneEntity) -> SceneEntity {
    SceneEntity {
        resource: model_resource(&entity.resource),
        model: scene_model(&entity.model),
        pose: scene_pose(&entity.pose),
        transform: EntityTransform {
            origin: entity.transform.origin,
            axis: entity.transform.axis,
            scale: entity.transform.scale,
        },
        previous_origin: entity.previous_origin,
        lighting_origin: entity.lighting_origin,
        color: entity.color,
        skin: entity.skin,
        shader_time_seconds: entity.shader_time.as_seconds_f64(),
        flags: EntityFlags::Q3 {
            bits: entity.flags as u32,
        },
        attachments: entity.attachments.iter().map(scene_attachment).collect(),
        actor_slot: entity.actor.as_ref().map(ActorId::slot),
    }
}

/// Pass entity for a presented entity, carrying its actor and opacity.
fn pass_entity(entity: &Q3SceneEntity) -> ScenePassEntity {
    ScenePassEntity {
        entity: scene_entity(entity),
        actor: entity.actor.clone(),
        opacity: entity.opacity,
    }
}

/// Map foundation pass options into renderer options.
fn client_pass_options(options: &FoundationModelSourceOptions) -> ModelSourceOptions {
    ModelSourceOptions {
        custom_shader: options.custom_shader.clone(),
        custom_skin: options.custom_skin.as_ref().map(|skin| {
            skin.iter()
                .map(|entry| CustomSkinEntry {
                    name: entry.name.clone(),
                    shader: entry.shader.clone(),
                })
                .collect()
        }),
        ..ModelSourceOptions::default()
    }
}

/// Resolve a Q3 pass's options, defaulting the character lookup without assets.
fn resolve_pass_options(
    assets: Option<&Q3CharacterAssets>,
    pass: &Q3CharacterPass,
    entity: &Q3SceneEntity,
) -> ModelSourceOptions {
    let resolved = match (&pass.options, assets) {
        (Q3PassOptions::Foreign(resolve), _) => resolve(entity),
        (Q3PassOptions::Character, Some(assets)) => pass.options(assets, entity),
        (Q3PassOptions::Character, None) => FoundationModelSourceOptions {
            custom_shader: pass.shader.clone(),
            custom_skin: None,
        },
    };
    client_pass_options(&resolved)
}

/// Shared source-options resolver (donor `(entity) => ModelSourceOptions`).
type OptionsResolver = Rc<dyn Fn(&SceneEntity) -> ModelSourceOptions>;

/// Freeze a Q3 pass's options into a scene resolver.
///
/// The resolver reads the captured pass entity; the donor re-resolves per render
/// with the attached entity, but the ported resolvers ignore their argument.
fn pass_options_resolver(assets: Option<Q3CharacterAssets>, pass: &Q3CharacterPass) -> OptionsResolver {
    let pass = pass.clone();
    Rc::new(move |_: &SceneEntity| resolve_pass_options(assets.as_ref(), &pass, &pass.entity))
}

/// Render clock for a source time.
fn render_time(time: &SourceTime) -> RenderSourceTime {
    match *time {
        SourceTime::Seconds(value) => RenderSourceTime::Seconds(f64::from(value)),
        SourceTime::Milliseconds(value) => RenderSourceTime::Milliseconds(f64::from(value)),
    }
}

/// Default resolver (donor `() => ({})`).
fn default_options() -> OptionsResolver {
    Rc::new(|_| ModelSourceOptions::default())
}

/// Minimal weapon-view source for a presentation.
fn view_source(source: &ScenePresentation) -> WeaponViewSource {
    WeaponViewSource {
        view_weapon: source.view_weapon,
        family: source.family,
        origin: source.origin,
    }
}

/// Selected-weapon source for a presentation.
fn weapon_source(source: &ScenePresentation, visible: bool) -> Q3WeaponSource {
    Q3WeaponSource {
        actor: source.actor.clone(),
        content: source.content.clone(),
        path: source.path.clone(),
        visible,
        origin: source.origin,
        angles: source.angles,
        frame: source.frame,
        old_frame: source.old_frame,
        back_lerp: source.back_lerp,
        weapon: source.q3_weapon.clone(),
        model_anchor: source.model_anchor.clone(),
        model_attachments: source
            .model_attachments
            .iter()
            .map(|attachment| Q3WeaponAttachment {
                tag: attachment.tag.clone(),
                path: attachment.path.clone(),
            })
            .collect(),
    }
}

/// Selected-weapon cache key (donor `` `${content}/${slot}/${generation}` ``).
fn weapon_key(source: &ScenePresentation) -> String {
    format!(
        "{}/{}/{}",
        source.content,
        source.actor.slot(),
        source.actor.generation()
    )
}

/// Fetch or create a cached selected-weapon presenter.
fn selected_presenter<'a>(
    selected: &'a mut HashMap<String, SelectedQ3WeaponPresenter>,
    character_assets: Option<&Q3CharacterAssets>,
    key: String,
) -> &'a mut SelectedQ3WeaponPresenter {
    if !selected.contains_key(&key) {
        let animation = character_assets.map(|assets| assets.animation.clone());
        selected.insert(key.clone(), SelectedQ3WeaponPresenter::new(animation));
    }
    selected
        .get_mut(&key)
        .expect("selected weapon presenter inserted above")
}

/// Weapon asset reads over scene assets.
struct WeaponAssets<'a, A> {
    assets: &'a mut A,
}

impl<A: SceneModelAssets> Q3WeaponAssets for WeaponAssets<'_, A> {
    fn open(&mut self, content: &ContentId, path: &str) -> Option<Vec<u8>> {
        self.assets.open(content, path)
    }

    fn model(&mut self, content: &ContentId, path: &str) -> Option<Q3WeaponModelAsset> {
        self.assets.q3_model(content, path)
    }
}

/// Whether an operation draws world polygons (donor `polygon`).
fn is_world_polygon(operation: &SceneOperation) -> bool {
    matches!(operation, SceneOperation::Group(group)
        if matches!(&group.order, SceneGroupOrder::Source { source, .. }
            if source.entity == SourceEntityOrder::World))
}

/// Model pass (donor `ModelPass`).
struct ScenePass {
    entity: ScenePassEntity,
    resolve: Option<HeldAttachment>,
    time: Option<SourceTime>,
    options: OptionsResolver,
}

/// Model group (donor `ModelGroup`).
struct SceneGroupData<R> {
    renderer: R,
    passes: Vec<ScenePass>,
}

/// Pass reference (arena indices stand in for the donor's object identity).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct PassRef {
    group: usize,
    pass: usize,
}

/// Presentation object (donor `PresentationObject`).
struct PresentationObjectData {
    opacity: f32,
    passes: Vec<PassRef>,
}

/// Brush presentation (donor `BrushPresentation`).
#[derive(Debug, Clone, PartialEq)]
struct BrushPresentationData {
    scene: u64,
    model: i32,
    transform: ModelTransform,
    frame: i32,
    alpha: f32,
    q1_bsp: bool,
}

/// Prepared flare (donor `flares` entry).
#[derive(Debug, Clone, PartialEq)]
struct FlareData {
    flare: SceneFlare,
    origin: Vec3,
    image: RendererImage,
    image_path: String,
}

/// Prepared shader beam (donor `shaderBeams` entry).
#[derive(Debug, Clone, PartialEq)]
struct BeamData {
    material: RegisteredSceneMaterial,
    origin: Vec3,
    end: Vec3,
    width: f32,
}

/// Retained lightstyle (donor `lightStyles` entry).
#[derive(Debug, Clone, PartialEq)]
struct LightStyleData {
    pattern: String,
    owner: Option<PresentationOwner>,
}

/// Posed body part (donor `posed` entry).
struct PosedPart {
    part: SceneBodyPart,
    content: ContentId,
    entity: ScenePassEntity,
    options: OptionsResolver,
}

impl<'a> ScenePrepareInput<'a> {
    /// Bundle the required inputs with the donor's defaults (`fieldOfView` 90, no
    /// held weapons, visible view weapons, no bodies).
    pub fn new(
        viewer: Option<&'a ActorId>,
        snapshot: &'a WorldSnapshot,
        presentations: &'a [ScenePresentation],
        characters: &'a [Q3CharacterView],
    ) -> Self {
        Self {
            viewer,
            snapshot,
            presentations,
            characters,
            field_of_view: 90.0,
            held_weapons: &[],
            view_weapon_visible: true,
            bodies: &[],
            primary_bodies: &[],
        }
    }
}

/// World scene assembly (donor `ApplicationWorldScene`).
pub struct ApplicationWorldScene<A, R: SceneRendererFactory, W, H, B> {
    assets: A,
    renderers: R,
    world: W,
    held: H,
    bodies_impl: B,
    character_assets: Option<Q3CharacterAssets>,
    planar_shadows: Box<dyn Fn() -> bool>,
    groups: HashMap<String, usize>,
    group_list: Vec<SceneGroupData<R::Renderer>>,
    ordered: Vec<PassRef>,
    objects: HashMap<PassRef, usize>,
    object_list: Vec<PresentationObjectData>,
    characters: HashMap<String, Q3CharacterPresenter>,
    selected_weapons: HashMap<String, SelectedQ3WeaponPresenter>,
    light_styles: HashMap<i32, LightStyleData>,
    inline_models: Vec<InlineModel>,
    brush_models: Vec<BrushPresentationData>,
    prepared_time: f64,
    previous_time: f64,
    flares: Vec<FlareData>,
    shader_beams: Vec<BeamData>,
}

impl<A, R: SceneRendererFactory, W, H, B> ApplicationWorldScene<A, R, W, H, B>
where
    A: SceneModelAssets,
    W: SceneWorld,
    H: SceneHeldWeapons,
    B: SceneBodies,
{
    /// Build the scene over host seams with optional character assets.
    pub fn new(
        assets: A,
        renderers: R,
        world: W,
        held: H,
        bodies_impl: B,
        character_assets: Option<Q3CharacterAssets>,
        planar_shadows: impl Fn() -> bool + 'static,
    ) -> Self {
        Self {
            assets,
            renderers,
            world,
            held,
            bodies_impl,
            character_assets,
            planar_shadows: Box::new(planar_shadows),
            groups: HashMap::new(),
            group_list: Vec::new(),
            ordered: Vec::new(),
            objects: HashMap::new(),
            object_list: Vec::new(),
            characters: HashMap::new(),
            selected_weapons: HashMap::new(),
            light_styles: HashMap::new(),
            inline_models: Vec::new(),
            brush_models: Vec::new(),
            prepared_time: 0.0,
            previous_time: 0.0,
            flares: Vec::new(),
            shader_beams: Vec::new(),
        }
    }

    /// Retain lightstyles and retire owner styles (donor `receive`).
    pub fn receive<F>(&mut self, events: &[SimulationPresentationEvent<F>]) {
        for event in events {
            match &event.source {
                SourcePresentationEvent::PresentationOwner {
                    event: OwnerLifecycle::Retired { owner },
                } => {
                    self.light_styles
                        .retain(|_, style| !same_presentation_owner(style.owner.as_ref(), owner));
                }
                SourcePresentationEvent::Q1(Q1PresentationEvent::Lightstyle { style, pattern })
                | SourcePresentationEvent::Q2(Q2PresentationEvent::Lightstyle { style, pattern }) => {
                    self.light_styles.insert(
                        *style,
                        LightStyleData {
                            pattern: pattern.clone(),
                            owner: event.owner.clone(),
                        },
                    );
                }
                _ => {}
            }
        }
    }

    /// Sample a lightstyle pattern (donor `style`).
    pub fn style(&self, index: i32, absent: f64) -> f64 {
        let Some(style) = self.light_styles.get(&index) else {
            return absent;
        };
        if style.pattern.is_empty() {
            return absent;
        }
        let at = (self.prepared_time * 10.0).trunc() as i64 % style.pattern.len() as i64;
        if at < 0 {
            return f64::NAN;
        }
        f64::from(style.pattern.as_bytes()[at as usize]) - 97.0
    }

    /// Quake and Quake II style tables (donor `styles`).
    pub fn styles(&self) -> (Vec<i32>, Vec<Q2LightStyle>) {
        let q1 = (0..256)
            .map(|index| {
                if self.light_styles.contains_key(&index) {
                    (self.style(index, 12.0) * 22.0) as i32
                } else {
                    256
                }
            })
            .collect();
        let q2 = (0..256)
            .map(|index| {
                let value = (self.style(index, 12.0) / 12.0) as f32;
                Q2LightStyle {
                    rgb: vec3(value, value, value),
                    white: value * 3.0,
                }
            })
            .collect();
        (q1, q2)
    }

    /// Create a shared presentation object.
    fn new_object(&mut self, opacity: f32) -> usize {
        self.object_list.push(PresentationObjectData {
            opacity,
            passes: Vec::new(),
        });
        self.object_list.len() - 1
    }

    /// Append a pass, creating its content group on demand (donor `append`).
    #[allow(clippy::too_many_arguments)]
    fn append(
        &mut self,
        content: &ContentId,
        entity: ScenePassEntity,
        options: OptionsResolver,
        object: Option<usize>,
        shader_content: Option<&ContentId>,
        time: Option<SourceTime>,
        resolve: Option<HeldAttachment>,
    ) -> Result<(), SceneError> {
        let key = format!("{content}/{}", shader_content.unwrap_or(content));
        let index = match self.groups.get(&key) {
            Some(index) => *index,
            None => {
                let renderer = self.renderers.make_renderer(content, shader_content)?;
                self.group_list.push(SceneGroupData {
                    renderer,
                    passes: Vec::new(),
                });
                let index = self.group_list.len() - 1;
                self.groups.insert(key, index);
                index
            }
        };
        let opacity = entity.opacity;
        self.group_list[index].passes.push(ScenePass {
            entity,
            resolve,
            time,
            options,
        });
        let pass = self.group_list[index].passes.len() - 1;
        let reference = PassRef { group: index, pass };
        self.ordered.push(reference);
        let object = object.unwrap_or_else(|| self.new_object(opacity));
        self.object_list[object].passes.push(reference);
        self.objects.insert(reference, object);
        Ok(())
    }

    /// Append a body-aware pass, retaining posed parts for body materials (donor
    /// `appendBody`).
    #[allow(clippy::too_many_arguments)]
    fn append_body(
        &mut self,
        bodies: &[SceneBody],
        posed: &mut Vec<PosedPart>,
        posed_keys: &mut HashSet<String>,
        content: &ContentId,
        entity: ScenePassEntity,
        options: OptionsResolver,
        base: bool,
        object: Option<usize>,
    ) -> Result<(), SceneError> {
        let affected = entity
            .actor
            .as_ref()
            .is_some_and(|actor| bodies.iter().any(|body| body.actor == *actor));
        if !affected {
            return self.append(content, entity, options, object, None, None, None);
        }
        let Some(actor) = entity.actor.clone() else {
            return self.append(content, entity, options, object, None, None, None);
        };
        let id = entity.entity.resource.id.as_str();
        let part = if self
            .character_assets
            .as_ref()
            .is_some_and(|assets| assets.lower.resource.id.as_str() == id)
        {
            SceneBodyPart::Lower
        } else if self
            .character_assets
            .as_ref()
            .is_some_and(|assets| assets.upper.resource.id.as_str() == id)
        {
            SceneBodyPart::Upper
        } else if self
            .character_assets
            .as_ref()
            .is_some_and(|assets| assets.head.resource.id.as_str() == id)
        {
            SceneBodyPart::Head
        } else {
            SceneBodyPart::Body
        };
        let mut body = entity.clone();
        body.entity.attachments = Vec::new();
        let key = format!(
            "{}/{}/{}/{}",
            actor.slot(),
            actor.generation(),
            part.as_str(),
            body.entity.resource.id
        );
        if posed_keys.insert(key) {
            posed.push(PosedPart {
                part,
                content: content.clone(),
                entity: body.clone(),
                options: Rc::clone(&options),
            });
        }
        if !base || self.bodies_impl.body_base_visible(bodies, &actor, part) {
            self.append(content, body, Rc::clone(&options), object, None, None, None)?;
        }
        for attachment in &entity.entity.attachments {
            let Some((tag, scale)) = model_attachment_tag(&entity.entity, &attachment.tag) else {
                continue;
            };
            let resolved = attach_scene_entity(&entity.entity, &attachment.entity, &tag, scale);
            let child = ScenePassEntity {
                entity: resolved,
                actor: entity.actor.clone(),
                opacity: entity.opacity,
            };
            let is_part = self.character_assets.as_ref().is_some_and(|assets| {
                [
                    assets.lower.resource.id.as_str(),
                    assets.upper.resource.id.as_str(),
                    assets.head.resource.id.as_str(),
                ]
                .contains(&child.entity.resource.id.as_str())
            });
            if is_part {
                self.append_body(
                    bodies,
                    posed,
                    posed_keys,
                    content,
                    child,
                    Rc::clone(&options),
                    base,
                    object,
                )?;
            } else {
                self.append(content, child, Rc::clone(&options), object, None, None, None)?;
            }
        }
        Ok(())
    }

    /// Assemble the scene for one frame (donor `prepare`).
    pub fn prepare(&mut self, input: &ScenePrepareInput<'_>) -> Result<(), SceneError> {
        self.objects.clear();
        self.object_list.clear();
        self.ordered.clear();
        self.flares.clear();
        self.shader_beams.clear();
        self.previous_time = self.prepared_time;
        self.prepared_time = input.snapshot.frame.time.as_seconds_f64();
        for group in &mut self.group_list {
            group.passes.clear();
        }
        let mut inline_models = Vec::new();
        let mut brush_models = Vec::new();
        let mut posed = Vec::new();
        let mut posed_keys = HashSet::new();
        for original in input.presentations {
            let mut source = original.clone();
            if !source.visible
                || source.view_weapon
                    && !input.view_weapon_visible
                    && input.viewer.is_some_and(|viewer| *viewer == source.actor)
            {
                continue;
            }
            if let Some(beam) = &source.shader_beam {
                let material = self.assets.register_shader(&source.content, &beam.path)?;
                self.shader_beams.push(BeamData {
                    material,
                    origin: source.origin,
                    end: beam.end,
                    width: beam.width,
                });
                continue;
            }
            if let Some(flare) = &source.flare {
                let mut image_path = flare.image.clone();
                let mut texture = self.assets.flare_texture(&source.content, &image_path);
                if texture.is_none() && image_path != "misc/flare.tga" {
                    image_path = "misc/flare.tga".to_string();
                    texture = self.assets.flare_texture(&source.content, &image_path);
                }
                if let Some(texture) = texture {
                    self.flares.push(FlareData {
                        flare: flare.clone(),
                        origin: source.origin,
                        image: texture,
                        image_path,
                    });
                }
                continue;
            }
            if source.path.is_empty() {
                continue;
            }
            if !seat_model_visible(input.viewer, &source.actor, source.view_weapon) {
                continue;
            }
            if !source.view_weapon && input.characters.iter().any(|character| character.actor == source.actor) {
                continue;
            }
            let mut cable_start = None;
            if let Some(cable) = &original.grapple_cable {
                let local = input.viewer.is_some_and(|viewer| *viewer == cable.owner);
                let aim = qvm_angle_vectors(cable.owner_angles);
                let mut start = if cable.offhand {
                    add3(
                        add3(cable.owner_origin, vec3(0.0, 0.0, 26.0)),
                        scale3(aim.right, if local { -10.0 } else { -6.0 }),
                    )
                } else {
                    add3(
                        cable.owner_origin,
                        vec3(0.0, 0.0, if local { cable.view_height } else { 0.0 }),
                    )
                };
                if local && cable.offhand {
                    start = add3(start, scale3(aim.forward, 3.0));
                }
                cable_start = Some(start);
                source.path = if !cable.attached {
                    cable.flight.clone()
                } else if length3(sub3(source.origin, start)) > 64.0 {
                    cable.pull.clone()
                } else {
                    cable.hold.clone()
                };
            }
            if source.q3_weapon.is_some() {
                let framed = {
                    let presenter = selected_presenter(
                        &mut self.selected_weapons,
                        self.character_assets.as_ref(),
                        weapon_key(&source),
                    );
                    let mut assets = WeaponAssets {
                        assets: &mut self.assets,
                    };
                    presenter.frame(
                        &mut assets,
                        &weapon_source(&source, source.visible),
                        input.field_of_view,
                    )?
                };
                let entity = pass_entity(&framed);
                self.append(
                    &source.content,
                    entity,
                    Rc::new(|_| ModelSourceOptions {
                        view_model: true,
                        ..ModelSourceOptions::default()
                    }),
                    None,
                    None,
                    None,
                    None,
                )?;
                continue;
            }
            let asset = self.assets.model(&source.content, &source.path)?;
            let angles = if source.family == GameFamily::Q1 && asset.model_flags & 8 != 0 {
                let spun = (100.0 * self.prepared_time) as f32 * (65536.0 / 360.0);
                let masked = spun.trunc() as i32 & 65535;
                Vec3 {
                    x: source.angles.x,
                    y: (360.0 / 65536.0) * masked as f32,
                    z: source.angles.z,
                }
            } else {
                source.angles
            };
            let axis = angles_to_axis(angles);
            if matches!(asset.model, SceneModel::BrushModel) {
                let origin = weapon_view_origin(&view_source(&source));
                let alpha = source.alpha.unwrap_or(1.0);
                let Some(brush) = asset.brush else {
                    return Err(SceneError::BrushScene(source.path.clone()));
                };
                match brush.scene {
                    None => inline_models.push(InlineModel {
                        model: brush.model.max(0) as usize,
                        transform: ViewModelTransform {
                            origin,
                            axis,
                            scale: source.scale,
                        },
                        animation_frame: Some(source.frame as f32),
                        alternate_animation: Some(source.family == GameFamily::Q1 && source.frame != 0),
                        casts_shadow: alpha == 1.0,
                        entity_rgba: Some([255, 255, 255, (alpha * 255.0) as u8]),
                    }),
                    Some(scene) => brush_models.push(BrushPresentationData {
                        scene,
                        model: brush.model,
                        transform: ModelTransform {
                            origin,
                            axis,
                            scale: vec3(source.scale, source.scale, source.scale),
                        },
                        frame: source.frame,
                        alpha,
                        q1_bsp: brush.q1_bsp,
                    }),
                }
                continue;
            }
            let entity = SceneEntity {
                resource: asset.resource,
                model: asset.model,
                transform: EntityTransform {
                    origin: weapon_view_origin(&view_source(&source)),
                    axis,
                    scale: vec3(source.scale, source.scale, source.scale),
                },
                previous_origin: source.previous_origin.unwrap_or(source.origin),
                pose: ScenePose::Frame {
                    frame: source.frame,
                    previous_frame: source.old_frame,
                    back_lerp: source.back_lerp.unwrap_or(0.0),
                },
                skin: source.skin,
                color: vec4(1.0, 1.0, 1.0, 1.0),
                shader_time_seconds: 0.0,
                flags: match source.family {
                    GameFamily::Q1 => EntityFlags::Q1 {
                        bits: source.render_flags as u32,
                    },
                    GameFamily::Q2 => EntityFlags::Q2 {
                        bits: source.render_flags as u32,
                    },
                    GameFamily::Q3 => EntityFlags::Q3 {
                        bits: source.render_flags as u32,
                    },
                },
                lighting_origin: source.origin,
                attachments: Vec::new(),
                actor_slot: Some(source.actor.slot()),
            };
            let pass = ScenePassEntity {
                entity,
                actor: Some(source.actor.clone()),
                opacity: source.alpha.unwrap_or(1.0),
            };
            let mut attachments = Vec::new();
            for attachment in &source.model_attachments {
                let child = self.assets.model(&source.content, &attachment.path)?;
                attachments.push(ModelAttachment {
                    tag: attachment.tag.clone(),
                    entity: Box::new(SceneEntity {
                        resource: child.resource,
                        model: child.model,
                        transform: EntityTransform {
                            origin: vec3(0.0, 0.0, 0.0),
                            axis: angles_to_axis(vec3(0.0, 0.0, 0.0)),
                            scale: vec3(1.0, 1.0, 1.0),
                        },
                        attachments: Vec::new(),
                        ..pass.entity.clone()
                    }),
                });
            }
            if let (Some(cable), Some(start)) = (&original.grapple_cable, cable_start) {
                let delta = sub3(start, source.origin);
                let distance = length3(delta);
                let direction = normalize3_or_zero(delta);
                let axis = angles_to_axis(vector_to_angles(direction));
                let count = (distance / cable.segment_length).floor();
                if count > 65536.0 {
                    return Err(SceneError::CableLimit);
                }
                if count >= 0.0 {
                    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                    let count = count as i32;
                    for index in 0..=count {
                        let mut segment = pass.clone();
                        segment.entity.transform.origin =
                            sub3(start, scale3(direction, (index + 1) as f32 * cable.segment_length));
                        segment.entity.transform.axis = axis;
                        self.append(&source.content, segment, default_options(), None, None, None, None)?;
                    }
                }
                continue;
            }
            let view_weapon = source.view_weapon;
            let model_beam = source.model_beam;
            let indexed_skin = source.indexed_skin.clone();
            let player_colors = source.player_colors;
            let player = source.family == GameFamily::Q2 && source.path.starts_with("players/");
            let custom_shader = source.skin_path.clone();
            let options: OptionsResolver = Rc::new(move |_| ModelSourceOptions {
                view_model: view_weapon,
                model_beam,
                indexed_skin: indexed_skin.clone(),
                player_colors,
                player,
                custom_shader: custom_shader.clone(),
                ..ModelSourceOptions::default()
            });
            if source.native_held_weapon {
                let equipped = input
                    .presentations
                    .iter()
                    .find(|candidate| candidate.view_weapon && candidate.actor == source.actor);
                let Some(equipped) = equipped else { continue };
                if equipped.path.is_empty() && !equipped.held_weapon {
                    continue;
                }
                let declaration = self.held.declaration(equipped);
                if declaration == HeldWeaponDeclaration::None {
                    continue;
                }
                let attach = self.held.attachment(&source.content, &pass.entity);
                let passes: Vec<Q3CharacterPass> =
                    if equipped.q3_weapon.is_some() && declaration == HeldWeaponDeclaration::Absent {
                        let presenter = selected_presenter(
                            &mut self.selected_weapons,
                            self.character_assets.as_ref(),
                            weapon_key(equipped),
                        );
                        let mut assets = WeaponAssets {
                            assets: &mut self.assets,
                        };
                        presenter.world(
                            &mut assets,
                            &weapon_source(equipped, true),
                            &Q3WeaponCharacter {
                                origin: pass.entity.lighting_origin,
                                powerups: 0,
                            },
                            false,
                        )?
                    } else {
                        self.held.frame(
                            equipped,
                            &SceneWeaponCharacter::Simple {
                                origin: pass.entity.lighting_origin,
                                color: Some(pass.entity.color),
                                opacity: Some(pass.opacity),
                            },
                        )?
                    };
                if passes.is_empty() {
                    return Err(SceneError::HeldModel(equipped.path.clone()));
                }
                let object = self.new_object(pass.opacity);
                for held_pass in &passes {
                    let entity = pass_entity(&held_pass.entity);
                    let resolve = (attach.attach)(&entity.entity);
                    let content = held_pass.content.as_ref().unwrap_or(&equipped.content);
                    let options = pass_options_resolver(self.character_assets.clone(), held_pass);
                    self.append(content, entity, options, Some(object), None, None, Some(resolve))?;
                }
                continue;
            }
            let mut entity = pass;
            entity.entity.attachments = attachments;
            if source.view_weapon {
                self.append(&source.content, entity, options, None, None, None, None)?;
            } else {
                self.append_body(
                    input.bodies,
                    &mut posed,
                    &mut posed_keys,
                    &source.content,
                    entity,
                    options,
                    true,
                    None,
                )?;
            }
        }
        if self.character_assets.is_some() {
            let now_ms = (self.prepared_time * 1000.0).trunc() as i32;
            let frame_ms = (now_ms - (self.previous_time * 1000.0).trunc() as i32).max(0);
            for character in input.characters {
                let key = format!("{}:{}", character.actor.slot(), character.actor.generation());
                if !self.characters.contains_key(&key) {
                    let Some(assets) = self.character_assets.clone() else {
                        continue;
                    };
                    let mut presenter = Q3CharacterPresenter::new(assets);
                    presenter.reset(character, now_ms)?;
                    self.characters.insert(key.clone(), presenter);
                }
                let declaration = input
                    .presentations
                    .iter()
                    .find(|source| source.view_weapon && source.actor == character.actor)
                    .map(|equipped| (equipped, self.held.declaration(equipped)));
                let mut weapon = Vec::new();
                if let Some((equipped, declaration)) = &declaration {
                    if equipped.q3_weapon.is_some() && *declaration == HeldWeaponDeclaration::Absent {
                        let presenter = selected_presenter(
                            &mut self.selected_weapons,
                            self.character_assets.as_ref(),
                            weapon_key(equipped),
                        );
                        let mut assets = WeaponAssets {
                            assets: &mut self.assets,
                        };
                        weapon = presenter.world(
                            &mut assets,
                            &weapon_source(equipped, true),
                            &Q3WeaponCharacter {
                                origin: character.origin,
                                powerups: character.powerups,
                            },
                            input.viewer.is_some_and(|viewer| *viewer == character.actor),
                        )?;
                    } else if !input.viewer.is_some_and(|viewer| *viewer == character.actor) {
                        weapon = self
                            .held
                            .frame(equipped, &SceneWeaponCharacter::Character(character.clone()))?;
                    }
                }
                let presenter = self
                    .characters
                    .get_mut(&key)
                    .expect("character presenter inserted above");
                let passes = presenter.frame(
                    character,
                    &Q3CharacterRenderOptions {
                        time_ms: now_ms,
                        frame_ms,
                        shader_time: SourceTime::Seconds(0.0),
                        swing_speed: 0.3,
                        no_player_animations: false,
                        personal_model: input.viewer.is_some_and(|viewer| *viewer == character.actor),
                        shadow_plane: None,
                        weapon,
                    },
                )?;
                let object = self.new_object(character.opacity.unwrap_or(1.0));
                for pass in &passes {
                    let content = pass.content.clone().unwrap_or_else(|| self.assets.appearance_content());
                    let entity = pass_entity(&pass.entity);
                    let options = pass_options_resolver(self.character_assets.clone(), pass);
                    let is_lower = self
                        .character_assets
                        .as_ref()
                        .is_some_and(|assets| assets.lower.resource.id == pass.entity.resource.id);
                    if is_lower {
                        self.append_body(
                            input.bodies,
                            &mut posed,
                            &mut posed_keys,
                            &content,
                            entity,
                            options,
                            pass.shader.is_none(),
                            Some(object),
                        )?;
                    } else {
                        self.append(&content, entity, options, Some(object), None, None, None)?;
                    }
                }
            }
        }
        for held in input.held_weapons {
            let equipped = input.presentations.iter().find(|source| {
                source.view_weapon && held.parent.actor.as_ref().is_some_and(|actor| *actor == source.actor)
            });
            let Some(equipped) = equipped else { continue };
            let declaration = self.held.declaration(equipped);
            if declaration == HeldWeaponDeclaration::None {
                continue;
            }
            let Some((tag, scale)) = model_attachment_tag(&held.parent.entity, "tag_weapon") else {
                return Err(SceneError::TorsoAttachment);
            };
            let passes: Vec<Q3CharacterPass> =
                if equipped.q3_weapon.is_some() && declaration == HeldWeaponDeclaration::Absent {
                    let presenter = selected_presenter(
                        &mut self.selected_weapons,
                        self.character_assets.as_ref(),
                        weapon_key(equipped),
                    );
                    let mut assets = WeaponAssets {
                        assets: &mut self.assets,
                    };
                    presenter.world(
                        &mut assets,
                        &weapon_source(equipped, true),
                        &Q3WeaponCharacter {
                            origin: held.parent.entity.lighting_origin,
                            powerups: 0,
                        },
                        false,
                    )?
                } else {
                    self.held.frame(
                        equipped,
                        &SceneWeaponCharacter::Simple {
                            origin: held.parent.entity.lighting_origin,
                            color: Some(vec4(1.0, 1.0, 1.0, 1.0)),
                            opacity: None,
                        },
                    )?
                };
            if passes.is_empty() {
                return Err(SceneError::HeldModel(equipped.path.clone()));
            }
            for (index, pass) in passes.iter().enumerate() {
                let content = pass.content.as_ref().unwrap_or(&equipped.content);
                let attach = |entity: &SceneEntity| {
                    let mut attached = attach_scene_entity(&held.parent.entity, entity, &tag, scale);
                    attached.flags = held.parent.entity.flags;
                    attached.lighting_origin = held.parent.entity.lighting_origin;
                    attached
                };
                if index != 0 {
                    let entity = ScenePassEntity {
                        entity: attach(&scene_entity(&pass.entity)),
                        actor: pass.entity.actor.clone(),
                        opacity: pass.entity.opacity,
                    };
                    let options = pass_options_resolver(self.character_assets.clone(), pass);
                    self.append(content, entity, options, None, None, None, None)?;
                    continue;
                }
                for source_pass in &held.passes {
                    let shader = source_pass.shader.clone();
                    let mut entity = attach(&scene_entity(&pass.entity));
                    if shader.is_some() {
                        let c = source_pass.color;
                        entity.color = vec4(c.x / 255.0, c.y / 255.0, c.z / 255.0, c.w / 255.0);
                    }
                    entity.shader_time_seconds = source_pass.shader_time;
                    let wrapped = ScenePassEntity {
                        entity,
                        actor: pass.entity.actor.clone(),
                        opacity: pass.entity.opacity,
                    };
                    let base = pass_options_resolver(self.character_assets.clone(), pass);
                    let options: OptionsResolver = Rc::new(move |current| ModelSourceOptions {
                        custom_shader: shader.clone(),
                        ..base(current)
                    });
                    let shader_content = if source_pass.shader.is_none() {
                        None
                    } else {
                        Some(&held.content)
                    };
                    self.append(content, wrapped, options, None, shader_content, None, None)?;
                }
            }
        }
        for source in input.primary_bodies {
            if let Some(actor) = &source.entity.actor {
                let key = format!(
                    "{}/{}/{}/{}",
                    actor.slot(),
                    actor.generation(),
                    source.part.as_str(),
                    source.entity.entity.resource.id
                );
                if posed_keys.insert(key) {
                    let options = source.options.clone();
                    posed.push(PosedPart {
                        part: source.part,
                        content: source.content.clone(),
                        entity: source.entity.clone(),
                        options: Rc::new(move |_| options.clone()),
                    });
                }
            }
            if !source.base
                || self
                    .bodies_impl
                    .body_base_visible(input.bodies, &source.actor, source.part)
            {
                let options = source.options.clone();
                self.append(
                    &source.content,
                    source.entity.clone(),
                    Rc::new(move |_| options.clone()),
                    None,
                    Some(&source.shader_content),
                    Some(SourceTime::Milliseconds(source.time_ms)),
                    None,
                )?;
            }
        }
        for model in posed {
            let Some(actor) = model.entity.actor.clone() else {
                continue;
            };
            for body in input.bodies {
                if body.actor != actor {
                    continue;
                }
                for pass in self.bodies_impl.body_materials(body, model.part) {
                    let mut entity = model.entity.entity.clone();
                    let c = pass.shader_rgba;
                    entity.color = vec4(c.x / 255.0, c.y / 255.0, c.z / 255.0, c.w / 255.0);
                    entity.flags = EntityFlags::Q3 {
                        bits: pass.render_flags as u32,
                    };
                    entity.shader_time_seconds = pass.shader_time;
                    entity.lighting_origin = pass.lighting_origin;
                    let wrapped = ScenePassEntity {
                        entity,
                        actor: model.entity.actor.clone(),
                        opacity: model.entity.opacity,
                    };
                    let inner = Rc::clone(&model.options);
                    let custom_shader = pass.custom_shader.map(|shader| shader.name);
                    let custom_skin = pass.custom_skin.map(|skin| skin.surfaces);
                    let shader_tex_coord = pass.shader_tex_coord;
                    let non_normalized_axes = pass.non_normalized_axes;
                    let options: OptionsResolver = Rc::new(move |current| ModelSourceOptions {
                        custom_shader: custom_shader.clone(),
                        custom_skin: custom_skin.clone(),
                        shader_tex_coord: Some(shader_tex_coord),
                        non_normalized_axes,
                        ..inner(current)
                    });
                    self.append(
                        &model.content,
                        wrapped,
                        options,
                        None,
                        Some(&body.content),
                        Some(SourceTime::Milliseconds(body.time_ms)),
                        None,
                    )?;
                }
            }
        }
        self.inline_models = inline_models;
        self.brush_models = brush_models;
        for group in &mut self.group_list {
            for pass in &group.passes {
                let options = (pass.options)(&pass.entity.entity);
                group.renderer.preload(&pass.entity.entity, &options)?;
            }
        }
        Ok(())
    }

    /// Prepare one pass for the view (donor `prepare` inside `modelOperations`).
    fn prepare_pass(
        &self,
        reference: PassRef,
        input: &WorldViewInput,
        infrared: bool,
        weapon_camera: &SceneCamera,
        skinning: &ModelSkinningFrame,
    ) -> Result<Vec<SceneModelGroup>, SceneError> {
        let pass = &self.group_list[reference.group].passes[reference.pass];
        let entity = match &pass.resolve {
            None => Some(pass.entity.entity.clone()),
            Some(resolve) => (resolve.resolve)(input.camera.origin, HeldResolveMode::View),
        };
        let Some(entity) = entity else { return Ok(Vec::new()) };
        let mut options = (pass.options)(&entity);
        if options.view_model && matches!(input.camera.clip, CameraClip::Portal { .. }) {
            return Ok(Vec::new());
        }
        let current;
        let current = match &pass.time {
            None => input,
            Some(time) => {
                current = WorldViewInput {
                    time: render_time(time),
                    ..input.clone()
                };
                &current
            }
        };
        let scoped;
        let scoped = if options.view_model {
            scoped = WorldViewInput {
                camera: *weapon_camera,
                ..current.clone()
            };
            &scoped
        } else {
            current
        };
        options.infrared = infrared;
        options.no_world_model = input.no_world_model;
        options.planar_shadow = !input.no_world_model && (self.planar_shadows)();
        self.group_list[reference.group]
            .renderer
            .prepare(&entity, scoped, &options, skinning)
    }

    /// Model, flare, beam, and brush operations (donor `modelOperations`).
    fn model_operations(
        &mut self,
        input: &WorldViewInput,
        infrared: bool,
        weapon_camera: &SceneCamera,
        skinning: &ModelSkinningFrame,
    ) -> Result<Vec<SceneOperation>, SceneError> {
        let mut model_operations = Vec::new();
        let mut emitted = HashSet::new();
        for reference in self.ordered.clone() {
            let object = self.objects.get(&reference).copied();
            let direct = object.is_none_or(|object| self.object_list[object].opacity == 1.0);
            if direct {
                model_operations.extend(
                    self.prepare_pass(reference, input, infrared, weapon_camera, skinning)?
                        .into_iter()
                        .map(SceneOperation::Group),
                );
                continue;
            }
            let object = object.expect("translucent pass has an object");
            if !emitted.insert(object) {
                continue;
            }
            let opacity = self.object_list[object].opacity;
            if opacity != 0.0 {
                let mut batches = Vec::new();
                for pass in self.object_list[object].passes.clone() {
                    batches.extend(scene_model_batches(&self.prepare_pass(
                        pass,
                        input,
                        infrared,
                        weapon_camera,
                        skinning,
                    )?));
                }
                model_operations.push(SceneOperation::Group(SceneGroup {
                    order: SceneGroupOrder::Sequence {
                        phase: SequencePhase::Translucent,
                    },
                    operations: vec![RenderOperation::ObjectOpacity { opacity, batches }],
                }));
            }
        }
        if !self.flares.is_empty() {
            let mut batches = Vec::new();
            for flare in self.flares.clone() {
                batches.push(self.world.prepare_flare(
                    &flare.flare,
                    flare.origin,
                    &input.camera,
                    &flare.image,
                    &flare.image_path,
                )?);
            }
            model_operations.push(SceneOperation::Group(sequence_draw_group(
                SequencePhase::Translucent,
                batches,
            )));
        }
        for beam in self.shader_beams.clone() {
            let batches = self.world.prepare_shader_beam(
                &beam.material,
                beam.origin,
                beam.end,
                beam.width,
                input.camera.origin,
                input,
            )?;
            model_operations.push(SceneOperation::Group(sequence_draw_group(
                SequencePhase::Translucent,
                batches,
            )));
        }
        let mut brushes = Vec::new();
        for brush in self.brush_models.clone() {
            brushes.extend(self.world.prepare_brush_model(
                brush.scene,
                brush.model,
                &brush.transform,
                brush.frame,
                brush.q1_bsp && brush.frame != 0,
                [255, 255, 255, (brush.alpha * 255.0) as u8],
                input,
            )?);
        }
        brushes.extend(model_operations);
        Ok(brushes)
    }

    /// Prepare the full view with shadows and world operations (donor `view`).
    pub fn view(
        &mut self,
        mut input: WorldViewInput,
        operations: &[SceneOperation],
        shadow_lights: &[SceneLight],
        infrared: bool,
        weapon_camera: Option<SceneCamera>,
    ) -> Result<PreparedWorldView, SceneError> {
        let weapon_camera = weapon_camera.unwrap_or(input.camera);
        if input.source.is_none() {
            let order = create_source_scene_order(self.assets.material_registrations());
            input.source = Some(create_world_surface_admission(order));
        }
        input.inline_models.clone_from(&self.inline_models);
        let (q1_styles, q2_styles) = self.styles();
        input.q1_styles = q1_styles;
        input.q2_styles = q2_styles;
        let skinning = ModelSkinningFrame::default();
        if !shadow_lights.is_empty() && !input.no_world_model {
            let mut casters = Vec::new();
            for (group_index, group) in self.group_list.iter().enumerate() {
                for (pass_index, pass) in group.passes.iter().enumerate() {
                    let reference = PassRef {
                        group: group_index,
                        pass: pass_index,
                    };
                    let opacity = self
                        .objects
                        .get(&reference)
                        .map(|object| self.object_list[*object].opacity)
                        .unwrap_or(1.0);
                    if opacity != 1.0 {
                        continue;
                    }
                    let entity = match &pass.resolve {
                        None => Some(pass.entity.entity.clone()),
                        Some(resolve) => (resolve.resolve)(input.camera.origin, HeldResolveMode::Shadow),
                    };
                    let Some(entity) = entity else { continue };
                    let current;
                    let current = match &pass.time {
                        None => &input,
                        Some(time) => {
                            current = WorldViewInput {
                                time: render_time(time),
                                ..input.clone()
                            };
                            &current
                        }
                    };
                    let options = (pass.options)(&entity);
                    casters.extend(group.renderer.prepare_shadow_caster(
                        &entity,
                        current,
                        &options,
                        &skinning,
                        shadow_lights,
                    )?);
                }
            }
            let mut lights = shadow_lights.to_vec();
            lights.extend(input.lights.iter().map(|light| SceneLight {
                origin: light.origin,
                radius: light.radius,
                color: light.color,
                additive: true,
                profile: LightProfile::Q2 {
                    scale: 1.0,
                    cone: None,
                    shadow: LightShadow::None,
                },
            }));
            let shadows = self.world.prepare_shadows(&lights, &input, &casters)?;
            input.q2_fragment_lighting = Some(shadows.lighting);
            input.before_view = shadows.operations;
        }
        self.world.prepare_world_operations(&mut input)?;
        let model_operations = self.model_operations(&input, infrared, &weapon_camera, &skinning)?;
        let (mut merged, rest): (Vec<SceneOperation>, Vec<SceneOperation>) =
            operations.iter().cloned().partition(is_world_polygon);
        merged.extend(model_operations);
        merged.extend(rest);
        input.operations = merged;
        self.world.prepare_view(input)
    }

    /// Inline-model and model operations without a full view (donor `supplemental`).
    ///
    /// Per-model animation overrides ride on a scoped input copy, like the donor's
    /// spread.
    pub fn supplemental(
        &mut self,
        input: &WorldViewInput,
        weapon_camera: Option<SceneCamera>,
        infrared: bool,
    ) -> Result<Vec<SceneOperation>, SceneError> {
        let weapon_camera = weapon_camera.unwrap_or(input.camera);
        let mut operations = Vec::new();
        for model in self.inline_models.clone() {
            let mut scoped = input.clone();
            if let Some(frame) = model.animation_frame {
                scoped.animation_frame = Some(frame);
            }
            if let Some(alternate) = model.alternate_animation {
                scoped.alternate_animation = alternate;
            }
            operations.extend(self.world.prepare_model(&model, &scoped)?);
        }
        operations.extend(self.model_operations(input, infrared, &weapon_camera, &ModelSkinningFrame::default())?);
        Ok(operations)
    }

    /// Release groups, caches, and beams (donor `close`).
    pub fn close(&mut self) {
        self.ordered.clear();
        self.objects.clear();
        self.object_list.clear();
        self.group_list.clear();
        self.groups.clear();
        self.characters.clear();
        self.selected_weapons.clear();
        self.shader_beams.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_client::materials::compile::{
        default_shader_profile, shader_render_material, CompiledMaterial, RegisteredExplicitShader, RegisteredStage,
        RegistrationOutcome,
    };
    use qa_client::materials::finish::{finish_implicit_shader, FinishImplicitShaderInput, ImplicitShaderKind};
    use qa_client::materials::material::ShaderDefinition;
    use qa_client::materials::state::CullFace as MaterialCullFace;
    use qa_client::render::scene::material_registrations::MaterialRegistrationTable;
    use qa_client::render::types::{
        AlphaTest, BatchLighting, BatchPrimitive, BatchVertices, BlendFactor, CullFace, DepthTest, ImageSource,
        RenderState, RenderView, RenderViewState, ResourceOwner, TextureBinding, ViewTarget,
    };
    use qa_client::view::Rect;
    use qa_content::contract::{
        LooseMount, MountId, MountIdentity, MountPlanId, ResourceId, ResourceProvenance, ResourceResolution,
    };
    use qa_content::md3::{Md3Model, SkinSurface};
    use qa_content::q3::foundation::animation_config::parse_player_animation_config;
    use qa_content::q3::foundation::arsenal::q3_spawn_animation;
    use qa_content::q3::foundation::assets::{Q3CharacterPart, Q3CharacterSelection};
    use qa_content::q3scene::SceneMd3;
    use qa_core::identity::{IdentityOwner, ProviderId};
    use qa_core::math::identity_mat4;
    use qa_core::time::{FrameContext, FramePhase};
    use std::cell::RefCell;

    #[derive(Debug, Default)]
    struct RenderCounters {
        preloads: usize,
        prepares: usize,
        casters: usize,
    }

    #[derive(Clone, Default)]
    struct StubFactory {
        counters: Rc<RefCell<RenderCounters>>,
    }

    struct StubRenderer {
        counters: Rc<RefCell<RenderCounters>>,
    }

    impl SceneRendererFactory for StubFactory {
        type Renderer = StubRenderer;

        fn make_renderer(
            &mut self,
            _content: &ContentId,
            _shader_content: Option<&ContentId>,
        ) -> Result<StubRenderer, SceneError> {
            Ok(StubRenderer {
                counters: Rc::clone(&self.counters),
            })
        }
    }

    impl SceneRenderer for StubRenderer {
        fn preload(&mut self, _entity: &SceneEntity, _options: &ModelSourceOptions) -> Result<(), SceneError> {
            self.counters.borrow_mut().preloads += 1;
            Ok(())
        }

        fn prepare(
            &self,
            _entity: &SceneEntity,
            _input: &WorldViewInput,
            _options: &ModelSourceOptions,
            _skinning: &ModelSkinningFrame,
        ) -> Result<Vec<SceneModelGroup>, SceneError> {
            self.counters.borrow_mut().prepares += 1;
            Ok(vec![SceneGroup {
                order: SceneGroupOrder::Sequence {
                    phase: SequencePhase::Opaque,
                },
                operations: Vec::new(),
            }])
        }

        fn prepare_shadow_caster(
            &self,
            entity: &SceneEntity,
            _input: &WorldViewInput,
            _options: &ModelSourceOptions,
            _skinning: &ModelSkinningFrame,
            _shadow_lights: &[SceneLight],
        ) -> Result<Vec<ModelShadowCaster>, SceneError> {
            self.counters.borrow_mut().casters += 1;
            Ok(vec![ModelShadowCaster {
                origin: entity.lighting_origin,
                meshes: Vec::new(),
            }])
        }
    }

    struct StubAssets {
        brush: Option<SceneBrushRef>,
        brush_missing: bool,
        model_flags: i32,
        flare: Option<RendererImage>,
        flare_missing: Vec<String>,
        material: Option<RegisteredSceneMaterial>,
        appearance: ContentId,
    }

    impl StubAssets {
        fn new(material: RegisteredSceneMaterial) -> Self {
            Self {
                brush: None,
                brush_missing: false,
                model_flags: 0,
                flare: None,
                flare_missing: Vec::new(),
                material: Some(material),
                appearance: ContentId("q3:test:appearance:1".to_string()),
            }
        }
    }

    impl SceneModelAssets for StubAssets {
        fn model(&mut self, content: &ContentId, path: &str) -> Result<SceneModelAsset, SceneError> {
            let brush_model = self.brush.is_some() || self.brush_missing;
            Ok(SceneModelAsset {
                resource: ModelResource {
                    id: format!("{content}/{path}"),
                    requested_path: path.to_string(),
                    digest: 1,
                },
                model: if brush_model {
                    SceneModel::BrushModel
                } else {
                    SceneModel::Q3Md3(empty_md3("test"))
                },
                model_flags: self.model_flags,
                brush: self.brush.clone(),
            })
        }

        fn open(&mut self, _content: &ContentId, _path: &str) -> Option<Vec<u8>> {
            None
        }

        fn q3_model(&mut self, _content: &ContentId, _path: &str) -> Option<Q3WeaponModelAsset> {
            None
        }

        fn flare_texture(&mut self, _content: &ContentId, path: &str) -> Option<RendererImage> {
            if self.flare_missing.iter().any(|missing| missing == path) {
                return None;
            }
            self.flare.clone()
        }

        fn register_shader(
            &mut self,
            _content: &ContentId,
            _path: &str,
        ) -> Result<RegisteredSceneMaterial, SceneError> {
            self.material
                .clone()
                .ok_or_else(|| SceneError::Host("no material".to_string()))
        }

        fn appearance_content(&self) -> ContentId {
            self.appearance.clone()
        }

        fn material_registrations(&mut self) -> Vec<ShaderRegistration> {
            Vec::new()
        }
    }

    #[derive(Default)]
    struct StubWorld {
        shadow_calls: usize,
        world_ops: usize,
        views: Vec<WorldViewInput>,
        models: Vec<(usize, Option<f32>, Option<bool>)>,
        brushes: Vec<(u64, i32, i32, bool, [u8; 4])>,
        flares: usize,
        beams: usize,
    }

    impl SceneWorld for StubWorld {
        fn prepare_shadows(
            &mut self,
            _lights: &[SceneLight],
            _input: &WorldViewInput,
            _casters: &[ModelShadowCaster],
        ) -> Result<SceneShadowResult, SceneError> {
            self.shadow_calls += 1;
            Ok(SceneShadowResult {
                lighting: qa_client::render::scene::world::Q2FragmentLighting {
                    lights: Vec::new(),
                    atlas: None,
                },
                operations: Vec::new(),
            })
        }

        fn prepare_world_operations(&mut self, _input: &mut WorldViewInput) -> Result<(), SceneError> {
            self.world_ops += 1;
            Ok(())
        }

        fn prepare_view(&mut self, input: WorldViewInput) -> Result<PreparedWorldView, SceneError> {
            self.views.push(input.clone());
            let viewport = input.camera.viewport;
            Ok(PreparedWorldView {
                image_operations: Vec::new(),
                view: RenderView {
                    state: RenderViewState {
                        viewport: qa_client::render::types::Rect {
                            x: viewport.x as f32,
                            y: viewport.y as f32,
                            width: viewport.width as f32,
                            height: viewport.height as f32,
                        },
                        clear: None,
                        clip_plane: None,
                    },
                    target: ViewTarget::Preview("test".to_string()),
                    time: RenderSourceTime::Seconds(0.0),
                    before_view: Vec::new(),
                    operations: Vec::new(),
                },
            })
        }

        fn prepare_model(
            &mut self,
            model: &InlineModel,
            _input: &WorldViewInput,
        ) -> Result<Vec<SceneOperation>, SceneError> {
            self.models
                .push((model.model, model.animation_frame, model.alternate_animation));
            Ok(Vec::new())
        }

        fn prepare_brush_model(
            &mut self,
            scene: u64,
            model: i32,
            _transform: &ModelTransform,
            frame: i32,
            alternate_animation: bool,
            entity_rgba: [u8; 4],
            _input: &WorldViewInput,
        ) -> Result<Vec<SceneOperation>, SceneError> {
            self.brushes
                .push((scene, model, frame, alternate_animation, entity_rgba));
            Ok(Vec::new())
        }

        fn prepare_flare(
            &mut self,
            _flare: &SceneFlare,
            _origin: Vec3,
            _camera: &SceneCamera,
            _image: &RendererImage,
            _image_path: &str,
        ) -> Result<DrawBatch, SceneError> {
            self.flares += 1;
            Ok(DrawBatch {
                fog: None,
                luminance_alpha: false,
                indices: Vec::new(),
                texture: TextureBinding::BindImage(test_image()),
                state: RenderState {
                    blend: (BlendFactor::SrcAlpha, BlendFactor::OneMinusSrcAlpha),
                    depth_test: DepthTest::Always,
                    depth_write: false,
                    alpha_test: AlphaTest::None,
                    cull: CullFace::None,
                    depth_range: [0.0, 1.0],
                    polygon_offset: None,
                },
                lighting: BatchLighting::Vertex,
                primitive: BatchPrimitive::Triangles,
                vertices: BatchVertices::Single(Vec::new()),
            })
        }

        fn prepare_shader_beam(
            &mut self,
            _material: &RegisteredSceneMaterial,
            _origin: Vec3,
            _end: Vec3,
            _width: f32,
            _camera_origin: Vec3,
            _input: &WorldViewInput,
        ) -> Result<Vec<DrawBatch>, SceneError> {
            self.beams += 1;
            Ok(Vec::new())
        }
    }

    struct StubHeld {
        declaration: HeldWeaponDeclaration,
        passes: Vec<Q3CharacterPass>,
        frames: usize,
        resolve_none: bool,
    }

    impl StubHeld {
        fn new() -> Self {
            Self {
                declaration: HeldWeaponDeclaration::Absent,
                passes: Vec::new(),
                frames: 0,
                resolve_none: false,
            }
        }
    }

    impl SceneHeldWeapons for StubHeld {
        fn declaration(&mut self, _equipped: &ScenePresentation) -> HeldWeaponDeclaration {
            self.declaration
        }

        fn frame(
            &mut self,
            _equipped: &ScenePresentation,
            _character: &SceneWeaponCharacter,
        ) -> Result<Vec<Q3CharacterPass>, SceneError> {
            self.frames += 1;
            Ok(self.passes.clone())
        }

        fn attachment(&mut self, _content: &ContentId, _entity: &SceneEntity) -> HeldAttachmentFactory {
            let none = self.resolve_none;
            HeldAttachmentFactory {
                attach: Box::new(move |entity: &SceneEntity| {
                    let entity = entity.clone();
                    HeldAttachment {
                        resolve: Box::new(move |_, _| if none { None } else { Some(entity.clone()) }),
                    }
                }),
            }
        }
    }

    struct StubBodies {
        base_visible: bool,
        materials: Vec<SceneBodyMaterial>,
    }

    impl StubBodies {
        fn new() -> Self {
            Self {
                base_visible: true,
                materials: Vec::new(),
            }
        }
    }

    impl SceneBodies for StubBodies {
        fn body_base_visible(&self, _bodies: &[SceneBody], _actor: &ActorId, _part: SceneBodyPart) -> bool {
            self.base_visible
        }

        fn body_materials(&self, _body: &SceneBody, _part: SceneBodyPart) -> Vec<SceneBodyMaterial> {
            self.materials.clone()
        }
    }

    type TestScene = ApplicationWorldScene<StubAssets, StubFactory, StubWorld, StubHeld, StubBodies>;

    fn test_material() -> RegisteredSceneMaterial {
        let definition = ShaderDefinition {
            name: "beam".to_string(),
            stages: Vec::new(),
            surface_parms: Vec::new(),
            cull: MaterialCullFace::Back,
            sort: None,
            sky: None,
            fog: None,
            sun: None,
            deforms: Vec::new(),
            polygon_offset: false,
            no_mipmaps: false,
            no_picmip: false,
            entity_mergable: false,
            portal_range: 0.0,
            clamp_time: 0.0,
            warnings: Vec::new(),
            compiler_directives: Vec::new(),
        };
        let finished = finish_implicit_shader(&FinishImplicitShaderInput {
            name: "beam".to_string(),
            base_image: RegisteredStage::Missing,
            profile: default_shader_profile(),
            kind: ImplicitShaderKind::Default,
        })
        .expect("implicit finish");
        let mut table = MaterialRegistrationTable::default();
        table.admit(CompiledMaterial {
            registered: RegisteredExplicitShader {
                definition: definition.clone(),
                stages: Vec::new(),
                sky: None,
                outcome: RegistrationOutcome::Defined,
            },
            finished,
            material: shader_render_material(&definition).expect("view"),
        })
    }

    fn test_image() -> RendererImage {
        let authority = IdentityOwner::create("scene-test").unwrap();
        RendererImage {
            owner: ResourceOwner::new(1, authority.session().clone(), 0),
            ordinal: 3,
            source: ImageSource::Generated {
                name: "flare".to_string(),
            },
            width: 64,
            height: 64,
        }
    }

    fn content() -> ContentId {
        ContentId("q3:test:scene:1".to_string())
    }

    fn empty_md3(name: &str) -> SceneMd3 {
        SceneMd3 {
            name: name.to_string(),
            source_model: Md3Model {
                name: name.to_string(),
                flags: 0,
                skin_count: 0,
                frames: Vec::new(),
                tags: Vec::new(),
                surfaces: Vec::new(),
            },
            frames: Vec::new(),
            tags: Vec::new(),
            surfaces: Vec::new(),
        }
    }

    fn dummy_reference(path: &str) -> ResolvedResourceReference {
        ResolvedResourceReference {
            id: ResourceId(format!("resource:test:{path}")),
            requested_path: path.to_string(),
            provenance: ResourceProvenance::Loose {
                mount: LooseMount {
                    identity: MountIdentity {
                        id: MountId("mount:test:loose".to_string()),
                        content: ContentId("q3:test:pkg:1".to_string()),
                        generation: 0,
                    },
                    root_path: "/test".to_string(),
                },
                member_path: path.to_string(),
            },
            digest: ContentDigest("sha256:0123456789abcdef0123456789abcdef".to_string()),
            byte_length: 8,
            resolution: ResourceResolution::DefaultOrder {
                plan: MountPlanId("mount-plan:test:p".to_string()),
                rank: 0,
            },
        }
    }

    fn presentation(actor: ActorId) -> ScenePresentation {
        ScenePresentation {
            actor,
            content: content(),
            family: GameFamily::Q3,
            path: "models/test.md3".to_string(),
            frame: 0,
            old_frame: 0,
            back_lerp: None,
            skin: 0,
            skin_path: None,
            indexed_skin: None,
            player_colors: None,
            render_flags: 0,
            origin: vec3(0.0, 0.0, 0.0),
            previous_origin: None,
            model_beam: None,
            shader_beam: None,
            flare: None,
            model_attachments: Vec::new(),
            model_anchor: None,
            grapple_cable: None,
            angles: vec3(0.0, 0.0, 0.0),
            scale: 1.0,
            alpha: None,
            visible: true,
            view_weapon: false,
            q3_weapon: None,
            native_held_weapon: false,
            held_weapon: false,
        }
    }

    fn snapshot(time: SourceTime) -> WorldSnapshot {
        WorldSnapshot {
            frame: FrameContext {
                frame: 1,
                time,
                elapsed: SourceTime::Milliseconds(16),
                phase: FramePhase::FrameEntry,
            },
            actors: Vec::new(),
            bodies: Vec::new(),
            inventories: Vec::new(),
        }
    }

    fn camera() -> SceneCamera {
        SceneCamera {
            origin: vec3(0.0, 0.0, 0.0),
            axis: angles_to_axis(vec3(0.0, 0.0, 0.0)),
            projection: identity_mat4(),
            viewport: Rect {
                x: 0,
                y: 0,
                width: 640,
                height: 480,
            },
            clip: CameraClip::None,
        }
    }

    fn test_scene() -> (TestScene, Rc<RefCell<RenderCounters>>) {
        test_scene_with(StubAssets::new(test_material()), StubHeld::new())
    }

    fn test_scene_with(assets: StubAssets, held: StubHeld) -> (TestScene, Rc<RefCell<RenderCounters>>) {
        let factory = StubFactory::default();
        let counters = Rc::clone(&factory.counters);
        let scene = ApplicationWorldScene::new(
            assets,
            factory,
            StubWorld::default(),
            held,
            StubBodies::new(),
            None,
            || false,
        );
        (scene, counters)
    }

    fn q3_entity(actor: ActorId) -> Q3SceneEntity {
        Q3SceneEntity {
            actor: Some(actor),
            resource: dummy_reference("models/held.md3"),
            model: Q3PresentedModel::Md3(empty_md3("held")),
            opacity: 0.5,
            transform: ModelTransform {
                origin: vec3(1.0, 2.0, 3.0),
                axis: angles_to_axis(vec3(0.0, 0.0, 0.0)),
                scale: vec3(1.0, 1.0, 1.0),
            },
            previous_origin: vec3(0.0, 0.0, 0.0),
            pose: Q3ModelPose::Frame {
                frame: 0,
                previous_frame: 0,
                back_lerp: 0.0,
            },
            skin: 0,
            color: vec4(1.0, 1.0, 1.0, 1.0),
            shader_time: SourceTime::Seconds(0.0),
            flags: 0,
            lighting_origin: vec3(1.0, 2.0, 3.0),
            shadow_plane: 0.0,
            attachments: Vec::new(),
        }
    }

    fn held_pass(actor: ActorId) -> Q3CharacterPass {
        Q3CharacterPass {
            content: None,
            entity: q3_entity(actor),
            shader: None,
            options: Q3PassOptions::Foreign(Rc::new(|_| FoundationModelSourceOptions::default())),
        }
    }

    fn lightstyle_event(
        style: i32,
        pattern: &str,
        owner: Option<PresentationOwner>,
    ) -> SimulationPresentationEvent<()> {
        SimulationPresentationEvent {
            source: SourcePresentationEvent::Q1(Q1PresentationEvent::Lightstyle {
                style,
                pattern: pattern.to_string(),
            }),
            owner,
            recipient: None,
            sequence: 1,
            content: content(),
            seconds: 0.0,
            source_entity: None,
        }
    }

    #[test]
    fn seat_model_visibility_matrix() {
        let owner = IdentityOwner::create("scene-visibility").unwrap();
        let actor = owner.actor(1, 1);
        let other = owner.actor(2, 1);
        assert!(seat_model_visible(Some(&actor), &actor, true));
        assert!(!seat_model_visible(Some(&other), &actor, true));
        assert!(!seat_model_visible(None, &actor, true));
        assert!(!seat_model_visible(Some(&actor), &actor, false));
        assert!(seat_model_visible(Some(&other), &actor, false));
        assert!(seat_model_visible(None, &actor, false));
    }

    #[test]
    fn receive_lightstyle_and_retire() {
        let (mut scene, _) = test_scene();
        let token = PresentationOwner {
            provider: ProviderId::new("test", "scene"),
            generation: 1,
        };
        scene.receive(&[lightstyle_event(3, "mmnmm", Some(token.clone()))]);
        assert_eq!(scene.style(3, 99.0), 12.0);
        assert_eq!(scene.style(4, 99.0), 99.0);
        scene.receive(&[SimulationPresentationEvent::<()> {
            source: SourcePresentationEvent::PresentationOwner {
                event: OwnerLifecycle::Retired { owner: token },
            },
            owner: None,
            recipient: None,
            sequence: 2,
            content: content(),
            seconds: 0.0,
            source_entity: None,
        }]);
        assert_eq!(scene.style(3, 99.0), 99.0);
    }

    #[test]
    fn style_tables_cover_absent_and_empty() {
        let (mut scene, _) = test_scene();
        scene.receive(&[lightstyle_event(3, "mmnmm", None)]);
        scene.receive(&[lightstyle_event(5, "", None)]);
        let (q1, q2) = scene.styles();
        assert_eq!(q1.len(), 256);
        assert_eq!(q1[3], 12 * 22);
        assert_eq!(q1[4], 256);
        assert_eq!(q1[5], 12 * 22);
        assert_eq!(q2[3].rgb, vec3(1.0, 1.0, 1.0));
        assert_eq!(q2[3].white, 3.0);
    }

    #[test]
    fn prepare_appends_simple_pass() {
        let (mut scene, counters) = test_scene();
        let owner = IdentityOwner::create("scene-simple").unwrap();
        let snap = snapshot(SourceTime::Seconds(1.0));
        let presentations = vec![presentation(owner.actor(1, 1))];
        scene
            .prepare(&ScenePrepareInput::new(None, &snap, &presentations, &[]))
            .unwrap();
        assert_eq!(scene.prepared_time, 1.0);
        assert_eq!(scene.ordered.len(), 1);
        assert_eq!(scene.group_list.len(), 1);
        assert_eq!(counters.borrow().preloads, 1);
        let pass = &scene.group_list[0].passes[0];
        let options = (pass.options)(&pass.entity.entity);
        assert!(!options.view_model);
        assert!(!options.player);
        assert_eq!(options.custom_shader, None);
        assert_eq!(pass.entity.entity.actor_slot, Some(1));
        assert_eq!(pass.entity.actor, Some(owner.actor(1, 1)));
        assert_eq!(pass.entity.opacity, 1.0);
    }

    #[test]
    fn prepare_skips_hidden_and_view_weapons() {
        let owner = IdentityOwner::create("scene-skip").unwrap();
        let actor = owner.actor(1, 1);
        let snap = snapshot(SourceTime::Seconds(1.0));
        let mut hidden = presentation(actor.clone());
        hidden.visible = false;
        let (mut scene, _) = test_scene();
        scene
            .prepare(&ScenePrepareInput::new(None, &snap, &[hidden], &[]))
            .unwrap();
        assert!(scene.ordered.is_empty());

        let mut weapon = presentation(actor.clone());
        weapon.view_weapon = true;
        let (mut scene, _) = test_scene();
        let weapons = [weapon.clone()];
        let mut input = ScenePrepareInput::new(Some(&actor), &snap, &weapons, &[]);
        input.view_weapon_visible = false;
        scene.prepare(&input).unwrap();
        assert!(scene.ordered.is_empty());

        let (mut scene, _) = test_scene();
        scene
            .prepare(&ScenePrepareInput::new(Some(&actor), &snap, &[weapon], &[]))
            .unwrap();
        assert_eq!(scene.ordered.len(), 1);
    }

    #[test]
    fn prepare_shader_beam_and_flare_fallback() {
        let owner = IdentityOwner::create("scene-beam").unwrap();
        let snap = snapshot(SourceTime::Seconds(1.0));
        let mut beam = presentation(owner.actor(1, 1));
        beam.shader_beam = Some(SceneShaderBeam {
            path: "beam".to_string(),
            end: vec3(1.0, 0.0, 0.0),
            width: 2.0,
        });
        let mut flare = presentation(owner.actor(2, 1));
        flare.flare = Some(SceneFlare {
            image: "custom.tga".to_string(),
            fade_start: 0.0,
            fade_end: 1.0,
            scale: 1.0,
            color: vec3(1.0, 1.0, 1.0),
            rim_color: None,
            lock_angle: false,
        });
        let mut assets = StubAssets::new(test_material());
        assets.flare = Some(test_image());
        assets.flare_missing = vec!["custom.tga".to_string()];
        let (mut scene, _) = test_scene_with(assets, StubHeld::new());
        scene
            .prepare(&ScenePrepareInput::new(None, &snap, &[beam, flare], &[]))
            .unwrap();
        assert!(scene.ordered.is_empty());
        assert_eq!(scene.shader_beams.len(), 1);
        assert_eq!(scene.flares.len(), 1);
        assert_eq!(scene.flares[0].image_path, "misc/flare.tga");
    }

    #[test]
    fn prepare_missing_flare_skips() {
        let owner = IdentityOwner::create("scene-noflare").unwrap();
        let snap = snapshot(SourceTime::Seconds(1.0));
        let mut flare = presentation(owner.actor(1, 1));
        flare.flare = Some(SceneFlare {
            image: "custom.tga".to_string(),
            fade_start: 0.0,
            fade_end: 1.0,
            scale: 1.0,
            color: vec3(1.0, 1.0, 1.0),
            rim_color: None,
            lock_angle: false,
        });
        let mut assets = StubAssets::new(test_material());
        assets.flare_missing = vec!["custom.tga".to_string(), "misc/flare.tga".to_string()];
        let (mut scene, _) = test_scene_with(assets, StubHeld::new());
        scene
            .prepare(&ScenePrepareInput::new(None, &snap, &[flare], &[]))
            .unwrap();
        assert!(scene.flares.is_empty());
    }

    #[test]
    fn prepare_grapple_cable_segments() {
        let owner = IdentityOwner::create("scene-cable").unwrap();
        let actor = owner.actor(1, 1);
        let snap = snapshot(SourceTime::Seconds(1.0));
        let mut source = presentation(actor.clone());
        source.origin = vec3(0.0, 0.0, 100.0);
        source.grapple_cable = Some(SceneGrappleCable {
            owner: actor,
            owner_origin: vec3(0.0, 0.0, 0.0),
            owner_angles: vec3(0.0, 0.0, 0.0),
            view_height: 26.0,
            offhand: false,
            attached: true,
            flight: "flight.md3".to_string(),
            pull: "pull.md3".to_string(),
            hold: "hold.md3".to_string(),
            segment_length: 32.0,
        });
        let (mut scene, _) = test_scene();
        scene
            .prepare(&ScenePrepareInput::new(None, &snap, &[source], &[]))
            .unwrap();
        assert_eq!(scene.ordered.len(), 4);
    }

    #[test]
    fn prepare_grapple_cable_limit_errors() {
        let owner = IdentityOwner::create("scene-cable-limit").unwrap();
        let actor = owner.actor(1, 1);
        let snap = snapshot(SourceTime::Seconds(1.0));
        let mut source = presentation(actor.clone());
        source.origin = vec3(0.0, 0.0, 100000.0);
        source.grapple_cable = Some(SceneGrappleCable {
            owner: actor,
            owner_origin: vec3(0.0, 0.0, 0.0),
            owner_angles: vec3(0.0, 0.0, 0.0),
            view_height: 26.0,
            offhand: false,
            attached: true,
            flight: "flight.md3".to_string(),
            pull: "pull.md3".to_string(),
            hold: "hold.md3".to_string(),
            segment_length: 1.0,
        });
        let (mut scene, _) = test_scene();
        assert!(matches!(
            scene.prepare(&ScenePrepareInput::new(None, &snap, &[source], &[])),
            Err(SceneError::CableLimit)
        ));
    }

    #[test]
    fn prepare_brush_models_split_inline_and_foreign() {
        let owner = IdentityOwner::create("scene-brush").unwrap();
        let snap = snapshot(SourceTime::Seconds(1.0));
        let mut assets = StubAssets::new(test_material());
        assets.brush = Some(SceneBrushRef {
            model: 2,
            scene: None,
            q1_bsp: false,
        });
        let (mut scene, _) = test_scene_with(assets, StubHeld::new());
        scene
            .prepare(&ScenePrepareInput::new(
                None,
                &snap,
                &[presentation(owner.actor(1, 1))],
                &[],
            ))
            .unwrap();
        assert_eq!(scene.inline_models.len(), 1);
        assert_eq!(scene.inline_models[0].model, 2);
        assert_eq!(scene.inline_models[0].animation_frame, Some(0.0));
        assert_eq!(scene.inline_models[0].entity_rgba, Some([255, 255, 255, 255]));
        assert!(scene.brush_models.is_empty());

        let mut assets = StubAssets::new(test_material());
        assets.brush = Some(SceneBrushRef {
            model: 3,
            scene: Some(7),
            q1_bsp: true,
        });
        let (mut scene, _) = test_scene_with(assets, StubHeld::new());
        scene
            .prepare(&ScenePrepareInput::new(
                None,
                &snap,
                &[presentation(owner.actor(1, 1))],
                &[],
            ))
            .unwrap();
        assert!(scene.inline_models.is_empty());
        assert_eq!(scene.brush_models.len(), 1);
        assert_eq!(scene.brush_models[0].scene, 7);
    }

    #[test]
    fn prepare_brush_without_scene_errors() {
        let owner = IdentityOwner::create("scene-brush-missing").unwrap();
        let snap = snapshot(SourceTime::Seconds(1.0));
        let mut assets = StubAssets::new(test_material());
        assets.brush_missing = true;
        let (mut scene, _) = test_scene_with(assets, StubHeld::new());
        assert!(matches!(
            scene.prepare(&ScenePrepareInput::new(
                None,
                &snap,
                &[presentation(owner.actor(1, 1))],
                &[]
            )),
            Err(SceneError::BrushScene(_))
        ));
    }

    #[test]
    fn prepare_q1_spin_flags_gate_axis() {
        let owner = IdentityOwner::create("scene-spin").unwrap();
        let snap = snapshot(SourceTime::Seconds(1.0));
        let mut source = presentation(owner.actor(1, 1));
        source.family = GameFamily::Q1;
        source.angles = vec3(0.0, 30.0, 0.0);
        let mut assets = StubAssets::new(test_material());
        assets.model_flags = 8;
        let (mut scene, _) = test_scene_with(assets, StubHeld::new());
        scene
            .prepare(&ScenePrepareInput::new(None, &snap, &[source], &[]))
            .unwrap();
        let spun = scene.group_list[0].passes[0].entity.entity.transform.axis;
        assert_ne!(spun, angles_to_axis(vec3(0.0, 30.0, 0.0)));

        let mut source = presentation(owner.actor(1, 1));
        source.family = GameFamily::Q1;
        source.angles = vec3(0.0, 30.0, 0.0);
        let (mut scene, _) = test_scene();
        scene
            .prepare(&ScenePrepareInput::new(None, &snap, &[source], &[]))
            .unwrap();
        assert_eq!(
            scene.group_list[0].passes[0].entity.entity.transform.axis,
            angles_to_axis(vec3(0.0, 30.0, 0.0))
        );
    }

    #[test]
    fn prepare_native_held_weapon_resolves() {
        let owner = IdentityOwner::create("scene-native").unwrap();
        let actor = owner.actor(1, 1);
        let snap = snapshot(SourceTime::Seconds(1.0));
        let mut native = presentation(actor.clone());
        native.native_held_weapon = true;
        let mut equipped = presentation(actor.clone());
        equipped.view_weapon = true;
        equipped.path = "guns/shotgun.md3".to_string();
        equipped.held_weapon = true;
        let mut held = StubHeld::new();
        held.declaration = HeldWeaponDeclaration::Present;
        held.passes = vec![held_pass(actor.clone())];
        let (mut scene, _) = test_scene_with(StubAssets::new(test_material()), held);
        scene
            .prepare(&ScenePrepareInput::new(None, &snap, &[native, equipped], &[]))
            .unwrap();
        assert_eq!(scene.ordered.len(), 1);
        let pass = &scene.group_list[0].passes[0];
        assert!(pass.resolve.is_some());
        assert_eq!(pass.entity.actor, Some(actor.clone()));
        assert_eq!(pass.entity.opacity, 0.5);
        assert_eq!(pass.entity.entity.actor_slot, Some(actor.slot()));
        assert!(matches!(pass.entity.entity.flags, EntityFlags::Q3 { .. }));
    }

    #[test]
    fn prepare_native_held_weapon_skips_and_errors() {
        let owner = IdentityOwner::create("scene-native-skip").unwrap();
        let actor = owner.actor(1, 1);
        let snap = snapshot(SourceTime::Seconds(1.0));
        let native = || {
            let mut native = presentation(actor.clone());
            native.native_held_weapon = true;
            native
        };
        let equipped = || {
            let mut equipped = presentation(actor.clone());
            equipped.view_weapon = true;
            equipped.path = "guns/shotgun.md3".to_string();
            equipped.held_weapon = true;
            equipped
        };
        let mut held = StubHeld::new();
        held.declaration = HeldWeaponDeclaration::None;
        held.passes = vec![held_pass(actor.clone())];
        let (mut scene, _) = test_scene_with(StubAssets::new(test_material()), held);
        scene
            .prepare(&ScenePrepareInput::new(None, &snap, &[native(), equipped()], &[]))
            .unwrap();
        assert!(scene.ordered.is_empty());

        let mut held = StubHeld::new();
        held.declaration = HeldWeaponDeclaration::Present;
        let (mut scene, _) = test_scene_with(StubAssets::new(test_material()), held);
        assert!(matches!(
            scene.prepare(&ScenePrepareInput::new(None, &snap, &[native(), equipped()], &[])),
            Err(SceneError::HeldModel(_))
        ));
    }

    #[test]
    fn prepare_character_registers_presenter() {
        let owner = IdentityOwner::create("scene-character").unwrap();
        let actor = owner.actor(1, 1);
        let snap = snapshot(SourceTime::Seconds(1.0));
        let mut text = String::from("sex f\nfootsteps boot\nheadoffset 1 2 3\nfixedlegs\nfixedtorso\n");
        for frame in 0..31 {
            text.push_str(&format!("{frame} 6 0 10\n"));
        }
        let part = |name: &str| Q3CharacterPart {
            resource: dummy_reference(&format!("models/{name}.md3")),
            model: empty_md3(name),
            skin_resource: dummy_reference(&format!("models/{name}.skin")),
            surfaces: vec![SkinSurface {
                name: name.to_string(),
                shader: format!("models/{name}"),
            }],
        };
        let assets = Q3CharacterAssets {
            selection: Q3CharacterSelection {
                model: "sarge".to_string(),
                skin: "default".to_string(),
                head_model: String::new(),
                head_skin: "default".to_string(),
                team: None,
                team_name: String::new(),
            },
            lower: part("lower"),
            upper: part("upper"),
            head: part("head"),
            animation_resource: dummy_reference("models/animation.cfg"),
            animation: parse_player_animation_config(&text, "<test>").unwrap(),
            icon: None,
        };
        let character = Q3CharacterView {
            actor: actor.clone(),
            origin: vec3(10.0, 20.0, 30.0),
            angles: vec3(0.0, 45.0, 0.0),
            velocity: vec3(0.0, 0.0, 0.0),
            movement_direction: 0.0,
            animation: q3_spawn_animation(),
            source_flags: 0x80,
            powerups: 0,
            team: None,
            color: vec4(1.0, 1.0, 1.0, 1.0),
            scale: None,
            opacity: None,
        };
        let factory = StubFactory::default();
        let mut scene = ApplicationWorldScene::new(
            StubAssets::new(test_material()),
            factory,
            StubWorld::default(),
            StubHeld::new(),
            StubBodies::new(),
            Some(assets),
            || false,
        );
        scene
            .prepare(&ScenePrepareInput::new(None, &snap, &[], &[character]))
            .unwrap();
        assert_eq!(scene.characters.len(), 1);
        assert!(scene.ordered.is_empty());
    }

    #[test]
    fn view_merges_operations_around_models() {
        let owner = IdentityOwner::create("scene-view").unwrap();
        let snap = snapshot(SourceTime::Seconds(1.0));
        let (mut scene, counters) = test_scene();
        scene
            .prepare(&ScenePrepareInput::new(
                None,
                &snap,
                &[presentation(owner.actor(1, 1))],
                &[],
            ))
            .unwrap();
        let polygon = SceneOperation::Group(SceneGroup {
            order: SceneGroupOrder::Source {
                material: test_material(),
                source: qa_client::render::scene::submissions::SourceSurfaceOrder {
                    view: create_source_scene_order(Vec::new()),
                    entity: SourceEntityOrder::World,
                    surface: 0,
                    fog: 0,
                    dlight: 0,
                },
            },
            operations: Vec::new(),
        });
        let normal = SceneOperation::Group(SceneGroup {
            order: SceneGroupOrder::Sequence {
                phase: SequencePhase::Opaque,
            },
            operations: Vec::new(),
        });
        let input = WorldViewInput::new(
            camera(),
            ViewTarget::Preview("test".to_string()),
            RenderSourceTime::Seconds(0.0),
        );
        scene.view(input, &[normal, polygon], &[], false, None).unwrap();
        assert_eq!(counters.borrow().prepares, 1);
        assert_eq!(scene.world.views.len(), 1);
        assert_eq!(scene.world.world_ops, 1);
        let operations = &scene.world.views[0].operations;
        assert_eq!(operations.len(), 3);
        assert!(is_world_polygon(&operations[0]));
        assert!(!is_world_polygon(&operations[1]));
        assert!(scene.world.views[0].source.is_some());
    }

    #[test]
    fn view_shadow_path_prepares_casters() {
        let owner = IdentityOwner::create("scene-shadow").unwrap();
        let snap = snapshot(SourceTime::Seconds(1.0));
        let (mut scene, counters) = test_scene();
        scene
            .prepare(&ScenePrepareInput::new(
                None,
                &snap,
                &[presentation(owner.actor(1, 1))],
                &[],
            ))
            .unwrap();
        let light = SceneLight {
            origin: vec3(0.0, 0.0, 100.0),
            color: vec3(1.0, 1.0, 1.0),
            radius: 300.0,
            additive: false,
            profile: LightProfile::Simple,
        };
        let input = WorldViewInput::new(
            camera(),
            ViewTarget::Preview("test".to_string()),
            RenderSourceTime::Seconds(0.0),
        );
        scene.view(input, &[], &[light], false, None).unwrap();
        assert_eq!(counters.borrow().casters, 1);
        assert_eq!(scene.world.shadow_calls, 1);
        assert!(scene.world.views[0].q2_fragment_lighting.is_some());
    }

    #[test]
    fn supplemental_applies_inline_overrides() {
        let owner = IdentityOwner::create("scene-supplemental").unwrap();
        let snap = snapshot(SourceTime::Seconds(1.0));
        let mut assets = StubAssets::new(test_material());
        assets.brush = Some(SceneBrushRef {
            model: 2,
            scene: None,
            q1_bsp: false,
        });
        let (mut scene, _) = test_scene_with(assets, StubHeld::new());
        scene
            .prepare(&ScenePrepareInput::new(
                None,
                &snap,
                &[presentation(owner.actor(1, 1))],
                &[],
            ))
            .unwrap();
        let input = WorldViewInput::new(
            camera(),
            ViewTarget::Preview("test".to_string()),
            RenderSourceTime::Seconds(0.0),
        );
        scene.supplemental(&input, None, false).unwrap();
        assert_eq!(scene.world.models, vec![(2, Some(0.0), Some(false))]);
    }

    #[test]
    fn close_releases_groups_and_beams() {
        let owner = IdentityOwner::create("scene-close").unwrap();
        let snap = snapshot(SourceTime::Seconds(1.0));
        let (mut scene, _) = test_scene();
        scene
            .prepare(&ScenePrepareInput::new(
                None,
                &snap,
                &[presentation(owner.actor(1, 1))],
                &[],
            ))
            .unwrap();
        assert!(!scene.ordered.is_empty());
        scene.close();
        assert!(scene.ordered.is_empty());
        assert!(scene.groups.is_empty());
        assert!(scene.group_list.is_empty());
        assert!(scene.shader_beams.is_empty());
    }
}
