//! CPU texture storage.
//!
//! Donor provenance: `src/render/cpu/textures.ts` in full — CPU texture
//! storage adapted from the Quake III `tr_image.c` `R_CreateImage` path.
//! Original renderer copyright (C) 1999-2005 Id Software, Inc.
//! Indexed expansion follows `src/formats/images/palette.ts`
//! `expandIndexedImage` (combined layer).

use std::collections::HashMap;
use std::sync::Arc;

use qa_core::math::{vec4, Vec4};

use super::super::error::RenderError;
use super::super::types::{
    DepthImageLevel, ImageLevel, ImageResourceOperation, LevelContent, PaletteTransparency, RenderImage, RendererImage,
    ResourceOwner, TextureBinding, TextureFilter, TextureSampling,
};
use super::triangle_kernel::{
    BoundTexture, ConstantTexture, ImageInternalFormat, MipMapping, Sample, TextureStorage, UploadedTexture,
};

/// Stored per-image content beyond the expanded RGBA levels.
#[derive(Debug, Clone)]
enum StoredContent {
    /// True-color image; levels hold the pixels directly.
    Rgba,
    /// Indexed image; updates expand through these parameters.
    Indexed {
        /// 768 RGB palette bytes.
        palette: Vec<u8>,
        /// Transparency rule.
        transparency: PaletteTransparency,
        /// Optional 256-entry colormap translation.
        translation: Option<Vec<u8>>,
    },
}

#[derive(Debug)]
struct TextureObject {
    content: StoredContent,
    levels: Vec<TextureStorage>,
    mipmap: bool,
    sampling: TextureSampling,
    border_color: Vec4,
}

/// Shared depth-atlas level for shadow sampling.
#[derive(Debug, Clone)]
pub struct SharedDepth {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// Row-major depth samples.
    pub pixels: Arc<Vec<f32>>,
}

#[derive(Debug)]
struct DepthObject {
    width: u32,
    height: u32,
    pixels: Arc<Vec<f32>>,
}

fn storage(level: &ImageLevel) -> Result<TextureStorage, RenderError> {
    if level.width < 1 || level.height < 1 || level.pixels.len() != level.width as usize * level.height as usize * 4 {
        return Err(RenderError::BadDimensions {
            width: level.width,
            height: level.height,
            detail: "invalid CPU RGBA image level".to_string(),
        });
    }
    let uniform = level.pixels.as_chunks::<4>().0.iter().next().and_then(|first| {
        level
            .pixels
            .as_chunks::<4>()
            .0
            .iter()
            .all(|chunk| chunk == first)
            .then(|| Sample {
                r: f32::from(first[0]) / 255.0,
                g: f32::from(first[1]) / 255.0,
                b: f32::from(first[2]) / 255.0,
                a: f32::from(first[3]) / 255.0,
            })
    });
    Ok(TextureStorage {
        width: level.width,
        height: level.height,
        pixels: Arc::new(level.pixels.clone()),
        internal_format: ImageInternalFormat::Rgba8,
        has_alpha: true,
        uniform,
    })
}

/// Expand one indexed level to RGBA.
///
/// Transparency tests the original index; the translated index selects the
/// palette entry. The combined layer keeps every non-transparent texel.
pub fn expand_indexed_level(
    pixels: &[u8],
    width: u32,
    height: u32,
    palette: &[u8],
    transparency: &PaletteTransparency,
    translation: Option<&[u8]>,
) -> Result<ImageLevel, RenderError> {
    if width < 1 || height < 1 || pixels.len() != width as usize * height as usize {
        return Err(RenderError::BadDimensions {
            width,
            height,
            detail: "indexed level pixels must hold one byte per pixel".to_string(),
        });
    }
    if palette.len() != 768 {
        return Err(RenderError::BadDimensions {
            width,
            height,
            detail: format!("Quake palette must contain 768 bytes, got {}", palette.len()),
        });
    }
    if let Some(table) = translation {
        if table.len() != 256 {
            return Err(RenderError::BadDimensions {
                width,
                height,
                detail: format!("palette translation must contain 256 bytes, got {}", table.len()),
            });
        }
    }
    let transparent = match transparency {
        PaletteTransparency::Opaque => None,
        PaletteTransparency::Index(index) => Some(*index),
        PaletteTransparency::Q1Fence => Some(255),
    };
    let mut expanded = Vec::with_capacity(pixels.len() * 4);
    for original in pixels {
        let index = translation.map_or(*original, |table| table[usize::from(*original)]);
        let base = usize::from(index) * 3;
        expanded.push(palette[base]);
        expanded.push(palette[base + 1]);
        expanded.push(palette[base + 2]);
        expanded.push(u8::from(transparent != Some(*original)) * 255);
    }
    Ok(ImageLevel {
        width,
        height,
        pixels: expanded,
    })
}

const fn mip_filter(filter: TextureFilter) -> bool {
    !matches!(filter, TextureFilter::Nearest | TextureFilter::Linear)
}

const fn linear_filter(filter: TextureFilter) -> bool {
    matches!(
        filter,
        TextureFilter::Linear | TextureFilter::LinearMipmapNearest | TextureFilter::LinearMipmapLinear
    )
}

/// Uploaded pixels are detached from the caller; image ordinals belong to one renderer generation.
#[derive(Debug, Default)]
pub struct CpuImages {
    textures: HashMap<u32, TextureObject>,
    depths: HashMap<u32, Vec<DepthObject>>,
    units: [Option<u32>; 2],
    owner: Option<ResourceOwner>,
}

impl CpuImages {
    /// Create an image store for one renderer lifetime.
    #[must_use]
    pub fn new(owner: ResourceOwner) -> Self {
        Self {
            owner: Some(owner),
            ..Self::default()
        }
    }

    fn owner(&self) -> &ResourceOwner {
        self.owner.as_ref().expect("CPU image store has no owner")
    }

    fn require_owner(&self, image: &RendererImage) -> Result<(), RenderError> {
        self.owner()
            .require(&image.owner, "CPU image belongs to another renderer owner")
    }

    fn require(&self, image: &RendererImage) -> Result<&TextureObject, RenderError> {
        self.require_owner(image)?;
        self.textures
            .get(&image.ordinal)
            .ok_or(RenderError::UnknownImage(image.ordinal))
    }

    /// Fail unless the image is uploaded to this store.
    pub fn validate(&self, image: &RendererImage) -> Result<(), RenderError> {
        self.require(image).map(|_| ())
    }

    /// Base depth level shared for shadow sampling.
    pub fn depth_shared(&self, image: &RendererImage) -> Result<SharedDepth, RenderError> {
        self.require_owner(image)?;
        let levels = self.depths.get(&image.ordinal).ok_or(RenderError::PixelMismatch(
            image.ordinal,
            "CPU image is not a registered depth atlas".to_string(),
        ))?;
        let base = &levels[0];
        Ok(SharedDepth {
            width: base.width,
            height: base.height,
            pixels: Arc::clone(&base.pixels),
        })
    }

    /// Base depth dimensions.
    pub fn depth_dimensions(&self, image: &RendererImage) -> Result<(u32, u32), RenderError> {
        let shared = self.depth_shared(image)?;
        Ok((shared.width, shared.height))
    }

    /// Copy the base depth level out for atlas rendering.
    pub fn depth_copy_out(&self, image: &RendererImage) -> Result<Vec<f32>, RenderError> {
        Ok(self.depth_shared(image)?.pixels.as_ref().clone())
    }

    /// Write the base depth level back after atlas rendering.
    pub fn depth_copy_in(&mut self, image: &RendererImage, pixels: &[f32]) -> Result<(), RenderError> {
        self.require_owner(image)?;
        let levels = self.depths.get_mut(&image.ordinal).ok_or(RenderError::PixelMismatch(
            image.ordinal,
            "CPU image is not a registered depth atlas".to_string(),
        ))?;
        let base = &mut levels[0];
        if pixels.len() != base.width as usize * base.height as usize {
            return Err(RenderError::BadDimensions {
                width: base.width,
                height: base.height,
                detail: "CPU depth writeback dimensions disagree".to_string(),
            });
        }
        let owned = Arc::make_mut(&mut base.pixels);
        owned.copy_from_slice(pixels);
        Ok(())
    }

    /// Apply an image resource operation, panicking on invalid input.
    pub fn apply(&mut self, operation: &ImageResourceOperation) {
        if let Err(error) = self.try_apply(operation) {
            panic!("{error}");
        }
    }

    /// Apply an image resource operation, reporting failures.
    pub fn try_apply(&mut self, operation: &ImageResourceOperation) -> Result<(), RenderError> {
        match operation {
            ImageResourceOperation::CreateImage {
                image,
                content,
                sampling,
            } => {
                self.require_owner(image)?;
                if self.textures.contains_key(&image.ordinal) || self.depths.contains_key(&image.ordinal) {
                    return Err(RenderError::ImageConflict(image.ordinal));
                }
                if let RenderImage::Depth32f { levels } = content {
                    let [base, rest @ ..] = levels.as_slice() else {
                        return Err(RenderError::BadLevel {
                            ordinal: image.ordinal,
                            level: 0,
                        });
                    };
                    let copy = |level: &DepthImageLevel| -> Result<DepthObject, RenderError> {
                        if level.width < 1
                            || level.height < 1
                            || level.pixels.len() != level.width as usize * level.height as usize
                        {
                            return Err(RenderError::BadDimensions {
                                width: level.width,
                                height: level.height,
                                detail: "invalid CPU depth image dimensions".to_string(),
                            });
                        }
                        Ok(DepthObject {
                            width: level.width,
                            height: level.height,
                            pixels: Arc::new(level.pixels.clone()),
                        })
                    };
                    let mut stored = Vec::with_capacity(levels.len());
                    stored.push(copy(base)?);
                    for level in rest {
                        stored.push(copy(level)?);
                    }
                    self.depths.insert(image.ordinal, stored);
                    return Ok(());
                }
                let base = match content {
                    RenderImage::Indexed8 { levels, .. } | RenderImage::Rgba8 { levels, .. } => {
                        levels.first().ok_or(RenderError::BadLevel {
                            ordinal: image.ordinal,
                            level: 0,
                        })?
                    }
                    RenderImage::Depth32f { .. } => unreachable!("depth content handled above"),
                };
                if base.width != image.width || base.height != image.height {
                    return Err(RenderError::BadDimensions {
                        width: base.width,
                        height: base.height,
                        detail: "CPU image dimensions disagree".to_string(),
                    });
                }
                let (stored_content, border_color) = match content {
                    RenderImage::Rgba8 { border_color, .. } => (StoredContent::Rgba, *border_color),
                    RenderImage::Indexed8 {
                        palette,
                        transparency,
                        translation,
                        ..
                    } => (
                        StoredContent::Indexed {
                            palette: palette.colors.clone(),
                            transparency: transparency.clone(),
                            translation: translation.clone(),
                        },
                        vec4(0.0, 0.0, 0.0, 0.0),
                    ),
                    RenderImage::Depth32f { .. } => unreachable!("depth content handled above"),
                };
                let levels = match content {
                    RenderImage::Rgba8 { levels, .. } => levels.iter().map(storage).collect::<Result<Vec<_>, _>>()?,
                    RenderImage::Indexed8 {
                        levels,
                        palette,
                        transparency,
                        translation,
                        ..
                    } => levels
                        .iter()
                        .map(|level| {
                            expand_indexed_level(
                                &level.pixels,
                                level.width,
                                level.height,
                                &palette.colors,
                                transparency,
                                translation.as_deref(),
                            )
                            .and_then(|expanded| storage(&expanded))
                        })
                        .collect::<Result<Vec<_>, _>>()?,
                    RenderImage::Depth32f { .. } => unreachable!("depth content handled above"),
                };
                self.textures.insert(
                    image.ordinal,
                    TextureObject {
                        content: stored_content,
                        levels,
                        mipmap: mip_filter(sampling.filter),
                        sampling: *sampling,
                        border_color,
                    },
                );
                Ok(())
            }
            ImageResourceOperation::UpdateImage { image, level, content } => {
                if let LevelContent::Depth(incoming) = content {
                    self.require_owner(image)?;
                    let levels = self.depths.get_mut(&image.ordinal).ok_or(RenderError::PixelMismatch(
                        image.ordinal,
                        "CPU image is not a registered depth atlas".to_string(),
                    ))?;
                    let stored = levels.get_mut(*level as usize).ok_or(RenderError::BadLevel {
                        ordinal: image.ordinal,
                        level: *level,
                    })?;
                    if stored.width != incoming.width
                        || stored.height != incoming.height
                        || stored.pixels.len() != incoming.pixels.len()
                    {
                        return Err(RenderError::BadDimensions {
                            width: incoming.width,
                            height: incoming.height,
                            detail: "CPU depth update dimensions disagree".to_string(),
                        });
                    }
                    Arc::make_mut(&mut stored.pixels).copy_from_slice(&incoming.pixels);
                    return Ok(());
                }
                let LevelContent::Rgba(incoming) = content else {
                    unreachable!("depth content handled above")
                };
                self.require_owner(image)?;
                let object = self
                    .textures
                    .get_mut(&image.ordinal)
                    .ok_or(RenderError::UnknownImage(image.ordinal))?;
                let stored = object.levels.get(*level as usize).ok_or(RenderError::BadLevel {
                    ordinal: image.ordinal,
                    level: *level,
                })?;
                if stored.width != incoming.width || stored.height != incoming.height {
                    return Err(RenderError::BadDimensions {
                        width: incoming.width,
                        height: incoming.height,
                        detail: "CPU update dimensions must match the existing image level".to_string(),
                    });
                }
                let expanded = match &object.content {
                    StoredContent::Rgba => {
                        if incoming.pixels.len() != incoming.width as usize * incoming.height as usize * 4 {
                            return Err(RenderError::BadDimensions {
                                width: incoming.width,
                                height: incoming.height,
                                detail: "invalid CPU RGBA image level".to_string(),
                            });
                        }
                        incoming.clone()
                    }
                    StoredContent::Indexed {
                        palette,
                        transparency,
                        translation,
                    } => expand_indexed_level(
                        &incoming.pixels,
                        incoming.width,
                        incoming.height,
                        palette,
                        transparency,
                        translation.as_deref(),
                    )?,
                };
                object.levels[*level as usize] = storage(&expanded)?;
                Ok(())
            }
            ImageResourceOperation::ReleaseImage { image } => {
                self.require_owner(image)?;
                if self.depths.remove(&image.ordinal).is_some() {
                    return Ok(());
                }
                self.require(image)?;
                self.textures.remove(&image.ordinal);
                for unit in &mut self.units {
                    if *unit == Some(image.ordinal) {
                        *unit = None;
                    }
                }
                Ok(())
            }
            ImageResourceOperation::TextureMode { filter } => {
                for object in self.textures.values_mut() {
                    if object.mipmap {
                        object.sampling.filter = *filter;
                    }
                }
                Ok(())
            }
        }
    }

    /// Bind an image to one texture unit.
    pub fn bind(&mut self, unit: u32, binding: &TextureBinding) {
        assert!(unit <= 1, "CPU texture unit {unit} is outside its image");
        match binding {
            TextureBinding::DynamicImage(_) => panic!("dynamic texture must resolve before binding"),
            TextureBinding::RetainCurrentTexture => {}
            TextureBinding::BindImage(image) => {
                if let Err(error) = self.require(image) {
                    panic!("{error}");
                }
                self.units[unit as usize] = Some(image.ordinal);
            }
        }
    }

    /// Resolve the texture bound to one unit.
    #[must_use]
    pub fn bound(&self, unit: u32) -> BoundTexture {
        assert!(unit <= 1, "CPU texture unit {unit} is outside its image");
        let ordinal = self.units[unit as usize];
        let Some(object) = ordinal.and_then(|ordinal| self.textures.get(&ordinal)) else {
            return BoundTexture::Incomplete;
        };
        let Some(base) = object.levels.first() else {
            return BoundTexture::Incomplete;
        };
        // Uniform textures sample one constant color: repeat wraps inside the
        // level, nearest clamps inside, and linear clamp taps match the border.
        if let Some(sample) = base.uniform {
            let linear = linear_filter(object.sampling.filter);
            let every_uniform = object.levels.iter().all(|level| level.uniform == Some(sample));
            let border_safe = object.sampling.repeat
                || !linear
                || (object.border_color.x == sample.r
                    && object.border_color.y == sample.g
                    && object.border_color.z == sample.b
                    && object.border_color.w == sample.a);
            if every_uniform && border_safe {
                return BoundTexture::Constant(ConstantTexture {
                    internal_format: ImageInternalFormat::Rgba8,
                    sample,
                });
            }
        }
        let filter = object.sampling.filter;
        let linear = linear_filter(filter);
        let mipmapping = if !mip_filter(filter) {
            MipMapping::None
        } else if matches!(
            filter,
            TextureFilter::NearestMipmapLinear | TextureFilter::LinearMipmapLinear
        ) {
            MipMapping::Linear
        } else {
            MipMapping::Nearest
        };
        let mut levels = vec![base.clone()];
        if mipmapping != MipMapping::None {
            let (mut width, mut height) = (base.width, base.height);
            for child in object.levels.iter().skip(1) {
                if width <= 1 && height <= 1 {
                    break;
                }
                width = (width / 2).max(1);
                height = (height / 2).max(1);
                if child.width != width || child.height != height {
                    return BoundTexture::Incomplete;
                }
                levels.push(child.clone());
            }
        }
        BoundTexture::Image(UploadedTexture {
            internal_format: ImageInternalFormat::Rgba8,
            repeat: object.sampling.repeat,
            minify_linear: linear,
            magnify_linear: linear,
            mipmapping,
            magnification_limit: 1.0,
            levels,
            border_color: object.border_color,
        })
    }

    /// Release every image and unbind both units.
    pub fn clear(&mut self) {
        self.textures.clear();
        self.depths.clear();
        self.units = [None, None];
    }
}

#[cfg(test)]
mod tests {
    use qa_core::identity::IdentityOwner;
    use qa_core::math::vec4;

    use super::super::super::types::{fresh_owner_identity, ImageSource, ResourceOwner};
    use super::super::triangle_kernel::sample_bound;
    use super::*;

    fn owner() -> ResourceOwner {
        let authority = IdentityOwner::create("cpu-textures").unwrap();
        ResourceOwner::new(fresh_owner_identity(), authority.session().clone(), 0)
    }

    fn image(owner: &ResourceOwner, ordinal: u32, width: u32, height: u32) -> RendererImage {
        RendererImage {
            owner: owner.clone(),
            ordinal,
            source: ImageSource::Generated {
                name: format!("test{ordinal}"),
            },
            width,
            height,
        }
    }

    fn sampling() -> TextureSampling {
        TextureSampling {
            repeat: true,
            filter: TextureFilter::Nearest,
        }
    }

    #[test]
    fn indexed_expansion_applies_palette_transparency_and_translation() {
        let mut palette = vec![0u8; 768];
        palette[0..3].copy_from_slice(&[10, 20, 30]);
        palette[3..6].copy_from_slice(&[40, 50, 60]);
        palette[765..768].copy_from_slice(&[70, 80, 90]);
        let level = expand_indexed_level(&[0, 1, 255, 0], 2, 2, &palette, &PaletteTransparency::Q1Fence, None).unwrap();
        assert_eq!(
            level.pixels,
            vec![10, 20, 30, 255, 40, 50, 60, 255, 70, 80, 90, 0, 10, 20, 30, 255]
        );
        let mut translation = vec![0u8; 256];
        translation[0] = 1;
        let translated =
            expand_indexed_level(&[0], 1, 1, &palette, &PaletteTransparency::Opaque, Some(&translation)).unwrap();
        assert_eq!(translated.pixels, vec![40, 50, 60, 255]);
    }

    #[test]
    fn upload_bind_and_sample_round_trip() {
        let owner = owner();
        let mut images = CpuImages::new(owner.clone());
        let handle = image(&owner, 3, 2, 1);
        images.apply(&ImageResourceOperation::CreateImage {
            image: handle.clone(),
            content: RenderImage::Rgba8 {
                levels: vec![ImageLevel {
                    width: 2,
                    height: 1,
                    pixels: vec![255, 0, 0, 255, 0, 255, 0, 128],
                }],
                border_color: vec4(0.0, 0.0, 0.0, 0.0),
            },
            sampling: sampling(),
        });
        images.bind(0, &TextureBinding::BindImage(handle.clone()));
        let bound = images.bound(0);
        let mut out = Sample::default();
        sample_bound(&bound, 0.1, 0.5, 0.0, &mut out);
        assert_eq!((out.r, out.g, out.b, out.a), (1.0, 0.0, 0.0, 1.0));
        sample_bound(&bound, 0.9, 0.5, 0.0, &mut out);
        assert!((out.a - 128.0 / 255.0).abs() < 1e-6);
        assert!(matches!(images.bound(1), BoundTexture::Incomplete));
        images.apply(&ImageResourceOperation::ReleaseImage { image: handle });
        assert!(matches!(images.bound(0), BoundTexture::Incomplete));
    }

    #[test]
    fn indexed_upload_samples_transparent_texels() {
        let owner = owner();
        let mut images = CpuImages::new(owner.clone());
        let handle = image(&owner, 5, 2, 1);
        let mut palette = vec![0u8; 768];
        palette[0..3].copy_from_slice(&[200, 100, 50]);
        palette[3..6].copy_from_slice(&[1, 2, 3]);
        images.apply(&ImageResourceOperation::CreateImage {
            image: handle.clone(),
            content: RenderImage::Indexed8 {
                levels: vec![ImageLevel {
                    width: 2,
                    height: 1,
                    pixels: vec![0, 1],
                }],
                palette: super::super::super::types::Palette {
                    colors: palette,
                    source: "test".to_string(),
                },
                transparency: PaletteTransparency::Index(1),
                fullbright: None,
                translation: None,
            },
            sampling: sampling(),
        });
        images.bind(0, &TextureBinding::BindImage(handle));
        let mut out = Sample::default();
        sample_bound(&images.bound(0), 0.1, 0.5, 0.0, &mut out);
        assert!((out.r - 200.0 / 255.0).abs() < 1e-6);
        assert_eq!(out.a, 1.0);
        sample_bound(&images.bound(0), 0.9, 0.5, 0.0, &mut out);
        assert_eq!(out.a, 0.0);
    }

    #[test]
    fn depth_atlas_upload_and_update_round_trip() {
        let owner = owner();
        let mut images = CpuImages::new(owner.clone());
        let handle = image(&owner, 7, 2, 2);
        images.apply(&ImageResourceOperation::CreateImage {
            image: handle.clone(),
            content: RenderImage::Depth32f {
                levels: vec![DepthImageLevel {
                    width: 2,
                    height: 2,
                    pixels: vec![0.5; 4],
                }],
            },
            sampling: sampling(),
        });
        assert_eq!(images.depth_copy_out(&handle).unwrap(), vec![0.5; 4]);
        images.apply(&ImageResourceOperation::UpdateImage {
            image: handle.clone(),
            level: 0,
            content: LevelContent::Depth(DepthImageLevel {
                width: 2,
                height: 2,
                pixels: vec![0.25; 4],
            }),
        });
        assert_eq!(images.depth_shared(&handle).unwrap().pixels.as_ref(), &vec![0.25; 4]);
    }

    #[test]
    fn foreign_owner_is_rejected() {
        let mut images = CpuImages::new(owner());
        let foreign = image(&owner(), 1, 1, 1);
        let result = images.try_apply(&ImageResourceOperation::ReleaseImage { image: foreign });
        assert!(matches!(result, Err(RenderError::ForeignOwner(_))));
    }
}
