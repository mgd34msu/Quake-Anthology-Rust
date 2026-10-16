//! Cached GL 4.4 command consumer. SDL context ownership stays in platform.
//!
//! Projection and stage order follow qsrc Q3 tr_main.c and tr_backend.c.
use crate::BackendStats;
use crate::assets::{
    AlphaTest, AssetAdmission, AssetCounts, Assets, CubeSkyParams, Cull, DepthFunc, Filter,
    MaterialId, MaterialSettings, Sampler, Sky, Stage, TcGen, TextureIntensity, Vertex, Wrap,
};
use crate::scene::{
    BlendPhase, Command, CommandList, Draw2d, DrawItem, DrawKind, PatchSelection, PatchTriangles,
    Refdef, SceneEntity, SceneParticle, Span, Viewport,
};
use crate::shader::reference::ResolvedShader;
use crate::shader::{AlphaGen, RgbGen, StageBlend};
use crate::sky::{CloudGrid, CubeFace, FaceBounds, Rotation, SkyBatches, SkyClip, SkyKey};
use crate::stage::{DeformOp, DrawInputs, PreparedStage, StageEvaluator, TexCoordOp};
use crate::world::geometry::{PatchGrid, WorldVertex};
use qa_core::presentation::{DisplayConfiguration, RendererConfiguration};
use qa_core::primitives::Vec3;
use std::ffi::{CStr, c_char, c_void};
use std::marker::PhantomData;
use std::mem::{MaybeUninit, offset_of, size_of, size_of_val};
use std::ptr::{self, NonNull};

mod composite;
use composite::MaterialTargets;
mod preparation;
use preparation::{PreparedAssets, StaticObjects, UploadBindings};

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
    get_string_index: "glGetStringi"(u32, u32) -> *const u8;
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
    texture_sub_image: "glTexSubImage2D"(u32, i32, i32, i32, i32, i32, u32, u32, *const c_void);
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
    draw_buffer: "glDrawBuffer"(u32);
    gen_framebuffers: "glGenFramebuffers"(i32, *mut u32);
    bind_framebuffer: "glBindFramebuffer"(u32, u32);
    delete_framebuffers: "glDeleteFramebuffers"(i32, *const u32);
    framebuffer_texture: "glFramebufferTexture2D"(u32, u32, u32, u32, i32);
    check_framebuffer: "glCheckFramebufferStatus"(u32) -> u32;
    framebuffer_parameter: "glGetFramebufferAttachmentParameteriv"(u32, u32, u32, *mut i32);
    blit_framebuffer: "glBlitFramebuffer"(i32, i32, i32, i32, i32, i32, i32, i32, u32, u32);
    gen_renderbuffers: "glGenRenderbuffers"(i32, *mut u32);
    bind_renderbuffer: "glBindRenderbuffer"(u32, u32);
    renderbuffer_storage: "glRenderbufferStorage"(u32, u32, i32, i32);
    framebuffer_renderbuffer: "glFramebufferRenderbuffer"(u32, u32, u32, u32);
    delete_renderbuffers: "glDeleteRenderbuffers"(i32, *const u32);
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
const CLIP_DISTANCE0: u32 = 0x3000;
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
layout(location=5) in vec3 a_old_position;
layout(location=6) in vec3 a_old_normal;
uniform mat4 u_mvp;
uniform vec4 u_frame_weights;
uniform vec4 u_color;
uniform vec4 u_alpha_value;
uniform vec4 u_view_origin;
uniform vec4 u_clip_plane;
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
    precise vec3 normal=a_normal;
    if(u_frame_weights.x!=0.0) {
        precise vec3 old_position=a_old_position*u_frame_weights.x;
        precise vec3 new_position=a_position*u_frame_weights.y;
        precise vec3 old_normal=a_old_normal*u_frame_weights.x;
        precise vec3 new_normal=a_normal*u_frame_weights.y;
        position=old_position+new_position;
        normal=normal_fast(old_normal+new_normal);
    }
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
    gl_ClipDistance[0]=dot(u_clip_plane,vec4(position,1.0));
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
    if(u_alpha_value.w!=0.0) color.a=u_alpha_value.z;
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

enum SurfaceDraw {
    Static(Span),
    Patch(PatchTriangles),
}

fn surface_draw(
    patch: Option<&PatchGrid>,
    first_vertex: u32,
    selection: PatchSelection,
    mesh_indices: Span,
) -> SurfaceDraw {
    match patch {
        Some(grid) if !selection.is_full(grid) => {
            SurfaceDraw::Patch(selection.triangles(grid, first_vertex))
        }
        _ => SurfaceDraw::Static(mesh_indices),
    }
}

fn triangle_chunk(
    triangles: &mut impl Iterator<Item = [u32; 3]>,
    capacity: usize,
    mut emit: impl FnMut(usize, u32),
) -> usize {
    let mut written = 0;
    for triangle in triangles.take(capacity / 3) {
        for vertex in triangle {
            emit(written, vertex);
            written += 1;
        }
    }
    written
}

struct Uniforms {
    mvp: i32,
    frame_weights: i32,
    color: i32,
    texgen: i32,
    alpha_value: i32,
    view_origin: i32,
    clip_plane: i32,
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
    texture: Option<u32>,
    blend: Option<Option<StageBlend>>,
    depth: Option<bool>,
    depth_write: Option<bool>,
    depth_func: Option<DepthFunc>,
    cull: Option<Cull>,
    sampler: u32,
    polygon_offset: Option<bool>,
    depth_hack: bool,
    frame_weights: Option<[f32; 2]>,
}
impl State {
    fn bind_texture(&mut self, gl: &Gl, texture: u32) {
        if self.texture != Some(texture) {
            // SAFETY: The backend owns the current GL context and loaded entry
            // points; numeric texture handles belong to its cold registration.
            unsafe { (gl.bind_texture)(TEXTURE_2D, texture) };
            self.texture = Some(texture);
        }
    }
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
impl Dynamic {
    fn begin(&mut self, gl: &Gl) -> bool {
        self.slot = self.next;
        self.next = (self.next + 1) % RING_SLOTS;
        self.used = 0;
        self.acquired = !self.blocked[self.slot];
        let fence = self.fences[self.slot];
        if !fence.is_null() {
            // Wait for the bytes this slot owns instead of discarding correct
            // draws merely because the GPU trails the CPU by three frames.
            // This driver wait remains part of measured backend time.
            let result = unsafe { (gl.wait_fence)(fence, 1, 1_000_000_000) };
            if result == 0x911a || result == 0x911c {
                unsafe { (gl.delete_fence)(fence) };
                self.fences[self.slot] = ptr::null_mut();
            } else {
                self.acquired = false;
                if result == 0x911d {
                    self.blocked[self.slot] = true;
                }
            }
        }
        self.acquired
    }

    fn end(&mut self, gl: &Gl) {
        if self.acquired && self.used != 0 {
            let fence = unsafe { (gl.fence)(0x9117, 0) };
            self.fences[self.slot] = fence;
            if fence.is_null() {
                // A failed fence cannot establish safe reuse of mapped bytes.
                self.blocked[self.slot] = true;
            }
        }
        self.acquired = false;
    }

    fn space(&mut self, gl: &Gl, minimum: usize) -> Option<usize> {
        self.mapped?;
        if !self.acquired || minimum > self.capacity {
            return None;
        }
        if minimum > self.capacity - self.used {
            self.end(gl);
            if !self.begin(gl) {
                return None;
            }
        }
        Some(self.capacity - self.used)
    }

    fn reserve(&mut self, gl: &Gl, count: usize) -> Option<usize> {
        self.space(gl, count)?;
        let first = self.slot * self.capacity + self.used;
        self.used += count;
        Some(first)
    }

    fn upload_range(
        &mut self,
        gl: &Gl,
        capacity: usize,
        fill: impl FnOnce(&mut [MaybeUninit<Vertex>]) -> usize,
    ) -> Option<(usize, usize)> {
        let first = self.reserve(gl, capacity)?;
        let mapped = self.mapped?;
        // SAFETY: reserve bounds this exclusively owned range in the mapped
        // slot; begin waited for its previous GPU consumer. Storage need not
        // be initialized until fill writes its returned prefix.
        let output = unsafe {
            std::slice::from_raw_parts_mut(
                mapped.as_ptr().add(first).cast::<MaybeUninit<Vertex>>(),
                capacity,
            )
        };
        let count = fill(output);
        if count > capacity {
            self.used -= capacity;
            return None;
        }
        self.used -= capacity - count;
        Some((first, count))
    }
}

/// Resolved once, before modules register assets into the cold asset owner.
pub struct GlContext {
    gl: Gl,
    configuration: std::sync::Arc<RendererConfiguration>,
    _context_thread: PhantomData<*mut ()>,
}
impl GlContext {
    /// # Safety
    /// `resolve` must return this thread's current GL context entry points.
    /// Keep that context current and alive through the consuming GlBackend.
    pub unsafe fn load(
        resolve: impl FnMut(&CStr) -> *const c_void,
        display: DisplayConfiguration,
    ) -> Result<Self, String> {
        let gl = unsafe { Gl::load(resolve)? };
        let mut values = [0; 5];
        for (target, name) in values
            .iter_mut()
            .zip([0x821b, 0x821c, 0x0d33, 0x8872, 0x821d])
        {
            unsafe { (gl.get_integer)(name, target) };
        }
        let [major, minor, texture_size, texture_units, extensions] = values;
        if major < 4 || (major == 4 && minor < 4) {
            return Err(format!(
                "GL 4.4 is required; current context is {major}.{minor}"
            ));
        }
        if texture_size <= 0 || texture_units <= 0 || extensions < 0 {
            return Err("invalid GL context limits".into());
        }
        let mut configuration = RendererConfiguration {
            display,
            max_texture_size: texture_size as u32,
            max_active_textures: texture_units as u32,
            texture_env_add: true,
            ..RendererConfiguration::default()
        };
        for (target, name) in [
            (&mut configuration.renderer, 0x1f01),
            (&mut configuration.vendor, 0x1f00),
            (&mut configuration.version, 0x1f02),
        ] {
            let text = unsafe { context_string(&gl, name)? };
            target
                .set_bytes(
                    &text.as_bytes()[..text
                        .len()
                        .min(qa_core::presentation::RENDERER_TEXT_BYTES - 1)],
                )
                .map_err(|_| "GL identity capacity")?;
        }
        for index in 0..extensions as u32 {
            let bytes = unsafe { (gl.get_string_index)(0x1f03, index) };
            if bytes.is_null() {
                return Err("missing GL extension string".into());
            }
            let bytes = unsafe { CStr::from_ptr(bytes.cast()) }.to_bytes();
            let separator = usize::from(!configuration.extensions.as_bytes().is_empty());
            if configuration.extensions.as_bytes().len() + separator + bytes.len()
                > qa_core::presentation::RENDERER_EXTENSIONS_BYTES - 1
            {
                break;
            }
            if separator != 0 {
                configuration
                    .extensions
                    .append_bytes(b" ")
                    .map_err(|_| "GL extensions capacity")?;
            }
            configuration
                .extensions
                .append_bytes(bytes)
                .map_err(|_| "GL extensions capacity")?;
        }
        Ok(Self {
            gl,
            configuration: std::sync::Arc::new(configuration),
            _context_thread: PhantomData,
        })
    }
    pub fn configuration(&self) -> &std::sync::Arc<RendererConfiguration> {
        &self.configuration
    }
}

/// Owns GPU resources on the thread with the current SDL GL context.
pub struct GlBackend {
    gl: Gl,
    gpu_teardown: bool,
    configuration: std::sync::Arc<RendererConfiguration>,
    width: u32,
    height: u32,
    program: u32,
    material_targets: MaterialTargets,
    scoped_sky: Option<Option<[FaceBounds; 6]>>,
    uniforms: Uniforms,
    static_vao: u32,
    model_vao: u32,
    static_buffers: [u32; 2],
    meshes: Box<[Mesh]>,
    textures: Box<[u32]>,
    texture_params: Box<[TextureParams]>,
    lightmap_updates: crate::lightmap::AtlasUpdates,
    admission: Option<AssetAdmission>,
    samplers: [u32; 8],
    table_buffer: u32,
    table_texture: u32,
    evaluator: StageEvaluator,
    skies: SkyBatches,
    sky_clouds: Box<[Option<CloudGrid>]>,
    dynamic: Dynamic,
    state: State,
    _context_thread: PhantomData<*mut ()>,
}

impl GlBackend {
    /// # Safety
    /// `context` must contain this current context's GL entry points. Keep that
    /// context current on this thread until ordinary backend destruction. If it
    /// cannot be restored, consume the backend with context_lost instead.
    /// Call prepare_assets after appending assets, before submitting their IDs.
    pub unsafe fn load(
        context: GlContext,
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
        let GlContext {
            gl, configuration, ..
        } = context;
        if configuration.display.width != width || configuration.display.height != height {
            return Err("GL context render-target size changed during registration".into());
        }
        let lightmap_updates = crate::lightmap::AtlasUpdates::load(assets)?;
        let program = unsafe { compile_program(&gl)? };
        // This backend owns the context. Unclipped and 2D stages publish a
        // positive constant distance, so the plane needs no frame-time toggle.
        unsafe { (gl.enable)(CLIP_DISTANCE0) };
        let uniforms = unsafe {
            Uniforms {
                mvp: (gl.uniform_location)(program, c"u_mvp".as_ptr()),
                frame_weights: (gl.uniform_location)(program, c"u_frame_weights".as_ptr()),
                color: (gl.uniform_location)(program, c"u_color".as_ptr()),
                texgen: (gl.uniform_location)(program, c"u_texgen".as_ptr()),
                alpha_value: (gl.uniform_location)(program, c"u_alpha_value".as_ptr()),
                view_origin: (gl.uniform_location)(program, c"u_view_origin".as_ptr()),
                clip_plane: (gl.uniform_location)(program, c"u_clip_plane".as_ptr()),
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
            gpu_teardown: true,
            configuration,
            width,
            height,
            program,
            material_targets: MaterialTargets::default(),
            scoped_sky: None,
            uniforms,
            static_vao: 0,
            model_vao: 0,
            static_buffers: [0; 2],
            meshes: Box::new([]),
            textures: Box::new([]),
            texture_params: Box::new([]),
            lightmap_updates,
            admission: None,
            samplers: [0; 8],
            table_buffer: 0,
            table_texture: 0,
            evaluator: StageEvaluator::load(),
            sky_clouds: Box::new([]),
            skies: SkyBatches::new(0, 0),
            dynamic: Dynamic {
                capacity: dynamic_vertices,
                ..Dynamic::default()
            },
            state: State::default(),
            _context_thread: PhantomData,
        };
        unsafe {
            backend.prepare_assets(assets, None)?;
            backend.load_device_assets();
            backend.material_targets.load(
                &backend.gl,
                width,
                height,
                &backend.configuration.display,
            )?;
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
            vertex_layout(gl, 0, None);
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

    /// Releases Rust storage after losing access to this backend's context.
    /// The owning window must destroy that context to reclaim its GPU objects.
    pub fn context_lost(mut self) {
        self.gpu_teardown = false;
    }

    /// Renderer identity and GL version, captured once at context load.
    pub fn renderer_info(&self) -> (&str, &str) {
        (
            self.configuration.renderer.as_str(),
            self.configuration.version.as_str(),
        )
    }

    /// Optional verification probe. This is not called by the frame path.
    pub fn take_error(&self) -> Option<u32> {
        let error = unsafe { (self.gl.get_error)() };
        (error != 0).then_some(error)
    }

    /// The current numeric asset prefix, committed after every upload succeeds.
    pub fn admitted_assets(&self) -> Option<AssetCounts> {
        self.admission.as_ref().map(AssetAdmission::counts)
    }

    /// Cold append admission. Rejected uploads retain the complete old prefix.
    pub fn prepare_assets(
        &mut self,
        assets: &Assets,
        scene: Option<crate::scene::Limits>,
    ) -> Result<(), String> {
        if let Some(scene) = scene {
            if scene.resolved()?.vertices > self.dynamic.capacity {
                return Err(
                    "GL dynamic vertex capacity cannot grow during asset preparation".to_owned(),
                );
            }
        }
        let sky_capacity = if assets
            .materials()
            .iter()
            .any(|material| matches!(material.settings.sky, Some(Sky::Cube { .. })))
        {
            scene
                .unwrap_or_default()
                .draw_capacity()
                .ok_or("GL sky batch capacity")?
                .max(self.skies.capacity())
        } else {
            0
        };
        let Some(admission) = AssetAdmission::prepare(self.admission.as_ref(), assets)? else {
            if sky_capacity > self.skies.capacity() {
                self.skies = SkyBatches::new(assets.materials().len(), sky_capacity);
            }
            return Ok(());
        };
        let lightmap_updates = self.lightmap_updates.prepare_append(assets)?;
        self.state.vao = 0;
        self.state.texture = None;
        let _bindings = UploadBindings::new(&self.gl);
        let mut prepared = PreparedAssets::prepare(
            &self.gl,
            assets,
            &self.textures,
            &self.texture_params,
            sky_capacity,
        )?;
        std::mem::swap(&mut self.static_vao, &mut prepared.objects.static_vao);
        std::mem::swap(&mut self.model_vao, &mut prepared.objects.model_vao);
        std::mem::swap(&mut self.static_buffers, &mut prepared.objects.buffers);
        std::mem::swap(&mut self.meshes, &mut prepared.meshes);
        std::mem::swap(&mut self.textures, &mut prepared.textures);
        std::mem::swap(&mut self.texture_params, &mut prepared.texture_params);
        std::mem::swap(&mut self.sky_clouds, &mut prepared.sky_clouds);
        std::mem::swap(&mut self.skies, &mut prepared.skies);
        if let Some(lightmap_updates) = lightmap_updates {
            self.lightmap_updates = lightmap_updates;
        }
        prepared.retain_textures();
        self.admission = Some(admission);
        Ok(())
    }

    unsafe fn load_device_assets(&mut self) {
        let gl = &self.gl;
        unsafe {
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
    }

    /// Consumes the same sealed packet as the software backend. This does not
    /// swap the SDL window; platform presents after the backend has finished.
    #[expect(
        clippy::collapsible_if,
        reason = "The explicit post-HUD palette pass separates view-command selection from that view's phase and bounds validation."
    )]
    pub fn render(&mut self, list: &CommandList, assets: &Assets) -> BackendStats {
        let mut stats = BackendStats {
            rejected: list.rejected.min(u32::MAX as u64) as u32,
            ..BackendStats::default()
        };
        unsafe { (self.gl.use_program)(self.program) };
        if !self.dynamic.begin(&self.gl) {
            stats.rejected = stats.rejected.saturating_add(1);
        }
        let mut inputs = DrawInputs::default();
        let mut lightstyles = [crate::scene::LightStyle::default(); 256];
        for command in list.commands() {
            match *command {
                Command::Empty => {}
                Command::Clear(color) => self.clear(color),
                Command::View(view) => {
                    let Some(tangent) = view.refdef.validate(self.width, self.height) else {
                        stats.rejected = stats.rejected.saturating_add(1);
                        continue;
                    };
                    stats.views = stats.views.saturating_add(1);
                    stats.pending_lights =
                        stats.pending_lights.saturating_add(view.scene.lights.count);
                    self.viewport(view.refdef.viewport, true);
                    self.depth_state(true, true);
                    unsafe { (self.gl.clear)(DEPTH_BUFFER_BIT) };
                    if view.refdef.hyperspace {
                        let color = crate::stage::unit_rgba(view.refdef.hyperspace_color());
                        unsafe {
                            (self.gl.clear_color)(color[0], color[1], color[2], color[3]);
                            (self.gl.clear)(COLOR_BUFFER_BIT);
                        }
                        continue;
                    }
                    let projection = view_projection(view.refdef, tangent);
                    inputs = DrawInputs::for_view(&view.refdef);
                    lightstyles = view.refdef.lightstyles;
                    self.collect_skies(
                        list,
                        view.scene.draws,
                        assets,
                        inputs.view_origin,
                        &mut stats,
                    );
                    for &item in list.draws(view.scene.draws) {
                        let opacity = crate::scene::draw_entity(list, item)
                            .and_then(|entity| entity.material_opacity);
                        if opacity.is_some_and(|opacity| opacity.value() == 0.) {
                            continue;
                        }
                        let before = stats.rejected;
                        if opacity.is_some() {
                            self.material_targets.begin(
                                &self.gl,
                                view.refdef.viewport,
                                self.height,
                            );
                            self.scoped_sky = crate::sky::item_material(item, list, assets)
                                .filter(|&id| {
                                    assets.material(id).is_some_and(|material| {
                                        matches!(material.settings.sky, Some(Sky::Cube { .. }))
                                    })
                                })
                                .map(|_| {
                                    crate::sky::submission_bounds(
                                        list,
                                        item,
                                        assets,
                                        view.refdef.origin,
                                    )
                                });
                        }
                        self.draw_item(
                            item,
                            list,
                            assets,
                            &projection,
                            inputs,
                            &view.refdef,
                            &mut stats,
                        );
                        self.scoped_sky = None;
                        if let Some(opacity) = opacity {
                            self.material_targets.finish(
                                &self.gl,
                                opacity,
                                before == stats.rejected,
                                self.static_vao,
                            );
                            // The composite program owns only color. Reestablish the
                            // ordinary executor's context and invalidate touched caches.
                            unsafe {
                                (self.gl.use_program)(self.program);
                                (self.gl.depth_range)(0., 1.);
                            }
                            self.state = State {
                                vao: u32::MAX,
                                texture: None,
                                sampler: u32::MAX,
                                ..State::default()
                            };
                        }
                    }
                    for particle in list.particles(view.scene.particles) {
                        self.particle(
                            *particle,
                            &view.refdef,
                            assets,
                            &projection,
                            inputs,
                            &mut stats,
                        );
                    }
                    if view.refdef.blend_phase == BlendPhase::AfterView {
                        self.tint(view.refdef.viewport, view.refdef.blend, &mut stats);
                    }
                    self.damage_vignette(&view.refdef, &mut stats);
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
                    if !self.draw_2d(draw, assets, inputs, &lightstyles) {
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
                    && view.refdef.validate(self.width, self.height).is_some()
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
        self.dynamic.end(&self.gl);
        stats
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "One draw consumes validated view and copied scene inputs"
    )]
    fn draw_item(
        &mut self,
        item: DrawItem,
        list: &CommandList,
        assets: &Assets,
        projection: &[f32; 16],
        inputs: DrawInputs,
        view: &Refdef,
        stats: &mut BackendStats,
    ) {
        let Some(shader) = crate::scene::item_shader(list, assets, item) else {
            stats.rejected = stats.rejected.saturating_add(1);
            return;
        };
        if !self.refresh_lightmap(shader, &view.lightstyles, assets) {
            stats.rejected = stats.rejected.saturating_add(1);
            return;
        }
        match item.kind {
            DrawKind::Surface => self.world_surface(
                item, shader, list, assets, projection, inputs, view.far, stats,
            ),
            DrawKind::Entity => {
                if let Some(entity) = crate::scene::draw_entity(list, item) {
                    self.entity(
                        &entity,
                        shader,
                        SkyKey::for_item(item, list, assets),
                        assets,
                        projection,
                        inputs,
                        view.far,
                        stats,
                    );
                } else {
                    stats.rejected = stats.rejected.saturating_add(1);
                }
            }
            DrawKind::Poly => {
                let poly = list.poly(item.index);
                let inputs = inputs.for_poly(poly, list);
                let Some(settings) = shader.settings(assets) else {
                    stats.rejected = stats.rejected.saturating_add(1);
                    return;
                };
                if settings.fog.is_some() {
                    stats.rejected = stats.rejected.saturating_add(1);
                    return;
                }
                if matches!(settings.sky, Some(Sky::Cube { .. })) {
                    self.draw_sky(
                        shader,
                        SkyKey::for_item(item, list, assets),
                        assets,
                        projection,
                        inputs,
                        view.far,
                        stats,
                    );
                    return;
                }
                let vertices = list.vertices(poly.vertices);
                let Some(first) = self.upload(vertices) else {
                    stats.rejected = stats.rejected.saturating_add(1);
                    return;
                };
                if !self.draw_dynamic(
                    assets,
                    shader,
                    first,
                    vertices.len(),
                    projection,
                    inputs,
                    true,
                    false,
                ) {
                    stats.rejected = stats.rejected.saturating_add(1);
                    return;
                }
                stats.triangles = stats
                    .triangles
                    .saturating_add(vertices.len().saturating_sub(2) as u32);
            }
        }
    }

    fn upload(&mut self, vertices: &[Vertex]) -> Option<usize> {
        self.upload_with(vertices.len(), |index| vertices[index])
    }

    fn upload_with(
        &mut self,
        count: usize,
        mut vertex: impl FnMut(usize) -> Vertex,
    ) -> Option<usize> {
        self.dynamic
            .upload_range(&self.gl, count, |output| {
                for (index, target) in output.iter_mut().enumerate() {
                    target.write(vertex(index));
                }
                output.len()
            })
            .map(|(first, _)| first)
    }

    fn upload_triangle_chunk(
        &mut self,
        triangles: &mut impl Iterator<Item = [u32; 3]>,
        vertices: &[WorldVertex],
    ) -> Option<(usize, usize)> {
        let capacity = self.dynamic.space(&self.gl, 3)? / 3 * 3;
        self.dynamic.upload_range(&self.gl, capacity, |output| {
            triangle_chunk(triangles, output.len(), |offset, index| {
                output[offset].write(vertices[index as usize].vertex);
            })
        })
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
        let color = crate::stage::unit_rgba(color);
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
        self.skies.begin_frame();
        crate::sky::for_each_sky_triangle(list, draws, assets, |key, submission| {
            match submission {
                crate::sky::SkySubmission::Begin(item) => {
                    if crate::scene::draw_entity(list, item)
                        .is_some_and(|entity| entity.material_opacity.is_some())
                    {
                        return false;
                    }
                    return self.sky_clip(key, assets, stats).is_some();
                }
                crate::sky::SkySubmission::Triangle(points) => {
                    if !self
                        .skies
                        .clip_mut(key)
                        .is_some_and(|clip| clip.add_polygon(&points, origin))
                    {
                        stats.rejected = stats.rejected.saturating_add(1);
                    }
                }
                crate::sky::SkySubmission::MissingSurface => {
                    stats.rejected = stats.rejected.saturating_add(1)
                }
            }
            true
        });
    }

    fn sky_clip(
        &mut self,
        key: SkyKey,
        assets: &Assets,
        stats: &mut BackendStats,
    ) -> Option<&mut SkyClip> {
        let material = assets.material(key.material())?;
        if material.settings.fog.is_some()
            || !matches!(material.settings.sky, Some(Sky::Cube { .. }))
        {
            return None;
        }
        let clip = self.skies.clip_mut(key);
        if clip.is_none() {
            stats.rejected = stats.rejected.saturating_add(1);
        }
        clip
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "A sealed sky draw carries its exact batch key with the existing projection and stage inputs"
    )]
    fn draw_sky(
        &mut self,
        shader: ResolvedShader,
        key: Option<SkyKey>,
        assets: &Assets,
        projection: &[f32; 16],
        mut inputs: DrawInputs,
        far: f32,
        stats: &mut BackendStats,
    ) {
        let Some(binding) = shader.resources(assets) else {
            stats.rejected = stats.rejected.saturating_add(1);
            return;
        };
        inputs.lightmap = binding.lightmap;
        let material_id = binding.material;
        let Some(material) = assets.material(material_id) else {
            stats.rejected = stats.rejected.saturating_add(1);
            return;
        };
        let Some(settings) = shader.settings(assets) else {
            stats.rejected = stats.rejected.saturating_add(1);
            return;
        };
        let Some(Sky::Cube {
            outer_box,
            inner_box: _,
            clouds: _,
            rotation,
            params,
        }) = settings.sky
        else {
            return;
        };
        let bounds = match self.scoped_sky.take() {
            Some(bounds) => bounds,
            None => key.and_then(|key| self.skies.take_for_draw(key)),
        };
        let Some(mut bounds) = bounds else {
            return;
        };
        if settings.fog.is_some() {
            stats.rejected = stats.rejected.saturating_add(1);
            return;
        }
        bounds = crate::sky::draw_bounds(bounds, rotation);
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
            settings,
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
            let Some(vertices) = crate::sky::box_face_vertices(
                face,
                bounds[face.index()],
                params,
                rotation,
                inputs.view_origin,
                inputs.time_seconds(),
                far,
            ) else {
                continue;
            };
            let Some(first) = self.upload(&vertices) else {
                stats.rejected = stats.rejected.saturating_add(1);
                continue;
            };
            let stage = crate::sky::box_face_stage(&images, face, params);
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
        settings: MaterialSettings,
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
        let Some(Some(grid)) = self.sky_clouds.get(material_id.0 as usize) else {
            stats.rejected = stats.rejected.saturating_add(1);
            return;
        };
        let count = crate::sky::cloud_triangle_count(bounds) * 3;
        if count == 0 {
            return;
        }
        let Some((first, written)) = self.dynamic.upload_range(&self.gl, count, |output| {
            let mut written = 0;
            for triangle in crate::sky::cloud_triangles(
                bounds,
                grid,
                params.distance.value(far),
                inputs.view_origin,
            ) {
                for vertex in triangle {
                    output[written].write(vertex);
                    written += 1;
                }
            }
            written
        }) else {
            stats.rejected = stats.rejected.saturating_add(1);
            return;
        };
        self.bind_vao(self.dynamic.vao);
        for &stage in &material.stages {
            // The shared load-generated cloud grid already supplies this
            // generator's coordinates, like native tess.texCoords[0].
            let stage = crate::sky::cloud_stage(stage);
            if !self.stage(stage, settings, inputs, projection, true, false) {
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

    #[expect(
        clippy::too_many_arguments,
        reason = "A mesh draw consumes its sealed shader alongside copied view and entity inputs."
    )]
    fn entity(
        &mut self,
        entity: &SceneEntity,
        shader: ResolvedShader,
        key: Option<SkyKey>,
        assets: &Assets,
        projection: &[f32; 16],
        mut inputs: DrawInputs,
        far: f32,
        stats: &mut BackendStats,
    ) {
        let Some(model_id) = entity.model.mesh() else {
            stats.rejected = stats.rejected.saturating_add(1);
            return;
        };
        let (Some(model), Some(&mesh)) =
            (assets.model(model_id), self.meshes.get(model_id.0 as usize))
        else {
            stats.rejected = stats.rejected.saturating_add(1);
            return;
        };
        let Some(frames) = model.posed_frames(entity) else {
            stats.rejected = stats.rejected.saturating_add(1);
            return;
        };
        let Some(binding) = shader.resources(assets) else {
            stats.rejected = stats.rejected.saturating_add(1);
            return;
        };
        let material_id = binding.material;
        let Some(material) = assets.material(material_id) else {
            stats.rejected = stats.rejected.saturating_add(1);
            return;
        };
        let Some(settings) = shader.settings(assets) else {
            stats.rejected = stats.rejected.saturating_add(1);
            return;
        };
        inputs.lightmap = binding.lightmap;
        if settings.fog.is_some() {
            stats.rejected = stats.rejected.saturating_add(1);
            return;
        }
        if mesh.count == 0 {
            return;
        }
        if matches!(settings.sky, Some(Sky::Cube { .. })) {
            let inputs = inputs.entity_inputs(entity, false);
            self.draw_sky(shader, key, assets, projection, inputs, far, stats);
            return;
        }
        let matrix = multiply(projection, &model_matrix(entity));
        inputs = inputs.for_entity(entity);
        self.bind_vao(self.model_vao);
        if frames.resolved_vertices() {
            let Some(first) = self.upload_with(frames.len(), |index| frames.vertex(index)) else {
                stats.rejected = stats.rejected.saturating_add(1);
                return;
            };
            // SAFETY: upload_with reserves and initializes the complete frame;
            // its relative indices are the same validated static index table.
            unsafe {
                (self.gl.bind_buffer)(ARRAY_BUFFER, self.dynamic.buffer);
                vertex_layout(&self.gl, first * size_of::<Vertex>(), None);
            }
            self.frame_weights([0.0, 1.0]);
        } else {
            let [current, previous] = frames
                .offsets()
                .map(|offset| (mesh.base_vertex as usize + offset) * size_of::<Vertex>());
            // SAFETY: registration bounds both frame spans and their indices;
            // prepare_assets bounds the complete owned static byte allocation.
            unsafe {
                (self.gl.bind_buffer)(ARRAY_BUFFER, self.static_buffers[0]);
                vertex_layout(&self.gl, current, Some(previous));
            }
            self.frame_weights(frames.weights());
        }
        self.set_depth_hack(entity.depth_hack);
        for stage in material.stages.iter() {
            if !self.stage(*stage, settings, inputs, &matrix, true, false) {
                stats.rejected = stats.rejected.saturating_add(1);
                continue;
            }
            unsafe {
                (self.gl.draw_elements)(
                    TRIANGLES,
                    mesh.count,
                    UNSIGNED_INT,
                    (mesh.first_index * size_of::<u32>()) as *const c_void,
                    0,
                );
            }
            stats.stages = stats.stages.saturating_add(1);
        }
        stats.triangles = stats.triangles.saturating_add(mesh.count as u32 / 3);
        self.set_depth_hack(false);
    }

    fn particle(
        &mut self,
        particle: SceneParticle,
        view: &Refdef,
        assets: &Assets,
        projection: &[f32; 16],
        inputs: DrawInputs,
        stats: &mut BackendStats,
    ) {
        if particle.alpha == 0 {
            return;
        }
        let Some(shader) = particle.shader.resolve(assets, &()) else {
            stats.rejected = stats.rejected.saturating_add(1);
            return;
        };
        if !self.refresh_lightmap(shader, &view.lightstyles, assets) {
            stats.rejected = stats.rejected.saturating_add(1);
            return;
        }
        let Some(vertices) = assets
            .palette(particle.palette)
            .and_then(|palette| crate::particles::gl_vertices(particle, view, palette))
        else {
            stats.rejected = stats.rejected.saturating_add(1);
            return;
        };
        let vertices = vertices.vertices();
        let Some(first) = self.upload(vertices) else {
            stats.rejected = stats.rejected.saturating_add(1);
            return;
        };
        if !self.draw_dynamic(
            assets,
            shader,
            first,
            vertices.len(),
            projection,
            inputs,
            true,
            false,
        ) {
            stats.rejected = stats.rejected.saturating_add(1);
            return;
        }
        stats.triangles = stats.triangles.saturating_add(vertices.len() as u32 - 2);
        if let Some(material) = shader
            .resources(assets)
            .and_then(|shader| assets.material(shader.material))
        {
            stats.stages = stats.stages.saturating_add(material.stages.len() as u32);
        }
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "The one surface draw uses its packet instance, static binding and view inputs."
    )]
    fn world_surface(
        &mut self,
        item: DrawItem,
        shader: ResolvedShader,
        list: &CommandList,
        assets: &Assets,
        projection: &[f32; 16],
        mut inputs: DrawInputs,
        far: f32,
        stats: &mut BackendStats,
    ) {
        let surface = list.surface(item.index);
        let Some(world) = assets.world(surface.world) else {
            stats.rejected = stats.rejected.saturating_add(1);
            return;
        };
        let Some(binding) = assets.surface_binding(surface.presentation, surface.surface) else {
            stats.rejected = stats.rejected.saturating_add(1);
            return;
        };
        let Some(shader_binding) = shader.resources(assets) else {
            stats.rejected = stats.rejected.saturating_add(1);
            return;
        };
        let material_id = shader_binding.material;
        let (Some(material), Some(&mesh)) = (
            assets.material(material_id),
            self.meshes.get(world.mesh.0 as usize),
        ) else {
            stats.rejected = stats.rejected.saturating_add(1);
            return;
        };
        let Some(settings) = shader.settings(assets) else {
            stats.rejected = stats.rejected.saturating_add(1);
            return;
        };
        if settings.fog.is_some() {
            stats.rejected = stats.rejected.saturating_add(1);
            return;
        }
        if binding.mesh_indices.count == 0 {
            return;
        }
        if let Some(Sky::Cube { .. }) = settings.sky {
            let inputs = surface.entity.map_or(inputs, |index| {
                inputs.entity_inputs(list.entity(index), false)
            });
            self.draw_sky(
                shader,
                SkyKey::for_item(item, list, assets),
                assets,
                projection,
                inputs,
                far,
                stats,
            );
            stats.surfaces = stats.surfaces.saturating_add(1);
            return;
        }
        let entity = surface.entity.map(|index| list.entity(index));
        let matrix = entity.map(|entity| multiply(projection, &model_matrix(entity)));
        let projection = matrix.as_ref().unwrap_or(projection);
        if let Some(entity) = entity {
            inputs = inputs.for_entity(entity);
        }
        inputs.lightmap = shader_binding.lightmap;
        inputs.texture_scale = binding.texture_scale;
        self.set_depth_hack(entity.is_some_and(|entity| entity.depth_hack));
        let geometry = world.geometry();
        let source = &geometry.surfaces[surface.surface as usize];
        match surface_draw(
            source.patch.as_ref(),
            source.vertices.first,
            surface.patch_lod,
            binding.mesh_indices,
        ) {
            SurfaceDraw::Static(indices) => {
                self.bind_vao(self.static_vao);
                self.surface_stages(
                    &material.stages,
                    settings,
                    inputs,
                    projection,
                    stats,
                    |gl| unsafe {
                        (gl.draw_elements)(
                            TRIANGLES,
                            indices.count as i32,
                            UNSIGNED_INT,
                            ((mesh.first_index + indices.first as usize) * size_of::<u32>())
                                as *const c_void,
                            mesh.base_vertex,
                        );
                    },
                );
                stats.triangles = stats.triangles.saturating_add(indices.count / 3);
            }
            SurfaceDraw::Patch(triangles) => {
                let mut triangles = triangles.peekable();
                while triangles.peek().is_some() {
                    let Some((first, count)) =
                        self.upload_triangle_chunk(&mut triangles, &geometry.vertices)
                    else {
                        stats.rejected = stats.rejected.saturating_add(1);
                        break;
                    };
                    self.bind_vao(self.dynamic.vao);
                    self.surface_stages(
                        &material.stages,
                        settings,
                        inputs,
                        projection,
                        stats,
                        |gl| unsafe {
                            (gl.draw_arrays)(TRIANGLES, first as i32, count as i32);
                        },
                    );
                    stats.triangles = stats.triangles.saturating_add((count / 3) as u32);
                }
            }
        }
        stats.surfaces = stats.surfaces.saturating_add(1);
        self.set_depth_hack(false);
    }

    fn surface_stages(
        &mut self,
        stages: &[Stage],
        settings: MaterialSettings,
        inputs: DrawInputs,
        projection: &[f32; 16],
        stats: &mut BackendStats,
        mut draw: impl FnMut(&Gl),
    ) {
        for stage in stages {
            if !self.stage(*stage, settings, inputs, projection, true, false) {
                stats.rejected = stats.rejected.saturating_add(1);
                continue;
            }
            draw(&self.gl);
            stats.stages = stats.stages.saturating_add(1);
        }
    }

    fn refresh_lightmap(
        &mut self,
        shader: ResolvedShader,
        lightstyles: &[crate::scene::LightStyle; 256],
        assets: &Assets,
    ) -> bool {
        let Some(binding) = shader.resources(assets) else {
            return false;
        };
        let Some((presentation, surface)) = binding.styled_surface else {
            return true;
        };
        let (gl, state, textures) = (&self.gl, &mut self.state, &self.textures);
        self.lightmap_updates
            .refresh(
                presentation,
                surface,
                lightstyles,
                assets,
                |image, region, pixels| {
                    let Some(&texture) = textures.get(image.0 as usize) else {
                        return false;
                    };
                    state.bind_texture(gl, texture);
                    // SAFETY: The registered region is within this owned texture;
                    // pixels are a complete RGBA rectangle in load-sized scratch.
                    // RGBA rows are four-byte multiples, retaining the context's
                    // existing unpack alignment without changing its pixel state.
                    unsafe {
                        (gl.texture_sub_image)(
                            TEXTURE_2D,
                            0,
                            region.x as i32,
                            region.y as i32,
                            region.width as i32,
                            region.height as i32,
                            RGBA,
                            UNSIGNED_BYTE,
                            pixels.as_ptr().cast(),
                        );
                    }
                    true
                },
            )
            .is_ok()
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_dynamic(
        &mut self,
        assets: &Assets,
        shader: ResolvedShader,
        first: usize,
        count: usize,
        matrix: &[f32; 16],
        mut inputs: DrawInputs,
        depth: bool,
        force_alpha: bool,
    ) -> bool {
        let Some(binding) = shader.resources(assets) else {
            return false;
        };
        let Some(material) = assets.material(binding.material) else {
            return false;
        };
        let Some(settings) = shader.settings(assets) else {
            return false;
        };
        inputs.lightmap = binding.lightmap;
        if settings.fog.is_some() {
            return false;
        }
        self.bind_vao(self.dynamic.vao);
        self.set_depth_hack(false);
        let mut complete = true;
        for stage in material.stages.iter() {
            if !self.stage(*stage, settings, inputs, matrix, depth, force_alpha) {
                complete = false;
                continue;
            }
            unsafe { (self.gl.draw_arrays)(TRIANGLE_FAN, first as i32, count as i32) };
        }
        complete
    }

    fn draw_2d(
        &mut self,
        draw: Draw2d,
        assets: &Assets,
        mut inputs: DrawInputs,
        lightstyles: &[crate::scene::LightStyle; 256],
    ) -> bool {
        let Some(shader) = draw.shader.resolve(assets, &()) else {
            return false;
        };
        if !self.refresh_lightmap(shader, lightstyles, assets) {
            return false;
        }
        let vertices = quad(draw.rect, draw.texcoords, draw.color);
        let Some(first) = self.upload(&vertices) else {
            return false;
        };
        let projection = orthographic(self.width, self.height);
        inputs.entity_color = draw.color;
        self.draw_dynamic(
            assets,
            shader,
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
            blend: Some(StageBlend::ALPHA),
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

    fn damage_vignette(&mut self, view: &crate::scene::Refdef, stats: &mut BackendStats) {
        let Some(vignette) = crate::scene::DamageVignette::load(view) else {
            return;
        };
        let (vertices, count) = vignette.triangles(view.viewport);
        let Some(first) = self.upload(&vertices[..count]) else {
            stats.rejected = stats.rejected.saturating_add(1);
            return;
        };
        self.viewport(
            Viewport {
                width: self.width,
                height: self.height,
                ..Viewport::default()
            },
            false,
        );
        self.bind_vao(self.dynamic.vao);
        self.set_depth_hack(false);
        let stage = Stage {
            rgb_gen: RgbGen::Vertex,
            alpha_gen: AlphaGen::Vertex,
            blend: Some(StageBlend::ALPHA),
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
        unsafe { (self.gl.draw_arrays)(TRIANGLES, first as i32, count as i32) };
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
            Some(StageBlend::ALPHA)
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
        let cull = if depth {
            settings.cull.for_view(prepared.inputs.reflected_view)
        } else {
            Cull::None
        };
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
        self.state.bind_texture(&self.gl, texture);
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
                [
                    prepared.uniform_alpha as f32,
                    portal,
                    prepared.float_alpha.unwrap_or(0.0),
                    u8::from(prepared.float_alpha.is_some()) as f32,
                ]
                .as_ptr(),
            );
            (self.gl.uniform_color)(
                self.uniforms.view_origin,
                1,
                vec4(inputs.view_origin).as_ptr(),
            );
            let clip = inputs.clip_plane.map_or([0., 0., 0., 1.], |plane| {
                [
                    plane.normal.0[0],
                    plane.normal.0[1],
                    plane.normal.0[2],
                    -plane.distance,
                ]
            });
            (self.gl.uniform_color)(self.uniforms.clip_plane, 1, clip.as_ptr());
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
        if vao != self.model_vao {
            self.frame_weights([0.0, 1.0]);
        }
        if self.state.vao != vao {
            unsafe { (self.gl.bind_vertex_array)(vao) };
            self.state.vao = vao;
        }
    }
    fn frame_weights(&mut self, weights: [f32; 2]) {
        if self.state.frame_weights != Some(weights) {
            unsafe {
                (self.gl.uniform_color)(
                    self.uniforms.frame_weights,
                    1,
                    [weights[0], weights[1], 0.0, 0.0].as_ptr(),
                );
            }
            self.state.frame_weights = Some(weights);
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
        if !self.gpu_teardown {
            return;
        }
        self.material_targets.delete(&self.gl);
        StaticObjects {
            static_vao: self.static_vao,
            model_vao: self.model_vao,
            buffers: self.static_buffers,
        }
        .delete(&self.gl);
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
            (self.gl.delete_buffers)(1, &self.dynamic.buffer);
            (self.gl.delete_vertex_arrays)(1, &self.dynamic.vao);
            (self.gl.delete_textures)(self.textures.len() as i32, self.textures.as_ptr());
            (self.gl.delete_buffers)(1, &self.table_buffer);
            (self.gl.delete_textures)(1, &self.table_texture);
            (self.gl.delete_samplers)(8, self.samplers.as_ptr());
            (self.gl.delete_program)(self.program);
        }
    }
}

unsafe fn vertex_layout(gl: &Gl, current: usize, previous: Option<usize>) {
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
            (current + offset_of!(Vertex, position)) as *const c_void,
        );
        (gl.vertex_attribute)(
            1,
            2,
            FLOAT,
            0,
            stride,
            (current + offset_of!(Vertex, texcoord)) as *const c_void,
        );
        (gl.vertex_attribute)(
            2,
            2,
            FLOAT,
            0,
            stride,
            (current + offset_of!(Vertex, lightmap_coord)) as *const c_void,
        );
        (gl.vertex_attribute)(
            3,
            4,
            UNSIGNED_BYTE,
            1,
            stride,
            (current + offset_of!(Vertex, color)) as *const c_void,
        );
        (gl.vertex_attribute)(
            4,
            3,
            FLOAT,
            0,
            stride,
            (current + offset_of!(Vertex, normal)) as *const c_void,
        );
        if let Some(previous) = previous {
            for (index, offset) in [
                (5, offset_of!(Vertex, position)),
                (6, offset_of!(Vertex, normal)),
            ] {
                (gl.enable_vertex_attribute)(index);
                (gl.vertex_attribute)(
                    index,
                    3,
                    FLOAT,
                    0,
                    stride,
                    (previous + offset) as *const c_void,
                );
            }
        }
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
    unsafe { compile_program_sources(gl, VERTEX_SHADER, FRAGMENT_SHADER) }
}
unsafe fn compile_program_sources(
    gl: &Gl,
    vertex_source: &str,
    fragment_source: &str,
) -> Result<u32, String> {
    let vertex = unsafe { compile_shader(gl, 0x8b31, vertex_source)? };
    let fragment = match unsafe { compile_shader(gl, 0x8b30, fragment_source) } {
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

fn view_projection(view: Refdef, tangent: [f32; 2]) -> [f32; 16] {
    let [x, y] = crate::scene::projection_scales(tangent);
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

#[cfg(test)]
#[path = "../tests/gl_math/oracle.rs"]
mod tests;
