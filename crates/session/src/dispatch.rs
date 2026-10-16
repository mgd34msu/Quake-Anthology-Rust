pub use qa_core::primitives::RuleSetId;
use qa_core::primitives::{
    CallbackCall, CallbackId, EntityId, ModuleId, NativeEntityRef, ThinkBinding, ThinkTime,
};
use qa_world::entities::EntityTable;

/// A native function, QC function or foreign export binds its boundary adapter
/// once in a numeric module table, without resource-name lookup during calls.
pub struct FunctionBinding<C, R = bool> {
    pub entry: u32,
    pub call: fn(&mut C, ModuleId, u32, CallbackCall<'_>) -> R,
}

pub struct FunctionTable<C, R = bool> {
    modules: Box<[Option<ModuleBinding>]>,
    functions: Box<[ResolvedFunction<C, R>]>,
}

/// A callback may finish immediately or return owned execution state. The
/// synchronous entry completes that state before treating the call as accepted.
/// Borrowed arguments remain with the caller rather than inside the outcome.
pub trait CallOutcome<C>: Sized {
    fn rejected() -> Self;
    fn complete(self, world: &mut C, call: CallbackCall<'_>) -> bool;
}

impl<C> CallOutcome<C> for bool {
    #[inline]
    fn rejected() -> Self {
        false
    }

    #[inline]
    fn complete(self, _world: &mut C, _call: CallbackCall<'_>) -> bool {
        self
    }
}

/// A checked callback in the immutable loaded table. Guest adapters consume
/// its numeric entry after ending their current memory borrow.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ResolvedCall {
    entity: EntityId,
    module: ModuleId,
    path: u32,
    entry: u32,
}
impl ResolvedCall {
    pub fn entity(self) -> EntityId {
        self.entity
    }
    pub fn module(self) -> ModuleId {
        self.module
    }
    pub fn entry(self) -> u32 {
        self.entry
    }
}

pub trait CallbackResolver {
    fn resolve_callback(
        &self,
        entities: &EntityTable,
        module: ModuleId,
        callback: CallbackId,
        entity: EntityId,
    ) -> Result<ResolvedCall, CallError>;
}

struct ResolvedFunction<C, R> {
    function: FunctionBinding<C, R>,
    module: ModuleId,
    timing: ThinkRules,
}

/// Immutable native function-index conversion, copied into a module at load.
#[derive(Clone, Copy, Debug, Default)]
pub struct ModuleBinding {
    first: u32,
    count: u32,
    missing: u32,
}

impl ModuleBinding {
    /// Resolve at spawn, or whenever the native callback or owner changes.
    /// Null and invalid functions retain their native value and use the
    /// module's timing policy before a due call is rejected.
    pub fn think(self, callback: Option<CallbackId>) -> ThinkBinding {
        ThinkBinding {
            callback,
            path: callback
                .filter(|id| id.0 < self.count)
                .map_or(self.missing, |id| self.first + id.0),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ThinkFrame {
    Seconds { now: f64, step: f64 },
    Milliseconds { now: i64 },
}

struct ThinkRules {
    seconds: bool,
    frame_end: bool,
    integer_float: bool,
    repeat: bool,
    tolerance: f64,
}

impl ThinkRules {
    fn load(rules: RuleSetId) -> Self {
        Self {
            seconds: matches!(ThinkTime::module_clock(rules), ThinkTime::Seconds(_)),
            frame_end: matches!(rules, RuleSetId::Quake | RuleSetId::QuakeWorld),
            integer_float: rules == RuleSetId::Quake3,
            repeat: rules == RuleSetId::QuakeWorld,
            tolerance: if rules == RuleSetId::Quake2 {
                0.001
            } else {
                0.0
            },
        }
    }

    #[inline]
    fn evaluate(&self, due: ThinkTime, frame: ThinkFrame) -> Result<Option<ThinkTime>, CallError> {
        match (due, frame) {
            (ThinkTime::Seconds(due), ThinkFrame::Seconds { now, step }) if self.seconds => {
                let mut due = due as f32;
                if self.frame_end {
                    // Q1/QW compare float nextthink against double frame time.
                    if due <= 0.0 || f64::from(due) > now + step {
                        return Ok(None);
                    }
                    if f64::from(due) < now {
                        due = now as f32;
                    }
                    Ok(Some(ThinkTime::Seconds(f64::from(due))))
                } else {
                    // Q2 stores the clock in float, then promotes for 0.001.
                    let now = now as f32;
                    if due <= 0.0 || f64::from(due) > f64::from(now) + self.tolerance {
                        return Ok(None);
                    }
                    Ok(Some(ThinkTime::Seconds(f64::from(now))))
                }
            }
            (ThinkTime::Milliseconds(due), ThinkFrame::Milliseconds { now }) if !self.seconds => {
                if self.integer_float {
                    // Q3 compares native int32 times as float, while the
                    // callback keeps the integer clock above float precision.
                    let due = (due as i32) as f32;
                    let now = now as i32;
                    if due <= 0.0 || due > now as f32 {
                        return Ok(None);
                    }
                    Ok(Some(ThinkTime::Milliseconds(i64::from(now))))
                } else {
                    // Rerelease gtime_t keeps signed int64 milliseconds.
                    if due <= 0 || due > now {
                        return Ok(None);
                    }
                    Ok(Some(ThinkTime::Milliseconds(now)))
                }
            }
            _ => Err(CallError::TimeKind),
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum TableError {
    DuplicateModule,
    FunctionCapacity,
}

#[derive(Debug, PartialEq, Eq)]
pub enum CallError {
    MissingModule,
    MissingFunction,
    TimeKind,
    StaleEntity,
    Rejected,
}

impl<C: ThinkWorld, R: CallOutcome<C>> FunctionTable<C, R> {
    pub fn load(
        modules: impl IntoIterator<Item = (ModuleId, RuleSetId, Vec<FunctionBinding<C, R>>)>,
    ) -> Result<Self, TableError> {
        let mut tables = Vec::new();
        // Path zero is the unbound/missing-module marker, never a live entry.
        let mut resolved = vec![ResolvedFunction {
            function: FunctionBinding {
                entry: 0,
                call: |_, _, _, _| R::rejected(),
            },
            module: ModuleId::default(),
            timing: ThinkRules::load(RuleSetId::Quake3),
        }];
        for (module, timing, functions) in modules {
            let count = u32::try_from(functions.len()).map_err(|_| TableError::FunctionCapacity)?;
            let first = u32::try_from(resolved.len()).map_err(|_| TableError::FunctionCapacity)?;
            let missing = first
                .checked_add(count)
                .ok_or(TableError::FunctionCapacity)?;
            if missing == u32::MAX {
                return Err(TableError::FunctionCapacity);
            }
            let slot = module.0 as usize;
            if slot >= tables.len() {
                tables.resize_with(slot + 1, || None);
            }
            if tables[slot].is_some() {
                return Err(TableError::DuplicateModule);
            }
            tables[slot] = Some(ModuleBinding {
                first,
                count,
                missing,
            });
            for function in functions {
                resolved.push(ResolvedFunction {
                    function,
                    module,
                    timing: ThinkRules::load(timing),
                });
            }
            resolved.push(ResolvedFunction {
                function: FunctionBinding {
                    entry: 0,
                    call: |_, _, _, _| R::rejected(),
                },
                module,
                timing: ThinkRules::load(timing),
            });
        }
        Ok(Self {
            modules: tables.into_boxed_slice(),
            functions: resolved.into_boxed_slice(),
        })
    }

    fn binding(&self, module: ModuleId) -> Option<ModuleBinding> {
        self.modules.get(module.0 as usize).copied().flatten()
    }
    pub fn module_binding(&self, module: ModuleId) -> ModuleBinding {
        self.binding(module).unwrap_or_default()
    }

    /// Install a module's load-selected rejected-call adapter without adding
    /// another timing path. QC must clear native nextthink even when the due
    /// native function is missing; other hosts retain their default rejection.
    pub fn bind_missing(&mut self, module: ModuleId, function: FunctionBinding<C, R>) -> bool {
        let Some(binding) = self.binding(module) else {
            return false;
        };
        let Some(entry) = self.functions.get_mut(binding.missing as usize) else {
            return false;
        };
        entry.function = function;
        true
    }

    /// Bind the current lifetime at a load/spawn or native field-write boundary.
    /// Rebind after ownership changes; internal paths never go on the wire.
    pub fn bind_think(
        &self,
        entities: &mut EntityTable,
        entity: EntityId,
        callback: Option<CallbackId>,
    ) -> bool {
        let Some(slot) = entities.resolve(entity) else {
            return false;
        };
        let binding = self
            .module_binding(entities.columns.owner[slot])
            .think(callback);
        entities.columns.set_think_function(slot, binding);
        true
    }

    pub fn resolve(
        &self,
        entities: &EntityTable,
        callback: CallbackId,
        entity: EntityId,
    ) -> Result<ResolvedCall, CallError> {
        let slot = entities.resolve(entity).ok_or(CallError::StaleEntity)?;
        let module = entities.columns.owner[slot];
        let binding = self.binding(module).ok_or(CallError::MissingModule)?;
        if callback.0 >= binding.count {
            return Err(CallError::MissingFunction);
        }
        self.resolve_binding(entities, binding.think(Some(callback)), entity)
    }

    /// Resolve an already cached native binding, including the module's
    /// rejected-call slot for null or invalid native function words.
    pub fn resolve_binding(
        &self,
        entities: &EntityTable,
        cached: ThinkBinding,
        entity: EntityId,
    ) -> Result<ResolvedCall, CallError> {
        let slot = entities.resolve(entity).ok_or(CallError::StaleEntity)?;
        let module = entities.columns.owner[slot];
        let binding = self.binding(module).ok_or(CallError::MissingModule)?;
        let path = cached.path;
        if path < binding.first || path > binding.missing {
            return Err(CallError::MissingFunction);
        }
        let function = self
            .functions
            .get(path as usize)
            .filter(|function| function.module == module)
            .ok_or(CallError::MissingFunction)?;
        Ok(ResolvedCall {
            entity,
            module,
            path,
            entry: function.function.entry,
        })
    }

    /// Start the checked callback without completing its owned outcome.
    pub fn dispatch(
        &self,
        world: &mut C,
        callback: CallbackId,
        call: CallbackCall<'_>,
    ) -> Result<R, CallError> {
        let resolved = self.resolve(world.entities(), callback, call.entity())?;
        self.dispatch_resolved(world, resolved, call)
    }

    /// Revalidate a previously resolved callback after an intervening call may
    /// have changed its entity lifetime or owner. The current table remains the
    /// authority for both its internal path and native entry.
    pub fn dispatch_resolved(
        &self,
        world: &mut C,
        resolved: ResolvedCall,
        call: CallbackCall<'_>,
    ) -> Result<R, CallError> {
        if call.entity() != resolved.entity {
            return Err(CallError::StaleEntity);
        }
        let entities = world.entities();
        let slot = entities
            .resolve(resolved.entity)
            .ok_or(CallError::StaleEntity)?;
        if entities.columns.owner[slot] != resolved.module {
            return Err(CallError::MissingModule);
        }
        let binding = self
            .binding(resolved.module)
            .ok_or(CallError::MissingModule)?;
        // The module's final slot is its existing rejected-call adapter. Due
        // thinks may execute it after clearing nextthink, even for null funcs.
        if resolved.path < binding.first || resolved.path > binding.missing {
            return Err(CallError::MissingFunction);
        }
        let function = self
            .functions
            .get(resolved.path as usize)
            .filter(|function| {
                function.module == resolved.module && function.function.entry == resolved.entry
            })
            .ok_or(CallError::MissingFunction)?;
        Ok((function.function.call)(
            world,
            resolved.module,
            resolved.entry,
            call,
        ))
    }

    pub fn invoke(
        &self,
        world: &mut C,
        callback: CallbackId,
        call: CallbackCall<'_>,
    ) -> Result<(), CallError> {
        if self.dispatch(world, callback, call)?.complete(world, call) {
            Ok(())
        } else {
            Err(CallError::Rejected)
        }
    }
}

impl<C: ThinkWorld, R: CallOutcome<C>> CallbackResolver for FunctionTable<C, R> {
    fn resolve_callback(
        &self,
        entities: &EntityTable,
        module: ModuleId,
        callback: CallbackId,
        entity: EntityId,
    ) -> Result<ResolvedCall, CallError> {
        let resolved = self.resolve(entities, callback, entity)?;
        if resolved.module != module {
            return Err(CallError::MissingModule);
        }
        Ok(resolved)
    }
}

pub struct PreparedNativeThink {
    pub subject: NativeEntityRef,
    pub binding: ThinkBinding,
}

pub trait ThinkWorld {
    fn entities(&mut self) -> &mut EntityTable;
    fn prepare_native_think(
        &mut self,
        _entity: EntityId,
        _subject: Option<NativeEntityRef>,
        _time: ThinkTime,
    ) -> Result<Option<PreparedNativeThink>, CallError> {
        Ok(None)
    }
}

impl ThinkWorld for crate::clients::Server {
    fn entities(&mut self) -> &mut EntityTable {
        &mut self.entities
    }
}

#[derive(Default, Debug, PartialEq, Eq)]
pub struct DispatchStats {
    pub called: u32,
    pub rejected: u32,
}

#[derive(Default, Debug, PartialEq, Eq)]
pub struct ThinkResult {
    pub called: u32,
    pub rejected: u32,
    pub current_entity: Option<EntityId>,
}

/// Complete one admitted think without another clock or repetition policy.
/// Native preparation runs before selecting the current function. A weak native
/// subject can use its module's live anchor solely for call authorization.
pub fn call_think_once<C: ThinkWorld, R: CallOutcome<C>>(
    world: &mut C,
    table: &FunctionTable<C, R>,
    entity: EntityId,
    subject: Option<NativeEntityRef>,
    time: ThinkTime,
    rebind: impl FnOnce(&mut C) -> Option<EntityId>,
) -> ThinkResult {
    call_think_once_with_completion(
        world,
        table,
        entity,
        subject,
        time,
        rebind,
        |world, outcome, call| outcome.complete(world, call),
    )
}

pub fn call_think_once_with_completion<C: ThinkWorld, R: CallOutcome<C>>(
    world: &mut C,
    table: &FunctionTable<C, R>,
    entity: EntityId,
    subject: Option<NativeEntityRef>,
    time: ThinkTime,
    rebind: impl FnOnce(&mut C) -> Option<EntityId>,
    complete: impl FnOnce(&mut C, R, CallbackCall<'_>) -> bool,
) -> ThinkResult {
    let execute = (|| {
        let entities = world.entities();
        let slot = entities.resolve(entity)?;
        let fallback = entities.columns.think_binding(slot);
        if subject.is_none() {
            entities.columns.next_think[slot] = None;
        }
        let prepared = world.prepare_native_think(entity, subject, time).ok()?;
        let (binding, native) = match prepared {
            Some(prepared) => (prepared.binding, Some(prepared.subject)),
            None if subject.is_none() => (fallback, None),
            None => return None,
        };
        let resolved = table
            .resolve_binding(world.entities(), binding, entity)
            .ok()?;
        let call = CallbackCall::Think {
            entity,
            time,
            prepared_native: native,
        };
        Some(
            table
                .dispatch_resolved(world, resolved, call)
                .is_ok_and(|outcome| complete(world, outcome, call)),
        )
    })();
    if execute == Some(true) {
        ThinkResult {
            called: 1,
            rejected: 0,
            current_entity: rebind(world),
        }
    } else {
        ThinkResult {
            called: 0,
            rejected: 1,
            current_entity: if subject.is_none() {
                world.entities().resolve(entity).map(|_| entity)
            } else {
                None
            },
        }
    }
}

/// Run one current entity lifetime. A due deadline is cleared independently of
/// its callback. QW repeats successful reschedules before the next source slot;
/// rejection or lifetime removal stops that entity, without a synthetic cap.
pub fn run_think<C: ThinkWorld, R: CallOutcome<C>>(
    world: &mut C,
    table: &FunctionTable<C, R>,
    entity: EntityId,
    clock: impl FnMut(ModuleId) -> Option<ThinkFrame>,
) -> ThinkResult {
    run_think_with_rebind(world, table, entity, clock, |world, entity| {
        world.entities().resolve(entity).map(|_| entity)
    })
}

/// The callback boundary may resolve a new live handle for the same native
/// row. The supplied resolver runs only after an accepted call; this remains
/// the sole timing/repetition loop and never makes a stale handle live again.
pub fn run_think_with_rebind<C: ThinkWorld, R: CallOutcome<C>>(
    world: &mut C,
    table: &FunctionTable<C, R>,
    entity: EntityId,
    clock: impl FnMut(ModuleId) -> Option<ThinkFrame>,
    rebind: impl FnMut(&mut C, EntityId) -> Option<EntityId>,
) -> ThinkResult {
    run_think_with_rebind_and_completion(
        world,
        table,
        entity,
        clock,
        rebind,
        |world, outcome, call| outcome.complete(world, call),
    )
}

pub fn run_think_with_rebind_and_completion<C: ThinkWorld, R: CallOutcome<C>>(
    world: &mut C,
    table: &FunctionTable<C, R>,
    mut entity: EntityId,
    mut clock: impl FnMut(ModuleId) -> Option<ThinkFrame>,
    mut rebind: impl FnMut(&mut C, EntityId) -> Option<EntityId>,
    mut complete: impl FnMut(&mut C, R, CallbackCall<'_>) -> bool,
) -> ThinkResult {
    let mut result = ThinkResult::default();
    loop {
        let entities = world.entities();
        let Some(slot) = entities.resolve(entity) else {
            result.current_entity = None;
            return result;
        };
        result.current_entity = Some(entity);
        let Some(due) = entities.columns.next_think[slot] else {
            return result;
        };
        let binding = entities.columns.think_binding(slot);
        let Some(functions) = table
            .functions
            .get(binding.path as usize)
            .filter(|_| binding.path != 0)
        else {
            result.rejected = result.rejected.saturating_add(1);
            return result;
        };
        let module = functions.module;
        let Some(frame) = clock(module) else {
            result.rejected = result.rejected.saturating_add(1);
            return result;
        };
        let time = match functions.timing.evaluate(due, frame) {
            Ok(Some(time)) => time,
            Ok(None) => return result,
            Err(_) => {
                result.rejected = result.rejected.saturating_add(1);
                return result;
            }
        };
        let once = call_think_once_with_completion(
            world,
            table,
            entity,
            None,
            time,
            |world| rebind(world, entity),
            &mut complete,
        );
        result.current_entity = once.current_entity;
        result.rejected = result.rejected.saturating_add(once.rejected);
        if once.rejected != 0 {
            return result;
        }
        result.called = result.called.saturating_add(once.called);
        if !functions.timing.repeat {
            return result;
        }
        let Some(current) = result.current_entity else {
            return result;
        };
        entity = current;
    }
}

/// Serial source-slot order; each entity resolves its owning module's frame.
/// Placement among native physics operations belongs to the module caller.
pub fn run_thinks<C: ThinkWorld, R: CallOutcome<C>>(
    world: &mut C,
    table: &FunctionTable<C, R>,
    mut clock: impl FnMut(ModuleId) -> Option<ThinkFrame>,
) -> DispatchStats {
    let mut stats = DispatchStats::default();
    let mut start = 0;
    while let Some(entity) = world.entities().next_active(start) {
        start = entity.slot as usize + 1;
        let result = run_think(world, table, entity, &mut clock);
        stats.called = stats.called.saturating_add(result.called);
        stats.rejected = stats.rejected.saturating_add(result.rejected);
    }
    stats
}
