//! Quake palettes, colormaps, and gamma tables (owned pixels).
//!
//! Donor: `src/formats/images/palette.ts` (adapted from Q1
//! VID_SetPalette/Check_Gamma and Q2 GL_Upload8/GL_InitImages,
//! GPL-2.0-or-later).

use super::indexed::IndexedImage;
use super::{fail, ContentError, ImageLevel};

/// Decoded 256-color RGB palette (`Palette`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Palette {
    /// RGB colors (768 bytes).
    pub colors: Vec<u8>,
    /// Palette source name.
    pub source: String,
}

/// Indexed transparency (`PaletteTransparency`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaletteTransparency {
    /// No transparent index.
    Opaque,
    /// Transparent palette index.
    Index(u8),
    /// Quake fence transparency (index 255).
    Q1Fence,
}

impl PaletteTransparency {
    fn index(self) -> Option<u8> {
        match self {
            PaletteTransparency::Opaque => None,
            PaletteTransparency::Index(index) => Some(index),
            PaletteTransparency::Q1Fence => Some(255),
        }
    }
}

/// Fullbright palette range (`fullbright`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FullbrightRange {
    /// First fullbright index.
    pub first: u16,
    /// Last fullbright index.
    pub last: u16,
}

/// Indexed render image with palette and options (`IndexedRenderImage`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexedRenderImage {
    /// Indexed mip levels.
    pub levels: Vec<IndexedImage>,
    /// Palette.
    pub palette: Palette,
    /// Transparency rule.
    pub transparency: PaletteTransparency,
    /// Fullbright range.
    pub fullbright: Option<FullbrightRange>,
    /// Optional 256-entry translation table.
    pub translation: Option<Vec<u8>>,
}

/// Expansion layer (`PaletteLayer`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaletteLayer {
    /// Ordinary and fullbright texels.
    Combined,
    /// Non-fullbright texels only.
    Ordinary,
    /// Fullbright texels only.
    Fullbright,
}

/// Gamma table profile (`GammaProfile`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum GammaProfile {
    /// Quake GL gamma.
    Q1Gl {
        /// Gamma value.
        gamma: f64,
    },
    /// Quake II GL gamma.
    Q2Gl {
        /// Gamma value.
        gamma: f64,
        /// Texture intensity.
        intensity: f64,
        /// Apply gamma only.
        only_gamma: bool,
    },
    /// Quake III gamma.
    Q3 {
        /// Gamma value.
        gamma: f64,
        /// Texture intensity.
        intensity: f64,
        /// Overbright bits (0..=2).
        overbright_bits: u32,
        /// Apply gamma only.
        only_gamma: bool,
    },
}

/// Decode a 256-color palette (`decodePalette`).
pub fn decode_palette(bytes: &[u8], source: &str) -> Result<Palette, ContentError> {
    if bytes.len() != 768 {
        return Err(fail(source, 0, "A Quake palette requires 256 RGB entries".to_string()));
    }
    Ok(Palette {
        colors: bytes.to_vec(),
        source: source.to_string(),
    })
}

/// Decode a Q1 colormap (`decodeQ1Colormap`).
pub fn decode_q1_colormap(bytes: &[u8]) -> Result<(Vec<u8>, FullbrightRange), ContentError> {
    if bytes.len() < 16385 {
        return Err(fail(
            "<colormap>",
            0,
            "Q1 colormap requires 64 lighting rows and a fullbright count".to_string(),
        ));
    }
    let count = bytes[16384];
    Ok((
        bytes[..16384].to_vec(),
        FullbrightRange {
            first: 256 - u16::from(count),
            last: 255,
        },
    ))
}

/// Build an indexed render image (`indexedRenderImage`).
pub fn indexed_render_image(
    levels: Vec<IndexedImage>,
    palette: Palette,
    transparency: PaletteTransparency,
    fullbright: Option<FullbrightRange>,
    translation: Option<Vec<u8>>,
) -> Result<IndexedRenderImage, ContentError> {
    if palette.colors.len() != 768 {
        return Err(fail(
            "<palette>",
            0,
            "A Quake palette requires 256 RGB entries".to_string(),
        ));
    }
    if matches!(translation.as_ref(), Some(table) if table.len() != 256) {
        return Err(fail(
            "<palette>",
            0,
            "Palette translation requires 256 entries".to_string(),
        ));
    }
    for level in &levels {
        let count = level.width as u64 * level.height as u64;
        if level.width == 0 || level.height == 0 || level.indices.len() as u64 != count {
            return Err(fail(
                "<palette>",
                0,
                "Indexed image dimensions do not match its indices".to_string(),
            ));
        }
    }
    Ok(IndexedRenderImage {
        levels,
        palette,
        transparency,
        fullbright,
        translation,
    })
}

/// Build a Q1 player shirt/pants translation (`q1PlayerTranslation`).
pub fn q1_player_translation(top: u8, bottom: u8) -> Result<Vec<u8>, ContentError> {
    if top > 13 || bottom > 13 {
        return Err(fail("<palette>", 0, "Q1 player colors must be in 0..13".to_string()));
    }
    let mut translation: Vec<u8> = (0..=255u16).map(|index| index as u8).collect();
    for index in 0..16u16 {
        let top_base = u16::from(top) * 16;
        let bottom_base = u16::from(bottom) * 16;
        translation[16 + index as usize] = (if top_base < 128 {
            top_base + index
        } else {
            top_base + 15 - index
        }) as u8;
        translation[96 + index as usize] = (if bottom_base < 128 {
            bottom_base + index
        } else {
            bottom_base + 15 - index
        }) as u8;
    }
    Ok(translation)
}

/// Expand one indexed level to RGBA (`expandIndexedImage`).
pub fn expand_indexed_image(
    image: &IndexedRenderImage,
    level: usize,
    layer: PaletteLayer,
) -> Result<ImageLevel, ContentError> {
    let indexed = match image.levels.get(level) {
        Some(level) => level,
        None => return Err(fail("<palette>", 0, format!("Missing image mip {level}"))),
    };
    let mut pixels = vec![0u8; indexed.indices.len() * 4];
    for (offset, original) in indexed.indices.iter().enumerate() {
        let index = match image.translation.as_ref() {
            Some(table) => match table.get(usize::from(*original)).copied() {
                Some(mapped) => mapped,
                None => return Err(fail("<palette>", 0, "Incomplete palette translation".to_string())),
            },
            None => *original,
        };
        let transparent = image.transparency.index().is_some_and(|at| *original == at);
        let fullbright = image
            .fullbright
            .is_some_and(|range| u16::from(index) >= range.first && u16::from(index) <= range.last);
        let visible = !transparent
            && (matches!(layer, PaletteLayer::Combined)
                || (if matches!(layer, PaletteLayer::Fullbright) {
                    fullbright
                } else {
                    !fullbright
                }));
        let base = usize::from(index) * 3;
        pixels[offset * 4] = image.palette.colors[base];
        pixels[offset * 4 + 1] = image.palette.colors[base + 1];
        pixels[offset * 4 + 2] = image.palette.colors[base + 2];
        pixels[offset * 4 + 3] = u8::from(visible) * 255;
    }
    Ok(ImageLevel {
        width: indexed.width,
        height: indexed.height,
        pixels,
    })
}

/// Build a 256-entry gamma table (`buildGammaTable`).
pub fn build_gamma_table(profile: &GammaProfile) -> Result<[u8; 256], ContentError> {
    let gamma = match profile {
        GammaProfile::Q1Gl { gamma } => *gamma,
        GammaProfile::Q2Gl { gamma, .. } => *gamma,
        GammaProfile::Q3 { gamma, .. } => *gamma,
    };
    if !gamma.is_finite() || gamma <= 0.0 {
        return Err(fail("<gamma>", 0, "Gamma must be positive and finite".to_string()));
    }
    if let GammaProfile::Q3 { overbright_bits, .. } = profile {
        if *overbright_bits > 2 {
            return Err(fail("<gamma>", 0, "Q3 overbright bits must be in 0..2".to_string()));
        }
    }
    let mut table = [0u8; 256];
    for (index, slot) in table.iter_mut().enumerate() {
        let mut value = match profile {
            GammaProfile::Q1Gl { gamma } => {
                let rounded = (*gamma as f32) as f64;
                let lit = ((index as f64 + 1.0) / 256.0).powf(rounded) as f32;
                let scaled = ((lit as f64 * 255.0) as f32) as f64;
                ((scaled + 0.5) as f32) as f64
            }
            GammaProfile::Q2Gl { gamma, .. } | GammaProfile::Q3 { gamma, .. } if *gamma == 1.0 => index as f64,
            GammaProfile::Q3 { gamma, .. } => {
                let base = ((index as f64 / 255.0) as f32) as f64;
                let exponent = ((1.0 / (*gamma as f32) as f64) as f32) as f64;
                255.0 * base.powf(exponent) + 0.5
            }
            GammaProfile::Q2Gl { gamma, .. } => {
                let rounded = (*gamma as f32) as f64;
                let lit = 255.0 * ((index as f64 + 0.5) / 255.5).powf(rounded) + 0.5;
                (lit as f32) as f64
            }
        };
        if let GammaProfile::Q3 { overbright_bits, .. } = profile {
            value = value.trunc() * f64::from(1u32 << overbright_bits);
        }
        *slot = value.trunc().clamp(0.0, 255.0) as u8;
    }
    Ok(table)
}

/// Apply gamma correction to RGBA pixels (`applyImageGamma`).
pub fn apply_image_gamma(level: &ImageLevel, profile: &GammaProfile) -> Result<ImageLevel, ContentError> {
    if level.pixels.len() as u64 != level.width as u64 * level.height as u64 * 4 {
        return Err(fail("<gamma>", 0, "Gamma input must be RGBA8".to_string()));
    }
    let table = build_gamma_table(profile)?;
    let intensity = match profile {
        GammaProfile::Q1Gl { .. } => 1.0,
        GammaProfile::Q2Gl {
            intensity, only_gamma, ..
        }
        | GammaProfile::Q3 {
            intensity, only_gamma, ..
        } => {
            if *only_gamma {
                1.0
            } else if intensity.is_nan() {
                f64::NAN
            } else {
                intensity.max(1.0)
            }
        }
    };
    if !intensity.is_finite() {
        return Err(fail("<gamma>", 0, "Intensity must be finite".to_string()));
    }
    let mut pixels = level.pixels.clone();
    let factor = (intensity as f32) as f64;
    for (offset, byte) in pixels.iter_mut().enumerate() {
        if offset % 4 == 3 {
            continue;
        }
        let mapped = (((f64::from(*byte) * factor) as f32).trunc() as usize).min(255);
        *byte = table[mapped];
    }
    Ok(ImageLevel {
        width: level.width,
        height: level.height,
        pixels,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn palette() -> Palette {
        let colors: Vec<u8> = (0..256u16)
            .flat_map(|index| [index as u8, (index + 1) as u8, (index + 2) as u8])
            .collect();
        Palette {
            colors,
            source: "<test>".to_string(),
        }
    }

    fn render(transparency: PaletteTransparency, fullbright: Option<FullbrightRange>) -> IndexedRenderImage {
        indexed_render_image(
            vec![IndexedImage {
                width: 2,
                height: 1,
                indices: vec![1, 250],
            }],
            palette(),
            transparency,
            fullbright,
            None,
        )
        .unwrap()
    }

    #[test]
    fn palette_and_colormap_decode() {
        let colors = vec![7u8; 768];
        let decoded = decode_palette(&colors, "<test>").unwrap();
        assert_eq!(decoded.source, "<test>");
        assert!(decode_palette(&[0u8; 100], "<test>").is_err());

        let mut bytes = vec![3u8; 16384];
        bytes.push(16);
        let (levels, fullbright) = decode_q1_colormap(&bytes).unwrap();
        assert_eq!(levels.len(), 16384);
        assert_eq!((fullbright.first, fullbright.last), (240, 255));
        assert!(decode_q1_colormap(&[0u8; 100]).is_err());
    }

    #[test]
    fn translation_reverses_high_ramps() {
        let table = q1_player_translation(2, 13).unwrap();
        assert_eq!(table[16], 32);
        assert_eq!(table[31], 47);
        // bottom = 13 -> base 208 >= 128, reversed.
        assert_eq!(table[96], 223);
        assert_eq!(table[111], 208);
        assert!(q1_player_translation(14, 0).is_err());
    }

    #[test]
    fn expand_layers_and_transparency() {
        let fullbright = Some(FullbrightRange { first: 240, last: 255 });
        let image = render(PaletteTransparency::Index(1), fullbright);
        let combined = expand_indexed_image(&image, 0, PaletteLayer::Combined).unwrap();
        assert_eq!(combined.pixels, vec![1, 2, 3, 0, 250, 251, 252, 255]);
        let ordinary = expand_indexed_image(&image, 0, PaletteLayer::Ordinary).unwrap();
        assert_eq!(ordinary.pixels[7], 0);
        let bright = expand_indexed_image(&image, 0, PaletteLayer::Fullbright).unwrap();
        assert_eq!(bright.pixels[3], 0);
        assert_eq!(bright.pixels[7], 255);

        let fence = render(PaletteTransparency::Q1Fence, None);
        let out = expand_indexed_image(&fence, 0, PaletteLayer::Combined).unwrap();
        assert_eq!(out.pixels[3], 255);

        assert!(expand_indexed_image(&image, 3, PaletteLayer::Combined).is_err());
    }

    #[test]
    fn translation_applies_to_color_and_fullbright() {
        let mut table: Vec<u8> = (0..=255u16).map(|index| index as u8).collect();
        table[1] = 250;
        let image = indexed_render_image(
            vec![IndexedImage {
                width: 1,
                height: 1,
                indices: vec![1],
            }],
            palette(),
            PaletteTransparency::Opaque,
            Some(FullbrightRange { first: 240, last: 255 }),
            Some(table),
        )
        .unwrap();
        let out = expand_indexed_image(&image, 0, PaletteLayer::Fullbright).unwrap();
        assert_eq!(out.pixels, vec![250, 251, 252, 255]);
    }

    #[test]
    fn gamma_table_spot_values() {
        // q2 gamma 1 is the identity.
        let table = build_gamma_table(&GammaProfile::Q2Gl {
            gamma: 1.0,
            intensity: 2.0,
            only_gamma: false,
        })
        .unwrap();
        assert_eq!(table[0], 0);
        assert_eq!(table[137], 137);
        assert_eq!(table[255], 255);

        // q1-gl gamma 1: table[0] = trunc(fround(fround(1/256)*255)+0.5) = 1.
        let table = build_gamma_table(&GammaProfile::Q1Gl { gamma: 1.0 }).unwrap();
        assert_eq!(table[0], 1);
        assert_eq!(table[255], 255);

        // q3 gamma 1 with one overbright bit doubles, clamped at 255.
        let table = build_gamma_table(&GammaProfile::Q3 {
            gamma: 1.0,
            intensity: 1.0,
            overbright_bits: 1,
            only_gamma: true,
        })
        .unwrap();
        assert_eq!(table[100], 200);
        assert_eq!(table[200], 255);

        assert!(build_gamma_table(&GammaProfile::Q1Gl { gamma: 0.0 }).is_err());
        assert!(build_gamma_table(&GammaProfile::Q3 {
            gamma: 1.0,
            intensity: 1.0,
            overbright_bits: 3,
            only_gamma: true,
        })
        .is_err());
    }

    #[test]
    fn apply_gamma_maps_rgb_only() {
        let level = ImageLevel {
            width: 1,
            height: 1,
            pixels: vec![100, 100, 100, 100],
        };
        let out = apply_image_gamma(
            &level,
            &GammaProfile::Q3 {
                gamma: 1.0,
                intensity: 1.0,
                overbright_bits: 1,
                only_gamma: true,
            },
        )
        .unwrap();
        assert_eq!(out.pixels, vec![200, 200, 200, 100]);
    }
}
