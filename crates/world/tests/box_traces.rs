use qa_core::primitives::{Body, EntityId, Vec3};
use qa_world::collision::{Contents, TraceQuery, TraceRules, boxes::trace_box};

#[test]
fn box_hull_expands_by_query_bounds_and_offsets_the_endpoint() {
    let entity = EntityId {
        slot: 3,
        generation: 7,
    };
    let body = Body {
        position: Vec3([20.0, 0.0, 0.0]),
        mins: Vec3([-10.0; 3]),
        maxs: Vec3([10.0; 3]),
        ..Body::default()
    };
    let trace = trace_box(
        TraceQuery {
            start: Vec3([100.0, 0.0, 0.0]),
            end: Vec3::default(),
            mins: Vec3([-16.0; 3]),
            maxs: Vec3([16.0; 3]),
            mask: Contents::SOLID,
            rules: TraceRules::LEGACY,
        },
        &body,
        entity,
    );
    assert_eq!(trace.fraction, (100.0 - 46.0 - 0.03125) / 100.0);
    assert_eq!(trace.end.0[0], 46.03125);
    assert_eq!(trace.plane.normal, Vec3([1.0, 0.0, 0.0]));
    assert_eq!(trace.entity, Some(entity));
    let foreign_contact = trace_box(
        TraceQuery {
            start: Vec3([100.0, 0.0, 0.0]),
            end: Vec3::default(),
            mins: Vec3([-16.0; 3]),
            maxs: Vec3([16.0; 3]),
            mask: Contents::SOLID,
            rules: TraceRules::ARENA,
        },
        &body,
        entity,
    );
    assert_eq!(foreign_contact.fraction, (100.0 - 46.0 - 0.125) / 100.0);
    assert_eq!(foreign_contact.end.0[0], 46.125);
    let embedded = trace_box(
        TraceQuery {
            start: body.position,
            end: body.position,
            mins: Vec3::default(),
            maxs: Vec3::default(),
            mask: Contents::SOLID,
            rules: TraceRules::LEGACY,
        },
        &body,
        entity,
    );
    assert!(embedded.start_solid && embedded.all_solid);
    assert_eq!(embedded.entity, Some(entity));
}
