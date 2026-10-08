use super::*;

#[derive(Debug)]
pub struct Colormap<'a> {
    pub rows: &'a [[u8; 256]],
    pub first_fullbright: u16,
}
impl<'a> Colormap<'a> {
    pub fn parse(bytes: &'a [u8]) -> Result<Self, FormatError> {
        let data = bytes.get(..16385).ok_or(FormatError::Truncated)?;
        Ok(Self {
            rows: data[..16384].as_chunks::<256>().0,
            first_fullbright: 256 - u16::from(data[16384]),
        })
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MipFormat {
    Quake,
    Quake64,
    Wal,
}
#[derive(Clone, Copy, Debug)]
pub struct MipTexture<'a> {
    pub name: &'a [u8],
    pub width: u32,
    pub height: u32,
    pub shift: u32,
    /// Empty levels denote an external texture.
    pub levels: [&'a [u8]; 4],
    pub animation: &'a [u8],
    pub flags: i32,
    pub contents: i32,
    pub value: i32,
}
impl<'a> MipTexture<'a> {
    pub fn parse(bytes: &'a [u8], format: MipFormat) -> Result<Self, FormatError> {
        let wal = format == MipFormat::Wal;
        let mut r = Reader::new(bytes);
        let name = r.name(if wal { 32 } else { 16 })?;
        let width = r.u32()?;
        let height = r.u32()?;
        pixel_count(width, height)?;
        let shift = if format == MipFormat::Quake64 {
            r.u32()?
        } else {
            0
        };
        let offsets = [r.u32()?, r.u32()?, r.u32()?, r.u32()?];
        let (animation, flags, contents, value) = if wal {
            (r.name(32)?, r.i32()?, r.i32()?, r.i32()?)
        } else {
            (&[][..], 0, 0, 0)
        };
        let mut levels = [&[][..]; 4];
        for (mip, &offset) in offsets.iter().enumerate() {
            if offset == 0 && !wal {
                continue;
            }
            let w = (width >> mip).max(1);
            let h = (height >> mip).max(1);
            let length = pixel_count(w, h)?;
            levels[mip] = r.section(offset as usize, length, 1, r.at, bytes.len())?;
        }
        Ok(Self {
            name,
            width,
            height,
            shift,
            levels,
            animation,
            flags,
            contents,
            value,
        })
    }
}
pub(super) fn qpic(bytes: &[u8]) -> Result<IndexedImage<'_>, FormatError> {
    let mut r = Reader::new(bytes);
    let width = r.count(1, i32::MAX as usize)? as u32;
    let height = r.count(1, i32::MAX as usize)? as u32;
    let count = pixel_count(width, height)?;
    Ok(IndexedImage {
        width,
        height,
        indices: Cow::Borrowed(r.take(count)?),
        palette: None,
    })
}
pub(super) fn pcx(bytes: &[u8], policy: RasterPolicy) -> Result<DecodedImage<'_>, FormatError> {
    let mut r = Reader::new(bytes);
    let header = r.take(128)?;
    if header[..4] != [10, 5, 1, 8] {
        return Err(FormatError::Unsupported);
    }
    let mut bounds = Reader::new(&header[4..12]);
    let mut xmin = u32::from(bounds.u16()?);
    let mut ymin = u32::from(bounds.u16()?);
    let xmax = u32::from(bounds.u16()?);
    let ymax = u32::from(bounds.u16()?);
    let legacy = policy != RasterPolicy::Standard;
    if legacy {
        xmin = 0;
        ymin = 0;
    }
    let (limit_x, limit_y) = if policy == RasterPolicy::Quake2 {
        (640, 480)
    } else {
        (1024, 1024)
    };
    if xmax < xmin || ymax < ymin || (legacy && (xmax >= limit_x || ymax >= limit_y)) {
        return Err(FormatError::InvalidRange);
    }
    let width = xmax - xmin + 1;
    let height = ymax - ymin + 1;
    let count = pixel_count(width, height)?;
    let stride = if legacy {
        width as usize
    } else {
        Reader::new(&header[66..]).u16()? as usize
    };
    let palette_marker = bytes
        .len()
        .checked_sub(769)
        .filter(|&at| at >= 128 && bytes[at] == 12);
    let planes = if legacy || (header[65] == 0 && palette_marker.is_some()) {
        1
    } else {
        header[65] as usize
    };
    if ![1, 3].contains(&planes) || stride < width as usize {
        return Err(FormatError::Unsupported);
    }
    let palette_at = if legacy {
        Some(bytes.len().checked_sub(768).ok_or(FormatError::Truncated)?)
    } else if planes == 1 {
        palette_marker.map(|at| at + 1)
    } else {
        None
    };
    let encoded_end = if legacy {
        bytes.len()
    } else {
        palette_at.map_or(bytes.len(), |at| at - 1)
    };
    let row_size = stride * planes;
    if encoded_end < 128 || (row_size * height as usize).div_ceil(63) > encoded_end - 128 {
        return Err(FormatError::Truncated);
    }
    r.bytes = &bytes[..encoded_end];
    let mut decoded = vec![0; count * planes];
    for y in 0..height as usize {
        let mut x = 0;
        while x < row_size {
            let packet = r.u8()?;
            let (run, value) = if packet & 192 == 192 {
                ((packet & 63) as usize, r.u8()?)
            } else {
                (1, packet)
            };
            if run == 0 {
                return Err(FormatError::InvalidValue);
            }
            if legacy {
                let at = y * width as usize + x;
                decoded
                    .get_mut(at..at + run)
                    .ok_or(FormatError::InvalidRange)?
                    .fill(value);
            } else {
                if run > row_size - x {
                    return Err(FormatError::InvalidRange);
                }
                if planes == 1 {
                    if x < width as usize {
                        let at = y * width as usize + x;
                        decoded[at..at + run.min(width as usize - x)].fill(value);
                    }
                } else {
                    for byte in x..x + run {
                        let plane = byte / stride;
                        let column = byte % stride;
                        if column < width as usize {
                            decoded[(y * width as usize + column) * planes + plane] = value;
                        }
                    }
                }
            }
            x += run;
        }
    }
    if planes == 3 {
        let mut pixels = Vec::with_capacity(count * 4);
        for p in decoded.as_chunks::<3>().0 {
            pixels.extend_from_slice(&[p[0], p[1], p[2], 255]);
        }
        Ok(DecodedImage::Rgba(RgbaImage {
            width,
            height,
            pixels,
        }))
    } else {
        let palette = palette_at
            .map(|at| Palette::from_rgb(&bytes[at..]))
            .transpose()?;
        Ok(DecodedImage::Indexed(IndexedImage {
            width,
            height,
            indices: Cow::Owned(decoded),
            palette,
        }))
    }
}
