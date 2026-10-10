use qa_network::{
    commands::packet::Error,
    message::{Encoding, Reader, Writer},
    snapshots::Q2Header,
};

#[test]
fn full_numbers_and_packed_offsets_preserve_native_metadata() -> Result<(), Error> {
    for sequence in [
        0,
        1,
        30,
        0x07ff_ffff,
        0x0800_0000,
        i32::MAX as u32,
        u32::MAX,
    ] {
        for distance in [0, 1, 30, 31] {
            let delta = if distance == 31 {
                -1
            } else {
                sequence.wrapping_sub(distance) as i32
            };
            for packed in [false, true] {
                let header = Q2Header {
                    sequence,
                    delta,
                    flags: 0xa5,
                    player_flags: 0x93,
                };
                let mut bytes = [0; 300];
                let mut writer = Writer::new(&mut bytes, Encoding::Bytes);
                let areas = [0xaa; 255];
                if packed {
                    header.write::<true>(&mut writer, &areas)?;
                } else {
                    header.write::<false>(&mut writer, &areas)?;
                }
                let mut reader = Reader::new(writer.bytes(), Encoding::Bytes);
                assert_eq!(reader.read_bits(8)?, 20);
                let mut decoded_areas = [0; 255];
                let (decoded, count) = if packed {
                    Q2Header::read::<true>(&mut reader, &mut decoded_areas, 255)?
                } else {
                    Q2Header::read::<false>(&mut reader, &mut decoded_areas, 255)?
                };
                assert_eq!(count, 255);
                assert_eq!(decoded_areas, areas);
                let native_sequence = if packed {
                    sequence & 0x07ff_ffff
                } else {
                    sequence
                };
                assert_eq!(decoded.sequence, native_sequence);
                assert_eq!(
                    decoded.delta,
                    if packed && delta != -1 {
                        native_sequence.wrapping_sub(distance) as i32
                    } else {
                        delta
                    }
                );
                assert_eq!(decoded.flags, if packed { 5 } else { 0xa5 });
                assert_eq!(decoded.player_flags, if packed { 0x93 } else { 0 });
                assert_eq!(reader.byte_position(), writer.size());
            }
        }
    }
    Ok(())
}

#[test]
fn truncated_frames_and_native_area_limits_are_bounded() -> Result<(), Error> {
    for packed in [false, true] {
        let header = Q2Header {
            sequence: 123,
            delta: 99,
            flags: 9,
            player_flags: 0xc5,
        };
        let mut bytes = [0; 64];
        let mut writer = Writer::new(&mut bytes, Encoding::Bytes);
        if packed {
            header.write::<true>(&mut writer, &[0xff; 32])?;
        } else {
            header.write::<false>(&mut writer, &[0xff; 32])?;
        }
        let length = writer.size();
        for capacity in 0..length {
            let mut short = [0; 64];
            let mut writer = Writer::new(&mut short[..capacity], Encoding::Bytes);
            assert!(
                if packed {
                    header.write::<true>(&mut writer, &[0xff; 32])
                } else {
                    header.write::<false>(&mut writer, &[0xff; 32])
                }
                .is_err()
            );
        }
        for length in 1..length {
            let mut reader = Reader::new(&bytes[1..length], Encoding::Bytes);
            let mut areas = [0; 32];
            assert!(
                if packed {
                    Q2Header::read::<true>(&mut reader, &mut areas, 32)
                } else {
                    Q2Header::read::<false>(&mut reader, &mut areas, 32)
                }
                .is_err()
            );
        }
        for (capacity, limit) in [(31, 32), (32, 31)] {
            let mut reader = Reader::new(&bytes[1..length], Encoding::Bytes);
            let mut areas = [0; 32];
            assert!(
                if packed {
                    Q2Header::read::<true>(&mut reader, &mut areas[..capacity], limit)
                } else {
                    Q2Header::read::<false>(&mut reader, &mut areas[..capacity], limit)
                }
                .is_err()
            );
        }
    }
    let mut bytes = [0; 300];
    let mut writer = Writer::new(&mut bytes, Encoding::Bytes);
    let header = Q2Header {
        sequence: 1,
        delta: -1,
        flags: 0,
        player_flags: 0,
    };
    assert_eq!(
        header.write::<false>(&mut writer, &[0; 256]),
        Err(Error::Count)
    );
    assert_eq!(writer.size(), 0);
    Ok(())
}
