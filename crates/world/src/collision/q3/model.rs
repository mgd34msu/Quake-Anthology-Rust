//! Owned temporary collision hulls translated from id Software's
//! `cm_load.c` and `cm_trace.c`.
//!
//! Donor provenance: `/home/buzzkill/Projects/quake-typescript/src/world/collision/q3/model.ts`.

use std::cell::Cell;

use qa_core::math::{add3, angles_to_axis, dot3, length3, scale3, sub3, vec3, Bounds, Vec3};
use qa_core::numeric::{bits_to_float32, float32_to_bits};

use super::counters::CollisionCounters;
use super::map_resource::{CollisionBoxHull, BODY_CONTENTS};
use super::world::{
    empty_source_trace, js_max, js_min, source_trace_end, source_trace_view, CollisionWorld, ModelTransform,
    SourceTracePlane, SourceTraceResult, TraceQuery, TraceResult, TraceShape,
};
use crate::error::WorldError;

/// Trace query against a temporary model (never names a submodel).
pub use super::world::TraceQuery as TemporaryTraceQuery;

/// Temporary model kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TempModelKind {
    /// Box hull.
    Box,
    /// Capsule hull.
    Capsule,
}

/// Borrowed box-hull storage plus the map visit stamp it shares.
#[derive(Debug, Clone, Copy)]
pub struct TempHull<'a> {
    /// Box hull records.
    pub hull: &'a CollisionBoxHull,
    /// Map check count for brush dedup.
    pub map_check_count: &'a Cell<i32>,
}

#[derive(Debug, Clone, Copy)]
struct PreparedShape {
    center: Vec3,
    mins: Vec3,
    extents: Vec3,
    capsule: bool,
    radius: f32,
    halfheight: f32,
    offset: Vec3,
}

#[derive(Debug, Clone, Copy)]
enum TemporaryModelData {
    Box { bounds: Bounds },
    Capsule { bounds: Bounds, target: PreparedShape },
}

struct TraceWork {
    fraction: f32,
    all_solid: bool,
    start_solid: bool,
    plane: SourceTracePlane,
    surface_flags: i32,
    contents: i32,
}

fn finite(point: Vec3) -> bool {
    point.x.is_finite() && point.y.is_finite() && point.z.is_finite()
}

fn validate_bounds(bounds: &Bounds) -> Result<(), WorldError> {
    if !finite(bounds.min)
        || !finite(bounds.max)
        || bounds.min.x > bounds.max.x
        || bounds.min.y > bounds.max.y
        || bounds.min.z > bounds.max.z
    {
        return Err(WorldError::BadCollisionRecord(
            "temporary hull requires finite ordered bounds".to_string(),
        ));
    }
    Ok(())
}

fn prepare(shape: &TraceShape) -> Result<PreparedShape, WorldError> {
    match shape {
        TraceShape::Point => Ok(PreparedShape {
            center: vec3(0.0, 0.0, 0.0),
            mins: vec3(0.0, 0.0, 0.0),
            extents: vec3(0.0, 0.0, 0.0),
            capsule: false,
            radius: 0.0,
            halfheight: 0.0,
            offset: vec3(0.0, 0.0, 0.0),
        }),
        TraceShape::Box { mins, maxs } | TraceShape::Capsule { mins, maxs } => {
            validate_bounds(&Bounds { min: *mins, max: *maxs })?;
            let center = scale3(add3(*mins, *maxs), 0.5);
            let mins = sub3(*mins, center);
            let extents = sub3(*maxs, center);
            let radius = js_min(extents.x, extents.z);
            Ok(PreparedShape {
                center,
                mins,
                extents,
                capsule: matches!(shape, TraceShape::Capsule { .. }),
                radius,
                halfheight: extents.z,
                offset: vec3(0.0, 0.0, extents.z - radius),
            })
        }
    }
}

fn computed_box_plane(bounds: &Bounds, index: usize) -> Result<SourceTracePlane, WorldError> {
    match index {
        0 => Ok(SourceTracePlane {
            normal: vec3(1.0, 0.0, 0.0),
            distance: bounds.max.x,
            plane_type: 0,
            signbits: 0,
        }),
        1 => Ok(SourceTracePlane {
            normal: vec3(-1.0, 0.0, 0.0),
            distance: -bounds.min.x,
            plane_type: 3,
            signbits: 1,
        }),
        2 => Ok(SourceTracePlane {
            normal: vec3(0.0, 1.0, 0.0),
            distance: bounds.max.y,
            plane_type: 1,
            signbits: 0,
        }),
        3 => Ok(SourceTracePlane {
            normal: vec3(0.0, -1.0, 0.0),
            distance: -bounds.min.y,
            plane_type: 4,
            signbits: 2,
        }),
        4 => Ok(SourceTracePlane {
            normal: vec3(0.0, 0.0, 1.0),
            distance: bounds.max.z,
            plane_type: 2,
            signbits: 0,
        }),
        5 => Ok(SourceTracePlane {
            normal: vec3(0.0, 0.0, -1.0),
            distance: -bounds.min.z,
            plane_type: 5,
            signbits: 4,
        }),
        _ => Err(WorldError::BadCollisionRecord(
            "computed temporary box side outside six planes".to_string(),
        )),
    }
}

#[allow(clippy::too_many_arguments)]
fn trace_box(
    work: &mut TraceWork,
    bounds: &Bounds,
    start: Vec3,
    end: Vec3,
    shape: &PreparedShape,
    stationary: bool,
    mask: i32,
    counters: &CollisionCounters,
    storage: Option<&TempHull>,
    retained_position_bounds: Option<&Bounds>,
) -> Result<(), WorldError> {
    if let Some(storage) = storage {
        if storage.hull.brush_check_count.get() == storage.map_check_count.get() {
            return Ok(());
        }
        storage.hull.brush_check_count.set(storage.map_check_count.get());
    }
    let contents = BODY_CONTENTS;
    if mask & contents == 0 {
        return Ok(());
    }
    if stationary {
        let (min, max) = match retained_position_bounds {
            Some(retained) => (retained.min, retained.max),
            None if shape.capsule => {
                // The donor rounds each bound once from binary64.
                let grow = |at: f32, span: f32| -> (f32, f32) {
                    (
                        (f64::from(at) - f64::from(span) - f64::from(shape.radius)) as f32,
                        (f64::from(at) + f64::from(span) + f64::from(shape.radius)) as f32,
                    )
                };
                let (min_x, max_x) = grow(start.x, shape.offset.x.abs());
                let (min_y, max_y) = grow(start.y, shape.offset.y.abs());
                let (min_z, max_z) = grow(start.z, shape.offset.z.abs());
                (vec3(min_x, min_y, min_z), vec3(max_x, max_y, max_z))
            }
            None => (add3(start, shape.mins), add3(start, shape.extents)),
        };
        let brush_bounds = match storage {
            Some(storage) => storage.hull.brush_bounds.get(),
            None => *bounds,
        };
        if min.x > brush_bounds.max.x
            || min.y > brush_bounds.max.y
            || min.z > brush_bounds.max.z
            || max.x < brush_bounds.min.x
            || max.y < brush_bounds.min.y
            || max.z < brush_bounds.min.z
        {
            return Ok(());
        }
    }
    if !stationary {
        CollisionCounters::bump(&counters.c_brush_traces);
    }
    let mut enter = -1.0f32;
    let mut leave = 1.0f32;
    let mut start_out = false;
    let mut get_out = false;
    let mut lead: Option<(SourceTracePlane, i32)> = None;
    for index in (if stationary { 6 } else { 0 })..6 {
        let (plane, surface_flags) = match storage {
            None => (computed_box_plane(bounds, index)?, 0),
            Some(storage) => {
                let side = storage.hull.read_side(index)?;
                (
                    SourceTracePlane {
                        normal: side.plane.normal,
                        distance: side.plane.distance,
                        plane_type: side.plane.plane_type,
                        signbits: side.plane.signbits,
                    },
                    side.surface_flags,
                )
            }
        };
        let normal = plane.normal;
        let mut expansion = shape.radius;
        if !shape.capsule {
            if plane.signbits >= 8 {
                return Err(WorldError::BadCollisionRecord(
                    "CM brush plane signbits outside trace offsets".to_string(),
                ));
            }
            expansion = -dot3(
                normal,
                vec3(
                    if plane.signbits & 1 != 0 {
                        shape.extents.x
                    } else {
                        shape.mins.x
                    },
                    if plane.signbits & 2 != 0 {
                        shape.extents.y
                    } else {
                        shape.mins.y
                    },
                    if plane.signbits & 4 != 0 {
                        shape.extents.z
                    } else {
                        shape.mins.z
                    },
                ),
            );
        }
        let distance = plane.distance + expansion;
        let (mut first, mut last) = (start, end);
        if shape.capsule {
            let offset = if dot3(normal, shape.offset) > 0.0 {
                scale3(shape.offset, -1.0)
            } else {
                shape.offset
            };
            first = add3(start, offset);
            last = add3(end, offset);
        }
        let d1 = dot3(first, normal) - distance;
        if stationary {
            if d1 > 0.0 {
                return Ok(());
            }
            continue;
        }
        let d2 = dot3(last, normal) - distance;
        if d1 > 0.0 {
            start_out = true;
        }
        if d2 > 0.0 {
            get_out = true;
        }
        if d1 > 0.0 && (d2 >= 0.125 || d2 >= d1) {
            return Ok(());
        }
        if d1 <= 0.0 && d2 <= 0.0 {
            continue;
        }
        if d1 > d2 {
            let crossed = 0.0f32.max((d1 - 0.125) / (d1 - d2));
            if crossed > enter {
                enter = crossed;
                lead = Some((plane, surface_flags));
            }
        } else {
            leave = leave.min(1.0f32.min((d1 + 0.125) / (d1 - d2)));
        }
    }
    if !start_out {
        work.start_solid = true;
        if !get_out {
            work.all_solid = true;
            work.fraction = 0.0;
            work.contents = contents;
        }
    } else if enter < leave && enter > -1.0 && enter < work.fraction && lead.is_some() {
        let (plane, surface_flags) = lead.expect("lead checked above");
        work.fraction = enter.max(0.0);
        work.plane = SourceTracePlane {
            normal: plane.normal,
            distance: plane.distance,
            plane_type: plane.plane_type,
            signbits: plane.signbits,
        };
        work.surface_flags = surface_flags;
        work.contents = contents;
    }
    Ok(())
}

/// The source capsule quadratic uses two inverse-square-root Newton steps.
fn capsule_square_root(number: f32) -> f32 {
    let x = number * 0.5;
    let mut y = bits_to_float32(0x5f37_59dfu32.wrapping_sub(float32_to_bits(number) >> 1));
    y *= 1.5 - (x * y) * y;
    y *= 1.5 - (x * y) * y;
    number * y
}

fn distance_from_line_squared(point: Vec3, start: Vec3, end: Vec3, direction: Vec3) -> f32 {
    let projection = add3(start, scale3(direction, dot3(sub3(point, start), direction)));
    for axis in 0..3 {
        let (projected, first, last) = match axis {
            0 => (projection.x, start.x, end.x),
            1 => (projection.y, start.y, end.y),
            _ => (projection.z, start.z, end.z),
        };
        if (projected > first && projected > last) || (projected < first && projected < last) {
            let delta = sub3(
                point,
                if (projected - first).abs() < (projected - last).abs() {
                    start
                } else {
                    end
                },
            );
            return dot3(delta, delta);
        }
    }
    let delta = sub3(point, projection);
    dot3(delta, delta)
}

#[allow(clippy::too_many_arguments)]
fn trace_rounded(
    work: &mut TraceWork,
    origin: Vec3,
    radius: f32,
    halfheight: Option<f32>,
    start: Vec3,
    end: Vec3,
    model_origin: Vec3,
) {
    let cylinder = halfheight.is_some();
    let start2d = if cylinder { vec3(start.x, start.y, 0.0) } else { start };
    let end2d = if cylinder { vec3(end.x, end.y, 0.0) } else { end };
    let origin2d = if cylinder {
        vec3(origin.x, origin.y, 0.0)
    } else {
        origin
    };
    let delta = sub3(start2d, origin2d);
    let within_caps = halfheight.is_none_or(|half| start.z <= origin.z + half && start.z >= origin.z - half);
    if within_caps && dot3(delta, delta) < radius * radius {
        work.fraction = 0.0;
        work.start_solid = true;
        let end_delta = sub3(end2d, origin2d);
        if dot3(end_delta, end_delta) < radius * radius {
            work.all_solid = true;
        }
        return;
    }
    let movement = sub3(end2d, start2d);
    let length = length3(movement);
    let direction = if length == 0.0 {
        vec3(0.0, 0.0, 0.0)
    } else {
        scale3(movement, 1.0 / length)
    };
    let closest = distance_from_line_squared(origin2d, start2d, end2d, direction);
    let end_delta = sub3(end2d, origin2d);
    let near_radius = radius + 0.125;
    if closest >= radius * radius && dot3(end_delta, end_delta) > near_radius * near_radius {
        return;
    }
    let inflated = radius + 1.0;
    let b = 2.0 * dot3(direction, delta);
    let c = dot3(delta, delta) - inflated * inflated;
    let determinant = b * b - 4.0 * c;
    if determinant <= 0.0 {
        return;
    }
    let mut fraction = (-b - capsule_square_root(determinant)) * 0.5;
    fraction = if fraction < 0.0 { 0.0 } else { fraction / length };
    if fraction.is_nan() || fraction >= work.fraction {
        return;
    }
    let intersection = source_trace_end(start, end, f64::from(fraction));
    if let Some(half) = halfheight {
        if intersection.z > origin.z + half || intersection.z < origin.z - half {
            return;
        }
    }
    let mut normal = sub3(intersection, origin);
    if cylinder {
        normal = vec3(normal.x, normal.y, 0.0);
    }
    normal = scale3(normal, 1.0 / inflated);
    work.fraction = fraction;
    work.plane.normal = normal;
    work.plane.distance = dot3(normal, add3(model_origin, intersection));
    work.contents = BODY_CONTENTS;
}

fn position_capsule(work: &mut TraceWork, target: &PreparedShape, start: Vec3, shape: &PreparedShape) {
    let top = add3(start, shape.offset);
    let bottom = sub3(start, shape.offset);
    let upper = add3(target.center, target.offset);
    let lower = sub3(target.center, target.offset);
    let radius = shape.radius + target.radius;
    let squared = radius * radius;
    for endpoint in [top, bottom] {
        for target_endpoint in [upper, lower] {
            let delta = sub3(target_endpoint, endpoint);
            if dot3(delta, delta) < squared {
                work.all_solid = true;
                work.start_solid = true;
                work.fraction = 0.0;
            }
        }
    }
    // Preserve CM_TestCapsuleInCapsule's original upper/lower comparison order.
    if (top.z >= upper.z && top.z <= lower.z) || (bottom.z >= upper.z && bottom.z <= lower.z) {
        let delta = vec3(top.x - upper.x, top.y - upper.y, 0.0);
        if dot3(delta, delta) < squared {
            work.all_solid = true;
            work.start_solid = true;
            work.fraction = 0.0;
        }
    }
}

/// Temporary box/capsule collision model.
#[derive(Debug)]
pub struct TemporaryCollisionModel<'a> {
    model: TemporaryModelData,
    counters: &'a CollisionCounters,
    storage: Option<TempHull<'a>>,
}

impl<'a> TemporaryCollisionModel<'a> {
    /// Wrap owned bounds with shared counters and optional box storage.
    pub fn new(
        kind: TempModelKind,
        bounds: Bounds,
        counters: &'a CollisionCounters,
        storage: Option<TempHull<'a>>,
    ) -> Result<Self, WorldError> {
        let owned = Bounds {
            min: vec3(bounds.min.x, bounds.min.y, bounds.min.z),
            max: vec3(bounds.max.x, bounds.max.y, bounds.max.z),
        };
        let model = match kind {
            TempModelKind::Box => {
                // CM_TempBoxModel stores raw plane distances, including CG's
                // transient zero-solid hull.
                if !finite(owned.min) || !finite(owned.max) {
                    return Err(WorldError::BadCollisionRecord(
                        "temporary box requires finite plane distances".to_string(),
                    ));
                }
                TemporaryModelData::Box { bounds: owned }
            }
            TempModelKind::Capsule => {
                validate_bounds(&bounds)?;
                TemporaryModelData::Capsule {
                    bounds: owned,
                    target: prepare(&TraceShape::Capsule {
                        mins: owned.min,
                        maxs: owned.max,
                    })?,
                }
            }
        };
        Ok(Self {
            model,
            counters,
            storage,
        })
    }

    /// Model kind.
    #[must_use]
    pub fn kind(&self) -> TempModelKind {
        match &self.model {
            TemporaryModelData::Box { .. } => TempModelKind::Box,
            TemporaryModelData::Capsule { .. } => TempModelKind::Capsule,
        }
    }

    /// Model bounds: live box-brush bounds when storage is borrowed.
    #[must_use]
    pub fn bounds(&self) -> Bounds {
        match (&self.model, &self.storage) {
            (TemporaryModelData::Box { .. }, Some(storage)) => storage.hull.brush_bounds.get(),
            (TemporaryModelData::Box { bounds }, None) => *bounds,
            (TemporaryModelData::Capsule { bounds, .. }, _) => *bounds,
        }
    }

    /// Contents at a point against the temporary box brush.
    pub fn point_contents(&self, point: Vec3) -> Result<i32, WorldError> {
        if !finite(point) {
            return Err(WorldError::BadCollisionRecord(
                "point contents requires finite coordinates".to_string(),
            ));
        }
        if let Some(storage) = &self.storage {
            let mut index = 0;
            while index < storage.hull.brush_side_count() {
                let plane = storage.hull.read_side(index)?.plane;
                if dot3(point, plane.normal) > plane.distance {
                    break;
                }
                index += 1;
            }
            return Ok(if index == storage.hull.brush_side_count() {
                BODY_CONTENTS
            } else {
                0
            });
        }
        // The source point-contents path tests the temporary box brush,
        // including capsule handles. Each owned model initializes that brush
        // from its bounds.
        let bounds = match &self.model {
            TemporaryModelData::Box { bounds } => bounds,
            TemporaryModelData::Capsule { bounds, .. } => bounds,
        };
        Ok(
            if point.x >= bounds.min.x
                && point.x <= bounds.max.x
                && point.y >= bounds.min.y
                && point.y <= bounds.max.y
                && point.z >= bounds.min.z
                && point.z <= bounds.max.z
            {
                BODY_CONTENTS
            } else {
                0
            },
        )
    }

    /// Contents at a point against a transformed temporary model.
    pub fn transformed_point_contents(&self, point: Vec3, origin: Vec3, angles: Vec3) -> Result<i32, WorldError> {
        if !finite(origin) || !finite(angles) {
            return Err(WorldError::BadCollisionRecord(
                "model transform requires finite coordinates".to_string(),
            ));
        }
        let mut local = sub3(point, origin);
        if self.kind() == TempModelKind::Capsule {
            let axis = angles_to_axis(angles);
            local = vec3(dot3(local, axis[0]), dot3(local, axis[1]), dot3(local, axis[2]));
        }
        self.point_contents(local)
    }

    /// Trace a sweep and project its source record.
    pub fn trace(&self, query: &TraceQuery) -> Result<TraceResult, WorldError> {
        Ok(source_trace_view(&self.trace_source(query)?))
    }

    /// Raw source trace against this model.
    pub fn trace_source(&self, query: &TraceQuery) -> Result<SourceTraceResult, WorldError> {
        self.source(query, None, None)
    }

    /// Transformed sweep projected into its trace view.
    pub fn transformed_trace(&self, query: &TraceQuery, origin: Vec3, angles: Vec3) -> Result<TraceResult, WorldError> {
        Ok(source_trace_view(
            &self.transformed_trace_source(query, origin, angles)?,
        ))
    }

    /// Raw source trace against a transformed temporary model.
    pub fn transformed_trace_source(
        &self,
        query: &TraceQuery,
        origin: Vec3,
        angles: Vec3,
    ) -> Result<SourceTraceResult, WorldError> {
        self.source(query, Some(&ModelTransform { origin, angles }), None)
    }

    /// The source capsule swap can resolve its box handle to actual
    /// submodel 255.
    pub fn trace_capsule_replacement_source(
        &self,
        query: &TraceQuery,
        world: &CollisionWorld,
        transform: Option<&ModelTransform>,
    ) -> Result<SourceTraceResult, WorldError> {
        self.source(query, transform, Some(world))
    }

    fn source(
        &self,
        query: &TraceQuery,
        transform: Option<&ModelTransform>,
        replacement: Option<&CollisionWorld>,
    ) -> Result<SourceTraceResult, WorldError> {
        let origin = transform.map_or(vec3(0.0, 0.0, 0.0), |transform| transform.origin);
        let angles = transform.map_or(vec3(0.0, 0.0, 0.0), |transform| transform.angles);
        if transform.is_some() && (!finite(origin) || !finite(angles)) {
            return Err(WorldError::BadCollisionRecord(
                "model transform requires finite coordinates".to_string(),
            ));
        }
        let mut shape = prepare(&query.shape)?;
        let axis = if self.kind() != TempModelKind::Box && (angles.x != 0.0 || angles.y != 0.0 || angles.z != 0.0) {
            Some(angles_to_axis(angles))
        } else {
            None
        };
        if let Some(axis) = &axis {
            shape.offset = vec3(
                axis[0].z * shape.offset.z,
                -axis[1].z * shape.offset.z,
                axis[2].z * shape.offset.z,
            );
        }
        let centered_start = add3(query.start, shape.center);
        let centered_end = add3(query.end, shape.center);
        let rotate = |point: Vec3| -> Vec3 {
            match &axis {
                None => point,
                Some(axis) => vec3(dot3(point, axis[0]), dot3(point, axis[1]), dot3(point, axis[2])),
            }
        };
        let mut start = match transform {
            None => centered_start,
            Some(_) => rotate(sub3(centered_start, origin)),
        };
        let mut finish = match transform {
            None => centered_end,
            Some(_) => rotate(sub3(centered_end, origin)),
        };
        let stationary = match transform {
            None => query.start.x == query.end.x && query.start.y == query.end.y && query.start.z == query.end.z,
            Some(_) => start.x == finish.x && start.y == finish.y && start.z == finish.z,
        };
        if transform.is_some() {
            // CM_Trace centers CM_TransformedBoxTrace's stored bounds a second time.
            let center = scale3(add3(shape.mins, shape.extents), 0.5);
            shape.mins = sub3(shape.mins, center);
            shape.extents = sub3(shape.extents, center);
            start = add3(start, center);
            finish = add3(finish, center);
        }
        let result = self.trace_inner(query, start, finish, &shape, stationary, origin, replacement)?;
        if transform.is_none() {
            return Ok(result);
        }
        let end = source_trace_end(query.start, query.end, f64::from(result.fraction));
        let Some(axis) = axis else {
            return Ok(SourceTraceResult { end, ..result });
        };
        if result.fraction == 1.0 {
            return Ok(SourceTraceResult { end, ..result });
        }
        let normal = result.plane.normal;
        let rotated = vec3(
            (f64::from(axis[0].x * normal.x) + f64::from(axis[1].x * normal.y)) as f32 + axis[2].x * normal.z,
            (f64::from(axis[0].y * normal.x) + f64::from(axis[1].y * normal.y)) as f32 + axis[2].y * normal.z,
            (f64::from(axis[0].z * normal.x) + f64::from(axis[1].z * normal.y)) as f32 + axis[2].z * normal.z,
        );
        Ok(SourceTraceResult {
            end,
            plane: SourceTracePlane {
                normal: rotated,
                ..result.plane
            },
            ..result
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn trace_inner(
        &self,
        query: &TraceQuery,
        start: Vec3,
        end: Vec3,
        shape: &PreparedShape,
        stationary: bool,
        model_origin: Vec3,
        replacement: Option<&CollisionWorld>,
    ) -> Result<SourceTraceResult, WorldError> {
        if !finite(query.start) || !finite(query.end) {
            return Err(WorldError::BadCollisionRecord(
                "trace requires finite coordinates and integer contents mask".to_string(),
            ));
        }
        CollisionCounters::bump(&self.counters.c_traces);
        let mut work = TraceWork {
            fraction: 1.0,
            all_solid: false,
            start_solid: false,
            plane: empty_source_trace().plane,
            surface_flags: 0,
            contents: 0,
        };
        match &self.model {
            TemporaryModelData::Box { bounds } => {
                trace_box(
                    &mut work,
                    bounds,
                    start,
                    end,
                    shape,
                    stationary,
                    query.mask,
                    self.counters,
                    self.storage.as_ref(),
                    None,
                )?;
            }
            TemporaryModelData::Capsule { bounds, target } => {
                if !shape.capsule {
                    // CM_TraceBoundingBoxThroughCapsule swaps the stationary
                    // box and capsule.
                    if let Some(world) = replacement {
                        return world.trace_capsule_replacement_source(
                            query,
                            &super::world::CapsuleReplacementTrace {
                                start: sub3(start, target.center),
                                end: sub3(end, target.center),
                                mins: shape.mins,
                                extents: shape.extents,
                                radius: target.radius,
                                offset: target.offset,
                                bounds: Bounds {
                                    min: add3(start, shape.mins),
                                    max: add3(start, shape.extents),
                                },
                                stationary,
                                point_trace: shape.mins.x == 0.0 && shape.mins.y == 0.0 && shape.mins.z == 0.0,
                            },
                        );
                    } else if let Some(storage) = &self.storage {
                        trace_box(
                            &mut work,
                            &storage.hull.brush_bounds.get(),
                            sub3(start, target.center),
                            sub3(end, target.center),
                            target,
                            stationary,
                            query.mask,
                            self.counters,
                            Some(storage),
                            Some(&Bounds {
                                min: add3(start, shape.mins),
                                max: add3(start, shape.extents),
                            }),
                        )?;
                    } else if stationary {
                        // CM_TestBoundingBoxInCapsule retains the original
                        // trace bounds while replacing the brush with
                        // size[0]/size[1]. CM_TestBoxInBrush tests those
                        // bounds and skips all six axial planes; retain this
                        // stationary source case.
                        let min = add3(start, shape.mins);
                        let max = add3(start, shape.extents);
                        if query.mask & BODY_CONTENTS != 0
                            && min.x <= shape.extents.x
                            && min.y <= shape.extents.y
                            && min.z <= shape.extents.z
                            && max.x >= shape.mins.x
                            && max.y >= shape.mins.y
                            && max.z >= shape.mins.z
                        {
                            work.fraction = 0.0;
                            work.all_solid = true;
                            work.start_solid = true;
                            work.contents = BODY_CONTENTS;
                        }
                    } else {
                        trace_box(
                            &mut work,
                            &Bounds {
                                min: shape.mins,
                                max: shape.extents,
                            },
                            sub3(start, target.center),
                            sub3(end, target.center),
                            target,
                            stationary,
                            query.mask,
                            self.counters,
                            None,
                            None,
                        )?;
                    }
                } else if stationary {
                    position_capsule(&mut work, target, start, shape);
                } else {
                    // The donor rounds each sweep bound once from binary64.
                    let sweep = |at: f32, to: f32, span: f32| -> (f32, f32) {
                        (
                            (f64::from(js_min(at, to)) - f64::from(span) - f64::from(shape.radius)) as f32,
                            (f64::from(js_max(at, to)) + f64::from(span) + f64::from(shape.radius)) as f32,
                        )
                    };
                    let (min_x, max_x) = sweep(start.x, end.x, shape.offset.x.abs());
                    let (min_y, max_y) = sweep(start.y, end.y, shape.offset.y.abs());
                    let (min_z, max_z) = sweep(start.z, end.z, shape.offset.z.abs());
                    let sweep_min = vec3(min_x, min_y, min_z);
                    let sweep_max = vec3(max_x, max_y, max_z);
                    if !(sweep_min.x > bounds.max.x + 1.0
                        || sweep_min.y > bounds.max.y + 1.0
                        || sweep_min.z > bounds.max.z + 1.0
                        || sweep_max.x < bounds.min.x - 1.0
                        || sweep_max.y < bounds.min.y - 1.0
                        || sweep_max.z < bounds.min.z - 1.0)
                    {
                        let radius = target.radius + shape.radius;
                        let halfheight = target.halfheight + shape.halfheight - radius;
                        if (start.x != end.x || start.y != end.y) && halfheight > 0.0 {
                            trace_rounded(
                                &mut work,
                                target.center,
                                radius,
                                Some(halfheight),
                                start,
                                end,
                                model_origin,
                            );
                        }
                        trace_rounded(
                            &mut work,
                            add3(target.center, target.offset),
                            radius,
                            None,
                            sub3(start, shape.offset),
                            sub3(end, shape.offset),
                            model_origin,
                        );
                        trace_rounded(
                            &mut work,
                            sub3(target.center, target.offset),
                            radius,
                            None,
                            add3(start, shape.offset),
                            add3(end, shape.offset),
                            model_origin,
                        );
                    }
                }
            }
        }
        Ok(SourceTraceResult {
            fraction: work.fraction,
            end: if work.fraction == 1.0 {
                query.end
            } else {
                source_trace_end(query.start, query.end, f64::from(work.fraction))
            },
            all_solid: work.all_solid,
            start_solid: work.start_solid,
            plane: work.plane,
            surface_flags: work.surface_flags,
            contents: work.contents,
        })
    }
}

/// Owned temporary box model.
pub fn create_box_model<'a>(
    bounds: Bounds,
    counters: &'a CollisionCounters,
    storage: Option<TempHull<'a>>,
) -> Result<TemporaryCollisionModel<'a>, WorldError> {
    TemporaryCollisionModel::new(TempModelKind::Box, bounds, counters, storage)
}

/// Owned temporary capsule model.
pub fn create_capsule_model<'a>(
    bounds: Bounds,
    counters: &'a CollisionCounters,
    storage: Option<TempHull<'a>>,
) -> Result<TemporaryCollisionModel<'a>, WorldError> {
    TemporaryCollisionModel::new(TempModelKind::Capsule, bounds, counters, storage)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    use std::rc::Rc;

    use super::super::map_resource::{CollisionBrushSide, CollisionPlane};

    const BODY: i32 = BODY_CONTENTS;

    fn counters() -> CollisionCounters {
        CollisionCounters::new()
    }

    fn boxed(mins: Vec3, maxs: Vec3) -> TraceShape {
        TraceShape::Box { mins, maxs }
    }

    fn capsulized(mins: Vec3, maxs: Vec3) -> TraceShape {
        TraceShape::Capsule { mins, maxs }
    }

    fn query(start: Vec3, end: Vec3, shape: TraceShape, mask: i32) -> TraceQuery {
        TraceQuery {
            start,
            end,
            shape,
            mask,
            model_index: None,
            curves: None,
            player_curve_clip: None,
        }
    }

    fn box_model(counters: &CollisionCounters) -> TemporaryCollisionModel<'_> {
        create_box_model(
            Bounds {
                min: vec3(-16.0, -16.0, -16.0),
                max: vec3(16.0, 16.0, 16.0),
            },
            counters,
            None,
        )
        .expect("box model")
    }

    fn capsule_model(counters: &CollisionCounters) -> TemporaryCollisionModel<'_> {
        create_capsule_model(
            Bounds {
                min: vec3(-8.0, -8.0, -24.0),
                max: vec3(8.0, 8.0, 24.0),
            },
            counters,
            None,
        )
        .expect("capsule model")
    }

    #[test]
    fn temp_box_traces_match_donor() {
        let counters = counters();
        let model = box_model(&counters);
        let hit = model
            .trace_source(&query(
                vec3(0.0, 0.0, 50.0),
                vec3(0.0, 0.0, -50.0),
                TraceShape::Point,
                BODY,
            ))
            .expect("trace");
        assert_eq!(hit.fraction, 0.3387500047683716f64 as f32);
        assert_eq!(hit.end, vec3(0.0, 0.0, 16.125));
        assert_eq!(hit.contents, BODY);
        assert_eq!(hit.plane.plane_type, 2);
        assert_eq!(hit.plane.signbits, 0);
        let small = boxed(vec3(-4.0, -4.0, -4.0), vec3(4.0, 4.0, 4.0));
        let hit = model
            .trace_source(&query(vec3(0.0, 0.0, 50.0), vec3(0.0, 0.0, -50.0), small, BODY))
            .expect("trace");
        assert_eq!(hit.fraction, 0.29875001311302185f64 as f32);
        assert_eq!(hit.end, vec3(0.0, 0.0, 20.124998092651367f64 as f32));
        let small = capsulized(vec3(-4.0, -4.0, -4.0), vec3(4.0, 4.0, 4.0));
        let hit = model
            .trace_source(&query(vec3(0.0, 0.0, 50.0), vec3(0.0, 0.0, -50.0), small, BODY))
            .expect("trace");
        assert_eq!(hit.fraction, 0.29875001311302185f64 as f32);
        assert_eq!(hit.end, vec3(0.0, 0.0, 20.124998092651367f64 as f32));
        let odd = boxed(vec3(-5.0, -3.0, -7.0), vec3(3.0, 6.0, 2.0));
        let hit = model
            .trace_source(&query(vec3(40.0, 30.0, 50.0), vec3(-40.0, -30.0, -50.0), odd, BODY))
            .expect("trace");
        assert_eq!(hit.fraction, 0.26875001192092896f64 as f32);
        assert_eq!(hit.end, vec3(18.5, 13.875, 23.124998092651367f64 as f32));
        let miss = model
            .trace_source(&query(
                vec3(0.0, 0.0, 50.0),
                vec3(0.0, 0.0, -50.0),
                TraceShape::Point,
                1,
            ))
            .expect("trace");
        assert_eq!(miss.fraction, 1.0);
        assert_eq!(miss.contents, 0);
        let stuck = model
            .trace_source(&query(
                vec3(0.0, 0.0, 0.0),
                vec3(0.0, 0.0, 0.0),
                TraceShape::Point,
                BODY,
            ))
            .expect("trace");
        assert!(stuck.all_solid && stuck.start_solid);
        assert_eq!(stuck.contents, BODY);
        let stuck = model
            .trace_source(&query(
                vec3(0.0, 0.0, 0.0),
                vec3(0.0, 0.0, 0.0),
                capsulized(vec3(-4.0, -4.0, -4.0), vec3(4.0, 4.0, 4.0)),
                BODY,
            ))
            .expect("trace");
        assert!(stuck.all_solid && stuck.start_solid);
        let free = model
            .trace_source(&query(
                vec3(0.0, 0.0, 50.0),
                vec3(0.0, 0.0, 50.0),
                boxed(vec3(-4.0, -4.0, -4.0), vec3(4.0, 4.0, 4.0)),
                BODY,
            ))
            .expect("trace");
        assert_eq!(free.fraction, 1.0);
        assert!(!free.start_solid);
        let shifted = model
            .transformed_trace_source(
                &query(vec3(0.0, 0.0, 50.0), vec3(0.0, 0.0, -50.0), TraceShape::Point, BODY),
                vec3(0.0, 0.0, 8.0),
                vec3(0.0, 30.0, 0.0),
            )
            .expect("trace");
        assert_eq!(shifted.fraction, 0.25874999165534973f64 as f32);
        assert_eq!(shifted.end, vec3(0.0, 0.0, 24.125));
    }

    #[test]
    fn temp_capsule_traces_match_donor() {
        let counters = counters();
        let model = capsule_model(&counters);
        // Capsule hulls ignore the contents mask, exactly like the donor.
        for shape in [TraceShape::Point, boxed(vec3(-4.0, -4.0, -4.0), vec3(4.0, 4.0, 4.0))] {
            let axial = model
                .trace_source(&query(vec3(0.0, 0.0, 60.0), vec3(0.0, 0.0, -60.0), shape, 1))
                .expect("trace");
            assert_eq!(axial.fraction, 1.0);
        }
        let small = capsulized(vec3(-4.0, -4.0, -4.0), vec3(4.0, 4.0, 4.0));
        let hit = model
            .trace_source(&query(vec3(30.0, 0.0, 60.0), vec3(-30.0, 0.0, -60.0), small, 1))
            .expect("trace");
        assert_eq!(hit.fraction, 0.31243565678596497f64 as f32);
        assert_eq!(
            hit.end,
            vec3(11.253860473632812f64 as f32, 0.0, 22.507720947265625f64 as f32)
        );
        assert_eq!(hit.contents, BODY);
        assert_eq!(
            hit.plane.normal,
            vec3(0.8656815886497498f64 as f32, 0.0, 0.5005939602851868f64 as f32)
        );
        assert_eq!(hit.plane.distance, 21.009489059448242f64 as f32);
        let odd = capsulized(vec3(-5.0, -3.0, -9.0), vec3(3.0, 6.0, 7.0));
        let hit = model
            .trace_source(&query(vec3(40.0, 30.0, 60.0), vec3(-40.0, -30.0, -60.0), odd, 1))
            .expect("trace");
        assert_eq!(hit.fraction, 0.37225285172462463f64 as f32);
        assert_eq!(
            hit.end,
            vec3(
                10.219772338867188f64 as f32,
                7.664829254150391f64 as f32,
                15.329658508300781f64 as f32
            )
        );
        assert_eq!(
            hit.plane.normal,
            vec3(0.7092132568359375f64 as f32, 0.7049868702888489f64 as f32, 0.0)
        );
        let stuck = model
            .trace_source(&query(vec3(0.0, 0.0, 0.0), vec3(0.0, 0.0, 0.0), TraceShape::Point, 1))
            .expect("trace");
        assert_eq!(stuck.fraction, 1.0);
        let moved = model
            .transformed_trace_source(
                &query(
                    vec3(0.0, 0.0, 60.0),
                    vec3(0.0, 0.0, -60.0),
                    boxed(vec3(-4.0, -4.0, -4.0), vec3(4.0, 4.0, 4.0)),
                    1,
                ),
                vec3(5.0, 0.0, 0.0),
                vec3(0.0, 0.0, 0.0),
            )
            .expect("trace");
        assert_eq!(moved.fraction, 1.0);
    }

    #[test]
    fn temp_models_cover_contents_validation_and_storage() {
        let counters = counters();
        let model = box_model(&counters);
        assert_eq!(model.kind(), TempModelKind::Box);
        assert_eq!(model.point_contents(vec3(0.0, 0.0, 0.0)).expect("in"), BODY);
        assert_eq!(model.point_contents(vec3(99.0, 0.0, 0.0)).expect("out"), 0);
        let capsule = capsule_model(&counters);
        assert_eq!(capsule.kind(), TempModelKind::Capsule);
        assert_eq!(capsule.point_contents(vec3(0.0, 0.0, 0.0)).expect("in"), BODY);
        assert_eq!(capsule.point_contents(vec3(0.0, 0.0, 99.0)).expect("out"), 0);
        let error = create_box_model(
            Bounds {
                min: vec3(f32::NAN, 0.0, 0.0),
                max: vec3(1.0, 1.0, 1.0),
            },
            &counters,
            None,
        )
        .expect_err("NaN bounds must fail");
        assert_eq!(error.to_string(), "temporary box requires finite plane distances");
        let hull = test_hull();
        hull.set_bounds(vec3(-16.0, -16.0, -16.0), vec3(16.0, 16.0, 16.0), false);
        let stamp = Cell::new(1);
        let stored = create_box_model(
            model.bounds(),
            &counters,
            Some(TempHull {
                hull: &hull,
                map_check_count: &stamp,
            }),
        )
        .expect("stored box");
        let hit = stored
            .trace_source(&query(
                vec3(0.0, 0.0, 50.0),
                vec3(0.0, 0.0, -50.0),
                TraceShape::Point,
                BODY,
            ))
            .expect("trace");
        assert_eq!(hit.fraction, 0.3387500047683716f64 as f32);
        assert_eq!(hit.end, vec3(0.0, 0.0, 16.125));
        assert_eq!(stored.point_contents(vec3(0.0, 0.0, 0.0)).expect("in"), BODY);
    }

    /// Box hull mirroring `initialize_box_hull` for storage-path coverage.
    fn test_hull() -> CollisionBoxHull {
        let mut planes = [CollisionPlane {
            normal: vec3(0.0, 0.0, 0.0),
            distance: 0.0,
            plane_type: 0,
            signbits: 0,
        }; 12];
        for index in 0..6 {
            for opposite in 0..2 {
                let axis = index >> 1;
                let sign = if opposite == 0 { 1.0 } else { -1.0 };
                planes[index * 2 + opposite] = CollisionPlane {
                    normal: vec3(
                        if axis == 0 { sign } else { 0.0 },
                        if axis == 1 { sign } else { 0.0 },
                        if axis == 2 { sign } else { 0.0 },
                    ),
                    distance: 0.0,
                    plane_type: axis as i32 + opposite as i32 * 3,
                    signbits: if opposite == 0 { 0 } else { 1 << axis },
                };
            }
        }
        let mut sides = [CollisionBrushSide {
            plane: 0,
            shader: 0,
            surface_flags: 0,
        }; 6];
        for (index, side) in sides.iter_mut().enumerate() {
            side.plane = index * 2 + (index & 1);
        }
        CollisionBoxHull {
            bounds: Rc::new(Cell::new(Bounds {
                min: vec3(0.0, 0.0, 0.0),
                max: vec3(0.0, 0.0, 0.0),
            })),
            brush_bounds: Cell::new(Bounds {
                min: vec3(0.0, 0.0, 0.0),
                max: vec3(0.0, 0.0, 0.0),
            }),
            brush_check_count: Cell::new(0),
            first_side: 0,
            sides,
            planes: Cell::new(planes),
        }
    }
}
