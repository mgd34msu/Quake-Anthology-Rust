//! Native signed conversion used by the original VM arithmetic instructions.
pub(crate) fn native_integer(value: impl Into<f64>) -> i32 {
    let value = value.into();
    if (-2147483648.0..2147483648.0).contains(&value) {
        value as i32
    } else {
        i32::MIN
    }
}
