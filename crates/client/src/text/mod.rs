//! Text: layout, fonts, localization, captions.
//!
//! Ported from the TypeScript donor's `src/text/*`: glyph atlases
//! (`atlas`), Q2 `kfont` parsing (`kfont`), layout and measurement
//! (`layout`), localization grammar and catalogs (`localization`,
//! `localization-resources`), captions plus SRT/WebVTT (`captions`,
//! `media-captions`), 2D drawing coordinates (`draw2d`), Q3 DAT fonts
//! and string drawing (`q3-font`, `q3-font-registry`), UI and
//! world-space text (`ui`, `world`), mount wiring (`mounted`), and the
//! TrueType engine (`truetype`).
//!
//! The TrueType engine (`truetype`) parses sfnt faces and rasterizes
//! monochrome and COLR v0 color glyphs; the atlas registry (`atlas`)
//! builds `kfont` and TrueType uploads over `qa-content` image decoders,
//! and the Q3 registry (`q3-font-registry`) generates DAT records from
//! TrueType sources when no cached record exists.

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
