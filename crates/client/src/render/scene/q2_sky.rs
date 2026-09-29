//! Quake 2 skybox sides (`gl_warp.c`).
//!
//! Donor provenance: `src/render/scene/q2-sky.ts` (`q2SkySides` with
//! `ClipSkyPolygon`, `MakeSkyVec`, `R_DrawSkyBox`). Sky polygons are
//! clipped against the six sky planes, projected to cube faces, and
//! emitted as one operation per visible face. GPL-2.0-or-later.

use qa_core::math::{add3, dot3, normalize3_or_zero, rotate_point_around_vector, scale3, sub3, vec3, Vec2, Vec3, Vec4};

use crate::materials::geometry::MaterialGeometry;
use crate::materials::sky::sky_vector;
use crate::render::error::RenderError;
use crate::render::types::{RenderOperation, RendererImage, SkyVertex};

/// Q2 sky view: six face images plus rotation state.
#[derive(Debug, Clone, PartialEq)]
pub struct Q2SkyView {
    /// Faces in `SKY_FACE_SUFFIXES` order: rt, lf, bk, ft, up, dn.
    pub images: Vec<RendererImage>,
    /// Rotation angle in degrees, or degrees per second when auto-rotating.
    pub rotation: f32,
    /// Whether rotation advances with time.
    pub auto_rotate: bool,
    /// Rotation axis.
    pub axis: Vec3,
}

#[derive(Debug, Clone, Copy)]
struct FaceBounds {
    min_s: f32,
    min_t: f32,
    max_s: f32,
    max_t: f32,
}

impl FaceBounds {
    fn empty() -> Self {
        Self {
            min_s: 9999.0,
            min_t: 9999.0,
            max_s: -9999.0,
            max_t: -9999.0,
        }
    }

    fn visible(&self) -> bool {
        self.min_s < self.max_s && self.min_t < self.max_t
    }
}

const PLANES: [Vec3; 6] = [
    Vec3 { x: 1.0, y: 1.0, z: 0.0 },
    Vec3 {
        x: 1.0,
        y: -1.0,
        z: 0.0,
    },
    Vec3 {
        x: 0.0,
        y: -1.0,
        z: 1.0,
    },
    Vec3 { x: 0.0, y: 1.0, z: 1.0 },
    Vec3 { x: 1.0, y: 0.0, z: 1.0 },
    Vec3 {
        x: -1.0,
        y: 0.0,
        z: 1.0,
    },
];

fn project_polygon(points: &[Vec3], bounds: &mut [FaceBounds]) {
    let mut sum = vec3(0.0, 0.0, 0.0);
    for point in points {
        sum = add3(sum, *point);
    }
    let (x, y, z) = (sum.x.abs(), sum.y.abs(), sum.z.abs());
    let face = if x > y && x > z {
        if sum.x < 0.0 {
            1
        } else {
            0
        }
    } else if y > z && y > x {
        if sum.y < 0.0 {
            3
        } else {
            2
        }
    } else if sum.z < 0.0 {
        5
    } else {
        4
    };
    let range = &mut bounds[face];
    for point in points {
        let (divisor, s, t) = match face {
            0 => (point.x, -point.y, point.z),
            1 => (-point.x, point.y, point.z),
            2 => (point.y, point.x, point.z),
            3 => (-point.y, -point.x, point.z),
            4 => (point.z, -point.y, -point.x),
            _ => (-point.z, -point.y, point.x),
        };
        if divisor < 0.001 {
            continue;
        }
        let s = s / divisor;
        let t = t / divisor;
        range.min_s = range.min_s.min(s);
        range.min_t = range.min_t.min(t);
        range.max_s = range.max_s.max(s);
        range.max_t = range.max_t.max(t);
    }
}

fn clip_polygon(points: Vec<Vec3>, stage: usize, bounds: &mut [FaceBounds]) -> Result<(), RenderError> {
    if points.len() > 62 {
        return Err(RenderError::Backend(
            "Q2 sky polygon exceeds MAX_CLIP_VERTS".to_string(),
        ));
    }
    if stage == 6 {
        project_polygon(&points, bounds);
        return Ok(());
    }
    let normal = PLANES[stage];
    let distances: Vec<f32> = points.iter().map(|point| dot3(*point, normal)).collect();
    let sides: Vec<i32> = distances
        .iter()
        .map(|distance| {
            if *distance > 0.1 {
                1
            } else if *distance < -0.1 {
                -1
            } else {
                0
            }
        })
        .collect();
    if !sides.contains(&1) || !sides.contains(&-1) {
        return clip_polygon(points, stage + 1, bounds);
    }
    let mut front = Vec::new();
    let mut back = Vec::new();
    for (index, point) in points.iter().enumerate() {
        let next = (index + 1) % points.len();
        let side = sides[index];
        let next_side = sides[next];
        if side >= 0 {
            front.push(*point);
        }
        if side <= 0 {
            back.push(*point);
        }
        if side == 0 || next_side == 0 || side == next_side {
            continue;
        }
        let fraction = distances[index] / (distances[index] - distances[next]);
        let intersection = add3(*point, scale3(sub3(points[next], *point), fraction));
        front.push(intersection);
        back.push(intersection);
    }
    clip_polygon(front, stage + 1, bounds)?;
    clip_polygon(back, stage + 1, bounds)
}

/// Clip sky geometry against the sky planes and emit one operation per
/// visible cube face.
pub fn q2_sky_sides(
    geometry: &MaterialGeometry,
    origin: Vec3,
    sky: &Q2SkyView,
    seconds: f32,
    project: &dyn Fn(Vec3) -> Vec4,
) -> Result<Vec<RenderOperation>, RenderError> {
    if sky.images.len() != 6 {
        return Err(RenderError::Backend(
            "Q2 sky requires six registered images".to_string(),
        ));
    }
    let mut bounds = vec![FaceBounds::empty(); 6];
    for base in (0..geometry.indices.len()).step_by(3) {
        let mut points = Vec::with_capacity(3);
        for offset in 0..3 {
            let vertex_index = geometry
                .indices
                .get(base + offset)
                .copied()
                .ok_or(RenderError::BadBatch {
                    index: base + offset,
                    detail: format!("Q2 sky index {} outside {}", base + offset, geometry.indices.len()),
                })? as usize;
            let vertex = geometry.vertices.get(vertex_index).ok_or(RenderError::BadBatch {
                index: vertex_index,
                detail: format!("Q2 sky vertex {vertex_index} outside {}", geometry.vertices.len()),
            })?;
            points.push(sub3(vertex.position, origin));
        }
        clip_polygon(points, 0, &mut bounds)?;
    }
    if !bounds.iter().any(FaceBounds::visible) {
        return Ok(Vec::new());
    }
    let angle = if sky.auto_rotate {
        seconds * sky.rotation
    } else {
        sky.rotation
    };
    let axis = normalize3_or_zero(sky.axis);
    let rotate = angle != 0.0 && dot3(axis, axis) != 0.0;
    let seam: f32 = if sky.rotation != 0.0 { 1.0 / 256.0 } else { 1.0 / 512.0 };
    let coordinate = |value: f32| seam.max((1.0 - seam).min((value + 1.0) * 0.5));
    let mut operations = Vec::new();
    for (face, clipped) in bounds.iter().enumerate() {
        let range = if sky.rotation != 0.0 {
            FaceBounds {
                min_s: -1.0,
                min_t: -1.0,
                max_s: 1.0,
                max_t: 1.0,
            }
        } else {
            *clipped
        };
        if !range.visible() {
            continue;
        }
        let mut strip = Vec::with_capacity(4);
        for (s, t) in [
            (range.min_s, range.min_t),
            (range.min_s, range.max_t),
            (range.max_s, range.min_t),
            (range.max_s, range.max_t),
        ] {
            let point = sky_vector(face, s, t, 2300.0).map_err(|error| RenderError::Backend(error.to_string()))?;
            let direction = if rotate {
                rotate_point_around_vector(axis, point, f64::from(angle))
            } else {
                point
            };
            strip.push(SkyVertex {
                position: project(add3(origin, direction)),
                tex_coord: Vec2 {
                    x: coordinate(s),
                    y: 1.0 - coordinate(t),
                },
            });
        }
        operations.push(RenderOperation::SkySide {
            image: sky.images[face].clone(),
            color: Vec4 {
                x: 1.0,
                y: 1.0,
                z: 1.0,
                w: 1.0,
            },
            strips: vec![strip],
        });
    }
    Ok(operations)
}

#[cfg(test)]
mod tests {
    use qa_core::identity::IdentityOwner;
    use qa_core::math::{vec2, vec3, vec4};

    use crate::materials::geometry::MaterialVertex;
    use crate::render::types::{fresh_owner_identity, ImageSource, ResourceOwner};

    use super::*;

    fn test_image(ordinal: u32) -> RendererImage {
        let authority = IdentityOwner::create("q2-sky-test").unwrap();
        RendererImage {
            owner: ResourceOwner::new(fresh_owner_identity(), authority.session().clone(), 0),
            ordinal,
            source: ImageSource::Generated {
                name: format!("sky{ordinal}"),
            },
            width: 256,
            height: 256,
        }
    }

    fn test_sky() -> Q2SkyView {
        Q2SkyView {
            images: (0..6).map(test_image).collect(),
            rotation: 0.0,
            auto_rotate: false,
            axis: vec3(0.0, 0.0, 1.0),
        }
    }

    fn vertex(position: Vec3) -> MaterialVertex {
        MaterialVertex::new(
            position,
            vec3(0.0, 0.0, 1.0),
            vec2(0.0, 0.0),
            vec2(0.0, 0.0),
            [255, 255, 255, 255],
        )
    }

    #[test]
    fn wrong_image_count_errors() {
        let geometry = MaterialGeometry::empty();
        let project = |point: Vec3| vec4(point.x, point.y, point.z, 1.0);
        for count in [0, 5, 7] {
            let sky = Q2SkyView {
                images: (0..count).map(test_image).collect(),
                ..test_sky()
            };
            assert!(matches!(
                q2_sky_sides(&geometry, vec3(0.0, 0.0, 0.0), &sky, 0.0, &project),
                Err(RenderError::Backend(_))
            ));
        }
    }

    #[test]
    fn empty_geometry_produces_no_operations() {
        let project = |point: Vec3| vec4(point.x, point.y, point.z, 1.0);
        let operations = q2_sky_sides(
            &MaterialGeometry::empty(),
            vec3(0.0, 0.0, 0.0),
            &test_sky(),
            0.0,
            &project,
        )
        .unwrap();
        assert!(operations.is_empty());
    }

    #[test]
    fn single_triangle_produces_sky_side_operations() {
        let geometry = MaterialGeometry {
            vertices: vec![
                vertex(vec3(100.0, -10.0, 10.0)),
                vertex(vec3(100.0, 10.0, 10.0)),
                vertex(vec3(100.0, 0.0, -10.0)),
            ],
            indices: vec![0, 1, 2],
        };
        let project = |point: Vec3| vec4(point.x, point.y, point.z, 1.0);
        let operations = q2_sky_sides(&geometry, vec3(0.0, 0.0, 0.0), &test_sky(), 0.0, &project).unwrap();
        assert!(!operations.is_empty());
        for operation in &operations {
            let RenderOperation::SkySide { strips, color, .. } = operation else {
                panic!("expected sky-side operations, got {operation:?}");
            };
            assert_eq!(strips.len(), 1);
            assert_eq!(strips[0].len(), 4);
            assert_eq!(
                *color,
                Vec4 {
                    x: 1.0,
                    y: 1.0,
                    z: 1.0,
                    w: 1.0
                }
            );
        }
    }

    #[test]
    fn oversized_polygon_errors() {
        assert!(matches!(
            clip_polygon(vec![vec3(0.0, 0.0, 0.0); 63], 0, &mut [FaceBounds::empty(); 6]),
            Err(RenderError::Backend(_))
        ));
    }

    #[test]
    fn bad_geometry_index_errors() {
        let geometry = MaterialGeometry {
            vertices: vec![vertex(vec3(100.0, 0.0, 0.0))],
            indices: vec![0, 1, 2],
        };
        let project = |point: Vec3| vec4(point.x, point.y, point.z, 1.0);
        assert!(matches!(
            q2_sky_sides(&geometry, vec3(0.0, 0.0, 0.0), &test_sky(), 0.0, &project),
            Err(RenderError::BadBatch { .. })
        ));
    }
}
