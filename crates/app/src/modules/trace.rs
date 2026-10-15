//! Trace reads borrow a module only until their selected field operation returns.
use super::{Module, Program};
use qa_compat::{
    entities::TraceError,
    quakec::Vm,
    services::{CallContext, CallError, ForeignTraceRead},
};
use qa_core::primitives::{Bounds, CollisionOwner, EntityId, GeometryId, ModuleId, NativeEntity};
use qa_world::collision::{
    CollisionStore, NativeModelNames, NativeTraceEntities, NativeTraceFields, NativeTraceTarget,
};
use std::cell::RefCell;

enum Source<'a> {
    Rows {
        before: &'a [Option<Module>],
        after: &'a [Option<Module>],
    },
    Current {
        vm: RefCell<&'a mut Vm>,
        context: CallContext,
        geometry: Option<GeometryId>,
    },
}

pub(super) struct TraceView<'a> {
    source: Source<'a>,
    error: TraceError,
}

impl<'a> TraceView<'a> {
    pub(super) fn rows(before: &'a [Option<Module>], after: &'a [Option<Module>]) -> Self {
        Self {
            source: Source::Rows { before, after },
            error: TraceError::default(),
        }
    }

    pub(super) fn all(rows: &'a [Option<Module>]) -> Self {
        Self::rows(rows, &[])
    }

    pub(super) fn current(
        vm: &'a mut Vm,
        context: CallContext,
        geometry: Option<GeometryId>,
    ) -> Self {
        Self {
            source: Source::Current {
                vm: RefCell::new(vm),
                context,
                geometry,
            },
            error: TraceError::default(),
        }
    }

    pub(super) fn borrow(&self) -> ForeignTraceRead<'_> {
        ForeignTraceRead {
            view: self,
            error: &self.error,
        }
    }

    pub(super) fn edit<T>(
        &self,
        call: impl FnOnce(&mut Vm) -> Result<T, CallError>,
    ) -> Result<T, CallError> {
        let Source::Current { vm, .. } = &self.source else {
            return Err(CallError::Aborted);
        };
        let mut vm = vm.try_borrow_mut().map_err(|_| CallError::Aborted)?;
        call(&mut vm)
    }

    fn row(&self, module: ModuleId) -> Option<&Module> {
        let Source::Rows { before, after } = &self.source else {
            return None;
        };
        let index = module.0 as usize;
        let row = if index < before.len() {
            before.get(index)
        } else {
            after.get(index.checked_sub(before.len() + 1)?)
        }?
        .as_ref()?;
        (row.state.can_invoke() && matches!(&row.request.program, Program::QuakeC(_)))
            .then_some(row)
    }

    fn with_vm<T>(
        &self,
        module: ModuleId,
        call: impl FnOnce(&Vm, CallContext, Option<GeometryId>) -> Result<T, CallError>,
    ) -> Result<Option<T>, CallError> {
        match &self.source {
            Source::Rows { .. } => {
                let Some(row) = self.row(module) else {
                    return Ok(None);
                };
                let Program::QuakeC(qc) = &row.request.program else {
                    return Ok(None);
                };
                call(&qc.vm, row.request.context, row.geometry).map(Some)
            }
            Source::Current {
                vm,
                context,
                geometry,
            } if context.module == module => {
                let vm = vm.try_borrow().map_err(|_| CallError::Aborted)?;
                call(&vm, *context, *geometry).map(Some)
            }
            Source::Current { .. } => Ok(None),
        }
    }

    fn read<T>(
        &self,
        native: NativeEntity,
        call: impl FnOnce(&dyn NativeTraceEntities) -> Option<T>,
    ) -> Option<T> {
        self.error
            .capture(|| {
                self.with_vm(native.module, |vm, context, geometry| {
                    vm.with_trace_view(context, 0, geometry, |view, _| Ok(call(view)))
                })
            })
            .flatten()
            .flatten()
    }
}

impl NativeTraceEntities for TraceView<'_> {
    fn failed(&self) -> bool {
        self.error.failed()
    }

    fn supports(&self, module: ModuleId) -> bool {
        match &self.source {
            Source::Rows { .. } => self.row(module).is_some(),
            Source::Current { context, .. } => context.module == module,
        }
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
        models: Option<&dyn NativeModelNames>,
    ) -> Option<NativeTraceTarget> {
        self.read(native, |view| {
            view.target(native, cached, point_contents, store, models)
        })
    }
}

#[cfg(test)]
#[path = "../../tests/modules/foreign_trace.rs"]
mod tests;
