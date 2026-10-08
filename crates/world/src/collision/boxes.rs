use super::{
    Trace, TraceQuery,
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

pub fn trace_box(query: TraceQuery, body: &Body, entity: EntityId) -> Trace {
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
    let local = |point: Vec3| {
        Vec3(std::array::from_fn(|axis| {
            point.0[axis] - body.position.0[axis]
        }))
    };
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
        trace.end = Vec3(std::array::from_fn(|axis| {
            trace.end.0[axis] + body.position.0[axis]
        }));
    } else {
        trace.end = end;
    }
    if trace.fraction < 1.0 || trace.start_solid {
        trace.entity = Some(entity);
    }
    trace.brush_solid = false;
    trace
}
