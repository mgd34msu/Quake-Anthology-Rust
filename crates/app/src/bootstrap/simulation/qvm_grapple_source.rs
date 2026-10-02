//! QVM offhand-grapple source: one initialized source VM borrows session
//! players and owns only its hook actors.
//!
//! Provenance: `src/app/bootstrap/simulation/qvm-grapple-source.ts`.
//!
//! Bridges from the donor universe. The recording [`QvmModule`] never
//! executes bytecode, so no guest trap locates the entity/client tables
//! during initialization; when [`QvmGrappleSource::create`] finds unlocated
//! tables it applies the canonical layout (scratch immediately below the
//! reserved VM stack per the donor's formula, then 1024 entity records,
//! then the 64 client records) and documents it. A future executing
//! integration may re-locate table words and counts; strides always come
//! from the profile. Trap dispatch copies the guest allocation out-and-back
//! through the `client_state` universe (`HostCall`/`SyscallMemory`), the
//! same bridge as the Q3 guest runtime, while the non-cvar common traps
//! mirror `common-syscalls.ts` arm for arm. Trap handlers record events
//! into an outbox drained after the state borrow releases, so session event
//! sinks may call back into the source; every other session callback
//! (targets, velocity, damage, actors, bodies, scene) runs under the state
//! borrow and must not re-enter the source.
//!
//! [`QvmModule`]: qa_guest::qvm::game_data::QvmModule

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::{Rc, Weak};

use qa_content::contract::{
    ContentId, GameFamily, QvmAbiProfile as ContractAbiProfile, QvmGrappleCable, QvmGrappleDefinition,
    QvmGrapplePresentation as ContractPresentation, QvmGrappleViewAnchor, QvmGrappleViewAttachment,
};
use qa_content::mounts::MountedContent;
use qa_content::q3::base::world::{TraceContact, TraceShape as WorldTraceShape};
use qa_content::q3::base::world_adapter::{Q3TraceHit, Q3TraceQuery, Q3TraceResult};
use qa_content::q3::foundation::player_pose::qvm_angle_vectors;
use qa_core::cmd::Dialect;
use qa_core::cmd_buffer::{
    BufferError, BufferOptions, BufferServices, CommandBuffer, CommandContext, ForwardedCommand, ScriptRead,
};
use qa_core::cvar::{CvarError, CvarRegistry};
use qa_core::identity::{ActorId, OwnedActor, ProviderId, SavedActorId};
use qa_core::math::{vec3, Bounds, Vec3};
use qa_core::numeric::Q3_BINARY32_PROFILE;
use qa_guest::error::GuestError;
use qa_guest::qvm::client_collision_syscalls::{TraceRecord, TraceShape as GuestTraceShape};
use qa_guest::qvm::client_state::{
    AbiProfile as ClientAbi, CallKind as ClientKind, HostCall as ClientCall, QvmRole as ClientRole, SyscallMemory,
    WireUserCommand as ClientWireCommand,
};
use qa_guest::qvm::common_syscalls::QvmCalendar;
use qa_guest::qvm::cvar_syscalls::{cvar_syscall, CvarHost, CvarValue, CvarVmBinding};
use qa_guest::qvm::entity_tokens::{CommonParseCursor, CommonParseState};
use qa_guest::qvm::file_syscalls::{
    file_syscall, FileMounts, FilesCheckpoint, HandleCheckpoint, OpenedFile, QvmFiles, ReadHandleCheckpoint,
    WriteHandleCheckpoint, WriteMode,
};
use qa_guest::qvm::game::QvmGame;
use qa_guest::qvm::game_data::{
    AbiProfile as GameAbi, CallKind as GameKind, ModuleIdentity as GameModuleIdentity, ProfileValue, QvmArtifact,
    QvmCheckpoint, QvmFunctionCall, QvmGameDataState, QvmGameImport, QvmHostCall as GameCall, QvmHostState,
    QvmRole as GameRole,
};
use qa_guest::qvm::grapple_profile::{
    QvmCable, QvmGrappleCallbacks as ProfileCallbacks, QvmGrappleFields as ProfileFields,
    QvmGrappleGlobals as ProfileGlobals, QvmGrappleMovement as ProfileMovement,
    QvmGrapplePresentation as ProfilePresentation, QvmGrappleProfile, QvmGrappleWord as ProfileWord, QvmViewAnchor,
    QvmViewAttachment,
};
use qa_guest::qvm::grapple_provider::{
    QvmGrappleBridge, QvmGrappleCheckpoint as ProviderCheckpoint, QvmGrappleGame, QvmGrappleProjection,
    QvmGrappleProvider, QvmGrappleScratch, QvmSavedActor,
};
use qa_guest::qvm::legacy_bot_abi::{BOTLIB_AAS_INITIALIZED, BOTLIB_SETUP};
use qa_guest::qvm::server_game_syscalls::{
    server_game_syscall, Bounds as ServerBounds, ServerGameHost, ServerSpatialHost, ServerTraceQuery,
};
use qa_guest::qvm::shared_entity_record::{write_qvm_shared_entity, QvmSharedEntity};
use qa_world::body::BodyState;
use qa_world::save::records::{read_saved_actor, write_saved_actor};
use qa_world::save::shared::{read_vector, write_vector};
use qa_world::save::value::{
    arr, boolean, decode_checkpoint_value, encode_checkpoint_value, int, num, obj, str as save_str, SaveJson,
    SaveReader,
};
use qa_world::WorldError;

use super::q3::host::Q3SourceEvent;
use super::qvm_weapon_behavior::{qvm_weapon_initialization_entities, QvmWeaponError};
use super::types::{Q3GrappleCable, Q3WeaponPresentation, ShaderBeam, SimulationPresentation};

/// Client capacity (donor `64`).
const MAX_CLIENTS: usize = 64;
/// Configstring capacity (donor `1024`).
const MAX_CONFIGSTRINGS: i32 = 1024;
/// Entity number for a trace that completed (donor `1023`).
const ENTITYNUM_NONE: i32 = 1023;
/// Entity number for a world trace hit (donor `1022`).
const ENTITYNUM_WORLD: i32 = 1022;
/// Entity capacity for the canonical table layout (donor `MAX_GENTITIES`).
const NUM_ENTITIES: usize = 1024;
/// VM stack reservation below the allocation top (donor `65536`).
const VM_STACK_RESERVED: usize = 65536;
/// Host checkpoint format tag (donor `q3:grapple-host`).
const HOST_FORMAT: &str = "q3:grapple-host";
/// Source checkpoint version (donor `1`).
const CHECKPOINT_VERSION: u32 = 1;
/// Calendar span for `G_REAL_TIME` (nine words).
const CALENDAR_BYTES: usize = 36;
/// `G_ARGC` trap (donor `src/compat/qvm/abi.ts`).
const G_ARGC: i32 = 8;
/// `G_ARGV` trap (donor `src/compat/qvm/abi.ts`).
const G_ARGV: i32 = 9;
/// `G_SEND_CONSOLE_COMMAND` trap (donor `src/compat/qvm/abi.ts`).
const G_SEND_CONSOLE_COMMAND: i32 = 14;
/// `G_REAL_TIME` trap (donor `src/compat/qvm/abi.ts`).
const G_REAL_TIME: i32 = 41;
/// Maximum `G_ERROR` text (donor truncates to 4095).
const MAX_ERROR_CHARS: usize = 4095;
/// Player-state `pm_type` offset (donor `4`).
const PS_PM_TYPE: usize = 4;
/// Player-state origin offset (donor `20`).
const PS_ORIGIN: usize = 20;
/// Player-state velocity offset (donor `32`).
const PS_VELOCITY: usize = 32;
/// Player-state viewheight offset (donor `164`).
const PS_VIEWHEIGHT: usize = 164;
/// Player-state stats base (donor health at `184`).
const PS_HEALTH: usize = 184;
/// Player-state viewangles offset (donor `152`).
const PS_VIEWANGLES: usize = 152;
/// Player-state persistant team offset (donor `260`).
const PS_TEAM: usize = 260;
/// Contents flag for a live player body (donor `0x2000000`).
const CONTENTS_PLAYERCLIP_BODY: i32 = 0x200_0000;
/// Contents flag for a corpse body (donor `0x4000000`).
const CONTENTS_CORPSE: i32 = 0x400_0000;
/// Hang-sound distance: hook point within 64 units of the chest (donor `64`).
const HANG_DISTANCE: f32 = 64.0;
/// Chest height above the owner origin for the hang test (donor `26`).
const CHEST_HEIGHT: f32 = 26.0;
/// Hook equipment definition (donor `q3:equipment/hook`).
const HOOK_DEFINITION: &str = "q3:equipment/hook";

/// QVM grapple source failure.
#[derive(Debug, thiserror::Error)]
pub enum QvmGrappleSourceError {
    /// Guest failure.
    #[error(transparent)]
    Guest(#[from] GuestError),
    /// Checkpoint failure.
    #[error(transparent)]
    World(#[from] WorldError),
    /// Cvar failure.
    #[error(transparent)]
    Cvar(#[from] CvarError),
    /// Console failure.
    #[error(transparent)]
    Buffer(#[from] BufferError),
    /// Weapon initialization failure.
    #[error(transparent)]
    Weapon(#[from] QvmWeaponError),
    /// Invalid source state or checkpoint.
    #[error("{0}")]
    Invalid(String),
}

impl QvmGrappleSourceError {
    /// Build an invalid-state error.
    fn invalid(message: impl Into<String>) -> Self {
        Self::Invalid(message.into())
    }
}

/// Grapple team (donor `"free" | "red" | "blue" | "spectator"`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum QvmGrappleTeam {
    /// Free-for-all.
    Free,
    /// Red team.
    Red,
    /// Blue team.
    Blue,
    /// Spectator.
    Spectator,
}

impl QvmGrappleTeam {
    /// Persistant team number (donor `{ free: 0, red: 1, blue: 2, spectator: 3 }`).
    #[must_use]
    pub fn number(self) -> i32 {
        match self {
            Self::Free => 0,
            Self::Red => 1,
            Self::Blue => 2,
            Self::Spectator => 3,
        }
    }
}

/// Grapple target kind (donor `QvmGrappleTarget` union).
#[derive(Debug, Clone, PartialEq)]
pub enum QvmGrappleTargetKind {
    /// Shared actor target.
    Actor {
        /// Mover flag.
        mover: bool,
    },
    /// Borrowed player target.
    Player {
        /// Client userinfo.
        userinfo: String,
        /// Team.
        team: QvmGrappleTeam,
        /// View height.
        view_height: f64,
    },
}

/// Borrowed grapple target (donor `QvmGrappleTarget`).
#[derive(Debug, Clone, PartialEq)]
pub struct QvmGrappleTarget {
    /// Target actor.
    pub actor: ActorId,
    /// Target body.
    pub body: BodyState,
    /// Target health.
    pub health: f64,
    /// Target kind.
    pub kind: QvmGrappleTargetKind,
}

/// Grapple damage report (donor `QvmGrappleDamage`).
#[derive(Debug, Clone, PartialEq)]
pub struct QvmGrappleDamage {
    /// Victim.
    pub target: ActorId,
    /// Inflictor, if any.
    pub inflictor: Option<ActorId>,
    /// Attacker, if any.
    pub attacker: Option<ActorId>,
    /// Damage direction.
    pub direction: Vec3,
    /// Damage point.
    pub point: Vec3,
    /// Amount.
    pub amount: i32,
    /// Flags.
    pub flags: i32,
    /// Method.
    pub method: i32,
}

/// Hook binding side (donor `binding?: "offhand" | "slot"`, default offhand).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum QvmGrappleBinding {
    /// Offhand hook (donor default).
    #[default]
    Offhand,
    /// Weapon-slot hook.
    Slot,
}

/// Session actor registry surface the source consumes (donor
/// `SessionActorRegistry`, narrowed to the five methods used here).
pub trait QvmGrappleActors {
    /// Allocate an owned actor for a definition.
    fn allocate(&self, provider: &ProviderId, definition: &str) -> OwnedActor;
    /// Whether an actor is live.
    fn is_live(&self, actor: &ActorId) -> bool;
    /// Release an owned actor.
    fn release(&self, actor: &OwnedActor);
    /// Resolve a saved actor.
    fn resolve_saved(&self, saved: &SavedActorId) -> Option<OwnedActor>;
    /// Observe actor release; returns an unsubscribe callback.
    fn on_release(&self, callback: Box<dyn Fn(&OwnedActor)>) -> Box<dyn Fn()>;
}

/// Shared body table surface the source consumes (donor `SharedBodyTable`,
/// narrowed to the three methods used here).
pub trait QvmGrappleBodies {
    /// Read a body.
    fn read(&self, actor: &ActorId) -> Option<BodyState>;
    /// Write a body.
    fn write(&self, actor: &OwnedActor, body: BodyState);
    /// Link a body.
    fn link(&self, actor: &OwnedActor);
}

/// Shared scene surface the source consumes (donor `SharedSceneQueries`,
/// world target with the Q3 binary32 numeric profile).
pub trait QvmGrappleScene {
    /// Trace against the world.
    fn trace_scene(&self, query: &Q3TraceQuery) -> Q3TraceResult;
    /// Contents at a point against the world.
    fn point_contents_scene(&self, query: &Q3TraceQuery, point: Vec3) -> i32;
    /// Actors touching bounds.
    fn query_actors(&self, bounds: &Bounds) -> Vec<ActorId>;
    /// Leaf containing a point.
    fn point_leaf(&self, point: Vec3) -> i32;
    /// Cluster of a leaf.
    fn leaf_cluster(&self, leaf: i32) -> i32;
    /// Area of a leaf.
    fn leaf_area(&self, leaf: i32) -> i32;
    /// PVS visibility between clusters.
    fn cluster_visible(&self, from: i32, cluster: i32) -> bool;
    /// Whether two areas are connected.
    fn areas_connected(&self, first: i32, second: i32) -> bool;
}

/// Borrowed-target listing (donor `targets()`).
pub type QvmGrappleTargets = Rc<dyn Fn() -> Vec<QvmGrappleTarget>>;
/// Engine event sink (donor `event(event)`).
pub type QvmGrappleEvent = Rc<dyn Fn(Q3SourceEvent)>;
/// Owner velocity sink (donor `velocity(actor, velocity)`).
pub type QvmGrappleVelocity = Rc<dyn Fn(&ActorId, Vec3)>;
/// Damage sink (donor `damage(damage)`).
pub type QvmGrappleDamageFn = Rc<dyn Fn(QvmGrappleDamage)>;
/// Current-context assertion (donor `assertCurrent()`).
pub type QvmGrappleAssertCurrent = Rc<dyn Fn()>;
/// Real-time clock with an optional calendar sink (donor `realTime`).
pub type QvmGrappleRealTime = Rc<dyn Fn(Option<&mut dyn FnMut(QvmCalendar)>) -> i32>;

/// Grapple source construction options (donor `QvmGrappleSourceOptions`).
pub struct QvmGrappleSourceOptions {
    /// Module artifact.
    pub artifact: QvmArtifact,
    /// Grapple definition.
    pub profile: QvmGrappleDefinition,
    /// Hook provider.
    pub provider: ProviderId,
    /// Mounted content.
    pub mounts: MountedContent,
    /// Session actors.
    pub actors: Rc<dyn QvmGrappleActors>,
    /// Shared bodies.
    pub bodies: Rc<dyn QvmGrappleBodies>,
    /// Shared scene.
    pub scene: Rc<dyn QvmGrappleScene>,
    /// Console context.
    pub context: CommandContext,
    /// Random seed.
    pub seed: i32,
    /// Entity text.
    pub entity_text: String,
    /// Hook binding side.
    pub binding: QvmGrappleBinding,
    /// Borrowed targets.
    pub targets: QvmGrappleTargets,
    /// Engine event sink.
    pub event: QvmGrappleEvent,
    /// Owner velocity sink.
    pub velocity: QvmGrappleVelocity,
    /// Damage sink.
    pub damage: QvmGrappleDamageFn,
    /// Current-context assertion.
    pub assert_current: QvmGrappleAssertCurrent,
    /// Real-time clock.
    pub real_time: QvmGrappleRealTime,
}

/// Mirrored shared actor (donor `Binding`).
#[derive(Debug, Clone)]
struct Binding {
    /// Shared actor.
    actor: ActorId,
    /// Source entity pointer.
    pointer: i32,
    /// Client binding flag.
    client: bool,
    /// Last mirrored origin.
    origin: Vec3,
}

/// Owned hook tether (donor `Tether`).
#[derive(Debug, Clone)]
struct Tether {
    /// Owned hook actor.
    actor: OwnedActor,
    /// Live projection.
    projection: QvmGrappleProjection,
}

/// Saved grapple binding (donor checkpoint `bindings` row).
#[derive(Debug, Clone, PartialEq)]
pub struct QvmGrappleSourceBinding {
    /// Saved actor.
    pub actor: SavedActorId,
    /// Source entity pointer.
    pub pointer: i32,
    /// Client binding flag.
    pub client: bool,
    /// Last mirrored origin.
    pub origin: Vec3,
}

/// Saved hook tether (donor checkpoint `tethers` row).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QvmGrappleSourceTether {
    /// Saved owner.
    pub owner: SavedActorId,
    /// Saved hook actor.
    pub actor: SavedActorId,
}

/// Saved provider continuation (donor checkpoint `grapple`).
#[derive(Debug, Clone)]
pub struct QvmGrappleSourceGrapple {
    /// Version (always 1).
    pub version: u32,
    /// Profile declaration.
    pub profile: String,
    /// Module checkpoint.
    pub module: QvmCheckpoint,
    /// Saved owners in admission order.
    pub owners: Vec<SavedActorId>,
}

/// Grapple source checkpoint (donor `QvmGrappleSourceCheckpoint`).
#[derive(Debug, Clone)]
pub struct QvmGrappleSourceCheckpoint {
    /// Version (always 1).
    pub version: u32,
    /// Provider continuation.
    pub grapple: QvmGrappleSourceGrapple,
    /// Saved bindings.
    pub bindings: Vec<QvmGrappleSourceBinding>,
    /// Saved tethers.
    pub tethers: Vec<QvmGrappleSourceTether>,
}

/// Hook weapon view (donor `weaponView` row).
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
    /// Model attachments.
    pub model_attachments: Vec<QvmGrappleViewAttachment>,
    /// Model anchor.
    pub model_anchor: QvmGrappleViewAnchor,
    /// Q3 weapon presentation.
    pub q3_weapon: Q3WeaponPresentation,
}

/// Convert a contract definition into the guest provider profile.
/// Offsets truncate like the donor's `Math.trunc` call sites; the provider
/// re-validates ranges against its declaration round-trip.
fn profile_from_definition(definition: &QvmGrappleDefinition) -> Result<QvmGrappleProfile, QvmGrappleSourceError> {
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    fn offset(value: u32) -> usize {
        value as usize
    }
    let abi_profile = match definition.abi_profile {
        ContractAbiProfile::Modern => GameAbi::Modern,
        ContractAbiProfile::Legacy116n => GameAbi::Legacy,
    };
    let entity_stride = usize::try_from(definition.entity_stride)
        .map_err(|_| QvmGrappleSourceError::invalid("Grapple entity stride exceeds the address space"))?;
    let client_stride = usize::try_from(definition.client_stride)
        .map_err(|_| QvmGrappleSourceError::invalid("Grapple client stride exceeds the address space"))?;
    let fields = &definition.fields;
    let callbacks = &definition.callbacks;
    let globals = &definition.globals;
    let presentation = &definition.presentation;
    #[allow(clippy::cast_possible_truncation)]
    let profile = QvmGrappleProfile {
        id: definition.id.clone(),
        title: definition.title.clone(),
        module: GameModuleIdentity {
            id: format!("{}:{}", definition.module.id.namespace, definition.module.id.name),
            artifact_path: definition.module.artifact_path.clone(),
            digest: definition.module.digest.as_str().to_string(),
            revision: definition.module.revision.clone(),
        },
        abi_profile,
        entity_stride,
        client_stride,
        fields: ProfileFields {
            inuse: offset(fields.inuse),
            client: offset(fields.client),
            parent: offset(fields.parent),
            target: offset(fields.target),
            mover: fields.mover.map(offset),
            hook: offset(fields.hook),
            health: offset(fields.health),
            takedamage: offset(fields.takedamage),
            event_time: offset(fields.event_time),
            free_after_event: offset(fields.free_after_event),
        },
        globals: ProfileGlobals {
            time: offset(globals.time),
            frame: offset(globals.frame),
            movement: offset(globals.movement),
            forward: offset(globals.forward),
            ground_plane: offset(globals.ground_plane),
        },
        callbacks: ProfileCallbacks {
            allocate: offset(callbacks.allocate),
            free: offset(callbacks.free),
            fire: offset(callbacks.fire),
            release: offset(callbacks.release),
            force_release: offset(callbacks.force_release),
            missile: offset(callbacks.missile),
            follow: callbacks.follow.map(offset),
            think: offset(callbacks.think),
            pull: offset(callbacks.pull),
            move_mover_hooks: callbacks.move_mover_hooks.map(offset),
            damage: offset(callbacks.damage),
            same_team: offset(callbacks.same_team),
            player_move: offset(callbacks.player_move),
        },
        fire_arguments: definition.fire_arguments.iter().map(|value| *value as i32).collect(),
        movement: ProfileMovement {
            byte_length: usize::try_from(definition.movement.byte_length)
                .map_err(|_| QvmGrappleSourceError::invalid("Grapple movement span exceeds the address space"))?,
            words: definition
                .movement
                .words
                .iter()
                .map(|word| ProfileWord {
                    offset: offset(word.offset),
                    value: word.value as i32,
                })
                .collect(),
        },
        initial_cvars: definition
            .initial_cvars
            .iter()
            .map(|(name, value)| (name.clone(), value.clone()))
            .collect(),
        event_lifetime_ms: definition.event_lifetime_milliseconds as i64,
        grapple_damage_method: definition.grapple_damage_method as i64,
        presentation: ProfilePresentation {
            projectile_model: presentation.projectile_model.clone(),
            view_model: presentation.view_model.clone(),
            weapon_index: presentation.weapon_index as i64,
            view_anchor: QvmViewAnchor {
                path: presentation.view_anchor.path.clone(),
                tag: presentation.view_anchor.tag.clone(),
                offset: presentation.view_anchor.offset,
                fov_above: presentation.view_anchor.fov_offset.above as i64,
                fov_scale: presentation.view_anchor.fov_offset.scale,
            },
            view_attachments: presentation
                .view_attachments
                .iter()
                .map(|attachment| QvmViewAttachment {
                    path: attachment.path.clone(),
                    tag: attachment.tag.clone(),
                })
                .collect(),
            cable: match &presentation.cable {
                QvmGrappleCable::Shader { path, width } => QvmCable::Shader {
                    path: path.clone(),
                    width: *width as i64,
                },
                QvmGrappleCable::Model {
                    flight,
                    pull,
                    hold,
                    segment_length,
                } => QvmCable::Model {
                    flight: flight.clone(),
                    pull: pull.clone(),
                    hold: hold.clone(),
                    segment_length: *segment_length as i64,
                },
            },
            fire_sound: presentation.fire_sound.clone(),
            attach_sound: presentation.attach_sound.clone(),
            release_sound: presentation.release_sound.clone(),
            pull_sound: presentation.pull_sound.clone(),
            hang_sound: presentation.hang_sound.clone(),
        },
        pulling_flag: definition.pulling_flag as i32,
    };
    Ok(profile)
}

/// Shared mounts handle: the file owner and console script reads borrow the
/// same mounted content.
#[derive(Clone)]
struct SharedMounts {
    /// Mounted content.
    mounts: Rc<RefCell<MountedContent>>,
}

impl FileMounts for SharedMounts {
    fn open(&mut self, path: &str) -> Option<OpenedFile> {
        let found = self.mounts.borrow_mut().open(path, |_| true).ok()??;
        Some(OpenedFile {
            bytes: found.bytes,
            pk3: matches!(
                found.reference.provenance,
                qa_content::contract::ResourceProvenance::Archive { .. }
            ),
        })
    }

    fn list_files(&mut self, path: &str, extension: &str) -> Vec<String> {
        self.mounts.borrow_mut().list_files(path, extension).unwrap_or_default()
    }
}

/// Console services over mounted scripts (donor `readScript`).
struct ConsoleServices<'a> {
    /// Shared mounts.
    mounts: Rc<RefCell<MountedContent>>,
    /// Deferred engine events.
    outbox: &'a mut Vec<Q3SourceEvent>,
}

impl BufferServices for ConsoleServices<'_> {
    fn read_script(&mut self, name: &str, _source: &CommandContext) -> ScriptRead {
        match self.mounts.borrow_mut().open(name, |_| true) {
            Ok(Some(resource)) => ScriptRead::Ready(Some(String::from_utf8_lossy(&resource.bytes).into_owned())),
            Ok(None) => ScriptRead::Ready(None),
            Err(error) => ScriptRead::Failed(error.to_string()),
        }
    }

    fn forward_to_server(&mut self, command: &ForwardedCommand) {
        self.outbox.push(Q3SourceEvent::Print {
            text: format!("Grapple source dropped forwarded command: {}\n", command.raw),
        });
    }
}

/// Owned source handles: state plus the provider cell. Methods manage
/// short state borrows and never hold one across a provider call, so
/// provider bridge callbacks always find the state unborrowed.
#[derive(Clone)]
struct Handles {
    /// Mutable state.
    inner: Rc<RefCell<Inner>>,
    /// Provider cell (shared borrows nest, so provider callbacks that
    /// re-enter provider methods are safe).
    provider: Rc<RefCell<Option<QvmGrappleProvider>>>,
}

/// Weak source handles for VM hooks, trap dispatch, and the provider bridge.
#[derive(Clone)]
struct WeakHandles {
    /// Weak state.
    inner: Weak<RefCell<Inner>>,
    /// Weak provider cell.
    provider: Weak<RefCell<Option<QvmGrappleProvider>>>,
}

impl WeakHandles {
    /// Upgrade to owned handles.
    fn upgrade(&self) -> Option<Handles> {
        Some(Handles {
            inner: self.inner.upgrade()?,
            provider: self.provider.upgrade()?,
        })
    }
}

/// Provider bridge into source state.
struct Bridge {
    /// Weak handles.
    handles: WeakHandles,
}

impl Bridge {
    /// Borrow source handles (bridges never outlive the source).
    fn handles(&self) -> Handles {
        self.handles
            .upgrade()
            .unwrap_or_else(|| panic!("QVM grapple bridge outlives its source"))
    }
}

impl QvmGrappleBridge for Bridge {
    fn synchronize(&self) {
        self.handles().synchronize().unwrap_or_else(|error| panic!("{error}"));
    }

    fn entity(&self, actor: &ActorId) -> Option<usize> {
        self.handles()
            .inner
            .borrow()
            .bindings
            .get(actor)
            .map(|binding| usize::try_from(binding.pointer).unwrap_or_else(|_| panic!("Grapple pointer is negative")))
    }

    fn actor(&self, pointer: i32) -> Option<ActorId> {
        self.handles().actor_for_pointer(pointer)
    }

    fn restore_actor(&self, saved: &QvmSavedActor) -> ActorId {
        self.handles()
            .restore_actor(&SavedActorId {
                slot: saved.slot,
                generation: saved.generation,
            })
            .unwrap_or_else(|error| panic!("{error}"))
    }

    fn publish(&self, owner: &ActorId, projection: Option<&QvmGrappleProjection>) {
        self.handles().publish(owner, projection.cloned());
    }

    fn velocity(&self, owner: &ActorId, velocity: &Vec3) {
        let handles = self.handles();
        let velocity_fn = handles.inner.borrow().velocity.clone();
        velocity_fn(owner, *velocity);
    }

    fn scratch(&self) -> QvmGrappleScratch {
        self.handles().inner.borrow().scratch
    }
}

/// Mutable grapple source state.
struct Inner {
    /// Guest game.
    game: QvmGame,
    /// Converted provider profile.
    profile: QvmGrappleProfile,
    /// Contract presentation (sounds, models, cable).
    presentation: ContractPresentation,
    /// Event lifetime in milliseconds.
    event_lifetime_ms: f64,
    /// Cvar registry.
    cvars: CvarRegistry,
    /// Guest files.
    files: QvmFiles,
    /// Console program.
    commands: CommandBuffer,
    /// Console context.
    context: CommandContext,
    /// Shared mounts for script reads.
    mounts: Rc<RefCell<MountedContent>>,
    /// Configstrings by index.
    configstrings: HashMap<i32, String>,
    /// Client userinfo by slot.
    userinfo: HashMap<i32, String>,
    /// Mirrored shared actors.
    bindings: HashMap<ActorId, Binding>,
    /// Owned hook tethers by owner.
    tethers: HashMap<ActorId, Tether>,
    /// Entity-text parser.
    parser: CommonParseState,
    /// Entity-text cursor.
    cursor: CommonParseCursor,
    /// Hook provider.
    provider_id: ProviderId,
    /// Binding side.
    binding_side: QvmGrappleBinding,
    /// Borrowed targets.
    targets: QvmGrappleTargets,
    /// Engine event sink.
    event: QvmGrappleEvent,
    /// Owner velocity sink.
    velocity: QvmGrappleVelocity,
    /// Damage sink.
    damage: QvmGrappleDamageFn,
    /// Current-context assertion.
    assert_current: QvmGrappleAssertCurrent,
    /// Real-time clock.
    real_time: QvmGrappleRealTime,
    /// Session actors.
    actors: Rc<dyn QvmGrappleActors>,
    /// Shared bodies.
    bodies: Rc<dyn QvmGrappleBodies>,
    /// Shared scene.
    scene: Rc<dyn QvmGrappleScene>,
    /// Milliseconds since start.
    milliseconds: i32,
    /// Closed flag.
    closed: bool,
    /// Synchronization guard.
    synchronizing: bool,
    /// Tether identities pinned during restore.
    restoring_tethers: Option<HashMap<ActorId, OwnedActor>>,
    /// Actor-release unsubscribe.
    unsubscribe: Option<Box<dyn Fn()>>,
    /// VM cvar bindings by handle.
    vm_cvars: HashMap<i32, String>,
    /// Next VM cvar handle.
    next_vm_handle: i32,
    /// Reserved scratch span.
    scratch: QvmGrappleScratch,
    /// Deferred trap events.
    outbox: Vec<Q3SourceEvent>,
}

/// One initialized source VM (donor `QvmGrappleSource`).
#[derive(Clone)]
pub struct QvmGrappleSource {
    /// Mutable state.
    inner: Rc<RefCell<Inner>>,
    /// Provider cell.
    provider: Rc<RefCell<Option<QvmGrappleProvider>>>,
}

/// Align a byte offset up to 16 (donor `Math.ceil(length / 16) * 16`).
fn align16(length: usize) -> usize {
    length.div_ceil(16) * 16
}

/// Classify a plane normal (donor `trace_t` plane type/signbits).
fn plane_type(normal: Vec3) -> (u8, u8) {
    let plane_type = if normal.x == 1.0 {
        0
    } else if normal.y == 1.0 {
        1
    } else if normal.z == 1.0 {
        2
    } else {
        3
    };
    let signbits = u8::from(normal.x < 0.0) | (u8::from(normal.y < 0.0) << 1) | (u8::from(normal.z < 0.0) << 2);
    (plane_type, signbits)
}

/// Fixed cvar registrations (donor constructor table).
const FIXED_CVARS: [(&str, &str); 7] = [
    ("g_log", ""),
    ("cm_noCurves", "0"),
    ("cm_playerCurveClip", "1"),
    ("bot_enable", "0"),
    ("sv_maxclients", "64"),
    ("dedicated", "1"),
    ("g_gametype", "0"),
];

impl QvmGrappleSource {
    /// Create an initialized source (donor `QvmGrappleSource.create`).
    pub fn create(options: QvmGrappleSourceOptions) -> Result<Self, QvmGrappleSourceError> {
        let profile = profile_from_definition(&options.profile)?;
        let entity_init = qvm_weapon_initialization_entities(&options.entity_text)?;
        let mut cvars = CvarRegistry::new(Dialect::Q3);
        for (name, value) in FIXED_CVARS {
            cvars.register(name, value, 0)?;
        }
        let mut initial: Vec<(String, String)> = options
            .profile
            .initial_cvars
            .iter()
            .map(|(name, value)| (name.clone(), value.clone()))
            .collect();
        initial.sort();
        for (name, value) in &initial {
            cvars.register(name, value, 0)?;
            cvars.set(name, value, true)?;
        }
        for notice in cvars.take_notifications() {
            (options.event)(Q3SourceEvent::Print { text: notice });
        }
        let event_print = Rc::clone(&options.event);
        let mut commands = CommandBuffer::new(Dialect::Q3, options.context.clone(), BufferOptions::new())?;
        commands.set_printer(move |text, _| {
            event_print(Q3SourceEvent::Print { text: text.to_string() });
        });
        let mounts = Rc::new(RefCell::new(options.mounts));
        let event_files = Rc::clone(&options.event);
        let files = QvmFiles::new(
            Box::new(SharedMounts {
                mounts: Rc::clone(&mounts),
            }),
            None,
            move |text| {
                event_files(Q3SourceEvent::Print { text: text.to_string() });
            },
        );
        let game = QvmGame::new(options.artifact.clone(), Rc::new(|_| Ok(None)))?;
        game.data.set_client_count(MAX_CLIENTS)?;
        let seed = options.seed;
        let provider_cell: Rc<RefCell<Option<QvmGrappleProvider>>> = Rc::new(RefCell::new(None));
        let inner = Rc::new(RefCell::new(Inner {
            game: game.clone(),
            profile,
            presentation: options.profile.presentation.clone(),
            event_lifetime_ms: options.profile.event_lifetime_milliseconds,
            cvars,
            files,
            commands,
            context: options.context,
            mounts,
            configstrings: HashMap::new(),
            userinfo: HashMap::new(),
            bindings: HashMap::new(),
            tethers: HashMap::new(),
            parser: CommonParseState::default(),
            cursor: CommonParseCursor::new(entity_init.into_bytes()),
            provider_id: options.provider,
            binding_side: options.binding,
            targets: options.targets,
            event: options.event,
            velocity: options.velocity,
            damage: options.damage,
            assert_current: options.assert_current,
            real_time: options.real_time,
            actors: options.actors,
            bodies: options.bodies,
            scene: options.scene,
            milliseconds: 0,
            closed: false,
            synchronizing: false,
            restoring_tethers: None,
            unsubscribe: None,
            vm_cvars: HashMap::new(),
            next_vm_handle: 0,
            scratch: QvmGrappleScratch {
                word: 0,
                byte_length: 0,
            },
            outbox: Vec::new(),
        }));
        let weak_handles = || WeakHandles {
            inner: Rc::downgrade(&inner),
            provider: Rc::downgrade(&provider_cell),
        };
        {
            let weak = weak_handles();
            game.module
                .set_host(Some(Rc::new(move |call| dispatch_syscall(&weak, call))));
        }
        {
            let same_team = inner.borrow().profile.callbacks.same_team;
            let weak = weak_handles();
            game.module
                .bind_function(same_team, Rc::new(move |call| hook_same_team(&weak, call)));
            let damage = inner.borrow().profile.callbacks.damage;
            let weak = weak_handles();
            game.module
                .bind_function(damage, Rc::new(move |call| hook_damage(&weak, call)));
        }
        let built = (|| -> Result<(), QvmGrappleSourceError> {
            let unsubscribe = {
                let actors = inner.borrow().actors.clone();
                let weak = weak_handles();
                actors.on_release(Box::new(move |actor| {
                    let id = actor.id().clone();
                    if let Some(handles) = weak.upgrade() {
                        handles.remove_actor(&id).unwrap_or_else(|error| panic!("{error}"));
                    }
                }))
            };
            inner.borrow_mut().unsubscribe = Some(unsubscribe);
            game.initialize(0, seed, false)?;
            {
                let mut guard = inner.borrow_mut();
                let Inner {
                    commands,
                    cvars,
                    outbox,
                    mounts,
                    ..
                } = &mut *guard;
                let mut services = ConsoleServices {
                    mounts: Rc::clone(mounts),
                    outbox,
                };
                commands.execute(cvars, &mut services)?;
            }
            Self::reserve_scratch(&inner, &options.artifact)?;
            let provider = QvmGrappleProvider::new(
                QvmGrappleGame {
                    module: game.module.clone(),
                    data: game.data.clone(),
                },
                &options.artifact,
                inner.borrow().profile.clone(),
                Rc::new(Bridge {
                    handles: weak_handles(),
                }),
            )?;
            *provider_cell.borrow_mut() = Some(provider);
            Ok(())
        })();
        match built {
            Ok(()) => {
                let pending = std::mem::take(&mut inner.borrow_mut().outbox);
                let event = inner.borrow().event.clone();
                for pending in pending {
                    (event)(pending);
                }
                Ok(Self {
                    inner,
                    provider: provider_cell,
                })
            }
            Err(error) => {
                close_inner(&mut inner.borrow_mut());
                Err(error)
            }
        }
    }

    /// Owned handles for state plus provider access.
    fn handles(&self) -> Handles {
        Handles {
            inner: Rc::clone(&self.inner),
            provider: Rc::clone(&self.provider),
        }
    }

    /// Reserve the scratch span below the VM stack and locate unlocated
    /// tables with the canonical layout.
    fn reserve_scratch(inner: &Rc<RefCell<Inner>>, artifact: &QvmArtifact) -> Result<(), QvmGrappleSourceError> {
        let image = &artifact.image;
        let word = align16(image.data_length + image.literal_length + image.bss_length);
        let movement_bytes = inner.borrow().profile.movement.byte_length;
        let byte_length = 64.max(movement_bytes);
        if word + byte_length > image.allocated_data_length.saturating_sub(VM_STACK_RESERVED) {
            return Err(QvmGrappleSourceError::invalid(
                "Grapple source has no reserved scratch below the VM stack",
            ));
        }
        inner.borrow_mut().scratch = QvmGrappleScratch { word, byte_length };
        if inner.borrow().game.data.entity_stride_bytes() == 0 {
            let (entity_stride, client_stride) = {
                let guard = inner.borrow();
                (guard.profile.entity_stride, guard.profile.client_stride)
            };
            let entities_word = align16(word + byte_length);
            let entities_end = entities_word + NUM_ENTITIES * entity_stride;
            let clients_word = align16(entities_end);
            let clients_end = clients_word + MAX_CLIENTS * client_stride;
            if clients_end > image.allocated_data_length.saturating_sub(VM_STACK_RESERVED) {
                return Err(QvmGrappleSourceError::invalid(
                    "Grapple image is too small for its entity tables",
                ));
            }
            let entities_word_i32 = i32::try_from(entities_word)
                .map_err(|_| QvmGrappleSourceError::invalid("Grapple entity tables exceed the address space"))?;
            let clients_word_i32 = i32::try_from(clients_word)
                .map_err(|_| QvmGrappleSourceError::invalid("Grapple entity tables exceed the address space"))?;
            inner.borrow().game.data.locate(
                entities_word_i32,
                NUM_ENTITIES,
                entity_stride,
                clients_word_i32,
                client_stride,
            )?;
        }
        Ok(())
    }

    /// Borrow the guest game (donor `vm`).
    #[must_use]
    pub fn game(&self) -> QvmGame {
        self.inner.borrow().game.clone()
    }

    /// Borrow the session actors (donor `host.actors`).
    #[must_use]
    pub fn actors(&self) -> Rc<dyn QvmGrappleActors> {
        Rc::clone(&self.inner.borrow().actors)
    }

    /// Borrow the shared bodies (donor `host.bodies`).
    #[must_use]
    pub fn bodies(&self) -> Rc<dyn QvmGrappleBodies> {
        Rc::clone(&self.inner.borrow().bodies)
    }

    /// Begin a source frame (donor `beginFrame`).
    pub fn begin_frame(&self, milliseconds: i32, frame: i32) -> Result<(), QvmGrappleSourceError> {
        let handles = self.handles();
        handles.current()?;
        handles.inner.borrow_mut().milliseconds = milliseconds;
        handles.with_provider(|provider| {
            provider.begin_frame(milliseconds, frame)?;
            provider.step()?;
            Ok(())
        })?;
        let (game, profile, event_lifetime_ms) = {
            let guard = handles.inner.borrow();
            (guard.game.clone(), guard.profile.clone(), guard.event_lifetime_ms)
        };
        for slot in MAX_CLIENTS..game.data.num_entities() {
            let record = game.data.entity_bytes(slot)?;
            if record.get_i32(profile.fields.inuse)? != 0
                && record.get_i32(profile.fields.free_after_event)? != 0
                && f64::from(milliseconds - record.get_i32(profile.fields.event_time)?) > event_lifetime_ms
            {
                let pointer = i32::try_from(record.offset)
                    .map_err(|_| QvmGrappleSourceError::invalid("Grapple entity pointer exceeds its range"))?;
                game.module.call(&[pointer], profile.callbacks.free)?;
            }
        }
        let tethers: Vec<(ActorId, QvmGrappleProjection)> = handles
            .inner
            .borrow()
            .tethers
            .iter()
            .map(|(owner, tether)| (owner.clone(), tether.projection.clone()))
            .collect();
        for (owner, projection) in &tethers {
            if projection.pulling {
                let (bodies, presentation) = {
                    let guard = handles.inner.borrow();
                    (Rc::clone(&guard.bodies), guard.presentation.clone())
                };
                let body = bodies.read(owner);
                let point = projection.point;
                let hanging = body.as_ref().is_some_and(|body| {
                    let dx = point.x - body.origin.x;
                    let dy = point.y - body.origin.y;
                    let dz = point.z - body.origin.z - CHEST_HEIGHT;
                    dx.hypot(dy).hypot(dz) <= HANG_DISTANCE
                });
                let path = if hanging {
                    presentation.hang_sound
                } else {
                    presentation.pull_sound
                };
                handles.sound(owner, path.as_deref(), true);
            }
        }
        Ok(())
    }

    /// Fire an owner's hook (donor `fire`).
    pub fn fire(&self, actor: &ActorId) -> Result<(), QvmGrappleSourceError> {
        let handles = self.handles();
        handles.current()?;
        let previous = self.hook(actor);
        handles.with_provider(|provider| {
            provider.fire(actor)?;
            Ok(())
        })?;
        if previous.is_none() && self.hook(actor).is_some() {
            let path = handles.inner.borrow().presentation.fire_sound.clone();
            handles.sound(actor, path.as_deref(), false);
        }
        Ok(())
    }

    /// Admit a borrowed player (donor `admit`).
    pub fn admit(&self, actor: &ActorId) -> Result<(), QvmGrappleSourceError> {
        let handles = self.handles();
        handles.current()?;
        handles.mirror(actor)?;
        let client = handles
            .inner
            .borrow()
            .bindings
            .get(actor)
            .is_some_and(|binding| binding.client);
        if !client {
            return Err(QvmGrappleSourceError::invalid(
                "Grapple equipment requires an admitted source client",
            ));
        }
        Ok(())
    }

    /// Release an owner's hook (donor `release`).
    pub fn release(&self, actor: &ActorId) -> Result<(), QvmGrappleSourceError> {
        let handles = self.handles();
        let previous = self.hook(actor);
        handles.with_provider(|provider| {
            provider.release(actor, false)?;
            Ok(())
        })?;
        if previous.is_some() && self.hook(actor).is_none() {
            let path = handles.inner.borrow().presentation.release_sound.clone();
            handles.sound(actor, path.as_deref(), false);
        }
        Ok(())
    }

    /// Owned hook actor for an owner, if any (donor `hook`).
    #[must_use]
    pub fn hook(&self, actor: &ActorId) -> Option<ActorId> {
        self.inner
            .borrow()
            .tethers
            .get(actor)
            .map(|tether| tether.actor.id().clone())
    }

    /// Hook weapon view for a bound player (donor `weaponView`).
    pub fn weapon_view(&self, actor: &ActorId) -> Result<QvmGrappleWeaponView, QvmGrappleSourceError> {
        let handles = self.handles();
        let (pointer, game) = {
            let guard = handles.inner.borrow();
            let binding = guard
                .bindings
                .get(actor)
                .ok_or_else(|| QvmGrappleSourceError::invalid("Hook view has no source player"))?;
            if !binding.client {
                return Err(QvmGrappleSourceError::invalid("Hook view has no source player"));
            }
            (binding.pointer, guard.game.clone())
        };
        let slot = handles.slot(pointer)?;
        let state = game.data.copy_player_state(slot)?;
        let guard = handles.inner.borrow();
        let presentation = guard.presentation.clone();
        let firing = guard.tethers.contains_key(actor);
        Ok(QvmGrappleWeaponView {
            path: presentation.view_model.clone(),
            frame: 0,
            kick_origin: vec3(0.0, 0.0, 0.0),
            kick_pitch: 0.0,
            model_attachments: presentation.view_attachments.clone(),
            model_anchor: presentation.view_anchor.clone(),
            q3_weapon: Q3WeaponPresentation {
                time_milliseconds: guard.milliseconds,
                torso_animation: state.torso_animation,
                last_fire_milliseconds: None,
                firing,
                horizontal_speed: f64::from(state.velocity.x.hypot(state.velocity.y)),
                bob_cycle: f64::from(state.bob_cycle),
                weapon: presentation.weapon_index as i32,
            },
        })
    }

    /// Tether presentations (donor `presentations`).
    pub fn presentations(&self, content: &ContentId) -> Vec<SimulationPresentation> {
        let guard = self.inner.borrow();
        let mut result = Vec::new();
        let presentation = guard.presentation.clone();
        let targets = (guard.targets)();
        for (owner, tether) in &guard.tethers {
            let body = guard.bodies.read(tether.actor.id());
            let player = targets.iter().find(|target| target.actor == *owner);
            let (Some(body), Some(player)) = (body, player) else {
                continue;
            };
            let common = |path: String, origin: Vec3| SimulationPresentation {
                held_weapon: None,
                native_held_weapon: false,
                weapon_item: None,
                replaces_body: false,
                render_source_client: false,
                flare: None,
                actor: tether.actor.id().clone(),
                content: content.clone(),
                family: GameFamily::Q3,
                path,
                frame: 0,
                old_frame: 0,
                back_lerp: None,
                skin: 0,
                skin_path: None,
                indexed_skin: None,
                player_colors: None,
                effects: 0,
                render_flags: 0,
                origin,
                previous_origin: None,
                model_beam: None,
                shader_beam: None,
                model_attachments: None,
                model_anchor: None,
                q3_grapple_cable: None,
                angles: body.angles,
                scale: 1.0,
                alpha: None,
                visible: true,
                view_weapon: false,
                q3_weapon: None,
            };
            result.push(common(presentation.projectile_model.clone(), body.origin));
            let view_height = match &player.kind {
                QvmGrappleTargetKind::Player { view_height, .. } => *view_height,
                QvmGrappleTargetKind::Actor { .. } => 0.0,
            };
            #[allow(clippy::cast_possible_truncation)]
            let start = Vec3 {
                x: player.body.origin.x,
                y: player.body.origin.y,
                z: player.body.origin.z + view_height as f32,
            };
            match &presentation.cable {
                QvmGrappleCable::Shader { path, width } => {
                    let mut cable = common(String::new(), start);
                    cable.shader_beam = Some(ShaderBeam {
                        path: path.clone(),
                        end: body.origin,
                        width: *width,
                    });
                    result.push(cable);
                }
                QvmGrappleCable::Model {
                    flight,
                    pull,
                    hold,
                    segment_length,
                } => {
                    let mut cable = common(flight.clone(), body.origin);
                    cable.q3_grapple_cable = Some(Q3GrappleCable {
                        owner: owner.clone(),
                        owner_origin: player.body.origin,
                        owner_angles: player.body.angles,
                        view_height,
                        offhand: guard.binding_side != QvmGrappleBinding::Slot,
                        attached: tether.projection.pulling,
                        flight: flight.clone(),
                        pull: pull.clone(),
                        hold: hold.clone(),
                        segment_length: *segment_length,
                    });
                    result.push(cable);
                }
            }
        }
        result
    }

    /// Whether an owner's tether is pulling (donor `pulling`).
    #[must_use]
    pub fn pulling(&self, actor: &ActorId) -> bool {
        self.inner
            .borrow()
            .tethers
            .get(actor)
            .is_some_and(|tether| tether.projection.pulling)
    }

    /// Pull an owner's tether along the owner view (donor `pull`).
    pub fn pull(&self, actor: &ActorId) -> Result<(), QvmGrappleSourceError> {
        let handles = self.handles();
        let bodies = handles.inner.borrow().bodies.clone();
        let body = bodies.read(actor);
        let Some(body) = body else { return Ok(()) };
        handles.synchronize()?;
        let forward = qvm_angle_vectors(body.angles).forward;
        handles.with_provider(|provider| {
            provider.pull(actor, &forward)?;
            Ok(())
        })?;
        Ok(())
    }

    /// Capture the source checkpoint (donor `capture`).
    pub fn capture(&self) -> Result<QvmGrappleSourceCheckpoint, QvmGrappleSourceError> {
        let handles = self.handles();
        let captured = handles.with_provider(|provider| Ok(provider.capture()?))?;
        let guard = handles.inner.borrow();
        let host = capture_host(&guard)?;
        let mut bindings: Vec<QvmGrappleSourceBinding> = guard
            .bindings
            .values()
            .map(|binding| QvmGrappleSourceBinding {
                actor: SavedActorId::from(&binding.actor),
                pointer: binding.pointer,
                client: binding.client,
                origin: binding.origin,
            })
            .collect();
        bindings.sort_by_key(|binding| (binding.actor.slot, binding.actor.generation));
        let mut tethers: Vec<QvmGrappleSourceTether> = guard
            .tethers
            .iter()
            .map(|(owner, tether)| QvmGrappleSourceTether {
                owner: SavedActorId::from(owner),
                actor: SavedActorId::from(tether.actor.id()),
            })
            .collect();
        tethers.sort_by_key(|tether| {
            (
                tether.owner.slot,
                tether.owner.generation,
                tether.actor.slot,
                tether.actor.generation,
            )
        });
        Ok(QvmGrappleSourceCheckpoint {
            version: CHECKPOINT_VERSION,
            grapple: QvmGrappleSourceGrapple {
                version: captured.version,
                profile: captured.profile,
                module: QvmCheckpoint {
                    host_state: QvmHostState {
                        module: captured.module.module.clone(),
                        format: HOST_FORMAT.to_string(),
                        bytes: ProfileValue::Bytes(encode_checkpoint_value(&host)),
                    },
                    ..captured.module
                },
                owners: captured
                    .owners
                    .iter()
                    .map(|owner| SavedActorId {
                        slot: owner.slot,
                        generation: owner.generation,
                    })
                    .collect(),
            },
            bindings,
            tethers,
        })
    }

    /// Restore the source checkpoint (donor `restore`).
    pub fn restore(&self, checkpoint: &QvmGrappleSourceCheckpoint) -> Result<(), QvmGrappleSourceError> {
        if checkpoint.version != CHECKPOINT_VERSION {
            return Err(QvmGrappleSourceError::invalid("Invalid grapple source checkpoint"));
        }
        let handles = self.handles();
        {
            let (actors, provider_id) = {
                let guard = handles.inner.borrow();
                (Rc::clone(&guard.actors), guard.provider_id.clone())
            };
            let mut guard = handles.inner.borrow_mut();
            guard.bindings.clear();
            for entry in &checkpoint.bindings {
                let actor = restore_actor_id(&actors, &entry.actor)?;
                if guard.bindings.contains_key(&actor)
                    || guard.bindings.values().any(|binding| binding.pointer == entry.pointer)
                {
                    return Err(QvmGrappleSourceError::invalid("Duplicate saved grapple binding"));
                }
                guard.bindings.insert(
                    actor.clone(),
                    Binding {
                        actor,
                        pointer: entry.pointer,
                        client: entry.client,
                        origin: entry.origin,
                    },
                );
            }
            let mut restored = HashMap::new();
            for entry in &checkpoint.tethers {
                let owner = restore_actor_id(&actors, &entry.owner)?;
                let actor = actors.resolve_saved(&entry.actor);
                let Some(actor) = actor else {
                    return Err(QvmGrappleSourceError::invalid("Invalid saved shared tether"));
                };
                if actor.owner() != &provider_id || restored.contains_key(&owner) {
                    return Err(QvmGrappleSourceError::invalid("Invalid saved shared tether"));
                }
                restored.insert(owner, actor);
            }
            restore_host(&mut guard, &checkpoint.grapple.module)?;
            guard.tethers.clear();
            guard.restoring_tethers = Some(restored);
        }
        let owners = checkpoint
            .grapple
            .owners
            .iter()
            .map(|owner| QvmSavedActor {
                slot: owner.slot,
                generation: owner.generation,
            })
            .collect();
        let outcome = handles.with_provider(|provider| {
            provider.restore(&ProviderCheckpoint {
                version: checkpoint.grapple.version,
                profile: checkpoint.grapple.profile.clone(),
                module: checkpoint.grapple.module.clone(),
                owners,
            })?;
            Ok(())
        });
        handles.inner.borrow_mut().restoring_tethers = None;
        outcome
    }

    /// Close the source (donor `close`).
    pub fn close(&self) {
        let handles = self.handles();
        if handles.inner.borrow().closed {
            return;
        }
        let _ = handles.with_provider(|provider| {
            let _ = provider.close();
            Ok(())
        });
        close_inner(&mut handles.inner.borrow_mut());
    }
}

/// Resolve a saved actor, rejecting missing shared actors (donor
/// `restoreActor`).
fn restore_actor_id(actors: &Rc<dyn QvmGrappleActors>, saved: &SavedActorId) -> Result<ActorId, QvmGrappleSourceError> {
    actors
        .resolve_saved(saved)
        .map(|actor| actor.id().clone())
        .ok_or_else(|| QvmGrappleSourceError::invalid("Saved grapple refers to a missing shared actor"))
}

impl Handles {
    /// Run a body against the provider (donor `core`).
    fn with_provider<R>(
        &self,
        run: impl FnOnce(&QvmGrappleProvider) -> Result<R, QvmGrappleSourceError>,
    ) -> Result<R, QvmGrappleSourceError> {
        let guard = self.provider.borrow();
        let provider = guard
            .as_ref()
            .ok_or_else(|| QvmGrappleSourceError::invalid("Grapple source has not initialized"))?;
        run(provider)
    }

    /// Assert the calling context and open state (donor `current`).
    fn current(&self) -> Result<(), QvmGrappleSourceError> {
        let guard = self.inner.borrow();
        (guard.assert_current)();
        if guard.closed {
            return Err(QvmGrappleSourceError::invalid("Grapple source is closed"));
        }
        Ok(())
    }

    /// Entity slot for a source pointer (donor `slot`).
    fn slot(&self, pointer: i32) -> Result<usize, QvmGrappleSourceError> {
        let game = self.inner.borrow().game.clone();
        Ok(game.data.number_from_pointer(pointer)?)
    }

    /// Canonical actor for a source pointer (donor `actorForPointer`).
    fn actor_for_pointer(&self, pointer: i32) -> Option<ActorId> {
        if pointer == 0 {
            return None;
        }
        let game = self.inner.borrow().game.clone();
        let layout = game.data.checkpoint();
        let relative = pointer.checked_sub(layout.entities_word as i32).unwrap_or(-1);
        if layout.entity_stride != 0 {
            let slot = relative / layout.entity_stride as i32;
            if slot == ENTITYNUM_WORLD || slot == ENTITYNUM_NONE {
                return None;
            }
        }
        let slot = game.data.number_from_pointer(pointer).ok()?;
        self.actor_for_slot(slot)
    }

    /// Canonical actor for an entity slot (donor `actorForSlot`).
    fn actor_for_slot(&self, slot: usize) -> Option<ActorId> {
        let guard = self.inner.borrow();
        for binding in guard.bindings.values() {
            if guard.game.data.number_from_pointer(binding.pointer).ok() == Some(slot) {
                return Some(binding.actor.clone());
            }
        }
        for tether in guard.tethers.values() {
            let Ok(hook) = i32::try_from(tether.projection.hook) else {
                continue;
            };
            if guard.game.data.number_from_pointer(hook).ok() == Some(slot) {
                return Some(tether.actor.id().clone());
            }
        }
        None
    }

    /// Restore a saved actor (donor `restoreActor`).
    fn restore_actor(&self, saved: &SavedActorId) -> Result<ActorId, QvmGrappleSourceError> {
        let actors = self.inner.borrow().actors.clone();
        restore_actor_id(&actors, saved)
    }

    /// Mirror a shared actor into the source VM (donor `mirror`).
    fn mirror(&self, actor: &ActorId) -> Result<i32, QvmGrappleSourceError> {
        let targets_fn = self.inner.borrow().targets.clone();
        let target = targets_fn()
            .into_iter()
            .find(|target| target.actor == *actor)
            .ok_or_else(|| QvmGrappleSourceError::invalid("Grapple collision refers to an unavailable shared actor"))?;
        let bound = self.inner.borrow().bindings.get(actor).map(|binding| binding.pointer);
        if let Some(pointer) = bound {
            return self.refresh_mirror(&target, pointer);
        }
        let pointer = match &target.kind {
            QvmGrappleTargetKind::Player { userinfo, .. } => {
                let mut slot = 0;
                while slot < MAX_CLIENTS as i32 && self.inner.borrow().userinfo.contains_key(&slot) {
                    slot += 1;
                }
                if slot == MAX_CLIENTS as i32 {
                    return Err(QvmGrappleSourceError::invalid(
                        "Grapple source exhausted its client capacity",
                    ));
                }
                let game = self.inner.borrow().game.clone();
                let pointer = i32::try_from(game.data.entity_bytes(slot as usize)?.offset)
                    .map_err(|_| QvmGrappleSourceError::invalid("Grapple entity pointer exceeds its range"))?;
                self.inner.borrow_mut().bindings.insert(
                    target.actor.clone(),
                    Binding {
                        actor: target.actor.clone(),
                        pointer,
                        client: true,
                        origin: target.body.origin,
                    },
                );
                self.inner.borrow_mut().userinfo.insert(slot, userinfo.clone());
                if let Some(denial) = game.client_connect(slot, true, false)? {
                    return Err(QvmGrappleSourceError::invalid(format!(
                        "Grapple source rejected client: {denial}"
                    )));
                }
                game.client_begin(slot)?;
                pointer
            }
            QvmGrappleTargetKind::Actor { .. } => {
                let (game, allocate) = {
                    let guard = self.inner.borrow();
                    (guard.game.clone(), guard.profile.callbacks.allocate)
                };
                let pointer = game.module.call(&[], allocate)?;
                self.slot(pointer)?;
                if self
                    .inner
                    .borrow()
                    .bindings
                    .values()
                    .any(|entry| entry.pointer == pointer)
                {
                    return Err(QvmGrappleSourceError::invalid(
                        "Grapple source reused an occupied actor",
                    ));
                }
                self.inner.borrow_mut().bindings.insert(
                    target.actor.clone(),
                    Binding {
                        actor: target.actor.clone(),
                        pointer,
                        client: false,
                        origin: target.body.origin,
                    },
                );
                pointer
            }
        };
        self.refresh_mirror(&target, pointer)
    }

    /// Refresh a mirrored binding from its target body (donor `mirror`
    /// record writes).
    #[allow(clippy::too_many_lines)]
    fn refresh_mirror(&self, target: &QvmGrappleTarget, pointer: i32) -> Result<i32, QvmGrappleSourceError> {
        let (game, profile, milliseconds) = {
            let guard = self.inner.borrow();
            (guard.game.clone(), guard.profile.clone(), guard.milliseconds)
        };
        let slot = game.data.number_from_pointer(pointer)?;
        let mut entity = game.data.entity_from_pointer(pointer)?;
        let body = &target.body;
        if let QvmGrappleTargetKind::Player {
            userinfo,
            team,
            view_height,
        } = &target.kind
        {
            let current = self
                .inner
                .borrow()
                .userinfo
                .get(&(slot as i32))
                .cloned()
                .unwrap_or_default();
            if current != *userinfo {
                self.inner.borrow_mut().userinfo.insert(slot as i32, userinfo.clone());
                game.client_userinfo_changed(slot as i32)?;
            }
            let ps = game.data.public_player_bytes(slot)?;
            for (offset, vector) in [
                (PS_ORIGIN, body.origin),
                (PS_VELOCITY, body.velocity),
                (PS_VIEWANGLES, body.angles),
            ] {
                ps.set_f32(offset, vector.x)?;
                ps.set_f32(offset + 4, vector.y)?;
                ps.set_f32(offset + 8, vector.z)?;
            }
            #[allow(clippy::cast_possible_truncation)]
            ps.set_i32(PS_HEALTH, target.health as i32)?;
            #[allow(clippy::cast_possible_truncation)]
            ps.set_i32(PS_VIEWHEIGHT, *view_height as i32)?;
            ps.set_i32(PS_TEAM, team.number())?;
            #[allow(clippy::cast_possible_truncation)]
            let pm_type = if *team == QvmGrappleTeam::Spectator {
                2
            } else if target.health <= 0.0 {
                3
            } else {
                0
            };
            ps.set_i32(PS_PM_TYPE, pm_type)?;
            entity.r.contents = if *team == QvmGrappleTeam::Spectator {
                0
            } else if target.health > 0.0 {
                CONTENTS_PLAYERCLIP_BODY
            } else {
                CONTENTS_CORPSE
            };
            entity.s.e_type = 1;
        } else if let QvmGrappleTargetKind::Actor { mover } = &target.kind {
            entity.s.e_type = if *mover { 4 } else { 0 };
            if *mover && self.provider.borrow().is_some() {
                let origin = self
                    .inner
                    .borrow()
                    .bindings
                    .get(&target.actor)
                    .map(|binding| binding.origin);
                if let Some(origin) = origin {
                    let delta = Vec3 {
                        x: body.origin.x - origin.x,
                        y: body.origin.y - origin.y,
                        z: body.origin.z - origin.z,
                    };
                    if delta.x != 0.0 || delta.y != 0.0 || delta.z != 0.0 {
                        let actor = target.actor.clone();
                        self.with_provider(|provider| {
                            provider.mover_moved(&actor, &delta)?;
                            Ok(())
                        })?;
                    }
                }
            }
        }
        entity.r.current_origin = body.origin;
        entity.r.current_angles = body.angles;
        entity.r.mins = body.bounds.min;
        entity.r.maxs = body.bounds.max;
        entity.s.origin = body.origin;
        entity.s.angles = body.angles;
        entity.s.pos.base = body.origin;
        entity.s.pos.delta = body.velocity;
        entity.s.pos.time = milliseconds;
        write_record(&game, slot, &entity)?;
        #[allow(clippy::cast_possible_truncation)]
        game.data
            .entity_bytes(slot)?
            .set_i32(profile.fields.health, target.health as i32)?;
        game.data
            .entity_bytes(slot)?
            .set_i32(profile.fields.takedamage, i32::from(target.health > 0.0))?;
        if let Some(binding) = self.inner.borrow_mut().bindings.get_mut(&target.actor) {
            binding.origin = body.origin;
        }
        Ok(pointer)
    }

    /// Drop mirrors for departed targets and mirror every target (donor
    /// `synchronize`).
    fn synchronize(&self) -> Result<(), QvmGrappleSourceError> {
        if self.inner.borrow().synchronizing {
            return Ok(());
        }
        self.inner.borrow_mut().synchronizing = true;
        let outcome = self.synchronize_body();
        self.inner.borrow_mut().synchronizing = false;
        outcome
    }

    /// Synchronization body.
    fn synchronize_body(&self) -> Result<(), QvmGrappleSourceError> {
        let targets_fn = self.inner.borrow().targets.clone();
        let targets = targets_fn();
        let stale: Vec<ActorId> = self
            .inner
            .borrow()
            .bindings
            .keys()
            .filter(|actor| !targets.iter().any(|target| target.actor == **actor))
            .cloned()
            .collect();
        for actor in &stale {
            self.remove_actor(actor)?;
        }
        for target in &targets {
            self.mirror(&target.actor)?;
        }
        Ok(())
    }

    /// Publish or retire an owner's tether (donor `publish`).
    fn publish(&self, owner: &ActorId, projection: Option<QvmGrappleProjection>) {
        let mut guard = self.inner.borrow_mut();
        let previous = guard.tethers.get(owner).cloned();
        let Some(projection) = projection else {
            if let Some(previous) = previous {
                guard.tethers.remove(owner);
                if guard.actors.is_live(previous.actor.id()) {
                    guard.actors.release(&previous.actor);
                }
            }
            return;
        };
        let restoring = guard
            .restoring_tethers
            .as_ref()
            .and_then(|restored| restored.get(owner).cloned());
        let provider_id = guard.provider_id.clone();
        let actor = previous
            .as_ref()
            .map(|previous| previous.actor.clone())
            .or(restoring)
            .unwrap_or_else(|| guard.actors.allocate(&provider_id, HOOK_DEFINITION));
        guard.tethers.insert(
            owner.clone(),
            Tether {
                actor: actor.clone(),
                projection: projection.clone(),
            },
        );
        let record = guard
            .game
            .data
            .entity_from_pointer(i32::try_from(projection.hook).unwrap_or(i32::MAX))
            .unwrap_or_else(|error| panic!("Grapple tether hook is invalid: {error}"));
        let body = BodyState {
            origin: if projection.pulling {
                projection.point
            } else {
                projection.origin
            },
            velocity: projection.velocity,
            angles: record.r.current_angles,
            bounds: Bounds {
                min: record.r.mins,
                max: record.r.maxs,
            },
            ground: projection.mover.clone(),
        };
        guard.bodies.write(&actor, body);
        guard.bodies.link(&actor);
        let attach = projection.pulling
            && previous.as_ref().is_none_or(|previous| !previous.projection.pulling)
            && guard.restoring_tethers.is_none();
        let path = attach.then(|| guard.presentation.attach_sound.clone()).flatten();
        drop(guard);
        if let Some(path) = path {
            self.sound(actor.id(), Some(path.as_str()), false);
        }
    }

    /// Emit a positional sound (donor `sound`).
    fn sound(&self, actor: &ActorId, path: Option<&str>, loop_sound: bool) {
        let (bodies, event) = {
            let guard = self.inner.borrow();
            (Rc::clone(&guard.bodies), Rc::clone(&guard.event))
        };
        let (Some(path), Some(body)) = (path, bodies.read(actor)) else {
            return;
        };
        event(Q3SourceEvent::Sound {
            actor: actor.clone(),
            origin: body.origin,
            velocity: body.velocity,
            path: path.to_string(),
            channel: 0,
            volume: 1.0,
            loop_sound,
        });
    }

    /// Remove a mirrored actor (donor `removeActor`).
    fn remove_actor(&self, actor: &ActorId) -> Result<(), QvmGrappleSourceError> {
        if self.inner.borrow().closed || self.provider.borrow().is_none() {
            return Ok(());
        }
        let actor = actor.clone();
        self.with_provider(|provider| {
            provider.actor_released(&actor)?;
            Ok(())
        })?;
        let binding = self.inner.borrow().bindings.get(&actor).cloned();
        let Some(binding) = binding else { return Ok(()) };
        let (game, free) = {
            let guard = self.inner.borrow();
            (guard.game.clone(), guard.profile.callbacks.free)
        };
        if binding.client {
            let slot = game.data.number_from_pointer(binding.pointer)?;
            game.client_disconnect(slot as i32)?;
            self.inner.borrow_mut().userinfo.remove(&(slot as i32));
        } else {
            game.module.call(&[binding.pointer], free)?;
        }
        self.inner.borrow_mut().bindings.remove(&actor);
        Ok(())
    }
}

/// Read-modify-write a shared entity record (the donor mutates live
/// records; the port writes owned copies back explicitly).
fn write_record(game: &QvmGame, slot: usize, entity: &QvmSharedEntity) -> Result<(), QvmGrappleSourceError> {
    let window = game.data.entity_bytes(slot)?;
    let mut bytes = window.copy_bytes(0, window.len)?;
    write_qvm_shared_entity(&mut bytes, entity, game.data.abi_profile())?;
    window.memory.write_bytes(window.offset, &bytes)?;
    Ok(())
}

/// Close shared state (donor `close` body).
fn close_inner(inner: &mut Inner) {
    if inner.closed {
        return;
    }
    if let Some(unsubscribe) = inner.unsubscribe.take() {
        unsubscribe();
    }
    inner.closed = true;
    inner.bindings.clear();
    inner.tethers.clear();
    inner.files.close_all();
    inner.game.retire();
}

/// Dispatch one trap (donor `syscall`).
fn dispatch_syscall(handles: &WeakHandles, call: &GameCall) -> Result<Option<i32>, GuestError> {
    let Some(handles) = handles.upgrade() else {
        return Ok(None);
    };
    handles
        .current()
        .map_err(|error| GuestError::invalid(error.to_string()))?;
    let outcome = handles.dispatch(call)?;
    let pending = std::mem::take(&mut handles.inner.borrow_mut().outbox);
    let event = handles.inner.borrow().event.clone();
    for pending in pending {
        event(pending);
    }
    Ok(outcome)
}

/// Bridged trap: client-universe call plus writable memory plus the real
/// allocation length (the bridge pads to a power of two).
struct BridgedTrap {
    /// Client-universe call.
    call: ClientCall,
    /// Writable guest memory.
    memory: SyscallMemory,
    /// Real allocation length (prefix of the padded memory).
    real_len: usize,
}

/// Copy a `game_data` trap into the `client_state` universe.
fn bridge_trap(call: &GameCall) -> Result<BridgedTrap, GuestError> {
    let kind = match call.kind {
        GameKind::Engine => ClientKind::Engine,
        GameKind::Extension => ClientKind::Extension,
    };
    let role = match call.role {
        GameRole::Qagame => ClientRole::Qagame,
        GameRole::Cgame => ClientRole::Cgame,
        GameRole::Ui => ClientRole::Ui,
    };
    let abi = match call.abi_profile {
        GameAbi::Modern => ClientAbi::Modern,
        GameAbi::Legacy => ClientAbi::Legacy,
    };
    let real_len = call.guest.len();
    let mut bytes = call.guest.read_bytes(0, real_len)?;
    let padded = real_len.next_power_of_two().max(64);
    bytes.resize(padded, 0);
    Ok(BridgedTrap {
        call: ClientCall {
            kind,
            role,
            code: call.code,
            words: call.words.clone(),
            abi_profile: abi,
        },
        memory: SyscallMemory::from_bytes(bytes)?,
        real_len,
    })
}

/// Copy bridged memory back into the guest allocation.
fn unbridge_trap(call: &GameCall, bridged: &BridgedTrap) -> Result<(), GuestError> {
    call.guest
        .write_bytes(0, &bridged.memory.as_slice()[..bridged.real_len])
}

impl Handles {
    /// Dispatch one trap through the common, cvar, file, and server-game
    /// layers plus the disabled-botlib arm (donor `syscall`).
    fn dispatch(&self, call: &GameCall) -> Result<Option<i32>, GuestError> {
        if let Some(value) = self.common_trap(call)? {
            return Ok(Some(value));
        }
        if call.kind == GameKind::Engine {
            let mut bridged = bridge_trap(call)?;
            let mut host = self.clone();
            let result = cvar_syscall(&bridged.call, &mut bridged.memory, &mut host)?.or(file_syscall(
                &bridged.call,
                &mut bridged.memory,
                &mut self.inner.borrow_mut().files,
            )?
            .or(server_game_syscall(&bridged.call, &mut bridged.memory, &mut host)?));
            unbridge_trap(call, &bridged)?;
            if result.is_some() {
                return Ok(result);
            }
        }
        if call.kind == GameKind::Engine
            && call.role == GameRole::Qagame
            && (call.code == BOTLIB_SETUP || call.code == BOTLIB_AAS_INITIALIZED)
        {
            return Ok(Some(0));
        }
        Ok(None)
    }

    /// Mirror the non-cvar common traps (donor `qvmCommonSyscall` for
    /// `qagame`, minus the cvar traps handled by `cvar_syscall`).
    #[allow(clippy::too_many_lines)]
    fn common_trap(&self, call: &GameCall) -> Result<Option<i32>, GuestError> {
        if call.kind != GameKind::Engine || call.role != GameRole::Qagame {
            return Ok(None);
        }
        match call.code {
            c if c == QvmGameImport::G_PRINT => {
                let text = call.guest.read_string(call.int(1)?)?;
                self.inner.borrow_mut().outbox.push(Q3SourceEvent::Print { text });
                Ok(Some(0))
            }
            c if c == QvmGameImport::G_ERROR => {
                let mut text = call.guest.read_string(call.int(1)?)?;
                if text.len() > MAX_ERROR_CHARS {
                    text.truncate(MAX_ERROR_CHARS);
                }
                Err(GuestError::callback(format!("QVM game error: {text}")))
            }
            c if c == QvmGameImport::G_MILLISECONDS => Ok(Some(self.inner.borrow().milliseconds)),
            c if c == G_ARGC => Ok(Some(call.command_arguments.clone().unwrap_or_default().len() as i32)),
            c if c == G_ARGV => {
                let argv = call.command_arguments.clone().unwrap_or_default();
                let index = call.int(1)?;
                let text = argv
                    .get(usize::try_from(index).unwrap_or(usize::MAX))
                    .cloned()
                    .unwrap_or_default();
                call.guest
                    .write_string(call.int(2)?, &text, usize::try_from(call.int(3)?).unwrap_or(0))?;
                Ok(Some(0))
            }
            c if c == G_SEND_CONSOLE_COMMAND => {
                let when = call.int(1)?;
                let pointer = call.int(2)?;
                let mut guard = self.inner.borrow_mut();
                let context = guard.context.clone();
                let services_mounts = Rc::clone(&guard.mounts);
                match when {
                    0 => {
                        let text = if pointer == 0 {
                            None
                        } else {
                            Some(call.guest.read_string(pointer)?)
                        };
                        if let Some(text) = text {
                            guard
                                .commands
                                .insert(&text, Some(&context), Some(Dialect::Q3))
                                .map_err(|error| {
                                    GuestError::invalid(format!("Grapple console rejected text: {error}"))
                                })?;
                        }
                        let Inner {
                            commands,
                            cvars,
                            outbox,
                            ..
                        } = &mut *guard;
                        let mut services = ConsoleServices {
                            mounts: services_mounts,
                            outbox,
                        };
                        commands
                            .execute(cvars, &mut services)
                            .map_err(|error| GuestError::invalid(format!("Grapple console failed: {error}")))?;
                        Ok(Some(0))
                    }
                    1 => {
                        let text = call.guest.read_string(pointer)?;
                        guard
                            .commands
                            .insert(&text, Some(&context), Some(Dialect::Q3))
                            .map_err(|error| GuestError::invalid(format!("Grapple console rejected text: {error}")))?;
                        Ok(Some(0))
                    }
                    2 => {
                        let text = call.guest.read_string(pointer)?;
                        guard
                            .commands
                            .append(&text, Some(&context), Some(Dialect::Q3))
                            .map_err(|error| GuestError::invalid(format!("Grapple console rejected text: {error}")))?;
                        Ok(Some(0))
                    }
                    _ => Err(GuestError::invalid("Cbuf_ExecuteText: bad exec_when")),
                }
            }
            c if c == G_REAL_TIME => {
                let pointer = call.int(1)?;
                let real_time = self.inner.borrow().real_time.clone();
                if pointer == 0 {
                    return Ok(Some(real_time(None)));
                }
                call.guest.span(pointer, CALENDAR_BYTES)?;
                let guest = call.guest.clone();
                let write = |calendar: QvmCalendar| -> Result<(), GuestError> {
                    let fields = [
                        calendar.second,
                        calendar.minute,
                        calendar.hour,
                        calendar.day,
                        calendar.month,
                        calendar.year,
                        calendar.weekday,
                        calendar.year_day,
                        calendar.is_dst,
                    ];
                    let base = guest
                        .pointer(pointer)
                        .ok_or_else(|| GuestError::invalid("QVM real-time calendar is outside its allocation"))?;
                    for (index, value) in fields.iter().enumerate() {
                        guest.write_i32(base + index * 4, *value)?;
                    }
                    Ok(())
                };
                let mut sink = |calendar: QvmCalendar| {
                    write(calendar).unwrap_or_else(|_| panic!("QVM calendar span was preflighted"));
                };
                Ok(Some(real_time(Some(&mut sink))))
            }
            _ => Ok(None),
        }
    }
}

impl CvarHost for Handles {
    fn bind_vm(&mut self, name: &str, default: &str, flags: i32) -> i32 {
        let mut guard = self.inner.borrow_mut();
        let handle = guard.next_vm_handle;
        guard.next_vm_handle += 1;
        guard.vm_cvars.insert(handle, name.to_string());
        let _ = guard.cvars.register(name, default, flags as u32);
        for notice in guard.cvars.take_notifications() {
            guard.outbox.push(Q3SourceEvent::Print { text: notice });
        }
        handle
    }

    fn read_vm(&mut self, handle: i32) -> Option<CvarVmBinding> {
        let guard = self.inner.borrow();
        let name = guard.vm_cvars.get(&handle)?.clone();
        let snapshot = guard.cvars.get(&name)?;
        Some(CvarVmBinding {
            modification_count: snapshot.modification_count as i32,
            value: snapshot.value,
            numeric_value: snapshot.numeric_value,
            integer_value: snapshot.integer_value,
        })
    }

    fn get(&mut self, name: &str) -> Option<CvarValue> {
        let snapshot = self.inner.borrow().cvars.get(name)?;
        Some(CvarValue {
            value: snapshot.value,
            numeric_value: snapshot.numeric_value,
            integer_value: snapshot.integer_value,
        })
    }

    fn set(&mut self, name: &str, value: &str) {
        let mut guard = self.inner.borrow_mut();
        let _ = guard.cvars.set(name, value, false);
        for notice in guard.cvars.take_notifications() {
            guard.outbox.push(Q3SourceEvent::Print { text: notice });
        }
    }

    fn set_value(&mut self, name: &str, value: f32) {
        let mut guard = self.inner.borrow_mut();
        let _ = guard.cvars.set_value(name, f64::from(value));
        for notice in guard.cvars.take_notifications() {
            guard.outbox.push(Q3SourceEvent::Print { text: notice });
        }
    }

    fn reset(&mut self, name: &str) {
        let mut guard = self.inner.borrow_mut();
        let _ = guard.cvars.reset(name, false);
        for notice in guard.cvars.take_notifications() {
            guard.outbox.push(Q3SourceEvent::Print { text: notice });
        }
    }

    fn register(&mut self, name: &str, default: &str, flags: i32) {
        let mut guard = self.inner.borrow_mut();
        let _ = guard.cvars.register(name, default, flags as u32);
        for notice in guard.cvars.take_notifications() {
            guard.outbox.push(Q3SourceEvent::Print { text: notice });
        }
    }

    fn info_string(&mut self, flags: i32) -> String {
        self.inner
            .borrow_mut()
            .cvars
            .info_string(flags as u32, None)
            .unwrap_or_default()
    }
}

impl ServerGameHost for Handles {
    fn abi_profile(&self) -> ClientAbi {
        match self.inner.borrow().game.module.abi_profile() {
            GameAbi::Modern => ClientAbi::Modern,
            GameAbi::Legacy => ClientAbi::Legacy,
        }
    }

    fn max_clients(&self) -> i32 {
        MAX_CLIENTS as i32
    }

    fn number_from_pointer(&mut self, word: i32) -> i32 {
        self.inner
            .borrow()
            .game
            .data
            .number_from_pointer(word)
            .map(|slot| slot as i32)
            .unwrap_or_else(|_| panic!("Grapple entity pointer {word} has no source slot"))
    }

    fn get_userinfo(&mut self, slot: i32) -> String {
        self.inner.borrow().userinfo.get(&slot).cloned().unwrap_or_default()
    }

    fn set_userinfo(&mut self, slot: i32, value: &str) {
        self.inner.borrow_mut().userinfo.insert(slot, value.to_string());
    }

    fn get_user_command(&mut self, _slot: i32) -> ClientWireCommand {
        ClientWireCommand {
            server_time: self.inner.borrow().milliseconds,
            angles: [0, 0, 0],
            buttons: 0,
            weapon: 0,
            forwardmove: 0,
            rightmove: 0,
            upmove: 0,
        }
    }

    fn drop_client(&mut self, slot: i32, reason: &str) {
        panic!("Grapple source rejected borrowed client {slot}: {reason}");
    }

    fn send_server_command(&mut self, slot: i32, text: &str) {
        self.inner.borrow_mut().outbox.push(Q3SourceEvent::ServerCommand {
            client: slot,
            text: text.to_string(),
        });
    }

    fn config_get(&mut self, index: i32) -> String {
        self.inner
            .borrow()
            .configstrings
            .get(&index)
            .cloned()
            .unwrap_or_default()
    }

    fn config_set(&mut self, index: i32, value: &str) {
        let mut guard = self.inner.borrow_mut();
        guard.configstrings.insert(index, value.to_string());
        guard.outbox.push(Q3SourceEvent::Configstring {
            index,
            value: value.to_string(),
        });
    }

    fn server_info(&mut self) -> String {
        self.inner
            .borrow_mut()
            .cvars
            .info_string(qa_core::cvar::flags::SERVER_INFO, None)
            .unwrap_or_default()
    }

    fn entity_token(&mut self) -> (String, bool) {
        let mut guard = self.inner.borrow_mut();
        let Inner { parser, cursor, .. } = &mut *guard;
        let token = parser.parse(cursor, true).unwrap_or_else(|error| panic!("{error}"));
        (token, cursor.offset().is_none())
    }
}

impl ServerSpatialHost for Handles {
    fn trace(&mut self, query: &ServerTraceQuery) -> TraceRecord {
        let zero = vec3(0.0, 0.0, 0.0);
        let shape = if query.mins == zero && query.maxs == zero {
            WorldTraceShape::Point
        } else {
            match query.shape {
                GuestTraceShape::Capsule => WorldTraceShape::Capsule {
                    mins: query.mins,
                    maxs: query.maxs,
                },
                GuestTraceShape::Box => WorldTraceShape::Box {
                    mins: query.mins,
                    maxs: query.maxs,
                },
            }
        };
        let pass_entity = usize::try_from(query.pass_entity_num).unwrap_or(usize::MAX);
        let (scene, cvars) = {
            let guard = self.inner.borrow();
            (
                Rc::clone(&guard.scene),
                guard.cvars.variable_value("cm_noCurves") == 0.0,
            )
        };
        let player_curve_clip = self.inner.borrow().cvars.variable_value("cm_playerCurveClip") != 0.0;
        let result = scene.trace_scene(&Q3TraceQuery {
            start: query.start,
            end: query.end,
            shape,
            pass_actor: self.actor_for_slot(pass_entity),
            mask: query.mask,
            curves: cvars,
            player_curve_clip,
            numeric: Q3_BINARY32_PROFILE,
        });
        let entity_num = match &result.hit {
            Q3TraceHit::Actor { actor } => self
                .mirror(actor)
                .and_then(|pointer| self.slot(pointer))
                .map(|slot| slot as i32)
                .unwrap_or_else(|error| panic!("{error}")),
            _ if result.fraction == 1.0 => ENTITYNUM_NONE,
            _ => ENTITYNUM_WORLD,
        };
        let (plane_normal, plane_distance) = match &result.contact {
            TraceContact::Plane { plane } => (plane.normal, plane.distance),
            TraceContact::None => (zero, 0.0),
        };
        let (plane_type, plane_signbits) = plane_type(plane_normal);
        TraceRecord {
            all_solid: result.all_solid,
            start_solid: result.start_solid,
            fraction: result.fraction,
            end: result.end,
            plane_normal,
            plane_distance,
            plane_type,
            plane_signbits,
            surface_flags: result.surface_flags,
            contents: result.contents,
            entity_num,
        }
    }

    fn point_contents(&mut self, point: Vec3, pass_entity_num: i32) -> i32 {
        let pass_entity = usize::try_from(pass_entity_num).unwrap_or(usize::MAX);
        let scene = self.inner.borrow().scene.clone();
        scene.point_contents_scene(
            &Q3TraceQuery {
                start: point,
                end: point,
                shape: WorldTraceShape::Point,
                pass_actor: self.actor_for_slot(pass_entity),
                mask: -1,
                curves: true,
                player_curve_clip: true,
                numeric: Q3_BINARY32_PROFILE,
            },
            point,
        )
    }

    fn area_entities(&mut self, bounds: ServerBounds, maximum: i32) -> Vec<i32> {
        let scene = self.inner.borrow().scene.clone();
        let bounds = Bounds {
            min: bounds.min,
            max: bounds.max,
        };
        let mut actors: Vec<ActorId> = scene
            .query_actors(&bounds)
            .into_iter()
            .filter(|actor| {
                !self
                    .inner
                    .borrow()
                    .tethers
                    .values()
                    .any(|tether| tether.actor.id() == actor)
            })
            .collect();
        if maximum >= 0 {
            actors.truncate(maximum as usize);
        }
        actors
            .iter()
            .map(|actor| {
                self.mirror(actor)
                    .and_then(|pointer| self.slot(pointer))
                    .map(|slot| slot as i32)
                    .unwrap_or_else(|error| panic!("{error}"))
            })
            .collect()
    }

    fn entity_contact(&mut self, _bounds: ServerBounds, _slot: i32, _capsule: bool) -> bool {
        panic!("Grapple source requested undeclared brush contact");
    }

    fn set_brush_model(&mut self, _slot: i32, _name: &str) {
        panic!("Grapple source attempted to own a map brush");
    }

    fn adjust_area_portal_state(&mut self, _slot: i32, _open: bool) {
        panic!("Grapple source attempted to change map portals");
    }

    fn in_pvs(&mut self, first: Vec3, second: Vec3, ignore_portals: bool) -> bool {
        let scene = self.inner.borrow().scene.clone();
        let a = scene.point_leaf(first);
        let b = scene.point_leaf(second);
        scene.cluster_visible(scene.leaf_cluster(a), scene.leaf_cluster(b))
            && (ignore_portals || scene.areas_connected(scene.leaf_area(a), scene.leaf_area(b)))
    }

    fn areas_connected(&mut self, first: i32, second: i32) -> bool {
        self.inner.borrow().scene.clone().areas_connected(first, second)
    }

    fn link(&mut self, slot: i32) {
        let game = self.inner.borrow().game.clone();
        let number = usize::try_from(slot).unwrap_or_else(|_| panic!("Grapple link slot {slot} is invalid"));
        let mut entity = game
            .data
            .entity(number)
            .unwrap_or_else(|error| panic!("Grapple link slot {slot} is invalid: {error}"));
        entity.r.linked = true;
        entity.r.absmin = Vec3 {
            x: entity.r.current_origin.x + entity.r.mins.x - 1.0,
            y: entity.r.current_origin.y + entity.r.mins.y - 1.0,
            z: entity.r.current_origin.z + entity.r.mins.z - 1.0,
        };
        entity.r.absmax = Vec3 {
            x: entity.r.current_origin.x + entity.r.maxs.x + 1.0,
            y: entity.r.current_origin.y + entity.r.maxs.y + 1.0,
            z: entity.r.current_origin.z + entity.r.maxs.z + 1.0,
        };
        write_record(&game, number, &entity).unwrap_or_else(|error| panic!("{error}"));
    }

    fn unlink(&mut self, slot: i32) {
        let game = self.inner.borrow().game.clone();
        let number = usize::try_from(slot).unwrap_or_else(|_| panic!("Grapple unlink slot {slot} is invalid"));
        let mut entity = game
            .data
            .entity(number)
            .unwrap_or_else(|error| panic!("Grapple unlink slot {slot} is invalid: {error}"));
        entity.r.linked = false;
        write_record(&game, number, &entity).unwrap_or_else(|error| panic!("{error}"));
    }
}

/// Bound `sameTeam` callback (donor constructor hook).
fn hook_same_team(handles: &WeakHandles, call: &mut QvmFunctionCall) -> i32 {
    let Some(handles) = handles.upgrade() else {
        return call.proceed();
    };
    if call.words.len() < 2 {
        return call.proceed();
    }
    let (Some(first), Some(second)) = (
        handles.actor_for_pointer(call.words[0]),
        handles.actor_for_pointer(call.words[1]),
    ) else {
        return call.proceed();
    };
    let targets_fn = handles.inner.borrow().targets.clone();
    let targets = targets_fn();
    let a = targets.iter().find(|target| target.actor == first);
    let b = targets.iter().find(|target| target.actor == second);
    let (Some(a), Some(b)) = (a, b) else { return 0 };
    let team = |target: &QvmGrappleTarget| match &target.kind {
        QvmGrappleTargetKind::Player { team, .. } => Some(*team),
        QvmGrappleTargetKind::Actor { .. } => None,
    };
    match (team(a), team(b)) {
        (Some(a), Some(b)) if (a == QvmGrappleTeam::Red || a == QvmGrappleTeam::Blue) && a == b => 1,
        _ => 0,
    }
}

/// Bound `damage` callback (donor constructor hook).
fn hook_damage(handles: &WeakHandles, call: &mut QvmFunctionCall) -> i32 {
    let Some(handles) = handles.upgrade() else {
        return call.proceed();
    };
    if call.words.len() < 8 {
        return call.proceed();
    }
    let pointer = call.words[0];
    let Ok(slot) = handles.slot(pointer) else {
        return call.proceed();
    };
    let actor = handles.actor_for_slot(slot);
    let Some(actor) = actor else {
        return call.proceed();
    };
    if !handles.inner.borrow().bindings.contains_key(&actor) {
        return call.proceed();
    }
    let reference = |index: usize| handles.actor_for_pointer(call.words[index]);
    let vector = |index: usize| {
        let word = call.words[index];
        if word == 0 {
            return Some(vec3(0.0, 0.0, 0.0));
        }
        usize::try_from(word)
            .ok()
            .and_then(|offset| call.memory.read_vec3(offset).ok())
    };
    let (Some(direction), Some(point)) = (vector(3), vector(4)) else {
        return call.proceed();
    };
    let damage = QvmGrappleDamage {
        target: actor.clone(),
        inflictor: reference(1),
        attacker: reference(2),
        direction,
        point,
        amount: call.words[5],
        flags: call.words[6],
        method: call.words[7],
    };
    let (damage_fn, targets_fn, game, health_field) = {
        let guard = handles.inner.borrow();
        (
            guard.damage.clone(),
            guard.targets.clone(),
            guard.game.clone(),
            guard.profile.fields.health,
        )
    };
    damage_fn(damage);
    let health = targets_fn()
        .into_iter()
        .find(|target| target.actor == actor)
        .map(|target| target.health);
    #[allow(clippy::cast_possible_truncation)]
    let health = health.unwrap_or(0.0) as i32;
    let Ok(record) = game.data.entity_bytes(slot) else {
        return call.proceed();
    };
    if record.set_i32(health_field, health).is_err() {
        return call.proceed();
    }
    0
}

/// Capture the VM host save image (donor `QvmGame` `hostState.checkpoint`).
fn capture_host(inner: &Inner) -> Result<SaveJson, QvmGrappleSourceError> {
    let data = inner.game.data.checkpoint();
    let mut configstrings: Vec<(i32, String)> = inner
        .configstrings
        .iter()
        .map(|(index, value)| (*index, value.clone()))
        .collect();
    configstrings.sort();
    let mut userinfo: Vec<(i32, String)> = inner
        .userinfo
        .iter()
        .map(|(slot, value)| (*slot, value.clone()))
        .collect();
    userinfo.sort();
    let parser = inner.parser.capture_save_state();
    let field = |name: &str| parser.record_get(name);
    let text = |name: &str| match field(name) {
        Some(ProfileValue::Str(value)) => value.clone(),
        _ => String::new(),
    };
    let line = match field("line") {
        Some(ProfileValue::Int(line)) => *line,
        _ => 0,
    };
    Ok(obj(vec![
        ("version", int(CHECKPOINT_VERSION as i64)),
        ("milliseconds", int(i64::from(inner.milliseconds))),
        (
            "data",
            obj(vec![
                ("entitiesWord", int(data.entities_word as i64)),
                ("numEntities", int(data.num_entities as i64)),
                ("entityStride", int(data.entity_stride as i64)),
                ("clientsWord", int(data.clients_word as i64)),
                ("clientStride", int(data.client_stride as i64)),
            ]),
        ),
        (
            "cvars",
            obj(vec![
                ("dialect", save_str("q3")),
                (
                    "variables",
                    arr(inner
                        .cvars
                        .snapshots(0)
                        .iter()
                        .map(|snapshot| {
                            obj(vec![
                                ("name", save_str(&snapshot.name)),
                                ("value", save_str(&snapshot.value)),
                                ("resetValue", save_str(&snapshot.reset_value)),
                                (
                                    "latched",
                                    snapshot
                                        .latched_value
                                        .as_ref()
                                        .map_or(SaveJson::Null, |latched| save_str(latched)),
                                ),
                                ("flags", int(i64::from(snapshot.flags))),
                                ("modified", boolean(snapshot.modified)),
                                ("modificationCount", int(i64::from(snapshot.modification_count))),
                            ])
                        })
                        .collect()),
                ),
            ]),
        ),
        (
            "configstrings",
            arr(configstrings
                .into_iter()
                .map(|(index, value)| obj(vec![("index", int(i64::from(index))), ("value", save_str(&value))]))
                .collect()),
        ),
        (
            "userinfo",
            arr(userinfo
                .into_iter()
                .map(|(slot, value)| obj(vec![("slot", int(i64::from(slot))), ("value", save_str(&value))]))
                .collect()),
        ),
        ("files", encode_files(&inner.files.capture_checkpoint()?)),
        ("entityText", save_str(&String::from_utf8_lossy(&inner.cursor.source))),
        (
            "cursor",
            inner
                .cursor
                .offset()
                .map_or(SaveJson::Null, |offset| int(offset as i64)),
        ),
        (
            "parser",
            obj(vec![
                ("token", save_str(&text("token"))),
                ("line", int(line)),
                ("name", save_str(&text("name"))),
            ]),
        ),
    ]))
}

/// Restore the VM host save image (donor `QvmGame` `hostState.restore`).
fn restore_host(inner: &mut Inner, module: &QvmCheckpoint) -> Result<(), QvmGrappleSourceError> {
    if module.host_state.format != HOST_FORMAT {
        return Err(QvmGrappleSourceError::invalid("Invalid QVM grapple host checkpoint"));
    }
    let ProfileValue::Bytes(bytes) = &module.host_state.bytes else {
        return Err(QvmGrappleSourceError::invalid("Invalid QVM grapple host checkpoint"));
    };
    let image = decode_checkpoint_value(bytes)?;
    let reader = SaveReader::new(&image);
    reader.field("version").literal_i64(CHECKPOINT_VERSION as i64)?;
    if reader.field("entityText").string()? != String::from_utf8_lossy(&inner.cursor.source) {
        return Err(QvmGrappleSourceError::invalid(
            "Grapple entity text differs from its source",
        ));
    }
    inner.milliseconds = i32::try_from(reader.field("milliseconds").integer(0)?)
        .map_err(|_| reader.field("milliseconds").fail("expected an integer in range"))?;
    let data = reader.field("data");
    let word = |name: &str| -> Result<usize, WorldError> {
        usize::try_from(data.field(name).integer(0)?).map_err(|_| data.field(name).fail("expected an integer in range"))
    };
    inner.game.data.restore(&QvmGameDataState {
        entities_word: word("entitiesWord")?,
        num_entities: word("numEntities")?,
        entity_stride: word("entityStride")?,
        clients_word: word("clientsWord")?,
        client_stride: word("clientStride")?,
    })?;
    let cvars = reader.field("cvars");
    cvars.field("dialect").literal_str("q3")?;
    let mut registry = CvarRegistry::new(Dialect::Q3);
    for entry in cvars.field("variables").list(|cell| {
        Ok::<_, WorldError>((
            cell.field("name").string()?,
            cell.field("value").string()?,
            cell.field("resetValue").string()?,
            cell.field("latched").nullable(|latched| latched.string())?,
            cell.field("flags").integer(0)?,
        ))
    })? {
        let (name, value, reset_value, latched, flags) = entry;
        let flags = u32::try_from(flags).map_err(|_| cvars.field("variables").fail("expected an integer in range"))?;
        registry.register(&name, &reset_value, flags)?;
        registry.set(&name, &value, true)?;
        if let Some(latched) = latched {
            registry.stage(&name, &latched)?;
        }
    }
    inner.cvars = registry;
    inner.userinfo.clear();
    for entry in reader
        .field("userinfo")
        .list(|cell| Ok::<_, WorldError>((cell.field("slot").integer(0)?, cell.field("value").string()?)))?
    {
        let (slot, value) = entry;
        let slot = i32::try_from(slot).map_err(|_| reader.field("userinfo").fail("expected an integer in range"))?;
        if slot >= MAX_CLIENTS as i32 || inner.userinfo.contains_key(&slot) {
            return Err(QvmGrappleSourceError::invalid("Invalid saved grapple client"));
        }
        inner.userinfo.insert(slot, value);
    }
    inner.configstrings.clear();
    for entry in reader
        .field("configstrings")
        .list(|cell| Ok::<_, WorldError>((cell.field("index").integer(0)?, cell.field("value").string()?)))?
    {
        let (index, value) = entry;
        let index =
            i32::try_from(index).map_err(|_| reader.field("configstrings").fail("expected an integer in range"))?;
        if index >= MAX_CONFIGSTRINGS || inner.configstrings.contains_key(&index) {
            return Err(QvmGrappleSourceError::invalid("Invalid saved grapple configstring"));
        }
        inner.configstrings.insert(index, value);
    }
    let offset = reader.field("cursor").nullable(|cell| cell.integer(0))?;
    let offset = offset
        .map(|offset| usize::try_from(offset).map_err(|_| reader.field("cursor").fail("expected an integer in range")))
        .transpose()?;
    inner.cursor.set_offset(offset)?;
    let parser = reader.field("parser");
    inner.parser.restore_save_state(&ProfileValue::record(vec![
        ("token", ProfileValue::Str(parser.field("token").string()?)),
        ("line", ProfileValue::Int(parser.field("line").integer(0)?)),
        ("name", ProfileValue::Str(parser.field("name").string()?)),
    ]))?;
    let files = decode_files(&reader.field("files"))?;
    inner.files.restore_checkpoint(&files)?;
    Ok(())
}

/// Encode open file handles (donor files checkpoint rows).
fn encode_files(checkpoint: &FilesCheckpoint) -> SaveJson {
    arr(checkpoint
        .handles
        .iter()
        .map(|(slot, handle)| match handle {
            HandleCheckpoint::Read(read) => obj(vec![
                ("slot", int(i64::from(*slot))),
                ("kind", save_str("read")),
                ("bytes", SaveJson::Bytes(read.bytes.clone())),
                ("position", int(read.position as i64)),
                ("pk3", boolean(read.pk3)),
            ]),
            HandleCheckpoint::Write(write) => obj(vec![
                ("slot", int(i64::from(*slot))),
                ("kind", save_str("write")),
                ("path", save_str(&write.path)),
                ("mode", save_str(write_mode_name(write.mode))),
                ("position", int(write.position as i64)),
            ]),
        })
        .collect())
}

/// Decode open file handles.
fn decode_files(reader: &SaveReader) -> Result<FilesCheckpoint, WorldError> {
    let handles = reader.list(|entry| {
        let slot = entry.field("slot").integer(0)? as i32;
        let handle = match entry.field("kind").choice_str(&["read", "write"])?.as_str() {
            "read" => HandleCheckpoint::Read(ReadHandleCheckpoint {
                bytes: entry.field("bytes").bytes()?,
                position: entry.field("position").integer(0)? as usize,
                pk3: entry.field("pk3").boolean()?,
            }),
            _ => HandleCheckpoint::Write(WriteHandleCheckpoint {
                path: entry.field("path").string()?,
                mode: parse_write_mode(&entry.field("mode").choice_str(&["write", "append", "append-sync"])?)?,
                position: entry.field("position").integer(0)? as usize,
            }),
        };
        Ok((slot, handle))
    })?;
    Ok(FilesCheckpoint { handles })
}

/// Write-mode checkpoint name.
fn write_mode_name(mode: WriteMode) -> &'static str {
    match mode {
        WriteMode::Write => "write",
        WriteMode::Append => "append",
        WriteMode::AppendSync => "append-sync",
    }
}

/// Parse a write-mode checkpoint name.
fn parse_write_mode(name: &str) -> Result<WriteMode, WorldError> {
    match name {
        "write" => Ok(WriteMode::Write),
        "append" => Ok(WriteMode::Append),
        "append-sync" => Ok(WriteMode::AppendSync),
        _ => Err(WorldError::BadSave("expected a file write mode".to_string())),
    }
}

/// Read a profile value tree from a checkpoint reader.
fn read_profile_value(reader: &SaveReader) -> Result<ProfileValue, WorldError> {
    match reader.value {
        None => Err(reader.fail("expected a profile value")),
        Some(SaveJson::Null) => Ok(ProfileValue::Null),
        Some(SaveJson::Bool(value)) => Ok(ProfileValue::Bool(*value)),
        Some(SaveJson::Number(value)) => Ok(ProfileValue::Float(*value)),
        Some(SaveJson::BigInt(value)) => {
            let value = i64::try_from(*value).map_err(|_| reader.fail("expected an integer in range"))?;
            Ok(ProfileValue::Int(value))
        }
        Some(SaveJson::Bytes(value)) => Ok(ProfileValue::Bytes(value.clone())),
        Some(SaveJson::String(value)) => Ok(ProfileValue::Str(value.clone())),
        Some(SaveJson::Array(_)) => Ok(ProfileValue::Array(reader.list(|cell| read_profile_value(&cell))?)),
        Some(SaveJson::Object(members)) => {
            let mut fields = Vec::with_capacity(members.len());
            for (key, _) in members {
                fields.push((key.clone(), read_profile_value(&reader.field(key))?));
            }
            Ok(ProfileValue::Record(fields))
        }
    }
}

/// Capture a profile value tree into a checkpoint image.
fn capture_profile_value(value: &ProfileValue) -> SaveJson {
    match value {
        ProfileValue::Undefined | ProfileValue::Null => SaveJson::Null,
        ProfileValue::Bool(value) => boolean(*value),
        ProfileValue::Int(value) => int(*value),
        ProfileValue::Float(value) => num(*value),
        ProfileValue::Str(value) => save_str(value),
        ProfileValue::Bytes(value) => SaveJson::Bytes(value.clone()),
        ProfileValue::Array(values) => arr(values.iter().map(capture_profile_value).collect()),
        ProfileValue::Record(fields) => obj(fields
            .iter()
            .map(|(key, value)| (key.as_str(), capture_profile_value(value)))
            .collect()),
    }
}

/// Capture a module identity into a checkpoint image.
fn capture_module_identity(identity: &qa_guest::qvm::game_data::ModuleIdentity) -> SaveJson {
    obj(vec![
        ("id", save_str(&identity.id)),
        ("artifactPath", save_str(&identity.artifact_path)),
        ("digest", save_str(&identity.digest)),
        ("revision", save_str(&identity.revision)),
    ])
}

/// Read a module identity from a checkpoint reader.
fn read_module_identity(reader: SaveReader) -> Result<qa_guest::qvm::game_data::ModuleIdentity, WorldError> {
    Ok(qa_guest::qvm::game_data::ModuleIdentity {
        id: reader.field("id").string()?,
        artifact_path: reader.field("artifactPath").string()?,
        digest: reader.field("digest").string()?,
        revision: reader.field("revision").string()?,
    })
}

/// Read a module checkpoint from a checkpoint reader.
fn read_qvm_checkpoint(reader: &SaveReader) -> Result<QvmCheckpoint, WorldError> {
    let abi = match reader
        .field("abi")
        .choice_str(&["q3-modern", "q3-1.16n-base"])?
        .as_str()
    {
        "q3-modern" => GameAbi::Modern,
        _ => GameAbi::Legacy,
    };
    let host_state = reader.field("hostState");
    Ok(QvmCheckpoint {
        module: read_module_identity(reader.field("module"))?,
        api_kind: reader.field("apiKind").string()?,
        api_version: i32::try_from(reader.field("apiVersion").integer(i64::MIN)?)
            .map_err(|_| reader.field("apiVersion").fail("expected an integer in range"))?,
        abi_profile: abi,
        data: reader.field("data").bytes()?,
        instruction_index: usize::try_from(reader.field("instructionIndex").integer(0)?)
            .map_err(|_| reader.field("instructionIndex").fail("expected an integer in range"))?,
        operand_stack: reader
            .field("operandStack")
            .list(|cell| cell.integer(i64::MIN))?
            .into_iter()
            .map(|value| {
                i32::try_from(value).map_err(|_| reader.field("operandStack").fail("expected an integer in range"))
            })
            .collect::<Result<Vec<_>, _>>()?,
        program_stack: usize::try_from(reader.field("programStack").integer(0)?)
            .map_err(|_| reader.field("programStack").fail("expected an integer in range"))?,
        host_state: QvmHostState {
            module: read_module_identity(host_state.field("module"))?,
            format: host_state.field("format").string()?,
            bytes: read_profile_value(&host_state.field("bytes"))?,
        },
    })
}

/// Capture a module checkpoint into a checkpoint image.
fn capture_qvm_checkpoint(checkpoint: &QvmCheckpoint) -> SaveJson {
    let abi = match checkpoint.abi_profile {
        GameAbi::Modern => "q3-modern",
        GameAbi::Legacy => "q3-1.16n-base",
    };
    obj(vec![
        ("kind", save_str("qvm")),
        ("module", capture_module_identity(&checkpoint.module)),
        ("apiKind", save_str(&checkpoint.api_kind)),
        ("apiVersion", int(i64::from(checkpoint.api_version))),
        ("abi", save_str(abi)),
        ("data", SaveJson::Bytes(checkpoint.data.clone())),
        ("instructionIndex", int(checkpoint.instruction_index as i64)),
        (
            "operandStack",
            arr(checkpoint
                .operand_stack
                .iter()
                .map(|word| int(i64::from(*word)))
                .collect()),
        ),
        ("programStack", int(checkpoint.program_stack as i64)),
        (
            "hostState",
            obj(vec![
                ("module", capture_module_identity(&checkpoint.host_state.module)),
                ("format", save_str(&checkpoint.host_state.format)),
                ("bytes", capture_profile_value(&checkpoint.host_state.bytes)),
            ]),
        ),
    ])
}

/// Read a grapple source checkpoint (donor
/// `readQvmGrappleSourceCheckpoint`).
pub fn read_qvm_grapple_source_checkpoint(reader: &SaveReader) -> Result<QvmGrappleSourceCheckpoint, WorldError> {
    let grapple = reader.field("grapple");
    let module_field = grapple.field("module");
    if module_field.field("kind").string()? != "qvm" {
        return Err(module_field.fail("Expected QVM grapple continuation"));
    }
    let module = read_qvm_checkpoint(&module_field)?;
    Ok(QvmGrappleSourceCheckpoint {
        version: reader.field("version").literal_i64(CHECKPOINT_VERSION as i64)? as u32,
        grapple: QvmGrappleSourceGrapple {
            version: grapple.field("version").literal_i64(CHECKPOINT_VERSION as i64)? as u32,
            profile: grapple.field("profile").string()?,
            module,
            owners: grapple.field("owners").list(read_saved_actor)?,
        },
        bindings: reader.field("bindings").list(|entry| {
            Ok(QvmGrappleSourceBinding {
                actor: read_saved_actor(entry.field("actor"))?,
                pointer: entry.field("pointer").integer(1)? as i32,
                client: entry.field("client").boolean()?,
                origin: read_vector(entry.field("origin"))?,
            })
        })?,
        tethers: reader.field("tethers").list(|entry| {
            Ok(QvmGrappleSourceTether {
                owner: read_saved_actor(entry.field("owner"))?,
                actor: read_saved_actor(entry.field("actor"))?,
            })
        })?,
    })
}

/// Capture a grapple source checkpoint in donor save shape.
#[must_use]
pub fn capture_qvm_grapple_source_checkpoint(checkpoint: &QvmGrappleSourceCheckpoint) -> SaveJson {
    obj(vec![
        ("version", int(i64::from(checkpoint.version))),
        (
            "grapple",
            obj(vec![
                ("version", int(i64::from(checkpoint.grapple.version))),
                ("profile", save_str(&checkpoint.grapple.profile)),
                ("module", capture_qvm_checkpoint(&checkpoint.grapple.module)),
                (
                    "owners",
                    arr(checkpoint
                        .grapple
                        .owners
                        .iter()
                        .map(|owner| write_saved_actor(*owner))
                        .collect()),
                ),
            ]),
        ),
        (
            "bindings",
            arr(checkpoint
                .bindings
                .iter()
                .map(|binding| {
                    obj(vec![
                        ("actor", write_saved_actor(binding.actor)),
                        ("pointer", int(i64::from(binding.pointer))),
                        ("client", boolean(binding.client)),
                        ("origin", write_vector(binding.origin)),
                    ])
                })
                .collect()),
        ),
        (
            "tethers",
            arr(checkpoint
                .tethers
                .iter()
                .map(|tether| {
                    obj(vec![
                        ("owner", write_saved_actor(tether.owner)),
                        ("actor", write_saved_actor(tether.actor)),
                    ])
                })
                .collect()),
        ),
    ])
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use qa_content::contract::{
        create_content_digest, create_mount_plan_id, ModuleIdentity as ContractModuleIdentity,
        QvmGrappleCallbacks as ContractCallbacks, QvmGrappleFields as ContractFields, QvmGrappleFovOffset,
        QvmGrappleGlobals as ContractGlobals, QvmGrappleMovement as ContractMovement,
        QvmGrapplePresentation as ContractPresentation, QvmGrappleViewAnchor as ContractAnchor, ResolvedMountPlan,
    };
    use qa_content::mounts::{open_mount_plan, OpenMountOptions};
    use qa_core::cmd_buffer::CommandOrigin;
    use qa_core::identity::{IdentityOwner, SessionId};
    use qa_guest::qvm::game_data::{QvmImage, QvmInstruction, QvmOpcode, QvmSharedMemory};
    use qa_world::registry::ActorRegistry;

    use super::*;

    const ENTITY_TEXT: &str = "{\n\"classname\" \"worldspawn\"\n}\n";

    struct FakeActors {
        registry: RefCell<ActorRegistry>,
        all: RefCell<Vec<OwnedActor>>,
        released: RefCell<HashSet<(u32, u32)>>,
    }

    impl FakeActors {
        fn new(owner: IdentityOwner) -> Self {
            Self {
                registry: RefCell::new(ActorRegistry::new(owner, 64).unwrap()),
                all: RefCell::new(Vec::new()),
                released: RefCell::new(HashSet::new()),
            }
        }
    }

    impl QvmGrappleActors for FakeActors {
        fn allocate(&self, provider: &ProviderId, definition: &str) -> OwnedActor {
            let actor = self
                .registry
                .borrow_mut()
                .allocate(provider.clone(), definition)
                .unwrap();
            self.all.borrow_mut().push(actor.clone());
            actor
        }

        fn is_live(&self, actor: &ActorId) -> bool {
            !self.released.borrow().contains(&(actor.slot(), actor.generation()))
        }

        fn release(&self, actor: &OwnedActor) {
            self.released
                .borrow_mut()
                .insert((actor.id().slot(), actor.id().generation()));
            let _ = self.registry.borrow_mut().release(actor);
        }

        fn resolve_saved(&self, saved: &SavedActorId) -> Option<OwnedActor> {
            self.all
                .borrow()
                .iter()
                .find(|actor| actor.id().slot() == saved.slot && actor.id().generation() == saved.generation)
                .cloned()
        }

        fn on_release(&self, _callback: Box<dyn Fn(&OwnedActor)>) -> Box<dyn Fn()> {
            Box::new(|| {})
        }
    }

    struct FakeBodies {
        bodies: RefCell<HashMap<ActorId, BodyState>>,
        linked: RefCell<HashSet<ActorId>>,
    }

    impl FakeBodies {
        fn new() -> Self {
            Self {
                bodies: RefCell::new(HashMap::new()),
                linked: RefCell::new(HashSet::new()),
            }
        }
    }

    impl QvmGrappleBodies for FakeBodies {
        fn read(&self, actor: &ActorId) -> Option<BodyState> {
            self.bodies.borrow().get(actor).cloned()
        }

        fn write(&self, actor: &OwnedActor, body: BodyState) {
            self.bodies.borrow_mut().insert(actor.id().clone(), body);
        }

        fn link(&self, actor: &OwnedActor) {
            self.linked.borrow_mut().insert(actor.id().clone());
        }
    }

    struct FakeScene {
        hit: RefCell<Q3TraceHit>,
        fraction: RefCell<f32>,
        actors: RefCell<Vec<ActorId>>,
        contents: RefCell<i32>,
        visible: RefCell<bool>,
        connected: RefCell<bool>,
    }

    impl FakeScene {
        fn new() -> Self {
            Self {
                hit: RefCell::new(Q3TraceHit::None),
                fraction: RefCell::new(1.0),
                actors: RefCell::new(Vec::new()),
                contents: RefCell::new(0),
                visible: RefCell::new(true),
                connected: RefCell::new(true),
            }
        }
    }

    impl QvmGrappleScene for FakeScene {
        fn trace_scene(&self, query: &Q3TraceQuery) -> Q3TraceResult {
            Q3TraceResult {
                fraction: *self.fraction.borrow(),
                end: query.end,
                hit: self.hit.borrow().clone(),
                contact: TraceContact::None,
                start_solid: false,
                all_solid: false,
                contents: *self.contents.borrow(),
                surface_flags: 0,
            }
        }

        fn point_contents_scene(&self, _query: &Q3TraceQuery, _point: Vec3) -> i32 {
            *self.contents.borrow()
        }

        fn query_actors(&self, _bounds: &Bounds) -> Vec<ActorId> {
            self.actors.borrow().clone()
        }

        fn point_leaf(&self, point: Vec3) -> i32 {
            if point.x < 0.0 {
                1
            } else {
                2
            }
        }

        fn leaf_cluster(&self, leaf: i32) -> i32 {
            leaf + 9
        }

        fn leaf_area(&self, leaf: i32) -> i32 {
            leaf + 2
        }

        fn cluster_visible(&self, _from: i32, _cluster: i32) -> bool {
            *self.visible.borrow()
        }

        fn areas_connected(&self, _first: i32, _second: i32) -> bool {
            *self.connected.borrow()
        }
    }

    struct Fixture {
        source: QvmGrappleSource,
        actors: Rc<FakeActors>,
        bodies: Rc<FakeBodies>,
        scene: Rc<FakeScene>,
        targets: Rc<RefCell<Vec<QvmGrappleTarget>>>,
        events: Rc<RefCell<Vec<Q3SourceEvent>>>,
        velocities: Rc<RefCell<Vec<(ActorId, Vec3)>>>,
        damages: Rc<RefCell<Vec<QvmGrappleDamage>>>,
    }

    fn test_body() -> BodyState {
        BodyState {
            origin: vec3(1.0, 2.0, 3.0),
            angles: vec3(0.0, 90.0, 0.0),
            velocity: vec3(4.0, 5.0, 6.0),
            bounds: Bounds {
                min: vec3(-15.0, -15.0, -24.0),
                max: vec3(15.0, 15.0, 32.0),
            },
            ground: None,
        }
    }

    fn test_definition() -> QvmGrappleDefinition {
        let digest = create_content_digest(&"ab".repeat(32)).unwrap();
        QvmGrappleDefinition {
            id: "grapple".to_string(),
            title: "Grapple".to_string(),
            module: ContractModuleIdentity {
                id: ProviderId::new("test", "qagame"),
                artifact_path: "test".to_string(),
                digest,
                revision: "1".to_string(),
            },
            abi_profile: ContractAbiProfile::Modern,
            entity_stride: 1024,
            client_stride: 1024,
            fields: ContractFields {
                inuse: 516,
                client: 520,
                parent: 524,
                target: 528,
                mover: None,
                hook: 468,
                health: 532,
                takedamage: 536,
                event_time: 540,
                free_after_event: 544,
            },
            globals: ContractGlobals {
                time: 64,
                frame: 68,
                movement: 72,
                forward: 76,
                ground_plane: 88,
            },
            callbacks: ContractCallbacks {
                allocate: 1,
                free: 2,
                fire: 3,
                release: 4,
                force_release: 5,
                missile: 6,
                follow: None,
                think: 7,
                pull: 8,
                move_mover_hooks: None,
                damage: 9,
                same_team: 10,
                player_move: 11,
            },
            fire_arguments: Vec::new(),
            movement: ContractMovement {
                byte_length: 64,
                words: Vec::new(),
            },
            initial_cvars: HashMap::new(),
            event_lifetime_milliseconds: 1000.0,
            grapple_damage_method: 0.0,
            presentation: ContractPresentation {
                projectile_model: "models/hook.md3".to_string(),
                view_model: "models/v_hook.md3".to_string(),
                weapon_index: 7.0,
                view_anchor: ContractAnchor {
                    path: "anchor".to_string(),
                    tag: "tag".to_string(),
                    offset: vec3(0.0, 0.0, 0.0),
                    fov_offset: QvmGrappleFovOffset { above: 1.0, scale: 2.0 },
                },
                view_attachments: Vec::new(),
                cable: QvmGrappleCable::Shader {
                    path: "cable".to_string(),
                    width: 4.0,
                },
                fire_sound: Some("fire.wav".to_string()),
                attach_sound: Some("attach.wav".to_string()),
                release_sound: Some("release.wav".to_string()),
                pull_sound: Some("pull.wav".to_string()),
                hang_sound: Some("hang.wav".to_string()),
            },
            pulling_flag: 64.0,
        }
    }

    fn test_artifact(definition: &QvmGrappleDefinition) -> QvmArtifact {
        QvmArtifact {
            module: GameModuleIdentity {
                id: "test:qagame".to_string(),
                artifact_path: definition.module.artifact_path.clone(),
                digest: definition.module.digest.as_str().to_string(),
                revision: "1".to_string(),
            },
            role: GameRole::Qagame,
            abi_profile: None,
            image: QvmImage {
                instructions: (0..32)
                    .map(|index| QvmInstruction::word(QvmOpcode::OpEnter, 0, index * 8))
                    .collect(),
                data_length: 4096,
                allocated_data_length: 2 * 1024 * 1024,
                ..Default::default()
            },
        }
    }

    fn test_mounts() -> MountedContent {
        let plan = ResolvedMountPlan {
            id: create_mount_plan_id("grapple-test", "1").unwrap(),
            mounts: Vec::new(),
            default_order: Vec::new(),
            prefix_orders: Vec::new(),
        };
        open_mount_plan(
            &plan,
            OpenMountOptions {
                pure: None,
                q3_restriction: None,
                links: Vec::new(),
                loose_comparison: None,
            },
        )
        .unwrap()
    }

    fn fixture() -> Fixture {
        let owner = IdentityOwner::create("grapple-test").unwrap();
        let session: SessionId = owner.session().clone();
        let actors = Rc::new(FakeActors::new(owner));
        let bodies = Rc::new(FakeBodies::new());
        let scene = Rc::new(FakeScene::new());
        let targets = Rc::new(RefCell::new(Vec::new()));
        let events = Rc::new(RefCell::new(Vec::new()));
        let velocities = Rc::new(RefCell::new(Vec::new()));
        let damages = Rc::new(RefCell::new(Vec::new()));
        let definition = test_definition();
        let artifact = test_artifact(&definition);
        let targets_fn = Rc::clone(&targets);
        let events_fn = Rc::clone(&events);
        let velocities_fn = Rc::clone(&velocities);
        let damages_fn = Rc::clone(&damages);
        let options = QvmGrappleSourceOptions {
            artifact,
            profile: definition,
            provider: ProviderId::new("test", "grapple"),
            mounts: test_mounts(),
            actors: Rc::clone(&actors) as Rc<dyn QvmGrappleActors>,
            bodies: Rc::clone(&bodies) as Rc<dyn QvmGrappleBodies>,
            scene: Rc::clone(&scene) as Rc<dyn QvmGrappleScene>,
            context: CommandContext::new(session, CommandOrigin::LocalConsole),
            seed: 7,
            entity_text: ENTITY_TEXT.to_string(),
            binding: QvmGrappleBinding::Offhand,
            targets: Rc::new(move || targets_fn.borrow().clone()),
            event: Rc::new(move |event| events_fn.borrow_mut().push(event)),
            velocity: Rc::new(move |actor, velocity| velocities_fn.borrow_mut().push((actor.clone(), velocity))),
            damage: Rc::new(move |damage| damages_fn.borrow_mut().push(damage)),
            assert_current: Rc::new(|| {}),
            real_time: Rc::new(|_| 1234),
        };
        let source = QvmGrappleSource::create(options).unwrap();
        Fixture {
            source,
            actors,
            bodies,
            scene,
            targets,
            events,
            velocities,
            damages,
        }
    }

    fn add_player(fixture: &Fixture, team: QvmGrappleTeam) -> ActorId {
        let owned = fixture
            .actors
            .allocate(&ProviderId::new("test", "players"), "test:player");
        let actor = owned.id().clone();
        let body = test_body();
        fixture.bodies.write(&owned, body.clone());
        fixture.targets.borrow_mut().push(QvmGrappleTarget {
            actor: actor.clone(),
            body,
            health: 100.0,
            kind: QvmGrappleTargetKind::Player {
                userinfo: format!("\\name\\player{}", actor.slot()),
                team,
                view_height: 26.0,
            },
        });
        actor
    }

    #[test]
    fn create_initializes_source() {
        let fixture = fixture();
        let game = fixture.source.game();
        assert_eq!(game.data.num_clients(), MAX_CLIENTS);
        assert_eq!(game.data.entity_stride_bytes(), 1024);
        assert_eq!(game.data.client_stride_bytes(), 1024);
        let owned = fixture
            .actors
            .allocate(&ProviderId::new("test", "players"), "test:player");
        assert!(fixture.source.hook(owned.id()).is_none());
        assert!(fixture
            .source
            .presentations(&ContentId("test:content".to_string()))
            .is_empty());
    }

    #[test]
    fn admit_binds_player_and_view() {
        let fixture = fixture();
        let actor = add_player(&fixture, QvmGrappleTeam::Red);
        fixture.source.admit(&actor).unwrap();
        fixture.source.admit(&actor).unwrap();
        let view = fixture.source.weapon_view(&actor).unwrap();
        assert_eq!(view.path, "models/v_hook.md3");
        assert_eq!(view.frame, 0);
        assert!(!view.q3_weapon.firing);
        assert_eq!(view.q3_weapon.weapon, 7);
        assert!(fixture.source.hook(&actor).is_none());
        assert!(!fixture.source.pulling(&actor));
    }

    #[test]
    fn admit_rejects_unknown_actor() {
        let fixture = fixture();
        let owned = fixture
            .actors
            .allocate(&ProviderId::new("test", "players"), "test:player");
        let error = fixture.source.admit(owned.id()).unwrap_err();
        assert!(error.to_string().contains("unavailable shared actor"), "{error}");
    }

    #[test]
    fn admit_rejects_non_player() {
        let fixture = fixture();
        let owned = fixture
            .actors
            .allocate(&ProviderId::new("test", "actors"), "test:actor");
        let actor = owned.id().clone();
        fixture.targets.borrow_mut().push(QvmGrappleTarget {
            actor: actor.clone(),
            body: test_body(),
            health: 50.0,
            kind: QvmGrappleTargetKind::Actor { mover: false },
        });
        let game = fixture.source.game();
        let free_pointer = game.data.entity_bytes(64).unwrap().offset as i32;
        game.module.bind_function(1, Rc::new(move |_| free_pointer));
        let error = fixture.source.admit(&actor).unwrap_err();
        assert!(error.to_string().contains("admitted source client"), "{error}");
    }

    #[test]
    fn mirror_writes_player_state() {
        let fixture = fixture();
        let actor = add_player(&fixture, QvmGrappleTeam::Blue);
        fixture.source.admit(&actor).unwrap();
        let game = fixture.source.game();
        let record = game.data.entity_bytes(0).unwrap();
        assert_eq!(record.get_i32(532).unwrap(), 100);
        assert_eq!(record.get_i32(536).unwrap(), 1);
        let ps = game.data.public_player_bytes(0).unwrap();
        assert_eq!(ps.get_f32(PS_ORIGIN).unwrap(), 1.0);
        assert_eq!(ps.get_i32(PS_TEAM).unwrap(), 2);
    }

    #[test]
    fn fire_without_hook_publishes_velocity() {
        let fixture = fixture();
        let actor = add_player(&fixture, QvmGrappleTeam::Red);
        fixture.source.admit(&actor).unwrap();
        let game = fixture.source.game();
        let layout = game.data.checkpoint();
        game.data.entity_bytes(0).unwrap().set_i32(516, 1).unwrap();
        game.data
            .entity_bytes(0)
            .unwrap()
            .set_i32(520, layout.clients_word as i32)
            .unwrap();
        fixture.source.fire(&actor).unwrap();
        assert!(fixture.source.hook(&actor).is_none());
        assert_eq!(fixture.velocities.borrow().len(), 1);
        assert_eq!(fixture.velocities.borrow()[0].0, actor);
        fixture.source.release(&actor).unwrap();
    }

    #[test]
    fn begin_frame_frees_expired_events() {
        let fixture = fixture();
        let game = fixture.source.game();
        let calls = Rc::new(RefCell::new(Vec::new()));
        let calls_hook = Rc::clone(&calls);
        game.module.bind_function(
            2,
            Rc::new(move |call| {
                calls_hook.borrow_mut().push(call.words.clone());
                let pointer = call.words[0] as usize;
                let _ = call.memory.write_i32(pointer + 516, 0);
                0
            }),
        );
        game.data.entity_bytes(64).unwrap().set_i32(516, 1).unwrap();
        game.data.entity_bytes(64).unwrap().set_i32(544, 1).unwrap();
        game.data.entity_bytes(64).unwrap().set_i32(540, 0).unwrap();
        fixture.source.begin_frame(2000, 1).unwrap();
        assert_eq!(game.data.entity_bytes(64).unwrap().get_i32(516).unwrap(), 0);
        assert_eq!(calls.borrow().len(), 1);
        fixture.source.begin_frame(2001, 2).unwrap();
        assert_eq!(calls.borrow().len(), 1);
    }

    #[test]
    fn damage_hook_reports_and_syncs_health() {
        let fixture = fixture();
        let actor = add_player(&fixture, QvmGrappleTeam::Red);
        fixture.source.admit(&actor).unwrap();
        let game = fixture.source.game();
        let pointer = game.data.entity_bytes(0).unwrap().offset as i32;
        let scratch = 48;
        game.module.memory().write_vec3(scratch, &vec3(0.0, 0.0, 1.0)).unwrap();
        game.module
            .memory()
            .write_vec3(scratch + 12, &vec3(1.0, 2.0, 3.0))
            .unwrap();
        let result = game
            .module
            .call(&[pointer, 0, 0, scratch as i32, (scratch + 12) as i32, 50, 0, 1], 9);
        assert_eq!(result.unwrap(), 0);
        assert_eq!(fixture.damages.borrow().len(), 1);
        let damage = fixture.damages.borrow()[0].clone();
        assert_eq!(damage.target, actor);
        assert_eq!(damage.amount, 50);
        assert_eq!(damage.direction, vec3(0.0, 0.0, 1.0));
        assert_eq!(damage.point, vec3(1.0, 2.0, 3.0));
        assert_eq!(game.data.entity_bytes(0).unwrap().get_i32(532).unwrap(), 100);
    }

    #[test]
    fn same_team_hook_compares_teams() {
        let fixture = fixture();
        let red_a = add_player(&fixture, QvmGrappleTeam::Red);
        let red_b = add_player(&fixture, QvmGrappleTeam::Red);
        let blue = add_player(&fixture, QvmGrappleTeam::Blue);
        fixture.source.admit(&red_a).unwrap();
        fixture.source.admit(&red_b).unwrap();
        fixture.source.admit(&blue).unwrap();
        let game = fixture.source.game();
        let pointer = |slot: usize| game.data.entity_bytes(slot).unwrap().offset as i32;
        assert_eq!(game.module.call(&[pointer(0), pointer(1)], 10).unwrap(), 1);
        assert_eq!(game.module.call(&[pointer(0), pointer(2)], 10).unwrap(), 0);
    }

    fn trace_query() -> ServerTraceQuery {
        ServerTraceQuery {
            start: vec3(0.0, 0.0, 0.0),
            end: vec3(0.0, 0.0, 100.0),
            mins: vec3(0.0, 0.0, 0.0),
            maxs: vec3(0.0, 0.0, 0.0),
            shape: GuestTraceShape::Box,
            pass_entity_num: ENTITYNUM_NONE,
            mask: -1,
        }
    }

    #[test]
    fn trace_maps_actor_hits_and_misses() {
        let fixture = fixture();
        let actor = add_player(&fixture, QvmGrappleTeam::Free);
        fixture.source.admit(&actor).unwrap();
        *fixture.scene.hit.borrow_mut() = Q3TraceHit::Actor { actor: actor.clone() };
        *fixture.scene.fraction.borrow_mut() = 0.5;
        let mut host = fixture.source.handles();
        let record = host.trace(&trace_query());
        assert_eq!(record.entity_num, 0);
        assert_eq!(record.fraction, 0.5);
        *fixture.scene.hit.borrow_mut() = Q3TraceHit::None;
        *fixture.scene.fraction.borrow_mut() = 1.0;
        assert_eq!(host.trace(&trace_query()).entity_num, ENTITYNUM_NONE);
        *fixture.scene.fraction.borrow_mut() = 0.25;
        assert_eq!(host.trace(&trace_query()).entity_num, ENTITYNUM_WORLD);
    }

    #[test]
    fn spatial_queries_cover_contents_areas_and_links() {
        let fixture = fixture();
        let actor = add_player(&fixture, QvmGrappleTeam::Free);
        fixture.source.admit(&actor).unwrap();
        *fixture.scene.contents.borrow_mut() = 3;
        fixture.scene.actors.borrow_mut().push(actor.clone());
        let mut host = fixture.source.handles();
        assert_eq!(host.point_contents(vec3(1.0, 1.0, 1.0), ENTITYNUM_NONE), 3);
        assert_eq!(
            host.area_entities(
                ServerBounds {
                    min: vec3(-99.0, -99.0, -99.0),
                    max: vec3(99.0, 99.0, 99.0)
                },
                -1
            ),
            vec![0]
        );
        assert!(host
            .area_entities(
                ServerBounds {
                    min: vec3(-99.0, -99.0, -99.0),
                    max: vec3(99.0, 99.0, 99.0)
                },
                0
            )
            .is_empty());
        assert!(host.in_pvs(vec3(-1.0, 0.0, 0.0), vec3(1.0, 0.0, 0.0), false));
        *fixture.scene.visible.borrow_mut() = false;
        assert!(!host.in_pvs(vec3(-1.0, 0.0, 0.0), vec3(1.0, 0.0, 0.0), true));
        assert!(host.areas_connected(3, 4));
        host.link(0);
        assert!(fixture.source.game().data.entity(0).unwrap().r.linked);
        host.unlink(0);
        assert!(!fixture.source.game().data.entity(0).unwrap().r.linked);
    }

    #[test]
    fn configstrings_roundtrip_with_events() {
        let fixture = fixture();
        let mut host = fixture.source.handles();
        host.config_set(5, "value");
        assert_eq!(host.config_get(5), "value");
        assert_eq!(host.config_get(6), "");
        host.set_userinfo(0, "info");
        assert_eq!(host.get_userinfo(0), "info");
        host.send_server_command(0, "cmd");
        let outbox = host.inner.borrow().outbox.clone();
        assert_eq!(outbox.len(), 2);
    }

    #[test]
    fn entity_token_consumes_initialization_text() {
        let fixture = fixture();
        let mut host = fixture.source.handles();
        let (token, ended) = host.entity_token();
        assert_eq!(token, "{");
        assert!(!ended);
    }

    fn game_call(code: i32, words: Vec<i32>) -> GameCall {
        GameCall {
            kind: GameKind::Engine,
            role: GameRole::Qagame,
            code,
            words,
            guest: QvmSharedMemory::new(64).unwrap(),
            abi_profile: GameAbi::Modern,
            command_arguments: None,
        }
    }

    fn weak_handles(source: &QvmGrappleSource) -> WeakHandles {
        WeakHandles {
            inner: Rc::downgrade(&source.inner),
            provider: Rc::downgrade(&source.provider),
        }
    }

    #[test]
    fn dispatch_covers_print_milliseconds_botlib_and_unknown() {
        let fixture = fixture();
        fixture.source.begin_frame(4242, 1).unwrap();
        let weak = weak_handles(&fixture.source);
        let call = game_call(QvmGameImport::G_MILLISECONDS, vec![QvmGameImport::G_MILLISECONDS]);
        assert_eq!(dispatch_syscall(&weak, &call).unwrap(), Some(4242));
        let call = game_call(QvmGameImport::G_PRINT, vec![QvmGameImport::G_PRINT, 8]);
        call.guest.write_string(8, "hello", 32).unwrap();
        assert_eq!(dispatch_syscall(&weak, &call).unwrap(), Some(0));
        assert!(fixture.events.borrow().iter().any(|event| matches!(
            event,
            Q3SourceEvent::Print { text } if text == "hello"
        )));
        let setup = game_call(BOTLIB_SETUP, vec![BOTLIB_SETUP]);
        assert_eq!(dispatch_syscall(&weak, &setup).unwrap(), Some(0));
        let unknown = game_call(9999, vec![9999]);
        assert_eq!(dispatch_syscall(&weak, &unknown).unwrap(), None);
    }

    #[test]
    fn capture_restore_roundtrip() {
        let fixture = fixture();
        let actor = add_player(&fixture, QvmGrappleTeam::Red);
        fixture.source.admit(&actor).unwrap();
        fixture.source.begin_frame(500, 1).unwrap();
        let checkpoint = fixture.source.capture().unwrap();
        assert_eq!(checkpoint.version, CHECKPOINT_VERSION);
        assert_eq!(checkpoint.bindings.len(), 1);
        assert!(checkpoint.bindings[0].client);
        assert!(checkpoint.tethers.is_empty());
        fixture.source.restore(&checkpoint).unwrap();
        let view = fixture.source.weapon_view(&actor).unwrap();
        assert_eq!(view.q3_weapon.time_milliseconds, 500);
        let again = fixture.source.capture().unwrap();
        assert_eq!(again.bindings.len(), 1);
        assert_eq!(again.bindings[0].pointer, checkpoint.bindings[0].pointer);
    }

    #[test]
    fn restore_rejects_bad_versions_and_duplicates() {
        let fixture = fixture();
        let actor = add_player(&fixture, QvmGrappleTeam::Red);
        let checkpoint = fixture.source.capture().unwrap();
        let bad = QvmGrappleSourceCheckpoint {
            version: 2,
            ..checkpoint.clone()
        };
        assert!(fixture.source.restore(&bad).is_err());
        let missing = QvmGrappleSourceCheckpoint {
            bindings: vec![QvmGrappleSourceBinding {
                actor: SavedActorId { slot: 9, generation: 9 },
                pointer: 64,
                client: false,
                origin: vec3(0.0, 0.0, 0.0),
            }],
            ..checkpoint.clone()
        };
        assert!(fixture.source.restore(&missing).is_err());
        let saved = SavedActorId::from(&actor);
        let duplicate = QvmGrappleSourceCheckpoint {
            bindings: vec![
                QvmGrappleSourceBinding {
                    actor: saved,
                    pointer: 64,
                    client: false,
                    origin: vec3(0.0, 0.0, 0.0),
                },
                QvmGrappleSourceBinding {
                    actor: saved,
                    pointer: 128,
                    client: false,
                    origin: vec3(0.0, 0.0, 0.0),
                },
            ],
            ..checkpoint.clone()
        };
        let error = fixture.source.restore(&duplicate).unwrap_err();
        assert!(error.to_string().contains("Duplicate saved grapple binding"), "{error}");
    }

    #[test]
    fn checkpoint_image_roundtrip() {
        let fixture = fixture();
        let actor = add_player(&fixture, QvmGrappleTeam::Red);
        fixture.source.admit(&actor).unwrap();
        let checkpoint = fixture.source.capture().unwrap();
        let image = capture_qvm_grapple_source_checkpoint(&checkpoint);
        let reader = SaveReader::new(&image);
        let restored = read_qvm_grapple_source_checkpoint(&reader).unwrap();
        assert_eq!(restored.version, CHECKPOINT_VERSION);
        assert_eq!(restored.grapple.profile, checkpoint.grapple.profile);
        assert_eq!(restored.bindings.len(), 1);
        assert_eq!(restored.bindings[0].actor, SavedActorId::from(&actor));
        assert!(restored.tethers.is_empty());
        fixture.source.restore(&restored).unwrap();
    }

    #[test]
    fn checkpoint_reader_rejects_non_qvm_module() {
        let fixture = fixture();
        let checkpoint = fixture.source.capture().unwrap();
        let mut image = capture_qvm_grapple_source_checkpoint(&checkpoint);
        let SaveJson::Object(members) = &mut image else {
            panic!("checkpoint image is an object")
        };
        let grapple = members.iter_mut().find(|(key, _)| key == "grapple").unwrap();
        let SaveJson::Object(grapple) = &mut grapple.1 else {
            panic!("grapple row is an object")
        };
        let module = grapple.iter_mut().find(|(key, _)| key == "module").unwrap();
        let SaveJson::Object(module) = &mut module.1 else {
            panic!("module row is an object")
        };
        let kind = module.iter_mut().find(|(key, _)| key == "kind").unwrap();
        kind.1 = save_str("quakec");
        let reader = SaveReader::new(&image);
        let error = read_qvm_grapple_source_checkpoint(&reader).unwrap_err();
        assert!(
            error.to_string().contains("Expected QVM grapple continuation"),
            "{error}"
        );
    }

    #[test]
    fn close_retires_source() {
        let fixture = fixture();
        let actor = add_player(&fixture, QvmGrappleTeam::Red);
        fixture.source.close();
        fixture.source.close();
        let error = fixture.source.admit(&actor).unwrap_err();
        assert!(error.to_string().contains("closed"), "{error}");
    }

    #[test]
    fn profile_converter_maps_definition() {
        let mut definition = test_definition();
        definition.entity_stride = 2048;
        definition.fire_arguments = vec![1.0, 2.0];
        let profile = profile_from_definition(&definition).unwrap();
        assert_eq!(profile.entity_stride, 2048);
        assert_eq!(profile.client_stride, 1024);
        assert_eq!(profile.fields.health, 532);
        assert_eq!(profile.callbacks.same_team, 10);
        assert_eq!(profile.fire_arguments, vec![1, 2]);
        assert_eq!(profile.pulling_flag, 64);
        assert_eq!(profile.presentation.weapon_index, 7);
        assert_eq!(definition.module.id.namespace, "test");
    }

    #[test]
    fn pull_without_tether_is_ok() {
        let fixture = fixture();
        let actor = add_player(&fixture, QvmGrappleTeam::Red);
        fixture.source.admit(&actor).unwrap();
        fixture.source.pull(&actor).unwrap();
        assert!(!fixture.source.pulling(&actor));
    }

    #[test]
    fn presentations_cover_shader_and_model_cables() {
        for cable in [
            QvmGrappleCable::Shader {
                path: "cable".to_string(),
                width: 4.0,
            },
            QvmGrappleCable::Model {
                flight: "flight.md3".to_string(),
                pull: "pull.md3".to_string(),
                hold: "hold.md3".to_string(),
                segment_length: 8.0,
            },
        ] {
            let fixture = fixture();
            let actor = add_player(&fixture, QvmGrappleTeam::Red);
            fixture.source.admit(&actor).unwrap();
            let hook = fixture
                .actors
                .allocate(&ProviderId::new("test", "grapple"), HOOK_DEFINITION);
            fixture.bodies.write(&hook, test_body());
            fixture.source.inner.borrow_mut().tethers.insert(
                actor.clone(),
                Tether {
                    actor: hook.clone(),
                    projection: QvmGrappleProjection {
                        hook: 64,
                        origin: vec3(0.0, 0.0, 0.0),
                        velocity: vec3(0.0, 0.0, 0.0),
                        target: None,
                        mover: None,
                        pulling: true,
                        point: vec3(0.0, 0.0, 10.0),
                        owner_velocity: vec3(0.0, 0.0, 0.0),
                    },
                },
            );
            fixture.source.inner.borrow_mut().presentation.cable = cable.clone();
            let rows = fixture.source.presentations(&ContentId("test:content".to_string()));
            assert_eq!(rows.len(), 2);
            assert_eq!(rows[0].actor, *hook.id());
            assert_eq!(rows[0].path, "models/hook.md3");
            match cable {
                QvmGrappleCable::Shader { .. } => {
                    assert!(rows[1].shader_beam.is_some());
                    assert!(rows[1].q3_grapple_cable.is_none());
                }
                QvmGrappleCable::Model { .. } => {
                    assert!(rows[1].shader_beam.is_none());
                    let cable = rows[1].q3_grapple_cable.as_ref().unwrap();
                    assert_eq!(cable.owner, actor);
                    assert!(cable.attached);
                    assert!(cable.offhand);
                }
            }
        }
    }
}
