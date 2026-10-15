use super::*;

const ENTRY: usize = 0x1600;
const SEGMENT_BIAS: usize = 0x2000;
const SYSCALL: usize = 0x1ac0;
const TEXT: usize = 0x1b00;
const NAME: usize = 0x1b50;
const NESTED: usize = 0x1b80;
const ARGUMENT: usize = 0x1be0;

fn rip(code: &mut Vec<u8>, operation: &[u8], target: usize) {
    code.extend_from_slice(operation);
    code.extend((target as i32 - (ENTRY + code.len() + 4) as i32).to_le_bytes());
}
fn number(code: &mut Vec<u8>, value: u32) {
    code.push(0xbf); // mov edi,number
    code.extend(value.to_le_bytes());
}
fn syscall(code: &mut Vec<u8>) {
    code.extend([0x31, 0xc0]); // native varargs has no vector arguments
    rip(code, &[0xff, 0x15], SYSCALL);
}
fn now(code: &mut Vec<u8>, target: usize) {
    number(code, 14);
    code.extend([0x31, 0xf6]);
    rip(code, &[0x48, 0x8d, 0x15], target);
    syscall(code);
}
fn finish(code: &mut Vec<u8>) {
    code.extend([0x48, 0x83, 0xc4, 8, 0xc3]);
}
fn fixture(nested: bool) -> FrameHost {
    let mut code = vec![0x48, 0x83, 0xec, 8];
    code.extend([0x83, 0xff, 9, 0x0f, 0x84]); // cmp edi,9; je callback
    let callback_jump = code.len();
    code.extend([0; 4]);
    now(&mut code, TEXT);
    number(&mut code, 6);
    rip(&mut code, &[0x48, 0x8d, 0x35], NAME);
    syscall(&mut code);
    finish(&mut code);
    let callback = code.len();
    code[callback_jump..callback_jump + 4]
        .copy_from_slice(&(callback as i32 - callback_jump as i32 - 4).to_le_bytes());
    // Read raw Other argv through the same common copy ABI used by stock Q3.
    number(&mut code, 9);
    code.extend([0xbe, 1, 0, 0, 0]);
    rip(&mut code, &[0x48, 0x8d, 0x15], ARGUMENT);
    code.extend([0xb9, 16, 0, 0, 0]);
    syscall(&mut code);
    rip(&mut code, &[0x80, 0x3d], ARGUMENT);
    // cmp byte [rip+displacement],0xff has an additional immediate byte.
    let displacement = code.len() - 4;
    let relative = ARGUMENT as i32 - (ENTRY + code.len() + 1) as i32;
    code[displacement..displacement + 4].copy_from_slice(&relative.to_le_bytes());
    code.extend([0xff, 0x0f, 0x85]);
    let bad_byte = code.len();
    code.extend([0; 4]);
    now(&mut code, NESTED);
    number(&mut code, 8);
    syscall(&mut code);
    code.extend([0x83, 0xf8, 2, 0x0f, 0x85]); // outer callback argv survives inner EXEC_NOW
    let bad_count = code.len();
    code.extend([0; 4]);
    code.extend([0xb8, 1, 0, 0, 0]);
    finish(&mut code);
    let failed = code.len();
    code.extend([0x31, 0xc0]);
    finish(&mut code);
    for jump in [bad_byte, bad_count] {
        code[jump..jump + 4].copy_from_slice(&(failed as i32 - jump as i32 - 4).to_le_bytes());
    }
    assert!(code.len() <= 0x140);
    // Keep the builder's 48-byte symbol entry clear of its adjacent symbol table.
    let mut entry = vec![0xe9];
    entry.extend((ENTRY as i32 - 0x13c0 - 5).to_le_bytes());
    let mut image = function_image(Encoding::Elf, &entry);
    image.bytes[ENTRY + SEGMENT_BIAS..ENTRY + SEGMENT_BIAS + code.len()].copy_from_slice(&code);
    let exports = [
        NamedExport {
            parameters: &[NativeScalar::Word; 13],
            result: NativeScalar::Word,
            name: b"vmMain",
            command: Some(10),
        },
        NamedExport {
            parameters: &[NativeScalar::Word],
            result: NativeScalar::Void,
            name: b"dllEntry",
            command: None,
        },
        NamedExport {
            parameters: &[NativeScalar::Word; 13],
            result: NativeScalar::Word,
            name: b"vmMain",
            command: Some(9),
        },
    ];
    let mut vm = Vm::map_image(image, &exports, &[], Duration::from_secs(3)).unwrap();
    assert_eq!(vm.export_for_command(9), Some(2));
    assert!(!vm.declares_export(9));
    let pointer = vm.import_callback();
    let memory = vm.process.memory_mut().unwrap();
    for (at, bytes) in [
        (
            TEXT,
            if nested {
                b"native_callback \xff\0".as_slice()
            } else {
                b"set _qa_native_now 17\0".as_slice()
            },
        ),
        (
            NAME,
            if nested {
                b"_qa_native_nested\0".as_slice()
            } else {
                b"_qa_native_now\0".as_slice()
            },
        ),
        (NESTED, b"set _qa_native_nested 23\0".as_slice()),
    ] {
        memory[at + SEGMENT_BIAS..at + SEGMENT_BIAS + bytes.len()].copy_from_slice(bytes);
    }
    let mut runtime = Runtime::load(1, []).unwrap();
    let mut request = super::request(
        &mut runtime,
        ModuleId(1),
        RuleSetId::Quake3,
        TickRate::FrameDriven,
        vm,
    );
    request.entries = vec![0, 1];
    request.prepare = vec![Export::words(CallbackId(1), &[pointer])];
    let mut host = FrameHost::load_modules(
        Console::new(Context::default()).unwrap(),
        runtime,
        TickRate::FrameDriven,
        vec![request],
    )
    .unwrap();
    host.initialize_modules(Phase::Server);
    assert_eq!(host.module_state(ModuleId(1)), Some(State::Running));
    host
}

pub(super) fn run() {
    for nested in [false, true] {
        let mut host = fixture(nested);
        host.console
            .append("set _qa_native_queued 5\n", Context::default())
            .unwrap();
        host.call_module_export(
            ModuleId(1),
            Export::words(CallbackId(0), &[]),
            ThinkTime::Milliseconds(0),
        )
        .unwrap();
        assert_eq!(
            host.module_counts(ModuleId(1)).unwrap().last_result,
            Some(ModuleResult::Native(if nested { 23 } else { 17 }))
        );
        assert_eq!(host.module_counts(ModuleId(1)).unwrap().traps, 0);
        assert!(host.console.cvars.find("_qa_native_queued").is_none());
        host.execute_commands();
        assert_eq!(
            host.console
                .cvars
                .value(host.console.cvars.find("_qa_native_queued").unwrap()),
            5.0
        );
        host.shutdown_modules();
    }
}
