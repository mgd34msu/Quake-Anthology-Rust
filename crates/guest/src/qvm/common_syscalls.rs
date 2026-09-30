//! Console, argument, and command traps shared by all QVM roles.
//!
//! Port of `src/compat/qvm/common-syscalls.ts` (Q3 `sv_game.c` / `cl_cgame.c` /
//! `cl_ui.c` common traps; GPL-2.0-or-later). Handles print/error,
//! milliseconds, argv/argc/args, console commands, and game `G_REAL_TIME`.
//! `None` means unhandled; zero remains a completed guest result.
//!
//! Local mirrors (the cvar module is owned by another worker):
//!
//! - `QvmCvarServices` mirrors the `QvmCvarServices` surface from
//!   `src/compat/qvm/cvar-syscalls.ts` (`bind_vm`, `read_vm`, `get`, `set`,
//!   `set_value`, `reset`, `register`, `info_string`) so the cvar worker can
//!   implement this exact trait.
//! - `QvmCommonServices::dispatch_cvar` is the seam where the real
//!   `qvmCvarSyscall` plugs in; this file never duplicates its trap table.
//!
//! Sync-port note: the donor console `executeNow` may return a promise; here
//! it is synchronous.

use crate::error::GuestError;

use super::abi::{QvmCgameImport, QvmGameImport, QvmUiImport};
use super::interpreter::{qvm_drop_error, qvm_fatal_error};
use super::syscalls::{QvmHostCall, QvmHostCode, QvmRole, QvmTrapCode};

/// Broken-down calendar time for `G_REAL_TIME`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QvmCalendar {
    /// Seconds.
    pub second: i32,
    /// Minutes.
    pub minute: i32,
    /// Hours.
    pub hour: i32,
    /// Day of month.
    pub day: i32,
    /// Month (0-11).
    pub month: i32,
    /// Year.
    pub year: i32,
    /// Weekday.
    pub weekday: i32,
    /// Day of year.
    pub year_day: i32,
    /// Daylight saving flag.
    pub is_dst: i32,
}

/// One console-variable record read from the registry.
#[derive(Debug, Clone, PartialEq)]
pub struct QvmCvarRecord {
    /// Modification count.
    pub modification_count: i32,
    /// String value.
    pub value: String,
    /// Numeric value.
    pub numeric_value: f32,
    /// Integer value.
    pub integer_value: i32,
}

/// Cvar registry surface used by QVM traps. (Local mirror; the cvar worker
/// owns the implementation.)
pub trait QvmCvarServices {
    /// Bind a VM cvar, returning its handle.
    fn bind_vm(&mut self, name: &str, default: &str, flags: i32) -> i32;
    /// Read a VM cvar by handle.
    fn read_vm(&self, handle: i32) -> Option<QvmCvarRecord>;
    /// Look up a cvar by name.
    fn get(&self, name: &str) -> Option<QvmCvarRecord>;
    /// Set a cvar.
    fn set(&mut self, name: &str, value: &str, force: bool);
    /// Set a float cvar.
    fn set_value(&mut self, name: &str, value: f32);
    /// Reset a cvar, optionally forcing.
    fn reset(&mut self, name: &str, force: bool);
    /// Register a cvar.
    fn register(&mut self, name: &str, default: &str, flags: i32);
    /// Info string for `flags`.
    fn info_string(&self, flags: i32) -> String;
}

/// Engine services for the common traps. Role-specific methods fail when
/// called for the wrong role.
pub trait QvmCommonServices {
    /// Bound role.
    fn role(&self) -> QvmRole;
    /// Cvar trap seam (the real `qvmCvarSyscall` plugs in here).
    fn dispatch_cvar(&mut self, call: &mut QvmHostCall<'_, '_, '_>) -> Result<Option<i32>, GuestError>;
    /// Print a message.
    fn print(&mut self, text: &str);
    /// Milliseconds since start.
    fn milliseconds(&mut self) -> i32;
    /// Fallback command arguments (used when the module captured none).
    fn base_arguments(&mut self) -> Vec<String>;
    /// Execute console text now (`None` runs the current buffer).
    fn execute_now(&mut self, text: Option<&str>);
    /// Insert console text at the front.
    fn insert_command(&mut self, text: &str);
    /// Append console text.
    fn append_command(&mut self, text: &str);
    /// Register a console command (cgame only).
    fn register_command(&mut self, name: &str) -> Result<(), GuestError>;
    /// Remove a console command (cgame only).
    fn remove_command(&mut self, name: &str) -> Result<(), GuestError>;
    /// Send a reliable client command (cgame only).
    fn reliable_command(&mut self, text: &str) -> Result<(), GuestError>;
    /// Real time with an optional calendar sink (qagame only).
    fn real_time(
        &mut self,
        output: Option<&mut dyn FnMut(QvmCalendar) -> Result<(), GuestError>>,
    ) -> Result<i32, GuestError>;
}

/// Handle the common traps. `None` means unhandled by this service.
pub fn qvm_common_syscall(
    call: &mut QvmHostCall<'_, '_, '_>,
    services: &mut dyn QvmCommonServices,
) -> Result<Option<i32>, GuestError> {
    let QvmHostCode::Engine(code) = call.code else {
        return Ok(None);
    };
    if call.role != services.role() {
        return Ok(None);
    }
    if let Some(value) = services.dispatch_cvar(call)? {
        return Ok(Some(value));
    }
    let trap = code;
    match services.role() {
        QvmRole::Ui => {
            let QvmTrapCode::Ui(trap) = trap else {
                return Ok(None);
            };
            match trap {
                QvmUiImport::UiPrint => {
                    let text = call.guest.read_string(call.words.get_i32(4)?)?;
                    services.print(&text);
                    Ok(Some(0))
                }
                QvmUiImport::UiError => {
                    let mut text = call.guest.read_string(call.words.get_i32(4)?)?;
                    text.truncate(4095);
                    Err(qvm_drop_error(text))
                }
                QvmUiImport::UiMilliseconds => Ok(Some(services.milliseconds())),
                QvmUiImport::UiArgc => Ok(Some(argv(call, services).len() as i32)),
                QvmUiImport::UiArgv => {
                    let argv = argv(call, services);
                    let index = call.words.get_i32(4)?;
                    let text = argv.get(index as usize).cloned().unwrap_or_default();
                    call.guest
                        .write_string(call.words.get_i32(8)?, &text, call.words.get_i32(12)? as usize)?;
                    Ok(Some(0))
                }
                QvmUiImport::UiCmdExecutetext => {
                    let when = call.words.get_i32(4)?;
                    let pointer = call.words.get_i32(8)?;
                    match when {
                        0 => {
                            let text = if pointer == 0 {
                                None
                            } else {
                                Some(call.guest.read_string(pointer)?)
                            };
                            services.execute_now(text.as_deref());
                            Ok(Some(0))
                        }
                        1 => {
                            let text = call.guest.read_string(pointer)?;
                            services.insert_command(&text);
                            Ok(Some(0))
                        }
                        2 => {
                            let text = call.guest.read_string(pointer)?;
                            services.append_command(&text);
                            Ok(Some(0))
                        }
                        _ => Err(qvm_fatal_error("Cbuf_ExecuteText: bad exec_when")),
                    }
                }
                _ => Ok(None),
            }
        }
        QvmRole::Qagame => {
            let QvmTrapCode::Game(trap) = trap else {
                return Ok(None);
            };
            match trap {
                QvmGameImport::GPrint => {
                    let text = call.guest.read_string(call.words.get_i32(4)?)?;
                    services.print(&text);
                    Ok(Some(0))
                }
                QvmGameImport::GError => {
                    let mut text = call.guest.read_string(call.words.get_i32(4)?)?;
                    text.truncate(4095);
                    Err(qvm_drop_error(text))
                }
                QvmGameImport::GMilliseconds => Ok(Some(services.milliseconds())),
                QvmGameImport::GArgc => Ok(Some(argv(call, services).len() as i32)),
                QvmGameImport::GArgv => {
                    let argv = argv(call, services);
                    let index = call.words.get_i32(4)?;
                    let text = argv.get(index as usize).cloned().unwrap_or_default();
                    call.guest
                        .write_string(call.words.get_i32(8)?, &text, call.words.get_i32(12)? as usize)?;
                    Ok(Some(0))
                }
                QvmGameImport::GRealTime => {
                    let pointer = call.words.get_i32(4)?;
                    if pointer == 0 {
                        return services.real_time(None).map(Some);
                    }
                    // Preflight the span before the service runs, matching the
                    // donor (which validates inside the sink before any write).
                    let output = call.guest.view(pointer, 36, 0)?;
                    let mut write = |calendar: QvmCalendar| -> Result<(), GuestError> {
                        let fields = [
                            calendar.second,
                            calendar.minute,
                            calendar.hour,
                            calendar.day,
                            calendar.month,
                            calendar.year,
                            calendar.weekday,
                            calendar.year_day,
                            calendar.is_dst,
                        ];
                        for (index, value) in fields.iter().enumerate() {
                            output.set_i32(index * 4, *value)?;
                        }
                        Ok(())
                    };
                    services.real_time(Some(&mut write)).map(Some)
                }
                QvmGameImport::GSendConsoleCommand => {
                    let when = call.words.get_i32(4)?;
                    let pointer = call.words.get_i32(8)?;
                    match when {
                        0 => {
                            let text = if pointer == 0 {
                                None
                            } else {
                                Some(call.guest.read_string(pointer)?)
                            };
                            services.execute_now(text.as_deref());
                            Ok(Some(0))
                        }
                        1 => {
                            let text = call.guest.read_string(pointer)?;
                            services.insert_command(&text);
                            Ok(Some(0))
                        }
                        2 => {
                            let text = call.guest.read_string(pointer)?;
                            services.append_command(&text);
                            Ok(Some(0))
                        }
                        _ => Err(qvm_fatal_error("Cbuf_ExecuteText: bad exec_when")),
                    }
                }
                _ => Ok(None),
            }
        }
        QvmRole::Cgame => {
            let QvmTrapCode::Cgame(trap) = trap else {
                return Ok(None);
            };
            match trap {
                QvmCgameImport::CgPrint => {
                    let text = call.guest.read_string(call.words.get_i32(4)?)?;
                    services.print(&text);
                    Ok(Some(0))
                }
                QvmCgameImport::CgError => {
                    let mut text = call.guest.read_string(call.words.get_i32(4)?)?;
                    text.truncate(4095);
                    Err(qvm_drop_error(text))
                }
                QvmCgameImport::CgMilliseconds => Ok(Some(services.milliseconds())),
                QvmCgameImport::CgArgc => Ok(Some(argv(call, services).len() as i32)),
                QvmCgameImport::CgArgv => {
                    let argv = argv(call, services);
                    let index = call.words.get_i32(4)?;
                    let text = argv.get(index as usize).cloned().unwrap_or_default();
                    call.guest
                        .write_string(call.words.get_i32(8)?, &text, call.words.get_i32(12)? as usize)?;
                    Ok(Some(0))
                }
                QvmCgameImport::CgArgs => {
                    let argv = argv(call, services);
                    let value = argv.get(1..).unwrap_or(&[]).join(" ");
                    if value.len() >= 1024 {
                        return Err(GuestError::invalid("Cmd_Args exceeds its source buffer"));
                    }
                    call.guest
                        .write_string(call.words.get_i32(4)?, &value, call.words.get_i32(8)? as usize)?;
                    Ok(Some(0))
                }
                QvmCgameImport::CgSendconsolecommand => {
                    let text = call.guest.read_string(call.words.get_i32(4)?)?;
                    services.append_command(&text);
                    Ok(Some(0))
                }
                QvmCgameImport::CgAddcommand => {
                    let text = call.guest.read_string(call.words.get_i32(4)?)?;
                    services.register_command(&text)?;
                    Ok(Some(0))
                }
                QvmCgameImport::CgRemovecommand => {
                    let text = call.guest.read_string(call.words.get_i32(4)?)?;
                    services.remove_command(&text)?;
                    Ok(Some(0))
                }
                QvmCgameImport::CgSendclientcommand => {
                    let text = call.guest.read_string(call.words.get_i32(4)?)?;
                    services.reliable_command(&text)?;
                    Ok(Some(0))
                }
                _ => Ok(None),
            }
        }
    }
}

fn argv(call: &QvmHostCall<'_, '_, '_>, services: &mut dyn QvmCommonServices) -> Vec<String> {
    call.command_arguments
        .clone()
        .unwrap_or_else(|| services.base_arguments())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::rc::Rc;

    use super::super::allocation::QvmAllocationProfile;
    use super::super::image::{QvmImage, QvmInstruction, QvmOpcode, QvmOperand};
    use super::super::interpreter::{QvmInterpreter, QvmSemantics};
    use super::super::syscalls::{create_qvm_system_call, QvmAbiProfile};

    #[derive(Default)]
    struct FakeServices {
        role: Option<QvmRole>,
        printed: Vec<String>,
        executed: Vec<Option<String>>,
        inserted: Vec<String>,
        appended: Vec<String>,
        registered: Vec<String>,
        removed: Vec<String>,
        reliable: Vec<String>,
        base: Vec<String>,
        realtime: Option<QvmCalendar>,
    }

    impl FakeServices {
        fn for_role(role: QvmRole) -> Self {
            Self {
                role: Some(role),
                ..Self::default()
            }
        }
    }

    impl QvmCommonServices for FakeServices {
        fn role(&self) -> QvmRole {
            self.role.unwrap()
        }

        fn dispatch_cvar(&mut self, _call: &mut QvmHostCall<'_, '_, '_>) -> Result<Option<i32>, GuestError> {
            Ok(None)
        }

        fn print(&mut self, text: &str) {
            self.printed.push(text.to_string());
        }

        fn milliseconds(&mut self) -> i32 {
            4242
        }

        fn base_arguments(&mut self) -> Vec<String> {
            self.base.clone()
        }

        fn execute_now(&mut self, text: Option<&str>) {
            self.executed.push(text.map(str::to_string));
        }

        fn insert_command(&mut self, text: &str) {
            self.inserted.push(text.to_string());
        }

        fn append_command(&mut self, text: &str) {
            self.appended.push(text.to_string());
        }

        fn register_command(&mut self, name: &str) -> Result<(), GuestError> {
            self.registered.push(name.to_string());
            Ok(())
        }

        fn remove_command(&mut self, name: &str) -> Result<(), GuestError> {
            self.removed.push(name.to_string());
            Ok(())
        }

        fn reliable_command(&mut self, text: &str) -> Result<(), GuestError> {
            self.reliable.push(text.to_string());
            Ok(())
        }

        fn real_time(
            &mut self,
            output: Option<&mut dyn FnMut(QvmCalendar) -> Result<(), GuestError>>,
        ) -> Result<i32, GuestError> {
            if let Some(write) = output {
                let calendar = self.realtime.unwrap();
                write(calendar)?;
            }
            Ok(99)
        }
    }

    fn image(program: Vec<(QvmOpcode, QvmOperand)>) -> QvmImage {
        let mut offset = 0usize;
        let instructions = program
            .into_iter()
            .map(|(opcode, operand)| {
                let instruction = QvmInstruction {
                    byte_offset: offset,
                    opcode,
                    operand,
                };
                offset += 1 + opcode.operand_width();
                instruction
            })
            .collect();
        QvmImage {
            source: "test".to_string(),
            instructions,
            code_offset: 0,
            code_length: offset,
            data_length: 256,
            literal_length: 0,
            bss_length: 0,
            initialized_data: vec![0; 256],
            allocated_data_length: 256,
            data_mask: 255,
        }
    }

    /// ENTER frame; ARG-publish `args` at words 1..; CONST target; CALL; LEAVE.
    /// The frame must exceed the highest ARG offset so published words stay
    /// below the caller's return slot.
    fn trap_program(trap: i32, args: &[i32]) -> Vec<(QvmOpcode, QvmOperand)> {
        use QvmOpcode as O;
        let frame = (16).max(8 + args.len() as i32 * 4);
        let mut program = vec![(O::OpEnter, QvmOperand::Word(frame))];
        for (index, value) in args.iter().enumerate() {
            program.push((O::OpConst, QvmOperand::Word(*value)));
            program.push((O::OpArg, QvmOperand::Byte((8 + index * 4) as u8)));
        }
        program.push((O::OpConst, QvmOperand::Word(-1 - trap)));
        program.push((O::OpCall, QvmOperand::None));
        program.push((O::OpLeave, QvmOperand::Word(frame)));
        program
    }

    struct Harness {
        vm: QvmInterpreter,
        services: Rc<RefCell<FakeServices>>,
    }

    fn make_harness(role: QvmRole, trap: i32, args: &[i32]) -> Harness {
        let vm = QvmInterpreter::new(
            &image(trap_program(trap, args)),
            QvmAllocationProfile::Unaccounted,
            None,
            QvmSemantics::Interpreted,
        )
        .unwrap();
        let services = Rc::new(RefCell::new(FakeServices::for_role(role)));
        Harness { vm, services }
    }

    fn run(harness: &mut Harness, role: QvmRole, argv: Option<Vec<String>>) -> Result<i32, GuestError> {
        let services = Rc::clone(&harness.services);
        let system = create_qvm_system_call(
            role,
            move |call: &mut QvmHostCall<'_, '_, '_>| -> Result<i32, GuestError> {
                let mut services = services.borrow_mut();
                qvm_common_syscall(call, &mut *services)?
                    .ok_or_else(|| GuestError::callback("unhandled common trap in test"))
            },
            Box::new(move || argv.clone()),
            QvmAbiProfile::Modern,
        );
        harness.vm.invoke(&system, &[0; 10], 0, None)
    }

    #[test]
    fn print_milliseconds_and_argv() {
        let mut harness = make_harness(QvmRole::Qagame, 0, &[64]);
        harness.vm.memory().write_bytes(64, b"hello\0").unwrap();
        assert_eq!(run(&mut harness, QvmRole::Qagame, None).unwrap(), 0);
        assert_eq!(harness.services.borrow().printed, vec!["hello".to_string()]);

        let mut harness = make_harness(QvmRole::Qagame, 2, &[]);
        assert_eq!(run(&mut harness, QvmRole::Qagame, None).unwrap(), 4242);

        let mut harness = make_harness(QvmRole::Qagame, 8, &[]);
        assert_eq!(
            run(
                &mut harness,
                QvmRole::Qagame,
                Some(vec!["prog".to_string(), "x".to_string()])
            )
            .unwrap(),
            2
        );
        let mut harness = make_harness(QvmRole::Qagame, 9, &[1, 96, 16]);
        assert_eq!(
            run(
                &mut harness,
                QvmRole::Qagame,
                Some(vec!["prog".to_string(), "x".to_string()])
            )
            .unwrap(),
            0
        );
        assert_eq!(harness.vm.memory().read_string(96).unwrap(), "x");
    }

    #[test]
    fn errors_drop_and_bad_exec_when_is_fatal() {
        let mut harness = make_harness(QvmRole::Ui, 0, &[64]);
        harness.vm.memory().write_bytes(64, b"boom\0").unwrap();
        let error = run(&mut harness, QvmRole::Ui, None).unwrap_err();
        assert_eq!(error.to_string(), "drop: boom");

        let mut harness = make_harness(QvmRole::Ui, 12, &[9, 64]);
        let error = run(&mut harness, QvmRole::Ui, None).unwrap_err();
        assert_eq!(error.to_string(), "fatal: Cbuf_ExecuteText: bad exec_when");
    }

    #[test]
    fn execute_text_routes_by_when() {
        for (when, expected) in [(0, "now"), (1, "front"), (2, "back")] {
            let mut harness = make_harness(QvmRole::Qagame, 14, &[when, 64]);
            harness.vm.memory().write_bytes(64, b"say hi\0").unwrap();
            assert_eq!(run(&mut harness, QvmRole::Qagame, None).unwrap(), 0);
            let services = harness.services.borrow();
            match expected {
                "now" => assert_eq!(services.executed, vec![Some("say hi".to_string())]),
                "front" => assert_eq!(services.inserted, vec!["say hi".to_string()]),
                _ => assert_eq!(services.appended, vec!["say hi".to_string()]),
            }
        }
        // Null pointer with when=0 runs the current buffer.
        let mut harness = make_harness(QvmRole::Qagame, 14, &[0, 0]);
        assert_eq!(run(&mut harness, QvmRole::Qagame, None).unwrap(), 0);
        assert_eq!(harness.services.borrow().executed, vec![None]);
    }

    #[test]
    fn cgame_commands_and_args() {
        let mut harness = make_harness(QvmRole::Cgame, 9, &[96, 32]);
        assert_eq!(
            run(
                &mut harness,
                QvmRole::Cgame,
                Some(vec!["prog".to_string(), "a".to_string(), "b".to_string()])
            )
            .unwrap(),
            0
        );
        assert_eq!(harness.vm.memory().read_string(96).unwrap(), "a b");

        let mut harness = make_harness(QvmRole::Cgame, 14, &[64]);
        harness.vm.memory().write_bytes(64, b"kill\0").unwrap();
        assert_eq!(run(&mut harness, QvmRole::Cgame, None).unwrap(), 0);
        assert_eq!(harness.services.borrow().appended, vec!["kill".to_string()]);

        let mut harness = make_harness(QvmRole::Cgame, 15, &[64]);
        harness.vm.memory().write_bytes(64, b"+attack\0").unwrap();
        assert_eq!(run(&mut harness, QvmRole::Cgame, None).unwrap(), 0);
        assert_eq!(harness.services.borrow().registered, vec!["+attack".to_string()]);

        let mut harness = make_harness(QvmRole::Cgame, 16, &[64]);
        harness.vm.memory().write_bytes(64, b"say x\0").unwrap();
        assert_eq!(run(&mut harness, QvmRole::Cgame, None).unwrap(), 0);
        assert_eq!(harness.services.borrow().reliable, vec!["say x".to_string()]);

        let mut harness = make_harness(QvmRole::Cgame, 72, &[64]);
        harness.vm.memory().write_bytes(64, b"+attack\0").unwrap();
        assert_eq!(run(&mut harness, QvmRole::Cgame, None).unwrap(), 0);
        assert_eq!(harness.services.borrow().removed, vec!["+attack".to_string()]);
    }

    #[test]
    fn real_time_writes_the_calendar() {
        let mut harness = make_harness(QvmRole::Qagame, 41, &[128]);
        harness.services.borrow_mut().realtime = Some(QvmCalendar {
            second: 1,
            minute: 2,
            hour: 3,
            day: 4,
            month: 5,
            year: 2026,
            weekday: 6,
            year_day: 7,
            is_dst: 0,
        });
        assert_eq!(run(&mut harness, QvmRole::Qagame, None).unwrap(), 99);
        let memory = harness.vm.memory();
        assert_eq!(memory.get_i32(128).unwrap(), 1);
        assert_eq!(memory.get_i32(128 + 20).unwrap(), 2026);

        let mut harness = make_harness(QvmRole::Qagame, 41, &[0]);
        assert_eq!(run(&mut harness, QvmRole::Qagame, None).unwrap(), 99);
    }
}
