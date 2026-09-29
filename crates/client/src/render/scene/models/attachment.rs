//! Model attachment alignment (donor
//! `src/render/scene/models/attachment.ts`).

use qa_core::math::{scale3, vec3};

use crate::render::error::RenderError;

use super::transform::{compose_model_transform, model_local_delta};
use super::types::EntityTransform;

/// Register a model-space grip against a destination socket without changing
/// its size: invert the source transform, then compose with the destination.
pub fn align_model_attachment(
    source: &EntityTransform,
    destination: &EntityTransform,
) -> Result<EntityTransform, RenderError> {
    let inverse = EntityTransform {
        origin: model_local_delta(source, scale3(source.origin, -1.0))?,
        axis: [
            model_local_delta(source, vec3(1.0, 0.0, 0.0))?,
            model_local_delta(source, vec3(0.0, 1.0, 0.0))?,
            model_local_delta(source, vec3(0.0, 0.0, 1.0))?,
        ],
        scale: vec3(1.0, 1.0, 1.0),
    };
    Ok(compose_model_transform(destination, &inverse))
}

#[cfg(test)]
mod tests {
    use super::super::transform::model_world_point;
    use super::*;
    use qa_core::math::vec3;

    #[test]
    fn alignment_registers_source_at_destination() {
        let source = EntityTransform {
            origin: vec3(1.0, 2.0, 3.0),
            ..EntityTransform::identity()
        };
        let destination = EntityTransform {
            origin: vec3(10.0, 0.0, 0.0),
            ..EntityTransform::identity()
        };
        let aligned = align_model_attachment(&source, &destination).expect("align");
        let moved = model_world_point(&aligned, source.origin);
        assert!((moved.x - 10.0).abs() < 1e-5);
        assert!(moved.y.abs() < 1e-5);
        assert!(moved.z.abs() < 1e-5);
    }

    #[test]
    fn identity_alignment_is_inverse() {
        let source = EntityTransform {
            origin: vec3(1.0, 0.0, 0.0),
            ..EntityTransform::identity()
        };
        let aligned = align_model_attachment(&source, &EntityTransform::identity()).expect("align");
        let moved = model_world_point(&aligned, vec3(1.0, 0.0, 0.0));
        assert!(moved.x.abs() < 1e-5);
        assert!(moved.y.abs() < 1e-5);
        assert!(moved.z.abs() < 1e-5);
    }

    #[test]
    fn singular_source_errors() {
        let source = EntityTransform {
            origin: vec3(0.0, 0.0, 0.0),
            axis: EntityTransform::identity().axis,
            scale: vec3(0.0, 1.0, 1.0),
        };
        assert!(align_model_attachment(&source, &EntityTransform::identity()).is_err());
    }
}
