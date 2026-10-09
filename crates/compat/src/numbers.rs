//! Native signed conversion used by the original VM arithmetic instructions.
pub(crate) fn native_integer(value: f32) -> i32 {
    if (-2147483648.0..2147483648.0).contains(&value) {
        value as i32
    } else {
        i32::MIN
    }
}
