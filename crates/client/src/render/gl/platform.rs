//! Live GL adapter over the `qa-platform` procedure tables.

use std::ffi::{c_char, c_void, CStr, CString};

use qa_platform::error::Result;
use qa_platform::gl::Gl;
use qa_platform::gl_framebuffers::GlFramebuffers;
use qa_platform::gl_programs::GlPrograms;
use qa_platform::sdl_render_context::SdlRenderContext;

use super::GlContext;

/// [`GlContext`] over real GL symbols. Constructed only with a live SDL
/// context; every call selects that context before invoking its symbol.
pub struct PlatformGlContext<'a> {
    gl: Gl,
    programs: GlPrograms,
    framebuffers: GlFramebuffers,
    context: &'a mut dyn SdlRenderContext,
}

impl<'a> PlatformGlContext<'a> {
    /// Resolve every GL table through `context`, retaining procedures.
    pub fn load(context: &'a mut dyn SdlRenderContext) -> Result<Self> {
        let gl = Gl::load(&mut *context)?;
        let programs = GlPrograms::load(&mut *context)?;
        let framebuffers = GlFramebuffers::load(&mut *context)?;
        Ok(Self {
            gl,
            programs,
            framebuffers,
            context,
        })
    }

    fn current(&mut self) {
        self.context.make_current().expect("GL context must be current");
    }

    /// Release all procedure leases. Idempotent.
    pub fn close(&mut self) {
        self.gl.close();
        self.programs.close();
        self.framebuffers.close();
    }

    #[must_use]
    pub fn is_closed(&self) -> bool {
        self.gl.is_closed() && self.programs.is_closed() && self.framebuffers.is_closed()
    }
}

fn gl_bool(value: bool) -> u8 {
    u8::from(value)
}

impl GlContext for PlatformGlContext<'_> {
    fn viewport(&mut self, x: i32, y: i32, width: i32, height: i32) {
        self.current();
        // SAFETY: live glViewport with integer geometry; context is current.
        unsafe {
            (self.gl.symbols.gl_viewport)(x, y, width, height);
        }
    }
    fn scissor(&mut self, x: i32, y: i32, width: i32, height: i32) {
        self.current();
        // SAFETY: live glScissor with integer geometry; context is current.
        unsafe {
            (self.gl.symbols.gl_scissor)(x, y, width, height);
        }
    }
    fn clear_color(&mut self, red: f32, green: f32, blue: f32, alpha: f32) {
        self.current();
        // SAFETY: live glClearColor with float components; context is current.
        unsafe {
            (self.gl.symbols.gl_clear_color)(red, green, blue, alpha);
        }
    }
    fn clear_depth(&mut self, depth: f64) {
        self.current();
        // SAFETY: live glClearDepth with a scalar depth; context is current.
        unsafe {
            (self.gl.symbols.gl_clear_depth)(depth);
        }
    }
    fn clear_stencil(&mut self, stencil: i32) {
        self.current();
        // SAFETY: live glClearStencil with a scalar value; context is current.
        unsafe {
            (self.gl.symbols.gl_clear_stencil)(stencil);
        }
    }
    fn clear(&mut self, mask: u32) {
        self.current();
        // SAFETY: live glClear with a buffer mask; context is current.
        unsafe {
            (self.gl.symbols.gl_clear)(mask);
        }
    }
    fn enable(&mut self, cap: u32) {
        self.current();
        // SAFETY: live glEnable with an enumerant; context is current.
        unsafe {
            (self.gl.symbols.gl_enable)(cap);
        }
    }
    fn disable(&mut self, cap: u32) {
        self.current();
        // SAFETY: live glDisable with an enumerant; context is current.
        unsafe {
            (self.gl.symbols.gl_disable)(cap);
        }
    }
    fn is_enabled(&mut self, cap: u32) -> bool {
        self.current();
        // SAFETY: live glIsEnabled with an enumerant; context is current.
        unsafe { (self.gl.symbols.gl_is_enabled)(cap) != 0 }
    }
    fn get_integerv(&mut self, pname: u32, out: &mut [i32]) {
        self.current();
        // SAFETY: live glGetIntegerv writing exactly out.len() ints; context is current.
        unsafe {
            (self.gl.symbols.gl_get_integerv)(pname, out.as_mut_ptr());
        }
    }
    fn get_floatv(&mut self, pname: u32, out: &mut [f32]) {
        self.current();
        // SAFETY: live glGetFloatv writing exactly out.len() floats; context is current.
        unsafe {
            (self.gl.symbols.gl_get_floatv)(pname, out.as_mut_ptr());
        }
    }
    fn get_string(&mut self, name: u32) -> String {
        self.current();
        // SAFETY: live glGetString with an enumerant; context is current.
        let pointer = unsafe { (self.gl.symbols.gl_get_string)(name) };
        if pointer.is_null() {
            return String::new();
        }
        // SAFETY: the driver returns a valid NUL-terminated string.
        unsafe { CStr::from_ptr(pointer.cast::<c_char>()).to_string_lossy().into_owned() }
    }
    fn blend_func(&mut self, src: u32, dst: u32) {
        self.current();
        // SAFETY: live glBlendFunc with enumerants; context is current.
        unsafe {
            (self.gl.symbols.gl_blend_func)(src, dst);
        }
    }
    fn depth_func(&mut self, func: u32) {
        self.current();
        // SAFETY: live glDepthFunc with an enumerant; context is current.
        unsafe {
            (self.gl.symbols.gl_depth_func)(func);
        }
    }
    fn depth_mask(&mut self, flag: bool) {
        self.current();
        // SAFETY: live glDepthMask with a boolean byte; context is current.
        unsafe {
            (self.gl.symbols.gl_depth_mask)(gl_bool(flag));
        }
    }
    fn color_mask(&mut self, red: bool, green: bool, blue: bool, alpha: bool) {
        self.current();
        // SAFETY: live glColorMask with boolean bytes; context is current.
        unsafe {
            (self.gl.symbols.gl_color_mask)(gl_bool(red), gl_bool(green), gl_bool(blue), gl_bool(alpha));
        }
    }
    fn stencil_func(&mut self, func: u32, reference: i32, mask: u32) {
        self.current();
        // SAFETY: live glStencilFunc with scalar arguments; context is current.
        unsafe {
            (self.gl.symbols.gl_stencil_func)(func, reference, mask);
        }
    }
    fn stencil_op(&mut self, fail: u32, zfail: u32, zpass: u32) {
        self.current();
        // SAFETY: live glStencilOp with enumerants; context is current.
        unsafe {
            (self.gl.symbols.gl_stencil_op)(fail, zfail, zpass);
        }
    }
    fn stencil_mask(&mut self, mask: u32) {
        self.current();
        // SAFETY: live glStencilMask with a bit mask; context is current.
        unsafe {
            (self.gl.symbols.gl_stencil_mask)(mask);
        }
    }
    fn depth_range(&mut self, near: f64, far: f64) {
        self.current();
        // SAFETY: live glDepthRange with scalar bounds; context is current.
        unsafe {
            (self.gl.symbols.gl_depth_range)(near, far);
        }
    }
    fn polygon_mode(&mut self, face: u32, mode: u32) {
        self.current();
        // SAFETY: live glPolygonMode with enumerants; context is current.
        unsafe {
            (self.gl.symbols.gl_polygon_mode)(face, mode);
        }
    }
    fn shade_model(&mut self, model: u32) {
        self.current();
        // SAFETY: live glShadeModel with an enumerant; context is current.
        unsafe {
            (self.gl.symbols.gl_shade_model)(model);
        }
    }
    fn polygon_offset(&mut self, factor: f32, units: f32) {
        self.current();
        // SAFETY: live glPolygonOffset with scalars; context is current.
        unsafe {
            (self.gl.symbols.gl_polygon_offset)(factor, units);
        }
    }
    fn line_width(&mut self, width: f32) {
        self.current();
        // SAFETY: live glLineWidth with a scalar; context is current.
        unsafe {
            (self.gl.symbols.gl_line_width)(width);
        }
    }
    fn cull_face(&mut self, mode: u32) {
        self.current();
        // SAFETY: live glCullFace with an enumerant; context is current.
        unsafe {
            (self.gl.symbols.gl_cull_face)(mode);
        }
    }
    fn front_face(&mut self, mode: u32) {
        self.current();
        // SAFETY: live glFrontFace with an enumerant; context is current.
        unsafe {
            (self.gl.symbols.gl_front_face)(mode);
        }
    }
    fn clip_plane(&mut self, plane: u32, equation: &[f64; 4]) {
        self.current();
        // SAFETY: live glClipPlane reading four doubles; context is current.
        unsafe {
            (self.gl.symbols.gl_clip_plane)(plane, equation.as_ptr());
        }
    }
    fn matrix_mode(&mut self, mode: u32) {
        self.current();
        // SAFETY: live glMatrixMode with an enumerant; context is current.
        unsafe {
            (self.gl.symbols.gl_matrix_mode)(mode);
        }
    }
    fn load_identity(&mut self) {
        self.current();
        // SAFETY: live glLoadIdentity without arguments; context is current.
        unsafe {
            (self.gl.symbols.gl_load_identity)();
        }
    }
    fn ortho(&mut self, left: f64, right: f64, bottom: f64, top: f64, near: f64, far: f64) {
        self.current();
        // SAFETY: live glOrtho with scalar bounds; context is current.
        unsafe {
            (self.gl.symbols.gl_ortho)(left, right, bottom, top, near, far);
        }
    }
    fn begin(&mut self, mode: u32) {
        self.current();
        // SAFETY: live glBegin with a primitive enumerant; context is current.
        unsafe {
            (self.gl.symbols.gl_begin)(mode);
        }
    }
    fn end(&mut self) {
        self.current();
        // SAFETY: live glEnd closing the open primitive; context is current.
        unsafe {
            (self.gl.symbols.gl_end)();
        }
    }
    fn color_3f(&mut self, red: f32, green: f32, blue: f32) {
        self.current();
        // SAFETY: live glColor3f with float components; context is current.
        unsafe {
            (self.gl.symbols.gl_color_3f)(red, green, blue);
        }
    }
    fn color_4f(&mut self, red: f32, green: f32, blue: f32, alpha: f32) {
        self.current();
        // SAFETY: live glColor4f with float components; context is current.
        unsafe {
            (self.gl.symbols.gl_color_4f)(red, green, blue, alpha);
        }
    }
    fn tex_coord_2f(&mut self, s: f32, t: f32) {
        self.current();
        // SAFETY: live glTexCoord2f with float components; context is current.
        unsafe {
            (self.gl.symbols.gl_tex_coord_2f)(s, t);
        }
    }
    fn vertex_2f(&mut self, x: f32, y: f32) {
        self.current();
        // SAFETY: live glVertex2f with float components; context is current.
        unsafe {
            (self.gl.symbols.gl_vertex_2f)(x, y);
        }
    }
    fn vertex_4f(&mut self, x: f32, y: f32, z: f32, w: f32) {
        self.current();
        // SAFETY: live glVertex4f with float components; context is current.
        unsafe {
            (self.gl.symbols.gl_vertex_4f)(x, y, z, w);
        }
    }
    fn enable_client_state(&mut self, array: u32) {
        self.current();
        // SAFETY: live glEnableClientState with an enumerant; context is current.
        unsafe {
            (self.gl.symbols.gl_enable_client_state)(array);
        }
    }
    fn disable_client_state(&mut self, array: u32) {
        self.current();
        // SAFETY: live glDisableClientState with an enumerant; context is current.
        unsafe {
            (self.gl.symbols.gl_disable_client_state)(array);
        }
    }
    fn push_client_attrib(&mut self, mask: u32) {
        self.current();
        // SAFETY: live glPushClientAttrib with a bit mask; context is current.
        unsafe {
            (self.gl.symbols.gl_push_client_attrib)(mask);
        }
    }
    fn pop_client_attrib(&mut self) {
        self.current();
        // SAFETY: live glPopClientAttrib without arguments; context is current.
        unsafe {
            (self.gl.symbols.gl_pop_client_attrib)();
        }
    }
    fn vertex_pointer(&mut self, size: i32, stride: i32, data: &[f32]) {
        self.current();
        // SAFETY: live glVertexPointer over client floats outliving the call; context is current.
        unsafe {
            (self.gl.symbols.gl_vertex_pointer)(size, super::FLOAT, stride, data.as_ptr().cast::<c_void>());
        }
    }
    fn color_pointer(&mut self, size: i32, stride: i32, data: &[f32]) {
        self.current();
        // SAFETY: live glColorPointer over client floats outliving the call; context is current.
        unsafe {
            (self.gl.symbols.gl_color_pointer)(size, super::FLOAT, stride, data.as_ptr().cast::<c_void>());
        }
    }
    fn tex_coord_pointer(&mut self, size: i32, stride: i32, data: &[f32]) {
        self.current();
        // SAFETY: live glTexCoordPointer over client floats outliving the call; context is current.
        unsafe {
            (self.gl.symbols.gl_tex_coord_pointer)(size, super::FLOAT, stride, data.as_ptr().cast::<c_void>());
        }
    }
    fn draw_elements(&mut self, mode: u32, indices: &[u32]) {
        self.current();
        // SAFETY: live glDrawElements over client indices outliving the call; context is current.
        unsafe {
            (self.gl.symbols.gl_draw_elements)(
                mode,
                indices.len() as i32,
                super::UNSIGNED_INT,
                indices.as_ptr().cast::<c_void>(),
            );
        }
    }
    fn gen_buffers(&mut self, count: i32) -> Vec<u32> {
        self.current();
        let mut names = vec![0u32; count.max(0) as usize];
        if names.is_empty() {
            return names;
        }
        // SAFETY: live glGenBuffers writing count names; context is current.
        unsafe {
            (self.gl.symbols.gl_gen_buffers)(count, names.as_mut_ptr());
        }
        names
    }
    fn delete_buffers(&mut self, names: &[u32]) {
        if names.is_empty() {
            return;
        }
        self.current();
        // SAFETY: live glDeleteBuffers reading the name slice; context is current.
        unsafe {
            (self.gl.symbols.gl_delete_buffers)(names.len() as i32, names.as_ptr());
        }
    }
    fn bind_buffer(&mut self, target: u32, name: u32) {
        self.current();
        // SAFETY: live glBindBuffer with a target and name; context is current.
        unsafe {
            (self.gl.symbols.gl_bind_buffer)(target, name);
        }
    }
    fn active_texture(&mut self, unit: u32) {
        self.current();
        // SAFETY: live glActiveTexture with a unit enumerant; context is current.
        unsafe {
            (self.gl.symbols.gl_active_texture)(unit);
        }
    }
    fn client_active_texture(&mut self, unit: u32) {
        self.current();
        // SAFETY: live glClientActiveTexture with a unit enumerant; context is current.
        unsafe {
            (self.gl.symbols.gl_client_active_texture)(unit);
        }
    }
    fn gen_textures(&mut self, count: i32) -> Vec<u32> {
        self.current();
        let mut names = vec![0u32; count.max(0) as usize];
        if names.is_empty() {
            return names;
        }
        // SAFETY: live glGenTextures writing count names; context is current.
        unsafe {
            (self.gl.symbols.gl_gen_textures)(count, names.as_mut_ptr());
        }
        names
    }
    fn delete_textures(&mut self, names: &[u32]) {
        if names.is_empty() {
            return;
        }
        self.current();
        // SAFETY: live glDeleteTextures reading the name slice; context is current.
        unsafe {
            (self.gl.symbols.gl_delete_textures)(names.len() as i32, names.as_ptr());
        }
    }
    fn bind_texture(&mut self, target: u32, name: u32) {
        self.current();
        // SAFETY: live glBindTexture with a target and name; context is current.
        unsafe {
            (self.gl.symbols.gl_bind_texture)(target, name);
        }
    }
    fn tex_parameteri(&mut self, target: u32, pname: u32, value: i32) {
        self.current();
        // SAFETY: live glTexParameteri with enumerants and a scalar; context is current.
        unsafe {
            (self.gl.symbols.gl_tex_parameteri)(target, pname, value);
        }
    }
    fn tex_parameterfv(&mut self, target: u32, pname: u32, values: &[f32; 4]) {
        self.current();
        // SAFETY: live glTexParameterfv reading four floats; context is current.
        unsafe {
            (self.gl.symbols.gl_tex_parameterfv)(target, pname, values.as_ptr());
        }
    }
    fn tex_image_2d_null(
        &mut self,
        target: u32,
        level: i32,
        internal: i32,
        width: i32,
        height: i32,
        border: i32,
        format: u32,
        ty: u32,
    ) {
        self.current();
        // SAFETY: live glTexImage2D with null pixels; context is current.
        unsafe {
            (self.gl.symbols.gl_tex_image_2d)(
                target,
                level,
                internal,
                width,
                height,
                border,
                format,
                ty,
                std::ptr::null(),
            );
        }
    }
    fn tex_image_2d_bytes(
        &mut self,
        target: u32,
        level: i32,
        internal: i32,
        width: i32,
        height: i32,
        border: i32,
        format: u32,
        ty: u32,
        pixels: &[u8],
    ) {
        self.current();
        // SAFETY: live glTexImage2D over client bytes outliving the call; context is current.
        unsafe {
            (self.gl.symbols.gl_tex_image_2d)(
                target,
                level,
                internal,
                width,
                height,
                border,
                format,
                ty,
                pixels.as_ptr().cast::<c_void>(),
            );
        }
    }
    fn tex_image_2d_floats(
        &mut self,
        target: u32,
        level: i32,
        internal: i32,
        width: i32,
        height: i32,
        border: i32,
        format: u32,
        ty: u32,
        pixels: &[f32],
    ) {
        self.current();
        // SAFETY: live glTexImage2D over client floats outliving the call; context is current.
        unsafe {
            (self.gl.symbols.gl_tex_image_2d)(
                target,
                level,
                internal,
                width,
                height,
                border,
                format,
                ty,
                pixels.as_ptr().cast::<c_void>(),
            );
        }
    }
    fn copy_tex_image_2d(
        &mut self,
        target: u32,
        level: i32,
        internal: i32,
        x: i32,
        y: i32,
        width: i32,
        height: i32,
        border: i32,
    ) {
        self.current();
        // SAFETY: live glCopyTexImage2D with integer geometry; context is current.
        unsafe {
            (self.gl.symbols.gl_copy_tex_image_2d)(target, level, internal as u32, x, y, width, height, border);
        }
    }
    fn tex_sub_image_2d_bytes(
        &mut self,
        target: u32,
        level: i32,
        x: i32,
        y: i32,
        width: i32,
        height: i32,
        format: u32,
        ty: u32,
        pixels: &[u8],
    ) {
        self.current();
        // SAFETY: live glTexSubImage2D over client bytes outliving the call; context is current.
        unsafe {
            (self.gl.symbols.gl_tex_sub_image_2d)(
                target,
                level,
                x,
                y,
                width,
                height,
                format,
                ty,
                pixels.as_ptr().cast::<c_void>(),
            );
        }
    }
    fn tex_sub_image_2d_floats(
        &mut self,
        target: u32,
        level: i32,
        x: i32,
        y: i32,
        width: i32,
        height: i32,
        format: u32,
        ty: u32,
        pixels: &[f32],
    ) {
        self.current();
        // SAFETY: live glTexSubImage2D over client floats outliving the call; context is current.
        unsafe {
            (self.gl.symbols.gl_tex_sub_image_2d)(
                target,
                level,
                x,
                y,
                width,
                height,
                format,
                ty,
                pixels.as_ptr().cast::<c_void>(),
            );
        }
    }
    fn get_tex_image_floats(&mut self, target: u32, level: i32, format: u32, ty: u32, dest: &mut [f32]) {
        self.current();
        // SAFETY: live glGetTexImage writing the float destination; context is current.
        unsafe {
            (self.gl.symbols.gl_get_tex_image)(target, level, format, ty, dest.as_mut_ptr().cast::<c_void>());
        }
    }
    fn create_shader(&mut self, ty: u32) -> u32 {
        self.current();
        // SAFETY: live glCreateShader with a shader enumerant; context is current.
        unsafe { (self.programs.symbols.gl_create_shader)(ty) }
    }
    fn shader_source(&mut self, shader: u32, source: &str) {
        self.programs
            .shader_source(&mut *self.context, shader, source)
            .expect("valid GL shader source");
    }
    fn compile_shader(&mut self, shader: u32) {
        self.current();
        // SAFETY: live glCompileShader with an allocated shader; context is current.
        unsafe {
            (self.programs.symbols.gl_compile_shader)(shader);
        }
    }
    fn get_shaderiv(&mut self, shader: u32, pname: u32) -> i32 {
        self.current();
        let mut value = 0;
        // SAFETY: live glGetShaderiv writing one int; context is current.
        unsafe {
            (self.programs.symbols.gl_get_shaderiv)(shader, pname, &mut value);
        }
        value
    }
    fn get_shader_info_log(&mut self, shader: u32, buf: &mut [u8]) -> usize {
        self.current();
        let mut size = 0;
        // SAFETY: live glGetShaderInfoLog writing at most buf.len() bytes; context is current.
        unsafe {
            (self.programs.symbols.gl_get_shader_info_log)(shader, buf.len() as i32, &mut size, buf.as_mut_ptr());
        }
        size.max(0) as usize
    }
    fn delete_shader(&mut self, shader: u32) {
        self.current();
        // SAFETY: live glDeleteShader with an allocated shader; context is current.
        unsafe {
            (self.programs.symbols.gl_delete_shader)(shader);
        }
    }
    fn create_program(&mut self) -> u32 {
        self.current();
        // SAFETY: live glCreateProgram without arguments; context is current.
        unsafe { (self.programs.symbols.gl_create_program)() }
    }
    fn attach_shader(&mut self, program: u32, shader: u32) {
        self.current();
        // SAFETY: live glAttachShader with allocated objects; context is current.
        unsafe {
            (self.programs.symbols.gl_attach_shader)(program, shader);
        }
    }
    fn link_program(&mut self, program: u32) {
        self.current();
        // SAFETY: live glLinkProgram with an allocated program; context is current.
        unsafe {
            (self.programs.symbols.gl_link_program)(program);
        }
    }
    fn get_programiv(&mut self, program: u32, pname: u32) -> i32 {
        self.current();
        let mut value = 0;
        // SAFETY: live glGetProgramiv writing one int; context is current.
        unsafe {
            (self.programs.symbols.gl_get_programiv)(program, pname, &mut value);
        }
        value
    }
    fn get_program_info_log(&mut self, program: u32, buf: &mut [u8]) -> usize {
        self.current();
        let mut size = 0;
        // SAFETY: live glGetProgramInfoLog writing at most buf.len() bytes; context is current.
        unsafe {
            (self.programs.symbols.gl_get_program_info_log)(program, buf.len() as i32, &mut size, buf.as_mut_ptr());
        }
        size.max(0) as usize
    }
    fn delete_program(&mut self, program: u32) {
        self.current();
        // SAFETY: live glDeleteProgram with an allocated program; context is current.
        unsafe {
            (self.programs.symbols.gl_delete_program)(program);
        }
    }
    fn use_program(&mut self, program: u32) {
        self.current();
        // SAFETY: live glUseProgram with a program name; context is current.
        unsafe {
            (self.programs.symbols.gl_use_program)(program);
        }
    }
    fn get_uniform_location(&mut self, program: u32, name: &str) -> i32 {
        let name = CString::new(name).expect("uniform name has no NUL byte");
        self.current();
        // SAFETY: live glGetUniformLocation over a pinned NUL name; context is current.
        unsafe { (self.programs.symbols.gl_get_uniform_location)(program, name.as_ptr().cast::<u8>()) }
    }
    fn uniform_1i(&mut self, location: i32, value: i32) {
        self.current();
        // SAFETY: live glUniform1i with scalars; context is current.
        unsafe {
            (self.programs.symbols.gl_uniform_1i)(location, value);
        }
    }
    fn uniform_1f(&mut self, location: i32, value: f32) {
        self.current();
        // SAFETY: live glUniform1f with scalars; context is current.
        unsafe {
            (self.programs.symbols.gl_uniform_1f)(location, value);
        }
    }
    fn uniform_3f(&mut self, location: i32, x: f32, y: f32, z: f32) {
        self.current();
        // SAFETY: live glUniform3f with scalars; context is current.
        unsafe {
            (self.programs.symbols.gl_uniform_3f)(location, x, y, z);
        }
    }
    fn uniform_4f(&mut self, location: i32, x: f32, y: f32, z: f32, w: f32) {
        self.current();
        // SAFETY: live glUniform4f with scalars; context is current.
        unsafe {
            (self.programs.symbols.gl_uniform_4f)(location, x, y, z, w);
        }
    }
    fn uniform_matrix_4fv(&mut self, location: i32, value: &[f32; 16]) {
        self.current();
        // SAFETY: live glUniformMatrix4fv reading sixteen floats; context is current.
        unsafe {
            (self.programs.symbols.gl_uniform_matrix_4fv)(location, 1, 0u8, value.as_ptr());
        }
    }
    fn gen_framebuffers(&mut self, count: i32) -> Vec<u32> {
        self.current();
        let mut names = vec![0u32; count.max(0) as usize];
        if names.is_empty() {
            return names;
        }
        // SAFETY: live glGenFramebuffers writing count names; context is current.
        unsafe {
            (self.framebuffers.symbols.gl_gen_framebuffers)(count, names.as_mut_ptr());
        }
        names
    }
    fn delete_framebuffers(&mut self, names: &[u32]) {
        if names.is_empty() {
            return;
        }
        self.current();
        // SAFETY: live glDeleteFramebuffers reading the name slice; context is current.
        unsafe {
            (self.framebuffers.symbols.gl_delete_framebuffers)(names.len() as i32, names.as_ptr());
        }
    }
    fn bind_framebuffer(&mut self, target: u32, name: u32) {
        self.current();
        // SAFETY: live glBindFramebuffer with a target and name; context is current.
        unsafe {
            (self.framebuffers.symbols.gl_bind_framebuffer)(target, name);
        }
    }
    fn framebuffer_texture_2d(&mut self, target: u32, attachment: u32, textarget: u32, texture: u32, level: i32) {
        self.current();
        // SAFETY: live glFramebufferTexture2D with enumerants and names; context is current.
        unsafe {
            (self.framebuffers.symbols.gl_framebuffer_texture_2d)(target, attachment, textarget, texture, level);
        }
    }
    fn check_framebuffer_status(&mut self, target: u32) -> u32 {
        self.current();
        // SAFETY: live glCheckFramebufferStatus with a target; context is current.
        unsafe { (self.framebuffers.symbols.gl_check_framebuffer_status)(target) }
    }
    fn blit_framebuffer(&mut self, src: [i32; 4], dst: [i32; 4], mask: u32, filter: u32) {
        self.current();
        // SAFETY: live glBlitFramebuffer with integer geometry; context is current.
        unsafe {
            (self.framebuffers.symbols.gl_blit_framebuffer)(
                src[0], src[1], src[2], src[3], dst[0], dst[1], dst[2], dst[3], mask, filter,
            );
        }
    }
    fn get_framebuffer_attachment_parameteriv(&mut self, target: u32, attachment: u32, pname: u32) -> i32 {
        self.current();
        let mut value = 0;
        // SAFETY: live glGetFramebufferAttachmentParameteriv writing one int; context is current.
        unsafe {
            (self.framebuffers.symbols.gl_get_framebuffer_attachment_parameteriv)(
                target, attachment, pname, &mut value,
            );
        }
        value
    }
    fn read_buffer(&mut self, mode: u32) {
        self.current();
        // SAFETY: live glReadBuffer with an enumerant; context is current.
        unsafe {
            (self.framebuffers.symbols.gl_read_buffer)(mode);
        }
    }
    fn draw_buffer(&mut self, mode: u32) {
        self.current();
        // SAFETY: live glDrawBuffer with an enumerant; context is current.
        unsafe {
            (self.gl.symbols.gl_draw_buffer)(mode);
        }
    }
    fn pixel_storei(&mut self, pname: u32, value: i32) {
        self.current();
        // SAFETY: live glPixelStorei with an enumerant and scalar; context is current.
        unsafe {
            (self.gl.symbols.gl_pixel_storei)(pname, value);
        }
    }
    fn read_pixels_bytes(&mut self, x: i32, y: i32, width: i32, height: i32, format: u32, ty: u32, dest: &mut [u8]) {
        self.current();
        // SAFETY: live glReadPixels writing the byte destination; context is current.
        unsafe {
            (self.gl.symbols.gl_read_pixels)(x, y, width, height, format, ty, dest.as_mut_ptr().cast::<c_void>());
        }
    }
    fn read_pixels_floats(&mut self, x: i32, y: i32, width: i32, height: i32, format: u32, ty: u32, dest: &mut [f32]) {
        self.current();
        // SAFETY: live glReadPixels writing the float destination; context is current.
        unsafe {
            (self.gl.symbols.gl_read_pixels)(x, y, width, height, format, ty, dest.as_mut_ptr().cast::<c_void>());
        }
    }
    fn finish(&mut self) {
        self.current();
        // SAFETY: live glFinish without arguments; context is current.
        unsafe {
            (self.gl.symbols.gl_finish)();
        }
    }
    fn get_error(&mut self) -> u32 {
        self.current();
        // SAFETY: live glGetError without arguments; context is current.
        unsafe { (self.gl.symbols.gl_get_error)() }
    }
    fn swap_buffers(&mut self) {
        self.current();
        self.context.swap().expect("swap GL buffers");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_platform::error::Error;
    use qa_platform::sdl_render_context::ProcedureGuard;

    struct EmptyContext;

    impl SdlRenderContext for EmptyContext {
        fn drawable_size(&mut self) -> Result<(u32, u32)> {
            Ok((64, 64))
        }
        fn rendering_enabled(&self) -> bool {
            true
        }
        fn set_rendering_enabled(&mut self, _enabled: bool) -> Result<()> {
            Ok(())
        }
        fn make_current(&mut self) -> Result<()> {
            Ok(())
        }
        fn get_gl_proc_address(&mut self, name: &str) -> Result<*mut c_void> {
            Err(Error::native("SDL_GL_GetProcAddress", format!("{name} is missing")))
        }
        fn retain_procedures(&mut self) -> Result<ProcedureGuard> {
            Err(Error::native("retain", "no procedures to retain"))
        }
        fn swap(&mut self) -> Result<()> {
            Ok(())
        }
    }

    #[test]
    fn load_without_symbols_reports_gl() {
        let mut context = EmptyContext;
        let error = match PlatformGlContext::load(&mut context) {
            Ok(_) => panic!("symbols are missing"),
            Err(error) => error,
        };
        assert!(error.is_unavailable(), "{error}");
        assert!(error.to_string().contains("gl"), "{error}");
    }
}
