//! MD3 scene records and character metadata (`src/formats/q3-model/scene.ts`).
//!
//! Donor provenance: `src/formats/q3-model/scene.ts` (scene records
//! mirror `tr_model.c` registration; attachment tags mirror `R_LerpTag`).

use qa_core::math::{vec3, Axis, Bounds, Vec2, Vec3};

use crate::md3::{Md3Model, Md3Vertex};
use crate::md5::SkeletonJointPose;
use crate::quaternion::rotate_quaternion_axis;

/// Scene MD3 frame.
#[derive(Debug, Clone, PartialEq)]
pub struct SceneMd3Frame {
    /// Name.
    pub name: String,
    /// Bounds.
    pub bounds: Bounds,
    /// Local origin.
    pub local_origin: Vec3,
    /// Radius.
    pub radius: f32,
}

/// Scene MD3 tag.
#[derive(Debug, Clone, PartialEq)]
pub struct SceneMd3Tag {
    /// Name.
    pub name: String,
    /// Origin.
    pub origin: Vec3,
    /// Axes.
    pub axis: Axis,
}

/// Scene MD3 surface.
#[derive(Debug, Clone, PartialEq)]
pub struct SceneMd3Surface {
    /// Name.
    pub name: String,
    /// Shader names.
    pub shaders: Vec<String>,
    /// Texture coordinates.
    pub texture_coordinates: Vec<Vec2>,
    /// Indices.
    pub indices: Vec<u32>,
    /// Per-frame vertices.
    pub frames: Vec<Vec<Md3Vertex>>,
}

/// Scene MD3 model with its source records (`DecodedSceneMd3`).
#[derive(Debug, Clone, PartialEq)]
pub struct SceneMd3 {
    /// Name.
    pub name: String,
    /// Source records.
    pub source_model: Md3Model,
    /// Frames.
    pub frames: Vec<SceneMd3Frame>,
    /// Per-frame tags.
    pub tags: Vec<Vec<SceneMd3Tag>>,
    /// Surfaces.
    pub surfaces: Vec<SceneMd3Surface>,
}

/// Adapt an MD3 model to its scene record (`toSceneMd3`).
#[must_use]
pub fn to_scene_md3(model: Md3Model) -> SceneMd3 {
    SceneMd3 {
        name: model.name.clone(),
        source_model: model.clone(),
        frames: model
            .frames
            .iter()
            .map(|frame| SceneMd3Frame {
                name: frame.name.clone(),
                bounds: frame.bounds,
                local_origin: frame.origin,
                radius: frame.radius,
            })
            .collect(),
        tags: model
            .tags
            .iter()
            .map(|tags| {
                tags.iter()
                    .map(|tag| SceneMd3Tag {
                        name: tag.name.clone(),
                        origin: tag.origin,
                        axis: tag.axes,
                    })
                    .collect()
            })
            .collect(),
        surfaces: model
            .surfaces
            .iter()
            .map(|surface| SceneMd3Surface {
                name: surface.name.clone(),
                shaders: surface.shaders.iter().map(|shader| shader.name.clone()).collect(),
                texture_coordinates: surface.tex_coords.clone(),
                indices: surface.triangles.iter().flat_map(|triangle| triangle.indices).collect(),
                frames: surface.frames.clone(),
            })
            .collect(),
    }
}

/// Character source family (`"q1" | "q2" | "q3"`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CharacterFamily {
    /// Quake.
    Q1,
    /// Quake II.
    Q2,
    /// Quake III.
    Q3,
}

/// Character model part (`CharacterModelPart`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CharacterModelPart {
    /// Part name.
    pub name: String,
    /// Model path.
    pub model_path: String,
    /// Skin path, when separate.
    pub skin_path: Option<String>,
}

/// Model transform (`ModelTransform`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ModelTransform {
    /// Origin.
    pub origin: Vec3,
    /// Axes.
    pub axis: Axis,
    /// Scale.
    pub scale: Vec3,
}

/// Character attachment anchor (`CharacterAttachmentAnchor`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CharacterAttachmentAnchor {
    /// MD3 tag anchor.
    Md3Tag {
        /// Tag name.
        tag: String,
    },
    /// Skeleton joint anchor.
    SkeletonJoint {
        /// Joint name.
        joint: String,
    },
    /// Model origin anchor.
    ModelOrigin,
}

/// Character attachment (`CharacterAttachment`).
#[derive(Debug, Clone, PartialEq)]
pub struct CharacterAttachment {
    /// Attachment name.
    pub name: String,
    /// Model part.
    pub model_part: String,
    /// Transform.
    pub transform: ModelTransform,
    /// Anchor.
    pub anchor: CharacterAttachmentAnchor,
}

/// Character animation clip (`CharacterAnimationClip`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CharacterAnimationClip {
    /// Action name.
    pub action: String,
    /// Model part.
    pub part: String,
    /// Clip name.
    pub clip: String,
}

/// Character model metadata (`CharacterModelMetadata`).
#[derive(Debug, Clone, PartialEq)]
pub struct CharacterModelMetadata {
    /// Source family.
    pub source_family: CharacterFamily,
    /// Model parts.
    pub parts: Vec<CharacterModelPart>,
    /// Attachments.
    pub attachments: Vec<CharacterAttachment>,
    /// Animations.
    pub animations: Vec<CharacterAnimationClip>,
    /// Visual scale.
    pub visual_scale: Vec3,
}

/// Joint attachment tag (`JointAttachmentPose`).
#[derive(Debug, Clone, PartialEq)]
pub struct JointAttachmentTag {
    /// Tag name.
    pub name: String,
    /// Origin.
    pub origin: Vec3,
    /// Axes.
    pub axis: Axis,
    /// Scale.
    pub scale: f32,
}

/// Build an attachment tag from a skeleton joint pose
/// (`jointAttachmentTag`).
#[must_use]
pub fn joint_attachment_tag(name: &str, pose: &SkeletonJointPose) -> JointAttachmentTag {
    JointAttachmentTag {
        name: name.to_string(),
        origin: pose.position,
        axis: [
            rotate_quaternion_axis(pose.orientation, vec3(1.0, 0.0, 0.0)),
            rotate_quaternion_axis(pose.orientation, vec3(0.0, 1.0, 0.0)),
            rotate_quaternion_axis(pose.orientation, vec3(0.0, 0.0, 1.0)),
        ],
        scale: pose.scale,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::md3::{Md3Frame, Md3Surface, Md3Tag};
    use crate::quaternion::md5_quaternion;
    use qa_core::math::{vec3 as make_vec3, Bounds};

    fn model_fixture() -> Md3Model {
        Md3Model {
            name: "test".to_string(),
            flags: 0,
            skin_count: 0,
            frames: vec![Md3Frame {
                bounds: Bounds {
                    min: make_vec3(0.0, 0.0, 0.0),
                    max: make_vec3(1.0, 1.0, 1.0),
                },
                origin: make_vec3(0.0, 0.0, 0.0),
                radius: 1.0,
                name: "frame".to_string(),
            }],
            tags: vec![vec![Md3Tag {
                name: "tag".to_string(),
                origin: make_vec3(1.0, 2.0, 3.0),
                axes: [
                    make_vec3(1.0, 0.0, 0.0),
                    make_vec3(0.0, 1.0, 0.0),
                    make_vec3(0.0, 0.0, 1.0),
                ],
            }]],
            surfaces: vec![Md3Surface {
                name: "head".to_string(),
                flags: 0,
                shaders: vec![],
                triangles: vec![],
                tex_coords: vec![],
                frames: vec![vec![]],
            }],
        }
    }

    #[test]
    fn scene_records() {
        let scene = to_scene_md3(model_fixture());
        assert_eq!(scene.name, "test");
        assert_eq!(scene.frames[0].local_origin, make_vec3(0.0, 0.0, 0.0));
        assert_eq!(scene.tags[0][0].origin, make_vec3(1.0, 2.0, 3.0));
        assert_eq!(scene.surfaces[0].name, "head");
        assert_eq!(scene.source_model.name, "test");
        let tag = joint_attachment_tag(
            "tag_weapon",
            &SkeletonJointPose {
                position: make_vec3(1.0, 2.0, 3.0),
                orientation: md5_quaternion(make_vec3(0.0, 0.0, 0.0)),
                scale: 2.0,
            },
        );
        assert_eq!(tag.name, "tag_weapon");
        assert_eq!(tag.origin, make_vec3(1.0, 2.0, 3.0));
        assert_eq!(tag.axis[0], make_vec3(1.0, 0.0, 0.0));
        assert_eq!(tag.scale, 2.0);
    }
}
