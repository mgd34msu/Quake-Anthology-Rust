//! Quake III base/game: format.
//!
//! Donor provenance: `src/content/q3/base/game/format.ts`.

// Intra-group imports: sibling modules split from the same flat port.

// ---------------------------------------------------------------------------
// Game formatting (format.ts).
// ---------------------------------------------------------------------------

/// Game format buffer bytes (`BIG_BUFFER_BYTES`).
pub const BIG_BUFFER_BYTES: usize = 32_000;

/// Left-adjust flag (`LADJUST`).
pub(crate) const LADJUST: i32 = 0x04;

/// Zero-pad flag (`ZEROPAD`).
pub(crate) const ZEROPAD: i32 = 0x80;

/// Game format argument (`GameFormatArgument`).
#[derive(Debug, Clone, PartialEq)]
pub enum GameFormatArgument {
    /// Integer argument.
    Int(i32),
    /// Float argument.
    Float(f64),
    /// String argument.
    Text(String),
    /// Null string argument.
    Null,
}

impl From<i32> for GameFormatArgument {
    fn from(value: i32) -> Self {
        Self::Int(value)
    }
}

impl From<f32> for GameFormatArgument {
    fn from(value: f32) -> Self {
        Self::Float(f64::from(value))
    }
}

impl From<f64> for GameFormatArgument {
    fn from(value: f64) -> Self {
        Self::Float(value)
    }
}

impl From<&str> for GameFormatArgument {
    fn from(value: &str) -> Self {
        Self::Text(value.to_string())
    }
}

impl From<String> for GameFormatArgument {
    fn from(value: String) -> Self {
        Self::Text(value)
    }
}

/// Bounded format output (`FormatOutput`).
pub(crate) struct FormatOutput {
    /// Output units.
    value: Vec<char>,
}

impl FormatOutput {
    /// Empty output.
    fn new() -> Self {
        Self { value: Vec::new() }
    }

    /// Reserve buffer space (`reserve`).
    fn reserve(&self, bytes: usize) {
        if self.value.len() + bytes >= BIG_BUFFER_BYTES {
            panic!("game format exceeds the 32000-byte Com_sprintf buffer");
        }
    }

    /// Append a byte unit (`appendByte`).
    fn append_byte(&mut self, byte: i32) {
        self.reserve(1);
        self.value.push(char::from((byte & 255) as u8));
    }

    /// Append string units (`appendBytes`).
    fn append_chars(&mut self, value: &[char], length: usize) {
        self.reserve(length);
        self.value.extend(value.iter().take(length));
    }

    /// Append repeated byte units (`appendRepeated`).
    fn append_repeated(&mut self, byte: i32, count: i32) {
        if count < 0 {
            panic!("game format padding count is unsafe");
        }
        let count = count as usize;
        self.reserve(count);
        self.value
            .extend(std::iter::repeat_n(char::from((byte & 255) as u8), count));
    }

    /// Apply NUL and destination bounds (`finish`).
    fn finish(&self, max_bytes: usize) -> String {
        let end = self
            .value
            .iter()
            .position(|unit| *unit == '\0')
            .unwrap_or(self.value.len());
        self.value[..end.min(max_bytes.saturating_sub(1))].iter().collect()
    }
}

/// String byte length with validation (`byteLength`).
pub(crate) fn format_byte_length(value: &[char], limit: usize) -> usize {
    let end = value.len().min(limit);
    for (index, unit) in value.iter().enumerate().take(end) {
        if *unit == '\0' {
            return index;
        }
        if (*unit as u32) > 255 {
            panic!("game format strings must contain byte-valued code units");
        }
    }
    end
}

/// Fetch a format argument (`argumentAt`).
pub(crate) fn argument_at(args: &[GameFormatArgument], index: usize, specifier: char) -> &GameFormatArgument {
    args.get(index)
        .unwrap_or_else(|| panic!("missing argument {index} for %{specifier}"))
}

/// Integer format argument (`integerArgument`).
pub(crate) fn integer_argument(value: &GameFormatArgument, index: usize, specifier: char) -> i32 {
    match value {
        GameFormatArgument::Int(number) => *number,
        GameFormatArgument::Float(number) => {
            if number.fract() != 0.0 || *number < f64::from(i32::MIN) || *number > f64::from(i32::MAX) {
                panic!("argument {index} for %{specifier} must be a signed 32-bit integer");
            }
            *number as i32
        }
        _ => panic!("argument {index} for %{specifier} must be a number"),
    }
}

/// Float format argument (`floatArgument`).
pub(crate) fn float_argument(value: &GameFormatArgument, index: usize) -> f32 {
    let number = match value {
        GameFormatArgument::Float(number) => *number,
        GameFormatArgument::Int(number) => f64::from(*number),
        _ => panic!("argument {index} for %f must be a number"),
    };
    let stored = number as f32;
    if !stored.is_finite() || stored.abs() > 2_147_483_647.0 {
        panic!("argument {index} for %f is outside the source's safe int-cast range");
    }
    stored
}

/// Reversed integer bytes (`reversedIntegerBytes`).
pub(crate) fn reversed_integer_bytes(value: i32) -> Vec<i32> {
    let mut remaining = if value < 0 { value.wrapping_neg() } else { value };
    let mut bytes = Vec::new();
    loop {
        bytes.push(48 + remaining % 10);
        remaining /= 10;
        if remaining == 0 {
            break;
        }
    }
    if value < 0 {
        bytes.push(45);
    }
    bytes
}

/// Integer formatting (`addInt`).
pub(crate) fn add_int(output: &mut FormatOutput, value: i32, width: i32, flags: i32) {
    let reversed = reversed_integer_bytes(value);
    if (flags & LADJUST) == 0 {
        let padding = if width > reversed.len() as i32 {
            width - reversed.len() as i32
        } else {
            0
        };
        output.append_repeated(if (flags & ZEROPAD) != 0 { 48 } else { 32 }, padding);
    }
    for index in (0..reversed.len()).rev() {
        output.append_byte(reversed[index]);
    }
    if (flags & LADJUST) != 0 {
        let remaining = width.wrapping_sub(reversed.len() as i32);
        if remaining < 0 {
            panic!("left-adjusted integer width would enter the source's negative padding loop");
        }
        output.append_repeated(if (flags & ZEROPAD) != 0 { 48 } else { 32 }, remaining);
    }
}

/// Float formatting (`addFloat`).
pub(crate) fn add_float(output: &mut FormatOutput, value: f32, width: i32, precision: i32) {
    let mut remaining = if value < 0.0 { -value } else { value };
    let integer = remaining.trunc() as i32;
    let mut reversed = reversed_integer_bytes(integer);
    if value < 0.0 {
        reversed.push(45);
    }
    let padding = if width > reversed.len() as i32 {
        width - reversed.len() as i32
    } else {
        0
    };
    output.append_repeated(32, padding);
    for index in (0..reversed.len()).rev() {
        output.append_byte(reversed[index]);
    }
    let digits = if precision < 0 { 6 } else { precision };
    if digits > 32 {
        panic!("float precision would overflow AddFloat's 32-byte digit buffer");
    }
    if digits == 0 {
        return;
    }
    output.append_byte(46);
    for _ in 0..digits {
        remaining -= remaining.trunc();
        remaining *= 10.0;
        output.append_byte(48 + remaining.trunc() as i32 % 10);
    }
}

/// String formatting (`addString`).
pub(crate) fn add_string(output: &mut FormatOutput, value: Option<&[char]>, width: i32, precision: i32) {
    static NULL_TEXT: [char; 6] = ['(', 'n', 'u', 'l', 'l', ')'];
    let (text, effective) = match value {
        None => (NULL_TEXT.as_slice(), -1),
        Some(text) => (text, precision),
    };
    let length = format_byte_length(text, if effective < 0 { text.len() } else { effective as usize });
    output.append_chars(text, length);
    let padding = width.wrapping_sub(length as i32);
    if padding > 0 {
        output.append_repeated(32, padding);
    }
}

/// Format-string byte at an index (`formatByte`).
pub(crate) fn format_byte(format: &[char], index: usize) -> Option<u32> {
    if index >= format.len() {
        return None;
    }
    let byte = format[index] as u32;
    if byte == 0 {
        return None;
    }
    if byte > 255 {
        panic!("game format strings must contain byte-valued code units");
    }
    Some(byte)
}

/// Format with `bg_lib.c` rules and `Q_strncpyz` bounds (`gameFormat`).
#[must_use]
pub fn game_format_bounded(format: &str, args: &[GameFormatArgument], max_bytes: usize) -> String {
    if max_bytes < 1 {
        panic!("game format destination capacity must be a positive safe integer");
    }
    let format: Vec<char> = format.chars().collect();
    let mut output = FormatOutput::new();
    let mut cursor = 0usize;
    let mut argument_index = 0usize;
    while let Some(literal) = format_byte(&format, cursor) {
        if literal != 37 {
            output.append_byte(literal as i32);
            cursor += 1;
            continue;
        }
        cursor += 1;
        let mut flags = 0;
        let mut width = 0;
        let mut precision = -1;
        loop {
            let Some(specifier_byte) = format_byte(&format, cursor) else {
                panic!("unterminated game format specifier");
            };
            cursor += 1;
            if specifier_byte == 45 {
                flags |= LADJUST;
                continue;
            }
            if specifier_byte == 46 {
                let mut parsed = 0i32;
                loop {
                    let digit = format_byte(&format, cursor);
                    if digit.is_none_or(|code| !(48..=57).contains(&code)) {
                        break;
                    }
                    let digit = digit.unwrap_or(48) as i32;
                    parsed = parsed.wrapping_mul(10).wrapping_add(digit - 48);
                    cursor += 1;
                }
                precision = if parsed < 0 { -1 } else { parsed };
                continue;
            }
            if specifier_byte == 48 {
                flags |= ZEROPAD;
                continue;
            }
            if (49..=57).contains(&specifier_byte) {
                let mut parsed = 0i32;
                let mut digit = specifier_byte;
                while (48..=57).contains(&digit) {
                    parsed = parsed.wrapping_mul(10).wrapping_add(digit as i32 - 48);
                    let Some(next) = format_byte(&format, cursor) else {
                        panic!("unterminated game format specifier");
                    };
                    cursor += 1;
                    digit = next;
                }
                width = parsed;
                cursor -= 1;
                continue;
            }
            let specifier = char::from_u32(specifier_byte).unwrap_or('?');
            if specifier_byte == 37 {
                output.append_byte(37);
                break;
            }
            let argument = argument_at(args, argument_index, specifier);
            if specifier_byte == 100 || specifier_byte == 105 {
                add_int(
                    &mut output,
                    integer_argument(argument, argument_index, specifier),
                    width,
                    flags,
                );
            } else if specifier_byte == 102 {
                add_float(&mut output, float_argument(argument, argument_index), width, precision);
            } else if specifier_byte == 115 {
                match argument {
                    GameFormatArgument::Text(text) => {
                        let chars: Vec<char> = text.chars().collect();
                        add_string(&mut output, Some(&chars), width, precision);
                    }
                    GameFormatArgument::Null => add_string(&mut output, None, width, precision),
                    _ => panic!("argument {argument_index} for %s must be a string or null"),
                }
            } else {
                output.append_byte(integer_argument(argument, argument_index, specifier));
            }
            argument_index += 1;
            break;
        }
    }
    output.finish(max_bytes)
}

/// Format with the default 32000-byte destination (`gameFormat`).
#[must_use]
pub fn game_format(format: &str, args: &[GameFormatArgument]) -> String {
    game_format_bounded(format, args, BIG_BUFFER_BYTES)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn game_format_scoreboard_shapes() {
        assert_eq!(
            game_format(
                "%5i %4i %4i %s",
                &[
                    GameFormatArgument::Int(1),
                    GameFormatArgument::Int(2),
                    GameFormatArgument::Int(3),
                    GameFormatArgument::Text("name".to_string())
                ],
            ),
            "    1    2    3 name"
        );
        assert_eq!(
            game_format(
                " SPECT %3i %4i %s",
                &[
                    GameFormatArgument::Int(5),
                    GameFormatArgument::Int(6),
                    GameFormatArgument::Text("x".to_string())
                ],
            ),
            " SPECT   5    6 x"
        );
        assert_eq!(
            game_format(
                "%i:%i%i",
                &[
                    GameFormatArgument::Int(1),
                    GameFormatArgument::Int(2),
                    GameFormatArgument::Int(3)
                ],
            ),
            "1:23"
        );
        assert_eq!(game_format("%2i", &[GameFormatArgument::Int(7)]), " 7");
        assert_eq!(game_format("100%%", &[]), "100%");
        assert_eq!(game_format_bounded("abcdef", &[], 4), "abc");
        assert_eq!(
            game_format_bounded("%s", &[GameFormatArgument::Text("toolong".to_string())], 4),
            "too"
        );
    }
}
