use crate::entities::EntityTable;
use qa_core::{
    names::{NameMatch, NameTable},
    primitives::{EntityId, NameId},
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RefreshStats {
    pub initial_builds: u64,
    pub changed_slots: u64,
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct TargetEntry {
    name: NameId,
    folded: NameId,
    id: EntityId,
}

pub struct TargetIndex {
    entries: Vec<(NameId, EntityId)>,
    folded: Vec<(NameId, EntityId)>,
    slots: Box<[Option<TargetEntry>]>,
    initialized: bool,
    stats: RefreshStats,
}

impl TargetIndex {
    pub fn new(table: &EntityTable) -> Self {
        Self {
            entries: Vec::with_capacity(table.capacity()),
            folded: Vec::with_capacity(table.capacity()),
            slots: vec![None; table.capacity()].into_boxed_slice(),
            initialized: false,
            stats: RefreshStats::default(),
        }
    }

    pub fn stats(&self) -> RefreshStats {
        self.stats
    }

    fn update_slot(&mut self, table: &EntityTable, names: &NameTable, slot: usize) {
        let next = table.id_at(slot).and_then(|id| {
            let name = table.columns.targetname(slot)?;
            Some(TargetEntry {
                name,
                folded: names.folded(name)?,
                id,
            })
        });
        if self.slots[slot] == next {
            return;
        }
        if let Some(old) = self.slots[slot] {
            remove(&mut self.entries, (old.name, old.id));
            remove(&mut self.folded, (old.folded, old.id));
        }
        if let Some(next) = next {
            insert(&mut self.entries, (next.name, next.id));
            insert(&mut self.folded, (next.folded, next.id));
        }
        self.slots[slot] = next;
    }

    /// One index consumes this table's coalesced named changes. The table,
    /// index and immutable names share one load lifetime and fixed capacity.
    pub fn refresh(&mut self, table: &mut EntityTable, names: &NameTable) -> bool {
        if self.slots.len() != table.capacity() {
            return false;
        }
        let mut refreshed = false;
        if !self.initialized {
            for id in table.active() {
                if table.columns.targetname(id.slot as usize).is_some() {
                    self.update_slot(table, names, id.slot as usize);
                }
            }
            self.initialized = true;
            self.stats.initial_builds = self.stats.initial_builds.saturating_add(1);
            refreshed = true;
        }
        let mut start = 0;
        while let Some(slot) = table.take_target_change(start) {
            self.update_slot(table, names, slot);
            self.stats.changed_slots = self.stats.changed_slots.saturating_add(1);
            start = slot + 1;
            refreshed = true;
        }
        refreshed
    }

    pub fn find(
        &self,
        name: NameId,
        matching: NameMatch,
        names: &NameTable,
    ) -> impl Iterator<Item = EntityId> + '_ {
        let (entries, name) = match matching {
            NameMatch::Exact => (&self.entries, Some(name)),
            NameMatch::Folded => (&self.folded, names.folded(name)),
        };
        let (begin, end) = name.map_or((0, 0), |name| {
            (
                entries.partition_point(|(key, _)| key.0 < name.0),
                entries.partition_point(|(key, _)| key.0 <= name.0),
            )
        });
        entries[begin..end].iter().map(|(_, id)| *id)
    }
}

fn remove(rows: &mut Vec<(NameId, EntityId)>, value: (NameId, EntityId)) {
    let key = (value.0.0, value.1.slot);
    let index = rows.partition_point(|(name, id)| (name.0, id.slot) < key);
    if rows.get(index) == Some(&value) {
        rows.remove(index);
    }
}

fn insert(rows: &mut Vec<(NameId, EntityId)>, value: (NameId, EntityId)) {
    let key = (value.0.0, value.1.slot);
    let index = rows.partition_point(|(name, id)| (name.0, id.slot) < key);
    rows.insert(index, value);
}
