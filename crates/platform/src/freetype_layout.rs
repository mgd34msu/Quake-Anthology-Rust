//! Public FreeType 2 record layouts (`freetype.h` / `ftimage.h`).
//!
//! Port of donor `src/platform/freetype-layout.ts`. Only the two qualified
//! ABIs are described: 64-bit little-endian LP64 (Linux/macOS) and Windows
//! x64 LLP64. Unsupported hosts fail here, before any native memory is read.

use crate::error::{Error, Result};

/// Width of C `long` on the qualified ABI.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LongWidth {
    /// 32-bit `long` (Windows x64 LLP64).
    L32,
    /// 64-bit `long` (LP64).
    L64,
}

/// Byte offsets into public FreeType records.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FreeTypeLayout {
    /// Size of C `long` in bytes.
    pub long_bytes: usize,
    /// Offset of `glyph` within `FT_FaceRec`.
    pub face_glyph: usize,
    /// Offset of `format` within `FT_GlyphSlotRec`.
    pub slot_format: usize,
    /// Offset of `bitmap` within `FT_GlyphSlotRec`.
    pub slot_bitmap: usize,
    /// Offset of `outline` within `FT_GlyphSlotRec`.
    pub slot_outline: usize,
}

impl FreeTypeLayout {
    /// LP64 layout (Linux/macOS x64/arm64, little-endian).
    pub const LP64: Self = Self {
        long_bytes: 8,
        face_glyph: 152,
        slot_format: 144,
        slot_bitmap: 152,
        slot_outline: 200,
    };
    /// LLP64 layout (Windows x64, little-endian).
    pub const LLP64: Self = Self {
        long_bytes: 4,
        face_glyph: 120,
        slot_format: 96,
        slot_bitmap: 104,
        slot_outline: 152,
    };

    /// Width of C `long` for this layout.
    #[must_use]
    pub fn long_width(self) -> LongWidth {
        if self.long_bytes == 4 {
            LongWidth::L32
        } else {
            LongWidth::L64
        }
    }
}

/// Select the record layout for a (`platform`, `arch`, little-endian) triple,
/// or `None` before loading anything on unsupported ABIs.
#[must_use]
pub fn free_type_layout(platform: &str, arch: &str, little_endian: bool) -> Option<FreeTypeLayout> {
    if !little_endian {
        return None;
    }
    if platform == "win32" && arch == "x64" {
        return Some(FreeTypeLayout::LLP64);
    }
    if (platform == "linux" || platform == "darwin") && (arch == "x64" || arch == "arm64") {
        return Some(FreeTypeLayout::LP64);
    }
    None
}

/// Layout for the running host, or `None` on unqualified ABIs.
#[must_use]
pub fn host_free_type_layout() -> Option<FreeTypeLayout> {
    let platform = if cfg!(windows) {
        "win32"
    } else if cfg!(target_os = "macos") {
        "darwin"
    } else {
        "linux"
    };
    // Donor arch names are `x64`/`arm64`; Rust reports `x86_64`/`aarch64`.
    let arch = match std::env::consts::ARCH {
        "x86_64" => "x64",
        "aarch64" => "arm64",
        other => other,
    };
    free_type_layout(platform, arch, cfg!(target_endian = "little"))
}

/// Read a C `long` metric at `offset`, narrowing LP64 values to source int32.
pub fn free_type_metric(view: &[u8], offset: usize, layout: FreeTypeLayout) -> Result<i32> {
    let end = offset
        .checked_add(layout.long_bytes)
        .ok_or_else(|| Error::OutOfRange("FreeType metric offset overflows".to_string()))?;
    let bytes = view
        .get(offset..end)
        .ok_or_else(|| Error::OutOfRange("FreeType metric is outside the record".to_string()))?;
    if layout.long_bytes == 4 {
        let mut raw = [0u8; 4];
        raw.copy_from_slice(bytes);
        return Ok(i32::from_le_bytes(raw));
    }
    let mut raw = [0u8; 8];
    raw.copy_from_slice(bytes);
    let value = i64::from_le_bytes(raw);
    i32::try_from(value).map_err(|_| Error::OutOfRange("FreeType metric exceeds source int32".to_string()))
}

/// Validate native bitmap dimensions and return the allocation length.
///
/// Rejects malformed row lengths before allocating or copying, mirroring the
/// donor's pixel-mode row rules and 64 MiB cap.
pub fn free_type_bitmap_length(width: i64, height: i64, pitch: i64, pixel_mode: u8) -> Result<usize> {
    if width < 0 || height < 0 || pitch.unsigned_abs() > 0x7fff_ffff {
        return Err(Error::OutOfRange("invalid FreeType bitmap dimensions".to_string()));
    }
    let row_bytes: i64 = match pixel_mode {
        0 => {
            if width != 0 && height != 0 {
                return Err(Error::OutOfRange(
                    "nonempty FreeType bitmap has no pixel mode".to_string(),
                ));
            }
            0
        }
        1 => (width + 7) / 8,
        2 | 5 | 6 => width,
        3 => (width + 3) / 4,
        4 => (width + 1) / 2,
        7 => width
            .checked_mul(4)
            .ok_or_else(|| Error::OutOfRange("FreeType bitmap row overflows".to_string()))?,
        _ => {
            return Err(Error::OutOfRange("unsupported FreeType pixel mode".to_string()));
        }
    };
    let stride = pitch.abs();
    let length = stride
        .checked_mul(height)
        .ok_or_else(|| Error::OutOfRange("FreeType bitmap exceeds row bounds".to_string()))?;
    if (height > 0 && stride < row_bytes) || length > 64 * 1024 * 1024 {
        return Err(Error::OutOfRange(
            "FreeType bitmap exceeds row or 64 MiB allocation bounds".to_string(),
        ));
    }
    usize::try_from(length).map_err(|_| Error::OutOfRange("FreeType bitmap length exceeds address space".to_string()))
}

/// Reverse rows copied from the lowest address of a negative-pitch bitmap.
pub fn normalize_free_type_bitmap_rows(bytes: &mut [u8], height: usize, pitch: i64) -> Result<()> {
    let stride = usize::try_from(pitch.abs())
        .map_err(|_| Error::OutOfRange("FreeType bitmap storage does not match its rows".to_string()))?;
    let expected = stride
        .checked_mul(height)
        .ok_or_else(|| Error::OutOfRange("FreeType bitmap storage overflows".to_string()))?;
    if bytes.len() != expected {
        return Err(Error::OutOfRange(
            "FreeType bitmap storage does not match its rows".to_string(),
        ));
    }
    if pitch >= 0 || height < 2 {
        return Ok(());
    }
    for top in 0..height / 2 {
        let bottom = height - 1 - top;
        let (lower, upper) = bytes.split_at_mut(bottom * stride);
        let top_row = &mut lower[top * stride..(top + 1) * stride];
        let bottom_row = &mut upper[..stride];
        top_row.swap_with_slice(bottom_row);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout_selection_matches_donor() {
        assert_eq!(free_type_layout("linux", "x64", true), Some(FreeTypeLayout::LP64));
        assert_eq!(free_type_layout("darwin", "arm64", true), Some(FreeTypeLayout::LP64));
        assert_eq!(free_type_layout("win32", "x64", true), Some(FreeTypeLayout::LLP64));
        assert_eq!(free_type_layout("linux", "x64", false), None);
        assert_eq!(free_type_layout("win32", "arm64", true), None);
        assert_eq!(free_type_layout("freebsd", "x64", true), None);
    }

    #[test]
    fn metric_narrows_lp64_to_int32() {
        let mut record = vec![0u8; 160];
        record[48..56].copy_from_slice(&12345678i64.to_le_bytes());
        assert_eq!(free_type_metric(&record, 48, FreeTypeLayout::LP64).unwrap(), 12_345_678);
        record[48..56].copy_from_slice(&0x1_0000_0000i64.to_le_bytes());
        assert!(free_type_metric(&record, 48, FreeTypeLayout::LP64).is_err());
        let mut narrow = vec![0u8; 160];
        narrow[48..52].copy_from_slice(&(-42i32).to_le_bytes());
        assert_eq!(free_type_metric(&narrow, 48, FreeTypeLayout::LLP64).unwrap(), -42);
        assert!(free_type_metric(&narrow, 200, FreeTypeLayout::LLP64).is_err());
    }

    #[test]
    fn bitmap_length_enforces_rows_and_cap() {
        assert_eq!(free_type_bitmap_length(10, 4, 12, 2).unwrap(), 48);
        assert_eq!(free_type_bitmap_length(9, 1, 2, 1).unwrap(), 2);
        assert_eq!(free_type_bitmap_length(0, 0, 0, 0).unwrap(), 0);
        assert!(free_type_bitmap_length(4, 4, 4, 0).is_err());
        assert!(free_type_bitmap_length(10, 4, 8, 2).is_err());
        assert!(free_type_bitmap_length(10, 4, 12, 9).is_err());
        assert!(free_type_bitmap_length(1 << 20, 1 << 20, 1 << 20, 2).is_err());
        assert!(free_type_bitmap_length(-1, 4, 12, 2).is_err());
    }

    #[test]
    fn row_normalization_reverses_negative_pitch() {
        let mut bytes = vec![1u8, 2, 3, 4, 5, 6];
        normalize_free_type_bitmap_rows(&mut bytes, 3, -2).unwrap();
        assert_eq!(bytes, vec![5, 6, 3, 4, 1, 2]);
        let mut same = vec![1u8, 2, 3, 4];
        normalize_free_type_bitmap_rows(&mut same, 2, 2).unwrap();
        assert_eq!(same, vec![1, 2, 3, 4]);
        let mut bad = vec![0u8; 3];
        assert!(normalize_free_type_bitmap_rows(&mut bad, 2, 2).is_err());
    }
}
