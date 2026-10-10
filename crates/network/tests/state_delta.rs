use qa_network::{
    message::{Encoding, Reader, Writer},
    states,
};

#[test]
fn rerelease_player_flags_preserve_wide_native_words() -> Result<(), String> {
    let from = states::Q2ReproPlayer::default();
    let mut to = from;
    to.words[3] = 1.25f32.to_bits();
    to.words[6] = (-2.5f32).to_bits();
    to.words[7] = 65535;
    to.words[8] = 65535;
    to.words[18] = (-32768i32) as u32;
    to.words[22] = 65535;
    to.words[23] = 65535;
    to.words[24] = (-32768i32) as u32;
    to.words[27] = 32767;
    to.words[40] = 255;
    to.words[41] = (-128i32) as u32;
    to.words[42] = (-32768i32) as u32;
    to.stats[63] = (-32768i32) as u32;
    let mut bytes = [0; 1400];
    let mut writer = Writer::new(&mut bytes, Encoding::Bytes);
    let extra =
        states::write_q2_repro_player(&mut writer, &from, &to).map_err(|e| e.to_string())?;
    assert_eq!(extra, 255);
    assert_eq!(&writer.bytes()[..2], &[0x18, 0xb0]);
    let mut reader = Reader::new(writer.bytes(), Encoding::Bytes);
    assert_eq!(
        states::read_q2_repro_player(&mut reader, &from, extra).map_err(|e| e.to_string())?,
        to
    );
    assert_eq!(reader.byte_position(), writer.size());
    for length in 0..writer.size() {
        assert!(
            states::read_q2_repro_player(
                &mut Reader::new(&writer.bytes()[..length], Encoding::Bytes),
                &from,
                extra
            )
            .is_err()
        );
    }
    Ok(())
}

#[test]
fn rerelease_player_fields_change_without_other_group_flags() -> Result<(), String> {
    let from = states::Q2ReproPlayer::default();
    let mut bytes = [0; 1400];
    for (word, value, expected) in [
        (23, 65535, &[0, 0x20, 255, 255][..]),
        (38, 255, &[0, 4, 64, 255][..]),
    ] {
        let mut to = from;
        to.words[word] = value;
        let mut writer = Writer::new(&mut bytes, Encoding::Bytes);
        let extra =
            states::write_q2_repro_player(&mut writer, &from, &to).map_err(|e| e.to_string())?;
        assert_eq!(extra, 0);
        assert_eq!(writer.bytes(), expected);
        assert_eq!(
            states::read_q2_repro_player(
                &mut Reader::new(writer.bytes(), Encoding::Bytes),
                &from,
                extra
            )
            .map_err(|e| e.to_string())?,
            to
        );
    }
    let mut to = from;
    to.words[1] = (-0.0f32).to_bits();
    let mut writer = Writer::new(&mut bytes, Encoding::Bytes);
    assert_eq!(
        states::write_q2_repro_player(&mut writer, &from, &to).map_err(|e| e.to_string())?,
        0
    );
    assert_eq!(writer.bytes(), &[0, 0]);
    assert_eq!(
        states::read_q2_repro_player(&mut Reader::new(writer.bytes(), Encoding::Bytes), &from, 0)
            .map_err(|e| e.to_string())?,
        from
    );
    Ok(())
}

#[test]
fn rerelease_stat_mask_keeps_high_bits_before_all_signed_values() -> Result<(), String> {
    let from = std::array::from_fn(|i| i as u32);
    let mut to = from;
    to[0] = (-32768i32) as u32;
    to[31] = 32767;
    to[32] = (-1i32) as u32;
    to[63] = (-12345i32) as u32;
    let mut bytes = [0; 136];
    let mut writer = Writer::new(&mut bytes, Encoding::Bytes);
    states::write_q2_rr_stats(&mut writer, &from, &to).map_err(|e| e.to_string())?;
    assert_eq!(
        writer.bytes(),
        &[
            1, 0, 0, 128, 1, 0, 0, 128, 0, 128, 255, 127, 255, 255, 199, 207
        ]
    );
    let mut reader = Reader::new(writer.bytes(), Encoding::Bytes);
    assert_eq!(
        states::read_q2_rr_stats(&mut reader, &from).map_err(|e| e.to_string())?,
        to
    );
    assert_eq!(reader.byte_position(), writer.size());
    for length in 0..writer.size() {
        assert!(
            states::read_q2_rr_stats(
                &mut Reader::new(&writer.bytes()[..length], Encoding::Bytes),
                &from
            )
            .is_err()
        );
    }
    let mut writer = Writer::new(&mut bytes, Encoding::Bytes);
    states::write_q2_rr_stats(&mut writer, &from, &from).map_err(|e| e.to_string())?;
    assert_eq!(writer.bytes(), &[0; 8]);
    assert_eq!(
        states::read_q2_rr_stats(&mut Reader::new(writer.bytes(), Encoding::Bytes), &from)
            .map_err(|e| e.to_string())?,
        from
    );
    let all = std::array::from_fn(|i| (i as i32 - 32768) as u32);
    let mut writer = Writer::new(&mut bytes, Encoding::Bytes);
    states::write_q2_rr_stats(&mut writer, &from, &all).map_err(|e| e.to_string())?;
    assert_eq!(writer.size(), 136);
    assert_eq!(&writer.bytes()[..8], &[255; 8]);
    assert_eq!(
        states::read_q2_rr_stats(&mut Reader::new(writer.bytes(), Encoding::Bytes), &from)
            .map_err(|e| e.to_string())?,
        all
    );
    Ok(())
}
