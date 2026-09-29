//! Q2 rerelease core engine imports: strings, cvars, tags, info keys.
//!
//! Donor: `src/compat/q2/rerelease/imports.ts` — bridges the shared engine
//! services (`Com_Print`, `cvar`, `TagMalloc`, ...) into guest imports.

use std::collections::HashMap;

use qa_guest::core::contracts::{GuestAddress, GuestCallResult, GuestCallValue};
use qa_guest::core::memory::SparseGuestMemory;
use qa_guest::GuestError;
use thiserror::Error;

use super::layouts::{cvar_layout, field_offset};

/// Core import failure.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum CoreImportError {
    /// Guest string has no terminator within its limit.
    #[error("Q2 guest string has no terminator within its source limit")]
    Unterminated,
    /// Tag free of an unowned allocation.
    #[error("Q2 TagFree received an unowned or already freed allocation")]
    UnownedFree,
    /// Tag malloc size exceeds the mapped-memory limit.
    #[error("Q2 TagMalloc size exceeds mapped-memory allocation limit")]
    OversizeAlloc,
    /// Invalid info output size.
    #[error("Invalid Q2 info output size")]
    BadInfoSize,
    /// Guest raised a fatal error.
    #[error("Q2 {0}: {1}")]
    GuestFatal(String, String),
    /// Import is not implemented by the core dispatcher.
    #[error("Missing Q2 rerelease {0} import {1}")]
    MissingImport(String, String),
    /// Guest memory failure.
    #[error(transparent)]
    Guest(#[from] GuestError),
}

/// Read a NUL-terminated guest string within a byte limit.
pub fn read_guest_string(
    memory: &mut SparseGuestMemory,
    address: GuestAddress,
    maximum: usize,
) -> Result<String, CoreImportError> {
    let found = memory.find_zero(address, maximum)?;
    if found < 0 {
        return Err(CoreImportError::Unterminated);
    }
    let bytes = memory.copy(address, found as usize)?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

/// Cvar snapshot mirrored into one guest `cvar_t`.
#[derive(Debug, Clone, PartialEq)]
pub struct CvarSnapshot {
    /// Cvar name.
    pub name: String,
    /// Current value.
    pub value: String,
    /// Latched value, if any.
    pub latched_value: Option<String>,
    /// Flags.
    pub flags: u32,
    /// Modification count.
    pub modification_count: i32,
    /// Numeric value.
    pub numeric_value: f32,
    /// Integer value.
    pub integer_value: i32,
}

/// Session cvar registry behind the guest cvar records.
pub trait CvarRegistry {
    /// Look up a cvar.
    fn get(&self, name: &str) -> Option<CvarSnapshot>;
    /// Register (or fetch) a cvar.
    fn register(&mut self, name: &str, value: &str, flags: u32) -> Option<CvarSnapshot>;
    /// Set a cvar; `force` bypasses latch rules.
    fn set(&mut self, name: &str, value: &str, force: bool) -> Option<CvarSnapshot>;
}

/// Engine services behind the core imports.
pub trait RereleaseCoreServices {
    /// Print engine text.
    fn print(&mut self, text: &str);
    /// Fetch a configstring.
    fn get_configstring(&self, index: i32) -> String;
    /// Store a configstring.
    fn set_configstring(&mut self, index: i32, value: &str);
    /// Resolve a resource index.
    fn resource_index(&mut self, kind: &str, name: &str) -> i32;
    /// Current server frame.
    fn server_frame(&self) -> u32;
    /// Command arguments.
    fn command_arguments(&self) -> Vec<String>;
    /// Command tail.
    fn command_tail(&self) -> String;
    /// Queue a command.
    fn add_command(&mut self, text: &str);
    /// Resolve an extension, if present.
    fn extension(&self, name: &str, api: &str) -> Option<GuestAddress>;
    /// Optional override for transport/localization/rendering calls.
    fn invoke_override(&mut self, _api: &str, _name: &str, _args: &[GuestCallValue]) -> Option<GuestCallResult> {
        None
    }
}

struct Allocation {
    size: usize,
    tag: i32,
}

struct GuestCvar {
    address: GuestAddress,
    string: GuestAddress,
    latched_string: GuestAddress,
    flags: GuestAddress,
    modified_count: GuestAddress,
    value: GuestAddress,
    integer: GuestAddress,
}

/// Guest cvar records mirror one session registry; tagged bytes stay
/// source-owned.
pub struct RereleaseCoreImports<C, S> {
    allocations: HashMap<u64, Allocation>,
    strings: HashMap<String, GuestAddress>,
    cvars: HashMap<String, GuestCvar>,
    last_cvar: Option<GuestAddress>,
    /// Session cvar registry.
    pub cvars_registry: C,
    /// Engine services.
    pub services: S,
}

impl<C: CvarRegistry, S: RereleaseCoreServices> RereleaseCoreImports<C, S> {
    /// Create over a registry and services.
    #[must_use]
    pub fn new(cvars_registry: C, services: S) -> Self {
        Self {
            allocations: HashMap::new(),
            strings: HashMap::new(),
            cvars: HashMap::new(),
            last_cvar: None,
            cvars_registry,
            services,
        }
    }

    /// Intern an engine string in guest memory.
    pub fn intern(&mut self, memory: &mut SparseGuestMemory, text: &str) -> Result<GuestAddress, GuestError> {
        if let Some(previous) = self.strings.get(text) {
            return Ok(*previous);
        }
        let bytes = text.as_bytes();
        let address = memory.allocate(&qa_guest::core::contracts::GuestAllocationOptions::bytes(
            bytes.len() + 1,
        ))?;
        memory.write(address, bytes)?;
        memory.write_u8(memory.offset(address, bytes.len() as i64)?, 0)?;
        self.strings.insert(text.to_string(), address);
        Ok(address)
    }

    /// Refresh every guest cvar record from the registry.
    pub fn refresh_cvars(&mut self, memory: &mut SparseGuestMemory) -> Result<(), CoreImportError> {
        let names: Vec<String> = self.cvars.keys().cloned().collect();
        for name in names {
            if let Some(value) = self.cvars_registry.get(&name) {
                self.write_cvar(memory, &name, &value)?;
            }
        }
        Ok(())
    }

    fn write_cvar(
        &mut self,
        memory: &mut SparseGuestMemory,
        name: &str,
        value: &CvarSnapshot,
    ) -> Result<(), CoreImportError> {
        let record = self.cvars.get(name).expect("cvar record");
        let (string, latched, flags, modified, numeric, integer) = (
            record.string,
            record.latched_string,
            record.flags,
            record.modified_count,
            record.value,
            record.integer,
        );
        let text = self.intern(memory, &value.value)?;
        memory.write_pointer(string, Some(text))?;
        let latched_address = match &value.latched_value {
            None => None,
            Some(text) => Some(self.intern(memory, text)?),
        };
        memory.write_pointer(latched, latched_address)?;
        memory.write_u32(flags, value.flags)?;
        memory.write_i32(modified, value.modification_count.max(1))?;
        memory.write_f32(numeric, value.numeric_value)?;
        memory.write_i32(integer, value.integer_value)?;
        Ok(())
    }

    /// Fetch or create the guest record for a cvar snapshot.
    pub fn cvar(
        &mut self,
        memory: &mut SparseGuestMemory,
        value: Option<CvarSnapshot>,
    ) -> Result<Option<GuestAddress>, CoreImportError> {
        let Some(value) = value else {
            return Ok(None);
        };
        if !self.cvars.contains_key(&value.name) {
            let layout = cvar_layout();
            let address = memory.allocate(&qa_guest::core::contracts::GuestAllocationOptions::bytes(
                layout.byte_length,
            ))?;
            let at = |memory: &SparseGuestMemory, name: &str| {
                field_offset(&layout, name)
                    .map_err(|_| CoreImportError::MissingImport("game".to_string(), name.to_string()))
                    .and_then(|offset| memory.offset(address, offset as i64).map_err(CoreImportError::Guest))
            };
            let record = GuestCvar {
                address,
                string: at(memory, "string")?,
                latched_string: at(memory, "latched_string")?,
                flags: at(memory, "flags")?,
                modified_count: at(memory, "modified_count")?,
                value: at(memory, "value")?,
                integer: at(memory, "integer")?,
            };
            let name_address = self.intern(memory, &value.name)?;
            memory.write_pointer(address, Some(name_address))?;
            let next = at(memory, "next")?;
            memory.write_pointer(next, self.last_cvar)?;
            self.last_cvar = Some(address);
            self.cvars.insert(value.name.clone(), record);
        }
        self.write_cvar(memory, &value.name.clone(), &value)?;
        Ok(self.cvars.get(&value.name).map(|record| record.address))
    }

    /// Free one tagged allocation.
    pub fn free(
        &mut self,
        memory: &mut SparseGuestMemory,
        address: Option<GuestAddress>,
    ) -> Result<(), CoreImportError> {
        let Some(address) = address else {
            return Ok(());
        };
        memory.check(address, 1, qa_guest::core::contracts::GuestAccess::Write)?;
        let Some(record) = self.allocations.remove(&address.offset) else {
            return Err(CoreImportError::UnownedFree);
        };
        memory.unmap(address, record.size)?;
        Ok(())
    }

    /// Dispatch one core import.
    pub fn invoke(
        &mut self,
        memory: &mut SparseGuestMemory,
        api: &str,
        name: &str,
        args: &[GuestCallValue],
    ) -> Result<GuestCallResult, CoreImportError> {
        if let Some(supplied) = self.services.invoke_override(api, name, args) {
            return Ok(supplied);
        }
        let pointer = |index: usize| -> Option<GuestAddress> {
            match args.get(index) {
                Some(GuestCallValue::Pointer(address)) => *address,
                _ => None,
            }
        };
        let required = |index: usize| -> Result<GuestAddress, CoreImportError> {
            pointer(index).ok_or(CoreImportError::Unterminated)
        };
        let integer = |index: usize| -> i64 {
            match args.get(index) {
                Some(GuestCallValue::Int32(value)) => i64::from(*value),
                Some(GuestCallValue::Uint32(value)) => i64::from(*value),
                Some(GuestCallValue::Int64(value)) => *value,
                Some(GuestCallValue::Uint64(value)) => *value as i64,
                _ => 0,
            }
        };
        match name {
            "Com_Print" => {
                let text = read_guest_string(memory, required(0)?, 1_048_576)?;
                self.services.print(&text);
                Ok(GuestCallResult::Void)
            }
            "Com_Error" => {
                let text = read_guest_string(memory, required(0)?, 1_048_576)?;
                Err(CoreImportError::GuestFatal(api.to_string(), text))
            }
            "get_configstring" => {
                let value = self.services.get_configstring(integer(0) as i32);
                let address = self.intern(memory, &value)?;
                Ok(GuestCallResult::Value(GuestCallValue::Pointer(Some(address))))
            }
            "configstring" => {
                let value = match pointer(1) {
                    None => String::new(),
                    Some(address) => read_guest_string(memory, address, 1_048_576)?,
                };
                self.services.set_configstring(integer(0) as i32, &value);
                Ok(GuestCallResult::Void)
            }
            "modelindex" | "soundindex" | "imageindex" => {
                let text = read_guest_string(memory, required(0)?, 1_048_576)?;
                let kind = name.strip_suffix("index").unwrap_or(name);
                let index = self.services.resource_index(kind, &text);
                Ok(GuestCallResult::Value(GuestCallValue::Int32(index)))
            }
            "ServerFrame" => Ok(GuestCallResult::Value(GuestCallValue::Uint32(
                self.services.server_frame(),
            ))),
            "argc" => Ok(GuestCallResult::Value(GuestCallValue::Int32(
                self.services.command_arguments().len() as i32,
            ))),
            "argv" => {
                let list = self.services.command_arguments();
                let text = list.get(integer(0) as usize).cloned().unwrap_or_default();
                let address = self.intern(memory, &text)?;
                Ok(GuestCallResult::Value(GuestCallValue::Pointer(Some(address))))
            }
            "args" => {
                let tail = self.services.command_tail();
                let address = self.intern(memory, &tail)?;
                Ok(GuestCallResult::Value(GuestCallValue::Pointer(Some(address))))
            }
            "AddCommandString" => {
                let text = read_guest_string(memory, required(0)?, 1_048_576)?;
                self.services.add_command(&text);
                Ok(GuestCallResult::Void)
            }
            "GetExtension" => {
                let text = read_guest_string(memory, required(0)?, 1_048_576)?;
                let address = self.services.extension(&text, api);
                Ok(GuestCallResult::Value(GuestCallValue::Pointer(address)))
            }
            "Info_ValueForKey" => self.info_value_for_key(memory, args),
            "cvar" => {
                let name_text = read_guest_string(memory, required(0)?, 1_048_576)?;
                let value_text = read_guest_string(memory, required(1)?, 1_048_576)?;
                let snapshot = self.cvars_registry.register(&name_text, &value_text, integer(2) as u32);
                let address = self.cvar(memory, snapshot)?;
                Ok(GuestCallResult::Value(GuestCallValue::Pointer(address)))
            }
            "cvar_set" | "cvar_forceset" => {
                let name_text = read_guest_string(memory, required(0)?, 1_048_576)?;
                let value_text = read_guest_string(memory, required(1)?, 1_048_576)?;
                let snapshot = self
                    .cvars_registry
                    .set(&name_text, &value_text, name == "cvar_forceset");
                let address = self.cvar(memory, snapshot)?;
                Ok(GuestCallResult::Value(GuestCallValue::Pointer(address)))
            }
            "TagMalloc" => {
                let raw = integer(0);
                if !(0..=0x1000_0000).contains(&raw) {
                    return Err(CoreImportError::OversizeAlloc);
                }
                let size = (raw as usize).max(1);
                let tag = integer(1) as i32;
                let address = memory.allocate(&qa_guest::core::contracts::GuestAllocationOptions::bytes(size))?;
                self.allocations.insert(address.offset, Allocation { size, tag });
                Ok(GuestCallResult::Value(GuestCallValue::Pointer(Some(address))))
            }
            "TagFree" => {
                self.free(memory, pointer(0))?;
                Ok(GuestCallResult::Void)
            }
            "FreeTags" => {
                let tag = integer(0) as i32;
                let owned: Vec<u64> = self
                    .allocations
                    .iter()
                    .filter(|(_, record)| record.tag == tag)
                    .map(|(offset, _)| *offset)
                    .collect();
                for offset in owned {
                    let address = GuestAddress::new(memory.address_space(), offset);
                    self.free(memory, Some(address))?;
                }
                Ok(GuestCallResult::Void)
            }
            _ => Err(CoreImportError::MissingImport(api.to_string(), name.to_string())),
        }
    }

    /// Case-sensitive `q2repro` info reader with full source length return.
    fn info_value_for_key(
        &mut self,
        memory: &mut SparseGuestMemory,
        args: &[GuestCallValue],
    ) -> Result<GuestCallResult, CoreImportError> {
        let pointer = |index: usize| -> Option<GuestAddress> {
            match args.get(index) {
                Some(GuestCallValue::Pointer(address)) => *address,
                _ => None,
            }
        };
        let integer = |index: usize| -> i64 {
            match args.get(index) {
                Some(GuestCallValue::Int32(value)) => i64::from(*value),
                Some(GuestCallValue::Uint32(value)) => i64::from(*value),
                Some(GuestCallValue::Int64(value)) => *value,
                Some(GuestCallValue::Uint64(value)) => *value as i64,
                _ => 0,
            }
        };
        let input = pointer(0).ok_or(CoreImportError::Unterminated)?;
        let key_address = pointer(1).ok_or(CoreImportError::Unterminated)?;
        let maximum = integer(3);
        if maximum < 0 {
            return Err(CoreImportError::BadInfoSize);
        }
        let key = read_guest_string(memory, key_address, 1_048_576)?;
        let value = read_guest_string(memory, input, 2048)?;
        let mut cursor = usize::from(value.starts_with('\\'));
        let mut found = "";
        while cursor < value.len() {
            let Some(separator) = value[cursor..].find('\\').map(|at| at + cursor) else {
                break;
            };
            let end = value[separator + 1..]
                .find('\\')
                .map(|at| at + separator + 1)
                .unwrap_or(value.len());
            if value[cursor..separator] == key {
                found = &value[separator + 1..end];
                break;
            }
            cursor = end + 1;
        }
        let bytes = found.as_bytes();
        if maximum != 0 {
            let output = pointer(2).ok_or(CoreImportError::Unterminated)?;
            let count = bytes.len().min(maximum as usize - 1);
            memory.check(output, count + 1, qa_guest::core::contracts::GuestAccess::Write)?;
            memory.write(output, &bytes[..count])?;
            memory.write_u8(memory.offset(output, count as i64)?, 0)?;
        }
        Ok(GuestCallResult::Value(GuestCallValue::Uint64(bytes.len() as u64)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_core::identity::ProviderId;
    use qa_guest::core::contracts::{ContentDigest, GuestAllocationOptions, ModuleIdentity};
    use std::collections::HashMap;

    struct FakeRegistry {
        values: HashMap<String, CvarSnapshot>,
    }

    impl CvarRegistry for FakeRegistry {
        fn get(&self, name: &str) -> Option<CvarSnapshot> {
            self.values.get(name).cloned()
        }
        fn register(&mut self, name: &str, value: &str, flags: u32) -> Option<CvarSnapshot> {
            let snapshot = CvarSnapshot {
                name: name.to_string(),
                value: value.to_string(),
                latched_value: None,
                flags,
                modification_count: 1,
                numeric_value: value.parse().unwrap_or(0.0),
                integer_value: value.parse().unwrap_or(0),
            };
            self.values.insert(name.to_string(), snapshot.clone());
            Some(snapshot)
        }
        fn set(&mut self, name: &str, value: &str, _force: bool) -> Option<CvarSnapshot> {
            self.register(name, value, 0)
        }
    }

    struct FakeServices {
        printed: Vec<String>,
        configstrings: HashMap<i32, String>,
        commands: Vec<String>,
    }

    impl RereleaseCoreServices for FakeServices {
        fn print(&mut self, text: &str) {
            self.printed.push(text.to_string());
        }
        fn get_configstring(&self, index: i32) -> String {
            self.configstrings.get(&index).cloned().unwrap_or_default()
        }
        fn set_configstring(&mut self, index: i32, value: &str) {
            self.configstrings.insert(index, value.to_string());
        }
        fn resource_index(&mut self, kind: &str, name: &str) -> i32 {
            (kind.len() + name.len()) as i32
        }
        fn server_frame(&self) -> u32 {
            44
        }
        fn command_arguments(&self) -> Vec<String> {
            vec!["game".to_string(), "+map".to_string()]
        }
        fn command_tail(&self) -> String {
            "+map base1".to_string()
        }
        fn add_command(&mut self, text: &str) {
            self.commands.push(text.to_string());
        }
        fn extension(&self, _name: &str, _api: &str) -> Option<GuestAddress> {
            None
        }
    }

    fn harness() -> (SparseGuestMemory, RereleaseCoreImports<FakeRegistry, FakeServices>) {
        let module = ModuleIdentity::new(
            ProviderId::new("q2", "imports-test"),
            "game.dll",
            ContentDigest::new("sha256", "00"),
            "test",
        );
        let memory = SparseGuestMemory::new(module, 8, 0x1_0000).expect("memory");
        let imports = RereleaseCoreImports::new(
            FakeRegistry { values: HashMap::new() },
            FakeServices {
                printed: Vec::new(),
                configstrings: HashMap::new(),
                commands: Vec::new(),
            },
        );
        (memory, imports)
    }

    fn write_bytes(memory: &mut SparseGuestMemory, bytes: &[u8]) -> GuestAddress {
        let address = memory
            .allocate(&GuestAllocationOptions::bytes(bytes.len()))
            .expect("alloc");
        memory.write(address, bytes).expect("write");
        address
    }

    #[test]
    fn print_configstring_and_cvar_round_trip() {
        let (mut memory, mut imports) = harness();
        let text = write_bytes(&mut memory, b"hello\0");
        imports
            .invoke(&mut memory, "game", "Com_Print", &[GuestCallValue::Pointer(Some(text))])
            .expect("print");
        assert_eq!(imports.services.printed, vec!["hello".to_string()]);
        let value = write_bytes(&mut memory, b"dm1\0");
        imports
            .invoke(
                &mut memory,
                "game",
                "configstring",
                &[GuestCallValue::Int32(2), GuestCallValue::Pointer(Some(value))],
            )
            .expect("set");
        let fetched = imports
            .invoke(&mut memory, "game", "get_configstring", &[GuestCallValue::Int32(2)])
            .expect("get");
        let GuestCallResult::Value(GuestCallValue::Pointer(Some(address))) = fetched else {
            panic!("pointer result");
        };
        assert_eq!(read_guest_string(&mut memory, address, 64).expect("read"), "dm1");
        let name = write_bytes(&mut memory, b"sv_gravity\0");
        let initial = write_bytes(&mut memory, b"800\0");
        let cvar = imports
            .invoke(
                &mut memory,
                "game",
                "cvar",
                &[
                    GuestCallValue::Pointer(Some(name)),
                    GuestCallValue::Pointer(Some(initial)),
                    GuestCallValue::Uint32(0),
                ],
            )
            .expect("cvar");
        assert!(matches!(cvar, GuestCallResult::Value(GuestCallValue::Pointer(Some(_)))));
        imports.refresh_cvars(&mut memory).expect("refresh");
        let frame = imports.invoke(&mut memory, "game", "ServerFrame", &[]).expect("frame");
        assert_eq!(frame, GuestCallResult::Value(GuestCallValue::Uint32(44)));
    }

    #[test]
    fn tags_allocate_free_and_info_reads() {
        let (mut memory, mut imports) = harness();
        let allocated = imports
            .invoke(
                &mut memory,
                "game",
                "TagMalloc",
                &[GuestCallValue::Uint64(64), GuestCallValue::Int32(765)],
            )
            .expect("malloc");
        let GuestCallResult::Value(GuestCallValue::Pointer(Some(address))) = allocated else {
            panic!("pointer result");
        };
        imports
            .invoke(
                &mut memory,
                "game",
                "TagFree",
                &[GuestCallValue::Pointer(Some(address))],
            )
            .expect("free");
        let stray = memory.allocate(&GuestAllocationOptions::bytes(8)).expect("alloc");
        let again = imports.invoke(&mut memory, "game", "TagFree", &[GuestCallValue::Pointer(Some(stray))]);
        assert_eq!(again.unwrap_err(), CoreImportError::UnownedFree);
        let info = write_bytes(&mut memory, b"\\name\\soldier\\team\\red\0");
        let key = write_bytes(&mut memory, b"team\0");
        let output = memory.allocate(&GuestAllocationOptions::bytes(16)).expect("alloc");
        let length = imports
            .invoke(
                &mut memory,
                "game",
                "Info_ValueForKey",
                &[
                    GuestCallValue::Pointer(Some(info)),
                    GuestCallValue::Pointer(Some(key)),
                    GuestCallValue::Pointer(Some(output)),
                    GuestCallValue::Uint64(16),
                ],
            )
            .expect("info");
        assert_eq!(length, GuestCallResult::Value(GuestCallValue::Uint64(3)));
        assert_eq!(read_guest_string(&mut memory, output, 16).expect("read"), "red");
    }
}
