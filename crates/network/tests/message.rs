use qa_network::message::{Encoding, ErrorKind, Reader, Writer};

#[test]
fn byte_prefix_patch_is_bounded_and_never_edits_bitstream_symbols() {
    for encoding in [Encoding::Bytes, Encoding::Bits, Encoding::Q3] {
        let mut data = [0xa5; 8];
        let mut writer = Writer::new(&mut data, encoding);
        writer.write_bits(0x1234, 16).expect("body");
        let size = writer.size();
        assert_eq!(
            writer.patch_byte(size, 0xff).expect_err("unwritten").kind,
            ErrorKind::Width
        );
        if encoding == Encoding::Bytes {
            writer.patch_byte(0, 0xff).expect("prefix");
            assert_eq!(writer.bytes(), &[0xff, 0x12]);
        } else {
            let mut before = [0; 8];
            before[..size].copy_from_slice(writer.bytes());
            assert_eq!(
                writer.patch_byte(0, 0xff).expect_err("encoding").kind,
                ErrorKind::Width
            );
            assert_eq!(writer.bytes(), &before[..size]);
        }
        assert_eq!(writer.size(), size);
    }
}

#[test]
fn little_endian_native_scalars_and_ieee_payloads() {
    let mut data = [0; 32];
    let mut writer = Writer::new(&mut data, Encoding::Bytes);
    writer.write_bits(0x1234_5678, 32).expect("long");
    writer.write_bits((-321i32) as u32, 16).expect("short");
    writer.write_bits(0x80, 8).expect("char");
    writer
        .write_float(f32::from_bits(0x7fa1_2345))
        .expect("float");
    assert_eq!(
        writer.bytes(),
        &[
            0x78, 0x56, 0x34, 0x12, 0xbf, 0xfe, 0x80, 0x45, 0x23, 0xa1, 0x7f
        ]
    );
    let mut reader = Reader::new(writer.bytes(), Encoding::Bytes);
    assert_eq!(reader.read_bits(32), Ok(0x1234_5678));
    assert_eq!(reader.read_signed(16), Ok(-321));
    assert_eq!(reader.read_signed(8), Ok(-128));
    assert_eq!(reader.read_float().expect("float").to_bits(), 0x7fa1_2345);
    let error = reader.read_bits(8).expect_err("truncated");
    assert_eq!((error.byte, error.kind), (11, ErrorKind::Truncated));
    assert!(error.to_string().contains("at byte 11"));
}

#[test]
fn mixed_bit_widths_and_signed_native_widths() {
    for encoding in [Encoding::Bits, Encoding::Q3] {
        let mut data = [0; 8192];
        let mut writer = Writer::new(&mut data, encoding);
        for width in 1..=32 {
            writer.write_bits(0x8765_4321, width).expect("bits");
        }
        for width in [8, 16, 32] {
            writer.write_bits((-17i32) as u32, width).expect("signed");
        }
        let mut reader = Reader::new(writer.bytes(), encoding);
        for width in 1..=32 {
            assert_eq!(
                reader.read_bits(width).expect("bits"),
                (0x8765_4321u64 & ((1u64 << width) - 1)) as u32
            );
        }
        for width in [8, 16, 32] {
            assert_eq!(reader.read_signed(width), Ok(-17));
        }
        assert_eq!(reader.bit_position(), writer.bit_position());
    }
}

#[test]
fn reused_dirty_buffers_match_clean_streams_including_padding() {
    for encoding in [Encoding::Bytes, Encoding::Bits, Encoding::Q3] {
        let mut clean = [0; 512];
        let mut reused = [0xa5; 512];
        let mut a = Writer::new(&mut clean, encoding);
        let mut b = Writer::new(&mut reused, encoding);
        for i in 0..128u32 {
            let width = if encoding == Encoding::Bytes {
                [8, 16, 32][i as usize % 3]
            } else {
                (i % 32 + 1) as u8
            };
            let word = i.wrapping_mul(0x9876_5431);
            a.write_bits(word, width).expect("clean");
            b.write_bits(word, width).expect("reused");
            assert_eq!(b.bytes(), a.bytes());
            assert_eq!(b.bit_position(), a.bit_position());
        }
        let used = b.size();
        assert_eq!(&reused[used..], vec![0xa5; 512 - used]);
    }
}

#[test]
fn empty_bit_writer_leaves_load_sized_storage_untouched() {
    for encoding in [Encoding::Bits, Encoding::Q3] {
        let mut data = [0xa5; 32768];
        let writer = Writer::new(&mut data, encoding);
        assert!(writer.bytes().is_empty());
        assert_eq!(writer.bit_position(), 0);
        assert!(data.iter().all(|byte| *byte == 0xa5));
    }
}

#[test]
fn oob_signature_and_body_share_the_message_path() {
    let mut data = [0; 16];
    let mut writer = Writer::out_of_band(&mut data).expect("oob");
    writer.write_data(b"getinfo\0").expect("body");
    assert_eq!(writer.bytes(), b"\xff\xff\xff\xffgetinfo\0");
    let mut reader = Reader::out_of_band(writer.bytes()).expect("oob");
    let mut body = [0; 8];
    reader.read_data(&mut body).expect("body");
    assert_eq!(&body, b"getinfo\0");
    assert_eq!(
        Reader::out_of_band(b"wrong").err().expect("signature").kind,
        ErrorKind::OutOfBand
    );
}

#[test]
fn capacity_error_preserves_the_previous_scalar_and_untouched_memory() {
    for encoding in [Encoding::Bytes, Encoding::Bits, Encoding::Q3] {
        let mut guarded = [0xa5; 12];
        let mut writer = Writer::new(&mut guarded[4..6], encoding);
        writer.write_bits(3, 8).expect("first");
        let before = writer.bytes().to_vec();
        let bits = writer.bit_position();
        assert_eq!(
            writer.write_bits(u32::MAX, 32).expect_err("capacity").kind,
            ErrorKind::Capacity
        );
        assert_eq!(writer.bytes(), before);
        assert_eq!(writer.bit_position(), bits);
        assert_eq!(&guarded[..4], &[0xa5; 4]);
        assert_eq!(&guarded[6..], &[0xa5; 6]);
    }
}

#[test]
fn invalid_width_and_q3_reserved_symbol_are_errors() {
    let mut bytes = [0; 8];
    for encoding in [Encoding::Bytes, Encoding::Bits, Encoding::Q3] {
        let mut writer = Writer::new(&mut bytes, encoding);
        for width in [0, 33, 255] {
            assert_eq!(
                writer.write_bits(0, width).expect_err("width").kind,
                ErrorKind::Width
            );
            assert_eq!(
                Reader::new(&[], encoding)
                    .read_signed(width)
                    .expect_err("width")
                    .kind,
                ErrorKind::Width
            );
        }
    }
    // Original table's NYT code is reserved in fixed Q3 MSG streams.
    let mut reader = Reader::new(&[0, 1], Encoding::Q3);
    assert_eq!(
        reader.read_bits(8).expect_err("reserved NYT").kind,
        ErrorKind::Symbol
    );
    assert_eq!(reader.bit_position(), 11);
    let mut reader = Reader::new(&[0], Encoding::Q3);
    let error = reader.read_bits(8).expect_err("truncated code");
    assert_eq!((error.byte, error.kind), (1, ErrorKind::Truncated));
    assert_eq!(reader.bit_position(), 8);
}
