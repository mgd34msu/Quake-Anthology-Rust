use crate::FormatError;
use qa_core::primitives::Vec3;

pub(crate) struct Reader<'a> {
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
    pub fn u8(&mut self) -> Result<u8, FormatError> {
        Ok(self.take(1)?[0])
    }
    pub fn u16(&mut self) -> Result<u16, FormatError> {
        Ok(u16::from_le_bytes(
            self.take(2)?
                .try_into()
                .map_err(|_| FormatError::Truncated)?,
        ))
    }
    pub fn raw_float(&mut self) -> Result<f32, FormatError> {
        Ok(f32::from_bits(self.u32()?))
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
        if !value.is_finite() {
            return Err(FormatError::InvalidValue);
        }
        Ok(value)
    }
    pub fn vector(&mut self) -> Result<Vec3, FormatError> {
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
