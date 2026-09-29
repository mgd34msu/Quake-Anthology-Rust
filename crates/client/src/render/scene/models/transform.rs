//! Model-space transforms and tag attachments (donor
//! `src/render/scene/models/transform.ts`).

use qa_content::md3::{interpolate_md3_tags, Md3Tag};
use qa_content::md5::sample_md5_pose;
use qa_content::q3scene::joint_attachment_tag;
use qa_core::math::{
    add3, add_point_to_bounds, cross3, dot3, empty_bounds, length3, scale3, sub3, vec3, Axis, Bounds, Vec3,
};

use crate::render::error::RenderError;

use super::types::{EntityTransform, ModelTag, SceneEntity, SceneModel, ScenePose};

/// Transform a model-space direction into world space.
#[must_use]
pub fn model_world_direction(transform: &EntityTransform, value: Vec3) -> Vec3 {
    let [forward, left, up] = transform.axis;
    let x = value.x * transform.scale.x;
    let y = value.y * transform.scale.y;
    let z = value.z * transform.scale.z;
    vec3(
        forward.x * x + left.x * y + up.x * z,
        forward.y * x + left.y * y + up.y * z,
        forward.z * x + left.z * y + up.z * z,
    )
}

/// Transform a model-space point into world space.
#[must_use]
pub fn model_world_point(transform: &EntityTransform, value: Vec3) -> Vec3 {
    let [forward, left, up] = transform.axis;
    let x = value.x * transform.scale.x;
    let y = value.y * transform.scale.y;
    let z = value.z * transform.scale.z;
    vec3(
        (f64::from(transform.origin.x) + f64::from(forward.x * x + left.x * y + up.x * z)) as f32,
        (f64::from(transform.origin.y) + f64::from(forward.y * x + left.y * y + up.y * z)) as f32,
        (f64::from(transform.origin.z) + f64::from(forward.z * x + left.z * y + up.z * z)) as f32,
    )
}

/// Q3 `R_RotateForEntity`: camera origin in model space, with reciprocal
/// first-axis length for non-normalized weapon axes.
#[must_use]
pub fn q3_model_view_origin(transform: &EntityTransform, camera: Vec3, non_normalized_axes: bool) -> Vec3 {
    let delta = sub3(camera, transform.origin);
    let [forward, left, up] = transform.axis;
    let length = length3(forward);
    let scale = if non_normalized_axes {
        if length == 0.0 {
            0.0
        } else {
            1.0 / length
        }
    } else {
        1.0
    };
    vec3(
        dot3(delta, forward) * scale,
        dot3(delta, left) * scale,
        dot3(delta, up) * scale,
    )
}

/// Transform a world-space delta into model space.
pub fn model_local_delta(transform: &EntityTransform, value: Vec3) -> Result<Vec3, RenderError> {
    let a = scale3(transform.axis[0], transform.scale.x);
    let b = scale3(transform.axis[1], transform.scale.y);
    let c = scale3(transform.axis[2], transform.scale.z);
    let bc = cross3(b, c);
    let ca = cross3(c, a);
    let ab = cross3(a, b);
    let determinant = dot3(a, bc);
    if determinant == 0.0 {
        return Err(RenderError::BadBatch {
            index: 0,
            detail: "model transform is singular".to_string(),
        });
    }
    Ok(vec3(
        dot3(value, bc) / determinant,
        dot3(value, ca) / determinant,
        dot3(value, ab) / determinant,
    ))
}

/// Compose complete linear transforms into axis columns, retaining shear.
#[must_use]
pub fn compose_model_transform(parent: &EntityTransform, child: &EntityTransform) -> EntityTransform {
    let axis: Axis = [
        model_world_direction(parent, scale3(child.axis[0], child.scale.x)),
        model_world_direction(parent, scale3(child.axis[1], child.scale.y)),
        model_world_direction(parent, scale3(child.axis[2], child.scale.z)),
    ];
    EntityTransform {
        origin: model_world_point(parent, child.origin),
        axis,
        scale: vec3(1.0, 1.0, 1.0),
    }
}

/// Transform local bounds into world bounds through all eight corners.
#[must_use]
pub fn model_world_bounds(transform: &EntityTransform, bounds: &Bounds) -> Bounds {
    let mut result = empty_bounds();
    for corner in 0..8 {
        let point = vec3(
            if corner & 1 != 0 { bounds.max.x } else { bounds.min.x },
            if corner & 2 != 0 { bounds.max.y } else { bounds.min.y },
            if corner & 4 != 0 { bounds.max.z } else { bounds.min.z },
        );
        result = add_point_to_bounds(result, model_world_point(transform, point));
    }
    result
}

/// Interpolated attachment tag plus the tag's uniform scale.
///
/// Only named tags/joints present in the decoded source can attach a model.
pub fn model_attachment_tag(entity: &SceneEntity, name: &str) -> Option<(ModelTag, f32)> {
    match (&entity.model, &entity.pose) {
        (
            SceneModel::Q3Md3(model),
            ScenePose::Frame {
                frame,
                previous_frame,
                back_lerp,
            },
        ) => {
            if model.frames.is_empty() {
                return None;
            }
            let last = model.frames.len() - 1;
            let clamp = |frame: i32| (frame.max(0) as usize).min(last);
            let first = model
                .tags
                .get(clamp(*previous_frame))?
                .iter()
                .find(|tag| tag.name == name)?;
            let second = model.tags.get(clamp(*frame))?.iter().find(|tag| tag.name == name)?;
            let start = Md3Tag {
                name: name.to_string(),
                origin: first.origin,
                axes: first.axis,
            };
            let end = Md3Tag {
                name: name.to_string(),
                origin: second.origin,
                axes: second.axis,
            };
            let tag = interpolate_md3_tags(&start, &end, name, 1.0 - back_lerp);
            Some((
                ModelTag {
                    name: name.to_string(),
                    origin: tag.origin,
                    axis: tag.axes,
                },
                1.0,
            ))
        }
        (SceneModel::Md5(model), pose) => {
            let index = model.joints.iter().position(|joint| joint.name == name)?;
            let joints = match pose {
                ScenePose::Skeleton { joints } => joints.clone(),
                ScenePose::Frame {
                    frame,
                    previous_frame,
                    back_lerp,
                } => {
                    if model.frames.is_empty() {
                        return None;
                    }
                    sample_md5_pose(&model.frames, *frame, *previous_frame, *back_lerp)
                }
            };
            let joint = joints.get(index)?;
            let tag = joint_attachment_tag(name, joint);
            Some((
                ModelTag {
                    name: tag.name,
                    origin: tag.origin,
                    axis: tag.axis,
                },
                tag.scale,
            ))
        }
        _ => None,
    }
}

/// Attach a child entity to a parent tag, carrying motion deltas and the
/// parent's lighting origin.
#[must_use]
pub fn attach_scene_entity(parent: &SceneEntity, child: &SceneEntity, tag: &ModelTag, scale: f32) -> SceneEntity {
    let tag_transform = compose_model_transform(
        &parent.transform,
        &EntityTransform {
            origin: tag.origin,
            axis: tag.axis,
            scale: vec3(scale, scale, scale),
        },
    );
    let transform = compose_model_transform(&tag_transform, &child.transform);
    let delta = model_world_direction(&tag_transform, sub3(child.previous_origin, child.transform.origin));
    SceneEntity {
        transform,
        previous_origin: add3(transform.origin, delta),
        lighting_origin: parent.lighting_origin,
        ..child.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::super::types::{EntityFlags, ModelResource};
    use super::*;
    use qa_core::math::{vec3, vec4, Bounds};

    fn identity_entity() -> SceneEntity {
        SceneEntity {
            resource: ModelResource {
                id: "test".to_string(),
                requested_path: "test".to_string(),
                digest: 0,
            },
            model: SceneModel::BrushModel,
            pose: ScenePose::Frame {
                frame: 0,
                previous_frame: 0,
                back_lerp: 0.0,
            },
            transform: EntityTransform::identity(),
            previous_origin: vec3(0.0, 0.0, 0.0),
            lighting_origin: vec3(0.0, 0.0, 5.0),
            color: vec4(1.0, 1.0, 1.0, 1.0),
            skin: 0,
            shader_time_seconds: 0.0,
            flags: EntityFlags::Q3 { bits: 0 },
            attachments: Vec::new(),
            actor_slot: None,
        }
    }

    #[test]
    fn identity_transform_preserves_points() {
        let transform = EntityTransform::identity();
        let point = vec3(1.0, 2.0, 3.0);
        assert_eq!(model_world_point(&transform, point), point);
        assert_eq!(model_world_direction(&transform, point), point);
    }

    #[test]
    fn scaled_offset_transform_moves_points() {
        let transform = EntityTransform {
            origin: vec3(10.0, 0.0, 0.0),
            axis: EntityTransform::identity().axis,
            scale: vec3(2.0, 2.0, 2.0),
        };
        assert_eq!(model_world_point(&transform, vec3(1.0, 1.0, 1.0)), vec3(12.0, 2.0, 2.0));
        assert_eq!(
            model_world_direction(&transform, vec3(1.0, 0.0, 0.0)),
            vec3(2.0, 0.0, 0.0)
        );
    }

    #[test]
    fn local_delta_inverts_world_direction() {
        let transform = EntityTransform {
            origin: vec3(4.0, 5.0, 6.0),
            axis: EntityTransform::identity().axis,
            scale: vec3(2.0, 0.5, 1.0),
        };
        let local = vec3(1.0, 2.0, 3.0);
        let world = model_world_direction(&transform, local);
        let round_trip = model_local_delta(&transform, world).expect("invertible");
        assert!((round_trip.x - local.x).abs() < 1e-5);
        assert!((round_trip.y - local.y).abs() < 1e-5);
        assert!((round_trip.z - local.z).abs() < 1e-5);
    }

    #[test]
    fn singular_transform_errors() {
        let transform = EntityTransform {
            origin: vec3(0.0, 0.0, 0.0),
            axis: EntityTransform::identity().axis,
            scale: vec3(0.0, 1.0, 1.0),
        };
        assert!(model_local_delta(&transform, vec3(1.0, 0.0, 0.0)).is_err());
    }

    #[test]
    fn compose_chains_origins() {
        let parent = EntityTransform {
            origin: vec3(1.0, 0.0, 0.0),
            ..EntityTransform::identity()
        };
        let child = EntityTransform {
            origin: vec3(0.0, 2.0, 0.0),
            ..EntityTransform::identity()
        };
        let composed = compose_model_transform(&parent, &child);
        assert_eq!(composed.origin, vec3(1.0, 2.0, 0.0));
        assert_eq!(composed.scale, vec3(1.0, 1.0, 1.0));
    }

    #[test]
    fn world_bounds_cover_corners() {
        let transform = EntityTransform {
            origin: vec3(1.0, 0.0, 0.0),
            ..EntityTransform::identity()
        };
        let bounds = Bounds {
            min: vec3(-1.0, -1.0, -1.0),
            max: vec3(1.0, 1.0, 1.0),
        };
        let world = model_world_bounds(&transform, &bounds);
        assert_eq!(world.min, vec3(0.0, -1.0, -1.0));
        assert_eq!(world.max, vec3(2.0, 1.0, 1.0));
    }

    #[test]
    fn q3_view_origin_projects_delta() {
        let transform = EntityTransform::identity();
        let view = q3_model_view_origin(&transform, vec3(3.0, 4.0, 5.0), false);
        assert_eq!(view, vec3(3.0, 4.0, 5.0));
    }

    #[test]
    fn attach_carries_lighting_origin() {
        let parent = identity_entity();
        let child = identity_entity();
        let tag = ModelTag {
            name: "tag".to_string(),
            origin: vec3(1.0, 0.0, 0.0),
            axis: EntityTransform::identity().axis,
        };
        let attached = attach_scene_entity(&parent, &child, &tag, 1.0);
        assert_eq!(attached.transform.origin, vec3(1.0, 0.0, 0.0));
        assert_eq!(attached.lighting_origin, parent.lighting_origin);
    }

    #[test]
    fn brush_models_have_no_tags() {
        let entity = identity_entity();
        assert!(model_attachment_tag(&entity, "tag_torso").is_none());
    }
}
