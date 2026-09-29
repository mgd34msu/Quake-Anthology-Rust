//! Stage color evaluation (`ComputeColors`, `R_BindAnimatedImage`).
//!
//! Donor provenance: `src/materials/color.ts` (from Q3 renderer).

use qa_content::md3::{inverse_sqrt32, normalize_fast3};
use qa_core::math::{dot3, sub3, vec3, Vec3, Vec4};

use super::deform::RendererNoise;
use super::geometry::MaterialVertex;
use super::material::{evaluate_waveform, ShaderStage, SourceColorGen, WaveKind, Waveform};
use super::q3_lighting::EntityLighting;
use crate::ClientError;

/// Stage color context (`StageColorContext`).
pub struct StageColorContext<'a> {
    /// Shader time.
    pub time: f32,
    /// Identity light scale.
    pub identity_light: f32,
    /// Entity RGBA bytes.
    pub entity_rgba: [u8; 4],
    /// Entity lighting (for `lightingDiffuse`).
    pub lighting: Option<EntityLighting>,
    /// View origin.
    pub view_origin: Vec3,
    /// Local (model-space) view origin.
    pub local_view_origin: Vec3,
    /// Renderer noise.
    pub noise: &'a RendererNoise,
    /// Previous stage color (normalized).
    pub previous_color: Vec4,
}

fn byte(value: f32) -> Result<u8, ClientError> {
    if !value.is_finite() {
        return Err(ClientError::BadMaterial(
            "Shader color reaches undefined source float-to-integer conversion".to_string(),
        ));
    }
    let integer = value.trunc() as i64;
    if !(i64::from(i32::MIN)..=i64::from(i32::MAX)).contains(&integer) {
        return Err(ClientError::BadMaterial(
            "Shader color reaches undefined source float-to-integer conversion".to_string(),
        ));
    }
    #[allow(clippy::cast_possible_truncation)]
    Ok((integer as i32) as u8)
}

fn normalized_byte(value: f32) -> Result<u8, ClientError> {
    byte(value * 255.0)
}

fn wave_byte(value: f32) -> Result<u8, ClientError> {
    normalized_byte(value.clamp(0.0, 1.0))
}

/// Evaluate a full stage color (`evaluateStageColor`).
pub fn evaluate_stage_color(
    stage: &ShaderStage,
    vertex: &MaterialVertex,
    context: &StageColorContext,
    skip_alpha: bool,
    source_rgb: Option<SourceColorGen>,
) -> Result<Vec4, ClientError> {
    let color = evaluate_stage_rgb_color(stage, vertex, context, source_rgb)?;
    if skip_alpha {
        return Ok(color);
    }
    let mut next = StageColorContext {
        time: context.time,
        identity_light: context.identity_light,
        entity_rgba: context.entity_rgba,
        lighting: context.lighting,
        view_origin: context.view_origin,
        local_view_origin: context.local_view_origin,
        noise: context.noise,
        previous_color: color,
    };
    let _ = &mut next;
    evaluate_stage_alpha(stage, vertex, &next, source_rgb)
}

fn waveform_color(wave: &Waveform, time: f32, identity_light: f32, noise: &RendererNoise) -> Result<Vec4, ClientError> {
    let glow = if wave.kind == WaveKind::Noise {
        wave.base + noise.sample(0.0, 0.0, 0.0, (time + wave.phase) * wave.frequency) * wave.amplitude
    } else {
        evaluate_waveform(wave, time)? * identity_light
    };
    let color = f32::from(wave_byte(glow)?) / 255.0;
    Ok(qa_core::math::vec4(color, color, color, 1.0))
}

#[allow(clippy::too_many_lines)]
fn evaluate_stage_rgb_color(
    stage: &ShaderStage,
    vertex: &MaterialVertex,
    context: &StageColorContext,
    source_rgb: Option<SourceColorGen>,
) -> Result<Vec4, ClientError> {
    use super::material::ColorGen;
    let identity_light = context.identity_light;
    let entity = context.entity_rgba;
    let red: u8;
    let green: u8;
    let blue: u8;
    let mut alpha = normalized_byte(context.previous_color.w)?;
    if source_rgb == Some(SourceColorGen::Bad) {
        red = normalized_byte(identity_light)?;
        green = red;
        blue = red;
        alpha = red;
    } else {
        match &stage.rgb_gen {
            ColorGen::Identity => {
                red = 255;
                green = 255;
                blue = 255;
                alpha = 255;
            }
            ColorGen::IdentityLighting => {
                red = normalized_byte(identity_light)?;
                green = red;
                blue = red;
                alpha = red;
            }
            ColorGen::Entity => {
                red = entity[0];
                green = entity[1];
                blue = entity[2];
                alpha = entity[3];
            }
            ColorGen::OneMinusEntity => {
                red = 255 - entity[0];
                green = 255 - entity[1];
                blue = 255 - entity[2];
                alpha = 255 - entity[3];
            }
            ColorGen::Vertex => {
                red = byte(f32::from(vertex.color[0]) * identity_light)?;
                green = byte(f32::from(vertex.color[1]) * identity_light)?;
                blue = byte(f32::from(vertex.color[2]) * identity_light)?;
                alpha = vertex.color[3];
            }
            ColorGen::ExactVertex => {
                red = vertex.color[0];
                green = vertex.color[1];
                blue = vertex.color[2];
                alpha = vertex.color[3];
            }
            ColorGen::OneMinusVertex => {
                red = byte((255 - vertex.color[0]) as f32 * identity_light)?;
                green = byte((255 - vertex.color[1]) as f32 * identity_light)?;
                blue = byte((255 - vertex.color[2]) as f32 * identity_light)?;
            }
            ColorGen::Const(color) => {
                red = normalized_byte(color.x)?;
                green = normalized_byte(color.y)?;
                blue = normalized_byte(color.z)?;
                alpha = match &stage.alpha_gen {
                    super::material::AlphaGen::Const(alpha) => byte(alpha * 255.0)?,
                    _ => 0,
                };
            }
            ColorGen::Wave(wave) => {
                return waveform_color(wave, context.time, identity_light, context.noise);
            }
            ColorGen::LightingDiffuse => {
                let lighting = context
                    .lighting
                    .ok_or_else(|| ClientError::BadMaterial("lightingDiffuse requires entity lighting".to_string()))?;
                let color = diffuse_color(&vertex_normal(vertex), &lighting);
                red = color[0];
                green = color[1];
                blue = color[2];
                alpha = if dot3(vertex_normal(vertex), lighting.light_dir) <= 0.0 {
                    (lighting.ambient_light_int >> 24) as u8
                } else {
                    255
                };
            }
        }
    }
    Ok(qa_core::math::vec4(
        f32::from(byte(f32::from(red))?) / 255.0,
        f32::from(byte(f32::from(green))?) / 255.0,
        f32::from(byte(f32::from(blue))?) / 255.0,
        f32::from(byte(f32::from(alpha))?) / 255.0,
    ))
}

fn vertex_normal(vertex: &MaterialVertex) -> Vec3 {
    vertex.normal
}

#[allow(clippy::too_many_lines)]
fn evaluate_stage_alpha(
    stage: &ShaderStage,
    vertex: &MaterialVertex,
    context: &StageColorContext,
    source_rgb: Option<SourceColorGen>,
) -> Result<Vec4, ClientError> {
    use super::material::{AlphaGen, ColorGen};
    let identity_light = context.identity_light;
    let entity = context.entity_rgba;
    let previous = context.previous_color;
    let mut alpha = normalized_byte(previous.w)?;
    match &stage.alpha_gen {
        AlphaGen::Identity => {
            if source_rgb == Some(SourceColorGen::Bad)
                || !matches!(stage.rgb_gen, ColorGen::Identity)
                    && (!matches!(stage.rgb_gen, ColorGen::Vertex) || identity_light != 1.0)
            {
                alpha = 255;
            }
        }
        AlphaGen::Entity => {
            // ParseStage compares alphaGen to CGEN_IDENTITY (2), which is AGEN_ENTITY.
            if source_rgb.is_some()
                || !matches!(stage.rgb_gen, ColorGen::Identity) && !matches!(stage.rgb_gen, ColorGen::LightingDiffuse)
            {
                alpha = entity[3];
            }
        }
        AlphaGen::OneMinusEntity => alpha = 255 - entity[3],
        AlphaGen::Vertex => alpha = vertex.color[3],
        AlphaGen::OneMinusVertex => alpha = 255 - vertex.color[3],
        AlphaGen::Const(value) => alpha = byte(value * 255.0)?,
        AlphaGen::Wave(wave) => alpha = wave_byte(evaluate_waveform(wave, context.time)?)?,
        AlphaGen::Portal(range) => {
            let delta = sub3(vertex.position, context.view_origin);
            alpha = wave_byte(dot3(delta, delta).sqrt() / range)?;
        }
        AlphaGen::LightingSpecular => {
            alpha = specular_alpha(&vertex.position, &vertex.normal, &context.local_view_origin);
        }
    }
    Ok(qa_core::math::vec4(
        previous.x,
        previous.y,
        previous.z,
        f32::from(byte(f32::from(alpha))?) / 255.0,
    ))
}

/// Keep animation phase aligned with the 1024-entry table
/// (`animatedPictureIndex`).
pub fn animated_picture_index(time: f32, frequency: f32, count: usize) -> Result<usize, ClientError> {
    if count < 1 {
        return Err(ClientError::BadMaterial(
            "Animated picture needs registered frames".to_string(),
        ));
    }
    let scaled = (time * frequency) * 1024.0;
    if !scaled.is_finite() {
        return Err(ClientError::BadMaterial(
            "Animated picture index reaches undefined source float-to-int conversion".to_string(),
        ));
    }
    let value = scaled.trunc() as i64;
    if !(i64::from(i32::MIN)..=i64::from(i32::MAX)).contains(&value) {
        return Err(ClientError::BadMaterial(
            "Animated picture index reaches undefined source float-to-int conversion".to_string(),
        ));
    }
    #[allow(clippy::cast_possible_truncation)]
    let index = (value as i32) >> 10;
    Ok((index.max(0) as usize) % count)
}

/// `RB_CalcDiffuseColor` unsigned bytes with source truncation
/// (`diffuseColor`).
pub fn diffuse_color(normal: &Vec3, lighting: &EntityLighting) -> [u8; 3] {
    let incoming = dot3(*normal, lighting.light_dir);
    if incoming <= 0.0 {
        return [
            (lighting.ambient_light_int & 255) as u8,
            ((lighting.ambient_light_int >> 8) & 255) as u8,
            ((lighting.ambient_light_int >> 16) & 255) as u8,
        ];
    }
    let component = |ambient: f32, directed: f32| -> u8 {
        let value = (ambient + incoming * directed).trunc() as i32;
        value.min(255) as u8
    };
    [
        component(lighting.ambient_light.x, lighting.directed_light.x),
        component(lighting.ambient_light.y, lighting.directed_light.y),
        component(lighting.ambient_light.z, lighting.directed_light.z),
    ]
}

/// Stock specular helper with its fixed local light (`specularAlpha`).
pub fn specular_alpha(position: &Vec3, normal: &Vec3, viewer_origin: &Vec3) -> u8 {
    let light = normalize_fast3(sub3(vec3(-960.0, 1980.0, 96.0), *position));
    let d = dot3(*normal, light);
    let reflected = vec3(
        normal.x * 2.0 * d - light.x,
        normal.y * 2.0 * d - light.y,
        normal.z * 2.0 * d - light.z,
    );
    let viewer = sub3(*viewer_origin, *position);
    let mut l = dot3(reflected, viewer) * inverse_sqrt32(dot3(viewer, viewer));
    if l < 0.0 {
        return 0;
    }
    l *= l;
    l *= l;
    (l * 255.0).trunc().min(255.0) as u8
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::materials::material::{AlphaGen, ColorGen, ShaderMap, TexGen};
    use crate::materials::state::{AlphaTest, DepthTest, OPAQUE_BLEND};
    use qa_core::math::{vec2, vec4};

    fn stage() -> ShaderStage {
        ShaderStage {
            map: ShaderMap::None,
            blend: OPAQUE_BLEND,
            depth_func: DepthTest::LessEqual,
            depth_write: true,
            alpha_func: AlphaTest::None,
            detail: false,
            rgb_gen: ColorGen::Identity,
            alpha_gen: super::super::material::AlphaGen::Identity,
            tc_gen: TexGen::Texture,
            tc_mods: Vec::new(),
        }
    }

    fn vertex() -> MaterialVertex {
        MaterialVertex::new(
            vec3(0.0, 0.0, 0.0),
            vec3(0.0, 0.0, 1.0),
            vec2(0.0, 0.0),
            vec2(0.0, 0.0),
            [10, 20, 30, 40],
        )
    }

    fn context<'a>(noise: &'a RendererNoise) -> StageColorContext<'a> {
        StageColorContext {
            time: 0.0,
            identity_light: 1.0,
            entity_rgba: [1, 2, 3, 4],
            lighting: None,
            view_origin: vec3(0.0, 0.0, 0.0),
            local_view_origin: vec3(0.0, 0.0, 0.0),
            noise,
            previous_color: vec4(0.0, 0.0, 0.0, 0.0),
        }
    }

    #[test]
    fn identity_stage_is_white() {
        let noise = RendererNoise::new();
        let color = evaluate_stage_color(&stage(), &vertex(), &context(&noise), false, None).unwrap();
        assert_eq!(color, vec4(1.0, 1.0, 1.0, 1.0));
    }

    #[test]
    fn exact_vertex_passes_bytes_through() {
        let noise = RendererNoise::new();
        let color = evaluate_stage_color(&stage(), &vertex(), &context(&noise), false, None).unwrap();
        assert_eq!(color.w, 1.0);
        let mut stage = stage();
        stage.rgb_gen = ColorGen::ExactVertex;
        stage.alpha_gen = AlphaGen::Vertex;
        let color = evaluate_stage_color(&stage, &vertex(), &context(&noise), false, None).unwrap();
        assert!((color.x - 10.0 / 255.0).abs() < 1e-6);
        assert!((color.w - 40.0 / 255.0).abs() < 1e-6);
    }

    #[test]
    fn animated_picture_wraps() {
        assert_eq!(animated_picture_index(0.0, 1.0, 4).unwrap(), 0);
        assert_eq!(animated_picture_index(1.0, 1.0, 4).unwrap(), 1);
        assert!(animated_picture_index(0.0, 1.0, 0).is_err());
    }

    #[test]
    fn specular_backface_is_zero() {
        let alpha = specular_alpha(&vec3(0.0, 0.0, 0.0), &vec3(0.0, 0.0, -1.0), &vec3(0.0, 0.0, -100.0));
        assert_eq!(alpha, 0);
    }
}
