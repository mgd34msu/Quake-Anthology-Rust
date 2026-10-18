//! Retained source selections key cold resource and numeric binding tables.
use qa_content::vfs::SearchView;
pub(crate) use qa_content::vfs::{SelectionId, Selections};
use qa_core::{
    bindings::{BindingId, OrdinalBindings},
    primitives::NameId,
};

#[derive(Clone)]
pub(crate) struct ScopedBindings<K> {
    scopes: Selections<OrdinalBindings<K>>,
    names: usize,
    capacity: usize,
}
impl<K: BindingId> ScopedBindings<K> {
    pub(crate) fn load(names: usize, capacity: usize) -> Self {
        Self {
            scopes: Selections::default(),
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
        let id = self
            .scopes
            .intern(view, self.capacity, || {
                OrdinalBindings::load(names, 0, 0, true)
            })
            .map_err(|_| ())?;
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
        self.scopes.entries().flat_map(|(view, _)| {
            self.entries(view)
                .map(move |(name, record)| (view, name, record))
        })
    }
    /// Records append during cold admission; rollback removes only the rejected suffix.
    pub(crate) fn discard_suffix(&mut self, records: usize) -> Result<(), ()> {
        for names in self.scopes.values_mut() {
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
