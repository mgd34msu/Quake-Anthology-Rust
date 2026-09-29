//! Owned WAD2/WAD3 image archives.
//!
//! Donor: `decodeWad`/`decodeWadImage` in `src/formats/images/wad.ts`
//! (adapted from Q1 wad.c/wad.h, GPL-2.0-or-later).
//! Archive parsing delegates to the borrowed [`crate::wad`]
//! decoders; lump images delegate to the owned
//! [`super::indexed`] converters.

use qa_core::binary::BinaryReader;

use super::indexed::{decode_q1_mip_texture, decode_qpic, IndexedImage, Q1MipTexture};
use super::{fail, ContentError};
use crate::wad as borrowed;

/// Owned WAD archive kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OwnedWadKind {
    /// Quake WAD2.
    Wad2,
    /// Half-Life WAD3.
    Wad3,
}

/// Owned WAD lump (`WadLump`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwnedWadLump {
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
    /// Owned lump bytes.
    pub bytes: Vec<u8>,
}

/// Owned WAD image archive (`WadImageArchive`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwnedWadArchive {
    /// Archive kind.
    pub kind: OwnedWadKind,
    /// Lumps.
    pub lumps: Vec<OwnedWadLump>,
}

/// Owned decoded WAD image (`WadImage`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OwnedWadImage {
    /// QPIC image (type 66).
    Qpic(IndexedImage),
    /// Mip texture with an optional WAD3 palette.
    Miptex {
        /// Texture.
        texture: Q1MipTexture,
        /// Palette bytes (768) for embedded WAD3 textures.
        palette: Option<Vec<u8>>,
    },
    /// Standalone 768-byte palette (type 64).
    Palette {
        /// Colors.
        colors: Vec<u8>,
    },
    /// Undecoded lump.
    Raw(OwnedWadLump),
}

/// Decode an owned WAD archive (`decodeWad`).
pub fn decode_wad(bytes: &[u8], source: &str) -> Result<OwnedWadArchive, ContentError> {
    let archive = borrowed::decode_wad(bytes, source)?;
    Ok(OwnedWadArchive {
        kind: match archive.kind {
            borrowed::WadKind::Wad2 => OwnedWadKind::Wad2,
            borrowed::WadKind::Wad3 => OwnedWadKind::Wad3,
        },
        lumps: archive
            .lumps
            .iter()
            .map(|lump| OwnedWadLump {
                name: lump.name.clone(),
                type_id: lump.type_id,
                compression: lump.compression,
                offset: lump.offset,
                disk_size: lump.disk_size,
                byte_len: lump.byte_len,
                bytes: lump.bytes.to_vec(),
            })
            .collect(),
    })
}

/// Decode one owned WAD lump image (`decodeWadImage`).
pub fn decode_wad_image(
    archive: &OwnedWadArchive,
    lump: &OwnedWadLump,
    source: &str,
) -> Result<OwnedWadImage, ContentError> {
    let at = lump.offset.max(0) as usize;
    if lump.compression != 0 {
        return Err(fail(
            source,
            at,
            format!("WAD compression {} is unsupported", lump.compression),
        ));
    }
    if lump.disk_size != lump.byte_len {
        return Err(fail(source, at, "Uncompressed WAD lump size mismatch".to_string()));
    }
    if lump.type_id == 66 {
        return Ok(OwnedWadImage::Qpic(decode_qpic(
            &lump.bytes,
            &format!("{source}:{}", lump.name),
        )?));
    }
    if lump.type_id == 68 || (archive.kind == OwnedWadKind::Wad3 && lump.type_id == 67) {
        let texture = decode_q1_mip_texture(&lump.bytes, &format!("{source}:{}", lump.name))?;
        let mut palette = None;
        if archive.kind == OwnedWadKind::Wad3 {
            if let Q1MipTexture::Embedded { levels, .. } = &texture {
                let mut reader = BinaryReader::new(&lump.bytes, source);
                reader.seek(36)?;
                let end = reader.u32()? as usize + levels[3].indices.len();
                reader.seek(end)?;
                let count = reader.u16()?;
                if count != 256 {
                    return Err(fail(source, end, "WAD3 miptex requires 256 palette colors".to_string()));
                }
                palette = Some(reader.bytes(usize::from(count) * 3)?);
            }
        }
        return Ok(OwnedWadImage::Miptex { texture, palette });
    }
    if lump.type_id == 64 && lump.bytes.len() == 768 {
        return Ok(OwnedWadImage::Palette {
            colors: lump.bytes.clone(),
        });
    }
    Ok(OwnedWadImage::Raw(lump.clone()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn miptex_bytes() -> Vec<u8> {
        let mut out = vec![0u8; 16];
        out[..4].copy_from_slice(b"test");
        out.extend_from_slice(&16u32.to_le_bytes());
        out.extend_from_slice(&16u32.to_le_bytes());
        for offset in [40u32, 296, 360, 376] {
            out.extend_from_slice(&offset.to_le_bytes());
        }
        out.extend_from_slice(&[1u8; 256]);
        out.extend_from_slice(&[2u8; 64]);
        out.extend_from_slice(&[3u8; 16]);
        out.extend_from_slice(&[4u8; 4]);
        out
    }

    fn archive_bytes(kind: &[u8; 4], lumps: &[(Vec<u8>, u8, &str)]) -> Vec<u8> {
        let mut out = kind.to_vec();
        out.extend_from_slice(&(lumps.len() as i32).to_le_bytes());
        let dir = 12 + lumps.iter().map(|(bytes, _, _)| bytes.len()).sum::<usize>();
        out.extend_from_slice(&(dir as i32).to_le_bytes());
        let mut offsets = Vec::new();
        for (bytes, _, _) in lumps {
            offsets.push(out.len());
            out.extend_from_slice(bytes);
        }
        for ((bytes, type_id, name), offset) in lumps.iter().zip(offsets) {
            out.extend_from_slice(&(offset as i32).to_le_bytes());
            out.extend_from_slice(&(bytes.len() as i32).to_le_bytes());
            out.extend_from_slice(&(bytes.len() as i32).to_le_bytes());
            out.push(*type_id);
            out.push(0);
            out.extend_from_slice(&[0u8; 2]);
            let mut lump_name = [0u8; 16];
            lump_name[..name.len()].copy_from_slice(name.as_bytes());
            out.extend_from_slice(&lump_name);
        }
        out
    }

    fn qpic_bytes() -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&2i32.to_le_bytes());
        out.extend_from_slice(&2i32.to_le_bytes());
        out.extend_from_slice(&[5, 6, 7, 8]);
        out
    }

    #[test]
    fn wad2_lump_kinds() {
        let bytes = archive_bytes(
            b"WAD2",
            &[
                (qpic_bytes(), 66, "CONCHARS"),
                (miptex_bytes(), 68, "TEST"),
                (vec![9u8; 768], 64, "PALETTE"),
                (vec![1, 2, 3], 7, "RAW"),
            ],
        );
        let archive = decode_wad(&bytes, "<test>").unwrap();
        assert_eq!(archive.kind, OwnedWadKind::Wad2);
        assert_eq!(archive.lumps.len(), 4);
        assert_eq!(archive.lumps[0].name, "conchars");

        match decode_wad_image(&archive, &archive.lumps[0].clone(), "<test>").unwrap() {
            OwnedWadImage::Qpic(image) => {
                assert_eq!((image.width, image.height), (2, 2));
                assert_eq!(image.indices, vec![5, 6, 7, 8]);
            }
            other => panic!("expected qpic, got {other:?}"),
        }
        match decode_wad_image(&archive, &archive.lumps[1].clone(), "<test>").unwrap() {
            OwnedWadImage::Miptex { texture, palette } => {
                assert_eq!(palette, None);
                assert!(matches!(texture, Q1MipTexture::Embedded { .. }));
            }
            other => panic!("expected miptex, got {other:?}"),
        }
        match decode_wad_image(&archive, &archive.lumps[2].clone(), "<test>").unwrap() {
            OwnedWadImage::Palette { colors } => assert_eq!(colors.len(), 768),
            other => panic!("expected palette, got {other:?}"),
        }
        assert!(matches!(
            decode_wad_image(&archive, &archive.lumps[3].clone(), "<test>").unwrap(),
            OwnedWadImage::Raw(_)
        ));
    }

    #[test]
    fn wad3_miptex_carries_palette() {
        let mut miptex = miptex_bytes();
        miptex.extend_from_slice(&256u16.to_le_bytes());
        miptex.extend_from_slice(&[11u8; 768]);
        let bytes = archive_bytes(b"WAD3", &[(miptex, 67, "WALL")]);
        let archive = decode_wad(&bytes, "<test>").unwrap();
        assert_eq!(archive.kind, OwnedWadKind::Wad3);
        match decode_wad_image(&archive, &archive.lumps[0].clone(), "<test>").unwrap() {
            OwnedWadImage::Miptex { palette, .. } => {
                assert_eq!(palette.unwrap(), vec![11u8; 768]);
            }
            other => panic!("expected miptex, got {other:?}"),
        }
    }

    #[test]
    fn rejects_compressed_lump() {
        let bytes = archive_bytes(b"WAD2", &[(qpic_bytes(), 66, "CONCHARS")]);
        let mut archive = decode_wad(&bytes, "<test>").unwrap();
        archive.lumps[0].compression = 1;
        let error = decode_wad_image(&archive, &archive.lumps[0].clone(), "<test>").unwrap_err();
        assert_eq!(error.message, "WAD compression 1 is unsupported");
    }
}
