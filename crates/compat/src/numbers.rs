//! Native signed conversion used by the original VM arithmetic instructions.
pub(crate) fn fits_native_integer(value: impl Into<f64>) -> bool {
    (-2147483648.0..2147483648.0).contains(&value.into())
}
pub(crate) fn native_integer(value: impl Into<f64>) -> i32 {
    let value = value.into();
    if fits_native_integer(value) {
        value as i32
    } else {
        i32::MIN
    }
}

pub(crate) fn fixed_float(
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
