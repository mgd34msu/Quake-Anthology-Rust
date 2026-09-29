//! GIF decoder (owned pixels).
//!
//! Donor: `decodeGif` in `src/formats/images/gif.ts`
//! (adapted from quake-2-re-ts `qcommon/gif.ts`, GPL-2.0-or-later).
//! Disposal 2 composites to transparency, matching the rerelease port.

use qa_core::binary::BinaryReader;

use super::{fail, ContentError, ImageLevel};

/// GIF frame rectangle (`rect`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GifRect {
    /// Left offset.
    pub x: u16,
    /// Top offset.
    pub y: u16,
    /// Frame width.
    pub width: u16,
    /// Frame height.
    pub height: u16,
}

/// Decoded GIF frame (`GifFrame`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GifFrame {
    /// Full-canvas composited image.
    pub image: ImageLevel,
    /// Frame rectangle.
    pub rect: GifRect,
    /// De-interlaced frame indices (`rect.width * rect.height`).
    pub indices: Vec<u8>,
    /// Frame palette copy.
    pub palette: Vec<u8>,
    /// Transparent index, when present.
    pub transparent_index: Option<u8>,
    /// Frame delay in centiseconds.
    pub delay_centiseconds: u16,
    /// Disposal method.
    pub disposal: u8,
}

/// Decoded GIF image (`GifImage`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GifImage {
    /// Logical screen width.
    pub width: u16,
    /// Logical screen height.
    pub height: u16,
    /// Frames (at least one).
    pub frames: Vec<GifFrame>,
    /// NETSCAPE/ANIMEXTS loop count, when present.
    pub loop_count: Option<u16>,
    /// Background color index.
    pub background_index: u8,
    /// Global palette, when present.
    pub global_palette: Option<Vec<u8>>,
}

fn sub_blocks(reader: &mut BinaryReader<'_>) -> Result<Vec<u8>, ContentError> {
    let mut bytes = Vec::new();
    loop {
        let count = reader.u8()?;
        if count == 0 {
            break;
        }
        bytes.extend_from_slice(&reader.bytes(usize::from(count))?);
    }
    Ok(bytes)
}

fn lzw(data: &[u8], minimum: u8, count: usize, source: &str) -> Result<Vec<u8>, ContentError> {
    if !(2..=8).contains(&minimum) {
        return Err(fail(source, 0, "Invalid GIF LZW code size".to_string()));
    }
    let clear = 1usize << minimum;
    let end = clear + 1;
    let mut output = vec![0u8; count];
    let mut dictionary: Vec<Vec<u8>> = Vec::new();
    let mut size = usize::from(minimum) + 1;
    let mut next = end + 1;
    let mut bit = 0usize;
    let mut written = 0usize;
    let mut previous: Option<Vec<u8>> = None;
    let reset = |dictionary: &mut Vec<Vec<u8>>, size: &mut usize, next: &mut usize, previous: &mut Option<Vec<u8>>| {
        dictionary.clear();
        for index in 0..clear {
            dictionary.push(vec![index as u8]);
        }
        dictionary.push(Vec::new());
        dictionary.push(Vec::new());
        *size = usize::from(minimum) + 1;
        *next = end + 1;
        *previous = None;
    };
    reset(&mut dictionary, &mut size, &mut next, &mut previous);
    while written < count {
        if bit + size > data.len() * 8 {
            return Err(fail(source, bit >> 3, "Truncated GIF LZW data".to_string()));
        }
        let mut code = 0usize;
        for index in 0..size {
            code |= (usize::from(data[bit >> 3] >> (bit & 7) & 1)) << index;
            bit += 1;
        }
        if code == clear {
            reset(&mut dictionary, &mut size, &mut next, &mut previous);
            continue;
        }
        if code == end {
            break;
        }
        let mut entry = dictionary.get(code).cloned();
        if entry.is_none() && code == next {
            if let Some(previous) = previous.as_ref() {
                if let Some(first) = previous.first().copied() {
                    let mut grown = previous.clone();
                    grown.push(first);
                    entry = Some(grown);
                }
            }
        }
        let first = entry.as_ref().and_then(|entry| entry.first().copied());
        match (&entry, first) {
            (Some(entry), Some(_)) if entry.len() <= count - written => {}
            _ => return Err(fail(source, bit >> 3, "Invalid GIF LZW code".to_string())),
        }
        let entry = entry.unwrap_or_default();
        let first = first.unwrap_or_default();
        output[written..written + entry.len()].copy_from_slice(&entry);
        written += entry.len();
        if previous.is_some() && next < 4096 {
            let mut grown = previous.clone().unwrap_or_default();
            grown.push(first);
            dictionary.push(grown);
            next += 1;
            if next == 1usize << size && size < 12 {
                size += 1;
            }
        }
        previous = Some(entry);
    }
    if written != count {
        return Err(fail(source, bit >> 3, "Incomplete GIF image".to_string()));
    }
    Ok(output)
}

/// Decode a GIF image (`decodeGif`).
pub fn decode_gif(bytes: &[u8], source: &str) -> Result<GifImage, ContentError> {
    let mut reader = BinaryReader::new(bytes, source);
    let signature = reader.fixed_byte_string(6)?;
    if signature != "GIF87a" && signature != "GIF89a" {
        return Err(fail(source, 0, "Invalid GIF signature".to_string()));
    }
    let width = reader.u16()?;
    let height = reader.u16()?;
    let packed = reader.u8()?;
    let background_index = reader.u8()?;
    reader.skip(1)?;
    if width == 0 || height == 0 || u64::from(width) * u64::from(height) * 4 > 0x7fff_ffff {
        return Err(fail(source, 6, "Invalid GIF dimensions".to_string()));
    }
    let width_usize = usize::from(width);
    let height_usize = usize::from(height);
    let global_palette = if packed & 128 == 0 {
        None
    } else {
        Some(reader.bytes((2usize << (packed & 7)) * 3)?)
    };
    let mut canvas = vec![0u8; width_usize * height_usize * 4];
    let mut frames: Vec<GifFrame> = Vec::new();
    let mut transparent_index: Option<u8> = None;
    let mut delay_centiseconds = 0u16;
    let mut disposal = 0u8;
    let mut loop_count: Option<u16> = None;
    loop {
        let block = reader.u8()?;
        if block == 59 {
            break;
        }
        if block == 33 {
            let label = reader.u8()?;
            if label == 249 {
                if reader.u8()? != 4 {
                    return Err(fail(
                        source,
                        reader.offset(),
                        "Invalid GIF graphic control extension".to_string(),
                    ));
                }
                let flags = reader.u8()?;
                delay_centiseconds = reader.u16()?;
                let index = reader.u8()?;
                transparent_index = if flags & 1 == 0 { None } else { Some(index) };
                disposal = (flags >> 2) & 7;
                if reader.u8()? != 0 {
                    return Err(fail(
                        source,
                        reader.offset(),
                        "Invalid GIF extension terminator".to_string(),
                    ));
                }
            } else if label == 255 {
                let app_length = reader.u8()?;
                let application = reader.fixed_byte_string(usize::from(app_length))?;
                let payload = sub_blocks(&mut reader)?;
                if (application == "NETSCAPE2.0" || application == "ANIMEXTS1.0")
                    && payload.len() >= 3
                    && payload[0] == 1
                {
                    loop_count = Some(u16::from_le_bytes([payload[1], payload[2]]));
                }
            } else {
                sub_blocks(&mut reader)?;
            }
            continue;
        }
        if block != 44 {
            return Err(fail(source, reader.offset() - 1, format!("Unknown GIF block {block}")));
        }
        let x = reader.u16()?;
        let y = reader.u16()?;
        let frame_width = reader.u16()?;
        let frame_height = reader.u16()?;
        let flags = reader.u8()?;
        if frame_width == 0
            || frame_height == 0
            || u32::from(x) + u32::from(frame_width) > u32::from(width)
            || u32::from(y) + u32::from(frame_height) > u32::from(height)
        {
            return Err(fail(
                source,
                reader.offset(),
                "GIF frame exceeds its logical screen".to_string(),
            ));
        }
        let palette = if flags & 128 == 0 {
            global_palette.clone()
        } else {
            Some(reader.bytes((2usize << (flags & 7)) * 3)?)
        };
        let palette = match palette {
            Some(palette) => palette,
            None => return Err(fail(source, reader.offset(), "GIF frame has no palette".to_string())),
        };
        let minimum = reader.u8()?;
        let encoded = sub_blocks(&mut reader)?;
        let decoded = lzw(
            &encoded,
            minimum,
            usize::from(frame_width) * usize::from(frame_height),
            source,
        )?;
        let frame_height_usize = usize::from(frame_height);
        let frame_width_usize = usize::from(frame_width);
        let mut rows: Vec<usize> = Vec::with_capacity(frame_height_usize);
        if flags & 64 == 0 {
            for row in 0..frame_height_usize {
                rows.push(row);
            }
        } else {
            for (start, step) in [(0usize, 8usize), (4, 8), (2, 4), (1, 2)] {
                let mut row = start;
                while row < frame_height_usize {
                    rows.push(row);
                    row += step;
                }
            }
        }
        let mut indices = vec![0u8; decoded.len()];
        let previous = if disposal == 3 { Some(canvas.clone()) } else { None };
        for row in 0..frame_height_usize {
            let destination_y = match rows.get(row).copied() {
                Some(row) => row,
                None => return Err(fail(source, reader.offset(), "Incomplete GIF interlace".to_string())),
            };
            for column in 0..frame_width_usize {
                let index = decoded[row * frame_width_usize + column];
                if usize::from(index) * 3 + 3 > palette.len() {
                    return Err(fail(
                        source,
                        reader.offset(),
                        "GIF palette index out of range".to_string(),
                    ));
                }
                indices[destination_y * frame_width_usize + column] = index;
                if Some(index) != transparent_index {
                    let base = usize::from(index) * 3;
                    let at = ((usize::from(y) + destination_y) * width_usize + usize::from(x) + column) * 4;
                    canvas[at] = palette[base];
                    canvas[at + 1] = palette[base + 1];
                    canvas[at + 2] = palette[base + 2];
                    canvas[at + 3] = 255;
                }
            }
        }
        frames.push(GifFrame {
            image: ImageLevel {
                width: u32::from(width),
                height: u32::from(height),
                pixels: canvas.clone(),
            },
            rect: GifRect {
                x,
                y,
                width: frame_width,
                height: frame_height,
            },
            indices,
            palette: palette.clone(),
            transparent_index,
            delay_centiseconds,
            disposal,
        });
        if disposal == 2 {
            for row in usize::from(y)..usize::from(y) + frame_height_usize {
                let from = (row * width_usize + usize::from(x)) * 4;
                let to = (row * width_usize + usize::from(x) + frame_width_usize) * 4;
                canvas[from..to].fill(0);
            }
        } else if let Some(previous) = previous {
            canvas = previous;
        }
        transparent_index = None;
        delay_centiseconds = 0;
        disposal = 0;
    }
    if frames.is_empty() {
        return Err(fail(source, reader.offset(), "GIF has no image frames".to_string()));
    }
    Ok(GifImage {
        width,
        height,
        frames,
        loop_count,
        background_index,
        global_palette,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn lzw_encode(minimum: u8, pixels: &[u8]) -> Vec<u8> {
        let clear = 1usize << minimum;
        let end = clear + 1;
        let mut bytes = Vec::new();
        let mut acc = 0u32;
        let mut acc_bits = 0u32;
        let mut emit = |code: usize, size: u32| {
            acc |= (code as u32) << acc_bits;
            acc_bits += size;
            while acc_bits >= 8 {
                bytes.push((acc & 0xff) as u8);
                acc >>= 8;
                acc_bits -= 8;
            }
        };
        let mut table: HashMap<Vec<u8>, usize> = HashMap::new();
        for index in 0..clear {
            table.insert(vec![index as u8], index);
        }
        let mut size = u32::from(minimum) + 1;
        let mut next = end + 1;
        emit(clear, size);
        let mut prefix: Vec<u8> = Vec::new();
        for &pixel in pixels {
            let mut grown = prefix.clone();
            grown.push(pixel);
            if table.contains_key(&grown) {
                prefix = grown;
                continue;
            }
            emit(table[&prefix], size);
            if next < 4096 {
                table.insert(grown, next);
                next += 1;
                // Deferred one entry: the decoder bumps only after reading
                // the code that follows this addition.
                if next == (1usize << size) + 1 && size < 12 {
                    size += 1;
                }
            } else {
                table.retain(|key, _| key.len() == 1);
                emit(clear, size);
                size = u32::from(minimum) + 1;
                next = end + 1;
            }
            prefix = vec![pixel];
        }
        if !prefix.is_empty() {
            emit(table[&prefix], size);
        }
        emit(end, size);
        if acc_bits > 0 {
            bytes.push((acc & 0xff) as u8);
        }
        bytes
    }

    fn gif_bytes(width: u16, height: u16, global: Option<&[u8]>, blocks: &[Vec<u8>], trailer: bool) -> Vec<u8> {
        let mut out = b"GIF89a".to_vec();
        out.extend_from_slice(&width.to_le_bytes());
        out.extend_from_slice(&height.to_le_bytes());
        if let Some(palette) = global {
            let colors = palette.len() / 3;
            let size_field = (colors.trailing_zeros() - 1) as u8;
            out.push(0x80 | size_field);
            out.push(0);
            out.push(0);
            out.extend_from_slice(palette);
        } else {
            out.extend_from_slice(&[0, 0, 0]);
        }
        for block in blocks {
            out.extend_from_slice(block);
        }
        if trailer {
            out.push(59);
        }
        out
    }

    fn image_block(x: u16, y: u16, w: u16, h: u16, flags: u8, minimum: u8, data: &[u8]) -> Vec<u8> {
        let mut out = vec![44];
        out.extend_from_slice(&x.to_le_bytes());
        out.extend_from_slice(&y.to_le_bytes());
        out.extend_from_slice(&w.to_le_bytes());
        out.extend_from_slice(&h.to_le_bytes());
        out.push(flags);
        out.push(minimum);
        for chunk in data.chunks(255) {
            out.push(chunk.len() as u8);
            out.extend_from_slice(chunk);
        }
        out.push(0);
        out
    }

    fn gce(disposal: u8, transparent: Option<u8>, delay: u16) -> Vec<u8> {
        let flags = (disposal << 2) | u8::from(transparent.is_some());
        vec![
            33,
            249,
            4,
            flags,
            (delay & 0xff) as u8,
            (delay >> 8) as u8,
            transparent.unwrap_or(0),
            0,
        ]
    }

    #[test]
    fn decodes_single_frame() {
        let palette = [255, 0, 0, 0, 255, 0];
        let data = lzw_encode(2, &[1, 0]);
        let block = image_block(0, 0, 2, 1, 0, 2, &data);
        let bytes = gif_bytes(2, 1, Some(&palette), &[block], true);
        let image = decode_gif(&bytes, "<test>").unwrap();
        assert_eq!((image.width, image.height), (2, 1));
        assert_eq!(image.frames.len(), 1);
        assert_eq!(image.frames[0].indices, vec![1, 0]);
        assert_eq!(image.frames[0].image.pixels, vec![0, 255, 0, 255, 255, 0, 0, 255]);
        assert_eq!(image.loop_count, None);
    }

    #[test]
    fn transparent_disposal_two_clears() {
        let palette = [255, 0, 0, 0, 255, 0];
        let first = [
            gce(2, Some(0), 5),
            image_block(0, 0, 2, 1, 0, 2, &lzw_encode(2, &[1, 0])),
        ]
        .concat();
        let second = image_block(0, 0, 2, 1, 0, 2, &lzw_encode(2, &[0, 0]));
        let bytes = gif_bytes(2, 1, Some(&palette), &[first, second], true);
        let image = decode_gif(&bytes, "<test>").unwrap();
        assert_eq!(image.frames.len(), 2);
        assert_eq!(image.frames[0].transparent_index, Some(0));
        assert_eq!(image.frames[0].delay_centiseconds, 5);
        assert_eq!(image.frames[0].disposal, 2);
        // First frame: index 0 transparent, second pixel red-opaque on canvas.
        assert_eq!(image.frames[0].image.pixels, vec![0, 255, 0, 255, 0, 0, 0, 0]);
        // Disposal 2 cleared the region before the second frame composited.
        assert_eq!(image.frames[1].image.pixels, vec![255, 0, 0, 255, 255, 0, 0, 255]);
    }

    #[test]
    fn decodes_interlaced_rows_in_order() {
        let palette = [10, 0, 0, 20, 0, 0];
        // 1x4 rows [0,1,0,1]; interlace stores pass order rows 0,2 then 1,3.
        let stored = [0, 0, 1, 1];
        let data = lzw_encode(2, &stored);
        let block = image_block(0, 0, 1, 4, 64, 2, &data);
        let bytes = gif_bytes(1, 4, Some(&palette), &[block], true);
        let image = decode_gif(&bytes, "<test>").unwrap();
        assert_eq!(image.frames[0].indices, vec![0, 1, 0, 1]);
    }

    #[test]
    fn reads_loop_extension() {
        let palette = [1, 2, 3, 4, 5, 6];
        let app = vec![
            33, 255, 11, b'N', b'E', b'T', b'S', b'C', b'A', b'P', b'E', b'2', b'.', b'0', 3, 1, 7, 0, 0,
        ];
        let block = image_block(0, 0, 1, 1, 0, 2, &lzw_encode(2, &[0]));
        let bytes = gif_bytes(1, 1, Some(&palette), &[app, block], true);
        let image = decode_gif(&bytes, "<test>").unwrap();
        assert_eq!(image.loop_count, Some(7));
    }

    #[test]
    fn rejects_lzw_errors_and_empty() {
        // Truncated LZW data.
        let palette = [1, 2, 3, 4, 5, 6];
        let block = image_block(0, 0, 1, 1, 0, 2, &[]);
        let bytes = gif_bytes(1, 1, Some(&palette), &[block], true);
        let error = decode_gif(&bytes, "<test>").unwrap_err();
        assert_eq!(error.message, "Truncated GIF LZW data");

        // No frames.
        let bytes = gif_bytes(1, 1, Some(&palette), &[], true);
        let error = decode_gif(&bytes, "<test>").unwrap_err();
        assert_eq!(error.message, "GIF has no image frames");

        // Bad signature.
        let mut bytes = gif_bytes(1, 1, Some(&palette), &[], true);
        bytes[0] = b'X';
        let error = decode_gif(&bytes, "<test>").unwrap_err();
        assert_eq!(error.message, "Invalid GIF signature");
    }
}
