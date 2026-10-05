//! GL texture lifetime and filtering (donor `src/render/gl/textures.ts`).

use std::collections::HashMap;

use super::{
    GlContext, CLAMP, CLAMP_TO_EDGE, DEPTH_COMPONENT, DEPTH_COMPONENT32F, FLOAT, LINEAR, LINEAR_MIPMAP_LINEAR,
    LINEAR_MIPMAP_NEAREST, NEAREST, NEAREST_MIPMAP_LINEAR, NEAREST_MIPMAP_NEAREST, PACK_ALIGNMENT, PACK_ROW_LENGTH,
    PACK_SKIP_PIXELS, PACK_SKIP_ROWS, PACK_SWAP_BYTES, PIXEL_PACK_BUFFER_BINDING, PIXEL_UNPACK_BUFFER_BINDING, REPEAT,
    RGBA, RGBA8, TEXTURE_2D, TEXTURE_BORDER_COLOR, TEXTURE_MAG_FILTER, TEXTURE_MAX_LEVEL, TEXTURE_MIN_FILTER,
    TEXTURE_WRAP_S, TEXTURE_WRAP_T, UNPACK_ALIGNMENT, UNPACK_ROW_LENGTH, UNPACK_SKIP_PIXELS, UNPACK_SKIP_ROWS,
    UNPACK_SWAP_BYTES, UNSIGNED_BYTE,
};
use crate::render::error::RenderError;
use crate::render::types::{
    DepthImageLevel, ImageLevel, ImageResourceOperation, LevelContent, PaletteTransparency, RenderImage, RendererImage,
    ResourceOwner, TextureBinding, TextureFilter,
};

/// Pixel-transfer direction for [`with_pixel_store`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PixelDirection {
    Pack,
    Unpack,
}

/// Native transfers must see tightly packed client memory, never a PBO offset.
pub fn with_pixel_store<C: GlContext>(gl: &mut C, direction: PixelDirection, operation: impl FnOnce(&mut C)) {
    let binding = match direction {
        PixelDirection::Pack => PIXEL_PACK_BUFFER_BINDING,
        PixelDirection::Unpack => PIXEL_UNPACK_BUFFER_BINDING,
    };
    let mut state = [0];
    gl.get_integerv(binding, &mut state);
    if state[0] != 0 {
        panic!(
            "{}",
            RenderError::Backend(
                "OpenGL pixel transfers require client memory without a bound pixel buffer".to_string()
            )
        );
    }
    let settings = match direction {
        PixelDirection::Pack => [
            PACK_ALIGNMENT,
            PACK_ROW_LENGTH,
            PACK_SKIP_PIXELS,
            PACK_SKIP_ROWS,
            PACK_SWAP_BYTES,
        ],
        PixelDirection::Unpack => [
            UNPACK_ALIGNMENT,
            UNPACK_ROW_LENGTH,
            UNPACK_SKIP_PIXELS,
            UNPACK_SKIP_ROWS,
            UNPACK_SWAP_BYTES,
        ],
    };
    let mut saved = [(0u32, 0i32); 5];
    for (slot, name) in settings.iter().enumerate() {
        gl.get_integerv(*name, &mut state);
        saved[slot] = (*name, state[0]);
    }
    for (slot, name) in settings.iter().enumerate() {
        gl.pixel_storei(*name, i32::from(slot == 0));
    }
    operation(gl);
    for (name, value) in saved {
        gl.pixel_storei(name, value);
    }
}

fn filters(filter: TextureFilter) -> (i32, i32) {
    match filter {
        TextureFilter::Nearest => (NEAREST, NEAREST),
        TextureFilter::Linear => (LINEAR, LINEAR),
        TextureFilter::NearestMipmapNearest => (NEAREST_MIPMAP_NEAREST, NEAREST),
        TextureFilter::LinearMipmapNearest => (LINEAR_MIPMAP_NEAREST, LINEAR),
        TextureFilter::NearestMipmapLinear => (NEAREST_MIPMAP_LINEAR, NEAREST),
        TextureFilter::LinearMipmapLinear => (LINEAR_MIPMAP_LINEAR, LINEAR),
    }
}

/// Expand one indexed level to RGBA, mirroring donor `expandIndexedImage`.
#[must_use]
pub fn expand_indexed_level(
    pixels: &[u8],
    width: u32,
    height: u32,
    palette: &[u8],
    transparency: &PaletteTransparency,
    translation: Option<&[u8]>,
) -> Vec<u8> {
    assert_eq!(palette.len(), 768, "A Quake palette requires 256 RGB entries");
    if let Some(table) = translation {
        assert_eq!(table.len(), 256, "Palette translation requires 256 entries");
    }
    let transparent_index = match transparency {
        PaletteTransparency::Opaque => None,
        PaletteTransparency::Index(index) => Some(*index),
        PaletteTransparency::Q1Fence => Some(255),
    };
    let mut rgba = Vec::with_capacity(pixels.len() * 4);
    for original in pixels {
        let mapped = translation.map_or(*original, |table| table[usize::from(*original)]);
        let base = usize::from(mapped) * 3;
        rgba.push(palette[base]);
        rgba.push(palette[base + 1]);
        rgba.push(palette[base + 2]);
        let visible = transparent_index != Some(*original);
        rgba.push(u8::from(visible) * 255);
    }
    assert_eq!(rgba.len(), width as usize * height as usize * 4);
    rgba
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Encoding {
    Indexed8,
    Rgba8,
    Depth32f,
}

/// Resident texture record.
#[derive(Debug, Clone)]
pub struct TextureRecord {
    /// GL texture name.
    pub name: u32,
    /// Whether the sampling filter uses mipmaps.
    pub mipmap: bool,
    /// Base-level width.
    pub width: u32,
    /// Base-level height.
    pub height: u32,
    encoding: Encoding,
    levels: Vec<(u32, u32)>,
    palette: Vec<u8>,
    transparency: PaletteTransparency,
    translation: Option<Vec<u8>>,
}

impl TextureRecord {
    #[must_use]
    pub fn is_depth(&self) -> bool {
        self.encoding == Encoding::Depth32f
    }
}

/// GL texture objects over a [`GlContext`].
#[derive(Debug)]
pub struct GlTextures {
    images: HashMap<u32, TextureRecord>,
    owner: ResourceOwner,
    maximum: u32,
}

impl GlTextures {
    #[must_use]
    pub fn new(owner: ResourceOwner, maximum: u32) -> Self {
        Self {
            images: HashMap::new(),
            owner,
            maximum,
        }
    }

    fn own(&self, image: &RendererImage) {
        if let Err(error) = self
            .owner
            .require(&image.owner, "OpenGL image belongs to another renderer owner")
        {
            panic!("{error}");
        }
    }

    #[must_use]
    pub fn registered(&self, image: &RendererImage) -> &TextureRecord {
        self.own(image);
        let texture = self.images.get(&image.ordinal).unwrap_or_else(|| {
            panic!("{}", RenderError::UnknownImage(image.ordinal));
        });
        if image.width != texture.width || image.height != texture.height {
            panic!(
                "{}",
                RenderError::BadDimensions {
                    width: image.width,
                    height: image.height,
                    detail: "OpenGL image handle dimensions differ from its allocation".to_string()
                }
            );
        }
        texture
    }

    pub fn bind<C: GlContext>(&self, gl: &mut C, binding: &TextureBinding) {
        match binding {
            TextureBinding::DynamicImage(_) => panic!(
                "{}",
                RenderError::Backend("Dynamic texture must resolve before binding".to_string())
            ),
            TextureBinding::BindImage(image) => {
                let name = self.registered(image).name;
                gl.bind_texture(TEXTURE_2D, name);
            }
            TextureBinding::RetainCurrentTexture => {}
        }
    }

    fn set_filter<C: GlContext>(gl: &mut C, filter: TextureFilter) {
        let (min, mag) = filters(filter);
        gl.tex_parameteri(TEXTURE_2D, TEXTURE_MIN_FILTER, min);
        gl.tex_parameteri(TEXTURE_2D, TEXTURE_MAG_FILTER, mag);
    }

    fn check_rgba_level(&self, level: &ImageLevel, channels: usize) {
        let maximum = self.maximum;
        if level.width < 1
            || level.height < 1
            || level.width > maximum
            || level.height > maximum
            || level.pixels.len() != level.width as usize * level.height as usize * channels
        {
            panic!(
                "{}",
                RenderError::BadDimensions {
                    width: level.width,
                    height: level.height,
                    detail: "OpenGL image dimensions do not match its pixel allocation".to_string()
                }
            );
        }
    }

    fn check_depth_level(&self, level: &DepthImageLevel) {
        let maximum = self.maximum;
        if level.width < 1
            || level.height < 1
            || level.width > maximum
            || level.height > maximum
            || level.pixels.len() != level.width as usize * level.height as usize
            || !level
                .pixels
                .iter()
                .all(|value| value.is_finite() && (0.0..=1.0).contains(value))
        {
            panic!(
                "{}",
                RenderError::BadDimensions {
                    width: level.width,
                    height: level.height,
                    detail: "OpenGL depth samples must be finite values in 0..1".to_string()
                }
            );
        }
    }

    pub fn apply<C: GlContext>(&mut self, gl: &mut C, operation: &ImageResourceOperation) {
        match operation {
            ImageResourceOperation::TextureMode { filter } => {
                for texture in self.images.values() {
                    if !texture.mipmap {
                        continue;
                    }
                    gl.bind_texture(TEXTURE_2D, texture.name);
                    Self::set_filter(gl, *filter);
                }
            }
            ImageResourceOperation::ReleaseImage { image } => {
                let name = self.registered(image).name;
                gl.delete_textures(&[name]);
                self.images.remove(&image.ordinal);
            }
            ImageResourceOperation::CreateImage {
                image,
                content,
                sampling,
            } => {
                self.own(image);
                if self.images.contains_key(&image.ordinal) {
                    panic!("{}", RenderError::ImageConflict(image.ordinal));
                }
                let (encoding, base_width, base_height, level_dims) = match content {
                    RenderImage::Indexed8 { levels, .. } => {
                        let first = levels.first().unwrap_or_else(|| {
                            panic!(
                                "{}",
                                RenderError::BadDimensions {
                                    width: 0,
                                    height: 0,
                                    detail: "OpenGL image needs a base mip level".to_string()
                                }
                            )
                        });
                        (
                            Encoding::Indexed8,
                            first.width,
                            first.height,
                            levels
                                .iter()
                                .map(|level| (level.width, level.height))
                                .collect::<Vec<_>>(),
                        )
                    }
                    RenderImage::Rgba8 { levels, .. } => {
                        let first = levels.first().unwrap_or_else(|| {
                            panic!(
                                "{}",
                                RenderError::BadDimensions {
                                    width: 0,
                                    height: 0,
                                    detail: "OpenGL image needs a base mip level".to_string()
                                }
                            )
                        });
                        (
                            Encoding::Rgba8,
                            first.width,
                            first.height,
                            levels
                                .iter()
                                .map(|level| (level.width, level.height))
                                .collect::<Vec<_>>(),
                        )
                    }
                    RenderImage::Depth32f { levels } => {
                        let first = levels.first().unwrap_or_else(|| {
                            panic!(
                                "{}",
                                RenderError::BadDimensions {
                                    width: 0,
                                    height: 0,
                                    detail: "OpenGL image needs a base mip level".to_string()
                                }
                            )
                        });
                        (
                            Encoding::Depth32f,
                            first.width,
                            first.height,
                            levels
                                .iter()
                                .map(|level| (level.width, level.height))
                                .collect::<Vec<_>>(),
                        )
                    }
                };
                if image.width != base_width || image.height != base_height {
                    panic!(
                        "{}",
                        RenderError::BadDimensions {
                            width: image.width,
                            height: image.height,
                            detail: "OpenGL image descriptor dimensions differ from its base mip level".to_string()
                        }
                    );
                }
                let max_level = base_width.max(base_height).ilog2() as usize;
                for (index, dims) in level_dims.iter().enumerate() {
                    let expect_width = (base_width >> index).max(1);
                    let expect_height = (base_height >> index).max(1);
                    if dims.0 != expect_width || dims.1 != expect_height || index > max_level {
                        panic!(
                            "{}",
                            RenderError::BadDimensions {
                                width: dims.0,
                                height: dims.1,
                                detail: "OpenGL mip levels must halve their preceding dimensions".to_string()
                            }
                        );
                    }
                }
                let expanded: Vec<(u32, u32, Vec<u8>)> = match content {
                    RenderImage::Indexed8 {
                        levels,
                        palette,
                        transparency,
                        translation,
                        ..
                    } => {
                        if palette.colors.len() != 768 {
                            panic!(
                                "{}",
                                RenderError::BadDimensions {
                                    width: base_width,
                                    height: base_height,
                                    detail: "A Quake palette requires 256 RGB entries".to_string()
                                }
                            );
                        }
                        levels
                            .iter()
                            .map(|level| {
                                self.check_rgba_level(level, 1);
                                let rgba = expand_indexed_level(
                                    &level.pixels,
                                    level.width,
                                    level.height,
                                    &palette.colors,
                                    transparency,
                                    translation.as_deref(),
                                );
                                (level.width, level.height, rgba)
                            })
                            .collect()
                    }
                    RenderImage::Rgba8 { levels, .. } => {
                        for level in levels {
                            self.check_rgba_level(level, 4);
                        }
                        Vec::new()
                    }
                    RenderImage::Depth32f { .. } => Vec::new(),
                };
                if let RenderImage::Depth32f { levels } = content {
                    for level in levels {
                        self.check_depth_level(level);
                    }
                }
                let names = gl.gen_textures(1);
                let name = names.first().copied().unwrap_or(0);
                if name == 0 {
                    panic!(
                        "{}",
                        RenderError::Backend("OpenGL could not allocate a texture".to_string())
                    );
                }
                gl.bind_texture(TEXTURE_2D, name);
                with_pixel_store(gl, PixelDirection::Unpack, |gl| match content {
                    RenderImage::Indexed8 { .. } => {
                        for (index, (width, height, pixels)) in expanded.iter().enumerate() {
                            gl.tex_image_2d_bytes(
                                TEXTURE_2D,
                                index as i32,
                                RGBA8,
                                *width as i32,
                                *height as i32,
                                0,
                                RGBA,
                                UNSIGNED_BYTE,
                                pixels,
                            );
                        }
                    }
                    RenderImage::Rgba8 { levels, .. } => {
                        for (index, level) in levels.iter().enumerate() {
                            gl.tex_image_2d_bytes(
                                TEXTURE_2D,
                                index as i32,
                                RGBA8,
                                level.width as i32,
                                level.height as i32,
                                0,
                                RGBA,
                                UNSIGNED_BYTE,
                                &level.pixels,
                            );
                        }
                    }
                    RenderImage::Depth32f { levels } => {
                        for (index, level) in levels.iter().enumerate() {
                            gl.tex_image_2d_floats(
                                TEXTURE_2D,
                                index as i32,
                                DEPTH_COMPONENT32F,
                                level.width as i32,
                                level.height as i32,
                                0,
                                DEPTH_COMPONENT,
                                FLOAT,
                                &level.pixels,
                            );
                        }
                    }
                });
                gl.tex_parameteri(TEXTURE_2D, TEXTURE_MAX_LEVEL, level_dims.len() as i32 - 1);
                Self::set_filter(gl, sampling.filter);
                let wrap = if sampling.repeat {
                    REPEAT
                } else if encoding == Encoding::Depth32f {
                    CLAMP_TO_EDGE
                } else {
                    CLAMP
                };
                gl.tex_parameteri(TEXTURE_2D, TEXTURE_WRAP_S, wrap);
                gl.tex_parameteri(TEXTURE_2D, TEXTURE_WRAP_T, wrap);
                if let RenderImage::Rgba8 { border_color, .. } = content {
                    gl.tex_parameterfv(
                        TEXTURE_2D,
                        TEXTURE_BORDER_COLOR,
                        &[border_color.x, border_color.y, border_color.z, border_color.w],
                    );
                }
                let error = gl.get_error();
                if error != 0 {
                    gl.delete_textures(&[name]);
                    panic!(
                        "{}",
                        RenderError::Backend(format!("OpenGL texture upload failed: 0x{error:x}"))
                    );
                }
                let (palette, transparency, translation) = match content {
                    RenderImage::Indexed8 {
                        palette,
                        transparency,
                        translation,
                        ..
                    } => (palette.colors.clone(), transparency.clone(), translation.clone()),
                    _ => (Vec::new(), PaletteTransparency::Opaque, None),
                };
                self.images.insert(
                    image.ordinal,
                    TextureRecord {
                        name,
                        mipmap: sampling.filter.uses_mipmap(),
                        width: base_width,
                        height: base_height,
                        encoding,
                        levels: level_dims,
                        palette,
                        transparency,
                        translation,
                    },
                );
            }
            ImageResourceOperation::UpdateImage { image, level, content } => {
                let mip = *level;
                let texture = self.registered(image).clone();
                let descriptor = texture.levels.get(mip as usize).copied();
                let (update_width, update_height) = match content {
                    LevelContent::Rgba(update) => (update.width, update.height),
                    LevelContent::Depth(update) => (update.width, update.height),
                };
                if descriptor != Some((update_width, update_height)) {
                    panic!(
                        "{}",
                        RenderError::BadLevel {
                            ordinal: image.ordinal,
                            level: mip
                        }
                    );
                }
                match (&texture.encoding, content) {
                    (Encoding::Depth32f, LevelContent::Depth(update)) => {
                        self.check_depth_level(update);
                        gl.bind_texture(TEXTURE_2D, texture.name);
                        with_pixel_store(gl, PixelDirection::Unpack, |gl| {
                            gl.tex_sub_image_2d_floats(
                                TEXTURE_2D,
                                mip as i32,
                                0,
                                0,
                                update.width as i32,
                                update.height as i32,
                                DEPTH_COMPONENT,
                                FLOAT,
                                &update.pixels,
                            );
                        });
                    }
                    (Encoding::Indexed8, LevelContent::Rgba(update)) => {
                        self.check_rgba_level(update, 1);
                        let rgba = expand_indexed_level(
                            &update.pixels,
                            update.width,
                            update.height,
                            &texture.palette,
                            &texture.transparency,
                            texture.translation.as_deref(),
                        );
                        gl.bind_texture(TEXTURE_2D, texture.name);
                        with_pixel_store(gl, PixelDirection::Unpack, |gl| {
                            gl.tex_sub_image_2d_bytes(
                                TEXTURE_2D,
                                mip as i32,
                                0,
                                0,
                                update.width as i32,
                                update.height as i32,
                                RGBA,
                                UNSIGNED_BYTE,
                                &rgba,
                            );
                        });
                    }
                    (Encoding::Rgba8, LevelContent::Rgba(update)) => {
                        self.check_rgba_level(update, 4);
                        gl.bind_texture(TEXTURE_2D, texture.name);
                        with_pixel_store(gl, PixelDirection::Unpack, |gl| {
                            gl.tex_sub_image_2d_bytes(
                                TEXTURE_2D,
                                mip as i32,
                                0,
                                0,
                                update.width as i32,
                                update.height as i32,
                                RGBA,
                                UNSIGNED_BYTE,
                                &update.pixels,
                            );
                        });
                    }
                    _ => panic!(
                        "{}",
                        RenderError::PixelMismatch(
                            image.ordinal,
                            "OpenGL update encoding differs from the registered image".to_string()
                        )
                    ),
                }
                let error = gl.get_error();
                if error != 0 {
                    panic!(
                        "{}",
                        RenderError::Backend(format!("OpenGL texture update failed: 0x{error:x}"))
                    );
                }
            }
        }
    }

    pub fn close<C: GlContext>(&mut self, gl: &mut C) {
        for texture in self.images.values() {
            gl.delete_textures(&[texture.name]);
        }
        self.images.clear();
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.images.len()
    }

    #[cfg(test)]
    pub fn is_empty(&self) -> bool {
        self.images.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::super::GlCall;
    use super::*;
    use crate::render::types::{ImageLevel, ImageSource, RenderImage, ResourceOwner, TextureSampling};
    use qa_core::identity::IdentityOwner;
    use qa_core::math::vec4;

    fn owner() -> ResourceOwner {
        let authority = IdentityOwner::create("gl-textures").unwrap();
        ResourceOwner::new(7, authority.session().clone(), 0)
    }

    fn image(owner: &ResourceOwner, ordinal: u32, width: u32, height: u32) -> RendererImage {
        RendererImage {
            owner: owner.clone(),
            ordinal,
            source: ImageSource::Generated {
                name: "test".to_string(),
            },
            width,
            height,
        }
    }

    #[test]
    fn creates_uploads_updates_and_releases_rgba() {
        let owner = owner();
        let mut gl = super::super::FakeGlContext::new();
        let mut textures = GlTextures::new(owner.clone(), 1024);
        let handle = image(&owner, 3, 2, 2);
        let content = RenderImage::Rgba8 {
            levels: vec![
                ImageLevel {
                    width: 2,
                    height: 2,
                    pixels: vec![1; 16],
                },
                ImageLevel {
                    width: 1,
                    height: 1,
                    pixels: vec![2; 4],
                },
            ],
            border_color: vec4(0.0, 0.0, 0.0, 1.0),
        };
        textures.apply(
            &mut gl,
            &ImageResourceOperation::CreateImage {
                image: handle.clone(),
                content,
                sampling: TextureSampling {
                    repeat: true,
                    filter: TextureFilter::LinearMipmapLinear,
                },
            },
        );
        assert_eq!(textures.len(), 1);
        assert!(textures.registered(&handle).mipmap);
        gl.assert_contains("upload", |call| {
            matches!(
                call,
                GlCall::TexImage2D {
                    level: 1,
                    width: 1,
                    height: 1,
                    ..
                }
            )
        });
        textures.apply(
            &mut gl,
            &ImageResourceOperation::TextureMode {
                filter: TextureFilter::Nearest,
            },
        );
        gl.assert_contains("filter", |call| {
            matches!(
                call,
                GlCall::TexParameteri {
                    pname: TEXTURE_MIN_FILTER,
                    value: NEAREST,
                    ..
                }
            )
        });
        textures.apply(
            &mut gl,
            &ImageResourceOperation::UpdateImage {
                image: handle.clone(),
                level: 1,
                content: LevelContent::Rgba(ImageLevel {
                    width: 1,
                    height: 1,
                    pixels: vec![9; 4],
                }),
            },
        );
        gl.assert_contains("sub upload", |call| {
            matches!(call, GlCall::TexSubImage2D { level: 1, .. })
        });
        textures.bind(&mut gl, &TextureBinding::BindImage(handle.clone()));
        textures.bind(&mut gl, &TextureBinding::RetainCurrentTexture);
        textures.apply(&mut gl, &ImageResourceOperation::ReleaseImage { image: handle });
        assert_eq!(textures.len(), 0);
        gl.assert_contains("delete", |call| matches!(call, GlCall::DeleteTextures { .. }));
    }

    #[test]
    fn expands_indexed_uploads_with_transparency() {
        let owner = owner();
        let mut gl = super::super::FakeGlContext::new();
        let mut textures = GlTextures::new(owner.clone(), 1024);
        let handle = image(&owner, 1, 2, 1);
        let mut palette = vec![0u8; 768];
        palette[0..3].copy_from_slice(&[10, 20, 30]);
        palette[3..6].copy_from_slice(&[40, 50, 60]);
        textures.apply(
            &mut gl,
            &ImageResourceOperation::CreateImage {
                image: handle.clone(),
                content: RenderImage::Indexed8 {
                    levels: vec![ImageLevel {
                        width: 2,
                        height: 1,
                        pixels: vec![0, 1],
                    }],
                    palette: crate::render::types::Palette {
                        colors: palette,
                        source: "test".to_string(),
                    },
                    transparency: PaletteTransparency::Index(1),
                    fullbright: None,
                    translation: None,
                },
                sampling: TextureSampling {
                    repeat: false,
                    filter: TextureFilter::Nearest,
                },
            },
        );
        gl.assert_contains("indexed upload", |call| {
            matches!(call, GlCall::TexImage2D { bytes: 8, .. })
        });
        assert_eq!(gl.count_matching(|call| matches!(call, GlCall::TexImage2D { .. })), 1);
        let rgba = expand_indexed_level(
            &[0, 1],
            2,
            1,
            &textures.registered(&handle).palette,
            &PaletteTransparency::Index(1),
            None,
        );
        assert_eq!(rgba, vec![10, 20, 30, 255, 40, 50, 60, 0]);
    }

    #[test]
    #[should_panic(expected = "another renderer owner")]
    fn rejects_foreign_owner() {
        let mut gl = super::super::FakeGlContext::new();
        let mut textures = GlTextures::new(owner(), 1024);
        let other = IdentityOwner::create("gl-foreign").unwrap();
        let foreign = image(&ResourceOwner::new(99, other.session().clone(), 0), 1, 1, 1);
        textures.apply(&mut gl, &ImageResourceOperation::ReleaseImage { image: foreign });
    }

    #[test]
    fn pixel_store_restores_settings() {
        let mut gl = super::super::FakeGlContext::new();
        gl.set_int(UNPACK_ALIGNMENT, vec![4]);
        with_pixel_store(&mut gl, PixelDirection::Unpack, |gl| {
            gl.assert_contains("tight unpack", |call| {
                matches!(
                    call,
                    GlCall::PixelStorei {
                        pname: UNPACK_ALIGNMENT,
                        value: 1
                    }
                )
            });
        });
        let restores = gl.calls_matching(|call| {
            matches!(
                call,
                GlCall::PixelStorei {
                    pname: UNPACK_ALIGNMENT,
                    value: 4
                }
            )
        });
        assert_eq!(restores.len(), 1);
    }
}
