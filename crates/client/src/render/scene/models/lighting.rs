//! Alias lighting rules (donor `src/render/scene/models/lighting.ts`).
//!
//! Quake II `gl_mesh.c` shell colors, alias light modulation, model-space
//! shadow projection, and per-light shadow fractions.

use qa_core::math::{vec3, Vec3};

use crate::render::types::{Q2FragmentLight, Q2ModelShadowLight};

/// Flag bits selecting a powerup shell tint.
pub const Q2_SHELL_MASK: u32 = 1024 | 2048 | 4096 | 65536 | 131072;

/// Shell tint for an entity's flags, or `None` when unpowered.
#[must_use]
pub fn q2_shell_color(flags: u32) -> Option<Vec3> {
    if flags & Q2_SHELL_MASK == 0 {
        return None;
    }
    let red = flags & 1024 != 0;
    let green = flags & 2048 != 0;
    let blue = flags & 4096 != 0;
    let double = flags & 65536 != 0;
    let half = flags & 131072 != 0;
    if red && green && blue {
        return Some(vec3(1.0, 1.0, 1.0));
    }
    if red {
        return Some(vec3(1.0, 0.0, if blue || double { 1.0 } else { 0.0 }));
    }
    if blue {
        return Some(vec3(0.0, if double { 1.0 } else { 0.0 }, 1.0));
    }
    if double {
        return Some(vec3(0.9, 0.7, 0.0));
    }
    Some(vec3(
        if half { 0.56 } else { 0.0 },
        if green {
            1.0
        } else if half {
            0.59
        } else {
            0.0
        },
        if half { 0.45 } else { 0.0 },
    ))
}

/// Modulate a sampled alias light by render flags, time, and infrared mode.
#[must_use]
pub fn q2_alias_light(flags: u32, sampled: Vec3, time_seconds: f64, monochrome: bool, infrared: bool) -> Vec3 {
    let shell = q2_shell_color(flags);
    let mut light = shell.unwrap_or(if flags & 8 != 0 { vec3(1.0, 1.0, 1.0) } else { sampled });
    if shell.is_none() && flags & 8 == 0 && monochrome {
        let value = light.x.max(light.y).max(light.z);
        light = vec3(value, value, value);
    }
    if flags & 1 != 0 && light.x <= 0.1 && light.y <= 0.1 && light.z <= 0.1 {
        light = vec3(0.1, 0.1, 0.1);
    }
    if flags & 512 != 0 {
        let pulse = 0.1 * (time_seconds * 7.0).sin() as f32;
        light = vec3(
            (light.x * 0.8).max(light.x + pulse),
            (light.y * 0.8).max(light.y + pulse),
            (light.z * 0.8).max(light.z + pulse),
        );
    }
    if infrared && flags & 32768 != 0 {
        vec3(1.0, 0.0, 0.0)
    } else {
        light
    }
}

/// Q2 model-space alias shadow projection onto the sampled world floor.
#[must_use]
pub fn alias_shadow_point(point: Vec3, shade_vector: Vec3, entity_height: f32, floor_height: f32) -> Vec3 {
    let height = entity_height - floor_height;
    vec3(
        point.x - shade_vector.x * (point.z + height),
        point.y - shade_vector.y * (point.z + height),
        -height + 1.0,
    )
}

/// GLQuake `GL_DrawAliasShadow` projection with model-space float stores.
#[must_use]
pub fn q1_alias_shadow_point(point: Vec3, shade_vector: Vec3, entity_height: f32, floor_height: f32) -> Vec3 {
    let height = entity_height - floor_height;
    let elevation = point.z + height;
    vec3(
        point.x - shade_vector.x * elevation,
        point.y - shade_vector.y * elevation,
        -height + 1.0,
    )
}

/// Q1 alias shadow direction for a model yaw angle in radians.
#[must_use]
pub fn q1_alias_shadow_direction(yaw: f32) -> Vec3 {
    let x = (-yaw).cos();
    let y = (-yaw).sin();
    let length = (x * x + y * y + 1.0).sqrt();
    let inverse = 1.0 / length;
    vec3(x * inverse, y * inverse, inverse)
}

/// Per-light shadow fractions: remove only each occluded light's share.
#[must_use]
pub fn alias_shadow_light_fractions(
    origin: Vec3,
    shade: Vec3,
    lights: &[Q2FragmentLight],
    modulate: f32,
    monochrome: bool,
) -> Vec<Q2ModelShadowLight> {
    let mut result = Vec::new();
    let share = |contribution: f32, channel: f32| {
        if channel > 0.0 && contribution > 0.0 {
            (contribution / channel).min(1.0)
        } else {
            0.0
        }
    };
    for light in lights {
        if light.shadow == crate::render::types::Q2ShadowProjection::None {
            continue;
        }
        let distance = ((origin.x - light.origin.x).powi(2)
            + (origin.y - light.origin.y).powi(2)
            + (origin.z - light.origin.z).powi(2))
        .sqrt();
        let amount = (light.radius - distance) / 256.0 * modulate;
        if amount <= 0.0 {
            continue;
        }
        let (mut r, mut g, mut b) = (amount * light.color.x, amount * light.color.y, amount * light.color.z);
        if monochrome {
            r = r.max(g).max(b);
            g = r;
            b = r;
        }
        let fraction = vec3(share(r, shade.x), share(g, shade.y), share(b, shade.z));
        if fraction.x <= 0.0 && fraction.y <= 0.0 && fraction.z <= 0.0 {
            continue;
        }
        result.push(Q2ModelShadowLight {
            origin: light.origin,
            radius: light.radius,
            fraction,
            shadow: light.shadow,
        });
    }
    result
}

/// Alias shadedots and skinned mesh lighting multiply the shade by at most two.
#[must_use]
pub fn alias_shade_divisor(shade: Vec3) -> f32 {
    1.0f32.max(shade.x.max(shade.y).max(shade.z) * 2.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::types::Q2ShadowProjection;

    fn shadowed_light() -> Q2FragmentLight {
        Q2FragmentLight {
            origin: vec3(0.0, 0.0, 10.0),
            radius: 300.0,
            color: vec3(1.0, 1.0, 1.0),
            scale: 1.0,
            cone: None,
            shadow: Q2ShadowProjection::Point {
                atlas_rect: qa_core::math::vec4(0.0, 0.0, 1.0, 1.0),
            },
        }
    }

    #[test]
    fn shell_colors_match_flags() {
        assert_eq!(q2_shell_color(0), None);
        assert_eq!(q2_shell_color(1024), Some(vec3(1.0, 0.0, 0.0)));
        assert_eq!(q2_shell_color(2048), Some(vec3(0.0, 1.0, 0.0)));
        assert_eq!(q2_shell_color(4096), Some(vec3(0.0, 0.0, 1.0)));
        assert_eq!(q2_shell_color(1024 | 2048 | 4096), Some(vec3(1.0, 1.0, 1.0)));
        assert_eq!(q2_shell_color(65536), Some(vec3(0.9, 0.7, 0.0)));
    }

    #[test]
    fn fullbright_flag_returns_white() {
        let light = q2_alias_light(8, vec3(0.2, 0.3, 0.4), 0.0, false, false);
        assert_eq!(light, vec3(1.0, 1.0, 1.0));
    }

    #[test]
    fn minlight_raises_dark_samples() {
        let light = q2_alias_light(1, vec3(0.0, 0.0, 0.0), 0.0, false, false);
        assert_eq!(light, vec3(0.1, 0.1, 0.1));
    }

    #[test]
    fn infrared_overrides_with_red() {
        let light = q2_alias_light(32768, vec3(0.5, 0.5, 0.5), 0.0, false, true);
        assert_eq!(light, vec3(1.0, 0.0, 0.0));
    }

    #[test]
    fn shadow_point_projects_to_floor() {
        let point = alias_shadow_point(vec3(0.0, 0.0, 8.0), vec3(0.0, 0.0, 1.0), 24.0, 0.0);
        assert_eq!(point.z, -23.0);
    }

    #[test]
    fn q1_shadow_direction_is_unit() {
        let direction = q1_alias_shadow_direction(1.0);
        let length = (direction.x * direction.x + direction.y * direction.y + direction.z * direction.z).sqrt();
        assert!((length - 1.0).abs() < 1e-6);
    }

    #[test]
    fn unshadowed_lights_produce_no_fractions() {
        let mut light = shadowed_light();
        light.shadow = Q2ShadowProjection::None;
        let fractions = alias_shadow_light_fractions(vec3(0.0, 0.0, 0.0), vec3(1.0, 1.0, 1.0), &[light], 1.0, false);
        assert!(fractions.is_empty());
    }

    #[test]
    fn shadowed_light_produces_fraction() {
        let fractions = alias_shadow_light_fractions(
            vec3(0.0, 0.0, 0.0),
            vec3(1.0, 1.0, 1.0),
            &[shadowed_light()],
            1.0,
            false,
        );
        assert_eq!(fractions.len(), 1);
        assert!(fractions[0].fraction.x > 0.0);
    }

    #[test]
    fn shade_divisor_clamps_at_one() {
        assert_eq!(alias_shade_divisor(vec3(0.1, 0.1, 0.1)), 1.0);
        assert_eq!(alias_shade_divisor(vec3(1.0, 0.5, 0.25)), 2.0);
    }
}
