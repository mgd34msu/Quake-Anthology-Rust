//! Native module exports and imports over the one owned-child backend.
use crate::{
    abi::{Addresses, CallTable, Invocation, UnknownCalls},
    memory::ModuleMemory,
    services::{CallContext, CallError, EngineServices},
};
use qa_core::sys_events::EventTime;
use qa_formats::program::native::{Encoding, Image};
use qa_platform::native::{
    NativeAbi, NativeEntry, NativeError, NativeImage, NativeImport, NativeProcess, NativeRegion,
    NativeScalar,
};
use std::time::Duration;

pub mod elf;

#[derive(Clone, Copy)]
struct Export {
    entry: NativeEntry,
    /// vmMain command selectors are declared at binding; ordinary exports
    /// receive only their native arguments.
    command: Option<u32>,
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
    Service(CallError),
    Process(NativeError),
}
pub struct Vm {
    pub process: NativeProcess,
    abi: NativeAbi,
    exports: Box<[Export]>,
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
        let targets = named
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
                Ok(symbol.address)
            })
            .collect::<Result<Box<[_]>, _>>()?;
        // Native ELF RELRO protects complete pages, rounding both ends down.
        // Split the one region table at those boundaries before mapping; the
        // same final table supplies OS rights and callable-entry checks.
        for &(address, bytes) in &image.relro {
            let begin = address & !4095;
            let end = address
                .checked_add(bytes as u64)
                .ok_or(Error::Process(NativeError::Extent))?
                & !4095;
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
            imports,
            base: image.base,
            pointer_bytes: (image.target.bits / 8) as u8,
            bytes: &image.bytes,
            regions: &regions,
            timeout,
        })
        .map_err(Error::Process)?;
        if targets.iter().any(|&address| !process.executable(address)) {
            return Err(Error::Export);
        }
        let exports = named
            .iter()
            .zip(targets.iter())
            .map(|(named, &address)| {
                Ok(Export {
                    entry: process
                        .bind(address, abi, named.parameters, named.result)
                        .map_err(Error::Process)?,
                    command: named.command,
                })
            })
            .collect::<Result<Box<[_]>, Error>>()?;
        Ok(Self {
            process,
            abi,
            exports,
        })
    }
    pub fn has_export(&self, ordinal: u32) -> bool {
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
        let export = *self.exports.get(ordinal as usize).ok_or(Error::Export)?;
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
        let result = self
            .process
            .invoke(export.entry, words, |call, base, bytes| {
                let mut memory =
                    ModuleMemory::borrow(base, bytes).map_err(|_| NativeError::Callback)?;
                let mut invocation = Invocation {
                    services: calls.services,
                    memory: &mut memory,
                    context: calls.context,
                    platform_time: calls.platform_time,
                    command: calls.command,
                    addresses: Addresses::Native,
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
        result.map_err(|error| rejected.map_or(Error::Process(error), Error::Service))
    }
}
