use qa_formats::program::quakec::Opcode;

pub fn program(operations: &[(Opcode, i16, i16, i16)], function_entries: &[i32]) -> Vec<u8> {
    program_with_vectors(operations, function_entries, &[])
}
pub fn program_with_vectors(
    operations: &[(Opcode, i16, i16, i16)],
    function_entries: &[i32],
    vectors: &[(u16, &[u8])],
) -> Vec<u8> {
    let mut globals = vec![(4, 34, &b"self"[..]), (2, 35, &b"time"[..])];
    globals.extend(vectors.iter().map(|&(offset, name)| (3, offset, name)));
    program_with_definitions(
        operations,
        function_entries,
        &globals,
        &[(2, 0, b"nextthink"), (2, 1, b"frame"), (6, 2, b"think")],
        8,
        64,
    )
}

pub fn program_with_definitions(
    operations: &[(Opcode, i16, i16, i16)],
    function_entries: &[i32],
    globals: &[(u16, u16, &[u8])],
    fields: &[(u16, u16, &[u8])],
    entity_fields: usize,
    global_words: usize,
) -> Vec<u8> {
    let mut statements = vec![0; 8];
    for &(op, a, b, c) in operations {
        statements.extend((op as u16).to_le_bytes());
        for n in [a, b, c] {
            statements.extend(n.to_le_bytes());
        }
    }
    let mut strings = vec![0];
    let mut globaldefs = Vec::new();
    let mut fielddefs = Vec::new();
    for &(kind, offset, name) in globals {
        globaldefs.extend(kind.to_le_bytes());
        globaldefs.extend(offset.to_le_bytes());
        globaldefs.extend((strings.len() as i32).to_le_bytes());
        strings.extend(name);
        strings.push(0);
    }
    for &(kind, offset, name) in fields {
        fielddefs.extend(kind.to_le_bytes());
        fielddefs.extend(offset.to_le_bytes());
        fielddefs.extend((strings.len() as i32).to_le_bytes());
        strings.extend(name);
        strings.push(0);
    }
    let mut functions = Vec::new();
    for &first in function_entries {
        for n in [first, 40, 8, 0, 0, 0, 0] {
            functions.extend(n.to_le_bytes());
        }
        functions.extend([0; 8]);
    }
    let mut globals = vec![0; global_words * 4];
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
    header.push(entity_fields as i32);
    for (i, n) in header.into_iter().enumerate() {
        bytes[i * 4..i * 4 + 4].copy_from_slice(&n.to_le_bytes());
    }
    bytes
}
