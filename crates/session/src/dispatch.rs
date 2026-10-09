pub use qa_core::primitives::RuleSetId;
use qa_core::primitives::{CallbackCall, CallbackId, EntityId, ModuleId, ThinkBinding, ThinkTime};
use qa_world::entities::EntityTable;

/// A native function, QC function or foreign export binds its boundary adapter
/// once in a numeric module table, without resource-name lookup during calls.
pub struct FunctionBinding<C> {
    pub entry: u32,
    pub call: fn(&mut C, ModuleId, u32, CallbackCall) -> bool,
}

pub struct FunctionTable<C> {
    modules: Box<[Option<ModuleBinding>]>,
    functions: Box<[ResolvedFunction<C>]>,
}

struct ResolvedFunction<C> {
    function: FunctionBinding<C>,
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
            seconds: matches!(
                rules,
                RuleSetId::Quake | RuleSetId::QuakeWorld | RuleSetId::Quake2
            ),
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

impl<C: ThinkWorld> FunctionTable<C> {
    pub fn load(
        modules: impl IntoIterator<Item = (ModuleId, RuleSetId, Vec<FunctionBinding<C>>)>,
    ) -> Result<Self, TableError> {
        let mut tables = Vec::new();
        // Path zero is the unbound/missing-module marker, never a live entry.
        let mut resolved = vec![ResolvedFunction {
            function: FunctionBinding {
                entry: 0,
                call: |_, _, _, _| false,
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
                    call: |_, _, _, _| false,
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

    pub fn module_binding(&self, module: ModuleId) -> ModuleBinding {
        self.modules
            .get(module.0 as usize)
            .copied()
            .flatten()
            .unwrap_or_default()
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

    pub fn invoke(
        &self,
        world: &mut C,
        callback: CallbackId,
        call: CallbackCall,
    ) -> Result<(), CallError> {
        let entities = world.entities();
        let slot = entities
            .resolve(call.entity())
            .ok_or(CallError::StaleEntity)?;
        let module = entities.columns.owner[slot];
        let binding = self
            .modules
            .get(module.0 as usize)
            .copied()
            .flatten()
            .ok_or(CallError::MissingModule)?;
        if callback.0 >= binding.count {
            return Err(CallError::MissingFunction);
        }
        let function = &self.functions[(binding.first + callback.0) as usize].function;
        if (function.call)(world, module, function.entry, call) {
            Ok(())
        } else {
            Err(CallError::Rejected)
        }
    }
}

pub trait ThinkWorld {
    fn entities(&mut self) -> &mut EntityTable;
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
    pub current_lifetime: bool,
}

/// Run one current entity lifetime. A due deadline is cleared independently of
/// its callback. QW repeats successful reschedules before the next source slot;
/// rejection or lifetime removal stops that entity, without a synthetic cap.
pub fn run_think<C: ThinkWorld>(
    world: &mut C,
    table: &FunctionTable<C>,
    entity: EntityId,
    mut clock: impl FnMut(ModuleId) -> Option<ThinkFrame>,
) -> ThinkResult {
    let mut result = ThinkResult::default();
    loop {
        let entities = world.entities();
        let Some(slot) = entities.resolve(entity) else {
            return result;
        };
        result.current_lifetime = true;
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
        entities.columns.next_think[slot] = None;
        let accepted = (functions.function.call)(
            world,
            module,
            functions.function.entry,
            CallbackCall::Think { entity, time },
        );
        result.current_lifetime = world.entities().resolve(entity).is_some();
        if !accepted {
            result.rejected = result.rejected.saturating_add(1);
            return result;
        }
        result.called = result.called.saturating_add(1);
        if !result.current_lifetime || !functions.timing.repeat {
            return result;
        }
    }
}

/// Serial source-slot order; each entity resolves its owning module's frame.
/// Placement among native physics operations belongs to the module caller.
pub fn run_thinks<C: ThinkWorld>(
    world: &mut C,
    table: &FunctionTable<C>,
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
