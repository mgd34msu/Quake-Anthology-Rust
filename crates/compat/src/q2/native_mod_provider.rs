//! Port of `src/compat/q2/native-mod-provider.ts`.
//! Bridges the whole native mod: projections, source calls, clients, armor,
//! items, pickups, objectives, and saves compose here over synthetic hosts.
//!
//! This port is deliberately self-contained: the sibling `native_mod_*`
//! bridges own their domains while the provider defines local synthetic
//! host/service traits capturing the donor surface it orchestrates.

use std::collections::{HashMap, HashSet};

use qa_core::math::Vec3;
use qa_guest::core::contracts::{GuestAccess, GuestAddress, GuestCallResult, GuestCallValue, GuestStorage};
use qa_guest::core::memory::SparseGuestMemory;
use qa_guest::error::GuestError;
use thiserror::Error;

// ---------------------------------------------------------------------------
// Errors and identity
// ---------------------------------------------------------------------------

/// Failures across the native mod provider.
#[derive(Debug, Clone, PartialEq, Error)]
pub enum ProviderError {
    /// The declaration is invalid.
    #[error("invalid native mod declaration: {0}")]
    InvalidDeclaration(String),
    /// No source host is attached.
    #[error("native mod has no source host")]
    MissingHost,
    /// The provider is closed.
    #[error("native mod is closed")]
    Closed,
    /// The donor owner retired.
    #[error("native donor owner retired")]
    Retired,
    /// A record is not declared.
    #[error("unknown native actor record: {0}")]
    MissingRecord(String),
    /// A named export is not declared.
    #[error("native entry export is not declared: {0}")]
    MissingExport(String),
    /// A field exceeds its record.
    #[error("native mod field exceeds its source record")]
    FieldRange,
    /// Fields overlap.
    #[error("overlapping native mod fields")]
    Overlap,
    /// An address follows a null pointer.
    #[error("native mod layout follows a null source pointer")]
    NullPointer,
    /// A value exceeds its encoding.
    #[error("native mod value exceeds its encoding: {0}")]
    ValueRange(String),
    /// A callback result is not a scalar.
    #[error("native mod operation must return a scalar")]
    BadResult,
    /// A slot is outside its record.
    #[error("native slot is outside its source record")]
    SlotRange,
    /// A projection was lost.
    #[error("native actor projection was lost")]
    LostProjection,
    /// A client row is unavailable.
    #[error("native client has no source row")]
    MissingClientRow,
    /// A client identity is stale.
    #[error("native client identity is no longer live")]
    IdentityStale,
    /// Source client capacity is exceeded.
    #[error("native source client capacity exceeded")]
    CapacityExceeded,
    /// A checkpoint is invalid.
    #[error("invalid native mod checkpoint: {0}")]
    BadCheckpoint(String),
    /// A restore cannot complete.
    #[error("native mod restore failed: {0}")]
    BadRestore(String),
    /// Underlying guest failure.
    #[error("native mod guest failure: {0}")]
    Guest(String),
}

impl From<GuestError> for ProviderError {
    fn from(error: GuestError) -> Self {
        Self::Guest(error.to_string())
    }
}

/// Generational actor handle local to the provider.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct NativeActorId {
    /// Registry slot.
    pub slot: u32,
    /// Slot generation.
    pub generation: u32,
}

/// Generational client handle local to the provider.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct NativeClientId {
    /// Client slot.
    pub slot: u32,
    /// Slot generation.
    pub generation: u32,
}

/// Save-safe actor reference.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SavedActorId {
    /// Registry slot.
    pub slot: u32,
    /// Slot generation.
    pub generation: u32,
}

impl From<NativeActorId> for SavedActorId {
    fn from(id: NativeActorId) -> Self {
        Self {
            slot: id.slot,
            generation: id.generation,
        }
    }
}

/// Actor handle with its owning provider name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwnedActor {
    /// Actor handle.
    pub id: NativeActorId,
    /// Owning provider.
    pub owner: String,
}

// ---------------------------------------------------------------------------
// Synthetic host and service traits
// ---------------------------------------------------------------------------

/// Source entity table geometry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EntityTable {
    /// Table base address.
    pub base: GuestAddress,
    /// Row stride in bytes.
    pub stride: usize,
    /// Live row count.
    pub count: usize,
    /// Table capacity.
    pub capacity: usize,
}

/// Guest machine surface the provider needs.
pub trait ProviderHost {
    /// Guest memory.
    fn memory(&mut self) -> &mut SparseGuestMemory;
    /// Image base address.
    fn image_base(&self) -> GuestAddress;
    /// Named export address, if declared.
    fn entry(&self, name: &str) -> Option<GuestAddress>;
    /// Source entity table.
    fn entities(&self) -> EntityTable;
    /// View-model index for a 1-based client row.
    fn weapon_model(&self, slot: usize) -> u32;
    /// Invoke a guest entry with lowered arguments.
    fn invoke_entry(&mut self, address: GuestAddress, values: &[GuestCallValue]) -> GuestCallResult;
}

/// Shared world surface the provider needs.
pub trait ProviderServices {
    /// Current shared time in seconds.
    fn time_secs(&self) -> f64;
    /// Allocate an owned actor at a source slot.
    fn allocate_actor(&mut self, slot: usize, label: &str) -> OwnedActor;
    /// Whether an actor is live.
    fn actor_is_live(&self, actor: NativeActorId) -> bool;
    /// Release an owned actor.
    fn actor_release(&mut self, actor: OwnedActor);
    /// Resolve a saved reference.
    fn actor_reference(&mut self, saved: SavedActorId) -> NativeActorId;
    /// Resolve a live owned actor.
    fn actor_resolve(&self, actor: NativeActorId) -> Option<OwnedActor>;
    /// Source slot of an actor, if bound.
    fn actor_source_slot(&self, actor: NativeActorId) -> Option<usize>;
    /// Drain externally released actors.
    fn take_released_actors(&mut self) -> Vec<NativeActorId>;
    /// Client handle for an actor.
    fn client_for_actor(&self, actor: NativeActorId) -> Option<NativeClientId>;
    /// Actor for a client handle.
    fn client_actor(&self, client: NativeClientId) -> Option<NativeActorId>;
    /// All connected clients.
    fn client_list(&self) -> Vec<(NativeActorId, NativeClientId)>;
    /// Userinfo string for a client.
    fn client_userinfo(&self, client: NativeClientId) -> String;
    /// Store a userinfo string.
    fn client_set_userinfo(&mut self, client: NativeClientId, value: String);
    /// Drop a client with a reason.
    fn client_drop(&mut self, client: NativeClientId, reason: String);
    /// Canonical inventory count.
    fn inventory_count(&self, actor: NativeActorId, item: &str) -> f64;
    /// Store a canonical inventory count.
    fn inventory_set(&mut self, actor: NativeActorId, item: &str, count: f64);
    /// Whether an actor owns a canonical inventory item.
    fn inventory_owns(&self, actor: NativeActorId, item: &str) -> bool;
}

// ---------------------------------------------------------------------------
// Declaration types
// ---------------------------------------------------------------------------

/// Module target API.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApiKind {
    /// Classic game API.
    Classic,
    /// Rerelease game API.
    Rerelease,
}

/// Record base addressing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModRecordBase {
    /// Source public edict table.
    Entities,
    /// Client rows addressed through source edicts.
    Clients,
    /// Fixed source address.
    Address(ModAddress),
}

/// Shared field binding.
#[derive(Debug, Clone, PartialEq)]
pub enum ModFieldBinding {
    /// Private source storage.
    Private,
    /// Seeded scalar constant.
    Constant(f64),
    /// Seeded vector constant.
    ConstantVector(Vec3),
    /// Canonical inventory count.
    Inventory {
        /// Canonical item.
        item: String,
    },
    /// Canonical inventory capacity.
    InventoryCapacity {
        /// Canonical item.
        item: String,
    },
    /// Match team.
    Team,
    /// Match score.
    Score,
    /// Linked actor record.
    Record {
        /// Linked record id.
        record: String,
    },
    /// Objective address word.
    Address,
    /// Writable bounds minimum.
    BoundsMin,
    /// Writable bounds maximum.
    BoundsMax,
}

/// One record field.
#[derive(Debug, Clone, PartialEq)]
pub struct ModRecordField {
    /// Byte offset.
    pub offset: usize,
    /// Lane encoding.
    pub encoding: GuestStorage,
    /// Field binding.
    pub binding: ModFieldBinding,
}

/// Actor record declaration.
#[derive(Debug, Clone, PartialEq)]
pub struct ModActorRecord {
    /// Record id.
    pub id: String,
    /// Row stride in bytes.
    pub stride: usize,
    /// Row capacity.
    pub capacity: usize,
    /// First source slot.
    pub first_slot: usize,
    /// Base addressing.
    pub base: ModRecordBase,
    /// Declared fields.
    pub fields: Vec<ModRecordField>,
}

/// Source address with indirections.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModAddress {
    /// Image RVA.
    pub rva: u64,
    /// Pointer indirections.
    pub indirections: Vec<i64>,
}

/// Entry reference.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModEntryRef {
    /// Named export.
    Export(String),
    /// Image RVA.
    Rva(u64),
}

/// Call acceptance rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallAccepts {
    /// Any result accepts.
    Always,
    /// Zero rejects.
    NonZero,
}

/// Call return shape.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallReturns {
    /// No return value.
    Void,
    /// Integer decision.
    Int32,
}

/// Source exclusion region.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModSkip {
    /// Region entry RVA.
    pub entry: u64,
    /// Region join RVA.
    pub join: u64,
}

/// Lowered callback value.
#[derive(Debug, Clone, PartialEq)]
pub enum ModValueKind {
    /// Scalar input.
    Float {
        /// Input name.
        input: String,
    },
    /// Actor input.
    Actor {
        /// Input name.
        input: String,
        /// Actor record.
        record: String,
    },
    /// Client row input.
    Client {
        /// Input name.
        input: String,
    },
    /// Time input.
    Time {
        /// Input name.
        input: String,
        /// Lane encoding.
        encoding: GuestStorage,
    },
    /// Vector input.
    Vector {
        /// Input name.
        input: String,
    },
    /// String input.
    Text {
        /// Input name.
        input: String,
    },
    /// Fixed address.
    Address(ModAddress),
    /// Userinfo input.
    Userinfo {
        /// Input name.
        input: String,
    },
    /// Client user command.
    UserCommand,
}

/// Saved-and-restored global around a call.
#[derive(Debug, Clone, PartialEq)]
pub struct ModGlobal {
    /// Global address.
    pub address: ModAddress,
    /// Lane encoding.
    pub encoding: GuestStorage,
    /// Lowered value.
    pub value: ModValueKind,
}

/// Declared source call.
#[derive(Debug, Clone, PartialEq)]
pub struct ModCallDecl {
    /// Call id.
    pub id: String,
    /// Call entry.
    pub entry: ModEntryRef,
    /// Acceptance rule.
    pub accepts: CallAccepts,
    /// Return shape.
    pub returns: CallReturns,
    /// Exclusion regions.
    pub skips: Vec<ModSkip>,
    /// Lowered arguments.
    pub arguments: Vec<ModValueKind>,
    /// Saved globals.
    pub globals: Vec<ModGlobal>,
}

/// Client output declaration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModClientOutput {
    /// Scalar output word.
    Scalar {
        /// Record id.
        record: String,
        /// Byte offset.
        offset: usize,
        /// Lane encoding.
        encoding: GuestStorage,
    },
    /// Vector output word.
    Vector {
        /// Record id.
        record: String,
        /// Byte offset.
        offset: usize,
    },
    /// Body-shape output words.
    BodyShape {
        /// Minimums word.
        min: (String, usize),
        /// Maximums word.
        max: (String, usize),
    },
}

/// Staged client input field.
#[derive(Debug, Clone, PartialEq)]
pub struct ModInputField {
    /// Record id.
    pub record: String,
    /// Byte offset.
    pub offset: usize,
    /// Lowered value.
    pub value: ModValueKind,
}

/// Input binding declaration.
#[derive(Debug, Clone, PartialEq)]
pub struct ModInputBinding {
    /// Binding id.
    pub id: String,
    /// Binding calls.
    pub calls: Vec<ModCallDecl>,
}

/// Client pose declaration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModPose {
    /// View-height field.
    pub view_height: (String, usize, GuestStorage),
    /// Crouched bit field.
    pub crouched: (String, usize, GuestStorage),
    /// Crouch bitmask.
    pub crouch_mask: i64,
}

/// Client declaration.
#[derive(Debug, Clone, PartialEq)]
pub struct ModClientsDecl {
    /// Maximum source rows.
    pub maximum: usize,
    /// Private record arrays.
    pub records: Vec<String>,
    /// Admission calls.
    pub admit: Vec<ModCallDecl>,
    /// Userinfo calls.
    pub userinfo: Vec<ModCallDecl>,
    /// Disconnect calls.
    pub disconnect: Vec<ModCallDecl>,
    /// Command calls.
    pub command: Vec<ModCallDecl>,
    /// Frame calls.
    pub frame: Vec<ModCallDecl>,
    /// End-of-frame calls.
    pub end_frame: Vec<ModCallDecl>,
    /// Input bindings.
    pub input: Vec<ModInputBinding>,
    /// Staged input fields.
    pub input_fields: Vec<ModInputField>,
    /// Client outputs.
    pub outputs: Vec<ModClientOutput>,
    /// Client pose.
    pub pose: Option<ModPose>,
}

/// Protection channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ProtectionChannel {
    /// Regular armor.
    Regular,
    /// Powered protection.
    Powered,
}

/// Absorption declaration.
#[derive(Debug, Clone, PartialEq)]
pub enum ModAbsorb {
    /// Plain source call.
    SourceCall {
        /// Source call.
        call: ModCallDecl,
    },
    /// Scoped donor region.
    SourceRegion {
        /// Source call.
        call: ModCallDecl,
        /// Frame entry RVA.
        frame_entry: u64,
        /// Region entry RVA.
        entry_rva: u64,
        /// Region join RVA.
        join_rva: u64,
        /// Frame exit RVA.
        frame_exit: u64,
    },
    /// Direct native entry.
    Native {
        /// Native entry.
        entry: ModEntryRef,
        /// Spark count, if any.
        sparks: Option<f64>,
    },
}

/// Protection declaration.
#[derive(Debug, Clone, PartialEq)]
pub struct ModProtectionDecl {
    /// Definition id.
    pub id: String,
    /// Protection channel.
    pub channel: ProtectionChannel,
    /// Absorption declaration.
    pub absorb: ModAbsorb,
    /// Flags ABI.
    pub flags_abi: ApiKind,
}

/// Item kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModItemKind {
    /// Weapon item.
    Weapon,
    /// Any other item.
    Other,
}

/// Item definition.
#[derive(Debug, Clone, PartialEq)]
pub struct ModItemDef {
    /// Canonical item.
    pub item: String,
    /// Item kind.
    pub kind: ModItemKind,
    /// Ammo item, if any.
    pub ammo: Option<String>,
    /// Use action call, if any.
    pub use_call: Option<ModCallDecl>,
    /// Drop action call, if any.
    pub drop_call: Option<ModCallDecl>,
}

/// Items declaration.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ModItemsDecl {
    /// Item definitions.
    pub definitions: Vec<ModItemDef>,
    /// Weapon selection request calls.
    pub selection_requests: Vec<ModCallDecl>,
    /// Whether a dispatcher is declared.
    pub has_dispatcher: bool,
}

/// Pickup operation.
#[derive(Debug, Clone, PartialEq)]
pub enum ModPickupOp {
    /// Single grant call.
    BooleanGrant {
        /// Grant call.
        grant: ModCallDecl,
    },
    /// Gate call plus grant call.
    GateThenGrant {
        /// Gate call.
        gate: ModCallDecl,
        /// Grant call.
        grant: ModCallDecl,
    },
}

/// Pickup context field.
#[derive(Debug, Clone, PartialEq)]
pub struct ModContextField {
    /// Record id.
    pub record: String,
    /// Byte offset.
    pub offset: usize,
    /// Lowered value.
    pub value: ModValueKind,
}

/// Pickup declaration.
#[derive(Debug, Clone, PartialEq)]
pub struct ModPickupDecl {
    /// Rule id.
    pub id: String,
    /// Pickup operation.
    pub operation: ModPickupOp,
    /// Context fields.
    pub context: Vec<ModContextField>,
}

/// Objective declaration.
#[derive(Debug, Clone, PartialEq)]
pub struct ModObjectiveDecl {
    /// Objective id.
    pub id: String,
    /// State address, if any.
    pub state: Option<ModAddress>,
    /// Carrier address, if any.
    pub carrier: Option<ModAddress>,
    /// Target address, if any.
    pub target: Option<ModAddress>,
    /// Whether the objective is owned.
    pub owned: bool,
    /// Change call, if any.
    pub change: Option<ModCallDecl>,
}

/// Callback stage.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallbackStage {
    /// Observe only.
    Observe,
    /// Override.
    Override,
}

/// Component callback declaration.
#[derive(Debug, Clone, PartialEq)]
pub struct ModCallbackDecl {
    /// Callback id.
    pub id: String,
    /// Callback stage.
    pub stage: CallbackStage,
    /// Return shape.
    pub returns: CallReturns,
    /// Component operation name.
    pub operation: String,
    /// Source call.
    pub call: ModCallDecl,
}

/// Owned-actor lifetime words (pickup-context exclusion).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModOwnedLifetime {
    /// Ground pointer offset.
    pub ground: usize,
    /// Think pointer offset.
    pub think: usize,
    /// Use pointer offset, if any.
    pub use_offset: Option<usize>,
    /// Callback pointer offsets.
    pub callbacks: Vec<usize>,
    /// Next-think offset and encoding.
    pub nextthink: (usize, GuestStorage),
}

/// Source clock projection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModClockField {
    /// Destination address.
    pub address: ModAddress,
    /// Frame counter (`true`) or time (`false`).
    pub is_frame: bool,
    /// Milliseconds (`true`) or seconds (`false`).
    pub milliseconds: bool,
    /// Lane encoding.
    pub encoding: GuestStorage,
}

/// Owned-actor declaration.
#[derive(Debug, Clone, PartialEq)]
pub struct ModOwnedActorsDecl {
    /// Source frame period in seconds.
    pub frame_seconds: f64,
    /// Clock projections.
    pub clock: Vec<ModClockField>,
    /// Update entry.
    pub update: ModEntryRef,
    /// Callback ABI, if any.
    pub callbacks_abi: Option<ApiKind>,
    /// Damage ABI, if combat is declared.
    pub damage_abi: Option<ApiKind>,
    /// Whether causes use the classic edition.
    pub causes_classic: bool,
    /// Whether deferred damage is declared.
    pub deferred: bool,
    /// Lifetime words for exclusion checks.
    pub lifetime: Option<ModOwnedLifetime>,
}

/// Native mod declaration.
#[derive(Debug, Clone, PartialEq)]
pub struct NativeModDeclaration {
    /// Module target API.
    pub api: ApiKind,
    /// Guest pointer width.
    pub pointer_bytes: usize,
    /// Actor records.
    pub actor_records: Vec<ModActorRecord>,
    /// Engine entity record, if any.
    pub entity_record: Option<String>,
    /// Client declaration, if any.
    pub clients: Option<ModClientsDecl>,
    /// Component callbacks.
    pub callbacks: Vec<ModCallbackDecl>,
    /// Protection declarations.
    pub protection: Vec<ModProtectionDecl>,
    /// Items declaration, if any.
    pub items: Option<ModItemsDecl>,
    /// Pickup declarations.
    pub pickups: Vec<ModPickupDecl>,
    /// Objective declarations.
    pub objectives: Vec<ModObjectiveDecl>,
    /// Owned-actor declaration, if any.
    pub source_actors: Option<ModOwnedActorsDecl>,
    /// Initialize calls.
    pub initialize: Vec<ModCallDecl>,
    /// Projection calls.
    pub project: Vec<ModCallDecl>,
    /// Release calls.
    pub release: Vec<ModCallDecl>,
    /// Whether client presentation is declared.
    pub client_presentation: bool,
}

fn scalar_size(encoding: GuestStorage, pointer_bytes: usize) -> usize {
    encoding.byte_length(pointer_bytes)
}

fn value_length(value: &ModValueKind, pointer_bytes: usize) -> usize {
    match value {
        ModValueKind::Vector { .. } => 12,
        ModValueKind::Address(_) => pointer_bytes,
        ModValueKind::Time { encoding, .. } => scalar_size(*encoding, pointer_bytes),
        ModValueKind::Actor { .. }
        | ModValueKind::Client { .. }
        | ModValueKind::Text { .. }
        | ModValueKind::Userinfo { .. }
        | ModValueKind::UserCommand => pointer_bytes,
        ModValueKind::Float { .. } => 8,
    }
}

fn check_values(
    values: &[ModValueKind],
    available: &HashSet<&str>,
    has_clients: bool,
    records: &HashMap<String, ModActorRecord>,
) -> Result<(), ProviderError> {
    for value in values {
        match value {
            ModValueKind::UserCommand => {
                if !available.contains("view-angles") {
                    return Err(ProviderError::InvalidDeclaration(
                        "Native user command requires an active input application".to_string(),
                    ));
                }
            }
            ModValueKind::Address(_) => {}
            ModValueKind::Client { input } | ModValueKind::Userinfo { input } => {
                if !has_clients || !available.contains(input.as_str()) {
                    return Err(ProviderError::InvalidDeclaration(
                        "Unavailable native client input".to_string(),
                    ));
                }
            }
            ModValueKind::Actor { input, record } => {
                if !available.contains(input.as_str()) || !records.contains_key(record) {
                    return Err(ProviderError::InvalidDeclaration(
                        "Unavailable native actor input".to_string(),
                    ));
                }
            }
            ModValueKind::Time { input, .. } => {
                if !available.contains(input.as_str()) {
                    return Err(ProviderError::InvalidDeclaration(
                        "Unavailable native time input".to_string(),
                    ));
                }
            }
            ModValueKind::Vector { input } => {
                if !available.contains(input.as_str())
                    || !["point", "direction", "normal", "view-angles"].contains(&input.as_str())
                {
                    return Err(ProviderError::InvalidDeclaration(
                        "Native callback input representation differs from its declaration".to_string(),
                    ));
                }
            }
            ModValueKind::Text { input } => {
                if !available.contains(input.as_str()) || input != "item" {
                    return Err(ProviderError::InvalidDeclaration(
                        "Native callback input representation differs from its declaration".to_string(),
                    ));
                }
            }
            ModValueKind::Float { input } => {
                if !available.contains(input.as_str())
                    || ["point", "direction", "normal", "view-angles"].contains(&input.as_str())
                    || input == "item"
                    || ["self", "other", "activator", "attacker", "inflictor"].contains(&input.as_str())
                {
                    return Err(ProviderError::InvalidDeclaration(
                        "Native callback input representation differs from its declaration".to_string(),
                    ));
                }
            }
        }
    }
    Ok(())
}

fn check_call(
    call: &ModCallDecl,
    available: &HashSet<&str>,
    has_clients: bool,
    records: &HashMap<String, ModActorRecord>,
) -> Result<(), ProviderError> {
    let mut values = call.arguments.clone();
    values.extend(call.globals.iter().map(|global| global.value.clone()));
    check_values(&values, available, has_clients, records)?;
    for (index, region) in call.skips.iter().enumerate() {
        if region.join <= region.entry || call.skips[..index].iter().any(|other| other.entry == region.entry) {
            return Err(ProviderError::InvalidDeclaration(
                "Invalid or repeated native source exclusion".to_string(),
            ));
        }
    }
    Ok(())
}

fn available<'a>(inputs: &[&'a str]) -> HashSet<&'a str> {
    inputs.iter().copied().collect()
}

/// Validate a native mod declaration.
pub fn validate_native_mod_declaration(declaration: &NativeModDeclaration) -> Result<(), ProviderError> {
    let invalid = |message: &str| ProviderError::InvalidDeclaration(message.to_string());
    if declaration.pointer_bytes != 4 && declaration.pointer_bytes != 8 {
        return Err(invalid("pointer width"));
    }
    let owned = declaration.source_actors.as_ref();
    if let Some(owned) = owned {
        if !owned.frame_seconds.is_finite() || owned.frame_seconds <= 0.0 || owned.clock.is_empty() {
            return Err(invalid(
                "Native owned actors require a positive source frame period and clock",
            ));
        }
    }
    if owned.is_some_and(|owned| owned.callbacks_abi.is_some_and(|abi| abi != declaration.api)) {
        return Err(invalid("Native callback ABI differs from the selected module target"));
    }
    if let Some(owned) = owned {
        if let Some(damage) = owned.damage_abi {
            if owned.callbacks_abi.is_none()
                || damage != declaration.api
                || owned.causes_classic != (declaration.api == ApiKind::Classic)
            {
                return Err(invalid(
                    "Native combat requires its matching source callback and damage ABIs",
                ));
            }
        }
        if owned.deferred && declaration.api != ApiKind::Rerelease {
            return Err(invalid(
                "Native deferred damage requires its declared rerelease mod_t ABI",
            ));
        }
    }
    let mut records = HashMap::new();
    let mut authority = HashSet::new();
    for record in &declaration.actor_records {
        if record.id.is_empty()
            || records.contains_key(&record.id)
            || record.stride < 4
            || record.capacity < 1
            || record.capacity > 65536
        {
            return Err(invalid("Invalid native mod actor record"));
        }
        records.insert(record.id.clone(), record.clone());
        let mut occupied = HashSet::new();
        for field in &record.fields {
            let length = scalar_size(field.encoding, declaration.pointer_bytes);
            if length < 1 || field.offset + length > record.stride {
                return Err(invalid("Native mod field exceeds its source record"));
            }
            for byte in field.offset..field.offset + length {
                if !occupied.insert(byte) {
                    return Err(invalid("Overlapping native mod fields"));
                }
            }
            let shared_key = match &field.binding {
                ModFieldBinding::Inventory { item } => Some(format!("inventory:{item}")),
                ModFieldBinding::InventoryCapacity { item } => Some(format!("inventory-capacity:{item}")),
                ModFieldBinding::Team => Some("team".to_string()),
                ModFieldBinding::Score => Some("score".to_string()),
                _ => None,
            };
            if let Some(key) = shared_key {
                if !authority.insert(key.clone()) {
                    return Err(invalid(&format!("Multiple native stores for {key}")));
                }
            }
            if let ModFieldBinding::Constant(value) = &field.binding {
                check_scalar_range(*value, field.encoding, declaration.pointer_bytes)?;
            }
            if let ModFieldBinding::ConstantVector(value) = &field.binding {
                for component in [value.x, value.y, value.z] {
                    check_scalar_range(f64::from(component), GuestStorage::Float32, 4)?;
                }
            }
        }
    }
    if let Some(entity) = &declaration.entity_record {
        if !matches!(
            records.get(entity).map(|record| &record.base),
            Some(ModRecordBase::Entities)
        ) {
            return Err(invalid(
                "Native engine entity record must use the source public edict table",
            ));
        }
    }
    let clients = declaration.clients.as_ref();
    if declaration.client_presentation
        && (clients.is_none() || owned.is_none() || clients.map(|clients| clients.end_frame.len()).unwrap_or(0) == 0)
    {
        return Err(invalid(
            "Native client presentation requires declared original end-frame calls",
        ));
    }
    let mut channels = HashSet::new();
    for protection in &declaration.protection {
        if !channels.insert(protection.channel) {
            return Err(invalid("Duplicate native protection channel"));
        }
        if clients.is_none()
            || declaration.entity_record.is_none()
            || protection.id.is_empty()
            || protection.flags_abi != declaration.api
        {
            return Err(invalid(
                "Native protection requires declared clients and its matching source ABI",
            ));
        }
    }
    if let Some(clients) = clients {
        if clients.maximum < 1
            || clients.maximum > 256
            || clients.records.is_empty()
            || clients.records.iter().collect::<HashSet<_>>().len() != clients.records.len()
            || declaration.entity_record.is_none()
            || declaration
                .entity_record
                .as_ref()
                .and_then(|id| records.get(id))
                .map(|record| record.first_slot)
                != Some(1)
        {
            return Err(invalid("Invalid native component client layout"));
        }
        for id in &clients.records {
            let record = records
                .get(id)
                .ok_or_else(|| invalid("Invalid native component client layout"))?;
            if matches!(record.base, ModRecordBase::Entities) || record.capacity < clients.maximum {
                return Err(invalid(
                    "Native component clients require declared private record arrays",
                ));
            }
            if matches!(record.base, ModRecordBase::Clients)
                && (record.first_slot != 0 || record.capacity != clients.maximum)
            {
                return Err(invalid(
                    "Public native client rows must match the reserved source slots",
                ));
            }
        }
        for call in &clients.admit {
            if call.accepts == CallAccepts::NonZero && call.returns == CallReturns::Void {
                return Err(invalid("Native client admission requires its declared return value"));
            }
            if matches!(&call.entry, ModEntryRef::Export(name) if name == "ClientConnect")
                && call.accepts != CallAccepts::NonZero
            {
                return Err(invalid("Original native ClientConnect rejection cannot be ignored"));
            }
        }
        for record in records.values() {
            if record.capacity < clients.maximum {
                return Err(invalid("Native actor array cannot hold its declared clients"));
            }
        }
        // Client outputs need exclusive private storage.
        for output in &clients.outputs {
            let (record_id, offset, length) = match output {
                ModClientOutput::Scalar {
                    record,
                    offset,
                    encoding,
                } => (record, *offset, scalar_size(*encoding, declaration.pointer_bytes)),
                ModClientOutput::Vector { record, offset } => (record, *offset, 12),
                ModClientOutput::BodyShape { .. } => continue,
            };
            let record = records
                .get(record_id)
                .ok_or_else(|| invalid("Native client output requires exclusive private client storage"))?;
            let owned_here =
                clients.records.contains(&record.id) || Some(&record.id) == declaration.entity_record.as_ref();
            let inside_private = record.fields.iter().any(|field| {
                matches!(field.binding, ModFieldBinding::Private)
                    && field.offset <= offset
                    && offset + length <= field.offset + scalar_size(field.encoding, declaration.pointer_bytes)
            });
            if !owned_here || !inside_private {
                return Err(invalid(
                    "Native client output requires exclusive private client storage",
                ));
            }
        }
        for output in &clients.outputs {
            if let ModClientOutput::BodyShape { min, max } = output {
                for ((record_id, offset), binding) in [(min, "bounds-min"), (max, "bounds-max")] {
                    let record = records.get(record_id);
                    let bound = matches!(binding, "bounds-min");
                    let ok = record.is_some_and(|record| {
                        record.fields.iter().any(|field| {
                            field.offset == *offset
                                && (if bound {
                                    matches!(field.binding, ModFieldBinding::BoundsMin)
                                } else {
                                    matches!(field.binding, ModFieldBinding::BoundsMax)
                                })
                        })
                    });
                    if !ok {
                        return Err(invalid("Client body shape must name its writable source mins/maxs"));
                    }
                }
            }
        }
        // Input fields need declared private storage without overlaps.
        if !clients.input_fields.is_empty() && clients.input.is_empty() {
            return Err(invalid("Native input fields require declared input callbacks"));
        }
        let mut ranges: HashMap<&str, Vec<(usize, usize)>> = HashMap::new();
        for field in &clients.input_fields {
            let length = value_length(&field.value, declaration.pointer_bytes);
            let record = records
                .get(&field.record)
                .ok_or_else(|| invalid("Native input field requires declared private client storage"))?;
            let owned_here =
                Some(&record.id) == declaration.entity_record.as_ref() || clients.records.contains(&record.id);
            let inside_private = record.fields.iter().any(|candidate| {
                matches!(candidate.binding, ModFieldBinding::Private)
                    && field.offset >= candidate.offset
                    && field.offset + length
                        <= candidate.offset + scalar_size(candidate.encoding, declaration.pointer_bytes)
            });
            if !owned_here || field.offset + length > record.stride || !inside_private {
                return Err(invalid("Native input field requires declared private client storage"));
            }
            let previous = ranges.entry(field.record.as_str()).or_default();
            if previous
                .iter()
                .any(|(start, end)| field.offset < *end && *start < field.offset + length)
            {
                return Err(invalid("Overlapping native input fields"));
            }
            previous.push((field.offset, field.offset + length));
        }
        if let Some(pose) = &clients.pose {
            for field in [&pose.view_height, &pose.crouched] {
                let (record_id, offset, encoding) = (&field.0, field.1, field.2);
                let record = records
                    .get(record_id)
                    .ok_or_else(|| invalid("Native client pose requires separate private source fields"))?;
                let length = scalar_size(encoding, declaration.pointer_bytes);
                let shared_hit = record.fields.iter().any(|field| {
                    matches!(
                        field.binding,
                        ModFieldBinding::Inventory { .. }
                            | ModFieldBinding::InventoryCapacity { .. }
                            | ModFieldBinding::Team
                            | ModFieldBinding::Score
                    ) && offset < field.offset + scalar_size(field.encoding, declaration.pointer_bytes)
                        && field.offset < offset + length
                });
                if offset + length > record.stride || shared_hit {
                    return Err(invalid("Native client pose requires separate private source fields"));
                }
            }
            if pose.crouch_mask < 1 || pose.crouch_mask > 0x7fff_ffff {
                return Err(invalid("Invalid native source crouch mask"));
            }
        }
    }
    for record in records.values() {
        for field in &record.fields {
            if let ModFieldBinding::Record { record: linked } = &field.binding {
                if !records.contains_key(linked) {
                    return Err(invalid("Unknown linked native actor record"));
                }
            }
        }
        if matches!(record.base, ModRecordBase::Clients)
            && !clients.is_some_and(|clients| clients.records.contains(&record.id))
        {
            return Err(invalid(
                "Native public client records require declared client ownership",
            ));
        }
    }
    if !records.is_empty() && (declaration.project.is_empty() || declaration.release.is_empty()) {
        return Err(invalid(
            "Native actor projections require authored initialization and release callbacks",
        ));
    }
    let has_clients = clients.is_some();
    for objective in &declaration.objectives {
        for address in [&objective.state, &objective.carrier, &objective.target]
            .into_iter()
            .flatten()
        {
            check_values(
                &[ModValueKind::Address(address.clone())],
                &available(&[]),
                has_clients,
                &records,
            )?;
        }
        if objective.owned {
            if let Some(change) = &objective.change {
                check_call(
                    change,
                    &available(&["self", "other", "activator", "amount", "time"]),
                    has_clients,
                    &records,
                )?;
            }
        }
    }
    for pickup in &declaration.pickups {
        let available = available(&[
            "self",
            "other",
            "item",
            "time",
            "pickup-count",
            "pickup-has-count",
            "pickup-dropped",
        ]);
        match &pickup.operation {
            ModPickupOp::BooleanGrant { grant } => check_call(grant, &available, has_clients, &records)?,
            ModPickupOp::GateThenGrant { gate, grant } => {
                check_call(gate, &available, has_clients, &records)?;
                check_call(grant, &available, has_clients, &records)?;
            }
        }
        let mut ranges: Vec<(&str, usize, usize)> = Vec::new();
        for field in &pickup.context {
            check_values(std::slice::from_ref(&field.value), &available, has_clients, &records)?;
            let length = value_length(&field.value, declaration.pointer_bytes);
            let record = records
                .get(&field.record)
                .ok_or_else(|| invalid("Native pickup context requires separate declared source storage"))?;
            let client_owned = clients.is_some_and(|clients| clients.records.contains(&record.id));
            let inside = record.fields.iter().any(|candidate| {
                matches!(
                    candidate.binding,
                    ModFieldBinding::Private
                        | ModFieldBinding::Constant(_)
                        | ModFieldBinding::Address
                        | ModFieldBinding::ConstantVector(_)
                ) && candidate.offset <= field.offset
                    && field.offset + length
                        <= candidate.offset + scalar_size(candidate.encoding, declaration.pointer_bytes)
            });
            if client_owned
                || field.offset + length > record.stride
                || !inside
                || ranges.iter().any(|(record_id, start, end)| {
                    *record_id == field.record.as_str() && field.offset < *end && *start < field.offset + length
                })
            {
                return Err(invalid(
                    "Native pickup context requires separate declared source storage",
                ));
            }
            ranges.push((field.record.as_str(), field.offset, field.offset + length));
            if Some(&record.id) == declaration.entity_record.as_ref() {
                if let Some(lifetime) = owned.and_then(|owned| owned.lifetime.as_ref()) {
                    let mut pointers = vec![lifetime.ground, lifetime.think];
                    pointers.extend(lifetime.use_offset);
                    pointers.extend(lifetime.callbacks.iter().copied());
                    if pointers.iter().any(|offset| {
                        field.offset < offset + declaration.pointer_bytes && *offset < field.offset + length
                    }) || field.offset
                        < lifetime.nextthink.0 + scalar_size(lifetime.nextthink.1, declaration.pointer_bytes)
                        && lifetime.nextthink.0 < field.offset + length
                    {
                        return Err(invalid("Native pickup context overlaps source actor lifetime"));
                    }
                }
            }
        }
    }
    for protection in &declaration.protection {
        let available = available(&[
            "self",
            "attacker",
            "inflictor",
            "time",
            "amount",
            "damage-flags",
            "regular-protection-scale",
            "knockback",
            "direction",
            "point",
            "normal",
        ]);
        match &protection.absorb {
            ModAbsorb::SourceCall { call } => {
                if call.returns == CallReturns::Void {
                    return Err(invalid("Native protection requires a source return value"));
                }
                check_call(call, &available, has_clients, &records)?;
            }
            ModAbsorb::SourceRegion {
                call,
                frame_entry,
                entry_rva,
                join_rva,
                frame_exit,
            } => {
                let boundaries = [*frame_entry, *entry_rva, *join_rva, *frame_exit];
                let mut sorted = boundaries;
                sorted.sort_unstable();
                if sorted.windows(2).any(|pair| pair[0] == pair[1]) {
                    return Err(invalid("Invalid standalone native region frame"));
                }
                check_call(call, &available, has_clients, &records)?;
            }
            ModAbsorb::Native { .. } => {}
        }
    }
    if let Some(items) = &declaration.items {
        for item in &items.definitions {
            for call in [&item.use_call, &item.drop_call].into_iter().flatten() {
                check_call(call, &available(&["self", "time"]), has_clients, &records)?;
            }
        }
        for request in &items.selection_requests {
            check_call(request, &available(&["self", "time"]), has_clients, &records)?;
        }
    }
    for call in &declaration.initialize {
        check_call(call, &available(&["time"]), has_clients, &records)?;
    }
    for call in declaration.project.iter().chain(declaration.release.iter()) {
        check_call(call, &available(&["self", "time"]), has_clients, &records)?;
    }
    if let Some(clients) = clients {
        for call in clients
            .admit
            .iter()
            .chain(clients.userinfo.iter())
            .chain(clients.disconnect.iter())
            .chain(clients.command.iter())
            .chain(clients.frame.iter())
            .chain(clients.end_frame.iter())
        {
            check_call(call, &available(&["self", "time"]), has_clients, &records)?;
        }
        if (!clients.frame.is_empty() || !clients.end_frame.is_empty()) && owned.is_none() {
            return Err(invalid("Native client frames require the original source actor clock"));
        }
        let available = available(&[
            "self",
            "time",
            "elapsed",
            "view-angles",
            "attack",
            "jump",
            "impulse",
            "forward-move",
            "side-move",
            "up-move",
        ]);
        for binding in &clients.input {
            for call in &binding.calls {
                check_call(call, &available, has_clients, &records)?;
            }
        }
        for field in &clients.input_fields {
            check_values(std::slice::from_ref(&field.value), &available, has_clients, &records)?;
        }
    }
    let mut ids = HashSet::new();
    for callback in &declaration.callbacks {
        if !ids.insert(callback.id.clone())
            || callback.stage != CallbackStage::Observe && callback.returns == CallReturns::Void
        {
            return Err(invalid("Duplicate native callback or missing return value"));
        }
        let mut available_set = available(&["self", "time"]);
        if callback.stage == CallbackStage::Observe {
            available_set.insert("result");
        }
        let extra: &[&str] = match callback.operation.as_str() {
            "damage" => &[
                "attacker",
                "inflictor",
                "amount",
                "knockback",
                "direction",
                "point",
                "normal",
            ],
            "inventory.give" | "inventory.consume" => &["item", "amount"],
            "actor.use" => &["other", "activator"],
            "actor.touch" => &["other"],
            "actor.think" => &["elapsed"],
            "actor.pain" => &["attacker", "amount", "knockback"],
            _ => &["attacker", "inflictor", "amount", "knockback", "point"],
        };
        for name in extra {
            available_set.insert(name);
        }
        check_call(&callback.call, &available_set, has_clients, &records)?;
    }
    Ok(())
}

fn check_scalar_range(value: f64, encoding: GuestStorage, pointer_bytes: usize) -> Result<(), ProviderError> {
    if !value.is_finite() {
        return Err(ProviderError::ValueRange(format!("{encoding:?}")));
    }
    if encoding.is_float() {
        return Ok(());
    }
    let bits = scalar_size(encoding, pointer_bytes) * 8;
    let unsigned = matches!(
        encoding,
        GuestStorage::Uint8 | GuestStorage::Uint16 | GuestStorage::Uint32 | GuestStorage::Uint64
    );
    let bound = 2f64.powi(bits as i32 - i32::from(!unsigned));
    if value.trunc() != value || value < if unsigned { 0.0 } else { -bound } || value >= bound {
        return Err(ProviderError::ValueRange(format!("{encoding:?}")));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Runtime values and codec helpers
// ---------------------------------------------------------------------------

/// Runtime input value.
#[derive(Debug, Clone, PartialEq)]
pub enum RuntimeValue {
    /// Actor handle.
    Actor(Option<NativeActorId>),
    /// Scalar.
    Float(f64),
    /// String.
    Text(String),
    /// Vector.
    Vector(Vec3),
    /// Resolved address.
    Address(GuestAddress),
}

fn scalar_to_f64(bytes: &[u8], storage: GuestStorage) -> f64 {
    match storage {
        GuestStorage::Float32 => f64::from(f32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])),
        GuestStorage::Float64 => {
            let mut word = [0u8; 8];
            word.copy_from_slice(&bytes[..8]);
            f64::from_le_bytes(word)
        }
        GuestStorage::Int8 => f64::from(bytes[0] as i8),
        GuestStorage::Uint8 => f64::from(bytes[0]),
        GuestStorage::Int16 => f64::from(i16::from_le_bytes([bytes[0], bytes[1]])),
        GuestStorage::Uint16 => f64::from(u16::from_le_bytes([bytes[0], bytes[1]])),
        GuestStorage::Int32 => f64::from(i32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])),
        GuestStorage::Uint32 => f64::from(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])),
        GuestStorage::Int64 => {
            let mut word = [0u8; 8];
            word.copy_from_slice(&bytes[..8]);
            i64::from_le_bytes(word) as f64
        }
        GuestStorage::Uint64 => {
            let mut word = [0u8; 8];
            word.copy_from_slice(&bytes[..8]);
            u64::from_le_bytes(word) as f64
        }
        GuestStorage::Pointer => {
            let mut word = [0u8; 8];
            word.copy_from_slice(&bytes[..bytes.len().min(8)]);
            u64::from_le_bytes(word) as f64
        }
    }
}

fn f64_to_scalar(value: f64, storage: GuestStorage, pointer_bytes: usize) -> Vec<u8> {
    match storage {
        GuestStorage::Float32 => (value as f32).to_le_bytes().to_vec(),
        GuestStorage::Float64 => value.to_le_bytes().to_vec(),
        GuestStorage::Int8 | GuestStorage::Uint8 => vec![value as i64 as u8],
        GuestStorage::Int16 | GuestStorage::Uint16 => (value as i64 as i16).to_le_bytes().to_vec(),
        GuestStorage::Int32 | GuestStorage::Uint32 => (value as i64 as i32).to_le_bytes().to_vec(),
        GuestStorage::Int64 | GuestStorage::Uint64 => (value as i64).to_le_bytes().to_vec(),
        GuestStorage::Pointer => {
            if pointer_bytes == 4 {
                (value as u64 as u32).to_le_bytes().to_vec()
            } else {
                (value as u64).to_le_bytes().to_vec()
            }
        }
    }
}

fn vec_to_bytes(value: Vec3) -> [u8; 12] {
    let mut bytes = [0u8; 12];
    bytes[..4].copy_from_slice(&value.x.to_le_bytes());
    bytes[4..8].copy_from_slice(&value.y.to_le_bytes());
    bytes[8..].copy_from_slice(&value.z.to_le_bytes());
    bytes
}

// ---------------------------------------------------------------------------
// Provider state
// ---------------------------------------------------------------------------

/// One transfer frame with its projection observations.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TransferFrame {
    /// Observed shared words at frame entry.
    pub observations: HashMap<(usize, String, usize), f64>,
}

/// Client slot binding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientSlot {
    /// Client handle.
    pub client: NativeClientId,
    /// Source slot.
    pub slot: usize,
    /// Whether admission completed.
    pub admitted: bool,
}

/// Staged input field write.
#[derive(Debug, Clone, PartialEq, Eq)]
struct InputStore {
    address: GuestAddress,
    previous: Vec<u8>,
}

/// Client input application.
#[derive(Debug, Clone, PartialEq)]
pub struct InputApplication {
    /// Acting actor.
    pub actor: NativeActorId,
    /// Acting client.
    pub client: NativeClientId,
    /// Input values by name.
    pub values: HashMap<String, RuntimeValue>,
}

/// Saved actor row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SavedProjection {
    /// Saved actor.
    pub actor: SavedActorId,
    /// Source slot.
    pub slot: usize,
    /// Whether appearance was published.
    pub appearance: bool,
}

/// Saved client row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SavedClientRow {
    /// Saved actor.
    pub actor: SavedActorId,
    /// Source slot.
    pub slot: usize,
    /// Whether admission completed.
    pub admitted: bool,
}

/// Saved provider state.
#[derive(Debug, Clone, PartialEq)]
pub struct ProviderCheckpoint {
    /// Checkpoint version.
    pub version: u32,
    /// Map name.
    pub map: String,
    /// Module API.
    pub api: ApiKind,
    /// Projected actors.
    pub actors: Vec<SavedProjection>,
    /// Client rows.
    pub clients: Vec<SavedClientRow>,
    /// Owned frame state, if declared.
    pub owned: Option<(f64, u64)>,
    /// Shared word values.
    pub shared: HashMap<(usize, String, usize), f64>,
}

/// Native mod provider over synthetic host and services.
pub struct NativeModProvider<H: ProviderHost, S: ProviderServices> {
    declaration: NativeModDeclaration,
    host: H,
    services: S,
    instance: String,
    map: String,
    records: HashMap<String, ModActorRecord>,
    projections: HashMap<NativeActorId, usize>,
    pending_releases: HashSet<NativeActorId>,
    appearance: HashSet<NativeActorId>,
    frames: Vec<TransferFrame>,
    client_slots: HashMap<NativeActorId, ClientSlot>,
    denied: HashSet<NativeActorId>,
    authorities: HashMap<u64, NativeActorId>,
    next_authority: u64,
    active_skips: Vec<(u64, u64)>,
    input_staging: Vec<InputStore>,
    owned_frame: u64,
    owned_next: f64,
    userinfo_staged: HashMap<NativeActorId, String>,
    closed: bool,
    closing: bool,
    ready: bool,
    lifecycle: bool,
}

impl<H: ProviderHost, S: ProviderServices> NativeModProvider<H, S> {
    /// Build the provider, validating the declaration.
    pub fn new(
        declaration: NativeModDeclaration,
        host: H,
        services: S,
        instance: &str,
        map: &str,
    ) -> Result<Self, ProviderError> {
        validate_native_mod_declaration(&declaration)?;
        let mut records = HashMap::new();
        for record in &declaration.actor_records {
            records.insert(record.id.clone(), record.clone());
        }
        Ok(Self {
            declaration,
            host,
            services,
            instance: instance.to_string(),
            map: map.to_string(),
            records,
            projections: HashMap::new(),
            pending_releases: HashSet::new(),
            appearance: HashSet::new(),
            frames: Vec::new(),
            client_slots: HashMap::new(),
            denied: HashSet::new(),
            authorities: HashMap::new(),
            next_authority: 1,
            active_skips: Vec::new(),
            input_staging: Vec::new(),
            owned_frame: 0,
            owned_next: 0.0,
            userinfo_staged: HashMap::new(),
            closed: false,
            closing: false,
            ready: false,
            lifecycle: false,
        })
    }

    /// Owning provider name.
    #[must_use]
    pub fn instance(&self) -> &str {
        &self.instance
    }

    /// Map name.
    #[must_use]
    pub fn map(&self) -> &str {
        &self.map
    }

    fn current(&self) -> Result<(), ProviderError> {
        if self.closed {
            return Err(ProviderError::Closed);
        }
        Ok(())
    }

    // -- Address resolution, record addressing, scalar codec ----------------

    /// Resolve a source address through its indirections.
    pub fn resolve(&mut self, address: &ModAddress) -> Result<GuestAddress, ProviderError> {
        let base = self.host.image_base();
        let mut resolved = self
            .host
            .memory()
            .offset(base, i64::try_from(address.rva).unwrap_or(i64::MAX))?;
        for offset in &address.indirections {
            let pointer = self.host.memory().read_pointer(resolved)?;
            let Some(pointer) = pointer else {
                return Err(ProviderError::NullPointer);
            };
            resolved = self.host.memory().offset(pointer, *offset)?;
        }
        Ok(resolved)
    }

    /// Resolve an entry reference.
    pub fn resolve_entry(&mut self, entry: &ModEntryRef) -> Result<GuestAddress, ProviderError> {
        match entry {
            ModEntryRef::Export(name) => self
                .host
                .entry(name)
                .ok_or_else(|| ProviderError::MissingExport(name.clone())),
            ModEntryRef::Rva(rva) => {
                let base = self.host.image_base();
                Ok(self
                    .host
                    .memory()
                    .offset(base, i64::try_from(*rva).unwrap_or(i64::MAX))?)
            }
        }
    }

    fn record_base(&mut self, record: &ModActorRecord) -> Result<GuestAddress, ProviderError> {
        match &record.base {
            ModRecordBase::Address(address) => {
                let base = self.resolve(address)?;
                Ok(self
                    .host
                    .memory()
                    .offset(base, (record.first_slot * record.stride) as i64)?)
            }
            ModRecordBase::Clients => Err(ProviderError::InvalidDeclaration(
                "Native private client rows are addressed through their source edicts".to_string(),
            )),
            ModRecordBase::Entities => {
                let entities = self.host.entities();
                if entities.stride != record.stride || record.first_slot + record.capacity > entities.capacity {
                    return Err(ProviderError::InvalidDeclaration(
                        "Declared native actor layout differs from the source export table".to_string(),
                    ));
                }
                Ok(self
                    .host
                    .memory()
                    .offset(entities.base, (record.first_slot * record.stride) as i64)?)
            }
        }
    }

    /// Row address for a record and slot.
    pub fn record_address(&mut self, record_id: &str, slot: usize) -> Result<GuestAddress, ProviderError> {
        let record = self
            .records
            .get(record_id)
            .cloned()
            .ok_or_else(|| ProviderError::MissingRecord(record_id.to_string()))?;
        if slot >= record.capacity {
            return Err(ProviderError::SlotRange);
        }
        if matches!(record.base, ModRecordBase::Clients) {
            let entities = self.host.entities();
            return Ok(self
                .host
                .memory()
                .offset(entities.base, ((record.first_slot + slot) * record.stride) as i64)?);
        }
        let base = self.record_base(&record)?;
        Ok(self.host.memory().offset(base, (slot * record.stride) as i64)?)
    }

    /// Read a scalar from guest memory.
    pub fn scalar_read(&mut self, address: GuestAddress, encoding: GuestStorage) -> Result<f64, ProviderError> {
        let pointer_bytes = self.host.memory().pointer_bytes();
        let width = scalar_size(encoding, pointer_bytes);
        Ok(scalar_to_f64(&self.host.memory().copy(address, width)?, encoding))
    }

    /// Write a scalar into guest memory.
    pub fn scalar_write(
        &mut self,
        address: GuestAddress,
        value: f64,
        encoding: GuestStorage,
    ) -> Result<(), ProviderError> {
        check_scalar_range(value, encoding, self.host.memory().pointer_bytes())?;
        let pointer_bytes = self.host.memory().pointer_bytes();
        Ok(self
            .host
            .memory()
            .write(address, &f64_to_scalar(value, encoding, pointer_bytes))?)
    }

    /// Record base for a projected actor, if projected.
    pub fn pointer(&mut self, actor: NativeActorId, record_id: &str) -> Option<GuestAddress> {
        let slot = *self.projections.get(&actor)?;
        self.record_address(record_id, slot).ok()
    }

    /// Entity address for a projected actor.
    pub fn address(&mut self, actor: NativeActorId) -> Result<GuestAddress, ProviderError> {
        let entity = self
            .declaration
            .entity_record
            .clone()
            .ok_or(ProviderError::LostProjection)?;
        let slot = self
            .projections
            .get(&actor)
            .copied()
            .ok_or(ProviderError::LostProjection)?;
        self.record_address(&entity, slot)
    }

    /// Projected slot for an actor.
    #[must_use]
    pub fn slot_of(&self, actor: NativeActorId) -> Option<usize> {
        self.projections.get(&actor).copied()
    }

    /// Projected actor at a slot.
    #[must_use]
    pub fn actor_at(&self, slot: usize) -> Option<NativeActorId> {
        self.projections
            .iter()
            .find(|(_, bound)| **bound == slot)
            .map(|(actor, _)| *actor)
    }

    // -- Actor projections -------------------------------------------------

    /// Project an actor into a source slot.
    pub fn project(&mut self, slot: usize) -> Result<OwnedActor, ProviderError> {
        self.current()?;
        let actor = self.services.allocate_actor(slot, "native:mod-actor");
        self.projections.insert(actor.id, slot);
        if let Err(error) = self.seed(slot) {
            self.projections.remove(&actor.id);
            return Err(error);
        }
        let mut inputs = HashMap::new();
        inputs.insert("self".to_string(), RuntimeValue::Actor(Some(actor.id)));
        inputs.insert("time".to_string(), RuntimeValue::Float(self.services.time_secs()));
        for call in self.declaration.project.clone() {
            self.execute(&call, &inputs)?;
        }
        self.appearance.insert(actor.id);
        Ok(actor)
    }

    fn seed(&mut self, slot: usize) -> Result<(), ProviderError> {
        for record in self.declaration.actor_records.clone() {
            for field in record.fields {
                let address = self.record_address(&record.id, slot)?;
                match field.binding {
                    ModFieldBinding::Constant(value) => {
                        self.scalar_write(address, value, field.encoding)?;
                    }
                    ModFieldBinding::ConstantVector(value) => {
                        self.host.memory().write(address, &vec_to_bytes(value))?;
                    }
                    ModFieldBinding::Address => {
                        self.host.memory().write_pointer(address, None)?;
                    }
                    _ => {}
                }
            }
        }
        Ok(())
    }

    /// Drain externally released actors into the pending set.
    pub fn poll_releases(&mut self) {
        for actor in self.services.take_released_actors() {
            if self.projections.contains_key(&actor) {
                self.pending_releases.insert(actor);
            }
        }
    }

    /// Release pending actors through release calls.
    pub fn release_pending(&mut self) -> Result<(), ProviderError> {
        for actor in std::mem::take(&mut self.pending_releases) {
            if self.services.actor_is_live(actor) {
                let mut inputs = HashMap::new();
                inputs.insert("self".to_string(), RuntimeValue::Actor(Some(actor)));
                inputs.insert("time".to_string(), RuntimeValue::Float(self.services.time_secs()));
                for call in self.declaration.release.clone() {
                    self.execute(&call, &inputs)?;
                }
            }
            self.projections.remove(&actor);
            self.appearance.remove(&actor);
            self.client_slots.remove(&actor);
        }
        Ok(())
    }

    // -- Shared-field projection -------------------------------------------

    fn shared_fields(&self) -> Vec<(String, ModRecordField)> {
        let mut fields = Vec::new();
        for record in &self.declaration.actor_records {
            for field in &record.fields {
                if matches!(
                    field.binding,
                    ModFieldBinding::Inventory { .. } | ModFieldBinding::InventoryCapacity { .. }
                ) {
                    fields.push((record.id.clone(), field.clone()));
                }
            }
        }
        fields
    }

    /// Publish shared words into canonical inventory.
    pub fn refresh(&mut self) -> Result<(), ProviderError> {
        for (record_id, field) in self.shared_fields() {
            let (item, is_capacity) = match &field.binding {
                ModFieldBinding::Inventory { item } => (item.clone(), false),
                ModFieldBinding::InventoryCapacity { item } => (item.clone(), true),
                _ => continue,
            };
            let _ = is_capacity;
            for (actor, slot) in self.projections.clone() {
                let address = self.record_address(&record_id, slot)?;
                let value = self.scalar_read(address, field.encoding)?;
                if self.services.inventory_owns(actor, &item) {
                    self.services.inventory_set(actor, &item, value);
                }
            }
        }
        Ok(())
    }

    /// Snapshot shared words for a transfer frame.
    pub fn observe(&mut self) -> Result<HashMap<(usize, String, usize), f64>, ProviderError> {
        let mut snapshot = HashMap::new();
        for (record_id, field) in self.shared_fields() {
            for slot in self.projections.values().copied().collect::<Vec<_>>() {
                let address = self.record_address(&record_id, slot)?;
                let value = self.scalar_read(address, field.encoding)?;
                snapshot.insert((slot, record_id.clone(), field.offset), value);
            }
        }
        Ok(snapshot)
    }

    /// Commit shared-word changes into canonical inventory.
    pub fn flush(&mut self, committed: &mut dyn FnMut(NativeActorId, &str, f64, f64)) -> Result<(), ProviderError> {
        let before = self.frames.last().cloned().unwrap_or_default();
        let after = self.observe()?;
        for ((slot, record_id, offset), value) in &after {
            let previous = before.observations.get(&(*slot, record_id.clone(), *offset));
            if previous == Some(value) {
                continue;
            }
            let Some(actor) = self.actor_at(*slot) else { continue };
            let field = self
                .records
                .get(record_id)
                .and_then(|record| record.fields.iter().find(|field| field.offset == *offset));
            let Some(field) = field else { continue };
            let item = match &field.binding {
                ModFieldBinding::Inventory { item } | ModFieldBinding::InventoryCapacity { item } => item.clone(),
                _ => continue,
            };
            if self.services.inventory_owns(actor, &item) {
                let baseline = self.services.inventory_count(actor, &item);
                self.services.inventory_set(actor, &item, *value);
                committed(actor, &item, baseline, *value);
            }
        }
        if let Some(frame) = self.frames.last_mut() {
            frame.observations = after;
        }
        Ok(())
    }

    // -- Source calls ------------------------------------------------------

    /// Lower one value against ambient inputs.
    pub fn lower(
        &mut self,
        value: &ModValueKind,
        inputs: &HashMap<String, RuntimeValue>,
    ) -> Result<GuestCallValue, ProviderError> {
        match value {
            ModValueKind::Float { input } => match inputs.get(input) {
                Some(RuntimeValue::Float(value)) => Ok(GuestCallValue::Float64(*value)),
                _ => Err(ProviderError::InvalidDeclaration(format!(
                    "Unavailable native callback input {input}"
                ))),
            },
            ModValueKind::Actor { input, .. } => match inputs.get(input) {
                Some(RuntimeValue::Actor(Some(actor))) => Ok(GuestCallValue::Pointer(Some(self.address(*actor)?))),
                Some(RuntimeValue::Actor(None)) => Ok(GuestCallValue::Pointer(None)),
                _ => Err(ProviderError::InvalidDeclaration(format!(
                    "Unavailable native callback input {input}"
                ))),
            },
            ModValueKind::Client { input } => match inputs.get(input) {
                Some(RuntimeValue::Actor(Some(actor))) => {
                    let slot = self.client_slots.get(actor).map(|slot| slot.slot as i32).unwrap_or(-1);
                    Ok(GuestCallValue::Int32(slot))
                }
                _ => Err(ProviderError::InvalidDeclaration(format!(
                    "Unavailable native callback input {input}"
                ))),
            },
            ModValueKind::Time { input, encoding } => match inputs.get(input) {
                Some(RuntimeValue::Float(value)) => {
                    if *encoding == GuestStorage::Float32 {
                        Ok(GuestCallValue::Float32(*value as f32))
                    } else {
                        Ok(GuestCallValue::Float64(*value))
                    }
                }
                _ => Err(ProviderError::InvalidDeclaration(format!(
                    "Unavailable native callback input {input}"
                ))),
            },
            ModValueKind::Vector { .. } | ModValueKind::Text { .. } | ModValueKind::UserCommand => {
                Ok(GuestCallValue::Pointer(None))
            }
            ModValueKind::Address(address) => Ok(GuestCallValue::Pointer(Some(self.resolve(address)?))),
            ModValueKind::Userinfo { input } => match inputs.get(input) {
                Some(RuntimeValue::Text(_)) => Ok(GuestCallValue::Pointer(None)),
                _ => Err(ProviderError::InvalidDeclaration(format!(
                    "Unavailable native callback input {input}"
                ))),
            },
        }
    }

    fn number_result(result: &GuestCallResult) -> Result<f64, ProviderError> {
        match result {
            GuestCallResult::Void => Ok(0.0),
            GuestCallResult::Value(GuestCallValue::Int32(value)) => Ok(f64::from(*value)),
            GuestCallResult::Value(GuestCallValue::Uint32(value)) => Ok(f64::from(*value)),
            GuestCallResult::Value(GuestCallValue::Int64(value)) => Ok(*value as f64),
            GuestCallResult::Value(GuestCallValue::Uint64(value)) => Ok(*value as f64),
            GuestCallResult::Value(GuestCallValue::Float32(value)) => Ok(f64::from(*value)),
            GuestCallResult::Value(GuestCallValue::Float64(value)) => Ok(*value),
            _ => Err(ProviderError::BadResult),
        }
    }

    /// Execute a source call with lowered arguments and saved globals.
    pub fn execute(
        &mut self,
        call: &ModCallDecl,
        inputs: &HashMap<String, RuntimeValue>,
    ) -> Result<Option<i32>, ProviderError> {
        self.current()?;
        let address = self.resolve_entry(&call.entry)?;
        let mut values = Vec::with_capacity(call.arguments.len());
        for argument in &call.arguments {
            values.push(self.lower(argument, inputs)?);
        }
        for region in &call.skips {
            let base = self.host.image_base();
            self.active_skips
                .push((base.offset + region.entry, base.offset + region.join));
        }
        let skip_count = call.skips.len();
        let outcome = self.invoke_guarded(address, &values, call, inputs);
        for _ in 0..skip_count {
            self.active_skips.pop();
        }
        let result = outcome?;
        if call.returns == CallReturns::Void {
            return Ok(None);
        }
        Ok(Some(Self::number_result(&result)? as i32))
    }

    fn invoke_guarded(
        &mut self,
        address: GuestAddress,
        values: &[GuestCallValue],
        call: &ModCallDecl,
        inputs: &HashMap<String, RuntimeValue>,
    ) -> Result<GuestCallResult, ProviderError> {
        // Save globals, stage userinfo corrections, then invoke.
        let mut saved = Vec::new();
        for global in &call.globals {
            let address = self.resolve(&global.address)?;
            let previous = self.scalar_read(address, global.encoding)?;
            let value = self.lower(&global.value, inputs)?;
            let scalar = Self::number_result(&GuestCallResult::Value(value))?;
            self.scalar_write(address, scalar, global.encoding)?;
            saved.push((address, global.encoding, previous));
        }
        let staged_userinfo: Vec<(NativeActorId, String)> = self
            .userinfo_staged
            .iter()
            .map(|(actor, info)| (*actor, info.clone()))
            .collect();
        for (actor, info) in &staged_userinfo {
            if let Some(client) = self.services.client_for_actor(*actor) {
                self.services.client_set_userinfo(client, info.clone());
            }
        }
        self.userinfo_staged.clear();
        let result = self.host.invoke_entry(address, values);
        for (address, encoding, previous) in saved {
            self.scalar_write(address, previous, encoding)?;
        }
        // Guest corrections to staged userinfo would commit here; the
        // synthetic host leaves staged strings in place.
        let _ = staged_userinfo;
        Ok(result)
    }

    /// Execute a raw guest entry with an optional authority token.
    pub fn execute_entry(
        &mut self,
        address: GuestAddress,
        values: &[GuestCallValue],
        returns: bool,
        authority: Option<u64>,
    ) -> Result<Option<GuestCallResult>, ProviderError> {
        if authority.is_some_and(|token| !self.authorities.contains_key(&token)) {
            return Err(ProviderError::Retired);
        }
        self.current()?;
        let result = self.host.invoke_entry(address, values);
        if !returns {
            return Ok(None);
        }
        Ok(Some(result))
    }

    /// Claim an authority token for an actor.
    pub fn claim_authority(&mut self, actor: NativeActorId) -> u64 {
        let token = self.next_authority;
        self.next_authority += 1;
        self.authorities.insert(token, actor);
        token
    }

    /// Release an authority token.
    pub fn release_authority(&mut self, token: u64) {
        self.authorities.remove(&token);
    }

    /// Run a closure inside a transfer frame.
    pub fn transfer<R>(&mut self, invoke: impl FnOnce() -> R) -> R {
        self.frames.push(TransferFrame::default());
        let result = invoke();
        self.frames.pop();
        result
    }

    /// Stage a userinfo correction for the next source call.
    pub fn stage_userinfo(&mut self, actor: NativeActorId, info: &str) {
        self.userinfo_staged.insert(actor, info.to_string());
    }

    // -- Clients -----------------------------------------------------------

    fn client_require(&mut self, actor: NativeActorId) -> Result<ClientSlot, ProviderError> {
        let slot = self
            .client_slots
            .get(&actor)
            .cloned()
            .ok_or(ProviderError::IdentityStale)?;
        let client = self
            .services
            .client_for_actor(actor)
            .ok_or(ProviderError::IdentityStale)?;
        if client != slot.client || self.services.client_actor(client) != Some(actor) {
            return Err(ProviderError::IdentityStale);
        }
        Ok(slot)
    }

    /// Source slot for a client actor, reserving a row on first use.
    pub fn client_slot(&mut self, actor: NativeActorId) -> Result<Option<usize>, ProviderError> {
        if let Some(slot) = self.client_slots.get(&actor) {
            let slot = slot.slot;
            self.client_require(actor)?;
            return Ok(Some(slot));
        }
        if self.denied.contains(&actor) {
            return Err(ProviderError::IdentityStale);
        }
        let maximum = self
            .declaration
            .clients
            .as_ref()
            .map(|clients| clients.maximum)
            .unwrap_or(0);
        let Some(client) = self.services.client_for_actor(actor) else {
            return Ok(None);
        };
        let occupied: HashSet<usize> = self.client_slots.values().map(|slot| slot.slot).collect();
        let mut slot = 0;
        while occupied.contains(&slot) {
            slot += 1;
        }
        if slot >= maximum {
            return Err(ProviderError::CapacityExceeded);
        }
        self.client_slots.insert(
            actor,
            ClientSlot {
                client,
                slot,
                admitted: false,
            },
        );
        Ok(Some(slot))
    }

    /// Admit one client actor through admit calls.
    pub fn admit_client(&mut self, actor: NativeActorId) -> Result<bool, ProviderError> {
        if self.denied.contains(&actor) {
            return Ok(false);
        }
        if self.client_slot(actor)?.is_none() {
            return Err(ProviderError::MissingClientRow);
        }
        let slot = self.client_require(actor)?;
        if slot.admitted {
            return Ok(true);
        }
        let admit = self
            .declaration
            .clients
            .clone()
            .map(|clients| clients.admit)
            .unwrap_or_default();
        let mut inputs = HashMap::new();
        inputs.insert("self".to_string(), RuntimeValue::Actor(Some(actor)));
        inputs.insert("time".to_string(), RuntimeValue::Float(self.services.time_secs()));
        for call in &admit {
            let result = self.execute(call, &inputs)?;
            if call.accepts == CallAccepts::NonZero && result.is_none_or(|value| value == 0) {
                let client = self.client_require(actor)?.client;
                self.services.client_drop(client, "Connection refused".to_string());
                self.denied.insert(actor);
                return Ok(false);
            }
        }
        if let Some(slot) = self.client_slots.get_mut(&actor) {
            slot.admitted = true;
        }
        Ok(true)
    }

    /// Whether an actor completed admission.
    #[must_use]
    pub fn client_admitted(&self, actor: NativeActorId) -> bool {
        self.client_slots.get(&actor).is_some_and(|slot| slot.admitted) && !self.denied.contains(&actor)
    }

    /// Run frame calls for a 1-based source row.
    pub fn client_frame(&mut self, slot: usize) -> Result<bool, ProviderError> {
        let found = self
            .client_slots
            .iter()
            .find(|(_, bound)| bound.slot + 1 == slot)
            .map(|(actor, _)| *actor);
        let Some(actor) = found else {
            return Ok(false);
        };
        if self.client_admitted(actor) {
            let frame = self
                .declaration
                .clients
                .clone()
                .map(|clients| clients.frame)
                .unwrap_or_default();
            let mut inputs = HashMap::new();
            inputs.insert("self".to_string(), RuntimeValue::Actor(Some(actor)));
            inputs.insert("time".to_string(), RuntimeValue::Float(self.services.time_secs()));
            for call in &frame {
                self.execute(call, &inputs)?;
            }
        }
        Ok(true)
    }

    /// View-model index for a client actor's source row.
    pub fn viewmodel(&mut self, actor: NativeActorId) -> Result<u32, ProviderError> {
        let slot = self.client_require(actor)?.slot;
        Ok(self.host.weapon_model(slot + 1))
    }

    /// Run a client command through command calls.
    pub fn invoke_command(&mut self, actor: NativeActorId) -> Result<bool, ProviderError> {
        let command = self
            .declaration
            .clients
            .clone()
            .map(|clients| clients.command)
            .unwrap_or_default();
        if command.is_empty() {
            return Ok(false);
        }
        if !self.admit_client(actor)? {
            return Ok(true);
        }
        let mut inputs = HashMap::new();
        inputs.insert("self".to_string(), RuntimeValue::Actor(Some(actor)));
        inputs.insert("time".to_string(), RuntimeValue::Float(self.services.time_secs()));
        for call in &command {
            self.execute(call, &inputs)?;
        }
        Ok(true)
    }

    // -- Input staging -----------------------------------------------------

    /// Stage input fields for one application; returns a session token.
    pub fn open_input(&mut self, application: &InputApplication) -> Result<u64, ProviderError> {
        self.current()?;
        let slot = self
            .projections
            .get(&application.actor)
            .copied()
            .ok_or(ProviderError::MissingClientRow)?;
        let fields = self
            .declaration
            .clients
            .clone()
            .map(|clients| clients.input_fields)
            .unwrap_or_default();
        let mut stores = Vec::new();
        for field in &fields {
            let address = self.record_address(&field.record, slot)?;
            let value = self.lower_input(&field.value, &application.values)?;
            let length = value_length(&field.value, self.host.memory().pointer_bytes());
            self.host.memory().check(address, length, GuestAccess::Write)?;
            let previous = self.host.memory().copy(address, length)?;
            self.host.memory().write(address, &value)?;
            stores.push(InputStore { address, previous });
        }
        let token = self.input_staging.len() as u64 + 1;
        self.input_staging.extend(stores);
        Ok(token)
    }

    fn lower_input(
        &mut self,
        value: &ModValueKind,
        values: &HashMap<String, RuntimeValue>,
    ) -> Result<Vec<u8>, ProviderError> {
        match value {
            ModValueKind::Vector { input } => match values.get(input) {
                Some(RuntimeValue::Vector(vector)) => Ok(vec_to_bytes(*vector).to_vec()),
                _ => Err(ProviderError::InvalidDeclaration(
                    "Native input vector is unavailable".to_string(),
                )),
            },
            ModValueKind::Time { input, encoding } => match values.get(input) {
                Some(RuntimeValue::Float(value)) => {
                    Ok(f64_to_scalar(*value, *encoding, self.host.memory().pointer_bytes()))
                }
                _ => Err(ProviderError::InvalidDeclaration(format!(
                    "Unavailable native callback input {input}"
                ))),
            },
            ModValueKind::Float { input } => match values.get(input) {
                Some(RuntimeValue::Float(value)) => Ok(value.to_le_bytes().to_vec()),
                _ => Err(ProviderError::InvalidDeclaration(format!(
                    "Unavailable native callback input {input}"
                ))),
            },
            _ => Err(ProviderError::InvalidDeclaration(
                "Native input field representation differs".to_string(),
            )),
        }
    }

    /// Roll back staged input fields.
    pub fn close_input(&mut self, failed: bool) {
        if failed {
            for store in std::mem::take(&mut self.input_staging).iter().rev() {
                let _ = self.host.memory().write(store.address, &store.previous);
            }
        } else {
            self.input_staging.clear();
        }
    }

    // -- Objectives and owned actors ---------------------------------------

    /// Read an objective state word.
    pub fn objective_value(&mut self, id: &str) -> Result<Option<f64>, ProviderError> {
        let objective = self
            .declaration
            .objectives
            .iter()
            .find(|objective| objective.id == id)
            .cloned();
        let Some(objective) = objective else { return Ok(None) };
        let Some(state) = objective.state else { return Ok(None) };
        let address = self.resolve(&state)?;
        Ok(Some(self.scalar_read(address, GuestStorage::Float64)?))
    }

    /// Write an objective state word.
    pub fn set_objective_value(&mut self, id: &str, value: f64) -> Result<(), ProviderError> {
        let objective = self
            .declaration
            .objectives
            .iter()
            .find(|objective| objective.id == id)
            .cloned();
        let Some(objective) = objective else { return Ok(()) };
        let Some(state) = objective.state else { return Ok(()) };
        let address = self.resolve(&state)?;
        self.scalar_write(address, value, GuestStorage::Float64)
    }

    /// Fire an objective change call.
    pub fn fire_objective_change(
        &mut self,
        id: &str,
        inputs: &HashMap<String, RuntimeValue>,
    ) -> Result<(), ProviderError> {
        let change = self
            .declaration
            .objectives
            .iter()
            .find(|objective| objective.id == id)
            .and_then(|objective| objective.change.clone());
        if let Some(change) = change {
            self.execute(&change, inputs)?;
        }
        Ok(())
    }

    /// Advance the owned-actor clock, running update for projected slots.
    pub fn advance_owned(&mut self, now: f64) -> Result<(), ProviderError> {
        let owned = self.declaration.source_actors.clone();
        let Some(owned) = owned else { return Ok(()) };
        if self.owned_next == 0.0 {
            self.owned_next = now + owned.frame_seconds;
        }
        while self.owned_next <= now + 1e-9 {
            self.owned_frame += 1;
            for clock in &owned.clock {
                let value = if clock.is_frame {
                    self.owned_frame as f64
                } else if clock.milliseconds {
                    self.owned_next * 1000.0
                } else {
                    self.owned_next
                };
                let address = self.resolve(&clock.address)?;
                self.scalar_write(address, value, clock.encoding)?;
            }
            let update = self.resolve_entry(&owned.update)?;
            for slot in self.projections.values().copied().collect::<Vec<_>>() {
                let address = self.address(self.actor_at(slot).ok_or(ProviderError::LostProjection)?)?;
                self.host
                    .invoke_entry(update, &[GuestCallValue::Pointer(Some(address))]);
            }
            self.owned_next += owned.frame_seconds;
        }
        Ok(())
    }

    // -- Checkpoint, restore, close ----------------------------------------

    /// Capture provider state.
    pub fn checkpoint(&mut self) -> Result<ProviderCheckpoint, ProviderError> {
        if !self.denied.is_empty() {
            return Err(ProviderError::BadCheckpoint(
                "cannot save before rejected clients disconnect".to_string(),
            ));
        }
        let actors = self
            .projections
            .iter()
            .map(|(actor, slot)| SavedProjection {
                actor: SavedActorId::from(*actor),
                slot: *slot,
                appearance: self.appearance.contains(actor),
            })
            .collect();
        let clients = self
            .client_slots
            .iter()
            .map(|(actor, slot)| SavedClientRow {
                actor: SavedActorId::from(*actor),
                slot: slot.slot,
                admitted: slot.admitted,
            })
            .collect();
        Ok(ProviderCheckpoint {
            version: 1,
            map: self.map.clone(),
            api: self.declaration.api,
            actors,
            clients,
            owned: self
                .declaration
                .source_actors
                .is_some()
                .then_some((self.owned_next, self.owned_frame)),
            shared: self.observe()?,
        })
    }

    /// Validate a checkpoint against the declaration.
    pub fn validate_checkpoint(&self, checkpoint: &ProviderCheckpoint) -> Result<(), ProviderError> {
        if checkpoint.version != 1 || checkpoint.map != self.map || checkpoint.api != self.declaration.api {
            return Err(ProviderError::BadCheckpoint("owner differs".to_string()));
        }
        let capacity = self
            .declaration
            .actor_records
            .iter()
            .map(|record| record.capacity)
            .min()
            .unwrap_or(0);
        if (!checkpoint.actors.is_empty() && self.declaration.actor_records.is_empty())
            || checkpoint.actors.iter().any(|entry| entry.slot >= capacity)
            || checkpoint
                .actors
                .iter()
                .map(|entry| entry.slot)
                .collect::<HashSet<_>>()
                .len()
                != checkpoint.actors.len()
        {
            return Err(ProviderError::BadCheckpoint("actor rows differ".to_string()));
        }
        let maximum = self
            .declaration
            .clients
            .as_ref()
            .map(|clients| clients.maximum)
            .unwrap_or(0);
        if checkpoint.clients.len() > maximum
            || checkpoint.clients.iter().any(|entry| entry.slot >= maximum)
            || checkpoint
                .clients
                .iter()
                .map(|entry| entry.slot)
                .collect::<HashSet<_>>()
                .len()
                != checkpoint.clients.len()
        {
            return Err(ProviderError::BadCheckpoint("client rows differ".to_string()));
        }
        if checkpoint.owned.is_some() != self.declaration.source_actors.is_some() {
            return Err(ProviderError::BadCheckpoint("owned actors differ".to_string()));
        }
        Ok(())
    }

    /// Restore provider state from a validated checkpoint.
    pub fn restore(&mut self, checkpoint: &ProviderCheckpoint) -> Result<(), ProviderError> {
        self.validate_checkpoint(checkpoint)?;
        self.restore_inner(checkpoint)
    }

    fn restore_inner(&mut self, checkpoint: &ProviderCheckpoint) -> Result<(), ProviderError> {
        self.projections.clear();
        self.appearance.clear();
        self.client_slots.clear();
        self.denied.clear();
        for entry in &checkpoint.actors {
            let id = self.services.actor_reference(entry.actor);
            let owned = self
                .services
                .actor_resolve(id)
                .ok_or_else(|| ProviderError::BadRestore("saved actor is unavailable".to_string()))?;
            if owned.owner != self.instance || self.services.actor_source_slot(id) != Some(entry.slot) {
                return Err(ProviderError::BadRestore("saved actor differs".to_string()));
            }
            self.projections.insert(id, entry.slot);
            if entry.appearance {
                self.appearance.insert(id);
            }
        }
        for entry in &checkpoint.clients {
            let id = self.services.actor_reference(entry.actor);
            let client = self
                .services
                .client_for_actor(id)
                .ok_or_else(|| ProviderError::BadRestore("saved client is unavailable".to_string()))?;
            self.client_slots.insert(
                id,
                ClientSlot {
                    client,
                    slot: entry.slot,
                    admitted: entry.admitted,
                },
            );
        }
        if let Some((next, frame)) = checkpoint.owned {
            self.owned_next = next;
            self.owned_frame = frame;
        }
        for ((slot, record_id, offset), value) in &checkpoint.shared {
            let encoding = {
                let record = self
                    .records
                    .get(record_id)
                    .ok_or_else(|| ProviderError::BadRestore("saved field record differs".to_string()))?;
                record
                    .fields
                    .iter()
                    .find(|field| field.offset == *offset)
                    .ok_or_else(|| ProviderError::BadRestore("saved field differs".to_string()))?
                    .encoding
            };
            let address = self.record_address(record_id, *slot)?;
            self.scalar_write(address, *value, encoding)?;
        }
        self.refresh()
    }

    /// Begin lifecycle: run initialize calls once.
    pub fn begin_lifecycle(&mut self) -> Result<(), ProviderError> {
        if self.lifecycle {
            return Ok(());
        }
        self.lifecycle = true;
        let mut inputs = HashMap::new();
        inputs.insert("time".to_string(), RuntimeValue::Float(self.services.time_secs()));
        for call in self.declaration.initialize.clone() {
            self.execute(&call, &inputs)?;
        }
        self.ready = true;
        Ok(())
    }

    /// Whether the provider is ready.
    #[must_use]
    pub fn is_ready(&self) -> bool {
        self.ready
    }

    /// Close the provider, releasing projections.
    pub fn close(&mut self) -> Result<(), ProviderError> {
        if self.closed {
            return Ok(());
        }
        self.closing = true;
        for actor in self.projections.keys().copied().collect::<Vec<_>>() {
            self.pending_releases.insert(actor);
        }
        let released = self.release_pending();
        self.client_slots.clear();
        self.denied.clear();
        self.authorities.clear();
        self.active_skips.clear();
        self.frames.clear();
        self.closed = true;
        self.closing = false;
        released
    }
}

// ---------------------------------------------------------------------------
// Synthetic host and services
// ---------------------------------------------------------------------------

/// Scripted guest entry handler.
pub type EntryHandler = dyn Fn(&mut SyntheticProviderHost, &[GuestCallValue]) -> GuestCallResult;

/// Headless provider host: memory, exports, entities, and scripted entries.
pub struct SyntheticProviderHost {
    /// Guest memory.
    pub memory: SparseGuestMemory,
    image_base: GuestAddress,
    exports: HashMap<String, GuestAddress>,
    entities: EntityTable,
    handlers: HashMap<u64, std::rc::Rc<EntryHandler>>,
    /// Invoked entry offsets.
    pub invoked: Vec<u64>,
}

impl SyntheticProviderHost {
    /// Build a host over mapped memory.
    pub fn new(memory: SparseGuestMemory, image_base: GuestAddress, entities: EntityTable) -> Self {
        Self {
            memory,
            image_base,
            exports: HashMap::new(),
            entities,
            handlers: HashMap::new(),
            invoked: Vec::new(),
        }
    }

    /// Register a named export.
    pub fn register_export(&mut self, name: &str, address: GuestAddress) {
        self.exports.insert(name.to_string(), address);
    }

    /// Register a scripted entry handler.
    pub fn register_handler(
        &mut self,
        offset: u64,
        handler: impl Fn(&mut SyntheticProviderHost, &[GuestCallValue]) -> GuestCallResult + 'static,
    ) {
        self.handlers.insert(offset, std::rc::Rc::new(handler));
    }
}

impl ProviderHost for SyntheticProviderHost {
    fn memory(&mut self) -> &mut SparseGuestMemory {
        &mut self.memory
    }

    fn image_base(&self) -> GuestAddress {
        self.image_base
    }

    fn entry(&self, name: &str) -> Option<GuestAddress> {
        self.exports.get(name).copied()
    }

    fn entities(&self) -> EntityTable {
        self.entities
    }

    fn weapon_model(&self, slot: usize) -> u32 {
        slot as u32
    }

    fn invoke_entry(&mut self, address: GuestAddress, values: &[GuestCallValue]) -> GuestCallResult {
        self.invoked.push(address.offset);
        if let Some(handler) = self.handlers.get(&address.offset).cloned() {
            return handler(self, values);
        }
        GuestCallResult::Void
    }
}

/// Headless provider services: actors, clients, and inventory maps.
pub struct SyntheticProviderServices {
    time: f64,
    actors: HashMap<NativeActorId, OwnedActor>,
    slots: HashMap<NativeActorId, usize>,
    released: Vec<NativeActorId>,
    clients: HashMap<NativeActorId, NativeClientId>,
    userinfos: HashMap<NativeClientId, String>,
    drops: Vec<(NativeClientId, String)>,
    inventory: HashMap<(NativeActorId, String), f64>,
}

impl SyntheticProviderServices {
    /// Build empty services.
    #[must_use]
    pub fn new(time: f64) -> Self {
        Self {
            time,
            actors: HashMap::new(),
            slots: HashMap::new(),
            released: Vec::new(),
            clients: HashMap::new(),
            userinfos: HashMap::new(),
            drops: Vec::new(),
            inventory: HashMap::new(),
        }
    }

    /// Connect a client identity.
    pub fn connect(&mut self, actor: NativeActorId, client: NativeClientId, userinfo: &str) {
        self.clients.insert(actor, client);
        self.userinfos.insert(client, userinfo.to_string());
    }

    /// Dropped clients.
    #[must_use]
    pub fn drops(&self) -> &[(NativeClientId, String)] {
        &self.drops
    }

    /// Overwrite shared time.
    pub fn set_time(&mut self, time: f64) {
        self.time = time;
    }
}

impl ProviderServices for SyntheticProviderServices {
    fn time_secs(&self) -> f64 {
        self.time
    }

    fn allocate_actor(&mut self, slot: usize, label: &str) -> OwnedActor {
        let _ = label;
        let id = NativeActorId {
            slot: slot as u32,
            generation: 1,
        };
        let owned = OwnedActor {
            id,
            owner: "test:mod".to_string(),
        };
        self.actors.insert(id, owned.clone());
        self.slots.insert(id, slot);
        owned
    }

    fn actor_is_live(&self, actor: NativeActorId) -> bool {
        self.actors.contains_key(&actor)
    }

    fn actor_release(&mut self, actor: OwnedActor) {
        self.actors.remove(&actor.id);
    }

    fn actor_reference(&mut self, saved: SavedActorId) -> NativeActorId {
        NativeActorId {
            slot: saved.slot,
            generation: saved.generation,
        }
    }

    fn actor_resolve(&self, actor: NativeActorId) -> Option<OwnedActor> {
        self.actors.get(&actor).cloned()
    }

    fn actor_source_slot(&self, actor: NativeActorId) -> Option<usize> {
        self.slots.get(&actor).copied()
    }

    fn take_released_actors(&mut self) -> Vec<NativeActorId> {
        std::mem::take(&mut self.released)
    }

    fn client_for_actor(&self, actor: NativeActorId) -> Option<NativeClientId> {
        self.clients.get(&actor).copied()
    }

    fn client_actor(&self, client: NativeClientId) -> Option<NativeActorId> {
        self.clients
            .iter()
            .find(|(_, bound)| **bound == client)
            .map(|(actor, _)| *actor)
    }

    fn client_list(&self) -> Vec<(NativeActorId, NativeClientId)> {
        self.clients.iter().map(|(actor, client)| (*actor, *client)).collect()
    }

    fn client_userinfo(&self, client: NativeClientId) -> String {
        self.userinfos.get(&client).cloned().unwrap_or_default()
    }

    fn client_set_userinfo(&mut self, client: NativeClientId, value: String) {
        self.userinfos.insert(client, value);
    }

    fn client_drop(&mut self, client: NativeClientId, reason: String) {
        self.drops.push((client, reason));
        self.clients.retain(|_, bound| *bound != client);
    }

    fn inventory_count(&self, actor: NativeActorId, item: &str) -> f64 {
        self.inventory.get(&(actor, item.to_string())).copied().unwrap_or(0.0)
    }

    fn inventory_set(&mut self, actor: NativeActorId, item: &str, count: f64) {
        self.inventory.insert((actor, item.to_string()), count);
    }

    fn inventory_owns(&self, actor: NativeActorId, item: &str) -> bool {
        self.inventory.contains_key(&(actor, item.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_core::identity::ProviderId;
    use qa_guest::core::contracts::{ContentDigest, GuestMapOptions, GuestPermissions, ModuleIdentity};

    fn actor(slot: u32) -> NativeActorId {
        NativeActorId { slot, generation: 1 }
    }

    fn client(slot: u32) -> NativeClientId {
        NativeClientId { slot, generation: 1 }
    }

    fn call(id: &str) -> ModCallDecl {
        ModCallDecl {
            id: id.to_string(),
            entry: ModEntryRef::Rva(0x1000),
            accepts: CallAccepts::Always,
            returns: CallReturns::Void,
            skips: Vec::new(),
            arguments: Vec::new(),
            globals: Vec::new(),
        }
    }

    fn declaration() -> NativeModDeclaration {
        NativeModDeclaration {
            api: ApiKind::Classic,
            pointer_bytes: 4,
            actor_records: vec![
                ModActorRecord {
                    id: "edicts".to_string(),
                    stride: 256,
                    capacity: 8,
                    first_slot: 1,
                    base: ModRecordBase::Entities,
                    fields: vec![
                        ModRecordField {
                            offset: 0,
                            encoding: GuestStorage::Int32,
                            binding: ModFieldBinding::Inventory {
                                item: "q2:shells".to_string(),
                            },
                        },
                        ModRecordField {
                            offset: 8,
                            encoding: GuestStorage::Int32,
                            binding: ModFieldBinding::Constant(7.0),
                        },
                    ],
                },
                ModActorRecord {
                    id: "clients".to_string(),
                    stride: 64,
                    capacity: 2,
                    first_slot: 0,
                    base: ModRecordBase::Clients,
                    fields: vec![ModRecordField {
                        offset: 0,
                        encoding: GuestStorage::Int32,
                        binding: ModFieldBinding::Private,
                    }],
                },
            ],
            entity_record: Some("edicts".to_string()),
            clients: Some(ModClientsDecl {
                maximum: 2,
                records: vec!["clients".to_string()],
                admit: vec![call("admit")],
                userinfo: Vec::new(),
                disconnect: Vec::new(),
                command: vec![call("command")],
                frame: Vec::new(),
                end_frame: Vec::new(),
                input: Vec::new(),
                input_fields: Vec::new(),
                outputs: Vec::new(),
                pose: None,
            }),
            callbacks: Vec::new(),
            protection: Vec::new(),
            items: None,
            pickups: Vec::new(),
            objectives: vec![ModObjectiveDecl {
                id: "fraglimit".to_string(),
                state: Some(ModAddress {
                    rva: 0x500,
                    indirections: Vec::new(),
                }),
                carrier: None,
                target: None,
                owned: false,
                change: None,
            }],
            source_actors: None,
            initialize: vec![call("init")],
            project: vec![call("project")],
            release: vec![call("release")],
            client_presentation: false,
        }
    }

    fn fixture() -> NativeModProvider<SyntheticProviderHost, SyntheticProviderServices> {
        let module = ModuleIdentity::new(
            ProviderId::new("test", "provider"),
            "provider.so",
            ContentDigest {
                algorithm: "none".to_string(),
                value: "0".to_string(),
            },
            "r1",
        );
        let mut memory = SparseGuestMemory::new(module, 4, 0x100000).unwrap();
        let image_base = memory
            .map(&GuestMapOptions::new(0x0, 0x4000, GuestPermissions::ReadWriteExecute))
            .unwrap();
        let table_base = memory
            .map(&GuestMapOptions::new(0x20000, 256 * 16, GuestPermissions::ReadWrite))
            .unwrap();
        let entities = EntityTable {
            base: table_base,
            stride: 256,
            count: 16,
            capacity: 16,
        };
        let mut host = SyntheticProviderHost::new(memory, image_base, entities);
        host.register_handler(0x1000, |_, _| GuestCallResult::Void);
        let mut services = SyntheticProviderServices::new(1.0);
        services.connect(actor(1), client(1), "\\name\\a\\");
        services.inventory.insert((actor(1), "q2:shells".to_string()), 0.0);
        NativeModProvider::new(declaration(), host, services, "test:mod", "q2dm1").unwrap()
    }

    #[test]
    fn validation_rejects_overlaps_and_duplicates() {
        let mut bad = declaration();
        bad.actor_records[0].fields.push(ModRecordField {
            offset: 2,
            encoding: GuestStorage::Int32,
            binding: ModFieldBinding::Private,
        });
        assert!(matches!(
            validate_native_mod_declaration(&bad),
            Err(ProviderError::InvalidDeclaration(_))
        ));
        let mut channels = declaration();
        channels.protection = vec![
            ModProtectionDecl {
                id: "a".to_string(),
                channel: ProtectionChannel::Regular,
                absorb: ModAbsorb::Native {
                    entry: ModEntryRef::Rva(0x100),
                    sparks: None,
                },
                flags_abi: ApiKind::Classic,
            },
            ModProtectionDecl {
                id: "b".to_string(),
                channel: ProtectionChannel::Regular,
                absorb: ModAbsorb::Native {
                    entry: ModEntryRef::Rva(0x200),
                    sparks: None,
                },
                flags_abi: ApiKind::Classic,
            },
        ];
        assert!(matches!(
            validate_native_mod_declaration(&channels),
            Err(ProviderError::InvalidDeclaration(_))
        ));
        validate_native_mod_declaration(&declaration()).unwrap();
    }

    #[test]
    fn projection_seeds_refreshes_and_flushes() {
        let mut provider = fixture();
        provider.begin_lifecycle().unwrap();
        assert!(provider.is_ready());
        let owned = provider.project(1).unwrap();
        assert_eq!(owned.owner, "test:mod");
        assert_eq!(provider.slot_of(owned.id), Some(1));
        // Seeded constant is present.
        let address = provider.record_address("edicts", 1).unwrap();
        let field = provider.host.memory().offset(address, 8).unwrap();
        let constant = provider.scalar_read(field, GuestStorage::Int32).unwrap();
        assert_eq!(constant, 7.0);
        // Guest writes publish through refresh + flush.
        provider.scalar_write(address, 30.0, GuestStorage::Int32).unwrap();
        provider.refresh().unwrap();
        // Flush commits the observed delta.
        provider.frames.push(TransferFrame::default());
        let mut committed = Vec::new();
        provider
            .flush(&mut |actor, item, before, after| {
                committed.push((actor, item.to_string(), before, after));
            })
            .unwrap();
        assert_eq!(committed.len(), 1);
        assert_eq!(committed[0].2, 30.0);
        assert_eq!(committed[0].3, 30.0);
        provider.close().unwrap();
    }

    #[test]
    fn source_calls_save_globals_and_stage_userinfo() {
        let mut provider = fixture();
        let global = ModAddress {
            rva: 0x600,
            indirections: Vec::new(),
        };
        let address = provider.resolve(&global).unwrap();
        provider.scalar_write(address, 11.0, GuestStorage::Int32).unwrap();
        let seen = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let probe = seen.clone();
        provider.host.register_handler(0x1000, move |host, values| {
            probe.borrow_mut().push(values.len());
            let current = host.memory.copy(address, 4).unwrap();
            assert_eq!(current, 42i32.to_le_bytes());
            GuestCallResult::Value(GuestCallValue::Int32(3))
        });
        let call = ModCallDecl {
            id: "with-global".to_string(),
            entry: ModEntryRef::Rva(0x1000),
            accepts: CallAccepts::Always,
            returns: CallReturns::Int32,
            skips: vec![ModSkip {
                entry: 0x10,
                join: 0x20,
            }],
            arguments: vec![ModValueKind::Float {
                input: "time".to_string(),
            }],
            globals: vec![ModGlobal {
                address: global,
                encoding: GuestStorage::Int32,
                value: ModValueKind::Float {
                    input: "amount".to_string(),
                },
            }],
        };
        let mut inputs = HashMap::new();
        inputs.insert("time".to_string(), RuntimeValue::Float(1.0));
        inputs.insert("amount".to_string(), RuntimeValue::Float(42.0));
        provider.stage_userinfo(actor(1), "\\name\\b\\");
        let result = provider.execute(&call, &inputs).unwrap();
        assert_eq!(result, Some(3));
        assert_eq!(*seen.borrow(), vec![1]);
        assert!(provider.active_skips.is_empty());
        let restored = provider.scalar_read(address, GuestStorage::Int32).unwrap();
        assert_eq!(restored, 11.0);
        assert_eq!(provider.services.client_userinfo(client(1)), "\\name\\b\\".to_string());
    }

    #[test]
    fn checkpoint_restore_and_client_admission() {
        let mut provider = fixture();
        assert!(provider.admit_client(actor(1)).unwrap());
        assert!(provider.client_admitted(actor(1)));
        assert!(provider.invoke_command(actor(1)).unwrap());
        let owned = provider.project(1).unwrap();
        assert_eq!(owned.id, actor(1));
        let checkpoint = provider.checkpoint().unwrap();
        assert_eq!(checkpoint.actors.len(), 1);
        assert_eq!(checkpoint.clients.len(), 1);
        provider.validate_checkpoint(&checkpoint).unwrap();
        // Mutate, then restore.
        let target = provider.address(actor(1)).unwrap();
        provider.scalar_write(target, 99.0, GuestStorage::Int32).unwrap();
        provider.restore(&checkpoint).unwrap();
        let target = provider.address(actor(1)).unwrap();
        let restored = provider.scalar_read(target, GuestStorage::Int32).unwrap();
        assert_eq!(restored, 0.0);
        // Objectives roundtrip.
        assert_eq!(provider.objective_value("fraglimit").unwrap(), Some(0.0));
        provider.set_objective_value("fraglimit", 20.0).unwrap();
        assert_eq!(provider.objective_value("fraglimit").unwrap(), Some(20.0));
        provider.close().unwrap();
        assert!(matches!(provider.project(2), Err(ProviderError::Closed)));
    }
}
