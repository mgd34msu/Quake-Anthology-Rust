use crate::{FormatError, word};

#[derive(Debug, PartialEq, Eq)]
pub struct ModelHeader {
    pub skins: u32,
    pub vertices: u32,
    pub triangles: u32,
    pub frames: u32,
}

impl ModelHeader {
    pub fn parse(bytes: &[u8]) -> Result<Self, FormatError> {
        if bytes.len() < 84 {
            return Err(FormatError::Truncated);
        }
        if bytes.get(..4) != Some(b"IDPO") || word(bytes, 4)? != 6 {
            return Err(FormatError::Unsupported);
        }
        let result = Self {
            skins: word(bytes, 48)?,
            vertices: word(bytes, 60)?,
            triangles: word(bytes, 64)?,
            frames: word(bytes, 68)?,
        };
        if [
            result.skins,
            result.vertices,
            result.triangles,
            result.frames,
        ]
        .iter()
        .any(|v| *v == 0 || *v > i32::MAX as u32)
        {
            return Err(FormatError::InvalidRange);
        }
        Ok(result)
    }
}
