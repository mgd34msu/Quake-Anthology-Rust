//! Ordered Q3 stage evaluation (`tr_shade.c`, `tr_shade_calc.c`).
//!
//! Donor provenance: `src/materials/evaluate.ts`. Also owns the headless
//! batch types shared with the dlight and legacy evaluators (local
//! analogues of the `DrawBatch` contract; no GPU calls).

use qa_core::math::{Vec2, Vec3, Vec4};

use super::color::{evaluate_stage_color, StageColorContext};
use super::compile::{CompiledMaterial, FinishedImagePlayback, FinishedStageBinding};
use super::deform::{deform_geometry, DeformGeometry, DeformView, ProjectionShadowContext, RendererNoise};
use super::dlight::{project_dlight_texture, receives_projected_dlights};
use super::fog::{attenuate_fog_color, fog_pass_state, FogAdjustment};
use super::geometry::{MaterialDeformState, MaterialGeometry, MaterialVertex};
use super::iterator::{
    source_material_iterator, FinishedAlphaGen, FinishedIteratorStage, IteratorDriver, MaterialIteratorInput,
    MaterialIteratorProfile, MultitextureEnv,
};
use super::material::{evaluate_tex_coords, stage_state, TexCoordContext, TexGen, WaveKind};
use super::material::{SourceColorGen, SourceTcGen};
use super::q3_lighting::{DynamicLight, EntityLighting};
use super::state::{source_state_changes, AlphaTest, DepthTest, PolygonMode, RenderState, SourceStateChange};
use crate::ClientError;

/// A batch texture reference.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextureRef {
    /// Bind an image handle.
    BindImage(u32),
    /// Dynamic cinematic image.
    DynamicImage(u32),
    /// Retain current texture.
    RetainCurrentTexture,
}

/// Batch texturing mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Texturing {
    /// Single texture.
    Single,
    /// Paired textures.
    Pair,
}

/// Paired-texture environment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PairEnv {
    /// Modulate.
    Modulate,
    /// Add.
    Add,
}

/// Batch lighting (`BatchLighting` subset).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BatchLighting {
    /// Vertex colors.
    Vertex,
    /// Q2 world lighting pass.
    Q2World {
        /// Pass name.
        pass: Q2LightPass,
    },
}

/// Q2 world lighting pass.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Q2LightPass {
    /// Model.
    Model,
    /// Material lightmap.
    MaterialLightmap,
    /// Texture.
    Texture,
    /// Lightmap.
    Lightmap,
}

/// Batch fog (`BatchFog` subset).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BatchFog {
    /// Density.
    pub density: f32,
    /// Color.
    pub color: Vec3,
    /// Effect.
    pub effect: FogAdjustment,
}

/// A batch vertex.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BatchVertex {
    /// Projected position.
    pub position: Vec4,
    /// Texture coordinates.
    pub tex_coord: Vec2,
    /// Second texture coordinates.
    pub tex_coord2: Option<Vec2>,
    /// Color.
    pub color: Vec4,
}

/// A material batch (headless `DrawBatch` analogue).
#[derive(Debug, Clone, PartialEq)]
pub struct MaterialBatch {
    /// Lighting.
    pub lighting: BatchLighting,
    /// Fragment fog.
    pub fog: Option<BatchFog>,
    /// Texturing mode.
    pub texturing: Texturing,
    /// Render state.
    pub state: RenderState,
    /// Texture.
    pub texture: TextureRef,
    /// Second texture.
    pub second_texture: Option<(TextureRef, PairEnv)>,
    /// Indices.
    pub indices: Vec<u32>,
    /// Vertices.
    pub vertices: Vec<BatchVertex>,
}

/// Dynamic-light input for a draw.
#[derive(Debug, Clone, PartialEq)]
pub struct DynamicLightInput {
    /// Lights.
    pub lights: Vec<DynamicLight>,
    /// Mask.
    pub mask: u32,
    /// Dlight image handle.
    pub image: u32,
}

/// Fog volume input for a draw.
pub struct FogVolumeInput<'a> {
    /// Fog coordinates callback.
    pub coordinates: &'a dyn Fn(Vec3) -> Vec2,
    /// Fog texture.
    pub texture: TextureRef,
    /// Fog color.
    pub color: Vec4,
}

/// Q1 fog input for a draw.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Q1FogInput {
    /// Density.
    pub density: f32,
    /// Color.
    pub color: Vec3,
    /// Texture.
    pub texture: TextureRef,
}

/// Scene-owned dynamic-light batches hook.
pub type DynamicLightBatches<'a> = Option<&'a dyn Fn(&MaterialGeometry) -> Vec<MaterialBatch>>;

/// Material draw context (`MaterialDrawContext`).
pub struct MaterialDrawContext<'a> {
    /// Shader time.
    pub time: f32,
    /// Time offset.
    pub time_offset: f32,
    /// Reference definition time.
    pub refdef_time: f32,
    /// Identity light.
    pub identity_light: f32,
    /// Entity RGBA bytes.
    pub entity_rgba: [u8; 4],
    /// Entity lighting.
    pub lighting: Option<EntityLighting>,
    /// View origin.
    pub view_origin: Vec3,
    /// Local view origin.
    pub local_view_origin: Vec3,
    /// Renderer noise.
    pub noise: &'a RendererNoise,
    /// Shader texcoord.
    pub shader_tex_coord: Vec2,
    /// Deform view.
    pub deform_view: DeformView,
    /// Projection shadow.
    pub projection_shadow: Option<ProjectionShadowContext>,
    /// Render text.
    pub render_text: Vec<String>,
    /// Dynamic lights.
    pub dynamic_lights: Option<DynamicLightInput>,
    /// Scene-owned dynamic-light batches hook.
    pub dynamic_light_batches: DynamicLightBatches<'a>,
    /// Depth range.
    pub depth_range: [f32; 2],
    /// Polygon offset.
    pub polygon_offset: Option<super::state::PolygonOffset>,
    /// Q1 fog.
    pub q1_fog: Option<Q1FogInput>,
    /// Fog volume.
    pub fog: Option<FogVolumeInput<'a>>,
    /// Project callback.
    pub project: &'a dyn Fn(Vec3) -> Vec4,
}

fn texture_binding(bundle: &FinishedIteratorStage, time: f32) -> Result<TextureRef, ClientError> {
    let Some(binding) = bundle.binding.as_ref() else {
        return Err(ClientError::BadMaterial(
            "Collapsed stage lost its registered texture".to_string(),
        ));
    };
    match &binding.binding {
        FinishedStageBinding::RetainCurrentTexture => Ok(TextureRef::RetainCurrentTexture),
        FinishedStageBinding::Video { source } => Ok(TextureRef::DynamicImage(*source)),
        FinishedStageBinding::Images { playback } => match playback {
            FinishedImagePlayback::Single { image } => Ok(TextureRef::BindImage(*image)),
            FinishedImagePlayback::Animation { frequency, frames } => {
                let index = super::color::animated_picture_index(time, *frequency, frames.len())?;
                frames.get(index).copied().map(TextureRef::BindImage).ok_or_else(|| {
                    ClientError::BadMaterial("Animation frame is outside the registered image bundle".to_string())
                })
            }
        },
    }
}

fn bundle_coordinates(
    bundle: &FinishedIteratorStage,
    vertex: &MaterialVertex,
    time: f32,
    context: &MaterialDrawContext,
) -> Result<Vec2, ClientError> {
    let mut generator = bundle.stage.tc_gen;
    let mut input = vertex.tex_coord;
    match bundle.tc_gen {
        SourceTcGen::Bad => {
            return Err(ClientError::BadMaterial(
                "Uninitialized source texture coordinates need retained tess storage".to_string(),
            ));
        }
        SourceTcGen::Identity => {
            input = qa_core::math::vec2(0.0, 0.0);
            generator = TexGen::Texture;
        }
        SourceTcGen::Texture => generator = TexGen::Texture,
        SourceTcGen::Lightmap => generator = TexGen::Lightmap,
        SourceTcGen::EnvironmentMapped => generator = TexGen::Environment,
        SourceTcGen::Fog => {
            let Some(fog) = &context.fog else {
                return Err(ClientError::BadMaterial(
                    "Fog texture coordinates require the current volume".to_string(),
                ));
            };
            input = (fog.coordinates)(vertex.position);
            generator = TexGen::Texture;
        }
        SourceTcGen::Vector => {
            if !matches!(generator, TexGen::Vector { .. }) {
                return Err(ClientError::BadMaterial(
                    "Vector texture coordinates lost source projection vectors".to_string(),
                ));
            }
        }
    }
    let mut stage = bundle.stage.clone();
    stage.tc_gen = generator;
    evaluate_tex_coords(
        &stage,
        input,
        vertex.position,
        vertex.normal,
        time,
        &TexCoordContext {
            lightmap: Some(vertex.lightmap_coord),
            view_origin: Some(context.local_view_origin),
            shader_tex_coord: Some(context.shader_tex_coord),
        },
    )
}

fn retained_state(state_bits: u32, initial: &RenderState) -> Result<RenderState, ClientError> {
    let mut result = *initial;
    for change in source_state_changes(None, state_bits)? {
        match change {
            SourceStateChange::DepthFunction(value) => result.depth_test = value,
            SourceStateChange::BlendEnabled { source, destination } => {
                result.blend = super::state::Blend { source, destination };
            }
            SourceStateChange::BlendDisabled => {
                result.blend = super::state::OPAQUE_BLEND;
            }
            SourceStateChange::DepthWrite(value) => result.depth_write = value,
            SourceStateChange::DepthTest(enabled) => {
                if !enabled {
                    result.depth_test = DepthTest::Always;
                    result.depth_write = false;
                }
            }
            SourceStateChange::AlphaTest(value) => result.alpha_test = value,
            SourceStateChange::PolygonMode(PolygonMode::Line) => {
                return Err(ClientError::BadMaterial(
                    "Material polygon-line state requires explicit line geometry".to_string(),
                ));
            }
            SourceStateChange::PolygonMode(PolygonMode::Fill) => {}
        }
    }
    Ok(result)
}

/// Prepare material batches (`prepareMaterialBatches`). Takes the input
/// geometry by value so the common no-deform path moves it straight into
/// the deformed result instead of cloning it twice per surface per frame.
pub fn prepare_material_batches(
    compiled: &CompiledMaterial,
    input: MaterialGeometry,
    context: &MaterialDrawContext,
) -> Result<Vec<MaterialBatch>, ClientError> {
    if compiled.finished.iterator.kind == super::iterator::MaterialIteratorKind::Sky {
        return Err(ClientError::BadMaterial(
            "Sky materials require sky-box/cloud geometry preparation before ordinary stage evaluation".to_string(),
        ));
    }
    evaluate_material_passes(compiled, input, context)
}

/// Evaluate material passes (`evaluateMaterialPasses`).
pub fn evaluate_material_passes(
    compiled: &CompiledMaterial,
    input: MaterialGeometry,
    context: &MaterialDrawContext,
) -> Result<Vec<MaterialBatch>, ClientError> {
    let definition = &compiled.registered.definition;
    let mut time = context.time - context.time_offset;
    if definition.clamp_time != 0.0 && time >= definition.clamp_time {
        time = definition.clamp_time;
    }
    // Without deformations the deform pass is the identity, so move the
    // input through instead of cloning it into state and snapshotting it
    // back out. (A `[None]` deform list still takes the slow path; the
    // loop no-ops there, so both paths agree.)
    let geometry = if definition.deforms.is_empty() {
        DeformGeometry::from(input)
    } else {
        let mut state = MaterialDeformState::new(
            input,
            context.refdef_time,
            context.render_text.clone(),
            Some(definition.name.clone()),
        );
        deform_geometry(
            &mut state,
            &definition.deforms,
            &context.deform_view,
            time,
            context.noise,
            context.projection_shadow.as_ref(),
        )?
    };
    let mut batches = Vec::new();
    let mut previous_colors = vec![qa_core::math::vec4(0.0, 0.0, 0.0, 0.0); geometry.vertices.len()];
    let iterator = &compiled.finished.iterator;
    // Scene-owned lighting hooks force the generic iterator; without
    // hooks the finished iterator stands as compiled.
    let owned_iterator;
    let iterator = if context.dynamic_light_batches.is_some() {
        owned_iterator = source_material_iterator(
            &MaterialIteratorInput {
                stages: compiled.finished.source_stages.clone(),
                sky: definition.sky.is_some(),
                polygon_offset: definition.polygon_offset,
                deform_count: definition.deforms.len(),
            },
            &MaterialIteratorProfile {
                ignore_fast_path: true,
                multitexture: false,
                texture_env_add: false,
                driver: IteratorDriver::Generic,
            },
        );
        &owned_iterator
    } else {
        iterator
    };
    let mut first_vertices: Option<Vec<BatchVertex>> = None;
    for pass in &iterator.passes {
        let Some(first) = pass.bundles.first() else {
            continue;
        };
        if !first.active() {
            continue;
        }
        let mut base = stage_state(&pass.stage, definition.cull);
        base.depth_range = context.depth_range;
        base.polygon_offset = if definition.polygon_offset {
            context.polygon_offset
        } else {
            None
        };
        let render_state = retained_state(pass.state_bits, &base)?;
        let texture = texture_binding(first, time)?;
        let adjustment = pass.fog_adjustment;
        let fragment_fog = match &context.q1_fog {
            Some(q1) if q1.density > 0.0 => Some(BatchFog {
                density: q1.density,
                color: q1.color,
                effect: adjustment,
            }),
            _ => None,
        };
        let mut vertices = Vec::with_capacity(geometry.vertices.len());
        for (index, vertex) in geometry.vertices.iter().enumerate() {
            let previous = previous_colors[index];
            let color_context = StageColorContext {
                time,
                identity_light: context.identity_light,
                entity_rgba: context.entity_rgba,
                lighting: context.lighting,
                view_origin: context.view_origin,
                local_view_origin: context.local_view_origin,
                noise: context.noise,
                previous_color: previous,
            };
            let mut color = evaluate_stage_color(
                &pass.stage,
                vertex,
                &color_context,
                pass.alpha_gen == FinishedAlphaGen::Skip,
                Some(pass.rgb_gen),
            )?;
            if let Some(fog) = &context.fog {
                color = attenuate_fog_color(color, adjustment, (fog.coordinates)(vertex.position));
            }
            previous_colors[index] = color;
            vertices.push(BatchVertex {
                position: first_vertices
                    .as_ref()
                    .and_then(|first| first.get(index))
                    .map_or_else(|| (context.project)(vertex.position), |vertex| vertex.position),
                color,
                tex_coord: bundle_coordinates(first, vertex, time, context)?,
                tex_coord2: None,
            });
        }
        if first_vertices.is_none() {
            first_vertices = Some(vertices.clone());
        }
        match pass.bundles.get(1) {
            None => batches.push(MaterialBatch {
                lighting: if first.is_lightmap {
                    BatchLighting::Q2World {
                        pass: Q2LightPass::MaterialLightmap,
                    }
                } else if pass.rgb_gen == SourceColorGen::LightingDiffuse {
                    BatchLighting::Q2World {
                        pass: Q2LightPass::Model,
                    }
                } else {
                    BatchLighting::Vertex
                },
                fog: fragment_fog,
                texturing: Texturing::Single,
                state: render_state,
                texture,
                second_texture: None,
                indices: geometry.indices.clone(),
                vertices,
            }),
            Some(second) => {
                if !second.active() {
                    return Err(ClientError::BadMaterial(
                        "Collapsed stage lost its second registered texture".to_string(),
                    ));
                }
                let second_texture = texture_binding(second, time)?;
                let mut paired = Vec::with_capacity(vertices.len());
                for (vertex, source) in vertices.into_iter().zip(geometry.vertices.iter()) {
                    paired.push(BatchVertex {
                        tex_coord2: Some(bundle_coordinates(second, source, time, context)?),
                        ..vertex
                    });
                }
                batches.push(MaterialBatch {
                    lighting: BatchLighting::Vertex,
                    fog: fragment_fog,
                    texturing: Texturing::Pair,
                    state: render_state,
                    texture,
                    second_texture: Some((
                        second_texture,
                        if iterator.multitexture_env == MultitextureEnv::Add {
                            PairEnv::Add
                        } else {
                            PairEnv::Modulate
                        },
                    )),
                    indices: geometry.indices.clone(),
                    vertices: paired,
                });
            }
        }
    }
    if let Some(hook) = context.dynamic_light_batches {
        if receives_projected_dlights(compiled) {
            // Convert only when the hook runs; most surfaces take no
            // dynamic-light hook and skip this third geometry clone.
            batches.extend(hook(&MaterialGeometry::from(geometry.clone())));
        }
    } else if let Some(dynamic) = &context.dynamic_lights {
        if receives_projected_dlights(compiled) {
            for mut batch in project_dlight_texture(
                &geometry,
                dynamic.mask,
                &dynamic.lights,
                dynamic.image,
                context.project,
                definition.cull,
            )? {
                batch.state.depth_range = context.depth_range;
                batch.state.polygon_offset = if definition.polygon_offset {
                    context.polygon_offset
                } else {
                    None
                };
                batches.push(batch);
            }
        }
    }
    if let Some(fog) = &context.fog {
        if compiled.finished.fog_pass != super::fog::FogPass::None {
            let mut state = fog_pass_state(compiled.finished.fog_pass, definition.cull);
            state.depth_range = context.depth_range;
            state.polygon_offset = if definition.polygon_offset {
                context.polygon_offset
            } else {
                None
            };
            batches.push(MaterialBatch {
                lighting: BatchLighting::Vertex,
                fog: None,
                texturing: Texturing::Single,
                texture: fog.texture,
                state,
                second_texture: None,
                indices: geometry.indices.clone(),
                vertices: geometry
                    .vertices
                    .iter()
                    .map(|vertex| BatchVertex {
                        position: (context.project)(vertex.position),
                        tex_coord: (fog.coordinates)(vertex.position),
                        tex_coord2: None,
                        color: fog.color,
                    })
                    .collect(),
            });
        }
    }
    if let Some(q1) = &context.q1_fog {
        if q1.density > 0.0 {
            for batch in &mut batches {
                if batch.fog.is_none() {
                    batch.fog = Some(BatchFog {
                        density: q1.density,
                        color: q1.color,
                        effect: FogAdjustment::None,
                    });
                }
            }
            if compiled.finished.fog_pass != super::fog::FogPass::None {
                let mut state = fog_pass_state(compiled.finished.fog_pass, definition.cull);
                state.depth_range = context.depth_range;
                state.polygon_offset = if definition.polygon_offset {
                    context.polygon_offset
                } else {
                    None
                };
                batches.push(MaterialBatch {
                    lighting: BatchLighting::Vertex,
                    fog: Some(BatchFog {
                        density: q1.density,
                        color: q1.color,
                        effect: FogAdjustment::Rgba,
                    }),
                    texturing: Texturing::Single,
                    texture: q1.texture,
                    state,
                    second_texture: None,
                    indices: geometry.indices.clone(),
                    vertices: geometry
                        .vertices
                        .iter()
                        .map(|vertex| BatchVertex {
                            position: (context.project)(vertex.position),
                            tex_coord: qa_core::math::vec2(0.0, 0.0),
                            tex_coord2: None,
                            color: qa_core::math::vec4(1.0, 1.0, 1.0, 1.0),
                        })
                        .collect(),
                });
            }
        }
    }
    let _ = (AlphaTest::None, WaveKind::None);
    Ok(batches)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::materials::compile::{
        compile_shader_script, CompileOptions, RegisteredImage, ShaderRegistrationHost, SourceImageRequest,
    };
    use qa_core::math::{vec2, vec3, vec4};

    struct Host;

    impl ShaderRegistrationHost for Host {
        fn white_image(&self) -> RegisteredImage {
            RegisteredImage { image: 1, tmu: 0 }
        }

        fn default_image(&self) -> RegisteredImage {
            RegisteredImage { image: 2, tmu: 0 }
        }

        fn lightmap_image(&self) -> RegisteredImage {
            RegisteredImage { image: 3, tmu: 1 }
        }

        fn find_image(&mut self, _request: &SourceImageRequest) -> Option<RegisteredImage> {
            Some(RegisteredImage { image: 4, tmu: 0 })
        }

        fn play_shader_cinematic(&mut self, _name: &str) -> Option<crate::materials::compile::RegisteredShaderVideo> {
            None
        }

        fn apply_sun(&mut self, _sun: crate::materials::material::RegisteredSun) {}

        fn initialize_sky_tex_coords(&mut self, _height: f32) {}

        fn print_warning(&mut self, _message: &str) {}
    }

    fn vertex(position: Vec3) -> MaterialVertex {
        MaterialVertex::new(
            position,
            vec3(0.0, 0.0, 1.0),
            vec2(0.0, 0.0),
            vec2(0.0, 0.0),
            [255, 255, 255, 255],
        )
    }

    #[test]
    fn evaluates_single_pass() {
        let mut host = Host;
        let materials = compile_shader_script(
            "rock\n{\n {\n map textures/rock.tga\n }\n}\n",
            &mut host,
            "<test>",
            &CompileOptions::default(),
        )
        .unwrap();
        let noise = RendererNoise::new();
        let project = |position: Vec3| vec4(position.x, position.y, position.z, 1.0);
        let context = MaterialDrawContext {
            time: 0.0,
            time_offset: 0.0,
            refdef_time: 0.0,
            identity_light: 1.0,
            entity_rgba: [255, 255, 255, 255],
            lighting: None,
            view_origin: vec3(0.0, 0.0, 0.0),
            local_view_origin: vec3(0.0, 0.0, 0.0),
            noise: &noise,
            shader_tex_coord: vec2(0.0, 0.0),
            deform_view: DeformView {
                axis: [vec3(1.0, 0.0, 0.0), vec3(0.0, 1.0, 0.0), vec3(0.0, 0.0, 1.0)],
                mirror: false,
                entity_axis: None,
                non_normalized_axis: None,
            },
            projection_shadow: None,
            render_text: Vec::new(),
            dynamic_lights: None,
            dynamic_light_batches: None,
            depth_range: [0.0, 1.0],
            polygon_offset: None,
            q1_fog: None,
            fog: None,
            project: &project,
        };
        let geometry = MaterialGeometry {
            vertices: vec![
                vertex(vec3(0.0, 0.0, 0.0)),
                vertex(vec3(1.0, 0.0, 0.0)),
                vertex(vec3(0.0, 1.0, 0.0)),
            ],
            indices: vec![0, 1, 2],
        };
        let batches = prepare_material_batches(&materials[0], geometry, &context).unwrap();
        // One material pass; no fog volume means no fog batch.
        assert_eq!(batches.len(), 1);
        assert_eq!(batches[0].vertices.len(), 3);
    }
}
