//! QVM client-input binding: envelopes, movement applications, aim mapping.
//!
//! Provenance: `src/compat/qvm/game-input.ts`.
//!
//! The hub delivers host effects synchronously and hooks return plain words,
//! so the port performs the donor's post-proceed finishes inline (in donor
//! order) and records the first hook failure for [`QvmInputBinding::take_error`]
//! instead of throwing through the hook. Async branches collapse to the sync
//! path; the "cannot await disconnect" failure is unrepresentable.
//!
//! Local mirrors (the donor's neighboring modules belong to sibling workers):
//! the user-command record of `src/compat/qvm/client-state-record.ts`
//! ([`QvmUserCommandRecord`], [`QVM_USER_COMMAND_BYTES`],
//! [`read_qvm_user_command`], [`write_qvm_user_command`]), the used subset of
//! `src/contracts/mod-client-outputs.ts` ([`QvmMovementMode`],
//! [`QvmMovementOutputs`]), the `q3` arm of `src/contracts/protocol.ts`
//! ([`Q3UserCommand`]), the used subset of `src/contracts/time.ts`
//! ([`QvmSourceTime`], [`QvmFramePhase`], [`QvmFrameContext`]),
//! `src/contracts/session.ts` ([`QvmCommandSource`], [`QvmActorCommand`]),
//! `src/contracts/gameplay.ts` ([`QvmArsenalIntent`]),
//! `src/world/session/mod-clients.ts` ([`QvmClientIdentity`],
//! [`QvmClientCommand`], [`QvmApplicationScope`], [`QvmClientApplication`]),
//! `src/world/session/mod-client-applications.ts` ([`QvmApplicationInput`],
//! [`QvmClientApplications`]), `src/movement/q3/view.ts`
//! ([`qvm_view_angles`]), and the `q3` arm of
//! `src/movement/client-outputs.ts` ([`apply_stance_command`]).
//!
//! [`QvmGame`]: super::game::QvmGame

use std::cell::RefCell;
use std::rc::Rc;

use qa_core::identity::{ActorId, ClientId, ProviderId, SeatId};
use qa_core::math::{vec3, Vec3};
use qa_world::combat::ItemId;

use super::game::QvmGame;
use super::game_data::{AbiProfile, ModuleIdentity, QvmCancellationScope, QvmFunctionCall, QvmHookFn, QvmModule};
use crate::error::GuestError;

/// User-command record bytes.
pub const QVM_USER_COMMAND_BYTES: usize = 24;

/// Wire user command (mirror of `WireUserCommand`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QvmUserCommandRecord {
    /// Server time in milliseconds.
    pub server_time: i32,
    /// Angle words.
    pub angles: [i32; 3],
    /// Buttons.
    pub buttons: i32,
    /// Weapon.
    pub weapon: i32,
    /// Forward move.
    pub forward_move: i32,
    /// Right move.
    pub right_move: i32,
    /// Up move.
    pub up_move: i32,
}

/// Read a user-command record.
pub fn read_qvm_user_command(bytes: &[u8], profile: AbiProfile) -> Result<QvmUserCommandRecord, GuestError> {
    if bytes.len() < QVM_USER_COMMAND_BYTES {
        return Err(GuestError::invalid("QVM user command exceeds its record"));
    }
    let word =
        |offset: usize| i32::from_le_bytes([bytes[offset], bytes[offset + 1], bytes[offset + 2], bytes[offset + 3]]);
    let server_time = word(0);
    if profile.is_modern() {
        return Ok(QvmUserCommandRecord {
            server_time,
            angles: [word(4), word(8), word(12)],
            buttons: word(16),
            weapon: i32::from(bytes[20]),
            forward_move: i32::from(bytes[21] as i8),
            right_move: i32::from(bytes[22] as i8),
            up_move: i32::from(bytes[23] as i8),
        });
    }
    let packed = bytes[4];
    Ok(QvmUserCommandRecord {
        server_time,
        angles: [word(8), word(12), word(16)],
        buttons: i32::from(packed & 31) | if packed & 128 == 0 { 0 } else { 2048 },
        weapon: i32::from(bytes[5]),
        forward_move: i32::from(bytes[20] as i8),
        right_move: i32::from(bytes[21] as i8),
        up_move: i32::from(bytes[22] as i8),
    })
}

/// Write a user-command record; update mode preserves legacy private bits.
pub fn write_qvm_user_command(
    bytes: &mut [u8],
    command: &QvmUserCommandRecord,
    profile: AbiProfile,
    update: bool,
) -> Result<(), GuestError> {
    if bytes.len() < QVM_USER_COMMAND_BYTES {
        return Err(GuestError::invalid("QVM user command exceeds its record"));
    }
    bytes[0..4].copy_from_slice(&command.server_time.to_le_bytes());
    let angles = if profile.is_modern() { 4 } else { 8 };
    for (index, angle) in command.angles.iter().enumerate() {
        let offset = angles + index * 4;
        bytes[offset..offset + 4].copy_from_slice(&angle.to_le_bytes());
    }
    if profile.is_modern() {
        bytes[16..20].copy_from_slice(&command.buttons.to_le_bytes());
        bytes[20] = command.weapon as u8;
        bytes[21] = command.forward_move as u8;
        bytes[22] = command.right_move as u8;
        bytes[23] = command.up_move as u8;
        return Ok(());
    }
    bytes[4] = (if update { bytes[4] & 96 } else { 0 })
        | ((command.buttons & 31) as u8)
        | (if command.buttons & 2048 == 0 { 0 } else { 128 });
    bytes[5] = command.weapon as u8;
    bytes[20] = command.forward_move as u8;
    bytes[21] = command.right_move as u8;
    bytes[22] = command.up_move as u8;
    Ok(())
}

/// Requested movement mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QvmMovementMode {
    /// Normal movement.
    Normal,
    /// Noclip movement.
    Noclip,
    /// Frozen movement.
    Freeze,
}

/// Movement outputs consumed by input (mode and stance only).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct QvmMovementOutputs {
    /// Requested movement mode.
    pub mode: Option<QvmMovementMode>,
    /// Requested crouch stance.
    pub stance: Option<bool>,
}

/// Quake III user command (the `q3` dialect arm).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Q3UserCommand {
    /// Server time in milliseconds.
    pub server_time_ms: i32,
    /// Angle words.
    pub angle_words: [i32; 3],
    /// Buttons.
    pub buttons: i32,
    /// Weapon.
    pub weapon: i32,
    /// Forward move.
    pub forward_move: i32,
    /// Right move.
    pub right_move: i32,
    /// Up move.
    pub up_move: i32,
}

impl From<&QvmUserCommandRecord> for Q3UserCommand {
    fn from(command: &QvmUserCommandRecord) -> Self {
        Self {
            server_time_ms: command.server_time,
            angle_words: command.angles,
            buttons: command.buttons,
            weapon: command.weapon,
            forward_move: command.forward_move,
            right_move: command.right_move,
            up_move: command.up_move,
        }
    }
}

/// Source clock reading.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct QvmSourceTime {
    /// Seconds or milliseconds.
    pub kind: QvmTimeKind,
    /// Clock value.
    pub value: f64,
}

/// Source clock units.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QvmTimeKind {
    /// Seconds.
    Seconds,
    /// Milliseconds.
    Milliseconds,
}

/// Frame phase.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QvmFramePhase {
    /// Frame entry.
    FrameEntry,
    /// Client command.
    ClientCommand,
    /// Entity prethink.
    EntityPrethink,
    /// Entity physics.
    EntityPhysics,
    /// Entity think.
    EntityThink,
    /// Client end frame.
    ClientEndFrame,
    /// Frame exit.
    FrameExit,
}

/// Frame context.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct QvmFrameContext {
    /// Frame ordinal.
    pub frame: i64,
    /// Frame time.
    pub time: QvmSourceTime,
    /// Elapsed time.
    pub elapsed: QvmSourceTime,
    /// Frame phase.
    pub phase: QvmFramePhase,
}

/// Angle space of an explicit aim.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QvmAngleSpace {
    /// Absolute aim.
    Absolute,
    /// Source-relative aim.
    SourceRelative,
}

/// Command source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QvmCommandSource {
    /// Local seat.
    LocalSeat {
        /// Seat handle.
        seat: SeatId,
        /// Client handle.
        client: ClientId,
    },
    /// Remote client.
    RemoteClient {
        /// Client handle.
        client: ClientId,
    },
    /// Bot provider.
    Bot {
        /// Provider name.
        provider: ProviderId,
    },
}

/// Arsenal intent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QvmArsenalIntent {
    /// Owning provider.
    pub provider: ProviderId,
    /// Selected weapon (`None` retains).
    pub weapon: Option<ItemId>,
    /// Use the holdable item.
    pub use_holdable: bool,
    /// Impulse.
    pub impulse: Option<i32>,
}

/// Actor command carrying a Quake III command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QvmActorCommand {
    /// Acting actor.
    pub actor: ActorId,
    /// Command source.
    pub source: QvmCommandSource,
    /// Sequence number.
    pub sequence: i64,
    /// Effective command.
    pub command: Q3UserCommand,
    /// Explicit aim space, if any.
    pub angle_space: Option<QvmAngleSpace>,
    /// Arsenal intent, if any.
    pub arsenal: Option<QvmArsenalIntent>,
}

/// Connected-client identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QvmClientIdentity {
    /// Client handle.
    pub client: ClientId,
    /// Actor handle.
    pub actor: ActorId,
}

/// Accepted client command receipt.
#[derive(Debug, Clone, PartialEq)]
pub struct QvmClientCommand {
    /// Accepted input.
    pub input: QvmActorCommand,
    /// Accepted time.
    pub time: QvmSourceTime,
}

/// Client application scope.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QvmApplicationScope {
    /// Whole client command.
    ClientCommand,
    /// Movement slice.
    MovementSlice,
}

/// Live client application.
#[derive(Debug, Clone, PartialEq)]
pub struct QvmClientApplication {
    /// Client identity.
    pub identity: QvmClientIdentity,
    /// Invocation ordinal.
    pub invocation: u64,
    /// Parent invocation, if any.
    pub parent_invocation: Option<u64>,
    /// Application scope.
    pub scope: QvmApplicationScope,
    /// Effective command.
    pub command: Q3UserCommand,
    /// Aim space.
    pub angle_space: QvmAngleSpace,
    /// Absolute aim.
    pub absolute_aim: Vec3,
    /// Frame context.
    pub frame: QvmFrameContext,
    /// Accepted receipt, if any.
    pub accepted: Option<QvmClientCommand>,
    /// Arsenal intent, if any.
    pub arsenal: Option<QvmArsenalIntent>,
    /// Impulse control.
    pub impulse: i32,
}

/// Application input (invocation assigned by the journal).
#[derive(Debug, Clone, PartialEq)]
pub struct QvmApplicationInput {
    /// Client identity.
    pub identity: QvmClientIdentity,
    /// Requested parent invocation, if any.
    pub parent_invocation: Option<u64>,
    /// Application scope.
    pub scope: QvmApplicationScope,
    /// Input command.
    pub command: Q3UserCommand,
    /// Aim space.
    pub angle_space: QvmAngleSpace,
    /// Absolute aim.
    pub absolute_aim: Vec3,
    /// Frame context.
    pub frame: QvmFrameContext,
    /// Accepted receipt, if any.
    pub accepted: Option<QvmClientCommand>,
    /// Arsenal intent, if any.
    pub arsenal: Option<QvmArsenalIntent>,
    /// Impulse control.
    pub impulse: i32,
}

/// Client application journal (used surface of `ModClientApplications`).
pub trait QvmClientApplications {
    /// Whether any listener is subscribed.
    fn active(&self) -> bool;
    /// Begin an application, running before-listeners through `encode_aim`.
    fn begin(
        &self,
        input: &QvmApplicationInput,
        encode_aim: &mut dyn FnMut(&Vec3, &Q3UserCommand) -> Result<Q3UserCommand, GuestError>,
    ) -> Result<Option<QvmClientApplication>, GuestError>;
    /// Finish an application; `None` is a no-op.
    fn finish(&self, application: Option<&QvmClientApplication>, failed: bool) -> Result<(), GuestError>;
}

/// Computed view angles plus adjusted delta words.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct QvmViewAngles {
    /// View angles in degrees.
    pub angles: Vec3,
    /// Adjusted delta words.
    pub delta_words: [i32; 3],
}

fn sign16(value: i32) -> i32 {
    (value << 16) >> 16
}

/// Quake III view-angle update (`PM_UpdateViewAngles`).
#[must_use]
pub fn qvm_view_angles(
    command: [i32; 3],
    delta: [i32; 3],
    previous: &Vec3,
    health: i32,
    movement_type: i32,
    intermission: &[i32],
) -> QvmViewAngles {
    if intermission.contains(&movement_type) || (movement_type != 2 && health <= 0) {
        return QvmViewAngles {
            angles: previous.clone(),
            delta_words: delta,
        };
    }
    let mut delta_words = delta;
    let pitch = sign16(command[0].wrapping_add(delta[0]));
    let pitch = if pitch > 16000 {
        delta_words[0] = 16000_i32.wrapping_sub(command[0]);
        16000
    } else if pitch < -16000 {
        delta_words[0] = (-16000_i32).wrapping_sub(command[0]);
        -16000
    } else {
        pitch
    };
    let scale = 360.0 / 65536.0;
    QvmViewAngles {
        angles: vec3(
            pitch as f32 * scale as f32,
            sign16(command[1].wrapping_add(delta[1])) as f32 * scale as f32,
            sign16(command[2].wrapping_add(delta[2])) as f32 * scale as f32,
        ),
        delta_words,
    }
}

/// Apply a crouch request to a Quake III command.
///
/// Returns `None` when the command is unchanged; the donor compares
/// references, but rewriting identical values is idempotent, so value
/// equality selects the same writes.
#[must_use]
pub fn apply_stance_command(command: &Q3UserCommand, crouched: Option<bool>) -> Option<Q3UserCommand> {
    let crouched = crouched?;
    let up_move = if crouched {
        -command.up_move.abs().max(1)
    } else {
        command.up_move.max(0)
    };
    if up_move == command.up_move {
        return None;
    }
    Some(Q3UserCommand { up_move, ..*command })
}

/// Source movement-mode words.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QvmMovementModes {
    /// Normal word.
    pub normal: i32,
    /// Noclip word.
    pub noclip: i32,
    /// Freeze word.
    pub freeze: i32,
}

impl QvmMovementModes {
    /// Word for a requested mode.
    #[must_use]
    pub const fn word(self, mode: QvmMovementMode) -> i32 {
        match mode {
            QvmMovementMode::Normal => self.normal,
            QvmMovementMode::Noclip => self.noclip,
            QvmMovementMode::Freeze => self.freeze,
        }
    }
}

/// Hooked input entry points.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QvmInputEntries {
    /// ClientThink entry.
    pub client_think: usize,
    /// RunClient entry.
    pub run_client: usize,
    /// ClientSpawn entry.
    pub client_spawn: usize,
    /// Pmove entry.
    pub move_entry: usize,
    /// PmoveSingle entry.
    pub slice: usize,
}

/// Declared input layout.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QvmInputDefinition {
    /// Owning module.
    pub module: ModuleIdentity,
    /// Entity stride in bytes.
    pub entity_stride: usize,
    /// Client stride in bytes.
    pub client_stride: usize,
    /// Client pointer offset within an entity record.
    pub client_pointer: usize,
    /// Intermission movement types.
    pub intermission: Vec<i32>,
    /// Movement-mode words, if any.
    pub movement_modes: Option<QvmMovementModes>,
    /// Hooked entries.
    pub entries: QvmInputEntries,
}

/// Release listener type.
pub type QvmReleaseListener = Rc<dyn Fn(ActorId)>;
/// Release remover type.
pub type QvmReleaseHandle = Box<dyn FnOnce()>;

/// Host services consumed by the input binding.
pub trait QvmInputServices {
    /// Application journal.
    fn applications(&self) -> &dyn QvmClientApplications;
    /// Movement outputs for an actor.
    fn client_outputs(&self, _actor: &ActorId) -> Option<QvmMovementOutputs> {
        None
    }
    /// Identity of a wire slot.
    fn identity(&self, slot: usize) -> Option<QvmClientIdentity>;
    /// Whether an identity is live.
    fn live(&self, identity: &QvmClientIdentity) -> bool;
    /// Accepted command for an actor.
    fn accepted(&self, actor: &ActorId) -> Option<QvmClientCommand>;
    /// Current frame context.
    fn frame(&self) -> QvmFrameContext;
    /// Observe a completed spawn.
    fn spawned(&self, _identity: &QvmClientIdentity) {}
    /// Whether [`QvmInputServices::spawned`] is overridden.
    fn observes_spawns(&self) -> bool {
        false
    }
    /// Wrap a movement invocation.
    fn movement(
        &self,
        call: &mut QvmFunctionCall,
        _kind: QvmApplicationScope,
        run: &mut dyn FnMut(&mut QvmFunctionCall) -> Result<i32, GuestError>,
    ) -> Result<i32, GuestError> {
        run(call)
    }
    /// Subscribe to actor release; returns a remover.
    fn on_release(&self, listener: QvmReleaseListener) -> QvmReleaseHandle;
}

/// Source module owning the input binding.
pub trait QvmInputSource {
    /// Source game.
    fn game(&self) -> &QvmGame;
    /// Input declaration.
    fn definition(&self) -> &QvmInputDefinition;
    /// Mark a slot retiring.
    fn retiring(&self, slot: usize, identity: &QvmClientIdentity);
    /// Whether a slot retired.
    fn retired(&self, slot: usize) -> bool;
    /// Disconnect a dead slot.
    fn disconnect(
        &self,
        slot: usize,
        identity: &QvmClientIdentity,
        call: &mut QvmFunctionCall,
    ) -> Result<(), GuestError>;
    /// Run a movement application for a slot.
    fn movement(
        &self,
        slot: usize,
        call: &mut QvmFunctionCall,
        run: &mut dyn FnMut(&mut QvmFunctionCall) -> Result<i32, GuestError>,
    ) -> Result<i32, GuestError>;
}

/// Enveloped client scope.
#[derive(Debug, Clone, PartialEq, Eq)]
struct QvmClientScope {
    /// Scope ordinal.
    id: u64,
    /// Wire slot.
    slot: usize,
    /// Client identity.
    identity: QvmClientIdentity,
    /// Cancellation token.
    cancellation: QvmCancellationScope,
}

/// Suspended command frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct QvmCommandFrame {
    /// Owning scope ordinal.
    scope_id: u64,
    /// Command address.
    address: usize,
}

/// Projected movement-mode word.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct QvmProjectedType {
    /// Word before projection.
    before: i32,
    /// Projected word.
    value: i32,
}

/// Shared binding state behind hook closures.
struct QvmInputState {
    /// Source module.
    source: Rc<dyn QvmInputSource>,
    /// Host services.
    services: Rc<dyn QvmInputServices>,
    /// Enveloped scopes, innermost last.
    clients: Vec<QvmClientScope>,
    /// Slots inside ClientSpawn.
    spawning: Vec<usize>,
    /// Suspended command frames.
    command_frames: Vec<QvmCommandFrame>,
    /// Innermost live application.
    current: Option<QvmClientApplication>,
    /// Cancelled scope ordinals.
    cancelled: Vec<u64>,
    /// Next scope ordinal.
    next_scope: u64,
    /// First recorded hook failure.
    error: Option<GuestError>,
}

/// Cloned handles plus snapshots for one lock-free step.
#[derive(Clone)]
struct QvmInputDeps {
    /// Source module.
    source: Rc<dyn QvmInputSource>,
    /// Host services.
    services: Rc<dyn QvmInputServices>,
}

impl QvmInputState {
    /// Clone shared handles.
    fn deps(&self) -> QvmInputDeps {
        QvmInputDeps {
            source: Rc::clone(&self.source),
            services: Rc::clone(&self.services),
        }
    }
}

/// Locate the public player state owned by a slot's entity record.
fn client_pointer(deps: &QvmInputDeps, slot: usize) -> Result<usize, GuestError> {
    let game = deps.source.game();
    let definition = deps.source.definition();
    let located = game.data.checkpoint();
    if located.entity_stride != definition.entity_stride || located.client_stride != definition.client_stride {
        return Err(GuestError::invalid(
            "Located QVM input records differ from their artifact declaration",
        ));
    }
    let client = game.data.entity_bytes(slot)?.get_i32(definition.client_pointer)?;
    let expected = located
        .clients_word
        .checked_add(
            slot.checked_mul(located.client_stride)
                .ok_or_else(|| GuestError::invalid("QVM input entity does not own its located public player state"))?,
        )
        .ok_or_else(|| GuestError::invalid("QVM input entity does not own its located public player state"))?;
    game.data.public_player_bytes(slot)?;
    if i64::from(client) != i64::try_from(expected).unwrap_or(-1) {
        return Err(GuestError::invalid(
            "QVM input entity does not own its located public player state",
        ));
    }
    usize::try_from(client)
        .map_err(|_| GuestError::invalid("QVM input entity does not own its located public player state"))
}

/// Current cancellation token for a scope's actor.
fn cancellation_token(clients: &[QvmClientScope], scope: &QvmClientScope) -> QvmCancellationScope {
    clients
        .iter()
        .find(|entry| entry.identity.actor == scope.identity.actor)
        .unwrap_or(scope)
        .cancellation
}

/// Disconnect a dead scope and cancel its current envelope.
fn check_live(
    state: &Rc<RefCell<QvmInputState>>,
    call: &mut QvmFunctionCall,
    scope: &QvmClientScope,
) -> Result<(), GuestError> {
    let deps = state.borrow().deps();
    if deps.services.live(&scope.identity) {
        return Ok(());
    }
    deps.source.retiring(scope.slot, &scope.identity);
    deps.source.disconnect(scope.slot, &scope.identity, call)?;
    let token = {
        let inner = state.borrow();
        cancellation_token(&inner.clients, scope)
    };
    let _ = token;
    call.cancel_function();
    let mut inner = state.borrow_mut();
    if !inner.cancelled.contains(&scope.id) {
        inner.cancelled.push(scope.id);
    }
    Ok(())
}

/// Aim word for an absolute aim in degrees.
fn aim_word(degrees: f32, delta: i32) -> i32 {
    let words = f64::from(degrees) * 65536.0 / 360.0;
    (words.trunc().rem_euclid(65536.0) as i32).wrapping_sub(delta)
}

/// Mutable movement-application context.
struct QvmApplyContext {
    /// Scope ordinal.
    scope_id: u64,
    /// Scope identity.
    identity: QvmClientIdentity,
    /// Wire slot.
    slot: usize,
    /// Command address.
    address: usize,
    /// Suspended overlapping bytes, if any.
    suspended: Option<Vec<u8>>,
    /// Enclosing application.
    previous: Option<QvmClientApplication>,
    /// Live application.
    application: Option<QvmClientApplication>,
    /// Projected movement-mode word.
    projected: Option<QvmProjectedType>,
    /// Whether the source invocation failed.
    failed: bool,
}

/// Apply movement outputs and project the movement mode.
fn apply_output(
    deps: &QvmInputDeps,
    state: &Rc<RefCell<QvmInputState>>,
    ctx: &mut QvmApplyContext,
    kind: QvmApplicationScope,
    input: &Q3UserCommand,
) -> Result<(), GuestError> {
    let game = deps.source.game();
    let definition = deps.source.definition();
    let output = if kind == QvmApplicationScope::MovementSlice {
        let actor = ctx.application.as_ref().map_or_else(
            || input_actor(deps, state, ctx),
            |application| Ok(application.identity.actor.clone()),
        )?;
        deps.services.client_outputs(&actor)
    } else {
        None
    };
    if let Some(mode) = output.and_then(|output| output.mode) {
        let modes = definition
            .movement_modes
            .ok_or_else(|| GuestError::invalid("Original QVM movement modes require their exact source declaration"))?;
        let address = client_pointer(deps, ctx.slot)?
            .checked_add(4)
            .ok_or_else(|| GuestError::invalid("QVM movement type escapes module memory"))?;
        let memory = game.module.memory();
        let before = memory.read_i32(address)?;
        let value = modes.word(mode);
        memory.write_i32(address, value)?;
        ctx.projected = Some(QvmProjectedType { before, value });
    }
    let base = ctx
        .application
        .as_ref()
        .map_or(input, |application| &application.command);
    let stance = output.and_then(|output| output.stance);
    let Some(effective) = apply_stance_command(base, stance) else {
        return Ok(());
    };
    let memory = game.module.memory();
    let mut bytes = memory.read_bytes(ctx.address, QVM_USER_COMMAND_BYTES)?;
    write_qvm_user_command(
        &mut bytes,
        &QvmUserCommandRecord {
            server_time: effective.server_time_ms,
            angles: effective.angle_words,
            buttons: effective.buttons,
            weapon: effective.weapon,
            forward_move: effective.forward_move,
            right_move: effective.right_move,
            up_move: effective.up_move,
        },
        game.module.abi_profile(),
        true,
    )?;
    memory.write_bytes(ctx.address, &bytes)?;
    Ok(())
}

/// Actor owning an application context without a live application.
fn input_actor(
    deps: &QvmInputDeps,
    state: &Rc<RefCell<QvmInputState>>,
    ctx: &QvmApplyContext,
) -> Result<ActorId, GuestError> {
    let inner = state.borrow();
    inner
        .clients
        .iter()
        .find(|scope| scope.id == ctx.scope_id)
        .map(|scope| scope.identity.actor.clone())
        .or_else(|| deps.services.identity(ctx.slot).map(|identity| identity.actor))
        .ok_or_else(|| GuestError::invalid("QVM movement slice lost its client identity"))
}

/// Run the source invocation between liveness checks.
fn apply_inner(
    deps: &QvmInputDeps,
    state: &Rc<RefCell<QvmInputState>>,
    call: &mut QvmFunctionCall,
    scope: &QvmClientScope,
    ctx: &mut QvmApplyContext,
    kind: QvmApplicationScope,
    input: &Q3UserCommand,
) -> Result<i32, GuestError> {
    check_live(state, call, scope)?;
    apply_output(deps, state, ctx, kind, input)?;
    let result = call.proceed();
    ctx.failed = false;
    call.effect(|| {});
    let application = ctx.application.take();
    deps.services.applications().finish(application.as_ref(), false)?;
    check_live(state, call, scope)?;
    Ok(result)
}

/// Finish an application and restore projected state.
fn finish_apply(
    deps: &QvmInputDeps,
    state: &Rc<RefCell<QvmInputState>>,
    ctx: QvmApplyContext,
) -> Result<(), GuestError> {
    let primary = deps
        .services
        .applications()
        .finish(ctx.application.as_ref(), ctx.failed)
        .err();
    let cleanup = (|| -> Result<(), GuestError> {
        if let Some(projected) = ctx.projected {
            if deps.services.live(&ctx.identity) {
                let game = deps.source.game();
                let address = client_pointer(deps, ctx.slot)?
                    .checked_add(4)
                    .ok_or_else(|| GuestError::invalid("QVM movement type escapes module memory"))?;
                let memory = game.module.memory();
                if memory.read_i32(address)? == projected.value {
                    memory.write_i32(address, projected.before)?;
                }
            }
        }
        {
            let mut inner = state.borrow_mut();
            inner.current = ctx.previous;
            if let Some(index) = inner
                .command_frames
                .iter()
                .rposition(|frame| frame.scope_id == ctx.scope_id && frame.address == ctx.address)
            {
                inner.command_frames.remove(index);
            }
        }
        if let Some(suspended) = ctx.suspended {
            deps.source
                .game()
                .module
                .memory()
                .write_bytes(ctx.address, &suspended)?;
        }
        Ok(())
    })()
    .err();
    if let Some(error) = cleanup {
        return Err(error);
    }
    if let Some(error) = primary {
        return Err(error);
    }
    Ok(())
}

/// Run one movement application for an enveloped scope.
fn apply_movement(
    state: &Rc<RefCell<QvmInputState>>,
    call: &mut QvmFunctionCall,
    scope: &QvmClientScope,
    movement: usize,
    kind: QvmApplicationScope,
) -> Result<i32, GuestError> {
    let deps = state.borrow().deps();
    let game = deps.source.game();
    let definition = deps.source.definition();
    let player = game.data.copy_player_state(scope.slot)?;
    let address = movement
        .checked_add(4)
        .ok_or_else(|| GuestError::invalid("QVM movement command escapes module memory"))?;
    let memory = game.module.memory();
    let bytes = memory.read_bytes(address, QVM_USER_COMMAND_BYTES)?;
    let command = read_qvm_user_command(&bytes, game.module.abi_profile())?;
    let input = Q3UserCommand::from(&command);
    let overlapping = state
        .borrow()
        .command_frames
        .iter()
        .any(|frame| frame.address == address && frame.scope_id != scope.id);
    let suspended = if overlapping {
        Some(memory.read_bytes(address, QVM_USER_COMMAND_BYTES)?)
    } else {
        None
    };
    {
        let mut inner = state.borrow_mut();
        inner.command_frames.push(QvmCommandFrame {
            scope_id: scope.id,
            address,
        });
    }
    let remaining = i64::from(command.server_time) - i64::from(player.command_time_ms);
    let milliseconds = if kind == QvmApplicationScope::ClientCommand {
        remaining.clamp(0, 1000)
    } else {
        remaining.clamp(1, 200)
    };
    let previous = state.borrow().current.clone();
    let aim = qvm_view_angles(
        command.angles,
        player.delta_angle_words,
        &player.view_angles,
        player.stats[0],
        player.movement_type,
        &definition.intermission,
    )
    .angles;
    let same_actor = previous
        .as_ref()
        .is_some_and(|previous| previous.identity.actor == scope.identity.actor);
    let mut frame = deps.services.frame();
    frame.phase = QvmFramePhase::ClientCommand;
    frame.elapsed = QvmSourceTime {
        kind: QvmTimeKind::Milliseconds,
        value: milliseconds as f64,
    };
    let input_state = QvmApplicationInput {
        identity: scope.identity.clone(),
        parent_invocation: previous.as_ref().map(|previous| previous.invocation),
        scope: kind,
        command: input,
        angle_space: QvmAngleSpace::SourceRelative,
        absolute_aim: aim,
        frame,
        accepted: deps.services.accepted(&scope.identity.actor),
        arsenal: if kind == QvmApplicationScope::MovementSlice && same_actor {
            previous.as_ref().and_then(|previous| previous.arsenal.clone())
        } else {
            None
        },
        impulse: if kind == QvmApplicationScope::MovementSlice && same_actor {
            previous.as_ref().map_or(0, |previous| previous.impulse)
        } else {
            0
        },
    };
    let source = Rc::clone(&deps.source);
    let slot = scope.slot;
    let mut encode_aim = |aim: &Vec3, effective: &Q3UserCommand| -> Result<Q3UserCommand, GuestError> {
        let current = source.game().data.copy_player_state(slot)?;
        Ok(Q3UserCommand {
            angle_words: [
                aim_word(aim.x, current.delta_angle_words[0]),
                aim_word(aim.y, current.delta_angle_words[1]),
                aim_word(aim.z, current.delta_angle_words[2]),
            ],
            ..*effective
        })
    };
    let application = deps.services.applications().begin(&input_state, &mut encode_aim)?;
    state.borrow_mut().current = application.clone();
    let mut ctx = QvmApplyContext {
        scope_id: scope.id,
        identity: scope.identity.clone(),
        slot: scope.slot,
        address,
        suspended,
        previous,
        application,
        projected: None,
        failed: true,
    };
    let outcome = apply_inner(&deps, state, call, scope, &mut ctx, kind, &input);
    finish_apply(&deps, state, ctx)?;
    outcome
}

/// Resolve the enveloped scope owning a movement pointer.
fn movement_scope(
    deps: &QvmInputDeps,
    state: &Rc<RefCell<QvmInputState>>,
    pointer: i32,
) -> Result<Option<QvmClientScope>, GuestError> {
    let clients: Vec<QvmClientScope> = state.borrow().clients.clone();
    for scope in clients.iter().rev() {
        let owned = client_pointer(deps, scope.slot)?;
        if usize::try_from(pointer).is_ok_and(|want| want == owned) {
            return Ok(Some(scope.clone()));
        }
    }
    Ok(None)
}

/// Run a movement hook for one scope kind.
fn run_movement(
    state: &Rc<RefCell<QvmInputState>>,
    call: &mut QvmFunctionCall,
    kind: QvmApplicationScope,
) -> Result<i32, GuestError> {
    let deps = state.borrow().deps();
    if !deps.services.applications().active() || state.borrow().clients.is_empty() {
        return Ok(call.proceed());
    }
    let movement_word = call.argument(0)?;
    let movement = usize::try_from(movement_word)
        .map_err(|_| GuestError::invalid("QVM movement pointer escapes module memory"))?;
    let memory = deps.source.game().module.memory();
    let pointer = memory.read_i32(movement)?;
    let scope = movement_scope(&deps, state, pointer)?;
    let Some(scope) = scope else {
        return Ok(call.proceed());
    };
    if state.borrow().spawning.contains(&scope.slot) {
        return Ok(call.proceed());
    }
    let state = Rc::clone(state);
    deps.source.movement(scope.slot, call, &mut |call| {
        apply_movement(&state, call, &scope, movement, kind)
    })
}

/// Envelope a client entry point.
fn envelope(state: &Rc<RefCell<QvmInputState>>, call: &mut QvmFunctionCall, slot: usize) -> Result<i32, GuestError> {
    let deps = state.borrow().deps();
    if deps.source.retired(slot) {
        return Ok(0);
    }
    if !deps.services.applications().active() || state.borrow().spawning.contains(&slot) {
        return Ok(call.proceed());
    }
    let identity = deps.services.identity(slot);
    let Some(identity) = identity else {
        return Ok(call.proceed());
    };
    if !deps.services.live(&identity) {
        return Ok(call.proceed());
    }
    client_pointer(&deps, slot)?;
    let id = {
        let mut inner = state.borrow_mut();
        let id = inner.next_scope;
        inner.next_scope += 1;
        inner.clients.push(QvmClientScope {
            id,
            slot,
            identity,
            cancellation: call.cancellation_scope(),
        });
        id
    };
    let result = call.proceed();
    state.borrow_mut().clients.retain(|scope| scope.id != id);
    Ok(result)
}

/// Run a hook closure, recording the first failure.
fn run_hook(state: &Rc<RefCell<QvmInputState>>, run: impl FnOnce() -> Result<i32, GuestError>) -> i32 {
    match run() {
        Ok(result) => result,
        Err(error) => {
            let mut inner = state.borrow_mut();
            if inner.error.is_none() {
                inner.error = Some(error);
            }
            0
        }
    }
}

/// QVM client-input binding.
pub struct QvmInputBinding {
    /// Shared state.
    state: Rc<RefCell<QvmInputState>>,
    /// Hooked module.
    module: QvmModule,
    /// Bound hook ids.
    hooks: Vec<u64>,
    /// Release remover.
    release: Option<QvmReleaseHandle>,
}

impl QvmInputBinding {
    /// Bind input entries and release notifications.
    #[must_use]
    pub fn new(source: Rc<dyn QvmInputSource>, services: Rc<dyn QvmInputServices>) -> Self {
        let module = source.game().module.clone();
        let entries = source.definition().entries;
        let state = Rc::new(RefCell::new(QvmInputState {
            source,
            services,
            clients: Vec::new(),
            spawning: Vec::new(),
            command_frames: Vec::new(),
            current: None,
            cancelled: Vec::new(),
            next_scope: 0,
            error: None,
        }));
        let mut hooks = Vec::new();
        let bind = |hooks: &mut Vec<u64>, entry: usize, hook: QvmHookFn| {
            hooks.push(module.bind_function(entry, hook));
        };
        let spawn = Rc::clone(&state);
        bind(
            &mut hooks,
            entries.client_spawn,
            Rc::new(move |call| {
                run_hook(&spawn, || {
                    let deps = spawn.borrow().deps();
                    if !deps.services.applications().active() && !deps.services.observes_spawns() {
                        return Ok(call.proceed());
                    }
                    let slot = deps.source.game().data.number_from_pointer(call.argument(0)?)?;
                    spawn.borrow_mut().spawning.push(slot);
                    let result = call.proceed();
                    spawn.borrow_mut().spawning.pop();
                    call.effect(|| {});
                    let notify = (|| -> Result<(), GuestError> {
                        if let Some(identity) = deps.services.identity(slot) {
                            if deps.services.live(&identity) {
                                deps.services.spawned(&identity);
                            }
                        }
                        Ok(())
                    })();
                    notify?;
                    Ok(result)
                })
            }),
        );
        let think = Rc::clone(&state);
        bind(
            &mut hooks,
            entries.client_think,
            Rc::new(move |call| {
                run_hook(&think, || {
                    let slot = usize::try_from(call.argument(0)?)
                        .map_err(|_| GuestError::invalid("QVM client slot is negative"))?;
                    envelope(&think, call, slot)
                })
            }),
        );
        let run = Rc::clone(&state);
        bind(
            &mut hooks,
            entries.run_client,
            Rc::new(move |call| {
                run_hook(&run, || {
                    let deps = run.borrow().deps();
                    let slot = deps.source.game().data.number_from_pointer(call.argument(0)?)?;
                    envelope(&run, call, slot)
                })
            }),
        );
        let pmove = Rc::clone(&state);
        bind(
            &mut hooks,
            entries.move_entry,
            Rc::new(move |call| {
                run_hook(&pmove, || {
                    let deps = pmove.borrow().deps();
                    let pmove = Rc::clone(&pmove);
                    deps.services
                        .movement(call, QvmApplicationScope::ClientCommand, &mut |call| {
                            run_movement(&pmove, call, QvmApplicationScope::ClientCommand)
                        })
                })
            }),
        );
        let slice = Rc::clone(&state);
        bind(
            &mut hooks,
            entries.slice,
            Rc::new(move |call| {
                run_hook(&slice, || {
                    let deps = slice.borrow().deps();
                    let slice = Rc::clone(&slice);
                    deps.services
                        .movement(call, QvmApplicationScope::MovementSlice, &mut |call| {
                            run_movement(&slice, call, QvmApplicationScope::MovementSlice)
                        })
                })
            }),
        );
        let released = Rc::clone(&state);
        let release = state.borrow().services.on_release(Rc::new(move |actor: ActorId| {
            let scopes: Vec<QvmClientScope> = released.borrow().clients.clone();
            for scope in scopes {
                if scope.identity.actor == actor {
                    released.borrow().source.retiring(scope.slot, &scope.identity);
                }
            }
        }));
        Self {
            state,
            module,
            hooks,
            release: Some(release),
        }
    }

    /// Take the first recorded hook failure, if any.
    pub fn take_error(&self) -> Option<GuestError> {
        self.state.borrow_mut().error.take()
    }

    /// Cancelled scope ordinals.
    #[must_use]
    pub fn cancelled_scopes(&self) -> Vec<u64> {
        self.state.borrow().cancelled.clone()
    }

    /// Remove hooks and release notifications in reverse bind order.
    pub fn close(&mut self) {
        for id in self.hooks.drain(..).rev() {
            self.module.remove_hook(id);
        }
        if let Some(release) = self.release.take() {
            release();
        }
    }

    /// Push a synthetic envelope scope.
    ///
    /// Test-only nesting harness: the hub's `proceed()` is a stub, so the real
    /// interpreter's think-inside-move nesting is opened directly.
    #[cfg(test)]
    fn test_push_scope(&self, slot: usize, identity: QvmClientIdentity) -> u64 {
        let mut inner = self.state.borrow_mut();
        let id = inner.next_scope;
        inner.next_scope += 1;
        inner.clients.push(QvmClientScope {
            id,
            slot,
            identity,
            cancellation: QvmCancellationScope,
        });
        id
    }

    /// Pop a synthetic envelope scope.
    #[cfg(test)]
    fn test_pop_scope(&self, id: u64) {
        self.state.borrow_mut().clients.retain(|scope| scope.id != id);
    }
}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};
    use std::collections::{HashMap, HashSet};
    use std::rc::Rc;

    use qa_core::identity::IdentityOwner;
    use qa_core::math::vec3;

    use super::super::game::QvmGame;
    use super::super::game_data::{
        ModuleIdentity, QvmArtifact, QvmFunctionCall, QvmHostFn, QvmImage, QvmRole, QvmSharedMemory,
    };
    use super::super::player_record::QvmPlayerState;
    use super::*;

    const ENTITIES: usize = 4096;
    const CLIENTS: usize = 8192;
    const ENTITY_STRIDE: usize = 516;
    const CLIENT_STRIDE: usize = 512;
    const CLIENT_POINTER: usize = 16;
    const SLOTS: usize = 4;
    const MOVE_ADDR: usize = 16384;

    fn client_ptr(slot: usize) -> i32 {
        (CLIENTS + slot * CLIENT_STRIDE) as i32
    }

    struct FixtureJournal {
        active: Cell<bool>,
        dead: Rc<RefCell<HashSet<ActorId>>>,
        next: RefCell<u64>,
        begun: RefCell<Vec<QvmApplicationInput>>,
        applications: RefCell<Vec<QvmClientApplication>>,
        finished: RefCell<Vec<(u64, bool)>>,
        remap_aim: RefCell<Option<Vec3>>,
        on_begin: RefCell<Option<Rc<dyn Fn(&QvmApplicationInput)>>>,
        probe: RefCell<Option<(QvmSharedMemory, usize)>>,
        probed: RefCell<Vec<i32>>,
    }

    impl QvmClientApplications for FixtureJournal {
        fn active(&self) -> bool {
            self.active.get()
        }

        fn begin(
            &self,
            input: &QvmApplicationInput,
            encode_aim: &mut dyn FnMut(&Vec3, &Q3UserCommand) -> Result<Q3UserCommand, GuestError>,
        ) -> Result<Option<QvmClientApplication>, GuestError> {
            self.begun.borrow_mut().push(input.clone());
            let hook = self.on_begin.borrow_mut().take();
            // Reserve the invocation before running the hook: nested begins
            // must sort after their outer frame.
            let invocation = {
                let mut next = self.next.borrow_mut();
                *next += 1;
                *next
            };
            if let Some(hook) = hook {
                hook(input);
            }
            let mut command = input.command;
            let aim = self.remap_aim.borrow_mut().take();
            if let Some(aim) = aim {
                command = encode_aim(&aim, &command)?;
            }
            if !self.active.get() || self.dead.borrow().contains(&input.identity.actor) {
                return Ok(None);
            }
            let application = QvmClientApplication {
                identity: input.identity.clone(),
                invocation,
                parent_invocation: input.parent_invocation,
                scope: input.scope,
                command,
                angle_space: input.angle_space,
                absolute_aim: input.absolute_aim.clone(),
                frame: input.frame,
                accepted: input.accepted.clone(),
                arsenal: input.arsenal.clone(),
                impulse: input.impulse,
            };
            self.applications.borrow_mut().push(application.clone());
            Ok(Some(application))
        }

        fn finish(&self, application: Option<&QvmClientApplication>, failed: bool) -> Result<(), GuestError> {
            if let Some((memory, address)) = self.probe.borrow().as_ref() {
                self.probed.borrow_mut().push(memory.read_i32(*address)?);
            }
            if let Some(application) = application {
                self.finished.borrow_mut().push((application.invocation, failed));
            }
            Ok(())
        }
    }

    struct FixtureServices {
        journal: FixtureJournal,
        identities: HashMap<usize, QvmClientIdentity>,
        dead: Rc<RefCell<HashSet<ActorId>>>,
        outputs: RefCell<HashMap<ActorId, QvmMovementOutputs>>,
        spawned: RefCell<Vec<ActorId>>,
        observes: Cell<bool>,
        release: Rc<RefCell<Option<QvmReleaseListener>>>,
        frame: QvmFrameContext,
        accepted: Option<QvmClientCommand>,
    }

    impl QvmInputServices for FixtureServices {
        fn applications(&self) -> &dyn QvmClientApplications {
            &self.journal
        }

        fn client_outputs(&self, actor: &ActorId) -> Option<QvmMovementOutputs> {
            self.outputs.borrow().get(actor).copied()
        }

        fn identity(&self, slot: usize) -> Option<QvmClientIdentity> {
            self.identities.get(&slot).cloned()
        }

        fn live(&self, identity: &QvmClientIdentity) -> bool {
            !self.dead.borrow().contains(&identity.actor)
        }

        fn accepted(&self, _actor: &ActorId) -> Option<QvmClientCommand> {
            self.accepted.clone()
        }

        fn frame(&self) -> QvmFrameContext {
            self.frame
        }

        fn spawned(&self, identity: &QvmClientIdentity) {
            self.spawned.borrow_mut().push(identity.actor.clone());
        }

        fn observes_spawns(&self) -> bool {
            self.observes.get()
        }

        fn on_release(&self, listener: QvmReleaseListener) -> QvmReleaseHandle {
            *self.release.borrow_mut() = Some(Rc::clone(&listener));
            let release = Rc::clone(&self.release);
            Box::new(move || {
                release.borrow_mut().take();
            })
        }
    }

    struct FixtureSource {
        game: QvmGame,
        definition: QvmInputDefinition,
        retiring: RefCell<Vec<usize>>,
        retired: RefCell<Vec<usize>>,
        disconnects: RefCell<Vec<usize>>,
        movements: RefCell<Vec<usize>>,
    }

    impl QvmInputSource for FixtureSource {
        fn game(&self) -> &QvmGame {
            &self.game
        }

        fn definition(&self) -> &QvmInputDefinition {
            &self.definition
        }

        fn retiring(&self, slot: usize, _identity: &QvmClientIdentity) {
            self.retiring.borrow_mut().push(slot);
        }

        fn retired(&self, slot: usize) -> bool {
            self.retired.borrow().contains(&slot)
        }

        fn disconnect(
            &self,
            slot: usize,
            _identity: &QvmClientIdentity,
            _call: &mut QvmFunctionCall,
        ) -> Result<(), GuestError> {
            self.disconnects.borrow_mut().push(slot);
            Ok(())
        }

        fn movement(
            &self,
            slot: usize,
            call: &mut QvmFunctionCall,
            run: &mut dyn FnMut(&mut QvmFunctionCall) -> Result<i32, GuestError>,
        ) -> Result<i32, GuestError> {
            self.movements.borrow_mut().push(slot);
            run(call)
        }
    }

    struct Harness {
        binding: Rc<RefCell<QvmInputBinding>>,
        source: Rc<FixtureSource>,
        services: Rc<FixtureServices>,
        identities: Vec<QvmClientIdentity>,
    }

    impl Harness {
        fn memory(&self) -> QvmSharedMemory {
            self.source.game.module.memory()
        }

        fn set_player(&self, slot: usize, command_time_ms: i32) {
            let mut player = QvmPlayerState::default();
            player.command_time_ms = command_time_ms;
            player.stats[0] = 100;
            self.source.game.data.write_player_state(slot, &player).unwrap();
        }

        fn write_command(&self, address: usize, command: &QvmUserCommandRecord) {
            let memory = self.memory();
            let mut bytes = memory.read_bytes(address, QVM_USER_COMMAND_BYTES).unwrap();
            write_qvm_user_command(&mut bytes, command, AbiProfile::Modern, false).unwrap();
            memory.write_bytes(address, &bytes).unwrap();
        }

        fn read_command(&self, address: usize) -> QvmUserCommandRecord {
            let bytes = self.memory().read_bytes(address, QVM_USER_COMMAND_BYTES).unwrap();
            read_qvm_user_command(&bytes, AbiProfile::Modern).unwrap()
        }
    }

    fn harness() -> Harness {
        let owner = IdentityOwner::create("input-test").unwrap();
        let identities: Vec<QvmClientIdentity> = (0..2)
            .map(|slot| QvmClientIdentity {
                client: owner.client(slot, 1),
                actor: owner.actor(slot, 1),
            })
            .collect();
        let mut image = QvmImage::default();
        image.allocated_data_length = 65536;
        let artifact = QvmArtifact {
            module: ModuleIdentity {
                id: "test:qagame".to_string(),
                artifact_path: "test".to_string(),
                digest: "test".to_string(),
                revision: "1".to_string(),
            },
            role: QvmRole::Qagame,
            abi_profile: None,
            image,
        };
        let host: QvmHostFn = Rc::new(|_| Ok(None));
        let game = QvmGame::new(artifact, host).unwrap();
        game.data
            .locate(ENTITIES as i32, SLOTS, ENTITY_STRIDE, CLIENTS as i32, CLIENT_STRIDE)
            .unwrap();
        for slot in 0..SLOTS {
            game.data
                .entity_bytes(slot)
                .unwrap()
                .set_i32(CLIENT_POINTER, client_ptr(slot))
                .unwrap();
            game.data.write_player_state(slot, &QvmPlayerState::default()).unwrap();
        }
        let dead = Rc::new(RefCell::new(HashSet::new()));
        let source = Rc::new(FixtureSource {
            game,
            definition: QvmInputDefinition {
                module: ModuleIdentity::default(),
                entity_stride: ENTITY_STRIDE,
                client_stride: CLIENT_STRIDE,
                client_pointer: CLIENT_POINTER,
                intermission: vec![4, 5],
                movement_modes: Some(QvmMovementModes {
                    normal: 0,
                    noclip: 1,
                    freeze: 4,
                }),
                entries: QvmInputEntries {
                    client_think: 11,
                    run_client: 23,
                    client_spawn: 37,
                    move_entry: 51,
                    slice: 67,
                },
            },
            retiring: RefCell::new(Vec::new()),
            retired: RefCell::new(Vec::new()),
            disconnects: RefCell::new(Vec::new()),
            movements: RefCell::new(Vec::new()),
        });
        let services = Rc::new(FixtureServices {
            journal: FixtureJournal {
                active: Cell::new(true),
                dead: Rc::clone(&dead),
                next: RefCell::new(0),
                begun: RefCell::new(Vec::new()),
                applications: RefCell::new(Vec::new()),
                finished: RefCell::new(Vec::new()),
                remap_aim: RefCell::new(None),
                on_begin: RefCell::new(None),
                probe: RefCell::new(None),
                probed: RefCell::new(Vec::new()),
            },
            identities: [(0, identities[0].clone()), (1, identities[1].clone())]
                .into_iter()
                .collect(),
            dead,
            outputs: RefCell::new(HashMap::new()),
            spawned: RefCell::new(Vec::new()),
            observes: Cell::new(true),
            release: Rc::new(RefCell::new(None)),
            frame: QvmFrameContext {
                frame: 7,
                time: QvmSourceTime {
                    kind: QvmTimeKind::Milliseconds,
                    value: 1000.0,
                },
                elapsed: QvmSourceTime {
                    kind: QvmTimeKind::Milliseconds,
                    value: 50.0,
                },
                phase: QvmFramePhase::FrameEntry,
            },
            accepted: None,
        });
        let binding = Rc::new(RefCell::new(QvmInputBinding::new(
            Rc::clone(&source) as Rc<dyn QvmInputSource>,
            Rc::clone(&services) as Rc<dyn QvmInputServices>,
        )));
        Harness {
            binding,
            source,
            services,
            identities,
        }
    }

    #[test]
    fn user_command_layouts_round_trip() {
        let command = QvmUserCommandRecord {
            server_time: 1000,
            angles: [11, 22, 33],
            buttons: 0x1_00FF,
            weapon: 7,
            forward_move: 127,
            right_move: -128,
            up_move: -1,
        };
        let mut modern = vec![0u8; QVM_USER_COMMAND_BYTES];
        write_qvm_user_command(&mut modern, &command, AbiProfile::Modern, false).unwrap();
        assert_eq!(read_qvm_user_command(&modern, AbiProfile::Modern).unwrap(), command);

        let mut legacy = vec![0xAAu8; QVM_USER_COMMAND_BYTES];
        write_qvm_user_command(
            &mut legacy,
            &QvmUserCommandRecord {
                buttons: 31 | 2048,
                ..command
            },
            AbiProfile::Legacy,
            true,
        )
        .unwrap();
        assert_eq!(legacy[4], 0xA0 | 31 | 128);
        let read = read_qvm_user_command(&legacy, AbiProfile::Legacy).unwrap();
        assert_eq!(read.buttons, 31 | 2048);
        assert_eq!(read.angles, command.angles);
        assert!(read_qvm_user_command(&modern[..8], AbiProfile::Modern).is_err());
    }

    #[test]
    fn view_angles_follow_pm_update_view_angles() {
        let previous = vec3(1.0, 2.0, 3.0);
        let frozen = qvm_view_angles([100, 200, 300], [1, 2, 3], &previous, 100, 4, &[4, 5]);
        assert_eq!(frozen.angles, previous);
        assert_eq!(frozen.delta_words, [1, 2, 3]);
        let dead = qvm_view_angles([100, 200, 300], [1, 2, 3], &previous, 0, 0, &[4, 5]);
        assert_eq!(dead.angles, previous);
        let dying = qvm_view_angles([100, 0, 0], [0, 0, 0], &previous, 0, 2, &[4, 5]);
        assert_ne!(dying.angles, previous);

        let clamped = qvm_view_angles([16000, 0, 0], [500, 0, 0], &previous, 100, 0, &[4, 5]);
        assert_eq!(clamped.delta_words[0], 16000 - 16000);
        assert!((clamped.angles.x - 16000.0 * 360.0 / 65536.0).abs() < 1e-3);
        let wrapped = qvm_view_angles([70000, 0, 0], [0, 0, 0], &previous, 100, 0, &[4, 5]);
        assert_eq!(wrapped.delta_words[0], 16000 - 70000);
    }

    #[test]
    fn stance_command_maps_crouch() {
        let base = Q3UserCommand {
            server_time_ms: 0,
            angle_words: [0; 3],
            buttons: 0,
            weapon: 0,
            forward_move: 0,
            right_move: 0,
            up_move: 0,
        };
        assert_eq!(apply_stance_command(&base, None), None);
        let crouched = apply_stance_command(&base, Some(true)).unwrap();
        assert_eq!(crouched.up_move, -1);
        let stay = Q3UserCommand { up_move: 5, ..base };
        assert_eq!(apply_stance_command(&stay, Some(false)), None);
        let down = Q3UserCommand { up_move: -5, ..base };
        assert_eq!(apply_stance_command(&down, Some(false)).unwrap().up_move, 0);
        let deep = Q3UserCommand { up_move: 5, ..base };
        assert_eq!(apply_stance_command(&deep, Some(true)).unwrap().up_move, -5);
    }

    #[test]
    fn retired_slots_return_zero_without_envelope() {
        let fixture = harness();
        fixture.source.retired.borrow_mut().push(0);
        let result = fixture.source.game.module.call(&[0], 11).unwrap();
        assert_eq!(result, 0);
        assert!(fixture.services.journal.begun.borrow().is_empty());
        assert!(fixture.binding.borrow().take_error().is_none());
    }

    #[test]
    fn inactive_journal_proceeds_without_envelope() {
        let fixture = harness();
        fixture.services.journal.active.set(false);
        fixture.services.observes.set(false);
        fixture.source.game.module.call(&[0], 11).unwrap();
        fixture.source.game.module.call(&[ENTITIES as i32], 23).unwrap();
        fixture.source.game.module.call(&[ENTITIES as i32], 37).unwrap();
        assert!(fixture.services.journal.begun.borrow().is_empty());
        assert!(fixture.services.spawned.borrow().is_empty());
        assert!(fixture.binding.borrow().take_error().is_none());

        fixture.services.journal.active.set(true);
        fixture.source.game.module.call(&[3], 11).unwrap();
        assert!(fixture.services.journal.begun.borrow().is_empty());
    }

    #[test]
    fn spawn_notifies_live_identities() {
        let fixture = harness();
        fixture.source.game.module.call(&[ENTITIES as i32], 37).unwrap();
        assert_eq!(
            fixture.services.spawned.borrow().as_slice(),
            &[fixture.identities[0].actor.clone()]
        );
        fixture
            .services
            .dead
            .borrow_mut()
            .insert(fixture.identities[1].actor.clone());
        fixture
            .source
            .game
            .module
            .call(&[(ENTITIES + ENTITY_STRIDE) as i32], 37)
            .unwrap();
        assert_eq!(fixture.services.spawned.borrow().len(), 1);
        assert!(fixture.binding.borrow().take_error().is_none());
    }

    fn movement_fixture() -> Harness {
        let fixture = harness();
        fixture.set_player(0, 900);
        let mut player = fixture.source.game.data.copy_player_state(0).unwrap();
        player.delta_angle_words = [10, 20, 30];
        fixture.source.game.data.write_player_state(0, &player).unwrap();
        fixture.memory().write_i32(MOVE_ADDR, client_ptr(0)).unwrap();
        fixture.write_command(
            MOVE_ADDR + 4,
            &QvmUserCommandRecord {
                server_time: 1000,
                angles: [1000, 2000, 3000],
                buttons: 3,
                weapon: 5,
                forward_move: 10,
                right_move: -10,
                up_move: 0,
            },
        );
        fixture
    }

    #[test]
    fn movement_applies_command_scope() {
        let fixture = movement_fixture();
        let scope = fixture
            .binding
            .borrow()
            .test_push_scope(0, fixture.identities[0].clone());
        let result = fixture.source.game.module.call(&[MOVE_ADDR as i32], 51).unwrap();
        fixture.binding.borrow().test_pop_scope(scope);
        assert_eq!(result, 0);
        assert_eq!(fixture.source.movements.borrow().as_slice(), &[0]);
        let begun = fixture.services.journal.begun.borrow();
        assert_eq!(begun.len(), 1);
        assert_eq!(begun[0].scope, QvmApplicationScope::ClientCommand);
        assert_eq!(begun[0].frame.phase, QvmFramePhase::ClientCommand);
        assert_eq!(begun[0].frame.elapsed.value, 100.0);
        assert_eq!(begun[0].impulse, 0);
        assert_eq!(begun[0].arsenal, None);
        assert!((begun[0].absolute_aim.x - 5.5481).abs() < 1e-3);
        assert!((begun[0].absolute_aim.y - 11.0962).abs() < 1e-3);
        assert!((begun[0].absolute_aim.z - 16.6443).abs() < 1e-3);
        assert_eq!(fixture.services.journal.finished.borrow().as_slice(), &[(1, false)]);
        assert!(fixture.binding.borrow().take_error().is_none());
        let unchanged = fixture.read_command(MOVE_ADDR + 4);
        assert_eq!(unchanged.up_move, 0);
    }

    #[test]
    fn slice_projects_mode_and_applies_stance() {
        let fixture = movement_fixture();
        let type_addr = client_ptr(0) as usize + 4;
        fixture.memory().write_i32(type_addr, 7).unwrap();
        fixture.services.outputs.borrow_mut().insert(
            fixture.identities[0].actor.clone(),
            QvmMovementOutputs {
                mode: Some(QvmMovementMode::Noclip),
                stance: Some(true),
            },
        );
        *fixture.services.journal.probe.borrow_mut() = Some((fixture.memory(), type_addr));
        let scope = fixture
            .binding
            .borrow()
            .test_push_scope(0, fixture.identities[0].clone());
        fixture.source.game.module.call(&[MOVE_ADDR as i32], 67).unwrap();
        fixture.binding.borrow().test_pop_scope(scope);
        let begun = fixture.services.journal.begun.borrow();
        assert_eq!(begun[0].scope, QvmApplicationScope::MovementSlice);
        assert_eq!(fixture.services.journal.probed.borrow().as_slice(), &[1, 1]);
        assert_eq!(fixture.memory().read_i32(type_addr).unwrap(), 7);
        assert_eq!(fixture.read_command(MOVE_ADDR + 4).up_move, -1);
        assert_eq!(fixture.services.journal.finished.borrow().as_slice(), &[(1, false)]);
        assert!(fixture.binding.borrow().take_error().is_none());
    }

    #[test]
    fn aim_mapper_rewrites_angle_words() {
        let fixture = movement_fixture();
        let mut player = QvmPlayerState::default();
        player.command_time_ms = 900;
        player.stats[0] = 100;
        player.delta_angle_words = [100, 200, 300];
        fixture.source.game.data.write_player_state(0, &player).unwrap();
        *fixture.services.journal.remap_aim.borrow_mut() = Some(vec3(90.0, 0.0, 0.0));
        let scope = fixture
            .binding
            .borrow()
            .test_push_scope(0, fixture.identities[0].clone());
        fixture.source.game.module.call(&[MOVE_ADDR as i32], 51).unwrap();
        fixture.binding.borrow().test_pop_scope(scope);
        let applications = fixture.services.journal.applications.borrow();
        assert_eq!(applications.len(), 1);
        assert_eq!(applications[0].command.angle_words, [16284, -200, -300]);
        assert!(fixture.binding.borrow().take_error().is_none());
    }

    #[test]
    fn dead_scopes_disconnect_and_cancel() {
        let fixture = movement_fixture();
        fixture
            .services
            .dead
            .borrow_mut()
            .insert(fixture.identities[0].actor.clone());
        let scope = fixture
            .binding
            .borrow()
            .test_push_scope(0, fixture.identities[0].clone());
        fixture.source.game.module.call(&[MOVE_ADDR as i32], 51).unwrap();
        fixture.binding.borrow().test_pop_scope(scope);
        assert_eq!(fixture.source.retiring.borrow().as_slice(), &[0, 0]);
        assert_eq!(fixture.source.disconnects.borrow().as_slice(), &[0, 0]);
        assert_eq!(fixture.binding.borrow().cancelled_scopes(), &[scope]);
        assert_eq!(fixture.services.journal.begun.borrow().len(), 1);
        assert!(fixture.services.journal.finished.borrow().is_empty());
        assert!(fixture.binding.borrow().take_error().is_none());
    }

    #[test]
    fn overlapping_frames_suspend_and_restore() {
        let fixture = movement_fixture();
        fixture.write_command(
            MOVE_ADDR + 4,
            &QvmUserCommandRecord {
                server_time: 1000,
                angles: [0, 0, 0],
                buttons: 0,
                weapon: 0,
                forward_move: 0,
                right_move: 0,
                up_move: -5,
            },
        );
        fixture.services.outputs.borrow_mut().insert(
            fixture.identities[0].actor.clone(),
            QvmMovementOutputs {
                mode: None,
                stance: Some(false),
            },
        );
        let binding = Rc::clone(&fixture.binding);
        let module = fixture.source.game.module.clone();
        let identity = fixture.identities[0].clone();
        let nested = Rc::new(move |_: &QvmApplicationInput| {
            let inner = binding.borrow().test_push_scope(0, identity.clone());
            module.call(&[MOVE_ADDR as i32], 67).unwrap();
            binding.borrow().test_pop_scope(inner);
        });
        *fixture.services.journal.on_begin.borrow_mut() = Some(Rc::clone(&nested) as Rc<dyn Fn(&QvmApplicationInput)>);
        let outer = fixture
            .binding
            .borrow()
            .test_push_scope(0, fixture.identities[0].clone());
        fixture.source.game.module.call(&[MOVE_ADDR as i32], 51).unwrap();
        fixture.binding.borrow().test_pop_scope(outer);
        *fixture.services.journal.on_begin.borrow_mut() = None;
        assert_eq!(
            fixture.services.journal.finished.borrow().as_slice(),
            &[(2, false), (1, false)]
        );
        assert_eq!(fixture.read_command(MOVE_ADDR + 4).up_move, -5);
        assert!(fixture.binding.borrow().take_error().is_none());
    }

    #[test]
    fn release_retires_matching_scopes() {
        let fixture = harness();
        let scope = fixture
            .binding
            .borrow()
            .test_push_scope(0, fixture.identities[0].clone());
        let other = fixture
            .binding
            .borrow()
            .test_push_scope(1, fixture.identities[1].clone());
        let listener = fixture.services.release.borrow().clone().unwrap();
        listener(fixture.identities[0].actor.clone());
        assert_eq!(fixture.source.retiring.borrow().as_slice(), &[0]);
        fixture.binding.borrow().test_pop_scope(scope);
        fixture.binding.borrow().test_pop_scope(other);
    }

    #[test]
    fn close_removes_hooks_and_release() {
        let fixture = harness();
        fixture.binding.borrow_mut().close();
        fixture.source.game.module.call(&[0], 11).unwrap();
        fixture.source.game.module.call(&[ENTITIES as i32], 37).unwrap();
        assert!(fixture.services.journal.begun.borrow().is_empty());
        assert!(fixture.services.spawned.borrow().is_empty());
        assert!(fixture.services.release.borrow().is_none());
    }

    #[test]
    fn hook_failures_surface_once() {
        let fixture = harness();
        fixture.memory().write_i32(ENTITIES + CLIENT_POINTER, 12345).unwrap();
        assert_eq!(fixture.source.game.module.call(&[0], 11).unwrap(), 0);
        assert_eq!(fixture.source.game.module.call(&[0], 11).unwrap(), 0);
        let error = fixture.binding.borrow().take_error().unwrap();
        assert!(format!("{error:?}").contains("located public player state"));
        assert!(fixture.binding.borrow().take_error().is_none());
        fixture
            .memory()
            .write_i32(ENTITIES + CLIENT_POINTER, client_ptr(0))
            .unwrap();
        fixture.source.game.module.call(&[0], 11).unwrap();
        assert!(fixture.binding.borrow().take_error().is_none());
    }
}
