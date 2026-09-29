//! Port of `src/compat/q2/native-mod-deferred.ts`.
//! Bridges deferred (batched) native damage: the original accumulator owns
//! batching while these records retain the originating canonical attack.

use std::collections::{HashMap, HashSet};

use qa_core::math::Vec3;
use qa_guest::core::contracts::{GuestAddress, GuestStorage};
use qa_guest::core::memory::SparseGuestMemory;
use qa_guest::error::GuestError;
use thiserror::Error;

/// Failures tracking deferred native damage.
#[derive(Debug, Clone, PartialEq, Error)]
pub enum DeferredError {
    /// A deferred pointer left the source table.
    #[error("native deferred damage pointer is outside its source table")]
    PointerOutsideTable,
    /// A receipt arrived without original attack provenance.
    #[error("native deferred damage has no original attack provenance")]
    MissingProvenance,
    /// A saved record names an actor this mod does not own.
    #[error("saved deferred damage target is not owned by this native mod")]
    ForeignTarget,
    /// A saved pointer exceeds the restored table.
    #[error("saved deferred damage pointer exceeds the restored source table")]
    SavedPointerRange,
    /// A saved target lost its source slot.
    #[error("saved deferred damage target lost its source slot")]
    LostSlot,
    /// Underlying guest failure.
    #[error("deferred damage guest failure: {0}")]
    Guest(String),
}

impl From<GuestError> for DeferredError {
    fn from(error: GuestError) -> Self {
        Self::Guest(error.to_string())
    }
}

/// Generational actor handle local to the deferred bridge.
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

/// Damage delivery mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Delivery {
    /// Direct hit.
    Direct,
    /// Radius damage.
    Radius,
}

/// Canonical damage request retained across the deferred batch.
#[derive(Debug, Clone, PartialEq)]
pub struct DamageRequest {
    /// Damage target.
    pub target: NativeActorId,
    /// Attacker, if any.
    pub attacker: Option<NativeActorId>,
    /// Inflictor, if any.
    pub inflictor: Option<NativeActorId>,
    /// Base damage.
    pub amount: f64,
    /// Knockback impulse.
    pub knockback: f64,
    /// Impact point.
    pub point: Vec3,
    /// Shot direction.
    pub direction: Vec3,
    /// Surface normal.
    pub normal: Vec3,
    /// Delivery mode.
    pub delivery: Delivery,
    /// Canonical cause label.
    pub cause: String,
}

/// Reaction the deferred batch resolves to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reaction {
    /// Pain reaction.
    Pain,
    /// Death reaction.
    Death,
}

/// Scalar field inside a source entity record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScalarField {
    /// Byte offset inside the record.
    pub offset: usize,
    /// Lane storage.
    pub storage: GuestStorage,
}

impl ScalarField {
    /// Byte width at a pointer width.
    #[must_use]
    pub fn width(&self, pointer_bytes: usize) -> usize {
        self.storage.byte_length(pointer_bytes)
    }
}

/// Declared deferred-damage accumulator layout.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeferredDamageDefinition {
    /// Attacker pointer offset.
    pub attacker: usize,
    /// Inflictor pointer offset.
    pub inflictor: usize,
    /// Accumulated blood field.
    pub blood: ScalarField,
    /// Accumulated knockback field.
    pub knockback: ScalarField,
    /// Impact point offset (12 bytes).
    pub point: usize,
    /// Three-byte cause record offset.
    pub mod_offset: usize,
    /// Receipt flag offset (1 byte).
    pub receipt: usize,
}

/// Entity-relative write range (offset inside one source record).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EntityWriteRange {
    /// Offset inside the record.
    pub byte_offset: usize,
    /// Written length in bytes.
    pub byte_length: usize,
}

/// Host surface the deferred state needs: entity table plus scalar access.
pub trait DeferredDamageHost {
    /// Base address of one entity slot.
    fn entity_address(&mut self, slot: usize) -> Result<GuestAddress, DeferredError>;
    /// Entity table slot count.
    fn entity_count(&self) -> usize;
    /// Guest pointer width in bytes.
    fn pointer_bytes(&self) -> usize;
    /// Owner of a slot, if bound.
    fn owner(&self, slot: usize) -> Option<NativeActorId>;
    /// Whether an actor is live.
    fn is_live(&self, actor: NativeActorId) -> bool;
    /// Source slot of an actor.
    fn source_slot(&self, actor: NativeActorId) -> Option<usize>;
    /// Read a scalar field.
    fn read_scalar(&mut self, slot: usize, field: &ScalarField) -> Result<f64, DeferredError>;
    /// Write a scalar field.
    fn write_scalar(&mut self, slot: usize, field: &ScalarField, value: f64) -> Result<(), DeferredError>;
    /// Read a pointer field as a table slot.
    fn pointer_slot(&mut self, slot: usize, offset: usize) -> Result<Option<usize>, DeferredError>;
    /// Write a pointer field from a table slot.
    fn write_pointer_slot(&mut self, slot: usize, offset: usize, target: Option<usize>) -> Result<(), DeferredError>;
    /// Read the impact point vector.
    fn read_point(&mut self, slot: usize, definition: &DeferredDamageDefinition) -> Result<Vec3, DeferredError>;
    /// Write the impact point vector.
    fn write_point(
        &mut self,
        slot: usize,
        definition: &DeferredDamageDefinition,
        point: Vec3,
    ) -> Result<(), DeferredError>;
    /// Read the three-byte cause record.
    fn read_cause(&mut self, slot: usize, definition: &DeferredDamageDefinition) -> Result<[u8; 3], DeferredError>;
    /// Write the three-byte cause record.
    fn write_cause(
        &mut self,
        slot: usize,
        definition: &DeferredDamageDefinition,
        cause: &[u8; 3],
    ) -> Result<(), DeferredError>;
    /// Read current health for reaction classification.
    fn read_health(&mut self, slot: usize, health: &ScalarField) -> Result<f64, DeferredError> {
        self.read_scalar(slot, health)
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

/// Memory-backed deferred host over a synthetic entity table.
pub struct SyntheticDeferredHost {
    /// Guest memory.
    pub memory: SparseGuestMemory,
    table_base: GuestAddress,
    stride: usize,
    count: usize,
    owners: HashMap<usize, NativeActorId>,
    live: HashSet<NativeActorId>,
}

impl SyntheticDeferredHost {
    /// Build a host over a mapped table region.
    pub fn new(memory: SparseGuestMemory, table_base: GuestAddress, stride: usize, count: usize) -> Self {
        Self {
            memory,
            table_base,
            stride,
            count,
            owners: HashMap::new(),
            live: HashSet::new(),
        }
    }

    /// Bind an owner to a slot and mark it live.
    pub fn bind(&mut self, slot: usize, actor: NativeActorId) {
        self.owners.insert(slot, actor);
        self.live.insert(actor);
    }

    fn at(&self, slot: usize, offset: usize) -> Result<GuestAddress, DeferredError> {
        let base = self.memory.offset(self.table_base, (slot * self.stride) as i64)?;
        Ok(self.memory.offset(base, offset as i64)?)
    }
}

impl DeferredDamageHost for SyntheticDeferredHost {
    fn entity_address(&mut self, slot: usize) -> Result<GuestAddress, DeferredError> {
        if slot >= self.count {
            return Err(DeferredError::PointerOutsideTable);
        }
        Ok(self.memory.offset(self.table_base, (slot * self.stride) as i64)?)
    }

    fn entity_count(&self) -> usize {
        self.count
    }

    fn pointer_bytes(&self) -> usize {
        self.memory.pointer_bytes()
    }

    fn owner(&self, slot: usize) -> Option<NativeActorId> {
        self.owners.get(&slot).copied()
    }

    fn is_live(&self, actor: NativeActorId) -> bool {
        self.live.contains(&actor)
    }

    fn source_slot(&self, actor: NativeActorId) -> Option<usize> {
        self.owners
            .iter()
            .find(|(_, bound)| **bound == actor)
            .map(|(slot, _)| *slot)
    }

    fn read_scalar(&mut self, slot: usize, field: &ScalarField) -> Result<f64, DeferredError> {
        let address = self.at(slot, field.offset)?;
        let width = field.width(self.memory.pointer_bytes());
        Ok(scalar_to_f64(&self.memory.copy(address, width)?, field.storage))
    }

    fn write_scalar(&mut self, slot: usize, field: &ScalarField, value: f64) -> Result<(), DeferredError> {
        let address = self.at(slot, field.offset)?;
        let bytes = f64_to_scalar(value, field.storage, self.memory.pointer_bytes());
        Ok(self.memory.write(address, &bytes)?)
    }

    fn pointer_slot(&mut self, slot: usize, offset: usize) -> Result<Option<usize>, DeferredError> {
        let address = self.at(slot, offset)?;
        let Some(target) = self.memory.read_pointer(address)? else {
            return Ok(None);
        };
        if target.offset < self.table_base.offset {
            return Err(DeferredError::PointerOutsideTable);
        }
        let relative = target.offset - self.table_base.offset;
        if !relative.is_multiple_of(self.stride as u64) || relative / self.stride as u64 >= self.count as u64 {
            return Err(DeferredError::PointerOutsideTable);
        }
        Ok(Some((relative / self.stride as u64) as usize))
    }

    fn write_pointer_slot(&mut self, slot: usize, offset: usize, target: Option<usize>) -> Result<(), DeferredError> {
        let address = self.at(slot, offset)?;
        let resolved = match target {
            None => None,
            Some(slot) => Some(self.memory.offset(self.table_base, (slot * self.stride) as i64)?),
        };
        Ok(self.memory.write_pointer(address, resolved)?)
    }

    fn read_point(&mut self, slot: usize, definition: &DeferredDamageDefinition) -> Result<Vec3, DeferredError> {
        Ok(self.memory.read_f32x3(self.at(slot, definition.point)?)?)
    }

    fn write_point(
        &mut self,
        slot: usize,
        definition: &DeferredDamageDefinition,
        point: Vec3,
    ) -> Result<(), DeferredError> {
        let address = self.at(slot, definition.point)?;
        let mut bytes = Vec::with_capacity(12);
        for component in [point.x, point.y, point.z] {
            bytes.extend_from_slice(&component.to_le_bytes());
        }
        Ok(self.memory.write(address, &bytes)?)
    }

    fn read_cause(&mut self, slot: usize, definition: &DeferredDamageDefinition) -> Result<[u8; 3], DeferredError> {
        let bytes = self.memory.copy(self.at(slot, definition.mod_offset)?, 3)?;
        Ok([bytes[0], bytes[1], bytes[2]])
    }

    fn write_cause(
        &mut self,
        slot: usize,
        definition: &DeferredDamageDefinition,
        cause: &[u8; 3],
    ) -> Result<(), DeferredError> {
        Ok(self.memory.write(self.at(slot, definition.mod_offset)?, cause)?)
    }
}

/// Saved deferred-damage record.
#[derive(Debug, Clone, PartialEq)]
pub struct SavedNativeDeferredDamage {
    /// Damage target.
    pub target: SavedActorId,
    /// Retained canonical request.
    pub request: DamageRequest,
    /// Accumulated blood.
    pub blood: f64,
    /// Accumulated knockback.
    pub knockback: f64,
    /// Impact point.
    pub point: Vec3,
    /// Cause record bytes.
    pub cause_bytes: [u8; 3],
    /// Attacker table slot.
    pub attacker_slot: Option<usize>,
    /// Inflictor table slot.
    pub inflictor_slot: Option<usize>,
}

/// Tracks pending deferred batches per actor.
#[derive(Debug, Default)]
pub struct NativeModDeferredDamageState {
    pending: HashMap<NativeActorId, DamageRequest>,
    processing: Vec<DamageRequest>,
}

impl NativeModDeferredDamageState {
    /// Build empty deferred state.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// React to a write inside one entity record.
    pub fn changed(
        &mut self,
        host: &mut impl DeferredDamageHost,
        definition: &DeferredDamageDefinition,
        slot: usize,
        range: EntityWriteRange,
        current: Option<&DamageRequest>,
    ) -> Result<(), DeferredError> {
        let actor = host.owner(slot);
        let Some(actor) = actor else { return Ok(()) };
        let hits = |offset: usize, bytes: usize| {
            range.byte_offset < offset + bytes && offset < range.byte_offset + range.byte_length
        };
        let pointer_bytes = host.pointer_bytes();
        if hits(definition.receipt, 1) && host.read_scalar(slot, &definition.blood)? != 0.0 {
            let Some(request) = current else {
                return Err(DeferredError::MissingProvenance);
            };
            if request.target != actor {
                return Err(DeferredError::MissingProvenance);
            }
            self.pending.insert(actor, request.clone());
        }
        if hits(definition.blood.offset, definition.blood.width(pointer_bytes))
            && host.read_scalar(slot, &definition.blood)? == 0.0
        {
            self.pending.remove(&actor);
        }
        Ok(())
    }

    /// Request currently processed for an actor, if any.
    #[must_use]
    pub fn attack(&self, actor: NativeActorId) -> Option<&DamageRequest> {
        self.processing.last().filter(|request| request.target == actor)
    }

    /// Run the original deferred processor, publishing the source reaction.
    // Seven parameters mirror the donor `process` signature one-to-one;
    // bundling them would hide the call-site argument order the donor fixes.
    #[allow(clippy::too_many_arguments)]
    pub fn process<R>(
        &mut self,
        host: &mut impl DeferredDamageHost,
        definition: &DeferredDamageDefinition,
        health: &ScalarField,
        actor: NativeActorId,
        slot: usize,
        react: &mut dyn FnMut(&DamageRequest, Reaction),
        original: impl FnOnce() -> R,
    ) -> Result<R, DeferredError> {
        let pending = self.pending.get(&actor).cloned();
        let (Some(stored), blood) = (pending, host.read_scalar(slot, &definition.blood)?) else {
            return Ok(original());
        };
        if blood == 0.0 {
            return Ok(original());
        }
        self.pending.remove(&actor);
        let request = DamageRequest {
            knockback: host.read_scalar(slot, &definition.knockback)?,
            point: host.read_point(slot, definition)?,
            ..stored
        };
        self.processing.push(request.clone());
        let reaction = if host.read_health(slot, health)? <= 0.0 {
            Reaction::Death
        } else {
            Reaction::Pain
        };
        react(&request, reaction);
        let result = original();
        self.processing.pop();
        Ok(result)
    }

    /// Drop pending state for a released actor.
    pub fn release(&mut self, actor: NativeActorId) {
        self.pending.remove(&actor);
    }

    /// Capture pending batches.
    pub fn checkpoint(
        &self,
        host: &mut impl DeferredDamageHost,
        definition: &DeferredDamageDefinition,
    ) -> Result<Vec<SavedNativeDeferredDamage>, DeferredError> {
        let mut records = Vec::new();
        for (actor, request) in &self.pending {
            let Some(slot) = host.source_slot(*actor) else { continue };
            if !host.is_live(*actor) {
                continue;
            }
            records.push(SavedNativeDeferredDamage {
                target: SavedActorId::from(*actor),
                request: request.clone(),
                blood: host.read_scalar(slot, &definition.blood)?,
                knockback: host.read_scalar(slot, &definition.knockback)?,
                point: host.read_point(slot, definition)?,
                cause_bytes: host.read_cause(slot, definition)?,
                attacker_slot: host.pointer_slot(slot, definition.attacker)?,
                inflictor_slot: host.pointer_slot(slot, definition.inflictor)?,
            });
        }
        Ok(records)
    }

    /// Validate saved records against live ownership.
    pub fn validate_saved(
        &self,
        host: &impl DeferredDamageHost,
        records: &[SavedNativeDeferredDamage],
    ) -> Result<(), DeferredError> {
        for record in records {
            let target = NativeActorId {
                slot: record.target.slot,
                generation: record.target.generation,
            };
            if host.source_slot(target).is_none() || !host.is_live(target) {
                return Err(DeferredError::ForeignTarget);
            }
        }
        Ok(())
    }

    /// Restore pending batches into the accumulator words.
    pub fn restore(
        &mut self,
        host: &mut impl DeferredDamageHost,
        definition: &DeferredDamageDefinition,
        records: &[SavedNativeDeferredDamage],
    ) -> Result<(), DeferredError> {
        for record in records {
            for slot in [record.attacker_slot, record.inflictor_slot] {
                if slot.is_some_and(|slot| slot >= host.entity_count()) {
                    return Err(DeferredError::SavedPointerRange);
                }
            }
        }
        self.validate_saved(host, records)?;
        self.pending.clear();
        for record in records {
            let target = NativeActorId {
                slot: record.target.slot,
                generation: record.target.generation,
            };
            let Some(slot) = host.source_slot(target) else {
                return Err(DeferredError::LostSlot);
            };
            host.write_pointer_slot(slot, definition.attacker, record.attacker_slot)?;
            host.write_pointer_slot(slot, definition.inflictor, record.inflictor_slot)?;
            host.write_scalar(slot, &definition.blood, record.blood)?;
            host.write_scalar(slot, &definition.knockback, record.knockback)?;
            host.write_point(slot, definition, record.point)?;
            host.write_cause(slot, definition, &record.cause_bytes)?;
            self.pending.insert(target, record.request.clone());
        }
        Ok(())
    }

    /// Drop all pending batches.
    pub fn clear(&mut self) {
        self.pending.clear();
        self.processing.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_core::identity::ProviderId;
    use qa_guest::core::contracts::{ContentDigest, GuestMapOptions, GuestPermissions, ModuleIdentity};

    const STRIDE: usize = 128;

    fn test_host() -> (SyntheticDeferredHost, DeferredDamageDefinition, ScalarField) {
        let module = ModuleIdentity::new(
            ProviderId::new("test", "deferred"),
            "deferred.so",
            ContentDigest {
                algorithm: "none".to_string(),
                value: "0".to_string(),
            },
            "r1",
        );
        let mut memory = SparseGuestMemory::new(module, 4, 0x90000).unwrap();
        let table_base = memory
            .map(&GuestMapOptions::new(0x20000, STRIDE * 8, GuestPermissions::ReadWrite))
            .unwrap();
        let mut host = SyntheticDeferredHost::new(memory, table_base, STRIDE, 8);
        host.bind(2, NativeActorId { slot: 2, generation: 1 });
        host.bind(3, NativeActorId { slot: 3, generation: 1 });
        let definition = DeferredDamageDefinition {
            attacker: 0,
            inflictor: 4,
            blood: ScalarField {
                offset: 8,
                storage: GuestStorage::Int32,
            },
            knockback: ScalarField {
                offset: 12,
                storage: GuestStorage::Int32,
            },
            point: 16,
            mod_offset: 28,
            receipt: 31,
        };
        let health = ScalarField {
            offset: 32,
            storage: GuestStorage::Int32,
        };
        (host, definition, health)
    }

    fn request() -> DamageRequest {
        DamageRequest {
            target: NativeActorId { slot: 2, generation: 1 },
            attacker: Some(NativeActorId { slot: 3, generation: 1 }),
            inflictor: None,
            amount: 25.0,
            knockback: 100.0,
            point: Vec3 { x: 1.0, y: 2.0, z: 3.0 },
            direction: Vec3 { x: 0.0, y: 0.0, z: 1.0 },
            normal: Vec3 { x: 0.0, y: 1.0, z: 0.0 },
            delivery: Delivery::Direct,
            cause: "q2:blaster".to_string(),
        }
    }

    #[test]
    fn receipt_captures_pending_and_process_publishes_reaction() {
        let (mut host, definition, health) = test_host();
        let mut state = NativeModDeferredDamageState::new();
        host.write_scalar(2, &definition.blood, 25.0).unwrap();
        host.write_scalar(2, &health, 40.0).unwrap();
        let incoming = request();
        state
            .changed(
                &mut host,
                &definition,
                2,
                EntityWriteRange {
                    byte_offset: 31,
                    byte_length: 1,
                },
                Some(&incoming),
            )
            .unwrap();
        assert!(state.attack(NativeActorId { slot: 2, generation: 1 }).is_none());
        let mut reactions = Vec::new();
        let processed = state
            .process(
                &mut host,
                &definition,
                &health,
                NativeActorId { slot: 2, generation: 1 },
                2,
                &mut |request, reaction| reactions.push((request.amount, reaction)),
                || "original-ran",
            )
            .unwrap();
        assert_eq!(processed, "original-ran");
        assert_eq!(reactions, vec![(25.0, Reaction::Pain)]);
        host.write_scalar(2, &definition.blood, 0.0).unwrap();
        state
            .changed(
                &mut host,
                &definition,
                2,
                EntityWriteRange {
                    byte_offset: 8,
                    byte_length: 4,
                },
                None,
            )
            .unwrap();
    }

    #[test]
    fn receipt_without_provenance_fails_and_checkpoint_roundtrips() {
        let (mut host, definition, health) = test_host();
        let mut state = NativeModDeferredDamageState::new();
        host.write_scalar(2, &definition.blood, 10.0).unwrap();
        let missing = state.changed(
            &mut host,
            &definition,
            2,
            EntityWriteRange {
                byte_offset: 31,
                byte_length: 1,
            },
            None,
        );
        assert_eq!(missing, Err(DeferredError::MissingProvenance));

        let incoming = request();
        state
            .changed(
                &mut host,
                &definition,
                2,
                EntityWriteRange {
                    byte_offset: 31,
                    byte_length: 1,
                },
                Some(&incoming),
            )
            .unwrap();
        host.write_pointer_slot(2, definition.attacker, Some(3)).unwrap();
        host.write_cause(2, &definition, &[9, 1, 0]).unwrap();
        host.write_scalar(2, &definition.knockback, 120.0).unwrap();
        let saved = state.checkpoint(&mut host, &definition).unwrap();
        assert_eq!(saved.len(), 1);
        assert_eq!(saved[0].attacker_slot, Some(3));
        assert_eq!(saved[0].cause_bytes, [9, 1, 0]);
        assert_eq!(saved[0].knockback, 120.0);
        state.clear();
        state.restore(&mut host, &definition, &saved).unwrap();
        assert_eq!(host.read_scalar(2, &definition.blood).unwrap(), 10.0);
        assert_eq!(host.pointer_slot(2, definition.attacker).unwrap(), Some(3));
        let _ = health;
    }
}
