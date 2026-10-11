use qa_compat::{command::NativeCommand, memory::ModuleMemory, services::CallError};

#[test]
fn native_command_pointers_keep_raw_tokens_tail_and_empty_indices() {
    let base = 0x10000;
    let mut bytes = vec![0xa5; NativeCommand::byte_length()];
    let mut command = NativeCommand::load(base).unwrap();
    let mut memory = ModuleMemory::borrow(base, &mut bytes).unwrap();
    command
        .prepare(
            &mut memory,
            &[b"say", b"\x80quoted space", b"third"],
            b"\"\x80quoted space\"   third ",
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
    command.prepare(&mut memory, &[b"x"], b" ").unwrap();
    assert_eq!(command.argv(0), argv);
    assert_eq!(memory.cstring(argv).unwrap(), b"x");
    assert_eq!(memory.cstring(command.argv(1)).unwrap(), b"");
    assert_eq!(memory.cstring(command.args()).unwrap(), b" ");
    assert_eq!(memory.read(command.args() + 2, 1).unwrap(), &[b'q']);
    command.prepare(&mut memory, &[], b"").unwrap();
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
        assert_eq!(command.prepare(&mut memory, tokens, tail), Err(error));
    }
    let tokens = [b"".as_slice(); 1025];
    assert_eq!(
        command.prepare(&mut memory, &tokens, b""),
        Err(CallError::Capacity)
    );
    let large = vec![b'x'; NativeCommand::byte_length()];
    assert_eq!(
        command.prepare(&mut memory, &[&large], b""),
        Err(CallError::Capacity)
    );
    assert_eq!(
        command.prepare(&mut memory, &[], &large),
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
            &[b"x"],
            b""
        ),
        Err(CallError::Memory)
    );
    assert_eq!(short, [0xa5; 16]);
}
