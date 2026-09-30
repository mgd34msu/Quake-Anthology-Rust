//! Quake III presentation: ui adapters.
//!
//! Donor provenance: `src/content/q3/presentation/ui-adapters.ts`.

use qa_core::math::vec3;

// Intra-group imports: sibling modules split from the same flat port.
use crate::q3::presentation::mirrors_present_hud::*;

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
        let mut entity = create_model_entity(request.model.clone());
        let length = 0.5 * (bounds.max.z - bounds.min.z);
        entity.origin = vec3(
            length / 0.268,
            0.5 * (bounds.min.y + bounds.max.y),
            -0.5 * (bounds.min.z + bounds.max.z),
        );
        entity.lighting_origin = entity.origin;
        entity.old_origin = entity.origin;
        entity.axis = qvm_angles_to_axis(vec3(0.0, request.angle, 0.0));
        entity.render_flags = RF_LIGHTING_ORIGIN | RF_NOSHADOW;
        self.resources.borrow_mut().clear_scene();
        self.resources.borrow_mut().add_ref_entity(&entity);
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
