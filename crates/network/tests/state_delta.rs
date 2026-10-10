use qa_network::{
    message::{Encoding, Reader, Writer},
    states,
};

#[test]
fn kex_player_keeps_native_float_widths_and_extended_tail_order() -> Result<(), String> {
    let from = states::Q2KexPlayer::default();
    let mut to = from;
    for word in (1..=6).chain(10..=12).chain(16..=18).chain(24..=29) {
        to.words[word] = ((word as f32 - 12.0) * 1.25).to_bits();
    }
    for word in [7, 8, 22] {
        to.words[word] = 65535;
    }
    for word in [9, 13, 14, 15, 19, 20, 21] {
        to.words[word] = (-32768i32) as u32;
    }
    to.words[0] = 7;
    to.words[23] = 511;
    to.words[30..=39].fill(255);
    to.words[40] = 255;
    to.words[41] = (-128i32) as u32;
    to.stats[0] = 32767;
    to.stats[63] = (-32768i32) as u32;
    let mut bytes = [0; 1400];
    let mut writer = Writer::new(&mut bytes, Encoding::Bytes);
    states::write_q2_kex_player(&mut writer, &from, &to).map_err(|e| e.to_string())?;
    assert_eq!(&writer.bytes()[..5], &[17, 255, 255, 1, 0]);
    let mut reader = Reader::new(writer.bytes(), Encoding::Bytes);
    assert_eq!(
        states::read_q2_kex_player(&mut reader, &from).map_err(|e| e.to_string())?,
        to
    );
    assert_eq!(reader.byte_position(), writer.size());
    for length in 0..writer.size() {
        assert!(
            states::read_q2_kex_player(
                &mut Reader::new(&writer.bytes()[..length], Encoding::Bytes),
                &from,
            )
            .is_err()
        );
    }
    // The reference consumes team ID without exposing it to player storage.
    let team = [17, 0, 128, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 7];
    let mut reader = Reader::new(&team, Encoding::Bytes);
    assert_eq!(
        states::read_q2_kex_player(&mut reader, &from).map_err(|e| e.to_string())?,
        from
    );
    assert_eq!(reader.byte_position(), team.len());
    assert!(
        states::read_q2_kex_player(&mut Reader::new(&team[..13], Encoding::Bytes), &from).is_err()
    );
    assert!(
        states::read_q2_kex_player(&mut Reader::new(&[0; 11], Encoding::Bytes), &from).is_err()
    );
    Ok(())
}

#[test]
fn kex_gun_presence_shares_the_frame_word_without_changing_held_values() -> Result<(), String> {
    let mut from = states::Q2KexPlayer::default();
    from.words[23] = 511;
    let mut bytes = [0; 1400];
    for (bit, word) in (24..=29).chain([40]).enumerate() {
        let mut to = from;
        to.words[word] = if word == 40 { 255 } else { 1.25f32.to_bits() };
        let mut writer = Writer::new(&mut bytes, Encoding::Bytes);
        states::write_q2_kex_player(&mut writer, &from, &to).map_err(|e| e.to_string())?;
        assert_eq!(&writer.bytes()[..3], &[17, 0, 32]);
        assert_eq!(
            &writer.bytes()[3..5],
            &(511u16 | (1 << (9 + bit))).to_le_bytes()
        );
        let decoded =
            states::read_q2_kex_player(&mut Reader::new(writer.bytes(), Encoding::Bytes), &from)
                .map_err(|e| e.to_string())?;
        assert_eq!(decoded, to);
    }
    let mut to = from;
    to.words[1] = (-0.0f32).to_bits();
    let mut writer = Writer::new(&mut bytes, Encoding::Bytes);
    states::write_q2_kex_player(&mut writer, &from, &to).map_err(|e| e.to_string())?;
    assert_eq!(writer.bytes(), &[17, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    assert_eq!(
        states::read_q2_kex_player(&mut Reader::new(writer.bytes(), Encoding::Bytes), &from)
            .map_err(|e| e.to_string())?,
        from
    );
    Ok(())
}

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
    stat_records(
        states::write_q2_rr_stats,
        states::read_q2_rr_stats,
        &[
            1, 0, 0, 128, 1, 0, 0, 128, 0, 128, 255, 127, 255, 255, 199, 207,
        ],
        &[255; 8],
    )
}

#[test]
fn kex_stat_masks_interleave_their_own_signed_values() -> Result<(), String> {
    stat_records(
        states::write_q2_kex_stats,
        states::read_q2_kex_stats,
        &[
            1, 0, 0, 128, 0, 128, 255, 127, 1, 0, 0, 128, 255, 255, 199, 207,
        ],
        &[255; 4],
    )
}

type StatWriter =
    fn(&mut Writer<'_>, &[u32; 64], &[u32; 64]) -> Result<(), qa_network::message::Error>;
type StatReader = fn(&mut Reader<'_>, &[u32; 64]) -> Result<[u32; 64], qa_network::message::Error>;

fn stat_records(
    write: StatWriter,
    read: StatReader,
    expected: &[u8],
    mask_prefix: &[u8],
) -> Result<(), String> {
    let from = std::array::from_fn(|i| i as u32);
    let mut to = from;
    to[0] = (-32768i32) as u32;
    to[31] = 32767;
    to[32] = (-1i32) as u32;
    to[63] = (-12345i32) as u32;
    let mut bytes = [0; 136];
    let mut writer = Writer::new(&mut bytes, Encoding::Bytes);
    write(&mut writer, &from, &to).map_err(|e| e.to_string())?;
    assert_eq!(writer.bytes(), expected);
    let mut reader = Reader::new(writer.bytes(), Encoding::Bytes);
    assert_eq!(read(&mut reader, &from).map_err(|e| e.to_string())?, to);
    assert_eq!(reader.byte_position(), writer.size());
    for length in 0..writer.size() {
        assert!(
            read(
                &mut Reader::new(&writer.bytes()[..length], Encoding::Bytes),
                &from
            )
            .is_err()
        );
    }
    let mut writer = Writer::new(&mut bytes, Encoding::Bytes);
    write(&mut writer, &from, &from).map_err(|e| e.to_string())?;
    assert_eq!(writer.bytes(), &[0; 8]);
    assert_eq!(
        read(&mut Reader::new(writer.bytes(), Encoding::Bytes), &from).map_err(|e| e.to_string())?,
        from
    );
    let all = std::array::from_fn(|i| (i as i32 - 32768) as u32);
    let mut writer = Writer::new(&mut bytes, Encoding::Bytes);
    write(&mut writer, &from, &all).map_err(|e| e.to_string())?;
    assert_eq!(writer.size(), 136);
    assert_eq!(&writer.bytes()[..mask_prefix.len()], mask_prefix);
    assert_eq!(
        read(&mut Reader::new(writer.bytes(), Encoding::Bytes), &from).map_err(|e| e.to_string())?,
        all
    );
    Ok(())
}
