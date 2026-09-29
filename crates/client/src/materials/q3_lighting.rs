//! Q3 light grid and entity lighting (`tr_bsp.c`, `tr_light.c`).
//!
//! Donor provenance: `src/materials/q3-lighting.ts`.

use qa_content::md3::renderer_sine;
use qa_core::math::{
    add3, dot3, length3, normalize3, normalize3_or_zero, scale3, sub3, vec3, Axis, Bounds, Vec3,
};

/// `RF_MINLIGHT` (unconditional in the source).
pub const RF_MINLIGHT: u32 = 1;
/// `RF_LIGHTING_ORIGIN`.
pub const RF_LIGHTING_ORIGIN: u32 = 128;
/// `RF_FIRST_PERSON` (diagnostics only).
pub const RF_FIRST_PERSON: u32 = 4;

/// A light-grid sample (`Q3LightGridPoint` subset).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LightGridSample {
    /// Ambient RGB bytes.
    pub ambient: [u8; 3],
    /// Directed RGB bytes.
    pub directed: [u8; 3],
    /// Latitude/longitude bytes.
    pub lat_long: [u8; 2],
}

/// Light grid (`LightGrid`).
#[derive(Debug, Clone, PartialEq)]
pub struct LightGrid {
    /// Grid origin.
    pub origin: Vec3,
    /// Cell size.
    pub size: Vec3,
    /// Inverse cell size.
    pub inverse_size: Vec3,
    /// Sample counts (X varies fastest).
    pub bounds: [usize; 3],
    /// Samples.
    pub samples: Vec<LightGridSample>,
}

/// Lighting scales (`LightingScales`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LightingScales {
    /// Ambient scale.
    pub ambient_scale: f32,
    /// Directed scale.
    pub directed_scale: f32,
}

/// A lighting sample (`LightingSample`, byte units).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LightingSample {
    /// Ambient light.
    pub ambient_light: Vec3,
    /// Directed light.
    pub directed_light: Vec3,
    /// Light direction (world space).
    pub light_dir: Vec3,
}

/// A lighting entity (`LightingEntity`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LightingEntity {
    /// Origin.
    pub origin: Vec3,
    /// Lighting origin.
    pub lighting_origin: Vec3,
    /// Axis.
    pub axis: Axis,
    /// Render flags.
    pub render_flags: u32,
}

/// A dynamic light (`DynamicLight`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DynamicLight {
    /// Origin.
    pub origin: Vec3,
    /// Radius.
    pub radius: f32,
    /// Color.
    pub color: Vec3,
    /// Additive blending.
    pub additive: bool,
}

/// Entity lighting state (`EntityLightingState`).
#[derive(Debug, Clone, PartialEq)]
pub struct EntityLightingState {
    /// Ambient scale.
    pub ambient_scale: f32,
    /// Directed scale.
    pub directed_scale: f32,
    /// Light grid.
    pub grid: Option<LightGrid>,
    /// No world model.
    pub no_world_model: bool,
    /// Identity light.
    pub identity_light: f32,
    /// Identity light byte cap.
    pub identity_light_byte: f32,
    /// Sun direction fallback.
    pub sun_direction: Vec3,
    /// Dynamic lights.
    pub dynamic_lights: Vec<DynamicLight>,
}

/// Computed entity lighting (`EntityLighting`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EntityLighting {
    /// Ambient light (bytes).
    pub ambient_light: Vec3,
    /// Directed light (bytes).
    pub directed_light: Vec3,
    /// Light direction (entity-local).
    pub light_dir: Vec3,
    /// Unsigned RGBA packet.
    pub ambient_light_int: u32,
}

fn shifted_color(color: [u8; 3], shift: u32) -> [u8; 3] {
    let r = u32::from(color[0]) << shift;
    let g = u32::from(color[1]) << shift;
    let b = u32::from(color[2]) << shift;
    if (r | g | b) <= 255 {
        return [r as u8, g as u8, b as u8];
    }
    let maximum = r.max(g).max(b);
    [
        ((r * 255 / maximum) & 255) as u8,
        ((g * 255 / maximum) & 255) as u8,
        ((b * 255 / maximum) & 255) as u8,
    ]
}

fn grid_float(token: &str) -> f32 {
    let value = token.trim().to_ascii_lowercase();
    if let Some(rest) = value.strip_prefix("0x").or_else(|| {
        value
            .strip_prefix("+0x")
            .or_else(|| value.strip_prefix("-0x"))
    }) {
        let negative = value.starts_with('-');
        let (mantissa, exponent) = match rest.find('p') {
            Some(index) => (&rest[..index], rest[index + 1..].parse::<i32>().unwrap_or(0)),
            None => (rest, 0),
        };
        let (digits, fractional) = match mantissa.find('.') {
            Some(dot) => (
                format!("{}{}", &mantissa[..dot], &mantissa[dot + 1..]),
                mantissa.len() - dot - 1,
            ),
            None => (mantissa.to_string(), 0),
        };
        let parsed = i64::from_str_radix(digits.as_str(), 16).unwrap_or(0);
        let magnitude = parsed as f64 * 2f64.powi(exponent - fractional as i32 * 4);
        return (if negative { -magnitude } else { magnitude }) as f32;
    }
    value.parse::<f64>().unwrap_or(f64::NAN) as f32
}

/// Parse a `gridsize` value (`worldGridSize`, partial values keep defaults).
pub fn world_grid_size(worldspawn: &[(String, String)]) -> Vec3 {
    let mut size = vec3(64.0, 64.0, 128.0);
    for (key, value) in worldspawn {
        if key.to_ascii_lowercase() != "gridsize" {
            continue;
        }
        let mut rest = value.as_str();
        for axis in ["x", "y", "z"] {
            let token = take_grid_token(rest);
            let Some((token, remaining)) = token else {
                break;
            };
            let parsed = grid_float(token);
            match axis {
                "x" => size.x = parsed,
                "y" => size.y = parsed,
                _ => size.z = parsed,
            }
            rest = remaining;
        }
    }
    size
}

fn take_grid_token(rest: &str) -> Option<(&str, &str)> {
    let trimmed = rest.trim_start();
    let offset = rest.len() - trimmed.len();
    if trimmed.is_empty() {
        return None;
    }
    let bytes = trimmed.as_bytes();
    let mut end = 0usize;
    if bytes[0] == b'+' || bytes[0] == b'-' {
        end += 1;
    }
    let lower = trimmed.to_ascii_lowercase();
    if lower[end..].starts_with("0x") {
        end += 2;
        while end < bytes.len() && (bytes[end].is_ascii_hexdigit() || bytes[end] == b'.') {
            end += 1;
        }
        if bytes.get(end) == Some(&b'p') || bytes.get(end) == Some(&b'P') {
            end += 1;
            if bytes.get(end) == Some(&b'+') || bytes.get(end) == Some(&b'-') {
                end += 1;
            }
            while end < bytes.len() && bytes[end].is_ascii_digit() {
                end += 1;
            }
        }
        return Some((&trimmed[..end], &rest[offset + end..]));
    }
    if lower[end..].starts_with("inf") {
        return Some((&trimmed[..end + 3], &rest[offset + end + 3..]));
    }
    if lower[end..].starts_with("nan") {
        return Some((&trimmed[..end + 3], &rest[offset + end + 3..]));
    }
    let mut saw_digit = false;
    while end < bytes.len() && bytes[end].is_ascii_digit() {
        saw_digit = true;
        end += 1;
    }
    if bytes.get(end) == Some(&b'.') {
        end += 1;
        while end < bytes.len() && bytes[end].is_ascii_digit() {
            saw_digit = true;
            end += 1;
        }
    }
    if !saw_digit {
        return None;
    }
    if bytes.get(end) == Some(&b'e') || bytes.get(end) == Some(&b'E') {
        let mut probe = end + 1;
        if bytes.get(probe) == Some(&b'+') || bytes.get(probe) == Some(&b'-') {
            probe += 1;
        }
        if bytes.get(probe).is_some_and(|byte| byte.is_ascii_digit()) {
            end = probe + 1;
            while end < bytes.len() && bytes[end].is_ascii_digit() {
                end += 1;
            }
        }
    }
    Some((&trimmed[..end], &rest[offset + end..]))
}

fn grid_axis(minimum: f32, maximum: f32, size: f32) -> (f32, i64) {
    let origin = size * (minimum / size).ceil();
    let end = size * (maximum / size).floor();
    ((origin), ((end - origin) / size + 1.0).trunc() as i64)
}

/// Lighting map inputs for grid loading (`LightingMap`).
#[derive(Debug, Clone, PartialEq)]
pub struct LightingMap {
    /// Grid samples.
    pub light_grid: Vec<LightGridSample>,
    /// World bounds (model zero).
    pub world_bounds: Bounds,
    /// Entity records (worldspawn first).
    pub entity_records: Vec<Vec<(String, String)>>,
}

/// Load the light grid (`prepareLightGrid`).
pub fn prepare_light_grid(
    map: &LightingMap,
    map_overbright_bits: i32,
    overbright_bits: i32,
) -> Result<(Option<LightGrid>, Vec<String>), crate::ClientError> {
    let shift = map_overbright_bits - overbright_bits;
    if shift < 0 || shift > 15 {
        return Err(crate::ClientError::BadMaterial(
            "Light-grid overbright shift must be an integer in 0..15; other values have undefined source C shifts or overflow".to_string(),
        ));
    }
    let worldspawn = map.entity_records.first().cloned().unwrap_or_default();
    let size = world_grid_size(&worldspawn);
    if ![size.x, size.y, size.z]
        .iter()
        .all(|value| value.is_finite() && *value > 0.0)
    {
        return Ok((
            None,
            vec!["Invalid worldspawn gridsize: dimensions must be finite and positive".to_string()],
        ));
    }
    let (ox, nx) = grid_axis(map.world_bounds.min.x, map.world_bounds.max.x, size.x);
    let (oy, ny) = grid_axis(map.world_bounds.min.y, map.world_bounds.max.y, size.y);
    let (oz, nz) = grid_axis(map.world_bounds.min.z, map.world_bounds.max.z, size.z);
    if [nx, ny, nz].iter().any(|count| *count <= 0) {
        return Ok((
            None,
            vec!["Invalid light-grid bounds: no finite positive sample layout".to_string()],
        ));
    }
    let count = nx * ny * nz;
    if map.light_grid.len() as i64 != count {
        return Ok((
            None,
            vec![format!(
                "Light grid mismatch: expected {count} samples, found {}",
                map.light_grid.len()
            )],
        ));
    }
    Ok((
        Some(LightGrid {
            origin: vec3(ox, oy, oz),
            inverse_size: vec3(1.0 / size.x, 1.0 / size.y, 1.0 / size.z),
            size,
            bounds: [nx as usize, ny as usize, nz as usize],
            samples: map
                .light_grid
                .iter()
                .map(|point| LightGridSample {
                    ambient: shifted_color(point.ambient, shift as u32),
                    directed: shifted_color(point.directed, shift as u32),
                    lat_long: point.lat_long,
                })
                .collect(),
        }),
        Vec::new(),
    ))
}

fn sample_axis(coordinate: f32, inverse_size: f32, count: usize) -> (usize, f32) {
    let v = coordinate * inverse_size;
    let position = v.floor() as i64;
    (
        position.clamp(0, count as i64 - 1) as usize,
        v - position as f32,
    )
}

/// Sample the grid (`lightForPoint`).
pub fn light_for_point(
    grid: Option<&LightGrid>,
    point: &Vec3,
    scales: &LightingScales,
) -> Result<Option<LightingSample>, crate::ClientError> {
    let Some(grid) = grid else {
        return Ok(None);
    };
    if ![point.x, point.y, point.z].iter().all(|value| value.is_finite()) {
        return Err(crate::ClientError::BadMaterial(
            "Light sample origin must be finite".to_string(),
        ));
    }
    let relative = sub3(*point, grid.origin);
    let (px, fx) = sample_axis(relative.x, grid.inverse_size.x, grid.bounds[0]);
    let (py, fy) = sample_axis(relative.y, grid.inverse_size.y, grid.bounds[1]);
    let (pz, fz) = sample_axis(relative.z, grid.inverse_size.z, grid.bounds[2]);
    let mut ambient = vec3(0.0, 0.0, 0.0);
    let mut directed = vec3(0.0, 0.0, 0.0);
    let mut direction = vec3(0.0, 0.0, 0.0);
    let mut total_factor = 0.0f32;
    for corner in 0..8 {
        let mut factor = if corner & 1 != 0 { fx } else { 1.0 - fx };
        factor *= if corner & 2 != 0 { fy } else { 1.0 - fy };
        factor *= if corner & 4 != 0 { fz } else { 1.0 - fz };
        let sx = px + (corner & 1);
        let sy = py + ((corner >> 1) & 1);
        let sz = pz + ((corner >> 2) & 1);
        let mut index = sx + grid.bounds[0] * (sy + grid.bounds[1] * sz);
        if index >= grid.samples.len() {
            index = sx.min(grid.bounds[0] - 1)
                + grid.bounds[0]
                    * (sy.min(grid.bounds[1] - 1) + grid.bounds[1] * sz.min(grid.bounds[2] - 1));
        }
        let data = grid.samples.get(index).ok_or_else(|| {
            crate::ClientError::BadMaterial(
                "Prepared light-grid sample layout is inconsistent".to_string(),
            )
        })?;
        if data.ambient.iter().map(|value| u32::from(*value)).sum::<u32>() == 0 {
            continue;
        }
        total_factor += factor;
        ambient = add3(
            ambient,
            scale3(
                vec3(
                    f32::from(data.ambient[0]),
                    f32::from(data.ambient[1]),
                    f32::from(data.ambient[2]),
                ),
                factor,
            ),
        );
        directed = add3(
            directed,
            scale3(
                vec3(
                    f32::from(data.directed[0]),
                    f32::from(data.directed[1]),
                    f32::from(data.directed[2]),
                ),
                factor,
            ),
        );
        let latitude = i32::from(data.lat_long[1]) * 4;
        let longitude = i32::from(data.lat_long[0]) * 4;
        let normal = vec3(
            renderer_sine(latitude + 256) * renderer_sine(longitude),
            renderer_sine(latitude) * renderer_sine(longitude),
            renderer_sine(longitude + 256),
        );
        direction = add3(direction, scale3(normal, factor));
    }
    if total_factor > 0.0 && total_factor < 0.99 {
        let inverse = 1.0 / total_factor;
        ambient = scale3(ambient, inverse);
        directed = scale3(directed, inverse);
    }
    Ok(Some(LightingSample {
        ambient_light: scale3(ambient, scales.ambient_scale),
        directed_light: scale3(directed, scales.directed_scale),
        light_dir: normalize3_or_zero(direction),
    }))
}

/// Entity-lighting diagnostics sink (`EntityLightingDiagnostics`).
pub trait EntityLightingDiagnostics {
    /// Whether diagnostics are enabled.
    fn enabled(&self) -> bool;
    /// Print diagnostics text.
    fn print(&mut self, text: &str);
}

/// Set up entity lighting (`setupEntityLighting`).
pub fn setup_entity_lighting(
    entity: &LightingEntity,
    state: &EntityLightingState,
    diagnostics: Option<&mut dyn EntityLightingDiagnostics>,
) -> Result<EntityLighting, crate::ClientError> {
    let origin = if entity.render_flags & RF_LIGHTING_ORIGIN != 0 {
        entity.lighting_origin
    } else {
        entity.origin
    };
    let fallback = state.identity_light * 150.0;
    let grid_sample = if state.no_world_model {
        None
    } else {
        light_for_point(
            state.grid.as_ref(),
            &origin,
            &LightingScales {
                ambient_scale: state.ambient_scale,
                directed_scale: state.directed_scale,
            },
        )?
    };
    let sample = grid_sample.unwrap_or(LightingSample {
        ambient_light: vec3(fallback, fallback, fallback),
        directed_light: vec3(fallback, fallback, fallback),
        light_dir: state.sun_direction,
    });
    // The original has `if (1)`: every entity gets minlight.
    let minimum = state.identity_light * 32.0;
    let mut ambient = add3(sample.ambient_light, vec3(minimum, minimum, minimum));
    let mut directed = sample.directed_light;
    let mut direction = scale3(sample.light_dir, length3(directed));
    for light in &state.dynamic_lights {
        let relative = sub3(light.origin, origin);
        let distance = length3(relative).max(16.0);
        let power = 16.0 * (light.radius * light.radius);
        let attenuation = power / (distance * distance);
        directed = add3(directed, scale3(light.color, attenuation));
        direction = add3(direction, scale3(normalize3(relative), attenuation));
    }
    ambient = vec3(
        ambient.x.min(state.identity_light_byte),
        ambient.y.min(state.identity_light_byte),
        ambient.z.min(state.identity_light_byte),
    );
    if let Some(diagnostics) = diagnostics {
        if diagnostics.enabled() && entity.render_flags & RF_FIRST_PERSON != 0 {
            let mut max1 = ambient.x.trunc() as i32;
            let mut max2 = directed.x.trunc() as i32;
            if ambient.y > max1 as f32 {
                max1 = ambient.y.trunc() as i32;
            } else if ambient.z > max1 as f32 {
                max1 = ambient.z.trunc() as i32;
            }
            if directed.y > max2 as f32 {
                max2 = directed.y.trunc() as i32;
            } else if directed.z > max2 as f32 {
                max2 = directed.z.trunc() as i32;
            }
            diagnostics.print(&format!("amb:{max1}  dir:{max2}\n"));
        }
    }
    let ambient_light_int = ((ambient.x.trunc() as i32 & 255) as u32)
        | (((ambient.y.trunc() as i32 & 255) as u32) << 8)
        | (((ambient.z.trunc() as i32 & 255) as u32) << 16)
        | 0xff00_0000;
    let direction = normalize3(direction);
    Ok(EntityLighting {
        ambient_light: ambient,
        directed_light: directed,
        light_dir: vec3(
            dot3(direction, entity.axis[0]),
            dot3(direction, entity.axis[1]),
            dot3(direction, entity.axis[2]),
        ),
        ambient_light_int,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entity() -> LightingEntity {
        LightingEntity {
            origin: vec3(0.0, 0.0, 0.0),
            lighting_origin: vec3(0.0, 0.0, 0.0),
            axis: [
                vec3(1.0, 0.0, 0.0),
                vec3(0.0, 1.0, 0.0),
                vec3(0.0, 0.0, 1.0),
            ],
            render_flags: 0,
        }
    }

    #[test]
    fn fallback_lighting_uses_identity() {
        let lighting = setup_entity_lighting(
            &entity(),
            &EntityLightingState {
                ambient_scale: 1.0,
                directed_scale: 1.0,
                grid: None,
                no_world_model: true,
                identity_light: 1.0,
                identity_light_byte: 255.0,
                sun_direction: vec3(0.0, 0.0, 1.0),
                dynamic_lights: Vec::new(),
            },
            None,
        )
        .unwrap();
        assert_eq!(lighting.ambient_light, vec3(182.0, 182.0, 182.0));
        assert_eq!(lighting.ambient_light_int, 0xffb6_b6b6);
    }

    #[test]
    fn grid_mismatch_reports_diagnostic() {
        let map = LightingMap {
            light_grid: Vec::new(),
            world_bounds: qa_core::math::Bounds {
                min: vec3(0.0, 0.0, 0.0),
                max: vec3(64.0, 64.0, 128.0),
            },
            entity_records: vec![Vec::new()],
        };
        let (grid, diagnostics) = prepare_light_grid(&map, 0, 0).unwrap();
        assert!(grid.is_none());
        assert_eq!(diagnostics.len(), 1);
    }

    #[test]
    fn gridsize_partial_keeps_defaults() {
        let size = world_grid_size(&[("gridsize".to_string(), "32".to_string())]);
        assert_eq!(size, vec3(32.0, 64.0, 128.0));
    }

    #[test]
    fn bad_shift_is_an_error() {
        let map = LightingMap {
            light_grid: Vec::new(),
            world_bounds: qa_core::math::Bounds {
                min: vec3(0.0, 0.0, 0.0),
                max: vec3(64.0, 64.0, 128.0),
            },
            entity_records: Vec::new(),
        };
        assert!(prepare_light_grid(&map, 0, 1).is_err());
    }
}
