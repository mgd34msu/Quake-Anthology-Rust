//! Logical model handles and profile rows share one owned registration table.
use super::*;

struct Record {
    state: ModelFile,
    profiles: Vec<usize>,
}
pub(super) struct ModelFiles {
    records: Vec<Record>,
    names: qa_core::bindings::OrdinalBindings<NameId>,
}
impl ModelFiles {
    pub(super) fn load(names: usize, inline: usize) -> Result<Self, String> {
        let capacity = names
            .checked_add(inline)
            .and_then(|count| count.checked_add(1))
            .filter(|&count| count <= i32::MAX as usize)
            .ok_or("model file handles")?;
        Ok(Self {
            records: vec![Record {
                state: ModelFile::Empty,
                profiles: Vec::new(),
            }],
            names: qa_core::bindings::OrdinalBindings::load(capacity, names, 0, false),
        })
    }
    pub(super) fn len(&self) -> usize {
        self.records.len()
    }
    pub(super) fn find(&self, name: NameId) -> Option<ModelFileId> {
        self.names.ordinal(name).map(ModelFileId)
    }
    pub(super) fn name(&self, id: ModelFileId) -> Option<NameId> {
        self.names.get(id.0)
    }
    pub(super) fn get(&self, id: ModelFileId) -> Option<&ModelFile> {
        Some(&self.records.get(id.0 as usize)?.state)
    }
    pub(super) fn rows(&self, id: ModelFileId) -> &Vec<usize> {
        &self.records[id.0 as usize].profiles
    }
    pub(super) fn rows_mut(&mut self, id: ModelFileId) -> &mut Vec<usize> {
        &mut self.records[id.0 as usize].profiles
    }
    pub(super) fn all_rows(&self) -> impl Iterator<Item = &Vec<usize>> {
        self.records.iter().map(|record| &record.profiles)
    }
    pub(super) fn all_rows_mut(&mut self) -> impl Iterator<Item = &mut Vec<usize>> {
        self.records.iter_mut().map(|record| &mut record.profiles)
    }
    pub(super) fn entries_mut(
        &mut self,
    ) -> impl Iterator<Item = (ModelFileId, &ModelFile, &mut Vec<usize>)> {
        self.records.iter_mut().enumerate().map(|(index, record)| {
            (
                ModelFileId(index as u32),
                &record.state,
                &mut record.profiles,
            )
        })
    }
    pub(super) fn iter(&self) -> impl Iterator<Item = &ModelFile> {
        self.records.iter().map(|record| &record.state)
    }
    pub(super) fn admit(&mut self, name: Option<NameId>) -> Result<ModelFileId, CallError> {
        if let Some(id) = name.and_then(|name| self.find(name)) {
            return Ok(id);
        }
        if self.records.len() >= self.names.capacity() {
            return Err(CallError::Capacity);
        }
        let id = ModelFileId(u32::try_from(self.records.len()).map_err(|_| CallError::Capacity)?);
        self.records
            .try_reserve(1)
            .map_err(|_| CallError::Capacity)?;
        self.names
            .set(id.0, name)
            .map_err(|_| CallError::Capacity)?;
        self.records.push(Record {
            state: ModelFile::Empty,
            profiles: Vec::new(),
        });
        Ok(id)
    }
}
impl std::ops::Index<ModelFileId> for ModelFiles {
    type Output = ModelFile;
    fn index(&self, id: ModelFileId) -> &Self::Output {
        &self.records[id.0 as usize].state
    }
}
impl std::ops::IndexMut<ModelFileId> for ModelFiles {
    fn index_mut(&mut self, id: ModelFileId) -> &mut Self::Output {
        &mut self.records[id.0 as usize].state
    }
}
