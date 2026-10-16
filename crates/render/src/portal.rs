//! One Q3 portal camera transform over the shared scene records.
use crate::{
    assets::Assets,
    scene::{CommandList, DrawItem, DrawKind, Refdef, SceneEntity, SceneModel, SceneRanges},
    shader::AlphaGen,
};
use qa_core::{
    math,
    primitives::{Plane, Vec3},
};

/// The matching plane retains Q3's untranslated normal for inline models;
/// the surface plane is their actual world orientation.
pub fn camera(
    surface: Plane,
    matching: Plane,
    entities: &[SceneEntity],
    view: Refdef,
) -> Option<Refdef> {
    if view.portal || view.hyperspace || !surface.is_valid() || !matching.is_valid() {
        return None;
    }
    let entity = entities.iter().find(|entity| {
        matches!(entity.model, SceneModel::Portal { .. })
            && matching.signed_distance(entity.origin).abs() <= 64.
    })?;
    let SceneModel::Portal {
        camera: destination,
        rotation,
    } = entity.model
    else {
        return None;
    };
    let side = math::perpendicular_vector(surface.normal);
    let surface_axes = [surface.normal, side, math::cross(surface.normal, side)];
    let reflected = entity.origin == destination;
    let (surface_origin, camera_origin, camera_axes) = if reflected {
        let origin = surface.normal * surface.distance;
        (
            origin,
            origin,
            [-surface_axes[0], surface_axes[1], surface_axes[2]],
        )
    } else {
        let origin = entity.origin - surface.normal * surface.signed_distance(entity.origin);
        let forward = -entity.axes[0];
        let left = -entity.axes[1];
        let angle = if entity.old_frame != 0 {
            Some(if entity.frame != 0 {
                (view.time_ms as f32 / 1000.) * entity.frame as i32 as f32
            } else {
                let wave = f64::from(view.time_ms as f32 * 0.003).sin() as f32;
                rotation as f32 + wave * 4.
            })
        } else {
            (rotation != 0).then_some(rotation as f32)
        };
        let axes = if let Some(angle) = angle {
            let left = math::rotate_about_axis(forward, left, angle);
            [forward, left, math::cross(forward, left)]
        } else {
            [forward, left, entity.axes[2]]
        };
        (origin, destination, axes)
    };
    let transform = |vector: Vec3| {
        let mut result = Vec3::default();
        for axis in 0..3 {
            result = result + camera_axes[axis] * vector.dot(surface_axes[axis]);
        }
        result
    };
    let normal = -camera_axes[0];
    Some(Refdef {
        origin: transform(view.origin - surface_origin) + camera_origin,
        axes: view.axes.map(transform),
        portal: true,
        reflected_view: reflected,
        pvs_origin: Some(destination),
        clip_plane: Some(Plane::oriented(normal, camera_origin.dot(normal))),
        blend: [0.; 4],
        palette_transform: None,
        ..view
    })
}

fn planes(list: &CommandList, assets: &Assets, item: DrawItem) -> Option<(Plane, Plane)> {
    let (original, entity) = match item.kind {
        DrawKind::Surface => {
            let reference = list.surface(item.index);
            let geometry = assets.world(reference.world)?.geometry();
            let surface = geometry.surfaces.get(reference.surface as usize)?;
            let plane = if let Some(plane) = surface.plane {
                plane
            } else if surface.kind == crate::world::geometry::GeometryKind::Triangles {
                let indices = &geometry.indices[surface.indices.range()];
                let [a, b, c, ..] = indices else {
                    return None;
                };
                Plane::from_points(
                    geometry.vertices[*a as usize].vertex.position,
                    geometry.vertices[*b as usize].vertex.position,
                    geometry.vertices[*c as usize].vertex.position,
                )?
            } else {
                Plane::oriented(Vec3([1., 0., 0.]), 0.)
            };
            (plane, reference.entity.map(|index| list.entity(index)))
        }
        DrawKind::Poly => {
            let vertices = list.vertices(list.poly(item.index).vertices);
            let [a, b, c, ..] = vertices else {
                return None;
            };
            (
                Plane::from_points(a.position, b.position, c.position)?,
                None,
            )
        }
        DrawKind::Entity => (
            Plane::oriented(Vec3([1., 0., 0.]), 0.),
            Some(list.entity(item.index)),
        ),
    };
    Some(if let Some(entity) = entity {
        let normal = math::transform_point(Vec3::default(), entity.axes, original.normal);
        (
            Plane::oriented(normal, original.distance + normal.dot(entity.origin)),
            Plane::oriented(
                original.normal,
                original.distance + original.normal.dot(entity.origin),
            ),
        )
    } else {
        (original, original)
    })
}

pub(crate) fn first_child(
    list: &CommandList,
    source: SceneRanges,
    assets: &Assets,
    view: Refdef,
    native: &dyn crate::shader::reference::NativeShaders,
) -> Option<Refdef> {
    if view.portal || view.hyperspace {
        return None;
    }
    let frustum = crate::world::frustum(view).ok()?;
    for &item in list.draws(source.draws) {
        let (material, _) = crate::scene::source_material_lightmap(assets, item, native)?;
        let material = assets.material(material)?;
        if material.settings.sort > 1. {
            break;
        }
        if material.settings.sort != 1. {
            continue;
        }
        let Some((surface, matching)) = planes(list, assets, item) else {
            continue;
        };
        let Some(child) = camera(surface, matching, list.entities(source.entities), view) else {
            continue;
        };
        let range = material
            .stages
            .iter()
            .rev()
            .find_map(|stage| match stage.alpha_gen {
                AlphaGen::Portal(range) => Some(range),
                _ => None,
            })
            .unwrap_or(256.);
        let mut outside = 0x3f_u8;
        let mut front = 0usize;
        let mut nearest = 100_000_000_f32;
        let valid = crate::scene::for_each_triangle(list, item, assets, |vertices| {
            for vertex in vertices {
                let mut flags = 0u8;
                for (axis, plane) in frustum.iter().enumerate() {
                    if plane.signed_distance(vertex.position) <= 0. {
                        flags |= 1 << axis;
                    }
                }
                outside &= flags;
            }
            let relative = vertices[0].position - view.origin;
            nearest = nearest.min(relative.dot(relative));
            let normal = if item.kind == DrawKind::Poly {
                surface.normal
            } else {
                vertices[0].normal
            };
            front += usize::from(relative.dot(normal) < 0.);
        });
        if valid && outside == 0 && front != 0 && (child.reflected_view || nearest <= range * range)
        {
            return Some(child);
        }
    }
    None
}
