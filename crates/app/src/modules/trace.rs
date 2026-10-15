//! Trace reads borrow a module only until their selected field operation returns.
use super::{Module, Program, execution::TraceAccess};
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

struct Rows<'a> {
    before: &'a mut [Option<Module>],
    after: &'a mut [Option<Module>],
}
impl Rows<'_> {
    fn row(&self, module: ModuleId) -> Option<&Module> {
        let index = module.0 as usize;
        let row = if index < self.before.len() {
            self.before.get(index)
        } else {
            self.after.get(index.checked_sub(self.before.len() + 1)?)
        }?
        .as_ref()?;
        row.state.can_invoke().then_some(row)
    }
    fn row_mut(&mut self, module: ModuleId) -> Option<&mut Module> {
        let index = module.0 as usize;
        let row = if index < self.before.len() {
            self.before.get_mut(index)
        } else {
            self.after
                .get_mut(index.checked_sub(self.before.len() + 1)?)
        }?
        .as_mut()?;
        row.state.can_invoke().then_some(row)
    }
}

enum Source<'a> {
    Rows(RefCell<Rows<'a>>),
    Current {
        vm: RefCell<&'a mut Vm>,
        context: CallContext,
        geometry: Option<GeometryId>,
    },
}

pub(super) struct TraceView<'a> {
    source: Source<'a>,
    error: TraceError,
    access: TraceAccess<'a>,
}

impl<'a> TraceView<'a> {
    pub(super) fn rows(
        before: &'a mut [Option<Module>],
        after: &'a mut [Option<Module>],
        access: TraceAccess<'a>,
    ) -> Self {
        Self {
            source: Source::Rows(RefCell::new(Rows { before, after })),
            error: TraceError::default(),
            access,
        }
    }

    pub(super) fn all(rows: &'a mut [Option<Module>], access: TraceAccess<'a>) -> Self {
        Self::rows(rows, &mut [], access)
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
            access: TraceAccess::empty(),
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

    fn anchor(&self, module: ModuleId) -> Result<Option<EntityId>, CallError> {
        let Source::Rows(rows) = &self.source else {
            return Ok(None);
        };
        let rows = rows.try_borrow().map_err(|_| CallError::Aborted)?;
        Ok(rows.row(module).map(|row| row.request.anchor))
    }

    fn with_view<T>(
        &self,
        module: ModuleId,
        call: impl FnOnce(Option<&dyn NativeTraceEntities>) -> Result<T, CallError>,
    ) -> Result<Option<T>, CallError> {
        match &self.source {
            Source::Rows(rows) => {
                let mut rows = rows.try_borrow_mut().map_err(|_| CallError::Aborted)?;
                let Some(row) = rows.row_mut(module) else {
                    return Ok(None);
                };
                let context = row.request.context;
                let geometry = row.geometry;
                match &mut row.request.program {
                    Program::QuakeC(qc) => {
                        qc.vm
                            .with_trace_view(context, 0, geometry, |view, _| call(Some(view)))
                    }
                    Program::Qvm { vm, .. } => vm.with_trace_view(context, geometry, call),
                    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
                    Program::Native { vm, .. } => vm
                        .with_trace_view(context, geometry, self.access.native(module), call)
                        .map_err(|error| match error {
                            qa_compat::native::Error::Service(error) => error,
                            _ => CallError::Aborted,
                        }),
                }
                .map(Some)
            }
            Source::Current {
                vm,
                context,
                geometry,
            } if context.module == module => {
                let vm = vm.try_borrow().map_err(|_| CallError::Aborted)?;
                vm.with_trace_view(*context, 0, *geometry, |view, _| call(Some(view)))
                    .map(Some)
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
            .capture(|| self.with_view(native.module, |view| Ok(view.and_then(call))))
            .flatten()
            .flatten()
    }
}

impl NativeTraceEntities for TraceView<'_> {
    fn failed(&self) -> bool {
        self.error.failed()
    }

    fn supports(&self, module: ModuleId) -> bool {
        self.error
            .capture(|| {
                self.with_view(module, |view| {
                    Ok(view.is_some_and(|view| view.supports(module)))
                })
            })
            .map_or(true, |supported| supported.unwrap_or(false))
    }

    fn anchor(&self, module: ModuleId) -> Option<EntityId> {
        self.error
            .capture(|| TraceView::anchor(self, module))
            .flatten()
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
