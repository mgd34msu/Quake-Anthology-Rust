//! Cached GL 4.4 command consumer. SDL context ownership stays in platform.
//!
//! Projection and stage order follow qsrc Q3 tr_main.c and tr_backend.c.
use crate::BackendStats;
use crate::assets::{AlphaTest, Assets, Blend, DepthFunc, MaterialId, Stage, TcGen, Vertex};
use crate::scene::{BlendPhase, Command, CommandList, Draw2d, Refdef, SceneEntity, Viewport};
use qa_core::primitives::Vec3;
use std::ffi::{CStr, c_char, c_void};
use std::marker::PhantomData;
use std::mem::{offset_of, size_of};
use std::ptr::{self, NonNull};

type Sync = *mut c_void;

macro_rules! procedures {
    ($( $name:ident : $symbol:literal ( $( $arg:ty ),* ) $( -> $result:ty )?; )*) => {
        struct Gl {
            $( $name: unsafe extern "system" fn($( $arg ),*) $( -> $result )?, )*
        }
        impl Gl {
            unsafe fn load(mut resolve: impl FnMut(&CStr) -> *const c_void) -> Result<Self, String> {
                Ok(Self {
                    $( $name: {
                        let symbol = unsafe {
                            CStr::from_bytes_with_nul_unchecked(concat!($symbol, "\0").as_bytes())
                        };
                        let address = resolve(symbol);
                        if address.is_null() {
                            return Err(format!("GL 4.4 procedure {} is unavailable", $symbol));
                        }
                        unsafe { std::mem::transmute::<*const c_void,
                            unsafe extern "system" fn($( $arg ),*) $( -> $result )?>(address) }
                    }, )*
                })
            }
        }
    };
}

procedures! {
    get_integer: "glGetIntegerv"(u32, *mut i32);
    get_string: "glGetString"(u32) -> *const u8;
    get_error: "glGetError"() -> u32;
    enable: "glEnable"(u32);
    disable: "glDisable"(u32);
    viewport: "glViewport"(i32, i32, i32, i32);
    scissor: "glScissor"(i32, i32, i32, i32);
    clear_color: "glClearColor"(f32, f32, f32, f32);
    color_mask: "glColorMask"(u8, u8, u8, u8);
    clear: "glClear"(u32);
    depth_mask: "glDepthMask"(u8);
    depth_func: "glDepthFunc"(u32);
    depth_range: "glDepthRange"(f64, f64);
    blend_func: "glBlendFunc"(u32, u32);
    cull_face: "glCullFace"(u32);
    front_face: "glFrontFace"(u32);
    gen_vertex_arrays: "glGenVertexArrays"(i32, *mut u32);
    bind_vertex_array: "glBindVertexArray"(u32);
    delete_vertex_arrays: "glDeleteVertexArrays"(i32, *const u32);
    gen_buffers: "glGenBuffers"(i32, *mut u32);
    bind_buffer: "glBindBuffer"(u32, u32);
    buffer_data: "glBufferData"(u32, isize, *const c_void, u32);
    buffer_storage: "glBufferStorage"(u32, isize, *const c_void, u32);
    map_buffer_range: "glMapBufferRange"(u32, isize, isize, u32) -> *mut c_void;
    unmap_buffer: "glUnmapBuffer"(u32) -> u8;
    delete_buffers: "glDeleteBuffers"(i32, *const u32);
    enable_vertex_attribute: "glEnableVertexAttribArray"(u32);
    vertex_attribute: "glVertexAttribPointer"(u32, i32, u32, u8, i32, *const c_void);
    gen_textures: "glGenTextures"(i32, *mut u32);
    bind_texture: "glBindTexture"(u32, u32);
    texture_image: "glTexImage2D"(u32, i32, i32, i32, i32, i32, u32, u32, *const c_void);
    texture_parameter: "glTexParameteri"(u32, u32, i32);
    generate_mipmap: "glGenerateMipmap"(u32);
    active_texture: "glActiveTexture"(u32);
    delete_textures: "glDeleteTextures"(i32, *const u32);
    create_shader: "glCreateShader"(u32) -> u32;
    shader_source: "glShaderSource"(u32, i32, *const *const c_char, *const i32);
    compile_shader: "glCompileShader"(u32);
    get_shader: "glGetShaderiv"(u32, u32, *mut i32);
    shader_log: "glGetShaderInfoLog"(u32, i32, *mut i32, *mut c_char);
    delete_shader: "glDeleteShader"(u32);
    create_program: "glCreateProgram"() -> u32;
    attach_shader: "glAttachShader"(u32, u32);
    link_program: "glLinkProgram"(u32);
    get_program: "glGetProgramiv"(u32, u32, *mut i32);
    program_log: "glGetProgramInfoLog"(u32, i32, *mut i32, *mut c_char);
    use_program: "glUseProgram"(u32);
    delete_program: "glDeleteProgram"(u32);
    uniform_location: "glGetUniformLocation"(u32, *const c_char) -> i32;
    uniform_matrix: "glUniformMatrix4fv"(i32, i32, u8, *const f32);
    uniform_color: "glUniform4fv"(i32, i32, *const f32);
    uniform_integer: "glUniform1i"(i32, i32);
    draw_elements: "glDrawElementsBaseVertex"(u32, i32, u32, *const c_void, i32);
    draw_arrays: "glDrawArrays"(u32, i32, i32);
    fence: "glFenceSync"(u32, u32) -> Sync;
    wait_fence: "glClientWaitSync"(Sync, u32, u64) -> u32;
    delete_fence: "glDeleteSync"(Sync);
    read_pixels: "glReadPixels"(i32, i32, i32, i32, u32, u32, *mut c_void);
    read_buffer: "glReadBuffer"(u32);
    pixel_store: "glPixelStorei"(u32, i32);
}

const ARRAY_BUFFER: u32 = 0x8892;
const ELEMENT_ARRAY_BUFFER: u32 = 0x8893;
const FLOAT: u32 = 0x1406;
const UNSIGNED_BYTE: u32 = 0x1401;
const UNSIGNED_INT: u32 = 0x1405;
const TEXTURE_2D: u32 = 0x0de1;
const DEPTH_TEST: u32 = 0x0b71;
const BLEND: u32 = 0x0be2;
const CULL_FACE: u32 = 0x0b44;
const SCISSOR_TEST: u32 = 0x0c11;
const COLOR_BUFFER_BIT: u32 = 0x4000;
const DEPTH_BUFFER_BIT: u32 = 0x0100;
const TRIANGLES: u32 = 0x0004;
const TRIANGLE_FAN: u32 = 0x0006;
const RGBA: u32 = 0x1908;
const MAP_FLAGS: u32 = 0x0002 | 0x0040 | 0x0080;
const RING_SLOTS: usize = 3;

const VERTEX_SHADER: &str = r#"#version 440 core
layout(location=0) in vec3 a_position;
layout(location=1) in vec2 a_texcoord;
layout(location=2) in vec2 a_lightmap;
layout(location=3) in vec4 a_color;
uniform mat4 u_mvp;
uniform vec4 u_color;
uniform int u_texgen;
uniform int u_vertex_color;
out vec2 texcoord;
out vec4 color;
void main() {
    gl_Position = u_mvp * vec4(a_position, 1.0);
    texcoord = u_texgen == 0 ? a_texcoord : a_lightmap;
    color = u_color * (u_vertex_color == 0 ? vec4(1.0) : a_color);
}
"#;
const FRAGMENT_SHADER: &str = r#"#version 440 core
uniform sampler2D u_image;
uniform int u_alpha_test;
in vec2 texcoord;
in vec4 color;
layout(location=0) out vec4 fragment;
void main() {
    vec4 value = texture(u_image, texcoord) * color;
    if (u_alpha_test == 1 && value.a <= 0.0) discard;
    if (u_alpha_test == 2 && value.a < 0.5) discard;
    fragment = value;
}
"#;

#[derive(Clone, Copy, Default)]
struct Mesh {
    first_index: usize,
    count: i32,
    base_vertex: i32,
}
struct Uniforms {
    mvp: i32,
    color: i32,
    texgen: i32,
    vertex_color: i32,
    alpha_test: i32,
}
#[derive(Default)]
struct State {
    vao: u32,
    texture: u32,
    blend: Option<Blend>,
    depth: Option<bool>,
    depth_write: Option<bool>,
    depth_func: Option<DepthFunc>,
    cull: Option<bool>,
    depth_hack: bool,
}
struct Dynamic {
    buffer: u32,
    vao: u32,
    mapped: Option<NonNull<Vertex>>,
    fences: [Sync; RING_SLOTS],
    blocked: [bool; RING_SLOTS],
    capacity: usize,
    next: usize,
    slot: usize,
    used: usize,
    acquired: bool,
}
impl Default for Dynamic {
    fn default() -> Self {
        Self {
            buffer: 0,
            vao: 0,
            mapped: None,
            fences: [ptr::null_mut(); RING_SLOTS],
            blocked: [false; RING_SLOTS],
            capacity: 0,
            next: 0,
            slot: 0,
            used: 0,
            acquired: false,
        }
    }
}

/// Owns GPU resources on the thread with the current SDL GL context.
pub struct GlBackend {
    gl: Gl,
    renderer: String,
    version: String,
    width: u32,
    height: u32,
    program: u32,
    uniforms: Uniforms,
    static_vao: u32,
    static_buffers: [u32; 2],
    meshes: Box<[Mesh]>,
    textures: Box<[u32]>,
    dynamic: Dynamic,
    state: State,
    _context_thread: PhantomData<*mut ()>,
}

impl GlBackend {
    /// # Safety
    /// `resolve` must return this current context's GL entry points. Keep that
    /// context current on this thread until this backend has been dropped.
    /// Assets must remain frozen after load; later registrations need a reload.
    pub unsafe fn load(
        resolve: impl FnMut(&CStr) -> *const c_void,
        assets: &Assets,
        width: u32,
        height: u32,
        dynamic_vertices: usize,
    ) -> Result<Self, String> {
        if width == 0 || height == 0 || width > i32::MAX as u32 || height > i32::MAX as u32 {
            return Err("invalid GL drawable size".into());
        }
        let dynamic_count = dynamic_vertices
            .checked_mul(RING_SLOTS)
            .filter(|&n| n != 0 && n <= i32::MAX as usize)
            .ok_or("invalid GL dynamic vertex capacity")?;
        let dynamic_bytes = dynamic_count
            .checked_mul(size_of::<Vertex>())
            .filter(|&n| n <= isize::MAX as usize)
            .ok_or("GL dynamic buffer size overflow")?;
        let gl = unsafe { Gl::load(resolve)? };
        let (mut major, mut minor) = (0, 0);
        unsafe {
            (gl.get_integer)(0x821b, &mut major);
            (gl.get_integer)(0x821c, &mut minor);
        }
        if major < 4 || (major == 4 && minor < 4) {
            return Err(format!(
                "GL 4.4 is required; current context is {major}.{minor}"
            ));
        }
        let renderer = unsafe { context_string(&gl, 0x1f01)? };
        let version = unsafe { context_string(&gl, 0x1f02)? };
        let program = unsafe { compile_program(&gl)? };
        let uniforms = unsafe {
            Uniforms {
                mvp: (gl.uniform_location)(program, c"u_mvp".as_ptr()),
                color: (gl.uniform_location)(program, c"u_color".as_ptr()),
                texgen: (gl.uniform_location)(program, c"u_texgen".as_ptr()),
                vertex_color: (gl.uniform_location)(program, c"u_vertex_color".as_ptr()),
                alpha_test: (gl.uniform_location)(program, c"u_alpha_test".as_ptr()),
            }
        };
        let mut backend = Self {
            gl,
            renderer,
            version,
            width,
            height,
            program,
            uniforms,
            static_vao: 0,
            static_buffers: [0; 2],
            meshes: Box::new([]),
            textures: Box::new([]),
            dynamic: Dynamic::default(),
            state: State::default(),
            _context_thread: PhantomData,
        };
        unsafe {
            backend.load_static(assets)?;
            let gl = &backend.gl;
            (gl.gen_buffers)(1, &mut backend.dynamic.buffer);
            (gl.bind_buffer)(ARRAY_BUFFER, backend.dynamic.buffer);
            (gl.buffer_storage)(ARRAY_BUFFER, dynamic_bytes as isize, ptr::null(), MAP_FLAGS);
            let mapped = (gl.map_buffer_range)(ARRAY_BUFFER, 0, dynamic_bytes as isize, MAP_FLAGS);
            backend.dynamic.mapped = NonNull::new(mapped.cast());
            if backend.dynamic.mapped.is_none() {
                return Err("GL persistent dynamic buffer mapping failed".into());
            }
            backend.dynamic.capacity = dynamic_vertices;
            (gl.gen_vertex_arrays)(1, &mut backend.dynamic.vao);
            (gl.bind_vertex_array)(backend.dynamic.vao);
            vertex_layout(gl);
            (gl.use_program)(program);
            (gl.uniform_integer)((gl.uniform_location)(program, c"u_image".as_ptr()), 0);
            (gl.active_texture)(0x84c0);
            (gl.depth_func)(0x0203);
            (gl.front_face)(0x0901);
            // Native Q3 CT_FRONT_SIDED retains clockwise projected triangles.
            (gl.cull_face)(0x0404);
            (gl.disable)(0x809d); // No multisample change to the native image.
            (gl.disable)(0x0bd0); // Explicit byte-color output; no default dithering.
            (gl.disable)(0x8db9); // Native gamma is a presentation operation.
            (gl.color_mask)(1, 1, 1, 1);
            (gl.bind_vertex_array)(0);
            let error = (gl.get_error)();
            if error != 0 {
                return Err(format!("GL asset upload failed with error 0x{error:04x}"));
            }
        }
        Ok(backend)
    }

    /// Renderer identity and GL version, captured once at context load.
    pub fn renderer_info(&self) -> (&str, &str) {
        (&self.renderer, &self.version)
    }

    /// Optional verification probe. This is not called by the frame path.
    pub fn take_error(&self) -> Option<u32> {
        let error = unsafe { (self.gl.get_error)() };
        (error != 0).then_some(error)
    }

    unsafe fn load_static(&mut self, assets: &Assets) -> Result<(), String> {
        let mut vertices = Vec::new();
        let mut indices = Vec::new();
        let mut meshes = Vec::with_capacity(assets.models().len());
        for model in assets.models() {
            let base_vertex = i32::try_from(vertices.len()).map_err(|_| "GL vertex table full")?;
            let count =
                i32::try_from(model.indices.len()).map_err(|_| "GL mesh index table full")?;
            let first_index = indices.len();
            vertices.extend_from_slice(&model.vertices);
            indices.extend_from_slice(&model.indices);
            meshes.push(Mesh {
                first_index,
                count,
                base_vertex,
            });
        }
        let vertex_bytes = vertices
            .len()
            .checked_mul(size_of::<Vertex>())
            .filter(|&n| n <= isize::MAX as usize)
            .ok_or("GL static vertex size overflow")?;
        let index_bytes = indices
            .len()
            .checked_mul(size_of::<u32>())
            .filter(|&n| n <= isize::MAX as usize)
            .ok_or("GL static index size overflow")?;
        self.meshes = meshes.into_boxed_slice();
        self.textures = vec![0; assets.images().len()].into_boxed_slice();
        let texture_count =
            i32::try_from(self.textures.len()).map_err(|_| "GL texture table full")?;
        let gl = &self.gl;
        unsafe {
            (gl.gen_vertex_arrays)(1, &mut self.static_vao);
            (gl.gen_buffers)(2, self.static_buffers.as_mut_ptr());
            (gl.bind_vertex_array)(self.static_vao);
            (gl.bind_buffer)(ARRAY_BUFFER, self.static_buffers[0]);
            (gl.buffer_data)(
                ARRAY_BUFFER,
                vertex_bytes as isize,
                vertices.as_ptr().cast(),
                0x88e4,
            );
            vertex_layout(gl);
            (gl.bind_buffer)(ELEMENT_ARRAY_BUFFER, self.static_buffers[1]);
            (gl.buffer_data)(
                ELEMENT_ARRAY_BUFFER,
                index_bytes as isize,
                indices.as_ptr().cast(),
                0x88e4,
            );
            (gl.gen_textures)(texture_count, self.textures.as_mut_ptr());
            (gl.pixel_store)(0x0cf5, 1);
            for (image, &texture) in assets.images().iter().zip(self.textures.iter()) {
                (gl.bind_texture)(TEXTURE_2D, texture);
                (gl.texture_image)(
                    TEXTURE_2D,
                    0,
                    0x8058,
                    image.width as i32,
                    image.height as i32,
                    0,
                    RGBA,
                    UNSIGNED_BYTE,
                    image.rgba.as_ptr().cast(),
                );
                (gl.texture_parameter)(TEXTURE_2D, 0x2801, 0x2701); // linear, nearest mip
                (gl.texture_parameter)(TEXTURE_2D, 0x2800, 0x2601);
                (gl.texture_parameter)(TEXTURE_2D, 0x2802, 0x2901);
                (gl.texture_parameter)(TEXTURE_2D, 0x2803, 0x2901);
                (gl.generate_mipmap)(TEXTURE_2D);
            }
            (gl.bind_texture)(TEXTURE_2D, 0);
        }
        Ok(())
    }

    /// Consumes the same sealed packet as the software backend. This does not
    /// swap the SDL window; platform presents after the backend has finished.
    pub fn render(&mut self, list: &CommandList, assets: &Assets) -> BackendStats {
        let mut stats = BackendStats {
            rejected: list.rejected.min(u32::MAX as u64) as u32,
            ..BackendStats::default()
        };
        unsafe { (self.gl.use_program)(self.program) };
        if !self.begin_dynamic() {
            stats.rejected = stats.rejected.saturating_add(1);
        }
        for command in list.commands() {
            match *command {
                Command::Empty => {}
                Command::Clear(color) => self.clear(color),
                Command::View(view) => {
                    if !valid_refdef(view.refdef, self.width, self.height) {
                        stats.rejected = stats.rejected.saturating_add(1);
                        continue;
                    }
                    stats.views = stats.views.saturating_add(1);
                    stats.pending_lights =
                        stats.pending_lights.saturating_add(view.scene.lights.count);
                    self.viewport(view.refdef.viewport, true);
                    self.depth_state(true, true);
                    unsafe { (self.gl.clear)(DEPTH_BUFFER_BIT) };
                    let projection = view_projection(view.refdef);
                    for entity in list.entities(view.scene.entities) {
                        self.entity(entity, assets, &projection, &mut stats);
                    }
                    for poly in list.polys(view.scene.polys) {
                        let vertices = list.vertices(poly.vertices);
                        let Some(first) = self.upload(vertices) else {
                            stats.rejected = stats.rejected.saturating_add(1);
                            continue;
                        };
                        if !self.draw_dynamic(
                            assets,
                            poly.material,
                            first,
                            vertices.len(),
                            &projection,
                            [1.0; 4],
                            true,
                            false,
                        ) {
                            stats.rejected = stats.rejected.saturating_add(1);
                            continue;
                        }
                        stats.triangles = stats
                            .triangles
                            .saturating_add(vertices.len().saturating_sub(2) as u32);
                    }
                    if view.refdef.blend_phase == BlendPhase::AfterView {
                        self.tint(view.refdef.viewport, view.refdef.blend, &mut stats);
                    }
                }
                Command::Draw2d(draw) => {
                    self.viewport(
                        Viewport {
                            x: 0,
                            y: 0,
                            width: self.width,
                            height: self.height,
                        },
                        false,
                    );
                    if !self.draw_2d(draw, assets) {
                        stats.rejected = stats.rejected.saturating_add(1);
                    } else {
                        stats.draws_2d = stats.draws_2d.saturating_add(1);
                    }
                }
            }
        }
        // FinalPalette is an RGBA tint until THE-862 supplies indexed palette
        // presentation. Its placement, once after the HUD per view, is retained.
        for command in list.commands() {
            if let Command::View(view) = command {
                if view.refdef.blend_phase == BlendPhase::FinalPalette
                    && valid_refdef(view.refdef, self.width, self.height)
                {
                    // Palette shifts change RGB without changing coverage.
                    unsafe {
                        (self.gl.color_mask)(1, 1, 1, 0);
                    }
                    self.tint(
                        view.refdef.blend_viewport.unwrap_or(view.refdef.viewport),
                        view.refdef.blend,
                        &mut stats,
                    );
                    unsafe {
                        (self.gl.color_mask)(1, 1, 1, 1);
                    }
                }
            }
        }
        self.end_dynamic();
        stats
    }

    fn begin_dynamic(&mut self) -> bool {
        let dynamic = &mut self.dynamic;
        dynamic.slot = dynamic.next;
        dynamic.next = (dynamic.next + 1) % RING_SLOTS;
        dynamic.used = 0;
        dynamic.acquired = !dynamic.blocked[dynamic.slot];
        let fence = dynamic.fences[dynamic.slot];
        if !fence.is_null() {
            // Wait for the bytes this slot owns instead of discarding correct
            // draws merely because the GPU trails the CPU by three frames.
            // This driver wait remains part of measured backend time.
            let result = unsafe { (self.gl.wait_fence)(fence, 1, 1_000_000_000) };
            if result == 0x911a || result == 0x911c {
                unsafe { (self.gl.delete_fence)(fence) };
                dynamic.fences[dynamic.slot] = ptr::null_mut();
            } else {
                dynamic.acquired = false;
                if result == 0x911d {
                    dynamic.blocked[dynamic.slot] = true;
                }
            }
        }
        dynamic.acquired
    }

    fn end_dynamic(&mut self) {
        if self.dynamic.acquired && self.dynamic.used != 0 {
            let fence = unsafe { (self.gl.fence)(0x9117, 0) };
            self.dynamic.fences[self.dynamic.slot] = fence;
            if fence.is_null() {
                // A failed fence cannot establish safe reuse of mapped bytes.
                self.dynamic.blocked[self.dynamic.slot] = true;
            }
        }
    }

    fn upload(&mut self, vertices: &[Vertex]) -> Option<usize> {
        let dynamic = &mut self.dynamic;
        if !dynamic.acquired || vertices.len() > dynamic.capacity - dynamic.used {
            return None;
        }
        let mapped = dynamic.mapped?;
        let first = dynamic.slot * dynamic.capacity + dynamic.used;
        unsafe {
            ptr::copy_nonoverlapping(
                vertices.as_ptr(),
                mapped.as_ptr().add(first),
                vertices.len(),
            )
        };
        dynamic.used += vertices.len();
        Some(first)
    }

    fn clear(&mut self, color: [u8; 4]) {
        self.viewport(
            Viewport {
                x: 0,
                y: 0,
                width: self.width,
                height: self.height,
            },
            false,
        );
        self.depth_state(false, true);
        self.set_depth_hack(false);
        let color = rgba(color);
        unsafe {
            (self.gl.clear_color)(color[0], color[1], color[2], color[3]);
            (self.gl.clear)(COLOR_BUFFER_BIT | DEPTH_BUFFER_BIT);
        }
    }

    fn viewport(&self, viewport: Viewport, scissor: bool) {
        let x = viewport.x as i32;
        let y = (self.height - viewport.y - viewport.height) as i32;
        unsafe {
            (self.gl.viewport)(x, y, viewport.width as i32, viewport.height as i32);
            if scissor {
                (self.gl.enable)(SCISSOR_TEST);
                (self.gl.scissor)(x, y, viewport.width as i32, viewport.height as i32);
            } else {
                (self.gl.disable)(SCISSOR_TEST);
            }
        }
    }

    fn entity(
        &mut self,
        entity: &SceneEntity,
        assets: &Assets,
        projection: &[f32; 16],
        stats: &mut BackendStats,
    ) {
        let (Some(model), Some(&mesh)) = (
            assets.model(entity.model),
            self.meshes.get(entity.model.0 as usize),
        ) else {
            stats.rejected = stats.rejected.saturating_add(1);
            return;
        };
        let material_id = entity.material.unwrap_or(model.material);
        let Some(material) = assets.material(material_id) else {
            stats.rejected = stats.rejected.saturating_add(1);
            return;
        };
        if mesh.count == 0 {
            return;
        }
        let matrix = multiply(projection, &model_matrix(entity));
        self.bind_vao(self.static_vao);
        self.set_depth_hack(entity.depth_hack);
        for stage in material.stages.iter() {
            if !self.stage(
                *stage,
                &matrix,
                rgba(entity.color),
                !material.two_sided,
                true,
                false,
            ) {
                stats.rejected = stats.rejected.saturating_add(1);
                continue;
            }
            unsafe {
                (self.gl.draw_elements)(
                    TRIANGLES,
                    mesh.count,
                    UNSIGNED_INT,
                    (mesh.first_index * size_of::<u32>()) as *const c_void,
                    mesh.base_vertex,
                );
            }
        }
        stats.triangles = stats.triangles.saturating_add(mesh.count as u32 / 3);
        self.set_depth_hack(false);
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_dynamic(
        &mut self,
        assets: &Assets,
        material_id: MaterialId,
        first: usize,
        count: usize,
        matrix: &[f32; 16],
        color: [f32; 4],
        depth: bool,
        force_alpha: bool,
    ) -> bool {
        let Some(material) = assets.material(material_id) else {
            return false;
        };
        self.bind_vao(self.dynamic.vao);
        self.set_depth_hack(false);
        let mut complete = true;
        for stage in material.stages.iter() {
            if !self.stage(
                *stage,
                matrix,
                color,
                depth && !material.two_sided,
                depth,
                force_alpha,
            ) {
                complete = false;
                continue;
            }
            unsafe { (self.gl.draw_arrays)(TRIANGLE_FAN, first as i32, count as i32) };
        }
        complete
    }

    fn draw_2d(&mut self, draw: Draw2d, assets: &Assets) -> bool {
        let vertices = quad(draw.rect, draw.texcoords, [255; 4]);
        let Some(first) = self.upload(&vertices) else {
            return false;
        };
        let projection = orthographic(self.width, self.height);
        self.draw_dynamic(
            assets,
            draw.material,
            first,
            vertices.len(),
            &projection,
            rgba(draw.color),
            false,
            true,
        )
    }

    fn tint(&mut self, viewport: Viewport, color: [f32; 4], stats: &mut BackendStats) {
        if color[3] <= 0.0 {
            return;
        }
        let vertices = quad(
            [
                viewport.x as f32,
                viewport.y as f32,
                viewport.width as f32,
                viewport.height as f32,
            ],
            [0.0, 0.0, 1.0, 1.0],
            [255; 4],
        );
        let Some(first) = self.upload(&vertices) else {
            stats.rejected = stats.rejected.saturating_add(1);
            return;
        };
        self.viewport(
            Viewport {
                x: 0,
                y: 0,
                width: self.width,
                height: self.height,
            },
            false,
        );
        self.bind_vao(self.dynamic.vao);
        self.set_depth_hack(false);
        let stage = Stage {
            blend: Blend::Alpha,
            ..Stage::default()
        };
        let projection = orthographic(self.width, self.height);
        if !self.stage(stage, &projection, color, false, false, false) {
            stats.rejected = stats.rejected.saturating_add(1);
            return;
        }
        unsafe { (self.gl.draw_arrays)(TRIANGLE_FAN, first as i32, 4) };
    }

    #[allow(clippy::too_many_arguments)]
    fn stage(
        &mut self,
        stage: Stage,
        matrix: &[f32; 16],
        color: [f32; 4],
        cull: bool,
        depth: bool,
        force_alpha: bool,
    ) -> bool {
        let Some(&texture) = self.textures.get(stage.image.0 as usize) else {
            return false;
        };
        let blend = if force_alpha && stage.blend == Blend::Opaque {
            Blend::Alpha
        } else {
            stage.blend
        };
        self.depth_state(depth, depth && stage.depth_write);
        if self.state.depth_func != Some(stage.depth_func) {
            let function = match stage.depth_func {
                DepthFunc::Lequal => 0x0203,
                DepthFunc::Equal => 0x0202,
                DepthFunc::Always => 0x0207,
            };
            unsafe { (self.gl.depth_func)(function) };
            self.state.depth_func = Some(stage.depth_func);
        }
        self.blend_state(blend);
        if self.state.cull != Some(cull) {
            unsafe {
                if cull {
                    (self.gl.enable)(CULL_FACE)
                } else {
                    (self.gl.disable)(CULL_FACE)
                }
            }
            self.state.cull = Some(cull);
        }
        if self.state.texture != texture {
            unsafe { (self.gl.bind_texture)(TEXTURE_2D, texture) };
            self.state.texture = texture;
        }
        unsafe {
            (self.gl.uniform_matrix)(self.uniforms.mvp, 1, 0, matrix.as_ptr());
            (self.gl.uniform_color)(self.uniforms.color, 1, color.as_ptr());
            (self.gl.uniform_integer)(
                self.uniforms.texgen,
                match stage.texgen {
                    TcGen::Texture => 0,
                    TcGen::Lightmap => 1,
                },
            );
            (self.gl.uniform_integer)(self.uniforms.vertex_color, i32::from(stage.vertex_color));
            (self.gl.uniform_integer)(
                self.uniforms.alpha_test,
                match stage.alpha_test {
                    AlphaTest::None => 0,
                    AlphaTest::GreaterZero => 1,
                    AlphaTest::AtLeastHalf => 2,
                },
            );
        }
        true
    }

    fn bind_vao(&mut self, vao: u32) {
        if self.state.vao != vao {
            unsafe { (self.gl.bind_vertex_array)(vao) };
            self.state.vao = vao;
        }
    }
    fn depth_state(&mut self, depth: bool, write: bool) {
        if self.state.depth != Some(depth) {
            unsafe {
                if depth {
                    (self.gl.enable)(DEPTH_TEST)
                } else {
                    (self.gl.disable)(DEPTH_TEST)
                }
            }
            self.state.depth = Some(depth);
        }
        if self.state.depth_write != Some(write) {
            unsafe { (self.gl.depth_mask)(u8::from(write)) };
            self.state.depth_write = Some(write);
        }
    }
    fn set_depth_hack(&mut self, enabled: bool) {
        if self.state.depth_hack != enabled {
            unsafe { (self.gl.depth_range)(0.0, if enabled { 0.3 } else { 1.0 }) };
            self.state.depth_hack = enabled;
        }
    }
    fn blend_state(&mut self, blend: Blend) {
        if self.state.blend == Some(blend) {
            return;
        }
        unsafe {
            if blend == Blend::Opaque {
                (self.gl.disable)(BLEND);
            } else {
                (self.gl.enable)(BLEND);
                let factors = match blend {
                    Blend::Opaque => (1, 0),
                    Blend::Alpha => (0x0302, 0x0303),
                    Blend::Add => (1, 1),
                    Blend::Multiply => (0x0306, 0),
                };
                (self.gl.blend_func)(factors.0, factors.1);
            }
        }
        self.state.blend = Some(blend);
    }

    /// Copies the current back buffer to GL's bottom-up RGBA bytes. Call before
    /// the platform swap. Readback is a private verification operation, not a frame stage.
    pub fn read_pixels(&self, out: &mut [u8]) -> bool {
        let Some(stride) = (self.width as usize).checked_mul(4) else {
            return false;
        };
        let Some(length) = stride.checked_mul(self.height as usize) else {
            return false;
        };
        if out.len() < length {
            return false;
        }
        unsafe {
            (self.gl.read_buffer)(0x0405);
            (self.gl.pixel_store)(0x0d05, 1);
            (self.gl.read_pixels)(
                0,
                0,
                self.width as i32,
                self.height as i32,
                RGBA,
                UNSIGNED_BYTE,
                out.as_mut_ptr().cast(),
            );
            if (self.gl.get_error)() != 0 {
                return false;
            }
        }
        true
    }
}

impl Drop for GlBackend {
    fn drop(&mut self) {
        unsafe {
            for &fence in &self.dynamic.fences {
                if !fence.is_null() {
                    (self.gl.delete_fence)(fence)
                }
            }
            if self.dynamic.mapped.is_some() {
                (self.gl.bind_buffer)(ARRAY_BUFFER, self.dynamic.buffer);
                (self.gl.unmap_buffer)(ARRAY_BUFFER);
            }
            (self.gl.delete_buffers)(2, self.static_buffers.as_ptr());
            (self.gl.delete_buffers)(1, &self.dynamic.buffer);
            (self.gl.delete_vertex_arrays)(1, &self.static_vao);
            (self.gl.delete_vertex_arrays)(1, &self.dynamic.vao);
            (self.gl.delete_textures)(self.textures.len() as i32, self.textures.as_ptr());
            (self.gl.delete_program)(self.program);
        }
    }
}

unsafe fn vertex_layout(gl: &Gl) {
    let stride = size_of::<Vertex>() as i32;
    unsafe {
        for index in 0..4 {
            (gl.enable_vertex_attribute)(index)
        }
        (gl.vertex_attribute)(
            0,
            3,
            FLOAT,
            0,
            stride,
            offset_of!(Vertex, position) as *const c_void,
        );
        (gl.vertex_attribute)(
            1,
            2,
            FLOAT,
            0,
            stride,
            offset_of!(Vertex, texcoord) as *const c_void,
        );
        (gl.vertex_attribute)(
            2,
            2,
            FLOAT,
            0,
            stride,
            offset_of!(Vertex, lightmap_coord) as *const c_void,
        );
        (gl.vertex_attribute)(
            3,
            4,
            UNSIGNED_BYTE,
            1,
            stride,
            offset_of!(Vertex, color) as *const c_void,
        );
    }
}

unsafe fn context_string(gl: &Gl, name: u32) -> Result<String, String> {
    let address = unsafe { (gl.get_string)(name) };
    if address.is_null() {
        return Err("GL context identity is unavailable".into());
    }
    Ok(unsafe { CStr::from_ptr(address.cast()) }
        .to_string_lossy()
        .into_owned())
}

unsafe fn compile_program(gl: &Gl) -> Result<u32, String> {
    let vertex = unsafe { compile_shader(gl, 0x8b31, VERTEX_SHADER)? };
    let fragment = match unsafe { compile_shader(gl, 0x8b30, FRAGMENT_SHADER) } {
        Ok(shader) => shader,
        Err(error) => {
            unsafe { (gl.delete_shader)(vertex) };
            return Err(error);
        }
    };
    let program = unsafe { (gl.create_program)() };
    if program == 0 {
        unsafe {
            (gl.delete_shader)(vertex);
            (gl.delete_shader)(fragment)
        };
        return Err("GL program creation failed".into());
    }
    let mut status = 0;
    unsafe {
        (gl.attach_shader)(program, vertex);
        (gl.attach_shader)(program, fragment);
        (gl.link_program)(program);
        (gl.get_program)(program, 0x8b82, &mut status);
        (gl.delete_shader)(vertex);
        (gl.delete_shader)(fragment);
    }
    if status == 0 {
        let log = unsafe { shader_log(gl, program, true) };
        unsafe { (gl.delete_program)(program) };
        return Err(format!("GL program link failed: {log}"));
    }
    Ok(program)
}
unsafe fn compile_shader(gl: &Gl, kind: u32, source: &str) -> Result<u32, String> {
    let shader = unsafe { (gl.create_shader)(kind) };
    if shader == 0 {
        return Err("GL shader creation failed".into());
    }
    let address = source.as_ptr().cast();
    let length = source.len() as i32;
    let mut status = 0;
    unsafe {
        (gl.shader_source)(shader, 1, &address, &length);
        (gl.compile_shader)(shader);
        (gl.get_shader)(shader, 0x8b81, &mut status);
    }
    if status == 0 {
        let log = unsafe { shader_log(gl, shader, false) };
        unsafe { (gl.delete_shader)(shader) };
        return Err(format!("GL shader compilation failed: {log}"));
    }
    Ok(shader)
}
unsafe fn shader_log(gl: &Gl, object: u32, program: bool) -> String {
    let mut length = 0;
    unsafe {
        if program {
            (gl.get_program)(object, 0x8b84, &mut length)
        } else {
            (gl.get_shader)(object, 0x8b84, &mut length)
        }
    }
    let mut bytes = vec![0_u8; length.clamp(1, 1_048_576) as usize];
    let mut written = 0;
    unsafe {
        if program {
            (gl.program_log)(
                object,
                bytes.len() as i32,
                &mut written,
                bytes.as_mut_ptr().cast(),
            )
        } else {
            (gl.shader_log)(
                object,
                bytes.len() as i32,
                &mut written,
                bytes.as_mut_ptr().cast(),
            )
        }
    }
    bytes.truncate((written.max(0) as usize).min(bytes.len()));
    String::from_utf8_lossy(&bytes).into_owned()
}

fn valid_refdef(view: Refdef, width: u32, height: u32) -> bool {
    valid_viewport(view.viewport, width, height)
        && (view.blend_phase != BlendPhase::FinalPalette
            || view
                .blend_viewport
                .is_none_or(|viewport| valid_viewport(viewport, width, height)))
        && view
            .origin
            .0
            .iter()
            .chain(view.axes.iter().flat_map(|axis| axis.0.iter()))
            .chain(view.fov.iter())
            .chain(view.blend.iter())
            .all(|f| f.is_finite())
        && view.fov.iter().all(|&f| f > 0.0 && f < 180.0)
        && view.near.is_finite()
        && view.near > 0.0
        && view.far.is_finite()
        && view.far > view.near
}
fn valid_viewport(viewport: Viewport, width: u32, height: u32) -> bool {
    viewport.width != 0
        && viewport.height != 0
        && viewport
            .x
            .checked_add(viewport.width)
            .is_some_and(|x| x <= width)
        && viewport
            .y
            .checked_add(viewport.height)
            .is_some_and(|y| y <= height)
}
fn rgba(color: [u8; 4]) -> [f32; 4] {
    color.map(|v| v as f32 / 255.0)
}
fn view_projection(view: Refdef) -> [f32; 16] {
    let x = 1.0 / (view.fov[0].to_radians() * 0.5).tan();
    let y = 1.0 / (view.fov[1].to_radians() * 0.5).tan();
    let a = (view.far + view.near) / (view.far - view.near);
    let b = -2.0 * view.far * view.near / (view.far - view.near);
    let (forward, left, up) = (view.axes[0], view.axes[1], view.axes[2]);
    [
        -left.0[0] * x,
        up.0[0] * y,
        forward.0[0] * a,
        forward.0[0],
        -left.0[1] * x,
        up.0[1] * y,
        forward.0[1] * a,
        forward.0[1],
        -left.0[2] * x,
        up.0[2] * y,
        forward.0[2] * a,
        forward.0[2],
        left.dot(view.origin) * x,
        -up.dot(view.origin) * y,
        -forward.dot(view.origin) * a + b,
        -forward.dot(view.origin),
    ]
}
fn model_matrix(entity: &SceneEntity) -> [f32; 16] {
    [
        entity.axes[0].0[0],
        entity.axes[0].0[1],
        entity.axes[0].0[2],
        0.0,
        entity.axes[1].0[0],
        entity.axes[1].0[1],
        entity.axes[1].0[2],
        0.0,
        entity.axes[2].0[0],
        entity.axes[2].0[1],
        entity.axes[2].0[2],
        0.0,
        entity.origin.0[0],
        entity.origin.0[1],
        entity.origin.0[2],
        1.0,
    ]
}
fn multiply(a: &[f32; 16], b: &[f32; 16]) -> [f32; 16] {
    std::array::from_fn(|index| {
        let row = index % 4;
        let column = index / 4 * 4;
        a[row] * b[column]
            + a[row + 4] * b[column + 1]
            + a[row + 8] * b[column + 2]
            + a[row + 12] * b[column + 3]
    })
}
fn orthographic(width: u32, height: u32) -> [f32; 16] {
    [
        2.0 / width as f32,
        0.0,
        0.0,
        0.0,
        0.0,
        -2.0 / height as f32,
        0.0,
        0.0,
        0.0,
        0.0,
        1.0,
        0.0,
        -1.0,
        1.0,
        0.0,
        1.0,
    ]
}
fn quad(rect: [f32; 4], uv: [f32; 4], color: [u8; 4]) -> [Vertex; 4] {
    let (x, y, w, h) = (rect[0], rect[1], rect[2], rect[3]);
    [
        ([x, y, 0.0], [uv[0], uv[1]]),
        ([x + w, y, 0.0], [uv[2], uv[1]]),
        ([x + w, y + h, 0.0], [uv[2], uv[3]]),
        ([x, y + h, 0.0], [uv[0], uv[3]]),
    ]
    .map(|(position, texcoord)| Vertex {
        position: Vec3(position),
        texcoord,
        lightmap_coord: texcoord,
        color,
    })
}
