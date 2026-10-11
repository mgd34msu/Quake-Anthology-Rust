use qa_compat::memory::{ModuleMemory, Width};

#[test]
fn integer_fields_keep_little_endian_widths_and_reject_partial_ranges() {
    let base = 0x1000;
    let mut memory = ModuleMemory::load(base, 9, &[0xa5; 9]).unwrap();
    for (width, expected) in [
        (Width::Byte, 0x98),
        (Width::Short, 0xba98),
        (Width::Word, 0xfedcba98),
    ] {
        memory.write(base, &[0xa5; 9]).unwrap();
        memory.write_field(base + 1, width, 0xfedcba98).unwrap();
        assert_eq!(memory.read_field(base + 1, width).unwrap(), expected);
        assert_eq!(memory.read(base, 1).unwrap(), &[0xa5]);
        assert_eq!(
            memory.read(base + 1 + width.len() as u64, 1).unwrap(),
            &[0xa5]
        );
        let invalid = base + 10 - width.len() as u64;
        let before = memory.read(base, 9).unwrap().to_vec();
        assert!(memory.write_field(invalid, width, 0).is_err());
        assert!(memory.read_field(invalid, width).is_err());
        assert_eq!(memory.read(base, 9).unwrap(), before);
        assert!(memory.read_field(base - 1, width).is_err());
    }
    memory.write_word(base + 1, i32::MIN).unwrap();
    assert_eq!(
        memory.read_field(base + 1, Width::Word).unwrap(),
        0x80000000
    );
    assert_eq!(memory.read_word(base + 1).unwrap(), i32::MIN);
}

#[test]
fn owned_and_borrowed_bytes_use_the_same_native_bounds_and_copy_order() {
    let base = 1u64 << 40;
    let mut bytes = [0u8; 64];
    let mut owned = ModuleMemory::load(base, bytes.len(), &bytes).unwrap();
    {
        let mut borrowed = ModuleMemory::borrow(base, &mut bytes).unwrap();
        for memory in [&mut owned, &mut borrowed] {
            memory.write(base, b"abcdef\0").unwrap();
            memory.copy(base + 1, base, 6).unwrap();
            assert_eq!(memory.cstring(base).unwrap(), b"aabcdef");
            memory.write_word(base + 16, i32::MIN).unwrap();
            assert_eq!(memory.read_word(base + 16).unwrap(), i32::MIN);
            memory.write_u64(base + 27, 0xfedc_ba98_7654_3210).unwrap();
            assert_eq!(memory.read_u64(base + 27).unwrap(), 0xfedc_ba98_7654_3210);
            assert_eq!(
                memory.read(base + 27, 8).unwrap(),
                &[0x10, 0x32, 0x54, 0x76, 0x98, 0xba, 0xdc, 0xfe]
            );
            assert!(memory.write_u64(base + 57, 0).is_err());
            assert!(memory.read_u64(base + 57).is_err());
            assert!(memory.read(base - 1, 1).is_err());
            assert!(memory.read(u64::MAX, 8).is_err());
            assert!(memory.read(base + 63, 2).is_err());
            assert!(memory.copy(base + 63, base, 2).is_err());
        }
        assert_eq!(
            owned.read(base, 64).unwrap(),
            borrowed.read(base, 64).unwrap()
        );
    }
    assert_eq!(&bytes[..8], b"aabcdef\0");
    assert!(ModuleMemory::borrow(u64::MAX, &mut bytes).is_err());
}
