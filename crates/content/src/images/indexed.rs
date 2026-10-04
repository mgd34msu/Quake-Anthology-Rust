//! Indexed Quake image formats (owned pixels).
//!
//! Donor: `src/formats/images/indexed.ts` (adapted from Q1
//! model.c/wad.c and Q2 gl_image.c/qfiles.h, GPL-2.0-or-later).
//! Q1 mip textures and QPICs delegate to the borrowed
//! [`crate::wad`] decoders and convert to owned storage.

use qa_core::binary::{BinaryReader, BinaryWriter};

use super::{fail, ContentError};
use crate::wad as borrowed;

/// Owned indexed image (`IndexedImage`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexedImage {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// Row-major palette indices (`width * height`).
    pub indices: Vec<u8>,
}

/// Decoded PCX image (`PcxImage`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PcxImage {
    /// Width in pixels.
    pub width: usize,
    /// Height in pixels.
    pub height: usize,
    /// Row-major palette indices (`width * height`).
    pub indices: Vec<u8>,
    /// Trailing 768-byte palette, when the `0x0C` marker is present.
    pub palette: Option<Vec<u8>>,
}

/// Four owned mip levels (`MipLevels`).
pub type MipLevels = [IndexedImage; 4];

/// Quake mip texture (`Q1MipTexture`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Q1MipTexture {
    /// Embedded levels.
    Embedded {
        /// Texture name.
        name: String,
        /// Width.
        width: u32,
        /// Height.
        height: u32,
        /// Four mip levels.
        levels: MipLevels,
    },
    /// External texture (all mip offsets zero).
    External {
        /// Texture name.
        name: String,
        /// Width.
        width: u32,
        /// Height.
        height: u32,
    },
}

/// Quake II WAL texture (`WalImage`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WalImage {
    /// Texture name.
    pub name: String,
    /// Width.
    pub width: u32,
    /// Height.
    pub height: u32,
    /// Four mip levels.
    pub levels: MipLevels,
    /// Animation name.
    pub animation: String,
    /// Surface flags.
    pub flags: i32,
    /// Contents flags.
    pub contents: i32,
    /// Light value.
    pub value: i32,
}

/// Decoded colored lighting (`decodeLit`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QlitImage {
    /// RGB samples.
    pub samples: Vec<u8>,
}

/// Lighting kind marker (`qlit-rgb8`).
pub const QLIT_KIND: &str = "qlit-rgb8";

fn dimensions_u32(width: u32, height: u32, source: &str) -> Result<u64, ContentError> {
    let count = u64::from(width) * u64::from(height);
    if width == 0 || height == 0 || count > 0x7fff_ffff {
        return Err(fail(source, 0, format!("Invalid image dimensions {width}x{height}")));
    }
    Ok(count)
}

fn dimensions_i32(width: i32, height: i32, source: &str) -> Result<u64, ContentError> {
    if width <= 0 || height <= 0 {
        return Err(fail(source, 0, format!("Invalid image dimensions {width}x{height}")));
    }
    let count = width as u64 * height as u64;
    if count > 0x7fff_ffff {
        return Err(fail(source, 0, format!("Invalid image dimensions {width}x{height}")));
    }
    Ok(count)
}

fn mip_level(
    reader: &BinaryReader<'_>,
    width: u32,
    height: u32,
    offset: u32,
    minimum: u32,
    source: &str,
) -> Result<IndexedImage, ContentError> {
    if offset < minimum {
        return Err(fail(
            source,
            offset as usize,
            "Mip data overlaps the header".to_string(),
        ));
    }
    let count = dimensions_u32(width, height, source)?;
    let indices = reader.view(offset as usize, count as usize)?.to_vec();
    Ok(IndexedImage { width, height, indices })
}

/// Decode a Quake mip texture (`decodeQ1MipTexture`).
pub fn decode_q1_mip_texture(bytes: &[u8], source: &str) -> Result<Q1MipTexture, ContentError> {
    match borrowed::decode_q1_mip_texture(bytes, source)? {
        borrowed::MipTexture::External { name, width, height } => Ok(Q1MipTexture::External { name, width, height }),
        borrowed::MipTexture::Embedded {
            name,
            width,
            height,
            levels,
        } => Ok(Q1MipTexture::Embedded {
            name,
            width,
            height,
            levels: [
                IndexedImage {
                    width,
                    height,
                    indices: levels[0].to_vec(),
                },
                IndexedImage {
                    width: width / 2,
                    height: height / 2,
                    indices: levels[1].to_vec(),
                },
                IndexedImage {
                    width: width / 4,
                    height: height / 4,
                    indices: levels[2].to_vec(),
                },
                IndexedImage {
                    width: width / 8,
                    height: height / 8,
                    indices: levels[3].to_vec(),
                },
            ],
        }),
    }
}

/// Decode a Quake II WAL texture (`decodeWal`).
pub fn decode_wal(bytes: &[u8], source: &str) -> Result<WalImage, ContentError> {
    let mut reader = BinaryReader::new(bytes, source);
    let name = reader.fixed_byte_string(32)?;
    let width = reader.u32()?;
    let height = reader.u32()?;
    dimensions_u32(width, height, source)?;
    let a = reader.u32()?;
    let b = reader.u32()?;
    let c = reader.u32()?;
    let d = reader.u32()?;
    let animation = reader.fixed_byte_string(32)?;
    let flags = reader.i32()?;
    let contents = reader.i32()?;
    let value = reader.i32()?;
    Ok(WalImage {
        name,
        width,
        height,
        animation,
        flags,
        contents,
        value,
        levels: [
            mip_level(&reader, width, height, a, 100, source)?,
            mip_level(&reader, (width >> 1).max(1), (height >> 1).max(1), b, 100, source)?,
            mip_level(&reader, (width >> 2).max(1), (height >> 2).max(1), c, 100, source)?,
            mip_level(&reader, (width >> 3).max(1), (height >> 3).max(1), d, 100, source)?,
        ],
    })
}

/// Decode a QPIC image (`decodeQpic`).
pub fn decode_qpic(bytes: &[u8], source: &str) -> Result<IndexedImage, ContentError> {
    let image = borrowed::decode_qpic(bytes, source)?;
    Ok(IndexedImage {
        width: image.width as u32,
        height: image.height as u32,
        indices: image.indices.to_vec(),
    })
}

/// Read one PCX run-length packet, returning the pixel value and the
/// run length (donor `LoadPCX` packet loop).
///
/// Shared by the Q1 screenshot decoder below and the Q3 decoder in
/// [`super::q3_pcx`]; the scanline loops stay split because Q1 pads rows
/// to `bytes_per_line` and rejects overruns while Q3 decodes tight rows
/// with its own overrun error.
pub fn read_pcx_run(reader: &mut BinaryReader<'_>) -> Result<(u8, usize), ContentError> {
    let head = reader.u8()?;
    if head & 192 == 192 {
        let value = reader.u8()?;
        Ok((value, usize::from(head & 63)))
    } else {
        Ok((head, 1))
    }
}

/// Decode a screenshot PCX (`decodePcx`).
pub fn decode_pcx(bytes: &[u8], source: &str) -> Result<PcxImage, ContentError> {
    let mut reader = BinaryReader::new(bytes, source);
    if reader.u8()? != 10 || reader.u8()? != 5 || reader.u8()? != 1 || reader.u8()? != 8 {
        return Err(fail(source, 0, "Expected version 5, RLE, 8-bit PCX".to_string()));
    }
    let xmin = reader.u16()?;
    let ymin = reader.u16()?;
    let xmax = reader.u16()?;
    let ymax = reader.u16()?;
    let width = i32::from(xmax) - i32::from(xmin) + 1;
    let height = i32::from(ymax) - i32::from(ymin) + 1;
    dimensions_i32(width, height, source)?;
    reader.seek(65)?;
    let planes = reader.u8()?;
    let bytes_per_line = reader.u16()?;
    if planes != 1 || i32::from(bytes_per_line) < width {
        return Err(fail(
            source,
            65,
            "Expected single-plane PCX with a complete scanline".to_string(),
        ));
    }
    let palette_offset = bytes.len().checked_sub(769);
    let has_palette = matches!(palette_offset, Some(at) if at >= 128 && bytes[at] == 12);
    let encoded_end = if has_palette {
        palette_offset.unwrap_or(bytes.len())
    } else {
        bytes.len()
    };
    let mut encoded = reader.section(128, encoded_end.saturating_sub(128))?;
    let width_usize = width as usize;
    let height_usize = height as usize;
    let stride = usize::from(bytes_per_line);
    let mut indices = vec![0u8; width_usize * height_usize];
    for y in 0..height_usize {
        let mut x = 0usize;
        while x < stride {
            let (value, count) = read_pcx_run(&mut encoded)?;
            if count == 0 || count > stride - x {
                return Err(fail(
                    source,
                    encoded.offset() + 128,
                    "PCX run exceeds its scanline".to_string(),
                ));
            }
            if x < width_usize {
                let end = (y * width_usize + x + count).min((y + 1) * width_usize);
                indices[y * width_usize + x..end].fill(value);
            }
            x += count;
        }
    }
    Ok(PcxImage {
        width: width_usize,
        height: height_usize,
        indices,
        palette: if has_palette {
            Some(bytes[encoded_end + 1..].to_vec())
        } else {
            None
        },
    })
}

/// Encode a Quake screenshot PCX (`encodePcx`).
pub fn encode_pcx(image: &IndexedImage, palette: &[u8]) -> Result<Vec<u8>, ContentError> {
    dimensions_u32(image.width, image.height, "PCX output")?;
    let count = image.width as usize * image.height as usize;
    if image.width > 65534 || image.height > 65536 || image.indices.len() != count || palette.len() != 768 {
        return Err(fail(
            "PCX output",
            0,
            "Invalid PCX output dimensions or palette".to_string(),
        ));
    }
    let stride = ((image.width + 1) & !1) as usize;
    let mut writer = BinaryWriter::new(128 + stride * image.height as usize * 2 + 769);
    writer.bytes(&[10, 5, 1, 8])?;
    writer.u16(0)?;
    writer.u16(0)?;
    writer.u16((image.width - 1) as u16)?;
    writer.u16((image.height - 1) as u16)?;
    writer.u16(image.width as u16)?;
    writer.u16(image.height.min(65535) as u16)?;
    writer.bytes(&[0u8; 49])?;
    writer.u8(1)?;
    writer.u16(stride as u16)?;
    writer.u16(1)?;
    writer.bytes(&[0u8; 58])?;
    for y in 0..image.height as usize {
        for x in 0..stride {
            let value = if x < image.width as usize {
                match image.indices.get(y * image.width as usize + x).copied() {
                    Some(value) => value,
                    None => return Err(fail("PCX output", 0, "Incomplete PCX pixels".to_string())),
                }
            } else {
                0
            };
            if value & 192 == 192 {
                writer.u8(193)?;
            }
            writer.u8(value)?;
        }
    }
    writer.u8(12)?;
    writer.bytes(palette)?;
    Ok(writer.finish())
}

/// Decode colored lighting (`decodeLit`).
pub fn decode_lit(bytes: &[u8], expected_samples: Option<usize>, source: &str) -> Result<QlitImage, ContentError> {
    let mut reader = BinaryReader::new(bytes, source);
    reader.expect_magic("QLIT")?;
    if reader.i32()? != 1 {
        return Err(fail(source, 4, "Only QLIT version 1 is supported".to_string()));
    }
    let matches_expected = match expected_samples {
        Some(expected) => expected.checked_mul(3) == Some(reader.remaining()),
        None => true,
    };
    if !reader.remaining().is_multiple_of(3) || !matches_expected {
        return Err(fail(source, 8, "QLIT sample count does not match lighting".to_string()));
    }
    let remaining = reader.remaining();
    Ok(QlitImage {
        samples: reader.bytes(remaining)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn miptex_bytes(external: bool) -> Vec<u8> {
        let mut out = vec![0u8; 16];
        out[..4].copy_from_slice(b"test");
        out.extend_from_slice(&16u32.to_le_bytes());
        out.extend_from_slice(&16u32.to_le_bytes());
        if external {
            out.extend_from_slice(&[0u8; 16]);
        } else {
            for offset in [40u32, 296, 360, 376] {
                out.extend_from_slice(&offset.to_le_bytes());
            }
            out.extend_from_slice(&[1u8; 256]);
            out.extend_from_slice(&[2u8; 64]);
            out.extend_from_slice(&[3u8; 16]);
            out.extend_from_slice(&[4u8; 4]);
        }
        out
    }

    #[test]
    fn q1mip_embedded_and_external() {
        match decode_q1_mip_texture(&miptex_bytes(false), "<test>").unwrap() {
            Q1MipTexture::Embedded {
                name,
                width,
                height,
                levels,
            } => {
                assert_eq!((name.as_str(), width, height), ("test", 16, 16));
                assert_eq!(levels[0].indices.len(), 256);
                assert_eq!(levels[3].indices, vec![4u8; 4]);
                assert_eq!((levels[3].width, levels[3].height), (2, 2));
            }
            Q1MipTexture::External { .. } => panic!("expected embedded"),
        }
        match decode_q1_mip_texture(&miptex_bytes(true), "<test>").unwrap() {
            Q1MipTexture::External { width, .. } => assert_eq!(width, 16),
            Q1MipTexture::Embedded { .. } => panic!("expected external"),
        }
    }

    #[test]
    fn qpic_decodes() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&2i32.to_le_bytes());
        bytes.extend_from_slice(&2i32.to_le_bytes());
        bytes.extend_from_slice(&[5, 6, 7, 8]);
        let image = decode_qpic(&bytes, "<test>").unwrap();
        assert_eq!((image.width, image.height), (2, 2));
        assert_eq!(image.indices, vec![5, 6, 7, 8]);
    }

    #[test]
    fn wal_decodes() {
        let mut out = vec![0u8; 32];
        out[..4].copy_from_slice(b"wall");
        out.extend_from_slice(&8u32.to_le_bytes());
        out.extend_from_slice(&8u32.to_le_bytes());
        for offset in [100u32, 164, 180, 184] {
            out.extend_from_slice(&offset.to_le_bytes());
        }
        out.extend_from_slice(&[0u8; 32]);
        out.extend_from_slice(&1i32.to_le_bytes());
        out.extend_from_slice(&2i32.to_le_bytes());
        out.extend_from_slice(&3i32.to_le_bytes());
        assert_eq!(out.len(), 100);
        out.extend_from_slice(&[9u8; 64]);
        out.extend_from_slice(&[8u8; 16]);
        out.extend_from_slice(&[7u8; 4]);
        out.extend_from_slice(&[6u8; 1]);
        let image = decode_wal(&out, "<test>").unwrap();
        assert_eq!((image.width, image.height), (8, 8));
        assert_eq!((image.flags, image.contents, image.value), (1, 2, 3));
        assert_eq!(image.levels[0].indices.len(), 64);
        assert_eq!(image.levels[3].indices, vec![6u8; 1]);
    }

    #[test]
    fn pcx_round_trip() {
        let image = IndexedImage {
            width: 3,
            height: 2,
            indices: vec![0, 192, 5, 200, 1, 2],
        };
        let palette: Vec<u8> = (0..768u16).map(|value| (value % 256) as u8).collect();
        let encoded = encode_pcx(&image, &palette).unwrap();
        let decoded = decode_pcx(&encoded, "<test>").unwrap();
        assert_eq!((decoded.width, decoded.height), (3, 2));
        assert_eq!(decoded.indices, image.indices);
        assert_eq!(decoded.palette.unwrap(), palette);
    }

    #[test]
    fn pcx_without_palette_is_none() {
        let image = IndexedImage {
            width: 2,
            height: 1,
            indices: vec![4, 5],
        };
        let palette = vec![9u8; 768];
        let mut encoded = encode_pcx(&image, &palette).unwrap();
        encoded.truncate(encoded.len() - 769);
        let decoded = decode_pcx(&encoded, "<test>").unwrap();
        assert_eq!(decoded.palette, None);
        assert_eq!(decoded.indices, vec![4, 5]);
    }

    #[test]
    fn pcx_run_exceeds_scanline() {
        let image = IndexedImage {
            width: 2,
            height: 1,
            indices: vec![1, 2],
        };
        let mut encoded = encode_pcx(&image, &vec![0u8; 768]).unwrap();
        // Stride 2: replace the row with a run of 3.
        encoded[128] = 0xc3;
        encoded[129] = 9;
        let error = decode_pcx(&encoded, "<test>").unwrap_err();
        assert_eq!(error.message, "PCX run exceeds its scanline");
    }

    #[test]
    fn pcx_packet_reads_literal_and_run() {
        use qa_core::binary::BinaryReader;

        let mut literal = BinaryReader::new(&[7u8], "<test>");
        assert_eq!(read_pcx_run(&mut literal).unwrap(), (7, 1));
        let mut run = BinaryReader::new(&[0xc5u8, 9], "<test>");
        assert_eq!(read_pcx_run(&mut run).unwrap(), (9, 5));
        let mut escaped = BinaryReader::new(&[0xc1u8, 192], "<test>");
        assert_eq!(read_pcx_run(&mut escaped).unwrap(), (192, 1));
        let mut truncated = BinaryReader::new(&[0xc2u8], "<test>");
        assert!(read_pcx_run(&mut truncated).is_err());
    }

    #[test]
    fn lit_ok_and_errors() {
        let mut bytes = b"QLIT".to_vec();
        bytes.extend_from_slice(&1i32.to_le_bytes());
        bytes.extend_from_slice(&[1, 2, 3, 4, 5, 6]);
        let image = decode_lit(&bytes, Some(2), "<test>").unwrap();
        assert_eq!(image.samples, vec![1, 2, 3, 4, 5, 6]);
        assert_eq!(QLIT_KIND, "qlit-rgb8");

        let mut bad_version = bytes.clone();
        bad_version[4] = 2;
        let error = decode_lit(&bad_version, None, "<test>").unwrap_err();
        assert_eq!(error.message, "Only QLIT version 1 is supported");

        let error = decode_lit(&bytes, Some(3), "<test>").unwrap_err();
        assert_eq!(error.message, "QLIT sample count does not match lighting");

        let mut bad_len = bytes.clone();
        bad_len.push(7);
        let error = decode_lit(&bad_len, None, "<test>").unwrap_err();
        assert_eq!(error.message, "QLIT sample count does not match lighting");
    }
}
