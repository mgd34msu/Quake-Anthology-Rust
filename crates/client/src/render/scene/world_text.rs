//! World-space text batches.
//!
//! Donor provenance: `src/render/scene/world-text.ts`
//! (`prepareWorldText`). Each visible glyph becomes one triangulated batch.
//! Font atlas pictures are headless handles in this workspace, so the plain
//! entry point retains the caller's bound atlas texture; use
//! [`prepare_world_text_with_images`] when concrete atlas images are known.

use qa_core::math::{add3, angles_to_axis, dot3, scale3, sub3, vec2};

use crate::render::types::{
    AlphaTest, BatchLighting, BatchPrimitive, BatchVertices, BlendFactor, CullFace, DepthTest, DrawBatch, RenderState,
    RenderVertex, TextureBinding,
};
use crate::text::atlas::{glyph_uv, resolve_text_glyph, TextFontSelection};
use crate::text::ui_world::{WorldText, WorldTextFont, WorldTextOrientation};
use crate::view::{project_point, SceneCamera};

fn prepare_core(
    texts: &[WorldText],
    camera: &SceneCamera,
    font_for: &dyn Fn(&WorldText) -> TextFontSelection,
    image_for: Option<&dyn Fn(u32) -> crate::render::types::RendererImage>,
    distance_cull_factor: Option<f32>,
) -> Vec<DrawBatch> {
    let mut batches = Vec::new();
    for text in texts {
        if let Some(text_factor) = text.input.distance_cull_factor {
            let factor = distance_cull_factor.unwrap_or(text_factor);
            let forward = dot3(sub3(text.input.origin, camera.origin), camera.axis[0]);
            if text.input.cell_size < forward * factor {
                continue;
            }
        }
        let selected = font_for(text);
        let font = if text.input.font == WorldTextFont::Classic {
            TextFontSelection::Classic {
                classic: selected.classic().clone(),
                unicode: None,
            }
        } else {
            selected
        };
        let axis = match text.input.orientation {
            WorldTextOrientation::Billboard => camera.axis,
            WorldTextOrientation::Fixed { angles } => angles_to_axis(angles),
        };
        let right = scale3(axis[1], -text.input.cell_size);
        let down = scale3(axis[2], -text.input.cell_size);
        for (row, line) in text.input.text.split('\n').enumerate() {
            let characters: Vec<char> = line.chars().collect();
            for (column, character) in characters.iter().enumerate() {
                let glyph = match resolve_text_glyph(&font, *character as u32, false) {
                    Ok(glyph) if glyph.visible => glyph,
                    _ => continue,
                };
                let uv = glyph_uv(&glyph);
                let mut corners = Vec::with_capacity(4);
                let mut failed = false;
                for (x, y) in [(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)] {
                    #[allow(clippy::cast_precision_loss)]
                    let point = add3(
                        text.input.origin,
                        add3(
                            scale3(right, column as f32 - characters.len() as f32 * 0.5 + x),
                            scale3(down, row as f32 + y),
                        ),
                    );
                    match project_point(camera, None, point) {
                        Ok(position) => corners.push(position),
                        Err(_) => {
                            failed = true;
                            break;
                        }
                    }
                }
                if failed {
                    continue;
                }
                let texture = match image_for {
                    Some(resolve) => TextureBinding::BindImage(resolve(glyph.atlas.picture.image)),
                    None => TextureBinding::RetainCurrentTexture,
                };
                batches.push(DrawBatch {
                    fog: None,
                    luminance_alpha: false,
                    indices: vec![0, 1, 2, 0, 2, 3],
                    texture,
                    state: RenderState {
                        blend: (BlendFactor::SrcAlpha, BlendFactor::OneMinusSrcAlpha),
                        depth_test: if text.input.depth_test {
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
                    primitive: BatchPrimitive::Triangles,
                    vertices: BatchVertices::Single(vec![
                        RenderVertex {
                            position: corners[0],
                            tex_coord: vec2(uv.s, uv.t),
                            color: text.input.color,
                        },
                        RenderVertex {
                            position: corners[1],
                            tex_coord: vec2(uv.s2, uv.t),
                            color: text.input.color,
                        },
                        RenderVertex {
                            position: corners[2],
                            tex_coord: vec2(uv.s2, uv.t2),
                            color: text.input.color,
                        },
                        RenderVertex {
                            position: corners[3],
                            tex_coord: vec2(uv.s, uv.t2),
                            color: text.input.color,
                        },
                    ]),
                });
            }
        }
    }
    batches
}

/// Project world-space text into one batch per visible glyph.
///
/// Atlas pictures are headless handles, so batches retain the currently
/// bound atlas texture.
pub fn prepare_world_text(
    texts: &[WorldText],
    camera: &SceneCamera,
    font_for: &dyn Fn(&WorldText) -> TextFontSelection,
    distance_cull_factor: Option<f32>,
) -> Vec<DrawBatch> {
    prepare_core(texts, camera, font_for, None, distance_cull_factor)
}

/// Project world-space text, binding each glyph batch to its atlas image via
/// `image_for` (called with the atlas picture handle).
pub fn prepare_world_text_with_images(
    texts: &[WorldText],
    camera: &SceneCamera,
    font_for: &dyn Fn(&WorldText) -> TextFontSelection,
    image_for: &dyn Fn(u32) -> crate::render::types::RendererImage,
    distance_cull_factor: Option<f32>,
) -> Vec<DrawBatch> {
    prepare_core(texts, camera, font_for, Some(image_for), distance_cull_factor)
}

#[cfg(test)]
mod tests {
    use qa_core::identity::IdentityOwner;
    use qa_core::math::{vec3, vec4};

    use crate::render::types::{fresh_owner_identity, ImageSource, RendererImage, ResourceOwner};
    use crate::text::atlas::{classic_charset, TextAtlas};
    use crate::text::ui_world::WorldTextInput;
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

    fn atlas() -> TextAtlas {
        classic_charset(7, 128, 128, "conchars", true).unwrap()
    }

    fn text(content: u32, body: &str) -> WorldText {
        WorldText {
            input: WorldTextInput {
                text: body.to_string(),
                origin: vec3(10.0, 0.0, 0.0),
                color: vec4(1.0, 1.0, 1.0, 1.0),
                cell_size: 8.0,
                distance_cull_factor: None,
                orientation: WorldTextOrientation::Billboard,
                depth_test: true,
                font: WorldTextFont::Classic,
            },
            content,
        }
    }

    #[test]
    fn empty_emits_nothing() {
        let camera = test_camera();
        let font = atlas();
        let font_for = |_: &WorldText| TextFontSelection::Classic {
            classic: font.clone(),
            unicode: None,
        };
        assert!(prepare_world_text(&[], &camera, &font_for, None).is_empty());
        let spaces = prepare_world_text(&[text(1, "  ")], &camera, &font_for, None);
        assert!(spaces.is_empty());
    }

    #[test]
    fn distance_cull_skips_far_text() {
        let camera = test_camera();
        let font = atlas();
        let font_for = |_: &WorldText| TextFontSelection::Classic {
            classic: font.clone(),
            unicode: None,
        };
        let mut far = text(1, "hi");
        far.input.origin = vec3(1000.0, 0.0, 0.0);
        far.input.distance_cull_factor = Some(0.1);
        assert!(prepare_world_text(&[far.clone()], &camera, &font_for, None).is_empty());
        far.input.distance_cull_factor = None;
        assert_eq!(prepare_world_text(&[far], &camera, &font_for, None).len(), 2);
        let mut near = text(2, "hi");
        near.input.distance_cull_factor = Some(0.1);
        assert_eq!(prepare_world_text(&[near], &camera, &font_for, None).len(), 2);
    }

    #[test]
    fn basic_text_emits_glyph_batches() {
        let camera = test_camera();
        let font = atlas();
        let font_for = |_: &WorldText| TextFontSelection::Classic {
            classic: font.clone(),
            unicode: None,
        };
        let batches = prepare_world_text(&[text(1, "a\nb")], &camera, &font_for, None);
        assert_eq!(batches.len(), 2);
        for batch in &batches {
            assert_eq!(batch.indices, vec![0, 1, 2, 0, 2, 3]);
            assert_eq!(batch.primitive, BatchPrimitive::Triangles);
            assert_eq!(batch.lighting, BatchLighting::Vertex);
            assert_eq!(
                batch.state.blend,
                (BlendFactor::SrcAlpha, BlendFactor::OneMinusSrcAlpha)
            );
            assert_eq!(batch.state.depth_test, DepthTest::LessEqual);
            assert!(!batch.state.depth_write);
            let BatchVertices::Single(vertices) = &batch.vertices else {
                panic!("glyph batch must be single-textured");
            };
            assert_eq!(vertices.len(), 4);
        }
        let BatchVertices::Single(first) = &batches[0].vertices else {
            unreachable!();
        };
        let BatchVertices::Single(second) = &batches[1].vertices else {
            unreachable!();
        };
        assert_ne!(first[0].tex_coord, second[0].tex_coord);
    }

    #[test]
    fn fixed_orientation_and_image_binding() {
        let camera = test_camera();
        let font = atlas();
        let font_for = |_: &WorldText| TextFontSelection::Classic {
            classic: font.clone(),
            unicode: None,
        };
        let authority = IdentityOwner::create("world-text-test").unwrap();
        let session = authority.session().clone();
        let image_for = |handle: u32| RendererImage {
            owner: ResourceOwner::new(fresh_owner_identity(), session.clone(), 0),
            ordinal: handle,
            source: ImageSource::Generated {
                name: "atlas".to_string(),
            },
            width: 128,
            height: 128,
        };
        let mut fixed = text(1, "z");
        fixed.input.orientation = WorldTextOrientation::Fixed {
            angles: vec3(0.0, 90.0, 0.0),
        };
        fixed.input.depth_test = false;
        let batches = prepare_world_text_with_images(&[fixed], &camera, &font_for, &image_for, None);
        assert_eq!(batches.len(), 1);
        assert_eq!(batches[0].state.depth_test, DepthTest::Always);
        let TextureBinding::BindImage(image) = &batches[0].texture else {
            panic!("image resolver must bind the atlas image");
        };
        assert_eq!(image.ordinal, 7);
    }
}
