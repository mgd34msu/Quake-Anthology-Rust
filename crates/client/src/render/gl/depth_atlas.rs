//! Depth-atlas shadow passes (donor `src/render/gl/depth-atlas.ts`).

use super::programs::StageProgram;
use super::{
    GlContext, ALPHA_TEST, ARRAY_BUFFER, BACK, BLEND, CLIENT_ATTRIB_STACK_DEPTH, CLIENT_VERTEX_ARRAY_BIT, CLIP_PLANE0,
    COLOR_ARRAY, COLOR_ARRAY_POINTER, COLOR_WRITEMASK, CULL_FACE_CAP, CULL_FACE_MODE, CURRENT_PROGRAM,
    DEPTH_ATTACHMENT, DEPTH_BUFFER_BIT, DEPTH_CLEAR_VALUE, DEPTH_FUNC_PNAME, DEPTH_RANGE, DEPTH_TEST, DEPTH_WRITEMASK,
    DRAW_BUFFER_PNAME, DRAW_FRAMEBUFFER, EDGE_FLAG_ARRAY, ELEMENT_ARRAY_BUFFER, FILL, FRAMEBUFFER, FRAMEBUFFER_BINDING,
    FRAMEBUFFER_COMPLETE, FRONT, FRONT_AND_BACK, INDEX_ARRAY, MAX_CLIENT_ATTRIB_STACK_DEPTH, NORMAL_ARRAY, NO_BUFFER,
    POLYGON_MODE_PNAME, POLYGON_OFFSET_FACTOR, POLYGON_OFFSET_FILL, POLYGON_OFFSET_UNITS, PRIMITIVE_RESTART,
    READ_BUFFER_PNAME, READ_FRAMEBUFFER, READ_FRAMEBUFFER_BINDING, SCISSOR_BOX, SCISSOR_TEST, STENCIL_TEST, TEXTURE0,
    TEXTURE_2D, TEXTURE_COORD_ARRAY, TRIANGLES, VERSION, VERTEX_ARRAY, VERTEX_ARRAY_POINTER, VIEWPORT,
};
use crate::render::error::RenderError;
use crate::render::types::{CullFace, DepthAtlasPass};

const POSITION_CAPACITY: usize = 65_536;
const INDEX_CAPACITY: usize = 196_608;

/// Depth-atlas render target over a [`GlContext`].
#[derive(Debug)]
pub struct DepthAtlasTarget {
    framebuffer: u32,
    positions: Vec<f32>,
    indices: Vec<u32>,
    primitive_restart: bool,
    texture_coordinates: i32,
    closed: bool,
}

impl DepthAtlasTarget {
    pub fn new<C: GlContext>(gl: &mut C) -> Self {
        let version = gl.get_string(VERSION);
        let (major, minor) = parse_version(&version);
        let mut coordinates = [0];
        gl.get_integerv(super::MAX_TEXTURE_COORDS, &mut coordinates);
        let names = gl.gen_framebuffers(1);
        let framebuffer = names.first().copied().unwrap_or(0);
        if framebuffer == 0 {
            panic!(
                "{}",
                RenderError::Backend("OpenGL could not allocate a shadow framebuffer".to_string())
            );
        }
        Self {
            framebuffer,
            positions: vec![0.0; POSITION_CAPACITY * 4],
            indices: vec![0; INDEX_CAPACITY],
            primitive_restart: major > 3 || major == 3 && minor >= 1,
            texture_coordinates: coordinates[0],
            closed: false,
        }
    }

    #[allow(clippy::too_many_lines)]
    pub fn draw<C: GlContext>(
        &mut self,
        gl: &mut C,
        stage: &StageProgram,
        texture: u32,
        width: i32,
        height: i32,
        passes: &[DepthAtlasPass],
    ) {
        if self.closed {
            panic!("{}", RenderError::Backend("OpenGL depth atlas is closed".to_string()));
        }
        for pass in passes {
            let viewport = &pass.viewport;
            for value in [viewport.x, viewport.y, viewport.width, viewport.height] {
                if value.fract() != 0.0 {
                    panic!(
                        "{}",
                        RenderError::BadViewport("OpenGL depth atlas viewport is outside its image".to_string())
                    );
                }
            }
            let (x, y, w, h) = (
                viewport.x as i32,
                viewport.y as i32,
                viewport.width as i32,
                viewport.height as i32,
            );
            if x < 0 || y < 0 || w <= 0 || h <= 0 || x + w > width || y + h > height {
                panic!(
                    "{}",
                    RenderError::BadViewport("OpenGL depth atlas viewport is outside its image".to_string())
                );
            }
            if let Some(clear) = pass.clear_depth {
                if !clear.is_finite() || !(0.0..=1.0).contains(&clear) {
                    panic!(
                        "{}",
                        RenderError::BadViewport("OpenGL depth atlas clear must be in 0..1".to_string())
                    );
                }
            }
            for draw in &pass.draws {
                if draw.indices.len() % 3 != 0 {
                    panic!(
                        "{}",
                        RenderError::BadBatch {
                            index: draw.indices.len(),
                            detail: "OpenGL depth atlas triangles are incomplete".to_string()
                        }
                    );
                }
                for position in &draw.positions {
                    if ![position.x, position.y, position.z, position.w]
                        .iter()
                        .all(|value| value.is_finite())
                    {
                        panic!(
                            "{}",
                            RenderError::BadBatch {
                                index: 0,
                                detail: "OpenGL depth atlas vertices must be finite float32".to_string()
                            }
                        );
                    }
                }
                for (slot, index) in draw.indices.iter().enumerate() {
                    if (*index as usize) >= draw.positions.len() {
                        panic!(
                            "{}",
                            RenderError::BadBatch {
                                index: slot,
                                detail: "OpenGL depth atlas vertex index is invalid".to_string()
                            }
                        );
                    }
                }
            }
        }
        let mut one = [0];
        gl.get_integerv(FRAMEBUFFER_BINDING, &mut one);
        let old_target = one[0] as u32;
        gl.get_integerv(READ_FRAMEBUFFER_BINDING, &mut one);
        let old_read_target = one[0] as u32;
        gl.get_integerv(DRAW_BUFFER_PNAME, &mut one);
        let old_draw_buffer = one[0] as u32;
        gl.get_integerv(READ_BUFFER_PNAME, &mut one);
        let old_read_buffer = one[0] as u32;
        let mut viewport = [0; 4];
        gl.get_integerv(VIEWPORT, &mut viewport);
        let mut scissor = [0; 4];
        gl.get_integerv(SCISSOR_BOX, &mut scissor);
        let mut color_mask = [0; 4];
        gl.get_integerv(COLOR_WRITEMASK, &mut color_mask);
        let mut depth_range = [0.0; 2];
        gl.get_floatv(DEPTH_RANGE, &mut depth_range);
        let mut depth_clear = [0.0; 1];
        gl.get_floatv(DEPTH_CLEAR_VALUE, &mut depth_clear);
        gl.get_integerv(DEPTH_WRITEMASK, &mut one);
        let depth_mask = one[0] != 0;
        gl.get_integerv(DEPTH_FUNC_PNAME, &mut one);
        let depth_function = one[0] as u32;
        gl.get_integerv(CULL_FACE_MODE, &mut one);
        let cull = one[0] as u32;
        let mut polygon_mode = [0; 2];
        gl.get_integerv(POLYGON_MODE_PNAME, &mut polygon_mode);
        let mut offset_factor = [0.0; 1];
        gl.get_floatv(POLYGON_OFFSET_FACTOR, &mut offset_factor);
        let mut offset_units = [0.0; 1];
        gl.get_floatv(POLYGON_OFFSET_UNITS, &mut offset_units);
        gl.get_integerv(CURRENT_PROGRAM, &mut one);
        let program = one[0] as u32;
        let mut enables = vec![
            (DEPTH_TEST, gl.is_enabled(DEPTH_TEST)),
            (CULL_FACE_CAP, gl.is_enabled(CULL_FACE_CAP)),
            (BLEND, gl.is_enabled(BLEND)),
            (STENCIL_TEST, gl.is_enabled(STENCIL_TEST)),
            (ALPHA_TEST, gl.is_enabled(ALPHA_TEST)),
            (CLIP_PLANE0, gl.is_enabled(CLIP_PLANE0)),
            (SCISSOR_TEST, gl.is_enabled(SCISSOR_TEST)),
            (POLYGON_OFFSET_FILL, gl.is_enabled(POLYGON_OFFSET_FILL)),
        ];
        if self.primitive_restart {
            enables.push((PRIMITIVE_RESTART, gl.is_enabled(PRIMITIVE_RESTART)));
        }
        gl.get_integerv(CLIENT_ATTRIB_STACK_DEPTH, &mut one);
        let depth = one[0];
        gl.get_integerv(MAX_CLIENT_ATTRIB_STACK_DEPTH, &mut one);
        if depth >= one[0] {
            panic!(
                "{}",
                RenderError::Backend("OpenGL client attribute stack is full".to_string())
            );
        }
        gl.push_client_attrib(CLIENT_VERTEX_ARRAY_BIT);
        gl.bind_buffer(ARRAY_BUFFER, 0);
        gl.bind_buffer(ELEMENT_ARRAY_BUFFER, 0);
        for array in [
            NORMAL_ARRAY,
            COLOR_ARRAY,
            INDEX_ARRAY,
            EDGE_FLAG_ARRAY,
            VERTEX_ARRAY_POINTER,
            COLOR_ARRAY_POINTER,
        ] {
            gl.disable_client_state(array);
        }
        for unit in 0..self.texture_coordinates {
            gl.client_active_texture(TEXTURE0 + unit as u32);
            gl.disable_client_state(TEXTURE_COORD_ARRAY);
        }
        gl.enable_client_state(VERTEX_ARRAY);
        gl.vertex_pointer(4, 0, &self.positions);
        if self.primitive_restart {
            gl.disable(PRIMITIVE_RESTART);
        }
        gl.bind_framebuffer(FRAMEBUFFER, self.framebuffer);
        gl.framebuffer_texture_2d(FRAMEBUFFER, DEPTH_ATTACHMENT, TEXTURE_2D, texture, 0);
        gl.draw_buffer(NO_BUFFER);
        gl.read_buffer(NO_BUFFER);
        let status = gl.check_framebuffer_status(FRAMEBUFFER);
        if status != FRAMEBUFFER_COMPLETE {
            panic!(
                "{}",
                RenderError::Backend(format!("OpenGL shadow framebuffer is incomplete: 0x{status:x}"))
            );
        }
        stage.use_depth(gl);
        gl.enable(DEPTH_TEST);
        gl.depth_mask(true);
        gl.depth_func(super::LEQUAL);
        gl.depth_range(0.0, 1.0);
        gl.enable(SCISSOR_TEST);
        gl.color_mask(false, false, false, false);
        gl.polygon_mode(FRONT_AND_BACK, FILL);
        for cap in [BLEND, STENCIL_TEST, ALPHA_TEST, CLIP_PLANE0] {
            gl.disable(cap);
        }
        for pass in passes {
            let rect = &pass.viewport;
            let (x, y, w, h) = (rect.x as i32, rect.y as i32, rect.width as i32, rect.height as i32);
            gl.viewport(x, y, w, h);
            gl.scissor(x, y, w, h);
            if let Some(clear) = pass.clear_depth {
                gl.clear_depth(f64::from(clear));
                gl.clear(DEPTH_BUFFER_BIT);
            }
            let mut vertex_count = 0usize;
            let mut index_count = 0usize;
            let mut previous: Option<(CullFace, Option<(u32, u32)>)> = None;
            for draw in &pass.draws {
                let offset = draw
                    .polygon_offset
                    .map(|offset| (offset.factor.to_bits(), offset.units.to_bits()));
                if previous != Some((draw.cull, offset)) {
                    if index_count != 0 {
                        gl.draw_elements(TRIANGLES, &self.indices[..index_count]);
                        vertex_count = 0;
                        index_count = 0;
                    }
                    match draw.cull {
                        CullFace::None => gl.disable(CULL_FACE_CAP),
                        CullFace::Back => {
                            gl.enable(CULL_FACE_CAP);
                            gl.cull_face(BACK);
                        }
                        CullFace::Front => {
                            gl.enable(CULL_FACE_CAP);
                            gl.cull_face(FRONT);
                        }
                    }
                    match draw.polygon_offset {
                        None => gl.disable(POLYGON_OFFSET_FILL),
                        Some(offset) => {
                            gl.enable(POLYGON_OFFSET_FILL);
                            gl.polygon_offset(offset.factor, offset.units);
                        }
                    }
                    previous = Some((draw.cull, offset));
                }
                if draw.positions.len() <= POSITION_CAPACITY && draw.indices.len() <= INDEX_CAPACITY {
                    if vertex_count + draw.positions.len() > POSITION_CAPACITY
                        || index_count + draw.indices.len() > INDEX_CAPACITY
                    {
                        gl.draw_elements(TRIANGLES, &self.indices[..index_count]);
                        vertex_count = 0;
                        index_count = 0;
                    }
                    let base = vertex_count;
                    for position in &draw.positions {
                        let slot = vertex_count * 4;
                        self.positions[slot] = position.x;
                        self.positions[slot + 1] = position.y;
                        self.positions[slot + 2] = position.z;
                        self.positions[slot + 3] = position.w;
                        vertex_count += 1;
                    }
                    for index in &draw.indices {
                        self.indices[index_count] = base as u32 + index;
                        index_count += 1;
                    }
                } else {
                    if index_count != 0 {
                        gl.draw_elements(TRIANGLES, &self.indices[..index_count]);
                    }
                    vertex_count = 0;
                    index_count = 0;
                    for triangle in draw.indices.as_chunks::<3>().0 {
                        if vertex_count + 3 > POSITION_CAPACITY || index_count + 3 > INDEX_CAPACITY {
                            gl.draw_elements(TRIANGLES, &self.indices[..index_count]);
                            vertex_count = 0;
                            index_count = 0;
                        }
                        for corner in triangle {
                            let position = draw.positions.get(*corner as usize).unwrap_or_else(|| {
                                panic!(
                                    "{}",
                                    RenderError::Backend("OpenGL validated depth vertex disappeared".to_string())
                                )
                            });
                            self.indices[index_count] = vertex_count as u32;
                            index_count += 1;
                            let slot = vertex_count * 4;
                            self.positions[slot] = position.x;
                            self.positions[slot + 1] = position.y;
                            self.positions[slot + 2] = position.z;
                            self.positions[slot + 3] = position.w;
                            vertex_count += 1;
                        }
                    }
                }
            }
            if index_count != 0 {
                gl.draw_elements(TRIANGLES, &self.indices[..index_count]);
            }
        }
        let error = gl.get_error();
        gl.pop_client_attrib();
        gl.framebuffer_texture_2d(FRAMEBUFFER, DEPTH_ATTACHMENT, TEXTURE_2D, 0, 0);
        gl.bind_framebuffer(DRAW_FRAMEBUFFER, old_target);
        gl.bind_framebuffer(READ_FRAMEBUFFER, old_read_target);
        gl.draw_buffer(old_draw_buffer);
        gl.read_buffer(old_read_buffer);
        gl.viewport(viewport[0], viewport[1], viewport[2], viewport[3]);
        gl.scissor(scissor[0], scissor[1], scissor[2], scissor[3]);
        gl.color_mask(
            color_mask[0] != 0,
            color_mask[1] != 0,
            color_mask[2] != 0,
            color_mask[3] != 0,
        );
        gl.depth_range(f64::from(depth_range[0]), f64::from(depth_range[1]));
        gl.clear_depth(f64::from(depth_clear[0]));
        gl.depth_mask(depth_mask);
        gl.depth_func(depth_function);
        gl.cull_face(cull);
        gl.polygon_mode(FRONT, polygon_mode[0] as u32);
        gl.polygon_mode(BACK, polygon_mode[1] as u32);
        gl.polygon_offset(offset_factor[0], offset_units[0]);
        for (cap, enabled) in enables {
            if enabled {
                gl.enable(cap);
            } else {
                gl.disable(cap);
            }
        }
        stage.restore(gl, program);
        if error != 0 {
            panic!(
                "{}",
                RenderError::Backend(format!("OpenGL shadow atlas draw failed: 0x{error:x}"))
            );
        }
    }

    pub fn close<C: GlContext>(&mut self, gl: &mut C) {
        if self.closed {
            return;
        }
        gl.delete_framebuffers(&[self.framebuffer]);
        self.closed = true;
    }
}

fn parse_version(version: &str) -> (u32, u32) {
    let mut parts = version.split('.');
    let major = parts
        .next()
        .unwrap_or("")
        .chars()
        .take_while(char::is_ascii_digit)
        .collect::<String>()
        .parse()
        .unwrap_or(0);
    let minor = parts
        .next()
        .unwrap_or("")
        .chars()
        .take_while(char::is_ascii_digit)
        .collect::<String>()
        .parse()
        .unwrap_or(0);
    (major, minor)
}

#[cfg(test)]
mod tests {
    use super::super::{FakeGlContext, GlCall};
    use super::*;
    use crate::render::types::{DepthAtlasDraw, DepthAtlasPass, Rect};
    use qa_core::math::vec4;

    fn pass() -> DepthAtlasPass {
        DepthAtlasPass {
            viewport: Rect {
                x: 0.0,
                y: 0.0,
                width: 32.0,
                height: 32.0,
            },
            clear_depth: Some(1.0),
            draws: vec![DepthAtlasDraw {
                positions: vec![
                    vec4(0.0, 0.0, 0.5, 1.0),
                    vec4(1.0, 0.0, 0.5, 1.0),
                    vec4(0.0, 1.0, 0.5, 1.0),
                ],
                indices: vec![0, 1, 2],
                cull: CullFace::Back,
                polygon_offset: None,
            }],
        }
    }

    #[test]
    fn draws_passes_and_restores_target() {
        let mut gl = FakeGlContext::new();
        let mut stage = StageProgram::new(&mut gl);
        let mut atlas = DepthAtlasTarget::new(&mut gl);
        gl.clear_log();
        atlas.draw(&mut gl, &stage, 9, 64, 64, &[pass()]);
        gl.assert_contains("attach", |call| {
            matches!(call, GlCall::FramebufferTexture2D { texture: 9, .. })
        });
        gl.assert_contains("triangles", |call| {
            matches!(
                call,
                GlCall::DrawElements {
                    mode: TRIANGLES,
                    count: 3
                }
            )
        });
        gl.assert_contains("detach", |call| {
            matches!(call, GlCall::FramebufferTexture2D { texture: 0, .. })
        });
        gl.assert_contains("depth program restore", |call| {
            matches!(call, GlCall::UseProgram { .. })
        });
        atlas.close(&mut gl);
        stage.close(&mut gl);
    }

    #[test]
    #[should_panic(expected = "outside its image")]
    fn rejects_out_of_bounds_viewport() {
        let mut gl = FakeGlContext::new();
        let stage = StageProgram::new(&mut gl);
        let mut atlas = DepthAtlasTarget::new(&mut gl);
        let mut bad = pass();
        bad.viewport.width = 128.0;
        atlas.draw(&mut gl, &stage, 9, 64, 64, &[bad]);
    }

    #[test]
    #[should_panic(expected = "incomplete")]
    fn rejects_incomplete_triangles() {
        let mut gl = FakeGlContext::new();
        let stage = StageProgram::new(&mut gl);
        let mut atlas = DepthAtlasTarget::new(&mut gl);
        let mut bad = pass();
        bad.draws[0].indices.pop();
        atlas.draw(&mut gl, &stage, 9, 64, 64, &[bad]);
    }
}
