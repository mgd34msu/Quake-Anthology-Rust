//! Real queued atoms enter the same QVM and owned-child module call tables.
use super::*;
use qa_app::host::FrameSource;
use qa_core::sys_events::key_catchers;
use std::collections::VecDeque;

const KEY_COUNT: u32 = 0;
const LAST_KEY: u32 = 4;
const LAST_DOWN: u32 = 8;
const HELD_KEY: u32 = 12;
const HELD_A: u32 = 16;
const CATCHERS: u32 = 20;
const MOUSE_COUNT: u32 = 24;
const MOUSE_X: u32 = 28;
const MOUSE_Y: u32 = 32;
const MENU_COUNT: u32 = 36;
const MENU_VALUE: u32 = 40;
const HANDLING_COUNT: u32 = 44;
const HANDLING_VALUE: u32 = 48;
const SWITCH_KEY: u32 = 52;
const SWITCH_DOWN: u32 = 56;
const SWITCH_MASK: u32 = 60;
const FAIL: u32 = 64;
const CLEAR_ON_UP: u32 = 68;
const CALLBACK_TIME: u32 = 72;
const UPPER: u32 = 512;
const LOWER: u32 = 528;
const FORWARD: u32 = 544;
const FORWARD_CASE: u32 = 560;
const ERROR: u32 = 576;
const OUTPUT: u32 = 640;
const EMPTY: u32 = 704;
const RELEASE_BIND: u32 = 736;
const RELEASE_CALLBACK: u32 = 832;

#[derive(Clone, Copy)]
enum Value {
    Constant(u32),
    Arg(u8),
    Word(u32),
    Pointer(u32),
    PointerArg(u8),
    ReadArg(u8),
}
enum Op {
    Write(u32, Value),
    WriteAt(u8, Value),
    Increment(u32),
    Call(u32, Vec<Value>, Option<u32>),
    IfEqual(Value, Value, Vec<Op>),
    Return(Value),
}
fn call(number: u32, args: &[Value], output: Option<u32>) -> Op {
    Op::Call(number, args.to_vec(), output)
}
fn bodies(ui: bool) -> Vec<Vec<Op>> {
    use Value::*;
    let (key, mouse, down, catcher, set_catcher) = if ui {
        (3, 4, 36, 40, 41)
    } else {
        (6, 7, 60, 61, 62)
    };
    let mut bodies: Vec<Vec<Op>> = (0..32).map(|_| vec![Op::Return(Constant(0))]).collect();
    bodies[key] = vec![
        Op::Increment(KEY_COUNT),
        Op::Write(LAST_KEY, Arg(0)),
        Op::Write(LAST_DOWN, Arg(1)),
        call(down, &[Arg(0)], Some(HELD_KEY)),
        call(down, &[Constant(97)], Some(HELD_A)),
        call(catcher, &[], Some(CATCHERS)),
        call(2, &[], Some(CALLBACK_TIME)),
        Op::IfEqual(
            Arg(0),
            Word(SWITCH_KEY),
            vec![Op::IfEqual(
                Arg(1),
                Word(SWITCH_DOWN),
                vec![call(set_catcher, &[Word(SWITCH_MASK)], None)],
            )],
        ),
        Op::IfEqual(
            Arg(1),
            Constant(0),
            vec![Op::IfEqual(
                Word(CLEAR_ON_UP),
                Constant(1),
                if ui {
                    vec![
                        call(set_catcher, &[Constant(0)], None),
                        call(12, &[Constant(2), Pointer(RELEASE_CALLBACK)], None),
                    ]
                } else {
                    vec![call(set_catcher, &[Constant(0)], None)]
                },
            )],
        ),
        Op::IfEqual(
            Word(FAIL),
            Constant(1),
            if ui {
                // A real checked-memory import failure avoids adding an
                // intentional print to the process-global diagnostic sink.
                vec![call(33, &[Constant(97), Constant(0), Constant(8)], None)]
            } else {
                vec![call(1, &[Pointer(ERROR)], None)]
            },
        ),
        Op::Return(Constant(0)),
    ];
    bodies[mouse] = vec![
        Op::Increment(MOUSE_COUNT),
        Op::Write(MOUSE_X, Arg(0)),
        Op::Write(MOUSE_Y, Arg(1)),
        call(down, &[Constant(97)], Some(HELD_A)),
        Op::Return(Constant(0)),
    ];
    if ui {
        bodies[7] = vec![
            Op::Increment(MENU_COUNT),
            Op::Write(MENU_VALUE, Arg(0)),
            call(set_catcher, &[Constant(key_catchers::UI)], None),
            Op::Return(Constant(0)),
        ];
    } else {
        bodies[8] = vec![
            Op::Increment(HANDLING_COUNT),
            Op::Write(HANDLING_VALUE, Arg(0)),
            call(catcher, &[], Some(CATCHERS)),
            Op::Return(Constant(0)),
        ];
    }
    bodies[10] = vec![Op::Return(ReadArg(0))];
    bodies[11] = vec![Op::WriteAt(0, Arg(1)), Op::Return(Constant(0))];
    if ui {
        for (entry, ordinal, args) in [
            (20, 33, vec![Arg(0), Pointer(OUTPUT), Constant(64)]),
            (21, 34, vec![Arg(0), Pointer(OUTPUT), Constant(64)]),
            (22, 35, vec![Arg(0), PointerArg(1)]),
            (23, 36, vec![Arg(0)]),
            (24, 37, vec![]),
            (25, 38, vec![Arg(0)]),
            (26, 39, vec![]),
        ] {
            bodies[entry] = vec![
                Op::Write(OUTPUT, Constant(0)),
                call(ordinal, &args, None),
                Op::Return(Word(OUTPUT)),
            ];
            if matches!(entry, 23 | 24 | 25 | 26) {
                bodies[entry] = vec![call(ordinal, &args, Some(OUTPUT)), Op::Return(Word(OUTPUT))];
            }
        }
    } else {
        bodies[23] = vec![call(60, &[Arg(0)], Some(OUTPUT)), Op::Return(Word(OUTPUT))];
        bodies[29] = vec![
            call(63, &[PointerArg(0)], Some(OUTPUT)),
            Op::Return(Word(OUTPUT)),
        ];
    }
    bodies[27] = vec![call(catcher, &[], Some(OUTPUT)), Op::Return(Word(OUTPUT))];
    bodies[28] = vec![call(set_catcher, &[Arg(0)], None), Op::Return(Constant(0))];
    bodies
}
fn data() -> [u8; 1024] {
    let mut data = [0; 1024];
    data[SWITCH_KEY as usize..SWITCH_KEY as usize + 4].copy_from_slice(&u32::MAX.to_le_bytes());
    for (offset, text) in [
        (UPPER, b"upper\0".as_slice()),
        (LOWER, b"lower\0"),
        (FORWARD, b"+forward\0"),
        (FORWARD_CASE, b"+FORWARD\0"),
        (ERROR, b"input fixture failure\0"),
        (RELEASE_BIND, b"+forward;set input_release_order released\0"),
        (RELEASE_CALLBACK, b"set input_release_order callback\n\0"),
    ] {
        data[offset as usize..offset as usize + text.len()].copy_from_slice(text);
    }
    data
}

fn qvm_value(ops: &mut Vec<(Opcode, i32)>, value: Value) {
    use Opcode::*;
    match value {
        Value::Constant(value) | Value::Pointer(value) => ops.push((Const, value as i32)),
        Value::Arg(index) | Value::PointerArg(index) | Value::ReadArg(index) => {
            ops.extend([(Local, 108 + i32::from(index) * 4), (Load4, 0)]);
            if matches!(value, Value::ReadArg(_)) {
                ops.push((Load4, 0));
            }
        }
        Value::Word(offset) => ops.extend([(Const, offset as i32), (Load4, 0)]),
    }
}
fn qvm_ops(ops: &mut Vec<(Opcode, i32)>, body: &[Op]) {
    use Opcode::*;
    for op in body {
        match op {
            Op::Write(offset, value) => {
                ops.push((Const, *offset as i32));
                qvm_value(ops, *value);
                ops.push((Store4, 0));
            }
            Op::WriteAt(arg, value) => {
                qvm_value(ops, Value::PointerArg(*arg));
                qvm_value(ops, *value);
                ops.push((Store4, 0));
            }
            Op::Increment(offset) => ops.extend([
                (Const, *offset as i32),
                (Const, *offset as i32),
                (Load4, 0),
                (Const, 1),
                (Add, 0),
                (Store4, 0),
            ]),
            Op::Call(number, args, output) => {
                if let Some(offset) = output {
                    ops.push((Const, *offset as i32));
                }
                for (index, &arg) in args.iter().enumerate() {
                    qvm_value(ops, arg);
                    ops.push((Arg, 8 + index as i32 * 4));
                }
                ops.extend([(Const, -(*number as i32) - 1), (Call, 0)]);
                ops.push((if output.is_some() { Store4 } else { Pop }, 0));
            }
            Op::IfEqual(left, right, body) => {
                qvm_value(ops, *left);
                qvm_value(ops, *right);
                let branch = ops.len();
                ops.push((Ne, 0));
                qvm_ops(ops, body);
                ops[branch].1 = ops.len() as i32;
            }
            Op::Return(value) => {
                qvm_value(ops, *value);
                ops.push((Leave, 96));
            }
        }
    }
}
fn qvm_input(bodies: Vec<Vec<Op>>, ui: bool) -> Program {
    use Opcode::*;
    let mut ops = vec![(Enter, 96)];
    let mut branches = vec![];
    for command in 0..bodies.len() {
        ops.extend([(Local, 104), (Load4, 0), (Const, command as i32), (Eq, 0)]);
        branches.push(ops.len() - 1);
    }
    ops.extend([(Const, 0), (Leave, 96)]);
    for (branch, body) in branches.into_iter().zip(bodies) {
        ops[branch].1 = ops.len() as i32;
        qvm_ops(&mut ops, &body);
    }
    Program::Qvm {
        vm: Box::new(service_program::program(&ops, &data())),
        imports: if ui {
            &qa_compat::abi::Q3_UI
        } else {
            &Q3_CLIENT
        },
    }
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
fn native_value(code: &mut Vec<u8>, value: Value, base: u64) {
    match value {
        Value::Constant(value) => {
            code.push(0xb8);
            code.extend(value.to_le_bytes());
        }
        Value::Arg(index) | Value::PointerArg(index) | Value::ReadArg(index) => {
            code.extend([0x48, 0x8b, 0x44, 0x24, 0x50 + index * 8]);
            if !matches!(value, Value::Arg(_)) {
                if matches!(value, Value::PointerArg(_)) {
                    // A guest null string stays null; data offset zero remains
                    // readable through the distinct numeric ReadArg export.
                    code.extend([0x48, 0x85, 0xc0, 0x74, 13]);
                }
                code.extend([0x49, 0xbb]);
                code.extend(base.to_le_bytes());
                code.extend([0x4c, 0x01, 0xd8]);
                if matches!(value, Value::ReadArg(_)) {
                    code.extend([0x8b, 0x00]);
                }
            }
        }
        Value::Word(offset) | Value::Pointer(offset) => {
            code.extend([0x48, 0xb8]);
            code.extend((base + u64::from(offset)).to_le_bytes());
            if matches!(value, Value::Word(_)) {
                code.extend([0x8b, 0x00]);
            }
        }
    }
}
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
fn native_ops(code: &mut Vec<u8>, body: &[Op], base: u64, callback: u64) {
    for op in body {
        match op {
            Op::Write(offset, value) => {
                native_value(code, *value, base);
                code.extend([0x49, 0xbb]);
                code.extend((base + u64::from(*offset)).to_le_bytes());
                code.extend([0x41, 0x89, 0x03]);
            }
            Op::WriteAt(index, value) => {
                native_value(code, *value, base);
                code.extend([0x49, 0x89, 0xc2]);
                native_value(code, Value::PointerArg(*index), base);
                code.extend([0x44, 0x89, 0x10]);
            }
            Op::Increment(offset) => {
                code.extend([0x49, 0xbb]);
                code.extend((base + u64::from(*offset)).to_le_bytes());
                code.extend([0x41, 0xff, 0x03]);
            }
            Op::Call(number, args, output) => {
                for (index, &arg) in args.iter().enumerate() {
                    native_value(code, arg, base);
                    if index < 3 {
                        code.extend(
                            [[0x48, 0x89, 0xc2], [0x49, 0x89, 0xc0], [0x49, 0x89, 0xc1]][index],
                        );
                    } else {
                        // Microsoft x64: the ordinal occupies RCX. Import
                        // arguments after RDX/R8/R9 use the caller's stack.
                        assert!(index < 9);
                        code.extend([0x48, 0x89, 0x44, 0x24, 0x20 + (index as u8 - 3) * 8]);
                    }
                }
                code.push(0xb9);
                code.extend(number.to_le_bytes());
                code.extend([0x48, 0xb8]);
                code.extend(callback.to_le_bytes());
                code.extend([0xff, 0xd0]);
                if let Some(offset) = output {
                    code.extend([0x49, 0xbb]);
                    code.extend((base + u64::from(*offset)).to_le_bytes());
                    code.extend([0x41, 0x89, 0x03]);
                }
            }
            Op::IfEqual(left, right, body) => {
                native_value(code, *left, base);
                code.extend([0x89, 0x44, 0x24, 0x70]);
                native_value(code, *right, base);
                code.extend([0x3b, 0x44, 0x24, 0x70, 0x0f, 0x85]);
                let branch = code.len();
                code.extend([0; 4]);
                native_ops(code, body, base, callback);
                let delta = (code.len() - branch - 4) as i32;
                code[branch..branch + 4].copy_from_slice(&delta.to_le_bytes());
            }
            Op::Return(value) => {
                native_value(code, *value, base);
                code.extend([0x48, 0x81, 0xc4, 0x88, 0, 0, 0, 0xc3]);
            }
        }
    }
}
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
fn native_input(bodies: Vec<Vec<Op>>, ui: bool) -> Program {
    use native_image::*;
    use qa_compat::native::{NamedExport, Vm};
    use qa_formats::program::native::{Image, LoadRole};
    use qa_platform::native::NativeScalar;
    let mut file = pe_export_fixture(64);
    put(&mut file, 392 + 8, 0x7000, 4);
    put(&mut file, 392 + 16, 0x7000, 4);
    put(&mut file, 208, 0x8000, 4);
    file.resize(0x7200, 0);
    pe_rva(&mut file, 0x1140, 0x1800, 4);
    pe_text(&mut file, 0x1800, &[0xc3]);
    let image = Image::parse(&file, None, LoadRole::Library).unwrap();
    let base = image.base;
    let mut vm = Vm::map_image(
        image,
        &(0..bodies.len())
            .map(|command| NamedExport {
                name: b"GetGameAPI",
                command: Some(command as u32),
                parameters: &[NativeScalar::Word; 13],
                result: NativeScalar::Word,
            })
            .collect::<Vec<_>>(),
        &[],
        Duration::from_secs(3),
    )
    .unwrap();
    let mut code = vec![];
    for (command, body) in bodies.into_iter().enumerate() {
        code.extend([0x83, 0xf9, command as u8, 0x0f, 0x85]);
        let branch = code.len();
        code.extend([0; 4]);
        code.extend([0x48, 0x81, 0xec, 0x88, 0, 0, 0]);
        code.extend([0x48, 0x89, 0x54, 0x24, 0x50]);
        code.extend([0x4c, 0x89, 0x44, 0x24, 0x58]);
        code.extend([0x4c, 0x89, 0x4c, 0x24, 0x60]);
        native_ops(&mut code, &body, base + 0x6000, vm.import_callback());
        let delta = (code.len() - branch - 4) as i32;
        code[branch..branch + 4].copy_from_slice(&delta.to_le_bytes());
    }
    code.extend([0x31, 0xc0, 0xc3]);
    assert!(code.len() < 0x4800);
    vm.process.memory_mut().unwrap()[0x1800..0x1800 + code.len()].copy_from_slice(&code);
    vm.process.memory_mut().unwrap()[0x6000..0x6400].copy_from_slice(&data());
    Program::Native {
        vm: Box::new(vm),
        imports: if ui {
            &qa_compat::abi::Q3_UI
        } else {
            &Q3_CLIENT
        },
    }
}

fn module(seat: usize, ui: bool) -> ModuleId {
    ModuleId((seat * 2 + if ui { 1 } else { 2 }) as u16)
}
fn invoke(host: &mut FrameHost, seat: usize, ui: bool, entry: u32, words: &[u64]) -> u32 {
    let id = module(seat, ui);
    host.call_module_export(
        id,
        Export::words(CallbackId(entry), words),
        ThinkTime::Milliseconds(1),
    )
    .unwrap();
    match host.module_counts(id).unwrap().last_result.unwrap() {
        ModuleResult::Qvm(value) => value as u32,
        ModuleResult::Native(value) => value as u32,
        _ => unreachable!(),
    }
}
fn read(host: &mut FrameHost, seat: usize, ui: bool, offset: u32) -> u32 {
    invoke(host, seat, ui, 10, &[u64::from(offset)])
}
fn write(host: &mut FrameHost, seat: usize, ui: bool, offset: u32, value: u32) {
    invoke(host, seat, ui, 11, &[u64::from(offset), u64::from(value)]);
}
fn catch(host: &mut FrameHost, seat: usize, ui: bool, mask: u32) {
    assert_eq!(invoke(host, seat, ui, 28, &[u64::from(mask)]), 0);
}
fn key(device: DeviceId, control: u16, down: bool, repeat: bool) -> EventKind<'static> {
    EventKind::Key {
        device,
        code: control,
        symbol: 0,
        down,
        repeat,
    }
}
#[derive(Default)]
struct Source {
    time: u64,
    polls: u64,
    pending: VecDeque<Vec<EventKind<'static>>>,
    captures: Vec<bool>,
}
impl FrameSource for Source {
    fn begin_frame(&mut self) -> Duration {
        Duration::from_millis(self.time)
    }
    fn poll_events(&mut self, queue: &mut SysEventQueue) {
        self.polls += 1;
        for kind in self.pending.pop_front().unwrap_or_default() {
            queue
                .push(SysEvent {
                    time: EventTime::from_milliseconds(self.time),
                    kind,
                })
                .unwrap();
        }
        queue
            .push(SysEvent {
                time: EventTime::from_milliseconds(self.time),
                kind: EventKind::Time,
            })
            .unwrap();
    }
    fn wait_time(&mut self, span: Duration) -> Duration {
        self.time += span.as_millis() as u64;
        Duration::from_millis(self.time)
    }
    fn request_mouse_capture(&mut self, requested: bool) {
        self.captures.push(requested);
    }
    fn elapsed(&self) -> Duration {
        Duration::ZERO
    }
    fn present(&mut self) {}
}
fn frame(
    host: &mut FrameHost,
    source: &mut Source,
    first: Vec<EventKind<'static>>,
    second: Vec<EventKind<'static>>,
) -> FrameResult {
    source.time += 16;
    source.pending.extend([first, second]);
    let polls = source.polls;
    let result = host.frame(source, true);
    assert_eq!(result.drains, 2);
    assert_eq!(source.polls - polls, 2);
    result
}
fn program(native: bool, ui: bool) -> Program {
    program_from(native, ui, bodies(ui))
}
fn program_from(native: bool, ui: bool, bodies: Vec<Vec<Op>>) -> Program {
    if native {
        #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
        {
            return native_input(bodies, ui);
        }
        #[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
        {
            panic!("owned-child fixture is unavailable on this target");
        }
    }
    qvm_input(bodies, ui)
}
fn fixture(native: bool) -> FrameHost {
    fixture_with(native, |_, _, _| {})
}
fn fixture_with(
    native: bool,
    mut customize: impl FnMut(usize, bool, &mut Vec<Vec<Op>>),
) -> FrameHost {
    fixture_config(
        native,
        |seat, ui, bodies| {
            customize(seat, ui, bodies);
            Vec::new()
        },
        |_| {},
    )
}
fn fixture_config(
    native: bool,
    customize: impl FnMut(usize, bool, &mut Vec<Vec<Op>>) -> Vec<Export>,
    before_initialize: impl FnOnce(&mut FrameHost),
) -> FrameHost {
    fixture_roles(native, native, customize, before_initialize)
}
fn fixture_roles(
    native: bool,
    cgame_native: bool,
    mut customize: impl FnMut(usize, bool, &mut Vec<Vec<Op>>) -> Vec<Export>,
    before_initialize: impl FnOnce(&mut FrameHost),
) -> FrameHost {
    let (mut runtime, ids) = runtime(2);
    let mut requests = vec![];
    for (seat, &client) in ids.iter().enumerate() {
        for ui in [true, false] {
            let mut bodies = bodies(ui);
            let initialize = customize(seat, ui, &mut bodies);
            let mut request = request(
                &mut runtime,
                module(seat, ui),
                client,
                program_from(if ui { native } else { cgame_native }, ui, bodies),
            );
            request.initialize = initialize;
            request.entries = (0..32).collect();
            request.instruction_budget = 10000;
            request.frame = Export::words(CallbackId(0), &[]);
            if !ui {
                request.frame.client = Some(client);
            }
            requests.push(request);
        }
    }
    // A fifth real module supplies a healthy SERVER provider, independent of
    // the two seats' client callbacks and their failure/teardown lifetimes.
    let mut server = request(&mut runtime, ModuleId(5), ids[0], program(native, false));
    match &mut server.program {
        Program::Qvm { imports, .. } => *imports = &qa_compat::abi::Q3_SERVER,
        #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
        Program::Native { imports, .. } => *imports = &qa_compat::abi::Q3_SERVER,
        _ => unreachable!(),
    }
    server.phase = Phase::Server;
    server.frame = Export::words(CallbackId(0), &[]);
    server.context.console.side = qa_console::catalog::Scope::Server;
    server.entries = (0..32).collect();
    requests.push(server);
    let mut host = FrameHost::load_modules(
        Console::new(Context::default()).unwrap(),
        runtime,
        TickRate::FrameDriven,
        requests,
    )
    .unwrap();
    host.local_clients = [Some(ids[0]), Some(ids[1]), None, None];
    assert!(
        host.runtime
            .input
            .assign(DeviceId::Keyboard(10), SeatId::FIRST)
    );
    assert!(
        host.runtime
            .input
            .assign(DeviceId::Keyboard(11), SeatId::new(1).unwrap())
    );
    assert!(
        host.runtime
            .input
            .assign(DeviceId::Mouse(10), SeatId::FIRST)
    );
    assert!(
        host.runtime
            .input
            .assign(DeviceId::Mouse(11), SeatId::new(1).unwrap())
    );
    before_initialize(&mut host);
    host.client_modules();
    host
}

#[path = "input/clear_states.rs"]
mod clear_states;
#[path = "input/late_assets.rs"]
mod late_assets;
#[path = "input/late_sounds.rs"]
mod late_sounds;

pub(super) fn late_native_sounds(native: bool) {
    late_sounds::run(native);
}

pub(super) fn queued_native_input(native: bool) {
    late_assets::run(native);
    clear_states::run(native);
    queued_callbacks_and_imports(native);
    binding_capture_releases_and_atomic_catchers(native);
    menu_failure_and_teardown(native);
    println!(
        "queued native input fixtures passed: {}",
        if native { "owned child" } else { "QVM" }
    );
}
fn queued_callbacks_and_imports(native: bool) {
    let mut host = fixture(native);
    let mut source = Source::default();
    let a = DeviceId::Keyboard(10);
    let b = DeviceId::Keyboard(11);
    invoke(&mut host, 0, true, 22, &[65, u64::from(UPPER)]);
    invoke(&mut host, 0, true, 22, &[97, u64::from(LOWER)]);
    assert_eq!(invoke(&mut host, 0, true, 21, &[65]), word(b"uppe"));
    assert_eq!(invoke(&mut host, 0, true, 21, &[97]), word(b"lowe"));
    assert_eq!(invoke(&mut host, 0, true, 20, &[65]), u32::from(b'A'));
    assert_eq!(invoke(&mut host, 0, true, 20, &[97]), u32::from(b'a'));
    invoke(&mut host, 0, true, 22, &[98, u64::from(EMPTY)]);
    assert_eq!(invoke(&mut host, 0, true, 21, &[98]), 0);
    assert_eq!(invoke(&mut host, 0, false, 29, &[u64::from(EMPTY)]), 98);
    assert_eq!(invoke(&mut host, 0, false, 29, &[0]), u32::MAX);
    invoke(&mut host, 0, true, 25, &[17]);
    assert_eq!(invoke(&mut host, 1, true, 24, &[]), 1);
    invoke(&mut host, 0, true, 25, &[0]);
    assert_eq!(invoke(&mut host, 0, true, 24, &[]), 0);
    invoke(&mut host, 0, true, 22, &[119, u64::from(FORWARD)]);
    assert_eq!(
        invoke(&mut host, 0, false, 29, &[u64::from(FORWARD_CASE)]),
        119
    );
    catch(&mut host, 0, true, key_catchers::UI | key_catchers::CGAME);
    catch(&mut host, 1, false, key_catchers::CGAME);
    frame(
        &mut host,
        &mut source,
        vec![
            key(a, 4, true, false),
            key(a, 4, true, true),
            key(b, 4, true, false),
        ],
        vec![],
    );
    assert_eq!(read(&mut host, 0, true, KEY_COUNT), 2);
    assert!(source.captures.iter().any(|requested| *requested));
    assert!(host.local_worlds.iter().all(Option::is_none));
    assert_eq!(read(&mut host, 0, true, LAST_KEY), 97);
    assert_eq!(read(&mut host, 0, true, HELD_KEY), 1);
    // These callbacks run before the first TIME marker updates host.time.
    assert_eq!(read(&mut host, 0, true, CALLBACK_TIME), 16);
    assert_eq!(read(&mut host, 1, false, CALLBACK_TIME), 16);
    assert_eq!(read(&mut host, 0, false, KEY_COUNT), 0);
    assert_eq!(read(&mut host, 1, false, KEY_COUNT), 1);
    assert_eq!(read(&mut host, 1, false, HELD_A), 1);
    assert_eq!(invoke(&mut host, 0, true, 23, &[97]), 1);
    assert_eq!(invoke(&mut host, 1, false, 23, &[97]), 1);
    frame(
        &mut host,
        &mut source,
        vec![key(a, 4, false, false), key(b, 4, false, false)],
        vec![
            EventKind::Text {
                device: a,
                text: "é",
            },
            EventKind::Mouse {
                device: DeviceId::Mouse(10),
                dx: -7,
                dy: 9,
            },
            EventKind::Text {
                device: b,
                text: "ignored",
            },
            EventKind::Mouse {
                device: DeviceId::Mouse(11),
                dx: 3,
                dy: -4,
            },
        ],
    );
    assert_eq!(read(&mut host, 0, true, KEY_COUNT), 4);
    assert_eq!(read(&mut host, 0, true, LAST_KEY), u32::from('é') | 1024);
    assert_eq!(read(&mut host, 0, true, LAST_DOWN), 1);
    assert_eq!(read(&mut host, 0, true, MOUSE_COUNT), 1);
    assert_eq!(read(&mut host, 0, true, MOUSE_X), (-7i32) as u32);
    assert_eq!(read(&mut host, 0, true, MOUSE_Y), 9);
    assert_eq!(read(&mut host, 1, false, KEY_COUNT), 2);
    assert_eq!(read(&mut host, 1, false, HELD_KEY), 0);
    assert_eq!(read(&mut host, 1, false, CALLBACK_TIME), 32);
    assert_eq!(read(&mut host, 1, false, MOUSE_COUNT), 1);
    assert_eq!(read(&mut host, 1, false, MOUSE_X), 3);
    assert_eq!(read(&mut host, 1, false, MOUSE_Y), (-4i32) as u32);
    frame(
        &mut host,
        &mut source,
        vec![key(a, 4, true, false), key(b, 4, true, false)],
        vec![],
    );
    invoke(&mut host, 0, true, 26, &[]);
    assert_eq!(invoke(&mut host, 0, true, 23, &[97]), 0);
    assert_eq!(invoke(&mut host, 1, false, 23, &[97]), 1);
    catch(&mut host, 0, true, key_catchers::UI);
    write(&mut host, 0, true, SWITCH_KEY, u32::from('a') | 1024);
    write(&mut host, 0, true, SWITCH_DOWN, 1);
    write(&mut host, 0, true, SWITCH_MASK, 0);
    let before = read(&mut host, 0, true, KEY_COUNT);
    frame(
        &mut host,
        &mut source,
        vec![EventKind::Text {
            device: a,
            text: "ab",
        }],
        vec![],
    );
    assert_eq!(read(&mut host, 0, true, KEY_COUNT), before + 1);
    assert_eq!(read(&mut host, 0, true, LAST_KEY), u32::from('a') | 1024);
    assert_eq!(invoke(&mut host, 0, true, 27, &[]), 0);
    catch(&mut host, 0, true, key_catchers::UI);
    write(&mut host, 0, true, SWITCH_KEY, 184);
    write(&mut host, 0, true, SWITCH_DOWN, 1);
    write(&mut host, 0, true, SWITCH_MASK, key_catchers::CGAME);
    let before = read(&mut host, 0, true, KEY_COUNT);
    let cg_before = read(&mut host, 0, false, KEY_COUNT);
    frame(
        &mut host,
        &mut source,
        vec![EventKind::MouseWheel {
            device: DeviceId::Mouse(10),
            x: 0,
            y: 1,
        }],
        vec![],
    );
    assert_eq!(read(&mut host, 0, true, KEY_COUNT), before + 1);
    assert_eq!(read(&mut host, 0, true, HELD_KEY), 1);
    assert_eq!(read(&mut host, 0, false, KEY_COUNT), cg_before + 1);
    assert_eq!(read(&mut host, 0, false, LAST_KEY), 184);
    assert_eq!(read(&mut host, 0, false, LAST_DOWN), 0);
    assert_eq!(read(&mut host, 0, false, HELD_KEY), 0);
}
fn binding_capture_releases_and_atomic_catchers(native: bool) {
    let mut host = fixture(native);
    let mut source = Source::default();
    let keyboard = DeviceId::Keyboard(10);
    invoke(&mut host, 0, true, 22, &[119, u64::from(RELEASE_BIND)]);
    catch(&mut host, 0, true, key_catchers::UI);
    let caught = frame(
        &mut host,
        &mut source,
        vec![key(keyboard, 26, true, false)],
        vec![],
    );
    assert_eq!(caught.commands[0].movement[0], 0.0);
    assert_eq!(read(&mut host, 0, true, HELD_KEY), 1);
    catch(&mut host, 0, true, 0);
    assert_eq!(
        frame(&mut host, &mut source, vec![], vec![]).commands[0].movement[0],
        0.0
    );
    frame(
        &mut host,
        &mut source,
        vec![key(keyboard, 26, false, false)],
        vec![],
    );
    let acquired = frame(
        &mut host,
        &mut source,
        vec![key(keyboard, 26, true, false)],
        vec![],
    );
    // Q3 samples elapsed key time, so a press at the command endpoint first
    // proves acquisition on the following held frame.
    assert_eq!(acquired.commands[0].movement[0], 0.0);
    assert!(frame(&mut host, &mut source, vec![], vec![]).commands[0].movement[0] > 0.0);
    catch(&mut host, 0, true, key_catchers::UI);
    write(&mut host, 0, true, CLEAR_ON_UP, 1);
    let released = frame(
        &mut host,
        &mut source,
        vec![key(keyboard, 26, false, false)],
        vec![],
    );
    assert_eq!(read(&mut host, 0, true, LAST_DOWN), 0);
    assert_eq!(read(&mut host, 0, true, HELD_KEY), 0);
    assert_eq!(invoke(&mut host, 0, true, 27, &[]), 0);
    assert_eq!(released.commands[0].movement[0], 0.0);
    let order = host.console.cvars.find("input_release_order").unwrap();
    assert_eq!(host.console.cvars.text(order), "callback");
    assert_eq!(
        frame(&mut host, &mut source, vec![], vec![]).commands[0].movement[0],
        0.0
    );
}
fn menu_failure_and_teardown(native: bool) {
    let mut host = fixture(native);
    let mut source = Source::default();
    let keyboard = DeviceId::Keyboard(10);
    frame(
        &mut host,
        &mut source,
        vec![key(keyboard, 41, true, false)],
        vec![],
    );
    assert_eq!(read(&mut host, 0, true, MENU_COUNT), 1);
    assert_eq!(read(&mut host, 0, true, MENU_VALUE), 1);
    assert_eq!(invoke(&mut host, 0, true, 27, &[]), key_catchers::UI);
    frame(
        &mut host,
        &mut source,
        vec![key(keyboard, 41, false, false)],
        vec![],
    );
    catch(&mut host, 0, true, key_catchers::UI | key_catchers::CGAME);
    frame(
        &mut host,
        &mut source,
        vec![key(keyboard, 41, true, false)],
        vec![],
    );
    assert_eq!(read(&mut host, 0, false, HANDLING_COUNT), 1);
    assert_eq!(read(&mut host, 0, false, HANDLING_VALUE), 0);
    assert_eq!(read(&mut host, 0, false, CATCHERS), key_catchers::UI);
    catch(&mut host, 0, true, 0);
    frame(
        &mut host,
        &mut source,
        vec![key(keyboard, 41, false, false)],
        vec![],
    );
    frame(
        &mut host,
        &mut source,
        vec![key(keyboard, 41, true, false)],
        vec![],
    );
    assert_eq!(read(&mut host, 0, true, MENU_COUNT), 2);
    assert_eq!(read(&mut host, 0, true, MENU_VALUE), 2);
    catch(&mut host, 0, true, key_catchers::UI);
    catch(&mut host, 1, true, key_catchers::UI);
    let seat0_before = read(&mut host, 0, true, KEY_COUNT);
    let seat1_before = read(&mut host, 1, true, KEY_COUNT);
    frame(
        &mut host,
        &mut source,
        vec![EventKind::Text {
            device: DeviceId::Keyboard(0),
            text: "x",
        }],
        vec![],
    );
    assert_eq!(read(&mut host, 0, true, KEY_COUNT), seat0_before);
    assert_eq!(read(&mut host, 1, true, KEY_COUNT), seat1_before + 1);
    assert_eq!(read(&mut host, 1, true, LAST_KEY), u32::from(b'x') | 1024);
    assert!(host.shutdown_module(module(0, true)));
    assert_eq!(host.runtime.input.key_catchers(SeatId::FIRST), 0);
    assert_eq!(invoke(&mut host, 1, true, 27, &[]), key_catchers::UI);
    assert_eq!(
        host.runtime
            .input
            .catcher_module(SeatId::new(1).unwrap(), key_catchers::UI),
        Some(module(1, true))
    );
    frame(
        &mut host,
        &mut source,
        vec![EventKind::Text {
            device: DeviceId::Keyboard(0),
            text: "y",
        }],
        vec![],
    );
    assert_eq!(read(&mut host, 1, true, KEY_COUNT), seat1_before + 2);
    assert_eq!(read(&mut host, 1, true, LAST_KEY), u32::from(b'y') | 1024);
    write(&mut host, 1, true, FAIL, 1);
    let before = host.runtime.server.world_frame;
    let provider_before = host.module_counts(ModuleId(5)).unwrap().calls;
    let result = frame(
        &mut host,
        &mut source,
        vec![key(DeviceId::Keyboard(11), 4, true, false)],
        vec![key(keyboard, 5, true, false)],
    );
    assert!(result.server_ticks > 0);
    assert!(host.runtime.server.world_frame > before);
    assert!(host.module_counts(ModuleId(5)).unwrap().calls > provider_before);
    assert_eq!(
        host.module_state(ModuleId(5)),
        Some(qa_app::modules::State::Running)
    );
    assert_eq!(invoke(&mut host, 0, false, 23, &[98]), 1);
    assert_ne!(
        host.module_state(module(1, true)),
        Some(qa_app::modules::State::Running)
    );
    assert_eq!(host.runtime.input.key_catchers(SeatId::new(1).unwrap()), 0);
    assert!(host.runtime.server.clients[0].connection.is_some());
}
