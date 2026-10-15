//! Rounded entity targets use the same body dispatcher and box/brush reducer.
use super::store::BodyTraceMode;
use super::{Trace, TraceError, TraceQuery, boxes::trace_convex_box, position as rounded};
use qa_core::{
    math,
    primitives::{Body, Bounds, Capsule, EntityId, Vec3},
};

const RADIUS_MARGIN: f32 = 1.0;

pub(super) struct CapsuleTrace {
    pub trace: Trace,
    pub local_plane: bool,
}

pub(super) fn trace_capsule(
    query: TraceQuery,
    body: &Body,
    entity: Option<EntityId>,
    model_origin: Vec3,
    scratch: &mut rounded::Scratch,
) -> Result<CapsuleTrace, TraceError> {
    let mut query = query;
    // The transformed wrapper and native kernel each recenter independently.
    query.center_box();
    let bounds = Bounds {
        mins: body.mins,
        maxs: body.maxs,
    };
    let target = Capsule::from_bounds(bounds);
    let center = math::midpoint(bounds.mins, bounds.maxs);
    let mut local_plane = false;
    let moving = query.capsule;
    let mut trace = if let Some(moving) = moving {
        if moving.offset.0[0] != 0.0 || moving.offset.0[1] != 0.0 {
            if !bounds.is_valid() {
                return Err(TraceError::Query);
            }
            let trace =
                rounded::trace_capsule_pair(query, center, target, scratch).map_err(|failure| {
                    match failure {
                        rounded::Failure::Scratch => TraceError::Scratch,
                        _ => TraceError::Query,
                    }
                })?;
            local_plane = trace.plane.is_valid();
            trace
        } else {
            let mut trace = Trace::clear(query.end);
            if query.start == query.end {
                position(query.start, moving, center, target, &mut trace);
            } else if query.swept_bounds(0.0).overlaps(Bounds {
                mins: bounds.mins - Vec3([RADIUS_MARGIN; 3]),
                maxs: bounds.maxs + Vec3([RADIUS_MARGIN; 3]),
            }) {
                let radius = target.radius + moving.radius;
                if query.start.0[0] != query.end.0[0] || query.start.0[1] != query.end.0[1] {
                    let half_height = (body.maxs.0[2] - center.0[2]) + query.maxs.0[2] - radius;
                    if half_height > 0.0 {
                        rounded_section(
                            &mut trace,
                            query,
                            center,
                            radius,
                            Some(half_height),
                            query.start,
                            query.end,
                            model_origin,
                        );
                    }
                }
                rounded_section(
                    &mut trace,
                    query,
                    center + target.offset,
                    radius,
                    None,
                    query.start - moving.offset,
                    query.end - moving.offset,
                    model_origin,
                );
                rounded_section(
                    &mut trace,
                    query,
                    center - target.offset,
                    radius,
                    None,
                    query.start + moving.offset,
                    query.end + moving.offset,
                    model_origin,
                );
            }
            trace
        }
    } else {
        // Native box/capsule collision swaps their shapes, retaining the
        // already computed query bounds for the temporary brush's cull.
        let culling_bounds = query.swept_bounds(0.0);
        query.start = query.start - center;
        query.end = query.end - center;
        query.capsule = Some(target);
        trace_convex_box(
            query,
            &Body {
                mins: query.mins,
                maxs: query.maxs,
                ..Body::default()
            },
            entity,
            BodyTraceMode::Direct,
            Some(culling_bounds),
        )?
    };
    if query.rules.reached_media && moving.is_some() {
        if !bounds.is_valid() {
            return Err(TraceError::Query);
        }
        rounded::capsule_pair_media(query, center, target, trace.fraction, scratch)
            .map_err(rounded::Failure::query_error)?
            .apply(&mut trace);
    }
    if trace.fraction < 1.0 || trace.start_solid {
        trace.entity = entity;
    }
    trace.brush_solid = false;
    Ok(CapsuleTrace { trace, local_plane })
}

fn position(start: Vec3, moving: Capsule, center: Vec3, target: Capsule, trace: &mut Trace) {
    let top = start + moving.offset;
    let bottom = start - moving.offset;
    let target_top = center + target.offset;
    let target_bottom = center - target.offset;
    let radius = moving.radius + target.radius;
    let radius_squared = radius * radius;
    let mut overlap = [
        target_top - top,
        target_top - bottom,
        target_bottom - top,
        target_bottom - bottom,
    ]
    .into_iter()
    .any(|delta| delta.dot(delta) < radius_squared);
    // Preserve CM_TestCapsuleInCapsule's native endpoint ordering and ties.
    if (top.0[2] >= target_top.0[2] && top.0[2] <= target_bottom.0[2])
        || (bottom.0[2] >= target_top.0[2] && bottom.0[2] <= target_bottom.0[2])
    {
        let delta = Vec3([top.0[0] - target_top.0[0], top.0[1] - target_top.0[1], 0.0]);
        overlap |= delta.dot(delta) < radius_squared;
    }
    if overlap {
        trace.start_solid = true;
        trace.all_solid = true;
        trace.fraction = 0.0;
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "One quadratic reducer serves the native spherical and cylindrical sections"
)]
fn rounded_section(
    trace: &mut Trace,
    query: TraceQuery,
    origin: Vec3,
    radius: f32,
    half_height: Option<f32>,
    start: Vec3,
    end: Vec3,
    model_origin: Vec3,
) {
    let project = |point: Vec3| {
        if half_height.is_some() {
            Vec3([point.0[0], point.0[1], 0.0])
        } else {
            point
        }
    };
    let start_projected = project(start);
    let end_projected = project(end);
    let origin_projected = project(origin);
    let mut direction = start_projected - origin_projected;
    let within_height = half_height.is_none_or(|height| {
        start.0[2] <= origin.0[2] + height && start.0[2] >= origin.0[2] - height
    });
    if within_height && direction.dot(direction) < radius * radius {
        trace.fraction = 0.0;
        trace.start_solid = true;
        direction = end_projected - origin_projected;
        if direction.dot(direction) < radius * radius {
            trace.all_solid = true;
        }
        return;
    }
    direction = end_projected - start_projected;
    let length = math::normalize(&mut direction);
    let nearest =
        math::segment_distance_squared(origin_projected, start_projected, end_projected, direction);
    let end_delta = end_projected - origin_projected;
    let near_radius = radius + query.rules.contact_epsilon as f32;
    if nearest >= radius * radius && end_delta.dot(end_delta) > near_radius * near_radius {
        return;
    }
    let relative = start - origin;
    let expanded_radius = radius + RADIUS_MARGIN;
    let (b, c) = if half_height.is_some() {
        (
            2.0 * (relative.0[0] * direction.0[0] + relative.0[1] * direction.0[1]),
            relative.0[0] * relative.0[0] + relative.0[1] * relative.0[1]
                - expanded_radius * expanded_radius,
        )
    } else {
        (
            2.0 * direction.dot(relative),
            relative.dot(relative) - expanded_radius * expanded_radius,
        )
    };
    let discriminant = b * b - 4.0 * c;
    if discriminant > 0.0 {
        let root = discriminant * math::reciprocal_sqrt_fast(discriminant, 2);
        let mut fraction = (-b - root) * 0.5;
        if fraction < 0.0 {
            fraction = 0.0;
        } else {
            fraction /= length;
        }
        if fraction < trace.fraction {
            let intersection = start.lerp(end, fraction);
            if half_height.is_some_and(|height| {
                intersection.0[2] > origin.0[2] + height || intersection.0[2] < origin.0[2] - height
            }) {
                return;
            }
            direction = project(intersection - origin) * (1.0 / expanded_radius);
            trace.fraction = fraction;
            trace.plane.normal = direction;
            trace.plane.distance = direction.dot(model_origin + intersection);
            trace.contents = super::Contents::BODY;
        }
    }
}
