//! Q1 scene fog application.
//!
//! Donor provenance: `src/render/scene/q1-fog.ts`
//! (`fogSceneOperations`). Quakespasm applies normal fog after
//! texture/lightmap composition, and black fog for additive
//! contributions. GPL-2.0-or-later.

use qa_core::math::{vec3, Vec3};

use crate::render::types::{BatchFog, BlendFactor, DrawBatch, FogEffect, RenderOperation, SceneFog};

/// Apply Q1 depth fog to batches that do not already carry fog.
#[must_use]
pub fn fog_scene_operations(operations: Vec<RenderOperation>, fog: &SceneFog) -> Vec<RenderOperation> {
    let SceneFog::Q1 { color, density, .. } = fog else {
        return operations;
    };
    if *density <= 0.0 {
        return operations;
    }
    operations
        .into_iter()
        .map(|operation| match operation {
            RenderOperation::Draw(batches) => RenderOperation::Draw(apply_to_batches(batches, color, *density)),
            RenderOperation::ObjectOpacity { opacity, batches } => RenderOperation::ObjectOpacity {
                opacity,
                batches: apply_to_batches(batches, color, *density),
            },
            other => other,
        })
        .collect()
}

/// Fog every batch that has none, black for additive contributions.
fn apply_to_batches(batches: Vec<DrawBatch>, color: &Vec3, density: f32) -> Vec<DrawBatch> {
    batches
        .into_iter()
        .map(|batch| {
            if batch.fog.is_some() {
                return batch;
            }
            let fog_color = if batch.state.blend.1 == BlendFactor::One {
                vec3(0.0, 0.0, 0.0)
            } else {
                *color
            };
            DrawBatch {
                fog: Some(BatchFog::Exp2 {
                    color: fog_color,
                    density,
                    effect: FogEffect::Color,
                }),
                ..batch
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use qa_core::math::{Vec2, Vec4};

    use crate::render::types::{BatchLighting, BatchPrimitive, BatchVertices, CullFace, RenderState, RenderVertex};

    use super::*;

    fn batch_with_blend(second: BlendFactor) -> DrawBatch {
        DrawBatch {
            fog: None,
            luminance_alpha: false,
            indices: vec![],
            texture: crate::render::types::TextureBinding::RetainCurrentTexture,
            state: RenderState {
                blend: (BlendFactor::SrcAlpha, second),
                ..RenderState::opaque(CullFace::Back)
            },
            lighting: BatchLighting::Vertex,
            primitive: BatchPrimitive::Triangles,
            vertices: BatchVertices::Single(vec![RenderVertex {
                position: Vec4 {
                    x: 0.0,
                    y: 0.0,
                    z: 0.0,
                    w: 1.0,
                },
                tex_coord: Vec2 { x: 0.0, y: 0.0 },
                color: Vec4 {
                    x: 1.0,
                    y: 1.0,
                    z: 1.0,
                    w: 1.0,
                },
            }]),
        }
    }

    fn q1_fog() -> SceneFog {
        SceneFog::Q1 {
            color: vec3(0.5, 0.5, 0.5),
            density: 0.02,
            sky_factor: 1.0,
        }
    }

    #[test]
    fn empty_operations_pass_through() {
        assert_eq!(fog_scene_operations(vec![], &q1_fog()), vec![]);
    }

    #[test]
    fn zero_density_and_non_q1_pass_through() {
        let operations = vec![RenderOperation::Draw(vec![batch_with_blend(BlendFactor::Zero)])];
        let still = SceneFog::Q1 {
            color: vec3(1.0, 0.0, 0.0),
            density: 0.0,
            sky_factor: 1.0,
        };
        assert_eq!(fog_scene_operations(operations.clone(), &still), operations);
        assert_eq!(fog_scene_operations(operations.clone(), &SceneFog::None), operations);
    }

    #[test]
    fn additive_batches_get_black_fog() {
        let operations = vec![RenderOperation::Draw(vec![batch_with_blend(BlendFactor::One)])];
        let fogged = fog_scene_operations(operations, &q1_fog());
        let RenderOperation::Draw(batches) = &fogged[0] else {
            panic!("expected a draw operation");
        };
        assert_eq!(
            batches[0].fog,
            Some(BatchFog::Exp2 {
                color: vec3(0.0, 0.0, 0.0),
                density: 0.02,
                effect: FogEffect::Color,
            })
        );
    }

    #[test]
    fn normal_batches_get_fog_color() {
        let operations = vec![RenderOperation::ObjectOpacity {
            opacity: 0.5,
            batches: vec![batch_with_blend(BlendFactor::Zero)],
        }];
        let fogged = fog_scene_operations(operations, &q1_fog());
        let RenderOperation::ObjectOpacity { batches, opacity } = &fogged[0] else {
            panic!("expected an object-opacity operation");
        };
        assert_eq!(*opacity, 0.5);
        assert_eq!(
            batches[0].fog,
            Some(BatchFog::Exp2 {
                color: vec3(0.5, 0.5, 0.5),
                density: 0.02,
                effect: FogEffect::Color,
            })
        );
    }

    #[test]
    fn existing_fog_is_kept() {
        let mut batch = batch_with_blend(BlendFactor::One);
        batch.fog = Some(BatchFog::Constant {
            color: vec3(1.0, 0.0, 0.0),
            amount: 0.25,
        });
        let operations = vec![RenderOperation::Draw(vec![batch])];
        let fogged = fog_scene_operations(operations.clone(), &q1_fog());
        assert_eq!(fogged, operations);
    }

    #[test]
    fn non_draw_operations_pass_through() {
        let operations = vec![
            RenderOperation::DepthRange([0.0, 1.0]),
            RenderOperation::Cull(CullFace::None),
        ];
        assert_eq!(fog_scene_operations(operations.clone(), &q1_fog()), operations);
    }
}
