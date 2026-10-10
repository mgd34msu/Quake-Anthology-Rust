#[path = "support/native_image.rs"]
mod native_image;
use native_image::*;

use qa_formats::{
    FormatError,
    program::native::{Image, LoadRole},
};
#[test]
fn native_exports_keep_aliases_ordinals_and_full_width_rebasing() {
    for bits in [32, 64] {
        let mut f = pe_export_fixture(bits);
        let preferred = if bits == 64 { 0x180000000 } else { 0x10000000 };
        let base = preferred - 0x100000;
        pe_directory(&mut f, bits, 5, 0x11e0, 12);
        pe_rva(&mut f, 0x11e0, 0x1000, 4);
        pe_rva(&mut f, 0x11e4, 12, 4);
        pe_rva(&mut f, 0x11e8, if bits == 64 { 0xa080 } else { 0x3080 }, 2);
        pe_rva(&mut f, 0x1080, preferred + 0x1088, bits / 8);
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
    let original = pe_fixture(32);
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
    pe_directory(&mut f, 32, 0, 0x800, 40);
    assert!(Image::parse(&f, None, LoadRole::Library).is_err());
    let mut f = original.clone();
    pe_directory(&mut f, 32, 14, 0x1000, 40);
    assert_eq!(
        Image::parse(&f, None, LoadRole::Library).err(),
        Some(FormatError::InvalidReference("PE CLR", 14))
    );
    assert!(Image::parse(&original, Some(u64::MAX - 100), LoadRole::Library).is_err());
}
#[test]
fn imports_keep_names_hints_ordinals_and_require_valid_terminated_tables() {
    for bits in [32, 64] {
        let mut f = pe_fixture(bits);
        let width = bits / 8;
        pe_directory(&mut f, bits, 1, 0x1100, 40);
        for (offset, value) in [(0, 0x1140), (12, 0x1180), (16, 0x1160)] {
            pe_rva(&mut f, 0x1100 + offset, value, 4);
        }
        pe_rva(&mut f, 0x1140, 0x11a0, width);
        pe_rva(&mut f, 0x1140 + width, (1u64 << (bits - 1)) | 17, width);
        pe_text(&mut f, 0x1180, b"runtime.dll\0");
        pe_rva(&mut f, 0x11a0, 9, 2);
        pe_text(&mut f, 0x11a2, b"call\0");
        let image = Image::parse(&f, None, LoadRole::Library).unwrap();
        assert_eq!(image.imports.len(), 2);
        assert_eq!(
            image.names.get(image.imports[0].name.unwrap()),
            Some(b"call".as_slice())
        );
        assert_eq!(image.imports[0].hint, Some(9));
        assert_eq!(image.imports[1].ordinal, Some(17));
        assert_eq!(image.imports[1].slot, image.base + 0x1160 + width as u64);
        pe_rva(
            &mut f,
            0x1140 + width,
            (1u64 << (bits - 1)) | 0x10011,
            width,
        );
        assert!(Image::parse(&f, None, LoadRole::Library).is_err());
        let mut f = pe_fixture(bits);
        pe_directory(&mut f, bits, 1, 0x1100, 20);
        pe_rva(&mut f, 0x1100 + 12, 0x1180, 4);
        pe_rva(&mut f, 0x1100 + 16, 0x1160, 4);
        pe_text(&mut f, 0x1180, b"runtime.dll\0");
        assert!(Image::parse(&f, None, LoadRole::Library).is_err());
    }
}

#[test]
fn native_split_word_relocations_and_tls_callback_addresses_are_preserved() {
    let mut f = pe_fixture(32);
    pe_directory(&mut f, 32, 5, 0x11e0, 16);
    pe_rva(&mut f, 0x11e0, 0x1000, 4);
    pe_rva(&mut f, 0x11e4, 16, 4);
    for (offset, value) in [(0, 0x1080), (2, 0x2082), (4, 0x4084), (6, 0x8001)] {
        pe_rva(&mut f, 0x11e8 + offset, value, 2);
    }
    pe_rva(&mut f, 0x1080, 0x1234, 2);
    pe_rva(&mut f, 0x1082, 0x5678, 2);
    pe_rva(&mut f, 0x1084, 0x9abc, 2);
    let image = Image::parse(&f, Some(0x0ff00000), LoadRole::Library).unwrap();
    assert_eq!(
        &image.bytes[0x1080..0x1086],
        &[0x24, 0x12, 0x78, 0x56, 0xac, 0x9a]
    );
    assert_eq!(image.relocations[2].addend, Some(-32767));
    for bits in [32, 64] {
        let mut f = pe_fixture(bits);
        let preferred = if bits == 64 { 0x180000000 } else { 0x10000000 };
        let width = bits / 8;
        pe_directory(&mut f, bits, 9, 0x1100, (width * 4 + 8) as u64);
        for (index, offset) in [(0, 0x10a0), (1, 0x10a4), (2, 0x10b0), (3, 0x10c0)] {
            pe_rva(&mut f, 0x1100 + index * width, preferred + offset, width);
        }
        pe_rva(&mut f, 0x1100 + width * 4, 16, 4);
        pe_rva(&mut f, 0x1100 + width * 4 + 4, 3 << 20, 4);
        pe_rva(&mut f, 0x10c0, preferred + 0x1080, width);
        let image = Image::parse(&f, None, LoadRole::Library).unwrap();
        let tls = image.tls.unwrap();
        assert_eq!(tls.file_bytes, 4);
        assert_eq!(tls.zero_bytes, 16);
        assert_eq!(tls.alignment, 4);
        assert_eq!(tls.index, Some(preferred + 0x10b0));
        assert_eq!(&*image.initializers, &[preferred + 0x1080]);
        pe_rva(&mut f, 0x10c0, preferred + 0x400, width);
        assert!(Image::parse(&f, None, LoadRole::Library).is_err());
    }
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

#[test]
fn elf_dynamic_names_tls_and_relro_share_the_loaded_image() {
    for bits in [32, 64] {
        let file = elf_dynamic_fixture(bits);
        let image = Image::parse(&file, Some(0x200000), LoadRole::Library).unwrap();
        assert_eq!(
            image.names.get(image.needed[0]),
            Some(b"libnative.so".as_slice())
        );
        assert_eq!(&*image.dynamic, &[(1, 1), (5, 0x3280), (10, 14)]);
        assert_eq!(&*image.relro, &[(0x203180, 0x100)]);
        let tls = image.tls.unwrap();
        assert_eq!(tls.address, 0x2032c0);
        assert_eq!(tls.file_bytes, 4);
        assert_eq!(tls.zero_bytes, 12);
        assert_eq!(&image.bytes[0x32c0..0x32c4], &[1, 2, 3, 4]);
    }
}
#[test]
fn elf_dynamic_metadata_cannot_read_unmapped_or_disagreeing_bytes() {
    for bits in [32, 64] {
        let original = elf_dynamic_fixture(bits);
        let header = if bits == 64 { 64 } else { 52 };
        let stride = if bits == 64 { 56 } else { 32 };
        for (field, value) in [
            (if bits == 64 { 16 } else { 8 }, 0x2180),
            (if bits == 64 { 8 } else { 4 }, 0x1280),
            (if bits == 64 { 32 } else { 16 }, 1),
            (if bits == 64 { 40 } else { 20 }, 0),
        ] {
            let mut file = original.clone();
            put(&mut file, header + stride * 2 + field, value, bits / 8);
            assert!(Image::parse(&file, None, LoadRole::Library).is_err());
        }
        let mut file = original.clone();
        put(&mut file, 0x1180 + 6 * (bits / 8), 5, bits / 8);
        assert!(Image::parse(&file, None, LoadRole::Library).is_err());
        let mut file = original.clone();
        put(&mut file, 0x1180 + (bits / 8), 14, bits / 8);
        assert!(Image::parse(&file, None, LoadRole::Library).is_err());
        let mut file = original.clone();
        file[0x1280..0x128e].fill(b'a');
        assert!(Image::parse(&file, None, LoadRole::Library).is_err());
    }
}
#[test]
fn elf_extended_section_counts_and_entry_permissions_are_checked() {
    for bits in [32, 64] {
        let mut file = elf_fixture(bits, false);
        let wide = bits == 64;
        let stride = if wide { 64 } else { 40 };
        file[0x1400..0x1400 + stride].fill(0);
        put(&mut file, if wide { 40 } else { 32 }, 0x1400, bits / 8);
        put(&mut file, if wide { 58 } else { 46 }, stride as u64, 2);
        put(&mut file, if wide { 56 } else { 44 }, 65535, 2);
        put(&mut file, 0x1400 + if wide { 32 } else { 20 }, 1, bits / 8);
        put(&mut file, 0x1400 + if wide { 44 } else { 28 }, 2, 4);
        assert!(Image::parse(&file, None, LoadRole::Library).is_ok());
        put(&mut file, 24, 128, bits / 8);
        assert!(Image::parse(&file, None, LoadRole::Library).is_err());
        put(&mut file, if wide { 64 + 4 } else { 52 + 24 }, 5, 4);
        let image = Image::parse(&file, Some(0x200000), LoadRole::Library).unwrap();
        assert_eq!(image.entry, 0x200080);
        put(
            &mut file,
            0x1400 + if wide { 44 } else { 28 },
            u32::MAX as u64,
            4,
        );
        assert!(Image::parse(&file, None, LoadRole::Library).is_err());
    }
}
#[test]
fn elf_truncated_headers_and_overflowing_loads_are_errors() {
    for bits in [32, 64] {
        let original = elf_fixture(bits, true);
        for end in 0..256 {
            assert!(Image::parse(&original[..end], None, LoadRole::Library).is_err());
        }
        let mut file = original.clone();
        let header = if bits == 64 { 64 } else { 52 };
        let stride = if bits == 64 { 56 } else { 32 };
        put(
            &mut file,
            header + stride + if bits == 64 { 40 } else { 20 },
            if bits == 64 {
                u64::MAX
            } else {
                u32::MAX as u64
            },
            bits / 8,
        );
        assert!(Image::parse(&file, None, LoadRole::Library).is_err());
    }
}

#[test]
fn elf_symbol_ordinals_native_attributes_and_tls_offsets_are_preserved() {
    for bits in [32, 64] {
        let (file, _) = elf_symbol_fixture(bits);
        let image = Image::parse(&file, Some(0x200000), LoadRole::Library).unwrap();
        assert_eq!(image.symbols.len(), 6);
        let export = image.symbol(b"vmMain").unwrap();
        assert_eq!(export.address, 0x2033c0);
        assert_eq!(export.bytes, 48);
        assert_eq!(export.kind, 2);
        assert!(image.symbol(b"VMMAIN").is_none());
        assert!(image.symbol(b"helper").is_none());
        assert!(image.symbol(b"external").is_none());
        assert!(!image.symbols[3].defined);
        assert!(image.symbols[3].weak);
        assert_eq!(image.symbol(b"COUNT").unwrap().address, 3);
        assert!(image.symbols[4].absolute);
        assert_eq!(image.symbol(b"tls").unwrap().address, 4);
        assert_eq!(image.symbols[5].kind, 6);
    }
}
#[test]
fn elf_stripped_symbols_use_checked_sysv_or_gnu_bucket_bounds() {
    for bits in [32, 64] {
        for gnu in [false, true] {
            let (mut file, mut tags) = elf_symbol_fixture(bits);
            tags.retain(|&(t, _)| t != 39);
            tags.push((if gnu { 0x6ffffef5 } else { 4 }, 0x3600));
            file[0x1600..0x1700].fill(0);
            if gnu {
                for (offset, value) in [(0, 1), (4, 1), (8, 1), (12, 5)] {
                    put(&mut file, 0x1600 + offset, value, 4);
                }
                let buckets = 0x1600 + 16 + bits / 8;
                put(&mut file, buckets, 1, 4);
                for i in 0..5 {
                    put(
                        &mut file,
                        buckets + 4 + i * 4,
                        if i == 4 { 1 } else { 2 },
                        4,
                    );
                }
            } else {
                put(&mut file, 0x1600, 1, 4);
                put(&mut file, 0x1604, 6, 4);
                put(&mut file, 0x1608, 1, 4);
            }
            elf_dynamic(&mut file, bits, &tags);
            let image = Image::parse(&file, None, LoadRole::Library).unwrap();
            assert_eq!(image.symbols.len(), 6);
            if gnu {
                put(&mut file, 0x1604, 2, 4);
            } else {
                put(&mut file, 0x1608, 6, 4);
            }
            assert!(Image::parse(&file, None, LoadRole::Library).is_err());
        }
    }
}
#[test]
fn elf_version_definitions_requirements_and_hidden_aliases_remain_distinct() {
    for bits in [32, 64] {
        let (mut file, mut tags) = elf_symbol_fixture(bits);
        tags.extend([
            (0x6ffffff0, 0x3800),
            (0x6ffffffc, 0x3840),
            (0x6ffffffd, 1),
            (0x6ffffffe, 0x3880),
            (0x6fffffff, 1),
        ]);
        file[0x1800..0x18c0].fill(0);
        for (i, value) in [1, 0x8002, 1, 3, 1, 1].into_iter().enumerate() {
            put(&mut file, 0x1800 + i * 2, value, 2);
        }
        for (offset, value, width) in [
            (0, 1, 2),
            (4, 2, 2),
            (6, 1, 2),
            (12, 20, 4),
            (20, elf_name(b"VER_1"), 4),
        ] {
            put(&mut file, 0x1840 + offset, value, width);
        }
        for (offset, value, width) in [
            (0, 1, 2),
            (2, 1, 2),
            (4, elf_name(b"libc.so.6"), 4),
            (8, 16, 4),
            (20, 2, 2),
            (22, 3, 2),
            (24, elf_name(b"VER_1"), 4),
        ] {
            put(&mut file, 0x1880 + offset, value, width);
        }
        elf_dynamic(&mut file, bits, &tags);
        let image = Image::parse(&file, None, LoadRole::Library).unwrap();
        assert!(image.symbol(b"vmMain").is_none());
        let s = image.symbol_version(b"vmMain", Some(b"VER_1")).unwrap();
        assert_eq!(s.address, 0x33c0);
        assert!(s.hidden_version);
        assert!(s.version.unwrap().library.is_none());
        let imported = image.symbols[3].version.unwrap();
        assert_eq!(
            image.names.get(imported.library.unwrap()),
            Some(b"libc.so.6".as_slice())
        );
        assert!(imported.weak);
        assert!(image.symbol_version(b"vmMain", Some(b"MISSING")).is_none());
        put(&mut file, 0x1802, 4, 2);
        assert!(Image::parse(&file, None, LoadRole::Library).is_err());
        put(&mut file, 0x1802, 2, 2);
        put(&mut file, 0x1896, 2, 2);
        assert!(Image::parse(&file, None, LoadRole::Library).is_err());
        tags.retain(|&(tag, _)| !matches!(tag, 0x6ffffffe | 0x6fffffff));
        elf_dynamic(&mut file, bits, &tags);
        put(&mut file, 0x1844, 1, 2);
        for i in 0..6 {
            put(&mut file, 0x1800 + i * 2, 1, 2);
        }
        let image = Image::parse(&file, None, LoadRole::Library).unwrap();
        assert!(image.symbol(b"vmMain").unwrap().version.is_none());
    }
}
#[test]
fn elf_static_symbols_and_extended_section_indices_use_one_symbol_table() {
    for bits in [32, 64] {
        let (mut file, _) = elf_symbol_fixture(bits);
        put(&mut file, if bits == 64 { 56 } else { 44 }, 2, 2);
        let stride = if bits == 64 { 64 } else { 40 };
        let symbol_stride = if bits == 64 { 24 } else { 16 };
        let mut section = |i: usize, kind, offset, length, link, entry| {
            let at = 0x2000 + i * stride;
            file[at..at + stride].fill(0);
            for (field, value, width) in [
                (4, kind, 4),
                (if bits == 64 { 24 } else { 16 }, offset, bits / 8),
                (if bits == 64 { 32 } else { 20 }, length, bits / 8),
                (if bits == 64 { 40 } else { 24 }, link, 4),
                (if bits == 64 { 56 } else { 36 }, entry, bits / 8),
            ] {
                put(&mut file, at + field, value, width);
            }
        };
        section(0, 0, 0, 0, 0, 0);
        section(1, 3, 0x1280, ELF_SYMBOL_NAMES.len() as u64, 0, 0);
        section(
            2,
            2,
            0x1400,
            (symbol_stride * 6) as u64,
            1,
            symbol_stride as u64,
        );
        section(3, 18, 0x18c0, 24, 2, 4);
        put(
            &mut file,
            if bits == 64 { 40 } else { 32 },
            0x2000,
            bits / 8,
        );
        put(
            &mut file,
            if bits == 64 { 58 } else { 46 },
            stride as u64,
            2,
        );
        put(&mut file, if bits == 64 { 60 } else { 48 }, 4, 2);
        file[0x18c0..0x18d8].fill(0);
        put(&mut file, 0x18c4, 1, 4);
        put(
            &mut file,
            0x1400 + symbol_stride + if bits == 64 { 6 } else { 14 },
            0xffff,
            2,
        );
        let image = Image::parse(&file, None, LoadRole::Library).unwrap();
        assert_eq!(image.symbols.len(), 6);
        assert_eq!(image.symbols[1].section, 1);
        assert_eq!(image.symbol(b"vmMain").unwrap().address, 0x33c0);
        put(
            &mut file,
            0x2000 + stride * 3 + if bits == 64 { 32 } else { 20 },
            20,
            bits / 8,
        );
        assert!(Image::parse(&file, None, LoadRole::Library).is_err());
    }
}
#[test]
fn elf_symbol_width_name_and_version_chains_fail_at_admission() {
    for bits in [32, 64] {
        let (original, tags) = elf_symbol_fixture(bits);
        for (tag, value) in [(11, 1), (39, 1), (10, 1), (6, 0x2000)] {
            let mut file = original.clone();
            let mut bad = tags.clone();
            *bad.iter_mut().find(|(t, _)| *t == tag).unwrap() = (tag, value);
            elf_dynamic(&mut file, bits, &bad);
            assert!(Image::parse(&file, None, LoadRole::Library).is_err());
        }
        let mut file = original.clone();
        let mut bad = tags.clone();
        bad.extend([(0x6ffffffc, 0x3840), (0x6ffffffd, u64::MAX)]);
        elf_dynamic(&mut file, bits, &bad);
        assert!(Image::parse(&file, None, LoadRole::Library).is_err());
    }
}

#[test]
fn elf_rel_rela_and_plt_keep_order_width_and_signed_addends_without_patching() {
    for bits in [32, 64] {
        let (mut file, mut tags) = elf_symbol_fixture(bits);
        let width = (bits / 8) as u64;
        tags.extend([
            (17, 0x3a00),
            (18, width * 4),
            (19, width * 2),
            (7, 0x3a80),
            (8, width * 6),
            (9, width * 3),
            (23, 0x3b00),
            (2, width * 2),
            (20, 17),
        ]);
        elf_relocation(&mut file, bits, 0x1a00, 0x33c0, 8, 0, None);
        elf_relocation(
            &mut file,
            bits,
            0x1a00 + (width * 2) as usize,
            0x33c0 + width,
            6,
            3,
            None,
        );
        elf_relocation(&mut file, bits, 0x1a80, 0x33e0, 1, 1, Some(-7));
        elf_relocation(
            &mut file,
            bits,
            0x1a80 + (width * 3) as usize,
            0x33f0,
            5,
            4,
            Some(0),
        );
        elf_relocation(&mut file, bits, 0x1b00, 0x33f8, 7, 3, None);
        put(&mut file, 0x13c0, 0x1234, bits / 8);
        elf_dynamic(&mut file, bits, &tags);
        let image = Image::parse(&file, Some(0x200000), LoadRole::Library).unwrap();
        let rows = &image.relocations;
        assert_eq!(rows.len(), 5);
        assert_eq!(
            rows.iter().map(|r| r.kind).collect::<Vec<_>>(),
            vec![8, 6, 1, 5, 7]
        );
        assert_eq!(rows[0].address, 0x2033c0);
        assert_eq!(rows[0].bytes, bits / 8);
        assert_eq!(rows[0].addend, None);
        assert_eq!(rows[2].addend, Some(-7));
        assert_eq!(rows[2].symbol, Some(1));
        assert_eq!(rows[3].bytes, 4);
        assert_eq!(
            &image.bytes[0x33c0..0x33c0 + bits / 8],
            &0x1234u64.to_le_bytes()[..bits / 8]
        );
        *tags.iter_mut().find(|(tag, _)| *tag == 23).unwrap() = (23, 0x3a80);
        *tags.iter_mut().find(|(tag, _)| *tag == 2).unwrap() = (2, width * 6);
        *tags.iter_mut().find(|(tag, _)| *tag == 20).unwrap() = (20, 7);
        elf_dynamic(&mut file, bits, &tags);
        let image = Image::parse(&file, None, LoadRole::Library).unwrap();
        assert_eq!(image.relocations.len(), 4);
    }
}
#[test]
fn elf_relr_expands_native_bitmap_order_and_requires_a_base() {
    for bits in [32, 64] {
        let (mut file, mut tags) = elf_symbol_fixture(bits);
        let width = bits / 8;
        tags.extend([(36, 0x3a00), (35, (width * 3) as u64), (37, width as u64)]);
        for (i, value) in [0x33c0, 0b1011, 0x33f0].into_iter().enumerate() {
            put(&mut file, 0x1a00 + i * width, value, width);
        }
        elf_dynamic(&mut file, bits, &tags);
        let image = Image::parse(&file, Some(0x200000), LoadRole::Library).unwrap();
        assert_eq!(
            image
                .relocations
                .iter()
                .map(|r| r.address)
                .collect::<Vec<_>>(),
            vec![
                0x2033c0,
                0x2033c0 + width as u64,
                0x2033c0 + width as u64 * 3,
                0x2033f0
            ]
        );
        assert!(
            image.relocations.iter().all(|r| r.kind == 8
                && r.symbol.is_none()
                && r.addend.is_none()
                && r.bytes == width)
        );
        put(&mut file, 0x1a00, 3, width);
        assert!(Image::parse(&file, None, LoadRole::Library).is_err());
        put(&mut file, 0x1a00, u64::MAX - 1, width);
        assert!(Image::parse(&file, None, LoadRole::Library).is_err());
    }
}
#[test]
fn elf_relocation_admission_rejects_bad_indices_fields_and_unpaired_tables() {
    for bits in [32, 64] {
        let (mut file, mut tags) = elf_symbol_fixture(bits);
        let width = (bits / 8) as u64;
        tags.extend([(7, 0x3a00), (8, width * 3), (9, width * 3)]);
        elf_dynamic(&mut file, bits, &tags);
        for (address, kind, symbol) in [
            (0x2000, 1, 1),
            (0x33c0, 1, 6),
            (0x33c0, 8, 1),
            (0x33c0, 5, 0),
            (0x33c0, 4, 1),
            (0x4180 - width + 1, 1, 1),
        ] {
            elf_relocation(&mut file, bits, 0x1a00, address, kind, symbol, Some(0));
            assert!(Image::parse(&file, None, LoadRole::Library).is_err());
        }
        elf_relocation(&mut file, bits, 0x1a00, 0x33c0, 1, 1, Some(0));
        assert!(Image::parse(&file, None, LoadRole::Library).is_ok());
        for tag in [7, 8, 9] {
            let mut bad = tags.clone();
            bad.retain(|&(t, _)| t != tag);
            elf_dynamic(&mut file, bits, &bad);
            assert!(Image::parse(&file, None, LoadRole::Library).is_err());
        }
        let mut bad = tags.clone();
        *bad.iter_mut().find(|(tag, _)| *tag == 8).unwrap() = (8, width * 3 - 1);
        elf_dynamic(&mut file, bits, &bad);
        assert!(Image::parse(&file, None, LoadRole::Library).is_err());
    }
}
#[test]
fn elf_x64_narrow_relocation_fields_keep_their_native_width() {
    let (mut file, mut tags) = elf_symbol_fixture(64);
    let kinds = [2, 10, 11, 12, 13, 14, 15, 24];
    tags.extend([(7, 0x3a00), (8, (kinds.len() * 24) as u64), (9, 24)]);
    for (i, kind) in kinds.into_iter().enumerate() {
        elf_relocation(
            &mut file,
            64,
            0x1a00 + i * 24,
            0x33c0 + i as u64 * 8,
            kind,
            1,
            Some(-1),
        );
    }
    elf_dynamic(&mut file, 64, &tags);
    let image = Image::parse(&file, None, LoadRole::Library).unwrap();
    assert_eq!(
        image
            .relocations
            .iter()
            .map(|r| r.bytes)
            .collect::<Vec<_>>(),
        vec![4, 4, 4, 2, 2, 1, 1, 8]
    );
    assert!(image.relocations.iter().all(|r| r.addend == Some(-1)));
    elf_relocation(&mut file, 64, 0x1a00, u64::MAX, 0, 0, Some(0));
    let image = Image::parse(&file, Some(0x200000), LoadRole::Library).unwrap();
    assert_eq!(image.relocations[0].bytes, 0);
    assert_eq!(image.relocations[0].address, u64::MAX);
}
