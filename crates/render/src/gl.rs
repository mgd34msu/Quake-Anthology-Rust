//! Cached GL 4.4 command consumer. SDL context ownership stays in platform.
//!
//! Projection and stage order follow qsrc Q3 tr_main.c and tr_backend.c.
use crate::BackendStats;
use crate::assets::{
    AlphaTest, Assets, CubeSkyParams, Cull, DepthFunc, Filter, MaterialId, MaterialSettings,
    Sampler, Sky, Stage, StageTexture, TcGen, TextureIntensity, Vertex, Wrap,
};
use crate::scene::{
    BlendPhase, Command, CommandList, Draw2d, DrawKind, Refdef, SceneEntity, Span, Viewport,
};
use crate::shader::{AlphaGen, BlendFactor, RgbGen, StageBlend};
use crate::sky::{CloudGrid, CubeFace, FaceBounds, Rotation, SkyClip};
use crate::stage::{DeformOp, DrawInputs, PreparedStage, StageEvaluator, TexCoordOp};
use qa_core::primitives::Vec3;
use qa_core::stamps::StampSet;
use std::ffi::{CStr, c_char, c_void};
use std::marker::PhantomData;
use std::mem::{offset_of, size_of, size_of_val};
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
    polygon_offset: "glPolygonOffset"(f32, f32);
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
    texture_buffer: "glTexBuffer"(u32, u32, u32);
    gen_samplers: "glGenSamplers"(i32, *mut u32);
    sampler_parameter: "glSamplerParameteri"(u32, u32, i32);
    bind_sampler: "glBindSampler"(u32, u32);
    delete_samplers: "glDeleteSamplers"(i32, *const u32);
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
    uniform_integers: "glUniform1iv"(i32, i32, *const i32);
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
layout(location=4) in vec3 a_normal;
uniform mat4 u_mvp;
uniform vec4 u_color;
uniform vec4 u_alpha_value;
uniform vec4 u_view_origin;
uniform vec4 u_texture_scale;
uniform vec4 u_light_ambient;
uniform vec4 u_light_directed;
uniform vec4 u_light_direction;
uniform vec4 u_specular_origin;
uniform vec4 u_tex_vectors[2];
uniform vec4 u_mod_data[8];
uniform vec4 u_deform_data[6];
uniform int u_mod_kind[4];
uniform int u_deform_kind[3];
uniform samplerBuffer u_tables;
uniform int u_texgen;
uniform int u_rgb_gen;
uniform int u_alpha_gen;
out vec2 texcoord;
out vec4 color;
float lookup(int index) { return texelFetch(u_tables,index).r; }
int wave_index(float phase) { return int(phase*1024.0)&1023; }
vec3 normal_fast(vec3 v) {
    float x=dot(v,v);
    float y=uintBitsToFloat(0x5f3759dfu-(floatBitsToUint(x)>>1u));
    return v*(y*(1.5-x*0.5*y*y));
}
int perm(int index) { return int(lookup(5632+(index&255))); }
float noise_value(ivec4 cell) { return lookup(5376+perm(cell.x+perm(cell.y+perm(cell.z+perm(cell.w))))); }
float native_lerp(float a,float b,float w) { return a*(1.0-w)+b*w; }
float noise(vec4 point) {
    ivec4 cell=ivec4(floor(point));
    vec4 f=point-floor(point);
    float time_values[2];
    for(int t=0;t<2;t++) {
        float depth[2];
        for(int z=0;z<2;z++) {
            float rows[2];
            for(int y=0;y<2;y++) {
                ivec4 p=cell+ivec4(0,y,z,t);
                rows[y]=native_lerp(noise_value(p),noise_value(p+ivec4(1,0,0,0)),f.x);
            }
            depth[z]=native_lerp(rows[0],rows[1],f.y);
        }
        time_values[t]=native_lerp(depth[0],depth[1],f.z);
    }
    return native_lerp(time_values[0],time_values[1],f.w);
}
void main() {
    precise vec3 position=a_position;
    vec3 normal=a_normal;
    for(int i=0;i<3;i++) {
        vec4 a=u_deform_data[i*2],b=u_deform_data[i*2+1];
        if(u_deform_kind[i]==1) {
            float phase=(a.w+(position.x+position.y+position.z)*b.y)+b.x;
            position+=normal*(a.y+lookup(int(a.x)+wave_index(phase))*a.z);
        } else if(u_deform_kind[i]==2) { position+=a.xyz;
        } else if(u_deform_kind[i]==3) {
            int index=int((1024.0/6.283185307179586)*(a_texcoord.x*a.x+a.z))&1023;
            position+=normal*(lookup(index)*a.y);
        } else if(u_deform_kind[i]==4) {
            for(int j=0;j<3;j++) normal[j]+=a.x*noise(vec4(position*0.98+vec3(float(j)*100.0,0,0),a.y));
            normal=normal_fast(normal);
        }
    }
    gl_Position=u_mvp*vec4(position,1.0);
    vec4 vertex_color=floor(a_color*255.0+0.5);
    vec4 value=u_color;
    if(u_rgb_gen==1) value=vertex_color;
    if(u_rgb_gen==2) value=vec4(floor(vertex_color.rgb*u_texture_scale.z),vertex_color.a);
    if(u_rgb_gen==3) value.rgb=floor((vec3(255.0)-vertex_color.rgb)*u_texture_scale.z);
    if(u_rgb_gen==4) value=vec4(floor(min(u_light_ambient.xyz+max(dot(normal,u_light_direction.xyz),0.0)*u_light_directed.xyz,vec3(255.0))),255.0);
    if(u_alpha_gen==0) value.a=u_alpha_value.x;
    if(u_alpha_gen==2 && !(u_rgb_gen==2 && u_texture_scale.z==1.0)) value.a=255.0;
    if(u_alpha_gen==3 && u_rgb_gen!=2) value.a=vertex_color.a;
    if(u_alpha_gen==4) value.a=255.0-vertex_color.a;
    if(u_alpha_gen==5) value.a=floor(clamp(length(position-u_view_origin.xyz)/u_alpha_value.y,0.0,1.0)*255.0);
    if(u_alpha_gen==6) {
        vec3 direction=normal_fast(u_specular_origin.xyz-position);
        vec3 reflected=normal*(2.0*dot(normal,direction))-direction;
        float incidence=max(dot(reflected,normal_fast(u_view_origin.xyz-position)),0.0);
        value.a=floor(clamp((incidence*incidence)*(incidence*incidence),0.0,1.0)*255.0);
    }
    color=value/255.0;
    if(u_texgen==0) texcoord=a_texcoord*u_texture_scale.xy;
    if(u_texgen==1) texcoord=a_lightmap;
    if(u_texgen==2) {
        vec3 viewer=normal_fast(u_view_origin.xyz-position);
        vec3 reflected=normal*(2.0*dot(normal,viewer))-viewer;
        texcoord=vec2(0.5+reflected.y*0.5,0.5-reflected.z*0.5);
    }
    if(u_texgen==3) texcoord=vec2(dot(u_tex_vectors[0].xyz,position),dot(u_tex_vectors[1].xyz,position));
    if(u_texgen==4) {
        vec3 direction=position-u_view_origin.xyz;
        direction.z*=u_tex_vectors[0].x;
        float projected=u_tex_vectors[0].y/length(direction);
        float scroll=u_tex_vectors[1].x*u_tex_vectors[0].w;
        scroll-=floor(trunc(scroll)/u_tex_vectors[0].z)*u_tex_vectors[0].z;
        texcoord=(direction.xy*projected+vec2(scroll))/u_tex_vectors[0].z;
    }
    if(u_texgen==5) {
        vec3 direction=position-u_view_origin.xyz;
        float radius=u_tex_vectors[0].x,height=u_tex_vectors[0].y;
        float square=dot(direction,direction);
        float p=(-direction.z*radius+sqrt(direction.z*direction.z*radius*radius+square*(2.0*radius*height+height*height)))/square;
        vec3 intersection=normalize(direction*p+vec3(0,0,radius));
        texcoord=acos(clamp(intersection.xy,vec2(-1),vec2(1)));
    }
    for(int i=0;i<4;i++) {
        vec4 a=u_mod_data[i*2],b=u_mod_data[i*2+1];
        if(u_mod_kind[i]==1) texcoord=vec2(dot(texcoord,a.xz),dot(texcoord,a.yw))+b.xy;
        if(u_mod_kind[i]==2) texcoord+=vec2(lookup(wave_index((position.x+position.z)*(1.0/128.0)*0.125+a.y)),lookup(wave_index(position.y*(1.0/128.0)*0.125+a.y)))*a.x;
        if(u_mod_kind[i]==3) {
            ivec2 index=ivec2((texcoord.yx*a.yx*b.x+vec2(b.y))*(256.0/6.283185307179586))&ivec2(255);
            texcoord+=vec2(lookup(5120+index.x),lookup(5120+index.y))*a.zw;
        }
    }
}
"#;
const FRAGMENT_SHADER: &str = r#"#version 440 core
uniform sampler2D u_image;
uniform int u_alpha_test;
uniform int u_clamp;
uniform vec4 u_image_scale;
in vec2 texcoord;
in vec4 color;
layout(location=0) out vec4 fragment;
void main() {
    vec2 uv=texcoord;
    if(u_clamp==1) uv=clamp(uv,vec2(0),vec2(1));
    if(u_clamp==2) { vec2 edge=0.5/vec2(textureSize(u_image,0)); uv=clamp(uv,edge,vec2(1)-edge); }
    vec4 value = texture(u_image, uv) * color;
    value.rgb *= u_image_scale.rgb;
    if (u_alpha_test == 1 && value.a <= 0.0) discard;
    if (u_alpha_test == 2 && value.a < 0.5) discard;
    if (u_alpha_test == 3 && value.a >= 0.5) discard;
    fragment = value;
}
"#;

#[derive(Clone, Copy, Default)]
struct Mesh {
    first_index: usize,
    count: i32,
    base_vertex: i32,
}
struct SkyBatch {
    clip: SkyClip,
    cloud: Option<CloudGrid>,
}
struct Uniforms {
    mvp: i32,
    color: i32,
    texgen: i32,
    alpha_value: i32,
    view_origin: i32,
    texture_scale: i32,
    light_ambient: i32,
    light_directed: i32,
    light_direction: i32,
    specular_origin: i32,
    tex_vectors: i32,
    mod_data: i32,
    mod_kind: i32,
    deform_data: i32,
    deform_kind: i32,
    rgb_gen: i32,
    alpha_gen: i32,
    alpha_test: i32,
    clamp: i32,
    image_scale: i32,
}
#[derive(Clone, Copy)]
struct TextureParams {
    inverse_intensity: f32,
    sampler: Option<Sampler>,
}
#[derive(Default)]
struct State {
    vao: u32,
    texture: u32,
    blend: Option<Option<StageBlend>>,
    depth: Option<bool>,
    depth_write: Option<bool>,
    depth_func: Option<DepthFunc>,
    cull: Option<Cull>,
    sampler: u32,
    polygon_offset: Option<bool>,
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
    texture_params: Box<[TextureParams]>,
    samplers: [u32; 8],
    table_buffer: u32,
    table_texture: u32,
    evaluator: StageEvaluator,
    sky_batches: Box<[SkyBatch]>,
    sky_touched: StampSet,
    sky_drawn: StampSet,
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
                alpha_value: (gl.uniform_location)(program, c"u_alpha_value".as_ptr()),
                view_origin: (gl.uniform_location)(program, c"u_view_origin".as_ptr()),
                texture_scale: (gl.uniform_location)(program, c"u_texture_scale".as_ptr()),
                light_ambient: (gl.uniform_location)(program, c"u_light_ambient".as_ptr()),
                light_directed: (gl.uniform_location)(program, c"u_light_directed".as_ptr()),
                light_direction: (gl.uniform_location)(program, c"u_light_direction".as_ptr()),
                specular_origin: (gl.uniform_location)(program, c"u_specular_origin".as_ptr()),
                tex_vectors: (gl.uniform_location)(program, c"u_tex_vectors[0]".as_ptr()),
                mod_data: (gl.uniform_location)(program, c"u_mod_data[0]".as_ptr()),
                mod_kind: (gl.uniform_location)(program, c"u_mod_kind[0]".as_ptr()),
                deform_data: (gl.uniform_location)(program, c"u_deform_data[0]".as_ptr()),
                deform_kind: (gl.uniform_location)(program, c"u_deform_kind[0]".as_ptr()),
                rgb_gen: (gl.uniform_location)(program, c"u_rgb_gen".as_ptr()),
                alpha_gen: (gl.uniform_location)(program, c"u_alpha_gen".as_ptr()),
                alpha_test: (gl.uniform_location)(program, c"u_alpha_test".as_ptr()),
                clamp: (gl.uniform_location)(program, c"u_clamp".as_ptr()),
                image_scale: (gl.uniform_location)(program, c"u_image_scale".as_ptr()),
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
            texture_params: Box::new([]),
            samplers: [0; 8],
            table_buffer: 0,
            table_texture: 0,
            evaluator: StageEvaluator::load(),
            sky_batches: Box::new([]),
            sky_touched: StampSet::new(assets.materials().len()),
            sky_drawn: StampSet::new(assets.materials().len()),
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
            (gl.uniform_integer)((gl.uniform_location)(program, c"u_tables".as_ptr()), 1);
            (gl.active_texture)(0x84c1);
            (gl.bind_texture)(0x8c2a, backend.table_texture);
            (gl.active_texture)(0x84c0);
            (gl.depth_func)(0x0203);
            (gl.front_face)(0x0901);
            (gl.polygon_offset)(-1.0, -2.0);
            // Native Q3 CT_FRONT_SIDED retains clockwise projected triangles.
            (gl.cull_face)(0x0404);
            (gl.disable)(0x809d); // No multisample change to the native image.
            (gl.disable)(0x0bd0); // Explicit byte-color output; no default dithering.
            (gl.disable)(0x8db9); // Native byte upload lookups; no implicit sRGB transform.
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
        self.sky_batches = assets
            .materials()
            .iter()
            .map(|material| {
                let cloud = match material.settings.sky {
                    Some(Sky::Cube { clouds, .. })
                        if clouds.valid() && !material.stages.is_empty() =>
                    {
                        Some(
                            CloudGrid::generate(clouds)
                                .map_err(|error| format!("{}: {error}", material.name))?,
                        )
                    }
                    _ => None,
                };
                Ok(SkyBatch {
                    clip: SkyClip::new(),
                    cloud,
                })
            })
            .collect::<Result<Vec<_>, String>>()?
            .into_boxed_slice();
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
        self.texture_params = assets
            .images()
            .iter()
            .map(|image| TextureParams {
                inverse_intensity: image.prepared.as_ref().map_or(1.0, |p| p.inverse_intensity),
                sampler: image.native_sampler,
            })
            .collect();
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
                if let Some(prepared) = &image.prepared {
                    for (level, pixels) in prepared.levels.iter().enumerate() {
                        (gl.texture_image)(
                            TEXTURE_2D,
                            level as i32,
                            0x8058,
                            pixels.width as i32,
                            pixels.height as i32,
                            0,
                            RGBA,
                            UNSIGNED_BYTE,
                            pixels.rgba.as_ptr().cast(),
                        );
                    }
                    (gl.texture_parameter)(
                        TEXTURE_2D,
                        0x813d,
                        prepared.levels.len().saturating_sub(1) as i32,
                    );
                } else {
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
                    (gl.generate_mipmap)(TEXTURE_2D);
                }
                (gl.texture_parameter)(TEXTURE_2D, 0x2801, 0x2701); // linear, nearest mip
                (gl.texture_parameter)(TEXTURE_2D, 0x2800, 0x2601);
                (gl.texture_parameter)(TEXTURE_2D, 0x2802, 0x2901);
                (gl.texture_parameter)(TEXTURE_2D, 0x2803, 0x2901);
            }
            (gl.bind_texture)(TEXTURE_2D, 0);
            (gl.gen_buffers)(1, &mut self.table_buffer);
            (gl.bind_buffer)(0x8c2a, self.table_buffer);
            let tables = self.evaluator.gpu_tables();
            (gl.buffer_data)(
                0x8c2a,
                size_of_val(tables) as isize,
                tables.as_ptr().cast(),
                0x88e4,
            );
            (gl.gen_textures)(1, &mut self.table_texture);
            (gl.bind_texture)(0x8c2a, self.table_texture);
            (gl.texture_buffer)(0x8c2a, 0x822e, self.table_buffer);
            (gl.gen_samplers)(8, self.samplers.as_mut_ptr());
            for (index, &sampler) in self.samplers.iter().enumerate() {
                let filter = if index & 2 != 0 { 0x2601 } else { 0x2600 };
                let min_filter = if index & 4 != 0 {
                    if index & 2 != 0 { 0x2701 } else { 0x2700 }
                } else {
                    filter
                };
                let wrap = if index & 1 != 0 { 0x812d } else { 0x2901 }; // clamp-to-border emulates legacy GL_CLAMP after UV clamp
                (gl.sampler_parameter)(sampler, 0x2800, filter);
                (gl.sampler_parameter)(sampler, 0x2801, min_filter);
                (gl.sampler_parameter)(sampler, 0x2802, wrap);
                (gl.sampler_parameter)(sampler, 0x2803, wrap);
            }
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
        let mut inputs = DrawInputs::default();
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
                    inputs = DrawInputs {
                        time_ms: view.refdef.time_ms,
                        view_origin: view.refdef.origin,
                        identity_light: view.refdef.identity_light,
                        ..DrawInputs::default()
                    };
                    self.collect_skies(
                        list,
                        view.scene.draws,
                        assets,
                        inputs.view_origin,
                        &mut stats,
                    );
                    for item in list.draws(view.scene.draws) {
                        match item.kind {
                            DrawKind::Surface => self.world_surface(
                                list.surface(item.index),
                                assets,
                                &projection,
                                inputs,
                                view.refdef.far,
                                &mut stats,
                            ),
                            DrawKind::Entity => self.entity(
                                list.entity(item.index),
                                assets,
                                &projection,
                                inputs,
                                view.refdef.far,
                                &mut stats,
                            ),
                            DrawKind::Poly => {
                                let poly = list.poly(item.index);
                                let Some(material) = assets.material(poly.material) else {
                                    stats.rejected = stats.rejected.saturating_add(1);
                                    continue;
                                };
                                if material.settings.fog.is_some() {
                                    stats.rejected = stats.rejected.saturating_add(1);
                                    continue;
                                }
                                if matches!(material.settings.sky, Some(Sky::Cube { .. })) {
                                    self.draw_sky(
                                        poly.material,
                                        assets,
                                        &projection,
                                        inputs,
                                        view.refdef.far,
                                        &mut stats,
                                    );
                                    continue;
                                }
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
                                    inputs,
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
                        }
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
                    if !self.draw_2d(draw, assets, inputs) {
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
        let first = self.reserve(vertices.len())?;
        let mapped = self.dynamic.mapped?;
        unsafe {
            ptr::copy_nonoverlapping(
                vertices.as_ptr(),
                mapped.as_ptr().add(first),
                vertices.len(),
            )
        };
        Some(first)
    }

    fn reserve(&mut self, count: usize) -> Option<usize> {
        let dynamic = &mut self.dynamic;
        if !dynamic.acquired || count > dynamic.capacity - dynamic.used {
            return None;
        }
        dynamic.mapped?;
        let first = dynamic.slot * dynamic.capacity + dynamic.used;
        dynamic.used += count;
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

    fn collect_skies(
        &mut self,
        list: &CommandList,
        draws: Span,
        assets: &Assets,
        origin: Vec3,
        stats: &mut BackendStats,
    ) {
        self.sky_touched.begin();
        self.sky_drawn.begin();
        // The frontend already selected visibility and owns these submissions.
        // All geometry contributes to one clip per material; no second PVS walk.
        for item in list.draws(draws) {
            match item.kind {
                DrawKind::Surface => {
                    let reference = list.surface(item.index);
                    let Some(world) = assets.world(reference.world) else {
                        continue;
                    };
                    let Some(binding) = world.bindings.get(reference.surface as usize) else {
                        continue;
                    };
                    let Some(clip) = self.sky_clip(binding.material, assets, stats) else {
                        continue;
                    };
                    let Some(surface) = world.geometry.surfaces.get(reference.surface as usize)
                    else {
                        stats.rejected = stats.rejected.saturating_add(1);
                        continue;
                    };
                    for triangle in
                        world.geometry.indices[surface.indices.indices()].chunks_exact(3)
                    {
                        let points = [triangle[0], triangle[1], triangle[2]]
                            .map(|index| world.geometry.vertices[index as usize].vertex.position);
                        if !clip.add_polygon(&points, origin) {
                            stats.rejected = stats.rejected.saturating_add(1);
                        }
                    }
                }
                DrawKind::Entity => {
                    let entity = list.entity(item.index);
                    let Some(model) = assets.model(entity.model) else {
                        continue;
                    };
                    let material_id = entity.material.unwrap_or(model.material);
                    let Some(clip) = self.sky_clip(material_id, assets, stats) else {
                        continue;
                    };
                    for triangle in model.indices.chunks_exact(3) {
                        let points = [triangle[0], triangle[1], triangle[2]].map(|index| {
                            let position = model.vertices[index as usize].position;
                            Vec3(std::array::from_fn(|axis| {
                                entity.origin.0[axis]
                                    + entity.axes[0].0[axis] * position.0[0]
                                    + entity.axes[1].0[axis] * position.0[1]
                                    + entity.axes[2].0[axis] * position.0[2]
                            }))
                        });
                        if !clip.add_polygon(&points, origin) {
                            stats.rejected = stats.rejected.saturating_add(1);
                        }
                    }
                }
                DrawKind::Poly => {
                    let poly = list.poly(item.index);
                    let Some(clip) = self.sky_clip(poly.material, assets, stats) else {
                        continue;
                    };
                    let vertices = list.vertices(poly.vertices);
                    for triangle in vertices[1..].windows(2) {
                        let points = [
                            vertices[0].position,
                            triangle[0].position,
                            triangle[1].position,
                        ];
                        if !clip.add_polygon(&points, origin) {
                            stats.rejected = stats.rejected.saturating_add(1);
                        }
                    }
                }
            }
        }
    }

    fn sky_clip(
        &mut self,
        material_id: MaterialId,
        assets: &Assets,
        stats: &mut BackendStats,
    ) -> Option<&mut SkyClip> {
        let material = assets.material(material_id)?;
        if material.settings.fog.is_some()
            || !matches!(material.settings.sky, Some(Sky::Cube { .. }))
        {
            return None;
        }
        let Some(batch) = self.sky_batches.get_mut(material_id.0 as usize) else {
            stats.rejected = stats.rejected.saturating_add(1);
            return None;
        };
        if !self.sky_touched.test_and_set(material_id.0 as usize) {
            batch.clip.clear();
        }
        Some(&mut batch.clip)
    }

    fn draw_sky(
        &mut self,
        material_id: MaterialId,
        assets: &Assets,
        projection: &[f32; 16],
        inputs: DrawInputs,
        far: f32,
        stats: &mut BackendStats,
    ) {
        let Some(material) = assets.material(material_id) else {
            stats.rejected = stats.rejected.saturating_add(1);
            return;
        };
        let Some(Sky::Cube {
            outer_box,
            inner_box: _,
            clouds: _,
            rotation,
            params,
        }) = material.settings.sky
        else {
            return;
        };
        let Some(batch) = self.sky_batches.get_mut(material_id.0 as usize) else {
            stats.rejected = stats.rejected.saturating_add(1);
            return;
        };
        if !self.sky_touched.contains(material_id.0 as usize)
            || self.sky_drawn.test_and_set(material_id.0 as usize)
        {
            return;
        }
        let mut bounds = *batch.clip.bounds();
        if !bounds.iter().any(|bound| bound.visible()) {
            return;
        }
        if material.settings.fog.is_some() {
            stats.rejected = stats.rejected.saturating_add(1);
            return;
        }
        if rotation.is_some_and(|rotation| rotation.degrees_per_second != 0.0) {
            // Native Q2 draws the complete cube when rotating; bounded old
            // rectangles would leave gaps after the rotation.
            bounds.fill(FaceBounds {
                mins: [-1.0; 2],
                maxs: [1.0; 2],
            });
        }
        self.set_depth_hack(false);
        if params.far_depth {
            unsafe {
                (self.gl.depth_range)(1.0, 1.0);
            }
        }
        if let Some(images) = outer_box {
            self.draw_sky_box(
                images, bounds, params, rotation, projection, inputs, far, stats,
            );
        }
        self.draw_clouds(
            material_id,
            material,
            bounds,
            params,
            projection,
            inputs,
            far,
            stats,
        );
        // Native Q3 parses innerbox images but leaves the inner draw TODO in
        // tr_sky.c. Preserving that field does not add a new stock visual pass.
        if params.far_depth {
            unsafe {
                (self.gl.depth_range)(0.0, 1.0);
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_sky_box(
        &mut self,
        images: [crate::assets::ImageId; 6],
        bounds: [FaceBounds; 6],
        params: CubeSkyParams,
        rotation: Option<Rotation>,
        projection: &[f32; 16],
        inputs: DrawInputs,
        far: f32,
        stats: &mut BackendStats,
    ) {
        let settings = MaterialSettings {
            cull: Cull::None,
            ..MaterialSettings::default()
        };
        for face in CubeFace::ALL {
            let bound = bounds[face.index()];
            if !bound.visible() {
                continue;
            }
            let (mins, maxs) = if params.snap_bounds {
                let Some([mins, maxs]) = bound.grid_bounds() else {
                    continue;
                };
                (
                    mins.map(|value| (value as f32 - 4.0) / 4.0),
                    maxs.map(|value| (value as f32 - 4.0) / 4.0),
                )
            } else {
                (bound.mins, bound.maxs)
            };
            let vertices = [
                [mins[0], mins[1]],
                [mins[0], maxs[1]],
                [maxs[0], maxs[1]],
                [maxs[0], mins[1]],
            ]
            .map(|st| {
                let cube = crate::sky::cube_vertex(
                    face,
                    st,
                    params.distance.value(far),
                    params.texcoord_range,
                );
                let direction = if let Some(rotation) = rotation {
                    crate::sky::unrotate(
                        cube.direction,
                        Rotation {
                            degrees_per_second: -rotation.degrees_per_second,
                            ..rotation
                        },
                        inputs.time_ms as f32 * 0.001,
                    )
                } else {
                    cube.direction
                };
                Vertex {
                    position: Vec3(std::array::from_fn(|i| {
                        inputs.view_origin.0[i] + direction.0[i]
                    })),
                    texcoord: cube.uv,
                    ..Vertex::default()
                }
            });
            let Some(first) = self.upload(&vertices) else {
                stats.rejected = stats.rejected.saturating_add(1);
                continue;
            };
            let stage = Stage {
                texture: StageTexture::Image(images[face.index()]),
                sampler: params.sampler,
                rgb_gen: RgbGen::IdentityLighting,
                depth_write: !params.far_depth,
                ..Stage::default()
            };
            self.bind_vao(self.dynamic.vao);
            if !self.stage(stage, settings, inputs, projection, true, false) {
                stats.rejected = stats.rejected.saturating_add(1);
                continue;
            }
            unsafe {
                (self.gl.draw_arrays)(TRIANGLE_FAN, first as i32, 4);
            }
            stats.stages = stats.stages.saturating_add(1);
            stats.triangles = stats.triangles.saturating_add(2);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_clouds(
        &mut self,
        material_id: MaterialId,
        material: &crate::assets::Material,
        bounds: [FaceBounds; 6],
        params: CubeSkyParams,
        projection: &[f32; 16],
        inputs: DrawInputs,
        far: f32,
        stats: &mut BackendStats,
    ) {
        if material.stages.is_empty() {
            return;
        }
        let Some(batch) = self.sky_batches.get(material_id.0 as usize) else {
            stats.rejected = stats.rejected.saturating_add(1);
            return;
        };
        if batch.cloud.is_none() {
            stats.rejected = stats.rejected.saturating_add(1);
            return;
        }
        let count = CubeFace::ALL
            .into_iter()
            .filter(|&face| face != CubeFace::NegativeZ)
            .filter_map(|face| bounds[face.index()].grid_bounds())
            .map(|[mins, maxs]| (maxs[0] - mins[0]) * (maxs[1] - mins[1]) * 6)
            .sum::<usize>();
        if count == 0 {
            return;
        }
        let Some(first) = self.reserve(count) else {
            stats.rejected = stats.rejected.saturating_add(1);
            return;
        };
        let Some(mapped) = self.dynamic.mapped else {
            stats.rejected = stats.rejected.saturating_add(1);
            return;
        };
        let Some(grid) = self.sky_batches[material_id.0 as usize].cloud.as_ref() else {
            stats.rejected = stats.rejected.saturating_add(1);
            return;
        };
        let mut written = 0;
        for face in CubeFace::ALL {
            if face == CubeFace::NegativeZ {
                continue;
            }
            let Some([mins, maxs]) = bounds[face.index()].grid_bounds() else {
                continue;
            };
            for t in mins[1]..maxs[1] {
                for s in mins[0]..maxs[0] {
                    // Native FillCloudySkySide keeps this diagonal and index
                    // order. All visible faces share one contiguous draw range.
                    for [s, t] in [
                        [s, t],
                        [s, t + 1],
                        [s + 1, t],
                        [s, t + 1],
                        [s + 1, t + 1],
                        [s + 1, t],
                    ] {
                        let st = [(s as f32 - 4.0) / 4.0, (t as f32 - 4.0) / 4.0];
                        let direction = crate::sky::cube_vertex(
                            face,
                            st,
                            params.distance.value(far),
                            [0.0, 1.0],
                        )
                        .direction;
                        let vertex = Vertex {
                            position: Vec3(std::array::from_fn(|i| {
                                inputs.view_origin.0[i] + direction.0[i]
                            })),
                            normal: Vec3::default(),
                            texcoord: grid.uv[face.index()][t][s],
                            ..Vertex::default()
                        };
                        unsafe {
                            mapped.as_ptr().add(first + written).write(vertex);
                        }
                        written += 1;
                    }
                }
            }
        }
        self.bind_vao(self.dynamic.vao);
        for &stage in &material.stages {
            // The shared load-generated cloud grid already supplies this
            // generator's coordinates, like native tess.texCoords[0].
            let stage = if matches!(stage.texgen, TcGen::CloudSky { .. }) {
                Stage {
                    texgen: TcGen::Texture,
                    ..stage
                }
            } else {
                stage
            };
            if !self.stage(stage, material.settings, inputs, projection, true, false) {
                stats.rejected = stats.rejected.saturating_add(1);
                continue;
            }
            unsafe {
                (self.gl.draw_arrays)(TRIANGLES, first as i32, written as i32);
            }
            stats.stages = stats.stages.saturating_add(1);
        }
        stats.triangles = stats.triangles.saturating_add(written as u32 / 3);
    }

    fn entity(
        &mut self,
        entity: &SceneEntity,
        assets: &Assets,
        projection: &[f32; 16],
        mut inputs: DrawInputs,
        far: f32,
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
        if material.settings.fog.is_some() {
            stats.rejected = stats.rejected.saturating_add(1);
            return;
        }
        if mesh.count == 0 {
            return;
        }
        if matches!(material.settings.sky, Some(Sky::Cube { .. })) {
            self.draw_sky(material_id, assets, projection, inputs, far, stats);
            return;
        }
        let matrix = multiply(projection, &model_matrix(entity));
        inputs.view_origin = crate::stage::entity_view_origin(inputs.view_origin, entity);
        inputs.entity_color = entity.color;
        inputs.entity_texcoord = entity.shader_texcoord;
        inputs.entity_shader_time = entity.shader_time;
        inputs.lighting = entity.lighting;
        self.bind_vao(self.static_vao);
        self.set_depth_hack(entity.depth_hack);
        for stage in material.stages.iter() {
            if !self.stage(*stage, material.settings, inputs, &matrix, true, false) {
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
            stats.stages = stats.stages.saturating_add(1);
        }
        stats.triangles = stats.triangles.saturating_add(mesh.count as u32 / 3);
        self.set_depth_hack(false);
    }

    fn world_surface(
        &mut self,
        surface: &crate::scene::SurfaceRef,
        assets: &Assets,
        projection: &[f32; 16],
        mut inputs: DrawInputs,
        far: f32,
        stats: &mut BackendStats,
    ) {
        let Some(world) = assets.world(surface.world) else {
            stats.rejected = stats.rejected.saturating_add(1);
            return;
        };
        let Some(binding) = world.bindings.get(surface.surface as usize) else {
            stats.rejected = stats.rejected.saturating_add(1);
            return;
        };
        let (Some(material), Some(&mesh)) = (
            assets.material(binding.material),
            self.meshes.get(world.mesh.0 as usize),
        ) else {
            stats.rejected = stats.rejected.saturating_add(1);
            return;
        };
        if material.settings.fog.is_some() {
            stats.rejected = stats.rejected.saturating_add(1);
            return;
        }
        if binding.mesh_indices.count == 0 {
            return;
        }
        if let Some(Sky::Cube { .. }) = material.settings.sky {
            self.draw_sky(binding.material, assets, projection, inputs, far, stats);
            stats.surfaces = stats.surfaces.saturating_add(1);
            return;
        }
        inputs.lightmap = binding.lightmap;
        inputs.texture_scale = binding.texture_scale;
        self.bind_vao(self.static_vao);
        self.set_depth_hack(false);
        for stage in &material.stages {
            if !self.stage(*stage, material.settings, inputs, projection, true, false) {
                stats.rejected = stats.rejected.saturating_add(1);
                continue;
            }
            unsafe {
                (self.gl.draw_elements)(
                    TRIANGLES,
                    binding.mesh_indices.count as i32,
                    UNSIGNED_INT,
                    ((mesh.first_index + binding.mesh_indices.first as usize) * size_of::<u32>())
                        as *const c_void,
                    mesh.base_vertex,
                );
            }
            stats.stages = stats.stages.saturating_add(1);
        }
        stats.surfaces = stats.surfaces.saturating_add(1);
        stats.triangles = stats
            .triangles
            .saturating_add(binding.mesh_indices.count / 3);
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_dynamic(
        &mut self,
        assets: &Assets,
        material_id: MaterialId,
        first: usize,
        count: usize,
        matrix: &[f32; 16],
        inputs: DrawInputs,
        depth: bool,
        force_alpha: bool,
    ) -> bool {
        let Some(material) = assets.material(material_id) else {
            return false;
        };
        if material.settings.fog.is_some() {
            return false;
        }
        self.bind_vao(self.dynamic.vao);
        self.set_depth_hack(false);
        let mut complete = true;
        for stage in material.stages.iter() {
            if !self.stage(
                *stage,
                material.settings,
                inputs,
                matrix,
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

    fn draw_2d(&mut self, draw: Draw2d, assets: &Assets, mut inputs: DrawInputs) -> bool {
        let vertices = quad(draw.rect, draw.texcoords, draw.color);
        let Some(first) = self.upload(&vertices) else {
            return false;
        };
        let projection = orthographic(self.width, self.height);
        inputs.entity_color = draw.color;
        self.draw_dynamic(
            assets,
            draw.material,
            first,
            vertices.len(),
            &projection,
            inputs,
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
            blend: Some(StageBlend {
                source: BlendFactor::SourceAlpha,
                destination: BlendFactor::OneMinusSourceAlpha,
            }),
            ..Stage::default()
        };
        let projection = orthographic(self.width, self.height);
        if !self.stage(
            stage,
            MaterialSettings {
                cull: Cull::None,
                ..MaterialSettings::default()
            },
            DrawInputs::default(),
            &projection,
            false,
            false,
        ) {
            stats.rejected = stats.rejected.saturating_add(1);
            return;
        }
        // Native GL screen blends use floats, rather than a stage generator's
        // byte colors. The phase remains shared with CPU palette presentation.
        let uniform_color = color.map(|value| value * 255.0);
        unsafe {
            (self.gl.uniform_color)(self.uniforms.color, 1, uniform_color.as_ptr());
            (self.gl.uniform_color)(
                self.uniforms.alpha_value,
                1,
                [color[3] * 255.0, 0.0, 0.0, 0.0].as_ptr(),
            );
            (self.gl.uniform_integer)(self.uniforms.alpha_gen, 0);
        }
        unsafe { (self.gl.draw_arrays)(TRIANGLE_FAN, first as i32, 4) };
    }

    #[allow(clippy::too_many_arguments)]
    fn stage(
        &mut self,
        stage: Stage,
        settings: MaterialSettings,
        inputs: DrawInputs,
        matrix: &[f32; 16],
        depth: bool,
        force_alpha: bool,
    ) -> bool {
        let Ok(prepared) = self.evaluator.prepare(&stage, settings, inputs) else {
            return false;
        };
        let Ok(deforms) = self.evaluator.prepare_deforms(&settings, &inputs) else {
            return false;
        };
        let Some(&texture) = self.textures.get(prepared.image.0 as usize) else {
            return false;
        };
        let blend = if force_alpha && stage.blend.is_none() {
            Some(StageBlend {
                source: BlendFactor::SourceAlpha,
                destination: BlendFactor::OneMinusSourceAlpha,
            })
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
        let cull = if depth { settings.cull } else { Cull::None };
        if self.state.cull != Some(cull) {
            unsafe {
                if cull != Cull::None {
                    (self.gl.enable)(CULL_FACE);
                    (self.gl.cull_face)(if cull == Cull::Front { 0x0404 } else { 0x0405 });
                } else {
                    (self.gl.disable)(CULL_FACE)
                }
            }
            self.state.cull = Some(cull);
        }
        if self.state.polygon_offset != Some(settings.polygon_offset) {
            unsafe {
                if settings.polygon_offset {
                    (self.gl.enable)(0x8037);
                } else {
                    (self.gl.disable)(0x8037);
                }
            }
            self.state.polygon_offset = Some(settings.polygon_offset);
        }
        if self.state.texture != texture {
            unsafe { (self.gl.bind_texture)(TEXTURE_2D, texture) };
            self.state.texture = texture;
        }
        let texture_params = self.texture_params[prepared.image.0 as usize];
        let image_sampler = texture_params.sampler.unwrap_or(stage.sampler);
        let sampler_index = usize::from(image_sampler.wrap == Wrap::Clamp)
            | (usize::from(image_sampler.filter == Filter::Linear) << 1)
            | (usize::from(image_sampler.mipmaps) << 2);
        let sampler = self.samplers[sampler_index];
        if self.state.sampler != sampler {
            unsafe {
                (self.gl.bind_sampler)(0, sampler);
            }
            self.state.sampler = sampler;
        }
        let rgb_scale = match stage.texture_intensity {
            TextureIntensity::Preserve => 1.0,
            TextureIntensity::NeutralizeUpload => texture_params.inverse_intensity,
        };
        self.stage_uniforms(&prepared, deforms, matrix, image_sampler, rgb_scale);
        true
    }

    fn stage_uniforms(
        &self,
        prepared: &PreparedStage,
        deforms: [DeformOp; 3],
        matrix: &[f32; 16],
        sampler: Sampler,
        rgb_scale: f32,
    ) {
        let (stage, inputs) = (prepared.stage, prepared.inputs);
        let color = prepared.uniform_color.map(|v| v as f32);
        let rgb = match stage.rgb_gen {
            RgbGen::ExactVertex => 1,
            RgbGen::Vertex => 2,
            RgbGen::OneMinusVertex => 3,
            RgbGen::LightingDiffuse => 4,
            _ => 0,
        };
        let alpha = match stage.alpha_gen {
            AlphaGen::Skip => 1,
            AlphaGen::Identity => 2,
            AlphaGen::Vertex => 3,
            AlphaGen::OneMinusVertex => 4,
            AlphaGen::Portal(_) => 5,
            AlphaGen::LightingSpecular => 6,
            _ => 0,
        };
        let portal = match stage.alpha_gen {
            AlphaGen::Portal(range) => range,
            _ => 1.0,
        };
        let vectors = match stage.texgen {
            TcGen::Vector(vectors) => [vec4(Vec3(vectors[0])), vec4(Vec3(vectors[1]))],
            TcGen::LayeredSky {
                flatten_z,
                projected_scale,
                texture_size,
                scroll_speed,
            } => [
                [flatten_z, projected_scale, texture_size, scroll_speed],
                [prepared.shader_time, 0.0, 0.0, 0.0],
            ],
            TcGen::CloudSky { radius, height } => [[radius, height, 0.0, 0.0], [0.0; 4]],
            _ => [[0.0; 4]; 2],
        };
        let light = inputs.lighting.unwrap_or_default();
        let (mod_kinds, mod_data) = pack_texmods(prepared.tcmods);
        let (deform_kinds, deform_data) = pack_deforms(deforms);
        unsafe {
            (self.gl.uniform_matrix)(self.uniforms.mvp, 1, 0, matrix.as_ptr());
            (self.gl.uniform_color)(self.uniforms.color, 1, color.as_ptr());
            (self.gl.uniform_color)(
                self.uniforms.image_scale,
                1,
                [rgb_scale, rgb_scale, rgb_scale, 1.0].as_ptr(),
            );
            (self.gl.uniform_color)(
                self.uniforms.alpha_value,
                1,
                [prepared.uniform_alpha as f32, portal, 0.0, 0.0].as_ptr(),
            );
            (self.gl.uniform_color)(
                self.uniforms.view_origin,
                1,
                vec4(inputs.view_origin).as_ptr(),
            );
            (self.gl.uniform_color)(
                self.uniforms.texture_scale,
                1,
                [
                    inputs.texture_scale[0],
                    inputs.texture_scale[1],
                    inputs.identity_light,
                    0.0,
                ]
                .as_ptr(),
            );
            (self.gl.uniform_color)(
                self.uniforms.light_ambient,
                1,
                vec4(Vec3(light.ambient)).as_ptr(),
            );
            (self.gl.uniform_color)(
                self.uniforms.light_directed,
                1,
                vec4(Vec3(light.directed)).as_ptr(),
            );
            (self.gl.uniform_color)(
                self.uniforms.light_direction,
                1,
                vec4(light.direction).as_ptr(),
            );
            (self.gl.uniform_color)(
                self.uniforms.specular_origin,
                1,
                vec4(inputs.specular_origin).as_ptr(),
            );
            (self.gl.uniform_color)(self.uniforms.tex_vectors, 2, vectors.as_ptr().cast());
            (self.gl.uniform_integers)(self.uniforms.mod_kind, 4, mod_kinds.as_ptr());
            (self.gl.uniform_color)(self.uniforms.mod_data, 8, mod_data.as_ptr().cast());
            (self.gl.uniform_integers)(self.uniforms.deform_kind, 3, deform_kinds.as_ptr());
            (self.gl.uniform_color)(self.uniforms.deform_data, 6, deform_data.as_ptr().cast());
            (self.gl.uniform_integer)(self.uniforms.rgb_gen, rgb);
            (self.gl.uniform_integer)(self.uniforms.alpha_gen, alpha);
            (self.gl.uniform_integer)(
                self.uniforms.texgen,
                match stage.texgen {
                    TcGen::Texture => 0,
                    TcGen::Lightmap => 1,
                    TcGen::Environment => 2,
                    TcGen::Vector(_) => 3,
                    TcGen::LayeredSky { .. } => 4,
                    TcGen::CloudSky { .. } => 5,
                },
            );
            (self.gl.uniform_integer)(
                self.uniforms.alpha_test,
                match stage.alpha_test {
                    AlphaTest::None => 0,
                    AlphaTest::GreaterZero => 1,
                    AlphaTest::AtLeastHalf => 2,
                    AlphaTest::LessThanHalf => 3,
                },
            );
            (self.gl.uniform_integer)(
                self.uniforms.clamp,
                if sampler.wrap == Wrap::Repeat {
                    0
                } else if sampler.filter == Filter::Nearest {
                    2
                } else {
                    1
                },
            );
        }
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
    fn blend_state(&mut self, blend: Option<StageBlend>) {
        if self.state.blend == Some(blend) {
            return;
        }
        unsafe {
            if let Some(blend) = blend {
                (self.gl.enable)(BLEND);
                (self.gl.blend_func)(blend.source as u32, blend.destination as u32);
            } else {
                (self.gl.disable)(BLEND);
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
            (self.gl.delete_buffers)(1, &self.table_buffer);
            (self.gl.delete_textures)(1, &self.table_texture);
            (self.gl.delete_samplers)(8, self.samplers.as_ptr());
            (self.gl.delete_program)(self.program);
        }
    }
}

unsafe fn vertex_layout(gl: &Gl) {
    let stride = size_of::<Vertex>() as i32;
    unsafe {
        for index in 0..5 {
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
        (gl.vertex_attribute)(
            4,
            3,
            FLOAT,
            0,
            stride,
            offset_of!(Vertex, normal) as *const c_void,
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
        && view.identity_light.is_finite()
        && (0.0..=1.0).contains(&view.identity_light)
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
        normal: Vec3([0.0, 0.0, 1.0]),
        texcoord,
        lightmap_coord: texcoord,
        color,
    })
}

fn vec4(v: Vec3) -> [f32; 4] {
    [v.0[0], v.0[1], v.0[2], 0.0]
}
fn pack_texmods(modifiers: [TexCoordOp; 4]) -> ([i32; 4], [[f32; 4]; 8]) {
    let mut kinds = [0; 4];
    let mut data = [[0.0; 4]; 8];
    for (index, modifier) in modifiers.into_iter().enumerate() {
        match modifier {
            TexCoordOp::None => {}
            TexCoordOp::Transform { matrix, translate } => {
                kinds[index] = 1;
                data[index * 2] = [matrix[0][0], matrix[0][1], matrix[1][0], matrix[1][1]];
                data[index * 2 + 1] = [translate[0], translate[1], 0.0, 0.0];
            }
            TexCoordOp::Turbulent { amplitude, now } => {
                kinds[index] = 2;
                data[index * 2] = [amplitude, now, 0.0, 0.0];
            }
            TexCoordOp::Warp {
                texel_scale,
                amplitude,
                frequency,
                now,
            } => {
                kinds[index] = 3;
                data[index * 2] = [texel_scale[0], texel_scale[1], amplitude[0], amplitude[1]];
                data[index * 2 + 1] = [frequency, now, 0.0, 0.0];
            }
        }
    }
    (kinds, data)
}
fn pack_deforms(deforms: [DeformOp; 3]) -> ([i32; 3], [[f32; 4]; 6]) {
    let mut kinds = [0; 3];
    let mut data = [[0.0; 4]; 6];
    for (index, deform) in deforms.into_iter().enumerate() {
        match deform {
            DeformOp::None => {}
            DeformOp::Wave {
                table,
                base,
                amplitude,
                phase,
                time_phase,
                spread,
            } => {
                kinds[index] = 1;
                data[index * 2] = [table as f32, base, amplitude, phase];
                data[index * 2 + 1] = [time_phase, spread, 0.0, 0.0];
            }
            DeformOp::Move(vector) => {
                kinds[index] = 2;
                data[index * 2] = vec4(vector);
            }
            DeformOp::Bulge { width, height, now } => {
                kinds[index] = 3;
                data[index * 2] = [width, height, now, 0.0];
            }
            DeformOp::Normal { amplitude, time } => {
                kinds[index] = 4;
                data[index * 2] = [amplitude, time, 0.0, 0.0];
            }
        }
    }
    (kinds, data)
}
