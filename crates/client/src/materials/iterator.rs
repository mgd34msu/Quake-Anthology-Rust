//! Stage iterators (`CollapseMultitexture` / `ComputeStageIteratorFunc`).
//!
//! Donor provenance: `src/materials/material-iterator.ts` (from
//! `renderer/tr_shader.c`).

use super::material::{ShaderStage, SourceColorGen, SourceTcGen, SourceWaveStorage};
use super::state::Blend;
use super::state::bits as state_bits;
use crate::materials::fog::FogAdjustment;

/// Finished alpha generator after `ParseStage`'s numeric comparison.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FinishedAlphaGen {
    /// Identity.
    Identity,
    /// Skip.
    Skip,
    /// Entity.
    Entity,
    /// One minus entity.
    OneMinusEntity,
    /// Vertex.
    Vertex,
    /// One minus vertex.
    OneMinusVertex,
    /// Lighting specular.
    LightingSpecular,
    /// Waveform.
    Wave,
    /// Portal.
    Portal,
    /// Constant.
    Const,
}

/// A finished iterator stage (`FinishedIteratorStage`).
#[derive(Debug, Clone, PartialEq)]
pub struct FinishedIteratorStage {
    /// Semantic stage.
    pub stage: ShaderStage,
    /// State bits.
    pub state_bits: u32,
    /// Numeric RGB generator.
    pub rgb_gen: SourceColorGen,
    /// Fog adjustment.
    pub fog_adjustment: FogAdjustment,
    /// Finished alpha generator.
    pub alpha_gen: FinishedAlphaGen,
    /// Numeric coordinate generator.
    pub tc_gen: SourceTcGen,
    /// RGB wave storage.
    pub rgb_wave: SourceWaveStorage,
    /// Alpha wave storage.
    pub alpha_wave: SourceWaveStorage,
    /// Lightmap stage.
    pub is_lightmap: bool,
    /// Vertex lightmap (never assigned by source).
    pub vertex_lightmap: bool,
    /// Active with binding.
    pub binding: Option<IteratorBinding>,
}

/// Active stage binding.
#[derive(Debug, Clone, PartialEq)]
pub struct IteratorBinding {
    /// Texture unit.
    pub image_tmu: u8,
    /// Stage binding.
    pub binding: crate::materials::compile::FinishedStageBinding,
}

impl FinishedIteratorStage {
    /// Whether the stage is active.
    #[must_use]
    pub fn active(&self) -> bool {
        self.binding.is_some()
    }
}

/// Iterator hardware profile (`MaterialIteratorProfile`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MaterialIteratorProfile {
    /// Ignore fast paths.
    pub ignore_fast_path: bool,
    /// Multitexture available.
    pub multitexture: bool,
    /// Texture-env-add available.
    pub texture_env_add: bool,
    /// Driver.
    pub driver: IteratorDriver,
}

/// Iterator driver.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IteratorDriver {
    /// Generic.
    Generic,
    /// Voodoo.
    Voodoo,
}

/// Iterator input (`MaterialIteratorInput`).
#[derive(Debug, Clone, PartialEq)]
pub struct MaterialIteratorInput {
    /// Finished stages.
    pub stages: Vec<FinishedIteratorStage>,
    /// Sky material.
    pub sky: bool,
    /// Polygon offset.
    pub polygon_offset: bool,
    /// Deform count.
    pub deform_count: usize,
}

/// One iterator pass (`IteratorPass`).
#[derive(Debug, Clone, PartialEq)]
pub struct IteratorPass {
    /// Semantic stage.
    pub stage: ShaderStage,
    /// State bits.
    pub state_bits: u32,
    /// Numeric RGB generator.
    pub rgb_gen: SourceColorGen,
    /// Fog adjustment.
    pub fog_adjustment: FogAdjustment,
    /// Finished alpha generator.
    pub alpha_gen: FinishedAlphaGen,
    /// One or two bundles.
    pub bundles: Vec<FinishedIteratorStage>,
}

/// Multitexture environment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MultitextureEnv {
    /// None.
    None,
    /// Modulate.
    Modulate,
    /// Add.
    Add,
}

/// Iterator kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MaterialIteratorKind {
    /// Generic.
    Generic,
    /// Vertex lit.
    VertexLit,
    /// Lightmapped multitexture.
    LightmappedMultitexture,
    /// Sky.
    Sky,
}

/// Computed iterator (`MaterialIterator`).
#[derive(Debug, Clone, PartialEq)]
pub struct MaterialIterator {
    /// Iterator kind.
    pub kind: MaterialIteratorKind,
    /// Passes.
    pub passes: Vec<IteratorPass>,
    /// Multitexture environment.
    pub multitexture_env: MultitextureEnv,
}

const FILTER_SOURCE: u32 = state_bits::SRCBLEND_DST_COLOR | state_bits::DSTBLEND_ZERO;
const FILTER_DESTINATION: u32 = state_bits::SRCBLEND_ZERO | state_bits::DSTBLEND_SRC_COLOR;
const ADDITIVE: u32 = state_bits::SRCBLEND_ONE | state_bits::DSTBLEND_ONE;

fn collapse_blend(first: u32, second: u32) -> Option<(MultitextureEnv, Blend, u32)> {
    use super::state::{BlendFactor, ADDITIVE_BLEND, FILTER_BLEND, OPAQUE_BLEND};
    if second == FILTER_SOURCE || second == FILTER_DESTINATION {
        if first == 0 {
            return Some((MultitextureEnv::Modulate, OPAQUE_BLEND, 0));
        }
        if first == FILTER_SOURCE || first == FILTER_DESTINATION {
            return Some((MultitextureEnv::Modulate, FILTER_BLEND, FILTER_SOURCE));
        }
    }
    if second == ADDITIVE {
        if first == 0 {
            return Some((MultitextureEnv::Add, OPAQUE_BLEND, 0));
        }
        if first == ADDITIVE {
            return Some((MultitextureEnv::Add, ADDITIVE_BLEND, ADDITIVE));
        }
    }
    let _ = BlendFactor::One;
    None
}

fn same_wave(a: &SourceWaveStorage, b: &SourceWaveStorage) -> bool {
    a.func == b.func
        && a.base.to_bits() == b.base.to_bits()
        && a.amplitude.to_bits() == b.amplitude.to_bits()
        && a.phase.to_bits() == b.phase.to_bits()
        && a.frequency.to_bits() == b.frequency.to_bits()
}

/// Compute the source material iterator (`sourceMaterialIterator`).
///
/// Only the first two stages are considered once; the source does not
/// collapse to a fixed point.
#[must_use]
pub fn source_material_iterator(
    input: &MaterialIteratorInput,
    profile: &MaterialIteratorProfile,
) -> MaterialIterator {
    let mut passes: Vec<IteratorPass> = input
        .stages
        .iter()
        .map(|stage| IteratorPass {
            stage: stage.stage.clone(),
            state_bits: stage.state_bits,
            rgb_gen: stage.rgb_gen,
            fog_adjustment: stage.fog_adjustment,
            alpha_gen: stage.alpha_gen,
            bundles: vec![stage.clone()],
        })
        .collect();
    let mut multitexture_env = MultitextureEnv::None;
    if profile.multitexture && input.stages.len() >= 2 {
        let a = &input.stages[0];
        let b = &input.stages[1];
        let tmu_ok = profile.driver != IteratorDriver::Voodoo
            || a.binding.as_ref().map(|binding| binding.image_tmu)
                != b.binding.as_ref().map(|binding| binding.image_tmu);
        let mask = state_bits::BLEND_MASK | state_bits::DEPTHMASK_TRUE;
        if a.active()
            && b.active()
            && tmu_ok
            && (a.state_bits & !mask) == (b.state_bits & !mask)
            && a.rgb_gen == b.rgb_gen
            && a.alpha_gen == b.alpha_gen
            && (a.rgb_gen != SourceColorGen::Waveform || same_wave(&a.rgb_wave, &b.rgb_wave))
            // Original typo compares alphaGen to CGEN_WAVEFORM (8), i.e. AGEN_PORTAL.
            && (a.alpha_gen != FinishedAlphaGen::Portal
                || same_wave(&a.alpha_wave, &b.alpha_wave))
        {
            let collapsed = collapse_blend(
                a.state_bits & state_bits::BLEND_MASK,
                b.state_bits & state_bits::BLEND_MASK,
            );
            if let Some((env, blend, bits)) = collapsed {
                let add_ok = env != MultitextureEnv::Add
                    || profile.texture_env_add && a.rgb_gen == SourceColorGen::Identity;
                if add_ok {
                    multitexture_env = env;
                    let bundles = if a.is_lightmap {
                        vec![b.clone(), a.clone()]
                    } else {
                        vec![a.clone(), b.clone()]
                    };
                    let mut stage = a.stage.clone();
                    stage.blend = blend;
                    passes.splice(
                        0..2,
                        [IteratorPass {
                            stage,
                            state_bits: (a.state_bits & !state_bits::BLEND_MASK) | bits,
                            rgb_gen: a.rgb_gen,
                            fog_adjustment: a.fog_adjustment,
                            alpha_gen: a.alpha_gen,
                            bundles,
                        }],
                    );
                }
            }
        }
    }
    let mut kind = MaterialIteratorKind::Generic;
    if input.sky {
        kind = MaterialIteratorKind::Sky;
    } else if !profile.ignore_fast_path
        && passes.len() == 1
        && !input.polygon_offset
        && input.deform_count == 0
    {
        let first = passes.first();
        let bundle = first.and_then(|first| first.bundles.first());
        let (Some(first), Some(bundle)) = (first, bundle) else {
            return MaterialIterator {
                kind,
                passes,
                multitexture_env,
            };
        };
        if first.rgb_gen == SourceColorGen::LightingDiffuse
            && first.alpha_gen == FinishedAlphaGen::Identity
            && bundle.tc_gen == SourceTcGen::Texture
            && multitexture_env == MultitextureEnv::None
        {
            kind = MaterialIteratorKind::VertexLit;
        }
        if first.rgb_gen == SourceColorGen::Identity
            && first.alpha_gen == FinishedAlphaGen::Identity
            && bundle.tc_gen == SourceTcGen::Texture
            && first.bundles.get(1).map(|bundle| bundle.tc_gen)
                == Some(SourceTcGen::Lightmap)
            && multitexture_env != MultitextureEnv::None
        {
            kind = MaterialIteratorKind::LightmappedMultitexture;
        }
    }
    MaterialIterator {
        kind,
        passes,
        multitexture_env,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::materials::compile::FinishedStageBinding;
    use crate::materials::material::{
        AlphaGen, ColorGen, ShaderMap, SourceAlphaGen, SourceWaveStorage, TexGen,
    };
    use crate::materials::state::{CullFace, OPAQUE_BLEND};

    fn stage(state: u32) -> FinishedIteratorStage {
        FinishedIteratorStage {
            stage: ShaderStage {
                map: ShaderMap::None,
                blend: OPAQUE_BLEND,
                depth_test: crate::materials::state::DepthTest::LessEqual,
                depth_write: true,
                alpha_func: crate::materials::state::AlphaTest::None,
                detail: false,
                rgb_gen: ColorGen::Identity,
                alpha_gen: AlphaGen::Identity,
                tc_gen: TexGen::Texture,
                tc_mods: Vec::new(),
            },
            state_bits: state,
            rgb_gen: SourceColorGen::Identity,
            fog_adjustment: FogAdjustment::None,
            alpha_gen: FinishedAlphaGen::Identity,
            tc_gen: SourceTcGen::Texture,
            rgb_wave: SourceWaveStorage::zero(),
            alpha_wave: SourceWaveStorage::zero(),
            is_lightmap: false,
            vertex_lightmap: false,
            binding: Some(IteratorBinding {
                image_tmu: 0,
                binding: FinishedStageBinding::RetainCurrentTexture,
            }),
        }
    }

    #[test]
    fn detects_vertex_lit_fast_path() {
        let mut lit = stage(state_bits::DEPTHMASK_TRUE);
        lit.rgb_gen = SourceColorGen::LightingDiffuse;
        let iterator = source_material_iterator(
            &MaterialIteratorInput {
                stages: vec![lit],
                sky: false,
                polygon_offset: false,
                deform_count: 0,
            },
            &MaterialIteratorProfile {
                ignore_fast_path: false,
                multitexture: true,
                texture_env_add: true,
                driver: IteratorDriver::Generic,
            },
        );
        assert_eq!(iterator.kind, MaterialIteratorKind::VertexLit);
    }

    #[test]
    fn sky_forces_sky_iterator() {
        let iterator = source_material_iterator(
            &MaterialIteratorInput {
                stages: vec![stage(0)],
                sky: true,
                polygon_offset: false,
                deform_count: 0,
            },
            &MaterialIteratorProfile {
                ignore_fast_path: false,
                multitexture: true,
                texture_env_add: true,
                driver: IteratorDriver::Generic,
            },
        );
        assert_eq!(iterator.kind, MaterialIteratorKind::Sky);
    }

    #[test]
    fn collapses_filter_pair() {
        let first = stage(0);
        let second = stage(FILTER_SOURCE);
        let iterator = source_material_iterator(
            &MaterialIteratorInput {
                stages: vec![first, second],
                sky: false,
                polygon_offset: false,
                deform_count: 0,
            },
            &MaterialIteratorProfile {
                ignore_fast_path: true,
                multitexture: true,
                texture_env_add: true,
                driver: IteratorDriver::Generic,
            },
        );
        assert_eq!(iterator.passes.len(), 1);
        assert_eq!(iterator.multitexture_env, MultitextureEnv::Modulate);
        let _ = (SourceAlphaGen::Identity, CullFace::Front);
    }
}
