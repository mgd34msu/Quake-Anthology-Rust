//! Debug line batches.
//!
//! Donor provenance: `src/render/scene/debug-shapes.ts`
//! (`prepareDebugShapes`). Lines group into runs by depth-test flag so each
//! batch keeps one depth function.

use qa_core::math::{vec2, Vec3, Vec4};

use crate::render::error::RenderError;
use crate::render::types::{
    AlphaTest, BatchLighting, BatchPrimitive, BatchVertices, BlendFactor, CullFace, DepthTest, DrawBatch, RenderState,
    RenderVertex, RendererImage, TextureBinding,
};
use crate::view::{project_point, SceneCamera};

/// One debug line segment.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DebugLine {
    /// Segment start in world space.
    pub start: Vec3,
    /// Segment end in world space.
    pub end: Vec3,
    /// Normalized line color.
    pub color: Vec4,
    /// Whether the segment tests against the depth buffer.
    pub depth_test: bool,
}

fn flush_run(
    batches: &mut Vec<DrawBatch>,
    vertices: &mut Vec<RenderVertex>,
    indices: &mut Vec<u32>,
    depth_test: bool,
    white_image: &RendererImage,
    line_width: f32,
) {
    if vertices.is_empty() {
        return;
    }
    batches.push(DrawBatch {
        fog: None,
        luminance_alpha: false,
        indices: std::mem::take(indices),
        texture: TextureBinding::BindImage(white_image.clone()),
        state: RenderState {
            blend: (BlendFactor::SrcAlpha, BlendFactor::OneMinusSrcAlpha),
            depth_test: if depth_test {
                DepthTest::LessEqual
            } else {
                DepthTest::Always
            },
            depth_write: false,
            alpha_test: AlphaTest::None,
            cull: CullFace::None,
            depth_range: [0.0, 1.0],
            polygon_offset: None,
        },
        lighting: BatchLighting::Vertex,
        primitive: BatchPrimitive::Lines { line_width },
        vertices: BatchVertices::Single(std::mem::take(vertices)),
    });
}

/// Project debug lines into line batches, grouped into runs by depth-test
/// flag. Rejects a non-finite or non-positive line width.
pub fn prepare_debug_shapes(
    lines: &[DebugLine],
    camera: &SceneCamera,
    white_image: &RendererImage,
    line_width: f32,
) -> Result<Vec<DrawBatch>, RenderError> {
    if !line_width.is_finite() || line_width <= 0.0 {
        return Err(RenderError::Backend("Invalid debug line width".to_string()));
    }
    let mut batches = Vec::new();
    let mut vertices: Vec<RenderVertex> = Vec::new();
    let mut indices: Vec<u32> = Vec::new();
    let mut depth_test: Option<bool> = None;
    for line in lines {
        if depth_test != Some(line.depth_test) {
            if let Some(previous) = depth_test {
                flush_run(
                    &mut batches,
                    &mut vertices,
                    &mut indices,
                    previous,
                    white_image,
                    line_width,
                );
            }
            depth_test = Some(line.depth_test);
        }
        for point in [line.start, line.end] {
            let position =
                project_point(camera, None, point).map_err(|error| RenderError::Backend(error.to_string()))?;
            indices.push(vertices.len() as u32);
            vertices.push(RenderVertex {
                position,
                tex_coord: vec2(0.0, 0.0),
                color: line.color,
            });
        }
    }
    if let Some(previous) = depth_test {
        flush_run(
            &mut batches,
            &mut vertices,
            &mut indices,
            previous,
            white_image,
            line_width,
        );
    }
    Ok(batches)
}

#[cfg(test)]
mod tests {
    use qa_core::identity::IdentityOwner;
    use qa_core::math::{vec3, vec4};

    use crate::render::types::{fresh_owner_identity, ImageSource, ResourceOwner};
    use crate::view::{CameraClip, Rect};

    use super::*;

    fn test_camera() -> SceneCamera {
        SceneCamera {
            origin: vec3(0.0, 0.0, 0.0),
            axis: [vec3(1.0, 0.0, 0.0), vec3(0.0, 1.0, 0.0), vec3(0.0, 0.0, 1.0)],
            projection: [
                1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
            ],
            viewport: Rect {
                x: 0,
                y: 0,
                width: 640,
                height: 480,
            },
            clip: CameraClip::None,
        }
    }

    fn white_image() -> RendererImage {
        let authority = IdentityOwner::create("debug-shapes-test").unwrap();
        RendererImage {
            owner: ResourceOwner::new(fresh_owner_identity(), authority.session().clone(), 0),
            ordinal: 0,
            source: ImageSource::Generated {
                name: "*white".to_string(),
            },
            width: 1,
            height: 1,
        }
    }

    fn line(depth_test: bool) -> DebugLine {
        DebugLine {
            start: vec3(0.0, 0.0, 0.0),
            end: vec3(1.0, 0.0, 0.0),
            color: vec4(1.0, 0.0, 0.0, 1.0),
            depth_test,
        }
    }

    #[test]
    fn rejects_bad_line_width() {
        let camera = test_camera();
        let white = white_image();
        for width in [0.0, -2.0, f32::NAN, f32::INFINITY] {
            assert!(
                matches!(
                    prepare_debug_shapes(&[line(true)], &camera, &white, width),
                    Err(RenderError::Backend(_))
                ),
                "width {width} must fail"
            );
        }
    }

    #[test]
    fn toggles_split_batches_with_counts() {
        let camera = test_camera();
        let white = white_image();
        let batches = prepare_debug_shapes(&[line(true), line(false)], &camera, &white, 2.0).unwrap();
        assert_eq!(batches.len(), 2);
        assert_eq!(batches[0].state.depth_test, DepthTest::LessEqual);
        assert_eq!(batches[1].state.depth_test, DepthTest::Always);
        for batch in &batches {
            assert_eq!(batch.primitive, BatchPrimitive::Lines { line_width: 2.0 });
            assert_eq!(batch.indices, vec![0, 1]);
            let BatchVertices::Single(vertices) = &batch.vertices else {
                panic!("debug batch must be single-textured");
            };
            assert_eq!(vertices.len(), 2);
            assert!(!batch.state.depth_write);
        }
        let merged = prepare_debug_shapes(&[line(true), line(true)], &camera, &white, 2.0).unwrap();
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].indices, vec![0, 1, 2, 3]);
        assert!(prepare_debug_shapes(&[], &camera, &white, 2.0).unwrap().is_empty());
    }
}
