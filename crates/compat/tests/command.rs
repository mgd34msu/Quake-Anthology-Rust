use qa_compat::{command::NativeCommand, memory::ModuleMemory, services::CallError};
use qa_console::command_text::Arguments;
use qa_core::primitives::CommandArgs;

#[test]
fn native_command_pointers_keep_raw_tokens_tail_and_empty_indices() {
    let base = 0x10000;
    let mut bytes = vec![0xa5; NativeCommand::byte_length()];
    let mut command = NativeCommand::load(base).unwrap();
    let mut memory = ModuleMemory::borrow(base, &mut bytes).unwrap();
    command
        .prepare(
            &mut memory,
            &Arguments::from_command(CommandArgs {
                argv: &[b"say", b"\x80quoted space", b"third"],
                raw: b"\"\x80quoted space\"   third ",
            }),
        )
        .unwrap();
    assert_eq!(memory.cstring(command.argv(0)).unwrap(), b"say");
    assert_eq!(
        memory.cstring(command.argv(1)).unwrap(),
        b"\x80quoted space"
    );
    assert_eq!(memory.cstring(command.argv(2)).unwrap(), b"third");
    for index in [3, u32::MAX] {
        assert_eq!(command.argv(index), base);
        assert_eq!(memory.cstring(command.argv(index)).unwrap(), b"");
    }
    assert_eq!(
        memory.cstring(command.args()).unwrap(),
        b"\"\x80quoted space\"   third "
    );
    let argv = command.argv(0);
    command
        .prepare(
            &mut memory,
            &Arguments::from_command(CommandArgs {
                argv: &[b"x"],
                raw: b" ",
            }),
        )
        .unwrap();
    assert_eq!(command.argv(0), argv);
    assert_eq!(memory.cstring(argv).unwrap(), b"x");
    assert_eq!(memory.cstring(command.argv(1)).unwrap(), b"");
    assert_eq!(memory.cstring(command.args()).unwrap(), b" ");
    assert_eq!(memory.read(command.args() + 2, 1).unwrap(), &[b'q']);
    command
        .prepare(
            &mut memory,
            &Arguments::from_command(CommandArgs {
                argv: &[],
                raw: b"",
            }),
        )
        .unwrap();
    assert_eq!(memory.cstring(command.argv(0)).unwrap(), b"");
    assert_eq!(memory.cstring(command.args()).unwrap(), b"");
}

#[test]
fn native_command_rejects_bad_extents_nuls_and_capacity_before_writing() {
    assert!(matches!(
        NativeCommand::load(u64::MAX),
        Err(CallError::Memory)
    ));
    let mut bytes = vec![0xa5; NativeCommand::byte_length()];
    let mut command = NativeCommand::load(0).unwrap();
    let mut memory = ModuleMemory::borrow(0, &mut bytes).unwrap();
    for (tokens, tail, error) in [
        (&[b"a\0b".as_slice()][..], b"".as_slice(), CallError::Text),
        (&[][..], b"a\0b".as_slice(), CallError::Text),
    ] {
        assert_eq!(
            command.prepare(
                &mut memory,
                &Arguments::from_command(CommandArgs {
                    argv: tokens,
                    raw: tail
                })
            ),
            Err(error)
        );
    }
    let tokens = [b"".as_slice(); 1025];
    assert_eq!(
        command.prepare(
            &mut memory,
            &Arguments::from_command(CommandArgs {
                argv: &tokens,
                raw: b""
            })
        ),
        Err(CallError::Capacity)
    );
    let large = vec![b'x'; NativeCommand::byte_length()];
    assert_eq!(
        command.prepare(
            &mut memory,
            &Arguments::from_command(CommandArgs {
                argv: &[&large],
                raw: b""
            })
        ),
        Err(CallError::Capacity)
    );
    assert_eq!(
        command.prepare(
            &mut memory,
            &Arguments::from_command(CommandArgs {
                argv: &[],
                raw: &large
            })
        ),
        Err(CallError::Capacity)
    );
    assert!(
        memory
            .read(0, NativeCommand::byte_length())
            .unwrap()
            .iter()
            .all(|&byte| byte == 0xa5)
    );
    let mut short = [0xa5; 16];
    assert_eq!(
        command.prepare(
            &mut ModuleMemory::borrow(0, &mut short).unwrap(),
            &Arguments::from_command(CommandArgs {
                argv: &[b"x"],
                raw: b""
            })
        ),
        Err(CallError::Memory)
    );
    assert_eq!(short, [0xa5; 16]);
}

#[test]
fn native_command_projects_parsed_spans_directly_and_preserves_high_byte_raw_args() {
    let mut parsed = qa_console::command_text::ParsedCommand::default();
    parsed
        .parse(
            b"say \"\xff space\"   third  ",
            qa_core::primitives::RuleSetId::Quake3,
        )
        .unwrap();
    let arguments = parsed.arguments();
    let base = 0x10000;
    let mut bytes = vec![0xa5; NativeCommand::byte_length()];
    let mut command = NativeCommand::load(base).unwrap();
    let mut memory = ModuleMemory::borrow(base, &mut bytes).unwrap();
    command.prepare(&mut memory, &arguments).unwrap();
    assert_eq!(memory.cstring(command.argv(0)).unwrap(), b"say");
    assert_eq!(memory.cstring(command.argv(1)).unwrap(), b"\xff space");
    assert_eq!(memory.cstring(command.argv(2)).unwrap(), b"third");
    assert_eq!(memory.cstring(command.argv(3)).unwrap(), b"");
    assert_eq!(
        memory.cstring(command.args()).unwrap(),
        b"\"\xff space\"   third  "
    );
    assert_eq!(arguments.get_bytes(1), b"\xff space");
    assert_eq!(arguments.raw_bytes(), b"\"\xff space\"   third  ");
}
