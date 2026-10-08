//! One decoded image path for world textures, models, HUD and menus.
mod codecs;
mod indexed;
mod mip;
mod raster;
mod wad;

use crate::{FormatError, read::Reader};
pub use indexed::{Colormap, MipFormat, MipTexture};
pub use mip::MipFilter;
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
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RasterPolicy {
    Standard,
    /// Original tight PCX rows; other formats use standard decoding.
    Quake2,
    Quake3,
}
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
    pub fn from_path(path: &[u8]) -> Option<Self> {
        let ext = path.rsplit(|&b| b == b'.').next()?;
        [
            (b"lmp".as_slice(), Self::Lmp),
            (b"pcx", Self::Pcx),
            (b"tga", Self::Tga),
            (b"bmp", Self::Bmp),
            (b"jpg", Self::Jpeg),
            (b"jpeg", Self::Jpeg),
            (b"png", Self::Png),
            (b"gif", Self::Gif),
        ]
        .into_iter()
        .find(|(name, _)| ext.eq_ignore_ascii_case(name))
        .map(|(_, format)| format)
    }
}
pub fn decode(
    bytes: &[u8],
    format: ImageFormat,
    policy: RasterPolicy,
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
#[derive(Default)]
pub struct PaletteOptions<'a> {
    pub transparent_index: Option<u8>,
    pub fullbright: Option<RangeInclusive<u8>>,
    pub translation: Option<&'a [u8; 256]>,
    pub layer: PaletteLayer,
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
        for &original in self.indices.iter() {
            let index = options
                .translation
                .map_or(original, |map| map[usize::from(original)]);
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
