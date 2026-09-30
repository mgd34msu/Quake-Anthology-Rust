//! Raw source entity-table views.
//!
//! Provenance: `src/compat/qvm/raw-entities.ts`.
//!
//! [`GuestLayout`], [`GuestFieldLayout`], and [`GuestAddress`] mirror
//! `src/contracts/execution.ts` (`GuestLayout`, `RawEntityView`,
//! `RawEntityTable`); views snapshot their record bytes, matching the donor
//! rule that old pointer views retain their original storage after
//! relocation.

use std::rc::Rc;

use qa_core::identity::ActorId;

use super::game_data::{ModuleIdentity, QvmGameData, QvmSharedMemory};
use super::shared_entity_record::QVM_SHARED_ENTITY_BYTES;
use crate::error::GuestError;

/// Guest field storage class.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuestFieldStorage {
    /// Unsigned bytes.
    Uint8,
    /// Signed 32-bit words.
    Int32,
    /// 32-bit floats.
    Float32,
}

/// One guest layout field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuestFieldLayout {
    /// Field name.
    pub name: String,
    /// Byte offset within the record.
    pub byte_offset: usize,
    /// Storage class.
    pub storage: GuestFieldStorage,
    /// Element count.
    pub count: usize,
}

/// Guest record layout descriptor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuestLayout {
    /// Layout id (`namespace:name`).
    pub id: String,
    /// Record byte length.
    pub byte_length: usize,
    /// Record alignment.
    pub alignment: usize,
    /// Guest pointer width.
    pub pointer_bytes: usize,
    /// Field layouts.
    pub fields: Vec<GuestFieldLayout>,
}

/// Guest address as an allocation byte offset.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GuestAddress {
    /// Byte offset within the allocation.
    pub byte_offset: u64,
}

/// Public `sharedEntity_t` layout of a 32-bit QVM entity record.
#[must_use]
pub fn qvm_shared_entity_layout() -> GuestLayout {
    let field = |name: &str, byte_offset: usize, storage: GuestFieldStorage, count: usize| GuestFieldLayout {
        name: name.to_string(),
        byte_offset,
        storage,
        count,
    };
    GuestLayout {
        id: "q3:shared-entity-qvm32".to_string(),
        byte_length: QVM_SHARED_ENTITY_BYTES,
        alignment: 4,
        pointer_bytes: 4,
        fields: vec![
            field("s", 0, GuestFieldStorage::Uint8, 208),
            field("r.s", 208, GuestFieldStorage::Uint8, 208),
            field("r.linked", 416, GuestFieldStorage::Int32, 1),
            field("r.linkcount", 420, GuestFieldStorage::Int32, 1),
            field("r.svFlags", 424, GuestFieldStorage::Int32, 1),
            field("r.singleClient", 428, GuestFieldStorage::Int32, 1),
            field("r.bmodel", 432, GuestFieldStorage::Int32, 1),
            field("r.mins", 436, GuestFieldStorage::Float32, 3),
            field("r.maxs", 448, GuestFieldStorage::Float32, 3),
            field("r.contents", 460, GuestFieldStorage::Int32, 1),
            field("r.absmin", 464, GuestFieldStorage::Float32, 3),
            field("r.absmax", 476, GuestFieldStorage::Float32, 3),
            field("r.currentOrigin", 488, GuestFieldStorage::Float32, 3),
            field("r.currentAngles", 500, GuestFieldStorage::Float32, 3),
            field("r.ownerNum", 512, GuestFieldStorage::Int32, 1),
        ],
    }
}

/// Resolve the current actor owning a slot (slot, view byte offset).
pub type QvmCurrentActor = Rc<dyn Fn(usize, u64) -> Option<ActorId>>;

/// One raw entity view with snapshotted record bytes.
#[derive(Clone)]
pub struct RawEntityView {
    /// Owning module.
    pub module: ModuleIdentity,
    /// Table slot.
    pub slot: usize,
    /// View address.
    pub address: GuestAddress,
    /// Table stride in bytes.
    pub stride_bytes: usize,
    /// Public record layout.
    pub public_layout: GuestLayout,
    /// Snapshotted record bytes (complete source-owned record).
    pub bytes: Vec<u8>,
    resolver: QvmCurrentActor,
}

impl std::fmt::Debug for RawEntityView {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RawEntityView")
            .field("slot", &self.slot)
            .field("address", &self.address)
            .field("stride_bytes", &self.stride_bytes)
            .field("bytes", &self.bytes.len())
            .finish()
    }
}

impl RawEntityView {
    /// Resolve the current actor on access (slots are reused on free).
    #[must_use]
    pub fn current_actor(&self) -> Option<ActorId> {
        (self.resolver)(self.slot, self.address.byte_offset)
    }
}

/// Raw source entity table over located game data.
#[derive(Clone)]
pub struct RawEntityTable {
    /// Owning module.
    pub module: ModuleIdentity,
    /// Table base address.
    pub base: GuestAddress,
    /// Table stride in bytes.
    pub stride_bytes: usize,
    /// Located entity count.
    pub count: usize,
    /// Table capacity.
    pub capacity: usize,
    /// Public record layout.
    pub layout: GuestLayout,
    memory: QvmSharedMemory,
    table_start: usize,
    resolver: QvmCurrentActor,
}

impl std::fmt::Debug for RawEntityTable {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RawEntityTable")
            .field("base", &self.base)
            .field("stride_bytes", &self.stride_bytes)
            .field("count", &self.count)
            .field("capacity", &self.capacity)
            .finish()
    }
}

impl RawEntityTable {
    /// Open the view for one slot.
    pub fn at_slot(&self, slot: usize) -> Result<RawEntityView, GuestError> {
        if slot >= self.capacity {
            return Err(GuestError::invalid("QVM entity slot exceeds table capacity"));
        }
        let offset = self
            .table_start
            .checked_add(slot.saturating_mul(self.stride_bytes))
            .ok_or_else(|| GuestError::invalid("QVM entity slot exceeds table capacity"))?;
        let bytes = self.memory.read_bytes(offset, self.stride_bytes)?;
        Ok(RawEntityView {
            module: self.module.clone(),
            slot,
            address: GuestAddress {
                byte_offset: (offset - self.table_start + self.base.byte_offset as usize) as u64,
            },
            stride_bytes: self.stride_bytes,
            public_layout: self.layout.clone(),
            bytes,
            resolver: Rc::clone(&self.resolver),
        })
    }

    /// Open the view addressed by a guest address.
    pub fn from_pointer(&self, address: GuestAddress) -> Result<RawEntityView, GuestError> {
        if address.byte_offset < self.base.byte_offset {
            return Err(GuestError::invalid("QVM pointer is outside the entity table"));
        }
        let displacement = address.byte_offset - self.base.byte_offset;
        if !displacement.is_multiple_of(self.stride_bytes as u64) {
            return Err(GuestError::invalid("QVM pointer is not an entity record boundary"));
        }
        self.at_slot((displacement / self.stride_bytes as u64) as usize)
    }
}

/// Capture a located source table.
pub fn qvm_raw_entity_table(
    module: &ModuleIdentity,
    data: &QvmGameData,
    memory: &QvmSharedMemory,
    capacity: usize,
    current_actor: QvmCurrentActor,
) -> Result<RawEntityTable, GuestError> {
    let first = data.entity_bytes(0)?;
    let stride_bytes = data.entity_stride_bytes();
    let start = first.offset;
    let base = if start == 0 { memory.len() } else { start };
    let count = data.num_entities();
    if capacity < count
        || stride_bytes < qvm_shared_entity_layout().byte_length
        || start + capacity.saturating_mul(stride_bytes) > memory.len()
    {
        return Err(GuestError::invalid("Invalid QVM entity table capacity or stride"));
    }
    Ok(RawEntityTable {
        module: module.clone(),
        base: GuestAddress {
            byte_offset: base as u64,
        },
        stride_bytes,
        count,
        capacity,
        layout: qvm_shared_entity_layout(),
        memory: memory.clone(),
        table_start: start,
        resolver: current_actor,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_core::identity::IdentityOwner;

    fn located() -> (ModuleIdentity, QvmGameData, QvmSharedMemory) {
        let memory = QvmSharedMemory::new(8192).unwrap();
        let data = QvmGameData::new(memory.clone(), super::super::game_data::AbiProfile::Modern);
        data.locate(64, 2, 560, 4096, 480).unwrap();
        let module = ModuleIdentity {
            id: "q3:qagame".to_string(),
            artifact_path: "qagame.qvm".to_string(),
            digest: "d".to_string(),
            revision: "r".to_string(),
        };
        (module, data, memory)
    }

    #[test]
    fn slots_snapshot_bytes_and_resolve_actors() {
        let (module, data, memory) = located();
        memory.write_i32(64, 1234).unwrap();
        let owner = IdentityOwner::create("test").unwrap();
        let actor = owner.actor(0, 1);
        let table = qvm_raw_entity_table(
            &module,
            &data,
            &memory,
            4,
            Rc::new(move |slot, _| (slot == 0).then(|| actor.clone())),
        )
        .unwrap();
        let view = table.at_slot(0).unwrap();
        assert_eq!(view.slot, 0);
        assert_eq!(i32::from_le_bytes(view.bytes[0..4].try_into().unwrap()), 1234);
        assert!(view.current_actor().is_some());
        assert!(table.at_slot(1).unwrap().current_actor().is_none());
        assert!(table.at_slot(4).is_err());
        memory.write_i32(64, 7).unwrap();
        assert_eq!(i32::from_le_bytes(view.bytes[0..4].try_into().unwrap()), 1234);
    }

    #[test]
    fn pointers_require_record_boundaries() {
        let (module, data, memory) = located();
        let table = qvm_raw_entity_table(&module, &data, &memory, 4, Rc::new(|_, _| None)).unwrap();
        let view = table.from_pointer(GuestAddress { byte_offset: 64 + 560 }).unwrap();
        assert_eq!(view.slot, 1);
        assert!(table.from_pointer(GuestAddress { byte_offset: 65 }).is_err());
        assert!(table.from_pointer(GuestAddress { byte_offset: 8 }).is_err());
    }

    #[test]
    fn layout_matches_donor_table() {
        let layout = qvm_shared_entity_layout();
        assert_eq!(layout.byte_length, 516);
        assert_eq!(layout.fields.len(), 15);
        assert_eq!(layout.fields[14].name, "r.ownerNum");
        assert_eq!(layout.fields[14].byte_offset, 512);
    }

    #[test]
    fn undersized_capacity_rejected() {
        let (module, data, memory) = located();
        assert!(qvm_raw_entity_table(&module, &data, &memory, 1, Rc::new(|_, _| None)).is_err());
    }
}
