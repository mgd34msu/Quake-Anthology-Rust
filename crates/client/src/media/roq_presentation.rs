//! RoQ presentation: shader and UI pixel extraction.
//!
//! Donor provenance: `src/media/roq-presentation.ts`
//! (`sourceRoqShaderPixels`, `sourceRoqUiPixels`; source
//! `CIN_DrawCinematic`/`CIN_UploadCinematic` buffer and resampling
//! rules, Copyright (C) 1999-2005 Id Software, Inc. GPL-2.0-or-later).

use super::roq::RoqDecoderScratch;
use super::roq_playback::RoqFramePointer;
use crate::ClientError;

/// One owned image level (`ImageLevel`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoqImageLevel {
    /// Width.
    pub width: usize,
    /// Height.
    pub height: usize,
    /// Row-major RGBA bytes.
    pub pixels: Vec<u8>,
}

/// Q3 wall movies read a fixed 256-square region of the frame
/// buffer, including retained neighboring bytes.
pub fn source_roq_shader_pixels(
    scratch: &RoqDecoderScratch,
    pointer: &RoqFramePointer,
) -> Result<RoqImageLevel, ClientError> {
    Ok(RoqImageLevel {
        width: 256,
        height: 256,
        pixels: scratch.view(pointer.offset, 256 * 256 * 4)?.to_vec(),
    })
}

/// Invoke after the queued rendering barrier to capture the live
/// frame-buffer bytes (`sourceRoqUiPixels`).
pub fn source_roq_ui_pixels(
    scratch: &RoqDecoderScratch,
    pointer: &RoqFramePointer,
    width: usize,
    height: usize,
    draw_width: usize,
    draw_height: usize,
    dirty: bool,
) -> Result<RoqImageLevel, ClientError> {
    if !dirty || (width == draw_width && height == draw_height) {
        return Ok(RoqImageLevel {
            width: draw_width,
            height: draw_height,
            pixels: scratch.view(pointer.offset, draw_width * draw_height * 4)?.to_vec(),
        });
    }
    let source = scratch.view(pointer.offset, 512 * 512 * 4)?;
    let mut result = vec![0u8; 256 * 256 * 4];
    let xm = width / 256;
    let ym = height / 256;
    let shift = if width == 512 { 9 } else { 8 };
    let read = |index: usize| {
        source
            .get(index)
            .copied()
            .ok_or_else(|| ClientError::BadMedia("Source cinematic resample exceeded linbuf".to_string()))
    };
    for y in 0..256 {
        for x in 0..256 {
            for channel in 0..4 {
                let value = if xm == 2 && ym == 2 {
                    let index = (y << 12) + x * 8 + channel;
                    ((u16::from(read(index)?)
                        + u16::from(read(index + 4)?)
                        + u16::from(read(index + 2048)?)
                        + u16::from(read(index + 2052)?))
                        >> 2) as u8
                } else if xm == 2 && ym == 1 {
                    let index = (y << 11) + x * 8 + channel;
                    ((u16::from(read(index)?) + u16::from(read(index + 4)?)) >> 1) as u8
                } else {
                    read((((y * ym) << shift) + x * xm) * 4 + channel)?
                };
                result[(y * 256 + x) * 4 + channel] = value;
            }
        }
    }
    Ok(RoqImageLevel {
        width: 256,
        height: 256,
        pixels: result,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pointer() -> RoqFramePointer {
        RoqFramePointer {
            offset: 0,
            byte_length: 256,
        }
    }

    #[test]
    fn shader_reads_fixed_square() {
        let scratch = RoqDecoderScratch::new();
        let level = source_roq_shader_pixels(&scratch, &pointer()).unwrap();
        assert_eq!((level.width, level.height), (256, 256));
        assert_eq!(level.pixels.len(), 256 * 256 * 4);
    }

    #[test]
    fn ui_fast_path_copies_draw_region() {
        let mut scratch = RoqDecoderScratch::new();
        scratch.frame(256, 0).unwrap().fill(7);
        let level = source_roq_ui_pixels(&scratch, &pointer(), 8, 8, 8, 8, true).unwrap();
        assert_eq!((level.width, level.height), (8, 8));
        assert!(level.pixels.iter().all(|byte| *byte == 7));
        let level = source_roq_ui_pixels(&scratch, &pointer(), 8, 8, 8, 8, false).unwrap();
        assert_eq!(level.pixels.len(), 8 * 8 * 4);
    }

    #[test]
    fn ui_resample_averages_down() {
        let mut scratch = RoqDecoderScratch::new();
        scratch.frame(512 * 512 * 4, 0).unwrap()[0] = 100;
        // 2x2 averaging over a 512x512 source: the top-left output
        // byte is the mean of four source bytes.
        let level = source_roq_ui_pixels(&scratch, &pointer(), 512, 512, 256, 256, true).unwrap();
        assert_eq!((level.width, level.height), (256, 256));
        assert_eq!(&level.pixels[..8], &[25, 0, 0, 0, 0, 0, 0, 0]);
        // Horizontal-only averaging over a 512x256 source.
        let level = source_roq_ui_pixels(&scratch, &pointer(), 512, 256, 256, 256, true).unwrap();
        assert_eq!(&level.pixels[..4], &[50, 0, 0, 0]);
        // Degenerate sizes take the direct sample path.
        let level = source_roq_ui_pixels(&scratch, &pointer(), 8, 8, 256, 256, true).unwrap();
        assert_eq!(&level.pixels[..4], &[100, 0, 0, 0]);
    }
}
