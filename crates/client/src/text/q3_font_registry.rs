//! Q3 font registry: cached DAT records and picture binding.
//!
//! Donor provenance: `src/text/q3-font-registry.ts`
//! (`RendererFontRegistry`).
//!
//! Data-level sync port: fonts load from cached 20548-byte DAT records
//! with picture binding through a sync host. FreeType glyph generation
//! needs a font engine and is deferred (registration from a DAT file
//! works; generation from a `.ttf` path reports
//! [`ClientError::DeferredEngine`]).

use super::draw2d::{FontFileReader, MaterialPicture, PictureAsset};
use super::q3_font::{read_font_data, RegisteredFont, RegisteredGlyph, FONT_GLYPH_COUNT, FONT_RECORD_SIZE};
use crate::ClientError;

/// Maximum cached fonts.
pub const MAX_CACHED_FONTS: usize = 6;

/// Font registration host (sync).
pub trait FontRegistrationHost {
    /// Register a picture by shader name.
    fn register_picture(&mut self, shader_name: &str) -> MaterialPicture;
}

/// A checkpoint row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FontCheckpointRow {
    /// DAT record.
    pub record: Vec<u8>,
    /// Picture orders per glyph (`None` for unbound).
    pub pictures: Vec<Option<u32>>,
}

/// A registry checkpoint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FontRegistryCheckpoint {
    /// Rows.
    pub rows: Vec<FontCheckpointRow>,
}

/// The font registry (`RendererFontRegistry`, sync data level).
pub struct RendererFontRegistry<'a> {
    reader: &'a mut dyn FontFileReader,
    host: &'a mut dyn FontRegistrationHost,
    fonts: Vec<(RegisteredFont, Vec<u8>)>,
    closed: bool,
}

impl<'a> RendererFontRegistry<'a> {
    /// New registry.
    #[must_use]
    pub fn new(reader: &'a mut dyn FontFileReader, host: &'a mut dyn FontRegistrationHost) -> Self {
        Self {
            reader,
            host,
            fonts: Vec::new(),
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
        })
    }

    /// Restore a checkpoint.
    pub fn restore_checkpoint(&mut self, checkpoint: &FontRegistryCheckpoint) -> Result<(), ClientError> {
        self.require_open()?;
        if !self.fonts.is_empty() {
            return Err(ClientError::BadFont("Font restore requires an empty owner".to_string()));
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
        Ok(())
    }

    /// Register a font (`registerFont`, DAT path only).
    ///
    /// `path` selects the DAT record (`fonts/fontImage_{size}.dat`);
    /// generation from a TrueType file is deferred.
    pub fn register_font(
        &mut self,
        path: Option<&str>,
        point_size: f32,
        print: &mut dyn FnMut(&str),
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
        if let Some((font, _)) = self
            .fonts
            .iter()
            .find(|(font, _)| font.name.eq_ignore_ascii_case(&name))
        {
            return Ok(Some(font.clone()));
        }
        if self.reader.read_file_length(&name) != FONT_RECORD_SIZE as i64 {
            return self.generate_font(path, size, print);
        }
        let Some(file) = self.reader.read_file_retained(&name) else {
            return self.generate_font(path, size, print);
        };
        if file.length != FONT_RECORD_SIZE || file.bytes.len() != FONT_RECORD_SIZE {
            let retained = super::draw2d::RetainedFontFile {
                bytes: file.bytes,
                length: file.length,
            };
            self.reader.free_file(&retained);
            return self.generate_font(path, size, print);
        }
        let mut record = file.bytes.clone();
        record.fill(0);
        record[..file.bytes.len()].copy_from_slice(&file.bytes);
        record[20484..].fill(0);
        for (index, byte) in name.bytes().enumerate() {
            record[20484 + index] = byte;
        }
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
        let data = read_font_data(&record, &name)?;
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
        self.fonts.push((font.clone(), record));
        Ok(Some(font))
    }

    fn generate_font(
        &mut self,
        _path: Option<&str>,
        _size: usize,
        print: &mut dyn FnMut(&str),
    ) -> Result<Option<RegisteredFont>, ClientError> {
        print("RE_RegisterFont: FreeType code not available\n");
        Err(ClientError::DeferredEngine("FreeType font generation"))
    }

    /// Close the registry.
    pub fn close(&mut self) {
        if self.closed {
            return;
        }
        self.closed = true;
        self.fonts.clear();
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

    fn dat_record() -> Vec<u8> {
        let mut record = vec![0u8; FONT_RECORD_SIZE];
        // glyphScale = 1.0 at 20480.
        record[20480..20484].copy_from_slice(&1f32.to_le_bytes());
        record
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
    fn missing_dat_defers_generation() {
        let mut reader = FixedReader { files: HashMap::new() };
        let mut host = FixedHost { next: 0 };
        let mut registry = RendererFontRegistry::new(&mut reader, &mut host);
        let err = registry
            .register_font(Some("fonts/x.ttf"), 12.0, &mut |_| {})
            .unwrap_err();
        assert!(matches!(err, ClientError::DeferredEngine(_)));
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
        let mut reader = FixedReader { files: HashMap::new() };
        let mut host = FixedHost { next: 0 };
        let mut restored = RendererFontRegistry::new(&mut reader, &mut host);
        restored.restore_checkpoint(&checkpoint).unwrap();
        assert_eq!(restored.capture_checkpoint().unwrap(), checkpoint);
    }
}
