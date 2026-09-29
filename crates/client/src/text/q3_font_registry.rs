//! Q3 font registry: cached DAT records, picture binding, DAT generation.
//!
//! Donor provenance: `src/text/q3-font-registry.ts`
//! (`RendererFontRegistry`, `FontGenerationServices`). Data-level sync port:
//! fonts load from cached 20548-byte DAT records with picture binding
//! through a sync host, and generate from a TrueType file through
//! [`crate::text::truetype`] when the DAT record is missing. The sync port
//! has no render-thread handoff or pending-registration queue: `&mut`
//! borrowing serializes registration.

use std::collections::{HashMap, HashSet};

use super::draw2d::{FontFileReader, MaterialPicture, PictureAsset};
use super::q3_font::{read_font_data, RegisteredFont, RegisteredGlyph, FONT_GLYPH_COUNT, FONT_RECORD_SIZE};
use super::truetype::{parse_font, rasterize_glyph, ParsedFont};
use crate::ClientError;

/// Maximum cached fonts.
pub const MAX_CACHED_FONTS: usize = 6;

/// Generated atlas edge in pixels.
const GENERATED_ATLAS_EDGE: u32 = 256;

/// Font registration host (sync).
pub trait FontRegistrationHost {
    /// Register a picture by shader name.
    fn register_picture(&mut self, shader_name: &str) -> MaterialPicture;
}

/// Font generation services (`FontGenerationServices`, sync).
pub trait FontGenerationServices {
    /// Register a generated 256 by 256 RGBA atlas; returns its picture.
    fn register_image(&mut self, name: &str, rgba: Vec<u8>) -> MaterialPicture;
    /// Whether generated DAT/TGA files should be written back.
    fn save_font_data(&self) -> bool;
    /// Write a generated file.
    fn write_file(&mut self, name: &str, bytes: Vec<u8>);
}

/// A rendered glyph bitmap (`FontGlyphBitmap`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FontGlyphBitmap {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// Row stride in pixels.
    pub pitch: u32,
    /// Pixels above the baseline plus one.
    pub top: i32,
    /// Baseline offset in 26.6 units.
    pub bottom: i32,
    /// Horizontal advance in pixels plus one.
    pub x_skip: i32,
    /// Row-major coverage bytes (`height * pitch`).
    pub pixels: Vec<u8>,
}

/// Render one ASCII-slot bitmap through the TrueType engine (`renderGlyph`).
fn render_glyph_bitmap(font: &ParsedFont, code: u32, size: u32) -> Option<FontGlyphBitmap> {
    let gid = font.cmap_lookup(code);
    let raster = rasterize_glyph(font, gid, size).ok()?;
    let metrics = font.metrics();
    let scale = f64::from(size) / f64::from(metrics.units_per_em);
    let top_bearing = (f64::from(metrics.ascent) * scale).floor() as i32;
    Some(FontGlyphBitmap {
        width: raster.width,
        height: raster.height,
        pitch: raster.width,
        top: top_bearing + 1,
        bottom: (top_bearing - raster.height as i32) * 64,
        x_skip: (f64::from(font.advance_width(gid)) * scale).floor() as i32 + 1,
        pixels: raster.coverage,
    })
}

/// Write one glyph slot (`writeGlyph`).
fn write_glyph(record: &mut [u8], code: u32, glyph: Option<&FontGlyphBitmap>, x: u32, y: u32) {
    let offset = code as usize * 80;
    record[offset..offset + 80].fill(0);
    let Some(glyph) = glyph else {
        return;
    };
    let ints = [
        glyph.height as i32,
        glyph.top,
        glyph.bottom,
        glyph.pitch as i32,
        glyph.x_skip,
        glyph.pitch as i32,
        glyph.height as i32,
    ];
    for (index, value) in ints.iter().enumerate() {
        record[offset + index * 4..offset + index * 4 + 4].copy_from_slice(&value.to_le_bytes());
    }
    let floats = [
        x as f32 / 256.0,
        y as f32 / 256.0,
        (x + glyph.pitch) as f32 / 256.0,
        (y + glyph.height) as f32 / 256.0,
    ];
    for (index, value) in floats.iter().enumerate() {
        record[offset + 28 + index * 4..offset + 28 + index * 4 + 4].copy_from_slice(&value.to_le_bytes());
    }
}

/// Write a zero-padded name (`writeName`).
fn write_name(record: &mut [u8], offset: usize, length: usize, name: &str) {
    record[offset..offset + length].fill(0);
    let bytes = name.as_bytes();
    let take = bytes.len().min(length - 1);
    record[offset..offset + take].copy_from_slice(&bytes[..take]);
}

/// Encode a generated atlas as TGA (`tga`).
fn tga_bytes(rgba: &[u8]) -> Vec<u8> {
    let mut bytes = vec![0u8; rgba.len() + 18];
    bytes[2] = 2;
    bytes[13] = 1;
    bytes[15] = 1;
    bytes[16] = 32;
    bytes[17] = 0x28;
    bytes[18..].copy_from_slice(rgba);
    bytes
}

/// A checkpoint row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FontCheckpointRow {
    /// DAT record.
    pub record: Vec<u8>,
    /// Picture orders per glyph (`None` for unbound).
    pub pictures: Vec<Option<u32>>,
}

/// A generated atlas row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GeneratedFontAtlas {
    /// Atlas image name.
    pub name: String,
    /// 256 by 256 RGBA bytes.
    pub rgba: Vec<u8>,
}

/// A registry checkpoint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FontRegistryCheckpoint {
    /// Rows.
    pub rows: Vec<FontCheckpointRow>,
    /// Generated atlases by name.
    pub generated: Vec<GeneratedFontAtlas>,
}

/// The font registry (`RendererFontRegistry`, sync data level).
pub struct RendererFontRegistry<'a> {
    reader: &'a mut dyn FontFileReader,
    host: &'a mut dyn FontRegistrationHost,
    generation: Option<&'a mut dyn FontGenerationServices>,
    fonts: Vec<(RegisteredFont, Vec<u8>)>,
    generated: HashMap<String, Vec<u8>>,
    closed: bool,
}

impl<'a> RendererFontRegistry<'a> {
    /// New registry without generation services.
    #[must_use]
    pub fn new(reader: &'a mut dyn FontFileReader, host: &'a mut dyn FontRegistrationHost) -> Self {
        Self {
            reader,
            host,
            generation: None,
            fonts: Vec::new(),
            generated: HashMap::new(),
            closed: false,
        }
    }

    /// New registry with generation services.
    #[must_use]
    pub fn with_generation(
        reader: &'a mut dyn FontFileReader,
        host: &'a mut dyn FontRegistrationHost,
        generation: &'a mut dyn FontGenerationServices,
    ) -> Self {
        Self {
            reader,
            host,
            generation: Some(generation),
            fonts: Vec::new(),
            generated: HashMap::new(),
            closed: false,
        }
    }

    fn require_open(&self) -> Result<(), ClientError> {
        if self.closed {
            return Err(ClientError::BadFont("Renderer font registry is closed".to_string()));
        }
        Ok(())
    }

    /// Capture a checkpoint.
    pub fn capture_checkpoint(&self) -> Result<FontRegistryCheckpoint, ClientError> {
        self.require_open()?;
        let mut generated: Vec<GeneratedFontAtlas> = self
            .generated
            .iter()
            .map(|(name, rgba)| GeneratedFontAtlas {
                name: name.clone(),
                rgba: rgba.clone(),
            })
            .collect();
        generated.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(FontRegistryCheckpoint {
            rows: self
                .fonts
                .iter()
                .map(|(font, record)| FontCheckpointRow {
                    record: record.clone(),
                    pictures: font
                        .glyphs
                        .iter()
                        .map(|glyph| match glyph.picture {
                            Some(PictureAsset::Material(material)) => Some(material.order),
                            _ => None,
                        })
                        .collect(),
                })
                .collect(),
            generated,
        })
    }

    /// Restore a checkpoint.
    pub fn restore_checkpoint(&mut self, checkpoint: &FontRegistryCheckpoint) -> Result<(), ClientError> {
        self.require_open()?;
        if !self.fonts.is_empty() {
            return Err(ClientError::BadFont("Font restore requires an empty owner".to_string()));
        }
        if !checkpoint.generated.is_empty() && self.generation.is_none() {
            return Err(ClientError::BadFont(
                "Generated font restore requires its atlas owner".to_string(),
            ));
        }
        let mut rebound = HashSet::new();
        for atlas in &checkpoint.generated {
            if atlas.rgba.len() != GENERATED_ATLAS_EDGE as usize * GENERATED_ATLAS_EDGE as usize * 4
                || !rebound.insert(atlas.name.clone())
            {
                return Err(ClientError::BadFont("invalid generated font atlas".to_string()));
            }
            if let Some(generation) = self.generation.as_deref_mut() {
                generation.register_image(&atlas.name, atlas.rgba.clone());
            }
            self.require_open()?;
        }
        if checkpoint.rows.len() > MAX_CACHED_FONTS {
            return Err(ClientError::BadFont("too many cached fonts".to_string()));
        }
        for row in &checkpoint.rows {
            if row.record.len() != FONT_RECORD_SIZE || row.pictures.len() != FONT_GLYPH_COUNT {
                return Err(ClientError::BadFont("invalid font record".to_string()));
            }
            let data = read_font_data(&row.record, "<font>")?;
            let mut glyphs = Vec::with_capacity(FONT_GLYPH_COUNT);
            for (index, metric) in data.glyphs.iter().enumerate() {
                let picture = row.pictures[index].map(|order| PictureAsset::Material(MaterialPicture { order }));
                glyphs.push(RegisteredGlyph {
                    metrics: metric.clone(),
                    picture,
                });
            }
            self.fonts.push((
                RegisteredFont {
                    name: data.name,
                    glyph_scale: data.glyph_scale,
                    glyphs,
                },
                row.record.clone(),
            ));
        }
        self.generated.extend(
            checkpoint
                .generated
                .iter()
                .map(|atlas| (atlas.name.clone(), atlas.rgba.clone())),
        );
        Ok(())
    }

    /// Register a font (`registerFont`).
    ///
    /// `path` selects the TrueType source used when the DAT record
    /// (`fonts/fontImage_{size}.dat`) is missing.
    pub fn register_font(
        &mut self,
        path: Option<&str>,
        point_size: f32,
        print: &mut dyn FnMut(&str),
    ) -> Result<Option<RegisteredFont>, ClientError> {
        self.load_font(path, point_size, print, None)
    }

    /// Register a font into a caller record (`registerFont` destination).
    pub fn register_font_into(
        &mut self,
        path: Option<&str>,
        point_size: f32,
        print: &mut dyn FnMut(&str),
        destination: &mut [u8],
    ) -> Result<Option<RegisteredFont>, ClientError> {
        if destination.len() != FONT_RECORD_SIZE {
            return Err(ClientError::BadFont(
                "fontInfo_t destination must contain exactly 20548 bytes".to_string(),
            ));
        }
        self.load_font(path, point_size, print, Some(destination))
    }

    fn load_font(
        &mut self,
        path: Option<&str>,
        point_size: f32,
        print: &mut dyn FnMut(&str),
        destination: Option<&mut [u8]>,
    ) -> Result<Option<RegisteredFont>, ClientError> {
        self.require_open()?;
        if !point_size.is_finite() {
            return Err(ClientError::BadFont("Invalid font point size".to_string()));
        }
        let size = point_size.trunc() as i32;
        let size = if size <= 0 { 12 } else { size as usize };
        if self.fonts.len() >= MAX_CACHED_FONTS {
            print("RE_RegisterFont: Too many fonts registered already.\n");
            return Ok(None);
        }
        let name = format!("fonts/fontImage_{size}.dat");
        if let Some((font, record)) = self
            .fonts
            .iter()
            .find(|(font, _)| font.name.eq_ignore_ascii_case(&name))
        {
            let font = font.clone();
            if let Some(destination) = destination {
                destination.copy_from_slice(record);
            }
            return Ok(Some(font));
        }
        let file = if self.reader.read_file_length(&name) == FONT_RECORD_SIZE as i64 {
            self.reader.read_file_retained(&name)
        } else {
            None
        };
        self.require_open()?;
        let Some(file) = file else {
            return self.generate_font(path, size, print, destination);
        };
        if file.length != FONT_RECORD_SIZE || file.bytes.len() != FONT_RECORD_SIZE {
            let retained = super::draw2d::RetainedFontFile {
                bytes: file.bytes,
                length: file.length,
            };
            self.reader.free_file(&retained);
            return self.generate_font(path, size, print, destination);
        }
        let mut fallback;
        let record: &mut [u8] = if let Some(destination) = destination {
            destination
        } else {
            fallback = vec![0u8; FONT_RECORD_SIZE];
            &mut fallback
        };
        record.copy_from_slice(&file.bytes);
        write_name(record, 20484, 64, &name);
        let mut pictures: Vec<MaterialPicture> = Vec::with_capacity(255);
        for index in 0..255 {
            let name_bytes = &record[index * 80 + 48..index * 80 + 80];
            let end = name_bytes.iter().position(|byte| *byte == 0).ok_or_else(|| {
                ClientError::BadFont("Font shader name has no terminator inside fontInfo_t".to_string())
            })?;
            let shader_name = String::from_utf8_lossy(&name_bytes[..end]).into_owned();
            let picture = self.host.register_picture(&shader_name);
            self.require_open()?;
            record[index * 80 + 44..index * 80 + 48].copy_from_slice(&picture.order.to_le_bytes());
            pictures.push(picture);
        }
        let data = read_font_data(record, &name)?;
        let mut glyphs = Vec::with_capacity(FONT_GLYPH_COUNT);
        for (index, metric) in data.glyphs.iter().enumerate() {
            let picture = if index == 255 {
                None
            } else {
                Some(
                    pictures
                        .get(index)
                        .copied()
                        .map(PictureAsset::Material)
                        .ok_or_else(|| ClientError::BadFont("Missing registered font glyph".to_string()))?,
                )
            };
            glyphs.push(RegisteredGlyph {
                metrics: metric.clone(),
                picture,
            });
        }
        let font = RegisteredFont {
            name: data.name,
            glyph_scale: data.glyph_scale,
            glyphs,
        };
        self.fonts.push((font.clone(), record.to_vec()));
        Ok(Some(font))
    }

    /// Generate a DAT record from a TrueType file (`generateFont`).
    fn generate_font(
        &mut self,
        path: Option<&str>,
        size: usize,
        print: &mut dyn FnMut(&str),
        destination: Option<&mut [u8]>,
    ) -> Result<Option<RegisteredFont>, ClientError> {
        self.require_open()?;
        if self.generation.is_none() {
            print("RE_RegisterFont: FreeType code not available\n");
            return Ok(None);
        }
        let file = self.reader.read_file_retained(path.unwrap_or(""));
        let Some(file) = file else {
            print("RE_RegisterFont: Unable to read font file\n");
            return Ok(None);
        };
        if file.length == 0 || file.bytes.is_empty() {
            self.reader.free_file(&file);
            print("RE_RegisterFont: Unable to read font file\n");
            return Ok(None);
        }
        let font = match parse_font(&file.bytes, path.unwrap_or("")) {
            Ok(font) => font,
            Err(_) => {
                self.reader.free_file(&file);
                print("RE_RegisterFont: unable to parse font file\n");
                return Ok(None);
            }
        };
        let size_u32 = size as u32;
        let bitmaps: Vec<Option<FontGlyphBitmap>> = (0..=255u32)
            .map(|code| render_glyph_bitmap(&font, code, size_u32))
            .collect();
        let mut max_height = 0u32;
        for code in 0..255u32 {
            if let Some(glyph) = &bitmaps[code as usize] {
                max_height = max_height.max(glyph.height);
            }
        }
        let mut fallback;
        let record: &mut [u8] = if let Some(destination) = destination {
            destination
        } else {
            fallback = vec![0u8; FONT_RECORD_SIZE];
            &mut fallback
        };
        let mut pictures: Vec<Option<MaterialPicture>> = vec![None; 256];
        let mut atlas = vec![0u8; GENERATED_ATLAS_EDGE as usize * GENERATED_ATLAS_EDGE as usize];
        let (mut x, mut y, mut last_start, mut image_number) = (0u32, 0u32, 0u32, 0u32);
        for code in 0..=255u32 {
            let glyph = &bitmaps[code as usize];
            let mut overflow = false;
            if let Some(glyph) = glyph {
                max_height = max_height.max(glyph.height);
                if x + glyph.pitch + 1 >= 255 {
                    if y + max_height + 1 >= 255 {
                        overflow = true;
                    } else {
                        x = 0;
                        y += max_height + 1;
                    }
                } else if y + max_height + 1 >= 255 {
                    overflow = true;
                }
                if !overflow {
                    if x + glyph.pitch > GENERATED_ATLAS_EDGE || y + glyph.height > GENERATED_ATLAS_EDGE {
                        self.reader.free_file(&file);
                        return Err(ClientError::BadFont("Font glyph exceeds atlas bounds".to_string()));
                    }
                    for row in 0..glyph.height {
                        let start = (row * glyph.pitch) as usize;
                        let dst = ((y + row) * GENERATED_ATLAS_EDGE + x) as usize;
                        atlas[dst..dst + glyph.pitch as usize]
                            .copy_from_slice(&glyph.pixels[start..start + glyph.pitch as usize]);
                    }
                }
            }
            if overflow || code == 255 {
                let maximum = atlas.iter().copied().max().unwrap_or(0);
                let scale = if maximum > 0 {
                    255.0f32 / f32::from(maximum)
                } else {
                    0.0
                };
                let mut rgba = vec![0u8; GENERATED_ATLAS_EDGE as usize * GENERATED_ATLAS_EDGE as usize * 4];
                for (pixel, value) in atlas.iter().enumerate() {
                    let base = pixel * 4;
                    rgba[base..base + 3].copy_from_slice(&[255, 255, 255]);
                    rgba[base + 3] = (f32::from(*value) * scale) as u8;
                }
                let image_name = format!("fonts/fontImage_{image_number}_{size}.tga");
                image_number += 1;
                if self
                    .generation
                    .as_ref()
                    .is_some_and(|generation| generation.save_font_data())
                {
                    let tga = tga_bytes(&rgba);
                    if let Some(generation) = self.generation.as_deref_mut() {
                        generation.write_file(&image_name, tga);
                    }
                }
                let picture = self
                    .generation
                    .as_deref_mut()
                    .map(|generation| generation.register_image(&image_name, rgba.clone()));
                let Some(picture) = picture else {
                    self.reader.free_file(&file);
                    print("RE_RegisterFont: FreeType code not available\n");
                    return Ok(None);
                };
                self.generated.insert(image_name.clone(), rgba);
                for index in last_start..code {
                    let base = index as usize * 80;
                    record[base + 44..base + 48].copy_from_slice(&picture.order.to_le_bytes());
                    write_name(record, base + 48, 32, &image_name);
                    pictures[index as usize] = Some(picture);
                }
                last_start = code;
                atlas.fill(0);
                x = 0;
                y = 0;
            } else {
                write_glyph(record, code, glyph.as_ref(), x, y);
                if let Some(glyph) = glyph {
                    x += glyph.pitch + 1;
                }
            }
        }
        record[20480..20484].copy_from_slice(&(48.0f32 / size as f32).to_le_bytes());
        write_name(record, 20484, 64, &format!("fonts/fontImage_{size}.dat"));
        let data = read_font_data(record, &format!("fonts/fontImage_{size}.dat"))?;
        let mut glyphs = Vec::with_capacity(FONT_GLYPH_COUNT);
        for (index, metric) in data.glyphs.iter().enumerate() {
            let picture = pictures.get(index).copied().flatten().map(PictureAsset::Material);
            glyphs.push(RegisteredGlyph {
                metrics: metric.clone(),
                picture,
            });
        }
        let result = RegisteredFont {
            name: data.name,
            glyph_scale: data.glyph_scale,
            glyphs,
        };
        self.fonts.push((result.clone(), record.to_vec()));
        if self
            .generation
            .as_ref()
            .is_some_and(|generation| generation.save_font_data())
        {
            let dat = record.to_vec();
            if let Some(generation) = self.generation.as_deref_mut() {
                generation.write_file(&format!("fonts/fontImage_{size}.dat"), dat);
            }
        }
        self.reader.free_file(&file);
        Ok(Some(result))
    }

    /// Close the registry.
    pub fn close(&mut self) {
        if self.closed {
            return;
        }
        self.closed = true;
        self.fonts.clear();
        self.generated.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    struct FixedReader {
        files: HashMap<String, Vec<u8>>,
    }

    impl FontFileReader for FixedReader {
        fn read_file_length(&mut self, path: &str) -> i64 {
            self.files.get(path).map_or(-1, |bytes| bytes.len() as i64)
        }

        fn read_file_retained(&mut self, path: &str) -> Option<super::super::draw2d::RetainedFontFile> {
            self.files
                .get(path)
                .map(|bytes| super::super::draw2d::RetainedFontFile {
                    bytes: bytes.clone(),
                    length: bytes.len(),
                })
        }

        fn free_file(&mut self, _file: &super::super::draw2d::RetainedFontFile) {}
    }

    struct FixedHost {
        next: u32,
    }

    impl FontRegistrationHost for FixedHost {
        fn register_picture(&mut self, _shader_name: &str) -> MaterialPicture {
            self.next += 1;
            MaterialPicture { order: self.next }
        }
    }

    struct FixedGeneration {
        next: u32,
        save: bool,
        images: Vec<(String, Vec<u8>)>,
        files: HashMap<String, Vec<u8>>,
    }

    impl FontGenerationServices for FixedGeneration {
        fn register_image(&mut self, name: &str, rgba: Vec<u8>) -> MaterialPicture {
            self.next += 1;
            self.images.push((name.to_string(), rgba));
            MaterialPicture { order: self.next }
        }

        fn save_font_data(&self) -> bool {
            self.save
        }

        fn write_file(&mut self, name: &str, bytes: Vec<u8>) {
            self.files.insert(name.to_string(), bytes);
        }
    }

    fn dat_record() -> Vec<u8> {
        let mut record = vec![0u8; FONT_RECORD_SIZE];
        // glyphScale = 1.0 at 20480.
        record[20480..20484].copy_from_slice(&1f32.to_le_bytes());
        record
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

    fn generation(save: bool) -> FixedGeneration {
        FixedGeneration {
            next: 100,
            save,
            images: Vec::new(),
            files: HashMap::new(),
        }
    }

    #[test]
    fn registers_from_dat() {
        let mut reader = FixedReader {
            files: HashMap::from([("fonts/fontImage_12.dat".to_string(), dat_record())]),
        };
        let mut host = FixedHost { next: 0 };
        let mut registry = RendererFontRegistry::new(&mut reader, &mut host);
        let mut log = String::new();
        let font = registry
            .register_font(None, 12.0, &mut |text| log.push_str(text))
            .unwrap()
            .unwrap();
        assert_eq!(font.glyphs.len(), 256);
        assert!(font.glyphs[255].picture.is_none());
        assert!(font.glyphs[0].picture.is_some());
    }

    #[test]
    fn generates_dat_from_truetype() {
        let mut reader = FixedReader {
            files: HashMap::from([("fonts/test.ttf".to_string(), fixture_ttf())]),
        };
        let mut host = FixedHost { next: 0 };
        let mut services = generation(true);
        let font = {
            let mut registry = RendererFontRegistry::with_generation(&mut reader, &mut host, &mut services);
            let mut log = String::new();
            let font = registry
                .register_font(Some("fonts/test.ttf"), 12.0, &mut |text| log.push_str(text))
                .unwrap()
                .unwrap();
            assert!(log.is_empty(), "{log}");
            assert_eq!(font.name, "fonts/fontImage_12.dat");
            assert_eq!(font.glyph_scale, 48.0 / 12.0);
            assert_eq!(font.glyphs.len(), 256);
            assert!(font.glyphs[255].picture.is_none());
            assert!(font.glyphs[65].picture.is_some());
            let metrics = &font.glyphs[65].metrics;
            assert_eq!((metrics.image_width, metrics.image_height), (7, 10));
            assert_eq!((metrics.top, metrics.x_skip), (10, 8));
            assert_eq!(metrics.shader_name, "fonts/fontImage_0_12.tga");
            let checkpoint = registry.capture_checkpoint().unwrap();
            assert_eq!(checkpoint.rows.len(), 1);
            assert_eq!(checkpoint.rows[0].record.len(), FONT_RECORD_SIZE);
            assert_eq!(checkpoint.generated.len(), 1);
            assert_eq!(checkpoint.generated[0].name, "fonts/fontImage_0_12.tga");
            assert_eq!(checkpoint.generated[0].rgba.len(), 256 * 256 * 4);
            let mut alphas = checkpoint.generated[0].rgba.iter().skip(3).step_by(4);
            assert!(alphas.clone().any(|alpha| *alpha == 255));
            assert!(alphas.any(|alpha| *alpha == 0));
            font
        };
        assert_eq!(services.images.len(), 1);
        assert_eq!(services.images[0].0, "fonts/fontImage_0_12.tga");
        let tga = &services.files["fonts/fontImage_0_12.tga"];
        assert_eq!(tga.len(), 256 * 256 * 4 + 18);
        assert_eq!((tga[2], tga[16], tga[17]), (2, 32, 0x28));
        assert_eq!(services.files["fonts/fontImage_12.dat"].len(), FONT_RECORD_SIZE);
        assert_eq!(font.glyphs.len(), 256);
    }

    #[test]
    fn generated_font_is_cached() {
        let mut reader = FixedReader {
            files: HashMap::from([("fonts/test.ttf".to_string(), fixture_ttf())]),
        };
        let mut host = FixedHost { next: 0 };
        let mut services = generation(false);
        let mut registry = RendererFontRegistry::with_generation(&mut reader, &mut host, &mut services);
        registry
            .register_font(Some("fonts/test.ttf"), 12.0, &mut |_| {})
            .unwrap();
        registry
            .register_font(Some("fonts/test.ttf"), 12.0, &mut |_| {})
            .unwrap();
        assert_eq!(services.images.len(), 1);
        assert!(services.files.is_empty());
    }

    #[test]
    fn generation_without_services_returns_none() {
        let mut reader = FixedReader {
            files: HashMap::from([("fonts/test.ttf".to_string(), fixture_ttf())]),
        };
        let mut host = FixedHost { next: 0 };
        let mut registry = RendererFontRegistry::new(&mut reader, &mut host);
        let mut log = String::new();
        let font = registry
            .register_font(Some("fonts/test.ttf"), 12.0, &mut |text| log.push_str(text))
            .unwrap();
        assert!(font.is_none());
        assert!(log.contains("FreeType code not available"), "{log}");
    }

    #[test]
    fn generation_without_font_file_returns_none() {
        let mut reader = FixedReader { files: HashMap::new() };
        let mut host = FixedHost { next: 0 };
        let mut services = generation(true);
        let mut registry = RendererFontRegistry::with_generation(&mut reader, &mut host, &mut services);
        let mut log = String::new();
        let font = registry
            .register_font(Some("fonts/test.ttf"), 12.0, &mut |text| log.push_str(text))
            .unwrap();
        assert!(font.is_none());
        assert!(log.contains("Unable to read font file"), "{log}");
    }

    #[test]
    fn generation_with_corrupt_font_returns_none() {
        let mut reader = FixedReader {
            files: HashMap::from([("fonts/test.ttf".to_string(), vec![0u8; 64])]),
        };
        let mut host = FixedHost { next: 0 };
        let mut services = generation(true);
        let mut registry = RendererFontRegistry::with_generation(&mut reader, &mut host, &mut services);
        let mut log = String::new();
        let font = registry
            .register_font(Some("fonts/test.ttf"), 12.0, &mut |text| log.push_str(text))
            .unwrap();
        assert!(font.is_none());
        assert!(!log.is_empty());
    }

    #[test]
    fn register_font_into_writes_destination() {
        let mut reader = FixedReader {
            files: HashMap::from([("fonts/fontImage_12.dat".to_string(), dat_record())]),
        };
        let mut host = FixedHost { next: 0 };
        let mut registry = RendererFontRegistry::new(&mut reader, &mut host);
        let mut destination = vec![0u8; FONT_RECORD_SIZE];
        registry
            .register_font_into(None, 12.0, &mut |_| {}, &mut destination)
            .unwrap();
        assert!(destination.iter().any(|byte| *byte != 0));
        let mut short = vec![0u8; 8];
        assert!(registry
            .register_font_into(None, 12.0, &mut |_| {}, &mut short)
            .is_err());
    }

    #[test]
    fn rejects_too_many_fonts() {
        let files: HashMap<String, Vec<u8>> = (10..17)
            .map(|size| (format!("fonts/fontImage_{size}.dat"), dat_record()))
            .collect();
        let mut reader = FixedReader { files };
        let mut host = FixedHost { next: 0 };
        let mut registry = RendererFontRegistry::new(&mut reader, &mut host);
        for size in 10..16 {
            registry.register_font(None, size as f32, &mut |_| {}).unwrap();
        }
        let mut log = String::new();
        let font = registry
            .register_font(None, 16.0, &mut |text| log.push_str(text))
            .unwrap();
        assert!(font.is_none());
        assert!(log.contains("Too many fonts"), "{log}");
    }

    #[test]
    fn checkpoint_round_trips() {
        let mut reader = FixedReader {
            files: HashMap::from([("fonts/fontImage_12.dat".to_string(), dat_record())]),
        };
        let mut host = FixedHost { next: 0 };
        let mut registry = RendererFontRegistry::new(&mut reader, &mut host);
        registry.register_font(None, 12.0, &mut |_| {}).unwrap();
        let checkpoint = registry.capture_checkpoint().unwrap();
        assert_eq!(checkpoint.rows.len(), 1);
        assert!(checkpoint.generated.is_empty());
        let mut reader = FixedReader { files: HashMap::new() };
        let mut host = FixedHost { next: 0 };
        let mut restored = RendererFontRegistry::new(&mut reader, &mut host);
        restored.restore_checkpoint(&checkpoint).unwrap();
        assert_eq!(restored.capture_checkpoint().unwrap(), checkpoint);
    }

    #[test]
    fn checkpoint_restores_generated_atlases() {
        let checkpoint = {
            let mut reader = FixedReader {
                files: HashMap::from([("fonts/test.ttf".to_string(), fixture_ttf())]),
            };
            let mut host = FixedHost { next: 0 };
            let mut services = generation(false);
            let mut registry = RendererFontRegistry::with_generation(&mut reader, &mut host, &mut services);
            registry
                .register_font(Some("fonts/test.ttf"), 12.0, &mut |_| {})
                .unwrap();
            registry.capture_checkpoint().unwrap()
        };
        assert_eq!(checkpoint.generated.len(), 1);
        let mut reader = FixedReader { files: HashMap::new() };
        let mut host = FixedHost { next: 0 };
        let mut services = generation(false);
        let mut restored = RendererFontRegistry::with_generation(&mut reader, &mut host, &mut services);
        restored.restore_checkpoint(&checkpoint).unwrap();
        assert_eq!(restored.capture_checkpoint().unwrap(), checkpoint);
        assert_eq!(services.images.len(), 1);
        let mut reader = FixedReader { files: HashMap::new() };
        let mut host = FixedHost { next: 0 };
        let mut bare = RendererFontRegistry::new(&mut reader, &mut host);
        assert!(bare.restore_checkpoint(&checkpoint).is_err());
    }
}
