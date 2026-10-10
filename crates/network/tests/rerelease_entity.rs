use qa_network::{
    message::{Encoding, ErrorKind, Reader, Writer},
    states::{self, Q2_REPRO_ENTITY_WORDS},
};

type Words = [u32; Q2_REPRO_ENTITY_WORDS];

#[test]
fn wide_entity_fields_are_unsigned_and_truncated_messages_fail() -> Result<(), String> {
    let from = [0; Q2_REPRO_ENTITY_WORDS];
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
    let mut from = [0; Q2_REPRO_ENTITY_WORDS];
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
    let mut from = [0; Q2_REPRO_ENTITY_WORDS];
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
    let from = [0; Q2_REPRO_ENTITY_WORDS];
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
