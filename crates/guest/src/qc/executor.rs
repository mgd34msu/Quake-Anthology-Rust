//! Port of `src/compat/qc/executor.ts`: the `GuestExecutor` bridge between
//! the scheduler and one [`QcMachine`], plus idle-boundary checkpoint
//! capture/restore and the raw entity table.
//!
//! Contracts absorption: [`GuestExecutor`], [`QuakeCExecutionProfile`],
//! [`QuakeCCheckpoint`], [`GuestPrivateState`], and [`QcRawEntityTable`]
//! mirror the `quakec` essentials of `src/contracts/execution.ts`,
//! reusing the runtime [`crate::core::contracts`] types
//! (`ModuleIdentity`, `GuestCallContext`, `GuestCallValue`, layouts,
//! saved callback bindings) and [`SaveRandomState`] for RNG state. The
//! JSON save codec form (`crate::checkpoint::GuestCheckpoint`) is a
//! separate persistence spelling, not used here.

use std::rc::Rc;

use qa_core::binary::{BinaryReader, BinaryWriter};
use qa_core::identity::ActorId;
use qa_core::numeric::NumericProfile;
use qa_world::save::shared::SaveRandomState;

use super::machine::{QcMachine, QcMachineSnapshot};
use super::profile::QuakeCHostProfile;
use super::program::QuakeCApi;
use crate::core::contracts::{
    fresh_address_space, GuestAddress, GuestCallContext, GuestCallResult, GuestCallValue, GuestCallbackReference,
    GuestLayout, ModuleIdentity, RawEntityView, SavedGuestCallbackBinding,
};
use crate::error::GuestError;

/// Magic heading the host half of a QuakeC checkpoint (`QCH1`).
pub const QC_HOST_CHECKPOINT_MAGIC: u32 = 0x3148_4351;

/// Host checkpoint format label.
pub const QC_HOST_CHECKPOINT_FORMAT: &str = "quakec:host-v1";

/// Serialized source-private bytes with their owning artifact and format.
#[derive(Debug, Clone, PartialEq)]
pub struct GuestPrivateState {
    /// Owning module.
    pub module: ModuleIdentity,
    /// Format id (`namespace:name`).
    pub format: String,
    /// Payload bytes.
    pub bytes: Vec<u8>,
}

/// Host-owned checkpoint contribution.
#[derive(Debug, Clone, PartialEq)]
pub struct QcHostSavedState {
    /// Host-private state.
    pub state: GuestPrivateState,
    /// Session RNG states.
    pub random: Vec<SaveRandomState>,
    /// Saved callback bindings.
    pub callbacks: Vec<SavedGuestCallbackBinding>,
}

/// Host half of the executor: private state plus RNG/callback retention.
/// Save hooks run only after the source callback returns.
pub trait QcExecutorHost {
    /// Capture host state.
    fn checkpoint(&mut self) -> Result<QcHostSavedState, GuestError>;
    /// Restore host state.
    fn restore(&mut self, saved: QcHostSavedState) -> Result<(), GuestError>;
}

/// Execution profile binding one module to its host description and
/// arithmetic. Mirrors the `quakec` variant of `ExecutionProfile`.
#[derive(Debug, Clone, PartialEq)]
pub struct QuakeCExecutionProfile {
    /// Guest module.
    pub module: ModuleIdentity,
    /// Host description.
    pub host: QuakeCHostProfile,
    /// Arithmetic profile.
    pub numeric: NumericProfile,
}

/// One QuakeC call-stack frame in a checkpoint (always empty at the idle
/// boundary the executor requires).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QuakeCStackFrame {
    /// Statement.
    pub statement: usize,
    /// Function index.
    pub function_index: usize,
}

/// Idle-boundary QuakeC checkpoint. Mirrors `QuakeCCheckpoint`.
#[derive(Debug, Clone, PartialEq)]
pub struct QuakeCCheckpoint {
    /// Guest module.
    pub module: ModuleIdentity,
    /// Program API.
    pub api: QuakeCApi,
    /// Global words image.
    pub globals: Vec<u8>,
    /// Entity record image.
    pub entities: Vec<u8>,
    /// Entity stride in bytes.
    pub entity_stride_bytes: usize,
    /// Live entity count.
    pub entity_count: usize,
    /// String-arena image.
    pub strings: Vec<u8>,
    /// Current statement.
    pub statement: usize,
    /// Current function (0 at idle).
    pub function_index: usize,
    /// Argument count.
    pub argument_count: usize,
    /// Call stack (empty at idle).
    pub call_stack: Vec<QuakeCStackFrame>,
    /// Locals image (empty at idle).
    pub locals: Vec<u8>,
    /// Session RNG states.
    pub random: Vec<SaveRandomState>,
    /// Saved callback bindings.
    pub callbacks: Vec<SavedGuestCallbackBinding>,
    /// Host half of the checkpoint.
    pub host_state: GuestPrivateState,
}

/// Guest execution bridge: calls run to completion (nested host callbacks
/// and their mutations finish before the enclosing call returns) and
/// checkpoints capture the idle boundary.
pub trait GuestExecutor {
    /// Bound execution profile.
    fn profile(&self) -> &QuakeCExecutionProfile;
    /// Invoke one guest callback.
    fn invoke(
        &mut self,
        context: &GuestCallContext,
        arguments: &[GuestCallValue],
    ) -> Result<GuestCallResult, GuestError>;
    /// Capture an idle-boundary checkpoint.
    fn checkpoint(&mut self) -> Result<QuakeCCheckpoint, GuestError>;
    /// Restore an idle-boundary checkpoint.
    fn restore(&mut self, checkpoint: &QuakeCCheckpoint) -> Result<(), GuestError>;
}

/// `GuestExecutor` implementation over one [`QcMachine`].
pub struct QuakeCExecutor<'a> {
    profile: QuakeCExecutionProfile,
    machine: &'a mut QcMachine,
    host: &'a mut dyn QcExecutorHost,
}

impl<'a> QuakeCExecutor<'a> {
    /// Bind a profile to a machine and host, verifying they agree.
    pub fn new(
        profile: QuakeCExecutionProfile,
        machine: &'a mut QcMachine,
        host: &'a mut dyn QcExecutorHost,
    ) -> Result<Self, GuestError> {
        if profile.host.api.kind() != machine.program().api.kind()
            || profile.numeric.id != machine.numeric().profile.id
            || profile.module.digest != machine.program().digest
        {
            return Err(machine.fail("execution profile disagrees with loaded program"));
        }
        Ok(Self { profile, machine, host })
    }

    /// Borrow the bound machine.
    #[must_use]
    pub fn machine(&self) -> &QcMachine {
        self.machine
    }

    /// Mutably borrow the bound machine.
    pub fn machine_mut(&mut self) -> &mut QcMachine {
        self.machine
    }
}

impl GuestExecutor for QuakeCExecutor<'_> {
    fn profile(&self) -> &QuakeCExecutionProfile {
        &self.profile
    }

    fn invoke(
        &mut self,
        context: &GuestCallContext,
        arguments: &[GuestCallValue],
    ) -> Result<GuestCallResult, GuestError> {
        let GuestCallbackReference::QuakeC {
            module: callback_module,
            function_index,
        } = &context.callback
        else {
            return Err(self.machine.fail("callback belongs to another guest module"));
        };
        if context.module != self.profile.module || *callback_module != self.profile.module {
            return Err(self.machine.fail("callback belongs to another guest module"));
        }
        if arguments.len() > 8 {
            return Err(self.machine.fail("too many QuakeC arguments"));
        }
        if let Some(view) = &context.itself {
            set_guest_entity(self.machine, &self.profile.module, "self", view)?;
        }
        if let Some(view) = &context.other {
            set_guest_entity(self.machine, &self.profile.module, "other", view)?;
        }
        for (index, value) in arguments.iter().enumerate() {
            let offset = 4 + index * 3;
            match value {
                GuestCallValue::Int32(value) => self
                    .machine
                    .globals_mut()
                    .set_int(offset, *value)
                    .map_err(|error| self.machine.fail(error.to_string()))?,
                GuestCallValue::Uint32(value) => self
                    .machine
                    .globals_mut()
                    .set_int(offset, *value as i32)
                    .map_err(|error| self.machine.fail(error.to_string()))?,
                GuestCallValue::Float32(value) => self
                    .machine
                    .globals_mut()
                    .set_float(offset, *value)
                    .map_err(|error| self.machine.fail(error.to_string()))?,
                GuestCallValue::Aggregate { layout, bytes } => {
                    if bytes.len() != 12 || layout.byte_length != 12 {
                        return Err(self.machine.fail("QuakeC aggregate argument must be three words"));
                    }
                    let start = offset * 4;
                    if start + 12 > self.machine.globals().bytes().len() {
                        return Err(self.machine.fail("QuakeC aggregate argument must be three words"));
                    }
                    self.machine.globals_mut().bytes_mut()[start..start + 12].copy_from_slice(bytes);
                }
                GuestCallValue::Pointer(_)
                | GuestCallValue::Int64(_)
                | GuestCallValue::Uint64(_)
                | GuestCallValue::Float64(_) => {
                    return Err(self.machine.fail(format!(
                        "QuakeC does not accept {} arguments; encode a source word or vector",
                        call_value_kind(value)
                    )));
                }
            }
        }
        self.machine.execute(*function_index as usize, arguments.len())?;
        let layout = GuestLayout::new("quakec:return-words", 12, 4, 4, Vec::new());
        if self.machine.globals().bytes().len() < 16 {
            return Err(self.machine.fail("missing QuakeC return words"));
        }
        Ok(GuestCallResult::Value(GuestCallValue::Aggregate {
            layout,
            bytes: self.machine.globals().bytes()[4..16].to_vec(),
        }))
    }

    fn checkpoint(&mut self) -> Result<QuakeCCheckpoint, GuestError> {
        capture_qc_checkpoint(self.machine, &self.profile.module, self.host)
    }

    fn restore(&mut self, checkpoint: &QuakeCCheckpoint) -> Result<(), GuestError> {
        restore_qc_checkpoint(self.machine, &self.profile.module, self.host, checkpoint)
    }
}

fn set_guest_entity(
    machine: &mut QcMachine,
    module: &ModuleIdentity,
    global: &str,
    view: &RawEntityView,
) -> Result<(), GuestError> {
    if view.module != *module {
        return Err(machine.fail(format!("{global} belongs to another guest module")));
    }
    let reference = machine
        .entities()
        .reference(view.slot)
        .map_err(|error| machine.fail(error.to_string()))?;
    let offset = machine.global_offset(global)?;
    machine
        .globals_mut()
        .set_int(offset, reference)
        .map_err(|error| machine.fail(error.to_string()))
}

fn call_value_kind(value: &GuestCallValue) -> &'static str {
    match value {
        GuestCallValue::Int32(_) => "int32",
        GuestCallValue::Uint32(_) => "uint32",
        GuestCallValue::Int64(_) => "int64",
        GuestCallValue::Uint64(_) => "uint64",
        GuestCallValue::Float32(_) => "float32",
        GuestCallValue::Float64(_) => "float64",
        GuestCallValue::Pointer(_) => "pointer",
        GuestCallValue::Aggregate { .. } => "aggregate",
    }
}

/// Capture an idle-boundary checkpoint from a machine and host.
pub fn capture_qc_checkpoint(
    machine: &QcMachine,
    module: &ModuleIdentity,
    host: &mut dyn QcExecutorHost,
) -> Result<QuakeCCheckpoint, GuestError> {
    let snapshot = machine.snapshot()?;
    let saved = host.checkpoint()?;
    if saved.state.module != *module {
        return Err(machine.fail("host checkpoint belongs to another module"));
    }
    let format = saved.state.format.as_bytes();
    let mut writer = BinaryWriter::new(24 + snapshot.profiling.len() * 4 + format.len() + saved.state.bytes.len());
    let mut write = |writer: &mut BinaryWriter| -> Result<(), qa_core::binary::BinaryError> {
        writer.u32(QC_HOST_CHECKPOINT_MAGIC)?;
        writer.u32(u32::from(snapshot.trace_enabled))?;
        writer.u32(snapshot.profiling.len() as u32)?;
        for count in &snapshot.profiling {
            writer.u32(*count)?;
        }
        writer.u32(format.len() as u32)?;
        writer.bytes(format)?;
        writer.u32(saved.state.bytes.len() as u32)?;
        writer.bytes(&saved.state.bytes)?;
        writer.u32(machine.entities().layout().variables_offset_bytes as u32)?;
        Ok(())
    };
    write(&mut writer).map_err(|error| machine.fail(error.to_string()))?;
    Ok(QuakeCCheckpoint {
        module: module.clone(),
        api: machine.program().api,
        globals: snapshot.globals,
        entities: snapshot.entities,
        entity_stride_bytes: machine.entities().layout().stride_bytes,
        entity_count: snapshot.entity_count,
        strings: snapshot.strings,
        statement: snapshot.statement,
        function_index: snapshot.function_index,
        argument_count: snapshot.argument_count,
        call_stack: Vec::new(),
        locals: Vec::new(),
        random: saved.random,
        callbacks: saved.callbacks,
        host_state: GuestPrivateState {
            module: module.clone(),
            format: QC_HOST_CHECKPOINT_FORMAT.to_string(),
            bytes: writer.finish(),
        },
    })
}

/// Restore an idle-boundary checkpoint into a machine and host.
pub fn restore_qc_checkpoint(
    machine: &mut QcMachine,
    module: &ModuleIdentity,
    host: &mut dyn QcExecutorHost,
    checkpoint: &QuakeCCheckpoint,
) -> Result<(), GuestError> {
    if checkpoint.module != *module
        || checkpoint.api != machine.program().api
        || checkpoint.host_state.module != *module
    {
        return Err(machine.fail("incompatible QuakeC checkpoint"));
    }
    if !checkpoint.call_stack.is_empty() || !checkpoint.locals.is_empty() || checkpoint.function_index != 0 {
        return Err(machine.fail("checkpoint is not at an idle callback boundary"));
    }
    if checkpoint.entity_stride_bytes != machine.entities().layout().stride_bytes
        || checkpoint.host_state.format != QC_HOST_CHECKPOINT_FORMAT
    {
        return Err(machine.fail("checkpoint layout mismatch"));
    }
    let mut reader = BinaryReader::new(&checkpoint.host_state.bytes, "QuakeC host checkpoint");
    let truncated = || machine.fail("truncated QuakeC host checkpoint");
    if reader.u32().map_err(|_| truncated())? != QC_HOST_CHECKPOINT_MAGIC {
        return Err(machine.fail("unknown QuakeC host checkpoint"));
    }
    let trace_enabled = reader.u32().map_err(|_| truncated())? != 0;
    let profiling_count = reader.u32().map_err(|_| truncated())?;
    let mut profiling = Vec::with_capacity(profiling_count.min(1 << 20) as usize);
    for _ in 0..profiling_count {
        profiling.push(reader.u32().map_err(|_| truncated())?);
    }
    let format_length = reader.u32().map_err(|_| truncated())? as usize;
    let format_bytes = reader.bytes(format_length).map_err(|_| truncated())?;
    let format = String::from_utf8(format_bytes).map_err(|_| machine.fail("invalid host state format"))?;
    let state_length = reader.u32().map_err(|_| truncated())? as usize;
    let state_bytes = reader.bytes(state_length).map_err(|_| truncated())?;
    let variables_offset = reader.u32().map_err(|_| truncated())? as usize;
    if variables_offset != machine.entities().layout().variables_offset_bytes || reader.remaining() != 0 {
        return Err(machine.fail("checkpoint entity variable offset mismatch"));
    }
    let separator = format.find(':');
    match separator {
        Some(at) if at >= 1 && at + 1 < format.len() => {}
        _ => return Err(machine.fail("invalid host state format")),
    }
    machine.restore(&QcMachineSnapshot {
        globals: checkpoint.globals.clone(),
        entities: checkpoint.entities.clone(),
        entity_count: checkpoint.entity_count,
        strings: checkpoint.strings.clone(),
        statement: checkpoint.statement,
        function_index: checkpoint.function_index,
        argument_count: checkpoint.argument_count,
        profiling,
        trace_enabled,
    })?;
    host.restore(QcHostSavedState {
        random: checkpoint.random.clone(),
        callbacks: checkpoint.callbacks.clone(),
        state: GuestPrivateState {
            module: module.clone(),
            format,
            bytes: state_bytes,
        },
    })
}

/// One raw entity record: complete source-owned bytes plus the actor
/// resolved at access time (slots are reused, so authority is resolved
/// per access, never cached).
#[derive(Debug, Clone, PartialEq)]
pub struct QcRawEntityView {
    /// Owning module.
    pub module: ModuleIdentity,
    /// Entity slot.
    pub slot: u32,
    /// Record address.
    pub address: GuestAddress,
    /// Record stride in bytes.
    pub stride_bytes: usize,
    /// Public record layout.
    pub public_layout: GuestLayout,
    /// Complete source-owned record bytes at access time.
    pub bytes: Vec<u8>,
    /// Actor holding the slot at access time.
    pub actor: Option<ActorId>,
}

/// Raw entity table over one machine. Views snapshot record bytes at
/// access (re-query for fresh bytes); the donor's live `DataView` cannot
/// alias through Rust borrows.
pub struct QcRawEntityTable<'a> {
    machine: &'a QcMachine,
    module: ModuleIdentity,
    space: u64,
    layout: GuestLayout,
    current_actor: Rc<dyn Fn(u32) -> Option<ActorId>>,
}

impl QcRawEntityTable<'_> {
    /// Live record count.
    #[must_use]
    pub fn count(&self) -> usize {
        self.machine.entities().count()
    }

    /// Slot capacity.
    #[must_use]
    pub fn capacity(&self) -> usize {
        self.machine.entities().capacity()
    }

    /// Public record layout.
    #[must_use]
    pub fn layout(&self) -> &GuestLayout {
        &self.layout
    }

    /// Table base address.
    #[must_use]
    pub fn base(&self) -> GuestAddress {
        GuestAddress::new(self.space, 0)
    }

    /// View the record at `slot`.
    pub fn at_slot(&self, slot: u32) -> Result<QcRawEntityView, GuestError> {
        let machine = self.machine;
        let reference = machine
            .entities()
            .reference(slot)
            .map_err(|error| machine.fail(error.to_string()))?;
        let bytes = machine
            .entities()
            .record_bytes(slot)
            .map_err(|error| machine.fail(error.to_string()))?
            .to_vec();
        Ok(QcRawEntityView {
            module: self.module.clone(),
            slot,
            address: GuestAddress::new(self.space, reference as u64),
            stride_bytes: machine.entities().layout().stride_bytes,
            public_layout: self.layout.clone(),
            bytes,
            actor: (self.current_actor)(slot),
        })
    }

    /// View the record at a table address.
    pub fn from_pointer(&self, address: GuestAddress) -> Result<QcRawEntityView, GuestError> {
        let machine = self.machine;
        if address.space != self.space {
            return Err(machine.fail("entity address belongs to another address space"));
        }
        if address.offset > 0x7fff_ffff {
            return Err(machine.fail("entity address belongs to another address space"));
        }
        let slot = machine
            .entities()
            .slot(address.offset as i32)
            .map_err(|error| machine.fail(error.to_string()))?;
        self.at_slot(slot)
    }
}

/// Expose complete live edict records with per-access actor authority.
pub fn create_qc_raw_entity_table<'a>(
    machine: &'a QcMachine,
    module: ModuleIdentity,
    layout: GuestLayout,
    current_actor: Rc<dyn Fn(u32) -> Option<ActorId>>,
) -> Result<QcRawEntityTable<'a>, GuestError> {
    if layout.byte_length > machine.entities().layout().stride_bytes {
        return Err(machine.fail("public layout exceeds QuakeC entity record"));
    }
    Ok(QcRawEntityTable {
        machine,
        module,
        space: fresh_address_space(),
        layout,
        current_actor,
    })
}

#[cfg(test)]
mod tests {
    use super::super::machine::{QcBuiltinRegistry, QcMachineOptions};
    use super::super::memory::{QcEntityLayout, QcEntityMemory};
    use super::super::profile::describe_qc_host;
    use super::super::program::{test_program, QcDefinition, QcFunction, QcOpcode, QcStatement, QcValueType};
    use super::*;
    use crate::core::contracts::ContentDigest;
    use qa_core::identity::{IdentityOwner, ProviderId};
    use qa_core::numeric::NumericOps;

    fn fixture_machine() -> QcMachine {
        // Function 1: temp = arg0.
        let statements = vec![
            QcStatement {
                opcode: QcOpcode::Done,
                a: 0,
                b: 0,
                c: 0,
            },
            QcStatement {
                opcode: QcOpcode::StoreF,
                a: 4,
                b: 30,
                c: 0,
            },
            QcStatement {
                opcode: QcOpcode::Return,
                a: 30,
                b: 0,
                c: 0,
            },
        ];
        let globals = vec![
            QcDefinition {
                value_type: QcValueType::Entity,
                native_type: 4,
                save: false,
                offset: 28,
                name: "self".to_string(),
            },
            QcDefinition {
                value_type: QcValueType::Entity,
                native_type: 4,
                save: false,
                offset: 29,
                name: "other".to_string(),
            },
            QcDefinition {
                value_type: QcValueType::Float,
                native_type: 2,
                save: false,
                offset: 30,
                name: "temp".to_string(),
            },
        ];
        let functions = vec![
            QcFunction {
                index: 0,
                first_statement: 0,
                parameter_start: 0,
                local_words: 0,
                name: "<null>".to_string(),
                file: String::new(),
                parameter_sizes: vec![],
                named_builtin: false,
            },
            QcFunction {
                index: 1,
                first_statement: 1,
                parameter_start: 33,
                local_words: 0,
                name: "main".to_string(),
                file: "test.qc".to_string(),
                parameter_sizes: vec![1],
                named_builtin: false,
            },
        ];
        let program = test_program(
            statements,
            globals,
            vec![],
            functions,
            b"\0".to_vec(),
            vec![0; 36 * 4],
            1,
        );
        let layout = QcEntityLayout {
            stride_bytes: 100,
            variables_offset_bytes: 96,
            field_words: 1,
        };
        let entities = QcEntityMemory::new(layout, 4, 2).unwrap();
        QcMachine::new(QcMachineOptions::new(
            program,
            NumericOps::select(qa_core::numeric::Q1_DONOR_PROFILE).unwrap(),
            entities,
            QcBuiltinRegistry::default(),
            Rc::new(|| true),
        ))
        .unwrap()
    }

    fn fixture_module(machine: &QcMachine) -> ModuleIdentity {
        ModuleIdentity::new(
            ProviderId::new("test", "qc"),
            "progs.dat",
            machine.program().digest.clone(),
            "rev",
        )
    }

    fn fixture_profile(machine: &QcMachine) -> QuakeCExecutionProfile {
        let module = fixture_module(machine);
        let layout = machine.entities().layout();
        let host = describe_qc_host(
            machine.program(),
            super::super::builtins::QcHostKind::Netquake,
            &layout,
            &QcBuiltinRegistry::default(),
            &[],
        )
        .unwrap();
        QuakeCExecutionProfile {
            module,
            host,
            numeric: qa_core::numeric::Q1_DONOR_PROFILE,
        }
    }

    struct FakeHost {
        state: GuestPrivateState,
    }

    impl QcExecutorHost for FakeHost {
        fn checkpoint(&mut self) -> Result<QcHostSavedState, GuestError> {
            Ok(QcHostSavedState {
                state: self.state.clone(),
                random: Vec::new(),
                callbacks: Vec::new(),
            })
        }

        fn restore(&mut self, saved: QcHostSavedState) -> Result<(), GuestError> {
            self.state = saved.state;
            Ok(())
        }
    }

    fn return_view(module: &ModuleIdentity) -> RawEntityView {
        RawEntityView {
            module: module.clone(),
            slot: 1,
            address: GuestAddress::new(7, 100),
            stride_bytes: 100,
            public_layout: GuestLayout::new("test:layout", 4, 4, 4, Vec::new()),
            bytes: Vec::new(),
        }
    }

    #[test]
    fn invoke_marshals_self_and_arguments() {
        let mut machine = fixture_machine();
        let profile = fixture_profile(&machine);
        let module = profile.module.clone();
        let mut host = FakeHost {
            state: GuestPrivateState {
                module: module.clone(),
                format: "test:host".to_string(),
                bytes: vec![9],
            },
        };
        let mut executor = QuakeCExecutor::new(profile, &mut machine, &mut host).unwrap();
        let context = GuestCallContext {
            module: module.clone(),
            callback: GuestCallbackReference::QuakeC {
                module: module.clone(),
                function_index: 1,
            },
            parent: None,
            itself: Some(return_view(&module)),
            other: None,
        };
        let result = executor.invoke(&context, &[GuestCallValue::Float32(2.5)]).unwrap();
        let GuestCallResult::Value(GuestCallValue::Aggregate { layout, bytes }) = result else {
            panic!("expected aggregate return");
        };
        assert_eq!(layout.id, "quakec:return-words");
        assert_eq!(bytes.len(), 12);
        assert_eq!(f32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]), 2.5);
        assert_eq!(executor.machine().globals().int(28).unwrap(), 100);
    }

    #[test]
    fn invoke_rejects_foreign_callbacks_and_wide_values() {
        let mut machine = fixture_machine();
        let profile = fixture_profile(&machine);
        let module = profile.module.clone();
        let mut other_module = module.clone();
        other_module.revision = "other".to_string();
        let mut host = FakeHost {
            state: GuestPrivateState {
                module: module.clone(),
                format: "test:host".to_string(),
                bytes: Vec::new(),
            },
        };
        let mut executor = QuakeCExecutor::new(profile, &mut machine, &mut host).unwrap();
        let foreign = GuestCallContext {
            module: other_module.clone(),
            callback: GuestCallbackReference::QuakeC {
                module: module.clone(),
                function_index: 1,
            },
            parent: None,
            itself: None,
            other: None,
        };
        assert!(executor.invoke(&foreign, &[]).is_err());
        let local = GuestCallContext {
            module: module.clone(),
            callback: GuestCallbackReference::QuakeC {
                module: module.clone(),
                function_index: 1,
            },
            parent: None,
            itself: None,
            other: None,
        };
        assert!(executor.invoke(&local, &[GuestCallValue::Float64(1.0)]).is_err());
        assert!(executor.invoke(&local, &[GuestCallValue::Int32(1); 9]).is_err());
    }

    #[test]
    fn checkpoint_round_trip_preserves_magic() {
        let mut machine = fixture_machine();
        machine.globals_mut().set_float(30, 6.0).unwrap();
        let module = fixture_module(&machine);
        let mut host = FakeHost {
            state: GuestPrivateState {
                module: module.clone(),
                format: "test:host".to_string(),
                bytes: vec![1, 2, 3],
            },
        };
        let checkpoint = capture_qc_checkpoint(&machine, &module, &mut host).unwrap();
        assert_eq!(
            &checkpoint.host_state.bytes[..4],
            &QC_HOST_CHECKPOINT_MAGIC.to_le_bytes()
        );
        assert_eq!(checkpoint.host_state.format, QC_HOST_CHECKPOINT_FORMAT);
        machine.globals_mut().set_float(30, 0.0).unwrap();
        host.state.bytes = vec![9];
        restore_qc_checkpoint(&mut machine, &module, &mut host, &checkpoint).unwrap();
        assert_eq!(machine.globals().float(30).unwrap(), 6.0);
        assert_eq!(host.state.bytes, vec![1, 2, 3]);
        assert_eq!(host.state.format, "test:host");
    }

    #[test]
    fn restore_rejects_corrupt_checkpoints() {
        let mut machine = fixture_machine();
        let module = fixture_module(&machine);
        let mut host = FakeHost {
            state: GuestPrivateState {
                module: module.clone(),
                format: "test:host".to_string(),
                bytes: Vec::new(),
            },
        };
        let checkpoint = capture_qc_checkpoint(&machine, &module, &mut host).unwrap();
        let mut bad_magic = checkpoint.clone();
        bad_magic.host_state.bytes[0] ^= 0xff;
        assert!(restore_qc_checkpoint(&mut machine, &module, &mut host, &bad_magic).is_err());
        let mut bad_stride = checkpoint.clone();
        bad_stride.entity_stride_bytes += 4;
        assert!(restore_qc_checkpoint(&mut machine, &module, &mut host, &bad_stride).is_err());
        let mut busy = checkpoint.clone();
        busy.function_index = 1;
        assert!(restore_qc_checkpoint(&mut machine, &module, &mut host, &busy).is_err());
    }

    #[test]
    fn raw_entity_table_resolves_slots_and_addresses() {
        let machine = fixture_machine();
        let module = fixture_module(&machine);
        let owner = IdentityOwner::create("test").unwrap();
        let actor = owner.actor(1, 1);
        let table = create_qc_raw_entity_table(
            &machine,
            module,
            GuestLayout::new("test:layout", 8, 4, 4, Vec::new()),
            Rc::new(move |slot| (slot == 1).then(|| actor.clone())),
        )
        .unwrap();
        assert_eq!(table.count(), 2);
        let view = table.at_slot(1).unwrap();
        assert_eq!(view.bytes.len(), 100);
        assert!(view.actor.is_some());
        let again = table.from_pointer(view.address).unwrap();
        assert_eq!(again.slot, 1);
        assert!(table.from_pointer(GuestAddress::new(0xdead, 100)).is_err());
        assert!(table.at_slot(9).is_err());
    }

    #[test]
    fn constructor_rejects_disagreeing_profiles() {
        let mut machine = fixture_machine();
        let mut profile = fixture_profile(&machine);
        profile.module.digest = ContentDigest::new("sha256", &"f".repeat(64));
        let mut host = FakeHost {
            state: GuestPrivateState {
                module: profile.module.clone(),
                format: "test:host".to_string(),
                bytes: Vec::new(),
            },
        };
        assert!(QuakeCExecutor::new(profile, &mut machine, &mut host).is_err());
    }
}
