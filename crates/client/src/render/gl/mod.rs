//! OpenGL backend (donor `src/render/gl/*`).

pub mod buffers;
pub mod depth_atlas;
pub mod fog;
pub mod fog_shader;
pub mod object_opacity;
pub mod output_gamma;
pub mod platform;
pub mod programs;
pub mod renderer;
pub mod shadow_shader;
pub mod textures;

// Donor `src/render/gl/*` passes raw GL enumerants; they are collected here so
// the backend modules share one definition instead of scattering literals.
pub const TEXTURE_2D: u32 = 0x0DE1;
pub const TEXTURE0: u32 = 0x84C0;
pub const ACTIVE_TEXTURE: u32 = 0x84E0;
pub const TEXTURE_BINDING_2D: u32 = 0x8069;
pub const MAX_TEXTURE_COORDS: u32 = 0x8872;
pub const MAX_TEXTURE_IMAGE_UNITS: u32 = 0x8871;
pub const MAX_TEXTURE_SIZE: u32 = 0x0D33;
pub const RGBA8: i32 = 0x8058;
pub const RGB8: i32 = 0x8051;
pub const RGB565: i32 = 0x8D62;
pub const LUMINANCE8: i32 = 0x8040;
pub const DEPTH_COMPONENT32F: i32 = 0x8CAC;
pub const DEPTH_COMPONENT16: i32 = 0x81A5;
pub const DEPTH_COMPONENT24: i32 = 0x81A6;
pub const DEPTH_COMPONENT32: i32 = 0x81A7;
pub const DEPTH24_STENCIL8: i32 = 0x88F0;
pub const DEPTH32F_STENCIL8: i32 = 0x8CAD;
pub const RGBA: u32 = 0x1908;
pub const LUMINANCE: u32 = 0x1909;
pub const DEPTH_COMPONENT: u32 = 0x1902;
pub const DEPTH_STENCIL: u32 = 0x84F9;
pub const STENCIL_INDEX: u32 = 0x1901;
pub const UNSIGNED_BYTE: u32 = 0x1401;
pub const UNSIGNED_INT: u32 = 0x1405;
pub const FLOAT: u32 = 0x1406;
pub const UNSIGNED_INT_24_8: u32 = 0x84FA;
pub const FLOAT_32_UNSIGNED_INT_24_8_REV: u32 = 0x8DAD;
pub const TEXTURE_MIN_FILTER: u32 = 0x2801;
pub const TEXTURE_MAG_FILTER: u32 = 0x2800;
pub const TEXTURE_WRAP_S: u32 = 0x2802;
pub const TEXTURE_WRAP_T: u32 = 0x2803;
pub const TEXTURE_MAX_LEVEL: u32 = 0x813D;
pub const TEXTURE_BORDER_COLOR: u32 = 0x1004;
pub const NEAREST: i32 = 0x2600;
pub const LINEAR: i32 = 0x2601;
pub const NEAREST_MIPMAP_NEAREST: i32 = 0x2700;
pub const LINEAR_MIPMAP_NEAREST: i32 = 0x2701;
pub const NEAREST_MIPMAP_LINEAR: i32 = 0x2702;
pub const LINEAR_MIPMAP_LINEAR: i32 = 0x2703;
pub const REPEAT: i32 = 0x2901;
pub const CLAMP_TO_EDGE: i32 = 0x812F;
pub const CLAMP: i32 = 0x2900;
pub const VIEWPORT: u32 = 0x0BA2;
pub const SCISSOR_BOX: u32 = 0x0C10;
pub const COLOR_BUFFER_BIT: u32 = 0x4000;
pub const DEPTH_BUFFER_BIT: u32 = 0x0100;
pub const STENCIL_BUFFER_BIT: u32 = 0x0400;
pub const DEPTH_TEST: u32 = 0x0B71;
pub const CULL_FACE_CAP: u32 = 0x0B44;
pub const CULL_FACE_MODE: u32 = 0x0B45;
pub const BLEND: u32 = 0x0BE2;
pub const STENCIL_TEST: u32 = 0x0B90;
pub const ALPHA_TEST: u32 = 0x0BC0;
pub const CLIP_PLANE0: u32 = 0x3000;
pub const SCISSOR_TEST: u32 = 0x0C11;
pub const POLYGON_OFFSET_FILL: u32 = 0x8037;
pub const POLYGON_OFFSET_FACTOR: u32 = 0x8038;
pub const POLYGON_OFFSET_UNITS: u32 = 0x2A00;
pub const DITHER: u32 = 0x0BD0;
pub const LIGHTING: u32 = 0x0B50;
pub const COLOR_MATERIAL: u32 = 0x0B60;
pub const DEPTH_WRITEMASK: u32 = 0x0B72;
pub const DEPTH_FUNC_PNAME: u32 = 0x0B74;
pub const COLOR_WRITEMASK: u32 = 0x0C23;
pub const COLOR_CLEAR_VALUE: u32 = 0x0B00;
pub const DEPTH_RANGE: u32 = 0x0B70;
pub const DEPTH_CLEAR_VALUE: u32 = 0x0B73;
pub const POLYGON_MODE_PNAME: u32 = 0x0B40;
pub const BLEND_SRC: u32 = 0x0BE1;
pub const BLEND_DST: u32 = 0x0BE0;
pub const FRONT: u32 = 0x0404;
pub const BACK: u32 = 0x0405;
pub const BACK_LEFT: u32 = 0x0402;
pub const BACK_RIGHT: u32 = 0x0403;
pub const FRONT_AND_BACK: u32 = 0x0408;
pub const FILL: u32 = 0x1B02;
pub const CCW: u32 = 0x0901;
pub const SMOOTH: u32 = 0x1D01;
pub const LESS: u32 = 0x0201;
pub const EQUAL: u32 = 0x0202;
pub const LEQUAL: u32 = 0x0203;
pub const ALWAYS: u32 = 0x0207;
pub const NOTEQUAL: u32 = 0x0205;
pub const KEEP: u32 = 0x1E00;
pub const INCR: u32 = 0x1E02;
pub const DECR: u32 = 0x1E03;
pub const ZERO: u32 = 0;
pub const ONE: u32 = 1;
pub const SRC_COLOR: u32 = 0x0300;
pub const ONE_MINUS_SRC_COLOR: u32 = 0x0301;
pub const SRC_ALPHA: u32 = 0x0302;
pub const ONE_MINUS_SRC_ALPHA: u32 = 0x0303;
pub const DST_ALPHA: u32 = 0x0304;
pub const ONE_MINUS_DST_ALPHA: u32 = 0x0305;
pub const DST_COLOR: u32 = 0x0306;
pub const ONE_MINUS_DST_COLOR: u32 = 0x0307;
pub const SRC_ALPHA_SATURATE: u32 = 0x0308;
pub const LINES: u32 = 1;
pub const TRIANGLES: u32 = 4;
pub const TRIANGLE_STRIP: u32 = 5;
pub const QUADS: u32 = 7;
pub const PROJECTION: u32 = 0x1701;
pub const MODELVIEW: u32 = 0x1700;
pub const VERTEX_ARRAY: u32 = 0x8074;
pub const NORMAL_ARRAY: u32 = 0x8075;
pub const COLOR_ARRAY: u32 = 0x8076;
pub const INDEX_ARRAY: u32 = 0x8077;
pub const TEXTURE_COORD_ARRAY: u32 = 0x8078;
pub const EDGE_FLAG_ARRAY: u32 = 0x8079;
pub const VERTEX_ARRAY_POINTER: u32 = 0x8457;
pub const COLOR_ARRAY_POINTER: u32 = 0x845E;
pub const CLIENT_VERTEX_ARRAY_BIT: u32 = 2;
pub const CLIENT_ATTRIB_STACK_DEPTH: u32 = 0x0BB1;
pub const MAX_CLIENT_ATTRIB_STACK_DEPTH: u32 = 0x0D3B;
pub const ARRAY_BUFFER: u32 = 0x8892;
pub const ELEMENT_ARRAY_BUFFER: u32 = 0x8893;
pub const PRIMITIVE_RESTART: u32 = 0x8F9D;
pub const VERTEX_SHADER: u32 = 0x8B31;
pub const FRAGMENT_SHADER: u32 = 0x8B30;
pub const COMPILE_STATUS: u32 = 0x8B81;
pub const LINK_STATUS: u32 = 0x8B82;
pub const CURRENT_PROGRAM: u32 = 0x8B8D;
pub const FRAMEBUFFER: u32 = 0x8D40;
pub const READ_FRAMEBUFFER: u32 = 0x8CA8;
pub const DRAW_FRAMEBUFFER: u32 = 0x8CA9;
pub const FRAMEBUFFER_BINDING: u32 = 0x8CA6;
pub const READ_FRAMEBUFFER_BINDING: u32 = 0x8CAA;
pub const DRAW_BUFFER_PNAME: u32 = 0x0C01;
pub const READ_BUFFER_PNAME: u32 = 0x0C02;
pub const COLOR_ATTACHMENT0: u32 = 0x8CE0;
pub const DEPTH_ATTACHMENT: u32 = 0x8D00;
pub const DEPTH_STENCIL_ATTACHMENT: u32 = 0x821A;
pub const FRAMEBUFFER_COMPLETE: u32 = 0x8CD5;
pub const FRAMEBUFFER_ATTACHMENT_COMPONENT_TYPE: u32 = 0x8211;
pub const BACK_LEFT_ATTACHMENT: u32 = 0x1801;
pub const SAMPLE_BUFFERS: u32 = 0x80A9;
pub const STENCIL_BITS: u32 = 0x0D57;
pub const DEPTH_BITS: u32 = 0x0D56;
pub const RED_BITS: u32 = 0x0D52;
pub const GREEN_BITS: u32 = 0x0D53;
pub const BLUE_BITS: u32 = 0x0D54;
pub const ALPHA_BITS: u32 = 0x0D55;
pub const STEREO: u32 = 0x0C33;
pub const VENDOR: u32 = 0x1F00;
pub const RENDERER: u32 = 0x1F01;
pub const VERSION: u32 = 0x1F02;
pub const SHADING_LANGUAGE_VERSION: u32 = 0x8B8C;
pub const PIXEL_PACK_BUFFER_BINDING: u32 = 0x88ED;
pub const PIXEL_UNPACK_BUFFER_BINDING: u32 = 0x88EF;
pub const PACK_ALIGNMENT: u32 = 0x0D05;
pub const PACK_ROW_LENGTH: u32 = 0x0D02;
pub const PACK_SKIP_PIXELS: u32 = 0x0D03;
pub const PACK_SKIP_ROWS: u32 = 0x0D04;
pub const PACK_SWAP_BYTES: u32 = 0x0D00;
pub const UNPACK_ALIGNMENT: u32 = 0x0CF5;
pub const UNPACK_ROW_LENGTH: u32 = 0x0CF2;
pub const UNPACK_SKIP_PIXELS: u32 = 0x0CF3;
pub const UNPACK_SKIP_ROWS: u32 = 0x0CF4;
pub const UNPACK_SWAP_BYTES: u32 = 0x0CF0;
pub const NO_BUFFER: u32 = 0;

/// Color/depth/stencil precision shared by the opacity and gamma scratch targets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FramebufferPrecision {
    pub depth_bits: i32,
    pub stencil_bits: i32,
    pub color_bits: i32,
    pub alpha_bits: i32,
}

/// Object-safe GL surface consumed by the backend. Every method maps to one
/// typed entry in `qa_platform`'s GL tables; vertex submission uses client
/// arrays exactly like the donor, so there is no buffer-upload or draw-arrays
/// entry (neither has a platform symbol nor a donor call site).
pub trait GlContext {
    fn viewport(&mut self, x: i32, y: i32, width: i32, height: i32);
    fn scissor(&mut self, x: i32, y: i32, width: i32, height: i32);
    fn clear_color(&mut self, red: f32, green: f32, blue: f32, alpha: f32);
    fn clear_depth(&mut self, depth: f64);
    fn clear_stencil(&mut self, stencil: i32);
    fn clear(&mut self, mask: u32);
    fn enable(&mut self, cap: u32);
    fn disable(&mut self, cap: u32);
    fn is_enabled(&mut self, cap: u32) -> bool;
    fn get_integerv(&mut self, pname: u32, out: &mut [i32]);
    fn get_floatv(&mut self, pname: u32, out: &mut [f32]);
    fn get_string(&mut self, name: u32) -> String;
    fn blend_func(&mut self, src: u32, dst: u32);
    fn depth_func(&mut self, func: u32);
    fn depth_mask(&mut self, flag: bool);
    fn color_mask(&mut self, red: bool, green: bool, blue: bool, alpha: bool);
    fn stencil_func(&mut self, func: u32, reference: i32, mask: u32);
    fn stencil_op(&mut self, fail: u32, zfail: u32, zpass: u32);
    fn stencil_mask(&mut self, mask: u32);
    fn depth_range(&mut self, near: f64, far: f64);
    fn polygon_mode(&mut self, face: u32, mode: u32);
    fn shade_model(&mut self, model: u32);
    fn polygon_offset(&mut self, factor: f32, units: f32);
    fn line_width(&mut self, width: f32);
    fn cull_face(&mut self, mode: u32);
    fn front_face(&mut self, mode: u32);
    fn clip_plane(&mut self, plane: u32, equation: &[f64; 4]);
    fn matrix_mode(&mut self, mode: u32);
    fn load_identity(&mut self);
    #[allow(clippy::too_many_arguments)]
    fn ortho(&mut self, left: f64, right: f64, bottom: f64, top: f64, near: f64, far: f64);
    fn begin(&mut self, mode: u32);
    fn end(&mut self);
    fn color_3f(&mut self, red: f32, green: f32, blue: f32);
    fn color_4f(&mut self, red: f32, green: f32, blue: f32, alpha: f32);
    fn tex_coord_2f(&mut self, s: f32, t: f32);
    fn vertex_2f(&mut self, x: f32, y: f32);
    fn vertex_4f(&mut self, x: f32, y: f32, z: f32, w: f32);
    fn enable_client_state(&mut self, array: u32);
    fn disable_client_state(&mut self, array: u32);
    fn push_client_attrib(&mut self, mask: u32);
    fn pop_client_attrib(&mut self);
    fn vertex_pointer(&mut self, size: i32, stride: i32, data: &[f32]);
    fn color_pointer(&mut self, size: i32, stride: i32, data: &[f32]);
    fn tex_coord_pointer(&mut self, size: i32, stride: i32, data: &[f32]);
    fn draw_elements(&mut self, mode: u32, indices: &[u32]);
    fn gen_buffers(&mut self, count: i32) -> Vec<u32>;
    fn delete_buffers(&mut self, names: &[u32]);
    fn bind_buffer(&mut self, target: u32, name: u32);
    fn active_texture(&mut self, unit: u32);
    fn client_active_texture(&mut self, unit: u32);
    fn gen_textures(&mut self, count: i32) -> Vec<u32>;
    fn delete_textures(&mut self, names: &[u32]);
    fn bind_texture(&mut self, target: u32, name: u32);
    fn tex_parameteri(&mut self, target: u32, pname: u32, value: i32);
    fn tex_parameterfv(&mut self, target: u32, pname: u32, values: &[f32; 4]);
    #[allow(clippy::too_many_arguments)]
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
    );
    #[allow(clippy::too_many_arguments)]
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
    );
    #[allow(clippy::too_many_arguments)]
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
    );
    #[allow(clippy::too_many_arguments)]
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
    );
    #[allow(clippy::too_many_arguments)]
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
    );
    #[allow(clippy::too_many_arguments)]
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
    );
    fn get_tex_image_floats(&mut self, target: u32, level: i32, format: u32, ty: u32, dest: &mut [f32]);
    fn create_shader(&mut self, ty: u32) -> u32;
    fn shader_source(&mut self, shader: u32, source: &str);
    fn compile_shader(&mut self, shader: u32);
    fn get_shaderiv(&mut self, shader: u32, pname: u32) -> i32;
    fn get_shader_info_log(&mut self, shader: u32, buf: &mut [u8]) -> usize;
    fn delete_shader(&mut self, shader: u32);
    fn create_program(&mut self) -> u32;
    fn attach_shader(&mut self, program: u32, shader: u32);
    fn link_program(&mut self, program: u32);
    fn get_programiv(&mut self, program: u32, pname: u32) -> i32;
    fn get_program_info_log(&mut self, program: u32, buf: &mut [u8]) -> usize;
    fn delete_program(&mut self, program: u32);
    fn use_program(&mut self, program: u32);
    fn get_uniform_location(&mut self, program: u32, name: &str) -> i32;
    fn uniform_1i(&mut self, location: i32, value: i32);
    fn uniform_1f(&mut self, location: i32, value: f32);
    fn uniform_3f(&mut self, location: i32, x: f32, y: f32, z: f32);
    fn uniform_4f(&mut self, location: i32, x: f32, y: f32, z: f32, w: f32);
    fn uniform_matrix_4fv(&mut self, location: i32, value: &[f32; 16]);
    fn gen_framebuffers(&mut self, count: i32) -> Vec<u32>;
    fn delete_framebuffers(&mut self, names: &[u32]);
    fn bind_framebuffer(&mut self, target: u32, name: u32);
    fn framebuffer_texture_2d(&mut self, target: u32, attachment: u32, textarget: u32, texture: u32, level: i32);
    fn check_framebuffer_status(&mut self, target: u32) -> u32;
    fn blit_framebuffer(&mut self, src: [i32; 4], dst: [i32; 4], mask: u32, filter: u32);
    fn get_framebuffer_attachment_parameteriv(&mut self, target: u32, attachment: u32, pname: u32) -> i32;
    fn read_buffer(&mut self, mode: u32);
    fn draw_buffer(&mut self, mode: u32);
    fn pixel_storei(&mut self, pname: u32, value: i32);
    #[allow(clippy::too_many_arguments)]
    fn read_pixels_bytes(&mut self, x: i32, y: i32, width: i32, height: i32, format: u32, ty: u32, dest: &mut [u8]);
    #[allow(clippy::too_many_arguments)]
    fn read_pixels_floats(&mut self, x: i32, y: i32, width: i32, height: i32, format: u32, ty: u32, dest: &mut [f32]);
    fn finish(&mut self);
    fn get_error(&mut self) -> u32;
    fn swap_buffers(&mut self);
}

/// One recorded GL call. Queries are recorded like any other call so tests can
/// assert save/restore discipline; use [`FakeGlContext::calls_matching`] to filter.
#[derive(Debug, Clone, PartialEq)]
pub enum GlCall {
    Viewport {
        x: i32,
        y: i32,
        width: i32,
        height: i32,
    },
    Scissor {
        x: i32,
        y: i32,
        width: i32,
        height: i32,
    },
    ClearColor {
        red: f32,
        green: f32,
        blue: f32,
        alpha: f32,
    },
    ClearDepth {
        depth: f64,
    },
    ClearStencil {
        stencil: i32,
    },
    Clear {
        mask: u32,
    },
    Enable {
        cap: u32,
    },
    Disable {
        cap: u32,
    },
    IsEnabled {
        cap: u32,
    },
    GetIntegerv {
        pname: u32,
        count: usize,
    },
    GetFloatv {
        pname: u32,
        count: usize,
    },
    GetString {
        name: u32,
    },
    BlendFunc {
        src: u32,
        dst: u32,
    },
    DepthFunc {
        func: u32,
    },
    DepthMask {
        flag: bool,
    },
    ColorMask {
        red: bool,
        green: bool,
        blue: bool,
        alpha: bool,
    },
    StencilFunc {
        func: u32,
        reference: i32,
        mask: u32,
    },
    StencilOp {
        fail: u32,
        zfail: u32,
        zpass: u32,
    },
    StencilMask {
        mask: u32,
    },
    DepthRange {
        near: f64,
        far: f64,
    },
    PolygonMode {
        face: u32,
        mode: u32,
    },
    ShadeModel {
        model: u32,
    },
    PolygonOffset {
        factor: f32,
        units: f32,
    },
    LineWidth {
        width: f32,
    },
    CullFace {
        mode: u32,
    },
    FrontFace {
        mode: u32,
    },
    ClipPlane {
        plane: u32,
        equation: [f64; 4],
    },
    MatrixMode {
        mode: u32,
    },
    LoadIdentity,
    Ortho {
        left: f64,
        right: f64,
        bottom: f64,
        top: f64,
        near: f64,
        far: f64,
    },
    Begin {
        mode: u32,
    },
    End,
    Color3f {
        red: f32,
        green: f32,
        blue: f32,
    },
    Color4f {
        red: f32,
        green: f32,
        blue: f32,
        alpha: f32,
    },
    TexCoord2f {
        s: f32,
        t: f32,
    },
    Vertex2f {
        x: f32,
        y: f32,
    },
    Vertex4f {
        x: f32,
        y: f32,
        z: f32,
        w: f32,
    },
    EnableClientState {
        array: u32,
    },
    DisableClientState {
        array: u32,
    },
    PushClientAttrib {
        mask: u32,
    },
    PopClientAttrib,
    VertexPointer {
        size: i32,
        stride: i32,
        len: usize,
    },
    ColorPointer {
        size: i32,
        stride: i32,
        len: usize,
    },
    TexCoordPointer {
        size: i32,
        stride: i32,
        len: usize,
    },
    DrawElements {
        mode: u32,
        count: usize,
    },
    GenBuffers {
        count: i32,
        names: Vec<u32>,
    },
    DeleteBuffers {
        names: Vec<u32>,
    },
    BindBuffer {
        target: u32,
        name: u32,
    },
    ActiveTexture {
        unit: u32,
    },
    ClientActiveTexture {
        unit: u32,
    },
    GenTextures {
        count: i32,
        names: Vec<u32>,
    },
    DeleteTextures {
        names: Vec<u32>,
    },
    BindTexture {
        target: u32,
        name: u32,
    },
    TexParameteri {
        target: u32,
        pname: u32,
        value: i32,
    },
    TexParameterfv {
        target: u32,
        pname: u32,
        values: [f32; 4],
    },
    TexImage2D {
        target: u32,
        level: i32,
        internal: i32,
        width: i32,
        height: i32,
        format: u32,
        ty: u32,
        bytes: usize,
    },
    CopyTexImage2D {
        target: u32,
        level: i32,
        internal: i32,
        x: i32,
        y: i32,
        width: i32,
        height: i32,
    },
    TexSubImage2D {
        target: u32,
        level: i32,
        x: i32,
        y: i32,
        width: i32,
        height: i32,
        format: u32,
        ty: u32,
        bytes: usize,
    },
    GetTexImage {
        target: u32,
        level: i32,
        format: u32,
        ty: u32,
        len: usize,
    },
    CreateShader {
        ty: u32,
        name: u32,
    },
    ShaderSource {
        shader: u32,
        len: usize,
    },
    CompileShader {
        shader: u32,
    },
    GetShaderiv {
        shader: u32,
        pname: u32,
    },
    GetShaderInfoLog {
        shader: u32,
    },
    DeleteShader {
        shader: u32,
    },
    CreateProgram {
        name: u32,
    },
    AttachShader {
        program: u32,
        shader: u32,
    },
    LinkProgram {
        program: u32,
    },
    GetProgramiv {
        program: u32,
        pname: u32,
    },
    GetProgramInfoLog {
        program: u32,
    },
    DeleteProgram {
        program: u32,
    },
    UseProgram {
        program: u32,
    },
    GetUniformLocation {
        program: u32,
        name: String,
    },
    Uniform1i {
        location: i32,
        value: i32,
    },
    Uniform1f {
        location: i32,
        value: f32,
    },
    Uniform3f {
        location: i32,
        x: f32,
        y: f32,
        z: f32,
    },
    Uniform4f {
        location: i32,
        x: f32,
        y: f32,
        z: f32,
        w: f32,
    },
    UniformMatrix4fv {
        location: i32,
        value: [f32; 16],
    },
    GenFramebuffers {
        count: i32,
        names: Vec<u32>,
    },
    DeleteFramebuffers {
        names: Vec<u32>,
    },
    BindFramebuffer {
        target: u32,
        name: u32,
    },
    FramebufferTexture2D {
        target: u32,
        attachment: u32,
        textarget: u32,
        texture: u32,
        level: i32,
    },
    CheckFramebufferStatus {
        target: u32,
    },
    BlitFramebuffer {
        src: [i32; 4],
        dst: [i32; 4],
        mask: u32,
        filter: u32,
    },
    GetFramebufferAttachmentParameteriv {
        target: u32,
        attachment: u32,
        pname: u32,
    },
    ReadBuffer {
        mode: u32,
    },
    DrawBuffer {
        mode: u32,
    },
    PixelStorei {
        pname: u32,
        value: i32,
    },
    ReadPixels {
        x: i32,
        y: i32,
        width: i32,
        height: i32,
        format: u32,
        ty: u32,
        len: usize,
    },
    Finish,
    GetError,
    SwapBuffers,
}

/// Test double recording every call with programmable query results.
#[derive(Debug)]
pub struct FakeGlContext {
    /// Recorded calls in order.
    pub log: Vec<GlCall>,
    next_name: u32,
    int_state: std::collections::HashMap<u32, Vec<i32>>,
    float_state: std::collections::HashMap<u32, Vec<f32>>,
    enabled: std::collections::HashSet<u32>,
    strings: std::collections::HashMap<u32, String>,
    uniform_locations: std::collections::HashMap<(u32, String), i32>,
    next_uniform: i32,
    shader_status: i32,
    program_status: i32,
    framebuffer_status: u32,
    attachment_component: i32,
    error: u32,
    read_bytes: Vec<u8>,
    read_floats: Vec<f32>,
    tex_floats: Vec<f32>,
}

impl FakeGlContext {
    #[must_use]
    pub fn new() -> Self {
        let mut fake = Self {
            log: Vec::new(),
            next_name: 1,
            int_state: std::collections::HashMap::new(),
            float_state: std::collections::HashMap::new(),
            enabled: std::collections::HashSet::new(),
            strings: std::collections::HashMap::new(),
            uniform_locations: std::collections::HashMap::new(),
            next_uniform: 0,
            shader_status: 1,
            program_status: 1,
            framebuffer_status: FRAMEBUFFER_COMPLETE,
            attachment_component: UNSIGNED_BYTE as i32,
            error: 0,
            read_bytes: Vec::new(),
            read_floats: Vec::new(),
            tex_floats: Vec::new(),
        };
        fake.set_int(STENCIL_BITS, vec![8]);
        fake.set_int(DEPTH_BITS, vec![24]);
        fake.set_int(RED_BITS, vec![8]);
        fake.set_int(GREEN_BITS, vec![8]);
        fake.set_int(BLUE_BITS, vec![8]);
        fake.set_int(ALPHA_BITS, vec![8]);
        fake.set_int(MAX_TEXTURE_SIZE, vec![1024]);
        fake.set_int(MAX_TEXTURE_COORDS, vec![8]);
        fake.set_int(MAX_TEXTURE_IMAGE_UNITS, vec![8]);
        fake.set_int(STEREO, vec![0]);
        fake.set_int(CURRENT_PROGRAM, vec![0]);
        fake.set_int(ACTIVE_TEXTURE, vec![TEXTURE0 as i32]);
        fake.set_int(VIEWPORT, vec![0, 0, 640, 480]);
        fake.set_int(SCISSOR_BOX, vec![0, 0, 640, 480]);
        fake.set_int(DEPTH_WRITEMASK, vec![1]);
        fake.set_int(BLEND_SRC, vec![SRC_ALPHA as i32]);
        fake.set_int(BLEND_DST, vec![ONE_MINUS_SRC_ALPHA as i32]);
        fake.set_int(TEXTURE_BINDING_2D, vec![0]);
        fake.set_int(FRAMEBUFFER_BINDING, vec![0]);
        fake.set_int(READ_FRAMEBUFFER_BINDING, vec![0]);
        fake.set_int(DRAW_BUFFER_PNAME, vec![BACK as i32]);
        fake.set_int(READ_BUFFER_PNAME, vec![BACK as i32]);
        fake.set_int(COLOR_WRITEMASK, vec![1, 1, 1, 1]);
        fake.set_int(DEPTH_FUNC_PNAME, vec![LEQUAL as i32]);
        fake.set_int(CULL_FACE_MODE, vec![BACK as i32]);
        fake.set_int(POLYGON_MODE_PNAME, vec![FILL as i32, FILL as i32]);
        fake.set_int(SAMPLE_BUFFERS, vec![0]);
        fake.set_int(CLIENT_ATTRIB_STACK_DEPTH, vec![0]);
        fake.set_int(MAX_CLIENT_ATTRIB_STACK_DEPTH, vec![16]);
        fake.set_int(PIXEL_PACK_BUFFER_BINDING, vec![0]);
        fake.set_int(PIXEL_UNPACK_BUFFER_BINDING, vec![0]);
        fake.set_float(COLOR_CLEAR_VALUE, vec![0.0, 0.0, 0.0, 0.0]);
        fake.set_float(DEPTH_RANGE, vec![0.0, 1.0]);
        fake.set_float(DEPTH_CLEAR_VALUE, vec![1.0]);
        fake.set_float(POLYGON_OFFSET_FACTOR, vec![0.0]);
        fake.set_float(POLYGON_OFFSET_UNITS, vec![0.0]);
        fake.strings.insert(VENDOR, "Fake".to_string());
        fake.strings.insert(RENDERER, "FakeGL".to_string());
        fake.strings.insert(VERSION, "2.1 Fake".to_string());
        fake.strings.insert(SHADING_LANGUAGE_VERSION, "1.20 Fake".to_string());
        fake
    }

    pub fn set_int(&mut self, pname: u32, values: Vec<i32>) {
        self.int_state.insert(pname, values);
    }

    pub fn set_float(&mut self, pname: u32, values: Vec<f32>) {
        self.float_state.insert(pname, values);
    }

    pub fn set_enabled(&mut self, cap: u32, enabled: bool) {
        if enabled {
            self.enabled.insert(cap);
        } else {
            self.enabled.remove(&cap);
        }
    }

    pub fn set_error(&mut self, error: u32) {
        self.error = error;
    }

    pub fn set_read_bytes(&mut self, bytes: Vec<u8>) {
        self.read_bytes = bytes;
    }

    pub fn set_read_floats(&mut self, floats: Vec<f32>) {
        self.read_floats = floats;
    }

    pub fn set_tex_floats(&mut self, floats: Vec<f32>) {
        self.tex_floats = floats;
    }

    pub fn set_framebuffer_status(&mut self, status: u32) {
        self.framebuffer_status = status;
    }

    pub fn set_attachment_component(&mut self, component: i32) {
        self.attachment_component = component;
    }

    pub fn set_shader_status(&mut self, status: i32) {
        self.shader_status = status;
    }

    pub fn set_program_status(&mut self, status: i32) {
        self.program_status = status;
    }

    fn alloc(&mut self, count: i32) -> Vec<u32> {
        let mut names = Vec::new();
        for _ in 0..count.max(0) {
            names.push(self.next_name);
            self.next_name += 1;
        }
        names
    }

    /// Calls matching `predicate`, in order.
    #[must_use]
    pub fn calls_matching(&self, predicate: impl Fn(&GlCall) -> bool) -> Vec<&GlCall> {
        self.log.iter().filter(|call| predicate(call)).collect()
    }

    /// Number of calls matching `predicate`.
    #[must_use]
    pub fn count_matching(&self, predicate: impl Fn(&GlCall) -> bool) -> usize {
        self.log.iter().filter(|call| predicate(call)).count()
    }

    /// Assert at least one call matches `predicate`.
    pub fn assert_contains(&self, what: &str, predicate: impl Fn(&GlCall) -> bool) {
        assert!(
            self.log.iter().any(predicate),
            "{what} not found in {} GL calls",
            self.log.len()
        );
    }

    /// Assert no call matches `predicate`.
    pub fn assert_absent(&self, what: &str, predicate: impl Fn(&GlCall) -> bool) {
        assert!(!self.log.iter().any(predicate), "{what} unexpectedly present in GL log");
    }

    /// Clear the recorded log, keeping programmed state.
    pub fn clear_log(&mut self) {
        self.log.clear();
    }

    /// Uniform location the fake will hand out for `program`/`name`.
    #[must_use]
    pub fn uniform_location(&self, program: u32, name: &str) -> Option<i32> {
        self.uniform_locations.get(&(program, name.to_string())).copied()
    }
}

impl Default for FakeGlContext {
    fn default() -> Self {
        Self::new()
    }
}

impl GlContext for FakeGlContext {
    fn viewport(&mut self, x: i32, y: i32, width: i32, height: i32) {
        self.log.push(GlCall::Viewport { x, y, width, height });
    }
    fn scissor(&mut self, x: i32, y: i32, width: i32, height: i32) {
        self.log.push(GlCall::Scissor { x, y, width, height });
    }
    fn clear_color(&mut self, red: f32, green: f32, blue: f32, alpha: f32) {
        self.log.push(GlCall::ClearColor {
            red,
            green,
            blue,
            alpha,
        });
    }
    fn clear_depth(&mut self, depth: f64) {
        self.log.push(GlCall::ClearDepth { depth });
    }
    fn clear_stencil(&mut self, stencil: i32) {
        self.log.push(GlCall::ClearStencil { stencil });
    }
    fn clear(&mut self, mask: u32) {
        self.log.push(GlCall::Clear { mask });
    }
    fn enable(&mut self, cap: u32) {
        self.enabled.insert(cap);
        self.log.push(GlCall::Enable { cap });
    }
    fn disable(&mut self, cap: u32) {
        self.enabled.remove(&cap);
        self.log.push(GlCall::Disable { cap });
    }
    fn is_enabled(&mut self, cap: u32) -> bool {
        self.log.push(GlCall::IsEnabled { cap });
        self.enabled.contains(&cap)
    }
    fn get_integerv(&mut self, pname: u32, out: &mut [i32]) {
        self.log.push(GlCall::GetIntegerv {
            pname,
            count: out.len(),
        });
        let values = self.int_state.get(&pname).cloned().unwrap_or_default();
        for (slot, value) in out.iter_mut().enumerate() {
            *value = values.get(slot).copied().unwrap_or(0);
        }
    }
    fn get_floatv(&mut self, pname: u32, out: &mut [f32]) {
        self.log.push(GlCall::GetFloatv {
            pname,
            count: out.len(),
        });
        let values = self.float_state.get(&pname).cloned().unwrap_or_default();
        for (slot, value) in out.iter_mut().enumerate() {
            *value = values.get(slot).copied().unwrap_or(0.0);
        }
    }
    fn get_string(&mut self, name: u32) -> String {
        self.log.push(GlCall::GetString { name });
        self.strings.get(&name).cloned().unwrap_or_default()
    }
    fn blend_func(&mut self, src: u32, dst: u32) {
        self.log.push(GlCall::BlendFunc { src, dst });
    }
    fn depth_func(&mut self, func: u32) {
        self.log.push(GlCall::DepthFunc { func });
    }
    fn depth_mask(&mut self, flag: bool) {
        self.log.push(GlCall::DepthMask { flag });
    }
    fn color_mask(&mut self, red: bool, green: bool, blue: bool, alpha: bool) {
        self.log.push(GlCall::ColorMask {
            red,
            green,
            blue,
            alpha,
        });
    }
    fn stencil_func(&mut self, func: u32, reference: i32, mask: u32) {
        self.log.push(GlCall::StencilFunc { func, reference, mask });
    }
    fn stencil_op(&mut self, fail: u32, zfail: u32, zpass: u32) {
        self.log.push(GlCall::StencilOp { fail, zfail, zpass });
    }
    fn stencil_mask(&mut self, mask: u32) {
        self.log.push(GlCall::StencilMask { mask });
    }
    fn depth_range(&mut self, near: f64, far: f64) {
        self.log.push(GlCall::DepthRange { near, far });
    }
    fn polygon_mode(&mut self, face: u32, mode: u32) {
        self.log.push(GlCall::PolygonMode { face, mode });
    }
    fn shade_model(&mut self, model: u32) {
        self.log.push(GlCall::ShadeModel { model });
    }
    fn polygon_offset(&mut self, factor: f32, units: f32) {
        self.log.push(GlCall::PolygonOffset { factor, units });
    }
    fn line_width(&mut self, width: f32) {
        self.log.push(GlCall::LineWidth { width });
    }
    fn cull_face(&mut self, mode: u32) {
        self.log.push(GlCall::CullFace { mode });
    }
    fn front_face(&mut self, mode: u32) {
        self.log.push(GlCall::FrontFace { mode });
    }
    fn clip_plane(&mut self, plane: u32, equation: &[f64; 4]) {
        self.log.push(GlCall::ClipPlane {
            plane,
            equation: *equation,
        });
    }
    fn matrix_mode(&mut self, mode: u32) {
        self.log.push(GlCall::MatrixMode { mode });
    }
    fn load_identity(&mut self) {
        self.log.push(GlCall::LoadIdentity);
    }
    fn ortho(&mut self, left: f64, right: f64, bottom: f64, top: f64, near: f64, far: f64) {
        self.log.push(GlCall::Ortho {
            left,
            right,
            bottom,
            top,
            near,
            far,
        });
    }
    fn begin(&mut self, mode: u32) {
        self.log.push(GlCall::Begin { mode });
    }
    fn end(&mut self) {
        self.log.push(GlCall::End);
    }
    fn color_3f(&mut self, red: f32, green: f32, blue: f32) {
        self.log.push(GlCall::Color3f { red, green, blue });
    }
    fn color_4f(&mut self, red: f32, green: f32, blue: f32, alpha: f32) {
        self.log.push(GlCall::Color4f {
            red,
            green,
            blue,
            alpha,
        });
    }
    fn tex_coord_2f(&mut self, s: f32, t: f32) {
        self.log.push(GlCall::TexCoord2f { s, t });
    }
    fn vertex_2f(&mut self, x: f32, y: f32) {
        self.log.push(GlCall::Vertex2f { x, y });
    }
    fn vertex_4f(&mut self, x: f32, y: f32, z: f32, w: f32) {
        self.log.push(GlCall::Vertex4f { x, y, z, w });
    }
    fn enable_client_state(&mut self, array: u32) {
        self.log.push(GlCall::EnableClientState { array });
    }
    fn disable_client_state(&mut self, array: u32) {
        self.log.push(GlCall::DisableClientState { array });
    }
    fn push_client_attrib(&mut self, mask: u32) {
        self.log.push(GlCall::PushClientAttrib { mask });
    }
    fn pop_client_attrib(&mut self) {
        self.log.push(GlCall::PopClientAttrib);
    }
    fn vertex_pointer(&mut self, size: i32, stride: i32, data: &[f32]) {
        self.log.push(GlCall::VertexPointer {
            size,
            stride,
            len: data.len(),
        });
    }
    fn color_pointer(&mut self, size: i32, stride: i32, data: &[f32]) {
        self.log.push(GlCall::ColorPointer {
            size,
            stride,
            len: data.len(),
        });
    }
    fn tex_coord_pointer(&mut self, size: i32, stride: i32, data: &[f32]) {
        self.log.push(GlCall::TexCoordPointer {
            size,
            stride,
            len: data.len(),
        });
    }
    fn draw_elements(&mut self, mode: u32, indices: &[u32]) {
        self.log.push(GlCall::DrawElements {
            mode,
            count: indices.len(),
        });
    }
    fn gen_buffers(&mut self, count: i32) -> Vec<u32> {
        let names = self.alloc(count);
        self.log.push(GlCall::GenBuffers {
            count,
            names: names.clone(),
        });
        names
    }
    fn delete_buffers(&mut self, names: &[u32]) {
        self.log.push(GlCall::DeleteBuffers { names: names.to_vec() });
    }
    fn bind_buffer(&mut self, target: u32, name: u32) {
        self.log.push(GlCall::BindBuffer { target, name });
    }
    fn active_texture(&mut self, unit: u32) {
        self.log.push(GlCall::ActiveTexture { unit });
    }
    fn client_active_texture(&mut self, unit: u32) {
        self.log.push(GlCall::ClientActiveTexture { unit });
    }
    fn gen_textures(&mut self, count: i32) -> Vec<u32> {
        let names = self.alloc(count);
        self.log.push(GlCall::GenTextures {
            count,
            names: names.clone(),
        });
        names
    }
    fn delete_textures(&mut self, names: &[u32]) {
        self.log.push(GlCall::DeleteTextures { names: names.to_vec() });
    }
    fn bind_texture(&mut self, target: u32, name: u32) {
        self.log.push(GlCall::BindTexture { target, name });
    }
    fn tex_parameteri(&mut self, target: u32, pname: u32, value: i32) {
        self.log.push(GlCall::TexParameteri { target, pname, value });
    }
    fn tex_parameterfv(&mut self, target: u32, pname: u32, values: &[f32; 4]) {
        self.log.push(GlCall::TexParameterfv {
            target,
            pname,
            values: *values,
        });
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
        let _ = border;
        self.log.push(GlCall::TexImage2D {
            target,
            level,
            internal,
            width,
            height,
            format,
            ty,
            bytes: 0,
        });
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
        let _ = border;
        self.log.push(GlCall::TexImage2D {
            target,
            level,
            internal,
            width,
            height,
            format,
            ty,
            bytes: pixels.len(),
        });
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
        let _ = border;
        self.log.push(GlCall::TexImage2D {
            target,
            level,
            internal,
            width,
            height,
            format,
            ty,
            bytes: pixels.len() * 4,
        });
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
        let _ = border;
        self.log.push(GlCall::CopyTexImage2D {
            target,
            level,
            internal,
            x,
            y,
            width,
            height,
        });
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
        self.log.push(GlCall::TexSubImage2D {
            target,
            level,
            x,
            y,
            width,
            height,
            format,
            ty,
            bytes: pixels.len(),
        });
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
        self.log.push(GlCall::TexSubImage2D {
            target,
            level,
            x,
            y,
            width,
            height,
            format,
            ty,
            bytes: pixels.len() * 4,
        });
    }
    fn get_tex_image_floats(&mut self, target: u32, level: i32, format: u32, ty: u32, dest: &mut [f32]) {
        self.log.push(GlCall::GetTexImage {
            target,
            level,
            format,
            ty,
            len: dest.len(),
        });
        for (slot, value) in dest.iter_mut().enumerate() {
            *value = self.tex_floats.get(slot).copied().unwrap_or(0.0);
        }
    }
    fn create_shader(&mut self, ty: u32) -> u32 {
        let names = self.alloc(1);
        let name = names[0];
        self.log.push(GlCall::CreateShader { ty, name });
        name
    }
    fn shader_source(&mut self, shader: u32, source: &str) {
        self.log.push(GlCall::ShaderSource {
            shader,
            len: source.len(),
        });
    }
    fn compile_shader(&mut self, shader: u32) {
        self.log.push(GlCall::CompileShader { shader });
    }
    fn get_shaderiv(&mut self, shader: u32, pname: u32) -> i32 {
        self.log.push(GlCall::GetShaderiv { shader, pname });
        self.shader_status
    }
    fn get_shader_info_log(&mut self, shader: u32, _buf: &mut [u8]) -> usize {
        self.log.push(GlCall::GetShaderInfoLog { shader });
        0
    }
    fn delete_shader(&mut self, shader: u32) {
        self.log.push(GlCall::DeleteShader { shader });
    }
    fn create_program(&mut self) -> u32 {
        let names = self.alloc(1);
        let name = names[0];
        self.log.push(GlCall::CreateProgram { name });
        name
    }
    fn attach_shader(&mut self, program: u32, shader: u32) {
        self.log.push(GlCall::AttachShader { program, shader });
    }
    fn link_program(&mut self, program: u32) {
        self.log.push(GlCall::LinkProgram { program });
    }
    fn get_programiv(&mut self, program: u32, pname: u32) -> i32 {
        self.log.push(GlCall::GetProgramiv { program, pname });
        self.program_status
    }
    fn get_program_info_log(&mut self, program: u32, _buf: &mut [u8]) -> usize {
        self.log.push(GlCall::GetProgramInfoLog { program });
        0
    }
    fn delete_program(&mut self, program: u32) {
        self.log.push(GlCall::DeleteProgram { program });
    }
    fn use_program(&mut self, program: u32) {
        self.log.push(GlCall::UseProgram { program });
    }
    fn get_uniform_location(&mut self, program: u32, name: &str) -> i32 {
        self.log.push(GlCall::GetUniformLocation {
            program,
            name: name.to_string(),
        });
        if let Some(location) = self.uniform_locations.get(&(program, name.to_string())) {
            return *location;
        }
        let location = self.next_uniform;
        self.next_uniform += 1;
        self.uniform_locations.insert((program, name.to_string()), location);
        location
    }
    fn uniform_1i(&mut self, location: i32, value: i32) {
        self.log.push(GlCall::Uniform1i { location, value });
    }
    fn uniform_1f(&mut self, location: i32, value: f32) {
        self.log.push(GlCall::Uniform1f { location, value });
    }
    fn uniform_3f(&mut self, location: i32, x: f32, y: f32, z: f32) {
        self.log.push(GlCall::Uniform3f { location, x, y, z });
    }
    fn uniform_4f(&mut self, location: i32, x: f32, y: f32, z: f32, w: f32) {
        self.log.push(GlCall::Uniform4f { location, x, y, z, w });
    }
    fn uniform_matrix_4fv(&mut self, location: i32, value: &[f32; 16]) {
        self.log.push(GlCall::UniformMatrix4fv {
            location,
            value: *value,
        });
    }
    fn gen_framebuffers(&mut self, count: i32) -> Vec<u32> {
        let names = self.alloc(count);
        self.log.push(GlCall::GenFramebuffers {
            count,
            names: names.clone(),
        });
        names
    }
    fn delete_framebuffers(&mut self, names: &[u32]) {
        self.log.push(GlCall::DeleteFramebuffers { names: names.to_vec() });
    }
    fn bind_framebuffer(&mut self, target: u32, name: u32) {
        self.log.push(GlCall::BindFramebuffer { target, name });
    }
    fn framebuffer_texture_2d(&mut self, target: u32, attachment: u32, textarget: u32, texture: u32, level: i32) {
        self.log.push(GlCall::FramebufferTexture2D {
            target,
            attachment,
            textarget,
            texture,
            level,
        });
    }
    fn check_framebuffer_status(&mut self, target: u32) -> u32 {
        self.log.push(GlCall::CheckFramebufferStatus { target });
        self.framebuffer_status
    }
    fn blit_framebuffer(&mut self, src: [i32; 4], dst: [i32; 4], mask: u32, filter: u32) {
        self.log.push(GlCall::BlitFramebuffer { src, dst, mask, filter });
    }
    fn get_framebuffer_attachment_parameteriv(&mut self, target: u32, attachment: u32, pname: u32) -> i32 {
        self.log.push(GlCall::GetFramebufferAttachmentParameteriv {
            target,
            attachment,
            pname,
        });
        self.attachment_component
    }
    fn read_buffer(&mut self, mode: u32) {
        self.log.push(GlCall::ReadBuffer { mode });
    }
    fn draw_buffer(&mut self, mode: u32) {
        self.log.push(GlCall::DrawBuffer { mode });
    }
    fn pixel_storei(&mut self, pname: u32, value: i32) {
        self.log.push(GlCall::PixelStorei { pname, value });
    }
    fn read_pixels_bytes(&mut self, x: i32, y: i32, width: i32, height: i32, format: u32, ty: u32, dest: &mut [u8]) {
        self.log.push(GlCall::ReadPixels {
            x,
            y,
            width,
            height,
            format,
            ty,
            len: dest.len(),
        });
        for (slot, value) in dest.iter_mut().enumerate() {
            *value = self.read_bytes.get(slot).copied().unwrap_or(0);
        }
    }
    fn read_pixels_floats(&mut self, x: i32, y: i32, width: i32, height: i32, format: u32, ty: u32, dest: &mut [f32]) {
        self.log.push(GlCall::ReadPixels {
            x,
            y,
            width,
            height,
            format,
            ty,
            len: dest.len(),
        });
        for (slot, value) in dest.iter_mut().enumerate() {
            *value = self.read_floats.get(slot).copied().unwrap_or(0.0);
        }
    }
    fn finish(&mut self) {
        self.log.push(GlCall::Finish);
    }
    fn get_error(&mut self) -> u32 {
        self.log.push(GlCall::GetError);
        self.error
    }
    fn swap_buffers(&mut self) {
        self.log.push(GlCall::SwapBuffers);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fake_records_calls_and_serves_programmed_state() {
        let mut gl = FakeGlContext::new();
        gl.viewport(0, 0, 64, 64);
        gl.enable(DEPTH_TEST);
        assert!(gl.is_enabled(DEPTH_TEST));
        assert!(!gl.is_enabled(BLEND));
        let mut out = [0; 1];
        gl.get_integerv(MAX_TEXTURE_SIZE, &mut out);
        assert_eq!(out, [1024]);
        let names = gl.gen_textures(2);
        assert_eq!(names.len(), 2);
        assert_ne!(names[0], 0);
        assert_ne!(names[0], names[1]);
        let first = gl.get_uniform_location(7, "u_fog_mode");
        let second = gl.get_uniform_location(7, "u_fog_mode");
        assert_eq!(first, second);
        assert_eq!(gl.uniform_location(7, "u_fog_mode"), Some(first));
        gl.assert_contains("viewport", |call| matches!(call, GlCall::Viewport { width: 64, .. }));
        gl.assert_absent("swap", |call| matches!(call, GlCall::SwapBuffers));
        assert_eq!(gl.count_matching(|call| matches!(call, GlCall::GetIntegerv { .. })), 1);
        gl.clear_log();
        assert!(gl.log.is_empty());
        gl.set_error(0x0500);
        assert_eq!(gl.get_error(), 0x0500);
    }

    #[test]
    fn fake_readback_fills_from_programmed_buffers() {
        let mut gl = FakeGlContext::new();
        gl.set_read_bytes(vec![9, 8, 7]);
        let mut dest = [0u8; 5];
        gl.read_pixels_bytes(0, 0, 2, 1, RGBA, UNSIGNED_BYTE, &mut dest);
        assert_eq!(dest, [9, 8, 7, 0, 0]);
        gl.set_read_floats(vec![0.5]);
        let mut depth = [0.0f32; 1];
        gl.read_pixels_floats(0, 0, 1, 1, DEPTH_COMPONENT, FLOAT, &mut depth);
        assert_eq!(depth, [0.5]);
        gl.set_tex_floats(vec![0.25, 0.75]);
        let mut tex = [0.0f32; 2];
        gl.get_tex_image_floats(TEXTURE_2D, 0, DEPTH_COMPONENT, FLOAT, &mut tex);
        assert_eq!(tex, [0.25, 0.75]);
        assert_eq!(gl.check_framebuffer_status(FRAMEBUFFER), FRAMEBUFFER_COMPLETE);
        assert_eq!(gl.get_string(VERSION), "2.1 Fake");
    }
}
