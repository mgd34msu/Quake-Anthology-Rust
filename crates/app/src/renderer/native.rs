//! An invocation-scoped borrow of the session's scene, catalog and view scratch.
use super::SceneComposer;
use crate::model_assets::{ModelCatalog, ModelProfile};
use qa_compat::{
    renderer::{SceneService, SceneView},
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
    ) {
        self.target = Some(Target {
            profile,
            viewport,
            seat,
            world,
            presentation,
        });
    }
}

pub(crate) struct SceneBorrow<'a> {
    composer: Option<&'a mut SceneComposer>,
    models: Option<&'a ModelCatalog>,
    role: &'a mut SceneRole,
}
impl<'a> SceneBorrow<'a> {
    pub(crate) fn new(
        composer: Option<&'a mut SceneComposer>,
        models: Option<&'a ModelCatalog>,
        role: &'a mut SceneRole,
        module: ModuleId,
    ) -> Self {
        let mut borrow = Self {
            composer,
            models,
            role,
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
