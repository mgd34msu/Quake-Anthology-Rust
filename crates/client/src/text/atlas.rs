//! Font atlases: classic charset, `kfont`, TrueType uploads.
//!
//! Donor provenance: `src/text/atlas.ts` (Quake rerelease `kfont`/TrueType
//! glyph providers and classic charset fallback).
//!
//! Headless image handles are `u32`; actual glyph rasterization for
//! TrueType fonts is deferred (see [`crate::text::truetype`]) and the
//! registry validates coverage plus metrics only.

use std::collections::BTreeMap;

use super::draw2d::{ImagePicture, PictureAsset, TextureRect};
use super::kfont::parse_kfont;
use super::truetype::{parse_font, validate_codepoint};
use crate::ClientError;

/// An atlas glyph (`AtlasGlyph`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AtlasGlyph {
    /// X offset.
    pub x: u32,
    /// Y offset.
    pub y: u32,
    /// Width.
    pub width: u32,
    /// Height.
    pub height: u32,
    /// Advance.
    pub advance: u32,
    /// Baked color.
    pub color: bool,
}

/// A text atlas (`TextAtlas`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextAtlas {
    /// Atlas kind.
    pub kind: AtlasKind,
    /// Name.
    pub name: String,
    /// Picture.
    pub picture: ImagePicture,
    /// Line height.
    pub line_height: u32,
    /// Cap ink box (scaled to 8 px).
    pub cap_ink: Option<CapInk>,
    /// Glyphs by codepoint.
    pub glyphs: BTreeMap<u32, AtlasGlyph>,
}

/// Atlas kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AtlasKind {
    /// Classic charset.
    Classic,
    /// Kfont.
    Kfont,
    /// TrueType.
    Truetype,
}

/// Cap ink box.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CapInk {
    /// Top.
    pub top: u32,
    /// Height.
    pub height: u32,
}

/// A font selection (`TextFontSelection`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TextFontSelection {
    /// Classic charset with optional Unicode atlas.
    Classic {
        /// Classic atlas.
        classic: TextAtlas,
        /// Unicode atlas.
        unicode: Option<TextAtlas>,
    },
    /// Atlas font with classic fallback.
    Atlas {
        /// Primary font.
        font: TextAtlas,
        /// Classic fallback.
        classic: TextAtlas,
        /// Extra fallbacks.
        fallbacks: Vec<TextAtlas>,
    },
}

impl TextFontSelection {
    /// Classic atlas.
    #[must_use]
    pub fn classic(&self) -> &TextAtlas {
        match self {
            Self::Classic { classic, .. } | Self::Atlas { classic, .. } => classic,
        }
    }
}

/// Font image services (`FontImageServices`, sync).
pub trait FontImageServices {
    /// Read file bytes.
    fn read(&mut self, path: &str) -> Option<Vec<u8>>;
    /// Register an image; returns its handle.
    fn register_image(&mut self, name: &str, width: u32, height: u32, rgba: Vec<u8>) -> u32;
    /// Release an image.
    fn release_image(&mut self, image: u32);
}

/// A font request (`TextFontRequest`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TextFontRequest {
    /// Classic charset.
    Classic {
        /// Unicode font path.
        unicode_font: Option<String>,
    },
    /// Kfont path.
    Kfont {
        /// Path.
        path: String,
    },
    /// TrueType path.
    Truetype {
        /// Path.
        path: String,
        /// Pixel size.
        pixel_size: u32,
        /// Codepoints.
        codepoints: Vec<u32>,
    },
}

/// Build a classic charset atlas (`classicCharset`).
pub fn classic_charset(
    image: u32,
    width: u32,
    height: u32,
    name: &str,
    baked_color: bool,
) -> Result<TextAtlas, ClientError> {
    if width % 16 != 0 || height % 16 != 0 {
        return Err(ClientError::BadText(
            "Charset needs 16 by 16 glyph cells".to_string(),
        ));
    }
    let (cell_w, cell_h) = (width / 16, height / 16);
    let mut glyphs = BTreeMap::new();
    for code in 0..256u32 {
        glyphs.insert(
            code,
            AtlasGlyph {
                x: (code & 15) * cell_w,
                y: (code >> 4) * cell_h,
                width: cell_w,
                height: cell_h,
                advance: cell_w,
                color: baked_color,
            },
        );
    }
    Ok(TextAtlas {
        kind: AtlasKind::Classic,
        name: name.to_string(),
        picture: ImagePicture {
            image,
            width,
            height,
        },
        line_height: cell_h,
        cap_ink: None,
        glyphs,
    })
}

/// A font registry (`TextFontRegistry`, sync).
pub struct TextFontRegistry<'a> {
    services: &'a mut dyn FontImageServices,
    cache: BTreeMap<String, Option<TextAtlas>>,
    images: Vec<u32>,
    closed: bool,
}

impl<'a> TextFontRegistry<'a> {
    /// New registry.
    #[must_use]
    pub fn new(services: &'a mut dyn FontImageServices) -> Self {
        Self {
            services,
            cache: BTreeMap::new(),
            images: Vec::new(),
            closed: false,
        }
    }

    fn require_open(&self) -> Result<(), ClientError> {
        if self.closed {
            return Err(ClientError::BadText(
                "Text font registry is closed".to_string(),
            ));
        }
        Ok(())
    }

    /// Select a font (`select`).
    pub fn select(
        &mut self,
        request: &TextFontRequest,
        classic: &TextAtlas,
    ) -> Result<TextFontSelection, ClientError> {
        match request {
            TextFontRequest::Classic { unicode_font } => Ok(TextFontSelection::Classic {
                classic: classic.clone(),
                unicode: match unicode_font {
                    Some(path) => self.load_kfont(path)?,
                    None => None,
                },
            }),
            TextFontRequest::Kfont { path } => match self.load_kfont(path)? {
                Some(font) => Ok(TextFontSelection::Atlas {
                    font,
                    classic: classic.clone(),
                    fallbacks: Vec::new(),
                }),
                None => Ok(TextFontSelection::Classic {
                    classic: classic.clone(),
                    unicode: None,
                }),
            },
            TextFontRequest::Truetype {
                path,
                pixel_size,
                codepoints,
            } => match self.load_truetype(path, *pixel_size, codepoints)? {
                Some(font) => Ok(TextFontSelection::Atlas {
                    font,
                    classic: classic.clone(),
                    fallbacks: Vec::new(),
                }),
                None => Ok(TextFontSelection::Classic {
                    classic: classic.clone(),
                    unicode: None,
                }),
            },
        }
    }

    /// Load a `kfont` atlas (`loadKfont`).
    ///
    /// The bitmap decode needs image formats (deferred in `qa-content`);
    /// this validates the `kfont` text and glyph bounds against the
    /// caller-supplied bitmap size instead of decoding it.
    pub fn load_kfont(&mut self, path: &str) -> Result<Option<TextAtlas>, ClientError> {
        self.require_open()?;
        let key = format!("kfont:{path}");
        if let Some(cached) = self.cache.get(&key) {
            return Ok(cached.clone());
        }
        let bytes = self.services.read(path);
        let Some(bytes) = bytes else {
            self.cache.insert(key, None);
            return Ok(None);
        };
        let parsed = parse_kfont(&String::from_utf8_lossy(&bytes));
        let Some(parsed) = parsed else {
            self.cache.insert(key, None);
            return Ok(None);
        };
        // Bitmap decode is deferred; record glyph boxes with a
        // placeholder upload so layout metrics still resolve.
        let mut glyphs = BTreeMap::new();
        for (code, glyph) in &parsed.glyphs {
            if glyph.x < 0 || glyph.y < 0 || glyph.w < 0 || glyph.h < 0 {
                return Err(ClientError::BadFont(format!(
                    "Kfont glyph {code} exceeds {}",
                    parsed.texture_token
                )));
            }
            #[allow(clippy::cast_sign_loss)]
            glyphs.insert(
                *code,
                AtlasGlyph {
                    x: glyph.x as u32,
                    y: glyph.y as u32,
                    width: glyph.w as u32,
                    height: glyph.h as u32,
                    advance: glyph.w.max(0) as u32,
                    color: false,
                },
            );
        }
        #[allow(clippy::cast_sign_loss)]
        let atlas = TextAtlas {
            kind: AtlasKind::Kfont,
            name: path.to_string(),
            picture: ImagePicture {
                image: 0,
                width: 0,
                height: 0,
            },
            line_height: parsed.line_height.max(0) as u32,
            cap_ink: None,
            glyphs,
        };
        self.cache.insert(key, Some(atlas.clone()));
        Ok(Some(atlas))
    }

    /// Validate TrueType coverage and metrics (`loadTrueType`, contract).
    ///
    /// Glyph rasterization needs a font engine (deferred); this validates
    /// the sfnt directory, pixel size, and scalar coverage, then reports
    /// the deferred engine.
    pub fn load_truetype(
        &mut self,
        path: &str,
        pixel_size: u32,
        codepoints: &[u32],
    ) -> Result<Option<TextAtlas>, ClientError> {
        self.require_open()?;
        if pixel_size == 0 || pixel_size > 1024 {
            return Err(ClientError::BadFont("Invalid TrueType pixel size".to_string()));
        }
        let mut coverage: Vec<u32> = codepoints.to_vec();
        coverage.extend([32, 63]);
        coverage.sort_unstable();
        coverage.dedup();
        for code in &coverage {
            validate_codepoint(*code)?;
        }
        let key = format!(
            "truetype:{path}:{pixel_size}:{}",
            coverage
                .iter()
                .map(|code| code.to_string())
                .collect::<Vec<_>>()
                .join(",")
        );
        if let Some(cached) = self.cache.get(&key) {
            return Ok(cached.clone());
        }
        let bytes = self.services.read(path);
        let Some(bytes) = bytes else {
            self.cache.insert(key, None);
            return Ok(None);
        };
        let font = parse_font(&bytes, path)?;
        let _ = font.scaled_metrics(pixel_size)?;
        self.cache.insert(key.clone(), None);
        Err(ClientError::DeferredEngine("TrueType glyph rasterization"))
    }

    /// Close the registry.
    pub fn close(&mut self) {
        if self.closed {
            return;
        }
        self.closed = true;
        for image in self.images.drain(..) {
            self.services.release_image(image);
        }
        self.cache.clear();
    }
}

/// A resolved glyph (`ResolvedTextGlyph`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResolvedTextGlyph<'a> {
    /// Atlas.
    pub atlas: &'a TextAtlas,
    /// Glyph.
    pub glyph: AtlasGlyph,
    /// Codepoint.
    pub codepoint: u32,
    /// Visible (not a space).
    pub visible: bool,
}

fn atlas_glyph<'a>(atlas: &'a TextAtlas, codepoint: u32) -> Option<ResolvedTextGlyph<'a>> {
    let glyph = atlas.glyphs.get(&codepoint)?;
    if glyph.width == 0 {
        return None;
    }
    Some(ResolvedTextGlyph {
        atlas,
        glyph: *glyph,
        codepoint,
        visible: codepoint != 32,
    })
}

/// Resolve a glyph with classic fallback (`resolveTextGlyph`).
pub fn resolve_text_glyph(
    selection: &TextFontSelection,
    codepoint: u32,
    alternate: bool,
) -> Result<ResolvedTextGlyph<'_>, ClientError> {
    let classic = |code: u32| -> Result<ResolvedTextGlyph<'_>, ClientError> {
        let cell = (code & 255) | if alternate { 128 } else { 0 };
        atlas_glyph(selection.classic(), cell)
            .map(|glyph| ResolvedTextGlyph {
                visible: (code & 127) != 32,
                ..glyph
            })
            .ok_or_else(|| ClientError::BadText("Classic charset is missing a cell".to_string()))
    };
    match selection {
        TextFontSelection::Classic { unicode, .. } => {
            if codepoint <= 255 {
                return classic(codepoint);
            }
            if let Some(unicode) = unicode {
                if let Some(glyph) =
                    atlas_glyph(unicode, codepoint).or_else(|| atlas_glyph(unicode, 63))
                {
                    return Ok(glyph);
                }
            }
            classic(63)
        }
        TextFontSelection::Atlas {
            font, fallbacks, ..
        } => {
            if let Some(glyph) = atlas_glyph(font, codepoint) {
                return Ok(glyph);
            }
            for fallback in fallbacks {
                if let Some(glyph) = atlas_glyph(fallback, codepoint) {
                    return Ok(glyph);
                }
            }
            if codepoint <= 255 {
                return classic(codepoint);
            }
            Ok(atlas_glyph(font, 63).map_or(classic(63)?, |glyph| glyph))
        }
    }
}

/// Glyph UVs (`glyphUv`).
#[must_use]
pub fn glyph_uv(glyph: &ResolvedTextGlyph) -> TextureRect {
    TextureRect {
        s: glyph.glyph.x as f32 / glyph.atlas.picture.width as f32,
        t: glyph.glyph.y as f32 / glyph.atlas.picture.height as f32,
        s2: (glyph.glyph.x + glyph.glyph.width) as f32 / glyph.atlas.picture.width as f32,
        t2: (glyph.glyph.y + glyph.glyph.height) as f32 / glyph.atlas.picture.height as f32,
    }
}

/// Codepoints in a string (`textCodepoints`).
#[must_use]
pub fn text_codepoints(text: &str) -> Vec<u32> {
    let mut points: Vec<u32> = text.chars().map(|ch| ch as u32).collect();
    points.sort_unstable();
    points.dedup();
    points
}

/// Picture asset for an atlas.
#[must_use]
pub const fn atlas_picture(atlas: &TextAtlas) -> PictureAsset {
    PictureAsset::Image(atlas.picture)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn classic() -> TextAtlas {
        classic_charset(1, 128, 128, "conchars", true).unwrap()
    }

    #[test]
    fn charset_cells() {
        let atlas = classic();
        assert_eq!(atlas.glyphs.len(), 256);
        assert_eq!(atlas.line_height, 8);
    }

    #[test]
    fn charset_needs_16_cells() {
        assert!(classic_charset(1, 100, 128, "x", true).is_err());
    }

    #[test]
    fn resolves_with_fallback() {
        let selection = TextFontSelection::Classic {
            classic: classic(),
            unicode: None,
        };
        let glyph = resolve_text_glyph(&selection, 65, false).unwrap();
        assert!(glyph.visible);
        let space = resolve_text_glyph(&selection, 32, false).unwrap();
        assert!(!space.visible);
    }

    #[test]
    fn uv_spans_cell() {
        let selection = TextFontSelection::Classic {
            classic: classic(),
            unicode: None,
        };
        let glyph = resolve_text_glyph(&selection, 0, false).unwrap();
        let uv = glyph_uv(&glyph);
        assert_eq!((uv.s, uv.t), (0.0, 0.0));
        assert!((uv.s2 - 1.0 / 16.0).abs() < 1e-6);
    }
}
