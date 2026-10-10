use qa_network::{
    message::{Encoding, ErrorKind, Reader, Writer},
    states::{self, Q2_RERELEASE_ENTITY_WORDS},
};

type Words = [u32; Q2_RERELEASE_ENTITY_WORDS];

#[test]
fn wide_entity_fields_are_unsigned_and_truncated_messages_fail() -> Result<(), String> {
    let from = [0; Q2_RERELEASE_ENTITY_WORDS];
    let mut to: Words = [
        65535,
        256,
        32768,
        65534,
        65535,
        0x8000,
        0xffffffff,
        0x80000000,
        1.25_f32.to_bits(),
        (-2.5_f32).to_bits(),
        0.125_f32.to_bits(),
        (-32768_i32) as u32,
        32767,
        (-1_i32) as u32,
        (-4.0_f32).to_bits(),
        8.0_f32.to_bits(),
        16.0_f32.to_bits(),
        16383,
        255,
        0xffffffff,
        0x8000,
        255,
        254,
        253,
        252,
    ];
    let mut bytes = [0; 1400];
    let mut writer = Writer::new(&mut bytes, Encoding::Bytes);
    states::write_q2_repro_entity(&mut writer, 65535, &from, Some(&to), true)
        .map_err(|e| e.to_string())?;
    let size = writer.size();
    let mut reader = Reader::new(&bytes[..size], Encoding::Bytes);
    let decoded = states::read_q2_repro_entity(&mut reader, &from).map_err(|e| e.to_string())?;
    assert_eq!(decoded.number, 65535);
    assert_eq!(decoded.words, Some(to));
    assert_eq!(reader.byte_position(), size);
    for prefix in 0..size {
        let mut reader = Reader::new(&bytes[..prefix], Encoding::Bytes);
        assert_eq!(
            states::read_q2_repro_entity(&mut reader, &from).map_err(|e| e.kind),
            Err(ErrorKind::Truncated)
        );
        let mut writer = Writer::new(&mut bytes[..prefix], Encoding::Bytes);
        assert_eq!(
            states::write_q2_repro_entity(&mut writer, 65535, &from, Some(&to), true)
                .map_err(|e| e.kind),
            Err(ErrorKind::Capacity)
        );
    }
    to[18] = 0;
    let mut writer = Writer::new(&mut bytes, Encoding::Bytes);
    states::write_q2_repro_entity(&mut writer, 65535, &from, None, false)
        .map_err(|e| e.to_string())?;
    let mut reader = Reader::new(writer.bytes(), Encoding::Bytes);
    let decoded = states::read_q2_repro_entity(&mut reader, &to).map_err(|e| e.to_string())?;
    assert_eq!(decoded.number, 65535);
    assert_eq!(decoded.words, None);
    Ok(())
}

#[test]
fn origins_compare_in_eighth_units_and_old_origin_follows_native_policy() -> Result<(), String> {
    let mut from = [0; Q2_RERELEASE_ENTITY_WORDS];
    from[8] = 1.25_f32.to_bits();
    from[14] = (-4.0_f32).to_bits();
    from[18] = 7;
    for (target, beam, force) in [
        (1.26_f32, false, false),
        (1.375, false, false),
        (1.26, true, false),
        (1.26, true, true),
    ] {
        let mut to = from;
        to[8] = target.to_bits();
        to[7] = if beam { 128 } else { 0 };
        to[18] = 0;
        to[14] = 16.0_f32.to_bits();
        let mut bytes = [0; 1400];
        let mut writer = Writer::new(&mut bytes, Encoding::Bytes);
        states::write_q2_repro_entity(&mut writer, 1, &from, Some(&to), force)
            .map_err(|e| e.to_string())?;
        let mut reader = Reader::new(writer.bytes(), Encoding::Bytes);
        let out = states::read_q2_repro_entity(&mut reader, &from)
            .map_err(|e| e.to_string())?
            .words
            .ok_or("missing entity")?;
        assert_eq!(out[8], if target == 1.26 { from[8] } else { to[8] });
        assert_eq!(
            out[14],
            if force {
                to[14]
            } else if beam {
                from[14]
            } else {
                from[8]
            }
        );
        assert_eq!(out[18], 0);
    }
    Ok(())
}

#[test]
fn native_sound_metadata_is_inline_and_loop_only_changes_are_not_emitted() -> Result<(), String> {
    let mut from = [0; Q2_RERELEASE_ENTITY_WORDS];
    from[17] = 16383;
    from[23] = 255;
    from[24] = 255;
    for sound in [16383, 2] {
        let mut to = from;
        to[17] = sound;
        to[23] = 0;
        to[24] = 0;
        let mut bytes = [0; 1400];
        let mut writer = Writer::new(&mut bytes, Encoding::Bytes);
        states::write_q2_repro_entity(&mut writer, 1, &from, Some(&to), false)
            .map_err(|e| e.to_string())?;
        if sound == 16383 {
            assert_eq!(writer.bytes(), [0, 1]);
        }
        let mut reader = Reader::new(writer.bytes(), Encoding::Bytes);
        let out = states::read_q2_repro_entity(&mut reader, &from)
            .map_err(|e| e.to_string())?
            .words
            .ok_or("missing entity")?;
        assert_eq!(out[17], sound);
        assert_eq!(
            &out[23..25],
            if sound == 16383 {
                &from[23..25]
            } else {
                &to[23..25]
            }
        );
    }
    Ok(())
}

#[test]
fn legacy_angles_and_dual_frame_flags_use_native_parser_precedence() -> Result<(), String> {
    let from = [0; Q2_RERELEASE_ENTITY_WORDS];
    let mut bytes = [0; 1400];
    let mut writer = Writer::new(&mut bytes, Encoding::Bytes);
    states::write_q2_entity_prefix(&mut writer, 1, (1 << 10) | (1 << 4) | (1 << 17), true)
        .map_err(|e| e.to_string())?;
    writer.write_bits(255, 8).map_err(|e| e.to_string())?;
    writer.write_bits(128, 8).map_err(|e| e.to_string())?;
    let mut reader = Reader::new(writer.bytes(), Encoding::Bytes);
    let out = states::read_q2_repro_entity(&mut reader, &from)
        .map_err(|e| e.to_string())?
        .words
        .ok_or("missing entity")?;
    assert_eq!(out[4], 255);
    // q2proto_var_angles_get_short_comp multiplies a native char by 0x101.
    assert_eq!(out[11], 32640);
    assert_eq!(reader.byte_position(), writer.size());
    Ok(())
}

#[test]
fn kex_effects_replace_both_halves_and_keep_native_unsigned_width_flags() -> Result<(), String> {
    let mut from = [0; Q2_RERELEASE_ENTITY_WORDS];
    from[6] = 0x12345678;
    from[20] = 0x100;
    for (low, high) in [(0xffffffff, 0), (255, 256), (32768, 32768), (65535, 65535)] {
        let mut to = from;
        to[6] = low;
        to[20] = high;
        to[5] = 32768;
        to[7] = 65535;
        let mut bytes = [0; 1400];
        let mut writer = Writer::new(&mut bytes, Encoding::Bytes);
        states::write_q2_kex_entity(
            &mut writer,
            8191,
            &from,
            Some(&to),
            false,
            false,
            &mut states::Q2KexWire {
                nonzero_solid: (&from)[19] != 0,
                baseline_solid: false,
            },
        )
        .map_err(|e| e.to_string())?;
        let mut reader = Reader::new(writer.bytes(), Encoding::Bytes);
        let header = states::read_q2_entity_prefix(&mut reader, true).map_err(|e| e.to_string())?;
        assert_eq!(
            header.flags & ((1 << 16) | (1 << 25)),
            (1 << 16) | (1 << 25)
        );
        assert_eq!(
            header.flags & ((1 << 12) | (1 << 18)),
            (1 << 12) | (1 << 18)
        );
        assert_eq!(header.flags & (1 << 29) != 0, high != 0);
        let mut reader = Reader::new(writer.bytes(), Encoding::Bytes);
        let decoded = states::read_q2_kex_entity(
            &mut reader,
            &from,
            false,
            &mut states::Q2KexWire {
                nonzero_solid: (&from)[19] != 0,
                baseline_solid: false,
            },
        )
        .map_err(|e| e.to_string())?;
        assert_eq!(decoded.number, 8191);
        assert_eq!(decoded.words, Some(to));
        assert_eq!(reader.byte_position(), writer.size());
    }
    Ok(())
}

#[test]
fn kex_demo_coordinate_precision_follows_the_new_solid_value() -> Result<(), String> {
    for (old_solid, new_solid, demo) in [
        (0, 0, true),
        (0, 1, true),
        (1, 0, true),
        (1, 1, true),
        (0, 0, false),
    ] {
        let mut from = [0; Q2_RERELEASE_ENTITY_WORDS];
        from[8] = 1.25_f32.to_bits();
        from[19] = old_solid;
        let mut to = from;
        to[8] = 1.26_f32.to_bits();
        to[11] = (-0.0_f32).to_bits();
        to[14] = (-2.26_f32).to_bits();
        to[19] = new_solid;
        let mut bytes = [0; 1400];
        let mut writer = Writer::new(&mut bytes, Encoding::Bytes);
        states::write_q2_kex_entity(
            &mut writer,
            1,
            &from,
            Some(&to),
            true,
            demo,
            &mut states::Q2KexWire {
                nonzero_solid: (&from)[19] != 0,
                baseline_solid: false,
            },
        )
        .map_err(|e| e.to_string())?;
        let mut reader = Reader::new(writer.bytes(), Encoding::Bytes);
        let out = states::read_q2_kex_entity(
            &mut reader,
            &from,
            demo,
            &mut states::Q2KexWire {
                nonzero_solid: (&from)[19] != 0,
                baseline_solid: false,
            },
        )
        .map_err(|e| e.to_string())?
        .words
        .ok_or("missing entity")?;
        assert_eq!(
            out[8],
            if demo && new_solid == 0 {
                1.25_f32.to_bits()
            } else {
                to[8]
            }
        );
        assert_eq!(
            out[14],
            if demo && new_solid == 0 {
                (-2.25_f32).to_bits()
            } else {
                to[14]
            }
        );
        assert_eq!(out[11], (-0.0_f32).to_bits());
        assert_eq!(out[19], new_solid);
        assert_eq!(reader.byte_position(), writer.size());
    }
    Ok(())
}

#[test]
fn kex_high_flags_emit_native_padding_and_incoming_reserved_values_are_ignored()
-> Result<(), String> {
    let mut from = [0; Q2_RERELEASE_ENTITY_WORDS];
    from[0] = 999;
    let mut to = from;
    to[22] = 255;
    let mut bytes = [0; 1400];
    let mut writer = Writer::new(&mut bytes, Encoding::Bytes);
    states::write_q2_kex_entity(
        &mut writer,
        256,
        &from,
        Some(&to),
        false,
        false,
        &mut states::Q2KexWire {
            nonzero_solid: (&from)[19] != 0,
            baseline_solid: false,
        },
    )
    .map_err(|e| e.to_string())?;
    let size = writer.size();
    assert_eq!(&writer.bytes()[size - 6..], [255, 0, 0, 0, 0, 0]);
    let mut reader = Reader::new(writer.bytes(), Encoding::Bytes);
    let header = states::read_q2_entity_prefix(&mut reader, true).map_err(|e| e.to_string())?;
    assert_eq!(header.flags >> 32, 255);
    for prefix in 0..size {
        let mut reader = Reader::new(&bytes[..prefix], Encoding::Bytes);
        assert_eq!(
            states::read_q2_kex_entity(
                &mut reader,
                &from,
                false,
                &mut states::Q2KexWire {
                    nonzero_solid: (&from)[19] != 0,
                    baseline_solid: false
                }
            )
            .map_err(|e| e.kind),
            Err(ErrorKind::Truncated)
        );
        let mut writer = Writer::new(&mut bytes[..prefix], Encoding::Bytes);
        assert_eq!(
            states::write_q2_kex_entity(
                &mut writer,
                256,
                &from,
                Some(&to),
                false,
                false,
                &mut states::Q2KexWire {
                    nonzero_solid: (&from)[19] != 0,
                    baseline_solid: false
                }
            )
            .map_err(|e| e.kind),
            Err(ErrorKind::Capacity)
        );
    }
    bytes[size - 5..size].copy_from_slice(&[255, 0x34, 0x12, 0x78, 0x56]);
    let mut reader = Reader::new(&bytes[..size], Encoding::Bytes);
    let decoded = states::read_q2_kex_entity(
        &mut reader,
        &from,
        false,
        &mut states::Q2KexWire {
            nonzero_solid: (&from)[19] != 0,
            baseline_solid: false,
        },
    )
    .map_err(|e| e.to_string())?;
    assert_eq!(decoded.number, 256);
    assert_eq!(decoded.words, Some(to));
    assert_eq!(reader.byte_position(), size);
    Ok(())
}
