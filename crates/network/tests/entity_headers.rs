use qa_network::{
    message::{Encoding, ErrorKind, Reader, Writer},
    states::{self, EntityHeader},
};

#[test]
fn prefixes_preserve_native_number_and_continuation_boundaries() -> Result<(), String> {
    for (number, flags, extended, expected) in [
        (0, 0, false, &[0, 0][..]),
        (255, 0, false, &[0, 255][..]),
        (256, 1 << 6, false, &[192, 1, 0, 1][..]),
        (65535, 1 << 32, true, &[128, 129, 128, 128, 1, 255, 255][..]),
        (
            65535,
            1 << 39,
            true,
            &[128, 129, 128, 128, 128, 255, 255][..],
        ),
    ] {
        let mut bytes = [0; 7];
        let mut writer = Writer::new(&mut bytes, Encoding::Bytes);
        let actual_flags = states::write_q2_entity_prefix(&mut writer, number, flags, extended)
            .map_err(|e| e.to_string())?;
        assert_eq!(writer.bytes(), expected);
        let mut reader = Reader::new(expected, Encoding::Bytes);
        assert_eq!(
            states::read_q2_entity_prefix(&mut reader, extended).map_err(|e| e.to_string())?,
            EntityHeader {
                number,
                flags: actual_flags
            }
        );
        assert_eq!(reader.byte_position(), expected.len());
        for prefix in 0..expected.len() {
            let mut reader = Reader::new(&expected[..prefix], Encoding::Bytes);
            assert_eq!(
                states::read_q2_entity_prefix(&mut reader, extended).map_err(|e| e.kind),
                Err(ErrorKind::Truncated)
            );
            let mut writer = Writer::new(&mut bytes[..prefix], Encoding::Bytes);
            assert_eq!(
                states::write_q2_entity_prefix(&mut writer, number, flags, extended)
                    .map_err(|e| e.kind),
                Err(ErrorKind::Capacity)
            );
        }
    }
    Ok(())
}

#[test]
fn legacy_reader_does_not_consume_a_fifth_flags_byte() -> Result<(), String> {
    let bytes = [128, 128, 128, 128, 9];
    let mut reader = Reader::new(&bytes, Encoding::Bytes);
    assert_eq!(
        states::read_q2_entity_prefix(&mut reader, false).map_err(|e| e.to_string())?,
        EntityHeader {
            number: 9,
            flags: 0x8080_8080
        }
    );
    assert_eq!(reader.byte_position(), bytes.len());
    let mut reader = Reader::new(&bytes, Encoding::Bytes);
    assert_eq!(
        states::read_q2_entity_prefix(&mut reader, true).map_err(|e| e.kind),
        Err(ErrorKind::Truncated)
    );
    for (extended, flags) in [(false, 1 << 32), (true, 1 << 40)] {
        let mut bytes = [0; 7];
        let mut writer = Writer::new(&mut bytes, Encoding::Bytes);
        assert_eq!(
            states::write_q2_entity_prefix(&mut writer, 1, flags, extended).map_err(|e| e.kind),
            Err(ErrorKind::Width)
        );
        assert_eq!(writer.size(), 0);
    }
    Ok(())
}
