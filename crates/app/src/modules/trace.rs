//! Other module rows are borrowed only until the current host operation yields.
use super::{Module, Program};
use qa_compat::{entities::TraceError, services::ForeignTraceRead};
use qa_core::primitives::{Bounds, CollisionOwner, EntityId, GeometryId, ModuleId, NativeEntity};
use qa_world::collision::{
    CollisionStore, NativeModelNames, NativeTraceEntities, NativeTraceFields, NativeTraceTarget,
};

pub(super) struct RowsView<'a> {
    before: &'a [Option<Module>],
    after: &'a [Option<Module>],
    error: TraceError,
}

impl<'a> RowsView<'a> {
    pub(super) fn new(before: &'a [Option<Module>], after: &'a [Option<Module>]) -> Self {
        Self {
            before,
            after,
            error: TraceError::default(),
        }
    }

    pub(super) fn borrow(&self) -> ForeignTraceRead<'_> {
        ForeignTraceRead {
            view: self,
            error: &self.error,
        }
    }

    fn row(&self, module: ModuleId) -> Option<&Module> {
        let index = module.0 as usize;
        let row = if index < self.before.len() {
            self.before.get(index)
        } else {
            self.after.get(index.checked_sub(self.before.len() + 1)?)
        }?
        .as_ref()?;
        (row.state.can_invoke() && matches!(&row.request.program, Program::QuakeC(_)))
            .then_some(row)
    }

    fn read<T>(
        &self,
        native: NativeEntity,
        call: impl FnOnce(&dyn NativeTraceEntities) -> Option<T>,
    ) -> Option<T> {
        let row = self.row(native.module)?;
        let Program::QuakeC(qc) = &row.request.program else {
            return None;
        };
        self.error
            .capture(|| {
                qc.vm
                    .with_trace_view(row.request.context, 0, row.geometry, |view, _| {
                        Ok(call(view))
                    })
            })
            .flatten()
    }
}

impl NativeTraceEntities for RowsView<'_> {
    fn supports(&self, module: ModuleId) -> bool {
        self.row(module).is_some()
    }

    fn anchor(&self, module: ModuleId) -> Option<EntityId> {
        self.row(module).map(|row| row.request.anchor)
    }

    fn owner(&self, native: NativeEntity) -> Option<CollisionOwner> {
        self.read(native, |view| view.owner(native))
    }

    fn fields(&self, native: NativeEntity, point_contents: bool) -> Option<NativeTraceFields> {
        self.read(native, |view| view.fields(native, point_contents))
    }

    fn bounds(&self, native: NativeEntity, cached: Bounds) -> Option<Bounds> {
        self.read(native, |view| view.bounds(native, cached))
    }

    fn target(
        &self,
        native: NativeEntity,
        cached: NativeTraceTarget,
        point_contents: bool,
        store: &CollisionStore,
        geometry: GeometryId,
        models: Option<&dyn NativeModelNames>,
    ) -> Option<NativeTraceTarget> {
        self.read(native, |view| {
            view.target(native, cached, point_contents, store, geometry, models)
        })
    }
}

#[cfg(test)]
#[path = "../../tests/modules/foreign_trace.rs"]
mod tests;
