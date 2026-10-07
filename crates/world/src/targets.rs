use crate::entities::EntityTable;
use qa_core::primitives::{EntityId, NameId};

pub struct TargetIndex {
    entries: Vec<(NameId, EntityId)>,
    revision: Option<u64>,
}

impl TargetIndex {
    pub fn new(table: &EntityTable) -> Self {
        Self {
            entries: Vec::with_capacity(table.capacity()),
            revision: None,
        }
    }

    /// The table and index share one map lifetime and its fixed capacity.
    pub fn refresh(&mut self, table: &EntityTable) -> bool {
        let revision = table.structural_revision();
        if self.revision == Some(revision) {
            return false;
        }
        self.entries.clear();
        self.entries.extend(table.active().filter_map(|id| {
            let name = table.columns.targetname(id.slot as usize);
            (name.0 != 0).then_some((name, id))
        }));
        self.entries
            .sort_unstable_by_key(|(name, id)| (name.0, id.slot));
        self.revision = Some(revision);
        true
    }

    pub fn find(&self, name: NameId) -> impl Iterator<Item = EntityId> + '_ {
        let begin = self.entries.partition_point(|(key, _)| key.0 < name.0);
        let end = self.entries.partition_point(|(key, _)| key.0 <= name.0);
        self.entries[begin..end].iter().map(|(_, id)| *id)
    }
}
