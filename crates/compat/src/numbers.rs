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
