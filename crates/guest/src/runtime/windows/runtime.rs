//! Windows guest process: image lifecycle, CFG, and import resolution.
//!
//! Donor: `src/guest/runtime/windows/runtime.ts`. One emulated Windows
//! thread, sharing the caller's CPU, memory, and callback table. Memory and
//! the call runner stay with the caller; every method takes them explicitly.

use std::cell::RefCell;
use std::rc::Rc;

use crate::abi::runner::{GuestCallFailure, GuestCallRequest, GuestCallRunner};
use crate::core::callbacks::HookState;
use crate::core::contracts::{
    GuestAccess, GuestAddress, GuestAllocationOptions, GuestCallbackReference, GuestCallContext,
    GuestCallResult, GuestCallSignature, GuestCallValue, GuestImage, GuestImport,
    GuestImportResolution, GuestImportResolver, GuestIntegerWidth, GuestPermissions, GuestRegister,
    GuestStorage, GuestSymbolName,
};
use crate::core::memory::SparseGuestMemory;
use crate::core::registers::GuestProcessorState;
use crate::error::GuestError;
use crate::pe::exports::resolve_pe_export;
use crate::pe::image::PeImage;
use crate::pe::loader::bind_pe_imports;
use crate::runtime::common::memory::{read_unsigned, write_pointer, write_unsigned};
use crate::runtime::windows::contracts::{
    canonical_library, import_key, last_error, set_last_error, unsupported_windows, SharedWindows,
    WindowsCapabilities, WindowsContext, WindowsImportCoverage, WindowsInitializeOptions,
    WindowsRuntimeOptions, WindowsServiceRegistrar, WindowsShared,
};
use crate::runtime::windows::crt::install_crt;
use crate::runtime::windows::kernel::install_kernel;
use crate::runtime::windows::msvc::streams::install_msvc_streams;

/// One emulated Windows thread.
pub struct WindowsGuestRuntime {
    context: WindowsContext,
    pointer_bytes: usize,
    address_space: u64,
    process_id: u32,
    thread_id: u32,
    teb: GuestAddress,
    peb: GuestAddress,
    static_tls: GuestAddress,
    prepared: Vec<PeImage>,
    initialized: Vec<PeImage>,
    tls_blocks: Vec<(PeImage, GuestAddress)>,
    cfg_check: Option<GuestAddress>,
    cfg_dispatch: Option<GuestAddress>,
}

impl WindowsGuestRuntime {
    /// Build a runtime over borrowed guest memory and shared hooks.
    pub fn new(
        hooks: Rc<HookState>,
        memory: &mut SparseGuestMemory,
        options: WindowsRuntimeOptions,
    ) -> Result<Self, GuestError> {
        let pointer_bytes = memory.pointer_bytes();
        let address_space = memory.address_space();
        let shared: SharedWindows = Rc::new(RefCell::new(WindowsShared {
            capabilities: options.capabilities,
            budget: 1_000_000,
            ..Default::default()
        }));
        let process_id = options.process_id.unwrap_or(1);
        let thread_id = options.thread_id.unwrap_or(1);
        let teb = memory.allocate(&GuestAllocationOptions {
            byte_length: 0x2000,
            alignment: 4096,
            permissions: GuestPermissions::ReadWrite,
            label: "Windows TEB".to_string(),
        })?;
        let peb = memory.allocate(&GuestAllocationOptions {
            byte_length: 0x1000,
            alignment: 4096,
            permissions: GuestPermissions::ReadWrite,
            label: "Windows PEB".to_string(),
        })?;
        let static_tls = memory.allocate(&GuestAllocationOptions {
            byte_length: pointer_bytes * 1024,
            alignment: 16,
            permissions: GuestPermissions::ReadWrite,
            label: "Windows static TLS vector".to_string(),
        })?;
        let width = pointer_bytes;
        write_pointer(memory, memory.offset(teb, if width == 4 { 0x18 } else { 0x30 })?, Some(teb))?;
        write_pointer(memory, memory.offset(teb, if width == 4 { 0x30 } else { 0x60 })?, Some(peb))?;
        write_pointer(
            memory,
            memory.offset(teb, if width == 4 { 0x2c } else { 0x58 })?,
            Some(static_tls),
        )?;
        write_unsigned(memory, teb, width, (1i128 << (width * 8)) - 1)?;
        write_unsigned(
            memory,
            memory.offset(teb, if width == 4 { 0x20 } else { 0x40 })?,
            width,
            i128::from(process_id),
        )?;
        write_unsigned(
            memory,
            memory.offset(teb, if width == 4 { 0x24 } else { 0x48 })?,
            width,
            i128::from(thread_id),
        )?;
        let context = WindowsContext {
            hooks: Rc::clone(&hooks),
            shared: Rc::clone(&shared),
            pointer_bytes,
        };
        {
            let mut registrar = WindowsServiceRegistrar::new(
                memory,
                hooks,
                Rc::clone(&shared),
                pointer_bytes,
                teb,
                process_id,
                thread_id,
            );
            install_kernel(&mut registrar)?;
            install_crt(&mut registrar)?;
            install_msvc_streams(&mut registrar)?;
        }
        // Microsoft STL xlocale: locale::id contains size_t _Id; its shared
        // counter is int.
        context.register_data(memory, "msvcp140.dll", "?_Id_cnt@id@locale@std@@0HA", &[0u8; 4])?;
        context.register_data(
            memory,
            "msvcp140.dll",
            "?id@?$numpunct@D@std@@2V0locale@2@A",
            &vec![0u8; width],
        )?;
        Ok(Self {
            context,
            pointer_bytes,
            address_space,
            process_id,
            thread_id,
            teb,
            peb,
            static_tls,
            prepared: Vec::new(),
            initialized: Vec::new(),
            tls_blocks: Vec::new(),
            cfg_check: None,
            cfg_dispatch: None,
        })
    }

    /// Pointer width in bytes.
    #[must_use]
    pub const fn pointer_bytes(&self) -> usize {
        self.pointer_bytes
    }

    /// Process id.
    #[must_use]
    pub const fn process_id(&self) -> u32 {
        self.process_id
    }

    /// Thread id.
    #[must_use]
    pub const fn thread_id(&self) -> u32 {
        self.thread_id
    }

    /// Thread environment block.
    #[must_use]
    pub const fn teb(&self) -> GuestAddress {
        self.teb
    }

    /// Process environment block.
    #[must_use]
    pub const fn peb(&self) -> GuestAddress {
        self.peb
    }

    /// Static TLS vector.
    #[must_use]
    pub const fn static_tls(&self) -> GuestAddress {
        self.static_tls
    }

    /// Host capabilities.
    #[must_use]
    pub fn capabilities(&self) -> WindowsCapabilities {
        self.context.shared.borrow().capabilities.clone()
    }

    /// Prepared images.
    #[must_use]
    pub fn images(&self) -> Vec<PeImage> {
        self.context.shared.borrow().images.clone()
    }

    /// Current thread `last-error` value.
    pub fn last_error(&self, memory: &mut SparseGuestMemory) -> Result<u32, GuestError> {
        last_error(memory, self.teb)
    }

    /// Assign the thread `last-error` value.
    pub fn set_last_error(&self, memory: &mut SparseGuestMemory, value: u32) -> Result<(), GuestError> {
        set_last_error(memory, self.teb, value)
    }

    /// Live heap allocation size, if the heap matches.
    #[must_use]
    pub fn allocation_size(&self, address: GuestAddress, heap: u64) -> Option<usize> {
        self.context.allocation_size(address, heap)
    }

    /// Handle of a loaded library or service-only module, if any.
    #[must_use]
    pub fn library_handle(&self, library: &str) -> Option<GuestAddress> {
        self.context.library_handle(library)
    }

    /// Add a library reference; null (with `last-error` 126) when unknown.
    pub fn load_library(
        &self,
        memory: &mut SparseGuestMemory,
        library: &str,
    ) -> Result<Option<GuestAddress>, GuestError> {
        self.context.load_library(memory, self.teb, library)
    }

    /// Release a library reference.
    pub fn free_library(
        &self,
        memory: &mut SparseGuestMemory,
        handle: GuestAddress,
    ) -> Result<bool, GuestError> {
        self.context.free_library(memory, self.teb, handle)
    }

    /// Resolve a service trap or prepared-image export, following forwarder
    /// chains.
    #[must_use]
    pub fn resolve_address(&self, library: &str, name: &str) -> Option<GuestAddress> {
        self.context.resolve_address(library, name)
    }

    /// Coverage of requested and reached imports.
    #[must_use]
    pub fn coverage(&self) -> Vec<WindowsImportCoverage> {
        let shared = self.context.shared.borrow();
        shared
            .imports
            .values()
            .filter(|entry| {
                shared.requested.contains(&format!("{}!{}", entry.library, entry.name))
                    || entry.reached > 0
            })
            .map(|entry| WindowsImportCoverage {
                library: entry.library.clone(),
                name: entry.name.clone(),
                supported: entry.supported,
                reached: entry.reached,
                failed: entry.failed,
                last_failure: entry.last_failure.clone(),
            })
            .collect()
    }

    /// Attach a runner: validate shared state, publish the stack range, and
    /// point the TEB segment at this thread.
    pub fn attach_runner(&self, runner: &mut GuestCallRunner<'_>) -> Result<(), GuestError> {
        if !Rc::ptr_eq(runner.hooks(), &self.context.hooks) {
            return Err(GuestError::invalid("Windows runner belongs to different guest state"));
        }
        let (state, memory) = runner.cpu_parts();
        if memory.address_space() != self.address_space {
            return Err(GuestError::invalid("Windows runner belongs to different guest state"));
        }
        let width = if self.pointer_bytes == 4 {
            GuestIntegerWidth::B32
        } else {
            GuestIntegerWidth::B64
        };
        let stack = state.registers.read(GuestRegister::Rsp, width, false)?;
        let mapping = memory
            .mappings()
            .into_iter()
            .find(|entry| stack > entry.base && stack <= entry.base + entry.byte_length as u64)
            .ok_or_else(|| GuestError::invalid("Windows thread stack must already be mapped"))?;
        write_unsigned(
            memory,
            memory.offset(self.teb, self.pointer_bytes as i64)?,
            self.pointer_bytes,
            i128::from(mapping.base + mapping.byte_length as u64),
        )?;
        write_unsigned(
            memory,
            memory.offset(self.teb, (self.pointer_bytes * 2) as i64)?,
            self.pointer_bytes,
            i128::from(mapping.base),
        )?;
        if self.pointer_bytes == 4 {
            state.segments[GuestProcessorState::FS].base = self.teb.offset;
        } else {
            state.segments[GuestProcessorState::GS].base = self.teb.offset;
        }
        Ok(())
    }

    /// Call signature for `library`.
    #[must_use]
    pub fn signature(
        &self,
        library: &str,
        parameters: &[GuestStorage],
        result: Option<GuestStorage>,
    ) -> GuestCallSignature {
        self.context.signature(library, parameters, result)
    }

    /// Invoke a nested guest call through an attached runner.
    #[allow(clippy::too_many_arguments)]
    pub fn invoke(
        &self,
        runner: &mut GuestCallRunner<'_>,
        context: &GuestCallContext,
        target: GuestAddress,
        parameters: &[GuestStorage],
        result: Option<GuestStorage>,
        args: Vec<GuestCallValue>,
        system: bool,
    ) -> Result<GuestCallResult, GuestError> {
        let library = if system { "kernel32.dll" } else { "ucrtbase.dll" };
        let signature = self.context.signature(library, parameters, result);
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
        map_nested(runner.invoke(&request))
    }

    /// Prepare an image: track, bind imports, carve TLS, patch CFG.
    pub fn prepare_image(
        &mut self,
        memory: &mut SparseGuestMemory,
        image: &PeImage,
    ) -> Result<(), GuestError> {
        if self.prepared.contains(image) {
            return Ok(());
        }
        if image.image.base.space != self.address_space {
            return Err(GuestError::invalid("Windows image belongs to another memory"));
        }
        if !self.context.shared.borrow().images.contains(image) {
            self.context.shared.borrow_mut().images.push(image.clone());
        }
        bind_pe_imports(image, memory, self)?;
        if let Some(tls) = &image.image.tls {
            let Some(index_address) = image.tls_index_address else {
                return Err(GuestError::invalid("Invalid Windows static TLS index"));
            };
            if self.tls_blocks.len() >= 1024 {
                return Err(GuestError::invalid("Invalid Windows static TLS index"));
            }
            let index = self.tls_blocks.len();
            let block = memory.allocate(&GuestAllocationOptions {
                byte_length: (tls.initialized.len() + tls.zero_fill_bytes).max(1),
                alignment: tls.alignment.max(16),
                permissions: GuestPermissions::ReadWrite,
                label: format!("{}: TLS", image.image.module.id.name),
            })?;
            memory.write(block, &tls.initialized)?;
            write_unsigned(memory, index_address, 4, index as i128)?;
            write_pointer(
                memory,
                memory.offset(self.static_tls, (index * self.pointer_bytes) as i64)?,
                Some(block),
            )?;
            self.tls_blocks.push((image.clone(), block));
        }
        // The CRT entry initializes its own cookie/complement. The guest
        // loader owns CFG dispatch.
        self.prepare_cfg(memory, image)?;
        self.prepared.push(image.clone());
        Ok(())
    }

    /// Initialize an image: TLS callbacks then the entry point with
    /// `DLL_PROCESS_ATTACH`.
    pub fn initialize(
        &mut self,
        runner: &mut GuestCallRunner<'_>,
        image: &PeImage,
        options: &WindowsInitializeOptions,
    ) -> Result<(), GuestError> {
        if self.initialized.contains(image) {
            return Ok(());
        }
        {
            let (_, memory) = runner.cpu_parts();
            self.prepare_image(memory, image)?;
        }
        self.context.shared.borrow_mut().budget = options.instruction_budget;
        let callbacks = image
            .image
            .tls
            .as_ref()
            .map(|tls| tls.callbacks.clone())
            .unwrap_or_default();
        for callback in callbacks {
            self.notification(runner, image, callback, 1, options, None)?;
        }
        if let Some(entry_point) = image.image.entry_point {
            let result = self.notification(runner, image, entry_point, 1, options, Some(GuestStorage::Int32))?;
            let GuestCallResult::Value(GuestCallValue::Int32(value)) = result else {
                return Err(attach_failed(image));
            };
            if value == 0 {
                return Err(attach_failed(image));
            }
        }
        self.initialized.push(image.clone());
        Ok(())
    }

    /// Detach an image: entry point then TLS callbacks with
    /// `DLL_PROCESS_DETACH`.
    pub fn detach(
        &mut self,
        runner: &mut GuestCallRunner<'_>,
        image: &PeImage,
        options: &WindowsInitializeOptions,
    ) -> Result<(), GuestError> {
        if !self.initialized.contains(image) {
            return Ok(());
        }
        self.context.shared.borrow_mut().budget = options.instruction_budget;
        if let Some(entry_point) = image.image.entry_point {
            self.notification(runner, image, entry_point, 0, options, Some(GuestStorage::Int32))?;
        }
        let callbacks = image
            .image
            .tls
            .as_ref()
            .map(|tls| tls.callbacks.clone())
            .unwrap_or_default();
        for callback in callbacks {
            self.notification(runner, image, callback, 0, options, None)?;
        }
        self.initialized.retain(|entry| entry != image);
        Ok(())
    }

    fn notification(
        &self,
        runner: &mut GuestCallRunner<'_>,
        image: &PeImage,
        target: GuestAddress,
        reason: u32,
        options: &WindowsInitializeOptions,
        result: Option<GuestStorage>,
    ) -> Result<GuestCallResult, GuestError> {
        let signature = self.context.signature(
            "kernel32.dll",
            &[GuestStorage::Pointer, GuestStorage::Uint32, GuestStorage::Pointer],
            result,
        );
        let request = GuestCallRequest {
            target,
            signature: signature.clone(),
            arguments: vec![
                GuestCallValue::Pointer(Some(image.image.base)),
                GuestCallValue::Uint32(reason),
                GuestCallValue::Pointer(None),
            ],
            context: GuestCallContext {
                module: options.context.module.clone(),
                callback: GuestCallbackReference::NativeGuest {
                    module: options.context.module.clone(),
                    address: target,
                    abi: signature.abi,
                },
                parent: Some(Box::new(options.context.clone())),
                itself: options.context.itself.clone(),
                other: options.context.other.clone(),
            },
            instruction_budget: options.instruction_budget,
        };
        map_nested(runner.invoke(&request))
    }

    fn prepare_cfg(
        &mut self,
        memory: &mut SparseGuestMemory,
        image: &PeImage,
    ) -> Result<(), GuestError> {
        let Some(config) = image.load_configuration.clone() else {
            return Ok(());
        };
        if image.pe.dll_characteristics & 0x4000 == 0 || config.guard_flags & 0x100 == 0 {
            return Ok(());
        }
        let width = self.pointer_bytes;
        let table_offset = if width == 4 { 80 } else { 128 };
        if config.bytes.len() < table_offset + width * 2 {
            return Err(GuestError::invalid("Truncated CFG load configuration"));
        }
        let table = read_unsigned(memory, memory.offset(config.address, table_offset as i64)?, width)?;
        let count =
            read_unsigned(memory, memory.offset(config.address, (table_offset + width) as i64)?, width)?;
        let extra = config.guard_flags >> 28;
        let stride = (extra + 4) as usize;
        if count > image.image.byte_length / stride as u64 {
            return Err(GuestError::invalid("Invalid CFG target count"));
        }
        let base = memory.pointer(table)?;
        if count != 0 && base.is_none() {
            return Err(GuestError::invalid("Null CFG target table"));
        }
        if let Some(base) = base {
            let entries = memory.copy(base, count as usize * stride)?;
            let mut previous: i64 = -1;
            for index in 0..count as usize {
                let start = index * stride;
                let rva = u32::from_le_bytes([
                    entries[start],
                    entries[start + 1],
                    entries[start + 2],
                    entries[start + 3],
                ]);
                let flags = if extra == 0 { 0 } else { entries[start + 4] };
                if rva as i64 <= previous
                    || u64::from(rva) >= image.image.byte_length
                    || flags & !3 != 0
                    || (stride > 5 && entries[start + 5..start + stride].iter().any(|byte| *byte != 0))
                {
                    return Err(GuestError::invalid("Invalid/unsupported CFG function metadata"));
                }
                previous = rva as i64;
                if flags & 1 != 0 || (flags & 2 != 0 && config.guard_flags & 0x8000 != 0) {
                    continue;
                }
                let address = memory.offset(image.image.base, i64::from(rva))?;
                memory.check(address, 1, GuestAccess::Execute)?;
                self.context.shared.borrow_mut().cfg_targets.insert(address.offset);
            }
        }
        if self.cfg_check.is_none() {
            let shared = Rc::clone(&self.context.shared);
            self.context.service(
                memory,
                "quake-runtime.dll",
                "guard-check",
                &[],
                None,
                Rc::new(move |ctx, context, _| {
                    let target = ctx.cpu_state().registers.read(
                        if width == 4 { GuestRegister::Rcx } else { GuestRegister::Rax },
                        if width == 4 { GuestIntegerWidth::B32 } else { GuestIntegerWidth::B64 },
                        false,
                    )?;
                    validate_cfg(ctx, &shared, target, context)?;
                    Ok(GuestCallResult::Void)
                }),
            )?;
            self.cfg_check = self.context.resolve_address("quake-runtime.dll", "guard-check");
            if width == 8 {
                let shared = Rc::clone(&self.context.shared);
                self.context.service(
                    memory,
                    "quake-runtime.dll",
                    "guard-dispatch-check",
                    &[GuestStorage::Pointer],
                    None,
                    Rc::new(move |ctx, context, args| {
                        let Some(GuestCallValue::Pointer(target)) = args.first() else {
                            return Err(GuestError::invalid("CFG dispatch target missing"));
                        };
                        let target = target.map(|address| address.offset).unwrap_or(0);
                        validate_cfg(ctx, &shared, target, context)?;
                        Ok(GuestCallResult::Void)
                    }),
                )?;
                let Some(check) = self.context.resolve_address("quake-runtime.dll", "guard-dispatch-check") else {
                    return Err(GuestError::callback("CFG checker binding missing"));
                };
                let mut code = vec![
                    0x50, 0x51, 0x52, 0x41, 0x50, 0x41, 0x51, 0x48, 0x83, 0xec, 0x20, 0x48, 0x89,
                    0xc1, 0x48, 0xb8,
                ];
                code.extend_from_slice(&check.offset.to_le_bytes());
                code.extend_from_slice(&[
                    0xff, 0xd0, 0x48, 0x83, 0xc4, 0x20, 0x41, 0x59, 0x41, 0x58, 0x5a, 0x59, 0x58,
                    0xff, 0xe0,
                ]);
                let address = memory.allocate(&GuestAllocationOptions {
                    byte_length: code.len(),
                    alignment: 16,
                    permissions: GuestPermissions::ReadWrite,
                    label: "Guest CFG dispatch instructions".to_string(),
                })?;
                memory.write(address, &code)?;
                memory.protect(address, code.len(), GuestPermissions::ReadExecute)?;
                self.cfg_dispatch = Some(address);
            } else {
                self.context.service(
                    memory,
                    "quake-runtime.dll",
                    "guard-dispatch-check",
                    &[],
                    None,
                    Rc::new(move |_, _, _| {
                        Err(unsupported_windows(
                            "quake-runtime.dll",
                            "guard-dispatch-check",
                            "i386 CFG dispatch convention is not implemented",
                        ))
                    }),
                )?;
                self.cfg_dispatch = self.context.resolve_address("quake-runtime.dll", "guard-dispatch-check");
            }
        }
        for (slot, value) in [
            (config.guard_check_slot, self.cfg_check),
            (config.guard_dispatch_slot, self.cfg_dispatch),
        ] {
            let (Some(slot), Some(value)) = (slot, value) else {
                continue;
            };
            let region = memory
                .mappings()
                .into_iter()
                .find(|mapping| {
                    slot.offset >= mapping.base
                        && slot.offset + width as u64 <= mapping.base + mapping.byte_length as u64
                })
                .ok_or_else(|| GuestError::invalid("Unmapped CFG pointer slot"))?;
            memory.protect(slot, width, GuestPermissions::ReadWrite)?;
            let outcome = write_pointer(memory, slot, Some(value));
            memory.protect(slot, width, region.permissions)?;
            outcome?;
        }
        Ok(())
    }
}

impl GuestImportResolver for WindowsGuestRuntime {
    fn resolve(
        &self,
        memory: &mut SparseGuestMemory,
        import: &GuestImport,
        requesting: &GuestImage,
    ) -> GuestImportResolution {
        let name = match &import.symbol {
            GuestSymbolName::Name { name, .. } => name.clone(),
            GuestSymbolName::Ordinal(ordinal) => format!("#{ordinal}"),
        };
        let library = canonical_library(&import.library);
        let key = import_key(&library, &name);
        self.context.shared.borrow_mut().requested.insert(key.clone());
        if let Some(data) = self.context.shared.borrow().data.get(&key).copied() {
            return GuestImportResolution::Guest {
                address: data,
                module: memory.module().clone(),
            };
        }
        let images = self.context.shared.borrow().images.clone();
        if let Some(guest) = images.iter().find(|image| {
            canonical_library(&image.image.module.artifact_path) == library
                && (image.image.base != requesting.base || image.image.module != requesting.module)
        }) {
            let lookup = |library: &str, _image: &PeImage| {
                images
                    .iter()
                    .find(|image| {
                        canonical_library(&image.image.module.artifact_path)
                            == canonical_library(library)
                    })
                    .cloned()
            };
            return match resolve_pe_export(guest, &import.symbol, &lookup) {
                Ok(target) => GuestImportResolution::Guest {
                    address: target.address,
                    module: target.image.image.module.clone(),
                },
                Err(error) => GuestImportResolution::Unresolved {
                    import: import.clone(),
                    detail: error.to_string(),
                },
            };
        }
        if let Some(entry) = self.context.shared.borrow().imports.get(&key) {
            return GuestImportResolution::Host {
                address: entry.address,
                callback: entry.callback.clone(),
            };
        }
        let signature = self.context.signature(&library, &[], None);
        let invoke_library = library.clone();
        let invoke_name = name.clone();
        if self
            .context
            .register_unsupported(
                memory,
                &library,
                &name,
                signature,
                Rc::new(move |_, _, _| {
                    Err(unsupported_windows(
                        &invoke_library,
                        &invoke_name,
                        "runtime service is not implemented",
                    ))
                }),
            )
            .is_err()
        {
            return GuestImportResolution::Unresolved {
                import: import.clone(),
                detail: format!("Windows lazy import registration failed: {key}"),
            };
        }
        match self.context.shared.borrow().imports.get(&key) {
            Some(entry) => GuestImportResolution::Host {
                address: entry.address,
                callback: entry.callback.clone(),
            },
            None => GuestImportResolution::Unresolved {
                import: import.clone(),
                detail: format!("Windows lazy import registration failed: {key}"),
            },
        }
    }
}

fn attach_failed(image: &PeImage) -> GuestError {
    GuestError::callback(format!(
        "Windows DLL_PROCESS_ATTACH failed: {}:{}",
        image.image.module.id.namespace, image.image.module.id.name
    ))
}

fn map_nested(result: Result<GuestCallResult, GuestCallFailure>) -> Result<GuestCallResult, GuestError> {
    result.map_err(|failure| match failure {
        GuestCallFailure::Guest(error) => error,
        GuestCallFailure::Stopped { stop, .. } => {
            GuestError::callback(format!("Nested Windows guest call stopped: {stop:?}"))
        }
    })
}

fn validate_cfg(
    ctx: &mut crate::core::callbacks::HostCallContext<'_, '_>,
    shared: &SharedWindows,
    target: u64,
    context: &GuestCallContext,
) -> Result<(), GuestError> {
    let _ = context;
    let valid = shared.borrow().cfg_targets.contains(&target)
        || ctx.hooks().callbacks.borrow().has_bound_trap(target);
    let address = ctx.memory().pointer(target)?;
    let (true, Some(address)) = (valid, address) else {
        return Err(unsupported_windows(
            "quake-runtime.dll",
            "control-flow-guard",
            format!("invalid indirect target 0x{target:x}"),
        ));
    };
    ctx.memory().check(address, 1, GuestAccess::Execute)
}
