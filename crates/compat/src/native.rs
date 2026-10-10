//! Native module exports and imports over the one owned-child backend.
use crate::{
    abi::{Addresses, CallTable, Invocation, UnknownCalls},
    memory::ModuleMemory,
    services::{CallContext, CallError, EngineServices},
};
use qa_core::sys_events::EventTime;
use qa_formats::program::native::{Encoding, Image};
use qa_platform::native::{NativeAbi, NativeError, NativeImage, NativeProcess, NativeRegion};
use std::time::Duration;

pub mod elf;

#[derive(Clone, Copy)]
pub struct Export {
    pub address: u64,
    /// vmMain command selectors are declared at binding; ordinary exports
    /// receive only their native arguments.
    pub command: Option<u32>,
}
pub struct NamedExport<'a> {
    pub name: &'a [u8],
    pub command: Option<u32>,
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
    /// this step only transfers its bytes to the authoritative child backing.
    pub fn map_image(
        image: Image,
        named: &[NamedExport<'_>],
        timeout: Duration,
    ) -> Result<Self, Error> {
        let exports = named
            .iter()
            .map(|entry| {
                let symbol = image.symbol(entry.name).ok_or(Error::Export)?;
                if symbol.forward.is_some() || symbol.kind == 10 {
                    return Err(Error::Export);
                }
                Ok(Export {
                    address: symbol.address,
                    command: entry.command,
                })
            })
            .collect::<Result<Box<[_]>, _>>()?;
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
            base: image.base,
            pointer_bytes: (image.target.bits / 8) as u8,
            bytes: &image.bytes,
            regions: &regions,
            timeout,
        })
        .map_err(Error::Process)?;
        if exports.iter().any(|e| !process.executable(e.address)) {
            return Err(Error::Export);
        }
        Ok(Self {
            process,
            abi,
            exports,
        })
    }
    pub fn has_export(&self, ordinal: u32) -> bool {
        (ordinal as usize) < self.exports.len()
    }
    pub fn call(
        &mut self,
        calls: &mut NativeCalls<'_, '_>,
        ordinal: u32,
        arguments: &[u64],
    ) -> Result<u64, Error> {
        let export = *self.exports.get(ordinal as usize).ok_or(Error::Export)?;
        let first = usize::from(export.command.is_some());
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
            .invoke(export.address, self.abi, words, |call, base, bytes| {
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
