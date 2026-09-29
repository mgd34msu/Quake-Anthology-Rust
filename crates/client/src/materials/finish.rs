//! `FinishShader` and `VertexLightingCollapse` (`tr_shader.c`).
//!
//! Donor provenance: `src/materials/material-finish.ts`.

use super::compile::{FinishedStageBinding, RegisteredStage};
use super::fog::{FogAdjustment, FogPass};
use super::iterator::{
    source_material_iterator, FinishedAlphaGen, FinishedIteratorStage, IteratorBinding, MaterialIterator,
    MaterialIteratorInput, MaterialIteratorProfile,
};
use super::material::{
    AlphaGen, ColorGen, ParsedStage, ShaderDefinition, ShaderMap, ShaderStage, SourceAlphaGen, SourceColorGen,
    SourceTcGen, SourceWaveFunc, SourceWaveStorage,
};
use super::state::bits as state_bits;
use super::state::{source_state_bits, PolygonMode, SourceStateInput};
use super::state::{CullFace, DepthTest, FILTER_BLEND, OPAQUE_BLEND};
use crate::ClientError;

/// Finish profile (`FinishShaderProfile`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FinishShaderProfile {
    /// Detail textures enabled.
    pub detail_textures: bool,
    /// Vertex lighting forced.
    pub vertex_light: bool,
    /// Fullscreen UI shader.
    pub ui_fullscreen: bool,
    /// Hardware workaround.
    pub hardware: FinishHardware,
    /// Iterator profile.
    pub iterator: MaterialIteratorProfile,
}

/// Finish hardware selector.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FinishHardware {
    /// Generic.
    Generic,
    /// Permedia2 (forces vertex-light collapse).
    Permedia2,
}

/// A finished shader stage (`FinishedShaderStage`).
pub type FinishedShaderStage = FinishedIteratorStage;

/// Finish diagnostic (`FinishShaderDiagnostic`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FinishShaderDiagnostic {
    /// Diagnostic kind.
    pub kind: FinishDiagnosticKind,
    /// Stage index.
    pub stage: Option<usize>,
    /// Message.
    pub message: String,
}

/// Finish diagnostic kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FinishDiagnosticKind {
    /// Stage has no image.
    MissingImage,
    /// Lightmap cleared (no lightmap stage).
    LightmapCleared,
}

/// A finished shader (`FinishedShader`).
#[derive(Debug, Clone, PartialEq)]
pub struct FinishedShader {
    /// Sort key.
    pub sort: i32,
    /// Lightmap index.
    pub lightmap_index: i32,
    /// Has a lightmap stage.
    pub has_lightmap_stage: bool,
    /// Stages before `CollapseMultitexture`.
    pub source_stages: Vec<FinishedShaderStage>,
    /// Pass count after collapse.
    pub num_unfogged_passes: usize,
    /// Computed iterator.
    pub iterator: MaterialIterator,
    /// Fog pass.
    pub fog_pass: FogPass,
    /// Diagnostics.
    pub diagnostics: Vec<FinishShaderDiagnostic>,
}

/// Finish input (`FinishShaderInput`).
#[derive(Debug, Clone, PartialEq)]
pub struct FinishShaderInput {
    /// Parsed definition.
    pub definition: ShaderDefinition,
    /// Lightmap index.
    pub lightmap_index: i32,
    /// One image result per parsed stage.
    pub images: Vec<RegisteredStage>,
    /// Profile.
    pub profile: FinishShaderProfile,
}

/// Implicit shader input (`FinishImplicitShaderInput`).
#[derive(Debug, Clone, PartialEq)]
pub struct FinishImplicitShaderInput {
    /// Shader name.
    pub name: String,
    /// Base image.
    pub base_image: RegisteredStage,
    /// Profile.
    pub profile: FinishShaderProfile,
    /// Implicit kind.
    pub kind: ImplicitShaderKind,
}

/// Implicit shader kind.
#[derive(Debug, Clone, PartialEq)]
pub enum ImplicitShaderKind {
    /// Default.
    Default,
    /// Stencil shadow.
    StencilShadow,
    /// Dynamic.
    Dynamic,
    /// Vertex.
    Vertex,
    /// Picture.
    Picture,
    /// White + base.
    White {
        /// White image.
        white_image: RegisteredStage,
    },
    /// Lightmap + base.
    Lightmap {
        /// Lightmap index.
        lightmap_index: i32,
        /// Lightmap image.
        lightmap_image: RegisteredStage,
    },
}

fn alpha_kind(source: SourceAlphaGen, _semantic: &AlphaGen) -> FinishedAlphaGen {
    match source {
        SourceAlphaGen::Identity => FinishedAlphaGen::Identity,
        SourceAlphaGen::Skip => FinishedAlphaGen::Skip,
        SourceAlphaGen::Entity => FinishedAlphaGen::Entity,
        SourceAlphaGen::OneMinusEntity => FinishedAlphaGen::OneMinusEntity,
        SourceAlphaGen::Vertex => FinishedAlphaGen::Vertex,
        SourceAlphaGen::OneMinusVertex => FinishedAlphaGen::OneMinusVertex,
        SourceAlphaGen::LightingSpecular => FinishedAlphaGen::LightingSpecular,
        SourceAlphaGen::Waveform => FinishedAlphaGen::Wave,
        SourceAlphaGen::Portal => FinishedAlphaGen::Portal,
        SourceAlphaGen::Const => FinishedAlphaGen::Const,
    }
}

#[derive(Debug, Clone)]
struct WorkingStage {
    stage: ShaderStage,
    state_bits: u32,
    active: bool,
    image_tmu: Option<u8>,
    binding: Option<FinishedStageBinding>,
    alpha_gen: FinishedAlphaGen,
    rgb_gen: SourceColorGen,
    rgb_wave: SourceWaveStorage,
    alpha_wave: SourceWaveStorage,
    is_lightmap: bool,
    vertex_lightmap: bool,
    tc_gen: SourceTcGen,
    fog_adjustment: FogAdjustment,
}

fn working_stage(stage: &ParsedStage, image: &RegisteredStage, portal_range: f32) -> WorkingStage {
    let mut semantic = stage.stage.clone();
    if matches!(semantic.alpha_gen, AlphaGen::Portal(_)) {
        semantic.alpha_gen = AlphaGen::Portal(portal_range);
    }
    let (active, image_tmu, binding) = match image {
        RegisteredStage::Loaded { tmu, binding } => (stage.source_state.active, Some(*tmu), Some(binding.clone())),
        RegisteredStage::Missing => (false, None, None),
    };
    WorkingStage {
        stage: semantic,
        state_bits: stage.source_state.state_bits,
        active: active && image_tmu.is_some(),
        image_tmu,
        binding,
        alpha_gen: alpha_kind(stage.source_state.alpha_gen, &stage.stage.alpha_gen),
        rgb_gen: stage.source_state.rgb_gen,
        rgb_wave: stage.source_state.rgb_wave,
        alpha_wave: stage.source_state.alpha_wave,
        is_lightmap: stage.source_state.is_lightmap,
        vertex_lightmap: stage.source_state.vertex_lightmap,
        tc_gen: stage.source_state.tc_gen,
        fog_adjustment: FogAdjustment::None,
    }
}

fn is_blended(stage: &WorkingStage) -> bool {
    stage.state_bits & state_bits::BLEND_MASK != 0
}

fn stage_fog_adjustment(stage: &WorkingStage) -> FogAdjustment {
    let blend = stage.state_bits & state_bits::BLEND_MASK;
    if blend == state_bits::SRCBLEND_ONE | state_bits::DSTBLEND_ONE
        || blend == state_bits::SRCBLEND_ZERO | state_bits::DSTBLEND_ONE_MINUS_SRC_COLOR
    {
        return FogAdjustment::Rgb;
    }
    if blend == state_bits::SRCBLEND_SRC_ALPHA | state_bits::DSTBLEND_ONE_MINUS_SRC_ALPHA {
        return FogAdjustment::Alpha;
    }
    if blend == state_bits::SRCBLEND_ONE | state_bits::DSTBLEND_ONE_MINUS_SRC_ALPHA {
        return FogAdjustment::Rgba;
    }
    FogAdjustment::None
}

fn vertex_lighting_collapse(stages: &mut [Option<WorkingStage>], sort: i32, lightmap_index: i32) {
    if stages.first().and_then(|stage| stage.as_ref()).is_none() {
        return;
    }
    if sort == 3 {
        let mut best_index = 0usize;
        let mut best_rank = -999_999i32;
        for index in 0..8 {
            let Some(candidate) = stages.get(index).and_then(|stage| stage.as_ref()) else {
                break;
            };
            if !candidate.active {
                break;
            }
            let mut rank = 0i32;
            if candidate.is_lightmap {
                rank -= 100;
            }
            if candidate.tc_gen != SourceTcGen::Texture {
                rank -= 5;
            }
            if !candidate.stage.tc_mods.is_empty() {
                rank -= 5;
            }
            if candidate.rgb_gen != SourceColorGen::Identity && candidate.rgb_gen != SourceColorGen::IdentityLighting {
                rank -= 3;
            }
            if rank > best_rank {
                best_rank = rank;
                best_index = index;
            }
        }
        let best = match stages.get(best_index).and_then(|stage| stage.clone()) {
            Some(best) => best,
            None => return,
        };
        if let Some(Some(first)) = stages.first_mut() {
            first.stage.map = best.stage.map.clone();
            first.stage.tc_gen = best.stage.tc_gen;
            first.stage.tc_mods.clone_from(&best.stage.tc_mods);
            first.image_tmu = best.image_tmu;
            first.binding.clone_from(&best.binding);
            first.is_lightmap = best.is_lightmap;
            first.vertex_lightmap = best.vertex_lightmap;
            first.tc_gen = best.tc_gen;
            first.stage.blend = OPAQUE_BLEND;
            first.stage.depth_write = true;
            first.stage.rgb_gen = if lightmap_index == -1 {
                ColorGen::LightingDiffuse
            } else {
                ColorGen::ExactVertex
            };
            first.state_bits = (first.state_bits & !state_bits::BLEND_MASK) | state_bits::DEPTHMASK_TRUE;
            first.rgb_gen = if lightmap_index == -1 {
                SourceColorGen::LightingDiffuse
            } else {
                SourceColorGen::ExactVertex
            };
            first.alpha_gen = FinishedAlphaGen::Skip;
        }
    } else {
        let second = match stages.get(1).and_then(|stage| stage.clone()) {
            Some(second) => second,
            None => return,
        };
        let first_is_lightmap = stages
            .first()
            .and_then(|stage| stage.as_ref())
            .is_some_and(|first| first.is_lightmap);
        if first_is_lightmap {
            stages[0] = Some(second.clone());
        }
        let collapsed = match stages.first_mut().and_then(|stage| stage.as_mut()) {
            Some(collapsed) => collapsed,
            None => return,
        };
        if collapsed.rgb_gen == SourceColorGen::OneMinusEntity || second.rgb_gen == SourceColorGen::OneMinusEntity {
            collapsed.stage.rgb_gen = ColorGen::IdentityLighting;
            collapsed.rgb_gen = SourceColorGen::IdentityLighting;
        }
        if collapsed.rgb_gen == SourceColorGen::Waveform && second.rgb_gen == SourceColorGen::Waveform {
            let a = collapsed.rgb_wave.func;
            let b = second.rgb_wave.func;
            if (a == SourceWaveFunc::Sawtooth && b == SourceWaveFunc::InverseSawtooth)
                || (a == SourceWaveFunc::InverseSawtooth && b == SourceWaveFunc::Sawtooth)
            {
                collapsed.stage.rgb_gen = ColorGen::IdentityLighting;
                collapsed.rgb_gen = SourceColorGen::IdentityLighting;
            }
        }
    }
    for stage in stages.iter_mut().skip(1).take(7) {
        *stage = None;
    }
}

fn finished_stage(stage: &WorkingStage) -> FinishedShaderStage {
    let binding = match (&stage.binding, stage.image_tmu) {
        (Some(binding), Some(tmu)) if stage.active => Some(IteratorBinding {
            image_tmu: tmu,
            binding: binding.clone(),
        }),
        _ => None,
    };
    FinishedShaderStage {
        stage: stage.stage.clone(),
        state_bits: stage.state_bits,
        rgb_gen: stage.rgb_gen,
        fog_adjustment: stage.fog_adjustment,
        alpha_gen: stage.alpha_gen,
        tc_gen: stage.tc_gen,
        rgb_wave: stage.rgb_wave,
        alpha_wave: stage.alpha_wave,
        is_lightmap: stage.is_lightmap,
        vertex_lightmap: stage.vertex_lightmap,
        binding,
    }
}

/// Finish a shader (`finishShader`).
pub fn finish_shader(input: &FinishShaderInput) -> Result<FinishedShader, ClientError> {
    if input.lightmap_index < -4 {
        return Err(ClientError::BadMaterial(
            "FinishShader lightmapIndex must be a source lightmap sentinel or non-negative int32".to_string(),
        ));
    }
    if input.images.len() != input.definition.stages.len() {
        return Err(ClientError::BadMaterial(
            "FinishShader needs exactly one image result for each parsed stage".to_string(),
        ));
    }
    let mut stages: Vec<Option<WorkingStage>> = Vec::with_capacity(8);
    for (index, parsed) in input.definition.stages.iter().enumerate() {
        let image = &input.images[index];
        stages.push(Some(working_stage(parsed, image, input.definition.portal_range)));
    }
    while stages.len() < 8 {
        stages.push(None);
    }

    let mut diagnostics = Vec::new();
    let mut sort = input.definition.sort.unwrap_or(0.0) as i32;
    if input.definition.sky.is_some() {
        sort = 2;
    }
    if input.definition.polygon_offset && sort == 0 {
        sort = 4;
    }
    let mut has_lightmap_stage = false;
    let mut stage_index = 0usize;
    while stage_index < 8 {
        let blended_first = stages[0].as_ref().is_some_and(is_blended);
        let current = match stages[stage_index].as_mut() {
            Some(current) => current,
            None => break,
        };
        if !current.active {
            diagnostics.push(FinishShaderDiagnostic {
                kind: FinishDiagnosticKind::MissingImage,
                stage: Some(stage_index),
                message: format!("Shader {} has a stage with no image", input.definition.name),
            });
            stage_index += 1;
            continue;
        }
        if current.stage.detail && !input.profile.detail_textures {
            if stage_index < 7 {
                for offset in stage_index..7 {
                    let next = stages[offset + 1].clone();
                    stages[offset] = next;
                }
                stages[stage_index + 1] = None;
            }
            stage_index += 1;
            continue;
        }
        if current.tc_gen == SourceTcGen::Bad {
            current.tc_gen = if current.is_lightmap {
                SourceTcGen::Lightmap
            } else {
                SourceTcGen::Texture
            };
            current.stage.tc_gen = if current.is_lightmap {
                super::material::TexGen::Lightmap
            } else {
                super::material::TexGen::Texture
            };
        }
        if current.is_lightmap {
            has_lightmap_stage = true;
        }
        if is_blended(current) && blended_first {
            current.fog_adjustment = stage_fog_adjustment(current);
            if sort == 0 {
                sort = if current.state_bits & state_bits::DEPTHMASK_TRUE != 0 {
                    5
                } else {
                    9
                };
            }
        }
        stage_index += 1;
    }
    if sort == 0 {
        sort = 3;
    }

    if stage_index > 1
        && ((input.profile.vertex_light && !input.profile.ui_fullscreen)
            || input.profile.hardware == FinishHardware::Permedia2)
    {
        vertex_lighting_collapse(&mut stages, sort, input.lightmap_index);
        stage_index = 1;
        has_lightmap_stage = false;
    }

    let mut lightmap_index = input.lightmap_index;
    if lightmap_index >= 0 && !has_lightmap_stage {
        diagnostics.push(FinishShaderDiagnostic {
            kind: FinishDiagnosticKind::LightmapCleared,
            stage: None,
            message: format!("Shader {} has lightmap but no lightmap stage", input.definition.name),
        });
        lightmap_index = -1;
    }

    let mut source_stages = Vec::new();
    for stage in stages.iter().take(stage_index) {
        let Some(stage) = stage else { break };
        source_stages.push(finished_stage(stage));
    }
    let iterator = source_material_iterator(
        &MaterialIteratorInput {
            stages: source_stages.clone(),
            sky: input.definition.sky.is_some(),
            polygon_offset: input.definition.polygon_offset,
            deform_count: input.definition.deforms.len(),
        },
        &input.profile.iterator,
    );
    let num_unfogged_passes = iterator.passes.len();
    if num_unfogged_passes == 0 {
        sort = 7;
    }
    let fog_pass = if sort <= 3 {
        FogPass::Equal
    } else if input.definition.surface_parms.iter().any(|parm| parm == "fog") {
        FogPass::LessEqual
    } else {
        FogPass::None
    };
    Ok(FinishedShader {
        sort,
        lightmap_index,
        has_lightmap_stage,
        source_stages,
        num_unfogged_passes,
        iterator,
        fog_pass,
        diagnostics,
    })
}

#[allow(clippy::too_many_arguments)]
fn implicit_stage(
    map: ShaderMap,
    rgb_gen: ColorGen,
    source_rgb_gen: SourceColorGen,
    alpha_gen: AlphaGen,
    source_alpha_gen: SourceAlphaGen,
    blend: super::state::Blend,
    depth_func: DepthTest,
    depth_write: bool,
    is_lightmap: bool,
) -> ParsedStage {
    let blend_input = if blend == OPAQUE_BLEND { None } else { Some(blend) };
    let state_bits = source_state_bits(
        &SourceStateInput {
            depth_test: match depth_func {
                DepthTest::Always => DepthTest::LessEqual,
                test => test,
            },
            depth_write,
            blend: blend_input,
            alpha_test: super::state::AlphaTest::None,
        },
        PolygonMode::Fill,
        depth_func != DepthTest::Always,
    )
    .unwrap_or(0);
    ParsedStage {
        stage: ShaderStage {
            map,
            blend,
            depth_func,
            depth_write,
            alpha_func: super::state::AlphaTest::None,
            detail: false,
            rgb_gen,
            alpha_gen,
            tc_gen: if is_lightmap {
                super::material::TexGen::Lightmap
            } else {
                super::material::TexGen::Texture
            },
            tc_mods: Vec::new(),
        },
        source_state: super::material::SourceStageState {
            active: true,
            state_bits,
            rgb_gen: source_rgb_gen,
            alpha_gen: source_alpha_gen,
            tc_gen: SourceTcGen::Bad,
            rgb_wave: SourceWaveStorage::zero(),
            alpha_wave: SourceWaveStorage::zero(),
            is_lightmap,
            vertex_lightmap: false,
        },
    }
}

fn implicit_definition(name: &str, stages: Vec<ParsedStage>, sort: Option<f32>) -> ShaderDefinition {
    ShaderDefinition {
        name: name.to_string(),
        stages,
        surface_parms: Vec::new(),
        cull: CullFace::Front,
        sort,
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
    }
}

/// Build implicit-shader finish input (`implicitShaderInput`).
pub fn implicit_shader_input(input: &FinishImplicitShaderInput) -> Result<FinishShaderInput, ClientError> {
    let is_picture = matches!(input.kind, ImplicitShaderKind::Picture);
    let base_map = ShaderMap::Image {
        name: input.name.clone(),
        clamp: is_picture,
    };
    let (lightmap_index, stages, images, sort) = match &input.kind {
        ImplicitShaderKind::Default | ImplicitShaderKind::StencilShadow => (
            -1,
            vec![implicit_stage(
                base_map,
                ColorGen::IdentityLighting,
                SourceColorGen::Bad,
                AlphaGen::Identity,
                SourceAlphaGen::Identity,
                OPAQUE_BLEND,
                DepthTest::LessEqual,
                true,
                false,
            )],
            vec![input.base_image.clone()],
            if matches!(input.kind, ImplicitShaderKind::StencilShadow) {
                Some(14.0)
            } else {
                None
            },
        ),
        ImplicitShaderKind::Dynamic => (
            -1,
            vec![implicit_stage(
                base_map,
                ColorGen::LightingDiffuse,
                SourceColorGen::LightingDiffuse,
                AlphaGen::Identity,
                SourceAlphaGen::Identity,
                OPAQUE_BLEND,
                DepthTest::LessEqual,
                true,
                false,
            )],
            vec![input.base_image.clone()],
            None,
        ),
        ImplicitShaderKind::Vertex => (
            -3,
            vec![implicit_stage(
                base_map,
                ColorGen::ExactVertex,
                SourceColorGen::ExactVertex,
                AlphaGen::Identity,
                SourceAlphaGen::Skip,
                OPAQUE_BLEND,
                DepthTest::LessEqual,
                true,
                false,
            )],
            vec![input.base_image.clone()],
            None,
        ),
        ImplicitShaderKind::Picture => (
            -4,
            vec![implicit_stage(
                base_map,
                ColorGen::Vertex,
                SourceColorGen::Vertex,
                AlphaGen::Vertex,
                SourceAlphaGen::Vertex,
                super::state::Blend {
                    source: super::state::BlendFactor::SrcAlpha,
                    destination: super::state::BlendFactor::OneMinusSrcAlpha,
                },
                DepthTest::Always,
                false,
                false,
            )],
            vec![input.base_image.clone()],
            None,
        ),
        ImplicitShaderKind::White { white_image } => (
            -2,
            vec![
                implicit_stage(
                    ShaderMap::WhiteImage,
                    ColorGen::IdentityLighting,
                    SourceColorGen::IdentityLighting,
                    AlphaGen::Identity,
                    SourceAlphaGen::Identity,
                    OPAQUE_BLEND,
                    DepthTest::LessEqual,
                    true,
                    false,
                ),
                implicit_stage(
                    base_map,
                    ColorGen::Identity,
                    SourceColorGen::Identity,
                    AlphaGen::Identity,
                    SourceAlphaGen::Identity,
                    FILTER_BLEND,
                    DepthTest::LessEqual,
                    false,
                    false,
                ),
            ],
            vec![white_image.clone(), input.base_image.clone()],
            None,
        ),
        ImplicitShaderKind::Lightmap {
            lightmap_index,
            lightmap_image,
        } => {
            if *lightmap_index < 0 {
                return Err(ClientError::BadMaterial(
                    "Implicit lightmap index must be a non-negative int32".to_string(),
                ));
            }
            (
                *lightmap_index,
                vec![
                    implicit_stage(
                        ShaderMap::Lightmap,
                        ColorGen::Identity,
                        SourceColorGen::Identity,
                        AlphaGen::Identity,
                        SourceAlphaGen::Identity,
                        OPAQUE_BLEND,
                        DepthTest::LessEqual,
                        true,
                        true,
                    ),
                    implicit_stage(
                        base_map,
                        ColorGen::Identity,
                        SourceColorGen::Identity,
                        AlphaGen::Identity,
                        SourceAlphaGen::Identity,
                        FILTER_BLEND,
                        DepthTest::LessEqual,
                        false,
                        false,
                    ),
                ],
                vec![lightmap_image.clone(), input.base_image.clone()],
                None,
            )
        }
    };
    Ok(FinishShaderInput {
        definition: implicit_definition(&input.name, stages, sort),
        lightmap_index,
        images,
        profile: input.profile,
    })
}

/// Finish an implicit shader (`finishImplicitShader`).
pub fn finish_implicit_shader(input: &FinishImplicitShaderInput) -> Result<FinishedShader, ClientError> {
    let prepared = implicit_shader_input(input)?;
    finish_shader(&prepared)
}

/// Finish a failed shader (`finishFailedShader`).
pub fn finish_failed_shader(
    name: &str,
    lightmap_index: i32,
    profile: FinishShaderProfile,
) -> Result<FinishedShader, ClientError> {
    finish_shader(&FinishShaderInput {
        definition: implicit_definition(name, Vec::new(), None),
        lightmap_index,
        images: Vec::new(),
        profile,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::materials::compile::FinishedImagePlayback;
    use crate::materials::iterator::{IteratorDriver, MaterialIteratorProfile};
    use crate::materials::material::parse_shader_script;
    use crate::materials::state::ADDITIVE_BLEND;

    fn profile() -> FinishShaderProfile {
        FinishShaderProfile {
            detail_textures: true,
            vertex_light: false,
            ui_fullscreen: false,
            hardware: FinishHardware::Generic,
            iterator: MaterialIteratorProfile {
                ignore_fast_path: false,
                multitexture: true,
                texture_env_add: true,
                driver: IteratorDriver::Generic,
            },
        }
    }

    #[test]
    fn opaque_shader_sorts_opaque() {
        let definitions = parse_shader_script("rock\n{\n {\n map textures/rock.tga\n }\n}\n", "<test>").unwrap();
        let finished = finish_shader(&FinishShaderInput {
            definition: definitions[0].clone(),
            lightmap_index: -1,
            images: vec![RegisteredStage::Loaded {
                tmu: 0,
                binding: FinishedStageBinding::Images {
                    playback: FinishedImagePlayback::Single { image: 7 },
                },
            }],
            profile: profile(),
        })
        .unwrap();
        assert_eq!(finished.sort, 3);
        assert_eq!(finished.fog_pass, FogPass::Equal);
        assert!(finished.diagnostics.is_empty());
        let _ = ADDITIVE_BLEND;
    }

    #[test]
    fn missing_image_keeps_diagnostic() {
        let definitions = parse_shader_script("rock\n{\n {\n map textures/rock.tga\n }\n}\n", "<test>").unwrap();
        let finished = finish_shader(&FinishShaderInput {
            definition: definitions[0].clone(),
            lightmap_index: -1,
            images: vec![RegisteredStage::Missing],
            profile: profile(),
        })
        .unwrap();
        assert_eq!(finished.diagnostics.len(), 1);
        assert_eq!(finished.diagnostics[0].kind, FinishDiagnosticKind::MissingImage);
    }

    #[test]
    fn implicit_default_is_opaque() {
        let finished = finish_implicit_shader(&FinishImplicitShaderInput {
            name: "implicit".to_string(),
            base_image: RegisteredStage::Loaded {
                tmu: 0,
                binding: FinishedStageBinding::RetainCurrentTexture,
            },
            profile: profile(),
            kind: ImplicitShaderKind::Default,
        })
        .unwrap();
        assert_eq!(finished.sort, 3);
    }
}
