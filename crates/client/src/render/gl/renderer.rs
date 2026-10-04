//! Ordered GL backend (donor `src/render/gl/renderer.ts`).

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::sync::Arc;

use super::buffers::GeometryBuffer;
use super::depth_atlas::DepthAtlasTarget;
use super::fog::Q2FogPass;
use super::object_opacity::GlObjectOpacity;
use super::output_gamma::{output_gamma_table, GlOutputGamma};
use super::programs::StageProgram;
use super::textures::{with_pixel_store, GlTextures, PixelDirection};
use super::{
    FramebufferPrecision, GlContext, ALPHA_BITS, ALPHA_TEST, BACK, BACK_LEFT, BACK_RIGHT, BLEND, BLUE_BITS, CCW,
    CLIP_PLANE0, COLOR_ARRAY, COLOR_MATERIAL, CULL_FACE_CAP, DEPTH_BITS, DEPTH_TEST, DST_ALPHA, DST_COLOR, EQUAL, FILL,
    FRONT, FRONT_AND_BACK, GREEN_BITS, LEQUAL, LIGHTING, LINES as GL_LINES, MAX_TEXTURE_COORDS,
    MAX_TEXTURE_IMAGE_UNITS, MAX_TEXTURE_SIZE, MODELVIEW, ONE, ONE_MINUS_DST_ALPHA, ONE_MINUS_DST_COLOR,
    ONE_MINUS_SRC_ALPHA, ONE_MINUS_SRC_COLOR, POLYGON_OFFSET_FILL, PROJECTION, QUADS, RED_BITS, RENDERER, SCISSOR_TEST,
    SHADING_LANGUAGE_VERSION, SMOOTH, SRC_ALPHA, SRC_ALPHA_SATURATE, SRC_COLOR, STENCIL_BITS, STENCIL_BUFFER_BIT,
    STENCIL_TEST, STEREO, TEXTURE0, TEXTURE_2D, TEXTURE_BINDING_2D, TEXTURE_COORD_ARRAY, TRIANGLES, TRIANGLE_STRIP,
    UNSIGNED_BYTE, VENDOR, VERSION, VERTEX_ARRAY,
};
use crate::render::error::RenderError;
use crate::render::types::{
    AlphaTest, BatchFog, BatchLighting, BatchPrimitive, BatchVertices, BlendFactor, CullFace, DepthTest, DrawBatch,
    DrawBuffer, ImageResourceOperation, OrderedBackend, PairEnvironment, PolygonOffset, PreparedDraw, RenderOperation,
    RenderState, RenderViewState, RendererImage, ResourceOwner, TextureBinding,
};
use qa_core::math::Vec4;

fn blend_factor(factor: BlendFactor) -> u32 {
    match factor {
        BlendFactor::Zero => super::ZERO,
        BlendFactor::One => ONE,
        BlendFactor::SrcColor => SRC_COLOR,
        BlendFactor::OneMinusSrcColor => ONE_MINUS_SRC_COLOR,
        BlendFactor::DstColor => DST_COLOR,
        BlendFactor::OneMinusDstColor => ONE_MINUS_DST_COLOR,
        BlendFactor::SrcAlpha => SRC_ALPHA,
        BlendFactor::OneMinusSrcAlpha => ONE_MINUS_SRC_ALPHA,
        BlendFactor::DstAlpha => DST_ALPHA,
        BlendFactor::OneMinusDstAlpha => ONE_MINUS_DST_ALPHA,
        BlendFactor::SrcAlphaSaturate => SRC_ALPHA_SATURATE,
    }
}

fn draw_buffer_value(buffer: DrawBuffer) -> u32 {
    match buffer {
        DrawBuffer::Front => FRONT,
        DrawBuffer::Back => BACK,
        DrawBuffer::BackLeft => BACK_LEFT,
        DrawBuffer::BackRight => BACK_RIGHT,
    }
}

fn depth_test_value(test: crate::render::types::DepthTest) -> u32 {
    match test {
        crate::render::types::DepthTest::LessEqual => LEQUAL,
        crate::render::types::DepthTest::Equal => EQUAL,
        crate::render::types::DepthTest::Always => super::ALWAYS,
    }
}

fn finite32(value: f32) -> bool {
    value.is_finite()
}

fn position_values(position: &Vec4) -> [f32; 4] {
    [position.x, position.y, position.z, position.w]
}

/// Driver identification strings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DriverInfo {
    pub vendor: String,
    pub renderer: String,
    pub version: String,
    pub shading_language: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DrawPhase {
    Prepared,
    Active,
    Drawn,
    Cleaned,
}

/// Ordered GL backend over a [`GlContext`].
pub struct GlRenderer<C: GlContext> {
    gl: RefCell<C>,
    owner: ResourceOwner,
    program: StageProgram,
    textures: GlTextures,
    active_arrays: bool,
    idle_geometry: Option<GeometryBuffer>,
    depth_atlas: Option<DepthAtlasTarget>,
    fog: Option<Q2FogPass>,
    object_opacity: Option<GlObjectOpacity>,
    opacity_active: bool,
    output_gamma: RefCell<Option<GlOutputGamma>>,
    gamma: f32,
    gamma_finished: Cell<bool>,
    draw_buffer: u32,
    alpha_test: AlphaTest,
    texture_unit: Option<u32>,
    matrices_identity: bool,
    current_cull: Option<CullFace>,
    current_depth: Option<(DepthTest, bool)>,
    current_blend: Option<(BlendFactor, BlendFactor)>,
    range: Option<[f32; 2]>,
    offset: Option<Option<PolygonOffset>>,
    closed: bool,
    width: u32,
    height: u32,
    stencil_bits: u32,
    depth_bits: u32,
    color_bits: u32,
    alpha_bits: u32,
    max_texture_size: u32,
    texture_units: u32,
    stereo_enabled: bool,
    driver: DriverInfo,
}

impl<C: GlContext> GlRenderer<C> {
    pub fn new(mut gl: C, owner: ResourceOwner, width: u32, height: u32) -> Self {
        let driver = DriverInfo {
            vendor: gl.get_string(VENDOR),
            renderer: gl.get_string(RENDERER),
            version: gl.get_string(VERSION),
            shading_language: gl.get_string(SHADING_LANGUAGE_VERSION),
        };
        let stencil_bits = integer(&mut gl, STENCIL_BITS);
        let depth_bits = integer(&mut gl, DEPTH_BITS);
        let color_bits = integer(&mut gl, RED_BITS) + integer(&mut gl, GREEN_BITS) + integer(&mut gl, BLUE_BITS);
        let alpha_bits = integer(&mut gl, ALPHA_BITS);
        let max_texture_size = integer(&mut gl, MAX_TEXTURE_SIZE);
        let texture_units = integer(&mut gl, MAX_TEXTURE_COORDS).min(integer(&mut gl, MAX_TEXTURE_IMAGE_UNITS));
        let stereo_enabled = integer(&mut gl, STEREO) != 0;
        if texture_units < 4 || max_texture_size < 1 || depth_bits < 1 {
            panic!(
                "{}",
                RenderError::Backend(
                    "OpenGL renderer requires four texture coordinate units and a depth framebuffer".to_string()
                )
            );
        }
        let program = StageProgram::new(&mut gl);
        let textures = GlTextures::new(owner.clone(), max_texture_size);
        let mut renderer = Self {
            gl: RefCell::new(gl),
            owner,
            program,
            textures,
            active_arrays: false,
            idle_geometry: None,
            depth_atlas: None,
            fog: None,
            object_opacity: None,
            opacity_active: false,
            output_gamma: RefCell::new(None),
            gamma: 1.0,
            gamma_finished: Cell::new(false),
            draw_buffer: BACK,
            alpha_test: AlphaTest::None,
            texture_unit: None,
            matrices_identity: false,
            current_cull: None,
            current_depth: None,
            current_blend: None,
            range: None,
            offset: None,
            closed: false,
            width,
            height,
            stencil_bits,
            depth_bits,
            color_bits,
            alpha_bits,
            max_texture_size,
            texture_units,
            stereo_enabled,
            driver,
        };
        renderer.identity_matrices();
        renderer.gl.borrow_mut().front_face(CCW);
        renderer.gl.borrow_mut().shade_model(SMOOTH);
        renderer.gl.borrow_mut().polygon_mode(FRONT_AND_BACK, FILL);
        renderer.gl.borrow_mut().disable(LIGHTING);
        renderer.gl.borrow_mut().disable(COLOR_MATERIAL);
        renderer.gl.borrow_mut().disable(ALPHA_TEST);
        renderer.gl.borrow_mut().disable(STENCIL_TEST);
        renderer.gl.borrow_mut().disable(CLIP_PLANE0);
        renderer.gl.borrow_mut().disable(POLYGON_OFFSET_FILL);
        renderer.gl.borrow_mut().color_mask(true, true, true, true);
        renderer.gl.borrow_mut().depth_mask(true);
        renderer.gl.borrow_mut().clear_stencil(0);
        renderer.gl.borrow_mut().stencil_mask(0xFFFF_FFFF);
        renderer.gl.borrow_mut().color_4f(1.0, 1.0, 1.0, 1.0);
        renderer.disable_arrays();
        renderer
    }

    fn ensure_open(&self) {
        if self.closed {
            panic!("{}", RenderError::Backend("OpenGL renderer is closed".to_string()));
        }
    }

    /// Update the drawable size after a window resize.
    pub fn set_drawable_size(&mut self, width: u32, height: u32) {
        self.width = width;
        self.height = height;
    }

    #[must_use]
    pub fn driver(&self) -> &DriverInfo {
        &self.driver
    }

    /// Texture coordinate units (construction requires at least four).
    #[must_use]
    pub fn texture_units(&self) -> u32 {
        self.texture_units
    }

    pub fn set_output_gamma(&mut self, gamma: f32) {
        let table = output_gamma_table(gamma);
        self.ensure_open();
        if gamma == self.gamma {
            return;
        }
        if self.active_arrays {
            panic!(
                "{}",
                RenderError::Backend("OpenGL gamma cannot interrupt a prepared draw".to_string())
            );
        }
        self.invalidate_state();
        match table {
            None => {
                let mut slot = self.output_gamma.borrow_mut();
                if let Some(pass) = slot.as_mut() {
                    pass.restore(&mut *self.gl.borrow_mut(), self.draw_buffer);
                    pass.close(&mut *self.gl.borrow_mut());
                }
                *slot = None;
            }
            Some(table) => {
                let precision = self.precision();
                let mut slot = self.output_gamma.borrow_mut();
                if slot.is_none() {
                    let mut pass = GlOutputGamma::new(&mut *self.gl.borrow_mut(), precision, &table);
                    pass.bind(
                        &mut *self.gl.borrow_mut(),
                        self.draw_buffer,
                        self.width as i32,
                        self.height as i32,
                    );
                    *slot = Some(pass);
                } else if let Some(pass) = slot.as_mut() {
                    pass.update(&mut *self.gl.borrow_mut(), &table);
                }
            }
        }
        self.gamma = gamma;
        self.gamma_finished.set(false);
    }

    fn precision(&self) -> FramebufferPrecision {
        FramebufferPrecision {
            depth_bits: self.depth_bits as i32,
            stencil_bits: self.stencil_bits as i32,
            color_bits: self.color_bits as i32,
            alpha_bits: self.alpha_bits as i32,
        }
    }

    fn draw_target(&self, rendering: bool) {
        if self.opacity_active {
            if let Some(opacity) = self.object_opacity.as_ref() {
                opacity.bind_target(&mut *self.gl.borrow_mut());
            }
            return;
        }
        let has_gamma = self.output_gamma.borrow().is_some();
        if has_gamma {
            if self.width > self.max_texture_size || self.height > self.max_texture_size {
                panic!(
                    "{}",
                    RenderError::Backend("OpenGL gamma target exceeds maximum texture size".to_string())
                );
            }
            let mut slot = self.output_gamma.borrow_mut();
            let pass = slot
                .as_mut()
                .unwrap_or_else(|| panic!("{}", RenderError::Backend("OpenGL gamma pass is missing".to_string())));
            let changed = pass.bind(
                &mut *self.gl.borrow_mut(),
                self.draw_buffer,
                self.width as i32,
                self.height as i32,
            );
            if rendering || changed {
                self.gamma_finished.set(false);
            }
        }
    }

    fn select_texture(&mut self, unit: u32) {
        if self.texture_unit == Some(unit) {
            return;
        }
        self.texture_unit = None;
        self.gl.borrow_mut().active_texture(TEXTURE0 + unit);
        self.gl.borrow_mut().client_active_texture(TEXTURE0 + unit);
        self.texture_unit = Some(unit);
    }

    fn identity_matrices(&mut self) {
        if self.matrices_identity {
            return;
        }
        for matrix in [MODELVIEW, PROJECTION] {
            self.gl.borrow_mut().matrix_mode(matrix);
            self.gl.borrow_mut().load_identity();
        }
        self.matrices_identity = true;
    }

    fn invalidate_state(&mut self) {
        self.texture_unit = None;
        self.matrices_identity = false;
        self.current_cull = None;
        self.current_depth = None;
        self.current_blend = None;
        self.range = None;
        self.offset = None;
    }

    fn apply_cull(&mut self, cull: CullFace) {
        if self.current_cull == Some(cull) {
            return;
        }
        self.current_cull = None;
        match cull {
            CullFace::None => self.gl.borrow_mut().disable(CULL_FACE_CAP),
            CullFace::Back => {
                self.gl.borrow_mut().enable(CULL_FACE_CAP);
                self.gl.borrow_mut().cull_face(BACK);
            }
            CullFace::Front => {
                self.gl.borrow_mut().enable(CULL_FACE_CAP);
                self.gl.borrow_mut().cull_face(FRONT);
            }
        }
        self.current_cull = Some(cull);
    }

    fn apply_polygon_offset(&mut self, value: Option<PolygonOffset>) {
        match value {
            None => {
                if self.offset == Some(None) {
                    return;
                }
                self.offset = None;
                self.gl.borrow_mut().disable(POLYGON_OFFSET_FILL);
                self.offset = Some(None);
            }
            Some(offset) => {
                if !finite32(offset.factor) || !finite32(offset.units) {
                    panic!(
                        "{}",
                        RenderError::NotFinite("OpenGL polygon offset must be finite".to_string())
                    );
                }
                if self.offset == Some(Some(offset)) {
                    return;
                }
                self.offset = None;
                self.gl.borrow_mut().enable(POLYGON_OFFSET_FILL);
                self.gl.borrow_mut().polygon_offset(offset.factor, offset.units);
                self.offset = Some(Some(offset));
            }
        }
    }

    fn apply_depth_range(&mut self, range: [f32; 2]) {
        if !range.iter().all(|value| value.is_finite()) {
            panic!(
                "{}",
                RenderError::NotFinite("OpenGL depth range must be finite".to_string())
            );
        }
        if self.range == Some(range) {
            return;
        }
        self.range = None;
        self.gl
            .borrow_mut()
            .depth_range(f64::from(range[0]), f64::from(range[1]));
        self.range = Some(range);
    }

    fn apply_state(&mut self, state: &RenderState) {
        if state.blend.1 == BlendFactor::SrcAlphaSaturate {
            panic!(
                "{}",
                RenderError::BadBatch {
                    index: 0,
                    detail: "OpenGL destination blend cannot use source alpha saturate".to_string()
                }
            );
        }
        // Depth and blend states persist across batches, so skip the GL
        // calls when the request matches. Nothing ever disables the depth
        // test or enables the alpha test, so those toggles emit at most
        // once (open disables alpha test already).
        let depth = (state.depth_test, state.depth_write);
        if self.current_depth != Some(depth) {
            self.gl.borrow_mut().enable(DEPTH_TEST);
            self.gl.borrow_mut().depth_func(depth_test_value(state.depth_test));
            self.gl.borrow_mut().depth_mask(state.depth_write);
            self.current_depth = Some(depth);
        }
        if self.current_blend != Some(state.blend) {
            if state.blend == (BlendFactor::One, BlendFactor::Zero) {
                self.gl.borrow_mut().disable(BLEND);
            } else {
                self.gl.borrow_mut().enable(BLEND);
                self.gl
                    .borrow_mut()
                    .blend_func(blend_factor(state.blend.0), blend_factor(state.blend.1));
            }
            self.current_blend = Some(state.blend);
        }
        self.alpha_test = state.alpha_test;
        self.apply_cull(state.cull);
        self.apply_depth_range(state.depth_range);
        self.apply_polygon_offset(state.polygon_offset);
    }

    fn disable_arrays(&mut self) {
        self.gl.borrow_mut().disable_client_state(VERTEX_ARRAY);
        self.gl.borrow_mut().disable_client_state(COLOR_ARRAY);
        for unit in [3, 2, 1, 0] {
            self.select_texture(unit);
            self.gl.borrow_mut().disable_client_state(TEXTURE_COORD_ARRAY);
        }
        self.active_arrays = false;
    }

    fn resolve_output(&mut self) {
        let has_gamma = self.output_gamma.borrow().is_some();
        if has_gamma {
            if self.active_arrays {
                panic!(
                    "{}",
                    RenderError::Backend("OpenGL gamma cannot interrupt a prepared draw".to_string())
                );
            }
            self.draw_target(false);
            if !self.gamma_finished.get() {
                self.invalidate_state();
                let mut slot = self.output_gamma.borrow_mut();
                if let Some(pass) = slot.as_mut() {
                    pass.finish(&mut *self.gl.borrow_mut(), self.draw_buffer);
                }
                self.gamma_finished.set(true);
            } else {
                let mut slot = self.output_gamma.borrow_mut();
                if let Some(pass) = slot.as_mut() {
                    pass.select_default(&mut *self.gl.borrow_mut(), self.draw_buffer);
                }
            }
        }
    }

    /// Read back top-left-origin RGBA pixels for captures.
    #[must_use]
    pub fn read_pixels(&mut self) -> Vec<u8> {
        self.ensure_open();
        self.resolve_output();
        let (width, height) = (self.width as usize, self.height as usize);
        let mut pixels = vec![0u8; width * height * 4];
        let mut top_down = vec![0u8; pixels.len()];
        with_pixel_store(&mut *self.gl.borrow_mut(), PixelDirection::Pack, |gl| {
            gl.read_pixels_bytes(
                0,
                0,
                width as i32,
                height as i32,
                super::RGBA,
                UNSIGNED_BYTE,
                &mut pixels,
            );
        });
        for row in 0..height {
            let src = row * width * 4;
            let dst = (height - row - 1) * width * 4;
            top_down[dst..dst + width * 4].copy_from_slice(&pixels[src..src + width * 4]);
        }
        top_down
    }

    /// Read back a depth32f image.
    #[must_use]
    pub fn read_depth_image(&mut self, image: &RendererImage) -> crate::render::types::DepthImageLevel {
        self.ensure_open();
        let texture = self.textures.registered(image).clone();
        if !texture.is_depth() {
            panic!(
                "{}",
                RenderError::Backend("OpenGL depth readback requires a depth32f image".to_string())
            );
        }
        self.select_texture(2);
        let previous = integer(&mut *self.gl.borrow_mut(), TEXTURE_BINDING_2D);
        let mut pixels = vec![0.0f32; image.width as usize * image.height as usize];
        self.gl.borrow_mut().bind_texture(TEXTURE_2D, texture.name);
        with_pixel_store(&mut *self.gl.borrow_mut(), PixelDirection::Pack, |gl| {
            gl.get_tex_image_floats(TEXTURE_2D, 0, super::DEPTH_COMPONENT, super::FLOAT, &mut pixels);
        });
        self.gl.borrow_mut().bind_texture(TEXTURE_2D, previous);
        self.select_texture(0);
        crate::render::types::DepthImageLevel {
            width: image.width,
            height: image.height,
            pixels,
        }
    }

    #[must_use]
    pub fn get_error(&mut self) -> u32 {
        self.ensure_open();
        self.gl.borrow_mut().get_error()
    }

    /// Resolve output and swap front/back buffers.
    pub fn present(&mut self) {
        self.ensure_open();
        self.resolve_output();
        self.gl.borrow_mut().swap_buffers();
        self.invalidate_state();
    }

    fn draw_sky_side(&mut self, image: &RendererImage, color: &Vec4, strips: &[Vec<crate::render::types::SkyVertex>]) {
        for strip in strips {
            if strip.len() < 2 || strip.len() % 2 != 0 {
                panic!(
                    "{}",
                    RenderError::BadBatch {
                        index: strip.len(),
                        detail: "OpenGL sky strips require paired row vertices".to_string()
                    }
                );
            }
            for vertex in strip {
                let values = [
                    vertex.position.x,
                    vertex.position.y,
                    vertex.position.z,
                    vertex.position.w,
                    vertex.tex_coord.x,
                    vertex.tex_coord.y,
                ];
                if !values.iter().all(|value| finite32(*value)) {
                    panic!(
                        "{}",
                        RenderError::NotFinite("OpenGL sky vertices must be finite".to_string())
                    );
                }
            }
        }
        self.identity_matrices();
        let alpha = self.alpha_test;
        self.program.use_stage(
            &mut *self.gl.borrow_mut(),
            None,
            alpha,
            &BatchLighting::Vertex,
            false,
            None,
        );
        self.select_texture(0);
        self.textures
            .bind(&mut *self.gl.borrow_mut(), &TextureBinding::BindImage(image.clone()));
        self.gl.borrow_mut().color_4f(color.x, color.y, color.z, color.w);
        for strip in strips {
            self.gl.borrow_mut().begin(TRIANGLE_STRIP);
            for vertex in strip {
                self.gl
                    .borrow_mut()
                    .tex_coord_2f(vertex.tex_coord.x, vertex.tex_coord.y);
                self.gl.borrow_mut().vertex_4f(
                    vertex.position.x,
                    vertex.position.y,
                    vertex.position.z,
                    vertex.position.w,
                );
            }
            self.gl.borrow_mut().end();
        }
    }

    fn draw_shadow(&mut self, positions: &[Vec4], white_image: &RendererImage, finish: bool, mirror: bool) {
        if self.stencil_bits < 4 {
            panic!(
                "{}",
                RenderError::Backend("OpenGL stencil shadows require at least four stencil bits".to_string())
            );
        }
        for position in positions {
            if !position_values(position).iter().all(|value| finite32(*value)) {
                panic!(
                    "{}",
                    RenderError::NotFinite("OpenGL shadow positions must be finite".to_string())
                );
            }
        }
        self.select_texture(0);
        self.textures.bind(
            &mut *self.gl.borrow_mut(),
            &TextureBinding::BindImage(white_image.clone()),
        );
        self.identity_matrices();
        self.program.use_stage(
            &mut *self.gl.borrow_mut(),
            None,
            AlphaTest::None,
            &BatchLighting::Vertex,
            false,
            None,
        );
        self.alpha_test = AlphaTest::None;
        self.invalidate_state();
        self.gl.borrow_mut().enable(DEPTH_TEST);
        self.gl.borrow_mut().depth_func(LEQUAL);
        self.gl.borrow_mut().enable(STENCIL_TEST);
        if finish {
            self.gl.borrow_mut().stencil_func(super::NOTEQUAL, 0, 255);
            self.gl.borrow_mut().disable(CLIP_PLANE0);
            self.gl.borrow_mut().disable(CULL_FACE_CAP);
            self.gl.borrow_mut().enable(BLEND);
            self.gl.borrow_mut().blend_func(DST_COLOR, super::ZERO);
            self.gl.borrow_mut().depth_mask(true);
            self.gl.borrow_mut().color_3f(0.6, 0.6, 0.6);
            draw_positions(&mut *self.gl.borrow_mut(), QUADS, positions);
            self.gl.borrow_mut().color_3f(1.0, 1.0, 1.0);
            self.gl.borrow_mut().disable(STENCIL_TEST);
        } else {
            self.gl.borrow_mut().enable(CULL_FACE_CAP);
            self.gl.borrow_mut().disable(BLEND);
            self.gl.borrow_mut().depth_mask(false);
            self.gl.borrow_mut().color_3f(0.2, 0.2, 0.2);
            self.gl.borrow_mut().color_mask(false, false, false, false);
            self.gl.borrow_mut().stencil_func(super::ALWAYS, 1, 255);
            self.gl.borrow_mut().cull_face(if mirror { FRONT } else { BACK });
            self.gl.borrow_mut().stencil_op(super::KEEP, super::KEEP, super::INCR);
            draw_positions(&mut *self.gl.borrow_mut(), TRIANGLES, positions);
            self.gl.borrow_mut().cull_face(if mirror { BACK } else { FRONT });
            self.gl.borrow_mut().stencil_op(super::KEEP, super::KEEP, super::DECR);
            draw_positions(&mut *self.gl.borrow_mut(), TRIANGLES, positions);
            self.gl.borrow_mut().color_mask(true, true, true, true);
        }
    }
}

fn integer<C: GlContext>(gl: &mut C, name: u32) -> u32 {
    let mut out = [0];
    gl.get_integerv(name, &mut out);
    if out[0] < 0 {
        panic!(
            "{}",
            RenderError::Backend("OpenGL returned an invalid capability".to_string())
        );
    }
    out[0] as u32
}

fn draw_positions<C: GlContext>(gl: &mut C, mode: u32, positions: &[Vec4]) {
    gl.begin(mode);
    for position in positions {
        gl.vertex_4f(position.x, position.y, position.z, position.w);
    }
    gl.end();
}

impl<C: GlContext> OrderedBackend for GlRenderer<C> {
    type Prepared<'a>
        = PreparedGl<'a, C>
    where
        Self: 'a;

    fn owner(&self) -> &ResourceOwner {
        &self.owner
    }

    fn width(&self) -> u32 {
        self.width
    }

    fn height(&self) -> u32 {
        self.height
    }

    fn stencil_bits(&self) -> u32 {
        self.stencil_bits
    }

    fn apply_image_resource(&mut self, operation: &ImageResourceOperation) {
        self.ensure_open();
        self.select_texture(0);
        let previous = integer(&mut *self.gl.borrow_mut(), TEXTURE_BINDING_2D);
        let released = match operation {
            ImageResourceOperation::ReleaseImage { image } => Some(self.textures.registered(image).name),
            _ => None,
        };
        self.textures.apply(&mut *self.gl.borrow_mut(), operation);
        self.gl
            .borrow_mut()
            .bind_texture(TEXTURE_2D, if released == Some(previous) { 0 } else { previous });
    }

    fn select_draw_buffer(&mut self, buffer: DrawBuffer, clear: bool) {
        self.ensure_open();
        if buffer == DrawBuffer::BackRight && !self.stereo_enabled {
            panic!(
                "{}",
                RenderError::Backend("OpenGL stereo draw buffer requires a stereo context".to_string())
            );
        }
        self.draw_buffer = draw_buffer_value(buffer);
        let has_gamma = self.output_gamma.borrow().is_some();
        if has_gamma {
            self.draw_target(true);
        } else {
            self.gl.borrow_mut().draw_buffer(self.draw_buffer);
        }
        if clear {
            self.gl.borrow_mut().clear_color(1.0, 0.0, 0.5, 1.0);
            self.gl
                .borrow_mut()
                .clear(super::COLOR_BUFFER_BIT | super::DEPTH_BUFFER_BIT);
        }
    }

    fn set_overdraw_measurement(&mut self, enabled: bool) {
        self.ensure_open();
        self.draw_target(true);
        if !enabled {
            self.gl.borrow_mut().disable(STENCIL_TEST);
            return;
        }
        if self.stencil_bits == 0 {
            panic!(
                "{}",
                RenderError::Backend("OpenGL overdraw measurement requires a stencil framebuffer".to_string())
            );
        }
        self.gl.borrow_mut().enable(STENCIL_TEST);
        self.gl.borrow_mut().stencil_mask(0xFFFF_FFFF);
        self.gl.borrow_mut().clear_stencil(0);
        self.gl.borrow_mut().stencil_func(super::ALWAYS, 0, 0xFFFF_FFFF);
        self.gl.borrow_mut().stencil_op(super::KEEP, super::INCR, super::INCR);
    }

    fn read_stencil_overdraw(&self, destination: &mut [u8]) {
        self.ensure_open();
        self.draw_target(false);
        let (width, height) = (self.width as usize, self.height as usize);
        if destination.len() < width * height {
            panic!(
                "{}",
                RenderError::BadDimensions {
                    width: self.width,
                    height: self.height,
                    detail: "OpenGL stencil destination is too small".to_string()
                }
            );
        }
        with_pixel_store(&mut *self.gl.borrow_mut(), PixelDirection::Pack, |gl| {
            gl.read_pixels_bytes(
                0,
                0,
                width as i32,
                height as i32,
                super::STENCIL_INDEX,
                UNSIGNED_BYTE,
                &mut destination[..width * height],
            );
        });
    }

    fn read_depth_pixel(&self, window_x: i32, window_y: i32) -> f32 {
        self.ensure_open();
        self.draw_target(false);
        if window_x < 0 || window_y < 0 || window_x >= self.width as i32 || window_y >= self.height as i32 {
            panic!(
                "{}",
                RenderError::BadDimensions {
                    width: self.width,
                    height: self.height,
                    detail: "OpenGL depth coordinates are outside the framebuffer".to_string()
                }
            );
        }
        let mut pixel = [0.0f32; 1];
        with_pixel_store(&mut *self.gl.borrow_mut(), PixelDirection::Pack, |gl| {
            gl.read_pixels_floats(
                window_x,
                window_y,
                1,
                1,
                super::DEPTH_COMPONENT,
                super::FLOAT,
                &mut pixel,
            );
        });
        pixel[0]
    }

    fn begin_view(&mut self, view: &RenderViewState) {
        self.invalidate_state();
        self.ensure_open();
        self.draw_target(true);
        let bottom = self.height as f32 - view.viewport.y - view.viewport.height;
        for value in [
            view.viewport.x,
            view.viewport.y,
            view.viewport.width,
            view.viewport.height,
            bottom,
        ] {
            if value.fract() != 0.0 || value < f64::from(i32::MIN) as f32 || value > f64::from(i32::MAX) as f32 {
                panic!(
                    "{}",
                    RenderError::BadViewport("OpenGL viewport requires positive int32 dimensions".to_string())
                );
            }
        }
        if view.viewport.width < 1.0 || view.viewport.height < 1.0 {
            panic!(
                "{}",
                RenderError::BadViewport("OpenGL viewport requires positive int32 dimensions".to_string())
            );
        }
        let (x, width, height) = (
            view.viewport.x as i32,
            view.viewport.width as i32,
            view.viewport.height as i32,
        );
        self.gl.borrow_mut().viewport(x, bottom as i32, width, height);
        self.gl.borrow_mut().enable(SCISSOR_TEST);
        self.gl.borrow_mut().scissor(x, bottom as i32, width, height);
        self.identity_matrices();
        match &view.clip_plane {
            None => self.gl.borrow_mut().disable(CLIP_PLANE0),
            Some(plane) => {
                let equation = position_values(plane);
                if !equation.iter().all(|value| value.is_finite()) {
                    panic!(
                        "{}",
                        RenderError::NotFinite("OpenGL clip plane must be finite".to_string())
                    );
                }
                self.gl.borrow_mut().clip_plane(
                    CLIP_PLANE0,
                    &[
                        f64::from(equation[0]),
                        f64::from(equation[1]),
                        f64::from(equation[2]),
                        f64::from(equation[3]),
                    ],
                );
                self.gl.borrow_mut().enable(CLIP_PLANE0);
            }
        }
        if let Some(clear) = &view.clear {
            if !clear.depth.is_finite()
                || clear
                    .color
                    .as_ref()
                    .is_some_and(|color| !position_values(color).iter().all(|value| finite32(*value)))
            {
                panic!(
                    "{}",
                    RenderError::NotFinite("OpenGL clear values must be finite".to_string())
                );
            }
            self.gl.borrow_mut().depth_mask(true);
            self.gl.borrow_mut().clear_depth(f64::from(clear.depth));
            if let Some(color) = &clear.color {
                self.gl.borrow_mut().clear_color(color.x, color.y, color.z, color.w);
            }
            if clear.stencil {
                self.gl.borrow_mut().stencil_mask(0xFFFF_FFFF);
                self.gl.borrow_mut().clear_stencil(0);
            }
            let mut mask = super::DEPTH_BUFFER_BIT;
            if clear.color.is_some() {
                mask |= super::COLOR_BUFFER_BIT;
            }
            if clear.stencil {
                mask |= STENCIL_BUFFER_BIT;
            }
            self.gl.borrow_mut().clear(mask);
        }
    }

    fn with_object_opacity(&mut self, opacity: f32, draw: impl FnOnce(&mut Self)) {
        self.ensure_open();
        if !opacity.is_finite() || !(0.0..=1.0).contains(&opacity) {
            panic!(
                "{}",
                RenderError::BadDimensions {
                    width: 0,
                    height: 0,
                    detail: "Object opacity must be in 0..1".to_string()
                }
            );
        }
        if self.opacity_active {
            panic!(
                "{}",
                RenderError::Backend("Nested OpenGL object opacity is unsupported".to_string())
            );
        }
        if opacity == 0.0 {
            return;
        }
        if opacity == 1.0 {
            draw(self);
            return;
        }
        if self.active_arrays {
            panic!(
                "{}",
                RenderError::Backend("Object opacity cannot interrupt a prepared draw".to_string())
            );
        }
        if self.width > self.max_texture_size || self.height > self.max_texture_size {
            panic!(
                "{}",
                RenderError::Backend("OpenGL opacity target exceeds maximum texture size".to_string())
            );
        }
        self.draw_target(true);
        self.invalidate_state();
        if self.object_opacity.is_none() {
            let precision = self.precision();
            let pass = GlObjectOpacity::new(&mut *self.gl.borrow_mut(), precision);
            self.object_opacity = Some(pass);
        }
        let session = {
            let pass = self
                .object_opacity
                .as_mut()
                .unwrap_or_else(|| panic!("{}", RenderError::Backend("OpenGL opacity pass is missing".to_string())));
            pass.begin(&mut *self.gl.borrow_mut(), self.width as i32, self.height as i32)
        };
        self.opacity_active = true;
        {
            let pass = self
                .object_opacity
                .as_mut()
                .unwrap_or_else(|| panic!("{}", RenderError::Backend("OpenGL opacity pass is missing".to_string())));
            pass.bind_target(&mut *self.gl.borrow_mut());
        }
        self.invalidate_state();
        draw(self);
        self.opacity_active = false;
        {
            let pass = self
                .object_opacity
                .as_mut()
                .unwrap_or_else(|| panic!("{}", RenderError::Backend("OpenGL opacity pass is missing".to_string())));
            pass.finish(
                &mut *self.gl.borrow_mut(),
                &session,
                opacity,
                self.width as i32,
                self.height as i32,
            );
        }
        self.invalidate_state();
    }

    fn draw_immediate(&mut self, operation: &RenderOperation) {
        self.ensure_open();
        self.draw_target(true);
        match operation {
            RenderOperation::Draw(batches) => {
                for batch in batches {
                    self.draw_batch(batch);
                }
            }
            RenderOperation::ObjectOpacity { opacity, batches } => {
                self.with_object_opacity(*opacity, |renderer| {
                    for batch in batches {
                        renderer.draw_batch(batch);
                    }
                });
            }
            RenderOperation::Q2Fog(operation) => {
                if self.active_arrays {
                    panic!(
                        "{}",
                        RenderError::Backend("OpenGL fog cannot interrupt a prepared draw".to_string())
                    );
                }
                self.invalidate_state();
                if self.fog.is_none() {
                    let pass = Q2FogPass::new(&mut *self.gl.borrow_mut());
                    self.fog = Some(pass);
                }
                let pass = self
                    .fog
                    .as_mut()
                    .unwrap_or_else(|| panic!("{}", RenderError::Backend("OpenGL fog pass is missing".to_string())));
                pass.draw(
                    &mut *self.gl.borrow_mut(),
                    operation,
                    self.width as i32,
                    self.height as i32,
                );
            }
            RenderOperation::DepthAtlas { image, passes } => {
                if self.active_arrays {
                    panic!(
                        "{}",
                        RenderError::Backend("OpenGL depth atlas cannot interrupt a prepared draw".to_string())
                    );
                }
                let texture = self.textures.registered(image).clone();
                if !texture.is_depth() {
                    panic!(
                        "{}",
                        RenderError::Backend("OpenGL depth atlas target requires a depth32f image".to_string())
                    );
                }
                self.invalidate_state();
                if self.depth_atlas.is_none() {
                    let target = DepthAtlasTarget::new(&mut *self.gl.borrow_mut());
                    self.depth_atlas = Some(target);
                }
                let target = self
                    .depth_atlas
                    .as_mut()
                    .unwrap_or_else(|| panic!("{}", RenderError::Backend("OpenGL depth atlas is missing".to_string())));
                target.draw(
                    &mut *self.gl.borrow_mut(),
                    &self.program,
                    texture.name,
                    image.width as i32,
                    image.height as i32,
                    passes,
                );
            }
            RenderOperation::DepthRange(range) => self.apply_depth_range(*range),
            RenderOperation::Cull(cull) => self.apply_cull(*cull),
            RenderOperation::PolygonOffset(value) => self.apply_polygon_offset(*value),
            RenderOperation::DisablePortalClip => self.gl.borrow_mut().disable(CLIP_PLANE0),
            RenderOperation::SkySide { image, color, strips } => self.draw_sky_side(image, color, strips),
            RenderOperation::ShadowVolume {
                positions,
                indices,
                mirror,
                white_image,
            } => {
                let resolved: Vec<Vec4> = indices
                    .iter()
                    .map(|index| {
                        positions.get(*index as usize).copied().unwrap_or_else(|| {
                            panic!(
                                "{}",
                                RenderError::BadBatch {
                                    index: *index as usize,
                                    detail: "OpenGL shadow vertex index is invalid".to_string()
                                }
                            )
                        })
                    })
                    .collect();
                if !resolved.len().is_multiple_of(3) {
                    panic!(
                        "{}",
                        RenderError::BadBatch {
                            index: resolved.len(),
                            detail: "OpenGL shadow triangles are incomplete".to_string()
                        }
                    );
                }
                self.draw_shadow(&resolved, white_image, false, *mirror);
            }
            RenderOperation::ShadowFinish { positions, white_image } => {
                self.draw_shadow(positions, white_image, true, false);
            }
        }
    }

    fn prepare_geometry(&mut self, batch: &DrawBatch) -> Self::Prepared<'_> {
        self.ensure_open();
        let mut buffer = self.idle_geometry.take().unwrap_or_default();
        buffer.pack(batch);
        let (paired, environment) = match &batch.vertices {
            BatchVertices::Single(_) => (false, None),
            BatchVertices::Pair { second_texture, .. } => (true, Some(second_texture.environment)),
        };
        let (mode, line_width) = match batch.primitive {
            BatchPrimitive::Triangles => (TRIANGLES, 1.0),
            BatchPrimitive::Lines { line_width } => (GL_LINES, line_width),
        };
        PreparedGl {
            renderer: self,
            buffer: Some(buffer),
            state: batch.state,
            environment,
            lighting: batch.lighting.clone(),
            fog: batch.fog,
            luminance_alpha: batch.luminance_alpha,
            mode,
            line_width,
            paired,
            phase: DrawPhase::Prepared,
            next_unit: 0,
            resolved: HashMap::new(),
        }
    }

    fn clear_color_buffer(&mut self) {
        self.ensure_open();
        self.draw_target(true);
        self.gl.borrow_mut().clear(super::COLOR_BUFFER_BIT);
    }

    fn draw_show_image(&mut self, image: &RendererImage, rect: &crate::render::types::Rect, proportional: bool) {
        self.ensure_open();
        self.draw_target(true);
        let _ = self.textures.registered(image);
        let width = rect.width * if proportional { image.width as f32 / 512.0 } else { 1.0 };
        let height = rect.height * if proportional { image.height as f32 / 512.0 } else { 1.0 };
        if ![rect.x, rect.y, width, height, rect.x + width, rect.y + height]
            .iter()
            .all(|value| finite32(*value))
        {
            panic!(
                "{}",
                RenderError::NotFinite("OpenGL image grid rectangle must be finite".to_string())
            );
        }
        self.matrices_identity = false;
        self.gl.borrow_mut().matrix_mode(PROJECTION);
        self.gl.borrow_mut().load_identity();
        self.gl
            .borrow_mut()
            .ortho(0.0, f64::from(self.width), f64::from(self.height), 0.0, 0.0, 1.0);
        self.gl.borrow_mut().matrix_mode(MODELVIEW);
        self.gl.borrow_mut().load_identity();
        let alpha = self.alpha_test;
        self.program.use_stage(
            &mut *self.gl.borrow_mut(),
            None,
            alpha,
            &BatchLighting::Vertex,
            false,
            None,
        );
        self.select_texture(0);
        self.textures
            .bind(&mut *self.gl.borrow_mut(), &TextureBinding::BindImage(image.clone()));
        self.gl.borrow_mut().color_4f(1.0, 1.0, 1.0, 1.0);
        self.gl.borrow_mut().begin(QUADS);
        self.gl.borrow_mut().tex_coord_2f(0.0, 0.0);
        self.gl.borrow_mut().vertex_2f(rect.x, rect.y);
        self.gl.borrow_mut().tex_coord_2f(1.0, 0.0);
        self.gl.borrow_mut().vertex_2f(rect.x + width, rect.y);
        self.gl.borrow_mut().tex_coord_2f(1.0, 1.0);
        self.gl.borrow_mut().vertex_2f(rect.x + width, rect.y + height);
        self.gl.borrow_mut().tex_coord_2f(0.0, 1.0);
        self.gl.borrow_mut().vertex_2f(rect.x, rect.y + height);
        self.gl.borrow_mut().end();
    }

    fn finish(&mut self) {
        self.ensure_open();
        self.resolve_output();
        self.gl.borrow_mut().finish();
    }

    fn close(&mut self) {
        if self.closed {
            return;
        }
        self.invalidate_state();
        self.disable_arrays();
        self.idle_geometry = None;
        {
            let mut slot = self.output_gamma.borrow_mut();
            if let Some(pass) = slot.as_mut() {
                pass.close(&mut *self.gl.borrow_mut());
            }
            *slot = None;
        }
        if let Some(pass) = self.object_opacity.as_mut() {
            pass.close(&mut *self.gl.borrow_mut());
        }
        self.object_opacity = None;
        if let Some(target) = self.depth_atlas.as_mut() {
            target.close(&mut *self.gl.borrow_mut());
        }
        self.depth_atlas = None;
        if let Some(pass) = self.fog.as_mut() {
            pass.close(&mut *self.gl.borrow_mut());
        }
        self.fog = None;
        self.invalidate_state();
        self.textures.close(&mut *self.gl.borrow_mut());
        for unit in [3, 2, 1, 0] {
            self.select_texture(unit);
            self.gl.borrow_mut().bind_texture(TEXTURE_2D, 0);
        }
        self.program.close(&mut *self.gl.borrow_mut());
        self.closed = true;
    }
}

/// Prepared draw borrowing the renderer.
pub struct PreparedGl<'a, C: GlContext> {
    renderer: &'a mut GlRenderer<C>,
    buffer: Option<GeometryBuffer>,
    state: RenderState,
    environment: Option<PairEnvironment>,
    lighting: BatchLighting,
    fog: Option<BatchFog>,
    luminance_alpha: bool,
    mode: u32,
    line_width: f32,
    paired: bool,
    phase: DrawPhase,
    next_unit: u32,
    resolved: HashMap<usize, RendererImage>,
}

impl<C: GlContext> PreparedGl<'_, C> {
    fn resolve_binding(&mut self, binding: &TextureBinding) -> TextureBinding {
        match binding {
            TextureBinding::DynamicImage(source) => {
                let key = Arc::as_ptr(source) as *const () as usize;
                if let Some(image) = self.resolved.get(&key) {
                    return TextureBinding::BindImage(image.clone());
                }
                let mut apply = |operation: ImageResourceOperation| self.renderer.apply_image_resource(&operation);
                let image = source.resolve(&mut apply);
                self.resolved.insert(key, image.clone());
                TextureBinding::BindImage(image)
            }
            other => other.clone(),
        }
    }
}

impl<C: GlContext> PreparedDraw for PreparedGl<'_, C> {
    fn begin(&mut self) {
        self.renderer.ensure_open();
        if self.phase != DrawPhase::Prepared || self.renderer.active_arrays {
            panic!(
                "{}",
                RenderError::Backend("OpenGL prepared draw is already active".to_string())
            );
        }
        let atlas = match &self.lighting {
            BatchLighting::Q2World { atlas: Some(atlas), .. } => {
                Some(self.renderer.textures.registered(&atlas.image).clone())
            }
            BatchLighting::Q2ModelShadow { atlas, .. } => Some(self.renderer.textures.registered(&atlas.image).clone()),
            _ => None,
        };
        if atlas.as_ref().is_some_and(|atlas| !atlas.is_depth()) {
            panic!(
                "{}",
                RenderError::Backend("Q2 shadow atlas requires a depth32f image".to_string())
            );
        }
        self.renderer.draw_target(true);
        let state = self.state;
        self.renderer.apply_state(&state);
        self.renderer.identity_matrices();
        let environment = self.environment;
        let alpha = self.state.alpha_test;
        let lighting = self.lighting.clone();
        let luminance = self.luminance_alpha;
        let fog = self.fog;
        self.renderer.program.use_stage(
            &mut *self.renderer.gl.borrow_mut(),
            environment,
            alpha,
            &lighting,
            luminance,
            fog,
        );
        self.renderer.active_arrays = true;
        self.phase = DrawPhase::Active;
        let buffer = self.buffer.as_ref().unwrap_or_else(|| {
            panic!(
                "{}",
                RenderError::Backend("OpenGL prepared draw lost its geometry".to_string())
            )
        });
        self.renderer.gl.borrow_mut().enable_client_state(VERTEX_ARRAY);
        self.renderer.gl.borrow_mut().enable_client_state(COLOR_ARRAY);
        if !buffer.arrays().positions.is_empty() {
            self.renderer
                .gl
                .borrow_mut()
                .vertex_pointer(4, 0, &buffer.arrays().positions);
            self.renderer
                .gl
                .borrow_mut()
                .color_pointer(4, 0, &buffer.arrays().colors);
        }
        for (unit, world) in [(2u32, true), (3, false)] {
            self.renderer.select_texture(unit);
            let values = if world {
                &buffer.arrays().world_positions
            } else {
                &buffer.arrays().normals
            };
            if values.is_empty() {
                self.renderer.gl.borrow_mut().disable_client_state(TEXTURE_COORD_ARRAY);
            } else {
                self.renderer.gl.borrow_mut().enable_client_state(TEXTURE_COORD_ARRAY);
                self.renderer.gl.borrow_mut().tex_coord_pointer(3, 0, values);
            }
        }
        if let Some(atlas) = atlas {
            self.renderer.select_texture(2);
            self.renderer.gl.borrow_mut().bind_texture(TEXTURE_2D, atlas.name);
        }
        self.renderer.select_texture(0);
        self.renderer.gl.borrow_mut().line_width(self.line_width);
    }

    fn apply_texture(&mut self, unit: u32, binding: &TextureBinding) {
        self.renderer.ensure_open();
        if self.phase != DrawPhase::Active || unit != self.next_unit || unit > 1 || unit == 1 && !self.paired {
            panic!(
                "{}",
                RenderError::Backend("OpenGL prepared texture order is invalid".to_string())
            );
        }
        let resolved = self.resolve_binding(binding);
        self.renderer.select_texture(unit);
        self.renderer
            .textures
            .bind(&mut *self.renderer.gl.borrow_mut(), &resolved);
        let buffer = self.buffer.as_ref().unwrap_or_else(|| {
            panic!(
                "{}",
                RenderError::Backend("OpenGL prepared draw lost its geometry".to_string())
            )
        });
        let coordinates = if unit == 0 {
            &buffer.arrays().coordinates
        } else {
            &buffer.arrays().coordinates2
        };
        self.renderer.gl.borrow_mut().enable_client_state(TEXTURE_COORD_ARRAY);
        if !coordinates.is_empty() {
            self.renderer.gl.borrow_mut().tex_coord_pointer(2, 0, coordinates);
        }
        self.next_unit += 1;
    }

    fn draw(&mut self) {
        self.renderer.ensure_open();
        if self.phase != DrawPhase::Active || self.next_unit != u32::from(self.paired) + 1 {
            panic!(
                "{}",
                RenderError::Backend("OpenGL prepared draw has unapplied texture slots".to_string())
            );
        }
        let buffer = self.buffer.as_ref().unwrap_or_else(|| {
            panic!(
                "{}",
                RenderError::Backend("OpenGL prepared draw lost its geometry".to_string())
            )
        });
        if !buffer.arrays().indices.is_empty() {
            self.renderer
                .gl
                .borrow_mut()
                .draw_elements(self.mode, &buffer.arrays().indices);
        }
        self.phase = DrawPhase::Drawn;
    }

    fn cleanup(&mut self) {
        if self.phase == DrawPhase::Cleaned {
            return;
        }
        if self.renderer.closed {
            self.phase = DrawPhase::Cleaned;
            return;
        }
        self.renderer.ensure_open();
        if self.phase != DrawPhase::Prepared {
            self.renderer.disable_arrays();
            self.renderer.gl.borrow_mut().line_width(1.0);
        }
        self.phase = DrawPhase::Cleaned;
        if let Some(buffer) = self.buffer.take() {
            self.renderer.idle_geometry = Some(buffer);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::{
        FakeGlContext, GlCall, CCW, LINES, NEAREST, QUADS, SMOOTH, TEXTURE0, TRIANGLES, TRIANGLE_STRIP,
    };
    use super::*;
    use crate::render::types::{
        BatchFog, DepthAtlasDraw, DepthAtlasPass, DepthImageLevel, DynamicImageSource, FogEffect, ImageLevel,
        ImageSource, LevelContent, MultitextureVertex, Q2Fog, Q2FogOperation, Q2HeightFog, Q2HeightStop, Rect,
        RenderCamera, RenderImage, RenderVertex, SkyVertex, TextureBundle, TextureFilter, TextureSampling, ViewClear,
        ViewClip,
    };
    use qa_core::identity::IdentityOwner;
    use qa_core::math::{vec2, vec3, vec4};

    fn owner() -> ResourceOwner {
        let authority = IdentityOwner::create("gl-renderer").unwrap();
        ResourceOwner::new(11, authority.session().clone(), 0)
    }

    fn renderer() -> GlRenderer<FakeGlContext> {
        GlRenderer::new(FakeGlContext::new(), owner(), 64, 64)
    }

    fn handle(owner: &ResourceOwner, ordinal: u32, width: u32, height: u32) -> RendererImage {
        RendererImage {
            owner: owner.clone(),
            ordinal,
            source: ImageSource::Generated {
                name: "test".to_string(),
            },
            width,
            height,
        }
    }

    fn register_rgba(renderer: &mut GlRenderer<FakeGlContext>, owner: &ResourceOwner, ordinal: u32) -> RendererImage {
        let image = handle(owner, ordinal, 2, 2);
        renderer.apply_image_resource(&ImageResourceOperation::CreateImage {
            image: image.clone(),
            content: RenderImage::Rgba8 {
                levels: vec![ImageLevel {
                    width: 2,
                    height: 2,
                    pixels: vec![128; 16],
                }],
                border_color: vec4(0.0, 0.0, 0.0, 1.0),
            },
            sampling: TextureSampling {
                repeat: true,
                filter: TextureFilter::Linear,
            },
        });
        image
    }

    fn triangle_batch(image: &RendererImage) -> DrawBatch {
        DrawBatch {
            fog: None,
            luminance_alpha: false,
            indices: vec![0, 1, 2],
            texture: TextureBinding::BindImage(image.clone()),
            state: RenderState::opaque(CullFace::Back),
            lighting: BatchLighting::Vertex,
            primitive: BatchPrimitive::Triangles,
            vertices: BatchVertices::Single(vec![
                RenderVertex {
                    position: vec4(-1.0, -1.0, 0.0, 1.0),
                    tex_coord: vec2(0.0, 0.0),
                    color: vec4(1.0, 1.0, 1.0, 1.0),
                },
                RenderVertex {
                    position: vec4(1.0, -1.0, 0.0, 1.0),
                    tex_coord: vec2(1.0, 0.0),
                    color: vec4(1.0, 1.0, 1.0, 1.0),
                },
                RenderVertex {
                    position: vec4(0.0, 1.0, 0.0, 1.0),
                    tex_coord: vec2(0.5, 1.0),
                    color: vec4(1.0, 1.0, 1.0, 1.0),
                },
            ]),
        }
    }

    fn view_state() -> RenderViewState {
        RenderViewState {
            viewport: Rect {
                x: 0.0,
                y: 0.0,
                width: 64.0,
                height: 64.0,
            },
            clear: Some(ViewClear {
                depth: 1.0,
                color: Some(vec4(0.0, 0.0, 0.0, 1.0)),
                stencil: true,
            }),
            clip_plane: None,
        }
    }

    #[test]
    fn construction_sets_initial_state() {
        let renderer = renderer();
        assert_eq!(renderer.width(), 64);
        assert_eq!(renderer.height(), 64);
        assert_eq!(renderer.stencil_bits(), 8);
        assert_eq!(renderer.driver().version, "2.1 Fake");
        let gl = renderer.gl.borrow();
        gl.assert_contains("front face", |call| matches!(call, GlCall::FrontFace { mode: CCW }));
        gl.assert_contains("shade model", |call| {
            matches!(call, GlCall::ShadeModel { model: SMOOTH })
        });
        gl.assert_contains("polygon mode", |call| matches!(call, GlCall::PolygonMode { .. }));
        gl.assert_contains("sampler setup", |call| matches!(call, GlCall::Uniform1i { .. }));
    }

    #[test]
    fn image_lifecycle_and_texture_mode() {
        let owner = owner();
        let mut renderer = GlRenderer::new(FakeGlContext::new(), owner.clone(), 64, 64);
        let image = handle(&owner, 5, 2, 2);
        renderer.apply_image_resource(&ImageResourceOperation::CreateImage {
            image: image.clone(),
            content: RenderImage::Rgba8 {
                levels: vec![
                    ImageLevel {
                        width: 2,
                        height: 2,
                        pixels: vec![1; 16],
                    },
                    ImageLevel {
                        width: 1,
                        height: 1,
                        pixels: vec![2; 4],
                    },
                ],
                border_color: vec4(0.0, 0.0, 0.0, 0.0),
            },
            sampling: TextureSampling {
                repeat: false,
                filter: TextureFilter::LinearMipmapLinear,
            },
        });
        renderer.apply_image_resource(&ImageResourceOperation::UpdateImage {
            image: image.clone(),
            level: 0,
            content: LevelContent::Rgba(ImageLevel {
                width: 2,
                height: 2,
                pixels: vec![3; 16],
            }),
        });
        renderer.apply_image_resource(&ImageResourceOperation::TextureMode {
            filter: TextureFilter::Nearest,
        });
        renderer.apply_image_resource(&ImageResourceOperation::ReleaseImage { image });
        let gl = renderer.gl.borrow();
        assert_eq!(gl.count_matching(|call| matches!(call, GlCall::TexImage2D { .. })), 2);
        gl.assert_contains("sub upload", |call| matches!(call, GlCall::TexSubImage2D { .. }));
        gl.assert_contains("filter switch", |call| {
            matches!(call, GlCall::TexParameteri { value: NEAREST, .. })
        });
        gl.assert_contains("release", |call| matches!(call, GlCall::DeleteTextures { .. }));
    }

    #[test]
    fn view_with_single_pair_and_line_batches() {
        let owner = owner();
        let mut renderer = GlRenderer::new(FakeGlContext::new(), owner.clone(), 64, 64);
        let first = register_rgba(&mut renderer, &owner, 1);
        let second = register_rgba(&mut renderer, &owner, 2);
        renderer.select_draw_buffer(DrawBuffer::Back, false);
        renderer.begin_view(&view_state());
        let mut pair = triangle_batch(&first);
        pair.vertices = BatchVertices::Pair {
            vertices: vec![
                MultitextureVertex {
                    base: RenderVertex {
                        position: vec4(0.0, 0.0, 0.0, 1.0),
                        tex_coord: vec2(0.0, 0.0),
                        color: vec4(1.0, 1.0, 1.0, 1.0),
                    },
                    tex_coord2: vec2(0.0, 0.0),
                },
                MultitextureVertex {
                    base: RenderVertex {
                        position: vec4(1.0, 0.0, 0.0, 1.0),
                        tex_coord: vec2(1.0, 0.0),
                        color: vec4(1.0, 1.0, 1.0, 1.0),
                    },
                    tex_coord2: vec2(1.0, 0.0),
                },
                MultitextureVertex {
                    base: RenderVertex {
                        position: vec4(0.0, 1.0, 0.0, 1.0),
                        tex_coord: vec2(0.0, 1.0),
                        color: vec4(1.0, 1.0, 1.0, 1.0),
                    },
                    tex_coord2: vec2(0.0, 1.0),
                },
            ],
            second_texture: TextureBundle {
                binding: TextureBinding::BindImage(second),
                environment: PairEnvironment::Modulate,
            },
        };
        pair.fog = Some(BatchFog::Constant {
            color: vec3(0.2, 0.2, 0.2),
            amount: 0.5,
        });
        let mut lines = triangle_batch(&first);
        lines.vertices = BatchVertices::Single(vec![
            RenderVertex {
                position: vec4(0.0, 0.0, 0.0, 1.0),
                tex_coord: vec2(0.0, 0.0),
                color: vec4(1.0, 0.0, 0.0, 1.0),
            },
            RenderVertex {
                position: vec4(1.0, 1.0, 0.0, 1.0),
                tex_coord: vec2(1.0, 1.0),
                color: vec4(1.0, 0.0, 0.0, 1.0),
            },
        ]);
        lines.indices = vec![0, 1];
        lines.primitive = BatchPrimitive::Lines { line_width: 2.0 };
        renderer.draw_immediate(&RenderOperation::Draw(vec![triangle_batch(&first), pair, lines]));
        renderer.draw_immediate(&RenderOperation::DepthRange([0.0, 1.0]));
        renderer.draw_immediate(&RenderOperation::Cull(CullFace::None));
        renderer.draw_immediate(&RenderOperation::PolygonOffset(None));
        renderer.draw_immediate(&RenderOperation::DisablePortalClip);
        let gl = renderer.gl.borrow();
        gl.assert_contains("viewport", |call| {
            matches!(
                call,
                GlCall::Viewport {
                    width: 64,
                    height: 64,
                    ..
                }
            )
        });
        gl.assert_contains("view clear", |call| matches!(call, GlCall::Clear { .. }));
        assert_eq!(
            gl.count_matching(|call| matches!(call, GlCall::DrawElements { mode: TRIANGLES, .. })),
            2
        );
        gl.assert_contains("lines", |call| {
            matches!(call, GlCall::DrawElements { mode: LINES, count: 2 })
        });
        gl.assert_contains(
            "line width",
            |call| matches!(call, GlCall::LineWidth { width } if *width == 2.0),
        );
        gl.assert_contains(
            "second unit",
            |call| matches!(call, GlCall::ActiveTexture { unit } if *unit == TEXTURE0 + 1),
        );
    }

    #[test]
    fn object_opacity_skips_zero_directs_one_and_blends_partial() {
        let owner = owner();
        let mut renderer = GlRenderer::new(FakeGlContext::new(), owner.clone(), 64, 64);
        let image = register_rgba(&mut renderer, &owner, 1);
        renderer.begin_view(&view_state());
        renderer.gl.borrow_mut().clear_log();
        renderer.draw_immediate(&RenderOperation::ObjectOpacity {
            opacity: 0.0,
            batches: vec![triangle_batch(&image)],
        });
        assert!(renderer.gl.borrow().log.is_empty());
        renderer.draw_immediate(&RenderOperation::ObjectOpacity {
            opacity: 1.0,
            batches: vec![triangle_batch(&image)],
        });
        {
            let gl = renderer.gl.borrow();
            gl.assert_contains("direct draw", |call| matches!(call, GlCall::DrawElements { .. }));
            gl.assert_absent("no backdrop copy", |call| {
                matches!(call, GlCall::BlitFramebuffer { .. })
            });
        }
        renderer.gl.borrow_mut().clear_log();
        renderer.draw_immediate(&RenderOperation::ObjectOpacity {
            opacity: 0.5,
            batches: vec![triangle_batch(&image)],
        });
        {
            let gl = renderer.gl.borrow();
            assert_eq!(
                gl.count_matching(|call| matches!(call, GlCall::BlitFramebuffer { .. })),
                2
            );
            gl.assert_contains("child draw", |call| matches!(call, GlCall::DrawElements { .. }));
            gl.assert_contains("composite", |call| matches!(call, GlCall::Begin { mode: QUADS }));
        }
    }

    #[test]
    fn depth_atlas_fog_shadow_and_sky_operations() {
        let owner = owner();
        let mut renderer = GlRenderer::new(FakeGlContext::new(), owner.clone(), 64, 64);
        let white = register_rgba(&mut renderer, &owner, 1);
        let atlas = handle(&owner, 2, 32, 32);
        renderer.apply_image_resource(&ImageResourceOperation::CreateImage {
            image: atlas.clone(),
            content: RenderImage::Depth32f {
                levels: vec![DepthImageLevel {
                    width: 32,
                    height: 32,
                    pixels: vec![1.0; 1024],
                }],
            },
            sampling: TextureSampling {
                repeat: false,
                filter: TextureFilter::Nearest,
            },
        });
        renderer.begin_view(&view_state());
        renderer.draw_immediate(&RenderOperation::DepthAtlas {
            image: atlas,
            passes: vec![DepthAtlasPass {
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
            }],
        });
        let mut projection = [0.0f32; 16];
        projection[0] = 1.0;
        projection[5] = 1.0;
        projection[10] = -1.5;
        projection[11] = -1.0;
        projection[14] = -2.0;
        renderer.draw_immediate(&RenderOperation::Q2Fog(Q2FogOperation {
            camera: RenderCamera {
                origin: vec3(0.0, 0.0, 0.0),
                axis: [vec3(1.0, 0.0, 0.0), vec3(0.0, 1.0, 0.0), vec3(0.0, 0.0, 1.0)],
                projection,
                viewport: Rect {
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
        }));
        renderer.draw_immediate(&RenderOperation::ShadowVolume {
            positions: vec![
                vec4(0.0, 0.0, 0.0, 1.0),
                vec4(1.0, 0.0, 0.0, 1.0),
                vec4(0.0, 1.0, 0.0, 1.0),
            ],
            indices: vec![0, 1, 2],
            mirror: false,
            white_image: white.clone(),
        });
        renderer.draw_immediate(&RenderOperation::ShadowFinish {
            positions: [
                vec4(-1.0, -1.0, 0.0, 1.0),
                vec4(1.0, -1.0, 0.0, 1.0),
                vec4(1.0, 1.0, 0.0, 1.0),
                vec4(-1.0, 1.0, 0.0, 1.0),
            ],
            white_image: white.clone(),
        });
        renderer.draw_immediate(&RenderOperation::SkySide {
            image: white,
            color: vec4(1.0, 1.0, 1.0, 1.0),
            strips: vec![vec![
                SkyVertex {
                    position: vec4(0.0, 0.0, 0.0, 1.0),
                    tex_coord: vec2(0.0, 0.0),
                },
                SkyVertex {
                    position: vec4(1.0, 0.0, 0.0, 1.0),
                    tex_coord: vec2(1.0, 0.0),
                },
            ]],
        });
        let gl = renderer.gl.borrow();
        gl.assert_contains("atlas attach", |call| {
            matches!(call, GlCall::FramebufferTexture2D { .. })
        });
        gl.assert_contains("fog snapshot", |call| matches!(call, GlCall::CopyTexImage2D { .. }));
        gl.assert_contains("stencil volume", |call| matches!(call, GlCall::StencilOp { .. }));
        gl.assert_contains("shadow finish blend", |call| matches!(call, GlCall::BlendFunc { .. }));
        gl.assert_contains("sky strip", |call| {
            matches!(call, GlCall::Begin { mode: TRIANGLE_STRIP })
        });
    }

    #[test]
    fn show_image_overdraw_depth_and_gamma_finish_close() {
        let owner = owner();
        let mut renderer = GlRenderer::new(FakeGlContext::new(), owner.clone(), 8, 8);
        let image = register_rgba(&mut renderer, &owner, 1);
        renderer.begin_view(&view_state());
        renderer.draw_show_image(
            &image,
            &Rect {
                x: 0.0,
                y: 0.0,
                width: 8.0,
                height: 8.0,
            },
            false,
        );
        renderer.set_overdraw_measurement(true);
        renderer.set_overdraw_measurement(false);
        renderer.gl.borrow_mut().set_read_floats(vec![0.25]);
        assert_eq!(renderer.read_depth_pixel(1, 2), 0.25);
        renderer.gl.borrow_mut().set_read_bytes(vec![7; 64]);
        let mut stencil = vec![0u8; 64];
        renderer.read_stencil_overdraw(&mut stencil);
        assert!(stencil.iter().all(|value| *value == 7));
        renderer.set_output_gamma(2.0);
        renderer.clear_color_buffer();
        renderer.finish();
        renderer.present();
        assert_eq!(renderer.get_error(), 0);
        let pixels = renderer.read_pixels();
        assert_eq!(pixels.len(), 8 * 8 * 4);
        renderer.close();
        renderer.close();
        let gl = renderer.gl.borrow();
        gl.assert_contains("show quad", |call| matches!(call, GlCall::Ortho { .. }));
        gl.assert_contains("overdraw setup", |call| matches!(call, GlCall::StencilFunc { .. }));
        gl.assert_contains("gamma quad", |call| matches!(call, GlCall::Begin { mode: QUADS }));
        gl.assert_contains("finish", |call| matches!(call, GlCall::Finish));
        gl.assert_contains("swap", |call| matches!(call, GlCall::SwapBuffers));
        gl.assert_contains("program teardown", |call| matches!(call, GlCall::DeleteProgram { .. }));
    }

    struct StaticSource {
        image: RendererImage,
    }

    impl crate::render::types::DynamicImageSource for StaticSource {
        fn resolve(&self, _apply: &mut dyn FnMut(ImageResourceOperation)) -> RendererImage {
            self.image.clone()
        }
    }

    #[test]
    fn dynamic_textures_resolve_once_per_draw() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        struct CountingSource {
            image: RendererImage,
            calls: AtomicUsize,
        }
        impl crate::render::types::DynamicImageSource for CountingSource {
            fn resolve(&self, _apply: &mut dyn FnMut(ImageResourceOperation)) -> RendererImage {
                self.calls.fetch_add(1, Ordering::SeqCst);
                self.image.clone()
            }
        }
        let owner = owner();
        let mut renderer = GlRenderer::new(FakeGlContext::new(), owner.clone(), 64, 64);
        let image = register_rgba(&mut renderer, &owner, 1);
        let source = Arc::new(CountingSource {
            image: image.clone(),
            calls: AtomicUsize::new(0),
        });
        let mut batch = triangle_batch(&image);
        batch.vertices = BatchVertices::Pair {
            vertices: vec![
                MultitextureVertex {
                    base: RenderVertex {
                        position: vec4(0.0, 0.0, 0.0, 1.0),
                        tex_coord: vec2(0.0, 0.0),
                        color: vec4(1.0, 1.0, 1.0, 1.0),
                    },
                    tex_coord2: vec2(0.0, 0.0),
                },
                MultitextureVertex {
                    base: RenderVertex {
                        position: vec4(1.0, 0.0, 0.0, 1.0),
                        tex_coord: vec2(1.0, 0.0),
                        color: vec4(1.0, 1.0, 1.0, 1.0),
                    },
                    tex_coord2: vec2(1.0, 0.0),
                },
                MultitextureVertex {
                    base: RenderVertex {
                        position: vec4(0.0, 1.0, 0.0, 1.0),
                        tex_coord: vec2(0.0, 1.0),
                        color: vec4(1.0, 1.0, 1.0, 1.0),
                    },
                    tex_coord2: vec2(0.0, 1.0),
                },
            ],
            second_texture: TextureBundle {
                binding: TextureBinding::DynamicImage(source.clone()),
                environment: PairEnvironment::Add,
            },
        };
        batch.texture = TextureBinding::DynamicImage(source.clone());
        renderer.draw_batch(&batch);
        assert_eq!(source.calls.load(Ordering::SeqCst), 1);
        let uses_fog = BatchFog::Exp2 {
            color: vec3(0.1, 0.1, 0.1),
            density: 0.01,
            effect: FogEffect::Rgb,
        };
        let _ = uses_fog;
        let plain = StaticSource { image };
        let resolved = plain.resolve(&mut |_| {});
        assert_eq!(resolved.ordinal, 1);
    }
}
