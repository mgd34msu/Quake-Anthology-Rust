use qa_network::{
    message::{Encoding, Reader, Writer},
    states,
};

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
