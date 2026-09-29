//! Synthetic ELF fixture tests: TLS relocations, versioned exports, and
//! indirect (IFUNC) resolution for both ELF classes.
//!
//! Donor: `tests/guest/elf/tls.test.ts`. The donor's Quake Live witness
//! suite needs supplied retail archives outside the source boundary, so this
//! ports the fully synthetic sectionless-ELF fixture only.

mod common;

use std::cell::Cell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use qa_guest::core::contracts::{
    GuestAddress, GuestImage, GuestImport, GuestImportResolution, GuestImportResolver, GuestSymbolName,
};
use qa_guest::core::memory::SparseGuestMemory;
use qa_guest::elf::loader::{load_elf, ElfLoadOptions};
use qa_guest::elf::relocate::{ElfTlsBindings, ElfTlsModule, ElfTlsResolution};
use qa_guest::error::GuestError;

use common::test_module;

const LOAD_BIAS: u64 = 0x2000_0000;

struct Fixture {
    bytes: Vec<u8>,
    wide: bool,
}

impl Fixture {
    fn word(&mut self, offset: usize, value: u64) {
        if self.wide {
            self.bytes[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
        } else {
            #[allow(clippy::cast_possible_truncation)]
            let value = value as u32;
            self.bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
        }
    }

    fn build(width: usize) -> Self {
        let wide = width == 8;
        let mut fixture = Self {
            bytes: vec![0; 4096],
            wide,
        };
        fixture.bytes[0..4].copy_from_slice(&0x464c_457fu32.to_le_bytes());
        fixture.bytes[4] = if wide { 2 } else { 1 };
        fixture.bytes[5] = 1;
        fixture.bytes[6] = 1;
        fixture.bytes[16..18].copy_from_slice(&3u16.to_le_bytes());
        fixture.bytes[18..20].copy_from_slice(&(if wide { 62u16 } else { 3u16 }).to_le_bytes());
        fixture.bytes[20..24].copy_from_slice(&1u32.to_le_bytes());
        fixture.word(if wide { 32 } else { 28 }, 64);
        let (ehsize, phentsize, phoff) = if wide {
            (64u16, 56u16, 52usize)
        } else {
            (52u16, 32u16, 40usize)
        };
        fixture.bytes[phoff..phoff + 2].copy_from_slice(&ehsize.to_le_bytes());
        fixture.bytes[phoff + 2..phoff + 4].copy_from_slice(&phentsize.to_le_bytes());
        fixture.bytes[phoff + 4..phoff + 6].copy_from_slice(&3u16.to_le_bytes());
        let tags: &[(u64, u64)] = &[
            (4, 0x4c0),
            (5, 0x480),
            (10, 22),
            (6, 0x400),
            (11, if wide { 24 } else { 16 }),
            (7, 0x500),
            (8, (width * 3 * 10) as u64),
            (9, (width * 3) as u64),
            (12, 0x800),
            (13, 0x810),
            (25, 0x980),
            (27, (width * 2) as u64),
            (26, 0x9a0),
            (28, (width * 2) as u64),
            (0x6fff_fffc, 0x680),
            (0x6fff_fffd, 1),
            (0x6fff_fff0, 0x4e0),
            (0, 0),
        ];
        let stride = if wide { 56 } else { 32 };
        let segment = |fixture: &mut Fixture, index: usize, kind: u32, offset: u64, file: u64, mem: u64, align: u64| {
            let p = 64 + index * stride;
            fixture.bytes[p..p + 4].copy_from_slice(&kind.to_le_bytes());
            fixture.bytes[p + if wide { 4 } else { 24 }..p + if wide { 8 } else { 28 }]
                .copy_from_slice(&7u32.to_le_bytes());
            fixture.word(p + if wide { 8 } else { 4 }, offset);
            fixture.word(p + if wide { 16 } else { 8 }, offset);
            fixture.word(p + if wide { 32 } else { 16 }, file);
            fixture.word(p + if wide { 40 } else { 20 }, mem);
            fixture.word(p + if wide { 48 } else { 28 }, align);
        };
        segment(&mut fixture, 0, 1, 0, 4096, 8192, 4096);
        segment(
            &mut fixture,
            1,
            2,
            0x200,
            (tags.len() * width * 2) as u64,
            (tags.len() * width * 2) as u64,
            width as u64,
        );
        segment(&mut fixture, 2, 7, 0x700, 16, 32, 16);
        for (index, (tag, value)) in tags.iter().enumerate() {
            fixture.word(0x200 + index * width * 2, *tag);
            fixture.word(0x200 + index * width * 2 + width, *value);
        }
        fixture.bytes[0x480..0x480 + 22].copy_from_slice(b"\0tls\0entry\0ELF_TEST_1\0");
        fixture.bytes[0x4c0..0x4c4].copy_from_slice(&1u32.to_le_bytes());
        fixture.bytes[0x4c4..0x4c8].copy_from_slice(&3u32.to_le_bytes());
        for (index, name, kind, value) in [(1usize, 1u32, 6u8, 4u64), (2, 5, 2, 0x800)] {
            let p = 0x400 + index * if wide { 24 } else { 16 };
            fixture.bytes[p..p + 4].copy_from_slice(&name.to_le_bytes());
            fixture.bytes[p + if wide { 4 } else { 12 }] = 16 + kind;
            fixture.bytes[p + if wide { 6 } else { 14 }..p + if wide { 8 } else { 16 }]
                .copy_from_slice(&1u16.to_le_bytes());
            fixture.word(p + if wide { 8 } else { 4 }, value);
            fixture.word(p + if wide { 16 } else { 8 }, 4);
        }
        fixture.bytes[0x4e2..0x4e4].copy_from_slice(&1u16.to_le_bytes());
        fixture.bytes[0x4e4..0x4e6].copy_from_slice(&2u16.to_le_bytes());
        fixture.bytes[0x680..0x682].copy_from_slice(&1u16.to_le_bytes());
        fixture.bytes[0x684..0x686].copy_from_slice(&2u16.to_le_bytes());
        fixture.bytes[0x686..0x688].copy_from_slice(&1u16.to_le_bytes());
        fixture.bytes[0x68c..0x690].copy_from_slice(&20u32.to_le_bytes());
        fixture.bytes[0x694..0x698].copy_from_slice(&11u32.to_le_bytes());
        let relocations: &[(u64, u64, u64, u64)] = &[
            (0x700, 8, 0, 0x800),
            (0x900, if wide { 16 } else { 35 }, 0, 0),
            (0x910, if wide { 17 } else { 36 }, 1, 7),
            (0x920, if wide { 18 } else { 14 }, 1, 3),
            (0x928, if wide { 23 } else { 37 }, 1, 5),
            (0x980, 8, 0, 0x800),
            (0x980 + width as u64, 8, 0, 0x810),
            (0x9a0, 8, 0, 0x820),
            (0x9a0 + width as u64, 8, 0, 0x830),
            (0x930, if wide { 37 } else { 42 }, 0, 0x840),
        ];
        for (index, (address, kind, symbol, addend)) in relocations.iter().enumerate() {
            let p = 0x500 + index * width * 3;
            fixture.word(p, *address);
            fixture.word(p + width, (symbol << if wide { 32 } else { 8 }) | kind);
            fixture.word(p + width * 2, *addend);
        }
        fixture.bytes[0x800..0x860].fill(0xc3);
        fixture
    }
}

struct Unresolved;
impl GuestImportResolver for Unresolved {
    fn resolve(
        &self,
        _memory: &mut SparseGuestMemory,
        import: &GuestImport,
        _requesting: &GuestImage,
    ) -> GuestImportResolution {
        GuestImportResolution::Unresolved {
            import: import.clone(),
            detail: "authored image has no external imports".to_string(),
        }
    }
}

struct Tls;
impl ElfTlsBindings for Tls {
    fn current(&self) -> ElfTlsModule {
        ElfTlsModule {
            module_id: 7,
            thread_pointer_offset: Some(-64),
        }
    }
    fn resolve(&self, _import: &GuestImport, _requesting: &GuestImage) -> Result<Option<ElfTlsResolution>, GuestError> {
        Ok(None)
    }
}

fn read(memory: &mut SparseGuestMemory, offset: u64, wide: bool) -> u64 {
    let address = memory.pointer(LOAD_BIAS + offset).unwrap().unwrap();
    if wide {
        memory.read_u64(address).unwrap()
    } else {
        u64::from(memory.read_u32(address).unwrap())
    }
}

#[test]
fn elf_tls_relocations_use_module_ids_signed_displacements_and_relocated_templates() {
    for wide in [false, true] {
        let width = if wide { 8 } else { 4 };
        let fixture = Fixture::build(width);
        let module = test_module("elf-tls");
        let dependencies = HashSet::new();
        let resolver = Unresolved;
        let tls = Tls;

        // TLS-bearing images need thread/module bindings before mapping.
        let mut memory = SparseGuestMemory::new(module.clone(), width, 0x10000).unwrap();
        let failed = load_elf(ElfLoadOptions {
            bytes: &fixture.bytes,
            module: module.clone(),
            memory: &mut memory,
            load_bias: LOAD_BIAS,
            resolver: &resolver,
            dependencies: &dependencies,
            tls: None,
            resolve_indirect: None,
            resolve_symbol_size: None,
            unique_symbols: None,
        });
        assert!(
            format!("{failed:?}").contains("requires a guest thread/module allocation"),
            "{failed:?}"
        );
        assert!(memory.mappings().is_empty());

        // Indirect relocations need explicit guest resolver execution.
        let mut memory = SparseGuestMemory::new(module.clone(), width, 0x10000).unwrap();
        let failed = load_elf(ElfLoadOptions {
            bytes: &fixture.bytes,
            module: module.clone(),
            memory: &mut memory,
            load_bias: LOAD_BIAS,
            resolver: &resolver,
            dependencies: &dependencies,
            tls: Some(&tls),
            resolve_indirect: None,
            resolve_symbol_size: None,
            unique_symbols: None,
        });
        assert!(
            format!("{failed:?}").contains("requires explicit guest resolver execution"),
            "{failed:?}"
        );
        assert!(memory.mappings().is_empty());

        // Unknown relocation types fail before mapping anything.
        let mut unsupported = fixture.bytes.clone();
        if wide {
            unsupported[0x508..0x510].copy_from_slice(&0xffff_ffffu64.to_le_bytes());
        } else {
            unsupported[0x504] = 0xff;
        }
        let mut memory = SparseGuestMemory::new(module.clone(), width, 0x10000).unwrap();
        let failed = load_elf(ElfLoadOptions {
            bytes: &unsupported,
            module: module.clone(),
            memory: &mut memory,
            load_bias: LOAD_BIAS,
            resolver: &resolver,
            dependencies: &dependencies,
            tls: Some(&tls),
            resolve_indirect: None,
            resolve_symbol_size: None,
            unique_symbols: None,
        });
        assert!(format!("{failed:?}").contains("unsupported"), "{failed:?}");
        assert!(memory.mappings().is_empty());

        let indirect_calls = Rc::new(Cell::new(0u32));
        let probe = Rc::clone(&indirect_calls);
        let mut memory = SparseGuestMemory::new(module.clone(), width, 0x10000).unwrap();
        let mut unique = HashMap::new();
        let image = load_elf(ElfLoadOptions {
            bytes: &fixture.bytes,
            module: module.clone(),
            memory: &mut memory,
            load_bias: LOAD_BIAS,
            resolver: &resolver,
            dependencies: &dependencies,
            tls: Some(&tls),
            resolve_indirect: Some(&|address: GuestAddress| {
                assert_eq!(address.offset, LOAD_BIAS + 0x840);
                probe.set(probe.get() + 1);
                Ok(GuestAddress::new(address.space, LOAD_BIAS + 0x850))
            }),
            resolve_symbol_size: None,
            unique_symbols: Some(&mut unique),
        })
        .unwrap();
        assert_eq!(indirect_calls.get(), 1);
        assert_eq!(read(&mut memory, 0x900, wide), 7);
        assert_eq!(read(&mut memory, 0x910, wide), if wide { 11 } else { 4 });
        let signed = read(&mut memory, 0x920, wide);
        assert_eq!(
            if wide {
                signed as i64
            } else {
                (signed as u32) as i32 as i64
            },
            -57
        );
        let slot = memory.pointer(LOAD_BIAS + 0x928).unwrap().unwrap();
        assert_eq!(memory.read_i32(slot).unwrap(), if wide { -55 } else { 65 });
        assert_eq!(read(&mut memory, 0x930, wide), LOAD_BIAS + 0x850);

        let template = image.image.tls.as_ref().expect("TLS template missing");
        let mut raw = [0u8; 8];
        raw.copy_from_slice(&template.initialized[..8]);
        let head = if wide {
            u64::from_le_bytes(raw)
        } else {
            u64::from(u32::from_le_bytes(raw[..4].try_into().unwrap()))
        };
        assert_eq!(head, LOAD_BIAS + 0x800);
        assert_eq!(template.zero_fill_bytes, 16);
        assert_eq!(template.alignment, 16);
        assert_eq!(image.tls_exports.len(), 1);
        assert_eq!(
            image.tls_exports[0].symbol,
            GuestSymbolName::Name {
                name: "tls".to_string(),
                version: None
            }
        );
        assert_eq!(image.tls_exports[0].offset, 4);
        assert_eq!(image.tls_exports[0].byte_length, 4);
        let symbols: Vec<_> = image.image.exports.iter().map(|export| &export.symbol).collect();
        assert_eq!(
            symbols,
            [
                &GuestSymbolName::Name {
                    name: "entry".to_string(),
                    version: Some("ELF_TEST_1".to_string())
                },
                &GuestSymbolName::Name {
                    name: "entry".to_string(),
                    version: None
                },
            ]
        );
        let initializers: Vec<u64> = image.image.initializers.iter().map(|address| address.offset).collect();
        assert_eq!(initializers, [0x800, 0x800, 0x810].map(|value| LOAD_BIAS + value));
        let finalizers: Vec<u64> = image.image.finalizers.iter().map(|address| address.offset).collect();
        assert_eq!(finalizers, [0x830, 0x820, 0x810].map(|value| LOAD_BIAS + value));
    }
}

#[test]
fn elf_inspection_rejects_bad_magic_and_truncated_headers() {
    use qa_guest::elf::parse::inspect_elf;
    let bad = vec![0u8; 64];
    assert!(inspect_elf(&bad).is_err());
    let short = vec![0x7f, b'E', b'L', b'F'];
    assert!(inspect_elf(&short).is_err());
    for wide in [false, true] {
        let fixture = Fixture::build(if wide { 8 } else { 4 });
        let inspection = inspect_elf(&fixture.bytes).unwrap();
        assert_eq!(inspection.relocations.len(), 10);
        assert_eq!(inspection.symbols.len(), 3);
    }
}
