use super::native_image::*;
use qa_compat::native::{Error, Vm};
use qa_formats::program::native::{Image, LoadRole};
use qa_platform::native::{NativeAbi, NativeError, runtime::FUNCTIONS};
use std::time::Duration;

fn elf(name: &[u8], provider: &[u8], version: Option<&[u8]>) -> Image {
    let (mut file, mut tags) = elf_symbol_fixture(64);
    let labels = [
        b"vmMain".as_slice(),
        b"helper",
        name,
        b"COUNT",
        b"tls",
        provider,
        version.unwrap_or(b""),
    ];
    let mut strings = vec![0];
    let offsets = labels.map(|label| {
        let at = strings.len() as u64;
        strings.extend(label);
        strings.push(0);
        at
    });
    file[0x1280..0x1280 + strings.len()].copy_from_slice(&strings);
    tags.iter_mut().find(|(tag, _)| *tag == 10).unwrap().1 = strings.len() as u64;
    for (index, offset) in offsets[..5].iter().copied().enumerate() {
        put(&mut file, 0x1400 + (index + 1) * 24, offset, 4);
    }
    put(&mut file, 0x1400 + 3 * 24 + 4, 0x12, 1); // strong undefined function.
    tags.extend([(7, 0x3a00), (8, 24), (9, 24)]);
    elf_relocation(&mut file, 64, 0x1a00, 0x33b0, 7, 3, Some(0));
    if version.is_some() {
        tags.extend([(0x6ffffff0, 0x3800), (0x6ffffffe, 0x3880), (0x6fffffff, 1)]);
        file[0x1800..0x18b0].fill(0);
        for (index, value) in [0, 1, 1, 2, 1, 1].into_iter().enumerate() {
            put(&mut file, 0x1800 + index * 2, value, 2);
        }
        for (offset, value, width) in [
            (0, 1, 2),
            (2, 1, 2),
            (4, offsets[5], 4),
            (8, 16, 4),
            (22, 2, 2),
            (24, offsets[6], 4),
        ] {
            put(&mut file, 0x1880 + offset, value, width);
        }
    }
    elf_dynamic(&mut file, 64, &tags);
    Image::parse(&file, Some(0x200000), LoadRole::Library).unwrap()
}
fn pe(provider: &[u8]) -> Image {
    let mut file = pe_fixture(64);
    pe_directory(&mut file, 64, 1, 0x1100, 40);
    for (offset, value) in [(0, 0x1128), (12, 0x1180), (16, 0x1148)] {
        pe_rva(&mut file, 0x1100 + offset, value, 4);
    }
    for first in [0x1128, 0x1148] {
        pe_rva(&mut file, first, 0x1168, 8);
        pe_rva(&mut file, first + 8, 0x1170, 8);
    }
    pe_text(&mut file, 0x1168, b"\0\0rand\0");
    pe_text(&mut file, 0x1170, b"\0\0srand\0");
    pe_text(&mut file, 0x1180, provider);
    pe_text(&mut file, 0x1180 + provider.len(), b"\0");
    Image::parse(&file, None, LoadRole::Library).unwrap()
}
fn invoke(
    vm: &mut Vm,
    ordinal: usize,
    abi: NativeAbi,
    name: &[u8],
    value: u64,
) -> Result<u64, NativeError> {
    let function = FUNCTIONS
        .iter()
        .find(|function| function.name == name)
        .unwrap();
    let entry = vm
        .process
        .bind(
            vm.process.import_pointer(ordinal).unwrap(),
            abi,
            function.parameters,
            function.result,
        )
        .unwrap();
    let mut words = [0; 13];
    words[0] = value;
    vm.process.invoke::<()>(entry, words, |_, _, _| {
        panic!("bound CRT random stays in child")
    })
}

fn elf_crt_random_name_and_version_bindings_patch_the_live_import_slot() {
    for name in [b"rand".as_slice(), b"srand"] {
        for version in [None, Some(b"GLIBC_2.2.5".as_slice())] {
            let mut vm = Vm::map_image(
                elf(name, b"libc.so.6", version),
                &[],
                &[],
                Duration::from_secs(3),
            )
            .unwrap();
            assert_eq!(vm.unresolved_imports().count(), 0);
            let patched = u64::from_le_bytes(
                vm.process.memory().unwrap()[0x33b0..0x33b8]
                    .try_into()
                    .unwrap(),
            );
            assert_eq!(patched, vm.process.import_pointer(0).unwrap());
            assert_eq!(
                invoke(&mut vm, 0, NativeAbi::SystemV, name, 42).unwrap(),
                if name == b"rand" { 1804289383 } else { 0 }
            );
        }
        for (provider, version) in [
            (b"libc.so.6".as_slice(), b"GLIBC_2.0".as_slice()),
            (b"libc.so.6", b"GLIBC_2.99"),
            (b"libm.so.6", b"GLIBC_2.2.5"),
        ] {
            assert!(matches!(
                Vm::map_image(
                    elf(name, provider, Some(version)),
                    &[],
                    &[],
                    Duration::from_secs(3)
                ),
                Err(Error::Binding(_))
            ));
        }
    }
}

fn pe_crt_random_imports_use_microsoft_profile_and_reject_unknown_provider_at_execution() {
    for provider in [
        b"msvcrt.dll".as_slice(),
        b"UCRTBASE.DLL",
        b"api-ms-win-crt-utility-l1-1-0.dll",
    ] {
        let mut vm = Vm::map_image(pe(provider), &[], &[], Duration::from_secs(3)).unwrap();
        assert_eq!(vm.unresolved_imports().count(), 0);
        assert_eq!(
            invoke(&mut vm, 0, NativeAbi::Microsoft, b"rand", 0).unwrap(),
            41
        );
        assert_eq!(
            invoke(&mut vm, 1, NativeAbi::Microsoft, b"srand", 1792).unwrap(),
            0
        );
        assert_eq!(
            invoke(&mut vm, 0, NativeAbi::Microsoft, b"rand", 0).unwrap(),
            5890
        );
    }
    let mut vm = Vm::map_image(pe(b"arbitrary.dll"), &[], &[], Duration::from_secs(3)).unwrap();
    assert_eq!(vm.unresolved_imports().count(), 2);
    assert!(matches!(
        invoke(&mut vm, 0, NativeAbi::Microsoft, b"rand", 0),
        Err(NativeError::ImportTrap { .. })
    ));
}

pub(super) fn run() {
    elf_crt_random_name_and_version_bindings_patch_the_live_import_slot();
    pe_crt_random_imports_use_microsoft_profile_and_reject_unknown_provider_at_execution();
    println!("Native CRT random ELF and PE bindings: both checks passed");
}
