//! Q2 fragment lighting for the CPU backend.
//!
//! Donor provenance: `src/render/cpu/lighting.ts` in full — the Q2 fragment
//! and shadow equations ported from `quake-2-re-ts` `ref_gl/gl_shader.ts`.
//! Copyright (C) Id Software and contributors.

use qa_core::math::{transform_vec4, vec3, vec4, Vec2, Vec3, Vec4};

use super::super::types::{
    BatchLighting, Q2FragmentLight, Q2LightPass, Q2ModelFragmentLight, Q2ModelShadowLight, Q2ShadowAtlas,
    Q2ShadowProjection,
};
use crate::materials::q2_lighting::{calc_dynamic_light_contribution, DynamicLightSample, SpotCone};

use super::textures::SharedDepth;
use super::triangle_kernel::Sample;

/// A clipped batch vertex with world-space lighting attributes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CpuVertex {
    /// Clip-space position.
    pub position: Vec4,
    /// World-space position.
    pub world_position: Vec3,
    /// World-space normal.
    pub world_normal: Vec3,
    /// Vertex color.
    pub color: Vec4,
    /// Primary texture coordinates.
    pub tex_coord: Vec2,
    /// Secondary texture coordinates.
    pub tex_coord2: Vec2,
}

/// Batch lighting parameters with their shadow depth.
#[derive(Debug, Clone)]
pub struct CpuLighting<'a> {
    /// Batch lighting parameters.
    pub parameters: &'a BatchLighting,
    /// Shadow depth atlas, present for shadowed passes.
    pub depth: Option<SharedDepth>,
}

/// Per-triangle lighting with interpolated corners.
#[derive(Debug, Clone)]
pub struct CpuTriangleLighting<'a> {
    /// Batch lighting parameters.
    pub parameters: &'a BatchLighting,
    /// Shadow depth atlas, present for shadowed passes.
    pub depth: Option<SharedDepth>,
    /// World-space corner positions.
    pub positions: [Vec3; 3],
    /// World-space corner normals.
    pub normals: [Vec3; 3],
}

/// World-space attributes for one batch vertex index.
pub fn world_attributes(lighting: &BatchLighting, index: usize) -> (Vec3, Vec3) {
    match lighting {
        BatchLighting::Vertex => (vec3(0.0, 0.0, 0.0), vec3(0.0, 0.0, 0.0)),
        BatchLighting::Q2World {
            world_positions,
            normals,
            ..
        } => (
            *world_positions
                .get(index)
                .expect("CPU fragment lighting arrays do not cover the vertex"),
            *normals
                .get(index)
                .expect("CPU fragment lighting arrays do not cover the vertex"),
        ),
        BatchLighting::Q2ModelShadow { world_positions, .. } => (
            *world_positions
                .get(index)
                .expect("CPU fragment lighting arrays do not cover the vertex"),
            vec3(0.0, 0.0, 0.0),
        ),
    }
}

fn bound(value: f32, low: f32, high: f32) -> f32 {
    value.clamp(low, high)
}

fn depth_sample(image: &SharedDepth, u: f32, v: f32) -> f32 {
    let x = bound(u * image.width as f32, 0.0, image.width as f32 - 1.0) as usize;
    let y = bound(v * image.height as f32, 0.0, image.height as f32 - 1.0) as usize;
    *image
        .pixels
        .get(y * image.width as usize + x)
        .expect("CPU shadow atlas sample escaped its image")
}

fn dynamic_sample(light: &Q2FragmentLight) -> DynamicLightSample {
    DynamicLightSample {
        origin: light.origin,
        radius: light.radius,
        color: light.color,
        scale: light.scale,
        cone: light.cone.map(|cone| SpotCone {
            direction: cone.direction,
            cos_half_angle: cone.cos_half_angle,
        }),
    }
}

/// Shadow visibility for one fragment; cone normalized depth bias and cube
/// world-unit slope bias follow the donor GLSL. Uses 4-tap PCF.
pub fn shadow_visibility(
    position: Vec3,
    origin: Vec3,
    radius: f32,
    shadow: &Q2ShadowProjection,
    atlas: &Q2ShadowAtlas,
    depth: &SharedDepth,
    model: bool,
) -> f32 {
    if matches!(shadow, Q2ShadowProjection::None) {
        return 1.0;
    }
    let texel = atlas.texel_size;
    // Each arm resolves the tap window plus its stored-depth comparison.
    enum Probe {
        Cone {
            base_x: f32,
            base_y: f32,
            z: f32,
            model: bool,
        },
        Point {
            base_x: f32,
            base_y: f32,
            axial: f32,
            pa: f32,
            pb: f32,
            bias: f32,
        },
    }
    let (probe, low_x, low_y, high_x, high_y) = match shadow {
        Q2ShadowProjection::None => unreachable!("unshadowed lights return above"),
        Q2ShadowProjection::Cone { matrix, atlas_rect } => {
            let clip = transform_vec4(*matrix, vec4(position.x, position.y, position.z, 1.0));
            if clip.w <= 0.0 {
                return 1.0;
            }
            let (x, y, z) = (clip.x / clip.w, clip.y / clip.w, clip.z / clip.w);
            if x < 0.0 || x > 1.0 || y < 0.0 || y > 1.0 || z > 1.0 {
                return 1.0;
            }
            (
                Probe::Cone {
                    base_x: x * atlas_rect.z + atlas_rect.x,
                    base_y: y * atlas_rect.w + atlas_rect.y,
                    z,
                    model,
                },
                atlas_rect.x + texel,
                atlas_rect.y + texel,
                atlas_rect.x + atlas_rect.z - texel,
                atlas_rect.y + atlas_rect.w - texel,
            )
        }
        Q2ShadowProjection::Point { atlas_rect } => {
            let (x, y, z) = (position.x - origin.x, position.y - origin.y, position.z - origin.z);
            let (ax, ay, az) = (x.abs(), y.abs(), z.abs());
            let (face, right, up, axial) = if ax >= ay && ax >= az {
                (if x >= 0.0 { 0 } else { 1 }, if x >= 0.0 { -y } else { y }, z, ax)
            } else if ay >= az {
                (if y >= 0.0 { 2 } else { 3 }, if y >= 0.0 { x } else { -x }, z, ay)
            } else {
                (if z >= 0.0 { 4 } else { 5 }, if z >= 0.0 { y } else { -y }, x, az)
            };
            let near = atlas.near_plane;
            if axial <= near {
                return 1.0;
            }
            let far = radius.max(near * 2.0);
            let pa = (far + near) / (near - far);
            let pb = 2.0 * far * near / (near - far);
            let cell_width = atlas_rect.z / 3.0;
            let cell_height = atlas_rect.w / 2.0;
            let cell_x = atlas_rect.x + (face % 3) as f32 * cell_width;
            let cell_y = atlas_rect.y + (face / 3) as f32 * cell_height;
            let bias = (if model { 5.0 } else { 1.0 })
                + axial * (2.0 / (cell_width / texel)) * (if model { 6.0 } else { 2.0 });
            (
                Probe::Point {
                    base_x: cell_x + (right / axial * 0.5 + 0.5) * cell_width,
                    base_y: cell_y + (up / axial * 0.5 + 0.5) * cell_height,
                    axial,
                    pa,
                    pb,
                    bias,
                },
                cell_x + texel,
                cell_y + texel,
                cell_x + cell_width - texel,
                cell_y + cell_height - texel,
            )
        }
    };
    let visible = |stored: f32| match probe {
        Probe::Cone { z, model, .. } => z - (if model { 0.0025 } else { 0.0005 }) <= stored,
        Probe::Point {
            axial, pa, pb, bias, ..
        } => axial - bias <= pb / (2.0 * stored - 1.0 + pa),
    };
    let (base_x, base_y) = match probe {
        Probe::Cone { base_x, base_y, .. } | Probe::Point { base_x, base_y, .. } => (base_x, base_y),
    };
    let mut lit = 0u32;
    for tap_y in 0..2 {
        for tap_x in 0..2 {
            let sample = depth_sample(
                depth,
                bound(base_x + (tap_x as f32 - 0.5) * texel, low_x, high_x),
                bound(base_y + (tap_y as f32 - 0.5) * texel, low_y, high_y),
            );
            lit += u32::from(visible(sample));
        }
    }
    lit as f32 * 0.25
}

fn alias_shade(
    position: Vec3,
    vertex: Vec4,
    scale: f32,
    lights: &[Q2ModelShadowLight],
    atlas: &Q2ShadowAtlas,
    depth: &SharedDepth,
) -> Vec3 {
    let (mut keep_r, mut keep_g, mut keep_b) = (1.0, 1.0, 1.0);
    for light in lights {
        if light.fraction.x == 0.0 && light.fraction.y == 0.0 && light.fraction.z == 0.0 {
            continue;
        }
        let occluded = 1.0 - shadow_visibility(position, light.origin, light.radius, &light.shadow, atlas, depth, true);
        keep_r -= light.fraction.x * occluded;
        keep_g -= light.fraction.y * occluded;
        keep_b -= light.fraction.z * occluded;
    }
    vec3(
        (vertex.x * scale * keep_r.max(0.0)).min(1.0),
        (vertex.y * scale * keep_g.max(0.0)).min(1.0),
        (vertex.z * scale * keep_b.max(0.0)).min(1.0),
    )
}

fn model_shadow_light(light: &Q2ModelFragmentLight) -> Q2ModelShadowLight {
    Q2ModelShadowLight {
        origin: light.light.origin,
        radius: light.light.radius,
        fraction: light.fraction,
        shadow: light.light.shadow,
    }
}

/// Shade one Q2 fragment: vertex modulation, dynamic lights with shadow
/// visibility, and the model/lightmap pass combines.
pub fn shade_q2_fragment(lighting: &CpuLighting, position: Vec3, normal: Vec3, vertex: Vec4, texel: &Sample) -> Sample {
    match lighting.parameters {
        BatchLighting::Vertex => Sample {
            r: vertex.x * texel.r,
            g: vertex.y * texel.g,
            b: vertex.z * texel.b,
            a: vertex.w * texel.a,
        },
        BatchLighting::Q2World { atlas, pass, .. } => {
            let is_model = matches!(pass, Q2LightPass::Model { .. });
            let is_texture = matches!(pass, Q2LightPass::Texture { .. });
            let is_lightmap = matches!(pass, Q2LightPass::Lightmap { .. });
            let is_material_lightmap = matches!(pass, Q2LightPass::MaterialLightmap { .. });
            let mut r = if is_model {
                vertex.x
            } else if !is_texture {
                texel.r
            } else {
                texel.r * vertex.x
            };
            let mut g = if is_model {
                vertex.y
            } else if !is_texture {
                texel.g
            } else {
                texel.g * vertex.y
            };
            let mut b = if is_model {
                vertex.z
            } else if !is_texture {
                texel.b
            } else {
                texel.b * vertex.z
            };
            if let Q2LightPass::Model { lights, shade_scale } = pass {
                if let Some(shade_scale) = shade_scale {
                    let (Some(atlas), Some(depth)) = (atlas, &lighting.depth) else {
                        panic!("CPU model shadows require their depth atlas");
                    };
                    let shadow_lights: Vec<Q2ModelShadowLight> = lights.iter().map(model_shadow_light).collect();
                    let shade = alias_shade(position, vertex, *shade_scale, &shadow_lights, atlas, depth);
                    r = shade.x;
                    g = shade.y;
                    b = shade.z;
                }
                let mut rgb = [r, g, b];
                add_model_lights(lighting, position, normal, lights, atlas, &mut rgb);
                [r, g, b] = rgb;
                r *= texel.r;
                g *= texel.g;
                b *= texel.b;
            } else {
                let lights = match pass {
                    Q2LightPass::Lightmap { lights }
                    | Q2LightPass::Texture { lights }
                    | Q2LightPass::MaterialLightmap { lights } => lights,
                    Q2LightPass::Model { .. } => unreachable!("model pass handled above"),
                };
                let mut rgb = [r, g, b];
                add_fragment_lights(lighting, position, normal, lights, atlas, &mut rgb);
                [r, g, b] = rgb;
            }
            if is_material_lightmap {
                r *= vertex.x;
                g *= vertex.y;
                b *= vertex.z;
            }
            Sample {
                r,
                g,
                b,
                a: if is_lightmap { 1.0 } else { texel.a * vertex.w },
            }
        }
        BatchLighting::Q2ModelShadow {
            lights,
            shade_scale,
            atlas,
            ..
        } => {
            let Some(depth) = &lighting.depth else {
                panic!("CPU model shadows require their depth atlas");
            };
            let shade = alias_shade(position, vertex, *shade_scale, lights, atlas, depth);
            Sample {
                r: texel.r * shade.x,
                g: texel.g * shade.y,
                b: texel.b * shade.z,
                a: texel.a * vertex.w,
            }
        }
    }
}

fn add_fragment_lights(
    lighting: &CpuLighting,
    position: Vec3,
    normal: Vec3,
    lights: &[Q2FragmentLight],
    atlas: &Option<Q2ShadowAtlas>,
    rgb: &mut [f32; 3],
) {
    for light in lights {
        if light.scale == 0.0 || (light.color.x == 0.0 && light.color.y == 0.0 && light.color.z == 0.0) {
            continue;
        }
        let contribution = calc_dynamic_light_contribution(&dynamic_sample(light), position, normal);
        let mut visibility = 1.0;
        if !matches!(light.shadow, Q2ShadowProjection::None) {
            let (Some(atlas), Some(depth)) = (atlas, &lighting.depth) else {
                panic!("CPU shadow light requires its depth atlas");
            };
            visibility = shadow_visibility(position, light.origin, light.radius, &light.shadow, atlas, depth, false);
        }
        rgb[0] += contribution.x * visibility;
        rgb[1] += contribution.y * visibility;
        rgb[2] += contribution.z * visibility;
    }
}

fn add_model_lights(
    lighting: &CpuLighting,
    position: Vec3,
    normal: Vec3,
    lights: &[Q2ModelFragmentLight],
    atlas: &Option<Q2ShadowAtlas>,
    rgb: &mut [f32; 3],
) {
    for model in lights {
        let light = &model.light;
        if light.scale == 0.0 || (light.color.x == 0.0 && light.color.y == 0.0 && light.color.z == 0.0) {
            continue;
        }
        let contribution = calc_dynamic_light_contribution(&dynamic_sample(light), position, normal);
        let mut visibility = 1.0;
        if !matches!(light.shadow, Q2ShadowProjection::None) {
            let (Some(atlas), Some(depth)) = (atlas, &lighting.depth) else {
                panic!("CPU shadow light requires its depth atlas");
            };
            visibility = shadow_visibility(position, light.origin, light.radius, &light.shadow, atlas, depth, false);
        }
        rgb[0] += contribution.x * visibility;
        rgb[1] += contribution.y * visibility;
        rgb[2] += contribution.z * visibility;
    }
}

/// Barycentric world-space interpolation.
#[must_use]
pub fn interpolate_world(a: Vec3, b: Vec3, c: Vec3, wa: f32, wb: f32, wc: f32) -> Vec3 {
    vec3(
        a.x * wa + b.x * wb + c.x * wc,
        a.y * wa + b.y * wb + c.y * wc,
        a.z * wa + b.z * wb + c.z * wc,
    )
}

#[cfg(test)]
mod tests {
    use qa_core::math::vec4;

    use super::*;

    #[test]
    fn world_attributes_cover_vertex_and_world_batches() {
        let (position, normal) = world_attributes(&BatchLighting::Vertex, 0);
        assert_eq!((position, normal), (vec3(0.0, 0.0, 0.0), vec3(0.0, 0.0, 0.0)));
        let lighting = BatchLighting::Q2World {
            world_positions: vec![vec3(1.0, 2.0, 3.0)],
            normals: vec![vec3(0.0, 0.0, 1.0)],
            atlas: None,
            pass: Q2LightPass::Texture { lights: Vec::new() },
        };
        let (position, normal) = world_attributes(&lighting, 0);
        assert_eq!(position, vec3(1.0, 2.0, 3.0));
        assert_eq!(normal, vec3(0.0, 0.0, 1.0));
    }

    #[test]
    fn unshadowed_visibility_is_full() {
        let atlas = Q2ShadowAtlas {
            image: crate::render::types::RendererImage {
                owner: crate::render::types::ResourceOwner::new(
                    1,
                    qa_core::identity::IdentityOwner::create("lighting")
                        .unwrap()
                        .session()
                        .clone(),
                    0,
                ),
                ordinal: 0,
                source: crate::render::types::ImageSource::Generated {
                    name: "atlas".to_string(),
                },
                width: 2,
                height: 2,
            },
            texel_size: 0.25,
            near_plane: 4.0,
        };
        let depth = SharedDepth {
            width: 2,
            height: 2,
            pixels: std::sync::Arc::new(vec![1.0; 4]),
        };
        assert_eq!(
            shadow_visibility(
                vec3(0.0, 0.0, 0.0),
                vec3(0.0, 0.0, 10.0),
                100.0,
                &Q2ShadowProjection::None,
                &atlas,
                &depth,
                false
            ),
            1.0
        );
    }

    #[test]
    fn vertex_shade_modulates_texel() {
        let lighting = CpuLighting {
            parameters: &BatchLighting::Vertex,
            depth: None,
        };
        let result = shade_q2_fragment(
            &lighting,
            vec3(0.0, 0.0, 0.0),
            vec3(0.0, 0.0, 1.0),
            vec4(0.5, 0.5, 0.5, 0.5),
            &Sample {
                r: 1.0,
                g: 0.5,
                b: 0.25,
                a: 1.0,
            },
        );
        assert_eq!((result.r, result.g, result.b, result.a), (0.5, 0.25, 0.125, 0.5));
    }

    #[test]
    fn texture_pass_adds_dynamic_light() {
        let lighting_params = BatchLighting::Q2World {
            world_positions: vec![vec3(0.0, 0.0, 0.0)],
            normals: vec![vec3(0.0, 0.0, 1.0)],
            atlas: None,
            pass: Q2LightPass::Texture {
                lights: vec![Q2FragmentLight {
                    origin: vec3(0.0, 0.0, 100.0),
                    radius: 200.0,
                    color: vec3(1.0, 1.0, 1.0),
                    scale: 1.0,
                    cone: None,
                    shadow: Q2ShadowProjection::None,
                }],
            },
        };
        let lighting = CpuLighting {
            parameters: &lighting_params,
            depth: None,
        };
        let dark = shade_q2_fragment(
            &lighting,
            vec3(0.0, 0.0, 0.0),
            vec3(0.0, 0.0, 1.0),
            vec4(1.0, 1.0, 1.0, 1.0),
            &Sample {
                r: 0.2,
                g: 0.2,
                b: 0.2,
                a: 1.0,
            },
        );
        assert!(dark.r > 0.2 && dark.g > 0.2 && dark.b > 0.2);
        assert_eq!(dark.a, 1.0);
    }

    #[test]
    fn world_interpolation_is_barycentric() {
        let result = interpolate_world(
            vec3(1.0, 0.0, 0.0),
            vec3(0.0, 1.0, 0.0),
            vec3(0.0, 0.0, 1.0),
            0.5,
            0.25,
            0.25,
        );
        assert_eq!(result, vec3(0.5, 0.25, 0.25));
    }
}
