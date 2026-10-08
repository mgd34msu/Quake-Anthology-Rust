use super::*;

fn color(r: &mut Reader<'_>, bits: u8, descriptor: u8) -> Result<[u8; 4], FormatError> {
    let p = r.take(usize::from(bits).div_ceil(8))?;
    if bits == 15 || bits == 16 {
        let word = u16::from_le_bytes([p[0], p[1]]);
        let mut rgba = [0, 0, 0, 255];
        for (a, shift) in [10, 5, 0].into_iter().enumerate() {
            let v = ((word >> shift) & 31) as u8;
            rgba[a] = (v << 3) | (v >> 2);
        }
        if bits == 16 && descriptor & 15 != 0 && word & 32768 == 0 {
            rgba[3] = 0;
        }
        Ok(rgba)
    } else {
        Ok([p[2], p[1], p[0], if bits == 32 { p[3] } else { 255 }])
    }
}
pub(super) fn tga(bytes: &[u8], policy: RasterPolicy) -> Result<RgbaImage, FormatError> {
    let mut r = Reader::new(bytes);
    let h = r.take(18)?;
    let kind = h[2];
    let depth = h[16];
    let descriptor = h[17];
    let width = u32::from(u16::from_le_bytes([h[12], h[13]]));
    let height = u32::from(u16::from_le_bytes([h[14], h[15]]));
    let count = pixel_count(width, height)?;
    let indexed = kind == 1 || kind == 9;
    let gray = kind == 3 || kind == 11;
    let rle = kind >= 9;
    let legacy = policy == RasterPolicy::Quake3;
    if legacy {
        if ![2, 3, 10].contains(&kind)
            || h[1] != 0
            || !(depth == 24 || depth == 32 || (kind == 3 && depth == 8))
        {
            return Err(FormatError::Unsupported);
        }
    } else if ![1, 2, 3, 9, 10, 11].contains(&kind)
        || descriptor & 192 != 0
        || h[1] != u8::from(indexed)
        || if indexed || gray {
            ![8, 16].contains(&depth)
        } else {
            ![16, 24, 32].contains(&depth)
        }
    {
        return Err(FormatError::Unsupported);
    }
    r.take(h[0] as usize)?;
    let first = usize::from(u16::from_le_bytes([h[3], h[4]]));
    let colors = if indexed {
        usize::from(u16::from_le_bytes([h[5], h[6]]))
    } else {
        0
    };
    let mut palette = Vec::new();
    if indexed {
        if ![15, 16, 24, 32].contains(&h[7]) || colors == 0 {
            return Err(FormatError::Unsupported);
        }
        r.remaining_records(colors, usize::from(h[7]).div_ceil(8))?;
        palette.reserve_exact(colors);
        for _ in 0..colors {
            palette.push(color(&mut r, h[7], descriptor)?);
        }
    }
    let minimum = if rle {
        count.div_ceil(128) * (usize::from(depth) / 8 + 1)
    } else {
        count * (usize::from(depth) / 8)
    };
    r.remaining_records(minimum, 1)?;
    let mut pixels = vec![0; count * 4];
    let mut at = 0;
    while at < count {
        let packet = if rle { r.u8()? } else { 0 };
        let run = if rle {
            usize::from(packet & 127) + 1
        } else {
            1
        };
        if run > count - at && !legacy {
            return Err(FormatError::InvalidRange);
        }
        let run = run.min(count - at);
        let mut rgba = [0; 4];
        for i in 0..run {
            if i == 0 || packet & 128 == 0 {
                rgba = if indexed {
                    let index = if depth == 8 {
                        usize::from(r.u8()?)
                    } else {
                        usize::from(r.u16()?)
                    };
                    *palette
                        .get(index.checked_sub(first).ok_or(FormatError::InvalidRange)?)
                        .ok_or(FormatError::InvalidRange)?
                } else if gray {
                    let value = r.u8()?;
                    [value, value, value, if depth == 16 { r.u8()? } else { 255 }]
                } else {
                    color(&mut r, depth, descriptor)?
                };
            }
            let mut x = (at % width as usize) as u32;
            let mut y = (at / width as usize) as u32;
            if !legacy && descriptor & 16 != 0 {
                x = width - x - 1;
            }
            if legacy || descriptor & 32 == 0 {
                y = height - y - 1;
            }
            let dest = (y as usize * width as usize + x as usize) * 4;
            pixels[dest..dest + 4].copy_from_slice(&rgba);
            at += 1;
        }
    }
    Ok(RgbaImage {
        width,
        height,
        pixels,
    })
}
pub(super) fn bmp(bytes: &[u8], policy: RasterPolicy) -> Result<RgbaImage, FormatError> {
    let mut r = Reader::new(bytes);
    if r.take(2)? != b"BM" {
        return Err(FormatError::Unsupported);
    }
    let file_length = r.u32()? as usize;
    r.take(4)?;
    let pixel_at = r.u32()? as usize;
    let header = r.u32()?;
    let width = r.count(1, i32::MAX as usize)? as u32;
    let signed_height = r.i32()?;
    if signed_height == i32::MIN || signed_height == 0 {
        return Err(FormatError::InvalidRange);
    }
    let height = signed_height.unsigned_abs();
    let count = pixel_count(width, height)?;
    let planes = r.u16()?;
    let depth = r.u16()?;
    let compression = r.u32()?;
    r.take(12)?;
    let colors = r.u32()? as usize;
    r.take(4)?;
    let legacy = policy == RasterPolicy::Quake3;
    if ![8, 24, 32].contains(&depth)
        || compression != 0
        || (!legacy && (header != 40 || planes != 1))
        || (legacy && file_length != bytes.len())
    {
        return Err(FormatError::Unsupported);
    }
    let palette_count = if depth == 8 {
        if legacy || colors == 0 { 256 } else { colors }
    } else {
        0
    };
    if palette_count > 256 {
        return Err(FormatError::InvalidRange);
    }
    let mut palette = Vec::new();
    r.remaining_records(palette_count, 4)?;
    for _ in 0..palette_count {
        let p = r.take(4)?;
        palette.push([p[2], p[1], p[0], 255]);
    }
    let offset = if legacy { r.at } else { pixel_at };
    let row = width as usize * (usize::from(depth) / 8);
    let stride = if legacy { row } else { (row + 3) & !3 };
    let data = r.section(offset, height as usize, stride, r.at, bytes.len())?;
    let mut pixels = vec![0; count * 4];
    for (source_y, row) in data.chunks_exact(stride).enumerate() {
        let y = if signed_height < 0 {
            source_y
        } else {
            height as usize - source_y - 1
        };
        for x in 0..width as usize {
            let p = &row[x * usize::from(depth) / 8..];
            let rgba = if depth == 8 {
                *palette
                    .get(usize::from(p[0]))
                    .ok_or(FormatError::InvalidRange)?
            } else {
                [p[2], p[1], p[0], if depth == 32 { p[3] } else { 255 }]
            };
            let dest = (y * width as usize + x) * 4;
            pixels[dest..dest + 4].copy_from_slice(&rgba);
        }
    }
    Ok(RgbaImage {
        width,
        height,
        pixels,
    })
}
