//! MD5 shadow envelopes (donor `src/render/scene/models/shadow-bounds.ts`).
//!
//! Conservative world-space spheres bounding actual MD5 skinning, including
//! signed biases, binary-32 accumulation, and sheared entity transforms.

use std::collections::HashMap;

use qa_content::md5::{Md5Mesh, SkeletonJointPose};
use qa_content::quaternion::rotate_quaternion_axis;
use qa_core::math::{vec3, Axis, Vec3};

use super::types::{EntityTransform, ShadowSphere};

const UNIT_AXES: Axis = [
    Vec3 { x: 1.0, y: 0.0, z: 0.0 },
    Vec3 { x: 0.0, y: 1.0, z: 0.0 },
    Vec3 { x: 0.0, y: 0.0, z: 1.0 },
];

/// Four binary-32 unit roundoffs also cover the binary-64 bound arithmetic.
const ROUNDOFF: f64 = 2.384185791015625e-7;
const UNDERFLOW: f64 = 1e-30;
const MAXIMUM: f64 = 1e30;

fn length(value: Vec3) -> f64 {
    f64::from(value.x).hypot(f64::from(value.y)).hypot(f64::from(value.z))
}

fn bounded(value: f64) -> bool {
    value.is_finite() && (0.0..=MAXIMUM).contains(&value)
}

fn dot(a: Vec3, b: Vec3) -> f64 {
    f64::from(a.x) * f64::from(b.x) + f64::from(a.y) * f64::from(b.y) + f64::from(a.z) * f64::from(b.z)
}

struct WeightEnvelope {
    joints: HashMap<u32, f64>,
    bias_sum: f64,
    count: u32,
}

fn weights(mesh: &Md5Mesh) -> Option<WeightEnvelope> {
    let mut joints = HashMap::new();
    let mut bias_sum = 0.0f64;
    let mut count = 0u32;
    for vertex in &mesh.vertices {
        if vertex.weights.count > 4096 {
            return None;
        }
        count = count.max(vertex.weights.count);
        let mut sum = 0.0f64;
        for offset in 0..vertex.weights.count {
            let weight = mesh.weights.get(vertex.weights.first as usize + offset as usize)?;
            let radius = length(weight.position);
            sum += f64::from(weight.bias.abs());
            if !bounded(radius) || !bounded(sum) {
                return None;
            }
            joints.insert(
                weight.joint,
                radius.max(joints.get(&weight.joint).copied().unwrap_or(0.0)),
            );
        }
        bias_sum = bias_sum.max(sum);
    }
    Some(WeightEnvelope {
        joints,
        bias_sum: bias_sum * (1.0 + ROUNDOFF),
        count,
    })
}

/// Gershgorin bounds the largest eigenvalue of A-transpose A, including shear.
fn operator_norm(columns: &Axis) -> f64 {
    let [a, b, c] = columns;
    let ab = dot(*a, *b).abs();
    let ac = dot(*a, *c).abs();
    let bc = dot(*b, *c).abs();
    (dot(*a, *a) + ab + ac)
        .max(dot(*b, *b) + ab + bc)
        .max(dot(*c, *c) + ac + bc)
        .sqrt()
        * (1.0 + ROUNDOFF)
}

/// World-space sphere bounding skinned MD5 meshes under a transform.
#[must_use]
pub fn md5_shadow_envelope(
    meshes: &[Md5Mesh],
    joints: &[SkeletonJointPose],
    transform: &EntityTransform,
) -> Option<ShadowSphere> {
    let mut radius = 0.0f64;
    for mesh in meshes {
        let metadata = weights(mesh)?;
        let mut point_radius = 0.0f64;
        for (index, weight_radius) in &metadata.joints {
            let joint = joints.get(*index as usize)?;
            if !bounded(f64::from(joint.scale.abs())) {
                return None;
            }
            let rotation: Axis = [
                rotate_quaternion_axis(joint.orientation, UNIT_AXES[0]),
                rotate_quaternion_axis(joint.orientation, UNIT_AXES[1]),
                rotate_quaternion_axis(joint.orientation, UNIT_AXES[2]),
            ];
            let norm = operator_norm(&rotation);
            let position = length(joint.position);
            let rotated = norm * weight_radius * (1.0 + ROUNDOFF) + UNDERFLOW;
            let point = (position + f64::from(joint.scale.abs()) * rotated) * (1.0 + ROUNDOFF) + UNDERFLOW;
            if ![norm, position, rotated, point].iter().all(|value| bounded(*value)) {
                return None;
            }
            point_radius = point_radius.max(point);
        }
        let accumulated = (metadata.bias_sum * point_radius + f64::from(metadata.count) * UNDERFLOW)
            * (1.0 + ROUNDOFF).powi(metadata.count as i32);
        if !bounded(accumulated) {
            return None;
        }
        radius = radius.max(accumulated);
    }
    let scales = [
        f64::from(transform.scale.x.abs()),
        f64::from(transform.scale.y.abs()),
        f64::from(transform.scale.z.abs()),
        radius * f64::from(transform.scale.x.abs()),
        radius * f64::from(transform.scale.y.abs()),
        radius * f64::from(transform.scale.z.abs()),
    ];
    if !scales.iter().all(|value| bounded(*value)) {
        return None;
    }
    if !transform.axis.iter().map(|axis| length(*axis)).all(bounded) {
        return None;
    }
    let column = |axis: Vec3, scale: f32| vec3(axis.x * scale, axis.y * scale, axis.z * scale);
    let columns: Axis = [
        column(transform.axis[0], transform.scale.x),
        column(transform.axis[1], transform.scale.y),
        column(transform.axis[2], transform.scale.z),
    ];
    let norm = operator_norm(&columns);
    let frobenius = columns
        .iter()
        .map(|axis| length(*axis))
        .fold(0.0f64, |sum, value| sum + value * value)
        .sqrt();
    let origin = length(transform.origin);
    let transformed = norm * radius;
    let products = frobenius * radius;
    if ![norm, frobenius, origin, transformed, products]
        .iter()
        .all(|value| bounded(*value))
    {
        return None;
    }
    let world_radius = transformed + 32.0 * ROUNDOFF * (products + origin) + UNDERFLOW;
    bounded(world_radius).then(|| ShadowSphere {
        origin: transform.origin,
        radius: 64.0f32.max(world_radius as f32),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_content::bsp::IndexRange;
    use qa_content::md5::{Md5Vertex, Md5Weight};
    use qa_core::math::{vec2, vec3, vec4};

    fn mesh() -> Md5Mesh {
        Md5Mesh {
            shader: "test".to_string(),
            vertices: vec![Md5Vertex {
                tex_coord: vec2(0.0, 0.0),
                normal: vec3(0.0, 0.0, 1.0),
                weights: IndexRange { first: 0, count: 1 },
            }],
            indices: vec![0],
            weights: vec![Md5Weight {
                joint: 0,
                bias: 1.0,
                position: vec3(1.0, 0.0, 0.0),
            }],
        }
    }

    fn joints() -> Vec<SkeletonJointPose> {
        vec![SkeletonJointPose {
            position: vec3(0.0, 0.0, 0.0),
            orientation: vec4(0.0, 0.0, 0.0, 1.0),
            scale: 1.0,
        }]
    }

    #[test]
    fn envelope_has_minimum_radius() {
        let sphere = md5_shadow_envelope(&[mesh()], &joints(), &EntityTransform::identity()).expect("sphere");
        assert!(sphere.radius >= 64.0);
        assert_eq!(sphere.origin, vec3(0.0, 0.0, 0.0));
    }

    #[test]
    fn envelope_follows_origin() {
        let transform = EntityTransform {
            origin: vec3(5.0, 6.0, 7.0),
            ..EntityTransform::identity()
        };
        let sphere = md5_shadow_envelope(&[mesh()], &joints(), &transform).expect("sphere");
        assert_eq!(sphere.origin, vec3(5.0, 6.0, 7.0));
    }

    #[test]
    fn missing_joint_returns_none() {
        assert!(md5_shadow_envelope(&[mesh()], &[], &EntityTransform::identity()).is_none());
    }

    #[test]
    fn overweight_mesh_returns_none() {
        let mut bad = mesh();
        bad.vertices[0].weights.count = 5000;
        assert!(md5_shadow_envelope(&[bad], &joints(), &EntityTransform::identity()).is_none());
    }

    #[test]
    fn operator_norm_bounds_identity() {
        let norm = operator_norm(&UNIT_AXES);
        assert!((1.0..1.01).contains(&norm));
    }
}
