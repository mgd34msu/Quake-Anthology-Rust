//! Quake image format decoders and encoders (owned pixels).
//!
//! Donor provenance: every file in `src/formats/images`
//! (`bmp.ts`, `gif.ts`, `indexed.ts`, `jpeg.ts`, `jpeg-encoder.ts`,
//! `mip.ts`, `palette.ts`, `png.ts`, `png-encoder.ts`, `q3-bmp.ts`,
//! `q3-pcx.ts`, `q3-tga.ts`, `tga.ts`, `wad.ts`, `index.ts`). Decoders
//! return owned pixel buffers; the borrowed WAD/MIP readers in
//! [`crate::wad`] stay the zero-copy path.

use qa_core::binary::BinaryError;

pub mod bmp;
pub mod gif;
pub mod indexed;
pub mod jpeg;
pub mod jpeg_encoder;
pub mod mip;
pub mod palette;
pub mod png;
pub mod q3_bmp;
pub mod q3_pcx;
pub mod q3_tga;
pub mod tga;
pub mod wad;

/// Image error type: the existing content error (`BinaryError`).
pub type ContentError = BinaryError;

/// One RGBA8 image level, row-major (`ImageLevel`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageLevel {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// Row-major RGBA bytes (`width * height * 4`).
    pub pixels: Vec<u8>,
}

/// Pixel count shared by dimension guards (`dimensions`).
///
/// Returns `None` when the product exceeds `0x7fffffff`.
pub(crate) fn pixel_count(width: u64, height: u64) -> Option<u64> {
    let count = width.checked_mul(height)?;
    if count > 0x7fff_ffff {
        None
    } else {
        Some(count)
    }
}

/// Build a validation error at an offset.
pub(crate) fn fail(source: &str, offset: usize, message: String) -> ContentError {
    ContentError::custom(source, offset, message)
}

pub use bmp::{decode_bmp, BmpImage, BmpIndexed};
pub use gif::{decode_gif, GifFrame, GifImage, GifRect};
pub use indexed::{
    decode_lit, decode_pcx, decode_q1_mip_texture, decode_qpic, decode_wal, encode_pcx, IndexedImage, MipLevels,
    PcxImage, Q1MipTexture, QlitImage, WalImage, QLIT_KIND,
};
pub use jpeg::{decode_jpeg, decode_jpeg_with_print, JpegImage, JpegSourceError};
pub use jpeg_encoder::{encode_jpeg, encode_jpeg_to, JpegEncodingProfile, RowOrder};
pub use mip::{generate_mip_chain, mip_image, resample_image, MipProfile, ResampleProfile};
pub use palette::{
    apply_image_gamma, build_gamma_table, decode_palette, decode_q1_colormap, expand_indexed_image,
    indexed_render_image, q1_player_translation, FullbrightRange, GammaProfile, IndexedRenderImage, Palette,
    PaletteLayer, PaletteTransparency,
};
pub use png::{decode_png, encode_png, PngImage, PngIndexed};
pub use q3_bmp::{decode_q3_bmp, BmpDropError};
pub use q3_pcx::{
    decode_q3_pcx, decode_q3_pcx_indexed, expand_q3_pcx, IndexedPcxImage, PcxRejection, Q3Pcx, Q3PcxIndexed,
};
pub use q3_tga::{decode_q3_tga, decode_q3_tga_with_warning};
pub use tga::{decode_tga, encode_tga, TgaImage, TgaIndexed};
pub use wad::{decode_wad, decode_wad_image, OwnedWadArchive, OwnedWadImage, OwnedWadKind, OwnedWadLump};
