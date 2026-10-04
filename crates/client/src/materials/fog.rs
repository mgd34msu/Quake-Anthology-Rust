//! BSP fog volumes, legacy Q1/Q2 fog (`tr_bsp.c`, `gl_fog.c`).
//!
//! Donor provenance: `src/materials/fog.ts` and
//! `src/materials/legacy-fog.ts`.

use qa_content::bsp::parse_q1_entities;
use qa_core::math::{dot3, scale3, sub3, vec3, Bounds, Plane, Vec2, Vec3, Vec4};

use super::material::ShaderDefinition;
use super::state::{AlphaTest, Blend, BlendFactor, CullFace, DepthTest, RenderState};
use crate::ClientError;

/// Fog brush map inputs (`FogBrushMap`).
#[derive(Debug, Clone, PartialEq)]
pub struct FogBrushMap {
    /// Fog records (brush, visible side).
    pub fogs: Vec<(usize, i32)>,
    /// Brushes (first side, side count).
    pub brushes: Vec<(usize, usize)>,
    /// Brush sides (plane index).
    pub brush_sides: Vec<usize>,
    /// Planes.
    pub planes: Vec<Plane>,
}

/// A fog volume (`FogVolume`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FogVolume {
    /// Bounds.
    pub bounds: Bounds,
    /// Inward-facing surface plane.
    pub surface: Option<Plane>,
    /// Fog color.
    pub color: Vec4,
    /// Texture-coordinate scale.
    pub tc_scale: f32,
}

/// Fog adjustment (`FogAdjustment`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FogAdjustment {
    /// None.
    None,
    /// RGB.
    Rgb,
    /// Alpha.
    Alpha,
    /// RGBA.
    Rgba,
}

/// Fog pass (`FogPass`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FogPass {
    /// None.
    None,
    /// Equal depth.
    Equal,
    /// Less-equal depth.
    LessEqual,
}

fn at<T: Copy>(items: &[T], index: usize) -> Result<T, ClientError> {
    items
        .get(index)
        .copied()
        .ok_or_else(|| ClientError::BadMaterial(format!("fog index {index} outside {}", items.len())))
}

/// Prepare a fog volume (`prepareFogVolume`).
pub fn prepare_fog_volume(
    map: &FogBrushMap,
    index: usize,
    color: Vec3,
    depth_for_opaque: f32,
) -> Result<FogVolume, ClientError> {
    let (brush_index, visible_side) = at(&map.fogs, index)?;
    let (first_side, side_count) = at(&map.brushes, brush_index)?;
    if side_count < 6 {
        return Err(ClientError::BadMaterial(format!(
            "fog brush {brush_index} has fewer than six axial sides"
        )));
    }
    let plane = |side: usize| -> Result<Plane, ClientError> {
        let side_index = at(&map.brush_sides, first_side + side)?;
        at(&map.planes, side_index)
    };
    let bounds = Bounds {
        min: vec3(-plane(0)?.distance, -plane(2)?.distance, -plane(4)?.distance),
        max: vec3(plane(1)?.distance, plane(3)?.distance, plane(5)?.distance),
    };
    let surface = if visible_side < 0 {
        None
    } else {
        let outside = plane(visible_side as usize)?;
        Some(Plane {
            normal: sub3(vec3(0.0, 0.0, 0.0), outside.normal),
            distance: -outside.distance,
        })
    };
    let byte = |value: f32| ((value * 255.0).trunc() as i32 & 255) as u8;
    Ok(FogVolume {
        bounds,
        surface,
        color: qa_core::math::vec4(
            f32::from(byte(color.x)) / 255.0,
            f32::from(byte(color.y)) / 255.0,
            f32::from(byte(color.z)) / 255.0,
            1.0,
        ),
        tc_scale: 1.0 / (depth_for_opaque.max(1.0) * 8.0),
    })
}

/// Fog coordinate mapper (`fogCoordinates`).
pub struct FogCoordinates {
    distance_vector: Vec3,
    offset: f32,
    surface: Option<Plane>,
    eye_depth: f32,
}

impl FogCoordinates {
    /// Build the coordinate mapper.
    #[must_use]
    pub fn new(fog: &FogVolume, origin: &Vec3, forward: &Vec3) -> Self {
        let distance_vector = scale3(*forward, fog.tc_scale);
        let offset = dot3(sub3(vec3(0.0, 0.0, 0.0), *origin), *forward) * fog.tc_scale + 1.0 / 512.0;
        let eye_depth = match fog.surface {
            None => 1.0,
            Some(surface) => dot3(*origin, surface.normal) - surface.distance,
        };
        Self {
            distance_vector,
            offset,
            surface: fog.surface,
            eye_depth,
        }
    }

    /// Map a position to fog coordinates.
    #[must_use]
    pub fn coordinates(&self, position: &Vec3) -> Vec2 {
        let x = dot3(*position, self.distance_vector) + self.offset;
        let Some(surface) = self.surface else {
            return qa_core::math::vec2(x, 31.0 / 32.0);
        };
        let depth = dot3(*position, surface.normal) - surface.distance;
        let y = if self.eye_depth < 0.0 {
            if depth < 1.0 {
                1.0 / 32.0
            } else {
                1.0 / 32.0 + (30.0 / 32.0 * depth) / (depth - self.eye_depth)
            }
        } else if depth < 0.0 {
            1.0 / 32.0
        } else {
            31.0 / 32.0
        };
        qa_core::math::vec2(x, y)
    }
}

fn fog_table(index: usize) -> f32 {
    (index as f32 / 255.0).sqrt()
}

/// Quantize density through the 256-entry table (`fogFactor`).
#[must_use]
pub fn fog_factor(s: f32, t: f32) -> f32 {
    let mut distance = s - 1.0 / 512.0;
    if distance < 0.0 || t < 1.0 / 32.0 {
        return 0.0;
    }
    if t < 31.0 / 32.0 {
        distance *= (t - 1.0 / 32.0) / (30.0 / 32.0);
    }
    distance *= 8.0;
    if distance > 1.0 {
        distance = 1.0;
    }
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    fog_table((distance * 255.0) as usize)
}

/// A fog texture level (`ImageLevel` subset).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FogTexture {
    /// Width (256).
    pub width: u32,
    /// Height (32).
    pub height: u32,
    /// RGBA pixels.
    pub pixels: Vec<u8>,
}

/// Build the fog lookup texture (`createFogTexture`).
#[allow(clippy::cast_possible_truncation)]
pub fn create_fog_texture() -> FogTexture {
    let mut pixels = vec![0u8; 256 * 32 * 4];
    for y in 0..32 {
        for x in 0..256 {
            let density = fog_factor((x as f32 + 0.5) / 256.0, (y as f32 + 0.5) / 32.0);
            let offset = (y * 256 + x) * 4;
            pixels[offset] = 255;
            pixels[offset + 1] = 255;
            pixels[offset + 2] = 255;
            pixels[offset + 3] = (255.0 * density) as u8;
        }
    }
    FogTexture {
        width: 256,
        height: 32,
        pixels,
    }
}

fn is_blended(blend: &Blend) -> bool {
    blend.source != BlendFactor::One || blend.destination != BlendFactor::Zero
}

/// Compute the fog adjustment for a stage pair (`fogAdjustment`).
#[must_use]
pub fn fog_adjustment(stage: &Blend, first: &Blend) -> FogAdjustment {
    if !is_blended(stage) || !is_blended(first) {
        return FogAdjustment::None;
    }
    match (stage.source, stage.destination) {
        (BlendFactor::One, BlendFactor::One) | (BlendFactor::Zero, BlendFactor::OneMinusSrcColor) => FogAdjustment::Rgb,
        (BlendFactor::SrcAlpha, BlendFactor::OneMinusSrcAlpha) => FogAdjustment::Alpha,
        (BlendFactor::One, BlendFactor::OneMinusSrcAlpha) => FogAdjustment::Rgba,
        _ => FogAdjustment::None,
    }
}

/// Attenuate a color by fog (`attenuateFogColor`).
#[must_use]
pub fn attenuate_fog_color(color: Vec4, adjustment: FogAdjustment, coordinates: Vec2) -> Vec4 {
    if adjustment == FogAdjustment::None {
        return color;
    }
    let attenuation = 1.0 - fog_factor(coordinates.x, coordinates.y);
    let modulate = |component: f32| ((component * 255.0).round() * attenuation).trunc() / 255.0;
    let rgb = matches!(adjustment, FogAdjustment::Rgb | FogAdjustment::Rgba);
    let alpha = matches!(adjustment, FogAdjustment::Alpha | FogAdjustment::Rgba);
    qa_core::math::vec4(
        if rgb { modulate(color.x) } else { color.x },
        if rgb { modulate(color.y) } else { color.y },
        if rgb { modulate(color.z) } else { color.z },
        if alpha { modulate(color.w) } else { color.w },
    )
}

/// Choose a sort key (`shaderSort`).
#[must_use]
pub fn shader_sort(shader: Option<&ShaderDefinition>) -> i32 {
    let Some(shader) = shader else {
        return 3;
    };
    if shader.stages.is_empty() {
        return 7;
    }
    if shader.sky.is_some() {
        return 2;
    }
    if let Some(sort) = shader.sort {
        if sort != 0.0 {
            return sort as i32;
        }
    }
    if shader.polygon_offset {
        return 4;
    }
    let first = &shader.stages[0].stage;
    if is_blended(&first.blend) {
        if first.depth_write {
            5
        } else {
            9
        }
    } else {
        3
    }
}

/// Choose a fog pass (`shaderFogPass`).
#[must_use]
pub fn shader_fog_pass(shader: Option<&ShaderDefinition>) -> FogPass {
    if shader_sort(shader) <= 3 {
        return FogPass::Equal;
    }
    match shader {
        Some(shader) if shader.surface_parms.iter().any(|parm| parm == "fog") => FogPass::LessEqual,
        _ => FogPass::None,
    }
}

/// Build fog-pass state (`fogPassState`).
#[must_use]
pub fn fog_pass_state(pass: FogPass, cull: CullFace) -> RenderState {
    debug_assert!(pass != FogPass::None);
    RenderState {
        blend: Blend {
            source: BlendFactor::SrcAlpha,
            destination: BlendFactor::OneMinusSrcAlpha,
        },
        depth_test: match pass {
            FogPass::Equal => DepthTest::Equal,
            _ => DepthTest::LessEqual,
        },
        depth_write: false,
        alpha_test: AlphaTest::None,
        cull,
        depth_range: [0.0, 1.0],
        polygon_offset: None,
    }
}

/// Q1 fog value (`Q1Fog`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Q1Fog {
    /// Density.
    pub density: f32,
    /// Color.
    pub color: Vec3,
}

/// Q1 fog transition (`Q1FogTransition`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Q1FogTransition {
    /// Previous fog.
    pub previous: Q1Fog,
    /// Target fog.
    pub target: Q1Fog,
    /// Start time.
    pub start: f32,
    /// Duration.
    pub duration: f32,
}

fn clamp01(value: f32) -> f32 {
    value.clamp(0.0, 1.0)
}

fn mix(a: Vec3, b: Vec3, fraction: f32) -> Vec3 {
    vec3(
        a.x + (b.x - a.x) * fraction,
        a.y + (b.y - a.y) * fraction,
        a.z + (b.z - a.z) * fraction,
    )
}

/// Q1 fog state with fades (`Q1FogState`).
#[derive(Debug, Clone, PartialEq)]
pub struct Q1FogState {
    previous: Q1Fog,
    target: Q1Fog,
    start: f32,
    duration: f32,
}

impl Q1FogState {
    /// Default state.
    #[must_use]
    pub fn new() -> Self {
        let fog = Q1Fog {
            density: 0.0,
            color: vec3(0.3, 0.3, 0.3),
        };
        Self {
            previous: fog,
            target: fog,
            start: 0.0,
            duration: 0.0,
        }
    }

    /// Update the target.
    pub fn update(&mut self, value: Q1Fog, time: f32, duration: f32) {
        self.previous = self.sample(time);
        self.target = Q1Fog {
            density: value.density.max(0.0),
            color: vec3(clamp01(value.color.x), clamp01(value.color.y), clamp01(value.color.z)),
        };
        self.start = time;
        self.duration = duration.max(0.0);
    }

    /// Capture the transition.
    #[must_use]
    pub const fn capture(&self) -> Q1FogTransition {
        Q1FogTransition {
            previous: self.previous,
            target: self.target,
            start: self.start,
            duration: self.duration,
        }
    }

    /// Install a transition.
    pub fn install(&mut self, value: Q1FogTransition) {
        self.previous = value.previous;
        self.target = value.target;
        self.start = value.start;
        self.duration = value.duration;
    }

    /// Sample at a time.
    #[must_use]
    pub fn sample(&self, time: f32) -> Q1Fog {
        let fraction = if self.duration == 0.0 {
            1.0
        } else {
            clamp01((time - self.start) / self.duration)
        };
        Q1Fog {
            density: self.previous.density + (self.target.density - self.previous.density) * fraction,
            color: mix(self.previous.color, self.target.color, fraction),
        }
    }
}

impl Default for Q1FogState {
    fn default() -> Self {
        Self::new()
    }
}

/// Parse worldspawn fog (`q1WorldFog`).
pub fn q1_world_fog(entities: &str) -> Result<Q1FogTransition, ClientError> {
    let parsed = parse_q1_entities(entities, "<fog>").map_err(|error| ClientError::BadMaterial(format!("{error}")))?;
    let mut state = Q1FogState::new();
    if let Some(world) = parsed.first() {
        for (key, value) in &world.properties {
            if key.trim_start_matches('_').trim_end() != "fog" {
                continue;
            }
            let values: Vec<f32> = value
                .split_whitespace()
                .map(|token| token.parse::<f32>().unwrap_or(f32::NAN))
                .collect();
            let mut defaults = [0.0, 0.3, 0.3, 0.3];
            for (index, slot) in defaults.iter_mut().enumerate() {
                match values.get(index) {
                    Some(value) if value.is_finite() => *slot = *value,
                    _ => break,
                }
            }
            state.update(
                Q1Fog {
                    density: defaults[0],
                    color: vec3(defaults[1], defaults[2], defaults[3]),
                },
                0.0,
                0.0,
            );
        }
    }
    Ok(state.capture())
}

/// Global fog amount (`globalFogAmount`).
#[must_use]
pub fn global_fog_amount(density_scaled: f32, frag_depth: f32) -> f32 {
    let d = density_scaled * frag_depth;
    1.0 - (-(d * d)).exp()
}

/// Height fog fraction (`heightFogFraction`, keeps the double subtraction).
#[must_use]
pub fn height_fog_fraction(world_z: f32, start: f32, end: f32) -> f32 {
    if end == start {
        return 0.0;
    }
    clamp01((world_z - start - start) / (end - start))
}

/// Height fog direction Z (`heightFogDirZ`).
#[must_use]
pub fn height_fog_dir_z(direction_z: f32) -> f32 {
    let sign = if direction_z > 0.0 {
        1.0
    } else if direction_z < 0.0 {
        -1.0
    } else {
        0.0
    };
    direction_z + 0.00001 * (1.0 - sign * sign)
}

/// Height fog extinction (`heightFogExtinction`).
#[must_use]
pub fn height_fog_extinction(view_z: f32, world_z: f32, start: f32, falloff: f32, direction_z: f32) -> f32 {
    let density =
        ((-falloff * (view_z - start)).exp() - (-falloff * (world_z - start)).exp()) / (falloff * direction_z);
    1.0 - clamp01((-density).exp())
}

/// Height fog amount (`heightFogAmount`).
#[must_use]
pub fn height_fog_amount(density: f32, depth: f32, extinction: f32) -> f32 {
    (1.0 - (-(density * depth)).exp()) * extinction
}

/// Q2 height fog endpoint.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Q2HeightEndpoint {
    /// Color.
    pub color: Vec3,
    /// Distance.
    pub distance: f32,
}

/// Q2 height fog.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Q2HeightFog {
    /// Density.
    pub density: f32,
    /// Start endpoint.
    pub start: Q2HeightEndpoint,
    /// End endpoint.
    pub end: Q2HeightEndpoint,
    /// Falloff.
    pub falloff: f32,
}

/// Q2 scene fog (`SceneFog` Q2 subset).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Q2SceneFog {
    /// Fog color.
    pub color: Vec3,
    /// Density.
    pub density: f32,
    /// Height fog.
    pub height: Q2HeightFog,
    /// Sky factor.
    pub sky_factor: f32,
}

/// Shared global (non-height) fog mix with wire density scaled by 64.
///
/// Q1 fog is exactly this step; Q2 layers height fog and sky blend on top,
/// keeping the subset relation explicit at both call sites.
fn global_fog_mix(color: Vec3, fog_color: Vec3, density: f32, depth: f32) -> Vec3 {
    mix(color, fog_color, global_fog_amount(density / 64.0, depth))
}

/// Q2 fog color (`q2FogColor`).
#[must_use]
pub fn q2_fog_color(color: Vec3, fog: &Q2SceneFog, point: Vec3, view: Vec3, depth: f32, sky: bool) -> Vec3 {
    let mut result = global_fog_mix(color, fog.color, fog.density, depth);
    if fog.height.density > 0.0 {
        let dx = point.x - view.x;
        let dy = point.y - view.y;
        let dz = point.z - view.z;
        let distance = (dx * dx + dy * dy + dz * dz).sqrt();
        let direction_z = height_fog_dir_z(if distance == 0.0 {
            0.0
        } else {
            (point.z - view.z) / distance
        });
        let extinction = height_fog_extinction(
            view.z,
            point.z,
            fog.height.start.distance,
            fog.height.falloff,
            direction_z,
        );
        let amount = height_fog_amount(fog.height.density, depth, extinction);
        let height_color = mix(
            fog.height.start.color,
            fog.height.end.color,
            height_fog_fraction(point.z, fog.height.start.distance, fog.height.end.distance),
        );
        result = mix(
            result,
            vec3(
                height_color.x * extinction,
                height_color.y * extinction,
                height_color.z * extinction,
            ),
            amount,
        );
    }
    if sky {
        result = mix(result, fog.color, fog.sky_factor);
    }
    result
}

/// Q1 fog color (`q1FogColor`, wire density divided by 64).
#[must_use]
pub fn q1_fog_color(color: Vec3, fog: &Q1Fog, depth: f32) -> Vec3 {
    global_fog_mix(color, fog.color, fog.density, depth)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fog_factor_edges() {
        assert_eq!(fog_factor(0.0, 0.5), 0.0);
        assert_eq!(fog_factor(0.5, 0.0), 0.0);
        assert!(fog_factor(2.0, 1.0) > 0.99);
    }

    #[test]
    fn fog_texture_dimensions() {
        let texture = create_fog_texture();
        assert_eq!((texture.width, texture.height), (256, 32));
        assert_eq!(texture.pixels.len(), 256 * 32 * 4);
    }

    #[test]
    fn q1_fog_fades() {
        let mut state = Q1FogState::new();
        state.update(
            Q1Fog {
                density: 8.0,
                color: vec3(1.0, 0.0, 0.0),
            },
            0.0,
            10.0,
        );
        assert_eq!(state.sample(0.0).density, 0.0);
        assert_eq!(state.sample(10.0).density, 8.0);
    }

    #[test]
    fn world_fog_reads_worldspawn() {
        let transition = q1_world_fog("{\n\"_fog\" \"4 0.1 0.2 0.3\"\n}\n").unwrap();
        assert_eq!(transition.target.density, 4.0);
        assert_eq!(transition.target.color, vec3(0.1, 0.2, 0.3));
    }

    #[test]
    fn height_fraction_keeps_double_subtraction() {
        assert_eq!(height_fog_fraction(10.0, 2.0, 12.0), (10.0 - 4.0) / 10.0);
        assert_eq!(height_fog_fraction(0.0, 5.0, 5.0), 0.0);
    }

    #[test]
    fn q1_fog_is_q2_global_subset() {
        let color = vec3(0.2, 0.4, 0.8);
        let fog_color = vec3(0.5, 0.5, 0.5);
        let q1 = q1_fog_color(
            color,
            &Q1Fog {
                density: 8.0,
                color: fog_color,
            },
            100.0,
        );
        let q2 = q2_fog_color(
            color,
            &Q2SceneFog {
                color: fog_color,
                density: 8.0,
                height: Q2HeightFog {
                    density: 0.0,
                    start: Q2HeightEndpoint {
                        color: fog_color,
                        distance: 0.0,
                    },
                    end: Q2HeightEndpoint {
                        color: fog_color,
                        distance: 1.0,
                    },
                    falloff: 1.0,
                },
                sky_factor: 0.0,
            },
            vec3(0.0, 0.0, 0.0),
            vec3(0.0, 0.0, 0.0),
            100.0,
            false,
        );
        assert_eq!(q1, q2);
        assert_eq!(global_fog_mix(color, fog_color, 0.0, 100.0), color);
    }

    #[test]
    fn sort_prefers_sky() {
        assert_eq!(shader_sort(None), 3);
        assert_eq!(shader_fog_pass(None), FogPass::Equal);
    }
}
