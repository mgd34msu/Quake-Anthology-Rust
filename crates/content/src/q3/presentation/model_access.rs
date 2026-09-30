//! Quake III presentation: model access.
//!
//! Donor provenance: `src/content/q3/presentation/model-access.ts`.

use crate::md3::interpolate_md3_tags;
use crate::md5::sample_md5_pose;
use crate::q3scene::joint_attachment_tag;
use qa_core::math::{Axis, Bounds, Vec3};

// Intra-group imports: sibling modules split from the same flat port.
use crate::q3::presentation::ref_entity::*;

// ---------------------------------------------------------------------------
// model-access.ts
// ---------------------------------------------------------------------------

/// Model bounds (`modelBounds`).
#[must_use]
pub fn model_bounds(source: &SceneModel) -> Bounds {
    let zero = Bounds {
        min: zero_vec3(),
        max: zero_vec3(),
    };
    match source {
        SceneModel::Default(_) => zero,
        SceneModel::Inline(inline) => inline.bounds,
        SceneModel::Loaded(loaded) => match &loaded.model {
            Q3DecodedModel::Bounded { bounds } => *bounds,
            Q3DecodedModel::Brush { models, index } => models.get(*index).copied().unwrap_or(zero),
            Q3DecodedModel::Framed { frames } => frames.first().copied().unwrap_or(zero),
            Q3DecodedModel::Md3(_) | Q3DecodedModel::Md5(_) => zero,
        },
    }
}

/// Interpolated model tag (`lerpModelTag`).
pub fn lerp_model_tag(source: &SceneModel, name: &str, start: i32, end: i32, fraction: f32) -> Option<ModelTag> {
    let SceneModel::Loaded(loaded) = source else {
        return None;
    };
    match &loaded.model {
        Q3DecodedModel::Md3(model) => {
            let last = model.frames.len().saturating_sub(1);
            let first = model
                .tags
                .get(start.min(last as i32).max(0) as usize)?
                .iter()
                .find(|tag| tag.name == name)?;
            let second = model
                .tags
                .get(end.min(last as i32).max(0) as usize)?
                .iter()
                .find(|tag| tag.name == name)?;
            let tag = interpolate_md3_tags(first, second, name, fraction);
            Some(ModelTag {
                origin: tag.origin,
                axes: tag.axes,
            })
        }
        Q3DecodedModel::Md5(md5) => {
            let index = md5.joint_names.iter().position(|joint| joint == name)?;
            let pose = sample_md5_pose(&md5.frames, end, start, 1.0 - fraction)
                .get(index)
                .copied()?;
            let tag = joint_attachment_tag(name, &pose);
            Some(ModelTag {
                origin: tag.origin,
                axes: tag.axis,
            })
        }
        _ => None,
    }
}

/// Interpolated tag pose.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ModelTag {
    /// Origin.
    pub origin: Vec3,
    /// Axes.
    pub axes: Axis,
}
