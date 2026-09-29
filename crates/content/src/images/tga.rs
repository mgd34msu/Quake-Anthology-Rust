//! Format-oriented TGA decoder and encoder (owned pixels).
//!
//! Donor: `decodeTga`/`encodeTga` in `src/formats/images/tga.ts`
//! (adapted from quake-1-re-ts and Q3 LoadTGA, GPL-2.0-or-later).

use qa_core::binary::BinaryReader;

use super::{fail, ContentError, ImageLevel};

/// Indexed TGA pixels plus their RGBA palette (`indexed`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TgaIndexed {
    /// Row-major palette indices (`width * height`).
    pub indices: Vec<u16>,
    /// First palette entry (`mapFirst`).
    pub first: u16,
    /// Palette entries as RGBA bytes.
    pub palette_rgba: Vec<u8>,
}

/// Decoded TGA image (`TgaImage`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TgaImage {
    /// Width in pixels.
    pub width: u16,
    /// Height in pixels.
    pub height: u16,
    /// Row-major RGBA bytes (`width * height * 4`).
    pub pixels: Vec<u8>,
    /// Image descriptor byte.
    pub descriptor: u8,
    /// Indexed pixels and palette, when present.
    pub indexed: Option<TgaIndexed>,
}

fn read_color(reader: &mut BinaryReader<'_>, bits: u8, descriptor: u8, source: &str) -> Result<[u8; 4], ContentError> {
    if bits == 15 || bits == 16 {
        let word = reader.u16()?;
        let r = ((word >> 10) & 31) as u8;
        let g = ((word >> 5) & 31) as u8;
        let b = (word & 31) as u8;
        let alpha = if bits == 16 && descriptor & 15 != 0 && word & 32768 == 0 {
            0
        } else {
            255
        };
        return Ok([(r << 3) | (r >> 2), (g << 3) | (g >> 2), (b << 3) | (b >> 2), alpha]);
    }
    if bits != 24 && bits != 32 {
        return Err(fail(
            source,
            reader.offset(),
            format!("Unsupported TGA color depth {bits}"),
        ));
    }
    let b = reader.u8()?;
    let g = reader.u8()?;
    let r = reader.u8()?;
    Ok([r, g, b, if bits == 32 { reader.u8()? } else { 255 }])
}

// Donor shape: mirrors the TGA pixel reader's operands one-to-one.
#[allow(clippy::too_many_arguments)]
fn read_pixel(
    reader: &mut BinaryReader<'_>,
    indexed: bool,
    gray: bool,
    depth: u8,
    palette: &[[u8; 4]],
    map_first: u16,
    descriptor: u8,
    source: &str,
) -> Result<([u8; 4], u16), ContentError> {
    if indexed {
        let index = if depth == 8 {
            u16::from(reader.u8()?)
        } else {
            reader.u16()?
        };
        let entry = index
            .checked_sub(map_first)
            .and_then(|at| palette.get(at as usize))
            .copied();
        match entry {
            Some(value) => Ok((value, index)),
            None => Err(fail(
                source,
                reader.offset(),
                "TGA palette index out of range".to_string(),
            )),
        }
    } else if gray {
        let value = reader.u8()?;
        Ok(([value, value, value, if depth == 16 { reader.u8()? } else { 255 }], 0))
    } else {
        Ok((read_color(reader, depth, descriptor, source)?, 0))
    }
}

/// Decode a TGA image (`decodeTga`).
pub fn decode_tga(bytes: &[u8], source: &str) -> Result<TgaImage, ContentError> {
    let mut reader = BinaryReader::new(bytes, source);
    let id_length = reader.u8()?;
    let map_type = reader.u8()?;
    let image_type = reader.u8()?;
    let map_first = reader.u16()?;
    let map_length = reader.u16()?;
    let map_depth = reader.u8()?;
    reader.skip(4)?;
    let width = reader.u16()?;
    let height = reader.u16()?;
    let depth = reader.u8()?;
    let descriptor = reader.u8()?;
    let indexed = image_type == 1 || image_type == 9;
    let gray = image_type == 3 || image_type == 11;
    let rle = image_type >= 9;
    let count = usize::from(width) * usize::from(height);
    let depth_ok = if indexed || gray {
        depth == 8 || depth == 16
    } else {
        depth == 16 || depth == 24 || depth == 32
    };
    if ![1u8, 2, 3, 9, 10, 11].contains(&image_type)
        || width == 0
        || height == 0
        || count as u64 * 4 > 0x7fff_ffff
        || descriptor & 192 != 0
        || map_type != u8::from(indexed)
        || !depth_ok
    {
        return Err(fail(source, 0, "Unsupported TGA header".to_string()));
    }
    reader.skip(usize::from(id_length))?;
    let mut palette = Vec::with_capacity(usize::from(map_length));
    if indexed {
        for _ in 0..map_length {
            palette.push(read_color(&mut reader, map_depth, descriptor, source)?);
        }
    }
    let mut pixels = vec![0u8; count * 4];
    let mut indices = indexed.then(|| vec![0u16; count]);
    let width_usize = usize::from(width);
    let height_usize = usize::from(height);
    let mut pixel = 0usize;
    while pixel < count {
        let packet = if rle { reader.u8()? } else { 0 };
        let run = if rle { usize::from(packet & 127) + 1 } else { 1 };
        if run > count - pixel {
            return Err(fail(
                source,
                reader.offset(),
                "TGA packet exceeds pixel count".to_string(),
            ));
        }
        if packet & 128 != 0 {
            let (rgba, current) = read_pixel(
                &mut reader,
                indexed,
                gray,
                depth,
                &palette,
                map_first,
                descriptor,
                source,
            )?;
            for _ in 0..run {
                let x = pixel % width_usize;
                let y = pixel / width_usize;
                let dx = if descriptor & 16 == 0 { x } else { width_usize - 1 - x };
                let dy = if descriptor & 32 == 0 { height_usize - 1 - y } else { y };
                let at = dy * width_usize + dx;
                pixels[at * 4..at * 4 + 4].copy_from_slice(&rgba);
                if let Some(store) = indices.as_mut() {
                    store[at] = current;
                }
                pixel += 1;
            }
        } else {
            for _ in 0..run {
                let (rgba, current) = read_pixel(
                    &mut reader,
                    indexed,
                    gray,
                    depth,
                    &palette,
                    map_first,
                    descriptor,
                    source,
                )?;
                let x = pixel % width_usize;
                let y = pixel / width_usize;
                let dx = if descriptor & 16 == 0 { x } else { width_usize - 1 - x };
                let dy = if descriptor & 32 == 0 { height_usize - 1 - y } else { y };
                let at = dy * width_usize + dx;
                pixels[at * 4..at * 4 + 4].copy_from_slice(&rgba);
                if let Some(store) = indices.as_mut() {
                    store[at] = current;
                }
                pixel += 1;
            }
        }
    }
    let mut palette_rgba = vec![0u8; palette.len() * 4];
    for (entry, rgba) in palette.iter().enumerate() {
        palette_rgba[entry * 4..entry * 4 + 4].copy_from_slice(rgba);
    }
    Ok(TgaImage {
        width,
        height,
        pixels,
        descriptor,
        indexed: indices.map(|store| TgaIndexed {
            indices: store,
            first: map_first,
            palette_rgba,
        }),
    })
}

/// Encode an uncompressed 32-bit TGA (`encodeTga`).
pub fn encode_tga(image: &ImageLevel) -> Vec<u8> {
    if image.width == 0
        || image.width > 65535
        || image.height == 0
        || image.height > 65535
        || image.pixels.len() != image.width as usize * image.height as usize * 4
    {
        panic!("Invalid TGA output");
    }
    let mut out = Vec::with_capacity(18 + image.pixels.len());
    out.extend_from_slice(&[0, 0, 2]);
    out.extend_from_slice(&[0u8; 9]);
    out.extend_from_slice(&(image.width as u16).to_le_bytes());
    out.extend_from_slice(&(image.height as u16).to_le_bytes());
    out.extend_from_slice(&[32, 40]);
    for chunk in image.pixels.as_chunks::<4>().0 {
        out.extend_from_slice(&[chunk[2], chunk[1], chunk[0], chunk[3]]);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn header(image_type: u8, map_type: u8, depth: u8, descriptor: u8, w: u16, h: u16) -> Vec<u8> {
        let mut out = vec![0, map_type, image_type, 0, 0, 0, 0, 0];
        out.extend_from_slice(&[0u8; 4]);
        out.extend_from_slice(&w.to_le_bytes());
        out.extend_from_slice(&h.to_le_bytes());
        out.push(depth);
        out.push(descriptor);
        out
    }

    #[test]
    fn round_trip_uncompressed() {
        let image = ImageLevel {
            width: 2,
            height: 1,
            pixels: vec![1, 2, 3, 4, 5, 6, 7, 8],
        };
        let encoded = encode_tga(&image);
        assert_eq!(encoded[2], 2);
        assert_eq!(encoded[17], 40);
        let decoded = decode_tga(&encoded, "<test>").unwrap();
        assert_eq!((decoded.width, decoded.height), (2, 1));
        assert_eq!(decoded.pixels, image.pixels);
        assert_eq!(decoded.indexed, None);
    }

    #[test]
    fn decodes_rle_truecolor() {
        let mut bytes = header(10, 0, 24, 0, 3, 1);
        bytes.extend_from_slice(&[0x81, 9, 8, 7, 0x00, 3, 2, 1]);
        let decoded = decode_tga(&bytes, "<test>").unwrap();
        assert_eq!(decoded.pixels, vec![7, 8, 9, 255, 7, 8, 9, 255, 1, 2, 3, 255]);
    }

    #[test]
    fn decodes_indexed_and_gray() {
        let mut bytes = header(1, 1, 8, 0, 2, 1);
        bytes[3..5].copy_from_slice(&0u16.to_le_bytes());
        bytes[5..7].copy_from_slice(&2u16.to_le_bytes());
        bytes[7] = 24;
        bytes.extend_from_slice(&[3, 2, 1, 6, 5, 4]);
        bytes.extend_from_slice(&[1, 0]);
        let decoded = decode_tga(&bytes, "<test>").unwrap();
        assert_eq!(decoded.pixels, vec![4, 5, 6, 255, 1, 2, 3, 255]);
        let indexed = decoded.indexed.unwrap();
        assert_eq!(indexed.indices, vec![1, 0]);
        assert_eq!(indexed.first, 0);
        assert_eq!(indexed.palette_rgba, vec![1, 2, 3, 255, 4, 5, 6, 255]);

        let mut gray = header(3, 0, 8, 0, 2, 1);
        gray.extend_from_slice(&[10, 20]);
        let decoded = decode_tga(&gray, "<test>").unwrap();
        assert_eq!(decoded.pixels, vec![10, 10, 10, 255, 20, 20, 20, 255]);
    }

    #[test]
    fn decodes_16bit_and_descriptor_flips() {
        // Red max: word 0x7C00 -> (31<<3)|(31>>2) = 255.
        let mut bytes = header(2, 0, 16, 0, 2, 1);
        bytes.extend_from_slice(&0x7c00u16.to_le_bytes());
        bytes.extend_from_slice(&0x03e0u16.to_le_bytes());
        let decoded = decode_tga(&bytes, "<test>").unwrap();
        assert_eq!(decoded.pixels, vec![255, 0, 0, 255, 0, 255, 0, 255]);

        // Horizontal flip (bit 4): stored order appears reversed.
        let mut flipped = header(2, 0, 24, 16, 2, 1);
        flipped.extend_from_slice(&[0, 0, 255, 0, 255, 0]);
        let decoded = decode_tga(&flipped, "<test>").unwrap();
        assert_eq!(decoded.pixels, vec![0, 255, 0, 255, 255, 0, 0, 255]);

        // Top-down (bit 5): first stored row is the top row.
        let mut top = header(2, 0, 24, 32, 1, 2);
        top.extend_from_slice(&[0, 0, 255, 0, 255, 0]);
        let decoded = decode_tga(&top, "<test>").unwrap();
        assert_eq!(decoded.pixels, vec![255, 0, 0, 255, 0, 255, 0, 255]);
    }

    #[test]
    fn rejects_packet_overflow() {
        let mut bytes = header(10, 0, 24, 0, 1, 1);
        bytes.extend_from_slice(&[0x81, 1, 2, 3]);
        let error = decode_tga(&bytes, "<test>").unwrap_err();
        assert_eq!(error.message, "TGA packet exceeds pixel count");
    }

    #[test]
    #[should_panic(expected = "Invalid TGA output")]
    fn encode_rejects_bad_input() {
        encode_tga(&ImageLevel {
            width: 0,
            height: 1,
            pixels: Vec::new(),
        });
    }
}
