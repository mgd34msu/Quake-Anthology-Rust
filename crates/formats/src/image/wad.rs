use super::*;

#[derive(Debug)]
pub struct WadLump<'a> {
    pub name: &'a [u8],
    pub bytes: &'a [u8],
    pub decoded_length: u32,
    pub kind: u8,
    pub compression: u8,
}
pub struct Wad<'a> {
    pub wad3: bool,
    pub lumps: Vec<WadLump<'a>>,
}
pub enum WadImage<'a> {
    Image(IndexedImage<'a>),
    Texture {
        texture: MipTexture<'a>,
        palette: Option<Palette>,
    },
    Palette(Palette),
    Raw(&'a [u8]),
}
impl<'a> Wad<'a> {
    pub fn parse(bytes: &'a [u8]) -> Result<Self, FormatError> {
        let mut r = Reader::new(bytes);
        let magic = r.take(4)?;
        if magic != b"WAD2" && magic != b"WAD3" {
            return Err(FormatError::Unsupported);
        }
        let count = r.count(0, i32::MAX as usize)?;
        let offset = r.count(12, i32::MAX as usize)?;
        let rows = r.section(offset, count, 32, 12, bytes.len())?;
        let mut lumps = Vec::with_capacity(count);
        for row in rows.as_chunks::<32>().0 {
            let mut s = Reader::new(row);
            let at = s.count(0, i32::MAX as usize)?;
            let disk = s.count(0, i32::MAX as usize)?;
            let decoded_length = s.count(0, i32::MAX as usize)? as u32;
            let kind = s.u8()?;
            let compression = s.u8()?;
            s.take(2)?;
            let name = s.name(16)?;
            let bytes = r.section(at, disk, 1, 0, bytes.len())?;
            lumps.push(WadLump {
                name,
                bytes,
                decoded_length,
                kind,
                compression,
            });
        }
        Ok(Self {
            wad3: magic == b"WAD3",
            lumps,
        })
    }
    pub fn find(&self, name: &[u8]) -> Option<usize> {
        self.lumps
            .iter()
            .position(|lump| lump.name.eq_ignore_ascii_case(name))
    }
    pub fn image(&self, index: usize) -> Result<WadImage<'a>, FormatError> {
        let lump = self
            .lumps
            .get(index)
            .ok_or(FormatError::InvalidReference("WAD lump", index))?;
        if lump.compression != 0 {
            return Err(FormatError::Unsupported);
        }
        if lump.decoded_length as usize != lump.bytes.len() {
            return Err(FormatError::InvalidRecordSize);
        }
        if lump.name.eq_ignore_ascii_case(b"conchars") && lump.bytes.len() == 128 * 128 {
            return Ok(WadImage::Image(IndexedImage {
                width: 128,
                height: 128,
                indices: Cow::Borrowed(lump.bytes),
                palette: None,
            }));
        }
        match lump.kind {
            66 => indexed::qpic(lump.bytes).map(WadImage::Image),
            68 | 67 if lump.kind == 68 || self.wad3 => {
                let texture = MipTexture::parse(lump.bytes, MipFormat::Quake)?;
                let palette = if self.wad3 && !texture.levels[3].is_empty() {
                    let last = Reader::new(&lump.bytes[36..]).u32()? as usize;
                    let mut r = Reader::new(
                        lump.bytes
                            .get(last + texture.levels[3].len()..)
                            .ok_or(FormatError::Truncated)?,
                    );
                    if r.u16()? != 256 {
                        return Err(FormatError::InvalidValue);
                    }
                    Some(Palette::from_rgb(r.take(768)?)?)
                } else {
                    None
                };
                Ok(WadImage::Texture { texture, palette })
            }
            64 if lump.bytes.len() == 768 => Palette::from_rgb(lump.bytes).map(WadImage::Palette),
            _ => Ok(WadImage::Raw(lump.bytes)),
        }
    }
}
