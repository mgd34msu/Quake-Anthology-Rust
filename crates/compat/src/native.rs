//! Native module exports and imports over the one owned-child backend.
use crate::{
    abi::{Addresses, CallTable, Invocation, UnknownCalls},
    memory::ModuleMemory,
    services::{CallContext, CallError, EngineServices},
};
use qa_core::sys_events::EventTime;
use qa_platform::native::{NativeAbi, NativeError, NativeProcess};

#[derive(Clone, Copy)]
pub struct Export {
    pub address: u64,
    /// vmMain command selectors are declared at binding; ordinary exports
    /// receive only their native arguments.
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
    pub fn load(
        process: NativeProcess,
        abi: NativeAbi,
        exports: Box<[Export]>,
    ) -> Result<Self, Error> {
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
