//! Scene view projector.
//!
//! Donor provenance: `src/render/scene/view.ts` (`createViewProjector`).
//! Projection, frustum, and model-transform math already live in
//! [`crate::view`]; this module keeps the camera-plus-model projector that
//! binds them for one scene view.

use qa_core::math::{Vec3, Vec4};

use crate::render::error::RenderError;
use crate::view::{ModelTransform, SceneCamera};

/// Projects points through one camera and optional model transform.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ViewProjector {
    camera: SceneCamera,
    model: Option<ModelTransform>,
}

impl ViewProjector {
    /// Bind a camera with an optional model transform.
    #[must_use]
    pub fn new(camera: SceneCamera, model: Option<ModelTransform>) -> Self {
        Self { camera, model }
    }

    /// Project one point to clip space.
    pub fn project(&self, point: Vec3) -> Result<Vec4, RenderError> {
        crate::view::project_point(&self.camera, self.model.as_ref(), point)
            .map_err(|error| RenderError::Backend(error.to_string()))
    }

    /// Bound camera.
    #[must_use]
    pub fn camera(&self) -> &SceneCamera {
        &self.camera
    }

    /// Bound model transform, if any.
    #[must_use]
    pub fn model(&self) -> Option<&ModelTransform> {
        self.model.as_ref()
    }
}

#[cfg(test)]
mod tests {
    use qa_core::math::vec3;

    use crate::view::CameraClip;

    use super::*;

    fn test_camera() -> SceneCamera {
        SceneCamera {
            origin: vec3(1.0, 2.0, 3.0),
            axis: [vec3(1.0, 0.0, 0.0), vec3(0.0, 1.0, 0.0), vec3(0.0, 0.0, 1.0)],
            projection: [
                1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
            ],
            viewport: crate::view::Rect {
                x: 0,
                y: 0,
                width: 640,
                height: 480,
            },
            clip: CameraClip::None,
        }
    }

    fn test_model(scale: f32) -> ModelTransform {
        ModelTransform {
            origin: vec3(4.0, 5.0, 6.0),
            axis: [vec3(1.0, 0.0, 0.0), vec3(0.0, 1.0, 0.0), vec3(0.0, 0.0, 1.0)],
            scale,
        }
    }

    #[test]
    fn project_matches_project_point_without_model() {
        let camera = test_camera();
        let projector = ViewProjector::new(camera, None);
        assert_eq!(projector.camera(), &camera);
        assert_eq!(projector.model(), None);
        let point = vec3(7.0, 8.0, 9.0);
        assert_eq!(
            projector.project(point).unwrap(),
            crate::view::project_point(&camera, None, point).unwrap()
        );
    }

    #[test]
    fn project_matches_project_point_with_model() {
        let camera = test_camera();
        let model = test_model(2.0);
        let projector = ViewProjector::new(camera, Some(model));
        assert_eq!(projector.model(), Some(&model));
        let point = vec3(0.5, -1.5, 2.5);
        assert_eq!(
            projector.project(point).unwrap(),
            crate::view::project_point(&camera, Some(&model), point).unwrap()
        );
    }

    #[test]
    fn zero_scale_model_errors() {
        let projector = ViewProjector::new(test_camera(), Some(test_model(0.0)));
        assert!(matches!(
            projector.project(vec3(0.0, 0.0, 0.0)),
            Err(RenderError::Backend(_))
        ));
        let infinite = ViewProjector::new(test_camera(), Some(test_model(f32::INFINITY)));
        assert!(matches!(
            infinite.project(vec3(0.0, 0.0, 0.0)),
            Err(RenderError::Backend(_))
        ));
    }
}
