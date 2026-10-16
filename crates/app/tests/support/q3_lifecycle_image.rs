//! Native Q3 files shared by initial-load and reload lifecycle fixtures.
#![allow(dead_code)]
use qa_formats::program::native::Encoding;
#[path = "../../../formats/tests/support/native_image.rs"]
mod native_image;
use native_image::*;

pub(super) fn function_file(encoding: Encoding, code: &[u8]) -> Vec<u8> {
    match encoding {
        Encoding::Elf => {
            let (mut file, tags) = elf_symbol_fixture(64);
            put(&mut file, 64 + 56 + 4, 7, 4);
            file[0x13c0..0x13c0 + code.len()].copy_from_slice(code);
            // Export dllEntry, storing its System V syscall pointer in the
            // shared slot used by vmMain. Its return is void in the native ABI.
            put(&mut file, 0x1400 + 48, elf_name(b"dllEntry"), 4);
            put(&mut file, 0x1400 + 48 + 4, 0x12, 1);
            file[0x1ac0..0x1ac8].fill(0);
            file[0x1300..0x1308].copy_from_slice(&[0x48, 0x89, 0x3d, 0xb9, 0x07, 0, 0, 0xc3]);
            elf_dynamic(&mut file, 64, &tags);
            file
        }
        Encoding::Pe => {
            let mut file = pe_export_fixture(64);
            file[640..640 + code.len()].copy_from_slice(code);
            pe_rva(&mut file, 0x1100 + 20, 2, 4);
            pe_rva(&mut file, 0x1100 + 24, 3, 4);
            pe_rva(&mut file, 0x1144, 0x10c0, 4);
            pe_rva(&mut file, 0x1158, 0x11b0, 4);
            pe_rva(&mut file, 0x1164, 1, 2);
            pe_text(&mut file, 0x11b0, b"dllEntry\0");
            file[704..712].copy_from_slice(&[0x48, 0x89, 0x0d, 0xb9, 0x06, 0, 0, 0xc3]);
            file
        }
    }
}

pub(super) fn append_trace_digit(file: &mut [u8], at: usize, trace: usize, digit: u8) {
    let mut code = vec![0x8b, 0x05];
    code.extend((trace as i32 - (at + 6) as i32).to_le_bytes());
    code.extend([0x6b, 0xc0, 10, 0x83, 0xc0, digit, 0x89, 0x05]);
    code.extend((trace as i32 - (at + 18) as i32).to_le_bytes());
    code.push(0xc3);
    file[at..at + code.len()].copy_from_slice(&code);
}

pub(super) fn elf_lifecycle_file() -> Vec<u8> {
    let mut file = function_file(Encoding::Elf, &[0xc3]);
    put(&mut file, 64 + 3 * 56, 0, 4); // no TLS dependency
    put(&mut file, 0x1400 + 24 + 8, 0x3600, 8); // game export code
    file[0x1a80..0x1a90].fill(0); // trace and initialization count
    for (at, digit) in [
        (0x1700, 1),
        (0x1740, 2),
        (0x1780, 3),
        (0x17c0, 6),
        (0x1800, 7),
        (0x1840, 8),
    ] {
        append_trace_digit(&mut file, at, 0x1a80, digit);
    }
    // The final native callback traps unless the whole reached order is exact.
    // This checks finalizer order without exposing raw module memory to app.
    file[0x1852..0x185c].copy_from_slice(&[0x3d, 0xa8, 0x61, 0xbc, 0, 0x74, 2, 0x0f, 0x0b, 0xc3]);
    // DT_INIT sees a native argc and separate valid, empty argv/envp vectors.
    let checks = [
        0x85, 0xff, 0x75, 16, 0x48, 0x83, 0x3e, 0, 0x75, 10, 0x48, 0x83, 0x3a, 0, 0x75, 4, 0xeb, 4,
        0x0f, 0x0b, 0x0f, 0x0b,
    ];
    file[0x1700..0x1700 + checks.len()].copy_from_slice(&checks);
    append_trace_digit(&mut file, 0x1700 + checks.len(), 0x1a80, 1);
    // dllEntry stores the shared syscall callback and appends digit 4.
    file[0x1300..0x1307].copy_from_slice(&[0x48, 0x89, 0x3d, 0xb9, 7, 0, 0]);
    append_trace_digit(&mut file, 0x1307, 0x1a80, 4);
    // Game command 1 is shutdown; all other commands return the reached trace.
    file[0x1600..0x1605].copy_from_slice(&[0x83, 0xff, 1, 0x74, 7]);
    file[0x1605..0x1607].copy_from_slice(&[0x8b, 0x05]);
    put(&mut file, 0x1607, (0x1a80 - 0x160b) as u64, 4);
    file[0x160b] = 0xc3;
    append_trace_digit(&mut file, 0x160c, 0x1a80, 5);
    for (at, value) in [
        (0x1a00, 0x3740),
        (0x1a08, 0x3780),
        (0x1a20, 0x37c0),
        (0x1a28, 0x3800),
    ] {
        put(&mut file, at, value, 8);
    }
    for (index, (at, value)) in [
        (0x3a00, 0x3740),
        (0x3a08, 0x3780),
        (0x3a20, 0x37c0),
        (0x3a28, 0x3800),
    ]
    .into_iter()
    .enumerate()
    {
        elf_relocation(&mut file, 64, 0x1b40 + index * 24, at, 8, 0, Some(value));
    }
    let (_, mut tags) = elf_symbol_fixture(64);
    tags.extend([
        (12, 0x3700),
        (25, 0x3a00),
        (27, 16),
        (26, 0x3a20),
        (28, 16),
        (13, 0x3840),
        (7, 0x3b40),
        (8, 96),
        (9, 24),
        (32, u64::MAX),
        (33, 8),
    ]); // ignored shared-object preinit
    elf_dynamic(&mut file, 64, &tags);
    file
}
