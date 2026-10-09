use qa_formats::{
    FormatError,
    program::native::{Image, LoadRole},
};
fn put(file: &mut [u8], at: usize, value: u64, width: usize) {
    file[at..at + width].copy_from_slice(&value.to_le_bytes()[..width]);
}
fn rva(file: &mut [u8], at: usize, value: u64, width: usize) {
    put(file, 512 + at - 4096, value, width);
}
fn text(file: &mut [u8], at: usize, value: &[u8]) {
    let offset = 512 + at - 4096;
    file[offset..offset + value.len()].copy_from_slice(value);
}
fn fixture(bits: usize) -> Vec<u8> {
    let mut f = vec![0; 1024];
    f[..2].copy_from_slice(b"MZ");
    put(&mut f, 60, 128, 4);
    f[128..132].copy_from_slice(b"PE\0\0");
    let optional = if bits == 64 { 240 } else { 224 };
    put(&mut f, 132, if bits == 64 { 0x8664 } else { 0x14c }, 2);
    put(&mut f, 134, 1, 2);
    put(&mut f, 148, optional, 2);
    put(&mut f, 150, 0x2002, 2);
    put(&mut f, 152, if bits == 64 { 0x20b } else { 0x10b }, 2);
    put(&mut f, 168, 0x1080, 4);
    put(
        &mut f,
        152 + if bits == 64 { 24 } else { 28 },
        if bits == 64 { 0x180000000 } else { 0x10000000 },
        bits / 8,
    );
    put(&mut f, 184, 4096, 4);
    put(&mut f, 188, 512, 4);
    put(&mut f, 208, 8192, 4);
    put(&mut f, 212, 512, 4);
    put(&mut f, 152 + if bits == 64 { 108 } else { 92 }, 16, 4);
    let section = 152 + optional as usize;
    f[section..section + 5].copy_from_slice(b".text");
    put(&mut f, section + 8, 4096, 4);
    put(&mut f, section + 12, 4096, 4);
    put(&mut f, section + 16, 512, 4);
    put(&mut f, section + 20, 512, 4);
    put(&mut f, section + 36, 0xe0000020, 4);
    f
}
fn directory(f: &mut [u8], bits: usize, index: usize, at: u64, length: u64) {
    let d = 152 + if bits == 64 { 112 } else { 96 };
    put(f, d + index * 8, at, 4);
    put(f, d + index * 8 + 4, length, 4);
}
#[test]
fn native_exports_keep_aliases_ordinals_and_full_width_rebasing() {
    for bits in [32, 64] {
        let mut f = fixture(bits);
        let preferred = if bits == 64 { 0x180000000 } else { 0x10000000 };
        let base = preferred - 0x100000;
        directory(&mut f, bits, 0, 0x1100, 0x80);
        for (offset, value) in [
            (16, 7),
            (20, 1),
            (24, 2),
            (28, 0x1140),
            (32, 0x1150),
            (36, 0x1160),
        ] {
            rva(&mut f, 0x1100 + offset, value, 4);
        }
        rva(&mut f, 0x1140, 0x1080, 4);
        rva(&mut f, 0x1150, 0x1190, 4);
        rva(&mut f, 0x1154, 0x11a0, 4);
        text(&mut f, 0x1190, b"GetGameAPI\0");
        text(&mut f, 0x11a0, b"Alias\0");
        directory(&mut f, bits, 5, 0x11e0, 12);
        rva(&mut f, 0x11e0, 0x1000, 4);
        rva(&mut f, 0x11e4, 12, 4);
        rva(&mut f, 0x11e8, if bits == 64 { 0xa080 } else { 0x3080 }, 2);
        rva(&mut f, 0x1080, preferred + 0x1088, bits / 8);
        let image = Image::parse(&f, Some(base), LoadRole::Library).unwrap();
        assert_eq!(image.symbols.len(), 3);
        assert_eq!(image.symbol(b"GetGameAPI").unwrap().address, base + 0x1080);
        assert_eq!(image.symbol(b"Alias").unwrap().ordinal, Some(7));
        assert!(image.symbol(b"getgameapi").is_none());
        assert_eq!(
            &image.bytes[0x1080..0x1080 + bits / 8],
            &(base + 0x1088).to_le_bytes()[..bits / 8]
        );
        assert!(image.bytes[0x1200..].iter().all(|&b| b == 0));
        assert!(image.regions[1].execute);
    }
}
#[test]
fn fixed_images_and_bad_headers_or_mapped_gaps_are_refused() {
    let original = fixture(32);
    assert!(Image::parse(&original, None, LoadRole::Library).is_ok());
    assert_eq!(
        Image::parse(&original, Some(0x11000000), LoadRole::Library).err(),
        Some(FormatError::InvalidReference("PE fixed base", 5))
    );
    for length in [0, 1, 63, 128, 152, 376, 1023] {
        assert!(Image::parse(&original[..length], None, LoadRole::Library).is_err());
    }
    let mut f = original.clone();
    put(&mut f, 376 + 12, 0, 4);
    assert!(Image::parse(&f, None, LoadRole::Library).is_err());
    let mut f = original.clone();
    directory(&mut f, 32, 0, 0x800, 40);
    assert!(Image::parse(&f, None, LoadRole::Library).is_err());
    let mut f = original.clone();
    directory(&mut f, 32, 14, 0x1000, 40);
    assert_eq!(
        Image::parse(&f, None, LoadRole::Library).err(),
        Some(FormatError::InvalidReference("PE CLR", 14))
    );
    assert!(Image::parse(&original, Some(u64::MAX - 100), LoadRole::Library).is_err());
}
#[test]
fn imports_keep_names_hints_ordinals_and_require_valid_terminated_tables() {
    for bits in [32, 64] {
        let mut f = fixture(bits);
        let width = bits / 8;
        directory(&mut f, bits, 1, 0x1100, 40);
        for (offset, value) in [(0, 0x1140), (12, 0x1180), (16, 0x1160)] {
            rva(&mut f, 0x1100 + offset, value, 4);
        }
        rva(&mut f, 0x1140, 0x11a0, width);
        rva(&mut f, 0x1140 + width, (1u64 << (bits - 1)) | 17, width);
        text(&mut f, 0x1180, b"runtime.dll\0");
        rva(&mut f, 0x11a0, 9, 2);
        text(&mut f, 0x11a2, b"call\0");
        let image = Image::parse(&f, None, LoadRole::Library).unwrap();
        assert_eq!(image.imports.len(), 2);
        assert_eq!(
            image.names.get(image.imports[0].name.unwrap()),
            Some(b"call".as_slice())
        );
        assert_eq!(image.imports[0].hint, Some(9));
        assert_eq!(image.imports[1].ordinal, Some(17));
        assert_eq!(image.imports[1].slot, image.base + 0x1160 + width as u64);
        rva(
            &mut f,
            0x1140 + width,
            (1u64 << (bits - 1)) | 0x10011,
            width,
        );
        assert!(Image::parse(&f, None, LoadRole::Library).is_err());
        let mut f = fixture(bits);
        directory(&mut f, bits, 1, 0x1100, 20);
        rva(&mut f, 0x1100 + 12, 0x1180, 4);
        rva(&mut f, 0x1100 + 16, 0x1160, 4);
        text(&mut f, 0x1180, b"runtime.dll\0");
        assert!(Image::parse(&f, None, LoadRole::Library).is_err());
    }
}

#[test]
fn native_split_word_relocations_and_tls_callback_addresses_are_preserved() {
    let mut f = fixture(32);
    directory(&mut f, 32, 5, 0x11e0, 16);
    rva(&mut f, 0x11e0, 0x1000, 4);
    rva(&mut f, 0x11e4, 16, 4);
    for (offset, value) in [(0, 0x1080), (2, 0x2082), (4, 0x4084), (6, 0x8001)] {
        rva(&mut f, 0x11e8 + offset, value, 2);
    }
    rva(&mut f, 0x1080, 0x1234, 2);
    rva(&mut f, 0x1082, 0x5678, 2);
    rva(&mut f, 0x1084, 0x9abc, 2);
    let image = Image::parse(&f, Some(0x0ff00000), LoadRole::Library).unwrap();
    assert_eq!(
        &image.bytes[0x1080..0x1086],
        &[0x24, 0x12, 0x78, 0x56, 0xac, 0x9a]
    );
    assert_eq!(image.relocations[2].addend, Some(-32767));
    for bits in [32, 64] {
        let mut f = fixture(bits);
        let preferred = if bits == 64 { 0x180000000 } else { 0x10000000 };
        let width = bits / 8;
        directory(&mut f, bits, 9, 0x1100, (width * 4 + 8) as u64);
        for (index, offset) in [(0, 0x10a0), (1, 0x10a4), (2, 0x10b0), (3, 0x10c0)] {
            rva(&mut f, 0x1100 + index * width, preferred + offset, width);
        }
        rva(&mut f, 0x1100 + width * 4, 16, 4);
        rva(&mut f, 0x1100 + width * 4 + 4, 3 << 20, 4);
        rva(&mut f, 0x10c0, preferred + 0x1080, width);
        let image = Image::parse(&f, None, LoadRole::Library).unwrap();
        let tls = image.tls.unwrap();
        assert_eq!(tls.file_bytes, 4);
        assert_eq!(tls.zero_bytes, 16);
        assert_eq!(tls.alignment, 4);
        assert_eq!(tls.index, Some(preferred + 0x10b0));
        assert_eq!(&*image.initializers, &[preferred + 0x1080]);
        rva(&mut f, 0x10c0, preferred + 0x400, width);
        assert!(Image::parse(&f, None, LoadRole::Library).is_err());
    }
}

fn elf_fixture(bits: usize, writable: bool) -> Vec<u8> {
    let mut f = vec![0x77; 8192];
    f[..256].fill(0);
    f[..4].copy_from_slice(b"\x7fELF");
    f[4] = if bits == 64 { 2 } else { 1 };
    f[5] = 1;
    f[6] = 1;
    put(&mut f, 16, 3, 2);
    put(&mut f, 18, if bits == 64 { 62 } else { 3 }, 2);
    put(&mut f, 20, 1, 4);
    let header = if bits == 64 { 64 } else { 52 };
    let stride = if bits == 64 { 56 } else { 32 };
    put(
        &mut f,
        if bits == 64 { 32 } else { 28 },
        header as u64,
        bits / 8,
    );
    put(&mut f, if bits == 64 { 52 } else { 40 }, header as u64, 2);
    put(&mut f, if bits == 64 { 54 } else { 42 }, stride as u64, 2);
    put(&mut f, if bits == 64 { 56 } else { 44 }, 2, 2);
    for (i, offset, address, file_bytes, memory_bytes) in
        [(0, 0, 0, 256, 256), (1, 0x1180, 0x3180, 8, 16)]
    {
        let at = header + i * stride;
        put(&mut f, at, 1, 4);
        put(
            &mut f,
            at + if bits == 64 { 4 } else { 24 },
            if writable { 6 } else { 4 },
            4,
        );
        for (field, value) in [
            (if bits == 64 { 8 } else { 4 }, offset),
            (if bits == 64 { 16 } else { 8 }, address),
            (if bits == 64 { 32 } else { 16 }, file_bytes),
            (if bits == 64 { 40 } else { 20 }, memory_bytes),
            (if bits == 64 { 48 } else { 28 }, 4096),
        ] {
            put(&mut f, at + field, value, bits / 8);
        }
    }
    f
}
#[test]
fn elf_library_and_program_keep_their_distinct_native_file_page_tails() {
    for bits in [32, 64] {
        let f = elf_fixture(bits, false);
        let library = Image::parse(&f, Some(0x200000), LoadRole::Library).unwrap();
        let program = Image::parse(&f, Some(0x200000), LoadRole::Program).unwrap();
        assert_eq!(library.bytes.len(), 0x4000);
        assert_eq!(library.base, 0x200000);
        assert_eq!(library.regions.len(), 2);
        assert_eq!(&library.bytes[0x3180..0x3188], &[0x77; 8]);
        assert_eq!(&library.bytes[0x3188..0x3190], &[0; 8]);
        assert_eq!(library.bytes[0x3190], 0x77);
        assert_eq!(&program.bytes[0x3188..0x3190], &[0x77; 8]);
        assert_eq!(program.bytes[0x3190], 0x77);
        assert!(library.bytes[0x1000..0x3000].iter().all(|&b| b == 0));
        let f = elf_fixture(bits, true);
        let program = Image::parse(&f, None, LoadRole::Program).unwrap();
        assert!(program.bytes[0x3188..0x4000].iter().all(|&b| b == 0));
    }
}
#[test]
fn elf_checks_segment_congruence_address_domains_and_extended_counts() {
    for bits in [32, 64] {
        let original = elf_fixture(bits, false);
        let header = if bits == 64 { 64 } else { 52 };
        let stride = if bits == 64 { 56 } else { 32 };
        let mut f = original.clone();
        put(
            &mut f,
            header + stride + if bits == 64 { 16 } else { 8 },
            0x3181,
            bits / 8,
        );
        assert!(Image::parse(&f, None, LoadRole::Library).is_err());
        let mut f = original.clone();
        put(&mut f, if bits == 64 { 56 } else { 44 }, 65535, 2);
        assert!(Image::parse(&f, None, LoadRole::Library).is_err());
        let mut f = original.clone();
        put(&mut f, 16, 2, 2);
        assert!(Image::parse(&f, Some(0x200000), LoadRole::Library).is_err());
        assert!(Image::parse(&original, Some(1), LoadRole::Library).is_err());
        assert!(Image::parse(&original, Some(u64::MAX - 4095), LoadRole::Library).is_err());
    }
}
