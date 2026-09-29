//! Model-space weapon grips (donor `src/render/scene/models/grip.ts`).
//!
//! Qualified source geometry supplies the animated socket; no destination
//! weapon pose is simulated.

use qa_core::math::{cross3, length3, scale3, sub3, Vec3};

use crate::render::error::RenderError;

use super::attachment::align_model_attachment;
use super::prepare::{interpolate_scene_md2, repair_frames};
use super::transform::{compose_model_transform, model_attachment_tag};
use super::types::{at, EntityTransform, SceneEntity, SceneModel, ScenePose};

/// Grip definition against a reference entity's source bytes.
#[derive(Debug, Clone, PartialEq)]
pub struct GripDefinition {
    /// Digest the reference entity must match.
    pub digest: u64,
    /// Grip anchor.
    pub anchor: GripAnchor,
    /// Grip offset from the anchor.
    pub grip: EntityTransform,
}

/// Joint or mesh-triangle anchor.
#[derive(Debug, Clone, PartialEq)]
pub enum GripAnchor {
    /// Named tag/joint anchor.
    Joint {
        /// Tag name.
        name: String,
    },
    /// Animated MD2 triangle anchor.
    Mesh {
        /// Source vertex indices (first three form the triangle).
        vertices: Vec<usize>,
        /// Reference frame for the bind triangle.
        reference_frame: usize,
    },
}

fn triangle(a: Vec3, b: Vec3, c: Vec3) -> Option<EntityTransform> {
    let forward = sub3(b, a);
    let normal = cross3(forward, sub3(c, a));
    let length = length3(forward);
    let area = length3(normal);
    if length == 0.0 || area == 0.0 {
        return None;
    }
    let x = scale3(forward, 1.0 / length);
    let z = scale3(normal, 1.0 / area);
    Some(EntityTransform {
        origin: a,
        axis: [x, cross3(z, x), z],
        scale: Vec3 { x: 1.0, y: 1.0, z: 1.0 },
    })
}

/// Bound grip resolving model-space sockets per entity pose.
#[derive(Debug, Clone)]
pub enum ModelGrip {
    /// Joint-anchored grip.
    Joint {
        /// Required resource identity.
        resource_id: String,
        /// Tag name.
        name: String,
        /// Grip offset.
        grip: EntityTransform,
    },
    /// Mesh-anchored grip over MD2 frames.
    Mesh {
        /// Required resource identity.
        resource_id: String,
        /// Source frames (trimmed to grip vertices).
        frames: qa_content::md2::Md2Model,
        /// Grip relative to the bind triangle.
        relative: EntityTransform,
    },
}

impl ModelGrip {
    /// Bind a grip definition to its reference entity.
    pub fn new(reference: &SceneEntity, definition: &GripDefinition) -> Result<Self, RenderError> {
        if reference.resource.digest != definition.digest {
            return Err(RenderError::Backend(
                "model attachment digest differs from its source bytes".to_string(),
            ));
        }
        match &definition.anchor {
            GripAnchor::Joint { name } => Ok(Self::Joint {
                resource_id: reference.resource.id.clone(),
                name: name.clone(),
                grip: definition.grip,
            }),
            GripAnchor::Mesh {
                vertices,
                reference_frame,
            } => {
                let SceneModel::Q2Md2 { model, .. } = &reference.model else {
                    return Err(RenderError::Backend(
                        "mesh attachment requires its declared MD2 source".to_string(),
                    ));
                };
                let mut trimmed = model.clone();
                trimmed.frames = model
                    .frames
                    .iter()
                    .map(|frame| {
                        let selected = vertices
                            .iter()
                            .map(|index| {
                                Ok((
                                    *at(&frame.vertices, *index, "attachment vertex")?,
                                    *at(&frame.compressed_vertices, *index, "attachment packed vertex")?,
                                ))
                            })
                            .collect::<Result<Vec<_>, RenderError>>()?;
                        let mut frame = frame.clone();
                        frame.vertices = selected.iter().map(|pair| pair.0).collect();
                        frame.compressed_vertices = selected.iter().map(|pair| pair.1).collect();
                        Ok(frame)
                    })
                    .collect::<Result<Vec<_>, RenderError>>()?;
                let positions = &at(&trimmed.frames, *reference_frame, "attachment reference frame")?.vertices;
                let a = at(positions, 0, "attachment vertex")?;
                let b = at(positions, 1, "attachment vertex")?;
                let c = at(positions, 2, "attachment vertex")?;
                let initial = triangle(
                    Vec3 {
                        x: a.position[0],
                        y: a.position[1],
                        z: a.position[2],
                    },
                    Vec3 {
                        x: b.position[0],
                        y: b.position[1],
                        z: b.position[2],
                    },
                    Vec3 {
                        x: c.position[0],
                        y: c.position[1],
                        z: c.position[2],
                    },
                )
                .ok_or_else(|| RenderError::Backend("model attachment reference is degenerate".to_string()))?;
                let inverse = align_model_attachment(&initial, &EntityTransform::identity())?;
                Ok(Self::Mesh {
                    resource_id: reference.resource.id.clone(),
                    frames: trimmed,
                    relative: compose_model_transform(&inverse, &definition.grip),
                })
            }
        }
    }

    /// Resolve the grip transform for a live entity pose.
    pub fn apply(&self, entity: &SceneEntity) -> Result<Option<EntityTransform>, RenderError> {
        match self {
            Self::Joint {
                resource_id,
                name,
                grip,
            } => {
                if &entity.resource.id != resource_id {
                    return Err(RenderError::Backend("model attachment source changed".to_string()));
                }
                let pose = match &entity.pose {
                    ScenePose::Frame { .. } => {
                        let repaired = repair_frames(entity)?;
                        ScenePose::Frame {
                            frame: repaired.frame as i32,
                            previous_frame: repaired.previous_frame as i32,
                            back_lerp: repaired.back_lerp,
                        }
                    }
                    ScenePose::Skeleton { .. } => entity.pose.clone(),
                };
                let posed = SceneEntity { pose, ..entity.clone() };
                let (tag, scale) = model_attachment_tag(&posed, name)
                    .ok_or_else(|| RenderError::Backend(format!("source model has no attachment {name}")))?;
                Ok(Some(compose_model_transform(
                    &EntityTransform {
                        origin: tag.origin,
                        axis: tag.axis,
                        scale: Vec3 {
                            x: scale,
                            y: scale,
                            z: scale,
                        },
                    },
                    grip,
                )))
            }
            Self::Mesh {
                resource_id,
                frames,
                relative,
            } => {
                if &entity.resource.id != resource_id || !matches!(entity.pose, ScenePose::Frame { .. }) {
                    return Err(RenderError::Backend(
                        "mesh attachment lost its original source pose".to_string(),
                    ));
                }
                let repaired = repair_frames(entity)?;
                let vertices = interpolate_scene_md2(
                    frames,
                    entity,
                    repaired.frame,
                    repaired.previous_frame,
                    repaired.back_lerp,
                    false,
                )?;
                let a = at(&vertices, 0, "attachment vertex")?;
                let b = at(&vertices, 1, "attachment vertex")?;
                let c = at(&vertices, 2, "attachment vertex")?;
                let Some(current) = triangle(a.position, b.position, c.position) else {
                    return Ok(None);
                };
                Ok(Some(compose_model_transform(&current, relative)))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::types::{EntityFlags, ModelResource};
    use super::*;
    use qa_core::math::{vec3, vec4};

    fn entity(model: SceneModel, digest: u64) -> SceneEntity {
        SceneEntity {
            resource: ModelResource {
                id: "weapon".to_string(),
                requested_path: "weapon".to_string(),
                digest,
            },
            model,
            pose: ScenePose::Frame {
                frame: 0,
                previous_frame: 0,
                back_lerp: 0.0,
            },
            transform: EntityTransform::identity(),
            previous_origin: vec3(0.0, 0.0, 0.0),
            lighting_origin: vec3(0.0, 0.0, 0.0),
            color: vec4(1.0, 1.0, 1.0, 1.0),
            skin: 0,
            shader_time_seconds: 0.0,
            flags: EntityFlags::Q2 { bits: 0 },
            attachments: Vec::new(),
            actor_slot: None,
        }
    }

    #[test]
    fn digest_mismatch_is_rejected() {
        let reference = entity(SceneModel::BrushModel, 1);
        let definition = GripDefinition {
            digest: 2,
            anchor: GripAnchor::Joint {
                name: "tag_weapon".to_string(),
            },
            grip: EntityTransform::identity(),
        };
        assert!(ModelGrip::new(&reference, &definition).is_err());
    }

    #[test]
    fn mesh_grip_requires_md2() {
        let reference = entity(SceneModel::BrushModel, 1);
        let definition = GripDefinition {
            digest: 1,
            anchor: GripAnchor::Mesh {
                vertices: vec![0, 1, 2],
                reference_frame: 0,
            },
            grip: EntityTransform::identity(),
        };
        assert!(ModelGrip::new(&reference, &definition).is_err());
    }

    #[test]
    fn joint_grip_rejects_changed_source() {
        let reference = entity(SceneModel::BrushModel, 1);
        let definition = GripDefinition {
            digest: 1,
            anchor: GripAnchor::Joint {
                name: "tag_weapon".to_string(),
            },
            grip: EntityTransform::identity(),
        };
        let grip = ModelGrip::new(&reference, &definition).expect("grip");
        let mut other = entity(SceneModel::BrushModel, 1);
        other.resource.id = "other".to_string();
        assert!(grip.apply(&other).is_err());
    }

    #[test]
    fn degenerate_triangle_returns_none() {
        assert!(triangle(vec3(0.0, 0.0, 0.0), vec3(0.0, 0.0, 0.0), vec3(1.0, 0.0, 0.0)).is_none());
        assert!(triangle(vec3(0.0, 0.0, 0.0), vec3(1.0, 0.0, 0.0), vec3(0.0, 1.0, 0.0)).is_some());
    }
}
