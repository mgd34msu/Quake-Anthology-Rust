//! Quake III BMP decoder (owned pixels).
//!
//! Donor: `decodeBmp` in `src/formats/images/q3-bmp.ts`
//! (ported from id Software `tr_image.c` LoadBMP, GPL-2.0-or-later).

use std::error::Error;
use std::fmt;

use qa_core::binary::BinaryReader;

use super::{fail, ContentError, ImageLevel};

/// LoadBMP failure the caller must dispatch through ERR_DROP (`BmpDropError`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BmpDropError {
    /// Input name for diagnostics.
    pub source: String,
    /// Drop message, including the trailing newline.
    pub message: String,
}

impl fmt::Display for BmpDropError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl Error for BmpDropError {}

impl From<BmpDropError> for ContentError {
    fn from(error: BmpDropError) -> Self {
        fail(&error.source, 0, error.message)
    }
}

fn drop_error(source: &str, message: String) -> ContentError {
    ContentError::from(BmpDropError {
        source: source.to_string(),
        message,
    })
}

/// Decode a Q3 BMP (`decodeBmp`, exported as `decodeQ3Bmp`).
pub fn decode_q3_bmp(bytes: &[u8], source: &str) -> Result<ImageLevel, ContentError> {
    let mut reader = BinaryReader::new(bytes, source);
    let id0 = reader.u8()?;
    let id1 = reader.u8()?;
    let file_size = reader.u32()?;
    reader.skip(12)?;
    let width = reader.i32()?;
    let signed_height = reader.i32()?;
    reader.skip(2)?;
    let bits_per_pixel = reader.u16()?;
    let compression = reader.u32()?;
    reader.skip(20)?;

    let palette = if bits_per_pixel == 8 {
        Some(reader.bytes(1024)?)
    } else {
        None
    };

    if id0 != 66 && id1 != 77 {
        return Err(drop_error(
            source,
            format!("LoadBMP: only Windows-style BMP files supported ({source})\n"),
        ));
    }
    if u64::from(file_size) != bytes.len() as u64 {
        return Err(drop_error(
            source,
            format!(
                "LoadBMP: header size does not match file size ({} vs. {}) ({source})\n",
                file_size as i32,
                bytes.len()
            ),
        ));
    }
    if compression != 0 {
        return Err(drop_error(
            source,
            format!("LoadBMP: only uncompressed BMP files supported ({source})\n"),
        ));
    }
    if bits_per_pixel < 8 {
        return Err(drop_error(
            source,
            format!("LoadBMP: monochrome and 4-bit BMP files not supported ({source})\n"),
        ));
    }

    if width < 0 {
        return Err(fail(source, 18, format!("negative BMP width {width}")));
    }
    if signed_height == i32::MIN {
        return Err(fail(
            source,
            22,
            "BMP height negation overflows signed 32-bit source arithmetic".to_string(),
        ));
    }
    let height = signed_height.unsigned_abs();
    let output_length = width as u64 * u64::from(height) * 4;
    if output_length > 0x7fff_ffff {
        return Err(fail(
            source,
            18,
            format!("decoded BMP size {output_length} overflows signed 32-bit source allocation arithmetic"),
        ));
    }
    if output_length == 0 {
        return Ok(ImageLevel {
            width: width as u32,
            height,
            pixels: Vec::new(),
        });
    }
    if bits_per_pixel == 16 {
        return Err(fail(
            source,
            28,
            "16-bit LoadBMP source path reads uninitialized output and writes beyond its allocation; unsupported source-indeterminate pixels".to_string(),
        ));
    }
    if bits_per_pixel != 8 && bits_per_pixel != 24 && bits_per_pixel != 32 {
        return Err(drop_error(
            source,
            format!("LoadBMP: illegal pixel_size '{bits_per_pixel}' in file '{source}'\n"),
        ));
    }
    let required = width as u64 * u64::from(height) * u64::from(bits_per_pixel / 8);
    if (reader.remaining() as u64) < required {
        return Err(fail(source, reader.offset(), "truncated BMP pixel data".to_string()));
    }

    let width_usize = width as usize;
    let height_usize = height as usize;
    let mut pixels = vec![0u8; output_length as usize];
    for row in (0..height_usize).rev() {
        let mut destination = row * width_usize * 4;
        for _ in 0..width_usize {
            if let Some(table) = palette.as_ref() {
                let index = reader.u8()? as usize * 4;
                pixels[destination] = table[index + 2];
                pixels[destination + 1] = table[index + 1];
                pixels[destination + 2] = table[index];
                pixels[destination + 3] = 255;
                destination += 4;
            } else {
                let blue = reader.u8()?;
                let green = reader.u8()?;
                let red = reader.u8()?;
                pixels[destination] = red;
                pixels[destination + 1] = green;
                pixels[destination + 2] = blue;
                pixels[destination + 3] = if bits_per_pixel == 32 { reader.u8()? } else { 255 };
                destination += 4;
            }
        }
    }
    Ok(ImageLevel {
        width: width as u32,
        height,
        pixels,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(id0: u8, id1: u8, bits: u16, pixels: &[u8], palette: Option<&[u8]>) -> Vec<u8> {
        let mut out = vec![id0, id1, 0, 0, 0, 0];
        out.extend_from_slice(&[0u8; 12]);
        out.extend_from_slice(&2i32.to_le_bytes());
        out.extend_from_slice(&1i32.to_le_bytes());
        out.extend_from_slice(&[0u8; 2]);
        out.extend_from_slice(&bits.to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(&[0u8; 20]);
        if let Some(table) = palette {
            out.extend_from_slice(table);
        }
        out.extend_from_slice(pixels);
        let len = out.len() as u32;
        out[2..6].copy_from_slice(&len.to_le_bytes());
        out
    }

    #[test]
    fn decodes_24bit_bottom_up() {
        let bytes = fixture(66, 77, 24, &[0, 0, 255, 0, 255, 0], None);
        let image = decode_q3_bmp(&bytes, "<test>").unwrap();
        assert_eq!((image.width, image.height), (2, 1));
        assert_eq!(image.pixels, vec![255, 0, 0, 255, 0, 255, 0, 255]);
    }

    #[test]
    fn magic_quirk_accepts_single_wrong_byte() {
        // The donor checks id0 != 66 && id1 != 77, so one correct byte passes.
        let bytes = fixture(66, 0, 24, &[1, 2, 3, 4, 5, 6], None);
        let image = decode_q3_bmp(&bytes, "<test>").unwrap();
        assert_eq!(image.pixels, vec![3, 2, 1, 255, 6, 5, 4, 255]);
        let bytes = fixture(0, 0, 24, &[1, 2, 3, 4, 5, 6], None);
        let error = decode_q3_bmp(&bytes, "<test>").unwrap_err();
        assert!(error.message.contains("only Windows-style BMP files"));
    }

    #[test]
    fn rejects_size_mismatch() {
        let mut bytes = fixture(66, 77, 24, &[1, 2, 3, 4, 5, 6], None);
        bytes[2] = 1;
        let error = decode_q3_bmp(&bytes, "<test>").unwrap_err();
        assert!(error.message.contains("header size does not match file size"));
    }

    #[test]
    fn rejects_16bit_and_truncation() {
        let bytes = fixture(66, 77, 16, &[0u8; 4], None);
        let error = decode_q3_bmp(&bytes, "<test>").unwrap_err();
        assert_eq!(error.offset, 28);
        assert!(error.message.contains("unsupported source-indeterminate pixels"));

        let mut bytes = fixture(66, 77, 24, &[1, 2, 3], None);
        let len = bytes.len() as u32;
        bytes[2..6].copy_from_slice(&len.to_le_bytes());
        let error = decode_q3_bmp(&bytes, "<test>").unwrap_err();
        assert_eq!(error.message, "truncated BMP pixel data");
    }

    #[test]
    fn drop_error_converts() {
        let error = BmpDropError {
            source: "<s>".to_string(),
            message: "boom\n".to_string(),
        };
        assert_eq!(format!("{error}"), "boom\n");
        let content = ContentError::from(error);
        assert_eq!((content.input.as_str(), content.offset), ("<s>", 0));
    }
}
