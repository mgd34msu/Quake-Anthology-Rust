//! WAD2/WAD3 image archives and Quake mip textures.
//!
//! Donor provenance: `decodeWad` and `decodeWadImage` in
//! `src/formats/images/wad.ts`, with `decodeQ1MipTexture` and
//! `decodeQpic` from `src/formats/images/indexed.ts` and palette
//! conversion from `q1TextureRgba` in `src/formats/q1-map/textures.ts`.
//!
//! Lump bytes, mip levels, and palettes are borrowed from the input;
//! only RGBA expansion allocates.

use qa_core::binary::{BinaryError, BinaryReader};

/// Maximum decoded image pixels (`0x7fffffff`).
const MAX_PIXELS: u64 = 0x7fff_ffff;

fn dimensions(source: &str, width: i64, height: i64) -> Result<u64, BinaryError> {
    if width <= 0 || height <= 0 {
        return Err(BinaryError {
            input: source.to_string(),
            offset: 0,
            message: format!("Invalid image dimensions {width}x{height}"),
        });
    }
    let count = width as u64 * height as u64;
    if count > MAX_PIXELS {
        return Err(BinaryError {
            input: source.to_string(),
            offset: 0,
            message: format!("Invalid image dimensions {width}x{height}"),
        });
    }
    Ok(count)
}

/// Quake mip texture (`Q1MipTexture`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MipTexture<'a> {
    /// Embedded levels borrowed from the input.
    Embedded {
        /// Texture name.
        name: String,
        /// Width.
        width: u32,
        /// Height.
        height: u32,
        /// Four mip levels.
        levels: [&'a [u8]; 4],
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

fn mip_level<'a>(
    reader: &BinaryReader<'a>,
    source: &str,
    width: u32,
    height: u32,
    offset: u32,
) -> Result<&'a [u8], BinaryError> {
    if offset < 40 {
        return Err(BinaryError {
            input: source.to_string(),
            offset: offset as usize,
            message: "Mip data overlaps the header".to_string(),
        });
    }
    let length = dimensions(source, i64::from(width), i64::from(height))? as usize;
    reader.view(offset as usize, length)
}

/// Decode a Quake mip texture (`decodeQ1MipTexture`).
pub fn decode_q1_mip_texture<'a>(data: &'a [u8], source: &str) -> Result<MipTexture<'a>, BinaryError> {
    let mut reader = BinaryReader::new(data, source);
    let name = reader.fixed_byte_string(16)?;
    let width = reader.u32()?;
    let height = reader.u32()?;
    dimensions(source, i64::from(width), i64::from(height))?;
    if width % 16 != 0 || height % 16 != 0 {
        return Err(BinaryError {
            input: source.to_string(),
            offset: 16,
            message: "Q1 mip dimensions must be multiples of 16".to_string(),
        });
    }
    let offsets = [reader.u32()?, reader.u32()?, reader.u32()?, reader.u32()?];
    if offsets == [0, 0, 0, 0] {
        return Ok(MipTexture::External { name, width, height });
    }
    let scales = [1u32, 2, 4, 8];
    let mut levels: [&[u8]; 4] = [&[], &[], &[], &[]];
    for (index, level) in levels.iter_mut().enumerate() {
        *level = mip_level(
            &reader,
            source,
            width / scales[index],
            height / scales[index],
            offsets[index],
        )?;
    }
    Ok(MipTexture::Embedded {
        name,
        width,
        height,
        levels,
    })
}

/// Decoded QPIC image (`IndexedImage`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QpicImage<'a> {
    /// Width.
    pub width: i32,
    /// Height.
    pub height: i32,
    /// Palette indices borrowed from the input.
    pub indices: &'a [u8],
}

/// Decode a QPIC image (`decodeQpic`).
pub fn decode_qpic<'a>(data: &'a [u8], source: &str) -> Result<QpicImage<'a>, BinaryError> {
    let mut reader = BinaryReader::new(data, source);
    let width = reader.i32()?;
    let height = reader.i32()?;
    let length = dimensions(source, i64::from(width), i64::from(height))? as usize;
    let indices = reader.view(reader.offset(), length)?;
    Ok(QpicImage { width, height, indices })
}

/// WAD archive kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WadKind {
    /// Quake WAD2.
    Wad2,
    /// Half-Life WAD3.
    Wad3,
}

/// WAD lump (`WadLump`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WadLump<'a> {
    /// Lowercased lump name.
    pub name: String,
    /// Lump type.
    pub type_id: u8,
    /// Compression (only 0 is supported).
    pub compression: u8,
    /// File offset.
    pub offset: i32,
    /// On-disk size.
    pub disk_size: i32,
    /// Decoded size.
    pub byte_len: i32,
    /// Lump bytes borrowed from the input.
    pub bytes: &'a [u8],
}

/// WAD image archive (`WadImageArchive`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WadArchive<'a> {
    /// Archive kind.
    pub kind: WadKind,
    /// Lumps.
    pub lumps: Vec<WadLump<'a>>,
}

/// Decode a WAD archive (`decodeWad`).
pub fn decode_wad<'a>(data: &'a [u8], source: &str) -> Result<WadArchive<'a>, BinaryError> {
    let mut reader = BinaryReader::new(data, source);
    let kind = reader.fixed_byte_string(4)?;
    let kind = match kind.as_str() {
        "WAD2" => WadKind::Wad2,
        "WAD3" => WadKind::Wad3,
        _ => {
            return Err(BinaryError {
                input: source.to_string(),
                offset: 0,
                message: "Expected WAD2 or WAD3".to_string(),
            });
        }
    };
    let lump_count = reader.i32()?;
    let directory_offset = reader.i32()?;
    if lump_count < 0 {
        return Err(BinaryError {
            input: source.to_string(),
            offset: 4,
            message: "Negative WAD lump count".to_string(),
        });
    }
    if directory_offset < 0 {
        return Err(BinaryError {
            input: source.to_string(),
            offset: 8,
            message: "Negative WAD directory offset".to_string(),
        });
    }
    let mut directory = reader.section(directory_offset as usize, lump_count as usize * 32)?;
    let mut lumps = Vec::with_capacity(lump_count as usize);
    for index in 0..lump_count as usize {
        let start = directory.i32()?;
        let disk_size = directory.i32()?;
        let byte_len = directory.i32()?;
        let type_id = directory.u8()?;
        let compression = directory.u8()?;
        directory.skip(2)?;
        let name = directory.fixed_byte_string(16)?.to_lowercase();
        if byte_len < 0 {
            return Err(BinaryError {
                input: source.to_string(),
                offset: directory_offset as usize + index * 32 + 8,
                message: "Negative WAD decoded size".to_string(),
            });
        }
        if start < 0 || disk_size < 0 {
            return Err(BinaryError {
                input: source.to_string(),
                offset: directory_offset as usize + index * 32,
                message: "Negative WAD lump range".to_string(),
            });
        }
        let bytes = reader.view(start as usize, disk_size as usize)?;
        lumps.push(WadLump {
            name,
            type_id,
            compression,
            offset: start,
            disk_size,
            byte_len,
            bytes,
        });
    }
    Ok(WadArchive { kind, lumps })
}

/// Decoded WAD image (`WadImage`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WadImage<'a> {
    /// QPIC image (type 66).
    Qpic(QpicImage<'a>),
    /// Mip texture with an optional WAD3 palette.
    Miptex {
        /// Texture.
        texture: MipTexture<'a>,
        /// Palette bytes (768) for embedded WAD3 textures.
        palette: Option<&'a [u8]>,
    },
    /// Standalone 768-byte palette (type 64).
    Palette {
        /// Colors.
        colors: &'a [u8],
    },
    /// Undecoded lump.
    Raw(WadLump<'a>),
}

/// Decode one WAD lump image (`decodeWadImage`).
pub fn decode_wad_image<'a>(
    archive: &WadArchive<'a>,
    lump: &WadLump<'a>,
    source: &str,
) -> Result<WadImage<'a>, BinaryError> {
    if lump.compression != 0 {
        return Err(BinaryError {
            input: source.to_string(),
            offset: lump.offset as usize,
            message: format!("WAD compression {} is unsupported", lump.compression),
        });
    }
    if lump.disk_size != lump.byte_len {
        return Err(BinaryError {
            input: source.to_string(),
            offset: lump.offset as usize,
            message: "Uncompressed WAD lump size mismatch".to_string(),
        });
    }
    if lump.type_id == 66 {
        return Ok(WadImage::Qpic(decode_qpic(
            lump.bytes,
            &format!("{source}:{}", lump.name),
        )?));
    }
    if lump.type_id == 68 || (archive.kind == WadKind::Wad3 && lump.type_id == 67) {
        let texture = decode_q1_mip_texture(lump.bytes, &format!("{source}:{}", lump.name))?;
        let mut palette = None;
        if archive.kind == WadKind::Wad3 {
            if let MipTexture::Embedded { levels, .. } = &texture {
                let mut reader = BinaryReader::new(lump.bytes, source);
                reader.seek(36)?;
                let end = reader.u32()? as usize + levels[3].len();
                reader.seek(end)?;
                let count = reader.u16()?;
                if count != 256 {
                    return Err(BinaryError {
                        input: source.to_string(),
                        offset: end,
                        message: "WAD3 miptex requires 256 palette colors".to_string(),
                    });
                }
                palette = Some(reader.view(reader.offset(), 256 * 3)?);
            }
        }
        return Ok(WadImage::Miptex { texture, palette });
    }
    if lump.type_id == 64 && lump.bytes.len() == 768 {
        return Ok(WadImage::Palette { colors: lump.bytes });
    }
    Ok(WadImage::Raw(lump.clone()))
}

/// Convert palette indices to RGBA (`q1TextureRgba`).
pub fn texture_rgba(pixels: &[u8], palette: &[u8], transparent_index: Option<u8>) -> Result<Vec<u8>, BinaryError> {
    if palette.len() != 768 {
        return Err(BinaryError {
            input: "<palette>".to_string(),
            offset: 0,
            message: format!("Quake palette must contain 768 bytes, got {}", palette.len()),
        });
    }
    let mut output = Vec::with_capacity(pixels.len() * 4);
    for pixel in pixels {
        let base = usize::from(*pixel) * 3;
        output.push(palette[base]);
        output.push(palette[base + 1]);
        output.push(palette[base + 2]);
        output.push(u8::from(transparent_index != Some(*pixel)) * 255);
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_core::binary::BinaryWriter;

    fn miptex_bytes() -> Vec<u8> {
        let mut writer = BinaryWriter::new(512);
        let mut name = [0u8; 16];
        name[..4].copy_from_slice(b"test");
        writer.bytes(&name).unwrap();
        writer.u32(16).unwrap();
        writer.u32(16).unwrap();
        writer.u32(40).unwrap();
        writer.u32(296).unwrap();
        writer.u32(360).unwrap();
        writer.u32(376).unwrap();
        writer.bytes(&[1u8; 256]).unwrap();
        writer.bytes(&[2u8; 64]).unwrap();
        writer.bytes(&[3u8; 16]).unwrap();
        writer.bytes(&[4u8; 4]).unwrap();
        writer.finish()
    }

    fn wad2_fixture() -> Vec<u8> {
        let mut qpic = BinaryWriter::new(16);
        qpic.i32(2).unwrap();
        qpic.i32(2).unwrap();
        qpic.bytes(&[5, 6, 7, 8]).unwrap();
        let qpic = qpic.finish();
        let miptex = miptex_bytes();
        let mut writer = BinaryWriter::new(2048);
        writer.bytes(b"WAD2").unwrap();
        writer.i32(3).unwrap();
        let dir_offset = 12 + qpic.len() + miptex.len() + 768;
        writer.i32(dir_offset as i32).unwrap();
        let qpic_offset = writer.offset();
        writer.bytes(&qpic).unwrap();
        let miptex_offset = writer.offset();
        writer.bytes(&miptex).unwrap();
        let palette_offset = writer.offset();
        writer.bytes(&[9u8; 768]).unwrap();
        assert_eq!(writer.offset(), dir_offset);
        for (offset, len, type_id, name) in [
            (qpic_offset, qpic.len(), 66u8, "CONCHARS"),
            (miptex_offset, miptex.len(), 68u8, "TEST"),
            (palette_offset, 768usize, 64u8, "PALETTE"),
        ] {
            writer.i32(offset as i32).unwrap();
            writer.i32(len as i32).unwrap();
            writer.i32(len as i32).unwrap();
            writer.u8(type_id).unwrap();
            writer.u8(0).unwrap();
            writer.u16(0).unwrap();
            let mut lump_name = [0u8; 16];
            lump_name[..name.len()].copy_from_slice(name.as_bytes());
            writer.bytes(&lump_name).unwrap();
        }
        writer.finish()
    }

    #[test]
    fn wad2_round_trip() {
        let bytes = wad2_fixture();
        let archive = decode_wad(&bytes, "<test>").unwrap();
        assert_eq!(archive.kind, WadKind::Wad2);
        assert_eq!(archive.lumps.len(), 3);
        assert_eq!(archive.lumps[0].name, "conchars");
        assert_eq!(archive.lumps[1].name, "test");

        match decode_wad_image(&archive, &archive.lumps[0], "<test>").unwrap() {
            WadImage::Qpic(image) => {
                assert_eq!((image.width, image.height), (2, 2));
                assert_eq!(image.indices, &[5, 6, 7, 8]);
            }
            other => panic!("expected qpic, got {other:?}"),
        }
        match decode_wad_image(&archive, &archive.lumps[1], "<test>").unwrap() {
            WadImage::Miptex { texture, palette } => {
                assert_eq!(palette, None);
                match texture {
                    MipTexture::Embedded { width, levels, .. } => {
                        assert_eq!(width, 16);
                        assert_eq!(levels[0].len(), 256);
                        assert_eq!(levels[3], &[4u8; 4]);
                    }
                    MipTexture::External { .. } => panic!("expected embedded miptex"),
                }
            }
            other => panic!("expected miptex, got {other:?}"),
        }
        assert!(matches!(
            decode_wad_image(&archive, &archive.lumps[2], "<test>").unwrap(),
            WadImage::Palette { .. }
        ));

        let palette: Vec<u8> = (0..768u16).map(|value| (value % 256) as u8).collect();
        let rgba = texture_rgba(&[0, 1], &palette, Some(1)).unwrap();
        assert_eq!(rgba, vec![0, 1, 2, 255, 3, 4, 5, 0]);
        assert!(texture_rgba(&[0], &[0u8; 100], None).is_err());
    }

    #[test]
    fn wad3_miptex_carries_palette() {
        let mut miptex = miptex_bytes();
        miptex.extend_from_slice(&256u16.to_le_bytes());
        miptex.extend_from_slice(&[11u8; 768]);
        let mut writer = BinaryWriter::new(2048);
        writer.bytes(b"WAD3").unwrap();
        writer.i32(1).unwrap();
        writer.i32((12 + miptex.len()) as i32).unwrap();
        writer.bytes(&miptex).unwrap();
        writer.i32(12).unwrap();
        writer.i32(miptex.len() as i32).unwrap();
        writer.i32(miptex.len() as i32).unwrap();
        writer.u8(67).unwrap();
        writer.u8(0).unwrap();
        writer.u16(0).unwrap();
        let mut name = [0u8; 16];
        name[..4].copy_from_slice(b"wall");
        writer.bytes(&name).unwrap();
        let bytes = writer.finish();

        let archive = decode_wad(&bytes, "<test>").unwrap();
        match decode_wad_image(&archive, &archive.lumps[0], "<test>").unwrap() {
            WadImage::Miptex { palette, .. } => {
                assert_eq!(palette.unwrap().len(), 768);
            }
            other => panic!("expected miptex, got {other:?}"),
        }
    }

    #[test]
    fn wad_rejects_bad_input() {
        assert!(decode_wad(b"WAD9\x00\x00\x00\x00\x00\x00\x00\x00", "<test>").is_err());
        let mut bad_count = wad2_fixture();
        bad_count[4] = 0xff;
        bad_count[5] = 0xff;
        bad_count[6] = 0xff;
        bad_count[7] = 0xff;
        assert!(decode_wad(&bad_count, "<test>").is_err());

        let mut compressed = wad2_fixture();
        // Compression byte of the first directory entry.
        let dir = 12 + 12 + 380 + 768;
        compressed[dir + 13] = 1;
        let archive = decode_wad(&compressed, "<test>").unwrap();
        assert!(decode_wad_image(&archive, &archive.lumps[0], "<test>").is_err());

        let mut bad_mip = miptex_bytes();
        bad_mip[16] = 15; // width not a multiple of 16
        assert!(decode_q1_mip_texture(&bad_mip, "<test>").is_err());
        let external = {
            let mut writer = BinaryWriter::new(64);
            writer.bytes(&[0u8; 16]).unwrap();
            writer.u32(16).unwrap();
            writer.u32(16).unwrap();
            writer.u32(0).unwrap();
            writer.u32(0).unwrap();
            writer.u32(0).unwrap();
            writer.u32(0).unwrap();
            writer.finish()
        };
        assert!(matches!(
            decode_q1_mip_texture(&external, "<test>").unwrap(),
            MipTexture::External { .. }
        ));
    }
}
