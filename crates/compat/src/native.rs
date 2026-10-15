//! Native module exports and imports over the one owned-child backend.
use crate::{
    abi::{
        Addresses, CallTable, HostRequest, ImportContinuation, ImportStep, Invocation, UnknownCalls,
    },
    memory::{ModuleMemory, ProcessMemoryError, process_memory},
    services::{CallContext, CallError, ENGINE_CALLS, EngineServices},
};
use qa_core::{names::NameTable, sys_events::EventTime, text::FixedText};
use qa_formats::program::native::{Encoding, Image};
use qa_platform::native::{
    NATIVE_REENTRY_LIMIT, NativeAbi, NativeCall, NativeCallToken, NativeEntry, NativeError,
    NativeImage, NativeImport, NativeImportToken, NativeProcess, NativeRegion, NativeScalar,
    NativeStop, PAGE_BYTES,
};
use std::{fmt::Write, sync::Arc, time::Duration};

pub mod elf;
use crate::entities::Entities;
pub mod q2;
mod runtime;
mod table;
pub use table::{ReturnedTable, TableFunction};

#[derive(Clone, Copy)]
struct Export {
    entry: NativeEntry,
    /// vmMain command selectors are declared at binding; ordinary exports
    /// receive only their native arguments.
    command: Option<u32>,
    reject_zero: bool,
}
impl Export {
    fn bind(
        process: &NativeProcess,
        abi: NativeAbi,
        address: u64,
        parameters: &[NativeScalar],
        result: NativeScalar,
        command: Option<u32>,
    ) -> Result<Self, Error> {
        Ok(Self {
            entry: process
                .bind(address, abi, parameters, result)
                .map_err(|error| match error {
                    NativeError::Extent => Error::Export,
                    error => Error::Process(error),
                })?,
            command,
            reject_zero: false,
        })
    }
}
#[derive(Clone, Copy)]
pub struct LifecycleCall {
    pub ordinal: u32,
    pub arguments: [u64; 3],
}
pub struct NamedExport<'a> {
    pub name: &'a [u8],
    pub command: Option<u32>,
    pub parameters: &'a [NativeScalar],
    pub result: NativeScalar,
}
#[derive(Debug)]
pub enum Error {
    Export,
    DllAttach,
    Service(CallError),
    Process(NativeError),
    Binding(String),
}
impl Error {
    pub fn recoverable(&self) -> bool {
        matches!(self, Self::Process(NativeError::ImportTrap { .. }))
    }
}
impl From<ProcessMemoryError> for Error {
    fn from(error: ProcessMemoryError) -> Self {
        match error {
            ProcessMemoryError::Process(error) => Self::Process(error),
            ProcessMemoryError::Memory(_) => Self::Service(CallError::Memory),
        }
    }
}
pub struct ImportTrapInfo<'a> {
    pub library: Option<&'a [u8]>,
    pub name: Option<&'a [u8]>,
    pub version: Option<&'a [u8]>,
    pub symbol_ordinal: Option<u16>,
    pub calls: u64,
}
pub struct Vm {
    pub process: NativeProcess,
    abi: NativeAbi,
    pointer_bytes: u8,
    exports: Box<[Option<Export>]>,
    table: Option<table::Table>,
    initialize: Box<[LifecycleCall]>,
    finalize: Box<[LifecycleCall]>,
    names: NameTable,
    traps: Box<[runtime::ImportTrap]>,
    cvars: Option<crate::cvars::NativeCvars>,
    native_command: Option<crate::command::NativeCommand>,
    native_configs: Option<crate::configstrings::NativeConfigs>,
    resources: Option<[qa_core::primitives::IndexSpan; 3]>,
    projection: Option<crate::projection::Projection>,
    pmove: Option<crate::pmove::Pmove>,
    client_think: Option<u32>,
    pmove_redirect: Option<(u32, usize)>,
    surfaces: Option<crate::surfaces::NativeSurfaces>,
    run_owner: Arc<()>,
    activation: u64,
    active_depth: usize,
}
pub struct NativeCalls<'a, 'engine> {
    pub services: &'a mut EngineServices<'engine>,
    pub table: &'static CallTable,
    pub context: CallContext,
    pub platform_time: EventTime,
    pub command: qa_console::command_text::Arguments<'a>,
    pub argument_selection: &'a mut crate::abi::ArgumentSelection,
    pub unknown: &'a mut UnknownCalls,
    pub native_player: Option<qa_core::primitives::ClientId>,
}
/// Owned high-level export state. No services, argv or guest bytes survive a
/// host stop; the owning caller reconstructs their short views on each step.
pub struct Run {
    owner: Arc<()>,
    call: Option<NativeCallToken>,
    export: Export,
    outer: bool,
    context: CallContext,
    native_player: Option<qa_core::primitives::ClientId>,
    platform_time: EventTime,
    activation: u64,
    depth: usize,
    pending: bool,
}
/// Owned host-stop continuation. The stopped child token and ABI continuation
/// stay together until their owning export is resumed.
pub struct PendingImport {
    stopped: NativeImportToken,
    continuation: ImportContinuation,
    owner: Arc<()>,
    activation: u64,
    depth: usize,
}
pub enum Step {
    Host {
        request: HostRequest,
        pending: PendingImport,
    },
    Returned(u64),
}
struct Suspended {
    run: Run,
    pending: Option<PendingImport>,
}
impl Vm {
    pub fn configure_q3_game(&mut self, layout: qa_core::primitives::Q3StatLayout) {
        self.projection = Some(crate::projection::Projection::load_q3(layout));
    }
    pub fn player_stat_layout(&self) -> qa_core::primitives::Q3StatLayout {
        self.projection
            .as_ref()
            .map_or(qa_core::primitives::Q3StatLayout::BASE, |projection| {
                projection.stat_layout()
            })
    }
    pub fn resource_ranges(&self) -> Option<&[qa_core::primitives::IndexSpan; 3]> {
        self.resources.as_ref()
    }
    /// Map a checked, bound image and register its ordinary exports. Import,
    /// TLS and initializer orchestration belongs to the owning image loader;
    /// this step transfers bytes to the authoritative child backing and applies
    /// final RELRO page rights.
    pub fn map_image(
        mut image: Image,
        named: &[NamedExport<'_>],
        imports: &[NativeImport<'_>],
        timeout: Duration,
    ) -> Result<Self, Error> {
        if image.target.bits != 64
            && !(image.target.bits == 32 && image.target.encoding == Encoding::Elf)
        {
            return Err(Error::Process(NativeError::Unsupported));
        }
        let pointer_bytes = (image.target.bits / 8) as u8;
        let source_bytes = image.bytes.len();
        let runtime = runtime::bind(&mut image, imports).map_err(Error::Binding)?;
        let imports: Vec<_> = imports.iter().copied().chain(runtime.imports).collect();
        let mut bindings = named
            .iter()
            .map(|entry| {
                let symbol = image.symbol(entry.name).ok_or(Error::Export)?;
                if symbol.forward.is_some() || symbol.kind == 10 {
                    return Err(Error::Export);
                }
                if entry.command.is_some()
                    && !entry.parameters.first().is_some_and(|kind| kind.integer())
                {
                    return Err(Error::Export);
                }
                Ok((
                    symbol.address,
                    entry.parameters,
                    entry.result,
                    entry.command,
                    false,
                ))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let lifecycle = if image.target.encoding == Encoding::Elf {
            elf::Lifecycle::load(&mut image, source_bytes)
                .map_err(|e| Error::Binding(format!("native lifecycle: {e:?}")))?
        } else {
            elf::Lifecycle {
                initialize: Vec::new(),
                finalize: Vec::new(),
            }
        };
        let mut initialize = Vec::new();
        let mut finalize = Vec::new();
        for (targets, calls, parameters, arguments) in [
            (
                lifecycle.initialize,
                &mut initialize,
                &[NativeScalar::I32, NativeScalar::Word, NativeScalar::Word][..],
                [
                    0,
                    runtime.startup.unwrap_or(0),
                    runtime.startup.unwrap_or(0) + u64::from(pointer_bytes),
                ],
            ),
            (lifecycle.finalize, &mut finalize, &[][..], [0; 3]),
        ] {
            for address in targets {
                let ordinal = u32::try_from(bindings.len()).map_err(|_| Error::Export)?;
                bindings.push((address, parameters, NativeScalar::Void, None, false));
                calls.push(LifecycleCall { ordinal, arguments });
            }
        }
        if image.target.encoding == Encoding::Pe {
            for (calls, reason) in [(&mut initialize, 1), (&mut finalize, 0)] {
                let mut targets: Vec<_> =
                    image.initializers.iter().map(|&at| (at, false)).collect();
                if image.entry != 0 {
                    if reason == 1 {
                        targets.push((image.entry, true));
                    } else {
                        targets.insert(0, (image.entry, true));
                    }
                }
                for (address, entry) in targets {
                    let ordinal = u32::try_from(bindings.len()).map_err(|_| Error::Export)?;
                    bindings.push((
                        address,
                        &[NativeScalar::Word, NativeScalar::U32, NativeScalar::Word],
                        if entry {
                            NativeScalar::I32
                        } else {
                            NativeScalar::Void
                        },
                        None,
                        entry && reason == 1,
                    ));
                    calls.push(LifecycleCall {
                        ordinal,
                        arguments: [image.base, reason, 0],
                    });
                }
            }
        }
        // Native ELF RELRO protects complete pages, rounding both ends down.
        // Split the one region table at those boundaries before mapping; the
        // same final table supplies OS rights and callable-entry checks.
        for &(address, bytes) in &image.relro {
            let mask = !(PAGE_BYTES as u64 - 1);
            let begin = address & mask;
            let end = address
                .checked_add(bytes as u64)
                .ok_or(Error::Process(NativeError::Extent))?
                & mask;
            if end <= begin {
                continue;
            }
            let begin = usize::try_from(
                begin
                    .checked_sub(image.base)
                    .ok_or(Error::Process(NativeError::Extent))?,
            )
            .map_err(|_| Error::Process(NativeError::Extent))?;
            let end = usize::try_from(
                end.checked_sub(image.base)
                    .ok_or(Error::Process(NativeError::Extent))?,
            )
            .map_err(|_| Error::Process(NativeError::Extent))?;
            let mut finalized = Vec::new();
            for region in &image.regions {
                let limit = region.offset + region.length;
                if region.offset >= end || limit <= begin {
                    finalized.push(*region);
                    continue;
                }
                if region.offset < begin {
                    finalized.push(qa_formats::program::native::Region {
                        length: begin - region.offset,
                        ..*region
                    });
                }
                let start = region.offset.max(begin);
                let stop = limit.min(end);
                finalized.push(qa_formats::program::native::Region {
                    offset: start,
                    length: stop - start,
                    read: true,
                    write: false,
                    execute: false,
                });
                if limit > end {
                    finalized.push(qa_formats::program::native::Region {
                        offset: end,
                        length: limit - end,
                        ..*region
                    });
                }
            }
            image.regions = finalized.into_boxed_slice();
        }
        let regions = image
            .regions
            .iter()
            .map(|region| NativeRegion {
                offset: region.offset,
                length: region.length,
                permissions: u8::from(region.read)
                    | (u8::from(region.write) << 1)
                    | (u8::from(region.execute) << 2),
            })
            .collect::<Box<[_]>>();
        let abi = match image.target.encoding {
            Encoding::Pe => NativeAbi::Microsoft,
            Encoding::Elf => NativeAbi::SystemV,
        };
        let process = NativeProcess::load(NativeImage {
            imports: &imports,
            base: image.base,
            pointer_bytes,
            bytes: &image.bytes,
            regions: &regions,
            timeout,
            runtime: runtime.config,
        })
        .map_err(Error::Process)?;
        let exports = bindings
            .into_iter()
            .map(|(address, parameters, result, command, reject_zero)| {
                let mut export = Export::bind(&process, abi, address, parameters, result, command)?;
                export.reject_zero = reject_zero;
                Ok(Some(export))
            })
            .collect::<Result<Box<[_]>, Error>>()?;
        Ok(Self {
            process,
            abi,
            pointer_bytes,
            exports,
            table: None,
            initialize: initialize.into_boxed_slice(),
            finalize: finalize.into_boxed_slice(),
            names: image.names,
            traps: runtime.traps.into_boxed_slice(),
            cvars: None,
            native_command: None,
            native_configs: None,
            resources: None,
            projection: None,
            pmove: None,
            client_think: None,
            pmove_redirect: None,
            surfaces: None,
            run_owner: Arc::new(()),
            activation: 0,
            active_depth: 0,
        })
    }
    pub fn unresolved_imports(&self) -> impl Iterator<Item = ImportTrapInfo<'_>> {
        self.traps.iter().map(|trap| ImportTrapInfo {
            library: trap.provider.and_then(|id| self.names.get(id)),
            name: trap.name.and_then(|id| self.names.get(id)),
            version: trap.version.and_then(|id| self.names.get(id)),
            symbol_ordinal: trap.symbol_ordinal,
            calls: trap.calls,
        })
    }
    pub fn initializers(&self) -> &[LifecycleCall] {
        &self.initialize
    }
    pub fn finalizers(&self) -> &[LifecycleCall] {
        &self.finalize
    }
    pub fn declares_export(&self, ordinal: u32) -> bool {
        (ordinal as usize) < self.exports.len()
    }
    /// Resolve a declared vmMain selector to its bound export index at load.
    pub fn export_for_command(&self, command: u32) -> Option<u32> {
        self.exports
            .iter()
            .position(|export| {
                export
                    .as_ref()
                    .is_some_and(|export| export.command == Some(command))
            })
            .and_then(|index| u32::try_from(index).ok())
    }
    pub fn export_count(&self) -> usize {
        self.exports.len()
    }
    pub fn import_callback(&self) -> u64 {
        self.process.callback(self.abi)
    }
    /// Session load supplies new shared service storage even for a retained VM.
    pub fn rebind_configstrings(&mut self) {
        if let Some(configs) = &mut self.native_configs {
            configs.rebind();
        }
    }
    pub fn bind_player_values(
        &mut self,
        values: qa_core::primitives::PlayerValueBindings<'_>,
        registry: &qa_gameplay::registry::Registry,
    ) -> Result<(), Error> {
        if let Some(projection) = &mut self.projection {
            projection
                .bind_values(values, registry)
                .map_err(Error::Service)?;
        }
        Ok(())
    }
    pub fn is_active(&self) -> bool {
        self.active_depth != 0
    }
    pub fn activation_capacity(&self) -> usize {
        NATIVE_REENTRY_LIMIT + 1
    }
    fn owns_active_run(&self, run: &Run) -> bool {
        Arc::ptr_eq(&self.run_owner, &run.owner)
            && run.call.is_some()
            && run.activation == self.activation
            && run.depth <= self.active_depth
    }
    fn validate_run(&self, run: &Run) -> Result<(), Error> {
        if !self.owns_active_run(run) || run.depth != self.active_depth {
            return Err(Error::Export);
        }
        Ok(())
    }
    fn validate_pending(&self, run: &Run, pending: &PendingImport) -> Result<(), Error> {
        self.validate_run(run)?;
        if !run.pending
            || !Arc::ptr_eq(&self.run_owner, &pending.owner)
            || pending.activation != run.activation
            || pending.depth != run.depth
        {
            return Err(Error::Export);
        }
        Ok(())
    }
    pub fn with_trace_view<T>(
        &mut self,
        context: CallContext,
        geometry: Option<qa_core::primitives::GeometryId>,
        pending: Option<(&Run, &PendingImport)>,
        read: impl FnOnce(Option<&dyn qa_world::collision::NativeTraceEntities>) -> Result<T, CallError>,
    ) -> Result<T, Error> {
        if let Some((run, pending)) = pending {
            self.validate_pending(run, pending)?;
        } else if self.is_active() {
            return Err(Error::Export);
        }
        let projection = &self.projection;
        let inspect = |memory: &ModuleMemory<'_>| {
            crate::entities::EntityProjection::with_trace_view(
                projection.as_ref().map(|projection| &projection.entities),
                memory,
                context,
                geometry,
                read,
            )
        };
        if let Some((_, pending)) = pending {
            self.process
                .with_import_memory(&pending.stopped, |base, bytes| {
                    Ok(ModuleMemory::borrow(base, bytes)
                        .map_err(|_| CallError::Memory)
                        .and_then(|memory| inspect(&memory)))
                })
                .map_err(Error::Process)?
                .map_err(Error::Service)
        } else {
            let memory = process_memory(&mut self.process)?;
            inspect(&memory).map_err(Error::Service)
        }
    }
    pub fn copy_import_command(
        &mut self,
        run: &Run,
        pending: &PendingImport,
        pointer: u64,
        out: &mut qa_console::command_text::ParsedCommand,
    ) -> Result<bool, Error> {
        self.validate_pending(run, pending)?;
        let mut rejected = None;
        let result = self
            .process
            .with_import_memory(&pending.stopped, |base, bytes| {
                let result = (|| {
                    let memory =
                        ModuleMemory::borrow(base, bytes).map_err(|_| CallError::Memory)?;
                    let text = memory.cstring(pointer)?;
                    out.set_raw(text).map_err(|_| CallError::Text)?;
                    Ok(!text.is_empty())
                })();
                result.map_err(|error| {
                    rejected = Some(error);
                    NativeError::Callback
                })
            });
        result.map_err(|error| rejected.map_or(Error::Process(error), Error::Service))
    }
    pub fn begin(
        &mut self,
        calls: &mut NativeCalls<'_, '_>,
        ordinal: u32,
        arguments: &[u64],
    ) -> Result<Run, Error> {
        let export = self
            .exports
            .get(ordinal as usize)
            .copied()
            .flatten()
            .ok_or(Error::Export)?;
        let first = usize::from(export.command.is_some());
        if arguments.len() > 13 - first {
            return Err(Error::Export);
        }
        let native_player = if self.client_think == Some(ordinal) {
            Some(
                calls
                    .native_player
                    .ok_or(Error::Service(CallError::Entity))?,
            )
        } else {
            None
        };
        let mut words = [0; 13];
        if let Some(command) = export.command {
            words[0] = u64::from(command);
        }
        words[first..first + arguments.len()].copy_from_slice(arguments);
        let outer = !self.is_active();
        if outer {
            if self.cvars.is_some()
                || self.native_command.is_some()
                || self.native_configs.is_some()
            {
                let mut memory = process_memory(&mut self.process)?;
                if let Some(cvars) = &mut self.cvars {
                    cvars
                        .refresh(calls.services.cvars, &mut memory)
                        .map_err(Error::Service)?;
                }
                if let Some(command) = &mut self.native_command {
                    command
                        .prepare(&mut memory, &calls.command)
                        .map_err(Error::Service)?;
                }
                if let Some(configs) = &mut self.native_configs {
                    configs
                        .refresh(calls.services.storage, calls.context.module, &mut memory)
                        .map_err(Error::Service)?;
                }
            }
            if let Some(projection) = &mut self.projection {
                projection
                    .seed(&calls.services.server.entities, calls.context.module)
                    .map_err(Error::Service)?;
            }
            self.process
                .set_event_time(calls.platform_time)
                .map_err(Error::Process)?;
        }
        let call = match self.process.enter(export.entry, words) {
            Ok(call) => call,
            Err(error) => {
                if self.process.pid() == 0 {
                    self.active_depth = 0;
                    calls
                        .services
                        .storage
                        .abort_client_callbacks(calls.context.module);
                }
                return Err(Error::Process(error));
            }
        };
        if outer {
            self.activation = self.activation.wrapping_add(1);
        }
        self.active_depth += 1;
        Ok(Run {
            owner: Arc::clone(&self.run_owner),
            call: Some(call),
            export,
            outer,
            context: calls.context,
            native_player,
            platform_time: calls.platform_time,
            activation: self.activation,
            depth: self.active_depth,
            pending: false,
        })
    }
    fn import_step(
        &mut self,
        calls: &mut NativeCalls<'_, '_>,
        run: &Run,
        stopped: &NativeImportToken,
        call: Option<NativeCall>,
        returned: Option<(ImportContinuation, u64)>,
    ) -> Result<ImportStep, Error> {
        let cvars = &mut self.cvars;
        let resources = self.resources.as_ref();
        let mut entities = self.projection.as_mut().map(|p| &mut p.entities);
        let surfaces = self.surfaces.as_ref();
        let native_command = self.native_command.as_ref();
        let native_configs = &mut self.native_configs;
        let pmove = &mut self.pmove;
        let abi = self.abi;
        let pointer_bytes = self.pointer_bytes;
        let mut rejected = None;
        let result = self.process.with_import_memory(stopped, |base, bytes| {
            let result = (|| {
                let mut memory =
                    ModuleMemory::borrow(base, bytes).map_err(|_| CallError::Memory)?;
                let mut invocation = Invocation {
                    services: calls.services,
                    memory: &mut memory,
                    native_cvars: cvars.as_mut(),
                    native_resources: resources,
                    native_entities: entities.as_deref_mut(),
                    native_surfaces: surfaces,
                    native_command,
                    native_configs: native_configs.as_mut(),
                    native_pmove: pmove.as_mut(),
                    native_player: run.native_player,
                    context: run.context,
                    platform_time: run.platform_time,
                    command: calls.command,
                    argument_selection: &mut *calls.argument_selection,
                    addresses: if call.as_ref().is_some_and(|call| call.function) {
                        Addresses::NativeFunction { abi, pointer_bytes }
                    } else {
                        Addresses::Native { abi, pointer_bytes }
                    },
                    arguments: call.as_ref().map_or(&[], |call| call.arguments.as_slice()),
                };
                let step = match returned {
                    Some((resume, value)) => resume.resume(&mut invocation, value)?,
                    None => calls.table.invoke(
                        call.as_ref().ok_or(CallError::Memory)?.number,
                        &mut invocation,
                        calls.unknown,
                    )?,
                };
                if let Some(configs) = &mut invocation.native_configs {
                    configs.refresh(
                        invocation.services.storage,
                        invocation.context.module,
                        invocation.memory,
                    )?;
                }
                Ok(step)
            })();
            result.map_err(|error| {
                rejected = Some(error);
                NativeError::Callback
            })
        });
        result.map_err(|error| rejected.map_or(Error::Process(error), Error::Service))
    }
    fn failed(&mut self, calls: &mut NativeCalls<'_, '_>, run: &mut Run, error: Error) -> Error {
        // Raw controller-token rejection does not change the child/transaction.
        // Preserve the valid owner so the caller may retry its correct stop.
        if matches!(&error, Error::Process(NativeError::Protocol)) && self.process.pid() != 0 {
            return error;
        }
        // Only advance can produce a recoverable child trap. The raw platform
        // has already invalidated its transaction and preserved its outer park.
        if let Some(call) = run.call.take() {
            if !error.recoverable() {
                let _ = self.process.abort(call, NativeError::Callback);
            }
        }
        self.active_depth = 0;
        calls
            .services
            .storage
            .abort_client_callbacks(run.context.module);
        self.record_trap(calls, run.context, &error);
        error
    }
    fn record_trap(
        &mut self,
        calls: &mut NativeCalls<'_, '_>,
        context: CallContext,
        error: &Error,
    ) {
        if let Error::Process(NativeError::ImportTrap { ordinal, address }) = error {
            if let Some(trap) = self.traps.iter_mut().find(|trap| trap.ordinal == *ordinal) {
                let first = trap.calls == 0;
                trap.calls = trap.calls.saturating_add(1);
                if first {
                    let mut text = FixedText::<2048>::default();
                    let format = (|| {
                        write!(text, "module {}: native import ", context.module.0)?;
                        if let Some(provider) = trap.provider.and_then(|id| self.names.get(id)) {
                            text.append_bytes(provider)?;
                            text.append_bytes(b":")?;
                        }
                        if let Some(name) = trap.name.and_then(|id| self.names.get(id)) {
                            text.append_bytes(name)?;
                        } else if let Some(ordinal) = trap.symbol_ordinal {
                            write!(text, "#{ordinal}")?;
                        }
                        if let Some(version) = trap.version.and_then(|id| self.names.get(id)) {
                            text.append_bytes(b"@")?;
                            text.append_bytes(version)?;
                        }
                        writeln!(text, " at {address:#x}")
                    })();
                    if format.is_ok() {
                        let _ = (ENGINE_CALLS.console)(calls.services, text.as_bytes());
                    }
                }
            }
        }
    }
    pub fn advance(
        &mut self,
        calls: &mut NativeCalls<'_, '_>,
        run: &mut Run,
    ) -> Result<Step, Error> {
        self.validate_run(run)?;
        if run.pending {
            return Err(Error::Export);
        }
        self.drive(calls, run, None)
    }
    fn drive(
        &mut self,
        calls: &mut NativeCalls<'_, '_>,
        run: &mut Run,
        mut imported: Option<(NativeImportToken, ImportStep)>,
    ) -> Result<Step, Error> {
        loop {
            if let Some((stopped, step)) = imported.take() {
                match step {
                    ImportStep::Return(value) => {
                        run.pending = false;
                        if let Err(error) = self.process.reply(stopped, value) {
                            return Err(self.failed(calls, run, Error::Process(error)));
                        }
                    }
                    ImportStep::Host {
                        request,
                        continuation,
                    } => {
                        run.pending = true;
                        return Ok(Step::Host {
                            request,
                            pending: PendingImport {
                                stopped,
                                continuation,
                                owner: Arc::clone(&self.run_owner),
                                activation: run.activation,
                                depth: run.depth,
                            },
                        });
                    }
                }
            }
            let stopped = match self
                .process
                .advance(run.call.as_ref().ok_or(Error::Export)?)
            {
                Ok(stopped) => stopped,
                Err(error) => return Err(self.failed(calls, run, Error::Process(error))),
            };
            match stopped {
                NativeStop::Returned(value) => {
                    run.call = None;
                    self.active_depth -= 1;
                    if run.export.reject_zero && value == 0 {
                        return Err(Error::DllAttach);
                    }
                    if run.outer {
                        if let Some(projection) = &mut self.projection {
                            let mut memory = process_memory(&mut self.process)?;
                            projection
                                .publish(
                                    &mut memory,
                                    calls.services,
                                    run.context,
                                    self.pmove.as_mut(),
                                    run.platform_time,
                                    run.export.command,
                                )
                                .map_err(Error::Service)?;
                        }
                    }
                    return Ok(Step::Returned(value));
                }
                NativeStop::Import { call, stopped } => {
                    let step = match self.import_step(calls, run, &stopped, Some(call), None) {
                        Ok(step) => step,
                        Err(error) => return Err(self.failed(calls, run, error)),
                    };
                    imported = Some((stopped, step));
                }
            }
        }
    }
    pub fn resume(
        &mut self,
        calls: &mut NativeCalls<'_, '_>,
        run: &mut Run,
        pending: &mut Option<PendingImport>,
        returned: u64,
    ) -> Result<Step, Error> {
        self.validate_pending(run, pending.as_ref().ok_or(Error::Export)?)?;
        let PendingImport {
            stopped,
            continuation,
            ..
        } = pending.take().ok_or(Error::Export)?;
        let step =
            match self.import_step(calls, run, &stopped, None, Some((continuation, returned))) {
                Ok(step) => step,
                Err(error) => return Err(self.failed(calls, run, error)),
            };
        self.drive(calls, run, Some((stopped, step)))
    }
    pub fn abort(
        &mut self,
        calls: &mut NativeCalls<'_, '_>,
        run: &mut Run,
        error: CallError,
    ) -> Error {
        if self.owns_active_run(run) {
            return self.failed(calls, run, Error::Service(error));
        }
        Error::Export
    }
    fn unwind_synchronous(
        &mut self,
        calls: &mut NativeCalls<'_, '_>,
        current: &mut Run,
        suspended: &mut [Option<Suspended>],
        error: Error,
    ) -> Error {
        if self.owns_active_run(current) {
            return self.failed(calls, current, error);
        }
        // A returned nested export has consumed its token before post-return
        // validation (notably DllAttach). Its nearest live ancestor still owns
        // the parked transaction and must cancel it before the stack is lost.
        for parent in suspended.iter_mut().rev().flatten() {
            if self.owns_active_run(&parent.run) {
                return self.failed(calls, &mut parent.run, error);
            }
        }
        error
    }
    /// Synchronous callers drain the same resumable service path. Cross-module
    /// host operations belong to the app's iterative FunctionTable driver.
    pub fn call(
        &mut self,
        calls: &mut NativeCalls<'_, '_>,
        ordinal: u32,
        arguments: &[u64],
    ) -> Result<u64, Error> {
        let mut suspended: [Option<Suspended>; NATIVE_REENTRY_LIMIT] =
            std::array::from_fn(|_| None);
        let mut depth = 0;
        let mut run = self.begin(calls, ordinal, arguments)?;
        let result = (|| {
            let mut step = self.advance(calls, &mut run)?;
            loop {
                match step {
                    Step::Returned(value) => {
                        if depth == 0 {
                            return Ok(value);
                        }
                        depth -= 1;
                        let mut parent = suspended[depth].take().ok_or(Error::Export)?;
                        run = parent.run;
                        step = self.resume(calls, &mut run, &mut parent.pending, value)?;
                    }
                    Step::Host { request, pending } => match request {
                        HostRequest::Callback(request) => {
                            if let Err(error) = request.validate(calls.services, run.context.module)
                            {
                                return Err(Error::Service(error));
                            }
                            let (time, arguments, client) = match request.words() {
                                Ok(words) => words,
                                Err(error) => return Err(Error::Service(error)),
                            };
                            if depth == NATIVE_REENTRY_LIMIT {
                                return Err(Error::Service(CallError::Aborted));
                            }
                            let context = calls.context;
                            let native_player = calls.native_player;
                            calls.context = run.context;
                            calls.context.clock = time;
                            calls.native_player = client;
                            let nested = self.begin(calls, request.target.entry(), &arguments);
                            calls.context = context;
                            calls.native_player = native_player;
                            let nested = nested?;
                            suspended[depth] = Some(Suspended {
                                run: std::mem::replace(&mut run, nested),
                                pending: Some(pending),
                            });
                            depth += 1;
                            step = self.advance(calls, &mut run)?;
                        }
                        HostRequest::ClearKeys {
                            seat,
                            context,
                            time,
                        } => {
                            if let Err(error) =
                                (ENGINE_CALLS.clear_keys)(calls.services, seat, context, time)
                            {
                                return Err(Error::Service(error));
                            }
                            step = self.resume(calls, &mut run, &mut Some(pending), 0)?;
                        }
                        HostRequest::TouchTriggers { .. } | HostRequest::ExecuteNow { .. } => {
                            return Err(Error::Service(CallError::Unsupported));
                        }
                    },
                }
            }
        })();
        result.map_err(|error| self.unwind_synchronous(calls, &mut run, &mut suspended, error))
    }
}

impl Vm {
    /// Resolve through the existing lifetime-checked native slot table without
    /// reading child memory or inventing an entity from a wire ordinal.
    pub fn entity_binding(
        &self,
        entities: &qa_world::entities::EntityTable,
        native: qa_core::primitives::NativeEntity,
    ) -> Option<qa_core::primitives::EntityId> {
        self.projection
            .as_ref()?
            .entities
            .binding(entities, native)
            .ok()?
    }
    pub fn bind_client(
        &mut self,
        entities: &qa_world::entities::EntityTable,
        native: qa_core::primitives::NativeEntity,
        entity: qa_core::primitives::EntityId,
    ) -> Result<(), Error> {
        self.projection
            .as_mut()
            .ok_or(Error::Export)?
            .bind_client(entities, native, entity)
            .map_err(Error::Service)
    }
    pub fn entities(&mut self) -> Result<Entities, Error> {
        let layout = &self.projection.as_ref().ok_or(Error::Export)?.entities;
        let memory = process_memory(&mut self.process).map_err(|error| match error {
            ProcessMemoryError::Memory(_) => Error::Export,
            error => error.into(),
        })?;
        layout.read(&memory).map_err(|_| Error::Export)
    }
}
