//! Output gamma remapping (donor `src/render/gl/output-gamma.ts`).

use std::collections::HashMap;

use super::programs::compile_program;
use super::textures::{with_pixel_store, PixelDirection};
use super::{
    FramebufferPrecision, GlContext, ACTIVE_TEXTURE, ALPHA_TEST, BACK_LEFT_ATTACHMENT, BLEND, CLIP_PLANE0,
    COLOR_ATTACHMENT0, COLOR_BUFFER_BIT, COLOR_WRITEMASK, CULL_FACE_CAP, CURRENT_PROGRAM, DEPTH24_STENCIL8,
    DEPTH32F_STENCIL8, DEPTH_ATTACHMENT, DEPTH_BUFFER_BIT, DEPTH_COMPONENT, DEPTH_COMPONENT16, DEPTH_COMPONENT24,
    DEPTH_COMPONENT32, DEPTH_COMPONENT32F, DEPTH_STENCIL, DEPTH_STENCIL_ATTACHMENT, DEPTH_TEST, DEPTH_WRITEMASK,
    DITHER, FILL, FLOAT, FLOAT_32_UNSIGNED_INT_24_8_REV, FRAMEBUFFER, FRAMEBUFFER_ATTACHMENT_COMPONENT_TYPE,
    FRAMEBUFFER_COMPLETE, FRONT_AND_BACK, LUMINANCE, LUMINANCE8, NEAREST, POLYGON_MODE_PNAME, POLYGON_OFFSET_FILL,
    QUADS, RGB565, RGB8, RGBA, RGBA8, SCISSOR_TEST, STENCIL_BUFFER_BIT, STENCIL_TEST, TEXTURE0, TEXTURE_2D,
    TEXTURE_BINDING_2D, TEXTURE_MAG_FILTER, TEXTURE_MIN_FILTER, TEXTURE_WRAP_S, TEXTURE_WRAP_T, UNSIGNED_BYTE,
    UNSIGNED_INT, UNSIGNED_INT_24_8, VIEWPORT,
};
use crate::render::error::RenderError;

pub const GAMMA_VERTEX_SHADER: &str = "#version 120
varying vec2 tc;
void main() { tc = gl_MultiTexCoord0.xy; gl_Position = gl_Vertex; }
";

pub const GAMMA_FRAGMENT_SHADER: &str = "#version 120
uniform sampler2D rawColor;
uniform sampler2D gammaTable;
varying vec2 tc;
float corrected(float value) {
  return texture2D(gammaTable, vec2((floor(clamp(value, 0.0, 1.0) * 255.0 + 0.5) + 0.5) / 256.0, 0.5)).r;
}
void main() {
  vec4 color = texture2D(rawColor, tc);
  gl_FragColor = vec4(corrected(color.r), corrected(color.g), corrected(color.b), color.a);
}
";

/// Q3 display-gamma table, or `None` when `gamma` is exactly one. Values above
/// one brighten output.
#[must_use]
pub fn output_gamma_table(gamma: f32) -> Option<[u8; 256]> {
    if !gamma.is_finite() || !(0.5..=3.0).contains(&gamma) {
        panic!(
            "{}",
            RenderError::BadDimensions {
                width: 0,
                height: 0,
                detail: "Output gamma must be between 0.5 and 3".to_string()
            }
        );
    }
    if gamma == 1.0 {
        return None;
    }
    let mut table = [0u8; 256];
    for (index, slot) in table.iter_mut().enumerate() {
        let base = (f64::from(index as u32) / 255.0) as f32;
        let value = 255.0 * base.powf(1.0 / gamma) + 0.5;
        *slot = value.trunc().clamp(0.0, 255.0) as u8;
    }
    Some(table)
}

/// Raw color buffers sharing the source depth/stencil across draw-buffer changes.
#[derive(Debug)]
pub struct GlOutputGamma {
    target: u32,
    depth: u32,
    table: u32,
    colors: HashMap<u32, u32>,
    program: u32,
    raw_uniform: i32,
    table_uniform: i32,
    depth_internal: i32,
    depth_format: u32,
    depth_type: u32,
    mask: u32,
    precision: FramebufferPrecision,
    width: i32,
    height: i32,
    closed: bool,
}

impl GlOutputGamma {
    pub fn new<C: GlContext>(gl: &mut C, precision: FramebufferPrecision, table: &[u8; 256]) -> Self {
        let component = gl.get_framebuffer_attachment_parameteriv(
            FRAMEBUFFER,
            BACK_LEFT_ATTACHMENT,
            FRAMEBUFFER_ATTACHMENT_COMPONENT_TYPE,
        );
        let floating = component == FLOAT as i32;
        let depth_internal = if precision.stencil_bits > 0 {
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
        let depth_format = if precision.stencil_bits > 0 {
            DEPTH_STENCIL
        } else {
            DEPTH_COMPONENT
        };
        let depth_type = if precision.stencil_bits > 0 {
            if floating {
                FLOAT_32_UNSIGNED_INT_24_8_REV
            } else {
                UNSIGNED_INT_24_8
            }
        } else if floating {
            FLOAT
        } else {
            UNSIGNED_INT
        };
        let mask = COLOR_BUFFER_BIT
            | DEPTH_BUFFER_BIT
            | if precision.stencil_bits > 0 {
                STENCIL_BUFFER_BIT
            } else {
                0
            };
        let target = gl.gen_framebuffers(1).first().copied().unwrap_or(0);
        let depth = gl.gen_textures(1).first().copied().unwrap_or(0);
        let table_name = gl.gen_textures(1).first().copied().unwrap_or(0);
        if target == 0 || depth == 0 || table_name == 0 {
            panic!("{}", RenderError::Backend("OpenGL gamma allocation failed".to_string()));
        }
        let program = compile_program(gl, GAMMA_VERTEX_SHADER, GAMMA_FRAGMENT_SHADER);
        let raw_uniform = gl.get_uniform_location(program, "rawColor");
        let table_uniform = gl.get_uniform_location(program, "gammaTable");
        if raw_uniform < 0 || table_uniform < 0 {
            gl.delete_program(program);
            panic!(
                "{}",
                RenderError::Backend("OpenGL gamma uniform is missing".to_string())
            );
        }
        let mut gamma = Self {
            target,
            depth,
            table: table_name,
            colors: HashMap::new(),
            program,
            raw_uniform,
            table_uniform,
            depth_internal,
            depth_format,
            depth_type,
            mask,
            precision,
            width: 0,
            height: 0,
            closed: false,
        };
        gamma.update(gl, table);
        gamma
    }

    fn integer<C: GlContext>(gl: &mut C, name: u32) -> i32 {
        let mut out = [0];
        gl.get_integerv(name, &mut out);
        out[0]
    }

    fn with_texture<C: GlContext>(gl: &mut C, name: u32, action: impl FnOnce(&mut C)) {
        let active = Self::integer(gl, ACTIVE_TEXTURE) as u32;
        gl.active_texture(TEXTURE0);
        let previous = Self::integer(gl, TEXTURE_BINDING_2D) as u32;
        gl.bind_texture(TEXTURE_2D, name);
        gl.tex_parameteri(TEXTURE_2D, TEXTURE_MIN_FILTER, NEAREST);
        gl.tex_parameteri(TEXTURE_2D, TEXTURE_MAG_FILTER, NEAREST);
        gl.tex_parameteri(TEXTURE_2D, TEXTURE_WRAP_S, super::CLAMP_TO_EDGE);
        gl.tex_parameteri(TEXTURE_2D, TEXTURE_WRAP_T, super::CLAMP_TO_EDGE);
        action(gl);
        gl.bind_texture(TEXTURE_2D, previous);
        gl.active_texture(active);
    }

    pub fn update<C: GlContext>(&mut self, gl: &mut C, table: &[u8; 256]) {
        let name = self.table;
        Self::with_texture(gl, name, |gl| {
            with_pixel_store(gl, PixelDirection::Unpack, |gl| {
                gl.tex_image_2d_bytes(TEXTURE_2D, 0, LUMINANCE8, 256, 1, 0, LUMINANCE, UNSIGNED_BYTE, table);
            });
        });
    }

    fn blit<C: GlContext>(&self, gl: &mut C, mask: u32) {
        let scissor = gl.is_enabled(SCISSOR_TEST);
        gl.disable(SCISSOR_TEST);
        gl.blit_framebuffer(
            [0, 0, self.width, self.height],
            [0, 0, self.width, self.height],
            mask,
            NEAREST as u32,
        );
        if scissor {
            gl.enable(SCISSOR_TEST);
        }
    }

    /// Bind the raw target for `buffer`; returns whether storage changed.
    pub fn bind<C: GlContext>(&mut self, gl: &mut C, buffer: u32, width: i32, height: i32) -> bool {
        let mut changed = false;
        if width != self.width || height != self.height {
            changed = true;
            for color in self.colors.values() {
                gl.delete_textures(&[*color]);
            }
            self.colors.clear();
            self.width = width;
            self.height = height;
            let (depth, internal, format, ty) = (self.depth, self.depth_internal, self.depth_format, self.depth_type);
            Self::with_texture(gl, depth, |gl| {
                gl.tex_image_2d_null(TEXTURE_2D, 0, internal, width, height, 0, format, ty);
            });
        }
        let initialize_depth = self.colors.is_empty();
        if let std::collections::hash_map::Entry::Vacant(e) = self.colors.entry(buffer) {
            changed = true;
            let color = gl.gen_textures(1).first().copied().unwrap_or(0);
            if color == 0 {
                panic!(
                    "{}",
                    RenderError::Backend("OpenGL gamma color allocation failed".to_string())
                );
            }
            e.insert(color);
            let precision = self.precision;
            Self::with_texture(gl, color, |gl| {
                let internal = if precision.alpha_bits > 0 {
                    RGBA8
                } else if precision.color_bits <= 16 {
                    RGB565
                } else {
                    RGB8
                };
                gl.tex_image_2d_null(TEXTURE_2D, 0, internal, width, height, 0, RGBA, UNSIGNED_BYTE);
            });
            gl.bind_framebuffer(FRAMEBUFFER, self.target);
            gl.framebuffer_texture_2d(FRAMEBUFFER, COLOR_ATTACHMENT0, TEXTURE_2D, color, 0);
            gl.framebuffer_texture_2d(
                FRAMEBUFFER,
                if precision.stencil_bits > 0 {
                    DEPTH_STENCIL_ATTACHMENT
                } else {
                    DEPTH_ATTACHMENT
                },
                TEXTURE_2D,
                self.depth,
                0,
            );
            gl.draw_buffer(COLOR_ATTACHMENT0);
            gl.read_buffer(COLOR_ATTACHMENT0);
            let status = gl.check_framebuffer_status(FRAMEBUFFER);
            if status != FRAMEBUFFER_COMPLETE {
                panic!(
                    "{}",
                    RenderError::Backend(format!("OpenGL gamma framebuffer incomplete: 0x{status:x}"))
                );
            }
            gl.bind_framebuffer(super::READ_FRAMEBUFFER, 0);
            gl.read_buffer(buffer);
            let mask = if initialize_depth { self.mask } else { COLOR_BUFFER_BIT };
            self.blit(gl, mask);
        }
        let color = self
            .colors
            .get(&buffer)
            .copied()
            .unwrap_or_else(|| panic!("{}", RenderError::Backend("OpenGL gamma color is missing".to_string())));
        gl.bind_framebuffer(FRAMEBUFFER, self.target);
        gl.framebuffer_texture_2d(FRAMEBUFFER, COLOR_ATTACHMENT0, TEXTURE_2D, color, 0);
        gl.draw_buffer(COLOR_ATTACHMENT0);
        gl.read_buffer(COLOR_ATTACHMENT0);
        changed
    }

    pub fn select_default<C: GlContext>(&self, gl: &mut C, buffer: u32) {
        gl.bind_framebuffer(FRAMEBUFFER, 0);
        gl.draw_buffer(buffer);
        gl.read_buffer(buffer);
    }

    pub fn finish<C: GlContext>(&mut self, gl: &mut C, buffer: u32) {
        let old_program = Self::integer(gl, CURRENT_PROGRAM) as u32;
        let active = Self::integer(gl, ACTIVE_TEXTURE) as u32;
        let mut viewport = [0; 4];
        gl.get_integerv(VIEWPORT, &mut viewport);
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
        gl.bind_framebuffer(FRAMEBUFFER, 0);
        gl.viewport(0, 0, self.width, self.height);
        gl.depth_mask(false);
        gl.color_mask(true, true, true, true);
        gl.polygon_mode(FRONT_AND_BACK, FILL);
        for (cap, _) in &enables {
            gl.disable(*cap);
        }
        gl.use_program(self.program);
        gl.uniform_1i(self.raw_uniform, 0);
        gl.uniform_1i(self.table_uniform, 1);
        gl.bind_texture(TEXTURE_2D, self.table);
        gl.active_texture(TEXTURE0);
        let colors: Vec<(u32, u32)> = self.colors.iter().map(|(buffer, color)| (*buffer, *color)).collect();
        for (draw_buffer, color) in colors {
            gl.draw_buffer(draw_buffer);
            gl.bind_texture(TEXTURE_2D, color);
            gl.begin(QUADS);
            gl.tex_coord_2f(0.0, 0.0);
            gl.vertex_4f(-1.0, -1.0, 0.0, 1.0);
            gl.tex_coord_2f(1.0, 0.0);
            gl.vertex_4f(1.0, -1.0, 0.0, 1.0);
            gl.tex_coord_2f(1.0, 1.0);
            gl.vertex_4f(1.0, 1.0, 0.0, 1.0);
            gl.tex_coord_2f(0.0, 1.0);
            gl.vertex_4f(-1.0, 1.0, 0.0, 1.0);
            gl.end();
        }
        gl.bind_texture(TEXTURE_2D, texture0);
        gl.active_texture(TEXTURE0 + 1);
        gl.bind_texture(TEXTURE_2D, texture1);
        gl.active_texture(active);
        gl.use_program(old_program);
        gl.depth_mask(depth_mask);
        gl.color_mask(
            color_mask[0] != 0,
            color_mask[1] != 0,
            color_mask[2] != 0,
            color_mask[3] != 0,
        );
        gl.polygon_mode(super::FRONT, modes[0] as u32);
        gl.polygon_mode(super::BACK, modes[1] as u32);
        gl.viewport(viewport[0], viewport[1], viewport[2], viewport[3]);
        for (cap, enabled) in enables {
            if enabled {
                gl.enable(cap);
            } else {
                gl.disable(cap);
            }
        }
        gl.draw_buffer(buffer);
        gl.read_buffer(buffer);
    }

    pub fn restore<C: GlContext>(&mut self, gl: &mut C, buffer: u32) {
        let colors: Vec<(u32, u32)> = self.colors.iter().map(|(buffer, color)| (*buffer, *color)).collect();
        for (draw_buffer, color) in colors {
            gl.bind_framebuffer(super::READ_FRAMEBUFFER, self.target);
            gl.framebuffer_texture_2d(super::READ_FRAMEBUFFER, COLOR_ATTACHMENT0, TEXTURE_2D, color, 0);
            gl.read_buffer(COLOR_ATTACHMENT0);
            gl.bind_framebuffer(super::DRAW_FRAMEBUFFER, 0);
            gl.draw_buffer(draw_buffer);
            self.blit(gl, self.mask);
        }
        gl.bind_framebuffer(FRAMEBUFFER, 0);
        gl.draw_buffer(buffer);
        gl.read_buffer(buffer);
    }

    pub fn close<C: GlContext>(&mut self, gl: &mut C) {
        if self.closed {
            return;
        }
        gl.bind_framebuffer(FRAMEBUFFER, 0);
        for color in self.colors.values() {
            gl.delete_textures(&[*color]);
        }
        self.colors.clear();
        gl.delete_textures(&[self.depth]);
        gl.delete_textures(&[self.table]);
        gl.delete_framebuffers(&[self.target]);
        gl.delete_program(self.program);
        self.closed = true;
    }
}

#[cfg(test)]
mod tests {
    use super::super::{FakeGlContext, GlCall, BACK};
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
    fn table_is_identity_at_one_and_monotonic_above() {
        assert!(output_gamma_table(1.0).is_none());
        let table = output_gamma_table(2.0).expect("table");
        assert_eq!(table[0], 0);
        assert_eq!(table[255], 255);
        for pair in table.windows(2) {
            assert!(pair[0] <= pair[1]);
        }
        assert!(table[128] > 128);
    }

    #[test]
    fn bind_finish_and_restore_round_trip() {
        let table = output_gamma_table(2.0).expect("table");
        let mut gl = FakeGlContext::new();
        let mut gamma = GlOutputGamma::new(&mut gl, precision(), &table);
        assert!(gamma.bind(&mut gl, BACK, 64, 64));
        assert!(!gamma.bind(&mut gl, BACK, 64, 64));
        gamma.finish(&mut gl, BACK);
        gl.assert_contains("gamma quad", |call| matches!(call, GlCall::Begin { mode: QUADS }));
        gamma.restore(&mut gl, BACK);
        gl.assert_contains("blit back", |call| matches!(call, GlCall::BlitFramebuffer { .. }));
        gamma.close(&mut gl);
        gl.assert_contains("target release", |call| {
            matches!(call, GlCall::DeleteFramebuffers { .. })
        });
    }

    #[test]
    #[should_panic(expected = "between 0.5 and 3")]
    fn rejects_out_of_range_gamma() {
        let _ = output_gamma_table(4.0);
    }
}
