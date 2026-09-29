//! MD3 interpolation envelopes (donor
//! `src/render/scene/models/md3-bounds.ts`).
//!
//! Bounds the same ordered binary-32 position operations used by MD3 frame
//! interpolation, so culling envelopes never miss an interpolated vertex.

use std::collections::HashMap;

use qa_content::q3scene::SceneMd3;
use qa_core::math::{add_point_to_bounds, empty_bounds, vec3, Bounds};

use super::transform::model_world_bounds;
use super::types::EntityTransform;

/// Per-frame vertex envelope of one MD3 model.
#[derive(Debug, Clone, PartialEq)]
struct FrameEnvelope {
    /// Local bounds.
    bounds: Bounds,
    /// Per-surface vertex counts.
    counts: Vec<usize>,
}

fn finite(bounds: &Bounds) -> bool {
    [
        bounds.min.x,
        bounds.min.y,
        bounds.min.z,
        bounds.max.x,
        bounds.max.y,
        bounds.max.z,
    ]
    .iter()
    .all(|value| value.is_finite())
        && bounds.min.x <= bounds.max.x
        && bounds.min.y <= bounds.max.y
        && bounds.min.z <= bounds.max.z
}

fn frame_envelope(model: &SceneMd3, index: usize) -> Option<FrameEnvelope> {
    let mut bounds = empty_bounds();
    let mut counts = Vec::with_capacity(model.surfaces.len());
    for surface in &model.surfaces {
        let vertices = surface.frames.get(index)?;
        if vertices.len() > surface.texture_coordinates.len() {
            return None;
        }
        counts.push(vertices.len());
        for vertex in vertices {
            let point = vertex.position;
            if ![point.x, point.y, point.z].iter().all(|value| value.is_finite()) {
                return None;
            }
            bounds = add_point_to_bounds(bounds, point);
        }
    }
    finite(&bounds).then_some(FrameEnvelope { bounds, counts })
}

/// Cached per-frame envelopes for one MD3 model instance.
#[derive(Debug, Default)]
pub struct Md3EnvelopeCache {
    /// Cached envelopes by frame index (`None` marks a failed frame).
    frames: HashMap<usize, Option<FrameEnvelope>>,
}

impl Md3EnvelopeCache {
    /// Empty cache.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    fn envelope(&mut self, model: &SceneMd3, index: usize) -> Option<FrameEnvelope> {
        if let Some(cached) = self.frames.get(&index) {
            return cached.clone();
        }
        let envelope = frame_envelope(model, index);
        self.frames.insert(index, envelope.clone());
        envelope
    }

    /// World-space envelope of the interpolated pose, or `None` when any
    /// input is non-finite or the frame pair disagrees on vertex counts.
    pub fn world_envelope(
        &mut self,
        model: &SceneMd3,
        frame: usize,
        previous_frame: usize,
        back_lerp: f32,
        transform: &EntityTransform,
    ) -> Option<Bounds> {
        let current = self.envelope(model, frame)?;
        if !back_lerp.is_finite() {
            return None;
        }
        let mut bounds = current.bounds;
        if back_lerp != 0.0 {
            let previous = self.envelope(model, previous_frame)?;
            if current.counts.iter().zip(previous.counts.iter()).any(|(a, b)| a != b) {
                return None;
            }
            let back = back_lerp;
            let front = 1.0 - back;
            let old_scale = (1.0f32 / 64.0) * back;
            let new_scale = (1.0f32 / 64.0) * front;
            if !old_scale.is_finite() || !new_scale.is_finite() {
                return None;
            }
            let component = |old: f32, next: f32| (old * 64.0) * old_scale + (next * 64.0) * new_scale;
            let old_min = if old_scale < 0.0 {
                previous.bounds.max
            } else {
                previous.bounds.min
            };
            let old_max = if old_scale < 0.0 {
                previous.bounds.min
            } else {
                previous.bounds.max
            };
            let new_min = if new_scale < 0.0 {
                current.bounds.max
            } else {
                current.bounds.min
            };
            let new_max = if new_scale < 0.0 {
                current.bounds.min
            } else {
                current.bounds.max
            };
            bounds = Bounds {
                min: vec3(
                    component(old_min.x, new_min.x),
                    component(old_min.y, new_min.y),
                    component(old_min.z, new_min.z),
                ),
                max: vec3(
                    component(old_max.x, new_max.x),
                    component(old_max.y, new_max.y),
                    component(old_max.z, new_max.z),
                ),
            };
        }
        if !finite(&bounds) {
            return None;
        }
        let world = model_world_bounds(transform, &bounds);
        finite(&world).then_some(world)
    }
}

/// World-space envelope without a retained cache.
#[must_use]
pub fn md3_world_envelope(
    model: &SceneMd3,
    frame: usize,
    previous_frame: usize,
    back_lerp: f32,
    transform: &EntityTransform,
) -> Option<Bounds> {
    Md3EnvelopeCache::new().world_envelope(model, frame, previous_frame, back_lerp, transform)
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_content::md3::Md3Vertex;
    use qa_content::q3scene::{SceneMd3Frame, SceneMd3Surface};
    use qa_core::math::{vec2, vec3};

    fn test_model() -> SceneMd3 {
        let frame = |offset: f32| {
            vec![
                Md3Vertex {
                    position: vec3(offset, 0.0, 0.0),
                    normal: vec3(0.0, 0.0, 1.0),
                },
                Md3Vertex {
                    position: vec3(offset + 2.0, 0.0, 0.0),
                    normal: vec3(0.0, 0.0, 1.0),
                },
            ]
        };
        SceneMd3 {
            name: "test".to_string(),
            source_model: qa_content::md3::Md3Model {
                name: "test".to_string(),
                flags: 0,
                skin_count: 0,
                frames: Vec::new(),
                tags: Vec::new(),
                surfaces: Vec::new(),
            },
            frames: vec![
                SceneMd3Frame {
                    name: "a".to_string(),
                    bounds: qa_core::math::Bounds {
                        min: vec3(0.0, 0.0, 0.0),
                        max: vec3(2.0, 0.0, 0.0),
                    },
                    local_origin: vec3(0.0, 0.0, 0.0),
                    radius: 2.0,
                },
                SceneMd3Frame {
                    name: "b".to_string(),
                    bounds: qa_core::math::Bounds {
                        min: vec3(4.0, 0.0, 0.0),
                        max: vec3(6.0, 0.0, 0.0),
                    },
                    local_origin: vec3(0.0, 0.0, 0.0),
                    radius: 2.0,
                },
            ],
            tags: vec![Vec::new(), Vec::new()],
            surfaces: vec![SceneMd3Surface {
                name: "surface".to_string(),
                shaders: vec!["shader".to_string()],
                texture_coordinates: vec![vec2(0.0, 0.0), vec2(1.0, 0.0)],
                indices: vec![0, 1, 0],
                frames: vec![frame(0.0), frame(4.0)],
            }],
        }
    }

    #[test]
    fn current_frame_envelope_covers_vertices() {
        let model = test_model();
        let bounds = md3_world_envelope(&model, 0, 0, 0.0, &EntityTransform::identity()).expect("bounds");
        assert_eq!(bounds.min.x, 0.0);
        assert_eq!(bounds.max.x, 2.0);
    }

    #[test]
    fn interpolated_envelope_covers_both_frames() {
        let model = test_model();
        let bounds = md3_world_envelope(&model, 1, 0, 0.5, &EntityTransform::identity()).expect("bounds");
        assert!(bounds.min.x <= 2.0);
        assert!(bounds.max.x >= 4.0);
    }

    #[test]
    fn world_offset_applies() {
        let model = test_model();
        let transform = EntityTransform {
            origin: vec3(10.0, 0.0, 0.0),
            ..EntityTransform::identity()
        };
        let bounds = md3_world_envelope(&model, 0, 0, 0.0, &transform).expect("bounds");
        assert_eq!(bounds.min.x, 10.0);
    }

    #[test]
    fn unknown_frame_returns_none() {
        let model = test_model();
        assert!(md3_world_envelope(&model, 7, 7, 0.0, &EntityTransform::identity()).is_none());
    }

    #[test]
    fn cache_reuses_envelopes() {
        let model = test_model();
        let mut cache = Md3EnvelopeCache::new();
        let first = cache.world_envelope(&model, 0, 0, 0.0, &EntityTransform::identity());
        let second = cache.world_envelope(&model, 0, 0, 0.0, &EntityTransform::identity());
        assert_eq!(first, second);
        assert_eq!(cache.frames.len(), 1);
    }

    #[test]
    fn mismatched_counts_return_none() {
        let mut model = test_model();
        model.surfaces[0].frames[1].push(Md3Vertex {
            position: vec3(9.0, 0.0, 0.0),
            normal: vec3(0.0, 0.0, 1.0),
        });
        model.surfaces[0].texture_coordinates.push(vec2(0.5, 0.0));
        assert!(md3_world_envelope(&model, 1, 0, 0.5, &EntityTransform::identity()).is_none());
    }
}
