//! An invocation-scoped borrow of the session's scene, catalog and view scratch.
use super::SceneComposer;
use crate::model_assets::{ModelCatalog, ModelProfile, ModelView};
use crate::modules::renderer_contexts::Contexts;
use qa_compat::{
    renderer::{EntityKind, EntitySubmission, SceneService, SceneView},
    services::CallError,
};
use qa_core::{primitives::ModuleId, sys_events::SeatId};
use qa_render::shader::reference::{ShaderContextId, ShaderRef, ShaderView};
use qa_render::{
    Vertex, Viewport,
    scene::{Draw2d, Light},
    world::{WorldId, WorldPresentationId},
};

#[derive(Clone, Copy)]
struct Target {
    profile: ModelProfile,
    viewport: Viewport,
    seat: Option<SeatId>,
    world: Option<(WorldId, WorldPresentationId)>,
    presentation: qa_render::material::world_load::PresentationDefaults,
    rails: [qa_console::cvars::View; 3],
    lighting: [qa_console::cvars::View; 2],
    software_waterwarp: qa_console::cvars::View,
}
impl Target {
    fn world_presentation(
        &self,
        resources: ModelView<'_>,
        world: WorldId,
    ) -> Option<qa_render::material::world_load::LoadedWorldPresentation> {
        resources.world_presentation(
            world,
            Some(self.profile),
            self.world
                .filter(|&(selected, _)| selected == world)
                .map(|(_, presentation)| presentation),
        )
    }
}

/// Mutable renderer state belongs to the module across registration and frames.
pub(crate) struct SceneRole {
    selected_world: Option<WorldId>,
    color: [u8; 4],
    target: Option<Target>,
}
impl Default for SceneRole {
    fn default() -> Self {
        Self {
            selected_world: None,
            color: [255; 4],
            target: None,
        }
    }
}
impl SceneRole {
    pub(crate) fn reset_instance(&mut self) {
        self.selected_world = None;
        self.color = [255; 4];
        if let Some(target) = &mut self.target {
            target.world = None;
        }
    }

    pub(crate) fn select_world(&mut self, world: WorldId) {
        self.selected_world = Some(world);
    }
    pub(crate) fn selected_world(&self) -> Option<WorldId> {
        self.selected_world.or_else(|| {
            self.target
                .and_then(|target| target.world.map(|world| world.0))
        })
    }
    pub(crate) fn presentation_for(&self, world: WorldId) -> Option<WorldPresentationId> {
        self.target.and_then(|target| {
            target
                .world
                .filter(|selected| selected.0 == world)
                .map(|selected| selected.1)
        })
    }
    pub(crate) fn profile(&self) -> Option<ModelProfile> {
        self.target.map(|target| target.profile)
    }

    pub(crate) fn configure(
        &mut self,
        profile: ModelProfile,
        viewport: Viewport,
        seat: Option<SeatId>,
        world: Option<(WorldId, WorldPresentationId)>,
        presentation: qa_render::material::world_load::PresentationDefaults,
        rails: [qa_console::cvars::View; 3],
        lighting: [qa_console::cvars::View; 2],
        software_waterwarp: qa_console::cvars::View,
    ) {
        self.target = Some(Target {
            profile,
            viewport,
            seat,
            world,
            presentation,
            rails,
            lighting,
            software_waterwarp,
        });
    }
}

pub(crate) struct SceneBorrow<'a> {
    composer: Option<&'a mut SceneComposer>,
    models: Option<&'a ModelCatalog>,
    assets: Option<&'a qa_render::Assets>,
    role: &'a mut SceneRole,
    rails: (i32, i32, f32),
    lighting: [f32; 2],
    software_waterwarp: bool,
    client: Option<qa_render::effects::EffectClient>,
    sun_direction: Option<qa_core::primitives::Vec3>,
    native: &'a Contexts,
    context: Option<ShaderContextId>,
}
impl<'a> SceneBorrow<'a> {
    pub(crate) fn new(
        composer: Option<&'a mut SceneComposer>,
        resources: Option<ModelView<'a>>,
        role: &'a mut SceneRole,
        cvars: &qa_console::cvars::Cvars,
        client: Option<qa_render::effects::EffectClient>,
        sun_direction: Option<qa_core::primitives::Vec3>,
        native: &'a Contexts,
        context: Option<ShaderContextId>,
    ) -> Self {
        let rails = role.target.as_ref().map_or((6, 16, 32.), |target| {
            (
                cvars.integer_view(target.rails[0]).unwrap_or(0),
                cvars.integer_view(target.rails[1]).unwrap_or(0),
                cvars.numeric(target.rails[2]).unwrap_or(0.),
            )
        });
        let lighting = role.target.as_ref().map_or([1.; 2], |target| {
            target
                .lighting
                .map(|view| cvars.numeric(view).unwrap_or(0.))
        });
        let software_waterwarp = role
            .target
            .as_ref()
            .is_some_and(|target| cvars.numeric(target.software_waterwarp).unwrap_or(0.0) != 0.0);
        Self {
            composer,
            models: resources.map(|view| view.catalog),
            assets: resources.map(|view| view.assets),
            role,
            rails,
            lighting,
            software_waterwarp,
            client,
            sun_direction,
            native,
            context,
        }
    }
}

fn shader_handle(
    native: &Contexts,
    context: Option<ShaderContextId>,
    models: &ModelCatalog,
    profile: ModelProfile,
    handle: i32,
) -> Option<ShaderRef> {
    context
        .and_then(|context| native.shader(context, handle))
        .map(ShaderRef::Native)
        .or_else(|| models.native_default(profile).map(ShaderRef::Direct))
}

/// Select the actual guest entry separately from reconstructing a short scene
/// borrow. Resuming a parent restores ownership without clearing child output.
pub(crate) fn select_publisher(
    composer: Option<&mut SceneComposer>,
    module: ModuleId,
    entering: bool,
) {
    if let Some(composer) = composer {
        if entering
            && composer.publisher != Some(module)
            && let Some(frame) = composer.frame.as_mut()
        {
            frame.clear_scene();
        }
        composer.publisher = Some(module);
    }
}
impl SceneService for SceneBorrow<'_> {
    fn active(&self) -> bool {
        self.composer
            .as_deref()
            .is_some_and(|composer| composer.frame.is_some())
    }
    fn viewport(&self) -> Option<Viewport> {
        self.role.target.map(|target| target.viewport)
    }
    fn select_world(&mut self, world: WorldId) -> Result<(), CallError> {
        // The cold/frozen catalog already validated the world name. Preserve
        // this selection even before the frontend has been allocated.
        self.role.selected_world = Some(world);
        Ok(())
    }
    fn selected_world(&self) -> Option<WorldId> {
        self.role.selected_world()
    }
    fn mark_fragments(
        &mut self,
        projection: &qa_render::marks::MarkProjection,
        max_points: usize,
        max_fragments: usize,
        emit: &mut dyn FnMut(
            qa_core::primitives::IndexSpan,
            &[qa_core::primitives::Vec3],
        ) -> Result<(), CallError>,
    ) -> Result<usize, CallError> {
        let (Some(world_id), Some(target), Some(assets), Some(models)) = (
            self.selected_world(),
            self.role.target.as_ref(),
            self.assets,
            self.models,
        ) else {
            return Ok(0);
        };
        let resources = ModelView {
            assets,
            catalog: models,
        };
        let Some(presentation) = target
            .world_presentation(resources, world_id)
            .and_then(|entry| assets.world_presentation(entry.id))
        else {
            return Ok(0);
        };
        let Some(world) = assets.world(world_id) else {
            return Ok(0);
        };
        let Some(view) = self
            .composer
            .as_deref_mut()
            .and_then(|composer| composer.scenes.get_mut(world_id.0 as usize))
            .and_then(Option::as_mut)
            .zip(target.seat)
            .map(|(scene, seat)| &mut scene.views[seat.index()])
        else {
            return Ok(0);
        };
        view.mark_fragments(
            world,
            presentation,
            assets,
            projection,
            max_points,
            max_fragments,
            emit,
        )
        .map_err(CallError::from)
    }
    fn clear(&mut self) {
        if let Some(frame) = self
            .composer
            .as_deref_mut()
            .and_then(|composer| composer.frame.as_mut())
        {
            frame.clear_scene();
        }
    }
    fn light(&mut self, light: Light) -> bool {
        self.composer
            .as_deref_mut()
            .and_then(|composer| composer.frame.as_mut())
            .is_some_and(|frame| frame.add_light(light))
    }
    fn entity(&mut self, submitted: EntitySubmission) -> bool {
        use qa_render::scene::SceneModel;
        let Some(composer) = self.composer.as_deref_mut() else {
            return false;
        };
        let inline = (submitted.kind == EntityKind::Model)
            .then(|| self.models?.renderer_inline(submitted.model))
            .flatten()
            .map(|binding| (binding, composer.has_world(binding.world)));
        let Some(frame) = composer.frame.as_mut() else {
            return false;
        };
        let Some(target) = self.role.target.as_ref() else {
            return frame.reject_submission();
        };
        let (Some(models), Some(assets)) = (self.models, self.assets) else {
            return frame.reject_submission();
        };
        let mut pose = submitted.pose;
        if !pose.origin.is_finite() {
            return frame.reject_submission();
        }
        if let Some((binding, world_present)) = inline {
            let Some(presentation) = target
                .world_presentation(
                    ModelView {
                        assets,
                        catalog: models,
                    },
                    binding.world,
                )
                .filter(|_| world_present)
            else {
                return frame.reject_submission();
            };
            pose.model = SceneModel::World {
                world: binding.world,
                presentation: presentation.id,
                surfaces: binding.surfaces,
            };
            pose.shader = None;
            return frame.add_entity(pose);
        }
        let shader = if submitted.custom_shader != 0 && submitted.kind != EntityKind::Beam {
            let Some(shader) = shader_handle(
                self.native,
                self.context,
                models,
                target.profile,
                submitted.custom_shader,
            ) else {
                return frame.reject_submission();
            };
            Some(shader)
        } else {
            None
        };
        if submitted.kind == EntityKind::Model {
            let Some(registration) = models.renderer_registration(submitted.model, target.profile)
            else {
                return frame.reject_submission();
            };
            let Some(bindings) = models.bindings(
                registration,
                u32::try_from(submitted.skin).unwrap_or(0),
                crate::model_assets::ModelMaterialPolicy {
                    custom_skin: submitted.custom_skin,
                    frame: u32::try_from(submitted.frame).unwrap_or(0),
                    ..Default::default()
                },
            ) else {
                return frame.reject_submission();
            };
            let choices = bindings.choices();
            let Some(first_choice) = choices.clone().next() else {
                return frame.reject_submission();
            };
            let nodes = self
                .context
                .and_then(|context| self.native.resource_nodes(context, first_choice.resource));
            let shaders = ShaderView {
                native: self.native,
                context: self.context,
            };
            if !choices.clone().all(|choice| {
                shader.is_some()
                    || shaders
                        .choice(choice.shader, choice.registrations, nodes, choice.default)
                        .is_some()
            }) {
                return frame.reject_submission();
            }
            let Some(first) = assets.model(first_choice.model) else {
                return frame.reject_submission();
            };
            let count = first.frame_count();
            if count == 0 {
                return frame.reject_submission();
            }
            let current = if submitted.wrap_frames {
                submitted.frame % count as i32
            } else {
                submitted.frame
            };
            let old = if submitted.wrap_frames {
                submitted.old_frame % count as i32
            } else {
                submitted.old_frame
            };
            if current < 0 || old < 0 || current as u32 >= count || old as u32 >= count {
                pose.frame = 0;
                pose.old_frame = 0;
            } else {
                pose.frame = current as u32;
                pose.old_frame = old as u32;
            }
            return frame.add_entities(choices.filter_map(|choice| {
                let mut entity = pose;
                entity.model = SceneModel::Mesh(choice.model);
                entity.shader = Some(shader.or_else(|| {
                    shaders.choice(choice.shader, choice.registrations, nodes, choice.default)
                })?);
                Some(entity)
            }));
        }
        pose.shader = Some(if submitted.kind == EntityKind::Beam {
            ShaderRef::Direct(composer.beam)
        } else {
            let Some(shader) =
                shader.or_else(|| models.native_default(target.profile).map(ShaderRef::Direct))
            else {
                return frame.reject_submission();
            };
            shader
        });
        pose.model = match submitted.kind {
            EntityKind::Model => return false,
            EntityKind::Sprite => SceneModel::Sprite {
                radius: submitted.radius,
                rotation: submitted.rotation,
            },
            EntityKind::Beam => SceneModel::Beam {
                end: submitted.old_origin - pose.origin,
                radius: 4.0,
                world_space: true,
            },
            EntityKind::RailCore => SceneModel::RailCore {
                end: submitted.old_origin,
                width: self.rails.0,
            },
            EntityKind::RailRings => SceneModel::RailRings {
                end: submitted.old_origin,
                width: self.rails.1,
                segment_length: self.rails.2,
            },
            EntityKind::Lightning => SceneModel::Lightning {
                end: submitted.old_origin,
            },
            EntityKind::Portal => SceneModel::Portal {
                camera: submitted.old_origin,
                rotation: submitted.skin,
            },
            EntityKind::PolyMarker => SceneModel::PolyMarker,
        };
        if submitted.kind == EntityKind::Beam {
            // R_RotateForEntity uses world orientation for RT_BEAM; its
            // surface routine intentionally leaves the origin addition out.
            pose.origin = qa_core::primitives::Vec3::default();
            pose.color = [255, 0, 0, 255];
        }
        pose.frame = submitted.frame as u32;
        pose.old_frame = submitted.old_frame as u32;
        frame.add_entity(pose)
    }
    fn poly(&mut self, shader: i32, count: usize, vertex: &mut dyn FnMut(usize) -> Vertex) -> bool {
        if shader == 0 {
            return false;
        }
        let Some(target) = self.role.target.as_ref() else {
            return false;
        };
        let Some(shader) = self.models.and_then(|models| {
            shader_handle(self.native, self.context, models, target.profile, shader)
        }) else {
            return false;
        };
        self.composer
            .as_deref_mut()
            .and_then(|composer| composer.frame.as_mut())
            .is_some_and(|frame| frame.add_poly_from(shader, count, vertex))
    }
    fn color(&mut self, color: Option<[f32; 4]>) {
        self.role.color = color
            .unwrap_or([1.; 4])
            .map(|component| (component.clamp(0., 1.) * 255.) as u8);
    }
    fn picture(&mut self, shader: i32, mut rect: [f32; 4], texcoords: [f32; 4]) -> bool {
        let Some(target) = self.role.target.as_ref() else {
            return false;
        };
        let Some(shader) = self.models.and_then(|models| {
            shader_handle(self.native, self.context, models, target.profile, shader)
        }) else {
            return false;
        };
        rect[0] += target.viewport.x as f32;
        rect[1] += target.viewport.y as f32;
        let Some(draw) = (Draw2d {
            rect,
            texcoords,
            shader,
            color: self.role.color,
        })
        .clipped(target.viewport) else {
            return false;
        };
        self.composer
            .as_deref_mut()
            .and_then(|composer| composer.frame.as_mut())
            .is_some_and(|frame| frame.draw_2d(draw))
    }
    fn view(&mut self, mut view: SceneView) -> Result<bool, CallError> {
        view.world &= !view
            .refdef
            .flags
            .contains(qa_core::primitives::PlayerViewFlags::NO_WORLD_MODEL);
        let selected_world = self.selected_world();
        let assets = self.assets.ok_or(CallError::Geometry)?;
        let resources = ModelView {
            assets,
            catalog: self.models.ok_or(CallError::Geometry)?,
        };
        let target = self.role.target.as_ref().ok_or(CallError::Geometry)?;
        let composer = self.composer.as_deref_mut().ok_or(CallError::Geometry)?;
        let local = view.refdef.viewport;
        if !local.fits(target.viewport.width, target.viewport.height) {
            return Err(CallError::Geometry);
        }
        view.refdef.viewport.x += target.viewport.x;
        view.refdef.viewport.y += target.viewport.y;
        super::apply_presentation(&mut view.refdef, &target.presentation, composer.kind);
        view.refdef.software_waterwarp &= self.software_waterwarp;
        view.refdef
            .validate(u32::MAX, u32::MAX)
            .ok_or(CallError::Geometry)?;
        let frame = composer.frame.as_mut().ok_or(CallError::Geometry)?;
        let modulate = selected_world
            .and_then(|world| target.world_presentation(resources, world))
            .map_or(1., |entry| entry.profile.q2_modulate);
        let point_view = selected_world.zip(target.seat).and_then(|(world, seat)| {
            composer
                .scenes
                .get_mut(world.0 as usize)?
                .as_mut()
                .map(|scene| &mut scene.views[seat.index()])
        });
        super::light_view_entities(
            frame,
            assets,
            selected_world,
            &view.refdef,
            view.world,
            self.lighting,
            point_view,
            modulate,
            self.sun_direction
                .unwrap_or_else(qa_render::lighting::default_sun_direction),
        );
        let recipient = (view.world && !view.refdef.hyperspace)
            .then_some(self.client)
            .flatten();
        let accepted = frame.for_client(recipient, |frame| -> Result<bool, CallError> {
            if view.world && !view.refdef.hyperspace {
                let world_id = selected_world.ok_or(CallError::Geometry)?;
                let presentation = target
                    .world_presentation(resources, world_id)
                    .ok_or(CallError::Geometry)?
                    .id;
                let seat = target.seat.ok_or(CallError::Geometry)?;
                let world = assets.world(world_id).ok_or(CallError::Geometry)?;
                let scratch = composer
                    .scenes
                    .get_mut(world_id.0 as usize)
                    .and_then(Option::as_mut)
                    .ok_or(CallError::Geometry)?;
                let surfaces = scratch.views[seat.index()]
                    .query(world, view.refdef, None, &view.hidden_areas)
                    .map_err(|_| CallError::Geometry)?;
                if !frame.add_world(world_id, presentation, surfaces) {
                    return Ok(false);
                }
                Ok(frame.render_world_scene(
                    world_id,
                    presentation,
                    view.refdef,
                    &view.hidden_areas,
                    &mut scratch.views[seat.index()],
                    assets,
                    ShaderView {
                        native: self.native,
                        context: self.context,
                    },
                ))
            } else {
                Ok(frame.render_scene(
                    view.refdef,
                    &view.hidden_areas,
                    assets,
                    ShaderView {
                        native: self.native,
                        context: self.context,
                    },
                ))
            }
        })?;
        if accepted && let Some(seat) = target.seat {
            composer.native_axes[seat.index()] = Some(view.refdef.axes);
        }
        Ok(accepted)
    }
}
