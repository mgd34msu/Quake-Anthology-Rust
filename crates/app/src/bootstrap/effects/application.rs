//! Transient-effect math and orchestration ported from Quake-Anthology-TS
//! `src/app/bootstrap/effects.ts`.
//!
//! Ports the donor's dependency-free effect logic (the Quake beam model
//! table, the Quake II player muzzle-flash profile table, explosion
//! animation math, the bonus-flash decay, and the shadow-light distance
//! fade, plus the shared effect color constants) and the `ApplicationEffects`
//! orchestrator with `ApplicationEffectFrame` and `UnhandledApplicationEffect`.
//! `SourceEffectSound` is re-exported from its canonical Rust home,
//! `crate::bootstrap::media::q3` (donor `effects/q3.ts`).
//!
//! Sync mappings (the donor awaits asset providers; this port resolves the
//! same data through synchronous seams): `ApplicationAssets` plus
//! `SceneQueries` arrive as [`ApplicationEffectHost`]; Q3 sub-effects arrive
//! as [`ApplicationQ3Effects`] behind a creation factory (the real
//! [`Q3ApplicationEffects`](crate::bootstrap::media::q3::Q3ApplicationEffects)
//! implements it); `SimulationPresentationEvent`, `SimulationPresentation`,
//! and the render `WorldSnapshot` arrive as absorbed mirrors carrying exactly
//! the fields this wave reads (the simulation lane owns the canonical ports).
//! The donor's post-await owner-current rechecks collapse to the single entry
//! check because synchronous host calls cannot interleave owner revisions,
//! and the donor's `preparedRenderers` cache lives host-side.

use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use qa_client::materials::evaluate::Q1FogInput;
use qa_client::materials::lighting::SurfaceDynamicLight;
use qa_client::materials::q3_lighting::DynamicLight as ClientDynamicLight;
use qa_client::render::q3_hardware::Q3Hardware;
use qa_client::render::scene::particles::legacy::{
    prepare_particle_batch, q2_beam_batch, IndexedProfile, ParticlePreparationContext,
};
use qa_client::render::scene::particles::SceneParticle;
use qa_client::render::scene::submissions::{
    sequence_draw_group, SceneGroupOrder, SceneOperation, SequencePhase, SourceEntityOrder, SourceSceneOrder,
};
use qa_client::render::scene::view::ViewProjector;
use qa_client::render::types::{AlphaTest, BlendFactor, CullFace, DepthTest, RenderState, RendererImage};
use qa_client::render::{LightProfile, LightShadow, ModelPose, SceneEntity, SceneLight};
use qa_client::view::{ModelTransform, SceneCamera};
use qa_content::contract::{presentation_owner_key, same_presentation_owner, ContentId, GameFamily, PresentationOwner};
use qa_content::q1::foundation::types::{Q1BeamStyle, Q1Effect, Q1Event, Q1Muzzle};
use qa_content::q2::base::player::types::Q2PlayerHand;
use qa_content::q2::base::player::view::add_q2_blend;
use qa_content::q2::foundation::effect_resources::Q2_TRANSIENT_MODELS;
use qa_content::q2::foundation::host::{Q2EffectEvent, Q2PresentationEvent};
use qa_content::q2::foundation::shadow_lights::Q2ShadowLightState;
use qa_content::q2::foundation::weapons::types::{Q2WeaponEvent, WeaponBeamEffect};
use qa_content::q2::missionpacks::entities::types::Q2MissionPackEntityEvent;
use qa_content::q2::missionpacks::types::Q2MissionPackPlayerEffect;
use qa_content::q2::multiplayer::ctf::types::Q2CtfEvent;
use qa_content::q2::multiplayer::lmctf::types::LmctfEvent;
use qa_content::q2::rerelease::types::Q2RereleaseEvent;
use qa_content::q3::foundation::presentation::Q3CharacterView;
use qa_content::q3::presentation::scene::Q3SceneAdmission;
use qa_core::identity::ActorId;
use qa_core::math::{add3, angles_to_axis, length3, normalize3_or_zero, scale3, sub3, vec3, vec4, Vec3, Vec4};
use thiserror::Error;

use super::particles::{ParticleError, Q2ImpactVariant, Q2RespawnKind, Q2TrailKind, SourceParticles};
use super::q2_muzzle::q2_monster_muzzle;
use crate::bootstrap::media::q2_view::{Q2EffectPlayerView, Q2EffectViews};
use crate::bootstrap::media::q3::{
    Q3ApplicationEffects, Q3CharacterPresentationEvent, Q3EffectError, Q3EffectFrameOutput, Q3EffectHost,
    Q3EffectPreload, SourceEffectPlayback,
};
use crate::bootstrap::simulation::q3_ballistics::{Q3BallisticEventKind, Q3SharedBallisticEvent};
use crate::bootstrap::simulation::random::SourceRandom;

pub use crate::bootstrap::media::q3::SourceEffectSound;

/// Zero vector shared by effect origins and directions.
pub const EFFECT_ZERO: Vec3 = Vec3 { x: 0.0, y: 0.0, z: 0.0 };
/// White effect color.
pub const EFFECT_WHITE: Vec3 = Vec3 { x: 1.0, y: 1.0, z: 1.0 };
/// Orange explosion color.
pub const EFFECT_ORANGE: Vec3 = Vec3 { x: 1.0, y: 0.5, z: 0.5 };

/// Quake beam model for a beam style (donor `q1BeamModels`).
#[must_use]
pub fn q1_beam_model(style: Q1BeamStyle) -> &'static str {
    match style {
        Q1BeamStyle::Lightning1 => "progs/bolt.mdl",
        Q1BeamStyle::Lightning2 => "progs/bolt2.mdl",
        Q1BeamStyle::Lightning3 => "progs/bolt3.mdl",
        Q1BeamStyle::Grapple => "progs/beam.mdl",
    }
}

/// Quake II player muzzle-flash light profile (donor `muzzle`).
///
/// The donor adds a `random & 31` radius jitter at push time; callers own
/// that jitter.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MuzzleFlashProfile {
    /// Light color.
    pub color: Vec3,
    /// Base light radius before jitter.
    pub radius: f64,
    /// Light duration in seconds.
    pub duration: f64,
}

/// Resolve a Quake II player muzzle-flash profile (donor `muzzle`).
///
/// Returns `None` for flashes with no source definition (15, 21-29, 40+).
#[must_use]
pub fn muzzle_flash_profile(flash: u32, silenced: bool) -> Option<MuzzleFlashProfile> {
    if !((flash <= 20 && flash != 15) || (30..=39).contains(&flash)) {
        return None;
    }
    let color = match flash {
        6 => Vec3 { x: 0.5, y: 0.5, z: 1.0 },
        7 => Vec3 { x: 1.0, y: 0.5, z: 0.2 },
        8 | 4 | 31 => Vec3 { x: 1.0, y: 0.5, z: 0.0 },
        3 => Vec3 {
            x: 1.0,
            y: 0.25,
            z: 0.0,
        },
        12 | 19 | 34 | 9 => Vec3 { x: 0.0, y: 1.0, z: 0.0 },
        10 | 36 => Vec3 { x: 1.0, y: 0.0, z: 0.0 },
        35 => Vec3 {
            x: -1.0,
            y: -1.0,
            z: -1.0,
        },
        17 | 38 => Vec3 { x: 0.0, y: 0.0, z: 1.0 },
        39 => Vec3 { x: 0.0, y: 1.0, z: 1.0 },
        16 | 18 | 20 => Vec3 { x: 1.0, y: 0.5, z: 0.5 },
        30 => Vec3 { x: 0.9, y: 0.7, z: 0.0 },
        _ => Vec3 { x: 1.0, y: 1.0, z: 0.0 },
    };
    let radius = match flash {
        4 => 225.0,
        5 => 250.0,
        3 => 200.0,
        _ if silenced => 100.0,
        _ => 200.0,
    };
    let duration = if flash == 33 || (36..=39).contains(&flash) {
        0.1
    } else if (9..=11).contains(&flash) {
        0.001
    } else if flash == 4 || flash == 5 {
        0.0001
    } else {
        0.0
    };
    Some(MuzzleFlashProfile {
        color,
        radius,
        duration,
    })
}

/// Explosion animation frame and fractional progress (donor
/// `explosionModel` frame math: 100ms per frame).
#[must_use]
pub fn explosion_frame(start_seconds: f64, now_seconds: f64) -> (u32, f64) {
    let fraction = (now_seconds.mul_add(1000.0, 0.0).round() - start_seconds.mul_add(1000.0, 0.0).round()) / 100.0;
    (fraction.floor().max(0.0) as u32, fraction)
}

/// Whether an explosion with `frames` total frames is still live (donor
/// `prepare` explosion filter).
#[must_use]
pub fn explosion_live(frame: u32, frames: u32) -> bool {
    i64::from(frame) < i64::from(frames) - 1
}

/// Explosion presentation kind (donor `Explosion["kind"]`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExplosionKind {
    /// Polygon explosion with fading alpha and stepped skin.
    Poly,
    /// Miscellaneous fading model.
    Misc,
    /// Constant full-bright flash.
    Flash,
}

/// Explosion model alpha (donor `explosionModel` alpha math).
#[must_use]
pub fn explosion_alpha(kind: ExplosionKind, fraction: f64, frames: u32, frame: u32) -> f64 {
    match kind {
        ExplosionKind::Flash => 1.0,
        ExplosionKind::Misc => {
            if frames < 2 {
                1.0
            } else {
                1.0 - fraction / f64::from(frames - 1)
            }
        }
        ExplosionKind::Poly => (16 - frame.min(16)) as f64 / 16.0,
    }
}

/// Explosion model skin (donor `explosionModel` skin math).
#[must_use]
pub fn explosion_skin(kind: ExplosionKind, frame: u32, skin: u32) -> u32 {
    match kind {
        ExplosionKind::Poly => {
            if frame < 10 {
                frame >> 1
            } else if frame < 13 {
                5
            } else {
                6
            }
        }
        ExplosionKind::Misc | ExplosionKind::Flash => skin,
    }
}

/// WinQuake bonus-flash blend percent (donor `sharedPlayerView`): 50
/// percent, decaying at 100 per second.
#[must_use]
pub fn bonus_flash_percent(until_seconds: f64, now_seconds: f64) -> f64 {
    ((until_seconds - now_seconds) * 100.0).clamp(0.0, 50.0)
}

/// Shadow-light distance fade (donor `shadowSceneLights` fade math).
#[must_use]
pub fn shadow_light_fade(fade_start: f64, fade_end: f64, distance: f64) -> f64 {
    if fade_start <= 1.0 && fade_end <= 1.0 || fade_start > fade_end {
        return 1.0;
    }
    let fraction = (distance / fade_end).clamp(0.0, 1.0);
    let start = fade_start / fade_end;
    if start <= 0.0 {
        fraction
    } else if start < 1.0 {
        let value = ((fraction - start) / (1.0 - start)).clamp(0.0, 1.0);
        1.0 - value * value * (3.0 - 2.0 * value)
    } else if fraction < 1.0 {
        1.0
    } else {
        0.0
    }
}

// ---------------------------------------------------------------------------
// Orchestrator (donor `ApplicationEffects`)
// ---------------------------------------------------------------------------

/// Maximum timed lights (donor `32`).
const MAX_TIMED_LIGHTS: usize = 32;
/// Maximum live steam jets (donor `32`).
const MAX_STEAM_JETS: usize = 32;
/// Maximum Q3 lights per frame (donor `slice(0, 32)`).
const MAX_FRAME_Q3_LIGHTS: usize = 32;
/// Q2 beam batch alpha byte (donor `76.5`, truncated by the `u8` conversion).
const Q2_BEAM_ALPHA: u8 = 76;
/// Q1 light-style table size (donor `256`).
const Q1_STYLE_COUNT: usize = 256;
/// Missing Q1 light-style value (donor `256`).
const Q1_STYLE_MISSING: f64 = 256.0;
/// Q2 transient model paths in donor `Object.values` order.
fn transient_model_paths() -> [&'static str; 8] {
    let models = Q2_TRANSIENT_MODELS;
    [
        models.cable,
        models.parasite,
        models.explosion,
        models.flash,
        models.rocket_explosion,
        models.smoke,
        models.lightning,
        models.bfg_explosion,
    ]
}

/// Application-effects failure (donor `Error` throws).
#[derive(Debug, Error)]
pub enum ApplicationEffectError {
    /// Effect world is closed.
    #[error("Effect world is closed")]
    Closed,
    /// Effect time rewound without replacing its world owner.
    #[error("Effect time rewound without replacing its world owner")]
    TimeRewound,
    /// Q3 ballistic effects require the selected weapon clock.
    #[error("Q3 ballistic effects require the selected weapon clock")]
    WeaponClockMissing,
    /// Q3 weapon effect clock rewound without replacing its world owner.
    #[error("Q3 weapon effect clock rewound without replacing its world owner")]
    WeaponClockRewound,
    /// Indexed effects require their source palette.
    #[error("Indexed effects require their source palette")]
    MissingPalette,
    /// Incomplete effect palette.
    #[error("Incomplete effect palette")]
    IncompletePalette,
    /// Particles have no source texture.
    #[error("Particles have no source texture")]
    MissingParticleTexture,
    /// Beam has no source palette.
    #[error("Beam has no source palette")]
    MissingBeamPalette,
    /// Static brush has no prepared world scene.
    #[error("Static brush {0} has no prepared world scene")]
    MissingBrushScene(String),
    /// Effect content is unknown to the host.
    #[error("Unknown effect content: {0}")]
    UnknownContent(String),
    /// Particle constructor failure.
    #[error(transparent)]
    Particle(#[from] ParticleError),
    /// Q3 sub-effect failure.
    #[error(transparent)]
    Q3(#[from] Q3EffectError),
    /// Effect host failure.
    #[error("Effect host failure: {0}")]
    Host(String),
}

/// Absorbed [`SimulationPresentationEvent`](super::super::simulation::types::SimulationPresentationEvent)
/// pick (`simulation/types.ts` port): the routing envelope plus exactly
/// the payloads this orchestrator reads.
#[derive(Debug, Clone, PartialEq)]
pub struct ApplicationEffectEvent {
    /// Presenting owner, when set.
    pub owner: Option<PresentationOwner>,
    /// Seat-actor recipient, when targeted.
    pub recipient: Option<ActorId>,
    /// Presentation sequence.
    pub sequence: i64,
    /// Content identity.
    pub content: ContentId,
    /// Event time in seconds.
    pub seconds: f64,
    /// Event payload.
    pub kind: ApplicationEffectEventKind,
}

/// Absorbed `SourcePresentationEvent` payloads consumed by the orchestrator.
#[derive(Debug, Clone, PartialEq)]
pub enum ApplicationEffectEventKind {
    /// Presentation-owner retirement or refresh.
    PresentationOwner(PresentationOwnerEvent),
    /// Q1 fog transition (skipped before sequencing).
    Q1Fog,
    /// Ignored sources (`view-reset`, `q2-player`, `q1-level`, `debug-graph`,
    /// and the sky/client/session/music/composition sources the donor falls
    /// through without reading).
    Ignored,
    /// Quake event.
    Q1(Q1Event),
    /// Quake II event.
    Q2(Q2PresentationEvent),
    /// Quake II weapon event.
    Q2Weapon(Q2WeaponEvent),
    /// Quake II composition event.
    Q2Composition(ApplicationCompositionEvent),
    /// Quake II rerelease event.
    Q2Rerelease(Q2RereleaseEvent),
    /// Quake III ballistic event.
    Q3Ballistics(Q3SharedBallisticEvent),
    /// Quake III source event.
    Q3Source(ApplicationQ3SourceEvent),
    /// Quake III character event.
    Q3Character(Q3CharacterPresentationEvent),
}

/// Presentation-owner event (donor `presentation-owner`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PresentationOwnerEvent {
    /// Affected owner.
    pub owner: PresentationOwner,
    /// Retirement or refresh.
    pub kind: PresentationOwnerEventKind,
}

/// Presentation-owner transition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PresentationOwnerEventKind {
    /// Owner retired: purge its pending and staged effects.
    Retired,
    /// Owner refreshed: drop its pending events only.
    Refreshed,
}

/// Canonical `Q2CompositionEvent` (donor
/// `src/content/composition/q2/types.ts`).
/// Only the grapple cable presents here; session kick and grapple-prediction
/// are rejected as having reached the presentation owner.
pub use qa_content::q2::composition::types::Q2CompositionEvent as ApplicationCompositionEvent;

/// Absorbed [`Q3SourceEvent`](super::super::simulation::q3::host::Q3SourceEvent)
/// pick (`simulation/q3/host.ts` port): the sound payload the
/// orchestrator emits plus the entity-event marker it rejects.
#[derive(Debug, Clone, PartialEq)]
pub enum ApplicationQ3SourceEvent {
    /// Positioned source sound.
    Sound {
        /// Sound owner.
        actor: ActorId,
        /// Sound origin.
        origin: Vec3,
        /// Loop velocity.
        velocity: Vec3,
        /// Sound path.
        path: String,
        /// Sound channel.
        channel: i32,
        /// Playback volume.
        volume: f32,
        /// Whether the sound loops.
        looping: bool,
    },
    /// Native entity event (rejected: needs the cgame snapshot context).
    EntityEvent,
    /// Any other source event (prints, commands, configstrings): ignored.
    Other,
}

/// Absorbed `SimulationPresentation` pick: exactly the fields the orchestrator
/// reads.
#[derive(Debug, Clone, PartialEq)]
pub struct EffectPresentation {
    /// Owning actor.
    pub actor: ActorId,
    /// Content identity.
    pub content: ContentId,
    /// Source family.
    pub family: GameFamily,
    /// Model path.
    pub path: String,
    /// Current frame.
    pub frame: i64,
    /// Effects bitmask.
    pub effects: i64,
    /// Origin.
    pub origin: Vec3,
    /// Angles in degrees.
    pub angles: Vec3,
    /// Visibility flag.
    pub visible: bool,
    /// View-weapon flag.
    pub view_weapon: bool,
}

/// Absorbed render-snapshot pick: frame time, Q1 light styles, bodies, actors.
#[derive(Debug, Clone, PartialEq)]
pub struct EffectSnapshot {
    /// Frame time.
    pub time: EffectFrameTime,
    /// Q1 light styles.
    pub light_styles: Vec<EffectLightStyle>,
    /// Live bodies.
    pub bodies: Vec<EffectBody>,
    /// Live actors.
    pub actors: Vec<ActorId>,
}

/// Frame time (donor `WorldSnapshot["frame"]["time"]`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EffectFrameTime {
    /// Time unit.
    pub kind: EffectTimeKind,
    /// Time value.
    pub value: f64,
}

/// Frame time unit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EffectTimeKind {
    /// Seconds.
    Seconds,
    /// Milliseconds.
    Milliseconds,
}

/// Q1 light style (donor `SceneLightStyle` with `kind: "q1"`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EffectLightStyle {
    /// Style slot.
    pub style: i64,
    /// Style value.
    pub value: f64,
}

/// Snapshot body pose (donor `snapshot.bodies[]`).
#[derive(Debug, Clone, PartialEq)]
pub struct EffectBody {
    /// Owning actor.
    pub actor: ActorId,
    /// Body origin.
    pub origin: Vec3,
    /// Body angles.
    pub angles: Vec3,
}

/// Selected weapon clock for Q3 ballistic preparation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EffectWeaponClock {
    /// Weapon content.
    pub content: ContentId,
    /// Weapon time in milliseconds.
    pub time_milliseconds: i32,
}

/// One effect frame (donor `ApplicationEffectFrame`).
#[derive(Debug, Clone, PartialEq)]
pub struct ApplicationEffectFrame {
    /// Q3 scene admissions.
    pub q3_admissions: Vec<Q3SceneAdmission>,
    /// Scene operations in submission order.
    pub operations: Vec<SceneOperation>,
    /// Sampled dynamic lights.
    pub lights: Vec<SurfaceDynamicLight>,
    /// Q3 dynamic lights (at most 32).
    pub q3_lights: Vec<ClientDynamicLight>,
}

/// Rejected presentation event (donor `UnhandledApplicationEffect`).
#[derive(Debug, Clone, PartialEq)]
pub struct UnhandledApplicationEffect {
    /// Rejected source event.
    pub source: ApplicationEffectEvent,
    /// Rejection reason.
    pub reason: String,
}

/// Drained per-recipient sounds (donor `drainRecipientSounds` row).
#[derive(Debug, Clone, PartialEq)]
pub struct RecipientEffectSounds {
    /// Receiving actor.
    pub recipient: ActorId,
    /// Drained sounds.
    pub sounds: Vec<SourceEffectSound>,
}

/// Transient-resource preload failure (donor `preloadTransientResources` row).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EffectPreloadFailure {
    /// Failing content.
    pub content: ContentId,
    /// Failing path or media label.
    pub path: String,
    /// Failure message.
    pub error: String,
}

/// Absorbed content-recipe pick for transient preloads (donor
/// `ApplicationAssets["content"]["recipe"]` plus the equipment/enemy extras).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EffectPreloadPlan {
    /// Character content.
    pub character: ContentId,
    /// Map-entities content.
    pub map_entities: ContentId,
    /// Weapon contents.
    pub weapons: Vec<ContentId>,
    /// Equipment/enemy extra contents.
    pub extras: Vec<ContentId>,
}

/// Loaded effect model (donor `ApplicationAssets.model` pick).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EffectModelAsset<B> {
    /// Content model handle.
    pub handle: u32,
    /// Wire entity number.
    pub entity_number: u32,
    /// Quake model flags (`q1-mdl` flags or `md5` replacement flags).
    pub q1_flags: i64,
    /// Brush scene for brush models.
    pub brush: Option<EffectBrushModel<B>>,
}

/// Static brush scene (donor `asset.brushScene` plus brush-model index).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EffectBrushModel<B> {
    /// Prepared world scene.
    pub scene: B,
    /// Brush-model index.
    pub model: i32,
}

/// Model-view input for staged-model preparation (donor `renderer.prepare`
/// options; the `{ kind: "preview", id: "effects" }` target is constant).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EffectModelView<'a> {
    /// View camera.
    pub camera: &'a SceneCamera,
    /// Effect time in seconds.
    pub time_seconds: f64,
    /// Sampled dynamic lights.
    pub lights: &'a [SurfaceDynamicLight],
}

/// Brush-view input for static-brush preparation (donor
/// `WorldScene.prepareModel` options).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EffectBrushView<'a> {
    /// View camera.
    pub camera: &'a SceneCamera,
    /// Effect time in seconds.
    pub time_seconds: f64,
    /// Sampled dynamic lights.
    pub lights: &'a [SurfaceDynamicLight],
    /// Q1 light-style table.
    pub q1_styles: &'a [f64; Q1_STYLE_COUNT],
    /// Q1 fog override.
    pub q1_fog: Option<Q1FogInput>,
    /// Animation frame.
    pub frame: i64,
}

/// Q3 sub-effect creation (sync mapping of the donor's
/// `Q3ApplicationEffects.create` await).
pub type Q3EffectsFactory<Q> =
    Rc<dyn Fn(&ContentId, Option<Q3EffectPreload>, Q3Hardware) -> Result<Q, ApplicationEffectError>>;

/// Q3 sub-effects consumed by the orchestrator (donor `Q3ApplicationEffects`
/// surface used by `effects.ts`).
pub trait ApplicationQ3Effects {
    /// Present one shared ballistic event.
    fn ballistic(&mut self, event: &Q3SharedBallisticEvent) -> Result<(), ApplicationEffectError>;
    /// Present one character event; `false` needs the full cgame payload.
    fn character_event(
        &mut self,
        event: &Q3CharacterPresentationEvent,
        origin: Vec3,
    ) -> Result<bool, ApplicationEffectError>;
    /// Advance to `now_milliseconds`.
    fn prepare(&mut self, now_milliseconds: i32, elapsed_milliseconds: i32) -> Result<(), ApplicationEffectError>;
    /// Build one Q3 effect frame.
    fn frame(
        &mut self,
        camera: &SceneCamera,
        source: &SourceSceneOrder,
        viewer: Option<&ActorId>,
        q1_fog: Option<Q1FogInput>,
    ) -> Result<Q3EffectFrameOutput, ApplicationEffectError>;
    /// Drain queued sounds.
    fn drain_sounds(&mut self) -> Vec<SourceEffectSound>;
    /// Reset round state.
    fn reset_round(&mut self);
    /// Release Q3 effects.
    fn close(&mut self);
}

impl<Q: Q3EffectHost + 'static> ApplicationQ3Effects for Q3ApplicationEffects<Q> {
    fn ballistic(&mut self, event: &Q3SharedBallisticEvent) -> Result<(), ApplicationEffectError> {
        Ok(self.ballistic(event)?)
    }

    fn character_event(
        &mut self,
        event: &Q3CharacterPresentationEvent,
        origin: Vec3,
    ) -> Result<bool, ApplicationEffectError> {
        Ok(self.event(event, origin)?)
    }

    fn prepare(&mut self, now_milliseconds: i32, elapsed_milliseconds: i32) -> Result<(), ApplicationEffectError> {
        Ok(self.prepare(now_milliseconds, elapsed_milliseconds)?)
    }

    fn frame(
        &mut self,
        camera: &SceneCamera,
        source: &SourceSceneOrder,
        viewer: Option<&ActorId>,
        q1_fog: Option<Q1FogInput>,
    ) -> Result<Q3EffectFrameOutput, ApplicationEffectError> {
        Ok(self.frame(camera, source, viewer, q1_fog)?)
    }

    fn drain_sounds(&mut self) -> Vec<SourceEffectSound> {
        self.drain_sounds()
    }

    fn reset_round(&mut self) {
        self.reset_round();
    }

    fn close(&mut self) {
        self.close();
    }
}

/// Absorbed [`ApplicationAssets`](super::super::assets::ApplicationAssets) plus `SceneQueries`
/// ([`ApplicationQ3SceneQueries`](super::super::q3_client::visibility::ApplicationQ3SceneQueries)) pick:
/// provider families, palettes, model loading/preparation, and the
/// content-recipe plan resolve against the asset cache; the player closure,
/// particle registry, staged-model preparation, and renderer caches stay
/// host-side (donor `preparedRenderers`). Synchronous mapping of the donor's
/// awaited asset calls.
pub trait ApplicationEffectHost: Clone + 'static {
    /// Prepared brush world scene.
    type BrushScene: Clone;

    /// Whether an actor is a player (donor `isPlayer` closure).
    fn is_player(&self, actor: &ActorId) -> bool;
    /// Provider family for content, or `None` when unknown.
    fn provider_family(&self, content: &ContentId) -> Option<GameFamily>;
    /// Whether content is the rerelease edition.
    fn is_rerelease(&self, content: &ContentId) -> bool;
    /// Source palette triples (0-255), or `None` when absent.
    fn palette(&self, content: &ContentId) -> Option<Vec<f32>>;
    /// White texture image for beam batches.
    fn white_image(&self, content: &ContentId) -> RendererImage;
    /// Register the source-particle image for a family.
    fn register_particle_image(&mut self, family: GameFamily) -> RendererImage;
    /// Release a source-particle image.
    fn release_particle_image(&mut self, image: RendererImage);
    /// Whether a content mount holds a path.
    fn mount_has(&self, content: &ContentId, path: &str) -> bool;
    /// Load a content model.
    fn load_model(
        &mut self,
        content: &ContentId,
        path: &str,
    ) -> Result<EffectModelAsset<Self::BrushScene>, ApplicationEffectError>;
    /// Preload one model path (donor `preloadModel`).
    fn preload_model(&mut self, content: &ContentId, path: &str) -> Result<(), ApplicationEffectError>;
    /// Transient-preload recipe plan.
    fn preload_plan(&self) -> EffectPreloadPlan;
    /// Map donor model flags (`kind`/`bits`) to `RF_*` bits.
    fn scene_flags(&self, family: GameFamily, bits: i64) -> u32;
    /// Warm staged models.
    fn preload_models(&mut self, content: &ContentId, models: &[SceneEntity]) -> Result<(), ApplicationEffectError>;
    /// Prepare staged models for one frame.
    fn prepare_models(
        &self,
        content: &ContentId,
        models: &[SceneEntity],
        view: &EffectModelView<'_>,
    ) -> Vec<SceneOperation>;
    /// Prepare one static brush for one frame.
    fn prepare_brush(
        &self,
        brush: &EffectBrushModel<Self::BrushScene>,
        transform: &ModelTransform,
        view: &EffectBrushView<'_>,
    ) -> Vec<SceneOperation>;
}
/// Per-content effect group (donor `Group`; provider and renderer live
/// host-side).
struct EffectGroup {
    family: GameFamily,
    palette: Option<Vec<f32>>,
    particles: SourceParticles,
    models: Vec<SceneEntity>,
    statics: Vec<StaticModelEntry>,
    beams: Vec<BeamModelEntry>,
    sampled: Vec<SceneParticle>,
}

/// Static staged model with its presenting owner.
#[derive(Debug, Clone)]
struct StaticModelEntry {
    owner: Option<PresentationOwner>,
    entity: SceneEntity,
}

/// Staged beam models with the first-person variant (donor `Group["beams"]`).
#[derive(Debug, Clone)]
struct BeamModelEntry {
    actor: Option<ActorId>,
    remote: Vec<SceneEntity>,
    local: Option<Vec<SceneEntity>>,
}

/// Timed dynamic light (donor `TimedLight`).
#[derive(Debug, Clone)]
struct TimedLight {
    origin: Vec3,
    radius: f32,
    minimum: f32,
    color: Vec3,
    born: f64,
    die: f64,
    decay: f32,
    actor: Option<ActorId>,
}

/// Live beam (donor `Beam`).
#[derive(Debug, Clone)]
struct EffectBeam {
    owner: Option<PresentationOwner>,
    content: ContentId,
    start: Vec3,
    end: Vec3,
    die: f64,
    width: f32,
    color: i32,
    model: Option<String>,
    family: GameFamily,
    actor: Option<ActorId>,
}

/// Live explosion (donor `Explosion`).
#[derive(Debug, Clone)]
struct EffectExplosion {
    content: ContentId,
    origin: Vec3,
    angles: Vec3,
    start: f64,
    frames: i64,
    base_frame: i64,
    path: String,
    kind: ExplosionKind,
    flags: i64,
    skin: i64,
    light: Option<ExplosionLight>,
}

/// Explosion light (donor `Explosion["light"]`).
#[derive(Debug, Clone, Copy)]
struct ExplosionLight {
    radius: f32,
    color: Vec3,
}

/// Live steam jet (donor `Steam`).
#[derive(Debug, Clone)]
struct EffectSteam {
    content: ContentId,
    event: Q2MissionPackEntityEvent,
    end: f64,
    next: f64,
}

/// Static brush (donor `staticBrushes` row).
#[derive(Debug, Clone)]
struct StaticBrush<B> {
    owner: Option<PresentationOwner>,
    brush: EffectBrushModel<B>,
    transform: ModelTransform,
    frame: i64,
}

/// Flashlight state (donor `flashlights` row).
#[derive(Debug, Clone)]
struct FlashlightState {
    actor: ActorId,
    hand: Q2PlayerHand,
    owner: Option<PresentationOwner>,
}

/// Source dynamic light with its owner (donor `sourceLights` row).
#[derive(Debug, Clone)]
struct SourceLightEntry {
    light: SurfaceDynamicLight,
    owner: Option<PresentationOwner>,
}

/// Shadow light with its owner (donor `shadowLights` row).
#[derive(Debug, Clone)]
struct ShadowLightEntry {
    state: Q2ShadowLightState,
    owner: Option<PresentationOwner>,
}

/// Tracker-pain overlay (donor `trackerPain` row).
#[derive(Debug, Clone)]
struct TrackerPain {
    content: ContentId,
    until: f64,
}

/// Captured actor pose (donor `poses` row).
#[derive(Debug, Clone)]
struct EffectPose {
    actor: ActorId,
    origin: Vec3,
    angles: Vec3,
}

/// Quake II entity trail head (donor `entityTrails` row).
#[derive(Debug, Clone)]
struct EntityTrail {
    content: ContentId,
    origin: Vec3,
    count: i32,
}

/// Quake entity trail head (donor `q1Trails` row).
#[derive(Debug, Clone)]
struct Q1Trail {
    content: ContentId,
    path: String,
    origin: Vec3,
}

/// Whether an operation is a world-entity scene group (donor `polygon`).
fn is_polygon_operation(operation: &SceneOperation) -> bool {
    matches!(
        operation,
        SceneOperation::Group(group)
            if matches!(&group.order, SceneGroupOrder::Source { source, .. } if source.entity == SourceEntityOrder::World)
    )
}

/// Quake/Quake II/Quake III transient-effect orchestrator (donor
/// `ApplicationEffects`). A world owns one event stream: preparation advances
/// it once; seat frames only sample it.
pub struct ApplicationEffects<H: ApplicationEffectHost, Q: ApplicationQ3Effects> {
    host: H,
    q3_factory: Q3EffectsFactory<Q>,
    seed: u32,
    random: SourceRandom,
    recipients: HashMap<ActorId, ApplicationEffects<H, Q>>,
    events_only: bool,
    groups: HashMap<ContentId, EffectGroup>,
    images: HashMap<GameFamily, RendererImage>,
    q3: HashMap<ContentId, Q>,
    q3_weapons: HashMap<ContentId, Q>,
    prepared_q3_weapons: HashMap<ContentId, Q>,
    q3_weapon_times: HashMap<ContentId, i32>,
    entity_trails: HashMap<ActorId, EntityTrail>,
    q1_trails: HashMap<ActorId, Q1Trail>,
    pending: Vec<ApplicationEffectEvent>,
    retired_owners: HashSet<String>,
    owner_revisions: HashMap<String, i64>,
    unhandled: Vec<UnhandledApplicationEffect>,
    beams: Vec<EffectBeam>,
    explosions: Vec<EffectExplosion>,
    static_brushes: Vec<StaticBrush<H::BrushScene>>,
    styles: Vec<EffectLightStyle>,
    lights: Vec<TimedLight>,
    sampled_lights: Vec<SurfaceDynamicLight>,
    flashlights: HashMap<ActorId, FlashlightState>,
    shadow_lights: HashMap<ActorId, ShadowLightEntry>,
    source_lights: HashMap<ActorId, SourceLightEntry>,
    player_views: Q2EffectViews,
    bonus_flashes: HashMap<ActorId, f64>,
    tracker_pain: HashMap<ActorId, TrackerPain>,
    steam: Vec<EffectSteam>,
    sounds: Vec<SourceEffectSound>,
    poses: Vec<EffectPose>,
    time: Option<f64>,
    sequence: i64,
    closed: bool,
    read_hardware: Rc<dyn Fn() -> Q3Hardware>,
}

impl<H: ApplicationEffectHost, Q: ApplicationQ3Effects> ApplicationEffects<H, Q> {
    /// Create effect state over a host, seed, and Q3 factory (donor
    /// constructor; `isPlayer` arrives via the host).
    pub fn new(host: H, seed: u32, q3_factory: Q3EffectsFactory<Q>) -> Self {
        Self {
            host,
            q3_factory,
            seed,
            random: SourceRandom::new(seed),
            recipients: HashMap::new(),
            events_only: false,
            groups: HashMap::new(),
            images: HashMap::new(),
            q3: HashMap::new(),
            q3_weapons: HashMap::new(),
            prepared_q3_weapons: HashMap::new(),
            q3_weapon_times: HashMap::new(),
            entity_trails: HashMap::new(),
            q1_trails: HashMap::new(),
            pending: Vec::new(),
            retired_owners: HashSet::new(),
            owner_revisions: HashMap::new(),
            unhandled: Vec::new(),
            beams: Vec::new(),
            explosions: Vec::new(),
            static_brushes: Vec::new(),
            styles: Vec::new(),
            lights: Vec::new(),
            sampled_lights: Vec::new(),
            flashlights: HashMap::new(),
            shadow_lights: HashMap::new(),
            source_lights: HashMap::new(),
            player_views: Q2EffectViews::new(),
            bonus_flashes: HashMap::new(),
            tracker_pain: HashMap::new(),
            steam: Vec::new(),
            sounds: Vec::new(),
            poses: Vec::new(),
            time: None,
            sequence: -1,
            closed: false,
            read_hardware: Rc::new(|| Q3Hardware::Generic),
        }
    }

    /// Bind the renderer-hardware reader (donor `bindRendererHardware`).
    pub fn bind_renderer_hardware(&mut self, read: Rc<dyn Fn() -> Q3Hardware>) {
        self.read_hardware = read;
    }

    /// Receive presentation events (donor `receive`).
    pub fn receive(&mut self, events: &[ApplicationEffectEvent]) -> Result<(), ApplicationEffectError> {
        if self.closed {
            return Err(ApplicationEffectError::Closed);
        }
        for event in events {
            if let ApplicationEffectEventKind::PresentationOwner(owner) = &event.kind {
                let key = presentation_owner_key(Some(&owner.owner));
                if owner.kind == PresentationOwnerEventKind::Retired {
                    self.retired_owners.insert(key.clone());
                }
                self.owner_revisions.insert(key.clone(), event.sequence);
                self.pending
                    .retain(|source| !same_presentation_owner(source.owner.as_ref(), &owner.owner));
                for effects in self.recipients.values_mut() {
                    effects.receive(std::slice::from_ref(event))?;
                }
                if owner.kind == PresentationOwnerEventKind::Refreshed {
                    continue;
                }
                self.beams
                    .retain(|beam| !same_presentation_owner(beam.owner.as_ref(), &owner.owner));
                self.flashlights
                    .retain(|_, light| !same_presentation_owner(light.owner.as_ref(), &owner.owner));
                self.shadow_lights
                    .retain(|_, light| !same_presentation_owner(light.owner.as_ref(), &owner.owner));
                self.source_lights
                    .retain(|_, light| !same_presentation_owner(light.owner.as_ref(), &owner.owner));
                self.static_brushes
                    .retain(|brush| !same_presentation_owner(brush.owner.as_ref(), &owner.owner));
                for group in self.groups.values_mut() {
                    group
                        .statics
                        .retain(|entry| !same_presentation_owner(entry.owner.as_ref(), &owner.owner));
                }
                continue;
            }
            if let Some(owner) = &event.owner {
                if self.retired_owners.contains(&presentation_owner_key(Some(owner))) {
                    continue;
                }
            }
            if let Some(recipient) = &event.recipient {
                let mut shared = event.clone();
                shared.recipient = None;
                let effects = self.recipient(recipient);
                effects.receive(std::slice::from_ref(&shared))?;
                continue;
            }
            if matches!(event.kind, ApplicationEffectEventKind::Q1Fog) {
                continue;
            }
            if event.sequence <= self.sequence {
                continue;
            }
            self.sequence = event.sequence;
            self.pending.push(event.clone());
        }
        Ok(())
    }

    /// Fetch or create the events-only recipient world for an actor.
    fn recipient(&mut self, recipient: &ActorId) -> &mut ApplicationEffects<H, Q> {
        if !self.recipients.contains_key(recipient) {
            let mut effects = ApplicationEffects::new(self.host.clone(), self.seed, Rc::clone(&self.q3_factory));
            effects.events_only = true;
            effects.read_hardware = Rc::clone(&self.read_hardware);
            self.recipients.insert(recipient.clone(), effects);
        }
        self.recipients.get_mut(recipient).expect("recipient inserted above")
    }

    /// Drain unhandled effects, tagging recipient entries (donor
    /// `drainUnhandled`).
    pub fn drain_unhandled(&mut self) -> Vec<UnhandledApplicationEffect> {
        let mut result = std::mem::take(&mut self.unhandled);
        for (recipient, effects) in &mut self.recipients {
            for mut entry in effects.drain_unhandled() {
                entry.source.recipient = Some(recipient.clone());
                result.push(entry);
            }
        }
        result
    }

    /// Drain per-recipient sounds (donor `drainRecipientSounds`).
    pub fn drain_recipient_sounds(&mut self) -> Vec<RecipientEffectSounds> {
        let mut result = Vec::new();
        for (recipient, effects) in &mut self.recipients {
            let sounds = effects.drain_sounds();
            if !sounds.is_empty() {
                result.push(RecipientEffectSounds {
                    recipient: recipient.clone(),
                    sounds,
                });
            }
        }
        result
    }

    /// Drain queued sounds: own first, then Q3 (donor `drainSounds`).
    pub fn drain_sounds(&mut self) -> Vec<SourceEffectSound> {
        let mut result = std::mem::take(&mut self.sounds);
        for effects in self.q3.values_mut().chain(self.q3_weapons.values_mut()) {
            result.extend(effects.drain_sounds());
        }
        result
    }

    /// Present one player's camera (donor `playerView`).
    pub fn player_view(&self, actor: &ActorId, camera: &SceneCamera) -> Q2EffectPlayerView {
        let shared = self.shared_player_view(actor, camera);
        let Some(effects) = self.recipients.get(actor) else {
            return shared;
        };
        let local = effects.player_view(actor, &shared.camera);
        Q2EffectPlayerView {
            camera: local.camera,
            infrared: shared.infrared || local.infrared,
            blend: match local.blend {
                None => shared.blend,
                Some(blend) => Some(add_q2_blend(
                    shared.blend.unwrap_or(Vec4 {
                        x: 0.0,
                        y: 0.0,
                        z: 0.0,
                        w: 0.0,
                    }),
                    vec3(blend.x, blend.y, blend.z),
                    f64::from(blend.w),
                )),
            },
        }
    }

    /// Present one player's camera without recipient overrides (donor
    /// `sharedPlayerView`).
    fn shared_player_view(&self, actor: &ActorId, camera: &SceneCamera) -> Q2EffectPlayerView {
        let seconds = self.time.unwrap_or(0.0);
        let view = self.player_views.frame(actor, camera, seconds, &|sphere| {
            self.pose(sphere).map(|pose| pose.origin)
        });
        let Some(until) = self.bonus_flashes.get(actor) else {
            return view;
        };
        let percent = bonus_flash_percent(*until, seconds);
        if percent == 0.0 {
            return view;
        }
        Q2EffectPlayerView {
            blend: Some(add_q2_blend(
                view.blend.unwrap_or(Vec4 {
                    x: 0.0,
                    y: 0.0,
                    z: 0.0,
                    w: 0.0,
                }),
                vec3(215.0 / 255.0, 186.0 / 255.0, 69.0 / 255.0),
                percent / 255.0,
            )),
            ..view
        }
    }

    /// Record an unhandled effect (donor `reject`).
    fn reject(&mut self, source: &ApplicationEffectEvent, reason: String) {
        self.unhandled.push(UnhandledApplicationEffect {
            source: source.clone(),
            reason,
        });
    }

    /// Resolve a captured actor pose (donor `pose`).
    fn pose(&self, actor: &ActorId) -> Option<&EffectPose> {
        self.poses.iter().find(|pose| &pose.actor == actor)
    }

    /// Fetch or create the per-content group (donor `group`, sync).
    fn group(&mut self, content: &ContentId) -> Result<&mut EffectGroup, ApplicationEffectError> {
        if !self.groups.contains_key(content) {
            let family = self
                .host
                .provider_family(content)
                .ok_or_else(|| ApplicationEffectError::UnknownContent(content.as_str().to_string()))?;
            let palette = self.host.palette(content);
            self.groups.insert(
                content.clone(),
                EffectGroup {
                    family,
                    palette,
                    particles: SourceParticles::new(SourceRandom::new(self.seed)),
                    models: Vec::new(),
                    statics: Vec::new(),
                    beams: Vec::new(),
                    sampled: Vec::new(),
                },
            );
            if family != GameFamily::Q3 && !self.images.contains_key(&family) {
                let image = self.host.register_particle_image(family);
                self.images.insert(family, image);
            }
        }
        self.groups
            .get_mut(content)
            .ok_or_else(|| ApplicationEffectError::UnknownContent(content.as_str().to_string()))
    }

    /// Resolve a provider family or fail unknown (donor `catalog.product`
    /// throw).
    fn family_or_unknown(&self, content: &ContentId) -> Result<GameFamily, ApplicationEffectError> {
        self.host
            .provider_family(content)
            .ok_or_else(|| ApplicationEffectError::UnknownContent(content.as_str().to_string()))
    }

    /// Run a particle constructor with the shared random stream. The donor
    /// shares one `SourceRandom` across the orchestrator and every group; the
    /// swap preserves that single stream across the owned-split port (group
    /// placeholder streams are never observed).
    fn with_particles<R>(
        &mut self,
        content: &ContentId,
        run: impl FnOnce(&mut SourceParticles) -> R,
    ) -> Result<R, ApplicationEffectError> {
        self.group(content)?;
        {
            let group = self
                .groups
                .get_mut(content)
                .ok_or_else(|| ApplicationEffectError::UnknownContent(content.as_str().to_string()))?;
            std::mem::swap(&mut self.random, group.particles.random_mut());
        }
        let output = {
            let group = self
                .groups
                .get_mut(content)
                .ok_or_else(|| ApplicationEffectError::UnknownContent(content.as_str().to_string()))?;
            run(&mut group.particles)
        };
        {
            let group = self
                .groups
                .get_mut(content)
                .ok_or_else(|| ApplicationEffectError::UnknownContent(content.as_str().to_string()))?;
            std::mem::swap(&mut self.random, group.particles.random_mut());
        }
        Ok(output)
    }

    /// Preload transient resources (donor `preloadTransientResources`, sync).
    pub fn preload_transient_resources(&mut self) -> Result<Vec<EffectPreloadFailure>, ApplicationEffectError> {
        let plan = self.host.preload_plan();
        let mut failures = Vec::new();
        if plan.character != plan.map_entities
            && self.family_or_unknown(&plan.character)? == GameFamily::Q3
            && !self.q3.contains_key(&plan.character)
        {
            match (self.q3_factory)(
                &plan.character,
                Some(Q3EffectPreload::Character),
                (self.read_hardware)(),
            ) {
                Ok(effects) => {
                    self.q3.insert(plan.character.clone(), effects);
                }
                Err(error) => failures.push(EffectPreloadFailure {
                    content: plan.character.clone(),
                    path: "Q3 character effect media".to_string(),
                    error: error.to_string(),
                }),
            }
        }
        let mut seen_weapons = HashSet::new();
        for content in &plan.weapons {
            if !seen_weapons.insert(content.clone()) {
                continue;
            }
            if *content == plan.map_entities
                || self.family_or_unknown(content)? != GameFamily::Q3
                || self.prepared_q3_weapons.contains_key(content)
                || self.q3_weapons.contains_key(content)
            {
                continue;
            }
            match (self.q3_factory)(content, Some(Q3EffectPreload::Weapons), (self.read_hardware)()) {
                Ok(effects) => {
                    self.prepared_q3_weapons.insert(content.clone(), effects);
                }
                Err(error) => failures.push(EffectPreloadFailure {
                    content: content.clone(),
                    path: "Q3 effect media".to_string(),
                    error: error.to_string(),
                }),
            }
        }
        let mut contents = vec![plan.map_entities.clone(), plan.character.clone()];
        contents.extend(plan.weapons.iter().cloned());
        contents.extend(plan.extras.iter().cloned());
        let mut seen_contents = HashSet::new();
        for content in &contents {
            if !seen_contents.insert(content.clone()) {
                continue;
            }
            if self.family_or_unknown(content)? != GameFamily::Q2 {
                continue;
            }
            for path in transient_model_paths() {
                if !self.host.mount_has(content, path) {
                    continue;
                }
                if let Err(error) = self.host.preload_model(content, path) {
                    failures.push(EffectPreloadFailure {
                        content: content.clone(),
                        path: path.to_string(),
                        error: error.to_string(),
                    });
                }
            }
        }
        Ok(failures)
    }

    /// Preload one model path (donor `preloadModel`, sync).
    pub fn preload_model(&mut self, content: &ContentId, path: &str) -> Result<(), ApplicationEffectError> {
        self.host.preload_model(content, path)
    }

    /// Push a timed light (donor `light`).
    #[allow(clippy::too_many_arguments)]
    fn light(
        &mut self,
        origin: Vec3,
        seconds: f64,
        radius: f32,
        duration: f64,
        color: Vec3,
        decay: f32,
        minimum: f32,
        actor: Option<ActorId>,
    ) {
        if let Some(actor) = &actor {
            if let Some(prior) = self.lights.iter().position(|light| light.actor.as_ref() == Some(actor)) {
                self.lights.remove(prior);
            }
        }
        if self.lights.len() >= MAX_TIMED_LIGHTS {
            self.lights.remove(0);
        }
        self.lights.push(TimedLight {
            origin,
            radius,
            minimum,
            color,
            born: seconds,
            die: seconds + duration,
            decay,
            actor,
        });
    }

    /// Push a beam, replacing the same actor/family/model beam (donor `beam`).
    fn push_beam(&mut self, beam: EffectBeam) {
        if let Some(actor) = &beam.actor {
            if let Some(old) = self.beams.iter().position(|value| {
                value.actor.as_ref() == Some(actor) && value.family == beam.family && value.model == beam.model
            }) {
                self.beams.remove(old);
            }
        }
        self.beams.push(beam);
    }

    /// Push a one-shot effect sound (donor Q1/Q2 `sound` closures).
    fn push_sound(&mut self, content: &ContentId, path: &str, origin: Vec3, seconds: f64) {
        self.sounds.push(SourceEffectSound {
            content: content.clone(),
            path: path.to_string(),
            origin,
            channel: 0,
            volume: 1.0,
            seconds: seconds as f32,
            playback: SourceEffectPlayback::Once,
        });
    }

    /// Push an entity-driven sampled light (donor `q2Entities` `light`).
    fn push_sampled(&mut self, origin: Vec3, radius: f32, color: Vec3) {
        self.sampled_lights.push(SurfaceDynamicLight {
            origin,
            radius,
            color,
            minimum: 0.0,
        });
    }
    /// Advance the event stream and stage one frame (donor `prepare`, sync).
    pub fn prepare(
        &mut self,
        snapshot: &EffectSnapshot,
        presentations: &[EffectPresentation],
        characters: &[Q3CharacterView],
        weapon_clock: Option<&EffectWeaponClock>,
    ) -> Result<(), ApplicationEffectError> {
        if self.closed {
            return Err(ApplicationEffectError::Closed);
        }
        let now = match snapshot.time.kind {
            EffectTimeKind::Seconds => snapshot.time.value,
            EffectTimeKind::Milliseconds => snapshot.time.value / 1000.0,
        };
        if let Some(time) = self.time {
            if now < time {
                return Err(ApplicationEffectError::TimeRewound);
            }
        }
        let elapsed = self.time.map_or(0.0, |time| now - time);
        let advance = elapsed > 0.0 || self.time.is_none();
        self.styles.clone_from(&snapshot.light_styles);
        self.poses = characters
            .iter()
            .map(|character| EffectPose {
                actor: character.actor.clone(),
                origin: character.origin,
                angles: character.angles,
            })
            .chain(
                presentations
                    .iter()
                    .filter(|pose| !pose.view_weapon)
                    .map(|pose| EffectPose {
                        actor: pose.actor.clone(),
                        origin: pose.origin,
                        angles: pose.angles,
                    }),
            )
            .chain(snapshot.bodies.iter().map(|body| EffectPose {
                actor: body.actor.clone(),
                origin: body.origin,
                angles: body.angles,
            }))
            .collect();
        for group in self.groups.values_mut() {
            group.models.clear();
            group.beams.clear();
        }
        let pending = std::mem::take(&mut self.pending);
        for source in &pending {
            self.event(source)?;
        }
        let live_actors: HashSet<ActorId> = snapshot.actors.iter().cloned().collect();
        self.flashlights.retain(|actor, _| live_actors.contains(actor));
        self.shadow_lights.retain(|actor, _| live_actors.contains(actor));
        self.source_lights.retain(|actor, _| live_actors.contains(actor));
        self.tracker_pain.retain(|actor, _| live_actors.contains(actor));
        self.player_views.retain(&live_actors);
        self.bonus_flashes
            .retain(|actor, until| live_actors.contains(actor) && *until > now);
        self.steam.retain(|steam| steam.end >= now);
        if advance {
            let due: Vec<(ContentId, Q2MissionPackEntityEvent, usize)> = self
                .steam
                .iter()
                .enumerate()
                .filter(|(_, jet)| jet.next <= now)
                .map(|(index, jet)| (jet.content.clone(), jet.event.clone(), index))
                .collect();
            for (content, event, index) in due {
                if let Q2MissionPackEntityEvent::Steam {
                    origin,
                    direction,
                    count,
                    color,
                    speed,
                    ..
                } = event
                {
                    let advanced = self.with_particles(&content, |particles| {
                        particles.q2_steam(
                            origin,
                            direction,
                            color as u8,
                            count.max(0) as u32,
                            speed as f32,
                            now as f32,
                            false,
                        )
                    })?;
                    if advanced {
                        self.steam[index].next += 0.1;
                    }
                }
            }
        }
        self.beams.retain(|beam| beam.die >= now);
        self.explosions.retain(|explosion| {
            let (frame, _) = explosion_frame(explosion.start, now);
            explosion_live(frame, explosion.frames.max(0) as u32)
        });
        self.lights
            .retain(|light| light.die >= now && light.radius - light.decay * (now - light.born) as f32 > 0.0);
        if !self.events_only {
            self.q1_entities(presentations, now, advance)?;
        }
        self.sampled_lights = self
            .lights
            .iter()
            .map(|light| SurfaceDynamicLight {
                origin: light.origin,
                radius: (light.radius - light.decay * (now - light.born) as f32).max(0.0),
                minimum: light.minimum,
                color: light.color,
            })
            .collect();
        self.sampled_lights
            .extend(self.source_lights.values().map(|entry| entry.light));
        if !self.events_only {
            self.q2_entities(presentations, now, advance)?;
        }
        let tracker: Vec<(ActorId, TrackerPain)> = self
            .tracker_pain
            .iter()
            .map(|(actor, effect)| (actor.clone(), effect.clone()))
            .collect();
        for (actor, effect) in tracker {
            if effect.until <= now {
                self.tracker_pain.remove(&actor);
                continue;
            }
            if !self.events_only
                && presentations.iter().any(|entity| {
                    entity.family == GameFamily::Q2 && entity.actor == actor && entity.visible && !entity.view_weapon
                })
            {
                continue;
            }
            let pose = self.pose(&actor).cloned();
            if let Some(pose) = pose {
                if advance {
                    self.with_particles(&effect.content, |particles| {
                        particles.q2_tracker_shell(pose.origin, now as f32);
                    })?;
                }
                self.sampled_lights.push(SurfaceDynamicLight {
                    origin: pose.origin,
                    radius: 155.0,
                    minimum: 0.0,
                    color: vec3(-1.0, -1.0, -1.0),
                });
            }
        }
        let beams = self.beams.clone();
        for beam in &beams {
            if beam.model.is_some() {
                self.beam_models(beam)?;
            }
        }
        let explosions = self.explosions.clone();
        for explosion in &explosions {
            self.explosion_model(explosion, now)?;
        }
        let contents: Vec<ContentId> = self.groups.keys().cloned().collect();
        for content in &contents {
            let preload = {
                let group = self
                    .groups
                    .get_mut(content)
                    .ok_or_else(|| ApplicationEffectError::UnknownContent(content.as_str().to_string()))?;
                let samples = group.particles.sample(now as f32, elapsed as f32);
                group.sampled = if group.family == GameFamily::Q1 {
                    samples.q1
                } else {
                    samples.q2
                };
                group
                    .statics
                    .iter()
                    .map(|entry| entry.entity)
                    .chain(group.models.iter().copied())
                    .chain(group.beams.iter().flat_map(|beam| {
                        beam.remote
                            .iter()
                            .copied()
                            .chain(beam.local.iter().flat_map(|local| local.iter().copied()))
                    }))
                    .collect::<Vec<SceneEntity>>()
            };
            self.host.preload_models(content, &preload)?;
        }
        for effects in self.q3.values_mut() {
            effects.prepare((now * 1000.0).trunc() as i32, (elapsed * 1000.0).trunc() as i32)?;
        }
        let weapon_contents: Vec<ContentId> = self.q3_weapons.keys().cloned().collect();
        for content in &weapon_contents {
            let Some(clock) = weapon_clock else {
                return Err(ApplicationEffectError::WeaponClockMissing);
            };
            if clock.content != *content {
                return Err(ApplicationEffectError::WeaponClockMissing);
            }
            let current = clock.time_milliseconds;
            let previous = self.q3_weapon_times.get(content).copied().unwrap_or(current);
            if current < previous {
                return Err(ApplicationEffectError::WeaponClockRewound);
            }
            let effects = self
                .q3_weapons
                .get_mut(content)
                .ok_or_else(|| ApplicationEffectError::UnknownContent(content.as_str().to_string()))?;
            effects.prepare(current, current - previous)?;
            self.q3_weapon_times.insert(content.clone(), current);
        }
        self.time = Some(now);
        let recipients: Vec<ActorId> = self.recipients.keys().cloned().collect();
        for recipient in &recipients {
            if !live_actors.contains(recipient) {
                if let Some(mut effects) = self.recipients.remove(recipient) {
                    effects.close();
                }
            } else if let Some(effects) = self.recipients.get_mut(recipient) {
                effects.prepare(snapshot, presentations, characters, weapon_clock)?;
            }
        }
        Ok(())
    }

    /// Sample one frame for a seat (donor `frame`).
    pub fn frame(
        &mut self,
        camera: &SceneCamera,
        source: &SourceSceneOrder,
        viewer: Option<&ActorId>,
        q1_fog: Option<Q1FogInput>,
    ) -> Result<ApplicationEffectFrame, ApplicationEffectError> {
        if self.closed {
            return Err(ApplicationEffectError::Closed);
        }
        let seconds = self.time.unwrap_or(0.0);
        let projector = ViewProjector::new(*camera, None);
        // `project_point` without a model transform cannot fail; the fallback
        // only satisfies the infallible donor projector shape.
        let project = |point: Vec3| {
            projector.project(point).unwrap_or(Vec4 {
                x: 0.0,
                y: 0.0,
                z: 0.0,
                w: 1.0,
            })
        };
        let mut prepared: Vec<SceneOperation> = Vec::new();
        let mut q3_lights: Vec<ClientDynamicLight> = Vec::new();
        let contents: Vec<ContentId> = self.groups.keys().cloned().collect();
        for content in &contents {
            let group = self
                .groups
                .get(content)
                .ok_or_else(|| ApplicationEffectError::UnknownContent(content.as_str().to_string()))?;
            let family = group.family;
            if family != GameFamily::Q3 && !group.sampled.is_empty() {
                let image = self
                    .images
                    .get(&family)
                    .ok_or(ApplicationEffectError::MissingParticleTexture)?;
                let profile = if family == GameFamily::Q1 {
                    IndexedProfile::Q1
                } else {
                    IndexedProfile::Q2
                };
                let palette = group.palette.as_ref().ok_or(ApplicationEffectError::MissingPalette)?;
                let widest = group.sampled.iter().filter_map(|particle| match particle {
                    SceneParticle::Indexed { palette_index, .. } => Some(usize::from(*palette_index)),
                    SceneParticle::Rgba { .. } => None,
                });
                if let Some(widest) = widest.max() {
                    if palette.len() < widest * 3 + 3 {
                        return Err(ApplicationEffectError::IncompletePalette);
                    }
                }
                let context = ParticlePreparationContext {
                    camera: *camera,
                    indexed_profile: profile,
                    palette_color: &|index: u8| {
                        let offset = usize::from(index) * 3;
                        vec3(palette[offset], palette[offset + 1], palette[offset + 2])
                    },
                };
                prepared.push(SceneOperation::Group(sequence_draw_group(
                    SequencePhase::Translucent,
                    vec![prepare_particle_batch(&group.sampled, &context, image, &project)],
                )));
            }
            let mut models: Vec<SceneEntity> = Vec::new();
            models.extend(group.statics.iter().map(|entry| entry.entity));
            models.extend(group.models.iter().copied());
            for beam in &group.beams {
                if viewer == beam.actor.as_ref() {
                    if let Some(local) = &beam.local {
                        models.extend(local.iter().copied());
                        continue;
                    }
                }
                models.extend(beam.remote.iter().copied());
            }
            let view = EffectModelView {
                camera,
                time_seconds: seconds,
                lights: &self.sampled_lights,
            };
            prepared.extend(self.host.prepare_models(content, &models, &view));
        }
        let unmodeled: Vec<EffectBeam> = self.beams.iter().filter(|beam| beam.model.is_none()).cloned().collect();
        for beam in &unmodeled {
            let group = self
                .groups
                .get(&beam.content)
                .ok_or(ApplicationEffectError::MissingBeamPalette)?;
            let color = Self::palette_color(group, beam.color as u8)?;
            prepared.push(SceneOperation::Group(sequence_draw_group(
                SequencePhase::Translucent,
                vec![q2_beam_batch(
                    beam.start,
                    beam.end,
                    beam.width,
                    [color.x as u8, color.y as u8, color.z as u8, Q2_BEAM_ALPHA],
                    &project,
                    &self.host.white_image(&beam.content),
                    &RenderState {
                        blend: (BlendFactor::SrcAlpha, BlendFactor::OneMinusSrcAlpha),
                        depth_test: DepthTest::LessEqual,
                        depth_write: false,
                        alpha_test: AlphaTest::None,
                        cull: CullFace::None,
                        depth_range: [0.0, 1.0],
                        polygon_offset: None,
                    },
                )],
            )));
        }
        let mut q1_styles = [Q1_STYLE_MISSING; Q1_STYLE_COUNT];
        for (index, slot) in q1_styles.iter_mut().enumerate() {
            if let Some(style) = self.styles.iter().find(|style| style.style == index as i64) {
                *slot = style.value;
            }
        }
        let mut operations: Vec<SceneOperation> = Vec::new();
        let brushes = self.static_brushes.clone();
        for brush in &brushes {
            let view = EffectBrushView {
                camera,
                time_seconds: seconds,
                lights: &self.sampled_lights,
                q1_styles: &q1_styles,
                q1_fog,
                frame: brush.frame,
            };
            operations.extend(self.host.prepare_brush(&brush.brush, &brush.transform, &view));
        }
        operations.extend(prepared);
        let mut source_lights: Vec<SurfaceDynamicLight> = Vec::new();
        let mut q3_admissions: Vec<Q3SceneAdmission> = Vec::new();
        let mut q3_contents: Vec<(bool, ContentId)> = self
            .q3
            .keys()
            .map(|content| (false, content.clone()))
            .chain(self.q3_weapons.keys().map(|content| (true, content.clone())))
            .collect();
        // Donor order is insertion order; sort for deterministic frames.
        q3_contents.sort_by(|(_, a), (_, b)| a.as_str().cmp(b.as_str()));
        for (weapons, content) in &q3_contents {
            let effects = if *weapons {
                self.q3_weapons.get_mut(content)
            } else {
                self.q3.get_mut(content)
            };
            let Some(effects) = effects else {
                continue;
            };
            let frame = effects.frame(camera, source, viewer, q1_fog)?;
            q3_admissions.push(frame.admission);
            operations.extend(frame.operations);
            for light in &frame.q3_lights {
                q3_lights.push(*light);
                source_lights.push(SurfaceDynamicLight {
                    origin: light.origin,
                    radius: light.radius,
                    minimum: 0.0,
                    color: light.color,
                });
            }
        }
        for light in &self.sampled_lights {
            q3_lights.push(ClientDynamicLight {
                origin: light.origin,
                radius: light.radius,
                color: light.color,
                additive: false,
            });
        }
        if let Some(viewer) = viewer {
            if let Some(effects) = self.recipients.get_mut(viewer) {
                let local = effects.frame(camera, source, Some(viewer), q1_fog)?;
                q3_admissions.extend(local.q3_admissions);
                operations.extend(local.operations);
                source_lights.extend(local.lights);
                q3_lights.extend(local.q3_lights);
            }
        }
        let mut polygon: Vec<SceneOperation> = Vec::new();
        let mut rest: Vec<SceneOperation> = Vec::new();
        for operation in operations {
            if is_polygon_operation(&operation) {
                polygon.push(operation);
            } else {
                rest.push(operation);
            }
        }
        polygon.extend(rest);
        q3_lights.truncate(MAX_FRAME_Q3_LIGHTS);
        let mut lights = self.sampled_lights.clone();
        lights.extend(source_lights);
        Ok(ApplicationEffectFrame {
            q3_admissions,
            operations: polygon,
            lights,
            q3_lights,
        })
    }

    /// Collect shadow-casting scene lights (donor `shadowSceneLights`).
    pub fn shadow_scene_lights(
        &self,
        camera: &SceneCamera,
        style: &dyn Fn(i32) -> f64,
        viewer: Option<&ActorId>,
    ) -> Vec<SceneLight> {
        let mut lights = Vec::new();
        if let Some(viewer) = viewer {
            if let Some(effects) = self.recipients.get(viewer) {
                lights.extend(effects.shadow_scene_lights(camera, style, Some(viewer)));
            }
        }
        for light in self.flashlights.values() {
            let Some(pose) = self.pose(&light.actor) else {
                continue;
            };
            let local = viewer == Some(&light.actor);
            let axis = if local {
                camera.axis
            } else {
                angles_to_axis(pose.angles)
            };
            // q2repro CL_AddPacketEntities per-pixel flashlight; scene axis[1]
            // is left.
            let offset = match light.hand {
                Q2PlayerHand::Center => 0.0,
                Q2PlayerHand::Left => 7.0,
                Q2PlayerHand::Right => -7.0,
            };
            let origin = if local {
                add3(camera.origin, scale3(axis[1], offset))
            } else {
                pose.origin
            };
            lights.push(SceneLight {
                origin,
                color: EFFECT_WHITE,
                radius: 512.0,
                additive: true,
                profile: LightProfile::Q2 {
                    scale: 2.0,
                    cone: Some((axis[0], (22.0 * std::f64::consts::PI / 180.0).cos() as f32)),
                    shadow: LightShadow::Cast { resolution: 512 },
                },
            });
        }
        for entry in self.shadow_lights.values() {
            let light = &entry.state;
            if !light.visible || light.radius <= 0.0 {
                continue;
            }
            let fade = shadow_light_fade(
                light.fade_start,
                light.fade_end,
                f64::from(length3(sub3(light.origin, camera.origin))),
            );
            if fade <= 0.0 {
                continue;
            }
            let scale = (light.intensity
                * fade
                * if light.lightstyle == -1 {
                    1.0
                } else {
                    style(light.lightstyle)
                }) as f32;
            lights.push(SceneLight {
                origin: light.origin,
                color: light.color,
                radius: light.radius as f32,
                additive: true,
                profile: LightProfile::Q2 {
                    scale,
                    cone: light.cone.map(|cone| (cone.direction, cone.cos_half_angle as f32)),
                    shadow: LightShadow::Cast {
                        resolution: light.resolution,
                    },
                },
            });
        }
        lights
    }
    /// Present one pending event (donor `event`, sync). The donor rechecks
    /// owner currency after every await; synchronous host calls cannot
    /// interleave owner revisions, so the single entry check is equivalent.
    fn event(&mut self, source: &ApplicationEffectEvent) -> Result<(), ApplicationEffectError> {
        let owner_key = presentation_owner_key(source.owner.as_ref());
        let revision = self.owner_revisions.get(&owner_key).copied().unwrap_or(0);
        let current = !self.closed
            && !self.retired_owners.contains(&owner_key)
            && source.sequence > self.owner_revisions.get(&owner_key).copied().unwrap_or(-1)
            && self.owner_revisions.get(&owner_key).copied().unwrap_or(0) == revision;
        if !current {
            return Ok(());
        }
        match &source.kind {
            ApplicationEffectEventKind::PresentationOwner(_)
            | ApplicationEffectEventKind::Q1Fog
            | ApplicationEffectEventKind::Ignored => Ok(()),
            ApplicationEffectEventKind::Q3Ballistics(event) => self.q3_ballistic(source, event),
            ApplicationEffectEventKind::Q3Source(event) => self.q3_source(source, event),
            ApplicationEffectEventKind::Q2Composition(event) => self.q2_composition(source, event),
            ApplicationEffectEventKind::Q2Rerelease(event) => self.q2_rerelease(source, event),
            ApplicationEffectEventKind::Q3Character(event) => self.q3_character(source, event),
            ApplicationEffectEventKind::Q1(event) => self.q1(source, event),
            ApplicationEffectEventKind::Q2Weapon(event) => self.q2_weapon(source, event),
            ApplicationEffectEventKind::Q2(event) => self.q2(source, event),
        }
    }

    /// Present one Q3 ballistic event (donor `event` Q3-ballistics branch).
    fn q3_ballistic(
        &mut self,
        source: &ApplicationEffectEvent,
        event: &Q3SharedBallisticEvent,
    ) -> Result<(), ApplicationEffectError> {
        if matches!(event.kind, Q3BallisticEventKind::RailAward { .. }) {
            self.reject(
                source,
                "Selected Q3 rail reward presentation has no source cgame binding".to_string(),
            );
            return Ok(());
        }
        if !self.q3_weapons.contains_key(&source.content) {
            let effects = if let Some(prepared) = self.prepared_q3_weapons.remove(&source.content) {
                prepared
            } else {
                (self.q3_factory)(&source.content, None, (self.read_hardware)())?
            };
            self.q3_weapons.insert(source.content.clone(), effects);
        }
        let effects = self
            .q3_weapons
            .get_mut(&source.content)
            .ok_or_else(|| ApplicationEffectError::UnknownContent(source.content.as_str().to_string()))?;
        effects.ballistic(event)
    }

    /// Present one Q3 source event (donor `event` Q3-source branch).
    fn q3_source(
        &mut self,
        source: &ApplicationEffectEvent,
        event: &ApplicationQ3SourceEvent,
    ) -> Result<(), ApplicationEffectError> {
        match event {
            ApplicationQ3SourceEvent::Sound {
                actor,
                origin,
                velocity,
                path,
                channel,
                volume,
                looping,
            } => {
                self.sounds.push(SourceEffectSound {
                    content: source.content.clone(),
                    path: path.clone(),
                    origin: *origin,
                    channel: *channel,
                    volume: *volume,
                    seconds: source.seconds as f32,
                    playback: if *looping {
                        SourceEffectPlayback::Loop {
                            actor: actor.clone(),
                            velocity: *velocity,
                        }
                    } else {
                        SourceEffectPlayback::Actor { actor: actor.clone() }
                    },
                });
            }
            ApplicationQ3SourceEvent::EntityEvent => {
                self.reject(
                    source,
                    "Native Q3 entity event requires its per-seat cgame snapshot and weapon presentation context"
                        .to_string(),
                );
            }
            ApplicationQ3SourceEvent::Other => {}
        }
        Ok(())
    }

    /// Present one Q2 composition event (donor `event` Q2-composition branch).
    fn q2_composition(
        &mut self,
        source: &ApplicationEffectEvent,
        event: &ApplicationCompositionEvent,
    ) -> Result<(), ApplicationEffectError> {
        match event {
            ApplicationCompositionEvent::MissionpackPlayer(event) => {
                if let Q2MissionPackPlayerEffect::TrackerPain { actor, until } = event {
                    self.tracker_pain.insert(
                        actor.clone(),
                        TrackerPain {
                            content: source.content.clone(),
                            until: *until,
                        },
                    );
                } else {
                    self.player_views.receive(event);
                }
            }
            ApplicationCompositionEvent::MissionpackEntity(event) => match event {
                Q2MissionPackEntityEvent::ForceWall { start, end, color } => {
                    let (start, end, color) = (*start, *end, *color);
                    self.with_particles(&source.content, |particles| {
                        particles.q2_force_wall(start, end, color as u8, source.seconds as f32);
                    })?;
                }
                Q2MissionPackEntityEvent::Steam {
                    id,
                    origin,
                    direction,
                    count,
                    color,
                    speed,
                    milliseconds,
                } => {
                    if *id == -1 {
                        let (origin, direction, count, color, speed) = (*origin, *direction, *count, *color, *speed);
                        self.with_particles(&source.content, |particles| {
                            particles.q2_steam(
                                origin,
                                direction,
                                color as u8,
                                count.max(0) as u32,
                                speed as f32,
                                source.seconds as f32,
                                false,
                            );
                        })?;
                    } else if self.steam.len() < MAX_STEAM_JETS {
                        self.steam.push(EffectSteam {
                            content: source.content.clone(),
                            event: event.clone(),
                            end: source.seconds + f64::from(*milliseconds) / 1000.0,
                            next: source.seconds,
                        });
                    }
                }
            },
            ApplicationCompositionEvent::Ctf(event) => {
                if let Q2CtfEvent::GrappleCable {
                    actor,
                    start,
                    end,
                    offset,
                } = event
                {
                    self.ctf_cable(source, actor, start, end, offset)?;
                }
            }
            ApplicationCompositionEvent::Lmctf(event) => {
                if let LmctfEvent::GrappleCable {
                    actor,
                    start,
                    end,
                    offset,
                } = event
                {
                    self.ctf_cable(source, actor, start, end, offset)?;
                }
            }
            ApplicationCompositionEvent::Kick { .. } | ApplicationCompositionEvent::GrapplePrediction { .. } => {
                self.reject(source, "Q2 session action reached the presentation owner".to_string());
            }
        }
        Ok(())
    }

    /// Stage a CTF/LMCTF grapple cable (donor Q2-composition cable branch).
    fn ctf_cable(
        &mut self,
        source: &ApplicationEffectEvent,
        actor: &ActorId,
        start: &Vec3,
        end: &Vec3,
        offset: &Vec3,
    ) -> Result<(), ApplicationEffectError> {
        self.group(&source.content)?;
        self.push_beam(EffectBeam {
            owner: source.owner.clone(),
            content: source.content.clone(),
            actor: Some(actor.clone()),
            start: add3(*start, *offset),
            end: *end,
            die: source.seconds + 0.2,
            width: 0.0,
            color: 0,
            model: Some(Q2_TRANSIENT_MODELS.cable.to_string()),
            family: GameFamily::Q2,
        });
        Ok(())
    }

    /// Present one Q2 rerelease event (donor `event` Q2-rerelease branch).
    fn q2_rerelease(
        &mut self,
        source: &ApplicationEffectEvent,
        event: &Q2RereleaseEvent,
    ) -> Result<(), ApplicationEffectError> {
        match event {
            Q2RereleaseEvent::Flashlight { actor, enabled, hand } => {
                if *enabled {
                    self.flashlights.insert(
                        actor.clone(),
                        FlashlightState {
                            actor: actor.clone(),
                            hand: *hand,
                            owner: source.owner.clone(),
                        },
                    );
                } else {
                    self.flashlights.remove(actor);
                }
            }
            Q2RereleaseEvent::DynamicLight {
                actor,
                origin,
                radius,
                color,
                visible,
            } => {
                if *visible {
                    self.source_lights.insert(
                        actor.clone(),
                        SourceLightEntry {
                            light: SurfaceDynamicLight {
                                origin: *origin,
                                radius: *radius as f32,
                                color: *color,
                                minimum: 0.0,
                            },
                            owner: source.owner.clone(),
                        },
                    );
                } else {
                    self.source_lights.remove(actor);
                }
            }
            _ => {}
        }
        Ok(())
    }

    /// Present one Q3 character event (donor `event` Q3-character branch).
    fn q3_character(
        &mut self,
        source: &ApplicationEffectEvent,
        event: &Q3CharacterPresentationEvent,
    ) -> Result<(), ApplicationEffectError> {
        let Some(pose) = self.pose(&event.actor).cloned() else {
            self.reject(source, "Q3 character event has no captured actor pose".to_string());
            return Ok(());
        };
        if !self.q3.contains_key(&source.content) {
            let effects = (self.q3_factory)(&source.content, None, (self.read_hardware)())?;
            self.q3.insert(source.content.clone(), effects);
        }
        let effects = self
            .q3
            .get_mut(&source.content)
            .ok_or_else(|| ApplicationEffectError::UnknownContent(source.content.as_str().to_string()))?;
        if !effects.character_event(event, pose.origin)? {
            self.reject(source, "Q3 event requires the full cgame snapshot payload".to_string());
        }
        Ok(())
    }

    /// Present one Quake event (donor `event` Q1 branch).
    fn q1(&mut self, source: &ApplicationEffectEvent, event: &Q1Event) -> Result<(), ApplicationEffectError> {
        match event {
            Q1Event::Effect { effect, actor, .. } if *effect == Q1Effect::Pickup => {
                if let Some(actor) = actor {
                    self.bonus_flashes.insert(actor.clone(), source.seconds + 0.5);
                }
                return Ok(());
            }
            Q1Event::StaticModel {
                path,
                frame,
                skin,
                origin,
                angles,
                ..
            } => {
                if path.is_empty() {
                    return Ok(());
                }
                let asset = self.host.load_model(&source.content, path)?;
                if let Some(brush) = asset.brush {
                    self.static_brushes.push(StaticBrush {
                        owner: source.owner.clone(),
                        brush,
                        transform: ModelTransform {
                            origin: *origin,
                            axis: angles_to_axis(*angles),
                            scale: 1.0,
                        },
                        frame: i64::from(*frame),
                    });
                    return Ok(());
                }
                let entity = SceneEntity {
                    entity_number: asset.entity_number,
                    model: asset.handle,
                    transform: ModelTransform {
                        origin: *origin,
                        axis: angles_to_axis(*angles),
                        scale: 1.0,
                    },
                    previous_origin: *origin,
                    pose: ModelPose {
                        frame: *frame,
                        old_frame: *frame,
                        back_lerp: 0.0,
                    },
                    skin: *skin,
                    color: vec4(1.0, 1.0, 1.0, 1.0),
                    flags: self.host.scene_flags(GameFamily::Q1, 0),
                    lighting_origin: Some(*origin),
                    shadow_plane: None,
                    opacity: None,
                };
                let group = self.group(&source.content)?;
                group.statics.push(StaticModelEntry {
                    owner: source.owner.clone(),
                    entity,
                });
                return Ok(());
            }
            Q1Event::Effect { .. } | Q1Event::Beam { .. } | Q1Event::ColoredExplosion { .. } => {}
            _ => return Ok(()),
        }
        self.group(&source.content)?;
        match event {
            Q1Event::Beam {
                style,
                actor,
                start,
                end,
            } => {
                self.push_beam(EffectBeam {
                    owner: source.owner.clone(),
                    content: source.content.clone(),
                    actor: Some(actor.clone()),
                    start: *start,
                    end: *end,
                    die: source.seconds + 0.2,
                    width: 0.0,
                    color: 0,
                    model: Some(q1_beam_model(*style).to_string()),
                    family: GameFamily::Q1,
                });
            }
            Q1Event::ColoredExplosion {
                origin,
                color_start,
                color_length,
            } => {
                if !(0..=255).contains(color_start) || !(1..=255).contains(color_length) {
                    return Err(ParticleError::InvalidColorRange.into());
                }
                let (origin, color_start, color_length) = (*origin, *color_start as u8, *color_length as u8);
                self.with_particles(&source.content, |particles| {
                    particles.q1_color_explosion(origin, source.seconds as f32, color_start, color_length)
                })??;
                self.light(origin, source.seconds, 350.0, 0.5, EFFECT_WHITE, 300.0, 0.0, None);
                self.push_sound(&source.content, "weapons/r_exp3.wav", origin, source.seconds);
            }
            Q1Event::Effect {
                effect,
                actor,
                origin,
                amount,
                muzzle,
            } => {
                self.q1_effect(source, *effect, actor.as_ref(), *origin, *amount, muzzle.as_ref())?;
            }
            _ => {}
        }
        Ok(())
    }

    /// Present one Quake temp-entity effect (donor Q1 `effect` switch).
    fn q1_effect(
        &mut self,
        source: &ApplicationEffectEvent,
        effect: Q1Effect,
        actor: Option<&ActorId>,
        origin: Vec3,
        amount: i32,
        muzzle: Option<&Q1Muzzle>,
    ) -> Result<(), ApplicationEffectError> {
        match effect {
            Q1Effect::Blood | Q1Effect::MeatSpray => {
                self.with_particles(&source.content, |particles| {
                    particles.q1_impact(origin, EFFECT_ZERO, 73, amount.max(0) as u32, source.seconds as f32);
                })?;
            }
            Q1Effect::Gunshot => {
                self.with_particles(&source.content, |particles| {
                    particles.q1_impact(origin, EFFECT_ZERO, 0, 20, source.seconds as f32);
                })?;
            }
            Q1Effect::Spike | Q1Effect::Superspike => {
                let count = if effect == Q1Effect::Spike { 10 } else { 20 };
                self.with_particles(&source.content, |particles| {
                    particles.q1_impact(origin, EFFECT_ZERO, 0, count, source.seconds as f32);
                })?;
                if !self.random.next_integer().is_multiple_of(5) {
                    self.push_sound(&source.content, "weapons/tink1.wav", origin, source.seconds);
                } else {
                    let ricochet = self.random.next_integer() & 3;
                    let path = if ricochet == 1 {
                        "weapons/ric1.wav"
                    } else if ricochet == 2 {
                        "weapons/ric2.wav"
                    } else {
                        "weapons/ric3.wav"
                    };
                    self.push_sound(&source.content, path, origin, source.seconds);
                }
            }
            Q1Effect::WizardSpike | Q1Effect::KnightSpike => {
                let wizard = effect == Q1Effect::WizardSpike;
                let (color, count) = if wizard { (20, 30) } else { (226, 20) };
                self.with_particles(&source.content, |particles| {
                    particles.q1_impact(origin, EFFECT_ZERO, color, count, source.seconds as f32);
                })?;
                self.push_sound(
                    &source.content,
                    if wizard { "wizard/hit.wav" } else { "hknight/hit.wav" },
                    origin,
                    source.seconds,
                );
            }
            Q1Effect::Explosion => {
                self.with_particles(&source.content, |particles| {
                    particles.q1_explosion(origin, source.seconds as f32, false);
                })?;
                self.light(origin, source.seconds, 350.0, 0.5, EFFECT_WHITE, 300.0, 0.0, None);
                self.push_sound(&source.content, "weapons/r_exp3.wav", origin, source.seconds);
            }
            Q1Effect::TarExplosion => {
                self.with_particles(&source.content, |particles| {
                    particles.q1_explosion(origin, source.seconds as f32, true);
                })?;
                self.push_sound(&source.content, "weapons/r_exp3.wav", origin, source.seconds);
            }
            Q1Effect::LavaSplash | Q1Effect::Teleport => {
                let lava = effect == Q1Effect::LavaSplash;
                self.with_particles(&source.content, |particles| {
                    particles.q1_splash(origin, source.seconds as f32, lava);
                })?;
            }
            Q1Effect::Muzzleflash => {
                let direction = actor
                    .and_then(|actor| self.pose(actor))
                    .map(|pose| angles_to_axis(pose.angles)[0])
                    .unwrap_or(EFFECT_ZERO);
                let origin = match muzzle {
                    Some(muzzle) => add3(muzzle.origin, scale3(angles_to_axis(muzzle.angles)[0], 18.0)),
                    None => add3(add3(origin, vec3(0.0, 0.0, 16.0)), scale3(direction, 18.0)),
                };
                let radius = 200.0 + (self.random.next_integer() & 31) as f32;
                self.light(
                    origin,
                    source.seconds,
                    radius,
                    0.1,
                    EFFECT_WHITE,
                    0.0,
                    32.0,
                    actor.cloned(),
                );
            }
            Q1Effect::Pickup => {}
        }
        Ok(())
    }
    /// Present one Q2 weapon event (donor `event` Q2-weapon branch).
    fn q2_weapon(
        &mut self,
        source: &ApplicationEffectEvent,
        event: &Q2WeaponEvent,
    ) -> Result<(), ApplicationEffectError> {
        match event {
            Q2WeaponEvent::Muzzleflash { .. } | Q2WeaponEvent::Beam { .. } => {}
            _ => return Ok(()),
        }
        self.group(&source.content)?;
        match event {
            Q2WeaponEvent::Muzzleflash { actor, flash, silenced } => {
                let Some(pose) = self.pose(actor).cloned() else {
                    self.reject(source, "Q2 muzzle flash has no captured actor pose".to_string());
                    return Ok(());
                };
                let axis = angles_to_axis(pose.angles);
                let origin = add3(add3(pose.origin, scale3(axis[0], 18.0)), scale3(axis[1], -16.0));
                if !self.muzzle(origin, source.seconds, *flash, *silenced, actor) {
                    self.reject(
                        source,
                        format!("Quake II player muzzle flash {flash} has no source definition"),
                    );
                }
                if *flash == 9 || *flash == 10 || *flash == 11 {
                    let kind = if *flash == 9 {
                        Q2RespawnKind::Login
                    } else if *flash == 10 {
                        Q2RespawnKind::Logout
                    } else {
                        Q2RespawnKind::Respawn
                    };
                    self.with_particles(&source.content, |particles| {
                        particles.q2_respawn(pose.origin, source.seconds as f32, kind);
                    })?;
                }
            }
            Q2WeaponEvent::Beam {
                effect,
                actor,
                start,
                end,
                duration,
            } => match effect {
                WeaponBeamEffect::Rail | WeaponBeamEffect::RailWater => {
                    let (start, end) = (*start, *end);
                    self.with_particles(&source.content, |particles| {
                        particles.q2_rail(start, end, source.seconds as f32);
                    })?;
                }
                WeaponBeamEffect::BubbleTrail => {
                    let (start, end) = (*start, *end);
                    self.with_particles(&source.content, |particles| {
                        particles.q2_bubbles(start, end, source.seconds as f32);
                    })?;
                }
                WeaponBeamEffect::BfgLaser | WeaponBeamEffect::BfgZap | WeaponBeamEffect::BfgLightning => {
                    let die = source.seconds + if *duration > 0.0 { *duration } else { 0.1 };
                    let color = 0xd0 + (self.random.next_integer() & 3) as i32;
                    self.push_beam(EffectBeam {
                        owner: source.owner.clone(),
                        content: source.content.clone(),
                        actor: actor.clone(),
                        start: *start,
                        end: *end,
                        die,
                        width: 4.0,
                        color,
                        model: if *effect == WeaponBeamEffect::BfgLightning {
                            Some(Q2_TRANSIENT_MODELS.lightning.to_string())
                        } else {
                            None
                        },
                        family: GameFamily::Q2,
                    });
                }
                WeaponBeamEffect::Heatbeam | WeaponBeamEffect::MonsterHeatbeam => {
                    self.reject(
                        source,
                        "Q2 heatbeam requires the source player-beam view and offset context".to_string(),
                    );
                }
            },
            _ => {}
        }
        Ok(())
    }

    /// Present one Quake II event (donor `event` Q2 fallthrough branch).
    fn q2(
        &mut self,
        source: &ApplicationEffectEvent,
        event: &Q2PresentationEvent,
    ) -> Result<(), ApplicationEffectError> {
        match event {
            Q2PresentationEvent::Effect(event) => self.q2_effect(source, event),
            Q2PresentationEvent::Beam(beam) => {
                self.group(&source.content)?;
                if !beam.visible {
                    self.beams.retain(|live| live.actor.as_ref() != Some(&beam.actor));
                    return Ok(());
                }
                self.push_beam(EffectBeam {
                    owner: source.owner.clone(),
                    content: source.content.clone(),
                    actor: Some(beam.actor.clone()),
                    start: beam.start,
                    end: beam.end,
                    die: f64::INFINITY,
                    width: beam.width as f32,
                    color: beam.color & 255,
                    model: None,
                    family: GameFamily::Q2,
                });
                Ok(())
            }
            Q2PresentationEvent::MonsterBeam { actor, start, end, .. } => {
                self.group(&source.content)?;
                self.push_beam(EffectBeam {
                    owner: source.owner.clone(),
                    content: source.content.clone(),
                    actor: Some(actor.clone()),
                    start: *start,
                    end: *end,
                    die: source.seconds + 0.2,
                    width: 0.0,
                    color: 0,
                    model: Some(Q2_TRANSIENT_MODELS.parasite.to_string()),
                    family: GameFamily::Q2,
                });
                Ok(())
            }
            Q2PresentationEvent::MonsterMuzzleflash {
                actor, flash, origin, ..
            } => self.monster_muzzle(source, actor, *origin, *flash),
            Q2PresentationEvent::EntityEvent { actor, event } => {
                if *event != 1 && *event != 6 && *event != 7 {
                    return Ok(());
                }
                let Some(pose) = self.pose(actor).cloned() else {
                    self.reject(source, "Q2 entity event has no captured actor pose".to_string());
                    return Ok(());
                };
                self.group(&source.content)?;
                if *event == 1 {
                    self.with_particles(&source.content, |particles| {
                        particles.q2_respawn(pose.origin, source.seconds as f32, Q2RespawnKind::Item);
                    })?;
                } else {
                    self.with_particles(&source.content, |particles| {
                        particles.q2_teleport(pose.origin, source.seconds as f32);
                    })?;
                }
                Ok(())
            }
            Q2PresentationEvent::DynamicLight(state) => {
                self.shadow_lights.insert(
                    state.actor.clone(),
                    ShadowLightEntry {
                        state: state.clone(),
                        owner: source.owner.clone(),
                    },
                );
                Ok(())
            }
            _ => Ok(()),
        }
    }

    /// Emit a Quake II player muzzle flash (donor `muzzle`, via
    /// [`muzzle_flash_profile`]).
    fn muzzle(&mut self, origin: Vec3, seconds: f64, flash: i32, silenced: bool, actor: &ActorId) -> bool {
        let Ok(flash) = u32::try_from(flash) else {
            return false;
        };
        let Some(profile) = muzzle_flash_profile(flash, silenced) else {
            return false;
        };
        let radius = profile.radius as f32 + (self.random.next_integer() & 31) as f32;
        self.light(
            origin,
            seconds,
            radius,
            profile.duration,
            profile.color,
            0.0,
            32.0,
            Some(actor.clone()),
        );
        true
    }

    /// Present one monster muzzle flash (donor `monsterMuzzle`).
    fn monster_muzzle(
        &mut self,
        source: &ApplicationEffectEvent,
        actor: &ActorId,
        origin: Vec3,
        flash: i32,
    ) -> Result<(), ApplicationEffectError> {
        let profile = u16::try_from(flash)
            .ok()
            .and_then(|flash| q2_monster_muzzle(flash, self.host.is_rerelease(&source.content)));
        let Some(profile) = profile else {
            self.reject(
                source,
                format!("Quake II monster muzzle flash {flash} has no source definition"),
            );
            return Ok(());
        };
        self.group(&source.content)?;
        let radius = profile.radius + (self.random.next_integer() & profile.mask) as f32;
        let duration = if profile.radius == 300.0 { 0.2 } else { 0.0 };
        self.light(
            origin,
            source.seconds,
            radius,
            duration,
            profile.color,
            0.0,
            32.0,
            Some(actor.clone()),
        );
        if profile.particles {
            self.with_particles(&source.content, |particles| {
                particles.q2_impact(
                    origin,
                    EFFECT_ZERO,
                    0,
                    40,
                    source.seconds as f32,
                    Q2ImpactVariant::Normal,
                );
            })?;
        }
        if profile.smoke {
            for (path, frames, kind, flags) in [
                (Q2_TRANSIENT_MODELS.smoke, 4, ExplosionKind::Misc, 32),
                (Q2_TRANSIENT_MODELS.flash, 2, ExplosionKind::Flash, 8),
            ] {
                self.explosions.push(EffectExplosion {
                    content: source.content.clone(),
                    origin,
                    angles: EFFECT_ZERO,
                    start: source.seconds - 0.1,
                    frames,
                    base_frame: 0,
                    path: path.to_string(),
                    kind,
                    flags,
                    skin: 0,
                    light: None,
                });
            }
        }
        Ok(())
    }

    /// Present one named Quake II effect (donor `q2Effect`).
    fn q2_effect(
        &mut self,
        source: &ApplicationEffectEvent,
        event: &Q2EffectEvent,
    ) -> Result<(), ApplicationEffectError> {
        let original = event.effect.clone();
        let name = original.strip_prefix("q2:").unwrap_or(&original).replace('_', "-");
        self.group(&source.content)?;
        let time = source.seconds;
        let origin = event.origin;
        let direction = event.direction;
        let count = event.count.max(0) as u32;
        match name.as_str() {
            "heatbeam-sparks" | "heatbeam-steam" => {
                let sparks = name == "heatbeam-sparks";
                let (color, count) = if sparks { (8, 50) } else { (0xe0, 20) };
                self.with_particles(&source.content, |particles| {
                    particles.q2_steam(origin, direction, color, count, 60.0, time as f32, false);
                })?;
                self.push_sound(&source.content, "weapons/lashit.wav", origin, time);
            }
            "chainfist-smoke" => {
                self.with_particles(&source.content, |particles| {
                    particles.q2_steam(origin, vec3(0.0, 0.0, 1.0), 0, 20, 20.0, time as f32, true);
                })?;
            }
            "tracker-explosion" => {
                self.with_particles(&source.content, |particles| {
                    particles.q2_color_explosion(origin, time as f32, 0, 1);
                })?;
                self.light(origin, time, 150.0, 0.1, vec3(-1.0, -1.0, -1.0), 0.0, 250.0, None);
                self.push_sound(&source.content, "weapons/disrupthit.wav", origin, time);
            }
            "blood" => self.impact(source, origin, direction, 0xe8, 60, time, Q2ImpactVariant::Normal)?,
            "moreblood" => self.impact(source, origin, direction, 0xe8, 250, time, Q2ImpactVariant::Normal)?,
            "gunshot" => {
                self.impact(source, origin, direction, 0, 40, time, Q2ImpactVariant::Normal)?;
            }
            "shotgun" => {
                self.impact(source, origin, direction, 0, 20, time, Q2ImpactVariant::Normal)?;
            }
            "sparks" | "bullet-sparks" => {
                self.impact(source, origin, direction, 0xe0, 6, time, Q2ImpactVariant::Normal)?;
            }
            "screen-sparks" => {
                self.impact(source, origin, direction, 0xd0, 40, time, Q2ImpactVariant::Normal)?;
            }
            "shield-sparks" => {
                self.impact(source, origin, direction, 0xb0, 40, time, Q2ImpactVariant::Normal)?;
            }
            "laser-sparks" => self.impact(
                source,
                origin,
                direction,
                event.color as u8,
                count,
                time,
                Q2ImpactVariant::Fixed,
            )?,
            "tunnel-sparks" => self.impact(
                source,
                origin,
                direction,
                event.color as u8,
                count,
                time,
                Q2ImpactVariant::Up,
            )?,
            "splash" => {
                const COLORS: [u8; 7] = [0, 0xe0, 0xb0, 0x50, 0xd0, 0xe0, 0xe8];
                let color = usize::try_from(event.color)
                    .ok()
                    .and_then(|index| COLORS.get(index))
                    .copied()
                    .unwrap_or(0);
                self.impact(source, origin, direction, color, count, time, Q2ImpactVariant::Normal)?;
            }
            "bluehyperblaster" => self.impact(source, origin, direction, 0xe0, 40, time, Q2ImpactVariant::Blaster)?,
            "blaster" | "blaster2" | "flechette" => {
                let color = if name == "blaster" {
                    0xe0
                } else if name == "blaster2" {
                    0xd0
                } else {
                    0x6f
                };
                self.impact(source, origin, direction, color, 40, time, Q2ImpactVariant::Blaster)?;
                let yaw = if direction.x != 0.0 {
                    f64::from(direction.y).atan2(f64::from(direction.x)) * 180.0 / std::f64::consts::PI
                } else if direction.y > 0.0 {
                    90.0
                } else if direction.y < 0.0 {
                    270.0
                } else {
                    0.0
                };
                let pitch = f64::from(direction.z).acos() * 180.0 / std::f64::consts::PI;
                let skin = if name == "blaster" {
                    0
                } else if name == "blaster2" {
                    1
                } else {
                    2
                };
                let color = if name == "blaster" {
                    vec3(1.0, 1.0, 0.0)
                } else if name == "blaster2" {
                    vec3(0.0, 1.0, 0.0)
                } else {
                    vec3(0.19, 0.41, 0.75)
                };
                self.explosions.push(EffectExplosion {
                    content: source.content.clone(),
                    origin,
                    angles: vec3(pitch as f32, yaw as f32, 0.0),
                    start: time - 0.1,
                    frames: 4,
                    base_frame: 0,
                    path: Q2_TRANSIENT_MODELS.explosion.to_string(),
                    kind: ExplosionKind::Misc,
                    flags: 8 | 32,
                    skin,
                    light: Some(ExplosionLight { radius: 150.0, color }),
                });
            }
            "greenblood" => self.impact(source, origin, direction, 0xdf, 30, time, Q2ImpactVariant::Fixed)?,
            "electric-sparks" => self.impact(source, origin, direction, 0x75, 40, time, Q2ImpactVariant::Normal)?,
            "player-teleport" | "other-teleport" => {
                self.with_particles(&source.content, |particles| {
                    particles.q2_teleport(origin, time as f32);
                })?;
            }
            "boss-teleport" => {
                self.with_particles(&source.content, |particles| {
                    particles.q2_big_teleport(origin, time as f32);
                })?;
            }
            "item-respawn" => {
                self.with_particles(&source.content, |particles| {
                    particles.q2_respawn(origin, time as f32, Q2RespawnKind::Item);
                })?;
            }
            "logout" => {
                self.with_particles(&source.content, |particles| {
                    particles.q2_respawn(origin, time as f32, Q2RespawnKind::Logout);
                })?;
            }
            "bfg-bigexplosion" => {
                self.with_particles(&source.content, |particles| {
                    particles.q2_explosion(origin, time as f32, true);
                })?;
            }
            "berserk-slam" => {
                self.with_particles(&source.content, |particles| {
                    particles.q2_berserk_slam(origin, direction, time as f32);
                })?;
            }
            "plain-explosion" => {
                let yaw = (self.random.next_integer() % 360) as f32;
                let base_frame = if self.random.next_unit() < 0.5 { 15 } else { 0 };
                self.explosions.push(EffectExplosion {
                    content: source.content.clone(),
                    origin,
                    angles: vec3(0.0, yaw, 0.0),
                    start: time - 0.1,
                    frames: 15,
                    base_frame,
                    path: Q2_TRANSIENT_MODELS.rocket_explosion.to_string(),
                    kind: ExplosionKind::Poly,
                    flags: 8,
                    skin: 0,
                    light: Some(ExplosionLight {
                        radius: 350.0,
                        color: EFFECT_ORANGE,
                    }),
                });
                self.push_sound(&source.content, "weapons/rocklx1a.wav", origin, time);
            }
            "explosion1"
            | "explosion2"
            | "rocket-explosion"
            | "rocket-explosion-water"
            | "grenade-explosion"
            | "grenade-explosion-water"
            | "bfg-explosion" => {
                let grenade = name == "explosion2" || name.starts_with("grenade");
                let bfg = name == "bfg-explosion";
                if !bfg {
                    self.with_particles(&source.content, |particles| {
                        particles.q2_explosion(origin, time as f32, false);
                    })?;
                }
                let frames = if bfg {
                    4
                } else if grenade {
                    19
                } else {
                    15
                };
                let yaw = (self.random.next_integer() % 360) as f32;
                let base_frame = if bfg {
                    0
                } else if grenade {
                    30
                } else if self.random.next_unit() < 0.5 {
                    15
                } else {
                    0
                };
                self.explosions.push(EffectExplosion {
                    content: source.content.clone(),
                    origin,
                    angles: vec3(0.0, yaw, 0.0),
                    start: time - 0.1,
                    frames,
                    base_frame,
                    path: if bfg {
                        Q2_TRANSIENT_MODELS.bfg_explosion
                    } else {
                        Q2_TRANSIENT_MODELS.rocket_explosion
                    }
                    .to_string(),
                    kind: ExplosionKind::Poly,
                    flags: if bfg { 8 | 32 } else { 8 },
                    skin: 0,
                    light: Some(ExplosionLight {
                        radius: 350.0,
                        color: if bfg { vec3(0.0, 1.0, 0.0) } else { EFFECT_ORANGE },
                    }),
                });
                if !bfg {
                    let path = if name.ends_with("-water") {
                        "weapons/xpld_wat.wav"
                    } else if grenade {
                        "weapons/grenlx1a.wav"
                    } else {
                        "weapons/rocklx1a.wav"
                    };
                    self.push_sound(&source.content, path, origin, time);
                }
            }
            "footstep" | "monster-footstep" | "fall" | "fall-short" | "fall-far" => {}
            _ => {
                self.reject(source, format!("Unresolved Quake II source effect {original}"));
            }
        }
        Ok(())
    }

    /// Emit one Q2 impact burst (donor `q2Effect` impact arms).
    #[allow(clippy::too_many_arguments)]
    fn impact(
        &mut self,
        source: &ApplicationEffectEvent,
        origin: Vec3,
        direction: Vec3,
        color: u8,
        count: u32,
        seconds: f64,
        variant: Q2ImpactVariant,
    ) -> Result<(), ApplicationEffectError> {
        self.with_particles(&source.content, |particles| {
            particles.q2_impact(origin, direction, color, count, seconds as f32, variant);
        })
    }
    /// Present Quake entity trails and lights (donor `q1Entities`, sync).
    fn q1_entities(
        &mut self,
        presentations: &[EffectPresentation],
        seconds: f64,
        advance: bool,
    ) -> Result<(), ApplicationEffectError> {
        if !advance {
            return Ok(());
        }
        let mut trails: HashMap<ActorId, Q1Trail> = HashMap::new();
        for entity in presentations {
            if entity.family != GameFamily::Q1 || entity.view_weapon || entity.path.is_empty() {
                continue;
            }
            let asset = self.host.load_model(&entity.content, &entity.path)?;
            let flags = asset.q1_flags;
            let prior_trail = self.q1_trails.get(&entity.actor).cloned();
            let delta = prior_trail
                .as_ref()
                .map_or(EFFECT_ZERO, |prior| sub3(entity.origin, prior.origin));
            let reset = prior_trail.as_ref().is_none_or(|prior| {
                prior.content != entity.content
                    || prior.path != entity.path
                    || delta.x.abs() > 100.0
                    || delta.y.abs() > 100.0
                    || delta.z.abs() > 100.0
            });
            let start = if reset {
                entity.origin
            } else {
                prior_trail.as_ref().map_or(entity.origin, |prior| prior.origin)
            };
            trails.insert(
                entity.actor.clone(),
                Q1Trail {
                    content: entity.content.clone(),
                    path: entity.path.clone(),
                    origin: entity.origin,
                },
            );
            if entity.effects & 1 != 0 {
                self.with_particles(&entity.content, |particles| {
                    particles.q1_entity(entity.origin, seconds as f32);
                })?;
            }
            if entity.effects & 2 != 0 {
                let forward = angles_to_axis(entity.angles)[0];
                let origin = add3(add3(entity.origin, vec3(0.0, 0.0, 16.0)), scale3(forward, 18.0));
                let radius = 200.0 + (self.random.next_integer() & 31) as f32;
                self.light(
                    origin,
                    seconds,
                    radius,
                    0.1,
                    EFFECT_WHITE,
                    0.0,
                    32.0,
                    Some(entity.actor.clone()),
                );
            }
            if entity.effects & 4 != 0 {
                let origin = add3(entity.origin, vec3(0.0, 0.0, 16.0));
                let radius = 400.0 + (self.random.next_integer() & 31) as f32;
                self.light(
                    origin,
                    seconds,
                    radius,
                    0.001,
                    EFFECT_WHITE,
                    0.0,
                    0.0,
                    Some(entity.actor.clone()),
                );
            }
            if entity.effects & 8 != 0 {
                let radius = 200.0 + (self.random.next_integer() & 31) as f32;
                self.light(
                    entity.origin,
                    seconds,
                    radius,
                    0.001,
                    EFFECT_WHITE,
                    0.0,
                    0.0,
                    Some(entity.actor.clone()),
                );
            }
            if self.host.is_rerelease(&entity.content) {
                if entity.effects & 16 != 0 {
                    let radius = 200.0 + (self.random.next_integer() & 31) as f32;
                    self.light(
                        entity.origin,
                        seconds,
                        radius,
                        0.001,
                        vec3(0.25, 0.25, 1.0),
                        0.0,
                        0.0,
                        Some(entity.actor.clone()),
                    );
                }
                if entity.effects & 32 != 0 {
                    let radius = 200.0 + (self.random.next_integer() & 31) as f32;
                    self.light(
                        entity.origin,
                        seconds,
                        radius,
                        0.001,
                        vec3(1.0, 0.25, 0.25),
                        0.0,
                        0.0,
                        Some(entity.actor.clone()),
                    );
                }
                if entity.effects & 64 != 0 {
                    let radius = 64.0 + (self.random.next_integer() & 31) as f32;
                    let duration = f64::from((seconds + 0.001) as f32) - seconds;
                    self.light(
                        entity.origin,
                        seconds,
                        radius,
                        duration,
                        vec3(1.0, 192.0 / 255.0, 120.0 / 255.0),
                        0.0,
                        0.0,
                        Some(entity.actor.clone()),
                    );
                }
            }
            let trail = if flags & 4 != 0 {
                Some(2)
            } else if flags & 32 != 0 {
                Some(4)
            } else if flags & 16 != 0 {
                Some(3)
            } else if flags & 64 != 0 {
                Some(5)
            } else if flags & 1 != 0 {
                Some(0)
            } else if flags & 2 != 0 {
                Some(1)
            } else if flags & 128 != 0 {
                Some(6)
            } else {
                None
            };
            if let Some(trail) = trail {
                self.with_particles(&entity.content, |particles| {
                    particles.q1_trail(start, entity.origin, trail, seconds as f32);
                })?;
                if trail == 0 {
                    self.light(
                        entity.origin,
                        seconds,
                        200.0,
                        0.01,
                        EFFECT_WHITE,
                        0.0,
                        0.0,
                        Some(entity.actor.clone()),
                    );
                }
            }
        }
        self.q1_trails = trails;
        Ok(())
    }

    /// Present Quake II entity trails and lights (donor `q2Entities`, sync).
    fn q2_entities(
        &mut self,
        presentations: &[EffectPresentation],
        seconds: f64,
        advance: bool,
    ) -> Result<(), ApplicationEffectError> {
        let mut trails: HashMap<ActorId, EntityTrail> = HashMap::new();
        for entity in presentations {
            if entity.family != GameFamily::Q2 || !entity.visible || entity.view_weapon {
                continue;
            }
            let prior_trail = self.entity_trails.get(&entity.actor).cloned();
            let delta = prior_trail
                .as_ref()
                .map_or(EFFECT_ZERO, |prior| sub3(prior.origin, entity.origin));
            let reset = prior_trail.as_ref().is_none_or(|prior| {
                prior.content != entity.content
                    || delta.x.abs() > 512.0
                    || delta.y.abs() > 512.0
                    || delta.z.abs() > 512.0
            });
            let start = if reset {
                entity.origin
            } else {
                prior_trail.as_ref().map_or(entity.origin, |prior| prior.origin)
            };
            let mut count = if reset {
                1024
            } else {
                prior_trail.as_ref().map_or(1024, |prior| prior.count)
            };
            let tracked = self
                .tracker_pain
                .get(&entity.actor)
                .is_some_and(|pain| pain.until > seconds);
            if entity.effects != 0 || tracked {
                self.group(&entity.content)?;
                let flags = entity.effects | if tracked { 0x8000_0000 } else { 0 };
                if advance && flags & 0x0002_0000 != 0 {
                    self.with_particles(&entity.content, |particles| {
                        particles.q2_teleporter(entity.origin, seconds as f32);
                    })?;
                }
                // CL_AddPacketEntities preserves this order; tracker overloads
                // blaster/hyperblaster.
                if flags & 0x10 != 0 {
                    if advance {
                        count = self.with_particles(&entity.content, |particles| {
                            particles.q2_diminishing_trail(
                                start,
                                entity.origin,
                                seconds as f32,
                                count,
                                Q2TrailKind::Rocket,
                            )
                        })?;
                    }
                    self.push_sampled(entity.origin, 200.0, vec3(1.0, 1.0, 0.0));
                } else if flags & 0x08 != 0 {
                    if advance {
                        let green = flags & 0x0400_0000 != 0;
                        self.with_particles(&entity.content, |particles| {
                            particles.q2_blaster_trail(start, entity.origin, seconds as f32, green);
                        })?;
                    }
                    let red = if flags & 0x0400_0000 != 0 { 0.0 } else { 1.0 };
                    self.push_sampled(entity.origin, 200.0, vec3(red, 1.0, 0.0));
                } else if flags & 0x40 != 0 {
                    let red = if flags & 0x0400_0000 != 0 { 0.0 } else { 1.0 };
                    self.push_sampled(entity.origin, 200.0, vec3(red, 1.0, 0.0));
                } else if flags & 0x02 != 0 {
                    if advance {
                        count = self.with_particles(&entity.content, |particles| {
                            particles.q2_diminishing_trail(
                                start,
                                entity.origin,
                                seconds as f32,
                                count,
                                Q2TrailKind::Blood,
                            )
                        })?;
                    }
                } else if flags & 0x20 != 0 {
                    if advance {
                        count = self.with_particles(&entity.content, |particles| {
                            particles.q2_diminishing_trail(
                                start,
                                entity.origin,
                                seconds as f32,
                                count,
                                Q2TrailKind::Smoke,
                            )
                        })?;
                    }
                } else if flags & 0x80 != 0 {
                    const BFG_RADIUS: [f32; 6] = [300.0, 400.0, 600.0, 300.0, 150.0, 75.0];
                    let radius = if flags & 0x2000 != 0 {
                        200.0
                    } else {
                        usize::try_from(entity.frame)
                            .ok()
                            .and_then(|frame| BFG_RADIUS.get(frame))
                            .copied()
                            .unwrap_or(0.0)
                    };
                    self.push_sampled(entity.origin, radius, vec3(0.0, 1.0, 0.0));
                } else if flags & 0x8000_0000 != 0 {
                    if flags & 0x0400_0000 != 0 {
                        let radius = 50.0 + 500.0 * ((seconds * 2.0).sin() + 1.0);
                        self.push_sampled(entity.origin, radius as f32, vec3(-1.0, -1.0, -1.0));
                    } else {
                        if advance {
                            self.with_particles(&entity.content, |particles| {
                                particles.q2_tracker_shell(start, seconds as f32);
                            })?;
                        }
                        self.push_sampled(entity.origin, 155.0, vec3(-1.0, -1.0, -1.0));
                    }
                } else if flags & 0x0400_0000 != 0 {
                    if advance {
                        self.with_particles(&entity.content, |particles| {
                            particles.q2_tracker_trail(start, entity.origin, seconds as f32);
                        })?;
                    }
                    self.push_sampled(entity.origin, 200.0, vec3(-1.0, -1.0, -1.0));
                } else if flags & 0x0020_0000 != 0 {
                    if advance {
                        count = self.with_particles(&entity.content, |particles| {
                            particles.q2_diminishing_trail(
                                start,
                                entity.origin,
                                seconds as f32,
                                count,
                                Q2TrailKind::GreenBlood,
                            )
                        })?;
                    }
                } else if flags & 0x0040_0000 != 0 {
                    self.push_sampled(entity.origin, 200.0, vec3(0.0, 0.0, 1.0));
                } else if flags & 0x0100_0000 != 0 {
                    if advance && flags & 0x2000 != 0 {
                        self.with_particles(&entity.content, |particles| {
                            particles.q2_blaster_trail(start, entity.origin, seconds as f32, false);
                        })?;
                    }
                    self.push_sampled(entity.origin, 130.0, EFFECT_ORANGE);
                }
            }
            trails.insert(
                entity.actor.clone(),
                EntityTrail {
                    content: entity.content.clone(),
                    origin: entity.origin,
                    count,
                },
            );
        }
        self.entity_trails = trails;
        Ok(())
    }

    /// Stage beam segment models (donor `beamModels`, sync).
    fn beam_models(&mut self, beam: &EffectBeam) -> Result<(), ApplicationEffectError> {
        let Some(model) = beam.model.clone() else {
            return Ok(());
        };
        let asset = self.host.load_model(&beam.content, &model)?;
        let mut rolls: Vec<u32> = Vec::new();
        let flags = self
            .host
            .scene_flags(beam.family, if model == Q2_TRANSIENT_MODELS.lightning { 8 } else { 0 });
        let remote = self.beam_segments(&asset, beam, beam.start, &mut rolls, flags);
        if beam.family == GameFamily::Q2 {
            let group = self.group(&beam.content)?;
            group.models.extend(remote);
            return Ok(());
        }
        let origin = beam
            .actor
            .as_ref()
            .and_then(|actor| self.pose(actor))
            .map(|pose| pose.origin);
        let local = origin.map(|origin| self.beam_segments(&asset, beam, origin, &mut rolls, flags));
        let entry = BeamModelEntry {
            actor: beam.actor.clone(),
            remote,
            local,
        };
        let group = self.group(&beam.content)?;
        group.beams.push(entry);
        Ok(())
    }

    /// Build one beam run of segment models (donor `beamModels` `build`).
    fn beam_segments(
        &mut self,
        asset: &EffectModelAsset<H::BrushScene>,
        beam: &EffectBeam,
        start: Vec3,
        rolls: &mut Vec<u32>,
        flags: u32,
    ) -> Vec<SceneEntity> {
        let mut models = Vec::new();
        let delta = sub3(beam.end, start);
        let length = length3(delta);
        let direction = normalize3_or_zero(delta);
        let horizontal = f64::from(delta.x).hypot(f64::from(delta.y));
        let yaw_angle = if horizontal == 0.0 {
            0.0
        } else {
            f64::from(delta.y).atan2(f64::from(delta.x)) * 180.0 / std::f64::consts::PI
        };
        let pitch_angle = if horizontal == 0.0 {
            if delta.z > 0.0 {
                90.0
            } else {
                270.0
            }
        } else {
            f64::from(delta.z).atan2(horizontal) * (if beam.family == GameFamily::Q1 { 180.0 } else { -180.0 })
                / std::f64::consts::PI
        };
        let q1 = beam.family == GameFamily::Q1;
        let yaw = if q1 {
            yaw_angle.trunc()
        } else if yaw_angle < 0.0 {
            yaw_angle + 360.0
        } else {
            yaw_angle
        };
        let pitch = if q1 {
            pitch_angle.trunc()
        } else if pitch_angle < 0.0 {
            pitch_angle + 360.0
        } else {
            pitch_angle
        };
        let lightning = beam.model.as_deref() == Some(Q2_TRANSIENT_MODELS.lightning);
        let model_length = if lightning { 35.0 } else { 30.0 };
        let beam_length = if lightning {
            f64::from(length) - 20.0
        } else {
            f64::from(length)
        };
        let short = lightning && beam_length <= model_length;
        let steps = if short {
            1
        } else {
            (beam_length / model_length).ceil().max(0.0) as usize
        };
        let spacing = if q1 {
            30.0
        } else if steps > 1 {
            (beam_length - model_length) / (steps - 1) as f64
        } else {
            0.0
        };
        for segment in 0..steps {
            let origin = if short {
                beam.end
            } else {
                add3(start, scale3(direction, (segment as f64 * spacing) as f32))
            };
            let roll = if let Some(roll) = rolls.get(segment) {
                *roll
            } else {
                let roll = self.random.next_integer() % 360;
                rolls.push(roll);
                roll
            };
            let angles = if lightning && !short {
                vec3(-pitch as f32, (yaw + 180.0) as f32, roll as f32)
            } else {
                vec3(pitch as f32, yaw as f32, roll as f32)
            };
            models.push(SceneEntity {
                entity_number: asset.entity_number,
                model: asset.handle,
                transform: ModelTransform {
                    origin,
                    axis: angles_to_axis(angles),
                    scale: 1.0,
                },
                previous_origin: origin,
                pose: ModelPose {
                    frame: 0,
                    old_frame: 0,
                    back_lerp: 0.0,
                },
                skin: 0,
                color: vec4(1.0, 1.0, 1.0, 1.0),
                flags,
                lighting_origin: Some(origin),
                shadow_plane: None,
                opacity: None,
            });
        }
        models
    }

    /// Stage one explosion model (donor `explosionModel`, sync).
    fn explosion_model(&mut self, explosion: &EffectExplosion, now: f64) -> Result<(), ApplicationEffectError> {
        let asset = self.host.load_model(&explosion.content, &explosion.path)?;
        let (frame, fraction) = explosion_frame(explosion.start, now);
        let alpha = explosion_alpha(explosion.kind, fraction, explosion.frames.max(0) as u32, frame);
        let skin = explosion_skin(explosion.kind, frame, explosion.skin.max(0) as u32);
        if let Some(light) = &explosion.light {
            self.sampled_lights.push(SurfaceDynamicLight {
                origin: explosion.origin,
                radius: light.radius * alpha as f32,
                color: light.color,
                minimum: 0.0,
            });
        }
        let flags = self.host.scene_flags(
            GameFamily::Q2,
            explosion.flags
                | if explosion.kind == ExplosionKind::Poly && frame >= 10 {
                    32
                } else {
                    0
                },
        );
        let entity = SceneEntity {
            entity_number: asset.entity_number,
            model: asset.handle,
            transform: ModelTransform {
                origin: explosion.origin,
                axis: angles_to_axis(explosion.angles),
                scale: 1.0,
            },
            previous_origin: explosion.origin,
            pose: ModelPose {
                frame: explosion.base_frame as i32 + frame as i32 + 1,
                old_frame: explosion.base_frame as i32 + frame as i32,
                back_lerp: (1.0 - (fraction - f64::from(frame))) as f32,
            },
            skin: skin as i32,
            color: vec4(1.0, 1.0, 1.0, alpha as f32),
            flags,
            lighting_origin: Some(explosion.origin),
            shadow_plane: None,
            opacity: None,
        };
        let group = self.group(&explosion.content)?;
        group.models.push(entity);
        Ok(())
    }

    /// Resolve one palette triple (donor `palette`).
    fn palette_color(group: &EffectGroup, index: u8) -> Result<Vec3, ApplicationEffectError> {
        let palette = group.palette.as_ref().ok_or(ApplicationEffectError::MissingPalette)?;
        let offset = usize::from(index) * 3;
        let (Some(x), Some(y), Some(z)) = (palette.get(offset), palette.get(offset + 1), palette.get(offset + 2))
        else {
            return Err(ApplicationEffectError::IncompletePalette);
        };
        Ok(vec3(*x, *y, *z))
    }

    /// Reset round state (donor `resetRound`).
    pub fn reset_round(&mut self) -> Result<(), ApplicationEffectError> {
        if self.closed {
            return Err(ApplicationEffectError::Closed);
        }
        for effects in self.recipients.values_mut() {
            effects.reset_round()?;
        }
        self.pending.clear();
        self.unhandled.clear();
        self.beams.clear();
        self.explosions.clear();
        self.static_brushes.clear();
        self.styles.clear();
        self.lights.clear();
        self.sampled_lights.clear();
        self.entity_trails.clear();
        self.q1_trails.clear();
        self.shadow_lights.clear();
        self.source_lights.clear();
        self.flashlights.clear();
        self.player_views.clear();
        self.bonus_flashes.clear();
        self.tracker_pain.clear();
        self.steam.clear();
        self.sounds.clear();
        self.poses.clear();
        self.time = None;
        self.q3_weapon_times.clear();
        for group in self.groups.values_mut() {
            group.particles.clear();
            group.models.clear();
            group.statics.clear();
            group.beams.clear();
            group.sampled.clear();
        }
        for effects in self
            .q3
            .values_mut()
            .chain(self.q3_weapons.values_mut())
            .chain(self.prepared_q3_weapons.values_mut())
        {
            effects.reset_round();
        }
        Ok(())
    }

    /// Release the effect world (donor `close`).
    pub fn close(&mut self) {
        if self.closed {
            return;
        }
        for effects in self.recipients.values_mut() {
            effects.close();
        }
        self.recipients.clear();
        self.closed = true;
        self.pending.clear();
        self.unhandled.clear();
        self.beams.clear();
        self.explosions.clear();
        self.lights.clear();
        self.sampled_lights.clear();
        self.static_brushes.clear();
        self.styles.clear();
        for effects in self
            .q3
            .values_mut()
            .chain(self.q3_weapons.values_mut())
            .chain(self.prepared_q3_weapons.values_mut())
        {
            effects.close();
        }
        self.prepared_q3_weapons.clear();
        for (_, image) in std::mem::take(&mut self.images) {
            self.host.release_particle_image(image);
        }
        self.groups.clear();
        self.q3.clear();
        self.q3_weapons.clear();
        self.q3_weapon_times.clear();
        self.entity_trails.clear();
        self.q1_trails.clear();
        self.shadow_lights.clear();
        self.source_lights.clear();
        self.flashlights.clear();
        self.player_views.clear();
        self.bonus_flashes.clear();
        self.tracker_pain.clear();
        self.steam.clear();
        self.sounds.clear();
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn beam_models_match_donor_table() {
        assert_eq!(q1_beam_model(Q1BeamStyle::Lightning1), "progs/bolt.mdl");
        assert_eq!(q1_beam_model(Q1BeamStyle::Lightning2), "progs/bolt2.mdl");
        assert_eq!(q1_beam_model(Q1BeamStyle::Lightning3), "progs/bolt3.mdl");
        assert_eq!(q1_beam_model(Q1BeamStyle::Grapple), "progs/beam.mdl");
    }

    #[test]
    fn undefined_flashes_have_no_profile() {
        assert_eq!(muzzle_flash_profile(15, false), None);
        assert_eq!(muzzle_flash_profile(25, false), None);
        assert_eq!(muzzle_flash_profile(40, false), None);
    }

    #[test]
    fn muzzle_profiles_match_donor_tables() {
        let rail = muzzle_flash_profile(7, false).expect("railgun flash");
        assert_eq!(rail.color, Vec3 { x: 1.0, y: 0.5, z: 0.2 });
        assert_eq!((rail.radius, rail.duration), (200.0, 0.0));
        let hyper = muzzle_flash_profile(4, false).expect("hyperblaster flash");
        assert_eq!((hyper.radius, hyper.duration), (225.0, 0.0001));
        let silenced = muzzle_flash_profile(0, true).expect("silenced flash");
        assert_eq!(silenced.radius, 100.0);
        let login = muzzle_flash_profile(9, false).expect("login flash");
        assert_eq!(login.duration, 0.001);
    }

    #[test]
    fn explosion_frames_advance_every_100ms() {
        let (frame, fraction) = explosion_frame(1.0, 1.25);
        assert_eq!((frame, fraction), (2, 2.5));
        assert!(explosion_live(2, 15));
        assert!(!explosion_live(14, 15));
        assert_eq!(explosion_alpha(ExplosionKind::Flash, 3.0, 4, 3), 1.0);
        assert_eq!(explosion_alpha(ExplosionKind::Misc, 1.0, 4, 1), 1.0 - 1.0 / 3.0);
        assert_eq!(explosion_alpha(ExplosionKind::Poly, 0.0, 15, 0), 1.0);
        assert_eq!(explosion_skin(ExplosionKind::Poly, 4, 0), 2);
        assert_eq!(explosion_skin(ExplosionKind::Poly, 11, 0), 5);
        assert_eq!(explosion_skin(ExplosionKind::Poly, 14, 0), 6);
        assert_eq!(explosion_skin(ExplosionKind::Misc, 3, 2), 2);
    }

    #[test]
    fn bonus_flash_decays_to_zero() {
        assert_eq!(bonus_flash_percent(1.5, 1.0), 50.0);
        assert_eq!(bonus_flash_percent(1.5, 1.25), 25.0);
        assert_eq!(bonus_flash_percent(1.5, 2.0), 0.0);
    }

    #[test]
    fn shadow_fade_matches_donor_branches() {
        assert_eq!(shadow_light_fade(0.0, 0.0, 100.0), 1.0);
        assert_eq!(shadow_light_fade(0.0, 500.0, 250.0), 0.5);
        assert_eq!(shadow_light_fade(500.0, 500.0, 250.0), 1.0);
        assert_eq!(shadow_light_fade(500.0, 500.0, 600.0), 0.0);
        let smooth = shadow_light_fade(250.0, 500.0, 375.0);
        assert!((0.0..=1.0).contains(&smooth));
    }
}

#[cfg(test)]
mod orchestrator_tests {
    use std::cell::RefCell;

    use qa_client::render::scene::submissions::{create_source_scene_order, SceneGroupOrder, SequencePhase};
    use qa_client::render::types::{ImageSource, RenderOperation, RendererImage, ResourceOwner};
    use qa_client::view::{CameraClip, Rect};
    use qa_content::q3::presentation::scene::{snapshot_q3_scene_admission, SceneAdmissionOrigin};
    use qa_core::identity::{IdentityOwner, ProviderId};

    use super::*;

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    struct TestBrush;

    #[derive(Debug, Clone, Default)]
    struct TestHostState {
        families: HashMap<ContentId, GameFamily>,
        rerelease: HashSet<ContentId>,
        players: HashSet<ActorId>,
        palettes: HashMap<ContentId, Vec<f32>>,
        mounts: HashSet<(ContentId, String)>,
        models: HashMap<(ContentId, String), EffectModelAsset<TestBrush>>,
        preloaded: Vec<(ContentId, String)>,
        staged: Vec<(ContentId, usize)>,
        brushes: usize,
        released: usize,
        plan: Option<EffectPreloadPlan>,
        preload_error: Option<String>,
    }

    #[derive(Debug, Clone, Default)]
    struct TestHost {
        state: Rc<RefCell<TestHostState>>,
    }

    fn test_image(name: &str) -> RendererImage {
        let owner = IdentityOwner::create("effects-test").expect("owner");
        RendererImage {
            owner: ResourceOwner::new(1, owner.session().clone(), 0),
            ordinal: 0,
            source: ImageSource::Generated { name: name.to_string() },
            width: 8,
            height: 8,
        }
    }

    impl ApplicationEffectHost for TestHost {
        type BrushScene = TestBrush;

        fn is_player(&self, actor: &ActorId) -> bool {
            self.state.borrow().players.contains(actor)
        }

        fn provider_family(&self, content: &ContentId) -> Option<GameFamily> {
            self.state.borrow().families.get(content).copied()
        }

        fn is_rerelease(&self, content: &ContentId) -> bool {
            self.state.borrow().rerelease.contains(content)
        }

        fn palette(&self, content: &ContentId) -> Option<Vec<f32>> {
            self.state.borrow().palettes.get(content).cloned()
        }

        fn white_image(&self, _content: &ContentId) -> RendererImage {
            test_image("white")
        }

        fn register_particle_image(&mut self, _family: GameFamily) -> RendererImage {
            test_image("particles")
        }

        fn release_particle_image(&mut self, _image: RendererImage) {
            self.state.borrow_mut().released += 1;
        }

        fn mount_has(&self, content: &ContentId, path: &str) -> bool {
            self.state
                .borrow()
                .mounts
                .contains(&(content.clone(), path.to_string()))
        }

        fn load_model(
            &mut self,
            content: &ContentId,
            path: &str,
        ) -> Result<EffectModelAsset<TestBrush>, ApplicationEffectError> {
            Ok(self
                .state
                .borrow()
                .models
                .get(&(content.clone(), path.to_string()))
                .cloned()
                .unwrap_or(EffectModelAsset {
                    handle: 7,
                    entity_number: 3,
                    q1_flags: 0,
                    brush: None,
                }))
        }

        fn preload_model(&mut self, content: &ContentId, path: &str) -> Result<(), ApplicationEffectError> {
            if let Some(error) = self.state.borrow().preload_error.clone() {
                return Err(ApplicationEffectError::Host(error));
            }
            self.state
                .borrow_mut()
                .preloaded
                .push((content.clone(), path.to_string()));
            Ok(())
        }

        fn preload_plan(&self) -> EffectPreloadPlan {
            self.state.borrow().plan.clone().unwrap_or_else(|| {
                let map = ContentId("q1:test:map:1".to_string());
                EffectPreloadPlan {
                    character: map.clone(),
                    map_entities: map,
                    weapons: Vec::new(),
                    extras: Vec::new(),
                }
            })
        }

        fn scene_flags(&self, _family: GameFamily, bits: i64) -> u32 {
            bits as u32
        }

        fn preload_models(
            &mut self,
            content: &ContentId,
            models: &[SceneEntity],
        ) -> Result<(), ApplicationEffectError> {
            self.state.borrow_mut().staged.push((content.clone(), models.len()));
            Ok(())
        }

        fn prepare_models(
            &self,
            _content: &ContentId,
            _models: &[SceneEntity],
            _view: &EffectModelView<'_>,
        ) -> Vec<SceneOperation> {
            Vec::new()
        }

        fn prepare_brush(
            &self,
            _brush: &EffectBrushModel<TestBrush>,
            _transform: &ModelTransform,
            _view: &EffectBrushView<'_>,
        ) -> Vec<SceneOperation> {
            self.state.borrow_mut().brushes += 1;
            vec![SceneOperation::Operation(RenderOperation::Draw(Vec::new()))]
        }
    }

    #[derive(Debug, Clone, Default)]
    struct FakeQ3State {
        ballistics: usize,
        characters: usize,
        prepared: Vec<(i32, i32)>,
        frames: usize,
        sounds: Vec<SourceEffectSound>,
        lights: Vec<ClientDynamicLight>,
        character_result: bool,
    }

    #[derive(Debug, Clone, Default)]
    struct FakeQ3 {
        state: Rc<RefCell<FakeQ3State>>,
    }

    impl ApplicationQ3Effects for FakeQ3 {
        fn ballistic(&mut self, _event: &Q3SharedBallisticEvent) -> Result<(), ApplicationEffectError> {
            self.state.borrow_mut().ballistics += 1;
            Ok(())
        }

        fn character_event(
            &mut self,
            _event: &Q3CharacterPresentationEvent,
            _origin: Vec3,
        ) -> Result<bool, ApplicationEffectError> {
            let mut state = self.state.borrow_mut();
            state.characters += 1;
            Ok(state.character_result)
        }

        fn prepare(&mut self, now_milliseconds: i32, elapsed_milliseconds: i32) -> Result<(), ApplicationEffectError> {
            self.state
                .borrow_mut()
                .prepared
                .push((now_milliseconds, elapsed_milliseconds));
            Ok(())
        }

        fn frame(
            &mut self,
            _camera: &SceneCamera,
            _source: &SourceSceneOrder,
            _viewer: Option<&ActorId>,
            _q1_fog: Option<Q1FogInput>,
        ) -> Result<Q3EffectFrameOutput, ApplicationEffectError> {
            let mut state = self.state.borrow_mut();
            state.frames += 1;
            Ok(Q3EffectFrameOutput {
                admission: snapshot_q3_scene_admission(SceneAdmissionOrigin::Mixed, Vec::new(), Vec::new()),
                operations: Vec::new(),
                q3_lights: state.lights.clone(),
            })
        }

        fn drain_sounds(&mut self) -> Vec<SourceEffectSound> {
            std::mem::take(&mut self.state.borrow_mut().sounds)
        }

        fn reset_round(&mut self) {}

        fn close(&mut self) {}
    }

    fn actor(slot: u32) -> ActorId {
        IdentityOwner::create("effects-test").expect("owner").actor(slot, 0)
    }

    fn content(text: &str) -> ContentId {
        ContentId(text.to_string())
    }

    fn test_camera() -> SceneCamera {
        SceneCamera {
            origin: vec3(0.0, 0.0, 64.0),
            axis: [vec3(1.0, 0.0, 0.0), vec3(0.0, 1.0, 0.0), vec3(0.0, 0.0, 1.0)],
            projection: [
                1.0, 0.0, 0.0, 0.0, //
                0.0, 1.0, 0.0, 0.0, //
                0.0, 0.0, 1.0, 0.0, //
                0.0, 0.0, 0.0, 1.0,
            ],
            viewport: Rect {
                x: 0,
                y: 0,
                width: 640,
                height: 480,
            },
            clip: CameraClip::None,
        }
    }

    struct Harness {
        host: TestHost,
        q3: FakeQ3,
        effects: ApplicationEffects<TestHost, FakeQ3>,
    }

    impl Harness {
        fn new() -> Self {
            let host = TestHost::default();
            let q3 = FakeQ3::default();
            q3.state.borrow_mut().character_result = true;
            let factory_q3 = q3.clone();
            let factory: Q3EffectsFactory<FakeQ3> = Rc::new(move |_, _, _| Ok(factory_q3.clone()));
            let effects = ApplicationEffects::new(host.clone(), 1, factory);
            Self { host, q3, effects }
        }

        fn with_family(&self, id: &str, family: GameFamily) {
            self.host.state.borrow_mut().families.insert(content(id), family);
        }

        fn with_palette(&self, id: &str) {
            let palette: Vec<f32> = (0..768).map(|index| (index % 256) as f32).collect();
            self.host.state.borrow_mut().palettes.insert(content(id), palette);
        }

        fn event(
            &self,
            id: &str,
            sequence: i64,
            seconds: f64,
            kind: ApplicationEffectEventKind,
        ) -> ApplicationEffectEvent {
            ApplicationEffectEvent {
                owner: None,
                recipient: None,
                sequence,
                content: content(id),
                seconds,
                kind,
            }
        }

        fn snapshot(&self, seconds: f64, actors: Vec<ActorId>) -> EffectSnapshot {
            EffectSnapshot {
                time: EffectFrameTime {
                    kind: EffectTimeKind::Seconds,
                    value: seconds,
                },
                light_styles: Vec::new(),
                bodies: Vec::new(),
                actors,
            }
        }

        fn q1_effect_event(&self, id: &str, sequence: i64, seconds: f64, effect: Q1Effect) -> ApplicationEffectEvent {
            self.event(
                id,
                sequence,
                seconds,
                ApplicationEffectEventKind::Q1(Q1Event::Effect {
                    effect,
                    actor: None,
                    origin: vec3(10.0, 20.0, 30.0),
                    amount: 20,
                    muzzle: None,
                }),
            )
        }

        fn q2_effect_event(&self, id: &str, sequence: i64, seconds: f64, name: &str) -> ApplicationEffectEvent {
            self.event(
                id,
                sequence,
                seconds,
                ApplicationEffectEventKind::Q2(Q2PresentationEvent::Effect(Q2EffectEvent {
                    effect: name.to_string(),
                    origin: vec3(1.0, 2.0, 3.0),
                    direction: vec3(0.0, 0.0, 1.0),
                    count: 40,
                    color: 0xe0,
                })),
            )
        }

        fn ballistic_event(
            &self,
            id: &str,
            sequence: i64,
            seconds: f64,
            kind: Q3BallisticEventKind,
        ) -> ApplicationEffectEvent {
            self.event(
                id,
                sequence,
                seconds,
                ApplicationEffectEventKind::Q3Ballistics(Q3SharedBallisticEvent {
                    actor: actor(1),
                    weapon: 7,
                    origin: vec3(0.0, 0.0, 0.0),
                    end: vec3(0.0, 0.0, 0.0),
                    normal: vec3(0.0, 0.0, 1.0),
                    target: None,
                    surface_flags: 0,
                    kind,
                    time_milliseconds: 1000,
                }),
            )
        }

        fn presentation(&self, id: &str, family: GameFamily, actor: ActorId) -> EffectPresentation {
            EffectPresentation {
                actor,
                content: content(id),
                family,
                path: String::new(),
                frame: 0,
                effects: 0,
                origin: vec3(100.0, 0.0, 0.0),
                angles: vec3(0.0, 0.0, 0.0),
                visible: true,
                view_weapon: false,
            }
        }
    }

    #[test]
    fn receive_dedupes_sequences_and_skips_fog() {
        let mut harness = Harness::new();
        harness.with_family("q1:test:map:1", GameFamily::Q1);
        let id = "q1:test:map:1";
        let player = actor(1);
        let events = vec![
            harness.event(id, 5, 1.0, ApplicationEffectEventKind::Q1Fog),
            harness.q1_effect_event(id, 1, 1.0, Q1Effect::Explosion),
            harness.q1_effect_event(id, 1, 1.0, Q1Effect::Explosion),
        ];
        harness.effects.receive(&events).expect("receive");
        let snapshot = harness.snapshot(1.0, vec![player]);
        harness.effects.prepare(&snapshot, &[], &[], None).expect("prepare");
        let sounds = harness.effects.drain_sounds();
        assert_eq!(sounds.len(), 1);
        assert_eq!(sounds[0].path, "weapons/r_exp3.wav");
    }

    #[test]
    fn recipient_events_present_locally() {
        let mut harness = Harness::new();
        harness.with_family("q1:test:map:1", GameFamily::Q1);
        let viewer = actor(1);
        let mut event = harness.q1_effect_event("q1:test:map:1", 1, 1.0, Q1Effect::Explosion);
        event.recipient = Some(viewer.clone());
        harness.effects.receive(&[event]).expect("receive");
        let snapshot = harness.snapshot(1.0, vec![viewer.clone()]);
        harness.effects.prepare(&snapshot, &[], &[], None).expect("prepare");
        assert!(harness.effects.drain_sounds().is_empty());
        let recipient = harness.effects.drain_recipient_sounds();
        assert_eq!(recipient.len(), 1);
        assert_eq!(recipient[0].recipient, viewer);
        assert_eq!(recipient[0].sounds.len(), 1);
        assert_eq!(recipient[0].sounds[0].path, "weapons/r_exp3.wav");
    }

    #[test]
    fn unhandled_events_report_reasons() {
        let mut harness = Harness::new();
        harness.with_family("q2:test:map:1", GameFamily::Q2);
        harness.with_family("q3:test:map:1", GameFamily::Q3);
        let rail_award = harness.ballistic_event(
            "q3:test:map:1",
            1,
            1.0,
            Q3BallisticEventKind::RailAward { count: 2, until: 100 },
        );
        let unknown = harness.q2_effect_event("q2:test:map:1", 2, 1.0, "q2:unknown-fx");
        let muzzle = harness.event(
            "q2:test:map:1",
            3,
            1.0,
            ApplicationEffectEventKind::Q2Weapon(Q2WeaponEvent::Muzzleflash {
                actor: actor(9),
                flash: 7,
                silenced: false,
            }),
        );
        let kick = harness.event(
            "q2:test:map:1",
            4,
            1.0,
            ApplicationEffectEventKind::Q2Composition(ApplicationCompositionEvent::Kick { actor: actor(1) }),
        );
        harness
            .effects
            .receive(&[rail_award, unknown, muzzle, kick])
            .expect("receive");
        let snapshot = harness.snapshot(1.0, vec![actor(1)]);
        harness.effects.prepare(&snapshot, &[], &[], None).expect("prepare");
        let unhandled = harness.effects.drain_unhandled();
        let reasons: Vec<&str> = unhandled.iter().map(|entry| entry.reason.as_str()).collect();
        assert_eq!(
            reasons,
            vec![
                "Selected Q3 rail reward presentation has no source cgame binding",
                "Unresolved Quake II source effect q2:unknown-fx",
                "Q2 muzzle flash has no captured actor pose",
                "Q2 session action reached the presentation owner",
            ]
        );
        assert!(harness.effects.drain_unhandled().is_empty());
    }

    #[test]
    fn recipient_unhandled_events_carry_their_recipient() {
        let mut harness = Harness::new();
        harness.with_family("q2:test:map:1", GameFamily::Q2);
        let viewer = actor(1);
        let mut event = harness.q2_effect_event("q2:test:map:1", 1, 1.0, "nope");
        event.recipient = Some(viewer.clone());
        harness.effects.receive(&[event]).expect("receive");
        let snapshot = harness.snapshot(1.0, vec![viewer.clone()]);
        harness.effects.prepare(&snapshot, &[], &[], None).expect("prepare");
        let unhandled = harness.effects.drain_unhandled();
        assert_eq!(unhandled.len(), 1);
        assert_eq!(unhandled[0].source.recipient, Some(viewer));
        assert_eq!(unhandled[0].reason, "Unresolved Quake II source effect nope");
    }

    #[test]
    fn sounds_map_paths_playback_and_drain_order() {
        let mut harness = Harness::new();
        harness.with_family("q1:test:map:1", GameFamily::Q1);
        harness.with_family("q2:test:map:1", GameFamily::Q2);
        harness.with_family("q3:test:map:1", GameFamily::Q3);
        let player = actor(1);
        let ballistic = harness.ballistic_event("q3:test:map:1", 1, 1.0, Q3BallisticEventKind::Remove);
        let q1 = harness.q1_effect_event("q1:test:map:1", 2, 1.0, Q1Effect::Explosion);
        let grenade = harness.q2_effect_event("q2:test:map:1", 3, 1.0, "q2:grenade-explosion");
        let water = harness.q2_effect_event("q2:test:map:1", 4, 1.0, "rocket-explosion-water");
        let looping = harness.event(
            "q3:test:map:1",
            5,
            1.0,
            ApplicationEffectEventKind::Q3Source(ApplicationQ3SourceEvent::Sound {
                actor: player.clone(),
                origin: vec3(4.0, 5.0, 6.0),
                velocity: vec3(0.0, 0.0, 0.0),
                path: "sound/loop.wav".to_string(),
                channel: 1,
                volume: 0.5,
                looping: true,
            }),
        );
        let once = harness.event(
            "q3:test:map:1",
            6,
            1.0,
            ApplicationEffectEventKind::Q3Source(ApplicationQ3SourceEvent::Sound {
                actor: player.clone(),
                origin: vec3(7.0, 8.0, 9.0),
                velocity: vec3(0.0, 0.0, 0.0),
                path: "sound/once.wav".to_string(),
                channel: 2,
                volume: 1.0,
                looping: false,
            }),
        );
        harness
            .effects
            .receive(&[ballistic, q1, grenade, water, looping, once])
            .expect("receive");
        let snapshot = harness.snapshot(1.0, vec![player.clone()]);
        let clock = EffectWeaponClock {
            content: content("q3:test:map:1"),
            time_milliseconds: 1000,
        };
        harness
            .effects
            .prepare(&snapshot, &[], &[], Some(&clock))
            .expect("prepare");
        // Stage one Q3 sound to pin the drain order (own sounds first).
        harness.q3.state.borrow_mut().sounds.push(SourceEffectSound {
            content: content("q3:test:map:1"),
            path: "sound/q3.wav".to_string(),
            origin: vec3(0.0, 0.0, 0.0),
            channel: 0,
            volume: 1.0,
            seconds: 1.0,
            playback: SourceEffectPlayback::Once,
        });
        let sounds = harness.effects.drain_sounds();
        let paths: Vec<&str> = sounds.iter().map(|sound| sound.path.as_str()).collect();
        assert_eq!(
            paths,
            vec![
                "weapons/r_exp3.wav",
                "weapons/grenlx1a.wav",
                "weapons/xpld_wat.wav",
                "sound/loop.wav",
                "sound/once.wav",
                "sound/q3.wav",
            ]
        );
        assert_eq!(sounds[0].playback, SourceEffectPlayback::Once);
        assert_eq!(sounds[0].channel, 0);
        assert_eq!(sounds[0].volume, 1.0);
        assert_eq!(
            sounds[3].playback,
            SourceEffectPlayback::Loop {
                actor: player.clone(),
                velocity: vec3(0.0, 0.0, 0.0),
            }
        );
        assert_eq!(
            sounds[4].playback,
            SourceEffectPlayback::Actor { actor: player.clone() }
        );
        assert_eq!(sounds[3].channel, 1);
        assert_eq!(sounds[3].volume, 0.5);
    }

    #[test]
    fn q3_source_entity_events_are_rejected() {
        let mut harness = Harness::new();
        harness.with_family("q3:test:map:1", GameFamily::Q3);
        let event = harness.event(
            "q3:test:map:1",
            1,
            1.0,
            ApplicationEffectEventKind::Q3Source(ApplicationQ3SourceEvent::EntityEvent),
        );
        harness.effects.receive(&[event]).expect("receive");
        let snapshot = harness.snapshot(1.0, vec![actor(1)]);
        harness.effects.prepare(&snapshot, &[], &[], None).expect("prepare");
        let unhandled = harness.effects.drain_unhandled();
        assert_eq!(unhandled.len(), 1);
        assert_eq!(
            unhandled[0].reason,
            "Native Q3 entity event requires its per-seat cgame snapshot and weapon presentation context"
        );
    }

    #[test]
    fn frame_routes_particles_recipients_brushes_and_q3() {
        let mut harness = Harness::new();
        harness.with_family("q1:test:map:1", GameFamily::Q1);
        harness.with_palette("q1:test:map:1");
        harness.with_family("q3:test:map:1", GameFamily::Q3);
        let viewer = actor(1);
        let other = actor(2);
        harness.host.state.borrow_mut().models.insert(
            (content("q1:test:map:1"), "maps/brush.bsp".to_string()),
            EffectModelAsset {
                handle: 9,
                entity_number: 4,
                q1_flags: 0,
                brush: Some(EffectBrushModel {
                    scene: TestBrush,
                    model: 2,
                }),
            },
        );
        let explosion = harness.q1_effect_event("q1:test:map:1", 1, 1.0, Q1Effect::Explosion);
        let mut local = harness.q1_effect_event("q1:test:map:1", 2, 1.0, Q1Effect::Blood);
        local.recipient = Some(viewer.clone());
        let brush = harness.event(
            "q1:test:map:1",
            3,
            1.0,
            ApplicationEffectEventKind::Q1(Q1Event::StaticModel {
                path: "maps/brush.bsp".to_string(),
                frame: 0,
                color_map: 0,
                skin: 0,
                origin: vec3(0.0, 0.0, 0.0),
                angles: vec3(0.0, 0.0, 0.0),
            }),
        );
        let character = harness.event(
            "q3:test:map:1",
            4,
            1.0,
            ApplicationEffectEventKind::Q3Character(Q3CharacterPresentationEvent {
                actor: viewer.clone(),
                sequence: 1,
                time_milliseconds: 1000,
                event: 0,
                parameter: 0,
            }),
        );
        harness
            .effects
            .receive(&[explosion, local, brush, character])
            .expect("receive");
        let presentation = harness.presentation("q1:test:map:1", GameFamily::Q1, viewer.clone());
        let snapshot = harness.snapshot(1.0, vec![viewer.clone(), other]);
        harness
            .effects
            .prepare(&snapshot, &[presentation], &[], None)
            .expect("prepare");
        assert_eq!(harness.q3.state.borrow().characters, 1);
        let camera = test_camera();
        let order = create_source_scene_order(Vec::new());
        let frame = harness.effects.frame(&camera, &order, None, None).expect("frame");
        // Brushes submit before particle batches.
        assert!(matches!(
            frame.operations.first(),
            Some(SceneOperation::Operation(RenderOperation::Draw(_)))
        ));
        assert!(
            frame.operations.iter().any(|operation| matches!(
                operation,
                SceneOperation::Group(group)
                    if matches!(
                        &group.order,
                        SceneGroupOrder::Sequence {
                            phase: SequencePhase::Translucent
                        }
                    )
            )),
            "explosion particle batch missing"
        );
        // Explosion light samples at full radius on its birth second.
        assert_eq!(frame.lights.len(), 1);
        assert_eq!(frame.lights[0].radius, 350.0);
        assert_eq!(frame.lights[0].minimum, 0.0);
        // Q3 admission merges; sampled lights mirror into Q3 lights.
        assert_eq!(frame.q3_admissions.len(), 1);
        assert_eq!(frame.q3_lights.len(), 1);
        assert_eq!(frame.q3_lights[0].radius, 350.0);
        // The recipient-local blood batch only joins the viewer's frame.
        let local_frame = harness
            .effects
            .frame(&camera, &order, Some(&viewer), None)
            .expect("local frame");
        assert_eq!(local_frame.lights.len(), 1);
        assert!(local_frame.operations.len() > frame.operations.len());
        assert_eq!(harness.q3.state.borrow().frames, 2);
        assert_eq!(harness.host.state.borrow().brushes, 2);
        assert!(harness
            .host
            .state
            .borrow()
            .staged
            .iter()
            .any(|(id, _)| id == &content("q1:test:map:1")));
        harness.effects.close();
        // Both the main world and the recipient world registered (and
        // released) their own source-particle image.
        assert_eq!(harness.host.state.borrow().released, 2);
    }

    #[test]
    fn muzzle_flash_lights_and_pickup_blends_the_view() {
        let mut harness = Harness::new();
        harness.with_family("q1:test:map:1", GameFamily::Q1);
        harness.with_family("q2:test:map:1", GameFamily::Q2);
        let player = actor(1);
        let presentation = harness.presentation("q2:test:map:1", GameFamily::Q2, player.clone());
        let muzzle = harness.event(
            "q2:test:map:1",
            1,
            1.0,
            ApplicationEffectEventKind::Q2Weapon(Q2WeaponEvent::Muzzleflash {
                actor: player.clone(),
                flash: 7,
                silenced: false,
            }),
        );
        let pickup = harness.event(
            "q1:test:map:1",
            2,
            1.0,
            ApplicationEffectEventKind::Q1(Q1Event::Effect {
                effect: Q1Effect::Pickup,
                actor: Some(player.clone()),
                origin: vec3(0.0, 0.0, 0.0),
                amount: 0,
                muzzle: None,
            }),
        );
        harness.effects.receive(&[muzzle, pickup]).expect("receive");
        let snapshot = harness.snapshot(1.0, vec![player.clone()]);
        harness
            .effects
            .prepare(&snapshot, &[presentation], &[], None)
            .expect("prepare");
        let camera = test_camera();
        let order = create_source_scene_order(Vec::new());
        let frame = harness.effects.frame(&camera, &order, None, None).expect("frame");
        assert_eq!(frame.lights.len(), 1);
        let light = &frame.lights[0];
        assert!(light.radius >= 200.0 && light.radius <= 231.0);
        assert_eq!(light.color, vec3(1.0, 0.5, 0.2));
        assert_eq!(light.minimum, 32.0);
        let view = harness.effects.player_view(&player, &camera);
        assert!(!view.infrared);
        assert!(view.blend.is_some());
    }

    #[test]
    fn retired_owners_drop_pending_and_future_events() {
        let mut harness = Harness::new();
        harness.with_family("q1:test:map:1", GameFamily::Q1);
        let player = actor(1);
        let owner = PresentationOwner {
            provider: ProviderId::new("test", "world"),
            generation: 2,
        };
        let mut pending = harness.q1_effect_event("q1:test:map:1", 1, 1.0, Q1Effect::Explosion);
        pending.owner = Some(owner.clone());
        let retire = harness.event(
            "q1:test:map:1",
            2,
            1.0,
            ApplicationEffectEventKind::PresentationOwner(PresentationOwnerEvent {
                owner: owner.clone(),
                kind: PresentationOwnerEventKind::Retired,
            }),
        );
        let mut late = harness.q1_effect_event("q1:test:map:1", 3, 1.0, Q1Effect::Explosion);
        late.owner = Some(owner.clone());
        harness.effects.receive(&[pending, retire, late]).expect("receive");
        let snapshot = harness.snapshot(1.0, vec![player]);
        harness.effects.prepare(&snapshot, &[], &[], None).expect("prepare");
        assert!(harness.effects.drain_sounds().is_empty());
    }

    #[test]
    fn closed_worlds_and_rewound_time_fail() {
        let mut harness = Harness::new();
        let player = actor(1);
        let first = harness.snapshot(2.0, vec![player.clone()]);
        harness.effects.prepare(&first, &[], &[], None).expect("prepare");
        let rewind = harness.snapshot(1.0, vec![player]);
        assert!(matches!(
            harness.effects.prepare(&rewind, &[], &[], None),
            Err(ApplicationEffectError::TimeRewound)
        ));
        harness.effects.close();
        assert!(matches!(
            harness.effects.receive(&[]),
            Err(ApplicationEffectError::Closed)
        ));
        let snapshot = harness.snapshot(3.0, Vec::new());
        assert!(matches!(
            harness.effects.prepare(&snapshot, &[], &[], None),
            Err(ApplicationEffectError::Closed)
        ));
        assert!(matches!(
            harness.effects.reset_round(),
            Err(ApplicationEffectError::Closed)
        ));
        harness.effects.close();
    }

    #[test]
    fn reset_round_clears_staged_state() {
        let mut harness = Harness::new();
        harness.with_family("q1:test:map:1", GameFamily::Q1);
        let player = actor(1);
        let explosion = harness.q1_effect_event("q1:test:map:1", 1, 2.0, Q1Effect::Explosion);
        harness.effects.receive(&[explosion]).expect("receive");
        let snapshot = harness.snapshot(2.0, vec![player.clone()]);
        harness.effects.prepare(&snapshot, &[], &[], None).expect("prepare");
        assert_eq!(harness.effects.drain_sounds().len(), 1);
        harness.effects.reset_round().expect("reset");
        // Time rewinds freely after a reset.
        let earlier = harness.snapshot(0.5, vec![player]);
        harness
            .effects
            .prepare(&earlier, &[], &[], None)
            .expect("prepare after reset");
        let camera = test_camera();
        let order = create_source_scene_order(Vec::new());
        let frame = harness.effects.frame(&camera, &order, None, None).expect("frame");
        assert!(frame.lights.is_empty());
        assert!(frame.operations.is_empty());
    }

    #[test]
    fn preload_transient_resources_covers_q3_and_q2() {
        let mut harness = Harness::new();
        let map = content("q1:test:map:1");
        let character = content("q3:test:char:1");
        let weapon = content("q3:test:weapon:1");
        let rogue = content("q2:test:map:1");
        harness.with_family("q1:test:map:1", GameFamily::Q1);
        harness.with_family("q3:test:char:1", GameFamily::Q3);
        harness.with_family("q3:test:weapon:1", GameFamily::Q3);
        harness.with_family("q2:test:map:1", GameFamily::Q2);
        harness.host.state.borrow_mut().plan = Some(EffectPreloadPlan {
            character: character.clone(),
            map_entities: map,
            weapons: vec![weapon.clone()],
            extras: vec![rogue.clone()],
        });
        harness
            .host
            .state
            .borrow_mut()
            .mounts
            .insert((rogue.clone(), Q2_TRANSIENT_MODELS.cable.to_string()));
        let failures = harness.effects.preload_transient_resources().expect("preload");
        assert!(failures.is_empty());
        assert_eq!(
            harness.host.state.borrow().preloaded,
            vec![(rogue, Q2_TRANSIENT_MODELS.cable.to_string())]
        );
        assert!(harness.effects.q3.contains_key(&character));
        assert!(harness.effects.prepared_q3_weapons.contains_key(&weapon));
        // A ballistic hit promotes the prepared entry.
        let ballistic = harness.ballistic_event("q3:test:weapon:1", 1, 1.0, Q3BallisticEventKind::Remove);
        harness.effects.receive(&[ballistic]).expect("receive");
        let snapshot = harness.snapshot(1.0, vec![actor(1)]);
        let clock = EffectWeaponClock {
            content: weapon.clone(),
            time_milliseconds: 1000,
        };
        harness
            .effects
            .prepare(&snapshot, &[], &[], Some(&clock))
            .expect("prepare");
        assert!(harness.effects.prepared_q3_weapons.is_empty());
        assert!(harness.effects.q3_weapons.contains_key(&weapon));
        assert_eq!(harness.q3.state.borrow().ballistics, 1);
        assert_eq!(harness.q3.state.borrow().prepared, vec![(1000, 0), (1000, 0)]);
    }

    #[test]
    fn dynamic_lights_feed_shadow_scene_lights() {
        let mut harness = Harness::new();
        harness.with_family("q2:test:map:1", GameFamily::Q2);
        let player = actor(1);
        let presentation = harness.presentation("q2:test:map:1", GameFamily::Q2, player.clone());
        let flashlight = harness.event(
            "q2:test:map:1",
            1,
            1.0,
            ApplicationEffectEventKind::Q2Rerelease(Q2RereleaseEvent::Flashlight {
                actor: player.clone(),
                enabled: true,
                hand: Q2PlayerHand::Left,
            }),
        );
        harness.effects.receive(&[flashlight]).expect("receive");
        let snapshot = harness.snapshot(1.0, vec![player.clone()]);
        harness
            .effects
            .prepare(&snapshot, &[presentation], &[], None)
            .expect("prepare");
        let camera = test_camera();
        let lights = harness.effects.shadow_scene_lights(&camera, &|_| 1.0, Some(&player));
        assert_eq!(lights.len(), 1);
        assert_eq!(lights[0].radius, 512.0);
        assert!(lights[0].additive);
        // Local view offsets the left-hand light along the camera left axis.
        assert_eq!(lights[0].origin, vec3(0.0, 7.0, 64.0));
    }
}
