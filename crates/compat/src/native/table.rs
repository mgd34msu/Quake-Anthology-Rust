//! Load-declared native function tables returned by a module API entry.
use super::{Error, Export, Vm};
use crate::memory::ModuleMemory;
use qa_platform::native::NativeScalar;

pub struct TableFunction {
    pub offset: usize,
    pub parameters: &'static [NativeScalar],
    pub result: NativeScalar,
}
pub struct ReturnedTable {
    pub version: i32,
    pub bytes: usize,
    pub functions: Box<[TableFunction]>,
}
pub(super) struct Table {
    layout: ReturnedTable,
    first: usize,
    address: Option<u64>,
}
impl Vm {
    /// Declare slots before session FunctionTable construction. Their native
    /// addresses become callable only after the API return has been checked.
    pub fn declare_table(&mut self, layout: ReturnedTable) -> Result<u32, Error> {
        if self.table.is_some()
            || layout.bytes < 4
            || layout.functions.is_empty()
            || layout.functions.iter().any(|function| {
                function.offset < 8
                    || function.offset % 8 != 0
                    || function
                        .offset
                        .checked_add(8)
                        .is_none_or(|n| n > layout.bytes)
            })
        {
            return Err(Error::Export);
        }
        let first = self.exports.len();
        let end = first
            .checked_add(layout.functions.len())
            .ok_or(Error::Export)?;
        let ordinal = u32::try_from(first).map_err(|_| Error::Export)?;
        u32::try_from(end).map_err(|_| Error::Export)?;
        let mut exports = std::mem::take(&mut self.exports).into_vec();
        exports.resize(end, None);
        self.exports = exports.into_boxed_slice();
        self.table = Some(Table {
            layout,
            first,
            address: None,
        });
        Ok(ordinal)
    }
    /// Called once during session initialization, with the child stopped.
    /// Resolve every entry before publishing any, so malformed tables cannot
    /// leave a partially callable module.
    pub fn bind_table(&mut self, address: u64) -> Result<(), Error> {
        let table = self.table.as_ref().ok_or(Error::Export)?;
        if table.address.is_some() || address == 0 || address % 8 != 0 {
            return Err(Error::Export);
        }
        let targets = {
            let base = self.process.base();
            let memory =
                ModuleMemory::borrow(base, self.process.memory_mut().map_err(Error::Process)?)
                    .map_err(|_| Error::Export)?;
            let version = memory.read_word(address).map_err(|_| Error::Export)?;
            if version != table.layout.version {
                return Err(Error::Binding(format!(
                    "native API version {version}, expected {}",
                    table.layout.version
                )));
            }
            let bytes = memory
                .read(address, table.layout.bytes)
                .map_err(|_| Error::Export)?;
            table
                .layout
                .functions
                .iter()
                .map(|function| {
                    u64::from_le_bytes(std::array::from_fn(|i| bytes[function.offset + i]))
                })
                .collect::<Vec<_>>()
        };
        let entries = table
            .layout
            .functions
            .iter()
            .zip(targets)
            .map(|(function, address)| {
                Export::bind(
                    &self.process,
                    self.abi,
                    address,
                    function.parameters,
                    function.result,
                    None,
                )
                .map(Some)
            })
            .collect::<Result<Vec<_>, Error>>()?;
        let first = table.first;
        self.exports[first..first + entries.len()].copy_from_slice(&entries);
        if let Some(table) = &mut self.table {
            table.address = Some(address);
        }
        Ok(())
    }
    /// Engine-readable ABI data stays in the same stopped-child memory.
    pub fn table_address(&self) -> Option<u64> {
        self.table.as_ref().and_then(|table| table.address)
    }
}
