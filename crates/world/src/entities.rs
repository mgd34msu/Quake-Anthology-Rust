use qa_core::bits::BitSet;
use qa_core::primitives::{
    Body, BodyAttachment, CallbackId, CollisionOwner, CollisionShape, CollisionTags,
    EntityAppearance, EntityFields, EntityId, EntityPose, ModelRules, ModuleId, Motion, NameId,
    NativeEntity, NativeEntityRef, ResourceNamespace, ThinkTime, Vec3,
};

pub const MAX_ENTITIES: usize = 8192;

/// A verified canonical lifetime, with an optional weak native row identity.
/// Native ordinals can rebind after callbacks; anonymous references retain
/// their exact generation when their original lifetime ends.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EntityReference {
    entity: EntityId,
    reference: Option<NativeEntityRef>,
}

impl EntityReference {
    pub const fn entity(self) -> EntityId {
        self.entity
    }

    pub const fn reference(self) -> Option<NativeEntityRef> {
        self.reference
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ReuseDelay {
    Seconds { seconds: f64, grace_end: f64 },
    Milliseconds { milliseconds: i64, grace_end: i64 },
}

impl ReuseDelay {
    pub const EDICT: Self = Self::Seconds {
        seconds: 0.5,
        grace_end: 2.0,
    };

    pub fn q3(level_start_ms: i64) -> Self {
        Self::Milliseconds {
            milliseconds: 1000,
            grace_end: level_start_ms + 2000,
        }
    }

    /// Native namespaces use the same delay for their ABI ordinal metadata;
    /// common lifetime allocation remains owned by EntityTable.
    pub fn eligible(self, now: ThinkTime, freed: Option<ThinkTime>) -> bool {
        let Some(freed) = freed else { return true };
        match self {
            Self::Seconds { seconds, grace_end } => {
                freed.seconds() < grace_end || now.seconds() - freed.seconds() > seconds
            }
            Self::Milliseconds {
                milliseconds,
                grace_end,
            } => {
                freed.milliseconds() <= grace_end
                    || now.milliseconds() - freed.milliseconds() >= milliseconds
            }
        }
    }

    pub fn freetime(self, now: ThinkTime) -> ThinkTime {
        match self {
            // Q1/QW/Q2 edict freetime is binary32, even when sv.time is double.
            Self::Seconds { .. } => ThinkTime::Seconds(now.seconds() as f32 as f64),
            Self::Milliseconds { .. } => ThinkTime::Milliseconds(now.milliseconds()),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FullTable {
    Reject,
    /// QW's last-edict rule is confined to the requesting module's lifetimes.
    /// Foreign modules and protected slots are never displaced.
    OverwriteLastOwned,
    /// A native namespace has selected this exact ordinal for forced reuse.
    /// It may replace only this owner's current or immediately retired handle.
    OverwriteOwned(EntityId),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AllocationPolicy {
    pub reuse: ReuseDelay,
    pub full: FullTable,
}

impl AllocationPolicy {
    pub fn for_rules(rules: qa_core::primitives::RuleSetId, level_start_ms: i64) -> Self {
        match rules {
            qa_core::primitives::RuleSetId::QuakeWorld => Self::QUAKEWORLD,
            qa_core::primitives::RuleSetId::Quake3 => Self::q3(level_start_ms),
            _ => Self::EDICT,
        }
    }

    pub const EDICT: Self = Self {
        reuse: ReuseDelay::EDICT,
        full: FullTable::Reject,
    };
    pub const QUAKEWORLD: Self = Self {
        reuse: ReuseDelay::EDICT,
        full: FullTable::OverwriteLastOwned,
    };
    pub fn q3(level_start_ms: i64) -> Self {
        Self {
            reuse: ReuseDelay::q3(level_start_ms),
            full: FullTable::Reject,
        }
    }
}

/// The caller unlinks a displaced lifetime from spatial/module state. Its
/// generation is already invalid, so no query can observe the cleared row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Allocation {
    pub id: EntityId,
    pub displaced: Option<EntityId>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum TableError {
    Capacity,
    ReservedSlots,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AttachmentError {
    Entity,
    Anchor,
    Offset,
    Cycle,
}

pub struct EntityColumns {
    pub published_fields: Box<[EntityFields]>,
    pub position: Box<[Vec3]>,
    pub velocity: Box<[Vec3]>,
    pub mins: Box<[Vec3]>,
    pub maxs: Box<[Vec3]>,
    pub angles: Box<[Vec3]>,
    pub angular_velocity: Box<[Option<Vec3>]>,
    pub motion: Box<[Motion]>,
    pub next_think: Box<[Option<ThinkTime>]>,
    think_fn: Box<[qa_core::primitives::ThinkBinding]>,
    pub touch: Box<[Option<CallbackId>]>,
    pub use_fn: Box<[Option<CallbackId>]>,
    pub blocked: Box<[Option<CallbackId>]>,
    pub pain: Box<[Option<CallbackId>]>,
    pub die: Box<[Option<CallbackId>]>,
    pub owner: Box<[ModuleId]>,
    pub native_entity: Box<[Option<NativeEntity>]>,
    pub collision_owner: Box<[CollisionOwner]>,
    pub collision_shape: Box<[CollisionShape]>,
    pub model_rules: Box<[ModelRules]>,
    /// None shares the physical pose. A published module contents pose does
    /// not alter physical broadphase bounds or movement traces.
    pub point_contents_pose: Box<[Option<EntityPose>]>,
    /// Canonical Contents bits. Protocol adapters perform native conversion;
    /// the entity columns never narrow these to a legacy wire width.
    pub collision_contents: Box<[u64]>,
    pub collision_tags: Box<[CollisionTags]>,
    pub classname: Box<[Option<NameId>]>,
    pub target: Box<[Option<NameId>]>,
    targetname: Box<[Option<NameId>]>,
    attachment: Box<[Option<BodyAttachment>]>,
    pub flags: Box<[u32]>,
    pub appearance: Box<[EntityAppearance]>,
}

impl EntityColumns {
    fn new(capacity: usize) -> Self {
        Self {
            published_fields: vec![EntityFields::default(); capacity].into_boxed_slice(),
            position: vec![Vec3::default(); capacity].into_boxed_slice(),
            velocity: vec![Vec3::default(); capacity].into_boxed_slice(),
            mins: vec![Vec3::default(); capacity].into_boxed_slice(),
            maxs: vec![Vec3::default(); capacity].into_boxed_slice(),
            angles: vec![Vec3::default(); capacity].into_boxed_slice(),
            angular_velocity: vec![None; capacity].into_boxed_slice(),
            motion: vec![Motion::Unsupported; capacity].into_boxed_slice(),
            next_think: vec![None; capacity].into_boxed_slice(),
            think_fn: vec![qa_core::primitives::ThinkBinding::default(); capacity]
                .into_boxed_slice(),
            touch: vec![None; capacity].into_boxed_slice(),
            use_fn: vec![None; capacity].into_boxed_slice(),
            blocked: vec![None; capacity].into_boxed_slice(),
            pain: vec![None; capacity].into_boxed_slice(),
            die: vec![None; capacity].into_boxed_slice(),
            owner: vec![ModuleId::default(); capacity].into_boxed_slice(),
            native_entity: vec![None; capacity].into_boxed_slice(),
            collision_owner: vec![CollisionOwner::None; capacity].into_boxed_slice(),
            collision_shape: vec![CollisionShape::None; capacity].into_boxed_slice(),
            model_rules: vec![ModelRules::default(); capacity].into_boxed_slice(),
            point_contents_pose: vec![None; capacity].into_boxed_slice(),
            collision_contents: vec![0; capacity].into_boxed_slice(),
            collision_tags: vec![CollisionTags::default(); capacity].into_boxed_slice(),
            classname: vec![None; capacity].into_boxed_slice(),
            target: vec![None; capacity].into_boxed_slice(),
            targetname: vec![None; capacity].into_boxed_slice(),
            attachment: vec![None; capacity].into_boxed_slice(),
            flags: vec![0; capacity].into_boxed_slice(),
            appearance: vec![EntityAppearance::default(); capacity].into_boxed_slice(),
        }
    }

    fn clear(&mut self, slot: usize) {
        self.published_fields[slot] = EntityFields::default();
        self.position[slot] = Vec3::default();
        self.velocity[slot] = Vec3::default();
        self.mins[slot] = Vec3::default();
        self.maxs[slot] = Vec3::default();
        self.angles[slot] = Vec3::default();
        self.angular_velocity[slot] = None;
        self.motion[slot] = Motion::Unsupported;
        self.next_think[slot] = None;
        self.think_fn[slot] = qa_core::primitives::ThinkBinding::default();
        self.touch[slot] = None;
        self.use_fn[slot] = None;
        self.blocked[slot] = None;
        self.pain[slot] = None;
        self.die[slot] = None;
        self.owner[slot] = ModuleId::default();
        self.native_entity[slot] = None;
        self.collision_owner[slot] = CollisionOwner::None;
        self.collision_shape[slot] = CollisionShape::None;
        self.model_rules[slot] = ModelRules::default();
        self.point_contents_pose[slot] = None;
        self.collision_contents[slot] = 0;
        self.collision_tags[slot] = CollisionTags::default();
        self.classname[slot] = None;
        self.target[slot] = None;
        self.targetname[slot] = None;
        self.attachment[slot] = None;
        self.flags[slot] = 0;
        self.appearance[slot] = EntityAppearance::default();
    }

    pub fn body(&self, slot: usize) -> Body {
        Body {
            position: self.position[slot],
            velocity: self.velocity[slot],
            mins: self.mins[slot],
            maxs: self.maxs[slot],
        }
    }

    pub fn think_binding(&self, slot: usize) -> qa_core::primitives::ThinkBinding {
        self.think_fn[slot]
    }

    pub fn think_function(&self, slot: usize) -> Option<CallbackId> {
        self.think_fn[slot].callback
    }

    pub fn set_think_function(&mut self, slot: usize, binding: qa_core::primitives::ThinkBinding) {
        self.think_fn[slot] = binding;
    }

    pub fn set_body(&mut self, slot: usize, body: Body) {
        self.set_position(slot, body.position);
        self.set_velocity(slot, body.velocity);
        self.set_bounds(slot, body.mins, body.maxs);
    }

    pub fn set_position(&mut self, slot: usize, position: Vec3) {
        self.position[slot] = position;
        self.published_fields[slot].insert(EntityFields::POSITION);
    }

    pub fn set_velocity(&mut self, slot: usize, velocity: Vec3) {
        self.velocity[slot] = velocity;
        self.published_fields[slot].insert(EntityFields::VELOCITY);
    }

    pub fn set_angles(&mut self, slot: usize, angles: Vec3) {
        self.angles[slot] = angles;
        self.published_fields[slot].insert(EntityFields::ANGLES);
    }

    pub fn set_bounds(&mut self, slot: usize, mins: Vec3, maxs: Vec3) {
        self.mins[slot] = mins;
        self.maxs[slot] = maxs;
        self.published_fields[slot].insert(EntityFields::BOUNDS);
    }

    pub fn targetname(&self, slot: usize) -> Option<NameId> {
        self.targetname[slot]
    }
}

pub struct EntityTable {
    pub columns: EntityColumns,
    generations: Box<[u32]>,
    freed_at: Box<[Option<ThinkTime>]>,
    reuse: Box<[ReuseDelay]>,
    last_owner: Box<[ModuleId]>,
    never_free: Box<[bool]>,
    /// One means dead, including generation-retired slots and unused tail bits.
    free_bits: BitSet,
    target_dirty: BitSet,
    target_dirty_count: usize,
    reserved: usize,
    live_count: usize,
    /// Fixed load capacity, ordered by first attachment insertion. Updating an
    /// attachment retains its position; detach and reattach moves it to the end.
    attachment_order: Vec<EntityId>,
}

impl EntityTable {
    /// Reserve the world and client slots at load. Capacity never grows in play.
    pub fn new(capacity: usize, reserved: usize) -> Result<Self, TableError> {
        if capacity == 0 || capacity > MAX_ENTITIES {
            return Err(TableError::Capacity);
        }
        if reserved > capacity {
            return Err(TableError::ReservedSlots);
        }
        let mut free_bits = BitSet::filled(capacity);
        for slot in 0..reserved {
            free_bits.remove(slot);
        }
        Ok(Self {
            columns: EntityColumns::new(capacity),
            generations: vec![1; capacity].into_boxed_slice(),
            freed_at: vec![None; capacity].into_boxed_slice(),
            reuse: vec![ReuseDelay::EDICT; capacity].into_boxed_slice(),
            last_owner: vec![ModuleId::default(); capacity].into_boxed_slice(),
            never_free: vec![false; capacity].into_boxed_slice(),
            free_bits,
            target_dirty: BitSet::empty(capacity),
            target_dirty_count: 0,
            reserved,
            live_count: reserved,
            attachment_order: Vec::with_capacity(capacity),
        })
    }

    pub fn capacity(&self) -> usize {
        self.generations.len()
    }

    pub fn reserved_slots(&self) -> usize {
        self.reserved
    }

    fn client_index_at_slot(&self, slot: usize) -> Option<usize> {
        slot.checked_sub(1)
            .filter(|&index| index < self.reserved.saturating_sub(1))
    }

    /// The load-reserved common client index, never a native entity number.
    pub fn client_index(&self, id: EntityId) -> Option<usize> {
        self.client_index_at_slot(self.resolve(id)?)
    }

    /// Native flags may add player-like actors. The actual common clients
    /// retain their role even when another native namespace publishes them.
    pub fn collision_tags(&self, id: EntityId, native: CollisionTags) -> Option<CollisionTags> {
        let slot = self.resolve(id)?;
        Some(CollisionTags(
            native.0
                | if self.client_index_at_slot(slot).is_some() {
                    CollisionTags::PLAYER.0
                } else {
                    0
                },
        ))
    }

    pub fn len(&self) -> usize {
        self.live_count
    }

    pub fn is_empty(&self) -> bool {
        self.live_count == 0
    }

    fn mark_target_dirty(&mut self, slot: usize) {
        if self.target_dirty.insert(slot) {
            self.target_dirty_count += 1;
        }
    }

    pub(crate) fn take_target_change(&mut self, start: usize) -> Option<usize> {
        if self.target_dirty_count == 0 {
            return None;
        }
        let slot = self.target_dirty.next_set(start)?;
        self.target_dirty.remove(slot);
        self.target_dirty_count -= 1;
        Some(slot)
    }

    fn clear_columns(&mut self, slot: usize) {
        // Called for release, client reset and QW displacement as well as claim.
        // Clear direct children before the slot can acquire another lifetime.
        self.attachment_order.retain(|id| {
            let child = id.slot as usize;
            if child == slot
                || self.columns.attachment[child]
                    .is_some_and(|follow| follow.anchor.slot as usize == slot)
            {
                self.columns.attachment[child] = None;
                false
            } else {
                true
            }
        });
        if self.columns.targetname[slot].is_some() {
            self.mark_target_dirty(slot);
        }
        self.columns.clear(slot);
    }

    /// Match C body.c:389-406 without allocating an insertion record in play.
    pub fn attach(&mut self, id: EntityId, follow: BodyAttachment) -> Result<(), AttachmentError> {
        let slot = self.resolve(id).ok_or(AttachmentError::Entity)?;
        if !follow.offset.is_finite() {
            return Err(AttachmentError::Offset);
        }
        self.resolve(follow.anchor).ok_or(AttachmentError::Anchor)?;
        let mut anchor = Some(follow.anchor);
        for _ in 0..self.capacity() {
            let Some(current) = anchor else { break };
            if current == id {
                return Err(AttachmentError::Cycle);
            }
            anchor = self.attachment(current).map(|attachment| attachment.anchor);
        }
        if anchor.is_some() {
            return Err(AttachmentError::Cycle);
        }
        if self.columns.attachment[slot].is_none() {
            self.attachment_order.push(id);
        }
        self.columns.attachment[slot] = Some(follow);
        Ok(())
    }

    pub fn detach(&mut self, id: EntityId) -> bool {
        let Some(slot) = self.resolve(id) else {
            return false;
        };
        if self.columns.attachment[slot].take().is_none() {
            return false;
        }
        self.attachment_order.retain(|attached| *attached != id);
        true
    }

    pub fn attachment(&self, id: EntityId) -> Option<BodyAttachment> {
        self.columns.attachment[self.resolve(id)?]
    }

    pub fn attachments(&self) -> impl Iterator<Item = (EntityId, BodyAttachment)> + '_ {
        self.attachment_order
            .iter()
            .filter_map(|&id| self.attachment(id).map(|follow| (id, follow)))
    }

    /// Canonical attachment order, borrowed only until the next body mutation.
    pub fn attachment_ids(&self) -> &[EntityId] {
        &self.attachment_order
    }

    pub fn set_targetname(&mut self, id: EntityId, name: Option<NameId>) -> bool {
        let Some(slot) = self.resolve(id) else {
            return false;
        };
        if self.columns.targetname[slot] != name {
            self.columns.targetname[slot] = name;
            self.mark_target_dirty(slot);
        }
        true
    }

    pub fn id_at(&self, slot: usize) -> Option<EntityId> {
        let generation = *self.generations.get(slot)?;
        (!self.free_bits.contains(slot)).then_some(EntityId {
            slot: slot as u32,
            generation,
        })
    }

    /// Resolve a lifetime handle once before accessing the hot columns.
    pub fn resolve(&self, id: EntityId) -> Option<usize> {
        let slot = id.slot as usize;
        self.id_at(slot)
            .filter(|current| *current == id)
            .map(|_| slot)
    }

    /// Inclusive source-slot cursor. The returned handle owns no table borrow,
    /// so a callback can mutate lifetimes before the next ascending query.
    pub fn next_active(&self, start: usize) -> Option<EntityId> {
        let slot = self.free_bits.next_clear(start)?;
        Some(EntityId {
            slot: slot as u32,
            generation: self.generations[slot],
        })
    }

    pub fn active(&self) -> impl Iterator<Item = EntityId> + '_ {
        let mut start = 0;
        std::iter::from_fn(move || {
            let id = self.next_active(start)?;
            start = id.slot as usize + 1;
            Some(id)
        })
    }

    /// Native ordinals resolve only through live common lifetimes, in slot order.
    pub fn find_native(&self, native: NativeEntity, exclude: Option<EntityId>) -> Option<EntityId> {
        self.active().find(|id| {
            Some(*id) != exclude && self.columns.native_entity[id.slot as usize] == Some(native)
        })
    }

    pub fn capture_reference(&self, entity: EntityId) -> Option<EntityReference> {
        let slot = self.resolve(entity)?;
        let reference = match self.columns.native_entity[slot] {
            Some(native) => {
                if native.module != self.columns.owner[slot] {
                    return None;
                }
                let ordinal = u32::try_from(native.slot).ok()?;
                Some(NativeEntityRef::entity(
                    ResourceNamespace::Module(native.module),
                    ordinal,
                ))
            }
            None => None,
        };
        Some(EntityReference { entity, reference })
    }

    pub fn resolve_reference(&self, captured: EntityReference) -> Option<EntityId> {
        let Some(reference) = captured.reference else {
            return self.resolve(captured.entity).map(|_| captured.entity);
        };
        let ResourceNamespace::Module(module) = reference.namespace else {
            return None;
        };
        let slot = i32::try_from(reference.ordinal).ok()?;
        let current = self.find_native(NativeEntity { module, slot }, None)?;
        (self.columns.owner[current.slot as usize] == module).then_some(current)
    }

    /// A disconnected client's slot stays reserved, but its lifetime ends.
    pub fn reset_client(&mut self, id: EntityId) -> Option<EntityId> {
        let slot = self.client_index(id)? + 1;
        self.clear_columns(slot);
        self.generations[slot] += 1;
        if self.generations[slot] == u32::MAX {
            self.free_bits.insert(slot);
            self.live_count -= 1;
            return None;
        }
        self.id_at(slot)
    }

    /// Lowest eligible slot. Reuse follows the previous owning module's rule;
    /// the new lifetime receives the requesting module's load-chosen policy.
    #[expect(
        clippy::collapsible_if,
        reason = "Keep the full-table policy branch separate from its owned-slot eligibility search"
    )]
    pub fn allocate(
        &mut self,
        now: ThinkTime,
        owner: ModuleId,
        policy: AllocationPolicy,
    ) -> Option<Allocation> {
        if let FullTable::OverwriteOwned(id) = policy.full {
            let slot = id.slot as usize;
            if !self.replaceable(slot, owner)
                || !(self.id_at(slot) == Some(id)
                    || self.id_at(slot).is_none()
                        && id.generation.checked_add(1) == Some(self.generations[slot])
                        && self.freed_at[slot].is_some())
            {
                return None;
            }
            return Some(self.replace(slot, owner, policy));
        }
        let mut start = self.reserved;
        while let Some(slot) = self.free_bits.next_set(start) {
            start = slot + 1;
            if self.generations[slot] == u32::MAX
                || !self.reuse[slot].eligible(now, self.freed_at[slot])
            {
                continue;
            }
            return Some(self.claim(slot, owner, policy, None));
        }
        if policy.full == FullTable::OverwriteLastOwned {
            if let Some(slot) = (self.reserved..self.capacity())
                .rev()
                .find(|&slot| self.replaceable(slot, owner))
            {
                return Some(self.replace(slot, owner, policy));
            }
        }
        None
    }

    fn replaceable(&self, slot: usize, owner: ModuleId) -> bool {
        slot >= self.reserved
            && slot < self.capacity()
            && self.last_owner[slot] == owner
            && self.generations[slot] < u32::MAX
            && (self.id_at(slot).is_none() || self.generations[slot] < u32::MAX - 1)
            && !self.never_free[slot]
            && (self.id_at(slot).is_some() || self.freed_at[slot].is_some())
    }

    fn replace(&mut self, slot: usize, owner: ModuleId, policy: AllocationPolicy) -> Allocation {
        let displaced = self.id_at(slot);
        if displaced.is_some() {
            self.generations[slot] += 1;
        }
        self.claim(slot, owner, policy, displaced)
    }

    fn claim(
        &mut self,
        slot: usize,
        owner: ModuleId,
        policy: AllocationPolicy,
        displaced: Option<EntityId>,
    ) -> Allocation {
        if self.free_bits.remove(slot) {
            self.live_count += 1;
        }
        self.clear_columns(slot);
        self.columns.owner[slot] = owner;
        self.last_owner[slot] = owner;
        self.reuse[slot] = policy.reuse;
        self.never_free[slot] = false;
        Allocation {
            id: EntityId {
                slot: slot as u32,
                generation: self.generations[slot],
            },
            displaced,
        }
    }

    /// Q3 neverFree and Q2 body queues share lifetime protection. Prefix slots
    /// reserved at load remain protected independently of this flag.
    pub fn set_never_free(&mut self, id: EntityId, value: bool) -> bool {
        let Some(slot) = self.resolve(id) else {
            return false;
        };
        self.never_free[slot] = value;
        true
    }

    pub fn can_release(&self, id: EntityId) -> bool {
        self.resolve(id)
            .is_some_and(|slot| slot >= self.reserved && !self.never_free[slot])
    }

    pub fn release(&mut self, id: EntityId, now: ThinkTime) -> bool {
        if !self.can_release(id) {
            return false;
        }
        let slot = id.slot as usize;
        self.free_bits.insert(slot);
        self.live_count -= 1;
        self.freed_at[slot] = Some(self.reuse[slot].freetime(now));
        self.clear_columns(slot);
        self.generations[slot] += 1;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exhausted_generations_require_explicit_release_before_retirement() -> Result<(), &'static str>
    {
        let mut table = EntityTable::new(3, 1).map_err(|_| "table")?;
        let first = table
            .allocate(
                ThinkTime::Seconds(3.0),
                ModuleId(1),
                AllocationPolicy::QUAKEWORLD,
            )
            .ok_or("first")?
            .id;
        let last = table
            .allocate(
                ThinkTime::Seconds(3.0),
                ModuleId(1),
                AllocationPolicy::QUAKEWORLD,
            )
            .ok_or("last")?
            .id;
        table.generations[last.slot as usize] = u32::MAX - 1;
        let dying = table.id_at(last.slot as usize).ok_or("dying")?;
        let mut area = crate::area::AreaGrid::load(
            3,
            qa_core::primitives::Bounds {
                mins: Vec3([-100.0; 3]),
                maxs: Vec3([100.0; 3]),
            },
        )
        .map_err(|_| "area")?;
        assert!(area.link(
            &table,
            dying,
            crate::area::LinkFlags::SOLID,
            crate::area::LinkOrder::Tail,
            crate::area::LinkIntent::Explicit,
        ));
        let replacement = table
            .allocate(
                ThinkTime::Seconds(3.0),
                ModuleId(1),
                AllocationPolicy::QUAKEWORLD,
            )
            .ok_or("replacement")?;
        assert_eq!(replacement.displaced, Some(first));
        assert!(table.resolve(dying).is_some());
        assert_eq!(table.len(), 3);
        assert!(area.unlink(dying));
        assert!(table.release(dying, ThinkTime::Seconds(3.0)));
        assert_eq!(table.generations[last.slot as usize], u32::MAX);
        assert!(table.free_bits.contains(last.slot as usize));
        assert!(table.id_at(last.slot as usize).is_none());
        assert!(table.active().all(|id| id.slot != last.slot));
        assert!(table.release(replacement.id, ThinkTime::Seconds(3.0)));
        let current = table
            .allocate(
                ThinkTime::Seconds(4.0),
                ModuleId(1),
                AllocationPolicy::EDICT,
            )
            .ok_or("current")?
            .id;
        assert_eq!(current.slot, first.slot);
        Ok(())
    }

    #[test]
    fn full_table_does_not_silently_clear_an_unreportable_last_lifetime() -> Result<(), &'static str>
    {
        let mut table = EntityTable::new(2, 1).map_err(|_| "table")?;
        let id = table
            .allocate(
                ThinkTime::Seconds(3.0),
                ModuleId(1),
                AllocationPolicy::QUAKEWORLD,
            )
            .ok_or("entity")?
            .id;
        table.generations[id.slot as usize] = u32::MAX - 1;
        let current = table.id_at(id.slot as usize).ok_or("current")?;
        let body = table.columns.body(current.slot as usize);
        assert!(
            table
                .allocate(
                    ThinkTime::Seconds(3.0),
                    ModuleId(1),
                    AllocationPolicy::QUAKEWORLD
                )
                .is_none()
        );
        assert!(table.resolve(current).is_some());
        assert_eq!(table.len(), 2);
        let retained = table.columns.body(current.slot as usize);
        assert_eq!(retained.position, body.position);
        assert_eq!(retained.velocity, body.velocity);
        assert_eq!(retained.mins, body.mins);
        assert_eq!(retained.maxs, body.maxs);
        Ok(())
    }

    #[test]
    fn reserved_client_generation_exhaustion_never_returns_a_wrapped_handle()
    -> Result<(), &'static str> {
        let mut table = EntityTable::new(2, 2).map_err(|_| "table")?;
        table.generations[1] = u32::MAX - 1;
        let client = table.id_at(1).ok_or("client")?;
        assert!(table.reset_client(client).is_none());
        assert!(table.resolve(client).is_none());
        assert!(table.free_bits.contains(1));
        assert_eq!(table.active().map(|id| id.slot).collect::<Vec<_>>(), [0]);
        assert!(
            table
                .allocate(
                    ThinkTime::Seconds(5.0),
                    ModuleId(1),
                    AllocationPolicy::QUAKEWORLD
                )
                .is_none()
        );
        assert_eq!(table.len(), 1);
        Ok(())
    }
}
