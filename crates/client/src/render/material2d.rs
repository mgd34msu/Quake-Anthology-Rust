//! Material-backed 2D pictures: fonts and menu art drawn through registered
//! shader stages.
//!
//! Donor provenance: `src/render/commands/material2d.ts`
//! (`prepareMaterialText`). The local text layer carries no compiled shader
//! on its material pictures, so 2D material draws arrive as the
//! renderer-local [`MaterialTextDraw`]. The clipped quad evaluates through
//! [`prepare_material_batches`](crate::materials::evaluate::prepare_material_batches)
//! with the viewport projection, no fog volume, and 2D state overrides
//! (depth test always passes, depth writes off, no culling).

use qa_core::math::{vec2, vec3, Vec3, Vec4};

use super::frame::clip_picture;
use super::types::{Rect, TextureRect};
use crate::materials::compile::CompiledMaterial;
use crate::materials::evaluate::{prepare_material_batches, MaterialBatch, MaterialDrawContext};
use crate::materials::geometry::{MaterialGeometry, MaterialVertex};
use crate::materials::material::normalize_shader_name;
use crate::materials::state::{CullFace, DepthTest};
use crate::render::scene::material_registrations::{current_remap, snapshot_materials};
use crate::text::draw2d::TextureRect as TextTextureRect;
use crate::ClientError;

/// Renderer-local mirror of the donor `MaterialTextDraw`: destination, art,
/// and the compiled shader the text layer selected. The donor's seat plays
/// no role in batch preparation and is not carried.
#[derive(Debug, Clone)]
pub struct MaterialTextDraw<'a> {
    /// Destination rectangle in viewport coordinates.
    pub rect: Rect,
    /// Source texture coordinates.
    pub uv: TextTextureRect,
    /// Draw color (normalized).
    pub color: Vec4,
    /// Compiled shader for the picture.
    pub compiled: &'a CompiledMaterial,
}

fn byte(component: f32) -> u8 {
    (component * 255.0).round().clamp(0.0, 255.0) as u8
}

fn quad_vertex(x: f32, y: f32, s: f32, t: f32, color: [u8; 4]) -> MaterialVertex {
    MaterialVertex::new(vec3(x, y, 0.0), vec3(0.0, 0.0, 1.0), vec2(s, t), vec2(0.0, 0.0), color)
}

/// Resolve the effective shader: the remap replacement when one is published
/// and admitted, else the draw's own shader. The remap clock offset applies
/// whenever a remap exists.
fn effective_shader<'a>(
    compiled: &'a CompiledMaterial,
    admitted: &'a [crate::render::scene::material_registrations::RegisteredSceneMaterial],
) -> (&'a CompiledMaterial, f32) {
    let Some(remap) = current_remap(&compiled.registered.definition.name) else {
        return (compiled, 0.0);
    };
    let replacement = admitted.iter().find(|material| {
        normalize_shader_name(&material.registered.definition.name) == normalize_shader_name(&remap.material)
    });
    (
        replacement.map_or(compiled, |material| &material.compiled),
        remap.time_offset,
    )
}

/// Prepare 2D batches for a material picture. Returns no batches when the
/// quad clips away.
///
/// # Errors
///
/// Returns [`ClientError`] when stage evaluation fails.
pub fn prepare_material_text(
    draw: &MaterialTextDraw,
    viewport: &Rect,
    context: &MaterialDrawContext<'_>,
) -> Result<Vec<MaterialBatch>, ClientError> {
    let uv = TextureRect {
        s1: draw.uv.s,
        t1: draw.uv.t,
        s2: draw.uv.s2,
        t2: draw.uv.t2,
    };
    let Some((rect, uv)) = clip_picture(&draw.rect, &uv, viewport) else {
        return Ok(Vec::new());
    };
    let right = rect.x + rect.width;
    let bottom = rect.y + rect.height;
    let color = [
        byte(draw.color.x),
        byte(draw.color.y),
        byte(draw.color.z),
        byte(draw.color.w),
    ];
    let geometry = MaterialGeometry {
        vertices: vec![
            quad_vertex(rect.x, rect.y, uv.s1, uv.t1, color),
            quad_vertex(right, rect.y, uv.s2, uv.t1, color),
            quad_vertex(right, bottom, uv.s2, uv.t2, color),
            quad_vertex(rect.x, bottom, uv.s1, uv.t2, color),
        ],
        indices: vec![0, 1, 2, 0, 2, 3],
    };
    let admitted = snapshot_materials();
    let (compiled, offset) = effective_shader(draw.compiled, &admitted);
    let project = |position: Vec3| Vec4 {
        x: 2.0 * (position.x - viewport.x) / viewport.width - 1.0,
        y: 1.0 - 2.0 * (position.y - viewport.y) / viewport.height,
        z: 0.0,
        w: 1.0,
    };
    let child = MaterialDrawContext {
        time: context.time,
        time_offset: context.time_offset + offset,
        refdef_time: context.refdef_time,
        identity_light: context.identity_light,
        entity_rgba: color,
        lighting: context.lighting,
        view_origin: context.view_origin,
        local_view_origin: context.local_view_origin,
        noise: context.noise,
        shader_tex_coord: context.shader_tex_coord,
        deform_view: context.deform_view,
        projection_shadow: context.projection_shadow,
        render_text: context.render_text.clone(),
        dynamic_lights: context.dynamic_lights.clone(),
        dynamic_light_batches: context.dynamic_light_batches,
        depth_range: context.depth_range,
        polygon_offset: context.polygon_offset,
        q1_fog: context.q1_fog,
        fog: None,
        project: &project,
    };
    let mut batches = prepare_material_batches(compiled, geometry, &child)?;
    for batch in &mut batches {
        batch.state.depth_test = DepthTest::Always;
        batch.state.depth_write = false;
        batch.state.cull = CullFace::None;
    }
    Ok(batches)
}

#[cfg(test)]
mod tests {
    use qa_core::math::{vec3, vec4};

    use super::*;
    use crate::materials::compile::{
        compile_shader_script, CompileOptions, RegisteredImage, ShaderRegistrationHost, SourceImageRequest,
    };
    use crate::materials::deform::{DeformView, RendererNoise};
    use crate::materials::evaluate::TextureRef;
    use crate::render::scene::material_registrations::{
        admit_material, lock_remap_tests, publish_remap, remove_remap, MaterialRemap,
    };

    struct Host;

    impl ShaderRegistrationHost for Host {
        fn white_image(&self) -> RegisteredImage {
            RegisteredImage { image: 1, tmu: 0 }
        }

        fn default_image(&self) -> RegisteredImage {
            RegisteredImage { image: 2, tmu: 0 }
        }

        fn lightmap_image(&self) -> RegisteredImage {
            RegisteredImage { image: 3, tmu: 1 }
        }

        fn find_image(&mut self, request: &SourceImageRequest) -> Option<RegisteredImage> {
            let image = if request.name.contains("replacement") { 9 } else { 4 };
            Some(RegisteredImage { image, tmu: 0 })
        }

        fn play_shader_cinematic(&mut self, _name: &str) -> Option<crate::materials::compile::RegisteredShaderVideo> {
            None
        }

        fn apply_sun(&mut self, _sun: crate::materials::material::RegisteredSun) {}

        fn initialize_sky_tex_coords(&mut self, _height: f32) {}

        fn print_warning(&mut self, _message: &str) {}
    }

    fn compile(name: &str, texture: &str) -> CompiledMaterial {
        let mut host = Host;
        compile_shader_script(
            &format!("{name}\n{{\n {{\n map {texture}\n }}\n}}\n"),
            &mut host,
            "<test>",
            &CompileOptions::default(),
        )
        .unwrap()
        .remove(0)
    }

    fn viewport() -> Rect {
        Rect {
            x: 0.0,
            y: 0.0,
            width: 640.0,
            height: 480.0,
        }
    }

    fn draw<'a>(compiled: &'a CompiledMaterial) -> MaterialTextDraw<'a> {
        MaterialTextDraw {
            rect: Rect {
                x: 320.0,
                y: 240.0,
                width: 64.0,
                height: 64.0,
            },
            uv: TextTextureRect {
                s: 0.0,
                t: 0.0,
                s2: 1.0,
                t2: 1.0,
            },
            color: vec4(1.0, 1.0, 1.0, 1.0),
            compiled,
        }
    }

    struct Fixture {
        noise: RendererNoise,
        project: Box<dyn Fn(Vec3) -> Vec4>,
    }

    fn fixture() -> Fixture {
        Fixture {
            noise: RendererNoise::new(),
            project: Box::new(|position: Vec3| vec4(position.x, position.y, position.z, 1.0)),
        }
    }

    fn context<'a>(fixture: &'a Fixture) -> MaterialDrawContext<'a> {
        MaterialDrawContext {
            time: 0.0,
            time_offset: 0.0,
            refdef_time: 0.0,
            identity_light: 1.0,
            entity_rgba: [255, 255, 255, 255],
            lighting: None,
            view_origin: vec3(0.0, 0.0, 0.0),
            local_view_origin: vec3(0.0, 0.0, 0.0),
            noise: &fixture.noise,
            shader_tex_coord: vec2(0.0, 0.0),
            deform_view: DeformView {
                axis: [vec3(1.0, 0.0, 0.0), vec3(0.0, 1.0, 0.0), vec3(0.0, 0.0, 1.0)],
                mirror: false,
                entity_axis: None,
                non_normalized_axis: None,
            },
            projection_shadow: None,
            render_text: Vec::new(),
            dynamic_lights: None,
            dynamic_light_batches: None,
            depth_range: [0.0, 1.0],
            polygon_offset: None,
            q1_fog: None,
            fog: None,
            project: &*fixture.project,
        }
    }

    #[test]
    fn valid_quad_yields_overridden_batches() {
        let compiled = compile("material2d-valid", "textures/rock.tga");
        let fixture = fixture();
        let batches = prepare_material_text(&draw(&compiled), &viewport(), &context(&fixture)).unwrap();
        assert_eq!(batches.len(), 1);
        assert_eq!(batches[0].texture, TextureRef::BindImage(4));
        assert_eq!(batches[0].vertices.len(), 4);
        assert_eq!(batches[0].state.depth_test, DepthTest::Always);
        assert!(!batches[0].state.depth_write);
        assert_eq!(batches[0].state.cull, CullFace::None);
        let corners: Vec<(f32, f32)> = batches[0]
            .vertices
            .iter()
            .map(|vertex| (vertex.position.x, vertex.position.y))
            .collect();
        for (actual, expected) in
            corners
                .iter()
                .zip([(0.0, 0.0), (0.2, 0.0), (0.2, -0.266_666_68), (0.0, -0.266_666_68)])
        {
            assert!((actual.0 - expected.0).abs() < 1e-6, "{actual:?}");
            assert!((actual.1 - expected.1).abs() < 1e-6, "{actual:?}");
        }
    }

    #[test]
    fn clipped_away_quad_yields_no_batches() {
        let compiled = compile("material2d-clipped", "textures/rock.tga");
        let fixture = fixture();
        let mut input = draw(&compiled);
        input.rect.x = 700.0;
        let batches = prepare_material_text(&input, &viewport(), &context(&fixture)).unwrap();
        assert!(batches.is_empty());
    }

    #[test]
    fn empty_quad_yields_no_batches() {
        let compiled = compile("material2d-empty", "textures/rock.tga");
        let fixture = fixture();
        let mut input = draw(&compiled);
        input.rect.width = 0.0;
        let batches = prepare_material_text(&input, &viewport(), &context(&fixture)).unwrap();
        assert!(batches.is_empty());
    }

    #[test]
    fn partial_clip_keeps_visible_span() {
        let compiled = compile("material2d-partial", "textures/rock.tga");
        let fixture = fixture();
        let mut input = draw(&compiled);
        input.rect.x = 600.0;
        let batches = prepare_material_text(&input, &viewport(), &context(&fixture)).unwrap();
        assert_eq!(batches.len(), 1);
        let left = batches[0].vertices[0].position.x;
        let right = batches[0].vertices[1].position.x;
        assert!((left - (2.0 * 600.0 / 640.0 - 1.0)).abs() < 1e-6);
        assert!((right - 1.0).abs() < 1e-6);
        assert!((batches[0].vertices[0].tex_coord.x - 0.0).abs() < 1e-6);
        assert!((batches[0].vertices[1].tex_coord.x - 0.625).abs() < 1e-6);
    }

    #[test]
    fn remap_switches_to_the_admitted_replacement() {
        let _remap_lock = lock_remap_tests();
        let original = compile("material2d-remap-src", "textures/rock.tga");
        let replacement = compile("material2d-remap-dst", "textures/replacement.tga");
        admit_material(replacement);
        let fixture = fixture();
        // The remap table is process-global; retry the publish/prepare
        // window until the replacement is observed.
        let mut batches = Vec::new();
        for _ in 0..50 {
            publish_remap(
                "material2d-remap-src",
                MaterialRemap {
                    material: "material2d-remap-dst".to_string(),
                    time_offset: 0.5,
                },
            );
            batches = prepare_material_text(&draw(&original), &viewport(), &context(&fixture)).unwrap();
            if batches.first().map(|batch| &batch.texture) == Some(&TextureRef::BindImage(9)) {
                break;
            }
        }
        remove_remap("material2d-remap-src");
        assert_eq!(batches.len(), 1);
        assert_eq!(batches[0].texture, TextureRef::BindImage(9));
    }

    #[test]
    fn dangling_remap_falls_back_to_original() {
        let _remap_lock = lock_remap_tests();
        let original = compile("material2d-dangling", "textures/rock.tga");
        publish_remap(
            "material2d-dangling",
            MaterialRemap {
                material: "material2d-never-admitted".to_string(),
                time_offset: 1.0,
            },
        );
        let fixture = fixture();
        let batches = prepare_material_text(&draw(&original), &viewport(), &context(&fixture)).unwrap();
        remove_remap("material2d-dangling");
        assert_eq!(batches.len(), 1);
        assert_eq!(batches[0].texture, TextureRef::BindImage(4));
    }
}
