//! Quake III presentation: ui adapters.
//!
//! Donor provenance: `src/content/q3/presentation/ui-adapters.ts`.

use qa_core::math::vec3;

// Intra-group imports: sibling modules split from the same flat port.
use crate::q3::presentation::draw_tools::{Draw2D, Rect2d};
use crate::q3::presentation::hud::{shared, Shared};
use crate::q3::presentation::mission_hud::CinematicService;
use crate::q3::presentation::ref_entity::{RF_LIGHTING_ORIGIN, RF_NOSHADOW};
use crate::q3::presentation::refdef::RDF_NOWORLDMODEL;
use crate::q3::presentation::resources::RendererResources;
use crate::q3::presentation::retail_snapshot::{create_model_entity_with, create_refdef, RefEntity, SceneModel};
use qa_core::math::angles_to_axis;

/// Model paint request (`UiModelPaintRequest`).
#[derive(Clone)]
pub struct UiModelPaintRequest {
    /// Drawing context (must share the painter's queue).
    pub draw: Draw2D,
    /// Model.
    pub model: SceneModel,
    /// Rectangle.
    pub rect: Rect2d,
    /// Time.
    pub time: i32,
    /// Angle.
    pub angle: f32,
    /// Horizontal FOV (0 means viewport width).
    pub field_of_view_x: f32,
    /// Vertical FOV (0 means viewport height).
    pub field_of_view_y: f32,
}

/// Seat model painter (`EngineUiModelPainter`).
pub struct EngineUiModelPainter {
    /// Renderer resources.
    pub resources: Shared<dyn RendererResources>,
    /// Seat drawing context.
    pub commands: Draw2D,
}

impl EngineUiModelPainter {
    /// Assemble a painter.
    pub fn new(resources: Shared<dyn RendererResources>, commands: Draw2D) -> Self {
        Self { resources, commands }
    }

    /// Paint a model (`paint`).
    pub fn paint(&self, request: &UiModelPaintRequest) {
        if !request.draw.shares_queue(&self.commands) {
            panic!("UI model painter must use its seat's drawing queue");
        }
        let viewport = request.draw.adjust(request.rect);
        let mut refdef = create_refdef();
        refdef.render_flags = RDF_NOWORLDMODEL;
        refdef.view_axis = [vec3(1.0, 0.0, 0.0), vec3(0.0, 1.0, 0.0), vec3(0.0, 0.0, 1.0)];
        refdef.x = viewport.x.trunc() as i32;
        refdef.y = viewport.y.trunc() as i32;
        refdef.width = viewport.width.trunc() as i32;
        refdef.height = viewport.height.trunc() as i32;
        refdef.fov_x = if request.field_of_view_x != 0.0 {
            request.field_of_view_x
        } else {
            viewport.width
        };
        refdef.fov_y = if request.field_of_view_y != 0.0 {
            request.field_of_view_y
        } else {
            viewport.height
        };
        refdef.time = request.time;
        let bounds = self.resources.borrow().model_bounds(&request.model);
        let mut entity = create_model_entity_with(request.model.clone());
        let length = 0.5 * (bounds.max.z - bounds.min.z);
        entity.origin = vec3(
            length / 0.268,
            0.5 * (bounds.min.y + bounds.max.y),
            -0.5 * (bounds.min.z + bounds.max.z),
        );
        entity.lighting_origin = entity.origin;
        entity.old_origin = entity.origin;
        entity.axis = angles_to_axis(vec3(0.0, request.angle, 0.0));
        entity.render_flags = RF_LIGHTING_ORIGIN | RF_NOSHADOW;
        self.resources.borrow_mut().clear_scene();
        self.resources.borrow_mut().add_ref_entity(RefEntity::Model(entity));
        self.resources.borrow_mut().render_scene(&refdef);
    }

    /// Shared-queue handle for hosts.
    pub fn shared_painter(
        resources: Shared<dyn RendererResources>,
        commands: Draw2D,
    ) -> Shared<dyn ModelPainterService> {
        shared(PainterService(Self::new(resources, commands)))
    }
}

/// Model painter service (`paintModel`).
pub trait ModelPainterService {
    /// Paint a model.
    fn paint(&mut self, request: &UiModelPaintRequest);
}

/// Service wrapper over [`EngineUiModelPainter`].
pub struct PainterService(pub EngineUiModelPainter);

impl ModelPainterService for PainterService {
    fn paint(&mut self, request: &UiModelPaintRequest) {
        self.0.paint(request);
    }
}

/// Seat cinematics handle (`EngineUiCinematics`).
pub type EngineUiCinematics = Shared<dyn CinematicService>;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::q3::base::shared::definitions::Product;
    use crate::q3::presentation::draw_tools::{rect2d, CoordinateSpace, HudDrawSink};
    use crate::q3::presentation::hud::tests::*;
    use crate::q3::presentation::hud::{shared, Shared};
    use crate::q3::presentation::retail_snapshot::SceneModel;

    #[test]
    fn model_painter_happy_path() {
        let game = world(Product::Baseq3);
        let painter = EngineUiModelPainter::new(game.resources.clone(), game.draw.clone());
        painter.paint(&UiModelPaintRequest {
            draw: game.draw.clone(),
            model: SceneModel::Loaded { id: 7 },
            rect: rect2d(0.0, 0.0, 100.0, 100.0),
            time: 5,
            angle: 10.0,
            field_of_view_x: 0.0,
            field_of_view_y: 60.0,
        });
        assert_eq!(game.resources.borrow().scenes, vec!["clear", "add", "render"]);
    }

    #[test]
    #[should_panic(expected = "UI model painter must use its seat's drawing queue")]
    fn model_painter_queue_mismatch() {
        let game = world(Product::Baseq3);
        let other: Shared<dyn HudDrawSink> = shared(FakeSink::default());
        let painter = EngineUiModelPainter::new(game.resources.clone(), game.draw.clone());
        painter.paint(&UiModelPaintRequest {
            draw: Draw2D::new(other, CoordinateSpace::Stretch640, 640, 480),
            model: SceneModel::Default,
            rect: rect2d(0.0, 0.0, 10.0, 10.0),
            time: 0,
            angle: 0.0,
            field_of_view_x: 0.0,
            field_of_view_y: 0.0,
        });
    }
}
