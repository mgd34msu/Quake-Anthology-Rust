//! Native stage math shared by both consumers. Tables and noise are load-owned;
//! a prepared draw and its evaluated vertices are fixed-size values.
//! References: Q3 tr_shade.c:217,653,853; tr_shade_calc.c:27,118,677,888.
use crate::assets::{ImageId, MaterialSettings, Stage, StageTexture, TcMod, Vertex};
use crate::shader::{
    AlphaFunc, AlphaGen, BlendFactor, Deform, RgbGen, StageBlend, TexCoordGen, TexMod,
    WaveFunction, Waveform,
};
use qa_core::{
    math::{length, normalized_fast},
    primitives::Vec3,
};

/// Native R_RotateForEntity uses a single axis-length compensation selected by
/// the submitted entity; this is deliberately shared by both draw consumers.
pub fn entity_view_origin(view: Vec3, entity: &crate::scene::SceneEntity) -> Vec3 {
    let delta = Vec3(std::array::from_fn(|i| view.0[i] - entity.origin.0[i]));
    let scale = if entity.non_normalized_axes {
        let length = entity.axes[0].dot(entity.axes[0]).sqrt();
        if length == 0.0 { 0.0 } else { 1.0 / length }
    } else {
        1.0
    };
    Vec3(entity.axes.map(|axis| delta.dot(axis) * scale))
}

pub const TABLE_SIZE: usize = 1024;
pub const WARP_TABLE: usize = 5 * TABLE_SIZE;
pub const NOISE_TABLE: usize = WARP_TABLE + 256;
pub const NOISE_PERM: usize = NOISE_TABLE + 256;
pub const TABLE_VALUES: usize = NOISE_PERM + 256;
// Original GL liquid lookup values, qsrc quake/WinQuake/gl_warp_sin.h.
const NATIVE_WARP_SIN: [f32; 256] = [
    0.0,
    0.19633,
    0.392541,
    0.588517,
    0.784137,
    0.979285,
    1.17384,
    1.3677,
    1.56072,
    1.75281,
    1.94384,
    2.1337,
    2.32228,
    2.50945,
    2.69512,
    2.87916,
    3.06147,
    3.24193,
    3.42044,
    3.59689,
    3.77117,
    3.94319,
    4.11282,
    4.27998,
    4.44456,
    4.60647,
    4.76559,
    4.92185,
    5.07515,
    5.22538,
    5.37247,
    5.51632,
    5.65685,
    5.79398,
    5.92761,
    6.05767,
    6.18408,
    6.30677,
    6.42566,
    6.54068,
    6.65176,
    6.75883,
    6.86183,
    6.9607,
    7.05537,
    7.14579,
    7.23191,
    7.31368,
    7.39104,
    7.46394,
    7.53235,
    7.59623,
    7.65552,
    7.71021,
    7.76025,
    7.80562,
    7.84628,
    7.88222,
    7.91341,
    7.93984,
    7.96148,
    7.97832,
    7.99036,
    7.99759,
    8.0,
    7.99759,
    7.99036,
    7.97832,
    7.96148,
    7.93984,
    7.91341,
    7.88222,
    7.84628,
    7.80562,
    7.76025,
    7.71021,
    7.65552,
    7.59623,
    7.53235,
    7.46394,
    7.39104,
    7.31368,
    7.23191,
    7.14579,
    7.05537,
    6.9607,
    6.86183,
    6.75883,
    6.65176,
    6.54068,
    6.42566,
    6.30677,
    6.18408,
    6.05767,
    5.92761,
    5.79398,
    5.65685,
    5.51632,
    5.37247,
    5.22538,
    5.07515,
    4.92185,
    4.76559,
    4.60647,
    4.44456,
    4.27998,
    4.11282,
    3.94319,
    3.77117,
    3.59689,
    3.42044,
    3.24193,
    3.06147,
    2.87916,
    2.69512,
    2.50945,
    2.32228,
    2.1337,
    1.94384,
    1.75281,
    1.56072,
    1.3677,
    1.17384,
    0.979285,
    0.784137,
    0.588517,
    0.392541,
    0.19633,
    9.79717e-16,
    -0.19633,
    -0.392541,
    -0.588517,
    -0.784137,
    -0.979285,
    -1.17384,
    -1.3677,
    -1.56072,
    -1.75281,
    -1.94384,
    -2.1337,
    -2.32228,
    -2.50945,
    -2.69512,
    -2.87916,
    -3.06147,
    -3.24193,
    -3.42044,
    -3.59689,
    -3.77117,
    -3.94319,
    -4.11282,
    -4.27998,
    -4.44456,
    -4.60647,
    -4.76559,
    -4.92185,
    -5.07515,
    -5.22538,
    -5.37247,
    -5.51632,
    -5.65685,
    -5.79398,
    -5.92761,
    -6.05767,
    -6.18408,
    -6.30677,
    -6.42566,
    -6.54068,
    -6.65176,
    -6.75883,
    -6.86183,
    -6.9607,
    -7.05537,
    -7.14579,
    -7.23191,
    -7.31368,
    -7.39104,
    -7.46394,
    -7.53235,
    -7.59623,
    -7.65552,
    -7.71021,
    -7.76025,
    -7.80562,
    -7.84628,
    -7.88222,
    -7.91341,
    -7.93984,
    -7.96148,
    -7.97832,
    -7.99036,
    -7.99759,
    -8.0,
    -7.99759,
    -7.99036,
    -7.97832,
    -7.96148,
    -7.93984,
    -7.91341,
    -7.88222,
    -7.84628,
    -7.80562,
    -7.76025,
    -7.71021,
    -7.65552,
    -7.59623,
    -7.53235,
    -7.46394,
    -7.39104,
    -7.31368,
    -7.23191,
    -7.14579,
    -7.05537,
    -6.9607,
    -6.86183,
    -6.75883,
    -6.65176,
    -6.54068,
    -6.42566,
    -6.30677,
    -6.18408,
    -6.05767,
    -5.92761,
    -5.79398,
    -5.65685,
    -5.51632,
    -5.37247,
    -5.22538,
    -5.07515,
    -4.92185,
    -4.76559,
    -4.60647,
    -4.44456,
    -4.27998,
    -4.11282,
    -3.94319,
    -3.77117,
    -3.59689,
    -3.42044,
    -3.24193,
    -3.06147,
    -2.87916,
    -2.69512,
    -2.50945,
    -2.32228,
    -2.1337,
    -1.94384,
    -1.75281,
    -1.56072,
    -1.3677,
    -1.17384,
    -0.979285,
    -0.784137,
    -0.588517,
    -0.392541,
    -0.19633,
];

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EntityLighting {
    /// Native lighting values are in 0..255 units, not normalized RGB.
    pub ambient: [f32; 3],
    pub directed: [f32; 3],
    pub direction: Vec3,
}
impl Default for EntityLighting {
    fn default() -> Self {
        Self {
            ambient: [255.0; 3],
            directed: [0.0; 3],
            direction: Vec3([0.0, 0.0, 1.0]),
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DrawInputs {
    pub time_ms: u64,
    pub entity_color: [u8; 4],
    pub entity_texcoord: [f32; 2],
    pub entity_shader_time: f32,
    /// Camera and lighting vectors are in this draw's model-local coordinates.
    pub view_origin: Vec3,
    pub identity_light: f32,
    pub lightmap: ImageId,
    pub texture_scale: [f32; 2],
    pub lighting: Option<EntityLighting>,
    /// Native Q3 specular alpha uses this fixed light independently of an
    /// entity's diffuse lighting sample (tr_shade_calc.c:1038).
    pub specular_origin: Vec3,
}
impl Default for DrawInputs {
    fn default() -> Self {
        Self {
            time_ms: 0,
            entity_color: [255; 4],
            entity_texcoord: [0.0; 2],
            entity_shader_time: 0.0,
            view_origin: Vec3::default(),
            identity_light: 1.0,
            lightmap: ImageId(0),
            texture_scale: [1.0; 2],
            lighting: None,
            specular_origin: Vec3([-960.0, 1980.0, 96.0]),
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PreparedStage {
    pub stage: Stage,
    pub inputs: DrawInputs,
    pub image: ImageId,
    pub shader_time: f32,
    /// Uniform color generators are resolved once, including native byte
    /// quantization. Vertex generators replace these channels before clipping.
    pub uniform_color: [u8; 4],
    pub uniform_alpha: u8,
    pub tcmods: [TexCoordOp; 4],
}
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum TexCoordOp {
    #[default]
    None,
    Transform {
        matrix: [[f32; 2]; 2],
        translate: [f32; 2],
    },
    Turbulent {
        amplitude: f32,
        now: f32,
    },
    Warp {
        texel_scale: [f32; 2],
        amplitude: [f32; 2],
        frequency: f32,
        now: f32,
    },
}
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum DeformOp {
    #[default]
    None,
    Wave {
        table: usize,
        base: f32,
        amplitude: f32,
        phase: f32,
        time_phase: f32,
        spread: f32,
    },
    Move(Vec3),
    Bulge {
        width: f32,
        height: f32,
        now: f32,
    },
    Normal {
        amplitude: f32,
        time: f32,
    },
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EvaluatedVertex {
    pub position: Vec3,
    pub normal: Vec3,
    pub texcoord: [f32; 2],
    pub color: [u8; 4],
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StageError {
    Animation,
    MissingLighting,
    UnsupportedWave,
    GeometryDeform,
    SingularStretch,
}

pub struct StageEvaluator {
    tables: Box<[f32]>,
}
impl Default for StageEvaluator {
    fn default() -> Self {
        Self::load()
    }
}
impl StageEvaluator {
    /// The native Linux noise initialization uses glibc srand(1001). This Rust
    /// implementation owns its state and never touches libc's global RNG.
    pub fn load() -> Self {
        let mut rng = NativeRand::new(1001);
        let mut noise = [0.0; 256];
        let mut perm = [0; 256];
        for i in 0..256 {
            noise[i] = (rng.next() as f32 / i32::MAX as f32) * 2.0 - 1.0;
            perm[i] = (rng.next() as f32 / i32::MAX as f32 * 255.0) as u8;
        }
        Self::with_noise(noise, perm)
    }
    /// Other native CRT presets can supply their original seeded tables at
    /// load. The evaluator and GPU table layout stay identical.
    pub fn with_noise(noise: [f32; 256], perm: [u8; 256]) -> Self {
        let mut tables = vec![0.0; TABLE_VALUES].into_boxed_slice();
        for i in 0..TABLE_SIZE {
            let degrees = i as f32 * 360.0 / (TABLE_SIZE - 1) as f32;
            let radians = degrees * std::f32::consts::PI / 180.0;
            tables[i] = (radians as f64).sin() as f32;
            tables[TABLE_SIZE + i] = if i < 512 { 1.0 } else { -1.0 };
            tables[3 * TABLE_SIZE + i] = i as f32 / TABLE_SIZE as f32;
            tables[4 * TABLE_SIZE + i] = 1.0 - tables[3 * TABLE_SIZE + i];
            tables[2 * TABLE_SIZE + i] = if i < 256 {
                i as f32 / 256.0
            } else if i < 512 {
                1.0 - tables[2 * TABLE_SIZE + i - 256]
            } else {
                -tables[2 * TABLE_SIZE + i - 512]
            };
        }
        // Q1/Q2's native liquid table is one exact 256-sample cycle.
        for i in 0..256 {
            tables[WARP_TABLE + i] = NATIVE_WARP_SIN[i] * 0.125;
        }
        tables[NOISE_TABLE..NOISE_PERM].copy_from_slice(&noise);
        for i in 0..256 {
            tables[NOISE_PERM + i] = perm[i] as f32;
        }
        Self { tables }
    }
    /// GL uploads this load-owned table once as a texture buffer.
    pub fn gpu_tables(&self) -> &[f32] {
        &self.tables
    }
    pub fn wave(&self, wave: Waveform, time: f32) -> Result<f32, StageError> {
        let Some(offset) = wave_offset(wave.function) else {
            return Err(StageError::UnsupportedWave);
        };
        Ok(wave.base
            + self.tables[offset + wave_index(wave.phase + time * wave.frequency)] * wave.amplitude)
    }
    #[expect(
        clippy::collapsible_if,
        reason = "Keep the native CGEN_LIGHTING_DIFFUSE selection separate from this boundary's missing-lighting failure."
    )]
    pub fn prepare(
        &self,
        stage: &Stage,
        settings: MaterialSettings,
        inputs: DrawInputs,
    ) -> Result<PreparedStage, StageError> {
        if matches!(stage.rgb_gen, RgbGen::LightingDiffuse) {
            if inputs.lighting.is_none() {
                return Err(StageError::MissingLighting);
            }
        }
        let time = shader_time(settings, inputs);
        let image = match stage.texture {
            StageTexture::Image(id) => id,
            StageTexture::Lightmap => inputs.lightmap,
            StageTexture::Animation {
                images,
                count,
                frequency,
            } => {
                if count == 0 || count > 8 {
                    return Err(StageError::Animation);
                }
                let index = (((time * frequency * 1024.0) as i32) >> 10).max(0) as usize;
                images[index % count as usize]
            }
        };
        // A zero stretch produces non-finite texture coordinates in qsrc;
        // scope the failure to this draw instead of uploading undefined data.
        let mut tcmods = [TexCoordOp::None; 4];
        for (output, modifier) in tcmods.iter_mut().zip(stage.tcmods) {
            if let Some(modifier) = modifier {
                *output = self.prepare_texmod(modifier, time, inputs)?;
            }
        }
        let uniform_color = match stage.rgb_gen {
            RgbGen::Identity => [255; 4],
            RgbGen::IdentityLighting => [byte(inputs.identity_light); 4],
            RgbGen::Entity => inputs.entity_color,
            RgbGen::OneMinusEntity => inputs.entity_color.map(|v| 255 - v),
            RgbGen::Const(rgb) => [
                constant_byte(rgb[0]),
                constant_byte(rgb[1]),
                constant_byte(rgb[2]),
                match stage.alpha_gen {
                    AlphaGen::Const(a) => constant_byte(a),
                    _ => 255,
                },
            ],
            RgbGen::Wave(wave) => {
                let glow = if wave.function == WaveFunction::Noise {
                    wave.base
                        + self.noise([0.0, 0.0, 0.0, (time + wave.phase) * wave.frequency])
                            * wave.amplitude
                } else {
                    self.wave(wave, time)? * inputs.identity_light
                };
                [byte(glow), byte(glow), byte(glow), 255]
            }
            _ => [255; 4],
        };
        let uniform_alpha = match stage.alpha_gen {
            AlphaGen::Entity => inputs.entity_color[3],
            AlphaGen::OneMinusEntity => 255 - inputs.entity_color[3],
            AlphaGen::Const(a) => constant_byte(a),
            AlphaGen::Wave(wave) => byte(self.wave(wave, time)?),
            _ => 255,
        };
        Ok(PreparedStage {
            stage: *stage,
            inputs,
            image,
            shader_time: time,
            uniform_color,
            uniform_alpha,
            tcmods,
        })
    }
    pub fn evaluate(&self, prepared: &PreparedStage, vertex: &Vertex) -> EvaluatedVertex {
        let (stage, inputs) = (prepared.stage, prepared.inputs);
        let mut color = prepared.uniform_color;
        match stage.rgb_gen {
            RgbGen::ExactVertex => color = vertex.color,
            RgbGen::Vertex => {
                color = vertex.color;
                for channel in &mut color[..3] {
                    *channel = (*channel as f32 * inputs.identity_light) as u8;
                }
            }
            RgbGen::OneMinusVertex => {
                for (channel, &value) in color[..3].iter_mut().zip(&vertex.color[..3]) {
                    *channel = ((255 - value) as f32 * inputs.identity_light) as u8;
                }
            }
            RgbGen::LightingDiffuse => {
                let lighting = inputs.lighting.unwrap_or_default(); // prepare checked presence
                let incoming = vertex.normal.dot(lighting.direction).max(0.0);
                for (i, channel) in color[..3].iter_mut().enumerate() {
                    *channel =
                        (lighting.ambient[i] + incoming * lighting.directed[i]).min(255.0) as u8;
                }
            }
            _ => {}
        }
        match stage.alpha_gen {
            AlphaGen::Skip => {}
            AlphaGen::Identity => {
                if !matches!(stage.rgb_gen, RgbGen::Identity)
                    && !(matches!(stage.rgb_gen, RgbGen::Vertex) && inputs.identity_light == 1.0)
                {
                    color[3] = 255;
                }
            }
            AlphaGen::Vertex => {
                if !matches!(stage.rgb_gen, RgbGen::Vertex) {
                    color[3] = vertex.color[3];
                }
            }
            AlphaGen::OneMinusVertex => color[3] = 255 - vertex.color[3],
            AlphaGen::Portal(range) => {
                color[3] = byte(length(vertex.position - inputs.view_origin) / range)
            }
            AlphaGen::LightingSpecular => {
                let direction = normalized_fast(inputs.specular_origin - vertex.position);
                let reflected = (vertex.normal * (2.0 * vertex.normal.dot(direction))) - direction;
                let viewer = normalized_fast(inputs.view_origin - vertex.position);
                let incidence = reflected.dot(viewer).max(0.0);
                color[3] = byte((incidence * incidence) * (incidence * incidence));
            }
            _ => color[3] = prepared.uniform_alpha,
        }
        let mut uv = match stage.texgen {
            TexCoordGen::Texture => [
                vertex.texcoord[0] * inputs.texture_scale[0],
                vertex.texcoord[1] * inputs.texture_scale[1],
            ],
            TexCoordGen::Lightmap => vertex.lightmap_coord,
            TexCoordGen::Vector(vectors) => vectors.map(|v| Vec3(v).dot(vertex.position)),
            TexCoordGen::Environment => {
                let viewer = normalized_fast(inputs.view_origin - vertex.position);
                let reflected = (vertex.normal * (2.0 * vertex.normal.dot(viewer))) - viewer;
                [0.5 + reflected.0[1] * 0.5, 0.5 - reflected.0[2] * 0.5]
            }
            TexCoordGen::LayeredSky {
                flatten_z,
                projected_scale,
                texture_size,
                scroll_speed,
            } => crate::sky::sphere_uv(
                vertex.position - inputs.view_origin,
                prepared.shader_time,
                flatten_z,
                projected_scale,
                texture_size,
                scroll_speed,
            )
            .unwrap_or([0.0; 2]),
            TexCoordGen::CloudSky { radius, height } => crate::sky::cloud_uv(
                vertex.position - inputs.view_origin,
                crate::sky::CloudSphere { radius, height },
            )
            .unwrap_or([0.0; 2]),
        };
        for modifier in prepared.tcmods {
            uv = self.texmod(modifier, uv, vertex.position);
        }
        EvaluatedVertex {
            position: vertex.position,
            normal: vertex.normal,
            texcoord: uv,
            color,
        }
    }
    pub fn prepare_deforms(
        &self,
        settings: &MaterialSettings,
        inputs: &DrawInputs,
    ) -> Result<[DeformOp; 3], StageError> {
        let time = shader_time(*settings, *inputs);
        let mut result = [DeformOp::None; 3];
        for (target, deform) in result.iter_mut().zip(settings.deforms) {
            *target = match deform {
                None => DeformOp::None,
                Some(Deform::Wave { spread, wave }) => DeformOp::Wave {
                    table: wave_offset(wave.function).ok_or(StageError::UnsupportedWave)?,
                    base: wave.base,
                    amplitude: wave.amplitude,
                    phase: wave.phase,
                    time_phase: time * wave.frequency,
                    spread: if wave.frequency == 0.0 { 0.0 } else { spread },
                },
                Some(Deform::Move { vector, wave }) => {
                    DeformOp::Move(Vec3(vector) * self.wave(wave, time)?)
                }
                Some(Deform::Bulge {
                    width,
                    height,
                    speed,
                }) => DeformOp::Bulge {
                    width,
                    height,
                    now: inputs.time_ms as f32 * speed * 0.001,
                },
                Some(Deform::Normal {
                    amplitude,
                    frequency,
                }) => DeformOp::Normal {
                    amplitude,
                    time: time * frequency,
                },
                _ => return Err(StageError::GeometryDeform),
            };
        }
        Ok(result)
    }
    pub fn deform_vertex(
        &self,
        settings: &MaterialSettings,
        inputs: &DrawInputs,
        vertex: Vertex,
    ) -> Result<Vertex, StageError> {
        Ok(self.apply_deforms(self.prepare_deforms(settings, inputs)?, vertex))
    }
    pub fn apply_deforms(&self, deforms: [DeformOp; 3], mut vertex: Vertex) -> Vertex {
        for deform in deforms {
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
                    let offset =
                        (vertex.position.0[0] + vertex.position.0[1] + vertex.position.0[2])
                            * spread;
                    let value = base
                        + self.tables[table + wave_index(phase + offset + time_phase)] * amplitude;
                    vertex.position = vertex.position + (vertex.normal * value);
                }
                DeformOp::Move(vector) => vertex.position = vertex.position + vector,
                DeformOp::Bulge { width, height, now } => {
                    let index = ((1024.0 / std::f32::consts::TAU)
                        * (vertex.texcoord[0] * width + now))
                        as i32;
                    vertex.position = vertex.position
                        + (vertex.normal * (self.tables[(index & 1023) as usize] * height));
                }
                DeformOp::Normal { amplitude, time } => {
                    for i in 0..3 {
                        vertex.normal.0[i] += amplitude
                            * self.noise([
                                i as f32 * 100.0 + vertex.position.0[0] * 0.98,
                                vertex.position.0[1] * 0.98,
                                vertex.position.0[2] * 0.98,
                                time,
                            ]);
                    }
                    vertex.normal = normalized_fast(vertex.normal);
                }
            }
        }
        vertex
    }
    fn prepare_texmod(
        &self,
        modifier: TcMod,
        time: f32,
        inputs: DrawInputs,
    ) -> Result<TexCoordOp, StageError> {
        let translation = |translate| TexCoordOp::Transform {
            matrix: [[1.0, 0.0], [0.0, 1.0]],
            translate,
        };
        Ok(match modifier {
            TcMod::Warp(warp) => TexCoordOp::Warp {
                texel_scale: warp.texel_scale,
                amplitude: warp.amplitude,
                frequency: warp.frequency,
                now: time * warp.time_scale,
            },
            TcMod::Flow(flow) => {
                let cycle = time * flow.speed;
                let phase = cycle - cycle.trunc();
                translation(if phase == 0.0 {
                    flow.cycle_start
                } else {
                    flow.amplitude.map(|a| a * phase)
                })
            }
            TcMod::Script(script) => match script {
                TexMod::Scale(value) => TexCoordOp::Transform {
                    matrix: [[value[0], 0.0], [0.0, value[1]]],
                    translate: [0.0; 2],
                },
                TexMod::Scroll(speed) => {
                    let v = speed.map(|s| s * time);
                    translation(v.map(|v| v - v.floor()))
                }
                TexMod::Transform { matrix, translate } => {
                    TexCoordOp::Transform { matrix, translate }
                }
                TexMod::Rotate(speed) => {
                    let index = (-speed * time * (1024.0 / 360.0)) as i32;
                    let sin = self.tables[(index & 1023) as usize];
                    let cos = self.tables[((index + 256) & 1023) as usize];
                    TexCoordOp::Transform {
                        matrix: [[cos, sin], [-sin, cos]],
                        translate: [0.5 - 0.5 * cos + 0.5 * sin, 0.5 - 0.5 * sin - 0.5 * cos],
                    }
                }
                TexMod::Stretch(wave) => {
                    let wave = self.wave(wave, time)?;
                    if wave == 0.0 {
                        return Err(StageError::SingularStretch);
                    }
                    let p = 1.0 / wave;
                    TexCoordOp::Transform {
                        matrix: [[p, 0.0], [0.0, p]],
                        translate: [0.5 - 0.5 * p; 2],
                    }
                }
                TexMod::Turbulent {
                    amplitude,
                    phase,
                    frequency,
                    ..
                } => TexCoordOp::Turbulent {
                    amplitude,
                    now: phase + time * frequency,
                },
                TexMod::EntityTranslate => translation(inputs.entity_texcoord),
            },
        })
    }
    pub fn texmod(&self, modifier: TexCoordOp, uv: [f32; 2], position: Vec3) -> [f32; 2] {
        match modifier {
            TexCoordOp::None => uv,
            TexCoordOp::Transform { matrix, translate } => transform(uv, matrix, translate),
            TexCoordOp::Turbulent { amplitude, now } => [
                uv[0]
                    + self.tables
                        [wave_index((position.0[0] + position.0[2]) * (1.0 / 128.0) * 0.125 + now)]
                        * amplitude,
                uv[1]
                    + self.tables[wave_index(position.0[1] * (1.0 / 128.0) * 0.125 + now)]
                        * amplitude,
            ],
            TexCoordOp::Warp {
                texel_scale,
                amplitude,
                frequency,
                now,
            } => {
                let index_s = (((uv[1] * texel_scale[1] * frequency + now)
                    * (256.0 / std::f32::consts::TAU)) as i32
                    & 255) as usize;
                let index_t = (((uv[0] * texel_scale[0] * frequency + now)
                    * (256.0 / std::f32::consts::TAU)) as i32
                    & 255) as usize;
                [
                    uv[0] + self.tables[WARP_TABLE + index_s] * amplitude[0],
                    uv[1] + self.tables[WARP_TABLE + index_t] * amplitude[1],
                ]
            }
        }
    }
    #[expect(
        clippy::needless_range_loop,
        reason = "Preserve tr_noise.c R_NoiseGet4f lattice indices and x/y/z/t interpolation order."
    )]
    pub fn noise(&self, point: [f32; 4]) -> f32 {
        let cell = point.map(|p| p.floor() as i32);
        let frac = std::array::from_fn::<_, 4, _>(|i| point[i] - point[i].floor());
        let mut value = [0.0; 2];
        for t in 0..2 {
            let mut depth = [0.0; 2];
            for z in 0..2 {
                let mut rows = [0.0; 2];
                for y in 0..2 {
                    rows[y] = lerp(
                        self.noise_value(
                            cell[0],
                            cell[1] + y as i32,
                            cell[2] + z as i32,
                            cell[3] + t as i32,
                        ),
                        self.noise_value(
                            cell[0] + 1,
                            cell[1] + y as i32,
                            cell[2] + z as i32,
                            cell[3] + t as i32,
                        ),
                        frac[0],
                    );
                }
                depth[z] = lerp(rows[0], rows[1], frac[1]);
            }
            value[t] = lerp(depth[0], depth[1], frac[2]);
        }
        lerp(value[0], value[1], frac[3])
    }
    fn noise_value(&self, x: i32, y: i32, z: i32, t: i32) -> f32 {
        let perm = |i: i32| self.tables[NOISE_PERM + (i & 255) as usize] as i32;
        self.tables[NOISE_TABLE
            + perm(x.wrapping_add(perm(y.wrapping_add(perm(z.wrapping_add(perm(t))))))) as usize]
    }
}
pub fn shader_time(settings: MaterialSettings, inputs: DrawInputs) -> f32 {
    let time = inputs.time_ms as f32 * 0.001 - inputs.entity_shader_time - settings.time_offset;
    if let Some(clamp) = settings.clamp_time.filter(|&clamp| clamp > 0.0) {
        time.min(clamp)
    } else {
        time
    }
}
pub fn wave_offset(function: WaveFunction) -> Option<usize> {
    Some(match function {
        WaveFunction::Sin => 0,
        WaveFunction::Square => 1024,
        WaveFunction::Triangle => 2048,
        WaveFunction::Sawtooth => 3072,
        WaveFunction::InverseSawtooth => 4096,
        WaveFunction::Noise => return None,
    })
}
fn wave_index(phase: f32) -> usize {
    ((phase * 1024.0) as i32 & 1023) as usize
}
fn transform(uv: [f32; 2], matrix: [[f32; 2]; 2], translate: [f32; 2]) -> [f32; 2] {
    [
        uv[0] * matrix[0][0] + uv[1] * matrix[1][0] + translate[0],
        uv[0] * matrix[0][1] + uv[1] * matrix[1][1] + translate[1],
    ]
}
fn byte(value: f32) -> u8 {
    (255.0 * value.clamp(0.0, 1.0)) as u8
}
fn constant_byte(value: f32) -> u8 {
    (255.0 * value) as i32 as u8
}
fn lerp(a: f32, b: f32, weight: f32) -> f32 {
    a * (1.0 - weight) + b * weight
}

pub fn alpha_pass(function: AlphaFunc, alpha: f32) -> bool {
    match function {
        AlphaFunc::None => true,
        AlphaFunc::GreaterZero => alpha > 0.0,
        AlphaFunc::LessThanHalf => alpha < 0.5,
        AlphaFunc::AtLeastHalf => alpha >= 0.5,
    }
}
pub fn blend_pixel(blend: Option<StageBlend>, source: [f32; 4], destination: [f32; 4]) -> [f32; 4] {
    let Some(blend) = blend else {
        return source;
    };
    let factor = |factor: BlendFactor, channel: usize| match factor {
        BlendFactor::Zero => 0.0,
        BlendFactor::One => 1.0,
        BlendFactor::SourceColor => source[channel],
        BlendFactor::OneMinusSourceColor => 1.0 - source[channel],
        BlendFactor::DestinationColor => destination[channel],
        BlendFactor::OneMinusDestinationColor => 1.0 - destination[channel],
        BlendFactor::SourceAlpha => source[3],
        BlendFactor::OneMinusSourceAlpha => 1.0 - source[3],
        BlendFactor::DestinationAlpha => destination[3],
        BlendFactor::OneMinusDestinationAlpha => 1.0 - destination[3],
        BlendFactor::SourceAlphaSaturate => {
            if channel == 3 {
                1.0
            } else {
                source[3].min(1.0 - destination[3])
            }
        }
    };
    std::array::from_fn(|i| {
        (source[i] * factor(blend.source, i) + destination[i] * factor(blend.destination, i))
            .clamp(0.0, 1.0)
    })
}
struct NativeRand {
    state: [u32; 31],
    front: usize,
    back: usize,
}
impl NativeRand {
    fn new(seed: u32) -> Self {
        let mut state = [0; 31];
        state[0] = seed;
        for i in 1..31 {
            state[i] = (16807_u64 * state[i - 1] as u64 % 2147483647) as u32;
        }
        let mut rand = Self {
            state,
            front: 3,
            back: 0,
        };
        for _ in 0..310 {
            rand.next();
        }
        rand
    }
    fn next(&mut self) -> u32 {
        self.state[self.front] = self.state[self.front].wrapping_add(self.state[self.back]);
        let result = self.state[self.front] >> 1;
        self.front = (self.front + 1) % 31;
        self.back = (self.back + 1) % 31;
        result
    }
}
