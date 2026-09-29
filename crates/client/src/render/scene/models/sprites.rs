//! Sprite orientation and quads (donor
//! `src/render/scene/models/sprites.ts`).
//!
//! Q1 `r_sprite.c` and Q2 `gl_rmain.c` sprite placement.

use qa_content::spr::{Sp2Frame, SprModel, SpriteFrame, SpriteOrientation};
use qa_core::math::{add3, dot3, normalize3, scale3, sub3, vec2, vec3, Axis, Vec3, Vec4};

use crate::materials::geometry::{MaterialGeometry, MaterialVertex};
use crate::view::{CameraClip, SceneCamera};

use super::types::EntityTransform;

/// Camera-facing sprite basis for a Q1 sprite, or `None` when the poster is
/// edge-on to the view.
#[must_use]
pub fn q1_sprite_axes(model: &SprModel, transform: &EntityTransform, camera: &SceneCamera, roll: f32) -> Option<Axis> {
    let right = scale3(camera.axis[1], -1.0);
    let up = camera.axis[2];
    match model.orientation {
        SpriteOrientation::ParallelUpright => {
            let direction = normalize3(sub3(transform.origin, camera.origin));
            if direction.z.abs() > 0.999_848 {
                return None;
            }
            let r = normalize3(vec3(direction.y, -direction.x, 0.0));
            Some([vec3(-r.y, r.x, 0.0), scale3(r, -1.0), vec3(0.0, 0.0, 1.0)])
        }
        SpriteOrientation::FacingUpright => {
            let direction = camera.axis[0];
            if direction.z.abs() > 0.999_848 {
                return None;
            }
            let r = normalize3(vec3(direction.y, -direction.x, 0.0));
            Some([vec3(-r.y, r.x, 0.0), scale3(r, -1.0), vec3(0.0, 0.0, 1.0)])
        }
        SpriteOrientation::Parallel => Some(camera.axis),
        SpriteOrientation::Oriented => Some(transform.axis),
        SpriteOrientation::ParallelOriented => {
            let radians = roll * std::f32::consts::PI / 180.0;
            let sine = radians.sin();
            let cosine = radians.cos();
            Some([
                camera.axis[0],
                scale3(add3(scale3(right, cosine), scale3(up, sine)), -1.0),
                add3(scale3(right, -sine), scale3(up, cosine)),
            ])
        }
    }
}

/// Sprite extents in model units.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SpriteExtents {
    /// Left edge.
    pub left: f32,
    /// Right edge.
    pub right: f32,
    /// Top edge.
    pub top: f32,
    /// Bottom edge.
    pub bottom: f32,
}

/// Build a camera-facing sprite quad in world space.
#[must_use]
pub fn sprite_quad(
    origin: Vec3,
    axis: &Axis,
    extents: SpriteExtents,
    color: [u8; 4],
    mirror: bool,
) -> MaterialGeometry {
    let right = scale3(axis[1], if mirror { 1.0 } else { -1.0 });
    let up = axis[2];
    let normal = scale3(axis[0], -1.0);
    let vertex = |x: f32, y: f32, s: f32, t: f32| {
        MaterialVertex::new(
            add3(add3(origin, scale3(right, x)), scale3(up, y)),
            normal,
            vec2(s, t),
            vec2(s, t),
            color,
        )
    };
    MaterialGeometry {
        vertices: vec![
            vertex(extents.left, extents.top, 0.0, 0.0),
            vertex(extents.right, extents.top, 1.0, 0.0),
            vertex(extents.right, extents.bottom, 1.0, 1.0),
            vertex(extents.left, extents.bottom, 0.0, 1.0),
        ],
        indices: vec![0, 1, 3, 3, 1, 2],
    }
}

/// Q1 sprite geometry: poster displaced along its forward axis by beam length,
/// culled when behind the camera or edge-on.
#[must_use]
pub fn q1_sprite_geometry(
    model: &SprModel,
    frame: &SpriteFrame,
    transform: &EntityTransform,
    camera: &SceneCamera,
    color: [u8; 4],
    roll: f32,
) -> MaterialGeometry {
    let Some(axis) = q1_sprite_axes(model, transform, camera, roll) else {
        return MaterialGeometry::empty();
    };
    let origin = add3(transform.origin, scale3(axis[0], -model.beam_length));
    if dot3(axis[0], sub3(camera.origin, origin)) >= 0.0 {
        return MaterialGeometry::empty();
    }
    let mirror = matches!(camera.clip, CameraClip::Portal { mirror: true, .. });
    sprite_quad(
        origin,
        &axis,
        SpriteExtents {
            left: frame.origin_x as f32 * transform.scale.x,
            right: (frame.origin_x + frame.width) as f32 * transform.scale.x,
            top: frame.origin_y as f32 * transform.scale.z,
            bottom: (frame.origin_y - frame.height) as f32 * transform.scale.z,
        },
        color,
        mirror,
    )
}

/// Q2 SP2 sprite geometry: always parallel to the camera axes.
#[must_use]
pub fn q2_sprite_geometry(
    frame: &Sp2Frame,
    transform: &EntityTransform,
    camera: &SceneCamera,
    color: [u8; 4],
) -> MaterialGeometry {
    let mirror = matches!(camera.clip, CameraClip::Portal { mirror: true, .. });
    sprite_quad(
        transform.origin,
        &camera.axis,
        SpriteExtents {
            left: -(frame.origin_x as f32) * transform.scale.x,
            right: (frame.width - frame.origin_x) as f32 * transform.scale.x,
            top: (frame.height - frame.origin_y) as f32 * transform.scale.z,
            bottom: -(frame.origin_y as f32) * transform.scale.z,
        },
        color,
        mirror,
    )
}

/// Whether a color carries translucency (helper for sprite batches).
#[must_use]
pub fn sprite_color_is_translucent(color: Vec4) -> bool {
    color.w < 1.0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::view::{Rect, SceneCamera};
    use qa_core::math::{vec3, vec4};

    fn camera() -> SceneCamera {
        SceneCamera {
            origin: vec3(0.0, -10.0, 0.0),
            axis: [vec3(0.0, 1.0, 0.0), vec3(-1.0, 0.0, 0.0), vec3(0.0, 0.0, 1.0)],
            projection: [0.0; 16],
            viewport: Rect {
                x: 0,
                y: 0,
                width: 640,
                height: 480,
            },
            clip: CameraClip::None,
        }
    }

    fn sprite_model(orientation: SpriteOrientation) -> SprModel {
        SprModel {
            orientation,
            bounding_radius: 10.0,
            max_width: 64,
            max_height: 64,
            beam_length: 0.0,
            sync: qa_content::common::SyncType::Synchronized,
            frames: Vec::new(),
            bounds: qa_content::common::Bounds {
                min: [0.0; 3],
                max: [0.0; 3],
            },
        }
    }

    #[test]
    fn parallel_sprite_uses_camera_axes() {
        let model = sprite_model(SpriteOrientation::Parallel);
        let camera = camera();
        let axes = q1_sprite_axes(&model, &EntityTransform::identity(), &camera, 0.0).expect("axes");
        assert_eq!(axes, camera.axis);
    }

    #[test]
    fn oriented_sprite_uses_model_axes() {
        let model = sprite_model(SpriteOrientation::Oriented);
        let transform = EntityTransform::identity();
        let axes = q1_sprite_axes(&model, &transform, &camera(), 0.0).expect("axes");
        assert_eq!(axes, transform.axis);
    }

    #[test]
    fn upright_sprite_is_vertical() {
        let model = sprite_model(SpriteOrientation::ParallelUpright);
        let axes = q1_sprite_axes(&model, &EntityTransform::identity(), &camera(), 0.0).expect("axes");
        assert_eq!(axes[2], vec3(0.0, 0.0, 1.0));
    }

    #[test]
    fn edge_on_sprite_returns_none() {
        let model = sprite_model(SpriteOrientation::FacingUpright);
        let camera = SceneCamera {
            origin: vec3(0.0, 0.0, -10.0),
            axis: [vec3(0.0, 0.0, 1.0), vec3(-1.0, 0.0, 0.0), vec3(0.0, 1.0, 0.0)],
            projection: [0.0; 16],
            viewport: Rect {
                x: 0,
                y: 0,
                width: 640,
                height: 480,
            },
            clip: CameraClip::None,
        };
        assert!(q1_sprite_axes(&model, &EntityTransform::identity(), &camera, 0.0).is_none());
    }

    #[test]
    fn sprite_quad_has_four_vertices_and_two_triangles() {
        let quad = sprite_quad(
            vec3(0.0, 0.0, 0.0),
            &camera().axis,
            SpriteExtents {
                left: -1.0,
                right: 1.0,
                top: 1.0,
                bottom: -1.0,
            },
            [255, 255, 255, 255],
            false,
        );
        assert_eq!(quad.vertices.len(), 4);
        assert_eq!(quad.indices, vec![0, 1, 3, 3, 1, 2]);
    }

    #[test]
    fn q1_sprite_geometry_matches_frame_extents() {
        let model = sprite_model(SpriteOrientation::Parallel);
        let frame = SpriteFrame {
            origin_x: -32,
            origin_y: 32,
            width: 64,
            height: 64,
            pixels: Vec::new(),
        };
        let geometry = q1_sprite_geometry(
            &model,
            &frame,
            &EntityTransform::identity(),
            &camera(),
            [255, 255, 255, 255],
            0.0,
        );
        assert_eq!(geometry.vertices.len(), 4);
        assert_eq!(geometry.indices.len(), 6);
    }

    #[test]
    fn sprite_behind_camera_is_culled() {
        let model = sprite_model(SpriteOrientation::Parallel);
        let frame = SpriteFrame {
            origin_x: 0,
            origin_y: 0,
            width: 8,
            height: 8,
            pixels: Vec::new(),
        };
        let camera = SceneCamera {
            origin: vec3(0.0, 10.0, 0.0),
            ..camera()
        };
        let geometry = q1_sprite_geometry(
            &model,
            &frame,
            &EntityTransform::identity(),
            &camera,
            [255, 255, 255, 255],
            0.0,
        );
        assert!(geometry.vertices.is_empty());
    }

    #[test]
    fn translucency_follows_alpha() {
        assert!(sprite_color_is_translucent(vec4(1.0, 1.0, 1.0, 0.5)));
        assert!(!sprite_color_is_translucent(vec4(1.0, 1.0, 1.0, 1.0)));
    }
}
