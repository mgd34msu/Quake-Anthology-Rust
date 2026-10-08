//! Cold VFS image/palette resolution. Frames retain numeric asset handles.
use crate::assets::{Assets, ImageId, PaletteId, Sampler, upload};
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
    Upload(upload::UploadError),
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
impl From<upload::UploadError> for ResourceError {
    fn from(error: upload::UploadError) -> Self {
        Self::Upload(error)
    }
}

/// Raw load settings selected from the world's cvar source, independently of
/// the client's movement/modules. Gamma is already an exponent: legacy
/// vid_gamma, or the reciprocal of canonical r_gamma.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ImageSettings {
    pub gamma_exponent: f32,
    /// GLQuake's startup palette correction; its gamma cvar is separate.
    pub palette_exponent: f32,
    pub intensity: f32,
    pub picmip: u8,
    pub round_images_down: bool,
    pub simple_mipmaps: bool,
    /// Bounded cold maximum, not a claim about the current driver's limit.
    pub max_dimension: u32,
    /// Native Q2 gl_skymip affects sky UV bounds, not its upload mip chain.
    pub sky_mip: bool,
}
impl ImageSettings {
    pub fn native(family: u8) -> Self {
        Self {
            gamma_exponent: 1.0,
            palette_exponent: if family == 1 { 0.7 } else { 1.0 },
            intensity: if family == 2 { 2.0 } else { 1.0 },
            picmip: u8::from(family == 3),
            round_images_down: family != 1,
            simple_mipmaps: true,
            max_dimension: match family {
                1 => 1024,
                2 => 256,
                _ => upload::MAX_DIMENSION,
            },
            sky_mip: false,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImageRole {
    Surface,
    Pic,
    LayeredSky,
    CubeSky,
    Lightmap,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ImageUse {
    pub role: ImageRole,
    pub sampler: Sampler,
    pub allow_picmip: bool,
}
impl Default for ImageUse {
    fn default() -> Self {
        Self {
            role: ImageRole::Surface,
            sampler: Sampler::default(),
            allow_picmip: true,
        }
    }
}
impl ImageUse {
    pub fn pic() -> Self {
        Self {
            role: ImageRole::Pic,
            sampler: Sampler {
                mipmaps: false,
                ..Sampler::default()
            },
            allow_picmip: false,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ResolvedImage {
    pub id: ImageId,
    /// Native first-registration flags own the effective image sampler.
    pub sampler: Sampler,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImageConflict {
    pub name: String,
    pub first: ImageUse,
    pub requested: ImageUse,
}

struct ImageRecipes {
    policy: RasterPolicy,
    settings: ImageSettings,
    gamma: upload::RgbLut,
    light: upload::LightScale,
}
impl ImageRecipes {
    fn new(policy: RasterPolicy, settings: ImageSettings) -> Result<Self, ResourceError> {
        if settings.picmip >= 32
            || settings.max_dimension == 0
            || settings.max_dimension > upload::MAX_DIMENSION
        {
            return Err(upload::UploadError::Extent.into());
        }
        let gamma = upload::gamma_lut(
            if policy == RasterPolicy::Standard {
                settings.palette_exponent
            } else {
                settings.gamma_exponent
            },
            match policy {
                RasterPolicy::Standard => upload::GammaCurve::PalettePower,
                RasterPolicy::Quake2 => upload::GammaCurve::HalfPixelPower,
                RasterPolicy::Quake3 => upload::GammaCurve::BytePower,
            },
        )?;
        let light = upload::light_scale(gamma, settings.intensity)?;
        Ok(Self {
            policy,
            settings,
            gamma,
            light,
        })
    }
    fn effective_use(&self, mut usage: ImageUse) -> ImageUse {
        if matches!(
            usage.role,
            ImageRole::Pic | ImageRole::LayeredSky | ImageRole::Lightmap
        ) || (self.policy == RasterPolicy::Quake2 && usage.role == ImageRole::CubeSky)
        {
            usage.sampler.mipmaps = false;
            usage.allow_picmip = false;
        }
        usage
    }
    fn params(&self, usage: ImageUse) -> Result<upload::UploadParams, ResourceError> {
        use upload::{ColorOrder, ExtentRound, MipmapBuild, ResizeFilter, UploadExtent};
        let usage = self.effective_use(usage);
        if matches!(usage.role, ImageRole::Lightmap | ImageRole::LayeredSky) {
            return Ok(upload::UploadParams::default());
        }
        let mipmaps = usage.sampler.mipmaps;
        let settings = self.settings;
        let round = if settings.round_images_down {
            ExtentRound::Down
        } else {
            ExtentRound::Up
        };
        let (extent, resize, kernel, order, rgb_lut, inverse_intensity) = match self.policy {
            RasterPolicy::Standard => (
                UploadExtent::PowerOfTwo {
                    round: ExtentRound::Up,
                    // GLQuake applies gl_picmip even to non-mipmapped pics.
                    drop: settings.picmip,
                    max_dimension: settings.max_dimension,
                },
                ResizeFilter::Nearest,
                MipmapBuild::LegacyBox,
                ColorOrder::BeforeResize,
                self.light.rgb_lut,
                self.light.inverse_intensity,
            ),
            RasterPolicy::Quake2 => (
                UploadExtent::PowerOfTwo {
                    round: if mipmaps { round } else { ExtentRound::Up },
                    drop: if mipmaps && usage.allow_picmip {
                        settings.picmip
                    } else {
                        0
                    },
                    max_dimension: settings.max_dimension,
                },
                ResizeFilter::FourTap,
                MipmapBuild::LegacyBox,
                if mipmaps {
                    ColorOrder::AfterResize
                } else {
                    ColorOrder::AfterResizeIfChanged
                },
                if mipmaps {
                    self.light.rgb_lut
                } else {
                    self.gamma
                },
                if mipmaps {
                    self.light.inverse_intensity
                } else {
                    1.0
                },
            ),
            RasterPolicy::Quake3 => {
                let kernel = if settings.simple_mipmaps {
                    MipmapBuild::Box
                } else {
                    MipmapBuild::Weighted
                };
                if kernel == MipmapBuild::Weighted {
                    return Err(upload::UploadError::WeightedMipUnsupported.into());
                }
                (
                    UploadExtent::PowerOfTwoMip {
                        round,
                        drop: if usage.allow_picmip {
                            settings.picmip
                        } else {
                            0
                        },
                        max_dimension: settings.max_dimension,
                        kernel,
                    },
                    ResizeFilter::FourTap,
                    kernel,
                    if mipmaps {
                        ColorOrder::AfterResize
                    } else {
                        ColorOrder::AfterResizeIfChanged
                    },
                    if mipmaps {
                        self.light.rgb_lut
                    } else {
                        self.gamma
                    },
                    if mipmaps {
                        self.light.inverse_intensity
                    } else {
                        1.0
                    },
                )
            }
        };
        Ok(upload::UploadParams {
            extent,
            resize,
            mipmaps: if mipmaps { kernel } else { MipmapBuild::None },
            color_order: order,
            rgb_lut,
            inverse_intensity,
        })
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
    recipes: ImageRecipes,
    resolved: Vec<(String, ResolvedImage, ImageUse)>,
    pub conflicts: Vec<ImageConflict>,
}
impl<'a> Images<'a> {
    pub fn new(
        vfs: &'a Vfs,
        assets: &'a mut Assets,
        policy: RasterPolicy,
        settings: ImageSettings,
    ) -> Result<Self, ResourceError> {
        Ok(Self {
            assets,
            vfs,
            reader: ArchiveReader::default(),
            recipes: ImageRecipes::new(policy, settings)?,
            resolved: Vec::new(),
            conflicts: Vec::new(),
        })
    }
    pub fn upload_params(&self, usage: ImageUse) -> Result<upload::UploadParams, ResourceError> {
        self.recipes.params(usage)
    }
    /// Correct palette RGB before sky splitting/averaging, as R_InitSky reads
    /// the already-corrected d_8to24table rather than correcting its average.
    pub fn corrected_palette(&self, palette: PaletteId) -> Result<[[u8; 4]; 256], ResourceError> {
        let colors = self
            .assets
            .palette(palette)
            .ok_or(ResourceError::Asset("missing image palette"))?;
        Ok(std::array::from_fn(|index| {
            let mut color = colors.color(index as u8).to_le_bytes();
            for channel in &mut color[..3] {
                *channel = self.recipes.light.rgb_lut.0[*channel as usize];
            }
            // GLQuake's d_8to24table clears alpha for palette index 255.
            if index == 255 {
                color[3] = 0;
            }
            color
        }))
    }
    pub fn embedded(
        &mut self,
        mip: &MipTexture<'_>,
        palette: PaletteId,
        cutout: bool,
    ) -> Result<ImageId, ResourceError> {
        let texture = IndexedTexture::load(mip.width, mip.height, mip.levels, cutout)
            .map_err(ResourceError::Asset)?;
        let gl_pixels = if self.recipes.policy == RasterPolicy::Quake2 {
            let colors = self
                .assets
                .palette(palette)
                .ok_or(ResourceError::Asset("missing image palette"))?;
            let rgba = std::array::from_fn(|index| colors.color(index as u8).to_le_bytes());
            Some(upload::expand_indexed(
                mip.levels[0],
                mip.width,
                mip.height,
                &rgba,
                Some(255),
                upload::AlphaFringe::NativeNeighbors,
            )?)
        } else {
            None
        };
        let id = self
            .assets
            .register_indexed_image(texture, palette)
            .map_err(ResourceError::Asset)?;
        self.prepare(id, ImageUse::default(), gl_pixels.as_deref())?;
        Ok(id)
    }
    pub fn wal(
        &mut self,
        name: &str,
        palette: PaletteId,
        cutout: bool,
    ) -> Result<ImageId, ResourceError> {
        let key = format!("wal:{name}:{}:{cutout}", palette.0);
        if let Ok(index) = self.resolved.binary_search_by(|entry| entry.0.cmp(&key)) {
            return Ok(self.resolved[index].1.id);
        }
        let bytes = read_file(self.vfs, name.as_bytes(), &mut self.reader)?;
        let mip = MipTexture::parse(&bytes, qa_formats::image::MipFormat::Wal)?;
        let id = self.embedded(&mip, palette, cutout)?;
        self.remember(key, id, ImageUse::default());
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
        usage: ImageUse,
    ) -> Result<ImageId, ResourceError> {
        let key = format!(
            "pcx:{name}:{}:{transparent_index:?}:{rgba_override:?}:{usage:?}",
            palette.0
        );
        if let Ok(index) = self.resolved.binary_search_by(|entry| entry.0.cmp(&key)) {
            return Ok(self.resolved[index].1.id);
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
        let mut gl_pixels = None;
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
            let colors = self
                .assets
                .palette(palette)
                .ok_or(ResourceError::Asset("missing image palette"))?;
            let rgba = std::array::from_fn(|index| colors.color(index as u8).to_le_bytes());
            gl_pixels = Some(upload::expand_indexed(
                &decoded.indices,
                decoded.width,
                decoded.height,
                &rgba,
                Some(255),
                upload::AlphaFringe::NativeNeighbors,
            )?);
            self.assets.register_indexed_image(texture, palette)
        }
        .map_err(ResourceError::Asset)?;
        self.prepare(id, usage, gl_pixels.as_deref())?;
        self.remember(key, id, usage);
        Ok(id)
    }
    /// Original Q3 lookup tries TGA, then JPEG for a missing TGA. Explicit
    /// supported extensions are retained; newer formats use the same decoder.
    pub fn raster(&mut self, name: &str, usage: ImageUse) -> Result<ResolvedImage, ResourceError> {
        let policy = self.recipes.policy;
        let canonical = name.replace('\\', "/").to_ascii_lowercase();
        let key = format!("raster:{policy:?}:{canonical}");
        if let Ok(index) = self.resolved.binary_search_by(|entry| entry.0.cmp(&key)) {
            let first = self.resolved[index].2;
            let requested = self.recipes.effective_use(usage);
            if policy == RasterPolicy::Quake3
                && (first.sampler != requested.sampler
                    || first.allow_picmip != requested.allow_picmip)
                && !self
                    .conflicts
                    .iter()
                    .any(|entry| entry.name == canonical && entry.requested == requested)
            {
                self.conflicts.push(ImageConflict {
                    name: canonical,
                    first,
                    requested,
                });
            }
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
        self.prepare(id, usage, None)?;
        let resolved = self.remember(key, id, usage);
        Ok(resolved)
    }
    fn prepare(
        &mut self,
        id: ImageId,
        usage: ImageUse,
        rgba: Option<&[u8]>,
    ) -> Result<(), ResourceError> {
        let params = self.recipes.params(usage)?;
        match rgba {
            Some(rgba) => self.assets.prepare_image_with_rgba(id, rgba, params),
            None => self.assets.prepare_image(id, params),
        }
        .map_err(ResourceError::Asset)?;
        if self.recipes.policy == RasterPolicy::Quake3 {
            self.assets
                .set_image_native_sampler(id, self.recipes.effective_use(usage).sampler)
                .map_err(ResourceError::Asset)?;
        }
        Ok(())
    }
    fn remember(&mut self, key: String, id: ImageId, usage: ImageUse) -> ResolvedImage {
        let usage = self.recipes.effective_use(usage);
        let resolved = ResolvedImage {
            id,
            sampler: usage.sampler,
        };
        let at = self
            .resolved
            .binary_search_by(|entry| entry.0.cmp(&key))
            .unwrap_or_else(|at| at);
        self.resolved.insert(at, (key, resolved, usage));
        resolved
    }
}
