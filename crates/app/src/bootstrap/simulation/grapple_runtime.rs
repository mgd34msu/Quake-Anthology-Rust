//! Grapple equipment runtime: input edges and tether state over one source game.
//!
//! Port of Quake-Anthology-TS `src/app/bootstrap/simulation/grapple-runtime.ts`
//! (`GrappleRuntime`, `GrappleRuntimeCheckpoint`, `GrappleSlotHost`).
//!
//! Adaptations from the donor object graph:
//! - The runtime owns its dedicated source arena (`runtime.ts` builds a fresh
//!   `Q1EntityServices`/`Q2EntityServices` per grapple); the session borrows
//!   nothing. Q1 threewave behavior arrives as content free functions over
//!   the owned game, and Q2 weapons as per-call adapters over runtime-owned
//!   animation state.
//! - `EquipmentWeaponHandoff` is a `'static` trait object owned by the weapon
//!   slot, so the runtime shares its game, cores, and tables with handoffs
//!   through `Rc<RefCell<..>>` (the `q3_ballistics` precedent). Every access
//!   uses dynamic borrows and one borrow spans each public operation, so a
//!   reentrant session callback panics instead of aliasing. Session hosts
//!   must not call back into the runtime.
//! - The content `GrappleHooks`/`GrappleWeaponPresentation` hooks are bare
//!   `fn` pointers, so equipment hooks derive session state through the game
//!   host (view state, combat, gravity) and default the rest; see
//!   [`equipment_grapple_hooks`]. Fired view kicks land in a pending slot the
//!   slot step drains into the firing animation.
//! - Q2 hook prediction suppression is a runtime latch the session polls via
//!   [`GrappleRuntime::prediction`]; the content hook has no write-back
//!   channel, so the runtime re-derives the latch from the pulling state
//!   after each CTF step and clears it on release.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::Mutex;

use qa_content::contract::{
    GrappleBinding, GrappleMechanicDetail, GrappleSelection, QvmGrappleViewAnchor, QvmGrappleViewAttachment,
    SourceEdition,
};
use qa_content::q1::equipment::grapple::{
    grapple_fire, grapple_hook, grapple_pulling, grapple_release, grapple_state, grapple_trail,
};
use qa_content::q1::equipment::weapon::{
    weapon_animate, weapon_attack, weapon_holster, weapon_is_holstered, weapon_resume,
};
use qa_content::q1::foundation::entity_services::Q1EntityServices;
use qa_content::q1::Q1Error;
use qa_content::q2::equipment::ctf_grapple::{ctf_grapple_callbacks, Q2CtfGrappleEquipment};
use qa_content::q2::equipment::grapple_services::{
    capture_ctf_grapple, capture_lmctf_grapple, restore_ctf_grapple, restore_lmctf_grapple, CtfGrappleCheckpoint,
    CtfGrapplePhase, GrappleAnchor, GrappleCableEvent, GrappleHand, GrappleHooks, GrappleNoise, GrapplePose,
    LmctfGrappleCheckpoint,
};
use qa_content::q2::equipment::grapple_weapon::{
    create_grapple_weapon_state, GrappleStepInput, GrappleWeaponPresentation, GrappleWeaponSource, GrappleWeaponState,
    Q2GrappleWeapon,
};
use qa_content::q2::equipment::lmctf_grapple::{lmctf_grapple_callbacks, LmctfGrappleEquipment};
use qa_content::q2::foundation::host::{Q2Edition, Q2GameServices};
use qa_content::q2::foundation::weapons::generic_frame::Q2GenericFrameState;
use qa_core::identity::{ActorId, OwnedActor, SavedActorId};
use qa_core::math::{vec3, Vec3};
use qa_guest::checkpoint::GuestCheckpoint;
use qa_world::WorldError;
use thiserror::Error;

use super::equipment_runtime::Q2FoundationCheckpointBridge;
use super::random::{RandomCheckpoint, SourceRandom};
use super::weapon_slot::{EquipmentWeaponHandoff, WeaponReference};
use crate::persistence::q1::foundation::Q1FoundationCheckpoint as Q1FoundationSave;
use crate::persistence::q2::foundation::Q2FoundationCheckpoint as Q2FoundationSave;

/// Checkpoint version for [`GrappleRuntimeCheckpoint`].
pub(crate) const CHECKPOINT_VERSION: u8 = 2;

/// Q2 standing eye height, the equipment pose default (donor
/// `src/content/q2/base/player/character.ts` resets `viewHeight` to 22; the
/// session-supplied ducked/corpse heights cannot reach `fn` hooks).
const Q2_STANDING_VIEW_HEIGHT: f64 = 22.0;

/// Input and tether bookkeeping for one admitted actor.
#[derive(Debug, Clone, PartialEq)]
struct GrappleControl {
    teleport_bit: Option<u8>,
    jump: bool,
    held: bool,
    pressed: bool,
    released: bool,
    previous_velocity: Vec3,
    prediction_suppressed: bool,
}

impl GrappleControl {
    fn idle() -> Self {
        Self {
            teleport_bit: None,
            jump: false,
            held: false,
            pressed: false,
            released: false,
            previous_velocity: vec3(0.0, 0.0, 0.0),
            prediction_suppressed: false,
        }
    }
}

/// One saved input record.
#[derive(Debug, Clone, PartialEq)]
pub struct GrappleControlEntry {
    /// Owning actor.
    pub actor: SavedActorId,
    /// Last observed teleport bit (only 0 or 4 persist, per the donor).
    pub teleport_bit: Option<u8>,
    /// Jump flag.
    pub jump: bool,
    /// Held flag.
    pub held: bool,
    /// Latched press.
    pub pressed: bool,
    /// Latched release.
    pub released: bool,
    /// Previous velocity.
    pub previous_velocity: Vec3,
    /// Prediction suppression.
    pub prediction_suppressed: bool,
}

/// Q2 slot weapon animation for one admitted actor.
#[derive(Debug, Clone, PartialEq)]
pub struct GrappleWeaponAnimation {
    /// Weapon state.
    pub state: GrappleWeaponState,
    /// Next frame time.
    pub next_frame_at: f64,
    /// Last kick origin.
    pub kick_origin: Vec3,
    /// Last kick pitch.
    pub kick_pitch: f64,
}

/// One saved weapon animation record.
#[derive(Debug, Clone, PartialEq)]
pub struct GrappleWeaponAnimationEntry {
    /// Owning actor.
    pub actor: SavedActorId,
    /// Weapon state.
    pub state: GrappleWeaponState,
    /// Next frame time.
    pub next_frame_at: f64,
    /// Last kick origin.
    pub kick_origin: Vec3,
    /// Last kick pitch.
    pub kick_pitch: f64,
}

/// One saved CTF grapple state record.
#[derive(Debug, Clone, PartialEq)]
pub struct CtfGrappleStateEntry {
    /// Owning actor.
    pub actor: SavedActorId,
    /// Grapple state.
    pub state: CtfGrappleCheckpoint,
}

/// One saved LMCTF grapple state record.
#[derive(Debug, Clone, PartialEq)]
pub struct LmctfGrappleStateEntry {
    /// Owning actor.
    pub actor: SavedActorId,
    /// Grapple state.
    pub state: LmctfGrappleCheckpoint,
}

/// Mirror of `QvmGrappleCheckpoint` from donor `src/compat/qvm/grapple-provider.ts`
/// (canonical home: `simulation::qvm_grapple_source`); unify post-merge.
#[derive(Debug, Clone, PartialEq)]
pub struct QvmGrappleCheckpoint {
    /// Checkpoint version (always 1).
    pub version: u8,
    /// Owning profile name.
    pub profile: String,
    /// QVM continuation.
    pub module: GuestCheckpoint,
    /// Hook owners.
    pub owners: Vec<SavedActorId>,
}

/// Mirror of one `QvmGrappleSourceCheckpoint["bindings"]` entry from donor
/// `src/app/bootstrap/simulation/qvm-grapple-source.ts`
/// (canonical home: `simulation::qvm_grapple_source`); unify post-merge.
#[derive(Debug, Clone, PartialEq)]
pub struct QvmGrappleBindingCheckpoint {
    /// Bound actor.
    pub actor: SavedActorId,
    /// Entity pointer.
    pub pointer: i64,
    /// Client binding.
    pub client: bool,
    /// Mirrored origin.
    pub origin: Vec3,
}

/// Mirror of one `QvmGrappleSourceCheckpoint["tethers"]` entry from donor
/// `src/app/bootstrap/simulation/qvm-grapple-source.ts`
/// (canonical home: `simulation::qvm_grapple_source`); unify post-merge.
#[derive(Debug, Clone, PartialEq)]
pub struct QvmGrappleTetherCheckpoint {
    /// Tether owner.
    pub owner: SavedActorId,
    /// Hook actor.
    pub actor: SavedActorId,
}

/// Mirror of `QvmGrappleSourceCheckpoint` from donor
/// `src/app/bootstrap/simulation/qvm-grapple-source.ts`
/// (canonical home: `simulation::qvm_grapple_source`); unify post-merge.
#[derive(Debug, Clone, PartialEq)]
pub struct QvmGrappleSourceCheckpoint {
    /// Checkpoint version (always 1).
    pub version: u8,
    /// Provider checkpoint.
    pub grapple: QvmGrappleCheckpoint,
    /// Actor bindings.
    pub bindings: Vec<QvmGrappleBindingCheckpoint>,
    /// Live tethers.
    pub tethers: Vec<QvmGrappleTetherCheckpoint>,
}

/// Mirror of the `q3Weapon` half of the donor `QvmGrappleSource.weaponView`
/// result (canonical home: `simulation::qvm_grapple_source`); unify post-merge.
#[derive(Debug, Clone, PartialEq)]
pub struct QvmGrappleQ3Weapon {
    /// Source time in milliseconds.
    pub time_ms: i64,
    /// Torso animation.
    pub torso_animation: i32,
    /// Last fire time (always none: the hook never fires the view weapon).
    pub last_fire_ms: Option<i64>,
    /// Hook is out.
    pub firing: bool,
    /// Horizontal speed.
    pub horizontal_speed: f64,
    /// Bob cycle.
    pub bob_cycle: i32,
    /// Weapon index.
    pub weapon: f64,
}

/// Mirror of the donor `QvmGrappleSource.weaponView` result
/// (canonical home: `simulation::qvm_grapple_source`); unify post-merge.
#[derive(Debug, Clone, PartialEq)]
pub struct QvmGrappleWeaponView {
    /// View model path.
    pub path: String,
    /// Frame.
    pub frame: i32,
    /// Kick origin.
    pub kick_origin: Vec3,
    /// Kick pitch.
    pub kick_pitch: f64,
    /// View attachments.
    pub model_attachments: Vec<QvmGrappleViewAttachment>,
    /// View anchor.
    pub model_anchor: QvmGrappleViewAnchor,
    /// Q3 weapon state.
    pub q3_weapon: QvmGrappleQ3Weapon,
}

/// Source half of the checkpoint.
#[derive(Debug, Clone, PartialEq)]
pub enum GrappleSourceCheckpoint {
    /// Threewave entities.
    Q1Threewave {
        /// Foundation entities in the persistence shape.
        entities: Q1FoundationSave,
    },
    /// CTF entities plus hook states.
    Q2Ctf {
        /// Foundation entities in the persistence shape.
        entities: Q2FoundationSave,
        /// Hook states.
        states: Vec<CtfGrappleStateEntry>,
    },
    /// LMCTF entities plus hook states.
    Q2Lmctf {
        /// Foundation entities in the persistence shape.
        entities: Q2FoundationSave,
        /// Hook states.
        states: Vec<LmctfGrappleStateEntry>,
    },
    /// QVM component plus holstered owners.
    Q3Qvm {
        /// Source checkpoint.
        component: QvmGrappleSourceCheckpoint,
        /// Holstered owners.
        holstered: Vec<SavedActorId>,
    },
}

/// Grapple runtime checkpoint (`GrappleRuntimeCheckpoint`).
#[derive(Debug, Clone, PartialEq)]
pub struct GrappleRuntimeCheckpoint {
    /// Checkpoint version (always 2).
    pub version: u8,
    /// Q2 slot animations.
    pub weapon_animations: Vec<GrappleWeaponAnimationEntry>,
    /// Per-actor input records.
    pub controls: Vec<GrappleControlEntry>,
    /// Source RNG stream.
    pub random: RandomCheckpoint,
    /// Source continuations.
    pub source: GrappleSourceCheckpoint,
}

/// View-model presentation for one actor (`GrappleRuntime.weaponView`).
#[derive(Debug, Clone, PartialEq)]
pub enum GrappleWeaponView {
    /// Q1/Q2 view.
    Standard {
        /// View model path.
        path: String,
        /// Frame.
        frame: i32,
        /// Kick origin.
        kick_origin: Vec3,
        /// Kick pitch.
        kick_pitch: f64,
    },
    /// QVM view.
    Qvm(QvmGrappleWeaponView),
}

/// Value seam: QVM grapple game behavior (donor `QvmGrappleSource` in
/// `src/app/bootstrap/simulation/qvm-grapple-source.ts`; canonical home:
/// `simulation::qvm_grapple_source`); unify post-merge.
pub trait QvmGrappleGame {
    /// Admit a source client.
    fn admit(&mut self, actor: &ActorId) -> Result<(), GrappleError>;
    /// Release an owner's hook.
    fn release(&mut self, actor: &ActorId);
    /// Fire an owner's hook.
    fn fire(&mut self, actor: &ActorId);
    /// Live hook, if any.
    fn hook(&self, actor: &ActorId) -> Option<ActorId>;
    /// Whether an owner is pulling.
    fn pulling(&self, actor: &ActorId) -> bool;
    /// View-model presentation.
    fn weapon_view(&self, actor: &ActorId) -> Result<QvmGrappleWeaponView, GrappleError>;
    /// Capture the source checkpoint.
    fn capture(&self) -> QvmGrappleSourceCheckpoint;
    /// Restore the source checkpoint.
    fn restore(&mut self, checkpoint: &QvmGrappleSourceCheckpoint) -> Result<(), GrappleError>;
    /// Whether an actor is live.
    fn host_is_live(&self, actor: &ActorId) -> bool;
    /// Read an actor body.
    fn host_body(&self, actor: &ActorId) -> Option<qa_world::body::BodyState>;
    /// Resolve a saved actor reference.
    fn host_resolve_saved(&self, saved: SavedActorId) -> Option<OwnedActor>;
    /// Reference a saved actor in the current session.
    fn host_reference_saved(&mut self, saved: SavedActorId) -> ActorId;
    /// Release QVM resources (donor `game.close`).
    fn close(&mut self);
}

/// Value seam: QVM grapple core behavior (donor `QvmGrappleProvider` in
/// `src/compat/qvm/grapple-provider.ts`; canonical home:
/// `simulation::qvm_grapple_source`); unify post-merge.
pub trait QvmGrappleCore {
    /// Release an owner's hook, forcing through a dead owner when set.
    fn release(&mut self, actor: &ActorId, force: bool);
}

/// Value seam: persistence-shaped Q1 foundation entities in and out of the
/// live arena (donor `Q1EntityServices.capture`/`restore` in
/// `src/content/q1/foundation/entity-services.ts` over the
/// `src/persistence/q1-foundation.ts` record; canonical home: the Q1 session
/// partition); unify post-merge.
pub trait Q1FoundationCheckpointBridge {
    /// Capture the arena entities into the persistence shape.
    fn capture_entities(&self, game: &Q1EntityServices) -> Q1FoundationSave;
    /// Restore a persisted entities snapshot into the arena.
    fn restore_entities(
        &self,
        game: &mut Q1EntityServices,
        checkpoint: &Q1FoundationSave,
        schedule_thinks: bool,
    ) -> Result<(), WorldError>;
}

/// Foundation bridge matching the selected source kind.
pub enum GrappleFoundationBridge {
    /// Q1 bridge.
    Q1(Box<dyn Q1FoundationCheckpointBridge>),
    /// Q2 bridge.
    Q2(Box<dyn Q2FoundationCheckpointBridge>),
    /// QVM sources carry no foundation entities.
    Qvm,
}

/// Session slot presentation minus the runtime-recorded kick (donor
/// `Omit<GrappleWeaponPresentation, "kick">`).
#[derive(Debug, Clone, Copy)]
pub struct GrappleSlotPresentation {
    /// Play the attack animation.
    pub attack_animation: fn(),
    /// Play the reverse animation.
    pub reverse_animation: fn(),
    /// Play the powerup sound.
    pub powerup_sound: fn(),
    /// Read the animation frame time.
    pub animation_time: fn(&Q2GenericFrameState) -> f64,
}

/// Session weapon-slot surface for the selected grapple (`GrappleSlotHost`).
pub trait GrappleSlotHost {
    /// Whether the grapple slot is selected.
    fn selected(&self, actor: &ActorId) -> bool;
    /// Whether the grapple slot is available.
    fn available(&self, actor: &ActorId) -> bool;
    /// Show a weapon frame.
    fn frame(&self, actor: &ActorId, frame: i32);
    /// Read the slot presentation.
    fn presentation(&self, actor: &ActorId) -> GrappleSlotPresentation;
}

/// Grapple equipment failure.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum GrappleError {
    /// Selection disables the grapple.
    #[error("Grapple equipment is disabled")]
    Disabled,
    /// Selected slot binding has no source weapon host.
    #[error("Selected grapple slot needs its source weapon host")]
    MissingSlotHost,
    /// Foundation bridge differs from the selected source.
    #[error("Grapple foundation bridge differs from selected equipment")]
    BridgeMismatch,
    /// Actor has no live body.
    #[error("Grapple needs an admitted actor and body")]
    UnknownActor,
    /// Offhand grapple owns no weapon slot.
    #[error("Offhand grapple has no weapon slot")]
    NoWeaponSlot,
    /// Q2 animation owner is missing.
    #[error("Missing grapple weapon animation owner")]
    MissingWeapon,
    /// Q2 animation needs its selected slot host.
    #[error("Q2 grapple animation requires its selected slot host")]
    AnimationBinding,
    /// Q2 slot animation is missing.
    #[error("Missing Q2 slot animation")]
    MissingSlotStep,
    /// Actor was never admitted.
    #[error("Actor has no selected offhand grapple")]
    MissingControls,
    /// Saved source differs from the selected equipment.
    #[error("Saved grapple source differs from selected equipment")]
    SourceMismatch,
    /// Saved input owner is missing, stale, or duplicated.
    #[error("Invalid saved grapple input owner")]
    InvalidControlsOwner,
    /// QVM source failure.
    #[error("QVM grapple failure: {0}")]
    Qvm(String),
    /// Threewave failure.
    #[error("Threewave grapple failure: {0}")]
    Q1(String),
    /// Source RNG restore failed.
    #[error("Grapple random restore failed")]
    Random,
    /// Foundation entities restore failed.
    #[error("Grapple entities restore failed: {0}")]
    Foundation(String),
}

impl From<Q1Error> for GrappleError {
    fn from(error: Q1Error) -> Self {
        GrappleError::Q1(error.to_string())
    }
}

/// Pending view kick from the last weapon fire, drained by the slot step.
static PENDING_KICK: Mutex<Option<(Vec3, f64)>> = Mutex::new(None);

/// Record a fired view kick for the firing animation.
fn record_kick(origin: Vec3, pitch: f64) {
    *PENDING_KICK.lock().unwrap_or_else(|error| error.into_inner()) = Some((origin, pitch));
}

/// Take the pending view kick, if the last step fired.
fn take_kick() -> Option<(Vec3, f64)> {
    PENDING_KICK.lock().unwrap_or_else(|error| error.into_inner()).take()
}

fn hook_pose(actor: ActorId, game: &mut Q2GameServices) -> GrapplePose {
    let angles = game
        .host
        .player_view_state(&actor)
        .map(|view| view.view_angles)
        .or_else(|| game.host.bodies().read(&actor).map(|body| body.angles))
        .unwrap_or_else(|| vec3(0.0, 0.0, 0.0));
    GrapplePose {
        angles,
        hand: GrappleHand::Right,
        view_height: Q2_STANDING_VIEW_HEIGHT,
        gravity: 1.0,
        gravity_vector: vec3(0.0, 0.0, -1.0),
    }
}

fn hook_anchor(actor: ActorId, game: &mut Q2GameServices) -> GrappleAnchor {
    if actor == game.host.world_actor() {
        return GrappleAnchor::World;
    }
    if game.host.is_player(&actor) {
        GrappleAnchor::Player
    } else {
        // Corpse/box/brush distinctions need session scene queries, which
        // cannot reach `fn` hooks.
        GrappleAnchor::None
    }
}

fn hook_dead(actor: ActorId, game: &mut Q2GameServices) -> bool {
    game.host
        .combat()
        .read(&actor)
        .is_some_and(|combat| combat.can_take_damage && combat.health <= 0.0)
}

fn hook_previous_velocity(actor: ActorId, game: &mut Q2GameServices) -> Vec3 {
    game.host
        .player_view_state(&actor)
        .map(|view| view.old_velocity)
        .unwrap_or_else(|| vec3(0.0, 0.0, 0.0))
}

fn hook_set_previous_velocity(_actor: ActorId, _velocity: Vec3, _game: &mut Q2GameServices) {
    // No host write-back channel: the session re-primes view-state velocity
    // every frame, so only mid-step writes are lost.
}

fn hook_volume(_actor: ActorId, _game: &mut Q2GameServices) -> f64 {
    // The session silencer check cannot reach `fn` hooks.
    1.0
}

fn hook_noise(_actor: ActorId, _game: &mut Q2GameServices, _origin: Vec3, _kind: GrappleNoise) {
    // The equipment arena has no listeners; the session propagates its own noise.
}

fn hook_set_prediction(_actor: ActorId, _suppressed: bool, _game: &mut Q2GameServices) {
    // The runtime re-derives the latch from the pulling state after each step.
}

fn hook_gravity(game: &mut Q2GameServices) -> f64 {
    game.host.gravity()
}

fn hook_emit(_event: GrappleCableEvent, _game: &mut Q2GameServices) {
    // Q2PresentationEvent carries no cable channel.
}

/// Build the equipment grapple hooks.
///
/// Each hook derives session state through the game host where the host
/// exposes it (view state, combat, gravity) and otherwise falls back to the
/// donor's session-independent defaults; see the hook bodies for the exact
/// per-hook derivation. Build the Q2 cores with these hooks and pass the
/// cores back into [`GrappleRuntime::new`].
pub fn equipment_grapple_hooks() -> GrappleHooks {
    GrappleHooks {
        pose: hook_pose,
        anchor: hook_anchor,
        dead: hook_dead,
        previous_velocity: hook_previous_velocity,
        set_previous_velocity: hook_set_previous_velocity,
        volume: hook_volume,
        noise: hook_noise,
        set_grapple_prediction: hook_set_prediction,
        gravity: hook_gravity,
        emit: hook_emit,
    }
}

/// Dedicated source arena plus its equipment core (`GrappleSource`).
pub enum GrappleSource {
    /// Threewave arena; the grapple service lives in the game.
    Q1Threewave {
        /// Arena.
        game: Q1EntityServices,
    },
    /// CTF arena plus core.
    Q2Ctf {
        /// Arena.
        game: Q2GameServices,
        /// Core.
        core: Q2CtfGrappleEquipment,
    },
    /// LMCTF arena plus core.
    Q2Lmctf {
        /// Arena.
        game: Q2GameServices,
        /// Core.
        core: LmctfGrappleEquipment,
    },
    /// QVM game plus core.
    Q3Qvm {
        /// Game.
        game: Box<dyn QvmGrappleGame>,
        /// Core.
        core: Box<dyn QvmGrappleCore>,
    },
}

/// Source kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GrappleKind {
    Q1Threewave,
    Q2Ctf,
    Q2Lmctf,
    Q3Qvm,
}

/// Fresh Q2 slot animation.
fn fresh_animation() -> GrappleWeaponAnimation {
    GrappleWeaponAnimation {
        state: create_grapple_weapon_state(),
        next_frame_at: 0.0,
        kick_origin: vec3(0.0, 0.0, 0.0),
        kick_pitch: 0.0,
    }
}

/// Runtime-owned game, cores, and tables shared with handoffs.
struct GrappleInner {
    source: GrappleSource,
    controls: HashMap<ActorId, GrappleControl>,
    weapon_animations: HashMap<ActorId, GrappleWeaponAnimation>,
    q2_weapons: HashMap<ActorId, Q2GrappleWeapon>,
    q3_holstered: HashSet<ActorId>,
    random: SourceRandom,
}

impl GrappleInner {
    fn kind(&self) -> GrappleKind {
        match &self.source {
            GrappleSource::Q1Threewave { .. } => GrappleKind::Q1Threewave,
            GrappleSource::Q2Ctf { .. } => GrappleKind::Q2Ctf,
            GrappleSource::Q2Lmctf { .. } => GrappleKind::Q2Lmctf,
            GrappleSource::Q3Qvm { .. } => GrappleKind::Q3Qvm,
        }
    }

    fn resolve_saved(&mut self, saved: SavedActorId) -> Option<OwnedActor> {
        match &mut self.source {
            GrappleSource::Q1Threewave { game } => game.host.actors.resolve_saved(&saved),
            GrappleSource::Q2Ctf { game, .. } | GrappleSource::Q2Lmctf { game, .. } => {
                game.host.actors().resolve_saved(saved)
            }
            GrappleSource::Q3Qvm { game, .. } => game.host_resolve_saved(saved),
        }
    }

    fn admitted(&mut self, actor: &ActorId) -> bool {
        match &mut self.source {
            GrappleSource::Q1Threewave { game } => {
                game.host.actors.is_live(actor) && game.host.bodies.read(actor).is_some()
            }
            GrappleSource::Q2Ctf { game, .. } | GrappleSource::Q2Lmctf { game, .. } => {
                game.host.actors().is_live(actor) && game.host.bodies().read(actor).is_some()
            }
            GrappleSource::Q3Qvm { game, .. } => game.host_is_live(actor) && game.host_body(actor).is_some(),
        }
    }

    fn admit_inner(
        &mut self,
        actor: &ActorId,
        slot_binding: bool,
        edition: Q2Edition,
        presentation: Option<GrappleSlotPresentation>,
    ) -> Result<(), GrappleError> {
        if !self.admitted(actor) {
            return Err(GrappleError::UnknownActor);
        }
        if let GrappleSource::Q3Qvm { game, .. } = &mut self.source {
            game.admit(actor)?;
        }
        self.controls.insert(actor.clone(), GrappleControl::idle());
        if !slot_binding {
            return Ok(());
        }
        match self.kind() {
            GrappleKind::Q1Threewave => {
                if let GrappleSource::Q1Threewave { game } = &mut self.source {
                    grapple_state(game, actor)?;
                }
            }
            GrappleKind::Q3Qvm => {
                self.q3_holstered.insert(actor.clone());
            }
            GrappleKind::Q2Ctf | GrappleKind::Q2Lmctf => {
                let presentation = presentation.ok_or(GrappleError::AnimationBinding)?;
                self.bind_weapon(actor, fresh_animation(), edition, presentation)?;
            }
        }
        Ok(())
    }

    fn bind_weapon(
        &mut self,
        actor: &ActorId,
        animation: GrappleWeaponAnimation,
        edition: Q2Edition,
        presentation: GrappleSlotPresentation,
    ) -> Result<(), GrappleError> {
        let source = match &self.source {
            GrappleSource::Q2Ctf { core, .. } => GrappleWeaponSource::Ctf { core: *core, edition },
            GrappleSource::Q2Lmctf { core, .. } => GrappleWeaponSource::Lmctf { core: *core },
            _ => return Err(GrappleError::AnimationBinding),
        };
        self.weapon_animations.insert(actor.clone(), animation);
        self.q2_weapons.insert(
            actor.clone(),
            Q2GrappleWeapon {
                actor: actor.clone(),
                source,
                presentation: GrappleWeaponPresentation {
                    kick: record_kick,
                    attack_animation: presentation.attack_animation,
                    reverse_animation: presentation.reverse_animation,
                    powerup_sound: presentation.powerup_sound,
                    animation_time: presentation.animation_time,
                },
            },
        );
        Ok(())
    }

    fn hook_inner(&mut self, actor: &ActorId) -> Option<ActorId> {
        match &mut self.source {
            GrappleSource::Q1Threewave { game } => grapple_hook(game, actor),
            GrappleSource::Q2Ctf { game, .. } => game
                .equipment
                .ctf_states
                .get(actor)
                .and_then(|state| state.grapple.clone()),
            GrappleSource::Q2Lmctf { game, .. } => game
                .equipment
                .lmctf_states
                .get(actor)
                .and_then(|state| state.hook.clone()),
            GrappleSource::Q3Qvm { game, .. } => game.hook(actor),
        }
    }

    fn pulling_inner(&mut self, actor: &ActorId) -> bool {
        match &mut self.source {
            GrappleSource::Q1Threewave { game } => grapple_pulling(game, actor),
            GrappleSource::Q2Ctf { game, .. } => game
                .equipment
                .ctf_states
                .get(actor)
                .is_some_and(|state| state.grapple_state != CtfGrapplePhase::Fly),
            GrappleSource::Q2Lmctf { game, .. } => game
                .equipment
                .lmctf_states
                .get(actor)
                .is_some_and(|state| state.hook_state == 2),
            GrappleSource::Q3Qvm { game, .. } => game.pulling(actor),
        }
    }

    fn gravity_scale_inner(&mut self, actor: &ActorId) -> i32 {
        match &mut self.source {
            GrappleSource::Q2Lmctf { game, core } => core.gravity_scale(game, actor.clone()),
            _ => 1,
        }
    }

    fn clear_edges(&mut self, actor: &ActorId) {
        if let Some(control) = self.controls.get_mut(actor) {
            control.pressed = false;
            control.released = false;
        }
    }

    /// Re-derive the CTF prediction latch from the pulling state; the content
    /// hook cannot write it back.
    fn sync_prediction(&mut self, actor: &ActorId) {
        if self.kind() != GrappleKind::Q2Ctf {
            return;
        }
        let pulling = self.pulling_inner(actor);
        if let Some(control) = self.controls.get_mut(actor) {
            control.prediction_suppressed = pulling;
        }
    }

    fn release_inner(&mut self, actor: &ActorId) -> Result<(), GrappleError> {
        if let Some(control) = self.controls.get_mut(actor) {
            control.held = false;
            control.pressed = false;
            control.released = false;
            control.prediction_suppressed = false;
        }
        let kind = self.kind();
        if self.hook_inner(actor).is_none() && kind != GrappleKind::Q1Threewave {
            return Ok(());
        }
        match &mut self.source {
            GrappleSource::Q1Threewave { game } => Ok(grapple_release(game, actor)?),
            GrappleSource::Q2Ctf { game, core } => {
                core.reset(actor.clone(), game);
                Ok(())
            }
            GrappleSource::Q2Lmctf { game, core } => {
                core.abort(actor.clone(), game);
                Ok(())
            }
            GrappleSource::Q3Qvm { core, .. } => {
                core.release(actor, true);
                Ok(())
            }
        }
    }

    fn step_inner(
        &mut self,
        actor: &ActorId,
        enabled: bool,
        slot_binding: bool,
        rerelease: bool,
        slot_host: Option<&dyn GrappleSlotHost>,
    ) -> Result<(), GrappleError> {
        let Some(control) = self.controls.get(actor).cloned() else {
            return Ok(());
        };
        if !enabled {
            return self.release_inner(actor);
        }
        if slot_binding {
            let host = slot_host.ok_or(GrappleError::MissingSlotHost)?;
            return self.step_slot_inner(actor, rerelease, host);
        }
        let kind = self.kind();
        if control.released && kind == GrappleKind::Q3Qvm {
            self.clear_edges(actor);
            if let GrappleSource::Q3Qvm { game, .. } = &mut self.source {
                game.release(actor);
            }
            return Ok(());
        }
        if control.released {
            return self.release_inner(actor);
        }
        if !control.held {
            return Ok(());
        }
        self.clear_edges(actor);
        match &mut self.source {
            GrappleSource::Q1Threewave { game } => {
                if control.pressed {
                    grapple_fire(game, actor)?;
                }
                grapple_trail(game, actor);
                Ok(())
            }
            GrappleSource::Q2Ctf { game, core } => {
                if control.pressed {
                    core.offhand(actor.clone(), game, true);
                }
                if !game.host.actors().is_live(actor) {
                    return Ok(());
                }
                core.player_frame(actor.clone(), game, true);
                self.sync_prediction(actor);
                Ok(())
            }
            GrappleSource::Q2Lmctf { game, core } => {
                let hooked = game
                    .equipment
                    .lmctf_states
                    .get(actor)
                    .is_some_and(|grapple| grapple.hook.is_some());
                if control.pressed || hooked {
                    core.fire(actor.clone(), game);
                }
                Ok(())
            }
            GrappleSource::Q3Qvm { game, .. } => {
                if control.pressed {
                    game.fire(actor);
                }
                Ok(())
            }
        }
    }

    fn step_slot_inner(
        &mut self,
        actor: &ActorId,
        rerelease: bool,
        slot_host: &dyn GrappleSlotHost,
    ) -> Result<(), GrappleError> {
        let (pressed, released) = self
            .controls
            .get(actor)
            .map(|control| (control.pressed, control.released))
            .unwrap_or((false, false));
        self.clear_edges(actor);
        match self.kind() {
            GrappleKind::Q3Qvm => {
                if released || !slot_host.selected(actor) {
                    if let GrappleSource::Q3Qvm { game, .. } = &mut self.source {
                        game.release(actor);
                    }
                } else if slot_host.selected(actor)
                    && slot_host.available(actor)
                    && pressed
                    && !self.q3_holstered.contains(actor)
                {
                    if let GrappleSource::Q3Qvm { game, .. } = &mut self.source {
                        game.fire(actor);
                    }
                }
                Ok(())
            }
            GrappleKind::Q1Threewave => {
                if slot_host.selected(actor) && self.controls.get(actor).is_some_and(|control| control.held) {
                    if let GrappleSource::Q1Threewave { game } = &mut self.source {
                        weapon_attack(game, actor)?;
                    }
                }
                if let GrappleSource::Q1Threewave { game } = &mut self.source {
                    weapon_animate(game, actor)?;
                    grapple_trail(game, actor);
                }
                Ok(())
            }
            GrappleKind::Q2Ctf | GrappleKind::Q2Lmctf => {
                let kind = self.kind();
                if pressed && slot_host.selected(actor) {
                    self.weapon_animations
                        .get_mut(actor)
                        .ok_or(GrappleError::MissingSlotStep)?
                        .state
                        .animation
                        .latched_attack = true;
                }
                let (now, frame_seconds) = match &mut self.source {
                    GrappleSource::Q2Ctf { game, .. } | GrappleSource::Q2Lmctf { game, .. } => {
                        (game.host.now(), game.host.frame_seconds())
                    }
                    _ => return Err(GrappleError::MissingSlotStep),
                };
                let held = self.controls.get(actor).is_some_and(|control| control.held);
                let advance = match self.weapon_animations.get(actor) {
                    Some(animation) => rerelease || now + 0.000001 >= animation.next_frame_at,
                    None => return Err(GrappleError::MissingSlotStep),
                };
                if advance {
                    let game = match &mut self.source {
                        GrappleSource::Q2Ctf { game, .. } | GrappleSource::Q2Lmctf { game, .. } => game,
                        _ => return Err(GrappleError::MissingSlotStep),
                    };
                    let animation = self
                        .weapon_animations
                        .get_mut(actor)
                        .ok_or(GrappleError::MissingSlotStep)?;
                    let weapon = self.q2_weapons.get(actor).ok_or(GrappleError::MissingSlotStep)?;
                    animation.next_frame_at = ((animation.next_frame_at + 0.1) * 1000.0).round() / 1000.0;
                    weapon.step(
                        &mut animation.state,
                        game,
                        &GrappleStepInput {
                            attack: held,
                            now,
                            frame_seconds,
                            instant_switch: false,
                            holster: false,
                            weapon_thunk: false,
                            latched_holster: false,
                        },
                    );
                    if let Some((origin, pitch)) = take_kick() {
                        animation.kick_origin = origin;
                        animation.kick_pitch = pitch;
                    }
                }
                let game = match &mut self.source {
                    GrappleSource::Q2Ctf { game, .. } | GrappleSource::Q2Lmctf { game, .. } => game,
                    _ => return Err(GrappleError::MissingSlotStep),
                };
                let animation = self
                    .weapon_animations
                    .get_mut(actor)
                    .ok_or(GrappleError::MissingSlotStep)?;
                let weapon = self.q2_weapons.get(actor).ok_or(GrappleError::MissingSlotStep)?;
                weapon.player_frame(&mut animation.state, game);
                if kind == GrappleKind::Q2Ctf {
                    self.sync_prediction(actor);
                }
                Ok(())
            }
        }
    }

    fn observe_teleport_inner(&mut self, actor: &ActorId, bit: u8) -> Result<(), GrappleError> {
        if !self.controls.contains_key(actor) {
            return Ok(());
        }
        let previous = self.controls.get(actor).and_then(|control| control.teleport_bit);
        if previous.is_some_and(|previous| previous != bit) {
            self.release_inner(actor)?;
        }
        if let Some(control) = self.controls.get_mut(actor) {
            control.teleport_bit = Some(bit);
        }
        Ok(())
    }

    fn released_inner(&mut self, actor: &ActorId) {
        if let (Some(weapon), Some(animation)) = (self.q2_weapons.get(actor), self.weapon_animations.get_mut(actor)) {
            weapon.released(&mut animation.state);
        }
    }

    fn weapon_view_inner(&mut self, actor: &ActorId) -> Option<GrappleWeaponView> {
        match &mut self.source {
            GrappleSource::Q1Threewave { game } => {
                let frame = game
                    .threewave_grapple
                    .as_ref()
                    .and_then(|service| service.states.get(actor))
                    .map_or(0, |state| state.weapon_frame);
                Some(GrappleWeaponView::Standard {
                    path: "progs/v_star.mdl".to_string(),
                    frame,
                    kick_origin: vec3(0.0, 0.0, 0.0),
                    kick_pitch: 0.0,
                })
            }
            GrappleSource::Q2Ctf { .. } | GrappleSource::Q2Lmctf { .. } => {
                let weapon = self.q2_weapons.get(actor)?;
                let animation = self.weapon_animations.get(actor)?;
                Some(GrappleWeaponView::Standard {
                    path: weapon.definition().view_model,
                    frame: animation.state.animation.frame,
                    kick_origin: animation.kick_origin,
                    kick_pitch: animation.kick_pitch,
                })
            }
            GrappleSource::Q3Qvm { game, .. } => game.weapon_view(actor).ok().map(GrappleWeaponView::Qvm),
        }
    }

    fn holster_handoff(&mut self, actor: &ActorId) {
        match self.kind() {
            GrappleKind::Q3Qvm => {
                self.q3_holstered.insert(actor.clone());
                self.release_inner(actor).expect("QVM grapple release is infallible");
            }
            GrappleKind::Q1Threewave => {
                if let GrappleSource::Q1Threewave { game } = &mut self.source {
                    weapon_holster(game, actor).expect("threewave holster owner is admitted");
                }
            }
            GrappleKind::Q2Ctf | GrappleKind::Q2Lmctf => {
                let weapon = self
                    .q2_weapons
                    .get(actor)
                    .cloned()
                    .expect("Missing grapple weapon animation owner");
                let animation = self
                    .weapon_animations
                    .get_mut(actor)
                    .expect("Missing grapple weapon animation owner");
                weapon.holster(&mut animation.state);
            }
        }
    }

    fn is_holstered_inner(&self, actor: &ActorId) -> bool {
        match self.kind() {
            GrappleKind::Q3Qvm => self.q3_holstered.contains(actor),
            GrappleKind::Q1Threewave => weapon_is_holstered(),
            GrappleKind::Q2Ctf | GrappleKind::Q2Lmctf => self
                .q2_weapons
                .get(actor)
                .zip(self.weapon_animations.get(actor))
                .is_some_and(|(weapon, animation)| weapon.is_holstered(&animation.state)),
        }
    }

    fn resume_handoff(&mut self, actor: &ActorId, slot_host: Option<&dyn GrappleSlotHost>) {
        match self.kind() {
            GrappleKind::Q3Qvm => {
                self.q3_holstered.remove(actor);
            }
            GrappleKind::Q1Threewave => {
                if let GrappleSource::Q1Threewave { game } = &mut self.source {
                    weapon_resume(game, actor).expect("threewave resume owner is admitted");
                }
                if let Some(host) = slot_host {
                    host.frame(actor, 0);
                }
            }
            GrappleKind::Q2Ctf | GrappleKind::Q2Lmctf => {
                let weapon = self
                    .q2_weapons
                    .get(actor)
                    .cloned()
                    .expect("Missing grapple weapon animation owner");
                let animation = self
                    .weapon_animations
                    .get_mut(actor)
                    .expect("Missing grapple weapon animation owner");
                weapon.resume(&mut animation.state);
                let now = match &mut self.source {
                    GrappleSource::Q2Ctf { game, .. } | GrappleSource::Q2Lmctf { game, .. } => game.host.now(),
                    _ => unreachable!("resume matches the source kind"),
                };
                animation.next_frame_at = ((now + 0.1) * 1000.0).round() / 1000.0;
            }
        }
    }
}

/// Grapple equipment runtime (`GrappleRuntime`).
///
/// Input and tether state reference the same session actors and body table as
/// the primary weapon. All methods share one dynamic borrow per operation;
/// session hosts must not call back into the runtime.
pub struct GrappleRuntime {
    selection: GrappleSelection,
    slot_host: Option<Rc<dyn GrappleSlotHost>>,
    bridge: GrappleFoundationBridge,
    inner: Rc<RefCell<GrappleInner>>,
}

impl GrappleRuntime {
    /// Create a runtime over an enabled selection and its source.
    ///
    /// The session builds Q2 cores with [`equipment_grapple_hooks`], registers
    /// the threewave service and weapon hooks on Q1 arenas, and shares the
    /// slot host when the binding selects the weapon slot.
    pub fn new(
        selection: GrappleSelection,
        source: GrappleSource,
        random: SourceRandom,
        slot_host: Option<Rc<dyn GrappleSlotHost>>,
        bridge: GrappleFoundationBridge,
    ) -> Result<Self, GrappleError> {
        let GrappleSelection::Enabled { binding, .. } = &selection else {
            return Err(GrappleError::Disabled);
        };
        if *binding == GrappleBinding::Slot && slot_host.is_none() {
            return Err(GrappleError::MissingSlotHost);
        }
        let bridge_ok = matches!(
            (&source, &bridge),
            (GrappleSource::Q1Threewave { .. }, GrappleFoundationBridge::Q1(_))
                | (
                    GrappleSource::Q2Ctf { .. } | GrappleSource::Q2Lmctf { .. },
                    GrappleFoundationBridge::Q2(_)
                )
                | (GrappleSource::Q3Qvm { .. }, GrappleFoundationBridge::Qvm)
        );
        if !bridge_ok {
            return Err(GrappleError::BridgeMismatch);
        }
        let mut inner = GrappleInner {
            source,
            controls: HashMap::new(),
            weapon_animations: HashMap::new(),
            q2_weapons: HashMap::new(),
            q3_holstered: HashSet::new(),
            random,
        };
        match &mut inner.source {
            GrappleSource::Q2Ctf { game, core } => {
                game.source_callbacks.register(&ctf_grapple_callbacks());
                core.bind(game);
            }
            GrappleSource::Q2Lmctf { game, core } => {
                game.source_callbacks.register(&lmctf_grapple_callbacks());
                core.bind(game);
            }
            _ => {}
        }
        Ok(Self {
            selection,
            slot_host,
            bridge,
            inner: Rc::new(RefCell::new(inner)),
        })
    }

    /// The validated selection.
    #[must_use]
    pub fn selection(&self) -> &GrappleSelection {
        &self.selection
    }

    /// Close the QVM source game when the source is QVM-backed (donor
    /// `close` grapple arm; other sources hold no game to release).
    pub fn close_qvm_game(&self) {
        if let GrappleSource::Q3Qvm { game, .. } = &mut self.inner.borrow_mut().source {
            game.close();
        }
    }

    /// Run a closure over the owned source arena (frame setup, think dispatch).
    pub fn with_source<R>(&self, access: impl FnOnce(&mut GrappleSource) -> R) -> R {
        access(&mut self.inner.borrow_mut().source)
    }

    fn slot_binding(&self) -> bool {
        matches!(
            &self.selection,
            GrappleSelection::Enabled {
                binding: GrappleBinding::Slot,
                ..
            }
        )
    }

    fn rerelease(&self) -> bool {
        matches!(
            &self.selection,
            GrappleSelection::Enabled {
                mechanic: GrappleMechanicDetail::Q2Ctf {
                    edition: SourceEdition::Rerelease
                },
                ..
            }
        )
    }

    fn ctf_edition(&self) -> Q2Edition {
        match &self.selection {
            GrappleSelection::Enabled {
                mechanic: GrappleMechanicDetail::Q2Ctf { edition },
                ..
            } => match edition {
                SourceEdition::Classic => Q2Edition::Classic,
                SourceEdition::Rerelease => Q2Edition::Rerelease,
            },
            _ => Q2Edition::Classic,
        }
    }

    /// Admit an actor.
    pub fn admit(&self, actor: &ActorId) -> Result<(), GrappleError> {
        let slot_binding = self.slot_binding();
        let presentation = if slot_binding {
            self.slot_host.as_ref().map(|host| host.presentation(actor))
        } else {
            None
        };
        let edition = self.ctf_edition();
        self.inner
            .borrow_mut()
            .admit_inner(actor, slot_binding, edition, presentation)
    }

    /// Owned weapon reference.
    #[must_use]
    pub fn weapon(&self) -> WeaponReference {
        let (provider, item) = match &self.selection {
            GrappleSelection::Enabled { source, mechanic, .. } => {
                let item = match mechanic {
                    GrappleMechanicDetail::Q1Threewave { .. } => "q1:ctf/weapon/grapple",
                    GrappleMechanicDetail::Q2Ctf { .. } => "q2:weapon_grapple",
                    GrappleMechanicDetail::Q3Qvm { .. } => "q3:weapon_grapplinghook",
                    GrappleMechanicDetail::Q2Lmctf => "q2:weapon_hook",
                };
                (source.provider.clone(), item)
            }
            GrappleSelection::Disabled => {
                unreachable!("GrappleRuntime::new rejects disabled selections")
            }
        };
        WeaponReference {
            provider,
            item: item.to_string(),
        }
    }

    /// Slot-owned weapon handoff for one actor.
    pub fn handoff(&self, actor: &ActorId) -> Result<GrappleHandoff, GrappleError> {
        if !self.slot_binding() {
            return Err(GrappleError::NoWeaponSlot);
        }
        let inner = self.inner.borrow();
        match inner.kind() {
            GrappleKind::Q1Threewave => {
                if self.slot_host.is_none() {
                    return Err(GrappleError::MissingSlotHost);
                }
            }
            GrappleKind::Q2Ctf | GrappleKind::Q2Lmctf => {
                if !inner.q2_weapons.contains_key(actor) {
                    return Err(GrappleError::MissingWeapon);
                }
            }
            GrappleKind::Q3Qvm => {}
        }
        drop(inner);
        Ok(GrappleHandoff {
            weapon: self.weapon(),
            actor: actor.clone(),
            inner: Rc::clone(&self.inner),
            slot_host: self.slot_host.clone(),
        })
    }

    /// View-model presentation for one actor.
    #[must_use]
    pub fn weapon_view(&self, actor: &ActorId) -> Option<GrappleWeaponView> {
        if !self.slot_binding() {
            return None;
        }
        self.inner.borrow_mut().weapon_view_inner(actor)
    }

    /// Notify the Q2 weapon of a release.
    pub fn released(&self, actor: &ActorId) {
        self.inner.borrow_mut().released_inner(actor);
    }

    /// Observe a teleport bit, releasing across teleport changes.
    pub fn observe_teleport(&self, actor: &ActorId, bit: u8) -> Result<(), GrappleError> {
        self.inner.borrow_mut().observe_teleport_inner(actor, bit)
    }

    /// Jump flag.
    #[must_use]
    pub fn jump(&self, actor: &ActorId) -> bool {
        self.inner
            .borrow()
            .controls
            .get(actor)
            .is_some_and(|control| control.jump)
    }

    /// Set the jump flag.
    pub fn set_jump(&self, actor: &ActorId, jump: bool) {
        if let Some(control) = self.inner.borrow_mut().controls.get_mut(actor) {
            control.jump = jump;
        }
    }

    /// Held flag.
    #[must_use]
    pub fn held(&self, actor: &ActorId) -> bool {
        self.inner
            .borrow()
            .controls
            .get(actor)
            .is_some_and(|control| control.held)
    }

    /// Latch the held flag and its edges.
    pub fn input(&self, actor: &ActorId, held: bool) -> Result<(), GrappleError> {
        let mut inner = self.inner.borrow_mut();
        let previous = inner
            .controls
            .get(actor)
            .cloned()
            .ok_or(GrappleError::MissingControls)?;
        inner.controls.insert(
            actor.clone(),
            GrappleControl {
                held,
                pressed: previous.pressed || (held && !previous.held),
                released: previous.released || (!held && previous.held),
                ..previous
            },
        );
        Ok(())
    }

    /// Previous velocity.
    #[must_use]
    pub fn previous_velocity(&self, actor: &ActorId) -> Vec3 {
        self.inner
            .borrow()
            .controls
            .get(actor)
            .map(|control| control.previous_velocity)
            .unwrap_or_else(|| vec3(0.0, 0.0, 0.0))
    }

    /// Set the previous velocity.
    pub fn set_previous_velocity(&self, actor: &ActorId, velocity: Vec3) {
        if let Some(control) = self.inner.borrow_mut().controls.get_mut(actor) {
            control.previous_velocity = velocity;
        }
    }

    /// Prediction suppression.
    #[must_use]
    pub fn prediction(&self, actor: &ActorId) -> bool {
        self.inner
            .borrow()
            .controls
            .get(actor)
            .is_some_and(|control| control.prediction_suppressed)
    }

    /// Set prediction suppression.
    pub fn set_prediction(&self, actor: &ActorId, suppressed: bool) {
        if let Some(control) = self.inner.borrow_mut().controls.get_mut(actor) {
            control.prediction_suppressed = suppressed;
        }
    }

    /// Live hook, if any.
    #[must_use]
    pub fn hook(&self, actor: &ActorId) -> Option<ActorId> {
        self.inner.borrow_mut().hook_inner(actor)
    }

    /// Whether an owner is pulling.
    #[must_use]
    pub fn pulling(&self, actor: &ActorId) -> bool {
        self.inner.borrow_mut().pulling_inner(actor)
    }

    /// Gravity scale (LMCTF suspends gravity while close pulling).
    #[must_use]
    pub fn gravity_scale(&self, actor: &ActorId) -> i32 {
        self.inner.borrow_mut().gravity_scale_inner(actor)
    }

    /// Release an owner's hook.
    pub fn release(&self, actor: &ActorId) -> Result<(), GrappleError> {
        self.inner.borrow_mut().release_inner(actor)
    }

    /// Step one actor's grapple.
    pub fn step(&self, actor: &ActorId, enabled: bool) -> Result<(), GrappleError> {
        let slot_host = self.slot_host.clone();
        self.inner.borrow_mut().step_inner(
            actor,
            enabled,
            self.slot_binding(),
            self.rerelease(),
            slot_host.as_deref(),
        )
    }

    /// Capture the runtime checkpoint.
    #[must_use]
    pub fn capture(&self) -> GrappleRuntimeCheckpoint {
        let inner = self.inner.borrow();
        let mut weapon_animations: Vec<GrappleWeaponAnimationEntry> = inner
            .weapon_animations
            .iter()
            .map(|(actor, animation)| GrappleWeaponAnimationEntry {
                actor: SavedActorId::from(actor),
                state: animation.state.clone(),
                next_frame_at: animation.next_frame_at,
                kick_origin: animation.kick_origin,
                kick_pitch: animation.kick_pitch,
            })
            .collect();
        weapon_animations.sort_by_key(|entry| (entry.actor.slot, entry.actor.generation));
        let mut controls: Vec<GrappleControlEntry> = inner
            .controls
            .iter()
            .map(|(actor, control)| GrappleControlEntry {
                actor: SavedActorId::from(actor),
                teleport_bit: control.teleport_bit,
                jump: control.jump,
                held: control.held,
                pressed: control.pressed,
                released: control.released,
                previous_velocity: control.previous_velocity,
                prediction_suppressed: control.prediction_suppressed,
            })
            .collect();
        controls.sort_by_key(|entry| (entry.actor.slot, entry.actor.generation));
        let source = match (&inner.source, &self.bridge) {
            (GrappleSource::Q1Threewave { game }, GrappleFoundationBridge::Q1(bridge)) => {
                GrappleSourceCheckpoint::Q1Threewave {
                    entities: bridge.capture_entities(game),
                }
            }
            (GrappleSource::Q2Ctf { game, .. }, GrappleFoundationBridge::Q2(bridge)) => {
                let mut states: Vec<CtfGrappleStateEntry> = game
                    .equipment
                    .ctf_states
                    .iter()
                    .map(|(actor, state)| CtfGrappleStateEntry {
                        actor: SavedActorId::from(actor),
                        state: capture_ctf_grapple(state),
                    })
                    .collect();
                states.sort_by_key(|entry| (entry.actor.slot, entry.actor.generation));
                GrappleSourceCheckpoint::Q2Ctf {
                    entities: bridge.capture_entities(game),
                    states,
                }
            }
            (GrappleSource::Q2Lmctf { game, .. }, GrappleFoundationBridge::Q2(bridge)) => {
                let mut states: Vec<LmctfGrappleStateEntry> = game
                    .equipment
                    .lmctf_states
                    .iter()
                    .map(|(actor, state)| LmctfGrappleStateEntry {
                        actor: SavedActorId::from(actor),
                        state: capture_lmctf_grapple(state),
                    })
                    .collect();
                states.sort_by_key(|entry| (entry.actor.slot, entry.actor.generation));
                GrappleSourceCheckpoint::Q2Lmctf {
                    entities: bridge.capture_entities(game),
                    states,
                }
            }
            (GrappleSource::Q3Qvm { game, .. }, GrappleFoundationBridge::Qvm) => {
                let mut holstered: Vec<SavedActorId> = inner.q3_holstered.iter().map(SavedActorId::from).collect();
                holstered.sort_by_key(|saved| (saved.slot, saved.generation));
                GrappleSourceCheckpoint::Q3Qvm {
                    component: game.capture(),
                    holstered,
                }
            }
            _ => unreachable!("GrappleRuntime::new matches bridges to sources"),
        };
        GrappleRuntimeCheckpoint {
            version: CHECKPOINT_VERSION,
            weapon_animations,
            controls,
            random: inner.random.checkpoint(),
            source,
        }
    }

    /// Restore a checkpoint over the arena.
    pub fn restore(&self, checkpoint: GrappleRuntimeCheckpoint) -> Result<(), GrappleError> {
        let mut inner = self.inner.borrow_mut();
        inner
            .random
            .restore(&checkpoint.random)
            .map_err(|_| GrappleError::Random)?;
        let kinds_match = matches!(
            (&inner.source, &checkpoint.source),
            (
                GrappleSource::Q1Threewave { .. },
                GrappleSourceCheckpoint::Q1Threewave { .. }
            ) | (GrappleSource::Q2Ctf { .. }, GrappleSourceCheckpoint::Q2Ctf { .. })
                | (GrappleSource::Q2Lmctf { .. }, GrappleSourceCheckpoint::Q2Lmctf { .. })
                | (GrappleSource::Q3Qvm { .. }, GrappleSourceCheckpoint::Q3Qvm { .. })
        );
        if !kinds_match {
            return Err(GrappleError::SourceMismatch);
        }
        match (&mut inner.source, &checkpoint.source, &self.bridge) {
            (
                GrappleSource::Q1Threewave { game },
                GrappleSourceCheckpoint::Q1Threewave { entities },
                GrappleFoundationBridge::Q1(bridge),
            ) => {
                bridge
                    .restore_entities(game, entities, false)
                    .map_err(|error| GrappleError::Foundation(error.to_string()))?;
            }
            (
                GrappleSource::Q2Ctf { game, .. },
                GrappleSourceCheckpoint::Q2Ctf { entities, states },
                GrappleFoundationBridge::Q2(bridge),
            ) => {
                bridge
                    .restore_entities(game, entities)
                    .map_err(|error| GrappleError::Foundation(error.to_string()))?;
                game.equipment.ctf_states.clear();
                for entry in states {
                    let id = game.host.actors().reference_saved(entry.actor);
                    let state = restore_ctf_grapple(entry.state.clone(), game);
                    game.equipment.ctf_states.insert(id, state);
                }
            }
            (
                GrappleSource::Q2Lmctf { game, .. },
                GrappleSourceCheckpoint::Q2Lmctf { entities, states },
                GrappleFoundationBridge::Q2(bridge),
            ) => {
                bridge
                    .restore_entities(game, entities)
                    .map_err(|error| GrappleError::Foundation(error.to_string()))?;
                game.equipment.lmctf_states.clear();
                for entry in states {
                    let id = game.host.actors().reference_saved(entry.actor);
                    let state = restore_lmctf_grapple(entry.state.clone(), game);
                    game.equipment.lmctf_states.insert(id, state);
                }
            }
            (
                GrappleSource::Q3Qvm { game, .. },
                GrappleSourceCheckpoint::Q3Qvm { component, holstered },
                GrappleFoundationBridge::Qvm,
            ) => {
                game.restore(component)?;
                let mut actors = Vec::with_capacity(holstered.len());
                for saved in holstered {
                    actors.push(game.host_reference_saved(*saved));
                }
                inner.q3_holstered.clear();
                inner.q3_holstered.extend(actors);
            }
            _ => return Err(GrappleError::SourceMismatch),
        }
        inner.weapon_animations.clear();
        inner.q2_weapons.clear();
        for entry in &checkpoint.weapon_animations {
            let owner = inner.resolve_saved(entry.actor).ok_or(GrappleError::MissingWeapon)?;
            let presentation = self.slot_host.as_ref().map(|host| host.presentation(owner.id()));
            let Some(presentation) = presentation else {
                return Err(GrappleError::AnimationBinding);
            };
            inner.bind_weapon(
                owner.id(),
                GrappleWeaponAnimation {
                    state: entry.state.clone(),
                    next_frame_at: entry.next_frame_at,
                    kick_origin: entry.kick_origin,
                    kick_pitch: entry.kick_pitch,
                },
                self.ctf_edition(),
                presentation,
            )?;
        }
        inner.controls.clear();
        for entry in &checkpoint.controls {
            let owner = inner
                .resolve_saved(entry.actor)
                .ok_or(GrappleError::InvalidControlsOwner)?;
            if inner.controls.contains_key(owner.id()) {
                return Err(GrappleError::InvalidControlsOwner);
            }
            inner.controls.insert(
                owner.id().clone(),
                GrappleControl {
                    teleport_bit: entry.teleport_bit,
                    jump: entry.jump,
                    held: entry.held,
                    pressed: entry.pressed,
                    released: entry.released,
                    previous_velocity: entry.previous_velocity,
                    prediction_suppressed: entry.prediction_suppressed,
                },
            );
        }
        Ok(())
    }

    /// Drop an actor's tables. The session calls this alongside the arena
    /// release fan-out; the arena cannot carry the donor's `onRelease`
    /// subscription itself.
    pub fn on_actor_released(&self, actor: &ActorId) {
        let mut inner = self.inner.borrow_mut();
        inner.controls.remove(actor);
        inner.weapon_animations.remove(actor);
        inner.q2_weapons.remove(actor);
        inner.q3_holstered.remove(actor);
    }
}

/// Slot-owned weapon handoff over the shared grapple tables
/// (`EquipmentWeaponHandoff`).
pub struct GrappleHandoff {
    weapon: WeaponReference,
    actor: ActorId,
    inner: Rc<RefCell<GrappleInner>>,
    slot_host: Option<Rc<dyn GrappleSlotHost>>,
}

impl EquipmentWeaponHandoff for GrappleHandoff {
    fn weapon(&self) -> &WeaponReference {
        &self.weapon
    }

    fn holster(&mut self) {
        self.inner.borrow_mut().holster_handoff(&self.actor);
    }

    fn is_holstered(&self) -> bool {
        self.inner.borrow().is_holstered_inner(&self.actor)
    }

    fn resume(&mut self) {
        let slot_host = self.slot_host.clone();
        self.inner
            .borrow_mut()
            .resume_handoff(&self.actor, slot_host.as_deref());
    }
}

#[cfg(test)]
pub mod fakes {
    //! Fake Q1 arena, slot host, and QVM source for the grapple tests.
    use std::cell::{Cell, RefCell};
    use std::collections::{HashMap, HashSet};
    use std::rc::Rc;

    use qa_content::contract::{ArmorState, InventoryEntry, ItemId, QvmGrappleFovOffset, RegularArmorState};
    use qa_content::q1::equipment::grapple::{
        register_threewave_grapple, ThreewaveAnchor, ThreewaveGrappleHost, ThreewaveGrappleInput,
    };
    use qa_content::q1::equipment::weapon::{register_threewave_weapon, ThreewaveWeaponHooks};
    use qa_content::q1::foundation::entity_services::Q1EntityServices;
    use qa_content::q1::foundation::gameplay::{
        ActorObservation, BodyAttachment, BodyState, CombatState, CombatTraits, DamageOutcome, DamageRequest,
        LinkedBody, SourceSlot,
    };
    use qa_content::q1::foundation::host::{
        Q1ActorCallbackTable, Q1Contents, Q1DamageAdjustHook, Q1FoundationHost, Q1GameplayAuthority, Q1PusherStatus,
        Q1PusherStep, Q1SessionActorRegistry, Q1SharedBodyTable, Q1SharedInventoryTable,
    };
    use qa_content::q1::foundation::types::{Q1Edition, Q1FoundationOptions, Q1PrecacheProgram, Q1Trace};
    use qa_content::q1::Q1Error;
    use qa_core::identity::{ActorId, IdentityOwner, OwnedActor, ProviderId, SavedActorId};
    use qa_core::math::{vec3, Vec3};
    use qa_guest::checkpoint::{GameApi, GuestCheckpoint, GuestPrivateState, ModuleIdentity};
    use qa_world::body::BodyState as WorldBodyState;

    use super::super::equipment_runtime::fakes as equipment_fakes;
    use super::*;

    /// Minting fake Q1 actor registry.
    pub struct FakeQ1Actors {
        identities: IdentityOwner,
        provider: ProviderId,
        live: HashSet<ActorId>,
        owned: HashMap<ActorId, OwnedActor>,
        next_slot: u32,
    }

    impl FakeQ1Actors {
        /// Fresh registry.
        pub fn new() -> Self {
            Self {
                identities: IdentityOwner::create("grapple-q1-test").expect("owner"),
                provider: ProviderId::new("q1", "test"),
                live: HashSet::new(),
                owned: HashMap::new(),
                next_slot: 1,
            }
        }

        fn mint_inner(&mut self, owner: &ProviderId) -> OwnedActor {
            let slot = self.next_slot;
            self.next_slot += 1;
            let id = self.identities.actor(slot, 0);
            let owned = self.identities.owned_actor(&id, owner.clone()).expect("fake owner");
            self.live.insert(id.clone());
            self.owned.insert(id, owned.clone());
            owned
        }
    }

    impl Default for FakeQ1Actors {
        fn default() -> Self {
            Self::new()
        }
    }

    impl Q1SessionActorRegistry for FakeQ1Actors {
        fn allocate_at_source(
            &mut self,
            owner: &ProviderId,
            _source_slot: u32,
            _definition: &str,
        ) -> Result<OwnedActor, Q1Error> {
            Ok(self.mint_inner(owner))
        }

        fn assert_owned(&self, actor: &OwnedActor) -> Result<(), Q1Error> {
            if self.live.contains(actor.id()) {
                Ok(())
            } else {
                Err(Q1Error::Message("fake actor is not live".to_string()))
            }
        }

        fn resolve_owned(&self, actor: &ActorId) -> Option<OwnedActor> {
            if self.live.contains(actor) {
                self.owned.get(actor).cloned()
            } else {
                None
            }
        }

        fn is_live(&self, actor: &ActorId) -> bool {
            self.live.contains(actor)
        }

        fn observations(&self) -> Vec<ActorObservation> {
            self.owned
                .values()
                .filter(|owned| self.live.contains(owned.id()))
                .map(|owned| ActorObservation {
                    id: owned.id().clone(),
                    owner: self.provider.clone(),
                    definition: "q1:test".to_string(),
                })
                .collect()
        }

        fn source_of(&self, _actor: &ActorId) -> Option<SourceSlot> {
            None
        }

        fn release(&mut self, actor: &OwnedActor) -> Result<(), Q1Error> {
            self.live.remove(actor.id());
            Ok(())
        }

        fn resolve_saved(&self, saved: &SavedActorId) -> Option<OwnedActor> {
            self.owned
                .values()
                .find(|owned| {
                    owned.id().slot() == saved.slot
                        && owned.id().generation() == saved.generation
                        && self.live.contains(owned.id())
                })
                .cloned()
        }

        fn reference_saved(&mut self, saved: &SavedActorId) -> ActorId {
            self.identities.actor(saved.slot, saved.generation)
        }
    }

    /// Fake Q1 body table.
    pub struct FakeQ1Bodies {
        bodies: HashMap<ActorId, BodyState>,
        attachments: HashMap<ActorId, BodyAttachment>,
    }

    impl FakeQ1Bodies {
        /// Fresh table.
        pub fn new() -> Self {
            Self {
                bodies: HashMap::new(),
                attachments: HashMap::new(),
            }
        }
    }

    impl Default for FakeQ1Bodies {
        fn default() -> Self {
            Self::new()
        }
    }

    impl Q1SharedBodyTable for FakeQ1Bodies {
        fn create(&mut self, actor: &OwnedActor, initial: &BodyState) -> Result<(), Q1Error> {
            self.bodies.insert(actor.id().clone(), initial.clone());
            Ok(())
        }

        fn read(&self, actor: &ActorId) -> Option<BodyState> {
            self.bodies.get(actor).cloned()
        }

        fn write(&mut self, actor: &OwnedActor, state: &BodyState) -> Result<(), Q1Error> {
            self.bodies.insert(actor.id().clone(), state.clone());
            Ok(())
        }

        fn link(&mut self, _actor: &OwnedActor) -> Result<(), Q1Error> {
            Ok(())
        }

        fn linked(&self, _actor: &ActorId) -> Option<LinkedBody> {
            None
        }

        fn attach(&mut self, actor: &OwnedActor, attachment: &BodyAttachment) -> Result<(), Q1Error> {
            self.attachments.insert(actor.id().clone(), attachment.clone());
            Ok(())
        }

        fn detach(&mut self, actor: &OwnedActor) -> Result<(), Q1Error> {
            self.attachments.remove(actor.id());
            Ok(())
        }
    }

    /// Fake Q1 callback table.
    pub struct FakeQ1Callbacks {
        bound: HashSet<ActorId>,
    }

    impl FakeQ1Callbacks {
        /// Fresh table.
        pub fn new() -> Self {
            Self { bound: HashSet::new() }
        }
    }

    impl Default for FakeQ1Callbacks {
        fn default() -> Self {
            Self::new()
        }
    }

    impl Q1ActorCallbackTable for FakeQ1Callbacks {
        fn bind(&mut self, actor: &OwnedActor) {
            self.bound.insert(actor.id().clone());
        }

        fn unbind(&mut self, actor: &OwnedActor) {
            self.bound.remove(actor.id());
        }

        fn is_bound(&self, actor: &ActorId) -> bool {
            self.bound.contains(actor)
        }
    }

    /// Fake Q1 combat authority.
    pub struct FakeQ1Combat {
        combat: HashMap<ActorId, CombatState>,
    }

    impl FakeQ1Combat {
        /// Fresh authority.
        pub fn new() -> Self {
            Self { combat: HashMap::new() }
        }
    }

    impl Default for FakeQ1Combat {
        fn default() -> Self {
            Self::new()
        }
    }

    impl Q1GameplayAuthority for FakeQ1Combat {
        fn create(&mut self, actor: &OwnedActor, initial: &CombatState) -> Result<(), Q1Error> {
            self.combat.insert(actor.id().clone(), initial.clone());
            Ok(())
        }

        fn read(&self, actor: &ActorId) -> Option<CombatState> {
            self.combat.get(actor).cloned()
        }

        fn set_health(&mut self, actor: &OwnedActor, health: f64) -> Result<(), Q1Error> {
            if let Some(combat) = self.combat.get_mut(actor.id()) {
                combat.health = health;
            }
            Ok(())
        }

        fn set_armor(&mut self, _actor: &OwnedActor, _armor: &ArmorState) -> Result<(), Q1Error> {
            Ok(())
        }

        fn set_traits(&mut self, _actor: &OwnedActor, _traits: CombatTraits) -> Result<(), Q1Error> {
            Ok(())
        }

        fn set_regular_armor(&mut self, _actor: &OwnedActor, _regular: &RegularArmorState) -> Result<(), Q1Error> {
            Ok(())
        }

        fn set_regular_points(&mut self, _actor: &OwnedActor, _points: f64) -> Result<(), Q1Error> {
            Ok(())
        }

        fn bind_damage_adjustment(&mut self, _actor: &OwnedActor, _adjust: Q1DamageAdjustHook) {}

        fn apply(&mut self, input: &DamageRequest) -> DamageOutcome {
            DamageOutcome::StaleTarget { request: input.clone() }
        }
    }

    /// Fake Q1 inventory table.
    pub struct FakeQ1Inventory {
        inventory: HashMap<ActorId, Vec<InventoryEntry>>,
    }

    impl FakeQ1Inventory {
        /// Fresh table.
        pub fn new() -> Self {
            Self {
                inventory: HashMap::new(),
            }
        }
    }

    impl Default for FakeQ1Inventory {
        fn default() -> Self {
            Self::new()
        }
    }

    impl Q1SharedInventoryTable for FakeQ1Inventory {
        fn create(&mut self, actor: &OwnedActor, entries: &[InventoryEntry]) -> Result<(), Q1Error> {
            self.inventory
                .entry(actor.id().clone())
                .or_default()
                .extend(entries.iter().cloned());
            Ok(())
        }

        fn entries(&self, actor: &ActorId) -> Vec<InventoryEntry> {
            self.inventory.get(actor).cloned().unwrap_or_default()
        }

        fn has(&self, actor: &ActorId) -> bool {
            self.inventory.get(actor).is_some_and(|entries| !entries.is_empty())
        }

        fn count(&self, actor: &ActorId, item: &ItemId) -> f64 {
            self.inventory
                .get(actor)
                .and_then(|entries| entries.iter().find(|entry| &entry.item == item))
                .map_or(0.0, |entry| entry.count)
        }

        fn consume(&mut self, actor: &OwnedActor, item: &ItemId, count: f64) -> bool {
            let Some(entry) = self
                .inventory
                .get_mut(actor.id())
                .and_then(|entries| entries.iter_mut().find(|entry| &entry.item == item))
            else {
                return false;
            };
            if entry.count < count {
                return false;
            }
            entry.count -= count;
            true
        }

        fn give(&mut self, actor: &OwnedActor, item: &ItemId, count: f64) -> f64 {
            let entries = self.inventory.entry(actor.id().clone()).or_default();
            if let Some(entry) = entries.iter_mut().find(|entry| &entry.item == item) {
                entry.count += count;
            } else {
                entries.push(InventoryEntry {
                    item: item.clone(),
                    count,
                    capacity: count,
                    count_policy: None,
                });
            }
            count
        }

        fn configure(&mut self, actor: &OwnedActor, entry: &InventoryEntry) -> Result<(), Q1Error> {
            let entries = self.inventory.entry(actor.id().clone()).or_default();
            if let Some(slot) = entries.iter_mut().find(|slot| slot.item == entry.item) {
                *slot = entry.clone();
            } else {
                entries.push(entry.clone());
            }
            Ok(())
        }

        fn adjust_source_counter(&mut self, actor: &OwnedActor, item: &ItemId, delta: f64) -> Result<f64, Q1Error> {
            let Some(entry) = self
                .inventory
                .get_mut(actor.id())
                .and_then(|entries| entries.iter_mut().find(|entry| &entry.item == item))
            else {
                return Ok(0.0);
            };
            entry.count += delta;
            Ok(entry.count)
        }
    }

    /// Cooperative threewave host.
    pub struct FakeThreewaveHost;

    impl ThreewaveGrappleHost for FakeThreewaveHost {
        fn input(&self, _actor: &ActorId) -> ThreewaveGrappleInput {
            ThreewaveGrappleInput {
                held: true,
                release: false,
                jump: false,
                view_angles: vec3(0.0, 0.0, 0.0),
                teleport_until: 0.0,
            }
        }

        fn aim(&self, _actor: &ActorId, forward: Vec3) -> Vec3 {
            forward
        }

        fn anchor(&self, _actor: &ActorId) -> ThreewaveAnchor {
            ThreewaveAnchor {
                solid: true,
                centered: false,
                player: false,
            }
        }

        fn can_attach(&self, _owner: &ActorId, _target: &ActorId) -> bool {
            true
        }

        fn can_pulse(&self, _owner: &ActorId, _target: &ActorId) -> bool {
            true
        }

        fn can_damage(&self, _target: &ActorId, _owner: &ActorId) -> bool {
            true
        }
    }

    /// Pass-through threewave weapon hooks.
    pub struct FakeThreewaveWeaponHooks;

    impl ThreewaveWeaponHooks for FakeThreewaveWeaponHooks {
        fn launch(&mut self, _game: &mut Q1EntityServices, _actor: &ActorId) -> Result<Option<bool>, Q1Error> {
            Ok(None)
        }

        fn animated(&mut self, _game: &mut Q1EntityServices, _actor: &ActorId, _frame: i32) -> Result<(), Q1Error> {
            Ok(())
        }
    }

    /// Classic Q1 game options for tests.
    pub fn test_q1_options() -> Q1FoundationOptions {
        Q1FoundationOptions {
            provider: None,
            precache_program: Some(Q1PrecacheProgram::Id1),
            edition: Q1Edition::Classic,
            physics_edition: None,
            skill: 1,
            deathmatch: 0,
            coop: false,
            campaign: ProviderId::new("q1", "campaign"),
            combat_provider: ProviderId::new("q1", "combat"),
            movement_provider: ProviderId::new("q1", "movement"),
            inventory_provider: ProviderId::new("q1", "inventory"),
            gravity: 800.0,
            max_clients: Some(4),
            no_exit: None,
            teamplay: None,
            aim_threshold: None,
        }
    }

    /// Build a Q1 arena over fake tables with threewave services registered.
    pub fn test_q1_game() -> Q1EntityServices {
        let host = Q1FoundationHost {
            actors: Box::new(FakeQ1Actors::new()),
            bodies: Box::new(FakeQ1Bodies::new()),
            callbacks: Box::new(FakeQ1Callbacks::new()),
            combat: Box::new(FakeQ1Combat::new()),
            inventory: Box::new(FakeQ1Inventory::new()),
            original_pickups: None,
            punch_angles: None,
            weapon_behavior: None,
            register_entity: None,
            source_damage_modifier: None,
            source_damage_powerup_owner: None,
            random: Box::new(|| 0.5),
            trace: Box::new(|request| Q1Trace {
                fraction: 1.0,
                end: request.end,
                normal: vec3(0.0, 0.0, 1.0),
                actor: None,
                start_solid: false,
                all_solid: false,
                sky: false,
                in_open: true,
                in_water: false,
            }),
            contents: Box::new(|_| Q1Contents::Empty),
            walk_move: Box::new(|_, _, _| true),
            change_yaw: Box::new(|_| {}),
            move_to_goal: Box::new(|_, _, _, _| {}),
            check_bottom: Box::new(|_| true),
            schedule_think: Box::new(|_, _| {}),
            cancel_think: Box::new(|_| {}),
            emit: Box::new(|_| {}),
            transition: Box::new(|_| {}),
            players: Box::new(Vec::new),
            check_client: Box::new(|_| None),
            classname: Box::new(|_| String::new()),
            powerup: Box::new(|_, _, _| {}),
            step_pusher: Box::new(|actor, _| Q1PusherStep {
                actor: actor.clone(),
                status: Q1PusherStatus::Moved,
                moved: Vec::new(),
            }),
            weapon_impact: None,
            weapon_volume: None,
            monster_target: None,
            set_gravity: None,
            control_player: None,
            source_target: None,
            powerup_expires: None,
            source_damage_multiplier: None,
            is_bot: None,
        };
        let mut game = Q1EntityServices::new(host, test_q1_options()).expect("game");
        register_threewave_grapple(&mut game, Box::new(FakeThreewaveHost)).expect("grapple");
        register_threewave_weapon(&mut game, Box::new(FakeThreewaveWeaponHooks)).expect("weapon");
        game.begin_frame(100.0, 0.1);
        game
    }

    /// Recording Q1 foundation bridge over inert snapshots.
    pub struct FakeQ1Bridge {
        /// Restored snapshots, in order.
        pub restored: RefCell<Vec<Q1FoundationSave>>,
    }

    impl FakeQ1Bridge {
        /// Fresh bridge.
        pub fn new() -> Self {
            Self {
                restored: RefCell::new(Vec::new()),
            }
        }
    }

    impl Default for FakeQ1Bridge {
        fn default() -> Self {
            Self::new()
        }
    }

    impl Q1FoundationCheckpointBridge for FakeQ1Bridge {
        fn capture_entities(&self, _game: &Q1EntityServices) -> Q1FoundationSave {
            Q1FoundationSave {
                provider: "q1".to_string(),
                precache_phase: "server".to_string(),
                precache_models: Vec::new(),
                precache_sounds: Vec::new(),
                edition: "classic".to_string(),
                time: 100.0,
                frame_seconds: 0.1,
                force_retouch: 0.0,
                sequence: 0,
                next_dynamic_slot: 1,
                total_secrets: 0.0,
                found_secrets: 0.0,
                total_monsters: 0.0,
                killed_monsters: 0.0,
                world_type: 0.0,
                map_name: "test".to_string(),
                basis: (vec3(0.0, 0.0, 0.0), vec3(0.0, 0.0, 0.0), vec3(0.0, 0.0, 0.0)),
                world: None,
                sight_entity: None,
                sight_time: 0.0,
                intermission: None,
                entities: Vec::new(),
                players: Vec::new(),
                extensions: Vec::new(),
            }
        }

        fn restore_entities(
            &self,
            _game: &mut Q1EntityServices,
            checkpoint: &Q1FoundationSave,
            _schedule_thinks: bool,
        ) -> Result<(), WorldError> {
            self.restored.borrow_mut().push(checkpoint.clone());
            Ok(())
        }
    }

    fn noop() {}

    fn anim_time(_state: &Q2GenericFrameState) -> f64 {
        0.1
    }

    /// Recording slot host with flippable selection.
    pub struct FakeSlotHost {
        /// Selected flag.
        pub selected: Cell<bool>,
        /// Available flag.
        pub available: Cell<bool>,
        /// Shown frames, in order.
        pub frames: RefCell<Vec<(ActorId, i32)>>,
    }

    impl FakeSlotHost {
        /// Selected and available host.
        pub fn new() -> Self {
            Self {
                selected: Cell::new(true),
                available: Cell::new(true),
                frames: RefCell::new(Vec::new()),
            }
        }
    }

    impl Default for FakeSlotHost {
        fn default() -> Self {
            Self::new()
        }
    }

    impl GrappleSlotHost for FakeSlotHost {
        fn selected(&self, _actor: &ActorId) -> bool {
            self.selected.get()
        }

        fn available(&self, _actor: &ActorId) -> bool {
            self.available.get()
        }

        fn frame(&self, actor: &ActorId, frame: i32) {
            self.frames.borrow_mut().push((actor.clone(), frame));
        }

        fn presentation(&self, _actor: &ActorId) -> GrappleSlotPresentation {
            GrappleSlotPresentation {
                attack_animation: noop,
                reverse_animation: noop,
                powerup_sound: noop,
                animation_time: anim_time,
            }
        }
    }

    /// Shared observable QVM game state.
    #[derive(Default)]
    pub struct FakeQvmState {
        /// Live hooks by owner.
        pub hooks: HashMap<ActorId, ActorId>,
        /// Pulling owners.
        pub pulling: HashSet<ActorId>,
        /// Fired owners, in order.
        pub fired: Vec<ActorId>,
        /// Released owners, in order.
        pub released: Vec<ActorId>,
        /// Restored components, in order.
        pub restored: Vec<QvmGrappleSourceCheckpoint>,
    }

    /// Minting fake QVM grapple game.
    pub struct FakeQvmGame {
        identities: IdentityOwner,
        live: HashSet<ActorId>,
        owned: HashMap<ActorId, OwnedActor>,
        bodies: HashMap<ActorId, WorldBodyState>,
        next_slot: u32,
        shared: Rc<RefCell<FakeQvmState>>,
    }

    impl FakeQvmGame {
        /// Fresh game plus its shared observable state.
        pub fn new() -> (Self, Rc<RefCell<FakeQvmState>>) {
            let shared = Rc::new(RefCell::new(FakeQvmState::default()));
            (
                Self {
                    identities: IdentityOwner::create("grapple-qvm-test").expect("owner"),
                    live: HashSet::new(),
                    owned: HashMap::new(),
                    bodies: HashMap::new(),
                    next_slot: 1,
                    shared: Rc::clone(&shared),
                },
                shared,
            )
        }

        /// Mint a live actor with a body.
        pub fn mint(&mut self) -> OwnedActor {
            let slot = self.next_slot;
            self.next_slot += 1;
            let id = self.identities.actor(slot, 0);
            let owned = self
                .identities
                .owned_actor(&id, ProviderId::new("q3", "test"))
                .expect("fake owner");
            self.live.insert(id.clone());
            self.owned.insert(id.clone(), owned.clone());
            self.bodies.insert(
                id,
                WorldBodyState {
                    origin: vec3(0.0, 0.0, 0.0),
                    angles: vec3(0.0, 0.0, 0.0),
                    velocity: vec3(0.0, 0.0, 0.0),
                    bounds: qa_core::math::Bounds {
                        min: vec3(-15.0, -15.0, -24.0),
                        max: vec3(15.0, 15.0, 32.0),
                    },
                    ground: None,
                },
            );
            owned
        }

        fn module_identity() -> ModuleIdentity {
            ModuleIdentity {
                id: "q3:test".to_string(),
                artifact_path: "test.qvm".to_string(),
                digest: "sha256:00".to_string(),
                revision: "1".to_string(),
            }
        }
    }

    impl QvmGrappleGame for FakeQvmGame {
        fn admit(&mut self, actor: &ActorId) -> Result<(), GrappleError> {
            if self.live.contains(actor) {
                Ok(())
            } else {
                Err(GrappleError::Qvm(
                    "Grapple equipment requires an admitted source client".to_string(),
                ))
            }
        }

        fn release(&mut self, actor: &ActorId) {
            let mut shared = self.shared.borrow_mut();
            shared.hooks.remove(actor);
            shared.pulling.remove(actor);
            shared.released.push(actor.clone());
        }

        fn close(&mut self) {}

        fn fire(&mut self, actor: &ActorId) {
            let hook = self.mint();
            let mut shared = self.shared.borrow_mut();
            shared.hooks.insert(actor.clone(), hook.id().clone());
            shared.pulling.insert(actor.clone());
            shared.fired.push(actor.clone());
        }

        fn hook(&self, actor: &ActorId) -> Option<ActorId> {
            self.shared.borrow().hooks.get(actor).cloned()
        }

        fn pulling(&self, actor: &ActorId) -> bool {
            self.shared.borrow().pulling.contains(actor)
        }

        fn weapon_view(&self, actor: &ActorId) -> Result<QvmGrappleWeaponView, GrappleError> {
            if !self.live.contains(actor) {
                return Err(GrappleError::Qvm("Hook view has no source player".to_string()));
            }
            Ok(QvmGrappleWeaponView {
                path: "progs/v_hook.mdl".to_string(),
                frame: 0,
                kick_origin: vec3(0.0, 0.0, 0.0),
                kick_pitch: 0.0,
                model_attachments: vec![qa_content::contract::QvmGrappleViewAttachment {
                    path: "hook".to_string(),
                    tag: "tag_hook".to_string(),
                }],
                model_anchor: qa_content::contract::QvmGrappleViewAnchor {
                    path: "hand".to_string(),
                    tag: "tag_hand".to_string(),
                    offset: vec3(0.0, 0.0, 0.0),
                    fov_offset: QvmGrappleFovOffset { above: 0.0, scale: 1.0 },
                },
                q3_weapon: QvmGrappleQ3Weapon {
                    time_ms: 100,
                    torso_animation: 0,
                    last_fire_ms: None,
                    firing: self.shared.borrow().pulling.contains(actor),
                    horizontal_speed: 0.0,
                    bob_cycle: 0,
                    weapon: 10.0,
                },
            })
        }

        fn capture(&self) -> QvmGrappleSourceCheckpoint {
            let module = Self::module_identity();
            QvmGrappleSourceCheckpoint {
                version: 1,
                grapple: QvmGrappleCheckpoint {
                    version: 1,
                    profile: "test".to_string(),
                    // The unit API variant is a test placeholder: the fake
                    // never interprets the module.
                    module: GuestCheckpoint::Qvm {
                        module: module.clone(),
                        random: Vec::new(),
                        callbacks: Vec::new(),
                        api: GameApi::Q2ClassicGame,
                        abi_profile: "test".to_string(),
                        data: Vec::new(),
                        instruction_index: 0,
                        program_stack: 0,
                        operand_stack: Vec::new(),
                        host_state: GuestPrivateState {
                            module,
                            format: "q3:grapple-test".to_string(),
                            bytes: Vec::new(),
                        },
                    },
                    owners: self.shared.borrow().hooks.keys().map(SavedActorId::from).collect(),
                },
                bindings: self
                    .owned
                    .values()
                    .map(|owned| QvmGrappleBindingCheckpoint {
                        actor: SavedActorId::from(owned.id()),
                        pointer: i64::from(owned.id().slot()),
                        client: true,
                        origin: vec3(0.0, 0.0, 0.0),
                    })
                    .collect(),
                tethers: self
                    .shared
                    .borrow()
                    .hooks
                    .iter()
                    .map(|(owner, hook)| QvmGrappleTetherCheckpoint {
                        owner: SavedActorId::from(owner),
                        actor: SavedActorId::from(hook),
                    })
                    .collect(),
            }
        }

        fn restore(&mut self, checkpoint: &QvmGrappleSourceCheckpoint) -> Result<(), GrappleError> {
            self.shared.borrow_mut().restored.push(checkpoint.clone());
            Ok(())
        }

        fn host_is_live(&self, actor: &ActorId) -> bool {
            self.live.contains(actor)
        }

        fn host_body(&self, actor: &ActorId) -> Option<WorldBodyState> {
            self.bodies.get(actor).cloned()
        }

        fn host_resolve_saved(&self, saved: SavedActorId) -> Option<OwnedActor> {
            self.owned
                .values()
                .find(|owned| {
                    owned.id().slot() == saved.slot
                        && owned.id().generation() == saved.generation
                        && self.live.contains(owned.id())
                })
                .cloned()
        }

        fn host_reference_saved(&mut self, saved: SavedActorId) -> ActorId {
            self.identities.actor(saved.slot, saved.generation)
        }
    }

    /// Shared QVM release log.
    pub type QvmReleaseLog = Rc<RefCell<Vec<(ActorId, bool)>>>;

    /// Recording fake QVM core.
    pub struct FakeQvmCore {
        shared: Rc<RefCell<FakeQvmState>>,
        /// Released owners with the force flag, in order.
        pub released: QvmReleaseLog,
    }

    impl FakeQvmCore {
        /// Fresh core over shared tether state, plus its release log.
        pub fn new(shared: Rc<RefCell<FakeQvmState>>) -> (Self, QvmReleaseLog) {
            let released = Rc::new(RefCell::new(Vec::new()));
            (
                Self {
                    shared,
                    released: Rc::clone(&released),
                },
                released,
            )
        }
    }

    impl QvmGrappleCore for FakeQvmCore {
        fn release(&mut self, actor: &ActorId, force: bool) {
            let mut shared = self.shared.borrow_mut();
            shared.hooks.remove(actor);
            shared.pulling.remove(actor);
            drop(shared);
            self.released.borrow_mut().push((actor.clone(), force));
        }
    }

    /// Re-export the equipment fakes for grapple tests over Q2 arenas.
    pub use equipment_fakes::{
        test_body as q2_test_body, test_game as q2_test_game, test_options as q2_test_options,
        FakeBridge as Q2FakeBridge,
    };
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::rc::Rc;

    use qa_content::contract::{
        ContentId, ModuleIdentity, ProviderReference, QvmAbiProfile, QvmGrappleCable, QvmGrappleCallbacks,
        QvmGrappleDefinition, QvmGrappleFields, QvmGrappleGlobals, QvmGrappleMovement, QvmGrapplePresentation,
    };
    use qa_content::q2::equipment::ctf_grapple::Q2CtfGrappleEquipment;
    use qa_content::q2::equipment::grapple_weapon::GrappleHandoff as WeaponHandoffState;
    use qa_content::q2::equipment::lmctf_grapple::LmctfGrappleEquipment;
    use qa_core::identity::ProviderId;
    use qa_core::math::vec3;

    use super::fakes::*;
    use super::*;

    fn provider_ref(family: &str) -> ProviderReference {
        ProviderReference {
            provider: ProviderId::new(family, "test"),
            content: ContentId(format!("{family}:test:test:1")),
        }
    }

    fn q1_selection(binding: GrappleBinding) -> GrappleSelection {
        GrappleSelection::Enabled {
            source: provider_ref("q1"),
            binding,
            mechanic: GrappleMechanicDetail::Q1Threewave {
                edition: SourceEdition::Classic,
            },
        }
    }

    fn ctf_selection(binding: GrappleBinding) -> GrappleSelection {
        GrappleSelection::Enabled {
            source: provider_ref("q2"),
            binding,
            mechanic: GrappleMechanicDetail::Q2Ctf {
                edition: SourceEdition::Classic,
            },
        }
    }

    fn lmctf_selection(binding: GrappleBinding) -> GrappleSelection {
        GrappleSelection::Enabled {
            source: provider_ref("q2"),
            binding,
            mechanic: GrappleMechanicDetail::Q2Lmctf,
        }
    }

    fn qvm_profile() -> QvmGrappleDefinition {
        QvmGrappleDefinition {
            id: "hook".to_string(),
            title: "Hook".to_string(),
            module: ModuleIdentity {
                id: ProviderId::new("q3", "grapple/hook"),
                artifact_path: "vm/hook.qvm".to_string(),
                digest: qa_content::contract::ContentDigest("sha256:00".to_string()),
                revision: "1".to_string(),
            },
            abi_profile: QvmAbiProfile::Modern,
            entity_stride: 8,
            client_stride: 8,
            fields: QvmGrappleFields {
                inuse: 0,
                client: 1,
                parent: 2,
                target: 3,
                mover: None,
                hook: 4,
                health: 5,
                takedamage: 6,
                event_time: 7,
                free_after_event: 8,
            },
            globals: QvmGrappleGlobals {
                time: 0,
                frame: 1,
                movement: 2,
                forward: 3,
                ground_plane: 4,
            },
            callbacks: QvmGrappleCallbacks {
                allocate: 0,
                free: 1,
                fire: 2,
                release: 3,
                force_release: 4,
                missile: 5,
                follow: None,
                think: 6,
                pull: 7,
                move_mover_hooks: None,
                damage: 8,
                same_team: 9,
                player_move: 10,
            },
            fire_arguments: Vec::new(),
            movement: QvmGrappleMovement {
                byte_length: 0,
                words: Vec::new(),
            },
            initial_cvars: HashMap::new(),
            event_lifetime_milliseconds: 0.0,
            grapple_damage_method: 0.0,
            presentation: QvmGrapplePresentation {
                projectile_model: "models/hook/tris.md3".to_string(),
                view_model: "models/v_hook/tris.md3".to_string(),
                weapon_index: 1.0,
                view_anchor: qa_content::contract::QvmGrappleViewAnchor {
                    path: "models/anchor/tris.md3".to_string(),
                    tag: "tag".to_string(),
                    offset: vec3(0.0, 0.0, 0.0),
                    fov_offset: qa_content::contract::QvmGrappleFovOffset { above: 0.0, scale: 1.0 },
                },
                view_attachments: Vec::new(),
                cable: QvmGrappleCable::Model {
                    flight: "models/cable/fly.md3".to_string(),
                    pull: "models/cable/pull.md3".to_string(),
                    hold: "models/cable/hold.md3".to_string(),
                    segment_length: 1.0,
                },
                fire_sound: None,
                attach_sound: None,
                release_sound: None,
                pull_sound: None,
                hang_sound: None,
            },
            pulling_flag: 0.0,
        }
    }

    fn qvm_selection(binding: GrappleBinding) -> GrappleSelection {
        GrappleSelection::Enabled {
            source: provider_ref("q3"),
            binding,
            mechanic: GrappleMechanicDetail::Q3Qvm { profile: qvm_profile() },
        }
    }

    fn slot_host_for(binding: GrappleBinding) -> (Option<Rc<dyn GrappleSlotHost>>, Rc<FakeSlotHost>) {
        let host = Rc::new(FakeSlotHost::new());
        let shared = (binding == GrappleBinding::Slot).then(|| Rc::clone(&host) as Rc<dyn GrappleSlotHost>);
        (shared, host)
    }

    fn ctf_runtime(binding: GrappleBinding) -> (GrappleRuntime, Rc<FakeSlotHost>, Rc<RefCell<f64>>) {
        let (game, clock) = q2_test_game();
        let (slot_host, host) = slot_host_for(binding);
        let runtime = GrappleRuntime::new(
            ctf_selection(binding),
            GrappleSource::Q2Ctf {
                game,
                core: Q2CtfGrappleEquipment::new(equipment_grapple_hooks()),
            },
            SourceRandom::new(1),
            slot_host,
            GrappleFoundationBridge::Q2(Box::new(Q2FakeBridge::new())),
        )
        .expect("runtime");
        (runtime, host, clock)
    }

    fn lmctf_runtime(binding: GrappleBinding) -> GrappleRuntime {
        let (game, _) = q2_test_game();
        let (slot_host, _) = slot_host_for(binding);
        GrappleRuntime::new(
            lmctf_selection(binding),
            GrappleSource::Q2Lmctf {
                game,
                core: LmctfGrappleEquipment::new(equipment_grapple_hooks()),
            },
            SourceRandom::new(1),
            slot_host,
            GrappleFoundationBridge::Q2(Box::new(Q2FakeBridge::new())),
        )
        .expect("runtime")
    }

    fn q1_runtime(binding: GrappleBinding) -> (GrappleRuntime, Rc<FakeSlotHost>) {
        let (slot_host, host) = slot_host_for(binding);
        let runtime = GrappleRuntime::new(
            q1_selection(binding),
            GrappleSource::Q1Threewave { game: test_q1_game() },
            SourceRandom::new(1),
            slot_host,
            GrappleFoundationBridge::Q1(Box::new(FakeQ1Bridge::new())),
        )
        .expect("runtime");
        (runtime, host)
    }

    struct QvmRig {
        runtime: GrappleRuntime,
        shared: Rc<RefCell<FakeQvmState>>,
        released: QvmReleaseLog,
        first: OwnedActor,
        second: OwnedActor,
    }

    fn qvm_runtime(binding: GrappleBinding) -> QvmRig {
        let (mut game, shared) = FakeQvmGame::new();
        let first = game.mint();
        let second = game.mint();
        let (core, released) = FakeQvmCore::new(Rc::clone(&shared));
        let (slot_host, _) = slot_host_for(binding);
        let runtime = GrappleRuntime::new(
            qvm_selection(binding),
            GrappleSource::Q3Qvm {
                game: Box::new(game),
                core: Box::new(core),
            },
            SourceRandom::new(1),
            slot_host,
            GrappleFoundationBridge::Qvm,
        )
        .expect("runtime");
        QvmRig {
            runtime,
            shared,
            released,
            first,
            second,
        }
    }

    fn q2_actor(runtime: &GrappleRuntime) -> OwnedActor {
        runtime.with_source(|source| match source {
            GrappleSource::Q2Ctf { game, .. } | GrappleSource::Q2Lmctf { game, .. } => {
                let owned = game.host.actors().allocate(&ProviderId::new("q2", "test"), "q2:test");
                game.host.bodies().create(&owned, &q2_test_body());
                owned
            }
            _ => panic!("q2 source"),
        })
    }

    fn q1_actor(runtime: &GrappleRuntime) -> ActorId {
        runtime.with_source(|source| match source {
            GrappleSource::Q1Threewave { game } => {
                let actor = game.create("player", None, None).expect("player");
                game.set_health(&actor, 100.0).expect("health");
                actor
            }
            _ => panic!("q1 source"),
        })
    }

    #[test]
    fn rejects_bad_construction() {
        let (game, _) = q2_test_game();
        let core = Q2CtfGrappleEquipment::new(equipment_grapple_hooks());
        assert!(matches!(
            GrappleRuntime::new(
                GrappleSelection::Disabled,
                GrappleSource::Q2Ctf { game, core },
                SourceRandom::new(1),
                None,
                GrappleFoundationBridge::Q2(Box::new(Q2FakeBridge::new())),
            ),
            Err(GrappleError::Disabled)
        ));
        let (runtime, _, _) = ctf_runtime(GrappleBinding::Offhand);
        let owner = q2_actor(&runtime);
        assert!(matches!(runtime.handoff(owner.id()), Err(GrappleError::NoWeaponSlot)));
        drop(runtime);
        let (qvm_game, shared) = FakeQvmGame::new();
        let (qvm_core, _) = FakeQvmCore::new(shared);
        assert!(matches!(
            GrappleRuntime::new(
                ctf_selection(GrappleBinding::Slot),
                GrappleSource::Q3Qvm {
                    game: Box::new(qvm_game),
                    core: Box::new(qvm_core),
                },
                SourceRandom::new(1),
                Some(Rc::new(FakeSlotHost::new()) as Rc<dyn GrappleSlotHost>),
                GrappleFoundationBridge::Q2(Box::new(Q2FakeBridge::new())),
            ),
            Err(GrappleError::BridgeMismatch)
        ));
    }

    #[test]
    fn missing_slot_host_rejected() {
        let game = test_q1_game();
        assert!(matches!(
            GrappleRuntime::new(
                q1_selection(GrappleBinding::Slot),
                GrappleSource::Q1Threewave { game },
                SourceRandom::new(1),
                None,
                GrappleFoundationBridge::Q1(Box::new(FakeQ1Bridge::new())),
            ),
            Err(GrappleError::MissingSlotHost)
        ));
    }

    #[test]
    fn weapon_items_by_mechanic() {
        let (ctf, _, _) = ctf_runtime(GrappleBinding::Slot);
        assert_eq!(ctf.weapon().item, "q2:weapon_grapple");
        let lmctf = lmctf_runtime(GrappleBinding::Offhand);
        assert_eq!(lmctf.weapon().item, "q2:weapon_hook");
        let (q1, _) = q1_runtime(GrappleBinding::Slot);
        assert_eq!(q1.weapon().item, "q1:ctf/weapon/grapple");
        let qvm = qvm_runtime(GrappleBinding::Slot);
        assert_eq!(qvm.runtime.weapon().item, "q3:weapon_grapplinghook");
    }

    #[test]
    fn ctf_slot_admit_binds_weapon_and_view() {
        let (runtime, _, _) = ctf_runtime(GrappleBinding::Slot);
        let owner = q2_actor(&runtime);
        runtime.admit(owner.id()).expect("admit");
        assert_eq!(runtime.hook(owner.id()), None);
        assert!(!runtime.pulling(owner.id()));
        let view = runtime.weapon_view(owner.id()).expect("view");
        match view {
            GrappleWeaponView::Standard { path, frame, .. } => {
                assert!(!path.is_empty());
                assert_eq!(frame, 0);
            }
            GrappleWeaponView::Qvm(_) => panic!("standard view"),
        }
        let mut handoff = runtime.handoff(owner.id()).expect("handoff");
        assert_eq!(handoff.weapon().item, "q2:weapon_grapple");
        assert!(handoff.is_holstered());
        handoff.resume();
        assert!(!handoff.is_holstered());
        handoff.holster();
        // Lowering reads active until a step completes it.
        assert!(!handoff.is_holstered());
        let checkpoint = runtime.capture();
        assert_eq!(
            checkpoint.weapon_animations[0].state.handoff,
            WeaponHandoffState::Holstering
        );
        handoff.resume();
        assert!(!handoff.is_holstered());
        let checkpoint = runtime.capture();
        assert_eq!(checkpoint.weapon_animations.len(), 1);
        assert_eq!(checkpoint.weapon_animations[0].next_frame_at, 100.1);
        runtime.with_source(|source| match source {
            GrappleSource::Q2Ctf { game, .. } => {
                assert!(game.equipment.ctf.is_some());
            }
            _ => panic!("q2 source"),
        });
    }

    #[test]
    fn ctf_slot_step_fires_and_kicks() {
        let (runtime, _, clock) = ctf_runtime(GrappleBinding::Slot);
        let owner = q2_actor(&runtime);
        runtime.admit(owner.id()).expect("admit");
        // Slot selection resumes the holstered weapon before the first step.
        runtime.handoff(owner.id()).expect("handoff").resume();
        runtime.input(owner.id(), true).expect("press");
        runtime.step(owner.id(), true).expect("step");
        let checkpoint = runtime.capture();
        assert!(!checkpoint.controls[0].pressed);
        assert!(checkpoint.controls[0].held);
        // Resume armed 100.1; the first step holds for the frame time.
        assert_eq!(checkpoint.weapon_animations[0].next_frame_at, 100.1);
        for _ in 0..20 {
            if runtime.hook(owner.id()).is_some() {
                break;
            }
            *clock.borrow_mut() += 0.1;
            runtime.step(owner.id(), true).expect("step");
        }
        assert!(runtime.hook(owner.id()).is_some());
        let checkpoint = runtime.capture();
        assert_ne!(checkpoint.weapon_animations[0].kick_origin, vec3(0.0, 0.0, 0.0));
    }

    #[test]
    fn ctf_offhand_press_release() {
        let (runtime, _, _) = ctf_runtime(GrappleBinding::Offhand);
        let owner = q2_actor(&runtime);
        runtime.admit(owner.id()).expect("admit");
        assert!(runtime.weapon_view(owner.id()).is_none());
        runtime.input(owner.id(), true).expect("press");
        runtime.step(owner.id(), true).expect("step");
        assert!(!runtime.prediction(owner.id()));
        runtime.input(owner.id(), false).expect("release");
        runtime.step(owner.id(), true).expect("release step");
        assert_eq!(runtime.hook(owner.id()), None);
    }

    #[test]
    fn ctf_capture_restore_round_trip() {
        let (runtime, _, _) = ctf_runtime(GrappleBinding::Slot);
        let owner = q2_actor(&runtime);
        runtime.admit(owner.id()).expect("admit");
        runtime.input(owner.id(), true).expect("press");
        let checkpoint = runtime.capture();
        assert_eq!(checkpoint.version, 2);
        assert!(matches!(checkpoint.source, GrappleSourceCheckpoint::Q2Ctf { .. }));
        runtime.step(owner.id(), true).expect("step");
        runtime.restore(checkpoint).expect("restore");
        let again = runtime.capture();
        assert!(again.controls[0].pressed);
        assert_eq!(again.weapon_animations[0].next_frame_at, 0.0);
    }

    #[test]
    fn restore_rejects_mismatched_source() {
        let (ctf, _, _) = ctf_runtime(GrappleBinding::Slot);
        let owner = q2_actor(&ctf);
        ctf.admit(owner.id()).expect("admit");
        let checkpoint = ctf.capture();
        let (q1, _) = q1_runtime(GrappleBinding::Offhand);
        assert!(matches!(q1.restore(checkpoint), Err(GrappleError::SourceMismatch)));
    }

    #[test]
    fn restore_rejects_duplicate_controls() {
        let (runtime, _, _) = ctf_runtime(GrappleBinding::Offhand);
        let owner = q2_actor(&runtime);
        runtime.admit(owner.id()).expect("admit");
        let mut checkpoint = runtime.capture();
        checkpoint.controls.push(checkpoint.controls[0].clone());
        assert!(matches!(
            runtime.restore(checkpoint),
            Err(GrappleError::InvalidControlsOwner)
        ));
    }

    #[test]
    fn teleport_bit_change_releases() {
        let (runtime, _, _) = ctf_runtime(GrappleBinding::Offhand);
        let owner = q2_actor(&runtime);
        runtime.admit(owner.id()).expect("admit");
        runtime.input(owner.id(), true).expect("press");
        runtime.observe_teleport(owner.id(), 0).expect("observe");
        assert!(runtime.held(owner.id()));
        runtime.observe_teleport(owner.id(), 0).expect("same bit");
        assert!(runtime.held(owner.id()));
        runtime.observe_teleport(owner.id(), 4).expect("changed bit");
        assert!(!runtime.held(owner.id()));
    }

    #[test]
    fn input_rejects_unadmitted_actor() {
        let (runtime, _, _) = ctf_runtime(GrappleBinding::Offhand);
        let owner = q2_actor(&runtime);
        assert!(matches!(
            runtime.input(owner.id(), true),
            Err(GrappleError::MissingControls)
        ));
    }

    #[test]
    fn lmctf_offhand_basics() {
        let runtime = lmctf_runtime(GrappleBinding::Offhand);
        let owner = q2_actor(&runtime);
        assert_eq!(runtime.gravity_scale(owner.id()), 1);
        runtime.admit(owner.id()).expect("admit");
        runtime.input(owner.id(), true).expect("press");
        runtime.step(owner.id(), true).expect("step");
        assert!(runtime.hook(owner.id()).is_some());
        runtime.release(owner.id()).expect("release");
        assert_eq!(runtime.hook(owner.id()), None);
    }

    #[test]
    fn q1_slot_fire_and_release() {
        let (runtime, _) = q1_runtime(GrappleBinding::Slot);
        let actor = q1_actor(&runtime);
        runtime.admit(&actor).expect("admit");
        let view = runtime.weapon_view(&actor).expect("view");
        match view {
            GrappleWeaponView::Standard { path, frame, .. } => {
                assert_eq!(path, "progs/v_star.mdl");
                assert_eq!(frame, 0);
            }
            GrappleWeaponView::Qvm(_) => panic!("standard view"),
        }
        runtime.input(&actor, true).expect("press");
        runtime.step(&actor, true).expect("step");
        assert!(runtime.hook(&actor).is_some());
        let view = runtime.weapon_view(&actor).expect("view");
        match view {
            // Attack raises frame 2; the trailing animate advances to 3.
            GrappleWeaponView::Standard { frame, .. } => assert_eq!(frame, 3),
            GrappleWeaponView::Qvm(_) => panic!("standard view"),
        }
        runtime.release(&actor).expect("release");
        assert_eq!(runtime.hook(&actor), None);
    }

    #[test]
    fn q1_handoff_resume_frames() {
        let (runtime, host) = q1_runtime(GrappleBinding::Slot);
        let actor = q1_actor(&runtime);
        runtime.admit(&actor).expect("admit");
        let mut handoff = runtime.handoff(&actor).expect("handoff");
        assert_eq!(handoff.weapon().item, "q1:ctf/weapon/grapple");
        handoff.holster();
        assert!(handoff.is_holstered());
        handoff.resume();
        assert_eq!(host.frames.borrow().as_slice(), &[(actor.clone(), 0)]);
    }

    #[test]
    fn q1_capture_restore_round_trip() {
        let (runtime, _) = q1_runtime(GrappleBinding::Slot);
        let actor = q1_actor(&runtime);
        runtime.admit(&actor).expect("admit");
        runtime.input(&actor, true).expect("press");
        let checkpoint = runtime.capture();
        assert!(matches!(checkpoint.source, GrappleSourceCheckpoint::Q1Threewave { .. }));
        runtime.step(&actor, true).expect("step");
        runtime.restore(checkpoint).expect("restore");
        let again = runtime.capture();
        assert!(again.controls[0].pressed);
    }

    #[test]
    fn qvm_slot_fire_release_view() {
        let rig = qvm_runtime(GrappleBinding::Slot);
        let (runtime, shared, core_log, first) = (&rig.runtime, &rig.shared, &rig.released, &rig.first);
        runtime.admit(first.id()).expect("admit");
        let mut handoff = runtime.handoff(first.id()).expect("handoff");
        assert!(handoff.is_holstered());
        handoff.resume();
        assert!(!handoff.is_holstered());
        runtime.input(first.id(), true).expect("press");
        runtime.step(first.id(), true).expect("step");
        assert_eq!(shared.borrow().fired.as_slice(), &[first.id().clone()]);
        assert!(runtime.hook(first.id()).is_some());
        assert!(runtime.pulling(first.id()));
        let view = runtime.weapon_view(first.id()).expect("view");
        match view {
            GrappleWeaponView::Qvm(view) => {
                assert_eq!(view.path, "progs/v_hook.mdl");
                assert!(view.q3_weapon.firing);
            }
            GrappleWeaponView::Standard { .. } => panic!("qvm view"),
        }
        runtime.release(first.id()).expect("release");
        assert_eq!(runtime.hook(first.id()), None);
        assert_eq!(core_log.borrow().as_slice(), &[(first.id().clone(), true)]);
    }

    #[test]
    fn qvm_offhand_release_edge() {
        let rig = qvm_runtime(GrappleBinding::Offhand);
        let (runtime, shared, first) = (&rig.runtime, &rig.shared, &rig.first);
        runtime.admit(first.id()).expect("admit");
        runtime.input(first.id(), true).expect("press");
        runtime.step(first.id(), true).expect("step");
        assert_eq!(shared.borrow().fired.len(), 1);
        runtime.input(first.id(), false).expect("release");
        runtime.step(first.id(), true).expect("release step");
        assert_eq!(shared.borrow().released.as_slice(), &[first.id().clone()]);
        assert_eq!(runtime.hook(first.id()), None);
    }

    #[test]
    fn qvm_capture_restore_round_trip() {
        let rig = qvm_runtime(GrappleBinding::Slot);
        let (runtime, shared, first, second) = (&rig.runtime, &rig.shared, &rig.first, &rig.second);
        runtime.admit(first.id()).expect("admit");
        runtime.admit(second.id()).expect("admit");
        let mut handoff = runtime.handoff(first.id()).expect("handoff");
        handoff.resume();
        let checkpoint = runtime.capture();
        match &checkpoint.source {
            GrappleSourceCheckpoint::Q3Qvm { holstered, .. } => {
                assert_eq!(holstered.len(), 1);
            }
            _ => panic!("qvm source"),
        }
        handoff.holster();
        runtime.restore(checkpoint).expect("restore");
        assert_eq!(shared.borrow().restored.len(), 1);
        let again = runtime.handoff(first.id()).expect("handoff");
        assert!(!again.is_holstered());
        let second_handoff = runtime.handoff(second.id()).expect("handoff");
        assert!(second_handoff.is_holstered());
    }

    #[test]
    fn on_actor_released_clears_tables() {
        let (runtime, _, _) = ctf_runtime(GrappleBinding::Slot);
        let owner = q2_actor(&runtime);
        runtime.admit(owner.id()).expect("admit");
        runtime.input(owner.id(), true).expect("press");
        runtime.on_actor_released(owner.id());
        assert!(matches!(
            runtime.input(owner.id(), true),
            Err(GrappleError::MissingControls)
        ));
        assert!(matches!(runtime.handoff(owner.id()), Err(GrappleError::MissingWeapon)));
    }
}
