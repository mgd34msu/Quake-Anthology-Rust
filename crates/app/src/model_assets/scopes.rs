//! Retained source selections key cold resource and numeric binding tables.
use qa_content::vfs::SearchView;
use qa_core::{
    bindings::{BindingId, OrdinalBindings},
    primitives::NameId,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct SelectionId(usize);
#[derive(Clone)]
pub(super) struct Selections<T> {
    entries: Vec<(SearchView, T)>,
}
impl<T> Selections<T> {
    pub(super) fn new() -> Self {
        Self {
            entries: Vec::new(),
        }
    }
    pub(super) fn find(&self, view: &SearchView) -> Option<SelectionId> {
        self.entries
            .iter()
            .position(|(selected, _)| selected.same_selection(view))
            .map(SelectionId)
    }
    pub(super) fn intern(
        &mut self,
        view: SearchView,
        capacity: usize,
        create: impl FnOnce() -> T,
    ) -> Result<SelectionId, ()> {
        if let Some(id) = self.find(&view) {
            return Ok(id);
        }
        if self.entries.len() >= capacity {
            return Err(());
        }
        self.entries.try_reserve(1).map_err(|_| ())?;
        let id = SelectionId(self.entries.len());
        self.entries.push((view, create()));
        Ok(id)
    }
    fn view(&self, id: SelectionId) -> &SearchView {
        &self.entries[id.0].0
    }
    pub(super) fn get(&self, id: SelectionId) -> &T {
        &self.entries[id.0].1
    }
    fn get_mut(&mut self, id: SelectionId) -> &mut T {
        &mut self.entries[id.0].1
    }
    fn values(&self) -> impl Iterator<Item = &T> {
        self.entries.iter().map(|(_, value)| value)
    }
}

#[derive(Clone)]
pub(super) struct ScopedBindings<K> {
    scopes: Selections<OrdinalBindings<K>>,
    names: usize,
    capacity: usize,
}
impl<K: BindingId> ScopedBindings<K> {
    pub(super) fn load(names: usize, capacity: usize) -> Self {
        Self {
            scopes: Selections::new(),
            names,
            capacity,
        }
    }
    pub(super) fn find(&self, view: &SearchView, name: NameId) -> Option<K> {
        self.scopes.get(self.scopes.find(view)?).get(name.0)
    }
    pub(super) fn bind(
        &mut self,
        view: SearchView,
        name: NameId,
        value: K,
    ) -> Result<SelectionId, ()> {
        if name.0 as usize >= self.names {
            return Err(());
        }
        let names = self.names;
        let id = self.scopes.intern(view, self.capacity, || {
            OrdinalBindings::load(names, 0, 0, true)
        })?;
        self.scopes
            .get_mut(id)
            .set(name.0, Some(value))
            .map_err(|_| ())?;
        Ok(id)
    }
    pub(super) fn view(&self, id: SelectionId) -> &SearchView {
        self.scopes.view(id)
    }
    pub(super) fn named(&self, name: NameId) -> impl Iterator<Item = K> + '_ {
        self.scopes
            .values()
            .filter_map(move |names| names.get(name.0))
    }
}
