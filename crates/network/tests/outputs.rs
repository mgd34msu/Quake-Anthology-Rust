use qa_core::primitives::PrintKind;
use qa_network::{
    commands::packet::Protocol,
    outputs::{self, Error, Prints},
};

#[test]
fn rerelease_center_levels_keep_svc_print_and_legacy_clients_get_centerprint() {
    for level in [4, 5] {
        let mut bytes = [0xcc; 64];
        let n = outputs::print(
            Protocol::Quake2Repro1038,
            PrintKind::Center,
            level,
            b"center",
            &mut bytes,
        )
        .unwrap();
        assert_eq!(
            &bytes[..n],
            &[10, level, b'c', b'e', b'n', b't', b'e', b'r', 0]
        );
        let record = Prints::new(Protocol::Quake2Repro1038, &bytes[..n])
            .next()
            .unwrap()
            .unwrap();
        assert_eq!(record.kind, PrintKind::Center);
        assert_eq!(record.level, Some(level));
        let n = outputs::print(
            Protocol::Quake2_34,
            PrintKind::Center,
            level,
            b"center",
            &mut bytes,
        )
        .unwrap();
        assert_eq!(&bytes[..n], b"\x0fcenter\0");
    }
    let record = Prints::new(Protocol::Quake2Repro1038, b"\x0a\x06tts\0")
        .next()
        .unwrap()
        .unwrap();
    assert_eq!((record.kind, record.level), (PrintKind::Console, Some(6)));
}

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
        Ok(14)
    );
    assert_eq!(
        Prints::new(Protocol::Quake2_34, b"\x0funterminated").next(),
        Some(Err(Error::Truncated))
    );
    Ok(())
}

#[test]
fn native_q2_streams_keep_their_abi_coordinate_width_and_reject_foreign_wire_forms() {
    use qa_core::primitives::RuleSetId;
    let short = [3, 0, 8, 0, 239, 255, 0, 128, 5];
    let float = [3, 0, 0, 0, 136, 63, 0, 0, 8, 192, 0, 0, 128, 69, 5];
    for (source, protocol, bytes) in [
        (RuleSetId::Quake2, Protocol::Quake2_34, short.as_slice()),
        (
            RuleSetId::Quake2Rerelease,
            Protocol::Quake2Repro1038,
            float.as_slice(),
        ),
    ] {
        let mut encoded = [0; 64];
        let result = outputs::native(protocol, source, bytes, &mut encoded, true, |cue, _| {
            Ok(cue)
        })
        .unwrap();
        assert_eq!(result.unsupported, 0);
        assert_eq!(&encoded[..result.length], bytes);
        for foreign in [
            Protocol::NetQuake15,
            Protocol::QuakeWorld28,
            Protocol::Quake3_68,
        ] {
            assert_eq!(
                outputs::native(foreign, source, bytes, &mut encoded, true, |cue, _| Ok(cue)),
                Err(Error::Unsupported)
            );
        }
    }
    assert_eq!(
        outputs::native(
            Protocol::Quake2_34,
            RuleSetId::Quake2Rerelease,
            &float,
            &mut [0; 64],
            true,
            |cue, _| Ok(cue)
        ),
        Err(Error::Unsupported)
    );
    assert_eq!(
        outputs::native(
            Protocol::Quake2Repro1038,
            RuleSetId::Quake2,
            &short,
            &mut [0; 64],
            true,
            |cue, _| Ok(cue)
        ),
        Err(Error::Unsupported)
    );
}

#[test]
fn q2_native_streams_transcode_known_services_in_order_without_changing_raw_text()
-> Result<(), Error> {
    use qa_core::primitives::RuleSetId;
    use qa_network::outputs::{Cue, NativeService};
    // Print, fully present sound fields, player muzzle flash, layout and nop.
    let source = [
        10, 2, b'r', 0x80, 0xff, 0, 9, 31, 49, 204, 128, 125, 41, 0, 247, 255, 162, 0, 1, 4, 1, 5,
        0, 128, 4, b'l', 0x80, 0, 6,
    ];
    let mut output = [0xcc; 128];
    let encoded = outputs::native(
        Protocol::Quake2Repro1038,
        RuleSetId::Quake2,
        &source,
        &mut output,
        true,
        |cue, _| Ok(cue),
    )?;
    assert_eq!(encoded.unsupported, 0);
    assert_eq!(output[encoded.length], 0xcc);
    let mut remaining = &output[..encoded.length];
    let (record, n) = outputs::read_native_service(Protocol::Quake2Repro1038, remaining)?;
    let NativeService::Print(print) = record else {
        panic!("print");
    };
    assert_eq!(
        (print.kind, print.text),
        (PrintKind::Console, &b"r\x80\xff"[..])
    );
    remaining = &remaining[n..];
    let (record, n) = outputs::read_native_service(Protocol::Quake2Repro1038, remaining)?;
    let NativeService::Cue(Cue::Sound(sound)) = record else {
        panic!("sound");
    };
    assert_eq!((sound.entity, sound.channel, sound.index), (5, 1, 49));
    assert_eq!(sound.position.0, [-1.125, 20.25, 128.125]);
    assert_eq!(
        (sound.volume, sound.attenuation, sound.time_offset),
        (204, 128, 125)
    );
    remaining = &remaining[n..];
    let (record, n) = outputs::read_native_service(Protocol::Quake2Repro1038, remaining)?;
    let NativeService::Cue(Cue::MuzzleFlash(muzzle)) = record else {
        panic!("muzzle");
    };
    assert_eq!((muzzle.entity, muzzle.flash, muzzle.silenced), (5, 0, true));
    remaining = &remaining[n..];
    let (record, n) = outputs::read_native_service(Protocol::Quake2Repro1038, remaining)?;
    let NativeService::Print(print) = record else {
        panic!("layout");
    };
    assert_eq!((print.kind, print.text), (PrintKind::Layout, &b"l\x80"[..]));
    remaining = &remaining[n..];
    assert!(matches!(
        outputs::read_native_service(Protocol::Quake2Repro1038, remaining)?.0,
        NativeService::Nop
    ));
    assert_eq!(remaining, &[6]);
    let mut roundtrip = [0xcc; 128];
    let result = outputs::native(
        Protocol::Quake2_34,
        RuleSetId::Quake2Rerelease,
        &output[..encoded.length],
        &mut roundtrip,
        true,
        |cue, _| Ok(cue),
    )?;
    assert_eq!(&roundtrip[..result.length], &source);
    assert_eq!(result.unsupported, 0);
    Ok(())
}

#[test]
fn unrepresentable_rerelease_services_drop_individually_and_later_records_continue()
-> Result<(), Error> {
    use qa_core::primitives::RuleSetId;
    let source = [
        9, 32, 1, 4, // sound ordinal 1025 cannot fit protocol 34.
        9, 8, 49, 0, 32, // native entity 1024 cannot fit classic Q2.
        32, 7, 0, 2, 3, // monster flash 770 cannot fit classic Q2.
        32, 7, 0, 17, 0, // a wide-encoded flash 17 can use the classic encoding.
        9, 32, 49, 0, // explicitly wide low ordinal becomes an eight-bit index.
        10, 5, b'c', 0x80, 0, 4, b'l', 0xff, 0,
    ];
    let mut output = [0xcc; 128];
    let result = outputs::native(
        Protocol::Quake2_34,
        RuleSetId::Quake2Rerelease,
        &source,
        &mut output,
        true,
        |cue, _| Ok(cue),
    )?;
    assert_eq!(result.unsupported, 3);
    assert_eq!(
        &output[..result.length],
        &[2, 7, 0, 17, 9, 0, 49, 15, b'c', 0x80, 0, 4, b'l', 0xff, 0]
    );
    assert_eq!(output[result.length], 0xcc);
    assert_eq!(
        outputs::native(
            Protocol::Quake2_34,
            RuleSetId::Quake2Rerelease,
            &[3, 0],
            &mut output,
            true,
            |cue, _| Ok(cue)
        ),
        Err(Error::Unsupported)
    );
    for cut in 1..4 {
        assert!(
            outputs::native(
                Protocol::Quake2_34,
                RuleSetId::Quake2Rerelease,
                &source[..cut],
                &mut output,
                true,
                |cue, _| Ok(cue)
            )
            .is_err()
        );
    }
    assert!(matches!(
        outputs::native(
            Protocol::Quake2_34,
            RuleSetId::Quake2Rerelease,
            &source,
            &mut [0; 1],
            true,
            |cue, _| Ok(cue)
        ),
        Err(Error::Message(_))
    ));
    Ok(())
}

#[test]
fn classic_print_priority_does_not_turn_into_a_rerelease_center_display() {
    for level in [4, 5] {
        let mut bytes = [0; 64];
        let result = outputs::native(
            Protocol::Quake2Repro1038,
            qa_core::primitives::RuleSetId::Quake2,
            &[10, level, b'p', 0],
            &mut bytes,
            true,
            |cue, _| Ok(cue),
        )
        .unwrap();
        let print = Prints::new(Protocol::Quake2Repro1038, &bytes[..result.length])
            .next()
            .unwrap()
            .unwrap();
        assert_eq!(print.kind, PrintKind::Console);
        assert_eq!(print.level, Some(2));
    }
}

#[test]
fn same_protocol_secondary_sound_uses_the_supplied_namespace_projection() {
    use outputs::{Cue, Sound};
    use qa_core::primitives::{RuleSetId, Vec3};
    for (protocol, rules, ordinal) in [
        (Protocol::Quake2_34, RuleSetId::Quake2, 255u32),
        (Protocol::Quake2Repro1038, RuleSetId::Quake2Rerelease, 2047),
    ] {
        let sound = Sound {
            flags: 12,
            index: 1,
            volume: 255,
            attenuation: 64,
            time_offset: 0,
            entity: 1,
            channel: 2,
            position: Vec3([1.0, 2.0, 3.0]),
        };
        let mut source = [0; 64];
        let n = outputs::write_cue(protocol, Cue::Sound(sound), &mut source).unwrap();
        let mut result = [0; 64];
        let mut visited = 0;
        let encoded = outputs::native(
            protocol,
            rules,
            &source[..n],
            &mut result,
            false,
            |cue, _| {
                let Cue::Sound(mut cue) = cue else {
                    panic!("unexpected cue")
                };
                visited += 1;
                cue = cue.with_index(ordinal)?;
                cue.entity = 47;
                Ok(Cue::Sound(cue))
            },
        )
        .unwrap();
        assert_eq!(visited, 1, "same-wire services must not bypass projection");
        let (Cue::Sound(projected), consumed) =
            outputs::read_cue(protocol, &result[..encoded.length]).unwrap()
        else {
            panic!("wrong service")
        };
        assert_eq!(consumed, encoded.length);
        assert_eq!(
            (projected.index, projected.entity(), projected.channel),
            (ordinal as u16, Some(47), 2)
        );
        assert_eq!(projected.position, sound.position);
        assert_eq!(encoded.unsupported, 0);
    }
}

#[test]
fn pending_resource_stops_submission_and_foreign_opaque_suffix_is_explicit() {
    use qa_core::primitives::{RuleSetId, Vec3};
    let protocol = Protocol::Quake2_34;
    let mut bytes = [0; 128];
    let prefix = outputs::print(protocol, PrintKind::Console, 2, b"prefix", &mut bytes).unwrap();
    let n = outputs::write_cue(
        protocol,
        outputs::Cue::Sound(outputs::Sound {
            flags: 4,
            index: 1,
            volume: 255,
            attenuation: 64,
            time_offset: 0,
            entity: 0,
            channel: 0,
            position: Vec3::default(),
        }),
        &mut bytes[prefix..],
    )
    .unwrap();
    let mut encoded = [0; 128];
    assert_eq!(
        outputs::native(
            protocol,
            RuleSetId::Quake2,
            &bytes[..prefix + n],
            &mut encoded,
            false,
            |_, _| Err(Error::Pending)
        ),
        Err(Error::Pending)
    );
    // Unknown payload lengths cannot be safely remapped. Only an original
    // primary namespace may keep its original same-protocol suffix.
    let source = b"\x0a\x02before\0\x03opaque";
    assert_eq!(
        outputs::native(
            protocol,
            RuleSetId::Quake2,
            source,
            &mut encoded,
            false,
            |cue, _| Ok(cue)
        ),
        Err(Error::Unsupported)
    );
    let result = outputs::native(
        protocol,
        RuleSetId::Quake2,
        source,
        &mut encoded,
        true,
        |cue, _| Ok(cue),
    )
    .unwrap();
    assert_eq!(&encoded[..result.length], source);
}
