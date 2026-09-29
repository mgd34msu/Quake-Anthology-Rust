//! Projected dynamic lights (`ProjectDlightTexture`, `R_Dlight*`).
//!
//! Donor provenance: `src/materials/dlight.ts` (from `tr_shade.c`,
//! `tr_world.c`, `tr_light.c`).

use qa_core::math::{dot3, sub3, vec3, Axis, Bounds, Plane, Vec3, Vec4};

use super::compile::CompiledMaterial;
use super::deform::DeformGeometry;
use super::evaluate::{BatchLighting, BatchVertex, MaterialBatch, TextureRef, Texturing};
use super::iterator::MaterialIteratorKind;
use super::q3_lighting::DynamicLight;
use super::state::{AlphaTest, Blend, BlendFactor, CullFace, DepthTest, RenderState};
use crate::ClientError;

/// Light bit with source wraparound (`1 << index` is mod 32 in C/JS).
fn bit(index: usize) -> u32 {
    1u32 << (index & 31)
}

/// Transform dlights into model space (`transformDlights`).
#[must_use]
pub fn transform_dlights(
    lights: &[DynamicLight],
    origin: Vec3,
    axis: &Axis,
) -> Vec<DynamicLight> {
    lights
        .iter()
        .map(|light| {
            let relative = sub3(light.origin, origin);
            DynamicLight {
                origin: vec3(
                    dot3(relative, axis[0]),
                    dot3(relative, axis[1]),
                    dot3(relative, axis[2]),
                ),
                ..*light
            }
        })
        .collect()
}

/// Split a dlight mask by a plane (`splitDlightMask`).
#[must_use]
pub fn split_dlight_mask(
    lights: &[DynamicLight],
    mask: u32,
    plane: &Plane,
) -> (u32, u32) {
    if mask == 0 {
        return (0, 0);
    }
    let mut front = 0u32;
    let mut back = 0u32;
    for (index, light) in lights.iter().enumerate() {
        let light_bit = bit(index);
        if mask & light_bit == 0 {
            continue;
        }
        let distance = dot3(light.origin, plane.normal) - plane.distance;
        if distance > -light.radius {
            front |= light_bit;
        }
        if distance < light.radius {
            back |= light_bit;
        }
    }
    (front, back)
}

/// Cull a dlight mask by a face plane (`faceDlightMask`).
#[must_use]
pub fn face_dlight_mask(lights: &[DynamicLight], mut mask: u32, plane: &Plane) -> u32 {
    for (index, light) in lights.iter().enumerate() {
        let light_bit = bit(index);
        if mask & light_bit == 0 {
            continue;
        }
        let distance = dot3(light.origin, plane.normal) - plane.distance;
        if distance < -light.radius || distance > light.radius {
            mask &= !light_bit;
        }
    }
    mask
}

/// Cull a dlight mask by bounds (`gridDlightMask`).
#[must_use]
pub fn grid_dlight_mask(lights: &[DynamicLight], mut mask: u32, bounds: &Bounds) -> u32 {
    for (index, light) in lights.iter().enumerate() {
        let light_bit = bit(index);
        if mask & light_bit == 0 {
            continue;
        }
        let origin = light.origin;
        let radius = light.radius;
        if origin.x - radius > bounds.max.x
            || origin.x + radius < bounds.min.x
            || origin.y - radius > bounds.max.y
            || origin.y + radius < bounds.min.y
            || origin.z - radius > bounds.max.z
            || origin.z + radius < bounds.min.z
        {
            mask &= !light_bit;
        }
    }
    mask
}

/// Cull dlights for a bmodel (`bmodelDlightMask`, transformed origins).
#[must_use]
pub fn bmodel_dlight_mask(lights: &[DynamicLight], bounds: &Bounds) -> u32 {
    let mut mask = 0u32;
    for (index, light) in lights.iter().enumerate() {
        let origin = light.origin;
        let radius = light.radius;
        if origin.x - bounds.max.x > radius
            || bounds.min.x - origin.x > radius
            || origin.y - bounds.max.y > radius
            || bounds.min.y - origin.y > radius
            || origin.z - bounds.max.z > radius
            || bounds.min.z - origin.z > radius
        {
            continue;
        }
        mask |= bit(index);
    }
    mask
}

/// Whether a material receives projected dlights
/// (`receivesProjectedDlights`).
#[must_use]
pub fn receives_projected_dlights(material: &CompiledMaterial) -> bool {
    if material.finished.sort > 3 {
        return false;
    }
    let iterator = material.finished.iterator.kind;
    // Sky clouds call the generic iterator; skyParms alone is not SURF_SKY.
    (iterator != MaterialIteratorKind::Generic && iterator != MaterialIteratorKind::Sky)
        || !material
            .registered
            .definition
            .surface_parms
            .iter()
            .any(|flag| flag == "nodlight" || flag == "sky")
}

/// Project dlight textures (`projectDlightTexture`).
pub fn project_dlight_texture(
    geometry: &DeformGeometry,
    mask: u32,
    lights: &[DynamicLight],
    image: u32,
    project: &dyn Fn(Vec3) -> Vec4,
    cull: CullFace,
) -> Result<Vec<MaterialBatch>, ClientError> {
    let mut batches = Vec::new();
    for (index, light) in lights.iter().enumerate() {
        if mask & bit(index) == 0 {
            continue;
        }
        let radius = light.radius;
        let scale = 1.0 / radius;
        let color = vec3(
            light.color.x * 255.0,
            light.color.y * 255.0,
            light.color.z * 255.0,
        );
        let mut clip_bits = Vec::with_capacity(geometry.vertices.len());
        let mut vertices = Vec::with_capacity(geometry.vertices.len());
        for vertex in &geometry.vertices {
            let distance = sub3(light.origin, vertex.position);
            let tex_coord = qa_core::math::vec2(
                0.5 + distance.x * scale,
                0.5 + distance.y * scale,
            );
            let mut clip = (i32::from(tex_coord.x < 0.0)
                | i32::from(tex_coord.x > 1.0) * 2
                | i32::from(tex_coord.y < 0.0) * 4
                | i32::from(tex_coord.y > 1.0) * 8) as u32;
            let modulate = if distance.z > radius {
                clip |= 16;
                0.0
            } else if distance.z < -radius {
                clip |= 32;
                0.0
            } else {
                let height = distance.z.abs();
                if height < radius * 0.5 {
                    1.0
                } else {
                    2.0 * (radius - height) * scale
                }
            };
            clip_bits.push(clip);
            // Linux myftol truncates; byte assignment keeps low eight bits.
            let byte = |component: f32| {
                (((component * modulate).trunc() as i32) & 255) as u8 as f32 / 255.0
            };
            vertices.push(BatchVertex {
                position: project(vertex.position),
                tex_coord,
                tex_coord2: None,
                color: qa_core::math::vec4(byte(color.x), byte(color.y), byte(color.z), 1.0),
            });
        }
        let mut indices = Vec::new();
        let mut offset = 0usize;
        while offset + 2 < geometry.indices.len() + 2 && offset < geometry.indices.len() {
            if offset + 2 >= geometry.indices.len() {
                break;
            }
            let a = geometry.indices[offset] as usize;
            let b = geometry.indices[offset + 1] as usize;
            let c = geometry.indices[offset + 2] as usize;
            let (Some(ac), Some(bc), Some(cc)) = (
                clip_bits.get(a),
                clip_bits.get(b),
                clip_bits.get(c),
            ) else {
                return Err(ClientError::BadMaterial(
                    "ProjectDlightTexture: triangle has no active clipBits entry; inactive source scratch is indeterminate".to_string(),
                ));
            };
            if (ac & bc & cc) == 0 {
                indices.extend([a as u32, b as u32, c as u32]);
            }
            offset += 3;
        }
        if indices.is_empty() {
            continue;
        }
        batches.push(MaterialBatch {
            lighting: BatchLighting::Vertex,
            fog: None,
            texturing: Texturing::Single,
            state: RenderState {
                blend: Blend {
                    source: if light.additive {
                        BlendFactor::One
                    } else {
                        BlendFactor::DstColor
                    },
                    destination: BlendFactor::One,
                },
                depth_test: DepthTest::Equal,
                depth_write: false,
                alpha_test: AlphaTest::None,
                cull,
                depth_range: [0.0, 1.0],
                polygon_offset: None,
            },
            texture: TextureRef::BindImage(image),
            second_texture: None,
            indices,
            vertices,
        });
    }
    Ok(batches)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn light() -> DynamicLight {
        DynamicLight {
            origin: vec3(0.0, 0.0, 64.0),
            radius: 128.0,
            color: vec3(1.0, 1.0, 1.0),
            additive: false,
        }
    }

    #[test]
    fn masks_split_and_cull() {
        let lights = vec![light()];
        let plane = Plane {
            normal: vec3(0.0, 0.0, 1.0),
            distance: 0.0,
        };
        assert_eq!(split_dlight_mask(&lights, 1, &plane), (1, 1));
        assert_eq!(face_dlight_mask(&lights, 1, &plane), 1);
        let far = Plane {
            normal: vec3(0.0, 0.0, 1.0),
            distance: 1000.0,
        };
        assert_eq!(face_dlight_mask(&lights, 1, &far), 0);
    }

    #[test]
    fn bmodel_mask_accepts_inside() {
        let lights = vec![light()];
        let bounds = Bounds {
            min: vec3(-100.0, -100.0, -100.0),
            max: vec3(100.0, 100.0, 100.0),
        };
        assert_eq!(bmodel_dlight_mask(&lights, &bounds), 1);
    }

    #[test]
    fn projection_yields_batches() {
        use crate::materials::geometry::{MaterialGeometry, MaterialVertex};
        let vertex = |position: Vec3| {
            MaterialVertex::new(
                position,
                vec3(0.0, 0.0, 1.0),
                qa_core::math::vec2(0.0, 0.0),
                qa_core::math::vec2(0.0, 0.0),
                [255, 255, 255, 255],
            )
        };
        let geometry = DeformGeometry::from(MaterialGeometry {
            vertices: vec![
                vertex(vec3(-10.0, -10.0, 0.0)),
                vertex(vec3(10.0, -10.0, 0.0)),
                vertex(vec3(0.0, 10.0, 0.0)),
            ],
            indices: vec![0, 1, 2],
        });
        let batches = project_dlight_texture(
            &geometry,
            1,
            &[light()],
            5,
            &|position| qa_core::math::vec4(position.x, position.y, position.z, 1.0),
            CullFace::Front,
        )
        .unwrap();
        assert_eq!(batches.len(), 1);
    }
}
