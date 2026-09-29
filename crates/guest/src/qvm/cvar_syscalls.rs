//! Cvar trap bridge for game, cgame, and ui modules.
//!
//! Provenance: `src/compat/qvm/cvar-syscalls.ts` (cvar traps from Quake III
//! Arena `sv_game.c`, `cl_cgame.c`, and `cl_ui.c`; `vmCvar_t`
//! registration/update from `qcommon/cvar.c`). [`CvarHost`] is a local
//! mirror of the `CvarRegistry` subset the donor consumes.

use super::client_state::{HostCall, QvmRole, SyscallMemory};
use crate::error::GuestError;

/// Byte length of `vmCvar_t`.
pub const QVM_CVAR_BYTES: usize = 272;
/// Maximum cvar value length accepted by `Cvar_Update`.
pub const QVM_CVAR_VALUE_MAX: usize = 255;

/// Live cvar value observed by the bridge.
#[derive(Debug, Clone)]
pub struct CvarValue {
    /// String value.
    pub value: String,
    /// Numeric value.
    pub numeric_value: f32,
    /// Integer value.
    pub integer_value: i32,
}

/// VM-registered cvar binding observed by the bridge.
#[derive(Debug, Clone)]
pub struct CvarVmBinding {
    /// Modification count.
    pub modification_count: i32,
    /// String value.
    pub value: String,
    /// Numeric value.
    pub numeric_value: f32,
    /// Integer value.
    pub integer_value: i32,
}

/// Host cvar registry surface used by the traps.
pub trait CvarHost {
    /// Bind a VM cvar, returning its handle.
    fn bind_vm(&mut self, name: &str, default: &str, flags: i32) -> i32;
    /// Read a VM cvar binding by handle.
    fn read_vm(&mut self, handle: i32) -> Option<CvarVmBinding>;
    /// Look up a cvar by name.
    fn get(&mut self, name: &str) -> Option<CvarValue>;
    /// Set a cvar value.
    fn set(&mut self, name: &str, value: &str);
    /// Set a cvar numeric value.
    fn set_value(&mut self, name: &str, value: f32);
    /// Reset a cvar to its default.
    fn reset(&mut self, name: &str);
    /// Register a cvar.
    fn register(&mut self, name: &str, default: &str, flags: i32);
    /// Build an info string for the given flags.
    fn info_string(&mut self, flags: i32) -> String;
}

fn update(memory: &mut SyscallMemory, word: i32, cvars: &mut dyn CvarHost) -> Result<(), GuestError> {
    let range = memory.span(word, QVM_CVAR_BYTES, 0)?;
    let source = match cvars.read_vm(memory.read_i32(range.start)?) {
        Some(source) => source,
        None => return Ok(()),
    };
    if source.modification_count == memory.read_i32(range.start + 4)? {
        return Ok(());
    }
    if source.value.len() > QVM_CVAR_VALUE_MAX {
        return Err(GuestError::invalid("Cvar_Update: value exceeds MAX_CVAR_VALUE_STRING"));
    }
    memory.write_i32(range.start + 4, source.modification_count)?;
    memory.fill(range.start + 16, 256, 0)?;
    for (index, byte) in source.value.bytes().enumerate() {
        memory.set(range.start + 16 + index, byte)?;
    }
    memory.write_f32(range.start + 8, source.numeric_value)?;
    memory.write_i32(range.start + 12, source.integer_value)?;
    Ok(())
}

fn register(call: &HostCall, memory: &mut SyscallMemory, cvars: &mut dyn CvarHost) -> Result<(), GuestError> {
    let word = call.int(1)?;
    let handle = cvars.bind_vm(
        &memory.read_string(call.int(2)?)?,
        &memory.read_string(call.int(3)?)?,
        call.int(4)?,
    );
    if memory.pointer(word).is_none() {
        return Ok(());
    }
    let range = memory.span(word, QVM_CVAR_BYTES, 0)?;
    memory.write_i32(range.start, handle)?;
    memory.write_i32(range.start + 4, -1)?;
    update(memory, word, cvars)
}

fn set(call: &HostCall, memory: &mut SyscallMemory, cvars: &mut dyn CvarHost) -> Result<(), GuestError> {
    let name = memory.read_string(call.int(1)?)?;
    let word = call.int(2)?;
    if memory.pointer(word).is_none() {
        cvars.reset(&name);
    } else {
        let value = memory.read_string(word)?;
        cvars.set(&name, &value);
    }
    Ok(())
}

fn variable_string(call: &HostCall, memory: &mut SyscallMemory, cvars: &mut dyn CvarHost) -> Result<(), GuestError> {
    let name = memory.read_string(call.int(1)?)?;
    let word = call.int(2)?;
    match cvars.get(&name) {
        None => {
            let range = memory.span(word, 1, 0)?;
            memory.set(range.start, 0)?;
        }
        Some(source) => memory.write_string(word, &source.value, call.int(3)? as usize)?,
    }
    Ok(())
}

/// Dispatch a cvar trap. Returns `Ok(None)` when the trap belongs to another owner.
pub fn cvar_syscall(
    call: &HostCall,
    memory: &mut SyscallMemory,
    cvars: &mut dyn CvarHost,
) -> Result<Option<i32>, GuestError> {
    if call.role != QvmRole::Ui {
        match call.code {
            3 => {
                register(call, memory, cvars)?;
                return Ok(Some(0));
            }
            4 => {
                update(memory, call.int(1)?, cvars)?;
                return Ok(Some(0));
            }
            5 => {
                set(call, memory, cvars)?;
                return Ok(Some(0));
            }
            6 => {
                if call.role == QvmRole::Cgame {
                    variable_string(call, memory, cvars)?;
                    return Ok(Some(0));
                }
                let name = memory.read_string(call.int(1)?)?;
                return Ok(Some(cvars.get(&name).map_or(0, |source| source.integer_value)));
            }
            7 => {
                if call.role == QvmRole::Cgame {
                    return Ok(None);
                }
                variable_string(call, memory, cvars)?;
                return Ok(Some(0));
            }
            _ => return Ok(None),
        }
    }
    match call.code {
        3 => {
            set(call, memory, cvars)?;
            Ok(Some(0))
        }
        4 => {
            let name = memory.read_string(call.int(1)?)?;
            let value = cvars.get(&name).map_or(0.0, |source| source.numeric_value);
            Ok(Some(value.to_bits() as i32))
        }
        5 => {
            variable_string(call, memory, cvars)?;
            Ok(Some(0))
        }
        6 => {
            let name = memory.read_string(call.int(1)?)?;
            let value = call.float(2)?;
            cvars.set_value(&name, value);
            Ok(Some(0))
        }
        7 => {
            let name = memory.read_string(call.int(1)?)?;
            cvars.reset(&name);
            Ok(Some(0))
        }
        8 => {
            let name = memory.read_string(call.int(1)?)?;
            let default = memory.read_string(call.int(2)?)?;
            let flags = call.int(3)?;
            cvars.register(&name, &default, flags);
            Ok(Some(0))
        }
        9 => {
            let flags = call.int(1)?;
            let word = call.int(2)?;
            let capacity = call.int(3)?;
            let info = cvars.info_string(flags);
            memory.write_string(word, &info, capacity as usize)?;
            Ok(Some(0))
        }
        50 => {
            register(call, memory, cvars)?;
            Ok(Some(0))
        }
        51 => {
            update(memory, call.int(1)?, cvars)?;
            Ok(Some(0))
        }
        _ => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use super::super::client_state::{AbiProfile, CallKind};
    use super::*;
    use std::collections::HashMap;

    struct FakeCvars {
        values: HashMap<String, CvarValue>,
        bound: HashMap<i32, CvarVmBinding>,
        next: i32,
        log: Vec<String>,
    }

    impl FakeCvars {
        fn new() -> Self {
            Self {
                values: HashMap::from([(
                    "sv_fps".to_string(),
                    CvarValue {
                        value: "20".to_string(),
                        numeric_value: 20.0,
                        integer_value: 20,
                    },
                )]),
                bound: HashMap::new(),
                next: 7,
                log: Vec::new(),
            }
        }
    }

    impl CvarHost for FakeCvars {
        fn bind_vm(&mut self, name: &str, default: &str, _flags: i32) -> i32 {
            let handle = self.next;
            self.next += 1;
            let value = self
                .values
                .get(name)
                .map_or(default, |bound| bound.value.as_str())
                .to_string();
            self.bound.insert(
                handle,
                CvarVmBinding {
                    modification_count: 3,
                    value,
                    numeric_value: 20.0,
                    integer_value: 20,
                },
            );
            handle
        }
        fn read_vm(&mut self, handle: i32) -> Option<CvarVmBinding> {
            self.bound.get(&handle).cloned()
        }
        fn get(&mut self, name: &str) -> Option<CvarValue> {
            self.values.get(name).cloned()
        }
        fn set(&mut self, name: &str, value: &str) {
            self.log.push(format!("set {name}={value}"));
            self.values.insert(
                name.to_string(),
                CvarValue {
                    value: value.to_string(),
                    numeric_value: value.parse().unwrap_or(0.0),
                    integer_value: value.parse().unwrap_or(0),
                },
            );
        }
        fn set_value(&mut self, name: &str, value: f32) {
            self.log.push(format!("setvalue {name}={value}"));
        }
        fn reset(&mut self, name: &str) {
            self.log.push(format!("reset {name}"));
        }
        fn register(&mut self, name: &str, default: &str, flags: i32) {
            self.log.push(format!("register {name}={default}:{flags}"));
        }
        fn info_string(&mut self, flags: i32) -> String {
            format!("\\flags\\{flags}")
        }
    }

    fn memory() -> SyscallMemory {
        SyscallMemory::new(4096).unwrap()
    }

    fn words(memory: &mut SyscallMemory, texts: &[(&str, usize)]) {
        for (text, at) in texts {
            memory.write_string(*at as i32, text, text.len() + 1).unwrap();
        }
    }

    fn call(role: QvmRole, code: i32, args: &[i32]) -> HostCall {
        HostCall {
            kind: CallKind::Engine,
            role,
            code,
            words: core::iter::once(code).chain(args.iter().copied()).collect(),
            abi_profile: AbiProfile::Modern,
        }
    }

    #[test]
    fn game_register_writes_record() {
        let mut memory = memory();
        words(&mut memory, &[("sv_fps", 512), ("30", 600)]);
        let mut cvars = FakeCvars::new();
        let result = cvar_syscall(&call(QvmRole::Qagame, 3, &[128, 512, 600, 0]), &mut memory, &mut cvars).unwrap();
        assert_eq!(result, Some(0));
        assert_eq!(memory.read_i32(128).unwrap(), 7);
        assert_eq!(memory.read_i32(132).unwrap(), 3);
        assert_eq!(memory.read_string(128 + 16).unwrap(), "20");
        assert_eq!(memory.read_f32(136).unwrap(), 20.0);
        assert_eq!(memory.read_i32(140).unwrap(), 20);
    }

    #[test]
    fn update_skips_current_and_missing() {
        let mut memory = memory();
        memory.write_i32(128, 999).unwrap();
        memory.write_i32(132, 3).unwrap();
        let mut cvars = FakeCvars::new();
        assert_eq!(
            cvar_syscall(&call(QvmRole::Qagame, 4, &[128]), &mut memory, &mut cvars).unwrap(),
            Some(0)
        );
        assert_eq!(memory.read_i32(132).unwrap(), 3);
    }

    #[test]
    fn game_set_and_reset_paths() {
        let mut memory = memory();
        words(&mut memory, &[("sv_fps", 512), ("40", 600)]);
        let mut cvars = FakeCvars::new();
        assert_eq!(
            cvar_syscall(&call(QvmRole::Qagame, 5, &[512, 600]), &mut memory, &mut cvars).unwrap(),
            Some(0)
        );
        assert_eq!(
            cvar_syscall(&call(QvmRole::Qagame, 5, &[512, 0]), &mut memory, &mut cvars).unwrap(),
            Some(0)
        );
        assert_eq!(cvars.log, vec!["set sv_fps=40".to_string(), "reset sv_fps".to_string()]);
    }

    #[test]
    fn game_integer_value_and_string_buffer() {
        let mut memory = memory();
        words(&mut memory, &[("sv_fps", 512), ("nope", 600)]);
        let mut cvars = FakeCvars::new();
        assert_eq!(
            cvar_syscall(&call(QvmRole::Qagame, 6, &[512]), &mut memory, &mut cvars).unwrap(),
            Some(20)
        );
        assert_eq!(
            cvar_syscall(&call(QvmRole::Qagame, 6, &[600]), &mut memory, &mut cvars).unwrap(),
            Some(0)
        );
        assert_eq!(
            cvar_syscall(&call(QvmRole::Qagame, 7, &[512, 128, 64]), &mut memory, &mut cvars).unwrap(),
            Some(0)
        );
        assert_eq!(memory.read_string(128).unwrap(), "20");
        assert_eq!(
            cvar_syscall(&call(QvmRole::Qagame, 7, &[600, 128, 64]), &mut memory, &mut cvars).unwrap(),
            Some(0)
        );
        assert_eq!(memory.get(128).unwrap(), 0);
    }

    #[test]
    fn cgame_string_buffer_and_unhandled_update() {
        let mut memory = memory();
        words(&mut memory, &[("sv_fps", 512)]);
        let mut cvars = FakeCvars::new();
        assert_eq!(
            cvar_syscall(&call(QvmRole::Cgame, 6, &[512, 128, 64]), &mut memory, &mut cvars).unwrap(),
            Some(0)
        );
        assert_eq!(memory.read_string(128).unwrap(), "20");
        assert_eq!(
            cvar_syscall(&call(QvmRole::Cgame, 7, &[512, 128, 64]), &mut memory, &mut cvars).unwrap(),
            None
        );
        assert_eq!(
            cvar_syscall(&call(QvmRole::Cgame, 8, &[]), &mut memory, &mut cvars).unwrap(),
            None
        );
    }

    #[test]
    fn ui_traps() {
        let mut memory = memory();
        words(&mut memory, &[("sv_fps", 512), ("9", 600)]);
        let mut cvars = FakeCvars::new();
        assert_eq!(
            cvar_syscall(&call(QvmRole::Ui, 3, &[512, 600]), &mut memory, &mut cvars).unwrap(),
            Some(0)
        );
        assert_eq!(
            cvar_syscall(&call(QvmRole::Ui, 4, &[512]), &mut memory, &mut cvars).unwrap(),
            Some(9.0f32.to_bits() as i32)
        );
        assert_eq!(
            cvar_syscall(&call(QvmRole::Ui, 5, &[512, 128, 64]), &mut memory, &mut cvars).unwrap(),
            Some(0)
        );
        assert_eq!(memory.read_string(128).unwrap(), "9");
        assert_eq!(
            cvar_syscall(
                &call(QvmRole::Ui, 6, &[512, 1.5f32.to_bits() as i32]),
                &mut memory,
                &mut cvars
            )
            .unwrap(),
            Some(0)
        );
        assert_eq!(
            cvar_syscall(&call(QvmRole::Ui, 7, &[512]), &mut memory, &mut cvars).unwrap(),
            Some(0)
        );
        assert_eq!(
            cvar_syscall(&call(QvmRole::Ui, 8, &[512, 600, 1]), &mut memory, &mut cvars).unwrap(),
            Some(0)
        );
        assert_eq!(
            cvar_syscall(&call(QvmRole::Ui, 9, &[2, 128, 64]), &mut memory, &mut cvars).unwrap(),
            Some(0)
        );
        assert_eq!(memory.read_string(128).unwrap(), "\\flags\\2");
        assert_eq!(
            cvar_syscall(&call(QvmRole::Ui, 50, &[256, 512, 600, 0]), &mut memory, &mut cvars).unwrap(),
            Some(0)
        );
        assert_eq!(memory.read_i32(256).unwrap(), 7);
        assert_eq!(
            cvar_syscall(&call(QvmRole::Ui, 51, &[256]), &mut memory, &mut cvars).unwrap(),
            Some(0)
        );
        assert_eq!(
            cvar_syscall(&call(QvmRole::Ui, 52, &[]), &mut memory, &mut cvars).unwrap(),
            None
        );
    }
}
