use qa_core::primitives::{Body, CallbackId, EntityId, ModuleId, NameId, Think, Vec3};

pub const MAX_ENTITIES: usize = 8192;

/// Native level time at the module boundary. Q3 keeps integer milliseconds;
/// Q1/QW/Q2 retain their original seconds and freetime precision.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum EntityTime {
    Seconds(f64),
    Milliseconds(i64),
}

impl From<f64> for EntityTime {
    fn from(value: f64) -> Self {
        Self::Seconds(value)
    }
}

impl EntityTime {
    fn seconds(self) -> f64 {
        match self {
            Self::Seconds(value) => value,
            Self::Milliseconds(value) => value as f64 / 1000.0,
        }
    }

    fn milliseconds(self) -> i64 {
        match self {
            Self::Milliseconds(value) => value,
            Self::Seconds(value) => (value * 1000.0).round() as i64,
        }
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

    fn eligible(self, now: EntityTime, freed: Option<EntityTime>) -> bool {
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

    fn freetime(self, now: EntityTime) -> EntityTime {
        match self {
            // Q1/QW/Q2 edict freetime is binary32, even when sv.time is double.
            Self::Seconds { .. } => EntityTime::Seconds(now.seconds() as f32 as f64),
            Self::Milliseconds { .. } => EntityTime::Milliseconds(now.milliseconds()),
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

pub struct EntityColumns {
    pub position: Box<[Vec3]>,
    pub velocity: Box<[Vec3]>,
    pub mins: Box<[Vec3]>,
    pub maxs: Box<[Vec3]>,
    pub angles: Box<[Vec3]>,
    pub next_think: Box<[Option<Think>]>,
    pub touch: Box<[Option<CallbackId>]>,
    pub use_fn: Box<[Option<CallbackId>]>,
    pub blocked: Box<[Option<CallbackId>]>,
    pub pain: Box<[Option<CallbackId>]>,
    pub die: Box<[Option<CallbackId>]>,
    pub owner: Box<[ModuleId]>,
    pub classname: Box<[NameId]>,
    targetname: Box<[NameId]>,
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
            touch: vec![None; capacity].into_boxed_slice(),
            use_fn: vec![None; capacity].into_boxed_slice(),
            blocked: vec![None; capacity].into_boxed_slice(),
            pain: vec![None; capacity].into_boxed_slice(),
            die: vec![None; capacity].into_boxed_slice(),
            owner: vec![ModuleId::default(); capacity].into_boxed_slice(),
            classname: vec![NameId::default(); capacity].into_boxed_slice(),
            targetname: vec![NameId::default(); capacity].into_boxed_slice(),
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
        self.touch[slot] = None;
        self.use_fn[slot] = None;
        self.blocked[slot] = None;
        self.pain[slot] = None;
        self.die[slot] = None;
        self.owner[slot] = ModuleId::default();
        self.classname[slot] = NameId::default();
        self.targetname[slot] = NameId::default();
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

    pub fn set_body(&mut self, slot: usize, body: Body) {
        self.position[slot] = body.position;
        self.velocity[slot] = body.velocity;
        self.mins[slot] = body.mins;
        self.maxs[slot] = body.maxs;
    }

    pub fn targetname(&self, slot: usize) -> NameId {
        self.targetname[slot]
    }
}

pub struct EntityTable {
    pub columns: EntityColumns,
    generations: Box<[u32]>,
    live: Box<[bool]>,
    freed_at: Box<[Option<EntityTime>]>,
    reuse: Box<[ReuseDelay]>,
    last_owner: Box<[ModuleId]>,
    never_free: Box<[bool]>,
    free_bits: Box<[u64]>,
    reserved: usize,
    live_count: usize,
    revision: u64,
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
        let mut live = vec![false; capacity].into_boxed_slice();
        live[..reserved].fill(true);
        let mut free_bits = vec![0u64; capacity.div_ceil(64)].into_boxed_slice();
        for slot in reserved..capacity {
            free_bits[slot / 64] |= 1 << (slot % 64);
        }
        Ok(Self {
            columns: EntityColumns::new(capacity),
            generations: vec![1; capacity].into_boxed_slice(),
            live,
            freed_at: vec![None; capacity].into_boxed_slice(),
            reuse: vec![ReuseDelay::EDICT; capacity].into_boxed_slice(),
            last_owner: vec![ModuleId::default(); capacity].into_boxed_slice(),
            never_free: vec![false; capacity].into_boxed_slice(),
            free_bits,
            reserved,
            live_count: reserved,
            revision: 0,
        })
    }

    pub fn capacity(&self) -> usize {
        self.live.len()
    }

    pub fn len(&self) -> usize {
        self.live_count
    }

    pub fn is_empty(&self) -> bool {
        self.live_count == 0
    }

    pub fn structural_revision(&self) -> u64 {
        self.revision
    }

    pub fn set_targetname(&mut self, id: EntityId, name: NameId) -> bool {
        let Some(slot) = self.resolve(id) else {
            return false;
        };
        if self.columns.targetname[slot] != name {
            self.columns.targetname[slot] = name;
            self.revision = self.revision.wrapping_add(1);
        }
        true
    }

    pub fn id_at(&self, slot: usize) -> Option<EntityId> {
        self.live
            .get(slot)
            .copied()
            .filter(|live| *live)
            .map(|_| EntityId {
                slot: slot as u32,
                generation: self.generations[slot],
            })
    }

    /// Resolve a lifetime handle once before accessing the hot columns.
    pub fn resolve(&self, id: EntityId) -> Option<usize> {
        let slot = id.slot as usize;
        self.live
            .get(slot)
            .copied()
            .filter(|live| *live)
            .filter(|_| self.generations[slot] == id.generation)
            .map(|_| slot)
    }

    pub fn active(&self) -> impl Iterator<Item = EntityId> + '_ {
        self.live.iter().enumerate().filter_map(|(slot, live)| {
            live.then_some(EntityId {
                slot: slot as u32,
                generation: self.generations[slot],
            })
        })
    }

    /// A disconnected client's slot stays reserved, but its lifetime ends.
    pub fn reset_client(&mut self, id: EntityId) -> Option<EntityId> {
        let slot = self
            .resolve(id)
            .filter(|slot| *slot > 0 && *slot < self.reserved)?;
        self.columns.clear(slot);
        self.revision = self.revision.wrapping_add(1);
        self.generations[slot] += 1;
        if self.generations[slot] == u32::MAX {
            self.live[slot] = false;
            self.live_count -= 1;
            return None;
        }
        self.id_at(slot)
    }

    /// Lowest eligible slot. Reuse follows the previous owning module's rule;
    /// the new lifetime receives the requesting module's load-chosen policy.
    pub fn allocate(
        &mut self,
        now: impl Into<EntityTime>,
        owner: ModuleId,
        policy: AllocationPolicy,
    ) -> Option<Allocation> {
        let now = now.into();
        for word in 0..self.free_bits.len() {
            let mut bits = self.free_bits[word];
            while bits != 0 {
                let bit = bits.trailing_zeros() as usize;
                bits &= bits - 1;
                let slot = word * 64 + bit;
                if !self.reuse[slot].eligible(now, self.freed_at[slot]) {
                    continue;
                }
                return self.claim(slot, owner, policy, None);
            }
        }
        if policy.full == FullTable::OverwriteLastOwned {
            if let Some(slot) = (self.reserved..self.capacity()).rev().find(|&slot| {
                self.last_owner[slot] == owner
                    && self.generations[slot] < u32::MAX
                    && (!self.live[slot] || self.generations[slot] < u32::MAX - 1)
                    && !self.never_free[slot]
                    && (self.live[slot] || self.freed_at[slot].is_some())
            }) {
                let displaced = self.id_at(slot);
                if displaced.is_some() {
                    self.generations[slot] += 1;
                }
                return self.claim(slot, owner, policy, displaced);
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
    ) -> Option<Allocation> {
        self.free_bits[slot / 64] &= !(1 << (slot % 64));
        if !self.live[slot] {
            self.live_count += 1;
        }
        self.live[slot] = true;
        self.revision = self.revision.wrapping_add(1);
        self.columns.clear(slot);
        self.columns.owner[slot] = owner;
        self.last_owner[slot] = owner;
        self.reuse[slot] = policy.reuse;
        self.never_free[slot] = false;
        Some(Allocation {
            id: self.id_at(slot)?,
            displaced,
        })
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

    pub fn release(&mut self, id: EntityId, now: impl Into<EntityTime>) -> bool {
        let Some(slot) = self.resolve(id) else {
            return false;
        };
        if slot < self.reserved || self.never_free[slot] {
            return false;
        }
        self.live[slot] = false;
        self.live_count -= 1;
        self.revision = self.revision.wrapping_add(1);
        self.freed_at[slot] = Some(self.reuse[slot].freetime(now.into()));
        self.columns.clear(slot);
        self.generations[slot] += 1;
        if self.generations[slot] != u32::MAX {
            self.free_bits[slot / 64] |= 1 << (slot % 64);
        }
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
            .allocate(3.0, ModuleId(1), AllocationPolicy::QUAKEWORLD)
            .ok_or("first")?
            .id;
        let last = table
            .allocate(3.0, ModuleId(1), AllocationPolicy::QUAKEWORLD)
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
        assert!(area.link(&table, dying, crate::area::LinkFlags::SOLID));
        let replacement = table
            .allocate(3.0, ModuleId(1), AllocationPolicy::QUAKEWORLD)
            .ok_or("replacement")?;
        assert_eq!(replacement.displaced, Some(first));
        assert!(table.resolve(dying).is_some());
        assert_eq!(table.len(), 3);
        assert!(area.unlink(dying));
        assert!(table.release(dying, 3.0));
        assert_eq!(table.generations[last.slot as usize], u32::MAX);
        assert!(table.release(replacement.id, 3.0));
        let current = table
            .allocate(4.0, ModuleId(1), AllocationPolicy::EDICT)
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
            .allocate(3.0, ModuleId(1), AllocationPolicy::QUAKEWORLD)
            .ok_or("entity")?
            .id;
        table.generations[id.slot as usize] = u32::MAX - 1;
        let current = table.id_at(id.slot as usize).ok_or("current")?;
        let revision = table.structural_revision();
        assert!(
            table
                .allocate(3.0, ModuleId(1), AllocationPolicy::QUAKEWORLD)
                .is_none()
        );
        assert!(table.resolve(current).is_some());
        assert_eq!(table.len(), 2);
        assert_eq!(table.structural_revision(), revision);
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
        assert!(
            table
                .allocate(5.0, ModuleId(1), AllocationPolicy::QUAKEWORLD)
                .is_none()
        );
        assert_eq!(table.len(), 1);
        Ok(())
    }
}
