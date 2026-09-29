//! TrueType engine: parsing, outline flattening, rasterization, atlas builds.
//!
//! Donor provenance: `src/text/truetype.ts` (`parseFont`, `ParsedFontT`,
//! `rasterizeContours`, `rasterizeGlyph`, `rasterizeColorGlyph`,
//! `buildFontAtlas`, `latin1Codepoints`).
//!
//! Unhinted `glyf`/CFF outlines, CID subroutines, legacy `kern`, and
//! COLR v0/CPAL color layers are supported. GPOS, COLR v1 paints, CFF
//! arithmetic, and `seac` composition are unsupported, matching the
//! donor.

use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};

use qa_core::binary::{BinaryError, BinaryReader};

use crate::ClientError;

/// A table record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TableRecord {
    /// Offset.
    pub offset: usize,
    /// Length.
    pub length: usize,
}

/// Outline format.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutlineFormat {
    /// TrueType outlines.
    Glyf,
    /// CFF outlines.
    Cff,
}

/// Face metrics (`ParsedFontT` scalar surface).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FontMetrics {
    /// Units per em.
    pub units_per_em: u16,
    /// Glyph count.
    pub num_glyphs: u16,
    /// Ascender.
    pub ascent: i16,
    /// Descender.
    pub descent: i16,
    /// Line gap.
    pub line_gap: i16,
    /// Outline format.
    pub outline_format: OutlineFormat,
}

/// Scaled metrics at a pixel size.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ScaledMetrics {
    /// Pixels per em.
    pub pixels_per_em: f32,
    /// Line height in pixels.
    pub line_height: f32,
}

/// A flattened outline point (`FlattenedContourT` element, font units).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ContourPoint {
    /// X coordinate.
    pub x: f64,
    /// Y coordinate.
    pub y: f64,
}

/// A flattened contour (`FlattenedContourT`).
pub type FlattenedContour = Vec<ContourPoint>;

/// A raw (unflattened) outline point.
#[derive(Debug, Clone, Copy, PartialEq)]
struct RawPoint {
    x: f64,
    y: f64,
    on_curve: bool,
}

/// A COLR v0 color layer (`ColorLayerT`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ColorLayer {
    /// Layer glyph id.
    pub gid: u32,
    /// Palette index (`0xffff` selects the foreground color).
    pub palette_index: u16,
}

/// Foreground-color sentinel (`COLR_FOREGROUND_SENTINEL`).
pub const COLR_FOREGROUND_SENTINEL: u16 = 0xffff;

/// An RGBA palette color.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PaletteColor {
    /// Red.
    pub r: u8,
    /// Green.
    pub g: u8,
    /// Blue.
    pub b: u8,
    /// Alpha.
    pub a: u8,
}

impl PaletteColor {
    /// Opaque white (default color-glyph foreground).
    pub const WHITE: Self = Self {
        r: 255,
        g: 255,
        b: 255,
        a: 255,
    };
}

/// A rasterized monochrome glyph (`RasterizedGlyphT`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RasterizedGlyph {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// Row-major coverage bytes (0-255), top row first.
    pub coverage: Vec<u8>,
}

/// A rasterized color glyph (`RasterizedColorGlyphT`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RasterizedColorGlyph {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// Row-major RGBA bytes, top row first.
    pub pixels: Vec<u8>,
}

/// A packed atlas cell (`AtlasRectT`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AtlasRect {
    /// X offset in the atlas.
    pub x: u32,
    /// Y offset in the atlas.
    pub y: u32,
    /// Cell width.
    pub width: u32,
    /// Cell height.
    pub height: u32,
    /// True for color (RGBA) cells, false for coverage cells.
    pub color: bool,
}

/// A packed font atlas (`FontAtlasT`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FontAtlas {
    /// Atlas width in pixels.
    pub width: u32,
    /// Atlas height in pixels.
    pub height: u32,
    /// Row-major RGBA bytes.
    pub pixels: Vec<u8>,
    /// Cells by codepoint.
    pub glyphs: BTreeMap<u32, AtlasRect>,
    /// Tallest cell height in pixels.
    pub line_height: u32,
}

/// A cmap lookup closure source.
#[derive(Debug, Clone, Copy)]
struct CmapSubtable {
    offset: usize,
    format: u16,
}

/// `glyf` outline storage.
#[derive(Debug, Clone)]
struct GlyfOutlines {
    offset: usize,
    length: usize,
    loca: Vec<u32>,
}

/// A parsed CFF font (`CffFont`).
#[derive(Debug, Clone)]
struct CffFont {
    char_strings: Vec<Vec<u8>>,
    global_subrs: Vec<Vec<u8>>,
    global_bias: usize,
    is_cid: bool,
    default_local_subrs: Vec<Vec<u8>>,
    default_local_bias: usize,
    fd_local_subrs: Vec<Vec<Vec<u8>>>,
    fd_local_bias: Vec<usize>,
    fd_select: Option<Vec<u8>>,
}

/// A parsed CPAL table.
#[derive(Debug, Clone)]
struct CpalTable {
    num_entries: usize,
    num_palettes: usize,
    indices: Vec<u16>,
    records: Vec<PaletteColor>,
}

/// A parsed font face (`ParsedFontT`).
#[derive(Debug, Clone)]
pub struct ParsedFont {
    tables: Vec<(String, TableRecord)>,
    metrics: FontMetrics,
    cmap: CmapSubtable,
    advances: Vec<u16>,
    bytes: Vec<u8>,
    source: String,
    glyf: Option<GlyfOutlines>,
    cff: Option<CffFont>,
    kern: HashMap<u32, i32>,
    colr_base: HashMap<u32, (usize, usize)>,
    colr_layers: Vec<(u16, u16)>,
    cpal: Option<CpalTable>,
    contours_cache: RefCell<HashMap<u32, Vec<FlattenedContour>>>,
}

impl ParsedFont {
    /// Face metrics.
    #[must_use]
    pub const fn metrics(&self) -> FontMetrics {
        self.metrics
    }

    /// Table record by tag.
    #[must_use]
    pub fn table(&self, tag: &str) -> Option<TableRecord> {
        self.tables
            .iter()
            .find(|(name, _)| name == tag)
            .map(|(_, record)| *record)
    }

    /// Glyph id for a codepoint (`cmapLookup`, 0 when unmapped).
    #[must_use]
    pub fn cmap_lookup(&self, codepoint: u32) -> u32 {
        let view = &self.bytes;
        match self.cmap.format {
            4 => {
                if codepoint > 0xffff {
                    return 0;
                }
                let base = self.cmap.offset;
                let seg_count = get_u16(view, base + 6) as usize / 2;
                let end_base = base + 14;
                let start_base = end_base + seg_count * 2 + 2;
                let delta_base = start_base + seg_count * 2;
                let range_base = delta_base + seg_count * 2;
                for index in 0..seg_count {
                    let end = u32::from(get_u16(view, end_base + index * 2));
                    if codepoint > end {
                        continue;
                    }
                    let start = u32::from(get_u16(view, start_base + index * 2));
                    if codepoint < start {
                        return 0;
                    }
                    let delta = i32::from(get_i16(view, delta_base + index * 2));
                    let range_offset = usize::from(get_u16(view, range_base + index * 2));
                    if range_offset == 0 {
                        return ((codepoint as i32 + delta) & 0xffff) as u32;
                    }
                    let address = range_base + index * 2 + range_offset + (codepoint - start) as usize * 2;
                    let glyph = u32::from(get_u16(view, address));
                    if glyph == 0 {
                        return 0;
                    }
                    return ((glyph as i32 + delta) & 0xffff) as u32;
                }
                0
            }
            _ => {
                let base = self.cmap.offset;
                let groups = get_u32(view, base + 12) as usize;
                let groups_base = base + 16;
                let (mut low, mut high) = (0usize, groups.saturating_sub(1));
                while groups > 0 && low <= high {
                    let mid = (low + high) >> 1;
                    let group = groups_base + mid * 12;
                    let start = get_u32(view, group);
                    let end = get_u32(view, group + 4);
                    if codepoint < start {
                        if mid == 0 {
                            break;
                        }
                        high = mid - 1;
                    } else if codepoint > end {
                        low = mid + 1;
                    } else {
                        return get_u32(view, group + 8) + (codepoint - start);
                    }
                }
                0
            }
        }
    }

    /// Advance width for a glyph id (`advanceWidth`, 0 when invalid).
    #[must_use]
    pub fn advance_width(&self, gid: u32) -> u16 {
        if gid as usize >= self.advances.len() {
            return 0;
        }
        self.advances[gid as usize]
    }

    /// Kerning adjustment for a glyph pair (`kerning`, 0 when absent).
    #[must_use]
    pub fn kerning(&self, left_gid: u32, right_gid: u32) -> i32 {
        let key = left_gid.saturating_mul(65536).saturating_add(right_gid);
        self.kern.get(&key).copied().unwrap_or(0)
    }

    /// COLR v0 color layers for a glyph id (`colorLayers`).
    #[must_use]
    pub fn color_layers(&self, gid: u32) -> Option<Vec<ColorLayer>> {
        let (first, num) = *self.colr_base.get(&gid)?;
        let mut out = Vec::new();
        for index in 0..num {
            let (layer_gid, palette_index) = *self.colr_layers.get(first + index)?;
            out.push(ColorLayer {
                gid: u32::from(layer_gid),
                palette_index,
            });
        }
        Some(out)
    }

    /// CPAL palette color (`paletteColor`).
    #[must_use]
    pub fn palette_color(&self, palette_index: u32, palette_id: u32) -> Option<PaletteColor> {
        let cpal = self.cpal.as_ref()?;
        if palette_id as usize >= cpal.num_palettes || palette_index as usize >= cpal.num_entries {
            return None;
        }
        let first = usize::from(cpal.indices[palette_id as usize]);
        cpal.records.get(first + palette_index as usize).copied()
    }

    /// Flattened outlines for a glyph id (`contours`).
    pub fn contours(&self, gid: u32) -> Result<Vec<FlattenedContour>, ClientError> {
        if let Some(cached) = self.contours_cache.borrow().get(&gid) {
            return Ok(cached.clone());
        }
        let flat = self.decode_contours(gid)?;
        self.contours_cache.borrow_mut().insert(gid, flat.clone());
        Ok(flat)
    }

    /// Scale metrics to a pixel size.
    pub fn scaled_metrics(&self, pixel_size: u32) -> Result<ScaledMetrics, ClientError> {
        if pixel_size == 0 || pixel_size > 1024 {
            return Err(ClientError::BadFont("Invalid TrueType pixel size".to_string()));
        }
        let scale = pixel_size as f32 / f32::from(self.metrics.units_per_em);
        Ok(ScaledMetrics {
            pixels_per_em: pixel_size as f32,
            line_height: f32::from(self.metrics.ascent - self.metrics.descent + self.metrics.line_gap) * scale,
        })
    }

    fn decode_contours(&self, gid: u32) -> Result<Vec<FlattenedContour>, ClientError> {
        if self.metrics.outline_format == OutlineFormat::Cff {
            let cff = self
                .cff
                .as_ref()
                .ok_or_else(|| bad_font(&self.source, "malformed CFF table"))?;
            return cff_contours(&self.source, cff, gid);
        }
        let glyf = self
            .glyf
            .as_ref()
            .ok_or_else(|| bad_font(&self.source, "no outline table found (need glyf+loca or CFF )"))?;
        if gid as usize >= self.metrics.num_glyphs as usize {
            return Ok(Vec::new());
        }
        let raw = glyf_raw(&self.source, &self.bytes, glyf, gid, 0)?;
        Ok(raw.iter().map(Vec::as_slice).map(flatten_quadratic_contour).collect())
    }
}

fn bad_font(source: &str, reason: &str) -> ClientError {
    ClientError::BadFont(format!("{source}: {reason}"))
}

/// Total big-endian readers for lazy lookup paths (0 when out of range).
fn get_u16(bytes: &[u8], offset: usize) -> u16 {
    bytes
        .get(offset..offset.saturating_add(2))
        .and_then(|window| <[u8; 2]>::try_from(window).ok())
        .map(u16::from_be_bytes)
        .unwrap_or(0)
}

fn get_i16(bytes: &[u8], offset: usize) -> i16 {
    bytes
        .get(offset..offset.saturating_add(2))
        .and_then(|window| <[u8; 2]>::try_from(window).ok())
        .map(i16::from_be_bytes)
        .unwrap_or(0)
}

fn get_u32(bytes: &[u8], offset: usize) -> u32 {
    bytes
        .get(offset..offset.saturating_add(4))
        .and_then(|window| <[u8; 4]>::try_from(window).ok())
        .map(u32::from_be_bytes)
        .unwrap_or(0)
}

/// Checked big-endian readers for outline decoding.
fn read_u8(bytes: &[u8], offset: usize, source: &str, what: &str) -> Result<u8, ClientError> {
    bytes
        .get(offset)
        .copied()
        .ok_or_else(|| bad_font(source, &format!("{what} exceeds table bounds")))
}

fn read_i8(bytes: &[u8], offset: usize, source: &str, what: &str) -> Result<i8, ClientError> {
    read_u8(bytes, offset, source, what).map(|value| value as i8)
}

fn read_u16(bytes: &[u8], offset: usize, source: &str, what: &str) -> Result<u16, ClientError> {
    bytes
        .get(offset..offset.saturating_add(2))
        .and_then(|window| <[u8; 2]>::try_from(window).ok())
        .map(u16::from_be_bytes)
        .ok_or_else(|| bad_font(source, &format!("{what} exceeds table bounds")))
}

fn read_i16(bytes: &[u8], offset: usize, source: &str, what: &str) -> Result<i16, ClientError> {
    bytes
        .get(offset..offset.saturating_add(2))
        .and_then(|window| <[u8; 2]>::try_from(window).ok())
        .map(i16::from_be_bytes)
        .ok_or_else(|| bad_font(source, &format!("{what} exceeds table bounds")))
}

fn read_u32(bytes: &[u8], offset: usize, source: &str, what: &str) -> Result<u32, ClientError> {
    bytes
        .get(offset..offset.saturating_add(4))
        .and_then(|window| <[u8; 4]>::try_from(window).ok())
        .map(u32::from_be_bytes)
        .ok_or_else(|| bad_font(source, &format!("{what} exceeds table bounds")))
}

fn read_tag(bytes: &[u8], offset: usize) -> Result<String, ClientError> {
    if offset + 4 > bytes.len() {
        return Err(ClientError::BadFont(
            "not a recognized sfnt file (bad signature)".to_string(),
        ));
    }
    Ok(String::from_utf8_lossy(&bytes[offset..offset + 4]).into_owned())
}

/// Parse an sfnt font (`parseFont`).
pub fn parse_font(bytes: &[u8], source: &str) -> Result<ParsedFont, ClientError> {
    let map_err = |error: BinaryError| ClientError::BadFont(format!("{source}: {error}"));
    if bytes.len() < 12 {
        return Err(bad_font(source, "not a recognized sfnt file (bad signature)"));
    }
    let version = get_u32(bytes, 0);
    let tag = read_tag(bytes, 0).map_err(|_| bad_font(source, "not a recognized sfnt file (bad signature)"))?;
    if version != 0x0001_0000 && tag != "OTTO" && tag != "true" && version != 0x7472_7565 {
        return Err(bad_font(source, "not a recognized sfnt file (bad signature)"));
    }
    let reader = BinaryReader::new(bytes, source);
    let num_tables = get_u16(bytes, 4) as usize;
    let mut tables: Vec<(String, TableRecord)> = Vec::new();
    for index in 0..num_tables {
        let entry = 12 + index * 16;
        if entry + 16 > bytes.len() {
            return Err(bad_font(source, "not a recognized sfnt file (bad signature)"));
        }
        let tag = read_tag(bytes, entry).map_err(|_| bad_font(source, "not a recognized sfnt file (bad signature)"))?;
        let offset = get_u32(bytes, entry + 8) as usize;
        let length = get_u32(bytes, entry + 12) as usize;
        reader.view(offset, length).map_err(map_err)?;
        tables.push((tag, TableRecord { offset, length }));
    }
    let table = |name: &str| tables.iter().find(|(tag, _)| tag == name).map(|(_, record)| *record);
    let (Some(head), Some(hhea), Some(maxp), Some(hmtx), Some(cmap)) = (
        table("head"),
        table("hhea"),
        table("maxp"),
        table("hmtx"),
        table("cmap"),
    ) else {
        return Err(bad_font(source, "missing a required table (head/hhea/maxp/hmtx/cmap)"));
    };
    if head.length < 54 {
        return Err(bad_font(source, "head table too short"));
    }
    if hhea.length < 36 {
        return Err(bad_font(source, "hhea table too short"));
    }
    if maxp.length < 6 {
        return Err(bad_font(source, "maxp table too short"));
    }
    let units_per_em = get_u16(bytes, head.offset + 18);
    if units_per_em == 0 {
        return Err(bad_font(source, "invalid unitsPerEm"));
    }
    let index_to_loc_format = get_i16(bytes, head.offset + 50);
    let ascent = get_i16(bytes, hhea.offset + 4);
    let descent = get_i16(bytes, hhea.offset + 6);
    let line_gap = get_i16(bytes, hhea.offset + 8);
    let number_of_h_metrics = get_u16(bytes, hhea.offset + 34);
    let num_glyphs = get_u16(bytes, maxp.offset + 4);
    if number_of_h_metrics < 1
        || number_of_h_metrics > num_glyphs
        || hmtx.length < usize::from(number_of_h_metrics) * 4 + usize::from(num_glyphs - number_of_h_metrics) * 2
    {
        return Err(bad_font(source, "invalid horizontal metrics"));
    }
    let mut advances = vec![0u16; usize::from(num_glyphs)];
    let mut offset = hmtx.offset;
    let mut last = 0u16;
    for (glyph, advance) in advances.iter_mut().enumerate() {
        if glyph < usize::from(number_of_h_metrics) {
            last = get_u16(bytes, offset);
            offset += 4;
        }
        *advance = last;
    }
    let cmap_subtable = select_cmap(bytes, cmap.offset, source)?
        .ok_or_else(|| bad_font(source, "no supported cmap subtable (need format 4 or 12)"))?;
    let (glyf, cff, outline_format) = select_outlines(bytes, source, &table, index_to_loc_format, num_glyphs)?;
    let kern = parse_kern(bytes, source, table("kern"))?;
    let (colr_base, colr_layers) = parse_colr(bytes, source, table("COLR"))?;
    let cpal = parse_cpal(bytes, source, table("CPAL"))?;
    Ok(ParsedFont {
        tables,
        metrics: FontMetrics {
            units_per_em,
            num_glyphs,
            ascent,
            descent,
            line_gap,
            outline_format,
        },
        cmap: cmap_subtable,
        advances,
        bytes: bytes.to_vec(),
        source: source.to_string(),
        glyf,
        cff,
        kern,
        colr_base,
        colr_layers,
        cpal,
        contours_cache: RefCell::new(HashMap::new()),
    })
}

fn select_cmap(bytes: &[u8], cmap_offset: usize, source: &str) -> Result<Option<CmapSubtable>, ClientError> {
    if cmap_offset + 4 > bytes.len() {
        return Ok(None);
    }
    let num_tables = get_u16(bytes, cmap_offset + 2) as usize;
    let mut best: Option<(u16, i32, usize)> = None;
    for index in 0..num_tables {
        let record = cmap_offset + 4 + index * 8;
        if record + 8 > bytes.len() {
            return Err(bad_font(source, "cmap table too short"));
        }
        let platform = get_u16(bytes, record);
        let encoding = get_u16(bytes, record + 2);
        let sub_offset = cmap_offset + get_u32(bytes, record + 4) as usize;
        if sub_offset + 2 > bytes.len() {
            return Err(bad_font(source, "cmap table too short"));
        }
        let format = get_u16(bytes, sub_offset);
        let score = if format == 12
            && ((platform == 3 && encoding == 10) || (platform == 0 && (encoding == 4 || encoding == 6)))
        {
            100
        } else if format == 12 {
            90
        } else if format == 4 && ((platform == 3 && encoding == 1) || platform == 0) {
            80
        } else if format == 4 {
            70
        } else {
            -1
        };
        if score > best.map_or(-1, |(_, score, _)| score) {
            best = Some((format, score, sub_offset));
        }
    }
    Ok(best.map(|(format, _, offset)| CmapSubtable { offset, format }))
}

fn select_outlines(
    bytes: &[u8],
    source: &str,
    table: &dyn Fn(&str) -> Option<TableRecord>,
    index_to_loc_format: i16,
    num_glyphs: u16,
) -> Result<(Option<GlyfOutlines>, Option<CffFont>, OutlineFormat), ClientError> {
    if let (Some(glyf), Some(loca)) = (table("glyf"), table("loca")) {
        let stride = if index_to_loc_format == 0 { 2 } else { 4 };
        if (index_to_loc_format != 0 && index_to_loc_format != 1)
            || loca.length < (usize::from(num_glyphs) + 1) * stride
        {
            return Err(bad_font(source, "invalid loca table"));
        }
        let mut offsets = Vec::with_capacity(usize::from(num_glyphs) + 1);
        for index in 0..=usize::from(num_glyphs) {
            if index_to_loc_format == 0 {
                offsets.push(u32::from(read_u16(bytes, loca.offset + index * 2, source, "loca")?) * 2);
            } else {
                offsets.push(read_u32(bytes, loca.offset + index * 4, source, "loca")?);
            }
        }
        return Ok((
            Some(GlyfOutlines {
                offset: glyf.offset,
                length: glyf.length,
                loca: offsets,
            }),
            None,
            OutlineFormat::Glyf,
        ));
    }
    if let Some(cff) = table("CFF ") {
        let end = cff.offset.saturating_add(cff.length);
        let Some(view) = bytes.get(cff.offset..end) else {
            return Err(bad_font(source, "malformed CFF table"));
        };
        let font = parse_cff(view).map_err(|reason| bad_font(source, &reason))?;
        let Some(font) = font else {
            return Err(bad_font(source, "malformed CFF table"));
        };
        return Ok((None, Some(font), OutlineFormat::Cff));
    }
    Err(bad_font(source, "no outline table found (need glyf+loca or CFF )"))
}

/// Legacy `kern` format 0 pairs (`parseKernFormat0`).
fn parse_kern(bytes: &[u8], source: &str, kern: Option<TableRecord>) -> Result<HashMap<u32, i32>, ClientError> {
    let mut pairs = HashMap::new();
    let Some(kern) = kern else {
        return Ok(pairs);
    };
    if kern.length < 4 {
        return Ok(pairs);
    }
    if read_u16(bytes, kern.offset, source, "kern")? != 0 {
        return Ok(pairs);
    }
    let num_tables = read_u16(bytes, kern.offset + 2, source, "kern")? as usize;
    let mut position = kern.offset + 4;
    for _ in 0..num_tables {
        if position + 6 > kern.offset + kern.length {
            break;
        }
        let sub_version = read_u16(bytes, position, source, "kern")?;
        let sub_length = read_u16(bytes, position + 2, source, "kern")? as usize;
        let coverage = read_u16(bytes, position + 4, source, "kern")?;
        if sub_version == 0 && coverage >> 8 == 0 {
            let num_pairs = read_u16(bytes, position + 6, source, "kern")? as usize;
            let mut cursor = position + 14;
            for _ in 0..num_pairs {
                let left = u32::from(read_u16(bytes, cursor, source, "kern")?);
                let right = u32::from(read_u16(bytes, cursor + 2, source, "kern")?);
                let value = i32::from(read_i16(bytes, cursor + 4, source, "kern")?);
                pairs.insert(left * 65536 + right, value);
                cursor += 6;
            }
        }
        position += sub_length.max(6);
    }
    Ok(pairs)
}

/// COLR v0 base records plus layer records.
type ColrTables = (HashMap<u32, (usize, usize)>, Vec<(u16, u16)>);

/// COLR v0 base/layer records (`parseCOLR`).
fn parse_colr(bytes: &[u8], source: &str, colr: Option<TableRecord>) -> Result<ColrTables, ClientError> {
    let empty = (HashMap::new(), Vec::new());
    let Some(colr) = colr else {
        return Ok(empty);
    };
    if colr.length < 14 {
        return Ok(empty);
    }
    if read_u16(bytes, colr.offset, source, "COLR")? != 0 {
        return Ok(empty);
    }
    let num_base = read_u16(bytes, colr.offset + 2, source, "COLR")? as usize;
    let base_offset = colr.offset + read_u32(bytes, colr.offset + 4, source, "COLR")? as usize;
    let layer_offset = colr.offset + read_u32(bytes, colr.offset + 8, source, "COLR")? as usize;
    let num_layers = read_u16(bytes, colr.offset + 12, source, "COLR")? as usize;
    let mut base = HashMap::new();
    let mut cursor = base_offset;
    for _ in 0..num_base {
        let gid = u32::from(read_u16(bytes, cursor, source, "COLR")?);
        let first = read_u16(bytes, cursor + 2, source, "COLR")? as usize;
        let num = read_u16(bytes, cursor + 4, source, "COLR")? as usize;
        base.insert(gid, (first, num));
        cursor += 6;
    }
    let mut layers = Vec::new();
    cursor = layer_offset;
    for _ in 0..num_layers {
        let gid = read_u16(bytes, cursor, source, "COLR")?;
        let palette_index = read_u16(bytes, cursor + 2, source, "COLR")?;
        layers.push((gid, palette_index));
        cursor += 4;
    }
    Ok((base, layers))
}

/// CPAL palette records (`parseCPAL`).
fn parse_cpal(bytes: &[u8], source: &str, cpal: Option<TableRecord>) -> Result<Option<CpalTable>, ClientError> {
    let Some(cpal) = cpal else {
        return Ok(None);
    };
    if cpal.length < 12 {
        return Ok(None);
    }
    let num_entries = read_u16(bytes, cpal.offset + 2, source, "CPAL")? as usize;
    let num_palettes = read_u16(bytes, cpal.offset + 4, source, "CPAL")? as usize;
    let num_records = read_u16(bytes, cpal.offset + 6, source, "CPAL")? as usize;
    let records_offset = cpal.offset + read_u32(bytes, cpal.offset + 8, source, "CPAL")? as usize;
    let mut indices = Vec::with_capacity(num_palettes);
    for index in 0..num_palettes {
        indices.push(read_u16(bytes, cpal.offset + 12 + index * 2, source, "CPAL")?);
    }
    let mut records = Vec::with_capacity(num_records);
    for index in 0..num_records {
        let cursor = records_offset + index * 4;
        let blue = read_u8(bytes, cursor, source, "CPAL")?;
        let green = read_u8(bytes, cursor + 1, source, "CPAL")?;
        let red = read_u8(bytes, cursor + 2, source, "CPAL")?;
        let alpha = read_u8(bytes, cursor + 3, source, "CPAL")?;
        records.push(PaletteColor {
            r: red,
            g: green,
            b: blue,
            a: alpha,
        });
    }
    Ok(Some(CpalTable {
        num_entries,
        num_palettes,
        indices,
        records,
    }))
}

/// Raw `glyf` contours for a glyph id (`getRaw`).
fn glyf_raw(
    source: &str,
    bytes: &[u8],
    glyf: &GlyfOutlines,
    gid: u32,
    depth: u32,
) -> Result<Vec<Vec<RawPoint>>, ClientError> {
    if depth > 8 {
        return Err(bad_font(source, "Composite glyph recursion exceeds eight levels"));
    }
    let Some(start) = glyf.loca.get(gid as usize).copied() else {
        return Ok(Vec::new());
    };
    let Some(end) = glyf.loca.get(gid as usize + 1).copied() else {
        return Ok(Vec::new());
    };
    if end < start || end as usize > glyf.length {
        return Err(bad_font(source, "Glyph location exceeds glyf table bounds"));
    }
    if end == start {
        return Ok(Vec::new());
    }
    let base = glyf.offset + start as usize;
    let Some(view) = bytes.get(base..base + (end - start) as usize) else {
        return Err(bad_font(source, "Glyph location exceeds glyf table bounds"));
    };
    let num_contours = read_i16(view, 0, source, "glyph")?;
    if num_contours >= 0 {
        parse_simple_glyph(view, source, num_contours as usize)
    } else {
        parse_composite_glyph(source, bytes, glyf, view, depth)
    }
}

/// A simple `glyf` glyph (`parseSimpleGlyph`).
fn parse_simple_glyph(view: &[u8], source: &str, num_contours: usize) -> Result<Vec<Vec<RawPoint>>, ClientError> {
    let mut cursor = 10usize;
    let mut end_points = Vec::with_capacity(num_contours);
    for _ in 0..num_contours {
        end_points.push(read_u16(view, cursor, source, "glyph")? as usize);
        cursor += 2;
    }
    let num_points = end_points.last().copied().map_or(0, |last| last + 1);
    let instruction_length = read_u16(view, cursor, source, "glyph")? as usize;
    cursor += 2 + instruction_length;
    let mut flags = Vec::new();
    while flags.len() < num_points {
        let flag = read_u8(view, cursor, source, "glyph")?;
        cursor += 1;
        flags.push(flag);
        if flag & 0x08 != 0 {
            let mut repeat = read_u8(view, cursor, source, "glyph")? as usize;
            cursor += 1;
            while repeat > 0 && flags.len() < num_points {
                flags.push(flag);
                repeat -= 1;
            }
        }
    }
    let mut xs = Vec::with_capacity(num_points);
    let mut x = 0i32;
    for flag in &flags {
        if flag & 0x02 != 0 {
            let dx = i32::from(read_u8(view, cursor, source, "glyph")?);
            cursor += 1;
            x += if flag & 0x10 != 0 { dx } else { -dx };
        } else if flag & 0x10 == 0 {
            x += i32::from(read_i16(view, cursor, source, "glyph")?);
            cursor += 2;
        }
        xs.push(x);
    }
    let mut ys = Vec::with_capacity(num_points);
    let mut y = 0i32;
    for flag in &flags {
        if flag & 0x04 != 0 {
            let dy = i32::from(read_u8(view, cursor, source, "glyph")?);
            cursor += 1;
            y += if flag & 0x20 != 0 { dy } else { -dy };
        } else if flag & 0x20 == 0 {
            y += i32::from(read_i16(view, cursor, source, "glyph")?);
            cursor += 2;
        }
        ys.push(y);
    }
    let mut contours = Vec::new();
    let mut start = 0usize;
    for end in end_points {
        if end >= num_points {
            return Err(bad_font(source, "glyph contour exceeds point count"));
        }
        let mut points = Vec::new();
        for index in start..=end {
            points.push(RawPoint {
                x: f64::from(xs[index]),
                y: f64::from(ys[index]),
                on_curve: flags[index] & 0x01 != 0,
            });
        }
        contours.push(points);
        start = end + 1;
    }
    Ok(contours)
}

const ARG_1_AND_2_ARE_WORDS: u16 = 0x0001;
const ARGS_ARE_XY_VALUES: u16 = 0x0002;
const WE_HAVE_A_SCALE: u16 = 0x0008;
const MORE_COMPONENTS: u16 = 0x0020;
const WE_HAVE_AN_X_AND_Y_SCALE: u16 = 0x0040;
const WE_HAVE_A_TWO_BY_TWO: u16 = 0x0080;

/// A composite `glyf` glyph (`parseCompositeGlyph`).
fn parse_composite_glyph(
    source: &str,
    bytes: &[u8],
    glyf: &GlyfOutlines,
    view: &[u8],
    depth: u32,
) -> Result<Vec<Vec<RawPoint>>, ClientError> {
    let mut cursor = 10usize;
    let mut contours = Vec::new();
    loop {
        let flags = read_u16(view, cursor, source, "glyph")?;
        cursor += 2;
        let glyph_index = u32::from(read_u16(view, cursor, source, "glyph")?);
        cursor += 2;
        let (mut dx, mut dy) = (0.0, 0.0);
        if flags & ARG_1_AND_2_ARE_WORDS != 0 {
            let first = read_i16(view, cursor, source, "glyph")?;
            let second = read_i16(view, cursor + 2, source, "glyph")?;
            cursor += 4;
            if flags & ARGS_ARE_XY_VALUES != 0 {
                dx = f64::from(first);
                dy = f64::from(second);
            }
        } else {
            let first = read_i8(view, cursor, source, "glyph")?;
            let second = read_i8(view, cursor + 1, source, "glyph")?;
            cursor += 2;
            if flags & ARGS_ARE_XY_VALUES != 0 {
                dx = f64::from(first);
                dy = f64::from(second);
            }
        }
        let (mut a, mut b, mut c, mut d) = (1.0, 0.0, 0.0, 1.0);
        if flags & WE_HAVE_A_SCALE != 0 {
            a = f64::from(read_i16(view, cursor, source, "glyph")?) / 16384.0;
            d = a;
            cursor += 2;
        } else if flags & WE_HAVE_AN_X_AND_Y_SCALE != 0 {
            a = f64::from(read_i16(view, cursor, source, "glyph")?) / 16384.0;
            d = f64::from(read_i16(view, cursor + 2, source, "glyph")?) / 16384.0;
            cursor += 4;
        } else if flags & WE_HAVE_A_TWO_BY_TWO != 0 {
            a = f64::from(read_i16(view, cursor, source, "glyph")?) / 16384.0;
            b = f64::from(read_i16(view, cursor + 2, source, "glyph")?) / 16384.0;
            c = f64::from(read_i16(view, cursor + 4, source, "glyph")?) / 16384.0;
            d = f64::from(read_i16(view, cursor + 6, source, "glyph")?) / 16384.0;
            cursor += 8;
        }
        if depth < 8 {
            for contour in glyf_raw(source, bytes, glyf, glyph_index, depth + 1)? {
                contours.push(
                    contour
                        .iter()
                        .map(|point| RawPoint {
                            x: a * point.x + c * point.y + dx,
                            y: b * point.x + d * point.y + dy,
                            on_curve: point.on_curve,
                        })
                        .collect(),
                );
            }
        }
        if flags & MORE_COMPONENTS == 0 {
            break;
        }
    }
    Ok(contours)
}

/// Flatten one quadratic contour (`flattenQuadraticContour`).
fn flatten_quadratic_contour(raw_points: &[RawPoint]) -> FlattenedContour {
    let count = raw_points.len();
    if count < 2 {
        return Vec::new();
    }
    let mut expanded = Vec::with_capacity(count + 1);
    for (index, current) in raw_points.iter().enumerate() {
        let next = &raw_points[(index + 1) % count];
        expanded.push(*current);
        if !current.on_curve && !next.on_curve {
            expanded.push(RawPoint {
                x: (current.x + next.x) / 2.0,
                y: (current.y + next.y) / 2.0,
                on_curve: true,
            });
        }
    }
    let Some(start) = expanded.iter().position(|point| point.on_curve) else {
        return Vec::new();
    };
    let mut ring = Vec::with_capacity(expanded.len());
    ring.extend_from_slice(&expanded[start..]);
    ring.extend_from_slice(&expanded[..start]);
    let len = ring.len();
    let mut out = vec![ContourPoint {
        x: ring[0].x,
        y: ring[0].y,
    }];
    let mut current = ContourPoint {
        x: ring[0].x,
        y: ring[0].y,
    };
    let mut index = 1usize;
    while index < len {
        let point = &ring[index];
        if point.on_curve {
            out.push(ContourPoint { x: point.x, y: point.y });
            current = ContourPoint { x: point.x, y: point.y };
            index += 1;
        } else {
            let end = &ring[(index + 1) % len];
            let end_point = ContourPoint { x: end.x, y: end.y };
            flatten_quad_segment(current, ContourPoint { x: point.x, y: point.y }, end_point, &mut out);
            current = end_point;
            index += 2;
        }
    }
    out
}

/// Flatten one quadratic segment (`flattenQuadSegment`).
fn flatten_quad_segment(start: ContourPoint, control: ContourPoint, end: ContourPoint, out: &mut FlattenedContour) {
    const SEGMENTS: u32 = 8;
    for step in 1..=SEGMENTS {
        let t = f64::from(step) / f64::from(SEGMENTS);
        let mt = 1.0 - t;
        out.push(ContourPoint {
            x: mt * mt * start.x + 2.0 * mt * t * control.x + t * t * end.x,
            y: mt * mt * start.y + 2.0 * mt * t * control.y + t * t * end.y,
        });
    }
}

/// Read a CFF INDEX (`readCffIndex`).
fn read_cff_index(data: &[u8], position: usize) -> Result<(Vec<Vec<u8>>, usize), String> {
    if position + 2 > data.len() {
        return Err("CFF INDEX exceeds table bounds".to_string());
    }
    let count = u16::from_be_bytes([data[position], data[position + 1]]) as usize;
    let mut cursor = position + 2;
    if count == 0 {
        return Ok((Vec::new(), cursor));
    }
    let Some(off_size) = data.get(cursor).copied() else {
        return Err("CFF INDEX exceeds table bounds".to_string());
    };
    if !(1..=4).contains(&off_size) {
        return Err(format!("Invalid CFF INDEX offset size {off_size}"));
    }
    cursor += 1;
    let mut offsets = Vec::with_capacity(count + 1);
    for _ in 0..=count {
        if cursor + off_size as usize > data.len() {
            return Err("CFF INDEX exceeds table bounds".to_string());
        }
        let mut value = 0usize;
        for _ in 0..off_size {
            value = value * 256 + usize::from(data[cursor]);
            cursor += 1;
        }
        offsets.push(value);
    }
    let data_start = cursor - 1;
    if offsets[0] != 1 {
        return Err("CFF INDEX offsets must start at one".to_string());
    }
    let mut items = Vec::with_capacity(count);
    for index in 0..count {
        let start = data_start.saturating_add(offsets[index]);
        let end = data_start.saturating_add(offsets[index + 1]);
        if end < start || end > data.len() {
            return Err("CFF INDEX item exceeds table bounds".to_string());
        }
        items.push(data[start..end].to_vec());
    }
    Ok((items, data_start.saturating_add(offsets[count])))
}

/// Parse a CFF DICT (`parseCffDict`).
fn parse_cff_dict(bytes: &[u8]) -> Result<HashMap<u32, Vec<i32>>, String> {
    let mut dict = HashMap::new();
    let mut operands = Vec::new();
    let mut index = 0usize;
    while index < bytes.len() {
        let b0 = bytes[index];
        if b0 <= 21 {
            let op = if b0 == 12 {
                let Some(next) = bytes.get(index + 1) else {
                    return Err("CFF DICT exceeds table bounds".to_string());
                };
                index += 2;
                1200 + u32::from(*next)
            } else {
                index += 1;
                u32::from(b0)
            };
            dict.insert(op, std::mem::take(&mut operands));
        } else if b0 == 28 {
            if index + 3 > bytes.len() {
                return Err("CFF DICT exceeds table bounds".to_string());
            }
            operands.push(i16::from_be_bytes([bytes[index + 1], bytes[index + 2]]) as i32);
            index += 3;
        } else if b0 == 29 {
            if index + 5 > bytes.len() {
                return Err("CFF DICT exceeds table bounds".to_string());
            }
            operands.push(i32::from_be_bytes([
                bytes[index + 1],
                bytes[index + 2],
                bytes[index + 3],
                bytes[index + 4],
            ]));
            index += 5;
        } else if b0 == 30 {
            index += 1;
            let mut done = false;
            while !done && index < bytes.len() {
                let byte = bytes[index];
                index += 1;
                if byte >> 4 == 0xf || byte & 0xf == 0xf {
                    done = true;
                }
            }
            operands.push(0);
        } else if (32..=246).contains(&b0) {
            operands.push(i32::from(b0) - 139);
            index += 1;
        } else if (247..=250).contains(&b0) {
            let Some(next) = bytes.get(index + 1) else {
                return Err("CFF DICT exceeds table bounds".to_string());
            };
            operands.push((i32::from(b0) - 247) * 256 + i32::from(*next) + 108);
            index += 2;
        } else if (251..=254).contains(&b0) {
            let Some(next) = bytes.get(index + 1) else {
                return Err("CFF DICT exceeds table bounds".to_string());
            };
            operands.push(-(i32::from(b0) - 251) * 256 - i32::from(*next) - 108);
            index += 2;
        } else {
            index += 1;
        }
    }
    Ok(dict)
}

fn subr_bias(count: usize) -> usize {
    if count < 1240 {
        107
    } else if count < 33900 {
        1131
    } else {
        32768
    }
}

/// Private DICT plus local subroutines (`parsePrivateAndLocalSubrs`).
fn parse_private_and_local_subrs(cff: &[u8], private: Option<(usize, usize)>) -> Result<(Vec<Vec<u8>>, usize), String> {
    let Some((size, offset)) = private else {
        return Ok((Vec::new(), subr_bias(0)));
    };
    let start = offset.min(cff.len());
    let end = offset.saturating_add(size).min(cff.len());
    let dict = parse_cff_dict(&cff[start..end])?;
    let Some(rel) = dict.get(&19).and_then(|operands| operands.first().copied()) else {
        return Ok((Vec::new(), subr_bias(0)));
    };
    let position = offset as i64 + i64::from(rel);
    if position < 0 || position as usize > cff.len() {
        return Err("CFF INDEX exceeds table bounds".to_string());
    }
    let (items, _) = read_cff_index(cff, position as usize)?;
    let bias = subr_bias(items.len());
    Ok((items, bias))
}

/// Private DICT size/offset entry (`privateEntryOf`).
fn private_entry_of(dict: &HashMap<u32, Vec<i32>>) -> Option<(usize, usize)> {
    let entry = dict.get(&18)?;
    if entry.len() != 2 {
        return None;
    }
    Some((entry[0].max(0) as usize, entry[1].max(0) as usize))
}

/// Fill one FDSelect range, clamped to the glyph count.
fn fill_fd_range(result: &mut [u8], first: usize, end: usize, fd: u8) {
    let start = first.min(result.len());
    let end = end.min(result.len());
    if start < end {
        result[start..end].fill(fd);
    }
}

/// FDSelect table (`parseFdSelect`).
fn parse_fd_select(cff: &[u8], offset: usize, num_glyphs: usize) -> Result<Vec<u8>, String> {
    let mut result = vec![0u8; num_glyphs];
    let Some(format) = cff.get(offset).copied() else {
        return Err("CFF FDSelect exceeds table bounds".to_string());
    };
    if format == 0 {
        for (index, slot) in result.iter_mut().enumerate() {
            let Some(value) = cff.get(offset + 1 + index).copied() else {
                return Err("CFF FDSelect exceeds table bounds".to_string());
            };
            *slot = value;
        }
    } else if format == 3 {
        if offset + 3 > cff.len() {
            return Err("CFF FDSelect exceeds table bounds".to_string());
        }
        let num_ranges = u16::from_be_bytes([cff[offset + 1], cff[offset + 2]]) as usize;
        let mut cursor = offset + 3;
        let get_u16 = |at: usize| -> Result<usize, String> {
            if at + 2 > cff.len() {
                return Err("CFF FDSelect exceeds table bounds".to_string());
            }
            Ok(u16::from_be_bytes([cff[at], cff[at + 1]]) as usize)
        };
        let get_u8 = |at: usize| -> Result<u8, String> {
            cff.get(at)
                .copied()
                .ok_or_else(|| "CFF FDSelect exceeds table bounds".to_string())
        };
        let mut prev_first = get_u16(cursor)?;
        let mut prev_fd = get_u8(cursor + 2)?;
        cursor += 3;
        for _ in 1..num_ranges {
            let first = get_u16(cursor)?;
            let fd = get_u8(cursor + 2)?;
            fill_fd_range(&mut result, prev_first, first, prev_fd);
            prev_first = first;
            prev_fd = fd;
            cursor += 3;
        }
        let sentinel = get_u16(cursor)?;
        fill_fd_range(&mut result, prev_first, sentinel, prev_fd);
    }
    Ok(result)
}

/// Parse the CFF table (`parseCFF`).
fn parse_cff(cff: &[u8]) -> Result<Option<CffFont>, String> {
    if cff.len() < 4 {
        return Ok(None);
    }
    let mut position = usize::from(cff[2]);
    let (_, end) = read_cff_index(cff, position)?;
    position = end;
    let (top_items, end) = read_cff_index(cff, position)?;
    position = end;
    let (_, end) = read_cff_index(cff, position)?;
    position = end;
    let (global_subrs, _) = read_cff_index(cff, position)?;
    let Some(top) = top_items.first() else {
        return Ok(None);
    };
    let top_dict = parse_cff_dict(top)?;
    let Some(char_strings_off) = top_dict.get(&17).and_then(|operands| operands.first().copied()) else {
        return Ok(None);
    };
    if char_strings_off < 0 || char_strings_off as usize > cff.len() {
        return Err("CFF INDEX exceeds table bounds".to_string());
    }
    let (char_strings, _) = read_cff_index(cff, char_strings_off as usize)?;
    let is_cid = top_dict.contains_key(&1230);
    let global_bias = subr_bias(global_subrs.len());
    let (default_local_subrs, default_local_bias) = parse_private_and_local_subrs(cff, private_entry_of(&top_dict))?;
    let mut fd_local_subrs = Vec::new();
    let mut fd_local_bias = Vec::new();
    let mut fd_select = None;
    if is_cid {
        if let Some(fd_array_off) = top_dict.get(&1236).and_then(|operands| operands.first().copied()) {
            if fd_array_off < 0 || fd_array_off as usize > cff.len() {
                return Err("CFF INDEX exceeds table bounds".to_string());
            }
            let (fd_items, _) = read_cff_index(cff, fd_array_off as usize)?;
            for fd_bytes in &fd_items {
                let fd_dict = parse_cff_dict(fd_bytes)?;
                let (subrs, bias) = parse_private_and_local_subrs(cff, private_entry_of(&fd_dict))?;
                fd_local_subrs.push(subrs);
                fd_local_bias.push(bias);
            }
        }
        if let Some(fd_select_off) = top_dict.get(&1237).and_then(|operands| operands.first().copied()) {
            if fd_select_off < 0 || fd_select_off as usize > cff.len() {
                return Err("CFF INDEX exceeds table bounds".to_string());
            }
            fd_select = Some(parse_fd_select(cff, fd_select_off as usize, char_strings.len())?);
        }
    }
    Ok(Some(CffFont {
        char_strings,
        global_subrs,
        global_bias,
        is_cid,
        default_local_subrs,
        default_local_bias,
        fd_local_subrs,
        fd_local_bias,
        fd_select,
    }))
}

/// Type 2 charstring interpreter state (`Type2Context`).
struct Type2Context {
    x: f64,
    y: f64,
    contours: Vec<FlattenedContour>,
    current: Option<FlattenedContour>,
    stack: Vec<f64>,
    n_stems: usize,
    width_parsed: bool,
}

fn type2_move_to(ctx: &mut Type2Context, dx: f64, dy: f64) {
    if let Some(current) = ctx.current.take() {
        if !current.is_empty() {
            ctx.contours.push(current);
        }
    }
    ctx.x += dx;
    ctx.y += dy;
    ctx.current = Some(vec![ContourPoint { x: ctx.x, y: ctx.y }]);
}

fn type2_line_to(ctx: &mut Type2Context, dx: f64, dy: f64) {
    ctx.x += dx;
    ctx.y += dy;
    match ctx.current.as_mut() {
        Some(current) => current.push(ContourPoint { x: ctx.x, y: ctx.y }),
        None => ctx.current = Some(vec![ContourPoint { x: ctx.x, y: ctx.y }]),
    }
}

fn type2_emit_curve(ctx: &mut Type2Context, c1x: f64, c1y: f64, c2x: f64, c2y: f64, ex: f64, ey: f64) {
    if ctx.current.is_none() {
        ctx.current = Some(vec![ContourPoint { x: ctx.x, y: ctx.y }]);
    }
    let (p0x, p0y) = (ctx.x, ctx.y);
    const SEGMENTS: u32 = 8;
    for step in 1..=SEGMENTS {
        let t = f64::from(step) / f64::from(SEGMENTS);
        let mt = 1.0 - t;
        if let Some(current) = ctx.current.as_mut() {
            current.push(ContourPoint {
                x: mt * mt * mt * p0x + 3.0 * mt * mt * t * c1x + 3.0 * mt * t * t * c2x + t * t * t * ex,
                y: mt * mt * mt * p0y + 3.0 * mt * mt * t * c1y + 3.0 * mt * t * t * c2y + t * t * t * ey,
            });
        }
    }
    ctx.x = ex;
    ctx.y = ey;
}

fn type2_curve_to(ctx: &mut Type2Context, dx1: f64, dy1: f64, dx2: f64, dy2: f64, dx3: f64, dy3: f64) {
    let c1x = ctx.x + dx1;
    let c1y = ctx.y + dy1;
    let c2x = c1x + dx2;
    let c2y = c1y + dy2;
    type2_emit_curve(ctx, c1x, c1y, c2x, c2y, c2x + dx3, c2y + dy3);
}

fn type2_maybe_take_width(ctx: &mut Type2Context, has_extra: bool) {
    if ctx.width_parsed {
        return;
    }
    ctx.width_parsed = true;
    if has_extra && !ctx.stack.is_empty() {
        ctx.stack.remove(0);
    }
}

fn type2_operand(stack: &[f64], index: usize, source: &str) -> Result<f64, ClientError> {
    stack
        .get(index)
        .copied()
        .ok_or_else(|| bad_font(source, "CFF charstring stack underflow"))
}

fn type2_hflex(ctx: &mut Type2Context, stack: &[f64], source: &str) -> Result<(), ClientError> {
    let dx1 = type2_operand(stack, 0, source)?;
    let dx2 = type2_operand(stack, 1, source)?;
    let dy2 = type2_operand(stack, 2, source)?;
    let dx3 = type2_operand(stack, 3, source)?;
    let dx4 = type2_operand(stack, 4, source)?;
    let dx5 = type2_operand(stack, 5, source)?;
    let dx6 = type2_operand(stack, 6, source)?;
    let (x0, y0) = (ctx.x, ctx.y);
    let (c1x, c1y) = (x0 + dx1, y0);
    let (c2x, c2y) = (c1x + dx2, c1y + dy2);
    let p1x = c2x + dx3;
    type2_emit_curve(ctx, c1x, c1y, c2x, c2y, p1x, c2y);
    let (c3x, c3y) = (p1x + dx4, c2y);
    let c4x = c3x + dx5;
    type2_emit_curve(ctx, c3x, c3y, c4x, c3y, c4x + dx6, y0);
    Ok(())
}

fn type2_flex(ctx: &mut Type2Context, stack: &[f64], source: &str) -> Result<(), ClientError> {
    let mut args = [0.0; 12];
    for (index, slot) in args.iter_mut().enumerate() {
        *slot = type2_operand(stack, index, source)?;
    }
    type2_curve_to(ctx, args[0], args[1], args[2], args[3], args[4], args[5]);
    type2_curve_to(ctx, args[6], args[7], args[8], args[9], args[10], args[11]);
    Ok(())
}

fn type2_hflex1(ctx: &mut Type2Context, stack: &[f64], source: &str) -> Result<(), ClientError> {
    let mut args = [0.0; 9];
    for (index, slot) in args.iter_mut().enumerate() {
        *slot = type2_operand(stack, index, source)?;
    }
    let (x0, y0) = (ctx.x, ctx.y);
    let (c1x, c1y) = (x0 + args[0], y0 + args[1]);
    let (c2x, c2y) = (c1x + args[2], c1y + args[3]);
    let p1x = c2x + args[4];
    type2_emit_curve(ctx, c1x, c1y, c2x, c2y, p1x, c2y);
    let (c3x, c3y) = (p1x + args[5], c2y);
    let (c4x, c4y) = (c3x + args[6], c3y + args[7]);
    type2_emit_curve(ctx, c3x, c3y, c4x, c4y, c4x + args[8], y0);
    Ok(())
}

fn type2_flex1(ctx: &mut Type2Context, stack: &[f64], source: &str) -> Result<(), ClientError> {
    let mut args = [0.0; 11];
    for (index, slot) in args.iter_mut().enumerate() {
        *slot = type2_operand(stack, index, source)?;
    }
    let (x0, y0) = (ctx.x, ctx.y);
    let (c1x, c1y) = (x0 + args[0], y0 + args[1]);
    let (c2x, c2y) = (c1x + args[2], c1y + args[3]);
    let (p1x, p1y) = (c2x + args[4], c2y + args[5]);
    type2_emit_curve(ctx, c1x, c1y, c2x, c2y, p1x, p1y);
    let (c3x, c3y) = (p1x + args[6], p1y + args[7]);
    let (c4x, c4y) = (c3x + args[8], c3y + args[9]);
    let sum_dx = args[0] + args[2] + args[4] + args[6] + args[8];
    let sum_dy = args[1] + args[3] + args[5] + args[7] + args[9];
    if sum_dx.abs() > sum_dy.abs() {
        type2_emit_curve(ctx, c3x, c3y, c4x, c4y, c4x + args[10], y0);
    } else {
        type2_emit_curve(ctx, c3x, c3y, c4x, c4y, x0, c4y + args[10]);
    }
    Ok(())
}

fn type2_escape(ctx: &mut Type2Context, op: u8, source: &str) -> Result<(), ClientError> {
    let stack = std::mem::take(&mut ctx.stack);
    let result = match op {
        34 => type2_hflex(ctx, &stack, source),
        35 => type2_flex(ctx, &stack, source),
        36 => type2_hflex1(ctx, &stack, source),
        37 => type2_flex1(ctx, &stack, source),
        _ => Ok(()),
    };
    ctx.stack.clear();
    result
}

/// Execute one charstring (`execCharstring`).
#[allow(clippy::too_many_arguments)]
fn exec_charstring(
    code: &[u8],
    ctx: &mut Type2Context,
    global_subrs: &[Vec<u8>],
    global_bias: usize,
    local_subrs: &[Vec<u8>],
    local_bias: usize,
    depth: u32,
    source: &str,
) -> Result<(), ClientError> {
    if depth > 10 {
        return Ok(());
    }
    let mut index = 0usize;
    while index < code.len() {
        let b0 = code[index];
        if b0 >= 32 || b0 == 28 {
            let value = if b0 == 28 {
                if index + 3 > code.len() {
                    return Err(bad_font(source, "CFF charstring exceeds table bounds"));
                }
                index += 3;
                f64::from(i16::from_be_bytes([code[index - 2], code[index - 1]]))
            } else if b0 <= 246 {
                index += 1;
                f64::from(b0) - 139.0
            } else if b0 <= 250 {
                if index + 2 > code.len() {
                    return Err(bad_font(source, "CFF charstring exceeds table bounds"));
                }
                index += 2;
                f64::from(b0 - 247) * 256.0 + f64::from(code[index - 1]) + 108.0
            } else if b0 <= 254 {
                if index + 2 > code.len() {
                    return Err(bad_font(source, "CFF charstring exceeds table bounds"));
                }
                index += 2;
                -f64::from(b0 - 251) * 256.0 - f64::from(code[index - 1]) - 108.0
            } else {
                if index + 5 > code.len() {
                    return Err(bad_font(source, "CFF charstring exceeds table bounds"));
                }
                index += 5;
                f64::from(i32::from_be_bytes([
                    code[index - 4],
                    code[index - 3],
                    code[index - 2],
                    code[index - 1],
                ])) / 65536.0
            };
            ctx.stack.push(value);
            continue;
        }
        index += 1;
        match b0 {
            1 | 3 | 18 | 23 => {
                let odd = ctx.stack.len() % 2 == 1;
                type2_maybe_take_width(ctx, odd);
                ctx.n_stems += ctx.stack.len() / 2;
                ctx.stack.clear();
            }
            19 | 20 => {
                let odd = ctx.stack.len() % 2 == 1;
                type2_maybe_take_width(ctx, odd);
                ctx.n_stems += ctx.stack.len() / 2;
                ctx.stack.clear();
                index = index.saturating_add((ctx.n_stems + 7) >> 3);
            }
            21 => {
                let extra = ctx.stack.len() > 2;
                type2_maybe_take_width(ctx, extra);
                let dx = type2_operand(&ctx.stack, 0, source)?;
                let dy = type2_operand(&ctx.stack, 1, source)?;
                type2_move_to(ctx, dx, dy);
                ctx.stack.clear();
            }
            22 => {
                let extra = ctx.stack.len() > 1;
                type2_maybe_take_width(ctx, extra);
                let dx = type2_operand(&ctx.stack, 0, source)?;
                type2_move_to(ctx, dx, 0.0);
                ctx.stack.clear();
            }
            4 => {
                let extra = ctx.stack.len() > 1;
                type2_maybe_take_width(ctx, extra);
                let dy = type2_operand(&ctx.stack, 0, source)?;
                type2_move_to(ctx, 0.0, dy);
                ctx.stack.clear();
            }
            5 => {
                let stack = std::mem::take(&mut ctx.stack);
                let mut cursor = 0usize;
                while cursor + 1 < stack.len() {
                    type2_line_to(ctx, stack[cursor], stack[cursor + 1]);
                    cursor += 2;
                }
            }
            6 => {
                let stack = std::mem::take(&mut ctx.stack);
                let mut horizontal = true;
                for value in &stack {
                    if horizontal {
                        type2_line_to(ctx, *value, 0.0);
                    } else {
                        type2_line_to(ctx, 0.0, *value);
                    }
                    horizontal = !horizontal;
                }
            }
            7 => {
                let stack = std::mem::take(&mut ctx.stack);
                let mut horizontal = false;
                for value in &stack {
                    if horizontal {
                        type2_line_to(ctx, *value, 0.0);
                    } else {
                        type2_line_to(ctx, 0.0, *value);
                    }
                    horizontal = !horizontal;
                }
            }
            8 => {
                let stack = std::mem::take(&mut ctx.stack);
                let mut cursor = 0usize;
                while cursor + 5 < stack.len() {
                    type2_curve_to(
                        ctx,
                        stack[cursor],
                        stack[cursor + 1],
                        stack[cursor + 2],
                        stack[cursor + 3],
                        stack[cursor + 4],
                        stack[cursor + 5],
                    );
                    cursor += 6;
                }
            }
            24 => {
                let stack = std::mem::take(&mut ctx.stack);
                let mut cursor = 0usize;
                while cursor + 5 < stack.len().saturating_sub(2) {
                    type2_curve_to(
                        ctx,
                        stack[cursor],
                        stack[cursor + 1],
                        stack[cursor + 2],
                        stack[cursor + 3],
                        stack[cursor + 4],
                        stack[cursor + 5],
                    );
                    cursor += 6;
                }
                let dx = type2_operand(&stack, cursor, source)?;
                let dy = type2_operand(&stack, cursor + 1, source)?;
                type2_line_to(ctx, dx, dy);
            }
            25 => {
                let stack = std::mem::take(&mut ctx.stack);
                let mut cursor = 0usize;
                while cursor + 1 < stack.len().saturating_sub(6) {
                    type2_line_to(ctx, stack[cursor], stack[cursor + 1]);
                    cursor += 2;
                }
                let mut args = [0.0; 6];
                for (offset, slot) in args.iter_mut().enumerate() {
                    *slot = type2_operand(&stack, cursor + offset, source)?;
                }
                type2_curve_to(ctx, args[0], args[1], args[2], args[3], args[4], args[5]);
            }
            26 => {
                let stack = std::mem::take(&mut ctx.stack);
                let mut cursor = 0usize;
                let mut dx1 = 0.0;
                if stack.len() % 4 == 1 {
                    dx1 = type2_operand(&stack, 0, source)?;
                    cursor = 1;
                }
                while cursor + 3 < stack.len() {
                    let c1x = ctx.x + dx1;
                    let c1y = ctx.y + stack[cursor];
                    let c2x = c1x + stack[cursor + 1];
                    let c2y = c1y + stack[cursor + 2];
                    type2_emit_curve(ctx, c1x, c1y, c2x, c2y, c2x, c2y + stack[cursor + 3]);
                    dx1 = 0.0;
                    cursor += 4;
                }
            }
            27 => {
                let stack = std::mem::take(&mut ctx.stack);
                let mut cursor = 0usize;
                let mut dy1 = 0.0;
                if stack.len() % 4 == 1 {
                    dy1 = type2_operand(&stack, 0, source)?;
                    cursor = 1;
                }
                while cursor + 3 < stack.len() {
                    let c1x = ctx.x + stack[cursor];
                    let c1y = ctx.y + dy1;
                    let c2x = c1x + stack[cursor + 1];
                    let c2y = c1y + stack[cursor + 2];
                    type2_emit_curve(ctx, c1x, c1y, c2x, c2y, c2x + stack[cursor + 3], c2y);
                    dy1 = 0.0;
                    cursor += 4;
                }
            }
            30 | 31 => {
                let stack = std::mem::take(&mut ctx.stack);
                let mut horizontal = b0 == 31;
                let mut cursor = 0usize;
                while cursor + 3 < stack.len() {
                    let has_extra = cursor + 5 == stack.len();
                    if horizontal {
                        let c1x = ctx.x + stack[cursor];
                        let c1y = ctx.y;
                        let c2x = c1x + stack[cursor + 1];
                        let c2y = c1y + stack[cursor + 2];
                        let ey = c2y + stack[cursor + 3];
                        let ex = if has_extra { c2x + stack[cursor + 4] } else { c2x };
                        type2_emit_curve(ctx, c1x, c1y, c2x, c2y, ex, ey);
                    } else {
                        let c1x = ctx.x;
                        let c1y = ctx.y + stack[cursor];
                        let c2x = c1x + stack[cursor + 1];
                        let c2y = c1y + stack[cursor + 2];
                        let ex = c2x + stack[cursor + 3];
                        let ey = if has_extra { c2y + stack[cursor + 4] } else { c2y };
                        type2_emit_curve(ctx, c1x, c1y, c2x, c2y, ex, ey);
                    }
                    horizontal = !horizontal;
                    cursor += 4;
                }
            }
            10 => {
                let operand = ctx
                    .stack
                    .pop()
                    .ok_or_else(|| bad_font(source, "CFF callsubr has no operand"))?;
                let sub = resolve_subr(local_subrs, local_bias, operand, source, "CFF local subroutine")?;
                exec_charstring(
                    sub,
                    ctx,
                    global_subrs,
                    global_bias,
                    local_subrs,
                    local_bias,
                    depth + 1,
                    source,
                )?;
            }
            29 => {
                let operand = ctx
                    .stack
                    .pop()
                    .ok_or_else(|| bad_font(source, "CFF callgsubr has no operand"))?;
                let sub = resolve_subr(global_subrs, global_bias, operand, source, "CFF global subroutine")?;
                exec_charstring(
                    sub,
                    ctx,
                    global_subrs,
                    global_bias,
                    local_subrs,
                    local_bias,
                    depth + 1,
                    source,
                )?;
            }
            11 => return Ok(()),
            14 => {
                let extra = ctx.stack.len() == 1 || ctx.stack.len() == 5;
                type2_maybe_take_width(ctx, extra);
                return Ok(());
            }
            12 => {
                let Some(op) = code.get(index).copied() else {
                    return Err(bad_font(source, "CFF charstring exceeds table bounds"));
                };
                index += 1;
                type2_escape(ctx, op, source)?;
            }
            _ => {
                ctx.stack.clear();
            }
        }
    }
    Ok(())
}

fn resolve_subr<'a>(
    subrs: &'a [Vec<u8>],
    bias: usize,
    operand: f64,
    source: &str,
    what: &str,
) -> Result<&'a [u8], ClientError> {
    let index = operand + bias as f64;
    if !index.is_finite() || index < 0.0 || index.fract() != 0.0 {
        return Err(bad_font(source, &format!("{what} is missing")));
    }
    subrs
        .get(index as usize)
        .map(Vec::as_slice)
        .ok_or_else(|| bad_font(source, &format!("{what} is missing")))
}

/// CFF contours for a glyph id (`getCffContours`).
fn cff_contours(source: &str, cff: &CffFont, gid: u32) -> Result<Vec<FlattenedContour>, ClientError> {
    let Some(code) = cff.char_strings.get(gid as usize) else {
        return Ok(Vec::new());
    };
    let (mut local_subrs, mut local_bias) = (cff.default_local_subrs.as_slice(), cff.default_local_bias);
    if cff.is_cid {
        if let Some(select) = cff.fd_select.as_ref() {
            let fd = usize::from(
                select
                    .get(gid as usize)
                    .copied()
                    .ok_or_else(|| bad_font(source, "CFF glyph FD selection is missing"))?,
            );
            local_subrs = cff
                .fd_local_subrs
                .get(fd)
                .ok_or_else(|| bad_font(source, "CFF font dictionary is missing"))?;
            local_bias = *cff
                .fd_local_bias
                .get(fd)
                .ok_or_else(|| bad_font(source, "CFF local subroutine bias is missing"))?;
        }
    }
    let mut ctx = Type2Context {
        x: 0.0,
        y: 0.0,
        contours: Vec::new(),
        current: None,
        stack: Vec::new(),
        n_stems: 0,
        width_parsed: false,
    };
    exec_charstring(
        code,
        &mut ctx,
        &cff.global_subrs,
        cff.global_bias,
        local_subrs,
        local_bias,
        0,
        source,
    )?;
    if let Some(current) = ctx.current.take() {
        if !current.is_empty() {
            ctx.contours.push(current);
        }
    }
    Ok(ctx.contours)
}

/// A fill edge (`FillEdge`).
#[derive(Debug, Clone, Copy)]
struct FillEdge {
    x0: f64,
    y0: f64,
    x1: f64,
    y1: f64,
    dir: i32,
}

/// Accumulate one horizontal span (`accumulateSpan`).
fn accumulate_span(coverage: &mut [f32], row: usize, width: usize, xa: f64, xb: f64, weight: f64) {
    let mut start = xa;
    let mut end = xb;
    if end <= start {
        return;
    }
    start = start.max(0.0);
    end = end.min(width as f64);
    if end <= start {
        return;
    }
    let row_base = row * width;
    let first_px = start.floor() as usize;
    let last_px = (end - 1e-9).floor() as usize;
    if first_px == last_px {
        coverage[row_base + first_px] += ((end - start) * weight) as f32;
        return;
    }
    coverage[row_base + first_px] += ((first_px as f64 + 1.0 - start) * weight) as f32;
    for px in first_px + 1..last_px {
        coverage[row_base + px] += weight as f32;
    }
    coverage[row_base + last_px] += ((end - last_px as f64) * weight) as f32;
}

/// Rasterize pixel-space contours (`rasterizeContours`).
pub fn rasterize_contours(
    contours_px: &[FlattenedContour],
    width: u32,
    height: u32,
    supersample_y: u32,
) -> Result<RasterizedGlyph, ClientError> {
    let width_usize = width as usize;
    let height_usize = height as usize;
    let len = width_usize
        .checked_mul(height_usize)
        .ok_or_else(|| ClientError::BadFont("Font atlas exceeds allocation bounds".to_string()))?;
    let mut coverage = vec![0.0f32; len];
    let mut edges = Vec::new();
    for contour in contours_px {
        let count = contour.len();
        if count < 2 {
            continue;
        }
        for index in 0..count {
            let p0 = &contour[index];
            let p1 = &contour[(index + 1) % count];
            if p0.y == p1.y {
                continue;
            }
            if p0.y < p1.y {
                edges.push(FillEdge {
                    x0: p0.x,
                    y0: p0.y,
                    x1: p1.x,
                    y1: p1.y,
                    dir: 1,
                });
            } else {
                edges.push(FillEdge {
                    x0: p1.x,
                    y0: p1.y,
                    x1: p0.x,
                    y1: p0.y,
                    dir: -1,
                });
            }
        }
    }
    let samples = supersample_y.max(1);
    let inv = 1.0 / f64::from(samples);
    for row in 0..height_usize {
        for sample in 0..samples {
            let sy = row as f64 + (f64::from(sample) + 0.5) * inv;
            let mut crossings: Vec<(f64, i32)> = Vec::new();
            for edge in &edges {
                if sy >= edge.y0 && sy < edge.y1 {
                    let t = (sy - edge.y0) / (edge.y1 - edge.y0);
                    crossings.push((edge.x0 + t * (edge.x1 - edge.x0), edge.dir));
                }
            }
            if crossings.len() < 2 {
                continue;
            }
            crossings.sort_by(|a, b| a.0.total_cmp(&b.0));
            let mut winding = 0i32;
            for pair in crossings.windows(2) {
                winding += pair[0].1;
                if winding != 0 {
                    accumulate_span(&mut coverage, row, width_usize, pair[0].0, pair[1].0, inv);
                }
            }
        }
    }
    let mut out = vec![0u8; len];
    for (index, slot) in out.iter_mut().enumerate() {
        let value = coverage[index];
        *slot = if value <= 0.0 {
            0
        } else if value >= 1.0 {
            255
        } else {
            (f64::from(value) * 255.0).round() as u8
        };
    }
    Ok(RasterizedGlyph {
        width,
        height,
        coverage: out,
    })
}

/// Rasterize one glyph (`rasterizeGlyph`).
pub fn rasterize_glyph(font: &ParsedFont, gid: u32, px: u32) -> Result<RasterizedGlyph, ClientError> {
    let metrics = font.metrics();
    let scale = f64::from(px) / f64::from(metrics.units_per_em);
    let width = (f64::from(font.advance_width(gid)) * scale).round().max(1.0) as u32;
    let contours = font.contours(gid)?;
    let mut y_min = 0.0;
    for contour in &contours {
        for point in contour {
            if point.y < y_min {
                y_min = point.y;
            }
        }
    }
    let bottom = y_min.min(0.0);
    let height = ((f64::from(metrics.ascent) - bottom) * scale).round().max(1.0) as u32;
    if contours.is_empty() {
        let len = (width as usize)
            .checked_mul(height as usize)
            .ok_or_else(|| ClientError::BadFont("Font atlas exceeds allocation bounds".to_string()))?;
        return Ok(RasterizedGlyph {
            width,
            height,
            coverage: vec![0u8; len],
        });
    }
    let px_contours: Vec<FlattenedContour> = contours
        .iter()
        .map(|contour| {
            contour
                .iter()
                .map(|point| ContourPoint {
                    x: point.x * scale,
                    y: (f64::from(metrics.ascent) - point.y) * scale,
                })
                .collect()
        })
        .collect();
    rasterize_contours(&px_contours, width, height, 4)
}

/// Composite one layer texel (`compositeOver`).
fn composite_over(pixels: &mut [u8], index: usize, src_coverage: u8, color: PaletteColor) {
    let src_a = (f64::from(src_coverage) / 255.0) * (f64::from(color.a) / 255.0);
    if src_a <= 0.0 {
        return;
    }
    let base = index * 4;
    let dst_a = f64::from(pixels[base + 3]) / 255.0;
    let out_a = src_a + dst_a * (1.0 - src_a);
    if out_a <= 0.0 {
        return;
    }
    let dst_factor = dst_a * (1.0 - src_a);
    pixels[base] = ((f64::from(color.r) * src_a + f64::from(pixels[base]) * dst_factor) / out_a).round() as u8;
    pixels[base + 1] = ((f64::from(color.g) * src_a + f64::from(pixels[base + 1]) * dst_factor) / out_a).round() as u8;
    pixels[base + 2] = ((f64::from(color.b) * src_a + f64::from(pixels[base + 2]) * dst_factor) / out_a).round() as u8;
    pixels[base + 3] = (out_a * 255.0).round() as u8;
}

/// Rasterize one color glyph (`rasterizeColorGlyph`).
pub fn rasterize_color_glyph(
    font: &ParsedFont,
    base_gid: u32,
    px: u32,
    palette_id: u32,
    foreground: PaletteColor,
) -> Result<Option<RasterizedColorGlyph>, ClientError> {
    let Some(layers) = font.color_layers(base_gid) else {
        return Ok(None);
    };
    let metrics = font.metrics();
    let scale = f64::from(px) / f64::from(metrics.units_per_em);
    let width = (f64::from(font.advance_width(base_gid)) * scale).round().max(1.0) as u32;
    let mut resolved: Vec<(Vec<FlattenedContour>, PaletteColor)> = Vec::new();
    let mut y_min = 0.0;
    for layer in &layers {
        let color = if layer.palette_index == COLR_FOREGROUND_SENTINEL {
            Some(foreground)
        } else {
            font.palette_color(u32::from(layer.palette_index), palette_id)
        };
        let Some(color) = color else {
            continue;
        };
        let contours = font.contours(layer.gid)?;
        for contour in &contours {
            for point in contour {
                if point.y < y_min {
                    y_min = point.y;
                }
            }
        }
        resolved.push((contours, color));
    }
    let bottom = y_min.min(0.0);
    let height = ((f64::from(metrics.ascent) - bottom) * scale).round().max(1.0) as u32;
    let len = (width as usize)
        .checked_mul(height as usize)
        .and_then(|cells| cells.checked_mul(4))
        .ok_or_else(|| ClientError::BadFont("Font atlas exceeds allocation bounds".to_string()))?;
    let mut pixels = vec![0u8; len];
    for (contours, color) in &resolved {
        if contours.is_empty() {
            continue;
        }
        let px_contours: Vec<FlattenedContour> = contours
            .iter()
            .map(|contour| {
                contour
                    .iter()
                    .map(|point| ContourPoint {
                        x: point.x * scale,
                        y: (f64::from(metrics.ascent) - point.y) * scale,
                    })
                    .collect()
            })
            .collect();
        let raster = rasterize_contours(&px_contours, width, height, 4)?;
        for (index, coverage) in raster.coverage.iter().enumerate() {
            if *coverage == 0 {
                continue;
            }
            composite_over(&mut pixels, index, *coverage, *color);
        }
    }
    Ok(Some(RasterizedColorGlyph { width, height, pixels }))
}

enum AtlasCell {
    Mono {
        codepoint: u32,
        width: u32,
        height: u32,
        coverage: Vec<u8>,
    },
    Color {
        codepoint: u32,
        width: u32,
        height: u32,
        rgba: Vec<u8>,
    },
}

impl AtlasCell {
    const fn codepoint(&self) -> u32 {
        match self {
            Self::Mono { codepoint, .. } | Self::Color { codepoint, .. } => *codepoint,
        }
    }

    const fn width(&self) -> u32 {
        match self {
            Self::Mono { width, .. } | Self::Color { width, .. } => *width,
        }
    }

    const fn height(&self) -> u32 {
        match self {
            Self::Mono { height, .. } | Self::Color { height, .. } => *height,
        }
    }
}

/// Build a packed atlas (`buildFontAtlas`).
pub fn build_font_atlas(
    font: &ParsedFont,
    codepoints: &[u32],
    px: u32,
    max_width: u32,
) -> Result<FontAtlas, ClientError> {
    if px == 0 || max_width < 1 {
        return Err(ClientError::BadFont(
            "Font atlas pixel size and width must be positive".to_string(),
        ));
    }
    let mut cells = Vec::new();
    let mut line_height = 0u32;
    for codepoint in codepoints {
        let gid = font.cmap_lookup(*codepoint);
        if gid == 0 {
            continue;
        }
        if font.color_layers(gid).is_some() {
            let Some(raster) = rasterize_color_glyph(font, gid, px, 0, PaletteColor::WHITE)? else {
                continue;
            };
            line_height = line_height.max(raster.height);
            cells.push(AtlasCell::Color {
                codepoint: *codepoint,
                width: raster.width,
                height: raster.height,
                rgba: raster.pixels,
            });
            continue;
        }
        let raster = rasterize_glyph(font, gid, px)?;
        line_height = line_height.max(raster.height);
        cells.push(AtlasCell::Mono {
            codepoint: *codepoint,
            width: raster.width,
            height: raster.height,
            coverage: raster.coverage,
        });
    }
    let mut sorted: Vec<&AtlasCell> = cells.iter().collect();
    sorted.sort_by_key(|cell| std::cmp::Reverse(cell.height()));
    const GAP: u32 = 1;
    let mut atlas_width = max_width;
    for cell in &cells {
        atlas_width = atlas_width.max(cell.width() + GAP * 2);
    }
    let mut cursor_x = GAP;
    let mut cursor_y = GAP;
    let mut shelf_height = 0u32;
    let mut placements: HashMap<u32, (u32, u32)> = HashMap::new();
    for cell in sorted {
        if cursor_x + cell.width() + GAP > atlas_width {
            cursor_x = GAP;
            cursor_y += shelf_height + GAP;
            shelf_height = 0;
        }
        placements.insert(cell.codepoint(), (cursor_x, cursor_y));
        cursor_x += cell.width() + GAP;
        shelf_height = shelf_height.max(cell.height());
    }
    let atlas_height = cursor_y + shelf_height + GAP;
    let len = (atlas_width as usize)
        .checked_mul(atlas_height as usize)
        .and_then(|cells| cells.checked_mul(4))
        .ok_or_else(|| ClientError::BadFont("Font atlas exceeds allocation bounds".to_string()))?;
    let mut pixels = vec![0u8; len];
    let mut glyphs = BTreeMap::new();
    for cell in &cells {
        let Some((x, y)) = placements.get(&cell.codepoint()).copied() else {
            continue;
        };
        match cell {
            AtlasCell::Color {
                width, height, rgba, ..
            } => {
                for row in 0..*height {
                    for col in 0..*width {
                        let src = (row * width + col) as usize * 4;
                        let dst = ((y + row) * atlas_width + (x + col)) as usize * 4;
                        pixels[dst..dst + 4].copy_from_slice(&rgba[src..src + 4]);
                    }
                }
            }
            AtlasCell::Mono {
                width,
                height,
                coverage,
                ..
            } => {
                for row in 0..*height {
                    for col in 0..*width {
                        let alpha = coverage[(row * width + col) as usize];
                        let dst = ((y + row) * atlas_width + (x + col)) as usize * 4;
                        pixels[dst..dst + 4].copy_from_slice(&[255, 255, 255, alpha]);
                    }
                }
            }
        }
        glyphs.insert(
            cell.codepoint(),
            AtlasRect {
                x,
                y,
                width: cell.width(),
                height: cell.height(),
                color: matches!(cell, AtlasCell::Color { .. }),
            },
        );
    }
    Ok(FontAtlas {
        width: atlas_width,
        height: atlas_height,
        pixels,
        glyphs,
        line_height,
    })
}

/// Latin-1 coverage (`latin1Codepoints`).
#[must_use]
pub fn latin1_codepoints() -> Vec<u32> {
    (0x20..=0x7e).chain(0xa0..=0xff).collect()
}

/// Validate a coverage scalar (`loadTrueType` coverage check).
pub fn validate_codepoint(code: u32) -> Result<(), ClientError> {
    if code > 0x10ffff || (0xd800..=0xdfff).contains(&code) {
        return Err(ClientError::BadFont(
            "Font coverage must contain Unicode scalar values".to_string(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let mut blobs = Vec::new();
        for (tag, data) in tables {
            let bytes = tag.as_bytes();
            font.extend_from_slice(&[bytes[0], bytes[1], bytes[2], bytes[3]]);
            put_u32(&mut font, 0);
            put_u32(&mut font, offset as u32);
            put_u32(&mut font, data.len() as u32);
            blobs.push(data.clone());
            offset += data.len();
            while !offset.is_multiple_of(4) {
                offset += 1;
            }
        }
        for blob in &blobs {
            font.extend_from_slice(blob);
            while !font.len().is_multiple_of(4) {
                font.push(0);
            }
        }
        font
    }

    fn head_table(units_per_em: u16) -> Vec<u8> {
        let mut table = Vec::new();
        put_u32(&mut table, 0x0001_0000);
        put_u32(&mut table, 0);
        put_u32(&mut table, 0);
        put_u32(&mut table, 0x5f0f_3cf5);
        put_u16(&mut table, 0);
        put_u16(&mut table, units_per_em);
        table.extend_from_slice(&[0u8; 16]);
        for _ in 0..4 {
            put_i16(&mut table, 0);
        }
        put_u16(&mut table, 0);
        put_u16(&mut table, 0);
        put_i16(&mut table, 0);
        put_i16(&mut table, 1);
        put_i16(&mut table, 0);
        table
    }

    fn hhea_table(num_h_metrics: u16) -> Vec<u8> {
        let mut table = Vec::new();
        put_u32(&mut table, 0x0001_0000);
        put_i16(&mut table, 800);
        put_i16(&mut table, -200);
        put_i16(&mut table, 0);
        put_u16(&mut table, 600);
        put_i16(&mut table, 0);
        put_i16(&mut table, 0);
        put_i16(&mut table, 0);
        put_i16(&mut table, 1);
        put_i16(&mut table, 0);
        put_i16(&mut table, 0);
        table.extend_from_slice(&[0u8; 8]);
        put_i16(&mut table, 0);
        put_u16(&mut table, num_h_metrics);
        table
    }

    fn maxp_table(num_glyphs: u16) -> Vec<u8> {
        let mut table = Vec::new();
        put_u32(&mut table, 0x0001_0000);
        put_u16(&mut table, num_glyphs);
        table
    }

    fn hmtx_table(advances: &[u16]) -> Vec<u8> {
        let mut table = Vec::new();
        for advance in advances {
            put_u16(&mut table, *advance);
            put_i16(&mut table, 0);
        }
        table
    }

    fn cmap_format4(segments: &[(u16, u16, i16)]) -> Vec<u8> {
        let mut segs: Vec<(u16, u16, i16)> = segments.to_vec();
        segs.push((0xffff, 0xffff, 1));
        let count = segs.len();
        let mut entry_selector = 0u16;
        while (1usize << (entry_selector + 1)) <= count {
            entry_selector += 1;
        }
        let search_range = 2 * (1 << entry_selector);
        let mut subtable = Vec::new();
        put_u16(&mut subtable, 4);
        put_u16(&mut subtable, (16 + 8 * count) as u16);
        put_u16(&mut subtable, 0);
        put_u16(&mut subtable, (2 * count) as u16);
        put_u16(&mut subtable, search_range);
        put_u16(&mut subtable, entry_selector);
        put_u16(&mut subtable, (2 * count) as u16 - search_range);
        for (_, end, _) in &segs {
            put_u16(&mut subtable, *end);
        }
        put_u16(&mut subtable, 0);
        for (start, _, _) in &segs {
            put_u16(&mut subtable, *start);
        }
        for (_, _, delta) in &segs {
            put_i16(&mut subtable, *delta);
        }
        for _ in &segs {
            put_u16(&mut subtable, 0);
        }
        let mut table = Vec::new();
        put_u16(&mut table, 0);
        put_u16(&mut table, 1);
        put_u16(&mut table, 3);
        put_u16(&mut table, 1);
        put_u32(&mut table, 12);
        table.extend_from_slice(&subtable);
        table
    }

    fn cmap_format12(groups: &[(u32, u32, u32)]) -> Vec<u8> {
        let mut subtable = Vec::new();
        put_u16(&mut subtable, 12);
        put_u16(&mut subtable, 0);
        put_u32(&mut subtable, (16 + 12 * groups.len()) as u32);
        put_u32(&mut subtable, 0);
        put_u32(&mut subtable, groups.len() as u32);
        for (start, end, glyph) in groups {
            put_u32(&mut subtable, *start);
            put_u32(&mut subtable, *end);
            put_u32(&mut subtable, *glyph);
        }
        let mut table = Vec::new();
        put_u16(&mut table, 0);
        put_u16(&mut table, 1);
        put_u16(&mut table, 3);
        put_u16(&mut table, 10);
        put_u32(&mut table, 12);
        table.extend_from_slice(&subtable);
        table
    }

    fn square_glyph() -> Vec<u8> {
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
        glyph
    }

    fn composite_glyph() -> Vec<u8> {
        let mut glyph = Vec::new();
        put_i16(&mut glyph, -1);
        put_i16(&mut glyph, 100);
        put_i16(&mut glyph, 50);
        put_i16(&mut glyph, 600);
        put_i16(&mut glyph, 750);
        put_u16(&mut glyph, 0x0003);
        put_u16(&mut glyph, 1);
        put_i16(&mut glyph, 100);
        put_i16(&mut glyph, 50);
        glyph
    }

    fn loca_table(offsets: &[u32]) -> Vec<u8> {
        let mut table = Vec::new();
        for offset in offsets {
            put_u32(&mut table, *offset);
        }
        table
    }

    fn synthetic_ttf() -> Vec<u8> {
        let square = square_glyph();
        assemble(&[
            ("head", head_table(1000)),
            ("hhea", hhea_table(2)),
            ("maxp", maxp_table(2)),
            ("hmtx", hmtx_table(&[600, 600])),
            ("cmap", cmap_format4(&[(63, 63, -62), (65, 65, -64)])),
            ("loca", loca_table(&[0, 0, square.len() as u32])),
            ("glyf", square),
        ])
    }

    fn synthetic_ttf_composite() -> Vec<u8> {
        let mut glyf = square_glyph();
        let composite = composite_glyph();
        let square_len = glyf.len() as u32;
        glyf.extend_from_slice(&composite);
        let total = glyf.len() as u32;
        assemble(&[
            ("head", head_table(1000)),
            ("hhea", hhea_table(3)),
            ("maxp", maxp_table(3)),
            ("hmtx", hmtx_table(&[600, 600, 600])),
            ("cmap", cmap_format4(&[(63, 63, -62), (65, 65, -64), (66, 66, -64)])),
            ("loca", loca_table(&[0, 0, square_len, total])),
            ("glyf", glyf),
        ])
    }

    fn synthetic_ttf_format12() -> Vec<u8> {
        let square = square_glyph();
        assemble(&[
            ("head", head_table(1000)),
            ("hhea", hhea_table(2)),
            ("maxp", maxp_table(2)),
            ("hmtx", hmtx_table(&[600, 600])),
            ("cmap", cmap_format12(&[(0x1f_600, 0x1f_600, 1)])),
            ("loca", loca_table(&[0, 0, square.len() as u32])),
            ("glyf", square),
        ])
    }

    #[test]
    fn parses_synthetic_font() {
        let font = parse_font(&synthetic_ttf(), "<test>").unwrap();
        let metrics = font.metrics();
        assert_eq!(metrics.units_per_em, 1000);
        assert_eq!(metrics.num_glyphs, 2);
        assert_eq!((metrics.ascent, metrics.descent, metrics.line_gap), (800, -200, 0));
        assert_eq!(metrics.outline_format, OutlineFormat::Glyf);
        assert_eq!(font.cmap_lookup(65), 1);
        assert_eq!(font.cmap_lookup(63), 1);
        assert_eq!(font.cmap_lookup(32), 0);
        assert_eq!(font.cmap_lookup(0x1_0000), 0);
        assert_eq!(font.advance_width(1), 600);
        assert_eq!(font.advance_width(9), 0);
        assert_eq!(font.kerning(1, 1), 0);
        assert!(font.color_layers(1).is_none());
        assert!(font.palette_color(0, 0).is_none());
        assert!(font.table("head").is_some());
        assert!(font.table("CFF ").is_none());
    }

    #[test]
    fn selects_format12_cmap() {
        let font = parse_font(&synthetic_ttf_format12(), "<test>").unwrap();
        assert_eq!(font.cmap_lookup(0x1f_600), 1);
        assert_eq!(font.cmap_lookup(0x1f_601), 0);
        assert_eq!(font.cmap_lookup(65), 0);
    }

    #[test]
    fn rejects_bad_signature() {
        assert!(parse_font(&[0u8; 4], "<test>").is_err());
        let mut bad = synthetic_ttf();
        bad[0] = b'X';
        let err = parse_font(&bad, "<test>").unwrap_err();
        assert!(err.to_string().contains("bad signature"), "{err}");
    }

    #[test]
    fn rejects_missing_table() {
        let square = square_glyph();
        let font = assemble(&[
            ("head", head_table(1000)),
            ("hhea", hhea_table(2)),
            ("maxp", maxp_table(2)),
            ("hmtx", hmtx_table(&[600, 600])),
            ("loca", loca_table(&[0, 0, square.len() as u32])),
            ("glyf", square),
        ]);
        let err = parse_font(&font, "<test>").unwrap_err();
        assert!(err.to_string().contains("required table"), "{err}");
    }

    #[test]
    fn rejects_bad_units_per_em() {
        let square = square_glyph();
        let font = assemble(&[
            ("head", head_table(0)),
            ("hhea", hhea_table(2)),
            ("maxp", maxp_table(2)),
            ("hmtx", hmtx_table(&[600, 600])),
            ("cmap", cmap_format4(&[(65, 65, -64)])),
            ("loca", loca_table(&[0, 0, square.len() as u32])),
            ("glyf", square),
        ]);
        let err = parse_font(&font, "<test>").unwrap_err();
        assert!(err.to_string().contains("unitsPerEm"), "{err}");
    }

    #[test]
    fn contours_flatten_square() {
        let font = parse_font(&synthetic_ttf(), "<test>").unwrap();
        assert!(font.contours(0).unwrap().is_empty());
        assert!(font.contours(99).unwrap().is_empty());
        let contours = font.contours(1).unwrap();
        assert_eq!(contours.len(), 1);
        let xs: Vec<f64> = contours[0].iter().map(|point| point.x).collect();
        let ys: Vec<f64> = contours[0].iter().map(|point| point.y).collect();
        assert_eq!(contours[0].len(), 4);
        assert_eq!(xs, vec![0.0, 500.0, 500.0, 0.0]);
        assert_eq!(ys, vec![0.0, 0.0, 700.0, 700.0]);
    }

    #[test]
    fn contours_apply_composite_offset() {
        let font = parse_font(&synthetic_ttf_composite(), "<test>").unwrap();
        assert_eq!(font.cmap_lookup(66), 2);
        let contours = font.contours(2).unwrap();
        assert_eq!(contours.len(), 1);
        let (mut min_x, mut min_y) = (f64::MAX, f64::MAX);
        let (mut max_x, mut max_y) = (f64::MIN, f64::MIN);
        for point in &contours[0] {
            min_x = min_x.min(point.x);
            min_y = min_y.min(point.y);
            max_x = max_x.max(point.x);
            max_y = max_y.max(point.y);
        }
        assert_eq!((min_x, min_y), (100.0, 50.0));
        assert_eq!((max_x, max_y), (600.0, 750.0));
    }

    #[test]
    fn rasterizes_square_glyph() {
        let font = parse_font(&synthetic_ttf(), "<test>").unwrap();
        let raster = rasterize_glyph(&font, 1, 16).unwrap();
        assert_eq!((raster.width, raster.height), (10, 13));
        assert_eq!(raster.coverage.len(), 130);
        assert!(raster.coverage.contains(&255));
        assert!(raster.coverage.contains(&0));
        let empty = rasterize_glyph(&font, 0, 16).unwrap();
        assert!(empty.coverage.iter().all(|value| *value == 0));
        assert!(rasterize_color_glyph(&font, 1, 16, 0, PaletteColor::WHITE)
            .unwrap()
            .is_none());
    }

    #[test]
    fn rasterize_contours_fills_box() {
        let square = vec![
            ContourPoint { x: 2.0, y: 2.0 },
            ContourPoint { x: 6.0, y: 2.0 },
            ContourPoint { x: 6.0, y: 6.0 },
            ContourPoint { x: 2.0, y: 6.0 },
        ];
        let raster = rasterize_contours(&[square], 8, 8, 4).unwrap();
        assert_eq!(raster.coverage[0], 0);
        assert_eq!(raster.coverage[3 * 8 + 3], 255);
        assert_eq!(raster.coverage[4 * 8 + 4], 255);
        assert_eq!(raster.coverage[7 * 8 + 7], 0);
    }

    #[test]
    fn builds_font_atlas() {
        let font = parse_font(&synthetic_ttf(), "<test>").unwrap();
        let atlas = build_font_atlas(&font, &[65, 63, 32], 16, 512).unwrap();
        assert_eq!((atlas.width, atlas.height), (512, 15));
        assert_eq!(atlas.line_height, 13);
        assert_eq!(atlas.pixels.len(), 512 * 15 * 4);
        let cell = atlas.glyphs[&65];
        assert_eq!(
            (cell.x, cell.y, cell.width, cell.height, cell.color),
            (1, 1, 10, 13, false)
        );
        assert_eq!(atlas.glyphs[&63].x, 12);
        assert!(!atlas.glyphs.contains_key(&32));
        let raster = rasterize_glyph(&font, 1, 16).unwrap();
        let probe = ((1 + 6) * 512 + (1 + 4)) as usize * 4;
        assert_eq!(raster.coverage[6 * 10 + 4], 255);
        assert_eq!(&atlas.pixels[probe..probe + 4], &[255, 255, 255, 255]);
        assert_eq!(&atlas.pixels[..4], &[0, 0, 0, 0]);
    }

    #[test]
    fn build_font_atlas_guards_sizes() {
        let font = parse_font(&synthetic_ttf(), "<test>").unwrap();
        assert!(build_font_atlas(&font, &[65], 0, 512).is_err());
        assert!(build_font_atlas(&font, &[65], 16, 0).is_err());
    }

    #[test]
    fn latin1_ranges_skip_controls() {
        let points = latin1_codepoints();
        assert_eq!(points.len(), 95 + 96);
        assert_eq!(points[0], 0x20);
        assert!(points.contains(&0x7e));
        assert!(!points.contains(&0x7f));
        assert!(!points.contains(&0x9f));
        assert!(points.contains(&0xa0));
        assert_eq!(points[points.len() - 1], 0xff);
    }

    #[test]
    fn validates_coverage_scalars() {
        assert!(validate_codepoint(0).is_ok());
        assert!(validate_codepoint(0x10_ffff).is_ok());
        assert!(validate_codepoint(0x11_0000).is_err());
        assert!(validate_codepoint(0xd800).is_err());
        assert!(validate_codepoint(0xdfff).is_err());
    }

    #[test]
    fn scaled_metrics_guard_pixel_size() {
        let font = parse_font(&synthetic_ttf(), "<test>").unwrap();
        let scaled = font.scaled_metrics(16).unwrap();
        assert_eq!(scaled.pixels_per_em, 16.0);
        assert!((scaled.line_height - 16.0).abs() < 1e-6);
        assert!(font.scaled_metrics(0).is_err());
        assert!(font.scaled_metrics(1025).is_err());
    }
}
