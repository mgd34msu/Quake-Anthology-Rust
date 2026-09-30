//! JavaScript value semantics shared by donor ports.
//!
//! String ordering, line splitting, UTF-16 slicing, and number conversions
//! that must match the donor exactly: manifests sort, slice, and convert
//! with JavaScript rules, not Rust defaults.

use std::cmp::Ordering;

/// Compare strings by UTF-16 code unit (JavaScript `<`).
///
/// Rust byte order differs from JavaScript order for non-BMP characters,
/// so every donor `.sort()` on strings goes through this comparator.
#[must_use]
pub fn compare_text(left: &str, right: &str) -> Ordering {
    left.encode_utf16().cmp(right.encode_utf16())
}

/// Byte offsets where each TypeScript line starts.
///
/// Lines break on `\n`, `\r\n`, `\r`, `\u{2028}`, and `\u{2029}`, matching
/// the TypeScript scanner.
#[must_use]
pub fn line_starts(source: &str) -> Vec<usize> {
    let mut starts = vec![0];
    let bytes = source.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'\r' {
            if bytes.get(index + 1) == Some(&b'\n') {
                index += 1;
            }
            starts.push(index + 1);
        } else if bytes[index] == b'\n' {
            starts.push(index + 1);
        } else if bytes[index] == 0xE2 && bytes.get(index + 1) == Some(&0x80) && matches!(bytes.get(index + 2), Some(0xA8 | 0xA9)) {
            starts.push(index + 3);
            index += 2;
        }
        index += 1;
    }
    starts
}

/// 1-based line and column for a byte offset.
///
/// Columns count UTF-16 code units from the line start, matching
/// `getLineAndCharacterOfPosition`.
#[must_use]
pub fn line_col_utf16(source: &str, starts: &[usize], offset: usize) -> (usize, usize) {
    let line = starts.partition_point(|start| *start <= offset).max(1);
    let start = starts.get(line.saturating_sub(1)).copied().unwrap_or(0);
    let column = source.get(start..offset.min(source.len())).map_or(1, |text| text.encode_utf16().count() + 1);
    (line, column)
}

/// Longest head of `text` within `max_units` UTF-16 code units.
///
/// Never splits a surrogate pair: a character crossing the boundary is
/// omitted entirely.
#[must_use]
pub fn head_utf16(text: &str, max_units: usize) -> &str {
    let mut units = 0;
    let mut end = 0;
    for (index, ch) in text.char_indices() {
        units += ch.len_utf16();
        if units > max_units {
            break;
        }
        end = index + ch.len_utf8();
    }
    &text[..end]
}

/// Longest tail of `text` within `max_units` UTF-16 code units.
///
/// Never splits a surrogate pair: a character crossing the boundary is
/// omitted entirely.
#[must_use]
pub fn tail_utf16(text: &str, max_units: usize) -> &str {
    let mut units = 0;
    let mut start = text.len();
    for (index, ch) in text.char_indices().rev() {
        units += ch.len_utf16();
        if units > max_units {
            break;
        }
        start = index;
    }
    &text[start..]
}

/// Binary32 store: `Math.fround` round-to-nearest ties-to-even.
#[must_use]
pub fn fround(value: f64) -> f64 {
    f64::from(value as f32)
}

/// JavaScript `ToInt32`: truncate toward zero, then wrap modulo 2^32.
#[must_use]
pub fn to_int32(value: f64) -> i32 {
    if !value.is_finite() {
        return 0;
    }
    let truncated = value.trunc();
    let wrapped = ((truncated % 4_294_967_296.0) + 4_294_967_296.0) % 4_294_967_296.0;
    if wrapped >= 2_147_483_648.0 { (wrapped - 4_294_967_296.0) as i32 } else { wrapped as i32 }
}

/// JavaScript `ToUint32` (`>>> 0`): truncate toward zero, then wrap modulo 2^32.
#[must_use]
pub fn to_uint32(value: f64) -> u32 {
    if !value.is_finite() {
        return 0;
    }
    let truncated = value.trunc();
    (((truncated % 4_294_967_296.0) + 4_294_967_296.0) % 4_294_967_296.0) as u32
}

/// JavaScript `Math.min`: NaN propagates, `-0` beats `+0`.
#[must_use]
pub fn math_min(left: f64, right: f64) -> f64 {
    if left.is_nan() || right.is_nan() {
        f64::NAN
    } else if left == 0.0 && right == 0.0 {
        if left.is_sign_negative() || right.is_sign_negative() { -0.0 } else { 0.0 }
    } else {
        left.min(right)
    }
}

/// JavaScript `Math.max`: NaN propagates, `+0` beats `-0`.
#[must_use]
pub fn math_max(left: f64, right: f64) -> f64 {
    if left.is_nan() || right.is_nan() {
        f64::NAN
    } else if left == 0.0 && right == 0.0 {
        if left.is_sign_negative() && right.is_sign_negative() { -0.0 } else { 0.0 }
    } else {
        left.max(right)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn orders_non_bmp_by_code_unit() {
        assert_eq!(compare_text("a", "b"), Ordering::Less);
        assert_eq!(compare_text("b", "a"), Ordering::Greater);
        assert_eq!(compare_text("a", "a"), Ordering::Equal);
        assert_eq!(compare_text("\u{10000}", "\u{e000}"), Ordering::Less);
        assert_eq!(compare_text("\u{e000}", "\u{10000}"), Ordering::Greater);
    }

    #[test]
    fn splits_typescript_lines() {
        let source = "a\nb\r\nc\rd\u{2028}e\u{2029}f";
        assert_eq!(line_starts(source), vec![0, 2, 5, 7, 11, 15]);
        assert_eq!(line_col_utf16(source, &line_starts(source), 0), (1, 1));
        assert_eq!(line_col_utf16(source, &line_starts(source), 5), (3, 1));
    }

    #[test]
    fn counts_columns_in_utf16_units() {
        let source = "a\u{1f600}b";
        let starts = line_starts(source);
        assert_eq!(line_col_utf16(source, &starts, 1), (1, 2));
        assert_eq!(line_col_utf16(source, &starts, 5), (1, 4));
    }

    #[test]
    fn clamps_utf16_slices_at_boundaries() {
        assert_eq!(head_utf16("abcdef", 3), "abc");
        assert_eq!(head_utf16("a\u{1f600}b", 2), "a");
        assert_eq!(head_utf16("a\u{1f600}b", 3), "a\u{1f600}");
        assert_eq!(tail_utf16("abcdef", 3), "def");
        assert_eq!(tail_utf16("a\u{1f600}b", 2), "b");
        assert_eq!(tail_utf16("a\u{1f600}b", 3), "\u{1f600}b");
    }

    #[test]
    fn converts_js_integers() {
        assert_eq!(to_int32(3.9), 3);
        assert_eq!(to_int32(-3.9), -3);
        assert_eq!(to_int32(2_147_483_648.0), -2_147_483_648);
        assert_eq!(to_int32(4_294_967_295.0), -1);
        assert_eq!(to_int32(f64::NAN), 0);
        assert_eq!(to_int32(f64::INFINITY), 0);
        assert_eq!(to_uint32(-1.0), 4_294_967_295);
        assert_eq!(to_uint32(2_147_483_648.0), 2_147_483_648);
        assert_eq!(to_uint32(f64::NEG_INFINITY), 0);
    }

    #[test]
    fn propagates_nan_in_min_max() {
        assert!(math_min(f64::NAN, 1.0).is_nan());
        assert!(math_max(1.0, f64::NAN).is_nan());
        assert_eq!(math_min(-0.0, 0.0).to_bits(), (-0.0f64).to_bits());
        assert_eq!(math_max(-0.0, 0.0).to_bits(), 0.0f64.to_bits());
    }
}
