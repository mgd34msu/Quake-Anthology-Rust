//! Cold VFS image/palette resolution. Frames retain numeric asset handles.
use crate::assets::{Assets, ImageId, PaletteId};
use crate::surface_cache::{IndexedTexture, PaletteLighting};
use qa_content::vfs::{Vfs, VfsError};
use qa_formats::FormatError;
use qa_formats::archive::ArchiveReader;
use qa_formats::image::{
    Colormap, DecodedImage, ImageFormat, MipTexture, Palette, PaletteOptions, RasterPolicy, decode,
};

#[derive(Debug)]
pub enum ResourceError {
    Missing(String),
    Vfs(VfsError),
    Format(FormatError),
    Size,
    ShortRead,
    Asset(&'static str),
}
impl From<VfsError> for ResourceError {
    fn from(error: VfsError) -> Self {
        Self::Vfs(error)
    }
}
impl From<FormatError> for ResourceError {
    fn from(error: FormatError) -> Self {
        Self::Format(error)
    }
}

pub fn read_file(
    vfs: &Vfs,
    name: &[u8],
    reader: &mut ArchiveReader,
) -> Result<Vec<u8>, ResourceError> {
    let file = vfs
        .open(name)
        .ok_or_else(|| ResourceError::Missing(String::from_utf8_lossy(name).into_owned()))?;
    let size = usize::try_from(vfs.length(file)?).map_err(|_| ResourceError::Size)?;
    if size > 256 * 1024 * 1024 {
        return Err(ResourceError::Size);
    }
    let mut bytes = vec![0; size];
    if vfs.read_into_reusing(file, &mut bytes, reader)? != size {
        return Err(ResourceError::ShortRead);
    }
    Ok(bytes)
}

/// These are resource formats, not renderer pipelines. The selected palette
/// can later present any world, independently of its movement/module family.
pub enum PaletteSource {
    Lmp {
        colors: &'static [u8],
        shades: &'static [u8],
    },
    Pcx {
        image: &'static [u8],
    },
}
pub fn load_palette(
    vfs: &Vfs,
    assets: &mut Assets,
    source: PaletteSource,
) -> Result<PaletteId, ResourceError> {
    let mut reader = ArchiveReader::default();
    let palette = match source {
        PaletteSource::Lmp { colors, shades } => {
            let colors = read_file(vfs, colors, &mut reader)?;
            let shades = read_file(vfs, shades, &mut reader)?;
            let parsed = Colormap::parse(&shades)?;
            PaletteLighting::load(&colors, &shades[..64 * 256], None, parsed.first_fullbright)
                .map_err(ResourceError::Asset)?
        }
        PaletteSource::Pcx { image } => {
            let bytes = read_file(vfs, image, &mut reader)?;
            let DecodedImage::Indexed(decoded) =
                decode(&bytes, ImageFormat::Pcx, RasterPolicy::Quake2)?
            else {
                return Err(ResourceError::Asset("palette PCX must be indexed"));
            };
            if decoded.width != 256 || decoded.height != 320 {
                return Err(ResourceError::Asset(
                    "invalid software shade/translucency layout",
                ));
            }
            let colors = decoded
                .palette
                .ok_or(ResourceError::Asset("palette PCX has no RGB palette"))?;
            let rgb: Vec<_> = colors
                .0
                .iter()
                .flat_map(|color| color[..3].iter().copied())
                .collect();
            PaletteLighting::load(
                &rgb,
                &decoded.indices[..64 * 256],
                Some(&decoded.indices[64 * 256..]),
                256,
            )
            .map_err(ResourceError::Asset)?
        }
    };
    assets
        .register_palette(palette)
        .map_err(ResourceError::Asset)
}

/// Canonical file names form a sorted load index. No content fingerprints or
/// strings participate in a frame lookup. Different palettes stay distinct.
pub struct Images<'a> {
    pub assets: &'a mut Assets,
    vfs: &'a Vfs,
    reader: ArchiveReader,
    resolved: Vec<(String, ImageId)>,
}
impl<'a> Images<'a> {
    pub fn new(vfs: &'a Vfs, assets: &'a mut Assets) -> Self {
        Self {
            assets,
            vfs,
            reader: ArchiveReader::default(),
            resolved: Vec::new(),
        }
    }
    pub fn embedded(
        &mut self,
        mip: &MipTexture<'_>,
        palette: PaletteId,
        cutout: bool,
    ) -> Result<ImageId, ResourceError> {
        let texture = IndexedTexture::load(mip.width, mip.height, mip.levels, cutout)
            .map_err(ResourceError::Asset)?;
        self.assets
            .register_indexed_image(texture, palette)
            .map_err(ResourceError::Asset)
    }
    pub fn wal(
        &mut self,
        name: &str,
        palette: PaletteId,
        cutout: bool,
    ) -> Result<ImageId, ResourceError> {
        let key = format!("wal:{name}:{}:{cutout}", palette.0);
        if let Ok(index) = self.resolved.binary_search_by(|entry| entry.0.cmp(&key)) {
            return Ok(self.resolved[index].1);
        }
        let bytes = read_file(self.vfs, name.as_bytes(), &mut self.reader)?;
        let mip = MipTexture::parse(&bytes, qa_formats::image::MipFormat::Wal)?;
        let id = self.embedded(&mip, palette, cutout)?;
        self.remember(key, id);
        Ok(id)
    }
    /// Retain original PCX indices and select the presentation's global palette.
    /// A separate native RGBA resource can supply GL pixels (Q2 sky TGA) without
    /// quantizing it or replacing the software resource's dimensions.
    pub fn indexed_pcx(
        &mut self,
        name: &str,
        palette: PaletteId,
        transparent_index: Option<u8>,
        rgba_override: Option<&str>,
    ) -> Result<ImageId, ResourceError> {
        let key = format!(
            "pcx:{name}:{}:{transparent_index:?}:{rgba_override:?}",
            palette.0
        );
        if let Ok(index) = self.resolved.binary_search_by(|entry| entry.0.cmp(&key)) {
            return Ok(self.resolved[index].1);
        }
        let bytes = read_file(self.vfs, name.as_bytes(), &mut self.reader)?;
        let DecodedImage::Indexed(decoded) =
            decode(&bytes, ImageFormat::Pcx, RasterPolicy::Quake2)?
        else {
            return Err(ResourceError::Asset("PCX resource must retain indices"));
        };
        let texture = IndexedTexture::load_base(
            decoded.width,
            decoded.height,
            &decoded.indices,
            transparent_index,
        )
        .map_err(ResourceError::Asset)?;
        let id = if let Some(path) = rgba_override {
            let format = ImageFormat::from_path(path.as_bytes())
                .ok_or(ResourceError::Asset("unknown RGBA override format"))?;
            let bytes = read_file(self.vfs, path.as_bytes(), &mut self.reader)?;
            let DecodedImage::Rgba(rgba) = decode(&bytes, format, RasterPolicy::Quake2)? else {
                return Err(ResourceError::Asset(
                    "RGBA override must have original color pixels",
                ));
            };
            self.assets
                .register_rgba_with_indexed(rgba.width, rgba.height, &rgba.pixels, texture)
        } else {
            self.assets.register_indexed_image(texture, palette)
        }
        .map_err(ResourceError::Asset)?;
        self.remember(key, id);
        Ok(id)
    }
    /// Original Q3 lookup tries TGA, then JPEG for a missing TGA. Explicit
    /// supported extensions are retained; newer formats use the same decoder.
    pub fn raster(&mut self, name: &str, policy: RasterPolicy) -> Result<ImageId, ResourceError> {
        let canonical = name.replace('\\', "/").to_ascii_lowercase();
        let key = format!("raster:{policy:?}:{canonical}");
        if let Ok(index) = self.resolved.binary_search_by(|entry| entry.0.cmp(&key)) {
            return Ok(self.resolved[index].1);
        }
        let known = ImageFormat::from_path(canonical.as_bytes());
        let candidates = if known.is_some() {
            let jpeg = canonical
                .strip_suffix(".tga")
                .map(|stem| format!("{stem}.jpg"));
            [Some(canonical.clone()), jpeg]
        } else {
            [
                Some(format!("{canonical}.tga")),
                Some(format!("{canonical}.jpg")),
            ]
        };
        let mut selected = None;
        for path in candidates.into_iter().flatten() {
            if self.vfs.open(path.as_bytes()).is_some() {
                selected = Some(path);
                break;
            }
        }
        let path = selected.ok_or(ResourceError::Missing(canonical))?;
        let bytes = read_file(self.vfs, path.as_bytes(), &mut self.reader)?;
        let format = ImageFormat::from_path(path.as_bytes())
            .ok_or(ResourceError::Asset("unknown image format"))?;
        // Native Q2 pics/skins ignore their embedded PCX palette and retain
        // indices for the selected global palette. Generic RGBA expansion
        // cannot preserve that contract; callers need an indexed-PCX path.
        if policy == RasterPolicy::Quake2 && format == ImageFormat::Pcx {
            return Err(ResourceError::Asset(
                "Q2 PCX requires indexed palette registration",
            ));
        }
        let rgba = match decode(&bytes, format, policy)? {
            DecodedImage::Rgba(image) => image,
            DecodedImage::Indexed(image) => {
                let palette: &Palette = image
                    .palette
                    .as_ref()
                    .ok_or(ResourceError::Asset("raster has no palette"))?;
                image.expand(palette, &PaletteOptions::default())?
            }
        };
        let id = self
            .assets
            .register_image(rgba.width, rgba.height, &rgba.pixels)
            .map_err(ResourceError::Asset)?;
        self.remember(key, id);
        Ok(id)
    }
    fn remember(&mut self, key: String, id: ImageId) {
        let at = self
            .resolved
            .binary_search_by(|entry| entry.0.cmp(&key))
            .unwrap_or_else(|at| at);
        self.resolved.insert(at, (key, id));
    }
}
