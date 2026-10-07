use qa_core::primitives::{CallbackCall, CallbackId, ModuleId};
use qa_world::entities::EntityTable;

/// A native function, QC function or foreign export binds its boundary adapter
/// once. Dispatch has neither a game discriminator nor a resource-name lookup.
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

#[derive(Clone, Copy, Debug)]
pub enum ThinkTiming {
    FrameEnd,
    Current { tolerance: f64 },
}

impl ThinkTiming {
    fn cutoff(self, now: f64, frame_end: f64) -> f64 {
        match self {
            Self::FrameEnd => frame_end,
            Self::Current { tolerance } => now + tolerance,
        }
    }
    fn callback_time(self, now: f64, due: f64) -> f64 {
        match self {
            Self::FrameEnd => due.max(now),
            Self::Current { .. } => now,
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum TableError {
    DuplicateModule,
    FunctionCapacity,
    ThinkTiming,
}

#[derive(Debug, PartialEq, Eq)]
pub enum CallError {
    MissingModule,
    MissingFunction,
    StaleEntity,
    Rejected,
}

impl<C: ThinkWorld> FunctionTable<C> {
    pub fn load(
        modules: impl IntoIterator<Item = (ModuleId, ThinkTiming, Vec<FunctionBinding<C>>)>,
    ) -> Result<Self, TableError> {
        let mut tables = Vec::new();
        for (module, timing, functions) in modules {
            if functions.len() > 65536 {
                return Err(TableError::FunctionCapacity);
            }
            if let ThinkTiming::Current { tolerance } = timing
                && (!tolerance.is_finite() || !(0.0..=0.1).contains(&tolerance))
            {
                return Err(TableError::ThinkTiming);
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

#[derive(Default, Debug)]
pub struct DispatchStats {
    pub called: u32,
    pub rejected: u32,
}

/// Load Q1's FrameEnd, Q2's Current { tolerance: 0.001 }, or Q3's Current
/// { tolerance: 0.0 } on the owning module. Mixed entities use their own clock
/// policy. Clear before calling and inspect the current lifetime at each slot.
pub fn run_thinks<C: ThinkWorld>(
    world: &mut C,
    table: &FunctionTable<C>,
    now: f64,
    frame_end: f64,
) -> DispatchStats {
    let mut stats = DispatchStats::default();
    let capacity = world.entities().capacity();
    for slot in 0..capacity {
        let entities = world.entities();
        let Some(entity) = entities.id_at(slot) else {
            continue;
        };
        let Some(think) = entities.columns.next_think[slot] else {
            continue;
        };
        let module = entities.columns.owner[slot];
        let timing = table
            .modules
            .get(module.0 as usize)
            .and_then(Option::as_ref)
            .map(|module| module.timing)
            .unwrap_or(ThinkTiming::FrameEnd);
        if think.at <= 0.0 || think.at > timing.cutoff(now, frame_end) {
            continue;
        }
        entities.columns.next_think[slot] = None;
        let call = CallbackCall::Think {
            entity,
            time: timing.callback_time(now, think.at),
        };
        if table.invoke(world, think.callback, call).is_ok() {
            stats.called += 1;
        } else {
            stats.rejected += 1;
        }
    }
    stats
}
