//! Port of `src/compat/qc/program.ts` (Quake `pr_comp.h`/`pr_edict.c`, id
//! Software, GPL-2.0-or-later): QuakeC `progs.dat` loader, opcode table,
//! definitions, and function metadata.
//!
//! Local mirrors (owned elsewhere, defined here minimally): [`QuakeCApi`]
//! mirrors the `QuakeCApiIdentity` contract (`src/contracts/execution.ts`);
//! the crate-wide save codec has its own `GameApi` spelling in
//! `crate::checkpoint`. Content digests reuse
//! [`qa_core`]-style `sha256:` text via `crate::core::contracts::ContentDigest`.

use std::collections::HashMap;

use qa_core::binary::BinaryReader;

use crate::core::contracts::ContentDigest;
use crate::error::GuestError;

/// QuakeC API identity: NetQuake (`progs.dat` CRC 5927) or QuakeWorld
/// (`qwprogs.dat` CRC 54730). Both are program version 6.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum QuakeCApi {
    /// NetQuake QuakeC API.
    Netquake,
    /// QuakeWorld QuakeC API.
    Quakeworld,
}

impl QuakeCApi {
    /// API kind label (`q1-netquake` / `q1-quakeworld`).
    #[must_use]
    pub const fn kind(self) -> &'static str {
        match self {
            Self::Netquake => "q1-netquake",
            Self::Quakeworld => "q1-quakeworld",
        }
    }

    /// Pinned program version (always 6).
    #[must_use]
    pub const fn program_version(self) -> i32 {
        6
    }

    /// System layout CRC pinned by the donor loader.
    #[must_use]
    pub const fn system_crc(self) -> i32 {
        match self {
            Self::Netquake => 5927,
            Self::Quakeworld => 54730,
        }
    }

    /// Whether this is the QuakeWorld API (negative engine-string IDs).
    #[must_use]
    pub const fn is_quakeworld(self) -> bool {
        matches!(self, Self::Quakeworld)
    }
}

/// QuakeC statement opcode. Discriminants match `pr_comp.h`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u16)]
pub enum QcOpcode {
    /// End of function.
    Done = 0,
    /// Float multiply.
    MulF = 1,
    /// Vector dot product.
    MulV = 2,
    /// Float-vector scale.
    MulFV = 3,
    /// Vector-float scale.
    MulVF = 4,
    /// Float divide.
    DivF = 5,
    /// Float add.
    AddF = 6,
    /// Vector add.
    AddV = 7,
    /// Float subtract.
    SubF = 8,
    /// Vector subtract.
    SubV = 9,
    /// Float equality.
    EqF = 10,
    /// Vector equality.
    EqV = 11,
    /// String equality.
    EqS = 12,
    /// Entity equality.
    EqE = 13,
    /// Function equality.
    EqFn = 14,
    /// Float inequality.
    NeF = 15,
    /// Vector inequality.
    NeV = 16,
    /// String inequality (byte comparison).
    NeS = 17,
    /// Entity inequality.
    NeE = 18,
    /// Function inequality.
    NeFn = 19,
    /// Less than or equal.
    Le = 20,
    /// Greater than or equal.
    Ge = 21,
    /// Less than.
    Lt = 22,
    /// Greater than.
    Gt = 23,
    /// Load float field.
    LoadF = 24,
    /// Load vector field.
    LoadV = 25,
    /// Load string field.
    LoadS = 26,
    /// Load entity field.
    LoadEnt = 27,
    /// Load field field.
    LoadFld = 28,
    /// Load function field.
    LoadFn = 29,
    /// Address of entity field.
    Address = 30,
    /// Store float.
    StoreF = 31,
    /// Store vector.
    StoreV = 32,
    /// Store string.
    StoreS = 33,
    /// Store entity.
    StoreEnt = 34,
    /// Store field.
    StoreFld = 35,
    /// Store function.
    StoreFn = 36,
    /// Store through float pointer.
    StorePF = 37,
    /// Store through vector pointer.
    StorePV = 38,
    /// Store through string pointer.
    StorePS = 39,
    /// Store through entity pointer.
    StorePEnt = 40,
    /// Store through field pointer.
    StorePFld = 41,
    /// Store through function pointer.
    StorePFn = 42,
    /// Return from function.
    Return = 43,
    /// Logical not (float).
    NotF = 44,
    /// Logical not (vector).
    NotV = 45,
    /// Logical not (string).
    NotS = 46,
    /// Logical not (entity).
    NotEnt = 47,
    /// Logical not (function).
    NotFn = 48,
    /// Branch if nonzero.
    If = 49,
    /// Branch if zero.
    IfNot = 50,
    /// Call with 0 arguments.
    Call0 = 51,
    /// Call with 1 argument.
    Call1 = 52,
    /// Call with 2 arguments.
    Call2 = 53,
    /// Call with 3 arguments.
    Call3 = 54,
    /// Call with 4 arguments.
    Call4 = 55,
    /// Call with 5 arguments.
    Call5 = 56,
    /// Call with 6 arguments.
    Call6 = 57,
    /// Call with 7 arguments.
    Call7 = 58,
    /// Call with 8 arguments.
    Call8 = 59,
    /// Set frame/think/nextthink on self.
    State = 60,
    /// Unconditional branch.
    Goto = 61,
    /// Logical and.
    And = 62,
    /// Logical or.
    Or = 63,
    /// Bitwise and.
    BitAnd = 64,
    /// Bitwise or.
    BitOr = 65,
}

impl QcOpcode {
    /// Highest valid opcode discriminant.
    pub const MAX: u16 = QcOpcode::BitOr as u16;

    /// Argument count for `Call0`..=`Call8`, else `None`.
    #[must_use]
    pub const fn call_arguments(self) -> Option<u8> {
        let raw = self as u16;
        if raw >= QcOpcode::Call0 as u16 && raw <= QcOpcode::Call8 as u16 {
            Some((raw - QcOpcode::Call0 as u16) as u8)
        } else {
            None
        }
    }
}

impl TryFrom<u16> for QcOpcode {
    type Error = u16;

    fn try_from(value: u16) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Self::Done),
            1 => Ok(Self::MulF),
            2 => Ok(Self::MulV),
            3 => Ok(Self::MulFV),
            4 => Ok(Self::MulVF),
            5 => Ok(Self::DivF),
            6 => Ok(Self::AddF),
            7 => Ok(Self::AddV),
            8 => Ok(Self::SubF),
            9 => Ok(Self::SubV),
            10 => Ok(Self::EqF),
            11 => Ok(Self::EqV),
            12 => Ok(Self::EqS),
            13 => Ok(Self::EqE),
            14 => Ok(Self::EqFn),
            15 => Ok(Self::NeF),
            16 => Ok(Self::NeV),
            17 => Ok(Self::NeS),
            18 => Ok(Self::NeE),
            19 => Ok(Self::NeFn),
            20 => Ok(Self::Le),
            21 => Ok(Self::Ge),
            22 => Ok(Self::Lt),
            23 => Ok(Self::Gt),
            24 => Ok(Self::LoadF),
            25 => Ok(Self::LoadV),
            26 => Ok(Self::LoadS),
            27 => Ok(Self::LoadEnt),
            28 => Ok(Self::LoadFld),
            29 => Ok(Self::LoadFn),
            30 => Ok(Self::Address),
            31 => Ok(Self::StoreF),
            32 => Ok(Self::StoreV),
            33 => Ok(Self::StoreS),
            34 => Ok(Self::StoreEnt),
            35 => Ok(Self::StoreFld),
            36 => Ok(Self::StoreFn),
            37 => Ok(Self::StorePF),
            38 => Ok(Self::StorePV),
            39 => Ok(Self::StorePS),
            40 => Ok(Self::StorePEnt),
            41 => Ok(Self::StorePFld),
            42 => Ok(Self::StorePFn),
            43 => Ok(Self::Return),
            44 => Ok(Self::NotF),
            45 => Ok(Self::NotV),
            46 => Ok(Self::NotS),
            47 => Ok(Self::NotEnt),
            48 => Ok(Self::NotFn),
            49 => Ok(Self::If),
            50 => Ok(Self::IfNot),
            51 => Ok(Self::Call0),
            52 => Ok(Self::Call1),
            53 => Ok(Self::Call2),
            54 => Ok(Self::Call3),
            55 => Ok(Self::Call4),
            56 => Ok(Self::Call5),
            57 => Ok(Self::Call6),
            58 => Ok(Self::Call7),
            59 => Ok(Self::Call8),
            60 => Ok(Self::State),
            61 => Ok(Self::Goto),
            62 => Ok(Self::And),
            63 => Ok(Self::Or),
            64 => Ok(Self::BitAnd),
            65 => Ok(Self::BitOr),
            _ => Err(value),
        }
    }
}

/// QuakeC value type. `Opaque` covers compiler-only struct definitions kept
/// by FTE-generated rerelease programs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum QcValueType {
    /// No value.
    Void,
    /// String reference.
    String,
    /// Float word.
    Float,
    /// Three float words.
    Vector,
    /// Entity reference.
    Entity,
    /// Field offset.
    Field,
    /// Function index.
    Function,
    /// Entity pointer.
    Pointer,
    /// Unknown native type; words stay addressable, text saves unavailable.
    Opaque,
}

impl QcValueType {
    /// Map a raw definition type to its value type.
    #[must_use]
    pub const fn from_raw(raw: u16) -> Self {
        match raw {
            0 => Self::Void,
            1 => Self::String,
            2 => Self::Float,
            3 => Self::Vector,
            4 => Self::Entity,
            5 => Self::Field,
            6 => Self::Function,
            7 => Self::Pointer,
            _ => Self::Opaque,
        }
    }

    /// Word count occupied by one value of this type.
    #[must_use]
    pub const fn words(self) -> usize {
        match self {
            Self::Vector => 3,
            _ => 1,
        }
    }
}

/// One global or entity-field definition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QcDefinition {
    /// Value type.
    pub value_type: QcValueType,
    /// Raw native type (`raw & 0x7fff`).
    pub native_type: u16,
    /// Whether text saves persist this definition.
    pub save: bool,
    /// Word offset within globals or entity variables.
    pub offset: usize,
    /// Definition name.
    pub name: String,
}

/// One compiled statement: opcode plus three word operands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QcStatement {
    /// Operation.
    pub opcode: QcOpcode,
    /// First operand word.
    pub a: u16,
    /// Second operand word.
    pub b: u16,
    /// Third operand word.
    pub c: u16,
}

/// One compiled function or builtin declaration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QcFunction {
    /// Function index (0 is the null function).
    pub index: usize,
    /// First statement; negative `-n` is numbered builtin `n`, zero with
    /// zero locals/parameters is a named builtin.
    pub first_statement: i32,
    /// First parameter/local word.
    pub parameter_start: usize,
    /// Local word count saved across calls.
    pub local_words: usize,
    /// Function name.
    pub name: String,
    /// Defining file.
    pub file: String,
    /// Parameter word sizes (1 or 3).
    pub parameter_sizes: Vec<u8>,
    /// Whether this is a named (rerelease `ex_*`) builtin.
    pub named_builtin: bool,
}

impl QcFunction {
    /// Whether this function runs interpreted statements.
    #[must_use]
    pub fn is_interpreted(&self) -> bool {
        !self.named_builtin && self.first_statement > 0
    }
}

/// A loaded QuakeC program: statements, definitions, functions, strings,
/// and initial global words.
#[derive(Debug, Clone)]
pub struct QcProgram {
    /// Load label (`progs.dat` by default).
    pub source: String,
    /// API identity selected by the system CRC.
    pub api: QuakeCApi,
    /// Compiled statements.
    pub statements: Vec<QcStatement>,
    /// Global definitions.
    pub globals: Vec<QcDefinition>,
    /// Entity-field definitions.
    pub fields: Vec<QcDefinition>,
    /// Functions (index 0 is the null function).
    pub functions: Vec<QcFunction>,
    /// String table bytes.
    pub strings: Vec<u8>,
    /// Initial global words.
    pub initial_globals: Vec<u8>,
    /// Entity variable word count.
    pub entity_field_words: usize,
    /// CRC-16 of the loaded bytes.
    pub checksum: u16,
    /// SHA-256 content digest of the loaded bytes.
    pub digest: ContentDigest,
    globals_by_name: HashMap<String, usize>,
    fields_by_name: HashMap<String, usize>,
    functions_by_name: HashMap<String, usize>,
}

impl QcProgram {
    /// Look up a function by index. Index 0 is invalid.
    pub fn function_at(&self, index: usize) -> Result<&QcFunction, GuestError> {
        if index == 0 {
            return Err(self.program_error(format!("invalid function {index}")));
        }
        self.functions
            .get(index)
            .ok_or_else(|| self.program_error(format!("invalid function {index}")))
    }

    /// Look up a function by name. The null function never matches.
    pub fn function_named(&self, name: &str) -> Result<&QcFunction, GuestError> {
        let function = self
            .functions_by_name
            .get(name)
            .and_then(|index| self.functions.get(*index));
        match function {
            Some(function) if function.index != 0 => Ok(function),
            _ => Err(self.program_error(format!("missing function {name}"))),
        }
    }

    /// Look up a global definition by name.
    #[must_use]
    pub fn global_named(&self, name: &str) -> Option<&QcDefinition> {
        self.globals_by_name
            .get(name)
            .and_then(|index| self.globals.get(*index))
    }

    /// Look up an entity-field definition by name.
    #[must_use]
    pub fn field_named(&self, name: &str) -> Option<&QcDefinition> {
        self.fields_by_name.get(name).and_then(|index| self.fields.get(*index))
    }

    /// Exclusive end statement of `function` (next function's entry, or
    /// the end of the statement table).
    #[must_use]
    pub fn function_end(&self, function: &QcFunction) -> usize {
        self.functions
            .iter()
            .filter(|other| other.first_statement > function.first_statement)
            .map(|other| other.first_statement.max(0) as usize)
            .min()
            .unwrap_or(self.statements.len())
    }

    fn program_error(&self, detail: String) -> GuestError {
        GuestError::bad_image("progs.dat", format!("{}: {detail}", self.source))
    }
}

/// Build a synthetic program for headless fixture tests (no real game
/// module). First-wins name maps match the loader.
#[cfg(test)]
pub fn test_program(
    statements: Vec<QcStatement>,
    globals: Vec<QcDefinition>,
    fields: Vec<QcDefinition>,
    functions: Vec<QcFunction>,
    strings: Vec<u8>,
    initial_globals: Vec<u8>,
    entity_field_words: usize,
) -> QcProgram {
    let mut globals_by_name = HashMap::new();
    for (index, definition) in globals.iter().enumerate() {
        globals_by_name.entry(definition.name.clone()).or_insert(index);
    }
    let mut fields_by_name = HashMap::new();
    for (index, definition) in fields.iter().enumerate() {
        fields_by_name.entry(definition.name.clone()).or_insert(index);
    }
    let mut functions_by_name = HashMap::new();
    for (index, function) in functions.iter().enumerate() {
        functions_by_name.entry(function.name.clone()).or_insert(index);
    }
    QcProgram {
        source: "test.dat".to_string(),
        api: QuakeCApi::Netquake,
        statements,
        globals,
        fields,
        functions,
        strings,
        initial_globals,
        entity_field_words,
        checksum: 0,
        digest: ContentDigest::new("sha256", &"0".repeat(64)),
        globals_by_name,
        fields_by_name,
        functions_by_name,
    }
}

/// Interpret a branch operand as a signed 16-bit displacement.
#[must_use]
pub const fn signed_qc_branch(word: u16) -> i32 {
    (word as i16) as i32
}

/// Read a NUL-terminated byte string at `offset` (bytes map 1:1 to chars).
pub fn qc_byte_string(bytes: &[u8], offset: i64) -> Result<String, GuestError> {
    if offset < 0 {
        return Err(GuestError::bad_image(
            "progs.dat",
            format!("invalid string offset {offset}"),
        ));
    }
    let start = offset as usize;
    if start >= bytes.len() {
        return Err(GuestError::bad_image(
            "progs.dat",
            format!("invalid string offset {offset}"),
        ));
    }
    let mut text = String::new();
    for byte in &bytes[start..] {
        if *byte == 0 {
            return Ok(text);
        }
        text.push(char::from(*byte));
    }
    Err(GuestError::bad_image(
        "progs.dat",
        format!("unterminated string at {offset}"),
    ))
}

fn crc16(bytes: &[u8]) -> u16 {
    let mut crc: u16 = 0xffff;
    for byte in bytes {
        crc ^= u16::from(*byte) << 8;
        for _ in 0..8 {
            if crc & 0x8000 != 0 {
                crc = (crc << 1) ^ 0x1021;
            } else {
                crc <<= 1;
            }
        }
    }
    crc
}

/// Self-contained SHA-256 (FIPS 180-4); the workspace has no hash crate.
fn sha256(message: &[u8]) -> [u8; 32] {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5, 0xd807aa98,
        0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786,
        0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8,
        0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13,
        0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819,
        0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a,
        0x5b9cca4f, 0x682e6ff3, 0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
        0xc67178f2,
    ];
    let mut state: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19,
    ];
    let bit_len = (message.len() as u64).wrapping_mul(8);
    let mut padded = Vec::with_capacity(message.len().next_multiple_of(64) + 64);
    padded.extend_from_slice(message);
    padded.push(0x80);
    while padded.len() % 64 != 56 {
        padded.push(0);
    }
    padded.extend_from_slice(&bit_len.to_be_bytes());
    for block in padded.chunks_exact(64) {
        let mut schedule = [0u32; 64];
        for (slot, chunk) in schedule.iter_mut().zip(block.chunks_exact(4)).take(16) {
            *slot = u32::from_be_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
        }
        for index in 16..64 {
            let s0 = schedule[index - 15].rotate_right(7)
                ^ schedule[index - 15].rotate_right(18)
                ^ (schedule[index - 15] >> 3);
            let s1 = schedule[index - 2].rotate_right(17)
                ^ schedule[index - 2].rotate_right(19)
                ^ (schedule[index - 2] >> 10);
            schedule[index] = schedule[index - 16]
                .wrapping_add(s0)
                .wrapping_add(schedule[index - 7])
                .wrapping_add(s1);
        }
        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut h] = state;
        for index in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ ((!e) & g);
            let t1 = h
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[index])
                .wrapping_add(schedule[index]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(maj);
            h = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b;
            b = a;
            a = t1.wrapping_add(t2);
        }
        for (slot, delta) in state.iter_mut().zip([a, b, c, d, e, f, g, h]) {
            *slot = slot.wrapping_add(delta);
        }
    }
    let mut digest = [0u8; 32];
    for (slot, word) in digest.chunks_exact_mut(4).zip(state) {
        slot.copy_from_slice(&word.to_be_bytes());
    }
    digest
}

fn hex_lower(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut text = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        text.push(HEX[(byte >> 4) as usize] as char);
        text.push(HEX[(byte & 0xf) as usize] as char);
    }
    text
}

fn image_error(source: &str, detail: impl Into<String>) -> GuestError {
    GuestError::bad_image("progs.dat", format!("{}: {}", source, detail.into()))
}

fn map_reader(source: &str, error: qa_core::binary::BinaryError) -> GuestError {
    image_error(source, error.to_string())
}

/// Load a version-6 `progs.dat` image, selecting the API by system CRC.
pub fn load_qc_program(bytes: &[u8], expected_api: Option<QuakeCApi>, source: &str) -> Result<QcProgram, GuestError> {
    let mut reader = BinaryReader::new(bytes, source);
    let version = reader.i32().map_err(|error| map_reader(source, error))?;
    let crc = reader.i32().map_err(|error| map_reader(source, error))?;
    if version != 6 {
        return Err(image_error(source, format!("unsupported program version {version}")));
    }
    let api = match crc {
        5927 => QuakeCApi::Netquake,
        54730 => QuakeCApi::Quakeworld,
        _ => {
            return Err(image_error(source, format!("unknown system layout CRC {crc}")));
        }
    };
    if let Some(expected) = expected_api {
        if expected != api {
            return Err(image_error(
                source,
                format!("expected {}, found {}", expected.kind(), api.kind()),
            ));
        }
    }
    let mut section = |stride: usize| -> Result<BinaryReader<'_>, GuestError> {
        let offset = reader.i32().map_err(|error| map_reader(source, error))?;
        let count = reader.i32().map_err(|error| map_reader(source, error))?;
        if count < 0 {
            return Err(image_error(source, "negative section count"));
        }
        if offset < 0 {
            return Err(image_error(source, "negative section offset"));
        }
        let length = (count as usize)
            .checked_mul(stride)
            .ok_or_else(|| image_error(source, "section too large"))?;
        reader
            .records(offset as usize, length, stride)
            .map_err(|error| map_reader(source, error))
    };
    let mut statements_reader = section(8)?;
    let mut globals_reader = section(8)?;
    let mut fields_reader = section(8)?;
    let mut functions_reader = section(36)?;
    let mut strings_reader = section(1)?;
    let mut values_reader = section(4)?;
    let entity_field_words = reader.i32().map_err(|error| map_reader(source, error))?;
    if entity_field_words <= 0 || entity_field_words > 65536 {
        return Err(image_error(
            source,
            format!("invalid entity field count {entity_field_words}"),
        ));
    }
    let entity_field_words = entity_field_words as usize;
    let strings_length = strings_reader.length();
    let strings = strings_reader
        .bytes(strings_length)
        .map_err(|error| map_reader(source, error))?;
    if strings.first() != Some(&0) {
        return Err(image_error(source, "string zero must be empty"));
    }
    let values_length = values_reader.length();
    let initial_globals = values_reader
        .bytes(values_length)
        .map_err(|error| map_reader(source, error))?;
    if initial_globals.len() < 28 * 4 {
        return Err(image_error(source, "missing reserved global words"));
    }
    let mut definitions = |input: &mut BinaryReader<'_>, field: bool| -> Result<Vec<QcDefinition>, GuestError> {
        let mut result = Vec::new();
        while input.remaining() > 0 {
            let raw = input.u16().map_err(|error| map_reader(source, error))?;
            let offset = input.u16().map_err(|error| map_reader(source, error))?;
            let name_offset = input.i32().map_err(|error| map_reader(source, error))?;
            let name = qc_byte_string(&strings, i64::from(name_offset))
                .map_err(|_| image_error(source, format!("invalid definition name at {name_offset}")))?;
            let value_type = QcValueType::from_raw(raw & 0x7fff);
            if field && raw & 0x8000 != 0 {
                return Err(image_error(source, format!("invalid definition {name}")));
            }
            let limit = if field {
                entity_field_words
            } else {
                initial_globals.len() / 4
            };
            if usize::from(offset) + value_type.words() > limit {
                return Err(image_error(source, format!("definition {name} exceeds its memory")));
            }
            result.push(QcDefinition {
                value_type,
                native_type: raw & 0x7fff,
                save: raw & 0x8000 != 0,
                offset: usize::from(offset),
                name,
            });
        }
        Ok(result)
    };
    let globals = definitions(&mut globals_reader, false)?;
    let fields = definitions(&mut fields_reader, true)?;
    let mut statements = Vec::new();
    while statements_reader.remaining() > 0 {
        let opcode = statements_reader.u16().map_err(|error| map_reader(source, error))?;
        let opcode =
            QcOpcode::try_from(opcode).map_err(|raw| image_error(source, format!("unsupported opcode {raw}")))?;
        let a = statements_reader.u16().map_err(|error| map_reader(source, error))?;
        let b = statements_reader.u16().map_err(|error| map_reader(source, error))?;
        let c = statements_reader.u16().map_err(|error| map_reader(source, error))?;
        statements.push(QcStatement { opcode, a, b, c });
    }
    let mut functions = Vec::new();
    while functions_reader.remaining() > 0 {
        let first_statement = functions_reader.i32().map_err(|error| map_reader(source, error))?;
        let parameter_start = functions_reader.i32().map_err(|error| map_reader(source, error))?;
        let local_words = functions_reader.i32().map_err(|error| map_reader(source, error))?;
        let _profile = functions_reader.i32().map_err(|error| map_reader(source, error))?;
        let name_offset = functions_reader.i32().map_err(|error| map_reader(source, error))?;
        let file_offset = functions_reader.i32().map_err(|error| map_reader(source, error))?;
        let name = qc_byte_string(&strings, i64::from(name_offset))
            .map_err(|_| image_error(source, "invalid function name"))?;
        let file = qc_byte_string(&strings, i64::from(file_offset))
            .map_err(|_| image_error(source, "invalid function file"))?;
        let count = functions_reader.i32().map_err(|error| map_reader(source, error))?;
        let sizes = functions_reader.bytes(8).map_err(|error| map_reader(source, error))?;
        if count < 0
            || count > 8
            || local_words < 0
            || parameter_start < 0
            || parameter_start as usize + local_words as usize > initial_globals.len() / 4
        {
            return Err(image_error(
                source,
                format!("invalid function locals/parameters for {name}"),
            ));
        }
        let mut parameter_sizes = Vec::new();
        for size in sizes.iter().take(count as usize) {
            if first_statement > 0 && *size != 1 && *size != 3 {
                return Err(image_error(source, format!("invalid parameter size for {name}")));
            }
            parameter_sizes.push(*size);
        }
        let parameter_words: usize = parameter_sizes.iter().map(|size| *size as usize).sum();
        if first_statement >= statements.len() as i32
            || first_statement > 0 && parameter_start as usize + parameter_words > initial_globals.len() / 4
        {
            return Err(image_error(source, format!("invalid entry point for {name}")));
        }
        let index = functions.len();
        functions.push(QcFunction {
            index,
            first_statement,
            parameter_start: parameter_start as usize,
            local_words: local_words as usize,
            name,
            file,
            parameter_sizes,
            named_builtin: index > 0 && first_statement == 0 && parameter_start == 0 && local_words == 0,
        });
    }
    if functions.is_empty() || statements.is_empty() {
        return Err(image_error(source, "empty function/statement table"));
    }
    let mut globals_by_name = HashMap::new();
    for (index, definition) in globals.iter().enumerate() {
        globals_by_name.entry(definition.name.clone()).or_insert(index);
    }
    let mut fields_by_name = HashMap::new();
    for (index, definition) in fields.iter().enumerate() {
        fields_by_name.entry(definition.name.clone()).or_insert(index);
    }
    let mut functions_by_name = HashMap::new();
    for (index, function) in functions.iter().enumerate() {
        functions_by_name.entry(function.name.clone()).or_insert(index);
    }
    Ok(QcProgram {
        source: source.to_string(),
        api,
        statements,
        globals,
        fields,
        functions,
        strings,
        initial_globals,
        entity_field_words,
        checksum: crc16(bytes),
        digest: ContentDigest::new("sha256", &hex_lower(&sha256(bytes))),
        globals_by_name,
        fields_by_name,
        functions_by_name,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_image() -> Vec<u8> {
        // Minimal version-6 image: 2 statements, 1 global, 1 field,
        // 2 functions, small string table, 28 reserved words.
        let mut bytes = Vec::new();
        let push_i32 = |bytes: &mut Vec<u8>, value: i32| bytes.extend_from_slice(&value.to_le_bytes());
        let header_len = 4 + 6 * 8 + 4;
        let statements = vec![
            (QcOpcode::Done as u16, 0u16, 0u16, 0u16),
            (QcOpcode::Return as u16, 1u16, 0u16, 0u16),
        ];
        let strings = b"\0main\0file.qc\0x\0".to_vec();
        // Offsets: "main"=1, "file.qc"=6, "x"=14.
        let globals = vec![(2u16, 28u16, 14i32)];
        let fields = vec![(2u16, 0u16, 14i32)];
        let functions = vec![
            (0i32, 0i32, 0i32, 0i32, 1i32, 6i32, 0i32, [0u8; 8]),
            (1i32, 28i32, 1i32, 0i32, 1i32, 6i32, 0i32, [0u8; 8]),
        ];
        let values = vec![0u8; 30 * 4];
        let mut offset = header_len as i32;
        let mut sections = Vec::new();
        let blobs: Vec<Vec<u8>> = vec![
            {
                let mut blob = Vec::new();
                for (op, a, b, c) in &statements {
                    for word in [*op, *a, *b, *c] {
                        blob.extend_from_slice(&word.to_le_bytes());
                    }
                }
                blob
            },
            {
                let mut blob = Vec::new();
                for (raw, at, name) in &globals {
                    blob.extend_from_slice(&raw.to_le_bytes());
                    blob.extend_from_slice(&at.to_le_bytes());
                    blob.extend_from_slice(&name.to_le_bytes());
                }
                blob
            },
            {
                let mut blob = Vec::new();
                for (raw, at, name) in &fields {
                    blob.extend_from_slice(&raw.to_le_bytes());
                    blob.extend_from_slice(&at.to_le_bytes());
                    blob.extend_from_slice(&name.to_le_bytes());
                }
                blob
            },
            {
                let mut blob = Vec::new();
                for (first, params, locals, profile, name, file, count, sizes) in &functions {
                    for word in [*first, *params, *locals, *profile, *name, *file, *count] {
                        blob.extend_from_slice(&word.to_le_bytes());
                    }
                    blob.extend_from_slice(sizes);
                }
                blob
            },
            strings.clone(),
            values.clone(),
        ];
        let counts = [2i32, 1, 1, 2, strings.len() as i32, 30];
        for (blob, count) in blobs.iter().zip(counts) {
            sections.push((offset, count));
            offset += blob.len() as i32;
        }
        push_i32(&mut bytes, 6);
        push_i32(&mut bytes, 5927);
        for (at, count) in sections {
            push_i32(&mut bytes, at);
            push_i32(&mut bytes, count);
        }
        push_i32(&mut bytes, 4);
        for blob in &blobs {
            bytes.extend_from_slice(blob);
        }
        bytes
    }

    #[test]
    fn loads_minimal_netquake_program() {
        let bytes = test_image();
        let program = load_qc_program(&bytes, None, "progs.dat").unwrap();
        assert_eq!(program.api, QuakeCApi::Netquake);
        assert_eq!(program.statements.len(), 2);
        assert_eq!(program.functions.len(), 2);
        assert_eq!(program.entity_field_words, 4);
        assert_eq!(program.function_named("main").unwrap().index, 1);
        assert!(program.function_at(0).is_err());
        assert_eq!(program.global_named("x").unwrap().offset, 28);
        assert_eq!(program.field_named("x").unwrap().offset, 0);
        assert!(program.digest.value.len() == 64);
        assert_eq!(program.digest.algorithm, "sha256");
        assert_eq!(program.function_end(&program.functions[1].clone()), 2);
    }

    #[test]
    fn rejects_wrong_version_and_crc() {
        let mut bytes = test_image();
        bytes[0] = 7;
        assert!(load_qc_program(&bytes, None, "progs.dat").is_err());
        let mut bytes = test_image();
        bytes[4] = 1;
        bytes[5] = 2;
        assert!(load_qc_program(&bytes, None, "progs.dat").is_err());
        let bytes = test_image();
        assert!(load_qc_program(&bytes, Some(QuakeCApi::Quakeworld), "progs.dat").is_err());
    }

    #[test]
    fn rejects_bad_opcode_and_truncation() {
        let mut bytes = test_image();
        // First statement opcode lives right after the 56-byte header.
        bytes[56] = 0xff;
        bytes[57] = 0xff;
        assert!(load_qc_program(&bytes, None, "progs.dat").is_err());
        assert!(load_qc_program(&bytes[..10], None, "progs.dat").is_err());
    }

    #[test]
    fn branch_is_signed_16_bit() {
        assert_eq!(signed_qc_branch(1), 1);
        assert_eq!(signed_qc_branch(0xffff), -1);
        assert_eq!(signed_qc_branch(0x8000), -32768);
    }

    #[test]
    fn byte_strings_map_bytes_one_to_one() {
        let bytes = b"\0hi\xff\0";
        assert_eq!(qc_byte_string(bytes, 0).unwrap(), "");
        assert_eq!(qc_byte_string(bytes, 1).unwrap(), "hiÿ");
        assert!(qc_byte_string(bytes, -1).is_err());
        assert!(qc_byte_string(bytes, 99).is_err());
        assert!(qc_byte_string(b"abc", 0).is_err());
    }

    #[test]
    fn sha256_matches_fips_vector() {
        assert_eq!(
            hex_lower(&sha256(b"abc")),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn opcode_call_counts() {
        assert_eq!(QcOpcode::Call0.call_arguments(), Some(0));
        assert_eq!(QcOpcode::Call8.call_arguments(), Some(8));
        assert_eq!(QcOpcode::Done.call_arguments(), None);
        assert!(QcOpcode::try_from(66).is_err());
    }
}
