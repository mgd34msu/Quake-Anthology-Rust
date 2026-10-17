//! Retained model records have their own identity, independent of name slots.
use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct ModelSourceId(u32);

#[derive(Clone)]
pub(super) struct ModelSources {
    records: Vec<ModelSourceFile>,
    names: qa_core::bindings::OrdinalBindings<NameId>,
}
impl ModelSources {
    pub(super) fn load(capacity: usize) -> Self {
        Self {
            records: Vec::new(),
            names: qa_core::bindings::OrdinalBindings::load(capacity, capacity, 0, true),
        }
    }
    pub(super) fn find(&self, name: NameId) -> Option<ModelSourceId> {
        self.names.ordinal(name).map(ModelSourceId)
    }
    pub(super) fn name(&self, source: ModelSourceId) -> Option<NameId> {
        self.names.get(source.0)
    }
    pub(super) fn get(&self, source: ModelSourceId) -> Option<&ModelSourceFile> {
        self.records.get(source.0 as usize)
    }
    pub(super) fn intern(&mut self, name: NameId) -> Result<ModelSourceId, FileError> {
        if let Some(source) = self.find(name) {
            return Ok(source);
        }
        let source = ModelSourceId(
            u32::try_from(self.records.len())
                .map_err(|_| FileError::capacity("model source slots"))?,
        );
        self.records
            .try_reserve(1)
            .map_err(|_| FileError::capacity("model source slots"))?;
        self.names
            .set(source.0, Some(name))
            .map_err(|_| FileError::capacity("model source name binding"))?;
        self.records.push(ModelSourceFile::Empty);
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
                if !matches!(previous, ModelSourceFile::Empty) {
                    *record = previous;
                }
            }
            for record in &mut self.records {
                record.rollback_resources(retained, retained_tags);
            }
        }
    }
}
impl std::ops::Index<ModelSourceId> for ModelSources {
    type Output = ModelSourceFile;
    fn index(&self, source: ModelSourceId) -> &Self::Output {
        &self.records[source.0 as usize]
    }
}
impl std::ops::IndexMut<ModelSourceId> for ModelSources {
    fn index_mut(&mut self, source: ModelSourceId) -> &mut Self::Output {
        &mut self.records[source.0 as usize]
    }
}
