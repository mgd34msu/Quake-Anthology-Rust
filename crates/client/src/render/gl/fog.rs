//! Q2 depth-snapshot fog passes (donor `src/render/gl/fog.ts`).

use std::collections::HashMap;

use super::fog_shader::{
    build_fog_fragment_shader_source, build_fog_vertex_shader_source, fog_uniforms_for, FogPassKind,
};
use super::programs::compile_program;
use super::{
    GlContext, ACTIVE_TEXTURE, ALPHA_TEST, BLEND, BLEND_DST, BLEND_SRC, CLAMP_TO_EDGE, CLIP_PLANE0, COLOR_CLEAR_VALUE,
    CULL_FACE_CAP, CURRENT_PROGRAM, DEPTH_COMPONENT24, DEPTH_TEST, DEPTH_WRITEMASK, NEAREST, QUADS, SCISSOR_BOX,
    SCISSOR_TEST, STENCIL_TEST, TEXTURE0, TEXTURE_2D, TEXTURE_BINDING_2D, TEXTURE_MAG_FILTER, TEXTURE_MIN_FILTER,
    TEXTURE_WRAP_S, TEXTURE_WRAP_T, VIEWPORT,
};
use crate::render::error::RenderError;
use crate::render::types::Q2FogOperation;

#[derive(Debug)]
struct FogProgram {
    name: u32,
    uniforms: HashMap<String, i32>,
}

/// Q2 global/height/sky fog over a [`GlContext`].
#[derive(Debug)]
pub struct Q2FogPass {
    programs: HashMap<FogPassKind, FogProgram>,
    texture: u32,
    closed: bool,
}

impl Q2FogPass {
    pub fn new<C: GlContext>(gl: &mut C) -> Self {
        let names = gl.gen_textures(1);
        let texture = names.first().copied().unwrap_or(0);
        if texture == 0 {
            panic!(
                "{}",
                RenderError::Backend("OpenGL could not allocate the fog depth texture".to_string())
            );
        }
        Self {
            programs: HashMap::new(),
            texture,
            closed: false,
        }
    }

    fn program<C: GlContext>(&mut self, gl: &mut C, kind: FogPassKind) -> &FogProgram {
        self.programs.entry(kind).or_insert_with(|| {
            let name = compile_program(
                gl,
                &build_fog_vertex_shader_source(kind),
                &build_fog_fragment_shader_source(kind),
            );
            let mut uniforms = HashMap::new();
            for key in fog_uniforms_for(kind) {
                let location = gl.get_uniform_location(name, key);
                if location < 0 {
                    gl.delete_program(name);
                    panic!(
                        "{}",
                        RenderError::Backend(format!("OpenGL fog uniform is missing: {key}"))
                    );
                }
                uniforms.insert((*key).to_string(), location);
            }
            FogProgram { name, uniforms }
        });
        self.programs
            .get(&kind)
            .unwrap_or_else(|| panic!("{}", RenderError::Backend("OpenGL fog program is missing".to_string())))
    }

    fn uniform_location(program: &FogProgram, name: &str) -> i32 {
        program.uniforms.get(name).copied().unwrap_or_else(|| {
            panic!(
                "{}",
                RenderError::Backend(format!("OpenGL fog uniform was not linked: {name}"))
            )
        })
    }

    pub fn draw<C: GlContext>(
        &mut self,
        gl: &mut C,
        operation: &Q2FogOperation,
        target_width: i32,
        target_height: i32,
    ) {
        if self.closed {
            panic!("{}", RenderError::Backend("OpenGL fog pass is closed".to_string()));
        }
        let fog = &operation.fog;
        if fog.density <= 0.0
            && (fog.height.density <= 0.0 || fog.height.falloff <= 0.0)
            && (fog.sky_factor <= 0.0 || !operation.sky_drawn)
        {
            return;
        }
        let rect = &operation.camera.viewport;
        let bottom = target_height as f32 - rect.y - rect.height;
        for value in [rect.x, rect.y, rect.width, rect.height] {
            if value.fract() != 0.0 {
                panic!(
                    "{}",
                    RenderError::BadViewport("OpenGL fog viewport is outside its render target".to_string())
                );
            }
        }
        let (x, y, width, height) = (rect.x as i32, rect.y as i32, rect.width as i32, rect.height as i32);
        if x < 0 || y < 0 || width < 1 || height < 1 || x + width > target_width || y + height > target_height {
            panic!(
                "{}",
                RenderError::BadViewport("OpenGL fog viewport is outside its render target".to_string())
            );
        }
        let projection = &operation.camera.projection;
        if projection[1] != 0.0
            || projection[2] != 0.0
            || projection[3] != 0.0
            || projection[4] != 0.0
            || projection[6] != 0.0
            || projection[7] != 0.0
            || projection[8] != 0.0
            || projection[9] != 0.0
            || projection[12] != 0.0
            || projection[13] != 0.0
            || projection[0] == 0.0
            || projection[5] == 0.0
            || projection[11] != -1.0
            || projection[15] != 0.0
        {
            panic!(
                "{}",
                RenderError::BadProjection("Q2 fog requires the source symmetric perspective projection".to_string())
            );
        }
        let a = -projection[10];
        let b = -projection[14];
        let tan_x = 1.0 / projection[0];
        let tan_y = 1.0 / projection[5];
        let mut one = [0];
        gl.get_integerv(CURRENT_PROGRAM, &mut one);
        let old_program = one[0] as u32;
        gl.get_integerv(ACTIVE_TEXTURE, &mut one);
        let old_active = one[0] as u32;
        let mut old_viewport = [0; 4];
        gl.get_integerv(VIEWPORT, &mut old_viewport);
        let mut old_scissor = [0; 4];
        gl.get_integerv(SCISSOR_BOX, &mut old_scissor);
        gl.get_integerv(DEPTH_WRITEMASK, &mut one);
        let old_depth_mask = one[0] != 0;
        gl.get_integerv(BLEND_SRC, &mut one);
        let old_blend_src = one[0] as u32;
        gl.get_integerv(BLEND_DST, &mut one);
        let old_blend_dst = one[0] as u32;
        let mut old_color = [0.0; 4];
        gl.get_floatv(COLOR_CLEAR_VALUE, &mut old_color);
        let enables = [
            DEPTH_TEST,
            CULL_FACE_CAP,
            ALPHA_TEST,
            STENCIL_TEST,
            CLIP_PLANE0,
            SCISSOR_TEST,
            BLEND,
        ]
        .map(|cap| (cap, gl.is_enabled(cap)));
        gl.active_texture(TEXTURE0);
        gl.get_integerv(TEXTURE_BINDING_2D, &mut one);
        let old_texture = one[0] as u32;
        gl.bind_texture(TEXTURE_2D, self.texture);
        gl.tex_parameteri(TEXTURE_2D, TEXTURE_MIN_FILTER, NEAREST);
        gl.tex_parameteri(TEXTURE_2D, TEXTURE_MAG_FILTER, NEAREST);
        gl.tex_parameteri(TEXTURE_2D, TEXTURE_WRAP_S, CLAMP_TO_EDGE);
        gl.tex_parameteri(TEXTURE_2D, TEXTURE_WRAP_T, CLAMP_TO_EDGE);
        let bottom = bottom as i32;
        gl.copy_tex_image_2d(TEXTURE_2D, 0, DEPTH_COMPONENT24, x, bottom, width, height, 0);
        gl.viewport(x, bottom, width, height);
        gl.scissor(x, bottom, width, height);
        for cap in [DEPTH_TEST, CULL_FACE_CAP, ALPHA_TEST, STENCIL_TEST, CLIP_PLANE0] {
            gl.disable(cap);
        }
        gl.enable(SCISSOR_TEST);
        gl.enable(BLEND);
        gl.blend_func(super::SRC_ALPHA, super::ONE_MINUS_SRC_ALPHA);
        gl.depth_mask(false);
        gl.color_4f(1.0, 1.0, 1.0, 1.0);
        for kind in FogPassKind::ALL {
            let skip = match kind {
                FogPassKind::Global => fog.density <= 0.0,
                FogPassKind::Height => fog.height.density <= 0.0 || fog.height.falloff <= 0.0,
                FogPassKind::Sky => fog.sky_factor <= 0.0 || !operation.sky_drawn,
            };
            if skip {
                continue;
            }
            let program = self.program(gl, kind);
            let depth = Self::uniform_location(program, "u_depth");
            let far = Self::uniform_location(program, "u_far_depth");
            let proj = if kind == FogPassKind::Sky {
                None
            } else {
                Some(Self::uniform_location(program, "u_proj"))
            };
            let extra: Vec<(String, i32)> = fog_uniforms_for(kind)
                .iter()
                .filter(|key| !["u_depth", "u_far_depth", "u_proj", "u_fog_color"].contains(key))
                .map(|key| ((*key).to_string(), Self::uniform_location(program, key)))
                .collect();
            let fog_color = if kind == FogPassKind::Height {
                None
            } else {
                Some(Self::uniform_location(program, "u_fog_color"))
            };
            gl.use_program(program.name);
            gl.uniform_1i(depth, 0);
            gl.uniform_1f(far, operation.far_depth);
            if let Some(location) = proj {
                gl.uniform_4f(location, a, b, 0.0, 0.0);
            }
            match kind {
                FogPassKind::Global => gl.uniform_4f(
                    fog_color.unwrap_or(-1),
                    fog.color.x,
                    fog.color.y,
                    fog.color.z,
                    fog.density / 64.0,
                ),
                FogPassKind::Sky => gl.uniform_4f(
                    fog_color.unwrap_or(-1),
                    fog.color.x,
                    fog.color.y,
                    fog.color.z,
                    fog.sky_factor,
                ),
                FogPassKind::Height => {
                    let axis = &operation.camera.axis;
                    let height_fog = &fog.height;
                    let uniform = |name: &str| {
                        extra
                            .iter()
                            .find_map(|(key, location)| (key == name).then_some(*location))
                            .unwrap_or_else(|| {
                                panic!(
                                    "{}",
                                    RenderError::Backend(format!("OpenGL fog uniform was not linked: {name}"))
                                )
                            })
                    };
                    gl.uniform_3f(
                        uniform("u_vieworg"),
                        operation.camera.origin.x,
                        operation.camera.origin.y,
                        operation.camera.origin.z,
                    );
                    gl.uniform_3f(uniform("u_forward"), axis[0].x, axis[0].y, axis[0].z);
                    gl.uniform_3f(uniform("u_right"), -axis[1].x, -axis[1].y, -axis[1].z);
                    gl.uniform_3f(uniform("u_up"), axis[2].x, axis[2].y, axis[2].z);
                    gl.uniform_4f(uniform("u_tan"), tan_x, tan_y, 0.0, 0.0);
                    gl.uniform_4f(
                        uniform("u_hf_start"),
                        height_fog.start.color.x,
                        height_fog.start.color.y,
                        height_fog.start.color.z,
                        height_fog.start.distance,
                    );
                    gl.uniform_4f(
                        uniform("u_hf_end"),
                        height_fog.end.color.x,
                        height_fog.end.color.y,
                        height_fog.end.color.z,
                        height_fog.end.distance,
                    );
                    gl.uniform_1f(uniform("u_hf_density"), height_fog.density);
                    gl.uniform_1f(uniform("u_hf_falloff"), height_fog.falloff);
                }
            }
            gl.begin(QUADS);
            gl.tex_coord_2f(0.0, 0.0);
            gl.vertex_2f(-1.0, -1.0);
            gl.tex_coord_2f(1.0, 0.0);
            gl.vertex_2f(1.0, -1.0);
            gl.tex_coord_2f(1.0, 1.0);
            gl.vertex_2f(1.0, 1.0);
            gl.tex_coord_2f(0.0, 1.0);
            gl.vertex_2f(-1.0, 1.0);
            gl.end();
        }
        let error = gl.get_error();
        gl.use_program(old_program);
        gl.bind_texture(TEXTURE_2D, old_texture);
        gl.active_texture(old_active);
        gl.depth_mask(old_depth_mask);
        gl.blend_func(old_blend_src, old_blend_dst);
        gl.viewport(old_viewport[0], old_viewport[1], old_viewport[2], old_viewport[3]);
        gl.scissor(old_scissor[0], old_scissor[1], old_scissor[2], old_scissor[3]);
        gl.color_4f(old_color[0], old_color[1], old_color[2], old_color[3]);
        for (cap, enabled) in enables {
            if enabled {
                gl.enable(cap);
            } else {
                gl.disable(cap);
            }
        }
        if error != 0 {
            panic!(
                "{}",
                RenderError::Backend(format!("OpenGL Q2 fog pass failed: 0x{error:x}"))
            );
        }
    }

    pub fn close<C: GlContext>(&mut self, gl: &mut C) {
        if self.closed {
            return;
        }
        for program in self.programs.values() {
            gl.delete_program(program.name);
        }
        self.programs.clear();
        gl.delete_textures(&[self.texture]);
        self.closed = true;
    }
}

#[cfg(test)]
mod tests {
    use super::super::{FakeGlContext, GlCall};
    use super::*;
    use crate::render::types::{Q2Fog, Q2HeightFog, Q2HeightStop, RenderCamera, ViewClip};
    use qa_core::math::{identity_mat4, vec3};

    fn operation() -> Q2FogOperation {
        Q2FogOperation {
            camera: RenderCamera {
                origin: vec3(0.0, 0.0, 0.0),
                axis: [vec3(1.0, 0.0, 0.0), vec3(0.0, 1.0, 0.0), vec3(0.0, 0.0, 1.0)],
                projection: identity_mat4(),
                viewport: crate::render::types::Rect {
                    x: 0.0,
                    y: 0.0,
                    width: 64.0,
                    height: 64.0,
                },
                clip: ViewClip::None,
            },
            fog: Q2Fog {
                color: vec3(0.5, 0.5, 0.5),
                density: 0.02,
                sky_factor: 0.0,
                height: Q2HeightFog {
                    start: Q2HeightStop {
                        color: vec3(0.0, 0.0, 0.0),
                        distance: 0.0,
                    },
                    end: Q2HeightStop {
                        color: vec3(1.0, 1.0, 1.0),
                        distance: 100.0,
                    },
                    density: 0.0,
                    falloff: 0.0,
                },
            },
            far_depth: 0.999,
            sky_drawn: false,
        }
    }

    fn symmetric_projection() -> [f32; 16] {
        let mut projection = [0.0f32; 16];
        projection[0] = 1.0;
        projection[5] = 1.0;
        projection[10] = -1.5;
        projection[11] = -1.0;
        projection[14] = -2.0;
        projection
    }

    #[test]
    fn draws_global_pass_and_restores_state() {
        let mut gl = FakeGlContext::new();
        let mut fog = Q2FogPass::new(&mut gl);
        let mut op = operation();
        op.camera.projection = symmetric_projection();
        fog.draw(&mut gl, &op, 64, 64);
        gl.assert_contains("depth snapshot", |call| matches!(call, GlCall::CopyTexImage2D { .. }));
        gl.assert_contains("fog quad", |call| matches!(call, GlCall::Begin { mode: QUADS }));
        gl.assert_contains("program restore", |call| {
            matches!(call, GlCall::UseProgram { program: 0 })
        });
        fog.close(&mut gl);
        gl.assert_contains("texture release", |call| matches!(call, GlCall::DeleteTextures { .. }));
    }

    #[test]
    fn skips_passes_when_fog_is_inactive() {
        let mut gl = FakeGlContext::new();
        let mut fog = Q2FogPass::new(&mut gl);
        let mut op = operation();
        op.camera.projection = symmetric_projection();
        op.fog.density = 0.0;
        gl.clear_log();
        fog.draw(&mut gl, &op, 64, 64);
        assert!(gl.log.is_empty());
        fog.close(&mut gl);
    }

    #[test]
    #[should_panic(expected = "symmetric perspective")]
    fn rejects_non_symmetric_projection() {
        let mut gl = FakeGlContext::new();
        let mut fog = Q2FogPass::new(&mut gl);
        fog.draw(&mut gl, &operation(), 64, 64);
    }
}
