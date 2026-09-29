//! QuakeC gameplay-mod provider: projections, invocation, lifecycle.
//!
//! Ported from `src/compat/qc/mod-provider.ts`. This module is also the home
//! of the shared declaration mirrors absorbed from `src/contracts/mods.ts`
//! (mod identity and checkpoints), `src/contracts/mod-callbacks.ts` (callback
//! declarations, source calls, actor fields, items, protection, pickups,
//! console commands), `src/contracts/original-pickups.ts` (pickup rules),
//! `src/contracts/source-match.ts` (team/score fields), `src/contracts/source-
//! items.ts` (item action names), `src/contracts/qc-weapon-stage.ts` (weapon
//! stage declarations), `src/contracts/mod-client-presentation.ts` (client
//! presentation), `src/contracts/mod-client-outputs.ts` (client outputs),
//! `src/contracts/numeric.ts` (`RandomSource`), and the `asciiFold` /
//! `tokenizeCommand` mirrors from `src/core/commands/text.ts`.
//!
//! Local mirrors (owned by other workers' modules, noted here): `QcValueType`
//! and `QcProgramView` mirror `src/compat/qc/program.ts`; `QcProviderMachine`
//! mirrors the `QcMachine` surface from `src/compat/qc/machine.ts` that the
//! provider touches; `QcProviderServices` mirrors the `ModHostServices`
//! surface from `src/world/session/mods.ts` that the provider touches. The
//! host composes the sibling `qc::mod_*` modules; the provider owns actor
//! projections, invocation depth, console dispatch, and its own checkpoint.

use std::collections::{HashMap, HashSet};

use qa_core::identity::{ActorId, OwnedActor, ProviderId, SavedActorId};
use qa_core::math::{Bounds, Vec3};
use qa_core::time::{FrameContext, SourceTime};

use crate::error::GuestError;
use crate::fields::{FieldLayout, FieldValue};
use crate::traits::{GameContext, GameModule, GameReaction, GameTouch};

// ---------------------------------------------------------------------------
// Item and content identifiers
// ---------------------------------------------------------------------------

/// Canonical item identifier (`namespace:name`).
pub type ItemId = String;

/// Content digest of a declared artifact.
pub type ContentDigest = String;

/// Content collection identifier.
pub type ContentId = String;

/// Guest module identifier.
pub type ModuleId = String;

// ---------------------------------------------------------------------------
// Program view (mirrors `src/compat/qc/program.ts`)
// ---------------------------------------------------------------------------

/// QuakeC value type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum QcValueType {
    /// No value.
    Void,
    /// String index.
    String,
    /// Single float.
    Float,
    /// Three-component vector.
    Vector,
    /// Entity reference.
    Entity,
    /// Field reference.
    Field,
    /// Function reference.
    Function,
    /// Raw pointer.
    Pointer,
    /// Opaque builtin value.
    Opaque,
}

/// QuakeC API family.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum QcApiKind {
    /// NetQuake.
    Q1Netquake,
    /// QuakeWorld.
    Q1Quakeworld,
}

/// Function metadata needed for declaration validation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QcFunctionView {
    /// Function index (0 is the null function).
    pub index: i32,
    /// Function name.
    pub name: String,
    /// First statement, or 0/negative for builtins.
    pub first_statement: i32,
    /// First parameter word.
    pub parameter_start: i32,
    /// Parameter word sizes.
    pub parameter_sizes: Vec<i32>,
    /// Whether this slot is a named builtin.
    pub named_builtin: bool,
}

/// Read-only program metadata for declaration validation.
pub trait QcProgramView {
    /// Program artifact digest.
    fn digest(&self) -> &str;
    /// API family.
    fn api_kind(&self) -> QcApiKind;
    /// Entity field type by name.
    fn field_type(&self, name: &str) -> Option<QcValueType>;
    /// Global type by name.
    fn global_type(&self, name: &str) -> Option<QcValueType>;
    /// Function metadata by name.
    fn function_named(&self, name: &str) -> Option<QcFunctionView>;
    /// Function metadata by index.
    fn function_at(&self, index: i32) -> Option<QcFunctionView>;
    /// All function metadata.
    fn functions(&self) -> Vec<QcFunctionView>;
}

// ---------------------------------------------------------------------------
// Callback inputs, values, and source calls
// ---------------------------------------------------------------------------

/// Named callback input produced by the host or read from source words.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ModCallbackInput {
    /// Client view angles vector.
    ViewAngles,
    /// Attack button scalar.
    Attack,
    /// Jump button scalar.
    Jump,
    /// Impulse scalar.
    Impulse,
    /// Forward move scalar.
    ForwardMove,
    /// Side move scalar.
    SideMove,
    /// Up move scalar.
    UpMove,
    /// Subject actor.
    Self_,
    /// Other actor.
    Other,
    /// Activator actor.
    Activator,
    /// Attacker actor.
    Attacker,
    /// Inflictor actor.
    Inflictor,
    /// Damage or count amount.
    Amount,
    /// Packed damage flags.
    DamageFlags,
    /// Regular protection scale.
    RegularProtectionScale,
    /// Knockback scalar.
    Knockback,
    /// Damage point.
    Point,
    /// Damage direction.
    Direction,
    /// Damage normal.
    Normal,
    /// Item identifier string.
    Item,
    /// Current time.
    Time,
    /// Frame elapsed time.
    Elapsed,
    /// Observed result.
    Result,
    /// Pickup count override.
    PickupCount,
    /// Whether the pickup carries a count override.
    PickupHasCount,
    /// Whether the pickup was dropped.
    PickupDropped,
}

impl ModCallbackInput {
    /// Donor input name.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::ViewAngles => "view-angles",
            Self::Attack => "attack",
            Self::Jump => "jump",
            Self::Impulse => "impulse",
            Self::ForwardMove => "forward-move",
            Self::SideMove => "side-move",
            Self::UpMove => "up-move",
            Self::Self_ => "self",
            Self::Other => "other",
            Self::Activator => "activator",
            Self::Attacker => "attacker",
            Self::Inflictor => "inflictor",
            Self::Amount => "amount",
            Self::DamageFlags => "damage-flags",
            Self::RegularProtectionScale => "regular-protection-scale",
            Self::Knockback => "knockback",
            Self::Point => "point",
            Self::Direction => "direction",
            Self::Normal => "normal",
            Self::Item => "item",
            Self::Time => "time",
            Self::Elapsed => "elapsed",
            Self::Result => "result",
            Self::PickupCount => "pickup-count",
            Self::PickupHasCount => "pickup-has-count",
            Self::PickupDropped => "pickup-dropped",
        }
    }
}

/// Client command input subset.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ModClientInput {
    /// View angles vector.
    ViewAngles,
    /// Attack button scalar.
    Attack,
    /// Jump button scalar.
    Jump,
    /// Impulse scalar.
    Impulse,
    /// Forward move scalar.
    ForwardMove,
    /// Side move scalar.
    SideMove,
    /// Up move scalar.
    UpMove,
}

impl ModClientInput {
    /// Widen to the shared callback-input space.
    #[must_use]
    pub fn as_callback_input(&self) -> ModCallbackInput {
        match self {
            Self::ViewAngles => ModCallbackInput::ViewAngles,
            Self::Attack => ModCallbackInput::Attack,
            Self::Jump => ModCallbackInput::Jump,
            Self::Impulse => ModCallbackInput::Impulse,
            Self::ForwardMove => ModCallbackInput::ForwardMove,
            Self::SideMove => ModCallbackInput::SideMove,
            Self::UpMove => ModCallbackInput::UpMove,
        }
    }
}

/// Declared call argument: a host input or a literal.
#[derive(Debug, Clone, PartialEq)]
pub enum ModCallbackValue {
    /// Host-provided input.
    Input(ModCallbackInput),
    /// Float literal.
    Float(f64),
    /// String literal.
    String(String),
    /// Vector literal.
    Vector(Vec3),
}

/// Constant actor-field value (never a host input).
#[derive(Debug, Clone, PartialEq)]
pub enum ModConstantValue {
    /// Float literal.
    Float(f64),
    /// String literal.
    String(String),
    /// Vector literal.
    Vector(Vec3),
}

/// Runtime value: literals plus actor references.
#[derive(Debug, Clone, PartialEq)]
pub enum ModRuntimeValue {
    /// Actor reference (null when absent).
    Actor(Option<ActorId>),
    /// Float value.
    Float(f64),
    /// String value.
    String(String),
    /// Vector value.
    Vector(Vec3),
}

/// Runtime value alias used by the provider surface.
pub type QcModValue = ModRuntimeValue;

/// Runtime inputs keyed by callback input.
pub type QcModInputs = HashMap<ModCallbackInput, QcModValue>;

/// Named global override for one source call.
#[derive(Debug, Clone, PartialEq)]
pub struct ModSourceGlobal {
    /// Global name.
    pub name: String,
    /// Value expression.
    pub value: ModCallbackValue,
}

/// Declared call into compiled source.
#[derive(Debug, Clone, PartialEq)]
pub struct ModSourceCall {
    /// Function name.
    pub function: String,
    /// Positional arguments.
    pub arguments: Vec<ModCallbackValue>,
    /// Global overrides.
    pub globals: Vec<ModSourceGlobal>,
}

// ---------------------------------------------------------------------------
// Actor fields and match helpers
// ---------------------------------------------------------------------------

/// Client-input write policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ClientInputUpdate {
    /// Always write the validated input.
    Always,
    /// Write only nonzero validated input.
    Nonzero,
}

/// Declared original value and its shared team identity.
#[derive(Debug, Clone, PartialEq)]
pub struct SourceTeamValue {
    /// Original scalar value.
    pub value: f64,
    /// Shared team identity (null when unassigned).
    pub team: Option<String>,
}

/// Actor-field binding.
#[derive(Debug, Clone, PartialEq)]
pub enum ModActorBinding {
    /// Shared team identity.
    Team {
        /// Declared value mapping.
        values: Vec<SourceTeamValue>,
    },
    /// Shared score.
    Score,
    /// Canonical health.
    Health,
    /// Canonical origin.
    Origin,
    /// Canonical velocity.
    Velocity,
    /// Canonical angles.
    Angles,
    /// Canonical bounds minimum.
    BoundsMin,
    /// Canonical bounds maximum.
    BoundsMax,
    /// Think function pointer.
    Think,
    /// Next think time.
    Nextthink,
    /// Private source storage.
    Private,
    /// Canonical classname.
    Classname,
    /// Client view offset.
    ViewOffset,
    /// Canonical client flags with private bits.
    ClientFlags {
        /// Whether the grounded bit is projected.
        grounded: bool,
        /// Private bit mask.
        private_mask: Option<i32>,
    },
    /// Canonical client input.
    ClientInput {
        /// Input channel.
        input: ModClientInput,
        /// Write policy.
        update: ClientInputUpdate,
        /// Scalar encoding scale.
        scale: Option<f64>,
    },
    /// Client userinfo key.
    Userinfo {
        /// Info key.
        key: String,
    },
    /// Canonical inventory count.
    Inventory {
        /// Item identifier.
        item: ItemId,
    },
    /// Fixed constant.
    Constant {
        /// Constant value.
        value: ModConstantValue,
    },
}

/// Declared actor field.
#[derive(Debug, Clone, PartialEq)]
pub struct ModActorField {
    /// Field name.
    pub field: String,
    /// Binding.
    pub binding: ModActorBinding,
}

/// Resolve a shared team identity from an original scalar.
pub fn source_team(values: &[SourceTeamValue], value: f64) -> Result<Option<String>, GuestError> {
    values
        .iter()
        .find(|entry| entry.value == value)
        .map(|entry| entry.team.clone())
        .ok_or_else(|| GuestError::invalid(format!("Original team value {value} has no declared shared identity")))
}

/// Resolve an original scalar from a shared team identity.
pub fn original_team(values: &[SourceTeamValue], team: Option<&str>) -> Result<f64, GuestError> {
    values
        .iter()
        .find(|entry| entry.team.as_deref() == team)
        .map(|entry| entry.value)
        .ok_or_else(|| {
            GuestError::invalid(format!(
                "Shared team {} has no declared original value",
                team.unwrap_or("unassigned")
            ))
        })
}

/// Validate a team/score match field.
pub fn validate_source_match_field(binding: &ModActorBinding) -> Result<(), GuestError> {
    let values = match binding {
        ModActorBinding::Score => return Ok(()),
        ModActorBinding::Team { values } => values,
        _ => return Err(GuestError::invalid("Match field must bind team or score")),
    };
    if values.is_empty()
        || values
            .iter()
            .any(|entry| !entry.value.is_finite() || entry.team.as_deref() == Some(""))
    {
        return Err(GuestError::invalid(
            "Team projection requires distinct original values and shared identities",
        ));
    }
    let mut scalars = Vec::with_capacity(values.len());
    let mut teams = Vec::with_capacity(values.len());
    for entry in values {
        // f64 has no Hash; compare by exact equality instead.
        if scalars.iter().any(|known: &f64| *known == entry.value) || teams.contains(&entry.team) {
            return Err(GuestError::invalid(
                "Team projection requires distinct original values and shared identities",
            ));
        }
        scalars.push(entry.value);
        teams.push(entry.team.clone());
    }
    Ok(())
}

/// Minimal match-player surface used by team/score fields.
pub trait ModMatchServices {
    /// Shared team identity of an admitted player.
    fn player_team(&self, actor: &ActorId) -> Option<Option<String>>;
    /// Shared score of an admitted player.
    fn player_score(&self, actor: &ActorId) -> Option<f64>;
    /// Assign a shared team identity.
    fn set_player_team(&mut self, actor: &ActorId, team: Option<String>) -> Result<(), GuestError>;
    /// Assign a shared score.
    fn set_player_score(&mut self, actor: &ActorId, score: f64) -> Result<(), GuestError>;
}

/// Read a projected team/score scalar.
pub fn read_source_match_field(
    services: &dyn ModMatchServices,
    actor: &ActorId,
    binding: &ModActorBinding,
) -> Result<f64, GuestError> {
    match binding {
        ModActorBinding::Score => Ok(services.player_score(actor).unwrap_or(0.0)),
        ModActorBinding::Team { values } => {
            let team = services.player_team(actor).flatten();
            original_team(values, team.as_deref())
        }
        _ => Err(GuestError::invalid("Match read requires a team or score binding")),
    }
}

/// Write a projected team/score scalar.
pub fn write_source_match_field(
    services: &mut dyn ModMatchServices,
    actor: &ActorId,
    binding: &ModActorBinding,
    value: f64,
) -> Result<(), GuestError> {
    match binding {
        ModActorBinding::Score => services.set_player_score(actor, value),
        ModActorBinding::Team { values } => {
            let team = source_team(values, value)?;
            services.set_player_team(actor, team)
        }
        _ => Err(GuestError::invalid("Match write requires a team or score binding")),
    }
}

// ---------------------------------------------------------------------------
// Combat, armor stages, and protection
// ---------------------------------------------------------------------------

/// Verified source statement word.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct QcStatement {
    /// Opcode.
    pub opcode: i32,
    /// First operand.
    pub a: i32,
    /// Second operand.
    pub b: i32,
    /// Third operand.
    pub c: i32,
}

/// Weapon-stage repeat region.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QcWeaponRepeat {
    /// Function name.
    pub function: String,
    /// Entry statement.
    pub entry: i32,
    /// Exit statement.
    pub exit: i32,
    /// Result word and value.
    pub result: (i32, i32),
    /// Verified statements.
    pub statements: Vec<QcStatement>,
}

/// Declared weapon-stage dispatcher.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct QcWeaponStageDeclaration {
    /// Dispatcher function.
    pub dispatcher: String,
    /// Continuation functions.
    pub continuations: Vec<String>,
    /// Repeat regions.
    pub repeats: Vec<QcWeaponRepeat>,
}

/// Damage-scale region kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ModQcDamageScaleKind {
    /// Multiply damage.
    #[default]
    Multiplier,
    /// Pass damage through.
    Identity,
    /// Run a transform region.
    Transform,
}

/// Declared damage-scale region.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModQcDamageScale {
    /// Region kind.
    pub kind: ModQcDamageScaleKind,
    /// Function name.
    pub function: String,
    /// Entry statement.
    pub entry: i32,
    /// Exit statement.
    pub exit: i32,
    /// Damage word.
    pub damage: i32,
    /// Verified statements.
    pub statements: Vec<QcStatement>,
}

/// Armor-stage flag layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ModQcArmorStageFlags {
    /// No flag word.
    None,
    /// Packed flag bits.
    Bits {
        /// Flag word.
        word: i32,
        /// No-armor bit.
        no_armor: i32,
        /// No-power-armor bit.
        no_power_armor: i32,
        /// No-regular-armor bit.
        no_regular_armor: i32,
        /// Energy bit.
        energy: i32,
    },
}

/// Caller-local regular protection scale.
#[derive(Debug, Clone, PartialEq)]
pub struct ModQcRegularScale {
    /// Caller function.
    pub caller: String,
    /// Statement index.
    pub statement: i32,
    /// Scale factor.
    pub scale: f64,
}

/// Declared armor-stage region.
#[derive(Debug, Clone, PartialEq)]
pub struct ModQcArmorStage {
    /// Function name.
    pub function: String,
    /// Entry statement.
    pub entry: i32,
    /// Exit statement.
    pub exit: i32,
    /// Target word.
    pub target: i32,
    /// Damage word.
    pub damage: i32,
    /// Saved word.
    pub saved: i32,
    /// Caller-local regular scales.
    pub regular_scale: Vec<ModQcRegularScale>,
    /// Flag layout.
    pub flags: ModQcArmorStageFlags,
    /// Verified statements.
    pub statements: Vec<QcStatement>,
}

/// Declared points-only armor grant.
#[derive(Debug, Clone, PartialEq)]
pub struct ModQcEmptyArmor {
    /// Armor item.
    pub item: ItemId,
    /// Absorption fraction.
    pub absorption: f64,
}

/// Declared combat lowering.
#[derive(Debug, Clone, PartialEq)]
pub struct ModCombatDeclaration {
    /// Damage function call.
    pub damage: ModSourceCall,
    /// Optional damage scale.
    pub damage_scale: Option<ModQcDamageScale>,
    /// Optional armor stage.
    pub armor_stage: Option<ModQcArmorStage>,
    /// Optional points-only armor.
    pub empty_armor: Option<ModQcEmptyArmor>,
}

/// Protection channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ProtectionChannel {
    /// Regular armor.
    Regular,
    /// Powered protection.
    Powered,
}

/// Protection admission policy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProtectionAdmission {
    /// Claim the channel.
    Claim,
    /// Replace the current primary.
    ReplaceCurrentPrimary,
    /// Replace one owner's primary.
    ReplacePrimary {
        /// Owner to replace.
        owner: ProviderId,
    },
}

impl Default for ProtectionAdmission {
    fn default() -> Self {
        Self::Claim
    }
}

/// Protection absorb lowering.
#[derive(Debug, Clone, PartialEq)]
pub enum ModProtectionAbsorb {
    /// Whole-function absorb.
    Function {
        /// Absorb call.
        call: ModSourceCall,
    },
    /// Verified region absorb.
    Region {
        /// Absorb call.
        call: ModSourceCall,
        /// Armor stage.
        stage: ModQcArmorStage,
    },
}

impl ModProtectionAbsorb {
    /// Absorb call.
    #[must_use]
    pub fn call(&self) -> &ModSourceCall {
        match self {
            Self::Function { call } | Self::Region { call, .. } => call,
        }
    }
}

/// Declared damage-flag bits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ModProtectionFlags {
    /// No-armor bit.
    pub no_armor: i32,
    /// No-power-armor bit.
    pub no_power_armor: i32,
    /// No-regular-armor bit.
    pub no_regular_armor: i32,
    /// Energy bit.
    pub energy: i32,
    /// Radius-delivery bit.
    pub radius: i32,
}

/// Regular-armor selection value.
#[derive(Debug, Clone, PartialEq)]
pub struct ModRegularSelectionValue {
    /// Original scalar.
    pub value: f64,
    /// Selected item.
    pub item: Option<ItemId>,
}

/// Regular-armor selection storage.
#[derive(Debug, Clone, PartialEq)]
pub struct ModRegularSelection {
    /// Selection field.
    pub field: String,
    /// Selection mask.
    pub mask: Option<i32>,
    /// Declared values.
    pub values: Vec<ModRegularSelectionValue>,
}

/// Regular-armor storage.
#[derive(Debug, Clone, PartialEq)]
pub struct ModRegularStorage {
    /// Points field.
    pub points: String,
    /// Fixed item (no selection).
    pub item: Option<ItemId>,
    /// Optional selection.
    pub selection: Option<ModRegularSelection>,
}

/// Powered-protection kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PoweredKind {
    /// No protection.
    None,
    /// Screen.
    Screen,
    /// Shield.
    Shield,
}

/// Powered-protection selection value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModPoweredSelectionValue {
    /// Original scalar.
    pub value: f64,
    /// Selected kind.
    pub kind: PoweredKind,
}

/// Powered-protection selection storage.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModPoweredSelection {
    /// Selection field.
    pub field: String,
    /// Selection mask.
    pub mask: Option<i32>,
    /// Declared values.
    pub values: Vec<ModPoweredSelectionValue>,
}

/// Powered-protection storage.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModPoweredStorage {
    /// Cells field.
    pub cells: String,
    /// Fixed kind (no selection).
    pub kind: PoweredKind,
    /// Optional selection.
    pub selection: Option<ModPoweredSelection>,
}

/// Declared protection channel.
#[derive(Debug, Clone, PartialEq)]
pub struct ModQcProtection {
    /// Rule identifier.
    pub id: String,
    /// Admission policy.
    pub admission: ProtectionAdmission,
    /// Channel.
    pub channel: ProtectionChannel,
    /// Regular storage (regular channel).
    pub regular: Option<ModRegularStorage>,
    /// Powered storage (powered channel).
    pub powered: Option<ModPoweredStorage>,
    /// Absorb lowering.
    pub absorb: ModProtectionAbsorb,
    /// Declared flag bits.
    pub flags: ModProtectionFlags,
}

// ---------------------------------------------------------------------------
// Pickups
// ---------------------------------------------------------------------------

/// Inventory write scope.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PickupFields {
    /// Count only.
    Count,
    /// Capacity only.
    Capacity,
    /// Count and capacity.
    CountAndCapacity,
}

/// Declared pickup write.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PickupWrite {
    /// Protection write.
    Protection {
        /// Channel.
        channel: ProtectionChannel,
    },
    /// Inventory write.
    Inventory {
        /// Item identifier.
        item: ItemId,
        /// Write scope.
        fields: PickupFields,
    },
}

/// Grant acceptance rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GrantAccepts {
    /// Nonzero result accepts.
    Nonzero,
    /// Always accepts.
    Always,
}

/// Declared pickup operation.
#[derive(Debug, Clone, PartialEq)]
pub enum PickupOperation {
    /// Single boolean grant.
    BooleanGrant {
        /// Grant call.
        grant: ModSourceCall,
    },
    /// Gate followed by a grant.
    GateThenGrant {
        /// Gate call.
        gate: ModSourceCall,
        /// Grant call.
        grant: ModSourceCall,
        /// Grant acceptance rule.
        grant_accepts: GrantAccepts,
    },
}

/// Declared pickup rule.
#[derive(Debug, Clone, PartialEq)]
pub struct ModPickupRule {
    /// Rule identifier.
    pub id: String,
    /// Declared writes (at least one).
    pub writes: Vec<PickupWrite>,
    /// Offered items.
    pub offered: Vec<ItemId>,
    /// Operation.
    pub operation: PickupOperation,
}

// ---------------------------------------------------------------------------
// Console commands
// ---------------------------------------------------------------------------

/// Console argument scalar type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ModConsoleArgType {
    /// String argument.
    String,
    /// Float argument.
    Float,
}

/// Console value expression.
#[derive(Debug, Clone, PartialEq)]
pub enum ModConsoleValue {
    /// Float literal.
    Float(f64),
    /// String literal.
    String(String),
    /// Vector literal.
    Vector(Vec3),
    /// Command-line argument.
    Argument {
        /// Argument index.
        index: usize,
        /// Argument type.
        arg_type: ModConsoleArgType,
    },
    /// Full argument text.
    ArgumentsText,
    /// Argument count.
    ArgumentCount,
}

/// Console global override.
#[derive(Debug, Clone, PartialEq)]
pub struct ModConsoleGlobal {
    /// Global name.
    pub name: String,
    /// Value expression.
    pub value: ModConsoleValue,
}

/// Declared console command.
#[derive(Debug, Clone, PartialEq)]
pub struct ModConsoleCommand {
    /// Command name.
    pub name: String,
    /// Function name.
    pub function: String,
    /// Positional arguments.
    pub arguments: Vec<ModConsoleValue>,
    /// Global overrides.
    pub globals: Vec<ModConsoleGlobal>,
}

// ---------------------------------------------------------------------------
// Client input and outputs
// ---------------------------------------------------------------------------

/// Client input output produced by source.
#[derive(Debug, Clone, PartialEq)]
pub enum ModClientInputOutput {
    /// Set a scalar input.
    SetScalar {
        /// Input channel.
        input: ModClientInput,
        /// New value.
        value: f64,
    },
    /// Set view angles.
    SetAngles {
        /// New angles.
        value: Vec3,
    },
    /// Consume inputs.
    Consume {
        /// Consumed inputs.
        inputs: Vec<ModClientInput>,
    },
}

/// Declared source input output.
#[derive(Debug, Clone, PartialEq)]
pub enum ModQcInputOutput {
    /// Watch a field for writes.
    Field {
        /// Field name.
        field: String,
    },
    /// Watch a handler for entry.
    Handler {
        /// Function name.
        function: String,
        /// Consumed inputs.
        inputs: Vec<ModClientInput>,
    },
}

/// Input binding scope.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ModInputScope {
    /// Client command.
    ClientCommand,
    /// Movement slice.
    MovementSlice,
}

/// Input binding phase.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ModInputPhase {
    /// Before movement.
    Before,
    /// After movement.
    After,
}

/// Declared client-input binding.
#[derive(Debug, Clone, PartialEq)]
pub struct ModClientInputBinding {
    /// Scope.
    pub scope: ModInputScope,
    /// Phase.
    pub phase: ModInputPhase,
    /// Calls.
    pub calls: Vec<ModSourceCall>,
    /// Before-phase outputs.
    pub outputs: Vec<ModQcInputOutput>,
}

/// Client movement mode output.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ModClientMovementMode {
    /// Normal movement.
    Normal,
    /// Noclip movement.
    Noclip,
    /// Frozen movement.
    Freeze,
}

/// Movement-mode mapping value.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ModMovementModeValue {
    /// Original scalar.
    pub value: f64,
    /// Movement mode.
    pub mode: ModClientMovementMode,
}

/// Stance mapping value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModStanceValue {
    /// Original scalar.
    pub value: f64,
    /// Crouched flag.
    pub crouched: bool,
}

/// Declared client-output mapping.
#[derive(Debug, Clone, PartialEq)]
pub enum ModClientOutput {
    /// Body shape from bounds fields.
    BodyShape {
        /// Minimum field.
        min: String,
        /// Maximum field.
        max: String,
    },
    /// View offset from a vector field.
    ViewOffsetField {
        /// Field name.
        field: String,
    },
    /// View offset from a height scalar.
    ViewOffsetHeight {
        /// Height field.
        height: String,
    },
    /// Movement mode from a masked scalar.
    MovementMode {
        /// Field name.
        field: String,
        /// Bit mask.
        mask: Option<i32>,
        /// Mapping values.
        values: Vec<ModMovementModeValue>,
    },
    /// Stance from a masked scalar.
    Stance {
        /// Field name.
        field: String,
        /// Bit mask.
        mask: Option<i32>,
        /// Mapping values.
        values: Vec<ModStanceValue>,
    },
}

/// Declared client lifecycle.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ModClientDeclaration {
    /// Output mappings.
    pub outputs: Vec<ModClientOutput>,
    /// Maximum clients.
    pub maximum: i32,
    /// Admit calls.
    pub admit: Vec<ModSourceCall>,
    /// Userinfo calls.
    pub userinfo: Vec<ModSourceCall>,
    /// Disconnect calls.
    pub disconnect: Vec<ModSourceCall>,
    /// Per-frame calls.
    pub frame: Vec<ModSourceCall>,
    /// Input bindings.
    pub input: Vec<ModClientInputBinding>,
}

// ---------------------------------------------------------------------------
// Items
// ---------------------------------------------------------------------------

/// Declared item icon.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModItemIconDeclaration {
    /// Image path.
    Image {
        /// Image path.
        path: String,
    },
    /// WAD picture lump.
    WadPicture {
        /// WAD path.
        path: String,
        /// Lump name.
        lump: String,
    },
    /// Shader name.
    Shader {
        /// Shader name.
        name: String,
    },
}

/// Item admission policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ItemAdmission {
    /// Add the item.
    #[default]
    Add,
    /// Replace the primary.
    ReplacePrimary,
}

/// Item action.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SourceItemAction {
    /// Use the item.
    Use,
    /// Drop the item.
    Drop,
}

/// Declared item actions.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ModItemActions {
    /// Use call.
    pub use_call: Option<ModSourceCall>,
    /// Drop call.
    pub drop_call: Option<ModSourceCall>,
}

/// Declared action names in canonical order.
#[must_use]
pub fn source_item_action_names(actions: &ModItemActions) -> Vec<SourceItemAction> {
    let mut names = Vec::new();
    if actions.use_call.is_some() {
        names.push(SourceItemAction::Use);
    }
    if actions.drop_call.is_some() {
        names.push(SourceItemAction::Drop);
    }
    names
}

/// Item kind.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModItemKind {
    /// Stacked counter.
    Counter,
    /// Weapon with optional ammo.
    Weapon {
        /// Ammo item.
        ammo: Option<ItemId>,
    },
}

/// Declared item definition.
#[derive(Debug, Clone, PartialEq)]
pub struct ModItemDefinition {
    /// Item identifier.
    pub item: ItemId,
    /// Display label.
    pub label: String,
    /// Icon declaration.
    pub icon: Option<Option<ModItemIconDeclaration>>,
    /// Admission policy.
    pub admission: ItemAdmission,
    /// Item kind.
    pub kind: ModItemKind,
    /// Declared actions.
    pub actions: Option<ModItemActions>,
}

/// Counter capacity source.
#[derive(Debug, Clone, PartialEq)]
pub enum ModItemCapacity {
    /// Fixed capacity.
    Constant {
        /// Capacity value.
        value: f64,
    },
    /// Capacity field.
    Field {
        /// Field name.
        field: String,
    },
}

/// Packed-inventory bit entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModPackedItem {
    /// Item identifier.
    pub item: ItemId,
    /// Bit mask.
    pub mask: i32,
}

/// Declared item storage.
#[derive(Debug, Clone, PartialEq)]
pub enum ModItemStorage {
    /// Counter field.
    Counter {
        /// Count field.
        field: String,
        /// Item identifier.
        item: ItemId,
        /// Capacity source.
        capacity: ModItemCapacity,
    },
    /// Packed bit field.
    Bits {
        /// Storage field.
        field: String,
        /// Private bit mask.
        private_mask: i32,
        /// Packed items.
        items: Vec<ModPackedItem>,
    },
}

/// Weapon selector value.
#[derive(Debug, Clone, PartialEq)]
pub struct ModWeaponSelectorValue {
    /// Original scalar.
    pub value: f64,
    /// Selected item.
    pub item: ItemId,
}

/// Weapon selector mapping.
#[derive(Debug, Clone, PartialEq)]
pub struct ModWeaponSelector {
    /// Selector field.
    pub field: String,
    /// Mapping values.
    pub values: Vec<ModWeaponSelectorValue>,
}

/// Weapon select mapping with its call.
#[derive(Debug, Clone, PartialEq)]
pub struct ModWeaponSelect {
    /// Selector field.
    pub field: String,
    /// Mapping values.
    pub values: Vec<ModWeaponSelectorValue>,
    /// Select call.
    pub call: ModSourceCall,
}

/// Weapon model fields.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModWeaponModel {
    /// Model field.
    pub field: String,
    /// Frame field.
    pub frame: String,
}

/// Declared weapon consumer.
#[derive(Debug, Clone, PartialEq)]
pub struct ModWeapons {
    /// Weapon stage.
    pub stage: QcWeaponStageDeclaration,
    /// Selected-weapon mapping.
    pub selected: ModWeaponSelector,
    /// Select-weapon mapping.
    pub select: ModWeaponSelect,
    /// Resume calls.
    pub resume: Vec<ModSourceCall>,
    /// Model fields.
    pub model: ModWeaponModel,
}

/// Declared source items.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ModQcItems {
    /// Item definitions.
    pub definitions: Vec<ModItemDefinition>,
    /// Item storage.
    pub storage: Vec<ModItemStorage>,
    /// Weapon consumer.
    pub weapons: Option<ModWeapons>,
}

// ---------------------------------------------------------------------------
// Objectives, presentation, cvars, and callbacks
// ---------------------------------------------------------------------------

/// Objective storage selector.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QcModObjectiveStorage {
    /// Global name.
    Global(String),
    /// Entity-field path.
    EntityField {
        /// Root global.
        global: String,
        /// Indirection fields.
        indirections: Vec<String>,
        /// Leaf field.
        field: String,
    },
}

/// Objective scalar mapping.
#[derive(Debug, Clone, PartialEq)]
pub struct SourceObjectiveValue {
    /// Original scalar.
    pub value: f64,
    /// Stage name.
    pub stage: String,
    /// Completion flag.
    pub complete: bool,
}

/// Objective state storage.
#[derive(Debug, Clone, PartialEq)]
pub struct QcObjectiveState {
    /// Storage selector.
    pub storage: QcModObjectiveStorage,
    /// Scalar mapping.
    pub values: Vec<SourceObjectiveValue>,
}

/// Objective declaration role.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QcObjectiveRole {
    /// Owned objective.
    Owned {
        /// Campaign-gate flag.
        campaign_gate: bool,
        /// Bot-goal flag.
        bot_goal: bool,
        /// Change call.
        change: Option<ModSourceCall>,
    },
    /// Borrowed objective.
    Borrowed {
        /// Writable flag.
        writable: bool,
    },
}

/// Declared source objective.
#[derive(Debug, Clone, PartialEq)]
pub struct QcObjectiveDeclaration {
    /// Objective identifier.
    pub id: String,
    /// State storage.
    pub state: QcObjectiveState,
    /// Carrier storage.
    pub carrier: Option<QcModObjectiveStorage>,
    /// Target storage.
    pub target: Option<QcModObjectiveStorage>,
    /// Role.
    pub role: QcObjectiveRole,
}

/// Client HUD presentation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum QcClientHud {
    /// No HUD ownership.
    #[default]
    None,
    /// Replace vitals.
    ReplaceVitals,
}

/// Client view presentation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum QcClientView {
    /// No view ownership.
    #[default]
    None,
    /// Set-view camera.
    SetView,
}

/// Declared client presentation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct QcModClientPresentation {
    /// HUD mode.
    pub hud: QcClientHud,
    /// View mode.
    pub view: QcClientView,
}

/// Declared console variable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModCvar {
    /// Variable name.
    pub name: String,
    /// Default value.
    pub value: String,
}

/// Declared program artifact.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModProgramRef {
    /// Artifact path.
    pub path: String,
    /// Artifact digest.
    pub digest: ContentDigest,
}

/// Callback operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ModCallbackOperation {
    /// Actor think.
    ActorThink,
    /// Actor touch.
    ActorTouch,
    /// Actor use.
    ActorUse,
    /// Actor pain.
    ActorPain,
    /// Actor death.
    ActorDie,
    /// Damage.
    Damage,
    /// Inventory give.
    InventoryGive,
    /// Inventory consume.
    InventoryConsume,
}

/// Callback stage.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ModCallbackStage {
    /// Observe only.
    Observe,
    /// Transform the result.
    Transform,
    /// Replace the operation.
    Replace,
}

/// Callback result selector.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ModCallbackResult {
    /// Damage amount.
    Amount,
    /// Knockback scalar.
    Knockback,
    /// Boolean handled flag.
    Boolean,
}

/// Callback binding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModCallbackBinding {
    /// Callback identifier.
    pub id: String,
    /// Operation.
    pub operation: ModCallbackOperation,
    /// Stage.
    pub stage: ModCallbackStage,
    /// Result selector.
    pub result: Option<ModCallbackResult>,
}

/// Declared callback.
#[derive(Debug, Clone, PartialEq)]
pub struct ModCallback {
    /// Binding.
    pub binding: ModCallbackBinding,
    /// Source call.
    pub call: ModSourceCall,
}

/// Fold ASCII uppercase to lowercase (mirrors `asciiFold`).
#[must_use]
pub fn ascii_fold(text: &str) -> String {
    text.bytes()
        .map(|byte| if byte.is_ascii_uppercase() { byte + 32 } else { byte })
        .map(char::from)
        .collect()
}

/// Split command text into tokens. This mirrors the `tokenizeCommand` behavior
/// the provider relies on: whitespace separation with double-quote grouping
/// and backslash escapes inside quotes.
#[must_use]
pub fn split_command_tokens(text: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut in_token = false;
    let mut in_quotes = false;
    let mut chars = text.chars().peekable();
    while let Some(char) = chars.next() {
        if in_quotes {
            if char == '"' {
                in_quotes = false;
            } else if char == '\\' {
                if let Some(next) = chars.next() {
                    current.push(next);
                }
            } else {
                current.push(char);
            }
            continue;
        }
        if char.is_whitespace() {
            if in_token {
                tokens.push(std::mem::take(&mut current));
                in_token = false;
            }
        } else if char == '"' {
            in_quotes = true;
            in_token = true;
        } else {
            current.push(char);
            in_token = true;
        }
    }
    if in_token {
        tokens.push(current);
    }
    tokens
}

/// Source-call validator supplied by the `source-call` owner.
pub type SourceCallValidator<'a> = dyn Fn(&ModSourceCall, &[ModCallbackInput], &str) -> Result<(), GuestError> + 'a;

/// Validate a gameplay-mod declaration against its program.
pub fn validate_qc_mod(
    program: &dyn QcProgramView,
    declaration: &ModCallbackDeclaration,
    validate_call: &SourceCallValidator<'_>,
) -> Result<(), GuestError> {
    let program_ref = declaration
        .program
        .as_ref()
        .ok_or_else(|| GuestError::invalid("Gameplay mod declaration requires its program artifact"))?;
    if program.digest() != program_ref.digest {
        return Err(GuestError::invalid(
            "Gameplay mod program differs from its declared artifact digest",
        ));
    }
    validate_client_outputs(program, declaration)?;
    validate_protection_storage(program, declaration)?;
    validate_item_storage(program, declaration)?;
    validate_pickups(program, declaration, validate_call)?;
    validate_actor_fields(program, declaration)?;
    validate_client_presentation(program, declaration)?;
    validate_clients(program, declaration, validate_call)?;
    validate_objectives(program, declaration, validate_call)?;
    validate_commands(program, declaration, validate_call)?;
    validate_callbacks(declaration, validate_call)?;
    if declaration.combat.is_some() {
        validate_combat_declaration(program, declaration)?;
    }
    Ok(())
}

/// Validate the client-output field mappings.
fn validate_client_outputs(
    program: &dyn QcProgramView,
    declaration: &ModCallbackDeclaration,
) -> Result<(), GuestError> {
    let clients = match declaration.clients.as_ref() {
        Some(clients) => clients,
        None => return Ok(()),
    };
    let output_field = |name: &str, vector: bool| -> Result<(), GuestError> {
        let field = program.field_type(name);
        let binding = declaration.actor_fields.iter().find(|value| value.field == name);
        let expected = if vector {
            QcValueType::Vector
        } else {
            QcValueType::Float
        };
        let bound = matches!(
            binding.map(|value| &value.binding),
            Some(ModActorBinding::Private)
                | Some(ModActorBinding::ViewOffset)
                | Some(ModActorBinding::BoundsMin)
                | Some(ModActorBinding::BoundsMax)
                | Some(ModActorBinding::ClientFlags { .. })
        );
        let vector_ok = !vector
            || matches!(
                binding.map(|value| &value.binding),
                Some(ModActorBinding::Private)
                    | Some(ModActorBinding::ViewOffset)
                    | Some(ModActorBinding::BoundsMin)
                    | Some(ModActorBinding::BoundsMax)
            );
        let scalar_ok = vector
            || matches!(
                binding.map(|value| &value.binding),
                Some(ModActorBinding::Private) | Some(ModActorBinding::ClientFlags { .. })
            );
        if field != Some(expected) || !bound || !vector_ok || !scalar_ok {
            return Err(GuestError::invalid(
                "QC client outputs require declared private fields or the explicit view-offset field",
            ));
        }
        Ok(())
    };
    for output in &clients.outputs {
        match output {
            ModClientOutput::BodyShape { min, max } => {
                output_field(min, true)?;
                output_field(max, true)?;
                let min_binding = declaration.actor_fields.iter().find(|value| value.field == *min);
                let max_binding = declaration.actor_fields.iter().find(|value| value.field == *max);
                if !matches!(
                    min_binding.map(|value| &value.binding),
                    Some(ModActorBinding::BoundsMin)
                ) || !matches!(
                    max_binding.map(|value| &value.binding),
                    Some(ModActorBinding::BoundsMax)
                ) {
                    return Err(GuestError::invalid(
                        "Client body shape must name its declared source mins/maxs",
                    ));
                }
            }
            ModClientOutput::ViewOffsetField { field } => output_field(field, true)?,
            ModClientOutput::ViewOffsetHeight { height } => {
                output_field(height, false)?;
                let binding = declaration.actor_fields.iter().find(|value| value.field == *height);
                if matches!(
                    binding.map(|value| &value.binding),
                    Some(ModActorBinding::ClientFlags { .. })
                ) {
                    return Err(GuestError::invalid(
                        "QC client flag outputs must name an explicit mask of source-private bits",
                    ));
                }
            }
            ModClientOutput::MovementMode { field, .. } | ModClientOutput::Stance { field, .. } => {
                output_field(field, false)?;
            }
        }
        if let ModClientOutput::MovementMode { field, mask, .. } | ModClientOutput::Stance { field, mask, .. } = output
        {
            let binding = declaration.actor_fields.iter().find(|value| value.field == *field);
            if let Some(ModActorBinding::ClientFlags { private_mask, .. }) = binding.map(|value| &value.binding) {
                let mask = mask.unwrap_or(0);
                if mask == 0 || (mask & !(private_mask.unwrap_or(0))) != 0 {
                    return Err(GuestError::invalid(
                        "QC client flag outputs must name an explicit mask of source-private bits",
                    ));
                }
            }
        }
    }
    Ok(())
}

/// Validate protection declarations via the shared region check.
fn validate_protection_storage(
    program: &dyn QcProgramView,
    declaration: &ModCallbackDeclaration,
) -> Result<(), GuestError> {
    super::mod_protection::qc_protection_regions(program, declaration).map(|_| ())}

#[allow(dead_code)]
fn replaced_validate_protection_storage_body(
    program: &dyn QcProgramView,
    declaration: &ModCallbackDeclaration,
) -> Result<(), GuestError> {
    use std::collections::HashSet;
    let mut channels = HashSet::new();
    for definition in &declaration.protection {
        if declaration.clients.is_none() || !channels.insert(definition.channel) {
            return Err(GuestError::invalid(
                "QC protection requires clients and one declaration per channel",
            ));
        }
        let (count, selection) = match (&definition.channel, &definition.regular, &definition.powered) {
            (ProtectionChannel::Regular, Some(storage), _) => (
                storage.points.clone(),
                storage.selection.as_ref().map(|selection| {
                    (
                        selection.field.clone(),
                        selection.mask,
                        selection.values.iter().map(|value| value.value).collect::<Vec<_>>(),
                    )
                }),
            ),
            (ProtectionChannel::Powered, _, Some(storage)) => (
                storage.cells.clone(),
                storage.selection.as_ref().map(|selection| {
                    (
                        selection.field.clone(),
                        selection.mask,
                        selection.values.iter().map(|value| value.value).collect::<Vec<_>>(),
                    )
                }),
            ),
            _ => return Err(GuestError::invalid("QC protection requires storage for its channel")),
        };
        let mut names = vec![count];
        if let Some((field, _, _)) = &selection {
            names.push(field.clone());
        }
        for name in &names {
            let bound = declaration
                .actor_fields
                .iter()
                .any(|field| field.field == *name && matches!(field.binding, ModActorBinding::Private));
            if program.field_type(name) != Some(QcValueType::Float) || !bound {
                return Err(GuestError::invalid(format!(
                    "QC protection requires private float storage {name}"
                )));
            }
        }
        if let Some((field, mask, values)) = &selection {
            if field == &names[0]
                || values.is_empty()
                || values
                    .iter()
                    .any(|value| !value.is_finite() || f64::from(*value as f32) != *value)
            {
                return Err(GuestError::invalid("QC protection selection is not representable"));
            }
            let mut seen = Vec::with_capacity(values.len());
            for value in values {
                if seen.contains(value) {
                    return Err(GuestError::invalid("QC protection selection is not representable"));
                }
                seen.push(*value);
            }
            if let Some(mask) = mask {
                if *mask <= 0
                    || *mask > 0x7f_ffff
                    || values
                        .iter()
                        .any(|value| value.fract() != 0.0 || (*value as i32 & *mask) != *value as i32)
                {
                    return Err(GuestError::invalid("QC protection selection mask is invalid"));
                }
            }
        }
        let flags = [
            definition.flags.no_armor,
            definition.flags.no_power_armor,
            definition.flags.no_regular_armor,
            definition.flags.energy,
            definition.flags.radius,
        ];
        if flags.iter().any(|mask| *mask < 0 || *mask > 0x7f_ffff) {
            return Err(GuestError::invalid(
                "QC protection flags exceed source integer precision",
            ));
        }
    }
    Ok(())
}

/// Validate item storage declarations (structural half of `validateQcItems`).
fn validate_item_storage(program: &dyn QcProgramView, declaration: &ModCallbackDeclaration) -> Result<(), GuestError> {
    use std::collections::{HashMap, HashSet};
    let items = match declaration.items.as_ref() {
        Some(items) => items,
        None => return Ok(()),
    };
    if declaration.clients.is_none() || items.definitions.is_empty() {
        return Err(GuestError::invalid(
            "QC source items require canonical clients and definitions",
        ));
    }
    let mut definitions = HashMap::new();
    for definition in &items.definitions {
        if definition.label.is_empty() || definitions.insert(definition.item.clone(), definition).is_some() {
            return Err(GuestError::invalid("QC item definitions are empty or duplicated"));
        }
    }
    let field = |name: &str, expected: QcValueType, input: bool| -> Result<(), GuestError> {
        let bound = declaration.actor_fields.iter().any(|field| {
            field.field == name
                && (matches!(field.binding, ModActorBinding::Private)
                    || (input && matches!(field.binding, ModActorBinding::ClientInput { .. })))
        });
        if program.field_type(name) != Some(expected) || !bound {
            return Err(GuestError::invalid(format!(
                "QC item storage {name} requires declared original storage"
            )));
        }
        Ok(())
    };
    let offset_of = |name: &str| -> usize {
        declaration
            .actor_fields
            .iter()
            .position(|field| field.field == name)
            .unwrap_or(usize::MAX)
    };
    let mut bound = HashSet::new();
    let mut words: HashMap<usize, &str> = HashMap::new();
    for storage in &items.storage {
        match storage {
            ModItemStorage::Counter {
                field: name,
                item,
                capacity,
            } => {
                field(name, QcValueType::Float, false)?;
                if words.insert(offset_of(name), "count").is_some() {
                    return Err(GuestError::invalid("QC item storage fields overlap"));
                }
                if !definitions.contains_key(item) || !bound.insert(item.clone()) {
                    return Err(GuestError::invalid(format!(
                        "QC item {item} lacks distinct declared storage"
                    )));
                }
                match capacity {
                    ModItemCapacity::Field { field: capacity } => {
                        field(capacity, QcValueType::Float, false)?;
                        if words.get(&offset_of(capacity)) == Some(&"count") {
                            return Err(GuestError::invalid("QC item capacity overlaps source storage"));
                        }
                        words.insert(offset_of(capacity), "capacity");
                    }
                    ModItemCapacity::Constant { value } => {
                        if !value.is_finite() || *value < 0.0 || f64::from(*value as f32) != *value {
                            return Err(GuestError::invalid("QC item capacity exceeds its source ABI"));
                        }
                    }
                }
            }
            ModItemStorage::Bits {
                field: name,
                private_mask,
                items: packed,
            } => {
                field(name, QcValueType::Float, false)?;
                if words.insert(offset_of(name), "count").is_some() {
                    return Err(GuestError::invalid("QC item storage fields overlap"));
                }
                if *private_mask < 0 || *private_mask > 0xff_ffff || packed.is_empty() {
                    return Err(GuestError::invalid(
                        "QC packed inventory requires an exact binary32 mask",
                    ));
                }
                let mut mask = *private_mask;
                for entry in packed {
                    if !definitions.contains_key(&entry.item) || !bound.insert(entry.item.clone()) {
                        return Err(GuestError::invalid(format!(
                            "QC item {} lacks distinct declared storage",
                            entry.item
                        )));
                    }
                    if entry.mask < 1
                        || entry.mask > 0x80_0000
                        || (entry.mask & (entry.mask - 1)) != 0
                        || (mask & entry.mask) != 0
                    {
                        return Err(GuestError::invalid(
                            "QC packed inventory masks overlap or exceed source precision",
                        ));
                    }
                    mask |= entry.mask;
                }
            }
        }
    }
    if bound.len() != definitions.len() {
        return Err(GuestError::invalid("QC item definition has no source storage"));
    }
    let weapons: Vec<_> = items
        .definitions
        .iter()
        .filter(|definition| matches!(definition.kind, ModItemKind::Weapon { .. }))
        .collect();
    if (weapons.is_empty()) != (items.weapons.is_none()) {
        return Err(GuestError::invalid(
            "QC weapon definitions require their original source consumer",
        ));
    }
    if let Some(consumer) = items.weapons.as_ref() {
        for name in ["think", "nextthink"] {
            let bound = declaration.actor_fields.iter().any(|field| {
                field.field == name && matches!(&field.binding, ModActorBinding::Think | ModActorBinding::Nextthink)
            });
            if !bound {
                return Err(GuestError::invalid(
                    "QC weapons require continuing source think ownership",
                ));
            }
        }
        let mappings = [
            (&consumer.selected.field, &consumer.selected.values, false),
            (&consumer.select.field, &consumer.select.values, true),
        ];
        for (name, values, is_select) in mappings {
            field(name, QcValueType::Float, is_select)?;
            let mut seen_items = HashSet::new();
            let mut seen_values = Vec::new();
            for value in values {
                if !value.value.is_finite() || f64::from(value.value as f32) != value.value || value.value == 0.0 {
                    return Err(GuestError::invalid("QC weapon selectors differ from their definitions"));
                }
                seen_items.insert(value.item.clone());
                if seen_values.contains(&value.value) {
                    return Err(GuestError::invalid("QC weapon selectors differ from their definitions"));
                }
                seen_values.push(value.value);
            }
            if values.len() != weapons.len() || seen_items.len() != weapons.len() {
                return Err(GuestError::invalid("QC weapon selectors differ from their definitions"));
            }
            for weapon in &weapons {
                if !values.iter().any(|value| value.item == weapon.item) {
                    return Err(GuestError::invalid("QC weapon selectors differ from their definitions"));
                }
            }
        }
        field(&consumer.model.field, QcValueType::String, false)?;
        field(&consumer.model.frame, QcValueType::Float, false)?;
        for weapon in &weapons {
            if let ModItemKind::Weapon { ammo: Some(ammo) } = &weapon.kind {
                let declared = definitions.contains_key(ammo)
                    || declaration
                        .actor_fields
                        .iter()
                        .any(|field| matches!(&field.binding, ModActorBinding::Inventory { item } if item == ammo));
                if !declared {
                    return Err(GuestError::invalid("QC weapon has no declared ammo source"));
                }
            }
        }
    }
    Ok(())
}

/// Validate pickup rules and their declared storage.
fn validate_pickups(
    program: &dyn QcProgramView,
    declaration: &ModCallbackDeclaration,
    validate_call: &SourceCallValidator<'_>,
) -> Result<(), GuestError> {
    use std::collections::HashSet;
    let mut ids = HashSet::new();
    let mut offered = HashSet::new();
    for pickup in &declaration.pickups {
        if declaration.clients.is_none()
            || pickup.id.is_empty()
            || !ids.insert(pickup.id.clone())
            || pickup.offered.is_empty()
            || pickup.offered.iter().collect::<HashSet<_>>().len() != pickup.offered.len()
            || pickup.offered.iter().any(|item| offered.contains(item))
        {
            return Err(GuestError::invalid(
                "QC pickups require clients, unique rule ids and distinct offered items",
            ));
        }
        for item in &pickup.offered {
            offered.insert(item.clone());
        }
        if pickup.writes.is_empty() {
            return Err(GuestError::invalid(
                "QC pickup resource requires its declared source storage",
            ));
        }
        for write in &pickup.writes {
            match write {
                PickupWrite::Protection { channel } => {
                    if !declaration.protection.iter().any(|value| value.channel == *channel) {
                        return Err(GuestError::invalid(
                            "QC pickup resource requires its declared source storage",
                        ));
                    }
                }
                PickupWrite::Inventory { item, fields } => {
                    let actor_bound = *fields == PickupFields::Count
                        && declaration.actor_fields.iter().any(|field| {
                            matches!(&field.binding, ModActorBinding::Inventory { item: bound } if bound == item)
                        });
                    let item_bound = declaration.items.as_ref().is_some_and(|items| {
                        items.storage.iter().any(|storage| match storage {
                            ModItemStorage::Bits { items: packed, .. } => {
                                *fields == PickupFields::Count && packed.iter().any(|entry| entry.item == *item)
                            }
                            ModItemStorage::Counter {
                                item: stored, capacity, ..
                            } => {
                                stored == item
                                    && (*fields == PickupFields::Count
                                        || matches!(capacity, ModItemCapacity::Field { .. }))
                            }
                        })
                    });
                    if !(actor_bound || item_bound) {
                        return Err(GuestError::invalid(
                            "QC pickup resource requires its declared source storage",
                        ));
                    }
                }
            }
        }
        let calls: Vec<&ModSourceCall> = match &pickup.operation {
            PickupOperation::BooleanGrant { grant } => vec![grant],
            PickupOperation::GateThenGrant { gate, grant, .. } => vec![gate, grant],
        };
        for call in calls {
            validate_call(
                call,
                &[
                    ModCallbackInput::Self_,
                    ModCallbackInput::Other,
                    ModCallbackInput::Item,
                    ModCallbackInput::Time,
                    ModCallbackInput::PickupCount,
                    ModCallbackInput::PickupHasCount,
                    ModCallbackInput::PickupDropped,
                ],
                "pickup",
            )?;
        }
    }
    Ok(())
}

/// Validate actor-field bindings and their program types.
fn validate_actor_fields(program: &dyn QcProgramView, declaration: &ModCallbackDeclaration) -> Result<(), GuestError> {
    use std::collections::HashSet;
    for binding in ["team", "score"] {
        let count = declaration
            .actor_fields
            .iter()
            .filter(|field| match (&field.binding, binding) {
                (ModActorBinding::Team { .. }, "team") | (ModActorBinding::Score, "score") => true,
                _ => false,
            })
            .count();
        if count > 1 {
            return Err(GuestError::invalid(format!(
                "Duplicate original {binding} authority field"
            )));
        }
    }
    let think = declaration
        .actor_fields
        .iter()
        .filter(|field| matches!(field.binding, ModActorBinding::Think))
        .count();
    let nextthink = declaration
        .actor_fields
        .iter()
        .filter(|field| matches!(field.binding, ModActorBinding::Nextthink))
        .count();
    if think != nextthink || think > 1 {
        return Err(GuestError::invalid(
            "Mod source scheduling requires one think and one nextthink binding together",
        ));
    }
    let mut words = HashSet::new();
    for entry in &declaration.actor_fields {
        match &entry.binding {
            ModActorBinding::Team { .. } | ModActorBinding::Score => validate_source_match_field(&entry.binding)?,
            ModActorBinding::ClientInput { input, update, scale } => {
                if declaration
                    .clients
                    .as_ref()
                    .is_none_or(|clients| clients.input.is_empty())
                    || (*update == ClientInputUpdate::Nonzero && *input == ModClientInput::ViewAngles)
                {
                    return Err(GuestError::invalid(
                        "Mod client input fields require declared applications and scalar nonzero updates",
                    ));
                }
                if let Some(scale) = scale {
                    if !scale.is_finite() || *scale == 0.0 || *input == ModClientInput::ViewAngles {
                        return Err(GuestError::invalid(
                            "QC input scale requires a finite nonzero scalar encoding",
                        ));
                    }
                }
            }
            ModActorBinding::ClientFlags { grounded, private_mask } => {
                if let Some(mask) = private_mask {
                    let reserved = 8 | 128 | (if *grounded { 512 } else { 0 });
                    if *mask < 0 || *mask > 0x7f_ffff || (*mask & reserved) != 0 {
                        return Err(GuestError::invalid(
                            "Mod private client flags overlap canonical flags or exceed the source flag word",
                        ));
                    }
                }
            }
            ModActorBinding::Userinfo { key } => {
                if declaration.clients.is_none() || key.is_empty() || key.contains(['\\', '\0']) {
                    return Err(GuestError::invalid(
                        "Mod userinfo field requires a declared client and valid info key",
                    ));
                }
            }
            _ => {}
        }
        let field = program
            .field_type(&entry.field)
            .ok_or_else(|| GuestError::invalid(format!("Missing mod actor field {}", entry.field)))?;
        let expected = match &entry.binding {
            ModActorBinding::Private => field,
            ModActorBinding::Constant { value } => match value {
                ModConstantValue::Float(_) => QcValueType::Float,
                ModConstantValue::String(_) => QcValueType::String,
                ModConstantValue::Vector(_) => QcValueType::Vector,
            },
            ModActorBinding::ClientInput { input, .. } => {
                if *input == ModClientInput::ViewAngles {
                    QcValueType::Vector
                } else {
                    QcValueType::Float
                }
            }
            ModActorBinding::Classname | ModActorBinding::Userinfo { .. } => QcValueType::String,
            ModActorBinding::Think => QcValueType::Function,
            ModActorBinding::Team { .. }
            | ModActorBinding::Score
            | ModActorBinding::Health
            | ModActorBinding::Inventory { .. }
            | ModActorBinding::Nextthink
            | ModActorBinding::ClientFlags { .. } => QcValueType::Float,
            ModActorBinding::Origin
            | ModActorBinding::Velocity
            | ModActorBinding::Angles
            | ModActorBinding::BoundsMin
            | ModActorBinding::BoundsMax
            | ModActorBinding::ViewOffset => QcValueType::Vector,
        };
        if field != expected {
            return Err(GuestError::invalid(format!(
                "Mod actor field {} requires {expected:?}, found {field:?}",
                entry.field
            )));
        }
        let width = if expected == QcValueType::Vector { 3 } else { 1 };
        // Offsets are unavailable on the view; overlap is checked by field identity order.
        for word in 0..width {
            let key = format!("{}:{word}", entry.field);
            if !words.insert(key) {
                return Err(GuestError::invalid(format!(
                    "Overlapping mod actor field {}",
                    entry.field
                )));
            }
        }
    }
    Ok(())
}

/// Validate client presentation requirements.
fn validate_client_presentation(
    program: &dyn QcProgramView,
    declaration: &ModCallbackDeclaration,
) -> Result<(), GuestError> {
    let presentation = match declaration.client_presentation.as_ref() {
        Some(presentation) => presentation,
        None => return Ok(()),
    };
    if declaration.clients.is_none() {
        return Err(GuestError::invalid("QC client presentation requires declared clients"));
    }
    if presentation.hud != QcClientHud::None {
        for name in ["health", "armorvalue"] {
            if program.field_type(name) != Some(QcValueType::Float) {
                return Err(GuestError::invalid(format!("QC status requires original {name}")));
            }
        }
    }
    if presentation.view != QcClientView::None {
        for name in ["origin", "angles", "view_ofs"] {
            if program.field_type(name) != Some(QcValueType::Vector) {
                return Err(GuestError::invalid(format!("QC view requires original {name}")));
            }
        }
    }
    Ok(())
}

/// Validate client lifecycle declarations.
fn validate_clients(
    program: &dyn QcProgramView,
    declaration: &ModCallbackDeclaration,
    validate_call: &SourceCallValidator<'_>,
) -> Result<(), GuestError> {
    use std::collections::HashSet;
    let clients = match declaration.clients.as_ref() {
        Some(clients) => clients,
        None => return Ok(()),
    };
    if clients.maximum < 1 || clients.maximum >= 8191 {
        return Err(GuestError::invalid(
            "Mod client capacity must fit reserved QuakeC edicts",
        ));
    }
    for call in clients.admit.iter().chain(&clients.userinfo).chain(&clients.disconnect) {
        validate_call(
            call,
            &[ModCallbackInput::Self_, ModCallbackInput::Time],
            "client lifecycle",
        )?;
    }
    for call in &clients.frame {
        validate_call(
            call,
            &[
                ModCallbackInput::Self_,
                ModCallbackInput::Time,
                ModCallbackInput::Elapsed,
            ],
            "client frame",
        )?;
    }
    for binding in &clients.input {
        if binding.phase == ModInputPhase::Before {
            let mut outputs = HashSet::new();
            for output in &binding.outputs {
                let key = match output {
                    ModQcInputOutput::Field { field } => format!("field:{field}"),
                    ModQcInputOutput::Handler { function, .. } => format!("handler:{function}"),
                };
                if !outputs.insert(key) {
                    return Err(GuestError::invalid("QC input output declaration is duplicated"));
                }
                match output {
                    ModQcInputOutput::Field { field } => {
                        let bound = declaration.actor_fields.iter().any(|entry| {
                            entry.field == *field && matches!(entry.binding, ModActorBinding::ClientInput { .. })
                        });
                        if !bound {
                            return Err(GuestError::invalid("QC output requires declared client input storage"));
                        }
                    }
                    ModQcInputOutput::Handler { function, inputs } => {
                        let target = program.function_named(function);
                        let distinct = inputs.iter().collect::<HashSet<_>>().len() == inputs.len();
                        let valid = target.as_ref().is_some_and(|target| {
                            target.index != 0 && target.first_statement > 0 && !target.named_builtin
                        }) && program.global_type("self") == Some(QcValueType::Entity)
                            && !inputs.is_empty()
                            && distinct;
                        if !valid {
                            return Err(GuestError::invalid(
                                "QC output requires an original actor handler and distinct controls",
                            ));
                        }
                    }
                }
            }
        }
        for call in &binding.calls {
            validate_call(
                call,
                &[
                    ModCallbackInput::Self_,
                    ModCallbackInput::Time,
                    ModCallbackInput::Elapsed,
                    ModCallbackInput::ViewAngles,
                    ModCallbackInput::Attack,
                    ModCallbackInput::Jump,
                    ModCallbackInput::Impulse,
                    ModCallbackInput::ForwardMove,
                    ModCallbackInput::SideMove,
                    ModCallbackInput::UpMove,
                ],
                "client input",
            )?;
        }
    }
    Ok(())
}

/// Validate objective storage declarations.
fn validate_objectives(
    program: &dyn QcProgramView,
    declaration: &ModCallbackDeclaration,
    validate_call: &SourceCallValidator<'_>,
) -> Result<(), GuestError> {
    for objective in &declaration.objectives {
        validate_objective_storage(program, &objective.state.storage, QcValueType::Float)?;
        for storage in [&objective.carrier, &objective.target].into_iter().flatten() {
            validate_objective_storage(program, storage, QcValueType::Entity)?;
        }
        if let QcObjectiveRole::Owned {
            change: Some(change), ..
        } = &objective.role
        {
            validate_call(
                change,
                &[
                    ModCallbackInput::Self_,
                    ModCallbackInput::Other,
                    ModCallbackInput::Activator,
                    ModCallbackInput::Amount,
                    ModCallbackInput::Time,
                ],
                "objective change",
            )?;
        }
    }
    Ok(())
}

/// Validate one objective storage selector.
fn validate_objective_storage(
    program: &dyn QcProgramView,
    storage: &QcModObjectiveStorage,
    expected: QcValueType,
) -> Result<(), GuestError> {
    match storage {
        QcModObjectiveStorage::Global(name) => {
            if program.global_type(name) != Some(expected) {
                return Err(GuestError::invalid(format!(
                    "Objective global {name} requires original {expected:?} storage"
                )));
            }
        }
        QcModObjectiveStorage::EntityField {
            global,
            indirections,
            field,
        } => {
            if program.global_type(global) != Some(QcValueType::Entity) {
                return Err(GuestError::invalid(
                    "Objective field requires an original global entity reference",
                ));
            }
            for name in indirections {
                if program.field_type(name) != Some(QcValueType::Entity) {
                    return Err(GuestError::invalid(format!(
                        "Objective selector {name} requires an original entity field"
                    )));
                }
            }
            if program.field_type(field) != Some(expected) {
                return Err(GuestError::invalid(format!(
                    "Objective field {field} requires original {expected:?} storage"
                )));
            }
        }
    }
    Ok(())
}

/// Validate console commands.
fn validate_commands(
    _program: &dyn QcProgramView,
    declaration: &ModCallbackDeclaration,
    validate_call: &SourceCallValidator<'_>,
) -> Result<(), GuestError> {
    use std::collections::HashSet;
    let mut commands = HashSet::new();
    for command in &declaration.commands {
        let name = ascii_fold(&command.name);
        let tokens = split_command_tokens(&command.name);
        if tokens.len() != 1
            || tokens.first().is_none_or(|token| *token != command.name)
            || command.name.contains(';')
            || !commands.insert(name)
        {
            return Err(GuestError::invalid(format!(
                "Invalid or duplicate mod command {}",
                command.name
            )));
        }
        let call = ModSourceCall {
            function: command.function.clone(),
            arguments: command
                .arguments
                .iter()
                .map(|value| match value {
                    ModConsoleValue::Float(value) => ModCallbackValue::Float(*value),
                    ModConsoleValue::String(value) => ModCallbackValue::String(value.clone()),
                    ModConsoleValue::Vector(value) => ModCallbackValue::Vector(*value),
                    ModConsoleValue::Argument { arg_type, .. } => match arg_type {
                        ModConsoleArgType::String => ModCallbackValue::String(String::new()),
                        ModConsoleArgType::Float => ModCallbackValue::Float(0.0),
                    },
                    ModConsoleValue::ArgumentsText => ModCallbackValue::String(String::new()),
                    ModConsoleValue::ArgumentCount => ModCallbackValue::Float(0.0),
                })
                .collect(),
            globals: command
                .globals
                .iter()
                .map(|global| ModSourceGlobal {
                    name: global.name.clone(),
                    value: match &global.value {
                        ModConsoleValue::Float(value) => ModCallbackValue::Float(*value),
                        ModConsoleValue::String(value) => ModCallbackValue::String(value.clone()),
                        ModConsoleValue::Vector(value) => ModCallbackValue::Vector(*value),
                        ModConsoleValue::Argument { arg_type, .. } => match arg_type {
                            ModConsoleArgType::String => ModCallbackValue::String(String::new()),
                            ModConsoleArgType::Float => ModCallbackValue::Float(0.0),
                        },
                        ModConsoleValue::ArgumentsText => ModCallbackValue::String(String::new()),
                        ModConsoleValue::ArgumentCount => ModCallbackValue::Float(0.0),
                    },
                })
                .collect(),
        };
        validate_call(&call, &[], &format!("console command {}", command.name))?;
    }
    for call in &declaration.initialize {
        validate_call(
            call,
            &[ModCallbackInput::Self_, ModCallbackInput::Time],
            "initialization",
        )?;
    }
    if let Some(frame) = declaration.frame.as_ref() {
        validate_call(
            frame,
            &[
                ModCallbackInput::Self_,
                ModCallbackInput::Time,
                ModCallbackInput::Elapsed,
            ],
            "source frame",
        )?;
    }
    let mut cvars = std::collections::HashSet::new();
    for variable in &declaration.cvars {
        if !cvars.insert(variable.name.clone()) {
            return Err(GuestError::invalid(format!("Duplicate mod cvar {}", variable.name)));
        }
    }
    Ok(())
}

/// Validate declared callbacks.
fn validate_callbacks(
    declaration: &ModCallbackDeclaration,
    validate_call: &SourceCallValidator<'_>,
) -> Result<(), GuestError> {
    use std::collections::HashSet;
    let mut callbacks = HashSet::new();
    for callback in &declaration.callbacks {
        if !callbacks.insert(callback.binding.id.clone()) {
            return Err(GuestError::invalid(format!(
                "Duplicate mod callback {}",
                callback.binding.id
            )));
        }
        let mut available = vec![ModCallbackInput::Self_, ModCallbackInput::Time];
        if callback.binding.stage == ModCallbackStage::Observe {
            available.push(ModCallbackInput::Result);
        }
        let additional: &[ModCallbackInput] = match callback.binding.operation {
            ModCallbackOperation::Damage => &[
                ModCallbackInput::Attacker,
                ModCallbackInput::Inflictor,
                ModCallbackInput::Amount,
                ModCallbackInput::Knockback,
                ModCallbackInput::Direction,
                ModCallbackInput::Point,
                ModCallbackInput::Normal,
            ],
            ModCallbackOperation::InventoryGive | ModCallbackOperation::InventoryConsume => {
                &[ModCallbackInput::Item, ModCallbackInput::Amount]
            }
            ModCallbackOperation::ActorUse => &[ModCallbackInput::Other, ModCallbackInput::Activator],
            ModCallbackOperation::ActorTouch => &[ModCallbackInput::Other],
            ModCallbackOperation::ActorThink => &[ModCallbackInput::Elapsed],
            ModCallbackOperation::ActorPain => &[
                ModCallbackInput::Attacker,
                ModCallbackInput::Amount,
                ModCallbackInput::Knockback,
            ],
            ModCallbackOperation::ActorDie => &[
                ModCallbackInput::Attacker,
                ModCallbackInput::Inflictor,
                ModCallbackInput::Amount,
                ModCallbackInput::Knockback,
                ModCallbackInput::Point,
            ],
        };
        available.extend_from_slice(additional);
        validate_call(&callback.call, &available, &callback.binding.id)?;
    }
    Ok(())
}

/// Validate the combat declaration (structural half of `validateQcModCombat`).
fn validate_combat_declaration(
    program: &dyn QcProgramView,
    declaration: &ModCallbackDeclaration,
) -> Result<(), GuestError> {
    let combat = declaration
        .combat
        .as_ref()
        .ok_or_else(|| GuestError::invalid("Missing combat declaration"))?;
    if let Some(empty) = combat.empty_armor.as_ref() {
        if !["q1:item_armor1", "q1:item_armor2", "q1:item_armorInv"].contains(&empty.item.as_str())
            || !empty.absorption.is_finite()
            || f64::from(empty.absorption as f32) != empty.absorption
            || empty.absorption < 0.0
        {
            return Err(GuestError::invalid(
                "QC points-only armor requires an authored item and finite nonnegative absorption",
            ));
        }
    }
    if let Some(stage) = combat.armor_stage.as_ref() {
        validate_armor_stage_shape(stage)?;
    }
    if let Some(scale) = combat.damage_scale.as_ref() {
        if program.function_named(&scale.function).is_none() || scale.entry >= scale.exit {
            return Err(GuestError::invalid("QC damage scale requires a valid source region"));
        }
    }
    for name in [
        "health",
        "takedamage",
        "flags",
        "invincible_finished",
        "armorvalue",
        "armortype",
    ] {
        if program.field_type(name) != Some(QcValueType::Float) {
            return Err(GuestError::invalid(format!("QC combat requires float field {name}")));
        }
    }
    if program.function_named(&combat.damage.function).is_none() {
        return Err(GuestError::invalid("QC combat requires its declared damage function"));
    }
    Ok(())
}

/// Validate armor-stage region shape.
fn validate_armor_stage_shape(stage: &ModQcArmorStage) -> Result<(), GuestError> {
    if stage.entry >= stage.exit || stage.target < 0 || stage.damage < 0 || stage.saved < 0 {
        return Err(GuestError::invalid("QC armor stage requires a valid source region"));
    }
    if let ModQcArmorStageFlags::Bits { word, .. } = stage.flags {
        if word < 0 {
            return Err(GuestError::invalid("QC armor stage requires a valid source region"));
        }
    }
    Ok(())
}

/// Declaration format version.
pub const MOD_CALLBACK_DECLARATION_VERSION: u32 = 1;

// ---------------------------------------------------------------------------
// Mod identity and checkpoints (absorbed from `src/contracts/mods.ts`)
// ---------------------------------------------------------------------------

/// Selected package and authored component.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ModSelection {
    /// Package name.
    pub product: String,
    /// Component identifier.
    pub id: String,
}

/// Source provenance reference.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ProviderReference {
    /// Owning provider.
    pub provider: ProviderId,
    /// Content identifier.
    pub content: ContentId,
}

/// Mod purpose.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ModPurpose {
    /// Standalone game type.
    GameType,
    /// Addition to a game.
    Addition,
}

/// Mod availability.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModAvailability {
    /// Available for play.
    Available,
    /// Unavailable with a reason.
    Unavailable {
        /// Reason text.
        reason: String,
    },
}

/// Described gameplay mod.
#[derive(Debug, Clone, PartialEq)]
pub struct ModDescription {
    /// Selection.
    pub selection: ModSelection,
    /// Source reference.
    pub source: ProviderReference,
    /// Display title.
    pub title: String,
    /// Source title.
    pub source_title: String,
    /// Purpose.
    pub purpose: ModPurpose,
    /// Required mods.
    pub requires: Vec<ModSelection>,
    /// Conflicting mods.
    pub conflicts: Vec<ModSelection>,
    /// Availability.
    pub availability: ModAvailability,
}

/// Resolved gameplay mod with its QC declaration.
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedGameplayMod {
    /// Selection.
    pub selection: ModSelection,
    /// Source reference.
    pub source: ProviderReference,
    /// Display title.
    pub title: String,
    /// Source title.
    pub source_title: String,
    /// Required mods.
    pub requires: Vec<ModSelection>,
    /// Conflicting mods.
    pub conflicts: Vec<ModSelection>,
    /// Callback declaration.
    pub declaration: ModCallbackDeclaration,
    /// Declaration digest.
    pub declaration_digest: ContentDigest,
}

/// Guest module identity.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ModuleIdentity {
    /// Module identifier.
    pub id: ModuleId,
    /// Artifact path.
    pub artifact_path: String,
    /// Artifact digest.
    pub digest: ContentDigest,
    /// Artifact revision.
    pub revision: u32,
}

/// Provider checkpoint header (without bytes).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ProviderCheckpointRef {
    /// Provider.
    pub provider: ProviderId,
    /// Schema name.
    pub schema: String,
    /// Schema version.
    pub version: u32,
}

/// Guest checkpoint header.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct GuestCheckpointRef {
    /// Module identifier.
    pub module: ModuleId,
    /// Checkpoint format.
    pub format: String,
}

/// Mod instance identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModIdentity {
    /// Selection.
    pub selection: ModSelection,
    /// Source reference.
    pub source: ProviderReference,
    /// Declaration digest.
    pub declaration_digest: ContentDigest,
    /// Module identities.
    pub modules: Vec<ModuleIdentity>,
    /// Provider checkpoints.
    pub providers: Vec<ProviderCheckpointRef>,
}

/// Private per-instance mod state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModPrivateCheckpoint {
    /// Guest checkpoints.
    pub guests: Vec<GuestCheckpointRef>,
    /// Provider checkpoints.
    pub providers: Vec<ProviderCheckpointRef>,
}

/// Saved mod instance.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModCheckpoint {
    /// Identity.
    pub identity: ModIdentity,
    /// State.
    pub state: ModPrivateCheckpoint,
}

/// Saved mod session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModSessionCheckpoint {
    /// Checkpoint version (always 1).
    pub version: u32,
    /// Saved mods.
    pub mods: Vec<ModCheckpoint>,
}

/// Saved mod travel state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModTravelCheckpoint {
    /// Checkpoint version (always 1).
    pub version: u32,
    /// Saved mods with optional state.
    pub mods: Vec<ModTravelEntry>,
}

/// One travel entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModTravelEntry {
    /// Identity.
    pub identity: ModIdentity,
    /// State (null across travel boundaries).
    pub state: Option<ModPrivateCheckpoint>,
}

/// Whether a product name is valid.
fn valid_product(product: &str) -> bool {
    let mut chars = product.chars();
    match chars.next() {
        Some(first) if first.is_ascii_alphanumeric() => {}
        _ => return false,
    }
    chars.all(|char| char.is_ascii_alphanumeric() || matches!(char, '.' | '_' | '+' | '-'))
}

/// Whether a component identifier is valid.
fn valid_component(id: &str) -> bool {
    let mut chars = id.chars();
    match chars.next() {
        Some(first) if first.is_ascii_alphanumeric() => {}
        _ => return false,
    }
    chars.all(|char| char.is_ascii_alphanumeric() || matches!(char, '.' | '_' | '+' | ':' | '/' | '-'))
}

/// Canonical `PRODUCT/COMPONENT_ID` key.
pub fn mod_selection_key(selection: &ModSelection) -> Result<String, GuestError> {
    if !valid_product(&selection.product) || !valid_component(&selection.id) {
        return Err(GuestError::invalid(
            "Mod selection requires a package and an authored component ID",
        ));
    }
    Ok(format!("{}/{}", selection.product, selection.id))
}

/// Parse a canonical selection key.
pub fn read_mod_selection(value: &str) -> Result<ModSelection, GuestError> {
    let slash = value.find('/').unwrap_or(0);
    let selection = ModSelection {
        product: value[..slash].to_string(),
        id: value[slash + 1.min(value.len())..].to_string(),
    };
    if slash < 1 || mod_selection_key(&selection).as_deref() != Ok(value) {
        return Err(GuestError::invalid("Mod selection must be PRODUCT/COMPONENT_ID"));
    }
    Ok(selection)
}

/// Percent-encode a selection key the way `encodeURIComponent` does.
fn percent_encode_key(key: &str) -> String {
    let mut encoded = String::with_capacity(key.len());
    for byte in key.bytes() {
        let char = byte as char;
        if char.is_ascii_alphanumeric() || matches!(char, '-' | '_' | '.' | '!' | '~' | '*' | '\'' | '(' | ')') {
            encoded.push(char);
        } else {
            encoded.push_str(&format!("%{byte:02X}"));
        }
    }
    encoded
}

/// Provider identifier for a mod instance.
pub fn mod_instance_provider(selection: &ModSelection) -> Result<ProviderId, GuestError> {
    Ok(ProviderId::new(
        "mod",
        &percent_encode_key(&mod_selection_key(selection)?),
    ))
}

/// Whether two mod identities describe the same instance.
#[must_use]
pub fn same_mod_identity(left: &ModIdentity, right: &ModIdentity) -> bool {
    mod_selection_key(&left.selection).ok() == mod_selection_key(&right.selection).ok()
        && left.source == right.source
        && left.declaration_digest == right.declaration_digest
        && left.modules == right.modules
        && left.providers == right.providers
}

// ---------------------------------------------------------------------------
// Random source and media
// ---------------------------------------------------------------------------

/// Checkpointed random-generator state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RandomState {
    /// Quake III linear congruential generator.
    Q3Lcg {
        /// Seed.
        seed: i64,
        /// Draw count.
        draws: u64,
    },
    /// MSVCRT rand.
    MsvcrtRand {
        /// Seed.
        seed: i64,
        /// Draw count.
        draws: u64,
    },
    /// glibc random.
    GlibcRandom {
        /// State words.
        words: Vec<i64>,
        /// Front index.
        front: usize,
        /// Rear index.
        rear: usize,
        /// Draw count.
        draws: u64,
    },
    /// Quake II rerelease MT19937.
    Q2RereleaseMt19937 {
        /// State words.
        words: Vec<i64>,
        /// Index.
        index: usize,
        /// Draw count.
        draws: u64,
    },
    /// Opaque guest state.
    Guest {
        /// Module identifier.
        module: ModuleId,
        /// State bytes.
        bytes: Vec<u8>,
        /// Draw count.
        draws: u64,
    },
}

/// Random source with checkpoint support.
pub trait QcModRandom {
    /// Next random integer.
    fn next_integer(&mut self) -> i32;
    /// Next unit random.
    fn next_unit(&mut self) -> f64;
    /// Checkpoint the generator.
    fn checkpoint(&self) -> RandomState;
    /// Restore the generator.
    fn restore(&mut self, state: &RandomState) -> Result<(), GuestError>;
}

/// Prepared media resource.
#[derive(Debug, Clone, PartialEq)]
pub struct QcMediaResource {
    /// Requested resource path.
    pub requested_path: String,
    /// Model bounds.
    pub model_bounds: Option<Bounds>,
}

/// Prepared mod media.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct QcModMedia {
    /// Content identifier.
    pub content: ContentId,
    /// Resources by name.
    pub resources: HashMap<String, QcMediaResource>,
}

// ---------------------------------------------------------------------------
// Provider machine and services views
// ---------------------------------------------------------------------------

/// Machine surface the provider touches (mirrors `QcMachine`).
pub trait QcProviderMachine {
    /// Program metadata view.
    fn program(&self) -> &dyn QcProgramView;
    /// Entity-field word offset.
    fn field_offset(&self, name: &str) -> Result<i32, GuestError>;
    /// Global word offset.
    fn global_offset(&self, name: &str) -> Result<i32, GuestError>;
    /// Entity slot count.
    fn entity_count(&self) -> u32;
    /// Grow the entity store.
    fn set_entity_count(&mut self, count: u32) -> Result<(), GuestError>;
    /// Slot for an entity reference.
    fn entity_slot(&self, reference: i32) -> Result<u32, GuestError>;
    /// Reference for an entity slot.
    fn entity_reference(&self, slot: u32) -> i32;
    /// Clear an entity slot.
    fn zero_slot(&mut self, slot: u32) -> Result<(), GuestError>;
    /// Read a slot float.
    fn slot_float(&self, slot: u32, offset: i32) -> Result<f32, GuestError>;
    /// Write a slot float.
    fn set_slot_float(&mut self, slot: u32, offset: i32, value: f32) -> Result<(), GuestError>;
    /// Read a slot vector.
    fn slot_vector(&self, slot: u32, offset: i32) -> Result<Vec3, GuestError>;
    /// Write a slot vector.
    fn set_slot_vector(&mut self, slot: u32, offset: i32, value: Vec3) -> Result<(), GuestError>;
    /// Read a slot integer.
    fn slot_int(&self, slot: u32, offset: i32) -> Result<i32, GuestError>;
    /// Write a slot integer.
    fn set_slot_int(&mut self, slot: u32, offset: i32, value: i32) -> Result<(), GuestError>;
    /// Read a managed string.
    fn strings_get(&self, index: i32) -> Result<String, GuestError>;
    /// Allocate a managed string.
    fn strings_allocate(&mut self, text: &str) -> Result<i32, GuestError>;
    /// Write a global float.
    fn set_global_float(&mut self, name: &str, value: f32) -> Result<(), GuestError>;
    /// Execute a resolved call.
    fn invoke_resolved(
        &mut self,
        function: &str,
        arguments: &[ModRuntimeValue],
        globals: &[(String, ModRuntimeValue)],
    ) -> Result<f64, GuestError>;
}

/// Host services the provider touches (mirrors `ModHostServices`).
pub trait QcProviderServices {
    /// Current source time.
    fn now(&self) -> SourceTime;
    /// Whether an actor is live.
    fn is_live(&self, actor: &ActorId) -> bool;
    /// Resolve an owned actor.
    fn resolve_owned(&self, actor: &ActorId) -> Option<OwnedActor>;
    /// Canonical world actor.
    fn world_actor(&self) -> Option<ActorId>;
    /// Console variable value.
    fn cvar_value(&self, _name: &str) -> f64 {
        0.0
    }
    /// Console variable string.
    fn cvar_string(&self, _name: &str) -> String {
        String::new()
    }
    /// Whether client input is being applied.
    fn input_active(&self) -> bool {
        false
    }
    /// Publish client outputs.
    fn publish_client_outputs(&mut self) {}
    /// Flush routed messages.
    fn flush_messages(&mut self) {}
    /// Flush objective changes.
    fn flush_objectives(&mut self) {}
    /// Refresh objective state.
    fn refresh_objectives(&mut self) {}
    /// Advance owned source actors.
    fn advance_owned_actors(&mut self, _frame: &FrameContext) -> Result<(), GuestError> {
        Ok(())
    }
    /// Actors owned by the module.
    fn owned_actors(&self) -> Vec<OwnedActor> {
        Vec::new()
    }
    /// Source slot of an actor.
    fn source_slot(&self, _actor: &ActorId) -> Option<u32> {
        None
    }
    /// Whether a client actor is admitted.
    fn is_client_admitted(&self, _actor: &ActorId) -> bool {
        false
    }
    /// Reserved client slot.
    fn client_slot(&self, _actor: &ActorId) -> Option<u32> {
        None
    }
    /// Set-view target override.
    fn view_target(&self, _actor: &ActorId) -> Option<ActorId> {
        None
    }
    /// Whether match services are available.
    fn match_available(&self) -> bool {
        false
    }
}

/// Precached resource index.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QcPrecachedResource {
    /// Source index (1-based per kind).
    pub index: u32,
    /// Requested resource path.
    pub requested_path: String,
}

/// Client projection release outcome.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ClientRelease {
    /// Projection released.
    Released,
    /// Release deferred until the provider is idle.
    Deferred,
}

/// Maximum nested invocation depth.
pub const MAX_INVOKE_DEPTH: u32 = 64;

/// Saved provider state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QcModCheckpoint {
    /// Actor projections.
    pub projections: Vec<(SavedActorId, u32)>,
    /// Precached resource keys.
    pub precached: Vec<String>,
    /// Initialization flag.
    pub initialized: bool,
}

/// Model presentation for one owned actor.
#[derive(Debug, Clone, PartialEq)]
pub struct QcModelPresentation {
    /// Presented actor.
    pub actor: ActorId,
    /// Model path.
    pub path: String,
    /// Frame.
    pub frame: f32,
    /// Skin.
    pub skin: f32,
    /// Effects.
    pub effects: f32,
    /// Origin.
    pub origin: Vec3,
    /// Angles.
    pub angles: Vec3,
    /// Scale.
    pub scale: f32,
    /// Alpha.
    pub alpha: f32,
}

/// Client HUD vitals.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct QcHudVitals {
    /// Health.
    pub health: f32,
    /// Armor value.
    pub armor: f32,
}

/// Client view description.
#[derive(Debug, Clone, PartialEq)]
pub struct QcViewDescription {
    /// View target.
    pub target: ActorId,
    /// Target origin.
    pub origin: Vec3,
    /// Target angles.
    pub angles: Vec3,
    /// View offset.
    pub offset: Vec3,
}

/// Client presentation frame.
#[derive(Debug, Clone, PartialEq)]
pub struct QcClientFrame {
    /// HUD vitals.
    pub hud: Option<QcHudVitals>,
    /// View description.
    pub view: Option<QcViewDescription>,
}

/// Match-player snapshot.
#[derive(Debug, Clone, PartialEq)]
pub struct QcMatchPlayer {
    /// Owning provider.
    pub owner: ProviderId,
    /// Shared team.
    pub team: Option<String>,
    /// Shared score.
    pub score: f64,
}

/// Isolated source words projecting existing actors.
pub struct QcModProvider<M, S> {
    machine: M,
    services: S,
    module: ModuleIdentity,
    provider: ProviderId,
    declaration: ModCallbackDeclaration,
    media: Option<QcModMedia>,
    projections: HashMap<ActorId, u32>,
    actors_by_slot: HashMap<u32, ActorId>,
    precached: HashMap<String, QcPrecachedResource>,
    retired_projections: HashSet<ActorId>,
    depth: u32,
    loading: bool,
    initialized: bool,
    match_active: bool,
    closed: bool,
    presentation_generation: u64,
}

impl<M: QcProviderMachine, S: QcProviderServices> QcModProvider<M, S> {
    /// Build and validate a provider.
    pub fn new(
        machine: M,
        services: S,
        module: ModuleIdentity,
        provider: ProviderId,
        declaration: ModCallbackDeclaration,
        media: Option<QcModMedia>,
        validate_call: &SourceCallValidator<'_>,
    ) -> Result<Self, GuestError> {
        validate_qc_mod(machine.program(), &declaration, validate_call)?;
        if declaration.items.is_some() && media.is_none() {
            return Err(GuestError::invalid(
                "QC source items require their prepared content owner",
            ));
        }
        Ok(Self {
            machine,
            services,
            module,
            provider,
            declaration,
            media,
            projections: HashMap::new(),
            actors_by_slot: HashMap::new(),
            precached: HashMap::new(),
            retired_projections: HashSet::new(),
            depth: 0,
            loading: false,
            initialized: false,
            match_active: false,
            closed: false,
            presentation_generation: 0,
        })
    }

    /// Borrow the machine.
    #[must_use]
    pub fn machine(&self) -> &M {
        &self.machine
    }

    /// Borrow the machine mutably.
    pub fn machine_mut(&mut self) -> &mut M {
        &mut self.machine
    }

    /// Borrow the services.
    #[must_use]
    pub fn services(&self) -> &S {
        &self.services
    }

    /// Borrow the services mutably.
    pub fn services_mut(&mut self) -> &mut S {
        &mut self.services
    }

    /// Module identity.
    #[must_use]
    pub fn module(&self) -> &ModuleIdentity {
        &self.module
    }

    /// Provider identifier.
    #[must_use]
    pub fn provider(&self) -> &ProviderId {
        &self.provider
    }

    /// Callback declaration.
    #[must_use]
    pub fn declaration(&self) -> &ModCallbackDeclaration {
        &self.declaration
    }

    /// Current invocation depth.
    #[must_use]
    pub fn depth(&self) -> u32 {
        self.depth
    }

    /// Whether the provider is initialized.
    #[must_use]
    pub fn is_initialized(&self) -> bool {
        self.initialized
    }

    /// Whether the provider is closed.
    #[must_use]
    pub fn is_closed(&self) -> bool {
        self.closed
    }

    /// Presentation generation, bumped on restore.
    #[must_use]
    pub fn presentation_generation(&self) -> u64 {
        self.presentation_generation
    }

    /// Resolve the canonical actor for a source reference.
    pub fn actor(&self, reference: i32) -> Result<ActorId, GuestError> {
        if reference == 0 {
            let world = self.services.world_actor().filter(|world| self.services.is_live(world));
            return world.ok_or_else(|| GuestError::invalid("Mod source world has no canonical actor"));
        }
        let slot = self.machine.entity_slot(reference)?;
        self.actors_by_slot
            .get(&slot)
            .filter(|actor| self.services.is_live(actor))
            .cloned()
            .ok_or_else(|| GuestError::invalid("Gameplay mod references an absent or expired actor projection"))
    }

    /// Project a canonical actor into source words.
    pub fn reference(&mut self, actor: Option<&ActorId>) -> Result<i32, GuestError> {
        let Some(actor) = actor else {
            return Ok(0);
        };
        if self.services.world_actor().as_ref() == Some(actor) {
            return Ok(0);
        }
        let current = self
            .services
            .resolve_owned(actor)
            .map(|owned| owned.id().clone())
            .ok_or_else(|| GuestError::invalid("Gameplay mod cannot project a stale actor"))?;
        if let Some(slot) = self.projections.get(&current) {
            return Ok(self.machine.entity_reference(*slot));
        }
        let client_slot = self.services.client_slot(&current);
        let slot = match client_slot {
            Some(slot) => {
                self.machine.zero_slot(slot)?;
                slot
            }
            None => {
                let slot = self.machine.entity_count();
                self.machine.set_entity_count(slot + 1)?;
                slot
            }
        };
        self.projections.insert(current.clone(), slot);
        self.actors_by_slot.insert(slot, current.clone());
        for field in &self.declaration.actor_fields {
            if let ModActorBinding::Constant { value } = &field.binding {
                let offset = self.machine.field_offset(&field.field)?;
                match value {
                    ModConstantValue::Float(value) => {
                        self.machine.set_slot_float(slot, offset, *value as f32)?;
                    }
                    ModConstantValue::String(value) => {
                        let index = self.machine.strings_allocate(value)?;
                        self.machine.set_slot_int(slot, offset, index)?;
                    }
                    ModConstantValue::Vector(value) => {
                        self.machine.set_slot_vector(slot, offset, *value)?;
                    }
                }
            }
        }
        Ok(self.machine.entity_reference(slot))
    }

    /// Release a client projection, deferring while callbacks run.
    pub fn release_client_projection(&mut self, actor: &ActorId) -> Result<ClientRelease, GuestError> {
        if self.depth != 0 || self.services.input_active() {
            self.retired_projections.insert(actor.clone());
            return Ok(ClientRelease::Deferred);
        }
        if let Some(slot) = self.projections.remove(actor) {
            self.actors_by_slot.remove(&slot);
            self.machine.zero_slot(slot)?;
        }
        Ok(ClientRelease::Released)
    }

    /// Retire a projection on actor release.
    pub fn retire_projection(&mut self, actor: &ActorId) -> Result<(), GuestError> {
        self.retired_projections.insert(actor.clone());
        self.drain_retired_projections()
    }

    /// Drain retired projections once idle.
    pub fn drain_retired_projections(&mut self) -> Result<(), GuestError> {
        if self.depth != 0 || self.services.input_active() {
            return Ok(());
        }
        let retired: Vec<ActorId> = self.retired_projections.drain().collect();
        for actor in retired {
            self.release_client_projection(&actor)?;
        }
        Ok(())
    }

    /// Resolve or allocate a precached resource index.
    pub fn lookup(&mut self, kind: &str, name: &str) -> Option<QcPrecachedResource> {
        let key = format!("{kind}:{name}");
        if let Some(previous) = self.precached.get(&key) {
            return Some(previous.clone());
        }
        let resource = self.media.as_ref()?.resources.get(name)?;
        let prefix = format!("{kind}:");
        let index = self.precached.keys().filter(|key| key.starts_with(&prefix)).count() as u32 + 1;
        let value = QcPrecachedResource {
            index,
            requested_path: resource.requested_path.clone(),
        };
        self.precached.insert(key, value.clone());
        Some(value)
    }

    /// Resolve one call value against runtime inputs.
    fn resolve_value(&self, value: &ModCallbackValue, inputs: &QcModInputs) -> Result<ModRuntimeValue, GuestError> {
        match value {
            ModCallbackValue::Input(name) => inputs
                .get(name)
                .cloned()
                .ok_or_else(|| GuestError::invalid(format!("Gameplay mod call is missing input {}", name.name()))),
            ModCallbackValue::Float(value) => Ok(ModRuntimeValue::Float(*value)),
            ModCallbackValue::String(value) => Ok(ModRuntimeValue::String(value.clone())),
            ModCallbackValue::Vector(value) => Ok(ModRuntimeValue::Vector(*value)),
        }
    }

    /// Invoke a declared source call.
    pub fn invoke(&mut self, call: &ModSourceCall, inputs: &QcModInputs) -> Result<f64, GuestError> {
        if self.closed {
            return Err(GuestError::invalid("Gameplay mod is closed"));
        }
        if self.depth >= MAX_INVOKE_DEPTH {
            return Err(GuestError::DispatchDepth);
        }
        self.services.flush_objectives();
        self.services.refresh_objectives();
        let arguments = call
            .arguments
            .iter()
            .map(|value| self.resolve_value(value, inputs))
            .collect::<Result<Vec<_>, _>>()?;
        let mut globals = Vec::with_capacity(call.globals.len());
        for global in &call.globals {
            globals.push((global.name.clone(), self.resolve_value(&global.value, inputs)?));
        }
        self.depth += 1;
        let result = self.machine.invoke_resolved(&call.function, &arguments, &globals);
        self.depth -= 1;
        let value = result?;
        self.services.publish_client_outputs();
        if self.depth == 0 {
            self.services.flush_messages();
        }
        self.services.flush_objectives();
        self.drain_retired_projections()?;
        Ok(value)
    }

    /// Invoke a no-argument owned-actor callback.
    pub fn invoke_owned(
        &mut self,
        index: i32,
        actor: &ActorId,
        other: Option<&ActorId>,
        time: &SourceTime,
    ) -> Result<(), GuestError> {
        let target = self
            .machine
            .program()
            .function_at(index)
            .filter(|target| target.index != 0 && target.parameter_sizes.is_empty())
            .ok_or_else(|| GuestError::invalid("Mod actor callback must be a no-argument source function"))?;
        let mut inputs = QcModInputs::new();
        inputs.insert(ModCallbackInput::Self_, ModRuntimeValue::Actor(Some(actor.clone())));
        inputs.insert(ModCallbackInput::Other, ModRuntimeValue::Actor(other.cloned()));
        inputs.insert(ModCallbackInput::Time, ModRuntimeValue::Float(time.as_seconds_f64()));
        let call = ModSourceCall {
            function: target.name,
            arguments: Vec::new(),
            globals: ["self", "other", "time"]
                .into_iter()
                .zip([ModCallbackInput::Self_, ModCallbackInput::Other, ModCallbackInput::Time])
                .map(|(name, input)| ModSourceGlobal {
                    name: name.to_string(),
                    value: ModCallbackValue::Input(input),
                })
                .collect(),
        };
        self.invoke(&call, &inputs).map(|_| ())
    }

    /// Dispatch a console invocation.
    pub fn console_command(&mut self, argv: &[String], args_text: &str) -> Result<bool, GuestError> {
        let name = ascii_fold(argv.first().map(String::as_str).unwrap_or(""));
        let command = self
            .declaration
            .commands
            .iter()
            .find(|command| ascii_fold(&command.name) == name)
            .cloned();
        let Some(command) = command else {
            return Ok(false);
        };
        let call = super::mod_commands::qc_console_call(&command, argv, args_text)?;
        self.invoke(&call, &QcModInputs::new()).map(|_| true)
    }

    /// Run initialization calls once at an idle boundary.
    pub fn initialize(&mut self) -> Result<(), GuestError> {
        if self.initialized || self.depth != 0 {
            return Err(GuestError::invalid(
                "Mod source initialization must run once at an idle boundary",
            ));
        }
        self.loading = true;
        let now = self.services.now().as_seconds_f64();
        let mut inputs = QcModInputs::new();
        inputs.insert(ModCallbackInput::Self_, ModRuntimeValue::Actor(None));
        inputs.insert(ModCallbackInput::Time, ModRuntimeValue::Float(now));
        let calls = self.declaration.initialize.clone();
        for call in &calls {
            let result = self.invoke(call, &inputs);
            if result.is_err() {
                self.loading = false;
                return result.map(|_| ());
            }
        }
        self.loading = false;
        self.initialized = true;
        Ok(())
    }

    /// Advance one frame.
    pub fn advance(&mut self, frame: &FrameContext) -> Result<(), GuestError> {
        let elapsed = frame.elapsed.as_seconds_f64();
        let time = frame.time.as_seconds_f64() - elapsed;
        if self.machine.program().global_type("frametime") == Some(QcValueType::Float) {
            self.machine.set_global_float("frametime", elapsed as f32)?;
        }
        if let Some(call) = self.declaration.frame.clone() {
            let mut inputs = QcModInputs::new();
            inputs.insert(ModCallbackInput::Self_, ModRuntimeValue::Actor(None));
            inputs.insert(ModCallbackInput::Time, ModRuntimeValue::Float(time));
            inputs.insert(ModCallbackInput::Elapsed, ModRuntimeValue::Float(elapsed));
            self.invoke(&call, &inputs)?;
        }
        self.services.advance_owned_actors(frame)
    }

    /// Checkpoint provider-owned state.
    #[must_use]
    pub fn checkpoint(&self) -> QcModCheckpoint {
        let mut projections: Vec<(SavedActorId, u32)> = self
            .projections
            .iter()
            .map(|(actor, slot)| (SavedActorId::from(actor), *slot))
            .collect();
        projections.sort_by(|left, right| {
            (left.0.slot, left.0.generation, left.1).cmp(&(right.0.slot, right.0.generation, right.1))
        });
        let mut precached: Vec<String> = self.precached.keys().cloned().collect();
        precached.sort();
        QcModCheckpoint {
            projections,
            precached,
            initialized: self.initialized,
        }
    }

    /// Restore provider-owned state.
    pub fn restore(
        &mut self,
        saved: &QcModCheckpoint,
        resolve: &dyn Fn(&SavedActorId) -> Option<ActorId>,
    ) -> Result<(), GuestError> {
        self.presentation_generation += 1;
        if self.depth != 0 || self.services.input_active() {
            return Err(GuestError::invalid("Mod restore requires an idle callback boundary"));
        }
        let count = self.machine.entity_count();
        let mut projections = HashMap::new();
        let mut by_slot = HashMap::new();
        for (actor, slot) in &saved.projections {
            let live =
                resolve(actor).ok_or_else(|| GuestError::BadSave("Saved mod actor has no live target".to_string()))?;
            if *slot >= count || by_slot.contains_key(slot) || projections.contains_key(&live) {
                return Err(GuestError::BadSave("Invalid gameplay mod actor projection".to_string()));
            }
            projections.insert(live.clone(), *slot);
            by_slot.insert(*slot, live);
        }
        self.projections = projections;
        self.actors_by_slot = by_slot;
        self.precached.clear();
        for key in &saved.precached {
            let (kind, name) = key.split_once(':').unwrap_or(("", key.as_str()));
            if self.lookup(kind, name).is_none() {
                return Err(GuestError::BadSave("Saved mod resource no longer resolves".to_string()));
            }
        }
        self.initialized = saved.initialized;
        Ok(())
    }

    /// Model presentations for owned actors.
    pub fn presentations(&self) -> Result<Vec<QcModelPresentation>, GuestError> {
        let mut result = Vec::new();
        for actor in self.services.owned_actors() {
            let slot = match self.services.source_slot(actor.id()) {
                Some(slot) => slot,
                None => continue,
            };
            let model_offset = self.machine.field_offset("model")?;
            let path = self.machine.strings_get(self.machine.slot_int(slot, model_offset)?)?;
            if path.is_empty() {
                continue;
            }
            if self
                .media
                .as_ref()
                .is_none_or(|media| !media.resources.contains_key(&path))
            {
                return Err(GuestError::invalid(format!("Mod model was not prepared: {path}")));
            }
            let scalar = |name: &str| -> Result<f32, GuestError> {
                if self.machine.program().field_type(name).is_none() {
                    return Ok(0.0);
                }
                let offset = self.machine.field_offset(name)?;
                self.machine.slot_float(slot, offset)
            };
            let frame = scalar("frame")?;
            let alpha = scalar("alpha")?;
            let scale = scalar("scale")?;
            let origin_offset = self.machine.field_offset("origin")?;
            let angles_offset = self.machine.field_offset("angles")?;
            result.push(QcModelPresentation {
                actor: actor.id().clone(),
                path,
                frame,
                skin: scalar("skin")?,
                effects: scalar("effects")?,
                origin: self.machine.slot_vector(slot, origin_offset)?,
                angles: self.machine.slot_vector(slot, angles_offset)?,
                scale: if scale == 0.0 { 1.0 } else { scale },
                alpha: if alpha == 0.0 { 1.0 } else { alpha.clamp(0.0, 1.0) },
            });
        }
        Ok(result)
    }

    /// Client presentation frame.
    pub fn client_frame(&self, actor: &ActorId) -> Result<Option<QcClientFrame>, GuestError> {
        if self.closed || self.depth != 0 {
            return Err(GuestError::invalid(
                "QC client presentation requires an idle live source",
            ));
        }
        let declaration = match self.declaration.client_presentation.as_ref() {
            Some(declaration) => declaration,
            None => return Ok(None),
        };
        let slot = match self.projections.get(actor) {
            Some(slot) => *slot,
            None => return Ok(None),
        };
        if !self.services.is_live(actor) || !self.services.is_client_admitted(actor) {
            return Ok(None);
        }
        let hud = if declaration.hud == QcClientHud::None {
            None
        } else {
            let health = self.machine.slot_float(slot, self.machine.field_offset("health")?)?;
            let armor = self
                .machine
                .slot_float(slot, self.machine.field_offset("armorvalue")?)?;
            Some(QcHudVitals { health, armor })
        };
        let view = if declaration.view == QcClientView::None {
            None
        } else {
            let target = self.services.view_target(actor).unwrap_or_else(|| actor.clone());
            let target_slot = self
                .projections
                .get(&target)
                .filter(|_| self.services.is_live(&target))
                .ok_or_else(|| GuestError::invalid("QC client view target has no live projection"))?;
            let origin = self
                .machine
                .slot_vector(*target_slot, self.machine.field_offset("origin")?)?;
            let angles = self
                .machine
                .slot_vector(*target_slot, self.machine.field_offset("angles")?)?;
            let offset = self.machine.slot_vector(slot, self.machine.field_offset("view_ofs")?)?;
            Some(QcViewDescription {
                target,
                origin,
                angles,
                offset,
            })
        };
        Ok(Some(QcClientFrame { hud, view }))
    }

    /// Activate match bindings.
    pub fn activate_match(&mut self) -> Result<(), GuestError> {
        if self.match_active {
            return Ok(());
        }
        let needs_match = self
            .declaration
            .actor_fields
            .iter()
            .any(|field| matches!(field.binding, ModActorBinding::Team { .. } | ModActorBinding::Score));
        if !needs_match {
            return Ok(());
        }
        if !self.services.match_available() {
            return Err(GuestError::invalid(
                "Declared match fields require destination match services",
            ));
        }
        self.match_active = true;
        Ok(())
    }

    /// Match-player snapshot for an owned source actor.
    pub fn match_player(&self, actor: &ActorId) -> Result<Option<QcMatchPlayer>, GuestError> {
        if self.closed {
            return Ok(None);
        }
        let owned = self.services.resolve_owned(actor);
        let slot = self.services.source_slot(actor);
        let current_slot = self.projections.get(actor).copied();
        if owned.as_ref().is_none_or(|owned| owned.owner() != &self.provider) || slot != current_slot {
            return Ok(None);
        }
        let slot = slot.ok_or_else(|| GuestError::invalid("QuakeC match actor was retired"))?;
        let team_binding = self
            .declaration
            .actor_fields
            .iter()
            .find(|field| matches!(field.binding, ModActorBinding::Team { .. }));
        let score_binding = self
            .declaration
            .actor_fields
            .iter()
            .find(|field| matches!(field.binding, ModActorBinding::Score));
        let team = match team_binding {
            Some(field) => match &field.binding {
                ModActorBinding::Team { values } => {
                    let offset = self.machine.field_offset(&field.field)?;
                    source_team(values, f64::from(self.machine.slot_float(slot, offset)?))?
                }
                _ => None,
            },
            None => None,
        };
        let score = match score_binding {
            Some(field) => {
                let offset = self.machine.field_offset(&field.field)?;
                f64::from(self.machine.slot_float(slot, offset)?)
            }
            None => 0.0,
        };
        Ok(Some(QcMatchPlayer {
            owner: self.provider.clone(),
            team,
            score,
        }))
    }

    /// Assign a shared team through the source words.
    pub fn set_match_team(&mut self, actor: &ActorId, team: Option<String>) -> Result<(), GuestError> {
        let slot = self
            .projections
            .get(actor)
            .copied()
            .ok_or_else(|| GuestError::invalid("QuakeC match actor was retired"))?;
        let field = self
            .declaration
            .actor_fields
            .iter()
            .find(|field| matches!(field.binding, ModActorBinding::Team { .. }))
            .cloned()
            .ok_or_else(|| GuestError::invalid("QuakeC match actor has no team field"))?;
        let ModActorBinding::Team { values } = &field.binding else {
            return Err(GuestError::invalid("QuakeC match actor has no team field"));
        };
        let offset = self.machine.field_offset(&field.field)?;
        let value = original_team(values, team.as_deref())?;
        self.machine.set_slot_float(slot, offset, value as f32)
    }

    /// Assign a shared score through the source words.
    pub fn set_match_score(&mut self, actor: &ActorId, score: f64) -> Result<(), GuestError> {
        if !score.is_finite() {
            return Err(GuestError::invalid("QuakeC match actor has no finite score field"));
        }
        let slot = self
            .projections
            .get(actor)
            .copied()
            .ok_or_else(|| GuestError::invalid("QuakeC match actor was retired"))?;
        let field = self
            .declaration
            .actor_fields
            .iter()
            .find(|field| matches!(field.binding, ModActorBinding::Score))
            .cloned()
            .ok_or_else(|| GuestError::invalid("QuakeC match actor has no score field"))?;
        let offset = self.machine.field_offset(&field.field)?;
        self.machine.set_slot_float(slot, offset, score as f32)
    }

    /// Close the provider.
    pub fn close(&mut self) {
        if self.closed {
            return;
        }
        self.closed = true;
        self.projections.clear();
        self.actors_by_slot.clear();
        self.retired_projections.clear();
        self.precached.clear();
    }

    /// Actor-field layout for engine-side storage.
    fn field_layout(&self) -> FieldLayout {
        let mut layout = FieldLayout::new();
        for field in &self.declaration.actor_fields {
            let type_name = match &field.binding {
                ModActorBinding::Constant { value } => match value {
                    ModConstantValue::Float(_) => "float",
                    ModConstantValue::String(_) => "string",
                    ModConstantValue::Vector(_) => "vector",
                },
                ModActorBinding::Classname | ModActorBinding::Userinfo { .. } => "string",
                ModActorBinding::Origin
                | ModActorBinding::Velocity
                | ModActorBinding::Angles
                | ModActorBinding::BoundsMin
                | ModActorBinding::BoundsMax
                | ModActorBinding::ViewOffset => "vector",
                ModActorBinding::ClientInput { input, .. } => {
                    if *input == ModClientInput::ViewAngles {
                        "vector"
                    } else {
                        "float"
                    }
                }
                _ => "float",
            };
            layout = layout.field(&field.field, type_name);
        }
        layout
    }

    /// Invoke declared callbacks for one operation.
    fn dispatch_operation(
        &mut self,
        operation: ModCallbackOperation,
        time: SourceTime,
        build: &dyn Fn(&mut QcModInputs),
    ) -> Result<bool, GuestError> {
        let calls: Vec<ModSourceCall> = self
            .declaration
            .callbacks
            .iter()
            .filter(|callback| callback.binding.operation == operation)
            .map(|callback| callback.call.clone())
            .collect();
        if calls.is_empty() {
            return Ok(false);
        }
        for call in &calls {
            let mut inputs = QcModInputs::new();
            inputs.insert(ModCallbackInput::Time, ModRuntimeValue::Float(time.as_seconds_f64()));
            build(&mut inputs);
            self.invoke(call, &inputs)?;
        }
        Ok(true)
    }
}

impl<M: QcProviderMachine, S: QcProviderServices> GameModule for QcModProvider<M, S> {
    fn name(&self) -> &str {
        &self.module.id
    }

    fn layout(&self) -> FieldLayout {
        self.field_layout()
    }

    fn spawn(
        &mut self,
        context: &mut GameContext,
        actor: &ActorId,
        _classname: &str,
        _fields: &[(&str, &str)],
    ) -> Result<bool, GuestError> {
        let layout = self.field_layout();
        context.fields.allocate(actor, &layout)?;
        for field in &self.declaration.actor_fields {
            if let ModActorBinding::Constant { value } = &field.binding {
                let stored = match value {
                    ModConstantValue::Float(value) => FieldValue::Float(*value as f32),
                    ModConstantValue::String(value) => FieldValue::Text(value.clone()),
                    ModConstantValue::Vector(value) => FieldValue::Vector(*value),
                };
                context.fields.set(actor, &field.field, stored)?;
            }
        }
        self.reference(Some(actor))?;
        Ok(true)
    }

    fn think(&mut self, context: &mut GameContext, actor: &ActorId) -> Result<bool, GuestError> {
        let owned = actor.clone();
        let time = context.frame.time;
        self.dispatch_operation(ModCallbackOperation::ActorThink, time, &|inputs| {
            inputs.insert(ModCallbackInput::Self_, ModRuntimeValue::Actor(Some(owned.clone())));
        })
    }

    fn touch(&mut self, context: &mut GameContext, contact: &GameTouch) -> Result<bool, GuestError> {
        let target = contact.target.clone();
        let other = contact.other.clone();
        let time = context.frame.time;
        self.dispatch_operation(ModCallbackOperation::ActorTouch, time, &|inputs| {
            inputs.insert(ModCallbackInput::Self_, ModRuntimeValue::Actor(Some(target.clone())));
            inputs.insert(ModCallbackInput::Other, ModRuntimeValue::Actor(Some(other.clone())));
        })
    }

    fn use_on(
        &mut self,
        context: &mut GameContext,
        target: &ActorId,
        other: Option<&ActorId>,
        activator: Option<&ActorId>,
    ) -> Result<bool, GuestError> {
        let target = target.clone();
        let other = other.cloned();
        let activator = activator.cloned();
        let time = context.frame.time;
        self.dispatch_operation(ModCallbackOperation::ActorUse, time, &|inputs| {
            inputs.insert(ModCallbackInput::Self_, ModRuntimeValue::Actor(Some(target.clone())));
            inputs.insert(ModCallbackInput::Other, ModRuntimeValue::Actor(other.clone()));
            inputs.insert(ModCallbackInput::Activator, ModRuntimeValue::Actor(activator.clone()));
        })
    }

    fn pain(&mut self, context: &mut GameContext, reaction: &GameReaction) -> Result<bool, GuestError> {
        let reaction = reaction.clone();
        let time = context.frame.time;
        self.dispatch_operation(ModCallbackOperation::ActorPain, time, &|inputs| {
            inputs.insert(
                ModCallbackInput::Self_,
                ModRuntimeValue::Actor(Some(reaction.target.clone())),
            );
            inputs.insert(
                ModCallbackInput::Attacker,
                ModRuntimeValue::Actor(reaction.attacker.clone()),
            );
            inputs.insert(ModCallbackInput::Amount, ModRuntimeValue::Float(reaction.damage));
            inputs.insert(ModCallbackInput::Knockback, ModRuntimeValue::Float(reaction.kick));
        })
    }

    fn die(&mut self, context: &mut GameContext, reaction: &GameReaction) -> Result<bool, GuestError> {
        let reaction = reaction.clone();
        let time = context.frame.time;
        self.dispatch_operation(ModCallbackOperation::ActorDie, time, &|inputs| {
            inputs.insert(
                ModCallbackInput::Self_,
                ModRuntimeValue::Actor(Some(reaction.target.clone())),
            );
            inputs.insert(
                ModCallbackInput::Attacker,
                ModRuntimeValue::Actor(reaction.attacker.clone()),
            );
            inputs.insert(
                ModCallbackInput::Inflictor,
                ModRuntimeValue::Actor(reaction.inflictor.clone()),
            );
            inputs.insert(ModCallbackInput::Amount, ModRuntimeValue::Float(reaction.damage));
            inputs.insert(ModCallbackInput::Knockback, ModRuntimeValue::Float(reaction.kick));
            if let Some(point) = reaction.point {
                inputs.insert(ModCallbackInput::Point, ModRuntimeValue::Vector(point));
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_core::identity::IdentityOwner;
    use qa_core::math::vec3;
    use qa_core::time::FramePhase;

    use crate::fields::FieldTable;

    #[derive(Debug, Clone)]
    enum SlotWord {
        Float(f32),
        Int(i32),
        Vector(Vec3),
    }

    struct FakeProgram {
        digest: String,
        fields: HashMap<String, QcValueType>,
        globals: HashMap<String, QcValueType>,
        functions: HashMap<String, QcFunctionView>,
        by_index: HashMap<i32, QcFunctionView>,
    }

    impl QcProgramView for FakeProgram {
        fn digest(&self) -> &str {
            &self.digest
        }

        fn api_kind(&self) -> QcApiKind {
            QcApiKind::Q1Netquake
        }

        fn field_type(&self, name: &str) -> Option<QcValueType> {
            self.fields.get(name).copied()
        }

        fn global_type(&self, name: &str) -> Option<QcValueType> {
            self.globals.get(name).copied()
        }

        fn function_named(&self, name: &str) -> Option<QcFunctionView> {
            self.functions.get(name).cloned()
        }

        fn function_at(&self, index: i32) -> Option<QcFunctionView> {
            self.by_index.get(&index).cloned()
        }

        fn functions(&self) -> Vec<QcFunctionView> {
            self.functions.values().cloned().collect()
        }
    }

    struct FakeMachine {
        program: FakeProgram,
        offsets: HashMap<String, i32>,
        slots: Vec<HashMap<i32, SlotWord>>,
        strings: Vec<String>,
        globals: HashMap<String, f32>,
        invocations: Vec<(String, Vec<ModRuntimeValue>, Vec<(String, ModRuntimeValue)>)>,
        result: f64,
    }

    impl FakeMachine {
        fn new() -> Self {
            Self {
                program: FakeProgram {
                    digest: "abc".to_string(),
                    fields: HashMap::new(),
                    globals: HashMap::new(),
                    functions: HashMap::new(),
                    by_index: HashMap::new(),
                },
                offsets: HashMap::new(),
                slots: vec![HashMap::new()],
                strings: vec![String::new()],
                globals: HashMap::new(),
                invocations: Vec::new(),
                result: 0.0,
            }
        }

        fn with_field(mut self, name: &str, offset: i32, kind: QcValueType) -> Self {
            self.program.fields.insert(name.to_string(), kind);
            self.offsets.insert(name.to_string(), offset);
            self
        }
    }

    impl QcProviderMachine for FakeMachine {
        fn program(&self) -> &dyn QcProgramView {
            &self.program
        }

        fn field_offset(&self, name: &str) -> Result<i32, GuestError> {
            self.offsets
                .get(name)
                .copied()
                .ok_or_else(|| GuestError::UnknownField(name.to_string()))
        }

        fn global_offset(&self, name: &str) -> Result<i32, GuestError> {
            self.offsets
                .get(name)
                .copied()
                .ok_or_else(|| GuestError::UnknownField(name.to_string()))
        }

        fn entity_count(&self) -> u32 {
            self.slots.len() as u32
        }

        fn set_entity_count(&mut self, count: u32) -> Result<(), GuestError> {
            if (count as usize) < self.slots.len() {
                return Err(GuestError::invalid("Cannot shrink entity storage"));
            }
            self.slots.resize(count as usize, HashMap::new());
            Ok(())
        }

        fn entity_slot(&self, reference: i32) -> Result<u32, GuestError> {
            if reference <= 0 || reference as usize >= self.slots.len() {
                return Err(GuestError::invalid("Reference names no live slot"));
            }
            Ok(reference as u32)
        }

        fn entity_reference(&self, slot: u32) -> i32 {
            slot as i32
        }

        fn zero_slot(&mut self, slot: u32) -> Result<(), GuestError> {
            self.slots
                .get_mut(slot as usize)
                .ok_or_else(|| GuestError::invalid("Slot is out of range"))?
                .clear();
            Ok(())
        }

        fn slot_float(&self, slot: u32, offset: i32) -> Result<f32, GuestError> {
            match self.slots.get(slot as usize).and_then(|slot| slot.get(&offset)) {
                Some(SlotWord::Float(value)) => Ok(*value),
                Some(_) => Err(GuestError::FieldType(format!("word {offset}"))),
                None => Ok(0.0),
            }
        }

        fn set_slot_float(&mut self, slot: u32, offset: i32, value: f32) -> Result<(), GuestError> {
            self.slots
                .get_mut(slot as usize)
                .ok_or_else(|| GuestError::invalid("Slot is out of range"))?
                .insert(offset, SlotWord::Float(value));
            Ok(())
        }

        fn slot_vector(&self, slot: u32, offset: i32) -> Result<Vec3, GuestError> {
            match self.slots.get(slot as usize).and_then(|slot| slot.get(&offset)) {
                Some(SlotWord::Vector(value)) => Ok(*value),
                Some(_) => Err(GuestError::FieldType(format!("word {offset}"))),
                None => Ok(vec3(0.0, 0.0, 0.0)),
            }
        }

        fn set_slot_vector(&mut self, slot: u32, offset: i32, value: Vec3) -> Result<(), GuestError> {
            self.slots
                .get_mut(slot as usize)
                .ok_or_else(|| GuestError::invalid("Slot is out of range"))?
                .insert(offset, SlotWord::Vector(value));
            Ok(())
        }

        fn slot_int(&self, slot: u32, offset: i32) -> Result<i32, GuestError> {
            match self.slots.get(slot as usize).and_then(|slot| slot.get(&offset)) {
                Some(SlotWord::Int(value)) => Ok(*value),
                Some(SlotWord::Float(value)) => Ok(*value as i32),
                Some(_) => Err(GuestError::FieldType(format!("word {offset}"))),
                None => Ok(0),
            }
        }

        fn set_slot_int(&mut self, slot: u32, offset: i32, value: i32) -> Result<(), GuestError> {
            self.slots
                .get_mut(slot as usize)
                .ok_or_else(|| GuestError::invalid("Slot is out of range"))?
                .insert(offset, SlotWord::Int(value));
            Ok(())
        }

        fn strings_get(&self, index: i32) -> Result<String, GuestError> {
            self.strings
                .get(index as usize)
                .cloned()
                .ok_or_else(|| GuestError::invalid("String index is out of range"))
        }

        fn strings_allocate(&mut self, text: &str) -> Result<i32, GuestError> {
            self.strings.push(text.to_string());
            Ok(self.strings.len() as i32 - 1)
        }

        fn set_global_float(&mut self, name: &str, value: f32) -> Result<(), GuestError> {
            self.globals.insert(name.to_string(), value);
            Ok(())
        }

        fn invoke_resolved(
            &mut self,
            function: &str,
            arguments: &[ModRuntimeValue],
            globals: &[(String, ModRuntimeValue)],
        ) -> Result<f64, GuestError> {
            self.invocations
                .push((function.to_string(), arguments.to_vec(), globals.to_vec()));
            Ok(self.result)
        }
    }

    struct FakeServices {
        owner: IdentityOwner,
        live: HashSet<ActorId>,
        owned: HashMap<ActorId, OwnedActor>,
        world: Option<ActorId>,
        published: usize,
        flushed: usize,
        client_slots: HashMap<ActorId, u32>,
        admitted: HashSet<ActorId>,
        source_slots: HashMap<ActorId, u32>,
        owned_actors: Vec<OwnedActor>,
        match_avail: bool,
        view_targets: HashMap<ActorId, ActorId>,
    }

    impl FakeServices {
        fn new() -> Self {
            Self {
                owner: IdentityOwner::create("test").unwrap(),
                live: HashSet::new(),
                owned: HashMap::new(),
                world: None,
                published: 0,
                flushed: 0,
                client_slots: HashMap::new(),
                admitted: HashSet::new(),
                source_slots: HashMap::new(),
                owned_actors: Vec::new(),
                match_avail: false,
                view_targets: HashMap::new(),
            }
        }

        fn admit(&mut self, slot: u32, provider: &ProviderId) -> ActorId {
            let actor = self.owner.actor(slot, 1);
            self.live.insert(actor.clone());
            self.owned
                .insert(actor.clone(), self.owner.owned_actor(&actor, provider.clone()).unwrap());
            actor
        }
    }

    impl QcProviderServices for FakeServices {
        fn now(&self) -> SourceTime {
            SourceTime::Seconds(2.0)
        }

        fn is_live(&self, actor: &ActorId) -> bool {
            self.live.contains(actor)
        }

        fn resolve_owned(&self, actor: &ActorId) -> Option<OwnedActor> {
            self.owned.get(actor).cloned()
        }

        fn world_actor(&self) -> Option<ActorId> {
            self.world.clone()
        }

        fn publish_client_outputs(&mut self) {
            self.published += 1;
        }

        fn flush_messages(&mut self) {
            self.flushed += 1;
        }

        fn owned_actors(&self) -> Vec<OwnedActor> {
            self.owned_actors.clone()
        }

        fn source_slot(&self, actor: &ActorId) -> Option<u32> {
            self.source_slots.get(actor).copied()
        }

        fn is_client_admitted(&self, actor: &ActorId) -> bool {
            self.admitted.contains(actor)
        }

        fn client_slot(&self, actor: &ActorId) -> Option<u32> {
            self.client_slots.get(actor).copied()
        }

        fn view_target(&self, actor: &ActorId) -> Option<ActorId> {
            self.view_targets.get(actor).cloned()
        }

        fn match_available(&self) -> bool {
            self.match_avail
        }
    }

    fn module() -> ModuleIdentity {
        ModuleIdentity {
            id: "test:mod".to_string(),
            artifact_path: "progs.dat".to_string(),
            digest: "abc".to_string(),
            revision: 1,
        }
    }

    fn provider_id() -> ProviderId {
        ProviderId::new("mod", "test")
    }

    fn declaration() -> ModCallbackDeclaration {
        ModCallbackDeclaration {
            program: Some(ModProgramRef {
                path: "progs.dat".to_string(),
                digest: "abc".to_string(),
            }),
            ..ModCallbackDeclaration::default()
        }
    }

    fn accept_call(_call: &ModSourceCall, _inputs: &[ModCallbackInput], _context: &str) -> Result<(), GuestError> {
        Ok(())
    }

    fn fixture(declaration: ModCallbackDeclaration) -> QcModProvider<FakeMachine, FakeServices> {
        QcModProvider::new(
            FakeMachine::new(),
            FakeServices::new(),
            module(),
            provider_id(),
            declaration,
            None,
            &accept_call,
        )
        .unwrap()
    }

    #[test]
    fn selection_key_round_trip() {
        let selection = ModSelection {
            product: "id1".to_string(),
            id: "maps/e1m1".to_string(),
        };
        assert_eq!(mod_selection_key(&selection).unwrap(), "id1/maps/e1m1");
        assert_eq!(read_mod_selection("id1/maps/e1m1").unwrap(), selection);
        assert!(read_mod_selection("no-slash").is_err());
        assert!(mod_selection_key(&ModSelection {
            product: "".to_string(),
            id: "x".to_string()
        })
        .is_err());
    }

    #[test]
    fn instance_provider_encodes_key() {
        let selection = ModSelection {
            product: "id1".to_string(),
            id: "a/b:c".to_string(),
        };
        let provider = mod_instance_provider(&selection).unwrap();
        assert_eq!(provider.namespace, "mod");
        assert_eq!(provider.name, "id1%2Fa%2Fb%3Ac");
    }

    #[test]
    fn same_identity_compares_fields() {
        let identity = || ModIdentity {
            selection: ModSelection {
                product: "id1".to_string(),
                id: "x".to_string(),
            },
            source: ProviderReference {
                provider: provider_id(),
                content: "c".to_string(),
            },
            declaration_digest: "d".to_string(),
            modules: vec![module()],
            providers: vec![ProviderCheckpointRef {
                provider: provider_id(),
                schema: "s".to_string(),
                version: 1,
            }],
        };
        assert!(same_mod_identity(&identity(), &identity()));
        let mut other = identity();
        other.declaration_digest = "other".to_string();
        assert!(!same_mod_identity(&identity(), &other));
    }

    #[test]
    fn team_mapping_round_trip() {
        let values = vec![
            SourceTeamValue {
                value: 1.0,
                team: Some("red".to_string()),
            },
            SourceTeamValue { value: 2.0, team: None },
        ];
        assert_eq!(source_team(&values, 1.0).unwrap(), Some("red".to_string()));
        assert_eq!(original_team(&values, Some("red")).unwrap(), 1.0);
        assert!(source_team(&values, 9.0).is_err());
        assert!(original_team(&values, Some("blue")).is_err());
        assert!(validate_source_match_field(&ModActorBinding::Team { values }).is_ok());
        assert!(validate_source_match_field(&ModActorBinding::Score).is_ok());
        assert!(validate_source_match_field(&ModActorBinding::Team { values: vec![] }).is_err());
    }

    #[test]
    fn ascii_fold_and_tokenize() {
        assert_eq!(ascii_fold("AbC"), "abc");
        assert_eq!(
            split_command_tokens("give \"super shotgun\" 5"),
            vec!["give", "super shotgun", "5"]
        );
        assert_eq!(split_command_tokens("  solo  "), vec!["solo"]);
    }

    #[test]
    fn validation_rejects_digest_mismatch() {
        let program = FakeMachine::new().program;
        let mut bad = declaration();
        bad.program = Some(ModProgramRef {
            path: "progs.dat".to_string(),
            digest: "other".to_string(),
        });
        assert!(validate_qc_mod(&program, &bad, &accept_call).is_err());
        assert!(validate_qc_mod(&program, &declaration(), &accept_call).is_ok());
    }

    #[test]
    fn validation_rejects_duplicates_and_bad_clients() {
        let program = FakeMachine::new().program;
        let mut duplicated = declaration();
        duplicated.cvars.push(ModCvar {
            name: "skill".to_string(),
            value: "1".to_string(),
        });
        duplicated.cvars.push(ModCvar {
            name: "skill".to_string(),
            value: "2".to_string(),
        });
        assert!(validate_qc_mod(&program, &duplicated, &accept_call).is_err());
        let mut clients = declaration();
        clients.clients = Some(ModClientDeclaration {
            maximum: 0,
            ..ModClientDeclaration::default()
        });
        assert!(validate_qc_mod(&program, &clients, &accept_call).is_err());
    }

    #[test]
    fn validation_requires_combat_fields() {
        let program = FakeMachine::new().program;
        let mut combat = declaration();
        combat.combat = Some(ModCombatDeclaration {
            damage: ModSourceCall {
                function: "T_Damage".to_string(),
                arguments: vec![],
                globals: vec![],
            },
            damage_scale: None,
            armor_stage: None,
            empty_armor: None,
        });
        assert!(validate_qc_mod(&program, &combat, &accept_call).is_err());
    }

    #[test]
    fn projections_round_trip_through_references() {
        let mut provider = fixture(declaration());
        let actor = provider.services_mut().admit(4, &provider_id());
        let reference = provider.reference(Some(&actor)).unwrap();
        assert!(reference > 0);
        assert_eq!(provider.actor(reference).unwrap(), actor);
        assert_eq!(provider.reference(Some(&actor)).unwrap(), reference);
        assert_eq!(provider.reference(None).unwrap(), 0);
        assert_eq!(
            provider.release_client_projection(&actor).unwrap(),
            ClientRelease::Released
        );
        assert!(provider.actor(reference).is_err());
    }

    #[test]
    fn world_actor_projects_to_zero() {
        let mut provider = fixture(declaration());
        let world = provider.services_mut().owner.actor(0, 1);
        provider.services_mut().live.insert(world.clone());
        provider.services_mut().world = Some(world.clone());
        assert_eq!(provider.reference(Some(&world)).unwrap(), 0);
        assert_eq!(provider.actor(0).unwrap(), world);
    }

    #[test]
    fn invoke_resolves_inputs_and_publishes() {
        let mut provider = fixture(declaration());
        let owner = IdentityOwner::create("calls").unwrap();
        let actor = owner.actor(1, 1);
        let call = ModSourceCall {
            function: "think".to_string(),
            arguments: vec![
                ModCallbackValue::Input(ModCallbackInput::Self_),
                ModCallbackValue::Float(1.5),
            ],
            globals: vec![ModSourceGlobal {
                name: "time".to_string(),
                value: ModCallbackValue::Input(ModCallbackInput::Time),
            }],
        };
        let mut inputs = QcModInputs::new();
        inputs.insert(ModCallbackInput::Self_, ModRuntimeValue::Actor(Some(actor.clone())));
        inputs.insert(ModCallbackInput::Time, ModRuntimeValue::Float(2.0));
        provider.invoke(&call, &inputs).unwrap();
        assert_eq!(provider.depth(), 0);
        assert_eq!(provider.services().published, 1);
        assert_eq!(provider.services().flushed, 1);
        let (function, arguments, globals) = provider.machine().invocations.last().unwrap();
        assert_eq!(function, "think");
        assert_eq!(arguments[0], ModRuntimeValue::Actor(Some(actor)));
        assert_eq!(globals[0].0, "time");
        assert!(provider.invoke(&call, &QcModInputs::new()).is_err());
    }

    #[test]
    fn invoke_owned_requires_no_argument_function() {
        let mut provider = fixture(declaration());
        provider.machine_mut().program.functions.insert(
            "thinker".to_string(),
            QcFunctionView {
                index: 7,
                name: "thinker".to_string(),
                first_statement: 3,
                parameter_start: 0,
                parameter_sizes: vec![],
                named_builtin: false,
            },
        );
        provider.machine_mut().program.by_index.insert(
            7,
            QcFunctionView {
                index: 7,
                name: "thinker".to_string(),
                first_statement: 3,
                parameter_start: 0,
                parameter_sizes: vec![],
                named_builtin: false,
            },
        );
        let owner = IdentityOwner::create("owned").unwrap();
        let actor = owner.actor(2, 1);
        provider
            .invoke_owned(7, &actor, None, &SourceTime::Seconds(1.0))
            .unwrap();
        assert_eq!(provider.machine().invocations.last().unwrap().0, "thinker");
        assert!(provider
            .invoke_owned(9, &actor, None, &SourceTime::Seconds(1.0))
            .is_err());
    }

    #[test]
    fn console_command_dispatches_and_reports_unknown() {
        let mut declared = declaration();
        declared.commands.push(ModConsoleCommand {
            name: "give".to_string(),
            function: "cmd_give".to_string(),
            arguments: vec![ModConsoleValue::ArgumentCount],
            globals: vec![],
        });
        let mut provider = fixture(declared);
        let argv = vec!["give".to_string(), "shells".to_string()];
        assert!(provider.console_command(&argv, "shells").unwrap());
        assert_eq!(provider.machine().invocations.last().unwrap().0, "cmd_give");
        assert!(!provider.console_command(&["nope".to_string()], "").unwrap());
    }

    #[test]
    fn initialize_runs_once() {
        let mut declared = declaration();
        declared.initialize.push(ModSourceCall {
            function: "init".to_string(),
            arguments: vec![],
            globals: vec![],
        });
        let mut provider = fixture(declared);
        provider.initialize().unwrap();
        assert!(provider.is_initialized());
        assert_eq!(provider.machine().invocations.len(), 1);
        assert!(provider.initialize().is_err());
    }

    #[test]
    fn advance_runs_frame_call_and_frametime() {
        let mut declared = declaration();
        declared.frame = Some(ModSourceCall {
            function: "frame".to_string(),
            arguments: vec![],
            globals: vec![],
        });
        let mut provider = fixture(declared);
        provider
            .machine_mut()
            .program
            .globals
            .insert("frametime".to_string(), QcValueType::Float);
        let frame = FrameContext {
            frame: 3,
            time: SourceTime::Seconds(1.0),
            elapsed: SourceTime::Seconds(0.1),
            phase: FramePhase::FrameEntry,
        };
        provider.advance(&frame).unwrap();
        assert_eq!(provider.machine().invocations.last().unwrap().0, "frame");
        assert!((provider.machine().globals["frametime"] - 0.1).abs() < 1e-6);
    }

    #[test]
    fn checkpoint_restore_round_trip() {
        let mut provider = fixture(declaration());
        let actor = provider.services_mut().admit(6, &provider_id());
        provider.reference(Some(&actor)).unwrap();
        let saved = provider.checkpoint();
        assert_eq!(saved.projections.len(), 1);
        let mut revived = fixture(declaration());
        revived.machine_mut().set_entity_count(2).unwrap();
        revived.services_mut().live.insert(actor.clone());
        revived
            .restore(&saved, &|saved| (saved.slot == 6).then(|| actor.clone()))
            .unwrap();
        assert!(revived.is_initialized() == provider.is_initialized());
        assert_eq!(revived.presentation_generation(), 1);
        let mut missing = fixture(declaration());
        assert!(missing.restore(&saved, &|_| None).is_err());
    }

    #[test]
    fn lookup_assigns_per_kind_indices() {
        let mut media = QcModMedia::default();
        media.resources.insert(
            "a.mdl".to_string(),
            QcMediaResource {
                requested_path: "a.mdl".to_string(),
                model_bounds: None,
            },
        );
        media.resources.insert(
            "b.wav".to_string(),
            QcMediaResource {
                requested_path: "b.wav".to_string(),
                model_bounds: None,
            },
        );
        let mut provider = QcModProvider::new(
            FakeMachine::new(),
            FakeServices::new(),
            module(),
            provider_id(),
            declaration(),
            Some(media),
            &accept_call,
        )
        .unwrap();
        assert_eq!(provider.lookup("model", "a.mdl").unwrap().index, 1);
        assert_eq!(provider.lookup("sound", "b.wav").unwrap().index, 1);
        assert_eq!(provider.lookup("model", "a.mdl").unwrap().index, 1);
        assert!(provider.lookup("model", "missing.mdl").is_none());
    }

    #[test]
    fn match_player_reads_and_writes_team() {
        let mut declared = declaration();
        declared.actor_fields.push(ModActorField {
            field: "team_no".to_string(),
            binding: ModActorBinding::Team {
                values: vec![
                    SourceTeamValue {
                        value: 1.0,
                        team: Some("red".to_string()),
                    },
                    SourceTeamValue {
                        value: 2.0,
                        team: Some("blue".to_string()),
                    },
                ],
            },
        });
        let machine = FakeMachine::new().with_field("team_no", 4, QcValueType::Float);
        let mut provider = QcModProvider::new(
            machine,
            FakeServices::new(),
            module(),
            provider_id(),
            declared,
            None,
            &accept_call,
        )
        .unwrap();
        let actor = provider.services_mut().admit(9, &provider_id());
        let reference = provider.reference(Some(&actor)).unwrap();
        let slot = provider.machine().entity_slot(reference).unwrap();
        provider.services_mut().source_slots.insert(actor.clone(), slot);
        provider.services_mut().match_avail = true;
        provider.activate_match().unwrap();
        provider.machine_mut().set_slot_float(slot, 4, 2.0).unwrap();
        assert_eq!(
            provider.match_player(&actor).unwrap().unwrap().team,
            Some("blue".to_string())
        );
        provider.set_match_team(&actor, Some("red".to_string())).unwrap();
        assert_eq!(provider.machine().slot_float(slot, 4).unwrap(), 1.0);
        assert!(provider.set_match_team(&actor, Some("green".to_string())).is_err());
    }

    #[test]
    fn presentations_read_owned_models() {
        let machine = FakeMachine::new()
            .with_field("model", 1, QcValueType::String)
            .with_field("origin", 2, QcValueType::Vector)
            .with_field("angles", 3, QcValueType::Vector);
        let mut media = QcModMedia::default();
        media.resources.insert(
            "progs/player.mdl".to_string(),
            QcMediaResource {
                requested_path: "progs/player.mdl".to_string(),
                model_bounds: None,
            },
        );
        let mut provider = QcModProvider::new(
            machine,
            FakeServices::new(),
            module(),
            provider_id(),
            declaration(),
            Some(media),
            &accept_call,
        )
        .unwrap();
        let actor = provider.services_mut().admit(11, &provider_id());
        let owned = provider.services().resolve_owned(&actor).unwrap();
        provider.services_mut().owned_actors.push(owned);
        let reference = provider.reference(Some(&actor)).unwrap();
        let slot = provider.machine().entity_slot(reference).unwrap();
        provider.services_mut().source_slots.insert(actor.clone(), slot);
        let index = provider.machine_mut().strings_allocate("progs/player.mdl").unwrap();
        provider.machine_mut().set_slot_int(slot, 1, index).unwrap();
        provider
            .machine_mut()
            .set_slot_vector(slot, 2, vec3(1.0, 2.0, 3.0))
            .unwrap();
        let presentations = provider.presentations().unwrap();
        assert_eq!(presentations.len(), 1);
        assert_eq!(presentations[0].path, "progs/player.mdl");
        assert_eq!(presentations[0].scale, 1.0);
    }

    #[test]
    fn client_frame_reads_hud_and_view() {
        let machine = FakeMachine::new()
            .with_field("health", 1, QcValueType::Float)
            .with_field("armorvalue", 2, QcValueType::Float)
            .with_field("origin", 3, QcValueType::Vector)
            .with_field("angles", 4, QcValueType::Vector)
            .with_field("view_ofs", 5, QcValueType::Vector);
        let mut declared = declaration();
        declared.client_presentation = Some(QcModClientPresentation {
            hud: QcClientHud::ReplaceVitals,
            view: QcClientView::SetView,
        });
        let mut provider = QcModProvider::new(
            machine,
            FakeServices::new(),
            module(),
            provider_id(),
            declared,
            None,
            &accept_call,
        )
        .unwrap();
        let actor = provider.services_mut().admit(12, &provider_id());
        let reference = provider.reference(Some(&actor)).unwrap();
        let slot = provider.machine().entity_slot(reference).unwrap();
        provider.services_mut().admitted.insert(actor.clone());
        provider.machine_mut().set_slot_float(slot, 1, 75.0).unwrap();
        provider
            .machine_mut()
            .set_slot_vector(slot, 3, vec3(4.0, 5.0, 6.0))
            .unwrap();
        let frame = provider.client_frame(&actor).unwrap().unwrap();
        assert_eq!(frame.hud.unwrap().health, 75.0);
        assert_eq!(frame.view.unwrap().origin, vec3(4.0, 5.0, 6.0));
    }

    #[test]
    fn game_module_spawns_and_dispatches() {
        let mut declared = declaration();
        declared.actor_fields.push(ModActorField {
            field: "dmg".to_string(),
            binding: ModActorBinding::Constant {
                value: ModConstantValue::Float(40.0),
            },
        });
        declared.callbacks.push(ModCallback {
            binding: ModCallbackBinding {
                id: "test:think".to_string(),
                operation: ModCallbackOperation::ActorThink,
                stage: ModCallbackStage::Observe,
                result: None,
            },
            call: ModSourceCall {
                function: "think_cb".to_string(),
                arguments: vec![],
                globals: vec![],
            },
        });
        let machine = FakeMachine::new().with_field("dmg", 9, QcValueType::Float);
        let mut provider = QcModProvider::new(
            machine,
            FakeServices::new(),
            module(),
            provider_id(),
            declared,
            None,
            &accept_call,
        )
        .unwrap();
        assert_eq!(provider.name(), "test:mod");
        let actor = provider.services_mut().admit(14, &provider_id());
        let mut fields = FieldTable::new();
        let mut events = Vec::new();
        let frame = FrameContext {
            frame: 0,
            time: SourceTime::Seconds(0.0),
            elapsed: SourceTime::Seconds(0.0),
            phase: FramePhase::EntityThink,
        };
        let mut context = GameContext {
            fields: &mut fields,
            events: &mut events,
            frame,
            now: SourceTime::Seconds(0.0),
        };
        assert!(provider.spawn(&mut context, &actor, "monster", &[]).unwrap());
        assert_eq!(
            context.fields.get(&actor, "dmg").unwrap().as_float("dmg").unwrap(),
            40.0
        );
        assert!(provider.think(&mut context, &actor).unwrap());
        assert_eq!(provider.machine().invocations.last().unwrap().0, "think_cb");
        assert!(!provider
            .touch(
                &mut context,
                &GameTouch {
                    target: actor.clone(),
                    other: actor
                }
            )
            .unwrap());
        provider.close();
        assert!(provider.is_closed());
        assert!(provider
            .invoke(
                &ModSourceCall {
                    function: "x".to_string(),
                    arguments: vec![],
                    globals: vec![]
                },
                &QcModInputs::new()
            )
            .is_err());
    }
}

/// Gameplay-mod callback declaration.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ModCallbackDeclaration {
    /// Declared objectives.
    pub objectives: Vec<QcObjectiveDeclaration>,
    /// Client presentation.
    pub client_presentation: Option<QcModClientPresentation>,
    /// Program artifact.
    pub program: Option<ModProgramRef>,
    /// Actor fields.
    pub actor_fields: Vec<ModActorField>,
    /// Callbacks.
    pub callbacks: Vec<ModCallback>,
    /// Client lifecycle.
    pub clients: Option<ModClientDeclaration>,
    /// Console variables.
    pub cvars: Vec<ModCvar>,
    /// Initialization calls.
    pub initialize: Vec<ModSourceCall>,
    /// Per-frame call.
    pub frame: Option<ModSourceCall>,
    /// Console commands.
    pub commands: Vec<ModConsoleCommand>,
    /// Combat lowering.
    pub combat: Option<ModCombatDeclaration>,
    /// Protection channels.
    pub protection: Vec<ModQcProtection>,
    /// Pickup rules.
    pub pickups: Vec<ModPickupRule>,
    /// Source items.
    pub items: Option<ModQcItems>,
}
