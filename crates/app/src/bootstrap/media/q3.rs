//! Source cgame effect producers joined to shared assets, collision, and drawing.
//!
//! Sync port of Quake-Anthology-TS `src/app/bootstrap/effects/q3.ts`. The donor awaits
//! asset-provider promises; this port resolves the same registrations through
//! the synchronous [`Q3EffectHost`]. Engine services the donor imports
//! (assets, world, collision) arrive as host methods; content systems
//! ([`ClientEffects`], [`LocalEntitySystem`], [`ImpactMarkSystem`],
//! [`ClientWeaponMediaRegistry`], [`ParticleSystem`]) are the real shared
//! implementations.
//!
//! Two donor behaviors are adapted to the Rust pipeline, both without
//! changing what draws: material batches convert to draw operations
//! host-side (the batch converter lives in the app world), and effect-model
//! groups submit under sequence phases because [`ModelDrawGroup`] carries no
//! material for source sorting. Effect PCM bytes stay app-side: like the
//! donor's [`SourceEffectSound`] list, this module only carries sound paths.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use qa_client::materials::evaluate::Q1FogInput;
use qa_client::materials::geometry::MaterialGeometry;
use qa_client::materials::q3_lighting::DynamicLight as ClientDynamicLight;
use qa_client::render::error::RenderError;
use qa_client::render::q3_hardware::Q3Hardware;
use qa_client::render::scene::material_registrations::RegisteredSceneMaterial;
use qa_client::render::scene::models::renderer::{ModelMaterialProvider, ModelViewInput, SceneModelRenderer};
use qa_client::render::scene::models::types::{
    color_bytes, EntityFlags, EntityTransform, ModelDrawGroup, ModelGroupOrder, ModelResource, ModelSourceOptions,
    SceneEntity, SceneModel as RenderSceneModel, ScenePose,
};
use qa_client::render::scene::particles::primitives::{
    beam_batch, default_model_batch, poly_geometry, rail_geometry, sprite_geometry, BeamPose, PolyVertex, RailKind,
    RailPose, SpritePose, DEFAULT_RAIL_SETTINGS,
};
use qa_client::render::scene::particles::q3_system::{
    load_particle_animations, ParticleAnimationSet, ParticleExplosionRequest, ParticleHardware, ParticleHost,
    ParticleMedia, ParticleSystem,
};
use qa_client::render::scene::particles::q3_types::{
    ParticleClientState, ParticleResources, ParticleShader, ParticleTrace, ParticleTracer, RefPoly as ParticleRefPoly,
    TraceSolidity as ParticleSolidity,
};
use qa_client::render::scene::submissions::{
    compiled_draw_group, reserve_source_entity_range, sequence_draw_group, source_draw_group, SceneOperation,
    SequencePhase, SourceEntityOrder, SourceSceneOrder, SourceSurfaceOrder,
};
use qa_client::render::scene::view::ViewProjector;
use qa_client::render::types::{AlphaTest, BlendFactor, CullFace, DepthTest, DrawBatch, RenderState, RendererImage};
use qa_client::view::{CameraClip, SceneCamera};
use qa_client::ClientError;
use qa_content::catalog::{CatalogError, InstalledCatalog};
use qa_content::contract::ContentId;
use qa_content::q3::base::game::hitscan::{Q3ContactEvent, RailImpact};
use qa_content::q3::base::game::numeric::GameRandom;
use qa_content::q3::base::shared::definitions::{Product, Weapon};
use qa_content::q3::base::shared::direction_byte::{byte_to_direction, direction_to_byte};
use qa_content::q3::base::shared::trajectory::{evaluate_trajectory, evaluate_trajectory_delta, TrajectoryType};
use qa_content::q3::base::world::{TraceContact, TraceSolidity};
use qa_content::q3::presentation::audio::{PresentSound, SoundOptions, SoundOrigin};
use qa_content::q3::presentation::collision_host::TraceResult;
use qa_content::q3::presentation::effects::{
    ClientEffects, EffectFrame, EffectImports, EffectMedia, EffectMediaVariant, EffectOptions, MissionEffectMedia,
    SmokePuffOptions,
};
use qa_content::q3::presentation::entities::MissileTrail;
use qa_content::q3::presentation::local_entities::{
    LocalEntity, LocalEntityFrame, LocalEntityHost, LocalEntityHostMedia, LocalEntityMedia, LocalEntityPool,
    LocalEntitySceneSink, LocalEntitySystem, LocalEntityType, MissionLocalEntityMedia, ParticleExplosion, PresentAudio,
    PresentCollision, PresentMarks, PresentPrediction, PresentRandom,
};
use qa_content::q3::presentation::mark_projector::{world_mark_projector, PresentMarkWorld};
use qa_content::q3::presentation::marks::{ImpactMarkOptions, ImpactMarkRequest, ImpactMarkSystem};
use qa_content::q3::presentation::movement_host::MovementTrace;
use qa_content::q3::presentation::ref_entity::{
    copy_ref_entity, create_lightning_entity, create_model_entity, create_sprite_entity, default_model, PresentError,
    Q3AdmittedRefEntity, RefEntity, RefPoly as ContentRefPoly, RefPolyVertex as ContentRefPolyVertex,
    SceneModel as ContentSceneModel, SceneShader, SceneSkin, ShadedFields, RF_FIRST_PERSON, RF_NOSHADOW,
    RF_THIRD_PERSON,
};
use qa_content::q3::presentation::retail_snapshot::DynamicLight as RetailDynamicLight;
use qa_content::q3::presentation::scene::{
    admit_q3_poly, q3_procedural_fog, snapshot_q3_scene_admission, Q3FogSelection, Q3SceneAdmission,
    SceneAdmissionOrigin,
};
use qa_content::q3::presentation::weapons::{
    emit_plasma_trail, emit_rail_trail, emit_shotgun_presentation, emit_weapon_impact, ClientInfoView,
    ClientWeaponHost, ClientWeaponMediaRegistry, ImpactSound, PresentParticleAnimations, PresentRendererResources,
    PresentWorldScene, Q3ShotgunEvent as PresentationShotgunEvent, RegisteredWeaponEffects, ShotgunPresentationHost,
    ShotgunTrace, WeaponPresentationMedia, WeaponPresentationModels, WeaponPresentationSettings,
    WeaponPresentationShaders, WeaponPresentationSounds, WeaponRegistrationAudio, WeaponSelectionDrawing,
    CONTENTS_WATER, MASK_SHOT, SURF_METALSTEPS, SURF_NOIMPACT,
};
use qa_core::identity::ActorId;
use qa_core::math::{
    cross3, length3, normalize3_or_zero, perpendicular_vector, rotate_point_around_vector, sub3, vec2, vec3, vec4,
    Bounds, Plane, Vec2, Vec3, Vec4,
};
use qa_core::rng::Qrand;
use qa_world::movement::q3::constants::entity_event;
use thiserror::Error;

use crate::bootstrap::simulation::q3_ballistics::{Q3BallisticEventKind, Q3ImpactKind, Q3SharedBallisticEvent};

/// Q3 lava contents bit (donor raw `8`).
const CONTENTS_LAVA: i32 = 8;
/// Q3 slime contents bit (donor raw `16`).
const CONTENTS_SLIME: i32 = 16;
/// World entity number for traces (donor `1022`/`1023`).
const ENTITYNUM_WORLD: i32 = 1022;

/// Sound playback routing for one source effect sound.
#[derive(Debug, Clone, PartialEq)]
pub enum SourceEffectPlayback {
    /// One-shot at the origin.
    Once,
    /// One-shot following an actor.
    Actor {
        /// Owning actor.
        actor: ActorId,
    },
    /// Loop following an actor.
    Loop {
        /// Owning actor.
        actor: ActorId,
        /// Loop velocity.
        velocity: Vec3,
    },
}

/// Effect sound with its source path (donor `SourceEffectSound`).
#[derive(Debug, Clone, PartialEq)]
pub struct SourceEffectSound {
    /// Content identity.
    pub content: ContentId,
    /// Source path.
    pub path: String,
    /// Sound origin.
    pub origin: Vec3,
    /// Channel.
    pub channel: i32,
    /// Volume.
    pub volume: f32,
    /// Emission time in seconds.
    pub seconds: f32,
    /// Playback routing.
    pub playback: SourceEffectPlayback,
}

/// Character presentation event (donor `Q3CharacterPresentationEvent`): the
/// shared character event with a live actor handle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Q3CharacterPresentationEvent {
    /// Acting actor.
    pub actor: ActorId,
    /// Event sequence.
    pub sequence: i32,
    /// Event time in milliseconds.
    pub time_milliseconds: i32,
    /// Entity event with channel bits.
    pub event: i32,
    /// Event parameter.
    pub parameter: i32,
}

/// Eager media loading (donor `preload`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Q3EffectPreload {
    /// Character media with cinematic deferral.
    Character,
    /// Character plus weapon media.
    Weapons,
}

/// Absorbed shared-scene trace result: the Q3 trace fields the effect
/// systems consume.
#[derive(Debug, Clone, PartialEq)]
pub struct Q3EffectTrace {
    /// Completed fraction.
    pub fraction: f32,
    /// Trace end point.
    pub end: Vec3,
    /// Entirely inside solid.
    pub all_solid: bool,
    /// Started inside solid.
    pub start_solid: bool,
    /// Contact plane, when hit.
    pub contact: Option<Plane>,
    /// Contents at the end point.
    pub contents: i32,
    /// Surface flags.
    pub surface_flags: i32,
    /// Hit actor, when any.
    pub hit_actor: Option<ActorId>,
}

/// Owned material inputs for one effect draw; the host turns these plus the
/// geometry into draw batches.
#[derive(Debug, Clone, PartialEq)]
pub struct Q3EffectMaterialContext {
    /// View camera.
    pub camera: SceneCamera,
    /// Effect time in milliseconds.
    pub time_milliseconds: i32,
    /// Entity RGBA bytes.
    pub entity_rgba: [u8; 4],
    /// Shader texture-coordinate override.
    pub shader_tex_coord: Vec2,
    /// Shader clock offset in seconds.
    pub time_offset: f32,
    /// Procedural fog selection.
    pub fog: Option<Q3FogSelection>,
    /// Q1 fog override.
    pub q1_fog: Option<Q1FogInput>,
}

/// Absorbed application services: asset registration, collision queries, and
/// world drawing (donor [`ApplicationAssets`](super::super::assets::ApplicationAssets)
/// plus `SceneQueries`; the [`SceneQueries`](qa_bots::scene::SceneQueries)
/// port stays absorbed in this host).
/// All methods take `&self`; hosts keep registration caches behind
/// interior mutability like [`SettingCvars`](qa_client::ui::settings::SettingCvars).
pub trait Q3EffectHost {
    /// Register a sound path, or `None` when the source is missing (donor
    /// `SoundBank.register` returning `null`, which skips the sound).
    fn register_sound(&self, path: &str) -> Option<PresentSound>;
    /// Register a shader material, or `None` when the source is missing.
    fn register_material(&self, name: &str) -> Option<RegisteredSceneMaterial>;
    /// Whether a shader is a deferred cinematic.
    fn has_cinematic(&self, name: &str) -> bool;
    /// Whether a content mount holds a path.
    fn mount_exists(&self, content: &ContentId, path: &str) -> bool;
    /// Load a content model, or `None` when the source is missing.
    fn load_content_model(&self, content: &ContentId, path: &str) -> Option<ContentSceneModel>;
    /// Load a render model pair for an admitted path, or `None` when the
    /// source is missing.
    fn load_render_model(&self, content: &ContentId, path: &str) -> Option<(RenderSceneModel, ModelResource)>;
    /// Whether an actor is a player.
    fn is_player(&self, actor: &ActorId) -> bool;
    /// Q3-policy trace with optional box bounds and actor passing.
    fn trace(
        &self,
        start: Vec3,
        end: Vec3,
        bounds: Option<Bounds>,
        pass_actor: Option<&ActorId>,
        contents_mask: i32,
    ) -> Q3EffectTrace;
    /// Q3 point contents.
    fn point_contents(&self, point: Vec3) -> i32;
    /// World bounds maximum for the camera-far view origin.
    fn world_bounds_max(&self) -> Vec3;
    /// World fog selections.
    fn fog_selections(&self) -> Vec<Q3FogSelection>;
    /// Mark projection world.
    fn mark_world(&self) -> &PresentMarkWorld;
    /// White texture image.
    fn white_image(&self) -> &RendererImage;
    /// Default source material.
    fn default_material(&self) -> &RegisteredSceneMaterial;
    /// Prepare draw batches for effect geometry (donor
    /// `prepareMaterialBatches` plus the world batch conversion).
    fn prepare_material(
        &self,
        material: &RegisteredSceneMaterial,
        geometry: &MaterialGeometry,
        context: &Q3EffectMaterialContext,
    ) -> Result<Vec<DrawBatch>, Q3EffectError>;
}

/// Object-safe model rendering used by effect preparation (donor
/// `SceneModelRenderer`).
pub trait Q3ModelRenderer {
    /// Warm the renderer cache for entities.
    fn preload(
        &mut self,
        entities: &[SceneEntity],
        options: &dyn Fn(&SceneEntity) -> ModelSourceOptions,
    ) -> Result<(), RenderError>;
    /// Prepare draw groups for entities.
    fn prepare(
        &self,
        entities: &[SceneEntity],
        input: &ModelViewInput,
        options: &dyn Fn(&SceneEntity) -> ModelSourceOptions,
        skinning: Option<&qa_client::render::scene::models::types::ModelSkinningFrame>,
    ) -> Result<Vec<ModelDrawGroup>, RenderError>;
}

impl<P: ModelMaterialProvider> Q3ModelRenderer for SceneModelRenderer<P> {
    fn preload(
        &mut self,
        entities: &[SceneEntity],
        options: &dyn Fn(&SceneEntity) -> ModelSourceOptions,
    ) -> Result<(), RenderError> {
        SceneModelRenderer::preload(self, entities, options)
    }

    fn prepare(
        &self,
        entities: &[SceneEntity],
        input: &ModelViewInput,
        options: &dyn Fn(&SceneEntity) -> ModelSourceOptions,
        skinning: Option<&qa_client::render::scene::models::types::ModelSkinningFrame>,
    ) -> Result<Vec<ModelDrawGroup>, RenderError> {
        SceneModelRenderer::prepare(self, entities, input, options, skinning)
    }
}

/// Q3 application effects error.
#[derive(Debug, Error)]
pub enum Q3EffectError {
    /// Effect content is unknown to the catalog.
    #[error(transparent)]
    Catalog(#[from] CatalogError),
    /// Presentation failure.
    #[error(transparent)]
    Present(#[from] PresentError),
    /// Render failure.
    #[error(transparent)]
    Render(#[from] RenderError),
    /// Client failure.
    #[error(transparent)]
    Client(#[from] ClientError),
    /// Effect shader source is missing.
    #[error("Effect shader is missing: {0}")]
    MissingShader(String),
    /// Effect model source is missing.
    #[error("Effect model is missing: {0}")]
    MissingModel(String),
    /// Admitted effect model has no render pair.
    #[error("Admitted Q3 effect model lacks its render pair: {0}")]
    MissingRenderModel(String),
    /// Effect cinematic deferred until use.
    #[error("Effect cinematic deferred until use: {0}")]
    CinematicDeferred(String),
    /// Mark shader was never registered.
    #[error("Unregistered mark shader {0}")]
    UnregisteredMarkShader(String),
    /// Effect shader was never registered.
    #[error("Unregistered effect shader {0}")]
    UnregisteredEffectShader(String),
    /// Admitted model lost its prepared descriptor.
    #[error("Admitted Q3 effect model lost its prepared descriptor")]
    MissingEntityDescriptor,
    /// Ballistic weapon index names no weapon.
    #[error("Invalid Q3 effect weapon {0}")]
    InvalidWeapon(i32),
    /// Missionpack reflection needs its source presentation binding.
    #[error("Selected missionpack reflection requires its source presentation binding")]
    ReflectionBinding,
    /// World effect sound requires a fixed origin.
    #[error("World effect sound requires a fixed origin")]
    UnpositionedSound,
    /// Particle animation is unknown.
    #[error("Unknown Q3 particle animation {0}")]
    UnknownAnimation(String),
}

/// Captured reference entity with its admission index.
#[derive(Debug, Clone, PartialEq)]
struct CapturedRef {
    entity_index: usize,
    ref_entity: RefEntity,
    cull_radius: f32,
    hidden_for: Option<ActorId>,
}

/// Prepared renderer entity with its source options.
#[derive(Debug, Clone, PartialEq)]
struct PreparedEffectModel {
    entity_index: usize,
    entity: SceneEntity,
    custom_shader: Option<String>,
}

/// Ballistic event with its frame time.
#[derive(Debug, Clone, PartialEq)]
struct TimedBallistic {
    event: Q3SharedBallisticEvent,
    time: i32,
}

/// Last fire by weapon and time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct LastFire {
    weapon: i32,
    time: i32,
}

/// Push a source effect sound unless its registration is missing.
#[allow(clippy::too_many_arguments)]
fn push_sound(
    sounds: &RefCell<Vec<SourceEffectSound>>,
    content: &ContentId,
    time: &Cell<i32>,
    sound: Option<&PresentSound>,
    origin: Vec3,
    channel: i32,
    volume: f32,
    actor: Option<&ActorId>,
) {
    let Some(sound) = sound else { return };
    sounds.borrow_mut().push(SourceEffectSound {
        content: content.clone(),
        path: sound.path.clone(),
        origin,
        channel,
        volume,
        seconds: time.get() as f32 / 1000.0,
        playback: match actor {
            Some(actor) => SourceEffectPlayback::Actor { actor: actor.clone() },
            None => SourceEffectPlayback::Once,
        },
    });
}

/// Register an effect shader material, deferring cinematics while preloading.
fn register_effect_shader<H: Q3EffectHost>(
    host: &Rc<H>,
    shaders: &Rc<RefCell<HashMap<String, RegisteredSceneMaterial>>>,
    preloading: &Rc<Cell<bool>>,
    name: &str,
) -> Result<SceneShader, Q3EffectError> {
    if preloading.get() && host.has_cinematic(name) {
        return Err(Q3EffectError::CinematicDeferred(name.to_string()));
    }
    let material = host
        .register_material(name)
        .ok_or_else(|| Q3EffectError::MissingShader(name.to_string()))?;
    shaders.borrow_mut().insert(name.to_string(), material);
    Ok(SceneShader::new(name))
}

/// Map an absorbed trace to a movement trace.
fn movement_trace(trace: &Q3EffectTrace) -> MovementTrace {
    MovementTrace {
        base: TraceResult {
            fraction: trace.fraction,
            end: trace.end,
            solidity: if trace.all_solid {
                TraceSolidity::AllSolid
            } else if trace.start_solid {
                TraceSolidity::StartSolid
            } else {
                TraceSolidity::Clear
            },
            contact: match trace.contact {
                Some(plane) => TraceContact::Plane { plane },
                None => TraceContact::None,
            },
            contents: trace.contents,
            surface_flags: trace.surface_flags,
        },
        entity_num: if trace.fraction < 1.0 {
            ENTITYNUM_WORLD
        } else {
            ENTITYNUM_WORLD + 1
        },
    }
}

/// Effect sound imports over the shared sound list (donor `ClientEffects` imports).
struct EffectSoundImports {
    sounds: Rc<RefCell<Vec<SourceEffectSound>>>,
    content: ContentId,
    time: Rc<Cell<i32>>,
    random: Rc<RefCell<GameRandom>>,
}

impl EffectImports for EffectSoundImports {
    fn random_integer(&mut self) -> i32 {
        self.random.borrow_mut().rand()
    }

    fn start_sound(&mut self, origin: Vec3, _entity: i32, channel: i32, sound: Option<PresentSound>) {
        push_sound(
            &self.sounds,
            &self.content,
            &self.time,
            sound.as_ref(),
            origin,
            channel,
            1.0,
            None,
        );
    }
}

/// Prediction adapter over the host trace (donor `LocalEntitySystem` prediction).
struct EffectPrediction<H> {
    host: Rc<H>,
}

impl<H: Q3EffectHost> PresentPrediction for EffectPrediction<H> {
    fn trace_mover(&self, start: Vec3, end: Vec3, bounds: Bounds, _skip_number: i32, mask: i32) -> MovementTrace {
        movement_trace(&self.host.trace(start, end, Some(bounds), None, mask))
    }

    fn point_contents_pred(&self, point: Vec3, _pass_entity: i32) -> i32 {
        self.host.point_contents(point)
    }
}

/// Collision adapter over the host trace (donor point contents plus a point
/// trace for the raw shape query, which has no donor counterpart).
struct EffectCollision<H> {
    host: Rc<H>,
}

impl<H: Q3EffectHost> PresentCollision for EffectCollision<H> {
    fn collision_trace(&self, start: Vec3, end: Vec3, mask: i32) -> TraceResult {
        movement_trace(&self.host.trace(start, end, None, None, mask)).base
    }

    fn collision_contents(&self, point: Vec3) -> i32 {
        self.host.point_contents(point)
    }
}

/// Positioned-sound adapter over the shared sound list.
struct EffectAudio {
    sounds: Rc<RefCell<Vec<SourceEffectSound>>>,
    content: ContentId,
    time: Rc<Cell<i32>>,
    errors: Rc<RefCell<Vec<Q3EffectError>>>,
}

impl PresentAudio for EffectAudio {
    fn start_sound(&mut self, sound: Option<PresentSound>, options: &SoundOptions) {
        let SoundOrigin::Fixed { position } = options.origin else {
            self.errors.borrow_mut().push(Q3EffectError::UnpositionedSound);
            return;
        };
        push_sound(
            &self.sounds,
            &self.content,
            &self.time,
            sound.as_ref(),
            position,
            options.channel,
            options.volume as f32 / 127.0,
            None,
        );
    }
}

/// Random adapter over the shared game random.
struct EffectRandom {
    random: Rc<RefCell<GameRandom>>,
}

impl PresentRandom for EffectRandom {
    fn rand(&mut self) -> i32 {
        self.random.borrow_mut().rand()
    }

    fn random(&mut self) -> f32 {
        self.random.borrow_mut().random()
    }

    fn crandom(&mut self) -> f32 {
        self.random.borrow_mut().crandom()
    }
}

/// Mark adapter over the shared mark system.
struct EffectMarks {
    marks: Rc<RefCell<ImpactMarkSystem>>,
    errors: Rc<RefCell<Vec<Q3EffectError>>>,
}

impl PresentMarks for EffectMarks {
    fn impact_mark(&mut self, request: &ImpactMarkRequest) -> Vec<ContentRefPoly> {
        match self.marks.borrow_mut().impact_mark(request) {
            Ok(polys) => polys,
            Err(error) => {
                self.errors.borrow_mut().push(Q3EffectError::Present(error));
                Vec::new()
            }
        }
    }
}

/// Mark options over the shared clock (donor `{ clock, enabled, energyShader }`).
struct EffectMarkOptions {
    time: Rc<Cell<i32>>,
}

impl ImpactMarkOptions for EffectMarkOptions {
    fn clock(&self) -> i32 {
        self.time.get()
    }

    fn enabled(&self) -> bool {
        true
    }

    fn energy_shader(&self) -> Option<SceneShader> {
        None
    }
}

/// Weapon registration resources over the host (donor registry resources).
struct RegistryResources<H> {
    host: Rc<H>,
    content: ContentId,
    shaders: Rc<RefCell<HashMap<String, RegisteredSceneMaterial>>>,
    preloading: Rc<Cell<bool>>,
    errors: Rc<RefCell<Vec<Q3EffectError>>>,
}

impl<H: Q3EffectHost> PresentRendererResources for RegistryResources<H> {
    fn register_model(&mut self, path: &str) -> ContentSceneModel {
        if !self.host.mount_exists(&self.content, path) {
            return default_model();
        }
        self.host
            .load_content_model(&self.content, path)
            .unwrap_or_else(default_model)
    }

    fn register_skin(&mut self, _path: &str) -> Option<SceneSkin> {
        // Weapon registration never binds skins in this path (donor
        // resources expose models and shaders only).
        None
    }

    fn register_shader(&mut self, name: &str) -> Option<SceneShader> {
        match register_effect_shader(&self.host, &self.shaders, &self.preloading, name) {
            Ok(shader) => Some(shader),
            Err(error) => {
                self.errors.borrow_mut().push(error);
                None
            }
        }
    }

    fn register_shader_no_mip(&mut self, name: &str) -> Option<SceneShader> {
        self.register_shader(name)
    }

    fn load_world(&mut self, _mapname: &str) -> PresentWorldScene {
        // Weapon registration never loads worlds in this path.
        PresentWorldScene { model_count: 0 }
    }

    fn load_particle_animations(&mut self) -> PresentParticleAnimations {
        // Weapon registration never loads particle animations in this path.
        PresentParticleAnimations { names: Vec::new() }
    }
}

/// Weapon registration audio over the host.
struct RegistryAudio<H> {
    host: Rc<H>,
}

impl<H: Q3EffectHost> WeaponRegistrationAudio for RegistryAudio<H> {
    fn register_sound(&mut self, path: &str) -> Option<PresentSound> {
        self.host.register_sound(path)
    }
}

/// Particle prediction tracer over the host trace.
struct ParticleTracerAdapter<H> {
    host: Rc<H>,
}

impl<H: Q3EffectHost> ParticleTracer for ParticleTracerAdapter<H> {
    fn trace(&self, start: Vec3, end: Vec3, bounds: Bounds, _pass_entity: i32, contents: i32) -> ParticleTrace {
        let trace = self.host.trace(start, end, Some(bounds), None, contents);
        ParticleTrace {
            end: trace.end,
            entity_num: if trace.fraction < 1.0 {
                ENTITYNUM_WORLD
            } else {
                ENTITYNUM_WORLD + 1
            },
            solidity: if trace.all_solid {
                ParticleSolidity::AllSolid
            } else if trace.start_solid {
                ParticleSolidity::StartSolid
            } else {
                ParticleSolidity::Clear
            },
            fraction: trace.fraction,
        }
    }
}

/// Particle shader resources over the host (donor `loadParticleAnimations`
/// registers through the same shader helper).
struct ParticleResourcesAdapter<H> {
    host: Rc<H>,
    shaders: Rc<RefCell<HashMap<String, RegisteredSceneMaterial>>>,
    preloading: Rc<Cell<bool>>,
    errors: Rc<RefCell<Vec<Q3EffectError>>>,
}

impl<H: Q3EffectHost> ParticleResources for ParticleResourcesAdapter<H> {
    fn register_shader(&mut self, name: &str) -> Option<ParticleShader> {
        match register_effect_shader(&self.host, &self.shaders, &self.preloading, name) {
            Ok(_) => Some(ParticleShader::named(name)),
            Err(error) => {
                self.errors.borrow_mut().push(error);
                None
            }
        }
    }
}

/// Loaded weapon bundle (donor `WeaponEffects` without its closures): impact
/// media, particles, and loop/one-shot sounds. Stored as one option so a
/// failed load never leaves a partial bundle behind.
struct WeaponBundle {
    media: WeaponPresentationMedia,
    particles: ParticleSystem,
    plasma: SceneShader,
    smoke: SceneShader,
    nail_smoke: Option<SceneShader>,
    shotgun_smoke: SceneShader,
    quad: Option<PresentSound>,
    bounce: [Option<PresentSound>; 2],
}

/// Frame submission (donor `frame` return value).
#[derive(Debug, Clone, PartialEq)]
pub struct Q3EffectFrameOutput {
    /// Admitted entities plus mark and particle polygons.
    pub admission: Q3SceneAdmission,
    /// Draw operations in submission order.
    pub operations: Vec<SceneOperation>,
    /// Q3 dynamic lights for the frame.
    pub q3_lights: Vec<ClientDynamicLight>,
}

/// Liquid contents bits (donor `8 | 16 | 32`: lava, slime, water).
const CONTENTS_LIQUID: i32 = CONTENTS_LAVA | CONTENTS_SLIME | CONTENTS_WATER;

/// Opaque primitive state for beam and default-model batches (donor
/// `primitiveState`).
const PRIMITIVE_STATE: RenderState = RenderState {
    blend: (BlendFactor::One, BlendFactor::Zero),
    depth_test: DepthTest::LessEqual,
    depth_write: true,
    alpha_test: AlphaTest::None,
    cull: CullFace::Back,
    depth_range: [0.0, 1.0],
    polygon_offset: None,
};

/// Convert a presentation dynamic light to the client light.
fn retail_to_client_light(light: &RetailDynamicLight) -> ClientDynamicLight {
    ClientDynamicLight {
        origin: light.origin,
        radius: light.radius,
        color: light.color,
        additive: light.additive,
    }
}

/// Null weapon-selection drawing: effect presentation never draws selection
/// UI, so the bridge reports byte length and draws nothing.
#[derive(Debug, Default)]
struct NullWeaponDrawing;

impl WeaponSelectionDrawing for NullWeaponDrawing {
    fn fade_color(&self, _start: i32, _duration: i32) -> Option<Vec4> {
        None
    }

    fn set_color(&mut self, _color: Option<Vec4>) {}

    fn draw_pic(&mut self, _x: i32, _y: i32, _width: i32, _height: i32, _shader: Option<SceneShader>) {}

    fn draw_string_length(&self, text: &str) -> usize {
        text.len()
    }

    fn draw_big_string_color(&mut self, _x: i32, _y: i32, _text: &str, _color: Vec4) {}
}

/// Weapon host bridge over the effect state (donor `ImpactHost`): sounds join
/// the shared sound list, marks join the shared mark system, and particle
/// explosions join the weapon particle system. Added entities, lights, and
/// polys stage into the live scene lists; none of the bridged emit paths
/// (`emitWeaponImpact`, `emitRailTrail`, `emitPlasmaTrail`) add scene output
/// directly, so the staging stays empty in practice.
struct WeaponHostBridge<'a, H: Q3EffectHost> {
    host: &'a H,
    content: &'a ContentId,
    time: &'a Cell<i32>,
    random: &'a RefCell<GameRandom>,
    sounds: &'a RefCell<Vec<SourceEffectSound>>,
    errors: &'a RefCell<Vec<Q3EffectError>>,
    effects: &'a mut ClientEffects,
    marks: &'a RefCell<ImpactMarkSystem>,
    particles: &'a mut ParticleSystem,
    media: &'a WeaponPresentationMedia,
    registry_effects: &'a RegisteredWeaponEffects,
    product: Product,
    rage_pro: bool,
    refs: &'a mut Vec<CapturedRef>,
    lights: &'a mut Vec<ClientDynamicLight>,
    polys: &'a mut Vec<ContentRefPoly>,
    drawing: NullWeaponDrawing,
}

impl<H: Q3EffectHost> ClientWeaponHost for WeaponHostBridge<'_, H> {
    fn add_ref_entity(&mut self, entity: RefEntity) {
        let index = self.refs.len();
        self.refs.push(CapturedRef {
            entity_index: index,
            ref_entity: entity,
            cull_radius: 0.0,
            hidden_for: None,
        });
    }

    fn add_light(&mut self, light: RetailDynamicLight) {
        self.lights.push(retail_to_client_light(&light));
    }

    fn start_sound(&mut self, origin: Option<Vec3>, _entity: i32, channel: i32, sound: Option<PresentSound>) {
        let Some(origin) = origin else { return };
        push_sound(
            self.sounds,
            self.content,
            self.time,
            sound.as_ref(),
            origin,
            channel,
            1.0,
            None,
        );
    }

    fn add_loop_sound(
        &mut self,
        _entity: i32,
        origin: Vec3,
        _velocity: Vec3,
        sound: Option<PresentSound>,
        _real_loop: bool,
    ) {
        // Entity-numbered loop hooks carry no actor binding at bootstrap
        // scope, so they present positionally.
        push_sound(
            self.sounds,
            self.content,
            self.time,
            sound.as_ref(),
            origin,
            0,
            1.0,
            None,
        );
    }

    fn trace_mover(&self, start: Vec3, end: Vec3, bounds: Bounds, _skip_number: i32, mask: i32) -> MovementTrace {
        movement_trace(&self.host.trace(start, end, Some(bounds), None, mask))
    }

    fn point_contents_pred(&self, point: Vec3, _pass_entity: i32) -> i32 {
        self.host.point_contents(point)
    }

    fn collision_trace(&self, start: Vec3, end: Vec3, mask: i32) -> TraceResult {
        movement_trace(&self.host.trace(start, end, None, None, mask)).base
    }

    fn collision_contents(&self, point: Vec3) -> i32 {
        self.host.point_contents(point)
    }

    fn rand_i32(&mut self) -> i32 {
        self.random.borrow_mut().rand()
    }

    fn random_f32(&mut self) -> f32 {
        self.random.borrow_mut().random()
    }

    fn crandom_f32(&mut self) -> f32 {
        self.random.borrow_mut().crandom()
    }

    fn weapon_effects(&mut self) -> &mut ClientEffects {
        self.effects
    }

    fn impact_mark(&mut self, request: &ImpactMarkRequest) -> Vec<ContentRefPoly> {
        match self.marks.borrow_mut().impact_mark(request) {
            Ok(polys) => polys,
            Err(error) => {
                self.errors.borrow_mut().push(Q3EffectError::Present(error));
                Vec::new()
            }
        }
    }

    fn particle_explosion(&mut self, request: &ParticleExplosion) {
        let animation = match request.animation.as_str() {
            "explode1" => "explode1",
            other => {
                self.errors
                    .borrow_mut()
                    .push(Q3EffectError::UnknownAnimation(other.to_string()));
                return;
            }
        };
        if let Err(error) = self.particles.explosion(&ParticleExplosionRequest {
            animation,
            origin: request.origin,
            velocity: request.velocity,
            duration: request.duration,
            size_start: request.size_start as i32,
            size_end: request.size_end as i32,
        }) {
            self.errors.borrow_mut().push(Q3EffectError::Render(error));
        }
    }

    fn weapon_media(&self) -> &WeaponPresentationMedia {
        self.media
    }

    fn weapon_settings(&self) -> WeaponPresentationSettings {
        // The donor host supplies only `oldRocket`; the rail branch pins
        // `railTrailTime: 400` with `oldRail: false`, and the plasma branch
        // pins `noProjectileTrail: false` with `oldPlasma: false`. The bridge
        // reads nothing else, so the remaining rows stay neutral.
        WeaponPresentationSettings {
            brass_time: 0,
            rail_trail_time: 400,
            old_rail: false,
            no_projectile_trail: false,
            old_plasma: false,
            old_rocket: false,
            true_lightning: 0.0,
            draw_gun: false,
            fov: 0.0,
            gun_x: 0.0,
            gun_y: 0.0,
            gun_z: 0.0,
            gun_frame: 0,
            tracer_length: 0.0,
            tracer_width: 0.0,
            tracer_chance: 0.0,
            hardware_rage_pro: self.rage_pro,
        }
    }

    fn client_info_view(&self, _number: i32) -> ClientInfoView {
        ClientInfoView {
            color1: vec3(1.0, 1.0, 1.0),
            color2: vec3(1.0, 1.0, 1.0),
            animations: Vec::new(),
        }
    }

    fn load_sound(&mut self, path: &str) -> Option<PresentSound> {
        self.host.register_sound(path)
    }

    fn add_poly(&mut self, poly: ContentRefPoly) {
        self.polys.push(poly);
    }

    fn drawing(&mut self) -> &mut dyn WeaponSelectionDrawing {
        &mut self.drawing
    }
}

/// Shotgun presentation bridge (donor `media.shotgun`): point traces with the
/// shooter passed through, blood ownership tracking, and wall impacts through
/// the weapon host bridge.
struct ShotgunBridge<'a, H: Q3EffectHost> {
    weapon: WeaponHostBridge<'a, H>,
    pool: &'a mut LocalEntityPool,
    frame: EffectFrame,
    shooter: ActorId,
    blood_owners: &'a mut Vec<(RefEntity, ActorId)>,
    smoke_shader: Option<SceneShader>,
}

impl<H: Q3EffectHost> ShotgunPresentationHost<ActorId> for ShotgunBridge<'_, H> {
    fn smoke_enabled(&self) -> bool {
        true
    }

    fn trace(&self, start: Vec3, end: Vec3) -> ShotgunTrace<ActorId> {
        let trace = self.weapon.host.trace(start, end, None, Some(&self.shooter), MASK_SHOT);
        ShotgunTrace {
            end: trace.end,
            normal: trace.contact.map(|plane| plane.normal).unwrap_or(vec3(0.0, 0.0, 0.0)),
            surface_flags: trace.surface_flags,
            target: trace.hit_actor,
        }
    }

    fn water(&self, start: Vec3, end: Vec3) -> Vec3 {
        self.weapon.host.trace(start, end, None, None, CONTENTS_WATER).end
    }

    fn contents(&self, point: Vec3) -> i32 {
        self.weapon.host.point_contents(point)
    }

    fn is_player(&self, target: &ActorId) -> bool {
        self.weapon.host.is_player(target)
    }

    fn blood(&mut self, point: Vec3, _normal: Vec3, target: ActorId) {
        match self.weapon.effects.bleed_at(self.pool, &self.frame, point, false) {
            Ok(Some(sprite)) => self.blood_owners.push((RefEntity::Sprite(sprite), target)),
            Ok(None) => {}
            Err(error) => self.weapon.errors.borrow_mut().push(Q3EffectError::Present(error)),
        }
    }

    fn wall(&mut self, point: Vec3, normal: Vec3, sound: ImpactSound) {
        let product = self.weapon.product;
        let effects = self.weapon.registry_effects;
        let result = emit_weapon_impact(
            product,
            effects,
            &mut self.weapon,
            self.pool,
            &self.frame,
            Weapon::WpShotgun,
            0,
            point,
            normal,
            sound,
        );
        if let Err(error) = result {
            self.weapon.errors.borrow_mut().push(Q3EffectError::Present(error));
        }
    }

    fn bubbles(&mut self, start: Vec3, end: Vec3) {
        if let Err(error) = self
            .weapon
            .effects
            .bubble_trail(self.pool, &self.frame, start, end, 32.0)
        {
            self.weapon.errors.borrow_mut().push(Q3EffectError::Present(error));
        }
    }

    fn smoke(&mut self, origin: Vec3) {
        let time = self.weapon.time.get();
        match self.weapon.effects.smoke_puff(
            self.pool,
            &self.frame,
            &SmokePuffOptions {
                origin,
                velocity: vec3(0.0, 0.0, 8.0),
                radius: 32.0,
                color: vec4(1.0, 1.0, 1.0, 0.33),
                duration: 900,
                start_time: time,
                fade_in_time: 0,
                flags: 1,
                shader: self.smoke_shader.clone(),
            },
        ) {
            Ok(handle) => {
                if let Some(entity) = self.pool.get_mut(handle) {
                    entity.le_type = LocalEntityType::ScaleFade;
                }
            }
            Err(error) => self.weapon.errors.borrow_mut().push(Q3EffectError::Present(error)),
        }
    }
}

/// Local-entity scene sink (donor `prepare` callbacks): staged refs keep
/// their owning entity radius for camera-near culling and their blood owner
/// for first-person hiding.
struct EntitySceneSink<'a> {
    refs: &'a mut Vec<CapturedRef>,
    lights: &'a mut Vec<ClientDynamicLight>,
    active: &'a [LocalEntity],
    blood_owners: &'a [(RefEntity, ActorId)],
}

impl LocalEntitySceneSink for EntitySceneSink<'_> {
    fn add_ref_entity(&mut self, entity: &RefEntity) {
        let owner = self.active.iter().find(|local| local.ref_entity == *entity);
        let hidden = self
            .blood_owners
            .iter()
            .find(|(blood, _)| *blood == *entity)
            .map(|(_, owner)| owner.clone());
        self.refs.push(CapturedRef {
            entity_index: self.refs.len(),
            ref_entity: copy_ref_entity(entity),
            cull_radius: owner.map(|local| local.radius).unwrap_or(0.0),
            hidden_for: hidden,
        });
    }

    fn add_light(&mut self, light: &RetailDynamicLight) {
        self.lights.push(retail_to_client_light(light));
    }
}

/// Model source options for one effect model (donor per-entity options map).
fn effect_model_options(custom_shader: Option<String>, shader_tex_coord: Option<Vec2>) -> ModelSourceOptions {
    ModelSourceOptions {
        indexed_skin: None,
        model_beam: None,
        sync_base: 0.0,
        sprite_roll: 0.0,
        custom_shader,
        custom_skin: None,
        q3_lods: None,
        lod_scale: None,
        lod_bias: None,
        non_normalized_axes: false,
        no_world_model: false,
        shader_tex_coord,
        left_hand: 0,
        infrared: false,
        view_model: false,
        planar_shadow: false,
        player: false,
        overbright_models: None,
        player_colors: None,
    }
}

/// Submit one renderer model group (donor `renderer.prepare` output, already
/// operations there): sequence phases carry the opaque/translucent sort, and
/// compiled groups sort under the default source material because
/// [`ModelDrawGroup`] drops the compiled material.
fn model_group_operation(group: ModelDrawGroup, default_material: &RegisteredSceneMaterial) -> SceneOperation {
    match group.order {
        ModelGroupOrder::Opaque => SceneOperation::Group(sequence_draw_group(SequencePhase::Opaque, group.batches)),
        ModelGroupOrder::Translucent => {
            SceneOperation::Group(sequence_draw_group(SequencePhase::Translucent, group.batches))
        }
        ModelGroupOrder::Compiled => {
            SceneOperation::Group(compiled_draw_group(default_material.clone(), group.batches))
        }
    }
}

/// Convert a particle polygon to a content polygon for admission.
fn particle_poly_to_content(poly: &ParticleRefPoly) -> ContentRefPoly {
    ContentRefPoly {
        shader: poly.shader.as_ref().map(|shader| SceneShader::new(shader.name.clone())),
        vertices: poly
            .vertices
            .iter()
            .map(|vertex| ContentRefPolyVertex {
                position: vertex.position,
                tex_coord: vertex.tex_coord,
                color: vertex.color,
            })
            .collect(),
    }
}

/// Convert a content polygon to primitive vertices (donor `polyGeometry`).
fn content_poly_vertices(poly: &ContentRefPoly) -> Vec<PolyVertex> {
    poly.vertices
        .iter()
        .map(|vertex| PolyVertex {
            position: vertex.position,
            tex_coord: vertex.tex_coord,
            color: color_bytes(vertex.color),
        })
        .collect()
}

/// Push a looping source effect sound unless its registration is missing
/// (donor `media.loop`).
fn push_loop_sound(
    sounds: &RefCell<Vec<SourceEffectSound>>,
    content: &ContentId,
    time: &Cell<i32>,
    sound: Option<&PresentSound>,
    origin: Vec3,
    actor: &ActorId,
    velocity: Vec3,
) {
    let Some(sound) = sound else { return };
    sounds.borrow_mut().push(SourceEffectSound {
        content: content.clone(),
        path: sound.path.clone(),
        origin,
        channel: 0,
        volume: 1.0,
        seconds: time.get() as f32 / 1000.0,
        playback: SourceEffectPlayback::Loop {
            actor: actor.clone(),
            velocity,
        },
    });
}

/// Resolve a ref-entity shader to its registered material, defaulting when
/// the entity carries no custom shader (donor frame shader lookup).
fn effect_shader_material(
    shaders: &HashMap<String, RegisteredSceneMaterial>,
    custom: Option<&SceneShader>,
    default_material: &RegisteredSceneMaterial,
) -> Result<RegisteredSceneMaterial, Q3EffectError> {
    match custom {
        None => Ok(default_material.clone()),
        Some(shader) => shaders
            .get(&shader.name)
            .cloned()
            .ok_or_else(|| Q3EffectError::UnregisteredEffectShader(shader.name.clone())),
    }
}

/// Material context for one effect draw (donor `materialContext` plus the
/// ref-entity overrides).
#[allow(clippy::too_many_arguments)]
fn draw_material_context(
    camera: &SceneCamera,
    time_milliseconds: i32,
    entity_rgba: [u8; 4],
    shader_tex_coord: Vec2,
    time_offset: f32,
    fog: Option<Q3FogSelection>,
    q1_fog: Option<Q1FogInput>,
) -> Q3EffectMaterialContext {
    Q3EffectMaterialContext {
        camera: *camera,
        time_milliseconds,
        entity_rgba,
        shader_tex_coord,
        time_offset,
        fog,
        q1_fog,
    }
}

/// Render flags for any reference entity.
fn ref_render_flags(entity: &RefEntity) -> i32 {
    match entity {
        RefEntity::Model(entity) => entity.shading.render_flags,
        RefEntity::Sprite(entity) => entity.shading.render_flags,
        RefEntity::Beam(entity) => entity.shading.render_flags,
        RefEntity::RailCore(entity) => entity.shading.render_flags,
        RefEntity::RailRings(entity) => entity.shading.render_flags,
        RefEntity::Lightning(entity) => entity.shading.render_flags,
        RefEntity::Portal(entity) => entity.render_flags,
    }
}

/// Q3 application effects (donor `Q3ApplicationEffects`).
pub struct Q3ApplicationEffects<H: 'static> {
    content: ContentId,
    host: Rc<H>,
    product: Product,
    time: Rc<Cell<i32>>,
    random: Rc<RefCell<GameRandom>>,
    sounds: Rc<RefCell<Vec<SourceEffectSound>>>,
    adapter_errors: Rc<RefCell<Vec<Q3EffectError>>>,
    effects: ClientEffects,
    pool: LocalEntityPool,
    system: LocalEntitySystem,
    marks: Rc<RefCell<ImpactMarkSystem>>,
    registry: ClientWeaponMediaRegistry,
    weapons: Option<WeaponBundle>,
    pending_polys: Vec<ContentRefPoly>,
    rage_pro: bool,
    shaders: Rc<RefCell<HashMap<String, RegisteredSceneMaterial>>>,
    preloading: Rc<Cell<bool>>,
    renderer: Box<dyn Q3ModelRenderer>,
    render_models: HashMap<String, (RenderSceneModel, ModelResource)>,
    refs: Vec<CapturedRef>,
    admission: Q3SceneAdmission,
    lights: Vec<ClientDynamicLight>,
    models: Vec<PreparedEffectModel>,
    blood_owners: Vec<(RefEntity, ActorId)>,
    projectiles: HashMap<ActorId, TimedBallistic>,
    flashes: HashMap<ActorId, TimedBallistic>,
    last_fires: HashMap<ActorId, LastFire>,
    bolts: HashMap<ActorId, TimedBallistic>,
}

impl<H: Q3EffectHost> Q3ApplicationEffects<H> {
    /// Create application effects over content, catalog, host, and renderer.
    /// `preload` defers cinematics (`Character`) and additionally loads
    /// weapon media (`Weapons`); `read_hardware` classifies the GL driver.
    pub fn create(
        content: ContentId,
        catalog: &InstalledCatalog,
        host: H,
        renderer: Box<dyn Q3ModelRenderer>,
        preload: Option<Q3EffectPreload>,
        read_hardware: Option<&dyn Fn() -> Q3Hardware>,
    ) -> Result<Self, Q3EffectError> {
        let host = Rc::new(host);
        let product = if catalog.product(content.as_str())?.expectation.campaign == "missionpack" {
            Product::Missionpack
        } else {
            Product::Baseq3
        };
        let rage_pro = matches!(read_hardware.map(|read| read()), Some(Q3Hardware::RagePro));
        let time = Rc::new(Cell::new(0));
        let random = Rc::new(RefCell::new(GameRandom::default()));
        let sounds: Rc<RefCell<Vec<SourceEffectSound>>> = Rc::new(RefCell::new(Vec::new()));
        let adapter_errors: Rc<RefCell<Vec<Q3EffectError>>> = Rc::new(RefCell::new(Vec::new()));
        let shaders: Rc<RefCell<HashMap<String, RegisteredSceneMaterial>>> = Rc::new(RefCell::new(HashMap::new()));
        let preloading = Rc::new(Cell::new(preload.is_some()));
        let shader = |name: &str| register_effect_shader(&host, &shaders, &preloading, name);
        let sound = |path: &str| host.register_sound(path);
        let model = |path: &str| {
            host.load_content_model(&content, path)
                .ok_or_else(|| Q3EffectError::MissingModel(path.to_string()))
        };
        let gib = |part: &str| model(&format!("models/gibs/{part}.md3"));
        let media = EffectMedia {
            water_bubble_shader: Some(shader("waterBubble")?),
            smoke_puff_rage_pro_shader: Some(shader("smokePuffRagePro")?),
            blood_explosion_shader: Some(shader("bloodExplosion")?),
            teleport_effect_model: model(if product == Product::Baseq3 {
                "models/misc/telep.md3"
            } else {
                "models/powerups/pop.md3"
            })?,
            gib_skull: gib("skull")?,
            gib_brain: gib("brain")?,
            gib_abdomen: gib("abdomen")?,
            gib_arm: gib("arm")?,
            gib_chest: gib("chest")?,
            gib_fist: gib("fist")?,
            gib_foot: gib("foot")?,
            gib_forearm: gib("forearm")?,
            gib_intestine: gib("intestine")?,
            gib_leg: gib("leg")?,
            smoke2: model("models/weapons2/shells/s_shell.md3")?,
            variant: if product == Product::Baseq3 {
                EffectMediaVariant::Base {
                    teleport_effect_shader: Some(shader("teleportEffect")?),
                }
            } else {
                EffectMediaVariant::Mission(MissionEffectMedia {
                    lightning_shader: Some(shader("lightningBolt")?),
                    kamikaze_effect_model: model("models/weaphits/kamboom2.md3")?,
                    dish_flash_model: model("models/weaphits/boom01.md3")?,
                    rocket_explosion_shader: Some(shader("rocketExplosion")?),
                    obelisk_hit_sounds: [
                        sound("sound/items/obelisk_hit_01.wav"),
                        sound("sound/items/obelisk_hit_02.wav"),
                        sound("sound/items/obelisk_hit_03.wav"),
                    ],
                    invulnerability_impact_model: model("models/powerups/shield/impact.md3")?,
                    invulnerability_impact_sounds: [
                        sound("sound/items/invul_impact_01.wav"),
                        sound("sound/items/invul_impact_02.wav"),
                        sound("sound/items/invul_impact_03.wav"),
                    ],
                    invulnerability_juiced_model: model("models/powerups/shield/juicer.md3")?,
                    invulnerability_juiced_sound: sound("sound/items/invul_juiced.wav"),
                })
            },
        };
        shader("smokePuff")?;
        let effects = ClientEffects::new(
            product,
            product,
            media,
            EffectOptions {
                no_projectile_trail: false,
                blood: true,
                gibs: true,
                score_plum: true,
                hardware_rage_pro: rage_pro,
            },
            Box::new(EffectSoundImports {
                sounds: Rc::clone(&sounds),
                content: content.clone(),
                time: Rc::clone(&time),
                random: Rc::clone(&random),
            }),
        )?;
        let marks = Rc::new(RefCell::new(ImpactMarkSystem::new(
            world_mark_projector(host.mark_world())?,
            Box::new(EffectMarkOptions { time: Rc::clone(&time) }),
        )));
        let local_media = LocalEntityMedia {
            blood_trail_shader: Some(shader("bloodTrail")?),
            blood_mark_shader: Some(shader("bloodMark")?),
            burn_mark_shader: Some(shader("burnMark")?),
            number_shaders: [
                "zero_32b",
                "one_32b",
                "two_32b",
                "three_32b",
                "four_32b",
                "five_32b",
                "six_32b",
                "seven_32b",
                "eight_32b",
                "nine_32b",
                "minus_32b",
            ]
            .iter()
            .map(|name| shader(&format!("gfx/2d/numbers/{name}")).map(Some))
            .collect::<Result<Vec<_>, _>>()?,
            gib_bounce_sounds: [
                sound("sound/player/gibimp1.wav"),
                sound("sound/player/gibimp2.wav"),
                sound("sound/player/gibimp3.wav"),
            ],
        };
        let system = LocalEntitySystem::new(LocalEntityHost {
            prediction: Box::new(EffectPrediction { host: Rc::clone(&host) }),
            collision: Box::new(EffectCollision { host: Rc::clone(&host) }),
            audio: Box::new(EffectAudio {
                sounds: Rc::clone(&sounds),
                content: content.clone(),
                time: Rc::clone(&time),
                errors: Rc::clone(&adapter_errors),
            }),
            client_num: -1,
            random: Box::new(EffectRandom {
                random: Rc::clone(&random),
            }),
            marks: Box::new(EffectMarks {
                marks: Rc::clone(&marks),
                errors: Rc::clone(&adapter_errors),
            }),
            product,
            media: if product == Product::Baseq3 {
                LocalEntityHostMedia::Base(local_media)
            } else {
                LocalEntityHostMedia::Mission(MissionLocalEntityMedia {
                    base: local_media,
                    kamikaze_shock_wave: model("models/weaphits/kamwave.md3")?,
                    kamikaze_explode_sound: sound("sound/items/kam_explode.wav"),
                    kamikaze_implode_sound: sound("sound/items/kam_implode.wav"),
                })
            },
        });
        let registry = ClientWeaponMediaRegistry::new(
            product,
            Box::new(RegistryResources {
                host: Rc::clone(&host),
                content: content.clone(),
                shaders: Rc::clone(&shaders),
                preloading: Rc::clone(&preloading),
                errors: Rc::clone(&adapter_errors),
            }),
            Box::new(RegistryAudio { host: Rc::clone(&host) }),
        );
        let mut this = Self {
            content,
            host,
            product,
            time,
            random,
            sounds,
            adapter_errors,
            effects,
            pool: LocalEntityPool::new(product),
            system,
            marks,
            registry,
            weapons: None,
            pending_polys: Vec::new(),
            rage_pro,
            shaders,
            preloading,
            renderer,
            render_models: HashMap::new(),
            refs: Vec::new(),
            admission: snapshot_q3_scene_admission(SceneAdmissionOrigin::Mixed, Vec::new(), Vec::new()),
            lights: Vec::new(),
            models: Vec::new(),
            blood_owners: Vec::new(),
            projectiles: HashMap::new(),
            flashes: HashMap::new(),
            last_fires: HashMap::new(),
            bolts: HashMap::new(),
        };
        if preload == Some(Q3EffectPreload::Weapons) {
            this.load_weapons()?;
        }
        this.preloading.set(false);
        Ok(this)
    }

    /// Load the weapon media bundle once (donor `loadWeapons`).
    fn load_weapons(&mut self) -> Result<(), Q3EffectError> {
        if self.weapons.is_some() {
            return Ok(());
        }
        self.registry.register_weapon(Weapon::WpMachinegun as i32)?;
        self.drain_adapter_errors()?;
        let mission = self.product == Product::Missionpack;
        let host = Rc::clone(&self.host);
        let shaders = Rc::clone(&self.shaders);
        let preloading = Rc::clone(&self.preloading);
        let errors = Rc::clone(&self.adapter_errors);
        let content = self.content.clone();
        let shader = |name: &str| register_effect_shader(&host, &shaders, &preloading, name);
        let sound = |path: &str| host.register_sound(path);
        let model = |path: &str| {
            host.load_content_model(&content, path)
                .ok_or_else(|| Q3EffectError::MissingModel(path.to_string()))
        };
        let mission_sound = |path: &str| if mission { sound(path) } else { None };
        let smoke = shader("smokePuff")?;
        let nail_smoke = if mission { Some(shader("nailtrail")?) } else { None };
        let shotgun_smoke = shader("shotgunSmokePuff")?;
        let quad = sound("sound/items/damage3.wav");
        let weapon_media = WeaponPresentationMedia {
            models: WeaponPresentationModels {
                // Brass is view-weapon-only; impact emission never reads it.
                machinegun_brass: default_model(),
                shotgun_brass: default_model(),
                dish_flash: model("models/weaphits/boom01.md3")?,
                ring_flash: model("models/weaphits/ring02.md3")?,
                bullet_flash: model("models/weaphits/bullet.md3")?,
            },
            shaders: WeaponPresentationShaders {
                smoke_puff: Some(smoke.clone()),
                nail_puff: nail_smoke.clone(),
                shotgun_smoke_puff: Some(shotgun_smoke.clone()),
                // View-weapon rows unused by impact/rail/plasma emission.
                invis: None,
                battle_weapon: None,
                quad_weapon: None,
                select: None,
                noammo: None,
                hole_mark: Some(shader("gfx/damage/hole_lg_mrk")?),
                burn_mark: Some(shader("gfx/damage/burn_med_mrk")?),
                energy_mark: Some(shader("gfx/damage/plasma_mrk")?),
                bullet_mark: Some(shader("gfx/damage/bullet_mrk")?),
                tracer: None,
            },
            sounds: WeaponPresentationSounds {
                quad: quad.clone(),
                nail_hit_flesh: mission_sound("sound/weapons/nailgun/wnalimpl.wav"),
                nail_hit_metal: mission_sound("sound/weapons/nailgun/wnalimpm.wav"),
                nail_hit: mission_sound("sound/weapons/nailgun/wnalimpd.wav"),
                prox_explosion: mission_sound("sound/weapons/proxmine/wstbexpl.wav"),
                rocket_explosion: sound("sound/weapons/rocket/rocklx1a.wav"),
                plasma_explosion: sound("sound/weapons/plasma/plasmx1a.wav"),
                chaingun_hit_flesh: mission_sound("sound/weapons/vulcan/wvulimpl.wav"),
                chaingun_hit_metal: mission_sound("sound/weapons/vulcan/wvulimpm.wav"),
                chaingun_hit: mission_sound("sound/weapons/vulcan/wvulimpd.wav"),
                ricochet1: sound("sound/weapons/machinegun/ric1.wav"),
                ricochet2: sound("sound/weapons/machinegun/ric2.wav"),
                ricochet3: sound("sound/weapons/machinegun/ric3.wav"),
                tracer: None,
            },
        };
        let mut resources = ParticleResourcesAdapter {
            host: Rc::clone(&host),
            shaders: Rc::clone(&shaders),
            preloading: Rc::clone(&preloading),
            errors: Rc::clone(&errors),
        };
        let animations = load_particle_animations(&mut resources);
        drop(resources);
        self.drain_adapter_errors()?;
        shader("gfx/misc/tracer")?;
        let tracer_shader = ParticleShader::named("gfx/misc/tracer");
        let particles = ParticleSystem::new(
            ParticleClientState {
                time: self.time.get() as f32,
                view_axis: [vec3(1.0, 0.0, 0.0), vec3(0.0, 1.0, 0.0), vec3(0.0, 0.0, 1.0)],
                player_origin: None,
            },
            ParticleHost {
                animations: ParticleAnimationSet::Marks(animations),
                media: ParticleMedia {
                    tracer_shader: Some(tracer_shader),
                    smoke_puff_shader: Some(ParticleShader::named("smokePuff")),
                    water_bubble_shader: Some(ParticleShader::named("waterBubble")),
                },
                prediction: Box::new(ParticleTracerAdapter { host: Rc::clone(&host) }),
                random: Qrand::new(self.random.borrow().seed(), 0),
                hardware: if self.rage_pro {
                    ParticleHardware::RagePro
                } else {
                    ParticleHardware::Generic
                },
                config_string: Box::new(|_| String::new()),
                print: Box::new(|message| panic!("{message}")),
            },
        )?;
        self.weapons = Some(WeaponBundle {
            media: weapon_media,
            particles,
            plasma: shader("sprites/plasma1")?,
            smoke,
            nail_smoke,
            shotgun_smoke,
            quad,
            bounce: [
                sound("sound/weapons/grenade/hgrenb1a.wav"),
                sound("sound/weapons/grenade/hgrenb2a.wav"),
            ],
        });
        Ok(())
    }

    /// Drain stored-adapter errors, failing on the first.
    fn drain_adapter_errors(&self) -> Result<(), Q3EffectError> {
        if let Some(error) = self.adapter_errors.borrow_mut().drain(..).next() {
            return Err(error);
        }
        Ok(())
    }

    /// Current effect frame over the shared clock (donor `ClientEffects`
    /// state: null snap, predicted client zero).
    fn effect_frame(&self) -> EffectFrame {
        EffectFrame {
            time: self.time.get(),
            product: self.product,
            snap_client: None,
            predicted_client: 0,
        }
    }

    /// Present one shared ballistic event (donor `ballistic`, sync).
    pub fn ballistic(&mut self, event: &Q3SharedBallisticEvent) -> Result<(), Q3EffectError> {
        self.time.set(event.time_milliseconds);
        if matches!(event.kind, Q3BallisticEventKind::Remove) {
            self.projectiles.remove(&event.actor);
            self.bolts.remove(&event.actor);
            return Ok(());
        }
        self.load_weapons()?;
        self.registry.register_weapon(event.weapon)?;
        self.drain_adapter_errors()?;
        let time = self.time.get();
        let frame = self.effect_frame();
        let stored = self.registry.weapon(event.weapon)?.clone();
        let bundle = self
            .weapons
            .as_mut()
            .expect("load_weapons stores the bundle on success");
        let mut bridge = WeaponHostBridge {
            host: self.host.as_ref(),
            content: &self.content,
            time: &self.time,
            random: &self.random,
            sounds: &self.sounds,
            errors: &self.adapter_errors,
            effects: &mut self.effects,
            marks: &self.marks,
            particles: &mut bundle.particles,
            media: &bundle.media,
            registry_effects: &self.registry.effects,
            product: self.product,
            rage_pro: self.rage_pro,
            refs: &mut self.refs,
            lights: &mut self.lights,
            polys: &mut self.pending_polys,
            drawing: NullWeaponDrawing,
        };
        match &event.kind {
            Q3BallisticEventKind::Remove => unreachable!("removals return above"),
            Q3BallisticEventKind::Fire { volume } => {
                let previous = self.last_fires.get(&event.actor).copied();
                self.last_fires.insert(
                    event.actor.clone(),
                    LastFire {
                        weapon: event.weapon,
                        time,
                    },
                );
                self.flashes.insert(
                    event.actor.clone(),
                    TimedBallistic {
                        event: event.clone(),
                        time,
                    },
                );
                let lightning = Weapon::from_i32(event.weapon) == Some(Weapon::WpLightning);
                let dedup = previous.is_some_and(|fire| fire.weapon == event.weapon && time - fire.time <= 50);
                if lightning && dedup {
                    return Ok(());
                }
                let available: Vec<PresentSound> =
                    stored.flash_sounds.iter().filter_map(|sound| sound.clone()).collect();
                if !available.is_empty() {
                    let pick = &available[self.random.borrow_mut().rand() as usize % available.len()];
                    push_sound(
                        &self.sounds,
                        &self.content,
                        &self.time,
                        Some(pick),
                        event.origin,
                        2,
                        *volume,
                        Some(&event.actor),
                    );
                }
            }
            Q3BallisticEventKind::Projectile { trajectory } => {
                let prior = self.projectiles.insert(
                    event.actor.clone(),
                    TimedBallistic {
                        event: event.clone(),
                        time,
                    },
                );
                let Some(prior) = prior else { return Ok(()) };
                match stored.packet.missile_trail {
                    Some(MissileTrail::Plasma) => {
                        emit_plasma_trail(
                            time,
                            event.end,
                            vec3(0.0, 0.0, 0.0),
                            stored.flash_dlight_color,
                            self.registry.effects.rail_rings_shader.clone(),
                            &mut bridge,
                            &mut self.pool,
                        )?;
                    }
                    Some(MissileTrail::Grapple) | None => {}
                    Some(_) => {
                        let origin = evaluate_trajectory(trajectory, time);
                        let previous = evaluate_trajectory(trajectory, prior.time);
                        let contents = self.host.point_contents(origin);
                        let previous_contents = self.host.point_contents(previous);
                        if trajectory.trajectory_type == TrajectoryType::TrStationary {
                            return Ok(());
                        }
                        if contents & CONTENTS_LIQUID != 0 {
                            if contents & previous_contents & CONTENTS_WATER != 0 {
                                bridge
                                    .effects
                                    .bubble_trail(&mut self.pool, &frame, previous, origin, 8.0)?;
                            }
                        } else {
                            let mut tick = (prior.time + 50) / 50 * 50;
                            while tick <= time {
                                let shader = if stored.packet.missile_trail == Some(MissileTrail::Nail) {
                                    bundle.nail_smoke.clone()
                                } else {
                                    Some(bundle.smoke.clone())
                                };
                                let handle = bridge.effects.smoke_puff(
                                    &mut self.pool,
                                    &frame,
                                    &SmokePuffOptions {
                                        origin: evaluate_trajectory(trajectory, tick),
                                        velocity: vec3(0.0, 0.0, 0.0),
                                        radius: stored.packet.trail_radius,
                                        color: vec4(1.0, 1.0, 1.0, 0.33),
                                        duration: stored.packet.trail_time,
                                        start_time: tick,
                                        fade_in_time: 0,
                                        flags: 0,
                                        shader,
                                    },
                                )?;
                                if let Some(entity) = self.pool.get_mut(handle) {
                                    entity.le_type = LocalEntityType::ScaleFade;
                                }
                                tick += 50;
                            }
                        }
                    }
                }
            }
            Q3BallisticEventKind::Shotgun { shot } => {
                let mut shotgun = ShotgunBridge {
                    weapon: bridge,
                    pool: &mut self.pool,
                    frame,
                    shooter: event.actor.clone(),
                    blood_owners: &mut self.blood_owners,
                    smoke_shader: Some(bundle.shotgun_smoke.clone()),
                };
                emit_shotgun_presentation(
                    &mut shotgun,
                    &PresentationShotgunEvent {
                        muzzle: shot.muzzle,
                        direction: shot.direction,
                        seed: shot.seed,
                    },
                );
            }
            Q3BallisticEventKind::Rail { trail } => {
                let mut start = trail.start;
                emit_rail_trail(
                    time,
                    &self.registry.effects,
                    &mut bridge,
                    &mut self.pool,
                    &frame,
                    0,
                    &mut start,
                    trail.end,
                )?;
                if let RailImpact::Surface { normal } = trail.impact {
                    let direction = byte_to_direction(direction_to_byte(Some(normal)) as i32);
                    emit_weapon_impact(
                        self.product,
                        &self.registry.effects,
                        &mut bridge,
                        &mut self.pool,
                        &frame,
                        Weapon::WpRailgun,
                        0,
                        trail.end,
                        direction,
                        ImpactSound::Default,
                    )?;
                }
            }
            Q3BallisticEventKind::Contact { contact } => match contact {
                Q3ContactEvent::GauntletQuad => {
                    push_sound(
                        &self.sounds,
                        &self.content,
                        &self.time,
                        bundle.quad.as_ref(),
                        event.origin,
                        4,
                        1.0,
                        None,
                    );
                }
                Q3ContactEvent::Hit { point, target, .. } => {
                    if let Some(sprite) = bridge.effects.bleed_at(&mut self.pool, &frame, *point, false)? {
                        self.blood_owners.push((RefEntity::Sprite(sprite), target.clone()));
                    }
                }
                Q3ContactEvent::Miss { point, normal } => {
                    emit_weapon_impact(
                        self.product,
                        &self.registry.effects,
                        &mut bridge,
                        &mut self.pool,
                        &frame,
                        Weapon::WpNone,
                        0,
                        *point,
                        *normal,
                        ImpactSound::Default,
                    )?;
                }
                Q3ContactEvent::LightningReflection { .. } => {
                    return Err(Q3EffectError::ReflectionBinding);
                }
            },
            Q3BallisticEventKind::Bounce => {
                let pick = (self.random.borrow_mut().rand() & 1) as usize;
                push_sound(
                    &self.sounds,
                    &self.content,
                    &self.time,
                    bundle.bounce[pick].as_ref(),
                    event.end,
                    0,
                    1.0,
                    None,
                );
            }
            Q3BallisticEventKind::Trail => match Weapon::from_i32(event.weapon) {
                Some(Weapon::WpLightning) => {
                    self.bolts.insert(
                        event.actor.clone(),
                        TimedBallistic {
                            event: event.clone(),
                            time,
                        },
                    );
                }
                Some(Weapon::WpGrapplingHook) => {
                    if length3(sub3(event.end, event.origin)) < 64.0 {
                        self.bolts.remove(&event.actor);
                    } else {
                        self.bolts.insert(
                            event.actor.clone(),
                            TimedBallistic {
                                event: event.clone(),
                                time,
                            },
                        );
                    }
                }
                Some(Weapon::WpRailgun) => {
                    let mut start = event.origin;
                    emit_rail_trail(
                        time,
                        &self.registry.effects,
                        &mut bridge,
                        &mut self.pool,
                        &frame,
                        0,
                        &mut start,
                        event.end,
                    )?;
                }
                _ => {}
            },
            Q3BallisticEventKind::Impact { hit_kind } => {
                self.projectiles.remove(&event.actor);
                if event.surface_flags & SURF_NOIMPACT != 0 {
                    return Ok(());
                }
                let flesh = *hit_kind == Q3ImpactKind::Flesh;
                if flesh {
                    if let Some(target) = &event.target {
                        if let Some(sprite) = bridge.effects.bleed_at(&mut self.pool, &frame, event.end, false)? {
                            self.blood_owners.push((RefEntity::Sprite(sprite), target.clone()));
                        }
                    }
                    let mission = self.product == Product::Missionpack;
                    let heavy = matches!(
                        Weapon::from_i32(event.weapon),
                        Some(Weapon::WpRocketLauncher | Weapon::WpGrenadeLauncher)
                    );
                    let mission_heavy = mission
                        && matches!(
                            Weapon::from_i32(event.weapon),
                            Some(Weapon::WpNailgun | Weapon::WpChaingun | Weapon::WpProxLauncher)
                        );
                    if !heavy && !mission_heavy {
                        return Ok(());
                    }
                }
                let weapon = Weapon::from_i32(event.weapon).ok_or(Q3EffectError::InvalidWeapon(event.weapon))?;
                let sound = if flesh {
                    ImpactSound::Flesh
                } else if event.surface_flags & SURF_METALSTEPS != 0 {
                    ImpactSound::Metal
                } else {
                    ImpactSound::Default
                };
                emit_weapon_impact(
                    self.product,
                    &self.registry.effects,
                    &mut bridge,
                    &mut self.pool,
                    &frame,
                    weapon,
                    0,
                    event.end,
                    event.normal,
                    sound,
                )?;
            }
            Q3BallisticEventKind::RailAward { .. } => {
                // Accuracy awards present no effects (the donor has no such kind).
            }
        }
        self.drain_adapter_errors()
    }

    /// Present a character event (donor `event`): teleport and jump-pad
    /// effects run, gibs burst, and the handled set reports true. Pool
    /// allocation can fail, so the boolean travels in a result.
    pub fn event(&mut self, event: &Q3CharacterPresentationEvent, origin: Vec3) -> Result<bool, Q3EffectError> {
        self.time.set(event.time_milliseconds);
        let frame = self.effect_frame();
        match event.event & !0x300 {
            entity_event::PLAYER_TELEPORT_IN | entity_event::PLAYER_TELEPORT_OUT => {
                self.effects.spawn_effect(&mut self.pool, &frame, origin)?;
                Ok(true)
            }
            entity_event::JUMP_PAD => {
                self.effects.smoke_puff(
                    &mut self.pool,
                    &frame,
                    &SmokePuffOptions {
                        origin,
                        velocity: vec3(0.0, 0.0, 1.0),
                        radius: 32.0,
                        color: vec4(1.0, 1.0, 1.0, 0.33),
                        duration: 1000,
                        start_time: event.time_milliseconds,
                        fade_in_time: 0,
                        flags: 1,
                        shader: Some(SceneShader::new("smokePuff")),
                    },
                )?;
                Ok(true)
            }
            entity_event::GIB_PLAYER => {
                self.effects.gib_player(&mut self.pool, &frame, origin)?;
                Ok(true)
            }
            entity_event::NONE
            | entity_event::FOOTSTEP
            | entity_event::FOOTSTEP_METAL
            | entity_event::FOOTSPLASH
            | entity_event::FOOTWADE
            | entity_event::SWIM
            | entity_event::STEP_4
            | entity_event::STEP_8
            | entity_event::STEP_12
            | entity_event::STEP_16
            | entity_event::FALL_SHORT
            | entity_event::FALL_MEDIUM
            | entity_event::FALL_FAR
            | entity_event::JUMP
            | entity_event::WATER_TOUCH
            | entity_event::WATER_LEAVE
            | entity_event::WATER_UNDER
            | entity_event::WATER_CLEAR
            | entity_event::NOAMMO
            | entity_event::CHANGE_WEAPON
            | entity_event::PAIN
            | entity_event::DEATH1
            | entity_event::DEATH2
            | entity_event::DEATH3
            | entity_event::OBITUARY
            | entity_event::STOPLOOPINGSOUND
            | entity_event::TAUNT => Ok(true),
            _ => Ok(false),
        }
    }

    /// Stage one frame of effect entities, lights, and models (donor
    /// `prepare`, sync).
    pub fn prepare(&mut self, time_milliseconds: i32, elapsed_milliseconds: i32) -> Result<(), Q3EffectError> {
        self.time.set(time_milliseconds);
        self.refs.clear();
        self.lights.clear();
        self.models.clear();
        self.last_fires.retain(|_, fired| time_milliseconds - fired.time <= 50);
        if let Some(bundle) = self.weapons.as_ref() {
            for (actor, stored_event) in &self.projectiles {
                let event = &stored_event.event;
                let Q3BallisticEventKind::Projectile { trajectory } = &event.kind else {
                    continue;
                };
                let weapon = self.registry.weapon(event.weapon)?;
                let admitted = if Weapon::from_i32(event.weapon) == Some(Weapon::WpPlasmagun) {
                    let mut sprite = create_sprite_entity();
                    sprite.origin = event.end;
                    sprite.radius = 16.0;
                    sprite.shading.custom_shader = Some(bundle.plasma.clone());
                    RefEntity::Sprite(sprite)
                } else {
                    let mut model = create_model_entity(weapon.packet.missile_model.clone());
                    model.origin = event.end;
                    model.old_origin = event.end;
                    model.shading.render_flags = weapon.packet.missile_renderfx | RF_NOSHADOW;
                    let mut direction = normalize3_or_zero(trajectory.delta);
                    if length3(direction) == 0.0 {
                        direction = vec3(0.0, 0.0, 1.0);
                    }
                    let spin = if trajectory.trajectory_type == TrajectoryType::TrStationary {
                        0
                    } else {
                        time_milliseconds / 4
                    };
                    let side = rotate_point_around_vector(direction, perpendicular_vector(direction), f64::from(spin));
                    model.axis = [direction, side, cross3(direction, side)];
                    RefEntity::Model(model)
                };
                self.refs.push(CapturedRef {
                    entity_index: self.refs.len(),
                    ref_entity: admitted,
                    cull_radius: 0.0,
                    hidden_for: None,
                });
                if weapon.packet.missile_dlight != 0.0 {
                    self.lights.push(ClientDynamicLight {
                        origin: event.end,
                        radius: weapon.packet.missile_dlight,
                        color: weapon.packet.missile_dlight_color,
                        additive: false,
                    });
                }
                push_loop_sound(
                    &self.sounds,
                    &self.content,
                    &self.time,
                    weapon.packet.missile_sound.as_ref(),
                    event.end,
                    actor,
                    evaluate_trajectory_delta(trajectory, time_milliseconds),
                );
            }
            for (actor, fired) in std::mem::take(&mut self.flashes) {
                if time_milliseconds - fired.time > 20 {
                    continue;
                }
                let event = &fired.event;
                let weapon = self.registry.weapon(event.weapon)?;
                let mut model = create_model_entity(weapon.flash_model.clone());
                let direction = normalize3_or_zero(sub3(event.end, event.origin));
                let side = perpendicular_vector(direction);
                model.origin = event.origin;
                model.old_origin = event.origin;
                model.axis = [direction, side, cross3(direction, side)];
                self.refs.push(CapturedRef {
                    entity_index: self.refs.len(),
                    ref_entity: RefEntity::Model(model),
                    cull_radius: 0.0,
                    hidden_for: Some(actor.clone()),
                });
                if length3(weapon.flash_dlight_color) > 0.0 {
                    self.lights.push(ClientDynamicLight {
                        origin: event.origin,
                        radius: 300.0 + (self.random.borrow_mut().rand() & 31) as f32,
                        color: weapon.flash_dlight_color,
                        additive: false,
                    });
                }
                self.flashes.insert(actor, fired);
            }
            for (actor, bolt) in std::mem::take(&mut self.bolts) {
                if time_milliseconds - bolt.time > 50 {
                    continue;
                }
                let event = &bolt.event;
                let mut lightning = create_lightning_entity();
                lightning.origin = event.origin;
                lightning.old_origin = event.end;
                lightning.shading.custom_shader = self.registry.effects.lightning_shader.clone();
                self.refs.push(CapturedRef {
                    entity_index: self.refs.len(),
                    ref_entity: RefEntity::Lightning(lightning),
                    cull_radius: 0.0,
                    hidden_for: None,
                });
                let firing = self.registry.weapon(event.weapon)?.firing_sound.clone();
                push_loop_sound(
                    &self.sounds,
                    &self.content,
                    &self.time,
                    firing.as_ref(),
                    event.origin,
                    &actor,
                    vec3(0.0, 0.0, 0.0),
                );
                self.bolts.insert(actor, bolt);
            }
        }
        // Camera-near puff removal belongs to each seat; it must not retire
        // another seat's effect.
        let far = self.host.world_bounds_max();
        let active = self.pool.active_entities();
        let mut sink = EntitySceneSink {
            refs: &mut self.refs,
            lights: &mut self.lights,
            active: &active,
            blood_owners: &self.blood_owners,
        };
        self.system.add_entities(
            &mut self.pool,
            &mut self.effects,
            &LocalEntityFrame {
                time: time_milliseconds,
                frame_time: elapsed_milliseconds,
                view_origin: vec3(far.x + 65536.0, far.y + 65536.0, far.z + 65536.0),
            },
            &mut sink,
        )?;
        let fogs = self.host.fog_selections();
        let mut polygons = Vec::new();
        for poly in self.marks.borrow_mut().add_marks() {
            polygons.push(admit_q3_poly(&poly, &fogs)?);
        }
        for poly in std::mem::take(&mut self.pending_polys) {
            polygons.push(admit_q3_poly(&poly, &fogs)?);
        }
        let entities = self
            .refs
            .iter()
            .map(|captured| Q3AdmittedRefEntity::Entity(copy_ref_entity(&captured.ref_entity)))
            .collect();
        self.admission = snapshot_q3_scene_admission(SceneAdmissionOrigin::Mixed, entities, polygons);
        for captured in &self.refs {
            let RefEntity::Model(model) = &captured.ref_entity else {
                continue;
            };
            let path = match &model.model {
                ContentSceneModel::Loaded(loaded) => loaded.path.clone(),
                // Inline brush models never ride ref entities in the donor;
                // admit them through the same render-pair cache.
                ContentSceneModel::Inline(inline) => inline.path.clone(),
                ContentSceneModel::Default(_) => continue,
            };
            let pair = if let Some(pair) = self.render_models.get(&path) {
                pair.clone()
            } else {
                let loaded = self
                    .host
                    .load_render_model(&self.content, &path)
                    .ok_or_else(|| Q3EffectError::MissingRenderModel(path.clone()))?;
                self.render_models.insert(path.clone(), loaded.clone());
                loaded
            };
            let rgba = model.shading.shader_rgba;
            // The render entity has no shadow-plane row; the donor value has
            // no Rust counterpart.
            self.models.push(PreparedEffectModel {
                entity_index: captured.entity_index,
                entity: SceneEntity {
                    resource: pair.1,
                    model: pair.0,
                    pose: ScenePose::Frame {
                        frame: model.frame,
                        previous_frame: model.old_frame,
                        back_lerp: model.back_lerp,
                    },
                    transform: EntityTransform {
                        origin: model.origin,
                        axis: model.axis,
                        scale: vec3(1.0, 1.0, 1.0),
                    },
                    previous_origin: model.old_origin,
                    lighting_origin: model.lighting_origin,
                    color: vec4(rgba.x / 255.0, rgba.y / 255.0, rgba.z / 255.0, rgba.w / 255.0),
                    skin: model.skin_num,
                    shader_time_seconds: f64::from(model.shading.shader_time),
                    flags: EntityFlags::Q3 {
                        bits: model.shading.render_flags as u32,
                    },
                    attachments: Vec::new(),
                    actor_slot: None,
                },
                custom_shader: model.shading.custom_shader.as_ref().map(|shader| shader.name.clone()),
            });
        }
        for staged in &self.models {
            let options = effect_model_options(staged.custom_shader.clone(), None);
            self.renderer
                .preload(std::slice::from_ref(&staged.entity), &|_| options.clone())?;
        }
        Ok(())
    }

    /// Submit staged effects for one camera (donor `frame`).
    pub fn frame(
        &mut self,
        camera: &SceneCamera,
        source: &SourceSceneOrder,
        viewer: Option<&ActorId>,
        q1_fog: Option<Q1FogInput>,
    ) -> Result<Q3EffectFrameOutput, Q3EffectError> {
        let time = self.time.get();
        if let Some(bundle) = self.weapons.as_mut() {
            bundle.particles.set_time(time as f32, camera.axis);
        }
        let fogs = self.host.fog_selections();
        let mut polygons = self.admission.polygons.clone();
        if let Some(bundle) = self.weapons.as_mut() {
            for poly in bundle.particles.add_particles(Some(camera.origin))? {
                polygons.push(admit_q3_poly(&particle_poly_to_content(&poly), &fogs)?);
            }
        }
        let admission =
            snapshot_q3_scene_admission(SceneAdmissionOrigin::Mixed, self.admission.entities.clone(), polygons);
        let first_entity = reserve_source_entity_range(source, admission.entities.len() as u32)?;
        let mut operations = Vec::new();
        for (index, admitted) in admission.polygons.iter().enumerate() {
            let Some(shader) = admitted.poly.shader.as_ref() else {
                continue;
            };
            let material = self
                .shaders
                .borrow()
                .get(&shader.name)
                .cloned()
                .ok_or_else(|| Q3EffectError::UnregisteredMarkShader(shader.name.clone()))?;
            let geometry = poly_geometry(&content_poly_vertices(&admitted.poly));
            let batches = self.host.prepare_material(
                &material,
                &geometry,
                &draw_material_context(
                    camera,
                    time,
                    [255, 255, 255, 255],
                    vec2(0.0, 0.0),
                    0.0,
                    admitted.fog,
                    q1_fog,
                ),
            )?;
            operations.push(SceneOperation::Group(source_draw_group(
                material,
                SourceSurfaceOrder {
                    view: source.clone(),
                    entity: SourceEntityOrder::World,
                    surface: index as u32,
                    fog: admitted.fog.as_ref().map(|fog| fog.index as u32 + 1).unwrap_or(0),
                    dlight: 0,
                },
                batches,
            )?));
        }
        let models: HashMap<usize, &SceneEntity> = self
            .models
            .iter()
            .map(|staged| (staged.entity_index, &staged.entity))
            .collect();
        let customs: HashMap<usize, Option<String>> = self
            .models
            .iter()
            .map(|staged| (staged.entity_index, staged.custom_shader.clone()))
            .collect();
        let project = ViewProjector::new(*camera, None);
        // Degenerate projections collapse to the clip origin.
        let project_point = |point: Vec3| project.project(point).unwrap_or(vec4(0.0, 0.0, 0.0, 0.0));
        let default_material = self.host.default_material().clone();
        let white = self.host.white_image();
        let input = ModelViewInput {
            camera: *camera,
            time_seconds: f64::from(time) / 1000.0,
            dynamic_lights: Vec::new(),
            q2_lights: Vec::new(),
            q2_atlas: None,
            q3_lights: self.lights.clone(),
            identity_light: 1.0,
            q3_world: false,
            q2_world: false,
        };
        let portal = matches!(camera.clip, CameraClip::Portal { .. });
        let mirror = matches!(camera.clip, CameraClip::Portal { mirror: true, .. });
        for captured in &self.refs {
            let flags = ref_render_flags(&captured.ref_entity);
            if portal && flags & RF_FIRST_PERSON != 0 {
                continue;
            }
            if let Some(viewer) = viewer {
                if captured.hidden_for.as_ref() == Some(viewer) {
                    continue;
                }
            }
            let order_entity = SourceEntityOrder::RefEntity {
                index: first_entity + captured.entity_index as u32,
            };
            match &captured.ref_entity {
                RefEntity::Portal(_) => continue,
                RefEntity::Model(model) => {
                    if matches!(model.model, ContentSceneModel::Default(_)) {
                        if !portal && flags & RF_THIRD_PERSON != 0 {
                            continue;
                        }
                        let batch = default_model_batch(
                            &EntityTransform {
                                origin: model.origin,
                                axis: model.axis,
                                scale: vec3(1.0, 1.0, 1.0),
                            },
                            &project_point,
                            &PRIMITIVE_STATE,
                            white,
                        );
                        operations.push(SceneOperation::Group(source_draw_group(
                            default_material.clone(),
                            SourceSurfaceOrder {
                                view: source.clone(),
                                entity: order_entity,
                                surface: 0,
                                fog: 0,
                                dlight: 0,
                            },
                            vec![batch],
                        )?));
                        continue;
                    }
                    let entity = models
                        .get(&captured.entity_index)
                        .copied()
                        .ok_or(Q3EffectError::MissingEntityDescriptor)?;
                    let custom = customs.get(&captured.entity_index).cloned().unwrap_or(None);
                    for group in self.renderer.prepare(
                        std::slice::from_ref(entity),
                        &input,
                        &|_| effect_model_options(custom.clone(), Some(model.shading.shader_tex_coord)),
                        None,
                    )? {
                        operations.push(model_group_operation(group, &default_material));
                    }
                }
                RefEntity::Sprite(sprite) => {
                    if !portal && flags & RF_THIRD_PERSON != 0 {
                        continue;
                    }
                    if length3(sub3(sprite.origin, camera.origin)) < captured.cull_radius {
                        continue;
                    }
                    let material = effect_shader_material(
                        &self.shaders.borrow(),
                        sprite.shading.custom_shader.as_ref(),
                        &default_material,
                    )?;
                    let fog = q3_procedural_fog(sprite.origin, sprite.radius, &fogs);
                    let fog_index = fog.as_ref().map(|fog| fog.index as u32 + 1).unwrap_or(0);
                    let geometry = sprite_geometry(
                        &SpritePose {
                            origin: sprite.origin,
                            radius: sprite.radius,
                            rotation: sprite.rotation,
                            shader_rgba: sprite.shading.shader_rgba,
                        },
                        &camera.axis,
                        mirror,
                    );
                    let batches = self.host.prepare_material(
                        &material,
                        &geometry,
                        &draw_material_context(
                            camera,
                            time,
                            color_bytes(sprite.shading.shader_rgba),
                            sprite.shading.shader_tex_coord,
                            sprite.shading.shader_time,
                            fog,
                            q1_fog,
                        ),
                    )?;
                    operations.push(SceneOperation::Group(source_draw_group(
                        material,
                        SourceSurfaceOrder {
                            view: source.clone(),
                            entity: order_entity,
                            surface: 0,
                            fog: fog_index,
                            dlight: 0,
                        },
                        batches,
                    )?));
                }
                RefEntity::Beam(beam) => {
                    if !portal && flags & RF_THIRD_PERSON != 0 {
                        continue;
                    }
                    if length3(sub3(beam.origin, camera.origin)) < captured.cull_radius {
                        continue;
                    }
                    let material = effect_shader_material(
                        &self.shaders.borrow(),
                        beam.shading.custom_shader.as_ref(),
                        &default_material,
                    )?;
                    let fog = q3_procedural_fog(beam.origin, beam.radius, &fogs);
                    let fog_index = fog.as_ref().map(|fog| fog.index as u32 + 1).unwrap_or(0);
                    let batch = beam_batch(
                        &BeamPose {
                            origin: beam.origin,
                            old_origin: beam.old_origin,
                        },
                        &project_point,
                        &PRIMITIVE_STATE,
                        white,
                    );
                    operations.push(SceneOperation::Group(source_draw_group(
                        material,
                        SourceSurfaceOrder {
                            view: source.clone(),
                            entity: order_entity,
                            surface: 0,
                            fog: fog_index,
                            dlight: 0,
                        },
                        vec![batch],
                    )?));
                }
                RefEntity::RailCore(rail) => {
                    self.push_rail(
                        rail.origin,
                        rail.old_origin,
                        rail.radius,
                        &rail.shading,
                        RailKind::RailCore,
                        camera,
                        source,
                        order_entity,
                        captured.cull_radius,
                        &fogs,
                        &default_material,
                        q1_fog,
                        &mut operations,
                    )?;
                }
                RefEntity::RailRings(rail) => {
                    self.push_rail(
                        rail.origin,
                        rail.old_origin,
                        rail.radius,
                        &rail.shading,
                        RailKind::RailRings,
                        camera,
                        source,
                        order_entity,
                        captured.cull_radius,
                        &fogs,
                        &default_material,
                        q1_fog,
                        &mut operations,
                    )?;
                }
                RefEntity::Lightning(bolt) => {
                    self.push_rail(
                        bolt.origin,
                        bolt.old_origin,
                        bolt.radius,
                        &bolt.shading,
                        RailKind::Lightning,
                        camera,
                        source,
                        order_entity,
                        captured.cull_radius,
                        &fogs,
                        &default_material,
                        q1_fog,
                        &mut operations,
                    )?;
                }
            }
        }
        Ok(Q3EffectFrameOutput {
            admission,
            operations,
            q3_lights: self.lights.clone(),
        })
    }

    /// Submit one rail-family entity (donor `railGeometry` arm shared by rail
    /// core, rail rings, and lightning).
    #[allow(clippy::too_many_arguments)]
    fn push_rail(
        &self,
        origin: Vec3,
        old_origin: Vec3,
        radius: f32,
        shading: &ShadedFields,
        kind: RailKind,
        camera: &SceneCamera,
        source: &SourceSceneOrder,
        order_entity: SourceEntityOrder,
        cull_radius: f32,
        fogs: &[Q3FogSelection],
        default_material: &RegisteredSceneMaterial,
        q1_fog: Option<Q1FogInput>,
        operations: &mut Vec<SceneOperation>,
    ) -> Result<(), Q3EffectError> {
        let portal = matches!(camera.clip, CameraClip::Portal { .. });
        if !portal && shading.render_flags & RF_THIRD_PERSON != 0 {
            return Ok(());
        }
        if length3(sub3(origin, camera.origin)) < cull_radius {
            return Ok(());
        }
        let material =
            effect_shader_material(&self.shaders.borrow(), shading.custom_shader.as_ref(), default_material)?;
        let fog = q3_procedural_fog(origin, radius, fogs);
        let fog_index = fog.as_ref().map(|fog| fog.index as u32 + 1).unwrap_or(0);
        let geometry = rail_geometry(
            &RailPose {
                kind,
                origin,
                old_origin,
                shader_rgba: shading.shader_rgba,
            },
            camera.origin,
            &DEFAULT_RAIL_SETTINGS,
        )?;
        let batches = self.host.prepare_material(
            &material,
            &geometry,
            &draw_material_context(
                camera,
                self.time.get(),
                color_bytes(shading.shader_rgba),
                shading.shader_tex_coord,
                shading.shader_time,
                fog,
                q1_fog,
            ),
        )?;
        operations.push(SceneOperation::Group(source_draw_group(
            material,
            SourceSurfaceOrder {
                view: source.clone(),
                entity: order_entity,
                surface: 0,
                fog: fog_index,
                dlight: 0,
            },
            batches,
        )?));
        Ok(())
    }

    /// Drain queued source effect sounds (donor `drainSounds`).
    pub fn drain_sounds(&mut self) -> Vec<SourceEffectSound> {
        std::mem::take(&mut self.sounds.borrow_mut())
    }

    /// Reset per-round effect state (donor `resetRound`).
    pub fn reset_round(&mut self) {
        let active = self.pool.active_entities();
        self.blood_owners
            .retain(|(blood, _)| !active.iter().any(|local| local.ref_entity == *blood));
        self.pool.initialize();
        self.marks.borrow_mut().reset();
        if let Some(bundle) = self.weapons.as_mut() {
            bundle.particles.reset_round();
        }
        self.refs.clear();
        self.admission = snapshot_q3_scene_admission(SceneAdmissionOrigin::Mixed, Vec::new(), Vec::new());
        self.lights.clear();
        self.models.clear();
        self.sounds.borrow_mut().clear();
        self.pending_polys.clear();
        self.projectiles.clear();
        self.flashes.clear();
        self.last_fires.clear();
        self.bolts.clear();
    }

    /// Close the effects (donor `close`).
    pub fn close(&mut self) {
        self.reset_round();
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
    use qa_client::render::scene::models::types::ModelSkinningFrame;
    use qa_client::render::scene::submissions::create_source_scene_order;
    use qa_client::render::types::{ImageSource, RendererImage, ResourceOwner};
    use qa_client::view::Rect;
    use qa_content::catalog::{CatalogProduct, ProductAvailability, ProductExpectation};
    use qa_content::contract::GameFamily;
    use qa_content::q3::base::game::hitscan::Q3RailTrail;
    use qa_content::q3::base::shared::trajectory::Trajectory;
    use qa_content::q3::presentation::ref_entity::{PresentResource, Q3DecodedModel, SceneLoadedModel};
    use qa_core::identity::IdentityOwner;
    use qa_core::math::{identity_mat4, Bounds as MathBounds};

    fn test_material(name: &str) -> RegisteredSceneMaterial {
        let definition = ShaderDefinition {
            name: name.to_string(),
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
            name: name.to_string(),
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

    struct TestHost {
        white: RendererImage,
        default_material: RegisteredSceneMaterial,
        mark_world: PresentMarkWorld,
        cinematic: Option<String>,
        missing_models: bool,
        recorded: Rc<RefCell<Vec<RegisteredSceneMaterial>>>,
    }

    impl TestHost {
        fn new() -> Self {
            let authority = IdentityOwner::create("q3-effects-test").expect("owner");
            Self {
                white: RendererImage {
                    owner: ResourceOwner::new(1, authority.session().clone(), 0),
                    ordinal: 0,
                    source: ImageSource::Generated {
                        name: "white".to_string(),
                    },
                    width: 2,
                    height: 2,
                },
                default_material: test_material("default"),
                mark_world: PresentMarkWorld {
                    nodes: Vec::new(),
                    planes: Vec::new(),
                    leaves: Vec::new(),
                    leaf_surfaces: Vec::new(),
                    surfaces: Vec::new(),
                },
                cinematic: None,
                missing_models: false,
                recorded: Rc::new(RefCell::new(Vec::new())),
            }
        }
    }

    impl Q3EffectHost for TestHost {
        fn register_sound(&self, path: &str) -> Option<PresentSound> {
            Some(PresentSound::new(path))
        }

        fn register_material(&self, name: &str) -> Option<RegisteredSceneMaterial> {
            let material = test_material(name);
            self.recorded.borrow_mut().push(material.clone());
            Some(material)
        }

        fn has_cinematic(&self, name: &str) -> bool {
            self.cinematic.as_deref() == Some(name)
        }

        fn mount_exists(&self, _content: &ContentId, _path: &str) -> bool {
            true
        }

        fn load_content_model(&self, _content: &ContentId, path: &str) -> Option<ContentSceneModel> {
            if self.missing_models {
                return None;
            }
            Some(ContentSceneModel::Loaded(SceneLoadedModel {
                path: path.to_string(),
                model: Q3DecodedModel::Bounded {
                    bounds: MathBounds {
                        min: vec3(-8.0, -8.0, -8.0),
                        max: vec3(8.0, 8.0, 8.0),
                    },
                },
                resource: PresentResource::new(path),
            }))
        }

        fn load_render_model(&self, _content: &ContentId, path: &str) -> Option<(RenderSceneModel, ModelResource)> {
            Some((
                RenderSceneModel::BrushModel,
                ModelResource {
                    id: path.to_string(),
                    requested_path: path.to_string(),
                    digest: 0,
                },
            ))
        }

        fn is_player(&self, _actor: &ActorId) -> bool {
            true
        }

        fn trace(
            &self,
            _start: Vec3,
            end: Vec3,
            _bounds: Option<MathBounds>,
            _pass_actor: Option<&ActorId>,
            _contents_mask: i32,
        ) -> Q3EffectTrace {
            Q3EffectTrace {
                fraction: 1.0,
                end,
                all_solid: false,
                start_solid: false,
                contact: None,
                contents: 0,
                surface_flags: 0,
                hit_actor: None,
            }
        }

        fn point_contents(&self, _point: Vec3) -> i32 {
            0
        }

        fn world_bounds_max(&self) -> Vec3 {
            vec3(4096.0, 4096.0, 4096.0)
        }

        fn fog_selections(&self) -> Vec<Q3FogSelection> {
            Vec::new()
        }

        fn mark_world(&self) -> &PresentMarkWorld {
            &self.mark_world
        }

        fn white_image(&self) -> &RendererImage {
            &self.white
        }

        fn default_material(&self) -> &RegisteredSceneMaterial {
            &self.default_material
        }

        fn prepare_material(
            &self,
            _material: &RegisteredSceneMaterial,
            _geometry: &MaterialGeometry,
            _context: &Q3EffectMaterialContext,
        ) -> Result<Vec<DrawBatch>, Q3EffectError> {
            Ok(Vec::new())
        }
    }

    struct TestRenderer {
        preloaded: RefCell<Vec<SceneEntity>>,
    }

    impl Q3ModelRenderer for TestRenderer {
        fn preload(
            &mut self,
            entities: &[SceneEntity],
            _options: &dyn Fn(&SceneEntity) -> ModelSourceOptions,
        ) -> Result<(), RenderError> {
            self.preloaded.borrow_mut().extend(entities.iter().cloned());
            Ok(())
        }

        fn prepare(
            &self,
            _entities: &[SceneEntity],
            _input: &ModelViewInput,
            _options: &dyn Fn(&SceneEntity) -> ModelSourceOptions,
            _skinning: Option<&ModelSkinningFrame>,
        ) -> Result<Vec<ModelDrawGroup>, RenderError> {
            Ok(vec![ModelDrawGroup {
                order: ModelGroupOrder::Opaque,
                batches: Vec::new(),
            }])
        }
    }

    fn catalog() -> InstalledCatalog {
        InstalledCatalog::new(
            String::new(),
            vec![CatalogProduct {
                id: ContentId("baseq3".to_string()),
                expectation: ProductExpectation {
                    id: "baseq3".to_string(),
                    family: GameFamily::Q3,
                    edition: "classic".to_string(),
                    campaign: "baseq3".to_string(),
                    title: String::new(),
                    content_directory: String::new(),
                    base_product: None,
                    required_content_archives: Vec::new(),
                    required_programs: Vec::new(),
                    map_witness: None,
                    unresolved_reason: None,
                },
                availability: ProductAvailability::Installed,
                archives: Vec::new(),
                loose_root: None,
                user_content: None,
                maps: Vec::new(),
                diagnostics: Vec::new(),
            }],
            Vec::new(),
            0,
            None,
        )
        .expect("catalog")
    }

    fn effects_with(
        preload: Option<Q3EffectPreload>,
    ) -> (
        Q3ApplicationEffects<TestHost>,
        Rc<RefCell<Vec<RegisteredSceneMaterial>>>,
    ) {
        let catalog = catalog();
        let recorded = Rc::new(RefCell::new(Vec::new()));
        let mut host = TestHost::new();
        host.recorded = Rc::clone(&recorded);
        recorded.borrow_mut().push(host.default_material.clone());
        let effects = Q3ApplicationEffects::create(
            ContentId("baseq3".to_string()),
            &catalog,
            host,
            Box::new(TestRenderer {
                preloaded: RefCell::new(Vec::new()),
            }),
            preload,
            None,
        )
        .expect("effects create");
        (effects, recorded)
    }

    fn effects(preload: Option<Q3EffectPreload>) -> Q3ApplicationEffects<TestHost> {
        effects_with(preload).0
    }

    fn ranked_source(recorded: &RefCell<Vec<RegisteredSceneMaterial>>) -> SourceSceneOrder {
        create_source_scene_order(recorded.borrow().iter().map(|material| material.registration).collect())
    }

    fn actor(slot: u32) -> ActorId {
        IdentityOwner::create("q3-effects-actor").expect("owner").actor(slot, 0)
    }

    fn ballistic(actor: ActorId, weapon: i32, kind: Q3BallisticEventKind, time: i32) -> Q3SharedBallisticEvent {
        Q3SharedBallisticEvent {
            actor,
            weapon,
            origin: vec3(0.0, 0.0, 0.0),
            end: vec3(100.0, 0.0, 0.0),
            normal: vec3(0.0, 0.0, 1.0),
            target: None,
            surface_flags: 0,
            kind,
            time_milliseconds: time,
        }
    }

    fn camera() -> SceneCamera {
        SceneCamera {
            origin: vec3(0.0, -200.0, 64.0),
            axis: [vec3(1.0, 0.0, 0.0), vec3(0.0, 1.0, 0.0), vec3(0.0, 0.0, 1.0)],
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

    fn linear_trajectory() -> Trajectory {
        Trajectory {
            trajectory_type: TrajectoryType::TrLinear,
            time: 0,
            duration: 0,
            base: vec3(0.0, 0.0, 0.0),
            delta: vec3(100.0, 0.0, 0.0),
        }
    }

    #[test]
    fn create_character_media_starts_quiet() {
        let mut effects = effects(None);
        assert!(effects.drain_sounds().is_empty());
        effects.close();
    }

    #[test]
    fn create_weapons_bundle_preloads() {
        let mut effects = effects(Some(Q3EffectPreload::Weapons));
        assert!(effects.drain_sounds().is_empty());
        effects.close();
    }

    #[test]
    fn create_defers_cinematic_while_preloading() {
        let catalog = catalog();
        let mut host = TestHost::new();
        host.cinematic = Some("waterBubble".to_string());
        let result = Q3ApplicationEffects::create(
            ContentId("baseq3".to_string()),
            &catalog,
            host,
            Box::new(TestRenderer {
                preloaded: RefCell::new(Vec::new()),
            }),
            Some(Q3EffectPreload::Character),
            None,
        );
        assert!(matches!(result, Err(Q3EffectError::CinematicDeferred(name)) if name == "waterBubble"));
    }

    #[test]
    fn create_missing_model_errors() {
        let catalog = catalog();
        let mut host = TestHost::new();
        host.missing_models = true;
        let result = Q3ApplicationEffects::create(
            ContentId("baseq3".to_string()),
            &catalog,
            host,
            Box::new(TestRenderer {
                preloaded: RefCell::new(Vec::new()),
            }),
            None,
            None,
        );
        assert!(matches!(result, Err(Q3EffectError::MissingModel(_))));
    }

    #[test]
    fn fire_queues_actor_flash_sound() {
        let mut effects = effects(Some(Q3EffectPreload::Weapons));
        let shooter = actor(1);
        effects
            .ballistic(&ballistic(
                shooter.clone(),
                2,
                Q3BallisticEventKind::Fire { volume: 0.8 },
                100,
            ))
            .expect("fire");
        let sounds = effects.drain_sounds();
        assert_eq!(sounds.len(), 1);
        assert_eq!(sounds[0].channel, 2);
        assert_eq!(sounds[0].volume, 0.8);
        assert_eq!(sounds[0].seconds, 0.1);
        assert_eq!(sounds[0].content, ContentId("baseq3".to_string()));
        assert!(matches!(sounds[0].playback, SourceEffectPlayback::Actor { ref actor } if *actor == shooter));
        assert!(effects.drain_sounds().is_empty());
    }

    #[test]
    fn lightning_fire_dedups_within_window() {
        let mut effects = effects(Some(Q3EffectPreload::Weapons));
        let shooter = actor(2);
        effects
            .ballistic(&ballistic(
                shooter.clone(),
                6,
                Q3BallisticEventKind::Fire { volume: 1.0 },
                0,
            ))
            .expect("first fire");
        let first = effects.drain_sounds();
        assert_eq!(first.len(), 1);
        effects
            .ballistic(&ballistic(shooter, 6, Q3BallisticEventKind::Fire { volume: 1.0 }, 30))
            .expect("second fire");
        assert!(effects.drain_sounds().is_empty());
    }

    #[test]
    fn bounce_queues_positional_sound() {
        let mut effects = effects(Some(Q3EffectPreload::Weapons));
        effects
            .ballistic(&ballistic(actor(3), 4, Q3BallisticEventKind::Bounce, 10))
            .expect("bounce");
        let sounds = effects.drain_sounds();
        assert_eq!(sounds.len(), 1);
        assert_eq!(sounds[0].channel, 0);
        assert_eq!(sounds[0].volume, 1.0);
        assert_eq!(sounds[0].playback, SourceEffectPlayback::Once);
    }

    #[test]
    fn reflection_binding_errors() {
        let mut effects = effects(Some(Q3EffectPreload::Weapons));
        let result = effects.ballistic(&ballistic(
            actor(4),
            6,
            Q3BallisticEventKind::Contact {
                contact: Q3ContactEvent::LightningReflection {
                    start: vec3(0.0, 0.0, 0.0),
                    end: vec3(1.0, 0.0, 0.0),
                },
            },
            0,
        ));
        assert!(matches!(result, Err(Q3EffectError::ReflectionBinding)));
    }

    #[test]
    fn impact_with_unregistered_weapon_errors() {
        let mut effects = effects(Some(Q3EffectPreload::Weapons));
        let result = effects.ballistic(&ballistic(
            actor(5),
            14,
            Q3BallisticEventKind::Impact {
                hit_kind: Q3ImpactKind::Wall,
            },
            0,
        ));
        let error = result.expect_err("weapon 14 has no registry record");
        assert!(
            error.to_string().contains("Couldn't find weapon 14"),
            "unexpected: {error}"
        );
    }

    #[test]
    fn event_routes_by_number() {
        let mut effects = effects(Some(Q3EffectPreload::Weapons));
        let origin = vec3(10.0, 20.0, 30.0);
        let present = |effects: &mut Q3ApplicationEffects<TestHost>, number: i32| {
            effects
                .event(
                    &Q3CharacterPresentationEvent {
                        actor: actor(6),
                        sequence: 0,
                        time_milliseconds: 50,
                        event: number,
                        parameter: 0,
                    },
                    origin,
                )
                .expect("event")
        };
        assert!(present(&mut effects, entity_event::PLAYER_TELEPORT_IN));
        assert!(present(&mut effects, entity_event::PLAYER_TELEPORT_OUT | 0x100));
        assert!(present(&mut effects, entity_event::JUMP_PAD));
        assert!(present(&mut effects, entity_event::GIB_PLAYER));
        assert!(present(&mut effects, entity_event::FOOTSTEP));
        assert!(!present(&mut effects, entity_event::ITEM_PICKUP));
    }

    #[test]
    fn prepare_and_frame_submit() {
        let (mut effects, recorded) = effects_with(Some(Q3EffectPreload::Weapons));
        effects
            .ballistic(&ballistic(actor(7), 2, Q3BallisticEventKind::Fire { volume: 1.0 }, 0))
            .expect("fire");
        effects.prepare(0, 16).expect("prepare");
        let source = ranked_source(&recorded);
        let output = effects.frame(&camera(), &source, None, None).expect("frame");
        assert!(!output.admission.entities.is_empty());
        assert!(!output.operations.is_empty());
    }

    #[test]
    fn frame_hides_blood_for_owner() {
        let (mut effects, recorded) = effects_with(Some(Q3EffectPreload::Weapons));
        let target = actor(8);
        let mut hit = ballistic(actor(9), 1, Q3BallisticEventKind::Bounce, 0);
        hit.kind = Q3BallisticEventKind::Contact {
            contact: Q3ContactEvent::Hit {
                point: vec3(50.0, 0.0, 0.0),
                normal: vec3(0.0, 0.0, 1.0),
                target: target.clone(),
            },
        };
        effects.ballistic(&hit).expect("hit");
        effects.prepare(0, 16).expect("prepare");
        let source = ranked_source(&recorded);
        let shown = effects
            .frame(&camera(), &source, None, None)
            .expect("frame")
            .operations
            .len();
        let hidden = effects
            .frame(&camera(), &source, Some(&target), None)
            .expect("frame")
            .operations
            .len();
        assert_eq!(shown, 1);
        assert_eq!(hidden, 0);
    }

    #[test]
    fn reset_round_clears_state() {
        let (mut effects, recorded) = effects_with(Some(Q3EffectPreload::Weapons));
        effects
            .ballistic(&ballistic(actor(10), 2, Q3BallisticEventKind::Fire { volume: 1.0 }, 0))
            .expect("fire");
        assert!(!effects.drain_sounds().is_empty());
        effects.reset_round();
        effects.prepare(100, 16).expect("prepare");
        let source = ranked_source(&recorded);
        let output = effects.frame(&camera(), &source, None, None).expect("frame");
        assert!(output.admission.entities.is_empty());
        assert!(output.operations.is_empty());
        assert!(effects.drain_sounds().is_empty());
    }

    #[test]
    fn presentation_kinds_run() {
        let mut effects = effects(Some(Q3EffectPreload::Weapons));
        let shooter = actor(11);
        effects
            .ballistic(&ballistic(
                shooter.clone(),
                5,
                Q3BallisticEventKind::Projectile {
                    trajectory: linear_trajectory(),
                },
                0,
            ))
            .expect("projectile");
        effects
            .ballistic(&ballistic(
                shooter.clone(),
                5,
                Q3BallisticEventKind::Projectile {
                    trajectory: linear_trajectory(),
                },
                100,
            ))
            .expect("projectile again");
        let shotgun = qa_content::q3::base::game::hitscan::Q3ShotgunEvent {
            muzzle: vec3(0.0, 0.0, 0.0),
            direction: vec3(1.0, 0.0, 0.0),
            seed: 7,
        };
        effects
            .ballistic(&ballistic(
                shooter.clone(),
                3,
                Q3BallisticEventKind::Shotgun { shot: shotgun },
                100,
            ))
            .expect("shotgun");
        effects
            .ballistic(&ballistic(
                shooter.clone(),
                7,
                Q3BallisticEventKind::Rail {
                    trail: Q3RailTrail {
                        start: vec3(0.0, 0.0, 0.0),
                        end: vec3(100.0, 0.0, 0.0),
                        impact: RailImpact::Surface {
                            normal: vec3(0.0, 0.0, 1.0),
                        },
                    },
                },
                100,
            ))
            .expect("rail");
        effects
            .ballistic(&ballistic(shooter.clone(), 6, Q3BallisticEventKind::Trail, 100))
            .expect("trail");
        effects
            .ballistic(&ballistic(
                shooter.clone(),
                1,
                Q3BallisticEventKind::Contact {
                    contact: Q3ContactEvent::Miss {
                        point: vec3(5.0, 0.0, 0.0),
                        normal: vec3(0.0, 0.0, 1.0),
                    },
                },
                100,
            ))
            .expect("miss");
        let mut impact = ballistic(shooter, 5, Q3BallisticEventKind::Bounce, 100);
        impact.target = Some(actor(12));
        impact.kind = Q3BallisticEventKind::Impact {
            hit_kind: Q3ImpactKind::Flesh,
        };
        effects.ballistic(&impact).expect("impact");
        effects.prepare(100, 16).expect("prepare");
    }
}
