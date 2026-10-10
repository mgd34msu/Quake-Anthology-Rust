use qa_core::primitives::{
    Body, BodyAttachment, CallbackId, CollisionOwner, CollisionShape, CollisionTags, EntityId,
    EntityPose, ModelRules, ModuleId, NameId, NativeEntity, ThinkTime, Vec3,
};

pub const MAX_ENTITIES: usize = 8192;

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

    fn eligible(self, now: ThinkTime, freed: Option<ThinkTime>) -> bool {
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

    fn freetime(self, now: ThinkTime) -> ThinkTime {
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
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AllocationPolicy {
    pub reuse: ReuseDelay,
    pub full: FullTable,
}

impl AllocationPolicy {
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
    pub position: Box<[Vec3]>,
    pub velocity: Box<[Vec3]>,
    pub mins: Box<[Vec3]>,
    pub maxs: Box<[Vec3]>,
    pub angles: Box<[Vec3]>,
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
    pub classname: Box<[NameId]>,
    targetname: Box<[Option<NameId>]>,
    attachment: Box<[Option<BodyAttachment>]>,
    pub flags: Box<[u32]>,
    pub model: Box<[u32]>,
    pub frame: Box<[u32]>,
    pub skin: Box<[i32]>,
    pub effects: Box<[u32]>,
}

impl EntityColumns {
    fn new(capacity: usize) -> Self {
        Self {
            position: vec![Vec3::default(); capacity].into_boxed_slice(),
            velocity: vec![Vec3::default(); capacity].into_boxed_slice(),
            mins: vec![Vec3::default(); capacity].into_boxed_slice(),
            maxs: vec![Vec3::default(); capacity].into_boxed_slice(),
            angles: vec![Vec3::default(); capacity].into_boxed_slice(),
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
            classname: vec![NameId::default(); capacity].into_boxed_slice(),
            targetname: vec![None; capacity].into_boxed_slice(),
            attachment: vec![None; capacity].into_boxed_slice(),
            flags: vec![0; capacity].into_boxed_slice(),
            model: vec![0; capacity].into_boxed_slice(),
            frame: vec![0; capacity].into_boxed_slice(),
            skin: vec![0; capacity].into_boxed_slice(),
            effects: vec![0; capacity].into_boxed_slice(),
        }
    }

    fn clear(&mut self, slot: usize) {
        self.position[slot] = Vec3::default();
        self.velocity[slot] = Vec3::default();
        self.mins[slot] = Vec3::default();
        self.maxs[slot] = Vec3::default();
        self.angles[slot] = Vec3::default();
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
        self.classname[slot] = NameId::default();
        self.targetname[slot] = None;
        self.attachment[slot] = None;
        self.flags[slot] = 0;
        self.model[slot] = 0;
        self.frame[slot] = 0;
        self.skin[slot] = 0;
        self.effects[slot] = 0;
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
        self.position[slot] = body.position;
        self.velocity[slot] = body.velocity;
        self.mins[slot] = body.mins;
        self.maxs[slot] = body.maxs;
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
    free_bits: Box<[u64]>,
    target_dirty: Box<[u64]>,
    target_dirty_count: usize,
    reserved: usize,
    live_count: usize,
    /// Fixed load capacity, ordered by first attachment insertion. Updating an
    /// attachment retains its position; detach and reattach moves it to the end.
    attachment_order: Vec<EntityId>,
}

fn next_bit(words: &[u64], start: usize, capacity: usize, inverse: bool) -> Option<usize> {
    if start >= capacity {
        return None;
    }
    let mut word = start / 64;
    let mut mask = u64::MAX << (start % 64);
    while word < words.len() {
        let bits = if inverse { !words[word] } else { words[word] } & mask;
        if bits != 0 {
            let slot = word * 64 + bits.trailing_zeros() as usize;
            return (slot < capacity).then_some(slot);
        }
        word += 1;
        mask = u64::MAX;
    }
    None
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
        let mut free_bits = vec![u64::MAX; capacity.div_ceil(64)].into_boxed_slice();
        for slot in 0..reserved {
            free_bits[slot / 64] &= !(1 << (slot % 64));
        }
        Ok(Self {
            columns: EntityColumns::new(capacity),
            generations: vec![1; capacity].into_boxed_slice(),
            freed_at: vec![None; capacity].into_boxed_slice(),
            reuse: vec![ReuseDelay::EDICT; capacity].into_boxed_slice(),
            last_owner: vec![ModuleId::default(); capacity].into_boxed_slice(),
            never_free: vec![false; capacity].into_boxed_slice(),
            free_bits,
            target_dirty: vec![0; capacity.div_ceil(64)].into_boxed_slice(),
            target_dirty_count: 0,
            reserved,
            live_count: reserved,
            attachment_order: Vec::with_capacity(capacity),
        })
    }

    pub fn capacity(&self) -> usize {
        self.generations.len()
    }

    pub fn len(&self) -> usize {
        self.live_count
    }

    pub fn is_empty(&self) -> bool {
        self.live_count == 0
    }

    fn mark_target_dirty(&mut self, slot: usize) {
        let word = slot / 64;
        let bit = 1 << (slot % 64);
        if self.target_dirty[word] & bit == 0 {
            self.target_dirty[word] |= bit;
            self.target_dirty_count += 1;
        }
    }

    pub(crate) fn take_target_change(&mut self, start: usize) -> Option<usize> {
        if self.target_dirty_count == 0 {
            return None;
        }
        let slot = next_bit(&self.target_dirty, start, self.capacity(), false)?;
        self.target_dirty[slot / 64] &= !(1 << (slot % 64));
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
        if !follow.offset.0.iter().all(|value| value.is_finite()) {
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

    pub(crate) fn attachment_count(&self) -> usize {
        self.attachment_order.len()
    }

    pub(crate) fn attachment_at(&self, index: usize) -> EntityId {
        self.attachment_order[index]
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
        (self.free_bits[slot / 64] & (1 << (slot % 64)) == 0).then_some(EntityId {
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
        let slot = next_bit(&self.free_bits, start, self.capacity(), true)?;
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

    /// A disconnected client's slot stays reserved, but its lifetime ends.
    pub fn reset_client(&mut self, id: EntityId) -> Option<EntityId> {
        let slot = self
            .resolve(id)
            .filter(|slot| *slot > 0 && *slot < self.reserved)?;
        self.clear_columns(slot);
        self.generations[slot] += 1;
        if self.generations[slot] == u32::MAX {
            self.free_bits[slot / 64] |= 1 << (slot % 64);
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
        let mut start = self.reserved;
        while let Some(slot) = next_bit(&self.free_bits, start, self.capacity(), false) {
            start = slot + 1;
            if self.generations[slot] == u32::MAX
                || !self.reuse[slot].eligible(now, self.freed_at[slot])
            {
                continue;
            }
            return Some(self.claim(slot, owner, policy, None));
        }
        if policy.full == FullTable::OverwriteLastOwned {
            if let Some(slot) = (self.reserved..self.capacity()).rev().find(|&slot| {
                self.last_owner[slot] == owner
                    && self.generations[slot] < u32::MAX
                    && (self.id_at(slot).is_none() || self.generations[slot] < u32::MAX - 1)
                    && !self.never_free[slot]
                    && (self.id_at(slot).is_some() || self.freed_at[slot].is_some())
            }) {
                let displaced = self.id_at(slot);
                if displaced.is_some() {
                    self.generations[slot] += 1;
                }
                return Some(self.claim(slot, owner, policy, displaced));
            }
        }
        None
    }

    fn claim(
        &mut self,
        slot: usize,
        owner: ModuleId,
        policy: AllocationPolicy,
        displaced: Option<EntityId>,
    ) -> Allocation {
        if self.free_bits[slot / 64] & (1 << (slot % 64)) != 0 {
            self.live_count += 1;
        }
        self.free_bits[slot / 64] &= !(1 << (slot % 64));
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

    pub fn release(&mut self, id: EntityId, now: ThinkTime) -> bool {
        let Some(slot) = self.resolve(id) else {
            return false;
        };
        if slot < self.reserved || self.never_free[slot] {
            return false;
        }
        self.free_bits[slot / 64] |= 1 << (slot % 64);
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
        assert_ne!(
            table.free_bits[last.slot as usize / 64] & (1 << (last.slot % 64)),
            0
        );
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
        assert_ne!(table.free_bits[0] & (1 << 1), 0);
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
