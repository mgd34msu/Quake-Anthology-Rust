//! Load-owned targets for complete-material opacity. Stage execution remains
//! in GlBackend; this owner only changes the destination and commits color.
use super::*;

const FRAMEBUFFER: u32 = 0x8d40;
const READ_FRAMEBUFFER: u32 = 0x8ca8;
const DRAW_FRAMEBUFFER: u32 = 0x8ca9;
const COLOR_ATTACHMENT: u32 = 0x8ce0;
const RENDERBUFFER: u32 = 0x8d41;
const NEAREST: u32 = 0x2600;
const COMPOSITE_VERTEX: &str = r#"#version 440 core
void main() {
    vec2 p=vec2((gl_VertexID<<1)&2,gl_VertexID&2);
    gl_Position=vec4(p*2.0-1.0,0.0,1.0);
}
"#;
const COMPOSITE_FRAGMENT: &str = r#"#version 440 core
uniform sampler2D u_original;
uniform sampler2D u_completed;
uniform vec4 u_opacity;
layout(location=0) out vec4 fragment;
void main() {
    ivec2 pixel=ivec2(gl_FragCoord.xy);
    vec4 original=texelFetch(u_original,pixel,0);
    vec4 completed=texelFetch(u_completed,pixel,0);
    fragment=mix(original,completed,u_opacity.x);
}
"#;

#[derive(Default)]
pub(super) struct MaterialTargets {
    frames: [u32; 2],
    colors: [u32; 2],
    depth: u32,
    program: u32,
    opacity: i32,
    copy_bits: u32,
}
impl MaterialTargets {
    pub(super) fn load(
        &mut self,
        gl: &Gl,
        width: u32,
        height: u32,
        display: &DisplayConfiguration,
    ) -> Result<(), String> {
        let mut depth_component = 0;
        if display.depth_bits == 32 && display.stencil_bits == 0 {
            // SAFETY: The current default framebuffer is complete; this query
            // distinguishes float depth from normalized integer depth at load.
            unsafe {
                (gl.framebuffer_parameter)(FRAMEBUFFER, 0x1801, 0x8211, &mut depth_component);
            }
        }
        let (depth_format, attachment, copy_bits) = match (display.depth_bits, display.stencil_bits)
        {
            (16, 0) => (0x81a5, 0x8d00, DEPTH_BUFFER_BIT),
            (24, 0) => (0x81a6, 0x8d00, DEPTH_BUFFER_BIT),
            (32, 0) => (
                if depth_component == FLOAT as i32 {
                    0x8cac
                } else {
                    0x81a7
                },
                0x8d00,
                DEPTH_BUFFER_BIT,
            ),
            (24, 8) => (0x88f0, 0x821a, DEPTH_BUFFER_BIT | 0x0400),
            (32, 8) => (0x8cad, 0x821a, DEPTH_BUFFER_BIT | 0x0400),
            (0, 0) => (0, 0, 0),
            _ => return Err("unsupported material target depth/stencil format".into()),
        };
        self.copy_bits = COLOR_BUFFER_BIT | copy_bits;
        // SAFETY: Entry points belong to this current context. Dimensions were
        // validated by backend load; all object handles are owned here until drop.
        unsafe {
            self.program = compile_program_sources(gl, COMPOSITE_VERTEX, COMPOSITE_FRAGMENT)?;
            (gl.use_program)(self.program);
            (gl.uniform_integer)(
                (gl.uniform_location)(self.program, c"u_original".as_ptr()),
                0,
            );
            (gl.uniform_integer)(
                (gl.uniform_location)(self.program, c"u_completed".as_ptr()),
                2,
            );
            self.opacity = (gl.uniform_location)(self.program, c"u_opacity".as_ptr());
            (gl.gen_framebuffers)(2, self.frames.as_mut_ptr());
            (gl.gen_textures)(2, self.colors.as_mut_ptr());
            for index in 0..2 {
                (gl.bind_framebuffer)(FRAMEBUFFER, self.frames[index]);
                (gl.bind_texture)(TEXTURE_2D, self.colors[index]);
                (gl.texture_image)(
                    TEXTURE_2D,
                    0,
                    0x8058,
                    width as i32,
                    height as i32,
                    0,
                    RGBA,
                    UNSIGNED_BYTE,
                    ptr::null(),
                );
                (gl.texture_parameter)(TEXTURE_2D, 0x2801, NEAREST as i32);
                (gl.texture_parameter)(TEXTURE_2D, 0x2800, NEAREST as i32);
                (gl.framebuffer_texture)(
                    FRAMEBUFFER,
                    COLOR_ATTACHMENT,
                    TEXTURE_2D,
                    self.colors[index],
                    0,
                );
                (gl.draw_buffer)(COLOR_ATTACHMENT);
                (gl.read_buffer)(COLOR_ATTACHMENT);
                if index == 1 && depth_format != 0 {
                    (gl.gen_renderbuffers)(1, &mut self.depth);
                    (gl.bind_renderbuffer)(RENDERBUFFER, self.depth);
                    (gl.renderbuffer_storage)(
                        RENDERBUFFER,
                        depth_format,
                        width as i32,
                        height as i32,
                    );
                    (gl.framebuffer_renderbuffer)(
                        FRAMEBUFFER,
                        attachment,
                        RENDERBUFFER,
                        self.depth,
                    );
                }
                if (gl.check_framebuffer)(FRAMEBUFFER) != 0x8cd5 {
                    (gl.bind_framebuffer)(FRAMEBUFFER, 0);
                    return Err("material framebuffer is incomplete".into());
                }
            }
            (gl.bind_framebuffer)(FRAMEBUFFER, 0);
            (gl.bind_texture)(TEXTURE_2D, 0);
        }
        Ok(())
    }
    pub(super) fn begin(&self, gl: &Gl, viewport: Viewport, height: u32) {
        let x = viewport.x as i32;
        let y = (height - viewport.y - viewport.height) as i32;
        let right = x + viewport.width as i32;
        let top = y + viewport.height as i32;
        // SAFETY: Owned compatible attachments have complete load-sized storage.
        // The validated view bounds select identical source/destination pixels.
        unsafe {
            (gl.bind_framebuffer)(READ_FRAMEBUFFER, 0);
            for index in 0..2 {
                (gl.bind_framebuffer)(DRAW_FRAMEBUFFER, self.frames[index]);
                (gl.blit_framebuffer)(
                    x,
                    y,
                    right,
                    top,
                    x,
                    y,
                    right,
                    top,
                    if index == 0 {
                        COLOR_BUFFER_BIT
                    } else {
                        self.copy_bits
                    },
                    NEAREST,
                );
            }
            (gl.bind_framebuffer)(FRAMEBUFFER, self.frames[1]);
        }
    }
    pub(super) fn finish(
        &self,
        gl: &Gl,
        opacity: crate::scene::MaterialOpacity,
        commit: bool,
        vao: u32,
    ) {
        // SAFETY: No texture sampled below is attached to the real destination.
        // Stage execution has finished; color is the only committed attachment.
        unsafe {
            (gl.bind_framebuffer)(FRAMEBUFFER, 0);
            if commit {
                (gl.use_program)(self.program);
                for capability in [DEPTH_TEST, CULL_FACE, BLEND, CLIP_DISTANCE0, 0x8037] {
                    (gl.disable)(capability);
                }
                (gl.depth_mask)(0);
                (gl.bind_vertex_array)(vao);
                for (unit, texture) in [(0x84c0, self.colors[0]), (0x84c2, self.colors[1])] {
                    (gl.active_texture)(unit);
                    (gl.bind_texture)(TEXTURE_2D, texture);
                    (gl.bind_sampler)(unit - 0x84c0, 0);
                }
                (gl.uniform_color)(self.opacity, 1, [opacity.value(), 0., 0., 0.].as_ptr());
                (gl.draw_arrays)(TRIANGLES, 0, 3);
                (gl.enable)(CLIP_DISTANCE0);
            }
            (gl.active_texture)(0x84c0);
        }
    }
    pub(super) fn delete(&mut self, gl: &Gl) {
        // SAFETY: Backend drop retains the current owning context. Zero handles
        // are permitted for any objects not reached by a failed load.
        unsafe {
            (gl.delete_framebuffers)(2, self.frames.as_ptr());
            (gl.delete_textures)(2, self.colors.as_ptr());
            (gl.delete_renderbuffers)(1, &self.depth);
            (gl.delete_program)(self.program);
        }
    }
}
