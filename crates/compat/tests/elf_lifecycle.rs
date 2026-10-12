#![cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[path = "../../formats/tests/support/native_image.rs"]
#[allow(dead_code)]
mod native_image;

use native_image::{elf_dynamic, elf_symbol_fixture};
use qa_compat::{memory::ModuleMemory, native::elf::Lifecycle};
use qa_formats::{
    FormatError,
    program::native::{Image, LoadRole},
};

fn fixture(bits: usize) -> Image {
    let (mut file, mut tags) = elf_symbol_fixture(bits);
    let width = bits / 8;
    tags.extend([
        (12, 0x3700),
        (25, 0x3a00),
        (27, (2 * width) as u64),
        (26, 0x3a20),
        (28, (2 * width) as u64),
        (13, 0x3840),
    ]);
    elf_dynamic(&mut file, bits, &tags);
    let mut image = Image::parse(&file, Some(0x200000), LoadRole::Library).unwrap();
    let bias = image.base - image.preferred_base;
    let mut memory = ModuleMemory::borrow(image.base, &mut image.bytes).unwrap();
    for (array, entries) in [(0x3a00, [0x3740, 0x3780]), (0x3a20, [0x37c0, 0x3800])] {
        for (index, entry) in entries.into_iter().enumerate() {
            memory
                .write(
                    bias + array + (index * width) as u64,
                    &(bias + entry).to_le_bytes()[..width],
                )
                .unwrap();
        }
    }
    image
}

#[test]
fn init_and_reverse_fini_arrays_use_the_image_pointer_width() {
    for bits in [32, 64] {
        let mut image = fixture(bits);
        let source = image.bytes.len();
        let bias = image.base - image.preferred_base;
        let lifecycle = Lifecycle::load(&mut image, source).unwrap();
        assert_eq!(
            lifecycle.initialize,
            [0x3700, 0x3740, 0x3780].map(|at| bias + at)
        );
        assert_eq!(
            lifecycle.finalize,
            [0x3800, 0x37c0, 0x3840].map(|at| bias + at)
        );
    }
}

#[test]
fn malformed_array_sizes_are_rejected_for_both_widths() {
    for bits in [32, 64] {
        for tag in [27, 28] {
            let mut image = fixture(bits);
            image
                .dynamic
                .iter_mut()
                .find(|(kind, _)| *kind == tag)
                .unwrap()
                .1 -= 1;
            let source = image.bytes.len();
            assert!(matches!(
                Lifecycle::load(&mut image, source),
                Err(FormatError::InvalidRecordSize)
            ));
        }
    }
}

#[test]
fn observed_runtime_versions_do_not_cross_pointer_widths() {
    let strlen = qa_platform::native::runtime::FUNCTIONS
        .iter()
        .find(|f| f.name == b"strlen")
        .unwrap();
    assert!(strlen.supports_version(4, b"GLIBC_2.0"));
    assert!(strlen.supports_version(8, b"GLIBC_2.2.5"));
    assert!(!strlen.supports_version(8, b"GLIBC_2.0"));
    assert!(!strlen.supports_version(4, b"GLIBC_2.2.5"));
    assert!(!strlen.supports_version(4, b"GLIBC_2.99"));
}
