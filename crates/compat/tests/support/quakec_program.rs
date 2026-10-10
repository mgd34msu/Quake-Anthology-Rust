use qa_formats::program::quakec::Opcode;

pub fn program(operations: &[(Opcode, i16, i16, i16)], function_entries: &[i32]) -> Vec<u8> {
    let mut statements = vec![0; 8];
    for &(op, a, b, c) in operations {
        statements.extend((op as u16).to_le_bytes());
        for n in [a, b, c] {
            statements.extend(n.to_le_bytes());
        }
    }
    let strings = b"\0self\0time\0nextthink\0frame\0think\0";
    let mut globaldefs = Vec::new();
    let mut fielddefs = Vec::new();
    for (kind, offset, name) in [(4u16, 34u16, 1i32), (2, 35, 6)] {
        globaldefs.extend(kind.to_le_bytes());
        globaldefs.extend(offset.to_le_bytes());
        globaldefs.extend(name.to_le_bytes());
    }
    for (kind, offset, name) in [(2u16, 0u16, 11i32), (2, 1, 21), (6, 2, 27)] {
        fielddefs.extend(kind.to_le_bytes());
        fielddefs.extend(offset.to_le_bytes());
        fielddefs.extend(name.to_le_bytes());
    }
    let mut functions = Vec::new();
    for &first in function_entries {
        for n in [first, 40, 8, 0, 0, 0, 0] {
            functions.extend(n.to_le_bytes());
        }
        functions.extend([0; 8]);
    }
    let mut globals = vec![0; 64 * 4];
    globals[34 * 4..35 * 4].copy_from_slice(&48u32.to_le_bytes());
    globals[35 * 4..36 * 4].copy_from_slice(&3.0f32.to_bits().to_le_bytes());
    let lumps = [
        &statements[..],
        &globaldefs[..],
        &fielddefs[..],
        &functions[..],
        &strings[..],
        &globals[..],
    ];
    let strides = [8, 8, 8, 36, 1, 4];
    let mut header = vec![6i32, 5927];
    let mut bytes = vec![0; 60];
    for (lump, stride) in lumps.into_iter().zip(strides) {
        bytes.resize((bytes.len() + 3) & !3, 0);
        header.extend([bytes.len() as i32, (lump.len() / stride) as i32]);
        bytes.extend(lump);
    }
    header.push(8);
    for (i, n) in header.into_iter().enumerate() {
        bytes[i * 4..i * 4 + 4].copy_from_slice(&n.to_le_bytes());
    }
    bytes
}
