//! Retained source selections key cold resource and numeric binding tables.
use qa_content::vfs::SearchView;
use qa_core::{
    bindings::{BindingId, OrdinalBindings},
    primitives::NameId,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct SelectionId(usize);
#[derive(Clone)]
pub(crate) struct Selections<T> {
    entries: Vec<(SearchView, T)>,
}
impl<T> Selections<T> {
    pub(crate) fn new() -> Self {
        Self {
            entries: Vec::new(),
        }
    }
    pub(crate) fn from_entry(view: SearchView, value: T) -> Self {
        Self {
            entries: vec![(view, value)],
        }
    }
    pub(crate) fn find(&self, view: &SearchView) -> Option<SelectionId> {
        self.entries
            .iter()
            .position(|(selected, _)| selected.same_selection(view))
            .map(SelectionId)
    }
    pub(crate) fn intern(
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
    pub(crate) fn get(&self, id: SelectionId) -> &T {
        &self.entries[id.0].1
    }
    pub(crate) fn get_mut(&mut self, id: SelectionId) -> &mut T {
        &mut self.entries[id.0].1
    }
    pub(crate) fn values(&self) -> impl Iterator<Item = &T> {
        self.entries.iter().map(|(_, value)| value)
    }
}

#[derive(Clone)]
pub(crate) struct ScopedBindings<K> {
    scopes: Selections<OrdinalBindings<K>>,
    names: usize,
    capacity: usize,
}
impl<K: BindingId> ScopedBindings<K> {
    pub(crate) fn load(names: usize, capacity: usize) -> Self {
        Self {
            scopes: Selections::new(),
            names,
            capacity,
        }
    }
    pub(crate) fn find(&self, view: &SearchView, name: NameId) -> Option<K> {
        self.scopes.get(self.scopes.find(view)?).get(name.0)
    }
    pub(crate) fn bind(
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
    pub(crate) fn view(&self, id: SelectionId) -> &SearchView {
        self.scopes.view(id)
    }
    pub(crate) fn named(&self, name: NameId) -> impl Iterator<Item = K> + '_ {
        self.scopes
            .values()
            .filter_map(move |names| names.get(name.0))
    }
    pub(crate) fn entries(&self, source: &SearchView) -> impl Iterator<Item = (NameId, K)> + '_ {
        let names = self.scopes.find(source).map(|id| self.scopes.get(id));
        (0..self.names).filter_map(move |ordinal| {
            let ordinal = u32::try_from(ordinal).ok()?;
            Some((NameId(ordinal), names?.get(ordinal)?))
        })
    }
    pub(crate) fn all_entries(&self) -> impl Iterator<Item = (&SearchView, NameId, K)> + '_ {
        self.scopes.entries.iter().flat_map(|(view, _)| {
            self.entries(view)
                .map(move |(name, record)| (view, name, record))
        })
    }
    /// Records append during cold admission; rollback removes only the rejected suffix.
    pub(crate) fn discard_suffix(&mut self, records: usize) -> Result<(), ()> {
        for (_, names) in &mut self.scopes.entries {
            for ordinal in 0..names.capacity() {
                let ordinal = u32::try_from(ordinal).map_err(|_| ())?;
                if names.get(ordinal).is_some_and(|id| id.index() >= records) {
                    names.set(ordinal, None).map_err(|_| ())?;
                }
            }
        }
        Ok(())
    }
}
