use crate::{FormatError, span, word};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BspFormat {
    Quake,
    Quake2,
    Quake3,
}

pub struct Bsp<'a> {
    pub format: BspFormat,
    lumps: [&'a [u8]; 19],
    strides: &'static [usize],
}

impl<'a> Bsp<'a> {
    pub fn parse(bytes: &'a [u8]) -> Result<Self, FormatError> {
        let first = word(bytes, 0)?;
        let (format, base, strides): (_, _, &'static [usize]) = match first {
            29 => (
                BspFormat::Quake,
                4,
                &[0, 20, 0, 12, 0, 24, 40, 20, 0, 8, 28, 2, 4, 4, 64],
            ),
            0x50534249 => match word(bytes, 4)? {
                38 => (
                    BspFormat::Quake2,
                    8,
                    &[
                        0, 20, 12, 0, 28, 76, 20, 0, 28, 2, 2, 4, 4, 48, 12, 4, 0, 8, 8,
                    ],
                ),
                46 => (
                    BspFormat::Quake3,
                    8,
                    &[
                        0, 72, 16, 36, 48, 4, 4, 40, 12, 8, 44, 4, 72, 104, 49152, 8, 0,
                    ],
                ),
                _ => return Err(FormatError::Unsupported),
            },
            _ => return Err(FormatError::Unsupported),
        };
        let mut lumps = [&[][..]; 19];
        for (index, stride) in strides.iter().enumerate() {
            let data = span(
                bytes,
                word(bytes, base + index * 8)?,
                word(bytes, base + index * 8 + 4)?,
            )?;
            if *stride != 0 && data.len() % stride != 0 {
                return Err(FormatError::InvalidRecordSize);
            }
            lumps[index] = data;
        }
        Ok(Self {
            format,
            lumps,
            strides,
        })
    }

    pub fn lump(&self, index: usize) -> Option<&'a [u8]> {
        self.strides.get(index).map(|_| self.lumps[index])
    }

    pub fn record_count(&self, index: usize) -> Option<usize> {
        self.strides
            .get(index)
            .filter(|stride| **stride != 0)
            .map(|stride| self.lumps[index].len() / stride)
    }
}

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
