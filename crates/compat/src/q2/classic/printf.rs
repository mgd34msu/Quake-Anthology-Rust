//! Donor: `src/compat/q2/classic/printf.ts` — MSVC-shaped `printf`
//! formatting for the game imports.
//!
//! Bridges guest format strings plus variadic call values to host text for
//! `bprintf`/`dprintf`/`cprintf`/`centerprintf`/`error`.

use qa_guest::core::contracts::{GuestCallValue, GuestValueLayout};
use qa_guest::core::memory::SparseGuestMemory;

use super::layout::{q2_double, q2_int, q2_pointer, ClassicQ2Error, ClassicResult};
use super::records::read_classic_string;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StarOrNumber {
    Star,
    Number(i64),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Conversion {
    start: usize,
    end: usize,
    flags: String,
    width: StarOrNumber,
    precision: Option<StarOrNumber>,
    conv: char,
}

fn conversions(format: &str) -> ClassicResult<Vec<Conversion>> {
    let bytes = format.as_bytes();
    let mut entries = Vec::new();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] != b'%' {
            index += 1;
            continue;
        }
        let start = index;
        index += 1;
        let mut flags = String::new();
        while index < bytes.len() && b"-+ #0".contains(&bytes[index]) {
            flags.push(bytes[index] as char);
            index += 1;
        }
        let width = if index < bytes.len() && bytes[index] == b'*' {
            index += 1;
            StarOrNumber::Star
        } else {
            let mut value: i64 = 0;
            while index < bytes.len() && bytes[index].is_ascii_digit() {
                value = value.saturating_mul(10).saturating_add(i64::from(bytes[index] - b'0'));
                index += 1;
            }
            StarOrNumber::Number(value)
        };
        let mut precision = None;
        if index < bytes.len() && bytes[index] == b'.' {
            index += 1;
            precision = Some(if index < bytes.len() && bytes[index] == b'*' {
                index += 1;
                StarOrNumber::Star
            } else {
                let mut value: i64 = 0;
                while index < bytes.len() && bytes[index].is_ascii_digit() {
                    value = value.saturating_mul(10).saturating_add(i64::from(bytes[index] - b'0'));
                    index += 1;
                }
                StarOrNumber::Number(value)
            });
        }
        if index < bytes.len() && bytes[index] == b'l' {
            index += 1;
        }
        let conv = bytes.get(index).copied().unwrap_or(0) as char;
        if !"diuoxXcspfeEgG%".contains(conv) {
            let snippet: String = format[start..].chars().take(12).collect();
            return Err(ClassicQ2Error::invalid(format!(
                "Unsupported API 3 printf conversion at {snippet}"
            )));
        }
        index += 1;
        entries.push(Conversion {
            start,
            end: index,
            flags,
            width,
            precision,
            conv,
        });
    }
    Ok(entries)
}

/// Variadic layouts consumed by one format string.
pub fn classic_printf_layouts(format: &str) -> ClassicResult<Vec<GuestValueLayout>> {
    let mut result = Vec::new();
    for entry in conversions(format)? {
        if entry.conv == '%' {
            continue;
        }
        if entry.width == StarOrNumber::Star {
            result.push(q2_int());
        }
        if entry.precision == Some(StarOrNumber::Star) {
            result.push(q2_int());
        }
        result.push(if "feEgG".contains(entry.conv) {
            q2_double()
        } else if "sp".contains(entry.conv) {
            q2_pointer()
        } else {
            q2_int()
        });
    }
    Ok(result)
}

fn format_exponent(value: f64, digits: usize) -> String {
    let exp = value.abs().log10().floor() as i32;
    let scale = 10f64.powi(exp);
    let mut rounded = (value / scale * 10f64.powi(digits as i32)).round() / 10f64.powi(digits as i32);
    let mut exp = exp;
    if rounded.abs() >= 10.0 {
        exp += 1;
        rounded /= 10.0;
    }
    format!("{rounded:.digits$}e{exp:+03}")
}

fn strip_mantissa(text: &str) -> String {
    match text.find('e') {
        Some(at) => {
            let mantissa = strip_mantissa(&text[..at]);
            let exponent = &text[at + 1..];
            format!("{mantissa}e{exponent}")
        }
        None => {
            if text.contains('.') {
                text.trim_end_matches('0').trim_end_matches('.').to_string()
            } else {
                text.to_string()
            }
        }
    }
}

fn format_general(value: f64, precision: usize) -> String {
    let precision = precision.max(1);
    if value == 0.0 {
        return "0".to_string();
    }
    let exp = value.abs().log10().floor() as i32;
    if exp >= -4 && exp < precision as i32 {
        let frac = (precision as i32 - 1 - exp).max(0) as usize;
        strip_mantissa(&format!("{value:.frac$}"))
    } else {
        strip_mantissa(&format_exponent(value, precision - 1))
    }
}

fn format_float(conv: char, value: f64, precision: Option<usize>) -> String {
    if value.is_nan() {
        return "NaN".to_string();
    }
    if value.is_infinite() {
        return if value.is_sign_negative() {
            "-Infinity".to_string()
        } else {
            "Infinity".to_string()
        };
    }
    let mut text = match conv {
        'f' => format!("{value:.prec$}", prec = precision.unwrap_or(6)),
        'e' | 'E' => format_exponent(value, precision.unwrap_or(6)),
        _ => format_general(value, precision.unwrap_or(6)),
    };
    if value == 0.0 && value.is_sign_negative() && !text.starts_with('-') {
        text.insert(0, '-');
    }
    let exponent = text.find('e').map(|at| {
        let exp: i32 = text[at + 1..].parse().unwrap_or(0);
        format!("{exp:+03}")
    });
    if let Some(exponent) = exponent {
        let at = text.find('e').unwrap();
        text.truncate(at + 1);
        text.push_str(&exponent);
    }
    if conv.is_ascii_uppercase() {
        text.to_uppercase()
    } else {
        text
    }
}

/// Format one guest format string against its variadic values.
pub fn classic_printf(
    memory: &mut SparseGuestMemory,
    format: &str,
    arguments: &[GuestCallValue],
) -> ClassicResult<String> {
    let mut next = 0;
    let mut take = || -> ClassicResult<&GuestCallValue> {
        let item = arguments
            .get(next)
            .ok_or_else(|| ClassicQ2Error::invalid("API 3 printf argument missing"))?;
        next += 1;
        Ok(item)
    };
    let integer = |value: &GuestCallValue| -> ClassicResult<i64> {
        match value {
            GuestCallValue::Int32(value) => Ok(i64::from(*value)),
            GuestCallValue::Uint32(value) => Ok(i64::from(*value)),
            _ => Err(ClassicQ2Error::invalid("API 3 printf integer required")),
        }
    };
    let mut position = 0;
    let mut result = String::new();
    for entry in conversions(format)? {
        result.push_str(&format[position..entry.start]);
        position = entry.end;
        if entry.conv == '%' {
            result.push('%');
            continue;
        }
        let width = match entry.width {
            StarOrNumber::Star => integer(take()?)?,
            StarOrNumber::Number(value) => value,
        };
        let precision_input = match entry.precision {
            Some(StarOrNumber::Star) => Some(integer(take()?)?),
            Some(StarOrNumber::Number(value)) => Some(value),
            None => None,
        };
        let precision = precision_input.filter(|value| *value >= 0).map(|value| value as usize);
        if width.abs() > 65536 || precision.is_some_and(|value| value > 100) {
            return Err(ClassicQ2Error::invalid(
                "API 3 printf width or precision exceeds supported buffer",
            ));
        }
        let mut text = match entry.conv {
            's' | 'p' => {
                let GuestCallValue::Pointer(address) = take()? else {
                    return Err(ClassicQ2Error::invalid("API 3 printf pointer required"));
                };
                if entry.conv == 's' {
                    let text = read_classic_string(memory, *address, 65536)?;
                    match precision {
                        Some(limit) => text.chars().take(limit).collect(),
                        None => text,
                    }
                } else {
                    let offset = address.map_or(0, |value| value.offset);
                    format!("{offset:08x}")
                }
            }
            conv if "feEgG".contains(conv) => {
                let GuestCallValue::Float64(value) = take()? else {
                    return Err(ClassicQ2Error::invalid("API 3 printf promoted double required"));
                };
                format_float(conv, *value, precision)
            }
            conv => {
                let (signed, unsigned) = match take()? {
                    GuestCallValue::Int32(value) => (*value, *value as u32),
                    GuestCallValue::Uint32(value) => (*value as i32, *value),
                    _ => return Err(ClassicQ2Error::invalid("API 3 printf integer required")),
                };
                let mut text = if conv == 'c' {
                    char::from((unsigned & 255) as u8).to_string()
                } else if conv == 'd' || conv == 'i' {
                    signed.to_string()
                } else {
                    match conv {
                        'o' => format!("{unsigned:o}"),
                        'u' => unsigned.to_string(),
                        _ => format!("{unsigned:x}"),
                    }
                };
                if conv == 'X' {
                    text = text.to_uppercase();
                }
                if precision == Some(0) && unsigned == 0 && conv != 'c' {
                    text = if conv == 'o' && entry.flags.contains('#') {
                        "0".to_string()
                    } else {
                        String::new()
                    };
                }
                if let Some(limit) = precision {
                    if conv != 'c' {
                        text = if let Some(rest) = text.strip_prefix('-') {
                            format!("-{rest:0>limit$}")
                        } else {
                            format!("{text:0>limit$}")
                        };
                    }
                }
                if entry.flags.contains('#') && unsigned != 0 {
                    text = match conv {
                        'o' => format!("0{text}"),
                        'x' => format!("0x{text}"),
                        'X' => format!("0X{text}"),
                        _ => text,
                    };
                }
                text
            }
        };
        if "difeEgG".contains(entry.conv) && !text.starts_with('-') {
            if entry.flags.contains('+') {
                text.insert(0, '+');
            } else if entry.flags.contains(' ') {
                text.insert(0, ' ');
            }
        }
        let left = width < 0 || entry.flags.contains('-');
        let fill = if !left && entry.flags.contains('0') && precision.is_none() && !"sc".contains(entry.conv) {
            '0'
        } else {
            ' '
        };
        let target = width.unsigned_abs() as usize;
        if left {
            while text.len() < target {
                text.push(' ');
            }
        } else if fill == '0' && text.starts_with(['+', '-']) {
            let (sign, rest) = text.split_at(1);
            let width = target.saturating_sub(1);
            text = format!("{sign}{rest:0>width$}");
        } else if fill == '0' && (text.starts_with("0x") || text.starts_with("0X")) {
            let (prefix, rest) = text.split_at(2);
            let width = target.saturating_sub(2);
            text = format!("{prefix}{rest:0>width$}");
        } else {
            while text.len() < target {
                text.insert(0, fill);
            }
        }
        result.push_str(&text);
    }
    result.push_str(&format[position..]);
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::super::records::allocate_classic_string;
    use super::*;
    use qa_core::identity::ProviderId;
    use qa_guest::core::contracts::{ContentDigest, ModuleIdentity};

    fn test_memory() -> SparseGuestMemory {
        SparseGuestMemory::new(
            ModuleIdentity::new(
                ProviderId::new("q2", "printf-test"),
                "printf",
                ContentDigest::new("sha256", "0"),
                "test",
            ),
            4,
            0x10000,
        )
        .unwrap()
    }

    fn int(value: i32) -> GuestCallValue {
        GuestCallValue::Int32(value)
    }

    #[test]
    fn integers_strings_and_pointers_format() {
        let mut memory = test_memory();
        let name = allocate_classic_string(&mut memory, "soldier").unwrap();
        let text = classic_printf(
            &mut memory,
            "%s has %d health and 0x%x armor (%c) %p%%",
            &[
                GuestCallValue::Pointer(Some(name)),
                int(100),
                int(255),
                int(65),
                GuestCallValue::Pointer(Some(name)),
            ],
        )
        .unwrap();
        assert!(text.starts_with("soldier has 100 health and 0xff armor (A) "), "{text}");
        assert!(text.ends_with('%'), "{text}");
        let padded = classic_printf(
            &mut memory,
            "[%8d][%-8d][%04d][%.3d]",
            &[int(42), int(42), int(42), int(42)],
        )
        .unwrap();
        assert_eq!(padded, "[      42][42      ][0042][042]");
        let layouts = classic_printf_layouts("%s has %d health and 0x%x armor (%c) %p%%").unwrap();
        assert_eq!(layouts, vec![q2_pointer(), q2_int(), q2_int(), q2_int(), q2_pointer()]);
    }

    #[test]
    fn floats_widths_and_errors_match_donor() {
        let mut memory = test_memory();
        let text = classic_printf(
            &mut memory,
            "%f|%.2f|%e|%g|%G|%*d|%.*f|%+d|% d|%#x|%p",
            &[
                GuestCallValue::Float64(1.5),
                GuestCallValue::Float64(1.5),
                GuestCallValue::Float64(1000.0),
                GuestCallValue::Float64(1000.0),
                GuestCallValue::Float64(0.000012345),
                int(6),
                int(7),
                int(3),
                GuestCallValue::Float64(2.718_281_828_459_045),
                int(9),
                int(9),
                int(255),
                GuestCallValue::Pointer(None),
            ],
        )
        .unwrap();
        assert_eq!(
            text,
            "1.500000|1.50|1.000000e+03|1000|1.2345E-05|     7|2.718|+9| 9|0xff|00000000"
        );
        assert!(classic_printf(&mut memory, "%d %d", &[int(1)]).is_err());
        assert!(classic_printf(&mut memory, "%q", &[]).is_err());
        assert!(classic_printf(&mut memory, "%d", &[GuestCallValue::Float64(1.0)]).is_err());
        assert!(classic_printf(&mut memory, "%999999d", &[int(1)]).is_err());
    }
}
