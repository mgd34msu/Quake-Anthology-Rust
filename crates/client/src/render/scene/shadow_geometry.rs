//! Shadow-casting eligibility and shadow-pass deformation
//! (donor `src/render/scene/shadow-geometry.ts`).
//!
//! View models, transparent effects, and sprites have no world-space
//! silhouette; surviving geometry runs the same shader clock and deformation
//! as the color pass before projection into the atlas.

use crate::materials::compile::CompiledMaterial;
use crate::materials::deform::{deform_geometry, DeformView, ProjectionShadowContext, RendererNoise};
use crate::materials::geometry::{MaterialDeformState, MaterialGeometry};
use crate::materials::iterator::MaterialIteratorKind;
use crate::materials::material::VertexDeformation;
use crate::render::error::RenderError;

/// Entity flag family selecting the cast-shadow bit rules.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShadowFlagKind {
    /// Q2 `RF_*` bits.
    Q2,
    /// Q3 `RF_*` bits.
    Q3,
}

/// Whether flag bits and opacity leave a world-space silhouette.
#[must_use]
pub fn scene_flags_cast_shadow(kind: ShadowFlagKind, bits: u32, alpha: f32, view_model: bool) -> bool {
    if view_model {
        return false;
    }
    match kind {
        ShadowFlagKind::Q2 => bits & (4 | 16 | 32 | 128 | 8192 | 0x0020_0000) == 0,
        ShadowFlagKind::Q3 => {
            if bits & (4 | 8 | 64) != 0 {
                return false;
            }
            alpha >= 1.0
        }
    }
}

/// Whether an entity casts, given its flags, opacity, and presentation kind.
#[must_use]
pub fn entity_casts_shadow(flags: u32, flag_kind: ShadowFlagKind, alpha: f32, sprite: bool, view_model: bool) -> bool {
    if sprite || view_model {
        return false;
    }
    scene_flags_cast_shadow(flag_kind, flags, alpha, view_model)
}

/// Shader-clock and deformation inputs for the shadow pass.
pub struct ShadowMaterialContext<'a> {
    /// Shader time in seconds.
    pub time: f32,
    /// Entity shader-time offset in seconds.
    pub time_offset: f32,
    /// Reference definition time in milliseconds.
    pub refdef_time: f32,
    /// Render-text rows for `deformVertexes text`.
    pub render_text: Vec<String>,
    /// Deform view orientation.
    pub deform_view: DeformView,
    /// Shared noise tables.
    pub noise: &'a RendererNoise,
    /// Projection-shadow context, when retained.
    pub projection_shadow: Option<ProjectionShadowContext>,
}

/// Deform geometry for the shadow pass, or `None` for non-casting shaders.
///
/// Sky iterators, translucent sorts, non-drawing surfaces, and billboard
/// deformations never reach the atlas.
pub fn shadow_material_geometry(
    shader: &CompiledMaterial,
    geometry: &MaterialGeometry,
    context: &ShadowMaterialContext<'_>,
) -> Result<Option<MaterialGeometry>, RenderError> {
    let definition = &shader.registered.definition;
    if shader.finished.iterator.kind == MaterialIteratorKind::Sky
        || shader.finished.sort > 3
        || definition
            .surface_parms
            .iter()
            .any(|parm| matches!(parm.as_str(), "nodraw" | "trans" | "water" | "slime" | "lava"))
        || definition.deforms.iter().any(|deform| {
            matches!(
                deform,
                VertexDeformation::Autosprite | VertexDeformation::Autosprite2 | VertexDeformation::ProjectionShadow
            )
        })
    {
        return Ok(None);
    }
    let mut time = context.time - context.time_offset;
    if definition.clamp_time != 0.0 && time >= definition.clamp_time {
        time = definition.clamp_time;
    }
    let mut state = MaterialDeformState::new(
        geometry.clone(),
        context.refdef_time,
        context.render_text.clone(),
        Some(definition.name.clone()),
    );
    let deformed = deform_geometry(
        &mut state,
        &definition.deforms,
        &context.deform_view,
        time,
        context.noise,
        context.projection_shadow.as_ref(),
    )
    .map_err(|error| RenderError::Backend(error.to_string()))?;
    Ok(Some(deformed.into()))
}

#[cfg(test)]
mod tests {
    use qa_core::math::{vec2, vec3};

    use crate::materials::compile::{
        CompiledMaterial, RegisteredExplicitShader, RegistrationOutcome, RenderMaterialView,
    };
    use crate::materials::finish::{FinishShaderDiagnostic, FinishedShader};
    use crate::materials::fog::FogPass;
    use crate::materials::geometry::MaterialVertex;
    use crate::materials::iterator::{MaterialIterator, MaterialIteratorKind, MultitextureEnv};
    use crate::materials::material::ShaderDefinition;
    use crate::materials::state::CullFace;

    use super::*;

    fn definition(surface_parms: &[&str], deforms: Vec<VertexDeformation>) -> ShaderDefinition {
        ShaderDefinition {
            name: "test".to_string(),
            stages: Vec::new(),
            surface_parms: surface_parms.iter().map(ToString::to_string).collect(),
            cull: CullFace::Front,
            sort: None,
            sky: None,
            fog: None,
            sun: None,
            deforms,
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

    fn shader(
        sort: i32,
        kind: MaterialIteratorKind,
        surface_parms: &[&str],
        deforms: Vec<VertexDeformation>,
    ) -> CompiledMaterial {
        CompiledMaterial {
            registered: RegisteredExplicitShader {
                definition: definition(surface_parms, deforms.clone()),
                stages: Vec::new(),
                sky: None,
                outcome: RegistrationOutcome::Defined,
            },
            finished: FinishedShader {
                sort,
                lightmap_index: -1,
                has_lightmap_stage: false,
                source_stages: Vec::new(),
                num_unfogged_passes: 0,
                iterator: MaterialIterator {
                    kind,
                    passes: Vec::new(),
                    multitexture_env: MultitextureEnv::None,
                },
                fog_pass: FogPass::None,
                diagnostics: Vec::<FinishShaderDiagnostic>::new(),
            },
            material: RenderMaterialView {
                name: "test".to_string(),
                stages: Vec::new(),
                deformations: deforms,
                surface_parameters: Vec::new(),
                sort: None,
                cull: CullFace::Front,
                polygon_offset: false,
                no_mipmaps: false,
                no_picmip: false,
                entity_mergable: false,
                portal_range: 0.0,
                clamp_time: 0.0,
                sky: None,
                fog: None,
                sun: None,
            },
        }
    }

    fn geometry() -> MaterialGeometry {
        MaterialGeometry {
            vertices: vec![
                MaterialVertex::new(
                    vec3(0.0, 0.0, 0.0),
                    vec3(0.0, 0.0, 1.0),
                    vec2(0.0, 0.0),
                    vec2(0.0, 0.0),
                    [255, 255, 255, 255],
                ),
                MaterialVertex::new(
                    vec3(16.0, 0.0, 0.0),
                    vec3(0.0, 0.0, 1.0),
                    vec2(1.0, 0.0),
                    vec2(1.0, 0.0),
                    [255, 255, 255, 255],
                ),
                MaterialVertex::new(
                    vec3(0.0, 16.0, 0.0),
                    vec3(0.0, 0.0, 1.0),
                    vec2(0.0, 1.0),
                    vec2(0.0, 1.0),
                    [255, 255, 255, 255],
                ),
            ],
            indices: vec![0, 1, 2],
        }
    }

    fn context(noise: &RendererNoise) -> ShadowMaterialContext<'_> {
        ShadowMaterialContext {
            time: 1.0,
            time_offset: 0.0,
            refdef_time: 0.0,
            render_text: Vec::new(),
            deform_view: DeformView {
                axis: [vec3(1.0, 0.0, 0.0), vec3(0.0, 1.0, 0.0), vec3(0.0, 0.0, 1.0)],
                mirror: false,
                entity_axis: None,
                non_normalized_axis: None,
            },
            noise,
            projection_shadow: None,
        }
    }

    #[test]
    fn q2_bits_reject_shadow_casters() {
        assert!(scene_flags_cast_shadow(ShadowFlagKind::Q2, 0, 1.0, false));
        for bits in [4, 16, 32, 128, 8192, 0x0020_0000] {
            assert!(
                !scene_flags_cast_shadow(ShadowFlagKind::Q2, bits, 1.0, false),
                "bits {bits:#x} should not cast"
            );
        }
        assert!(scene_flags_cast_shadow(ShadowFlagKind::Q2, 1, 0.25, false));
        assert!(!scene_flags_cast_shadow(ShadowFlagKind::Q2, 0, 1.0, true));
    }

    #[test]
    fn q3_bits_and_alpha_gate_shadows() {
        assert!(scene_flags_cast_shadow(ShadowFlagKind::Q3, 0, 1.0, false));
        for bits in [4, 8, 64] {
            assert!(
                !scene_flags_cast_shadow(ShadowFlagKind::Q3, bits, 1.0, false),
                "bits {bits:#x} should not cast"
            );
        }
        assert!(!scene_flags_cast_shadow(ShadowFlagKind::Q3, 0, 0.5, false));
        assert!(!scene_flags_cast_shadow(ShadowFlagKind::Q3, 0, 1.0, true));
    }

    #[test]
    fn sprites_and_view_models_never_cast() {
        assert!(!entity_casts_shadow(0, ShadowFlagKind::Q2, 1.0, true, false));
        assert!(!entity_casts_shadow(0, ShadowFlagKind::Q2, 1.0, false, true));
        assert!(entity_casts_shadow(0, ShadowFlagKind::Q2, 1.0, false, false));
        assert!(!entity_casts_shadow(4, ShadowFlagKind::Q2, 1.0, false, false));
        assert!(!entity_casts_shadow(0, ShadowFlagKind::Q3, 0.5, false, false));
    }

    #[test]
    fn non_casting_shaders_return_none() {
        let noise = RendererNoise::new();
        let context = context(&noise);
        let geometry = geometry();
        let cases = [
            shader(3, MaterialIteratorKind::Sky, &[], Vec::new()),
            shader(4, MaterialIteratorKind::Generic, &[], Vec::new()),
            shader(3, MaterialIteratorKind::Generic, &["nodraw"], Vec::new()),
            shader(3, MaterialIteratorKind::Generic, &["water"], Vec::new()),
            shader(
                3,
                MaterialIteratorKind::Generic,
                &[],
                vec![VertexDeformation::Autosprite],
            ),
            shader(
                3,
                MaterialIteratorKind::Generic,
                &[],
                vec![VertexDeformation::Autosprite2],
            ),
            shader(
                3,
                MaterialIteratorKind::Generic,
                &[],
                vec![VertexDeformation::ProjectionShadow],
            ),
        ];
        for shader in &cases {
            assert_eq!(shadow_material_geometry(shader, &geometry, &context).unwrap(), None);
        }
    }

    #[test]
    fn plain_geometry_passes_through_identical() {
        let noise = RendererNoise::new();
        let context = context(&noise);
        let geometry = geometry();
        let shader = shader(3, MaterialIteratorKind::Generic, &[], Vec::new());
        assert_eq!(
            shadow_material_geometry(&shader, &geometry, &context).unwrap(),
            Some(geometry)
        );
    }
}
