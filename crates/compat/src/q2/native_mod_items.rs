//! Port of `src/compat/q2/native-mod-items.ts`.
//! Bridges source inventories: original native words are borrowed while
//! observers publish committed stores exactly once per poll.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use qa_guest::core::contracts::{GuestAddress, GuestStorage};
use qa_guest::core::memory::SparseGuestMemory;
use qa_guest::error::GuestError;
use qa_world::combat::ItemId;
use qa_world::inventory::{CountArithmetic, CountPolicy, InventoryEntry};
use thiserror::Error;

/// Failures in the native item bridge.
#[derive(Debug, Clone, PartialEq, Error)]
pub enum ItemError {
    /// Items need original client admission.
    #[error("native items require original client admission")]
    MissingClients,
    /// A duplicate source item was declared.
    #[error("duplicate native source item")]
    DuplicateItem,
    /// An item exceeds its client record.
    #[error("native source item exceeds its client record")]
    FieldRange,
    /// Two items overlap exclusively-owned storage.
    #[error("overlapping native source item storage")]
    OverlappingStorage,
    /// An item lacks distinct storage.
    #[error("native source item lacks distinct storage")]
    MissingStorage,
    /// A definition has no storage.
    #[error("native item definition lacks source storage")]
    UnboundDefinition,
    /// A packed declaration is invalid.
    #[error("invalid native packed inventory")]
    BadPacked,
    /// Two packed items share a bit.
    #[error("overlapping native item bits")]
    OverlappingBits,
    /// Packed bits exceed the storage width.
    #[error("native packed item exceeds storage width")]
    PackedWidth,
    /// A capacity constant is invalid.
    #[error("invalid native source capacity")]
    BadCapacity,
    /// Weapon items and the dispatcher disagree.
    #[error("native weapon items require an original dispatcher")]
    WeaponMismatch,
    /// A weapon stage declaration is invalid.
    #[error("native weapon stage lacks original decisions or actor ABI")]
    BadWeaponStage,
    /// Weapon pointers differ from admitted definitions.
    #[error("native weapon pointers differ from admitted source definitions")]
    SelectionMismatch,
    /// Weapon ammo lacks original storage.
    #[error("native weapon ammo lacks original storage")]
    MissingAmmo,
    /// A count is not finite.
    #[error("native inventory requires a finite source count")]
    BadCount,
    /// A count exceeds its integer storage.
    #[error("native inventory count exceeds its original integer storage")]
    CountRange,
    /// Undeclared bits are set in packed storage.
    #[error("native inventory contains undeclared item bits")]
    UndeclaredBits,
    /// A lease lost its original storage.
    #[error("native item lease lost its original storage")]
    LostLease,
    /// Capacity is owned by the original source.
    #[error("native capacity is owned by its original source")]
    ForeignCapacity,
    /// A packed write needs its ownership bit shape.
    #[error("native item requires its original ownership bit")]
    BadPackedWrite,
    /// A pickup changed an undeclared item.
    #[error("original pickup changed an undeclared native source item")]
    PickupScope,
    /// Admission needs the current admitted client.
    #[error("native items require the current admitted client")]
    BadAdmission,
    /// A source action is no longer admitted.
    #[error("source item action is no longer admitted")]
    ActionGone,
    /// A weapon request lost its actor.
    #[error("native weapon request lost its original actor")]
    RequestGone,
    /// A weapon presentation lost its owner.
    #[error("native weapon presentation lost its source owner")]
    PresentationGone,
    /// A saved request differs from its owner.
    #[error("saved native weapon request differs from its source owner")]
    SavedRequestMismatch,
    /// A saved request lost its owner.
    #[error("saved native request lost its exact source owner")]
    SavedOwnerGone,
    /// A saved request differs from its declaration.
    #[error("saved native request differs from its source declaration")]
    SavedDeclarationMismatch,
    /// A retired lease was used.
    #[error("native item lease retired")]
    Retired,
    /// Underlying guest failure.
    #[error("native item guest failure: {0}")]
    Guest(String),
}

impl From<GuestError> for ItemError {
    fn from(error: GuestError) -> Self {
        Self::Guest(error.to_string())
    }
}

/// Generational actor handle local to the item bridge.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct NativeActorId {
    /// Registry slot.
    pub slot: u32,
    /// Slot generation.
    pub generation: u32,
}

/// Save-safe actor reference.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

/// Scalar item field inside a record.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ItemField {
    /// Owning record.
    pub record: String,
    /// Byte offset.
    pub offset: usize,
    /// Lane storage.
    pub storage: GuestStorage,
}

impl ItemField {
    fn width(&self, pointer_bytes: usize) -> usize {
        self.storage.byte_length(pointer_bytes)
    }
}

/// Address reference: export name or image RVA.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AddressRef {
    /// Named export.
    Export(String),
    /// Image-relative offset.
    Rva(u64),
}

/// Counter capacity declaration.
#[derive(Debug, Clone, PartialEq)]
pub enum ItemCapacity {
    /// Fixed capacity.
    Constant(f64),
    /// Capacity stored in a field.
    Field(ItemField),
    /// Capacity stored at a source address.
    Source {
        /// Source address.
        address: AddressRef,
        /// Lane storage.
        storage: GuestStorage,
    },
}

/// One packed ownership bit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackedBit {
    /// Canonical item.
    pub item: ItemId,
    /// Ownership bit.
    pub mask: u32,
}

/// Item storage declaration.
#[derive(Debug, Clone, PartialEq)]
pub enum ItemStorage {
    /// Counter with capacity.
    Counter {
        /// Canonical item.
        item: ItemId,
        /// Count field.
        field: ItemField,
        /// Capacity.
        capacity: ItemCapacity,
    },
    /// Packed ownership bits.
    Packed {
        /// Packed field.
        field: ItemField,
        /// Declared bits.
        items: Vec<PackedBit>,
        /// Bits owned privately by the source.
        private_mask: u32,
    },
}

impl ItemStorage {
    fn field(&self) -> &ItemField {
        match self {
            Self::Counter { field, .. } | Self::Packed { field, .. } => field,
        }
    }
}

/// Item kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ItemKind {
    /// Weapon item.
    Weapon,
    /// Any other item.
    Other,
}

/// Declared source call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ItemCall {
    /// Call id.
    pub id: String,
}

/// One source item action.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ItemAction {
    /// Action name.
    pub action: String,
    /// Source call.
    pub call: ItemCall,
}

/// Item definition.
#[derive(Debug, Clone, PartialEq)]
pub struct ItemDefinition {
    /// Canonical item.
    pub item: ItemId,
    /// Item kind.
    pub kind: ItemKind,
    /// Ammo item, if any.
    pub ammo: Option<ItemId>,
    /// Source actions.
    pub actions: Vec<ItemAction>,
}

/// Weapon stage shape referenced by item validation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WeaponStageRef {
    /// Dispatcher argument count.
    pub dispatcher_arguments: usize,
    /// Dispatcher actor argument index.
    pub dispatcher_argument: usize,
    /// Decision region count.
    pub decision_regions: usize,
    /// Settled group count.
    pub settled_groups: usize,
    /// Continuation group count.
    pub continuation_groups: usize,
    /// Selection items.
    pub selection_items: Vec<ItemId>,
}

/// Native items declaration.
#[derive(Debug, Clone, PartialEq)]
pub struct ItemsDefinition {
    /// Item definitions.
    pub definitions: Vec<ItemDefinition>,
    /// Storage declarations.
    pub storage: Vec<ItemStorage>,
    /// Weapon stage reference, if any.
    pub weapons: Option<WeaponStageRef>,
}

/// Validate an items declaration against record layouts.
pub fn validate_native_mod_items(
    definition: &ItemsDefinition,
    records: &[(String, usize)],
    client_records: &[String],
    has_clients: bool,
    pointer_bytes: usize,
) -> Result<(), ItemError> {
    if !has_clients || definition.definitions.is_empty() {
        return Err(ItemError::MissingClients);
    }
    let stride = |id: &str| {
        records
            .iter()
            .find(|(record, _)| record == id)
            .map(|(_, stride)| *stride)
    };
    let mut defined = HashSet::new();
    for item in &definition.definitions {
        if !defined.insert(item.item.clone()) {
            return Err(ItemError::DuplicateItem);
        }
    }
    let mut occupied: HashMap<String, HashMap<usize, Option<String>>> = HashMap::new();
    let mut check_field = |record: &str,
                           offset: usize,
                           length: usize,
                           exclusive: bool,
                           shared: Option<String>|
     -> Result<(), ItemError> {
        let Some(stride) = stride(record) else {
            return Err(ItemError::FieldRange);
        };
        if !client_records.iter().any(|id| id == record) || offset + length > stride {
            return Err(ItemError::FieldRange);
        }
        if exclusive {
            let bytes = occupied.entry(record.to_string()).or_default();
            for index in offset..offset + length {
                if let Some(previous) = bytes.get(&index) {
                    if shared.is_none() || *previous != shared {
                        return Err(ItemError::OverlappingStorage);
                    }
                }
                bytes.insert(index, shared.clone());
            }
        }
        Ok(())
    };
    let mut bound = HashSet::new();
    for storage in &definition.storage {
        let field = storage.field();
        check_field(&field.record, field.offset, field.width(pointer_bytes), true, None)?;
        match storage {
            ItemStorage::Counter {
                item,
                capacity,
                field: _,
            } => {
                if let ItemCapacity::Field(capacity) = capacity {
                    check_field(
                        &capacity.record,
                        capacity.offset,
                        capacity.width(pointer_bytes),
                        true,
                        Some(format!("{}:{:?}", capacity.offset, capacity.storage)),
                    )?;
                }
                if !defined.contains(item) || !bound.insert(item.clone()) {
                    return Err(ItemError::MissingStorage);
                }
                if let ItemCapacity::Constant(value) = capacity {
                    if !value.is_finite() || *value < 0.0 {
                        return Err(ItemError::BadCapacity);
                    }
                }
            }
            ItemStorage::Packed {
                field,
                items,
                private_mask,
            } => {
                if !matches!(
                    field.storage,
                    GuestStorage::Int8
                        | GuestStorage::Uint8
                        | GuestStorage::Int16
                        | GuestStorage::Uint16
                        | GuestStorage::Int32
                        | GuestStorage::Uint32
                ) || items.is_empty()
                    || *private_mask > 0xffff_ffff
                {
                    return Err(ItemError::BadPacked);
                }
                let mut mask = *private_mask;
                for bit in items {
                    if !defined.contains(&bit.item) || !bound.insert(bit.item.clone()) {
                        return Err(ItemError::MissingStorage);
                    }
                    if bit.mask < 1
                        || bit.mask > 0x8000_0000
                        || (bit.mask & (bit.mask - 1)) != 0
                        || (mask & bit.mask) != 0
                    {
                        return Err(ItemError::OverlappingBits);
                    }
                    mask |= bit.mask;
                }
                let width = field.width(pointer_bytes);
                if width < 4 && (mask as u64) >= (1u64 << (width * 8)) {
                    return Err(ItemError::PackedWidth);
                }
            }
        }
    }
    if bound.len() != defined.len() {
        return Err(ItemError::UnboundDefinition);
    }
    let weapons: Vec<&ItemDefinition> = definition
        .definitions
        .iter()
        .filter(|item| item.kind == ItemKind::Weapon)
        .collect();
    if weapons.is_empty() != definition.weapons.is_none() {
        return Err(ItemError::WeaponMismatch);
    }
    if let Some(stage) = &definition.weapons {
        if stage.dispatcher_arguments < 1
            || stage.dispatcher_arguments > 16
            || stage.dispatcher_argument >= stage.dispatcher_arguments
            || stage.decision_regions == 0
            || stage.settled_groups == 0
            || stage.continuation_groups == 0
        {
            return Err(ItemError::BadWeaponStage);
        }
        if stage.selection_items.len() != weapons.len()
            || !stage
                .selection_items
                .iter()
                .all(|item| weapons.iter().any(|weapon| &weapon.item == item))
        {
            return Err(ItemError::SelectionMismatch);
        }
    }
    for weapon in &weapons {
        if weapon.ammo.as_ref().is_some_and(|ammo| !defined.contains(ammo)) {
            return Err(ItemError::MissingAmmo);
        }
    }
    Ok(())
}

fn validate_counter(value: f64, field: &ItemField, pointer_bytes: usize) -> Result<(), ItemError> {
    if !value.is_finite() {
        return Err(ItemError::BadCount);
    }
    if field.storage.is_float() {
        return Ok(());
    }
    let bits = field.width(pointer_bytes) * 8;
    let signed = matches!(
        field.storage,
        GuestStorage::Int8 | GuestStorage::Int16 | GuestStorage::Int32 | GuestStorage::Int64
    );
    let bound = 2f64.powi(bits as i32 - i32::from(signed));
    if value.trunc() != value
        || value.abs() > 9_007_199_254_740_992.0
        || value < if signed { -bound } else { 0.0 }
        || value >= bound
    {
        return Err(ItemError::CountRange);
    }
    Ok(())
}

/// Item operations: record addressing plus source scalar access.
pub trait ItemOperations {
    /// Guest memory backing the records.
    fn memory(&mut self) -> &mut SparseGuestMemory;
    /// Record base for an actor.
    fn pointer(&mut self, actor: NativeActorId, record: &str) -> Result<GuestAddress, ItemError>;
    /// Resolve a declared address.
    fn resolve(&self, address: &AddressRef) -> Result<GuestAddress, ItemError>;
    /// Whether an actor is live and admitted.
    fn is_live(&self, actor: NativeActorId) -> bool;
    /// Read a scalar.
    fn read(&mut self, address: GuestAddress, storage: GuestStorage) -> Result<f64, ItemError>;
    /// Write a scalar.
    fn write(&mut self, address: GuestAddress, storage: GuestStorage, value: f64) -> Result<(), ItemError>;
    /// Invoke a source call for an actor.
    fn invoke(&mut self, actor: NativeActorId, call: &ItemCall);
    /// Declared inventory writes of the active pickup, if any.
    fn pickup_cover(&self) -> Option<Vec<ItemId>>;
    /// Publish committed inventory stores.
    fn publish_stores(&mut self, actor: NativeActorId, changes: &[(InventoryEntry, InventoryEntry)]);
    /// Whether a lease retired.
    fn is_retired(&self, actor: NativeActorId) -> bool;
    /// Whether an actor holds weapon selection.
    fn weapon_selected(&self, actor: NativeActorId) -> bool;
    /// View-model index for an actor.
    fn weapon_model(&self, actor: NativeActorId) -> u32;
}

/// Weapon stage surface used by item requests.
pub trait ItemWeaponStage {
    /// Pending weapon item, if any.
    fn pending(&mut self, actor: NativeActorId) -> Option<ItemId>;
    /// Active weapon item, if any.
    fn active(&mut self, actor: NativeActorId) -> Option<ItemId>;
    /// Whether the stage settled for an actor.
    fn settled(&mut self, actor: NativeActorId) -> bool;
    /// Whether the stage continues for an actor.
    fn continuing(&mut self, actor: NativeActorId) -> bool;
    /// Request a weapon through its source call.
    fn request(&mut self, actor: NativeActorId, item: &ItemId) -> bool;
}

/// Weapon request status.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequestStatus {
    /// Awaiting source decision.
    Pending,
    /// Accepted by the source.
    Accepted,
    /// Refused by the source.
    Refused,
}

#[derive(Debug, Clone, PartialEq)]
struct WeaponRequest {
    id: u64,
    item: Option<ItemId>,
    status: RequestStatus,
}

struct ItemEntry {
    addresses: HashMap<String, u64>,
    previous: HashMap<ItemId, InventoryEntry>,
    observers: Vec<u64>,
    request: Option<WeaponRequest>,
    source_selection: Option<ItemId>,
}

/// Weapon presentation snapshot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WeaponPresentation {
    /// Active weapon item, if any.
    pub active: Option<ItemId>,
    /// Pending weapon item, if any.
    pub pending: Option<ItemId>,
    /// View-model index.
    pub model: u32,
}

/// Saved weapon request row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SavedItemRequest {
    /// Requesting actor.
    pub actor: SavedActorId,
    /// Request id.
    pub id: u64,
    /// Requested item, if any.
    pub item: Option<ItemId>,
    /// Request status.
    pub status: RequestStatus,
}

/// Saved item checkpoint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeItemCheckpoint {
    /// Checkpoint version.
    pub version: u32,
    /// Next request id.
    pub next_request: u64,
    /// Open requests.
    pub requests: Vec<SavedItemRequest>,
}

/// Native item bridge over synthetic operations.
pub struct NativeModItems<O: ItemOperations, W: ItemWeaponStage> {
    definition: ItemsDefinition,
    operations: O,
    stage: Option<W>,
    entries: HashMap<NativeActorId, ItemEntry>,
    by_item: HashMap<ItemId, usize>,
    next_request: u64,
    writing: u32,
    requesting: HashSet<NativeActorId>,
    dirty: Rc<RefCell<HashSet<(NativeActorId, usize)>>>,
}

impl<O: ItemOperations, W: ItemWeaponStage> NativeModItems<O, W> {
    /// Build the bridge.
    pub fn new(definition: ItemsDefinition, operations: O, stage: Option<W>) -> Self {
        let mut by_item = HashMap::new();
        for (index, storage) in definition.storage.iter().enumerate() {
            match storage {
                ItemStorage::Counter { item, .. } => {
                    by_item.insert(item.clone(), index);
                }
                ItemStorage::Packed { items, .. } => {
                    for bit in items {
                        by_item.insert(bit.item.clone(), index);
                    }
                }
            }
        }
        Self {
            definition,
            operations,
            stage,
            entries: HashMap::new(),
            by_item,
            next_request: 0,
            writing: 0,
            requesting: HashSet::new(),
            dirty: Rc::new(RefCell::new(HashSet::new())),
        }
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

    fn address(&mut self, actor: NativeActorId, field: &ItemField) -> Result<GuestAddress, ItemError> {
        let base = self.operations.pointer(actor, &field.record)?;
        Ok(self.operations.memory().offset(base, field.offset as i64)?)
    }

    fn capacity(&mut self, actor: NativeActorId, capacity: &ItemCapacity) -> Result<f64, ItemError> {
        match capacity {
            ItemCapacity::Constant(value) => Ok(*value),
            ItemCapacity::Field(field) => {
                let address = self.address(actor, field)?;
                let storage = field.storage;
                self.operations.read(address, storage)
            }
            ItemCapacity::Source { address, storage } => {
                let resolved = self.operations.resolve(address)?;
                self.operations.read(resolved, *storage)
            }
        }
    }

    fn read_storage(&mut self, actor: NativeActorId, storage: &ItemStorage) -> Result<Vec<InventoryEntry>, ItemError> {
        let pointer_bytes = self.operations.memory().pointer_bytes();
        let field = storage.field().clone();
        let address = self.address(actor, &field)?;
        let count = self.operations.read(address, field.storage)?;
        match storage {
            ItemStorage::Counter { item, field, capacity } => {
                let arithmetic = match field.storage {
                    GuestStorage::Float32 => CountArithmetic::Binary32,
                    GuestStorage::Int32 => CountArithmetic::Int32,
                    _ => CountArithmetic::Binary64,
                };
                Ok(vec![InventoryEntry {
                    item: item.clone(),
                    count,
                    capacity: self.capacity(actor, capacity)?,
                    count_policy: Some(CountPolicy::SourceCounter(arithmetic)),
                }])
            }
            ItemStorage::Packed {
                field,
                items,
                private_mask,
            } => {
                let width = field.width(pointer_bytes);
                let mask = items.iter().fold(*private_mask, |mask, bit| mask | bit.mask);
                let packed = (count as i64 as u32)
                    & if width == 4 {
                        u32::MAX
                    } else {
                        (1u32 << (width * 8)) - 1
                    };
                if packed & !mask != 0 {
                    return Err(ItemError::UndeclaredBits);
                }
                Ok(items
                    .iter()
                    .map(|bit| InventoryEntry {
                        item: bit.item.clone(),
                        count: f64::from(u8::from(count as i64 as u32 & bit.mask != 0)),
                        capacity: 1.0,
                        count_policy: None,
                    })
                    .collect())
            }
        }
    }

    fn is_current(&mut self, actor: NativeActorId) -> bool {
        let Some(entry) = self.entries.get(&actor) else {
            return false;
        };
        if !self.operations.is_live(actor) || self.operations.is_retired(actor) {
            return false;
        }
        for (record, address) in &entry.addresses {
            if !matches!(self.operations.pointer(actor, record), Ok(base) if base.offset == *address) {
                return false;
            }
        }
        true
    }

    /// Write one canonical inventory entry into source words.
    pub fn write(&mut self, actor: NativeActorId, value: &InventoryEntry) -> Result<(), ItemError> {
        let index = self.by_item.get(&value.item).copied().ok_or(ItemError::LostLease)?;
        let storage = self
            .definition
            .storage
            .get(index)
            .cloned()
            .ok_or(ItemError::LostLease)?;
        if !self.is_current(actor) {
            return Err(ItemError::LostLease);
        }
        self.writing += 1;
        let outcome = self.write_inner(actor, value, &storage);
        self.writing -= 1;
        outcome?;
        self.changed(actor, &[index], Some(value.item.clone()))?;
        Ok(())
    }

    fn write_inner(
        &mut self,
        actor: NativeActorId,
        value: &InventoryEntry,
        storage: &ItemStorage,
    ) -> Result<(), ItemError> {
        let pointer_bytes = self.operations.memory().pointer_bytes();
        match storage {
            ItemStorage::Counter { field, capacity, .. } => {
                if !matches!(capacity, ItemCapacity::Field(_)) && value.capacity != self.capacity(actor, capacity)? {
                    return Err(ItemError::ForeignCapacity);
                }
                validate_counter(value.count, field, pointer_bytes)?;
                if let ItemCapacity::Field(capacity) = capacity {
                    validate_counter(value.capacity, capacity, pointer_bytes)?;
                }
                let address = self.address(actor, field)?;
                let storage_kind = field.storage;
                self.operations.write(address, storage_kind, value.count)?;
                if let ItemCapacity::Field(capacity) = capacity {
                    let address = self.address(actor, capacity)?;
                    let storage_kind = capacity.storage;
                    self.operations.write(address, storage_kind, value.capacity)?;
                }
                Ok(())
            }
            ItemStorage::Packed { field, items, .. } => {
                let bit = items
                    .iter()
                    .find(|bit| bit.item == value.item)
                    .ok_or(ItemError::BadPackedWrite)?;
                if value.capacity != 1.0 || (value.count != 0.0 && value.count != 1.0) {
                    return Err(ItemError::BadPackedWrite);
                }
                let address = self.address(actor, field)?;
                let storage_kind = field.storage;
                let previous = self.operations.read(address, storage_kind)?;
                let next = if value.count == 0.0 {
                    (previous as i64 as u32) & !bit.mask
                } else {
                    (previous as i64 as u32) | bit.mask
                };
                let bits = field.width(pointer_bytes) * 8;
                let stored = if field.storage == GuestStorage::Int32
                    || field.storage == GuestStorage::Int16
                    || field.storage == GuestStorage::Int8
                {
                    ((next << (32 - bits)) as i32 >> (32 - bits)) as f64
                } else {
                    f64::from(next)
                };
                let storage_kind = field.storage;
                self.operations.write(address, storage_kind, stored)?;
                Ok(())
            }
        }
    }

    fn changed(
        &mut self,
        actor: NativeActorId,
        indexes: &[usize],
        canonical_write: Option<ItemId>,
    ) -> Result<(), ItemError> {
        if self.writing != 0 || !self.is_current(actor) {
            return Ok(());
        }
        let mut after = Vec::new();
        for index in indexes {
            if let Some(storage) = self.definition.storage.get(*index).cloned() {
                after.extend(self.read_storage(actor, &storage)?);
            }
        }
        let mut changes = Vec::new();
        for value in &after {
            let before = self
                .entries
                .get(&actor)
                .and_then(|entry| entry.previous.get(&value.item))
                .cloned();
            let Some(before) = before else { continue };
            if Some(&value.item) != canonical_write.as_ref()
                && (before.count != value.count || before.capacity != value.capacity)
            {
                if let Some(cover) = self.operations.pickup_cover() {
                    if !cover.contains(&value.item) {
                        return Err(ItemError::PickupScope);
                    }
                }
                changes.push((before, value.clone()));
            }
        }
        if let Some(entry) = self.entries.get_mut(&actor) {
            for value in &after {
                entry.previous.insert(value.item.clone(), value.clone());
            }
        }
        if !changes.is_empty() {
            self.operations.publish_stores(actor, &changes);
            if !self.is_current(actor) {
                return Err(ItemError::Retired);
            }
        }
        Ok(())
    }

    /// Admit an actor, binding storage observers.
    pub fn admit(&mut self, actor: NativeActorId) -> Result<(), ItemError> {
        if self.entries.contains_key(&actor) {
            return Ok(());
        }
        if !self.operations.is_live(actor) {
            return Err(ItemError::BadAdmission);
        }
        let mut addresses = HashMap::new();
        for storage in self.definition.storage.clone() {
            let fields: Vec<ItemField> = match &storage {
                ItemStorage::Counter { field, capacity, .. } => match capacity {
                    ItemCapacity::Field(extra) => vec![field.clone(), extra.clone()],
                    _ => vec![field.clone()],
                },
                ItemStorage::Packed { field, .. } => vec![field.clone()],
            };
            for field in &fields {
                let base = self.operations.pointer(actor, &field.record)?;
                addresses.insert(field.record.clone(), base.offset);
            }
        }
        let mut previous = HashMap::new();
        for storage in self.definition.storage.clone() {
            for value in self.read_storage(actor, &storage)? {
                previous.insert(value.item.clone(), value);
            }
        }
        let source_selection = match &mut self.stage {
            None => None,
            Some(stage) => stage.pending(actor).or_else(|| stage.active(actor)),
        };
        self.entries.insert(
            actor,
            ItemEntry {
                addresses,
                previous,
                observers: Vec::new(),
                request: None,
                source_selection,
            },
        );
        let pointer_bytes = self.operations.memory().pointer_bytes();
        let dirty = self.dirty.clone();
        let mut observers = Vec::new();
        let mut failed: Option<ItemError> = None;
        for (index, storage) in self.definition.storage.clone().iter().enumerate() {
            let watched: Vec<(GuestAddress, usize)> = match storage {
                ItemStorage::Counter { field, capacity, .. } => {
                    let mut watched = vec![(self.address(actor, field)?, field.width(pointer_bytes))];
                    match capacity {
                        ItemCapacity::Field(extra) => {
                            watched.push((self.address(actor, extra)?, extra.width(pointer_bytes)))
                        }
                        ItemCapacity::Source { address, storage } => {
                            watched.push((self.operations.resolve(address)?, storage.byte_length(pointer_bytes)))
                        }
                        ItemCapacity::Constant(_) => {}
                    }
                    watched
                }
                ItemStorage::Packed { field, .. } => {
                    vec![(self.address(actor, field)?, field.width(pointer_bytes))]
                }
            };
            for (address, width) in watched {
                let dirty = dirty.clone();
                match self.operations.memory().observe_writes(
                    address,
                    width,
                    Box::new(move |_| {
                        dirty.borrow_mut().insert((actor, index));
                    }),
                ) {
                    Ok(id) => observers.push(id),
                    Err(error) => {
                        failed = Some(ItemError::from(error));
                        break;
                    }
                }
            }
            if failed.is_some() {
                break;
            }
        }
        if let Some(error) = failed {
            let memory = self.operations.memory();
            for id in observers {
                memory.unobserve(id);
            }
            self.entries.remove(&actor);
            return Err(error);
        }
        if let Some(entry) = self.entries.get_mut(&actor) {
            entry.observers = observers;
        }
        Ok(())
    }

    /// Poll watched words for an actor, publishing committed stores.
    pub fn poll(&mut self, actor: NativeActorId) -> Result<(), ItemError> {
        let indexes: Vec<usize> = self
            .dirty
            .borrow()
            .iter()
            .filter(|(dirty, _)| *dirty == actor)
            .map(|(_, index)| *index)
            .collect();
        self.dirty.borrow_mut().retain(|(dirty, _)| *dirty != actor);
        if !indexes.is_empty() {
            self.changed(actor, &indexes, None)?;
        }
        // Source selection drift requests through the shared stage; the donor
        // observes selection pointers while this bridge polls them here.
        if self.stage.is_none() || !self.is_current(actor) {
            return Ok(());
        }
        let selection = match &mut self.stage {
            Some(stage) => stage.pending(actor).or_else(|| stage.active(actor)),
            None => None,
        };
        let drifted = self
            .entries
            .get(&actor)
            .is_some_and(|entry| selection.is_some() && selection != entry.source_selection);
        if let Some(entry) = self.entries.get_mut(&actor) {
            entry.source_selection = selection.clone();
        }
        if drifted && !self.requesting.contains(&actor) {
            self.weapon_resume(actor, selection)?;
        }
        Ok(())
    }

    /// Invoke a source item action.
    pub fn invoke_action(&mut self, actor: NativeActorId, item: &ItemId, action: &str) -> Result<(), ItemError> {
        let call = self
            .definition
            .definitions
            .iter()
            .find(|definition| &definition.item == item)
            .and_then(|definition| definition.actions.iter().find(|candidate| candidate.action == action))
            .map(|action| action.call.clone())
            .ok_or(ItemError::ActionGone)?;
        if !self.is_current(actor) {
            return Err(ItemError::ActionGone);
        }
        self.operations.invoke(actor, &call);
        Ok(())
    }

    /// Whether an actor accepts attack input right now.
    pub fn accepts_attack(&mut self, actor: NativeActorId) -> Result<bool, ItemError> {
        let Some(stage) = self.stage.as_mut() else {
            return Ok(true);
        };
        Ok(self.operations.weapon_selected(actor) || stage.continuing(actor))
    }

    /// Whether a cancellation targets a live entry.
    #[must_use]
    pub fn cancellation_current(&mut self, actor: NativeActorId) -> bool {
        self.operations.is_live(actor)
    }

    fn completed(&mut self, actor: NativeActorId) -> Result<(), ItemError> {
        let request = self.entries.get(&actor).and_then(|entry| entry.request.clone());
        let (Some(request), Some(stage)) = (request, self.stage.as_mut()) else {
            return Ok(());
        };
        if request.status != RequestStatus::Pending {
            return Ok(());
        }
        if stage.active(actor) == request.item {
            if let Some(entry) = self.entries.get_mut(&actor) {
                if let Some(request) = entry.request.as_mut() {
                    request.status = RequestStatus::Accepted;
                }
            }
        } else if stage.pending(actor) != request.item {
            if let Some(entry) = self.entries.get_mut(&actor) {
                if let Some(request) = entry.request.as_mut() {
                    request.status = RequestStatus::Refused;
                }
            }
        }
        Ok(())
    }

    /// Whether a weapon binding is current for an actor.
    pub fn weapon_current(&mut self, actor: NativeActorId) -> bool {
        self.stage.is_some() && self.is_current(actor)
    }

    /// Read the weapon presentation for an actor.
    pub fn weapon_read(&mut self, actor: NativeActorId) -> Result<WeaponPresentation, ItemError> {
        if !self.weapon_current(actor) {
            return Err(ItemError::PresentationGone);
        }
        let stage = self.stage.as_mut().ok_or(ItemError::PresentationGone)?;
        Ok(WeaponPresentation {
            active: stage.active(actor),
            pending: stage.pending(actor),
            model: self.operations.weapon_model(actor),
        })
    }

    /// Whether an item may be selected for an actor.
    pub fn weapon_accepts(&mut self, actor: NativeActorId, item: &ItemId) -> Result<bool, ItemError> {
        if !self.is_current(actor) || self.stage.is_none() {
            return Ok(false);
        }
        let Some(values) = self.definition.weapons.as_ref().map(|weapons| &weapons.selection_items) else {
            return Ok(false);
        };
        Ok(values.contains(item))
    }

    /// Select a weapon for an actor.
    pub fn weapon_select(&mut self, actor: NativeActorId, item: &ItemId) -> Result<bool, ItemError> {
        if !self.weapon_accepts(actor, item)? {
            return Ok(false);
        }
        Ok(self.weapon_resume(actor, Some(item.clone()))? != RequestStatus::Refused)
    }

    /// Holster check: holstering is a no-op but requires a live binding.
    pub fn weapon_holster(&mut self, actor: NativeActorId) -> Result<(), ItemError> {
        if !self.is_current(actor) {
            return Err(ItemError::Retired);
        }
        Ok(())
    }

    /// Whether the weapon stage settled for an actor.
    pub fn weapon_is_holstered(&mut self, actor: NativeActorId) -> Result<bool, ItemError> {
        if !self.is_current(actor) {
            return Ok(false);
        }
        let Some(stage) = self.stage.as_mut() else {
            return Ok(false);
        };
        Ok(stage.settled(actor))
    }

    /// Request (or resume) a weapon selection for an actor.
    pub fn weapon_resume(&mut self, actor: NativeActorId, item: Option<ItemId>) -> Result<RequestStatus, ItemError> {
        if !self.is_current(actor) {
            return Err(ItemError::RequestGone);
        }
        if self.stage.is_none() {
            return Err(ItemError::RequestGone);
        }
        self.next_request += 1;
        let id = self.next_request;
        let mut request = WeaponRequest {
            id,
            item: item.clone(),
            status: RequestStatus::Pending,
        };
        if item.is_none() {
            let settled = self.stage.as_mut().is_some_and(|stage| stage.settled(actor));
            let active = self.stage.as_mut().and_then(|stage| stage.active(actor));
            request.status = if active.is_some() && settled {
                RequestStatus::Accepted
            } else {
                RequestStatus::Refused
            };
        } else if let Some(item) = item {
            self.requesting.insert(actor);
            let accepted = self.weapon_accepts(actor, &item)?
                && self.stage.as_mut().is_some_and(|stage| stage.request(actor, &item));
            self.requesting.remove(&actor);
            if !accepted {
                request.status = RequestStatus::Refused;
            } else {
                let active = self.stage.as_mut().and_then(|stage| stage.active(actor));
                if active == Some(item) {
                    request.status = RequestStatus::Accepted;
                }
            }
        }
        let status = request.status;
        if let Some(entry) = self.entries.get_mut(&actor) {
            entry.request = Some(request);
        }
        self.completed(actor)?;
        Ok(self
            .entries
            .get(&actor)
            .and_then(|entry| entry.request.clone())
            .map(|request| request.status)
            .unwrap_or(status))
    }

    /// Restore a saved request handle after ownership checks.
    pub fn restore_request(
        &mut self,
        actor: NativeActorId,
        id: u64,
        item: Option<ItemId>,
    ) -> Result<RequestStatus, ItemError> {
        let request = self.entries.get(&actor).and_then(|entry| entry.request.clone());
        match request {
            Some(request) if self.is_current(actor) && request.id == id && request.item == item => Ok(request.status),
            _ => Err(ItemError::SavedRequestMismatch),
        }
    }

    /// Cancel an open request.
    pub fn cancel_request(&mut self, actor: NativeActorId) {
        if let Some(entry) = self.entries.get_mut(&actor) {
            if let Some(request) = entry.request.as_mut() {
                if request.status == RequestStatus::Pending {
                    request.status = RequestStatus::Refused;
                }
            }
            entry.request = None;
        }
    }

    /// Capture open requests.
    #[must_use]
    pub fn checkpoint(&self) -> NativeItemCheckpoint {
        NativeItemCheckpoint {
            version: 1,
            next_request: self.next_request,
            requests: self
                .entries
                .iter()
                .filter_map(|(actor, entry)| {
                    entry.request.clone().map(|request| SavedItemRequest {
                        actor: SavedActorId::from(*actor),
                        id: request.id,
                        item: request.item,
                        status: request.status,
                    })
                })
                .collect(),
        }
    }

    /// Restore open requests after ownership checks.
    pub fn restore(
        &mut self,
        checkpoint: &NativeItemCheckpoint,
        resolve: &dyn Fn(SavedActorId) -> NativeActorId,
    ) -> Result<(), ItemError> {
        if checkpoint.version != 1 {
            return Err(ItemError::SavedDeclarationMismatch);
        }
        for row in &checkpoint.requests {
            if row.id == 0 || row.id > checkpoint.next_request {
                return Err(ItemError::SavedDeclarationMismatch);
            }
            if let Some(item) = &row.item {
                let allowed = self
                    .definition
                    .weapons
                    .as_ref()
                    .is_some_and(|weapons| weapons.selection_items.contains(item));
                if self.definition.weapons.is_none() || !allowed {
                    return Err(ItemError::SavedDeclarationMismatch);
                }
            }
        }
        self.next_request = checkpoint.next_request;
        for row in &checkpoint.requests {
            let actor = resolve(row.actor);
            let Some(entry) = self.entries.get_mut(&actor) else {
                return Err(ItemError::SavedOwnerGone);
            };
            if !self.operations.is_live(actor) {
                return Err(ItemError::SavedOwnerGone);
            }
            entry.request = Some(WeaponRequest {
                id: row.id,
                item: row.item.clone(),
                status: row.status,
            });
        }
        Ok(())
    }

    /// Release an actor's bindings and observers.
    pub fn release(&mut self, actor: NativeActorId) {
        if let Some(entry) = self.entries.remove(&actor) {
            let memory = self.operations.memory();
            for id in entry.observers.iter().rev() {
                memory.unobserve(*id);
            }
            self.dirty.borrow_mut().retain(|(dirty, _)| *dirty != actor);
        }
    }

    /// Release every actor.
    pub fn clear(&mut self) {
        for actor in self.entries.keys().copied().collect::<Vec<_>>() {
            self.release(actor);
        }
    }
}

/// Memory-backed item operations over synthetic record rows.
pub struct SyntheticItemOperations {
    /// Guest memory.
    pub memory: SparseGuestMemory,
    image_base: GuestAddress,
    record_bases: HashMap<(String, u32), GuestAddress>,
    exports: HashMap<String, GuestAddress>,
    live: HashSet<NativeActorId>,
    retired: HashSet<NativeActorId>,
    selected: HashSet<NativeActorId>,
    cover: Option<Vec<ItemId>>,
    /// Published stores.
    pub published: Vec<(NativeActorId, Vec<(InventoryEntry, InventoryEntry)>)>,
    /// Invoked source calls.
    pub invokes: Vec<(NativeActorId, String)>,
}

impl SyntheticItemOperations {
    /// Build operations over guest memory.
    pub fn new(memory: SparseGuestMemory, image_base: GuestAddress) -> Self {
        Self {
            memory,
            image_base,
            record_bases: HashMap::new(),
            exports: HashMap::new(),
            live: HashSet::new(),
            retired: HashSet::new(),
            selected: HashSet::new(),
            cover: None,
            published: Vec::new(),
            invokes: Vec::new(),
        }
    }

    /// Bind a record base for an actor slot.
    pub fn bind_record(&mut self, actor: NativeActorId, record: &str, base: GuestAddress) {
        self.record_bases.insert((record.to_string(), actor.slot), base);
    }

    /// Register a named export.
    pub fn register_export(&mut self, name: &str, address: GuestAddress) {
        self.exports.insert(name.to_string(), address);
    }

    /// Mark an actor live.
    pub fn set_live(&mut self, actor: NativeActorId, live: bool) {
        if live {
            self.live.insert(actor);
        } else {
            self.live.remove(&actor);
        }
    }

    /// Mark a lease retired.
    pub fn set_retired(&mut self, actor: NativeActorId, retired: bool) {
        if retired {
            self.retired.insert(actor);
        } else {
            self.retired.remove(&actor);
        }
    }

    /// Mark weapon selection.
    pub fn set_selected(&mut self, actor: NativeActorId, selected: bool) {
        if selected {
            self.selected.insert(actor);
        } else {
            self.selected.remove(&actor);
        }
    }

    /// Set the active pickup cover.
    pub fn set_pickup_cover(&mut self, cover: Option<Vec<ItemId>>) {
        self.cover = cover;
    }
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

fn f64_to_scalar(value: f64, storage: GuestStorage) -> Vec<u8> {
    match storage {
        GuestStorage::Float32 => (value as f32).to_le_bytes().to_vec(),
        GuestStorage::Float64 => value.to_le_bytes().to_vec(),
        GuestStorage::Int8 | GuestStorage::Uint8 => vec![value as i64 as u8],
        GuestStorage::Int16 | GuestStorage::Uint16 => (value as i64 as i16).to_le_bytes().to_vec(),
        GuestStorage::Int32 | GuestStorage::Uint32 | GuestStorage::Pointer => {
            (value as i64 as i32).to_le_bytes().to_vec()
        }
        GuestStorage::Int64 | GuestStorage::Uint64 => (value as i64).to_le_bytes().to_vec(),
    }
}

impl ItemOperations for SyntheticItemOperations {
    fn memory(&mut self) -> &mut SparseGuestMemory {
        &mut self.memory
    }

    fn pointer(&mut self, actor: NativeActorId, record: &str) -> Result<GuestAddress, ItemError> {
        self.record_bases
            .get(&(record.to_string(), actor.slot))
            .copied()
            .ok_or(ItemError::LostLease)
    }

    fn resolve(&self, address: &AddressRef) -> Result<GuestAddress, ItemError> {
        match address {
            AddressRef::Export(name) => self.exports.get(name).copied().ok_or(ItemError::LostLease),
            AddressRef::Rva(rva) => Ok(self
                .memory
                .offset(self.image_base, i64::try_from(*rva).unwrap_or(i64::MAX))?),
        }
    }

    fn is_live(&self, actor: NativeActorId) -> bool {
        self.live.contains(&actor)
    }

    fn read(&mut self, address: GuestAddress, storage: GuestStorage) -> Result<f64, ItemError> {
        let width = storage.byte_length(self.memory.pointer_bytes());
        Ok(scalar_to_f64(&self.memory.copy(address, width)?, storage))
    }

    fn write(&mut self, address: GuestAddress, storage: GuestStorage, value: f64) -> Result<(), ItemError> {
        Ok(self.memory.write(address, &f64_to_scalar(value, storage))?)
    }

    fn invoke(&mut self, actor: NativeActorId, call: &ItemCall) {
        self.invokes.push((actor, call.id.clone()));
    }

    fn pickup_cover(&self) -> Option<Vec<ItemId>> {
        self.cover.clone()
    }

    fn publish_stores(&mut self, actor: NativeActorId, changes: &[(InventoryEntry, InventoryEntry)]) {
        self.published.push((actor, changes.to_vec()));
    }

    fn is_retired(&self, actor: NativeActorId) -> bool {
        self.retired.contains(&actor)
    }

    fn weapon_selected(&self, actor: NativeActorId) -> bool {
        self.selected.contains(&actor)
    }

    fn weapon_model(&self, actor: NativeActorId) -> u32 {
        u32::from(actor.slot)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_core::identity::ProviderId;
    use qa_guest::core::contracts::{ContentDigest, GuestMapOptions, GuestPermissions, ModuleIdentity};
    use qa_world::combat::item_id;

    fn actor(slot: u32) -> NativeActorId {
        NativeActorId { slot, generation: 1 }
    }

    fn counter(item: &str, offset: usize) -> ItemStorage {
        ItemStorage::Counter {
            item: item_id("q2", item),
            field: ItemField {
                record: "client".to_string(),
                offset,
                storage: GuestStorage::Int32,
            },
            capacity: ItemCapacity::Constant(100.0),
        }
    }

    fn definition() -> ItemsDefinition {
        ItemsDefinition {
            definitions: vec![
                ItemDefinition {
                    item: item_id("q2", "shells"),
                    kind: ItemKind::Other,
                    ammo: None,
                    actions: vec![ItemAction {
                        action: "use".to_string(),
                        call: ItemCall {
                            id: "use-shells".to_string(),
                        },
                    }],
                },
                ItemDefinition {
                    item: item_id("q2", "blaster"),
                    kind: ItemKind::Weapon,
                    ammo: Some(item_id("q2", "shells")),
                    actions: Vec::new(),
                },
            ],
            storage: vec![
                counter("shells", 0),
                ItemStorage::Packed {
                    field: ItemField {
                        record: "client".to_string(),
                        offset: 8,
                        storage: GuestStorage::Int32,
                    },
                    items: vec![PackedBit {
                        item: item_id("q2", "blaster"),
                        mask: 0x1,
                    }],
                    private_mask: 0,
                },
            ],
            weapons: Some(WeaponStageRef {
                dispatcher_arguments: 2,
                dispatcher_argument: 1,
                decision_regions: 1,
                settled_groups: 1,
                continuation_groups: 1,
                selection_items: vec![item_id("q2", "blaster")],
            }),
        }
    }

    struct TestStage {
        active: HashMap<NativeActorId, Option<ItemId>>,
        pending: HashMap<NativeActorId, Option<ItemId>>,
        settled: HashSet<NativeActorId>,
        continuing: HashSet<NativeActorId>,
    }

    impl ItemWeaponStage for TestStage {
        fn pending(&mut self, actor: NativeActorId) -> Option<ItemId> {
            self.pending.get(&actor).cloned().unwrap_or(None)
        }

        fn active(&mut self, actor: NativeActorId) -> Option<ItemId> {
            self.active.get(&actor).cloned().unwrap_or(None)
        }

        fn settled(&mut self, actor: NativeActorId) -> bool {
            self.settled.contains(&actor)
        }

        fn continuing(&mut self, actor: NativeActorId) -> bool {
            self.continuing.contains(&actor)
        }

        fn request(&mut self, actor: NativeActorId, item: &ItemId) -> bool {
            self.pending.insert(actor, Some(item.clone()));
            true
        }
    }

    fn fixture() -> NativeModItems<SyntheticItemOperations, TestStage> {
        let module = ModuleIdentity::new(
            ProviderId::new("test", "items"),
            "items.so",
            ContentDigest {
                algorithm: "none".to_string(),
                value: "0".to_string(),
            },
            "r1",
        );
        let mut memory = SparseGuestMemory::new(module, 4, 0xD0000).unwrap();
        let image_base = memory
            .map(&GuestMapOptions::new(0x0, 0x2000, GuestPermissions::ReadWrite))
            .unwrap();
        let row = memory
            .map(&GuestMapOptions::new(0x30000, 64, GuestPermissions::ReadWrite))
            .unwrap();
        let mut operations = SyntheticItemOperations::new(memory, image_base);
        operations.bind_record(actor(1), "client", row);
        operations.set_live(actor(1), true);
        let stage = TestStage {
            active: HashMap::new(),
            pending: HashMap::new(),
            settled: HashSet::new(),
            continuing: HashSet::new(),
        };
        NativeModItems::new(definition(), operations, Some(stage))
    }

    #[test]
    fn counter_write_and_external_store_publish() {
        let mut bridge = fixture();
        bridge.admit(actor(1)).unwrap();
        let entry = InventoryEntry {
            item: item_id("q2", "shells"),
            count: 25.0,
            capacity: 100.0,
            count_policy: None,
        };
        bridge.write(actor(1), &entry).unwrap();
        assert!(bridge.operations().published.is_empty());
        // External source write publishes on poll.
        let base = bridge.operations_mut().pointer(actor(1), "client").unwrap();
        let address = bridge.operations_mut().memory().offset(base, 0).unwrap();
        bridge
            .operations_mut()
            .memory()
            .write(address, &30i32.to_le_bytes())
            .unwrap();
        bridge.poll(actor(1)).unwrap();
        assert_eq!(bridge.operations().published.len(), 1);
        let (published_actor, changes) = &bridge.operations().published[0];
        assert_eq!(*published_actor, actor(1));
        assert_eq!(changes[0].0.count, 25.0);
        assert_eq!(changes[0].1.count, 30.0);
        bridge.release(actor(1));
    }

    #[test]
    fn packed_bits_reject_undeclared_storage() {
        let mut bridge = fixture();
        bridge.admit(actor(1)).unwrap();
        let blaster = InventoryEntry {
            item: item_id("q2", "blaster"),
            count: 1.0,
            capacity: 1.0,
            count_policy: None,
        };
        bridge.write(actor(1), &blaster).unwrap();
        let base = bridge.operations_mut().pointer(actor(1), "client").unwrap();
        let address = bridge.operations_mut().memory().offset(base, 8).unwrap();
        let stored = bridge.operations_mut().memory().copy(address, 4).unwrap();
        assert_eq!(stored, 1i32.to_le_bytes());
        // Undeclared bit set behind the bridge.
        bridge
            .operations_mut()
            .memory()
            .write(address, &0x8000_0000u32.to_le_bytes())
            .unwrap();
        let poll = bridge.poll(actor(1));
        assert_eq!(poll, Err(ItemError::UndeclaredBits));
    }

    #[test]
    fn validation_weapon_requests_and_checkpoint() {
        let mut overlapping = definition();
        overlapping.storage.push(counter("shells", 2));
        assert_eq!(
            validate_native_mod_items(
                &overlapping,
                &[("client".to_string(), 64)],
                &["client".to_string()],
                true,
                4,
            ),
            Err(ItemError::OverlappingStorage)
        );
        let mut bridge = fixture();
        validate_native_mod_items(
            &definition(),
            &[("client".to_string(), 64)],
            &["client".to_string()],
            true,
            4,
        )
        .unwrap();
        bridge.admit(actor(1)).unwrap();
        assert!(!bridge.accepts_attack(actor(1)).unwrap());
        bridge.operations_mut().set_selected(actor(1), true);
        assert!(bridge.accepts_attack(actor(1)).unwrap());
        let status = bridge.weapon_resume(actor(1), Some(item_id("q2", "blaster"))).unwrap();
        assert_eq!(status, RequestStatus::Pending);
        let checkpoint = bridge.checkpoint();
        assert_eq!(checkpoint.requests.len(), 1);
        bridge
            .restore(&checkpoint, &|saved| NativeActorId {
                slot: saved.slot,
                generation: saved.generation,
            })
            .unwrap();
        let restored = bridge
            .restore_request(actor(1), checkpoint.requests[0].id, Some(item_id("q2", "blaster")))
            .unwrap();
        assert_eq!(restored, RequestStatus::Pending);
        bridge.cancel_request(actor(1));
        let holstered = bridge.weapon_is_holstered(actor(1)).unwrap();
        assert!(!holstered);
        bridge.invoke_action(actor(1), &item_id("q2", "shells"), "use").unwrap();
        assert_eq!(bridge.operations().invokes, vec![(actor(1), "use-shells".to_string())]);
        bridge.clear();
    }
}
