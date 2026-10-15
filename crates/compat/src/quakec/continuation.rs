//! Owned stops over the single QuakeC opcode loop and its native local stack.
use super::{EntityAccess, Trap, Vm};
use std::sync::Arc;

/// Raw parameter triplets admitted by one executed CALL instruction. Strings
/// and entities remain native words, without borrowing their backing stores.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BuiltinCall {
    pub number: u32,
    pub(super) argc: usize,
    pub(super) arguments: [[u32; 3]; 8],
}
impl BuiltinCall {
    pub(crate) fn capture(vm: &Vm, number: u32, argc: usize) -> Result<Self, Trap> {
        if argc > 8 {
            return Err(Trap::Builtin);
        }
        let mut arguments = [[0; 3]; 8];
        for (index, to) in arguments[..argc].iter_mut().enumerate() {
            to.copy_from_slice(
                vm.image
                    .globals
                    .get(4 + index * 3..7 + index * 3)
                    .ok_or(Trap::Memory)?,
            );
        }
        Ok(Self {
            number,
            argc,
            arguments,
        })
    }
    pub fn argc(&self) -> usize {
        self.argc
    }
    pub fn arguments(&self) -> &[[u32; 3]] {
        &self.arguments[..self.argc]
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    Builtin(BuiltinCall),
    Returned([u32; 3]),
}

/// Complete, trap or abort a run before discarding it. No VM or service borrow
/// survives a builtin stop, and native locals remain in the owning VM's arena.
#[must_use]
#[derive(Debug)]
pub struct Run {
    owner: Arc<()>,
    state: Option<RunState>,
}

#[derive(Clone, Copy, Debug, Default)]
pub(super) struct Settings {
    pub remaining: u64,
    pub hooks: bool,
}

#[derive(Debug)]
pub(super) struct RunState {
    id: u64,
    parent: Option<u64>,
    pub pc: usize,
    pub exit_depth: usize,
    saved_statement: usize,
    saved_argc: usize,
    saved_settings: Option<Settings>,
}

pub(super) struct ActiveRun {
    id: u64,
    suspended: bool,
}

impl Vm {
    pub fn is_running(&self) -> bool {
        self.active_run.is_some()
    }
    /// Derive host activation storage from the existing native frame bound.
    pub fn activation_capacity(&self) -> usize {
        self.stack.len()
    }
    /// Explicit calls select fresh settings, including legacy calls made from
    /// a builtin. Their parent's remaining settings are restored on completion.
    pub fn begin_root(&mut self, function: u32, budget: u64, hooks: bool) -> Result<Run, Trap> {
        self.begin(
            function,
            Some(Settings {
                remaining: budget,
                hooks,
            }),
        )
    }
    /// Owned callbacks share an active VM's remaining budget and hook choice.
    pub fn begin_callback(&mut self, function: u32, budget: u64, hooks: bool) -> Result<Run, Trap> {
        if self.is_running() {
            self.begin(function, None)
        } else {
            self.begin_root(function, budget, hooks)
        }
    }
    fn begin(&mut self, function: u32, settings: Option<Settings>) -> Result<Run, Trap> {
        if self
            .active_run
            .as_ref()
            .is_some_and(|active| !active.suspended)
        {
            return Err(Trap::Continuation);
        }
        let id = self.next_run.checked_add(1).ok_or(Trap::Continuation)?;
        let exit_depth = self.depth;
        let saved_statement = self.statement;
        let saved_argc = self.argc;
        // push validates function/frame/local extents before changing them.
        // A rejected nested entry leaves its suspended parent unchanged.
        let pc = self.push(function as usize, 0)?;
        let saved_settings =
            settings.map(|settings| std::mem::replace(&mut self.settings, settings));
        let state = RunState {
            id,
            parent: self.active_run.as_ref().map(|active| active.id),
            pc,
            exit_depth,
            saved_statement,
            saved_argc,
            saved_settings,
        };
        self.next_run = id;
        self.active_run = Some(ActiveRun {
            id,
            suspended: false,
        });
        Ok(Run {
            owner: Arc::clone(&self.run_owner),
            state: Some(state),
        })
    }
    fn check_run(&self, run: &Run, suspended: Option<bool>) -> Result<(), Trap> {
        if !Arc::ptr_eq(&self.run_owner, &run.owner) {
            return Err(Trap::Continuation);
        }
        let state = run.state.as_ref().ok_or(Trap::Continuation)?;
        let active = self.active_run.as_ref().ok_or(Trap::Continuation)?;
        if active.id != state.id || suspended.is_some_and(|waiting| waiting != active.suspended) {
            return Err(Trap::Continuation);
        }
        Ok(())
    }
    fn finish(&mut self, run: &mut Run) -> Result<(), Trap> {
        let state = run.state.take().ok_or(Trap::Continuation)?;
        while self.depth > state.exit_depth {
            self.leave();
        }
        self.statement = state.saved_statement;
        self.argc = state.saved_argc;
        if let Some(settings) = state.saved_settings {
            self.settings = settings;
        }
        self.active_run = state.parent.map(|id| ActiveRun {
            id,
            suspended: true,
        });
        Ok(())
    }
    pub fn advance(&mut self, run: &mut Run, access: &EntityAccess<'_>) -> Result<Step, Trap> {
        self.check_run(run, Some(false))?;
        self.advance_ready(run, access)
    }
    /// The builtin adapter has already applied any return/global writes. Void
    /// builtins leave return cells unchanged; resume never synthesizes a value.
    pub fn resume(&mut self, run: &mut Run, access: &EntityAccess<'_>) -> Result<Step, Trap> {
        self.check_run(run, Some(true))?;
        if let Some(active) = &mut self.active_run {
            active.suspended = false;
        }
        self.advance_ready(run, access)
    }
    pub fn abort(&mut self, run: &mut Run) -> Result<(), Trap> {
        self.check_run(run, None)?;
        self.finish(run)
    }
    fn advance_ready(&mut self, run: &mut Run, access: &EntityAccess<'_>) -> Result<Step, Trap> {
        let state = run.state.as_mut().ok_or(Trap::Continuation)?;
        let result = if self.settings.hooks {
            self.run::<true>(state, access)
        } else {
            self.run::<false>(state, access)
        };
        match result {
            Ok(Step::Builtin(call)) => {
                if let Some(active) = &mut self.active_run {
                    active.suspended = true;
                }
                Ok(Step::Builtin(call))
            }
            result => {
                self.finish(run)?;
                result
            }
        }
    }
}
