//! Cold VFS image/palette resolution. Frames retain numeric asset handles.
use crate::assets::{Assets, ImageId, PaletteId, Sampler, upload};
use crate::surface_cache::{IndexedTexture, PaletteLighting};
use qa_content::vfs::{Vfs, VfsError};
use qa_core::primitives::{NameId, RuleSetId};
use qa_formats::FormatError;
use qa_formats::archive::ArchiveReader;
use qa_formats::image::{
    AlphaFringe, Colormap, DecodedImage, ImageFormat, IndexedImage, MipTexture, Palette,
    PaletteOptions, RgbaImage, Wad, WadImage, decode,
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

/// Raw load settings selected from the presentation's cvar source, independently of
/// the client's movement/modules. Gamma is already an exponent: legacy
/// vid_gamma, or the reciprocal of canonical r_gamma.
#[derive(Clone, Copy, Debug)]
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
impl PartialEq for ImageSettings {
    fn eq(&self, other: &Self) -> bool {
        // These are exact registration policies. Invalid float payloads must
        // still resolve to their own constructor diagnostic, never a fallback.
        self.gamma_exponent.to_bits() == other.gamma_exponent.to_bits()
            && self.palette_exponent.to_bits() == other.palette_exponent.to_bits()
            && self.intensity.to_bits() == other.intensity.to_bits()
            && self.picmip == other.picmip
            && self.round_images_down == other.round_images_down
            && self.simple_mipmaps == other.simple_mipmaps
            && self.max_dimension == other.max_dimension
            && self.sky_mip == other.sky_mip
    }
}
impl ImageSettings {
    pub fn native(source: RuleSetId) -> Self {
        let q1 = matches!(source, RuleSetId::Quake | RuleSetId::QuakeWorld);
        let q2 = matches!(source, RuleSetId::Quake2 | RuleSetId::Quake2Rerelease);
        Self {
            gamma_exponent: 1.0,
            palette_exponent: if q1 { 0.7 } else { 1.0 },
            intensity: if q2 { 2.0 } else { 1.0 },
            picmip: u8::from(source == RuleSetId::Quake3),
            round_images_down: !q1,
            simple_mipmaps: true,
            max_dimension: match source {
                RuleSetId::Quake | RuleSetId::QuakeWorld => 1024,
                RuleSetId::Quake2 | RuleSetId::Quake2Rerelease => 256,
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
    pub name: NameId,
    pub first: ImageUse,
    pub requested: ImageUse,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ImageRecipe {
    MissingSkin {
        palette: Option<PaletteId>,
    },
    Lmp {
        palette: PaletteId,
        transparent_index: Option<u8>,
        usage: ImageUse,
    },
    Wad {
        lump: NameId,
        palette: PaletteId,
        transparent_index: Option<u8>,
        usage: ImageUse,
    },
    Wal {
        palette: PaletteId,
        cutout: bool,
    },
    Pcx {
        palette: PaletteId,
        transparent_index: Option<u8>,
        rgba_override: Option<NameId>,
        usage: ImageUse,
    },
    Raster {
        rules: RuleSetId,
        indexed: Option<IndexedTarget>,
    },
}

/// Software colors are converted to this explicit presentation palette while
/// the image retains its source RGBA for GL. Masks reserve one palette entry;
/// opaque pixels never select it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IndexedTarget {
    pub palette: PaletteId,
    pub transparent_index: Option<u8>,
}
#[derive(Clone, Copy, PartialEq, Eq)]
struct ImageKey {
    name: NameId,
    recipe: ImageRecipe,
}

struct IndexedRemap {
    source: PaletteId,
    target: PaletteId,
    maps: Vec<(Option<u8>, [u8; 256])>,
}

fn palette_index(colors: &PaletteLighting, rgba: [u8; 4], transparent: Option<u8>) -> u8 {
    if rgba[3] == 0
        && let Some(index) = transparent
    {
        index
    } else {
        colors.nearest_index([rgba[0], rgba[1], rgba[2]], transparent)
    }
}

fn rgba_texture(
    rgba: &RgbaImage,
    colors: &PaletteLighting,
    transparent_index: Option<u8>,
) -> Result<IndexedTexture, ResourceError> {
    let indices: Vec<_> = rgba
        .pixels
        .chunks_exact(4)
        .map(|color| {
            palette_index(
                colors,
                [color[0], color[1], color[2], color[3]],
                transparent_index,
            )
        })
        .collect();
    IndexedTexture::load_base(rgba.width, rgba.height, &indices, transparent_index)
        .map_err(ResourceError::Asset)
}

struct ImageRecipes {
    rules: RuleSetId,
    settings: ImageSettings,
    gamma: upload::RgbLut,
    light: upload::LightScale,
}
impl ImageRecipes {
    fn new(rules: RuleSetId, settings: ImageSettings) -> Result<Self, ResourceError> {
        if settings.picmip >= 32
            || settings.max_dimension == 0
            || settings.max_dimension > upload::MAX_DIMENSION
        {
            return Err(upload::UploadError::Extent.into());
        }
        let gamma = upload::gamma_lut(
            if matches!(rules, RuleSetId::Quake | RuleSetId::QuakeWorld) {
                settings.palette_exponent
            } else {
                settings.gamma_exponent
            },
            match rules {
                RuleSetId::Quake | RuleSetId::QuakeWorld => upload::GammaCurve::PalettePower,
                RuleSetId::Quake2 | RuleSetId::Quake2Rerelease => {
                    upload::GammaCurve::HalfPixelPower
                }
                RuleSetId::Quake3 => upload::GammaCurve::BytePower,
            },
        )?;
        let light = upload::light_scale(gamma, settings.intensity)?;
        Ok(Self {
            rules,
            settings,
            gamma,
            light,
        })
    }
    fn effective_use(&self, mut usage: ImageUse) -> ImageUse {
        if matches!(
            usage.role,
            ImageRole::Pic | ImageRole::LayeredSky | ImageRole::Lightmap
        ) || (matches!(self.rules, RuleSetId::Quake2 | RuleSetId::Quake2Rerelease)
            && usage.role == ImageRole::CubeSky)
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
        let (extent, resize, kernel, order, rgb_lut, inverse_intensity) = match self.rules {
            RuleSetId::Quake | RuleSetId::QuakeWorld => (
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
            RuleSetId::Quake2 | RuleSetId::Quake2Rerelease => (
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
            RuleSetId::Quake3 => {
                let kernel = if settings.simple_mipmaps {
                    MipmapBuild::Box
                } else {
                    MipmapBuild::Weighted
                };
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
    read_file_with_limit(vfs, name, 256 * 1024 * 1024, reader)
}
fn read_file_with_limit(
    vfs: &Vfs,
    name: &[u8],
    limit: usize,
    reader: &mut ArchiveReader,
) -> Result<Vec<u8>, ResourceError> {
    vfs.read_file(name, limit, reader)
        .map_err(|error| match error {
            VfsError::Missing => ResourceError::Missing(String::from_utf8_lossy(name).into_owned()),
            VfsError::Size | VfsError::Limit | VfsError::Allocation => ResourceError::Size,
            VfsError::ShortRead => ResourceError::ShortRead,
            error => ResourceError::Vfs(error),
        })
}

const INDEXED_PIC_FILE_LIMIT: usize = 16 * 1024 * 1024;

/// These are resource formats, not renderer pipelines. The selected palette
/// can later present any world, independently of its movement/module family.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PaletteSource {
    Lmp {
        colors: &'static [u8],
        shades: &'static [u8],
    },
    Pcx {
        image: &'static [u8],
    },
}
impl PaletteSource {
    pub fn native(rules: RuleSetId) -> Option<Self> {
        match rules {
            RuleSetId::Quake | RuleSetId::QuakeWorld => Some(Self::Lmp {
                colors: b"gfx/palette.lmp",
                shades: b"gfx/colormap.lmp",
            }),
            RuleSetId::Quake2 | RuleSetId::Quake2Rerelease => Some(Self::Pcx {
                image: b"pics/colormap.pcx",
            }),
            RuleSetId::Quake3 => None,
        }
    }
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
                decode(&bytes, ImageFormat::Pcx, RuleSetId::Quake2.lineage())?
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
/// The existing recipe cache can outlive a temporary borrow of cold Assets.
pub struct ImagesState {
    reader: ArchiveReader,
    recipes: ImageRecipes,
    source_rules: RuleSetId,
    indexed_remap: Option<IndexedRemap>,
    raster_target: Option<IndexedTarget>,
    resolved: Vec<(ImageKey, ResolvedImage, ImageUse)>,
    conflicts: Vec<ImageConflict>,
}
#[derive(Clone, Copy)]
pub struct ImageRegistrationMark {
    conflicts: usize,
}
impl ImagesState {
    pub fn registration_mark(&self) -> ImageRegistrationMark {
        ImageRegistrationMark {
            conflicts: self.conflicts.len(),
        }
    }

    /// The name-sorted cache can insert anywhere, so retain by resource
    /// identity rather than truncating it to its previous number of entries.
    pub fn rollback_registration(&mut self, mark: ImageRegistrationMark, images: usize) {
        self.resolved
            .retain(|entry| (entry.1.id.0 as usize) < images);
        self.conflicts.truncate(mark.conflicts);
    }

    pub fn new(
        source_rules: RuleSetId,
        presentation_rules: RuleSetId,
        settings: ImageSettings,
        indexed_palettes: Option<(PaletteId, PaletteId)>,
    ) -> Result<Self, ResourceError> {
        Ok(Self {
            reader: ArchiveReader::default(),
            recipes: ImageRecipes::new(presentation_rules, settings)?,
            source_rules,
            indexed_remap: indexed_palettes
                .filter(|(source, target)| source != target)
                .map(|(source, target)| IndexedRemap {
                    source,
                    target,
                    maps: Vec::new(),
                }),
            raster_target: None,
            resolved: Vec::new(),
            conflicts: Vec::new(),
        })
    }
}
pub struct Images<'a> {
    pub assets: &'a mut Assets,
    vfs: &'a Vfs,
    state: ImagesState,
}
impl<'a> Images<'a> {
    /// Decode using source provenance, then upload with the presentation role.
    /// An optional source/target palette pair remaps software indices at load;
    /// native equal pairs retain exact original bytes and source RGBA.
    pub fn new(
        vfs: &'a Vfs,
        assets: &'a mut Assets,
        source_rules: RuleSetId,
        presentation_rules: RuleSetId,
        settings: ImageSettings,
        indexed_palettes: Option<(PaletteId, PaletteId)>,
    ) -> Result<Self, ResourceError> {
        Ok(Self::from_state(
            vfs,
            assets,
            ImagesState::new(source_rules, presentation_rules, settings, indexed_palettes)?,
        ))
    }
    pub fn from_state(vfs: &'a Vfs, assets: &'a mut Assets, state: ImagesState) -> Self {
        Self { assets, vfs, state }
    }
    pub fn into_state(self) -> ImagesState {
        self.state
    }
    pub fn conflicts(&self) -> &[ImageConflict] {
        &self.state.conflicts
    }
    /// The cold profile chooses software quantization independently of the
    /// decoder's source rules and source palette.
    pub fn set_raster_target(
        &mut self,
        target: Option<IndexedTarget>,
    ) -> Result<(), ResourceError> {
        if target.is_some_and(|target| self.assets.palette(target.palette).is_none()) {
            return Err(ResourceError::Asset("missing presentation image palette"));
        }
        if !self.state.resolved.is_empty() && target != self.state.raster_target {
            return Err(ResourceError::Asset("image target already registered"));
        }
        self.state.raster_target = target;
        Ok(())
    }
    pub fn raster_target(&self) -> Option<IndexedTarget> {
        self.state.raster_target
    }
    /// Native alias fallback: ref_gl's opaque red dot image and ref_soft's
    /// 0/255 checker retain independent representations in the same resource.
    pub fn missing_skin(&mut self, palette: Option<PaletteId>) -> Result<ImageId, ResourceError> {
        let key = ImageKey {
            name: self
                .assets
                .intern_name("*missing-skin")
                .map_err(ResourceError::Asset)?,
            recipe: ImageRecipe::MissingSkin { palette },
        };
        if let Some(index) = self.find(key) {
            return Ok(self.state.resolved[index].1.id);
        }
        // quake-2/ref_gl/gl_rmisc.c R_InitParticleTexture uses this 4x4
        // portion of dottexture, indexed [x&3][y&3], for r_notexture.
        const DOT: [[u8; 4]; 4] = [[0, 0, 0, 0], [0, 0, 1, 1], [0, 1, 1, 1], [0, 1, 1, 1]];
        let mut rgba = [0; 8 * 8 * 4];
        for (index, pixel) in rgba.chunks_exact_mut(4).enumerate() {
            let (x, y) = (index % 8, index / 8);
            pixel.copy_from_slice(&[DOT[x & 3][y & 3] * 255, 0, 0, 255]);
        }
        let id = if let Some(palette) = palette {
            // ref_soft/r_main.c R_InitTextures preserves four checker mips.
            let mips: [Vec<u8>; 4] = std::array::from_fn(|mip| {
                let width = 16 >> mip;
                (0..width * width)
                    .map(|index| {
                        if (index / width < width / 2) ^ (index % width < width / 2) {
                            0
                        } else {
                            255
                        }
                    })
                    .collect()
            });
            let texture = IndexedTexture::load(16, 16, mips.each_ref().map(Vec::as_slice), false)
                .map_err(ResourceError::Asset)?;
            self.register_indexed_rgba(8, 8, &rgba, texture, palette)?
        } else {
            self.assets
                .register_image(8, 8, &rgba)
                .map_err(ResourceError::Asset)?
        };
        let usage = ImageUse::default();
        self.prepare(id, usage, None)?;
        self.remember(key, id, usage);
        Ok(id)
    }
    fn remap_texture(
        &mut self,
        texture: IndexedTexture,
        palette: PaletteId,
    ) -> Result<IndexedTexture, ResourceError> {
        let Some(remap) = &mut self.state.indexed_remap else {
            return Ok(texture);
        };
        if palette != remap.source {
            return Ok(texture);
        }
        let transparent = texture.transparent_index();
        let index = match remap.maps.iter().position(|(mask, _)| *mask == transparent) {
            Some(index) => index,
            None => {
                let source = self
                    .assets
                    .palette(remap.source)
                    .ok_or(ResourceError::Asset("missing source image palette"))?;
                let target = self
                    .assets
                    .palette(remap.target)
                    .ok_or(ResourceError::Asset("missing presentation image palette"))?;
                let indices = source.translation_to(target, transparent);
                remap.maps.push((transparent, indices));
                remap.maps.len() - 1
            }
        };
        let translation = &remap.maps[index].1;
        let mips: [Option<Vec<u8>>; 4] = std::array::from_fn(|mip| {
            texture.mip(mip as u8).map(|level| {
                level
                    .indices()
                    .iter()
                    .map(|&index| translation[index as usize])
                    .collect()
            })
        });
        let base = texture
            .mip(0)
            .ok_or(ResourceError::Asset("missing indexed base mip"))?;
        if let [Some(a), Some(b), Some(c), Some(d)] = &mips {
            IndexedTexture::load_masked(base.width, base.height, [a, b, c, d], transparent)
        } else {
            IndexedTexture::load_base(
                base.width,
                base.height,
                mips[0]
                    .as_deref()
                    .ok_or(ResourceError::Asset("missing remapped base mip"))?,
                transparent,
            )
        }
        .map_err(ResourceError::Asset)
    }
    pub fn register_indexed(
        &mut self,
        texture: IndexedTexture,
        palette: PaletteId,
    ) -> Result<ImageId, ResourceError> {
        if self
            .state
            .indexed_remap
            .as_ref()
            .is_none_or(|remap| remap.source != palette)
        {
            return self
                .assets
                .register_indexed_image(texture, palette)
                .map_err(ResourceError::Asset);
        }
        let base = texture
            .mip(0)
            .ok_or(ResourceError::Asset("missing indexed base mip"))?;
        let colors = self
            .assets
            .palette(palette)
            .ok_or(ResourceError::Asset("missing source image palette"))?;
        let rgba = IndexedImage {
            width: base.width,
            height: base.height,
            indices: std::borrow::Cow::Borrowed(base.indices()),
            palette: None,
        }
        .expand(
            &Palette(Box::new(colors.rgba_table())),
            &PaletteOptions {
                transparent_index: texture.transparent_index(),
                ..PaletteOptions::default()
            },
        )?;
        self.register_indexed_rgba(rgba.width, rgba.height, &rgba.pixels, texture, palette)
    }
    /// Source RGBA and selected software indices are independently retained.
    pub fn register_indexed_rgba(
        &mut self,
        width: u32,
        height: u32,
        rgba: &[u8],
        texture: IndexedTexture,
        palette: PaletteId,
    ) -> Result<ImageId, ResourceError> {
        let texture = self.remap_texture(texture, palette)?;
        self.assets
            .register_rgba_with_indexed(width, height, rgba, texture)
            .map_err(ResourceError::Asset)
    }
    pub fn upload_params(&self, usage: ImageUse) -> Result<upload::UploadParams, ResourceError> {
        self.state.recipes.params(usage)
    }
    /// Correct palette RGB before sky splitting/averaging, as R_InitSky reads
    /// the already-corrected d_8to24table rather than correcting its average.
    pub fn corrected_palette(&self, palette: PaletteId) -> Result<[[u8; 4]; 256], ResourceError> {
        let colors = self
            .assets
            .palette(palette)
            .ok_or(ResourceError::Asset("missing image palette"))?;
        let mut table = colors.rgba_table();
        for (index, color) in table.iter_mut().enumerate() {
            for channel in &mut color[..3] {
                *channel = self.state.recipes.light.rgb_lut.0[*channel as usize];
            }
            // GLQuake's d_8to24table clears alpha for palette index 255.
            if index == 255 {
                color[3] = 0;
            }
        }
        Ok(table)
    }

    pub fn embedded(
        &mut self,
        mip: &MipTexture<'_>,
        palette: PaletteId,
        cutout: bool,
    ) -> Result<ImageId, ResourceError> {
        let texture = IndexedTexture::load(mip.width, mip.height, mip.levels, cutout)
            .map_err(ResourceError::Asset)?;
        let gl_pixels = if matches!(
            self.state.recipes.rules,
            RuleSetId::Quake2 | RuleSetId::Quake2Rerelease
        ) {
            let colors = self
                .assets
                .palette(palette)
                .ok_or(ResourceError::Asset("missing image palette"))?;
            let image = IndexedImage {
                width: mip.width,
                height: mip.height,
                indices: std::borrow::Cow::Borrowed(mip.levels[0]),
                palette: None,
            };
            let rgba = image.expand(
                &Palette(Box::new(colors.rgba_table())),
                &PaletteOptions {
                    transparent_index: Some(255),
                    fringe: AlphaFringe::NativeNeighbors,
                    ..PaletteOptions::default()
                },
            )?;
            upload::validate_dimensions(rgba.width, rgba.height)?;
            Some(rgba.pixels)
        } else {
            None
        };
        let id = self.register_indexed(texture, palette)?;
        self.prepare(id, ImageUse::default(), gl_pixels.as_deref())?;
        Ok(id)
    }
    pub fn wal(
        &mut self,
        name: &str,
        palette: PaletteId,
        cutout: bool,
    ) -> Result<ImageId, ResourceError> {
        let key = ImageKey {
            name: self
                .assets
                .intern_name(name)
                .map_err(ResourceError::Asset)?,
            recipe: ImageRecipe::Wal { palette, cutout },
        };
        if let Some(index) = self.find(key) {
            return Ok(self.state.resolved[index].1.id);
        }
        let bytes = read_file(self.vfs, name.as_bytes(), &mut self.state.reader)?;
        let mip = MipTexture::parse(&bytes, qa_formats::image::MipFormat::Wal)?;
        let id = self.embedded(&mip, palette, cutout)?;
        self.remember(key, id, ImageUse::default());
        Ok(id)
    }
    /// Native qpic files retain exact indices and the explicitly selected
    /// global palette. File admission matches the existing console loader.
    pub fn indexed_lmp(
        &mut self,
        name: &str,
        palette: PaletteId,
        transparent_index: Option<u8>,
        usage: ImageUse,
    ) -> Result<ResolvedImage, ResourceError> {
        let key = ImageKey {
            name: self
                .assets
                .intern_name(name)
                .map_err(ResourceError::Asset)?,
            recipe: ImageRecipe::Lmp {
                palette,
                transparent_index,
                usage,
            },
        };
        if let Some(index) = self.find(key) {
            return Ok(self.state.resolved[index].1);
        }
        let bytes = read_file_with_limit(
            self.vfs,
            name.as_bytes(),
            INDEXED_PIC_FILE_LIMIT,
            &mut self.state.reader,
        )?;
        let DecodedImage::Indexed(image) =
            decode(&bytes, ImageFormat::Lmp, self.state.source_rules.lineage())?
        else {
            return Err(ResourceError::Asset("qpic resource must retain indices"));
        };
        self.indexed_pic(key, &image, palette, transparent_index, usage)
    }
    /// WAD lookup uses its native folded lump names. Recipe identities retain
    /// the requested raw bytes separately from the canonical archive path.
    pub fn indexed_wad(
        &mut self,
        name: &str,
        lump: &[u8],
        palette: PaletteId,
        transparent_index: Option<u8>,
        usage: ImageUse,
    ) -> Result<ResolvedImage, ResourceError> {
        let key = ImageKey {
            name: self
                .assets
                .intern_name(name)
                .map_err(ResourceError::Asset)?,
            recipe: ImageRecipe::Wad {
                lump: self
                    .assets
                    .intern_resource_name(lump)
                    .map_err(ResourceError::Asset)?,
                palette,
                transparent_index,
                usage,
            },
        };
        if let Some(index) = self.find(key) {
            return Ok(self.state.resolved[index].1);
        }
        let bytes = read_file_with_limit(
            self.vfs,
            name.as_bytes(),
            INDEXED_PIC_FILE_LIMIT,
            &mut self.state.reader,
        )?;
        let wad = Wad::parse(&bytes)?;
        let index = wad
            .find(lump)
            .ok_or(ResourceError::Asset("missing indexed WAD lump"))?;
        let WadImage::Image(image) = wad.image(index)? else {
            return Err(ResourceError::Asset(
                "WAD resource is not an indexed bitmap",
            ));
        };
        self.indexed_pic(key, &image, palette, transparent_index, usage)
    }
    fn indexed_pic(
        &mut self,
        key: ImageKey,
        image: &IndexedImage<'_>,
        palette: PaletteId,
        transparent_index: Option<u8>,
        usage: ImageUse,
    ) -> Result<ResolvedImage, ResourceError> {
        let texture =
            IndexedTexture::load_base(image.width, image.height, &image.indices, transparent_index)
                .map_err(ResourceError::Asset)?;
        let id = self.register_indexed(texture, palette)?;
        self.prepare(id, usage, None)?;
        Ok(self.remember(key, id, usage))
    }
    /// Retain original PCX indices and select the presentation's global palette.
    /// A separate native RGBA resource supplies GL pixels (Q2 sky TGA). If the
    /// indexed resource is absent, quantize that resource once into the selected
    /// software palette. Native source/presentation pairs keep their exact
    /// original indices; foreign presentation remaps once at registration.
    pub fn indexed_pcx(
        &mut self,
        name: &str,
        palette: PaletteId,
        transparent_index: Option<u8>,
        rgba_override: Option<&str>,
        usage: ImageUse,
    ) -> Result<ImageId, ResourceError> {
        let key = ImageKey {
            name: self
                .assets
                .intern_name(name)
                .map_err(ResourceError::Asset)?,
            recipe: ImageRecipe::Pcx {
                palette,
                transparent_index,
                rgba_override: rgba_override
                    .map(|path| self.assets.intern_name(path))
                    .transpose()
                    .map_err(ResourceError::Asset)?,
                usage,
            },
        };
        if let Some(index) = self.find(key) {
            return Ok(self.state.resolved[index].1.id);
        }
        let bytes = self
            .vfs
            .open(name.as_bytes())
            .map(|_| read_file(self.vfs, name.as_bytes(), &mut self.state.reader))
            .transpose()?;
        let decoded = if let Some(bytes) = &bytes {
            let DecodedImage::Indexed(decoded) =
                decode(bytes, ImageFormat::Pcx, RuleSetId::Quake2.lineage())?
            else {
                return Err(ResourceError::Asset("PCX resource must retain indices"));
            };
            Some(decoded)
        } else {
            None
        };
        let rgba = if let Some(path) =
            rgba_override.filter(|path| self.vfs.open(path.as_bytes()).is_some())
        {
            let format = ImageFormat::from_path(path.as_bytes())
                .ok_or(ResourceError::Asset("unknown RGBA override format"))?;
            let bytes = read_file(self.vfs, path.as_bytes(), &mut self.state.reader)?;
            let DecodedImage::Rgba(rgba) = decode(&bytes, format, RuleSetId::Quake2.lineage())?
            else {
                return Err(ResourceError::Asset(
                    "RGBA override must have original color pixels",
                ));
            };
            Some(rgba)
        } else {
            None
        };
        let texture_palette = if decoded.is_none() {
            self.state
                .indexed_remap
                .as_ref()
                .map_or(palette, |remap| remap.target)
        } else {
            palette
        };
        let colors = self
            .assets
            .palette(texture_palette)
            .ok_or(ResourceError::Asset("missing image palette"))?;
        let texture = if let Some(decoded) = &decoded {
            IndexedTexture::load_base(
                decoded.width,
                decoded.height,
                &decoded.indices,
                transparent_index,
            )
            .map_err(ResourceError::Asset)?
        } else if let Some(rgba) = &rgba {
            rgba_texture(rgba, colors, transparent_index)?
        } else {
            return Err(ResourceError::Missing(name.to_owned()));
        };
        let mut gl_pixels = None;
        let id = if let Some(rgba) = rgba {
            self.register_indexed_rgba(
                rgba.width,
                rgba.height,
                &rgba.pixels,
                texture,
                texture_palette,
            )?
        } else {
            let decoded = decoded.ok_or(ResourceError::Asset("missing indexed image"))?;
            let rgba = decoded.expand(
                &Palette(Box::new(colors.rgba_table())),
                &PaletteOptions {
                    transparent_index: Some(255),
                    fringe: AlphaFringe::NativeNeighbors,
                    ..PaletteOptions::default()
                },
            )?;
            upload::validate_dimensions(rgba.width, rgba.height)?;
            gl_pixels = Some(rgba.pixels);
            self.register_indexed(texture, palette)?
        };
        self.prepare(id, usage, gl_pixels.as_deref())?;
        self.remember(key, id, usage);
        Ok(id)
    }
    /// Original Q3 lookup tries TGA, then JPEG for a missing TGA. Explicit
    /// supported extensions are retained; newer formats use the same decoder.
    pub fn raster(
        &mut self,
        name: &str,
        usage: ImageUse,
        indexed: Option<IndexedTarget>,
    ) -> Result<ResolvedImage, ResourceError> {
        let rules = self.state.source_rules;
        let key = ImageKey {
            name: self
                .assets
                .intern_name(name)
                .map_err(ResourceError::Asset)?,
            recipe: ImageRecipe::Raster { rules, indexed },
        };
        if let Some(index) = self.find(key) {
            let first = self.state.resolved[index].2;
            let requested = self.state.recipes.effective_use(usage);
            if self.state.recipes.rules == RuleSetId::Quake3
                && (first.sampler != requested.sampler
                    || first.allow_picmip != requested.allow_picmip)
                && !self
                    .state
                    .conflicts
                    .iter()
                    .any(|entry| entry.name == key.name && entry.requested == requested)
            {
                self.state.conflicts.push(ImageConflict {
                    name: key.name,
                    first,
                    requested,
                });
            }
            return Ok(self.state.resolved[index].1);
        }
        let canonical = self
            .assets
            .name(key.name)
            .ok_or(ResourceError::Asset("missing image name"))?
            .to_owned();
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
        let bytes = read_file(self.vfs, path.as_bytes(), &mut self.state.reader)?;
        let format = ImageFormat::from_path(path.as_bytes())
            .ok_or(ResourceError::Asset("unknown image format"))?;
        // Native Q2 pics/skins ignore their embedded PCX palette and retain
        // indices for the selected global palette. Generic RGBA expansion
        // cannot preserve that contract; callers need an indexed-PCX path.
        if matches!(rules, RuleSetId::Quake2 | RuleSetId::Quake2Rerelease)
            && format == ImageFormat::Pcx
        {
            return Err(ResourceError::Asset(
                "Q2 PCX requires indexed palette registration",
            ));
        }
        let rgba = match decode(&bytes, format, rules.lineage())? {
            DecodedImage::Rgba(image) => image,
            DecodedImage::Indexed(image) => {
                let palette: &Palette = image
                    .palette
                    .as_ref()
                    .ok_or(ResourceError::Asset("raster has no palette"))?;
                image.expand(palette, &PaletteOptions::default())?
            }
        };
        let id = if let Some(target) = indexed {
            let colors = self
                .assets
                .palette(target.palette)
                .ok_or(ResourceError::Asset("missing presentation image palette"))?;
            let texture = rgba_texture(&rgba, colors, target.transparent_index)?;
            self.assets
                .register_rgba_with_indexed(rgba.width, rgba.height, &rgba.pixels, texture)
        } else {
            self.assets
                .register_image(rgba.width, rgba.height, &rgba.pixels)
        }
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
        let params = self.state.recipes.params(usage)?;
        match rgba {
            Some(rgba) => self.assets.prepare_image_with_rgba(id, rgba, params),
            None => self.assets.prepare_image(id, params),
        }
        .map_err(ResourceError::Asset)?;
        if self.state.recipes.rules == RuleSetId::Quake3 {
            self.assets
                .set_image_native_sampler(id, self.state.recipes.effective_use(usage).sampler)
                .map_err(ResourceError::Asset)?;
        }
        Ok(())
    }
    fn find(&self, key: ImageKey) -> Option<usize> {
        let at = self
            .state
            .resolved
            .partition_point(|entry| entry.0.name.0 < key.name.0);
        self.state.resolved[at..]
            .iter()
            .take_while(|entry| entry.0.name == key.name)
            .position(|entry| entry.0 == key)
            .map(|index| at + index)
    }
    fn remember(&mut self, key: ImageKey, id: ImageId, usage: ImageUse) -> ResolvedImage {
        let usage = self.state.recipes.effective_use(usage);
        let resolved = ResolvedImage {
            id,
            sampler: usage.sampler,
        };
        let at = self
            .state
            .resolved
            .partition_point(|entry| entry.0.name.0 <= key.name.0);
        self.state.resolved.insert(at, (key, resolved, usage));
        resolved
    }
}
