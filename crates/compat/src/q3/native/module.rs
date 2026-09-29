//! Quake Live and Q3 1.32 native game-module bridge.
//!
//! Donor: `src/compat/q3/native/module.ts` — bridges the native game-module
//! entry surface (`dllEntry` table interface and `vmMain` command interface)
//! onto a headless synthetic runner. No real native libraries are loaded.

use std::collections::HashMap;

use qa_guest::core::contracts::{
    GuestAccess, GuestAddress, GuestAllocationOptions, GuestCallContext, GuestCallResult,
    GuestCallSignature, GuestCallValue, GuestCallbackReference, GuestExportTarget, GuestImage,
    GuestPermissions, GuestStorage, GuestSymbolName, GuestValueLayout, NativeAbi, NativeCallAbi,
};
use qa_guest::core::memory::SparseGuestMemory;
use qa_guest::error::GuestError;
use thiserror::Error;

/// Quake Live game API version the table interface must report.
pub const QL_GAME_API_VERSION: i32 = 10;

/// Quake Live export-table slot count validated at bind time.
pub const QL_EXPORT_SLOT_COUNT: usize = 11;

/// Extra 32-bit arguments `vmMain` accepts after the command.
pub const Q3_VM_MAIN_ARGUMENTS: usize = 12;

/// Failure binding or calling a Q3 native game module.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum Q3ModuleError {
    /// Imports, image, or memory belong to different guest processes.
    #[error("Native game imports belong to another guest process")]
    AddressSpaceMismatch,
    /// Image uses the wrong native profile for this module type.
    #[error("{0}")]
    WrongProfile(String),
    /// Required image export is missing.
    #[error("Native module has no {0} export")]
    MissingExport(String),
    /// Quake Live game API version is not the supported one.
    #[error("Quake Live game API {found} is unsupported; expected {expected}")]
    UnsupportedApi {
        /// Reported version.
        found: i32,
        /// Supported version.
        expected: i32,
    },
    /// `dllEntry` returned a null export table.
    #[error("Quake Live dllEntry returned a null export table")]
    NullExportTable,
    /// Quake Live export slot holds a null pointer.
    #[error("Missing Quake Live export slot {0}")]
    MissingExportSlot(usize),
    /// `vmMain` was given more than twelve extra arguments.
    #[error("Q3 vmMain accepts at most twelve arguments")]
    TooManyArguments,
    /// Native entry returned a value of the wrong ABI type.
    #[error("{0}")]
    WrongResultType(String),
    /// Underlying guest memory or runner failure.
    #[error(transparent)]
    Guest(#[from] GuestError),
}

/// Call signature for one native Q3 entry: scalar parameters, optional scalar
/// result (`None` is void), and an optional C variadic tail.
#[must_use]
pub fn native_q3_signature(
    abi: NativeAbi,
    parameters: &[GuestStorage],
    result: Option<GuestStorage>,
    variadic: bool,
) -> GuestCallSignature {
    GuestCallSignature {
        abi: NativeCallAbi::of(abi),
        parameters: parameters
            .iter()
            .map(|storage| GuestValueLayout::Scalar(*storage))
            .collect(),
        result: result.map(GuestValueLayout::Scalar),
        variadic,
    }
}

/// Address of the named image export, or `None` when missing or forwarded.
#[must_use]
pub fn native_q3_export(image: &GuestImage, name: &str) -> Option<GuestAddress> {
    image.exports.iter().find_map(|export| match (&export.symbol, &export.target) {
        (GuestSymbolName::Name { name: symbol, .. }, GuestExportTarget::Address(address))
            if symbol == name =>
        {
            Some(*address)
        }
        _ => None,
    })
}

/// Read guest integer argument `index` (any int32/uint32/int64/uint64 lane).
pub fn call_integer(args: &[GuestCallValue], index: usize) -> Result<i64, GuestError> {
    match args.get(index) {
        Some(GuestCallValue::Int32(value)) => Ok(i64::from(*value)),
        Some(GuestCallValue::Uint32(value)) => Ok(i64::from(*value)),
        Some(GuestCallValue::Int64(value)) => Ok(*value),
        Some(GuestCallValue::Uint64(value)) => Ok(*value as i64),
        Some(_) => Err(GuestError::invalid("Guest integer required")),
        None => Err(GuestError::invalid(format!(
            "Guest argument {index} is missing"
        ))),
    }
}

/// Read guest pointer argument `index` (null stays null).
pub fn call_pointer(
    args: &[GuestCallValue],
    index: usize,
) -> Result<Option<GuestAddress>, GuestError> {
    match args.get(index) {
        Some(GuestCallValue::Pointer(address)) => Ok(*address),
        Some(_) => Err(GuestError::invalid("Guest pointer required")),
        None => Err(GuestError::invalid(format!(
            "Guest argument {index} is missing"
        ))),
    }
}

/// Read guest pointer argument `index`, rejecting null.
pub fn call_required_pointer(
    args: &[GuestCallValue],
    index: usize,
) -> Result<GuestAddress, GuestError> {
    match call_pointer(args, index)? {
        Some(address) => Ok(address),
        None => Err(GuestError::invalid("Nonnull guest pointer required")),
    }
}

/// Scripted native entry: inspects call arguments and may read or write guest
/// memory, standing in for real machine code in headless tests.
pub type NativeHandler =
    Box<dyn Fn(&[GuestCallValue], &mut SparseGuestMemory) -> Result<GuestCallResult, GuestError>>;

/// One recorded synthetic native call.
#[derive(Debug, Clone, PartialEq)]
pub struct RecordedNativeCall {
    /// Called entry address.
    pub target: GuestAddress,
    /// Signature the call was marshalled with.
    pub signature: GuestCallSignature,
    /// Marshalled arguments.
    pub arguments: Vec<GuestCallValue>,
    /// Callback reference attached to the call context.
    pub callback: GuestCallbackReference,
    /// Instruction budget granted to the call.
    pub instruction_budget: u64,
}

/// Headless stand-in for the native call runner: owns guest memory, records
/// every invocation, and serves entries from scripted handlers.
pub struct SyntheticNativeRunner {
    memory: SparseGuestMemory,
    calls: Vec<RecordedNativeCall>,
    handlers: HashMap<u64, NativeHandler>,
}

impl SyntheticNativeRunner {
    /// Wrap freshly mapped guest memory with no bound entries.
    #[must_use]
    pub fn new(memory: SparseGuestMemory) -> Self {
        Self {
            memory,
            calls: Vec::new(),
            handlers: HashMap::new(),
        }
    }

    /// Shared guest memory.
    #[must_use]
    pub fn memory(&self) -> &SparseGuestMemory {
        &self.memory
    }

    /// Shared guest memory for script setup and assertions.
    #[must_use]
    pub fn memory_mut(&mut self) -> &mut SparseGuestMemory {
        &mut self.memory
    }

    /// Recorded calls in invocation order.
    #[must_use]
    pub fn calls(&self) -> &[RecordedNativeCall] {
        &self.calls
    }

    /// Serve `target` with `handler`, replacing any previous binding.
    pub fn on_call(
        &mut self,
        target: GuestAddress,
        handler: impl Fn(&[GuestCallValue], &mut SparseGuestMemory) -> Result<GuestCallResult, GuestError>
        + 'static,
    ) {
        self.handlers.insert(target.offset, Box::new(handler));
    }

    /// Invoke `target` after checking the argument count against `signature`.
    pub fn invoke(
        &mut self,
        target: GuestAddress,
        signature: &GuestCallSignature,
        arguments: &[GuestCallValue],
        context: &GuestCallContext,
        instruction_budget: u64,
    ) -> Result<GuestCallResult, GuestError> {
        let fixed = signature.parameters.len();
        let arity_ok = if signature.variadic {
            arguments.len() >= fixed
        } else {
            arguments.len() == fixed
        };
        if !arity_ok {
            return Err(GuestError::abi(format!(
                "Native call arity {} does not match signature with {fixed} parameters",
                arguments.len()
            )));
        }
        let Self {
            memory,
            calls,
            handlers,
        } = self;
        calls.push(RecordedNativeCall {
            target,
            signature: signature.clone(),
            arguments: arguments.to_vec(),
            callback: context.callback.clone(),
            instruction_budget,
        });
        match handlers.get(&target.offset) {
            Some(handler) => handler(arguments, memory),
            None => Err(GuestError::callback(format!(
                "Unbound synthetic native entry at 0x{:x}",
                target.offset
            ))),
        }
    }
}

/// Options shared by both native module profiles. The caller owns image
/// loading and the shared guest process; the runner is borrowed per call.
#[derive(Debug, Clone, PartialEq)]
pub struct NativeQ3ModuleOptions {
    /// Loaded executable image.
    pub image: GuestImage,
    /// Base call context for module entries.
    pub context: GuestCallContext,
    /// Instruction budget granted to each entry call.
    pub instruction_budget: u64,
}

/// Quake Live game API 10 module: `dllEntry` hands back an export table.
/// Official Q3's `dllEntry(syscall)`/`vmMain` interface is a different
/// profile; images exporting `vmMain` or `GetGameAPI` are rejected.
pub struct QuakeLiveGameModule {
    image: GuestImage,
    context: GuestCallContext,
    instruction_budget: u64,
    exports: GuestAddress,
    api_version: i32,
}

impl QuakeLiveGameModule {
    /// Run `dllEntry`, validate the API version, and bind the export table.
    pub fn new(
        options: NativeQ3ModuleOptions,
        runner: &mut SyntheticNativeRunner,
        imports: GuestAddress,
    ) -> Result<Self, Q3ModuleError> {
        let space = runner.memory().address_space();
        if options.image.base.space != imports.space || options.image.base.space != space {
            return Err(Q3ModuleError::AddressSpaceMismatch);
        }
        if native_q3_export(&options.image, "vmMain").is_some()
            || native_q3_export(&options.image, "GetGameAPI").is_some()
        {
            return Err(Q3ModuleError::WrongProfile(
                "Module does not use the Quake Live table interface".to_string(),
            ));
        }
        let entry = native_q3_export(&options.image, "dllEntry")
            .ok_or_else(|| Q3ModuleError::MissingExport("dllEntry".to_string()))?;
        let pointer_bytes = options.image.abi.pointer_bytes();
        let output = runner.memory_mut().allocate(&GuestAllocationOptions {
            byte_length: pointer_bytes + 4,
            alignment: 8,
            permissions: GuestPermissions::ReadWrite,
            label: "QL dllEntry output".to_string(),
        })?;
        let version = runner.memory().offset(output, pointer_bytes as i64)?;
        let module = Self {
            image: options.image,
            context: options.context,
            instruction_budget: options.instruction_budget,
            exports: output,
            api_version: 0,
        };
        module.invoke(
            runner,
            entry,
            &[GuestStorage::Pointer, GuestStorage::Pointer, GuestStorage::Pointer],
            None,
            &[
                GuestCallValue::Pointer(Some(output)),
                GuestCallValue::Pointer(Some(imports)),
                GuestCallValue::Pointer(Some(version)),
            ],
        )?;
        let api_version = runner.memory_mut().read_i32(version)?;
        if api_version != QL_GAME_API_VERSION {
            let _ = runner.memory_mut().unmap(output, pointer_bytes + 4);
            return Err(Q3ModuleError::UnsupportedApi {
                found: api_version,
                expected: QL_GAME_API_VERSION,
            });
        }
        let table = runner.memory_mut().read_pointer(output)?;
        let Some(table) = table else {
            let _ = runner.memory_mut().unmap(output, pointer_bytes + 4);
            return Err(Q3ModuleError::NullExportTable);
        };
        runner
            .memory_mut()
            .check(table, QL_EXPORT_SLOT_COUNT * pointer_bytes, GuestAccess::Read)?;
        runner.memory_mut().unmap(output, pointer_bytes + 4)?;
        Ok(Self {
            exports: table,
            api_version,
            ..module
        })
    }

    /// Bound export-table address.
    #[must_use]
    pub fn exports(&self) -> GuestAddress {
        self.exports
    }

    /// Validated game API version (always 10).
    #[must_use]
    pub fn api_version(&self) -> i32 {
        self.api_version
    }

    /// Loaded image.
    #[must_use]
    pub fn image(&self) -> &GuestImage {
        &self.image
    }

    fn invoke(
        &self,
        runner: &mut SyntheticNativeRunner,
        target: GuestAddress,
        parameters: &[GuestStorage],
        result: Option<GuestStorage>,
        args: &[GuestCallValue],
    ) -> Result<GuestCallResult, Q3ModuleError> {
        let signature = native_q3_signature(self.image.abi, parameters, result, false);
        let context = GuestCallContext {
            callback: GuestCallbackReference::NativeGuest {
                module: self.image.module.clone(),
                address: target,
                abi: signature.abi,
            },
            ..self.context.clone()
        };
        Ok(runner.invoke(target, &signature, args, &context, self.instruction_budget)?)
    }

    fn call_slot(
        &self,
        runner: &mut SyntheticNativeRunner,
        slot: usize,
        parameters: &[GuestStorage],
        result: Option<GuestStorage>,
        args: &[GuestCallValue],
    ) -> Result<GuestCallResult, Q3ModuleError> {
        let pointer_bytes = runner.memory().pointer_bytes();
        let slot_address = runner
            .memory()
            .offset(self.exports, (slot * pointer_bytes) as i64)?;
        let target = runner.memory_mut().read_pointer(slot_address)?;
        let Some(target) = target else {
            return Err(Q3ModuleError::MissingExportSlot(slot));
        };
        runner.memory_mut().check(target, 1, GuestAccess::Execute)?;
        self.invoke(runner, target, parameters, result, args)
    }

    /// Export slot 2: register game cvars.
    pub fn register_cvars(&self, runner: &mut SyntheticNativeRunner) -> Result<(), Q3ModuleError> {
        self.call_slot(runner, 2, &[], None, &[])?;
        Ok(())
    }

    /// Export slot 3: initialize a level.
    pub fn initialize(
        &self,
        runner: &mut SyntheticNativeRunner,
        level_time: i32,
        random_seed: i32,
        restart: bool,
    ) -> Result<(), Q3ModuleError> {
        self.call_slot(
            runner,
            3,
            &[GuestStorage::Int32, GuestStorage::Int32, GuestStorage::Int32],
            None,
            &[
                GuestCallValue::Int32(level_time),
                GuestCallValue::Int32(random_seed),
                GuestCallValue::Int32(i32::from(restart)),
            ],
        )?;
        Ok(())
    }

    /// Export slot 0: shut down, optionally for a restart.
    pub fn shutdown(
        &self,
        runner: &mut SyntheticNativeRunner,
        restart: bool,
    ) -> Result<(), Q3ModuleError> {
        self.call_slot(
            runner,
            0,
            &[GuestStorage::Int32],
            None,
            &[GuestCallValue::Int32(i32::from(restart))],
        )?;
        Ok(())
    }

    /// Export slot 1: run one server frame.
    pub fn run_frame(
        &self,
        runner: &mut SyntheticNativeRunner,
        time: i32,
    ) -> Result<(), Q3ModuleError> {
        self.call_slot(
            runner,
            1,
            &[GuestStorage::Int32],
            None,
            &[GuestCallValue::Int32(time)],
        )?;
        Ok(())
    }

    /// Export slot 4: run a console command; nonzero means handled.
    pub fn console_command(
        &self,
        runner: &mut SyntheticNativeRunner,
    ) -> Result<bool, Q3ModuleError> {
        let result = self.call_slot(runner, 4, &[], Some(GuestStorage::Int32), &[])?;
        match result {
            GuestCallResult::Value(GuestCallValue::Int32(value)) => Ok(value != 0),
            _ => Err(Q3ModuleError::WrongResultType(
                "Quake Live ConsoleCommand returned the wrong ABI type".to_string(),
            )),
        }
    }

    /// Export slot 8: connect a client; returns the denied-reason pointer.
    pub fn client_connect(
        &self,
        runner: &mut SyntheticNativeRunner,
        client: i32,
        first_time: bool,
        is_bot: bool,
    ) -> Result<Option<GuestAddress>, Q3ModuleError> {
        let result = self.call_slot(
            runner,
            8,
            &[GuestStorage::Int32, GuestStorage::Int32, GuestStorage::Int32],
            Some(GuestStorage::Pointer),
            &[
                GuestCallValue::Int32(client),
                GuestCallValue::Int32(i32::from(first_time)),
                GuestCallValue::Int32(i32::from(is_bot)),
            ],
        )?;
        match result {
            GuestCallResult::Value(GuestCallValue::Pointer(address)) => Ok(address),
            _ => Err(Q3ModuleError::WrongResultType(
                "Quake Live ClientConnect returned the wrong ABI type".to_string(),
            )),
        }
    }
}

/// Official Q3 1.32 native i386 module: `dllEntry` receives the variadic
/// syscall address once, then `vmMain` takes a command plus twelve 32-bit
/// arguments. Absolute guest addresses; no QVM memory masking applies.
pub struct Quake3VmModule {
    options: NativeQ3ModuleOptions,
    vm_main: GuestAddress,
}

impl Quake3VmModule {
    /// Bind `dllEntry`/`vmMain` and hand the module its syscall address.
    pub fn new(
        options: NativeQ3ModuleOptions,
        runner: &mut SyntheticNativeRunner,
        syscall: GuestAddress,
    ) -> Result<Self, Q3ModuleError> {
        if options.image.abi.pointer_bytes() != 4 {
            return Err(Q3ModuleError::WrongProfile(
                "The Q3 1.32 native profile requires i386; later intptr_t profiles need their own declared ABI"
                    .to_string(),
            ));
        }
        let entry = native_q3_export(&options.image, "dllEntry");
        let main = native_q3_export(&options.image, "vmMain");
        let (Some(entry), Some(main)) = (entry, main) else {
            return Err(Q3ModuleError::MissingExport(
                "dllEntry and vmMain".to_string(),
            ));
        };
        let signature = native_q3_signature(options.image.abi, &[GuestStorage::Pointer], None, false);
        runner.invoke(
            entry,
            &signature,
            &[GuestCallValue::Pointer(Some(syscall))],
            &options.context,
            options.instruction_budget,
        )?;
        Ok(Self {
            options,
            vm_main: main,
        })
    }

    /// Bound `vmMain` address.
    #[must_use]
    pub fn vm_main(&self) -> GuestAddress {
        self.vm_main
    }

    /// Call `vmMain` with `command` plus up to twelve arguments (zero-padded).
    pub fn call(
        &self,
        runner: &mut SyntheticNativeRunner,
        command: i32,
        arguments: &[i32],
    ) -> Result<i32, Q3ModuleError> {
        if arguments.len() > Q3_VM_MAIN_ARGUMENTS {
            return Err(Q3ModuleError::TooManyArguments);
        }
        let mut args = Vec::with_capacity(1 + Q3_VM_MAIN_ARGUMENTS);
        args.push(GuestCallValue::Int32(command));
        for value in arguments {
            args.push(GuestCallValue::Int32(*value));
        }
        while args.len() < 1 + Q3_VM_MAIN_ARGUMENTS {
            args.push(GuestCallValue::Int32(0));
        }
        let parameters = vec![GuestStorage::Int32; 1 + Q3_VM_MAIN_ARGUMENTS];
        let signature =
            native_q3_signature(self.options.image.abi, &parameters, Some(GuestStorage::Int32), false);
        let context = GuestCallContext {
            callback: GuestCallbackReference::NativeGuest {
                module: self.options.image.module.clone(),
                address: self.vm_main,
                abi: signature.abi,
            },
            ..self.options.context.clone()
        };
        let result = runner.invoke(
            self.vm_main,
            &signature,
            &args,
            &context,
            self.options.instruction_budget,
        )?;
        match result {
            GuestCallResult::Value(GuestCallValue::Int32(value)) => Ok(value),
            _ => Err(Q3ModuleError::WrongResultType(
                "Q3 vmMain returned the wrong ABI type".to_string(),
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use qa_core::identity::ProviderId;
    use qa_guest::core::contracts::{
        CallbackId, ContentDigest, GuestExport, GuestMapOptions, GuestPermissions, ModuleIdentity,
    };

    use super::*;

    fn test_identity() -> ModuleIdentity {
        ModuleIdentity::new(
            ProviderId::new("test", "q3-native"),
            "test-game.so",
            ContentDigest::new("sha256", "abc"),
            "rev",
        )
    }

    fn test_context(module: &ModuleIdentity) -> GuestCallContext {
        GuestCallContext {
            module: module.clone(),
            callback: GuestCallbackReference::TypeScript {
                provider: ProviderId::new("test", "root"),
                callback: CallbackId::new("test", "root"),
            },
            parent: None,
            itself: None,
            other: None,
        }
    }

    fn test_image(module: &ModuleIdentity, abi: NativeAbi, space: u64) -> GuestImage {
        GuestImage {
            module: module.clone(),
            abi,
            base: GuestAddress::new(space, 0x10000),
            preferred_base: 0x10000,
            byte_length: 0x1000,
            entry_point: None,
            mappings: Vec::new(),
            imports: Vec::new(),
            exports: Vec::new(),
            tls: None,
            initializers: Vec::new(),
            finalizers: Vec::new(),
            unwind: Vec::new(),
        }
    }

    fn map_executable(memory: &mut SparseGuestMemory, base: u64) {
        let mut options = GuestMapOptions::new(base, 0x1000, GuestPermissions::ReadWriteExecute);
        options.label = "synthetic native code".to_string();
        memory.map(&options).expect("map synthetic code");
    }

    #[test]
    fn quake_live_module_binds_table_and_calls_slots() {
        let module_id = test_identity();
        let memory = SparseGuestMemory::new(module_id.clone(), 8, 0x10000).expect("memory");
        let space = memory.address_space();
        let mut runner = SyntheticNativeRunner::new(memory);
        map_executable(runner.memory_mut(), 0x20000);
        let entry = GuestAddress::new(space, 0x20000);
        let register = GuestAddress::new(space, 0x20010);
        let table_base = 0x30000;
        let mut table_options =
            GuestMapOptions::new(table_base, 0x1000, GuestPermissions::ReadWrite);
        table_options.label = "synthetic export table".to_string();
        runner.memory_mut().map(&table_options).expect("map table");
        let table = GuestAddress::new(space, table_base);
        runner.on_call(entry, move |args, memory| {
            assert_eq!(args.len(), 3);
            let output = call_required_pointer(args, 0).expect("output");
            let version = call_required_pointer(args, 2).expect("version");
            memory.write_pointer(output, Some(table)).expect("write table");
            memory.write_i32(version, QL_GAME_API_VERSION).expect("write api");
            Ok(GuestCallResult::Void)
        });
        runner.on_call(register, |args, _| {
            assert!(args.is_empty());
            Ok(GuestCallResult::Void)
        });
        let mut image = test_image(&module_id, NativeAbi::LinuxX86_64, space);
        image.exports.push(GuestExport {
            symbol: GuestSymbolName::Name {
                name: "dllEntry".to_string(),
                version: None,
            },
            target: GuestExportTarget::Address(entry),
        });
        let context = test_context(&module_id);
        let options = NativeQ3ModuleOptions {
            image,
            context,
            instruction_budget: 1_000_000,
        };
        let imports = GuestAddress::new(space, 0x40000);
        let module = QuakeLiveGameModule::new(options, &mut runner, imports).expect("bind module");
        assert_eq!(module.api_version(), QL_GAME_API_VERSION);
        assert_eq!(module.exports(), table);
        let slot = runner
            .memory()
            .offset(table, 2 * 8)
            .expect("slot address");
        runner
            .memory_mut()
            .write_pointer(slot, Some(register))
            .expect("write slot");
        module.register_cvars(&mut runner).expect("register cvars");
        assert_eq!(runner.calls().len(), 2);
        assert_eq!(runner.calls()[1].target, register);
        assert!(matches!(
            runner.calls()[1].callback,
            GuestCallbackReference::NativeGuest { .. }
        ));
    }

    #[test]
    fn quake_live_module_rejects_wrong_profile_and_api() {
        let module_id = test_identity();
        let memory = SparseGuestMemory::new(module_id.clone(), 8, 0x10000).expect("memory");
        let space = memory.address_space();
        let mut runner = SyntheticNativeRunner::new(memory);
        map_executable(runner.memory_mut(), 0x20000);
        let entry = GuestAddress::new(space, 0x20000);
        let mut image = test_image(&module_id, NativeAbi::LinuxX86_64, space);
        image.exports.push(GuestExport {
            symbol: GuestSymbolName::Name {
                name: "vmMain".to_string(),
                version: None,
            },
            target: GuestExportTarget::Address(entry),
        });
        image.exports.push(GuestExport {
            symbol: GuestSymbolName::Name {
                name: "dllEntry".to_string(),
                version: None,
            },
            target: GuestExportTarget::Address(entry),
        });
        let options = NativeQ3ModuleOptions {
            image,
            context: test_context(&module_id),
            instruction_budget: 1000,
        };
        let imports = GuestAddress::new(space, 0x40000);
        assert!(matches!(
            QuakeLiveGameModule::new(options, &mut runner, imports),
            Err(Q3ModuleError::WrongProfile(_))
        ));

        let mut image = test_image(&module_id, NativeAbi::LinuxX86_64, space);
        image.exports.push(GuestExport {
            symbol: GuestSymbolName::Name {
                name: "dllEntry".to_string(),
                version: None,
            },
            target: GuestExportTarget::Address(entry),
        });
        runner.on_call(entry, |args, memory| {
            let output = call_required_pointer(args, 0).expect("output");
            let version = call_required_pointer(args, 2).expect("version");
            memory
                .write_pointer(output, Some(GuestAddress::new(output.space, 0x30000)))
                .expect("write table");
            memory.write_i32(version, 9).expect("write api");
            Ok(GuestCallResult::Void)
        });
        let options = NativeQ3ModuleOptions {
            image,
            context: test_context(&module_id),
            instruction_budget: 1000,
        };
        assert!(matches!(
            QuakeLiveGameModule::new(options, &mut runner, imports),
            Err(Q3ModuleError::UnsupportedApi {
                found: 9,
                expected: QL_GAME_API_VERSION
            })
        ));
    }

    #[test]
    fn q3_vm_module_pads_arguments_and_routes_syscall() {
        let module_id = test_identity();
        let memory = SparseGuestMemory::new(module_id.clone(), 4, 0x10000).expect("memory");
        let space = memory.address_space();
        let mut runner = SyntheticNativeRunner::new(memory);
        map_executable(runner.memory_mut(), 0x20000);
        let entry = GuestAddress::new(space, 0x20000);
        let main = GuestAddress::new(space, 0x20040);
        let syscall = GuestAddress::new(space, 0x7e00_0000);
        runner.on_call(entry, move |args, _| {
            assert_eq!(args, &[GuestCallValue::Pointer(Some(syscall))]);
            Ok(GuestCallResult::Void)
        });
        runner.on_call(main, |args, _| {
            assert_eq!(args.len(), 1 + Q3_VM_MAIN_ARGUMENTS);
            assert_eq!(args[0], GuestCallValue::Int32(7));
            assert_eq!(args[1], GuestCallValue::Int32(11));
            assert_eq!(args[2], GuestCallValue::Int32(13));
            assert!(args[3..].iter().all(|arg| *arg == GuestCallValue::Int32(0)));
            Ok(GuestCallResult::Value(GuestCallValue::Int32(42)))
        });
        let mut image = test_image(&module_id, NativeAbi::LinuxI386, space);
        for (name, target) in [("dllEntry", entry), ("vmMain", main)] {
            image.exports.push(GuestExport {
                symbol: GuestSymbolName::Name {
                    name: name.to_string(),
                    version: None,
                },
                target: GuestExportTarget::Address(target),
            });
        }
        let options = NativeQ3ModuleOptions {
            image,
            context: test_context(&module_id),
            instruction_budget: 5000,
        };
        let module = Quake3VmModule::new(options, &mut runner, syscall).expect("bind vm module");
        assert_eq!(module.vm_main(), main);
        assert_eq!(module.call(&mut runner, 7, &[11, 13]).expect("call"), 42);
        assert!(matches!(
            module.call(&mut runner, 0, &[0; 13]),
            Err(Q3ModuleError::TooManyArguments)
        ));
    }

    #[test]
    fn call_integer_and_pointer_helpers_match_donor_kinds() {
        let address = GuestAddress::new(9, 0x1000);
        let args = [
            GuestCallValue::Int32(-3),
            GuestCallValue::Uint32(7),
            GuestCallValue::Int64(-9),
            GuestCallValue::Uint64(11),
            GuestCallValue::Pointer(Some(address)),
            GuestCallValue::Pointer(None),
        ];
        assert_eq!(call_integer(&args, 0).expect("i32"), -3);
        assert_eq!(call_integer(&args, 1).expect("u32"), 7);
        assert_eq!(call_integer(&args, 2).expect("i64"), -9);
        assert_eq!(call_integer(&args, 3).expect("u64"), 11);
        assert!(call_integer(&args, 4).is_err());
        assert!(call_integer(&args, 9).is_err());
        assert_eq!(call_pointer(&args, 4).expect("ptr"), Some(address));
        assert_eq!(call_pointer(&args, 5).expect("null"), None);
        assert!(call_pointer(&args, 0).is_err());
        assert_eq!(call_required_pointer(&args, 4).expect("required"), address);
        assert!(call_required_pointer(&args, 5).is_err());
    }
}
