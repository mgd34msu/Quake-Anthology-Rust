//! Quake 2 intensity-scaled mipmap upload (`GL_Upload32`).
//!
//! Donor provenance: `src/render/scene/q2-image.ts` (`q2MipmappedImage`)
//! with `expandIndexedImage`/`applyImageGamma` from
//! `src/formats/images/palette.ts` and `generateMipChain` from
//! `src/formats/images/mip.ts`. Indexed pixels expand through the palette
//! (translation applied before lookup, transparency from the original
//! index), RGB channels scale by the q2-gl intensity of 2 with gamma 1,
//! and a box-filter chain runs down to 1x1.

use crate::render::types::{ImageLevel, PaletteTransparency, RenderImage};

fn transparent_index(transparency: &PaletteTransparency) -> Option<u8> {
    match transparency {
        PaletteTransparency::Opaque => None,
        PaletteTransparency::Index(index) => Some(*index),
        PaletteTransparency::Q1Fence => Some(255),
    }
}

fn expand_indexed(
    indexed: &ImageLevel,
    palette: &[u8],
    transparency: &PaletteTransparency,
    translation: Option<&[u8]>,
) -> ImageLevel {
    let opaque_at = transparent_index(transparency);
    let mut pixels = vec![0u8; indexed.pixels.len() * 4];
    for (offset, original) in indexed.pixels.iter().enumerate() {
        let index = translation
            .and_then(|table| table.get(usize::from(*original)).copied())
            .unwrap_or(*original);
        let base = usize::from(index) * 3;
        pixels[offset * 4] = palette.get(base).copied().unwrap_or(0);
        pixels[offset * 4 + 1] = palette.get(base + 1).copied().unwrap_or(0);
        pixels[offset * 4 + 2] = palette.get(base + 2).copied().unwrap_or(0);
        pixels[offset * 4 + 3] = if opaque_at == Some(*original) { 0 } else { 255 };
    }
    ImageLevel {
        width: indexed.width,
        height: indexed.height,
        pixels,
    }
}

fn apply_intensity(level: &ImageLevel) -> ImageLevel {
    let mut pixels = level.pixels.clone();
    for chunk in pixels.chunks_mut(4) {
        for channel in chunk.iter_mut().take(3) {
            *channel = (*channel as u16 * 2).min(255) as u8;
        }
    }
    ImageLevel {
        width: level.width,
        height: level.height,
        pixels,
    }
}

fn mip_box(input: &ImageLevel) -> ImageLevel {
    let width = (input.width >> 1).max(1);
    let height = (input.height >> 1).max(1);
    if input.width == 1 && input.height == 1 {
        return ImageLevel {
            width,
            height,
            pixels: input.pixels.clone(),
        };
    }
    let mut pixels = vec![0u8; (width * height * 4) as usize];
    for y in 0..height {
        for x in 0..width {
            let x2 = (x * 2 + 1).min(input.width - 1);
            let y2 = (y * 2 + 1).min(input.height - 1);
            for c in 0..4 {
                let sample = |sx: u32, sy: u32| {
                    input
                        .pixels
                        .get(((sy * input.width + sx) * 4 + c) as usize)
                        .copied()
                        .unwrap_or(0) as u16
                };
                pixels[((y * width + x) * 4 + c) as usize] =
                    ((sample(x * 2, y * 2) + sample(x2, y * 2) + sample(x * 2, y2) + sample(x2, y2)) >> 2) as u8;
            }
        }
    }
    ImageLevel { width, height, pixels }
}

fn mip_chain(base: ImageLevel) -> Vec<ImageLevel> {
    let mut levels = vec![base];
    while levels.last().is_some_and(|level| level.width > 1 || level.height > 1) {
        let next = mip_box(levels.last().expect("chain always holds a base level"));
        levels.push(next);
    }
    levels
}

/// `GL_Upload32`: intensity-scale the base level, then build mipmaps.
/// Depth images and UI/sky uploads pass through unscaled.
#[must_use]
pub fn q2_mipmapped_image(content: &RenderImage) -> RenderImage {
    if matches!(content, RenderImage::Depth32f { .. }) {
        return content.clone();
    }
    let base = match content {
        RenderImage::Indexed8 {
            levels,
            palette,
            transparency,
            translation,
            ..
        } => levels
            .first()
            .map(|indexed| expand_indexed(indexed, &palette.colors, transparency, translation.as_deref())),
        RenderImage::Rgba8 { levels, .. } => levels.first().cloned(),
        RenderImage::Depth32f { .. } => None,
    };
    let Some(base) = base else {
        return RenderImage::Rgba8 {
            levels: Vec::new(),
            border_color: qa_core::math::Vec4 {
                x: 0.0,
                y: 0.0,
                z: 0.0,
                w: 0.0,
            },
        };
    };
    RenderImage::Rgba8 {
        levels: mip_chain(apply_intensity(&base)),
        border_color: qa_core::math::Vec4 {
            x: 0.0,
            y: 0.0,
            z: 0.0,
            w: 0.0,
        },
    }
}

#[cfg(test)]
mod tests {
    use crate::render::types::{DepthImageLevel, Palette};

    use super::*;

    fn palette_with(red_at_1: [u8; 3]) -> Palette {
        let mut colors = vec![0u8; 768];
        colors[3..6].copy_from_slice(&red_at_1);
        colors[6..9].copy_from_slice(&[10, 20, 30]);
        Palette {
            colors,
            source: "test".to_string(),
        }
    }

    fn indexed(
        pixels: Vec<u8>,
        width: u32,
        height: u32,
        transparency: PaletteTransparency,
        translation: Option<Vec<u8>>,
    ) -> RenderImage {
        RenderImage::Indexed8 {
            levels: vec![ImageLevel { width, height, pixels }],
            palette: palette_with([100, 150, 200]),
            transparency,
            fullbright: None,
            translation,
        }
    }

    #[test]
    fn depth_passthrough_clones() {
        let content = RenderImage::Depth32f {
            levels: vec![DepthImageLevel {
                width: 2,
                height: 2,
                pixels: vec![0.5; 4],
            }],
        };
        assert_eq!(q2_mipmapped_image(&content), content);
    }

    #[test]
    fn indexed_expansion_maps_colors_and_alpha() {
        let content = indexed(vec![0, 1, 2, 1], 2, 2, PaletteTransparency::Opaque, None);
        let RenderImage::Rgba8 { levels, border_color } = q2_mipmapped_image(&content) else {
            panic!("expected rgba8 output");
        };
        assert_eq!(
            border_color,
            qa_core::math::Vec4 {
                x: 0.0,
                y: 0.0,
                z: 0.0,
                w: 0.0
            }
        );
        // Intensity doubles channels: palette 1 -> (200, 255, 255), palette 2 -> (20, 40, 60).
        assert_eq!(&levels[0].pixels[0..4], &[0, 0, 0, 255]);
        assert_eq!(&levels[0].pixels[4..8], &[200, 255, 255, 255]);
        assert_eq!(&levels[0].pixels[8..12], &[20, 40, 60, 255]);
    }

    #[test]
    fn transparent_index_clears_alpha() {
        let content = indexed(vec![0, 1, 2, 1], 2, 2, PaletteTransparency::Index(1), None);
        let RenderImage::Rgba8 { levels, .. } = q2_mipmapped_image(&content) else {
            panic!("expected rgba8 output");
        };
        assert_eq!(levels[0].pixels[3], 255);
        assert_eq!(levels[0].pixels[7], 0);
        assert_eq!(levels[0].pixels[11], 255);

        let fence = indexed(vec![0, 255], 2, 1, PaletteTransparency::Q1Fence, None);
        let RenderImage::Rgba8 { levels, .. } = q2_mipmapped_image(&fence) else {
            panic!("expected rgba8 output");
        };
        assert_eq!(levels[0].pixels[3], 255);
        assert_eq!(levels[0].pixels[7], 0);
    }

    #[test]
    fn translation_applies_before_lookup() {
        let mut table: Vec<u8> = (0..=255).collect();
        table[0] = 2;
        let content = indexed(vec![0], 1, 1, PaletteTransparency::Opaque, Some(table));
        let RenderImage::Rgba8 { levels, .. } = q2_mipmapped_image(&content) else {
            panic!("expected rgba8 output");
        };
        assert_eq!(&levels[0].pixels[0..4], &[20, 40, 60, 255]);
    }

    #[test]
    fn intensity_clamps_at_255() {
        let content = RenderImage::Rgba8 {
            levels: vec![ImageLevel {
                width: 1,
                height: 1,
                pixels: vec![200, 128, 127, 90],
            }],
            border_color: qa_core::math::Vec4 {
                x: 1.0,
                y: 1.0,
                z: 1.0,
                w: 1.0,
            },
        };
        let RenderImage::Rgba8 { levels, border_color } = q2_mipmapped_image(&content) else {
            panic!("expected rgba8 output");
        };
        assert_eq!(levels[0].pixels, vec![255, 255, 254, 90]);
        assert_eq!(
            border_color,
            qa_core::math::Vec4 {
                x: 0.0,
                y: 0.0,
                z: 0.0,
                w: 0.0
            }
        );
    }

    #[test]
    fn chain_lengths_cover_square_and_nonsquare() {
        let square = RenderImage::Rgba8 {
            levels: vec![ImageLevel {
                width: 4,
                height: 4,
                pixels: vec![64u8; 4 * 4 * 4],
            }],
            border_color: qa_core::math::Vec4 {
                x: 0.0,
                y: 0.0,
                z: 0.0,
                w: 0.0,
            },
        };
        let RenderImage::Rgba8 { levels, .. } = q2_mipmapped_image(&square) else {
            panic!("expected rgba8 output");
        };
        assert_eq!(levels.len(), 3);
        assert_eq!((levels[0].width, levels[0].height), (4, 4));
        assert_eq!((levels[1].width, levels[1].height), (2, 2));
        assert_eq!((levels[2].width, levels[2].height), (1, 1));
        assert_eq!(&levels[1].pixels[0..4], &[128, 128, 128, 64]);
        assert_eq!(&levels[2].pixels[0..4], &[128, 128, 128, 64]);

        let wide = RenderImage::Rgba8 {
            levels: vec![ImageLevel {
                width: 4,
                height: 2,
                pixels: vec![32u8; 4 * 2 * 4],
            }],
            border_color: qa_core::math::Vec4 {
                x: 0.0,
                y: 0.0,
                z: 0.0,
                w: 0.0,
            },
        };
        let RenderImage::Rgba8 { levels, .. } = q2_mipmapped_image(&wide) else {
            panic!("expected rgba8 output");
        };
        assert_eq!(levels.len(), 3);
        assert_eq!((levels[0].width, levels[0].height), (4, 2));
        assert_eq!((levels[1].width, levels[1].height), (2, 1));
        assert_eq!((levels[2].width, levels[2].height), (1, 1));
    }
}
