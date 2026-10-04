//! CPU triangle rasterization kernel.
//!
//! Donor provenance: `src/render/cpu/triangle-kernel.ts` in full — the
//! fixed-function state of Quake III Arena `code/renderer/tr_backend.c`
//! (`GL_State`/`GL_Cull`) and `tr_image.c` (`R_CreateImage`), with new
//! clipping and rasterization algorithms. Original renderer copyright (C)
//! 1999-2005 Id Software, Inc. Debug polygons follow `tr_main.c`
//! `R_DebugPolygon`/`R_DebugGraphics`.
//!
//! Texture pixels are shared with [`CpuImages`](super::textures::CpuImages)
//! through reference counting so binding never copies texel storage.

use std::sync::Arc;

use qa_core::math::{Vec2, Vec4};

use super::super::types::{AlphaTest, BatchFog, BatchLighting, BlendFactor, FogEffect, PairEnvironment};
use super::lighting::{interpolate_world, shade_q2_fragment, CpuLighting, CpuTriangleLighting};
use super::lines::LineTextureDerivative;

/// One normalized texture sample.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Sample {
    /// Red channel.
    pub r: f32,
    /// Green channel.
    pub g: f32,
    /// Blue channel.
    pub b: f32,
    /// Alpha channel.
    pub a: f32,
}

#[derive(Debug, Clone, Copy)]
struct RowSpan {
    min: i32,
    max: i32,
}

/// Native byte order, matching the donor's runtime probe.
pub const LITTLE_ENDIAN: bool = cfg!(target_endian = "little");

/// Clamp a normalized channel to `0..=1`.
#[must_use]
pub fn clamp(value: f32) -> f32 {
    value.clamp(0.0, 1.0)
}

/// Convert a normalized channel to a byte.
#[must_use]
pub fn byte(value: f32) -> u8 {
    (clamp(value) * 255.0).round() as u8
}

/// Pack normalized channels into one framebuffer word.
#[must_use]
pub fn color_word(r: f32, g: f32, b: f32, a: f32) -> i32 {
    let (r, g, b, a) = (
        i32::from(byte(r)),
        i32::from(byte(g)),
        i32::from(byte(b)),
        i32::from(byte(a)),
    );
    if LITTLE_ENDIAN {
        r | (g << 8) | (b << 16) | (a << 24)
    } else {
        (r << 24) | (g << 16) | (b << 8) | a
    }
}

fn trim_span(span: &mut RowSpan, slope: f32, row: f32, constant: f32, inclusive: bool) {
    if span.min > span.max {
        return;
    }
    if slope == 0.0 {
        let value = row + constant;
        if value < 0.0 || (value == 0.0 && !inclusive) {
            span.max = span.min - 1;
        }
        return;
    }
    let crossing = -(row + constant) / slope - 0.5;
    // Algebra locates the crossing; the original edge expression corrects
    // rounding and lower-left equality at the boundary. The expression is
    // monotone across the row, so every pixel in the resulting span is covered.
    if slope > 0.0 {
        let mut min = span.min.max((span.max + 1).min(crossing.floor() as i32));
        while min > span.min {
            let value = slope * (min as f32 - 0.5) + row + constant;
            if !(value > 0.0 || (value == 0.0 && inclusive)) {
                break;
            }
            min -= 1;
        }
        while min <= span.max {
            let value = slope * (min as f32 + 0.5) + row + constant;
            if value > 0.0 || (value == 0.0 && inclusive) {
                break;
            }
            min += 1;
        }
        span.min = min;
    } else {
        let mut max = (span.min - 1).max(span.max.min(crossing.ceil() as i32));
        while max < span.max {
            let value = slope * (max as f32 + 1.5) + row + constant;
            if !(value > 0.0 || (value == 0.0 && inclusive)) {
                break;
            }
            max += 1;
        }
        while max >= span.min {
            let value = slope * (max as f32 + 0.5) + row + constant;
            if value > 0.0 || (value == 0.0 && inclusive) {
                break;
            }
            max -= 1;
        }
        span.max = max;
    }
}

fn factor(
    kind: BlendFactor,
    source: f32,
    destination: f32,
    source_alpha: f32,
    destination_alpha: f32,
    alpha_channel: bool,
) -> f32 {
    match kind {
        BlendFactor::Zero => 0.0,
        BlendFactor::One => 1.0,
        BlendFactor::SrcColor => source,
        BlendFactor::OneMinusSrcColor => 1.0 - source,
        BlendFactor::DstColor => destination,
        BlendFactor::OneMinusDstColor => 1.0 - destination,
        BlendFactor::SrcAlpha => source_alpha,
        BlendFactor::OneMinusSrcAlpha => 1.0 - source_alpha,
        BlendFactor::DstAlpha => destination_alpha,
        BlendFactor::OneMinusDstAlpha => 1.0 - destination_alpha,
        BlendFactor::SrcAlphaSaturate => {
            if alpha_channel {
                1.0
            } else {
                source_alpha.min(1.0 - destination_alpha)
            }
        }
    }
}

/// Blend one channel through the batch blend state.
#[must_use]
pub fn blend(
    source: f32,
    destination: f32,
    source_alpha: f32,
    destination_alpha: f32,
    blend: &(BlendFactor, BlendFactor),
    alpha_channel: bool,
) -> u8 {
    byte(
        source
            * factor(
                blend.0,
                source,
                destination,
                source_alpha,
                destination_alpha,
                alpha_channel,
            )
            + destination
                * factor(
                    blend.1,
                    source,
                    destination,
                    source_alpha,
                    destination_alpha,
                    alpha_channel,
                ),
    )
}

/// Evaluate the alpha test for one fragment.
#[must_use]
pub const fn passes_alpha(alpha: f32, test: AlphaTest) -> bool {
    match test {
        AlphaTest::None => true,
        AlphaTest::GreaterZero => alpha > 0.0,
        AlphaTest::Less128 => alpha < 0.5,
        AlphaTest::GreaterEqual128 => alpha >= 0.5,
    }
}

fn wrap_texel(index: i32, size: u32) -> u32 {
    // UVs were reduced to [0,1), so bilinear neighbors are at most one texel outside.
    if index < 0 {
        (index + size as i32) as u32
    } else if index >= size as i32 {
        (index - size as i32) as u32
    } else {
        index as u32
    }
}

fn sample_level(
    texture: &TextureStorage,
    border: &Vec4,
    u: f32,
    v: f32,
    repeat: bool,
    linear: bool,
    output: &mut Sample,
) {
    let (width, height) = (texture.width, texture.height);
    let data = &texture.pixels;
    let u = if repeat { u - u.floor() } else { clamp(u) };
    let v = if repeat { v - v.floor() } else { clamp(v) };
    if !linear {
        let x = ((u * width as f32).floor() as u32).min(width - 1);
        let y = ((v * height as f32).floor() as u32).min(height - 1);
        let offset = ((y * width + x) * 4) as usize;
        output.r = f32::from(data[offset]) / 255.0;
        output.g = f32::from(data[offset + 1]) / 255.0;
        output.b = f32::from(data[offset + 2]) / 255.0;
        output.a = f32::from(data[offset + 3]) / 255.0;
        return;
    }
    let x = u * width as f32 - 0.5;
    let y = v * height as f32 - 0.5;
    let mut x0 = x.floor() as i32;
    let mut y0 = y.floor() as i32;
    let fx = x - x0 as f32;
    let fy = y - y0 as f32;
    let mut x1 = x0 + 1;
    let mut y1 = y0 + 1;
    if repeat {
        x0 = wrap_texel(x0, width) as i32;
        x1 = wrap_texel(x1, width) as i32;
        y0 = wrap_texel(y0, height) as i32;
        y1 = wrap_texel(y1, height) as i32;
    }
    let w00 = (1.0 - fx) * (1.0 - fy) / 255.0;
    let w10 = fx * (1.0 - fy) / 255.0;
    let w01 = (1.0 - fx) * fy / 255.0;
    let w11 = fx * fy / 255.0;
    let has_border = !repeat && (x0 < 0 || y0 < 0 || x1 >= width as i32 || y1 >= height as i32);
    // Channel extraction consumes signed 32-bit words; avoid unsigned-number conversion.
    let word = |x: i32, y: i32| -> u32 {
        let offset = ((y as u32 * width + x as u32) * 4) as usize;
        (u32::from(data[offset]) << 24)
            | (u32::from(data[offset + 1]) << 16)
            | (u32::from(data[offset + 2]) << 8)
            | u32::from(data[offset + 3])
    };
    let (c00, c10, c01, c11, border_weight) = if has_border {
        // A missing tap denotes a GL_CLAMP border tap, not the nearest edge texel.
        let p00 = x0 < 0 || y0 < 0;
        let p10 = x1 >= width as i32 || y0 < 0;
        let p01 = x0 < 0 || y1 >= height as i32;
        let p11 = x1 >= width as i32 || y1 >= height as i32;
        let weight = (if p00 { w00 } else { 0.0 }
            + if p10 { w10 } else { 0.0 }
            + if p01 { w01 } else { 0.0 }
            + if p11 { w11 } else { 0.0 })
            * 255.0;
        (
            if p00 { 0 } else { word(x0, y0) },
            if p10 { 0 } else { word(x1, y0) },
            if p01 { 0 } else { word(x0, y1) },
            if p11 { 0 } else { word(x1, y1) },
            weight,
        )
    } else {
        (word(x0, y0), word(x1, y0), word(x0, y1), word(x1, y1), 0.0)
    };
    output.r =
        (c00 >> 24) as f32 * w00 + (c10 >> 24) as f32 * w10 + (c01 >> 24) as f32 * w01 + (c11 >> 24) as f32 * w11;
    output.g = ((c00 >> 16) & 255) as f32 * w00
        + ((c10 >> 16) & 255) as f32 * w10
        + ((c01 >> 16) & 255) as f32 * w01
        + ((c11 >> 16) & 255) as f32 * w11;
    output.b = ((c00 >> 8) & 255) as f32 * w00
        + ((c10 >> 8) & 255) as f32 * w10
        + ((c01 >> 8) & 255) as f32 * w01
        + ((c11 >> 8) & 255) as f32 * w11;
    // RGB storage contributes no alpha to any texture environment.
    output.a = if texture.has_alpha {
        (c00 & 255) as f32 * w00 + (c10 & 255) as f32 * w10 + (c01 & 255) as f32 * w01 + (c11 & 255) as f32 * w11
    } else {
        1.0
    };
    if has_border {
        output.r += border.x * border_weight;
        output.g += border.y * border_weight;
        output.b += border.z * border_weight;
        if texture.has_alpha {
            output.a += border.w * border_weight;
        }
    }
}

// RGBA texture environments run before alpha testing, blending and framebuffer
// conversion. GL_ADD saturates RGB but modulates alpha, unlike additive blending.
#[must_use]
pub fn texture_color(previous: f32, texel: f32, environment: PairEnvironment) -> f32 {
    match environment {
        PairEnvironment::Modulate => previous * texel,
        PairEnvironment::Add => clamp(previous + texel),
        PairEnvironment::Replace => texel,
    }
}

/// Texture internal format.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageInternalFormat {
    /// RGB without alpha.
    Rgb8,
    /// RGBA.
    Rgba8,
}

/// Mipmap selection between levels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MipMapping {
    /// Base level only.
    None,
    /// Nearest level.
    Nearest,
    /// Linear blend between levels.
    Linear,
}

/// One uploaded texture level. Pixels are shared with the image store.
#[derive(Debug, Clone)]
pub struct TextureStorage {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// Row-major RGBA bytes.
    pub pixels: Arc<Vec<u8>>,
    /// Internal format.
    pub internal_format: ImageInternalFormat,
    /// Whether alpha contributes to texture environments.
    pub has_alpha: bool,
    /// Uniform-level fast path sample.
    pub uniform: Option<Sample>,
}

/// A bound resizable texture.
#[derive(Debug, Clone)]
pub struct UploadedTexture {
    /// Internal format.
    pub internal_format: ImageInternalFormat,
    /// Repeat (`true`) or clamp sampling.
    pub repeat: bool,
    /// Linear minification within a level.
    pub minify_linear: bool,
    /// Linear magnification within a level.
    pub magnify_linear: bool,
    /// Mipmap selection.
    pub mipmapping: MipMapping,
    /// `rho` at or below this samples the base level magnified.
    pub magnification_limit: f32,
    /// Complete mip chain, base first.
    pub levels: Vec<TextureStorage>,
    /// Clamp border color.
    pub border_color: Vec4,
}

/// A texture proven to sample one constant color.
#[derive(Debug, Clone)]
pub struct ConstantTexture {
    /// Internal format.
    pub internal_format: ImageInternalFormat,
    /// The constant sample.
    pub sample: Sample,
}

/// A texture bound to one unit.
#[derive(Debug, Clone)]
pub enum BoundTexture {
    /// Resizable image texture.
    Image(UploadedTexture),
    /// Constant-color texture.
    Constant(ConstantTexture),
    /// Unbound or mip-incomplete texture; sampling skips it.
    Incomplete,
}

/// Whether the internal format contributes alpha.
#[must_use]
pub const fn texture_has_alpha(format: ImageInternalFormat) -> bool {
    matches!(format, ImageInternalFormat::Rgba8)
}

/// Sample a bound texture at explicit LOD `rho`.
pub fn sample_bound(texture: &BoundTexture, u: f32, v: f32, rho: f32, output: &mut Sample) {
    sample_bound_components(texture, u, v, Some(rho), output, 0.0, 0.0, 0.0, 0.0);
}

#[allow(clippy::too_many_arguments)]
fn sample_bound_components(
    texture: &BoundTexture,
    u: f32,
    v: f32,
    rho: Option<f32>,
    output: &mut Sample,
    x_u: f32,
    x_v: f32,
    y_u: f32,
    y_v: f32,
) {
    let BoundTexture::Image(image) = texture else {
        if let BoundTexture::Constant(constant) = texture {
            *output = constant.sample;
            return;
        }
        panic!("sampling an incomplete texture");
    };
    let mut estimated = false;
    let mut rho = rho;
    if rho.is_none() {
        let estimate = if image.mipmapping == MipMapping::Nearest {
            nearest_rho_estimate(x_u, x_v, y_u, y_v)
        } else {
            0.0
        };
        if estimate > 0.0
            && (estimate * (1.0 + NEAREST_MIP_GUARD) <= image.magnification_limit
                || estimate * (1.0 - NEAREST_MIP_GUARD) > image.magnification_limit)
        {
            rho = Some(estimate);
            estimated = true;
        } else {
            rho = Some(derivative_length(x_u, x_v).max(derivative_length(y_u, y_v)));
        }
    }
    let rho = rho.unwrap_or(0.0);
    // OpenGL 2.1 equations 3.18, 3.27-3.29. This CPU profile uses ideal rho;
    // drivers may approximate it, so LOD is deterministic rather than driver exact.
    if rho <= image.magnification_limit {
        sample_level(
            &image.levels[0],
            &image.border_color,
            u,
            v,
            image.repeat,
            image.magnify_linear,
            output,
        );
        return;
    }
    if image.mipmapping == MipMapping::None {
        sample_level(
            &image.levels[0],
            &image.border_color,
            u,
            v,
            image.repeat,
            image.minify_linear,
            output,
        );
        return;
    }
    let last_level = image.levels.len() - 1;
    let mut lambda = (last_level as f32).min(rho.log2());
    if estimated {
        let shifted = lambda + 0.5;
        let fraction = shifted - shifted.floor();
        if !(fraction > NEAREST_MIP_GUARD && 1.0 - fraction > NEAREST_MIP_GUARD) {
            let exact = derivative_length(x_u, x_v).max(derivative_length(y_u, y_v));
            lambda = (last_level as f32).min(exact.log2());
        }
    }
    let between_levels = image.mipmapping == MipMapping::Linear;
    let selected = if between_levels {
        lambda.floor() as usize
    } else {
        ((lambda + 0.5).ceil() as usize).saturating_sub(1)
    };
    let first = image
        .levels
        .get(selected)
        .unwrap_or_else(|| panic!("texture LOD is outside its complete mip chain"));
    sample_level(
        first,
        &image.border_color,
        u,
        v,
        image.repeat,
        image.minify_linear,
        output,
    );
    let fraction = lambda - selected as f32;
    if !between_levels || fraction == 0.0 {
        return;
    }
    let (r, g, b, a) = (output.r, output.g, output.b, output.a);
    let second = image
        .levels
        .get(selected + 1)
        .unwrap_or_else(|| panic!("texture LOD blend is outside its complete mip chain"));
    sample_level(
        second,
        &image.border_color,
        u,
        v,
        image.repeat,
        image.minify_linear,
        output,
    );
    output.r = r * (1.0 - fraction) + output.r * fraction;
    output.g = g * (1.0 - fraction) + output.g * fraction;
    output.b = b * (1.0 - fraction) + output.b * fraction;
    output.a = a * (1.0 - fraction) + output.a * fraction;
}

/// Perspective texture-coordinate plane with a per-unit anchor.
#[derive(Debug, Clone, Copy)]
pub struct TexturePlaneDerivative {
    /// Common U offset, excluded from the derivative sum.
    pub u_anchor: f32,
    /// Common V offset, excluded from the derivative sum.
    pub v_anchor: f32,
    /// `d(U-Q)/dx` quotient terms.
    pub u_dx: f32,
    /// `d(V-Q)/dx` quotient terms.
    pub v_dx: f32,
    /// `dQ/dx` quotient term.
    pub q_dx: f32,
    /// `d(U-Q)/dy` quotient terms.
    pub u_dy: f32,
    /// `d(V-Q)/dy` quotient terms.
    pub v_dy: f32,
    /// `dQ/dy` quotient term.
    pub q_dy: f32,
}

fn derivative_length(x: f32, y: f32) -> f32 {
    if x == 0.0 {
        return y.abs();
    }
    if y == 0.0 {
        return x.abs();
    }
    x.hypot(y)
}

fn sample_perspective_bound(
    texture: &BoundTexture,
    u: f32,
    v: f32,
    reciprocal: f32,
    derivative: &TexturePlaneDerivative,
    output: &mut Sample,
) {
    let mut rho: Option<f32> = Some(0.0);
    let (mut x_u, mut x_v, mut y_u, mut y_v) = (0.0, 0.0, 0.0, 0.0);
    let (anchor_u, anchor_v) = if let BoundTexture::Image(image) = texture {
        if image.mipmapping == MipMapping::None {
            sample_level(
                &image.levels[0],
                &image.border_color,
                derivative.u_anchor + u,
                derivative.v_anchor + v,
                image.repeat,
                image.magnify_linear,
                output,
            );
            return;
        }
        let (width, height) = (image.levels[0].width as f32, image.levels[0].height as f32);
        // u/v are the quotient relative to a per-unit anchor. The common offset
        // contributes no derivative and never enters this cancellation-prone sum.
        x_u = (derivative.u_dx - u * derivative.q_dx) * reciprocal * width;
        x_v = (derivative.v_dx - v * derivative.q_dx) * reciprocal * height;
        y_u = (derivative.u_dy - u * derivative.q_dy) * reciprocal * width;
        y_v = (derivative.v_dy - v * derivative.q_dy) * reciprocal * height;
        rho = None;
        (derivative.u_anchor, derivative.v_anchor)
    } else {
        (derivative.u_anchor, derivative.v_anchor)
    };
    sample_bound_components(texture, anchor_u + u, anchor_v + v, rho, output, x_u, x_v, y_u, y_v);
}

const NEAREST_MIP_GUARD: f32 = 2.328_306_4e-10; // 2^-32
const MIN_NEAREST_COMPONENT: f64 = 6.223_015_277_861_142e-61; // 2^-200
const MAX_NEAREST_COMPONENT: f64 = 1.606_938_044_258_990_3e60; // 2^200

fn nearest_rho_estimate(x_u: f32, x_v: f32, y_u: f32, y_v: f32) -> f32 {
    let in_range = |value: f32| {
        let magnitude = f64::from(value.abs());
        magnitude == 0.0 || (MIN_NEAREST_COMPONENT..=MAX_NEAREST_COMPONENT).contains(&magnitude)
    };
    if in_range(x_u) && in_range(x_v) && in_range(y_u) && in_range(y_v) {
        (f64::from(x_u) * f64::from(x_u) + f64::from(x_v) * f64::from(x_v))
            .max(f64::from(y_u) * f64::from(y_u) + f64::from(y_v) * f64::from(y_v))
            .sqrt() as f32
    } else {
        0.0
    }
}

/// Sample a bound texture along a line with per-pixel derivatives.
pub fn sample_line_bound(
    texture: &BoundTexture,
    coordinate: &Vec2,
    derivative: &LineTextureDerivative,
    output: &mut Sample,
) {
    let rho = match texture {
        BoundTexture::Image(image) if image.mipmapping != MipMapping::None => (derivative.ds_per_pixel
            * image.levels[0].width as f32)
            .hypot(derivative.dt_per_pixel * image.levels[0].height as f32),
        _ => 0.0,
    };
    sample_bound(texture, coordinate.x, coordinate.y, rho, output);
}

/// CPU color, depth, and stencil storage. Rows run top to bottom.
#[derive(Debug, Clone)]
pub struct Framebuffer {
    /// Original framebuffer dimensions, independent of packed region storage.
    pub width: u32,
    /// Original framebuffer dimensions, independent of packed region storage.
    pub height: u32,
    /// Row-major RGBA bytes.
    pub pixels: Vec<u8>,
    /// Row-major window depth.
    pub depth: Vec<f32>,
    /// Row-major stencil, absent without stencil bits.
    pub stencil: Option<Vec<u32>>,
    /// Absolute origin of storage; stride is measured in pixels.
    pub origin_x: i32,
    /// Absolute origin of storage; stride is measured in pixels.
    pub origin_y: i32,
    /// Row stride in pixels.
    pub stride: u32,
}

impl Framebuffer {
    /// Allocate zeroed color with cleared depth (`1.0`).
    #[must_use]
    pub fn new(width: u32, height: u32, stencil: bool) -> Self {
        assert!(width >= 1 && height >= 1, "framebuffer dimensions must be positive");
        let count = (width as usize) * (height as usize);
        Self {
            width,
            height,
            pixels: vec![0u8; count * 4],
            depth: vec![1.0f32; count],
            stencil: stencil.then(|| vec![0u32; count]),
            origin_x: 0,
            origin_y: 0,
            stride: width,
        }
    }

    /// Pixel ordinal for absolute coordinates.
    #[must_use]
    pub fn pixel_index(&self, x: i32, y: i32) -> usize {
        let local_x = x - self.origin_x;
        let local_y = y - self.origin_y;
        assert!(
            local_x >= 0 && local_y >= 0,
            "fragment is outside its framebuffer storage"
        );
        let index = local_y as usize * self.stride as usize + local_x as usize;
        assert!(index < self.depth.len(), "fragment is outside its framebuffer storage");
        index
    }

    /// Load normalized color channels.
    #[must_use]
    pub fn load_normalized(&self, pixel: usize) -> [f32; 4] {
        let offset = pixel * 4;
        [
            f32::from(self.pixels[offset]) / 255.0,
            f32::from(self.pixels[offset + 1]) / 255.0,
            f32::from(self.pixels[offset + 2]) / 255.0,
            f32::from(self.pixels[offset + 3]) / 255.0,
        ]
    }

    /// Store color bytes.
    pub fn store_bytes(&mut self, pixel: usize, bytes: [u8; 4]) {
        self.whole().store_bytes(pixel, bytes);
    }

    /// Whole-buffer strip view.
    pub fn whole(&mut self) -> FramebufferStrip<'_> {
        let stencil = self.stencil.as_mut().map(|stencil| &mut stencil[..]);
        FramebufferStrip {
            pixels: &mut self.pixels,
            depth: &mut self.depth,
            stencil,
            stride: self.stride,
            origin_x: self.origin_x,
            origin_y: self.origin_y,
        }
    }

    /// Split storage into `count` contiguous row strips for strip-parallel
    /// shading. Strips are as even as possible; fewer come back when the
    /// buffer holds fewer rows. Strips borrow disjoint rows, so threads
    /// shading different strips never share a pixel.
    pub fn split_strips(&mut self, count: usize) -> Vec<FramebufferStrip<'_>> {
        let stride = self.stride as usize;
        assert!(stride > 0, "framebuffer stride must be positive");
        let rows = self.pixels.len() / (stride * 4);
        assert_eq!(
            self.depth.len(),
            rows * stride,
            "depth storage must match color storage"
        );
        if let Some(stencil) = &self.stencil {
            assert_eq!(stencil.len(), rows * stride, "stencil storage must match color storage");
        }
        let count = count.max(1).min(rows.max(1));
        let base = rows / count;
        let extra = rows % count;
        let mut pixels = &mut self.pixels[..];
        let mut depth = &mut self.depth[..];
        let mut stencil = self.stencil.as_mut().map(|stencil| &mut stencil[..]);
        let mut strips = Vec::with_capacity(count);
        let mut row = self.origin_y;
        for index in 0..count {
            let strip_rows = base + usize::from(index < extra);
            let cells = strip_rows * stride;
            let (head_pixels, rest_pixels) = pixels.split_at_mut(cells * 4);
            pixels = rest_pixels;
            let (head_depth, rest_depth) = depth.split_at_mut(cells);
            depth = rest_depth;
            let head_stencil = stencil.take().map(|strip| {
                let (head, rest) = strip.split_at_mut(cells);
                stencil = Some(rest);
                head
            });
            strips.push(FramebufferStrip {
                pixels: head_pixels,
                depth: head_depth,
                stencil: head_stencil,
                stride: self.stride,
                origin_x: self.origin_x,
                origin_y: row,
            });
            row += strip_rows as i32;
        }
        strips
    }
}

/// Mutable row-range view over a [`Framebuffer`] for strip-parallel shading.
///
/// Slices start at `origin_y`; pixel indices use the same
/// `(y - origin_y) * stride - origin_x + x` formula as the full buffer, so
/// shading code is identical for whole-buffer and strip views.
pub struct FramebufferStrip<'a> {
    /// Strip RGBA bytes.
    pub pixels: &'a mut [u8],
    /// Strip depth values.
    pub depth: &'a mut [f32],
    /// Strip stencil values, absent without stencil bits.
    pub stencil: Option<&'a mut [u32]>,
    /// Row stride in pixels (same as the parent).
    pub stride: u32,
    /// Absolute X origin (same as the parent).
    pub origin_x: i32,
    /// Absolute Y origin: the strip's first row.
    pub origin_y: i32,
}

impl FramebufferStrip<'_> {
    /// Load normalized color channels.
    #[must_use]
    pub fn load_normalized(&self, pixel: usize) -> [f32; 4] {
        let offset = pixel * 4;
        [
            f32::from(self.pixels[offset]) / 255.0,
            f32::from(self.pixels[offset + 1]) / 255.0,
            f32::from(self.pixels[offset + 2]) / 255.0,
            f32::from(self.pixels[offset + 3]) / 255.0,
        ]
    }

    /// Store color bytes.
    pub fn store_bytes(&mut self, pixel: usize, bytes: [u8; 4]) {
        let offset = pixel * 4;
        self.pixels[offset..offset + 4].copy_from_slice(&bytes);
    }
}

/// Stencil test function.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StencilFunction {
    /// Always pass.
    Always,
    /// Pass on nonzero masked value.
    NonZero,
}

/// Stencil update operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StencilOp {
    /// Keep the stored value.
    Keep,
    /// Increment saturating at the maximum.
    Increment,
    /// Decrement saturating at zero.
    Decrement,
}

/// Fast blending path selected from the blend state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlendMode {
    /// Source overwrites destination.
    Opaque,
    /// Source-alpha blend.
    Alpha,
    /// Additive blend.
    Add,
    /// Multiplicative blend.
    Multiply,
    /// Destination-color with inverse destination alpha.
    DstColorInverseDstAlpha,
    /// General blend through [`blend`].
    General,
}

/// Interpolated triangle state for [`run_triangle_rows`].
#[derive(Debug, Clone)]
pub struct TriangleSetup<'a> {
    /// Fog depth scale (`1.0` without Q2 lighting rescale).
    pub fog_depth_scale: f32,
    /// Fragment fog.
    pub fog: Option<BatchFog>,
    /// Luminance-alpha texture effect.
    pub luminance_alpha: bool,
    /// Fragment lighting.
    pub lighting: CpuTriangleLighting<'a>,
    /// Iteration bounds; coverage keeps absolute coordinates.
    pub min_x: i32,
    /// Iteration bounds; coverage keeps absolute coordinates.
    pub max_x: i32,
    /// Iteration bounds; coverage keeps absolute coordinates.
    pub min_y: i32,
    /// Iteration bounds; coverage keeps absolute coordinates.
    pub max_y: i32,
    /// Reciprocal attribute area.
    pub inverse_area: f32,
    /// Depth range near.
    pub depth_near: f32,
    /// Depth range far.
    pub depth_far: f32,
    /// Coverage edge coefficients.
    pub edge_ax: f32,
    /// Coverage edge coefficients.
    pub edge_ay: f32,
    /// Coverage edge coefficients.
    pub edge_ac: f32,
    /// Coverage edge coefficients.
    pub edge_bx: f32,
    /// Coverage edge coefficients.
    pub edge_by: f32,
    /// Coverage edge coefficients.
    pub edge_bc: f32,
    /// Coverage edge coefficients.
    pub edge_cx: f32,
    /// Coverage edge coefficients.
    pub edge_cy: f32,
    /// Coverage edge coefficients.
    pub edge_cc: f32,
    /// Attribute edge coefficients.
    pub attribute_ax: f32,
    /// Attribute edge coefficients.
    pub attribute_ay: f32,
    /// Attribute edge coefficients.
    pub attribute_ac: f32,
    /// Attribute edge coefficients.
    pub attribute_bx: f32,
    /// Attribute edge coefficients.
    pub attribute_by: f32,
    /// Attribute edge coefficients.
    pub attribute_bc: f32,
    /// Attribute edge coefficients.
    pub attribute_cx: f32,
    /// Attribute edge coefficients.
    pub attribute_cy: f32,
    /// Attribute edge coefficients.
    pub attribute_cc: f32,
    /// Clip Z over W per vertex.
    pub az: f32,
    /// Clip Z over W per vertex.
    pub bz: f32,
    /// Clip Z over W per vertex.
    pub cz: f32,
    /// Polygon-offset depth bias.
    pub polygon_depth_offset: f32,
    /// Constant-depth fast path value.
    pub plane_depth: f32,
    /// Common W scale over clip W per vertex.
    pub aiw: f32,
    /// Common W scale over clip W per vertex.
    pub biw: f32,
    /// Common W scale over clip W per vertex.
    pub ciw: f32,
    /// Anchored U over W per vertex.
    pub au: f32,
    /// Anchored U over W per vertex.
    pub bu: f32,
    /// Anchored U over W per vertex.
    pub cu: f32,
    /// Anchored V over W per vertex.
    pub av: f32,
    /// Anchored V over W per vertex.
    pub bv: f32,
    /// Anchored V over W per vertex.
    pub cv: f32,
    /// Anchored secondary U over W per vertex.
    pub au2: f32,
    /// Anchored secondary U over W per vertex.
    pub bu2: f32,
    /// Anchored secondary U over W per vertex.
    pub cu2: f32,
    /// Anchored secondary V over W per vertex.
    pub av2: f32,
    /// Anchored secondary V over W per vertex.
    pub bv2: f32,
    /// Anchored secondary V over W per vertex.
    pub cv2: f32,
    /// Red over W per vertex.
    pub ar: f32,
    /// Red over W per vertex.
    pub br: f32,
    /// Red over W per vertex.
    pub cr: f32,
    /// Green over W per vertex.
    pub ag: f32,
    /// Green over W per vertex.
    pub bg: f32,
    /// Green over W per vertex.
    pub cg: f32,
    /// Blue over W per vertex.
    pub ab: f32,
    /// Blue over W per vertex.
    pub bb: f32,
    /// Blue over W per vertex.
    pub cb: f32,
    /// Alpha over W per vertex.
    pub aa: f32,
    /// Alpha over W per vertex.
    pub ba: f32,
    /// Alpha over W per vertex.
    pub ca: f32,
    /// Lower-left fill convention per edge.
    pub edge_a_inclusive: bool,
    /// Lower-left fill convention per edge.
    pub edge_b_inclusive: bool,
    /// Lower-left fill convention per edge.
    pub edge_c_inclusive: bool,
    /// White-vertex fast path.
    pub white: bool,
    /// Write depth.
    pub depth_write: bool,
    /// Evaluate the stencil test.
    pub stencil_enabled: bool,
    /// Write color.
    pub color_write: bool,
    /// Sample textures (color write or alpha test active).
    pub texture_consumed: bool,
    /// Primary texture contributes alpha.
    pub primary_alpha: bool,
    /// Secondary texture contributes alpha.
    pub secondary_alpha: bool,
    /// Constant-depth fast path.
    pub constant_depth: bool,
    /// Framebuffer width.
    pub width: u32,
    /// Framebuffer height.
    pub height: u32,
    /// Blend factors.
    pub blend: (BlendFactor, BlendFactor),
    /// Destination alpha bits (`0` or `8`).
    pub alpha_bits: u32,
    /// Selected fast blending path.
    pub blending: BlendMode,
    /// Depth test.
    pub depth_test: super::super::types::DepthTest,
    /// Alpha test.
    pub alpha_test: AlphaTest,
    /// Secondary combine environment.
    pub secondary_environment: Option<PairEnvironment>,
    /// Primary texture.
    pub texture: BoundTexture,
    /// Secondary texture.
    pub secondary_texture: BoundTexture,
    /// Primary perspective plane.
    pub derivative: TexturePlaneDerivative,
    /// Secondary perspective plane.
    pub secondary_derivative: TexturePlaneDerivative,
    /// Stencil function.
    pub stencil_function: StencilFunction,
    /// Stencil compare mask.
    pub stencil_compare_mask: u32,
    /// Stencil write mask.
    pub stencil_write_mask: u32,
    /// Stencil saturating maximum.
    pub stencil_maximum: u32,
    /// Stencil update on depth failure.
    pub stencil_depth_fail: StencilOp,
    /// Stencil update on depth pass.
    pub stencil_depth_pass: StencilOp,
}

/// Rasterize setup rows `first_y..=last_y` into the framebuffer.
///
/// Returns whether any texture sampling ran. Limits restrict iteration only;
/// coverage and interpolation retain absolute coordinates.
pub fn run_triangle_rows(
    setup: &TriangleSetup,
    target: &mut FramebufferStrip,
    sampled: &mut Sample,
    first_y: i32,
    last_y: i32,
) -> bool {
    let mut span = RowSpan {
        min: setup.min_x,
        max: setup.max_x,
    };
    let mut did_sample = false;
    for y in setup.min_y.max(first_y)..=setup.max_y.min(last_y) {
        let sample_y = y as f32 + 0.5;
        let row_a = setup.edge_ay * sample_y;
        let row_b = setup.edge_by * sample_y;
        let row_c = setup.edge_cy * sample_y;
        let attribute_row_a = setup.attribute_ay * sample_y;
        let attribute_row_b = setup.attribute_by * sample_y;
        let attribute_row_c = setup.attribute_cy * sample_y;
        let row_offset = (y - target.origin_y) * target.stride as i32 - target.origin_x;
        span.min = setup.min_x;
        span.max = setup.max_x;
        trim_span(&mut span, setup.edge_ax, row_a, setup.edge_ac, setup.edge_a_inclusive);
        trim_span(&mut span, setup.edge_bx, row_b, setup.edge_bc, setup.edge_b_inclusive);
        trim_span(&mut span, setup.edge_cx, row_c, setup.edge_cc, setup.edge_c_inclusive);
        for x in span.min..=span.max {
            let sample_x = x as f32 + 0.5;
            let wa = (setup.attribute_ax * sample_x + attribute_row_a + setup.attribute_ac) * setup.inverse_area;
            let wb = (setup.attribute_bx * sample_x + attribute_row_b + setup.attribute_bc) * setup.inverse_area;
            let wc = (setup.attribute_cx * sample_x + attribute_row_c + setup.attribute_cc) * setup.inverse_area;
            let depth = if setup.constant_depth {
                setup.plane_depth
            } else {
                clamp(
                    clamp((setup.az * wa + setup.bz * wb + setup.cz * wc) * 0.5 + 0.5)
                        * (setup.depth_far - setup.depth_near)
                        + setup.depth_near
                        + setup.polygon_depth_offset,
                )
            };
            let pixel = (row_offset + x) as usize;
            let old_depth = *target.depth.get(pixel).expect("fragment is outside the depth buffer");
            let depth_passed = !((setup.depth_test == super::super::types::DepthTest::LessEqual && depth > old_depth)
                || (setup.depth_test == super::super::types::DepthTest::Equal && depth != old_depth));
            if !depth_passed && !setup.stencil_enabled {
                continue;
            }
            let inverse_w = setup.aiw * wa + setup.biw * wb + setup.ciw * wc;
            let reciprocal = 1.0 / inverse_w;
            let (mut r, mut g, mut blue, mut alpha) = if setup.white {
                let one = clamp(inverse_w * reciprocal);
                (one, one, one, one)
            } else {
                (
                    clamp((setup.ar * wa + setup.br * wb + setup.cr * wc) * reciprocal),
                    clamp((setup.ag * wa + setup.bg * wb + setup.cg * wc) * reciprocal),
                    clamp((setup.ab * wa + setup.bb * wb + setup.cb * wc) * reciprocal),
                    clamp((setup.aa * wa + setup.ba * wb + setup.ca * wc) * reciprocal),
                )
            };
            let vertex_color = qa_core::math::vec4(r, g, blue, alpha);
            let (mut texel_r, mut texel_g, mut texel_b, mut texel_a) = (1.0, 1.0, 1.0, 1.0);
            if setup.texture_consumed && !matches!(setup.texture, BoundTexture::Incomplete) {
                let u = (setup.au * wa + setup.bu * wb + setup.cu * wc) * reciprocal;
                let v = (setup.av * wa + setup.bv * wb + setup.cv * wc) * reciprocal;
                sample_perspective_bound(&setup.texture, u, v, reciprocal, &setup.derivative, sampled);
                did_sample = true;
                texel_r = sampled.r;
                texel_g = sampled.g;
                texel_b = sampled.b;
                texel_a = if setup.primary_alpha { sampled.a } else { 1.0 };
                if setup.luminance_alpha {
                    let modulation = (sampled.r + sampled.g + sampled.b) / 3.0 * alpha;
                    sampled.r *= modulation;
                    sampled.g *= modulation;
                    sampled.b *= modulation;
                    texel_r = sampled.r;
                    texel_g = sampled.g;
                    texel_b = sampled.b;
                }
                r *= sampled.r;
                g *= sampled.g;
                blue *= sampled.b;
                if setup.primary_alpha {
                    alpha *= sampled.a;
                }
            }
            if setup.texture_consumed && !matches!(setup.lighting.parameters, BatchLighting::Vertex) {
                let weights_a = setup.aiw * wa * reciprocal;
                let weights_b = setup.biw * wb * reciprocal;
                let weights_c = setup.ciw * wc * reciprocal;
                let position = interpolate_world(
                    setup.lighting.positions[0],
                    setup.lighting.positions[1],
                    setup.lighting.positions[2],
                    weights_a,
                    weights_b,
                    weights_c,
                );
                let normal = interpolate_world(
                    setup.lighting.normals[0],
                    setup.lighting.normals[1],
                    setup.lighting.normals[2],
                    weights_a,
                    weights_b,
                    weights_c,
                );
                let lighting = CpuLighting {
                    parameters: setup.lighting.parameters,
                    depth: setup.lighting.depth.clone(),
                };
                let result = shade_q2_fragment(
                    &lighting,
                    position,
                    normal,
                    vertex_color,
                    &Sample {
                        r: texel_r,
                        g: texel_g,
                        b: texel_b,
                        a: texel_a,
                    },
                );
                r = result.r;
                g = result.g;
                blue = result.b;
                alpha = result.a;
            }
            if setup.texture_consumed
                && setup.secondary_environment.is_some()
                && !matches!(setup.secondary_texture, BoundTexture::Incomplete)
            {
                let u = (setup.au2 * wa + setup.bu2 * wb + setup.cu2 * wc) * reciprocal;
                let v = (setup.av2 * wa + setup.bv2 * wb + setup.cv2 * wc) * reciprocal;
                sample_perspective_bound(
                    &setup.secondary_texture,
                    u,
                    v,
                    reciprocal,
                    &setup.secondary_derivative,
                    sampled,
                );
                did_sample = true;
                match setup.secondary_environment {
                    Some(PairEnvironment::Modulate) => {
                        r *= sampled.r;
                        g *= sampled.g;
                        blue *= sampled.b;
                        if setup.secondary_alpha {
                            alpha *= sampled.a;
                        }
                    }
                    Some(PairEnvironment::Add) => {
                        r = clamp(r + sampled.r);
                        g = clamp(g + sampled.g);
                        blue = clamp(blue + sampled.b);
                        if setup.secondary_alpha {
                            alpha *= sampled.a;
                        }
                    }
                    Some(PairEnvironment::Replace) => {
                        r = sampled.r;
                        g = sampled.g;
                        blue = sampled.b;
                        if setup.secondary_alpha {
                            alpha = sampled.a;
                        }
                    }
                    None => {}
                }
            }
            if let Some(fog) = &setup.fog {
                let (d, amount, effect, color) = match fog {
                    BatchFog::Exp2 { color, density, effect } => {
                        let d = density * setup.fog_depth_scale * reciprocal / 64.0;
                        (d, 1.0 - (-d * d).exp(), *effect, color)
                    }
                    BatchFog::Constant { color, amount } => (0.0, *amount, FogEffect::Color, color),
                };
                let _ = d;
                if effect != FogEffect::None {
                    r = clamp(r);
                    g = clamp(g);
                    blue = clamp(blue);
                }
                if effect == FogEffect::Color {
                    r += (color.x - r) * amount;
                    g += (color.y - g) * amount;
                    blue += (color.z - blue) * amount;
                }
                if effect == FogEffect::Rgb || effect == FogEffect::Rgba {
                    r *= 1.0 - amount;
                    g *= 1.0 - amount;
                    blue *= 1.0 - amount;
                }
                if effect == FogEffect::Alpha || effect == FogEffect::Rgba {
                    alpha *= 1.0 - amount;
                }
                if effect == FogEffect::Overlay {
                    r = color.x;
                    g = color.y;
                    blue = color.z;
                    alpha *= amount;
                }
            }
            if setup.alpha_test != AlphaTest::None && !passes_alpha(alpha, setup.alpha_test) {
                continue;
            }
            if setup.stencil_enabled
                && !stencil_fragment(
                    target.stencil.as_deref_mut(),
                    pixel,
                    depth_passed,
                    StencilTest {
                        function: setup.stencil_function,
                        compare_mask: setup.stencil_compare_mask,
                        write_mask: setup.stencil_write_mask,
                        maximum: setup.stencil_maximum,
                        depth_fail: setup.stencil_depth_fail,
                        depth_pass: setup.stencil_depth_pass,
                    },
                )
            {
                continue;
            }
            if !setup.color_write {
                continue;
            }
            if setup.blending == BlendMode::Opaque {
                target.store_bytes(
                    pixel,
                    [
                        byte(r),
                        byte(g),
                        byte(blue),
                        if setup.alpha_bits == 0 { 255 } else { byte(alpha) },
                    ],
                );
            } else {
                write_fragment(
                    target,
                    setup.alpha_bits,
                    pixel,
                    [r, g, blue, alpha],
                    &setup.blend,
                    setup.blending,
                );
            }
            if setup.depth_write {
                target.depth[pixel] = depth;
            }
        }
    }
    did_sample
}

fn write_fragment(
    target: &mut FramebufferStrip,
    alpha_bits: u32,
    pixel: usize,
    color: [f32; 4],
    blend_state: &(BlendFactor, BlendFactor),
    mode: BlendMode,
) {
    let (r, g, blue, alpha) = (clamp(color[0]), clamp(color[1]), clamp(color[2]), clamp(color[3]));
    let [dr, dg, db, da_full] = target.load_normalized(pixel);
    let da = if alpha_bits == 0 { 1.0 } else { da_full };
    let out_alpha = |value: f32| if alpha_bits == 0 { 255 } else { byte(value) };
    match mode {
        BlendMode::Opaque => unreachable!("opaque fragments bypass the blend writer"),
        BlendMode::Alpha => {
            let inverse = 1.0 - alpha;
            target.store_bytes(
                pixel,
                [
                    byte(r * alpha + dr * inverse),
                    byte(g * alpha + dg * inverse),
                    byte(blue * alpha + db * inverse),
                    out_alpha(alpha * alpha + da * inverse),
                ],
            );
        }
        BlendMode::Add => {
            target.store_bytes(
                pixel,
                [byte(r + dr), byte(g + dg), byte(blue + db), out_alpha(alpha + da)],
            );
        }
        BlendMode::Multiply => {
            target.store_bytes(
                pixel,
                [byte(r * dr), byte(g * dg), byte(blue * db), out_alpha(alpha * da)],
            );
        }
        BlendMode::DstColorInverseDstAlpha => {
            let inverse = 1.0 - da;
            target.store_bytes(
                pixel,
                [
                    byte(r * dr + dr * inverse),
                    byte(g * dg + dg * inverse),
                    byte(blue * db + db * inverse),
                    out_alpha(alpha * da + da * inverse),
                ],
            );
        }
        BlendMode::General => {
            target.store_bytes(
                pixel,
                [
                    blend(r, dr, alpha, da, blend_state, false),
                    blend(g, dg, alpha, da, blend_state, false),
                    blend(blue, db, alpha, da, blend_state, false),
                    if alpha_bits == 0 {
                        255
                    } else {
                        blend(alpha, da, alpha, da, blend_state, true)
                    },
                ],
            );
        }
    }
}

/// Stencil test configuration for one fragment.
#[derive(Debug, Clone, Copy)]
pub struct StencilTest {
    /// Test function.
    pub function: StencilFunction,
    /// Compare mask.
    pub compare_mask: u32,
    /// Write mask.
    pub write_mask: u32,
    /// Increment saturation maximum.
    pub maximum: u32,
    /// Update when depth fails.
    pub depth_fail: StencilOp,
    /// Update when depth passes.
    pub depth_pass: StencilOp,
}

/// Evaluate the stencil test for one fragment, applying the depth-selected update.
pub fn stencil_fragment(stencil: Option<&mut [u32]>, index: usize, depth_passed: bool, test: StencilTest) -> bool {
    let stencil = stencil.expect("stencil test has no configured storage");
    let previous = *stencil.get(index).expect("fragment is outside the stencil buffer");
    if test.function == StencilFunction::NonZero && (previous & test.compare_mask) == 0 {
        return false;
    }
    let operation = if depth_passed { test.depth_pass } else { test.depth_fail };
    let next = match operation {
        StencilOp::Keep => previous,
        StencilOp::Increment => test.maximum.min(previous + 1),
        StencilOp::Decrement => previous.saturating_sub(1),
    };
    stencil[index] = (previous & !test.write_mask) | (next & test.write_mask);
    depth_passed
}

#[cfg(test)]
mod tests {
    use qa_core::math::{vec3, vec4};

    use super::super::super::types::DepthTest;
    use super::*;

    fn flat_setup<'a>(lighting: CpuTriangleLighting<'a>) -> TriangleSetup<'a> {
        let derivative = TexturePlaneDerivative {
            u_anchor: 0.0,
            v_anchor: 0.0,
            u_dx: 0.0,
            v_dx: 0.0,
            q_dx: 0.0,
            u_dy: 0.0,
            v_dy: 0.0,
            q_dy: 0.0,
        };
        // Right triangle (0,0), (4,0), (0,4); coverage and attributes share edges.
        TriangleSetup {
            fog_depth_scale: 1.0,
            fog: None,
            luminance_alpha: false,
            lighting,
            min_x: 0,
            max_x: 3,
            min_y: 0,
            max_y: 3,
            inverse_area: 1.0 / 16.0,
            depth_near: 0.0,
            depth_far: 1.0,
            edge_ax: -4.0,
            edge_ay: -4.0,
            edge_ac: 16.0,
            edge_bx: 4.0,
            edge_by: 0.0,
            edge_bc: 0.0,
            edge_cx: 0.0,
            edge_cy: 4.0,
            edge_cc: 0.0,
            attribute_ax: -4.0,
            attribute_ay: -4.0,
            attribute_ac: 16.0,
            attribute_bx: 4.0,
            attribute_by: 0.0,
            attribute_bc: 0.0,
            attribute_cx: 0.0,
            attribute_cy: 4.0,
            attribute_cc: 0.0,
            az: 0.0,
            bz: 0.0,
            cz: 0.0,
            polygon_depth_offset: 0.0,
            plane_depth: 0.5,
            aiw: 1.0,
            biw: 1.0,
            ciw: 1.0,
            au: 0.0,
            bu: 0.0,
            cu: 0.0,
            av: 0.0,
            bv: 0.0,
            cv: 0.0,
            au2: 0.0,
            bu2: 0.0,
            cu2: 0.0,
            av2: 0.0,
            bv2: 0.0,
            cv2: 0.0,
            ar: 1.0,
            br: 0.0,
            cr: 0.0,
            ag: 0.0,
            bg: 0.0,
            cg: 0.0,
            ab: 0.0,
            bb: 0.0,
            cb: 0.0,
            aa: 1.0,
            ba: 1.0,
            ca: 1.0,
            edge_a_inclusive: false,
            edge_b_inclusive: true,
            edge_c_inclusive: false,
            white: false,
            depth_write: true,
            stencil_enabled: false,
            color_write: true,
            texture_consumed: true,
            primary_alpha: false,
            secondary_alpha: false,
            constant_depth: true,
            width: 4,
            height: 4,
            blend: (BlendFactor::One, BlendFactor::Zero),
            alpha_bits: 8,
            blending: BlendMode::Opaque,
            depth_test: DepthTest::LessEqual,
            alpha_test: AlphaTest::None,
            secondary_environment: None,
            texture: BoundTexture::Incomplete,
            secondary_texture: BoundTexture::Incomplete,
            derivative,
            secondary_derivative: derivative,
            stencil_function: StencilFunction::Always,
            stencil_compare_mask: 0xffff_ffff,
            stencil_write_mask: 0xffff_ffff,
            stencil_maximum: 255,
            stencil_depth_fail: StencilOp::Keep,
            stencil_depth_pass: StencilOp::Keep,
        }
    }

    #[test]
    fn channel_helpers_pack_bytes() {
        assert_eq!(byte(-0.5), 0);
        assert_eq!(byte(0.5), 128);
        assert_eq!(byte(2.0), 255);
        assert!(!passes_alpha(0.0, AlphaTest::GreaterZero));
        assert!(passes_alpha(0.25, AlphaTest::Less128));
        assert!(passes_alpha(0.75, AlphaTest::GreaterEqual128));
        let word = color_word(1.0, 0.0, 0.0, 1.0);
        let bytes = word.to_ne_bytes();
        assert_eq!(bytes, [255, 0, 0, 255]);
    }

    #[test]
    fn alpha_blend_matches_source_over() {
        let blend_state = (BlendFactor::SrcAlpha, BlendFactor::OneMinusSrcAlpha);
        assert_eq!(blend(1.0, 0.0, 0.5, 1.0, &blend_state, false), 128);
        assert_eq!(blend(0.5, 1.0, 1.0, 1.0, &blend_state, false), 128);
    }

    #[test]
    fn flat_red_triangle_covers_lower_left_half() {
        let lighting_params = BatchLighting::Vertex;
        let setup = flat_setup(CpuTriangleLighting {
            parameters: &lighting_params,
            depth: None,
            positions: [vec3(0.0, 0.0, 0.0); 3],
            normals: [vec3(0.0, 0.0, 1.0); 3],
        });
        let mut frame = Framebuffer::new(4, 4, false);
        let mut sampled = Sample::default();
        assert!(!run_triangle_rows(&setup, &mut frame.whole(), &mut sampled, 0, 3));
        // Covered exactly where x + y < 3; vertex A is red, B and C black.
        let red = |x: usize, y: usize| frame.pixels[(y * 4 + x) * 4];
        assert_eq!(red(0, 0), 191);
        assert_eq!(red(1, 0), 128);
        assert_eq!(red(2, 0), 64);
        assert_eq!(red(3, 0), 0);
        assert_eq!(red(0, 1), 128);
        assert_eq!(red(1, 1), 64);
        assert_eq!(red(2, 1), 0);
        assert_eq!(red(0, 2), 64);
        assert_eq!(red(1, 2), 0);
        assert_eq!(red(0, 3), 0);
        assert_eq!(frame.pixels[3], 255);
        assert_eq!(frame.depth[0], 0.5);
        assert_eq!(frame.depth[15], 1.0);
    }

    #[test]
    fn nearest_and_linear_sampling_match_texels() {
        let storage = TextureStorage {
            width: 2,
            height: 2,
            pixels: Arc::new(vec![
                255, 0, 0, 255, 0, 255, 0, 255, //
                0, 0, 255, 255, 255, 255, 255, 255,
            ]),
            internal_format: ImageInternalFormat::Rgba8,
            has_alpha: true,
            uniform: None,
        };
        let texture = BoundTexture::Image(UploadedTexture {
            internal_format: ImageInternalFormat::Rgba8,
            repeat: true,
            minify_linear: false,
            magnify_linear: false,
            mipmapping: MipMapping::None,
            magnification_limit: 1.0,
            levels: vec![storage],
            border_color: vec4(0.0, 0.0, 0.0, 0.0),
        });
        let mut out = Sample::default();
        sample_bound(&texture, 0.25, 0.25, 0.0, &mut out);
        assert_eq!((out.r, out.g, out.b, out.a), (1.0, 0.0, 0.0, 1.0));
        sample_bound(&texture, 0.75, 0.75, 0.0, &mut out);
        assert_eq!((out.r, out.g, out.b, out.a), (1.0, 1.0, 1.0, 1.0));
        let BoundTexture::Image(image) = texture else {
            unreachable!()
        };
        let linear = BoundTexture::Image(UploadedTexture {
            minify_linear: true,
            magnify_linear: true,
            ..image
        });
        sample_bound(&linear, 0.5, 0.5, 0.0, &mut out);
        assert!((out.r - 0.5).abs() < 1e-6);
        assert!((out.g - 0.5).abs() < 1e-6);
        assert!((out.b - 0.5).abs() < 1e-6);
    }

    #[test]
    fn stencil_write_masks_and_saturates() {
        let mut stencil = vec![0u32; 2];
        let increment = StencilTest {
            function: StencilFunction::Always,
            compare_mask: 0xffff_ffff,
            write_mask: 0xffff_ffff,
            maximum: 1,
            depth_fail: StencilOp::Keep,
            depth_pass: StencilOp::Increment,
        };
        assert!(stencil_fragment(Some(&mut stencil), 0, true, increment));
        assert!(stencil_fragment(Some(&mut stencil), 0, true, increment));
        assert_eq!(stencil[0], 1);
        assert!(!stencil_fragment(
            Some(&mut stencil),
            1,
            true,
            StencilTest {
                function: StencilFunction::NonZero,
                compare_mask: 0xff,
                write_mask: 0xff,
                maximum: 255,
                depth_fail: StencilOp::Keep,
                depth_pass: StencilOp::Keep,
            }
        ));
    }
}
