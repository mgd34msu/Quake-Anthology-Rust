//! Q3 portal cameras and portal-surface offscreen tests.
//!
//! Donor provenance: `src/render/scene/portal.ts` (`R_GetPortalOrientations`
//! and `SurfIsOffscreen` from `tr_main.c`).

use qa_core::math::{
    add3, cross3, dot3, normalize3, perpendicular_vector, rotate_point_around_vector, scale3, sub3, Axis, Plane, Vec3,
};

use crate::materials::geometry::MaterialGeometry;
use crate::render::RenderError;
use crate::view::{project_point, world_point, world_vector, CameraClip, ModelTransform, SceneCamera};

/// Portal-marker entity: `origin` on the surface, `old_origin` at the
/// remote view (equal origins mark a mirror).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PortalEntity {
    /// Surface origin.
    pub origin: Vec3,
    /// Remote camera origin.
    pub old_origin: Vec3,
    /// Entity basis.
    pub axis: Axis,
    /// Rotation speed or fixed angle selector.
    pub frame: i32,
    /// Rotation mode selector.
    pub old_frame: i32,
    /// Fixed rotation angle.
    pub skin_num: i32,
}

/// Portal child view plus its PVS origin.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PortalCamera {
    /// Child camera.
    pub camera: SceneCamera,
    /// PVS sampling origin.
    pub pvs_origin: Vec3,
    /// Whether the portal mirrors.
    pub mirror: bool,
}

fn transform(vector: Vec3, surface: &Axis, camera: &Axis) -> Vec3 {
    let mut result = Vec3 { x: 0.0, y: 0.0, z: 0.0 };
    for index in 0..3 {
        result = add3(result, scale3(camera[index], dot3(vector, surface[index])));
    }
    result
}

fn client_error(error: crate::ClientError) -> RenderError {
    RenderError::Backend(error.to_string())
}

/// Build the portal child camera for the first marker entity within 64
/// units of the surface plane, including a translated inline plane.
pub fn portal_camera(
    original: &Plane,
    entities: &[PortalEntity],
    view: &SceneCamera,
    milliseconds: f32,
    model: Option<&ModelTransform>,
) -> Result<Option<PortalCamera>, RenderError> {
    let (normal, distance) = match model {
        None => (original.normal, original.distance),
        Some(model) => {
            let normal = normalize3(world_vector(original.normal, model).map_err(client_error)?);
            let on_plane = scale3(original.normal, original.distance);
            let distance = dot3(normal, world_point(on_plane, model).map_err(client_error)?);
            (normal, distance)
        }
    };
    let entity = entities
        .iter()
        .find(|candidate| (dot3(candidate.origin, normal) - distance).abs() <= 64.0);
    let Some(entity) = entity else { return Ok(None) };
    let side = perpendicular_vector(normal);
    let surface_axis: Axis = [normal, side, cross3(normal, side)];
    let mirror = entity.origin == entity.old_origin;
    let (surface_origin, camera_origin, camera_axis) = if mirror {
        let surface_origin = scale3(normal, distance);
        (
            surface_origin,
            surface_origin,
            [scale3(normal, -1.0), surface_axis[1], surface_axis[2]],
        )
    } else {
        let surface_origin = add3(entity.origin, scale3(normal, -(dot3(entity.origin, normal) - distance)));
        let forward = scale3(entity.axis[0], -1.0);
        let left = scale3(entity.axis[1], -1.0);
        let angle = if entity.old_frame != 0 {
            Some(if entity.frame != 0 {
                milliseconds / 1000.0 * entity.frame as f32
            } else {
                entity.skin_num as f32 + (milliseconds * 0.003).sin() * 4.0
            })
        } else if entity.skin_num != 0 {
            Some(entity.skin_num as f32)
        } else {
            None
        };
        let rotated = match angle {
            None => left,
            Some(angle) => rotate_point_around_vector(forward, left, angle as f64),
        };
        let up = match angle {
            None => entity.axis[2],
            Some(_) => cross3(forward, rotated),
        };
        (surface_origin, entity.old_origin, [forward, rotated, up])
    };
    let plane_normal = scale3(camera_axis[0], -1.0);
    Ok(Some(PortalCamera {
        camera: SceneCamera {
            origin: add3(
                transform(sub3(view.origin, surface_origin), &surface_axis, &camera_axis),
                camera_origin,
            ),
            axis: [
                transform(view.axis[0], &surface_axis, &camera_axis),
                transform(view.axis[1], &surface_axis, &camera_axis),
                transform(view.axis[2], &surface_axis, &camera_axis),
            ],
            projection: view.projection,
            viewport: view.viewport,
            clip: CameraClip::Portal {
                plane: Plane {
                    normal: plane_normal,
                    distance: dot3(camera_origin, plane_normal),
                },
                mirror,
            },
        },
        pvs_origin: entity.old_origin,
        mirror,
    }))
}

/// Whether a portal surface is fully offscreen, backfacing, or beyond range.
pub fn portal_surface_offscreen(
    mesh: &MaterialGeometry,
    camera: &SceneCamera,
    range: f32,
    mirror: bool,
) -> Result<bool, RenderError> {
    let mut point_and: u32 = u32::MAX;
    for vertex in &mesh.vertices {
        let clip = project_point(camera, None, vertex.position).map_err(client_error)?;
        let mut flags = 0;
        for (index, component) in [clip.x, clip.y, clip.z].iter().enumerate() {
            if *component >= clip.w {
                flags |= 1 << (index * 2);
            } else if *component <= -clip.w {
                flags |= 1 << (index * 2 + 1);
            }
        }
        point_and &= flags;
    }
    if point_and != 0 {
        return Ok(true);
    }
    let mut triangles = mesh.indices.len() / 3;
    let mut shortest = 100_000_000.0f32;
    for chunk in mesh.indices.as_chunks::<3>().0 {
        let vertex = mesh
            .vertices
            .get(chunk[0] as usize)
            .ok_or_else(|| RenderError::BadBatch {
                index: chunk[0] as usize,
                detail: "Portal triangle index is outside its vertices".to_string(),
            })?;
        let relative = sub3(vertex.position, camera.origin);
        shortest = shortest.min(dot3(relative, relative));
        if dot3(relative, vertex.normal) >= 0.0 {
            triangles -= 1;
        }
    }
    Ok(triangles == 0 || !mirror && shortest > range * range)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::materials::geometry::MaterialVertex;
    use crate::view::Rect;
    use qa_core::math::{vec2, vec3};

    fn camera() -> SceneCamera {
        SceneCamera {
            origin: vec3(0.0, 0.0, 0.0),
            axis: [vec3(1.0, 0.0, 0.0), vec3(0.0, 1.0, 0.0), vec3(0.0, 0.0, 1.0)],
            projection: [
                1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, -1.0, -1.0, 0.0, 0.0, -8.0, 0.0,
            ],
            viewport: Rect {
                x: 0,
                y: 0,
                width: 64,
                height: 64,
            },
            clip: CameraClip::None,
        }
    }

    fn vertex(position: Vec3, normal: Vec3) -> MaterialVertex {
        MaterialVertex::new(position, normal, vec2(0.0, 0.0), vec2(0.0, 0.0), [255, 255, 255, 255])
    }

    #[test]
    fn mirror_portal_reflects_camera() {
        let plane = Plane {
            normal: vec3(1.0, 0.0, 0.0),
            distance: 64.0,
        };
        let at = vec3(64.0, 0.0, 0.0);
        let entity = PortalEntity {
            origin: at,
            old_origin: at,
            axis: [vec3(1.0, 0.0, 0.0), vec3(0.0, 1.0, 0.0), vec3(0.0, 0.0, 1.0)],
            frame: 0,
            old_frame: 0,
            skin_num: 0,
        };
        let child = portal_camera(&plane, &[entity], &camera(), 0.0, None)
            .expect("portal")
            .expect("child");
        assert!(child.mirror);
        assert_eq!(child.pvs_origin, at);
        assert!(matches!(child.camera.clip, CameraClip::Portal { mirror: true, .. }));
    }

    #[test]
    fn remote_portal_uses_old_origin() {
        let plane = Plane {
            normal: vec3(0.0, 0.0, 1.0),
            distance: 0.0,
        };
        let entity = PortalEntity {
            origin: vec3(0.0, 0.0, 0.0),
            old_origin: vec3(100.0, 0.0, 50.0),
            axis: [vec3(1.0, 0.0, 0.0), vec3(0.0, 1.0, 0.0), vec3(0.0, 0.0, 1.0)],
            frame: 0,
            old_frame: 0,
            skin_num: 0,
        };
        let child = portal_camera(&plane, &[entity], &camera(), 1000.0, None)
            .expect("portal")
            .expect("child");
        assert!(!child.mirror);
        assert_eq!(child.pvs_origin, vec3(100.0, 0.0, 50.0));
    }

    #[test]
    fn distant_entities_select_none() {
        let plane = Plane {
            normal: vec3(1.0, 0.0, 0.0),
            distance: 0.0,
        };
        let entity = PortalEntity {
            origin: vec3(500.0, 0.0, 0.0),
            old_origin: vec3(500.0, 0.0, 0.0),
            axis: [vec3(1.0, 0.0, 0.0), vec3(0.0, 1.0, 0.0), vec3(0.0, 0.0, 1.0)],
            frame: 0,
            old_frame: 0,
            skin_num: 0,
        };
        assert!(portal_camera(&plane, &[entity], &camera(), 0.0, None)
            .expect("portal")
            .is_none());
        assert!(portal_camera(&plane, &[], &camera(), 0.0, None)
            .expect("portal")
            .is_none());
    }

    #[test]
    fn offscreen_behind_camera_and_bad_index() {
        let camera = camera();
        let behind = MaterialGeometry {
            vertices: vec![
                vertex(vec3(-10.0, -1.0, -1.0), vec3(-1.0, 0.0, 0.0)),
                vertex(vec3(-10.0, 1.0, -1.0), vec3(-1.0, 0.0, 0.0)),
                vertex(vec3(-10.0, 0.0, 1.0), vec3(-1.0, 0.0, 0.0)),
            ],
            indices: vec![0, 1, 2],
        };
        assert!(portal_surface_offscreen(&behind, &camera, 1000.0, false).expect("behind"));
        let bad = MaterialGeometry {
            vertices: vec![vertex(vec3(10.0, 0.0, 0.0), vec3(-1.0, 0.0, 0.0))],
            indices: vec![5, 0, 0],
        };
        assert!(portal_surface_offscreen(&bad, &camera, 1000.0, false).is_err());
    }

    #[test]
    fn facing_triangle_onscreen_within_range() {
        let camera = camera();
        let mesh = MaterialGeometry {
            vertices: vec![
                vertex(vec3(10.0, -1.0, -1.0), vec3(-1.0, 0.0, 0.0)),
                vertex(vec3(10.0, 1.0, -1.0), vec3(-1.0, 0.0, 0.0)),
                vertex(vec3(10.0, 0.0, 1.0), vec3(-1.0, 0.0, 0.0)),
            ],
            indices: vec![0, 1, 2],
        };
        assert!(!portal_surface_offscreen(&mesh, &camera, 1000.0, false).expect("onscreen"));
        assert!(portal_surface_offscreen(&mesh, &camera, 5.0, false).expect("range"));
        assert!(!portal_surface_offscreen(&mesh, &camera, 5.0, true).expect("mirror range"));
    }
}
