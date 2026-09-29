//! Windows BMP decoder (owned pixels).
//!
//! Donor: `decodeBmp` in `src/formats/images/bmp.ts`
//! (adapted from quake-2-re-ts `qcommon/bmp.ts`, GPL-2.0-or-later).

use qa_core::binary::BinaryReader;

use super::{fail, ContentError};

/// 8-bit BMP indices plus their RGB palette.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BmpIndexed {
    /// Row-major palette indices (`width * height`).
    pub indices: Vec<u8>,
    /// RGB palette bytes (`color_count * 3`).
    pub palette: Vec<u8>,
}

/// Decoded BMP image (`BmpImage`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BmpImage {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// Row-major RGBA bytes (`width * height * 4`).
    pub pixels: Vec<u8>,
    /// 8-bit indices and palette, when present.
    pub indexed: Option<BmpIndexed>,
}

/// Decode a Windows BMP (`decodeBmp`).
pub fn decode_bmp(bytes: &[u8], source: &str) -> Result<BmpImage, ContentError> {
    let mut reader = BinaryReader::new(bytes, source);
    reader.expect_magic("BM")?;
    reader.skip(8)?;
    let offset = reader.u32()?;
    let header = reader.u32()?;
    let width = reader.i32()?;
    let signed_height = reader.i32()?;
    let planes = reader.u16()?;
    let depth = reader.u16()?;
    let compression = reader.u32()?;
    reader.skip(12)?;
    let mut color_count = reader.u32()?;
    reader.skip(4)?;
    let height = signed_height.unsigned_abs();
    if header != 40 || planes != 1 || compression != 0 || ![8u16, 24, 32].contains(&depth) {
        return Err(fail(
            source,
            14,
            "BMP requires BITMAPINFOHEADER, BI_RGB and 8/24/32-bit pixels".to_string(),
        ));
    }
    if width <= 0 || height == 0 || u64::from(width as u32) * u64::from(height) * 4 > 0x7fff_ffff {
        return Err(fail(source, 18, "Invalid BMP dimensions".to_string()));
    }
    let width_usize = width as usize;
    let height_usize = height as usize;
    let mut palette: Option<Vec<u8>> = None;
    if depth == 8 {
        if color_count == 0 {
            color_count = 256;
        }
        if color_count > 256 {
            return Err(fail(source, 46, "BMP palette has too many entries".to_string()));
        }
        let mut colors = vec![0u8; color_count as usize * 3];
        for entry in 0..color_count as usize {
            let b = reader.u8()?;
            let g = reader.u8()?;
            let r = reader.u8()?;
            reader.skip(1)?;
            colors[entry * 3] = r;
            colors[entry * 3 + 1] = g;
            colors[entry * 3 + 2] = b;
        }
        palette = Some(colors);
    }
    if u64::from(offset) < reader.offset() as u64 {
        return Err(fail(
            source,
            offset as usize,
            "BMP pixels overlap the header or palette".to_string(),
        ));
    }
    let stride = ((u64::from(width as u32) * u64::from(depth)).div_ceil(32) * 4) as usize;
    let data_len = usize::try_from(stride as u64 * height as u64).unwrap_or(usize::MAX);
    let data = reader.view(offset as usize, data_len)?;
    let mut pixels = vec![0u8; width_usize * height_usize * 4];
    let mut indices = palette.as_ref().map(|_| vec![0u8; width_usize * height_usize]);
    let bytes_per_pixel = usize::from(depth) / 8;
    for row in 0..height_usize {
        for x in 0..width_usize {
            let index = if signed_height < 0 {
                row * width_usize + x
            } else {
                (height_usize - row - 1) * width_usize + x
            };
            let at = row * stride + x * bytes_per_pixel;
            if let (Some(store), Some(colors)) = (indices.as_mut(), palette.as_ref()) {
                let color = data[at];
                if u32::from(color) >= color_count {
                    return Err(fail(
                        source,
                        offset as usize + at,
                        "BMP palette index out of range".to_string(),
                    ));
                }
                store[index] = color;
                let base = usize::from(color) * 3;
                pixels[index * 4] = colors[base];
                pixels[index * 4 + 1] = colors[base + 1];
                pixels[index * 4 + 2] = colors[base + 2];
                pixels[index * 4 + 3] = 255;
            } else {
                pixels[index * 4] = data[at + 2];
                pixels[index * 4 + 1] = data[at + 1];
                pixels[index * 4 + 2] = data[at];
                pixels[index * 4 + 3] = if depth == 32 { data[at + 3] } else { 255 };
            }
        }
    }
    Ok(BmpImage {
        width: width as u32,
        height,
        pixels,
        indexed: match (indices, palette) {
            (Some(indices), Some(palette)) => Some(BmpIndexed { indices, palette }),
            _ => None,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn header(width: i32, height: i32, depth: u16, offset: u32, colors: u32) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(b"BM");
        out.extend_from_slice(&[0u8; 8]);
        out.extend_from_slice(&offset.to_le_bytes());
        out.extend_from_slice(&40u32.to_le_bytes());
        out.extend_from_slice(&width.to_le_bytes());
        out.extend_from_slice(&height.to_le_bytes());
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&depth.to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(&[0u8; 12]);
        out.extend_from_slice(&colors.to_le_bytes());
        out.extend_from_slice(&[0u8; 4]);
        out
    }

    #[test]
    fn decodes_24bit_bottom_up() {
        let mut bytes = header(2, 2, 24, 54, 0);
        // Stride 8: bottom row red/green, top row blue/white.
        bytes.extend_from_slice(&[0, 0, 255, 0, 255, 0, 0, 0, 255, 0, 0, 255, 255, 255, 0, 0]);
        let image = decode_bmp(&bytes, "<test>").unwrap();
        assert_eq!((image.width, image.height), (2, 2));
        assert_eq!(image.indexed, None);
        assert_eq!(
            image.pixels,
            vec![0, 0, 255, 255, 255, 255, 255, 255, 255, 0, 0, 255, 0, 255, 0, 255]
        );
    }

    #[test]
    fn decodes_24bit_top_down() {
        let mut bytes = header(2, 1, 24, 54, 0);
        bytes.extend_from_slice(&[0, 0, 255, 0, 255, 0, 0, 0]);
        let image = decode_bmp(&bytes, "<test>").unwrap();
        assert_eq!((image.width, image.height), (2, 1));
        assert_eq!(image.pixels, vec![255, 0, 0, 255, 0, 255, 0, 255]);
    }

    #[test]
    fn decodes_8bit_palette() {
        let mut bytes = header(2, 1, 8, 54 + 8, 2);
        bytes.extend_from_slice(&[3, 2, 1, 0, 6, 5, 4, 0]);
        bytes.extend_from_slice(&[1, 0, 0, 0]);
        let image = decode_bmp(&bytes, "<test>").unwrap();
        assert_eq!(image.pixels, vec![4, 5, 6, 255, 1, 2, 3, 255]);
        let indexed = image.indexed.unwrap();
        assert_eq!(indexed.indices, vec![1, 0]);
        assert_eq!(indexed.palette, vec![1, 2, 3, 4, 5, 6]);
    }

    #[test]
    fn rejects_index_out_of_range() {
        let mut bytes = header(1, 1, 8, 54 + 8, 2);
        bytes.extend_from_slice(&[0, 0, 0, 0, 0, 0, 0, 0]);
        bytes.extend_from_slice(&[7, 0, 0, 0]);
        let error = decode_bmp(&bytes, "<test>").unwrap_err();
        assert_eq!(error.message, "BMP palette index out of range");
    }

    #[test]
    fn rejects_bad_header() {
        let bytes = header(0, 1, 24, 54, 0);
        let error = decode_bmp(&bytes, "<test>").unwrap_err();
        assert_eq!(error.message, "Invalid BMP dimensions");
        assert_eq!(error.offset, 18);
    }
}
