//! Quake III captured-source scene rendering inside the admitted view.
//!
//! Port of `/home/buzzkill/Projects/quake-typescript/src/app/bootstrap/q3-client/scene.ts` (`ApplicationQ3SceneRenderer`). The port owns
//! admission iteration, render-flag filtering, weapon offsets, draw-sort orders, remap selection,
//! and generated-primitive geometry; provider model preparation and brush-model surfaces dispatch
//! through [`Q3SceneModelPreparation`] and [`Q3SceneWorld`] because the renderer port cannot emit
//! source-ordered model groups and the loaded world lives outside this wave's scope.

use std::cell::RefCell;
use std::rc::Rc;

use qa_client::materials::deform::{DeformView, RendererNoise};
use qa_client::materials::evaluate::MaterialBatch;
use qa_client::materials::evaluate::{prepare_material_batches, FogVolumeInput, MaterialDrawContext, TextureRef};
use qa_client::materials::fog::{FogCoordinates, FogVolume as ClientFogVolume};
use qa_client::materials::state::PolygonOffset as StatePolygonOffset;
use qa_client::render::scene::material_registrations::RegisteredSceneMaterial;
use qa_client::render::scene::models::types::{
    CustomSkinEntry, EntityFlags, EntityTransform, ModelResource, ModelSourceOptions as ClientModelSourceOptions,
    SceneEntity as ClientSceneEntity, SceneModel as ClientSceneModel, ScenePose,
};
use qa_client::render::scene::particles::primitives::{
    beam_batch, default_model_batch, poly_geometry, rail_geometry, sprite_geometry, BeamPose, PolyVertex, RailKind,
    RailPose, SpritePose, DEFAULT_RAIL_SETTINGS,
};
use qa_client::render::scene::submissions::{
    source_draw_group, SceneGroupOrder, SceneOperation, SourceEntityOrder, SourceSceneOrder, SourceSurfaceOrder,
};
use qa_client::render::scene::view::ViewProjector;
use qa_client::render::scene::world::WorldViewInput;
use qa_client::render::types::{AlphaTest, BlendFactor, CullFace, DepthTest, RenderState, RendererImage};
use qa_client::view::{CameraClip, ModelTransform};
use qa_content::hash::md4_block_checksum_key;
use qa_content::q3::presentation::ref_entity::{
    Q3AdmittedRefEntity, Q3DecodedModel, RefEntity, SceneModel as ContentSceneModel, SceneShader as ContentSceneShader,
    ShadedFields, RF_FIRST_PERSON, RF_THIRD_PERSON,
};
use qa_content::q3::presentation::resources::RendererResources;
use qa_content::q3::presentation::retail_snapshot::SceneShader as RetailSceneShader;
use qa_content::q3::presentation::scene::{
    q3_procedural_fog, PresentEntityModel, PresentSceneEntity, Q3FogSelection, Q3SceneContent,
};
use qa_content::q3scene::to_scene_md3;
use qa_core::identity::ActorId;
use qa_core::math::{vec2, vec3, Vec3, Vec4};
use thiserror::Error;

use super::assets::Q3AssetShared;
use super::view::{offset_q3_view_presented_entity, offset_q3_view_reference, q3_weapon_camera};

/// Q3 scene render options (donor `Q3SceneRenderOptions`).
#[derive(Debug, Clone, PartialEq)]
pub struct Q3SceneRenderOptions {
    /// View translation for first-person attachments.
    pub view_offset: Option<Vec3>,
    /// Skip world-model lighting and fog.
    pub no_world_model: bool,
    /// Split-screen seat.
    pub split_screen: bool,
    /// Reserve first-person attachments for the supplemental weapon view.
    pub supplemental_view_weapon: bool,
}

/// Q3 scene rendering failure.
#[derive(Debug, Error)]
pub enum Q3SceneError {
    /// The renderer is closed.
    #[error("Q3 scene renderer is closed")]
    Closed,
    /// The view has no source admission.
    #[error("Q3 view has no source admission")]
    MissingSource,
    /// A model lost its selected asset provider.
    #[error("Cgame model lost its selected asset provider")]
    MissingProvider,
    /// An admitted model lost its prepared descriptor.
    #[error("Admitted Q3 model lost its prepared descriptor")]
    MissingDescriptor,
    /// An entity record has a polygon type.
    #[error("R_AddEntitySurfaces: Bad reType")]
    BadRefEntityType,
    /// Rendering failed.
    #[error(transparent)]
    Render(#[from] qa_client::render::error::RenderError),
    /// Material evaluation failed.
    #[error(transparent)]
    Client(#[from] qa_client::ClientError),
    /// Presentation lookup failed.
    #[error(transparent)]
    Present(#[from] qa_content::q3::presentation::state::PresentClientError),
}

/// Source draw-sort position for a prepared model entity.
#[derive(Debug, Clone)]
pub struct Q3ModelSource {
    /// Owning view.
    pub view: SourceSceneOrder,
    /// Entity half.
    pub entity: SourceEntityOrder,
}

/// Provider model preparation behind the scene renderer.
pub trait Q3SceneModelPreparation {
    /// Preload model images for one provider.
    fn preload_models(
        &mut self,
        provider: usize,
        entities: &[ClientSceneEntity],
        options: &dyn Fn(&ClientSceneEntity) -> ClientModelSourceOptions,
    ) -> Result<(), qa_client::render::error::RenderError>;

    /// Prepare one provider's entities into source-ordered operations.
    fn prepare_models(
        &mut self,
        provider: usize,
        entities: &[ClientSceneEntity],
        input: &WorldViewInput,
        source: &Q3ModelSource,
        options: &dyn Fn(&ClientSceneEntity) -> ClientModelSourceOptions,
    ) -> Result<Vec<SceneOperation>, qa_client::render::error::RenderError>;
}

/// Loaded-world services behind the scene renderer.
pub trait Q3SceneWorld {
    /// Prepare one brush-model submodel for a view.
    fn prepare_brush_model(
        &mut self,
        model: usize,
        transform: &ModelTransform,
        input: &mut WorldViewInput,
        entity: SourceEntityOrder,
    ) -> Result<Vec<SceneOperation>, qa_client::render::error::RenderError>;

    /// Fog selections for admission and procedural fog.
    fn fog_selections(&mut self) -> Vec<Q3FogSelection>;

    /// Require a registered material by shader name.
    fn require_material(
        &mut self,
        name: &str,
    ) -> Result<RegisteredSceneMaterial, qa_client::render::error::RenderError>;

    /// Resolve a remap replacement by shader name.
    fn resolve_remap(&mut self, name: &str) -> Option<(RegisteredSceneMaterial, f32)>;

    /// Default source material.
    fn default_material(&mut self) -> RegisteredSceneMaterial;

    /// Solid white image for generated batches.
    fn white_image(&mut self) -> RendererImage;

    /// Fog image ordinal for fogged contexts.
    fn fog_image(&mut self) -> u32;

    /// Full fog volume for a procedural selection.
    fn fog_volume(&mut self, selection: &Q3FogSelection) -> ClientFogVolume;

    /// Resolve evaluated material batches into draw batches.
    fn draw_batches(
        &mut self,
        batches: Vec<MaterialBatch>,
    ) -> Result<Vec<qa_client::render::types::DrawBatch>, qa_client::render::error::RenderError>;
}

/// Generated-primitive draw state (donor `state`).
fn primitive_state() -> RenderState {
    RenderState {
        blend: (BlendFactor::One, BlendFactor::Zero),
        depth_test: DepthTest::LessEqual,
        depth_write: true,
        alpha_test: AlphaTest::None,
        cull: CullFace::Back,
        depth_range: [0.0, 1.0],
        polygon_offset: None,
    }
}

/// Byte channels from a byte-range color.
fn byte_rgba(color: Vec4) -> [u8; 4] {
    [
        color.x.clamp(0.0, 255.0) as u8,
        color.y.clamp(0.0, 255.0) as u8,
        color.z.clamp(0.0, 255.0) as u8,
        color.w.clamp(0.0, 255.0) as u8,
    ]
}

/// Stable path digest for scene-entity resources (source bytes stay loader-side).
fn path_digest(path: &str) -> u64 {
    let bytes = path.as_bytes();
    (u64::from(md4_block_checksum_key(bytes, 0)) << 32) | u64::from(md4_block_checksum_key(bytes, 1))
}

/// Render flags for an admitted record (polygons carry none).
fn admitted_flags(entity: &Q3AdmittedRefEntity) -> i32 {
    match entity {
        Q3AdmittedRefEntity::Entity(reference) => match reference {
            RefEntity::Model(entity) => entity.shading.render_flags,
            RefEntity::Sprite(entity) => entity.shading.render_flags,
            RefEntity::Beam(entity) => entity.shading.render_flags,
            RefEntity::RailCore(entity) => entity.shading.render_flags,
            RefEntity::RailRings(entity) => entity.shading.render_flags,
            RefEntity::Lightning(entity) => entity.shading.render_flags,
            RefEntity::Portal(entity) => entity.render_flags,
        },
        Q3AdmittedRefEntity::Poly(_) => 0,
    }
}

/// Whether an addition is a world polygon group (donor `polygon`).
fn is_world_polygon(operation: &SceneOperation) -> bool {
    match operation {
        SceneOperation::Group(group) => match &group.order {
            SceneGroupOrder::Source { source, .. } => matches!(source.entity, SourceEntityOrder::World),
            _ => false,
        },
        _ => false,
    }
}

/// Retail shader lookup by name (the host matches pictures by name).
fn retail_shader_lookup(shader: &ContentSceneShader) -> RetailSceneShader {
    RetailSceneShader {
        id: 0,
        name: shader.name.clone(),
        material_order: 0,
    }
}

/// Convert a presented entity into a render entity.
fn client_scene_entity(entity: &PresentSceneEntity) -> ClientSceneEntity {
    let model = match &entity.model {
        PresentEntityModel::BrushModel { .. } => ClientSceneModel::BrushModel,
        PresentEntityModel::Decoded(decoded) => match decoded {
            Q3DecodedModel::Md3(model) => ClientSceneModel::Q3Md3(to_scene_md3(model.clone())),
            // Bounds-only upstream mirrors carry no drawable surfaces; the model renderer
            // skips brush models, so they submit nothing through the normal path.
            Q3DecodedModel::Md5(_)
            | Q3DecodedModel::Bounded { .. }
            | Q3DecodedModel::Brush { .. }
            | Q3DecodedModel::Framed { .. } => ClientSceneModel::BrushModel,
        },
    };
    ClientSceneEntity {
        resource: ModelResource {
            id: entity.resource.path.clone(),
            requested_path: entity.resource.path.clone(),
            digest: path_digest(&entity.resource.path),
        },
        model,
        pose: ScenePose::Frame {
            frame: entity.frame,
            previous_frame: entity.previous_frame,
            back_lerp: entity.back_lerp,
        },
        transform: EntityTransform {
            origin: entity.origin,
            axis: entity.axis,
            scale: vec3(1.0, 1.0, 1.0),
        },
        previous_origin: entity.previous_origin,
        lighting_origin: entity.lighting_origin,
        color: entity.color,
        skin: entity.skin,
        shader_time_seconds: f64::from(entity.shader_time),
        flags: EntityFlags::Q3 {
            bits: entity.render_flags as u32,
        },
        attachments: Vec::new(),
        actor_slot: entity.actor.as_ref().map(ActorId::slot),
    }
}

/// Map presented source options into renderer options.
fn client_options(
    options: &qa_content::q3::presentation::scene::ModelSourceOptions,
    no_world_model: bool,
    shader_tex_coord: Option<qa_core::math::Vec2>,
) -> ClientModelSourceOptions {
    ClientModelSourceOptions {
        custom_shader: options.custom_shader.clone(),
        custom_skin: options.custom_skin.as_ref().map(|skin| {
            skin.iter()
                .map(|entry| CustomSkinEntry {
                    name: entry.name.clone(),
                    shader: entry.shader.clone(),
                })
                .collect()
        }),
        non_normalized_axes: options.non_normalized_axes,
        no_world_model,
        shader_tex_coord,
        ..ClientModelSourceOptions::default()
    }
}

/// Renders captured source geometry inside the destination's admitted view.
pub struct ApplicationQ3SceneRenderer {
    /// Provider model preparation.
    models: Box<dyn Q3SceneModelPreparation>,
    /// Loaded-world services.
    world: Box<dyn Q3SceneWorld>,
    /// Resource pictures.
    resources: Box<dyn RendererResources>,
    /// Asset load tables for provider routing.
    providers: Rc<RefCell<Q3AssetShared>>,
    /// Renderer noise for material contexts.
    noise: RendererNoise,
    /// Whether the renderer closed.
    closed: bool,
}

impl ApplicationQ3SceneRenderer {
    /// Build a scene renderer over injected services.
    pub fn new(
        models: Box<dyn Q3SceneModelPreparation>,
        world: Box<dyn Q3SceneWorld>,
        resources: Box<dyn RendererResources>,
        providers: Rc<RefCell<Q3AssetShared>>,
    ) -> Self {
        Self {
            models,
            world,
            resources,
            providers,
            noise: RendererNoise::new(),
            closed: false,
        }
    }

    /// Whether the renderer closed.
    #[must_use]
    pub fn is_closed(&self) -> bool {
        self.closed
    }

    /// Reject calls after close.
    fn assert_current(&self) -> Result<(), Q3SceneError> {
        if self.closed {
            return Err(Q3SceneError::Closed);
        }
        Ok(())
    }

    /// Group presented models by provider slot, skipping brush models.
    fn group_models<'a>(
        &self,
        scene: &'a Q3SceneContent,
    ) -> Result<Vec<(usize, Vec<&'a qa_content::q3::presentation::scene::PresentedModel>)>, Q3SceneError> {
        let mut groups: Vec<(usize, Vec<&'a qa_content::q3::presentation::scene::PresentedModel>)> = Vec::new();
        for model in &scene.models {
            if matches!(model.entity.model, PresentEntityModel::BrushModel { .. }) {
                continue;
            }
            let slot = self
                .providers
                .borrow()
                .model_slots
                .iter()
                .find(|(value, _)| value == &model.source.model)
                .map(|(_, slot)| *slot)
                .ok_or(Q3SceneError::MissingProvider)?;
            match groups.iter_mut().find(|(value, _)| *value == slot) {
                Some((_, models)) => models.push(model),
                None => groups.push((slot, vec![model])),
            }
        }
        Ok(groups)
    }

    /// Provider slot for an admitted content model.
    fn provider_slot(&self, model: &ContentSceneModel) -> Result<usize, Q3SceneError> {
        self.providers
            .borrow()
            .model_slots
            .iter()
            .find(|(value, _)| value == model)
            .map(|(_, slot)| *slot)
            .ok_or(Q3SceneError::MissingProvider)
    }

    /// Preload model images for captured scenes.
    pub fn preload(&mut self, contents: &[Q3SceneContent]) -> Result<(), Q3SceneError> {
        self.assert_current()?;
        for content in contents {
            for (slot, models) in self.group_models(content)? {
                let pairs: Vec<(ClientSceneEntity, _)> = models
                    .iter()
                    .map(|model| (client_scene_entity(&model.entity), model))
                    .collect();
                let entities: Vec<ClientSceneEntity> = pairs.iter().map(|(entity, _)| entity.clone()).collect();
                self.models.preload_models(slot, &entities, &|entity| {
                    pairs
                        .iter()
                        .find(|(value, _)| value == entity)
                        .map(|(_, model)| client_options(&model.options, false, None))
                        .unwrap_or_default()
                })?;
                self.assert_current()?;
            }
        }
        Ok(())
    }
}

/// Effect-entity pose shared by the fog, order, and geometry paths.
struct EffectPose<'a> {
    /// Origin.
    origin: Vec3,
    /// Radius.
    radius: f32,
    /// Shading.
    shading: &'a ShadedFields,
}

impl ApplicationQ3SceneRenderer {
    /// Resolve a shader name into its effective registered material and remap offset.
    fn effective_material(&mut self, name: &str) -> Result<(RegisteredSceneMaterial, f32), Q3SceneError> {
        let required = self.world.require_material(name)?;
        if let Some((replacement, offset)) = self.world.resolve_remap(name) {
            return Ok((replacement, offset));
        }
        Ok((required, 0.0))
    }

    /// Submit one admitted polygon.
    fn submit_poly(
        &mut self,
        operations: &mut Vec<SceneOperation>,
        input: &WorldViewInput,
        view: &SourceSceneOrder,
        index: usize,
        poly: &qa_content::q3::presentation::scene::Q3AdmittedPoly,
        fog_image: u32,
    ) -> Result<(), Q3SceneError> {
        let shader = poly.poly.shader.as_ref().map(retail_shader_lookup);
        let picture = self.resources.picture(shader.as_ref())?;
        let (material, remap_offset) = self.effective_material(&picture.name)?;
        let fog = poly.fog;
        let volume = fog.map(|selection| self.world.fog_volume(&selection));
        let fog_coords = volume
            .as_ref()
            .map(|volume| FogCoordinates::new(volume, &input.camera.origin, &input.camera.axis[0]));
        let projector = ViewProjector::new(input.camera, None);
        let project = |point: Vec3| projector.project(point).expect("camera-only projection");
        let order = SourceSurfaceOrder {
            view: view.clone(),
            entity: SourceEntityOrder::World,
            surface: index as u32,
            fog: fog.map(|selection| (selection.index + 1) as u32).unwrap_or(0),
            dlight: 0,
        };
        let vertices: Vec<PolyVertex> = poly
            .poly
            .vertices
            .iter()
            .map(|vertex| PolyVertex {
                position: vertex.position,
                tex_coord: vertex.tex_coord,
                color: byte_rgba(vertex.color),
            })
            .collect();
        let geometry = poly_geometry(&vertices);
        let entity_rgba = input.entity_rgba;
        let fog_color = volume.as_ref().map(|volume| volume.color);
        let batches = if let Some(coords) = fog_coords.as_ref() {
            let coordinates = |point: Vec3| coords.coordinates(&point);
            let context = self.poly_context_inner(
                input,
                &project,
                Some(&coordinates),
                fog_image,
                entity_rgba,
                vec2(0.0, 0.0),
                remap_offset,
                fog_color,
            );
            prepare_material_batches(&material.compiled, geometry, &context)?
        } else {
            let context = self.poly_context_inner(
                input,
                &project,
                None,
                fog_image,
                entity_rgba,
                vec2(0.0, 0.0),
                remap_offset,
                None,
            );
            prepare_material_batches(&material.compiled, geometry, &context)?
        };
        let batches = self.world.draw_batches(batches)?;
        operations.push(SceneOperation::Group(source_draw_group(material, order, batches)?));
        Ok(())
    }

    /// Shared context assembly over caller-owned fog coordinates.
    #[allow(clippy::too_many_arguments)]
    fn poly_context_inner<'a>(
        &'a self,
        input: &WorldViewInput,
        project: &'a dyn Fn(Vec3) -> Vec4,
        coordinates: Option<&'a dyn Fn(Vec3) -> qa_core::math::Vec2>,
        fog_image: u32,
        entity_rgba: [u8; 4],
        shader_tex_coord: qa_core::math::Vec2,
        time_offset: f32,
        fog_color: Option<Vec4>,
    ) -> MaterialDrawContext<'a> {
        let time = input.time.as_seconds() as f32;
        MaterialDrawContext {
            time,
            time_offset,
            refdef_time: (time * 1000.0).trunc(),
            identity_light: input.identity_light,
            entity_rgba,
            lighting: input.lighting,
            view_origin: input.camera.origin,
            local_view_origin: input.camera.origin,
            noise: &self.noise,
            shader_tex_coord,
            deform_view: DeformView {
                axis: input.camera.axis,
                mirror: matches!(input.camera.clip, CameraClip::Portal { mirror: true, .. }),
                entity_axis: None,
                non_normalized_axis: None,
            },
            projection_shadow: input.projection_shadow,
            render_text: input.render_text.clone(),
            dynamic_lights: None,
            dynamic_light_batches: None,
            depth_range: [0.0, 1.0],
            polygon_offset: Some(StatePolygonOffset {
                factor: -1.0,
                units: -2.0,
            }),
            q1_fog: input.q1_fog.map(|fog| qa_client::materials::evaluate::Q1FogInput {
                density: fog.density,
                color: fog.color,
                texture: TextureRef::BindImage(fog_image),
            }),
            fog: match (fog_color, coordinates) {
                (Some(color), Some(coordinates)) => Some(FogVolumeInput {
                    coordinates,
                    texture: TextureRef::BindImage(fog_image),
                    color,
                }),
                _ => None,
            },
            project,
        }
    }

    /// Submit one effect entity (sprite, beam, rail, or lightning).
    #[allow(clippy::too_many_arguments)]
    fn submit_effect(
        &mut self,
        operations: &mut Vec<SceneOperation>,
        input: &WorldViewInput,
        view: &SourceSceneOrder,
        entity_order: SourceEntityOrder,
        entity: &RefEntity,
        options: &Q3SceneRenderOptions,
        fog_image: u32,
        white: &RendererImage,
        project: &dyn Fn(Vec3) -> Vec4,
        state: &RenderState,
    ) -> Result<(), Q3SceneError> {
        let pose = match entity {
            RefEntity::Sprite(entity) => EffectPose {
                origin: entity.origin,
                radius: entity.radius,
                shading: &entity.shading,
            },
            RefEntity::Beam(entity) => EffectPose {
                origin: entity.origin,
                radius: entity.radius,
                shading: &entity.shading,
            },
            RefEntity::RailCore(entity) => EffectPose {
                origin: entity.origin,
                radius: entity.radius,
                shading: &entity.shading,
            },
            RefEntity::RailRings(entity) => EffectPose {
                origin: entity.origin,
                radius: entity.radius,
                shading: &entity.shading,
            },
            RefEntity::Lightning(entity) => EffectPose {
                origin: entity.origin,
                radius: entity.radius,
                shading: &entity.shading,
            },
            RefEntity::Model(_) | RefEntity::Portal(_) => return Ok(()),
        };
        let mut material = self.world.default_material();
        let mut material_name: Option<String> = None;
        if let Some(shader) = &pose.shading.custom_shader {
            let picture = self.resources.picture(Some(&retail_shader_lookup(shader)))?;
            material_name = Some(picture.name.clone());
            material = self.world.require_material(&picture.name)?;
        }
        let mut time_offset = 0.0;
        if let Some(name) = &material_name {
            if let Some((replacement, offset)) = self.world.resolve_remap(name) {
                material = replacement;
                time_offset = offset;
            }
        }
        let fog = if options.no_world_model {
            None
        } else {
            q3_procedural_fog(pose.origin, pose.radius, &self.world.fog_selections())
        };
        let order = SourceSurfaceOrder {
            view: view.clone(),
            entity: entity_order,
            surface: 0,
            fog: fog.map(|selection| (selection.index + 1) as u32).unwrap_or(0),
            dlight: 0,
        };
        if let RefEntity::Beam(beam) = entity {
            let batches = vec![beam_batch(
                &BeamPose {
                    origin: beam.origin,
                    old_origin: beam.old_origin,
                },
                project,
                state,
                white,
            )];
            operations.push(SceneOperation::Group(source_draw_group(material, order, batches)?));
            return Ok(());
        }
        let geometry = match entity {
            RefEntity::Sprite(sprite) => sprite_geometry(
                &SpritePose {
                    origin: sprite.origin,
                    radius: sprite.radius,
                    rotation: sprite.rotation,
                    shader_rgba: sprite.shading.shader_rgba,
                },
                &input.camera.axis,
                matches!(input.camera.clip, CameraClip::Portal { mirror: true, .. }),
            ),
            RefEntity::RailCore(rail) => rail_geometry(
                &RailPose {
                    kind: RailKind::RailCore,
                    origin: rail.origin,
                    old_origin: rail.old_origin,
                    shader_rgba: rail.shading.shader_rgba,
                },
                input.camera.origin,
                &DEFAULT_RAIL_SETTINGS,
            )?,
            RefEntity::RailRings(rail) => rail_geometry(
                &RailPose {
                    kind: RailKind::RailRings,
                    origin: rail.origin,
                    old_origin: rail.old_origin,
                    shader_rgba: rail.shading.shader_rgba,
                },
                input.camera.origin,
                &DEFAULT_RAIL_SETTINGS,
            )?,
            RefEntity::Lightning(rail) => rail_geometry(
                &RailPose {
                    kind: RailKind::Lightning,
                    origin: rail.origin,
                    old_origin: rail.old_origin,
                    shader_rgba: rail.shading.shader_rgba,
                },
                input.camera.origin,
                &DEFAULT_RAIL_SETTINGS,
            )?,
            RefEntity::Model(_) | RefEntity::Portal(_) | RefEntity::Beam(_) => return Ok(()),
        };
        let volume = fog.map(|selection| self.world.fog_volume(&selection));
        let fog_coords = volume
            .as_ref()
            .map(|volume| FogCoordinates::new(volume, &input.camera.origin, &input.camera.axis[0]));
        let fog_color = volume.as_ref().map(|volume| volume.color);
        let entity_rgba = byte_rgba(pose.shading.shader_rgba);
        let shader_tex_coord = vec2(pose.shading.shader_tex_coord.x, pose.shading.shader_tex_coord.y);
        let offset = pose.shading.shader_time + time_offset;
        let projector = ViewProjector::new(input.camera, None);
        let project_surface = |point: Vec3| projector.project(point).expect("camera-only projection");
        let batches = if let Some(coords) = fog_coords.as_ref() {
            let coordinates = |point: Vec3| coords.coordinates(&point);
            let context = self.poly_context_inner(
                input,
                &project_surface,
                Some(&coordinates),
                fog_image,
                entity_rgba,
                shader_tex_coord,
                offset,
                fog_color,
            );
            prepare_material_batches(&material.compiled, geometry, &context)?
        } else {
            let context = self.poly_context_inner(
                input,
                &project_surface,
                None,
                fog_image,
                entity_rgba,
                shader_tex_coord,
                offset,
                None,
            );
            prepare_material_batches(&material.compiled, geometry, &context)?
        };
        let batches = self.world.draw_batches(batches)?;
        operations.push(SceneOperation::Group(source_draw_group(material, order, batches)?));
        Ok(())
    }
}

impl ApplicationQ3SceneRenderer {
    /// Submit captured source geometry as ordered operations.
    pub fn operations(
        &mut self,
        scene: &Q3SceneContent,
        input: &WorldViewInput,
        first_entity: u32,
        options: &Q3SceneRenderOptions,
        additions: &[SceneOperation],
    ) -> Result<Vec<SceneOperation>, Q3SceneError> {
        self.assert_current()?;
        let view = input
            .source
            .as_ref()
            .map(|source| source.view.clone())
            .ok_or(Q3SceneError::MissingSource)?;
        let mut operations = Vec::new();
        let fog_image = self.world.fog_image();
        for (index, poly) in scene.admission.polygons.iter().enumerate() {
            self.submit_poly(&mut operations, input, &view, index, poly, fog_image)?;
        }
        operations.extend(
            additions
                .iter()
                .filter(|operation| is_world_polygon(operation))
                .cloned(),
        );
        let models: std::collections::HashMap<usize, &qa_content::q3::presentation::scene::PresentedModel> =
            scene.models.iter().map(|model| (model.entity_index, model)).collect();
        let mut weapon_input = input.clone();
        weapon_input.camera = q3_weapon_camera(&input.camera, options.split_screen && !options.no_world_model);
        let projector = ViewProjector::new(input.camera, None);
        let project = |point: Vec3| projector.project(point).expect("camera-only projection");
        let white = self.world.white_image();
        let state = primitive_state();
        for (index, original) in scene.admission.entities.iter().enumerate() {
            let flags = admitted_flags(original);
            let entity = if !options.no_world_model && flags & RF_FIRST_PERSON != 0 {
                offset_q3_view_reference(original, options.view_offset)
            } else {
                original.clone()
            };
            let flags = admitted_flags(&entity);
            let entity_order = SourceEntityOrder::RefEntity {
                index: first_entity + index as u32,
            };
            if options.supplemental_view_weapon && !options.no_world_model && flags & RF_FIRST_PERSON != 0 {
                continue;
            }
            if matches!(input.camera.clip, CameraClip::Portal { .. }) && flags & RF_FIRST_PERSON != 0 {
                continue;
            }
            match &entity {
                Q3AdmittedRefEntity::Poly(_) => return Err(Q3SceneError::BadRefEntityType),
                Q3AdmittedRefEntity::Entity(RefEntity::Portal(_)) => continue,
                Q3AdmittedRefEntity::Entity(RefEntity::Model(model)) => {
                    if model.model.is_default() {
                        if matches!(input.camera.clip, CameraClip::None) && flags & RF_THIRD_PERSON != 0 {
                            continue;
                        }
                        let batches = vec![default_model_batch(
                            &EntityTransform {
                                origin: model.origin,
                                axis: model.axis,
                                scale: vec3(1.0, 1.0, 1.0),
                            },
                            &project,
                            &state,
                            &white,
                        )];
                        operations.push(SceneOperation::Group(source_draw_group(
                            self.world.default_material(),
                            SourceSurfaceOrder {
                                view: view.clone(),
                                entity: entity_order,
                                surface: 0,
                                fog: 0,
                                dlight: 0,
                            },
                            batches,
                        )?));
                        continue;
                    }
                    let presented = models.get(&index).ok_or(Q3SceneError::MissingDescriptor)?;
                    let selected = if flags & RF_FIRST_PERSON != 0 {
                        &weapon_input
                    } else {
                        input
                    };
                    let rendered = if flags & RF_FIRST_PERSON != 0 && !options.no_world_model {
                        offset_q3_view_presented_entity(&presented.entity, options.view_offset)
                    } else {
                        presented.entity.clone()
                    };
                    match &presented.entity.model {
                        PresentEntityModel::BrushModel { model: brush, .. } => {
                            let mut brush_input = selected.clone();
                            brush_input.animation_frame = Some(model.frame as f32);
                            brush_input.entity_rgba = byte_rgba(model.shading.shader_rgba);
                            let transform = ModelTransform {
                                origin: rendered.origin,
                                axis: model.axis,
                                scale: 1.0,
                            };
                            operations.extend(self.world.prepare_brush_model(
                                *brush,
                                &transform,
                                &mut brush_input,
                                entity_order,
                            )?);
                        }
                        PresentEntityModel::Decoded(_) => {
                            let slot = self.provider_slot(&model.model)?;
                            let converted = client_scene_entity(&rendered);
                            let source = Q3ModelSource {
                                view: view.clone(),
                                entity: entity_order,
                            };
                            let presented_options = &presented.options;
                            let no_world_model = options.no_world_model;
                            let shader_tex_coord =
                                vec2(model.shading.shader_tex_coord.x, model.shading.shader_tex_coord.y);
                            operations.extend(self.models.prepare_models(
                                slot,
                                &[converted],
                                selected,
                                &source,
                                &|_| client_options(presented_options, no_world_model, Some(shader_tex_coord)),
                            )?);
                        }
                    }
                }
                Q3AdmittedRefEntity::Entity(effect) => {
                    if matches!(input.camera.clip, CameraClip::None) && flags & RF_THIRD_PERSON != 0 {
                        continue;
                    }
                    self.submit_effect(
                        &mut operations,
                        input,
                        &view,
                        entity_order,
                        effect,
                        options,
                        fog_image,
                        &white,
                        &project,
                        &state,
                    )?;
                }
            }
        }
        operations.extend(
            additions
                .iter()
                .filter(|operation| !is_world_polygon(operation))
                .cloned(),
        );
        Ok(operations)
    }

    /// Close the renderer.
    pub fn close(&mut self) {
        self.closed = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_client::render::scene::resources::SceneImageRegistry;
    use qa_client::render::scene::shaders::SceneShaderRegistry;
    use qa_client::render::scene::submissions::create_source_scene_order;
    use qa_client::render::scene::textures::{SceneAsset, SceneAssetReader, SceneTextureLoader};
    use qa_client::render::types::{fresh_owner_identity, ImageSource, ResourceOwner, SourceTime, ViewTarget};
    use qa_client::view::{Rect, SceneCamera};
    use qa_content::contract::ContentId;
    use qa_content::q3::presentation::ref_entity::{
        create_beam_entity, create_model_entity, create_sprite_entity, PresentResource, PresentWorld, RefPoly,
        SceneInlineModel, SceneLoadedModel, SceneShader,
    };
    use qa_content::q3::presentation::resources::{
        Q3RendererResources, Q3ResourceHost, ResourceWorld, ResourceWorldMap, WorldScene,
    };
    use qa_content::q3::presentation::retail_snapshot::{
        SceneModel as RetailSceneModel, SceneShader as RetailSceneShader, SceneSkin as RetailSceneSkin,
    };
    use qa_content::q3::presentation::scene::{
        ModelSourceOptions as FoundationModelSourceOptions, PresentedModel, Q3AdmittedPoly, Q3SceneAdmission,
        Q3SceneAdmissionId, SceneAdmissionOrigin,
    };
    use qa_content::q3::presentation::state::PresentResult;
    use qa_core::identity::IdentityOwner;
    use qa_core::math::{vec4, Axis, Bounds};

    use super::super::assets::{
        OpenedQ3Asset, Q3AssetCatalog, Q3AssetError, Q3AssetModels, Q3AssetMounts, Q3AssetWorld, Q3LoadedModel,
        Q3RemapOutcome,
    };

    #[derive(Default)]
    struct StubMounts;

    impl Q3AssetMounts for StubMounts {
        fn open(&mut self, _content: &ContentId, _path: &str) -> Result<Option<OpenedQ3Asset>, Q3AssetError> {
            Ok(None)
        }

        fn resolve(&mut self, _content: &ContentId, _path: &str) -> bool {
            false
        }

        fn catalog_archives(&self) -> Vec<(String, Vec<String>)> {
            Vec::new()
        }

        fn plan_archives(&self, _content: &ContentId) -> Vec<String> {
            Vec::new()
        }

        fn loose_roots(&self, _content: &ContentId) -> Vec<std::path::PathBuf> {
            Vec::new()
        }
    }

    #[derive(Default)]
    struct StubModels;

    impl Q3AssetModels for StubModels {
        fn load_model(&mut self, _content: &ContentId, path: &str) -> Result<Q3LoadedModel, Q3AssetError> {
            Err(Q3AssetError::Load(path.to_string()))
        }

        fn open_skin(&mut self, _content: &ContentId, _path: &str) -> Result<Option<Vec<u8>>, Q3AssetError> {
            Ok(None)
        }

        fn inline_bounds(&mut self, _world: &PresentWorld, _index: usize) -> Option<Bounds> {
            None
        }
    }

    struct StubCatalog;

    impl Q3AssetCatalog for StubCatalog {
        fn character_content(&self) -> ContentId {
            ContentId("q3:test:baseq3:1".to_string())
        }

        fn weapon_contents(&self) -> Vec<ContentId> {
            Vec::new()
        }

        fn family_of(&self, _content: &ContentId) -> qa_content::contract::GameFamily {
            qa_content::contract::GameFamily::Q3
        }
    }

    struct StubAssetWorld;

    impl Q3AssetWorld for StubAssetWorld {
        fn world_model_bounds(&self) -> Vec<Bounds> {
            Vec::new()
        }

        fn remap_shader(
            &mut self,
            _original: &str,
            _replacement: &str,
            _time_offset: f32,
            _current: &dyn Fn() -> bool,
        ) -> Q3RemapOutcome {
            Q3RemapOutcome::Applied
        }

        fn fog_selections(&self) -> Vec<Q3FogSelection> {
            Vec::new()
        }
    }

    fn providers() -> Rc<RefCell<Q3AssetShared>> {
        Rc::new(RefCell::new(Q3AssetShared {
            mounts: Box::new(StubMounts),
            models: Box::new(StubModels),
            catalog: Box::new(StubCatalog),
            world: Box::new(StubAssetWorld),
            model_contents: Vec::new(),
            model_slots: Vec::new(),
            provider_contents: Vec::new(),
            loaded_models: Vec::new(),
            loaded_skins: Vec::new(),
            closed: false,
        }))
    }

    #[derive(Default)]
    struct StubPreparation {
        preloads: Vec<(usize, usize)>,
        prepares: Vec<(usize, PresentEntityModelKind, SourceEntityOrder, bool)>,
        marker: Option<SceneOperation>,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum PresentEntityModelKind {
        Decoded,
        Other,
    }

    impl Q3SceneModelPreparation for StubPreparation {
        fn preload_models(
            &mut self,
            provider: usize,
            entities: &[ClientSceneEntity],
            options: &dyn Fn(&ClientSceneEntity) -> ClientModelSourceOptions,
        ) -> Result<(), qa_client::render::error::RenderError> {
            for entity in entities {
                let _ = options(entity);
            }
            self.preloads.push((provider, entities.len()));
            Ok(())
        }

        fn prepare_models(
            &mut self,
            provider: usize,
            entities: &[ClientSceneEntity],
            input: &WorldViewInput,
            source: &Q3ModelSource,
            options: &dyn Fn(&ClientSceneEntity) -> ClientModelSourceOptions,
        ) -> Result<Vec<SceneOperation>, qa_client::render::error::RenderError> {
            let entity = entities.first().expect("one entity");
            let resolved = options(entity);
            let kind = match &entity.model {
                ClientSceneModel::BrushModel => PresentEntityModelKind::Other,
                _ => PresentEntityModelKind::Decoded,
            };
            self.prepares.push((
                provider,
                kind,
                source.entity,
                resolved.no_world_model && input.no_world_model,
            ));
            Ok(self.marker.clone().into_iter().collect())
        }
    }

    struct FakeReader;

    impl SceneAssetReader for FakeReader {
        fn read(&self, _path: &str) -> Result<Option<SceneAsset>, qa_client::render::error::RenderError> {
            Ok(None)
        }
    }

    fn registry() -> SceneShaderRegistry {
        let session = IdentityOwner::create("q3-scene-test").expect("owner").session().clone();
        let owner = ResourceOwner::new(fresh_owner_identity(), session, 0);
        let images = SceneImageRegistry::new(owner);
        let loader = SceneTextureLoader::new(images, Box::new(FakeReader), None, None, 224).expect("loader");
        let mut registry = SceneShaderRegistry::with_defaults(loader);
        registry.initialize_source_materials(&[]).expect("sources");
        registry
    }

    struct StubWorld {
        material: RegisteredSceneMaterial,
        remap: Option<(RegisteredSceneMaterial, f32)>,
        white: RendererImage,
        brushes: Vec<(usize, f32)>,
    }

    impl StubWorld {
        fn new() -> Self {
            let registry = registry();
            let material = registry.source_materials().expect("materials").default.clone();
            let owner = IdentityOwner::create("q3-scene-white").expect("owner");
            Self {
                material,
                remap: None,
                white: RendererImage {
                    owner: ResourceOwner::new(fresh_owner_identity(), owner.session().clone(), 0),
                    ordinal: 7,
                    source: ImageSource::Generated {
                        name: "white".to_string(),
                    },
                    width: 1,
                    height: 1,
                },
                brushes: Vec::new(),
            }
        }
    }

    impl Q3SceneWorld for StubWorld {
        fn prepare_brush_model(
            &mut self,
            model: usize,
            transform: &ModelTransform,
            input: &mut WorldViewInput,
            _entity: SourceEntityOrder,
        ) -> Result<Vec<SceneOperation>, qa_client::render::error::RenderError> {
            self.brushes
                .push((model, transform.origin.x + input.animation_frame.unwrap_or(0.0)));
            Ok(Vec::new())
        }

        fn fog_selections(&mut self) -> Vec<Q3FogSelection> {
            Vec::new()
        }

        fn require_material(
            &mut self,
            _name: &str,
        ) -> Result<RegisteredSceneMaterial, qa_client::render::error::RenderError> {
            Ok(self.material.clone())
        }

        fn resolve_remap(&mut self, _name: &str) -> Option<(RegisteredSceneMaterial, f32)> {
            self.remap.clone()
        }

        fn default_material(&mut self) -> RegisteredSceneMaterial {
            self.material.clone()
        }

        fn white_image(&mut self) -> RendererImage {
            self.white.clone()
        }

        fn fog_image(&mut self) -> u32 {
            3
        }

        fn fog_volume(&mut self, _selection: &Q3FogSelection) -> ClientFogVolume {
            ClientFogVolume {
                bounds: qa_core::math::Bounds {
                    min: vec3(-8.0, -8.0, -8.0),
                    max: vec3(8.0, 8.0, 8.0),
                },
                surface: None,
                color: vec4(0.5, 0.5, 0.5, 1.0),
                tc_scale: 1.0,
            }
        }

        fn draw_batches(
            &mut self,
            _batches: Vec<MaterialBatch>,
        ) -> Result<Vec<qa_client::render::types::DrawBatch>, qa_client::render::error::RenderError> {
            Ok(Vec::new())
        }
    }

    #[derive(Default)]
    struct StubHost {
        shaders: u32,
    }

    impl Q3ResourceHost for StubHost {
        fn zero_picture(&self) -> RetailSceneShader {
            RetailSceneShader {
                id: 0,
                name: String::new(),
                material_order: 0,
            }
        }

        fn load_model(&mut self, _path: &str) -> PresentResult<RetailSceneModel> {
            Ok(RetailSceneModel::Default)
        }

        fn load_skin(&mut self, _path: &str) -> PresentResult<Option<RetailSceneSkin>> {
            Ok(None)
        }

        fn load_shader(&mut self, path: &str, _mip: bool) -> PresentResult<Option<RetailSceneShader>> {
            self.shaders += 1;
            Ok(Some(RetailSceneShader {
                id: self.shaders,
                name: path.to_string(),
                material_order: self.shaders as i32,
            }))
        }

        fn load_world_scene(&mut self, _requested_path: &str) -> PresentResult<WorldScene> {
            Ok(WorldScene {
                model_bounds: Vec::new(),
            })
        }

        fn remap_shader(&mut self, _original: &str, _replacement: &str, _offset: &str) -> PresentResult<()> {
            Ok(())
        }

        fn clear_scene(&mut self) {}

        fn add_ref_entity(&mut self, _entity: qa_content::q3::presentation::retail_snapshot::RefEntity) {}

        fn add_poly(&mut self, _poly: qa_content::q3::presentation::retail_snapshot::RefPoly) {}

        fn add_light(&mut self, _light: qa_content::q3::presentation::retail_snapshot::DynamicLight) {}

        fn render_scene(&mut self, _refdef: &qa_content::q3::presentation::retail_snapshot::Refdef) {}
    }

    struct StubResourceWorld;

    impl ResourceWorld for StubResourceWorld {
        fn resource_map(&self) -> &ResourceWorldMap {
            static MAP: std::sync::OnceLock<ResourceWorldMap> = std::sync::OnceLock::new();
            MAP.get_or_init(|| ResourceWorldMap {
                entities: String::new(),
                nodes: Vec::new(),
                leaves: Vec::new(),
                planes: Vec::new(),
            })
        }

        fn cluster_pvs_byte(&self, _cluster: i32, _offset: usize) -> u8 {
            0
        }
    }

    fn axis() -> Axis {
        [vec3(1.0, 0.0, 0.0), vec3(0.0, 1.0, 0.0), vec3(0.0, 0.0, 1.0)]
    }

    fn camera() -> SceneCamera {
        SceneCamera {
            origin: vec3(0.0, 0.0, 64.0),
            axis: axis(),
            projection: [
                1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
            ],
            viewport: Rect {
                x: 0,
                y: 0,
                width: 640,
                height: 480,
            },
            clip: CameraClip::None,
        }
    }

    fn view_input(camera: SceneCamera) -> WorldViewInput {
        view_input_with_ranks(camera, Vec::new())
    }

    fn view_input_with_ranks(
        camera: SceneCamera,
        ranks: Vec<qa_client::render::scene::material_registrations::ShaderRegistration>,
    ) -> WorldViewInput {
        let mut input = WorldViewInput::new(
            camera,
            ViewTarget::Preview("test".to_string()),
            SourceTime::Milliseconds(1000.0),
        );
        input.source = Some(qa_client::render::scene::world::WorldSurfaceAdmission {
            view: create_source_scene_order(ranks),
            submitted_surfaces: std::collections::HashSet::new(),
            world_operations: None,
        });
        input
    }

    fn input_for(world: &Rc<RefCell<StubWorld>>) -> WorldViewInput {
        let ranks = vec![world.borrow().material.registration];
        view_input_with_ranks(camera(), ranks)
    }

    fn options() -> Q3SceneRenderOptions {
        Q3SceneRenderOptions {
            view_offset: None,
            no_world_model: false,
            split_screen: false,
            supplemental_view_weapon: false,
        }
    }

    fn presented(index: usize, entity: PresentSceneEntity) -> PresentedModel {
        PresentedModel {
            entity_index: index,
            entity,
            options: FoundationModelSourceOptions {
                custom_shader: None,
                custom_skin: None,
                non_normalized_axes: false,
            },
            source: create_model_entity(ContentSceneModel::default_model()),
        }
    }

    fn presented_entity(model: PresentEntityModel) -> PresentSceneEntity {
        PresentSceneEntity {
            actor: None,
            resource: PresentResource::new("models/box.md3"),
            model,
            origin: vec3(10.0, 0.0, 0.0),
            axis: axis(),
            previous_origin: vec3(9.0, 0.0, 0.0),
            frame: 1,
            previous_frame: 0,
            back_lerp: 0.0,
            skin: 0,
            color: vec4(1.0, 1.0, 1.0, 1.0),
            shader_time: 0.0,
            render_flags: 0,
            lighting_origin: vec3(10.0, 0.0, 8.0),
            shadow_plane: 0.0,
        }
    }

    fn content() -> Q3SceneContent {
        Q3SceneContent {
            admission: Q3SceneAdmission {
                id: Q3SceneAdmissionId {
                    origin: SceneAdmissionOrigin::Native,
                    token: 1,
                },
                entities: Vec::new(),
                polygons: Vec::new(),
            },
            models: Vec::new(),
            effects: Vec::new(),
            special_entities: Vec::new(),
            portals: Vec::new(),
            lights: Vec::new(),
        }
    }

    type TestRenderer = (
        ApplicationQ3SceneRenderer,
        Rc<RefCell<Q3AssetShared>>,
        Rc<RefCell<StubPreparation>>,
        Rc<RefCell<StubWorld>>,
    );

    fn renderer() -> TestRenderer {
        renderer_with(Q3RendererResources::<StubHost, StubResourceWorld>::new(
            StubHost::default(),
        ))
    }

    fn renderer_with(resources: Q3RendererResources<StubHost, StubResourceWorld>) -> TestRenderer {
        struct SharedPreparation(Rc<RefCell<StubPreparation>>);

        impl Q3SceneModelPreparation for SharedPreparation {
            fn preload_models(
                &mut self,
                provider: usize,
                entities: &[ClientSceneEntity],
                options: &dyn Fn(&ClientSceneEntity) -> ClientModelSourceOptions,
            ) -> Result<(), qa_client::render::error::RenderError> {
                self.0.borrow_mut().preload_models(provider, entities, options)
            }

            fn prepare_models(
                &mut self,
                provider: usize,
                entities: &[ClientSceneEntity],
                input: &WorldViewInput,
                source: &Q3ModelSource,
                options: &dyn Fn(&ClientSceneEntity) -> ClientModelSourceOptions,
            ) -> Result<Vec<SceneOperation>, qa_client::render::error::RenderError> {
                self.0
                    .borrow_mut()
                    .prepare_models(provider, entities, input, source, options)
            }
        }

        struct SharedWorld(Rc<RefCell<StubWorld>>);

        impl Q3SceneWorld for SharedWorld {
            fn prepare_brush_model(
                &mut self,
                model: usize,
                transform: &ModelTransform,
                input: &mut WorldViewInput,
                entity: SourceEntityOrder,
            ) -> Result<Vec<SceneOperation>, qa_client::render::error::RenderError> {
                self.0.borrow_mut().prepare_brush_model(model, transform, input, entity)
            }

            fn fog_selections(&mut self) -> Vec<Q3FogSelection> {
                self.0.borrow_mut().fog_selections()
            }

            fn require_material(
                &mut self,
                name: &str,
            ) -> Result<RegisteredSceneMaterial, qa_client::render::error::RenderError> {
                self.0.borrow_mut().require_material(name)
            }

            fn resolve_remap(&mut self, name: &str) -> Option<(RegisteredSceneMaterial, f32)> {
                self.0.borrow_mut().resolve_remap(name)
            }

            fn default_material(&mut self) -> RegisteredSceneMaterial {
                self.0.borrow_mut().default_material()
            }

            fn white_image(&mut self) -> RendererImage {
                self.0.borrow_mut().white_image()
            }

            fn fog_image(&mut self) -> u32 {
                self.0.borrow_mut().fog_image()
            }

            fn fog_volume(&mut self, _selection: &Q3FogSelection) -> ClientFogVolume {
                ClientFogVolume {
                    bounds: qa_core::math::Bounds {
                        min: vec3(-8.0, -8.0, -8.0),
                        max: vec3(8.0, 8.0, 8.0),
                    },
                    surface: None,
                    color: vec4(0.5, 0.5, 0.5, 1.0),
                    tc_scale: 1.0,
                }
            }

            fn draw_batches(
                &mut self,
                batches: Vec<MaterialBatch>,
            ) -> Result<Vec<qa_client::render::types::DrawBatch>, qa_client::render::error::RenderError> {
                self.0.borrow_mut().draw_batches(batches)
            }
        }

        let preparation = Rc::new(RefCell::new(StubPreparation::default()));
        let world = Rc::new(RefCell::new(StubWorld::new()));
        let providers = providers();
        let renderer = ApplicationQ3SceneRenderer::new(
            Box::new(SharedPreparation(preparation.clone())),
            Box::new(SharedWorld(world.clone())),
            Box::new(resources),
            providers.clone(),
        );
        (renderer, providers, preparation, world)
    }

    fn world_order(group: &SceneOperation) -> Option<SourceSurfaceOrder> {
        match group {
            SceneOperation::Group(group) => match &group.order {
                SceneGroupOrder::Source { source, .. } => Some(source.clone()),
                _ => None,
            },
            _ => None,
        }
    }

    #[test]
    fn preload_groups_by_provider_and_skips_brush() {
        let (mut renderer, providers, preparation, _) = renderer();
        let loaded = ContentSceneModel::Loaded(SceneLoadedModel {
            path: "models/box.md3".to_string(),
            model: Q3DecodedModel::Framed { frames: Vec::new() },
            resource: PresentResource::new("models/box.md3"),
        });
        providers
            .borrow_mut()
            .provider_contents
            .push(ContentId("q3:test:baseq3:1".to_string()));
        providers.borrow_mut().model_slots.push((loaded.clone(), 0));
        let decoded = presented(
            0,
            presented_entity(PresentEntityModel::Decoded(Q3DecodedModel::Framed {
                frames: Vec::new(),
            })),
        );
        let brush = PresentedModel {
            entity_index: 1,
            entity: presented_entity(PresentEntityModel::BrushModel {
                world: PresentWorld::new("world"),
                model: 2,
            }),
            options: FoundationModelSourceOptions {
                custom_shader: None,
                custom_skin: None,
                non_normalized_axes: false,
            },
            source: create_model_entity(ContentSceneModel::default_model()),
        };
        let mut scene = content();
        let mut first = decoded;
        first.source.model = loaded;
        scene.models.push(first);
        scene.models.push(brush);
        renderer.preload(&[scene]).expect("preload");
        assert_eq!(preparation.borrow().preloads, vec![(0, 1)]);
    }

    #[test]
    fn preload_rejects_missing_provider() {
        let (mut renderer, _, _, _) = renderer();
        let mut scene = content();
        scene.models.push(presented(
            0,
            presented_entity(PresentEntityModel::Decoded(Q3DecodedModel::Framed {
                frames: Vec::new(),
            })),
        ));
        assert!(matches!(renderer.preload(&[scene]), Err(Q3SceneError::MissingProvider)));
    }

    #[test]
    fn operations_require_source_admission() {
        let (mut renderer, _, _, _) = renderer();
        let mut input = view_input(camera());
        input.source = None;
        assert!(matches!(
            renderer.operations(&content(), &input, 0, &options(), &[]),
            Err(Q3SceneError::MissingSource)
        ));
    }

    #[test]
    fn operations_submit_world_polygons() {
        use qa_content::q3::presentation::resources::RendererResources;
        let mut resources = Q3RendererResources::<StubHost, StubResourceWorld>::new(StubHost::default());
        resources.register_shader("test-poly").expect("register");
        let (mut renderer, _, _, world) = renderer_with(resources);
        let mut scene = content();
        scene.admission.polygons.push(Q3AdmittedPoly {
            poly: RefPoly {
                shader: Some(SceneShader::new("test-poly")),
                vertices: vec![
                    qa_content::q3::presentation::ref_entity::RefPolyVertex {
                        position: vec3(0.0, 0.0, 0.0),
                        tex_coord: vec2(0.0, 0.0),
                        color: vec4(255.0, 255.0, 255.0, 255.0),
                    },
                    qa_content::q3::presentation::ref_entity::RefPolyVertex {
                        position: vec3(16.0, 0.0, 0.0),
                        tex_coord: vec2(1.0, 0.0),
                        color: vec4(255.0, 255.0, 255.0, 255.0),
                    },
                    qa_content::q3::presentation::ref_entity::RefPolyVertex {
                        position: vec3(0.0, 16.0, 0.0),
                        tex_coord: vec2(0.0, 1.0),
                        color: vec4(255.0, 255.0, 255.0, 255.0),
                    },
                ],
            },
            fog: None,
        });
        let operations = renderer
            .operations(&scene, &input_for(&world), 7, &options(), &[])
            .expect("operations");
        assert_eq!(operations.len(), 1);
        let order = world_order(&operations[0]).expect("world order");
        assert_eq!(order.surface, 0);
        assert!(matches!(order.entity, SourceEntityOrder::World));
    }

    #[test]
    fn operations_submit_default_models() {
        let (mut renderer, _, _, world) = renderer();
        let mut scene = content();
        scene
            .admission
            .entities
            .push(Q3AdmittedRefEntity::Entity(RefEntity::Model(create_model_entity(
                ContentSceneModel::default_model(),
            ))));
        let operations = renderer
            .operations(&scene, &input_for(&world), 3, &options(), &[])
            .expect("operations");
        assert_eq!(operations.len(), 1);
        let order = world_order(&operations[0]).expect("entity order");
        assert!(matches!(order.entity, SourceEntityOrder::RefEntity { index: 3 }));
    }

    #[test]
    fn operations_skip_third_person_defaults_outside_portals() {
        let (mut renderer, _, _, _) = renderer();
        let mut entity = create_model_entity(ContentSceneModel::default_model());
        entity.shading.render_flags = RF_THIRD_PERSON;
        let mut scene = content();
        scene
            .admission
            .entities
            .push(Q3AdmittedRefEntity::Entity(RefEntity::Model(entity)));
        let operations = renderer
            .operations(&scene, &view_input(camera()), 0, &options(), &[])
            .expect("operations");
        assert!(operations.is_empty());
    }

    #[test]
    fn operations_prepare_mesh_models_through_provider() {
        let (mut renderer, providers, preparation, _) = renderer();
        let loaded = ContentSceneModel::Loaded(SceneLoadedModel {
            path: "models/box.md3".to_string(),
            model: Q3DecodedModel::Framed { frames: Vec::new() },
            resource: PresentResource::new("models/box.md3"),
        });
        providers
            .borrow_mut()
            .provider_contents
            .push(ContentId("q3:test:baseq3:1".to_string()));
        providers.borrow_mut().model_slots.push((loaded.clone(), 0));
        let mut admitted = create_model_entity(ContentSceneModel::default_model());
        admitted.model = loaded;
        let mut scene = content();
        scene
            .admission
            .entities
            .push(Q3AdmittedRefEntity::Entity(RefEntity::Model(admitted)));
        scene.models.push(presented(
            0,
            presented_entity(PresentEntityModel::Decoded(Q3DecodedModel::Framed {
                frames: Vec::new(),
            })),
        ));
        let operations = renderer
            .operations(&scene, &view_input(camera()), 5, &options(), &[])
            .expect("operations");
        assert!(operations.is_empty());
        let calls = preparation.borrow();
        assert_eq!(calls.prepares.len(), 1);
        assert_eq!(calls.prepares[0].0, 0);
        assert!(matches!(calls.prepares[0].2, SourceEntityOrder::RefEntity { index: 5 }));
    }

    #[test]
    fn operations_prepare_brush_models_through_world() {
        let (mut renderer, providers, _, world) = renderer();
        let loaded = ContentSceneModel::Inline(SceneInlineModel {
            path: "*2".to_string(),
            index: 2,
            geometry: PresentWorld::new("world"),
            resource: PresentResource::new("*2"),
            bounds: qa_core::math::Bounds {
                min: vec3(0.0, 0.0, 0.0),
                max: vec3(8.0, 8.0, 8.0),
            },
        });
        providers
            .borrow_mut()
            .provider_contents
            .push(ContentId("q3:test:baseq3:1".to_string()));
        providers.borrow_mut().model_slots.push((loaded.clone(), 0));
        let mut admitted = create_model_entity(ContentSceneModel::default_model());
        admitted.model = loaded;
        admitted.frame = 4;
        let mut scene = content();
        scene
            .admission
            .entities
            .push(Q3AdmittedRefEntity::Entity(RefEntity::Model(admitted)));
        scene.models.push(presented(
            0,
            presented_entity(PresentEntityModel::BrushModel {
                world: PresentWorld::new("world"),
                model: 2,
            }),
        ));
        renderer
            .operations(&scene, &view_input(camera()), 0, &options(), &[])
            .expect("operations");
        assert_eq!(world.borrow().brushes, vec![(2, 14.0)]);
    }

    #[test]
    fn operations_submit_beams_and_sprites() {
        let (mut renderer, _, _, world) = renderer();
        let mut scene = content();
        scene
            .admission
            .entities
            .push(Q3AdmittedRefEntity::Entity(RefEntity::Beam(create_beam_entity())));
        scene
            .admission
            .entities
            .push(Q3AdmittedRefEntity::Entity(RefEntity::Sprite(create_sprite_entity())));
        let operations = renderer
            .operations(&scene, &input_for(&world), 0, &options(), &[])
            .expect("operations");
        assert_eq!(operations.len(), 2);
    }

    #[test]
    fn operations_reject_poly_entities_and_skip_portals() {
        use qa_content::q3::presentation::ref_entity::create_portal_entity;
        let (mut renderer, _, _, _) = renderer();
        let mut scene = content();
        scene
            .admission
            .entities
            .push(Q3AdmittedRefEntity::Entity(RefEntity::Portal(create_portal_entity())));
        let operations = renderer
            .operations(&scene, &view_input(camera()), 0, &options(), &[])
            .expect("portal skipped");
        assert!(operations.is_empty());
    }

    #[test]
    fn operations_order_additions_around_entities() {
        let (mut renderer, _, _, world) = renderer();
        let input = input_for(&world);
        let view = input.source.as_ref().expect("source").view.clone();
        let material = world.borrow().material.clone();
        let world = SceneOperation::Group(
            source_draw_group(
                material.clone(),
                SourceSurfaceOrder {
                    view: view.clone(),
                    entity: SourceEntityOrder::World,
                    surface: 9,
                    fog: 0,
                    dlight: 0,
                },
                Vec::new(),
            )
            .expect("world addition"),
        );
        let other = SceneOperation::Group(
            source_draw_group(
                material,
                SourceSurfaceOrder {
                    view: view.clone(),
                    entity: SourceEntityOrder::RefEntity { index: 1 },
                    surface: 0,
                    fog: 0,
                    dlight: 0,
                },
                Vec::new(),
            )
            .expect("entity addition"),
        );
        let mut scene = content();
        scene
            .admission
            .entities
            .push(Q3AdmittedRefEntity::Entity(RefEntity::Beam(create_beam_entity())));
        let operations = renderer
            .operations(&scene, &input, 0, &options(), &[other.clone(), world.clone()])
            .expect("operations");
        assert_eq!(operations.len(), 3);
        assert_eq!(operations[0], world);
        assert_eq!(operations[2], other);
    }

    #[test]
    fn operations_resolve_remaps() {
        let (mut renderer, _, _, world) = renderer();
        let replacement = StubWorld::new().material;
        let ranks = vec![world.borrow().material.registration, replacement.registration];
        world.borrow_mut().remap = Some((replacement, 1.5));
        let mut scene = content();
        scene
            .admission
            .entities
            .push(Q3AdmittedRefEntity::Entity(RefEntity::Sprite(create_sprite_entity())));
        let operations = renderer
            .operations(&scene, &view_input_with_ranks(camera(), ranks), 0, &options(), &[])
            .expect("operations");
        assert_eq!(operations.len(), 1);
    }

    #[test]
    fn operations_skip_supplemental_and_portal_weapons() {
        let (mut renderer, _, _, _) = renderer();
        let mut entity = create_model_entity(ContentSceneModel::default_model());
        entity.shading.render_flags = RF_FIRST_PERSON;
        let mut scene = content();
        scene
            .admission
            .entities
            .push(Q3AdmittedRefEntity::Entity(RefEntity::Model(entity)));
        let mut supplemental = options();
        supplemental.supplemental_view_weapon = true;
        let operations = renderer
            .operations(&scene, &view_input(camera()), 0, &supplemental, &[])
            .expect("supplemental");
        assert!(operations.is_empty());
        let mut portal = camera();
        portal.clip = CameraClip::Portal {
            plane: qa_core::math::Plane {
                normal: vec3(0.0, 0.0, 1.0),
                distance: 0.0,
            },
            mirror: false,
        };
        let operations = renderer
            .operations(&scene, &view_input(portal), 0, &options(), &[])
            .expect("portal");
        assert!(operations.is_empty());
    }

    #[test]
    fn operations_offset_first_person_models() {
        let (mut renderer, providers, preparation, _) = renderer();
        let loaded = ContentSceneModel::Loaded(SceneLoadedModel {
            path: "models/v_weap.md3".to_string(),
            model: Q3DecodedModel::Framed { frames: Vec::new() },
            resource: PresentResource::new("models/v_weap.md3"),
        });
        providers
            .borrow_mut()
            .provider_contents
            .push(ContentId("q3:test:baseq3:1".to_string()));
        providers.borrow_mut().model_slots.push((loaded.clone(), 0));
        let mut admitted = create_model_entity(ContentSceneModel::default_model());
        admitted.model = loaded;
        admitted.shading.render_flags = RF_FIRST_PERSON;
        let mut scene = content();
        scene
            .admission
            .entities
            .push(Q3AdmittedRefEntity::Entity(RefEntity::Model(admitted)));
        scene.models.push(presented(
            0,
            presented_entity(PresentEntityModel::Decoded(Q3DecodedModel::Framed {
                frames: Vec::new(),
            })),
        ));
        let mut armed = options();
        armed.view_offset = Some(vec3(0.0, 0.0, 4.0));
        renderer
            .operations(&scene, &view_input(camera()), 0, &armed, &[])
            .expect("operations");
        assert_eq!(preparation.borrow().prepares.len(), 1);
    }

    #[test]
    fn operations_reject_missing_descriptors() {
        let (mut renderer, providers, _, _) = renderer();
        let loaded = ContentSceneModel::Loaded(SceneLoadedModel {
            path: "models/box.md3".to_string(),
            model: Q3DecodedModel::Framed { frames: Vec::new() },
            resource: PresentResource::new("models/box.md3"),
        });
        providers
            .borrow_mut()
            .provider_contents
            .push(ContentId("q3:test:baseq3:1".to_string()));
        providers.borrow_mut().model_slots.push((loaded.clone(), 0));
        let mut admitted = create_model_entity(ContentSceneModel::default_model());
        admitted.model = loaded;
        let mut scene = content();
        scene
            .admission
            .entities
            .push(Q3AdmittedRefEntity::Entity(RefEntity::Model(admitted)));
        assert!(matches!(
            renderer.operations(&scene, &view_input(camera()), 0, &options(), &[]),
            Err(Q3SceneError::MissingDescriptor)
        ));
    }

    #[test]
    fn closed_renderer_rejects_calls() {
        let (mut renderer, _, _, _) = renderer();
        renderer.close();
        assert!(renderer.is_closed());
        assert!(matches!(renderer.preload(&[]), Err(Q3SceneError::Closed)));
        assert!(matches!(
            renderer.operations(&content(), &view_input(camera()), 0, &options(), &[]),
            Err(Q3SceneError::Closed)
        ));
    }

    #[test]
    fn entity_conversion_maps_models() {
        let framed = client_scene_entity(&presented_entity(PresentEntityModel::Decoded(Q3DecodedModel::Framed {
            frames: Vec::new(),
        })));
        assert!(matches!(framed.model, ClientSceneModel::BrushModel));
        assert_eq!(framed.transform.scale, vec3(1.0, 1.0, 1.0));
        assert!(matches!(framed.pose, ScenePose::Frame { frame: 1, .. }));
        assert!(framed.actor_slot.is_none());
        let brush = client_scene_entity(&presented_entity(PresentEntityModel::BrushModel {
            world: PresentWorld::new("world"),
            model: 1,
        }));
        assert!(matches!(brush.model, ClientSceneModel::BrushModel));
    }
}
