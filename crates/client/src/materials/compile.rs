//! Shader compilation: parser state joined with render contracts.
//!
//! Donor provenance: `src/materials/compile.ts` (`compileShaderScript`,
//! `shaderRenderMaterial`, `shaderSurfaceFlags`,
//! `compileImplicitMaterial`, `DEFAULT_SHADER_PROFILE`).
//!
//! The donor registers images through an async host; this port uses a
//! synchronous [`ShaderRegistrationHost`] (headless image handles are
//! `u32`). Request order, warnings, and defaulting match the donor.

use super::finish::{
    finish_implicit_shader, finish_shader, implicit_shader_input, FinishImplicitShaderInput, FinishShaderInput,
    FinishShaderProfile, FinishedShader,
};
use super::iterator::{IteratorDriver, MaterialIteratorProfile};
use super::material::{
    inspect_shader_script, ShaderDefinition, ShaderEntryResult, ShaderMap, SourceWaveFunc, SourceWaveStorage, WaveKind,
};
use super::state::{AlphaTest, Blend, CullFace, DepthTest};
use crate::ClientError;

/// Default finish profile (`DEFAULT_SHADER_PROFILE`).
#[must_use]
pub const fn default_shader_profile() -> FinishShaderProfile {
    FinishShaderProfile {
        detail_textures: true,
        vertex_light: false,
        ui_fullscreen: false,
        hardware: super::finish::FinishHardware::Generic,
        iterator: MaterialIteratorProfile {
            ignore_fast_path: false,
            multitexture: true,
            texture_env_add: true,
            driver: IteratorDriver::Generic,
        },
    }
}

/// A finished single image or frame animation (`FinishedImagePlayback`).
#[derive(Debug, Clone, PartialEq)]
pub enum FinishedImagePlayback {
    /// Single image handle.
    Single {
        /// Image handle.
        image: u32,
    },
    /// Frame animation.
    Animation {
        /// Playback frequency.
        frequency: f32,
        /// Frame image handles (non-empty).
        frames: Vec<u32>,
    },
}

/// Registered bundle data (`FinishedStageBinding`).
#[derive(Debug, Clone, PartialEq)]
pub enum FinishedStageBinding {
    /// Image playback.
    Images {
        /// Playback.
        playback: FinishedImagePlayback,
    },
    /// Video source id.
    Video {
        /// Cinematic source id.
        source: u32,
    },
    /// Retain current texture.
    RetainCurrentTexture,
}

/// An image request (`SourceImageRequest`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceImageRequest {
    /// Image name.
    pub name: String,
    /// Mipmaps wanted.
    pub mipmap: bool,
    /// Picmip allowed.
    pub allow_picmip: bool,
    /// Wrapping.
    pub wrap: ImageWrap,
}

/// Image wrapping.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageWrap {
    /// Repeat.
    Repeat,
    /// Clamp.
    Clamp,
}

/// A registered image (`RegisteredImage`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RegisteredImage {
    /// Image handle.
    pub image: u32,
    /// Texture unit.
    pub tmu: u8,
}

/// A registered shader video (`RegisteredShaderVideo`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RegisteredShaderVideo {
    /// Cinematic source id.
    pub source: u32,
    /// Poster image.
    pub image: RegisteredImage,
}

/// A registered shader stage (`RegisteredShaderStage`).
#[derive(Debug, Clone, PartialEq)]
pub enum RegisteredStage {
    /// Loaded stage.
    Loaded {
        /// Texture unit.
        tmu: u8,
        /// Binding.
        binding: FinishedStageBinding,
    },
    /// Missing stage.
    Missing,
}

/// Sky box face name (`SourceSkyFaceName`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceSkyFaceName {
    /// Right.
    Rt,
    /// Back.
    Bk,
    /// Left.
    Lf,
    /// Front.
    Ft,
    /// Up.
    Up,
    /// Down.
    Dn,
}

/// Sky box faces in source registration order.
pub const SKY_FACES: [SourceSkyFaceName; 6] = [
    SourceSkyFaceName::Rt,
    SourceSkyFaceName::Bk,
    SourceSkyFaceName::Lf,
    SourceSkyFaceName::Ft,
    SourceSkyFaceName::Up,
    SourceSkyFaceName::Dn,
];

/// A registered sky box (`RegisteredSkyBox`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegisteredSkyBox {
    /// Face image handles in [`SKY_FACES`] order.
    pub faces: [u32; 6],
}

impl RegisteredSkyBox {
    /// Image for a face.
    #[must_use]
    pub const fn image(&self, face: SourceSkyFaceName) -> u32 {
        self.faces[face as usize]
    }
}

/// A registered sky (`RegisteredSky`).
#[derive(Debug, Clone, PartialEq)]
pub struct RegisteredSky {
    /// Outer box.
    pub outer: Option<RegisteredSkyBox>,
    /// Inner box.
    pub inner: Option<RegisteredSkyBox>,
    /// Cloud height.
    pub cloud_height: f32,
}

/// Registration failure (`ShaderRegistrationFailure`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShaderRegistrationFailure {
    /// Source text rejected.
    SourceText {
        /// Error message.
        message: String,
    },
    /// Image missing.
    MissingImage {
        /// Failed request.
        request: SourceImageRequest,
    },
}

/// A registered explicit shader (`RegisteredExplicitShader`).
#[derive(Debug, Clone, PartialEq)]
pub struct RegisteredExplicitShader {
    /// Definition.
    pub definition: ShaderDefinition,
    /// Registered stages.
    pub stages: Vec<RegisteredStage>,
    /// Registered sky.
    pub sky: Option<RegisteredSky>,
    /// Registration outcome.
    pub outcome: RegistrationOutcome,
}

/// Registration outcome.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RegistrationOutcome {
    /// Defined.
    Defined,
    /// Defaulted with failure.
    Defaulted {
        /// Failure.
        failure: ShaderRegistrationFailure,
    },
}

/// Synchronous shader registration host (`ShaderRegistrationHost`).
pub trait ShaderRegistrationHost {
    /// White image.
    fn white_image(&self) -> RegisteredImage;
    /// Default image.
    fn default_image(&self) -> RegisteredImage;
    /// Lightmap image.
    fn lightmap_image(&self) -> RegisteredImage;
    /// Find an image.
    fn find_image(&mut self, request: &SourceImageRequest) -> Option<RegisteredImage>;
    /// Play a shader cinematic.
    fn play_shader_cinematic(&mut self, name: &str) -> Option<RegisteredShaderVideo>;
    /// Apply the sun.
    fn apply_sun(&mut self, sun: super::material::RegisteredSun);
    /// Initialize sky texture coordinates.
    fn initialize_sky_tex_coords(&mut self, height: f32);
    /// Print a warning.
    fn print_warning(&mut self, message: &str);
}

/// A compiled material (`CompiledMaterial`).
#[derive(Debug, Clone, PartialEq)]
pub struct CompiledMaterial {
    /// Registered shader.
    pub registered: RegisteredExplicitShader,
    /// Finished shader.
    pub finished: FinishedShader,
    /// Render-material view.
    pub material: RenderMaterialView,
}

/// Semantic waveform (`Waveform` contract view).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SemanticWaveform {
    /// Kind.
    pub kind: WaveKind,
    /// Base value.
    pub base: f32,
    /// Amplitude.
    pub amplitude: f32,
    /// Phase.
    pub phase: f32,
    /// Frequency.
    pub frequency: f32,
}

fn semantic_wave(wave: &SourceWaveStorage) -> Result<SemanticWaveform, ClientError> {
    let kind = match wave.func {
        SourceWaveFunc::None => WaveKind::None,
        SourceWaveFunc::Sin => WaveKind::Sin,
        SourceWaveFunc::Square => WaveKind::Square,
        SourceWaveFunc::Triangle => WaveKind::Triangle,
        SourceWaveFunc::Sawtooth => WaveKind::Sawtooth,
        SourceWaveFunc::InverseSawtooth => WaveKind::InverseSawtooth,
        SourceWaveFunc::Noise => WaveKind::Noise,
    };
    Ok(SemanticWaveform {
        kind,
        base: wave.base,
        amplitude: wave.amplitude,
        phase: wave.phase,
        frequency: wave.frequency,
    })
}

/// Render-material stage view (`RenderMaterial` Q3 stage subset).
#[derive(Debug, Clone, PartialEq)]
pub struct RenderStageView {
    /// Map.
    pub map: ShaderMap,
    /// Blend.
    pub blend: Blend,
    /// Depth test.
    pub depth_test: DepthTest,
    /// Depth write.
    pub depth_write: bool,
    /// Alpha test.
    pub alpha_test: AlphaTest,
    /// Detail.
    pub detail: bool,
    /// Color generator.
    pub color: super::material::ColorGen,
    /// Alpha generator.
    pub alpha: super::material::AlphaGen,
    /// Coordinate generator.
    pub coordinates: super::material::TexGen,
    /// Coordinate modifiers.
    pub modifiers: Vec<super::material::TexMod>,
    /// RGB wave.
    pub rgb_wave: SemanticWaveform,
    /// Alpha wave.
    pub alpha_wave: SemanticWaveform,
}

/// Render-material view (`RenderMaterial` Q3 subset).
#[derive(Debug, Clone, PartialEq)]
pub struct RenderMaterialView {
    /// Material name.
    pub name: String,
    /// Stages.
    pub stages: Vec<RenderStageView>,
    /// Deformations.
    pub deformations: Vec<super::material::VertexDeformation>,
    /// Surface parameters.
    pub surface_parameters: Vec<String>,
    /// Sort override.
    pub sort: Option<f32>,
    /// Culling.
    pub cull: CullFace,
    /// Polygon offset.
    pub polygon_offset: bool,
    /// No mipmaps.
    pub no_mipmaps: bool,
    /// No picmip.
    pub no_picmip: bool,
    /// Entity mergable.
    pub entity_mergable: bool,
    /// Portal range.
    pub portal_range: f32,
    /// Clamp time.
    pub clamp_time: f32,
    /// Sky parameters.
    pub sky: Option<super::material::SkyParms>,
    /// Fog parameters.
    pub fog: Option<super::material::FogParms>,
    /// Sun parameters.
    pub sun: Option<super::material::SunParms>,
}

/// Build the render-material view (`shaderRenderMaterial`).
pub fn shader_render_material(definition: &ShaderDefinition) -> Result<RenderMaterialView, ClientError> {
    let mut stages = Vec::with_capacity(definition.stages.len());
    for stage in &definition.stages {
        stages.push(RenderStageView {
            map: stage.stage.map.clone(),
            blend: stage.stage.blend,
            depth_test: stage.stage.depth_func,
            depth_write: stage.stage.depth_write,
            alpha_test: stage.stage.alpha_func,
            detail: stage.stage.detail,
            color: stage.stage.rgb_gen,
            alpha: stage.stage.alpha_gen,
            coordinates: stage.stage.tc_gen,
            modifiers: stage.stage.tc_mods.clone(),
            rgb_wave: semantic_wave(&stage.source_state.rgb_wave)?,
            alpha_wave: semantic_wave(&stage.source_state.alpha_wave)?,
        });
    }
    Ok(RenderMaterialView {
        name: definition.name.clone(),
        stages,
        deformations: definition.deforms.clone(),
        surface_parameters: definition.surface_parms.clone(),
        sort: definition.sort,
        cull: definition.cull,
        polygon_offset: definition.polygon_offset,
        no_mipmaps: definition.no_mipmaps,
        no_picmip: definition.no_picmip,
        entity_mergable: definition.entity_mergable,
        portal_range: definition.portal_range,
        clamp_time: definition.clamp_time,
        sky: definition.sky.clone(),
        fog: definition.fog,
        sun: definition.sun,
    })
}

/// Compile options (`compileShaderScript` options).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CompileOptions {
    /// Lightmap index.
    pub lightmap_index: i32,
    /// Finish profile.
    pub profile: FinishShaderProfile,
}

impl Default for CompileOptions {
    fn default() -> Self {
        Self {
            lightmap_index: -1,
            profile: default_shader_profile(),
        }
    }
}

fn sky_box_name(face: SourceSkyFaceName) -> &'static str {
    match face {
        SourceSkyFaceName::Rt => "rt",
        SourceSkyFaceName::Bk => "bk",
        SourceSkyFaceName::Lf => "lf",
        SourceSkyFaceName::Ft => "ft",
        SourceSkyFaceName::Up => "up",
        SourceSkyFaceName::Dn => "dn",
    }
}

fn register_sky_box(host: &mut dyn ShaderRegistrationHost, base: &str, wrap: ImageWrap) -> RegisteredSkyBox {
    let mut faces = [0u32; 6];
    for (index, face) in SKY_FACES.iter().enumerate() {
        let request = SourceImageRequest {
            name: format!("{base}_{}.tga", sky_box_name(*face)),
            mipmap: true,
            allow_picmip: true,
            wrap,
        };
        // Missing sky faces fall back to the default image.
        faces[index] = host
            .find_image(&request)
            .map_or_else(|| host.default_image().image, |image| image.image);
    }
    RegisteredSkyBox { faces }
}

/// Register one parsed definition against a host.
pub fn register_definition(
    host: &mut dyn ShaderRegistrationHost,
    definition: &ShaderDefinition,
) -> RegisteredExplicitShader {
    let white = host.white_image();
    let lightmap = host.lightmap_image();
    let mut stages = Vec::with_capacity(definition.stages.len());
    for stage in &definition.stages {
        match &stage.stage.map {
            ShaderMap::Lightmap => stages.push(RegisteredStage::Loaded {
                tmu: lightmap.tmu,
                binding: FinishedStageBinding::Images {
                    playback: FinishedImagePlayback::Single { image: lightmap.image },
                },
            }),
            ShaderMap::WhiteImage => stages.push(RegisteredStage::Loaded {
                tmu: white.tmu,
                binding: FinishedStageBinding::Images {
                    playback: FinishedImagePlayback::Single { image: white.image },
                },
            }),
            ShaderMap::Image { name, clamp } => {
                let request = SourceImageRequest {
                    name: name.clone(),
                    mipmap: !definition.no_mipmaps,
                    allow_picmip: !definition.no_picmip,
                    wrap: if *clamp { ImageWrap::Clamp } else { ImageWrap::Repeat },
                };
                match host.find_image(&request) {
                    Some(image) => stages.push(RegisteredStage::Loaded {
                        tmu: image.tmu,
                        binding: FinishedStageBinding::Images {
                            playback: FinishedImagePlayback::Single { image: image.image },
                        },
                    }),
                    None => {
                        host.print_warning(&format!(
                            "WARNING: R_FindImageFile could not find '{}' in shader '{}'\n",
                            request.name, definition.name
                        ));
                        stages.push(RegisteredStage::Missing);
                        while stages.len() < definition.stages.len() {
                            stages.push(RegisteredStage::Missing);
                        }
                        return RegisteredExplicitShader {
                            definition: definition.clone(),
                            stages,
                            sky: None,
                            outcome: RegistrationOutcome::Defaulted {
                                failure: ShaderRegistrationFailure::MissingImage { request },
                            },
                        };
                    }
                }
            }
            ShaderMap::Animation { frequency, frames } => {
                let mut images = Vec::new();
                let mut tmu = 0u8;
                let mut failed: Option<SourceImageRequest> = None;
                for frame in frames.iter().take(8) {
                    let request = SourceImageRequest {
                        name: frame.clone(),
                        mipmap: !definition.no_mipmaps,
                        allow_picmip: !definition.no_picmip,
                        wrap: ImageWrap::Repeat,
                    };
                    match host.find_image(&request) {
                        Some(image) => {
                            tmu = image.tmu;
                            images.push(image.image);
                        }
                        None => {
                            failed = Some(request);
                            break;
                        }
                    }
                }
                if let Some(request) = failed {
                    host.print_warning(&format!(
                        "WARNING: R_FindImageFile could not find '{}' in shader '{}'\n",
                        request.name, definition.name
                    ));
                    stages.push(RegisteredStage::Missing);
                    while stages.len() < definition.stages.len() {
                        stages.push(RegisteredStage::Missing);
                    }
                    return RegisteredExplicitShader {
                        definition: definition.clone(),
                        stages,
                        sky: None,
                        outcome: RegistrationOutcome::Defaulted {
                            failure: ShaderRegistrationFailure::MissingImage { request },
                        },
                    };
                }
                if images.is_empty() {
                    stages.push(RegisteredStage::Missing);
                } else {
                    stages.push(RegisteredStage::Loaded {
                        tmu,
                        binding: FinishedStageBinding::Images {
                            playback: FinishedImagePlayback::Animation {
                                frequency: *frequency,
                                frames: images,
                            },
                        },
                    });
                }
            }
            ShaderMap::Video { name } => match host.play_shader_cinematic(name) {
                Some(video) => stages.push(RegisteredStage::Loaded {
                    tmu: video.image.tmu,
                    binding: FinishedStageBinding::Video { source: video.source },
                }),
                None => stages.push(RegisteredStage::Missing),
            },
            ShaderMap::None => stages.push(RegisteredStage::Missing),
        }
    }
    let sky = definition.sky.as_ref().map(|sky| {
        if sky.cloud_height != 0.0 {
            host.initialize_sky_tex_coords(sky.cloud_height);
        }
        RegisteredSky {
            outer: sky
                .outer_box
                .as_ref()
                .map(|base| register_sky_box(host, base, ImageWrap::Clamp)),
            inner: sky
                .inner_box
                .as_ref()
                .map(|base| register_sky_box(host, base, ImageWrap::Repeat)),
            cloud_height: sky.cloud_height,
        }
    });
    if let Some(sun) = &definition.sun {
        host.apply_sun(super::material::sun_registration(sun));
    }
    RegisteredExplicitShader {
        definition: definition.clone(),
        stages,
        sky,
        outcome: RegistrationOutcome::Defined,
    }
}

/// Compile a shader script (`compileShaderScript`, sync host).
pub fn compile_shader_script(
    text: &str,
    host: &mut dyn ShaderRegistrationHost,
    source: &str,
    options: &CompileOptions,
) -> Result<Vec<CompiledMaterial>, ClientError> {
    let entries = inspect_shader_script(text, source)?;
    let mut materials = Vec::with_capacity(entries.len());
    for entry in entries {
        match entry.result {
            ShaderEntryResult::Accepted(definition) => {
                let registered = register_definition(host, &definition);
                let finished = finish_shader(&FinishShaderInput {
                    definition: registered.definition.clone(),
                    lightmap_index: options.lightmap_index,
                    images: registered.stages.clone(),
                    profile: options.profile,
                })?;
                let material = shader_render_material(&registered.definition)?;
                materials.push(CompiledMaterial {
                    registered,
                    finished,
                    material,
                });
            }
            ShaderEntryResult::Rejected { message, drop_message } => {
                if let Some(drop) = drop_message {
                    return Err(ClientError::BadShader(drop));
                }
                // Rejected text defaults like a missing shader record.
                let partial = ShaderDefinition {
                    name: entry.name.clone(),
                    stages: Vec::new(),
                    surface_parms: Vec::new(),
                    cull: CullFace::Front,
                    sort: None,
                    sky: None,
                    fog: None,
                    sun: None,
                    deforms: Vec::new(),
                    polygon_offset: false,
                    no_mipmaps: false,
                    no_picmip: false,
                    entity_mergable: false,
                    portal_range: 0.0,
                    clamp_time: 0.0,
                    warnings: Vec::new(),
                    compiler_directives: Vec::new(),
                };
                let registered = RegisteredExplicitShader {
                    definition: partial.clone(),
                    stages: Vec::new(),
                    sky: None,
                    outcome: RegistrationOutcome::Defaulted {
                        failure: ShaderRegistrationFailure::SourceText { message },
                    },
                };
                let finished = finish_shader(&FinishShaderInput {
                    definition: partial,
                    lightmap_index: options.lightmap_index,
                    images: Vec::new(),
                    profile: options.profile,
                })?;
                let material = shader_render_material(&registered.definition)?;
                materials.push(CompiledMaterial {
                    registered,
                    finished,
                    material,
                });
            }
        }
    }
    Ok(materials)
}

/// Surface-parameter flags (`SurfaceParameterFlags`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SurfaceParameterFlags {
    /// Surface flags.
    pub surface: u32,
    /// Content flags.
    pub contents: u32,
    /// Clear-solid metadata (retained, renderer contents untouched).
    pub clear_solid: bool,
}

fn surface_flag(parameter: &str) -> u32 {
    match parameter {
        "nodamage" => 1,
        "slick" => 2,
        "sky" => 4,
        "ladder" => 8,
        "noimpact" => 0x10,
        "nomarks" => 0x20,
        "flesh" => 0x40,
        "nodraw" => 0x80,
        "hint" => 0x100,
        "nolightmap" => 0x400,
        "pointlight" => 0x800,
        "metalsteps" => 0x1000,
        "nosteps" => 0x2000,
        "nonsolid" => 0x4000,
        "lightfilter" => 0x8000,
        "alphashadow" => 0x10000,
        "nodlight" => 0x20000,
        "dust" => 0x40000,
        _ => 0,
    }
}

fn content_flag(parameter: &str) -> u32 {
    match parameter {
        "lava" => 8,
        "slime" => 16,
        "water" => 32,
        "fog" => 64,
        "areaportal" => 0x8000,
        "playerclip" => 0x10000,
        "monsterclip" => 0x20000,
        "clusterportal" => 0x100000,
        "donotenter" => 0x200000,
        "origin" => 0x1000000,
        "detail" => 0x8000000,
        "structural" => 0x10000000,
        "trans" => 0x20000000,
        "nodrop" => 0x80000000,
        _ => 0,
    }
}

fn clears_solid(parameter: &str) -> bool {
    matches!(
        parameter,
        "water"
            | "slime"
            | "lava"
            | "playerclip"
            | "monsterclip"
            | "nodrop"
            | "nonsolid"
            | "origin"
            | "areaportal"
            | "clusterportal"
            | "donotenter"
            | "fog"
    )
}

/// Compute surface flags (`shaderSurfaceFlags`).
#[must_use]
pub fn shader_surface_flags(parameters: &[String]) -> SurfaceParameterFlags {
    let mut surface = 0u32;
    let mut contents = 0u32;
    let mut clear_solid = false;
    for parameter in parameters {
        let key = parameter.to_ascii_lowercase();
        surface |= surface_flag(&key);
        contents |= content_flag(&key);
        clear_solid |= clears_solid(&key);
    }
    SurfaceParameterFlags {
        surface,
        contents,
        clear_solid,
    }
}

/// Compile an implicit material (`compileImplicitMaterial`).
pub fn compile_implicit_material(input: &FinishImplicitShaderInput) -> Result<CompiledMaterial, ClientError> {
    let prepared = implicit_shader_input(input)?;
    let finished = finish_implicit_shader(input)?;
    let stages = prepared
        .images
        .iter()
        .map(|image| match image {
            RegisteredStage::Loaded { tmu, binding } => RegisteredStage::Loaded {
                tmu: *tmu,
                binding: binding.clone(),
            },
            RegisteredStage::Missing => RegisteredStage::Missing,
        })
        .collect();
    let registered = RegisteredExplicitShader {
        definition: prepared.definition.clone(),
        stages,
        sky: None,
        outcome: RegistrationOutcome::Defined,
    };
    let material = shader_render_material(&prepared.definition)?;
    Ok(CompiledMaterial {
        registered,
        finished,
        material,
    })
}

/// Re-export for definition parsing without a host.
pub use super::material::parse_shader_script as parse_definitions_only;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::materials::finish::{FinishHardware, ImplicitShaderKind};
    use crate::materials::material::{normalize_image_name, parse_shader_script, Waveform};

    struct FixedHost {
        next: u32,
        warnings: Vec<String>,
    }

    impl ShaderRegistrationHost for FixedHost {
        fn white_image(&self) -> RegisteredImage {
            RegisteredImage { image: 1, tmu: 0 }
        }

        fn default_image(&self) -> RegisteredImage {
            RegisteredImage { image: 2, tmu: 0 }
        }

        fn lightmap_image(&self) -> RegisteredImage {
            RegisteredImage { image: 3, tmu: 1 }
        }

        fn find_image(&mut self, request: &SourceImageRequest) -> Option<RegisteredImage> {
            if request.name.contains("missing") {
                return None;
            }
            self.next += 1;
            Some(RegisteredImage {
                image: self.next + 10,
                tmu: 0,
            })
        }

        fn play_shader_cinematic(&mut self, name: &str) -> Option<RegisteredShaderVideo> {
            if name.contains("missing") {
                return None;
            }
            Some(RegisteredShaderVideo {
                source: 9,
                image: RegisteredImage { image: 4, tmu: 0 },
            })
        }

        fn apply_sun(&mut self, _sun: super::super::material::RegisteredSun) {}

        fn initialize_sky_tex_coords(&mut self, _height: f32) {}

        fn print_warning(&mut self, message: &str) {
            self.warnings.push(message.to_string());
        }
    }

    #[test]
    fn compiles_script_with_found_images() {
        let mut host = FixedHost {
            next: 0,
            warnings: Vec::new(),
        };
        let materials = compile_shader_script(
            "rock\n{\n {\n map textures/rock.tga\n }\n}\n",
            &mut host,
            "<test>",
            &CompileOptions::default(),
        )
        .unwrap();
        assert_eq!(materials.len(), 1);
        assert_eq!(materials[0].registered.outcome, RegistrationOutcome::Defined);
        assert_eq!(materials[0].finished.sort, 3);
    }

    #[test]
    fn missing_image_defaults_with_warning() {
        let mut host = FixedHost {
            next: 0,
            warnings: Vec::new(),
        };
        let materials = compile_shader_script(
            "rock\n{\n {\n map textures/missing.tga\n }\n}\n",
            &mut host,
            "<test>",
            &CompileOptions::default(),
        )
        .unwrap();
        assert!(matches!(
            materials[0].registered.outcome,
            RegistrationOutcome::Defaulted { .. }
        ));
        assert_eq!(host.warnings.len(), 1);
    }

    #[test]
    fn surface_flags_match_donor_tables() {
        let flags = shader_surface_flags(&["sky".to_string(), "water".to_string(), "nodlight".to_string()]);
        assert_eq!(flags.surface, 4 | 0x20000);
        assert_eq!(flags.contents, 32);
        assert!(flags.clear_solid);
    }

    #[test]
    fn implicit_material_compiles() {
        let profile = default_shader_profile();
        let compiled = compile_implicit_material(&FinishImplicitShaderInput {
            name: "implicit".to_string(),
            base_image: RegisteredStage::Loaded {
                tmu: 0,
                binding: FinishedStageBinding::Images {
                    playback: FinishedImagePlayback::Single { image: 5 },
                },
            },
            profile,
            kind: ImplicitShaderKind::Dynamic,
        })
        .unwrap();
        assert_eq!(compiled.finished.sort, 3);
        let _ = (FinishHardware::Generic, Waveform::zero(WaveKind::None));
        let _ = parse_shader_script("a\n{\n skyparms x 1 y\n}\n", "<t>").unwrap();
        let _ = normalize_image_name("A\\B");
    }
}
