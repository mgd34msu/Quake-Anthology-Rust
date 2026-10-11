//! Native console numeric views. No locale or C runtime is used.
use crate::views::RuleSetId;

pub fn number(text: &str, source: RuleSetId) -> f32 {
    if matches!(source, RuleSetId::Quake | RuleSetId::QuakeWorld) {
        quake(text.as_bytes())
    } else {
        qa_core::text::atof_prefix(text).0 as f32
    }
}

pub fn integer(text: &str) -> i32 {
    qa_core::text::atoi_prefix(text.as_bytes())
}

fn quake(mut bytes: &[u8]) -> f32 {
    let sign = if bytes.first() == Some(&b'-') {
        bytes = &bytes[1..];
        -1.0
    } else {
        1.0
    };
    let mut value = 0.0f64;
    if bytes.starts_with(b"0x") || bytes.starts_with(b"0X") {
        for (byte, digit) in bytes[2..]
            .iter()
            .map_while(|&byte| qa_core::text::hex_digit(byte).map(|digit| (byte, digit)))
        {
            // Keep Q_atof's byte/base arithmetic: adding the decoded digit
            // directly rounds long values differently before the f32 cast.
            let extra = if byte.is_ascii_digit() { 0 } else { 10 };
            let base = byte - digit + extra;
            value = value * 16.0 + f64::from(byte) - f64::from(base) + f64::from(extra);
        }
    } else if bytes.first() == Some(&b'\'') {
        value = f64::from(bytes.get(1).copied().unwrap_or(0) as i8);
    } else {
        let mut places = 0;
        let mut decimal = false;
        for &byte in bytes {
            if byte == b'.' {
                decimal = true;
                places = 0;
                continue;
            }
            if !byte.is_ascii_digit() {
                break;
            }
            value = value * 10.0 + f64::from(byte) - f64::from(b'0');
            places += usize::from(decimal);
        }
        for _ in 0..places {
            value /= 10.0;
        }
    }
    (value * sign) as f32
}
