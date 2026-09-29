//! Lightstyles and lightmap accumulation (`gl_rlight.c`, Ironwail).
//!
//! Donor provenance: `src/materials/lighting.ts` (from Q1/Q2
//! `gl_rlight.c`/`gl_light.c`/`gl_rsurf.c`, Ironwail, q2repro).

use qa_core::math::{dot3, scale3, sub3, Plane, Vec2, Vec3, Vec4};

use crate::ClientError;

/// Q2 light style (`Q2LightStyle`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Q2LightStyle {
    /// RGB scale.
    pub rgb: Vec3,
    /// White (RGB sum).
    pub white: f32,
}

/// Q1 light style in 8.8 units (`q1LightStyle`).
///
/// Mode 1 leaves jumps of six letters or more unsmoothed.
#[must_use]
pub fn q1_light_style(map: &str, time: f32, interpolation: u8) -> i32 {
    if map.is_empty() {
        return 256;
    }
    let bytes = map.as_bytes();
    let phase = time * 10.0;
    let frame = phase.floor() as i64;
    let len = bytes.len() as i64;
    let index = ((frame % len) + len) % len;
    let current = i32::from(bytes[index as usize]).saturating_sub(97);
    let mut next = i32::from(bytes[(index + 1) as usize % bytes.len()]).saturating_sub(97);
    if interpolation < 2 && (next - current).abs() >= 6 {
        next = current;
    }
    let fraction = if interpolation == 0 { 0.0 } else { phase - frame as f32 };
    (current as f32 * 22.0 + (next - current) as f32 * 22.0 * fraction).trunc() as i32
}

/// Q2 light style (`q2LightStyle`, 100 ms ticks, `m` = 1.0).
#[must_use]
pub fn q2_light_style(map: &str, milliseconds: i32) -> Q2LightStyle {
    let frame = milliseconds / 100;
    let value = if map.is_empty() {
        1.0
    } else {
        let bytes = map.as_bytes();
        let len = bytes.len() as i32;
        let index = ((frame % len) + len) % len;
        f32::from(bytes[index as usize].saturating_sub(97)) / 12.0
    };
    Q2LightStyle {
        rgb: qa_core::math::vec3(value, value, value),
        white: value + value + value,
    }
}

/// Classic texture projection.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TextureProjection {
    /// S axis plus offset.
    pub s: Vec4,
    /// T axis plus offset.
    pub t: Vec4,
}

/// Decoupled lightmap mapping.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DecoupledLightmap {
    /// Sample axes.
    pub axes: [Vec3; 2],
    /// Offset.
    pub offset: Vec2,
}

/// Lightmap projection (`LightmapProjection`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LightmapProjection {
    /// Classic 16-unit projection.
    Classic {
        /// Texture projection.
        texture: TextureProjection,
        /// Texture minimums.
        texture_mins: Vec2,
    },
    /// Decoupled BSPX mapping.
    Decoupled {
        /// Mapping.
        mapping: DecoupledLightmap,
    },
}

/// Lightmap coordinates (`lightmapCoordinates`).
#[must_use]
pub fn lightmap_coordinates(position: Vec3, projection: &LightmapProjection) -> Vec2 {
    match projection {
        LightmapProjection::Decoupled { mapping } => qa_core::math::vec2(
            dot3(position, mapping.axes[0]) + mapping.offset.x,
            dot3(position, mapping.axes[1]) + mapping.offset.y,
        ),
        LightmapProjection::Classic { texture, texture_mins } => {
            let s = qa_core::math::vec3(texture.s.x, texture.s.y, texture.s.z);
            let t = qa_core::math::vec3(texture.t.x, texture.t.y, texture.t.z);
            qa_core::math::vec2(
                (dot3(position, s) + texture.s.w - texture_mins.x) / 16.0,
                (dot3(position, t) + texture.t.w - texture_mins.y) / 16.0,
            )
        }
    }
}

/// BSP lighting samples (`BspLighting` subset).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BspLighting {
    /// Luminance samples.
    Luminance8 {
        /// Samples.
        samples: Vec<u8>,
    },
    /// RGB samples.
    Rgb8 {
        /// Samples.
        samples: Vec<u8>,
    },
}

/// A lightmapped face (`LightmapFace`).
#[derive(Debug, Clone, PartialEq)]
pub struct LightmapFace {
    /// Width in texels.
    pub width: usize,
    /// Height in texels.
    pub height: usize,
    /// Lighting samples.
    pub lighting: Option<BspLighting>,
    /// Sample offset.
    pub offset: usize,
    /// Light styles.
    pub styles: Vec<u8>,
    /// Face plane.
    pub plane: Plane,
    /// Projection.
    pub projection: LightmapProjection,
}

/// A dynamic light affecting a surface (`SurfaceDynamicLight`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SurfaceDynamicLight {
    /// Origin.
    pub origin: Vec3,
    /// Radius.
    pub radius: f32,
    /// Minimum radius.
    pub minimum: f32,
    /// Color.
    pub color: Vec3,
}

fn element(values: &[f32], index: usize) -> Result<f32, ClientError> {
    values
        .get(index)
        .copied()
        .ok_or_else(|| ClientError::BadMaterial(format!("Lightmap sample {index} is outside {} values", values.len())))
}

fn face_size(face: &LightmapFace) -> Result<usize, ClientError> {
    if face.width < 1 || face.height < 1 {
        return Err(ClientError::BadMaterial(
            "Lightmap dimensions must be positive integers".to_string(),
        ));
    }
    Ok(face.width * face.height)
}

fn sample(face: &LightmapFace, style: usize, pixel: usize, channel: usize) -> u8 {
    let Some(lighting) = &face.lighting else {
        return 255;
    };
    let (samples, channels) = match lighting {
        BspLighting::Luminance8 { samples } => (samples, 1),
        BspLighting::Rgb8 { samples } => (samples, 3),
    };
    let size = face.width * face.height;
    let index = face.offset + (style * size + pixel) * channels + if channels == 1 { 0 } else { channel };
    samples.get(index).copied().unwrap_or(0)
}

fn add_dynamic_lights(block: &mut [f32], face: &LightmapFace, lights: &[SurfaceDynamicLight], scale: f32) {
    for light in lights {
        let distance = dot3(light.origin, face.plane.normal) - face.plane.distance;
        let radius = light.radius - distance.abs();
        if radius < light.minimum {
            continue;
        }
        let threshold = radius - light.minimum;
        let impact = sub3(light.origin, scale3(face.plane.normal, distance));
        let uv = lightmap_coordinates(impact, &face.projection);
        let (step_s, step_t) = match &face.projection {
            LightmapProjection::Classic { .. } => (16.0f32, 16.0f32),
            LightmapProjection::Decoupled { mapping } => {
                let a = mapping.axes[0];
                let b = mapping.axes[1];
                (
                    1.0 / (a.x * a.x + a.y * a.y + a.z * a.z).sqrt(),
                    1.0 / (b.x * b.x + b.y * b.y + b.z * b.z).sqrt(),
                )
            }
        };
        for y in 0..face.height {
            for x in 0..face.width {
                let sd = ((uv.x - x as f32) * step_s).trunc().abs() as i32;
                let td = ((uv.y - y as f32) * step_t).trunc().abs() as i32;
                let separation = if sd > td { sd + (td >> 1) } else { td + (sd >> 1) };
                if separation as f32 >= threshold {
                    continue;
                }
                let value = (radius - separation as f32) * scale;
                let offset = (y * face.width + x) * 3;
                block[offset] += value * light.color.x;
                block[offset + 1] += value * light.color.y;
                block[offset + 2] += value * light.color.z;
            }
        }
    }
}

/// Q1 lightmap encoding (`Q1LightmapEncoding`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Q1LightmapEncoding {
    /// RGB.
    Rgb,
    /// Inverted luminance.
    InvertedLuminance,
    /// Inverted alpha.
    InvertedAlpha,
}

/// Built lightmap encoding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BuiltLightmapEncoding {
    /// Q1 RGB.
    Rgb,
    /// Q1 inverted luminance.
    InvertedLuminance,
    /// Q1 inverted alpha.
    InvertedAlpha,
    /// Q2 RGB.
    Q2Rgb,
    /// Q2 monochrome.
    Q2Mono,
}

/// A built lightmap (`BuiltLightmap`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuiltLightmap {
    /// Width.
    pub width: usize,
    /// Height.
    pub height: usize,
    /// RGBA pixels.
    pub pixels: Vec<u8>,
    /// Encoding.
    pub encoding: BuiltLightmapEncoding,
}

/// Build a Q1 lightmap (`buildQ1Lightmap`).
pub fn build_q1_lightmap(
    face: &LightmapFace,
    styles: &[i32],
    encoding: Option<Q1LightmapEncoding>,
    fullbright: bool,
    dynamic_lights: &[SurfaceDynamicLight],
) -> Result<BuiltLightmap, ClientError> {
    let size = face_size(face)?;
    let mut block = vec![0f32; size * 3];
    let encoding = encoding.unwrap_or(match &face.lighting {
        Some(BspLighting::Rgb8 { .. }) => Q1LightmapEncoding::Rgb,
        _ => Q1LightmapEncoding::InvertedLuminance,
    });
    if fullbright || face.lighting.is_none() {
        block.fill(255.0 * 256.0);
    } else {
        for (map, style) in face.styles.iter().enumerate() {
            if *style == 255 {
                break;
            }
            let scale = styles.get(usize::from(*style)).copied().ok_or_else(|| {
                ClientError::BadMaterial(format!("Lightmap sample {} is outside {} values", style, styles.len()))
            })?;
            for pixel in 0..size {
                for channel in 0..3 {
                    let offset = pixel * 3 + channel;
                    block[offset] += f32::from(sample(face, map, pixel, channel)) * scale as f32;
                }
            }
        }
    }
    if !fullbright && face.lighting.is_some() {
        add_dynamic_lights(&mut block, face, dynamic_lights, 256.0);
    }
    let mut pixels = vec![0u8; size * 4];
    for pixel in 0..size {
        let brightness = |channel: usize| -> Result<u8, ClientError> {
            let sample = element(&block, pixel * 3 + channel)? as u32;
            Ok((sample >> 7).min(255) as u8)
        };
        match encoding {
            Q1LightmapEncoding::Rgb => {
                pixels[pixel * 4] = brightness(0)?;
                pixels[pixel * 4 + 1] = brightness(1)?;
                pixels[pixel * 4 + 2] = brightness(2)?;
                pixels[pixel * 4 + 3] = 255;
            }
            Q1LightmapEncoding::InvertedLuminance => {
                let value = 255 - brightness(0)?;
                pixels[pixel * 4] = value;
                pixels[pixel * 4 + 1] = value;
                pixels[pixel * 4 + 2] = value;
                pixels[pixel * 4 + 3] = 255;
            }
            Q1LightmapEncoding::InvertedAlpha => {
                pixels[pixel * 4] = 0;
                pixels[pixel * 4 + 1] = 0;
                pixels[pixel * 4 + 2] = 0;
                pixels[pixel * 4 + 3] = 255 - brightness(0)?;
            }
        }
    }
    Ok(BuiltLightmap {
        width: face.width,
        height: face.height,
        pixels,
        encoding: match encoding {
            Q1LightmapEncoding::Rgb => BuiltLightmapEncoding::Rgb,
            Q1LightmapEncoding::InvertedLuminance => BuiltLightmapEncoding::InvertedLuminance,
            Q1LightmapEncoding::InvertedAlpha => BuiltLightmapEncoding::InvertedAlpha,
        },
    })
}

/// Q2 monochrome mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Q2Mono {
    /// Color.
    Color,
    /// Luminance/intensity.
    Luminance,
    /// Alpha.
    Alpha,
    /// Contrast.
    Contrast,
}

/// Build a Q2 lightmap (`buildQ2Lightmap`).
pub fn build_q2_lightmap(
    face: &LightmapFace,
    styles: &[Q2LightStyle],
    modulate: f32,
    mono: Q2Mono,
    dynamic_lights: &[SurfaceDynamicLight],
) -> Result<BuiltLightmap, ClientError> {
    let size = face_size(face)?;
    let mut block = vec![0f32; size * 3];
    if face.lighting.is_none() {
        block.fill(255.0);
    } else {
        for (map, style_index) in face.styles.iter().enumerate() {
            if *style_index == 255 {
                break;
            }
            let style = styles
                .get(usize::from(*style_index))
                .copied()
                .ok_or_else(|| ClientError::BadMaterial(format!("Missing Q2 lightstyle {style_index}")))?;
            let scales = [style.rgb.x * modulate, style.rgb.y * modulate, style.rgb.z * modulate];
            for pixel in 0..size {
                for (channel, scale) in scales.iter().enumerate() {
                    let offset = pixel * 3 + channel;
                    block[offset] += f32::from(sample(face, map, pixel, channel)) * *scale;
                }
            }
        }
    }
    if face.lighting.is_some() {
        add_dynamic_lights(&mut block, face, dynamic_lights, 1.0);
    }
    let mut pixels = vec![0u8; size * 4];
    for pixel in 0..size {
        let mut r = element(&block, pixel * 3)?.max(0.0).trunc() as i32;
        let mut g = element(&block, pixel * 3 + 1)?.max(0.0).trunc() as i32;
        let mut b = element(&block, pixel * 3 + 2)?.max(0.0).trunc() as i32;
        let mut a = r.max(g).max(b);
        if a > 255 {
            let scale = 255.0 / a as f32;
            r = (r as f32 * scale).trunc() as i32;
            g = (g as f32 * scale).trunc() as i32;
            b = (b as f32 * scale).trunc() as i32;
            a = (a as f32 * scale).trunc() as i32;
        }
        let (r, g, b, a) = match mono {
            Q2Mono::Color => (r, g, b, a),
            Q2Mono::Luminance => (a, 0, 0, a),
            Q2Mono::Alpha => (0, 0, 0, 255 - a),
            Q2Mono::Contrast => {
                let alpha = 255 - (r + g + b) / 3;
                (r * alpha / 255, g * alpha / 255, b * alpha / 255, alpha)
            }
        };
        pixels[pixel * 4] = r as u8;
        pixels[pixel * 4 + 1] = g as u8;
        pixels[pixel * 4 + 2] = b as u8;
        pixels[pixel * 4 + 3] = a as u8;
    }
    Ok(BuiltLightmap {
        width: face.width,
        height: face.height,
        pixels,
        encoding: if mono == Q2Mono::Color {
            BuiltLightmapEncoding::Q2Rgb
        } else {
            BuiltLightmapEncoding::Q2Mono
        },
    })
}

/// Atlas coordinates with half-texel bias (`lightmapAtlasCoordinates`).
#[must_use]
pub fn lightmap_atlas_coordinates(
    position: Vec3,
    projection: &LightmapProjection,
    location: Vec2,
    atlas_width: f32,
    atlas_height: f32,
) -> Vec2 {
    let uv = lightmap_coordinates(position, projection);
    qa_core::math::vec2(
        (uv.x + location.x + 0.5) / atlas_width,
        (uv.y + location.y + 0.5) / atlas_height,
    )
}

/// Lightmap filtering.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LightmapFiltering {
    /// Nearest.
    Nearest,
    /// Bilinear.
    Bilinear,
}

/// Sample a lightmap (`sampleLightmap`).
#[must_use]
pub fn sample_lightmap(image: &BuiltLightmap, uv: Vec2, filtering: LightmapFiltering) -> Option<Vec4> {
    if uv.x < 0.0 || uv.y < 0.0 || uv.x > image.width as f32 - 1.0 || uv.y > image.height as f32 - 1.0 {
        return None;
    }
    let x = uv.x.floor() as usize;
    let y = uv.y.floor() as usize;
    let read = |sx: usize, sy: usize, channel: usize| -> f32 {
        f32::from(image.pixels[(sy * image.width + sx) * 4 + channel])
    };
    let component = |channel: usize| -> f32 {
        if filtering == LightmapFiltering::Nearest {
            return read(x, y, channel);
        }
        let x1 = (x + 1).min(image.width - 1);
        let y1 = (y + 1).min(image.height - 1);
        let fx = uv.x - x as f32;
        let fy = uv.y - y as f32;
        (read(x, y, channel) * (1.0 - fx) + read(x1, y, channel) * fx) * (1.0 - fy)
            + (read(x, y1, channel) * (1.0 - fx) + read(x1, y1, channel) * fx) * fy
    };
    Some(qa_core::math::vec4(
        component(0),
        component(1),
        component(2),
        component(3),
    ))
}

/// Skyline lightmap atlas (`LightmapAtlas`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LightmapAtlas {
    widths: Vec<i32>,
    width: usize,
    height: usize,
}

impl LightmapAtlas {
    /// New atlas.
    pub fn new(width: usize, height: usize) -> Result<Self, ClientError> {
        if width < 1 || height < 1 {
            return Err(ClientError::BadMaterial(
                "Invalid lightmap atlas dimensions".to_string(),
            ));
        }
        Ok(Self {
            widths: vec![0; width],
            width,
            height,
        })
    }

    /// Clear the atlas.
    pub fn clear(&mut self) {
        self.widths.fill(0);
    }

    /// Allocate a rectangle.
    pub fn allocate(&mut self, width: usize, height: usize) -> Result<Option<Vec2>, ClientError> {
        if width < 1 || height < 1 {
            return Err(ClientError::BadMaterial("Invalid lightmap rectangle".to_string()));
        }
        let mut best = self.height as i32;
        let mut location: Option<usize> = None;
        for x in 0..=self.width.saturating_sub(width) {
            let mut top = 0i32;
            let mut accepted = true;
            for column in 0..width {
                let value = self.widths[x + column];
                if value >= best {
                    accepted = false;
                    break;
                }
                top = top.max(value);
            }
            if accepted {
                best = top;
                location = Some(x);
            }
        }
        let Some(location) = location else {
            return Ok(None);
        };
        if best as usize + height > self.height {
            return Ok(None);
        }
        for column in 0..width {
            self.widths[location + column] = best + height as i32;
        }
        Ok(Some(qa_core::math::vec2(location as f32, best as f32)))
    }
}

/// Direct upload pixels (`directLightmapPixels`).
pub fn direct_lightmap_pixels(built: &BuiltLightmap) -> Result<BuiltLightmap, ClientError> {
    if built.encoding == BuiltLightmapEncoding::Q2Mono {
        return Err(ClientError::BadMaterial(
            "Q2 alternate monochrome encodings require their matching texture environment".to_string(),
        ));
    }
    let mut pixels = built.pixels.clone();
    for offset in (0..pixels.len()).step_by(4) {
        if built.encoding == BuiltLightmapEncoding::InvertedAlpha {
            let value = 255 - pixels[offset + 3];
            pixels[offset] = value;
            pixels[offset + 1] = value;
            pixels[offset + 2] = value;
        } else if built.encoding == BuiltLightmapEncoding::InvertedLuminance {
            for channel in 0..3 {
                pixels[offset + channel] = 255 - pixels[offset + channel];
            }
        }
        pixels[offset + 3] = 255;
    }
    Ok(BuiltLightmap {
        width: built.width,
        height: built.height,
        pixels,
        encoding: built.encoding,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn face() -> LightmapFace {
        LightmapFace {
            width: 2,
            height: 2,
            lighting: Some(BspLighting::Luminance8 { samples: vec![128; 4] }),
            offset: 0,
            styles: vec![0],
            plane: Plane {
                normal: qa_core::math::vec3(0.0, 0.0, 1.0),
                distance: 0.0,
            },
            projection: LightmapProjection::Decoupled {
                mapping: DecoupledLightmap {
                    axes: [qa_core::math::vec3(1.0, 0.0, 0.0), qa_core::math::vec3(0.0, 1.0, 0.0)],
                    offset: qa_core::math::vec2(0.0, 0.0),
                },
            },
        }
    }

    #[test]
    fn q1_style_mid_frame() {
        assert_eq!(q1_light_style("a", 0.0, 0), 0);
        assert_eq!(q1_light_style("", 99.0, 0), 256);
        assert_eq!(q1_light_style("m", 0.0, 0), 12 * 22);
    }

    #[test]
    fn q2_style_ticks() {
        let style = q2_light_style("m", 0);
        assert!((style.rgb.x - 1.0).abs() < 1e-6);
        assert!((style.white - 3.0).abs() < 1e-6);
    }

    #[test]
    fn q1_lightmap_builds() {
        let built = build_q1_lightmap(&face(), &[256], None, false, &[]).unwrap();
        assert_eq!(built.width, 2);
        assert_eq!(built.pixels.len(), 16);
    }

    #[test]
    fn q2_lightmap_builds() {
        let style = q2_light_style("m", 0);
        let built = build_q2_lightmap(&face(), &[style], 1.0, Q2Mono::Color, &[]).unwrap();
        assert_eq!(built.encoding, BuiltLightmapEncoding::Q2Rgb);
    }

    #[test]
    fn atlas_packs() {
        let mut atlas = LightmapAtlas::new(128, 128).unwrap();
        assert!(atlas.allocate(64, 64).unwrap().is_some());
        assert!(atlas.allocate(64, 64).unwrap().is_some());
        assert!(atlas.allocate(65, 65).unwrap().is_none());
    }

    #[test]
    fn direct_pixels_force_opaque() {
        let built = build_q1_lightmap(&face(), &[256], None, false, &[]).unwrap();
        let direct = direct_lightmap_pixels(&built).unwrap();
        assert!(direct.pixels.iter().skip(3).step_by(4).all(|alpha| *alpha == 255));
    }
}
