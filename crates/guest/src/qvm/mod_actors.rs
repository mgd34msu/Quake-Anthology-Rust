//! QVM mod actor semantics: reaction routing plus source damage.
//!
//! Ports `src/compat/qvm/mod-actors.ts` and absorbs the callback-contract
//! types its siblings share (`src/contracts/qvm-mod-callbacks.ts`: scalar and
//! source-call values, input pointers, actor records and fields, source-actor
//! lifecycle, combat declarations, client declarations, input outputs, and
//! objective addresses), the `ModCallbackInput`/`ModRuntimeValue` shapes from
//! `src/contracts/mod-callbacks.ts`, and the reaction/damage mirrors from
//! `src/contracts/world.ts`, `src/contracts/gameplay.ts`, and
//! `src/world/gameplay/authority.ts`. Combat-call lowering reuses
//! [`super::game_combat`]; trace bytes reuse [`super::trace_record`].
//! `QvmModInputPointer`, `QvmModInputPointerBase`, and `QvmModSourceCall` are
//! imported by [`super::item_storage`] and [`super::game_weapons`]; keep their
//! shapes stable. The full `QvmModCallbackDeclaration` lives with the
//! helper-owned `mod_provider` port; this file takes a validated subset
//! ([`QvmModActorsDeclaration`]) instead.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use qa_core::identity::{ActorId, OwnedActor};
use qa_core::math::{vec3, Vec3};

use super::game_combat::{
    qvm_canonical_damage_flags, qvm_combat_words, qvm_source_damage_flags, validate_qvm_combat_call, QvmCombatCall,
    QvmCombatMass, QvmCombatTeam, QvmDamageCause, QvmDamageFlags,
};
use super::game_data::{QvmArtifact, QvmFunctionCall, QvmHookFn, QvmModule, QvmOpcode, QvmSharedMemory};
use super::trace_record::{write_qvm_trace, QvmTracePlane, QvmTraceRecord, QVM_TRACE_BYTES};
use crate::error::GuestError;

/// Callback input name (mirror of `ModCallbackInput`).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct QvmCallbackInput(pub String);

impl QvmCallbackInput {
    /// Build an input name.
    pub fn named(name: &str) -> Self {
        Self(name.to_string())
    }
}

/// Runtime callback value (mirror of `ModRuntimeValue`).
#[derive(Debug, Clone, PartialEq)]
pub enum QvmRuntimeValue {
    /// Scalar value.
    Float(f64),
    /// Actor value (`None` is the null actor).
    Actor(Option<ActorId>),
    /// Vector value.
    Vector(Vec3),
    /// String value.
    Text(String),
    /// Named callback input reference.
    Input(QvmCallbackInput),
}

/// Lowered callback inputs.
pub type QvmModInputs = HashMap<QvmCallbackInput, QvmRuntimeValue>;

/// Source scalar encoding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QvmModScalar {
    /// Signed 32-bit word.
    Int32,
    /// Binary32 word.
    Float32,
}

/// Source call return shape.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QvmModReturn {
    /// Signed 32-bit word.
    Int32,
    /// Binary32 word.
    Float32,
    /// No return value.
    Void,
}

/// Source time units.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QvmTimeUnits {
    /// Seconds.
    Seconds,
    /// Milliseconds.
    Milliseconds,
}

/// Declared source-call value.
#[derive(Debug, Clone, PartialEq)]
pub enum QvmModValue {
    /// Scalar value.
    Scalar {
        /// Encoding.
        encoding: QvmModScalar,
        /// Value.
        value: QvmRuntimeValue,
    },
    /// Vector value.
    Vector(QvmRuntimeValue),
    /// String value.
    Text(QvmRuntimeValue),
    /// Actor pointer into a record.
    Actor {
        /// Record id.
        record: String,
        /// Input holding the actor.
        input: QvmCallbackInput,
    },
    /// Admitted client slot.
    Client {
        /// Input holding the actor.
        input: QvmCallbackInput,
    },
    /// Time value.
    Time {
        /// Input holding the time.
        input: QvmCallbackInput,
        /// Units.
        units: QvmTimeUnits,
        /// Encoding.
        encoding: QvmModScalar,
    },
    /// Data address.
    Address(i32),
}

/// Declared source-call global.
#[derive(Debug, Clone, PartialEq)]
pub struct QvmModGlobal {
    /// Global address.
    pub address: usize,
    /// Value.
    pub value: QvmModValue,
}

/// Declared original source call.
#[derive(Debug, Clone, PartialEq)]
pub struct QvmModSourceCall {
    /// Function entry instruction.
    pub entry: usize,
    /// Argument values.
    pub arguments: Vec<QvmModValue>,
    /// Global values.
    pub globals: Vec<QvmModGlobal>,
    /// Return shape.
    pub returns: QvmModReturn,
}

/// Input-pointer base address.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QvmModInputPointerBase {
    /// Caller argument word.
    Argument {
        /// Argument index.
        index: usize,
    },
    /// Guest global word.
    Global {
        /// Global address.
        address: usize,
    },
}

/// Caller-storage pointer path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QvmModInputPointer {
    /// Base address.
    pub base: QvmModInputPointerBase,
    /// Indirection offsets.
    pub indirections: Vec<usize>,
    /// Final offset.
    pub offset: usize,
}

/// Resolve a caller-storage pointer against a live call.
pub fn resolve_qvm_input_pointer(pointer: &QvmModInputPointer, call: &QvmFunctionCall) -> Result<usize, GuestError> {
    let mut address = match pointer.base {
        QvmModInputPointerBase::Argument { index } => call.argument(index)?,
        QvmModInputPointerBase::Global { address } => call.guest.read_i32(address)?,
    };
    for offset in &pointer.indirections {
        address = call
            .guest
            .read_i32(address as usize + *offset)
            .map_err(|_| GuestError::invalid("QVM input pointer follows an invalid source word"))?;
    }
    Ok(address as usize + pointer.offset)
}

/// Projection direction of a canonical actor field.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QvmFieldAccess {
    /// Host-to-source only.
    ReadOnly,
    /// Shared both directions.
    ReadWrite,
}

/// Match-team value mapping.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QvmTeamValue {
    /// Source value.
    pub value: i32,
    /// Namespaced team.
    pub team: String,
}

/// Actor-field binding.
#[derive(Debug, Clone, PartialEq)]
pub enum QvmModFieldBinding {
    /// Match team.
    Team {
        /// Team values.
        values: Vec<QvmTeamValue>,
    },
    /// Match score.
    Score,
    /// Combat health.
    Health {
        /// Encoding.
        encoding: QvmModScalar,
    },
    /// Inventory count.
    Inventory {
        /// Encoding.
        encoding: QvmModScalar,
        /// Item id.
        item: String,
    },
    /// Body origin.
    Origin,
    /// Body velocity.
    Velocity,
    /// Body angles.
    Angles,
    /// Bounds minimum.
    BoundsMin,
    /// Bounds maximum.
    BoundsMax,
    /// Linked record pointer.
    Record {
        /// Record id.
        record: String,
    },
    /// Constant scalar.
    Constant {
        /// Encoding.
        encoding: QvmModScalar,
        /// Value.
        value: f64,
    },
    /// Constant vector.
    ConstantVector(Vec3),
    /// Private storage.
    Private {
        /// Byte length.
        byte_length: usize,
    },
}

/// Declared actor-record field.
#[derive(Debug, Clone, PartialEq)]
pub struct QvmModActorField {
    /// Record offset.
    pub offset: usize,
    /// Projection direction (`None` shares both directions).
    pub access: Option<QvmFieldAccess>,
    /// Binding.
    pub binding: QvmModFieldBinding,
}

/// Declared actor-record array.
#[derive(Debug, Clone, PartialEq)]
pub struct QvmModActorRecord {
    /// Record id.
    pub id: String,
    /// Array address.
    pub address: usize,
    /// Row stride.
    pub stride: usize,
    /// Row capacity.
    pub capacity: usize,
    /// Fields.
    pub fields: Vec<QvmModActorField>,
}

/// Source-actor release entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QvmSourceRelease {
    /// Release entry instruction.
    pub entry: usize,
    /// Pointer argument index.
    pub argument: usize,
}

/// Guest function-pointer reaction fields.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct QvmSourceReactionFields {
    /// Touch callback field.
    pub touch: Option<usize>,
    /// Use callback field.
    pub use_: Option<usize>,
    /// Pain callback field.
    pub pain: Option<usize>,
    /// Die callback field.
    pub die: Option<usize>,
}

/// Declared source-actor lifecycle.
#[derive(Debug, Clone, PartialEq)]
pub struct QvmModSourceActors {
    /// Allocate entry instruction.
    pub allocate: usize,
    /// Release entry.
    pub release: QvmSourceRelease,
    /// In-use flag offset.
    pub inuse: usize,
    /// Event entity-type boundary.
    pub event_entity_type: i32,
    /// Per-actor update call.
    pub update: Option<QvmModSourceCall>,
    /// Exclusive frame declaration.
    pub frame: Option<super::mod_actor_frame::QvmModActorFrameDeclaration>,
    /// Fresh-initialization store instructions.
    pub initial_stores: Vec<usize>,
    /// Reaction fields.
    pub callbacks: Option<QvmSourceReactionFields>,
}

/// Declared source combat calls.
#[derive(Debug, Clone, PartialEq)]
pub struct QvmModCombatCalls {
    /// Damage call.
    pub damage: QvmCombatCall,
    /// Touch call.
    pub touch: QvmCombatCall,
    /// Use call.
    pub use_: QvmCombatCall,
    /// Pain call.
    pub pain: QvmCombatCall,
    /// Die call.
    pub die: QvmCombatCall,
}

/// Declared combat client record.
#[derive(Debug, Clone, PartialEq)]
pub struct QvmModCombatClient {
    /// Client pointer offset.
    pub pointer: usize,
    /// Client record id.
    pub record: String,
    /// Client health offset.
    pub health: usize,
    /// Client armor offset.
    pub armor: usize,
    /// Armor protection fraction.
    pub protection: f64,
    /// Client team offset.
    pub team: usize,
}

/// Combat ABI selection.
#[derive(Debug, Clone, PartialEq)]
pub enum QvmModCombatAbi {
    /// Original `G_Damage` ABI.
    Q3GDamage,
    /// Declared combat ABI.
    Declared {
        /// Combat calls.
        calls: QvmModCombatCalls,
        /// Damage-flag masks.
        damage_flags: QvmDamageFlags,
        /// Combat mass.
        mass: QvmCombatMass,
        /// Team mappings.
        teams: Vec<QvmCombatTeam>,
    },
}

/// Declared source combat.
#[derive(Debug, Clone, PartialEq)]
pub struct QvmModCombat {
    /// Damage entry instruction.
    pub entry: usize,
    /// Health offset.
    pub health: usize,
    /// Take-damage offset.
    pub takedamage: usize,
    /// Flags offset.
    pub flags: usize,
    /// Godmode mask.
    pub godmode: i64,
    /// No-knockback mask.
    pub no_knockback: i64,
    /// Damage globals.
    pub globals: Vec<QvmModGlobal>,
    /// Client record.
    pub client: Option<QvmModCombatClient>,
    /// ABI selection.
    pub abi: QvmModCombatAbi,
}

/// Objective address: direct word or global-pointer selector.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QvmModObjectiveAddress {
    /// Direct word address.
    Direct(usize),
    /// Global-pointer selector.
    Indirect(QvmModInputPointer),
}

/// Mod client input.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum QvmModClientInput {
    /// View angles.
    ViewAngles,
    /// Attack.
    Attack,
    /// Jump.
    Jump,
    /// Impulse.
    Impulse,
    /// Forward move.
    ForwardMove,
    /// Side move.
    SideMove,
    /// Up move.
    UpMove,
}

/// Input field value.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum QvmInputFieldValue {
    /// View angles.
    ViewAngles,
    /// Scaled scalar.
    Scalar {
        /// Input.
        input: QvmModClientInput,
        /// Encoding.
        encoding: QvmModScalar,
        /// Scale divisor.
        scale: f64,
    },
}

/// Expected handler return.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct QvmInputHandlerReturn {
    /// Encoding.
    pub encoding: QvmModScalar,
    /// Value.
    pub value: f64,
}

/// Declared input output.
#[derive(Debug, Clone, PartialEq)]
pub enum QvmModInputOutput {
    /// Record field projection.
    Field {
        /// Record id.
        record: String,
        /// Field offset.
        offset: usize,
        /// Value.
        value: QvmInputFieldValue,
    },
    /// Handler invocation.
    Handler {
        /// Handler entry.
        entry: usize,
        /// Actor record id.
        record: String,
        /// Actor pointer.
        pointer: QvmModInputPointer,
        /// Consumed inputs.
        inputs: Vec<QvmModClientInput>,
        /// Expected return.
        returns: Option<QvmInputHandlerReturn>,
    },
    /// User-command capture.
    Command {
        /// Handler entry.
        entry: usize,
        /// Actor record id.
        record: String,
        /// Actor pointer.
        pointer: QvmModInputPointer,
        /// Command pointer.
        command: QvmModInputPointer,
        /// Observed inputs.
        inputs: Vec<QvmModClientInput>,
    },
}

impl QvmModInputOutput {
    /// Actor record id of handler/command outputs.
    #[must_use]
    pub fn actor_record(&self) -> Option<&str> {
        match self {
            Self::Field { .. } => None,
            Self::Handler { record, .. } | Self::Command { record, .. } => Some(record),
        }
    }
}

/// Client-input binding phase.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QvmInputPhase {
    /// Before authoritative movement.
    Before,
    /// After authoritative movement.
    After,
}

/// Declared client-input binding.
#[derive(Debug, Clone, PartialEq)]
pub struct QvmModClientInputBinding {
    /// Source calls.
    pub calls: Vec<QvmModSourceCall>,
    /// Application scope.
    pub scope: String,
    /// Binding phase.
    pub phase: QvmInputPhase,
    /// Bound outputs.
    pub outputs: Vec<QvmModInputOutput>,
}

/// Client-output body-shape endpoints.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QvmClientOutputField {
    /// Record id.
    pub record: String,
    /// Field offset.
    pub offset: usize,
}

/// Declared client output (scalar/vector storage plus body shape).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QvmModClientOutput {
    /// Scalar output word.
    Scalar(QvmClientOutputField),
    /// Vector output word.
    Vector(QvmClientOutputField),
    /// Body-shape outputs.
    BodyShape {
        /// Minimums field.
        min: QvmClientOutputField,
        /// Maximums field.
        max: QvmClientOutputField,
    },
}

/// Declared source clients.
#[derive(Debug, Clone, PartialEq)]
pub struct QvmModClients {
    /// Client outputs.
    pub outputs: Vec<QvmModClientOutput>,
    /// Maximum clients.
    pub maximum: usize,
    /// Client record ids.
    pub records: Vec<String>,
    /// Player-state record id.
    pub player_state_record: String,
    /// Admission calls.
    pub admit: Vec<QvmModSourceCall>,
    /// Userinfo calls.
    pub userinfo: Vec<QvmModSourceCall>,
    /// Disconnect calls.
    pub disconnect: Vec<QvmModSourceCall>,
    /// Frame calls.
    pub frame: Vec<QvmModSourceCall>,
    /// Input bindings.
    pub input: Vec<QvmModClientInputBinding>,
}

/// Validated declaration subset for actor semantics.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct QvmModActorsDeclaration {
    /// Engine entity record id.
    pub entity_record: Option<String>,
    /// Actor records.
    pub actor_records: Vec<QvmModActorRecord>,
    /// Source-actor lifecycle.
    pub source_actors: Option<QvmModSourceActors>,
    /// Source combat.
    pub combat: Option<QvmModCombat>,
}

/// Damage delivery.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QvmDamageDelivery {
    /// Direct damage.
    Direct,
    /// Radius damage.
    Radius,
}

/// Damage cause.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QvmModCause {
    /// Quake III cause.
    Q3 {
        /// Canonical damage flags.
        damage_flags: i32,
        /// Means of death.
        means_of_death: i32,
    },
    /// Any other cause.
    Other,
}

/// Damage attack.
#[derive(Debug, Clone, PartialEq)]
pub struct QvmModAttack {
    /// Attack time in seconds.
    pub time_seconds: f64,
    /// Attacker.
    pub attacker: Option<ActorId>,
    /// Inflictor.
    pub inflictor: Option<ActorId>,
    /// Cause.
    pub cause: QvmModCause,
    /// Movement provider.
    pub movement_provider: String,
}

/// Damage request.
#[derive(Debug, Clone, PartialEq)]
pub struct QvmModDamageRequest {
    /// Target.
    pub target: ActorId,
    /// Amount.
    pub amount: f64,
    /// Knockback.
    pub knockback: f64,
    /// Direction.
    pub direction: Vec3,
    /// Point.
    pub point: Vec3,
    /// Normal.
    pub normal: Vec3,
    /// Delivery.
    pub delivery: QvmDamageDelivery,
    /// Attack.
    pub attack: QvmModAttack,
}

/// Damage reaction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QvmDamageReaction {
    /// No reaction.
    None,
    /// Pain reaction.
    Pain,
    /// Death reaction.
    Death,
}

/// Damage outcome.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QvmDamageOutcome {
    /// Applied damage.
    pub applied_damage: i32,
    /// Reaction.
    pub reaction: QvmDamageReaction,
}

/// Regular armor state.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum QvmRegularArmor {
    /// No armor.
    None,
    /// Quake III armor.
    Q3 {
        /// Armor points.
        points: i32,
        /// Protection fraction.
        protection: f64,
    },
}

/// Powered armor state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QvmPoweredArmor {
    /// No powered armor.
    None,
}

/// Armor state.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct QvmArmorState {
    /// Regular armor.
    pub regular: QvmRegularArmor,
    /// Powered armor.
    pub powered: QvmPoweredArmor,
}

/// Combat state.
#[derive(Debug, Clone, PartialEq)]
pub struct QvmCombatState {
    /// Health.
    pub health: i32,
    /// Whether the actor takes damage.
    pub can_take_damage: bool,
    /// Armor.
    pub armor: QvmArmorState,
    /// Mass.
    pub mass: f64,
    /// Whether the actor is invulnerable.
    pub invulnerable: bool,
    /// Whether knockback is suppressed.
    pub no_knockback: bool,
    /// Team.
    pub team: Option<String>,
}

/// Stored damage change.
#[derive(Debug, Clone, PartialEq)]
pub enum QvmDamageStore {
    /// Armor change.
    Armor {
        /// Before.
        before: QvmArmorState,
        /// After.
        after: QvmArmorState,
    },
    /// Health change.
    Health {
        /// Before.
        before: i32,
        /// After.
        after: i32,
    },
    /// Source velocity change.
    SourceVelocity {
        /// Before.
        before: Vec3,
        /// After.
        after: Vec3,
        /// Movement provider.
        movement_provider: String,
    },
}

/// Source-damage observer.
pub trait QvmDamageObserver {
    /// Observe the pre-reaction result.
    fn before_reaction(&self, result: &QvmDamageOutcome);
    /// Observe a stored change.
    fn stored(&self, change: &QvmDamageStore);
}

/// Body sample.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct QvmBodySample {
    /// Origin.
    pub origin: Vec3,
    /// Velocity.
    pub velocity: Vec3,
}

/// Attack provenance.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QvmAttackProvenance {
    /// Movement provider.
    pub movement_provider: String,
}

/// Touch-contact plane.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct QvmContactPlane {
    /// Normal.
    pub normal: Vec3,
    /// Distance.
    pub distance: f32,
}

/// Touch contact.
#[derive(Debug, Clone, PartialEq)]
pub struct QvmTouchContact {
    /// Contact owner.
    pub owner: OwnedActor,
    /// Other actor.
    pub other: ActorId,
    /// Contact plane.
    pub plane: Option<QvmContactPlane>,
    /// Surface (always `None` in this ABI).
    pub surface: Option<()>,
}

/// Pain reaction.
#[derive(Debug, Clone, PartialEq)]
pub struct QvmPainReaction {
    /// Reaction owner.
    pub owner: OwnedActor,
    /// Attack.
    pub attack: Option<QvmModAttack>,
    /// Attacker.
    pub attacker: Option<ActorId>,
    /// Damage.
    pub damage: f64,
    /// Knockback.
    pub kick: f64,
}

/// Death reaction.
#[derive(Debug, Clone, PartialEq)]
pub struct QvmDeathReaction {
    /// Pain fields.
    pub pain: QvmPainReaction,
    /// Inflictor.
    pub inflictor: Option<ActorId>,
    /// Death point.
    pub point: Vec3,
}

/// Bound combat operations.
pub struct QvmCombatBinding {
    /// Read combat state.
    pub read: Rc<dyn Fn() -> Result<QvmCombatState, GuestError>>,
    /// Run source damage.
    pub damage: Rc<dyn Fn(&QvmModDamageRequest) -> Result<QvmDamageOutcome, GuestError>>,
    /// Validate an armor write.
    pub validate_armor: Rc<dyn Fn(&QvmArmorState) -> Result<(), GuestError>>,
    /// Write health.
    pub write_health: Rc<dyn Fn(i32) -> Result<(), GuestError>>,
    /// Write armor.
    pub write_armor: Rc<dyn Fn(&QvmArmorState) -> Result<(), GuestError>>,
}

/// Bound actor reactions.
pub struct QvmActorReactions {
    /// Touch reaction.
    pub touch: Rc<dyn Fn(&QvmTouchContact) -> Result<(), GuestError>>,
    /// Use reaction.
    pub use_: Rc<dyn Fn(&OwnedActor, &ActorId, &ActorId) -> Result<(), GuestError>>,
    /// Pain reaction.
    pub pain: Rc<dyn Fn(&QvmPainReaction) -> Result<(), GuestError>>,
    /// Death reaction.
    pub die: Rc<dyn Fn(&QvmDeathReaction) -> Result<(), GuestError>>,
}

/// Host services for actor semantics.
pub trait QvmModActorServices {
    /// Whether shared actor callbacks exist.
    fn callbacks_available(&self) -> bool;
    /// Observe actor releases; returns the unsubscribe handle.
    fn on_actor_release(&self, on_release: Rc<dyn Fn(&OwnedActor)>) -> Box<dyn FnOnce()>;
    /// Apply foreign damage.
    fn combat_apply(&self, request: &QvmModDamageRequest);
    /// Apply owned damage through a composer.
    fn combat_apply_observed(
        &self,
        request: &QvmModDamageRequest,
        run: &dyn Fn(&QvmModDamageRequest) -> Result<QvmDamageOutcome, GuestError>,
    ) -> Result<(), GuestError>;
    /// Run source damage under an observer.
    fn run_source_damage(
        &self,
        input: &QvmModDamageRequest,
        run: &dyn Fn(Rc<dyn QvmDamageObserver>, &QvmModDamageRequest) -> Result<QvmDamageOutcome, GuestError>,
    ) -> Result<QvmDamageOutcome, GuestError>;
    /// Bind combat operations for an actor.
    fn rebind_combat(&self, actor: &OwnedActor, binding: QvmCombatBinding);
    /// Bind actor reactions.
    fn bind_actor_callbacks(&self, actor: &OwnedActor, reactions: QvmActorReactions);
    /// Route a source use callback.
    fn source_use(
        &self,
        owner: &OwnedActor,
        other: Option<&ActorId>,
        activator: Option<&ActorId>,
        proceed: &dyn Fn(&OwnedActor, &ActorId, &ActorId) -> Result<(), GuestError>,
    ) -> Result<(), GuestError>;
    /// Route a source touch callback.
    fn source_touch(
        &self,
        contact: &QvmTouchContact,
        proceed: &dyn Fn(&QvmTouchContact) -> Result<(), GuestError>,
    ) -> Result<(), GuestError>;
    /// Route a source pain callback.
    fn source_pain(
        &self,
        reaction: &QvmPainReaction,
        proceed: &dyn Fn(&QvmPainReaction) -> Result<(), GuestError>,
    ) -> Result<(), GuestError>;
    /// Route a source death callback.
    fn source_die(
        &self,
        reaction: &QvmDeathReaction,
        proceed: &dyn Fn(&QvmDeathReaction) -> Result<(), GuestError>,
    ) -> Result<(), GuestError>;
    /// Read a body sample.
    fn body(&self, actor: &ActorId) -> Option<QvmBodySample>;
    /// Current time in seconds.
    fn time_seconds(&self) -> f64;
    /// Attack provenance for a module.
    fn damage_context(&self, module_id: &str) -> Option<QvmAttackProvenance>;
    /// Match team of an actor.
    fn match_team(&self, actor: &ActorId) -> Option<String>;
}

/// Reserved scratch span; dropping it restores the previous cursor.
pub struct QvmScratchSpan {
    /// Span address.
    pub address: usize,
    /// Cursor restore.
    restore: Option<Box<dyn FnOnce()>>,
}

impl QvmScratchSpan {
    /// Build a span with its restore handle.
    pub fn new(address: usize, restore: Box<dyn FnOnce()>) -> Self {
        Self {
            address,
            restore: Some(restore),
        }
    }
}

impl Drop for QvmScratchSpan {
    fn drop(&mut self) {
        if let Some(restore) = self.restore.take() {
            restore();
        }
    }
}

/// Provider operations for actor semantics.
pub trait QvmModActorOperations {
    /// Source pointer for an actor (`None` is the null pointer).
    fn pointer(&self, actor: Option<&ActorId>) -> Result<usize, GuestError>;
    /// Canonical actor for a pointer word.
    fn actor(&self, pointer: i32) -> Option<ActorId>;
    /// Invoke a source call.
    fn invoke(&self, call: &QvmModSourceCall, inputs: &QvmModInputs) -> Result<i32, GuestError>;
    /// Reserve a scratch span.
    fn scratch_span(&self, size: usize) -> QvmScratchSpan;
}

/// Reaction kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum ReactionKind {
    /// Touch.
    Touch,
    /// Use.
    Use,
    /// Pain.
    Pain,
    /// Die.
    Die,
}

/// Reaction kinds in resolver order.
const CALLBACK_KINDS: [ReactionKind; 4] = [
    ReactionKind::Touch,
    ReactionKind::Use,
    ReactionKind::Pain,
    ReactionKind::Die,
];

/// Original `G_Damage`-ABI combat calls.
fn legacy_calls() -> QvmModCombatCalls {
    let call = |roles: &[(&str, usize)]| QvmCombatCall {
        roles: roles
            .iter()
            .map(|(name, index)| ((*name).to_string(), *index))
            .collect(),
        extras: Vec::new(),
    };
    QvmModCombatCalls {
        damage: call(&[
            ("target", 0),
            ("inflictor", 1),
            ("attacker", 2),
            ("direction", 3),
            ("point", 4),
            ("amount", 5),
            ("flags", 6),
            ("method", 7),
        ]),
        touch: call(&[("target", 0), ("other", 1), ("trace", 2)]),
        use_: call(&[("target", 0), ("other", 1), ("activator", 2)]),
        pain: call(&[("target", 0), ("attacker", 1), ("amount", 2)]),
        die: call(&[
            ("target", 0),
            ("inflictor", 1),
            ("attacker", 2),
            ("amount", 3),
            ("method", 4),
        ]),
    }
}

/// Original damage-flag masks.
const LEGACY_FLAGS: QvmDamageFlags = QvmDamageFlags {
    radius: 1,
    no_armor: 2,
    no_knockback: 4,
    no_protection: 8,
    no_team_protection: 16,
};

/// Argument byte count of a combat call.
fn argument_bytes(call: &QvmCombatCall) -> usize {
    (call.roles.len() + call.extras.len()) * 4
}

/// Check a caller frame covers a combat call.
fn require_arguments(bytes: usize, words: &[i32]) -> Result<(), GuestError> {
    if words.len() * 4 < bytes {
        return Err(GuestError::invalid(
            "QVM combat declaration exceeds the actual source caller frame",
        ));
    }
    Ok(())
}

/// Check a semantic field offset against its record.
fn check_field(record: &QvmModActorRecord, offset: usize) -> Result<(), GuestError> {
    if offset % 4 != 0 || offset + 4 > record.stride {
        return Err(GuestError::invalid(
            "QVM actor semantic field exceeds its declared record",
        ));
    }
    Ok(())
}

/// Validate actor callbacks and combat against an artifact.
pub fn validate_qvm_mod_actors(
    artifact: &QvmArtifact,
    declaration: &QvmModActorsDeclaration,
) -> Result<(), GuestError> {
    let callbacks = declaration.source_actors.as_ref().and_then(|actors| actors.callbacks);
    let combat = declaration.combat.as_ref();
    if callbacks.is_none() && combat.is_none() {
        return Ok(());
    }
    let record = declaration
        .entity_record
        .as_deref()
        .and_then(|id| declaration.actor_records.iter().find(|record| record.id == id));
    let Some(record) = record else {
        return Err(GuestError::invalid(
            "QVM actor callbacks and combat require declared source actors",
        ));
    };
    if declaration.source_actors.is_none() {
        return Err(GuestError::invalid(
            "QVM actor callbacks and combat require declared source actors",
        ));
    }
    if let Some(callbacks) = callbacks {
        for offset in [callbacks.touch, callbacks.use_, callbacks.pain, callbacks.die]
            .into_iter()
            .flatten()
        {
            check_field(record, offset)?;
        }
    }
    let Some(combat) = combat else {
        return Ok(());
    };
    if artifact
        .image
        .instruction(combat.entry)
        .map_or(true, |instruction| instruction.opcode != QvmOpcode::OpEnter)
    {
        return Err(GuestError::invalid("QVM damage entry is not a source function"));
    }
    for offset in [combat.health, combat.takedamage, combat.flags] {
        check_field(record, offset)?;
    }
    let mask_limit = match combat.abi {
        QvmModCombatAbi::Q3GDamage => 0x7fff_ffffi64,
        QvmModCombatAbi::Declared { .. } => 0xffff_ffffi64,
    };
    for mask in [combat.godmode, combat.no_knockback] {
        if mask <= 0 || mask > mask_limit {
            return Err(GuestError::invalid("Invalid QVM combat flag mask"));
        }
    }
    if callbacks.is_none() {
        return Err(GuestError::invalid(
            "QVM source damage requires declared reaction fields",
        ));
    }
    if let QvmModCombatAbi::Declared {
        calls,
        damage_flags,
        mass,
        teams,
    } = &combat.abi
    {
        let data_bytes = artifact.image.data_end();
        for call in [&calls.damage, &calls.touch, &calls.use_, &calls.pain, &calls.die] {
            validate_qvm_combat_call(call, data_bytes)?;
        }
        let mut used: u32 = 0;
        for mask in [
            damage_flags.radius,
            damage_flags.no_armor,
            damage_flags.no_knockback,
            damage_flags.no_protection,
            damage_flags.no_team_protection,
        ] {
            let bits = mask as u32;
            if mask <= 0 || !bits.is_power_of_two() || used & bits != 0 {
                return Err(GuestError::invalid(
                    "QVM source damage flag masks overlap or are not single positive bits",
                ));
            }
            used |= bits;
        }
        match mass {
            QvmCombatMass::Entity { offset, .. } => check_field(record, *offset)?,
            QvmCombatMass::Constant { value } => {
                if !value.is_finite() || *value < 0.0 {
                    return Err(GuestError::invalid("Invalid QVM source mass"));
                }
            }
        }
        if combat.client.is_none() && !teams.is_empty() {
            return Err(GuestError::invalid(
                "QVM source team mapping requires a client team field",
            ));
        }
        let mut values = HashSet::new();
        for team in teams {
            let namespaced = team
                .team
                .split_once(':')
                .is_some_and(|(left, right)| !left.is_empty() && !right.is_empty() && !left.contains(':'));
            if !namespaced || !values.insert(team.value) {
                return Err(GuestError::invalid("Invalid QVM source team mapping"));
            }
        }
    }
    if let Some(client) = combat.client.as_ref() {
        check_field(record, client.pointer)?;
        let record = declaration
            .actor_records
            .iter()
            .find(|record| record.id == client.record);
        let Some(record) = record else {
            return Err(GuestError::invalid("QVM combat client has no declared record"));
        };
        for offset in [client.health, client.armor, client.team] {
            check_field(record, offset)?;
        }
        if !(0.0..=1.0).contains(&client.protection) || !client.protection.is_finite() {
            return Err(GuestError::invalid("Invalid QVM armor protection"));
        }
    }
    Ok(())
}

/// Combat-call argument byte counts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct CallBytes {
    /// Damage bytes.
    damage: usize,
    /// Touch bytes.
    touch: usize,
    /// Use bytes.
    use_: usize,
    /// Pain bytes.
    pain: usize,
    /// Die bytes.
    die: usize,
}

/// Open damage frame.
struct DamageFrame {
    /// Request.
    request: QvmModDamageRequest,
    /// Observer.
    observer: Rc<dyn QvmDamageObserver>,
    /// Pre-damage state.
    before: QvmCombatState,
    /// Pre-damage velocity.
    velocity: Option<Vec3>,
    /// Whether the reaction ran.
    finished: bool,
    /// Running result.
    result: QvmDamageOutcome,
}

/// Guest fields stay authoritative; common operations enter original source functions.
pub struct QvmModActors {
    /// Source module.
    module: QvmModule,
    /// Validated declaration.
    declaration: QvmModActorsDeclaration,
    /// Host services.
    services: Rc<dyn QvmModActorServices>,
    /// Provider operations.
    operations: Rc<dyn QvmModActorOperations>,
    /// Owned actors.
    owned: HashSet<ActorId>,
    /// Combat calls.
    calls: QvmModCombatCalls,
    /// Damage-flag masks.
    flags: QvmDamageFlags,
    /// Call byte counts.
    call_bytes: CallBytes,
    /// Bound hook ids.
    hooks: RefCell<Vec<u64>>,
    /// Resolver enable flag.
    resolver_enabled: Rc<Cell<bool>>,
    /// Open damage frames.
    damage_frames: RefCell<Vec<DamageFrame>>,
    /// Pointer to owner.
    pointers: RefCell<HashMap<i32, OwnedActor>>,
    /// Owner to pointer.
    actor_pointers: RefCell<HashMap<ActorId, i32>>,
    /// Release unsubscribe handle.
    release_unsub: RefCell<Option<Box<dyn FnOnce()>>>,
    /// First stashed hook error.
    error: RefCell<Option<GuestError>>,
}

impl QvmModActors {
    /// Bind actor semantics over a module.
    pub fn create(
        module: QvmModule,
        declaration: QvmModActorsDeclaration,
        services: Rc<dyn QvmModActorServices>,
        operations: Rc<dyn QvmModActorOperations>,
        owned: HashSet<ActorId>,
    ) -> Result<Rc<Self>, GuestError> {
        if !services.callbacks_available() {
            return Err(GuestError::invalid(
                "QVM actor semantics require shared actor callbacks",
            ));
        }
        let (calls, flags) = match declaration.combat.as_ref() {
            Some(combat) => match &combat.abi {
                QvmModCombatAbi::Declared {
                    calls, damage_flags, ..
                } => (calls.clone(), *damage_flags),
                QvmModCombatAbi::Q3GDamage => (legacy_calls(), LEGACY_FLAGS),
            },
            None => (legacy_calls(), LEGACY_FLAGS),
        };
        let call_bytes = CallBytes {
            damage: argument_bytes(&calls.damage),
            touch: argument_bytes(&calls.touch),
            use_: argument_bytes(&calls.use_),
            pain: argument_bytes(&calls.pain),
            die: argument_bytes(&calls.die),
        };
        let this = Rc::new(Self {
            module,
            declaration,
            services,
            operations,
            owned,
            calls,
            flags,
            call_bytes,
            hooks: RefCell::new(Vec::new()),
            resolver_enabled: Rc::new(Cell::new(true)),
            damage_frames: RefCell::new(Vec::new()),
            pointers: RefCell::new(HashMap::new()),
            actor_pointers: RefCell::new(HashMap::new()),
            release_unsub: RefCell::new(None),
            error: RefCell::new(None),
        });
        this.bind()?;
        Ok(this)
    }

    /// Install the resolver, combat hook, and release observer.
    fn bind(self: &Rc<Self>) -> Result<(), GuestError> {
        let callback = {
            let this = Rc::clone(self);
            Rc::new(move |call: &mut QvmFunctionCall| {
                let entry = call.instruction_index;
                match this.source_callback(entry, call) {
                    Ok(value) => value,
                    Err(error) => {
                        this.stash(error);
                        0
                    }
                }
            }) as QvmHookFn
        };
        {
            let this = Rc::clone(self);
            let enabled = Rc::clone(&self.resolver_enabled);
            let callback = Rc::clone(&callback);
            self.module.bind_function_resolver(Rc::new(move |entry, _first, words| {
                if !enabled.get() {
                    return None;
                }
                match this.callback(entry, words) {
                    Ok(Some(_)) => Some(Rc::clone(&callback)),
                    Ok(None) => None,
                    Err(error) => {
                        this.stash(error);
                        None
                    }
                }
            }));
        }
        if let Some(combat) = self.declaration.combat.clone() {
            let this = Rc::clone(self);
            let id = self.module.bind_function(
                combat.entry,
                Rc::new(move |call: &mut QvmFunctionCall| {
                    let request = match this.source_request(call) {
                        Ok(request) => request,
                        Err(error) => {
                            this.stash(error);
                            return 0;
                        }
                    };
                    if !this.owned.contains(&request.target) {
                        this.services.combat_apply(&request);
                        return 0;
                    }
                    let cell = RefCell::new(call);
                    let outcome = this.services.combat_apply_observed(&request, &|composed| {
                        this.damage(composed, &|effective| this.replay_damage(&cell, &request, effective))
                    });
                    if let Err(error) = outcome {
                        this.stash(error);
                    }
                    0
                }),
            );
            self.hooks.borrow_mut().push(id);
        }
        let this = Rc::clone(self);
        let unsub = self.services.on_actor_release(Rc::new(move |actor: &OwnedActor| {
            let pointer = this.actor_pointers.borrow().get(actor.id()).copied();
            if let Some(pointer) = pointer {
                if this.pointers.borrow().get(&pointer) == Some(actor) {
                    this.pointers.borrow_mut().remove(&pointer);
                }
            }
            this.actor_pointers.borrow_mut().remove(actor.id());
        }));
        *self.release_unsub.borrow_mut() = Some(unsub);
        Ok(())
    }

    /// Stash the first hook error.
    fn stash(&self, error: GuestError) {
        if self.error.borrow().is_none() {
            *self.error.borrow_mut() = Some(error);
        }
    }

    /// Take the stashed hook error, if any.
    pub fn take_error(&self) -> Option<GuestError> {
        self.error.borrow_mut().take()
    }

    /// Shared guest memory.
    fn memory(&self) -> QvmSharedMemory {
        self.module.memory()
    }

    /// Read a guest word.
    fn word(&self, address: usize) -> Result<i32, GuestError> {
        self.memory().read_i32(address)
    }

    /// Write a guest word.
    fn store(&self, address: usize, value: i32) -> Result<(), GuestError> {
        self.memory().write_i32(address, value)
    }

    /// Read a guest vector (null reads zero).
    fn vector(&self, address: usize) -> Result<Vec3, GuestError> {
        if address == 0 {
            return Ok(vec3(0.0, 0.0, 0.0));
        }
        self.memory().read_vec3(address)
    }

    /// Write a guest vector.
    fn write_vector(&self, address: usize, value: &Vec3) -> Result<(), GuestError> {
        self.memory().write_vec3(address, value)
    }

    /// Resolve a combat client record address.
    fn client(&self, pointer: usize) -> Result<Option<usize>, GuestError> {
        let Some(definition) = self
            .declaration
            .combat
            .as_ref()
            .and_then(|combat| combat.client.as_ref())
        else {
            return Ok(None);
        };
        let address = self.word(pointer + definition.pointer)?;
        if address == 0 {
            return Ok(None);
        }
        let address = address as usize;
        let record = self
            .declaration
            .actor_records
            .iter()
            .find(|record| record.id == definition.record);
        let Some(record) = record else {
            return Err(GuestError::invalid("QVM combat client has no declared record"));
        };
        if address < record.address
            || address >= record.address + record.stride * record.capacity
            || (address - record.address) % record.stride != 0
        {
            return Err(GuestError::invalid(
                "QVM combat client pointer is outside its source declaration",
            ));
        }
        Ok(Some(address))
    }

    /// Read armor state for an entity pointer.
    fn armor(&self, pointer: usize) -> Result<QvmArmorState, GuestError> {
        let client = self.client(pointer)?;
        let definition = self
            .declaration
            .combat
            .as_ref()
            .and_then(|combat| combat.client.as_ref());
        let regular = match (client, definition) {
            (Some(client), Some(definition)) => QvmRegularArmor::Q3 {
                points: self.word(client + definition.armor)?,
                protection: definition.protection,
            },
            _ => QvmRegularArmor::None,
        };
        Ok(QvmArmorState {
            regular,
            powered: QvmPoweredArmor::None,
        })
    }

    /// Read combat state for an actor.
    fn read(&self, actor: &ActorId) -> Result<QvmCombatState, GuestError> {
        let Some(definition) = self.declaration.combat.as_ref() else {
            return Err(GuestError::invalid("Missing QVM combat declaration"));
        };
        let pointer = self.operations.pointer(Some(actor))?;
        let flags = self.word(pointer + definition.flags)?;
        let client = self.client(pointer)?;
        let team_value = match (client, definition.client.as_ref()) {
            (Some(client), Some(definition)) => self.word(client + definition.team)?,
            _ => 0,
        };
        let mass = match &definition.abi {
            QvmModCombatAbi::Q3GDamage => 200.0,
            QvmModCombatAbi::Declared { mass, .. } => match mass {
                QvmCombatMass::Constant { value } => *value,
                QvmCombatMass::Entity { offset, float_storage } => {
                    if *float_storage {
                        f64::from(self.memory().read_f32(pointer + *offset)?)
                    } else {
                        f64::from(self.word(pointer + *offset)?)
                    }
                }
            },
        };
        let shared_team = self
            .declaration
            .entity_record
            .as_deref()
            .and_then(|id| self.declaration.actor_records.iter().find(|record| record.id == id))
            .is_some_and(|record| {
                record
                    .fields
                    .iter()
                    .any(|field| matches!(field.binding, QvmModFieldBinding::Team { .. }))
            });
        let team = if shared_team {
            self.services.match_team(actor)
        } else {
            match &definition.abi {
                QvmModCombatAbi::Q3GDamage => {
                    if team_value == 1 || team_value == 2 {
                        Some(format!("q3:{team_value}"))
                    } else {
                        None
                    }
                }
                QvmModCombatAbi::Declared { teams, .. } => teams
                    .iter()
                    .find(|team| team.value == team_value)
                    .map(|team| team.team.clone()),
            }
        };
        Ok(QvmCombatState {
            health: self.word(pointer + definition.health)?,
            can_take_damage: self.word(pointer + definition.takedamage)? != 0,
            armor: self.armor(pointer)?,
            mass,
            invulnerable: flags & definition.godmode as i32 != 0,
            no_knockback: flags & definition.no_knockback as i32 != 0,
            team,
        })
    }

    /// Reaction entry for an actor.
    fn entry(&self, actor: &ActorId, kind: ReactionKind) -> Result<i32, GuestError> {
        let offset = match self
            .declaration
            .source_actors
            .as_ref()
            .and_then(|actors| actors.callbacks)
        {
            Some(callbacks) => match kind {
                ReactionKind::Touch => callbacks.touch,
                ReactionKind::Use => callbacks.use_,
                ReactionKind::Pain => callbacks.pain,
                ReactionKind::Die => callbacks.die,
            },
            None => None,
        };
        let Some(offset) = offset else {
            return Ok(0);
        };
        let pointer = self.operations.pointer(Some(actor))?;
        self.word(pointer + offset)
    }

    /// Admit an actor.
    pub fn admit(self: &Rc<Self>, actor: &OwnedActor) -> Result<(), GuestError> {
        let pointer = self.operations.pointer(Some(actor.id()))?;
        self.pointers.borrow_mut().insert(pointer as i32, actor.clone());
        self.actor_pointers
            .borrow_mut()
            .insert(actor.id().clone(), pointer as i32);
        if self.declaration.combat.is_some() {
            let read_this = Rc::clone(self);
            let read_id = actor.id().clone();
            let damage_this = Rc::clone(self);
            let health_this = Rc::clone(self);
            let health_id = actor.id().clone();
            let armor_this = Rc::clone(self);
            let armor_id = actor.id().clone();
            self.services.rebind_combat(
                actor,
                QvmCombatBinding {
                    read: Rc::new(move || read_this.read(&read_id)),
                    damage: Rc::new(move |request: &QvmModDamageRequest| {
                        damage_this.damage(request, &|effective| damage_this.invoke_damage(effective))
                    }),
                    validate_armor: Rc::new(|armor: &QvmArmorState| {
                        let regular = matches!(armor.regular, QvmRegularArmor::None | QvmRegularArmor::Q3 { .. });
                        if armor.powered != QvmPoweredArmor::None || !regular {
                            return Err(GuestError::invalid("Native Q3 armor requires Q3 armor values"));
                        }
                        Ok(())
                    }),
                    write_health: Rc::new(move |health: i32| {
                        let Some(definition) = health_this.declaration.combat.as_ref() else {
                            return Err(GuestError::invalid("Missing QVM combat declaration"));
                        };
                        let pointer = health_this.operations.pointer(Some(&health_id))?;
                        health_this.store(pointer + definition.health, health)?;
                        if let Some(client) = health_this.client(pointer)? {
                            if let Some(client_definition) = definition.client.as_ref() {
                                health_this.store(client + client_definition.health, health)?;
                            }
                        }
                        Ok(())
                    }),
                    write_armor: Rc::new(move |armor: &QvmArmorState| {
                        let pointer = armor_this.operations.pointer(Some(&armor_id))?;
                        let client = armor_this.client(pointer)?;
                        let definition = armor_this.declaration.combat.as_ref();
                        let (Some(client), Some(definition), Some(client_definition)) = (
                            client,
                            definition,
                            definition.and_then(|definition| definition.client.as_ref()),
                        ) else {
                            if armor.regular != QvmRegularArmor::None || armor.powered != QvmPoweredArmor::None {
                                return Err(GuestError::invalid("QVM actor has no declared armor store"));
                            }
                            return Ok(());
                        };
                        if armor.powered != QvmPoweredArmor::None
                            || !matches!(armor.regular, QvmRegularArmor::None | QvmRegularArmor::Q3 { .. })
                        {
                            return Err(GuestError::invalid("QVM source armor requires Q3 armor values"));
                        }
                        let points = match armor.regular {
                            QvmRegularArmor::None => 0,
                            QvmRegularArmor::Q3 { points, .. } => points,
                        };
                        armor_this.store(client + client_definition.armor, points)?;
                        Ok(())
                    }),
                },
            );
        }
        let touch_this = Rc::clone(self);
        let use_this = Rc::clone(self);
        let pain_this = Rc::clone(self);
        let die_this = Rc::clone(self);
        self.services.bind_actor_callbacks(
            actor,
            QvmActorReactions {
                touch: Rc::new(move |contact: &QvmTouchContact| touch_this.touch(contact)),
                use_: Rc::new(move |owner: &OwnedActor, other: &ActorId, activator: &ActorId| {
                    let words = qvm_combat_words(
                        &use_this.calls.use_,
                        &[
                            ("target", use_this.operations.pointer(Some(owner.id()))? as i32),
                            ("other", use_this.operations.pointer(Some(other))? as i32),
                            ("activator", use_this.operations.pointer(Some(activator))? as i32),
                        ],
                    )?;
                    use_this.call_actor(owner, ReactionKind::Use, &words)
                }),
                pain: Rc::new(move |reaction: &QvmPainReaction| {
                    let words = qvm_combat_words(
                        &pain_this.calls.pain,
                        &[
                            (
                                "target",
                                pain_this.operations.pointer(Some(reaction.owner.id()))? as i32,
                            ),
                            (
                                "attacker",
                                pain_this.operations.pointer(reaction.attacker.as_ref())? as i32,
                            ),
                            ("amount", reaction.damage.trunc() as i32),
                        ],
                    )?;
                    pain_this.call_actor(&reaction.owner, ReactionKind::Pain, &words)
                }),
                die: Rc::new(move |reaction: &QvmDeathReaction| {
                    let method = match reaction.pain.attack.as_ref().map(|attack| attack.cause) {
                        Some(QvmModCause::Q3 { means_of_death, .. }) => means_of_death,
                        _ => 0,
                    };
                    let words = qvm_combat_words(
                        &die_this.calls.die,
                        &[
                            (
                                "target",
                                die_this.operations.pointer(Some(reaction.pain.owner.id()))? as i32,
                            ),
                            (
                                "inflictor",
                                die_this.operations.pointer(reaction.inflictor.as_ref())? as i32,
                            ),
                            (
                                "attacker",
                                die_this.operations.pointer(reaction.pain.attacker.as_ref())? as i32,
                            ),
                            ("amount", reaction.pain.damage.trunc() as i32),
                            ("method", method),
                        ],
                    )?;
                    die_this.call_actor(&reaction.pain.owner, ReactionKind::Die, &words)
                }),
            },
        );
        Ok(())
    }

    /// Forget all projections.
    pub fn clear_actors(&self) {
        self.pointers.borrow_mut().clear();
        self.actor_pointers.borrow_mut().clear();
    }

    /// Invoke a source call with a time input.
    fn invoke(&self, entry: usize, words: Vec<i32>, globals: &[QvmModGlobal]) -> Result<i32, GuestError> {
        let call = QvmModSourceCall {
            entry,
            arguments: words
                .into_iter()
                .map(|value| QvmModValue::Scalar {
                    encoding: QvmModScalar::Int32,
                    value: QvmRuntimeValue::Float(f64::from(value)),
                })
                .collect(),
            globals: globals.to_vec(),
            returns: QvmModReturn::Int32,
        };
        let inputs = HashMap::from([(
            QvmCallbackInput::named("time"),
            QvmRuntimeValue::Float(self.services.time_seconds()),
        )]);
        self.operations.invoke(&call, &inputs)
    }

    /// Call an actor reaction entry, if any.
    fn call_actor(&self, actor: &OwnedActor, kind: ReactionKind, words: &[i32]) -> Result<(), GuestError> {
        let entry = self.entry(actor.id(), kind)?;
        if entry != 0 {
            let globals = self
                .declaration
                .combat
                .as_ref()
                .map(|combat| combat.globals.clone())
                .unwrap_or_default();
            self.invoke(entry as usize, words.to_vec(), &globals)?;
        }
        Ok(())
    }

    /// Run a touch reaction with a scratch trace.
    fn touch(&self, contact: &QvmTouchContact) -> Result<(), GuestError> {
        let span = self.operations.scratch_span(QVM_TRACE_BYTES);
        let address = span.address;
        let normal = contact.plane.map(|plane| plane.normal).unwrap_or(vec3(0.0, 0.0, 0.0));
        let trace = QvmTraceRecord {
            all_solid: false,
            start_solid: false,
            fraction: 0.0,
            end: self
                .services
                .body(contact.owner.id())
                .map(|body| body.origin)
                .unwrap_or(vec3(0.0, 0.0, 0.0)),
            plane: QvmTracePlane {
                normal,
                distance: contact.plane.map(|plane| plane.distance).unwrap_or(0.0),
                plane_type: if normal.x == 1.0 {
                    0
                } else if normal.y == 1.0 {
                    1
                } else if normal.z == 1.0 {
                    2
                } else {
                    3
                },
                signbits: u8::from(normal.x < 0.0) | u8::from(normal.y < 0.0) * 2 | u8::from(normal.z < 0.0) * 4,
            },
            surface_flags: 0,
            contents: 0,
            entity_num: 1023,
        };
        let mut bytes = vec![0u8; QVM_TRACE_BYTES];
        write_qvm_trace(&mut bytes, &trace)?;
        self.memory().write_bytes(address, &bytes)?;
        let words = qvm_combat_words(
            &self.calls.touch,
            &[
                ("target", self.operations.pointer(Some(contact.owner.id()))? as i32),
                ("other", self.operations.pointer(Some(&contact.other))? as i32),
                ("trace", address as i32),
            ],
        )?;
        self.call_actor(&contact.owner, ReactionKind::Touch, &words)
    }

    /// Match a callback entry against reaction signatures.
    fn callback(&self, entry: usize, words: &[i32]) -> Result<Option<(OwnedActor, ReactionKind)>, GuestError> {
        let mut matched = None;
        for kind in CALLBACK_KINDS {
            let definition = match kind {
                ReactionKind::Touch => &self.calls.touch,
                ReactionKind::Use => &self.calls.use_,
                ReactionKind::Pain => &self.calls.pain,
                ReactionKind::Die => &self.calls.die,
            };
            let target = definition.role("target")?;
            let Some(word) = words.get(target) else {
                continue;
            };
            let Some(owner) = self.pointers.borrow().get(word).cloned() else {
                continue;
            };
            if self.entry(owner.id(), kind)? != entry as i32 {
                continue;
            }
            let bytes = match kind {
                ReactionKind::Touch => self.call_bytes.touch,
                ReactionKind::Use => self.call_bytes.use_,
                ReactionKind::Pain => self.call_bytes.pain,
                ReactionKind::Die => self.call_bytes.die,
            };
            require_arguments(bytes, words)?;
            if matched.is_some() {
                return Err(GuestError::invalid(
                    "QVM callback entry has ambiguous source signatures",
                ));
            }
            matched = Some((owner, kind));
        }
        Ok(matched)
    }

    /// Proceed with edited argument words, restoring them afterwards.
    fn proceed(&self, cell: &RefCell<&mut QvmFunctionCall>, edits: &[(usize, i32)]) -> i32 {
        let saved: Vec<(usize, i32)> = {
            let call = cell.borrow();
            edits
                .iter()
                .map(|(index, _)| (*index, call.words.get(*index).copied().unwrap_or(0)))
                .collect()
        };
        let result = {
            let mut call = cell.borrow_mut();
            for (index, word) in edits {
                if let Some(slot) = call.words.get_mut(*index) {
                    *slot = *word;
                }
            }
            call.proceed()
        };
        let mut call = cell.borrow_mut();
        for (index, word) in saved {
            if let Some(slot) = call.words.get_mut(index) {
                *slot = word;
            }
        }
        result
    }

    /// Read a call word.
    fn call_word(cell: &RefCell<&mut QvmFunctionCall>, index: usize) -> Result<i32, GuestError> {
        cell.borrow()
            .words
            .get(index)
            .copied()
            .ok_or_else(|| GuestError::invalid("QVM callback word is outside its caller frame"))
    }

    /// Route a source reaction callback.
    fn source_callback(&self, entry: usize, call: &mut QvmFunctionCall) -> Result<i32, GuestError> {
        let words = call.words.clone();
        let Some((owner, kind)) = self.callback(entry, &words)? else {
            return Ok(call.proceed());
        };
        let cell = RefCell::new(call);
        let actor_at =
            |index: usize| -> Result<Option<ActorId>, GuestError> { Ok(self.actor(Self::call_word(&cell, index)?)) };
        if kind == ReactionKind::Use {
            let roles = &self.calls.use_;
            let target = roles.role("target")?;
            let other_role = roles.role("other")?;
            let activator_role = roles.role("activator")?;
            let other = actor_at(other_role)?;
            let activator = actor_at(activator_role)?;
            let owner_id = owner.id().clone();
            self.services.source_use(
                &owner,
                other.as_ref(),
                activator.as_ref(),
                &|owner, effective_other, effective_activator| {
                    let edits = [
                        (
                            target,
                            if owner.id() == &owner_id {
                                Self::call_word(&cell, target)?
                            } else {
                                self.operations.pointer(Some(owner.id()))? as i32
                            },
                        ),
                        (
                            other_role,
                            if Some(effective_other) == other.as_ref() {
                                Self::call_word(&cell, other_role)?
                            } else {
                                self.operations.pointer(Some(effective_other))? as i32
                            },
                        ),
                        (
                            activator_role,
                            if Some(effective_activator) == activator.as_ref() {
                                Self::call_word(&cell, activator_role)?
                            } else {
                                self.operations.pointer(Some(effective_activator))? as i32
                            },
                        ),
                    ];
                    self.proceed(&cell, &edits);
                    Ok(())
                },
            )?;
            return Ok(0);
        }
        if kind == ReactionKind::Touch {
            let roles = &self.calls.touch;
            let target = roles.role("target")?;
            let other_role = roles.role("other")?;
            let trace_role = roles.role("trace")?;
            let Some(other) = actor_at(other_role)? else {
                return Ok(cell.borrow_mut().proceed());
            };
            let trace = Self::call_word(&cell, trace_role)?;
            let contact = QvmTouchContact {
                owner: owner.clone(),
                other: other.clone(),
                plane: if trace == 0 {
                    None
                } else {
                    Some(QvmContactPlane {
                        normal: self.vector(trace as usize + 24)?,
                        distance: self.memory().read_f32(trace as usize + 36)?,
                    })
                },
                surface: None,
            };
            let owner_id = owner.id().clone();
            self.services.source_touch(&contact, &|effective| {
                if effective.plane != contact.plane || effective.surface != contact.surface {
                    return Err(GuestError::invalid(
                        "QVM source trace changes require callback replacement",
                    ));
                }
                let edits = [
                    (
                        target,
                        if effective.owner.id() == &owner_id {
                            Self::call_word(&cell, target)?
                        } else {
                            self.operations.pointer(Some(effective.owner.id()))? as i32
                        },
                    ),
                    (
                        other_role,
                        if effective.other == other {
                            Self::call_word(&cell, other_role)?
                        } else {
                            self.operations.pointer(Some(&effective.other))? as i32
                        },
                    ),
                ];
                self.proceed(&cell, &edits);
                Ok(())
            })?;
            return Ok(0);
        }
        let frame_index = self
            .damage_frames
            .borrow()
            .iter()
            .rposition(|frame| frame.request.target == *owner.id());
        let roles = if kind == ReactionKind::Pain {
            &self.calls.pain
        } else {
            &self.calls.die
        };
        let target = roles.role("target")?;
        let attacker_role = roles.role("attacker")?;
        let amount_role = roles.role("amount")?;
        let amount = Self::call_word(&cell, amount_role)?;
        let (attack, kick, point) = if let Some(index) = frame_index {
            let mut frames = self.damage_frames.borrow_mut();
            let finished = frames[index].finished;
            if !finished {
                Self::flush_frame(self, &mut frames, index)?;
                frames[index].finished = true;
                frames[index].result = QvmDamageOutcome {
                    applied_damage: amount,
                    reaction: if kind == ReactionKind::Die {
                        QvmDamageReaction::Death
                    } else {
                        QvmDamageReaction::Pain
                    },
                };
                let result = frames[index].result;
                frames[index].observer.before_reaction(&result);
            }
            let frame = &frames[index];
            (
                Some(frame.request.attack.clone()),
                frame.request.knockback,
                frame.request.point,
            )
        } else {
            (None, 0.0, vec3(0.0, 0.0, 0.0))
        };
        let reaction = QvmPainReaction {
            owner: owner.clone(),
            attack,
            attacker: actor_at(attacker_role)?,
            damage: f64::from(amount),
            kick,
        };
        let owner_id = owner.id().clone();
        if kind == ReactionKind::Pain {
            self.services.source_pain(&reaction, &|effective| {
                let edits = [
                    (
                        target,
                        if effective.owner.id() == &owner_id {
                            Self::call_word(&cell, target)?
                        } else {
                            self.operations.pointer(Some(effective.owner.id()))? as i32
                        },
                    ),
                    (
                        attacker_role,
                        if effective.attacker == reaction.attacker {
                            Self::call_word(&cell, attacker_role)?
                        } else {
                            self.operations.pointer(effective.attacker.as_ref())? as i32
                        },
                    ),
                    (amount_role, effective.damage.trunc() as i32),
                ];
                self.proceed(&cell, &edits);
                Ok(())
            })?;
        } else {
            let death_roles = &self.calls.die;
            let inflictor_role = death_roles.role("inflictor")?;
            let death = QvmDeathReaction {
                pain: reaction.clone(),
                inflictor: actor_at(inflictor_role)?,
                point,
            };
            self.services.source_die(&death, &|effective| {
                let edits = [
                    (
                        target,
                        if effective.pain.owner.id() == &owner_id {
                            Self::call_word(&cell, target)?
                        } else {
                            self.operations.pointer(Some(effective.pain.owner.id()))? as i32
                        },
                    ),
                    (
                        inflictor_role,
                        if effective.inflictor == death.inflictor {
                            Self::call_word(&cell, inflictor_role)?
                        } else {
                            self.operations.pointer(effective.inflictor.as_ref())? as i32
                        },
                    ),
                    (
                        attacker_role,
                        if effective.pain.attacker == death.pain.attacker {
                            Self::call_word(&cell, attacker_role)?
                        } else {
                            self.operations.pointer(effective.pain.attacker.as_ref())? as i32
                        },
                    ),
                    (amount_role, effective.pain.damage.trunc() as i32),
                ];
                self.proceed(&cell, &edits);
                Ok(())
            })?;
        }
        Ok(0)
    }

    /// Canonical actor for a pointer word.
    fn actor(&self, pointer: i32) -> Option<ActorId> {
        self.operations.actor(pointer)
    }

    /// Lower a source damage call to a request.
    fn source_request(&self, call: &QvmFunctionCall) -> Result<QvmModDamageRequest, GuestError> {
        require_arguments(self.call_bytes.damage, &call.words)?;
        let roles = &self.calls.damage;
        let word = |index: usize| -> Result<i32, GuestError> {
            call.words
                .get(index)
                .copied()
                .ok_or_else(|| GuestError::invalid("QVM damage word is outside its caller frame"))
        };
        let target = roles.role("target")?;
        let inflictor = roles.role("inflictor")?;
        let attacker = roles.role("attacker")?;
        let direction = roles.role("direction")?;
        let point = roles.role("point")?;
        let amount = roles.role("amount")?;
        let flags_role = roles.role("flags")?;
        let method = roles.role("method")?;
        let Some(target) = self.actor(word(target)?) else {
            return Err(GuestError::invalid("QVM damage target is not a canonical actor"));
        };
        let module_id = self.module.module_id().id;
        let Some(context) = self.services.damage_context(&module_id) else {
            return Err(GuestError::invalid(
                "QVM source damage requires canonical attack provenance",
            ));
        };
        let flags = word(flags_role)?;
        let amount = word(amount)?;
        let direction_word = word(direction)?;
        Ok(QvmModDamageRequest {
            target,
            amount: f64::from(amount),
            knockback: if flags & self.flags.no_knockback != 0 || direction_word == 0 {
                0.0
            } else {
                f64::from(amount)
            },
            direction: self.vector(direction_word as usize)?,
            point: self.vector(word(point)? as usize)?,
            normal: vec3(0.0, 0.0, 0.0),
            delivery: if flags & self.flags.radius == 0 {
                QvmDamageDelivery::Direct
            } else {
                QvmDamageDelivery::Radius
            },
            attack: QvmModAttack {
                time_seconds: self.services.time_seconds(),
                attacker: self.actor(word(attacker)?),
                inflictor: self.actor(word(inflictor)?),
                cause: QvmModCause::Q3 {
                    damage_flags: qvm_canonical_damage_flags(&self.flags, flags),
                    means_of_death: word(method)?,
                },
                movement_provider: context.movement_provider,
            },
        })
    }

    /// Replay damage with an effective request.
    fn replay_damage(
        &self,
        cell: &RefCell<&mut QvmFunctionCall>,
        original: &QvmModDamageRequest,
        effective: &QvmModDamageRequest,
    ) -> Result<i32, GuestError> {
        if original == effective {
            return Ok(cell.borrow_mut().proceed());
        }
        let roles = &self.calls.damage;
        let span = self.operations.scratch_span(24);
        let address = span.address;
        let flags = roles.role("flags")?;
        let original_flags = Self::call_word(cell, flags)?;
        let words = self.damage_words(effective, address, original_flags)?;
        let mut edits = Vec::new();
        for (name, index) in &roles.roles {
            if (name == "direction" && effective.direction == original.direction)
                || (name == "point" && effective.point == original.point)
                || (name == "attacker" && effective.attack.attacker == original.attack.attacker)
                || (name == "inflictor" && effective.attack.inflictor == original.attack.inflictor)
            {
                continue;
            }
            let Some(word) = words.get(*index).copied() else {
                return Err(GuestError::invalid("Missing declared QVM damage argument"));
            };
            edits.push((*index, word));
        }
        Ok(self.proceed(cell, &edits))
    }

    /// Flush one damage frame, publishing stored changes.
    fn flush_frame(this: &Self, frames: &mut [DamageFrame], index: usize) -> Result<(), GuestError> {
        if frames[index].finished {
            return Ok(());
        }
        let target = frames[index].request.target.clone();
        let before = frames[index].before.clone();
        let velocity = frames[index].velocity;
        let after = this.read(&target)?;
        let current_velocity = this.services.body(&target).map(|body| body.velocity);
        for active in frames.iter_mut() {
            if active.request.target == target {
                active.before = after.clone();
                active.velocity = current_velocity;
            }
        }
        let observer = Rc::clone(&frames[index].observer);
        if let (
            QvmRegularArmor::Q3 {
                points: before_points, ..
            },
            QvmRegularArmor::Q3 {
                points: after_points, ..
            },
        ) = (before.armor.regular, after.armor.regular)
        {
            if before_points != after_points {
                observer.stored(&QvmDamageStore::Armor {
                    before: before.armor,
                    after: after.armor,
                });
            }
        }
        if before.health != after.health {
            observer.stored(&QvmDamageStore::Health {
                before: before.health,
                after: after.health,
            });
        }
        if let (Some(velocity), Some(current)) = (velocity, current_velocity) {
            if velocity != current {
                observer.stored(&QvmDamageStore::SourceVelocity {
                    before: velocity,
                    after: current,
                    movement_provider: frames[index].request.attack.movement_provider.clone(),
                });
            }
        }
        let applied = frames[index].result.applied_damage + before.health - after.health;
        frames[index].result = QvmDamageOutcome {
            applied_damage: applied,
            reaction: QvmDamageReaction::None,
        };
        Ok(())
    }

    /// Run source damage for a request.
    fn damage(
        &self,
        input: &QvmModDamageRequest,
        execute: &dyn Fn(&QvmModDamageRequest) -> Result<i32, GuestError>,
    ) -> Result<QvmDamageOutcome, GuestError> {
        let mut frames = self.damage_frames.borrow_mut();
        let targets: Vec<usize> = frames
            .iter()
            .enumerate()
            .filter(|(_, frame)| frame.request.target == input.target)
            .map(|(index, _)| index)
            .collect();
        for index in targets.into_iter().rev() {
            Self::flush_frame(self, &mut frames, index)?;
        }
        drop(frames);
        self.services.run_source_damage(input, &|observer, request| {
            let mut frames = self.damage_frames.borrow_mut();
            let index = frames.len();
            frames.push(DamageFrame {
                request: request.clone(),
                observer,
                before: self.read(&request.target)?,
                velocity: self.services.body(&request.target).map(|body| body.velocity),
                finished: false,
                result: QvmDamageOutcome {
                    applied_damage: 0,
                    reaction: QvmDamageReaction::None,
                },
            });
            drop(frames);
            let outcome = (|| -> Result<QvmDamageOutcome, GuestError> {
                execute(request)?;
                let mut frames = self.damage_frames.borrow_mut();
                Self::flush_frame(self, &mut frames, index)?;
                Ok(frames[index].result)
            })();
            self.damage_frames.borrow_mut().pop();
            outcome
        })
    }

    /// Lower a damage request to source words.
    fn damage_words(
        &self,
        request: &QvmModDamageRequest,
        address: usize,
        original_flags: i32,
    ) -> Result<Vec<i32>, GuestError> {
        self.write_vector(address, &request.direction)?;
        self.write_vector(address + 12, &request.point)?;
        let cause = match request.attack.cause {
            QvmModCause::Q3 { means_of_death, .. } => means_of_death,
            QvmModCause::Other => 0,
        };
        let lowered = super::game_combat::QvmDamageRequest {
            cause: match request.attack.cause {
                QvmModCause::Q3 { damage_flags, .. } => QvmDamageCause::Q3 { damage_flags },
                QvmModCause::Other => QvmDamageCause::Environment,
            },
            radius_delivery: request.delivery == QvmDamageDelivery::Radius,
        };
        qvm_combat_words(
            &self.calls.damage,
            &[
                ("target", self.operations.pointer(Some(&request.target))? as i32),
                (
                    "inflictor",
                    self.operations.pointer(request.attack.inflictor.as_ref())? as i32,
                ),
                (
                    "attacker",
                    self.operations.pointer(request.attack.attacker.as_ref())? as i32,
                ),
                ("direction", address as i32),
                ("point", address as i32 + 12),
                ("amount", request.amount.trunc() as i32),
                ("flags", qvm_source_damage_flags(&self.flags, &lowered, original_flags)),
                ("method", cause),
            ],
        )
    }

    /// Invoke the damage entry for a request.
    fn invoke_damage(&self, request: &QvmModDamageRequest) -> Result<i32, GuestError> {
        let Some(definition) = self.declaration.combat.clone() else {
            return Err(GuestError::invalid("Missing QVM combat declaration"));
        };
        let span = self.operations.scratch_span(24);
        let words = self.damage_words(request, span.address, 0)?;
        self.invoke(definition.entry, words, &definition.globals)
    }

    /// Flush open frames before an actor release.
    pub fn before_release(&self, actor: &ActorId) -> Result<(), GuestError> {
        let mut frames = self.damage_frames.borrow_mut();
        let targets: Vec<usize> = frames
            .iter()
            .enumerate()
            .filter(|(_, frame)| frame.request.target == *actor)
            .map(|(index, _)| index)
            .collect();
        for index in targets.into_iter().rev() {
            Self::flush_frame(self, &mut frames, index)?;
            frames[index].finished = true;
        }
        Ok(())
    }

    /// Release bindings and projections.
    pub fn close(&self) {
        self.resolver_enabled.set(false);
        for id in self.hooks.borrow_mut().drain(..) {
            self.module.remove_hook(id);
        }
        if let Some(unsub) = self.release_unsub.borrow_mut().take() {
            unsub();
        }
        self.clear_actors();
    }
}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};
    use std::collections::{HashMap, HashSet};
    use std::rc::Rc;

    use qa_core::identity::{IdentityOwner, ProviderId};

    use super::super::game_data::{
        AbiProfile, ModuleIdentity, QvmArtifact, QvmImage, QvmInstruction, QvmOpcode, QvmRole,
    };
    use super::super::trace_record::read_qvm_trace;
    use super::*;

    const ENTITIES: usize = 4096;
    const CLIENTS: usize = 8192;

    struct FixtureObserver {
        before: RefCell<Vec<QvmDamageOutcome>>,
        stored: RefCell<Vec<QvmDamageStore>>,
    }

    impl QvmDamageObserver for FixtureObserver {
        fn before_reaction(&self, result: &QvmDamageOutcome) {
            self.before.borrow_mut().push(*result);
        }

        fn stored(&self, change: &QvmDamageStore) {
            self.stored.borrow_mut().push(change.clone());
        }
    }

    struct FixtureOperations {
        pointers: HashMap<ActorId, usize>,
        actors: HashMap<i32, ActorId>,
        invoked: RefCell<Vec<(usize, Vec<i32>)>>,
    }

    impl QvmModActorOperations for FixtureOperations {
        fn pointer(&self, actor: Option<&ActorId>) -> Result<usize, GuestError> {
            actor.and_then(|actor| self.pointers.get(actor).copied()).map_or_else(
                || {
                    if actor.is_none() {
                        Ok(0)
                    } else {
                        Err(GuestError::invalid("stale actor"))
                    }
                },
                Ok,
            )
        }

        fn actor(&self, pointer: i32) -> Option<ActorId> {
            self.actors.get(&pointer).cloned()
        }

        fn invoke(&self, call: &QvmModSourceCall, _inputs: &QvmModInputs) -> Result<i32, GuestError> {
            let words = call
                .arguments
                .iter()
                .map(|value| match value {
                    QvmModValue::Scalar {
                        value: QvmRuntimeValue::Float(value),
                        ..
                    } => Ok(*value as i32),
                    _ => Err(GuestError::invalid("unexpected test value")),
                })
                .collect::<Result<Vec<_>, _>>()?;
            self.invoked.borrow_mut().push((call.entry, words));
            Ok(0)
        }

        fn scratch_span(&self, _size: usize) -> QvmScratchSpan {
            QvmScratchSpan::new(32768, Box::new(|| {}))
        }
    }

    struct FixtureServices {
        applied: RefCell<Vec<QvmModDamageRequest>>,
        observed: RefCell<Vec<QvmModDamageRequest>>,
        bindings: RefCell<HashMap<ActorId, Rc<QvmCombatBinding>>>,
        reactions: RefCell<HashMap<ActorId, Rc<QvmActorReactions>>>,
        observers: RefCell<Vec<Rc<FixtureObserver>>>,
        release: RefCell<Option<Rc<dyn Fn(&OwnedActor)>>>,
        unsubscribed: Rc<Cell<bool>>,
        bodies: HashMap<ActorId, QvmBodySample>,
        teams: HashMap<ActorId, String>,
        uses: RefCell<Vec<(ActorId, Option<ActorId>, Option<ActorId>)>>,
        touches: RefCell<Vec<QvmTouchContact>>,
        pains: RefCell<Vec<QvmPainReaction>>,
        deaths: RefCell<Vec<QvmDeathReaction>>,
    }

    impl QvmModActorServices for FixtureServices {
        fn callbacks_available(&self) -> bool {
            true
        }

        fn on_actor_release(&self, on_release: Rc<dyn Fn(&OwnedActor)>) -> Box<dyn FnOnce()> {
            *self.release.borrow_mut() = Some(on_release);
            let unsubscribed = Rc::clone(&self.unsubscribed);
            Box::new(move || unsubscribed.set(true))
        }

        fn combat_apply(&self, request: &QvmModDamageRequest) {
            self.applied.borrow_mut().push(request.clone());
        }

        fn combat_apply_observed(
            &self,
            request: &QvmModDamageRequest,
            run: &dyn Fn(&QvmModDamageRequest) -> Result<QvmDamageOutcome, GuestError>,
        ) -> Result<(), GuestError> {
            self.observed.borrow_mut().push(request.clone());
            run(request)?;
            Ok(())
        }

        fn run_source_damage(
            &self,
            input: &QvmModDamageRequest,
            run: &dyn Fn(Rc<dyn QvmDamageObserver>, &QvmModDamageRequest) -> Result<QvmDamageOutcome, GuestError>,
        ) -> Result<QvmDamageOutcome, GuestError> {
            let observer = Rc::new(FixtureObserver {
                before: RefCell::new(Vec::new()),
                stored: RefCell::new(Vec::new()),
            });
            self.observers.borrow_mut().push(Rc::clone(&observer));
            run(observer, input)
        }

        fn rebind_combat(&self, actor: &OwnedActor, binding: QvmCombatBinding) {
            self.bindings.borrow_mut().insert(actor.id().clone(), Rc::new(binding));
        }

        fn bind_actor_callbacks(&self, actor: &OwnedActor, reactions: QvmActorReactions) {
            self.reactions
                .borrow_mut()
                .insert(actor.id().clone(), Rc::new(reactions));
        }

        fn source_use(
            &self,
            owner: &OwnedActor,
            other: Option<&ActorId>,
            activator: Option<&ActorId>,
            proceed: &dyn Fn(&OwnedActor, &ActorId, &ActorId) -> Result<(), GuestError>,
        ) -> Result<(), GuestError> {
            self.uses
                .borrow_mut()
                .push((owner.id().clone(), other.cloned(), activator.cloned()));
            if let (Some(other), Some(activator)) = (other, activator) {
                proceed(owner, other, activator)?;
            }
            Ok(())
        }

        fn source_touch(
            &self,
            contact: &QvmTouchContact,
            proceed: &dyn Fn(&QvmTouchContact) -> Result<(), GuestError>,
        ) -> Result<(), GuestError> {
            self.touches.borrow_mut().push(contact.clone());
            proceed(contact)
        }

        fn source_pain(
            &self,
            reaction: &QvmPainReaction,
            proceed: &dyn Fn(&QvmPainReaction) -> Result<(), GuestError>,
        ) -> Result<(), GuestError> {
            self.pains.borrow_mut().push(reaction.clone());
            proceed(reaction)
        }

        fn source_die(
            &self,
            reaction: &QvmDeathReaction,
            proceed: &dyn Fn(&QvmDeathReaction) -> Result<(), GuestError>,
        ) -> Result<(), GuestError> {
            self.deaths.borrow_mut().push(reaction.clone());
            proceed(reaction)
        }

        fn body(&self, actor: &ActorId) -> Option<QvmBodySample> {
            self.bodies.get(actor).copied()
        }

        fn time_seconds(&self) -> f64 {
            1.5
        }

        fn damage_context(&self, _module_id: &str) -> Option<QvmAttackProvenance> {
            Some(QvmAttackProvenance {
                movement_provider: "test-move".to_string(),
            })
        }

        fn match_team(&self, actor: &ActorId) -> Option<String> {
            self.teams.get(actor).cloned()
        }
    }

    struct Fixture {
        actors: Rc<QvmModActors>,
        module: QvmModule,
        operations: Rc<FixtureOperations>,
        services: Rc<FixtureServices>,
        owned: OwnedActor,
        other: ActorId,
    }

    fn artifact() -> QvmArtifact {
        let mut image = QvmImage::default();
        image.instructions = (0..32)
            .map(|index| QvmInstruction::word(QvmOpcode::OpEnter, 0, index * 8))
            .collect();
        image.data_length = 4096;
        image.allocated_data_length = 65536;
        QvmArtifact {
            module: ModuleIdentity {
                id: "test:qagame".to_string(),
                artifact_path: "test".to_string(),
                digest: "test".to_string(),
                revision: "1".to_string(),
            },
            role: QvmRole::Qagame,
            abi_profile: Some(AbiProfile::Modern),
            image,
        }
    }

    fn declaration() -> QvmModActorsDeclaration {
        QvmModActorsDeclaration {
            entity_record: Some("entity".to_string()),
            actor_records: vec![
                QvmModActorRecord {
                    id: "entity".to_string(),
                    address: ENTITIES,
                    stride: 512,
                    capacity: 4,
                    fields: Vec::new(),
                },
                QvmModActorRecord {
                    id: "client".to_string(),
                    address: CLIENTS,
                    stride: 512,
                    capacity: 4,
                    fields: Vec::new(),
                },
            ],
            source_actors: Some(QvmModSourceActors {
                allocate: 1,
                release: QvmSourceRelease { entry: 2, argument: 0 },
                inuse: 208,
                event_entity_type: 7,
                update: None,
                frame: None,
                initial_stores: Vec::new(),
                callbacks: Some(QvmSourceReactionFields {
                    touch: Some(248),
                    use_: Some(252),
                    pain: Some(256),
                    die: Some(260),
                }),
            }),
            combat: Some(QvmModCombat {
                entry: 5,
                health: 224,
                takedamage: 228,
                flags: 232,
                godmode: 1,
                no_knockback: 2,
                globals: Vec::new(),
                client: Some(QvmModCombatClient {
                    pointer: 212,
                    record: "client".to_string(),
                    health: 236,
                    armor: 240,
                    protection: 0.5,
                    team: 244,
                }),
                abi: QvmModCombatAbi::Q3GDamage,
            }),
        }
    }

    fn fixture() -> Fixture {
        let owner = IdentityOwner::create("mod-actors-test").unwrap();
        let actor = owner.actor(0, 1);
        let other = owner.actor(1, 1);
        let owned = owner.owned_actor(&actor, ProviderId::new("test", "mod")).unwrap();
        let module = QvmModule::new(artifact(), None, None).unwrap();
        module.memory().write_i32(ENTITIES + 224, 100).unwrap();
        module.memory().write_i32(ENTITIES + 228, 1).unwrap();
        module.memory().write_i32(ENTITIES + 212, CLIENTS as i32).unwrap();
        module.memory().write_i32(CLIENTS + 236, 100).unwrap();
        module.memory().write_i32(CLIENTS + 240, 50).unwrap();
        module.memory().write_i32(CLIENTS + 244, 1).unwrap();
        let operations = Rc::new(FixtureOperations {
            pointers: [(actor.clone(), ENTITIES), (other.clone(), ENTITIES + 512)]
                .into_iter()
                .collect(),
            actors: [
                (ENTITIES as i32, actor.clone()),
                ((ENTITIES + 512) as i32, other.clone()),
            ]
            .into_iter()
            .collect(),
            invoked: RefCell::new(Vec::new()),
        });
        let services = Rc::new(FixtureServices {
            applied: RefCell::new(Vec::new()),
            observed: RefCell::new(Vec::new()),
            bindings: RefCell::new(HashMap::new()),
            reactions: RefCell::new(HashMap::new()),
            observers: RefCell::new(Vec::new()),
            release: RefCell::new(None),
            unsubscribed: Rc::new(Cell::new(false)),
            bodies: [(
                actor.clone(),
                QvmBodySample {
                    origin: vec3(1.0, 2.0, 3.0),
                    velocity: vec3(0.0, 0.0, 0.0),
                },
            )]
            .into_iter()
            .collect(),
            teams: HashMap::new(),
            uses: RefCell::new(Vec::new()),
            touches: RefCell::new(Vec::new()),
            pains: RefCell::new(Vec::new()),
            deaths: RefCell::new(Vec::new()),
        });
        let actors = QvmModActors::create(
            module.clone(),
            declaration(),
            Rc::clone(&services) as Rc<dyn QvmModActorServices>,
            Rc::clone(&operations) as Rc<dyn QvmModActorOperations>,
            HashSet::from([actor.clone()]),
        )
        .unwrap();
        Fixture {
            actors,
            module,
            operations,
            services,
            owned,
            other,
        }
    }

    #[test]
    fn validates_declarations() {
        let artifact = artifact();
        assert!(validate_qvm_mod_actors(&artifact, &declaration()).is_ok());
        assert!(validate_qvm_mod_actors(&artifact, &QvmModActorsDeclaration::default()).is_ok());

        let mut bad = declaration();
        bad.combat.as_mut().unwrap().entry = 99;
        assert!(validate_qvm_mod_actors(&artifact, &bad).is_err());

        let mut bad = declaration();
        bad.combat.as_mut().unwrap().godmode = 0;
        assert!(validate_qvm_mod_actors(&artifact, &bad).is_err());

        let mut bad = declaration();
        bad.source_actors.as_mut().unwrap().callbacks = None;
        assert!(validate_qvm_mod_actors(&artifact, &bad).is_err());

        let mut bad = declaration();
        bad.source_actors = None;
        assert!(validate_qvm_mod_actors(&artifact, &bad).is_err());

        let mut bad = declaration();
        bad.combat.as_mut().unwrap().client.as_mut().unwrap().protection = 2.0;
        assert!(validate_qvm_mod_actors(&artifact, &bad).is_err());

        let mut bad = declaration();
        bad.combat.as_mut().unwrap().client.as_mut().unwrap().record = "missing".to_string();
        assert!(validate_qvm_mod_actors(&artifact, &bad).is_err());

        let mut bad = declaration();
        bad.combat.as_mut().unwrap().abi = QvmModCombatAbi::Declared {
            calls: legacy_calls(),
            damage_flags: QvmDamageFlags {
                radius: 1,
                no_armor: 1,
                no_knockback: 4,
                no_protection: 8,
                no_team_protection: 16,
            },
            mass: QvmCombatMass::Constant { value: 100.0 },
            teams: Vec::new(),
        };
        assert!(validate_qvm_mod_actors(&artifact, &bad).is_err());

        let mut bad = declaration();
        bad.combat.as_mut().unwrap().abi = QvmModCombatAbi::Declared {
            calls: legacy_calls(),
            damage_flags: LEGACY_FLAGS,
            mass: QvmCombatMass::Constant { value: -1.0 },
            teams: Vec::new(),
        };
        assert!(validate_qvm_mod_actors(&artifact, &bad).is_err());

        let mut bad = declaration();
        bad.combat.as_mut().unwrap().abi = QvmModCombatAbi::Declared {
            calls: legacy_calls(),
            damage_flags: LEGACY_FLAGS,
            mass: QvmCombatMass::Constant { value: 100.0 },
            teams: vec![QvmCombatTeam {
                value: 1,
                team: "q3:1".to_string(),
            }],
        };
        bad.combat.as_mut().unwrap().client = None;
        assert!(validate_qvm_mod_actors(&artifact, &bad).is_err());

        let mut bad = declaration();
        bad.combat.as_mut().unwrap().abi = QvmModCombatAbi::Declared {
            calls: legacy_calls(),
            damage_flags: LEGACY_FLAGS,
            mass: QvmCombatMass::Constant { value: 100.0 },
            teams: vec![QvmCombatTeam {
                value: 1,
                team: "flat".to_string(),
            }],
        };
        assert!(validate_qvm_mod_actors(&artifact, &bad).is_err());
    }

    #[test]
    fn admit_binds_and_reads_state() {
        let fixture = fixture();
        fixture.actors.admit(&fixture.owned).unwrap();
        let binding = fixture
            .services
            .bindings
            .borrow()
            .get(fixture.owned.id())
            .cloned()
            .unwrap();
        let state = (binding.read)().unwrap();
        assert_eq!(state.health, 100);
        assert!(state.can_take_damage);
        assert_eq!(
            state.armor.regular,
            QvmRegularArmor::Q3 {
                points: 50,
                protection: 0.5,
            }
        );
        assert_eq!(state.mass, 200.0);
        assert_eq!(state.team.as_deref(), Some("q3:1"));

        (binding.write_health)(80).unwrap();
        assert_eq!(fixture.module.memory().read_i32(ENTITIES + 224).unwrap(), 80);
        assert_eq!(fixture.module.memory().read_i32(CLIENTS + 236).unwrap(), 80);
        (binding.write_armor)(&QvmArmorState {
            regular: QvmRegularArmor::Q3 {
                points: 30,
                protection: 0.5,
            },
            powered: QvmPoweredArmor::None,
        })
        .unwrap();
        assert_eq!(fixture.module.memory().read_i32(CLIENTS + 240).unwrap(), 30);
        (binding.validate_armor)(&QvmArmorState {
            regular: QvmRegularArmor::None,
            powered: QvmPoweredArmor::None,
        })
        .unwrap();
    }

    #[test]
    fn combat_hook_routes_owned_and_foreign() {
        let fixture = fixture();
        fixture.actors.admit(&fixture.owned).unwrap();
        fixture.module.memory().write_vec3(16384, &vec3(0.0, 0.0, 1.0)).unwrap();
        let words = vec![ENTITIES as i32, 0, (ENTITIES + 512) as i32, 16384, 16396, 25, 0, 3];
        assert_eq!(fixture.module.call(&words, 5).unwrap(), 0);
        assert_eq!(fixture.services.observed.borrow().len(), 1);
        let request = fixture.services.observed.borrow()[0].clone();
        assert_eq!(request.target, *fixture.owned.id());
        assert_eq!(request.amount, 25.0);
        assert_eq!(request.knockback, 25.0);
        assert_eq!(request.delivery, QvmDamageDelivery::Direct);

        let foreign = vec![(ENTITIES + 512) as i32, 0, ENTITIES as i32, 16384, 16396, 10, 0, 3];
        assert_eq!(fixture.module.call(&foreign, 5).unwrap(), 0);
        assert_eq!(fixture.services.applied.borrow().len(), 1);

        let binding = fixture
            .services
            .bindings
            .borrow()
            .get(fixture.owned.id())
            .cloned()
            .unwrap();
        let outcome = (binding.damage)(&request).unwrap();
        assert_eq!(outcome.reaction, QvmDamageReaction::None);
        let invoked = fixture.operations.invoked.borrow();
        assert_eq!(invoked.last().unwrap().0, 5);
        assert_eq!(invoked.last().unwrap().1[0], ENTITIES as i32);
        assert_eq!(invoked.last().unwrap().1[3], 32768);
    }

    #[test]
    fn touch_reaction_writes_trace() {
        let fixture = fixture();
        fixture.actors.admit(&fixture.owned).unwrap();
        fixture.module.memory().write_i32(ENTITIES + 248, 9).unwrap();
        let reactions = fixture
            .services
            .reactions
            .borrow()
            .get(fixture.owned.id())
            .cloned()
            .unwrap();
        (reactions.touch)(&QvmTouchContact {
            owner: fixture.owned.clone(),
            other: fixture.other.clone(),
            plane: Some(QvmContactPlane {
                normal: vec3(0.0, 0.0, 1.0),
                distance: 5.0,
            }),
            surface: None,
        })
        .unwrap();
        let invoked = fixture.operations.invoked.borrow();
        assert_eq!(invoked.len(), 1);
        assert_eq!(invoked[0].0, 9);
        assert_eq!(invoked[0].1, vec![ENTITIES as i32, (ENTITIES + 512) as i32, 32768]);
        let bytes = fixture.module.memory().read_bytes(32768, QVM_TRACE_BYTES).unwrap();
        let trace = read_qvm_trace(&bytes).unwrap();
        assert!(!trace.all_solid);
        assert_eq!(trace.fraction, 0.0);
        assert_eq!(trace.entity_num, 1023);
        assert_eq!(trace.plane.plane_type, 2);
    }

    #[test]
    fn pain_callback_drives_frames() {
        let fixture = fixture();
        fixture.actors.admit(&fixture.owned).unwrap();
        fixture.module.memory().write_i32(ENTITIES + 256, 11).unwrap();
        assert_eq!(
            fixture
                .module
                .call(&[ENTITIES as i32, (ENTITIES + 512) as i32, 30], 11)
                .unwrap(),
            0
        );
        assert_eq!(fixture.services.pains.borrow().len(), 1);
        assert_eq!(fixture.services.pains.borrow()[0].damage, 30.0);
        assert!(fixture.services.observers.borrow().is_empty());

        let request = QvmModDamageRequest {
            target: fixture.owned.id().clone(),
            amount: 25.0,
            knockback: 25.0,
            direction: vec3(0.0, 0.0, 1.0),
            point: vec3(0.0, 0.0, 0.0),
            normal: vec3(0.0, 0.0, 0.0),
            delivery: QvmDamageDelivery::Direct,
            attack: QvmModAttack {
                time_seconds: 1.5,
                attacker: Some(fixture.other.clone()),
                inflictor: None,
                cause: QvmModCause::Q3 {
                    damage_flags: 0,
                    means_of_death: 3,
                },
                movement_provider: "test-move".to_string(),
            },
        };
        let actors = Rc::clone(&fixture.actors);
        let memory = fixture.module.memory();
        let outcome = actors
            .damage(&request, &|effective| {
                assert_eq!(effective.target, request.target);
                let mut call =
                    QvmFunctionCall::entered(11, vec![ENTITIES as i32, (ENTITIES + 512) as i32, 30], memory.clone());
                assert_eq!(actors.source_callback(11, &mut call).unwrap(), 0);
                Ok(7)
            })
            .unwrap();
        assert_eq!(outcome.applied_damage, 30);
        assert_eq!(outcome.reaction, QvmDamageReaction::Pain);
        let observers = fixture.services.observers.borrow();
        assert_eq!(observers.len(), 1);
        assert_eq!(
            observers[0].before.borrow().as_slice(),
            &[QvmDamageOutcome {
                applied_damage: 30,
                reaction: QvmDamageReaction::Pain,
            }]
        );
        assert!(observers[0].stored.borrow().is_empty());
    }

    #[test]
    fn resolver_ambiguity_stashes_error() {
        let fixture = fixture();
        fixture.actors.admit(&fixture.owned).unwrap();
        fixture.module.memory().write_i32(ENTITIES + 248, 9).unwrap();
        fixture.module.memory().write_i32(ENTITIES + 252, 9).unwrap();
        assert_eq!(
            fixture
                .module
                .call(&[ENTITIES as i32, (ENTITIES + 512) as i32, 0], 9)
                .unwrap(),
            0
        );
        assert!(fixture.actors.take_error().is_some());
        assert!(fixture.actors.take_error().is_none());
    }

    #[test]
    fn use_callback_and_proceed_restore_words() {
        let fixture = fixture();
        fixture.actors.admit(&fixture.owned).unwrap();
        fixture.module.memory().write_i32(ENTITIES + 252, 10).unwrap();
        assert_eq!(
            fixture
                .module
                .call(&[ENTITIES as i32, (ENTITIES + 512) as i32, 0], 10)
                .unwrap(),
            0
        );
        assert_eq!(fixture.services.uses.borrow().len(), 1);

        let memory = fixture.module.memory();
        let mut call = QvmFunctionCall::entered(10, vec![1, 2, 3], memory);
        call.proceed_value = 42;
        let cell = RefCell::new(&mut call);
        assert_eq!(fixture.actors.proceed(&cell, &[(1, 99)]), 42);
        assert_eq!(cell.borrow().words, vec![1, 2, 3]);
        assert!(cell.borrow().proceeded);
    }

    #[test]
    fn release_and_close_cleanup() {
        let fixture = fixture();
        fixture.actors.admit(&fixture.owned).unwrap();
        assert!(!fixture.actors.pointers.borrow().is_empty());
        let release = fixture.services.release.borrow().clone().unwrap();
        release(&fixture.owned);
        assert!(fixture.actors.pointers.borrow().is_empty());
        assert!(fixture.actors.actor_pointers.borrow().is_empty());

        fixture.module.set_default_return(7);
        fixture.actors.close();
        assert!(fixture.services.unsubscribed.get());
        assert_eq!(fixture.module.call(&[ENTITIES as i32], 5).unwrap(), 7);
        assert_eq!(fixture.module.call(&[ENTITIES as i32, 0, 0], 11).unwrap(), 7);
    }

    #[test]
    fn source_touch_and_die_route() {
        let fixture = fixture();
        fixture.actors.admit(&fixture.owned).unwrap();
        fixture.module.memory().write_i32(ENTITIES + 248, 9).unwrap();
        fixture.module.memory().write_i32(ENTITIES + 260, 12).unwrap();
        assert_eq!(
            fixture
                .module
                .call(&[ENTITIES as i32, (ENTITIES + 512) as i32, 0], 9)
                .unwrap(),
            0
        );
        assert_eq!(fixture.services.touches.borrow().len(), 1);
        assert!(fixture.services.touches.borrow()[0].plane.is_none());

        assert_eq!(
            fixture
                .module
                .call(
                    &[ENTITIES as i32, (ENTITIES + 512) as i32, (ENTITIES + 512) as i32, 40, 3],
                    12,
                )
                .unwrap(),
            0
        );
        assert_eq!(fixture.services.deaths.borrow().len(), 1);
        assert_eq!(
            fixture.services.deaths.borrow()[0].inflictor,
            Some(fixture.other.clone())
        );
        assert_eq!(fixture.services.deaths.borrow()[0].pain.damage, 40.0);
    }

    #[test]
    fn before_release_flushes_frames() {
        let fixture = fixture();
        fixture.actors.admit(&fixture.owned).unwrap();
        assert!(fixture.actors.before_release(fixture.owned.id()).is_ok());
        let request = QvmModDamageRequest {
            target: fixture.owned.id().clone(),
            amount: 10.0,
            knockback: 10.0,
            direction: vec3(0.0, 0.0, 1.0),
            point: vec3(0.0, 0.0, 0.0),
            normal: vec3(0.0, 0.0, 0.0),
            delivery: QvmDamageDelivery::Direct,
            attack: QvmModAttack {
                time_seconds: 1.5,
                attacker: None,
                inflictor: None,
                cause: QvmModCause::Other,
                movement_provider: "test-move".to_string(),
            },
        };
        let actors = Rc::clone(&fixture.actors);
        actors
            .damage(&request, &|effective| {
                actors.before_release(&effective.target).unwrap();
                Ok(0)
            })
            .unwrap();
    }

    #[test]
    fn resolves_input_pointers() {
        let fixture = fixture();
        let memory = fixture.module.memory();
        memory.write_i32(512, 1024).unwrap();
        memory.write_i32(516, 3000).unwrap();
        memory.write_i32(1028, 2048).unwrap();
        let call = QvmFunctionCall::entered(1, vec![512], memory);
        let argument = QvmModInputPointer {
            base: QvmModInputPointerBase::Argument { index: 0 },
            indirections: vec![4],
            offset: 8,
        };
        assert_eq!(resolve_qvm_input_pointer(&argument, &call).unwrap(), 3008);
        let global = QvmModInputPointer {
            base: QvmModInputPointerBase::Global { address: 512 },
            indirections: Vec::new(),
            offset: 4,
        };
        assert_eq!(resolve_qvm_input_pointer(&global, &call).unwrap(), 1028);
        let bad = QvmModInputPointer {
            base: QvmModInputPointerBase::Argument { index: 7 },
            indirections: Vec::new(),
            offset: 0,
        };
        assert!(resolve_qvm_input_pointer(&bad, &call).is_err());
    }
}
