//! Enhanced model replacement selection (donor
//! `src/render/scene/models/replacements.ts`).

use qa_content::replacements::QFamily;
use qa_core::math::{length3, sub3, Vec3};

use super::types::{SceneEntity, SceneModel};

/// Policy controlling enhanced (MD5) replacement models.
#[derive(Debug, Clone, PartialEq)]
pub struct ModelReplacementPolicy {
    /// Use enhanced Q1 replacements.
    pub q1_enhanced: bool,
    /// Load enhanced Q2 replacements.
    pub q2_load: bool,
    /// Use enhanced Q2 replacements.
    pub q2_use: bool,
    /// Q2 replacement distance cutoff.
    pub q2_distance: f32,
    /// Replacement distance cutoff, or source default.
    pub distance: ReplacementDistance,
}

/// Replacement distance cutoff.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ReplacementDistance {
    /// Source default (Q2 policy distance, Q1 always).
    Source,
    /// Explicit world-unit cutoff.
    Units(f32),
}

/// Default policy: replacements on, source distances.
pub const DEFAULT_MODEL_REPLACEMENT_POLICY: ModelReplacementPolicy = ModelReplacementPolicy {
    q1_enhanced: true,
    q2_load: true,
    q2_use: true,
    q2_distance: 2048.0,
    distance: ReplacementDistance::Source,
};

/// Whether the loader reads enhanced replacements for a family.
#[must_use]
pub const fn load_model_replacement(family: QFamily, policy: &ModelReplacementPolicy) -> bool {
    match family {
        QFamily::Q1 => policy.q1_enhanced,
        QFamily::Q2 => policy.q2_load,
    }
}

/// The replacement entity for an alias model, or `None` without one.
#[must_use]
pub fn replacement_entity(entity: &SceneEntity) -> Option<SceneEntity> {
    let replacement = match &entity.model {
        SceneModel::Q1Mdl { replacement, .. } | SceneModel::Q2Md2 { replacement, .. } => replacement.as_ref()?,
        _ => return None,
    };
    Some(SceneEntity {
        resource: replacement.resource.clone(),
        model: (*replacement.model).clone(),
        ..entity.clone()
    })
}

/// Select the view or shadow entity for an alias model.
///
/// Q2 shadows use the loaded skeleton independently of the eye-distance LOD.
#[must_use]
pub fn select_model_entity(
    entity: &SceneEntity,
    camera: Vec3,
    policy: &ModelReplacementPolicy,
    shadow: bool,
) -> SceneEntity {
    let is_q1 = matches!(entity.model, SceneModel::Q1Mdl { .. });
    let is_q2 = matches!(entity.model, SceneModel::Q2Md2 { .. });
    if !is_q1 && !is_q2 {
        return entity.clone();
    }
    if is_q1 {
        if !policy.q1_enhanced {
            return entity.clone();
        }
    } else if !policy.q2_load || !policy.q2_use {
        return entity.clone();
    }
    let distance = match policy.distance {
        ReplacementDistance::Source => {
            if is_q2 {
                policy.q2_distance
            } else {
                0.0
            }
        }
        ReplacementDistance::Units(units) => units,
    };
    if !shadow && distance > 0.0 && length3(sub3(entity.transform.origin, camera)) > distance {
        return entity.clone();
    }
    replacement_entity(entity).unwrap_or_else(|| entity.clone())
}

#[cfg(test)]
mod tests {
    use super::super::types::{EntityFlags, EntityTransform, ModelResource, ScenePose};
    use super::*;
    use qa_core::math::{vec3, vec4};

    fn entity(model: SceneModel) -> SceneEntity {
        SceneEntity {
            resource: ModelResource {
                id: "test".to_string(),
                requested_path: "test".to_string(),
                digest: 0,
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

    fn md2_entity(with_replacement: bool) -> SceneEntity {
        let replacement = with_replacement.then(|| super::super::types::SceneModelReplacement {
            resource: ModelResource {
                id: "replacement".to_string(),
                requested_path: "replacement".to_string(),
                digest: 1,
            },
            model: Box::new(SceneModel::BrushModel),
        });
        entity(SceneModel::Q2Md2 {
            model: qa_content::md2::Md2Model {
                skin_width: 64,
                skin_height: 64,
                skins: Vec::new(),
                texture_coordinates: Vec::new(),
                triangles: Vec::new(),
                frames: Vec::new(),
                gl_commands: Vec::new(),
                bounds: qa_content::common::Bounds {
                    min: [0.0; 3],
                    max: [0.0; 3],
                },
            },
            replacement,
        })
    }

    #[test]
    fn loader_policy_matches_family() {
        assert!(load_model_replacement(QFamily::Q1, &DEFAULT_MODEL_REPLACEMENT_POLICY));
        assert!(load_model_replacement(QFamily::Q2, &DEFAULT_MODEL_REPLACEMENT_POLICY));
        let off = ModelReplacementPolicy {
            q1_enhanced: false,
            q2_load: false,
            ..DEFAULT_MODEL_REPLACEMENT_POLICY
        };
        assert!(!load_model_replacement(QFamily::Q1, &off));
        assert!(!load_model_replacement(QFamily::Q2, &off));
    }

    #[test]
    fn replacement_entity_swaps_resource_and_model() {
        let swapped = replacement_entity(&md2_entity(true)).expect("replacement");
        assert_eq!(swapped.resource.id, "replacement");
        assert!(matches!(swapped.model, SceneModel::BrushModel));
        assert!(replacement_entity(&md2_entity(false)).is_none());
        assert!(replacement_entity(&entity(SceneModel::BrushModel)).is_none());
    }

    #[test]
    fn nearby_entity_selects_replacement() {
        let selected = select_model_entity(
            &md2_entity(true),
            vec3(0.0, 0.0, 0.0),
            &DEFAULT_MODEL_REPLACEMENT_POLICY,
            false,
        );
        assert_eq!(selected.resource.id, "replacement");
    }

    #[test]
    fn distant_entity_keeps_source() {
        let selected = select_model_entity(
            &md2_entity(true),
            vec3(99999.0, 0.0, 0.0),
            &DEFAULT_MODEL_REPLACEMENT_POLICY,
            false,
        );
        assert_eq!(selected.resource.id, "test");
    }

    #[test]
    fn shadow_purpose_ignores_distance() {
        let selected = select_model_entity(
            &md2_entity(true),
            vec3(99999.0, 0.0, 0.0),
            &DEFAULT_MODEL_REPLACEMENT_POLICY,
            true,
        );
        assert_eq!(selected.resource.id, "replacement");
    }

    #[test]
    fn disabled_policy_keeps_source() {
        let off = ModelReplacementPolicy {
            q2_use: false,
            ..DEFAULT_MODEL_REPLACEMENT_POLICY
        };
        let selected = select_model_entity(&md2_entity(true), vec3(0.0, 0.0, 0.0), &off, false);
        assert_eq!(selected.resource.id, "test");
    }
}
