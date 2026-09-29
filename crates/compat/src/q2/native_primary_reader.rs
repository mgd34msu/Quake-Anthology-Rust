//! Port of `src/compat/q2/native-primary-reader.ts`.
//! Bridges native-primary profile JSON into typed offsets, fields, tests and call signatures.

use qa_guest::core::contracts::{
    GuestCallSignature, GuestFieldLayout, GuestLayout, GuestRegister, GuestStorage, GuestValueLayout,
    NativeAbi, NativeCallAbi,
};

/// Classic (Xatrix PE) native artifact digest shared by every primary profile.
pub const CLASSIC_DIGEST: &str =
    "sha256:8187df3fd5b4d435d8227434d3351aad2b47e546236403e52adcd4d275810c45";
/// Rerelease (retail PE) native artifact digest shared by every primary profile.
pub const RETAIL_DIGEST: &str =
    "sha256:045d49c53722d9b922caf14f168dd28a97d4c514a6e443a3140560f8668baccd";

/// Minimal JSON value for native-primary profile documents.
#[derive(Debug, Clone, PartialEq)]
pub enum JsonValue {
    /// JSON null.
    Null,
    /// JSON boolean.
    Bool(bool),
    /// JSON integer literal.
    Int(i64),
    /// JSON floating-point literal.
    Float(f64),
    /// JSON string.
    Str(String),
    /// JSON array.
    Array(Vec<JsonValue>),
    /// JSON object with source order preserved.
    Object(Vec<(String, JsonValue)>),
}

/// Parse a JSON document. Returns the root value or a byte-oriented error.
pub fn parse_json(text: &str) -> Result<JsonValue, String> {
    let mut parser = JsonParser {
        bytes: text.as_bytes(),
        cursor: 0,
    };
    parser.skip_space();
    let value = parser.value()?;
    parser.skip_space();
    if parser.cursor != parser.bytes.len() {
        return Err(format!("trailing bytes at offset {}", parser.cursor));
    }
    Ok(value)
}

struct JsonParser<'a> {
    bytes: &'a [u8],
    cursor: usize,
}

impl JsonParser<'_> {
    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.cursor).copied()
    }

    fn skip_space(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\t' | b'\n' | b'\r')) {
            self.cursor += 1;
        }
    }

    fn expect(&mut self, byte: u8, what: &str) -> Result<(), String> {
        if self.peek() == Some(byte) {
            self.cursor += 1;
            Ok(())
        } else {
            Err(format!("expected {what} at offset {}", self.cursor))
        }
    }

    fn value(&mut self) -> Result<JsonValue, String> {
        match self.peek() {
            Some(b'n') => self.literal("null", JsonValue::Null),
            Some(b't') => self.literal("true", JsonValue::Bool(true)),
            Some(b'f') => self.literal("false", JsonValue::Bool(false)),
            Some(b'"') => Ok(JsonValue::Str(self.string()?)),
            Some(b'[') => self.array(),
            Some(b'{') => self.object(),
            Some(b'-') | Some(b'0'..=b'9') => self.number(),
            _ => Err(format!("unexpected byte at offset {}", self.cursor)),
        }
    }

    fn literal(&mut self, word: &str, value: JsonValue) -> Result<JsonValue, String> {
        if self.bytes[self.cursor..].starts_with(word.as_bytes()) {
            self.cursor += word.len();
            Ok(value)
        } else {
            Err(format!("expected {word} at offset {}", self.cursor))
        }
    }

    fn string(&mut self) -> Result<String, String> {
        self.expect(b'"', "string")?;
        let mut out = String::new();
        loop {
            let byte = self
                .peek()
                .ok_or_else(|| format!("unterminated string at offset {}", self.cursor))?;
            self.cursor += 1;
            match byte {
                b'"' => return Ok(out),
                b'\\' => {
                    let escape = self
                        .peek()
                        .ok_or_else(|| format!("unterminated escape at offset {}", self.cursor))?;
                    self.cursor += 1;
                    match escape {
                        b'"' => out.push('"'),
                        b'\\' => out.push('\\'),
                        b'/' => out.push('/'),
                        b'b' => out.push('\u{0008}'),
                        b'f' => out.push('\u{000C}'),
                        b'n' => out.push('\n'),
                        b'r' => out.push('\r'),
                        b't' => out.push('\t'),
                        b'u' => out.push(self.unicode()?),
                        _ => return Err(format!("invalid escape at offset {}", self.cursor)),
                    }
                }
                0x00..=0x1F => return Err(format!("control byte in string at offset {}", self.cursor)),
                _ => {
                    let start = self.cursor - 1;
                    let mut length = 1;
                    while start + length <= self.bytes.len()
                        && core::str::from_utf8(&self.bytes[start..start + length]).is_err()
                    {
                        length += 1;
                        if length > 4 {
                            return Err(format!("invalid UTF-8 at offset {start}"));
                        }
                    }
                    let text = core::str::from_utf8(&self.bytes[start..start + length])
                        .map_err(|_| format!("invalid UTF-8 at offset {start}"))?;
                    out.push_str(text);
                    self.cursor = start + length;
                }
            }
        }
    }

    fn unicode(&mut self) -> Result<char, String> {
        let code = self.hex4()?;
        if (0xd800..0xdc00).contains(&code) {
            if self.bytes.get(self.cursor..self.cursor + 2) == Some(b"\\u".as_slice()) {
                self.cursor += 2;
                let low = self.hex4()?;
                if (0xdc00..0xe000).contains(&low) {
                    let scalar = 0x1_0000 + ((code - 0xd800) << 10) + (low - 0xdc00);
                    return char::from_u32(scalar)
                        .ok_or_else(|| format!("invalid scalar at offset {}", self.cursor));
                }
            }
            return Err(format!("lone surrogate at offset {}", self.cursor));
        }
        char::from_u32(code).ok_or_else(|| format!("invalid scalar at offset {}", self.cursor))
    }

    fn hex4(&mut self) -> Result<u32, String> {
        if self.cursor + 4 > self.bytes.len() {
            return Err(format!("truncated escape at offset {}", self.cursor));
        }
        let mut value = 0u32;
        for byte in &self.bytes[self.cursor..self.cursor + 4] {
            value = value * 16
                + (*byte as char)
                    .to_digit(16)
                    .ok_or_else(|| format!("invalid hex escape at offset {}", self.cursor))?;
        }
        self.cursor += 4;
        Ok(value)
    }

    fn array(&mut self) -> Result<JsonValue, String> {
        self.expect(b'[', "array")?;
        let mut items = Vec::new();
        self.skip_space();
        if self.peek() == Some(b']') {
            self.cursor += 1;
            return Ok(JsonValue::Array(items));
        }
        loop {
            self.skip_space();
            items.push(self.value()?);
            self.skip_space();
            match self.peek() {
                Some(b',') => {
                    self.cursor += 1;
                }
                Some(b']') => {
                    self.cursor += 1;
                    return Ok(JsonValue::Array(items));
                }
                _ => return Err(format!("expected , or ] at offset {}", self.cursor)),
            }
        }
    }

    fn object(&mut self) -> Result<JsonValue, String> {
        self.expect(b'{', "object")?;
        let mut fields = Vec::new();
        self.skip_space();
        if self.peek() == Some(b'}') {
            self.cursor += 1;
            return Ok(JsonValue::Object(fields));
        }
        loop {
            self.skip_space();
            if self.peek() != Some(b'"') {
                return Err(format!("expected field name at offset {}", self.cursor));
            }
            let name = self.string()?;
            self.skip_space();
            self.expect(b':', ":")?;
            self.skip_space();
            let value = self.value()?;
            fields.push((name, value));
            self.skip_space();
            match self.peek() {
                Some(b',') => {
                    self.cursor += 1;
                }
                Some(b'}') => {
                    self.cursor += 1;
                    return Ok(JsonValue::Object(fields));
                }
                _ => return Err(format!("expected , or }} at offset {}", self.cursor)),
            }
        }
    }

    fn number(&mut self) -> Result<JsonValue, String> {
        let start = self.cursor;
        if self.peek() == Some(b'-') {
            self.cursor += 1;
        }
        let digits = |parser: &mut Self| {
            let from = parser.cursor;
            while matches!(parser.peek(), Some(b'0'..=b'9')) {
                parser.cursor += 1;
            }
            parser.cursor - from
        };
        if digits(self) == 0 {
            return Err(format!("invalid number at offset {start}"));
        }
        let mut float = false;
        if self.peek() == Some(b'.') {
            float = true;
            self.cursor += 1;
            if digits(self) == 0 {
                return Err(format!("invalid number at offset {start}"));
            }
        }
        if matches!(self.peek(), Some(b'e' | b'E')) {
            float = true;
            self.cursor += 1;
            if matches!(self.peek(), Some(b'+' | b'-')) {
                self.cursor += 1;
            }
            if digits(self) == 0 {
                return Err(format!("invalid number at offset {start}"));
            }
        }
        let text = core::str::from_utf8(&self.bytes[start..self.cursor])
            .map_err(|_| format!("invalid number at offset {start}"))?;
        if float {
            text.parse::<f64>()
                .map(JsonValue::Float)
                .map_err(|_| format!("invalid number at offset {start}"))
        } else {
            text.parse::<i64>()
                .map(JsonValue::Int)
                .map_err(|_| format!("number exceeds int64 at offset {start}"))
        }
    }
}

/// Schema reader over a [`JsonValue`] subtree with donor-style path failures.
///
/// Like the donor `SaveReader`, schema violations panic with the document path;
/// only JSON syntax errors are `Result`-typed.
pub struct Reader<'a> {
    value: &'a JsonValue,
    path: String,
}

impl<'a> Reader<'a> {
    /// Borrow a document root.
    #[must_use]
    pub fn root(value: &'a JsonValue) -> Self {
        Self {
            value,
            path: "$".to_string(),
        }
    }

    /// Current document path (for diagnostics).
    #[must_use]
    pub fn path(&self) -> &str {
        &self.path
    }

    /// Abort with a path-qualified schema failure. Mirrors `SaveReader.fail`.
    pub fn fail(&self, message: &str) -> ! {
        panic!("{}: {message}", self.path)
    }

    fn child(&self, value: &'a JsonValue, segment: &str) -> Reader<'a> {
        Reader {
            value,
            path: format!("{}{segment}", self.path),
        }
    }

    /// Borrow a required object field.
    #[must_use]
    pub fn field(&self, name: &str) -> Reader<'a> {
        match self.value {
            JsonValue::Object(fields) => match fields.iter().find(|(key, _)| key == name) {
                Some((_, value)) => self.child(value, &format!(".{name}")),
                None => self.fail(&format!("missing field {name}")),
            },
            _ => self.fail(&format!("missing field {name}")),
        }
    }

    /// Whether an object field is present.
    #[must_use]
    pub fn has(&self, name: &str) -> bool {
        matches!(self.value, JsonValue::Object(fields) if fields.iter().any(|(key, _)| key == name))
    }

    /// Whether this node is JSON null.
    #[must_use]
    pub fn is_null(&self) -> bool {
        matches!(self.value, JsonValue::Null)
    }

    /// Read a string.
    pub fn string(&self) -> String {
        match self.value {
            JsonValue::Str(text) => text.clone(),
            _ => self.fail("expected string"),
        }
    }

    /// Read a boolean.
    pub fn boolean(&self) -> bool {
        match self.value {
            JsonValue::Bool(value) => *value,
            _ => self.fail("expected boolean"),
        }
    }

    /// Read an integer at least `min`. Integral floats are accepted.
    pub fn integer(&self, min: i64) -> i64 {
        let value = match self.value {
            JsonValue::Int(value) => *value,
            JsonValue::Float(value) if value.fract() == 0.0 && *value >= -9.0e15 && *value <= 9.0e15 => {
                *value as i64
            }
            _ => self.fail("expected integer"),
        };
        if value < min {
            self.fail(&format!("expected integer at least {min}"));
        }
        value
    }

    /// Read an integer range-checked into `u32`.
    pub fn integer_u32(&self, min: u32) -> u32 {
        let value = self.integer(0);
        if value < i64::from(min) || value > i64::from(u32::MAX) {
            self.fail("expected uint32");
        }
        value as u32
    }

    /// Read a finite number.
    pub fn finite(&self) -> f64 {
        let value = match self.value {
            JsonValue::Int(value) => *value as f64,
            JsonValue::Float(value) => *value,
            _ => self.fail("expected number"),
        };
        if !value.is_finite() {
            self.fail("expected finite number");
        }
        value
    }

    /// Match one of the allowed labels and return its index.
    pub fn choice_index(&self, options: &[&str]) -> usize {
        let value = self.string();
        match options.iter().position(|option| *option == value) {
            Some(index) => index,
            None => self.fail(&format!("expected one of {}", options.join("|"))),
        }
    }

    /// Match one of the allowed integers and return it.
    pub fn choice_int(&self, options: &[i64]) -> i64 {
        let value = self.integer(i64::MIN);
        if options.contains(&value) {
            value
        } else {
            self.fail("unexpected integer choice")
        }
    }

    /// Require an exact string literal.
    pub fn literal_str(&self, expected: &str) {
        if self.string() != expected {
            self.fail(&format!("expected literal {expected}"));
        }
    }

    /// Require an exact boolean literal.
    pub fn literal_bool(&self, expected: bool) {
        if self.boolean() != expected {
            self.fail("unexpected boolean literal");
        }
    }

    /// Require an exact integer literal.
    pub fn literal_int(&self, expected: i64) {
        if self.integer(i64::MIN) != expected {
            self.fail(&format!("expected literal {expected}"));
        }
    }

    /// Read an optional subtree; JSON null maps to `None`.
    pub fn nullable<T>(&self, read: impl FnOnce(&Reader) -> T) -> Option<T> {
        if self.is_null() {
            None
        } else {
            Some(read(self))
        }
    }

    /// Read every array element with path-qualified children.
    pub fn list<T>(&self, mut read: impl FnMut(&Reader) -> T) -> Vec<T> {
        match self.value {
            JsonValue::Array(items) => items
                .iter()
                .enumerate()
                .map(|(index, item)| read(&self.child(item, &format!("[{index}]"))))
                .collect(),
            _ => self.fail("expected array"),
        }
    }
}

/// Scalar encoding of one native field or call slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NativeScalar {
    /// Signed 8-bit.
    Int8,
    /// Unsigned 8-bit.
    Uint8,
    /// Signed 16-bit.
    Int16,
    /// Unsigned 16-bit.
    Uint16,
    /// Signed 32-bit.
    Int32,
    /// Unsigned 32-bit.
    Uint32,
    /// Signed 64-bit.
    Int64,
    /// Unsigned 64-bit.
    Uint64,
    /// 32-bit float.
    Float32,
    /// 64-bit float.
    Float64,
}

impl NativeScalar {
    /// Profile label.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Int8 => "int8",
            Self::Uint8 => "uint8",
            Self::Int16 => "int16",
            Self::Uint16 => "uint16",
            Self::Int32 => "int32",
            Self::Uint32 => "uint32",
            Self::Int64 => "int64",
            Self::Uint64 => "uint64",
            Self::Float32 => "float32",
            Self::Float64 => "float64",
        }
    }

    /// Storage width in bytes.
    #[must_use]
    pub const fn width(self) -> usize {
        match self {
            Self::Int8 | Self::Uint8 => 1,
            Self::Int16 | Self::Uint16 => 2,
            Self::Int32 | Self::Uint32 | Self::Float32 => 4,
            Self::Int64 | Self::Uint64 | Self::Float64 => 8,
        }
    }

    /// Decode a profile label.
    #[must_use]
    pub const fn from_label(label: &str) -> Option<Self> {
        match label {
            "int8" => Some(Self::Int8),
            "uint8" => Some(Self::Uint8),
            "int16" => Some(Self::Int16),
            "uint16" => Some(Self::Uint16),
            "int32" => Some(Self::Int32),
            "uint32" => Some(Self::Uint32),
            "int64" => Some(Self::Int64),
            "uint64" => Some(Self::Uint64),
            "float32" => Some(Self::Float32),
            "float64" => Some(Self::Float64),
            _ => None,
        }
    }

    /// Guest value storage for call layouts.
    #[must_use]
    pub const fn storage(self) -> GuestStorage {
        match self {
            Self::Int8 => GuestStorage::Int8,
            Self::Uint8 => GuestStorage::Uint8,
            Self::Int16 => GuestStorage::Int16,
            Self::Uint16 => GuestStorage::Uint16,
            Self::Int32 => GuestStorage::Int32,
            Self::Uint32 => GuestStorage::Uint32,
            Self::Int64 => GuestStorage::Int64,
            Self::Uint64 => GuestStorage::Uint64,
            Self::Float32 => GuestStorage::Float32,
            Self::Float64 => GuestStorage::Float64,
        }
    }
}

/// Source record that owns a native field.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RecordKind {
    /// Entity (edict) record.
    Entity,
    /// Client record.
    Client,
    /// Loaded image bytes.
    Image,
}

impl RecordKind {
    /// Profile label.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Entity => "entity",
            Self::Client => "client",
            Self::Image => "image",
        }
    }

    /// Decode a profile label.
    #[must_use]
    pub const fn from_label(label: &str) -> Option<Self> {
        match label {
            "entity" => Some(Self::Entity),
            "client" => Some(Self::Client),
            "image" => Some(Self::Image),
            _ => None,
        }
    }
}

/// One typed native field: record plus byte offset plus encoding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct NativeItemField {
    /// Owning record.
    pub record: RecordKind,
    /// Byte offset within the record.
    pub offset: u32,
    /// Scalar encoding.
    pub encoding: NativeScalar,
}

/// Expected image address behind a pointer test.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PointerExpectation {
    /// Image-relative address.
    pub rva: u32,
    /// Pointer-chase displacements applied in order.
    pub indirections: Vec<u32>,
}

/// Scalar comparison operator.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TestComparison {
    /// Exact equality.
    Equals,
    /// Less-than-or-equal.
    AtMost,
}

/// Acceptance test over live native state.
#[derive(Debug, Clone, PartialEq)]
pub enum NativeItemTest {
    /// Masked scalar comparison.
    Scalar {
        /// Tested field.
        field: NativeItemField,
        /// Optional AND mask applied before comparison.
        mask: Option<u32>,
        /// Comparison operator.
        comparison: TestComparison,
        /// Expected value.
        value: f64,
    },
    /// Pointer identity after an optional chase.
    Pointer {
        /// Tested pointer field record.
        record: RecordKind,
        /// Tested pointer field offset.
        offset: u32,
        /// Expected image address, or null.
        value: Option<PointerExpectation>,
    },
}

/// Image-relative address with an optional pointer chase.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeModAddress {
    /// Image-relative address.
    pub rva: u32,
    /// Pointer-chase displacements applied in order.
    pub indirections: Vec<u32>,
}

/// Executable address range with an entry and a join.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct NativeRegion {
    /// Region entry RVA.
    pub entry: u32,
    /// Region join RVA.
    pub join: u32,
}

/// Read an image-relative offset bounded by the 32-bit address space.
pub fn native_offset(reader: &Reader) -> u32 {
    let value = reader.integer(0);
    if value > i64::from(u32::MAX) {
        reader.fail("native offset exceeds PE address space");
    }
    value as u32
}

/// Read a non-empty executable region.
pub fn native_region(reader: &Reader) -> NativeRegion {
    let entry = native_offset(&reader.field("entry"));
    let join = native_offset(&reader.field("join"));
    if entry == join {
        reader.fail("source region must contain instructions");
    }
    NativeRegion { entry, join }
}

/// Read a scalar encoding label.
pub fn native_scalar(reader: &Reader) -> NativeScalar {
    let label = reader.string();
    NativeScalar::from_label(&label).unwrap_or_else(|| reader.fail("unknown scalar encoding"))
}

/// Read a general-purpose register name.
pub fn native_register(reader: &Reader) -> GuestRegister {
    match reader.string().as_str() {
        "rax" => GuestRegister::Rax,
        "rcx" => GuestRegister::Rcx,
        "rdx" => GuestRegister::Rdx,
        "rbx" => GuestRegister::Rbx,
        "rsp" => GuestRegister::Rsp,
        "rbp" => GuestRegister::Rbp,
        "rsi" => GuestRegister::Rsi,
        "rdi" => GuestRegister::Rdi,
        "r8" => GuestRegister::R8,
        "r9" => GuestRegister::R9,
        "r10" => GuestRegister::R10,
        "r11" => GuestRegister::R11,
        "r12" => GuestRegister::R12,
        "r13" => GuestRegister::R13,
        "r14" => GuestRegister::R14,
        "r15" => GuestRegister::R15,
        _ => reader.fail("unknown register"),
    }
}

/// Read a typed native field.
pub fn native_field(reader: &Reader) -> NativeItemField {
    let record = reader.field("record").string();
    let record = RecordKind::from_label(&record).unwrap_or_else(|| reader.fail("unknown record"));
    NativeItemField {
        record,
        offset: native_offset(&reader.field("offset")),
        encoding: native_scalar(&reader.field("encoding")),
    }
}

/// Read a scalar or pointer acceptance test.
pub fn native_test(reader: &Reader) -> NativeItemTest {
    if reader.field("kind").choice_index(&["scalar", "pointer"]) == 1 {
        let field = reader.field("field");
        let record = field.field("record").string();
        let record =
            RecordKind::from_label(&record).unwrap_or_else(|| reader.fail("unknown record"));
        return NativeItemTest::Pointer {
            record,
            offset: native_offset(&field.field("offset")),
            value: reader.field("value").nullable(|value| PointerExpectation {
                rva: native_offset(&value.field("rva")),
                indirections: value.field("indirections").list(native_offset),
            }),
        };
    }
    let comparison = match reader.field("comparison").choice_index(&["equals", "at-most"]) {
        0 => TestComparison::Equals,
        _ => TestComparison::AtMost,
    };
    NativeItemTest::Scalar {
        field: native_field(&reader.field("field")),
        mask: reader.field("mask").nullable(native_offset),
        comparison,
        value: reader.field("value").finite(),
    }
}

/// ABI kind label for a [`NativeAbi`] image.
#[must_use]
pub fn abi_kind_label(abi: NativeAbi) -> &'static str {
    NativeCallAbi::of(abi).kind()
}

/// Check a declared ABI against the selected executable ABI.
pub fn native_abi(reader: &Reader, expected: NativeAbi) -> NativeAbi {
    reader.field("kind").literal_str(abi_kind_label(expected));
    reader.field("image").literal_str(expected.image());
    reader.field("call").literal_str(expected.call());
    reader
        .field("pointerBytes")
        .literal_int(expected.pointer_bytes() as i64);
    expected
}

fn scalar_layout(reader: &Reader) -> GuestValueLayout {
    reader.field("kind").literal_str("scalar");
    let storage = reader.field("storage").string();
    if storage == "pointer" {
        GuestValueLayout::Scalar(GuestStorage::Pointer)
    } else {
        let scalar =
            NativeScalar::from_label(&storage).unwrap_or_else(|| reader.fail("unknown storage"));
        GuestValueLayout::Scalar(scalar.storage())
    }
}

/// Read a guest call signature against the selected executable ABI.
pub fn native_signature(reader: &Reader, abi: NativeAbi) -> GuestCallSignature {
    let checked = native_abi(&reader.field("abi"), abi);
    let parameters = reader.field("parameters").list(scalar_layout);
    let result = reader.field("result");
    let result = if matches!(&result.value, JsonValue::Str(text) if text == "void") {
        None
    } else {
        Some(scalar_layout(&result))
    };
    reader.field("variadic").literal_bool(false);
    GuestCallSignature {
        abi: NativeCallAbi::of(checked),
        parameters,
        result,
        variadic: false,
    }
}

/// Read a guest record layout (`readLayout` equivalent).
pub fn read_guest_layout(reader: &Reader) -> GuestLayout {
    let id = reader.field("id").string();
    let byte_length = reader.field("byteLength").integer(0) as usize;
    let alignment = reader.field("alignment").integer(1) as usize;
    let pointer_bytes = reader.field("pointerBytes").integer(0);
    if pointer_bytes != 4 && pointer_bytes != 8 {
        reader.fail("layout pointer width must be 4 or 8");
    }
    let fields = reader.field("fields").list(|field| {
        let storage = field.field("storage").string();
        let storage = if storage == "pointer" {
            GuestStorage::Pointer
        } else {
            NativeScalar::from_label(&storage)
                .unwrap_or_else(|| field.fail("unknown field storage"))
                .storage()
        };
        GuestFieldLayout {
            name: field.field("name").string(),
            byte_offset: field.field("byteOffset").integer(0) as usize,
            storage,
            count: field.field("count").integer(1) as usize,
        }
    });
    GuestLayout::new(
        &id,
        byte_length,
        alignment,
        pointer_bytes as usize,
        fields,
    )
}

/// Read a `namespace:name` item reference.
pub fn namespaced(reader: &Reader) -> String {
    let value = reader.string();
    let mut parts = value.split(':');
    match (parts.next(), parts.next(), parts.next()) {
        (Some(namespace), Some(name), None) if !namespace.is_empty() && !name.is_empty() => value,
        _ => reader.fail("expected namespace:name reference"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_nested_profile_json() {
        let value = parse_json(
            r#"{"name":"q2:x","count":48,"ratio":1.5,"tags":["a","b"],"nested":{"entry":4464,"join":null},"ok":true}"#,
        )
        .expect("valid json");
        let root = Reader::root(&value);
        assert_eq!(root.field("name").string(), "q2:x");
        assert_eq!(root.field("count").integer(1), 48);
        assert_eq!(root.field("ratio").finite(), 1.5);
        assert!(root.field("ok").boolean());
        assert_eq!(root.field("tags").list(|item| item.string()), vec!["a", "b"]);
        assert_eq!(native_offset(&root.field("nested").field("entry")), 4464);
        assert!(root.field("nested").field("join").is_null());
        assert!(root.field("nested").has("entry"));
        assert!(!root.has("missing"));
    }

    #[test]
    fn rejects_trailing_garbage() {
        assert!(parse_json(r#"{"a":1} x"#).is_err());
        assert!(parse_json(r#"{"a":}"#).is_err());
        assert!(parse_json("\"\\ud800\"").is_err());
    }

    #[test]
    fn reads_scalar_and_pointer_tests() {
        let value = parse_json(
            r#"{"scalar":{"kind":"scalar","field":{"record":"client","offset":3528,"encoding":"int32"},"mask":null,"comparison":"equals","value":0},
            "pointer":{"kind":"pointer","field":{"record":"entity","offset":84},"value":{"rva":19144,"indirections":[4,8]}}}"#,
        )
        .expect("valid json");
        let root = Reader::root(&value);
        assert_eq!(
            native_test(&root.field("scalar")),
            NativeItemTest::Scalar {
                field: NativeItemField {
                    record: RecordKind::Client,
                    offset: 3528,
                    encoding: NativeScalar::Int32,
                },
                mask: None,
                comparison: TestComparison::Equals,
                value: 0.0,
            }
        );
        assert_eq!(
            native_test(&root.field("pointer")),
            NativeItemTest::Pointer {
                record: RecordKind::Entity,
                offset: 84,
                value: Some(PointerExpectation {
                    rva: 19144,
                    indirections: vec![4, 8],
                }),
            }
        );
    }

    #[test]
    fn reads_signature_and_layout() {
        let value = parse_json(
            r#"{"signature":{"abi":{"kind":"windows-i386","image":"pe32","call":"cdecl","pointerBytes":4},
            "parameters":[{"kind":"scalar","storage":"pointer"}],"result":"void","variadic":false},
            "layout":{"id":"q2:test","byteLength":16,"alignment":4,"pointerBytes":4,
            "fields":[{"name":"pers.weapon","byteOffset":8,"storage":"pointer","count":1}]}}"#,
        )
        .expect("valid json");
        let root = Reader::root(&value);
        let signature = native_signature(&root.field("signature"), NativeAbi::WindowsI386);
        assert_eq!(signature.abi, NativeCallAbi::Cdecl);
        assert_eq!(signature.result, None);
        assert_eq!(
            signature.parameters,
            vec![GuestValueLayout::Scalar(GuestStorage::Pointer)]
        );
        let layout = read_guest_layout(&root.field("layout"));
        assert_eq!(layout.byte_length, 16);
        assert_eq!(layout.fields[0].name, "pers.weapon");
    }

    #[test]
    #[should_panic(expected = "PE address space")]
    fn rejects_offsets_beyond_pe_space() {
        let value = parse_json(r#"{"offset":4294967296}"#).expect("valid json");
        let root = Reader::root(&value);
        native_offset(&root.field("offset"));
    }

    #[test]
    #[should_panic(expected = "must contain instructions")]
    fn rejects_empty_regions() {
        let value = parse_json(r#"{"entry":99,"join":99}"#).expect("valid json");
        let root = Reader::root(&value);
        native_region(&root);
    }
}
