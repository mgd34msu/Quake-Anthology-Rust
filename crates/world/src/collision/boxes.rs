use super::EntityTraceFlags;
use super::surfaces::{SurfaceRecord, SurfaceRows};
use super::{
    Contents, Trace, TraceQuery,
    brushes::{Brush, trace_brushes},
    hulls::{ClipNode, Frame, Hull},
};
use qa_core::primitives::{Axis, Body, EntityId, Plane, Vec3};

const fn nodes() -> [ClipNode; 6] {
    let mut result = [ClipNode {
        plane: 0,
        children: [-1; 2],
    }; 6];
    let mut index = 0;
    while index < 6 {
        result[index].plane = index as u32;
        result[index].children[(index & 1) ^ 1] = if index == 5 { -2 } else { index as i32 + 1 };
        index += 1;
    }
    result
}
static NODES: [ClipNode; 6] = nodes();

pub fn trace_box(query: TraceQuery, body: &Body, entity: Option<EntityId>) -> Trace {
    if query.entity_rules.quake_kind().is_none() {
        return trace_convex_box(query, body, entity);
    }
    let TraceQuery {
        start,
        end,
        mins,
        maxs,
        ..
    } = query;
    let planes: [Plane; 6] = std::array::from_fn(|index| {
        let axis = index / 2;
        Plane {
            encoding: None,
            normal: Vec3(std::array::from_fn(|coordinate| {
                if coordinate == axis { 1.0 } else { 0.0 }
            })),
            distance: if index & 1 == 0 {
                body.maxs.0[axis] - mins.0[axis]
            } else {
                body.mins.0[axis] - maxs.0[axis]
            },
            axis: Some([Axis::X, Axis::Y, Axis::Z][axis]),
        }
    });
    let local = |point: Vec3| point - body.position;
    let mut stack = [Frame::default(); 7];
    let mut trace = Hull {
        nodes: &NODES,
        planes: &planes,
        root: 0,
    }
    .trace(
        TraceQuery {
            start: local(start),
            end: local(end),
            ..query
        },
        &mut stack,
    );
    if trace.fraction < 1.0 {
        trace.end = trace.end + body.position;
    } else {
        trace.end = end;
    }
    if trace.fraction < 1.0 || trace.start_solid {
        trace.entity = entity;
    }
    trace.brush_solid = false;
    trace
}

fn trace_convex_box(query: TraceQuery, body: &Body, entity: Option<EntityId>) -> Trace {
    let original = query;
    let mut query = query;
    if query
        .entity_rules
        .filtering
        .contains(EntityTraceFlags::CENTER_BOX)
    {
        query.center_box();
    }
    // Native CM_InitBoxHull pairs +max/-min planes in X/Y/Z order. Its
    // temporary brush is MONSTER (Q2) / BODY (Q3), independently of r.contents.
    let planes = std::array::from_fn::<_, 6, _>(|index| {
        let axis = index / 2;
        let positive = index & 1 == 0;
        Plane {
            encoding: Some([
                if positive { axis as u8 } else { 3 + axis as u8 },
                if !positive
                    && query
                        .entity_rules
                        .filtering
                        .contains(EntityTraceFlags::CENTER_BOX)
                {
                    1 << axis
                } else {
                    0
                },
            ]),
            normal: Vec3(std::array::from_fn(|coordinate| {
                if coordinate == axis {
                    if positive { 1.0 } else { -1.0 }
                } else {
                    0.0
                }
            })),
            distance: if positive {
                body.maxs.0[axis]
            } else {
                -body.mins.0[axis]
            },
            axis: if positive {
                Some([Axis::X, Axis::Y, Axis::Z][axis])
            } else {
                None
            },
        }
    });
    let local = |point: Vec3| point - body.position;
    let mut trace = trace_brushes(
        TraceQuery {
            start: local(query.start),
            end: local(query.end),
            ..query
        },
        &planes,
        &[Brush {
            first_plane: 0,
            plane_count: 6,
            contents: Contents::BODY,
        }],
        SurfaceRows {
            records: &[SurfaceRecord::EMPTY],
            sides: &[0; 6],
            geometry: None,
        },
    );
    // CM_TransformedBoxTrace derives the endpoint from the original segment.
    trace.end = original.start.lerp(original.end, trace.fraction);
    if trace.fraction < 1.0 || trace.start_solid {
        trace.entity = entity;
    }
    trace.brush_solid = false;
    trace
}
