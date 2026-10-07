use qa_core::primitives::{Body, CallbackId, EntityId, ModuleId, NameId, Think, Vec3};

pub const MAX_ENTITIES: usize = 8192;

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
    freed_at: Box<[f64]>,
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
            freed_at: vec![f64::NEG_INFINITY; capacity].into_boxed_slice(),
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

    /// ED_Alloc: lowest eligible slot, early-map exception, then >0.5 seconds.
    pub fn allocate(&mut self, now: f64, owner: ModuleId) -> Option<EntityId> {
        for word in 0..self.free_bits.len() {
            let mut bits = self.free_bits[word];
            while bits != 0 {
                let bit = bits.trailing_zeros() as usize;
                bits &= bits - 1;
                let slot = word * 64 + bit;
                if self.freed_at[slot] >= 2.0 && now - self.freed_at[slot] <= 0.5 {
                    continue;
                }
                self.free_bits[word] &= !(1 << bit);
                self.live[slot] = true;
                self.live_count += 1;
                self.revision = self.revision.wrapping_add(1);
                self.columns.clear(slot);
                self.columns.owner[slot] = owner;
                return self.id_at(slot);
            }
        }
        None
    }

    pub fn release(&mut self, id: EntityId, now: f64) -> bool {
        let Some(slot) = self.resolve(id) else {
            return false;
        };
        if slot < self.reserved {
            return false;
        }
        self.live[slot] = false;
        self.live_count -= 1;
        self.revision = self.revision.wrapping_add(1);
        self.freed_at[slot] = now;
        self.columns.clear(slot);
        self.generations[slot] += 1;
        if self.generations[slot] != u32::MAX {
            self.free_bits[slot / 64] |= 1 << (slot % 64);
        }
        true
    }
}
