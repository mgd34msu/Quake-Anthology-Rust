//! TrueType data contract: sfnt directory, metrics, glyph-id lookup.
//!
//! Donor provenance: `src/text/truetype.ts` (`parseFont`, `ParsedFontT`
//! face/metrics/cmap surface).
//!
//! Contract only: this module validates the sfnt directory, required
//! tables, face metrics, and cmap format 4/12 glyph-id lookup. Outline
//! flattening and coverage rasterization (`contours`,
//! `rasterizeContours`, `rasterizeGlyph`, `rasterizeColorGlyph`,
//! `buildFontAtlas`) need a font engine and are deferred (see
//! [`ClientError::DeferredEngine`] and the crate README deferral
//! record).

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

/// A cmap lookup closure source.
#[derive(Debug, Clone, Copy)]
struct CmapSubtable {
    offset: usize,
    format: u16,
}

/// A parsed font face (contract surface).
#[derive(Debug, Clone)]
pub struct ParsedFont {
    tables: Vec<(String, TableRecord)>,
    metrics: FontMetrics,
    cmap: CmapSubtable,
    advances: Vec<u16>,
    bytes: Vec<u8>,
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
        let read_u16 = |offset: usize| -> u16 { u16::from_be_bytes([view[offset], view[offset + 1]]) };
        let read_u32 = |offset: usize| -> u32 {
            u32::from_be_bytes([view[offset], view[offset + 1], view[offset + 2], view[offset + 3]])
        };
        match self.cmap.format {
            4 => {
                if codepoint > 0xffff {
                    return 0;
                }
                let base = self.cmap.offset;
                let seg_count = read_u16(base + 6) as usize / 2;
                let end_base = base + 14;
                let start_base = end_base + seg_count * 2 + 2;
                let delta_base = start_base + seg_count * 2;
                let range_base = delta_base + seg_count * 2;
                for index in 0..seg_count {
                    let end = read_u16(end_base + index * 2) as u32;
                    if codepoint > end {
                        continue;
                    }
                    let start = read_u16(start_base + index * 2) as u32;
                    if codepoint < start {
                        return 0;
                    }
                    let delta = read_u16(delta_base + index * 2) as i16 as i32;
                    let range_offset = read_u16(range_base + index * 2) as usize;
                    if range_offset == 0 {
                        return ((codepoint as i32 + delta) & 0xffff) as u32;
                    }
                    let address = range_base + index * 2 + range_offset + (codepoint - start) as usize * 2;
                    let glyph = read_u16(address) as u32;
                    if glyph == 0 {
                        return 0;
                    }
                    return ((glyph as i32 + delta) & 0xffff) as u32;
                }
                0
            }
            _ => {
                let base = self.cmap.offset;
                let groups = read_u32(base + 12) as usize;
                let groups_base = base + 16;
                let (mut low, mut high) = (0usize, groups.saturating_sub(1));
                while groups > 0 && low <= high {
                    let mid = (low + high) >> 1;
                    let group = groups_base + mid * 12;
                    let start = read_u32(group);
                    let end = read_u32(group + 4);
                    if codepoint < start {
                        if mid == 0 {
                            break;
                        }
                        high = mid - 1;
                    } else if codepoint > end {
                        low = mid + 1;
                    } else {
                        return read_u32(group + 8) + (codepoint - start);
                    }
                }
                0
            }
        }
    }

    /// Advance width for a glyph id (`advanceWidth`, 0 when invalid).
    #[must_use]
    pub fn advance_width(&self, gid: u32) -> u16 {
        self.advances.get(gid as usize).copied().unwrap_or(0)
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
}

/// Latin-1 coverage (`latin1Codepoints`).
#[must_use]
pub fn latin1_codepoints() -> Vec<u32> {
    (0x20..=0xff).collect()
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

fn read_tag(bytes: &[u8], offset: usize) -> Result<String, ClientError> {
    if offset + 4 > bytes.len() {
        return Err(ClientError::BadFont(
            "not a recognized sfnt file (bad signature)".to_string(),
        ));
    }
    Ok(String::from_utf8_lossy(&bytes[offset..offset + 4]).into_owned())
}

/// Parse an sfnt font (`parseFont`, contract surface).
pub fn parse_font(bytes: &[u8], source: &str) -> Result<ParsedFont, ClientError> {
    let map_err = |error: BinaryError| ClientError::BadFont(format!("{source}: {error}"));
    if bytes.len() < 12 {
        return Err(ClientError::BadFont(format!(
            "{source}: not a recognized sfnt file (bad signature)"
        )));
    }
    let mut reader = BinaryReader::new(bytes, source);
    let mut magic = [0u8; 4];
    for slot in &mut magic {
        *slot = reader.u8().map_err(map_err)?;
    }
    let version = u32::from_be_bytes(magic);
    let tag = String::from_utf8_lossy(&magic).into_owned();
    if version != 0x0001_0000 && tag != "OTTO" && tag != "true" && version != 0x7472_7565 {
        return Err(ClientError::BadFont(format!(
            "{source}: not a recognized sfnt file (bad signature)"
        )));
    }
    // sfnt integers are big-endian; BinaryReader reads little-endian.
    let num_tables = {
        let raw = reader.bytes(2).map_err(map_err)?;
        u16::from_be_bytes([raw[0], raw[1]])
    };
    reader.skip(6).map_err(map_err)?;
    let mut tables: Vec<(String, TableRecord)> = Vec::new();
    for _ in 0..num_tables {
        let tag_bytes = reader.bytes(4).map_err(map_err)?;
        let tag = String::from_utf8_lossy(&tag_bytes).into_owned();
        reader.skip(4).map_err(map_err)?;
        let offset = {
            let raw = reader.bytes(4).map_err(map_err)?;
            u32::from_be_bytes([raw[0], raw[1], raw[2], raw[3]]) as usize
        };
        let length = {
            let raw = reader.bytes(4).map_err(map_err)?;
            u32::from_be_bytes([raw[0], raw[1], raw[2], raw[3]]) as usize
        };
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
        return Err(ClientError::BadFont(format!(
            "{source}: missing a required table (head/hhea/maxp/hmtx/cmap)"
        )));
    };
    if head.length < 54 {
        return Err(ClientError::BadFont(format!("{source}: head table too short")));
    }
    if hhea.length < 36 {
        return Err(ClientError::BadFont(format!("{source}: hhea table too short")));
    }
    if maxp.length < 6 {
        return Err(ClientError::BadFont(format!("{source}: maxp table too short")));
    }
    let view_u16 = |offset: usize| u16::from_be_bytes([bytes[offset], bytes[offset + 1]]);
    let view_i16 = |offset: usize| i16::from_be_bytes([bytes[offset], bytes[offset + 1]]);
    let units_per_em = view_u16(head.offset + 18);
    if units_per_em == 0 {
        return Err(ClientError::BadFont(format!("{source}: invalid unitsPerEm")));
    }
    let index_to_loc_format = view_i16(head.offset + 50);
    let ascent = view_i16(hhea.offset + 4);
    let descent = view_i16(hhea.offset + 6);
    let line_gap = view_i16(hhea.offset + 8);
    let number_of_h_metrics = view_u16(hhea.offset + 34);
    let num_glyphs = view_u16(maxp.offset + 4);
    if number_of_h_metrics < 1
        || number_of_h_metrics > num_glyphs
        || hmtx.length < usize::from(number_of_h_metrics) * 4 + usize::from(num_glyphs - number_of_h_metrics) * 2
    {
        return Err(ClientError::BadFont(format!("{source}: invalid horizontal metrics")));
    }
    let mut advances = vec![0u16; usize::from(num_glyphs)];
    let mut offset = hmtx.offset;
    let mut last = 0u16;
    for (glyph, advance) in advances.iter_mut().enumerate() {
        if glyph < usize::from(number_of_h_metrics) {
            last = view_u16(offset);
            offset += 4;
        }
        *advance = last;
    }
    let cmap_subtable = select_cmap(bytes, cmap.offset)
        .ok_or_else(|| ClientError::BadFont(format!("{source}: no supported cmap subtable (need format 4 or 12)")))?;
    let outline_format = match (table("glyf"), table("loca")) {
        (Some(_), Some(loca)) => {
            let stride = if index_to_loc_format == 0 { 2 } else { 4 };
            if (index_to_loc_format != 0 && index_to_loc_format != 1)
                || loca.length < (usize::from(num_glyphs) + 1) * stride
            {
                return Err(ClientError::BadFont(format!("{source}: invalid loca table")));
            }
            OutlineFormat::Glyf
        }
        _ if table("CFF ").is_some() => OutlineFormat::Cff,
        _ => {
            return Err(ClientError::BadFont(format!(
                "{source}: no outline table found (need glyf+loca or CFF )"
            )));
        }
    };
    let _ = read_tag(bytes, 0)?;
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
    })
}

fn select_cmap(bytes: &[u8], cmap_offset: usize) -> Option<CmapSubtable> {
    if cmap_offset + 4 > bytes.len() {
        return None;
    }
    let num_tables = u16::from_be_bytes([bytes[cmap_offset + 2], bytes[cmap_offset + 3]]) as usize;
    let mut best: Option<(u16, i32, usize)> = None;
    for index in 0..num_tables {
        let record = cmap_offset + 4 + index * 8;
        if record + 8 > bytes.len() {
            return None;
        }
        let platform = u16::from_be_bytes([bytes[record], bytes[record + 1]]);
        let encoding = u16::from_be_bytes([bytes[record + 2], bytes[record + 3]]);
        let sub_offset = cmap_offset
            + u32::from_be_bytes([
                bytes[record + 4],
                bytes[record + 5],
                bytes[record + 6],
                bytes[record + 7],
            ]) as usize;
        if sub_offset + 2 > bytes.len() {
            return None;
        }
        let format = u16::from_be_bytes([bytes[sub_offset], bytes[sub_offset + 1]]);
        let score = if format == 12
            && ((platform == 3 && encoding == 10) || (platform == 0 && (encoding == 4 || encoding == 6)))
        {
            100
        } else if format == 12 {
            90
        } else if format == 4 && platform == 3 && encoding == 1 {
            80
        } else if format == 4 && platform == 0 && encoding == 3 {
            70
        } else if format == 4 {
            60
        } else {
            -1
        };
        if score > best.map_or(-1, |(_, score, _)| score) {
            best = Some((format, score, sub_offset));
        }
    }
    best.map(|(format, _, offset)| CmapSubtable { offset, format })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn minimal_font() -> Vec<u8> {
        // sfnt with head/hhea/maxp/hmtx/cmap(format 4, one segment) and
        // empty glyf+loca for two glyphs.
        let mut bytes = vec![0u8; 512];
        bytes[0..4].copy_from_slice(&[0, 1, 0, 0]);
        bytes[4..6].copy_from_slice(&7u16.to_be_bytes());
        // searchRange/entrySelector/rangeShift stay zero.
        let mut directory = 12usize;
        let mut blob = 12 + 7 * 16;
        let mut table = |tag: &[u8; 4], data: &[u8]| {
            bytes[directory..directory + 4].copy_from_slice(tag);
            bytes[directory + 8..directory + 12].copy_from_slice(&(blob as u32).to_be_bytes());
            bytes[directory + 12..directory + 16].copy_from_slice(&(data.len() as u32).to_be_bytes());
            bytes[blob..blob + data.len()].copy_from_slice(data);
            directory += 16;
            blob += data.len();
        };
        let mut head = vec![0u8; 54];
        head[18..20].copy_from_slice(&1000u16.to_be_bytes());
        head[50..52].copy_from_slice(&0i16.to_be_bytes());
        let mut hhea = vec![0u8; 36];
        hhea[4..6].copy_from_slice(&800i16.to_be_bytes());
        hhea[6..8].copy_from_slice(&(-200i16).to_be_bytes());
        hhea[34..36].copy_from_slice(&2u16.to_be_bytes());
        let mut maxp = vec![0u8; 6];
        maxp[4..6].copy_from_slice(&2u16.to_be_bytes());
        let hmtx = vec![0u8, 100, 0, 0, 0, 120, 0, 0];
        // cmap: one format-4 subtable mapping 'A' (65) to gid 1.
        let mut cmap = vec![0u8; 16 + 32];
        cmap[2..4].copy_from_slice(&1u16.to_be_bytes());
        cmap[4..6].copy_from_slice(&3u16.to_be_bytes());
        cmap[6..8].copy_from_slice(&1u16.to_be_bytes());
        cmap[8..12].copy_from_slice(&16u32.to_be_bytes());
        let sub = 16usize;
        cmap[sub..sub + 2].copy_from_slice(&4u16.to_be_bytes());
        cmap[sub + 2..sub + 4].copy_from_slice(&32u16.to_be_bytes());
        cmap[sub + 6..sub + 8].copy_from_slice(&4u16.to_be_bytes());
        cmap[sub + 14..sub + 16].copy_from_slice(&65u16.to_be_bytes());
        cmap[sub + 16..sub + 18].copy_from_slice(&0xffffu16.to_be_bytes());
        cmap[sub + 20..sub + 22].copy_from_slice(&65u16.to_be_bytes());
        cmap[sub + 22..sub + 24].copy_from_slice(&0xffffu16.to_be_bytes());
        cmap[sub + 24..sub + 26].copy_from_slice(&(-64i16).to_be_bytes());
        cmap[sub + 26..sub + 28].copy_from_slice(&1i16.to_be_bytes());
        table(b"head", &head);
        table(b"hhea", &hhea);
        table(b"maxp", &maxp);
        table(b"hmtx", &hmtx);
        table(b"cmap", &cmap);
        table(b"glyf", &[]);
        table(b"loca", &[0, 0, 0, 0, 0, 0]);
        bytes.truncate(blob);
        bytes
    }

    #[test]
    fn parses_face_and_looks_up_glyph() {
        let font = parse_font(&minimal_font(), "<test>").unwrap();
        assert_eq!(font.metrics.units_per_em, 1000);
        assert_eq!(font.cmap_lookup(65), 1);
        assert_eq!(font.cmap_lookup(66), 0);
        assert_eq!(font.advance_width(1), 120);
        assert_eq!(font.advance_width(99), 0);
    }

    #[test]
    fn rejects_bad_signature() {
        assert!(parse_font(&[0u8; 32], "<test>").is_err());
    }

    #[test]
    fn coverage_validation() {
        assert!(validate_codepoint(0x10ffff).is_ok());
        assert!(validate_codepoint(0xd800).is_err());
        assert!(validate_codepoint(0x110000).is_err());
        assert_eq!(latin1_codepoints().len(), 224);
    }

    #[test]
    fn scaled_metrics() {
        let font = parse_font(&minimal_font(), "<test>").unwrap();
        let scaled = font.scaled_metrics(16).unwrap();
        assert!((scaled.line_height - 16.0).abs() < 1e-4);
        assert!(font.scaled_metrics(0).is_err());
    }
}
