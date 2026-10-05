//! Stage program selection and caching (donor `src/render/gl/programs.ts`).

use std::collections::HashMap;

use super::shadow_shader::{shadow_factor_lines, ShadowReceiver};
use super::{GlContext, COMPILE_STATUS, FRAGMENT_SHADER, LINK_STATUS, VERTEX_SHADER};
use crate::render::error::RenderError;
use crate::render::types::{
    AlphaTest, BatchFog, BatchLighting, FogEffect, PairEnvironment, Q2LightPass, Q2ShadowProjection,
};

pub const STAGE_VERTEX_SHADER: &str = "#version 120
varying vec4 vertexColor;
varying vec2 coordinates0;
varying vec2 coordinates1;
varying vec3 worldPosition;
varying vec3 worldNormal;
void main() {
  gl_Position = gl_ModelViewProjectionMatrix * gl_Vertex;
  gl_ClipVertex = gl_ModelViewMatrix * gl_Vertex;
  vertexColor = clamp(gl_Color, 0.0, 1.0);
  coordinates0 = gl_MultiTexCoord0.xy;
  coordinates1 = gl_MultiTexCoord1.xy;
  worldPosition = gl_MultiTexCoord2.xyz;
  worldNormal = gl_MultiTexCoord3.xyz;
}
";

const STAGE_FRAGMENT_HEAD: &str = "#version 120
uniform sampler2D primaryTexture;
uniform sampler2D secondaryTexture;
uniform int secondaryMode;
uniform int alphaMode;
uniform int u_fog_mode;
uniform vec3 u_fog_color;
uniform float u_fog_amount;
varying vec4 vertexColor;
varying vec2 coordinates0;
varying vec2 coordinates1;
varying vec3 worldPosition;
varying vec3 worldNormal;
uniform int u_lighting_mode;
uniform int u_luminance_alpha;
uniform int u_light_count;
uniform sampler2D u_shadow_map;
uniform float u_shadow_texel;
uniform float u_shadow_near;
uniform vec3 u_light_pos[8];
uniform float u_light_radius[8];
uniform vec3 u_light_color[8];
uniform float u_light_scale[8];
uniform vec3 u_light_cone_dir[8];
uniform float u_light_cone_cos[8];
uniform mat4 u_light_matrix[8];
uniform vec4 u_light_atlas[8];
uniform float u_light_shadow[8];
uniform vec3 u_light_frac[8];
uniform float u_shade_scale;
#define SHADOW_NEAR u_shadow_near
#define SHADOW_CUBE_COLS 3.0
#define SHADOW_CUBE_ROWS 2.0
vec3 dynamicLights() {
  vec3 shade = vec3(0.0);
  for (int i = 0; i < 8; i++) {
    if (i >= u_light_count) break;
    if (u_light_scale[i] == 0.0 || all(equal(u_light_color[i], vec3(0.0)))) continue;
    vec3 lightPosition = u_light_pos[i];
    float cone = u_light_cone_cos[i];
    if (cone == 0.0) lightPosition += worldNormal * 16.0;
    vec3 towardLight = lightPosition - worldPosition;
    float distance = length(towardLight);
    float radius = u_light_radius[i] + 64.0;
    float falloff = max(radius - distance - 64.0, 0.0) / radius;
    vec3 direction = towardLight / max(distance, 1.0);
    float lambert = u_light_color[i].r < 0.0 ? 1.0 : max(dot(worldNormal, direction), 0.0);
    vec3 result = u_light_color[i] * u_light_scale[i] * falloff * lambert;
    if (cone != 0.0) {
      float magnitude = -dot(direction, u_light_cone_dir[i]);
      result *= cone >= 1.0 ? 0.0 : max(1.0 - (1.0 - magnitude) / (1.0 - cone), 0.0);
    }
    if (u_light_shadow[i] != 0.0) {
";

const STAGE_FRAGMENT_MID: &str = "      result *= lit;
    }
    shade += result;
  }
  return shade;
}
vec4 modelShadow(vec4 texel) {
  vec3 shade = vertexColor.rgb * u_shade_scale;
  vec3 keep = vec3(1.0);
  for (int i = 0; i < 8; i++) {
    if (i >= u_light_count) break;
    if (all(equal(u_light_frac[i], vec3(0.0)))) continue;
    if (u_light_shadow[i] != 0.0) {
";

const STAGE_FRAGMENT_TAIL: &str = "      keep -= u_light_frac[i] * (1.0 - lit);
    }
  }
  shade *= max(keep, vec3(0.0));
  return vec4(texel.rgb * min(shade, vec3(1.0)), texel.a * vertexColor.a);
}
void main() {
  vec4 texel = texture2D(primaryTexture, coordinates0);
  if (u_luminance_alpha != 0) texel.rgb *= (texel.r + texel.g + texel.b) / 3.0 * vertexColor.a;
  vec4 color = clamp(texel * vertexColor, 0.0, 1.0);
  if (u_lighting_mode == 1) color = vec4(texel.rgb + dynamicLights(), 1.0);
  else if (u_lighting_mode == 2) color.rgb += dynamicLights();
  else if (u_lighting_mode == 3) color = modelShadow(texel);
  else if (u_lighting_mode == 4) color = vec4((texel.rgb + dynamicLights()) * vertexColor.rgb, texel.a * vertexColor.a);
  else if (u_lighting_mode == 5) {
    color = u_shade_scale > 0.0 ? modelShadow(texel) : texel * vertexColor;
    color.rgb += texel.rgb * dynamicLights();
  }
  if (secondaryMode != 0) {
    vec4 second = texture2D(secondaryTexture, coordinates1);
    if (secondaryMode == 1) color *= second;
    else if (secondaryMode == 2) color = vec4(color.rgb + second.rgb, color.a * second.a);
    else color = second;
    color = clamp(color, 0.0, 1.0);
  }
  float fogAmount = u_fog_mode == 2 ? u_fog_amount : 0.0;
  if (u_fog_mode != 0 && u_fog_mode != 2) {
    float distance = u_fog_amount / (64.0 * gl_FragCoord.w);
    fogAmount = 1.0 - exp(-distance * distance);
  }
  if (u_fog_mode != 0) color.rgb = clamp(color.rgb, 0.0, 1.0);
  if (u_fog_mode == 1 || u_fog_mode == 2) color.rgb = mix(color.rgb, u_fog_color, fogAmount);
  if (u_fog_mode == 3 || u_fog_mode == 5) color.rgb *= 1.0 - fogAmount;
  if (u_fog_mode == 4 || u_fog_mode == 5) color.a *= 1.0 - fogAmount;
  if (u_fog_mode == 6) color = vec4(u_fog_color, color.a * fogAmount);
  if (alphaMode == 1 && color.a <= 0.0) discard;
  if (alphaMode == 2 && color.a >= 0.5) discard;
  if (alphaMode == 3 && color.a < 0.5) discard;
  gl_FragColor = color;
}
";

/// Full stage fragment source with the shadow receiver snippets spliced in.
#[must_use]
pub fn stage_fragment_shader() -> String {
    let mut source = String::from(STAGE_FRAGMENT_HEAD);
    source.push_str(&shadow_factor_lines(ShadowReceiver::World).join("\n"));
    source.push('\n');
    source.push_str(STAGE_FRAGMENT_MID);
    source.push_str(&shadow_factor_lines(ShadowReceiver::Model).join("\n"));
    source.push('\n');
    source.push_str(STAGE_FRAGMENT_TAIL);
    source
}

pub const DEPTH_VERTEX_SHADER: &str = "#version 120
void main() { gl_Position = gl_Vertex; }
";

pub const DEPTH_FRAGMENT_SHADER: &str = "#version 120
void main() { gl_FragColor = vec4(1.0); }
";

const fn alpha_mode(test: AlphaTest) -> i32 {
    match test {
        AlphaTest::None => 0,
        AlphaTest::GreaterZero => 1,
        AlphaTest::Less128 => 2,
        AlphaTest::GreaterEqual128 => 3,
    }
}

const fn secondary_mode(environment: PairEnvironment) -> i32 {
    match environment {
        PairEnvironment::Modulate => 1,
        PairEnvironment::Add => 2,
        PairEnvironment::Replace => 3,
    }
}

const fn fog_mode(fog: Option<BatchFog>) -> i32 {
    match fog {
        None => 0,
        Some(BatchFog::Constant { .. }) => 2,
        Some(BatchFog::Exp2 { effect, .. }) => match effect {
            FogEffect::None => 0,
            FogEffect::Rgb => 3,
            FogEffect::Alpha => 4,
            FogEffect::Rgba => 5,
            FogEffect::Overlay => 6,
            FogEffect::Color => 1,
        },
    }
}

fn finite_uniforms(values: &[f32]) {
    if !values.iter().all(|value| value.is_finite()) {
        panic!(
            "{}",
            RenderError::NotFinite("OpenGL lighting uniforms must be finite float32 values".to_string())
        );
    }
}

/// Compile and link `vertex`/`fragment`; shaders are always deleted.
pub fn compile_program<C: GlContext>(gl: &mut C, vertex: &str, fragment: &str) -> u32 {
    let mut shaders = Vec::new();
    let build = |gl: &mut C, shaders: &mut Vec<u32>| -> u32 {
        for (kind, source) in [(VERTEX_SHADER, vertex), (FRAGMENT_SHADER, fragment)] {
            let shader = gl.create_shader(kind);
            if shader == 0 {
                panic!(
                    "{}",
                    RenderError::Backend("OpenGL could not allocate a shader".to_string())
                );
            }
            shaders.push(shader);
            gl.shader_source(shader, source);
            gl.compile_shader(shader);
            if gl.get_shaderiv(shader, COMPILE_STATUS) != 1 {
                let mut log = vec![0u8; 8192];
                let len = gl.get_shader_info_log(shader, &mut log);
                let text = String::from_utf8_lossy(&log[..len]).into_owned();
                panic!(
                    "{}",
                    RenderError::Backend(format!("OpenGL shader compilation failed: {text}"))
                );
            }
        }
        let linked = gl.create_program();
        if linked == 0 {
            panic!(
                "{}",
                RenderError::Backend("OpenGL could not allocate a program".to_string())
            );
        }
        for shader in shaders.iter() {
            gl.attach_shader(linked, *shader);
        }
        gl.link_program(linked);
        if gl.get_programiv(linked, LINK_STATUS) != 1 {
            let mut log = vec![0u8; 8192];
            let len = gl.get_program_info_log(linked, &mut log);
            let text = String::from_utf8_lossy(&log[..len]).into_owned();
            gl.delete_program(linked);
            panic!(
                "{}",
                RenderError::Backend(format!("OpenGL program linking failed: {text}"))
            );
        }
        linked
    };
    let program = build(gl, &mut shaders);
    for shader in shaders {
        gl.delete_shader(shader);
    }
    program
}

/// Retained-geometry vertex shader: same varyings as the stage shader, but
/// object-space positions transform by the `u_mvp` uniform instead of the
/// fixed-function matrices (which stay identity for projected batches).
pub const RETAINED_VERTEX_SHADER: &str = "#version 120\nuniform mat4 u_mvp;\nvarying vec4 vertexColor;\nvarying vec2 coordinates0;\nvarying vec2 coordinates1;\nvarying vec3 worldPosition;\nvarying vec3 worldNormal;\nvoid main() {\n  gl_Position = u_mvp * gl_Vertex;\n  gl_ClipVertex = u_mvp * gl_Vertex;\n  vertexColor = clamp(gl_Color, 0.0, 1.0);\n  coordinates0 = gl_MultiTexCoord0.xy;\n  coordinates1 = gl_MultiTexCoord1.xy;\n  worldPosition = gl_MultiTexCoord2.xyz;\n  worldNormal = gl_MultiTexCoord3.xyz;\n}\n";

/// Look up and cache one program uniform location.
fn program_uniform<C: GlContext>(gl: &mut C, program: u32, uniforms: &mut HashMap<String, i32>, name: &str) -> i32 {
    if let Some(location) = uniforms.get(name) {
        return *location;
    }
    let location = gl.get_uniform_location(program, name);
    if location < 0 {
        panic!(
            "{}",
            RenderError::Backend(format!("OpenGL stage uniform is missing: {name}"))
        );
    }
    uniforms.insert(name.to_string(), location);
    location
}

/// Bind the stage fragment sampler uniforms to their texture units.
fn bind_stage_samplers<C: GlContext>(gl: &mut C, program: u32, uniforms: &mut HashMap<String, i32>) {
    gl.use_program(program);
    let primary = program_uniform(gl, program, uniforms, "primaryTexture");
    gl.uniform_1i(primary, 0);
    let secondary = program_uniform(gl, program, uniforms, "secondaryTexture");
    gl.uniform_1i(secondary, 1);
    let shadow = program_uniform(gl, program, uniforms, "u_shadow_map");
    gl.uniform_1i(shadow, 2);
    gl.use_program(0);
}

/// Compiled stage program with uniform caches. The retained-geometry
/// program shares the stage fragment shader but needs its own uniform
/// locations (locations are per-program); value caches flush on program
/// switches so dedup never crosses programs.
#[derive(Debug)]
pub struct StageProgram {
    program: u32,
    depth_program: u32,
    retained_program: Option<u32>,
    uniforms: HashMap<String, i32>,
    retained_uniforms: HashMap<String, i32>,
    values_program: Option<u32>,
    integers: HashMap<i32, i32>,
    scalars: HashMap<i32, u32>,
    vectors3: HashMap<i32, [u32; 3]>,
    vectors4: HashMap<i32, [u32; 4]>,
    matrices: HashMap<i32, [u32; 16]>,
    closed: bool,
}

impl StageProgram {
    pub fn new<C: GlContext>(gl: &mut C) -> Self {
        let program = compile_program(gl, STAGE_VERTEX_SHADER, &stage_fragment_shader());
        let depth_program = compile_program(gl, DEPTH_VERTEX_SHADER, DEPTH_FRAGMENT_SHADER);
        let mut stage = Self {
            program,
            depth_program,
            retained_program: None,
            uniforms: HashMap::new(),
            retained_uniforms: HashMap::new(),
            values_program: None,
            integers: HashMap::new(),
            scalars: HashMap::new(),
            vectors3: HashMap::new(),
            vectors4: HashMap::new(),
            matrices: HashMap::new(),
            closed: false,
        };
        bind_stage_samplers(gl, program, &mut stage.uniforms);
        stage
    }

    #[must_use]
    pub fn program(&self) -> u32 {
        self.program
    }

    fn uniform<C: GlContext>(&mut self, gl: &mut C, name: &str) -> i32 {
        let program = self
            .values_program
            .expect("stage program is selected before uniform lookup");
        let locations = if Some(program) == self.retained_program {
            &mut self.retained_uniforms
        } else {
            &mut self.uniforms
        };
        program_uniform(gl, program, locations, name)
    }

    /// Bind one stage program, flushing value caches on switches.
    fn select<C: GlContext>(&mut self, gl: &mut C, program: u32) {
        if self.values_program != Some(program) {
            self.integers.clear();
            self.scalars.clear();
            self.vectors3.clear();
            self.vectors4.clear();
            self.matrices.clear();
            self.values_program = Some(program);
        }
        gl.use_program(program);
    }

    fn integer<C: GlContext>(&mut self, gl: &mut C, name: &str, value: i32) {
        let location = self.uniform(gl, name);
        if self.integers.get(&location) == Some(&value) {
            return;
        }
        gl.uniform_1i(location, value);
        self.integers.insert(location, value);
    }

    fn scalar<C: GlContext>(&mut self, gl: &mut C, name: &str, value: f32) {
        let location = self.uniform(gl, name);
        if self.scalars.get(&location) == Some(&value.to_bits()) {
            return;
        }
        gl.uniform_1f(location, value);
        self.scalars.insert(location, value.to_bits());
    }

    fn vector3<C: GlContext>(&mut self, gl: &mut C, name: &str, x: f32, y: f32, z: f32) {
        let location = self.uniform(gl, name);
        let bits = [x.to_bits(), y.to_bits(), z.to_bits()];
        if self.vectors3.get(&location) == Some(&bits) {
            return;
        }
        gl.uniform_3f(location, x, y, z);
        self.vectors3.insert(location, bits);
    }

    fn vector4<C: GlContext>(&mut self, gl: &mut C, name: &str, x: f32, y: f32, z: f32, w: f32) {
        let location = self.uniform(gl, name);
        let bits = [x.to_bits(), y.to_bits(), z.to_bits(), w.to_bits()];
        if self.vectors4.get(&location) == Some(&bits) {
            return;
        }
        gl.uniform_4f(location, x, y, z, w);
        self.vectors4.insert(location, bits);
    }

    fn matrix<C: GlContext>(&mut self, gl: &mut C, name: &str, value: &[f32; 16]) {
        let location = self.uniform(gl, name);
        let bits = value.map(f32::to_bits);
        if self.matrices.get(&location) == Some(&bits) {
            return;
        }
        gl.uniform_matrix_4fv(location, value);
        self.matrices.insert(location, bits);
    }

    #[allow(clippy::too_many_arguments)]
    pub fn use_stage<C: GlContext>(
        &mut self,
        gl: &mut C,
        environment: Option<PairEnvironment>,
        alpha_test: AlphaTest,
        lighting: &BatchLighting,
        luminance_alpha: bool,
        fog: Option<BatchFog>,
    ) {
        if self.closed {
            panic!("{}", RenderError::Backend("OpenGL stage program is closed".to_string()));
        }
        let program = self.program;
        self.select(gl, program);
        self.apply_stage(gl, environment, alpha_test, lighting, luminance_alpha, fog);
    }

    /// Bind the retained-geometry program, compiling it on first use.
    pub fn use_retained<C: GlContext>(&mut self, gl: &mut C) {
        if self.closed {
            panic!("{}", RenderError::Backend("OpenGL stage program is closed".to_string()));
        }
        if self.retained_program.is_none() {
            let program = compile_program(gl, RETAINED_VERTEX_SHADER, &stage_fragment_shader());
            bind_stage_samplers(gl, program, &mut self.retained_uniforms);
            self.retained_program = Some(program);
        }
        let program = self.retained_program.expect("retained program compiled before use");
        self.select(gl, program);
    }

    /// Apply the MVP plus stage uniforms to the bound retained program.
    #[allow(clippy::too_many_arguments)]
    pub fn apply_retained<C: GlContext>(
        &mut self,
        gl: &mut C,
        mvp: &[f32; 16],
        environment: Option<PairEnvironment>,
        alpha_test: AlphaTest,
        lighting: &BatchLighting,
        luminance_alpha: bool,
        fog: Option<BatchFog>,
    ) {
        if self.closed {
            panic!("{}", RenderError::Backend("OpenGL stage program is closed".to_string()));
        }
        if self.values_program != self.retained_program {
            panic!(
                "{}",
                RenderError::Backend("OpenGL retained program is bound before use".to_string())
            );
        }
        self.matrix(gl, "u_mvp", mvp);
        self.apply_stage(gl, environment, alpha_test, lighting, luminance_alpha, fog);
    }

    /// Apply stage uniforms to the currently bound program. The retained
    /// program shares the stage fragment shader, so retained draws bind
    /// their own program and reuse this for identical fragment shading.
    #[allow(clippy::too_many_arguments)]
    pub fn apply_stage<C: GlContext>(
        &mut self,
        gl: &mut C,
        environment: Option<PairEnvironment>,
        alpha_test: AlphaTest,
        lighting: &BatchLighting,
        luminance_alpha: bool,
        fog: Option<BatchFog>,
    ) {
        if self.closed {
            panic!("{}", RenderError::Backend("OpenGL stage program is closed".to_string()));
        }
        self.integer(gl, "u_fog_mode", fog_mode(fog));
        let (fr, fg, fb) = fog.map_or((0.0, 0.0, 0.0), |fog| match fog {
            BatchFog::Exp2 { color, .. } | BatchFog::Constant { color, .. } => (color.x, color.y, color.z),
        });
        self.vector3(gl, "u_fog_color", fr, fg, fb);
        let amount = fog.map_or(0.0, |fog| match fog {
            BatchFog::Exp2 { density, .. } => density,
            BatchFog::Constant { amount, .. } => amount,
        });
        self.scalar(gl, "u_fog_amount", amount);
        self.integer(gl, "secondaryMode", environment.map_or(0, secondary_mode));
        self.integer(gl, "alphaMode", alpha_mode(alpha_test));
        self.integer(gl, "u_luminance_alpha", i32::from(luminance_alpha));
        let lighting_mode = match lighting {
            BatchLighting::Vertex => 0,
            BatchLighting::Q2ModelShadow { .. } => 3,
            BatchLighting::Q2World { pass, .. } => match pass {
                Q2LightPass::Lightmap { .. } => 1,
                Q2LightPass::MaterialLightmap { .. } => 4,
                Q2LightPass::Model { .. } => 5,
                Q2LightPass::Texture { .. } => 2,
            },
        };
        self.integer(gl, "u_lighting_mode", lighting_mode);
        match lighting {
            BatchLighting::Vertex => {
                self.integer(gl, "u_light_count", 0);
            }
            BatchLighting::Q2World { atlas, pass, .. } => {
                let (count, shade_scale) = match pass {
                    Q2LightPass::Lightmap { lights }
                    | Q2LightPass::Texture { lights }
                    | Q2LightPass::MaterialLightmap { lights } => (lights.len(), None),
                    Q2LightPass::Model { lights, shade_scale } => (lights.len(), Some(shade_scale.unwrap_or(0.0))),
                };
                if count > 8 {
                    panic!(
                        "{}",
                        RenderError::BadBatch {
                            index: count,
                            detail: "Q2 fragment lighting accepts at most eight selected lights per draw".to_string()
                        }
                    );
                }
                self.integer(gl, "u_light_count", count as i32);
                if let Some(atlas) = atlas {
                    finite_uniforms(&[atlas.texel_size, atlas.near_plane]);
                    if atlas.texel_size <= 0.0 || atlas.near_plane <= 0.0 {
                        panic!(
                            "{}",
                            RenderError::BadBatch {
                                index: 0,
                                detail: "Q2 shadow atlas texel size and near plane must be positive".to_string()
                            }
                        );
                    }
                    self.scalar(gl, "u_shadow_texel", atlas.texel_size);
                    self.scalar(gl, "u_shadow_near", atlas.near_plane);
                }
                if let Some(scale) = shade_scale {
                    finite_uniforms(&[scale]);
                    self.scalar(gl, "u_shade_scale", scale);
                }
                match pass {
                    Q2LightPass::Lightmap { lights }
                    | Q2LightPass::Texture { lights }
                    | Q2LightPass::MaterialLightmap { lights } => {
                        for (index, light) in lights.iter().enumerate() {
                            self.fragment_light(gl, index, light, None, atlas.is_some());
                        }
                    }
                    Q2LightPass::Model { lights, .. } => {
                        for (index, model) in lights.iter().enumerate() {
                            self.fragment_light(
                                gl,
                                index,
                                &model.light,
                                Some([model.fraction.x, model.fraction.y, model.fraction.z]),
                                atlas.is_some(),
                            );
                        }
                    }
                }
            }
            BatchLighting::Q2ModelShadow {
                lights, shade_scale, ..
            } => {
                if lights.len() > 8 {
                    panic!(
                        "{}",
                        RenderError::BadBatch {
                            index: lights.len(),
                            detail: "Q2 fragment lighting accepts at most eight selected lights per draw".to_string()
                        }
                    );
                }
                self.integer(gl, "u_light_count", lights.len() as i32);
                finite_uniforms(&[*shade_scale]);
                self.scalar(gl, "u_shade_scale", *shade_scale);
                for (index, light) in lights.iter().enumerate() {
                    finite_uniforms(&[light.origin.x, light.origin.y, light.origin.z, light.radius]);
                    if light.radius <= 0.0 {
                        panic!(
                            "{}",
                            RenderError::BadBatch {
                                index,
                                detail: "Q2 fragment light radius must be positive".to_string()
                            }
                        );
                    }
                    self.vector3(
                        gl,
                        &format!("u_light_pos[{index}]"),
                        light.origin.x,
                        light.origin.y,
                        light.origin.z,
                    );
                    self.scalar(gl, &format!("u_light_radius[{index}]"), light.radius);
                    finite_uniforms(&[light.fraction.x, light.fraction.y, light.fraction.z]);
                    self.vector3(
                        gl,
                        &format!("u_light_frac[{index}]"),
                        light.fraction.x,
                        light.fraction.y,
                        light.fraction.z,
                    );
                    self.shadow(gl, index, &light.shadow, true);
                }
            }
        }
    }

    fn fragment_light<C: GlContext>(
        &mut self,
        gl: &mut C,
        index: usize,
        light: &crate::render::types::Q2FragmentLight,
        fraction: Option<[f32; 3]>,
        has_atlas: bool,
    ) {
        finite_uniforms(&[light.origin.x, light.origin.y, light.origin.z, light.radius]);
        if light.radius <= 0.0 {
            panic!(
                "{}",
                RenderError::BadBatch {
                    index,
                    detail: "Q2 fragment light radius must be positive".to_string()
                }
            );
        }
        self.vector3(
            gl,
            &format!("u_light_pos[{index}]"),
            light.origin.x,
            light.origin.y,
            light.origin.z,
        );
        self.scalar(gl, &format!("u_light_radius[{index}]"), light.radius);
        finite_uniforms(&[light.color.x, light.color.y, light.color.z, light.scale]);
        if let Some(cone) = &light.cone {
            finite_uniforms(&[
                cone.direction.x,
                cone.direction.y,
                cone.direction.z,
                cone.cos_half_angle,
            ]);
        }
        self.vector3(
            gl,
            &format!("u_light_color[{index}]"),
            light.color.x,
            light.color.y,
            light.color.z,
        );
        self.scalar(gl, &format!("u_light_scale[{index}]"), light.scale);
        self.scalar(
            gl,
            &format!("u_light_cone_cos[{index}]"),
            light.cone.as_ref().map_or(0.0, |cone| cone.cos_half_angle),
        );
        let (dx, dy, dz) = light.cone.as_ref().map_or((0.0, 0.0, 0.0), |cone| {
            (cone.direction.x, cone.direction.y, cone.direction.z)
        });
        self.vector3(gl, &format!("u_light_cone_dir[{index}]"), dx, dy, dz);
        match fraction {
            Some([x, y, z]) => {
                finite_uniforms(&[x, y, z]);
                self.vector3(gl, &format!("u_light_frac[{index}]"), x, y, z);
            }
            None => self.vector3(gl, &format!("u_light_frac[{index}]"), 0.0, 0.0, 0.0),
        }
        self.shadow(gl, index, &light.shadow, has_atlas);
    }

    fn shadow<C: GlContext>(&mut self, gl: &mut C, index: usize, shadow: &Q2ShadowProjection, has_atlas: bool) {
        if !matches!(shadow, Q2ShadowProjection::None) && !has_atlas {
            panic!(
                "{}",
                RenderError::Backend("Q2 shadow receiver is missing its atlas".to_string())
            );
        }
        let mode = match shadow {
            Q2ShadowProjection::None => 0.0,
            Q2ShadowProjection::Cone { .. } => 1.0,
            Q2ShadowProjection::Point { .. } => 2.0,
        };
        self.scalar(gl, &format!("u_light_shadow[{index}]"), mode);
        match shadow {
            Q2ShadowProjection::None => {}
            Q2ShadowProjection::Cone { matrix, atlas_rect } => {
                finite_uniforms(&[atlas_rect.x, atlas_rect.y, atlas_rect.z, atlas_rect.w]);
                self.vector4(
                    gl,
                    &format!("u_light_atlas[{index}]"),
                    atlas_rect.x,
                    atlas_rect.y,
                    atlas_rect.z,
                    atlas_rect.w,
                );
                finite_uniforms(matrix);
                self.matrix(gl, &format!("u_light_matrix[{index}]"), matrix);
            }
            Q2ShadowProjection::Point { atlas_rect } => {
                finite_uniforms(&[atlas_rect.x, atlas_rect.y, atlas_rect.z, atlas_rect.w]);
                self.vector4(
                    gl,
                    &format!("u_light_atlas[{index}]"),
                    atlas_rect.x,
                    atlas_rect.y,
                    atlas_rect.z,
                    atlas_rect.w,
                );
            }
        }
    }

    pub fn use_depth<C: GlContext>(&self, gl: &mut C) {
        gl.use_program(self.depth_program);
    }

    pub fn restore<C: GlContext>(&self, gl: &mut C, program: u32) {
        gl.use_program(program);
    }

    pub fn close<C: GlContext>(&mut self, gl: &mut C) {
        if self.closed {
            return;
        }
        gl.use_program(0);
        gl.delete_program(self.program);
        gl.delete_program(self.depth_program);
        if let Some(retained) = self.retained_program.take() {
            gl.delete_program(retained);
        }
        self.closed = true;
        self.retained_uniforms.clear();
        self.values_program = None;
        self.integers.clear();
        self.scalars.clear();
        self.vectors3.clear();
        self.vectors4.clear();
        self.matrices.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::super::{FakeGlContext, GlCall};
    use super::*;
    use crate::render::types::{BatchLighting, Q2LightPass};
    use qa_core::math::vec3;

    #[test]
    fn fragment_shader_embeds_both_shadow_receivers() {
        let source = stage_fragment_shader();
        assert!(source.contains("lproj.z - 0.0005"));
        assert!(source.contains("lproj.z - 0.0025"));
        assert!(source.contains("u_lighting_mode == 5"));
    }

    #[test]
    fn stage_binds_samplers_and_caches_uniforms() {
        let mut gl = FakeGlContext::new();
        let mut stage = StageProgram::new(&mut gl);
        assert_ne!(stage.program(), 0);
        gl.assert_contains("sampler", |call| matches!(call, GlCall::Uniform1i { value: 2, .. }));
        gl.clear_log();
        stage.use_stage(
            &mut gl,
            Some(PairEnvironment::Add),
            AlphaTest::GreaterZero,
            &BatchLighting::Vertex,
            true,
            None,
        );
        let first = gl.log.len();
        stage.use_stage(
            &mut gl,
            Some(PairEnvironment::Add),
            AlphaTest::GreaterZero,
            &BatchLighting::Vertex,
            true,
            None,
        );
        let cached: Vec<_> = gl.log[first..]
            .iter()
            .filter(|call| {
                matches!(
                    call,
                    GlCall::Uniform1i { .. } | GlCall::Uniform1f { .. } | GlCall::Uniform3f { .. }
                )
            })
            .collect();
        assert!(cached.is_empty(), "cached uniforms must not re-emit: {cached:?}");
        stage.close(&mut gl);
        gl.assert_contains("unbind", |call| matches!(call, GlCall::UseProgram { program: 0 }));
    }

    #[test]
    fn stage_uploads_q2_light_and_fog() {
        let mut gl = FakeGlContext::new();
        let mut stage = StageProgram::new(&mut gl);
        let lighting = BatchLighting::Q2World {
            world_positions: Vec::new(),
            normals: Vec::new(),
            atlas: None,
            pass: Q2LightPass::Texture {
                lights: vec![crate::render::types::Q2FragmentLight {
                    origin: vec3(1.0, 2.0, 3.0),
                    radius: 100.0,
                    color: vec3(1.0, 1.0, 1.0),
                    scale: 1.0,
                    cone: None,
                    shadow: Q2ShadowProjection::None,
                }],
            },
        };
        let fog = BatchFog::Exp2 {
            color: vec3(0.5, 0.5, 0.5),
            density: 0.02,
            effect: FogEffect::Color,
        };
        stage.use_stage(&mut gl, None, AlphaTest::None, &lighting, false, Some(fog));
        gl.assert_contains("light count", |call| matches!(call, GlCall::Uniform1i { value: 1, .. }));
        gl.assert_contains(
            "light pos",
            |call| matches!(call, GlCall::Uniform3f { x, .. } if *x == 1.0),
        );
        stage.close(&mut gl);
    }

    #[test]
    #[should_panic(expected = "at most eight")]
    fn rejects_ninth_light() {
        let mut gl = FakeGlContext::new();
        let mut stage = StageProgram::new(&mut gl);
        let light = crate::render::types::Q2FragmentLight {
            origin: vec3(0.0, 0.0, 0.0),
            radius: 10.0,
            color: vec3(1.0, 1.0, 1.0),
            scale: 1.0,
            cone: None,
            shadow: Q2ShadowProjection::None,
        };
        let lighting = BatchLighting::Q2World {
            world_positions: Vec::new(),
            normals: Vec::new(),
            atlas: None,
            pass: Q2LightPass::Texture { lights: vec![light; 9] },
        };
        stage.use_stage(&mut gl, None, AlphaTest::None, &lighting, false, None);
    }
}
