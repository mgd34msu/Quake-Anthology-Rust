//! Quake III PCX decoder (owned pixels).
//!
//! Donor: `decodePcxIndexed`/`expandPcx`/`decodePcx` in
//! `src/formats/images/q3-pcx.ts` (ported from id Software
//! `tr_image.c` LoadPCX/LoadPCX32, GPL-2.0-or-later).

use qa_core::binary::BinaryReader;

use super::{fail, ContentError, ImageLevel};

/// Header rejection message (`PcxRejection`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PcxRejection {
    /// `Bad pcx file ...` message, including the trailing newline.
    pub message: String,
}

/// Decoded Q3 PCX indices plus palette (`IndexedPcxImage`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexedPcxImage {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// Row-major palette indices (`width * height`).
    pub indices: Vec<u8>,
    /// RGB palette bytes (768).
    pub palette: Vec<u8>,
}

/// Indexed Q3 PCX outcome (`decodePcxIndexed`, exported as `decodeQ3PcxIndexed`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Q3PcxIndexed {
    /// Decoded indices and palette.
    Image(IndexedPcxImage),
    /// Header rejection (the source PRINT_ALL/null outcome).
    Rejected(PcxRejection),
}

/// Expanded Q3 PCX outcome (`decodePcx`, exported as `decodeQ3Pcx`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Q3Pcx {
    /// Decoded RGBA image.
    Image(ImageLevel),
    /// Header rejection.
    Rejected(PcxRejection),
}

/// Decode indexed Q3 PCX bytes (`decodePcxIndexed`).
pub fn decode_q3_pcx_indexed(bytes: &[u8], source: &str) -> Result<Q3PcxIndexed, ContentError> {
    let mut reader = BinaryReader::new(bytes, source);
    if reader.length() < 12 {
        return Err(fail(source, 0, "truncated PCX header".to_string()));
    }
    let manufacturer = reader.u8()?;
    let version = reader.u8()?;
    let encoding = reader.u8()?;
    let bits_per_pixel = reader.u8()?;
    reader.skip(4)?;
    let xmax = reader.u16()?;
    let ymax = reader.u16()?;
    let width = u32::from(xmax) + 1;
    let height = u32::from(ymax) + 1;
    if manufacturer != 0x0a || version != 5 || encoding != 1 || bits_per_pixel != 8 || xmax >= 1024 || ymax >= 1024 {
        return Ok(Q3PcxIndexed::Rejected(PcxRejection {
            message: format!("Bad pcx file {source} ({width} x {height}) ({xmax} x {ymax})\n"),
        }));
    }

    if reader.length() < 768 {
        return Err(fail(source, 0, "PCX palette precedes input".to_string()));
    }
    let width_usize = width as usize;
    let height_usize = height as usize;
    let mut indices = vec![0u8; width_usize * height_usize];
    let palette = reader.section(reader.length() - 768, 768)?.bytes(768)?;
    reader.seek(128)?;
    for y in 0..height_usize {
        let mut x = 0usize;
        while x < width_usize {
            let packet_offset = reader.offset();
            let mut data_byte = reader.u8()?;
            let mut run_length = 1usize;
            if data_byte & 0xc0 == 0xc0 {
                run_length = usize::from(data_byte & 0x3f);
                data_byte = reader.u8()?;
            }
            let destination = y * width_usize + x;
            if run_length > indices.len() - destination {
                return Err(fail(
                    source,
                    packet_offset,
                    "PCX RLE packet overruns image allocation".to_string(),
                ));
            }
            indices[destination..destination + run_length].fill(data_byte);
            x += run_length;
        }
    }

    Ok(Q3PcxIndexed::Image(IndexedPcxImage {
        width,
        height,
        indices,
        palette,
    }))
}

/// Expand indexed Q3 PCX pixels (`expandPcx`, exported as `expandQ3Pcx`).
pub fn expand_q3_pcx(image: &IndexedPcxImage) -> ImageLevel {
    let mut pixels = vec![0u8; image.indices.len() * 4];
    let mut destination = 0;
    for index in image.indices.iter() {
        let base = usize::from(*index) * 3;
        pixels[destination] = image.palette[base];
        pixels[destination + 1] = image.palette[base + 1];
        pixels[destination + 2] = image.palette[base + 2];
        pixels[destination + 3] = 255;
        destination += 4;
    }
    ImageLevel {
        width: image.width,
        height: image.height,
        pixels,
    }
}

/// Decode and expand Q3 PCX bytes (`decodePcx`, exported as `decodeQ3Pcx`).
pub fn decode_q3_pcx(bytes: &[u8], source: &str) -> Result<Q3Pcx, ContentError> {
    match decode_q3_pcx_indexed(bytes, source)? {
        Q3PcxIndexed::Image(image) => Ok(Q3Pcx::Image(expand_q3_pcx(&image))),
        Q3PcxIndexed::Rejected(rejection) => Ok(Q3Pcx::Rejected(rejection)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(xmax: u16, ymax: u16, rle: &[u8], palette: &[u8]) -> Vec<u8> {
        let mut out = vec![0x0a, 5, 1, 8, 0, 0, 0, 0];
        out.extend_from_slice(&xmax.to_le_bytes());
        out.extend_from_slice(&ymax.to_le_bytes());
        out.resize(128, 0);
        out.extend_from_slice(rle);
        // Pad so the palette occupies the final 768 bytes.
        let body = out.len();
        let total = body + 768;
        out.resize(total, 0);
        out[total - 768..total - 768 + palette.len()].copy_from_slice(palette);
        out
    }

    #[test]
    fn decodes_and_expands() {
        let mut palette = vec![0u8; 768];
        palette[0..3].copy_from_slice(&[10, 20, 30]);
        palette[3..6].copy_from_slice(&[40, 50, 60]);
        let bytes = fixture(1, 0, &[0, 1], &palette);
        match decode_q3_pcx(&bytes, "<test>").unwrap() {
            Q3Pcx::Image(image) => {
                assert_eq!((image.width, image.height), (2, 1));
                assert_eq!(image.pixels, vec![10, 20, 30, 255, 40, 50, 60, 255]);
            }
            Q3Pcx::Rejected(_) => panic!("expected image"),
        }
    }

    #[test]
    fn rejects_bad_dimensions_with_exact_message() {
        let mut bad = fixture(1, 0, &[0, 0], &vec![0u8; 768]);
        bad[8] = 0;
        bad[9] = 4; // xmax = 1024
        match decode_q3_pcx_indexed(&bad, "<test>").unwrap() {
            Q3PcxIndexed::Rejected(rejection) => {
                assert_eq!(rejection.message, "Bad pcx file <test> (1025 x 1) (1024 x 0)\n");
            }
            Q3PcxIndexed::Image(_) => panic!("expected rejection"),
        }
        match decode_q3_pcx(&bad, "<test>").unwrap() {
            Q3Pcx::Rejected(rejection) => assert!(rejection.message.starts_with("Bad pcx file")),
            Q3Pcx::Image(_) => panic!("expected rejection"),
        }
    }

    #[test]
    fn rejects_rle_overrun() {
        // 1x1 image, run of 2 overruns the single-pixel allocation.
        let bytes = fixture(0, 0, &[0xc2, 7], &vec![0u8; 768]);
        let error = decode_q3_pcx_indexed(&bytes, "<test>").unwrap_err();
        assert_eq!(error.message, "PCX RLE packet overruns image allocation");
        assert_eq!(error.offset, 128);
    }

    #[test]
    fn rejects_short_header() {
        let error = decode_q3_pcx_indexed(&[0x0a; 11], "<test>").unwrap_err();
        assert_eq!(error.message, "truncated PCX header");
    }

    #[test]
    fn expand_maps_indices() {
        let image = IndexedPcxImage {
            width: 1,
            height: 1,
            indices: vec![1],
            palette: {
                let mut palette = vec![0u8; 768];
                palette[3..6].copy_from_slice(&[7, 8, 9]);
                palette
            },
        };
        assert_eq!(expand_q3_pcx(&image).pixels, vec![7, 8, 9, 255]);
    }
}
