use qa_compat::{
    abi::{
        Addresses, CallTable, HostRequest, ImportStep, Invocation, Q3_CLIENT, Q3_UI, QvmCalls,
        UnknownCalls,
    },
    memory::ModuleMemory,
    services::{CallContext, CallError, ENGINE_CALLS, EngineServices, ServiceStorage},
};
use qa_console::{
    command_buffer::CAPACITY,
    commands::{BindOutput, Console, Host, ScriptError},
    views::Context,
};
use qa_core::{
    names::NameTable,
    primitives::{ModuleId, RuleSetId, ThinkTime},
    sys_events::{DeviceId, EventKind, EventTime, SeatId, SysEvent, key_catchers},
};
use qa_world::{
    collision::{CollisionStore, TraceScratch},
    entities::AllocationPolicy,
};

#[derive(Default)]
struct Capture {
    input: qa_input::Input,
    bytes: Vec<u8>,
    rejected: u64,
}
impl Host for Capture {
    fn print(&mut self, text: std::fmt::Arguments<'_>) {
        self.bytes.extend(text.to_string().as_bytes());
    }
    fn print_bytes(&mut self, text: &[u8]) {
        self.bytes.extend(text);
    }
    fn reject_input(&mut self, count: u64) {
        self.rejected += count;
    }
    fn read_script(&mut self, _: &str, _: &mut [u8]) -> Result<usize, ScriptError> {
        Err(ScriptError::Missing)
    }
    fn quit(&mut self) {}
    fn input(&mut self) -> &mut qa_input::Input {
        &mut self.input
    }
    fn input_time(&self) -> EventTime {
        EventTime::from_milliseconds(42)
    }
}
struct Fixture {
    server: qa_session::clients::Server,
    console: Console<Capture>,
    host: Capture,
    vfs: qa_content::vfs::Vfs,
    localization: qa_content::localization::Catalog,
    registry: qa_gameplay::registry::Registry,
    names: NameTable,
    storage: ServiceStorage,
    geometry: CollisionStore,
    scratch: TraceScratch,
}
impl Fixture {
    fn new() -> Self {
        let names = NameTable::load(qa_gameplay::registry::Registry::names_needed()).unwrap();
        let registry = qa_gameplay::registry::Registry::load(&names).unwrap();
        let console = Console::new(Context::default()).unwrap();
        let storage =
            ServiceStorage::load_native(&[(ModuleId(1), 0, RuleSetId::Quake3)], 0, &console.cvars)
                .unwrap();
        let geometry = CollisionStore::new();
        let scratch = geometry.scratch();
        Self {
            server: qa_session::clients::Server::load(2, 8, 1, 1, 1, 0).unwrap(),
            console,
            host: Capture::default(),
            vfs: Default::default(),
            localization: Default::default(),
            registry,
            names,
            storage,
            geometry,
            scratch,
        }
    }
    fn engine(&mut self) -> EngineServices<'_> {
        let (cvars, commands, editor) = self.console.module_parts();
        EngineServices {
            server: &mut self.server,
            callbacks: None,
            client_disconnect: None,
            input: &mut self.host.input,
            editor: Some(editor),
            cvars,
            commands,
            client_connection: None,
            client_collision: None,
            client_audio: None,
            renderer: None,
            scene: None,
            vfs: &self.vfs,
            model_loader: None,
            model_lookup: None,
            sound_loader: None,
            sound_bank: None,
            localization: &self.localization,
            registry: &self.registry,
            names: &mut self.names,
            storage: &mut self.storage,
            geometry: &self.geometry,
            world: None,
            visibility: None,
            portals: None,
            scratch: &mut self.scratch,
        }
    }
    fn context(seat: SeatId) -> CallContext {
        CallContext {
            module: ModuleId(seat.index() as u16 + 1),
            clock: ThinkTime::Milliseconds(99999),
            server_frame: 0,
            native_rules: RuleSetId::Quake3,
            trace_rules: RuleSetId::QuakeWorld,
            sound_policy: qa_gameplay::rules::sound_policy(RuleSetId::Quake2),
            console: Context {
                source: RuleSetId::Quake2Rerelease,
                seat,
                ..Context::default()
            },
            allocation: AllocationPolicy::EDICT,
            link_order: qa_gameplay::rules::link_order(RuleSetId::Quake),
        }
    }
    fn call(
        &mut self,
        table: &CallTable,
        mode: Addresses,
        memory: &mut ModuleMemory<'_>,
        number: u32,
        arguments: &[u64],
        seat: SeatId,
    ) -> Result<u64, CallError> {
        let mut unknown = UnknownCalls::load(2).unwrap();
        let mut engine = self.engine();
        let mut invocation = Invocation {
            services: &mut engine,
            memory,
            native_cvars: None,
            native_resources: None,
            native_entities: None,
            native_surfaces: None,
            native_command: None,
            native_configs: None,
            native_pmove: None,
            native_player: None,
            context: Self::context(seat),
            platform_time: EventTime::from_milliseconds(42),
            command: Default::default(),
            addresses: mode,
            arguments,
        };
        let mut step = table.invoke(number, &mut invocation, &mut unknown)?;
        assert_eq!(unknown.calls, 0);
        loop {
            match step {
                ImportStep::Return(value) => return Ok(value),
                ImportStep::Host {
                    request:
                        HostRequest::ClearKeys {
                            seat,
                            context,
                            time,
                        },
                    continuation,
                } => {
                    (ENGINE_CALLS.clear_keys)(invocation.services, seat, context, time)?;
                    step = continuation.resume(&mut invocation, 0)?;
                }
                ImportStep::Host { .. } => panic!("unexpected callback"),
            }
        }
    }
    fn key(&mut self, device: DeviceId, control: u16, down: bool) {
        let (_, commands, _) = self.console.module_parts();
        let mut sink = BindOutput::new(commands, Context::default());
        self.host.input.dispatch(
            SysEvent {
                time: EventTime::from_milliseconds(40),
                kind: EventKind::Key {
                    device,
                    code: control,
                    symbol: 0,
                    down,
                    repeat: false,
                },
            },
            &mut sink,
        );
        assert_eq!(sink.outcome().0, 0);
    }
    fn execute(&mut self) {
        self.console.execute_frame(&mut self.host);
    }
}
fn modes() -> [(Addresses, u64); 4] {
    [
        (Addresses::Qvm { mask: 8191 }, 0),
        (
            Addresses::Native {
                abi: qa_platform::native::NativeAbi::SystemV,
                pointer_bytes: 4,
            },
            0x2000_0000,
        ),
        (
            Addresses::Native {
                abi: qa_platform::native::NativeAbi::SystemV,
                pointer_bytes: 8,
            },
            1 << 40,
        ),
        (
            Addresses::Native {
                abi: qa_platform::native::NativeAbi::Microsoft,
                pointer_bytes: 8,
            },
            1 << 40,
        ),
    ]
}
fn text(memory: &mut ModuleMemory<'_>, at: u64, bytes: &[u8]) {
    memory.write(at, bytes).unwrap();
    memory.write(at + bytes.len() as u64, &[0]).unwrap();
}
fn input_vm(
    operations: &[(qa_formats::program::qvm::Opcode, i32)],
    data: &[u8],
) -> qa_compat::qvm::Vm {
    let mut code = Vec::new();
    for &(op, arg) in operations {
        code.push(op as u8);
        match op.operand_bytes() {
            1 => code.push(arg as u8),
            4 => code.extend(arg.to_le_bytes()),
            _ => {}
        }
    }
    code.resize(code.len().next_multiple_of(4), 0);
    let mut bytes = Vec::new();
    for word in [
        0x12721444,
        operations.len() as i32,
        32,
        code.len() as i32,
        32 + code.len() as i32,
        data.len() as i32,
        0,
        65536 - data.len() as i32,
    ] {
        bytes.extend(word.to_le_bytes());
    }
    bytes.extend(code);
    bytes.extend(data);
    qa_compat::qvm::Vm::load(qa_formats::program::qvm::Image::parse(&bytes).unwrap()).unwrap()
}
#[test]
fn native_key_bindings_keep_exact_ordinals_raw_bytes_empty_presence_and_native_order() {
    for (mode, base) in modes() {
        let mut f = Fixture::new();
        let mut m = ModuleMemory::load(base, 8192, &[]).unwrap();
        let seat = SeatId::FIRST;
        for (key, value) in [
            (65, b"upper".as_slice()),
            (97, b"lower"),
            (33, b"bang"),
            (49, b"digit"),
        ] {
            text(&mut m, base + 128, value);
            assert_eq!(
                f.call(&Q3_UI, mode, &mut m, 35, &[key, base + 128], seat),
                Ok(0)
            );
        }
        for (key, value) in [
            (65, b"upper".as_slice()),
            (97, b"lower"),
            (33, b"bang"),
            (49, b"digit"),
        ] {
            f.call(&Q3_UI, mode, &mut m, 34, &[key, base + 512, 64], seat)
                .unwrap();
            assert_eq!(m.cstring(base + 512).unwrap(), value);
        }
        let raw = b"echo raw\xff \"quoted\x80\"";
        text(&mut m, base + 128, raw);
        for key in [130, 9] {
            f.call(&Q3_UI, mode, &mut m, 35, &[key, base + 128], seat)
                .unwrap();
        }
        text(&mut m, base + 256, b"ECHO RAW\xff \"QUOTED\x80\"");
        assert_eq!(
            f.call(&Q3_CLIENT, mode, &mut m, 63, &[base + 256], seat),
            Ok(9)
        );
        f.call(&Q3_UI, mode, &mut m, 34, &[130, base + 512, 64], seat)
            .unwrap();
        assert_eq!(m.cstring(base + 512).unwrap(), raw);
        assert_eq!(
            f.call(&Q3_CLIENT, mode, &mut m, 63, &[0], seat),
            Ok(u32::MAX as u64)
        );
        text(&mut m, base + 128, b"");
        assert_eq!(
            f.call(&Q3_CLIENT, mode, &mut m, 63, &[base + 128], seat),
            Ok(u32::MAX as u64)
        );
        f.call(&Q3_UI, mode, &mut m, 35, &[222, base + 128], seat)
            .unwrap();
        assert_eq!(
            f.call(&Q3_CLIENT, mode, &mut m, 63, &[base + 128], seat),
            Ok(222)
        );
        assert_eq!(
            f.call(&Q3_UI, mode, &mut m, 35, &[u32::MAX as u64, u64::MAX], seat),
            Ok(0)
        );
        assert_eq!(
            f.call(&Q3_UI, mode, &mut m, 35, &[97, 0], seat),
            Err(CallError::Memory)
        );
        text(&mut m, base + 128, &vec![b'x'; 1025]);
        assert_eq!(
            f.call(&Q3_UI, mode, &mut m, 35, &[97, base + 128], seat),
            Err(CallError::Capacity)
        );
        f.call(&Q3_UI, mode, &mut m, 34, &[97, base + 512, 64], seat)
            .unwrap();
        assert_eq!(m.cstring(base + 512).unwrap(), b"lower");
    }
}
#[test]
fn native_name_buffers_preserve_raw_ascii_spelling_truncation_and_checked_spans() {
    for (mode, base) in modes() {
        let mut f = Fixture::new();
        let mut m = ModuleMemory::load(base, 8192, &[]).unwrap();
        let seat = SeatId::FIRST;
        for (key, expected) in [
            (65, b"A".as_slice()),
            (97, b"a"),
            (59, b"SEMICOLON"),
            (34, b"0x22"),
            (u32::MAX as u64, b"<KEY NOT FOUND>"),
            (256, b"<OUT OF RANGE>"),
        ] {
            m.write(base + 511, &[0xa5; 66]).unwrap();
            f.call(&Q3_UI, mode, &mut m, 33, &[key, base + 512, 64], seat)
                .unwrap();
            assert_eq!(m.cstring(base + 512).unwrap(), expected);
            assert_eq!(m.read(base + 511, 1).unwrap(), &[0xa5]);
            assert_eq!(
                m.read(base + 512 + expected.len() as u64 + 1, 1).unwrap(),
                &[0xa5]
            );
        }
        f.call(
            &Q3_UI,
            mode,
            &mut m,
            33,
            &[u32::MAX as u64, base + 512, 4],
            seat,
        )
        .unwrap();
        assert_eq!(m.read(base + 512, 4).unwrap(), b"<KE\0");
        m.write(base + 512, &[0xa5; 16]).unwrap();
        for length in [0, u32::MAX as u64, 8192] {
            assert!(
                f.call(&Q3_UI, mode, &mut m, 33, &[65, base + 512, length], seat)
                    .is_err()
            );
            assert_eq!(m.read(base + 512, 16).unwrap(), &[0xa5; 16]);
        }
        assert_eq!(
            f.call(&Q3_UI, mode, &mut m, 33, &[65, 0, 4], seat),
            Err(CallError::Memory)
        );
        if matches!(mode, Addresses::Qvm { .. }) {
            text(&mut m, 0, b"wrapped");
            f.call(&Q3_UI, mode, &mut m, 35, &[97, 8192], seat).unwrap();
            f.call(&Q3_UI, mode, &mut m, 34, &[97, base + 512, 8], seat)
                .unwrap();
            assert_eq!(m.cstring(base + 512).unwrap(), b"wrapped");
        }
    }
}
#[test]
fn key_queries_clear_and_editor_modes_use_the_same_seat_and_command_owners() {
    let mut f = Fixture::new();
    let mode = Addresses::Qvm { mask: 8191 };
    let mut m = ModuleMemory::load(0, 8192, &[]).unwrap();
    let a = SeatId::FIRST;
    let b = SeatId::ALL[1];
    assert!(f.host.input.assign(DeviceId::Keyboard(1), b));
    text(&mut m, 128, b"echo raw\xff");
    f.call(&Q3_UI, mode, &mut m, 35, &[97, 128], a).unwrap();
    f.key(DeviceId::Keyboard(0), 4, true);
    f.key(DeviceId::Keyboard(1), 4, true);
    for seat in [a, b] {
        assert_eq!(f.call(&Q3_UI, mode, &mut m, 36, &[97], seat), Ok(1));
    }
    f.execute();
    assert_eq!(f.host.bytes, b"raw\xff \nraw\xff \n");
    f.call(&Q3_UI, mode, &mut m, 39, &[], a).unwrap();
    assert_eq!(f.call(&Q3_CLIENT, mode, &mut m, 60, &[97], a), Ok(0));
    assert_eq!(f.call(&Q3_CLIENT, mode, &mut m, 60, &[97], b), Ok(1));
    f.call(
        &Q3_UI,
        mode,
        &mut m,
        41,
        &[u64::from(key_catchers::CONSOLE | key_catchers::UI)],
        a,
    )
    .unwrap();
    f.console
        .sync_editor_input(&mut f.host.input, EventTime(43));
    assert_eq!(f.call(&Q3_CLIENT, mode, &mut m, 61, &[], a), Ok(3));
    assert_eq!(
        f.host.input.catcher_module(a, key_catchers::UI),
        Some(ModuleId(1))
    );
    f.call(
        &Q3_CLIENT,
        mode,
        &mut m,
        62,
        &[u64::from(key_catchers::CGAME)],
        b,
    )
    .unwrap();
    assert_eq!(f.console.editor.seat(), Some(a)); // Clearing B's CONSOLE bit leaves A's editor.
    f.call(
        &Q3_UI,
        mode,
        &mut m,
        41,
        &[u64::from(key_catchers::CONSOLE | key_catchers::UI)],
        b,
    )
    .unwrap();
    assert_eq!(f.console.editor.seat(), Some(b));
    assert_eq!(f.call(&Q3_CLIENT, mode, &mut m, 61, &[], a), Ok(2));
    assert_eq!(f.call(&Q3_UI, mode, &mut m, 37, &[], b), Ok(0));
    f.call(&Q3_UI, mode, &mut m, 38, &[257], b).unwrap();
    assert_eq!(f.call(&Q3_UI, mode, &mut m, 37, &[], b), Ok(1));
}
#[test]
fn void_clear_under_command_pressure_cancels_the_source_without_trapping() {
    let mut f = Fixture::new();
    let mode = Addresses::Qvm { mask: 8191 };
    let mut m = ModuleMemory::load(0, 8192, &[]).unwrap();
    text(&mut m, 128, b"+forward");
    f.call(&Q3_UI, mode, &mut m, 35, &[97, 128], SeatId::FIRST)
        .unwrap();
    f.key(DeviceId::Keyboard(0), 4, true);
    f.execute();
    let (_, commands, _) = f.console.module_parts();
    commands
        .append(&"x".repeat(CAPACITY - 1), Context::default())
        .unwrap();
    let rejected = f.server.events.rejected_publications();
    assert_eq!(f.call(&Q3_UI, mode, &mut m, 39, &[], SeatId::FIRST), Ok(0));
    assert_eq!(f.server.events.rejected_publications(), rejected + 1);
    assert!(!f.host.input.is_down(SeatId::FIRST, 4));
    assert_eq!(
        f.call(&Q3_UI, mode, &mut m, 36, &[97], SeatId::FIRST),
        Ok(0)
    );
}
#[test]
fn qvm_executes_the_numbered_ui_imports_and_caches_input_export_capabilities() {
    use qa_formats::program::qvm::Opcode::*;
    let mut operations = vec![(Enter, 64)];
    for (number, args) in [(35, &[97, 128][..]), (38, &[17][..])] {
        for (index, &arg) in args.iter().enumerate() {
            operations.extend([(Const, arg), (Arg, 8 + index as i32 * 4)]);
        }
        operations.extend([(Const, -number - 1), (Call, 0), (Pop, 0)]);
    }
    operations.extend([(Const, -38), (Call, 0), (Leave, 64)]);
    let mut data = [0u8; 256];
    data[128..137].copy_from_slice(b"echo qvm\0");
    let mut vm = input_vm(&operations, &data);
    let mut f = Fixture::new();
    let mut unknown = UnknownCalls::load(2).unwrap();
    let mut engine = f.engine();
    let mut calls = QvmCalls {
        services: &mut engine,
        table: &Q3_UI,
        context: Fixture::context(SeatId::FIRST),
        platform_time: EventTime(42),
        command: Default::default(),
        unknown: &mut unknown,
    };
    assert_eq!(vm.call(&mut calls, [0; 10], 1000, false), Ok(1));
    assert_eq!(unknown.calls, 0);
    assert_eq!(f.host.input.binding(4).unwrap().bytes(), b"echo qvm");
    let ui = Q3_UI.input_exports().unwrap();
    let cg = Q3_CLIENT.input_exports().unwrap();
    assert_eq!(
        (ui.catcher, ui.key, ui.mouse, ui.character, ui.menu),
        (2, 3, 4, true, Some(7))
    );
    assert_eq!(
        (
            cg.catcher,
            cg.key,
            cg.mouse,
            cg.character,
            cg.event_handling
        ),
        (8, 6, 7, false, Some(8))
    );
    assert!(qa_compat::abi::Q3_SERVER.input_exports().is_none());
}

#[test]
fn qvm_clear_keys_yields_owned_host_work_before_changing_input() {
    use qa_compat::qvm::Step;
    use qa_formats::program::qvm::Opcode::*;
    let mut f = Fixture::new();
    let mut vm = input_vm(
        &[
            (Enter, 64),
            (Const, -40), // UI_KEY_CLEARSTATES
            (Call, 0),
            (Pop, 0),
            (Const, 97),
            (Arg, 8),
            (Const, -37), // UI_KEY_ISDOWN
            (Call, 0),
            (Leave, 64),
        ],
        &[0; 256],
    );
    f.key(DeviceId::Keyboard(0), 4, true);
    assert!(f.host.input.is_down(SeatId::FIRST, 4));
    assert!(!vm.is_active());
    let mut run = vm.begin_root([0; 10], 100, true).unwrap();
    assert!(vm.is_active());
    let Step::Syscall(syscall) = vm.advance(&mut run).unwrap() else {
        panic!("expected executed ClearStates import");
    };
    assert_eq!(syscall.number, 39);
    let mut unknown = UnknownCalls::load(2).unwrap();
    let context = Fixture::context(SeatId::FIRST);
    let queued = EventTime::from_milliseconds(42);
    let step = {
        let mut engine = f.engine();
        QvmCalls {
            services: &mut engine,
            table: &Q3_UI,
            context,
            platform_time: queued,
            command: Default::default(),
            unknown: &mut unknown,
        }
        .import(&mut vm, &syscall)
        .unwrap()
    };
    assert!(f.host.input.is_down(SeatId::FIRST, 4));
    let ImportStep::Host {
        request:
            HostRequest::ClearKeys {
                seat,
                context: captured,
                time,
            },
        continuation,
    } = step
    else {
        panic!("ClearStates must return owned host work");
    };
    assert_eq!(seat, SeatId::FIRST);
    assert_eq!(captured.source, RuleSetId::Quake2Rerelease);
    assert_eq!(captured.seat, context.console.seat);
    assert_eq!(time, queued);
    // Neither the parked VM nor the engine is borrowed by this host request.
    vm.memory.write_word(64, 777).unwrap();
    let step = {
        let mut engine = f.engine();
        (ENGINE_CALLS.clear_keys)(&mut engine, seat, captured, time).unwrap();
        QvmCalls {
            services: &mut engine,
            table: &Q3_UI,
            context,
            platform_time: queued,
            command: Default::default(),
            unknown: &mut unknown,
        }
        .resume_import(&mut vm, continuation, 123)
        .unwrap()
    };
    assert!(!f.host.input.is_down(SeatId::FIRST, 4));
    let ImportStep::Return(value) = step else {
        panic!("void ClearStates continuation must complete");
    };
    assert_eq!(value, 0); // A host callback's scalar cannot replace a void result.
    let Step::Syscall(syscall) = vm.resume(&mut run, value as i32).unwrap() else {
        panic!("guest must continue to its KeyIsDown import");
    };
    assert_eq!(syscall.number, 36);
    let step = {
        let mut engine = f.engine();
        QvmCalls {
            services: &mut engine,
            table: &Q3_UI,
            context,
            platform_time: queued,
            command: Default::default(),
            unknown: &mut unknown,
        }
        .import(&mut vm, &syscall)
        .unwrap()
    };
    assert!(matches!(step, ImportStep::Return(0)));
    assert_eq!(vm.resume(&mut run, 0), Ok(Step::Returned(0)));
    assert!(!vm.is_active());
    assert_eq!(vm.memory.read_word(64).unwrap(), 777);
    assert_eq!(unknown.calls, 0);
}

#[test]
fn qvm_import_uses_captured_scalar_arguments_and_current_guest_string_bytes() {
    use qa_compat::qvm::Step;
    use qa_formats::program::qvm::Opcode::*;
    let mut data = [0u8; 256];
    data[128..137].copy_from_slice(b"echo old\0");
    let mut vm = input_vm(
        &[
            (Enter, 64),
            (Const, 97),
            (Arg, 8),
            (Const, 128),
            (Arg, 12),
            (Const, -36), // UI_KEY_SETBINDING
            (Call, 0),
            (Leave, 64),
        ],
        &data,
    );
    let mut run = vm.begin_root([0; 10], 100, false).unwrap();
    let Step::Syscall(syscall) = vm.advance(&mut run).unwrap() else {
        panic!("expected executed SetBinding import");
    };
    assert_eq!(&syscall.arguments()[..3], &[35, 97, 128]);
    // Native arguments were already captured; the pointed-to text stays live.
    let frame = vm.memory.len() as u64 - 48 - 64;
    vm.memory.write_word(frame + 8, -1).unwrap();
    vm.memory.write_word(frame + 12, 0).unwrap();
    vm.memory.write(128, b"echo new\0").unwrap();
    let mut f = Fixture::new();
    let mut unknown = UnknownCalls::load(2).unwrap();
    let step = {
        let mut engine = f.engine();
        QvmCalls {
            services: &mut engine,
            table: &Q3_UI,
            context: Fixture::context(SeatId::FIRST),
            platform_time: EventTime(42),
            command: Default::default(),
            unknown: &mut unknown,
        }
        .import(&mut vm, &syscall)
        .unwrap()
    };
    assert!(matches!(step, ImportStep::Return(0)));
    assert_eq!(f.host.input.binding(4).unwrap().bytes(), b"echo new");
    assert_eq!(vm.resume(&mut run, 0), Ok(Step::Returned(0)));
    assert_eq!(unknown.calls, 0);
}

#[test]
fn synchronous_qvm_adapter_drains_the_same_clear_keys_host_request() {
    use qa_formats::program::qvm::Opcode::*;
    let mut vm = input_vm(
        &[(Enter, 64), (Const, -40), (Call, 0), (Leave, 64)],
        &[0; 256],
    );
    let mut f = Fixture::new();
    f.key(DeviceId::Keyboard(0), 4, true);
    let mut unknown = UnknownCalls::load(2).unwrap();
    let mut engine = f.engine();
    let mut calls = QvmCalls {
        services: &mut engine,
        table: &Q3_UI,
        context: Fixture::context(SeatId::FIRST),
        platform_time: EventTime(42),
        command: Default::default(),
        unknown: &mut unknown,
    };
    assert_eq!(vm.call(&mut calls, [0; 10], 100, false), Ok(0));
    assert!(!f.host.input.is_down(SeatId::FIRST, 4));
    assert!(!vm.is_active());
    assert_eq!(unknown.calls, 0);
}
