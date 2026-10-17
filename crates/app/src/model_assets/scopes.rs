//! Numeric name bindings are partitioned by the same owned VFS selections.
use qa_content::vfs::SearchView;
use qa_core::{
    bindings::{BindingId, OrdinalBindings},
    primitives::NameId,
};
#[derive(Clone)]
struct Scope<K> {
    view: SearchView,
    names: OrdinalBindings<K>,
}
#[derive(Clone)]
pub(super) struct ScopedBindings<K> {
    scopes: Vec<Scope<K>>,
    names: usize,
}
impl<K: BindingId> ScopedBindings<K> {
    pub(super) fn load(names: usize) -> Self {
        Self {
            scopes: Vec::new(),
            names,
        }
    }
    pub(super) fn find(&self, view: &SearchView, name: NameId) -> Option<K> {
        self.scopes
            .iter()
            .find(|scope| scope.view.same_selection(view))?
            .names
            .get(name.0)
    }
    pub(super) fn bind(&mut self, view: SearchView, name: NameId, value: K) -> Result<(), ()> {
        if name.0 as usize >= self.names {
            return Err(());
        }
        let index = if let Some(index) = self
            .scopes
            .iter()
            .position(|scope| scope.view.same_selection(&view))
        {
            index
        } else {
            self.scopes.try_reserve(1).map_err(|_| ())?;
            self.scopes.push(Scope {
                view,
                names: OrdinalBindings::load(self.names, 0, 0, true),
            });
            self.scopes.len() - 1
        };
        self.scopes[index]
            .names
            .set(name.0, Some(value))
            .map_err(|_| ())
    }
    pub(super) fn named(&self, name: NameId) -> impl Iterator<Item = K> + '_ {
        self.scopes
            .iter()
            .filter_map(move |scope| scope.names.get(name.0))
    }
}
