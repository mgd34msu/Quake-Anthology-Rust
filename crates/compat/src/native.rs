//! Native module exports and imports over the one owned-child backend.
use crate::{
    abi::{Addresses, CallTable, Invocation, UnknownCalls},
    memory::ModuleMemory,
    services::{CallContext, CallError, ENGINE_CALLS, EngineServices},
};
use qa_core::{names::NameTable, primitives::PrintKind, sys_events::EventTime, text::FixedText};
use qa_formats::program::native::{Encoding, Image};
use qa_platform::native::{
    NativeAbi, NativeEntry, NativeError, NativeImage, NativeImport, NativeProcess, NativeRegion,
    NativeScalar, PAGE_BYTES,
};
use std::{fmt::Write, time::Duration};

pub mod elf;
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
    exports: Box<[Option<Export>]>,
    table: Option<table::Table>,
    initialize: Box<[LifecycleCall]>,
    finalize: Box<[LifecycleCall]>,
    names: NameTable,
    traps: Box<[runtime::ImportTrap]>,
    cvars: Option<crate::cvars::NativeCvars>,
}
pub struct NativeCalls<'a, 'engine> {
    pub services: &'a mut EngineServices<'engine>,
    pub table: &'static CallTable,
    pub context: CallContext,
    pub platform_time: EventTime,
    pub command: &'a [&'a [u8]],
    pub unknown: &'a mut UnknownCalls,
}
impl Vm {
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
        if image.target.bits != 64 {
            return Err(Error::Process(NativeError::Unsupported));
        }
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
                    runtime.startup.unwrap_or(0) + 8,
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
            pointer_bytes: (image.target.bits / 8) as u8,
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
            exports,
            table: None,
            initialize: initialize.into_boxed_slice(),
            finalize: finalize.into_boxed_slice(),
            names: image.names,
            traps: runtime.traps.into_boxed_slice(),
            cvars: None,
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
    pub fn import_callback(&self) -> u64 {
        self.process.callback(self.abi)
    }
    pub fn call(
        &mut self,
        calls: &mut NativeCalls<'_, '_>,
        ordinal: u32,
        arguments: &[u64],
    ) -> Result<u64, Error> {
        let export = self
            .exports
            .get(ordinal as usize)
            .copied()
            .flatten()
            .ok_or(Error::Export)?;
        let first = usize::from(export.command.is_some());
        // Session dispatch supplies a fixed-capacity common payload. The
        // load-selected native entry admits only its declared argument slots;
        // unrelated payload words are dropped at this ABI boundary.
        if arguments.len() > 13 - first {
            return Err(Error::Export);
        }
        let mut words = [0; 13];
        if let Some(command) = export.command {
            words[0] = u64::from(command);
        }
        words[first..first + arguments.len()].copy_from_slice(arguments);
        let mut rejected = None;
        if let Some(cvars) = &mut self.cvars {
            let base = self.process.base();
            let mut memory =
                ModuleMemory::borrow(base, self.process.memory_mut().map_err(Error::Process)?)
                    .map_err(|_| Error::Service(CallError::Memory))?;
            cvars
                .refresh(calls.services.cvars, &mut memory)
                .map_err(Error::Service)?;
        }
        let cvars = &mut self.cvars;
        self.process
            .set_event_time(calls.platform_time)
            .map_err(Error::Process)?;
        let result = self
            .process
            .invoke(export.entry, words, |call, base, bytes| {
                let mut memory =
                    ModuleMemory::borrow(base, bytes).map_err(|_| NativeError::Callback)?;
                let mut invocation = Invocation {
                    services: calls.services,
                    memory: &mut memory,
                    native_cvars: cvars.as_mut(),
                    context: calls.context,
                    platform_time: calls.platform_time,
                    command: calls.command,
                    addresses: if call.function {
                        Addresses::NativeFunction
                    } else {
                        Addresses::Native
                    },
                    arguments: &call.arguments,
                };
                calls
                    .table
                    .invoke(call.number, &mut invocation, calls.unknown)
                    .map_err(|error| {
                        rejected = Some(error);
                        NativeError::Callback
                    })
            });
        if let Err(NativeError::ImportTrap { ordinal, address }) = &result {
            if let Some(trap) = self.traps.iter_mut().find(|trap| trap.ordinal == *ordinal) {
                let first = trap.calls == 0;
                trap.calls = trap.calls.saturating_add(1);
                if first {
                    let mut text = FixedText::<2048>::default();
                    let format = (|| {
                        write!(text, "module {}: native import ", calls.context.module.0)?;
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
                        let _ = (ENGINE_CALLS.print)(
                            calls.services,
                            None,
                            PrintKind::Console,
                            text.as_bytes(),
                        );
                    }
                }
            }
        }
        let value =
            result.map_err(|error| rejected.map_or(Error::Process(error), Error::Service))?;
        if export.reject_zero && value == 0 {
            return Err(Error::DllAttach);
        }
        Ok(value)
    }
}
