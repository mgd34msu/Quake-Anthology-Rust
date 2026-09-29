//! System V runtime contracts: capabilities, coverage, shared state.
//!
//! Donor: `src/guest/runtime/system-v/contracts.ts`. Host closures share
//! the [`SharedSystemV`] state; guest memory reaches them through dispatch.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use crate::abi::runner::{GuestCallFailure, GuestCallRequest};
use crate::core::callbacks::{GuestHostCallback, HookState, HostCallContext, HostCallbackFn};
use crate::core::contracts::{
    CallbackId, GuestAccess, GuestAddress, GuestCallContext, GuestCallResult, GuestCallSignature,
    GuestCallValue, GuestCallbackReference, GuestImportResolution, GuestStorage, GuestValueLayout,
    ModuleIdentity, NativeCallAbi,
};
use crate::core::memory::SparseGuestMemory;
use crate::error::GuestError;
use crate::runtime::common::memory::{allocate_bytes, read_unsigned};

/// Standard output stream selected by a write.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SystemVStream {
    /// Standard output.
    Stdout,
    /// Standard error.
    Stderr,
}

/// Host capabilities supplied to the System V guest.
#[derive(Clone, Default)]
pub struct SystemVCapabilities {
    /// Deterministic clock: seconds since the epoch.
    pub now_seconds: Option<Rc<dyn Fn() -> i64>>,
    /// Standard input source: up to `maximum_bytes` bytes.
    pub standard_input: Option<Rc<dyn Fn(usize) -> Vec<u8>>>,
    /// Standard output sink: returns bytes accepted.
    pub standard_output: Option<Rc<dyn Fn(SystemVStream, &[u8]) -> usize>>,
    /// Standard stream flush: zero on success.
    pub standard_flush: Option<Rc<dyn Fn(SystemVStream) -> i32>>,
    /// Whether standard output is a terminal (line buffering).
    pub output_is_terminal: bool,
}

impl std::fmt::Debug for SystemVCapabilities {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SystemVCapabilities")
            .field("now_seconds", &self.now_seconds.is_some())
            .field("standard_input", &self.standard_input.is_some())
            .field("standard_output", &self.standard_output.is_some())
            .field("standard_flush", &self.standard_flush.is_some())
            .field("output_is_terminal", &self.output_is_terminal)
            .finish()
    }
}

/// System V runtime construction options.
#[derive(Debug, Clone, Default)]
pub struct SystemVRuntimeOptions {
    /// Host capabilities.
    pub capabilities: SystemVCapabilities,
    /// Guest argument vector.
    pub argv: Vec<String>,
    /// Guest environment vector.
    pub environment: Vec<String>,
}

/// Support level of one runtime symbol.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SystemVSupport {
    /// Implemented service or data.
    Implemented,
    /// Registered trap that always reports unsupported.
    UnsupportedFunction,
    /// Reserved unreadable data reservation.
    UnsupportedData,
}

/// Coverage record of one runtime symbol.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SystemVImportCoverage {
    /// Providing library.
    pub library: String,
    /// Symbol name.
    pub name: String,
    /// Symbol version, if any.
    pub version: Option<String>,
    /// Support level.
    pub support: SystemVSupport,
    /// Calls observed.
    pub reached: u64,
}

/// Initialize/finalize options.
#[derive(Debug, Clone)]
pub struct SystemVInitializeOptions {
    /// Calling context.
    pub context: GuestCallContext,
    /// Instruction budget per lifecycle call.
    pub instruction_budget: u64,
}

/// Unsupported-service failure, mirroring the donor message.
pub fn unsupported_system_v(
    library: &str,
    symbol: &str,
    version: Option<&str>,
    detail: impl Into<String>,
) -> GuestError {
    GuestError::unsupported(format!(
        "Unsupported System V guest service {library}:{symbol}{}: {}",
        version.map_or(String::new(), |version| format!("@{version}")),
        detail.into()
    ))
}

/// One heap allocation tracked by address.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SystemVHeapEntry {
    /// Allocation address.
    pub address: GuestAddress,
    /// Allocation size.
    pub size: usize,
}

/// One static TLS block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SystemVTlsBlock {
    /// Owning module.
    pub module: ModuleIdentity,
    /// Process-wide module id (begins at one).
    pub module_id: u64,
    /// Block address.
    pub address: GuestAddress,
    /// Block length.
    pub byte_length: usize,
}

/// One `__cxa_atexit` destructor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SystemVDestructor {
    /// Destructor address.
    pub target: GuestAddress,
    /// Destructor argument.
    pub argument: Option<GuestAddress>,
    /// Owning DSO handle, if any.
    pub dso: Option<GuestAddress>,
    /// Whether the destructor already ran.
    pub called: bool,
}

/// One registered runtime symbol.
#[derive(Debug, Clone)]
pub struct SystemVSymbolEntry {
    /// Coverage record.
    pub coverage: SystemVImportCoverage,
    /// Trap or data address.
    pub address: GuestAddress,
    /// Callback identity for host traps.
    pub callback: Option<CallbackId>,
    /// Data length for sized symbols.
    pub byte_length: Option<usize>,
}

/// Mutable state shared between the runtime and its host closures.
#[derive(Debug, Default)]
pub struct SystemVShared {
    /// Host capabilities.
    pub capabilities: SystemVCapabilities,
    /// Registered symbols by `library:name@version` key.
    pub symbols: HashMap<String, SystemVSymbolEntry>,
    /// Live heap allocations by offset.
    pub heap: HashMap<u64, SystemVHeapEntry>,
    /// Loaded static TLS blocks.
    pub tls_blocks: Vec<SystemVTlsBlock>,
    /// Static TLS bytes consumed below the thread pointer.
    pub tls_used: u64,
    /// Registered destructors.
    pub destructors: Vec<SystemVDestructor>,
    /// Instruction budget for nested lifecycle calls.
    pub budget: u64,
}

/// Shared handle to [`SystemVShared`].
pub type SharedSystemV = Rc<RefCell<SystemVShared>>;

/// Symbol key: `library:name@version`.
pub fn symbol_key(library: &str, name: &str, version: Option<&str>) -> String {
    format!("{library}:{name}@{}", version.unwrap_or(""))
}

/// Call signature for `parameters` returning `result`.
pub fn system_v_signature(
    pointer_bytes: usize,
    parameters: &[GuestStorage],
    result: Option<GuestStorage>,
) -> GuestCallSignature {
    let abi = if pointer_bytes == 8 {
        NativeCallAbi::SystemVX86_64
    } else {
        NativeCallAbi::SystemVI386
    };
    GuestCallSignature {
        abi,
        parameters: parameters
            .iter()
            .map(|storage| GuestValueLayout::Scalar(*storage))
            .collect(),
        result: result.map(GuestValueLayout::Scalar),
        variadic: false,
    }
}

/// Invoke a nested guest call from a host closure.
#[allow(clippy::too_many_arguments)]
pub fn invoke_nested(
    ctx: &mut HostCallContext<'_, '_>,
    shared: &SharedSystemV,
    pointer_bytes: usize,
    context: &GuestCallContext,
    target: GuestAddress,
    parameters: &[GuestStorage],
    result: Option<GuestStorage>,
    args: Vec<GuestCallValue>,
) -> Result<GuestCallResult, GuestError> {
    let signature = system_v_signature(pointer_bytes, parameters, result);
    let budget = shared.borrow().budget;
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
    ctx.invoke(&request).map_err(|failure| match failure {
        GuestCallFailure::Guest(error) => error,
        GuestCallFailure::Stopped { stop, .. } => {
            GuestError::callback(format!("Nested System V guest call stopped: {stop:?}"))
        }
    })
}

/// Resolve a `__tls_get_addr`-style address within a loaded TLS block.
pub fn tls_address(
    shared: &SharedSystemV,
    memory: &mut SparseGuestMemory,
    thread_pointer: GuestAddress,
    module_id: u64,
    offset: u64,
) -> Result<GuestAddress, GuestError> {
    let pointer_bytes = memory.pointer_bytes();
    let block = shared
        .borrow()
        .tls_blocks
        .iter()
        .find(|entry| entry.module_id == module_id)
        .cloned()
        .ok_or_else(|| {
            GuestError::invalid("System V TLS index outside loaded module block")
        })?;
    if offset >= block.byte_length as u64 {
        return Err(GuestError::invalid(
            "System V TLS index outside loaded module block",
        ));
    }
    let dtv = memory
        .read_pointer(memory.offset(thread_pointer, pointer_bytes as i64)?)?
        .ok_or_else(|| GuestError::callback("System V guest DTV has no requested module"))?;
    let generation = read_unsigned(
        memory,
        memory.offset(dtv, -((2 * pointer_bytes) as i64))?,
        pointer_bytes,
    )?;
    if module_id > generation {
        return Err(GuestError::callback(
            "System V guest DTV has no requested module",
        ));
    }
    let address = memory
        .read_pointer(memory.offset(dtv, (module_id * (2 * pointer_bytes as u64)) as i64)?)?
        .ok_or_else(|| GuestError::callback("System V guest DTV module is unallocated"))?;
    memory.offset(address, offset as i64)
}

/// Memory-independent service context: shared hooks, state, and pointer
/// width. Construction holds it inside the registrar; runtime closures
/// capture clones and receive guest memory through dispatch.
#[derive(Debug, Clone)]
pub struct SystemVContext {
    /// Shared callback hooks.
    pub hooks: Rc<HookState>,
    /// Shared runtime state.
    pub shared: SharedSystemV,
    /// Pointer width in bytes.
    pub pointer_bytes: usize,
}

impl SystemVContext {
    /// Pointer-sized unsigned storage.
    pub fn pointer_storage(&self) -> GuestStorage {
        if self.pointer_bytes == 4 {
            GuestStorage::Uint32
        } else {
            GuestStorage::Uint64
        }
    }

    /// Pointer-sized signed storage.
    pub fn signed_pointer_storage(&self) -> GuestStorage {
        if self.pointer_bytes == 4 {
            GuestStorage::Int32
        } else {
            GuestStorage::Int64
        }
    }

    /// Call signature for `parameters` returning `result`.
    pub fn signature(
        &self,
        parameters: &[GuestStorage],
        result: Option<GuestStorage>,
    ) -> GuestCallSignature {
        system_v_signature(self.pointer_bytes, parameters, result)
    }

    /// Register `invoke` under every listed version.
    pub fn service(
        &self,
        memory: &mut SparseGuestMemory,
        library: &str,
        name: &str,
        versions: &[Option<&str>],
        parameters: &[GuestStorage],
        result: Option<GuestStorage>,
        invoke: HostCallbackFn,
    ) -> Result<(), GuestError> {
        for version in versions {
            self.function(
                memory,
                library,
                name,
                *version,
                self.signature(parameters, result),
                Rc::clone(&invoke),
                SystemVSupport::Implemented,
            )?;
        }
        Ok(())
    }

    /// Register an always-unsupported trap; returns its address.
    pub fn unavailable(
        &self,
        memory: &mut SparseGuestMemory,
        library: &str,
        name: &str,
        parameters: &[GuestStorage],
        result: Option<GuestStorage>,
        detail: &str,
    ) -> Result<GuestAddress, GuestError> {
        let library_owned = library.to_string();
        let name_owned = name.to_string();
        let detail_owned = detail.to_string();
        let signature = self.signature(parameters, result);
        let entry = self.function(
            memory,
            library,
            name,
            None,
            signature,
            Rc::new(move |_, _, _| {
                Err(unsupported_system_v(
                    &library_owned,
                    &name_owned,
                    None,
                    detail_owned.clone(),
                ))
            }),
            SystemVSupport::UnsupportedFunction,
        )?;
        Ok(entry.address)
    }

    /// Register guest data at `address`.
    pub fn data(
        &self,
        memory: &mut SparseGuestMemory,
        library: &str,
        name: &str,
        version: Option<&str>,
        address: GuestAddress,
        byte_length: usize,
    ) -> Result<(), GuestError> {
        memory.check(address, byte_length, GuestAccess::Read)?;
        let id = symbol_key(library, name, version);
        let mut shared = self.shared.borrow_mut();
        if shared.symbols.contains_key(&id) {
            return Err(GuestError::callback(format!("Duplicate System V data {id}")));
        }
        shared.symbols.insert(
            id,
            SystemVSymbolEntry {
                coverage: SystemVImportCoverage {
                    library: library.to_string(),
                    name: name.to_string(),
                    version: version.map(str::to_string),
                    support: SystemVSupport::Implemented,
                    reached: 0,
                },
                address,
                callback: None,
                byte_length: Some(byte_length),
            },
        );
        Ok(())
    }

    /// Trap or data address of a registered symbol, if any.
    pub fn resolve_address(
        &self,
        library: &str,
        name: &str,
        version: Option<&str>,
    ) -> Option<GuestAddress> {
        self.shared
            .borrow()
            .symbols
            .get(&symbol_key(library, name, version))
            .map(|entry| entry.address)
    }

    /// Allocate heap memory tracked for `free`.
    pub fn allocate(
        &self,
        memory: &mut SparseGuestMemory,
        size: usize,
    ) -> Result<GuestAddress, GuestError> {
        if size > 0x1000_0000 {
            return Err(GuestError::invalid(
                "System V guest allocation exceeds supported size",
            ));
        }
        let address = allocate_bytes(memory, size.max(1), "System V guest heap")?;
        self.shared.borrow_mut().heap.insert(
            address.offset,
            SystemVHeapEntry {
                address,
                size: size.max(1),
            },
        );
        Ok(address)
    }

    /// Release a heap allocation; null is a no-op.
    pub fn free(
        &self,
        memory: &mut SparseGuestMemory,
        address: Option<GuestAddress>,
    ) -> Result<(), GuestError> {
        let Some(address) = address else {
            return Ok(());
        };
        memory.check(address, 0, GuestAccess::Write)?;
        let entry = self.shared.borrow_mut().heap.remove(&address.offset);
        let Some(entry) = entry else {
            return Err(GuestError::callback("System V free of a non-live allocation"));
        };
        memory.unmap(entry.address, entry.size)
    }

    /// Live allocation size, if any.
    pub fn allocation_size(
        &self,
        memory: &mut SparseGuestMemory,
        address: GuestAddress,
    ) -> Result<Option<usize>, GuestError> {
        memory.check(address, 0, GuestAccess::Write)?;
        Ok(self
            .shared
            .borrow()
            .heap
            .get(&address.offset)
            .map(|entry| entry.size))
    }

    /// Register one symbol version with coverage counting.
    fn function(
        &self,
        memory: &mut SparseGuestMemory,
        library: &str,
        name: &str,
        version: Option<&str>,
        signature: GuestCallSignature,
        invoke: HostCallbackFn,
        support: SystemVSupport,
    ) -> Result<SystemVSymbolEntry, GuestError> {
        let id = symbol_key(library, name, version);
        if self.shared.borrow().symbols.contains_key(&id) {
            return Err(GuestError::callback(format!(
                "Duplicate System V symbol {id}"
            )));
        }
        let key = id.clone();
        let shared = Rc::clone(&self.shared);
        let callback = GuestHostCallback {
            id: CallbackId::new("system-v", &id),
            signature,
            invoke: Rc::new(move |ctx, context, args| {
                if let Some(entry) = shared.borrow_mut().symbols.get_mut(&key) {
                    entry.coverage.reached += 1;
                }
                invoke(ctx, context, args)
            }),
        };
        let address = self.hooks.callbacks.borrow_mut().bind(memory, callback.clone())?;
        let entry = SystemVSymbolEntry {
            coverage: SystemVImportCoverage {
                library: library.to_string(),
                name: name.to_string(),
                version: version.map(str::to_string),
                support,
                reached: 0,
            },
            address,
            callback: Some(callback.id),
            byte_length: None,
        };
        self.shared.borrow_mut().symbols.insert(id, entry.clone());
        Ok(entry)
    }

    /// Resolve a registered symbol to its import resolution.
    pub fn resolution(
        &self,
        memory: &SparseGuestMemory,
        library: &str,
        name: &str,
        version: Option<&str>,
    ) -> Option<GuestImportResolution> {
        let entry = self
            .shared
            .borrow()
            .symbols
            .get(&symbol_key(library, name, version))
            .cloned()?;
        match entry.callback {
            Some(callback) => Some(GuestImportResolution::Host {
                address: entry.address,
                callback,
            }),
            None => Some(GuestImportResolution::Guest {
                address: entry.address,
                module: memory.module().clone(),
            }),
        }
    }
}

/// Construction-time service registrar over borrowed guest memory.
pub struct SystemVServiceRegistrar<'m> {
    /// Guest memory under construction.
    pub memory: &'m mut SparseGuestMemory,
    /// Shared callback hooks.
    pub hooks: Rc<HookState>,
    /// Shared runtime state.
    pub shared: SharedSystemV,
    /// Pointer width in bytes.
    pub pointer_bytes: usize,
    /// Clonable context with the same hooks, state, and pointer width.
    pub context: SystemVContext,
    /// Thread `errno` slot.
    pub errno_address: GuestAddress,
    /// Thread pointer.
    pub thread_pointer: GuestAddress,
    /// Shared empty-string representation.
    pub empty_string: GuestAddress,
}

impl<'m> SystemVServiceRegistrar<'m> {
    /// Build a registrar; the context shares the hooks and state.
    pub fn new(
        memory: &'m mut SparseGuestMemory,
        hooks: Rc<HookState>,
        shared: SharedSystemV,
        pointer_bytes: usize,
        errno_address: GuestAddress,
        thread_pointer: GuestAddress,
        empty_string: GuestAddress,
    ) -> Self {
        let context = SystemVContext {
            hooks: Rc::clone(&hooks),
            shared: Rc::clone(&shared),
            pointer_bytes,
        };
        Self {
            memory,
            hooks,
            shared,
            pointer_bytes,
            context,
            errno_address,
            thread_pointer,
            empty_string,
        }
    }

    /// Pointer-sized unsigned storage.
    pub fn pointer_storage(&self) -> GuestStorage {
        self.context.pointer_storage()
    }

    /// Pointer-sized signed storage.
    pub fn signed_pointer_storage(&self) -> GuestStorage {
        self.context.signed_pointer_storage()
    }

    /// Call signature for `parameters` returning `result`.
    pub fn signature(
        &self,
        parameters: &[GuestStorage],
        result: Option<GuestStorage>,
    ) -> GuestCallSignature {
        self.context.signature(parameters, result)
    }

    /// Current `errno`.
    pub fn errno(&mut self) -> Result<i32, GuestError> {
        self.memory.read_i32(self.errno_address)
    }

    /// Assign `errno`.
    pub fn set_errno(&mut self, value: i32) -> Result<(), GuestError> {
        self.memory.write_i32(self.errno_address, value)
    }

    /// Register `invoke` under every listed version.
    pub fn service(
        &mut self,
        library: &str,
        name: &str,
        versions: &[Option<&str>],
        parameters: &[GuestStorage],
        result: Option<GuestStorage>,
        invoke: HostCallbackFn,
    ) -> Result<(), GuestError> {
        self.context.service(
            self.memory,
            library,
            name,
            versions,
            parameters,
            result,
            invoke,
        )
    }

    /// Register an always-unsupported trap; returns its address.
    pub fn unavailable(
        &mut self,
        library: &str,
        name: &str,
        parameters: &[GuestStorage],
        result: Option<GuestStorage>,
        detail: &str,
    ) -> Result<GuestAddress, GuestError> {
        self.context.unavailable(self.memory, library, name, parameters, result, detail)
    }

    /// Register guest data at `address`.
    pub fn data(
        &mut self,
        library: &str,
        name: &str,
        version: Option<&str>,
        address: GuestAddress,
        byte_length: usize,
    ) -> Result<(), GuestError> {
        self.context.data(self.memory, library, name, version, address, byte_length)
    }

    /// Trap or data address of a registered symbol, if any.
    pub fn resolve_address(
        &self,
        library: &str,
        name: &str,
        version: Option<&str>,
    ) -> Option<GuestAddress> {
        self.context.resolve_address(library, name, version)
    }

    /// Allocate heap memory tracked for `free`.
    pub fn allocate(&mut self, size: usize) -> Result<GuestAddress, GuestError> {
        self.context.allocate(self.memory, size)
    }

    /// Release a heap allocation; null is a no-op.
    pub fn free(&mut self, address: Option<GuestAddress>) -> Result<(), GuestError> {
        self.context.free(self.memory, address)
    }

    /// Live allocation size, if any.
    pub fn allocation_size(&mut self, address: GuestAddress) -> Result<Option<usize>, GuestError> {
        self.context.allocation_size(self.memory, address)
    }

    /// Resolve a registered symbol to its import resolution.
    pub fn resolution(
        &self,
        library: &str,
        name: &str,
        version: Option<&str>,
    ) -> Option<GuestImportResolution> {
        self.context.resolution(self.memory, library, name, version)
    }
}
