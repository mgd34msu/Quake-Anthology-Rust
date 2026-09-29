//! Q2 dynamic lights and light grid (`calc_dynamic_lights`).
//!
//! Donor provenance: `src/materials/q2-lighting.ts` and
//! `src/materials/q2-lightgrid.ts` (from q2repro).

use qa_core::math::{add3, scale3, vec3, Vec3};

use super::lighting::Q2LightStyle;

/// A Q2 fragment light sample (`DynamicLightSample`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DynamicLightSample {
    /// Origin.
    pub origin: Vec3,
    /// Radius.
    pub radius: f32,
    /// Color.
    pub color: Vec3,
    /// Intensity scale.
    pub scale: f32,
    /// Spot cone.
    pub cone: Option<SpotCone>,
}

/// A spot cone.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SpotCone {
    /// Direction.
    pub direction: Vec3,
    /// Cosine half-angle.
    pub cos_half_angle: f32,
}

/// Point-light falloff (`pointLightFalloff`).
#[must_use]
pub fn point_light_falloff(distance: f32, radius: f32) -> f32 {
    let effective = radius + 64.0;
    (effective - distance - 64.0).max(0.0) / effective
}

/// Spot-cone attenuation (`spotConeAttenuation`).
#[must_use]
pub fn spot_cone_attenuation(direction: Vec3, cone_direction: Vec3, cone_cos: f32) -> f32 {
    let magnitude = -(direction.x * cone_direction.x
        + direction.y * cone_direction.y
        + direction.z * cone_direction.z);
    if cone_cos >= 1.0 {
        return 0.0;
    }
    (1.0 - (1.0 - magnitude) * (1.0 / (1.0 - cone_cos))).max(0.0)
}

/// Dynamic-light contribution (`calcDynamicLightContribution`).
///
/// Point lights shift 16 units along the normal; negative red bypasses
/// Lambert.
#[must_use]
pub fn calc_dynamic_light_contribution(
    light: &DynamicLightSample,
    position: Vec3,
    normal: Vec3,
) -> Vec3 {
    let light_position = match light.cone {
        Some(_) => light.origin,
        None => vec3(
            light.origin.x + normal.x * 16.0,
            light.origin.y + normal.y * 16.0,
            light.origin.z + normal.z * 16.0,
        ),
    };
    let delta = vec3(
        light_position.x - position.x,
        light_position.y - position.y,
        light_position.z - position.z,
    );
    let distance = (delta.x * delta.x + delta.y * delta.y + delta.z * delta.z).sqrt();
    let falloff = point_light_falloff(distance, light.radius);
    let inverse = 1.0 / distance.max(1.0);
    let direction = vec3(delta.x * inverse, delta.y * inverse, delta.z * inverse);
    let lambert = if light.color.x < 0.0 {
        1.0
    } else {
        (direction.x * normal.x + direction.y * normal.y + direction.z * normal.z).max(0.0)
    };
    let mut scale = falloff * lambert * light.scale;
    if let Some(cone) = light.cone {
        scale *= spot_cone_attenuation(direction, cone.direction, cone.cos_half_angle);
    }
    vec3(
        light.color.x * scale,
        light.color.y * scale,
        light.color.z * scale,
    )
}

/// A light-grid octree child (`Q2Lightgrid` node reference).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LightgridChild {
    /// Occluded.
    Occluded,
    /// Leaf index.
    Leaf(usize),
    /// Node index.
    Node(usize),
}

/// A light-grid octree node.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LightgridNode {
    /// Split point.
    pub point: Vec3,
    /// Children.
    pub children: [LightgridChild; 8],
}

/// A light-grid leaf.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LightgridLeaf {
    /// Minimum corner.
    pub min: Vec3,
    /// Size.
    pub size: Vec3,
    /// Sample count.
    pub point_count: usize,
    /// First sample.
    pub first_sample: usize,
}

/// A light-grid sample.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LightgridSample {
    /// Style index (255 ends).
    pub style: u8,
    /// RGB.
    pub rgb: Vec3,
}

/// A Q2 light grid (`Q2Lightgrid`).
#[derive(Debug, Clone, PartialEq)]
pub struct Q2Lightgrid {
    /// Root child.
    pub root: LightgridChild,
    /// Nodes.
    pub nodes: Vec<LightgridNode>,
    /// Leaves.
    pub leaves: Vec<LightgridLeaf>,
    /// Samples.
    pub samples: Vec<LightgridSample>,
    /// Styles per point.
    pub style_count: usize,
    /// Grid minimum.
    pub min: Vec3,
    /// Grid scale.
    pub scale: Vec3,
}

/// Look up grid samples (`lookupQ2Lightgrid`).
#[must_use]
pub fn lookup_q2_lightgrid(grid: &Q2Lightgrid, point: Vec3) -> Option<&[LightgridSample]> {
    let mut child = grid.root;
    loop {
        match child {
            LightgridChild::Occluded => return None,
            LightgridChild::Leaf(index) => {
                let leaf = grid.leaves.get(index)?;
                let x = (point.x - leaf.min.x) as u32;
                let y = (point.y - leaf.min.y) as u32;
                let z = (point.z - leaf.min.z) as u32;
                let index = leaf.size.x as u32
                    .wrapping_mul((leaf.size.y as u32).wrapping_mul(z).wrapping_add(y))
                    .wrapping_add(x);
                if index as usize >= leaf.point_count {
                    return None;
                }
                let start = leaf.first_sample + index as usize * grid.style_count;
                return grid.samples.get(start..start + grid.style_count);
            }
            LightgridChild::Node(index) => {
                let node = grid.nodes.get(index)?;
                // Donor bit order: x->4, y->2, z->1.
                let child_index = (usize::from(point.x >= node.point.x) << 2)
                    | (usize::from(point.y >= node.point.y) << 1)
                    | usize::from(point.z >= node.point.z);
                child = *node.children.get(child_index)?;
            }
        }
    }
}

/// Q2 lighting adjustment (`Q2LightingAdjustment`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Q2LightingAdjustment {
    /// Additive bias.
    pub add: f32,
    /// Modulation.
    pub modulate: f32,
    /// Saturation.
    pub saturation: f32,
}

/// Adjust Q2 lighting (`adjustQ2Lighting`).
#[must_use]
pub fn adjust_q2_lighting(color: Vec3, adjustment: &Q2LightingAdjustment) -> Vec3 {
    let r = ((color.x + adjustment.add) * adjustment.modulate).max(0.0);
    let g = ((color.y + adjustment.add) * adjustment.modulate).max(0.0);
    let b = ((color.z + adjustment.add) * adjustment.modulate).max(0.0);
    if adjustment.saturation == 1.0 {
        return scale3(vec3(r, g, b), 1.0 / 255.0);
    }
    let luminance = r * 0.2126 + g * 0.7152 + b * 0.0722;
    scale3(
        vec3(
            luminance + (r - luminance) * adjustment.saturation,
            luminance + (g - luminance) * adjustment.saturation,
            luminance + (b - luminance) * adjustment.saturation,
        ),
        1.0 / 255.0,
    )
}

/// Grid point lighting (`q2LightGridPoint`).
///
/// Missing corners take the mean of valid corners before trilinear
/// interpolation.
#[must_use]
pub fn q2_light_grid_point(
    grid: &Q2Lightgrid,
    position: Vec3,
    styles: &[Q2LightStyle],
    adjustment: &Q2LightingAdjustment,
) -> Option<Vec3> {
    if grid.leaves.is_empty() {
        return None;
    }
    let point = vec3(
        (position.x - grid.min.x) * grid.scale.x,
        (position.y - grid.min.y) * grid.scale.y,
        (position.z - grid.min.z) * grid.scale.z,
    );
    let base = [
        point.x.trunc() as u32,
        point.y.trunc() as u32,
        point.z.trunc() as u32,
    ];
    let mut corners: Vec<Option<Vec3>> = Vec::with_capacity(8);
    let mut average = vec3(0.0, 0.0, 0.0);
    let mut count = 0u32;
    for index in 0..8 {
        let samples = lookup_q2_lightgrid(
            grid,
            vec3(
                f32::from((base[0] + (index & 1)) as u16),
                f32::from((base[1] + ((index >> 1) & 1)) as u16),
                f32::from((base[2] + ((index >> 2) & 1)) as u16),
            ),
        );
        let mut color = vec3(0.0, 0.0, 0.0);
        let mut valid = false;
        if let Some(samples) = samples {
            for sample in samples {
                if sample.style == 255 {
                    break;
                }
                let Some(style) = styles.get(usize::from(sample.style)) else {
                    break;
                };
                color = add3(color, scale3(sample.rgb, style.rgb.x));
                valid = true;
            }
        }
        if valid {
            count += 1;
            average = add3(average, color);
            corners.push(Some(color));
        } else {
            corners.push(None);
        }
    }
    if count == 0 {
        return None;
    }
    average = scale3(average, 1.0 / count as f32);
    let values: Vec<Vec3> = corners.into_iter().map(|corner| corner.unwrap_or(average)).collect();
    let interpolate = |a: Vec3, b: Vec3, fraction: f32| {
        add3(scale3(a, 1.0 - fraction), scale3(b, fraction))
    };
    let fx = point.x - base[0] as f32;
    let fy = point.y - base[1] as f32;
    let fz = point.z - base[2] as f32;
    let bottom = interpolate(
        interpolate(values[0], values[1], fx),
        interpolate(values[2], values[3], fx),
        fy,
    );
    let top = interpolate(
        interpolate(values[4], values[5], fx),
        interpolate(values[6], values[7], fx),
        fy,
    );
    Some(adjust_q2_lighting(interpolate(bottom, top, fz), adjustment))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn falloff_reaches_zero_at_radius() {
        assert_eq!(point_light_falloff(300.0, 200.0), 0.0);
        assert!(point_light_falloff(0.0, 200.0) > 0.0);
    }

    #[test]
    fn cone_closed_at_one() {
        assert_eq!(
            spot_cone_attenuation(vec3(0.0, 0.0, 1.0), vec3(0.0, 0.0, 1.0), 1.0),
            0.0
        );
    }

    #[test]
    fn contribution_falls_off() {
        let light = DynamicLightSample {
            origin: vec3(0.0, 0.0, 100.0),
            radius: 200.0,
            color: vec3(1.0, 1.0, 1.0),
            scale: 1.0,
            cone: None,
        };
        let near = calc_dynamic_light_contribution(&light, vec3(0.0, 0.0, 0.0), vec3(0.0, 0.0, 1.0));
        let far =
            calc_dynamic_light_contribution(&light, vec3(0.0, 0.0, -500.0), vec3(0.0, 0.0, 1.0));
        assert!(near.x > far.x);
    }

    #[test]
    fn empty_grid_returns_none() {
        let grid = Q2Lightgrid {
            root: LightgridChild::Occluded,
            nodes: Vec::new(),
            leaves: Vec::new(),
            samples: Vec::new(),
            style_count: 1,
            min: vec3(0.0, 0.0, 0.0),
            scale: vec3(1.0, 1.0, 1.0),
        };
        assert!(
            q2_light_grid_point(
                &grid,
                vec3(0.0, 0.0, 0.0),
                &[],
                &Q2LightingAdjustment {
                    add: 0.0,
                    modulate: 1.0,
                    saturation: 1.0
                },
            )
            .is_none()
        );
    }
}
