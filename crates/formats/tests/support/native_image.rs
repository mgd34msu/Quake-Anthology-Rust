//! Shared native image constructors for reader and binder checks.
pub(super) fn put(file: &mut [u8], at: usize, value: u64, width: usize) {
    file[at..at + width].copy_from_slice(&value.to_le_bytes()[..width]);
}

pub(super) fn elf_fixture(bits: usize, writable: bool) -> Vec<u8> {
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

pub(super) fn elf_segment(
    file: &mut [u8],
    bits: usize,
    index: usize,
    kind: u32,
    offset: u64,
    address: u64,
    file_bytes: u64,
    memory_bytes: u64,
) {
    let header = if bits == 64 { 64 } else { 52 };
    let stride = if bits == 64 { 56 } else { 32 };
    let at = header + index * stride;
    file[at..at + stride].fill(0);
    put(file, at, u64::from(kind), 4);
    for (field, value, width) in [
        (if bits == 64 { 4 } else { 24 }, 6, 4),
        (if bits == 64 { 8 } else { 4 }, offset, bits / 8),
        (if bits == 64 { 16 } else { 8 }, address, bits / 8),
        (if bits == 64 { 32 } else { 16 }, file_bytes, bits / 8),
        (if bits == 64 { 40 } else { 20 }, memory_bytes, bits / 8),
        (if bits == 64 { 48 } else { 28 }, 1, bits / 8),
    ] {
        put(file, at + field, value, width);
    }
}

pub(super) fn elf_dynamic_fixture(bits: usize) -> Vec<u8> {
    let mut file = elf_fixture(bits, true);
    put(&mut file, if bits == 64 { 56 } else { 44 }, 5, 2);
    elf_segment(&mut file, bits, 0, 1, 0, 0, 512, 512);
    elf_segment(&mut file, bits, 1, 1, 0x1180, 0x3180, 0x180, 0x200);
    elf_segment(
        &mut file,
        bits,
        2,
        2,
        0x1180,
        0x3180,
        (bits / 8 * 8) as u64,
        (bits / 8 * 8) as u64,
    );
    elf_segment(&mut file, bits, 3, 7, 0x12c0, 0x32c0, 4, 16);
    elf_segment(&mut file, bits, 4, 0x6474e552, 0x1180, 0x3180, 0, 0x100);
    for (i, (tag, value)) in [(1, 1), (5, 0x3280), (10, 14), (0, 0)]
        .into_iter()
        .enumerate()
    {
        put(&mut file, 0x1180 + i * (bits / 8) * 2, tag, bits / 8);
        put(
            &mut file,
            0x1180 + (i * 2 + 1) * (bits / 8),
            value,
            bits / 8,
        );
    }
    file[0x1280..0x128e].copy_from_slice(b"\0libnative.so\0");
    file[0x12c0..0x12c4].copy_from_slice(&[1, 2, 3, 4]);
    file
}

pub(super) const ELF_SYMBOL_NAMES: &[u8] =
    b"\0vmMain\0helper\0external\0COUNT\0tls\0VER_1\0libc.so.6\0dllEntry\0";

pub(super) fn elf_name(label: &[u8]) -> u64 {
    ELF_SYMBOL_NAMES
        .windows(label.len())
        .position(|s| s == label)
        .unwrap() as u64
}

pub(super) fn elf_dynamic(file: &mut [u8], bits: usize, tags: &[(u64, u64)]) {
    let bytes = ((tags.len() + 1) * 2 * (bits / 8)) as u64;
    elf_segment(file, bits, 2, 2, 0x1c00, 0x3c00, bytes, bytes);
    for (i, (tag, value)) in tags.iter().copied().chain([(0, 0)]).enumerate() {
        put(file, 0x1c00 + i * (bits / 8) * 2, tag, bits / 8);
        put(file, 0x1c00 + (i * 2 + 1) * (bits / 8), value, bits / 8);
    }
}

pub(super) fn elf_symbol_fixture(bits: usize) -> (Vec<u8>, Vec<(u64, u64)>) {
    let mut file = elf_dynamic_fixture(bits);
    file.resize(0x3000, 0);
    elf_segment(&mut file, bits, 1, 1, 0x1180, 0x3180, 0x1000, 0x1000);
    file[0x1280..0x1280 + ELF_SYMBOL_NAMES.len()].copy_from_slice(ELF_SYMBOL_NAMES);
    let stride = if bits == 64 { 24 } else { 16 };
    file[0x1400..0x1400 + stride * 6].fill(0);
    for (i, label, value, bytes, info, section) in [
        (1, b"vmMain".as_slice(), 0x33c0, 48, 0x12, 1),
        (2, b"helper".as_slice(), 0x3300, 8, 0x02, 1),
        (3, b"external".as_slice(), 0, 0, 0x22, 0),
        (4, b"COUNT".as_slice(), 3, 4, 0x11, 0xfff1),
        (5, b"tls".as_slice(), 4, 4, 0x16, 1),
    ] {
        let at = 0x1400 + i * stride;
        put(&mut file, at, elf_name(label), 4);
        put(&mut file, at + if bits == 64 { 4 } else { 12 }, info, 1);
        put(&mut file, at + if bits == 64 { 6 } else { 14 }, section, 2);
        put(
            &mut file,
            at + if bits == 64 { 8 } else { 4 },
            value,
            bits / 8,
        );
        put(
            &mut file,
            at + if bits == 64 { 16 } else { 8 },
            bytes,
            bits / 8,
        );
    }
    let tags = vec![
        (5, 0x3280),
        (10, ELF_SYMBOL_NAMES.len() as u64),
        (6, 0x3400),
        (11, stride as u64),
        (39, (stride * 6) as u64),
    ];
    elf_dynamic(&mut file, bits, &tags);
    (file, tags)
}

pub(super) fn elf_relocation(
    file: &mut [u8],
    bits: usize,
    at: usize,
    address: u64,
    kind: u32,
    symbol: u32,
    addend: Option<i64>,
) {
    let width = bits / 8;
    let info = if bits == 64 {
        (u64::from(symbol) << 32) | u64::from(kind)
    } else {
        (u64::from(symbol) << 8) | u64::from(kind)
    };
    put(file, at, address, width);
    put(file, at + width, info, width);
    if let Some(addend) = addend {
        put(file, at + width * 2, addend as u64, width);
    }
}

pub(super) fn pe_rva(file: &mut [u8], at: usize, value: u64, width: usize) {
    put(file, 512 + at - 4096, value, width);
}
pub(super) fn pe_text(file: &mut [u8], at: usize, value: &[u8]) {
    let offset = 512 + at - 4096;
    file[offset..offset + value.len()].copy_from_slice(value);
}
pub(super) fn pe_fixture(bits: usize) -> Vec<u8> {
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
pub(super) fn pe_directory(f: &mut [u8], bits: usize, index: usize, at: u64, length: u64) {
    let d = 152 + if bits == 64 { 112 } else { 96 };
    put(f, d + index * 8, at, 4);
    put(f, d + index * 8 + 4, length, 4);
}

pub(super) fn pe_export_fixture(bits: usize) -> Vec<u8> {
    let mut f = pe_fixture(bits);
    pe_directory(&mut f, bits, 0, 0x1100, 0x80);
    for (offset, value) in [
        (16, 7),
        (20, 1),
        (24, 2),
        (28, 0x1140),
        (32, 0x1150),
        (36, 0x1160),
    ] {
        pe_rva(&mut f, 0x1100 + offset, value, 4);
    }
    pe_rva(&mut f, 0x1140, 0x1080, 4);
    pe_rva(&mut f, 0x1150, 0x1190, 4);
    pe_rva(&mut f, 0x1154, 0x11a0, 4);
    pe_text(&mut f, 0x1190, b"GetGameAPI\0");
    pe_text(&mut f, 0x11a0, b"Alias\0");
    f
}
