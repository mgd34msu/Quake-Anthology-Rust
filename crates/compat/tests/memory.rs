use qa_compat::memory::ModuleMemory;

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
