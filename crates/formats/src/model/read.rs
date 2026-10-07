use super::*;

pub(super) const MAX_DECODED_BYTES: usize = 256 * 1024 * 1024;

pub(super) struct Reader<'a> {
    pub bytes: &'a [u8],
    pub at: usize,
}
impl<'a> Reader<'a> {
    pub fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, at: 0 }
    }
    pub fn take(&mut self, n: usize) -> Result<&'a [u8], FormatError> {
        let end = self.at.checked_add(n).ok_or(FormatError::InvalidRange)?;
        let bytes = self.bytes.get(self.at..end).ok_or(FormatError::Truncated)?;
        self.at = end;
        Ok(bytes)
    }
    pub fn i16(&mut self) -> Result<i16, FormatError> {
        Ok(i16::from_le_bytes(
            self.take(2)?
                .try_into()
                .map_err(|_| FormatError::Truncated)?,
        ))
    }
    pub fn u32(&mut self) -> Result<u32, FormatError> {
        Ok(u32::from_le_bytes(
            self.take(4)?
                .try_into()
                .map_err(|_| FormatError::Truncated)?,
        ))
    }
    pub fn i32(&mut self) -> Result<i32, FormatError> {
        Ok(self.u32()? as i32)
    }
    pub fn count(&mut self, min: usize, max: usize) -> Result<usize, FormatError> {
        let value = self.i32()?;
        if value < 0 || (value as usize) < min || value as usize > max {
            return Err(FormatError::InvalidRange);
        }
        Ok(value as usize)
    }
    pub fn float(&mut self) -> Result<f32, FormatError> {
        let value = f32::from_bits(self.u32()?);
        finite(value)?;
        Ok(value)
    }
    pub fn vec3(&mut self) -> Result<Vec3, FormatError> {
        Ok(Vec3([self.float()?, self.float()?, self.float()?]))
    }
    pub fn name(&mut self, length: usize) -> Result<&'a [u8], FormatError> {
        let bytes = self.take(length)?;
        Ok(&bytes[..bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len())])
    }
    pub fn section(
        &self,
        offset: usize,
        count: usize,
        stride: usize,
        header: usize,
        end: usize,
    ) -> Result<&'a [u8], FormatError> {
        if offset < header || end > self.bytes.len() || end < header {
            return Err(FormatError::InvalidRange);
        }
        let limit = offset
            .checked_add(count.checked_mul(stride).ok_or(FormatError::InvalidRange)?)
            .ok_or(FormatError::InvalidRange)?;
        if limit > end {
            return Err(FormatError::InvalidRange);
        }
        self.bytes
            .get(offset..limit)
            .ok_or(FormatError::InvalidRange)
    }
    pub fn remaining_records(&self, count: usize, stride: usize) -> Result<(), FormatError> {
        self.section(self.at, count, stride, 0, self.bytes.len())?;
        Ok(())
    }
}
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
