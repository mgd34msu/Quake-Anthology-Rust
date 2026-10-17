//! Cold VFS image/palette resolution. Frames retain numeric asset handles.
use crate::assets::{Assets, ImageId, PaletteId, Sampler, upload};
use crate::surface_cache::{IndexedTexture, PaletteLighting};
use qa_content::vfs::{Vfs, VfsError};
use qa_core::names::file_stem;
use qa_core::primitives::{NameId, RuleSetId};
use qa_formats::FormatError;
use qa_formats::archive::ArchiveReader;
use qa_formats::image::{
    AlphaFringe, Colormap, DecodedImage, ImageFormat, IndexedImage, MipTexture, Palette,
    PaletteOptions, RgbaImage, Wad, WadImage, decode,
};
mod indexed_skin;
mod search;
pub use indexed_skin::{
    IndexedSkinLayout, IndexedSkinSource, IndexedSkinTranslation, IndexedSkinView,
};
pub use search::ImageSearchOrder;
use search::{SelectedFile, SourceFormat};

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

#[cfg(test)]
#[path = "../../tests/resources/layered_sky.rs"]
mod layered_sky_tests;
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
    pub override_textures: i32,
    pub texture_overrides: u32,
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
            && self.override_textures == other.override_textures
            && self.texture_overrides == other.texture_overrides
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
            override_textures: 1,
            texture_overrides: u32::MAX,
        }
    }
}

/// Exact image preparation inputs, projected from a caller's full profile.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ImageProfile {
    pub source_rules: RuleSetId,
    pub presentation_rules: RuleSetId,
    pub source_palette: Option<PaletteId>,
    pub target_palette: Option<PaletteId>,
    pub image_settings: ImageSettings,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImageRole {
    Surface,
    NativeDefault,
    Pic,
    Font,
    Skin,
    Sprite,
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
    TranslatedSkin {
        source: u32,
        decoded: [u32; 2],
        layout: IndexedSkinLayout,
        settings: IndexedSkinTranslation,
    },
    Embedded {
        source: u32,
        palette: PaletteId,
        cutout: bool,
    },
    LayeredSky {
        source: u32,
        palette: PaletteId,
        overlay: bool,
    },
    NativeDefault {
        indexed: Option<IndexedTarget>,
    },
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
        role: Option<u32>,
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
            ImageRole::Pic | ImageRole::Font | ImageRole::LayeredSky | ImageRole::Lightmap
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
                    drop: if usage.role == ImageRole::NativeDefault {
                        0
                    } else {
                        settings.picmip
                    },
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

#[derive(Clone, Copy)]
struct ImageRecord {
    key: ImageKey,
    image: ResolvedImage,
    first_use: ImageUse,
    first_requested: NameId,
}

/// Cold rollback storage for the same image owner; it is never a live lookup cache.
pub struct ImageReload {
    resolved: Vec<ImageRecord>,
    conflicts: Vec<ImageConflict>,
}

/// Canonical file names form a sorted load index. No content fingerprints or
/// strings participate in a frame lookup. Different palettes stay distinct.
/// The existing recipe cache can outlive a temporary borrow of cold Assets.
pub struct ImagesState {
    reader: ArchiveReader,
    recipes: ImageRecipes,
    source_rules: RuleSetId,
    indexed_remap: Option<(PaletteId, PaletteId)>,
    indexed_maps: Vec<((PaletteId, PaletteId, Option<u8>), [u8; 256])>,
    raster_target: Option<IndexedTarget>,
    resolved: Vec<ImageRecord>,
    conflicts: Vec<ImageConflict>,
}
#[derive(Clone, Copy)]
pub struct ImageRegistrationMark {
    conflicts: usize,
}
impl ImagesState {
    /// Start a cold replay while retaining procedural helpers under this same policy.
    /// The receipt must be restored before discarding Assets on rejection.
    pub fn begin_reload(&mut self) -> ImageReload {
        let resolved = std::mem::take(&mut self.resolved);
        self.resolved
            .extend(resolved.iter().copied().filter(|entry| {
                matches!(
                    entry.key.recipe,
                    ImageRecipe::NativeDefault { .. } | ImageRecipe::MissingSkin { .. }
                )
            }));
        ImageReload {
            resolved,
            conflicts: std::mem::take(&mut self.conflicts),
        }
    }

    pub fn restore_reload(&mut self, reload: ImageReload) {
        self.resolved = reload.resolved;
        self.conflicts = reload.conflicts;
    }

    pub fn registration_mark(&self) -> ImageRegistrationMark {
        ImageRegistrationMark {
            conflicts: self.conflicts.len(),
        }
    }

    /// The name-sorted cache can insert anywhere, so retain by resource
    /// identity rather than truncating it to its previous number of entries.
    pub fn rollback_registration(&mut self, mark: ImageRegistrationMark, images: usize) {
        self.resolved
            .retain(|entry| (entry.image.id.0 as usize) < images);
        self.conflicts.truncate(mark.conflicts);
        self.indexed_maps.clear();
    }

    pub(crate) fn clear_conflicts(&mut self) {
        self.conflicts.clear();
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
            indexed_remap: indexed_palettes.filter(|(source, target)| source != target),
            indexed_maps: Vec::new(),
            raster_target: None,
            resolved: Vec::new(),
            conflicts: Vec::new(),
        })
    }
    pub fn set_raster_target(
        &mut self,
        assets: &Assets,
        target: Option<IndexedTarget>,
    ) -> Result<(), ResourceError> {
        if target.is_some_and(|target| assets.palette(target.palette).is_none()) {
            return Err(ResourceError::Asset("missing presentation image palette"));
        }
        if !self.resolved.is_empty() && target != self.raster_target {
            return Err(ResourceError::Asset("image target already registered"));
        }
        self.raster_target = target;
        Ok(())
    }
    /// Procedural Q3 builtin uses the same retained cache and upload recipe as files.
    pub fn native_default_image(
        &mut self,
        assets: &mut Assets,
        indexed: Option<IndexedTarget>,
        existing: Option<ImageId>,
    ) -> Result<ResolvedImage, ResourceError> {
        if let Some(id) = existing {
            assets
                .image(id)
                .ok_or(ResourceError::Asset("missing native default image"))?;
        }
        let key = ImageKey {
            name: assets
                .intern_name("*default")
                .map_err(ResourceError::Asset)?,
            recipe: ImageRecipe::NativeDefault { indexed },
        };
        if let Some(index) = self.find(key) {
            return Ok(self.resolved[index].image);
        }
        let usage = ImageUse {
            role: ImageRole::NativeDefault,
            allow_picmip: false,
            ..ImageUse::default()
        };
        let id = if let Some(id) = existing {
            id
        } else {
            let mut rgba = RgbaImage {
                width: 16,
                height: 16,
                pixels: vec![32; 16 * 16 * 4],
            };
            for y in 0..16 {
                for x in 0..16 {
                    if x == 0 || y == 0 || x == 15 || y == 15 {
                        rgba.pixels[(y * 16 + x) * 4..(y * 16 + x + 1) * 4].fill(255);
                    }
                }
            }
            let id = if let Some(target) = indexed {
                let palette = assets
                    .palette(target.palette)
                    .ok_or(ResourceError::Asset("missing default image palette"))?;
                let texture = rgba_texture(&rgba, palette, target.transparent_index)?;
                assets.register_rgba_with_indexed(16, 16, &rgba.pixels, texture)
            } else {
                assets.register_image(16, 16, &rgba.pixels)
            }
            .map_err(ResourceError::Asset)?;
            self.prepare(assets, id, usage, None)?;
            id
        };
        Ok(self.remember(key, key.name, id, usage))
    }
    fn native_default_key(&self, name: NameId) -> Option<ImageKey> {
        self.named_range(name).find_map(|index| {
            let key = self.resolved[index].key;
            matches!(key.recipe, ImageRecipe::NativeDefault { .. }).then_some(key)
        })
    }
    fn cached_use(&mut self, key: ImageKey, usage: ImageUse) -> Option<ResolvedImage> {
        let index = self.find(key)?;
        let first = self.recipes.effective_use(self.resolved[index].first_use);
        let requested = self.recipes.effective_use(usage);
        if self.recipes.rules == RuleSetId::Quake3
            && (first.sampler != requested.sampler || first.allow_picmip != requested.allow_picmip)
            && !self
                .conflicts
                .iter()
                .any(|entry| entry.name == key.name && entry.requested == requested)
        {
            self.conflicts.push(ImageConflict {
                name: key.name,
                first,
                requested,
            });
        }
        Some(self.resolved[index].image)
    }
    fn prepare(
        &mut self,
        assets: &mut Assets,
        id: ImageId,
        usage: ImageUse,
        rgba: Option<&[u8]>,
    ) -> Result<(), ResourceError> {
        let params = self.recipes.params(usage)?;
        match rgba {
            Some(rgba) => assets.prepare_image_with_rgba(id, rgba, params),
            None => assets.prepare_image(id, params),
        }
        .map_err(ResourceError::Asset)?;
        if self.recipes.rules == RuleSetId::Quake3 {
            assets
                .set_image_native_sampler(id, self.recipes.effective_use(usage).sampler)
                .map_err(ResourceError::Asset)?;
        }
        Ok(())
    }
    fn named_range(&self, name: NameId) -> std::ops::Range<usize> {
        let first = self
            .resolved
            .partition_point(|entry| entry.key.name.0 < name.0);
        let count = self.resolved[first..].partition_point(|entry| entry.key.name == name);
        first..first + count
    }
    fn find(&self, key: ImageKey) -> Option<usize> {
        self.named_range(key.name).find(|&index| {
            let old = self.resolved[index].key;
            old == key
                || self.source_rules == RuleSetId::Quake2Rerelease
                    && self
                        .file_identity(key.recipe)
                        .is_some_and(|identity| self.file_identity(old.recipe) == Some(identity))
        })
    }
    fn file_identity(
        &self,
        recipe: ImageRecipe,
    ) -> Option<(u32, Option<PaletteId>, Option<IndexedTarget>)> {
        let indexed = |palette, transparent_index| {
            let target = self
                .indexed_remap
                .as_ref()
                .filter(|(source, _)| *source == palette)
                .map_or(palette, |(_, target)| *target);
            (
                Some(palette),
                Some(IndexedTarget {
                    palette: target,
                    transparent_index,
                }),
            )
        };
        let (role, source, target) = match recipe {
            ImageRecipe::Pcx {
                palette,
                transparent_index,
                usage,
                ..
            }
            | ImageRecipe::Lmp {
                palette,
                transparent_index,
                usage,
            } => {
                let (source, target) = indexed(palette, transparent_index);
                (usage.role.override_bit()?, source, target)
            }
            ImageRecipe::Wal { palette, cutout } => {
                let (source, target) = indexed(palette, cutout.then_some(255));
                (ImageRole::Surface.override_bit()?, source, target)
            }
            ImageRecipe::Raster {
                indexed: target,
                role,
                ..
            } => {
                let source = target.map(|target| self.source_palette(target.palette));
                (role?, source, target)
            }
            _ => return None,
        };
        Some((role, source, target))
    }
    fn source_palette(&self, target: PaletteId) -> PaletteId {
        self.indexed_remap
            .as_ref()
            .filter(|(_, destination)| *destination == target)
            .map_or(target, |(source, _)| *source)
    }
    fn remember(
        &mut self,
        key: ImageKey,
        first_requested: NameId,
        id: ImageId,
        usage: ImageUse,
    ) -> ResolvedImage {
        let resolved = ResolvedImage {
            id,
            sampler: self.recipes.effective_use(usage).sampler,
        };
        let at = self
            .resolved
            .partition_point(|entry| entry.key.name.0 <= key.name.0);
        self.resolved.insert(
            at,
            ImageRecord {
                key,
                image: resolved,
                first_use: usage,
                first_requested,
            },
        );
        resolved
    }
}
pub struct Images<'a> {
    pub assets: &'a mut Assets,
    vfs: &'a Vfs,
    state: ImagesState,
}
impl<'a> Images<'a> {
    /// Replay admitted file requests in original image-admission order. Generated
    /// and embedded resources are retained or rebuilt by their owning source.
    /// A missing file is left for the material compiler's ordinary fallback.
    pub fn replay_reload(&mut self, reload: &ImageReload) -> Result<(), ResourceError> {
        let mut ordered: Vec<_> = reload.resolved.iter().collect();
        ordered.sort_by_key(|entry| entry.image.id.0);
        for entry in ordered {
            if matches!(
                entry.key.recipe,
                ImageRecipe::Embedded { .. }
                    | ImageRecipe::TranslatedSkin { .. }
                    | ImageRecipe::LayeredSky { .. }
                    | ImageRecipe::NativeDefault { .. }
                    | ImageRecipe::MissingSkin { .. }
            ) {
                continue;
            }
            let name = self
                .assets
                .name(entry.first_requested)
                .ok_or(ResourceError::Asset("missing image replay name"))?
                .to_owned();
            let result = self.try_registration(|images| match entry.key.recipe {
                ImageRecipe::Raster { indexed, .. } => {
                    images.raster(&name, entry.first_use, indexed).map(|_| ())
                }
                ImageRecipe::Wal { palette, cutout } => {
                    images.wal(&name, palette, cutout).map(|_| ())
                }
                ImageRecipe::Lmp {
                    palette,
                    transparent_index,
                    ..
                } => images
                    .indexed_lmp(&name, palette, transparent_index, entry.first_use)
                    .map(|_| ()),
                ImageRecipe::Wad {
                    lump,
                    palette,
                    transparent_index,
                    ..
                } => {
                    let lump = images
                        .assets
                        .resource_name(lump)
                        .ok_or(ResourceError::Asset("missing image replay lump"))?
                        .to_vec();
                    images
                        .indexed_wad(&name, &lump, palette, transparent_index, entry.first_use)
                        .map(|_| ())
                }
                ImageRecipe::Pcx {
                    palette,
                    transparent_index,
                    rgba_override,
                    ..
                } => {
                    let rgba_override = rgba_override
                        .map(|id| {
                            images
                                .assets
                                .name(id)
                                .map(str::to_owned)
                                .ok_or(ResourceError::Asset("missing image replay override"))
                        })
                        .transpose()?;
                    images
                        .indexed_pcx(
                            &name,
                            palette,
                            transparent_index,
                            rgba_override.as_deref(),
                            entry.first_use,
                        )
                        .map(|_| ())
                }
                ImageRecipe::Embedded { .. }
                | ImageRecipe::TranslatedSkin { .. }
                | ImageRecipe::LayeredSky { .. }
                | ImageRecipe::NativeDefault { .. }
                | ImageRecipe::MissingSkin { .. } => Ok(()),
            });
            match result {
                Ok(()) | Err(ResourceError::Missing(_)) => {}
                Err(error) => return Err(error),
            }
        }
        Ok(())
    }
    pub fn set_search_order(&mut self, order: ImageSearchOrder) {
        self.assets.set_image_search_order(order);
    }
    fn file_name(&mut self, name: &str) -> Result<NameId, ResourceError> {
        let name = if self.state.source_rules == RuleSetId::Quake2Rerelease {
            file_stem(name)
        } else {
            name
        };
        self.assets.intern_name(name).map_err(ResourceError::Asset)
    }
    fn select_file(
        &self,
        name: &str,
        usage: ImageUse,
        rgba_override: Option<&str>,
        embedded: bool,
    ) -> Result<Option<SelectedFile>, ResourceError> {
        let name = qa_core::names::canonical_path(name);
        let rgba_override = rgba_override.map(qa_core::names::canonical_path);
        self.assets.image_search_order().select(
            self.vfs,
            &name,
            self.state.source_rules,
            self.state.recipes.settings,
            usage,
            rgba_override.as_deref(),
            embedded,
        )
    }
    fn native_texture(
        &mut self,
        name: &str,
        format: SourceFormat,
        transparent: Option<u8>,
    ) -> Result<IndexedTexture, ResourceError> {
        let bytes = if format == SourceFormat::Raster(ImageFormat::Lmp) {
            read_file_with_limit(
                self.vfs,
                name.as_bytes(),
                INDEXED_PIC_FILE_LIMIT,
                &mut self.state.reader,
            )?
        } else {
            read_file(self.vfs, name.as_bytes(), &mut self.state.reader)?
        };
        if format == SourceFormat::Wal {
            let mip = MipTexture::parse(&bytes, qa_formats::image::MipFormat::Wal)?;
            return IndexedTexture::load_masked(mip.width, mip.height, mip.levels, transparent)
                .map_err(ResourceError::Asset);
        }
        let SourceFormat::Raster(format) = format else {
            return Err(ResourceError::Asset("unknown indexed image format"));
        };
        let source = if format == ImageFormat::Pcx {
            RuleSetId::Quake2
        } else {
            self.state.source_rules
        };
        let DecodedImage::Indexed(image) = decode(&bytes, format, source.lineage())? else {
            return Err(ResourceError::Asset("image must retain source indices"));
        };
        IndexedTexture::load_base(image.width, image.height, &image.indices, transparent)
            .map_err(ResourceError::Asset)
    }
    fn rgba_file(
        &mut self,
        selected: &SelectedFile,
        palette: Option<PaletteId>,
    ) -> Result<RgbaImage, ResourceError> {
        let bytes = read_file(self.vfs, selected.path.as_bytes(), &mut self.state.reader)?;
        let indexed;
        let image = match selected.format {
            SourceFormat::Wal => {
                let mip = MipTexture::parse(&bytes, qa_formats::image::MipFormat::Wal)?;
                indexed = IndexedImage {
                    width: mip.width,
                    height: mip.height,
                    indices: std::borrow::Cow::Borrowed(mip.levels[0]),
                    palette: None,
                };
                &indexed
            }
            SourceFormat::Raster(format) => {
                match decode(&bytes, format, self.state.source_rules.lineage())? {
                    DecodedImage::Rgba(image) => return Ok(image),
                    DecodedImage::Indexed(image) => {
                        indexed = image;
                        &indexed
                    }
                }
            }
        };
        let colors = palette.and_then(|palette| self.assets.palette(palette));
        let global = colors.map(|colors| Palette(Box::new(colors.rgba_table())));
        let palette = if matches!(
            self.state.source_rules,
            RuleSetId::Quake2 | RuleSetId::Quake2Rerelease
        ) {
            global.as_ref().or(image.palette.as_ref())
        } else {
            image.palette.as_ref().or(global.as_ref())
        }
        .ok_or(ResourceError::Asset("image has no palette"))?;
        Ok(image.expand(palette, &PaletteOptions::default())?)
    }
    fn texture_rgba(
        &self,
        texture: &IndexedTexture,
        palette: PaletteId,
        native_fringe: bool,
    ) -> Result<RgbaImage, ResourceError> {
        let base = texture
            .mip(0)
            .ok_or(ResourceError::Asset("missing indexed base mip"))?;
        let colors = self
            .assets
            .palette(palette)
            .ok_or(ResourceError::Asset("missing image palette"))?;
        Ok(IndexedImage {
            width: base.width,
            height: base.height,
            indices: std::borrow::Cow::Borrowed(base.indices()),
            palette: None,
        }
        .expand(
            &Palette(Box::new(colors.rgba_table())),
            &PaletteOptions {
                transparent_index: if native_fringe {
                    Some(255)
                } else {
                    texture.transparent_index()
                },
                fringe: if native_fringe {
                    AlphaFringe::NativeNeighbors
                } else {
                    AlphaFringe::KeepPalette
                },
                ..PaletteOptions::default()
            },
        )?)
    }
    fn indexed_file(
        &mut self,
        key: ImageKey,
        name: &str,
        format: SourceFormat,
        palette: PaletteId,
        transparent: Option<u8>,
        rgba_override: Option<&str>,
        usage: ImageUse,
    ) -> Result<ResolvedImage, ResourceError> {
        let selected = self
            .select_file(name, usage, rgba_override, false)?
            .ok_or_else(|| ResourceError::Missing(name.to_owned()))?;
        let original = qa_core::names::canonical_path(name);
        let original_selected = selected.path == original && selected.format == format;
        let texture = if original_selected {
            Some(self.native_texture(&original, format, transparent)?)
        } else if self.vfs.open(original.as_bytes()).is_some() {
            // An unselected original is only optional software/dimension data.
            // A broken original cannot invalidate a valid replacement.
            self.native_texture(&original, format, transparent).ok()
        } else {
            None
        };
        let rgba = if original_selected {
            None
        } else {
            Some(self.rgba_file(&selected, Some(palette))?)
        };
        let native_fringe = format.native_fringe(self.state.recipes.rules);
        let first_requested = self
            .assets
            .intern_name(name)
            .map_err(ResourceError::Asset)?;
        self.finish_indexed(
            key,
            first_requested,
            texture,
            rgba,
            palette,
            transparent,
            usage,
            native_fringe,
        )
    }
    fn finish_indexed(
        &mut self,
        key: ImageKey,
        first_requested: NameId,
        texture: Option<IndexedTexture>,
        rgba: Option<RgbaImage>,
        palette: PaletteId,
        transparent: Option<u8>,
        usage: ImageUse,
        native_fringe: bool,
    ) -> Result<ResolvedImage, ResourceError> {
        let texture_palette = if texture.is_some() {
            palette
        } else {
            self.state
                .indexed_remap
                .as_ref()
                .map_or(palette, |(_, target)| *target)
        };
        let id = if let Some(rgba) = rgba {
            let texture = if let Some(texture) = texture {
                texture
            } else {
                let colors = self
                    .assets
                    .palette(texture_palette)
                    .ok_or(ResourceError::Asset("missing image palette"))?;
                rgba_texture(&rgba, colors, transparent)?
            };
            let id = self.register_indexed_rgba(
                rgba.width,
                rgba.height,
                &rgba.pixels,
                texture,
                texture_palette,
            )?;
            self.prepare(id, usage, None)?;
            id
        } else {
            let texture = texture.ok_or(ResourceError::Asset("missing native image"))?;
            let gl_pixels = if native_fringe {
                Some(self.texture_rgba(&texture, palette, true)?)
            } else {
                None
            };
            let target = self.indexed_target(palette);
            let id = self.register_indexed(texture, palette, target)?;
            self.prepare(
                id,
                usage,
                gl_pixels.as_ref().map(|image| image.pixels.as_slice()),
            )?;
            id
        };
        Ok(self.remember(key, first_requested, id, usage))
    }
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
    pub(crate) fn state_assets(&mut self) -> (&mut ImagesState, &mut Assets) {
        (&mut self.state, self.assets)
    }
    /// Optional native registrations discard only their uncommitted resource suffix.
    pub fn try_registration<T, E: From<ResourceError>>(
        &mut self,
        register: impl FnOnce(&mut Self) -> Result<T, E>,
    ) -> Result<T, E> {
        let retained = self.assets.counts();
        let mark = self.state.registration_mark();
        let result = register(self);
        if result.is_err() {
            self.assets
                .discard_registration_suffix(retained)
                .map_err(|error| E::from(ResourceError::Asset(error)))?;
            self.state.rollback_registration(mark, retained.images);
        }
        result
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
        self.state.set_raster_target(self.assets, target)
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
            return Ok(self.state.resolved[index].image.id);
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
        self.remember(key, key.name, id, usage);
        Ok(id)
    }
    fn indexed_target(&self, palette: PaletteId) -> Option<PaletteId> {
        self.state
            .indexed_remap
            .filter(|(source, _)| *source == palette)
            .map(|(_, target)| target)
    }
    fn remap_texture(
        &mut self,
        texture: IndexedTexture,
        source: PaletteId,
        target: Option<PaletteId>,
    ) -> Result<IndexedTexture, ResourceError> {
        let Some(target) = target.filter(|&target| target != source) else {
            return Ok(texture);
        };
        let transparent = texture.transparent_index();
        let key = (source, target, transparent);
        let index = match self
            .state
            .indexed_maps
            .iter()
            .position(|(old, _)| *old == key)
        {
            Some(index) => index,
            None => {
                let source = self
                    .assets
                    .palette(source)
                    .ok_or(ResourceError::Asset("missing source image palette"))?;
                let target = self
                    .assets
                    .palette(target)
                    .ok_or(ResourceError::Asset("missing presentation image palette"))?;
                let indices = source.translation_to(target, transparent);
                self.state.indexed_maps.push((key, indices));
                self.state.indexed_maps.len() - 1
            }
        };
        let translation = &self.state.indexed_maps[index].1;
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
        let rebuilt = if let [Some(a), Some(b), Some(c), Some(d)] = &mips {
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
        .map_err(ResourceError::Asset)?;
        rebuilt
            .with_coordinate_extent(texture.coordinate_extent())
            .map_err(ResourceError::Asset)
    }
    /// Retain source RGB while converting only software indices to an explicit
    /// target. A resource may provide its source palette independently of the
    /// default source/presentation pair captured by this image owner.
    pub fn register_indexed(
        &mut self,
        texture: IndexedTexture,
        palette: PaletteId,
        target: Option<PaletteId>,
    ) -> Result<ImageId, ResourceError> {
        if target.is_none_or(|target| target == palette) {
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
        self.register_indexed_rgba_to(
            rgba.width,
            rgba.height,
            &rgba.pixels,
            texture,
            palette,
            target,
        )
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
        let target = self.indexed_target(palette);
        self.register_indexed_rgba_to(width, height, rgba, texture, palette, target)
    }
    fn register_indexed_rgba_to(
        &mut self,
        width: u32,
        height: u32,
        rgba: &[u8],
        texture: IndexedTexture,
        palette: PaletteId,
        target: Option<PaletteId>,
    ) -> Result<ImageId, ResourceError> {
        let texture = self.remap_texture(texture, palette, target)?;
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
        source: u32,
        mip: &MipTexture<'_>,
        palette: PaletteId,
        cutout: bool,
    ) -> Result<ImageId, ResourceError> {
        let key = ImageKey {
            name: self
                .assets
                .intern_resource_name(mip.name)
                .map_err(ResourceError::Asset)?,
            recipe: ImageRecipe::Embedded {
                source,
                palette,
                cutout,
            },
        };
        if let Some(index) = self.find(key) {
            let id = self.state.resolved[index].image.id;
            let image = self
                .assets
                .image(id)
                .ok_or(ResourceError::Asset("missing embedded image"))?;
            if image.logical_size() != [mip.width, mip.height] {
                return Err(ResourceError::Asset("embedded image dimensions changed"));
            }
            return Ok(id);
        }
        let replacement = if let Ok(name) = std::str::from_utf8(mip.name) {
            self.select_file(&format!("textures/{name}"), ImageUse::default(), None, true)?
        } else {
            None
        };
        let rgba = replacement
            .as_ref()
            .map(|selected| self.rgba_file(selected, Some(palette)))
            .transpose()?;
        let texture = IndexedTexture::load(mip.width, mip.height, mip.levels, cutout)
            .map_err(ResourceError::Asset)?;
        let native_fringe = matches!(
            self.state.recipes.rules,
            RuleSetId::Quake2 | RuleSetId::Quake2Rerelease
        );
        Ok(self
            .finish_indexed(
                key,
                key.name,
                Some(texture),
                rgba,
                palette,
                cutout.then_some(255),
                ImageUse::default(),
                native_fringe,
            )?
            .id)
    }
    pub(crate) fn layered_sky(
        &mut self,
        source: u32,
        mip: &MipTexture<'_>,
        palette: PaletteId,
    ) -> Result<[ImageId; 2], ResourceError> {
        if mip.width != 256 || mip.height != 128 {
            return Err(ResourceError::Asset("native layered sky dimensions"));
        }
        let name = self
            .assets
            .intern_resource_name(mip.name)
            .map_err(ResourceError::Asset)?;
        let keys = [false, true].map(|overlay| ImageKey {
            name,
            recipe: ImageRecipe::LayeredSky {
                source,
                palette,
                overlay,
            },
        });
        let cached = keys.map(|key| {
            self.find(key)
                .map(|index| self.state.resolved[index].image.id)
        });
        if let [Some(opaque), Some(masked)] = cached {
            return Ok([opaque, masked]);
        }
        let usage = ImageUse {
            role: ImageRole::LayeredSky,
            sampler: Sampler {
                mipmaps: false,
                ..Sampler::default()
            },
            allow_picmip: false,
        };
        let replacement = if let Ok(name) = std::str::from_utf8(mip.name) {
            self.select_file(&format!("textures/{name}"), usage, None, true)?
        } else {
            None
        };
        let replacement = replacement
            .as_ref()
            .map(|selected| {
                let rgba = self.rgba_file(selected, Some(palette))?;
                crate::sky::split_layered_rgba(rgba.width, rgba.height, &rgba.pixels)
                    .map_err(ResourceError::Asset)
            })
            .transpose()?;
        let colors = self
            .assets
            .palette(palette)
            .ok_or(ResourceError::Asset("missing sky palette"))?;
        let layers = crate::sky::split_layered_sky(mip.levels[0], &colors.rgba_table())
            .map_err(ResourceError::Asset)?;
        let corrected = if replacement.is_none() {
            Some(
                crate::sky::split_layered_sky(mip.levels[0], &self.corrected_palette(palette)?)
                    .map_err(ResourceError::Asset)?,
            )
        } else {
            // The replacement owns its RGBA. Native palette correction applies
            // only to the authored indexed layers.
            None
        };
        let params = self.upload_params(usage)?;
        let mut ids = [ImageId(0); 2];
        for (index, (raw, corrected, indices, transparent)) in [
            (
                &layers.opaque_rgba,
                corrected
                    .as_ref()
                    .map_or(&layers.opaque_rgba, |layers| &layers.opaque_rgba),
                &layers.opaque_indices,
                None,
            ),
            (
                &layers.masked_rgba,
                corrected
                    .as_ref()
                    .map_or(&layers.masked_rgba, |layers| &layers.masked_rgba),
                &layers.masked_indices,
                Some(0),
            ),
        ]
        .into_iter()
        .enumerate()
        {
            ids[index] = if let Some(id) = cached[index] {
                id
            } else {
                let (size, raw, corrected) = if let Some(replacement) = &replacement {
                    let pixels = if index == 0 {
                        replacement.opaque.as_flattened()
                    } else {
                        replacement.masked.as_flattened()
                    };
                    (replacement.size, pixels, pixels)
                } else {
                    (128, raw.as_ref(), corrected.as_ref())
                };
                let texture = IndexedTexture::load_base(128, 128, indices, transparent)
                    .map_err(ResourceError::Asset)?;
                let id = self.register_indexed_rgba(size, size, raw, texture, palette)?;
                self.assets
                    .prepare_image_with_rgba(id, corrected, params)
                    .map_err(ResourceError::Asset)?;
                self.remember(keys[index], name, id, usage);
                id
            };
        }
        Ok(ids)
    }
    pub fn wal(
        &mut self,
        name: &str,
        palette: PaletteId,
        cutout: bool,
    ) -> Result<ImageId, ResourceError> {
        let key = ImageKey {
            name: self.file_name(name)?,
            recipe: ImageRecipe::Wal { palette, cutout },
        };
        if let Some(index) = self.find(key) {
            return Ok(self.state.resolved[index].image.id);
        }
        Ok(self
            .indexed_file(
                key,
                name,
                SourceFormat::Wal,
                palette,
                cutout.then_some(255),
                None,
                ImageUse::default(),
            )?
            .id)
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
            name: self.file_name(name)?,
            recipe: ImageRecipe::Lmp {
                palette,
                transparent_index,
                usage,
            },
        };
        if let Some(index) = self.find(key) {
            return Ok(self.state.resolved[index].image);
        }
        self.indexed_file(
            key,
            name,
            SourceFormat::Raster(ImageFormat::Lmp),
            palette,
            transparent_index,
            None,
            usage,
        )
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
            return Ok(self.state.resolved[index].image);
        }
        let replacement = if let Ok(lump_name) = std::str::from_utf8(lump) {
            let archive = qa_core::names::canonical_path(name);
            self.select_file(
                &format!("{}/{lump_name}", file_stem(&archive)),
                usage,
                None,
                true,
            )?
        } else {
            None
        };
        let texture = (|| {
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
            IndexedTexture::load_base(image.width, image.height, &image.indices, transparent_index)
                .map_err(ResourceError::Asset)
        })();
        let texture = if replacement.is_some() {
            texture.ok()
        } else {
            Some(texture?)
        };
        let rgba = replacement
            .as_ref()
            .map(|selected| self.rgba_file(selected, Some(palette)))
            .transpose()?;
        self.finish_indexed(
            key,
            key.name,
            texture,
            rgba,
            palette,
            transparent_index,
            usage,
            false,
        )
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
            name: self.file_name(name)?,
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
            return Ok(self.state.resolved[index].image.id);
        }
        Ok(self
            .indexed_file(
                key,
                name,
                SourceFormat::Raster(ImageFormat::Pcx),
                palette,
                transparent_index,
                rgba_override,
                usage,
            )?
            .id)
    }
    /// Native sky fallback reuses the generated builtin and its first upload policy.
    pub(crate) fn native_default_image(&mut self) -> Result<ResolvedImage, ResourceError> {
        let name = self
            .assets
            .intern_name("*default")
            .map_err(ResourceError::Asset)?;
        let indexed = match self.state.native_default_key(name).map(|key| key.recipe) {
            Some(ImageRecipe::NativeDefault { indexed }) => indexed,
            _ => self.state.raster_target.map(|target| IndexedTarget {
                transparent_index: None,
                ..target
            }),
        };
        self.state.native_default_image(self.assets, indexed, None)
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
        let name_id = self.file_name(name)?;
        let opaque = indexed.map(|target| IndexedTarget {
            transparent_index: None,
            ..target
        });
        if self.assets.name(name_id) == Some("*default") {
            let cached = self.state.native_default_key(name_id);
            let key = if let Some(key) = cached {
                Some(key)
            } else if rules == RuleSetId::Quake3 {
                self.state.native_default_image(self.assets, opaque, None)?;
                Some(ImageKey {
                    name: name_id,
                    recipe: ImageRecipe::NativeDefault { indexed: opaque },
                })
            } else {
                None
            };
            if let Some(key) = key {
                return self
                    .state
                    .cached_use(key, usage)
                    .ok_or(ResourceError::Asset("missing native default recipe"));
            }
        }
        let key = ImageKey {
            name: name_id,
            recipe: ImageRecipe::Raster {
                rules,
                indexed,
                role: (rules == RuleSetId::Quake2Rerelease)
                    .then(|| usage.role.override_bit())
                    .flatten(),
            },
        };
        if let Some(resolved) = self.state.cached_use(key, usage) {
            return Ok(resolved);
        }
        let first_requested = self
            .assets
            .intern_name(name)
            .map_err(ResourceError::Asset)?;
        if let Some(target) = indexed
            && let Some(format) = SourceFormat::from_path(name)
            && format.retains_indices(rules)
        {
            return self.indexed_file(
                key,
                name,
                format,
                self.state.source_palette(target.palette),
                target.transparent_index,
                None,
                usage,
            );
        }
        let selected = self
            .select_file(name, usage, None, false)?
            .ok_or_else(|| ResourceError::Missing(name.to_owned()))?;
        if let Some(target) = indexed
            && selected.format.retains_indices(rules)
        {
            let texture =
                self.native_texture(&selected.path, selected.format, target.transparent_index)?;
            let native_fringe = selected.format.native_fringe(self.state.recipes.rules);
            return self.finish_indexed(
                key,
                first_requested,
                Some(texture),
                None,
                self.state.source_palette(target.palette),
                target.transparent_index,
                usage,
                native_fringe,
            );
        }
        if matches!(rules, RuleSetId::Quake2 | RuleSetId::Quake2Rerelease)
            && selected.format == SourceFormat::Raster(ImageFormat::Pcx)
            && indexed.is_none()
        {
            return Err(ResourceError::Asset(
                "Q2 PCX requires indexed palette registration",
            ));
        }
        let rgba = self.rgba_file(&selected, indexed.map(|target| target.palette))?;
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
        let resolved = self.remember(key, first_requested, id, usage);
        Ok(resolved)
    }
    fn prepare(
        &mut self,
        id: ImageId,
        usage: ImageUse,
        rgba: Option<&[u8]>,
    ) -> Result<(), ResourceError> {
        self.state.prepare(self.assets, id, usage, rgba)
    }
    fn find(&self, key: ImageKey) -> Option<usize> {
        self.state.find(key)
    }
    fn remember(
        &mut self,
        key: ImageKey,
        first_requested: NameId,
        id: ImageId,
        usage: ImageUse,
    ) -> ResolvedImage {
        self.state.remember(key, first_requested, id, usage)
    }
}
