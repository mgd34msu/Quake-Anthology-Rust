//! Retained model records have their own identity, independent of name slots.
use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct ModelSourceId(u32);

#[derive(Clone)]
struct Record {
    name: NameId,
    state: ModelSourceFile,
}
#[derive(Clone, Default)]
pub(super) struct ModelSources {
    records: Vec<Record>,
    by_name: Vec<Option<ModelSourceId>>,
}
impl ModelSources {
    pub(super) fn find(&self, name: NameId) -> Option<ModelSourceId> {
        self.by_name.get(name.0 as usize).copied().flatten()
    }
    pub(super) fn name(&self, source: ModelSourceId) -> Option<NameId> {
        Some(self.records.get(source.0 as usize)?.name)
    }
    pub(super) fn get(&self, source: ModelSourceId) -> Option<&ModelSourceFile> {
        Some(&self.records.get(source.0 as usize)?.state)
    }
    pub(super) fn intern(&mut self, name: NameId) -> Result<ModelSourceId, FileError> {
        if let Some(source) = self.find(name) {
            return Ok(source);
        }
        let index = name.0 as usize;
        let source = ModelSourceId(
            u32::try_from(self.records.len())
                .map_err(|_| FileError::capacity("model source slots"))?,
        );
        self.records
            .try_reserve(1)
            .map_err(|_| FileError::capacity("model source slots"))?;
        self.by_name
            .try_reserve((index + 1).saturating_sub(self.by_name.len()))
            .map_err(|_| FileError::capacity("model source names"))?;
        if self.by_name.len() <= index {
            self.by_name.resize(index + 1, None);
        }
        self.records.push(Record {
            name,
            state: ModelSourceFile::Empty,
        });
        self.by_name[index] = Some(source);
        Ok(source)
    }
    pub(super) fn invalidate(&mut self, name: NameId) -> bool {
        let Some(source) = self.find(name) else {
            return false;
        };
        if !matches!(self[source], ModelSourceFile::Invalid { .. }) {
            return false;
        }
        self[source] = ModelSourceFile::Empty;
        true
    }
    pub(super) fn rollback(
        &mut self,
        previous: Self,
        mode: RegistrationRollback,
        retained: AssetCounts,
        retained_tags: usize,
    ) {
        if matches!(mode, RegistrationRollback::Retry) {
            *self = previous;
        } else {
            for (record, previous) in self.records.iter_mut().zip(previous.records) {
                if !matches!(previous.state, ModelSourceFile::Empty) {
                    record.state = previous.state;
                }
            }
            for record in &mut self.records {
                record.state.rollback_resources(retained, retained_tags);
            }
        }
    }
}
impl std::ops::Index<ModelSourceId> for ModelSources {
    type Output = ModelSourceFile;
    fn index(&self, source: ModelSourceId) -> &Self::Output {
        &self.records[source.0 as usize].state
    }
}
impl std::ops::IndexMut<ModelSourceId> for ModelSources {
    fn index_mut(&mut self, source: ModelSourceId) -> &mut Self::Output {
        &mut self.records[source.0 as usize].state
    }
}
