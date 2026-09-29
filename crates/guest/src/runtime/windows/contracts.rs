//! Windows runtime contracts: capabilities, coverage, shared state.
//!
//! Donor: `src/guest/runtime/windows/contracts.ts`.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use crate::abi::runner::{GuestCallFailure, GuestCallRequest};
use crate::core::callbacks::{GuestHostCallback, HookState, HostCallContext, HostCallbackFn};
use crate::core::contracts::{
    CallbackId, GuestAddress, GuestCallContext, GuestCallResult, GuestCallSignature,
    GuestCallValue, GuestCallbackReference, GuestExportTarget, GuestStorage, GuestSymbolName,
    GuestValueLayout, NativeCallAbi,
};
use crate::core::memory::SparseGuestMemory;
use crate::error::GuestError;
use crate::pe::image::PeImage;
use crate::runtime::common::memory::{allocate_native_memory, native_allocation_bytes};

/// Guest file opened through host capabilities.
pub trait WindowsFile {
    /// Read up to `length` bytes at `offset`.
    fn read(&mut self, offset: usize, length: usize) -> Vec<u8>;
    /// Write `bytes` at `offset`; returns bytes accepted.
    fn write(&mut self, offset: usize, bytes: &[u8]) -> usize;
    /// Current file size.
    fn size(&mut self) -> usize;
    /// Truncate to `length` bytes.
    fn truncate(&mut self, length: usize);
    /// Flush buffered output.
    fn flush(&mut self);
    /// Close the file.
    fn close(&mut self);
}

/// File-open options for the `open_file` capability.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WindowsOpenOptions {
    /// Read access requested.
    pub read: bool,
    /// Write access requested.
    pub write: bool,
    /// Creation disposition.
    pub creation: u32,
}

/// Standard stream selected by a write.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowsStream {
    /// Standard output.
    Stdout,
    /// Standard error.
    Stderr,
}

/// Host capabilities supplied to the Windows guest.
#[derive(Clone, Default)]
pub struct WindowsCapabilities {
    /// Clock: milliseconds since the epoch.
    pub now_milliseconds: Option<Rc<dyn Fn() -> i64>>,
    /// Performance counter value.
    pub performance_counter: Option<Rc<dyn Fn() -> u64>>,
    /// Performance counter frequency.
    pub performance_frequency: Option<u64>,
    /// Process command line.
    pub command_line: Option<String>,
    /// Process environment.
    pub environment: Option<HashMap<String, String>>,
    /// File opener.
    pub open_file: Option<Rc<dyn Fn(&str, WindowsOpenOptions) -> Option<Box<dyn WindowsFile>>>>,
    /// Standard output sink.
    pub standard_output: Option<Rc<dyn Fn(WindowsStream, &[u8])>>,
    /// Standard input source: up to `length` bytes.
    pub standard_input: Option<Rc<dyn Fn(usize) -> Vec<u8>>>,
}

impl std::fmt::Debug for WindowsCapabilities {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WindowsCapabilities")
            .field("command_line", &self.command_line)
            .finish_non_exhaustive()
    }
}

/// Windows runtime construction options.
#[derive(Debug, Clone, Default)]
pub struct WindowsRuntimeOptions {
    /// Host capabilities.
    pub capabilities: WindowsCapabilities,
    /// Process id.
    pub process_id: Option<u32>,
    /// Thread id.
    pub thread_id: Option<u32>,
}

/// Coverage record of one Windows import.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WindowsImportCoverage {
    /// Providing library.
    pub library: String,
    /// Import name.
    pub name: String,
    /// Whether the import is implemented.
    pub supported: bool,
    /// Calls observed.
    pub reached: u64,
    /// Calls failed.
    pub failed: u64,
    /// Last failure message.
    pub last_failure: Option<String>,
}

/// Initialize options.
#[derive(Debug, Clone)]
pub struct WindowsInitializeOptions {
    /// Calling context.
    pub context: GuestCallContext,
    /// Instruction budget per lifecycle call.
    pub instruction_budget: u64,
}

/// Unsupported-import failure, mirroring the donor message.
pub fn unsupported_windows(
    library: &str,
    symbol: &str,
    detail: impl Into<String>,
) -> GuestError {
    GuestError::unsupported(format!(
        "Unsupported Windows guest import {library}!{symbol}: {}",
        detail.into()
    ))
}

/// Canonical library key: lowercase basename with a `.dll` suffix.
pub fn canonical_library(library: &str) -> String {
    let base = library.replace('\\', "/").rsplit('/').next().unwrap_or(library).to_string();
    let lower = base.to_lowercase();
    if lower.ends_with(".dll") {
        lower
    } else {
        format!("{lower}.dll")
    }
}

/// One registered Windows import.
#[derive(Debug, Clone)]
pub struct WindowsImportEntry {
    /// Canonical library.
    pub library: String,
    /// Import name.
    pub name: String,
    /// Callback identity.
    pub callback: CallbackId,
    /// Trap address.
    pub address: GuestAddress,
    /// Whether the import is implemented.
    pub supported: bool,
    /// Calls observed.
    pub reached: u64,
    /// Calls failed.
    pub failed: u64,
    /// Last failure message.
    pub last_failure: Option<String>,
}

/// One heap allocation tracked by address.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WindowsHeapAllocation {
    /// Allocation address.
    pub address: GuestAddress,
    /// Allocation size.
    pub size: usize,
    /// Owning heap key.
    pub heap: u64,
}

/// Mutable state shared between the runtime and its host closures.
#[derive(Debug, Default)]
pub struct WindowsShared {
    /// Host capabilities.
    pub capabilities: WindowsCapabilities,
    /// Registered imports by `library!name` key.
    pub imports: HashMap<String, WindowsImportEntry>,
    /// Registered data exports by `library!name` key.
    pub data: HashMap<String, GuestAddress>,
    /// Imports requested by images.
    pub requested: HashSet<String>,
    /// Library handles by canonical name.
    pub libraries: HashMap<String, GuestAddress>,
    /// Prepared guest images.
    pub images: Vec<PeImage>,
    /// Library reference counts by handle offset.
    pub library_references: HashMap<u64, usize>,
    /// Live heap allocations by offset.
    pub allocations: HashMap<u64, WindowsHeapAllocation>,
    /// Valid CFG indirect targets by offset.
    pub cfg_targets: HashSet<u64>,
    /// Instruction budget for nested lifecycle calls.
    pub budget: u64,
}

/// Shared handle to [`WindowsShared`].
pub type SharedWindows = Rc<RefCell<WindowsShared>>;

/// Call signature for `parameters` returning `result`.
pub fn windows_signature(
    pointer_bytes: usize,
    library: &str,
    parameters: &[GuestStorage],
    result: Option<GuestStorage>,
) -> GuestCallSignature {
    let abi = if pointer_bytes == 8 {
        NativeCallAbi::MicrosoftX64
    } else if canonical_library(library) == "kernel32.dll" {
        NativeCallAbi::Stdcall
    } else {
        NativeCallAbi::Cdecl
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
    shared: &SharedWindows,
    pointer_bytes: usize,
    context: &GuestCallContext,
    target: GuestAddress,
    parameters: &[GuestStorage],
    result: Option<GuestStorage>,
    args: Vec<GuestCallValue>,
    system: bool,
) -> Result<GuestCallResult, GuestError> {
    let library = if system { "kernel32.dll" } else { "ucrtbase.dll" };
    let signature = windows_signature(pointer_bytes, library, parameters, result);
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
            GuestError::callback(format!("Nested Windows guest call stopped: {stop:?}"))
        }
    })
}

/// Read the thread `last-error` value from the TEB.
pub fn last_error(
    memory: &mut SparseGuestMemory,
    teb: GuestAddress,
) -> Result<u32, GuestError> {
    let offset = if memory.pointer_bytes() == 4 { 0x34 } else { 0x68 };
    memory.read_u32(memory.offset(teb, offset)?)
}

/// Write the thread `last-error` value to the TEB.
pub fn set_last_error(
    memory: &mut SparseGuestMemory,
    teb: GuestAddress,
    value: u32,
) -> Result<(), GuestError> {
    let offset = if memory.pointer_bytes() == 4 { 0x34 } else { 0x68 };
    memory.write_u32(memory.offset(teb, offset)?, value)
}

/// Import key: `library!name`.
pub fn import_key(library: &str, name: &str) -> String {
    format!("{}!{name}", canonical_library(library))
}

/// Host clock in milliseconds since the epoch.
pub fn now_millis(shared: &SharedWindows) -> i64 {
    if let Some(now) = &shared.borrow().capabilities.now_milliseconds {
        return now();
    }
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or(0)
}

/// Memory-independent service context.
#[derive(Debug, Clone)]
pub struct WindowsContext {
    /// Shared callback hooks.
    pub hooks: Rc<HookState>,
    /// Shared runtime state.
    pub shared: SharedWindows,
    /// Pointer width in bytes.
    pub pointer_bytes: usize,
}

impl WindowsContext {
    /// Pointer-sized unsigned storage.
    pub fn pointer_storage(&self) -> GuestStorage {
        if self.pointer_bytes == 4 {
            GuestStorage::Uint32
        } else {
            GuestStorage::Uint64
        }
    }

    /// Call signature for `library`.
    pub fn signature(
        &self,
        library: &str,
        parameters: &[GuestStorage],
        result: Option<GuestStorage>,
    ) -> GuestCallSignature {
        windows_signature(self.pointer_bytes, library, parameters, result)
    }

    /// Register `invoke` with an explicit signature.
    pub fn register(
        &self,
        memory: &mut SparseGuestMemory,
        library: &str,
        name: &str,
        signature: GuestCallSignature,
        invoke: HostCallbackFn,
    ) -> Result<(), GuestError> {
        self.register_inner(memory, library, name, signature, invoke, true)?;
        Ok(())
    }

    /// Register a lazy unsupported trap with an explicit signature.
    pub fn register_unsupported(
        &self,
        memory: &mut SparseGuestMemory,
        library: &str,
        name: &str,
        signature: GuestCallSignature,
        invoke: HostCallbackFn,
    ) -> Result<(), GuestError> {
        self.register_inner(memory, library, name, signature, invoke, false)?;
        Ok(())
    }

    /// Register `invoke` with a conventional signature.
    pub fn service(
        &self,
        memory: &mut SparseGuestMemory,
        library: &str,
        name: &str,
        parameters: &[GuestStorage],
        result: Option<GuestStorage>,
        invoke: HostCallbackFn,
    ) -> Result<(), GuestError> {
        let signature = self.signature(library, parameters, result);
        self.register(memory, library, name, signature, invoke)
    }

    /// Register guest data bytes; returns their address.
    pub fn register_data(
        &self,
        memory: &mut SparseGuestMemory,
        library: &str,
        name: &str,
        bytes: &[u8],
    ) -> Result<GuestAddress, GuestError> {
        let key = import_key(library, name);
        {
            let shared = self.shared.borrow();
            if shared.data.contains_key(&key) || shared.imports.contains_key(&key) {
                return Err(GuestError::callback(format!(
                    "Duplicate Windows data export {key}"
                )));
            }
        }
        let address = memory.allocate(&crate::core::contracts::GuestAllocationOptions {
            byte_length: bytes.len(),
            alignment: 16,
            permissions: crate::core::contracts::GuestPermissions::ReadWrite,
            label: key.clone(),
        })?;
        memory.write(address, bytes)?;
        let mut shared = self.shared.borrow_mut();
        shared.data.insert(key, address);
        let normalized = canonical_library(library);
        if !shared.libraries.contains_key(&normalized) {
            let handle = memory.allocate(&crate::core::contracts::GuestAllocationOptions {
                byte_length: 16,
                alignment: 16,
                permissions: crate::core::contracts::GuestPermissions::ReadWrite,
                label: format!("Windows library handle {normalized}"),
            })?;
            shared.libraries.insert(normalized, handle);
        }
        Ok(address)
    }

    fn register_inner(
        &self,
        memory: &mut SparseGuestMemory,
        library: &str,
        name: &str,
        signature: GuestCallSignature,
        invoke: HostCallbackFn,
        supported: bool,
    ) -> Result<WindowsImportEntry, GuestError> {
        let normalized = canonical_library(library);
        let key = format!("{normalized}!{name}");
        if self.shared.borrow().imports.contains_key(&key) {
            return Err(GuestError::callback(format!(
                "Windows import already registered: {key}"
            )));
        }
        let entry_key = key.clone();
        let shared = Rc::clone(&self.shared);
        let callback = GuestHostCallback {
            id: CallbackId::new("windows", &key),
            signature,
            invoke: Rc::new(move |ctx, context, args| {
                {
                    let mut shared = shared.borrow_mut();
                    if let Some(entry) = shared.imports.get_mut(&entry_key) {
                        entry.reached += 1;
                    }
                }
                match invoke(ctx, context, args) {
                    Ok(result) => Ok(result),
                    Err(error) => {
                        let mut shared = shared.borrow_mut();
                        if let Some(entry) = shared.imports.get_mut(&entry_key) {
                            entry.failed += 1;
                            entry.last_failure = Some(error.to_string());
                        }
                        Err(error)
                    }
                }
            }),
        };
        let address = self.hooks.callbacks.borrow_mut().bind(memory, callback.clone())?;
        let entry = WindowsImportEntry {
            library: normalized.clone(),
            name: name.to_string(),
            callback: callback.id.clone(),
            address,
            supported,
            reached: 0,
            failed: 0,
            last_failure: None,
        };
        {
            let mut shared = self.shared.borrow_mut();
            shared.imports.insert(key, entry.clone());
            if !shared.libraries.contains_key(&normalized) {
                let handle = memory.allocate(&crate::core::contracts::GuestAllocationOptions {
                    byte_length: 16,
                    alignment: 16,
                    permissions: crate::core::contracts::GuestPermissions::ReadWrite,
                    label: format!("Windows library handle {normalized}"),
                })?;
                shared.libraries.insert(normalized, handle);
            }
        }
        Ok(entry)
    }

    /// Resolve an export through images, imports, and data, following
    /// forwarders.
    pub fn resolve_address(&self, library: &str, name: &str) -> Option<GuestAddress> {
        let mut library = canonical_library(library);
        let mut name = name.to_string();
        let mut seen = HashSet::new();
        for _ in 0..128 {
            let key = format!("{library}!{name}");
            if !seen.insert(key.clone()) {
                return None;
            }
            let shared = self.shared.borrow();
            let image = shared.images.iter().find(|image| {
                canonical_library(&image.image.module.artifact_path) == library
            });
            let Some(image) = image else {
                let entry = shared.imports.get(&key);
                if entry.is_some_and(|entry| entry.supported) {
                    return entry.map(|entry| entry.address);
                }
                return shared.data.get(&key).copied();
            };
            let exported = image.image.exports.iter().find(|export| match &export.symbol {
                GuestSymbolName::Name { name: candidate, .. } => *candidate == name,
                GuestSymbolName::Ordinal(ordinal) => format!("#{ordinal}") == name,
            })?;
            match &exported.target {
                GuestExportTarget::Address(address) => return Some(*address),
                GuestExportTarget::Forward { library: next, symbol } => {
                    library = canonical_library(next);
                    name = match symbol {
                        GuestSymbolName::Name { name, .. } => name.clone(),
                        GuestSymbolName::Ordinal(ordinal) => format!("#{ordinal}"),
                    };
                }
            }
        }
        None
    }

    /// Handle of a loaded library or service-only module, if any.
    pub fn library_handle(&self, library: &str) -> Option<GuestAddress> {
        let normalized = canonical_library(library);
        let shared = self.shared.borrow();
        if let Some(image) = shared
            .images
            .iter()
            .find(|image| canonical_library(&image.image.module.artifact_path) == normalized)
        {
            return Some(image.image.base);
        }
        let supported = shared.imports.values().any(|entry| entry.library == normalized && entry.supported)
            || shared.data.keys().any(|key| key.starts_with(&format!("{normalized}!")));
        if supported {
            shared.libraries.get(&normalized).copied()
        } else {
            None
        }
    }

    /// Primary module name of a handle, if any.
    pub fn library_name(&self, handle: GuestAddress) -> Option<String> {
        let shared = self.shared.borrow();
        if let Some((name, _)) = shared
            .libraries
            .iter()
            .find(|(_, address)| address.offset == handle.offset)
        {
            return Some(name.clone());
        }
        shared
            .images
            .iter()
            .find(|image| image.image.base.offset == handle.offset)
            .map(|image| image.image.module.artifact_path.clone())
    }

    /// Allocate heap memory; null (with `last-error` 8) on failure.
    pub fn allocate(
        &self,
        memory: &mut SparseGuestMemory,
        teb: GuestAddress,
        size: usize,
        heap: u64,
    ) -> Result<Option<GuestAddress>, GuestError> {
        if size > 0x1000_0000 {
            set_last_error(memory, teb, 8)?;
            return Ok(None);
        }
        let address = allocate_native_memory(memory, size, "Windows guest heap")?;
        self.shared.borrow_mut().allocations.insert(
            address.offset,
            WindowsHeapAllocation {
                address,
                size: size.max(1),
                heap,
            },
        );
        Ok(Some(address))
    }

    /// Release a heap allocation; null is a no-op success.
    pub fn free(
        &self,
        memory: &mut SparseGuestMemory,
        teb: GuestAddress,
        address: Option<GuestAddress>,
        heap: u64,
    ) -> Result<bool, GuestError> {
        let Some(address) = address else {
            return Ok(true);
        };
        let allocation = self.shared.borrow().allocations.get(&address.offset).copied();
        match allocation {
            Some(allocation) if allocation.heap == heap => {
                memory.unmap(allocation.address, native_allocation_bytes(allocation.size)?)?;
                self.shared.borrow_mut().allocations.remove(&address.offset);
                Ok(true)
            }
            _ => {
                set_last_error(memory, teb, 87)?;
                Ok(false)
            }
        }
    }

    /// Live allocation size, if the heap matches.
    pub fn allocation_size(&self, address: GuestAddress, heap: u64) -> Option<usize> {
        self.shared
            .borrow()
            .allocations
            .get(&address.offset)
            .filter(|entry| entry.heap == heap)
            .map(|entry| entry.size)
    }

    /// Add a library reference; null (with `last-error` 126) when unknown.
    pub fn load_library(
        &self,
        memory: &mut SparseGuestMemory,
        teb: GuestAddress,
        library: &str,
    ) -> Result<Option<GuestAddress>, GuestError> {
        let handle = self.library_handle(library);
        let Some(handle) = handle else {
            set_last_error(memory, teb, 126)?;
            return Ok(None);
        };
        let mut shared = self.shared.borrow_mut();
        *shared.library_references.entry(handle.offset).or_insert(0) += 1;
        Ok(Some(handle))
    }

    /// Release a library reference.
    pub fn free_library(
        &self,
        memory: &mut SparseGuestMemory,
        teb: GuestAddress,
        handle: GuestAddress,
    ) -> Result<bool, GuestError> {
        let count = self.shared.borrow().library_references.get(&handle.offset).copied().unwrap_or(0);
        if count == 0 {
            set_last_error(memory, teb, 6)?;
            return Ok(false);
        }
        let mut shared = self.shared.borrow_mut();
        if count == 1 {
            shared.library_references.remove(&handle.offset);
        } else {
            shared.library_references.insert(handle.offset, count - 1);
        }
        // Prepared images and built-in services retain their owner's initial
        // reference.
        Ok(true)
    }

    /// Release every allocation owned by `heap`.
    pub fn destroy_heap(
        &self,
        memory: &mut SparseGuestMemory,
        teb: GuestAddress,
        heap: u64,
    ) -> Result<(), GuestError> {
        let owned: Vec<GuestAddress> = self
            .shared
            .borrow()
            .allocations
            .values()
            .filter(|entry| entry.heap == heap)
            .map(|entry| entry.address)
            .collect();
        for address in owned {
            self.free(memory, teb, Some(address), heap)?;
        }
        Ok(())
    }
}

/// Construction-time service registrar over borrowed guest memory.
pub struct WindowsServiceRegistrar<'m> {
    /// Guest memory under construction.
    pub memory: &'m mut SparseGuestMemory,
    /// Shared callback hooks.
    pub hooks: Rc<HookState>,
    /// Shared runtime state.
    pub shared: SharedWindows,
    /// Pointer width in bytes.
    pub pointer_bytes: usize,
    /// Clonable context with the same hooks, state, and pointer width.
    pub context: WindowsContext,
    /// Thread environment block.
    pub teb: GuestAddress,
    /// Process id.
    pub process_id: u32,
    /// Thread id.
    pub thread_id: u32,
}

impl<'m> WindowsServiceRegistrar<'m> {
    /// Build a registrar; the context shares the hooks and state.
    pub fn new(
        memory: &'m mut SparseGuestMemory,
        hooks: Rc<HookState>,
        shared: SharedWindows,
        pointer_bytes: usize,
        teb: GuestAddress,
        process_id: u32,
        thread_id: u32,
    ) -> Self {
        let context = WindowsContext {
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
            teb,
            process_id,
            thread_id,
        }
    }

    /// Pointer-sized unsigned storage.
    pub fn pointer_storage(&self) -> GuestStorage {
        self.context.pointer_storage()
    }

    /// Call signature for `library`.
    pub fn signature(
        &self,
        library: &str,
        parameters: &[GuestStorage],
        result: Option<GuestStorage>,
    ) -> GuestCallSignature {
        self.context.signature(library, parameters, result)
    }

    /// Register `invoke` with an explicit signature.
    pub fn register(
        &mut self,
        library: &str,
        name: &str,
        signature: GuestCallSignature,
        invoke: HostCallbackFn,
    ) -> Result<(), GuestError> {
        self.context.register(self.memory, library, name, signature, invoke)
    }

    /// Register `invoke` with a conventional signature.
    pub fn service(
        &mut self,
        library: &str,
        name: &str,
        parameters: &[GuestStorage],
        result: Option<GuestStorage>,
        invoke: HostCallbackFn,
    ) -> Result<(), GuestError> {
        self.context.service(self.memory, library, name, parameters, result, invoke)
    }

    /// Register guest data bytes; returns their address.
    pub fn register_data(
        &mut self,
        library: &str,
        name: &str,
        bytes: &[u8],
    ) -> Result<GuestAddress, GuestError> {
        self.context.register_data(self.memory, library, name, bytes)
    }

    /// Resolve an export through images, imports, and data.
    pub fn resolve_address(&self, library: &str, name: &str) -> Option<GuestAddress> {
        self.context.resolve_address(library, name)
    }

    /// Handle of a loaded library or service-only module, if any.
    pub fn library_handle(&self, library: &str) -> Option<GuestAddress> {
        self.context.library_handle(library)
    }

    /// Primary module name of a handle, if any.
    pub fn library_name(&self, handle: GuestAddress) -> Option<String> {
        self.context.library_name(handle)
    }

    /// Current thread `last-error` value.
    pub fn last_error(&mut self) -> Result<u32, GuestError> {
        last_error(self.memory, self.teb)
    }

    /// Assign the thread `last-error` value.
    pub fn set_last_error(&mut self, value: u32) -> Result<(), GuestError> {
        set_last_error(self.memory, self.teb, value)
    }

    /// Allocate heap memory; null on failure.
    pub fn allocate(&mut self, size: usize, heap: u64) -> Result<Option<GuestAddress>, GuestError> {
        self.context.allocate(self.memory, self.teb, size, heap)
    }

    /// Release a heap allocation.
    pub fn free(&mut self, address: Option<GuestAddress>, heap: u64) -> Result<bool, GuestError> {
        self.context.free(self.memory, self.teb, address, heap)
    }

    /// Live allocation size, if the heap matches.
    pub fn allocation_size(&self, address: GuestAddress, heap: u64) -> Option<usize> {
        self.context.allocation_size(address, heap)
    }

    /// Add a library reference; null (with `last-error` 126) when unknown.
    pub fn load_library(&mut self, library: &str) -> Result<Option<GuestAddress>, GuestError> {
        self.context.load_library(self.memory, self.teb, library)
    }

    /// Release a library reference.
    pub fn free_library(&mut self, handle: GuestAddress) -> Result<bool, GuestError> {
        self.context.free_library(self.memory, self.teb, handle)
    }
}
