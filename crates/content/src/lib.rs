//! Content foundation: resource paths, VFS-adjacent helpers, and Quake
//! format decoders (BSP maps, MDL/MD2/SPR models, WAD archives).
//!
//! Donor provenance: `src/content` plus `src/formats` (including
//! `src/formats/images`, ported to [`images`]).
//!
//! Quake64 (Q1 magic `0x51363420`) classifies as [`BspKind::Q1`] but stays
//! rejected by the Q1 reader: packed lighting and BSPX extensions are out
//! of scope, as before.

use qa_core::binary::{BinaryError, BinaryReader};

pub mod archive;
pub mod bsp;
pub mod bsp2;
pub mod bsp3;
pub mod catalog;
pub mod common;
pub mod composition;
pub mod contract;
pub mod hash;
pub mod held_weapon;
pub mod images;
pub mod item_icon;
pub mod lod;
pub mod md2;
pub mod md3;
pub mod md4;
pub mod md5;
pub mod mdl;
pub mod model_attachment;
pub mod model_text;
pub mod mods;
pub mod monsters;
pub mod mounts;
pub mod normals;
pub mod paths;
pub mod q3_base;
pub mod q3_foundation;
pub mod q3_game_state;
pub mod q3_present_hud;
pub mod q3_present_scene;
pub mod q3_supply;
pub mod q3anim;
pub mod q3scene;
pub mod quaternion;
pub mod replacements;
pub mod spr;
pub mod user_data;
pub mod value;
pub mod wad;

/// BSP codec family (`classifyBsp` in `src/formats/bsp-kind.ts`).
///
/// Container ownership does not determine a map's stored codec.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BspKind {
    /// Quake BSP29 / BSP2 / 2PSB / Quake64.
    Q1,
    /// Quake II IBSP/QBSP version 38.
    Q2,
    /// Quake III IBSP versions 44/46.
    Q3,
}

/// Classify a map by its stored codec (`classifyBsp`).
pub fn classify_bsp(data: &[u8], source: &str) -> Result<BspKind, BinaryError> {
    let mut reader = BinaryReader::new(data, source);
    let magic = reader.u32()?;
    if magic == 29 || magic == 0x3250_5342 || magic == 0x4253_5032 || magic == 0x5136_3420 {
        return Ok(BspKind::Q1);
    }
    if magic == 0x5053_4249 || magic == 0x5053_4251 {
        let version = reader.u32()?;
        if version == 38 {
            return Ok(BspKind::Q2);
        }
        if magic == 0x5053_4249 && (version == 44 || version == 46) {
            return Ok(BspKind::Q3);
        }
        return Err(BinaryError {
            input: source.to_string(),
            offset: 4,
            message: format!("unsupported BSP version {version}"),
        });
    }
    Err(BinaryError {
        input: source.to_string(),
        offset: 0,
        message: format!("unsupported BSP identifier {magic}"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_families() {
        let mut q1 = [0u8; 8];
        q1[0] = 29;
        assert_eq!(classify_bsp(&q1, "<test>").unwrap(), BspKind::Q1);
        let quake64 = [0x20, 0x34, 0x36, 0x51, 0, 0, 0, 0];
        assert_eq!(classify_bsp(&quake64, "<test>").unwrap(), BspKind::Q1);
        let q2 = [b'I', b'B', b'S', b'P', 38, 0, 0, 0];
        assert_eq!(classify_bsp(&q2, "<test>").unwrap(), BspKind::Q2);
        let qbsp = [b'Q', b'B', b'S', b'P', 38, 0, 0, 0];
        assert_eq!(classify_bsp(&qbsp, "<test>").unwrap(), BspKind::Q2);
        let q3 = [b'I', b'B', b'S', b'P', 46, 0, 0, 0];
        assert_eq!(classify_bsp(&q3, "<test>").unwrap(), BspKind::Q3);
        let q3_test = [b'I', b'B', b'S', b'P', 44, 0, 0, 0];
        assert_eq!(classify_bsp(&q3_test, "<test>").unwrap(), BspKind::Q3);
        let bad_version = [b'I', b'B', b'S', b'P', 45, 0, 0, 0];
        let error = classify_bsp(&bad_version, "<test>").unwrap_err();
        assert_eq!(error.message, "unsupported BSP version 45");
        assert_eq!(error.offset, 4);
        let error = classify_bsp(b"XXXX....", "<test>").unwrap_err();
        assert!(
            error.message.contains("unsupported BSP identifier"),
            "{}",
            error.message
        );
        assert_eq!(error.offset, 0);
    }
}
