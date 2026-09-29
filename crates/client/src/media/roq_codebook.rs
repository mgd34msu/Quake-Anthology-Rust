//! RoQ codebooks: YUV tables and VQ codebook decode.
//!
//! Donor provenance: `src/media/roq-codebook.ts`
//! (`yuvToRgb565`, `yuvToRgba`, `SourceRoqCodebooks`).
//!
//! The donor shares one `Uint16Array` backing store per table between
//! the 16-bit allocation view and the byte codebook view. Only the byte
//! view is ever read back, so this port stores the three byte books
//! directly with identical little-endian write order.

use qa_core::binary::{BinaryError, BinaryReader};

use crate::ClientError;

fn map_err(error: BinaryError) -> ClientError {
    ClientError::BadMedia(error.to_string())
}

/// 2x2 book byte length (`256 * 16 * 4` samples).
pub const BOOK2_LEN: usize = 256 * 16 * 4 * 2;
/// 4x4 book byte length (`256 * 64 * 4` samples).
pub const BOOK4_LEN: usize = 256 * 64 * 4 * 2;
/// 8x8 book byte length (`256 * 256 * 4` samples).
pub const BOOK8_LEN: usize = 256 * 256 * 4 * 2;

fn chroma(coefficient: f32, value: i32, bias: f32) -> i32 {
    let factor = coefficient / 2.0 * 64.0 + 0.5;
    (factor * value as f32 + bias).trunc() as i32
}

/// Smoothed-double luma blend (`(near * 3 + far) / 4`).
fn blend3to1(near: u8, far: u8) -> u8 {
    ((u16::from(near) * 3 + u16::from(far)) / 4) as u8
}

/// `yuv_to_rgb`, retaining the separate 5/6/5-bit shifts.
#[must_use]
pub fn yuv_to_rgb565(y: u8, u: u8, v: u8) -> u16 {
    let yy = (i32::from(y) << 6) | (i32::from(y) >> 2);
    let x_u = 2 * i32::from(u) - 255;
    let x_v = 2 * i32::from(v) - 255;
    let r = ((yy + chroma(1.402, x_v, 32.0)) >> 9).clamp(0, 31);
    let g = ((yy + chroma(0.34414, -x_u, 0.0) + chroma(0.71414, -x_v, 32.0)) >> 8).clamp(0, 63);
    let b = ((yy + chroma(1.772, x_u, 32.0)) >> 9).clamp(0, 31);
    ((r << 11) | (g << 5) | b) as u16
}

/// `yuv_to_rgb24` represented as a little-endian RGBA word.
#[must_use]
pub fn yuv_to_rgba(y: u8, u: u8, v: u8) -> u32 {
    let yy = (i32::from(y) << 6) | (i32::from(y) >> 2);
    let x_u = 2 * i32::from(u) - 255;
    let x_v = 2 * i32::from(v) - 255;
    let r = ((yy + chroma(1.402, x_v, 32.0)) >> 6).clamp(0, 255);
    let g = ((yy + chroma(0.34414, -x_u, 0.0) + chroma(0.71414, -x_v, 32.0)) >> 6).clamp(0, 255);
    let b = ((yy + chroma(1.772, x_u, 32.0)) >> 6).clamp(0, 255);
    (r as u32) | ((g as u32) << 8) | ((b as u32) << 16) | 0xff00_0000
}

/// Codebook decode mode (`RoqCodebookMode`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RoqCodebookMode {
    /// Full 2x2 cells.
    Normal,
    /// Half-height cells.
    Half,
    /// Smoothed doubled cells.
    SmoothedDouble,
}

/// Codebook pixel format (`RoqCodebookFormat`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RoqCodebookFormat<'a> {
    /// One sample per pixel through a 256-entry gray lookup.
    Gray(&'a [u8]),
    /// Two samples per pixel (RGB565).
    Rgb565,
    /// Four samples per pixel (RGBA).
    Rgba,
}

/// Codebook decode profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RoqCodebookProfile {
    /// Source flags: zero flags require both tables.
    Source,
    /// Complete-file diagnostic: zero flags with no trailing VQ bytes
    /// decode only the 2x2 table.
    Diagnostic2x2,
}

/// Source global codebook allocations (`SourceRoqCodebooks`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceRoqCodebooks {
    book2: Vec<u8>,
    book4: Vec<u8>,
    book8: Vec<u8>,
}

impl Default for SourceRoqCodebooks {
    fn default() -> Self {
        Self::new()
    }
}

impl SourceRoqCodebooks {
    /// Zeroed tables.
    #[must_use]
    pub fn new() -> Self {
        Self {
            book2: vec![0u8; BOOK2_LEN],
            book4: vec![0u8; BOOK4_LEN],
            book8: vec![0u8; BOOK8_LEN],
        }
    }

    /// 2x2 book bytes.
    #[must_use]
    pub fn book2(&self) -> &[u8] {
        &self.book2
    }

    /// 4x4 book bytes.
    #[must_use]
    pub fn book4(&self) -> &[u8] {
        &self.book4
    }

    /// 8x8 book bytes.
    #[must_use]
    pub fn book8(&self) -> &[u8] {
        &self.book8
    }

    /// Mutable 2x2 book bytes (checkpoint restore).
    pub fn book2_mut(&mut self) -> &mut [u8] {
        &mut self.book2
    }

    /// Mutable 4x4 book bytes (checkpoint restore).
    pub fn book4_mut(&mut self) -> &mut [u8] {
        &mut self.book4
    }

    /// Mutable 8x8 book bytes (checkpoint restore).
    pub fn book8_mut(&mut self) -> &mut [u8] {
        &mut self.book8
    }

    /// Clear all tables.
    pub fn clear(&mut self) {
        self.book2.fill(0);
        self.book4.fill(0);
        self.book8.fill(0);
    }

    /// Decode a codebook chunk payload.
    pub fn decode(
        &mut self,
        reader: &mut BinaryReader<'_>,
        flags: u16,
        mode: RoqCodebookMode,
        format: RoqCodebookFormat<'_>,
        profile: RoqCodebookProfile,
    ) -> Result<(), ClientError> {
        let width = match format {
            RoqCodebookFormat::Gray(gray) => {
                if gray.len() < 256 {
                    return Err(ClientError::BadMedia(
                        "RoQ gray lookup requires at least 256 bytes".to_string(),
                    ));
                }
                1
            }
            RoqCodebookFormat::Rgb565 => 2,
            RoqCodebookFormat::Rgba => 4,
        };
        let count2 = {
            let high = usize::from(flags >> 8);
            if high == 0 {
                256
            } else {
                high
            }
        };
        let mut count4 = if flags == 0 { 256 } else { usize::from(flags & 255) };
        let cell_pixels = match mode {
            RoqCodebookMode::Half => 2,
            RoqCodebookMode::Normal => 4,
            RoqCodebookMode::SmoothedDouble => 8,
        };
        let mut cursor = 0usize;
        for _ in 0..count2 {
            if matches!(format, RoqCodebookFormat::Gray(_)) && mode != RoqCodebookMode::SmoothedDouble {
                let gray = match format {
                    RoqCodebookFormat::Gray(gray) => gray,
                    _ => unreachable!(),
                };
                // Gray normal/half publish each lookup before reading
                // the next source byte.
                let y = reader.u8().map_err(map_err)?;
                self.book2[cursor] = gray[usize::from(y)];
                cursor += 1;
                if mode == RoqCodebookMode::Half {
                    reader.skip(1).map_err(map_err)?;
                    let y = reader.u8().map_err(map_err)?;
                    self.book2[cursor] = gray[usize::from(y)];
                    cursor += 1;
                    reader.skip(3).map_err(map_err)?;
                } else {
                    for _ in 0..3 {
                        let y = reader.u8().map_err(map_err)?;
                        self.book2[cursor] = gray[usize::from(y)];
                        cursor += 1;
                    }
                    reader.skip(2).map_err(map_err)?;
                }
                continue;
            }
            let y0 = reader.u8().map_err(map_err)?;
            if mode == RoqCodebookMode::Half {
                reader.skip(1).map_err(map_err)?;
                let y2 = reader.u8().map_err(map_err)?;
                reader.skip(1).map_err(map_err)?;
                let u = reader.u8().map_err(map_err)?;
                let v = reader.u8().map_err(map_err)?;
                cursor = self.write_pixel(cursor, width, y0, u, v);
                cursor = self.write_pixel(cursor, width, y2, u, v);
                continue;
            }
            let y1 = reader.u8().map_err(map_err)?;
            let y2 = reader.u8().map_err(map_err)?;
            let y3 = reader.u8().map_err(map_err)?;
            let (u, v) = if matches!(format, RoqCodebookFormat::Gray(_)) {
                reader.skip(2).map_err(map_err)?;
                (0, 0)
            } else {
                (reader.u8().map_err(map_err)?, reader.u8().map_err(map_err)?)
            };
            if matches!(format, RoqCodebookFormat::Gray(_)) {
                let gray = match format {
                    RoqCodebookFormat::Gray(gray) => gray,
                    _ => unreachable!(),
                };
                // Smoothed-double gray publishes one lookup per write;
                // the chroma payload was skipped above.
                for y in [
                    y0,
                    y1,
                    blend3to1(y0, y2),
                    blend3to1(y1, y3),
                    blend3to1(y2, y0),
                    blend3to1(y3, y1),
                    y2,
                    y3,
                ] {
                    self.book2[cursor] = gray[usize::from(y)];
                    cursor += 1;
                }
                continue;
            }
            cursor = self.write_pixel(cursor, width, y0, u, v);
            cursor = self.write_pixel(cursor, width, y1, u, v);
            if mode == RoqCodebookMode::SmoothedDouble {
                for y in [
                    blend3to1(y0, y2),
                    blend3to1(y1, y3),
                    blend3to1(y2, y0),
                    blend3to1(y3, y1),
                ] {
                    cursor = self.write_pixel(cursor, width, y, u, v);
                }
            }
            cursor = self.write_pixel(cursor, width, y2, u, v);
            cursor = self.write_pixel(cursor, width, y3, u, v);
        }
        // Preserve the existing complete-file diagnostic extension;
        // source flags zero require both tables.
        if profile == RoqCodebookProfile::Diagnostic2x2 && flags == 0 && reader.remaining() == 0 {
            count4 = 0;
        }
        let row_pixels = if mode == RoqCodebookMode::Half { 1 } else { 2 };
        let rows = cell_pixels / row_pixels;
        let mut destination4 = 0usize;
        let mut destination8 = 0usize;
        for _ in 0..count4 * 2 {
            let mut left = usize::from(reader.u8().map_err(map_err)?) * cell_pixels * width;
            let mut right = usize::from(reader.u8().map_err(map_err)?) * cell_pixels * width;
            for _ in 0..rows {
                // VQ2TO4, or VQ2TO2 for half: join a/b and duplicate
                // every pixel and row into d.
                for start in [left, right] {
                    for column in 0..row_pixels {
                        let source = start + column * width;
                        for offset in 0..width {
                            let pixel = self.book2[source + offset];
                            self.book4[destination4] = pixel;
                            destination4 += 1;
                            self.book8[destination8 + offset] = pixel;
                            self.book8[destination8 + width + offset] = pixel;
                        }
                        destination8 += width * 2;
                    }
                }
                let row_bytes = row_pixels * width * 4;
                self.book8
                    .copy_within(destination8 - row_bytes..destination8, destination8);
                destination8 += row_bytes;
                left += row_pixels * width;
                right += row_pixels * width;
            }
        }
        Ok(())
    }

    fn write_pixel(&mut self, cursor: usize, width: usize, y: u8, u: u8, v: u8) -> usize {
        match width {
            2 => {
                let word = yuv_to_rgb565(y, u, v).to_le_bytes();
                self.book2[cursor] = word[0];
                self.book2[cursor + 1] = word[1];
            }
            _ => {
                let word = yuv_to_rgba(y, u, v).to_le_bytes();
                self.book2[cursor..cursor + 4].copy_from_slice(&word);
            }
        }
        cursor + width
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decode(payload: &[u8], flags: u16, mode: RoqCodebookMode, format: RoqCodebookFormat<'_>) -> SourceRoqCodebooks {
        let mut books = SourceRoqCodebooks::new();
        let mut reader = BinaryReader::new(payload, "<test>");
        books
            .decode(&mut reader, flags, mode, format, RoqCodebookProfile::Source)
            .unwrap();
        books
    }

    #[test]
    fn yuv_tables_match_donor() {
        // Oracle values from the donor under bun.
        let cases: &[(u8, u8, u8, u16, u32)] = &[
            (0, 128, 128, 0, 0xff01_0001),
            (255, 128, 128, 65535, 0xffff_ffff),
            (128, 0, 0, 2016, 0xff00_ff00),
            (128, 255, 255, 63519, 0xffff_00ff),
            (0, 0, 0, 1088, 0xff00_8b00),
            (255, 255, 255, 64447, 0xffff_75ff),
            (16, 100, 200, 28672, 0xff00_0077),
            (235, 16, 240, 65028, 0xff25_c2ff),
            (81, 90, 240, 61537, 0xff0e_0df1),
            (145, 54, 34, 3969, 0xff0e_f00d),
        ];
        for (y, u, v, rgb565, rgba) in cases {
            assert_eq!(yuv_to_rgb565(*y, *u, *v), *rgb565, "565 {y},{u},{v}");
            assert_eq!(yuv_to_rgba(*y, *u, *v), *rgba, "rgba {y},{u},{v}");
        }
    }

    #[test]
    fn normal_rgba_codebook_matches_donor() {
        let books = decode(
            &[16, 32, 48, 64, 100, 200, 0, 0, 0, 0],
            0x0101,
            RoqCodebookMode::Normal,
            RoqCodebookFormat::Rgba,
        );
        assert_eq!(
            &books.book2()[..16],
            &[119, 0, 0, 255, 135, 0, 0, 255, 151, 5, 0, 255, 167, 21, 15, 255]
        );
        assert_eq!(
            &books.book4()[..32],
            &[
                119, 0, 0, 255, 135, 0, 0, 255, 119, 0, 0, 255, 135, 0, 0, 255, 151, 5, 0, 255, 167, 21, 15, 255, 151,
                5, 0, 255, 167, 21, 15, 255
            ]
        );
        assert_eq!(
            &books.book8()[..32],
            &[
                119, 0, 0, 255, 119, 0, 0, 255, 135, 0, 0, 255, 135, 0, 0, 255, 119, 0, 0, 255, 119, 0, 0, 255, 135, 0,
                0, 255, 135, 0, 0, 255
            ]
        );
    }

    #[test]
    fn half_gray_and_565_modes_match_donor() {
        let half = decode(
            &[10, 99, 20, 99, 100, 200],
            0x0100,
            RoqCodebookMode::Half,
            RoqCodebookFormat::Rgba,
        );
        assert_eq!(&half.book2()[..8], &[113, 0, 0, 255, 123, 0, 0, 255]);
        let gray: Vec<u8> = (0..256).map(|index| 255 - index as u8).collect();
        let gray_books = decode(
            &[5, 6, 7, 8, 0, 0],
            0x0100,
            RoqCodebookMode::Normal,
            RoqCodebookFormat::Gray(&gray),
        );
        assert_eq!(&gray_books.book2()[..4], &[250, 249, 248, 247]);
        let rgb565 = decode(
            &[16, 32, 48, 64, 100, 200],
            0x0100,
            RoqCodebookMode::Normal,
            RoqCodebookFormat::Rgb565,
        );
        assert_eq!(&rgb565.book2()[..8], &[0, 112, 0, 128, 32, 144, 161, 160]);
    }

    #[test]
    fn smoothed_double_modes_match_donor() {
        let books = decode(
            &[16, 32, 48, 64, 100, 200],
            0x0100,
            RoqCodebookMode::SmoothedDouble,
            RoqCodebookFormat::Rgba,
        );
        assert_eq!(
            &books.book2()[..32],
            &[
                119, 0, 0, 255, 135, 0, 0, 255, 127, 0, 0, 255, 143, 0, 0, 255, 143, 0, 0, 255, 159, 13, 7, 255, 151,
                5, 0, 255, 167, 21, 15, 255
            ]
        );
        let gray: Vec<u8> = (0..256).map(|index| 255 - index as u8).collect();
        let books = decode(
            &[16, 32, 48, 64, 0, 0],
            0x0100,
            RoqCodebookMode::SmoothedDouble,
            RoqCodebookFormat::Gray(&gray),
        );
        assert_eq!(&books.book2()[..8], &[239, 223, 231, 215, 215, 199, 207, 191]);
    }

    #[test]
    fn zero_flags_require_both_tables() {
        // 256 cells + 512 VQ pairs of two bytes.
        let payload = vec![7u8; 256 * 6 + 1024];
        let books = decode(&payload, 0, RoqCodebookMode::Normal, RoqCodebookFormat::Rgba);
        assert_ne!(&books.book2()[..4], &[0, 0, 0, 0]);
        assert_ne!(&books.book4()[..4], &[0, 0, 0, 0]);
        // The diagnostic profile skips the 4x4/8x8 join when no VQ
        // bytes trail the 2x2 cells.
        let mut books = SourceRoqCodebooks::new();
        let cells = vec![7u8; 256 * 6];
        let mut reader = BinaryReader::new(&cells, "<test>");
        books
            .decode(
                &mut reader,
                0,
                RoqCodebookMode::Normal,
                RoqCodebookFormat::Rgba,
                RoqCodebookProfile::Diagnostic2x2,
            )
            .unwrap();
        assert_eq!(&books.book4()[..4], &[0, 0, 0, 0]);
        assert_eq!(&books.book8()[..4], &[0, 0, 0, 0]);
    }

    #[test]
    fn gray_lookup_and_truncation_are_errors() {
        let mut books = SourceRoqCodebooks::new();
        let mut reader = BinaryReader::new(&[1u8; 6], "<test>");
        assert!(books
            .decode(
                &mut reader,
                0x0100,
                RoqCodebookMode::Normal,
                RoqCodebookFormat::Gray(&[0u8; 100]),
                RoqCodebookProfile::Source,
            )
            .is_err());
        let mut reader = BinaryReader::new(&[1u8; 2], "<test>");
        assert!(books
            .decode(
                &mut reader,
                0x0100,
                RoqCodebookMode::Normal,
                RoqCodebookFormat::Rgba,
                RoqCodebookProfile::Source,
            )
            .is_err());
    }
}
