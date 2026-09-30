//! QVM mod items: canonical inventory over original source words.
//!
//! Ports `src/compat/qvm/mod-items.ts`. Declarations, storage, and the weapon
//! stage *declaration* reuse [`super::item_storage`]; the weapon *stage class*
//! and `validateQvmWeaponStage` are helper-owned (`super::mod_weapon_stage`):
//! this file defines the driver/host seam ([`QvmWeaponStageDriver`]) and takes
//! the stage validator as a parameter. Pickup write shapes reuse
//! [`super::mod_pickups`]; applications reuse [`super::mod_input`]. Source
//! item/lease/binding shapes mirror `src/contracts/source-items.ts`; the save
//! reader mirrors `src/persistence/value.ts`.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use qa_core::identity::{ActorId, ClientId, OwnedActor, ProviderId};
use qa_core::math::Bounds;
use qa_world::combat::ItemId;
use qa_world::inventory::InventoryEntry;

use super::game_data::{
    QvmCommittedWrite, QvmImage, QvmModule, QvmOpcode, QvmSharedMemory, QvmWriteRange, QVM_MAX_PRIVATE_ARGUMENT_WORDS,
};
use super::item_storage::{
    read_qvm_item_storage, validate_qvm_item_storage, write_qvm_item_storage, QvmItemActions, QvmItemCapacity,
    QvmItemField, QvmItemFieldUsage, QvmItemStorage, QvmItemStorageAccess, QvmModItemKind,
    QvmModItems as QvmModItemsDeclaration, QvmWeaponStage,
};
use super::mod_actors::{
    QvmInputPhase, QvmModActorRecord, QvmModFieldBinding, QvmModInputPointer, QvmModInputPointerBase, QvmModSourceCall,
};
use super::mod_input::ModClientApplication;
use super::mod_pickups::{QvmPickupWriteDecl, QvmPickupWriteFields};
use crate::error::GuestError;

/// Client input binding subset for weapon input checks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QvmModItemsInputBinding {
    /// Application scope.
    pub scope: String,
    /// Binding phase.
    pub phase: QvmInputPhase,
    /// Bound call entries.
    pub calls: Vec<usize>,
}

/// Client declaration subset for item validation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QvmModItemsClients {
    /// Client record ids.
    pub records: Vec<String>,
    /// Input bindings.
    pub input: Vec<QvmModItemsInputBinding>,
}

/// Validation context: the declaration subset item checks need.
#[derive(Debug, Clone, PartialEq)]
pub struct QvmModItemsContext {
    /// Actor records.
    pub actor_records: Vec<QvmModActorRecord>,
    /// Client declaration.
    pub clients: Option<QvmModItemsClients>,
}

/// Field usage for occupancy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FieldUsage {
    /// Counter/bits word.
    Storage,
    /// Capacity word.
    Capacity,
    /// Unowned view.
    View,
}

/// Storage fields including field capacity.
fn storage_fields(storage: &QvmItemStorage) -> Vec<QvmItemField> {
    match storage {
        QvmItemStorage::Counter { field, capacity, .. } => match capacity {
            QvmItemCapacity::Field { field: capacity } => vec![field.clone(), capacity.clone()],
            _ => vec![field.clone()],
        },
        QvmItemStorage::Bits { field, .. } => vec![field.clone()],
    }
}

/// Field storage width.
fn field_width(binding: &QvmModFieldBinding) -> usize {
    match binding {
        QvmModFieldBinding::Private { byte_length } => *byte_length,
        QvmModFieldBinding::Origin
        | QvmModFieldBinding::Velocity
        | QvmModFieldBinding::Angles
        | QvmModFieldBinding::BoundsMin
        | QvmModFieldBinding::BoundsMax
        | QvmModFieldBinding::ConstantVector(_) => 12,
        _ => 4,
    }
}

/// Whether a value fits a signed 32-bit word.
fn is_integer(value: i64) -> bool {
    value >= i64::from(i32::MIN) && value <= i64::from(i32::MAX)
}

/// Validate item declarations against their context.
pub fn validate_qvm_mod_items(
    items: &QvmModItemsDeclaration,
    context: &QvmModItemsContext,
    image: &QvmImage,
    validate_stage: &dyn Fn(&QvmWeaponStage, &QvmImage) -> Result<(), GuestError>,
) -> Result<(), GuestError> {
    let Some(clients) = context.clients.as_ref() else {
        return Err(GuestError::invalid("QVM items require source client admission"));
    };
    if items.definitions.is_empty() {
        return Err(GuestError::invalid("QVM items require source client admission"));
    }
    let definitions: HashSet<&ItemId> = items.definitions.iter().map(|value| &value.item).collect();
    if definitions.len() != items.definitions.len() {
        return Err(GuestError::invalid("Duplicate QVM source item definition"));
    }
    let occupied = Rc::new(RefCell::new(HashMap::new()));
    let field = |source: &QvmItemField, usage: FieldUsage| -> Result<(), GuestError> {
        let record = context.actor_records.iter().find(|value| value.id == source.record);
        let previous = occupied
            .borrow()
            .get(&format!("{}:{}", source.record, source.offset))
            .cloned();
        let Some(record) = record else {
            return Err(GuestError::invalid("Invalid QVM item source field"));
        };
        if !clients.records.contains(&record.id)
            || !source.offset.is_multiple_of(4)
            || source.offset + 4 > record.stride
            || usage != FieldUsage::View
                && previous.is_some()
                && !(usage == FieldUsage::Capacity && previous == Some(FieldUsage::Capacity))
        {
            return Err(GuestError::invalid("Invalid QVM item source field"));
        }
        if usage != FieldUsage::View {
            occupied
                .borrow_mut()
                .insert(format!("{}:{}", source.record, source.offset), usage);
        }
        for value in &record.fields {
            let length = field_width(&value.binding);
            let shared = !matches!(
                value.binding,
                QvmModFieldBinding::Private { .. } | QvmModFieldBinding::Constant { .. }
            );
            if value.offset < source.offset + 4 && source.offset < value.offset + length && shared {
                return Err(GuestError::invalid(
                    "QVM item field overlaps another canonical source projection",
                ));
            }
        }
        Ok(())
    };
    let owned: HashSet<ItemId> = definitions.into_iter().cloned().collect();
    validate_qvm_item_storage(&items.storage, &owned, image, &|source, usage| {
        field(
            source,
            match usage {
                QvmItemFieldUsage::Storage => FieldUsage::Storage,
                QvmItemFieldUsage::Capacity => FieldUsage::Capacity,
            },
        )
    })?;
    let weapons: Vec<_> = items
        .definitions
        .iter()
        .filter(|value| matches!(value.kind, QvmModItemKind::Weapon { .. }))
        .collect();
    if (weapons.is_empty()) != items.weapons.is_none() {
        return Err(GuestError::invalid(
            "QVM weapon items require an original source consumer",
        ));
    }
    let Some(bundle) = items.weapons.as_ref() else {
        return Ok(());
    };
    let stage = &bundle.stage;
    validate_stage(stage, image)?;
    let pointer = |source: &QvmModInputPointer| -> Result<(), GuestError> {
        match source.base {
            QvmModInputPointerBase::Argument { index } => {
                if index >= QVM_MAX_PRIVATE_ARGUMENT_WORDS {
                    return Err(GuestError::invalid("QVM weapon pointer exceeds its original call ABI"));
                }
            }
            QvmModInputPointerBase::Global { address } => {
                if address % 4 != 0 || address + 4 > image.initialized_data.len() + image.bss_length {
                    return Err(GuestError::invalid("QVM weapon pointer exceeds its original call ABI"));
                }
            }
        }
        if source
            .indirections
            .iter()
            .chain([source.offset].iter())
            .any(|value| value % 4 != 0)
        {
            return Err(GuestError::invalid("QVM weapon pointer is not aligned source storage"));
        }
        Ok(())
    };
    for actor in [&stage.dispatcher.actor, &stage.continuation.actor] {
        if !clients.records.contains(&actor.record) {
            return Err(GuestError::invalid(
                "QVM weapon pointer is not an admitted client record",
            ));
        }
        pointer(&actor.pointer)?;
    }
    let projection = &stage.continuation.projection;
    pointer(&projection.movement)?;
    if projection.byte_length < 12 || projection.byte_length % 4 != 0 {
        return Err(GuestError::invalid(
            "QVM weapon movement projection lacks its caller layout",
        ));
    }
    for offset in [projection.minimum, projection.maximum] {
        if offset < 0 || offset % 4 != 0 || offset + 12 > projection.byte_length as i64 {
            return Err(GuestError::invalid(
                "QVM weapon bounds exceed their original caller storage",
            ));
        }
    }
    if (projection.minimum - projection.maximum).abs() < 12 {
        return Err(GuestError::invalid("QVM weapon caller bounds overlap"));
    }
    let entry = bundle.input_entry;
    let inputs: Vec<_> = clients
        .input
        .iter()
        .filter(|binding| binding.calls.contains(&entry))
        .collect();
    if image
        .instruction(entry)
        .is_none_or(|instruction| instruction.opcode != QvmOpcode::OpEnter)
        || inputs.len() != 1
        || inputs[0].scope != "movement-slice"
        || inputs[0].phase != QvmInputPhase::After
    {
        return Err(GuestError::invalid(
            "QVM weapon input requires one declared original callback after authoritative movement",
        ));
    }
    field(&stage.selection.field, FieldUsage::View)?;
    field(&bundle.input_clock, FieldUsage::View)?;
    field(&projection.view_height, FieldUsage::View)?;
    field(&projection.ground, FieldUsage::View)?;
    for value in stage
        .settled
        .iter()
        .chain(stage.request.accepted.iter())
        .chain(stage.continuation.when.iter())
    {
        field(&value.field, FieldUsage::View)?;
        if !is_integer(value.value) || value.mask.is_some_and(|mask| mask > 0x7fff_ffff) {
            return Err(GuestError::invalid("Invalid QVM weapon source state predicate"));
        }
    }
    let values = &stage.selection.values;
    let unique_items: HashSet<&ItemId> = values.iter().map(|value| &value.item).collect();
    let unique_values: HashSet<i64> = values.iter().map(|value| value.value).collect();
    if values.len() != weapons.len()
        || unique_items.len() != weapons.len()
        || unique_values.len() != weapons.len()
        || values.iter().any(|value| {
            !is_integer(value.value) || value.value < 1 || !weapons.iter().any(|weapon| weapon.item == value.item)
        })
    {
        return Err(GuestError::invalid(
            "QVM weapon selection differs from its admitted definitions",
        ));
    }
    for weapon in weapons {
        if let QvmModItemKind::Weapon { ammo: Some(ammo), .. } = &weapon.kind {
            if !owned.contains(ammo) {
                return Err(GuestError::invalid("QVM weapon ammo lacks its source storage"));
            }
        }
    }
    Ok(())
}

/// Resolved item icon (mirror of `WeaponHudIcon`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QvmResolvedItemIcon {
    /// Image resource.
    Image {
        /// Owning content.
        content: String,
        /// Image path.
        path: String,
    },
    /// WAD picture resource.
    WadPicture {
        /// Owning content.
        content: String,
        /// WAD path.
        path: String,
        /// Lump name.
        lump: String,
    },
    /// Shader name.
    Shader {
        /// Owning content.
        content: String,
        /// Shader name.
        name: String,
    },
}

/// Resolve an icon declaration against its content.
fn resolve_item_icon(icon: &super::item_storage::QvmItemIcon, content: &str) -> QvmResolvedItemIcon {
    match icon {
        super::item_storage::QvmItemIcon::Image { path } => QvmResolvedItemIcon::Image {
            content: content.to_string(),
            path: path.clone(),
        },
        super::item_storage::QvmItemIcon::WadPicture { path, lump } => QvmResolvedItemIcon::WadPicture {
            content: content.to_string(),
            path: path.clone(),
            lump: lump.clone(),
        },
        super::item_storage::QvmItemIcon::Shader { name } => QvmResolvedItemIcon::Shader {
            content: content.to_string(),
            name: name.clone(),
        },
    }
}

/// Item definition source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QvmItemSource {
    /// Owning provider.
    pub provider: ProviderId,
    /// Owning content.
    pub content: String,
}

/// Source item definition.
#[derive(Debug, Clone, PartialEq)]
pub struct QvmSourceItemDefinition {
    /// Item id.
    pub item: ItemId,
    /// Display label.
    pub label: String,
    /// Item kind.
    pub kind: QvmModItemKind,
    /// Source.
    pub source: QvmItemSource,
    /// Icon, if any.
    pub icon: Option<QvmResolvedItemIcon>,
    /// Action names, if any.
    pub actions: Option<Vec<String>>,
}

/// Source item admission.
#[derive(Debug, Clone, PartialEq)]
pub struct QvmSourceItemAdmission {
    /// Admission.
    pub admission: super::item_storage::QvmItemAdmission,
    /// Definition.
    pub definition: QvmSourceItemDefinition,
}

/// Stored inventory change.
#[derive(Debug, Clone, PartialEq)]
pub struct QvmSourceItemStore {
    /// Entry before.
    pub before: InventoryEntry,
    /// Entry after.
    pub after: InventoryEntry,
}

/// Item store failure.
#[derive(Debug, Clone, PartialEq)]
pub enum QvmItemStoreError {
    /// Lease retired.
    Retired,
    /// Other failure.
    Other(GuestError),
}

/// Publishes stored item changes.
pub type QvmSourceItemStorePublish =
    Rc<dyn Fn(&[QvmSourceItemStore]) -> Result<(), QvmItemStoreError>>;
/// Reads one inventory entry.
pub type QvmInventoryEntryRead = Rc<dyn Fn(&str) -> Result<Option<InventoryEntry>, GuestError>>;
/// Invokes an item action.
pub type QvmItemActionInvoke = Rc<dyn Fn(&str, &str) -> Result<(), GuestError>>;

/// Source item lease.
pub struct QvmSourceItemLease {
    /// Currency check.
    pub current: Rc<dyn Fn() -> bool>,
    /// Publish stored changes.
    pub stored: QvmSourceItemStorePublish,
    /// Close the lease.
    pub close: Box<dyn FnOnce()>,
}

/// Live inventory state access.
#[derive(Clone)]
pub struct QvmItemStateAccess {
    /// Read all entries.
    pub read: Rc<dyn Fn() -> Result<Vec<InventoryEntry>, GuestError>>,
    /// Read one entry.
    pub entry: QvmInventoryEntryRead,
    /// Write one entry.
    pub write: Rc<dyn Fn(InventoryEntry) -> Result<(), GuestError>>,
    /// Whether capacity is field-backed.
    pub mutable_capacity: Rc<dyn Fn(&str) -> bool>,
}

/// Item admission binding.
#[derive(Clone)]
pub struct QvmItemsAdmission {
    /// Owning provider.
    pub owner: ProviderId,
    /// Items.
    pub items: Vec<QvmSourceItemAdmission>,
    /// Invoke an item action.
    pub invoke: QvmItemActionInvoke,
    /// Live state.
    pub state: QvmItemStateAccess,
}

/// Weapon request status.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QvmWeaponRequestStatus {
    /// Pending.
    Pending,
    /// Accepted.
    Accepted,
    /// Refused.
    Refused,
}

impl QvmWeaponRequestStatus {
    /// Parse a status name.
    fn parse(name: &str) -> Result<Self, GuestError> {
        match name {
            "pending" => Ok(Self::Pending),
            "accepted" => Ok(Self::Accepted),
            "refused" => Ok(Self::Refused),
            _ => Err(GuestError::invalid("Unknown QVM weapon request status")),
        }
    }
}

/// Weapon request handle.
#[derive(Clone)]
pub struct QvmWeaponRequestHandle {
    /// Request id.
    pub id: u64,
    /// Read status.
    pub status: Rc<dyn Fn() -> QvmWeaponRequestStatus>,
    /// Cancel the request.
    pub cancel: Rc<dyn Fn()>,
}

/// Weapon presentation.
#[derive(Debug, Clone, PartialEq)]
pub struct QvmWeaponPresentation {
    /// Source.
    pub source: QvmItemSource,
    /// Active item.
    pub active: Option<ItemId>,
    /// Pending item.
    pub pending: Option<ItemId>,
    /// Items.
    pub items: Vec<QvmSourceItemDefinition>,
}

/// Weapon handoff.
#[derive(Clone)]
pub struct QvmWeaponHandoff {
    /// Handoff kind (`source-input`).
    pub kind: String,
    /// Owning provider.
    pub provider: ProviderId,
    /// Whether an item is acceptable.
    pub accepts: Rc<dyn Fn(&str) -> bool>,
    /// Select an item.
    pub select: Rc<dyn Fn(&str) -> bool>,
    /// Holster.
    pub holster: Rc<dyn Fn() -> Result<(), GuestError>>,
    /// Whether holstered.
    pub is_holstered: Rc<dyn Fn() -> bool>,
    /// Resume a request.
    pub resume: Rc<dyn Fn(Option<ItemId>) -> Result<QvmWeaponRequestHandle, GuestError>>,
    /// Restore a saved request.
    pub restore_request: Rc<dyn Fn(u64, Option<ItemId>) -> Result<QvmWeaponRequestHandle, GuestError>>,
}

/// Source weapon binding.
#[derive(Clone)]
pub struct QvmSourceWeaponBinding {
    /// Currency check.
    pub current: Rc<dyn Fn() -> bool>,
    /// Read presentation.
    pub read: Rc<dyn Fn() -> Result<QvmWeaponPresentation, GuestError>>,
    /// Handoff.
    pub handoff: QvmWeaponHandoff,
}

/// Authoritative posture for weapon input.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct QvmWeaponPosture {
    /// Body bounds.
    pub bounds: Bounds,
    /// View height.
    pub view_height: f32,
    /// Ground level.
    pub ground: f32,
}

/// Player view for weapon input.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct QvmPlayerView {
    /// View offset.
    pub view_offset: qa_core::math::Vec3,
}

/// Weapon stage driver seam (helper-owned `QvmModWeaponStage` implements this).
pub trait QvmWeaponStageDriver {
    /// Whether the actor's weapon is settled.
    fn settled(&self, actor: &ActorId) -> bool;
    /// Active weapon item.
    fn active(&self, actor: &ActorId) -> Option<ItemId>;
    /// Apply around a movement slice.
    fn apply(&self, actor: &ActorId, run: &dyn Fn() -> i32) -> i32;
    /// Open an application; returns its closer.
    fn open(&self, application: &ModClientApplication) -> Box<dyn FnOnce()>;
    /// Cancel retired state.
    fn cancel_retired(&self, actor: &ActorId);
    /// Close the stage.
    fn close(&self);
}

/// Host services for mod items.
pub trait QvmModItemsServices {
    /// Resolve an owned actor.
    fn resolve_owned(&self, actor: &ActorId) -> Option<OwnedActor>;
    /// Bind inventory items.
    fn bind_items(&self, owner: &OwnedActor, admission: QvmItemsAdmission) -> QvmSourceItemLease;
    /// Count an item.
    fn inventory_count(&self, actor: &ActorId, item: &str) -> i32;
    /// Read body bounds.
    fn body_bounds(&self, actor: &ActorId) -> Option<Bounds>;
    /// Destination client for an actor.
    fn clients_for_actor(&self, actor: &ActorId) -> Option<ClientId>;
    /// Read a player view.
    fn player_view(&self, client: &ClientId) -> Option<QvmPlayerView>;
    /// Whether the provider weapon is selected.
    fn weapon_selected(&self, actor: &ActorId) -> bool;
    /// Bind a weapon; `None` means the service is unavailable.
    fn bind_weapon(&self, owner: &OwnedActor, binding: QvmSourceWeaponBinding) -> Option<Box<dyn FnOnce()>>;
    /// Resolve a saved actor.
    fn reference_saved(&self, saved: &QvmSavedModActor) -> Option<ActorId>;
}

/// Provider operations for mod items.
pub trait QvmModItemsOperations {
    /// Source pointer for an actor record.
    fn pointer(&self, actor: &ActorId, record: &str) -> Result<usize, GuestError>;
    /// Whether an actor is live.
    fn live(&self, actor: &ActorId) -> bool;
    /// Invoke a source call.
    fn invoke(&self, actor: &ActorId, call: &QvmModSourceCall) -> Result<i32, GuestError>;
    /// Ground level.
    fn ground(&self, actor: &ActorId) -> Result<f32, GuestError>;
    /// Current pickup writes, if any.
    fn pickup(&self) -> Option<Vec<QvmPickupWriteDecl>>;
}

/// Saved mod actor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QvmSavedModActor {
    /// Registry slot.
    pub slot: u32,
    /// Slot generation.
    pub generation: u32,
}

/// Saved weapon request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QvmSavedWeaponRequest {
    /// Actor.
    pub actor: QvmSavedModActor,
    /// Request id.
    pub id: u64,
    /// Item, if any.
    pub item: Option<ItemId>,
    /// Status.
    pub status: QvmWeaponRequestStatus,
}

/// Items checkpoint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QvmItemsCheckpoint {
    /// Version (always 1).
    pub version: u32,
    /// Next request id.
    pub next_request: u64,
    /// Saved requests.
    pub requests: Vec<QvmSavedWeaponRequest>,
}

/// Save value mirror.
#[derive(Debug, Clone, PartialEq)]
pub enum QvmSaveValue {
    /// Integer.
    Int(i64),
    /// Text.
    Text(String),
    /// List.
    List(Vec<QvmSaveValue>),
    /// Record.
    Record(HashMap<String, QvmSaveValue>),
    /// Null.
    Null,
}

/// Save reader mirror.
#[derive(Debug, Clone, Copy)]
pub struct QvmSaveReader<'a> {
    /// Value.
    value: &'a QvmSaveValue,
    /// Context path.
    context: &'static str,
}

impl<'a> QvmSaveReader<'a> {
    /// Read a value.
    pub fn new(value: &'a QvmSaveValue, context: &'static str) -> Self {
        Self { value, context }
    }

    /// Fail with context.
    fn fail<T>(&self, message: &str) -> Result<T, GuestError> {
        Err(GuestError::invalid(format!("{}: {message}", self.context)))
    }

    /// Read a record field.
    pub fn field(&self, name: &str) -> Result<QvmSaveReader<'a>, GuestError> {
        match self.value {
            QvmSaveValue::Record(fields) => fields
                .get(name)
                .map(|value| QvmSaveReader::new(value, self.context))
                .ok_or_else(|| GuestError::invalid(format!("{}: missing {name}", self.context))),
            _ => self.fail("expected a record"),
        }
    }

    /// Require a literal integer.
    pub fn literal(&self, expected: i64) -> Result<(), GuestError> {
        match self.value {
            QvmSaveValue::Int(value) if *value == expected => Ok(()),
            _ => self.fail("unexpected save version"),
        }
    }

    /// Read a bounded integer.
    pub fn integer(&self, minimum: i64) -> Result<i64, GuestError> {
        match self.value {
            QvmSaveValue::Int(value) if *value >= minimum => Ok(*value),
            _ => self.fail("expected an integer"),
        }
    }

    /// Read a list.
    pub fn list(&self) -> Result<Vec<QvmSaveReader<'a>>, GuestError> {
        match self.value {
            QvmSaveValue::List(values) => Ok(values
                .iter()
                .map(|value| QvmSaveReader::new(value, self.context))
                .collect()),
            _ => self.fail("expected a list"),
        }
    }

    /// Read a nullable namespaced id.
    pub fn nullable_namespaced(&self) -> Result<Option<String>, GuestError> {
        match self.value {
            QvmSaveValue::Null => Ok(None),
            QvmSaveValue::Text(value)
                if value
                    .split_once(':')
                    .is_some_and(|(left, right)| !left.is_empty() && !right.is_empty()) =>
            {
                Ok(Some(value.clone()))
            }
            _ => self.fail("expected a namespaced id"),
        }
    }

    /// Read a status choice.
    pub fn status(&self) -> Result<QvmWeaponRequestStatus, GuestError> {
        match self.value {
            QvmSaveValue::Text(value) => QvmWeaponRequestStatus::parse(value)
                .map_err(|_| GuestError::invalid(format!("{}: unknown status", self.context))),
            _ => self.fail("expected a status"),
        }
    }
}

/// Read a saved actor.
fn read_saved_actor(reader: &QvmSaveReader) -> Result<QvmSavedModActor, GuestError> {
    Ok(QvmSavedModActor {
        slot: reader.field("slot")?.integer(0)? as u32,
        generation: reader.field("generation")?.integer(0)? as u32,
    })
}

/// Pending weapon request.
#[derive(Debug, Clone, PartialEq, Eq)]
struct WeaponRequest {
    /// Request id.
    id: u64,
    /// Item, if any.
    item: Option<ItemId>,
    /// Status.
    status: QvmWeaponRequestStatus,
    /// Whether attempted.
    attempted: bool,
}

/// Admitted actor entry.
struct Entry {
    /// Owned actor.
    actor: OwnedActor,
    /// Record base addresses.
    addresses: HashMap<String, usize>,
    /// Item lease.
    lease: QvmSourceItemLease,
    /// Observer remover.
    unobserve: Option<Box<dyn FnOnce()>>,
    /// Pending request.
    request: Option<WeaponRequest>,
    /// Weapon remover.
    remove_weapon: Option<Box<dyn FnOnce()>>,
}

/// Storage access over an actor's records.
struct ItemsAccess<'a> {
    /// Memory.
    memory: QvmSharedMemory,
    /// Operations.
    operations: &'a dyn QvmModItemsOperations,
    /// Actor.
    actor: ActorId,
    /// Previous write overlay.
    previous: Option<QvmCommittedWrite>,
}

impl QvmItemStorageAccess for ItemsAccess<'_> {
    fn read(&self, field: &QvmItemField) -> Result<i32, GuestError> {
        let address = self.operations.pointer(&self.actor, &field.record)? + field.offset;
        self.scalar(address)
    }

    fn global(&self, address: usize) -> Result<i32, GuestError> {
        self.scalar(address)
    }

    fn write(&self, field: &QvmItemField, value: i32) -> Result<(), GuestError> {
        let address = self.operations.pointer(&self.actor, &field.record)? + field.offset;
        self.memory.write_i32(address, value)
    }
}

impl ItemsAccess<'_> {
    /// Read a word with the previous-write overlay.
    fn scalar(&self, address: usize) -> Result<i32, GuestError> {
        let mut bytes = self.memory.read_bytes(address, 4)?;
        if let Some(previous) = self.previous.as_ref() {
            for range in &previous.ranges {
                for (index, byte) in range.before.iter().enumerate() {
                    let target = range.byte_offset as i64 + index as i64 - address as i64;
                    if (0..4).contains(&target) {
                        bytes[target as usize] = *byte;
                    }
                }
            }
        }
        Ok(i32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }
}

/// Canonical inventory borrows actual original words; source writes publish without replay.
pub struct QvmModItems {
    /// Item definitions.
    definition: QvmModItemsDeclaration,
    /// Source module.
    module: QvmModule,
    /// Image.
    image: QvmImage,
    /// Host services.
    services: Rc<dyn QvmModItemsServices>,
    /// Owning provider.
    provider: ProviderId,
    /// Owning content.
    content: String,
    /// Provider operations.
    operations: Rc<dyn QvmModItemsOperations>,
    /// Weapon stage driver, if any.
    pub weapons: Option<Rc<dyn QvmWeaponStageDriver>>,
    /// Admitted entries.
    entries: RefCell<HashMap<ActorId, Entry>>,
    /// Admissions.
    admissions: Vec<QvmSourceItemAdmission>,
    /// Storage by item.
    by_item: HashMap<ItemId, QvmItemStorage>,
    /// Next request id.
    next_request: Cell<u64>,
    /// Consumed applications.
    applied: RefCell<Vec<ModClientApplication>>,
    /// First stashed observer error.
    error: RefCell<Option<GuestError>>,
}

impl QvmModItems {
    /// Bind item definitions.
    #[allow(clippy::too_many_arguments)]
    pub fn create(
        definition: QvmModItemsDeclaration,
        module: QvmModule,
        image: QvmImage,
        services: Rc<dyn QvmModItemsServices>,
        provider: ProviderId,
        content: String,
        operations: Rc<dyn QvmModItemsOperations>,
        weapons: Option<Rc<dyn QvmWeaponStageDriver>>,
    ) -> Rc<Self> {
        let admissions = definition
            .definitions
            .iter()
            .map(|item| QvmSourceItemAdmission {
                admission: item.admission,
                definition: QvmSourceItemDefinition {
                    item: item.item.clone(),
                    label: item.label.clone(),
                    kind: item.kind.clone(),
                    source: QvmItemSource {
                        provider: provider.clone(),
                        content: content.clone(),
                    },
                    icon: item.icon.as_ref().map(|icon| resolve_item_icon(icon, &content)),
                    actions: item
                        .actions
                        .as_ref()
                        .map(|actions| actions.action_names().into_iter().map(str::to_string).collect()),
                },
            })
            .collect();
        let by_item = definition
            .storage
            .iter()
            .flat_map(|storage| match storage {
                QvmItemStorage::Counter { item, .. } => vec![(item.clone(), storage.clone())],
                QvmItemStorage::Bits { items, .. } => items
                    .iter()
                    .map(|value| (value.item.clone(), storage.clone()))
                    .collect(),
            })
            .collect();
        Rc::new(Self {
            definition,
            module,
            image,
            services,
            provider,
            content,
            operations,
            weapons,
            entries: RefCell::new(HashMap::new()),
            admissions,
            by_item,
            next_request: Cell::new(0),
            applied: RefCell::new(Vec::new()),
            error: RefCell::new(None),
        })
    }

    /// Stash the first observer error.
    fn stash(&self, error: GuestError) {
        if self.error.borrow().is_none() {
            *self.error.borrow_mut() = Some(error);
        }
    }

    /// Take the stashed observer error, if any.
    pub fn take_error(&self) -> Option<GuestError> {
        self.error.borrow_mut().take()
    }

    /// Absolute address of a record field.
    fn address(&self, actor: &ActorId, field: &QvmItemField) -> Result<usize, GuestError> {
        Ok(self.operations.pointer(actor, &field.record)? + field.offset)
    }

    /// Whether an actor's entry is current.
    fn current(&self, actor: &ActorId) -> bool {
        let entries = self.entries.borrow();
        let Some(entry) = entries.get(actor) else {
            return false;
        };
        if self.services.resolve_owned(actor).as_ref() != Some(&entry.actor)
            || !self.operations.live(actor)
            || !(entry.lease.current)()
        {
            return false;
        }
        entry
            .addresses
            .iter()
            .all(|(record, address)| self.operations.pointer(actor, record).ok().as_ref() == Some(address))
    }

    /// Read storage entries.
    fn read(
        &self,
        actor: &ActorId,
        storage: &QvmItemStorage,
        previous: Option<QvmCommittedWrite>,
    ) -> Result<Vec<InventoryEntry>, GuestError> {
        read_qvm_item_storage(
            &self.image,
            storage,
            &ItemsAccess {
                memory: self.module.memory(),
                operations: &*self.operations,
                actor: actor.clone(),
                previous,
            },
        )
    }

    /// Write one storage entry.
    fn write(&self, actor: &ActorId, entry: InventoryEntry) -> Result<(), GuestError> {
        let storage = self.by_item.get(&entry.item).cloned();
        let Some(storage) = storage else {
            return Err(GuestError::invalid("QVM item storage is no longer admitted"));
        };
        if !self.current(actor) {
            return Err(GuestError::invalid("QVM item storage is no longer admitted"));
        }
        write_qvm_item_storage(
            &self.image,
            &storage,
            &entry,
            &ItemsAccess {
                memory: self.module.memory(),
                operations: &*self.operations,
                actor: actor.clone(),
                previous: None,
            },
        )
    }

    /// Admit an actor.
    pub fn admit(self: &Rc<Self>, actor: &ActorId) -> Result<(), GuestError> {
        if self.entries.borrow().contains_key(actor) {
            return Ok(());
        }
        let owner = self
            .services
            .resolve_owned(actor)
            .ok_or_else(|| GuestError::invalid("QVM source items require the current admitted client"))?;
        if !self.operations.live(actor) {
            return Err(GuestError::invalid(
                "QVM source items require the current admitted client",
            ));
        }
        let mut addresses = HashMap::new();
        for storage in &self.definition.storage {
            for field in storage_fields(storage) {
                addresses.insert(field.record.clone(), self.operations.pointer(actor, &field.record)?);
            }
        }
        let lease = self.services.bind_items(
            &owner,
            QvmItemsAdmission {
                owner: self.provider.clone(),
                items: self.admissions.clone(),
                invoke: {
                    let this = Rc::clone(self);
                    let actor = actor.clone();
                    Rc::new(move |item: &str, action: &str| {
                        let call = this
                            .definition
                            .definitions
                            .iter()
                            .find(|value| value.item == item)
                            .and_then(|value| value.actions.as_ref())
                            .and_then(|actions: &QvmItemActions| match action {
                                "use" => actions.use_call.clone(),
                                "drop" => actions.drop_call.clone(),
                                _ => None,
                            });
                        let Some(call) = call else {
                            return Err(GuestError::invalid("Source item action is no longer admitted"));
                        };
                        if !this.current(&actor) {
                            return Err(GuestError::invalid("Source item action is no longer admitted"));
                        }
                        this.operations.invoke(&actor, &call)?;
                        Ok(())
                    })
                },
                state: QvmItemStateAccess {
                    read: {
                        let this = Rc::clone(self);
                        let actor = actor.clone();
                        Rc::new(move || {
                            let mut entries = Vec::new();
                            for storage in &this.definition.storage {
                                entries.extend(this.read(&actor, storage, None)?);
                            }
                            Ok(entries)
                        })
                    },
                    entry: {
                        let this = Rc::clone(self);
                        let actor = actor.clone();
                        Rc::new(move |item: &str| {
                            let storage = this.by_item.get(item).cloned();
                            let Some(storage) = storage else {
                                return Ok(None);
                            };
                            Ok(this
                                .read(&actor, &storage, None)?
                                .into_iter()
                                .find(|value| value.item == item))
                        })
                    },
                    write: {
                        let this = Rc::clone(self);
                        let actor = actor.clone();
                        Rc::new(move |entry: InventoryEntry| this.write(&actor, entry))
                    },
                    mutable_capacity: {
                        let by_item = self.by_item.clone();
                        Rc::new(move |item: &str| {
                            matches!(
                                by_item.get(item),
                                Some(QvmItemStorage::Counter {
                                    capacity: QvmItemCapacity::Field { .. },
                                    ..
                                })
                            )
                        })
                    },
                },
            },
        );
        let watches: Vec<(QvmItemStorage, Vec<usize>)> = self
            .definition
            .storage
            .iter()
            .map(|storage| {
                let mut watched: Vec<usize> = storage_fields(storage)
                    .iter()
                    .map(|field| self.address(actor, field))
                    .collect::<Result<_, _>>()?;
                if let QvmItemStorage::Counter {
                    capacity: QvmItemCapacity::Source { overrides, .. },
                    ..
                } = storage
                {
                    watched.extend(overrides.iter().map(|value| value.address));
                }
                Ok((storage.clone(), watched))
            })
            .collect::<Result<_, GuestError>>()?;
        let pending: Rc<RefCell<Option<Vec<QvmSourceItemStore>>>> = Rc::new(RefCell::new(None));
        let ranges: Vec<QvmWriteRange> = watches
            .iter()
            .flat_map(|(_, addresses)| addresses.iter())
            .map(|address| QvmWriteRange {
                byte_offset: *address,
                byte_length: 4,
            })
            .collect();
        let publish_this = Rc::clone(self);
        let publish_actor = actor.clone();
        let publish_pending = Rc::clone(&pending);
        let publish_watches = watches.clone();
        let commit_this = Rc::clone(self);
        let commit_actor = actor.clone();
        let commit_pending = Rc::clone(&pending);
        let id = self.module.memory().observe_writes(
            ranges,
            Rc::new(move |event: &QvmCommittedWrite| {
                let mut changes = Vec::new();
                for (storage, addresses) in &publish_watches {
                    let touched = addresses.iter().any(|address| {
                        event.touches(&QvmWriteRange {
                            byte_offset: *address,
                            byte_length: 4,
                        })
                    });
                    if !touched {
                        continue;
                    }
                    let (before, after) = match (
                        publish_this.read(&publish_actor, storage, Some(event.clone())),
                        publish_this.read(&publish_actor, storage, None),
                    ) {
                        (Ok(before), Ok(after)) => (before, after),
                        _ => {
                            publish_this.stash(GuestError::invalid("QVM item storage read failed during observation"));
                            *publish_pending.borrow_mut() = None;
                            return;
                        }
                    };
                    for value in &after {
                        let previous = before.iter().find(|entry| entry.item == value.item);
                        let Some(previous) = previous else {
                            continue;
                        };
                        if previous.count == value.count && previous.capacity == value.capacity {
                            continue;
                        }
                        if let Some(pickup) = publish_this.operations.pickup() {
                            let declared = pickup.iter().any(|write| {
                                matches!(
                                    write,
                                    QvmPickupWriteDecl::Inventory { item, fields }
                                    if *item == value.item
                                        && (previous.count == value.count
                                            || *fields != QvmPickupWriteFields::Capacity)
                                        && (previous.capacity == value.capacity
                                            || *fields != QvmPickupWriteFields::Count)
                                )
                            });
                            if !declared {
                                publish_this.stash(GuestError::invalid(
                                    "Original pickup changed an undeclared QVM source item",
                                ));
                                *publish_pending.borrow_mut() = None;
                                return;
                            }
                        }
                        changes.push(QvmSourceItemStore {
                            before: previous.clone(),
                            after: value.clone(),
                        });
                    }
                }
                *publish_pending.borrow_mut() = Some(changes);
            }),
            Some(Rc::new(move |_: &QvmCommittedWrite| {
                let changes = commit_pending.borrow_mut().take();
                let (Some(changes), entry) = (changes, commit_this.entries.borrow().get(&commit_actor).is_some())
                else {
                    return;
                };
                if changes.is_empty() || !entry || !commit_this.current(&commit_actor) {
                    return;
                }
                let lease = commit_this
                    .entries
                    .borrow()
                    .get(&commit_actor)
                    .map(|entry| Rc::clone(&entry.lease.stored));
                let Some(stored) = lease else {
                    return;
                };
                match stored(&changes) {
                    Ok(()) => {
                        if !commit_this.current(&commit_actor) {
                            if let Some(weapons) = commit_this.weapons.as_ref() {
                                weapons.cancel_retired(&commit_actor);
                            }
                        }
                    }
                    Err(QvmItemStoreError::Retired) => {
                        if let Some(weapons) = commit_this.weapons.as_ref() {
                            weapons.cancel_retired(&commit_actor);
                        }
                        commit_this.stash(GuestError::invalid("Source item lease retired"));
                    }
                    Err(QvmItemStoreError::Other(error)) => commit_this.stash(error),
                }
            })),
        );
        let memory = self.module.memory();
        let entry = Entry {
            actor: owner.clone(),
            addresses,
            lease,
            unobserve: Some(Box::new(move || {
                memory.remove_observer(id);
            })),
            request: None,
            remove_weapon: Some(Box::new(|| {})),
        };
        self.entries.borrow_mut().insert(actor.clone(), entry);
        if self.weapons.is_some() {
            let binding = self.binding(actor)?;
            match self.services.bind_weapon(&owner, binding) {
                Some(remove) => {
                    if let Some(entry) = self.entries.borrow_mut().get_mut(actor) {
                        entry.remove_weapon = Some(remove);
                    }
                }
                None => {
                    self.release(actor);
                    return Err(GuestError::invalid("QVM source weapon service is unavailable"));
                }
            }
        }
        Ok(())
    }

    /// Selection value for an item.
    fn value(&self, item: Option<&str>) -> Option<i32> {
        let item = item?;
        self.definition
            .weapons
            .as_ref()?
            .stage
            .selection
            .values
            .iter()
            .find(|value| value.item == item)
            .map(|value| value.value as i32)
    }

    /// Requested selection value for an actor.
    pub fn requested(&self, actor: &ActorId) -> Option<i32> {
        let entries = self.entries.borrow();
        let request = entries.get(actor)?.request.as_ref()?;
        if !self.current(actor) || request.status == QvmWeaponRequestStatus::Refused {
            return None;
        }
        self.value(request.item.as_deref())
    }

    /// Whether an actor's weapon host is live.
    pub fn stage_live(&self, actor: &ActorId) -> bool {
        self.entries.borrow().contains_key(actor) && self.current(actor)
    }

    /// Authoritative posture for weapon input.
    pub fn stage_posture(&self, actor: &ActorId) -> Result<QvmWeaponPosture, GuestError> {
        let body = self.services.body_bounds(actor);
        let view = self
            .services
            .clients_for_actor(actor)
            .and_then(|client| self.services.player_view(&client));
        let (Some(bounds), Some(view)) = (body, view) else {
            return Err(GuestError::invalid(
                "QVM weapon input requires the actual authoritative posture",
            ));
        };
        Ok(QvmWeaponPosture {
            bounds,
            view_height: view.view_offset.z,
            ground: self.operations.ground(actor)?,
        })
    }

    /// Whether the provider weapon is selected.
    pub fn stage_selected(&self, actor: &ActorId) -> bool {
        self.services.weapon_selected(actor)
    }

    /// Mark a matching pending request attempted.
    pub fn stage_attempted(&self, actor: &ActorId, value: i32) {
        let mut entries = self.entries.borrow_mut();
        if let Some(entry) = entries.get_mut(actor) {
            if let Some(request) = entry.request.as_mut() {
                if request.status == QvmWeaponRequestStatus::Pending
                    && self.value(request.item.as_deref()) == Some(value)
                {
                    request.attempted = true;
                }
            }
        }
    }

    /// Accept a matching pending request.
    pub fn stage_accepted(&self, actor: &ActorId, value: i32) {
        let mut entries = self.entries.borrow_mut();
        if let Some(entry) = entries.get_mut(actor) {
            if let Some(request) = entry.request.as_mut() {
                if request.status == QvmWeaponRequestStatus::Pending
                    && self.value(request.item.as_deref()) == Some(value)
                {
                    request.status = QvmWeaponRequestStatus::Accepted;
                }
            }
        }
    }

    /// Complete a pending request.
    pub fn stage_completed(&self, actor: &ActorId) {
        let settled = self.weapons.as_ref().and_then(|weapons| {
            let entries = self.entries.borrow();
            let request = entries.get(actor)?.request.as_ref()?;
            if request.status != QvmWeaponRequestStatus::Pending {
                return None;
            }
            Some((weapons.active(actor) == request.item, request.attempted))
        });
        let Some((active, attempted)) = settled else {
            return;
        };
        if let Some(entry) = self.entries.borrow_mut().get_mut(actor) {
            if let Some(request) = entry.request.as_mut() {
                if request.status != QvmWeaponRequestStatus::Pending {
                    return;
                }
                if active {
                    request.status = QvmWeaponRequestStatus::Accepted;
                } else if attempted {
                    request.status = QvmWeaponRequestStatus::Refused;
                }
            }
        }
    }

    /// Invoke a source call for an actor.
    pub fn stage_invoke(&self, actor: &ActorId, call: &QvmModSourceCall) -> Result<i32, GuestError> {
        self.operations.invoke(actor, call)
    }

    /// Request a weapon capability.
    fn request_weapon(
        self: &Rc<Self>,
        actor: &ActorId,
        item: Option<ItemId>,
    ) -> Result<QvmWeaponRequestHandle, GuestError> {
        if !self.current(actor) {
            return Err(GuestError::invalid("QVM source weapon request lost its actor"));
        }
        let accepted = match item.as_deref() {
            None => self
                .weapons
                .as_ref()
                .is_some_and(|weapons| weapons.active(actor).is_some() && weapons.settled(actor)),
            Some(item) => self.accepts(actor, item),
        };
        let id = self.next_request.get() + 1;
        self.next_request.set(id);
        let status = if accepted {
            if item.is_none() {
                QvmWeaponRequestStatus::Accepted
            } else {
                QvmWeaponRequestStatus::Pending
            }
        } else {
            QvmWeaponRequestStatus::Refused
        };
        if let Some(entry) = self.entries.borrow_mut().get_mut(actor) {
            entry.request = Some(WeaponRequest {
                id,
                item,
                status,
                attempted: false,
            });
        }
        Ok(self.capability(actor, id))
    }

    /// Whether an item is acceptable.
    fn accepts(&self, actor: &ActorId, item: &str) -> bool {
        self.current(actor) && self.value(Some(item)).is_some() && self.services.inventory_count(actor, item) > 0
    }

    /// Capability handle for a request id.
    fn capability(self: &Rc<Self>, actor: &ActorId, id: u64) -> QvmWeaponRequestHandle {
        let status_this = Rc::clone(self);
        let status_actor = actor.clone();
        let cancel_this = Rc::clone(self);
        let cancel_actor = actor.clone();
        QvmWeaponRequestHandle {
            id,
            status: Rc::new(move || {
                if !status_this.current(&status_actor) {
                    return QvmWeaponRequestStatus::Refused;
                }
                status_this
                    .entries
                    .borrow()
                    .get(&status_actor)
                    .and_then(|entry| entry.request.as_ref())
                    .filter(|request| request.id == id)
                    .map(|request| request.status)
                    .unwrap_or(QvmWeaponRequestStatus::Refused)
            }),
            cancel: Rc::new(move || {
                if !cancel_this.current(&cancel_actor) {
                    return;
                }
                if let Some(entry) = cancel_this.entries.borrow_mut().get_mut(&cancel_actor) {
                    let stale = entry
                        .request
                        .as_ref()
                        .is_some_and(|request| request.id != id || request.status != QvmWeaponRequestStatus::Pending);
                    if stale {
                        return;
                    }
                    if let Some(request) = entry.request.as_mut() {
                        request.status = QvmWeaponRequestStatus::Refused;
                    }
                    entry.request = None;
                }
            }),
        }
    }

    /// Weapon binding for an actor.
    fn binding(self: &Rc<Self>, actor: &ActorId) -> Result<QvmSourceWeaponBinding, GuestError> {
        if self.weapons.is_none() {
            return Err(GuestError::invalid("QVM weapon stage is absent"));
        }
        let current_this = Rc::clone(self);
        let current_actor = actor.clone();
        let read_this = Rc::clone(self);
        let read_actor = actor.clone();
        let accepts_this = Rc::clone(self);
        let accepts_actor = actor.clone();
        let select_this = Rc::clone(self);
        let select_actor = actor.clone();
        let holster_this = Rc::clone(self);
        let holster_actor = actor.clone();
        let settled_this = Rc::clone(self);
        let settled_actor = actor.clone();
        let resume_this = Rc::clone(self);
        let resume_actor = actor.clone();
        let restore_this = Rc::clone(self);
        let restore_actor = actor.clone();
        Ok(QvmSourceWeaponBinding {
            current: Rc::new(move || current_this.current(&current_actor)),
            read: Rc::new(move || {
                if !read_this.current(&read_actor) {
                    return Err(GuestError::invalid("QVM weapon presentation lost its original actor"));
                }
                let active = read_this
                    .weapons
                    .as_ref()
                    .and_then(|weapons| weapons.active(&read_actor));
                let pending = read_this
                    .entries
                    .borrow()
                    .get(&read_actor)
                    .and_then(|entry| entry.request.as_ref())
                    .and_then(|request| {
                        if request.status == QvmWeaponRequestStatus::Refused || request.item == active {
                            None
                        } else {
                            request.item.clone()
                        }
                    });
                Ok(QvmWeaponPresentation {
                    source: QvmItemSource {
                        provider: read_this.provider.clone(),
                        content: read_this.content.clone(),
                    },
                    active,
                    pending,
                    items: read_this
                        .admissions
                        .iter()
                        .map(|value| value.definition.clone())
                        .collect(),
                })
            }),
            handoff: QvmWeaponHandoff {
                kind: "source-input".to_string(),
                provider: self.provider.clone(),
                accepts: Rc::new(move |item: &str| accepts_this.accepts(&accepts_actor, item)),
                select: Rc::new(move |item: &str| {
                    select_this.accepts(&select_actor, item)
                        && select_this
                            .request_weapon(&select_actor, Some(item.to_string()))
                            .is_ok_and(|handle| (handle.status)() != QvmWeaponRequestStatus::Refused)
                }),
                holster: Rc::new(move || {
                    if !holster_this.current(&holster_actor) {
                        return Err(GuestError::invalid("QVM source weapon retired while holstering"));
                    }
                    Ok(())
                }),
                is_holstered: Rc::new(move || {
                    settled_this.current(&settled_actor)
                        && settled_this
                            .weapons
                            .as_ref()
                            .is_some_and(|weapons| weapons.settled(&settled_actor))
                }),
                resume: Rc::new(move |item: Option<ItemId>| resume_this.request_weapon(&resume_actor, item)),
                restore_request: Rc::new(move |id: u64, item: Option<ItemId>| {
                    let entries = restore_this.entries.borrow();
                    let request = entries.get(&restore_actor).and_then(|entry| entry.request.as_ref());
                    if !restore_this.current(&restore_actor)
                        || request.is_none_or(|request| request.id != id || request.item != item)
                    {
                        return Err(GuestError::invalid(
                            "Saved QVM weapon request differs from its original source owner",
                        ));
                    }
                    Ok(restore_this.capability(&restore_actor, id))
                }),
            },
        })
    }

    /// Open a client application.
    pub fn open(&self, application: &ModClientApplication) -> Box<dyn FnOnce()> {
        self.weapons
            .as_ref()
            .map(|weapons| weapons.open(application))
            .unwrap_or_else(|| Box::new(|| {}))
    }

    /// Whether a call is the weapon input entry.
    pub fn applies(&self, call: &QvmModSourceCall) -> bool {
        self.definition
            .weapons
            .as_ref()
            .is_some_and(|weapons| weapons.input_entry == call.entry)
    }

    /// Apply weapon input around a movement slice.
    pub fn apply(&self, application: &ModClientApplication, run: &dyn Fn() -> i32) -> Result<i32, GuestError> {
        let Some(input) = self.definition.weapons.as_ref() else {
            return Ok(run());
        };
        if application.scope != "movement-slice" || self.applied.borrow().contains(application) {
            return Err(GuestError::invalid(
                "QVM weapon input must consume each actual movement slice exactly once",
            ));
        }
        let entry = self.entries.borrow().get(&application.identity.actor).is_some();
        if !entry || !self.current(&application.identity.actor) {
            return Err(GuestError::invalid("QVM weapon application lost its admitted actor"));
        }
        self.applied.borrow_mut().push(application.clone());
        let end = application.frame.time.as_milliseconds();
        let elapsed = application.frame.elapsed.as_milliseconds();
        if !end.is_finite() || !elapsed.is_finite() || elapsed < 0.0 {
            return Err(GuestError::invalid("Invalid applied source weapon clock"));
        }
        let clock = self.address(&application.identity.actor, &input.input_clock)?;
        let start = (end.trunc() - elapsed.trunc()) as i64 & 0xffff_ffff;
        self.module.memory().write_i32(clock, start as i32)?;
        match self.weapons.as_ref() {
            Some(weapons) => Ok(weapons.apply(&application.identity.actor, run)),
            None => Ok(run()),
        }
    }

    /// Checkpoint requests.
    pub fn checkpoint(&self) -> QvmItemsCheckpoint {
        QvmItemsCheckpoint {
            version: 1,
            next_request: self.next_request.get(),
            requests: self
                .entries
                .borrow()
                .values()
                .filter_map(|entry| {
                    entry.request.as_ref().map(|request| QvmSavedWeaponRequest {
                        actor: QvmSavedModActor {
                            slot: entry.actor.id().slot(),
                            generation: entry.actor.id().generation(),
                        },
                        id: request.id,
                        item: request.item.clone(),
                        status: request.status,
                    })
                })
                .collect(),
        }
    }

    /// Restore requests.
    pub fn restore(&self, value: &QvmSaveValue) -> Result<(), GuestError> {
        let reader = QvmSaveReader::new(value, "qvm.items");
        reader.field("version")?.literal(1)?;
        self.next_request.set(reader.field("nextRequest")?.integer(0)? as u64);
        let mut ids = HashSet::new();
        let mut actors = HashSet::new();
        for row in reader.field("requests")?.list()? {
            let saved = read_saved_actor(&row.field("actor")?)?;
            let actor = self
                .services
                .reference_saved(&saved)
                .ok_or_else(|| GuestError::invalid("Saved QVM weapon request is not its current admitted owner"))?;
            let id = row.field("id")?.integer(1)? as u64;
            let item = row.field("item")?.nullable_namespaced()?;
            let status = row.field("status")?.status()?;
            if !self.current(&actor)
                || id > self.next_request.get()
                || !ids.insert(id)
                || !actors.insert(actor.clone())
                || item.as_deref().is_some_and(|item| self.value(Some(item)).is_none())
            {
                return Err(GuestError::invalid(
                    "Saved QVM weapon request is not its current admitted owner",
                ));
            }
            let mut entries = self.entries.borrow_mut();
            let Some(entry) = entries.get_mut(&actor) else {
                return Err(GuestError::invalid(
                    "Saved QVM weapon request is not its current admitted owner",
                ));
            };
            entry.request = Some(WeaponRequest {
                id,
                item,
                status,
                attempted: false,
            });
        }
        Ok(())
    }

    /// Release all actors.
    pub fn clear(&self) {
        let actors: Vec<ActorId> = self.entries.borrow().keys().cloned().collect();
        for actor in actors {
            self.release(&actor);
        }
    }

    /// Release one actor.
    pub fn release(&self, actor: &ActorId) {
        let Some(mut entry) = self.entries.borrow_mut().remove(actor) else {
            return;
        };
        entry.request = None;
        if let Some(unobserve) = entry.unobserve.take() {
            unobserve();
        }
        if let Some(remove) = entry.remove_weapon.take() {
            remove();
        }
        (entry.lease.close)();
    }

    /// Close the stage and release all actors.
    pub fn close(&self) {
        if let Some(weapons) = self.weapons.as_ref() {
            weapons.close();
        }
        self.clear();
    }
}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};
    use std::collections::HashMap;
    use std::rc::Rc;

    use qa_core::identity::{IdentityOwner, ProviderId};
    use qa_core::math::{vec3, Bounds};
    use qa_world::inventory::InventoryEntry;

    use super::super::game_data::{
        AbiProfile, ModuleIdentity, QvmArtifact, QvmImage, QvmInstruction, QvmOpcode, QvmRole,
    };
    use super::super::item_storage::{
        QvmItemAdmission, QvmItemTest, QvmItemTestComparison, QvmProjection, QvmSelectionValue, QvmStageCall,
        QvmWeaponActor, QvmWeaponContinuation, QvmWeaponDispatcher, QvmWeaponPredicate, QvmWeaponRequest,
        QvmWeaponSelection,
    };
    use super::super::mod_actors::{QvmModInputPointer, QvmModInputPointerBase, QvmModReturn};
    use super::super::mod_input::{
        ModClientApplication, QvmModClientCommand, QvmModClientFrame, QvmModClientIdentity, QvmModTime,
    };
    use super::*;

    const CLIENTS: usize = 8192;

    struct FixtureDriver {
        settled: Rc<Cell<bool>>,
        active: RefCell<Option<ItemId>>,
        applied: Rc<Cell<usize>>,
        cancelled: RefCell<Vec<ActorId>>,
        closed: Rc<Cell<bool>>,
    }

    impl QvmWeaponStageDriver for FixtureDriver {
        fn settled(&self, _actor: &ActorId) -> bool {
            self.settled.get()
        }

        fn active(&self, _actor: &ActorId) -> Option<ItemId> {
            self.active.borrow().clone()
        }

        fn apply(&self, _actor: &ActorId, run: &dyn Fn() -> i32) -> i32 {
            self.applied.set(self.applied.get() + 1);
            run()
        }

        fn open(&self, _application: &ModClientApplication) -> Box<dyn FnOnce()> {
            Box::new(|| {})
        }

        fn cancel_retired(&self, actor: &ActorId) {
            self.cancelled.borrow_mut().push(actor.clone());
        }

        fn close(&self) {
            self.closed.set(true);
        }
    }

    struct FixtureServices {
        owned: RefCell<HashMap<ActorId, OwnedActor>>,
        lease_current: Rc<Cell<bool>>,
        stored: Rc<RefCell<Vec<Vec<QvmSourceItemStore>>>>,
        store_result: Rc<RefCell<Option<QvmItemStoreError>>>,
        admission: RefCell<Option<QvmItemsAdmission>>,
        leases_closed: Rc<Cell<usize>>,
        counts: RefCell<HashMap<(ActorId, String), i32>>,
        bounds: RefCell<HashMap<ActorId, Bounds>>,
        views: RefCell<HashMap<ClientId, QvmPlayerView>>,
        clients: RefCell<HashMap<ActorId, ClientId>>,
        selected: Rc<Cell<bool>>,
        binding: RefCell<Option<QvmSourceWeaponBinding>>,
        bind_available: Rc<Cell<bool>>,
        removed_weapon: Rc<Cell<usize>>,
        saved: RefCell<HashMap<(u32, u32), ActorId>>,
    }

    impl QvmModItemsServices for FixtureServices {
        fn resolve_owned(&self, actor: &ActorId) -> Option<OwnedActor> {
            self.owned.borrow().get(actor).cloned()
        }

        fn bind_items(&self, _owner: &OwnedActor, admission: QvmItemsAdmission) -> QvmSourceItemLease {
            *self.admission.borrow_mut() = Some(admission.clone());
            let current = Rc::clone(&self.lease_current);
            let stored = Rc::clone(&self.stored);
            let result = Rc::clone(&self.store_result);
            let closed = Rc::clone(&self.leases_closed);
            QvmSourceItemLease {
                current: Rc::new(move || current.get()),
                stored: Rc::new(move |changes: &[QvmSourceItemStore]| {
                    stored.borrow_mut().push(changes.to_vec());
                    match result.borrow().clone() {
                        None => Ok(()),
                        Some(error) => Err(error),
                    }
                }),
                close: Box::new(move || closed.set(closed.get() + 1)),
            }
        }

        fn inventory_count(&self, actor: &ActorId, item: &str) -> i32 {
            self.counts
                .borrow()
                .get(&(actor.clone(), item.to_string()))
                .copied()
                .unwrap_or(0)
        }

        fn body_bounds(&self, actor: &ActorId) -> Option<Bounds> {
            self.bounds.borrow().get(actor).copied()
        }

        fn clients_for_actor(&self, actor: &ActorId) -> Option<ClientId> {
            self.clients.borrow().get(actor).cloned()
        }

        fn player_view(&self, client: &ClientId) -> Option<QvmPlayerView> {
            self.views.borrow().get(client).copied()
        }

        fn weapon_selected(&self, _actor: &ActorId) -> bool {
            self.selected.get()
        }

        fn bind_weapon(&self, _owner: &OwnedActor, binding: QvmSourceWeaponBinding) -> Option<Box<dyn FnOnce()>> {
            if !self.bind_available.get() {
                return None;
            }
            *self.binding.borrow_mut() = Some(binding);
            let removed = Rc::clone(&self.removed_weapon);
            Some(Box::new(move || removed.set(removed.get() + 1)))
        }

        fn reference_saved(&self, saved: &QvmSavedModActor) -> Option<ActorId> {
            self.saved.borrow().get(&(saved.slot, saved.generation)).cloned()
        }
    }

    struct FixtureOperations {
        live: Rc<Cell<bool>>,
        invoked: RefCell<Vec<(ActorId, usize)>>,
        pickup: RefCell<Option<Vec<QvmPickupWriteDecl>>>,
    }

    impl QvmModItemsOperations for FixtureOperations {
        fn pointer(&self, _actor: &ActorId, record: &str) -> Result<usize, GuestError> {
            if record == "client" {
                Ok(CLIENTS)
            } else {
                Err(GuestError::invalid("unknown record"))
            }
        }

        fn live(&self, _actor: &ActorId) -> bool {
            self.live.get()
        }

        fn invoke(&self, actor: &ActorId, call: &QvmModSourceCall) -> Result<i32, GuestError> {
            self.invoked.borrow_mut().push((actor.clone(), call.entry));
            Ok(0)
        }

        fn ground(&self, _actor: &ActorId) -> Result<f32, GuestError> {
            Ok(4.0)
        }

        fn pickup(&self) -> Option<Vec<QvmPickupWriteDecl>> {
            self.pickup.borrow().clone()
        }
    }

    struct Fixture {
        items: Rc<QvmModItems>,
        module: QvmModule,
        services: Rc<FixtureServices>,
        operations: Rc<FixtureOperations>,
        driver: Rc<FixtureDriver>,
        actor: ActorId,
    }

    fn field(record: &str, offset: usize) -> QvmItemField {
        QvmItemField {
            record: record.to_string(),
            offset,
        }
    }

    fn source_call(entry: usize) -> QvmModSourceCall {
        QvmModSourceCall {
            entry,
            arguments: Vec::new(),
            globals: Vec::new(),
            returns: QvmModReturn::Int32,
        }
    }

    fn test(offset: usize, value: i64) -> QvmItemTest {
        super::super::item_storage::QvmItemTest {
            field: field("client", offset),
            mask: None,
            comparison: QvmItemTestComparison::Equals,
            value,
        }
    }

    fn stage() -> QvmWeaponStage {
        let actor = || QvmWeaponActor {
            record: "client".to_string(),
            pointer: QvmModInputPointer {
                base: QvmModInputPointerBase::Argument { index: 0 },
                indirections: Vec::new(),
                offset: 0,
            },
        };
        QvmWeaponStage {
            dispatcher: QvmWeaponDispatcher {
                entry: 5,
                actor: actor(),
            },
            predicates: vec![QvmWeaponPredicate {
                instruction: 6,
                unselected: false,
            }],
            settled: vec![test(80, 1)],
            selection: QvmWeaponSelection {
                field: field("client", 84),
                values: vec![QvmSelectionValue {
                    value: 2,
                    item: "q3:rocket".to_string(),
                }],
            },
            request: QvmWeaponRequest {
                entry: 6,
                argument: 0,
                accepted: Vec::new(),
            },
            continuation: QvmWeaponContinuation {
                entry: 7,
                actor: actor(),
                instruction: 8,
                original_taken: true,
                when: Vec::new(),
                predicates: Vec::new(),
                projection: QvmProjection {
                    movement: QvmModInputPointer {
                        base: QvmModInputPointerBase::Global { address: 100 },
                        indirections: Vec::new(),
                        offset: 0,
                    },
                    byte_length: 24,
                    minimum: 0,
                    maximum: 12,
                    view_height: field("client", 88),
                    ground: field("client", 92),
                },
                calls: vec![QvmStageCall {
                    instruction: 8,
                    call: source_call(7),
                }],
            },
        }
    }

    fn declaration() -> QvmModItemsDeclaration {
        QvmModItemsDeclaration {
            definitions: vec![
                super::super::item_storage::QvmModItemDefinition {
                    item: "q3:shells".to_string(),
                    label: "Shells".to_string(),
                    icon: None,
                    admission: QvmItemAdmission::Add,
                    actions: Some(QvmItemActions {
                        use_call: Some(source_call(11)),
                        drop_call: None,
                    }),
                    kind: QvmModItemKind::Counter,
                },
                super::super::item_storage::QvmModItemDefinition {
                    item: "q3:rocket".to_string(),
                    label: "Rocket".to_string(),
                    icon: None,
                    admission: QvmItemAdmission::Add,
                    actions: None,
                    kind: QvmModItemKind::Weapon {
                        ammo: Some("q3:shells".to_string()),
                        held: None,
                    },
                },
            ],
            storage: vec![
                QvmItemStorage::Counter {
                    field: field("client", 64),
                    item: "q3:shells".to_string(),
                    capacity: QvmItemCapacity::Constant { value: 50 },
                },
                QvmItemStorage::Counter {
                    field: field("client", 68),
                    item: "q3:rocket".to_string(),
                    capacity: QvmItemCapacity::Field {
                        field: field("client", 72),
                    },
                },
            ],
            weapons: Some(super::super::item_storage::QvmModItemWeapons {
                input_entry: 9,
                input_clock: field("client", 76),
                stage: stage(),
            }),
        }
    }

    fn context() -> QvmModItemsContext {
        QvmModItemsContext {
            actor_records: vec![QvmModActorRecord {
                id: "client".to_string(),
                address: CLIENTS,
                stride: 512,
                capacity: 4,
                fields: Vec::new(),
            }],
            clients: Some(QvmModItemsClients {
                records: vec!["client".to_string()],
                input: vec![QvmModItemsInputBinding {
                    scope: "movement-slice".to_string(),
                    phase: QvmInputPhase::After,
                    calls: vec![9],
                }],
            }),
        }
    }

    fn image() -> QvmImage {
        QvmImage {
            instructions: (0..32)
                .map(|index| QvmInstruction::word(QvmOpcode::OpEnter, 0, index * 8))
                .collect(),
            data_length: 8192,
            initialized_data: vec![0u8; 8192],
            allocated_data_length: 65536,
            ..Default::default()
        }
    }

    fn fixture() -> Fixture {
        let owner = IdentityOwner::create("mod-items-test").unwrap();
        let actor = owner.actor(0, 1);
        let client = owner.client(0, 1);
        let owned = owner.owned_actor(&actor, ProviderId::new("test", "mod")).unwrap();
        let artifact = QvmArtifact {
            module: ModuleIdentity {
                id: "test:qagame".to_string(),
                artifact_path: "test".to_string(),
                digest: "test".to_string(),
                revision: "1".to_string(),
            },
            role: QvmRole::Qagame,
            abi_profile: Some(AbiProfile::Modern),
            image: image(),
        };
        let module = QvmModule::new(artifact, None, None).unwrap();
        module.memory().write_i32(CLIENTS + 64, 10).unwrap();
        module.memory().write_i32(CLIENTS + 68, 1).unwrap();
        module.memory().write_i32(CLIENTS + 72, 5).unwrap();
        let services = Rc::new(FixtureServices {
            owned: RefCell::new([(actor.clone(), owned)].into_iter().collect()),
            lease_current: Rc::new(Cell::new(true)),
            stored: Rc::new(RefCell::new(Vec::new())),
            store_result: Rc::new(RefCell::new(None)),
            admission: RefCell::new(None),
            leases_closed: Rc::new(Cell::new(0)),
            counts: RefCell::new([((actor.clone(), "q3:rocket".to_string()), 1)].into_iter().collect()),
            bounds: RefCell::new(
                [(
                    actor.clone(),
                    Bounds {
                        min: vec3(-16.0, -16.0, -24.0),
                        max: vec3(16.0, 16.0, 32.0),
                    },
                )]
                .into_iter()
                .collect(),
            ),
            views: RefCell::new(
                [(
                    client.clone(),
                    QvmPlayerView {
                        view_offset: vec3(0.0, 0.0, 22.0),
                    },
                )]
                .into_iter()
                .collect(),
            ),
            clients: RefCell::new([(actor.clone(), client)].into_iter().collect()),
            selected: Rc::new(Cell::new(false)),
            binding: RefCell::new(None),
            bind_available: Rc::new(Cell::new(true)),
            removed_weapon: Rc::new(Cell::new(0)),
            saved: RefCell::new([((0, 1), actor.clone())].into_iter().collect()),
        });
        let operations = Rc::new(FixtureOperations {
            live: Rc::new(Cell::new(true)),
            invoked: RefCell::new(Vec::new()),
            pickup: RefCell::new(None),
        });
        let driver = Rc::new(FixtureDriver {
            settled: Rc::new(Cell::new(true)),
            active: RefCell::new(None),
            applied: Rc::new(Cell::new(0)),
            cancelled: RefCell::new(Vec::new()),
            closed: Rc::new(Cell::new(false)),
        });
        let items = QvmModItems::create(
            declaration(),
            module.clone(),
            image(),
            Rc::clone(&services) as Rc<dyn QvmModItemsServices>,
            ProviderId::new("test", "mod"),
            "test-content".to_string(),
            Rc::clone(&operations) as Rc<dyn QvmModItemsOperations>,
            Some(Rc::clone(&driver) as Rc<dyn QvmWeaponStageDriver>),
        );
        Fixture {
            items,
            module,
            services,
            operations,
            driver,
            actor,
        }
    }

    fn application(actor: &ActorId) -> ModClientApplication {
        ModClientApplication {
            identity: QvmModClientIdentity { actor: actor.clone() },
            scope: "movement-slice".to_string(),
            command: QvmModClientCommand {
                kind: "q3".to_string(),
                buttons: 0,
                server_time_ms: 0,
                angle_words: [0, 0, 0],
                angle_shorts: [0, 0, 0],
                angles: vec3(0.0, 0.0, 0.0),
                forward_move: 0.0,
                side_move: 0.0,
                up_move: 0.0,
            },
            frame: QvmModClientFrame {
                time: QvmModTime::Milliseconds(1000),
                elapsed: QvmModTime::Milliseconds(8),
            },
            absolute_aim: vec3(0.0, 0.0, 0.0),
        }
    }

    #[test]
    fn validates_declarations() {
        let seen = Rc::new(Cell::new(0));
        let seen_hook = Rc::clone(&seen);
        let validate = |stage: &QvmWeaponStage, image: &QvmImage| {
            let _ = (stage, image);
            seen_hook.set(seen_hook.get() + 1);
            Ok(())
        };
        assert!(validate_qvm_mod_items(&declaration(), &context(), &image(), &validate).is_ok());
        assert_eq!(seen.get(), 1);
        assert!(
            validate_qvm_mod_items(&declaration(), &context(), &image(), &|_, _| Err(GuestError::invalid(
                "bad stage"
            )))
            .is_err()
        );

        let mut bad = declaration();
        bad.definitions.clear();
        assert!(validate_qvm_mod_items(&bad, &context(), &image(), &validate).is_err());

        let mut missing = context();
        missing.clients = None;
        assert!(validate_qvm_mod_items(&declaration(), &missing, &image(), &validate).is_err());

        let mut bad = declaration();
        bad.definitions.push(bad.definitions[0].clone());
        assert!(validate_qvm_mod_items(&bad, &context(), &image(), &validate).is_err());

        let mut bad = declaration();
        bad.weapons = None;
        assert!(validate_qvm_mod_items(&bad, &context(), &image(), &validate).is_err());

        let mut bad = declaration();
        bad.definitions.retain(|value| value.item == "q3:shells");
        assert!(validate_qvm_mod_items(&bad, &context(), &image(), &validate).is_err());

        let mut bad = declaration();
        bad.storage[1] = QvmItemStorage::Counter {
            field: field("client", 64),
            item: "q3:rocket".to_string(),
            capacity: QvmItemCapacity::Constant { value: 5 },
        };
        assert!(validate_qvm_mod_items(&bad, &context(), &image(), &validate).is_err());

        let mut bad = declaration();
        bad.weapons.as_mut().unwrap().stage.continuation.projection.maximum = 8;
        assert!(validate_qvm_mod_items(&bad, &context(), &image(), &validate).is_err());

        let mut bad = declaration();
        bad.weapons.as_mut().unwrap().input_entry = 10;
        assert!(validate_qvm_mod_items(&bad, &context(), &image(), &validate).is_err());

        let mut bad = declaration();
        bad.weapons.as_mut().unwrap().stage.selection.values[0].value = 0;
        assert!(validate_qvm_mod_items(&bad, &context(), &image(), &validate).is_err());

        let mut bad = declaration();
        if let QvmModItemKind::Weapon { ammo, .. } = &mut bad.definitions[1].kind {
            *ammo = Some("q3:nails".to_string());
        }
        assert!(validate_qvm_mod_items(&bad, &context(), &image(), &validate).is_err());
    }

    #[test]
    fn admit_reads_and_observes() {
        let fixture = fixture();
        fixture.items.admit(&fixture.actor).unwrap();
        assert!(fixture.services.binding.borrow().is_some());

        fixture.module.memory().write_i32(CLIENTS + 64, 12).unwrap();
        let stored = fixture.services.stored.borrow();
        assert_eq!(stored.len(), 1);
        assert_eq!(stored[0].len(), 1);
        assert_eq!(stored[0][0].before.count, 10.0);
        assert_eq!(stored[0][0].after.count, 12.0);
        assert_eq!(stored[0][0].after.capacity, 50.0);
        drop(stored);
        assert!(fixture.items.take_error().is_none());
    }

    #[test]
    fn undeclared_pickup_writes_stash() {
        let fixture = fixture();
        fixture.items.admit(&fixture.actor).unwrap();
        *fixture.operations.pickup.borrow_mut() = Some(vec![QvmPickupWriteDecl::Inventory {
            item: "q3:nails".to_string(),
            fields: QvmPickupWriteFields::Count,
        }]);
        fixture.module.memory().write_i32(CLIENTS + 64, 12).unwrap();
        assert!(fixture.services.stored.borrow().is_empty());
        assert!(fixture.items.take_error().is_some());

        *fixture.operations.pickup.borrow_mut() = Some(vec![QvmPickupWriteDecl::Inventory {
            item: "q3:shells".to_string(),
            fields: QvmPickupWriteFields::Count,
        }]);
        fixture.module.memory().write_i32(CLIENTS + 64, 14).unwrap();
        assert_eq!(fixture.services.stored.borrow().len(), 1);
    }

    #[test]
    fn retired_leases_cancel_weapons() {
        let fixture = fixture();
        fixture.items.admit(&fixture.actor).unwrap();
        *fixture.services.store_result.borrow_mut() = Some(QvmItemStoreError::Retired);
        fixture.module.memory().write_i32(CLIENTS + 64, 12).unwrap();
        assert_eq!(
            fixture.driver.cancelled.borrow().as_slice(),
            std::slice::from_ref(&fixture.actor)
        );
        assert!(fixture.items.take_error().is_some());
    }

    #[test]
    fn requests_drive_selection_values() {
        let fixture = fixture();
        fixture.items.admit(&fixture.actor).unwrap();
        let binding = fixture.services.binding.borrow().clone().unwrap();
        assert!((binding.handoff.accepts)("q3:rocket"));
        assert!(!(binding.handoff.accepts)("q3:nails"));
        assert!((binding.handoff.select)("q3:rocket"));
        assert_eq!(fixture.items.requested(&fixture.actor), Some(2));
        fixture.items.stage_accepted(&fixture.actor, 2);
        let read = (binding.read)().unwrap();
        assert_eq!(read.pending.as_deref(), Some("q3:rocket"));
        fixture.items.stage_completed(&fixture.actor);
        assert_eq!(fixture.items.requested(&fixture.actor), Some(2));

        fixture.driver.active.borrow_mut().replace("q3:rocket".to_string());
        let holstered = (binding.handoff.resume)(None).unwrap();
        assert_eq!((holstered.status)(), QvmWeaponRequestStatus::Accepted);

        let handle = (binding.handoff.resume)(Some("q3:rocket".to_string())).unwrap();
        assert_eq!((handle.status)(), QvmWeaponRequestStatus::Pending);
        (handle.cancel)();
        assert_eq!((handle.status)(), QvmWeaponRequestStatus::Refused);
        assert!((binding.handoff.restore_request)(handle.id, Some("q3:rocket".to_string())).is_err());
    }

    #[test]
    fn unavailable_weapons_and_state_access() {
        let fixture = fixture();
        fixture.services.bind_available.set(false);
        assert!(fixture.items.admit(&fixture.actor).is_err());
        assert!(!fixture.items.stage_live(&fixture.actor));
        fixture.services.bind_available.set(true);
        fixture.items.admit(&fixture.actor).unwrap();

        let admission = fixture.services.admission.borrow().clone().unwrap();
        let entries = (admission.state.read)().unwrap();
        assert!(entries
            .iter()
            .any(|entry| entry.item == "q3:shells" && entry.count == 10.0));
        let rocket = (admission.state.entry)("q3:rocket").unwrap().unwrap();
        assert_eq!(rocket.capacity, 5.0);
        assert!((admission.state.entry)("q3:nails").unwrap().is_none());
        assert!((admission.state.mutable_capacity)("q3:rocket"));
        assert!(!(admission.state.mutable_capacity)("q3:shells"));
        (admission.invoke)("q3:shells", "use").unwrap();
        assert_eq!(
            fixture.operations.invoked.borrow().as_slice(),
            &[(fixture.actor.clone(), 11)]
        );
        assert!((admission.invoke)("q3:shells", "drop").is_err());
        (admission.state.write)(InventoryEntry {
            item: "q3:shells".to_string(),
            count: 20.0,
            capacity: 50.0,
            count_policy: None,
        })
        .unwrap();
        assert_eq!(fixture.module.memory().read_i32(CLIENTS + 64).unwrap(), 20);

        let bare = QvmModItems::create(
            declaration(),
            fixture.module.clone(),
            image(),
            Rc::clone(&fixture.services) as Rc<dyn QvmModItemsServices>,
            ProviderId::new("test", "mod"),
            "test-content".to_string(),
            Rc::clone(&fixture.operations) as Rc<dyn QvmModItemsOperations>,
            None,
        );
        bare.admit(&fixture.actor).unwrap();
        let application = application(&fixture.actor);
        let close = bare.open(&application);
        close();
        assert_eq!(bare.apply(&application, &|| 9).unwrap(), 9);
        bare.release(&fixture.actor);
    }

    #[test]
    fn apply_writes_clock_once() {
        let fixture = fixture();
        fixture.items.admit(&fixture.actor).unwrap();
        assert!(fixture.items.applies(&source_call(9)));
        assert!(!fixture.items.applies(&source_call(10)));
        let application = application(&fixture.actor);
        let result = fixture.items.apply(&application, &|| 7).unwrap();
        assert_eq!(result, 7);
        assert_eq!(fixture.driver.applied.get(), 1);
        assert_eq!(fixture.module.memory().read_i32(CLIENTS + 76).unwrap(), 992);
        assert!(fixture.items.apply(&application, &|| 7).is_err());

        let mut foreign = application.clone();
        foreign.scope = "client-command".to_string();
        assert!(fixture.items.apply(&foreign, &|| 7).is_err());
    }

    #[test]
    fn checkpoint_restores_requests() {
        let fixture = fixture();
        fixture.items.admit(&fixture.actor).unwrap();
        let binding = fixture.services.binding.borrow().clone().unwrap();
        assert!((binding.handoff.select)("q3:rocket"));
        let checkpoint = fixture.items.checkpoint();
        assert_eq!(checkpoint.version, 1);
        assert_eq!(checkpoint.requests.len(), 1);

        let value = QvmSaveValue::Record(
            [
                ("version".to_string(), QvmSaveValue::Int(1)),
                ("nextRequest".to_string(), QvmSaveValue::Int(1)),
                (
                    "requests".to_string(),
                    QvmSaveValue::List(vec![QvmSaveValue::Record(
                        [
                            (
                                "actor".to_string(),
                                QvmSaveValue::Record(
                                    [
                                        ("slot".to_string(), QvmSaveValue::Int(0)),
                                        ("generation".to_string(), QvmSaveValue::Int(1)),
                                    ]
                                    .into_iter()
                                    .collect(),
                                ),
                            ),
                            ("id".to_string(), QvmSaveValue::Int(1)),
                            ("item".to_string(), QvmSaveValue::Text("q3:rocket".to_string())),
                            ("status".to_string(), QvmSaveValue::Text("pending".to_string())),
                        ]
                        .into_iter()
                        .collect(),
                    )]),
                ),
            ]
            .into_iter()
            .collect(),
        );
        fixture.items.restore(&value).unwrap();
        assert_eq!(fixture.items.requested(&fixture.actor), Some(2));

        let bad = QvmSaveValue::Record(
            [
                ("version".to_string(), QvmSaveValue::Int(2)),
                ("nextRequest".to_string(), QvmSaveValue::Int(0)),
                ("requests".to_string(), QvmSaveValue::List(Vec::new())),
            ]
            .into_iter()
            .collect(),
        );
        assert!(fixture.items.restore(&bad).is_err());
    }

    #[test]
    fn posture_actions_and_close() {
        let fixture = fixture();
        fixture.items.admit(&fixture.actor).unwrap();
        let posture = fixture.items.stage_posture(&fixture.actor).unwrap();
        assert_eq!(posture.view_height, 22.0);
        assert_eq!(posture.ground, 4.0);
        assert!(!fixture.items.stage_selected(&fixture.actor));
        assert!(fixture.items.stage_live(&fixture.actor));

        fixture.items.close();
        assert!(fixture.driver.closed.get());
        assert_eq!(fixture.services.leases_closed.get(), 1);
        assert_eq!(fixture.services.removed_weapon.get(), 1);
        assert!(!fixture.items.stage_live(&fixture.actor));
    }
}
