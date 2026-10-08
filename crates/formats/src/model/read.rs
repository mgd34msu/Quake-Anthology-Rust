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
pub(super) fn empty_bounds() -> Bounds {
    Bounds {
        mins: Vec3([f32::INFINITY; 3]),
        maxs: Vec3([f32::NEG_INFINITY; 3]),
    }
}
pub(super) fn add_point(bounds: &mut Bounds, point: Vec3) {
    for a in 0..3 {
        bounds.mins.0[a] = bounds.mins.0[a].min(point.0[a]);
        bounds.maxs.0[a] = bounds.maxs.0[a].max(point.0[a]);
    }
}
pub(super) fn add_bounds(bounds: &mut Bounds, other: Bounds) {
    add_point(bounds, other.mins);
    add_point(bounds, other.maxs);
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
