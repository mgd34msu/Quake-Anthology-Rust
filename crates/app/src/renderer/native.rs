//! An invocation-scoped borrow of the session's scene, catalog and view scratch.
use super::SceneComposer;
use crate::model_assets::{ModelCatalog, ModelProfile};
use qa_compat::{
    renderer::{EntityKind, EntitySubmission, SceneService, SceneView},
    services::CallError,
};
use qa_core::{primitives::ModuleId, sys_events::SeatId};
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
}
impl Target {
    fn world_presentation<'a>(
        &self,
        scenes: &'a [Option<super::WorldScene>],
        world: WorldId,
    ) -> Option<&'a qa_render::material::world_load::LoadedWorldPresentation> {
        let scene = scenes.get(world.0 as usize)?.as_ref()?;
        if let Some((selected, presentation)) = self.world
            && selected == world
        {
            return scene
                .presentations
                .iter()
                .find(|entry| entry.id == presentation);
        }
        scene.presentations.iter().find(|entry| {
            entry.profile.rules == self.profile.rules
                && entry.profile.indexed_palette == self.profile.palette
                && entry.profile.image_settings == self.profile.image_settings
        })
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
    pub(crate) fn configure(
        &mut self,
        profile: ModelProfile,
        viewport: Viewport,
        seat: Option<SeatId>,
        world: Option<(WorldId, WorldPresentationId)>,
        presentation: qa_render::material::world_load::PresentationDefaults,
        rails: [qa_console::cvars::View; 3],
    ) {
        self.target = Some(Target {
            profile,
            viewport,
            seat,
            world,
            presentation,
            rails,
        });
    }
}

pub(crate) struct SceneBorrow<'a> {
    composer: Option<&'a mut SceneComposer>,
    models: Option<&'a ModelCatalog>,
    role: &'a mut SceneRole,
    rails: (i32, i32, f32),
}
impl<'a> SceneBorrow<'a> {
    pub(crate) fn new(
        composer: Option<&'a mut SceneComposer>,
        models: Option<&'a ModelCatalog>,
        role: &'a mut SceneRole,
        module: ModuleId,
        cvars: &qa_console::cvars::Cvars,
    ) -> Self {
        let rails = role.target.as_ref().map_or((6, 16, 32.), |target| {
            (
                cvars.integer_view(target.rails[0]).unwrap_or(0),
                cvars.integer_view(target.rails[1]).unwrap_or(0),
                cvars.numeric(target.rails[2]).unwrap_or(0.),
            )
        });
        let mut borrow = Self {
            composer,
            models,
            role,
            rails,
        };
        if let Some(composer) = borrow.composer.as_deref_mut()
            && composer.publisher != Some(module)
        {
            if let Some(frame) = composer.frame.as_mut() {
                frame.clear_scene();
            }
            composer.publisher = Some(module);
        }
        borrow
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
        self.role.selected_world.or_else(|| {
            self.role
                .target
                .as_ref()
                .and_then(|target| target.world.map(|world| world.0))
        })
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
        let Some(frame) = composer.frame.as_mut() else {
            return false;
        };
        let Some(target) = self.role.target.as_ref() else {
            return frame.reject_submission();
        };
        let Some(models) = self.models else {
            return frame.reject_submission();
        };
        let mut pose = submitted.pose;
        if !pose.origin.is_finite() {
            return frame.reject_submission();
        }
        let material = (submitted.custom_shader != 0).then(|| {
            models
                .shader_binding(submitted.custom_shader, target.profile)
                .unwrap_or(qa_render::MaterialId(0))
        });
        if submitted.kind == EntityKind::Model {
            if let Some(binding) = models.renderer_inline(submitted.model) {
                let Some(presentation) = target.world_presentation(&composer.scenes, binding.world)
                else {
                    return frame.reject_submission();
                };
                pose.model = SceneModel::World {
                    world: binding.world,
                    presentation: presentation.id,
                    surfaces: binding.surfaces,
                };
                pose.material = material;
                return frame.add_entity(pose);
            }
            let Some(registration) = models.renderer_registration(submitted.model, target.profile)
            else {
                return frame.reject_submission();
            };
            let Some(bindings) = models.bindings(
                registration,
                u32::try_from(submitted.skin).unwrap_or(0),
                crate::model_assets::ModelMaterialPolicy {
                    custom_skin: submitted.custom_skin,
                    ..Default::default()
                },
            ) else {
                return frame.reject_submission();
            };
            let Some(first) = bindings
                .clone()
                .next()
                .and_then(|binding| composer.assets.model(binding.model))
            else {
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
            return frame.add_entities(bindings.map(|binding| {
                let mut entity = pose;
                entity.model = SceneModel::Mesh(binding.model);
                entity.material = material.or(Some(binding.material));
                entity
            }));
        }
        pose.material = Some(if submitted.kind == EntityKind::Beam {
            composer.beam
        } else {
            material.unwrap_or(qa_render::MaterialId(0))
        });
        pose.model = match submitted.kind {
            EntityKind::Model => return false,
            EntityKind::Sprite => SceneModel::Sprite {
                radius: submitted.radius,
                rotation: submitted.rotation,
            },
            EntityKind::Beam => SceneModel::Beam {
                end: submitted.old_origin,
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
        pose.frame = submitted.frame as u32;
        pose.old_frame = submitted.old_frame as u32;
        frame.add_entity(pose)
    }
    fn poly(&mut self, shader: i32, count: usize, vertex: &mut dyn FnMut(usize) -> Vertex) -> bool {
        let Some(target) = self.role.target.as_ref() else {
            return false;
        };
        let Some(material) = self
            .models
            .and_then(|models| models.shader_binding(shader, target.profile))
        else {
            return false;
        };
        self.composer
            .as_deref_mut()
            .and_then(|composer| composer.frame.as_mut())
            .is_some_and(|frame| frame.add_poly_from(material, count, vertex))
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
        let Some(material) = self
            .models
            .and_then(|models| models.shader_binding(shader, target.profile))
        else {
            return false;
        };
        rect[0] += target.viewport.x as f32;
        rect[1] += target.viewport.y as f32;
        let Some(draw) = (Draw2d {
            rect,
            texcoords,
            material,
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
        let target = self.role.target.as_ref().ok_or(CallError::Geometry)?;
        let composer = self.composer.as_deref_mut().ok_or(CallError::Geometry)?;
        let local = view.refdef.viewport;
        if !local.fits(target.viewport.width, target.viewport.height) {
            return Err(CallError::Geometry);
        }
        view.refdef.viewport.x += target.viewport.x;
        view.refdef.viewport.y += target.viewport.y;
        super::apply_presentation(&mut view.refdef, &target.presentation, composer.kind);
        view.refdef
            .validate(u32::MAX, u32::MAX)
            .ok_or(CallError::Geometry)?;
        let frame = composer.frame.as_mut().ok_or(CallError::Geometry)?;
        if view.world && !view.refdef.hyperspace {
            let (world_id, presentation) = target.world.ok_or(CallError::Geometry)?;
            if self.role.selected_world != Some(world_id) {
                return Err(CallError::Geometry);
            }
            let seat = target.seat.ok_or(CallError::Geometry)?;
            let world = composer.assets.world(world_id).ok_or(CallError::Geometry)?;
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
        }
        let accepted = frame.render_scene(view.refdef, &view.hidden_areas, &composer.assets);
        if accepted && let Some(seat) = target.seat {
            composer.native_axes[seat.index()] = Some(view.refdef.axes);
        }
        Ok(accepted)
    }
}
