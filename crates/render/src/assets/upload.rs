//! Cold byte image preparation. Source RGBA and software indices remain owned
//! by their resource; only explicit GL mip levels are produced here.
//!
//! References: WinQuake/gl_draw.c GL_ResampleTexture/GL_MipMap;
//! quake-2/ref_gl/gl_image.c GL_Upload32/GL_LightScaleTexture;
//! quake-iii-arena/code/renderer/tr_image.c Upload32/R_MipMap.

pub const MAX_DIMENSION: u32 = 8192;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExtentRound {
    Up,
    Down,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UploadExtent {
    Original,
    /// Resize directly to the rounded, dropped and independently capped axes.
    PowerOfTwo {
        round: ExtentRound,
        drop: u8,
        max_dimension: u32,
    },
    /// Round/resample first, then reduce with the selected native mip kernel.
    /// The maximum dimension reduces both axes together.
    PowerOfTwoMip {
        round: ExtentRound,
        drop: u8,
        max_dimension: u32,
        kernel: MipmapBuild,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResizeFilter {
    /// Half-step horizontal samples and floor-selected source rows.
    Nearest,
    /// Four samples at native quarter/three-quarter positions, byte averaged.
    FourTap,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MipmapBuild {
    None,
    /// Original GL_MipMap, including its in-place rectangular tail behavior.
    LegacyBox,
    /// R_MipMap simple mode, including native one-dimensional pair averaging.
    Box,
    /// R_MipMap2's wrapped 4x4 weights and unchanged one-dimensional tails.
    Weighted,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ColorOrder {
    BeforeResize,
    AfterResize,
    /// Skip the lookup when the final resize/reduction phase is unchanged.
    /// A preliminary PowerOfTwoMip rounding pass is not that final phase.
    AfterResizeIfChanged,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RgbLut(pub [u8; 256]);
impl Default for RgbLut {
    fn default() -> Self {
        Self(std::array::from_fn(|value| value as u8))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GammaCurve {
    /// Check_Gamma: round(255 * ((byte + 1) / 256)^exponent).
    PalettePower,
    /// BuildGammaTable/GL_InitImages, with an exact identity special case.
    HalfPixelPower,
    /// R_SetColorMappings, exponent is already the reciprocal of r_gamma.
    BytePower,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AlphaFringe {
    KeepPalette,
    /// GL_Upload8 priority/bounds, including cross-row left/right neighbors.
    NativeNeighbors,
}

/// Expand a GL palette independently of the original CPU transparency choice.
/// GL_Upload8 performs this repair before resize, light scaling and mips.
pub fn expand_indexed(
    indices: &[u8],
    width: u32,
    height: u32,
    palette: &[[u8; 4]; 256],
    transparent: Option<u8>,
    fringe: AlphaFringe,
) -> Result<Box<[u8]>, UploadError> {
    let bytes = byte_count(width, height)?;
    let count = bytes / 4;
    if indices.len() != count {
        return Err(UploadError::PixelCount);
    }
    let width = width as usize;
    let mut pixels = vec![0; bytes];
    for (i, &index) in indices.iter().enumerate() {
        let mut color = palette[index as usize];
        if transparent == Some(index) {
            color[3] = 0;
            if fringe == AlphaFringe::NativeNeighbors {
                let index = if i > width && transparent != Some(indices[i - width]) {
                    indices[i - width]
                } else if i < count - width && transparent != Some(indices[i + width]) {
                    indices[i + width]
                } else if i > 0 && transparent != Some(indices[i - 1]) {
                    indices[i - 1]
                } else if i < count - 1 && transparent != Some(indices[i + 1]) {
                    indices[i + 1]
                } else {
                    0
                };
                color[..3].copy_from_slice(&palette[index as usize][..3]);
            }
        }
        pixels[i * 4..i * 4 + 4].copy_from_slice(&color);
    }
    Ok(pixels.into_boxed_slice())
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LightScale {
    pub rgb_lut: RgbLut,
    /// Keep this floating factor; quantizing it to a stage color changes 0.5.
    pub inverse_intensity: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct UploadParams {
    pub extent: UploadExtent,
    pub resize: ResizeFilter,
    pub mipmaps: MipmapBuild,
    pub color_order: ColorOrder,
    pub rgb_lut: RgbLut,
    pub inverse_intensity: f32,
}
impl Default for UploadParams {
    fn default() -> Self {
        Self {
            extent: UploadExtent::Original,
            resize: ResizeFilter::FourTap,
            mipmaps: MipmapBuild::None,
            color_order: ColorOrder::AfterResize,
            rgb_lut: RgbLut::default(),
            inverse_intensity: 1.0,
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct MipLevel {
    pub width: u32,
    pub height: u32,
    pub rgba: Box<[u8]>,
}

#[derive(Debug, PartialEq)]
pub struct PreparedImage {
    pub levels: Box<[MipLevel]>,
    pub inverse_intensity: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UploadError {
    Dimensions,
    PixelCount,
    Extent,
    NonPowerOfTwoMip,
    LegacyMipBounds,
    InvalidColor,
}

pub fn gamma_lut(exponent: f32, curve: GammaCurve) -> Result<RgbLut, UploadError> {
    if !exponent.is_finite() {
        return Err(UploadError::InvalidColor);
    }
    let mut result = RgbLut::default();
    if exponent == 1.0 && curve != GammaCurve::PalettePower {
        return Ok(result);
    }
    for (value, out) in result.0.iter_mut().enumerate() {
        // Preserve the original float assignment sites around the double pow.
        *out = match curve {
            GammaCurve::PalettePower => {
                let power = (((value + 1) as f64 / 256.0).powf(f64::from(exponent))) as f32;
                (power * 255.0 + 0.5).clamp(0.0, 255.0) as u8
            }
            GammaCurve::HalfPixelPower => {
                let rounded =
                    (255.0 * ((value as f64 + 0.5) / 255.5).powf(f64::from(exponent)) + 0.5) as f32;
                rounded.clamp(0.0, 255.0) as u8
            }
            GammaCurve::BytePower => {
                let base = value as f32 / 255.0;
                (255.0 * f64::from(base).powf(f64::from(exponent)) + 0.5).clamp(0.0, 255.0) as u8
            }
        };
    }
    Ok(result)
}

/// GL_InitImages clamps intensity <= 1 to 1. RGB alone is transformed;
/// intensity truncation/saturation occurs before the gamma lookup.
pub fn light_scale(gamma: RgbLut, intensity: f32) -> Result<LightScale, UploadError> {
    if !intensity.is_finite() {
        return Err(UploadError::InvalidColor);
    }
    let intensity = intensity.max(1.0);
    Ok(LightScale {
        rgb_lut: RgbLut(std::array::from_fn(|value| {
            let scaled = (value as f32 * intensity).min(255.0) as usize;
            gamma.0[scaled]
        })),
        inverse_intensity: 1.0 / intensity,
    })
}

pub fn prepare_rgba(
    width: u32,
    height: u32,
    rgba: &[u8],
    params: UploadParams,
) -> Result<PreparedImage, UploadError> {
    let length = byte_count(width, height)?;
    if rgba.len() != length {
        return Err(UploadError::PixelCount);
    }
    if !params.inverse_intensity.is_finite() || params.inverse_intensity <= 0.0 {
        return Err(UploadError::InvalidColor);
    }
    validate_kernel(params.mipmaps, true)?;
    let mut source = rgba.to_vec();
    if params.color_order == ColorOrder::BeforeResize {
        apply_lut(&mut source, params.rgb_lut);
    }
    let (mut pixels, mut width, mut height, changed) = match params.extent {
        UploadExtent::Original => (source, width, height, false),
        UploadExtent::PowerOfTwo {
            round,
            drop,
            max_dimension,
        } => {
            validate_extent(drop, max_dimension)?;
            let target_width = (rounded(width, round) >> drop).clamp(1, max_dimension);
            let target_height = (rounded(height, round) >> drop).clamp(1, max_dimension);
            let changed = target_width != width || target_height != height;
            let pixels = if changed {
                resample(
                    &source,
                    width,
                    height,
                    target_width,
                    target_height,
                    params.resize,
                )
            } else {
                source
            };
            (pixels, target_width, target_height, changed)
        }
        UploadExtent::PowerOfTwoMip {
            round,
            drop,
            max_dimension,
            kernel,
        } => {
            validate_extent(drop, max_dimension)?;
            validate_kernel(kernel, false)?;
            let mut w = rounded(width, round);
            let mut h = rounded(height, round);
            let mut pixels = if w != width || h != height {
                resample(&source, width, height, w, h, params.resize)
            } else {
                source
            };
            let mut target_width = (w >> drop).max(1);
            let mut target_height = (h >> drop).max(1);
            while target_width > max_dimension || target_height > max_dimension {
                target_width = (target_width >> 1).max(1);
                target_height = (target_height >> 1).max(1);
            }
            let changed = target_width != w || target_height != h;
            while w > target_width || h > target_height {
                reduce(&mut pixels, w, h, kernel)?;
                w = (w >> 1).max(1);
                h = (h >> 1).max(1);
            }
            // Keep the initialized backing for the legacy kernel's tails.
            (pixels, w, h, changed)
        }
    };
    if params.color_order == ColorOrder::AfterResize
        || (params.color_order == ColorOrder::AfterResizeIfChanged && changed)
    {
        apply_lut(&mut pixels[..byte_count(width, height)?], params.rgb_lut);
    }
    if params.mipmaps != MipmapBuild::None
        && (!width.is_power_of_two() || !height.is_power_of_two())
    {
        return Err(UploadError::NonPowerOfTwoMip);
    }
    let mut levels = Vec::with_capacity(if params.mipmaps == MipmapBuild::None {
        1
    } else {
        (width.max(height).ilog2() + 1) as usize
    });
    loop {
        levels.push(MipLevel {
            width,
            height,
            rgba: pixels[..byte_count(width, height)?].into(),
        });
        if params.mipmaps == MipmapBuild::None || (width == 1 && height == 1) {
            break;
        }
        reduce(&mut pixels, width, height, params.mipmaps)?;
        width = (width >> 1).max(1);
        height = (height >> 1).max(1);
    }
    Ok(PreparedImage {
        levels: levels.into_boxed_slice(),
        inverse_intensity: params.inverse_intensity,
    })
}

fn byte_count(width: u32, height: u32) -> Result<usize, UploadError> {
    if width == 0 || height == 0 || width > MAX_DIMENSION || height > MAX_DIMENSION {
        return Err(UploadError::Dimensions);
    }
    Ok(width as usize * height as usize * 4)
}

fn validate_extent(drop: u8, max_dimension: u32) -> Result<(), UploadError> {
    if drop >= 32 || max_dimension == 0 || max_dimension > MAX_DIMENSION {
        Err(UploadError::Extent)
    } else {
        Ok(())
    }
}

fn validate_kernel(kernel: MipmapBuild, allow_none: bool) -> Result<(), UploadError> {
    match kernel {
        MipmapBuild::None if !allow_none => Err(UploadError::Extent),
        _ => Ok(()),
    }
}

fn rounded(value: u32, round: ExtentRound) -> u32 {
    let next = value.next_power_of_two();
    if round == ExtentRound::Down && next != value {
        next >> 1
    } else {
        next
    }
}

fn apply_lut(pixels: &mut [u8], lut: RgbLut) {
    for pixel in pixels.chunks_exact_mut(4) {
        for channel in &mut pixel[..3] {
            *channel = lut.0[*channel as usize];
        }
    }
}

fn resample(
    input: &[u8],
    width: u32,
    height: u32,
    out_width: u32,
    out_height: u32,
    filter: ResizeFilter,
) -> Vec<u8> {
    let mut output = vec![0; out_width as usize * out_height as usize * 4];
    let step = (u64::from(width) << 16) / u64::from(out_width);
    for y in 0..out_height {
        for x in 0..out_width {
            let out = (y as usize * out_width as usize + x as usize) * 4;
            match filter {
                ResizeFilter::Nearest => {
                    let row = u64::from(y) * u64::from(height) / u64::from(out_height);
                    let column = ((step >> 1) + u64::from(x) * step) >> 16;
                    let offset = (row as usize * width as usize + column as usize) * 4;
                    output[out..out + 4].copy_from_slice(&input[offset..offset + 4]);
                }
                ResizeFilter::FourTap => {
                    let rows = [1, 3].map(|quarter| {
                        (u64::from(y) * 4 + quarter) * u64::from(height)
                            / (u64::from(out_height) * 4)
                    });
                    let columns =
                        [1, 3].map(|quarter| (quarter * (step >> 2) + u64::from(x) * step) >> 16);
                    let offsets = [
                        (rows[0] as usize * width as usize + columns[0] as usize) * 4,
                        (rows[0] as usize * width as usize + columns[1] as usize) * 4,
                        (rows[1] as usize * width as usize + columns[0] as usize) * 4,
                        (rows[1] as usize * width as usize + columns[1] as usize) * 4,
                    ];
                    for channel in 0..4 {
                        let total: u16 = offsets
                            .iter()
                            .map(|&offset| u16::from(input[offset + channel]))
                            .sum();
                        output[out + channel] = (total >> 2) as u8;
                    }
                }
            }
        }
    }
    output
}

fn reduce(
    pixels: &mut [u8],
    width: u32,
    height: u32,
    kernel: MipmapBuild,
) -> Result<(), UploadError> {
    if !width.is_power_of_two() || !height.is_power_of_two() {
        return Err(UploadError::NonPowerOfTwoMip);
    }
    if width == 1 && height == 1 {
        return Ok(());
    }
    if kernel == MipmapBuild::Weighted {
        reduce_weighted(pixels, width, height);
        return Ok(());
    }
    let row = width as usize * 4;
    if kernel == MipmapBuild::Box && (width == 1 || height == 1) {
        for pixel in 0..(width.max(height) as usize >> 1) {
            for channel in 0..4 {
                let total = u16::from(pixels[pixel * 8 + channel])
                    + u16::from(pixels[pixel * 8 + 4 + channel]);
                pixels[pixel * 4 + channel] = (total >> 1) as u8;
            }
        }
        return Ok(());
    }
    // GL_MipMap does not write once height reaches one. For width one it
    // advances three source pixels per output, retaining old backing bytes.
    let rows = height as usize >> 1;
    if rows == 0 {
        return Ok(());
    }
    let columns = (width as usize >> 1).max(1);
    let stride = columns * 8 + row;
    let end = (rows - 1) * stride + (columns - 1) * 8 + row + 8;
    if end > pixels.len() {
        return Err(UploadError::LegacyMipBounds);
    }
    for y in 0..rows {
        for x in 0..columns {
            let input = y * stride + x * 8;
            let output = (y * columns + x) * 4;
            for channel in 0..4 {
                let total = u16::from(pixels[input + channel])
                    + u16::from(pixels[input + 4 + channel])
                    + u16::from(pixels[input + row + channel])
                    + u16::from(pixels[input + row + 4 + channel]);
                pixels[output + channel] = (total >> 2) as u8;
            }
        }
    }
    Ok(())
}

/// tr_image.c R_MipMap2: source samples stay untouched until the complete
/// temporary output is ready. Wrap uses the native power-of-two bit masks.
fn reduce_weighted(pixels: &mut [u8], width: u32, height: u32) {
    let out_width = width >> 1;
    let out_height = height >> 1;
    // Native output is zero-sized once either axis is one. Upload32 clamps
    // its next dimensions to one, retaining the old initialized prefix.
    if out_width == 0 || out_height == 0 {
        return;
    }
    let mut temporary = vec![0; out_width as usize * out_height as usize * 4];
    let weights = [1u16, 2, 2, 1];
    for y in 0..out_height {
        for x in 0..out_width {
            let mut total = [0u16; 4];
            for (dy, &wy) in weights.iter().enumerate() {
                let row = ((y * 2 + dy as u32).wrapping_sub(1) & (height - 1)) as usize;
                for (dx, &wx) in weights.iter().enumerate() {
                    let column = ((x * 2 + dx as u32).wrapping_sub(1) & (width - 1)) as usize;
                    let source = (row * width as usize + column) * 4;
                    for channel in 0..4 {
                        total[channel] += wy * wx * u16::from(pixels[source + channel]);
                    }
                }
            }
            let target = (y as usize * out_width as usize + x as usize) * 4;
            for channel in 0..4 {
                temporary[target + channel] = (total[channel] / 36) as u8;
            }
        }
    }
    pixels[..temporary.len()].copy_from_slice(&temporary);
}
