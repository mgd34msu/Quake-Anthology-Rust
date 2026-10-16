//! One decoded image path for world textures, models, HUD and menus.
mod codecs;
mod indexed;
mod raster;
mod wad;

use crate::{FormatError, read::Reader};
pub use indexed::{Colormap, MipFormat, MipTexture};
use std::{borrow::Cow, ops::RangeInclusive};
pub use wad::{Wad, WadImage, WadLump};

pub(super) const MAX_BYTES: usize = 256 * 1024 * 1024;
pub(super) fn pixel_count(width: u32, height: u32) -> Result<usize, FormatError> {
    let count = (width as usize)
        .checked_mul(height as usize)
        .ok_or(FormatError::InvalidRange)?;
    if width == 0 || height == 0 || count > MAX_BYTES / 4 {
        return Err(FormatError::InvalidRange);
    }
    Ok(count)
}
#[derive(Clone, Debug)]
pub struct Palette(pub Box<[[u8; 4]; 256]>);
impl Palette {
    pub fn from_rgb(bytes: &[u8]) -> Result<Self, FormatError> {
        if bytes.len() != 768 {
            return Err(FormatError::InvalidRecordSize);
        }
        Ok(Self(Box::new(std::array::from_fn(|i| {
            [bytes[i * 3], bytes[i * 3 + 1], bytes[i * 3 + 2], 255]
        }))))
    }
}
#[derive(Debug)]
pub struct IndexedImage<'a> {
    pub width: u32,
    pub height: u32,
    pub indices: Cow<'a, [u8]>,
    pub palette: Option<Palette>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RgbaImage {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}
#[derive(Debug)]
pub enum DecodedImage<'a> {
    Indexed(IndexedImage<'a>),
    Rgba(RgbaImage),
}
pub use qa_core::primitives::Lineage;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImageFormat {
    Lmp,
    Pcx,
    Tga,
    Bmp,
    Jpeg,
    Png,
    Gif,
}
impl ImageFormat {
    /// File suffixes and their sole decoder identity. JPEG spellings remain
    /// separate entries because file search order selects concrete paths.
    pub const SUFFIXES: &'static [(&'static str, Self)] = &[
        ("lmp", Self::Lmp),
        ("pcx", Self::Pcx),
        ("tga", Self::Tga),
        ("bmp", Self::Bmp),
        ("jpg", Self::Jpeg),
        ("jpeg", Self::Jpeg),
        ("png", Self::Png),
        ("gif", Self::Gif),
    ];

    pub fn from_path(path: &[u8]) -> Option<Self> {
        let ext = path.rsplit(|&b| b == b'.').next()?;
        Self::SUFFIXES
            .iter()
            .find(|(name, _)| ext.eq_ignore_ascii_case(name.as_bytes()))
            .map(|&(_, format)| format)
    }
}
pub fn decode(
    bytes: &[u8],
    format: ImageFormat,
    policy: Lineage,
) -> Result<DecodedImage<'_>, FormatError> {
    match format {
        ImageFormat::Lmp => indexed::qpic(bytes).map(DecodedImage::Indexed),
        ImageFormat::Pcx => indexed::pcx(bytes, policy),
        ImageFormat::Tga => raster::tga(bytes, policy).map(DecodedImage::Rgba),
        ImageFormat::Bmp => raster::bmp(bytes, policy).map(DecodedImage::Rgba),
        ImageFormat::Jpeg => codecs::jpeg(bytes).map(DecodedImage::Rgba),
        ImageFormat::Png => codecs::png(bytes).map(DecodedImage::Rgba),
        ImageFormat::Gif => codecs::gif(bytes).map(DecodedImage::Rgba),
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PaletteLayer {
    #[default]
    Combined,
    Ordinary,
    Fullbright,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AlphaFringe {
    #[default]
    KeepPalette,
    /// GL_Upload8 priority/bounds, including cross-row left/right neighbors.
    NativeNeighbors,
}

#[derive(Default)]
pub struct PaletteOptions<'a> {
    pub transparent_index: Option<u8>,
    pub fullbright: Option<RangeInclusive<u8>>,
    pub translation: Option<&'a [u8; 256]>,
    pub layer: PaletteLayer,
    pub fringe: AlphaFringe,
}
impl IndexedImage<'_> {
    pub fn expand(
        &self,
        palette: &Palette,
        options: &PaletteOptions<'_>,
    ) -> Result<RgbaImage, FormatError> {
        let count = pixel_count(self.width, self.height)?;
        if self.indices.len() != count {
            return Err(FormatError::InvalidRecordSize);
        }
        let mut pixels = Vec::with_capacity(count * 4);
        let translated = |original| {
            options
                .translation
                .map_or(original, |map| map[usize::from(original)])
        };
        let width = self.width as usize;
        for (i, &original) in self.indices.iter().enumerate() {
            let index = translated(original);
            let full = options
                .fullbright
                .as_ref()
                .is_some_and(|range| range.contains(&index));
            let visible = options.transparent_index != Some(original)
                && match options.layer {
                    PaletteLayer::Combined => true,
                    PaletteLayer::Ordinary => !full,
                    PaletteLayer::Fullbright => full,
                };
            let mut rgba = palette.0[usize::from(index)];
            if !visible {
                rgba[3] = 0;
            }
            if options.transparent_index == Some(original)
                && options.fringe == AlphaFringe::NativeNeighbors
            {
                let transparent = options.transparent_index;
                let neighbor = if i > width && transparent != Some(self.indices[i - width]) {
                    self.indices[i - width]
                } else if i < count - width && transparent != Some(self.indices[i + width]) {
                    self.indices[i + width]
                } else if i > 0 && transparent != Some(self.indices[i - 1]) {
                    self.indices[i - 1]
                } else if i < count - 1 && transparent != Some(self.indices[i + 1]) {
                    self.indices[i + 1]
                } else {
                    0
                };
                rgba[..3].copy_from_slice(&palette.0[usize::from(translated(neighbor))][..3]);
            }
            pixels.extend_from_slice(&rgba);
        }
        Ok(RgbaImage {
            width: self.width,
            height: self.height,
            pixels,
        })
    }
}
pub fn player_translation(top: u8, bottom: u8) -> Result<[u8; 256], FormatError> {
    if top > 13 || bottom > 13 {
        return Err(FormatError::InvalidRange);
    }
    let mut table = std::array::from_fn(|i| i as u8);
    for i in 0..16 {
        table[16 + i] = top * 16 + if top < 8 { i as u8 } else { 15 - i as u8 };
        table[96 + i] = bottom * 16 + if bottom < 8 { i as u8 } else { 15 - i as u8 };
    }
    Ok(table)
}
