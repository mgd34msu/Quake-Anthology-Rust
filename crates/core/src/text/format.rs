//! Native console number formatting.
use crate::math::native_integer;

pub fn native_number(
    text: &mut impl std::fmt::Write,
    value: f64,
    width: usize,
    precision: usize,
) -> std::fmt::Result {
    let integer = native_integer(value);
    if value == f64::from(integer) {
        write!(text, "{integer}")
    } else {
        fixed_float(text, value, width, precision)
    }
}

pub fn fixed_float(
    text: &mut impl std::fmt::Write,
    value: f64,
    width: usize,
    precision: usize,
) -> std::fmt::Result {
    if value.is_nan() {
        let label = if value.is_sign_negative() {
            "-nan"
        } else {
            "nan"
        };
        write!(text, "{label:>width$}")
    } else {
        write!(text, "{value:width$.precision$}")
    }
}
