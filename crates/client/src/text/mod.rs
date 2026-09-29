//! Text: layout, fonts, localization, captions.
//!
//! Ported from the TypeScript donor's `src/text/*`: glyph atlases
//! (`atlas`), Q2 `kfont` parsing (`kfont`), layout and measurement
//! (`layout`), localization grammar and catalogs (`localization`,
//! `localization-resources`), captions plus SRT/WebVTT (`captions`,
//! `media-captions`), 2D drawing coordinates (`draw2d`), Q3 DAT fonts
//! and string drawing (`q3-font`, `q3-font-registry`), UI and
//! world-space text (`ui`, `world`), mount wiring (`mounted`), and the
//! TrueType data contract (`truetype`).
//!
//! Deferred engines (need external engines): TrueType glyph
//! rasterization and atlas building, FreeType Q3 font generation, and
//! `kfont`/TrueType bitmap decoding (needs image formats). The atlas,
//! registry, and layout contracts validate inputs and metrics without
//! them.

pub mod atlas;
pub mod captions;
pub mod draw2d;
pub mod kfont;
pub mod layout;
pub mod localization;
pub mod media_captions;
pub mod mounted;
pub mod q3_font;
pub mod q3_font_registry;
pub mod resources;
pub mod truetype;
pub mod ui_world;
