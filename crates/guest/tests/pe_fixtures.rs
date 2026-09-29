//! Synthetic PE fixture tests: mapping, relocations, TLS, imports,
//! forwarder resolution, unwind records, and malformed-input rejection.
//!
//! Donor: `tests/guest/pe/fixture.ts` plus `tests/guest/pe/loader.test.ts`.
//! The installed-DLL suite needs user-supplied retail binaries outside the
//! source boundary, so only the authored fixture ports here.

mod common;

use qa_guest::core::contracts::{
    GuestAddress, GuestImage, GuestImport, GuestImportResolution, GuestImportResolver,
    GuestPermissions, GuestSymbolName, ModuleIdentity, NativeAbi,
};
use qa_guest::core::memory::SparseGuestMemory;
use qa_guest::error::GuestError;
use qa_guest::pe::exports::resolve_pe_export;
use qa_guest::pe::image::PeImage;
use qa_guest::pe::loader::{bind_pe_imports, map_pe_image, MapPeImageOptions};

use common::{map, pe_fixture, test_module};

fn at(memory: &SparseGuestMemory, base: GuestAddress, rva: u64) -> GuestAddress {
    memory.offset(base, rva as i64).unwrap()
}

fn pointer_value(memory: &mut SparseGuestMemory, address: GuestAddress) -> u64 {
    memory.read_pointer(address).unwrap().unwrap().offset
}

#[test]
fn pe_sections_relocations_tls_and_entry_point_share_one_address_space() {
    for width in [4usize, 8usize] {
        let bytes = pe_fixture(width);
        let snapshot = bytes.clone();
        let mut memory = SparseGuestMemory::new(test_module("pe"), width, 0x10000).unwrap();
        let base = if width == 4 { 0x1012_0000 } else { 0x1_a000_0000 };
        let image = map_pe_image(MapPeImageOptions {
            bytes: &bytes,
            memory: &mut memory,
            module: None,
            base: Some(base),
            maximum_image_bytes: None,
        })
        .unwrap();
        assert_eq!(image.abi(), if width == 4 { NativeAbi::WindowsI386 } else { NativeAbi::WindowsX86_64 });
        assert_eq!(image.image.entry_point.unwrap().offset, base + 0x1000);
        assert!(image.image.initializers.is_empty());
        assert!(image.image.finalizers.is_empty());
        let slot = at(&memory, image.image.base, 0x3010);
        assert_eq!(pointer_value(&mut memory, slot), base + 0x1050);
        assert!(memory.copy(at(&memory, image.image.base, 0x3200), 0x200).unwrap().iter().all(|value| *value == 0));
        assert_eq!(memory.fetch(at(&memory, image.image.base, 0x1010), 1).unwrap(), [0xc3]);
        assert!(memory.write(at(&memory, image.image.base, 0x1010), &[0x90]).is_err());
        assert!(memory.fetch(at(&memory, image.image.base, 0x3000), 1).is_err());
        assert_eq!(memory.copy(at(&memory, image.image.base, 0x400), 4).unwrap(), [0, 0, 0, 0]);
        let tls = image.image.tls.as_ref().unwrap();
        assert_eq!(tls.initialized, [9, 8, 7, 6]);
        assert_eq!(tls.zero_fill_bytes, 12);
        assert_eq!(tls.alignment, 4);
        let callbacks: Vec<u64> = tls.callbacks.iter().map(|address| address.offset).collect();
        assert_eq!(callbacks, [base + 0x1020, base + 0x1030]);
        assert_eq!(image.tls_index_address.unwrap().offset, base + 0x3020);
        let configuration = image.load_configuration.as_ref().unwrap();
        assert_eq!(configuration.security_cookie_address.unwrap().offset, base + 0x3040);
        assert_eq!(configuration.guard_check_slot.unwrap().offset, base + 0x3060);
        assert_eq!(configuration.guard_dispatch_slot.unwrap().offset, base + 0x3070);
        assert_eq!(configuration.guard_flags, 0x100);
        assert_eq!(image.image.unwind.len(), if width == 8 { 1 } else { 0 });
        if width == 4 {
            assert_eq!(memory.read_u16(at(&memory, image.image.base, 0x3018)).unwrap(), 0x1134);
            assert_eq!(memory.read_u16(at(&memory, image.image.base, 0x301a)).unwrap(), 0x3344);
            assert_eq!(memory.read_u16(at(&memory, image.image.base, 0x301c)).unwrap(), 0x2012);
        } else {
            let region = &image.image.unwind[0];
            assert_eq!(region.metadata, [1, 4, 1, 0, 4, 0x32, 0, 0]);
            assert_eq!(region.start.offset, base + 0x1000);
            assert_eq!(region.end.offset, base + 0x1040);
        }
        assert_eq!(bytes, snapshot);
    }
}

struct PeResolver {
    target: GuestAddress,
    module: ModuleIdentity,
    fail_ordinal: bool,
}

impl GuestImportResolver for PeResolver {
    fn resolve(
        &self,
        _memory: &mut SparseGuestMemory,
        import: &GuestImport,
        _requesting: &GuestImage,
    ) -> GuestImportResolution {
        if self.fail_ordinal && matches!(import.symbol, GuestSymbolName::Ordinal(_)) {
            return GuestImportResolution::Unresolved {
                import: import.clone(),
                detail: "ordinal unavailable".to_string(),
            };
        }
        GuestImportResolution::Guest { address: self.target, module: self.module.clone() }
    }
}

#[test]
fn pe_named_and_ordinal_imports_bind_atomically_and_keep_iat_read_only() {
    for width in [4usize, 8usize] {
        let mut memory = SparseGuestMemory::new(test_module("pe"), width, 0x10000).unwrap();
        let image = map_pe_image(MapPeImageOptions {
            bytes: &pe_fixture(width),
            memory: &mut memory,
            module: None,
            base: None,
            maximum_image_bytes: None,
        })
        .unwrap();
        let target = map(&mut memory, 0x5000_0000, 16, GuestPermissions::ReadExecute, Some(vec![0xc3]));
        let symbols: Vec<_> = image.image.imports.iter().map(|entry| &entry.symbol).collect();
        assert_eq!(
            symbols,
            [
                &GuestSymbolName::Name { name: "Target".to_string(), version: None },
                &GuestSymbolName::Ordinal(7),
            ]
        );
        let iat = at(&memory, image.image.base, 0x2160);
        let original = memory.copy(iat, width * 2).unwrap();
        let mappings = memory.mappings();
        let failing = PeResolver { target, module: test_module("pe"), fail_ordinal: true };
        let failed = bind_pe_imports(&image, &mut memory, &failing);
        assert!(format!("{failed:?}").contains("ordinal unavailable"), "{failed:?}");
        assert_eq!(memory.copy(iat, width * 2).unwrap(), original);
        assert_eq!(memory.mappings(), mappings);
        let passing = PeResolver { target, module: test_module("pe"), fail_ordinal: false };
        assert_eq!(bind_pe_imports(&image, &mut memory, &passing).unwrap().len(), 2);
        assert_eq!(pointer_value(&mut memory, iat), target.offset);
        let ordinal_slot = memory.offset(iat, width as i64).unwrap();
        assert_eq!(pointer_value(&mut memory, ordinal_slot), target.offset);
        assert_eq!(memory.mappings(), mappings);
        assert!(memory.write(iat, &vec![0u8; width]).is_err());
    }
}

#[test]
fn pe_named_exports_and_ordinal_forwarders_resolve_and_reject_cycles() {
    let mut memory = SparseGuestMemory::new(test_module("pe"), 4, 0x10000).unwrap();
    let image = map_pe_image(MapPeImageOptions {
        bytes: &pe_fixture(4),
        memory: &mut memory,
        module: None,
        base: None,
        maximum_image_bytes: None,
    })
    .unwrap();
    let other = map_pe_image(MapPeImageOptions {
        bytes: &pe_fixture(4),
        memory: &mut memory,
        module: None,
        base: Some(0x2000_0000),
        maximum_image_bytes: None,
    })
    .unwrap();
    let requested = GuestSymbolName::Name { name: "Forward".to_string(), version: None };
    let lookup = |library: &str, _: &PeImage| (library == "other").then(|| other.clone());
    assert_eq!(resolve_pe_export(&image, &requested, &lookup).unwrap().address.offset, 0x2000_1010);
    let direct = GuestSymbolName::Name { name: "GetGameAPI".to_string(), version: None };
    assert_eq!(resolve_pe_export(&image, &direct, &|_, _| None).unwrap().address.offset, 0x1000_1010);
    assert!(format!("{:?}", resolve_pe_export(&image, &requested, &|_, _| None)).contains("unresolved forwarded library"));
    let mut cyclic = pe_fixture(4);
    cyclic[0x680..0x680 + 14].copy_from_slice(b"other.Forward\0");
    let cycle = map_pe_image(MapPeImageOptions {
        bytes: &cyclic,
        memory: &mut memory,
        module: None,
        base: Some(0x3000_0000),
        maximum_image_bytes: None,
    })
    .unwrap();
    let cycled = cycle.clone();
    assert!(format!("{:?}", resolve_pe_export(&cycle, &requested, &|_, _| Some(cycled.clone()))).contains("cyclic export forwarder"));
    let mut foreign_memory = SparseGuestMemory::new(test_module("pe"), 4, 0x10000).unwrap();
    let foreign = map_pe_image(MapPeImageOptions {
        bytes: &pe_fixture(4),
        memory: &mut foreign_memory,
        module: None,
        base: None,
        maximum_image_bytes: None,
    })
    .unwrap();
    assert!(format!("{:?}", resolve_pe_export(&image, &requested, &|_, _| Some(foreign.clone()))).contains("another guest address space"));
}

#[test]
fn pe_x64_unwind_chains_and_handler_locations_stay_available() {
    let mut bytes = pe_fixture(8);
    bytes[0x9a0..0x9a4].copy_from_slice(&[0x21, 0, 0, 0]);
    bytes[0x9a4..0x9a8].copy_from_slice(&0x1000u32.to_le_bytes());
    bytes[0x9a8..0x9ac].copy_from_slice(&0x1040u32.to_le_bytes());
    bytes[0x9ac..0x9b0].copy_from_slice(&0x23b0u32.to_le_bytes());
    bytes[0x9b0..0x9b4].copy_from_slice(&[9, 0, 0, 0]);
    bytes[0x9b4..0x9b8].copy_from_slice(&0x1050u32.to_le_bytes());
    bytes[0x9b8..0x9bc].copy_from_slice(&0x1234_5678u32.to_le_bytes());
    let mut memory = SparseGuestMemory::new(test_module("pe"), 8, 0x10000).unwrap();
    let image = map_pe_image(MapPeImageOptions {
        bytes: &bytes,
        memory: &mut memory,
        module: None,
        base: None,
        maximum_image_bytes: None,
    })
    .unwrap();
    let chained = image.unwind_records[0].chained.as_ref().unwrap();
    assert_eq!(chained.handler_rva, Some(0x1050));
    assert_eq!(chained.handler_data_rva, Some(0x23b8));
    assert_eq!(memory.read_u32(at(&memory, image.image.base, 0x23b8)).unwrap(), 0x1234_5678);
    bytes[0x9ac..0x9b0].copy_from_slice(&0x23a0u32.to_le_bytes());
    let mut memory = SparseGuestMemory::new(test_module("pe"), 8, 0x10000).unwrap();
    let failed = map_pe_image(MapPeImageOptions {
        bytes: &bytes,
        memory: &mut memory,
        module: None,
        base: None,
        maximum_image_bytes: None,
    });
    assert!(format!("{failed:?}").contains("cyclic unwind chain"), "{failed:?}");
}

fn patch(bytes: &mut [u8], offset: usize, value: &[u8]) {
    bytes[offset..offset + value.len()].copy_from_slice(value);
}

#[test]
fn pe_truncated_overlapping_and_unsupported_input_fails_before_mapping() {
    let cases: &[(&str, fn(&mut Vec<u8>))] = &[
        ("truncated", |bytes| bytes.truncate(96)),
        ("section overlap", |bytes| patch(bytes, 0x1ac, &0x1000u32.to_le_bytes())),
        ("unsupported relocation", |bytes| patch(bytes, 0x1008, &0x7200u16.to_le_bytes())),
        ("missing relocation", |bytes| {
            patch(bytes, 0x120, &0u32.to_le_bytes());
            patch(bytes, 0x124, &0u32.to_le_bytes());
        }),
        ("delay imports", |bytes| {
            patch(bytes, 0x160, &0x2100u32.to_le_bytes());
            patch(bytes, 0x164, &40u32.to_le_bytes());
        }),
        ("bad TLS callback", |bytes| patch(bytes, 0x840, &0x1000_3000u32.to_le_bytes())),
        ("unterminated import descriptors", |bytes| patch(bytes, 0x104, &20u32.to_le_bytes())),
    ];
    for (label, mutate) in cases {
        let mut bytes = pe_fixture(4);
        mutate(&mut bytes);
        let mut memory = SparseGuestMemory::new(test_module("pe"), 4, 0x10000).unwrap();
        let sentinel = map(&mut memory, 0x10000, 8, GuestPermissions::Read, Some(vec![17]));
        let prior = memory.mappings();
        let failed = map_pe_image(MapPeImageOptions {
            bytes: &bytes,
            memory: &mut memory,
            module: None,
            base: Some(0x2000_0000),
            maximum_image_bytes: None,
        });
        assert!(failed.is_err(), "{label}");
        assert_eq!(memory.mappings(), prior, "{label}");
        assert_eq!(memory.copy(sentinel, 1).unwrap(), [17], "{label}");
    }
    let truncated = pe_fixture(8)[..0x1100].to_vec();
    assert!(qa_guest::pe::format::parse_pe(&truncated).is_err());
    let mut memory = SparseGuestMemory::new(test_module("pe"), 4, 0x10000).unwrap();
    let failed = map_pe_image(MapPeImageOptions {
        bytes: &pe_fixture(8),
        memory: &mut memory,
        module: None,
        base: None,
        maximum_image_bytes: None,
    });
    assert!(format!("{failed:?}").contains("pointer width"), "{failed:?}");
}
