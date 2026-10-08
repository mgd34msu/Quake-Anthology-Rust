use crate::entities::EntityTable;
use qa_core::{
    names::{NameMatch, NameTable},
    primitives::{EntityId, NameId},
};

pub struct TargetIndex {
    entries: Vec<(NameId, EntityId)>,
    folded: Vec<(NameId, EntityId)>,
    revision: Option<u64>,
}

impl TargetIndex {
    pub fn new(table: &EntityTable) -> Self {
        Self {
            entries: Vec::with_capacity(table.capacity()),
            folded: Vec::with_capacity(table.capacity()),
            revision: None,
        }
    }

    /// The table and index share one map lifetime and its fixed capacity.
    pub fn refresh(&mut self, table: &EntityTable, names: &NameTable) -> bool {
        let revision = table.structural_revision();
        if self.revision == Some(revision) {
            return false;
        }
        self.entries.clear();
        self.folded.clear();
        self.entries.extend(table.active().filter_map(|id| {
            let name = table.columns.targetname(id.slot as usize)?;
            let key = names.folded(name)?;
            self.folded.push((key, id));
            Some((name, id))
        }));
        self.entries
            .sort_unstable_by_key(|(name, id)| (name.0, id.slot));
        self.folded
            .sort_unstable_by_key(|(name, id)| (name.0, id.slot));
        self.revision = Some(revision);
        true
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
