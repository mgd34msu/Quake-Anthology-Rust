//! Quake Live native game-import (engine service) table.
//!
//! Donor: `src/compat/q3/native/imports.ts` — bridges the 206-slot engine
//! import table from ioquakelive `g_public.h` onto a headless synthetic
//! engine/cvar/configstring host. Unknown slots fail explicitly; no actor id
//! is ever constructed from a guest client number.

use std::collections::HashMap;

use qa_guest::core::contracts::{
    GuestAccess, GuestAddress, GuestAllocationOptions, GuestCallContext, GuestCallResult, GuestCallSignature,
    GuestCallValue, GuestPermissions, GuestStorage, NativeAbi,
};
use qa_guest::core::memory::SparseGuestMemory;
use qa_guest::error::GuestError;
use thiserror::Error;

use super::module::{call_integer, call_pointer, call_required_pointer, native_q3_signature};
use super::syscall::TrapAddressAllocator;

/// Engine service slot count from ioquakelive `g_public.h`.
pub const Q3_IMPORT_SLOT_COUNT: usize = 206;

/// `vmCvar_t` byte length shared by both guest widths.
pub const VM_CVAR_STATE_BYTES: usize = 272;

/// `MAX_CVAR_VALUE_STRING`, including the terminator.
pub const MAX_CVAR_VALUE_STRING: usize = 256;

/// Upper bound for reading a guest NUL-terminated string.
const GUEST_STRING_LIMIT: usize = 1_048_576;

/// Failure serving a Quake Live engine import.
#[derive(Debug, Clone, PartialEq, Error)]
pub enum Q3ImportError {
    /// Import slot has no implementation in the supplied engine host.
    #[error("Quake Live game import {slot} ({service}) is unavailable in the supplied engine host")]
    ServiceUnavailable {
        /// Import slot.
        slot: usize,
        /// Service name.
        service: String,
        /// Call that reached the slot.
        context: GuestCallContext,
    },
    /// Engine host reported a game error.
    #[error("{0}")]
    Game(String),
    /// Import slot is outside the table.
    #[error("Quake Live import slot outside table")]
    SlotOutOfRange(usize),
    /// String output needs a positive capacity.
    #[error("Native string output requires positive source capacity")]
    BadCapacity,
    /// Cvar value does not fit `MAX_CVAR_VALUE_STRING`.
    #[error("Cvar_Update: source value exceeds MAX_CVAR_VALUE_STRING")]
    CvarValueTooLong,
    /// Guest string has no terminator within the read limit.
    #[error("Guest string has no terminator within the read limit")]
    UnterminatedString,
    /// Underlying guest memory failure.
    #[error(transparent)]
    Guest(#[from] GuestError),
}

/// Coverage record for one import slot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeQ3ServiceCoverage {
    /// Import slot.
    pub slot: usize,
    /// Service name.
    pub name: String,
    /// Whether the slot has an implementation.
    pub implemented: bool,
    /// Times the slot was reached.
    pub reached: u64,
}

/// Live cvar values exposed to the virtual machine.
#[derive(Debug, Clone, PartialEq)]
pub struct VmCvarValue {
    /// Modification count.
    pub modification_count: i32,
    /// String value.
    pub value: String,
    /// Floating-point interpretation.
    pub numeric_value: f32,
    /// Integer interpretation.
    pub integer_value: i32,
}

/// Engine side of the Quake Live game-import table: console, clients,
/// cvars, configstrings, and command arguments.
pub trait Q3EngineServices {
    /// Queue a console command.
    fn append_console_command(&mut self, command: &str);
    /// Print engine text.
    fn print(&mut self, text: &str);
    /// Drop a client with a reason. The client number stays a number.
    fn drop_client(&mut self, client: i32, reason: &str);
    /// Send a server command to a client (`-1` broadcasts).
    fn send_server_command(&mut self, client: i32, command: &str);
    /// Userinfo string for a client.
    fn userinfo(&self, client: i32) -> String;
    /// Set userinfo for a client.
    fn set_userinfo(&mut self, client: i32, info: &str);
    /// Floating-point value of a cvar (`0.0` when unset).
    fn cvar_variable_value(&self, name: &str) -> f32;
    /// String value of a cvar (empty when unset).
    fn cvar_variable_string(&self, name: &str) -> String;
    /// Set a cvar, bumping its modification count on change.
    fn cvar_set(&mut self, name: &str, value: &str);
    /// Register a VM cvar handle, creating the cvar from `default` when new.
    fn cvar_bind_vm(&mut self, name: &str, default: &str, flags: i32) -> i32;
    /// Read a VM cvar handle, or `None` when unbound.
    fn cvar_read_vm(&self, handle: i32) -> Option<VmCvarValue>;
    /// Set a configstring.
    fn configstring_set(&mut self, index: i32, value: &str);
    /// Get a configstring (empty when unset).
    fn configstring_get(&self, index: i32) -> String;
    /// Command arguments for `Argc`/`Argv`.
    fn command_arguments(&self) -> &[String];
}

/// One synthetic cvar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyntheticCvar {
    /// String value.
    pub value: String,
    /// Modification count.
    pub modification_count: i32,
}

/// Headless engine host: records calls and serves cvars, configstrings,
/// userinfo, and argv from plain maps.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyntheticQ3Host {
    /// Queued console commands.
    pub console_commands: Vec<String>,
    /// Printed engine text.
    pub prints: Vec<String>,
    /// Dropped clients with reasons.
    pub dropped_clients: Vec<(i32, String)>,
    /// Server commands by client.
    pub server_commands: Vec<(i32, String)>,
    /// Userinfo by client number.
    pub userinfo: HashMap<i32, String>,
    /// Cvars by name.
    pub cvars: HashMap<String, SyntheticCvar>,
    /// VM handle to cvar name.
    pub vm_handles: HashMap<i32, String>,
    /// Next VM handle to mint.
    pub next_vm_handle: i32,
    /// Configstrings by index.
    pub configstrings: HashMap<i32, String>,
    /// Command arguments.
    pub argv: Vec<String>,
}

impl SyntheticQ3Host {
    /// Fresh host with the given command arguments.
    #[must_use]
    pub fn new(argv: Vec<String>) -> Self {
        Self {
            console_commands: Vec::new(),
            prints: Vec::new(),
            dropped_clients: Vec::new(),
            server_commands: Vec::new(),
            userinfo: HashMap::new(),
            cvars: HashMap::new(),
            vm_handles: HashMap::new(),
            next_vm_handle: 1,
            configstrings: HashMap::new(),
            argv,
        }
    }

    /// Add a cvar with modification count zero.
    #[must_use]
    pub fn with_cvar(mut self, name: &str, value: &str) -> Self {
        self.cvars.insert(
            name.to_string(),
            SyntheticCvar {
                value: value.to_string(),
                modification_count: 0,
            },
        );
        self
    }

    fn numeric(value: &str) -> f32 {
        value.parse::<f32>().unwrap_or(0.0)
    }

    fn integer(value: &str) -> i32 {
        value.parse::<i32>().unwrap_or_else(|_| Self::numeric(value) as i32)
    }
}

impl Q3EngineServices for SyntheticQ3Host {
    fn append_console_command(&mut self, command: &str) {
        self.console_commands.push(command.to_string());
    }

    fn print(&mut self, text: &str) {
        self.prints.push(text.to_string());
    }

    fn drop_client(&mut self, client: i32, reason: &str) {
        self.dropped_clients.push((client, reason.to_string()));
    }

    fn send_server_command(&mut self, client: i32, command: &str) {
        self.server_commands.push((client, command.to_string()));
    }

    fn userinfo(&self, client: i32) -> String {
        self.userinfo.get(&client).cloned().unwrap_or_default()
    }

    fn set_userinfo(&mut self, client: i32, info: &str) {
        self.userinfo.insert(client, info.to_string());
    }

    fn cvar_variable_value(&self, name: &str) -> f32 {
        self.cvars.get(name).map_or(0.0, |cvar| Self::numeric(&cvar.value))
    }

    fn cvar_variable_string(&self, name: &str) -> String {
        self.cvars.get(name).map_or(String::new(), |cvar| cvar.value.clone())
    }

    fn cvar_set(&mut self, name: &str, value: &str) {
        let cvar = self.cvars.entry(name.to_string()).or_insert_with(|| SyntheticCvar {
            value: String::new(),
            modification_count: 0,
        });
        if cvar.value != value {
            cvar.value = value.to_string();
            cvar.modification_count += 1;
        }
    }

    fn cvar_bind_vm(&mut self, name: &str, default: &str, _flags: i32) -> i32 {
        if let Some(handle) = self
            .vm_handles
            .iter()
            .find_map(|(handle, bound)| (*bound == name).then_some(*handle))
        {
            return handle;
        }
        self.cvars.entry(name.to_string()).or_insert_with(|| SyntheticCvar {
            value: default.to_string(),
            modification_count: 0,
        });
        let handle = self.next_vm_handle;
        self.next_vm_handle += 1;
        self.vm_handles.insert(handle, name.to_string());
        handle
    }

    fn cvar_read_vm(&self, handle: i32) -> Option<VmCvarValue> {
        let cvar = self.cvars.get(self.vm_handles.get(&handle)?)?;
        Some(VmCvarValue {
            modification_count: cvar.modification_count,
            value: cvar.value.clone(),
            numeric_value: Self::numeric(&cvar.value),
            integer_value: Self::integer(&cvar.value),
        })
    }

    fn configstring_set(&mut self, index: i32, value: &str) {
        self.configstrings.insert(index, value.to_string());
    }

    fn configstring_get(&self, index: i32) -> String {
        self.configstrings.get(&index).cloned().unwrap_or_default()
    }

    fn command_arguments(&self) -> &[String] {
        &self.argv
    }
}

/// One import-slot service implementation.
pub type ImportHandler<H> = Box<
    dyn Fn(
        &GuestCallContext,
        &[GuestCallValue],
        &mut SparseGuestMemory,
        &mut H,
    ) -> Result<GuestCallResult, Q3ImportError>,
>;

struct ImportSlot<H> {
    slot: usize,
    name: String,
    parameters: Vec<GuestStorage>,
    result: Option<GuestStorage>,
    implemented: bool,
    reached: u64,
    trap: GuestAddress,
    handler: ImportHandler<H>,
}

/// Quake Live game-import table over a headless engine host.
pub struct QuakeLiveGameImports<H: Q3EngineServices> {
    memory: SparseGuestMemory,
    abi: NativeAbi,
    host: H,
    address: GuestAddress,
    traps: TrapAddressAllocator,
    slots: Vec<ImportSlot<H>>,
}

impl<H: Q3EngineServices> QuakeLiveGameImports<H> {
    /// Allocate the import table and bind every implemented service.
    pub fn new(mut memory: SparseGuestMemory, abi: NativeAbi, host: H) -> Result<Self, Q3ImportError> {
        if memory.pointer_bytes() != abi.pointer_bytes() {
            return Err(Q3ImportError::Game(
                "Quake Live import table ABI width differs from guest memory width".to_string(),
            ));
        }
        let space = memory.address_space();
        let address = memory.allocate(&GuestAllocationOptions {
            byte_length: Q3_IMPORT_SLOT_COUNT * abi.pointer_bytes(),
            alignment: 8,
            permissions: GuestPermissions::ReadWrite,
            label: "Quake Live game imports".to_string(),
        })?;
        let mut imports = Self {
            memory,
            abi,
            host,
            address,
            traps: TrapAddressAllocator::new(space, "ql-game-imports"),
            slots: Vec::with_capacity(Q3_IMPORT_SLOT_COUNT),
        };
        for slot in 0..Q3_IMPORT_SLOT_COUNT {
            let label = format!("unbound-{slot}");
            let service = label.clone();
            imports.bind(
                slot,
                &label,
                Vec::new(),
                None,
                Box::new(move |context, _args, _memory, _host| {
                    Err(Q3ImportError::ServiceUnavailable {
                        slot,
                        service: service.clone(),
                        context: context.clone(),
                    })
                }),
                false,
            )?;
        }
        imports.bind(
            0,
            "SendConsoleCommand",
            vec![GuestStorage::Pointer],
            None,
            Box::new(|_context, args, memory, host| {
                let text = Self::read_string(memory, call_required_pointer(args, 0)?)?;
                host.append_console_command(&text);
                Ok(GuestCallResult::Void)
            }),
            true,
        )?;
        // The shipped G_Printf formats into its own buffer before this call.
        // Its x64 call site passes only the buffer, without a variadic count.
        imports.bind(
            2,
            "Printf",
            vec![GuestStorage::Pointer],
            None,
            Box::new(|_context, args, memory, host| {
                let text = Self::read_string(memory, call_required_pointer(args, 0)?)?;
                host.print(&text);
                Ok(GuestCallResult::Void)
            }),
            true,
        )?;
        imports.bind(
            11,
            "Cvar_VariableValue",
            vec![GuestStorage::Pointer],
            Some(GuestStorage::Float32),
            Box::new(|_context, args, memory, host| {
                let text = Self::read_string(memory, call_required_pointer(args, 0)?)?;
                Ok(GuestCallResult::Value(GuestCallValue::Float32(
                    host.cvar_variable_value(&text),
                )))
            }),
            true,
        )?;
        imports.bind(
            12,
            "Cvar_Update",
            vec![GuestStorage::Pointer],
            None,
            Box::new(|_context, args, memory, host| {
                Self::update_cvar(memory, host, call_required_pointer(args, 0)?)?;
                Ok(GuestCallResult::Void)
            }),
            true,
        )?;
        imports.bind(
            13,
            "Cvar_VariableStringBuffer",
            vec![GuestStorage::Pointer, GuestStorage::Pointer, GuestStorage::Int32],
            None,
            Box::new(|_context, args, memory, host| {
                let name = Self::read_string(memory, call_required_pointer(args, 0)?)?;
                let output = call_required_pointer(args, 1)?;
                let capacity = call_integer(args, 2)?;
                Self::write_string(memory, output, &host.cvar_variable_string(&name), capacity)?;
                Ok(GuestCallResult::Void)
            }),
            true,
        )?;
        imports.bind(
            15,
            "Cvar_Set",
            vec![GuestStorage::Pointer, GuestStorage::Pointer],
            None,
            Box::new(|_context, args, memory, host| {
                let name = Self::read_string(memory, call_required_pointer(args, 0)?)?;
                let value = Self::read_string(memory, call_required_pointer(args, 1)?)?;
                host.cvar_set(&name, &value);
                Ok(GuestCallResult::Void)
            }),
            true,
        )?;
        imports.bind(
            17,
            "Cvar_Register",
            vec![
                GuestStorage::Pointer,
                GuestStorage::Pointer,
                GuestStorage::Pointer,
                GuestStorage::Int32,
            ],
            None,
            Box::new(|_context, args, memory, host| {
                let address = call_pointer(args, 0)?;
                let name = Self::read_string(memory, call_required_pointer(args, 1)?)?;
                let default = Self::read_string(memory, call_required_pointer(args, 2)?)?;
                let handle = host.cvar_bind_vm(&name, &default, call_integer(args, 3)? as i32);
                if let Some(address) = address {
                    memory.write_i32(address, handle)?;
                    memory.write_i32(memory.offset(address, 4)?, -1)?;
                    Self::update_cvar(memory, host, address)?;
                }
                Ok(GuestCallResult::Void)
            }),
            true,
        )?;
        imports.bind(
            18,
            "Argv",
            vec![GuestStorage::Int32, GuestStorage::Pointer, GuestStorage::Int32],
            None,
            Box::new(|_context, args, memory, host| {
                let index = call_integer(args, 0)?;
                let output = call_required_pointer(args, 1)?;
                let capacity = call_integer(args, 2)?;
                let argv = host.command_arguments();
                let value = if index >= 0 {
                    argv.get(index as usize).map(String::as_str).unwrap_or("")
                } else {
                    ""
                };
                Self::write_string(memory, output, value, capacity)?;
                Ok(GuestCallResult::Void)
            }),
            true,
        )?;
        imports.bind(
            20,
            "Argc",
            Vec::new(),
            Some(GuestStorage::Int32),
            Box::new(|_context, _args, _memory, host| {
                Ok(GuestCallResult::Value(GuestCallValue::Int32(
                    host.command_arguments().len() as i32,
                )))
            }),
            true,
        )?;
        imports.bind(
            23,
            "DropClient",
            vec![GuestStorage::Int32, GuestStorage::Pointer],
            None,
            Box::new(|_context, args, memory, host| {
                let client = call_integer(args, 0)? as i32;
                let reason = Self::read_string(memory, call_required_pointer(args, 1)?)?;
                host.drop_client(client, &reason);
                Ok(GuestCallResult::Void)
            }),
            true,
        )?;
        imports.bind(
            24,
            "SendServerCommand",
            vec![GuestStorage::Int32, GuestStorage::Pointer],
            None,
            Box::new(|_context, args, memory, host| {
                let client = call_integer(args, 0)? as i32;
                let command = Self::read_string(memory, call_required_pointer(args, 1)?)?;
                host.send_server_command(client, &command);
                Ok(GuestCallResult::Void)
            }),
            true,
        )?;
        imports.bind(
            25,
            "SetConfigstring",
            vec![GuestStorage::Int32, GuestStorage::Pointer],
            None,
            Box::new(|_context, args, memory, host| {
                let index = call_integer(args, 0)? as i32;
                let value = Self::read_string(memory, call_required_pointer(args, 1)?)?;
                host.configstring_set(index, &value);
                Ok(GuestCallResult::Void)
            }),
            true,
        )?;
        imports.bind(
            26,
            "GetConfigstring",
            vec![GuestStorage::Int32, GuestStorage::Pointer, GuestStorage::Int32],
            None,
            Box::new(|_context, args, memory, host| {
                let index = call_integer(args, 0)? as i32;
                let output = call_required_pointer(args, 1)?;
                let capacity = call_integer(args, 2)?;
                Self::write_string(memory, output, &host.configstring_get(index), capacity)?;
                Ok(GuestCallResult::Void)
            }),
            true,
        )?;
        imports.bind(
            28,
            "GetUserinfo",
            vec![GuestStorage::Int32, GuestStorage::Pointer, GuestStorage::Int32],
            None,
            Box::new(|_context, args, memory, host| {
                let client = call_integer(args, 0)? as i32;
                let output = call_required_pointer(args, 1)?;
                let capacity = call_integer(args, 2)?;
                Self::write_string(memory, output, &host.userinfo(client), capacity)?;
                Ok(GuestCallResult::Void)
            }),
            true,
        )?;
        imports.bind(
            29,
            "SetUserinfo",
            vec![GuestStorage::Int32, GuestStorage::Pointer],
            None,
            Box::new(|_context, args, memory, host| {
                let client = call_integer(args, 0)? as i32;
                let info = Self::read_string(memory, call_required_pointer(args, 1)?)?;
                host.set_userinfo(client, &info);
                Ok(GuestCallResult::Void)
            }),
            true,
        )?;
        Ok(imports)
    }

    /// Bind `slot` to `handler`, writing a fresh trap address into the guest
    /// table. Rebinding resets the slot's reached count.
    pub fn bind(
        &mut self,
        slot: usize,
        name: &str,
        parameters: Vec<GuestStorage>,
        result: Option<GuestStorage>,
        handler: ImportHandler<H>,
        implemented: bool,
    ) -> Result<GuestAddress, Q3ImportError> {
        if slot >= Q3_IMPORT_SLOT_COUNT {
            return Err(Q3ImportError::SlotOutOfRange(slot));
        }
        let trap = self.traps.bind();
        let at = self
            .memory
            .offset(self.address, (slot * self.abi.pointer_bytes()) as i64)?;
        self.memory.write_pointer(at, Some(trap))?;
        let record = ImportSlot {
            slot,
            name: name.to_string(),
            parameters,
            result,
            implemented,
            reached: 0,
            trap,
            handler,
        };
        if slot == self.slots.len() {
            self.slots.push(record);
        } else {
            // Construction binds slots in order, so any other in-range slot
            // is already bound and this is a rebinding.
            self.slots[slot] = record;
        }
        Ok(trap)
    }

    /// Invoke `slot` with guest arguments, counting the call.
    pub fn invoke_slot(
        &mut self,
        context: &GuestCallContext,
        slot: usize,
        args: &[GuestCallValue],
    ) -> Result<GuestCallResult, Q3ImportError> {
        if slot >= self.slots.len() {
            return Err(Q3ImportError::SlotOutOfRange(slot));
        }
        let Self {
            slots, memory, host, ..
        } = self;
        let record = &mut slots[slot];
        record.reached += 1;
        (record.handler)(context, args, memory, host)
    }

    /// Coverage snapshot over all slots.
    #[must_use]
    pub fn coverage(&self) -> Vec<NativeQ3ServiceCoverage> {
        self.slots
            .iter()
            .map(|record| NativeQ3ServiceCoverage {
                slot: record.slot,
                name: record.name.clone(),
                implemented: record.implemented,
                reached: record.reached,
            })
            .collect()
    }

    /// Guest import-table base address.
    #[must_use]
    pub fn address(&self) -> GuestAddress {
        self.address
    }

    /// Table ABI.
    #[must_use]
    pub fn abi(&self) -> NativeAbi {
        self.abi
    }

    /// Call signature bound for `slot`, or `None` when out of range.
    #[must_use]
    pub fn slot_signature(&self, slot: usize) -> Option<GuestCallSignature> {
        self.slots
            .get(slot)
            .map(|record| native_q3_signature(self.abi, &record.parameters, record.result, false))
    }

    /// Trap address written into the table for `slot`.
    #[must_use]
    pub fn slot_trap(&self, slot: usize) -> Option<GuestAddress> {
        self.slots.get(slot).map(|record| record.trap)
    }

    /// Shared guest memory.
    #[must_use]
    pub fn memory(&self) -> &SparseGuestMemory {
        &self.memory
    }

    /// Shared guest memory for script setup and assertions.
    pub fn memory_mut(&mut self) -> &mut SparseGuestMemory {
        &mut self.memory
    }

    /// Engine host.
    #[must_use]
    pub fn host(&self) -> &H {
        &self.host
    }

    /// Engine host for assertions.
    #[must_use]
    pub fn host_mut(&mut self) -> &mut H {
        &mut self.host
    }

    /// Copy `value` into a `capacity`-byte guest buffer, truncating to
    /// `capacity - 1` units and always terminating.
    pub fn write_string(
        memory: &mut SparseGuestMemory,
        address: GuestAddress,
        value: &str,
        capacity: i64,
    ) -> Result<(), Q3ImportError> {
        if capacity < 1 {
            return Err(Q3ImportError::BadCapacity);
        }
        let capacity = capacity as usize;
        memory.check(address, capacity, GuestAccess::Write)?;
        let mut bytes = vec![0u8; capacity];
        for (index, unit) in value.encode_utf16().take(capacity - 1).enumerate() {
            bytes[index] = (unit & 0xFF) as u8;
        }
        memory.write(address, &bytes)?;
        Ok(())
    }

    fn read_string(memory: &mut SparseGuestMemory, address: GuestAddress) -> Result<String, Q3ImportError> {
        let length = memory.find_zero(address, GUEST_STRING_LIMIT)?;
        if length < 0 {
            return Err(Q3ImportError::UnterminatedString);
        }
        let bytes = memory.copy(address, length as usize)?;
        Ok(bytes.iter().map(|byte| *byte as char).collect())
    }

    fn update_cvar(memory: &mut SparseGuestMemory, host: &mut H, address: GuestAddress) -> Result<(), Q3ImportError> {
        let handle = memory.read_i32(address)?;
        let cached = memory.read_i32(memory.offset(address, 4)?)?;
        let Some(value) = host.cvar_read_vm(handle) else {
            return Ok(());
        };
        if cached == value.modification_count {
            return Ok(());
        }
        // vmCvar_t has the same int/int/float/int/char[256] ABI on both
        // guest widths.
        memory.check(address, VM_CVAR_STATE_BYTES, GuestAccess::Write)?;
        memory.write_i32(memory.offset(address, 4)?, value.modification_count)?;
        if value.value.encode_utf16().count() >= MAX_CVAR_VALUE_STRING {
            return Err(Q3ImportError::CvarValueTooLong);
        }
        Self::write_string(
            memory,
            memory.offset(address, 16)?,
            &value.value,
            MAX_CVAR_VALUE_STRING as i64,
        )?;
        memory.write_f32(memory.offset(address, 8)?, value.numeric_value)?;
        memory.write_i32(memory.offset(address, 12)?, value.integer_value)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use qa_core::identity::ProviderId;
    use qa_guest::core::contracts::{CallbackId, ContentDigest, GuestCallbackReference, ModuleIdentity};

    use super::*;

    fn test_identity() -> ModuleIdentity {
        ModuleIdentity::new(
            ProviderId::new("test", "q3-imports"),
            "test-engine.so",
            ContentDigest::new("sha256", "abc"),
            "rev",
        )
    }

    fn test_context() -> GuestCallContext {
        GuestCallContext {
            module: test_identity(),
            callback: GuestCallbackReference::TypeScript {
                provider: ProviderId::new("test", "root"),
                callback: CallbackId::new("test", "root"),
            },
            parent: None,
            itself: None,
            other: None,
        }
    }

    fn write_cstring(memory: &mut SparseGuestMemory, text: &str) -> GuestAddress {
        let address = memory
            .allocate(&GuestAllocationOptions {
                byte_length: text.len() + 1,
                alignment: 1,
                permissions: GuestPermissions::ReadWrite,
                label: "test cstring".to_string(),
            })
            .expect("allocate cstring");
        let mut bytes = text.as_bytes().to_vec();
        bytes.push(0);
        memory.write(address, &bytes).expect("write cstring");
        address
    }

    fn read_cstring(memory: &mut SparseGuestMemory, address: GuestAddress, capacity: usize) -> String {
        let bytes = memory.copy(address, capacity).expect("copy cstring");
        let end = bytes.iter().position(|byte| *byte == 0).unwrap_or(capacity);
        bytes[..end].iter().map(|byte| *byte as char).collect()
    }

    fn allocate_bytes(memory: &mut SparseGuestMemory, byte_length: usize) -> GuestAddress {
        memory
            .allocate(&GuestAllocationOptions {
                byte_length,
                alignment: 8,
                permissions: GuestPermissions::ReadWrite,
                label: "test buffer".to_string(),
            })
            .expect("allocate buffer")
    }

    #[test]
    fn console_and_print_services_reach_host() {
        let memory = SparseGuestMemory::new(test_identity(), 8, 0x10000).expect("memory");
        let host = SyntheticQ3Host::new(vec!["q3".to_string(), "+map".to_string()]);
        let mut imports = QuakeLiveGameImports::new(memory, NativeAbi::LinuxX86_64, host).expect("imports");
        let context = test_context();
        let message = write_cstring(imports.memory_mut(), "fragged");
        let args = [GuestCallValue::Pointer(Some(message))];
        assert_eq!(
            imports.invoke_slot(&context, 2, &args).expect("printf"),
            GuestCallResult::Void
        );
        assert_eq!(
            imports.invoke_slot(&context, 0, &args).expect("console"),
            GuestCallResult::Void
        );
        assert_eq!(imports.host().prints, vec!["fragged".to_string()]);
        assert_eq!(imports.host().console_commands, vec!["fragged".to_string()]);

        let coverage = imports.coverage();
        assert_eq!(coverage.len(), Q3_IMPORT_SLOT_COUNT);
        assert!(coverage[0].implemented);
        assert!(!coverage[1].implemented);
        assert_eq!(coverage[2].reached, 1);
        assert_eq!(coverage[0].name, "SendConsoleCommand");
        assert!(matches!(
            imports.invoke_slot(&context, 1, &[]),
            Err(Q3ImportError::ServiceUnavailable { slot: 1, .. })
        ));
        assert!(matches!(
            imports.invoke_slot(&context, Q3_IMPORT_SLOT_COUNT, &[]),
            Err(Q3ImportError::SlotOutOfRange(_))
        ));

        let trap = imports.slot_trap(2).expect("slot trap");
        let at = imports.memory().offset(imports.address(), 2 * 8).expect("slot address");
        assert_eq!(imports.memory_mut().read_pointer(at).expect("table entry"), Some(trap));
        let signature = imports.slot_signature(11).expect("slot signature");
        assert_eq!(signature.parameters.len(), 1);
        assert!(signature.result.is_some());
    }

    #[test]
    fn cvar_register_update_and_query_roundtrip() {
        let memory = SparseGuestMemory::new(test_identity(), 8, 0x10000).expect("memory");
        let host = SyntheticQ3Host::new(Vec::new()).with_cvar("g_gravity", "800");
        let mut imports = QuakeLiveGameImports::new(memory, NativeAbi::LinuxX86_64, host).expect("imports");
        let context = test_context();
        let state = allocate_bytes(imports.memory_mut(), VM_CVAR_STATE_BYTES);
        let name = write_cstring(imports.memory_mut(), "g_gravity");
        let default = write_cstring(imports.memory_mut(), "800");
        imports
            .invoke_slot(
                &context,
                17,
                &[
                    GuestCallValue::Pointer(Some(state)),
                    GuestCallValue::Pointer(Some(name)),
                    GuestCallValue::Pointer(Some(default)),
                    GuestCallValue::Int32(0),
                ],
            )
            .expect("register");
        let handle = imports.memory_mut().read_i32(state).expect("handle");
        assert!(handle >= 1);
        let modcount_at = imports.memory().offset(state, 4).expect("modcount");
        assert_eq!(imports.memory_mut().read_i32(modcount_at).expect("modcount"), 0);
        let numeric_at = imports.memory().offset(state, 8).expect("numeric");
        assert_eq!(imports.memory_mut().read_f32(numeric_at).expect("numeric"), 800.0);
        let integer_at = imports.memory().offset(state, 12).expect("integer");
        assert_eq!(imports.memory_mut().read_i32(integer_at).expect("integer"), 800);
        let text_at = imports.memory().offset(state, 16).expect("text");
        assert_eq!(read_cstring(imports.memory_mut(), text_at, 256), "800");

        let value_args = [GuestCallValue::Pointer(Some(name))];
        assert_eq!(
            imports.invoke_slot(&context, 11, &value_args).expect("value"),
            GuestCallResult::Value(GuestCallValue::Float32(800.0))
        );
        imports.host_mut().cvar_set("g_gravity", "100");
        imports
            .invoke_slot(&context, 12, &[GuestCallValue::Pointer(Some(state))])
            .expect("update");
        let modcount_at = imports.memory().offset(state, 4).expect("modcount");
        assert_eq!(imports.memory_mut().read_i32(modcount_at).expect("modcount"), 1);
        let text_at = imports.memory().offset(state, 16).expect("text");
        assert_eq!(read_cstring(imports.memory_mut(), text_at, 256), "100");

        let buffer = allocate_bytes(imports.memory_mut(), 8);
        imports
            .invoke_slot(
                &context,
                13,
                &[
                    GuestCallValue::Pointer(Some(name)),
                    GuestCallValue::Pointer(Some(buffer)),
                    GuestCallValue::Int32(8),
                ],
            )
            .expect("string buffer");
        assert_eq!(read_cstring(imports.memory_mut(), buffer, 8), "100");

        let tiny = allocate_bytes(imports.memory_mut(), 3);
        imports
            .invoke_slot(
                &context,
                13,
                &[
                    GuestCallValue::Pointer(Some(name)),
                    GuestCallValue::Pointer(Some(tiny)),
                    GuestCallValue::Int32(3),
                ],
            )
            .expect("truncated buffer");
        assert_eq!(read_cstring(imports.memory_mut(), tiny, 3), "10");
    }

    #[test]
    fn argv_configstring_userinfo_and_client_services() {
        let memory = SparseGuestMemory::new(test_identity(), 8, 0x10000).expect("memory");
        let host = SyntheticQ3Host::new(vec!["q3".to_string(), "+connect".to_string()]);
        let mut imports = QuakeLiveGameImports::new(memory, NativeAbi::LinuxX86_64, host).expect("imports");
        let context = test_context();
        assert_eq!(
            imports.invoke_slot(&context, 20, &[]).expect("argc"),
            GuestCallResult::Value(GuestCallValue::Int32(2))
        );
        let buffer = allocate_bytes(imports.memory_mut(), 16);
        imports
            .invoke_slot(
                &context,
                18,
                &[
                    GuestCallValue::Int32(1),
                    GuestCallValue::Pointer(Some(buffer)),
                    GuestCallValue::Int32(16),
                ],
            )
            .expect("argv");
        assert_eq!(read_cstring(imports.memory_mut(), buffer, 16), "+connect");
        imports
            .invoke_slot(
                &context,
                18,
                &[
                    GuestCallValue::Int32(9),
                    GuestCallValue::Pointer(Some(buffer)),
                    GuestCallValue::Int32(16),
                ],
            )
            .expect("argv missing");
        assert_eq!(read_cstring(imports.memory_mut(), buffer, 16), "");

        let value = write_cstring(imports.memory_mut(), "mp/q3dm1");
        imports
            .invoke_slot(
                &context,
                25,
                &[GuestCallValue::Int32(2), GuestCallValue::Pointer(Some(value))],
            )
            .expect("set configstring");
        let out = allocate_bytes(imports.memory_mut(), 16);
        imports
            .invoke_slot(
                &context,
                26,
                &[
                    GuestCallValue::Int32(2),
                    GuestCallValue::Pointer(Some(out)),
                    GuestCallValue::Int32(16),
                ],
            )
            .expect("get configstring");
        assert_eq!(read_cstring(imports.memory_mut(), out, 16), "mp/q3dm1");

        let info = write_cstring(imports.memory_mut(), "name=grunt");
        imports
            .invoke_slot(
                &context,
                29,
                &[GuestCallValue::Int32(3), GuestCallValue::Pointer(Some(info))],
            )
            .expect("set userinfo");
        imports
            .invoke_slot(
                &context,
                28,
                &[
                    GuestCallValue::Int32(3),
                    GuestCallValue::Pointer(Some(out)),
                    GuestCallValue::Int32(16),
                ],
            )
            .expect("get userinfo");
        assert_eq!(read_cstring(imports.memory_mut(), out, 16), "name=grunt");

        let reason = write_cstring(imports.memory_mut(), "kicked");
        imports
            .invoke_slot(
                &context,
                23,
                &[GuestCallValue::Int32(3), GuestCallValue::Pointer(Some(reason))],
            )
            .expect("drop");
        imports
            .invoke_slot(
                &context,
                24,
                &[GuestCallValue::Int32(-1), GuestCallValue::Pointer(Some(reason))],
            )
            .expect("server command");
        assert_eq!(imports.host().dropped_clients, vec![(3, "kicked".to_string())]);
        assert_eq!(imports.host().server_commands, vec![(-1, "kicked".to_string())]);
    }
}
