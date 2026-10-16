use qa_core::primitives::{IndexSpan, JointPose, Vec3};
use std::sync::Arc;

/// Frame-major model-space joints shared by all meshes of one resource.
#[derive(Debug)]
pub struct Skeleton {
    joint_count: u32,
    poses: Box<[JointPose]>,
}
impl Skeleton {
    pub fn new(joint_count: u32, poses: Box<[JointPose]>) -> Result<Self, &'static str> {
        if joint_count == 0
            || poses.is_empty()
            || !poses.len().is_multiple_of(joint_count as usize)
            || poses.len() > u32::MAX as usize
            || poses.iter().any(|pose| {
                !pose.position.is_finite()
                    || !pose.orientation.iter().all(|value| value.is_finite())
                    || !pose.scale.is_finite()
            })
        {
            return Err("invalid skeletal poses");
        }
        Ok(Self { joint_count, poses })
    }
    pub fn joint_count(&self) -> u32 {
        self.joint_count
    }
    pub fn frame_count(&self) -> u32 {
        (self.poses.len() / self.joint_count as usize) as u32
    }
    pub(super) fn frame(&self, frame: u32) -> Option<&[JointPose]> {
        if frame >= self.frame_count() {
            return None;
        }
        let first = frame as usize * self.joint_count as usize;
        Some(&self.poses[first..first + self.joint_count as usize])
    }
    pub(crate) fn prepare(
        &self,
        current: u32,
        previous: u32,
        back: f32,
        output: &mut [PreparedJoint],
    ) -> Option<()> {
        if !back.is_finite()
            || !(0.0..=1.0).contains(&back)
            || output.len() != self.joint_count as usize
        {
            return None;
        }
        let direct = current == previous || back == 0.0;
        let current = self.frame(current)?;
        let previous = self.frame(previous)?;
        for ((output, current), previous) in output.iter_mut().zip(current).zip(previous) {
            let position = if direct {
                current.position
            } else {
                previous.position * back + current.position * (1.0 - back)
            };
            let orientation = if direct {
                current.orientation
            } else {
                qa_core::math::slerp_quaternion(previous.orientation, current.orientation, back)
            };
            let rows = qa_core::math::quaternion_rotation_rows(orientation);
            if !position.is_finite() || !rows.iter().all(|row| row.is_finite()) {
                return None;
            }
            *output = PreparedJoint {
                position,
                rows,
                scale: current.scale,
            };
        }
        Some(())
    }
}

pub use qa_core::primitives::SkeletalWeight;

/// Per-mesh weight ranges refer to the single shared skeleton.
pub struct SkeletalMesh {
    pub(super) skeleton: Arc<Skeleton>,
    pub(super) weights: Box<[SkeletalWeight]>,
    pub(super) vertex_weights: Box<[IndexSpan]>,
    pub(super) bind_normals: Box<[Vec3]>,
}
impl SkeletalMesh {
    pub fn new(
        skeleton: Arc<Skeleton>,
        weights: Box<[SkeletalWeight]>,
        vertex_weights: Box<[IndexSpan]>,
        bind_normals: Box<[Vec3]>,
    ) -> Result<Self, &'static str> {
        if vertex_weights.len() != bind_normals.len()
            || weights.len() > u32::MAX as usize
            || weights.iter().any(|weight| {
                weight.joint >= skeleton.joint_count()
                    || !weight.bias.is_finite()
                    || !(0.0..=1.0).contains(&weight.bias)
                    || !weight.offset.is_finite()
            })
            || vertex_weights
                .iter()
                .any(|range| !range.fits(weights.len()))
            || bind_normals.iter().any(|normal| !normal.is_finite())
        {
            return Err("invalid skeletal mesh weights");
        }
        Ok(Self {
            skeleton,
            weights,
            vertex_weights,
            bind_normals,
        })
    }
    pub(super) fn vertex(&self, index: usize, joints: &[PreparedJoint]) -> (Vec3, Vec3) {
        let mut position = Vec3::default();
        let mut normal = Vec3::default();
        for weight in &self.weights[self.vertex_weights[index].range()] {
            let joint = joints[weight.joint as usize];
            let rotated = Vec3(joint.rows.map(|row| weight.offset.dot(row)));
            let scaled = Vec3(std::array::from_fn(|axis| {
                rotated.0[axis] * joint.scale.0[axis]
            }));
            position = position + (joint.position + scaled) * weight.bias;
            let rotated_normal = Vec3(joint.rows.map(|row| self.bind_normals[index].dot(row)));
            normal = normal + rotated_normal * weight.bias;
        }
        (position, normal)
    }
}

/// Final packet transform rows; no quaternion interpolation occurs per vertex.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct PreparedJoint {
    position: Vec3,
    rows: [Vec3; 3],
    scale: Vec3,
}
