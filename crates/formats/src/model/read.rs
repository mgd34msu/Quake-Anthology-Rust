use super::*;

pub(super) const MAX_DECODED_BYTES: usize = 256 * 1024 * 1024;

pub(super) use crate::read::Reader;
pub(super) fn finite(value: f32) -> Result<(), FormatError> {
    if value.is_finite() {
        Ok(())
    } else {
        Err(FormatError::InvalidValue)
    }
}
pub(super) fn finite_vec(value: Vec3) -> Result<(), FormatError> {
    for v in value.0 {
        finite(v)?;
    }
    Ok(())
}
pub(super) fn bounded_index(index: usize, count: usize) -> Result<u32, FormatError> {
    if index >= count {
        Err(FormatError::InvalidReference("model index", index))
    } else {
        Ok(index as u32)
    }
}
pub(super) fn reserve<T>(values: &mut Vec<T>, additional: usize) -> Result<(), FormatError> {
    values
        .try_reserve_exact(additional)
        .map_err(|_| FormatError::InvalidRange)
}
