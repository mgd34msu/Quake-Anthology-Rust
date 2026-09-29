//! Font atlases: classic charset, `kfont`, TrueType uploads.
//!
//! Donor provenance: `src/text/atlas.ts` (Quake rerelease `kfont`/TrueType
//! glyph providers and classic charset fallback). `kfont` bitmaps decode
//! through `qa-content` PNG/TGA decoders and TrueType coverage rasterizes
//! through [`crate::text::truetype`]; headless image handles are `u32`.

use std::collections::BTreeMap;

use qa_content::images::png::decode_png;
use qa_content::images::tga::decode_tga;

use super::draw2d::{ImagePicture, PictureAsset, TextureRect};
use super::kfont::parse_kfont;
use super::truetype::{build_font_atlas, parse_font, validate_codepoint};
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
#[derive(Debug, Clone, PartialEq)]
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

/// Cap ink box (`capInk`, scaled to 8 px).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CapInk {
    /// Top.
    pub top: f32,
    /// Height.
    pub height: f32,
}

/// A font selection (`TextFontSelection`).
#[derive(Debug, Clone, PartialEq)]
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
    if !width.is_multiple_of(16) || !height.is_multiple_of(16) {
        return Err(ClientError::BadText("Charset needs 16 by 16 glyph cells".to_string()));
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
        picture: ImagePicture { image, width, height },
        line_height: cell_h,
        cap_ink: None,
        glyphs,
    })
}

/// Decode a `kfont` bitmap (`loadKfont` image branch).
fn decode_kfont_bitmap(bytes: &[u8], token: &str) -> Result<(u32, u32, Vec<u8>), ClientError> {
    if token.to_lowercase().ends_with(".png") {
        let image = decode_png(bytes, token).map_err(|error| ClientError::BadFont(error.to_string()))?;
        Ok((image.width, image.height, image.pixels))
    } else {
        let image = decode_tga(bytes, token).map_err(|error| ClientError::BadFont(error.to_string()))?;
        Ok((u32::from(image.width), u32::from(image.height), image.pixels))
    }
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
            return Err(ClientError::BadText("Text font registry is closed".to_string()));
        }
        Ok(())
    }

    /// Select a font (`select`).
    pub fn select(&mut self, request: &TextFontRequest, classic: &TextAtlas) -> Result<TextFontSelection, ClientError> {
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
    pub fn load_kfont(&mut self, path: &str) -> Result<Option<TextAtlas>, ClientError> {
        self.require_open()?;
        let key = format!("kfont:{path}");
        if let Some(cached) = self.cache.get(&key) {
            return Ok(cached.clone());
        }
        let atlas = self.load_kfont_uncached(path)?;
        self.cache.insert(key, atlas.clone());
        Ok(atlas)
    }

    fn load_kfont_uncached(&mut self, path: &str) -> Result<Option<TextAtlas>, ClientError> {
        let bytes = self.services.read(path);
        let Some(bytes) = bytes else {
            return Ok(None);
        };
        let parsed = parse_kfont(&String::from_utf8_lossy(&bytes));
        let Some(parsed) = parsed else {
            return Ok(None);
        };
        let bitmap = self.services.read(&parsed.texture_token);
        let Some(bitmap) = bitmap else {
            return Ok(None);
        };
        let (width, height, pixels) = decode_kfont_bitmap(&bitmap, &parsed.texture_token)?;
        let mut glyphs = BTreeMap::new();
        for (code, glyph) in &parsed.glyphs {
            if glyph.x < 0
                || glyph.y < 0
                || glyph.w < 0
                || glyph.h < 0
                || i64::from(glyph.x) + i64::from(glyph.w) > i64::from(width)
                || i64::from(glyph.y) + i64::from(glyph.h) > i64::from(height)
            {
                return Err(ClientError::BadFont(format!(
                    "Kfont glyph {code} exceeds {}",
                    parsed.texture_token
                )));
            }
            glyphs.insert(
                *code,
                AtlasGlyph {
                    x: glyph.x as u32,
                    y: glyph.y as u32,
                    width: glyph.w as u32,
                    height: glyph.h as u32,
                    advance: glyph.w as u32,
                    color: false,
                },
            );
        }
        if parsed.line_height < 0 {
            return Err(ClientError::BadFont(format!(
                "Kfont glyph line height exceeds {}",
                parsed.texture_token
            )));
        }
        self.register(
            AtlasKind::Kfont,
            path,
            width,
            height,
            pixels,
            parsed.line_height as u32,
            glyphs,
        )
        .map(Some)
    }

    /// Load a TrueType atlas (`loadTrueType`).
    pub fn load_truetype(
        &mut self,
        path: &str,
        pixel_size: u32,
        codepoints: &[u32],
    ) -> Result<Option<TextAtlas>, ClientError> {
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
        self.require_open()?;
        if let Some(cached) = self.cache.get(&key) {
            return Ok(cached.clone());
        }
        let atlas = self.load_truetype_uncached(path, pixel_size, &coverage, &key)?;
        self.cache.insert(key, atlas.clone());
        Ok(atlas)
    }

    fn load_truetype_uncached(
        &mut self,
        path: &str,
        pixel_size: u32,
        coverage: &[u32],
        key: &str,
    ) -> Result<Option<TextAtlas>, ClientError> {
        let bytes = self.services.read(path);
        let Some(bytes) = bytes else {
            return Ok(None);
        };
        let font = parse_font(&bytes, path)?;
        let built = build_font_atlas(&font, coverage, pixel_size, 512)?;
        let mut glyphs = BTreeMap::new();
        for (code, cell) in &built.glyphs {
            glyphs.insert(
                *code,
                AtlasGlyph {
                    x: cell.x,
                    y: cell.y,
                    width: cell.width,
                    height: cell.height,
                    advance: cell.width,
                    color: cell.color,
                },
            );
        }
        self.register(
            AtlasKind::Truetype,
            key,
            built.width,
            built.height,
            built.pixels,
            built.line_height,
            glyphs,
        )
        .map(Some)
    }

    /// Load paged TrueType atlases (`loadTrueTypePages`).
    pub fn load_truetype_pages(
        &mut self,
        path: &str,
        pixel_size: u32,
        codepoints: &[u32],
    ) -> Result<Vec<TextAtlas>, ClientError> {
        if pixel_size == 0 || pixel_size > 1024 {
            return Err(ClientError::BadFont("Invalid TrueType pixel size".to_string()));
        }
        self.require_open()?;
        let bytes = self.services.read(path);
        let Some(bytes) = bytes else {
            return Ok(Vec::new());
        };
        let font = parse_font(&bytes, path)?;
        let mut coverage = Vec::new();
        for code in codepoints {
            validate_codepoint(*code)?;
            if font.cmap_lookup(*code) != 0 {
                coverage.push(*code);
            }
        }
        let mut pages = Vec::new();
        for chunk in coverage.chunks(128) {
            let key = format!(
                "truetype-page:{path}:{pixel_size}:{}",
                chunk.iter().map(|code| code.to_string()).collect::<Vec<_>>().join(",")
            );
            if let Some(cached) = self.cache.get(&key) {
                if let Some(page) = cached.clone() {
                    pages.push(page);
                }
                continue;
            }
            let built = build_font_atlas(&font, chunk, pixel_size, 512)?;
            let mut glyphs = BTreeMap::new();
            for (code, cell) in &built.glyphs {
                glyphs.insert(
                    *code,
                    AtlasGlyph {
                        x: cell.x,
                        y: cell.y,
                        width: cell.width,
                        height: cell.height,
                        advance: cell.width,
                        color: cell.color,
                    },
                );
            }
            let page = self.register(
                AtlasKind::Truetype,
                &key,
                built.width,
                built.height,
                built.pixels,
                built.line_height,
                glyphs,
            )?;
            self.cache.insert(key, Some(page.clone()));
            pages.push(page);
        }
        Ok(pages)
    }

    /// Register an atlas image and scan cap ink (`register`).
    #[allow(clippy::too_many_arguments)]
    fn register(
        &mut self,
        kind: AtlasKind,
        name: &str,
        width: u32,
        height: u32,
        pixels: Vec<u8>,
        line_height: u32,
        glyphs: BTreeMap<u32, AtlasGlyph>,
    ) -> Result<TextAtlas, ClientError> {
        self.require_open()?;
        let image = self.services.register_image(name, width, height, pixels.clone());
        if self.closed {
            self.services.release_image(image);
            return Err(ClientError::BadText(
                "Text font registry closed during image registration".to_string(),
            ));
        }
        self.images.push(image);
        let mut top = line_height as i64;
        let mut bottom = -1i64;
        for code in [72, 48, 49, 50, 51, 52, 53, 54, 55, 56, 57] {
            let Some(glyph) = glyphs.get(&code) else {
                continue;
            };
            for y in 0..glyph.height {
                for x in 0..glyph.width {
                    let at = ((glyph.y + y) * width + glyph.x + x) as usize * 4 + 3;
                    if pixels.get(at).copied().unwrap_or(0) > 127 {
                        top = top.min(y as i64);
                        bottom = bottom.max(y as i64);
                    }
                }
            }
        }
        let cap_ink = if bottom < top {
            None
        } else {
            Some(CapInk {
                top: top as f32 * 8.0 / line_height as f32,
                height: (bottom - top + 1) as f32 * 8.0 / line_height as f32,
            })
        };
        Ok(TextAtlas {
            kind,
            name: name.to_string(),
            picture: ImagePicture { image, width, height },
            line_height,
            cap_ink,
            glyphs,
        })
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
#[derive(Debug, Clone, Copy, PartialEq)]
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
                if let Some(glyph) = atlas_glyph(unicode, codepoint).or_else(|| atlas_glyph(unicode, 63)) {
                    return Ok(glyph);
                }
            }
            classic(63)
        }
        TextFontSelection::Atlas { font, fallbacks, .. } => {
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
            Ok(atlas_glyph(font, 63).unwrap_or(classic(63)?))
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
    let mut seen = std::collections::BTreeSet::new();
    let mut points = Vec::new();
    for ch in text.chars() {
        let code = ch as u32;
        if seen.insert(code) {
            points.push(code);
        }
    }
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
    use std::collections::HashMap;

    struct FixedServices {
        files: HashMap<String, Vec<u8>>,
        images: Vec<(String, u32, u32, Vec<u8>)>,
        released: Vec<u32>,
    }

    impl FixedServices {
        fn new() -> Self {
            Self {
                files: HashMap::new(),
                images: Vec::new(),
                released: Vec::new(),
            }
        }
    }

    impl FontImageServices for FixedServices {
        fn read(&mut self, path: &str) -> Option<Vec<u8>> {
            self.files.get(path).cloned()
        }

        fn register_image(&mut self, name: &str, width: u32, height: u32, rgba: Vec<u8>) -> u32 {
            self.images.push((name.to_string(), width, height, rgba));
            self.images.len() as u32
        }

        fn release_image(&mut self, image: u32) {
            self.released.push(image);
        }
    }

    fn classic() -> TextAtlas {
        classic_charset(1, 128, 128, "conchars", true).unwrap()
    }

    fn tiny_tga(width: u16, height: u16, alpha: u8) -> Vec<u8> {
        let mut bytes = vec![0u8; 18];
        bytes[2] = 2;
        bytes[12..14].copy_from_slice(&width.to_le_bytes());
        bytes[14..16].copy_from_slice(&height.to_le_bytes());
        bytes[16] = 32;
        bytes[17] = 0x28;
        for _ in 0..u32::from(width) * u32::from(height) {
            bytes.extend_from_slice(&[255, 255, 255, alpha]);
        }
        bytes
    }

    fn put_u16(out: &mut Vec<u8>, value: u16) {
        out.extend_from_slice(&value.to_be_bytes());
    }

    fn put_i16(out: &mut Vec<u8>, value: i16) {
        out.extend_from_slice(&value.to_be_bytes());
    }

    fn put_u32(out: &mut Vec<u8>, value: u32) {
        out.extend_from_slice(&value.to_be_bytes());
    }

    fn assemble(tables: &[(&str, Vec<u8>)]) -> Vec<u8> {
        let mut font = Vec::new();
        put_u32(&mut font, 0x0001_0000);
        put_u16(&mut font, tables.len() as u16);
        put_u16(&mut font, 0);
        put_u16(&mut font, 0);
        put_u16(&mut font, 0);
        let mut offset = 12 + tables.len() * 16;
        for (tag, data) in tables {
            let bytes = tag.as_bytes();
            font.extend_from_slice(&[bytes[0], bytes[1], bytes[2], bytes[3]]);
            put_u32(&mut font, 0);
            put_u32(&mut font, offset as u32);
            put_u32(&mut font, data.len() as u32);
            offset += data.len();
            while !offset.is_multiple_of(4) {
                offset += 1;
            }
        }
        for (_, data) in tables {
            font.extend_from_slice(data);
            while !font.len().is_multiple_of(4) {
                font.push(0);
            }
        }
        font
    }

    fn fixture_ttf() -> Vec<u8> {
        let mut head = Vec::new();
        put_u32(&mut head, 0x0001_0000);
        put_u32(&mut head, 0);
        put_u32(&mut head, 0);
        put_u32(&mut head, 0x5f0f_3cf5);
        put_u16(&mut head, 0);
        put_u16(&mut head, 1000);
        head.extend_from_slice(&[0u8; 16]);
        for _ in 0..4 {
            put_i16(&mut head, 0);
        }
        put_u16(&mut head, 0);
        put_u16(&mut head, 0);
        put_i16(&mut head, 0);
        put_i16(&mut head, 1);
        put_i16(&mut head, 0);
        let mut hhea = Vec::new();
        put_u32(&mut hhea, 0x0001_0000);
        put_i16(&mut hhea, 800);
        put_i16(&mut hhea, -200);
        put_i16(&mut hhea, 0);
        put_u16(&mut hhea, 600);
        for _ in 0..6 {
            put_i16(&mut hhea, 0);
        }
        hhea.extend_from_slice(&[0u8; 8]);
        put_i16(&mut hhea, 0);
        put_u16(&mut hhea, 2);
        let mut maxp = Vec::new();
        put_u32(&mut maxp, 0x0001_0000);
        put_u16(&mut maxp, 2);
        let mut hmtx = Vec::new();
        for _ in 0..2 {
            put_u16(&mut hmtx, 600);
            put_i16(&mut hmtx, 0);
        }
        let mut subtable = Vec::new();
        put_u16(&mut subtable, 4);
        put_u16(&mut subtable, 40);
        put_u16(&mut subtable, 0);
        put_u16(&mut subtable, 6);
        put_u16(&mut subtable, 4);
        put_u16(&mut subtable, 1);
        put_u16(&mut subtable, 2);
        for end in [63u16, 72, 0xffff] {
            put_u16(&mut subtable, end);
        }
        put_u16(&mut subtable, 0);
        for start in [63u16, 72, 0xffff] {
            put_u16(&mut subtable, start);
        }
        put_i16(&mut subtable, -62);
        put_i16(&mut subtable, -71);
        put_i16(&mut subtable, 1);
        for _ in 0..3 {
            put_u16(&mut subtable, 0);
        }
        let mut cmap = Vec::new();
        put_u16(&mut cmap, 0);
        put_u16(&mut cmap, 1);
        put_u16(&mut cmap, 3);
        put_u16(&mut cmap, 1);
        put_u32(&mut cmap, 12);
        cmap.extend_from_slice(&subtable);
        let mut glyph = Vec::new();
        put_i16(&mut glyph, 1);
        put_i16(&mut glyph, 0);
        put_i16(&mut glyph, 0);
        put_i16(&mut glyph, 500);
        put_i16(&mut glyph, 700);
        put_u16(&mut glyph, 3);
        put_u16(&mut glyph, 0);
        glyph.extend_from_slice(&[1, 1, 1, 1]);
        for delta in [0i16, 500, 0, -500] {
            put_i16(&mut glyph, delta);
        }
        for delta in [0i16, 0, 700, 0] {
            put_i16(&mut glyph, delta);
        }
        let mut loca = Vec::new();
        put_u32(&mut loca, 0);
        put_u32(&mut loca, 0);
        put_u32(&mut loca, glyph.len() as u32);
        assemble(&[
            ("head", head),
            ("hhea", hhea),
            ("maxp", maxp),
            ("hmtx", hmtx),
            ("cmap", cmap),
            ("loca", loca),
            ("glyf", glyph),
        ])
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

    #[test]
    fn codepoints_keep_first_order() {
        assert_eq!(text_codepoints("baba\u{1f600}a"), vec![98, 97, 0x1f_600]);
        assert!(text_codepoints("").is_empty());
    }

    #[test]
    fn loads_kfont_bitmap() {
        let mut services = FixedServices::new();
        services.files.insert(
            "fonts/test.kfont".to_string(),
            b"texture \"font.tga\"\nmapchar {\n72 0 0 8 8 0\n}\n".to_vec(),
        );
        services.files.insert("font.tga".to_string(), tiny_tga(8, 8, 255));
        let mut registry = TextFontRegistry::new(&mut services);
        let atlas = registry.load_kfont("fonts/test.kfont").unwrap().unwrap();
        assert_eq!(atlas.kind, AtlasKind::Kfont);
        assert_eq!(atlas.line_height, 8);
        assert_eq!(atlas.glyphs[&72].width, 8);
        assert_eq!((atlas.picture.width, atlas.picture.height), (8, 8));
        assert_eq!(atlas.cap_ink, Some(CapInk { top: 0.0, height: 8.0 }));
        let cached = registry.load_kfont("fonts/test.kfont").unwrap().unwrap();
        assert_eq!(cached.picture.image, atlas.picture.image);
        assert!(registry.load_kfont("fonts/missing.kfont").unwrap().is_none());
    }

    #[test]
    fn kfont_rejects_overflowing_glyph() {
        let mut services = FixedServices::new();
        services.files.insert(
            "fonts/test.kfont".to_string(),
            b"texture \"font.tga\"\nmapchar {\n72 0 0 80 8 0\n}\n".to_vec(),
        );
        services.files.insert("font.tga".to_string(), tiny_tga(8, 8, 255));
        let mut registry = TextFontRegistry::new(&mut services);
        let err = registry.load_kfont("fonts/test.kfont").unwrap_err();
        assert!(err.to_string().contains("exceeds"), "{err}");
    }

    #[test]
    fn loads_truetype_atlas() {
        let mut services = FixedServices::new();
        services.files.insert("fonts/test.ttf".to_string(), fixture_ttf());
        let atlas = {
            let mut registry = TextFontRegistry::new(&mut services);
            assert!(registry
                .load_truetype("fonts/missing.ttf", 16, &[72])
                .unwrap()
                .is_none());
            assert!(registry.load_truetype("fonts/test.ttf", 0, &[72]).is_err());
            assert!(registry.load_truetype("fonts/test.ttf", 16, &[0xd800]).is_err());
            registry.load_truetype("fonts/test.ttf", 16, &[72]).unwrap().unwrap()
        };
        assert_eq!(atlas.kind, AtlasKind::Truetype);
        assert!(atlas.glyphs.contains_key(&72));
        assert!(atlas.glyphs.contains_key(&63));
        assert!(!atlas.glyphs.contains_key(&32));
        assert!(atlas.cap_ink.is_some());
        let upload = &services.images[atlas.picture.image as usize - 1];
        assert_eq!((upload.1, upload.2), (atlas.picture.width, atlas.picture.height));
        assert_eq!(
            upload.3.len(),
            atlas.picture.width as usize * atlas.picture.height as usize * 4
        );
        assert!(upload.3.contains(&255));
    }

    #[test]
    fn loads_truetype_pages() {
        let mut services = FixedServices::new();
        services.files.insert("fonts/test.ttf".to_string(), fixture_ttf());
        let mut registry = TextFontRegistry::new(&mut services);
        let pages = registry
            .load_truetype_pages("fonts/test.ttf", 16, &[72, 63, 66])
            .unwrap();
        assert_eq!(pages.len(), 1);
        assert!(pages[0].glyphs.contains_key(&72));
        assert!(pages[0].glyphs.contains_key(&63));
        assert!(!pages[0].glyphs.contains_key(&66));
        let missing = registry.load_truetype_pages("fonts/missing.ttf", 16, &[72]).unwrap();
        assert!(missing.is_empty());
    }

    #[test]
    fn select_falls_back_to_classic() {
        let mut services = FixedServices::new();
        services.files.insert("fonts/test.ttf".to_string(), fixture_ttf());
        let mut registry = TextFontRegistry::new(&mut services);
        let classic_atlas = classic();
        let request = TextFontRequest::Truetype {
            path: "fonts/test.ttf".to_string(),
            pixel_size: 16,
            codepoints: vec![72],
        };
        let selection = registry.select(&request, &classic_atlas).unwrap();
        assert!(matches!(selection, TextFontSelection::Atlas { .. }));
        let missing = TextFontRequest::Truetype {
            path: "fonts/missing.ttf".to_string(),
            pixel_size: 16,
            codepoints: vec![72],
        };
        let fallback = registry.select(&missing, &classic_atlas).unwrap();
        assert!(matches!(fallback, TextFontSelection::Classic { .. }));
    }

    #[test]
    fn close_releases_images_and_blocks_loads() {
        let mut services = FixedServices::new();
        services.files.insert("fonts/test.ttf".to_string(), fixture_ttf());
        {
            let mut registry = TextFontRegistry::new(&mut services);
            registry.load_truetype("fonts/test.ttf", 16, &[72]).unwrap();
            registry.close();
            assert!(registry.load_kfont("fonts/test.kfont").is_err());
            registry.close();
        }
        assert_eq!(services.released.len(), 1);
    }

    #[test]
    fn resolves_unicode_and_alternate_cells() {
        let mut services = FixedServices::new();
        services.files.insert("fonts/test.ttf".to_string(), fixture_ttf());
        let mut registry = TextFontRegistry::new(&mut services);
        let unicode = registry.load_truetype("fonts/test.ttf", 16, &[72]).unwrap().unwrap();
        let classic_selection = TextFontSelection::Classic {
            classic: classic(),
            unicode: Some(unicode),
        };
        let glyph = resolve_text_glyph(&classic_selection, 72, false).unwrap();
        assert_eq!(glyph.codepoint, 72);
        let missing = resolve_text_glyph(&classic_selection, 0x1f_600, false).unwrap();
        assert_eq!(missing.codepoint, 63);
        let atlas_selection = TextFontSelection::Atlas {
            font: registry.load_truetype("fonts/test.ttf", 16, &[72]).unwrap().unwrap(),
            classic: classic(),
            fallbacks: Vec::new(),
        };
        let alternate = resolve_text_glyph(&atlas_selection, 65, true).unwrap();
        // Alternate cell 193 (65 | 128): column 1 of 8 px cells.
        assert_eq!(alternate.glyph.x, 8);
        let picture = atlas_picture(atlas_selection.classic());
        assert!(matches!(picture, PictureAsset::Image(_)));
    }
}
