//! Donor: `src/compat/q2/classic/records.ts` — guest strings, vectors, and
//! the export-table edict roster.
//!
//! Bridges the DLL-owned edict array (export table descriptor plus
//! source-stride records) to the shared `ActorRegistry`, borrowing complete
//! source-owned edicts without copying them.

use std::collections::HashSet;

use qa_core::identity::{ActorId, OwnedActor, ProviderId};
use qa_core::math::Vec3;
use qa_guest::core::contracts::{GuestAccess, GuestAddress};
use qa_guest::core::memory::SparseGuestMemory;
use qa_guest::runtime::common::memory::{allocate_native_memory, native_allocation_bytes};
use qa_world::registry::ActorRegistry;

use super::layout::{
    ClassicQ2Error, ClassicResult, CLASSIC_Q2_CLIENT_PREFIX_BYTES, CLASSIC_Q2_EDICT_BYTES, CLASSIC_Q2_EXPORT_BYTES,
};

/// Read a NUL-terminated guest string (null reads as empty).
pub fn read_classic_string(
    memory: &mut SparseGuestMemory,
    address: Option<GuestAddress>,
    maximum: usize,
) -> ClassicResult<String> {
    let Some(base) = address else {
        return Ok(String::new());
    };
    let mut text = String::new();
    for index in 0..maximum {
        let byte = memory.read_u8(memory.offset(base, index as i64)?)?;
        if byte == 0 {
            return Ok(text);
        }
        text.push(char::from(byte));
    }
    let offset = base.offset;
    Err(ClassicQ2Error::invalid(format!(
        "Unterminated API 3 string at 0x{offset:x}"
    )))
}

/// Write a NUL-terminated guest string into a `capacity`-byte field.
pub fn write_classic_string(
    memory: &mut SparseGuestMemory,
    address: GuestAddress,
    text: &str,
    capacity: usize,
) -> ClassicResult<()> {
    if text.len() + 1 > capacity || text.contains('\0') {
        return Err(ClassicQ2Error::invalid("API 3 string exceeds its guest allocation"));
    }
    let mut bytes = Vec::with_capacity(text.len() + 1);
    for ch in text.chars() {
        if ch as u32 > 255 {
            return Err(ClassicQ2Error::invalid("API 3 strings require source byte characters"));
        }
        bytes.push(ch as u8);
    }
    bytes.push(0);
    memory.write(address, &bytes)?;
    Ok(())
}

/// Allocate a guest string with native word-load tail rounding.
pub fn allocate_classic_string(memory: &mut SparseGuestMemory, text: &str) -> ClassicResult<GuestAddress> {
    let address = allocate_native_memory(memory, text.len() + 1, "API 3 string")?;
    write_classic_string(memory, address, text, text.len() + 1)?;
    Ok(address)
}

/// Backing bytes held by [`allocate_classic_string`].
pub fn classic_string_allocation_bytes(text: &str) -> ClassicResult<usize> {
    Ok(native_allocation_bytes(text.len() + 1)?)
}

/// Read three consecutive floats as a vector.
pub fn read_classic_vector(memory: &mut SparseGuestMemory, address: GuestAddress) -> ClassicResult<Vec3> {
    Ok(memory.read_f32x3(address)?)
}

/// Write a vector as three consecutive floats (no allocation).
pub fn write_classic_vector(memory: &mut SparseGuestMemory, address: GuestAddress, vector: Vec3) -> ClassicResult<()> {
    memory.write_f32(address, vector.x)?;
    memory.write_f32(memory.offset(address, 4)?, vector.y)?;
    memory.write_f32(memory.offset(address, 8)?, vector.z)?;
    Ok(())
}

/// Borrowed source edict: slot plus address in the DLL array.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RawEntityView {
    /// Edict slot.
    pub slot: u32,
    /// Record address.
    pub address: GuestAddress,
    /// Source stride in bytes.
    pub stride_bytes: usize,
}

/// Live export-table edict descriptor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClassicEdictDescriptor {
    /// Array base.
    pub base: GuestAddress,
    /// Source stride in bytes.
    pub stride: usize,
    /// Live count (`num_edicts`).
    pub count: usize,
    /// Array capacity (`max_edicts`).
    pub capacity: usize,
}

/// Component projection for callers that resolve actors externally.
pub trait ClassicQ2ActorProjection {
    /// Project a record to its actor, if any.
    fn project(&self, memory: &mut SparseGuestMemory, record: &RawEntityView) -> ClassicResult<Option<OwnedActor>>;
    /// Resolve an actor back to its record address.
    fn address(&self, actor: &ActorId) -> ClassicResult<GuestAddress>;
}

/// Outcome of observing one record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EdictObservation {
    /// A fresh actor was allocated; the host must bind its services and
    /// release the actor if that bind fails.
    Bound(OwnedActor),
    /// The record already has an actor.
    Existing(OwnedActor),
}

impl EdictObservation {
    /// The observed actor.
    #[must_use]
    pub fn actor(&self) -> &OwnedActor {
        match self {
            Self::Bound(actor) | Self::Existing(actor) => actor,
        }
    }
}

/// Reads the DLL's current export descriptor and borrows complete
/// source-owned edicts. The actor registry stays host-owned and is passed
/// in, mirroring the donor's shared `SessionActorRegistry`.
pub struct ClassicQ2Edicts {
    exports: GuestAddress,
    provider: ProviderId,
    retained: HashSet<u32>,
    retired: HashSet<u32>,
    projection: Option<Box<dyn ClassicQ2ActorProjection>>,
    descriptor: Option<ClassicEdictDescriptor>,
}

impl ClassicQ2Edicts {
    /// Bind to an export table returned by `GetGameAPI`.
    pub fn new(
        memory: &mut SparseGuestMemory,
        exports: GuestAddress,
        provider: ProviderId,
        projection: Option<Box<dyn ClassicQ2ActorProjection>>,
    ) -> ClassicResult<Self> {
        memory.check(exports, CLASSIC_Q2_EXPORT_BYTES, GuestAccess::Read)?;
        if memory.read_i32(exports)? != 3 {
            return Err(ClassicQ2Error::invalid(
                "GetGameAPI returned an API version other than 3",
            ));
        }
        Ok(Self {
            exports,
            provider,
            retained: HashSet::new(),
            retired: HashSet::new(),
            projection,
            descriptor: None,
        })
    }

    /// Owning provider.
    #[must_use]
    pub fn provider(&self) -> &ProviderId {
        &self.provider
    }

    /// Read (and cache) the live edict descriptor.
    pub fn descriptor(&mut self, memory: &mut SparseGuestMemory) -> ClassicResult<ClassicEdictDescriptor> {
        let base = memory.read_pointer(memory.offset(self.exports, 64)?)?;
        let stride = memory.read_i32(memory.offset(self.exports, 68)?)?;
        let count = memory.read_i32(memory.offset(self.exports, 72)?)?;
        let capacity = memory.read_i32(memory.offset(self.exports, 76)?)?;
        let Some(base) = base else {
            return Err(ClassicQ2Error::invalid(
                "API 3 edicts are not allocated; Init has not completed",
            ));
        };
        if stride < CLASSIC_Q2_EDICT_BYTES as i32
            || stride % 4 != 0
            || count < 0
            || capacity < count
            || capacity > 65536
        {
            return Err(ClassicQ2Error::invalid("Invalid source API 3 edict descriptor"));
        }
        let descriptor = ClassicEdictDescriptor {
            base,
            stride: stride as usize,
            count: count as usize,
            capacity: capacity as usize,
        };
        memory.check(base, descriptor.stride * descriptor.capacity, GuestAccess::Read)?;
        if self.descriptor != Some(descriptor) {
            self.descriptor = Some(descriptor);
        }
        Ok(descriptor)
    }

    /// Borrow the record at `slot`.
    pub fn at(&mut self, memory: &mut SparseGuestMemory, slot: u32) -> ClassicResult<RawEntityView> {
        let descriptor = self.descriptor(memory)?;
        if slot as usize >= descriptor.count {
            return Err(ClassicQ2Error::invalid("API 3 edict slot exceeds num_edicts"));
        }
        Ok(RawEntityView {
            slot,
            address: base_offset(descriptor, slot),
            stride_bytes: descriptor.stride,
        })
    }

    /// Borrow the record starting at `address`.
    pub fn record_from_pointer(
        &mut self,
        memory: &mut SparseGuestMemory,
        address: GuestAddress,
    ) -> ClassicResult<RawEntityView> {
        if address.space != memory.address_space() {
            return Err(ClassicQ2Error::invalid("Foreign guest edict address space"));
        }
        let descriptor = self.descriptor(memory)?;
        if address.offset < descriptor.base.offset {
            return Err(ClassicQ2Error::invalid(
                "Pointer does not identify the start of an API 3 edict",
            ));
        }
        let difference = address.offset - descriptor.base.offset;
        if difference % descriptor.stride as u64 != 0 {
            return Err(ClassicQ2Error::invalid(
                "Pointer does not identify the start of an API 3 edict",
            ));
        }
        self.at(memory, (difference / descriptor.stride as u64) as u32)
    }

    /// Resolve the live authority for a record without reviving a retired
    /// input slot or a record whose address moved.
    pub fn current(
        &mut self,
        memory: &mut SparseGuestMemory,
        registry: &ActorRegistry,
        record: &RawEntityView,
    ) -> ClassicResult<Option<OwnedActor>> {
        if self.retired.contains(&record.slot) {
            return Ok(None);
        }
        let current = self.at(memory, record.slot)?;
        if current.address != record.address {
            return Ok(None);
        }
        if read_inuse(memory, &current)? == 0 && !self.retained.contains(&record.slot) {
            return Ok(None);
        }
        Ok(registry.at_source(&self.provider, record.slot))
    }

    /// Resolve an actor back to its record address.
    pub fn pointer(
        &mut self,
        memory: &mut SparseGuestMemory,
        registry: &ActorRegistry,
        actor: &ActorId,
    ) -> ClassicResult<GuestAddress> {
        if let Some(projection) = &self.projection {
            return projection.address(actor);
        }
        let Some((provider, slot)) = registry.source_of(actor) else {
            return Err(ClassicQ2Error::invalid(
                "Foreign actor requires an explicit native semantic edict adapter",
            ));
        };
        if provider != self.provider {
            return Err(ClassicQ2Error::invalid(
                "Foreign actor requires an explicit native semantic edict adapter",
            ));
        }
        Ok(self.at(memory, slot)?.address)
    }

    /// Observe one record: release freed actors, allocate fresh ones.
    pub fn observe(
        &mut self,
        memory: &mut SparseGuestMemory,
        registry: &mut ActorRegistry,
        address: GuestAddress,
    ) -> ClassicResult<Option<EdictObservation>> {
        let record = self.record_from_pointer(memory, address)?;
        if self.retired.contains(&record.slot) {
            return Ok(None);
        }
        if let Some(projection) = &self.projection {
            return Ok(projection.project(memory, &record)?.map(EdictObservation::Existing));
        }
        let existing = registry.at_source(&self.provider, record.slot);
        if read_inuse(memory, &record)? == 0 && !self.retained.contains(&record.slot) {
            if let Some(actor) = existing {
                registry.release(&actor)?;
            }
            return Ok(None);
        }
        if let Some(actor) = existing {
            return Ok(Some(EdictObservation::Existing(actor)));
        }
        let actor = registry.allocate_at_source(self.provider.clone(), record.slot, "q2-native:edict")?;
        Ok(Some(EdictObservation::Bound(actor)))
    }

    /// Retain a client slot across freed `inuse` transitions.
    pub fn retain_client(
        &mut self,
        memory: &mut SparseGuestMemory,
        registry: &mut ActorRegistry,
        slot: u32,
    ) -> ClassicResult<OwnedActor> {
        if slot < 1 {
            return Err(ClassicQ2Error::invalid("Invalid retained API 3 client slot"));
        }
        let record = self.at(memory, slot)?;
        self.retained.insert(slot);
        match self.observe(memory, registry, record.address)? {
            Some(observation) => Ok(observation.actor().clone()),
            None => Err(ClassicQ2Error::invalid("Retained source client has no actor")),
        }
    }

    /// Drop a client retention, releasing freed actors.
    pub fn release_client(
        &mut self,
        memory: &mut SparseGuestMemory,
        registry: &mut ActorRegistry,
        slot: u32,
    ) -> ClassicResult<()> {
        self.retained.remove(&slot);
        let record = self.at(memory, slot)?;
        self.observe(memory, registry, record.address)?;
        Ok(())
    }

    /// Release actors past `num_edicts`, observe every live slot, and report
    /// freshly bound actors so the host can bind their services.
    pub fn reconcile(
        &mut self,
        memory: &mut SparseGuestMemory,
        registry: &mut ActorRegistry,
    ) -> ClassicResult<Vec<OwnedActor>> {
        if self.projection.is_some() {
            return Ok(Vec::new());
        }
        let descriptor = self.descriptor(memory)?;
        for actor in registry.owned_by(&self.provider) {
            if let Some((_, slot)) = registry.source_of(actor.id()) {
                if slot as usize >= descriptor.count {
                    registry.release(&actor)?;
                }
            }
        }
        let mut bound = Vec::new();
        for slot in 0..descriptor.count as u32 {
            let record = self.at(memory, slot)?;
            if let Some(EdictObservation::Bound(actor)) = self.observe(memory, registry, record.address)? {
                bound.push(actor);
            }
        }
        Ok(bound)
    }

    /// Mark an input client slot retired so primary callers stop resolving it.
    pub fn retire_input_client(&mut self, slot: u32) {
        self.retired.insert(slot);
    }

    /// Clear an input-client retirement.
    pub fn finish_input_retirement(&mut self, slot: u32) {
        self.retired.remove(&slot);
    }

    /// Address of a slot's client prefix, if the record has a client.
    pub fn client_prefix_address(
        &mut self,
        memory: &mut SparseGuestMemory,
        slot: u32,
    ) -> ClassicResult<Option<GuestAddress>> {
        let record = self.at(memory, slot)?;
        let address = memory.read_pointer(memory.offset(record.address, 84)?)?;
        if let Some(address) = address {
            memory.check(address, CLASSIC_Q2_CLIENT_PREFIX_BYTES, GuestAccess::Read)?;
        }
        Ok(address)
    }

    /// Store the client ping word at prefix offset 184.
    pub fn set_client_ping(&mut self, memory: &mut SparseGuestMemory, slot: u32, ping: i32) -> ClassicResult<()> {
        match self.client_prefix_address(memory, slot)? {
            Some(address) => {
                memory.write_i32(memory.offset(address, 184)?, ping)?;
                Ok(())
            }
            None => Err(ClassicQ2Error::invalid("Edict has no API 3 client prefix")),
        }
    }
}

fn base_offset(descriptor: ClassicEdictDescriptor, slot: u32) -> GuestAddress {
    GuestAddress::new(
        descriptor.base.space,
        descriptor.base.offset + slot as u64 * descriptor.stride as u64,
    )
}

fn read_inuse(memory: &mut SparseGuestMemory, record: &RawEntityView) -> ClassicResult<i32> {
    Ok(memory.read_i32(memory.offset(record.address, 88)?)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_core::identity::{IdentityOwner, ProviderId};
    use qa_guest::core::contracts::{ContentDigest, ModuleIdentity};

    fn test_memory() -> SparseGuestMemory {
        SparseGuestMemory::new(
            ModuleIdentity::new(
                ProviderId::new("q2", "records-test"),
                "records",
                ContentDigest::new("sha256", "0"),
                "test",
            ),
            4,
            0x10000,
        )
        .unwrap()
    }

    fn test_registry() -> ActorRegistry {
        ActorRegistry::new(IdentityOwner::create("records-test").unwrap(), 64).unwrap()
    }

    fn export_table(memory: &mut SparseGuestMemory, stride: i32, count: i32) -> GuestAddress {
        let edicts = memory
            .allocate(&qa_guest::core::contracts::GuestAllocationOptions::bytes(
                stride as usize * count as usize,
            ))
            .unwrap();
        let exports = memory
            .allocate(&qa_guest::core::contracts::GuestAllocationOptions::bytes(
                CLASSIC_Q2_EXPORT_BYTES,
            ))
            .unwrap();
        memory.write_i32(exports, 3).unwrap();
        memory
            .write_pointer(memory.offset(exports, 64).unwrap(), Some(edicts))
            .unwrap();
        memory.write_i32(memory.offset(exports, 68).unwrap(), stride).unwrap();
        memory.write_i32(memory.offset(exports, 72).unwrap(), count).unwrap();
        memory.write_i32(memory.offset(exports, 76).unwrap(), count).unwrap();
        exports
    }

    #[test]
    fn strings_and_vectors_round_trip() {
        let mut memory = test_memory();
        let address = allocate_classic_string(&mut memory, "hello").unwrap();
        assert_eq!(read_classic_string(&mut memory, Some(address), 64).unwrap(), "hello");
        assert_eq!(read_classic_string(&mut memory, None, 64).unwrap(), "");
        assert_eq!(classic_string_allocation_bytes("hello").unwrap(), 4096);
        write_classic_string(&mut memory, address, "hi", 3).unwrap();
        assert_eq!(read_classic_string(&mut memory, Some(address), 64).unwrap(), "hi");
        assert!(write_classic_string(&mut memory, address, "toolong", 3).is_err());
        let vector = Vec3 {
            x: 1.0,
            y: -2.0,
            z: 3.5,
        };
        write_classic_vector(&mut memory, address, vector).unwrap();
        assert_eq!(read_classic_vector(&mut memory, address).unwrap(), vector);
    }

    #[test]
    fn edicts_bind_release_and_reconcile() {
        let mut memory = test_memory();
        let mut registry = test_registry();
        let provider = ProviderId::new("q2", "classic");
        let exports = export_table(&mut memory, 896, 4);
        let mut edicts = ClassicQ2Edicts::new(&mut memory, exports, provider.clone(), None).unwrap();
        let one = edicts.at(&mut memory, 1).unwrap();
        assert_eq!(one.slot, 1);
        assert_eq!(one.stride_bytes, 896);
        assert!(edicts
            .observe(&mut memory, &mut registry, one.address)
            .unwrap()
            .is_none());
        memory.write_i32(memory.offset(one.address, 88).unwrap(), 1).unwrap();
        let bound = edicts
            .observe(&mut memory, &mut registry, one.address)
            .unwrap()
            .unwrap();
        assert!(matches!(bound, EdictObservation::Bound(_)));
        let again = edicts
            .observe(&mut memory, &mut registry, one.address)
            .unwrap()
            .unwrap();
        assert!(matches!(again, EdictObservation::Existing(_)));
        assert_eq!(
            edicts.pointer(&mut memory, &registry, bound.actor().id()).unwrap(),
            one.address
        );
        memory.write_i32(memory.offset(one.address, 88).unwrap(), 0).unwrap();
        assert!(edicts
            .observe(&mut memory, &mut registry, one.address)
            .unwrap()
            .is_none());
        assert!(registry.at_source(&provider, 1).is_none());
        assert!(edicts
            .record_from_pointer(&mut memory, memory.offset(one.address, 1).unwrap())
            .is_err());
    }

    #[test]
    fn descriptor_validation_and_client_ping() {
        let mut memory = test_memory();
        let mut registry = test_registry();
        let exports = export_table(&mut memory, 896, 4);
        memory.write_i32(exports, 2).unwrap();
        assert!(ClassicQ2Edicts::new(&mut memory, exports, ProviderId::new("q2", "classic"), None).is_err());
        memory.write_i32(exports, 3).unwrap();
        let mut edicts = ClassicQ2Edicts::new(&mut memory, exports, ProviderId::new("q2", "classic"), None).unwrap();
        let descriptor = edicts.descriptor(&mut memory).unwrap();
        assert_eq!((descriptor.stride, descriptor.count, descriptor.capacity), (896, 4, 4));
        let two = edicts.at(&mut memory, 2).unwrap();
        memory.write_i32(memory.offset(two.address, 88).unwrap(), 1).unwrap();
        let client = memory
            .allocate(&qa_guest::core::contracts::GuestAllocationOptions::bytes(512))
            .unwrap();
        memory
            .write_pointer(memory.offset(two.address, 84).unwrap(), Some(client))
            .unwrap();
        edicts.set_client_ping(&mut memory, 2, 75).unwrap();
        assert_eq!(memory.read_i32(memory.offset(client, 184).unwrap()).unwrap(), 75);
        assert!(edicts.set_client_ping(&mut memory, 1, 9).is_err());
        let bound = edicts.reconcile(&mut memory, &mut registry).unwrap();
        assert_eq!(bound.len(), 1);
        assert_eq!(
            registry.source_of(bound[0].id()),
            Some((ProviderId::new("q2", "classic"), 2))
        );
        assert!(edicts.current(&mut memory, &registry, &two).unwrap().is_some());
        edicts.retire_input_client(2);
        assert!(edicts.current(&mut memory, &registry, &two).unwrap().is_none());
        edicts.finish_input_retirement(2);
        assert!(edicts.current(&mut memory, &registry, &two).unwrap().is_some());
    }
}
