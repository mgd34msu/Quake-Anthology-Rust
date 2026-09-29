//! System V guest process: image loading, lifecycle, and import resolution.
//!
//! Donor: `src/guest/runtime/system-v/runtime.ts`. One Linux guest process
//! and thread, using the caller's CPU, memory, and callback namespace.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use crate::abi::runner::{GuestCallFailure, GuestCallRequest, GuestCallRunner};
use crate::core::callbacks::HookState;
use crate::core::contracts::{
    GuestAddress, GuestAllocationOptions, GuestCallContext, GuestCallResult, GuestCallValue,
    GuestCallbackReference, GuestImage, GuestImport, GuestImportResolution, GuestImportResolver,
    GuestPermissions, GuestStorage, GuestSymbolName, ModuleIdentity,
};
use crate::core::memory::SparseGuestMemory;
use crate::core::registers::GuestProcessorState;
use crate::elf::loader::{load_elf, ElfGuestImage, ElfLoadOptions, ElfTlsExport};
use crate::elf::parse::inspect_elf;
use crate::elf::relocate::{ElfTlsBindings, ElfTlsModule, ElfTlsResolution};
use crate::error::GuestError;
use crate::runtime::common::memory::{string_bytes, write_unsigned};
use crate::runtime::system_v::contracts::{
    invoke_nested, symbol_key, unsupported_system_v, SharedSystemV, SystemVContext,
    SystemVImportCoverage, SystemVInitializeOptions, SystemVRuntimeOptions, SystemVServiceRegistrar,
    SystemVSupport, SystemVTlsBlock,
};
use crate::runtime::system_v::cxx::install_cxx;
use crate::runtime::system_v::cxx_data::{build_abi, SharedAbi};
use crate::runtime::system_v::iostream::{build_iostreams, SystemVIostreams};
use crate::runtime::system_v::libc::install_libc;
use crate::runtime::system_v::locale::build_classic_locale;
use crate::runtime::system_v::stdio::build_stdio;

const LIBRARIES: [&str; 6] = [
    "libc.so.6",
    "libm.so.6",
    "libstdc++.so.6",
    "libgcc_s.so.1",
    "ld-linux.so.2",
    "ld-linux-x86-64.so.2",
];

/// Image lifecycle state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SystemVImageState {
    /// Loaded, not initialized.
    Loaded,
    /// Initialization running.
    Initializing,
    /// Initialized.
    Initialized,
    /// Finalization running.
    Finalizing,
    /// Finalized.
    Finalized,
    /// Lifecycle failed.
    Failed,
}

/// One loaded image with its lifecycle state.
#[derive(Debug, Clone)]
pub struct SystemVLoadedImage {
    /// Loaded image.
    pub image: ElfGuestImage,
    /// Lifecycle state.
    pub state: SystemVImageState,
}

/// Lifecycle trace phase.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SystemVLifecyclePhase {
    /// Initializer call.
    Initialize,
    /// Finalizer call.
    Finalize,
}

/// One lifecycle call record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SystemVLifecycleEvent {
    /// Image module.
    pub image: ModuleIdentity,
    /// Lifecycle phase.
    pub phase: SystemVLifecyclePhase,
    /// Call target.
    pub target: GuestAddress,
}

/// TLS bindings for one image load.
struct LoadTlsBindings {
    module_id: u64,
    thread_pointer_offset: i64,
    thread_pointer: GuestAddress,
    shared: SharedSystemV,
    images: Vec<(ModuleIdentity, Option<String>, Vec<ElfTlsExport>)>,
}

impl ElfTlsBindings for LoadTlsBindings {
    fn current(&self) -> ElfTlsModule {
        ElfTlsModule {
            module_id: self.module_id,
            thread_pointer_offset: Some(self.thread_pointer_offset),
        }
    }

    fn resolve(
        &self,
        import: &GuestImport,
        _requesting: &GuestImage,
    ) -> Result<Option<ElfTlsResolution>, GuestError> {
        let GuestSymbolName::Name { name, version } = &import.symbol else {
            return Ok(None);
        };
        for (module, soname, tls_exports) in &self.images {
            if !import.library.is_empty() && soname.as_deref() != Some(import.library.as_str()) {
                continue;
            }
            let entry = tls_exports.iter().find(|entry| match &entry.symbol {
                GuestSymbolName::Name {
                    name: candidate,
                    version: candidate_version,
                } => candidate == name && candidate_version == version,
                GuestSymbolName::Ordinal(_) => false,
            });
            let block = self
                .shared
                .borrow()
                .tls_blocks
                .iter()
                .find(|block| block.module.id == module.id && block.module.digest == module.digest)
                .cloned();
            if let (Some(entry), Some(block)) = (entry, block) {
                return Ok(Some(ElfTlsResolution {
                    module_id: block.module_id,
                    thread_pointer_offset: Some(
                        block.address.offset as i64 - self.thread_pointer.offset as i64,
                    ),
                    offset: entry.offset,
                }));
            }
        }
        Ok(None)
    }
}

/// One Linux guest process and thread.
pub struct SystemVGuestRuntime {
    context: SystemVContext,
    pointer_bytes: usize,
    address_space: u64,
    argv: Vec<String>,
    thread_pointer: GuestAddress,
    dtv: GuestAddress,
    errno_address: GuestAddress,
    argv_address: GuestAddress,
    envp_address: GuestAddress,
    empty_string: GuestAddress,
    images: Vec<SystemVLoadedImage>,
    lifecycle_trace: Vec<SystemVLifecycleEvent>,
    unique_symbols: Rc<RefCell<HashMap<String, GuestAddress>>>,
    loading: Cell<bool>,
    /// Constructed iostreams (streams, locale, FILE layer).
    pub iostreams: SystemVIostreams,
    /// Shared C++ ABI data.
    pub abi: SharedAbi,
}

impl SystemVGuestRuntime {
    /// Build a runtime over borrowed guest memory and shared hooks.
    pub fn new(
        hooks: Rc<HookState>,
        memory: &mut SparseGuestMemory,
        options: SystemVRuntimeOptions,
    ) -> Result<Self, GuestError> {
        let pointer_bytes = memory.pointer_bytes();
        let address_space = memory.address_space();
        let shared: SharedSystemV = Rc::new(RefCell::new(crate::runtime::system_v::contracts::SystemVShared {
            capabilities: options.capabilities.clone(),
            budget: 100_000,
            ..Default::default()
        }));
        let tls_area = memory.allocate(&GuestAllocationOptions {
            byte_length: 0x11000,
            alignment: 4096,
            permissions: GuestPermissions::ReadWrite,
            label: "System V static TLS and TCB".to_string(),
        })?;
        let thread_pointer = memory.offset(tls_area, 0x10000)?;
        memory.write_pointer(thread_pointer, Some(thread_pointer))?;
        memory.write_pointer(memory.offset(thread_pointer, 2 * pointer_bytes as i64)?, Some(thread_pointer))?;
        let dtv_allocation = memory.allocate(&GuestAllocationOptions {
            byte_length: 1026 * 2 * pointer_bytes,
            alignment: 16,
            permissions: GuestPermissions::ReadWrite,
            label: "System V thread dynamic vector".to_string(),
        })?;
        let dtv = memory.offset(dtv_allocation, 2 * pointer_bytes as i64)?;
        write_unsigned(memory, dtv_allocation, pointer_bytes, 1024)?;
        write_unsigned(memory, dtv, pointer_bytes, 1)?;
        memory.write_pointer(memory.offset(thread_pointer, pointer_bytes as i64)?, Some(dtv))?;
        let errno_address = memory.allocate(&GuestAllocationOptions {
            byte_length: 4,
            alignment: 4,
            permissions: GuestPermissions::ReadWrite,
            label: "System V thread errno".to_string(),
        })?;
        let argv_address = write_strings(memory, &options.argv, "System V argument vector", "System V argument string")?;
        let envp_address = write_strings(memory, &options.environment, "System V argument vector", "System V argument string")?;
        let empty_string = memory.allocate(&GuestAllocationOptions {
            byte_length: pointer_bytes * 4,
            alignment: 16,
            permissions: GuestPermissions::ReadWrite,
            label: "GLIBCXX_3.4 shared empty string representation".to_string(),
        })?;
        let mut registrar = SystemVServiceRegistrar::new(
            memory,
            hooks,
            Rc::clone(&shared),
            pointer_bytes,
            errno_address,
            thread_pointer,
            empty_string,
        );
        registrar.data(
            "libstdc++.so.6",
            "_ZNSs4_Rep20_S_empty_rep_storageE",
            Some("GLIBCXX_3.4"),
            empty_string,
            pointer_bytes * 4,
        )?;
        install_libc(&mut registrar)?;
        install_cxx(&mut registrar)?;
        let abi_data = build_abi(&registrar.context, registrar.memory)?;
        let abi: SharedAbi = Rc::new(RefCell::new(abi_data));
        let stdio = build_stdio(&mut registrar)?;
        let locale = build_classic_locale(&mut registrar, &abi)?;
        let context = registrar.context.clone();
        let iostreams = build_iostreams(&context, registrar.memory, &abi, stdio, locale)?;
        Ok(Self {
            context,
            pointer_bytes,
            address_space,
            argv: options.argv,
            thread_pointer,
            dtv,
            errno_address,
            argv_address,
            envp_address,
            empty_string,
            images: Vec::new(),
            lifecycle_trace: Vec::new(),
            unique_symbols: Rc::new(RefCell::new(HashMap::new())),
            loading: Cell::new(false),
            iostreams,
            abi,
        })
    }

    /// Service context.
    pub fn context(&self) -> &SystemVContext {
        &self.context
    }

    /// Loaded images.
    pub fn images(&self) -> Vec<ElfGuestImage> {
        self.images.iter().map(|entry| entry.image.clone()).collect()
    }

    /// Symbol coverage records.
    pub fn coverage(&self) -> Vec<SystemVImportCoverage> {
        self.context
            .shared
            .borrow()
            .symbols
            .values()
            .map(|entry| entry.coverage.clone())
            .collect()
    }

    /// Lifecycle call trace.
    pub fn lifecycle_trace(&self) -> Vec<SystemVLifecycleEvent> {
        self.lifecycle_trace.clone()
    }

    /// Thread pointer.
    pub fn thread_pointer(&self) -> GuestAddress {
        self.thread_pointer
    }

    /// Thread `errno` slot.
    pub fn errno_address(&self) -> GuestAddress {
        self.errno_address
    }

    /// Shared empty-string representation.
    pub fn empty_string(&self) -> GuestAddress {
        self.empty_string
    }

    /// Libraries provided by this process.
    pub fn dependencies(&self) -> HashSet<String> {
        let mut dependencies: HashSet<String> = LIBRARIES.iter().map(|name| name.to_string()).collect();
        for entry in &self.images {
            if let Some(soname) = &entry.image.soname {
                dependencies.insert(soname.clone());
            }
        }
        dependencies
    }

    /// Attach a runner, installing the thread pointer segment base.
    pub fn attach_runner(&self, runner: &mut GuestCallRunner<'_>) -> Result<(), GuestError> {
        if !Rc::ptr_eq(runner.hooks(), &self.context.hooks) {
            return Err(GuestError::invalid("System V runner belongs to another process"));
        }
        let (state, memory) = runner.cpu_parts();
        if memory.address_space() != self.address_space {
            return Err(GuestError::invalid("System V runner belongs to another process"));
        }
        if self.pointer_bytes == 8 {
            state.segments[GuestProcessorState::FS].base = self.thread_pointer.offset;
        } else {
            state.segments[GuestProcessorState::GS].base = self.thread_pointer.offset;
        }
        Ok(())
    }

    /// Current `errno`.
    pub fn errno(&self, memory: &mut SparseGuestMemory) -> Result<i32, GuestError> {
        memory.read_i32(self.errno_address)
    }

    /// Assign `errno`.
    pub fn set_errno(&self, memory: &mut SparseGuestMemory, value: i32) -> Result<(), GuestError> {
        memory.write_i32(self.errno_address, value)
    }

    /// Resolve a registered symbol's data length, if any.
    pub fn resolve_symbol_size(&self, import: &GuestImport) -> Option<usize> {
        let GuestSymbolName::Name { name, version } = &import.symbol else {
            return None;
        };
        self.context
            .shared
            .borrow()
            .symbols
            .get(&symbol_key(&import.library, name, version.as_deref()))
            .and_then(|entry| entry.byte_length)
    }

    /// Invoke a nested guest call through an attached runner.
    pub fn invoke(
        &self,
        runner: &mut GuestCallRunner<'_>,
        context: &GuestCallContext,
        target: GuestAddress,
        parameters: &[GuestStorage],
        result: Option<GuestStorage>,
        args: Vec<GuestCallValue>,
    ) -> Result<GuestCallResult, GuestError> {
        let signature = self.context.signature(parameters, result);
        let budget = self.context.shared.borrow().budget;
        let request = GuestCallRequest {
            target,
            signature: signature.clone(),
            arguments: args,
            context: GuestCallContext {
                module: context.module.clone(),
                callback: GuestCallbackReference::NativeGuest {
                    module: context.module.clone(),
                    address: target,
                    abi: signature.abi,
                },
                parent: Some(Box::new(context.clone())),
                itself: context.itself.clone(),
                other: context.other.clone(),
            },
            instruction_budget: budget,
        };
        runner.invoke(&request).map_err(|failure| match failure {
            GuestCallFailure::Guest(error) => error,
            GuestCallFailure::Stopped { stop, .. } => {
                GuestError::callback(format!("Nested System V guest call stopped: {stop:?}"))
            }
        })
    }

    /// Load an ELF image into the process.
    pub fn load(
        &mut self,
        memory: &mut SparseGuestMemory,
        bytes: &[u8],
        module: ModuleIdentity,
        load_bias: u64,
    ) -> Result<ElfGuestImage, GuestError> {
        if self.loading.get() {
            return Err(GuestError::callback("System V recursive image loading is unsupported"));
        }
        let inspection = inspect_elf(bytes)?;
        let segment = inspection.segments.iter().find(|entry| entry.segment_type == 7);
        let module_id = self.context.shared.borrow().tls_blocks.len() as u64 + 1;
        if module_id > 1024 {
            return Err(GuestError::invalid("System V thread TLS module limit exceeded"));
        }
        let size = segment.map(|entry| entry.memory_size as u64).unwrap_or(0);
        let alignment = match segment {
            None => 1,
            Some(entry) if entry.alignment == 0 => 1,
            Some(entry) => entry.alignment,
        };
        if alignment > 4096 {
            return Err(GuestError::invalid(
                "System V static TLS alignment exceeds the thread allocation alignment",
            ));
        }
        let tls_used = self.context.shared.borrow().tls_used;
        let next = (tls_used + size + alignment - 1) / alignment * alignment;
        if next > 0x10000 {
            return Err(GuestError::invalid("System V static TLS exceeds thread allocation"));
        }
        let block = memory.offset(self.thread_pointer, -(next as i64))?;
        // Relocation validates provider addresses. No guest code runs while
        // unresolved data reservations are readable; execution sees
        // inaccessible pages, never fabricated RTTI.
        let reservations: Vec<GuestAddress> = self
            .context
            .shared
            .borrow()
            .symbols
            .values()
            .filter(|entry| entry.coverage.support == SystemVSupport::UnsupportedData)
            .map(|entry| entry.address)
            .collect();
        for address in &reservations {
            memory.protect(*address, 4096, GuestPermissions::Read)?;
        }
        self.loading.set(true);
        let tls = LoadTlsBindings {
            module_id,
            thread_pointer_offset: -(next as i64),
            thread_pointer: self.thread_pointer,
            shared: Rc::clone(&self.context.shared),
            images: self
                .images
                .iter()
                .map(|entry| {
                    (
                        entry.image.image.module.clone(),
                        entry.image.soname.clone(),
                        entry.image.tls_exports.clone(),
                    )
                })
                .collect(),
        };
        let shared = Rc::clone(&self.context.shared);
        let resolve_size = |import: &GuestImport, _image: &GuestImage| -> Result<Option<usize>, GuestError> {
            let GuestSymbolName::Name { name, version } = &import.symbol else {
                return Ok(None);
            };
            Ok(shared
                .borrow()
                .symbols
                .get(&symbol_key(&import.library, name, version.as_deref()))
                .and_then(|entry| entry.byte_length))
        };
        let dependencies = self.dependencies();
        let image = {
            let mut unique = self.unique_symbols.borrow_mut();
            let result = load_elf(ElfLoadOptions {
                bytes,
                module: module.clone(),
                memory: &mut *memory,
                load_bias,
                resolver: &*self,
                dependencies: &dependencies,
                tls: Some(&tls),
                resolve_indirect: None,
                resolve_symbol_size: Some(&resolve_size),
                unique_symbols: Some(&mut *unique),
            });
            self.loading.set(false);
            for address in &reservations {
                memory.protect(*address, 4096, GuestPermissions::None)?;
            }
            result?
        };
        if let Some(tls_template) = &image.image.tls {
            memory.write(block, &tls_template.initialized)?;
            memory.write_pointer(
                memory.offset(self.dtv, (module_id * 2 * self.pointer_bytes as u64) as i64)?,
                Some(block),
            )?;
            write_unsigned(memory, self.dtv, self.pointer_bytes, (module_id + 1) as i128)?;
            self.context.shared.borrow_mut().tls_blocks.push(SystemVTlsBlock {
                module: image.image.module.clone(),
                module_id,
                address: block,
                byte_length: size as usize,
            });
            self.context.shared.borrow_mut().tls_used = next;
        }
        self.images.push(SystemVLoadedImage {
            image: image.clone(),
            state: SystemVImageState::Loaded,
        });
        Ok(image)
    }

    /// Run image initializers (dependencies first).
    pub fn initialize(
        &mut self,
        runner: &mut GuestCallRunner<'_>,
        module: &ModuleIdentity,
        options: &SystemVInitializeOptions,
    ) -> Result<(), GuestError> {
        let index = self.images.iter().position(|entry| entry.image.image.module == *module);
        let Some(index) = index else {
            return Err(GuestError::callback("System V image is not loaded in this process"));
        };
        match self.images[index].state {
            SystemVImageState::Initialized | SystemVImageState::Initializing => return Ok(()),
            SystemVImageState::Loaded => {}
            state => {
                return Err(GuestError::callback(format!(
                    "Cannot initialize System V image in state {state:?}"
                )));
            }
        }
        self.context.shared.borrow_mut().budget = options.instruction_budget;
        self.images[index].state = SystemVImageState::Initializing;
        let result = self.initialize_inner(runner, index, options);
        if result.is_err() {
            self.images[index].state = SystemVImageState::Failed;
        } else {
            self.images[index].state = SystemVImageState::Initialized;
        }
        result
    }

    fn initialize_inner(
        &mut self,
        runner: &mut GuestCallRunner<'_>,
        index: usize,
        options: &SystemVInitializeOptions,
    ) -> Result<(), GuestError> {
        let preinitializers = self.images[index].image.preinitializers.clone();
        for target in preinitializers {
            self.run_initializer(runner, index, target, &options.context)?;
        }
        let needed = self.images[index].image.needed_libraries.clone();
        for dependency in needed {
            let other = self.images.iter().position(|entry| entry.image.soname.as_deref() == Some(dependency.as_str()));
            if let Some(other) = other {
                let module = self.images[other].image.image.module.clone();
                self.initialize(runner, &module, options)?;
            }
        }
        let initializers = self.images[index].image.image.initializers.clone();
        for target in initializers {
            self.run_initializer(runner, index, target, &options.context)?;
        }
        Ok(())
    }

    fn run_initializer(
        &mut self,
        runner: &mut GuestCallRunner<'_>,
        index: usize,
        target: GuestAddress,
        context: &GuestCallContext,
    ) -> Result<(), GuestError> {
        self.invoke(
            runner,
            context,
            target,
            &[GuestStorage::Int32, GuestStorage::Pointer, GuestStorage::Pointer],
            None,
            vec![
                GuestCallValue::Int32(self.argv.len() as i32),
                GuestCallValue::Pointer(Some(self.argv_address)),
                GuestCallValue::Pointer(Some(self.envp_address)),
            ],
        )?;
        let module = self.images[index].image.image.module.clone();
        self.lifecycle_trace.push(SystemVLifecycleEvent {
            image: module,
            phase: SystemVLifecyclePhase::Initialize,
            target,
        });
        Ok(())
    }

    /// Run image finalizers.
    pub fn finalize(
        &mut self,
        runner: &mut GuestCallRunner<'_>,
        module: &ModuleIdentity,
        options: &SystemVInitializeOptions,
    ) -> Result<(), GuestError> {
        let index = self.images.iter().position(|entry| entry.image.image.module == *module);
        match index {
            Some(index) if self.images[index].state == SystemVImageState::Finalized => return Ok(()),
            Some(index) if self.images[index].state == SystemVImageState::Initialized => {
                self.context.shared.borrow_mut().budget = options.instruction_budget;
                self.images[index].state = SystemVImageState::Finalizing;
                let finalizers = self.images[index].image.image.finalizers.clone();
                let mut failure = None;
                for target in finalizers {
                    if let Err(error) = self.invoke(runner, &options.context, target, &[], None, vec![]) {
                        failure = Some(error);
                        break;
                    }
                    let module = self.images[index].image.image.module.clone();
                    self.lifecycle_trace.push(SystemVLifecycleEvent {
                        image: module,
                        phase: SystemVLifecyclePhase::Finalize,
                        target,
                    });
                }
                self.images[index].state = if failure.is_some() {
                    SystemVImageState::Failed
                } else {
                    SystemVImageState::Finalized
                };
                if let Some(error) = failure {
                    return Err(error);
                }
                Ok(())
            }
            _ => Err(GuestError::callback("Only an initialized System V image can finalize")),
        }
    }

    /// Run registered destructors through an attached runner.
    pub fn finalize_destructors(
        &self,
        runner: &mut GuestCallRunner<'_>,
        context: &GuestCallContext,
        dso: Option<GuestAddress>,
    ) -> Result<(), GuestError> {
        loop {
            let next = {
                let mut shared = self.context.shared.borrow_mut();
                let index = shared.destructors.iter().rposition(|entry| {
                    !entry.called
                        && (dso.is_none() || entry.dso.map(|dso_| dso_.offset) == dso.map(|dso_| dso_.offset))
                });
                match index {
                    Some(index) => {
                        shared.destructors[index].called = true;
                        let entry = shared.destructors[index];
                        Some((entry.target, entry.argument))
                    }
                    None => None,
                }
            };
            let Some((target, argument)) = next else {
                return Ok(());
            };
            self.invoke(
                runner,
                context,
                target,
                &[GuestStorage::Pointer],
                None,
                vec![GuestCallValue::Pointer(argument)],
            )?;
        }
    }

    /// Resolve a TLS address within a loaded block.
    pub fn tls_address(
        &self,
        memory: &mut SparseGuestMemory,
        module_id: u64,
        offset: u64,
    ) -> Result<GuestAddress, GuestError> {
        crate::runtime::system_v::contracts::tls_address(
            &self.context.shared,
            memory,
            self.thread_pointer,
            module_id,
            offset,
        )
    }
}

impl GuestImportResolver for SystemVGuestRuntime {
    fn resolve(
        &self,
        memory: &mut SparseGuestMemory,
        import: &GuestImport,
        requesting: &GuestImage,
    ) -> GuestImportResolution {
        let GuestSymbolName::Name { name, version } = import.symbol.clone() else {
            return GuestImportResolution::Unresolved {
                import: import.clone(),
                detail: "ELF has no ordinal import namespace".to_string(),
            };
        };
        for entry in &self.images {
            if entry.image.image.module == requesting.module {
                continue;
            }
            if !import.library.is_empty() && entry.image.soname.as_deref() != Some(import.library.as_str()) {
                continue;
            }
            let exported = entry.image.image.exports.iter().find(|export| match &export.symbol {
                GuestSymbolName::Name {
                    name: candidate,
                    version: candidate_version,
                } => candidate == &name && candidate_version == &version,
                GuestSymbolName::Ordinal(_) => false,
            });
            if let Some(exported) = exported {
                if let crate::core::contracts::GuestExportTarget::Address(address) = exported.target {
                    return GuestImportResolution::Guest {
                        address,
                        module: entry.image.image.module.clone(),
                    };
                }
            }
        }
        let provider = if import.library.is_empty() {
            self.context
                .shared
                .borrow()
                .symbols
                .values()
                .find(|entry| entry.coverage.name == name && entry.coverage.version == version)
                .cloned()
                .and_then(|entry| {
                    let callback = entry.callback?;
                    Some((entry.address, callback))
                })
                .map(|(address, callback)| GuestImportResolution::Host { address, callback })
        } else {
            self.context.resolution(memory, &import.library, &name, version.as_deref())
        };
        if let Some(provider) = provider {
            return provider;
        }
        if import.weak
            || (import.library.is_empty()
                && requesting.exports.iter().any(|export| match &export.symbol {
                    GuestSymbolName::Name {
                        name: candidate,
                        ..
                    } => candidate == &name,
                    GuestSymbolName::Ordinal(_) => false,
                }))
        {
            return GuestImportResolution::Unresolved {
                import: import.clone(),
                detail: "No provider in this process; preserve weak-null or local definition".to_string(),
            };
        }
        if name.starts_with("_ZTV") || name.starts_with("_ZTI") {
            let address = memory.allocate(&GuestAllocationOptions {
                byte_length: 4096,
                alignment: 4096,
                permissions: if self.loading.get() {
                    GuestPermissions::Read
                } else {
                    GuestPermissions::None
                },
                label: format!("Unsupported System V data {name}@{}", version.as_deref().unwrap_or("")),
            });
            let Ok(address) = address else {
                return GuestImportResolution::Unresolved {
                    import: import.clone(),
                    detail: "System V data reservation failed".to_string(),
                };
            };
            let id = symbol_key(&import.library, &name, version.as_deref());
            self.context.shared.borrow_mut().symbols.insert(
                id,
                crate::runtime::system_v::contracts::SystemVSymbolEntry {
                    coverage: SystemVImportCoverage {
                        library: import.library.clone(),
                        name: name.clone(),
                        version: version.clone(),
                        support: SystemVSupport::UnsupportedData,
                        reached: 0,
                    },
                    address,
                    callback: None,
                    byte_length: None,
                },
            );
            return GuestImportResolution::Guest {
                address,
                module: memory.module().clone(),
            };
        }
        let detail = "no implementation for this exact symbol version".to_string();
        let library = import.library.clone();
        let symbol = name.clone();
        let invoke_library = library.clone();
        let invoke_symbol = symbol.clone();
        let invoke_version = version.clone();
        match self.context.service(
            memory,
            &library,
            &symbol,
            &[version.as_deref()],
            &[],
            None,
            Rc::new(move |_, context, _| {
                let _ = context;
                Err(unsupported_system_v(&invoke_library, &invoke_symbol, invoke_version.as_deref(), detail.clone()))
            }),
        ) {
            Ok(()) => self
                .context
                .resolution(memory, &import.library, &name, version.as_deref())
                .unwrap_or(GuestImportResolution::Unresolved {
                    import: import.clone(),
                    detail: "System V lazy symbol registration failed".to_string(),
                }),
            Err(error) => GuestImportResolution::Unresolved {
                import: import.clone(),
                detail: error.to_string(),
            },
        }
    }
}

impl SystemVGuestRuntime {
    /// Nested invoke from a host closure.
    pub fn invoke_nested(
        &self,
        ctx: &mut crate::core::callbacks::HostCallContext<'_, '_>,
        context: &GuestCallContext,
        target: GuestAddress,
        parameters: &[GuestStorage],
        result: Option<GuestStorage>,
        args: Vec<GuestCallValue>,
    ) -> Result<GuestCallResult, GuestError> {
        invoke_nested(
            ctx,
            &self.context.shared,
            self.pointer_bytes,
            context,
            target,
            parameters,
            result,
            args,
        )
    }
}

fn write_strings(
    memory: &mut SparseGuestMemory,
    values: &[String],
    vector_label: &str,
    string_label: &str,
) -> Result<GuestAddress, GuestError> {
    let pointer_bytes = memory.pointer_bytes();
    let vector = memory.allocate(&GuestAllocationOptions {
        byte_length: (values.len() + 1) * pointer_bytes,
        alignment: 16,
        permissions: GuestPermissions::ReadWrite,
        label: vector_label.to_string(),
    })?;
    for (index, value) in values.iter().enumerate() {
        let bytes = string_bytes(value, false);
        let address = memory.allocate(&GuestAllocationOptions {
            byte_length: bytes.len(),
            alignment: 16,
            permissions: GuestPermissions::ReadWrite,
            label: string_label.to_string(),
        })?;
        memory.write(address, &bytes)?;
        memory.write_pointer(memory.offset(vector, (index * pointer_bytes) as i64)?, Some(address))?;
    }
    Ok(vector)
}
