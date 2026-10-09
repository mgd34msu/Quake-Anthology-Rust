use qa_core::primitives::PrintKind;
use qa_network::{
    commands::packet::Protocol,
    outputs::{self, Error, Prints},
};

#[test]
fn print_opcodes_and_all_native_priority_bytes_remain_exact() -> Result<(), Error> {
    for (protocol, opcode) in [(Protocol::QuakeWorld28, 8), (Protocol::Quake2_34, 10)] {
        for level in 0..=255 {
            let mut bytes = [0xcc; 128];
            let n = outputs::print(
                protocol,
                PrintKind::Notify,
                level,
                b"raw\x80\xff",
                &mut bytes,
            )?;
            assert_eq!(&bytes[..n], &[opcode, level, b'r', b'a', b'w', 128, 255, 0]);
            assert_eq!(bytes[n], 0xcc);
            let print = Prints::new(protocol, &bytes[..n])
                .next()
                .ok_or(Error::Truncated)??;
            assert_eq!(print.level, Some(level));
            assert_eq!(print.text, b"raw\x80\xff");
        }
    }
    Ok(())
}

#[test]
fn center_layout_chat_nuls_and_concatenated_native_records_are_bounded() -> Result<(), Error> {
    let mut short = [0xcc; 3];
    assert!(matches!(
        outputs::print(Protocol::Quake2_34, PrintKind::Console, 2, b"a", &mut short),
        Err(Error::Message(_))
    ));
    assert_eq!(short, [0xcc; 3]);
    for (protocol, kind, expected) in [
        (
            Protocol::NetQuake15,
            PrintKind::Chat,
            &b"\x08\x01chat\0"[..],
        ),
        (
            Protocol::NetQuake15,
            PrintKind::Center,
            &b"\x1acenter\0"[..],
        ),
        (
            Protocol::QuakeWorld28,
            PrintKind::Center,
            &b"\x1acenter\0"[..],
        ),
        (Protocol::Quake2_34, PrintKind::Center, &b"\x0fcenter\0"[..]),
        (Protocol::Quake2_34, PrintKind::Layout, &b"\x04layout\0"[..]),
    ] {
        let text = match kind {
            PrintKind::Chat => &b"chat"[..],
            PrintKind::Layout => b"layout",
            _ => b"center",
        };
        let mut bytes = [0; 128];
        let n = outputs::print(protocol, kind, 2, text, &mut bytes)?;
        assert_eq!(&bytes[..n], expected);
        let print = Prints::new(protocol, &bytes[..n])
            .next()
            .ok_or(Error::Truncated)??;
        assert_eq!((print.kind, print.text), (kind, text));
    }
    let mut bytes = [0; 128];
    let n = outputs::print(
        Protocol::Quake2_34,
        PrintKind::Console,
        0,
        b"first\0ignored",
        &mut bytes,
    )?;
    let m = outputs::print(
        Protocol::Quake2_34,
        PrintKind::Console,
        1,
        b"second",
        &mut bytes[n..],
    )?;
    let mut prints = Prints::new(Protocol::Quake2_34, &bytes[..n + m]);
    assert_eq!(prints.next().ok_or(Error::Truncated)??.text, b"first");
    assert_eq!(prints.next().ok_or(Error::Truncated)??.level, Some(1));
    assert!(prints.next().is_none());
    assert_eq!(
        outputs::print(
            Protocol::QuakeWorld28,
            PrintKind::Layout,
            2,
            b"layout",
            &mut bytes
        ),
        Err(Error::Unsupported)
    );
    assert_eq!(
        outputs::print(
            Protocol::Quake3_68,
            PrintKind::Console,
            2,
            b"print",
            &mut bytes
        ),
        Err(Error::Unsupported)
    );
    assert_eq!(
        Prints::new(Protocol::Quake2_34, b"\x0funterminated").next(),
        Some(Err(Error::Truncated))
    );
    Ok(())
}
