//! Whole-object opacity compositing (donor `src/render/gl/object-opacity.ts`).

use super::programs::compile_program;
use super::{
    FramebufferPrecision, GlContext, ACTIVE_TEXTURE, ALPHA_TEST, BACK_LEFT_ATTACHMENT, BLEND, CLAMP_TO_EDGE,
    CLIP_PLANE0, COLOR_ATTACHMENT0, COLOR_BUFFER_BIT, COLOR_WRITEMASK, CULL_FACE_CAP, CURRENT_PROGRAM,
    DEPTH24_STENCIL8, DEPTH32F_STENCIL8, DEPTH_ATTACHMENT, DEPTH_BUFFER_BIT, DEPTH_COMPONENT, DEPTH_COMPONENT16,
    DEPTH_COMPONENT24, DEPTH_COMPONENT32, DEPTH_COMPONENT32F, DEPTH_STENCIL, DEPTH_STENCIL_ATTACHMENT, DEPTH_TEST,
    DEPTH_WRITEMASK, DITHER, DRAW_BUFFER_PNAME, DRAW_FRAMEBUFFER, FILL, FLOAT, FLOAT_32_UNSIGNED_INT_24_8_REV,
    FRAMEBUFFER, FRAMEBUFFER_ATTACHMENT_COMPONENT_TYPE, FRAMEBUFFER_BINDING, FRAMEBUFFER_COMPLETE, FRONT_AND_BACK,
    NEAREST, POLYGON_MODE_PNAME, POLYGON_OFFSET_FILL, QUADS, READ_BUFFER_PNAME, READ_FRAMEBUFFER,
    READ_FRAMEBUFFER_BINDING, RGB565, RGB8, RGBA, RGBA8, SAMPLE_BUFFERS, SCISSOR_BOX, SCISSOR_TEST, STENCIL_BUFFER_BIT,
    STENCIL_TEST, TEXTURE0, TEXTURE_2D, TEXTURE_BINDING_2D, TEXTURE_MAG_FILTER, TEXTURE_MIN_FILTER, TEXTURE_WRAP_S,
    TEXTURE_WRAP_T, UNSIGNED_BYTE, UNSIGNED_INT, UNSIGNED_INT_24_8, VIEWPORT,
};
use crate::render::error::RenderError;

pub const OPACITY_VERTEX_SHADER: &str = "#version 120
varying vec2 tc;
void main() { tc = gl_Vertex.xy * 0.5 + 0.5; gl_Position = gl_Vertex; }
";

pub const OPACITY_FRAGMENT_SHADER: &str = "#version 120
uniform sampler2D backdrop;
uniform sampler2D result;
uniform float opacity;
varying vec2 tc;
void main() {
  vec4 b = texture2D(backdrop, tc);
  gl_FragColor = b + opacity * (texture2D(result, tc) - b);
}
";

/// Saved destination state spanning one opacity child draw.
#[derive(Debug, Clone)]
pub struct OpacitySession {
    destination: u32,
    read_target: u32,
    draw_buffer: u32,
    read_buffer: u32,
    viewport: [i32; 4],
    scissor: [i32; 4],
    clipped: bool,
}

/// Scratch targets interpolating one object against the current backdrop.
#[derive(Debug)]
pub struct GlObjectOpacity {
    targets: [u32; 2],
    colors: [u32; 2],
    depth: u32,
    program: u32,
    backdrop_uniform: i32,
    result_uniform: i32,
    opacity_uniform: i32,
    dimensions: String,
    precision: FramebufferPrecision,
    closed: bool,
}

impl GlObjectOpacity {
    pub fn new<C: GlContext>(gl: &mut C, precision: FramebufferPrecision) -> Self {
        let targets = gl.gen_framebuffers(2);
        let colors = gl.gen_textures(2);
        let depth = gl.gen_textures(1);
        let targets = [
            targets.first().copied().unwrap_or(0),
            targets.get(1).copied().unwrap_or(0),
        ];
        let colors = [
            colors.first().copied().unwrap_or(0),
            colors.get(1).copied().unwrap_or(0),
        ];
        let depth = depth.first().copied().unwrap_or(0);
        if targets.contains(&0) || colors.contains(&0) || depth == 0 {
            panic!(
                "{}",
                RenderError::Backend("OpenGL opacity allocation failed".to_string())
            );
        }
        let program = compile_program(gl, OPACITY_VERTEX_SHADER, OPACITY_FRAGMENT_SHADER);
        let backdrop_uniform = gl.get_uniform_location(program, "backdrop");
        let result_uniform = gl.get_uniform_location(program, "result");
        let opacity_uniform = gl.get_uniform_location(program, "opacity");
        if backdrop_uniform < 0 || result_uniform < 0 || opacity_uniform < 0 {
            gl.delete_program(program);
            panic!("{}", RenderError::Backend("OpenGL opacity uniform missing".to_string()));
        }
        Self {
            targets,
            colors,
            depth,
            program,
            backdrop_uniform,
            result_uniform,
            opacity_uniform,
            dimensions: String::new(),
            precision,
            closed: false,
        }
    }

    fn integer<C: GlContext>(gl: &mut C, name: u32) -> i32 {
        let mut out = [0];
        gl.get_integerv(name, &mut out);
        out[0]
    }

    fn check<C: GlContext>(gl: &mut C) {
        let status = gl.check_framebuffer_status(FRAMEBUFFER);
        if status != FRAMEBUFFER_COMPLETE {
            panic!(
                "{}",
                RenderError::Backend(format!("OpenGL opacity framebuffer incomplete: 0x{status:x}"))
            );
        }
    }

    fn allocate<C: GlContext>(&mut self, gl: &mut C, width: i32, height: i32) {
        let bound = Self::integer(gl, FRAMEBUFFER_BINDING);
        let component = gl.get_framebuffer_attachment_parameteriv(
            READ_FRAMEBUFFER,
            if bound == 0 {
                BACK_LEFT_ATTACHMENT
            } else {
                DEPTH_ATTACHMENT
            },
            FRAMEBUFFER_ATTACHMENT_COMPONENT_TYPE,
        );
        let floating = component == FLOAT as i32;
        let dimensions = format!("{width}:{height}:{floating}");
        if dimensions == self.dimensions {
            return;
        }
        let active = Self::integer(gl, ACTIVE_TEXTURE) as u32;
        gl.active_texture(TEXTURE0);
        let previous = Self::integer(gl, TEXTURE_BINDING_2D) as u32;
        let precision = self.precision;
        let setup = |gl: &mut C, name: u32, internal: i32, format: u32, ty: u32| {
            gl.bind_texture(TEXTURE_2D, name);
            gl.tex_parameteri(TEXTURE_2D, TEXTURE_MIN_FILTER, NEAREST);
            gl.tex_parameteri(TEXTURE_2D, TEXTURE_MAG_FILTER, NEAREST);
            gl.tex_parameteri(TEXTURE_2D, TEXTURE_WRAP_S, CLAMP_TO_EDGE);
            gl.tex_parameteri(TEXTURE_2D, TEXTURE_WRAP_T, CLAMP_TO_EDGE);
            gl.tex_image_2d_null(TEXTURE_2D, 0, internal, width, height, 0, format, ty);
        };
        for color in self.colors {
            let internal = if precision.alpha_bits > 0 {
                RGBA8
            } else if precision.color_bits <= 16 {
                RGB565
            } else {
                RGB8
            };
            setup(gl, color, internal, RGBA, UNSIGNED_BYTE);
        }
        let stencil = precision.stencil_bits > 0;
        let internal = if stencil {
            if floating {
                DEPTH32F_STENCIL8
            } else {
                DEPTH24_STENCIL8
            }
        } else if floating {
            DEPTH_COMPONENT32F
        } else if precision.depth_bits <= 16 {
            DEPTH_COMPONENT16
        } else if precision.depth_bits <= 24 {
            DEPTH_COMPONENT24
        } else {
            DEPTH_COMPONENT32
        };
        let (format, ty) = if stencil {
            (
                DEPTH_STENCIL,
                if floating {
                    FLOAT_32_UNSIGNED_INT_24_8_REV
                } else {
                    UNSIGNED_INT_24_8
                },
            )
        } else {
            (DEPTH_COMPONENT, if floating { FLOAT } else { UNSIGNED_INT })
        };
        setup(gl, self.depth, internal, format, ty);
        for (index, target) in self.targets.iter().enumerate() {
            gl.bind_framebuffer(FRAMEBUFFER, *target);
            gl.framebuffer_texture_2d(FRAMEBUFFER, COLOR_ATTACHMENT0, TEXTURE_2D, self.colors[index], 0);
            if index == 1 {
                gl.framebuffer_texture_2d(
                    FRAMEBUFFER,
                    if stencil {
                        DEPTH_STENCIL_ATTACHMENT
                    } else {
                        DEPTH_ATTACHMENT
                    },
                    TEXTURE_2D,
                    self.depth,
                    0,
                );
            }
            gl.draw_buffer(COLOR_ATTACHMENT0);
            gl.read_buffer(COLOR_ATTACHMENT0);
            Self::check(gl);
        }
        self.dimensions = dimensions;
        gl.bind_texture(TEXTURE_2D, previous);
        gl.active_texture(active);
    }

    /// Bind the child-draw target. Called by the renderer's target selector
    /// while child geometry executes.
    pub fn bind_target<C: GlContext>(&self, gl: &mut C) {
        gl.bind_framebuffer(FRAMEBUFFER, self.targets[1]);
        gl.draw_buffer(COLOR_ATTACHMENT0);
        gl.read_buffer(COLOR_ATTACHMENT0);
    }

    /// Copy the backdrop into both scratch targets and return the session the
    /// child draw runs under. The caller binds the child target, runs the
    /// child, then calls [`GlObjectOpacity::finish`].
    pub fn begin<C: GlContext>(&mut self, gl: &mut C, width: i32, height: i32) -> OpacitySession {
        let session = OpacitySession {
            destination: Self::integer(gl, FRAMEBUFFER_BINDING) as u32,
            read_target: Self::integer(gl, READ_FRAMEBUFFER_BINDING) as u32,
            draw_buffer: Self::integer(gl, DRAW_BUFFER_PNAME) as u32,
            read_buffer: Self::integer(gl, READ_BUFFER_PNAME) as u32,
            viewport: {
                let mut out = [0; 4];
                gl.get_integerv(VIEWPORT, &mut out);
                out
            },
            scissor: {
                let mut out = [0; 4];
                gl.get_integerv(SCISSOR_BOX, &mut out);
                out
            },
            clipped: gl.is_enabled(SCISSOR_TEST),
        };
        if Self::integer(gl, SAMPLE_BUFFERS) != 0 {
            panic!(
                "{}",
                RenderError::Backend("Object opacity requires a single-sample framebuffer".to_string())
            );
        }
        self.allocate(gl, width, height);
        gl.disable(SCISSOR_TEST);
        for (index, target) in self.targets.iter().enumerate() {
            gl.bind_framebuffer(READ_FRAMEBUFFER, session.destination);
            gl.read_buffer(session.draw_buffer);
            gl.bind_framebuffer(DRAW_FRAMEBUFFER, *target);
            gl.draw_buffer(COLOR_ATTACHMENT0);
            let mut mask = COLOR_BUFFER_BIT;
            if index == 1 {
                mask |= DEPTH_BUFFER_BIT;
                if self.precision.stencil_bits > 0 {
                    mask |= STENCIL_BUFFER_BIT;
                }
            }
            gl.blit_framebuffer([0, 0, width, height], [0, 0, width, height], mask, NEAREST as u32);
        }
        let error = gl.get_error();
        if error != 0 {
            panic!(
                "{}",
                RenderError::Backend(format!("OpenGL opacity backdrop copy failed: 0x{error:x}"))
            );
        }
        if session.clipped {
            gl.enable(SCISSOR_TEST);
        }
        session
    }

    /// Composite the child result over the backdrop, then restore the session.
    pub fn finish<C: GlContext>(
        &mut self,
        gl: &mut C,
        session: &OpacitySession,
        opacity: f32,
        width: i32,
        height: i32,
    ) {
        gl.bind_framebuffer(DRAW_FRAMEBUFFER, session.destination);
        gl.draw_buffer(session.draw_buffer);
        self.composite(gl, opacity, width, height, session);
        gl.bind_framebuffer(DRAW_FRAMEBUFFER, session.destination);
        gl.draw_buffer(session.draw_buffer);
        gl.bind_framebuffer(READ_FRAMEBUFFER, session.read_target);
        gl.read_buffer(session.read_buffer);
        gl.scissor(
            session.scissor[0],
            session.scissor[1],
            session.scissor[2],
            session.scissor[3],
        );
        if session.clipped {
            gl.enable(SCISSOR_TEST);
        } else {
            gl.disable(SCISSOR_TEST);
        }
    }

    fn composite<C: GlContext>(&mut self, gl: &mut C, opacity: f32, width: i32, height: i32, session: &OpacitySession) {
        let program = Self::integer(gl, CURRENT_PROGRAM) as u32;
        let active = Self::integer(gl, ACTIVE_TEXTURE) as u32;
        let mut color_mask = [0; 4];
        gl.get_integerv(COLOR_WRITEMASK, &mut color_mask);
        let depth_mask = Self::integer(gl, DEPTH_WRITEMASK) != 0;
        let mut modes = [0; 2];
        gl.get_integerv(POLYGON_MODE_PNAME, &mut modes);
        let enables = [
            DEPTH_TEST,
            CULL_FACE_CAP,
            ALPHA_TEST,
            STENCIL_TEST,
            CLIP_PLANE0,
            SCISSOR_TEST,
            BLEND,
            DITHER,
            POLYGON_OFFSET_FILL,
        ]
        .map(|cap| (cap, gl.is_enabled(cap)));
        gl.active_texture(TEXTURE0);
        let texture0 = Self::integer(gl, TEXTURE_BINDING_2D) as u32;
        gl.active_texture(TEXTURE0 + 1);
        let texture1 = Self::integer(gl, TEXTURE_BINDING_2D) as u32;
        gl.viewport(0, 0, width, height);
        gl.depth_mask(false);
        gl.color_mask(true, true, true, true);
        gl.polygon_mode(FRONT_AND_BACK, FILL);
        for (cap, _) in &enables {
            gl.disable(*cap);
        }
        let x = 0
            .max(session.viewport[0])
            .max(if session.clipped { session.scissor[0] } else { 0 });
        let y = 0
            .max(session.viewport[1])
            .max(if session.clipped { session.scissor[1] } else { 0 });
        let right = width
            .min(session.viewport[0] + session.viewport[2])
            .min(if session.clipped {
                session.scissor[0] + session.scissor[2]
            } else {
                width
            });
        let top = height
            .min(session.viewport[1] + session.viewport[3])
            .min(if session.clipped {
                session.scissor[1] + session.scissor[3]
            } else {
                height
            });
        gl.enable(SCISSOR_TEST);
        gl.scissor(x, y, 0.max(right - x), 0.max(top - y));
        gl.use_program(self.program);
        gl.uniform_1i(self.backdrop_uniform, 0);
        gl.uniform_1i(self.result_uniform, 1);
        gl.uniform_1f(self.opacity_uniform, opacity);
        gl.bind_texture(TEXTURE_2D, self.colors[1]);
        gl.active_texture(TEXTURE0);
        gl.bind_texture(TEXTURE_2D, self.colors[0]);
        gl.begin(QUADS);
        gl.vertex_4f(-1.0, -1.0, 0.0, 1.0);
        gl.vertex_4f(1.0, -1.0, 0.0, 1.0);
        gl.vertex_4f(1.0, 1.0, 0.0, 1.0);
        gl.vertex_4f(-1.0, 1.0, 0.0, 1.0);
        gl.end();
        let error = gl.get_error();
        gl.bind_texture(TEXTURE_2D, texture0);
        gl.active_texture(TEXTURE0 + 1);
        gl.bind_texture(TEXTURE_2D, texture1);
        gl.active_texture(active);
        gl.use_program(program);
        gl.depth_mask(depth_mask);
        gl.color_mask(
            color_mask[0] != 0,
            color_mask[1] != 0,
            color_mask[2] != 0,
            color_mask[3] != 0,
        );
        gl.polygon_mode(super::FRONT, modes[0] as u32);
        gl.polygon_mode(super::BACK, modes[1] as u32);
        gl.viewport(
            session.viewport[0],
            session.viewport[1],
            session.viewport[2],
            session.viewport[3],
        );
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
                RenderError::Backend(format!("OpenGL opacity composite failed: 0x{error:x}"))
            );
        }
    }

    pub fn close<C: GlContext>(&mut self, gl: &mut C) {
        if self.closed {
            return;
        }
        gl.delete_framebuffers(&self.targets);
        gl.delete_textures(&self.colors);
        gl.delete_textures(&[self.depth]);
        gl.delete_program(self.program);
        self.closed = true;
    }
}

#[cfg(test)]
mod tests {
    use super::super::{FakeGlContext, GlCall};
    use super::*;

    fn precision() -> FramebufferPrecision {
        FramebufferPrecision {
            depth_bits: 24,
            stencil_bits: 8,
            color_bits: 24,
            alpha_bits: 8,
        }
    }

    #[test]
    fn backdrop_child_and_composite_round_trip() {
        let mut gl = FakeGlContext::new();
        let mut opacity = GlObjectOpacity::new(&mut gl, precision());
        gl.clear_log();
        let session = opacity.begin(&mut gl, 64, 64);
        assert_eq!(
            gl.count_matching(|call| matches!(call, GlCall::BlitFramebuffer { .. })),
            2
        );
        opacity.bind_target(&mut gl);
        gl.assert_contains("child target", |call| matches!(call, GlCall::BindFramebuffer { .. }));
        opacity.finish(&mut gl, &session, 0.5, 64, 64);
        gl.assert_contains("composite quad", |call| matches!(call, GlCall::Begin { mode: QUADS }));
        gl.assert_contains(
            "opacity uniform",
            |call| matches!(call, GlCall::Uniform1f { value, .. } if *value == 0.5),
        );
        opacity.close(&mut gl);
        gl.assert_contains("program release", |call| matches!(call, GlCall::DeleteProgram { .. }));
    }

    #[test]
    fn allocation_is_stable_across_frames() {
        let mut gl = FakeGlContext::new();
        let mut opacity = GlObjectOpacity::new(&mut gl, precision());
        let first = opacity.begin(&mut gl, 64, 64);
        opacity.finish(&mut gl, &first, 0.25, 64, 64);
        gl.clear_log();
        let second = opacity.begin(&mut gl, 64, 64);
        opacity.finish(&mut gl, &second, 0.25, 64, 64);
        gl.assert_absent("realloc", |call| matches!(call, GlCall::TexImage2D { .. }));
        opacity.close(&mut gl);
    }

    #[test]
    #[should_panic(expected = "single-sample")]
    fn rejects_multisample_targets() {
        let mut gl = FakeGlContext::new();
        gl.set_int(SAMPLE_BUFFERS, vec![1]);
        let mut opacity = GlObjectOpacity::new(&mut gl, precision());
        opacity.begin(&mut gl, 64, 64);
    }
}
