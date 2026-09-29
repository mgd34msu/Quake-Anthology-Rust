//! Sky projection, cube subdivision, cloud coordinates (`tr_sky.c`).
//!
//! Donor provenance: `src/materials/sky.ts`.

use qa_core::math::{add3, dot3, normalize3, scale3, sub3, Vec2, Vec3};

use super::deform::DeformGeometry;
use super::geometry::MaterialVertex;
use crate::ClientError;

/// Sky face suffixes in face order (`SKY_FACE_SUFFIXES`, with the
/// source's `bk`/`lf` permutation).
pub const SKY_FACE_SUFFIXES: [&str; 6] = ["rt", "lf", "bk", "ft", "up", "dn"];

const CLIP_PLANES: [Vec3; 6] = [
    Vec3 { x: 1.0, y: 1.0, z: 0.0 },
    Vec3 { x: 1.0, y: -1.0, z: 0.0 },
    Vec3 { x: 0.0, y: -1.0, z: 1.0 },
    Vec3 { x: 0.0, y: 1.0, z: 1.0 },
    Vec3 { x: 1.0, y: 0.0, z: 1.0 },
    Vec3 { x: -1.0, y: 0.0, z: 1.0 },
];

/// A sky face (`SkyFace`).
#[derive(Debug, Clone, PartialEq)]
pub struct SkyFace {
    /// Face index.
    pub face: usize,
    /// Face geometry.
    pub geometry: DeformGeometry,
    /// Triangle strips.
    pub strips: Vec<Vec<u32>>,
}

/// Sky geometry (`SkyGeometry`).
#[derive(Debug, Clone, PartialEq)]
pub struct SkyGeometry {
    /// Box faces.
    pub sky_box: Vec<SkyFace>,
    /// Clouds.
    pub clouds: DeformGeometry,
}

#[derive(Debug, Clone)]
struct FaceBounds {
    min_s: f32,
    min_t: f32,
    max_s: f32,
    max_t: f32,
}

/// Sky direction vector (`skyVector`).
pub fn sky_vector(face: usize, s: f32, t: f32, size: f32) -> Result<Vec3, ClientError> {
    let horizontal = s * size;
    let vertical = t * size;
    match face {
        0 => Ok(Vec3 {
            x: size,
            y: -horizontal,
            z: vertical,
        }),
        1 => Ok(Vec3 {
            x: -size,
            y: horizontal,
            z: vertical,
        }),
        2 => Ok(Vec3 {
            x: horizontal,
            y: size,
            z: vertical,
        }),
        3 => Ok(Vec3 {
            x: -horizontal,
            y: -size,
            z: vertical,
        }),
        4 => Ok(Vec3 {
            x: -vertical,
            y: -horizontal,
            z: size,
        }),
        5 => Ok(Vec3 {
            x: vertical,
            y: -horizontal,
            z: -size,
        }),
        _ => Err(ClientError::BadMaterial(
            "sky face must be 0..5".to_string(),
        )),
    }
}

fn project_polygon(points: &[Vec3], bounds: &mut [FaceBounds]) {
    let mut sum = Vec3 {
        x: 0.0,
        y: 0.0,
        z: 0.0,
    };
    for point in points {
        sum = add3(sum, *point);
    }
    let (x, y, z) = (sum.x.abs(), sum.y.abs(), sum.z.abs());
    let face = if x > y && x > z {
        usize::from(sum.x >= 0.0)
    } else if y > z && y > x {
        if sum.y < 0.0 { 3 } else { 2 }
    } else if sum.z < 0.0 {
        5
    } else {
        4
    };
    // Note: face 0/1 selection above matches `sum.x < 0 ? 1 : 0`.
    let face = if x > y && x > z {
        if sum.x < 0.0 { 1 } else { 0 }
    } else {
        face
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

fn clip_polygon(
    points: Vec<Vec3>,
    stage: usize,
    bounds: &mut [FaceBounds],
) -> Result<(), ClientError> {
    if points.len() > 62 {
        return Err(ClientError::BadMaterial(
            "sky polygon exceeds source clip vertex limit".to_string(),
        ));
    }
    if stage == 6 {
        project_polygon(&points, bounds);
        return Ok(());
    }
    let normal = CLIP_PLANES[stage];
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
        if side == 0 || next_side == 0 || next_side == side {
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

/// Cloud texture coordinates (`cloudTexCoord`).
pub fn cloud_tex_coord(face: usize, s: f32, t: f32, height: f32) -> Result<Vec2, ClientError> {
    if !height.is_finite() {
        return Err(ClientError::BadMaterial(
            "sky cloud height must be finite float32".to_string(),
        ));
    }
    let direction = sky_vector(face, s, t, 1024.0 / 1.75)?;
    let radius = 4096.0f32;
    let xx = direction.x * direction.x;
    let yy = direction.y * direction.y;
    let zz = direction.z * direction.z;
    let hh = height * height;
    // R_InitSkyTexCoords evaluates this float sum before sqrt(double).
    let mut discriminant = zz * (radius * radius);
    discriminant += 2.0 * xx * radius * height;
    discriminant += xx * hh;
    discriminant += 2.0 * yy * radius * height;
    discriminant += yy * hh;
    discriminant += 2.0 * zz * radius * height;
    discriminant += zz * hh;
    let inverse = 1.0 / (2.0 * dot3(direction, direction));
    let p = inverse * (-2.0 * direction.z * radius + 2.0 * discriminant.sqrt());
    let intersection = normalize3(qa_core::math::vec3(
        direction.x * p,
        direction.y * p,
        direction.z * p + radius,
    ));
    // Q_acos does not clamp; negative heights can leave NaN coordinates.
    Ok(qa_core::math::vec2(
        intersection.x.acos(),
        intersection.y.acos(),
    ))
}

/// Renderer-global sky builder (`SkyBuilder`).
#[derive(Debug, Clone)]
pub struct SkyBuilder {
    cloud_coords: Vec<Vec<Vec2>>,
    bounds: Vec<FaceBounds>,
}

impl SkyBuilder {
    /// New builder.
    #[must_use]
    pub fn new() -> Self {
        Self {
            cloud_coords: vec![vec![qa_core::math::vec2(0.0, 0.0); 81]; 6],
            bounds: Vec::new(),
        }
    }

    /// Initialize cloud coordinates (`initializeCloudCoordinates`).
    pub fn initialize_cloud_coordinates(&mut self, height: f32) -> Result<(), ClientError> {
        if !height.is_finite() {
            return Err(ClientError::BadMaterial(
                "sky cloud height must be finite float32".to_string(),
            ));
        }
        let mut coords = Vec::with_capacity(6);
        for face in 0..6 {
            let mut plane = Vec::with_capacity(81);
            for index in 0..81 {
                plane.push(cloud_tex_coord(
                    face,
                    (index % 9) as f32 / 4.0 - 1.0,
                    (index / 9) as f32 / 4.0 - 1.0,
                    height,
                )?);
            }
            coords.push(plane);
        }
        self.cloud_coords = coords;
        Ok(())
    }

    /// Clip meshes against the sky (`clip`).
    pub fn clip(&mut self, meshes: &[DeformGeometry], origin: Vec3) -> Result<(), ClientError> {
        let mut bounds = vec![
            FaceBounds {
                min_s: 9999.0,
                min_t: 9999.0,
                max_s: -9999.0,
                max_t: -9999.0,
            };
            6
        ];
        for mesh in meshes {
            let mut index = 0usize;
            while index + 2 < mesh.indices.len() + 2 && index < mesh.indices.len() {
                if index + 2 >= mesh.indices.len() {
                    break;
                }
                let mut points = Vec::with_capacity(3);
                for offset in 0..3 {
                    let vertex_index = mesh.indices[index + offset] as usize;
                    let vertex = mesh.vertices.get(vertex_index).ok_or_else(|| {
                        ClientError::BadMaterial(format!(
                            "sky index {vertex_index} outside {}",
                            mesh.vertices.len()
                        ))
                    })?;
                    points.push(sub3(vertex.position, origin));
                }
                clip_polygon(points, 0, &mut bounds)?;
                index += 3;
            }
        }
        self.bounds = bounds;
        Ok(())
    }

    /// Build box and cloud geometry (`build`).
    pub fn build(&self, origin: Vec3, far: f32) -> Result<SkyGeometry, ClientError> {
        let mut sky_box = Vec::new();
        let mut cloud_vertices = Vec::new();
        let mut cloud_indices = Vec::new();
        for (face, face_bounds) in self.bounds.iter().enumerate() {
            let clamp = |value: i32| value.clamp(-4, 4);
            let raw_min_s = (face_bounds.min_s * 4.0).floor() as i32;
            let raw_min_t = (face_bounds.min_t * 4.0).floor() as i32;
            let raw_max_s = (face_bounds.max_s * 4.0).ceil() as i32;
            let raw_max_t = (face_bounds.max_t * 4.0).ceil() as i32;
            if raw_min_s >= raw_max_s || raw_min_t >= raw_max_t {
                continue;
            }
            let (min_s, min_t, max_s, max_t) = (
                clamp(raw_min_s),
                clamp(raw_min_t),
                clamp(raw_max_s),
                clamp(raw_max_t),
            );
            let mut vertices = Vec::new();
            let mut indices = Vec::new();
            let width = (max_s - min_s + 1) as usize;
            for t in min_t..=max_t {
                for s in min_s..=max_s {
                    let direction = sky_vector(face, s as f32 / 4.0, t as f32 / 4.0, far / 1.75)?;
                    let position = add3(direction, origin);
                    vertices.push(MaterialVertex {
                        position,
                        normal: normalize3(scale3(direction, -1.0)),
                        tex_coord: qa_core::math::vec2(
                            (s as f32 / 4.0 + 1.0) / 2.0,
                            1.0 - (t as f32 / 4.0 + 1.0) / 2.0,
                        ),
                        lightmap_coord: qa_core::math::vec2(0.0, 0.0),
                        color: [255, 255, 255, 255],
                    });
                }
            }
            let mut strips = Vec::new();
            for t in 0..(max_t - min_t) as usize {
                let mut strip = Vec::new();
                for s in 0..width {
                    strip.push((s + t * width) as u32);
                    strip.push((s + (t + 1) * width) as u32);
                }
                strips.push(strip);
                for s in 0..(max_s - min_s) as usize {
                    let index = (s + t * width) as u32;
                    indices.extend([
                        index,
                        index + width as u32,
                        index + 1,
                        index + width as u32,
                        index + width as u32 + 1,
                        index + 1,
                    ]);
                }
            }
            sky_box.push(SkyFace {
                face,
                geometry: DeformGeometry {
                    vertices: vertices.clone(),
                    indices: indices.clone(),
                },
                strips,
            });
            if face == 5 {
                continue;
            }
            let cloud_start = cloud_vertices.len() as u32;
            for t in min_t..=max_t {
                for s in min_s..=max_s {
                    let vertex = &vertices[(s - min_s) as usize + (t - min_t) as usize * width];
                    let coord = self.cloud_coords[face][(s + 4) as usize + (t + 4) as usize * 9];
                    cloud_vertices.push(MaterialVertex {
                        tex_coord: coord,
                        ..*vertex
                    });
                }
            }
            cloud_indices.extend(indices.iter().map(|index| cloud_start + index));
        }
        Ok(SkyGeometry {
            sky_box,
            clouds: DeformGeometry {
                vertices: cloud_vertices,
                indices: cloud_indices,
            },
        })
    }
}

impl Default for SkyBuilder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn face_suffixes_keep_source_permutation() {
        assert_eq!(SKY_FACE_SUFFIXES, ["rt", "lf", "bk", "ft", "up", "dn"]);
    }

    #[test]
    fn sky_vector_faces() {
        assert_eq!(
            sky_vector(0, 0.0, 0.0, 1.0).unwrap(),
            Vec3 {
                x: 1.0,
                y: 0.0,
                z: 0.0
            }
        );
        assert!(sky_vector(6, 0.0, 0.0, 1.0).is_err());
    }

    #[test]
    fn cloud_coords_are_finite() {
        let coord = cloud_tex_coord(0, 0.0, 0.0, 128.0).unwrap();
        assert!(coord.x.is_finite() && coord.y.is_finite());
        assert!(cloud_tex_coord(0, 0.0, 0.0, f32::INFINITY).is_err());
    }

    #[test]
    fn builder_clips_and_builds() {
        use qa_core::math::{vec2, vec3};
        let vertex = |position: Vec3| MaterialVertex::new(
            position,
            vec3(0.0, 0.0, 1.0),
            vec2(0.0, 0.0),
            vec2(0.0, 0.0),
            [255, 255, 255, 255],
        );
        let mesh = DeformGeometry {
            vertices: vec![
                vertex(vec3(100.0, -10.0, 10.0)),
                vertex(vec3(100.0, 10.0, 10.0)),
                vertex(vec3(100.0, 0.0, -10.0)),
            ],
            indices: vec![0, 1, 2],
        };
        let mut builder = SkyBuilder::new();
        builder.initialize_cloud_coordinates(128.0).unwrap();
        builder.clip(&[mesh], vec3(0.0, 0.0, 0.0)).unwrap();
        let geometry = builder.build(vec3(0.0, 0.0, 0.0), 4096.0).unwrap();
        assert!(!geometry.sky_box.is_empty());
    }
}
