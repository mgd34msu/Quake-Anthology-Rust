//! Convex BSP cell clipping in the id Software winding/brush representation.
//!
//! Donor provenance: `/home/buzzkill/Projects/quake-typescript/src/world/geometry/q1-solid/polyhedron.ts`.
//!
//! All computation is `f64` like the donor; use the `From` conversions at
//! the `f32` engine-math boundary. The donor memoizes cell bounds in a
//! `WeakMap`; cells here are plain values so bounds are recomputed per
//! split, which is behavior-identical.

use qa_core::math::{Bounds, Plane, Vec3};

/// Double-precision 3-vector for solid geometry.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct DVec3 {
    /// X component.
    pub x: f64,
    /// Y component.
    pub y: f64,
    /// Z component.
    pub z: f64,
}

/// Double-precision plane in Hessian form.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct DPlane {
    /// Normal.
    pub normal: DVec3,
    /// Distance from the origin along the normal.
    pub distance: f64,
}

/// Double-precision axis-aligned bounds.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct DBounds {
    /// Minimum corner.
    pub min: DVec3,
    /// Maximum corner.
    pub max: DVec3,
}

impl From<Vec3> for DVec3 {
    fn from(value: Vec3) -> Self {
        Self {
            x: f64::from(value.x),
            y: f64::from(value.y),
            z: f64::from(value.z),
        }
    }
}

impl From<Plane> for DPlane {
    fn from(value: Plane) -> Self {
        Self {
            normal: DVec3::from(value.normal),
            distance: f64::from(value.distance),
        }
    }
}

impl From<Bounds> for DBounds {
    fn from(value: Bounds) -> Self {
        Self {
            min: DVec3::from(value.min),
            max: DVec3::from(value.max),
        }
    }
}

/// Build a double-precision vector.
#[must_use]
pub const fn dvec3(x: f64, y: f64, z: f64) -> DVec3 {
    DVec3 { x, y, z }
}

/// Dot product.
#[must_use]
pub fn dot(a: DVec3, b: DVec3) -> f64 {
    a.x * b.x + a.y * b.y + a.z * b.z
}

/// Vector sum.
#[must_use]
pub fn add(a: DVec3, b: DVec3) -> DVec3 {
    dvec3(a.x + b.x, a.y + b.y, a.z + b.z)
}

/// Vector difference.
#[must_use]
pub fn sub(a: DVec3, b: DVec3) -> DVec3 {
    dvec3(a.x - b.x, a.y - b.y, a.z - b.z)
}

/// Uniform scale.
#[must_use]
pub fn scale(a: DVec3, s: f64) -> DVec3 {
    dvec3(a.x * s, a.y * s, a.z * s)
}

/// Cross product.
#[must_use]
pub fn cross(a: DVec3, b: DVec3) -> DVec3 {
    dvec3(a.y * b.z - a.z * b.y, a.z * b.x - a.x * b.z, a.x * b.y - a.y * b.x)
}

/// Length.
#[must_use]
pub fn length(a: DVec3) -> f64 {
    dot(a, a).sqrt()
}

/// Unit vector.
#[must_use]
pub fn unit(a: DVec3) -> DVec3 {
    scale(a, 1.0 / length(a))
}

/// Negated plane.
#[must_use]
pub fn negate_plane(p: DPlane) -> DPlane {
    DPlane {
        normal: scale(p.normal, -1.0),
        distance: -p.distance,
    }
}

/// Linear interpolation.
#[must_use]
pub fn lerp(a: DVec3, b: DVec3, t: f64) -> DVec3 {
    add(a, scale(sub(b, a), t))
}

/// Coordinate axes.
pub const AXES: [DVec3; 3] = [dvec3(1.0, 0.0, 0.0), dvec3(0.0, 1.0, 0.0), dvec3(0.0, 0.0, 1.0)];

/// One clipped face of a convex cell.
#[derive(Debug, Clone, PartialEq)]
pub struct CellFace {
    /// Face plane.
    pub plane: DPlane,
    /// Wound vertices.
    pub vertices: Vec<DVec3>,
}

/// Convex polyhedron cell as clipped faces.
#[derive(Debug, Clone, PartialEq)]
pub struct ConvexCell {
    /// Faces.
    pub faces: Vec<CellFace>,
}

struct CellBounds {
    min_x: f64,
    min_y: f64,
    min_z: f64,
    max_x: f64,
    max_y: f64,
    max_z: f64,
}

fn bounds_for_cell(cell: &ConvexCell) -> Option<CellBounds> {
    let mut bounds = CellBounds {
        min_x: f64::INFINITY,
        min_y: f64::INFINITY,
        min_z: f64::INFINITY,
        max_x: f64::NEG_INFINITY,
        max_y: f64::NEG_INFINITY,
        max_z: f64::NEG_INFINITY,
    };
    let mut any = false;
    for face in &cell.faces {
        for point in &face.vertices {
            if !point.x.is_finite() || !point.y.is_finite() || !point.z.is_finite() {
                return None;
            }
            bounds.min_x = bounds.min_x.min(point.x);
            bounds.min_y = bounds.min_y.min(point.y);
            bounds.min_z = bounds.min_z.min(point.z);
            bounds.max_x = bounds.max_x.max(point.x);
            bounds.max_y = bounds.max_y.max(point.y);
            bounds.max_z = bounds.max_z.max(point.z);
            any = true;
        }
    }
    any.then_some(bounds)
}

/// Box cell from bounds.
#[must_use]
pub fn box_cell(bounds: &DBounds) -> ConvexCell {
    let (a, b) = (bounds.min, bounds.max);
    let p000 = dvec3(a.x, a.y, a.z);
    let p001 = dvec3(a.x, a.y, b.z);
    let p010 = dvec3(a.x, b.y, a.z);
    let p011 = dvec3(a.x, b.y, b.z);
    let p100 = dvec3(b.x, a.y, a.z);
    let p101 = dvec3(b.x, a.y, b.z);
    let p110 = dvec3(b.x, b.y, a.z);
    let p111 = dvec3(b.x, b.y, b.z);
    ConvexCell {
        faces: vec![
            CellFace {
                plane: DPlane {
                    normal: AXES[0],
                    distance: b.x,
                },
                vertices: vec![p100, p110, p111, p101],
            },
            CellFace {
                plane: DPlane {
                    normal: scale(AXES[0], -1.0),
                    distance: -a.x,
                },
                vertices: vec![p000, p001, p011, p010],
            },
            CellFace {
                plane: DPlane {
                    normal: AXES[1],
                    distance: b.y,
                },
                vertices: vec![p010, p011, p111, p110],
            },
            CellFace {
                plane: DPlane {
                    normal: scale(AXES[1], -1.0),
                    distance: -a.y,
                },
                vertices: vec![p000, p100, p101, p001],
            },
            CellFace {
                plane: DPlane {
                    normal: AXES[2],
                    distance: b.z,
                },
                vertices: vec![p001, p101, p111, p011],
            },
            CellFace {
                plane: DPlane {
                    normal: scale(AXES[2], -1.0),
                    distance: -a.z,
                },
                vertices: vec![p000, p010, p110, p100],
            },
        ],
    }
}

/// Keep `n.p <= d`. The query envelope bounds otherwise unbounded BSP cells.
#[must_use]
pub fn clip_cell(cell: &ConvexCell, plane: DPlane) -> Option<ConvexCell> {
    let mut outside = false;
    let mut inside = false;
    'classify: for face in &cell.faces {
        for point in &face.vertices {
            let d = dot(*point, plane.normal) - plane.distance;
            if d > 1e-8 {
                outside = true;
            }
            if d < -1e-8 {
                inside = true;
            }
            if outside && inside {
                break 'classify;
            }
        }
    }
    if !outside {
        return Some(cell.clone());
    }
    if !inside {
        return None;
    }
    let mut faces: Vec<CellFace> = Vec::new();
    let mut cap: Vec<DVec3> = Vec::new();
    for face in &cell.faces {
        let mut vertices: Vec<DVec3> = Vec::new();
        for (i, a) in face.vertices.iter().enumerate() {
            let Some(b) = face.vertices.get((i + 1) % face.vertices.len()) else {
                continue;
            };
            let da = dot(*a, plane.normal) - plane.distance;
            let db = dot(*b, plane.normal) - plane.distance;
            if da <= 0.0 {
                vertices.push(*a);
            }
            if (da < 0.0 && db > 0.0) || (da > 0.0 && db < 0.0) {
                let point = lerp(*a, *b, da / (da - db));
                vertices.push(point);
                if !cap.iter().any(|v| length(sub(*v, point)) < 1e-7) {
                    cap.push(point);
                }
            } else if da == 0.0 && !cap.iter().any(|v| length(sub(*v, *a)) < 1e-7) {
                cap.push(*a);
            }
        }
        if vertices.len() >= 3 {
            faces.push(CellFace {
                plane: face.plane,
                vertices,
            });
        }
    }
    close_cell(faces, &mut cap, plane)
}

fn close_cell(mut faces: Vec<CellFace>, cap: &mut [DVec3], plane: DPlane) -> Option<ConvexCell> {
    if cap.len() >= 3 {
        let center = scale(
            cap.iter().fold(dvec3(0.0, 0.0, 0.0), |sum, p| add(sum, *p)),
            1.0 / cap.len() as f64,
        );
        let axis = if plane.normal.z.abs() < 0.9 { AXES[2] } else { AXES[1] };
        let u = unit(cross(axis, plane.normal));
        let v = cross(plane.normal, u);
        cap.sort_by(|a, b| {
            let angle = |p: &DVec3| dot(sub(*p, center), v).atan2(dot(sub(*p, center), u));
            angle(a).partial_cmp(&angle(b)).unwrap_or(std::cmp::Ordering::Equal)
        });
        faces.push(CellFace {
            plane,
            vertices: cap.to_vec(),
        });
    }
    if faces.len() >= 4 {
        Some(ConvexCell { faces })
    } else {
        None
    }
}

fn cap_contains_point(cap: &[DVec3], point: DVec3) -> bool {
    cap.iter().any(|vertex| {
        let x = vertex.x - point.x;
        let y = vertex.y - point.y;
        let z = vertex.z - point.z;
        (x * x + y * y + z * z).sqrt() < 1e-7
    })
}

/// Split outcome: front keeps `n.p >= d`, back keeps `n.p <= d`.
pub struct CellSplit {
    /// Front fragment (`n.p >= d`), if any.
    pub front: Option<ConvexCell>,
    /// Back fragment (`n.p <= d`), if any.
    pub back: Option<ConvexCell>,
}

/// Both BSP children share each edge intersection and its cap insertion order.
#[must_use]
pub fn split_cell(cell: &ConvexCell, plane: DPlane) -> CellSplit {
    let n = plane.normal;
    if n.x.is_finite() && n.y.is_finite() && n.z.is_finite() && plane.distance.is_finite() {
        if let Some(bounds) = bounds_for_cell(cell) {
            let lower = (if n.x < 0.0 { bounds.max_x } else { bounds.min_x }) * n.x
                + (if n.y < 0.0 { bounds.max_y } else { bounds.min_y }) * n.y
                + (if n.z < 0.0 { bounds.max_z } else { bounds.min_z }) * n.z
                - plane.distance;
            let upper = (if n.x < 0.0 { bounds.min_x } else { bounds.max_x }) * n.x
                + (if n.y < 0.0 { bounds.min_y } else { bounds.max_y }) * n.y
                + (if n.z < 0.0 { bounds.min_z } else { bounds.max_z }) * n.z
                - plane.distance;
            if lower.is_finite() && upper.is_finite() {
                if upper < -1e-8 {
                    return CellSplit {
                        front: None,
                        back: Some(cell.clone()),
                    };
                }
                if lower > 1e-8 {
                    return CellSplit {
                        front: Some(cell.clone()),
                        back: None,
                    };
                }
                if lower >= -1e-8 && upper <= 1e-8 {
                    return CellSplit {
                        front: Some(cell.clone()),
                        back: Some(cell.clone()),
                    };
                }
            }
        }
    }
    let mut outside = false;
    let mut inside = false;
    'classify: for face in &cell.faces {
        for point in &face.vertices {
            let distance = dot(*point, plane.normal) - plane.distance;
            if !distance.is_finite() {
                return CellSplit {
                    front: clip_cell(cell, negate_plane(plane)),
                    back: clip_cell(cell, plane),
                };
            }
            if distance > 1e-8 {
                outside = true;
            }
            if distance < -1e-8 {
                inside = true;
            }
            if outside && inside {
                break 'classify;
            }
        }
    }
    if !outside || !inside {
        return CellSplit {
            front: if inside { None } else { Some(cell.clone()) },
            back: if outside { None } else { Some(cell.clone()) },
        };
    }
    let opposite = negate_plane(plane);
    let mut front: Vec<CellFace> = Vec::new();
    let mut back: Vec<CellFace> = Vec::new();
    let mut cap: Vec<DVec3> = Vec::new();
    for face in &cell.faces {
        let mut front_vertices: Vec<DVec3> = Vec::new();
        let mut back_vertices: Vec<DVec3> = Vec::new();
        for (i, a) in face.vertices.iter().enumerate() {
            let Some(b) = face.vertices.get((i + 1) % face.vertices.len()) else {
                continue;
            };
            let da = dot(*a, plane.normal) - plane.distance;
            let db = dot(*b, plane.normal) - plane.distance;
            if !da.is_finite() || !db.is_finite() {
                return CellSplit {
                    front: clip_cell(cell, opposite),
                    back: clip_cell(cell, plane),
                };
            }
            if da >= 0.0 {
                front_vertices.push(*a);
            }
            if da <= 0.0 {
                back_vertices.push(*a);
            }
            if (da < 0.0 && db > 0.0) || (da > 0.0 && db < 0.0) {
                let point = lerp(*a, *b, da / (da - db));
                front_vertices.push(point);
                back_vertices.push(point);
                if !cap_contains_point(&cap, point) {
                    cap.push(point);
                }
            } else if da == 0.0 && !cap_contains_point(&cap, *a) {
                cap.push(*a);
            }
        }
        if front_vertices.len() >= 3 {
            front.push(CellFace {
                plane: face.plane,
                vertices: front_vertices,
            });
        }
        if back_vertices.len() >= 3 {
            back.push(CellFace {
                plane: face.plane,
                vertices: back_vertices,
            });
        }
    }
    let mut front_cap = cap.clone();
    CellSplit {
        front: close_cell(front, &mut front_cap, opposite),
        back: close_cell(back, &mut cap, plane),
    }
}

/// Every vertex of every face.
#[must_use]
pub fn cell_vertices(cell: &ConvexCell) -> Vec<DVec3> {
    cell.faces
        .iter()
        .flat_map(|face| face.vertices.iter().copied())
        .collect()
}

/// Full separating axes include edge cross products, not only BSP face planes.
#[must_use]
pub fn box_separating_planes(cell: &ConvexCell, axes: &[DVec3]) -> Vec<DPlane> {
    let mut normals: Vec<DVec3> = Vec::new();
    let mut axial_seen: u32 = 0;
    let insert = |normal: DVec3, normals: &mut Vec<DVec3>, axial_seen: &mut u32| {
        let magnitude = length(normal);
        if magnitude < 1e-8 {
            return;
        }
        let n = scale(normal, 1.0 / magnitude);
        let axial = if n.y == 0.0 && n.z == 0.0 {
            if n.x == 1.0 {
                1
            } else if n.x == -1.0 {
                2
            } else {
                0
            }
        } else if n.x == 0.0 && n.z == 0.0 {
            if n.y == 1.0 {
                4
            } else if n.y == -1.0 {
                8
            } else {
                0
            }
        } else if n.x == 0.0 && n.y == 0.0 {
            if n.z == 1.0 {
                16
            } else if n.z == -1.0 {
                32
            } else {
                0
            }
        } else {
            0
        };
        if (*axial_seen & axial) != 0 {
            return;
        }
        if !normals.iter().any(|p| dot(*p, n) > 1.0 - 1e-10) {
            normals.push(n);
        }
        *axial_seen |= axial;
    };
    for face in &cell.faces {
        insert(face.plane.normal, &mut normals, &mut axial_seen);
        for (i, a) in face.vertices.iter().enumerate() {
            let Some(b) = face.vertices.get((i + 1) % face.vertices.len()) else {
                continue;
            };
            let edge = sub(*b, *a);
            for axis in axes {
                let n = cross(edge, *axis);
                insert(n, &mut normals, &mut axial_seen);
                insert(scale(n, -1.0), &mut normals, &mut axial_seen);
            }
        }
    }
    for axis in axes {
        insert(*axis, &mut normals, &mut axial_seen);
        insert(scale(*axis, -1.0), &mut normals, &mut axial_seen);
    }
    let vertices = cell_vertices(cell);
    normals
        .into_iter()
        .map(|normal| DPlane {
            normal,
            distance: vertices.iter().fold(f64::NEG_INFINITY, |d, p| d.max(dot(normal, *p))),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unit_bounds() -> DBounds {
        DBounds {
            min: dvec3(0.0, 0.0, 0.0),
            max: dvec3(8.0, 8.0, 8.0),
        }
    }

    #[test]
    fn vector_helpers_match_donor() {
        let a = dvec3(1.0, 2.0, 3.0);
        let b = dvec3(4.0, -1.0, 0.5);
        assert!((dot(a, b) - 3.5).abs() < 1e-12);
        assert_eq!(add(a, b), dvec3(5.0, 1.0, 3.5));
        assert_eq!(sub(a, b), dvec3(-3.0, 3.0, 2.5));
        assert_eq!(scale(a, 2.0), dvec3(2.0, 4.0, 6.0));
        assert_eq!(cross(AXES[0], AXES[1]), AXES[2]);
        assert!((length(a) - 14.0_f64.sqrt()).abs() < 1e-12);
        assert!((length(unit(a)) - 1.0).abs() < 1e-12);
        assert_eq!(lerp(a, b, 0.5), dvec3(2.5, 0.5, 1.75));
        let plane = DPlane {
            normal: AXES[0],
            distance: 3.0,
        };
        assert_eq!(negate_plane(plane).distance, -3.0);
    }

    #[test]
    fn box_cell_has_six_faces() {
        let cell = box_cell(&unit_bounds());
        assert_eq!(cell.faces.len(), 6);
        assert!(cell.faces.iter().all(|face| face.vertices.len() == 4));
        assert_eq!(cell_vertices(&cell).len(), 24);
    }

    #[test]
    fn clip_cell_keeps_back_side() {
        let cell = box_cell(&unit_bounds());
        let plane = DPlane {
            normal: AXES[0],
            distance: 4.0,
        };
        let clipped = clip_cell(&cell, plane).expect("keeps back");
        let max_x = cell_vertices(&clipped)
            .iter()
            .map(|v| v.x)
            .fold(f64::NEG_INFINITY, f64::max);
        assert!(max_x <= 4.0 + 1e-6);
        let untouched = clip_cell(
            &cell,
            DPlane {
                normal: AXES[0],
                distance: 100.0,
            },
        )
        .expect("inside");
        assert_eq!(untouched.faces.len(), 6);
        assert!(clip_cell(
            &cell,
            DPlane {
                normal: AXES[0],
                distance: -100.0
            }
        )
        .is_none());
    }

    #[test]
    fn split_cell_partitions_both_sides() {
        let cell = box_cell(&unit_bounds());
        let split = split_cell(
            &cell,
            DPlane {
                normal: AXES[1],
                distance: 4.0,
            },
        );
        let (front, back) = (split.front.expect("front"), split.back.expect("back"));
        let front_min_y = cell_vertices(&front).iter().map(|v| v.y).fold(f64::INFINITY, f64::min);
        let back_max_y = cell_vertices(&back)
            .iter()
            .map(|v| v.y)
            .fold(f64::NEG_INFINITY, f64::max);
        assert!(front_min_y >= 4.0 - 1e-6);
        assert!(back_max_y <= 4.0 + 1e-6);
        // Degenerate plane containment returns the cell on both sides.
        let contained = split_cell(
            &cell,
            DPlane {
                normal: AXES[2],
                distance: 4.0,
            },
        );
        assert!(contained.front.is_some() && contained.back.is_some());
    }

    #[test]
    fn separating_planes_cover_box_normals() {
        let cell = box_cell(&unit_bounds());
        let planes = box_separating_planes(&cell, &AXES);
        for axis in AXES {
            assert!(planes.iter().any(|p| dot(p.normal, axis) > 1.0 - 1e-9));
            assert!(planes.iter().any(|p| dot(p.normal, axis) < -(1.0 - 1e-9)));
        }
        let x_max = planes
            .iter()
            .find(|p| dot(p.normal, AXES[0]) > 1.0 - 1e-9)
            .expect("x plane");
        assert!((x_max.distance - 8.0).abs() < 1e-9);
    }
}
