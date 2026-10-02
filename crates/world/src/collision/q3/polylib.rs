//! Winding library translated from id Software's `code/qcommon/cm_polylib.c`
//! and `cm_polylib.h`.
//!
//! Donor provenance: `/home/buzzkill/Projects/quake-typescript/src/world/collision/q3/polylib.ts`.
//!
//! The count and float cells live in the winding allocation, as in the
//! donor; Rust borrows replace the donor's shared references, so freeing
//! consumes the winding. Numeric expressions keep the donor's binary64
//! intermediates and round once at the winding store.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use qa_core::math::{vec3, Bounds, Plane, Vec3};

use super::allocation::ZoneAllocation;
use super::counters::CollisionCounters;
use crate::error::WorldError;

/// Zone allocator behind [`CollisionWindingLibrary`].
pub trait CollisionWindingMemory {
    /// Allocate `bytes` zeroed bytes.
    fn allocate(&self, bytes: usize) -> ZoneAllocation;
    /// Release a freed winding allocation.
    fn free(&self, allocation: ZoneAllocation);
}

/// Which side of a plane a winding lies on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindingSide {
    /// In front.
    Front = 0,
    /// Behind.
    Back = 1,
    /// Coincident.
    On = 2,
    /// Spanning.
    Cross = 3,
}

/// On-plane epsilon (`Math.fround(0.1)`).
pub const ON_EPSILON: f32 = 0.1;
/// Map bounds retained by winding generation.
pub const MAX_MAP_BOUNDS: f32 = 65535.0;
/// Freed-winding stamp (`0xdeaddead`).
const FREED_STAMP: u32 = 0xdead_dead;
/// Largest winding allocation: `(0x7fffffff - 4) / 12` points.
const MAX_WINDING_POINTS: usize = (0x7fff_ffff_usize - 4) / 12;

fn at<T>(values: &[T], index: usize) -> Result<&T, WorldError> {
    values
        .get(index)
        .ok_or_else(|| WorldError::BadCollisionRecord(format!("Winding point {index} outside allocation")))
}

fn dot(a: Vec3, b: Vec3) -> f64 {
    f64::from(a.x) * f64::from(b.x) + f64::from(a.y) * f64::from(b.y) + f64::from(a.z) * f64::from(b.z)
}

fn cross(a: Vec3, b: Vec3) -> [f64; 3] {
    let (ax, ay, az) = (f64::from(a.x), f64::from(a.y), f64::from(a.z));
    let (bx, by, bz) = (f64::from(b.x), f64::from(b.y), f64::from(b.z));
    [ay * bz - az * by, az * bx - ax * bz, ax * by - ay * bx]
}

fn sub(a: Vec3, b: Vec3) -> [f64; 3] {
    [
        f64::from(a.x) - f64::from(b.x),
        f64::from(a.y) - f64::from(b.y),
        f64::from(a.z) - f64::from(b.z),
    ]
}

fn normalize(value: [f64; 3]) -> Vec3 {
    let length = (value[0] * value[0] + value[1] * value[1] + value[2] * value[2]).sqrt();
    if length == 0.0 {
        vec3(0.0, 0.0, 0.0)
    } else {
        vec3(
            (value[0] / length) as f32,
            (value[1] / length) as f32,
            (value[2] / length) as f32,
        )
    }
}

fn store(value: [f64; 3]) -> Vec3 {
    vec3(value[0] as f32, value[1] as f32, value[2] as f32)
}

/// Exact fixed-point decimal of a binary32 value, matching the donor's
/// BigInt expansion (half-to-even at `digits`).
fn fixed(value: f32, digits: usize) -> String {
    if !value.is_finite() {
        if value.is_nan() {
            return if value.is_sign_negative() {
                "-nan".to_string()
            } else {
                "nan".to_string()
            };
        }
        return if value.is_sign_negative() {
            "-inf".to_string()
        } else {
            "inf".to_string()
        };
    }
    format!("{:.digits$}", f64::from(value), digits = digits)
}

/// A winding whose count and float cells live in its allocation.
#[derive(Debug)]
pub struct CollisionWinding {
    bytes: RefCell<Vec<u8>>,
    capacity: usize,
    freed: Cell<bool>,
}

impl CollisionWinding {
    fn new(bytes: Vec<u8>, capacity: usize) -> Self {
        Self {
            bytes: RefCell::new(bytes),
            capacity,
            freed: Cell::new(false),
        }
    }

    fn check_live(&self) -> Result<(), WorldError> {
        if self.freed.get() {
            return Err(WorldError::BadCollisionRecord(
                "Winding allocation is freed".to_string(),
            ));
        }
        Ok(())
    }

    /// Live point count.
    pub fn num_points(&self) -> Result<usize, WorldError> {
        self.check_live()?;
        let bytes = self.bytes.borrow();
        let count = i32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
        usize::try_from(count)
            .ok()
            .filter(|count| *count <= self.capacity)
            .ok_or_else(|| WorldError::BadCollisionRecord("Winding count outside allocation".to_string()))
    }

    /// Store the live point count.
    pub fn set_num_points(&self, value: usize) -> Result<(), WorldError> {
        if value > self.capacity {
            return Err(WorldError::BadCollisionRecord(
                "Winding count outside allocation".to_string(),
            ));
        }
        self.check_live()?;
        let mut bytes = self.bytes.borrow_mut();
        #[allow(clippy::cast_possible_wrap)]
        bytes[0..4].copy_from_slice(&(value as i32).to_le_bytes());
        Ok(())
    }

    /// All live points.
    pub fn points(&self) -> Result<Vec<Vec3>, WorldError> {
        let count = self.num_points()?;
        (0..count).map(|index| self.point(index)).collect()
    }

    /// Read point `index` (validated against capacity, as in the donor).
    pub fn point(&self, index: usize) -> Result<Vec3, WorldError> {
        if index >= self.capacity {
            return Err(WorldError::BadCollisionRecord(format!(
                "Winding point {index} outside allocation"
            )));
        }
        self.check_live()?;
        let bytes = self.bytes.borrow();
        let offset = 4 + index * 12;
        Ok(vec3(
            f32::from_le_bytes([bytes[offset], bytes[offset + 1], bytes[offset + 2], bytes[offset + 3]]),
            f32::from_le_bytes([
                bytes[offset + 4],
                bytes[offset + 5],
                bytes[offset + 6],
                bytes[offset + 7],
            ]),
            f32::from_le_bytes([
                bytes[offset + 8],
                bytes[offset + 9],
                bytes[offset + 10],
                bytes[offset + 11],
            ]),
        ))
    }

    /// Write point `index`.
    pub fn set_point(&self, index: usize, point: Vec3) -> Result<(), WorldError> {
        if index >= self.capacity {
            return Err(WorldError::BadCollisionRecord(format!(
                "Winding point {index} outside allocation"
            )));
        }
        self.check_live()?;
        let mut bytes = self.bytes.borrow_mut();
        let offset = 4 + index * 12;
        bytes[offset..offset + 4].copy_from_slice(&point.x.to_le_bytes());
        bytes[offset + 4..offset + 8].copy_from_slice(&point.y.to_le_bytes());
        bytes[offset + 8..offset + 12].copy_from_slice(&point.z.to_le_bytes());
        Ok(())
    }

    /// Append a point, growing the live count.
    pub fn append(&self, point: Vec3) -> Result<(), WorldError> {
        let index = self.num_points()?;
        self.set_point(index, point)?;
        self.set_num_points(index + 1)
    }

    /// Stamp the allocation freed; freeing twice is fatal.
    pub fn mark_freed(&self) -> Result<(), WorldError> {
        let stamped = {
            let bytes = self.bytes.borrow();
            u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) == FREED_STAMP
        };
        if self.freed.get() || stamped {
            return Err(WorldError::BadCollisionRecord(
                "FreeWinding: freed a freed winding".to_string(),
            ));
        }
        self.bytes.borrow_mut()[0..4].copy_from_slice(&FREED_STAMP.to_le_bytes());
        self.freed.set(true);
        Ok(())
    }
}

/// Front/back split from [`CollisionWindingLibrary::clip`].
#[derive(Debug)]
pub struct ClipSplit {
    /// Front fragment, if any.
    pub front: Option<CollisionWinding>,
    /// Back fragment, if any.
    pub back: Option<CollisionWinding>,
}

/// Common-lived counters and the actual zone used by patch generation and
/// debug drawing.
#[derive(Default)]
pub struct CollisionWindingLibrary {
    memory: Option<Rc<dyn CollisionWindingMemory>>,
    /// Active windings.
    pub c_active_windings: Cell<i32>,
    /// Peak active windings.
    pub c_peak_windings: Cell<i32>,
    /// Total winding allocations.
    pub c_winding_allocs: Cell<i32>,
    /// Total winding points allocated.
    pub c_winding_points: Cell<i32>,
    /// Colinear points removed.
    pub c_removed: Cell<i32>,
}

impl std::fmt::Debug for CollisionWindingLibrary {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CollisionWindingLibrary")
            .field("memory", &self.memory.is_some())
            .field("c_active_windings", &self.c_active_windings.get())
            .field("c_peak_windings", &self.c_peak_windings.get())
            .field("c_winding_allocs", &self.c_winding_allocs.get())
            .field("c_winding_points", &self.c_winding_points.get())
            .field("c_removed", &self.c_removed.get())
            .finish()
    }
}

impl CollisionWindingLibrary {
    /// Borrow an optional zone; `None` allocates locally.
    #[must_use]
    pub fn new(memory: Option<Rc<dyn CollisionWindingMemory>>) -> Self {
        Self {
            memory,
            c_active_windings: Cell::new(0),
            c_peak_windings: Cell::new(0),
            c_winding_allocs: Cell::new(0),
            c_winding_points: Cell::new(0),
            c_removed: Cell::new(0),
        }
    }

    /// Allocate a winding with room for `points` points.
    pub fn alloc(&self, points: usize) -> Result<CollisionWinding, WorldError> {
        CollisionCounters::bump(&self.c_winding_allocs);
        self.c_winding_points
            .set(self.c_winding_points.get().wrapping_add(points as i32));
        CollisionCounters::bump(&self.c_active_windings);
        if self.c_active_windings.get() > self.c_peak_windings.get() {
            self.c_peak_windings.set(self.c_active_windings.get());
        }
        if points > MAX_WINDING_POINTS {
            return Err(WorldError::BadCollisionRecord(
                "Invalid winding allocation size".to_string(),
            ));
        }
        let size = 4 + points * 12;
        let mut allocation = match &self.memory {
            None => ZoneAllocation { bytes: vec![0; size] },
            Some(memory) => memory.allocate(size),
        };
        allocation.bytes.fill(0);
        allocation.bytes.resize(size, 0);
        Ok(CollisionWinding::new(allocation.bytes, points))
    }

    /// Free a winding, returning its allocation to the zone.
    pub fn free(&self, winding: CollisionWinding) -> Result<(), WorldError> {
        winding.mark_freed()?;
        self.c_active_windings.set(self.c_active_windings.get().wrapping_sub(1));
        if let Some(memory) = &self.memory {
            memory.free(ZoneAllocation {
                bytes: winding.bytes.into_inner(),
            });
        }
        Ok(())
    }

    /// Print a winding's points at one decimal place.
    pub fn pw(&self, winding: &CollisionWinding, print: &dyn Fn(&str)) -> Result<(), WorldError> {
        for point in winding.points()? {
            print(&format!(
                "({:>5}, {:>5}, {:>5})\n",
                fixed(point.x, 1),
                fixed(point.y, 1),
                fixed(point.z, 1)
            ));
        }
        Ok(())
    }

    /// Duplicate a winding's count and float cells.
    pub fn copy(&self, winding: &CollisionWinding) -> Result<CollisionWinding, WorldError> {
        let count = winding.num_points()?;
        let copy = self.alloc(count)?;
        for index in 0..count {
            copy.set_point(index, winding.point(index)?)?;
        }
        copy.set_num_points(count)?;
        Ok(copy)
    }

    /// Duplicate a winding with reversed point order.
    pub fn reverse(&self, winding: &CollisionWinding) -> Result<CollisionWinding, WorldError> {
        let count = winding.num_points()?;
        let copy = self.alloc(count)?;
        for index in (0..count).rev() {
            copy.append(winding.point(index)?)?;
        }
        Ok(copy)
    }

    /// Drop points colinear with their neighbors.
    pub fn remove_colinear_points(&self, winding: &CollisionWinding) -> Result<(), WorldError> {
        let points = winding.points()?;
        if points.is_empty() {
            return Ok(());
        }
        let mut kept = Vec::new();
        for (index, point) in points.iter().enumerate() {
            let next = at(&points, (index + 1) % points.len())?;
            let prev = at(&points, (index + points.len() - 1) % points.len())?;
            let a = normalize(sub(*next, *point));
            let b = normalize(sub(*point, *prev));
            if dot(a, b) < 0.999 {
                kept.push(*point);
            }
        }
        if kept.len() == points.len() {
            return Ok(());
        }
        self.c_removed
            .set(self.c_removed.get().wrapping_add((points.len() - kept.len()) as i32));
        winding.set_num_points(kept.len())?;
        for (index, point) in kept.iter().enumerate() {
            winding.set_point(index, *point)?;
        }
        Ok(())
    }

    /// Face plane through the first three points.
    pub fn plane(&self, winding: &CollisionWinding) -> Result<Plane, WorldError> {
        let point = winding.point(0)?;
        let normal = normalize(cross(
            store(sub(winding.point(2)?, point)),
            store(sub(winding.point(1)?, point)),
        ));
        let distance = dot(point, normal) as f32;
        Ok(Plane { normal, distance })
    }

    /// Fan area from the first point, accumulated in binary32.
    pub fn area(&self, winding: &CollisionWinding) -> Result<f32, WorldError> {
        let mut total = 0.0f32;
        let count = winding.num_points()?;
        for index in 2..count {
            let first = sub(winding.point(index - 1)?, winding.point(0)?);
            let second = sub(winding.point(index)?, winding.point(0)?);
            let c = cross(store(first), store(second));
            let triangle = 0.5 * (c[0] * c[0] + c[1] * c[1] + c[2] * c[2]).sqrt();
            total = (f64::from(total) + triangle) as f32;
        }
        Ok(total)
    }

    /// Bounds over the live points.
    pub fn bounds(&self, winding: &CollisionWinding) -> Result<Bounds, WorldError> {
        Ok(winding_bounds(&winding.points()?))
    }

    /// Centroid of the live points.
    pub fn center(&self, winding: &CollisionWinding) -> Result<Vec3, WorldError> {
        let mut sum = [0.0f64; 3];
        for point in winding.points()? {
            sum[0] += f64::from(point.x);
            sum[1] += f64::from(point.y);
            sum[2] += f64::from(point.z);
        }
        let scale = f64::from((1.0 / winding.num_points()? as f64) as f32);
        Ok(store([sum[0] * scale, sum[1] * scale, sum[2] * scale]))
    }

    /// Huge quad winding for a plane.
    pub fn base_for_plane(&self, plane: &Plane) -> Result<CollisionWinding, WorldError> {
        let normal = plane.normal;
        let mut maximum = -f64::from(MAX_MAP_BOUNDS);
        let mut major: Option<usize> = None;
        for (axis, component) in [normal.x, normal.y, normal.z].into_iter().enumerate() {
            let magnitude = f64::from(component).abs();
            if magnitude > maximum {
                maximum = magnitude;
                major = Some(axis);
            }
        }
        let Some(major) = major else {
            return Err(WorldError::BadCollisionRecord(
                "BaseWindingForPlane: no axis found".to_string(),
            ));
        };
        let initial = if major == 2 {
            vec3(1.0, 0.0, 0.0)
        } else {
            vec3(0.0, 0.0, 1.0)
        };
        let projected = dot(initial, normal);
        let up = normalize([
            f64::from(initial.x) - projected * f64::from(normal.x),
            f64::from(initial.y) - projected * f64::from(normal.y),
            f64::from(initial.z) - projected * f64::from(normal.z),
        ]);
        // Preserve the retained i386 x87 spill and extended reciprocal profile.
        let (nx, ny, nz) = (f64::from(normal.x), f64::from(normal.y), f64::from(normal.z));
        let (ux, uy, uz) = (f64::from(up.x), f64::from(up.y), f64::from(up.z));
        let bound = f64::from(MAX_MAP_BOUNDS);
        let right = [
            (f64::from((uy * nz) as f32) - uz * ny) * bound,
            (uz * nx - ux * nz) * bound,
            (ux * ny - uy * nx) * bound,
        ];
        let vertical = [ux * bound, uy * bound, uz * bound];
        let distance = f64::from(plane.distance);
        let origin = [nx * distance, ny * distance, nz * distance];
        let result = self.alloc(4)?;
        result.append(store([
            origin[0] - right[0] + vertical[0],
            origin[1] - right[1] + vertical[1],
            origin[2] - right[2] + vertical[2],
        ]))?;
        result.append(store([
            origin[0] + right[0] + vertical[0],
            origin[1] + right[1] + vertical[1],
            origin[2] + right[2] + vertical[2],
        ]))?;
        result.append(store([
            origin[0] + right[0] - vertical[0],
            origin[1] + right[1] - vertical[1],
            origin[2] + right[2] - vertical[2],
        ]))?;
        result.append(store([
            origin[0] - right[0] - vertical[0],
            origin[1] - right[1] - vertical[1],
            origin[2] - right[2] - vertical[2],
        ]))?;
        Ok(result)
    }

    /// Split a winding by a plane without freeing the input.
    pub fn clip(&self, winding: &CollisionWinding, plane: &Plane, epsilon: f32) -> Result<ClipSplit, WorldError> {
        let points = winding.points()?;
        let distances = winding_distances(&points, plane);
        if !distances.iter().any(|d| *d > epsilon) {
            return Ok(ClipSplit {
                front: None,
                back: Some(self.copy(winding)?),
            });
        }
        if !distances.iter().any(|d| *d < -epsilon) {
            return Ok(ClipSplit {
                front: Some(self.copy(winding)?),
                back: None,
            });
        }
        let front = self.alloc(points.len() + 4)?;
        let back = self.alloc(points.len() + 4)?;
        split_winding(&points, &distances, plane, epsilon, &front, Some(&back))?;
        check_clip_counts(&front, Some(&back))?;
        Ok(ClipSplit {
            front: Some(front),
            back: Some(back),
        })
    }

    /// Keep the front fragment, freeing the input unless it survives whole.
    pub fn chop_in_place(
        &self,
        winding: CollisionWinding,
        plane: &Plane,
        epsilon: f32,
    ) -> Result<Option<CollisionWinding>, WorldError> {
        let points = winding.points()?;
        let distances = winding_distances(&points, plane);
        if !distances.iter().any(|d| *d > epsilon) {
            self.free(winding)?;
            return Ok(None);
        }
        if !distances.iter().any(|d| *d < -epsilon) {
            return Ok(Some(winding));
        }
        let front = self.alloc(points.len() + 4)?;
        split_winding(&points, &distances, plane, epsilon, &front, None)?;
        check_clip_counts(&front, None)?;
        self.free(winding)?;
        Ok(Some(front))
    }

    /// Keep the front fragment, freeing the input and the back fragment.
    pub fn chop(&self, winding: CollisionWinding, plane: &Plane) -> Result<Option<CollisionWinding>, WorldError> {
        let split = self.clip(&winding, plane, ON_EPSILON)?;
        self.free(winding)?;
        if let Some(back) = split.back {
            self.free(back)?;
        }
        Ok(split.front)
    }

    /// Classify a winding against a plane.
    pub fn on_plane_side(&self, winding: &CollisionWinding, plane: &Plane) -> Result<WindingSide, WorldError> {
        let mut front = false;
        let mut back = false;
        for point in winding.points()? {
            let distance = (dot(point, plane.normal) - f64::from(plane.distance)) as f32;
            if distance < -ON_EPSILON {
                if front {
                    return Ok(WindingSide::Cross);
                }
                back = true;
                continue;
            }
            if distance > ON_EPSILON {
                if back {
                    return Ok(WindingSide::Cross);
                }
                front = true;
            }
        }
        Ok(if back {
            WindingSide::Back
        } else if front {
            WindingSide::Front
        } else {
            WindingSide::On
        })
    }

    /// Validate point count, area, range, planarity, edges, and convexity.
    pub fn check(&self, winding: &CollisionWinding) -> Result<(), WorldError> {
        if winding.num_points()? < 3 {
            return Err(WorldError::BadCollisionRecord(format!(
                "CheckWinding: {} points",
                winding.num_points()?
            )));
        }
        let area = self.area(winding)?;
        if area < 1.0 {
            return Err(WorldError::BadCollisionRecord(format!(
                "CheckWinding: {} area",
                fixed(area, 6)
            )));
        }
        let face = self.plane(winding)?;
        let points = winding.points()?;
        for (index, point) in points.iter().enumerate() {
            for value in [point.x, point.y, point.z] {
                if value > MAX_MAP_BOUNDS || value < -MAX_MAP_BOUNDS {
                    return Err(WorldError::BadCollisionRecord(format!(
                        "CheckFace: BUGUS_RANGE: {}",
                        fixed(value, 6)
                    )));
                }
            }
            let distance = (dot(*point, face.normal) - f64::from(face.distance)) as f32;
            if distance < -ON_EPSILON || distance > ON_EPSILON {
                return Err(WorldError::BadCollisionRecord(
                    "CheckWinding: point off plane".to_string(),
                ));
            }
            let direction = sub(points[(index + 1) % points.len()], *point);
            let edge = (direction[0] * direction[0] + direction[1] * direction[1] + direction[2] * direction[2]).sqrt();
            if edge < f64::from(ON_EPSILON) {
                return Err(WorldError::BadCollisionRecord(
                    "CheckWinding: degenerate edge".to_string(),
                ));
            }
            let edge_normal = normalize(cross(face.normal, store(direction)));
            let edge_distance = (dot(*point, edge_normal) as f32) + ON_EPSILON;
            for (other, query) in points.iter().enumerate() {
                if other != index && (dot(*query, edge_normal) as f32) > edge_distance {
                    return Err(WorldError::BadCollisionRecord("CheckWinding: non-convex".to_string()));
                }
            }
        }
        Ok(())
    }

    /// Fold a winding into a convex hull around `normal`.
    pub fn add_to_convex_hull(
        &self,
        winding: &CollisionWinding,
        hull: Option<CollisionWinding>,
        normal: Vec3,
    ) -> Result<CollisionWinding, WorldError> {
        let Some(hull) = hull else {
            return self.copy(winding);
        };
        let mut points = hull.points()?;
        for point in winding.points()? {
            let mut outside = false;
            let mut sides = Vec::with_capacity(points.len());
            for (index, query) in points.iter().enumerate() {
                let edge = normalize(sub(points[(index + 1) % points.len()], *query));
                let direction = cross(normal, edge);
                let delta = sub(point, *query);
                let distance = (delta[0] * direction[0] + delta[1] * direction[1] + delta[2] * direction[2]) as f32;
                if distance >= ON_EPSILON {
                    outside = true;
                }
                sides.push(distance >= -ON_EPSILON);
            }
            if !outside {
                continue;
            }
            let mut transition: Option<usize> = None;
            for (index, side) in sides.iter().enumerate() {
                if !side && sides[(index + 1) % sides.len()] {
                    transition = Some(index);
                    break;
                }
            }
            let Some(transition) = transition else {
                continue;
            };
            let mut next = vec![point];
            let start = (transition + 1) % points.len();
            for offset in 0..points.len() {
                if sides[(start + offset) % points.len()] && sides[(start + offset + 1) % points.len()] {
                    continue;
                }
                next.push(points[(start + offset + 1) % points.len()]);
            }
            if next.len() > 128 {
                return Err(WorldError::BadCollisionRecord(
                    "Winding convex hull exceeds MAX_HULL_POINTS".to_string(),
                ));
            }
            points = next;
        }
        self.free(hull)?;
        let result = self.alloc(points.len())?;
        for point in &points {
            result.append(*point)?;
        }
        Ok(result)
    }
}

fn winding_distances(points: &[Vec3], plane: &Plane) -> Vec<f32> {
    points
        .iter()
        .map(|point| (dot(*point, plane.normal) - f64::from(plane.distance)) as f32)
        .collect()
}

fn split_winding(
    points: &[Vec3],
    distances: &[f32],
    plane: &Plane,
    epsilon: f32,
    front: &CollisionWinding,
    back: Option<&CollisionWinding>,
) -> Result<(), WorldError> {
    for (index, point) in points.iter().enumerate() {
        let d1 = distances[index];
        let next = (index + 1) % points.len();
        let d2 = distances[next];
        let side1 = if d1 > epsilon {
            WindingSide::Front
        } else if d1 < -epsilon {
            WindingSide::Back
        } else {
            WindingSide::On
        };
        let side2 = if d2 > epsilon {
            WindingSide::Front
        } else if d2 < -epsilon {
            WindingSide::Back
        } else {
            WindingSide::On
        };
        if side1 == WindingSide::On {
            front.append(*point)?;
            if let Some(back) = back {
                back.append(*point)?;
            }
            continue;
        }
        if side1 == WindingSide::Front {
            front.append(*point)?;
        } else if let Some(back) = back {
            back.append(*point)?;
        }
        if side2 == WindingSide::On || side1 == side2 {
            continue;
        }
        let query = points[next];
        let fraction = f64::from(d1) / (f64::from(d1) - f64::from(d2));
        let normal = plane.normal;
        let mid = store([
            if normal.x == 1.0 {
                f64::from(plane.distance)
            } else if normal.x == -1.0 {
                -f64::from(plane.distance)
            } else {
                f64::from(point.x) + fraction * (f64::from(query.x) - f64::from(point.x))
            },
            if normal.y == 1.0 {
                f64::from(plane.distance)
            } else if normal.y == -1.0 {
                -f64::from(plane.distance)
            } else {
                f64::from(point.y) + fraction * (f64::from(query.y) - f64::from(point.y))
            },
            if normal.z == 1.0 {
                f64::from(plane.distance)
            } else if normal.z == -1.0 {
                -f64::from(plane.distance)
            } else {
                f64::from(point.z) + fraction * (f64::from(query.z) - f64::from(point.z))
            },
        ]);
        front.append(mid)?;
        if let Some(back) = back {
            back.append(mid)?;
        }
    }
    Ok(())
}

fn check_clip_counts(front: &CollisionWinding, back: Option<&CollisionWinding>) -> Result<(), WorldError> {
    // Appends already validate capacity, so the estimate branch below is
    // defensive, exactly as in the donor.
    let over_estimate =
        |winding: &CollisionWinding| -> Result<bool, WorldError> { Ok(winding.num_points()? > winding.capacity) };
    let back_over = match back {
        Some(winding) => over_estimate(winding)?,
        None => false,
    };
    if over_estimate(front)? || back_over {
        return Err(WorldError::BadCollisionRecord(
            "ClipWinding: points exceeded estimate".to_string(),
        ));
    }
    let back_points = match back {
        Some(winding) => winding.num_points()?,
        None => 0,
    };
    if front.num_points()? > 64 || back_points > 64 {
        return Err(WorldError::BadCollisionRecord(
            "ClipWinding: MAX_POINTS_ON_WINDING".to_string(),
        ));
    }
    Ok(())
}

/// Bounds over raw points; patch grids use this before any winding exists.
#[must_use]
pub fn winding_bounds(points: &[Vec3]) -> Bounds {
    let mut min = vec3(MAX_MAP_BOUNDS, MAX_MAP_BOUNDS, MAX_MAP_BOUNDS);
    let mut max = vec3(-MAX_MAP_BOUNDS, -MAX_MAP_BOUNDS, -MAX_MAP_BOUNDS);
    for point in points {
        min = vec3(
            if point.x < min.x { point.x } else { min.x },
            if point.y < min.y { point.y } else { min.y },
            if point.z < min.z { point.z } else { min.z },
        );
        max = vec3(
            if point.x > max.x { point.x } else { max.x },
            if point.y > max.y { point.y } else { max.y },
            if point.z > max.z { point.z } else { max.z },
        );
    }
    Bounds { min, max }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell as StdRefCell;

    fn square(lib: &CollisionWindingLibrary) -> CollisionWinding {
        let winding = lib.alloc(4).expect("alloc");
        winding.append(vec3(-1.0, -1.0, 0.0)).expect("append");
        winding.append(vec3(1.0, -1.0, 0.0)).expect("append");
        winding.append(vec3(1.0, 1.0, 0.0)).expect("append");
        winding.append(vec3(-1.0, 1.0, 0.0)).expect("append");
        winding
    }

    #[test]
    fn alloc_counts_and_zeroes() {
        let lib = CollisionWindingLibrary::new(None);
        let winding = lib.alloc(3).expect("alloc");
        assert_eq!(winding.num_points().expect("count"), 0);
        assert_eq!(winding.point(0).expect("zeroed"), vec3(0.0, 0.0, 0.0));
        assert_eq!(lib.c_winding_allocs.get(), 1);
        assert_eq!(lib.c_winding_points.get(), 3);
        assert_eq!(lib.c_active_windings.get(), 1);
        assert_eq!(lib.c_peak_windings.get(), 1);
        lib.free(winding).expect("free");
        assert_eq!(lib.c_active_windings.get(), 0);
    }

    #[test]
    fn alloc_rejects_huge_sizes_but_counts_first() {
        let lib = CollisionWindingLibrary::new(None);
        let error = lib.alloc(MAX_WINDING_POINTS + 1).expect_err("huge alloc must fail");
        assert_eq!(error.to_string(), "Invalid winding allocation size");
        assert_eq!(lib.c_winding_allocs.get(), 1);
    }

    #[test]
    fn double_free_is_fatal() {
        let lib = CollisionWindingLibrary::new(None);
        let winding = lib.alloc(2).expect("alloc");
        winding.mark_freed().expect("first free stamp");
        let error = winding.mark_freed().expect_err("second free must fail");
        assert_eq!(error.to_string(), "FreeWinding: freed a freed winding");
    }

    #[test]
    fn fixed_matches_donor_vectors() {
        let cases: &[(f32, &str, &str)] = &[
            (0.0, "0.0", "0.000000"),
            (-0.0, "-0.0", "-0.000000"),
            (1.5, "1.5", "1.500000"),
            (123.456, "123.5", "123.456001"),
            (2.5, "2.5", "2.500000"),
            (0.05, "0.1", "0.050000"),
            (0.15, "0.2", "0.150000"),
            (0.125, "0.1", "0.125000"),
            (1.005, "1.0", "1.005000"),
            (2.675, "2.7", "2.675000"),
            (65535.0, "65535.0", "65535.000000"),
            (-3.75, "-3.8", "-3.750000"),
            (1e10, "10000000000.0", "10000000000.000000"),
            (1e-5, "0.0", "0.000010"),
            (1e-40, "0.0", "0.000000"),
            (123456.789f64 as f32, "123456.8", "123456.789062"),
            (999.95, "1000.0", "999.950012"),
        ];
        for (value, one, six) in cases {
            assert_eq!(fixed(*value, 1), *one, "one digit for {value}");
            assert_eq!(fixed(*value, 6), *six, "six digits for {value}");
        }
        assert_eq!(fixed(f32::MAX, 1), "340282346638528859811704183484516925440.0");
        assert_eq!(fixed(f32::INFINITY, 1), "inf");
        assert_eq!(fixed(f32::NEG_INFINITY, 6), "-inf");
        assert_eq!(fixed(f32::NAN, 1), "nan");
        assert_eq!(fixed(f32::from_bits(0xFFC0_0000), 6), "-nan");
    }

    #[test]
    fn base_plane_chop_and_check_round_trip() {
        let lib = CollisionWindingLibrary::new(None);
        let base = lib
            .base_for_plane(&Plane {
                normal: vec3(0.0, 0.0, 1.0),
                distance: 0.0,
            })
            .expect("base");
        assert_eq!(base.num_points().expect("count"), 4);
        lib.check(&base).expect("base checks out");
        let kept = lib
            .chop(
                base,
                &Plane {
                    normal: vec3(1.0, 0.0, 0.0),
                    distance: 0.0,
                },
            )
            .expect("chop");
        let kept = kept.expect("front survives");
        assert_eq!(kept.num_points().expect("count"), 4);
        assert!(lib.area(&kept).expect("area") > 1.0);
        lib.free(kept).expect("free");
    }

    #[test]
    fn clip_reports_sides_without_freeing() {
        let lib = CollisionWindingLibrary::new(None);
        let winding = square(&lib);
        let split = lib
            .clip(
                &winding,
                &Plane {
                    normal: vec3(1.0, 0.0, 0.0),
                    distance: 0.0,
                },
                ON_EPSILON,
            )
            .expect("clip");
        assert_eq!(split.front.expect("front").num_points().expect("count"), 4);
        assert_eq!(split.back.expect("back").num_points().expect("count"), 4);
        assert_eq!(winding.num_points().expect("input kept"), 4);
        assert_eq!(
            lib.on_plane_side(
                &winding,
                &Plane {
                    normal: vec3(1.0, 0.0, 0.0),
                    distance: 0.0,
                }
            )
            .expect("side"),
            WindingSide::Cross
        );
    }

    #[test]
    fn helpers_cover_bounds_center_copy_reverse() {
        let lib = CollisionWindingLibrary::new(None);
        let winding = square(&lib);
        let bounds = lib.bounds(&winding).expect("bounds");
        assert_eq!(bounds.min, vec3(-1.0, -1.0, 0.0));
        assert_eq!(bounds.max, vec3(1.0, 1.0, 0.0));
        assert_eq!(lib.center(&winding).expect("center"), vec3(0.0, 0.0, 0.0));
        let face = lib.plane(&winding).expect("plane");
        assert_eq!(face.normal, vec3(0.0, 0.0, -1.0));
        assert_eq!(face.distance, 0.0);
        assert_eq!(lib.area(&winding).expect("area"), 4.0);
        let copy = lib.copy(&winding).expect("copy");
        assert_eq!(copy.points().expect("points"), winding.points().expect("points"));
        let reversed = lib.reverse(&winding).expect("reverse");
        assert_eq!(reversed.point(0).expect("point"), vec3(-1.0, 1.0, 0.0));
    }

    #[test]
    fn colinear_points_are_removed() {
        let lib = CollisionWindingLibrary::new(None);
        let winding = lib.alloc(4).expect("alloc");
        for point in [
            vec3(0.0, 0.0, 0.0),
            vec3(1.0, 0.0, 0.0),
            vec3(2.0, 0.0, 0.0),
            vec3(0.0, 1.0, 0.0),
        ] {
            winding.append(point).expect("append");
        }
        lib.remove_colinear_points(&winding).expect("remove");
        assert_eq!(winding.num_points().expect("count"), 3);
        assert_eq!(lib.c_removed.get(), 1);
    }

    #[test]
    fn check_rejects_degenerate_windings() {
        let lib = CollisionWindingLibrary::new(None);
        let winding = lib.alloc(2).expect("alloc");
        winding.append(vec3(0.0, 0.0, 0.0)).expect("append");
        winding.append(vec3(1.0, 0.0, 0.0)).expect("append");
        let error = lib.check(&winding).expect_err("two points must fail");
        assert_eq!(error.to_string(), "CheckWinding: 2 points");
    }

    #[test]
    fn print_winding_matches_source_format() {
        let lib = CollisionWindingLibrary::new(None);
        let winding = square(&lib);
        let out = StdRefCell::new(String::new());
        lib.pw(&winding, &|text| out.borrow_mut().push_str(text))
            .expect("print");
        assert_eq!(
            out.borrow().as_str(),
            "( -1.0,  -1.0,   0.0)\n(  1.0,  -1.0,   0.0)\n(  1.0,   1.0,   0.0)\n( -1.0,   1.0,   0.0)\n"
        );
    }

    #[test]
    fn zone_hooks_observe_allocations() {
        struct Hooks {
            allocated: StdRefCell<usize>,
            freed: StdRefCell<usize>,
        }
        impl CollisionWindingMemory for Hooks {
            fn allocate(&self, bytes: usize) -> ZoneAllocation {
                *self.allocated.borrow_mut() += 1;
                ZoneAllocation { bytes: vec![9; bytes] }
            }
            fn free(&self, allocation: ZoneAllocation) {
                assert_eq!(
                    u32::from_le_bytes(allocation.bytes[0..4].try_into().unwrap()),
                    FREED_STAMP
                );
                *self.freed.borrow_mut() += 1;
            }
        }
        let hooks = Rc::new(Hooks {
            allocated: StdRefCell::new(0),
            freed: StdRefCell::new(0),
        });
        let lib = CollisionWindingLibrary::new(Some(hooks.clone()));
        let winding = lib.alloc(1).expect("alloc");
        assert_eq!(winding.point(0).expect("zeroed"), vec3(0.0, 0.0, 0.0));
        lib.free(winding).expect("free");
        assert_eq!(*hooks.allocated.borrow(), 1);
        assert_eq!(*hooks.freed.borrow(), 1);
    }

    #[test]
    fn hull_fold_starts_from_nothing() {
        let lib = CollisionWindingLibrary::new(None);
        let winding = square(&lib);
        let hull = lib
            .add_to_convex_hull(&winding, None, vec3(0.0, 0.0, 1.0))
            .expect("seed hull");
        assert_eq!(hull.num_points().expect("count"), 4);
    }
}
