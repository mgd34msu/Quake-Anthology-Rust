//! Port of `src/compat/qc/machine.ts` (Quake `pr_exec.c`, id Software,
//! GPL-2.0-or-later): the QuakeC interpreter: call frames, statement
//! dispatch, function/inline host boundaries, and idle-boundary snapshots.
//!
//! Rust adaptations (documented, behavior-preserving): thrown failures
//! become [`GuestError`] (machine context is prefixed to the message);
//! the donor's `run(call, execute)` boundary closures become traits that
//! receive `&mut QcMachine` directly ([`QcFunctionBoundary::run`] returns
//! [`QcBoundaryAction::Enter`] or [`QcBoundaryAction::Skip`], and
//! [`QcMachine::cancel_source_function`] replaces the thrown cancellation);
//! hook callbacks live behind `Rc` so reentrant builtins never alias.
//! Hot statement dispatch allocates nothing per step.

use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use qa_core::math::Vec3;
use qa_core::numeric::NumericOps;

use super::memory::{QcEntityMemory, QcStrings, QcWords};
use super::program::{signed_qc_branch, QcFunction, QcOpcode, QcProgram};
use crate::error::GuestError;

/// A builtin implementation: synchronous, may reenter this same machine.
pub type QcBuiltin = Rc<dyn Fn(&mut QcMachine) -> Result<(), GuestError>>;

/// Per-statement tracer (runs only while `trace_enabled`).
pub type QcTraceHook = Rc<dyn Fn(&QcMachine)>;

/// Guest-call observer.
pub type QcObserveCallHook = Rc<dyn Fn(&QcCallSite)>;

/// Entity-store observer.
pub type QcObserveStoreHook = Rc<dyn Fn(&QcEntityStoreObservation)>;

/// Entity-access validator (reads and writes).
pub type QcValidateAccessHook = Rc<dyn Fn(i32, usize, u8, QcAccessKind) -> Result<(), GuestError>>;

/// Numbered (`-firstStatement`) and named (`ex_*`) builtin bindings.
#[derive(Clone, Default)]
pub struct QcBuiltinRegistry {
    /// Builtins by number.
    pub numbered: HashMap<i32, QcBuiltin>,
    /// Builtins by name.
    pub named: HashMap<String, QcBuiltin>,
}

/// Entity-store observation for save/compare hooks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QcEntityStoreObservation {
    /// Executing function index.
    pub function_index: usize,
    /// Executing statement.
    pub statement: usize,
    /// Entity byte reference.
    pub reference: i32,
    /// First variable word written.
    pub word: usize,
    /// Bytes before the store.
    pub before: Vec<u8>,
    /// Bytes after the store.
    pub after: Vec<u8>,
}

/// A guest call site observed or intercepted by the host.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QcCallSite {
    /// Called function index.
    pub function_index: usize,
    /// Calling function index.
    pub caller: usize,
    /// Calling statement.
    pub statement: usize,
}

/// Decision returned by [`QcFunctionBoundary::run`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QcBoundaryAction {
    /// Run the original source function.
    Enter,
    /// Skip the source function, returning these words (1..=3).
    Skip {
        /// Replacement return words.
        return_words: [i32; 3],
    },
}

/// Host boundary over admitted source functions. `run` may inspect and
/// stage guest state, reenter QuakeC, then return [`QcBoundaryAction::Enter`]
/// or skip the original with replacement return words.
pub trait QcFunctionBoundary {
    /// Whether `function_index` is intercepted.
    fn contains(&self, function_index: usize) -> bool;
    /// Intercept one call.
    fn run(&self, machine: &mut QcMachine, call: &QcCallSite) -> Result<QcBoundaryAction, GuestError>;
}

/// Scope of a standalone inline result word.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QcInlineScope {
    /// A parameter/local word of the region's function.
    Frame,
    /// A float global word (restored after standalone execution).
    Global,
}

/// Standalone result of an inline region.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QcInlineStandalone {
    /// Result word.
    pub saved: usize,
    /// Result scope.
    pub scope: QcInlineScope,
}

/// An admitted inline source region `[entry, exit)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QcInlineRegion {
    /// Owning function index.
    pub function_index: usize,
    /// First statement of the region.
    pub entry: usize,
    /// Join statement (excluded from the region).
    pub exit: usize,
    /// Whether the host may replace the region with a jump to the join.
    pub replaceable: bool,
    /// Standalone execution result, if the region can run alone.
    pub standalone: Option<QcInlineStandalone>,
}

/// Decision returned by [`QcInlineBoundary::run`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QcInlineAction {
    /// Execute the original region statements.
    Run,
    /// Jump to the join, skipping the region (requires `replaceable`).
    SkipToJoin,
}

/// Host boundary over admitted inline regions.
pub trait QcInlineBoundary {
    /// Admitted regions (validated at machine construction).
    fn regions(&self) -> &[QcInlineRegion];
    /// Intercept one region entry.
    fn run(&self, machine: &mut QcMachine, region: &QcInlineRegion) -> Result<QcInlineAction, GuestError>;
}

/// Entity-access direction for validation hooks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QcAccessKind {
    /// Word read.
    Read,
    /// Word write.
    Write,
}

/// Machine construction options. Hooks are optional; limits default to the
/// donor values (100000 statements, 32 frames, 2048 local words).
pub struct QcMachineOptions {
    /// Loaded program.
    pub program: QcProgram,
    /// Selected arithmetic.
    pub numeric: NumericOps,
    /// Edict storage (field layout must match the program).
    pub entities: QcEntityMemory,
    /// Builtin bindings.
    pub builtins: QcBuiltinRegistry,
    /// Whether the server is active (world-entity write guard).
    pub server_active: Rc<dyn Fn() -> bool>,
    /// Statement budget per top-level execution.
    pub statement_limit: usize,
    /// Call-frame limit.
    pub stack_limit: usize,
    /// Local-word limit across live frames.
    pub local_stack_words: usize,
    /// Per-statement tracer (runs only while `trace_enabled`).
    pub trace: Option<QcTraceHook>,
    /// Call observer (runs before argument staging is restored).
    pub observe_call: Option<QcObserveCallHook>,
    /// Function boundary over admitted functions.
    pub function_boundary: Option<Rc<dyn QcFunctionBoundary>>,
    /// Inline boundary over admitted regions.
    pub inline_boundary: Option<Rc<dyn QcInlineBoundary>>,
    /// Entity-store observer.
    pub observe_entity_store: Option<QcObserveStoreHook>,
    /// Entity-access validator (reads and writes).
    pub validate_entity_access: Option<QcValidateAccessHook>,
}

impl QcMachineOptions {
    /// Options with donor-default limits and no hooks.
    pub fn new(
        program: QcProgram,
        numeric: NumericOps,
        entities: QcEntityMemory,
        builtins: QcBuiltinRegistry,
        server_active: Rc<dyn Fn() -> bool>,
    ) -> Self {
        Self {
            program,
            numeric,
            entities,
            builtins,
            server_active,
            statement_limit: 100_000,
            stack_limit: 32,
            local_stack_words: 2048,
            trace: None,
            observe_call: None,
            function_boundary: None,
            inline_boundary: None,
            observe_entity_store: None,
            validate_entity_access: None,
        }
    }
}

/// Idle-boundary machine snapshot (globals, entities, strings, registers).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QcMachineSnapshot {
    /// Global words image.
    pub globals: Vec<u8>,
    /// Entity record image.
    pub entities: Vec<u8>,
    /// Live entity count.
    pub entity_count: usize,
    /// String-arena image.
    pub strings: Vec<u8>,
    /// Current statement.
    pub statement: usize,
    /// Current function (always 0 at an idle boundary).
    pub function_index: usize,
    /// Argument count.
    pub argument_count: usize,
    /// Per-function execution counters.
    pub profiling: Vec<u32>,
    /// Trace flag.
    pub trace_enabled: bool,
}

/// A builtin the program needs but the registry does not bind.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QcMissingBuiltin {
    /// Declaring function index.
    pub function_index: usize,
    /// Builtin name.
    pub name: String,
    /// Builtin number, or `None` for named builtins.
    pub number: Option<i32>,
}

struct CallStaging {
    words: Vec<u8>,
    argument_count: usize,
}

#[derive(Debug, Clone)]
struct Frame {
    statement: usize,
    function_index: usize,
    locals: Vec<u8>,
}

/// Internal control flow: guest failures plus boundary cancellation.
#[derive(Debug)]
enum Flow {
    Guest(GuestError),
    Cancelled,
}

impl From<GuestError> for Flow {
    fn from(error: GuestError) -> Self {
        Self::Guest(error)
    }
}

type FlowResult<T> = Result<T, Flow>;

/// The QuakeC interpreter. One instance per module; builtins may
/// synchronously reenter this same machine.
pub struct QcMachine {
    program: QcProgram,
    globals: QcWords,
    strings: QcStrings,
    entities: QcEntityMemory,
    numeric: NumericOps,
    profiling: Vec<u32>,
    builtins: QcBuiltinRegistry,
    server_active: Rc<dyn Fn() -> bool>,
    trace: Option<QcTraceHook>,
    observe_call: Option<QcObserveCallHook>,
    function_boundary: Option<Rc<dyn QcFunctionBoundary>>,
    inline_boundary: Option<Rc<dyn QcInlineBoundary>>,
    observe_entity_store: Option<QcObserveStoreHook>,
    validate_entity_access: Option<QcValidateAccessHook>,
    frames: Vec<Frame>,
    statement_limit: usize,
    stack_limit: usize,
    local_stack_limit: usize,
    local_words: usize,
    function_index: usize,
    statement: usize,
    argument_count: usize,
    builtin_depth: usize,
    boundary_depth: usize,
    boundary_seqs: u64,
    current_runs: Vec<u64>,
    pending_cancel: Option<(u64, [i32; 3])>,
    boundary_functions: HashSet<usize>,
    inline_regions: HashMap<usize, QcInlineRegion>,
    /// Trace flag; builtins 29/30 toggle it, `execute` clears it.
    pub trace_enabled: bool,
}

impl QcMachine {
    /// Build a machine, validating boundaries and limits.
    pub fn new(options: QcMachineOptions) -> Result<Self, GuestError> {
        if options.entities.layout().field_words != options.program.entity_field_words {
            return Err(GuestError::invalid("entity field layout disagrees with program"));
        }
        let globals = QcWords::new(options.program.initial_globals.clone())?;
        let strings = QcStrings::new(&options.program.strings, options.program.api.is_quakeworld());
        let profiling = vec![0; options.program.functions.len()];
        let mut boundary_functions = HashSet::new();
        if let Some(boundary) = &options.function_boundary {
            // Boundary membership is validated eagerly; probe every function.
            for index in 1..options.program.functions.len() {
                if boundary.contains(index) {
                    let function = options
                        .program
                        .function_at(index)
                        .map_err(|_| GuestError::invalid("function boundary requires an interpreted function"))?;
                    if function.named_builtin || function.first_statement < 0 {
                        return Err(GuestError::invalid(
                            "function boundary requires an interpreted function",
                        ));
                    }
                    boundary_functions.insert(index);
                }
            }
        }
        let mut inline_regions = HashMap::new();
        if let Some(boundary) = &options.inline_boundary {
            for region in boundary.regions() {
                let function = options
                    .program
                    .function_at(region.function_index)
                    .map_err(|_| GuestError::invalid("invalid inline source region"))?;
                let end = options.program.function_end(function);
                if function.named_builtin
                    || function.first_statement <= 0
                    || region.entry < function.first_statement as usize
                    || region.exit <= region.entry
                    || region.exit >= end
                    || inline_regions.contains_key(&region.entry)
                {
                    return Err(GuestError::invalid("invalid inline source region"));
                }
                if let Some(standalone) = &region.standalone {
                    let valid = match standalone.scope {
                        QcInlineScope::Global => {
                            standalone.saved >= 28
                                && standalone.saved * 4 + 4 <= globals.bytes().len()
                                && !(standalone.saved >= function.parameter_start
                                    && standalone.saved < function.parameter_start + function.local_words)
                                && options.program.globals.iter().any(|global| {
                                    global.offset == standalone.saved
                                        && global.value_type == super::program::QcValueType::Float
                                })
                        }
                        QcInlineScope::Frame => {
                            standalone.saved >= function.parameter_start
                                && standalone.saved < function.parameter_start + function.local_words
                        }
                    };
                    if !valid {
                        return Err(GuestError::invalid("invalid standalone inline result"));
                    }
                }
                inline_regions.insert(region.entry, *region);
            }
        }
        for limit in [options.statement_limit, options.stack_limit, options.local_stack_words] {
            if limit == 0 {
                return Err(GuestError::invalid("Invalid QuakeC execution limits"));
            }
        }
        Ok(Self {
            program: options.program,
            globals,
            strings,
            entities: options.entities,
            numeric: options.numeric,
            profiling,
            builtins: options.builtins,
            server_active: options.server_active,
            trace: options.trace,
            observe_call: options.observe_call,
            function_boundary: options.function_boundary,
            inline_boundary: options.inline_boundary,
            observe_entity_store: options.observe_entity_store,
            validate_entity_access: options.validate_entity_access,
            frames: Vec::new(),
            statement_limit: options.statement_limit,
            stack_limit: options.stack_limit,
            local_stack_limit: options.local_stack_words,
            local_words: 0,
            function_index: 0,
            statement: 0,
            argument_count: 0,
            builtin_depth: 0,
            boundary_depth: 0,
            boundary_seqs: 0,
            current_runs: Vec::new(),
            pending_cancel: None,
            boundary_functions,
            inline_regions,
            trace_enabled: false,
        })
    }

    /// Loaded program.
    #[must_use]
    pub fn program(&self) -> &QcProgram {
        &self.program
    }

    /// Global words.
    #[must_use]
    pub fn globals(&self) -> &QcWords {
        &self.globals
    }

    /// Mutable global words.
    pub fn globals_mut(&mut self) -> &mut QcWords {
        &mut self.globals
    }

    /// String arena.
    #[must_use]
    pub fn strings(&self) -> &QcStrings {
        &self.strings
    }

    /// Mutable string arena.
    pub fn strings_mut(&mut self) -> &mut QcStrings {
        &mut self.strings
    }

    /// Edict storage.
    #[must_use]
    pub fn entities(&self) -> &QcEntityMemory {
        &self.entities
    }

    /// Mutable edict storage.
    pub fn entities_mut(&mut self) -> &mut QcEntityMemory {
        &mut self.entities
    }

    /// Selected arithmetic.
    #[must_use]
    pub const fn numeric(&self) -> NumericOps {
        self.numeric
    }

    /// Per-function execution counters.
    #[must_use]
    pub fn profiling(&self) -> &[u32] {
        &self.profiling
    }

    /// Current argument count.
    #[must_use]
    pub const fn argc(&self) -> usize {
        self.argument_count
    }

    /// Current statement.
    #[must_use]
    pub const fn current_statement(&self) -> usize {
        self.statement
    }

    /// Current function index.
    #[must_use]
    pub const fn current_function(&self) -> usize {
        self.function_index
    }

    /// Live frame depth.
    #[must_use]
    pub fn depth(&self) -> usize {
        self.frames.len()
    }

    /// Build a contextual guest failure. Callers return it; nothing panics
    /// on guest input.
    pub fn fail(&self, message: impl Into<String>) -> GuestError {
        GuestError::cpu(format!(
            "QuakeC {}:{}: {}",
            self.function_index,
            self.statement,
            message.into()
        ))
    }

    /// Word offset of a global.
    pub fn global_offset(&self, name: &str) -> Result<usize, GuestError> {
        self.program
            .global_named(name)
            .map(|definition| definition.offset)
            .ok_or_else(|| self.fail(format!("missing global {name}")))
    }

    /// Word offset of an entity field.
    pub fn field_offset(&self, name: &str) -> Result<usize, GuestError> {
        self.program
            .field_named(name)
            .map(|definition| definition.offset)
            .ok_or_else(|| self.fail(format!("missing entity field {name}")))
    }

    /// Read an entity float field.
    pub fn entity_float(&self, reference: i32, field: &str) -> Result<f32, GuestError> {
        let word = self.field_offset(field)?;
        self.validate_entity(reference, word, 1, QcAccessKind::Read)?;
        let slot = self
            .entities
            .slot(reference)
            .map_err(|_| self.fail(format!("invalid entity reference {reference}")))?;
        self.entities
            .slot_float(slot, word)
            .map_err(|error| self.fail(error.to_string()))
    }

    /// Read an entity int field.
    pub fn entity_int(&self, reference: i32, field: &str) -> Result<i32, GuestError> {
        let word = self.field_offset(field)?;
        self.validate_entity(reference, word, 1, QcAccessKind::Read)?;
        let slot = self
            .entities
            .slot(reference)
            .map_err(|_| self.fail(format!("invalid entity reference {reference}")))?;
        self.entities
            .slot_int(slot, word)
            .map_err(|error| self.fail(error.to_string()))
    }

    /// Read an entity vector field.
    pub fn entity_vector(&self, reference: i32, field: &str) -> Result<Vec3, GuestError> {
        let word = self.field_offset(field)?;
        self.validate_entity(reference, word, 3, QcAccessKind::Read)?;
        let slot = self
            .entities
            .slot(reference)
            .map_err(|_| self.fail(format!("invalid entity reference {reference}")))?;
        self.entities
            .slot_vector(slot, word)
            .map_err(|error| self.fail(error.to_string()))
    }

    /// Write an entity float field, observing the store.
    pub fn set_entity_float(&mut self, reference: i32, field: &str, value: f32) -> Result<(), GuestError> {
        let word = self.field_offset(field)?;
        self.store_entity(reference, word, 1, |entities, slot| {
            entities.set_slot_float(slot, word, value)
        })
    }

    /// Write an entity int field, observing the store.
    pub fn set_entity_int(&mut self, reference: i32, field: &str, value: i32) -> Result<(), GuestError> {
        let word = self.field_offset(field)?;
        self.store_entity(reference, word, 1, |entities, slot| {
            entities.set_slot_int(slot, word, value)
        })
    }

    /// Write an entity vector field, observing the store.
    pub fn set_entity_vector(&mut self, reference: i32, field: &str, value: Vec3) -> Result<(), GuestError> {
        let word = self.field_offset(field)?;
        self.store_entity(reference, word, 3, |entities, slot| {
            entities.set_slot_vector(slot, word, value)
        })
    }

    fn validate_entity(&self, reference: i32, word: usize, words: u8, kind: QcAccessKind) -> Result<(), GuestError> {
        if let Some(validate) = self.validate_entity_access.clone() {
            validate(reference, word, words, kind).map_err(|error| self.fail(error.to_string()))?;
        }
        Ok(())
    }

    fn store_entity(
        &mut self,
        reference: i32,
        word: usize,
        words: u8,
        store: impl FnOnce(&mut QcEntityMemory, u32) -> Result<(), GuestError>,
    ) -> Result<(), GuestError> {
        self.validate_entity(reference, word, words, QcAccessKind::Write)?;
        if word + words as usize > self.entities.layout().field_words {
            return Err(self.fail(format!("entity word {word} outside variable storage")));
        }
        let slot = self
            .entities
            .slot(reference)
            .map_err(|_| self.fail(format!("invalid entity reference {reference}")))?;
        let observe = self.observe_entity_store.clone();
        let before = if observe.is_some() {
            let fields = self
                .entities
                .field_bytes(slot)
                .map_err(|error| self.fail(error.to_string()))?;
            Some(fields[word * 4..(word + words as usize) * 4].to_vec())
        } else {
            None
        };
        store(&mut self.entities, slot).map_err(|error| self.fail(error.to_string()))?;
        if let (Some(observe), Some(before)) = (observe, before) {
            let fields = self
                .entities
                .field_bytes(slot)
                .map_err(|error| self.fail(error.to_string()))?;
            observe(&QcEntityStoreObservation {
                function_index: self.function_index,
                statement: self.statement,
                reference,
                word,
                before,
                after: fields[word * 4..(word + words as usize) * 4].to_vec(),
            });
        }
        Ok(())
    }

    fn parameter_offset(&self, index: usize) -> Result<usize, GuestError> {
        if index >= 8 {
            return Err(self.fail(format!("invalid parameter {index}")));
        }
        Ok(4 + index * 3)
    }

    /// Read float argument `index`.
    pub fn arg_float(&self, index: usize) -> Result<f32, GuestError> {
        let offset = self.parameter_offset(index)?;
        self.globals.float(offset).map_err(|error| self.fail(error.to_string()))
    }

    /// Read int argument `index`.
    pub fn arg_int(&self, index: usize) -> Result<i32, GuestError> {
        let offset = self.parameter_offset(index)?;
        self.globals.int(offset).map_err(|error| self.fail(error.to_string()))
    }

    /// Read vector argument `index`.
    pub fn arg_vector(&self, index: usize) -> Result<Vec3, GuestError> {
        let offset = self.parameter_offset(index)?;
        self.globals
            .vector(offset)
            .map_err(|error| self.fail(error.to_string()))
    }

    /// Read string argument `index`.
    pub fn arg_string(&self, index: usize) -> Result<String, GuestError> {
        let reference = self.arg_int(index)?;
        self.strings
            .get(reference)
            .map_err(|error| self.fail(error.to_string()))
    }

    /// Return a float (words 1..=3 hold the single value at word 1).
    pub fn return_float(&mut self, value: f32) -> Result<(), GuestError> {
        self.globals
            .set_float(1, value)
            .map_err(|error| self.fail(error.to_string()))
    }

    /// Return an int.
    pub fn return_int(&mut self, value: i32) -> Result<(), GuestError> {
        self.globals
            .set_int(1, value)
            .map_err(|error| self.fail(error.to_string()))
    }

    /// Return a vector.
    pub fn return_vector(&mut self, value: Vec3) -> Result<(), GuestError> {
        self.globals
            .set_vector(1, value)
            .map_err(|error| self.fail(error.to_string()))
    }

    /// Concatenate string arguments from `first`.
    pub fn var_string(&self, first: usize) -> Result<String, GuestError> {
        let mut result = String::new();
        for index in first..self.argc() {
            result.push_str(&self.arg_string(index)?);
        }
        Ok(result)
    }

    /// Entity slot of the `self` global.
    pub fn self_slot(&self) -> Result<u32, GuestError> {
        let reference = self
            .globals
            .int(self.global_offset("self")?)
            .map_err(|error| self.fail(error.to_string()))?;
        self.entities
            .slot(reference)
            .map_err(|_| self.fail(format!("invalid entity reference {reference}")))
    }

    /// Builtins the program needs but the registry does not bind.
    pub fn missing_builtins(&self) -> Vec<QcMissingBuiltin> {
        let mut missing = Vec::new();
        for function in &self.program.functions {
            if function.first_statement < 0 && !self.builtins.numbered.contains_key(&-function.first_statement) {
                missing.push(QcMissingBuiltin {
                    function_index: function.index,
                    name: function.name.clone(),
                    number: Some(-function.first_statement),
                });
            } else if function.named_builtin && !self.builtins.named.contains_key(&function.name) {
                missing.push(QcMissingBuiltin {
                    function_index: function.index,
                    name: function.name.clone(),
                    number: None,
                });
            }
        }
        missing
    }

    /// Request cancellation of the innermost live boundary execution with
    /// replacement return words. The interpreter unwinds to that boundary
    /// at the next step; host code must propagate errors promptly.
    pub fn cancel_source_function(&mut self, return_words: [i32; 3]) -> Result<(), GuestError> {
        match self.current_runs.last().copied() {
            Some(run) => {
                self.pending_cancel = Some((run, return_words));
                Ok(())
            }
            None => Err(self.fail("function cancellation requires its live source execution")),
        }
    }

    fn builtin(&self, function: &QcFunction) -> Result<Option<QcBuiltin>, GuestError> {
        if !function.named_builtin && function.first_statement >= 0 {
            return Ok(None);
        }
        let builtin = if function.named_builtin {
            self.builtins.named.get(&function.name)
        } else {
            self.builtins.numbered.get(&-function.first_statement)
        };
        match builtin {
            Some(builtin) => Ok(Some(builtin.clone())),
            None => Err(self.fail(format!(
                "unbound builtin {} ({})",
                function.name,
                if function.named_builtin {
                    "named".to_string()
                } else {
                    (-function.first_statement).to_string()
                }
            ))),
        }
    }

    fn call_builtin(&mut self, builtin: QcBuiltin) -> FlowResult<()> {
        self.builtin_depth += 1;
        let result = builtin(self).map_err(Flow::Guest);
        self.builtin_depth -= 1;
        result
    }

    fn enter(&mut self, function: &QcFunction) -> FlowResult<()> {
        if self.frames.len() + 1 >= self.stack_limit {
            return Err(Flow::Guest(self.fail("stack overflow")));
        }
        if self.local_words + function.local_words > self.local_stack_limit {
            return Err(Flow::Guest(self.fail("locals stack overflow")));
        }
        let begin = function.parameter_start * 4;
        let end = begin + function.local_words * 4;
        if end > self.globals.bytes().len() {
            return Err(Flow::Guest(self.fail("invalid parameter layout")));
        }
        self.frames.push(Frame {
            statement: self.statement,
            function_index: self.function_index,
            locals: self.globals.bytes()[begin..end].to_vec(),
        });
        self.local_words += function.local_words;
        let mut destination = function.parameter_start;
        for (parameter, size) in function.parameter_sizes.iter().enumerate() {
            let source = 4 + parameter * 3;
            self.globals
                .copy_within(source, destination, *size as usize)
                .map_err(|_| Flow::Guest(self.fail("invalid parameter layout")))?;
            destination += *size as usize;
        }
        self.function_index = function.index;
        // A zero entry behaves like the donor (`statement = -1`, so the
        // loop opens at statement 0); wrapping avoids any debug panic.
        self.statement = (function.first_statement as usize).wrapping_sub(1);
        Ok(())
    }

    fn leave(&mut self) -> FlowResult<()> {
        let function = self
            .program
            .function_at(self.function_index)
            .map_err(|_| Flow::Guest(self.fail("stack underflow")))?
            .clone();
        let frame = self
            .frames
            .pop()
            .ok_or_else(|| Flow::Guest(self.fail("stack underflow")))?;
        let begin = function.parameter_start * 4;
        if begin + frame.locals.len() > self.globals.bytes().len() {
            return Err(Flow::Guest(self.fail("stack underflow")));
        }
        self.globals.bytes_mut()[begin..begin + frame.locals.len()].copy_from_slice(&frame.locals);
        self.local_words -= function.local_words;
        self.function_index = frame.function_index;
        self.statement = frame.statement;
        Ok(())
    }

    fn unwind_to(&mut self, depth: usize) -> FlowResult<()> {
        while self.frames.len() > depth {
            self.leave()?;
        }
        Ok(())
    }

    /// Execute a function with staged arguments. Clears the trace flag,
    /// like the donor.
    pub fn execute(&mut self, function_index: usize, argument_count: usize) -> Result<(), GuestError> {
        if argument_count > 8 {
            return Err(self.fail("invalid argument count"));
        }
        let function = self
            .program
            .function_at(function_index)
            .map_err(|_| self.fail(format!("invalid function {function_index}")))?
            .clone();
        self.argument_count = argument_count;
        self.trace_enabled = false;
        let call = QcCallSite {
            function_index,
            caller: self.function_index,
            statement: self.statement,
        };
        let staging = self.capture_call_staging();
        if let Some(observe) = self.observe_call.clone() {
            observe(&call);
        }
        self.restore_call_staging(&staging);
        let mut budget = self.statement_limit;
        self.invoke_function(&function, &call, &mut budget, &staging, None)
            .map_err(|flow| match flow {
                Flow::Guest(error) => error,
                Flow::Cancelled => self.fail("stray function cancellation"),
            })
    }

    /// Execute an admitted standalone inline region, returning its result
    /// word. Global results are restored afterwards.
    pub fn execute_region(&mut self, region: &QcInlineRegion, argument_count: usize) -> Result<f32, GuestError> {
        let admitted = self
            .inline_regions
            .get(&region.entry)
            .copied()
            .filter(|admitted| admitted.function_index == region.function_index && admitted.exit == region.exit);
        let Some(admitted) = admitted else {
            return Err(self.fail("standalone inline execution requires an admitted source region"));
        };
        let Some(standalone) = admitted.standalone else {
            return Err(self.fail("standalone inline execution requires an admitted source region"));
        };
        let function = self
            .program
            .function_at(region.function_index)
            .map_err(|_| self.fail("standalone inline execution requires an admitted source region"))?
            .clone();
        if argument_count != function.parameter_sizes.len() {
            return Err(self.fail("invalid standalone inline argument count"));
        }
        let global_result = standalone.scope == QcInlineScope::Global;
        let previous = if global_result {
            Some(
                self.globals
                    .int(standalone.saved)
                    .map_err(|error| self.fail(error.to_string()))?,
            )
        } else {
            None
        };
        let depth = self.frames.len();
        let previous_arguments = self.argument_count;
        self.argument_count = argument_count;
        let outcome = (|| -> FlowResult<f32> {
            self.enter(&function)?;
            self.statement = region.entry - 1;
            let mut budget = self.statement_limit;
            let stop = InlineStop {
                entry: admitted.entry,
                exit: admitted.exit,
                depth: self.frames.len(),
            };
            self.run_statements(depth, &mut budget, Some(&stop), None)?;
            self.globals
                .float(standalone.saved)
                .map_err(|error| Flow::Guest(self.fail(error.to_string())))
        })();
        while self.frames.len() > depth {
            if self.leave().is_err() {
                break;
            }
        }
        if let Some(previous) = previous {
            let _ = self.globals.set_int(standalone.saved, previous);
        }
        self.argument_count = previous_arguments;
        outcome.map_err(|flow| match flow {
            Flow::Guest(error) => error,
            Flow::Cancelled => self.fail("stray function cancellation"),
        })
    }

    fn capture_call_staging(&self) -> Option<CallStaging> {
        if self.observe_call.is_none() && self.boundary_functions.is_empty() {
            return None;
        }
        let end = 112.min(self.globals.bytes().len());
        Some(CallStaging {
            words: self.globals.bytes()[4.min(end)..end].to_vec(),
            argument_count: self.argument_count,
        })
    }

    fn restore_call_staging(&mut self, staging: &Option<CallStaging>) {
        if let Some(staging) = staging {
            let end = 4 + staging.words.len();
            if end <= self.globals.bytes().len() {
                self.globals.bytes_mut()[4..end].copy_from_slice(&staging.words);
            }
            self.argument_count = staging.argument_count;
        }
    }

    fn write_return_words(&mut self, words: &[i32; 3]) -> FlowResult<()> {
        for (index, word) in words.iter().enumerate() {
            self.globals
                .set_int(1 + index, *word)
                .map_err(|error| Flow::Guest(self.fail(error.to_string())))?;
        }
        Ok(())
    }

    fn invoke_function(
        &mut self,
        function: &QcFunction,
        call: &QcCallSite,
        budget: &mut usize,
        staging: &Option<CallStaging>,
        inherited_run: Option<u64>,
    ) -> FlowResult<()> {
        let boundary = match &self.function_boundary {
            Some(boundary) if self.boundary_functions.contains(&function.index) => boundary.clone(),
            _ => return self.run_function(function, budget, inherited_run),
        };
        self.boundary_seqs += 1;
        let run = self.boundary_seqs;
        let action = boundary
            .run(self, call)
            .map_err(|error| Flow::Guest(self.fail(error.to_string())))?;
        self.boundary_depth += 1;
        let entry_depth = self.frames.len();
        let outcome = match action {
            QcBoundaryAction::Skip { return_words } => {
                self.restore_call_staging(staging);
                self.write_return_words(&return_words)?;
                Ok(())
            }
            QcBoundaryAction::Enter => {
                self.restore_call_staging(staging);
                self.current_runs.push(run);
                let outcome = self.run_function(function, budget, Some(run));
                self.current_runs.pop();
                match outcome {
                    Err(Flow::Cancelled) if self.pending_cancel.map(|(seq, _)| seq) == Some(run) => {
                        let (_, words) = self.pending_cancel.take().unwrap_or((run, [0; 3]));
                        self.unwind_to(entry_depth)?;
                        self.write_return_words(&words)?;
                        Ok(())
                    }
                    outcome => outcome,
                }
            }
        };
        // Host confirmation may reenter QC after the callee returned.
        // Preserve its actual return, not guest state.
        let completed = outcome.is_ok().then(|| {
            let end = 16.min(self.globals.bytes().len());
            CallStaging {
                words: self.globals.bytes()[4.min(end)..end].to_vec(),
                argument_count: self.argument_count,
            }
        });
        self.boundary_depth -= 1;
        if outcome.is_ok() {
            self.restore_call_staging(&completed);
        }
        outcome
    }

    fn run_function(&mut self, function: &QcFunction, budget: &mut usize, run: Option<u64>) -> FlowResult<()> {
        let exit_depth = self.frames.len();
        if let Some(builtin) = self.builtin(function)? {
            self.call_builtin(builtin)?;
            self.check_cancel(run)?;
            return Ok(());
        }
        self.enter(function)?;
        match self.run_statements(exit_depth, budget, None, run) {
            Ok(()) => {
                self.check_cancel(run)?;
                Ok(())
            }
            Err(error) => {
                while self.frames.len() > exit_depth {
                    if self.leave().is_err() {
                        break;
                    }
                }
                Err(error)
            }
        }
    }

    fn check_cancel(&self, run: Option<u64>) -> FlowResult<()> {
        if let Some((seq, _)) = self.pending_cancel {
            if Some(seq) == run {
                return Err(Flow::Cancelled);
            }
        }
        Ok(())
    }

    #[allow(clippy::too_many_lines)]
    fn run_statements(
        &mut self,
        exit_depth: usize,
        budget: &mut usize,
        stop: Option<&InlineStop>,
        run: Option<u64>,
    ) -> FlowResult<()> {
        let inline_boundary = if self.inline_regions.is_empty() {
            None
        } else {
            self.inline_boundary.clone()
        };
        while self.frames.len() > exit_depth {
            if let Some(stop) = stop {
                if self.frames.len() == stop.depth {
                    if self.statement + 1 == stop.exit {
                        return Ok(());
                    }
                    if self.statement + 1 < stop.entry || self.statement + 1 > stop.exit {
                        return Err(Flow::Guest(self.fail("inline source region escaped its continuation")));
                    }
                }
            }
            self.check_cancel(run)?;
            self.statement += 1;
            let statement = *self
                .program
                .statements
                .get(self.statement)
                .ok_or_else(|| Flow::Guest(self.fail("statement outside program")))?;
            if let Some(boundary) = &inline_boundary {
                if let Some(region) = self.inline_regions.get(&self.statement).copied() {
                    let own_stop =
                        stop.is_some_and(|stop| self.frames.len() == stop.depth && stop.entry == self.statement);
                    if region.function_index == self.function_index && !own_stop {
                        if self.frames.is_empty() {
                            return Err(Flow::Guest(self.fail("inline source region has no frame")));
                        }
                        let depth = self.frames.len();
                        self.statement -= 1;
                        let action = boundary
                            .run(self, &region)
                            .map_err(|error| Flow::Guest(self.fail(error.to_string())))?;
                        match action {
                            QcInlineAction::SkipToJoin => {
                                if !region.replaceable {
                                    return Err(Flow::Guest(
                                        self.fail("inline source region is not admitted for replacement"),
                                    ));
                                }
                                self.statement = region.exit - 1;
                            }
                            QcInlineAction::Run => {
                                let nested = InlineStop {
                                    entry: region.entry,
                                    exit: region.exit,
                                    depth,
                                };
                                self.run_statements(exit_depth, budget, Some(&nested), run)?;
                            }
                        }
                        continue;
                    }
                }
            }
            *budget = budget.saturating_sub(1);
            if *budget == 0 {
                return Err(Flow::Guest(self.fail("runaway loop error")));
            }
            if let Some(counter) = self.profiling.get_mut(self.function_index) {
                *counter = counter.wrapping_add(1);
            }
            if self.trace_enabled {
                if let Some(trace) = self.trace.clone() {
                    trace(self);
                }
            }
            let (opcode, a, b, c) = (
                statement.opcode,
                usize::from(statement.a),
                usize::from(statement.b),
                usize::from(statement.c),
            );
            let numeric = self.numeric;
            match opcode {
                QcOpcode::AddF => self.alu2(a, b, c, |n, x, y| n.add(x, y))?,
                QcOpcode::SubF => self.alu2(a, b, c, |n, x, y| n.sub(x, y))?,
                QcOpcode::MulF => self.alu2(a, b, c, |n, x, y| n.mul(x, y))?,
                QcOpcode::DivF => self.alu2(a, b, c, |n, x, y| n.div(x, y))?,
                QcOpcode::AddV => self.vector2(a, b, c, |n, x, y| n.add(x, y))?,
                QcOpcode::SubV => self.vector2(a, b, c, |n, x, y| n.sub(x, y))?,
                QcOpcode::MulV => {
                    let (ax, ay, az) = self.global_triple(a)?;
                    let (bx, by, bz) = self.global_triple(b)?;
                    let dot = numeric.add(
                        numeric.add(
                            numeric.mul(f64::from(ax), f64::from(bx)),
                            numeric.mul(f64::from(ay), f64::from(by)),
                        ),
                        numeric.mul(f64::from(az), f64::from(bz)),
                    );
                    self.set_global_float(c, dot as f32)?;
                }
                QcOpcode::MulFV => {
                    let scalar = f64::from(self.global_float(a)?);
                    for index in 0..3 {
                        let value = numeric.mul(scalar, f64::from(self.global_float(b + index)?));
                        self.set_global_float(c + index, value as f32)?;
                    }
                }
                QcOpcode::MulVF => {
                    let scalar = f64::from(self.global_float(b)?);
                    for index in 0..3 {
                        let value = numeric.mul(scalar, f64::from(self.global_float(a + index)?));
                        self.set_global_float(c + index, value as f32)?;
                    }
                }
                QcOpcode::EqF => self.compare2(a, b, c, |x, y| x == y)?,
                QcOpcode::NeF => self.compare2(a, b, c, |x, y| x != y)?,
                QcOpcode::EqV => {
                    let equal = self.global_triple(a)? == self.global_triple(b)?;
                    self.set_global_float(c, f32::from(equal))?;
                }
                QcOpcode::NeV => {
                    let equal = self.global_triple(a)? == self.global_triple(b)?;
                    self.set_global_float(c, f32::from(!equal))?;
                }
                QcOpcode::EqS => {
                    let left = self.global_string(a)?;
                    let right = self.global_string(b)?;
                    self.set_global_float(c, f32::from(left == right))?;
                }
                QcOpcode::NeS => {
                    let left = self.global_string(a)?;
                    let right = self.global_string(b)?;
                    self.set_global_float(c, byte_compare(&left, &right) as f32)?;
                }
                QcOpcode::EqE | QcOpcode::EqFn => {
                    let equal = self.global_int(a)? == self.global_int(b)?;
                    self.set_global_float(c, f32::from(equal))?;
                }
                QcOpcode::NeE | QcOpcode::NeFn => {
                    let equal = self.global_int(a)? == self.global_int(b)?;
                    self.set_global_float(c, f32::from(!equal))?;
                }
                QcOpcode::Le => self.compare2(a, b, c, |x, y| x <= y)?,
                QcOpcode::Ge => self.compare2(a, b, c, |x, y| x >= y)?,
                QcOpcode::Lt => self.compare2(a, b, c, |x, y| x < y)?,
                QcOpcode::Gt => self.compare2(a, b, c, |x, y| x > y)?,
                QcOpcode::NotF => {
                    let value = f32::from(self.global_float(a)? == 0.0);
                    self.set_global_float(c, value)?;
                }
                QcOpcode::NotV => {
                    let (x, y, z) = self.global_triple(a)?;
                    self.set_global_float(c, f32::from(x == 0.0 && y == 0.0 && z == 0.0))?;
                }
                QcOpcode::NotS => {
                    let reference = self.global_int(a)?;
                    let empty = reference == 0
                        || self
                            .strings
                            .get(reference)
                            .map(|text| text.is_empty())
                            .map_err(|error| Flow::Guest(self.fail(error.to_string())))?;
                    self.set_global_float(c, f32::from(empty))?;
                }
                QcOpcode::NotEnt => {
                    let slot = self
                        .entities
                        .slot(self.global_int(a)?)
                        .map_err(|error| Flow::Guest(self.fail(error.to_string())))?;
                    self.set_global_float(c, f32::from(slot == 0))?;
                }
                QcOpcode::NotFn => {
                    let value = f32::from(self.global_int(a)? == 0);
                    self.set_global_float(c, value)?;
                }
                QcOpcode::And => {
                    let value = f32::from(self.global_float(a)? != 0.0 && self.global_float(b)? != 0.0);
                    self.set_global_float(c, value)?;
                }
                QcOpcode::Or => {
                    let value = f32::from(self.global_float(a)? != 0.0 || self.global_float(b)? != 0.0);
                    self.set_global_float(c, value)?;
                }
                QcOpcode::BitAnd => {
                    let left = self.to_int32(self.global_float(a)?)?;
                    let right = self.to_int32(self.global_float(b)?)?;
                    self.set_global_float(c, (left & right) as f32)?;
                }
                QcOpcode::BitOr => {
                    let left = self.to_int32(self.global_float(a)?)?;
                    let right = self.to_int32(self.global_float(b)?)?;
                    self.set_global_float(c, (left | right) as f32)?;
                }
                QcOpcode::StoreF | QcOpcode::StoreS | QcOpcode::StoreEnt | QcOpcode::StoreFld | QcOpcode::StoreFn => {
                    self.globals
                        .copy_within(a, b, 1)
                        .map_err(|error| Flow::Guest(self.fail(error.to_string())))?;
                }
                QcOpcode::StoreV => {
                    self.globals
                        .copy_within(a, b, 3)
                        .map_err(|error| Flow::Guest(self.fail(error.to_string())))?;
                }
                QcOpcode::LoadF | QcOpcode::LoadS | QcOpcode::LoadEnt | QcOpcode::LoadFld | QcOpcode::LoadFn => {
                    let reference = self.global_int(a)?;
                    let field = self.global_int(b)?;
                    let word = self.checked_word(field)?;
                    self.validate_entity(reference, word, 1, QcAccessKind::Read)?;
                    let slot = self
                        .entities
                        .slot(reference)
                        .map_err(|error| Flow::Guest(self.fail(error.to_string())))?;
                    let value = self
                        .entities
                        .slot_int(slot, word)
                        .map_err(|error| Flow::Guest(self.fail(error.to_string())))?;
                    self.set_global_int(c, value)?;
                }
                QcOpcode::LoadV => {
                    let reference = self.global_int(a)?;
                    let field = self.global_int(b)?;
                    let word = self.checked_word(field)?;
                    self.validate_entity(reference, word, 3, QcAccessKind::Read)?;
                    let slot = self
                        .entities
                        .slot(reference)
                        .map_err(|error| Flow::Guest(self.fail(error.to_string())))?;
                    for index in 0..3 {
                        let value = self
                            .entities
                            .slot_int(slot, word + index)
                            .map_err(|error| Flow::Guest(self.fail(error.to_string())))?;
                        self.set_global_int(c + index, value)?;
                    }
                }
                QcOpcode::Address => {
                    let reference = self.global_int(a)?;
                    let field = self.global_int(b)?;
                    let slot = self
                        .entities
                        .slot(reference)
                        .map_err(|error| Flow::Guest(self.fail(error.to_string())))?;
                    if slot == 0 && (self.server_active)() {
                        return Err(Flow::Guest(self.fail("assignment to world entity")));
                    }
                    let pointer = self
                        .entities
                        .pointer(reference, field)
                        .map_err(|error| Flow::Guest(self.fail(error.to_string())))?;
                    self.set_global_int(c, pointer)?;
                }
                QcOpcode::StorePF
                | QcOpcode::StorePS
                | QcOpcode::StorePEnt
                | QcOpcode::StorePFld
                | QcOpcode::StorePFn
                | QcOpcode::StorePV => {
                    let words: u8 = if opcode == QcOpcode::StorePV { 3 } else { 1 };
                    let pointer = self.global_int(b)?;
                    let (slot, word) = self
                        .entities
                        .resolve_pointer(pointer, words as usize)
                        .map_err(|error| Flow::Guest(self.fail(error.to_string())))?;
                    let reference = pointer
                        .wrapping_sub(self.entities.layout().variables_offset_bytes as i32)
                        .wrapping_sub(word as i32 * 4);
                    self.validate_entity(reference, word, words, QcAccessKind::Write)?;
                    let observe = self.observe_entity_store.clone();
                    let before = if observe.is_some() {
                        let fields = self
                            .entities
                            .field_bytes(slot)
                            .map_err(|error| Flow::Guest(self.fail(error.to_string())))?;
                        Some(fields[word * 4..(word + words as usize) * 4].to_vec())
                    } else {
                        None
                    };
                    for index in 0..words as usize {
                        let value = self.global_int(a + index)?;
                        self.entities
                            .set_slot_int(slot, word + index, value)
                            .map_err(|error| Flow::Guest(self.fail(error.to_string())))?;
                    }
                    if let (Some(observe), Some(before)) = (observe, before) {
                        let fields = self
                            .entities
                            .field_bytes(slot)
                            .map_err(|error| Flow::Guest(self.fail(error.to_string())))?;
                        observe(&QcEntityStoreObservation {
                            function_index: self.function_index,
                            statement: self.statement,
                            reference,
                            word,
                            before,
                            after: fields[word * 4..(word + words as usize) * 4].to_vec(),
                        });
                    }
                }
                QcOpcode::If => {
                    if self.global_int(a)? != 0 {
                        self.jump(statement.b)?;
                    }
                }
                QcOpcode::IfNot => {
                    if self.global_int(a)? == 0 {
                        self.jump(statement.b)?;
                    }
                }
                QcOpcode::Goto => self.jump(statement.a)?,
                QcOpcode::Call0
                | QcOpcode::Call1
                | QcOpcode::Call2
                | QcOpcode::Call3
                | QcOpcode::Call4
                | QcOpcode::Call5
                | QcOpcode::Call6
                | QcOpcode::Call7
                | QcOpcode::Call8 => {
                    let Some(argc) = opcode.call_arguments() else {
                        return Err(Flow::Guest(self.fail("unsupported opcode")));
                    };
                    self.argument_count = argc as usize;
                    let target = self.global_int(a)?;
                    let called = self
                        .program
                        .function_at(target as usize)
                        .map_err(|_| Flow::Guest(self.fail(format!("invalid function {target}"))))?
                        .clone();
                    let call = QcCallSite {
                        function_index: called.index,
                        caller: self.function_index,
                        statement: self.statement,
                    };
                    let staging = self.capture_call_staging();
                    if let Some(observe) = self.observe_call.clone() {
                        observe(&call);
                    }
                    self.restore_call_staging(&staging);
                    if self.boundary_functions.contains(&called.index) {
                        self.invoke_function(&called, &call, budget, &staging, run)?;
                    } else if let Some(builtin) = self.builtin(&called)? {
                        self.call_builtin(builtin)?;
                    } else {
                        self.enter(&called)?;
                    }
                }
                QcOpcode::State => self.op_state(a, b)?,
                QcOpcode::Done | QcOpcode::Return => {
                    if stop.is_some_and(|stop| self.frames.len() == stop.depth) {
                        return Err(Flow::Guest(
                            self.fail("inline source region returned before its continuation"),
                        ));
                    }
                    self.globals
                        .copy_within(a, 1, 3)
                        .map_err(|error| Flow::Guest(self.fail(error.to_string())))?;
                    self.leave()?;
                }
            }
        }
        Ok(())
    }

    fn jump(&mut self, operand: u16) -> FlowResult<()> {
        let next = self.statement as i64 + i64::from(signed_qc_branch(operand)) - 1;
        if next < 0 {
            return Err(Flow::Guest(self.fail("statement outside program")));
        }
        self.statement = next as usize;
        Ok(())
    }

    fn global_int(&self, word: usize) -> FlowResult<i32> {
        self.globals
            .int(word)
            .map_err(|error| Flow::Guest(self.fail(error.to_string())))
    }

    fn global_float(&self, word: usize) -> FlowResult<f32> {
        self.globals
            .float(word)
            .map_err(|error| Flow::Guest(self.fail(error.to_string())))
    }

    fn global_triple(&self, word: usize) -> FlowResult<(f32, f32, f32)> {
        Ok((
            self.global_float(word)?,
            self.global_float(word + 1)?,
            self.global_float(word + 2)?,
        ))
    }

    fn global_string(&self, word: usize) -> FlowResult<String> {
        let reference = self.global_int(word)?;
        self.strings
            .get(reference)
            .map_err(|error| Flow::Guest(self.fail(error.to_string())))
    }

    fn set_global_int(&mut self, word: usize, value: i32) -> FlowResult<()> {
        self.globals
            .set_int(word, value)
            .map_err(|error| Flow::Guest(self.fail(error.to_string())))
    }

    fn set_global_float(&mut self, word: usize, value: f32) -> FlowResult<()> {
        self.globals
            .set_float(word, value)
            .map_err(|error| Flow::Guest(self.fail(error.to_string())))
    }

    fn alu2(&mut self, a: usize, b: usize, c: usize, op: impl Fn(NumericOps, f64, f64) -> f64) -> FlowResult<()> {
        let value = op(
            self.numeric,
            f64::from(self.global_float(a)?),
            f64::from(self.global_float(b)?),
        );
        self.set_global_float(c, value as f32)
    }

    fn vector2(&mut self, a: usize, b: usize, c: usize, op: impl Fn(NumericOps, f64, f64) -> f64) -> FlowResult<()> {
        for index in 0..3 {
            let value = op(
                self.numeric,
                f64::from(self.global_float(a + index)?),
                f64::from(self.global_float(b + index)?),
            );
            self.set_global_float(c + index, value as f32)?;
        }
        Ok(())
    }

    fn compare2(&mut self, a: usize, b: usize, c: usize, op: impl Fn(f32, f32) -> bool) -> FlowResult<()> {
        let value = f32::from(op(self.global_float(a)?, self.global_float(b)?));
        self.set_global_float(c, value)
    }

    fn to_int32(&self, value: f32) -> FlowResult<i32> {
        self.numeric
            .to_int32(f64::from(value))
            .map_err(|error| Flow::Guest(self.fail(error.to_string())))
    }

    fn checked_word(&self, field: i32) -> FlowResult<usize> {
        if field < 0 {
            return Err(Flow::Guest(self.fail(format!("invalid field offset {field}"))));
        }
        Ok(field as usize)
    }

    fn op_state(&mut self, a: usize, b: usize) -> FlowResult<()> {
        let self_offset = self.global_offset("self").map_err(Flow::Guest)?;
        let time_offset = self.global_offset("time").map_err(Flow::Guest)?;
        let nextthink = self.field_offset("nextthink").map_err(Flow::Guest)?;
        let frame = self.field_offset("frame").map_err(Flow::Guest)?;
        let think = self.field_offset("think").map_err(Flow::Guest)?;
        let reference = self.global_int(self_offset)?;
        let time = f64::from(self.global_float(time_offset)?);
        let frame_value = self.global_float(a)?;
        let think_value = self.global_int(b)?;
        let slot = self
            .entities
            .slot(reference)
            .map_err(|error| Flow::Guest(self.fail(error.to_string())))?;
        if self.validate_entity_access.is_some() {
            for (word, value, is_float) in [
                (nextthink, self.numeric.add(time, 0.1) as f32, true),
                (frame, frame_value, true),
                (think, f32::from_bits(think_value as u32), false),
            ] {
                self.validate_entity(reference, word, 1, QcAccessKind::Write)?;
                let observe = self.observe_entity_store.clone();
                let before = if observe.is_some() {
                    let fields = self
                        .entities
                        .field_bytes(slot)
                        .map_err(|error| Flow::Guest(self.fail(error.to_string())))?;
                    Some(fields[word * 4..(word + 1) * 4].to_vec())
                } else {
                    None
                };
                if is_float {
                    self.entities
                        .set_slot_float(slot, word, value)
                        .map_err(|error| Flow::Guest(self.fail(error.to_string())))?;
                } else {
                    self.entities
                        .set_slot_int(slot, word, value.to_bits() as i32)
                        .map_err(|error| Flow::Guest(self.fail(error.to_string())))?;
                }
                if let (Some(observe), Some(before)) = (observe, before) {
                    let fields = self
                        .entities
                        .field_bytes(slot)
                        .map_err(|error| Flow::Guest(self.fail(error.to_string())))?;
                    observe(&QcEntityStoreObservation {
                        function_index: self.function_index,
                        statement: self.statement,
                        reference,
                        word,
                        before,
                        after: fields[word * 4..(word + 1) * 4].to_vec(),
                    });
                }
            }
            return Ok(());
        }
        self.entities
            .set_slot_float(slot, nextthink, self.numeric.add(time, 0.1) as f32)
            .map_err(|error| Flow::Guest(self.fail(error.to_string())))?;
        self.entities
            .set_slot_float(slot, frame, frame_value)
            .map_err(|error| Flow::Guest(self.fail(error.to_string())))?;
        self.entities
            .set_slot_int(slot, think, think_value)
            .map_err(|error| Flow::Guest(self.fail(error.to_string())))?;
        Ok(())
    }

    /// Snapshot globals, entities, strings, and counters. Requires an idle
    /// callback boundary.
    pub fn snapshot(&self) -> Result<QcMachineSnapshot, GuestError> {
        if !self.frames.is_empty() || self.builtin_depth != 0 || self.boundary_depth != 0 {
            return Err(self.fail("save requires an idle callback boundary"));
        }
        Ok(QcMachineSnapshot {
            globals: self.globals.bytes().to_vec(),
            entities: self.entities.bytes().to_vec(),
            entity_count: self.entities.count(),
            strings: self.strings.snapshot()?,
            statement: self.statement,
            function_index: self.function_index,
            argument_count: self.argument_count,
            profiling: self.profiling.clone(),
            trace_enabled: self.trace_enabled,
        })
    }

    /// Restore an idle-boundary snapshot.
    pub fn restore(&mut self, snapshot: &QcMachineSnapshot) -> Result<(), GuestError> {
        if !self.frames.is_empty() || self.builtin_depth != 0 || self.boundary_depth != 0 {
            return Err(self.fail("restore requires an idle callback boundary"));
        }
        if snapshot.globals.len() != self.globals.bytes().len()
            || snapshot.profiling.len() != self.profiling.len()
            || snapshot.function_index != 0
        {
            return Err(self.fail("incompatible machine checkpoint"));
        }
        self.strings
            .restore(&snapshot.strings)
            .map_err(|error| self.fail(error.to_string()))?;
        self.entities
            .restore(&snapshot.entities, snapshot.entity_count)
            .map_err(|error| self.fail(error.to_string()))?;
        self.globals.bytes_mut().copy_from_slice(&snapshot.globals);
        self.statement = snapshot.statement;
        self.function_index = 0;
        self.argument_count = snapshot.argument_count;
        self.trace_enabled = snapshot.trace_enabled;
        self.profiling.copy_from_slice(&snapshot.profiling);
        Ok(())
    }
}

struct InlineStop {
    entry: usize,
    exit: usize,
    depth: usize,
}

fn byte_compare(left: &str, right: &str) -> i32 {
    let left = left.as_bytes();
    let right = right.as_bytes();
    for index in 0..=left.len().min(right.len()) {
        let difference =
            i32::from(left.get(index).copied().unwrap_or(0)) - i32::from(right.get(index).copied().unwrap_or(0));
        if difference != 0 {
            return difference;
        }
    }
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    type RawStatement = (u16, u16, u16, u16);

    fn fixture_image(statements: &[RawStatement], fn1_first: i32) -> Vec<u8> {
        // Globals: 28 reserved + self(28) + time(29) + temp(30) + params(31..=32).
        let mut image = Vec::new();
        let push_i32 = |image: &mut Vec<u8>, value: i32| image.extend_from_slice(&value.to_le_bytes());
        let strings = b"\0main\0file.qc\0self\0time\0temp\0nextthink\0frame\0think\0";
        // main=1 file.qc=6 self=14 time=19 temp=24 nextthink=29 frame=39 think=45
        let globals = [(2u16, 28u16, 14i32), (2, 29, 19), (2, 30, 24)];
        let fields = [(2u16, 0u16, 29i32), (2, 1, 39), (2, 2, 45)];
        let functions = [
            (0i32, 0i32, 0i32, 0i32, 1i32, 6i32, 0i32),
            (fn1_first, 31i32, 0i32, 0i32, 1i32, 6i32, 2i32),
        ];
        let header_len = 60i32;
        let mut blobs: Vec<Vec<u8>> = Vec::new();
        let mut statement_blob = Vec::new();
        for (op, a, b, c) in statements {
            for word in [*op, *a, *b, *c] {
                statement_blob.extend_from_slice(&word.to_le_bytes());
            }
        }
        blobs.push(statement_blob);
        let mut global_blob = Vec::new();
        for (raw, at, name) in &globals {
            global_blob.extend_from_slice(&raw.to_le_bytes());
            global_blob.extend_from_slice(&at.to_le_bytes());
            global_blob.extend_from_slice(&name.to_le_bytes());
        }
        blobs.push(global_blob);
        let mut field_blob = Vec::new();
        for (raw, at, name) in &fields {
            field_blob.extend_from_slice(&raw.to_le_bytes());
            field_blob.extend_from_slice(&at.to_le_bytes());
            field_blob.extend_from_slice(&name.to_le_bytes());
        }
        blobs.push(field_blob);
        let mut function_blob = Vec::new();
        for (first, params, locals, profile, name, file, count) in &functions {
            for word in [*first, *params, *locals, *profile, *name, *file, *count] {
                function_blob.extend_from_slice(&word.to_le_bytes());
            }
            let mut sizes = [0u8; 8];
            if *count == 2 {
                sizes[0] = 1;
                sizes[1] = 1;
            }
            function_blob.extend_from_slice(&sizes);
        }
        blobs.push(function_blob);
        blobs.push(strings.to_vec());
        blobs.push(vec![0u8; 33 * 4]);
        let counts = [statements.len() as i32, 3, 3, 2, strings.len() as i32, 33];
        let mut offset = header_len;
        let mut sections = Vec::new();
        for (blob, count) in blobs.iter().zip(counts) {
            sections.push((offset, count));
            offset += blob.len() as i32;
        }
        push_i32(&mut image, 6);
        push_i32(&mut image, 5927);
        for (at, count) in sections {
            push_i32(&mut image, at);
            push_i32(&mut image, count);
        }
        push_i32(&mut image, 3);
        for blob in &blobs {
            image.extend_from_slice(blob);
        }
        image
    }

    fn default_statements() -> Vec<RawStatement> {
        vec![
            (QcOpcode::Done as u16, 0, 0, 0),
            (QcOpcode::AddF as u16, 4, 7, 30),
            (QcOpcode::Return as u16, 30, 0, 0),
        ]
    }

    fn fixture_program() -> QcProgram {
        let image = fixture_image(&default_statements(), 1);
        super::super::program::load_qc_program(&image, None, "test.dat").unwrap()
    }

    fn fixture_program_with(statements: &[RawStatement], fn1_first: i32) -> QcProgram {
        let image = fixture_image(statements, fn1_first);
        super::super::program::load_qc_program(&image, None, "test.dat").unwrap()
    }

    fn fixture_entities(program: &QcProgram, capacity: usize, count: usize) -> QcEntityMemory {
        let layout = super::super::memory::QcEntityLayout {
            stride_bytes: 96 + program.entity_field_words * 4,
            variables_offset_bytes: 96,
            field_words: program.entity_field_words,
        };
        QcEntityMemory::new(layout, capacity, count).unwrap()
    }

    fn fixture_machine() -> QcMachine {
        let program = fixture_program();
        let entities = fixture_entities(&program, 4, 2);
        let numeric = NumericOps::select(qa_core::numeric::Q1_DONOR_PROFILE).unwrap();
        QcMachine::new(QcMachineOptions::new(
            program,
            numeric,
            entities,
            QcBuiltinRegistry::default(),
            Rc::new(|| true),
        ))
        .unwrap()
    }

    #[test]
    fn executes_add_and_returns() {
        let mut machine = fixture_machine();
        machine.globals_mut().set_float(4, 2.0).unwrap();
        machine.globals_mut().set_float(7, 3.0).unwrap();
        machine.execute(1, 2).unwrap();
        assert_eq!(machine.globals().float(30).unwrap(), 5.0);
        assert_eq!(machine.globals().float(1).unwrap(), 5.0);
        assert_eq!(machine.depth(), 0);
        assert!(machine.profiling()[1] >= 2);
    }

    #[test]
    fn rejects_layout_mismatch() {
        let program = fixture_program();
        let bad_layout = super::super::memory::QcEntityLayout {
            stride_bytes: 100,
            variables_offset_bytes: 96,
            field_words: 1,
        };
        let entities = QcEntityMemory::new(bad_layout, 2, 1).unwrap();
        let numeric = NumericOps::select(qa_core::numeric::Q1_DONOR_PROFILE).unwrap();
        let options = QcMachineOptions::new(
            program,
            numeric,
            entities,
            QcBuiltinRegistry::default(),
            Rc::new(|| true),
        );
        assert!(QcMachine::new(options).is_err());
    }

    #[test]
    fn runaway_loop_is_bounded() {
        let statements = vec![(QcOpcode::Done as u16, 0, 0, 0), (QcOpcode::Goto as u16, 0, 0, 0)];
        let program = fixture_program_with(&statements, 1);
        let entities = fixture_entities(&program, 2, 1);
        let numeric = NumericOps::select(qa_core::numeric::Q1_DONOR_PROFILE).unwrap();
        let mut options = QcMachineOptions::new(
            program,
            numeric,
            entities,
            QcBuiltinRegistry::default(),
            Rc::new(|| true),
        );
        options.statement_limit = 50;
        let mut machine = QcMachine::new(options).unwrap();
        let error = machine.execute(1, 0).unwrap_err();
        assert!(error.to_string().contains("runaway loop error"));
    }

    #[test]
    fn deep_recursion_overflows_the_frame_stack() {
        let statements = vec![(QcOpcode::Done as u16, 0, 0, 0), (QcOpcode::Call0 as u16, 30, 0, 0)];
        let program = fixture_program_with(&statements, 1);
        let entities = fixture_entities(&program, 2, 1);
        let numeric = NumericOps::select(qa_core::numeric::Q1_DONOR_PROFILE).unwrap();
        let mut options = QcMachineOptions::new(
            program,
            numeric,
            entities,
            QcBuiltinRegistry::default(),
            Rc::new(|| true),
        );
        options.stack_limit = 4;
        let mut machine = QcMachine::new(options).unwrap();
        machine.globals_mut().set_int(30, 1).unwrap();
        let error = machine.execute(1, 0).unwrap_err();
        assert!(error.to_string().contains("stack overflow"));
        assert_eq!(machine.depth(), 0);
    }

    #[test]
    fn entity_helpers_observe_stores() {
        use std::cell::RefCell;
        let seen = Rc::new(RefCell::new(Vec::new()));
        let seen_clone = seen.clone();
        let program = fixture_program();
        let entities = fixture_entities(&program, 4, 2);
        let numeric = NumericOps::select(qa_core::numeric::Q1_DONOR_PROFILE).unwrap();
        let mut options = QcMachineOptions::new(
            program,
            numeric,
            entities,
            QcBuiltinRegistry::default(),
            Rc::new(|| true),
        );
        options.observe_entity_store = Some(Rc::new(move |store: &QcEntityStoreObservation| {
            seen_clone.borrow_mut().push(store.word);
        }));
        let mut machine = QcMachine::new(options).unwrap();
        let reference = machine.entities().reference(1).unwrap();
        machine.set_entity_float(reference, "frame", 7.0).unwrap();
        assert_eq!(machine.entity_float(reference, "frame").unwrap(), 7.0);
        assert_eq!(*seen.borrow(), vec![1]);
        assert!(machine.entity_float(reference, "missing").is_err());
    }

    #[test]
    fn snapshot_round_trip() {
        let mut machine = fixture_machine();
        machine.globals_mut().set_float(4, 2.0).unwrap();
        machine.globals_mut().set_float(7, 3.0).unwrap();
        machine.execute(1, 2).unwrap();
        let snapshot = machine.snapshot().unwrap();
        machine.globals_mut().set_float(30, 99.0).unwrap();
        machine.restore(&snapshot).unwrap();
        assert_eq!(machine.globals().float(30).unwrap(), 5.0);
        let mut bad = snapshot.clone();
        bad.function_index = 1;
        assert!(machine.restore(&bad).is_err());
    }

    #[test]
    fn function_boundary_can_skip() {
        struct Skipper;
        impl QcFunctionBoundary for Skipper {
            fn contains(&self, function_index: usize) -> bool {
                function_index == 1
            }
            fn run(&self, _machine: &mut QcMachine, _call: &QcCallSite) -> Result<QcBoundaryAction, GuestError> {
                Ok(QcBoundaryAction::Skip {
                    return_words: [11, 0, 0],
                })
            }
        }
        let program = fixture_program();
        let entities = fixture_entities(&program, 2, 1);
        let numeric = NumericOps::select(qa_core::numeric::Q1_DONOR_PROFILE).unwrap();
        let mut options = QcMachineOptions::new(
            program,
            numeric,
            entities,
            QcBuiltinRegistry::default(),
            Rc::new(|| true),
        );
        options.function_boundary = Some(Rc::new(Skipper));
        let mut machine = QcMachine::new(options).unwrap();
        machine.execute(1, 0).unwrap();
        assert_eq!(machine.globals().int(1).unwrap(), 11);
    }

    #[test]
    fn missing_builtins_are_listed() {
        let program = fixture_program_with(&default_statements(), -5);
        assert_eq!(program.functions[1].first_statement, -5);
        let entities = fixture_entities(&program, 2, 1);
        let machine = QcMachine::new(QcMachineOptions::new(
            program,
            NumericOps::select(qa_core::numeric::Q1_DONOR_PROFILE).unwrap(),
            entities,
            QcBuiltinRegistry::default(),
            Rc::new(|| true),
        ))
        .unwrap();
        let missing = machine.missing_builtins();
        assert_eq!(missing.len(), 1);
        assert_eq!(missing[0].number, Some(5));
    }

    #[test]
    fn byte_compare_matches_donor() {
        assert_eq!(byte_compare("a", "b"), -1);
        assert_eq!(byte_compare("b", "a"), 1);
        assert_eq!(byte_compare("a", "a"), 0);
        assert_eq!(byte_compare("a", "aa"), -97);
    }

    #[test]
    fn arguments_and_returns_round_trip() {
        let mut machine = fixture_machine();
        machine.globals_mut().set_float(4, 1.25).unwrap();
        machine.globals_mut().set_int(7, 9).unwrap();
        assert_eq!(machine.arg_float(0).unwrap(), 1.25);
        assert_eq!(machine.arg_int(1).unwrap(), 9);
        assert!(machine.arg_float(8).is_err());
        machine.return_int(3).unwrap();
        assert_eq!(machine.globals().int(1).unwrap(), 3);
    }
}
