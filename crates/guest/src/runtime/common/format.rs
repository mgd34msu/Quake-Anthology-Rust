//! UCRT/glibc narrow `printf` over checked guest bytes.
//!
//! Donor: `src/guest/runtime/common/format.ts`. No host `printf` or native
//! CRT calls; wide-to-multibyte conversion assumes the installed C locale,
//! never the host locale.

pub mod arguments;
pub mod float;
pub mod scan;

use std::collections::{HashMap, VecDeque};
use std::sync::{Mutex, OnceLock};

use crate::core::contracts::{GuestAccess, GuestAddress};
use crate::core::memory::SparseGuestMemory;
use crate::error::GuestError;
use crate::runtime::common::format::arguments::{
    FormatArgument, FormatArgumentType, FormatArguments, FormatDialect,
};
use crate::runtime::common::format::float::{format_float, FormatRounding};
use crate::runtime::common::memory::{read_string, write_unsigned};

/// Output termination rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FormatTermination {
    /// C99: always NUL-terminate when capacity allows.
    C99,
    /// UCRT: `-2` when the buffer fills exactly.
    Ucrt,
    /// UCRT legacy: `-1` on truncation.
    UcrtLegacy,
}

/// One `printf`-family request.
pub struct GuestFormatRequest<'m> {
    /// Guest memory.
    pub memory: &'m mut SparseGuestMemory,
    /// Runtime dialect.
    pub dialect: FormatDialect,
    /// Format string address.
    pub format: Option<GuestAddress>,
    /// Varargs address.
    pub arguments: Option<GuestAddress>,
    /// Output buffer address.
    pub buffer: Option<GuestAddress>,
    /// Output buffer capacity (`size_t`).
    pub capacity: u64,
    /// Termination rule.
    pub termination: FormatTermination,
    /// UCRT count flag, independent from legacy termination.
    pub continue_count: bool,
    /// Float rounding.
    pub rounding: FormatRounding,
    /// Exponent digits (`e` conversions).
    pub exponent_digits: usize,
    /// glibc fortify: forbid `%n` in writable format storage.
    pub fortify: bool,
}

/// One `printf`-family result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GuestFormatResult {
    /// Character count, or a negative error sentinel.
    pub result: i32,
    /// `errno` to report, if any.
    pub errno: Option<i32>,
}

struct FormatFailure {
    errno: i32,
}

struct BufferFull;

#[derive(Debug, Clone)]
struct Reference {
    index: usize,
    argument_type: FormatArgumentType,
}

#[derive(Debug, Clone)]
enum Amount {
    Literal(i64),
    Argument(Reference),
}

#[derive(Debug, Clone)]
struct Conversion {
    code: char,
    length: String,
    flags: String,
    width: Amount,
    precision: Option<Amount>,
    argument: Reference,
}

#[derive(Debug, Clone)]
enum Token {
    Literal(String),
    Conversion(Conversion),
}

#[derive(Debug, Clone)]
struct ParsedFormat {
    tokens: Vec<Token>,
    positional: bool,
    references: Vec<Reference>,
}

struct FormatCache {
    map: HashMap<String, ParsedFormat>,
    order: VecDeque<String>,
}

static PARSED_FORMATS: OnceLock<Mutex<HashMap<(FormatDialect, usize), FormatCache>>> =
    OnceLock::new();

fn retained_format(
    text: &str,
    dialect: FormatDialect,
    pointer_bytes: usize,
) -> Result<ParsedFormat, FormatFailure> {
    let caches = PARSED_FORMATS.get_or_init(|| Mutex::new(HashMap::new()));
    let mut caches = caches.lock().unwrap_or_else(|error| error.into_inner());
    let cache = caches
        .entry((dialect, pointer_bytes))
        .or_insert_with(|| FormatCache {
            map: HashMap::new(),
            order: VecDeque::new(),
        });
    if let Some(retained) = cache.map.get(text) {
        return Ok(retained.clone());
    }
    let result = parse(text, dialect, pointer_bytes)?;
    if text.chars().count() <= 4096 {
        if cache.map.len() == 256 {
            if let Some(oldest) = cache.order.pop_front() {
                cache.map.remove(&oldest);
            }
        }
        cache.order.push_back(text.to_string());
        cache.map.insert(text.to_string(), result.clone());
    }
    Ok(result)
}

fn integer_bits(length: &str, dialect: FormatDialect, pointer_bytes: usize) -> u32 {
    match length {
        "hh" => 8,
        "h" => 16,
        "ll" | "j" | "I64" | "q" => 64,
        "L" => {
            if dialect == FormatDialect::SystemV {
                64
            } else {
                32
            }
        }
        "l" => {
            if dialect == FormatDialect::Windows {
                32
            } else {
                (pointer_bytes * 8) as u32
            }
        }
        "z" | "t" | "I" => (pointer_bytes * 8) as u32,
        _ => 32,
    }
}

struct Parser<'a> {
    chars: &'a [char],
    at: usize,
    sequence: usize,
    positional: bool,
    sequential: bool,
    dialect: FormatDialect,
    pointer_bytes: usize,
}

impl Parser<'_> {
    fn invalid(&self) -> FormatFailure {
        FormatFailure { errno: 22 }
    }

    fn peek(&self) -> char {
        self.chars.get(self.at).copied().unwrap_or('\0')
    }

    fn number(&mut self) -> Result<i64, FormatFailure> {
        let start = self.at;
        while self.at < self.chars.len() && self.chars[self.at].is_ascii_digit() {
            self.at += 1;
        }
        let value: i64 = self.chars[start..self.at].iter().collect::<String>().parse().unwrap_or(i64::MAX);
        if value > 0x7fff_ffff {
            return Err(FormatFailure {
                errno: if self.dialect == FormatDialect::Windows { 132 } else { 75 },
            });
        }
        Ok(value)
    }

    fn position(&mut self) -> Result<Option<usize>, FormatFailure> {
        let start = self.at;
        let value = self.number()?;
        if self.peek() != '$' {
            self.at = start;
            return Ok(None);
        }
        if self.dialect == FormatDialect::Windows || value < 1 || value > 4096 {
            return Err(self.invalid());
        }
        self.at += 1;
        Ok(Some((value - 1) as usize))
    }

    fn reference(
        &mut self,
        argument_type: FormatArgumentType,
        index: Option<usize>,
        references: &mut Vec<Reference>,
    ) -> Result<Reference, FormatFailure> {
        if index.is_none() {
            self.sequential = true;
        } else {
            self.positional = true;
        }
        if self.positional && self.sequential {
            return Err(self.invalid());
        }
        let result = Reference {
            index: index.unwrap_or_else(|| {
                let next = self.sequence;
                self.sequence += 1;
                next
            }),
            argument_type,
        };
        references.push(result.clone());
        Ok(result)
    }

    fn amount(&mut self, references: &mut Vec<Reference>) -> Result<Amount, FormatFailure> {
        if self.peek() != '*' {
            return Ok(Amount::Literal(self.number()?));
        }
        self.at += 1;
        let index = self.position()?;
        Ok(Amount::Argument(self.reference(
            FormatArgumentType::Int,
            index,
            references,
        )?))
    }
}

fn parse(
    text: &str,
    dialect: FormatDialect,
    pointer_bytes: usize,
) -> Result<ParsedFormat, FormatFailure> {
    let chars: Vec<char> = text.chars().collect();
    let mut parser = Parser {
        chars: &chars,
        at: 0,
        sequence: 0,
        positional: false,
        sequential: false,
        dialect,
        pointer_bytes,
    };
    let mut tokens = Vec::new();
    let mut references = Vec::new();
    while parser.at < chars.len() {
        let start = parser.at;
        while parser.at < chars.len() && chars[parser.at] != '%' {
            parser.at += 1;
        }
        if parser.at > start {
            tokens.push(Token::Literal(chars[start..parser.at].iter().collect()));
        }
        if parser.at == chars.len() {
            break;
        }
        parser.at += 1;
        if parser.peek() == '%' {
            tokens.push(Token::Literal("%".to_string()));
            parser.at += 1;
            continue;
        }
        let index = parser.position()?;
        let mut flags = String::new();
        while parser.at < chars.len() && "-+ #0'".contains(chars[parser.at]) {
            flags.push(chars[parser.at]);
            parser.at += 1;
        }
        let width = parser.amount(&mut references)?;
        let mut precision = None;
        if parser.peek() == '.' {
            parser.at += 1;
            precision = Some(parser.amount(&mut references)?);
        }
        let mut length = String::new();
        for candidate in [
            "I64", "I32", "hh", "ll", "h", "l", "L", "j", "z", "t", "I", "w", "q",
        ] {
            let end = parser.at + candidate.len();
            if end <= chars.len() && chars[parser.at..end].iter().collect::<String>() == candidate {
                length = candidate.to_string();
                parser.at = end;
                break;
            }
        }
        let code = if parser.at < chars.len() {
            let code = chars[parser.at];
            parser.at += 1;
            code
        } else {
            return Err(parser.invalid());
        };
        if !"diouxXfFeEgGaAcCsSpn".contains(code) {
            return Err(parser.invalid());
        }
        if dialect == FormatDialect::SystemV && ["I", "I32", "I64", "w"].contains(&length.as_str()) {
            return Err(parser.invalid());
        }
        let argument_type = if "fFeEgGaA".contains(code) {
            if length == "L" {
                FormatArgumentType::LongDouble
            } else {
                FormatArgumentType::Double
            }
        } else if "sSpn".contains(code) {
            FormatArgumentType::Pointer
        } else if "cC".contains(code) {
            FormatArgumentType::Int
        } else if integer_bits(&length, dialect, pointer_bytes) == 64 {
            FormatArgumentType::Int64
        } else {
            FormatArgumentType::Int
        };
        let argument = parser.reference(argument_type, index, &mut references)?;
        tokens.push(Token::Conversion(Conversion {
            code,
            length,
            flags,
            width,
            precision,
            argument,
        }));
    }
    Ok(ParsedFormat {
        tokens,
        positional: parser.positional,
        references,
    })
}

#[derive(Default)]
struct Output {
    total: u64,
    used: u64,
}

impl Output {
    fn append(
        &mut self,
        request: &mut GuestFormatRequest<'_>,
        text: &str,
    ) -> Result<(), OutputHalt> {
        let units: Vec<u16> = text.encode_utf16().collect();
        if units.is_empty() {
            return Ok(());
        }
        let available = match request.buffer {
            None => 0,
            Some(_) => {
                if request.capacity > self.used {
                    request.capacity - self.used
                } else {
                    0
                }
            }
        };
        let copied = (available as usize).min(units.len());
        if let Some(buffer) = request.buffer {
            if copied > 0 {
                let bytes: Vec<u8> = units[..copied].iter().map(|unit| (unit & 0xff) as u8).collect();
                let at = request.memory.offset(buffer, self.used as i64).map_err(OutputHalt::guest)?;
                request.memory.write(at, &bytes).map_err(OutputHalt::guest)?;
                self.used += copied as u64;
            }
        }
        if request.buffer.is_some()
            && request.termination != FormatTermination::C99
            && !request.continue_count
            && (copied as u64) < units.len() as u64
        {
            return Err(OutputHalt::Full);
        }
        self.total += units.len() as u64;
        if self.total > 0x7fff_ffff {
            return Err(OutputHalt::Failure(FormatFailure {
                errno: if request.dialect == FormatDialect::Windows {
                    132
                } else {
                    75
                },
            }));
        }
        Ok(())
    }

    fn repeat(
        &mut self,
        request: &mut GuestFormatRequest<'_>,
        character: char,
        count: usize,
    ) -> Result<(), OutputHalt> {
        if count == 0 {
            return Ok(());
        }
        if request.termination == FormatTermination::C99
            || request.continue_count
            || request.buffer.is_none()
        {
            let available = match request.buffer {
                None => 0,
                Some(_) => request.capacity.saturating_sub(self.used) as usize,
            };
            let copied = count.min(available);
            let mut done = 0;
            while done < copied {
                let chunk = (copied - done).min(4096);
                self.append(request, &character.to_string().repeat(chunk))?;
                done += chunk;
            }
            self.total += (count - copied) as u64;
            if self.total > 0x7fff_ffff {
                return Err(OutputHalt::Failure(FormatFailure {
                    errno: if request.dialect == FormatDialect::Windows {
                        132
                    } else {
                        75
                    },
                }));
            }
        } else {
            let mut done = 0;
            while done < count {
                let chunk = (count - done).min(4096);
                self.append(request, &character.to_string().repeat(chunk))?;
                done += chunk;
            }
        }
        Ok(())
    }

    fn finish(
        &self,
        request: &mut GuestFormatRequest<'_>,
        error: Option<i32>,
        full: bool,
    ) -> Result<GuestFormatResult, GuestError> {
        let Some(buffer) = request.buffer else {
            return Ok(GuestFormatResult {
                result: if error.is_none() { self.total as i32 } else { -1 },
                errno: error,
            });
        };
        let capacity = request.capacity;
        if request.termination == FormatTermination::UcrtLegacy {
            if self.used < capacity {
                let at = request.memory.offset(buffer, self.used as i64)?;
                request.memory.write_u8(at, 0)?;
            }
            return Ok(GuestFormatResult {
                result: if error.is_some() || full || self.total > capacity {
                    -1
                } else {
                    self.total as i32
                },
                errno: error,
            });
        }
        if capacity > 0 {
            let at = if error.is_some()
                && request.dialect == FormatDialect::Windows
                && request.termination == FormatTermination::C99
            {
                0
            } else if self.used >= capacity {
                capacity - 1
            } else {
                self.used
            };
            let slot = request.memory.offset(buffer, at as i64)?;
            request.memory.write_u8(slot, 0)?;
        }
        if request.termination == FormatTermination::Ucrt
            && (capacity == 0 || self.used == capacity)
        {
            return Ok(GuestFormatResult {
                result: if capacity == 0 { -1 } else { -2 },
                errno: error,
            });
        }
        Ok(GuestFormatResult {
            result: if error.is_none() { self.total as i32 } else { -1 },
            errno: error,
        })
    }
}

enum OutputHalt {
    Full,
    Failure(FormatFailure),
    Guest(GuestError),
}

impl OutputHalt {
    fn guest(error: GuestError) -> Self {
        OutputHalt::Guest(error)
    }
}

fn integer_value(value: &FormatArgument) -> Result<u64, FormatFailure> {
    match value {
        FormatArgument::Integer(value) => Ok(*value),
        FormatArgument::Float(_) => Err(FormatFailure { errno: 22 }),
    }
}

fn readonly_format(request: &mut GuestFormatRequest<'_>, length: usize) -> bool {
    let Some(format) = request.format else {
        return false;
    };
    let mut cursor = format.offset;
    let end = cursor + length as u64 + 1;
    let mut mappings = request.memory.mappings();
    mappings.sort_by(|a, b| a.base.cmp(&b.base));
    for mapping in &mappings {
        let stop = mapping.base + mapping.byte_length as u64;
        if stop <= cursor {
            continue;
        }
        if mapping.base > cursor || mapping.permissions.allows(GuestAccess::Write) {
            return false;
        }
        cursor = stop;
        if cursor >= end {
            return true;
        }
    }
    false
}

fn string_argument(
    request: &mut GuestFormatRequest<'_>,
    address: Option<GuestAddress>,
    wide: bool,
    precision: Option<usize>,
) -> Result<String, OutputHalt> {
    let Some(address) = address else {
        if precision.is_some() && request.dialect == FormatDialect::SystemV && precision.unwrap_or(6) < 6 {
            return Ok(String::new());
        }
        let end = precision.unwrap_or(6).min(6);
        return Ok("(null)".chars().take(end).collect());
    };
    if !wide {
        if precision == Some(0) {
            return Ok(String::new());
        }
        let maximum = precision.unwrap_or(1024 * 1024);
        let length = request
            .memory
            .find_zero(address, maximum)
            .map_err(OutputHalt::guest)?;
        if length < 0 && precision.is_none() {
            return Err(OutputHalt::guest(GuestError::invalid(
                "Guest printf string exceeds runtime string limit",
            )));
        }
        let bytes = request
            .memory
            .copy(address, if length < 0 { maximum } else { length as usize })
            .map_err(OutputHalt::guest)?;
        return Ok(bytes.iter().map(|byte| *byte as char).collect());
    }
    let width = if request.dialect == FormatDialect::Windows { 2 } else { 4 };
    let mut text = String::new();
    let mut index = 0;
    while precision.is_none_or(|precision| text.chars().count() < precision) {
        if index >= 1024 * 1024 {
            return Err(OutputHalt::guest(GuestError::invalid(
                "Guest printf string exceeds runtime string limit",
            )));
        }
        let at = request
            .memory
            .offset(address, (index * width) as i64)
            .map_err(OutputHalt::guest)?;
        let code = if width == 4 {
            request.memory.read_u32(at).map_err(OutputHalt::guest)?
        } else if width == 2 {
            u32::from(request.memory.read_u16(at).map_err(OutputHalt::guest)?)
        } else {
            u32::from(request.memory.read_u8(at).map_err(OutputHalt::guest)?)
        };
        if code == 0 {
            break;
        }
        // The runtimes install the C locale. Wide-to-multibyte conversion
        // must not use the host locale.
        let limit = if request.dialect == FormatDialect::Windows {
            255
        } else {
            127
        };
        if code > limit {
            return Err(OutputHalt::Failure(FormatFailure {
                errno: if request.dialect == FormatDialect::Windows {
                    42
                } else {
                    84
                },
            }));
        }
        text.push(char::from_u32(code).unwrap_or(char::REPLACEMENT_CHARACTER));
        index += 1;
    }
    Ok(text)
}

fn uint_to_radix(mut value: u64, radix: u32) -> String {
    if value == 0 {
        return "0".to_string();
    }
    let mut digits = Vec::new();
    while value > 0 {
        digits.push(char::from_digit((value % u64::from(radix)) as u32, radix).expect("radix digit"));
        value /= u64::from(radix);
    }
    digits.iter().rev().collect()
}

struct Driver {
    reader: Option<FormatArguments>,
    values: HashMap<usize, FormatArgument>,
    positional: bool,
}

impl Driver {
    fn get(
        &mut self,
        memory: &mut SparseGuestMemory,
        reference: &Reference,
    ) -> Result<FormatArgument, OutputHalt> {
        if !self.positional {
            let reader = self.reader.as_mut().ok_or_else(|| {
                OutputHalt::guest(GuestError::callback("Missing printf argument reader"))
            })?;
            return reader.next(memory, reference.argument_type).map_err(OutputHalt::guest);
        }
        self.values.get(&reference.index).cloned().ok_or(OutputHalt::Failure(FormatFailure { errno: 22 }))
    }

    fn amount(
        &mut self,
        memory: &mut SparseGuestMemory,
        amount: &Amount,
    ) -> Result<i64, OutputHalt> {
        match amount {
            Amount::Literal(value) => Ok(*value),
            Amount::Argument(reference) => {
                let value = self.get(memory, reference)?;
                Ok(i64::from((integer_value(&value).map_err(OutputHalt::Failure)? as u32) as i32))
            }
        }
    }
}

/// Format one guest `printf` request into its buffer.
pub fn format_guest_buffer(
    request: &mut GuestFormatRequest<'_>,
) -> Result<GuestFormatResult, GuestError> {
    let pointer_bits = request.memory.pointer_bytes() * 8;
    if pointer_bits < 64 && request.capacity > (1u64 << pointer_bits) - 1 {
        return Err(GuestError::invalid(
            "Guest printf buffer count is not size_t",
        ));
    }
    if request.format.is_none() || (request.buffer.is_none() && request.capacity != 0) {
        return Ok(GuestFormatResult {
            result: -1,
            errno: Some(22),
        });
    }
    let mut output = Output::default();
    let outcome = format_inner(request, &mut output);
    match outcome {
        Ok(()) => output.finish(request, None, false),
        Err(OutputHalt::Full) => output.finish(request, None, true),
        Err(OutputHalt::Failure(failure)) => output.finish(request, Some(failure.errno), false),
        Err(OutputHalt::Guest(error)) => Err(error),
    }
}

fn format_inner(
    request: &mut GuestFormatRequest<'_>,
    output: &mut Output,
) -> Result<(), OutputHalt> {
    let format_address = request.format.expect("format checked");
    let text = read_string(request.memory, format_address, false).map_err(OutputHalt::guest)?;
    let parsed = retained_format(text.as_str(), request.dialect, request.memory.pointer_bytes())
        .map_err(OutputHalt::Failure)?;
    // A literal format need not touch a caller's otherwise unused va_list.
    let reader = if parsed.references.is_empty() {
        None
    } else {
        Some(
            FormatArguments::open(request.memory, request.dialect, request.arguments)
                .map_err(OutputHalt::guest)?,
        )
    };
    let mut driver = Driver {
        reader,
        values: HashMap::new(),
        positional: parsed.positional,
    };
    if parsed.positional {
        let mut types: HashMap<usize, FormatArgumentType> = HashMap::new();
        for reference in &parsed.references {
            if let Some(previous) = types.get(&reference.index) {
                if *previous != reference.argument_type {
                    return Err(OutputHalt::Failure(FormatFailure { errno: 22 }));
                }
            }
            types.insert(reference.index, reference.argument_type);
        }
        for index in 0..types.len() {
            let Some(argument_type) = types.get(&index).copied() else {
                return Err(OutputHalt::Failure(FormatFailure { errno: 22 }));
            };
            let reader = driver.reader.as_mut().ok_or(OutputHalt::Failure(FormatFailure { errno: 22 }))?;
            let value = reader.next(request.memory, argument_type).map_err(OutputHalt::guest)?;
            driver.values.insert(index, value);
        }
    }
    for token in parsed.tokens.clone() {
        match token {
            Token::Literal(text) => output.append(request, &text)?,
            Token::Conversion(conversion) => {
                convert(request, output, &mut driver, &conversion, text.chars().count())?;
            }
        }
    }
    Ok(())
}

fn convert(
    request: &mut GuestFormatRequest<'_>,
    output: &mut Output,
    driver: &mut Driver,
    token: &Conversion,
    format_length: usize,
) -> Result<(), OutputHalt> {
    let mut width = driver.amount(request.memory, &token.width)?;
    let mut precision = match &token.precision {
        None => None,
        Some(amount) => Some(driver.amount(request.memory, amount)?),
    };
    let left = token.flags.contains('-') || width < 0;
    width = width.abs();
    if precision.is_some_and(|precision| precision < 0) {
        precision = None;
    }
    if precision.is_some_and(|precision| precision > 1024 * 1024) {
        return Err(OutputHalt::guest(GuestError::invalid(
            "Guest printf precision exceeds runtime conversion limit",
        )));
    }
    let precision_usize = precision.map(|precision| precision as usize);
    let alternate = token.flags.contains('#');
    let plus = token.flags.contains('+');
    let blank = token.flags.contains(' ');
    let value = driver.get(request.memory, &token.argument)?;
    let code = token.code;
    let mut body = String::new();
    let mut prefix = String::new();
    let mut zero = token.flags.contains('0') && !left;
    if "diouxX".contains(code) {
        let signed = code == 'd' || code == 'i';
        let bits = integer_bits(&token.length, request.dialect, request.memory.pointer_bytes());
        let raw = integer_value(&value).map_err(OutputHalt::Failure)?;
        let number: i128 = if signed {
            match bits {
                8 => i128::from((raw as u8) as i8),
                16 => i128::from((raw as u16) as i16),
                32 => i128::from((raw as u32) as i32),
                _ => i128::from(raw as i64),
            }
        } else {
            match bits {
                8 => i128::from(raw as u8),
                16 => i128::from(raw as u16),
                32 => i128::from(raw as u32),
                _ => i128::from(raw),
            }
        };
        let magnitude = number.abs() as u64;
        let radix = if code == 'o' {
            8
        } else if code == 'x' || code == 'X' {
            16
        } else {
            10
        };
        body = if precision_usize == Some(0) && magnitude == 0 {
            String::new()
        } else {
            uint_to_radix(magnitude, radix)
        };
        if let Some(places) = precision_usize {
            if body.len() < places {
                body = format!("{:0>places$}", body);
            }
        } else if body.is_empty() {
            body = "0".to_string();
        }
        if code == 'X' {
            body = body.to_uppercase();
        }
        if signed {
            prefix = if number < 0 {
                "-".to_string()
            } else if plus {
                "+".to_string()
            } else if blank {
                " ".to_string()
            } else {
                String::new()
            };
        } else if alternate && radix == 16 && magnitude != 0 {
            prefix = if code == 'X' { "0X".to_string() } else { "0x".to_string() };
        } else if alternate && radix == 8 && !body.starts_with('0') {
            prefix = "0".to_string();
        }
        if precision.is_some() {
            zero = false;
        }
    } else if "fFeEgGaA".contains(code) {
        let FormatArgument::Float(float) = &value else {
            return Err(OutputHalt::Failure(FormatFailure { errno: 22 }));
        };
        body = format_float(
            float,
            code,
            precision_usize,
            alternate,
            request.rounding,
            request.dialect == FormatDialect::Windows,
            token.length == "L" && request.dialect == FormatDialect::SystemV,
            request.exponent_digits,
        );
        prefix = if float.negative() {
            "-".to_string()
        } else if plus {
            "+".to_string()
        } else if blank {
            " ".to_string()
        } else {
            String::new()
        };
        if !matches!(
            float,
            crate::floating_point::binary::BinaryValue::Finite { .. }
        ) {
            zero = false;
        }
        if body.starts_with("0x") || body.starts_with("0X") {
            prefix.push_str(&body[..2]);
            body = body[2..].to_string();
        }
    } else if code == 'p' {
        let raw = integer_value(&value).map_err(OutputHalt::Failure)?;
        let pointer = if request.memory.pointer_bytes() == 4 {
            u64::from(raw as u32)
        } else {
            raw
        };
        if request.dialect == FormatDialect::Windows {
            body = format!("{:X}", pointer);
            let width = request.memory.pointer_bytes() * 2;
            if body.len() < width {
                body = format!("{body:0>width$}");
            }
        } else if pointer == 0 {
            body = "(nil)".to_string();
            zero = false;
        } else {
            prefix = "0x".to_string();
            body = uint_to_radix(pointer, 16);
            let places = precision_usize.unwrap_or(1);
            if body.len() < places {
                body = format!("{body:0>places$}");
            }
        }
    } else if code == 's' || code == 'S' {
        let wide = token.length == "l" || token.length == "w" || code == 'S' && token.length != "h";
        let raw = integer_value(&value).map_err(OutputHalt::Failure)?;
        let address = request.memory.pointer(raw).map_err(OutputHalt::guest)?;
        body = string_argument(request, address, wide, precision_usize)?;
        zero = false;
    } else if code == 'c' || code == 'C' {
        let wide = token.length == "l" || token.length == "w" || code == 'C' && token.length != "h";
        let raw = integer_value(&value).map_err(OutputHalt::Failure)?;
        let mask = if wide {
            if request.dialect == FormatDialect::Windows {
                0xffff
            } else {
                0xffff_ffff
            }
        } else {
            0xff
        };
        let number = raw & mask;
        let limit = if request.dialect == FormatDialect::Windows {
            255
        } else {
            127
        };
        if wide && number > limit {
            return Err(OutputHalt::Failure(FormatFailure {
                errno: if request.dialect == FormatDialect::Windows {
                    42
                } else {
                    84
                },
            }));
        }
        body = char::from_u32(number as u32)
            .unwrap_or(char::REPLACEMENT_CHARACTER)
            .to_string();
        zero = false;
    } else if code == 'n' {
        if request.dialect == FormatDialect::Windows {
            return Err(OutputHalt::Failure(FormatFailure { errno: 22 }));
        }
        if request.fortify && !readonly_format(request, format_length) {
            return Err(OutputHalt::guest(GuestError::invalid(
                "glibc %n in writable format storage",
            )));
        }
        let raw = integer_value(&value).map_err(OutputHalt::Failure)?;
        let address = request
            .memory
            .pointer(raw)
            .map_err(OutputHalt::guest)?
            .ok_or_else(|| OutputHalt::guest(GuestError::invalid("Null printf count pointer")))?;
        let bytes = integer_bits(&token.length, request.dialect, request.memory.pointer_bytes()) / 8;
        write_unsigned(request.memory, address, bytes as usize, output.total as i128)
            .map_err(OutputHalt::guest)?;
        return Ok(());
    }
    let padding = (width as usize).saturating_sub(prefix.chars().count() + body.chars().count());
    if !left && !zero {
        output.repeat(request, ' ', padding)?;
    }
    output.append(request, &prefix)?;
    if zero {
        output.repeat(request, '0', padding)?;
    }
    output.append(request, &body)?;
    if left {
        output.repeat(request, ' ', padding)?;
    }
    Ok(())
}
