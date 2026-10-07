use qa_core::primitives::{Body, EntityId, Vec3};
use qa_world::collision::{Contents, boxes::trace_box};

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
        Vec3([100.0, 0.0, 0.0]),
        Vec3::default(),
        Vec3([-16.0; 3]),
        Vec3([16.0; 3]),
        Contents::SOLID,
        &body,
        entity,
    );
    assert_eq!(trace.fraction, (100.0 - 46.0 - 0.03125) / 100.0);
    assert_eq!(trace.end.0[0], 46.03125);
    assert_eq!(trace.plane.normal, Vec3([1.0, 0.0, 0.0]));
    assert_eq!(trace.entity, Some(entity));
    let embedded = trace_box(
        body.position,
        body.position,
        Vec3::default(),
        Vec3::default(),
        Contents::SOLID,
        &body,
        entity,
    );
    assert!(embedded.start_solid && embedded.all_solid);
    assert_eq!(embedded.entity, Some(entity));
}
