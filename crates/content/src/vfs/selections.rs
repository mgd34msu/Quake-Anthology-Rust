//! Cold resource rows keyed by the VFS owner and ordered source selection.
use super::{SearchView, VfsError};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SelectionId(usize);
#[derive(Clone)]
pub struct Selections<T> {
    entries: Vec<(SearchView, T)>,
}
impl<T> Default for Selections<T> {
    fn default() -> Self {
        Self {
            entries: Vec::new(),
        }
    }
}
impl<T> Selections<T> {
    pub fn from_entry(view: SearchView, value: T) -> Self {
        Self {
            entries: vec![(view, value)],
        }
    }
    pub fn find(&self, view: &SearchView) -> Option<SelectionId> {
        self.entries
            .iter()
            .position(|(selected, _)| selected.same_selection(view))
            .map(SelectionId)
    }
    pub fn intern(
        &mut self,
        view: SearchView,
        capacity: usize,
        create: impl FnOnce() -> T,
    ) -> Result<SelectionId, VfsError> {
        if let Some(id) = self.find(&view) {
            return Ok(id);
        }
        if self.entries.len() >= capacity {
            return Err(VfsError::Capacity);
        }
        self.entries
            .try_reserve(1)
            .map_err(|_| VfsError::Allocation)?;
        let id = SelectionId(self.entries.len());
        self.entries.push((view, create()));
        Ok(id)
    }
    pub fn view(&self, id: SelectionId) -> &SearchView {
        &self.entries[id.0].0
    }
    pub fn get(&self, id: SelectionId) -> &T {
        &self.entries[id.0].1
    }
    pub fn get_mut(&mut self, id: SelectionId) -> &mut T {
        &mut self.entries[id.0].1
    }
    pub fn entries(&self) -> impl Iterator<Item = (&SearchView, &T)> {
        self.entries.iter().map(|(view, value)| (view, value))
    }
    pub fn values_mut(&mut self) -> impl Iterator<Item = &mut T> {
        self.entries.iter_mut().map(|(_, value)| value)
    }
    pub fn values(&self) -> impl Iterator<Item = &T> {
        self.entries.iter().map(|(_, value)| value)
    }
}
