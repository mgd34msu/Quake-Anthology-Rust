#![cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[path = "../../formats/tests/support/native_image.rs"]
#[allow(dead_code)]
mod native_image;
use native_image::*;
use qa_compat::native::elf::{Bindings, Definition, Pass, TlsBlock, bind};
use qa_formats::{
    FormatError,
    program::native::{Image, LoadRole},
};

fn image(bits: usize, rows: &[(u32, u32, i64)]) -> Image {
    let (mut bytes, mut tags) = elf_symbol_fixture(bits);
    let stride = bits / 8 * 3;
    tags.extend([
        (7, 0x3a00),
        (8, (rows.len() * stride) as u64),
        (9, stride as u64),
    ]);
    for (i, &(kind, symbol, addend)) in rows.iter().enumerate() {
        elf_relocation(
            &mut bytes,
            bits,
            0x1a00 + i * stride,
            0x33c0 + i as u64 * 8,
            kind,
            symbol,
            Some(addend),
        );
    }
    elf_dynamic(&mut bytes, bits, &tags);
    Image::parse(&bytes, Some(0x200000), LoadRole::Library).unwrap()
}
fn word(image: &Image, index: usize) -> u64 {
    let row = &image.relocations[index];
    let at = (row.address - image.base) as usize;
    let mut bytes = [0; 8];
    bytes[..row.bytes].copy_from_slice(&image.bytes[at..at + row.bytes]);
    u64::from_le_bytes(bytes)
}

#[test]
fn address_relative_size_and_native_addend_rules_match_the_c_binder() {
    for bits in [32, 64] {
        let rows = [
            (1, 1, -7),
            (8, 0, 25),
            (6, 3, 99),
            (7, 3, 101),
            (2, 1, -2),
            (if bits == 64 { 33 } else { 38 }, 1, 3),
        ];
        let mut loaded = image(bits, &rows);
        let mut symbols = vec![None; loaded.symbols.len()];
        symbols[3] = Some(Definition::Address {
            address: 0x76543210,
            bytes: 7,
        });
        let indirect = vec![None; rows.len()];
        bind(
            &mut loaded,
            &Bindings {
                symbols: &symbols,
                local_tls: None,
                indirect: &indirect,
            },
            Pass::Regular,
        )
        .unwrap();
        assert_eq!(word(&loaded, 0), 0x2033c0 - 7);
        assert_eq!(word(&loaded, 1), 0x200000 + 25);
        assert_eq!(word(&loaded, 2), 0x76543210);
        assert_eq!(word(&loaded, 3), 0x76543210);
        assert_eq!(word(&loaded, 4), u64::from((-34i32) as u32));
        assert_eq!(word(&loaded, 5), 51);
        let (mut file, mut tags) = elf_symbol_fixture(bits);
        let width = bits / 8;
        tags.extend([
            (17, 0x3a00),
            (18, (width * 2) as u64),
            (19, (width * 2) as u64),
        ]);
        elf_relocation(&mut file, bits, 0x1a00, 0x33c0, 1, 1, None);
        put(&mut file, 0x13c0, (-7i64) as u64, width);
        elf_dynamic(&mut file, bits, &tags);
        let mut loaded = Image::parse(&file, Some(0x200000), LoadRole::Library).unwrap();
        let symbols = vec![None; loaded.symbols.len()];
        bind(
            &mut loaded,
            &Bindings {
                symbols: &symbols,
                local_tls: None,
                indirect: &[None],
            },
            Pass::Regular,
        )
        .unwrap();
        assert_eq!(word(&loaded, 0), 0x2033c0 - 7);
    }
}

#[test]
fn native_tls_forms_keep_module_offset_sign_and_addend_differences() {
    for bits in [32, 64] {
        let rows = if bits == 64 {
            vec![(16, 0, 8), (17, 5, 2), (18, 5, 3), (23, 5, 3)]
        } else {
            vec![(35, 0, 8), (36, 5, 2), (14, 5, 3), (34, 5, 3), (37, 5, 3)]
        };
        let mut loaded = image(bits, &rows);
        let symbols = vec![None; loaded.symbols.len()];
        let indirect = vec![None; rows.len()];
        let block = TlsBlock {
            module: 3,
            bytes: 16,
            thread_pointer_offset: -48,
        };
        bind(
            &mut loaded,
            &Bindings {
                symbols: &symbols,
                local_tls: Some(block),
                indirect: &indirect,
            },
            Pass::Regular,
        )
        .unwrap();
        assert_eq!(word(&loaded, 0), 3);
        assert_eq!(word(&loaded, 1), if bits == 64 { 6 } else { 4 });
        assert_eq!(
            word(&loaded, 2),
            if bits == 64 {
                (-41i64) as u64
            } else {
                u64::from((-41i32) as u32)
            }
        );
        assert_eq!(
            word(&loaded, 3),
            if bits == 64 {
                u64::from((-41i32) as u32)
            } else {
                47
            }
        );
        if bits == 32 {
            assert_eq!(word(&loaded, 4), 47);
        }
    }
}

#[test]
fn copy_uses_the_external_provider_prefix_and_preserves_remaining_bytes() {
    for bits in [32, 64] {
        let mut loaded = image(bits, &[(5, 4, 0)]);
        loaded.bytes[0x33c0..0x33c4].copy_from_slice(b"keep");
        let mut symbols = vec![None; loaded.symbols.len()];
        symbols[4] = Some(Definition::Copy {
            address: 0x400000,
            bytes: b"ab",
        });
        bind(
            &mut loaded,
            &Bindings {
                symbols: &symbols,
                local_tls: None,
                indirect: &[None],
            },
            Pass::Regular,
        )
        .unwrap();
        assert_eq!(&loaded.bytes[0x33c0..0x33c4], b"abep");
        symbols[4] = Some(Definition::Copy {
            address: loaded.base,
            bytes: b"ab",
        });
        assert_eq!(
            bind(
                &mut loaded,
                &Bindings {
                    symbols: &symbols,
                    local_tls: None,
                    indirect: &[None]
                },
                Pass::Regular
            ),
            Err(FormatError::InvalidRange)
        );
    }
}

#[test]
fn x64_narrow_fields_check_signed_limits_while_i386_wraps() {
    for (kind, wanted, valid) in [
        (10, u32::MAX as i64, true),
        (10, u32::MAX as i64 + 1, false),
        (11, i32::MAX as i64, true),
        (11, i32::MAX as i64 + 1, false),
        (11, i32::MIN as i64, true),
        (11, i32::MIN as i64 - 1, false),
        (12, 65535, true),
        (12, 65536, false),
        (14, 255, true),
        (14, 256, false),
    ] {
        let mut loaded = image(64, &[(kind, 3, wanted)]);
        let symbols = vec![None; loaded.symbols.len()];
        assert_eq!(
            bind(
                &mut loaded,
                &Bindings {
                    symbols: &symbols,
                    local_tls: None,
                    indirect: &[None]
                },
                Pass::Regular
            )
            .is_ok(),
            valid
        );
        if valid {
            assert_eq!(
                word(&loaded, 0),
                wanted as u64 & ((1u64 << (loaded.relocations[0].bytes * 8)) - 1)
            );
        }
    }
    let mut loaded = image(32, &[(1, 3, 1)]);
    let mut symbols = vec![None; loaded.symbols.len()];
    symbols[3] = Some(Definition::Address {
        address: u32::MAX as u64,
        bytes: 0,
    });
    bind(
        &mut loaded,
        &Bindings {
            symbols: &symbols,
            local_tls: None,
            indirect: &[None],
        },
        Pass::Regular,
    )
    .unwrap();
    assert_eq!(word(&loaded, 0), 0);
}

#[test]
fn indirect_entries_are_deferred_and_missing_strong_symbols_are_named() {
    for bits in [32, 64] {
        let mut loaded = image(bits, &[(if bits == 64 { 37 } else { 42 }, 0, 32)]);
        let symbols = vec![None; loaded.symbols.len()];
        let bindings = Bindings {
            symbols: &symbols,
            local_tls: None,
            indirect: &[Some(0x700000)],
        };
        let before = word(&loaded, 0);
        bind(&mut loaded, &bindings, Pass::Regular).unwrap();
        assert_eq!(word(&loaded, 0), before);
        bind(&mut loaded, &bindings, Pass::Indirect).unwrap();
        assert_eq!(word(&loaded, 0), 0x700000);
        let mut loaded = image(bits, &[(6, 3, 0)]);
        loaded.symbols[3].weak = false;
        assert_eq!(
            bind(
                &mut loaded,
                &Bindings {
                    symbols: &symbols,
                    local_tls: None,
                    indirect: &[None]
                },
                Pass::Regular
            ),
            Err(FormatError::InvalidReference("ELF unresolved symbol", 3))
        );
    }
}
