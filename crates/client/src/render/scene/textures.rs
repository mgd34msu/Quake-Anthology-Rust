//! Scene texture loading, sampling, and GIF animation.
//!
//! Donor provenance: `src/render/scene/textures.ts`. Loading stays
//! synchronous: the host [`SceneAssetReader`] serves bytes and an optional
//! [`SceneImageDecoder`] decodes formats without a local decoder (PNG, JPEG,
//! BMP, GIF). TGA, PCX, WAL, and LMP decode locally. Q2 intensity scaling and
//! mipmapping reuse `super::q2_image`; Q2 skin flooding reuses `super::skin`.

use std::collections::{HashMap, HashSet};

use qa_content::wad::MipTexture;

use crate::render::types::{
    ImageLevel, ImageSource, LevelContent, Palette, PaletteTransparency, RenderImage, RendererImage, TextureFilter,
    TextureSampling,
};
use crate::render::RenderError;

use super::image_policy::{default_image_policy, ImageFormat, ImagePolicy, ImageUsage};
use super::q2_image::q2_mipmapped_image;
use super::resources::SceneImageRegistry;
use super::skin::flood_skin;

/// Raw asset bytes plus their content source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SceneAsset {
    /// File bytes.
    pub bytes: Vec<u8>,
    /// Content source.
    pub source: ImageSource,
}

/// Host asset reader for texture bytes.
pub trait SceneAssetReader {
    /// Read an asset by content path.
    fn read(&self, path: &str) -> Result<Option<SceneAsset>, RenderError>;

    /// Alternate original source beneath a user replacement.
    fn read_original(&self, _path: &str) -> Result<Option<SceneAsset>, RenderError> {
        Ok(None)
    }
}

/// A loaded scene texture.
#[derive(Debug, Clone, PartialEq)]
pub struct SceneTexture {
    /// Logical name.
    pub name: String,
    /// Logical width (original source size).
    pub width: u32,
    /// Logical height.
    pub height: u32,
    /// Resident image.
    pub image: RendererImage,
    /// Uploaded content.
    pub content: RenderImage,
    /// Fullbright overlay image, if any.
    pub fullbright: Option<RendererImage>,
}

/// Texture source family.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TextureFamily {
    /// Quake 1.
    Q1,
    /// Quake 2.
    Q2,
    /// Quake 3.
    #[default]
    Q3,
}

/// Texture load options.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SceneTextureLoadOptions {
    /// Build mipmaps.
    pub mipmap: bool,
    /// Repeat sampling.
    pub repeat: bool,
    /// Source family.
    pub family: TextureFamily,
    /// Usage hint.
    pub usage: Option<ImageUsage>,
}

impl Default for SceneTextureLoadOptions {
    fn default() -> Self {
        Self {
            mipmap: true,
            repeat: true,
            family: TextureFamily::Q3,
            usage: None,
        }
    }
}

/// One decoded RGBA level.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RgbaLevel {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// Row-major RGBA bytes.
    pub pixels: Vec<u8>,
}

/// Decoded indexed levels plus an optional embedded palette.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexedLevels {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// One index buffer per mip level.
    pub levels: Vec<Vec<u8>>,
    /// Embedded 768-byte palette, if any.
    pub palette: Option<Vec<u8>>,
}

/// Host-decoded image for formats without a local decoder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DecodedSceneImage {
    /// Single true-color image.
    Rgba(RgbaLevel),
    /// Indexed image.
    Indexed(IndexedLevels),
    /// Animated true-color frames.
    Animated(Vec<RgbaLevel>),
}

/// Host decoder for PNG, JPEG, BMP, and GIF bytes.
pub trait SceneImageDecoder {
    /// Decode asset bytes; `path` selects the format by suffix.
    fn decode(&self, bytes: &[u8], path: &str) -> Result<DecodedSceneImage, RenderError>;
}

struct AnimationState {
    image: RendererImage,
    frames: Vec<RenderImage>,
    previous: usize,
}

/// Scene texture loader over a host asset reader.
pub struct SceneTextureLoader {
    images: SceneImageRegistry,
    reader: Box<dyn SceneAssetReader>,
    decoder: Option<Box<dyn SceneImageDecoder>>,
    palette: Option<Palette>,
    policy: Option<ImagePolicy>,
    fullbright_first: u8,
    loaded: HashMap<String, Option<SceneTexture>>,
    requests: HashMap<String, (String, SceneTextureLoadOptions)>,
    owned: Vec<RendererImage>,
    animations: HashMap<u32, AnimationState>,
    white: SceneTexture,
    missing: SceneTexture,
    closed: bool,
}

fn wire(message: String) -> RenderError {
    RenderError::BadWire(message)
}

fn rgba_content(image: RgbaLevel, mipmap: bool) -> RenderImage {
    RenderImage::Rgba8 {
        levels: if mipmap {
            mip_chain(image)
        } else {
            vec![ImageLevel {
                width: image.width,
                height: image.height,
                pixels: image.pixels,
            }]
        },
        border_color: qa_core::math::vec4(0.0, 0.0, 0.0, 0.0),
    }
}

/// Box-filter mip chain down to 1x1.
fn mip_chain(base: RgbaLevel) -> Vec<ImageLevel> {
    let mut levels = vec![ImageLevel {
        width: base.width,
        height: base.height,
        pixels: base.pixels,
    }];
    loop {
        let (next_width, next_height, next) = {
            let prior = levels.last().expect("mip chain keeps its base level");
            if prior.width <= 1 && prior.height <= 1 {
                break;
            }
            downsample_level(prior)
        };
        levels.push(ImageLevel {
            width: next_width,
            height: next_height,
            pixels: next,
        });
    }
    levels
}

/// One box-filter downsample step of a mip level.
fn downsample_level(prior: &ImageLevel) -> (u32, u32, Vec<u8>) {
    let (width, height, pixels) = (prior.width, prior.height, &prior.pixels);
    let next_width = (width / 2).max(1);
    let next_height = (height / 2).max(1);
    let mut next = vec![0u8; (next_width * next_height * 4) as usize];
    for y in 0..next_height {
        for x in 0..next_width {
            let mut sum = [0u32; 4];
            let mut count = 0;
            for dy in 0..2 {
                for dx in 0..2 {
                    let sx = (x * 2 + dx).min(width - 1);
                    let sy = (y * 2 + dy).min(height - 1);
                    let offset = ((sy * width + sx) * 4) as usize;
                    for channel in 0..4 {
                        sum[channel] += u32::from(pixels[offset + channel]);
                    }
                    count += 1;
                }
            }
            let offset = ((y * next_width + x) * 4) as usize;
            for channel in 0..4 {
                next[offset + channel] = (sum[channel] / count) as u8;
            }
        }
    }
    (next_width, next_height, next)
}

fn indexed_content(
    levels: Vec<(u32, u32, Vec<u8>)>,
    palette: Palette,
    transparency: PaletteTransparency,
    fullbright: Option<(u8, u8)>,
) -> RenderImage {
    RenderImage::Indexed8 {
        levels: levels
            .into_iter()
            .map(|(width, height, pixels)| ImageLevel { width, height, pixels })
            .collect(),
        palette,
        transparency,
        fullbright,
        translation: None,
    }
}

/// Decode an uncompressed or RLE true-color TGA.
fn decode_tga(bytes: &[u8], path: &str) -> Result<RgbaLevel, RenderError> {
    if bytes.len() < 18 {
        return Err(wire(format!("Truncated TGA header: {path}")));
    }
    let image_type = bytes[2];
    let width = u16::from_le_bytes([bytes[12], bytes[13]]) as u32;
    let height = u16::from_le_bytes([bytes[14], bytes[15]]) as u32;
    let depth = bytes[16];
    let descriptor = bytes[17];
    if width == 0 || height == 0 || (depth != 24 && depth != 32) {
        return Err(wire(format!("Unsupported TGA layout: {path}")));
    }
    if (image_type != 2 && image_type != 10) || bytes[1] != 0 {
        return Err(wire(format!("Unsupported TGA encoding: {path}")));
    }
    let channels = (depth / 8) as usize;
    let count = (width * height) as usize;
    let mut pixels = vec![0u8; count * 4];
    let mut read = 18 + bytes[0] as usize;
    let mut write = 0;
    let pixel = |bytes: &[u8], read: &mut usize| -> Result<[u8; 4], RenderError> {
        if *read + channels > bytes.len() {
            return Err(wire(format!("Truncated TGA pixels: {path}")));
        }
        let blue = bytes[*read];
        let green = bytes[*read + 1];
        let red = bytes[*read + 2];
        let alpha = if channels == 4 { bytes[*read + 3] } else { 255 };
        *read += channels;
        Ok([red, green, blue, alpha])
    };
    if image_type == 2 {
        for slot in pixels.as_chunks_mut::<4>().0 {
            slot.copy_from_slice(&pixel(bytes, &mut read)?);
        }
    } else {
        while write < count {
            if read >= bytes.len() {
                return Err(wire(format!("Truncated TGA packet: {path}")));
            }
            let header = bytes[read];
            read += 1;
            let run = (header & 0x7f) as usize + 1;
            if write + run > count {
                return Err(wire(format!("Invalid TGA run: {path}")));
            }
            if header & 0x80 != 0 {
                let value = pixel(bytes, &mut read)?;
                for _ in 0..run {
                    pixels[write * 4..write * 4 + 4].copy_from_slice(&value);
                    write += 1;
                }
            } else {
                for _ in 0..run {
                    pixels[write * 4..write * 4 + 4].copy_from_slice(&pixel(bytes, &mut read)?);
                    write += 1;
                }
            }
        }
    }
    if descriptor & 0x20 == 0 {
        let stride = width as usize * 4;
        for y in 0..height as usize / 2 {
            let (top, bottom) = (y * stride, (height as usize - 1 - y) * stride);
            for x in 0..stride {
                pixels.swap(top + x, bottom + x);
            }
        }
    }
    Ok(RgbaLevel { width, height, pixels })
}

/// Decoded PCX: width, height, indexed pixels, optional trailing palette.
type DecodedPcx = (u32, u32, Vec<u8>, Option<Vec<u8>>);

/// Decode an 8-bit PCX plus its trailing palette, if present.
fn decode_pcx(bytes: &[u8], path: &str) -> Result<DecodedPcx, RenderError> {
    if bytes.len() < 128 || bytes[0] != 0x0a || bytes[2] != 1 || bytes[3] != 8 || bytes[64] != 1 {
        return Err(wire(format!("Unsupported PCX layout: {path}")));
    }
    let int = |offset: usize| u16::from_le_bytes([bytes[offset], bytes[offset + 1]]) as u32;
    let (xmin, ymin, xmax, ymax) = (int(4), int(6), int(8), int(10));
    if xmax < xmin || ymax < ymin {
        return Err(wire(format!("Invalid PCX bounds: {path}")));
    }
    let (width, height) = (xmax - xmin + 1, ymax - ymin + 1);
    let stride = int(66) as usize;
    if width == 0 || height == 0 || stride < width as usize {
        return Err(wire(format!("Invalid PCX dimensions: {path}")));
    }
    let mut indices = vec![0u8; (width * height) as usize];
    let mut read = 128;
    for y in 0..height as usize {
        let mut x = 0;
        while x < stride {
            if read >= bytes.len() {
                return Err(wire(format!("Truncated PCX pixels: {path}")));
            }
            let header = bytes[read];
            read += 1;
            let (count, value) = if header & 0xc0 == 0xc0 {
                if read >= bytes.len() {
                    return Err(wire(format!("Truncated PCX run: {path}")));
                }
                let value = bytes[read];
                read += 1;
                ((header & 0x3f) as usize, value)
            } else {
                (1, header)
            };
            for _ in 0..count {
                if x < width as usize {
                    indices[y * width as usize + x] = value;
                }
                x += 1;
                if x >= stride {
                    break;
                }
            }
        }
    }
    let palette = if bytes.len() >= 769 && bytes[bytes.len() - 769] == 0x0c {
        Some(bytes[bytes.len() - 768..].to_vec())
    } else {
        None
    };
    Ok((width, height, indices, palette))
}

/// Decode a WAL texture's index levels.
fn decode_wal(bytes: &[u8], path: &str) -> Result<(u32, u32, Vec<Vec<u8>>), RenderError> {
    if bytes.len() < 100 {
        return Err(wire(format!("Truncated WAL header: {path}")));
    }
    let int =
        |offset: usize| u32::from_le_bytes([bytes[offset], bytes[offset + 1], bytes[offset + 2], bytes[offset + 3]]);
    let (width, height) = (int(32), int(36));
    if width == 0 || height == 0 || width > 4096 || height > 4096 {
        return Err(wire(format!("Invalid WAL dimensions: {path}")));
    }
    let mut levels = Vec::new();
    for mip in 0..4 {
        let offset = int(40 + mip * 4) as usize;
        let (w, h) = ((width >> mip).max(1), (height >> mip).max(1));
        let len = (w * h) as usize;
        if offset.checked_add(len).is_none_or(|end| end > bytes.len()) {
            return Err(wire(format!("Truncated WAL level: {path}")));
        }
        levels.push(bytes[offset..offset + len].to_vec());
    }
    Ok((width, height, levels))
}

impl SceneTextureLoader {
    /// Build a loader; registers the white and missing images.
    pub fn new(
        mut images: SceneImageRegistry,
        reader: Box<dyn SceneAssetReader>,
        palette: Option<Palette>,
        policy: Option<ImagePolicy>,
        fullbright_first: u8,
    ) -> Result<Self, RenderError> {
        let white_content = RenderImage::Rgba8 {
            levels: vec![ImageLevel {
                width: 1,
                height: 1,
                pixels: vec![255, 255, 255, 255],
            }],
            border_color: qa_core::math::vec4(1.0, 1.0, 1.0, 1.0),
        };
        let white_image = images.register(
            "*white",
            white_content.clone(),
            TextureSampling {
                repeat: true,
                filter: TextureFilter::Nearest,
            },
        )?;
        let mut pixels = vec![0u8; 16 * 16 * 4];
        for y in 0..16 {
            for x in 0..16 {
                let edge = x == 0 || y == 0 || x == 15 || y == 15;
                let color = if edge { [255, 255, 255, 255] } else { [32, 32, 32, 255] };
                pixels[(y * 16 + x) * 4..(y * 16 + x) * 4 + 4].copy_from_slice(&color);
            }
        }
        let missing_content = RenderImage::Rgba8 {
            levels: mip_chain(RgbaLevel {
                width: 16,
                height: 16,
                pixels,
            }),
            border_color: qa_core::math::vec4(0.0, 0.0, 0.0, 1.0),
        };
        let missing_image = images.register(
            "*default",
            missing_content.clone(),
            TextureSampling {
                repeat: true,
                filter: TextureFilter::LinearMipmapNearest,
            },
        )?;
        let loader = Self {
            images,
            reader,
            decoder: None,
            palette,
            policy,
            fullbright_first,
            loaded: HashMap::new(),
            requests: HashMap::new(),
            owned: vec![white_image.clone(), missing_image.clone()],
            animations: HashMap::new(),
            white: SceneTexture {
                name: "*white".to_string(),
                width: 1,
                height: 1,
                image: white_image,
                content: white_content,
                fullbright: None,
            },
            missing: SceneTexture {
                name: "*default".to_string(),
                width: 16,
                height: 16,
                image: missing_image,
                content: missing_content,
                fullbright: None,
            },
            closed: false,
        };
        Ok(loader)
    }

    /// Install the host decoder for PNG, JPEG, BMP, and GIF bytes.
    pub fn set_decoder(&mut self, decoder: Box<dyn SceneImageDecoder>) {
        self.decoder = Some(decoder);
    }

    /// The white fallback texture.
    #[must_use]
    pub fn white(&self) -> &SceneTexture {
        &self.white
    }

    /// The missing-texture fallback.
    #[must_use]
    pub fn missing(&self) -> &SceneTexture {
        &self.missing
    }

    /// Borrow the image registry.
    #[must_use]
    pub fn images(&self) -> &SceneImageRegistry {
        &self.images
    }

    /// Mutably borrow the image registry.
    pub fn images_mut(&mut self) -> &mut SceneImageRegistry {
        &mut self.images
    }

    /// First fullbright palette index.
    #[must_use]
    pub const fn fullbright_first(&self) -> u8 {
        self.fullbright_first
    }

    /// Whether the loader is closed.
    #[must_use]
    pub const fn is_closed(&self) -> bool {
        self.closed
    }

    fn require_open(&self) -> Result<(), RenderError> {
        if self.closed {
            return Err(RenderError::OutOfOrder("Scene texture loader is closed".to_string()));
        }
        Ok(())
    }

    /// Register content under a logical name, deriving a fullbright overlay
    /// for indexed content with a fullbright range.
    pub fn register(
        &mut self,
        name: &str,
        content: RenderImage,
        sampling: TextureSampling,
        source: Option<ImageSource>,
        logical_size: Option<(u32, u32)>,
    ) -> Result<SceneTexture, RenderError> {
        self.require_open()?;
        let source = source.unwrap_or_else(|| ImageSource::Generated { name: name.to_string() });
        let (width, height) = match &content {
            RenderImage::Indexed8 { levels, .. } | RenderImage::Rgba8 { levels, .. } => {
                let base = levels.first().ok_or_else(|| RenderError::BadDimensions {
                    width: 0,
                    height: 0,
                    detail: format!("Texture {name} has no levels"),
                })?;
                (base.width, base.height)
            }
            RenderImage::Depth32f { levels } => {
                let base = levels.first().ok_or_else(|| RenderError::BadDimensions {
                    width: 0,
                    height: 0,
                    detail: format!("Texture {name} has no levels"),
                })?;
                (base.width, base.height)
            }
        };
        let image = self
            .images
            .register_with_source(name, content.clone(), sampling, source)?;
        self.owned.push(image.clone());
        let mut fullbright = None;
        if let RenderImage::Indexed8 {
            levels,
            palette,
            transparency,
            fullbright: Some((first, last)),
            translation,
        } = &content
        {
            let convert = |level: &ImageLevel| {
                let mut pixels = vec![0u8; (level.width * level.height * 4) as usize];
                for (offset, index) in level.pixels.iter().enumerate() {
                    if *index < *first || *index > *last {
                        continue;
                    }
                    if !matches!(transparency, PaletteTransparency::Opaque)
                        && transparent_index(transparency) == Some(*index)
                    {
                        continue;
                    }
                    let translated = translation
                        .as_ref()
                        .and_then(|table| table.get(*index as usize).copied())
                        .unwrap_or(*index) as usize;
                    let base = translated * 3;
                    if let (Some(r), Some(g), Some(b)) = (
                        palette.colors.get(base),
                        palette.colors.get(base + 1),
                        palette.colors.get(base + 2),
                    ) {
                        pixels[offset * 4..offset * 4 + 4].copy_from_slice(&[*r, *g, *b, 255]);
                    }
                }
                ImageLevel {
                    width: level.width,
                    height: level.height,
                    pixels,
                }
            };
            let bright = RenderImage::Rgba8 {
                levels: levels.iter().map(convert).collect(),
                border_color: qa_core::math::vec4(0.0, 0.0, 0.0, 0.0),
            };
            let bright_image = self.images.register(&format!("{name}:fullbright"), bright, sampling)?;
            self.owned.push(bright_image.clone());
            fullbright = Some(bright_image);
        }
        let (width, height) = logical_size.unwrap_or((width, height));
        Ok(SceneTexture {
            name: name.to_string(),
            width,
            height,
            image,
            content,
            fullbright,
        })
    }

    /// Register an embedded Q1 mip texture; external textures return None.
    pub fn q1_embedded(&mut self, texture: &MipTexture) -> Result<Option<SceneTexture>, RenderError> {
        self.require_open()?;
        let (name, width, height, levels) = match texture {
            MipTexture::External { .. } => return Ok(None),
            MipTexture::Embedded {
                name,
                width,
                height,
                levels,
            } => (name, width, height, levels),
        };
        let palette = self
            .palette
            .clone()
            .ok_or_else(|| RenderError::Backend("Embedded Quake textures require their content palette".to_string()))?;
        let levels = levels
            .iter()
            .enumerate()
            .map(|(mip, indices)| ((*width >> mip).max(1), (*height >> mip).max(1), indices.to_vec()))
            .collect();
        let transparency = if name.starts_with('{') {
            PaletteTransparency::Q1Fence
        } else {
            PaletteTransparency::Opaque
        };
        let fullbright = if name.starts_with("sky") || name.starts_with('*') {
            None
        } else {
            Some((self.fullbright_first, 255))
        };
        self.register(
            name,
            indexed_content(levels, palette, transparency, fullbright),
            TextureSampling {
                repeat: true,
                filter: TextureFilter::LinearMipmapNearest,
            },
            None,
            None,
        )
        .map(Some)
    }

    /// Resample a texture for surface use; repeat+mipmap returns it unchanged.
    pub fn sample_surface(
        &mut self,
        texture: &SceneTexture,
        mipmap: bool,
        repeat: bool,
    ) -> Result<SceneTexture, RenderError> {
        self.require_open()?;
        if mipmap && repeat {
            return Ok(texture.clone());
        }
        let key = format!("\0surface:{}\0{mipmap}\0{repeat}", texture.image.ordinal);
        if let Some(cached) = self.loaded.get(&key) {
            return cached
                .clone()
                .ok_or_else(|| RenderError::Backend("Surface sampling cache lost its texture".to_string()));
        }
        let truncate = |levels: Vec<ImageLevel>| {
            if mipmap {
                levels
            } else {
                levels.into_iter().take(1).collect()
            }
        };
        let content = match &texture.content {
            RenderImage::Indexed8 {
                levels,
                palette,
                transparency,
                fullbright,
                translation,
            } => RenderImage::Indexed8 {
                levels: truncate(levels.clone()),
                palette: palette.clone(),
                transparency: transparency.clone(),
                fullbright: *fullbright,
                translation: translation.clone(),
            },
            RenderImage::Rgba8 { levels, border_color } => RenderImage::Rgba8 {
                levels: truncate(levels.clone()),
                border_color: *border_color,
            },
            RenderImage::Depth32f { levels } => RenderImage::Depth32f {
                levels: if mipmap {
                    levels.clone()
                } else {
                    levels.iter().take(1).cloned().collect()
                },
            },
        };
        let sampled = self.register(
            &texture.name,
            content,
            TextureSampling {
                repeat,
                filter: if mipmap {
                    TextureFilter::LinearMipmapNearest
                } else {
                    TextureFilter::Linear
                },
            },
            Some(texture.image.source.clone()),
            Some((texture.width, texture.height)),
        )?;
        if let Some(animation) = self.animations.get(&texture.image.ordinal) {
            let frames = animation
                .frames
                .iter()
                .map(|frame| match frame {
                    RenderImage::Rgba8 { levels, border_color } => RenderImage::Rgba8 {
                        levels: if mipmap {
                            levels.clone()
                        } else {
                            levels.iter().take(1).cloned().collect()
                        },
                        border_color: *border_color,
                    },
                    other => other.clone(),
                })
                .collect();
            self.animate(&sampled, frames);
        }
        self.loaded.insert(key, Some(sampled.clone()));
        Ok(sampled)
    }

    /// Load a texture by logical name, following the family candidate order.
    pub fn load(&mut self, name: &str, options: &SceneTextureLoadOptions) -> Result<Option<SceneTexture>, RenderError> {
        self.require_open()?;
        if options.family == TextureFamily::Q3 {
            if name == self.white.name {
                return Ok(Some(self.white.clone()));
            }
            if name == self.missing.name {
                return Ok(Some(self.missing.clone()));
            }
        }
        let key = format!(
            "{name}\0{}\0{}\0{:?}\0{:?}",
            options.mipmap, options.repeat, options.family, options.usage
        );
        if let Some(cached) = self.loaded.get(&key) {
            return Ok(cached.clone());
        }
        self.requests.insert(key.clone(), (name.to_string(), options.clone()));
        match self.load_uncached(name, options) {
            Ok(texture) => {
                self.loaded.insert(key, texture.clone());
                Ok(texture)
            }
            Err(error) => {
                self.loaded.remove(&key);
                self.requests.remove(&key);
                Err(error)
            }
        }
    }

    /// Replay every recorded request into a replacement loader.
    pub fn prepare_replacement(&self, replacement: &mut SceneTextureLoader) -> Result<(), RenderError> {
        self.require_open()?;
        for (name, options) in self.requests.values() {
            replacement.load(name, options)?;
        }
        self.require_open()
    }

    /// Advance GIF animations; the Q2 donor loops forever at 10 Hz.
    pub fn tick_animations(&mut self, milliseconds: f64) {
        let mut updates: Vec<(u32, usize)> = Vec::new();
        for animation in self.animations.values_mut() {
            if animation.frames.len() < 2 {
                continue;
            }
            let beat = (milliseconds / 100.0).floor() as i64;
            let len = animation.frames.len() as i64;
            let index = ((beat % len) + len) as usize % animation.frames.len();
            if index == animation.previous {
                continue;
            }
            animation.previous = index;
            updates.push((animation.image.ordinal, index));
        }
        for (ordinal, index) in updates {
            let Some(animation) = self.animations.get(&ordinal) else {
                continue;
            };
            let Some(frame) = animation.frames.get(index) else {
                continue;
            };
            if let RenderImage::Rgba8 { levels, .. } = frame {
                for (level, content) in levels.iter().enumerate() {
                    let _ = self
                        .images
                        .update(&animation.image, level as u32, LevelContent::Rgba(content.clone()));
                }
            }
        }
    }

    /// Release owned images and close the loader.
    pub fn dispose_images(&mut self) {
        self.close();
        for image in std::mem::take(&mut self.owned) {
            if self.images.is_resident(&image) {
                let _ = self.images.release(&image);
            }
        }
    }

    /// Close the loader, stopping animations and clearing caches.
    pub fn close(&mut self) {
        self.closed = true;
        self.animations.clear();
        self.loaded.clear();
        self.requests.clear();
    }

    fn animate(&mut self, texture: &SceneTexture, frames: Vec<RenderImage>) {
        if frames.len() < 2 {
            return;
        }
        self.animations.insert(
            texture.image.ordinal,
            AnimationState {
                image: texture.image.clone(),
                frames,
                previous: 0,
            },
        );
    }

    fn decode_with_host(&self, bytes: &[u8], path: &str) -> Result<DecodedSceneImage, RenderError> {
        self.decoder
            .as_ref()
            .map(|decoder| decoder.decode(bytes, path))
            .unwrap_or_else(|| Err(RenderError::Backend(format!("Unsupported scene image format: {path}"))))
    }

    #[allow(clippy::too_many_lines)]
    fn load_uncached(
        &mut self,
        name: &str,
        options: &SceneTextureLoadOptions,
    ) -> Result<Option<SceneTexture>, RenderError> {
        let dot = name.rfind('.');
        let slash = name.rfind('/');
        let explicit = dot.is_some_and(|dot| slash.is_none_or(|slash| dot > slash));
        let base = if explicit {
            &name[..dot.unwrap_or(name.len())]
        } else {
            name
        };
        let wall = options.usage == Some(ImageUsage::Wall)
            || options.usage.is_none() && (name.starts_with("textures/") || name.to_lowercase().ends_with(".wal"));
        let default_policy;
        let policy = match &self.policy {
            Some(policy) => Some(policy),
            None => {
                if options.family == TextureFamily::Q2 {
                    default_policy = default_image_policy();
                    Some(&default_policy)
                } else {
                    None
                }
            }
        };
        let source_extensions: &[&str] = match options.family {
            TextureFamily::Q2 => &[
                ".png",
                ".jpg",
                ".tga",
                ".jpeg",
                ".bmp",
                ".gif",
                if wall { ".wal" } else { ".pcx" },
            ],
            TextureFamily::Q1 => &[".lmp", ".tga", ".jpg", ".png", ".jpeg", ".pcx", ".bmp", ".gif"],
            TextureFamily::Q3 => &[".tga", ".jpg", ".png", ".jpeg", ".pcx", ".bmp", ".gif"],
        };
        let format_suffix = |format: &ImageFormat| match format {
            ImageFormat::Png => ".png",
            ImageFormat::Jpg => ".jpg",
            ImageFormat::Tga => ".tga",
            ImageFormat::Jpeg => ".jpeg",
            ImageFormat::Bmp => ".bmp",
            ImageFormat::Gif => ".gif",
        };
        let overrides: Vec<&str> = match policy.and_then(|policy| policy.formats.as_ref()) {
            Some(formats) => formats.iter().map(format_suffix).collect(),
            None => source_extensions
                .iter()
                .filter(|extension| ![".lmp", ".wal", ".pcx"].contains(extension))
                .copied()
                .collect(),
        };
        let mut extensions: Vec<&str> = if policy.is_some_and(|policy| policy.formats.is_some()) {
            overrides.clone()
        } else {
            source_extensions.to_vec()
        };
        if policy.is_some_and(|policy| policy.formats.is_some()) {
            if options.family == TextureFamily::Q1 {
                extensions.extend([".lmp", ".pcx"]);
            } else {
                extensions.push(if options.family == TextureFamily::Q2 && wall {
                    ".wal"
                } else {
                    ".pcx"
                });
            }
        }
        let requested_name = if !explicit && options.family == TextureFamily::Q2 && wall {
            format!("{name}.wal")
        } else {
            name.to_string()
        };
        let requested = if explicit {
            name[dot.unwrap_or(name.len()) + 1..].to_lowercase()
        } else if requested_name != name {
            "wal".to_string()
        } else {
            String::new()
        };
        let native =
            requested == "pcx" || requested == "wal" || options.family == TextureFamily::Q1 && requested == "lmp";
        let truecolor = ["png", "jpg", "tga", "jpeg", "bmp", "gif"].contains(&requested.as_str());
        let usage = options
            .usage
            .unwrap_or(if wall { ImageUsage::Wall } else { ImageUsage::Picture });
        let override_native = policy.is_some_and(|policy| {
            policy.override_level >= 1
                && policy.override_usages.contains(&usage)
                && (native || policy.override_level > 1 && truecolor)
        });
        let mut candidates = Vec::new();
        if override_native {
            candidates.extend(overrides.iter().map(|extension| format!("{base}{extension}")));
        }
        if explicit || requested_name != name {
            candidates.push(requested_name.clone());
        }
        candidates.extend(extensions.iter().map(|extension| format!("{base}{extension}")));
        let mut seen = HashSet::new();
        candidates.retain(|candidate| seen.insert(candidate.clone()));
        for path in candidates {
            let Some(asset) = self.reader.read(&path)? else {
                continue;
            };
            self.require_open()?;
            let suffix = path[path.rfind('.').unwrap_or(0)..].to_lowercase();
            let mut animation: Option<Vec<RenderImage>> = None;
            let mut content = match suffix.as_str() {
                ".gif" => {
                    let DecodedSceneImage::Animated(frames) = self.decode_with_host(&asset.bytes, &path)? else {
                        return Err(RenderError::Backend(format!("GIF decoder returned no frames: {path}")));
                    };
                    let frames = frames
                        .into_iter()
                        .map(|frame| {
                            let rgba = rgba_content(frame, options.mipmap);
                            if options.family == TextureFamily::Q2 && options.mipmap {
                                q2_mipmapped_image(&rgba)
                            } else {
                                rgba
                            }
                        })
                        .collect::<Vec<_>>();
                    let first = frames
                        .first()
                        .cloned()
                        .ok_or_else(|| RenderError::Backend(format!("GIF has no frames: {path}")))?;
                    animation = Some(frames);
                    first
                }
                ".lmp" => {
                    let palette = self.palette.clone().ok_or_else(|| {
                        RenderError::Backend("Quake picture textures require their content palette".to_string())
                    })?;
                    let pic =
                        qa_content::wad::decode_qpic(&asset.bytes, &path).map_err(|error| wire(error.to_string()))?;
                    if pic.width <= 0 || pic.height <= 0 {
                        return Err(wire(format!("Invalid LMP dimensions: {path}")));
                    }
                    indexed_content(
                        vec![(pic.width as u32, pic.height as u32, pic.indices.to_vec())],
                        palette,
                        PaletteTransparency::Opaque,
                        Some((self.fullbright_first, 255)),
                    )
                }
                ".wal" => {
                    let palette = self.palette.clone().ok_or_else(|| {
                        RenderError::Backend("WAL textures require their content palette".to_string())
                    })?;
                    let (width, height, levels) = decode_wal(&asset.bytes, &path)?;
                    indexed_content(
                        levels
                            .into_iter()
                            .enumerate()
                            .map(|(mip, indices)| ((width >> mip).max(1), (height >> mip).max(1), indices))
                            .collect(),
                        palette,
                        PaletteTransparency::Opaque,
                        None,
                    )
                }
                ".pcx" => {
                    let (width, height, indices, embedded) = decode_pcx(&asset.bytes, &path)?;
                    let embedded_palette = embedded.map(|colors| Palette {
                        colors,
                        source: path.clone(),
                    });
                    let palette = embedded_palette
                        .as_ref()
                        .or(self.palette.as_ref())
                        .ok_or_else(|| RenderError::Backend(format!("PCX has no palette: {path}")))?;
                    let resident = if options.family == TextureFamily::Q2
                        && matches!(options.usage, Some(ImageUsage::Skin | ImageUsage::Sprite))
                    {
                        self.palette.clone()
                    } else {
                        None
                    };
                    if let Some(resident) = resident {
                        let pixels = if options.usage == Some(ImageUsage::Skin) {
                            flood_skin(&indices, width, height, &resident)?
                        } else {
                            indices
                        };
                        indexed_content(
                            vec![(width, height, pixels)],
                            resident,
                            PaletteTransparency::Index(255),
                            None,
                        )
                    } else {
                        let colors = &palette.colors;
                        let mut pixels = vec![0u8; (width * height * 4) as usize];
                        for (offset, index) in indices.iter().enumerate() {
                            let mut color = *index;
                            if options.family == TextureFamily::Q2 && *index == 255 {
                                let above = offset
                                    .checked_sub(width as usize)
                                    .and_then(|at| indices.get(at).copied());
                                let below = indices.get(offset + width as usize).copied();
                                let left = offset.checked_sub(1).and_then(|at| indices.get(at).copied());
                                let right = indices.get(offset + 1).copied();
                                color = [above, below, left, right]
                                    .into_iter()
                                    .flatten()
                                    .find(|value| *value != 255)
                                    .unwrap_or(0);
                            }
                            let base = color as usize * 3;
                            let alpha = if options.family == TextureFamily::Q2 && *index == 255 {
                                0
                            } else {
                                255
                            };
                            pixels[offset * 4..offset * 4 + 4].copy_from_slice(&[
                                colors.get(base).copied().unwrap_or(0),
                                colors.get(base + 1).copied().unwrap_or(0),
                                colors.get(base + 2).copied().unwrap_or(0),
                                alpha,
                            ]);
                        }
                        rgba_content(RgbaLevel { width, height, pixels }, options.mipmap)
                    }
                }
                ".png" | ".tga" | ".bmp" | ".jpg" | ".jpeg" => {
                    let decoded = if suffix == ".tga" {
                        decode_tga(&asset.bytes, &path)?
                    } else {
                        match self.decode_with_host(&asset.bytes, &path)? {
                            DecodedSceneImage::Rgba(level) => level,
                            _ => {
                                return Err(RenderError::Backend(format!(
                                    "True-color decoder returned no image: {path}"
                                )));
                            }
                        }
                    };
                    rgba_content(decoded, options.mipmap)
                }
                _ => return Err(RenderError::Backend(format!("Unsupported scene image format: {path}"))),
            };
            let mut logical_size = match &content {
                RenderImage::Indexed8 { levels, .. } | RenderImage::Rgba8 { levels, .. } => {
                    levels.first().map(|level| (level.width, level.height))
                }
                RenderImage::Depth32f { levels } => levels.first().map(|level| (level.width, level.height)),
            };
            if options.family == TextureFamily::Q2
                && suffix != ".wal"
                && (name.to_lowercase().ends_with(".wal") || !explicit && wall)
            {
                let original = self
                    .reader
                    .read_original(&format!("{base}.wal"))?
                    .or_else(|| self.reader.read(&format!("{base}.wal")).unwrap_or(None));
                if let Some(original) = original {
                    let (width, height, _) = decode_wal(&original.bytes, &format!("{base}.wal"))?;
                    logical_size = Some((width, height));
                }
            }
            if options.family == TextureFamily::Q2 && name.to_lowercase().ends_with(".pcx") && suffix != ".pcx" {
                let requested = self
                    .reader
                    .read_original(name)?
                    .or_else(|| self.reader.read(name).unwrap_or(None));
                if let Some(requested) = requested {
                    let (width, height, _, _) = decode_pcx(&requested.bytes, name)?;
                    logical_size = Some((width, height));
                }
            }
            if options.family == TextureFamily::Q1 && requested == "lmp" && suffix != ".lmp" {
                let original = self
                    .reader
                    .read_original(name)?
                    .or_else(|| self.reader.read(name).unwrap_or(None));
                if let Some(original) = original {
                    let pic =
                        qa_content::wad::decode_qpic(&original.bytes, name).map_err(|error| wire(error.to_string()))?;
                    logical_size = Some((pic.width as u32, pic.height as u32));
                }
            }
            if animation.is_none() && options.family == TextureFamily::Q2 && options.mipmap {
                content = q2_mipmapped_image(&content);
            }
            let texture = self.register(
                name,
                content,
                TextureSampling {
                    repeat: options.repeat,
                    filter: if options.mipmap {
                        TextureFilter::LinearMipmapNearest
                    } else {
                        TextureFilter::Linear
                    },
                },
                Some(asset.source),
                logical_size,
            )?;
            if let Some(frames) = animation {
                self.animate(&texture, frames);
            }
            return Ok(Some(texture));
        }
        Ok(None)
    }
}

fn transparent_index(transparency: &PaletteTransparency) -> Option<u8> {
    match transparency {
        PaletteTransparency::Index(index) => Some(*index),
        PaletteTransparency::Q1Fence => Some(255),
        PaletteTransparency::Opaque => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::types::{fresh_owner_identity, ImageResourceOperation, ImageSource, ResourceOwner};
    use qa_core::identity::IdentityOwner;

    struct FakeReader {
        assets: HashMap<String, Vec<u8>>,
    }

    impl SceneAssetReader for FakeReader {
        fn read(&self, path: &str) -> Result<Option<SceneAsset>, RenderError> {
            Ok(self.assets.get(path).map(|bytes| SceneAsset {
                bytes: bytes.clone(),
                source: ImageSource::Resource {
                    requested_path: path.to_string(),
                },
            }))
        }
    }

    struct FakeDecoder;

    impl SceneImageDecoder for FakeDecoder {
        fn decode(&self, bytes: &[u8], path: &str) -> Result<DecodedSceneImage, RenderError> {
            if path.ends_with(".gif") {
                return Ok(DecodedSceneImage::Animated(vec![
                    RgbaLevel {
                        width: 2,
                        height: 1,
                        pixels: vec![255, 0, 0, 255, 0, 255, 0, 255],
                    },
                    RgbaLevel {
                        width: 2,
                        height: 1,
                        pixels: vec![0, 0, 255, 255, 255, 255, 0, 255],
                    },
                ]));
            }
            Ok(DecodedSceneImage::Rgba(RgbaLevel {
                width: 2,
                height: 1,
                pixels: bytes.iter().cycle().take(8).copied().collect(),
            }))
        }
    }

    fn owner() -> ResourceOwner {
        let session = IdentityOwner::create("textures-test")
            .expect("session")
            .session()
            .clone();
        ResourceOwner::new(fresh_owner_identity(), session, 0)
    }

    fn palette() -> Palette {
        let mut colors = vec![0u8; 768];
        colors[3] = 255;
        Palette {
            colors,
            source: "test".to_string(),
        }
    }

    fn loader(reader: FakeReader) -> SceneTextureLoader {
        let registry = SceneImageRegistry::new(owner());
        let mut loader = SceneTextureLoader::new(registry, Box::new(reader), Some(palette()), None, 224).unwrap();
        loader.set_decoder(Box::new(FakeDecoder));
        loader
    }

    fn tga(width: u32, height: u32, pixels: &[[u8; 3]]) -> Vec<u8> {
        let mut bytes = vec![0u8; 18];
        bytes[2] = 2;
        bytes[12..14].copy_from_slice(&(width as u16).to_le_bytes());
        bytes[14..16].copy_from_slice(&(height as u16).to_le_bytes());
        bytes[16] = 24;
        for pixel in pixels {
            bytes.extend([pixel[2], pixel[1], pixel[0]]);
        }
        bytes
    }

    #[test]
    fn white_and_missing_are_resident() {
        let loader = loader(FakeReader { assets: HashMap::new() });
        assert!(loader.images().is_resident(&loader.white().image));
        assert!(loader.images().is_resident(&loader.missing().image));
        assert_eq!((loader.white().width, loader.white().height), (1, 1));
    }

    #[test]
    fn indexed_registration_derives_fullbright() {
        let mut loader = loader(FakeReader { assets: HashMap::new() });
        let texture = loader
            .register(
                "full",
                indexed_content(
                    vec![(2, 1, vec![0, 1])],
                    palette(),
                    PaletteTransparency::Opaque,
                    Some((1, 255)),
                ),
                TextureSampling {
                    repeat: true,
                    filter: TextureFilter::Linear,
                },
                None,
                None,
            )
            .unwrap();
        assert!(texture.fullbright.is_some());
        let operations = loader.images_mut().drain_operations();
        assert!(operations
            .iter()
            .any(|operation| matches!(operation, ImageResourceOperation::CreateImage { .. })));
    }

    #[test]
    fn q1_embedded_rules_cover_fence_sky_and_external() {
        let mut loader = loader(FakeReader { assets: HashMap::new() });
        let embedded = MipTexture::Embedded {
            name: "{fence".to_string(),
            width: 16,
            height: 16,
            levels: [&[1u8; 256][..], &[1u8; 64][..], &[1u8; 16][..], &[1u8; 4][..]],
        };
        let texture = loader.q1_embedded(&embedded).unwrap().expect("fence");
        assert!(matches!(
            texture.content,
            RenderImage::Indexed8 {
                transparency: PaletteTransparency::Q1Fence,
                ..
            }
        ));
        let sky = MipTexture::Embedded {
            name: "sky1".to_string(),
            width: 16,
            height: 16,
            levels: [&[1u8; 256][..], &[1u8; 64][..], &[1u8; 16][..], &[1u8; 4][..]],
        };
        let texture = loader.q1_embedded(&sky).unwrap().expect("sky");
        assert!(texture.fullbright.is_none());
        let external = MipTexture::External {
            name: "ext".to_string(),
            width: 16,
            height: 16,
        };
        assert!(loader.q1_embedded(&external).unwrap().is_none());
    }

    #[test]
    fn load_follows_candidates_and_caches() {
        let mut assets = HashMap::new();
        assets.insert(
            "textures/rock.tga".to_string(),
            tga(2, 1, &[[10, 20, 30], [40, 50, 60]]),
        );
        let mut loader = loader(FakeReader { assets });
        let options = SceneTextureLoadOptions::default();
        let first = loader.load("textures/rock", &options).unwrap().expect("rock");
        assert_eq!((first.width, first.height), (2, 1));
        let second = loader.load("textures/rock", &options).unwrap().expect("cached");
        assert_eq!(first.image.ordinal, second.image.ordinal);
        assert!(loader.load("textures/absent", &options).unwrap().is_none());
    }

    fn pcx(width: u32, height: u32, runs: &[(u8, u8)], palette: Option<[u8; 768]>) -> Vec<u8> {
        let mut bytes = vec![0u8; 128];
        bytes[0] = 0x0a;
        bytes[2] = 1;
        bytes[3] = 8;
        bytes[8..10].copy_from_slice(&((width - 1) as u16).to_le_bytes());
        bytes[10..12].copy_from_slice(&((height - 1) as u16).to_le_bytes());
        bytes[64] = 1;
        bytes[66..68].copy_from_slice(&(width as u16).to_le_bytes());
        for (count, value) in runs {
            if *count == 1 && *value < 0xc0 {
                bytes.push(*value);
            } else {
                bytes.push(0xc0 | *count);
                bytes.push(*value);
            }
        }
        if let Some(palette) = palette {
            bytes.push(0x0c);
            bytes.extend(palette);
        }
        bytes
    }

    #[test]
    fn pcx_rle_and_embedded_palette_decode() {
        let mut embedded = [0u8; 768];
        embedded[7 * 3] = 200;
        embedded[7 * 3 + 1] = 100;
        embedded[7 * 3 + 2] = 50;
        let mut assets = HashMap::new();
        assets.insert("pic.pcx".to_string(), pcx(2, 1, &[(2, 7)], Some(embedded)));
        let mut loader = loader(FakeReader { assets });
        let texture = loader
            .load("pic.pcx", &SceneTextureLoadOptions::default())
            .unwrap()
            .expect("pcx");
        match &texture.content {
            RenderImage::Rgba8 { levels, .. } => {
                assert_eq!(levels[0].pixels[0..4], [200, 100, 50, 255]);
            }
            _ => panic!("pcx should expand to rgba"),
        }
        assert!(loader
            .load("pic.pcx", &SceneTextureLoadOptions::default())
            .unwrap()
            .is_some());
    }

    #[test]
    fn sample_surface_truncates_without_mipmap() {
        let mut assets = HashMap::new();
        assets.insert("wall.tga".to_string(), tga(4, 4, &[[1, 2, 3]; 16]));
        let mut loader = loader(FakeReader { assets });
        let texture = loader
            .load("wall", &SceneTextureLoadOptions::default())
            .unwrap()
            .expect("wall");
        let levels = match &texture.content {
            RenderImage::Rgba8 { levels, .. } => levels.len(),
            _ => 0,
        };
        assert!(levels > 1);
        let sampled = loader.sample_surface(&texture, false, true).unwrap();
        let sampled_levels = match &sampled.content {
            RenderImage::Rgba8 { levels, .. } => levels.len(),
            _ => 0,
        };
        assert_eq!(sampled_levels, 1);
    }

    #[test]
    fn gif_animation_ticks_at_ten_hertz() {
        let mut assets = HashMap::new();
        assets.insert("anim.gif".to_string(), vec![0x47, 0x49, 0x46]);
        let mut loader = loader(FakeReader { assets });
        let texture = loader
            .load("anim.gif", &SceneTextureLoadOptions::default())
            .unwrap()
            .expect("gif");
        let _ = loader.images_mut().drain_operations();
        loader.tick_animations(0.0);
        assert!(loader.images_mut().drain_operations().is_empty());
        let _ = &texture;
        loader.tick_animations(150.0);
        let operations = loader.images_mut().drain_operations();
        assert!(operations
            .iter()
            .all(|operation| matches!(operation, ImageResourceOperation::UpdateImage { .. })));
        assert!(!operations.is_empty());
    }

    #[test]
    fn closed_loader_rejects_work() {
        let mut loader = loader(FakeReader { assets: HashMap::new() });
        loader.close();
        assert!(loader.is_closed());
        assert!(loader.load("x", &SceneTextureLoadOptions::default()).is_err());
    }
}
