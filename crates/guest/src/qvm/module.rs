//! QVM module instance: interpreter ownership, argv capture, checkpoints.
//!
//! Port of `src/compat/qvm/module.ts` (`qvmNumericProfile`, `qvmApi`,
//! `QvmModule`, `QvmModuleOptions`). Owns one interpreter and its syscall
//! dispatcher, captures command arguments for `G_ARGC`/`G_ARGV`, lowers guest
//! executor calls to words, and checkpoints the allocation image.
//!
//! Sync-port notes:
//!
//! - The donor is sync-or-async; here everything is synchronous. There is no
//!   `call_async`/`command_async`, and `create` collapses into `new` (UI
//!   validation runs inline in the constructor).
//! - The donor routes nested `call`/`command` through a `currentEntry` handle;
//!   here nested calls from host code use the live `QvmSyscall` /
//!   `QvmFunctionCall` handles directly, and `call`/`command` while active
//!   fail (the interpreter reports it). `invoke_source_callback` reenters
//!   through the passed function-call handle. Hooks are not wrapped with the
//!   module liveness check (the interpreter guards invocation liveness), and
//!   `cancel_function` always reports the donor's missing-entry error.
//! - `evaluate_counter` takes the nested call arguments directly (see the
//!   interpreter docs).
//! - Checkpoint and API types reuse `crate::checkpoint` (`GuestCheckpoint::Qvm`,
//!   `GameApi`, `GuestPrivateState`, callback bindings) and
//!   `qa_world::save::shared::SaveRandomState`; `QvmModuleProfile` is local
//!   (no shared execution-profile equivalent exists).

use std::cell::RefCell;
use std::rc::Rc;

use qa_core::numeric::{NumericProfile, Q3_BINARY32_PROFILE};
use qa_world::save::shared::SaveRandomState;

use crate::checkpoint::{
    GameApi, GuestCallbackBinding as CheckpointCallbackBinding, GuestCheckpoint, GuestPrivateState,
    ModuleIdentity as CheckpointModuleIdentity,
};
use crate::core::contracts::{
    GuestAddress, GuestCallContext, GuestCallResult, GuestCallValue, GuestCallbackReference, ModuleIdentity,
};
use crate::error::GuestError;

use super::abi::QvmUiExport;
use super::allocation::QvmAllocationProfile;
use super::artifacts::ResolvedQvmArtifact;
use super::guest_memory::QvmGuestMemory;
use super::image::parse_qvm_restart;
use super::interpreter::{
    QvmArguments, QvmCancellationScope, QvmFunctionCall, QvmFunctionHook, QvmFunctionObserver, QvmFunctionResolver,
    QvmHookToken, QvmInterpreter, QvmObserverToken, QvmSemantics,
};
use super::memory::QvmMemory;
use super::regions::QvmRegionEvaluation;
use super::registry::VmRegistration;
use super::syscalls::{create_qvm_system_call, QvmAbiProfile, QvmHost, QvmRole, QvmSystemCall};

/// Pinned QVM magic.
pub const QVM_MAGIC_NUMBER: u32 = 0x1272_1444;

/// Numeric profile for QVM float conversions.
#[must_use]
pub fn qvm_numeric_profile() -> NumericProfile {
    Q3_BINARY32_PROFILE
}

/// API identity for `role` under `profile`, matching the donor versions.
#[must_use]
pub fn qvm_api(role: QvmRole, profile: QvmAbiProfile) -> GameApi {
    match (role, profile) {
        (QvmRole::Qagame, QvmAbiProfile::Modern) => GameApi::Q3Qagame { version: 8 },
        (QvmRole::Qagame, QvmAbiProfile::Legacy116n) => GameApi::Q3Qagame { version: 7 },
        (QvmRole::Cgame, QvmAbiProfile::Modern) => GameApi::Q3Cgame { version: 4 },
        (QvmRole::Cgame, QvmAbiProfile::Legacy116n) => GameApi::Q3Cgame { version: 3 },
        (QvmRole::Ui, QvmAbiProfile::Modern) => GameApi::Q3Ui { version: 6 },
        (QvmRole::Ui, QvmAbiProfile::Legacy116n) => GameApi::Q3Ui { version: 4 },
    }
}

/// Pad `words` to a ten-word guest entry block, rejecting longer slices.
pub fn qvm_arguments(words: &[i32]) -> Result<QvmArguments, GuestError> {
    if words.len() > 10 {
        return Err(GuestError::invalid("QVM entry accepts at most ten argument words"));
    }
    let mut padded = [0i32; 10];
    padded[..words.len()].copy_from_slice(words);
    Ok(padded)
}

/// Execution profile for a QVM module.
#[derive(Debug, Clone)]
pub struct QvmModuleProfile {
    /// Module identity.
    pub module: ModuleIdentity,
    /// API identity.
    pub api: GameApi,
    /// Pinned QVM magic.
    pub magic: u32,
    /// Numeric profile.
    pub numeric: NumericProfile,
}

/// Host-owned checkpoint contributions.
#[derive(Debug)]
pub struct QvmHostCheckpoint {
    /// Retained host state.
    pub state: GuestPrivateState,
    /// Random states.
    pub random: Vec<SaveRandomState>,
    /// Callback bindings.
    pub callbacks: Vec<CheckpointCallbackBinding>,
}

/// Host-owned durable state for checkpoints.
pub trait QvmHostState {
    /// Capture host state.
    fn checkpoint(&mut self) -> Result<QvmHostCheckpoint, GuestError>;
    /// Restore host state.
    fn restore(&mut self, checkpoint: QvmHostCheckpoint) -> Result<(), GuestError>;
}

/// Options for [`QvmModule::new`].
pub struct QvmModuleOptions {
    /// Resolved bytecode artifact.
    pub artifact: ResolvedQvmArtifact,
    /// Engine host.
    pub host: Box<dyn QvmHost>,
    /// Storage accounting.
    pub allocation: QvmAllocationProfile,
    /// Registry slot, if any.
    pub registration: Option<VmRegistration>,
    /// Initial command arguments.
    pub command_arguments: Option<Vec<String>>,
    /// Host-owned durable state, if any.
    pub host_state: Option<Box<dyn QvmHostState>>,
}

impl std::fmt::Debug for QvmModuleOptions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("QvmModuleOptions")
            .field("artifact", &self.artifact)
            .field("registration", &self.registration)
            .field("command_arguments", &self.command_arguments)
            .finish_non_exhaustive()
    }
}

fn checkpoint_module(module: &ModuleIdentity) -> CheckpointModuleIdentity {
    CheckpointModuleIdentity {
        id: format!("{}:{}", module.id.namespace, module.id.name),
        artifact_path: module.artifact_path.clone(),
        digest: format!("{}:{}", module.digest.algorithm, module.digest.value),
        revision: module.revision.clone(),
    }
}

/// Live QVM module instance.
pub struct QvmModule {
    module: ModuleIdentity,
    role: QvmRole,
    api: GameApi,
    abi_profile: QvmAbiProfile,
    interpreter: QvmInterpreter,
    guest_memory: QvmGuestMemory,
    system: QvmSystemCall<Box<dyn QvmHost>>,
    commands: Rc<RefCell<Option<Vec<String>>>>,
    registration: Option<VmRegistration>,
    host_state: Option<Box<dyn QvmHostState>>,
    retired: bool,
}

impl std::fmt::Debug for QvmModule {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("QvmModule")
            .field("module", &self.module)
            .field("role", &self.role)
            .field("retired", &self.retired)
            .finish_non_exhaustive()
    }
}

impl QvmModule {
    /// Create a module over a resolved bytecode artifact (replacements are
    /// native modules, handled by the caller). Validates the UI API version.
    pub fn new(options: QvmModuleOptions) -> Result<Self, GuestError> {
        let ResolvedQvmArtifact::Bytecode {
            module,
            role,
            image,
            abi_profile,
            ..
        } = options.artifact
        else {
            return Err(GuestError::invalid("QVM replacement artifacts are native modules"));
        };
        let interpreter = QvmInterpreter::new(
            &image,
            options.allocation,
            options.registration.clone(),
            QvmSemantics::Interpreted,
        )?;
        let guest_memory = QvmGuestMemory::new(module.clone(), interpreter.memory());
        let commands = Rc::new(RefCell::new(options.command_arguments));
        let captured = Rc::clone(&commands);
        let system = create_qvm_system_call(
            role,
            options.host,
            Box::new(move || (*captured.borrow()).clone()),
            abi_profile,
        );
        let api = qvm_api(role, abi_profile);
        let mut module_instance = Self {
            module,
            role,
            api,
            abi_profile,
            interpreter,
            guest_memory,
            system,
            commands,
            registration: options.registration,
            host_state: options.host_state,
            retired: false,
        };
        if module_instance.role == QvmRole::Ui {
            if let Err(error) = module_instance.validate_ui_version() {
                module_instance.retire();
                return Err(error);
            }
        }
        Ok(module_instance)
    }

    /// Module identity.
    #[must_use]
    pub fn module(&self) -> &ModuleIdentity {
        &self.module
    }

    /// Module role.
    #[must_use]
    pub fn role(&self) -> QvmRole {
        self.role
    }

    /// ABI profile.
    #[must_use]
    pub fn abi_profile(&self) -> QvmAbiProfile {
        self.abi_profile
    }

    /// Execution profile.
    #[must_use]
    pub fn profile(&self) -> QvmModuleProfile {
        QvmModuleProfile {
            module: self.module.clone(),
            api: self.api.clone(),
            magic: QVM_MAGIC_NUMBER,
            numeric: qvm_numeric_profile(),
        }
    }

    /// Owned interpreter.
    #[must_use]
    pub fn interpreter(&self) -> &QvmInterpreter {
        &self.interpreter
    }

    /// Interpreter address space.
    #[must_use]
    pub fn memory(&self) -> QvmMemory {
        self.interpreter.memory()
    }

    /// Executor-facing guest memory.
    #[must_use]
    pub fn guest_memory(&self) -> &QvmGuestMemory {
        &self.guest_memory
    }

    /// Cancel the current source invocation. Without the donor's suspended
    /// entry handle this always reports the missing-entry error.
    pub fn cancel_function(&self, _scope: &QvmCancellationScope) -> Result<i32, GuestError> {
        self.live()?;
        Err(GuestError::invalid(
            "QVM cancellation requires the current source invocation",
        ))
    }

    fn live(&self) -> Result<(), GuestError> {
        self.interpreter.memory().assert_not_publishing()?;
        self.interpreter.memory().assert_live()?;
        if self.retired {
            return Err(GuestError::invalid(format!(
                "{} QVM module has been retired",
                self.role.as_str()
            )));
        }
        Ok(())
    }

    fn validate_ui_version(&mut self) -> Result<(), GuestError> {
        let mut arguments = [0i32; 10];
        arguments[0] = QvmUiExport::UiGetapiversion as i32;
        let version = self.call(&arguments, 0)?;
        if self.abi_profile != QvmAbiProfile::Modern && version != 4 {
            return Err(GuestError::invalid(format!(
                "Legacy User Interface is version {version}, expected 4"
            )));
        }
        if version != 4 && version != 6 {
            return Err(GuestError::invalid(format!(
                "User Interface is version {version}, expected 6"
            )));
        }
        self.api = GameApi::Q3Ui {
            version: i64::from(version),
        };
        Ok(())
    }

    /// Direct guest entry; fails while another root call is active.
    pub fn call(&mut self, words: &[i32], entry: usize) -> Result<i32, GuestError> {
        self.live()?;
        if let Some(registration) = self.registration.as_ref() {
            registration.called();
        }
        let padded;
        let arguments: &[i32] = if entry == 0 || words.len() <= 10 {
            padded = qvm_arguments(words)?;
            &padded
        } else {
            words
        };
        self.interpreter.invoke(&self.system, arguments, entry, None)
    }

    /// Direct guest entry with captured command arguments.
    pub fn command(&mut self, words: &[i32], argv: Vec<String>) -> Result<i32, GuestError> {
        self.live()?;
        let previous = self.commands.borrow().clone();
        *self.commands.borrow_mut() = Some(argv);
        let result = self.call(words, 0);
        *self.commands.borrow_mut() = previous;
        result
    }

    /// Lower an executor call to words and run it.
    pub fn invoke(
        &mut self,
        context: &GuestCallContext,
        args: &[GuestCallValue],
    ) -> Result<GuestCallResult, GuestError> {
        self.live()?;
        let GuestCallbackReference::Qvm {
            module,
            instruction_index,
        } = &context.callback
        else {
            return Err(GuestError::invalid("QVM module requires its QVM callback"));
        };
        if module != &self.module || context.module != self.module {
            return Err(GuestError::invalid("QVM callback belongs to another module"));
        }
        let mut words = Vec::with_capacity(args.len());
        for arg in args {
            words.push(self.lower(arg)?);
        }
        let value = self.call(&words, *instruction_index as usize)?;
        Ok(GuestCallResult::Value(GuestCallValue::Int32(value)))
    }

    fn lower(&self, value: &GuestCallValue) -> Result<i32, GuestError> {
        match value {
            GuestCallValue::Int32(word) => Ok(*word),
            GuestCallValue::Uint32(word) => Ok(*word as i32),
            GuestCallValue::Float32(word) => Ok(word.to_bits() as i32),
            GuestCallValue::Pointer(None) => Ok(0),
            GuestCallValue::Pointer(Some(address)) => self.pointer_word(*address),
            GuestCallValue::Int64(_) => Err(GuestError::invalid("QVM callback requires ABI lowering for int64")),
            GuestCallValue::Uint64(_) => Err(GuestError::invalid("QVM callback requires ABI lowering for uint64")),
            GuestCallValue::Float64(_) => Err(GuestError::invalid("QVM callback requires ABI lowering for float64")),
            GuestCallValue::Aggregate { .. } => {
                Err(GuestError::invalid("QVM callback requires ABI lowering for aggregate"))
            }
        }
    }

    fn pointer_word(&self, address: GuestAddress) -> Result<i32, GuestError> {
        self.guest_memory.borrow(address, 0)?;
        if address.offset == 0 {
            return i32::try_from(self.interpreter.memory().len())
                .map_err(|_| GuestError::invalid("QVM memory length exceeds the guest word range"));
        }
        i32::try_from(address.offset)
            .map_err(|_| GuestError::invalid("QVM pointer offset exceeds the guest word range"))
    }

    /// Run a qualified standalone region in a live intercepted call.
    pub fn evaluate_region(
        &mut self,
        call: &mut QvmFunctionCall<'_, '_, '_>,
        region: &QvmRegionEvaluation,
        inputs: &[i32],
    ) -> Result<i32, GuestError> {
        self.live()?;
        call.evaluate_region(region, inputs)
    }

    /// Run an original counter leaf with one virtual word through the module host.
    pub fn evaluate_counter(
        &mut self,
        address: usize,
        functions: &[usize],
        stack: Option<super::interpreter::QvmEvaluationStack>,
        args: &[i32],
        entry: usize,
    ) -> Result<i32, GuestError> {
        self.live()?;
        self.interpreter
            .evaluate_counter(&self.system, address, 0, functions, stack, args, entry)
    }

    /// Invoke a source callback (nonnegative) or engine service (negative)
    /// through a live intercepted call.
    pub fn invoke_source_callback(
        &mut self,
        call: &mut QvmFunctionCall<'_, '_, '_>,
        pointer: i32,
        args: &[i32],
    ) -> Result<i32, GuestError> {
        self.live()?;
        if pointer >= 0 {
            return call.invoke(args, pointer as usize, None);
        }
        let scratch_length = ((args.len() + 1) * 4).next_power_of_two();
        let scratch = QvmMemory::new(vec![0; scratch_length])?;
        let words = scratch.data_view(0, (args.len() + 1) * 4)?;
        words.set_i32(0, -1 - pointer)?;
        for (index, arg) in args.iter().enumerate() {
            words.set_i32(4 + index * 4, *arg)?;
        }
        // The donor dispatches inside `call.effect`; the effect holds no guard
        // during the callback, so running its liveness checks first matches.
        call.effect(&mut || Ok::<(), GuestError>(()))?;
        let mut syscall = call.reenter_as_syscall(words);
        self.system.dispatch(&mut syscall)
    }

    /// Bind a source `OP_CALL` target owned by this module.
    pub fn bind_function(
        &mut self,
        reference: &GuestCallbackReference,
        hook: QvmFunctionHook,
    ) -> Result<QvmHookToken, GuestError> {
        self.live()?;
        let (module, instruction) = qvm_reference(reference)?;
        self.own(module)?;
        self.interpreter.bind_function(instruction as usize, hook)
    }

    /// Bind direct host entry too, keeping the same body.
    pub fn bind_invocation(
        &mut self,
        reference: &GuestCallbackReference,
        hook: QvmFunctionHook,
    ) -> Result<QvmHookToken, GuestError> {
        self.live()?;
        let (module, instruction) = qvm_reference(reference)?;
        self.own(module)?;
        self.interpreter.bind_invocation(instruction as usize, hook)
    }

    /// Observe a function owned by this module.
    pub fn observe_function(
        &mut self,
        reference: &GuestCallbackReference,
        observe: QvmFunctionObserver,
    ) -> Result<QvmObserverToken, GuestError> {
        self.live()?;
        let (module, instruction) = qvm_reference(reference)?;
        self.own(module)?;
        self.interpreter.observe_function(instruction as usize, observe)
    }

    /// Resolve live guest callback pointers for modules that request it.
    pub fn bind_function_resolver(&mut self, resolve: QvmFunctionResolver) -> Result<(), GuestError> {
        self.live()?;
        self.interpreter.bind_function_resolver(resolve)
    }

    /// Remove a hook token.
    pub fn unbind_function(&mut self, token: QvmHookToken) {
        self.interpreter.unbind_function(token);
    }

    /// Remove an observer token.
    pub fn unobserve_function(&mut self, token: QvmObserverToken) {
        self.interpreter.unobserve_function(token);
    }

    /// `VM_Restart`: zero the original allocation and copy fresh data.
    pub fn restart(&mut self, bytes: &[u8]) -> Result<(), GuestError> {
        self.live()?;
        let image = parse_qvm_restart(bytes, &self.module.artifact_path)?;
        self.interpreter.restart(&image)
    }

    /// Capture the allocation image plus host state. No invocation may run.
    pub fn checkpoint(&mut self) -> Result<GuestCheckpoint, GuestError> {
        self.live()?;
        if self.interpreter.is_active() {
            return Err(GuestError::invalid("QVM checkpoints require a completed source call"));
        }
        let host = match self.host_state.as_mut() {
            Some(state) => state.checkpoint()?,
            None => return Err(GuestError::invalid("QVM host has not bound checkpoint services")),
        };
        if host.state.module != checkpoint_module(&self.module) {
            return Err(GuestError::invalid("QVM host checkpoint belongs to another module"));
        }
        Ok(GuestCheckpoint::Qvm {
            module: checkpoint_module(&self.module),
            random: host.random,
            callbacks: host.callbacks,
            api: self.api.clone(),
            abi_profile: self.abi_profile.as_str().to_string(),
            data: self.interpreter.memory().snapshot(),
            instruction_index: 0,
            program_stack: self.interpreter.stack_pointer() as u64,
            operand_stack: Vec::new(),
            host_state: host.state,
        })
    }

    /// Restore a checkpoint captured by [`Self::checkpoint`].
    pub fn restore(&mut self, checkpoint: &GuestCheckpoint) -> Result<(), GuestError> {
        self.live()?;
        if self.interpreter.is_active() {
            return Err(GuestError::invalid("Cannot restore an active QVM"));
        }
        let GuestCheckpoint::Qvm {
            module,
            random,
            callbacks,
            api,
            abi_profile,
            data,
            instruction_index,
            program_stack,
            operand_stack,
            host_state,
        } = checkpoint
        else {
            return Err(GuestError::invalid("QVM checkpoint artifact or API mismatch"));
        };
        let expected = checkpoint_module(&self.module);
        if abi_profile != self.abi_profile.as_str()
            || *module != expected
            || *api != self.api
            || host_state.module != expected
        {
            return Err(GuestError::invalid("QVM checkpoint artifact or API mismatch"));
        }
        if *instruction_index != 0
            || !operand_stack.is_empty()
            || *program_stack != self.interpreter.memory().len() as u64
        {
            return Err(GuestError::invalid(
                "QVM checkpoint contains a suspended execution stack",
            ));
        }
        let Some(state) = self.host_state.as_mut() else {
            return Err(GuestError::invalid("QVM host has not bound checkpoint services"));
        };
        self.interpreter.restore_data(data)?;
        state.restore(QvmHostCheckpoint {
            state: host_state.clone(),
            random: random.clone(),
            callbacks: callbacks.clone(),
        })?;
        Ok(())
    }

    /// Retire the module, closing its memory and freeing its registration.
    pub fn retire(&mut self) {
        self.interpreter.memory().close();
        self.retired = true;
        if let Some(registration) = self.registration.take() {
            registration.free();
        }
    }

    fn own(&self, module: &ModuleIdentity) -> Result<(), GuestError> {
        if module != &self.module {
            return Err(GuestError::invalid("QVM reference belongs to another module"));
        }
        Ok(())
    }
}

fn qvm_reference(reference: &GuestCallbackReference) -> Result<(&ModuleIdentity, u32), GuestError> {
    match reference {
        GuestCallbackReference::Qvm {
            module,
            instruction_index,
        } => Ok((module, *instruction_index)),
        _ => Err(GuestError::invalid("QVM module requires its QVM callback")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::contracts::ContentDigest;
    use qa_core::identity::ProviderId;

    use super::super::image::{QvmImage, QvmInstruction, QvmOpcode, QvmOperand};
    use super::super::syscalls::QvmHostCall;

    fn test_module() -> ModuleIdentity {
        ModuleIdentity::new(
            ProviderId::new("qvm", "test"),
            "vm/test.qvm",
            ContentDigest::new("sha256", "abc"),
            "1",
        )
    }

    fn image(program: Vec<(QvmOpcode, QvmOperand)>) -> QvmImage {
        let mut offset = 0usize;
        let instructions = program
            .into_iter()
            .map(|(opcode, operand)| {
                let instruction = QvmInstruction {
                    byte_offset: offset,
                    opcode,
                    operand,
                };
                offset += 1 + opcode.operand_width();
                instruction
            })
            .collect();
        QvmImage {
            source: "test".to_string(),
            instructions,
            code_offset: 0,
            code_length: offset,
            data_length: 256,
            literal_length: 0,
            bss_length: 0,
            initialized_data: vec![0; 256],
            allocated_data_length: 256,
            data_mask: 255,
        }
    }

    struct StubState {
        module: ModuleIdentity,
        restored: bool,
    }

    impl QvmHostState for StubState {
        fn checkpoint(&mut self) -> Result<QvmHostCheckpoint, GuestError> {
            Ok(QvmHostCheckpoint {
                state: GuestPrivateState {
                    module: checkpoint_module(&self.module),
                    format: "stub".to_string(),
                    bytes: vec![9],
                },
                random: Vec::new(),
                callbacks: Vec::new(),
            })
        }

        fn restore(&mut self, checkpoint: QvmHostCheckpoint) -> Result<(), GuestError> {
            assert_eq!(checkpoint.state.bytes, vec![9]);
            self.restored = true;
            Ok(())
        }
    }

    fn options(image: QvmImage, role: QvmRole) -> QvmModuleOptions {
        QvmModuleOptions {
            artifact: ResolvedQvmArtifact::Bytecode {
                module: test_module(),
                role,
                known: None,
                image,
                abi_profile: QvmAbiProfile::Modern,
            },
            host: Box::new(|_call: &mut QvmHostCall<'_, '_, '_>| -> Result<i32, GuestError> { Ok(0) }),
            allocation: QvmAllocationProfile::Unaccounted,
            registration: None,
            command_arguments: None,
            host_state: None,
        }
    }

    fn stateful(image: QvmImage, role: QvmRole) -> QvmModuleOptions {
        let mut owned = options(image, role);
        owned.host_state = Some(Box::new(StubState {
            module: test_module(),
            restored: false,
        }));
        owned
    }

    #[test]
    fn call_runs_the_entry_and_reports_its_profile() {
        use QvmOpcode as O;
        let mut module = QvmModule::new(options(
            image(vec![
                (O::OpEnter, QvmOperand::Word(8)),
                (O::OpConst, QvmOperand::Word(11)),
                (O::OpLeave, QvmOperand::Word(8)),
            ]),
            QvmRole::Qagame,
        ))
        .unwrap();
        assert_eq!(module.call(&[0; 10], 0).unwrap(), 11);
        let profile = module.profile();
        assert_eq!(profile.magic, 0x12721444);
        assert_eq!(profile.api, GameApi::Q3Qagame { version: 8 });
        assert_eq!(profile.numeric.id, "q3:binary32");
        assert_eq!(module.abi_profile(), QvmAbiProfile::Modern);
    }

    #[test]
    fn invoke_lowers_values_and_pointers() {
        use QvmOpcode as O;
        let mut module = QvmModule::new(options(
            image(vec![
                (O::OpEnter, QvmOperand::Word(16)),
                (O::OpLocal, QvmOperand::Word(24)),
                (O::OpLoad4, QvmOperand::None),
                (O::OpLeave, QvmOperand::Word(16)),
            ]),
            QvmRole::Cgame,
        ))
        .unwrap();
        // Reads caller word 0 (sp+8) back.
        let context = GuestCallContext {
            module: test_module(),
            callback: GuestCallbackReference::Qvm {
                module: test_module(),
                instruction_index: 0,
            },
            parent: None,
            itself: None,
            other: None,
        };
        let result = module.invoke(&context, &[GuestCallValue::Int32(5)]).unwrap();
        assert_eq!(result, GuestCallResult::Value(GuestCallValue::Int32(5)));
        let foreign = GuestCallContext {
            callback: GuestCallbackReference::QuakeC {
                module: test_module(),
                function_index: 0,
            },
            ..context.clone()
        };
        assert!(module.invoke(&foreign, &[]).is_err());
    }

    #[test]
    fn checkpoint_requires_bound_host_state() {
        use QvmOpcode as O;
        let mut module = QvmModule::new(options(
            image(vec![
                (O::OpEnter, QvmOperand::Word(8)),
                (O::OpConst, QvmOperand::Word(0)),
                (O::OpLeave, QvmOperand::Word(8)),
            ]),
            QvmRole::Qagame,
        ))
        .unwrap();
        assert!(module.checkpoint().is_err());
    }

    #[test]
    fn checkpoint_round_trips_data() {
        use QvmOpcode as O;
        let mut module = QvmModule::new(stateful(
            image(vec![
                (O::OpEnter, QvmOperand::Word(8)),
                (O::OpConst, QvmOperand::Word(0)),
                (O::OpLeave, QvmOperand::Word(8)),
            ]),
            QvmRole::Qagame,
        ))
        .unwrap();
        module.interpreter.memory().write_bytes(32, &[4, 5, 6, 7]).unwrap();
        let checkpoint = module.checkpoint().unwrap();
        module.interpreter.memory().write_bytes(32, &[0, 0, 0, 0]).unwrap();
        module.restore(&checkpoint).unwrap();
        assert_eq!(module.interpreter.memory().read_bytes(32, 4).unwrap(), vec![4, 5, 6, 7]);
        assert!(matches!(&checkpoint, GuestCheckpoint::Qvm { .. }));
        if let GuestCheckpoint::Qvm {
            abi_profile,
            program_stack,
            ..
        } = &checkpoint
        {
            assert_eq!(abi_profile, "q3-modern");
            assert_eq!(*program_stack, 256);
        }
    }

    #[test]
    fn ui_modules_validate_their_api_version() {
        use QvmOpcode as O;
        let good = options(
            image(vec![
                (O::OpEnter, QvmOperand::Word(8)),
                (O::OpConst, QvmOperand::Word(6)),
                (O::OpLeave, QvmOperand::Word(8)),
            ]),
            QvmRole::Ui,
        );
        assert!(QvmModule::new(good).is_ok());
        let bad = options(
            image(vec![
                (O::OpEnter, QvmOperand::Word(8)),
                (O::OpConst, QvmOperand::Word(1)),
                (O::OpLeave, QvmOperand::Word(8)),
            ]),
            QvmRole::Ui,
        );
        assert!(QvmModule::new(bad).is_err());
    }

    #[test]
    fn retired_modules_reject_everything() {
        use QvmOpcode as O;
        let mut module = QvmModule::new(options(
            image(vec![
                (O::OpEnter, QvmOperand::Word(8)),
                (O::OpConst, QvmOperand::Word(0)),
                (O::OpLeave, QvmOperand::Word(8)),
            ]),
            QvmRole::Qagame,
        ))
        .unwrap();
        module.retire();
        assert!(module.call(&[0; 10], 0).is_err());
        assert!(module.checkpoint().is_err());
        assert_eq!(
            qvm_api(QvmRole::Qagame, QvmAbiProfile::Legacy116n),
            GameApi::Q3Qagame { version: 7 }
        );
        assert_eq!(
            qvm_api(QvmRole::Cgame, QvmAbiProfile::Modern),
            GameApi::Q3Cgame { version: 4 }
        );
        assert_eq!(
            qvm_api(QvmRole::Cgame, QvmAbiProfile::Legacy116n),
            GameApi::Q3Cgame { version: 3 }
        );
        assert_eq!(
            qvm_api(QvmRole::Ui, QvmAbiProfile::Modern),
            GameApi::Q3Ui { version: 6 }
        );
        assert_eq!(
            qvm_api(QvmRole::Ui, QvmAbiProfile::Legacy116n),
            GameApi::Q3Ui { version: 4 }
        );
    }

    #[test]
    fn entry_arguments_pad_and_reject_overlong_blocks() {
        assert_eq!(qvm_arguments(&[1, 2]).unwrap(), [1, 2, 0, 0, 0, 0, 0, 0, 0, 0]);
        assert!(qvm_arguments(&[0; 11]).is_err());
    }

    #[test]
    fn invoke_rejects_wide_values_and_foreign_owners() {
        use QvmOpcode as O;
        let mut module = QvmModule::new(options(
            image(vec![
                (O::OpEnter, QvmOperand::Word(8)),
                (O::OpConst, QvmOperand::Word(0)),
                (O::OpLeave, QvmOperand::Word(8)),
            ]),
            QvmRole::Qagame,
        ))
        .unwrap();
        let context = GuestCallContext {
            module: test_module(),
            callback: GuestCallbackReference::Qvm {
                module: test_module(),
                instruction_index: 0,
            },
            parent: None,
            itself: None,
            other: None,
        };
        assert!(module.invoke(&context, &[GuestCallValue::Int64(1)]).is_err());
        assert!(module.invoke(&context, &[GuestCallValue::Float64(1.0)]).is_err());
        let foreign = GuestCallContext {
            module: ModuleIdentity::new(
                ProviderId::new("qvm", "other"),
                "vm/other.qvm",
                ContentDigest::new("sha256", "def"),
                "1",
            ),
            ..context.clone()
        };
        assert!(module.invoke(&foreign, &[]).is_err());
    }

    #[test]
    fn cancellation_without_an_entry_reports_the_donor_error() {
        use QvmOpcode as O;
        let module = QvmModule::new(options(
            image(vec![
                (O::OpEnter, QvmOperand::Word(8)),
                (O::OpConst, QvmOperand::Word(0)),
                (O::OpLeave, QvmOperand::Word(8)),
            ]),
            QvmRole::Qagame,
        ))
        .unwrap();
        let scope = QvmCancellationScope { id: 0 };
        assert!(module.cancel_function(&scope).is_err());
        assert_eq!(module.memory().len(), 256);
        assert_eq!(module.guest_memory().module(), &test_module());
    }
}
