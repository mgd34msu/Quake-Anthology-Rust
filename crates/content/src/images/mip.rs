//! Image resampling and mipmap generation (owned pixels).
//!
//! Donor: `src/formats/images/mip.ts` (adapted from Q1/Q2
//! GL_ResampleTexture, GL_MipMap and Q3 R_MipMap2, GPL-2.0-or-later).

use super::{fail, ContentError, ImageLevel};

/// Resampling profile (`resampleImage`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResampleProfile {
    /// Quake point sampling.
    Q1,
    /// Quake II bilinear 4-tap.
    Q2,
    /// Quake III bilinear 4-tap.
    Q3,
}

/// Mipmap profile (`mipImage`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MipProfile {
    /// 2x2 box filter.
    Box,
    /// Q3 4x4 weighted filter (R_MipMap2).
    Q3Weighted,
}

fn check_image(image: &ImageLevel) -> Result<(), ContentError> {
    let count = image.width as u64 * image.height as u64 * 4;
    if image.width == 0 || image.height == 0 || image.pixels.len() as u64 != count {
        return Err(fail("<image>", 0, "Expected a complete RGBA8 image".to_string()));
    }
    Ok(())
}

/// Resample an RGBA8 image (`resampleImage`).
pub fn resample_image(
    input: &ImageLevel,
    width: u32,
    height: u32,
    profile: ResampleProfile,
) -> Result<ImageLevel, ContentError> {
    check_image(input)?;
    if width == 0 || height == 0 || width as u64 * height as u64 * 4 > 0x7fff_ffff {
        return Err(fail("<image>", 0, "Invalid resampled dimensions".to_string()));
    }
    let input_width = u64::from(input.width);
    let input_height = u64::from(input.height);
    let out_width = width as usize;
    let out_height = height as usize;
    let mut pixels = vec![0u8; out_width * out_height * 4];
    let step = input_width * 65536 / u64::from(width);
    for y in 0..out_height {
        for x in 0..out_width {
            if matches!(profile, ResampleProfile::Q1) {
                let sx = ((step / 2 + x as u64 * step) >> 16).min(input_width - 1) as usize;
                let sy = (y as u64 * input_height / u64::from(height)) as usize;
                let from = (sy * input_width as usize + sx) * 4;
                let to = (y * out_width + x) * 4;
                pixels[to..to + 4].copy_from_slice(&input.pixels[from..from + 4]);
            } else {
                let quarter = step / 4;
                let x1 = ((quarter + x as u64 * step) >> 16) as usize;
                let x2 = ((3 * quarter + x as u64 * step) >> 16) as usize;
                let y1 = (((y as f64 + 0.25) * input_height as f64) / f64::from(height)).floor() as usize;
                let y2 = (((y as f64 + 0.75) * input_height as f64) / f64::from(height)).floor() as usize;
                let stride = input_width as usize;
                for c in 0..4 {
                    let sum = u16::from(input.pixels[(y1 * stride + x1) * 4 + c])
                        + u16::from(input.pixels[(y1 * stride + x2) * 4 + c])
                        + u16::from(input.pixels[(y2 * stride + x1) * 4 + c])
                        + u16::from(input.pixels[(y2 * stride + x2) * 4 + c]);
                    pixels[(y * out_width + x) * 4 + c] = (sum >> 2) as u8;
                }
            }
        }
    }
    Ok(ImageLevel { width, height, pixels })
}

/// Halve an RGBA8 image (`mipImage`).
pub fn mip_image(input: &ImageLevel, profile: MipProfile) -> Result<ImageLevel, ContentError> {
    check_image(input)?;
    let input_width = input.width as usize;
    let input_height = input.height as usize;
    let width = (input_width >> 1).max(1);
    let height = (input_height >> 1).max(1);
    if input_width == 1 && input_height == 1 {
        return Ok(ImageLevel {
            width: 1,
            height: 1,
            pixels: input.pixels.clone(),
        });
    }
    if matches!(profile, MipProfile::Q3Weighted) {
        if input_width & (input_width - 1) != 0 || input_height & (input_height - 1) != 0 {
            return Err(fail(
                "<image>",
                0,
                "Q3 weighted mipmaps require power-of-two dimensions".to_string(),
            ));
        }
        if input_width == 1 || input_height == 1 {
            return Ok(ImageLevel {
                width: width as u32,
                height: height as u32,
                pixels: input.pixels[..width * height * 4].to_vec(),
            });
        }
    }
    let mut pixels = vec![0u8; width * height * 4];
    for y in 0..height {
        for x in 0..width {
            for c in 0..4 {
                let value = if matches!(profile, MipProfile::Q3Weighted) {
                    let mut total = 0u32;
                    for dy in -1i32..=2 {
                        for dx in -1i32..=2 {
                            let weight = (if dy == -1 || dy == 2 { 1u32 } else { 2 })
                                * (if dx == -1 || dx == 2 { 1u32 } else { 2 });
                            let sy = ((y as i32 * 2 + dy) & (input_height as i32 - 1)) as usize;
                            let sx = ((x as i32 * 2 + dx) & (input_width as i32 - 1)) as usize;
                            total += weight * u32::from(input.pixels[(sy * input_width + sx) * 4 + c]);
                        }
                    }
                    total / 36
                } else {
                    let x2 = (x * 2 + 1).min(input_width - 1);
                    let y2 = (y * 2 + 1).min(input_height - 1);
                    (u32::from(input.pixels[(y * 2 * input_width + x * 2) * 4 + c])
                        + u32::from(input.pixels[(y * 2 * input_width + x2) * 4 + c])
                        + u32::from(input.pixels[(y2 * input_width + x * 2) * 4 + c])
                        + u32::from(input.pixels[(y2 * input_width + x2) * 4 + c]))
                        >> 2
                };
                pixels[(y * width + x) * 4 + c] = value as u8;
            }
        }
    }
    Ok(ImageLevel {
        width: width as u32,
        height: height as u32,
        pixels,
    })
}

/// Generate a full mip chain (`generateMipChain`).
pub fn generate_mip_chain(input: &ImageLevel, profile: MipProfile) -> Result<Vec<ImageLevel>, ContentError> {
    let mut levels = vec![input.clone()];
    let mut level = input.clone();
    while level.width > 1 || level.height > 1 {
        level = mip_image(&level, profile)?;
        levels.push(level.clone());
    }
    Ok(levels)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn level(width: u32, height: u32, pixels: Vec<u8>) -> ImageLevel {
        ImageLevel { width, height, pixels }
    }

    #[test]
    fn resample_q1_picks_center_tap() {
        // 2x2 -> 1x1: step = 131072, sx = min(1, 65536>>16) = 1, sy = 0.
        let input = level(2, 2, vec![1, 0, 0, 255, 2, 0, 0, 255, 3, 0, 0, 255, 4, 0, 0, 255]);
        let out = resample_image(&input, 1, 1, ResampleProfile::Q1).unwrap();
        assert_eq!(out.pixels, vec![2, 0, 0, 255]);

        // 2x2 -> 4x4 doubles each texel.
        let out = resample_image(&input, 4, 4, ResampleProfile::Q1).unwrap();
        assert_eq!(out.pixels[0..4], [1, 0, 0, 255]);
        assert_eq!(out.pixels[8..12], [2, 0, 0, 255]);
        assert_eq!(out.pixels[32..36], [3, 0, 0, 255]);
    }

    #[test]
    fn resample_q2_averages_four_taps() {
        // 4x1 red ramp -> 2x1: x1/x2 = (0,1) and (2,3), y1 = y2 = 0.
        let input = level(4, 1, vec![0, 0, 0, 255, 4, 0, 0, 255, 8, 0, 0, 255, 12, 0, 0, 255]);
        let out = resample_image(&input, 2, 1, ResampleProfile::Q2).unwrap();
        assert_eq!(out.pixels, vec![2, 0, 0, 255, 10, 0, 0, 255]);
        let out = resample_image(&input, 2, 1, ResampleProfile::Q3).unwrap();
        assert_eq!(out.pixels, vec![2, 0, 0, 255, 10, 0, 0, 255]);
    }

    #[test]
    fn box_mip_averages() {
        let input = level(2, 2, vec![0, 0, 0, 255, 4, 0, 0, 255, 8, 0, 0, 255, 12, 0, 0, 255]);
        let out = mip_image(&input, MipProfile::Box).unwrap();
        assert_eq!((out.width, out.height), (1, 1));
        assert_eq!(out.pixels, vec![6, 0, 0, 255]);
    }

    #[test]
    fn q3_weighted_matches_hand_computation() {
        // 2x2 -> 1x1 with wraparound: red total 1944 / 36 = 54.
        let input = level(2, 2, vec![0, 0, 0, 255, 36, 0, 0, 255, 72, 0, 0, 255, 108, 0, 0, 255]);
        let out = mip_image(&input, MipProfile::Q3Weighted).unwrap();
        assert_eq!(out.pixels, vec![54, 0, 0, 255]);
    }

    #[test]
    fn q3_weighted_rejects_non_power_of_two() {
        let input = level(3, 2, vec![0u8; 24]);
        let error = mip_image(&input, MipProfile::Q3Weighted).unwrap_err();
        assert_eq!(error.message, "Q3 weighted mipmaps require power-of-two dimensions");
    }

    #[test]
    fn q3_weighted_tail_slices() {
        let input = level(1, 4, vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16]);
        let out = mip_image(&input, MipProfile::Q3Weighted).unwrap();
        assert_eq!((out.width, out.height), (1, 2));
        assert_eq!(out.pixels, vec![1, 2, 3, 4, 5, 6, 7, 8]);
    }

    #[test]
    fn chain_halves_to_one() {
        let input = level(4, 4, vec![7u8; 64]);
        let chain = generate_mip_chain(&input, MipProfile::Box).unwrap();
        let dims: Vec<(u32, u32)> = chain.iter().map(|level| (level.width, level.height)).collect();
        assert_eq!(dims, vec![(4, 4), (2, 2), (1, 1)]);
        assert!(chain.iter().all(|level| level.pixels.iter().all(|&b| b == 7)));
    }

    #[test]
    fn rejects_bad_images_and_dims() {
        let bad = level(2, 2, vec![0u8; 8]);
        let error = resample_image(&bad, 1, 1, ResampleProfile::Q1).unwrap_err();
        assert_eq!(error.message, "Expected a complete RGBA8 image");
        let good = level(1, 1, vec![0u8; 4]);
        let error = resample_image(&good, 0, 1, ResampleProfile::Q1).unwrap_err();
        assert_eq!(error.message, "Invalid resampled dimensions");
    }
}
