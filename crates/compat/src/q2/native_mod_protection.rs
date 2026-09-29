//! Port of `src/compat/q2/native-mod-protection.ts`.
//! Bridges component armor: original storage and absorption code serve while
//! reservations, observation, and inventory fuel compose on the boundary.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use qa_core::math::Vec3;
use qa_guest::core::contracts::{GuestAccess, GuestAddress, GuestStorage};
use qa_guest::core::memory::SparseGuestMemory;
use qa_guest::error::GuestError;
use qa_world::combat::{ArmorState, PoweredProtection, RegularArmor};
use thiserror::Error;

/// Failures in the native protection bridge.
#[derive(Debug, Clone, PartialEq, Error)]
pub enum ProtectionError {
    /// The bridge is not current.
    #[error("native protection bridge is not current")]
    NotCurrent,
    /// A live canonical client is required.
    #[error("native protection requires a live canonical client")]
    NoLiveClient,
    /// A client identity is no longer live.
    #[error("native protection client is no longer live")]
    IdentityStale,
    /// A client has not been admitted.
    #[error("native protection client has not been admitted")]
    NotAdmitted,
    /// No source client record exists.
    #[error("native protection has no source client record")]
    NoClientRecord,
    /// The source record is not active.
    #[error("native protection requires an active source record")]
    InactiveRecord,
    /// Power fuel lacks a canonical inventory field.
    #[error("native power fuel requires one declared canonical inventory field")]
    PowerFuelUnlinked,
    /// A protection declaration is empty or mis-channeled.
    #[error("native protection declaration is invalid")]
    BadDeclaration,
    /// A named entry is not declared.
    #[error("native protection entry is not declared: {0}")]
    MissingEntry(String),
    /// An armor stage target differs from its binding.
    #[error("native armor stage target differs from its binding")]
    TargetMismatch,
    /// A protection scale needs an explicit source input.
    #[error("native regular protection scale requires an explicit source input")]
    ScaleInputMissing,
    /// Power activation needs its original source operation.
    #[error("native power activation requires its original source operation")]
    PowerActivation,
    /// Selected armor has no source storage.
    #[error("selected native armor has no source storage")]
    MissingStorage,
    /// The declaration cannot represent an armor state.
    #[error("armor state differs from its declared native representation")]
    Unrepresentable,
    /// Restored counts differ from canonical inventory.
    #[error("restored native protection count differs from its canonical inventory")]
    RestoreMismatch,
    /// A canonical baseline was lost.
    #[error("native protection count lost its canonical baseline")]
    MissingBaseline,
    /// The donor owner retired.
    #[error("native protection donor owner retired")]
    Retired,
    /// Underlying guest failure.
    #[error("native protection guest failure: {0}")]
    Guest(String),
}

impl From<GuestError> for ProtectionError {
    fn from(error: GuestError) -> Self {
        Self::Guest(error.to_string())
    }
}

/// Generational actor handle local to the protection bridge.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct NativeActorId {
    /// Registry slot.
    pub slot: u32,
    /// Slot generation.
    pub generation: u32,
}

/// Generational client handle local to the protection bridge.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct NativeClientId {
    /// Client slot.
    pub slot: u32,
    /// Slot generation.
    pub generation: u32,
}

/// Protection channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ProtectionChannel {
    /// Regular armor.
    Regular,
    /// Powered protection.
    Powered,
}

/// Powered protection kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PowerKind {
    /// Screen.
    Screen,
    /// Shield.
    Shield,
}

/// One scalar protection field inside a record.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ProtectionField {
    /// Owning record.
    pub record: String,
    /// Byte offset.
    pub offset: usize,
    /// Lane storage.
    pub storage: GuestStorage,
}

impl ProtectionField {
    fn width(&self, pointer_bytes: usize) -> usize {
        self.storage.byte_length(pointer_bytes)
    }
}

/// Selection rule for one armor item.
#[derive(Debug, Clone, PartialEq)]
pub enum ArmorSelection {
    /// Selected while positive.
    Positive {
        /// Selection field.
        field: ProtectionField,
    },
    /// Selected at an exact value.
    Enum {
        /// Selection field.
        field: ProtectionField,
        /// Active value.
        value: f64,
        /// Inactive value.
        none: f64,
    },
}

impl ArmorSelection {
    fn field(&self) -> &ProtectionField {
        match self {
            Self::Positive { field } | Self::Enum { field, .. } => field,
        }
    }

    fn is_selected(&self, value: Option<f64>) -> bool {
        let Some(value) = value else { return false };
        match self {
            Self::Positive { .. } => value > 0.0,
            Self::Enum { value: active, .. } => value == *active,
        }
    }
}

/// Regular armor storage declaration.
#[derive(Debug, Clone, PartialEq)]
pub struct RegularStorage {
    /// Canonical item.
    pub item: String,
    /// Selection rule.
    pub selection: ArmorSelection,
    /// Points field.
    pub points: ProtectionField,
}

/// Power armor storage declaration.
#[derive(Debug, Clone, PartialEq)]
pub struct PowerStorage {
    /// Protection kind.
    pub kind: PowerKind,
    /// Selection rule.
    pub selection: ArmorSelection,
    /// Cells field.
    pub cells: ProtectionField,
    /// Optional enabled bitmask (field, mask).
    pub enabled: Option<(ProtectionField, i64)>,
}

/// Declared source call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProtectionCall {
    /// Call id.
    pub id: String,
}

/// Address reference: export name or image RVA.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AddressRef {
    /// Named export.
    Export(String),
    /// Image-relative offset.
    Rva(u64),
}

/// Absorption declaration.
#[derive(Debug, Clone, PartialEq)]
pub enum AbsorbDecl {
    /// Plain source call.
    SourceCall {
        /// Source call.
        call: ProtectionCall,
    },
    /// Scoped donor region.
    SourceRegion {
        /// Source call.
        call: ProtectionCall,
        /// Frame entry RVA.
        frame_entry: u64,
        /// Region entry RVA.
        entry_rva: u64,
        /// Region join RVA.
        join_rva: u64,
        /// Frame exit RVA.
        frame_exit: u64,
    },
    /// Direct native entry with lowered flags.
    Direct {
        /// Native entry.
        entry: AddressRef,
        /// Spark count for check-armor ABIs, if any.
        sparks: Option<f64>,
    },
}

/// Protection definition for one channel.
#[derive(Debug, Clone, PartialEq)]
pub struct ProtectionDefinition {
    /// Definition id.
    pub id: String,
    /// Protection channel.
    pub channel: ProtectionChannel,
    /// Absorption declaration.
    pub absorb: AbsorbDecl,
    /// Regular storage (regular channel only).
    pub regular: Vec<RegularStorage>,
    /// Power storage (powered channel only).
    pub power: Vec<PowerStorage>,
    /// Whether the absorb call declares the scale input.
    pub declares_scale_input: bool,
}

/// Damage delivery mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Delivery {
    /// Direct hit.
    Direct,
    /// Radius damage.
    Radius,
}

/// Armor stage flags.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StageFlags {
    /// Bypass all armor.
    pub no_armor: bool,
    /// Energy damage.
    pub energy: bool,
    /// Bypass regular armor.
    pub no_regular: bool,
    /// Bypass power armor.
    pub no_power: bool,
}

/// Armor stage input.
#[derive(Debug, Clone, PartialEq)]
pub struct ArmorStageInput {
    /// Damage target.
    pub target: NativeActorId,
    /// Incoming amount.
    pub amount: f64,
    /// Delivery mode.
    pub delivery: Delivery,
    /// Stage flags.
    pub flags: StageFlags,
    /// Impact point.
    pub point: Vec3,
    /// Surface normal.
    pub normal: Vec3,
    /// Shot direction.
    pub direction: Vec3,
    /// Knockback impulse.
    pub knockback: f64,
    /// Regular protection scale.
    pub regular_protection_scale: f64,
}

/// Armor stage result.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ArmorStageResult {
    /// Damage saved by armor.
    pub saved: f64,
}

/// Stored armor published during absorption.
pub trait ProtectionObserver {
    /// Publish committed armor stores.
    fn stored(
        &mut self,
        regular: Option<(RegularArmor, RegularArmor)>,
        powered: Option<(PoweredProtection, PoweredProtection)>,
    );
}

/// Canonical inventory change flushed during absorption.
#[derive(Debug, Clone, PartialEq)]
pub struct InventoryChange {
    /// Changed actor.
    pub actor: NativeActorId,
    /// Changed item.
    pub item: String,
    /// Previous count, if baselined.
    pub before: Option<f64>,
    /// New count.
    pub after: f64,
}

/// Runtime input value.
#[derive(Debug, Clone, PartialEq)]
pub enum RuntimeValue {
    /// Actor handle.
    Actor(Option<NativeActorId>),
    /// Scalar.
    Float(f64),
    /// Vector.
    Vector(Vec3),
}

/// Source call inputs.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ProtectionInputs {
    values: HashMap<String, RuntimeValue>,
}

impl ProtectionInputs {
    /// Read one input.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<&RuntimeValue> {
        self.values.get(name)
    }
}

/// Donor region reference for scoped absorption.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RegionRef {
    /// Frame entry RVA.
    pub frame_entry: u64,
    /// Region entry RVA.
    pub entry_rva: u64,
    /// Region join RVA.
    pub join_rva: u64,
    /// Frame exit RVA.
    pub frame_exit: u64,
}

/// Protection operations: clients, scalars, transfer, and source calls.
pub trait ProtectionOperations {
    /// Assert the bridge is current.
    fn assert_current(&self) -> Result<(), ProtectionError>;
    /// Whether an actor is live.
    fn is_live(&self, actor: NativeActorId) -> bool;
    /// Whether an actor is an admitted client.
    fn is_eligible(&self, actor: NativeActorId) -> bool;
    /// Source slot for an actor.
    fn slot(&self, actor: NativeActorId) -> Option<usize>;
    /// Client handle for an actor.
    fn client_of(&self, actor: NativeActorId) -> Option<NativeClientId>;
    /// Actor for a client handle.
    fn actor_of_client(&self, client: NativeClientId) -> Option<NativeActorId>;
    /// Connected client actors.
    fn client_actors(&self) -> Vec<NativeActorId>;
    /// Canonical inventory count.
    fn inventory_count(&self, actor: NativeActorId, item: &str) -> f64;
    /// Whether an actor owns a canonical inventory item.
    fn owns_inventory(&self, actor: NativeActorId, item: &str) -> bool;
    /// Read (or, with a value, write) a scalar.
    fn scalar(
        &mut self,
        base: GuestAddress,
        field: &ProtectionField,
        value: Option<f64>,
    ) -> Result<f64, ProtectionError>;
    /// Run a closure inside a transfer frame.
    fn transfer<R>(&mut self, invoke: impl FnOnce(&mut Self) -> R) -> R;
    /// Flush canonical inventory changes.
    fn flush(&mut self) -> Vec<InventoryChange>;
    /// Invoke a source call; `None` means retired.
    fn invoke(
        &mut self,
        call: &ProtectionCall,
        inputs: &ProtectionInputs,
        region: Option<RegionRef>,
        authority: Option<u64>,
    ) -> Option<i32>;
    /// Guest memory backing the records.
    fn memory(&mut self) -> &mut SparseGuestMemory;
    /// Image base address.
    fn image_base(&self) -> GuestAddress;
    /// Named export address, if declared.
    fn entry(&self, name: &str) -> Option<GuestAddress>;
    /// Record base for a slot, if projected.
    fn record_base(&mut self, slot: usize, record: &str) -> Option<GuestAddress>;
    /// Whether a source record is active.
    fn record_active(&self, slot: usize) -> bool;
    /// Current shared time in seconds.
    fn time_secs(&self) -> f64;
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ProtectionEntry {
    client: NativeClientId,
    channels: Vec<ProtectionChannel>,
    bound: bool,
}

#[derive(Debug, Clone)]
struct Counter {
    channel: ProtectionChannel,
    selection: String,
    field: ProtectionField,
    inventory: Option<String>,
}

#[derive(Debug, Clone)]
struct Stage {
    actor: NativeActorId,
    armor: ArmorState,
    suppressed: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct AuthoritySnapshot {
    actor: NativeActorId,
    slot: usize,
}

/// Native protection bridge over synthetic operations.
pub struct NativeModProtection<O: ProtectionOperations> {
    definitions: Vec<ProtectionDefinition>,
    instance: String,
    classic_api: bool,
    operations: O,
    entries: HashMap<NativeActorId, ProtectionEntry>,
    counters: Vec<Counter>,
    stages: Vec<Stage>,
    authorities: HashMap<u64, AuthoritySnapshot>,
    next_authority: u64,
    attached: bool,
    active: bool,
}

impl<O: ProtectionOperations> NativeModProtection<O> {
    /// Build the bridge, linking counters to canonical inventory fields.
    pub fn new(
        definitions: Vec<ProtectionDefinition>,
        instance: &str,
        classic_api: bool,
        inventory_fields: &[(ProtectionField, String)],
        operations: O,
    ) -> Result<Self, ProtectionError> {
        let mut channels = HashSet::new();
        for definition in &definitions {
            if !channels.insert(definition.channel) || definition.id.is_empty() {
                return Err(ProtectionError::BadDeclaration);
            }
            match definition.channel {
                ProtectionChannel::Regular => {
                    if definition.regular.is_empty() || !definition.power.is_empty() {
                        return Err(ProtectionError::BadDeclaration);
                    }
                }
                ProtectionChannel::Powered => {
                    if definition.power.is_empty() || !definition.regular.is_empty() {
                        return Err(ProtectionError::BadDeclaration);
                    }
                }
            }
        }
        let mut counters = Vec::new();
        for definition in &definitions {
            if definition.channel == ProtectionChannel::Regular {
                for item in &definition.regular {
                    counters.push(Counter {
                        channel: ProtectionChannel::Regular,
                        selection: item.item.clone(),
                        field: item.points.clone(),
                        inventory: inventory_fields
                            .iter()
                            .find(|(field, _)| {
                                field.record == item.points.record
                                    && field.offset == item.points.offset
                                    && field.storage == item.points.storage
                            })
                            .map(|(_, item)| item.clone()),
                    });
                }
            } else {
                for item in &definition.power {
                    let inventory = inventory_fields
                        .iter()
                        .find(|(field, _)| {
                            field.record == item.cells.record
                                && field.offset == item.cells.offset
                                && field.storage == item.cells.storage
                        })
                        .map(|(_, item)| item.clone());
                    if inventory.is_none() {
                        return Err(ProtectionError::PowerFuelUnlinked);
                    }
                    counters.push(Counter {
                        channel: ProtectionChannel::Powered,
                        selection: match item.kind {
                            PowerKind::Screen => "screen".to_string(),
                            PowerKind::Shield => "shield".to_string(),
                        },
                        field: item.cells.clone(),
                        inventory,
                    });
                }
            }
        }
        Ok(Self {
            definitions,
            instance: instance.to_string(),
            classic_api,
            operations,
            entries: HashMap::new(),
            counters,
            stages: Vec::new(),
            authorities: HashMap::new(),
            next_authority: 1,
            attached: false,
            active: false,
        })
    }

    /// Owning provider name.
    #[must_use]
    pub fn instance(&self) -> &str {
        &self.instance
    }

    /// Borrow the operations (for fixtures).
    #[must_use]
    pub fn operations(&self) -> &O {
        &self.operations
    }

    /// Mutably borrow the operations (for fixtures).
    pub fn operations_mut(&mut self) -> &mut O {
        &mut self.operations
    }

    fn inventory_items(&self, channel: Option<ProtectionChannel>) -> Vec<String> {
        let mut items = Vec::new();
        for counter in &self.counters {
            if let Some(inventory) = &counter.inventory {
                if channel.is_none_or(|channel| counter.channel == channel) && !items.contains(inventory) {
                    items.push(inventory.clone());
                }
            }
        }
        items
    }

    /// Attach to the host, checking absorb entries for execute permission.
    pub fn attach(&mut self) -> Result<(), ProtectionError> {
        for definition in self.definitions.clone() {
            match &definition.absorb {
                AbsorbDecl::SourceCall { .. } | AbsorbDecl::SourceRegion { .. } => {}
                AbsorbDecl::Direct { entry, .. } => {
                    let target = match entry {
                        AddressRef::Export(name) => self
                            .operations
                            .entry(name)
                            .ok_or_else(|| ProtectionError::MissingEntry(name.clone()))?,
                        AddressRef::Rva(rva) => {
                            let base = self.operations.image_base();
                            self.operations
                                .memory()
                                .offset(base, i64::try_from(*rva).unwrap_or(i64::MAX))?
                        }
                    };
                    self.operations.memory().check(target, 1, GuestAccess::Execute)?;
                }
            }
            if let AbsorbDecl::SourceRegion {
                frame_entry,
                entry_rva,
                join_rva,
                frame_exit,
                ..
            } = &definition.absorb
            {
                for rva in [frame_entry, entry_rva, join_rva, frame_exit] {
                    let base = self.operations.image_base();
                    let address = self
                        .operations
                        .memory()
                        .offset(base, i64::try_from(*rva).unwrap_or(i64::MAX))?;
                    self.operations.memory().check(address, 1, GuestAccess::Execute)?;
                }
            }
        }
        self.attached = true;
        Ok(())
    }

    /// Reserve protection for every connected client.
    pub fn reserve(&mut self) -> Result<(), ProtectionError> {
        for actor in self.operations.client_actors() {
            self.reserve_actor(actor)?;
        }
        Ok(())
    }

    /// Reserve protection for one actor.
    pub fn reserve_actor(&mut self, actor: NativeActorId) -> Result<(), ProtectionError> {
        self.operations.assert_current()?;
        if self.entries.contains_key(&actor) {
            self.require(actor)?;
            return Ok(());
        }
        let client = self.operations.client_of(actor).ok_or(ProtectionError::NoLiveClient)?;
        if !self.operations.is_live(actor) || self.operations.actor_of_client(client) != Some(actor) {
            return Err(ProtectionError::NoLiveClient);
        }
        for item in self.inventory_items(None) {
            if !self.operations.owns_inventory(actor, &item) {
                return Err(ProtectionError::NoLiveClient);
            }
        }
        let channels: Vec<ProtectionChannel> = self.definitions.iter().map(|definition| definition.channel).collect();
        self.entries.insert(
            actor,
            ProtectionEntry {
                client,
                channels,
                bound: false,
            },
        );
        Ok(())
    }

    fn require(&self, actor: NativeActorId) -> Result<&ProtectionEntry, ProtectionError> {
        let entry = self.entries.get(&actor).ok_or(ProtectionError::IdentityStale)?;
        if !self.operations.is_live(actor)
            || self.operations.client_of(actor) != Some(entry.client)
            || self.operations.actor_of_client(entry.client) != Some(actor)
        {
            return Err(ProtectionError::IdentityStale);
        }
        Ok(entry)
    }

    fn source_slot(&mut self, actor: NativeActorId) -> Result<usize, ProtectionError> {
        self.operations.assert_current()?;
        self.require(actor)?;
        if !self.operations.is_eligible(actor) {
            return Err(ProtectionError::NotAdmitted);
        }
        let slot = self.operations.slot(actor).ok_or(ProtectionError::NoClientRecord)?;
        if self.operations.record_base(slot, "client").is_none() {
            return Err(ProtectionError::NoClientRecord);
        }
        if !self.operations.record_active(slot) {
            return Err(ProtectionError::InactiveRecord);
        }
        Ok(slot)
    }

    fn field_value(&mut self, slot: usize, field: &ProtectionField) -> Result<Option<f64>, ProtectionError> {
        let Some(base) = self.operations.record_base(slot, &field.record) else {
            return Ok(None);
        };
        Ok(Some(self.operations.scalar(base, field, None)?))
    }

    fn required_value(&mut self, slot: usize, field: &ProtectionField) -> Result<f64, ProtectionError> {
        self.field_value(slot, field)?.ok_or(ProtectionError::MissingStorage)
    }

    fn storage_read(&mut self, slot: usize) -> Result<ArmorState, ProtectionError> {
        let mut regular = RegularArmor::None;
        for definition in self.definitions.clone() {
            for item in definition.regular {
                if item
                    .selection
                    .is_selected(self.field_value(slot, item.selection.field())?)
                {
                    regular = RegularArmor::Source {
                        points: self.required_value(slot, &item.points)?,
                        item: Some(item.item),
                    };
                    break;
                }
            }
            if !matches!(regular, RegularArmor::None) {
                break;
            }
        }
        let mut powered = PoweredProtection::None;
        for definition in self.definitions.clone() {
            for item in definition.power {
                let selected = item
                    .selection
                    .is_selected(self.field_value(slot, item.selection.field())?);
                let enabled = match &item.enabled {
                    None => true,
                    Some((field, mask)) => self.required_value(slot, field)? as i64 & mask != 0,
                };
                if selected && enabled {
                    let cells = self.required_value(slot, &item.cells)? as i32;
                    powered = match item.kind {
                        PowerKind::Screen => PoweredProtection::Screen { cells },
                        PowerKind::Shield => PoweredProtection::Shield { cells },
                    };
                    break;
                }
            }
            if !matches!(powered, PoweredProtection::None) {
                break;
            }
        }
        Ok(ArmorState { regular, powered })
    }

    fn counter(&self, channel: ProtectionChannel, state: &ArmorState) -> Option<&Counter> {
        let selection = match channel {
            ProtectionChannel::Regular => match &state.regular {
                RegularArmor::Source { item: Some(item), .. } => item.clone(),
                _ => return None,
            },
            ProtectionChannel::Powered => match &state.powered {
                PoweredProtection::Screen { .. } => "screen".to_string(),
                PoweredProtection::Shield { .. } => "shield".to_string(),
                PoweredProtection::None => return None,
            },
        };
        self.counters
            .iter()
            .find(|counter| counter.channel == channel && counter.selection == selection)
    }

    /// Read armor with canonical inventory overrides for linked counters.
    pub fn read(&mut self, actor: NativeActorId) -> Result<ArmorState, ProtectionError> {
        let slot = self.source_slot(actor)?;
        let state = self.storage_read(slot)?;
        let regular = self.counter(ProtectionChannel::Regular, &state).cloned();
        let power = self.counter(ProtectionChannel::Powered, &state).cloned();
        Ok(ArmorState {
            regular: match (&state.regular, regular) {
                (RegularArmor::Source { item, .. }, Some(counter)) if counter.inventory.is_some() => {
                    let inventory = counter.inventory.clone().unwrap_or_default();
                    RegularArmor::Source {
                        points: self.operations.inventory_count(actor, &inventory),
                        item: item.clone(),
                    }
                }
                _ => state.regular,
            },
            powered: match (&state.powered, power) {
                (PoweredProtection::Screen { .. }, Some(counter))
                | (PoweredProtection::Shield { .. }, Some(counter))
                    if counter.inventory.is_some() =>
                {
                    let inventory = counter.inventory.clone().unwrap_or_default();
                    let cells = self.operations.inventory_count(actor, &inventory) as i32;
                    match state.powered {
                        PoweredProtection::Screen { .. } => PoweredProtection::Screen { cells },
                        _ => PoweredProtection::Shield { cells },
                    }
                }
                _ => state.powered,
            },
        })
    }

    /// Read one counter field.
    pub fn read_count(&mut self, actor: NativeActorId, field: &ProtectionField) -> Result<f64, ProtectionError> {
        let slot = self.source_slot(actor)?;
        self.required_value(slot, field)
    }

    #[allow(clippy::type_complexity)]
    fn plan_stores(
        &mut self,
        slot: usize,
        current: &ArmorState,
        next: &ArmorState,
        channel: ProtectionChannel,
    ) -> Result<Vec<(GuestAddress, ProtectionField, f64)>, ProtectionError> {
        if channel == ProtectionChannel::Powered {
            let changed = match (&current.powered, &next.powered) {
                (PoweredProtection::None, PoweredProtection::None) => false,
                (PoweredProtection::Screen { .. }, PoweredProtection::Screen { .. })
                | (PoweredProtection::Shield { .. }, PoweredProtection::Shield { .. }) => false,
                _ => true,
            };
            if changed {
                return Err(ProtectionError::PowerActivation);
            }
        }
        let mut stores = Vec::new();
        let mut store =
            |operations: &mut O, slot: usize, field: &ProtectionField, value: f64| -> Result<(), ProtectionError> {
                let Some(base) = operations.record_base(slot, &field.record) else {
                    return Err(ProtectionError::NoClientRecord);
                };
                stores.push((base, field.clone(), value));
                Ok(())
            };
        if channel == ProtectionChannel::Regular {
            let target = match &next.regular {
                RegularArmor::None => None,
                RegularArmor::Source {
                    item: Some(item),
                    points,
                } => Some((item.clone(), *points)),
                _ => return Err(ProtectionError::Unrepresentable),
            };
            // Deselect anything currently selected first.
            for definition in self.definitions.clone() {
                for item in definition.regular {
                    let selected = item
                        .selection
                        .is_selected(self.field_value(slot, item.selection.field())?);
                    if selected {
                        match &item.selection {
                            ArmorSelection::Enum { field, none, .. } => {
                                store(&mut self.operations, slot, field, *none)?;
                            }
                            ArmorSelection::Positive { field } => {
                                store(&mut self.operations, slot, field, 0.0)?;
                            }
                        }
                        store(&mut self.operations, slot, &item.points, 0.0)?;
                    }
                }
            }
            if let Some((item, points)) = target {
                let declared = self
                    .definitions
                    .iter()
                    .flat_map(|definition| definition.regular.iter())
                    .find(|candidate| candidate.item == item)
                    .cloned()
                    .ok_or(ProtectionError::Unrepresentable)?;
                match &declared.selection {
                    ArmorSelection::Enum { field, value, .. } => {
                        store(&mut self.operations, slot, field, *value)?;
                    }
                    ArmorSelection::Positive { field } => {
                        let current_value = self.required_value(slot, field)?.max(1.0);
                        store(&mut self.operations, slot, field, current_value)?;
                    }
                }
                store(&mut self.operations, slot, &declared.points, points)?;
            }
        } else {
            let target = match &next.powered {
                PoweredProtection::None => None,
                PoweredProtection::Screen { cells } => Some((PowerKind::Screen, f64::from(*cells))),
                PoweredProtection::Shield { cells } => Some((PowerKind::Shield, f64::from(*cells))),
            };
            if target.is_none() {
                for definition in self.definitions.clone() {
                    for item in definition.power {
                        match &item.selection {
                            ArmorSelection::Enum { field, none, .. } => {
                                store(&mut self.operations, slot, field, *none)?;
                            }
                            ArmorSelection::Positive { field } => {
                                store(&mut self.operations, slot, field, 0.0)?;
                            }
                        }
                        if let Some((field, mask)) = &item.enabled {
                            let current_value = self.required_value(slot, field)? as i64;
                            store(&mut self.operations, slot, field, (current_value & !mask) as f64)?;
                        }
                    }
                }
            }
            if let Some((kind, cells)) = target {
                let declared = self
                    .definitions
                    .iter()
                    .flat_map(|definition| definition.power.iter())
                    .find(|candidate| candidate.kind == kind)
                    .cloned()
                    .ok_or(ProtectionError::Unrepresentable)?;
                match &declared.selection {
                    ArmorSelection::Enum { field, value, .. } => {
                        store(&mut self.operations, slot, field, *value)?;
                    }
                    ArmorSelection::Positive { field } => {
                        let current_value = self.required_value(slot, field)?.max(1.0);
                        store(&mut self.operations, slot, field, current_value)?;
                    }
                }
                store(&mut self.operations, slot, &declared.cells, cells)?;
            }
        }
        Ok(stores)
    }

    /// Validate a channel write without committing it.
    pub fn validate_write(
        &mut self,
        actor: NativeActorId,
        channel: ProtectionChannel,
        next: &ArmorState,
    ) -> Result<(), ProtectionError> {
        let slot = self.source_slot(actor)?;
        let current = self.storage_read(slot)?;
        let merged = match channel {
            ProtectionChannel::Regular => ArmorState {
                regular: next.regular.clone(),
                powered: current.powered.clone(),
            },
            ProtectionChannel::Powered => ArmorState {
                regular: current.regular.clone(),
                powered: next.powered.clone(),
            },
        };
        self.plan_stores(slot, &current, &merged, channel)?;
        Ok(())
    }

    /// Commit a channel write to source storage.
    pub fn write(
        &mut self,
        actor: NativeActorId,
        channel: ProtectionChannel,
        next: &ArmorState,
    ) -> Result<(), ProtectionError> {
        let slot = self.source_slot(actor)?;
        let current = self.storage_read(slot)?;
        let merged = match channel {
            ProtectionChannel::Regular => ArmorState {
                regular: next.regular.clone(),
                powered: current.powered.clone(),
            },
            ProtectionChannel::Powered => ArmorState {
                regular: current.regular.clone(),
                powered: next.powered.clone(),
            },
        };
        let stores = self.plan_stores(slot, &current, &merged, channel)?;
        for stage in &mut self.stages {
            stage.suppressed += 1;
        }
        let outcome = (|| -> Result<(), ProtectionError> {
            for (base, field, value) in &stores {
                self.operations.scalar(*base, field, Some(*value))?;
            }
            Ok(())
        })();
        for stage in &mut self.stages {
            stage.suppressed -= 1;
        }
        outcome?;
        if self.operations.is_live(actor) {
            let armor = self.read(actor)?;
            for stage in &mut self.stages {
                if stage.actor == actor {
                    stage.armor = armor.clone();
                }
            }
        }
        Ok(())
    }

    /// Bind one actor's reservations.
    pub fn bind_actor(&mut self, actor: NativeActorId) -> Result<(), ProtectionError> {
        if !self.active || !self.operations.is_eligible(actor) {
            return Ok(());
        }
        self.require(actor)?;
        self.source_slot(actor)?;
        if let Some(entry) = self.entries.get_mut(&actor) {
            entry.bound = true;
        }
        Ok(())
    }

    /// Reserve and bind every connected client.
    pub fn activate(&mut self) -> Result<(), ProtectionError> {
        self.reserve()?;
        self.active = true;
        for actor in self.entries.keys().copied().collect::<Vec<_>>() {
            self.bind_actor(actor)?;
        }
        Ok(())
    }

    /// Reset for restore: drop bindings, then reserve again.
    pub fn prepare_restore(&mut self) -> Result<(), ProtectionError> {
        self.close();
        self.reserve()
    }

    /// Check restored counters against canonical inventory.
    pub fn validate_restored_inventory(&mut self) -> Result<(), ProtectionError> {
        for actor in self.entries.keys().copied().collect::<Vec<_>>() {
            let slot = self.source_slot(actor)?;
            for counter in self.counters.clone() {
                if let Some(inventory) = counter.inventory {
                    let stored = self.required_value(slot, &counter.field)?;
                    if stored != self.operations.inventory_count(actor, &inventory) {
                        return Err(ProtectionError::RestoreMismatch);
                    }
                }
            }
        }
        Ok(())
    }

    fn damage_flags(&self, channel: ProtectionChannel, input: &ArmorStageInput) -> i32 {
        let flags = &input.flags;
        (i32::from(input.delivery == Delivery::Radius))
            | (i32::from(
                flags.no_armor
                    || self.classic_api
                        && (if channel == ProtectionChannel::Powered {
                            flags.no_power
                        } else {
                            flags.no_regular
                        }),
            ) * 2)
            | (i32::from(flags.energy) * 4)
            | (i32::from(flags.no_regular) * 128)
            | (i32::from(!self.classic_api && flags.no_power) * 256)
    }

    fn authority_current(&mut self, token: u64) -> bool {
        let Some(snapshot) = self.authorities.get(&token).copied() else {
            return false;
        };
        self.active
            && self.attached
            && self.require(snapshot.actor).is_ok()
            && self.operations.is_eligible(snapshot.actor)
            && self.operations.slot(snapshot.actor) == Some(snapshot.slot)
            && self.operations.record_active(snapshot.slot)
    }

    /// Absorb damage through one definition's source entry.
    pub fn absorb(
        &mut self,
        actor: NativeActorId,
        definition_id: &str,
        input: &ArmorStageInput,
        observer: &mut dyn ProtectionObserver,
    ) -> Result<ArmorStageResult, ProtectionError> {
        if input.target != actor {
            return Err(ProtectionError::TargetMismatch);
        }
        let definition = self
            .definitions
            .iter()
            .find(|definition| definition.id == definition_id)
            .cloned()
            .ok_or(ProtectionError::BadDeclaration)?;
        let scale = input.regular_protection_scale;
        if definition.channel == ProtectionChannel::Regular && scale != 1.0 && !definition.declares_scale_input {
            return Err(ProtectionError::ScaleInputMissing);
        }
        let mut values = HashMap::new();
        values.insert("self".to_string(), RuntimeValue::Actor(Some(actor)));
        values.insert("amount".to_string(), RuntimeValue::Float(input.amount));
        values.insert(
            "damage-flags".to_string(),
            RuntimeValue::Float(f64::from(self.damage_flags(definition.channel, input))),
        );
        values.insert("regular-protection-scale".to_string(), RuntimeValue::Float(scale));
        values.insert("point".to_string(), RuntimeValue::Vector(input.point));
        values.insert("normal".to_string(), RuntimeValue::Vector(input.normal));
        values.insert("direction".to_string(), RuntimeValue::Vector(input.direction));
        values.insert("knockback".to_string(), RuntimeValue::Float(input.knockback));
        values.insert("time".to_string(), RuntimeValue::Float(self.operations.time_secs()));
        let inputs = ProtectionInputs { values };
        let (call, region) = match &definition.absorb {
            AbsorbDecl::SourceCall { call } => (call.clone(), None),
            AbsorbDecl::SourceRegion {
                call,
                frame_entry,
                entry_rva,
                join_rva,
                frame_exit,
            } => (
                call.clone(),
                Some(RegionRef {
                    frame_entry: *frame_entry,
                    entry_rva: *entry_rva,
                    join_rva: *join_rva,
                    frame_exit: *frame_exit,
                }),
            ),
            AbsorbDecl::Direct { entry, sparks } => {
                let id = match entry {
                    AddressRef::Export(name) => format!("direct:{name}"),
                    AddressRef::Rva(rva) => format!("direct:{rva:#x}"),
                };
                let _ = sparks;
                (ProtectionCall { id }, None)
            }
        };
        let slot = self.source_slot(actor)?;
        let authority = if region.is_some() {
            let token = self.next_authority;
            self.next_authority += 1;
            self.authorities.insert(token, AuthoritySnapshot { actor, slot });
            Some(token)
        } else {
            None
        };
        let saved = self.observe(actor, observer, authority, |operations| {
            operations.invoke(&call, &inputs, region, authority)
        })?;
        if let Some(token) = authority {
            self.authorities.remove(&token);
        }
        Ok(ArmorStageResult {
            saved: f64::from(saved.unwrap_or(0)),
        })
    }

    /// Observe source armor words around a source invocation.
    pub fn observe<R>(
        &mut self,
        actor: NativeActorId,
        observer: &mut dyn ProtectionObserver,
        authority: Option<u64>,
        invoke: impl FnOnce(&mut O) -> R,
    ) -> Result<R, ProtectionError> {
        if authority.is_some_and(|token| !self.authority_current(token)) {
            return Err(ProtectionError::Retired);
        }
        let slot = self.source_slot(actor)?;
        let armor = self.read(actor)?;
        self.stages.push(Stage {
            actor,
            armor,
            suppressed: 0,
        });
        // Watch every counter field for this slot.
        let mut groups: HashMap<u64, (GuestAddress, Vec<ProtectionField>)> = HashMap::new();
        for counter in self.counters.clone() {
            if let Some(base) = self.operations.record_base(slot, &counter.field.record) {
                groups
                    .entry(base.offset)
                    .or_insert_with(|| (base, Vec::new()))
                    .1
                    .push(counter.field);
            }
        }
        let dirty = Rc::new(RefCell::new(false));
        let mut ids = Vec::new();
        for (base, fields) in groups.values() {
            let pointer_bytes = self.operations.memory().pointer_bytes();
            let start = fields.iter().map(|field| field.offset).min().unwrap_or(0);
            let end = fields
                .iter()
                .map(|field| field.offset + field.width(pointer_bytes))
                .max()
                .unwrap_or(0);
            let address = self.operations.memory().offset(*base, start as i64)?;
            let _watched = fields.clone();
            let flag = dirty.clone();
            let id = self.operations.memory().observe_writes(
                address,
                end.saturating_sub(start),
                Box::new(move |_| {
                    *flag.borrow_mut() = true;
                }),
            )?;
            ids.push(id);
        }
        let result = self.operations.transfer(invoke);
        if authority.is_some_and(|token| !self.authority_current(token)) {
            self.teardown_observers(&ids, actor);
            return Err(ProtectionError::Retired);
        }
        // Flush canonical inventory changes back into counters first.
        let changes = self.operations.flush();
        for change in &changes {
            if change.actor != actor {
                continue;
            }
            let Some(before) = change.before else {
                self.teardown_observers(&ids, actor);
                return Err(ProtectionError::MissingBaseline);
            };
            let _ = before;
            for counter in self.counters.clone() {
                if counter.inventory.as_deref() == Some(change.item.as_str()) {
                    let stored = self.required_value(slot, &counter.field)?;
                    if stored != change.after {
                        for stage in &mut self.stages {
                            stage.suppressed += 1;
                        }
                        let base = self
                            .operations
                            .record_base(slot, &counter.field.record)
                            .ok_or(ProtectionError::NoClientRecord)?;
                        let outcome = self.operations.scalar(base, &counter.field, Some(change.after));
                        for stage in &mut self.stages {
                            stage.suppressed -= 1;
                        }
                        outcome?;
                    }
                }
            }
        }
        if dirty.replace(false) || !changes.is_empty() {
            self.publish(actor, observer, authority)?;
        }
        self.teardown_observers(&ids, actor);
        Ok(result)
    }

    fn publish(
        &mut self,
        actor: NativeActorId,
        observer: &mut dyn ProtectionObserver,
        authority: Option<u64>,
    ) -> Result<(), ProtectionError> {
        if authority.is_some_and(|token| !self.authority_current(token)) {
            return Err(ProtectionError::Retired);
        }
        if !self.operations.is_live(actor) {
            return Ok(());
        }
        self.source_slot(actor)?;
        let before = self
            .stages
            .iter()
            .find(|stage| stage.actor == actor)
            .map(|stage| stage.armor.clone())
            .unwrap_or_else(|| ArmorState {
                regular: RegularArmor::None,
                powered: PoweredProtection::None,
            });
        let after = self.read(actor)?;
        for stage in &mut self.stages {
            if stage.actor == actor {
                stage.armor = after.clone();
            }
        }
        let suppressed = self
            .stages
            .iter()
            .find(|stage| stage.actor == actor)
            .is_some_and(|stage| stage.suppressed > 0);
        if suppressed {
            return Ok(());
        }
        let regular = if before.regular != after.regular {
            Some((before.regular, after.regular))
        } else {
            None
        };
        let powered = if before.powered != after.powered {
            Some((before.powered, after.powered))
        } else {
            None
        };
        if regular.is_some() || powered.is_some() {
            observer.stored(regular, powered);
        }
        if authority.is_some_and(|token| !self.authority_current(token)) {
            return Err(ProtectionError::Retired);
        }
        Ok(())
    }

    fn teardown_observers(&mut self, ids: &[u64], actor: NativeActorId) {
        let memory = self.operations.memory();
        for id in ids {
            memory.unobserve(*id);
        }
        if let Some(position) = self.stages.iter().rposition(|stage| stage.actor == actor) {
            self.stages.remove(position);
        }
    }

    /// Release one actor's reservations.
    pub fn release(&mut self, actor: NativeActorId) {
        self.stages.retain(|stage| stage.actor != actor);
        self.entries.remove(&actor);
    }

    /// Release every reservation.
    pub fn close(&mut self) {
        self.active = false;
        self.stages.clear();
        self.entries.clear();
        self.authorities.clear();
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

    fn field(offset: usize) -> ProtectionField {
        ProtectionField {
            record: "client".to_string(),
            offset,
            storage: GuestStorage::Int32,
        }
    }

    fn definitions() -> Vec<ProtectionDefinition> {
        vec![
            ProtectionDefinition {
                id: "regular".to_string(),
                channel: ProtectionChannel::Regular,
                absorb: AbsorbDecl::SourceCall {
                    call: ProtectionCall {
                        id: "absorb-regular".to_string(),
                    },
                },
                regular: vec![RegularStorage {
                    item: "q2:jacket".to_string(),
                    selection: ArmorSelection::Enum {
                        field: field(0),
                        value: 1.0,
                        none: 0.0,
                    },
                    points: field(4),
                }],
                power: Vec::new(),
                declares_scale_input: true,
            },
            ProtectionDefinition {
                id: "powered".to_string(),
                channel: ProtectionChannel::Powered,
                absorb: AbsorbDecl::SourceCall {
                    call: ProtectionCall {
                        id: "absorb-power".to_string(),
                    },
                },
                regular: Vec::new(),
                power: vec![PowerStorage {
                    kind: PowerKind::Screen,
                    selection: ArmorSelection::Enum {
                        field: field(8),
                        value: 1.0,
                        none: 0.0,
                    },
                    cells: field(12),
                    enabled: None,
                }],
                declares_scale_input: false,
            },
        ]
    }

    type Handler = Rc<dyn Fn(&mut TestOps, &ProtectionCall, &ProtectionInputs) -> Option<i32>>;

    struct TestOps {
        memory: SparseGuestMemory,
        image_base: GuestAddress,
        rows: HashMap<u32, GuestAddress>,
        live: HashSet<NativeActorId>,
        eligible: HashSet<NativeActorId>,
        slots: HashMap<NativeActorId, usize>,
        clients: HashMap<NativeActorId, NativeClientId>,
        inventory: HashMap<(NativeActorId, String), f64>,
        handler: Handler,
        invoked: Vec<String>,
        flushed: Vec<InventoryChange>,
        time: f64,
    }

    impl TestOps {
        fn scalar_to_f64(bytes: &[u8]) -> f64 {
            f64::from(i32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
        }
    }

    impl ProtectionOperations for TestOps {
        fn assert_current(&self) -> Result<(), ProtectionError> {
            Ok(())
        }

        fn is_live(&self, actor: NativeActorId) -> bool {
            self.live.contains(&actor)
        }

        fn is_eligible(&self, actor: NativeActorId) -> bool {
            self.eligible.contains(&actor)
        }

        fn slot(&self, actor: NativeActorId) -> Option<usize> {
            self.slots.get(&actor).copied()
        }

        fn client_of(&self, actor: NativeActorId) -> Option<NativeClientId> {
            self.clients.get(&actor).copied()
        }

        fn actor_of_client(&self, client: NativeClientId) -> Option<NativeActorId> {
            self.clients
                .iter()
                .find(|(_, bound)| **bound == client)
                .map(|(actor, _)| *actor)
        }

        fn client_actors(&self) -> Vec<NativeActorId> {
            self.clients.keys().copied().collect()
        }

        fn inventory_count(&self, actor: NativeActorId, item: &str) -> f64 {
            self.inventory.get(&(actor, item.to_string())).copied().unwrap_or(0.0)
        }

        fn owns_inventory(&self, actor: NativeActorId, item: &str) -> bool {
            self.inventory.contains_key(&(actor, item.to_string()))
        }

        fn scalar(
            &mut self,
            base: GuestAddress,
            field: &ProtectionField,
            value: Option<f64>,
        ) -> Result<f64, ProtectionError> {
            let address = self.memory.offset(base, field.offset as i64)?;
            if let Some(value) = value {
                self.memory.write(address, &(value as i32).to_le_bytes())?;
            }
            Ok(Self::scalar_to_f64(&self.memory.copy(address, 4)?))
        }

        fn transfer<R>(&mut self, invoke: impl FnOnce(&mut Self) -> R) -> R {
            invoke(self)
        }

        fn flush(&mut self) -> Vec<InventoryChange> {
            std::mem::take(&mut self.flushed)
        }

        fn invoke(
            &mut self,
            call: &ProtectionCall,
            inputs: &ProtectionInputs,
            _region: Option<RegionRef>,
            _authority: Option<u64>,
        ) -> Option<i32> {
            self.invoked.push(call.id.clone());
            let handler = self.handler.clone();
            handler(self, call, inputs)
        }

        fn memory(&mut self) -> &mut SparseGuestMemory {
            &mut self.memory
        }

        fn image_base(&self) -> GuestAddress {
            self.image_base
        }

        fn entry(&self, _name: &str) -> Option<GuestAddress> {
            None
        }

        fn record_base(&mut self, slot: usize, record: &str) -> Option<GuestAddress> {
            if record != "client" {
                return None;
            }
            self.rows.get(&(slot as u32)).copied()
        }

        fn record_active(&self, slot: usize) -> bool {
            self.rows.contains_key(&(slot as u32))
        }

        fn time_secs(&self) -> f64 {
            self.time
        }
    }

    struct Fixture {
        protection: NativeModProtection<TestOps>,
    }

    fn fixture(handler: Handler) -> Fixture {
        let module = ModuleIdentity::new(
            ProviderId::new("test", "protection"),
            "protection.so",
            ContentDigest {
                algorithm: "none".to_string(),
                value: "0".to_string(),
            },
            "r1",
        );
        let mut memory = SparseGuestMemory::new(module, 4, 0xE0000).unwrap();
        let image_base = memory
            .map(&GuestMapOptions::new(0x0, 0x2000, GuestPermissions::ReadWriteExecute))
            .unwrap();
        let row = memory
            .map(&GuestMapOptions::new(0x30000, 64, GuestPermissions::ReadWrite))
            .unwrap();
        let operations = TestOps {
            memory,
            image_base,
            rows: [(1u32, row)].into_iter().collect(),
            live: [actor(1)].into_iter().collect(),
            eligible: [actor(1)].into_iter().collect(),
            slots: [(actor(1), 1)].into_iter().collect(),
            clients: [(actor(1), client(1))].into_iter().collect(),
            inventory: [((actor(1), "q2:cells".to_string()), 50.0)].into_iter().collect(),
            handler,
            invoked: Vec::new(),
            flushed: Vec::new(),
            time: 3.0,
        };
        let protection = NativeModProtection::new(
            definitions(),
            "test:mod",
            true,
            &[(field(12), "q2:cells".to_string())],
            operations,
        )
        .unwrap();
        Fixture { protection }
    }

    fn input() -> ArmorStageInput {
        ArmorStageInput {
            target: actor(1),
            amount: 40.0,
            delivery: Delivery::Direct,
            flags: StageFlags {
                no_armor: false,
                energy: false,
                no_regular: false,
                no_power: false,
            },
            point: Vec3 { x: 0.0, y: 0.0, z: 0.0 },
            normal: Vec3 { x: 0.0, y: 0.0, z: 1.0 },
            direction: Vec3 { x: 1.0, y: 0.0, z: 0.0 },
            knockback: 0.0,
            regular_protection_scale: 1.0,
        }
    }

    struct Recorder {
        events: Vec<(
            Option<(RegularArmor, RegularArmor)>,
            Option<(PoweredProtection, PoweredProtection)>,
        )>,
    }

    impl ProtectionObserver for Recorder {
        fn stored(
            &mut self,
            regular: Option<(RegularArmor, RegularArmor)>,
            powered: Option<(PoweredProtection, PoweredProtection)>,
        ) {
            self.events.push((regular, powered));
        }
    }

    #[test]
    fn absorb_saves_and_power_activation_is_rejected() {
        let handler: Handler = Rc::new(|operations, call, inputs| {
            assert_eq!(call.id, "absorb-regular");
            assert_eq!(inputs.get("amount"), Some(&RuntimeValue::Float(40.0)));
            let base = operations.record_base(1, "client").unwrap();
            operations.scalar(base, &field(0), Some(1.0)).unwrap();
            operations.scalar(base, &field(4), Some(70.0)).unwrap();
            Some(25)
        });
        let mut fixture = fixture(handler);
        fixture.protection.attach().unwrap();
        fixture.protection.activate().unwrap();
        // Seed jacket storage before absorbing.
        fixture
            .protection
            .write(
                actor(1),
                ProtectionChannel::Regular,
                &ArmorState {
                    regular: RegularArmor::Source {
                        points: 100.0,
                        item: Some("q2:jacket".to_string()),
                    },
                    powered: PoweredProtection::None,
                },
            )
            .unwrap();
        let mut recorder = Recorder { events: Vec::new() };
        let result = fixture
            .protection
            .absorb(actor(1), "regular", &input(), &mut recorder)
            .unwrap();
        assert_eq!(result.saved, 25.0);
        assert_eq!(recorder.events.len(), 1);
        assert!(matches!(
            &recorder.events[0].0,
            Some((
                RegularArmor::Source { points: 100.0, .. },
                RegularArmor::Source { points: 70.0, .. }
            ))
        ));

        // Power activation without its source operation is rejected.
        let activation = fixture.protection.write(
            actor(1),
            ProtectionChannel::Powered,
            &ArmorState {
                regular: RegularArmor::None,
                powered: PoweredProtection::Screen { cells: 10 },
            },
        );
        assert_eq!(activation, Err(ProtectionError::PowerActivation));
    }

    #[test]
    fn flush_syncs_fuel_and_restore_validates() {
        let handler: Handler = Rc::new(|_, _, _| Some(0));
        let mut fixture = fixture(handler);
        fixture.protection.attach().unwrap();
        fixture.protection.activate().unwrap();
        // Seed screen storage directly, then flush a canonical change.
        let base = fixture.protection.operations_mut().record_base(1, "client").unwrap();
        fixture
            .protection
            .operations_mut()
            .scalar(base, &field(8), Some(1.0))
            .unwrap();
        fixture
            .protection
            .operations_mut()
            .scalar(base, &field(12), Some(50.0))
            .unwrap();
        fixture.protection.operations_mut().flushed.push(InventoryChange {
            actor: actor(1),
            item: "q2:cells".to_string(),
            before: Some(50.0),
            after: 40.0,
        });
        let mut recorder = Recorder { events: Vec::new() };
        fixture
            .protection
            .absorb(actor(1), "powered", &input(), &mut recorder)
            .unwrap();
        let cells = fixture
            .protection
            .operations_mut()
            .scalar(base, &field(12), None)
            .unwrap();
        assert_eq!(cells, 40.0);

        fixture
            .protection
            .operations_mut()
            .scalar(base, &field(12), Some(99.0))
            .unwrap();
        assert_eq!(
            fixture.protection.validate_restored_inventory(),
            Err(ProtectionError::RestoreMismatch)
        );
    }
}
