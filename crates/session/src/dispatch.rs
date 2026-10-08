use qa_core::primitives::{CallbackCall, CallbackId, EntityId, ModuleId, ThinkTime};
use qa_world::entities::EntityTable;

/// A native function, QC function or foreign export binds its boundary adapter
/// once in a numeric module table, without resource-name lookup during calls.
pub struct FunctionBinding<C> {
    pub entry: u32,
    pub call: fn(&mut C, ModuleId, u32, CallbackCall) -> bool,
}

pub struct FunctionTable<C> {
    modules: Box<[Option<ModuleFunctions<C>>]>,
}

struct ModuleFunctions<C> {
    functions: Box<[FunctionBinding<C>]>,
    timing: ThinkTiming,
}

/// Native scheduling arithmetic is selected once for the module, independently
/// of map, movement rules and the clocks of other loaded modules.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ThinkTiming {
    Quake,
    QuakeWorld,
    Quake2,
    Quake2Rerelease,
    Quake3,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ThinkFrame {
    Seconds { now: f64, step: f64 },
    Milliseconds { now: i64 },
}

impl ThinkTiming {
    fn evaluate(self, due: ThinkTime, frame: ThinkFrame) -> Result<Option<ThinkTime>, CallError> {
        match (self, due, frame) {
            (
                Self::Quake | Self::QuakeWorld,
                ThinkTime::Seconds(due),
                ThinkFrame::Seconds { now, step },
            ) => {
                // Q1/QW sv_phys.c SV_RunThink stores thinktime in float, but
                // sv.time + host_frametime and the past-time test are double.
                let mut due = due as f32;
                if due <= 0.0 || f64::from(due) > now + step {
                    return Ok(None);
                }
                if f64::from(due) < now {
                    due = now as f32;
                }
                Ok(Some(ThinkTime::Seconds(f64::from(due))))
            }
            (Self::Quake2, ThinkTime::Seconds(due), ThinkFrame::Seconds { now, .. }) => {
                // Q2 g_phys.c SV_RunThink: both stored times are float; the
                // unsuffixed 0.001 promotes level.time before the addition.
                let due = due as f32;
                let now = now as f32;
                if due <= 0.0 || f64::from(due) > f64::from(now) + 0.001 {
                    return Ok(None);
                }
                Ok(Some(ThinkTime::Seconds(f64::from(now))))
            }
            (
                Self::Quake2Rerelease,
                ThinkTime::Milliseconds(due),
                ThinkFrame::Milliseconds { now },
            ) => {
                // Rerelease gtime_t retains int64_t milliseconds throughout.
                if due <= 0 || due > now {
                    return Ok(None);
                }
                Ok(Some(ThinkTime::Milliseconds(now)))
            }
            (Self::Quake3, ThinkTime::Milliseconds(due), ThinkFrame::Milliseconds { now }) => {
                // Q3 G_RunThink narrows native int nextthink to float, then
                // promotes int level.time to float for comparison. The callback
                // still sees integer level.time, not that rounded float.
                let due = (due as i32) as f32;
                let now = now as i32;
                if due <= 0.0 || due > now as f32 {
                    return Ok(None);
                }
                Ok(Some(ThinkTime::Milliseconds(i64::from(now))))
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
        modules: impl IntoIterator<Item = (ModuleId, ThinkTiming, Vec<FunctionBinding<C>>)>,
    ) -> Result<Self, TableError> {
        let mut tables = Vec::new();
        for (module, timing, functions) in modules {
            if functions.len() > u32::MAX as usize {
                return Err(TableError::FunctionCapacity);
            }
            let slot = module.0 as usize;
            if slot >= tables.len() {
                tables.resize_with(slot + 1, || None);
            }
            if tables[slot].is_some() {
                return Err(TableError::DuplicateModule);
            }
            tables[slot] = Some(ModuleFunctions {
                functions: functions.into_boxed_slice(),
                timing,
            });
        }
        Ok(Self {
            modules: tables.into_boxed_slice(),
        })
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
        let functions = self
            .modules
            .get(module.0 as usize)
            .and_then(Option::as_ref)
            .ok_or(CallError::MissingModule)?;
        let function = functions
            .functions
            .get(callback.0 as usize)
            .ok_or(CallError::MissingFunction)?;
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
        let module = entities.columns.owner[slot];
        let Some(functions) = table
            .modules
            .get(module.0 as usize)
            .and_then(Option::as_ref)
        else {
            result.rejected = result.rejected.saturating_add(1);
            return result;
        };
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
        let callback = entities.columns.think_fn[slot];
        entities.columns.next_think[slot] = None;
        let accepted = callback
            .ok_or(CallError::MissingFunction)
            .and_then(|callback| {
                table.invoke(world, callback, CallbackCall::Think { entity, time })
            });
        result.current_lifetime = world.entities().resolve(entity).is_some();
        if accepted.is_err() {
            result.rejected = result.rejected.saturating_add(1);
            return result;
        }
        result.called = result.called.saturating_add(1);
        if !result.current_lifetime || functions.timing != ThinkTiming::QuakeWorld {
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
