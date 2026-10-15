use qa_core::primitives::{Body, CollisionShape, ModelRules, RuleSetId, Vec3};
use qa_world::collision::{
    CollisionStore, Contents, TraceQuery, store::BodyTraceMode, trace_policy,
};

fn query(start: Vec3, end: Vec3) -> TraceQuery<'static> {
    let (rules, entities) = trace_policy(RuleSetId::Quake3);
    TraceQuery {
        mask: Contents::BODY,
        ..TraceQuery::point(start, end, rules, entities)
    }
}

#[test]
fn direct_temporary_box_keeps_native_contact_bytes_and_ignores_target_pose() {
    let store = CollisionStore::new();
    let mut scratch = store.scratch();
    let body = Body {
        position: Vec3([4096.0; 3]),
        mins: Vec3([-1.0; 3]),
        maxs: Vec3([1.0; 3]),
        ..Body::default()
    };
    let trace = store
        .trace_body(
            query(Vec3([-4.0, 0.0, 0.0]), Vec3([4.0, 0.0, 0.0])),
            CollisionShape::Box,
            &body,
            Vec3([90.0; 3]),
            ModelRules::default(),
            BodyTraceMode::Direct,
            None,
            &mut scratch,
        )
        .unwrap();
    // qsrc CM_BoxTrace against the six BODY planes, SURFACE_CLIP_EPSILON=.125.
    assert_eq!(trace.fraction.to_bits(), (23.0f32 / 64.0).to_bits());
    assert_eq!(
        trace.end.0.map(f32::to_bits),
        [-1.125f32, 0.0, 0.0].map(f32::to_bits)
    );
    assert_eq!(trace.plane.normal, Vec3([-1.0, 0.0, 0.0]));
    assert_eq!(trace.plane.distance.to_bits(), 1.0f32.to_bits());
    assert_eq!(trace.plane.type_sign(), [3, 1]);
    assert_eq!(trace.contents, Contents::BODY);
    assert_eq!(trace.entity, None);
}

#[test]
fn direct_clear_copies_the_endpoint_but_transformed_clear_recalculates_it() {
    let store = CollisionStore::new();
    let mut scratch = store.scratch();
    let body = Body {
        mins: Vec3([-1.0; 3]),
        maxs: Vec3([1.0; 3]),
        ..Body::default()
    };
    let query = TraceQuery {
        mins: Vec3([-0.5, 0.0, 0.0]),
        maxs: Vec3([2.0, 0.0, 0.0]),
        mask: Contents::SOLID,
        ..query(Vec3([8192.0, 0.0, 0.0]), Vec3([0.0001, 0.0, 0.0]))
    };
    for (mode, expected) in [
        (BodyTraceMode::Direct, 0.0001f32),
        (BodyTraceMode::Transformed, 0.0f32),
    ] {
        let trace = store
            .trace_body(
                query,
                CollisionShape::Box,
                &body,
                Vec3::default(),
                ModelRules::default(),
                mode,
                None,
                &mut scratch,
            )
            .unwrap();
        assert_eq!(trace.fraction.to_bits(), 1.0f32.to_bits());
        assert_eq!(trace.end.0[0].to_bits(), expected.to_bits());
        assert!(!trace.start_solid && !trace.all_solid);
    }
}

#[test]
fn asymmetric_large_origin_distinguishes_direct_kernel_from_transformed_wrapper() {
    let store = CollisionStore::new();
    let mut scratch = store.scratch();
    let origin = 16_777_216.0;
    let query = TraceQuery {
        mins: Vec3::default(),
        maxs: Vec3([6.0, 0.0, 0.0]),
        ..query(Vec3([origin, 0.0, 0.0]), Vec3([origin, 0.0, 0.0]))
    };
    let direct = Body {
        mins: Vec3([origin - 10.0, -10.0, -10.0]),
        maxs: Vec3([origin, 10.0, 10.0]),
        ..Body::default()
    };
    let transformed = Body {
        position: Vec3([origin, 0.0, 0.0]),
        mins: Vec3([-10.0; 3]),
        maxs: Vec3([0.0, 10.0, 10.0]),
        ..Body::default()
    };
    for (mode, body, solid) in [
        (BodyTraceMode::Direct, direct, true),
        (BodyTraceMode::Transformed, transformed, false),
    ] {
        let trace = store
            .trace_body(
                query,
                CollisionShape::Box,
                &body,
                Vec3::default(),
                ModelRules::default(),
                mode,
                None,
                &mut scratch,
            )
            .unwrap();
        assert_eq!(trace.start_solid, solid);
        assert_eq!(trace.all_solid, solid);
        assert_eq!(
            trace.fraction.to_bits(),
            if solid { 0.0f32 } else { 1.0 }.to_bits()
        );
    }
}

#[test]
fn body_contents_share_native_bounds_ties_and_capsule_occupancy() {
    let store = CollisionStore::new();
    let body = Body {
        position: Vec3([100.0, -100.0, 20.0]),
        mins: Vec3([-15.0, -15.0, -24.0]),
        maxs: Vec3([15.0, 15.0, 32.0]),
        ..Body::default()
    };
    for (mode, origin) in [
        (BodyTraceMode::Direct, Vec3::default()),
        (BodyTraceMode::Transformed, body.position),
    ] {
        for (rules, boundary) in [
            (RuleSetId::Quake2, Contents::EMPTY),
            (RuleSetId::Quake3, Contents::BODY),
        ] {
            assert_eq!(
                store
                    .point_contents_body(
                        origin + Vec3([15.0, 0.0, 0.0]),
                        CollisionShape::Box,
                        &body,
                        Vec3([45.0; 3]),
                        ModelRules::default(),
                        trace_policy(rules).1,
                        mode,
                    )
                    .unwrap(),
                boundary,
            );
        }
        for (point, expected) in [
            (Vec3([0.0, 0.0, 32.0]), Contents::BODY),
            (Vec3([15.0, 15.0, 0.0]), Contents::EMPTY),
        ] {
            assert_eq!(
                store
                    .point_contents_body(
                        origin + point,
                        CollisionShape::Capsule,
                        &body,
                        Vec3::default(),
                        ModelRules::default(),
                        trace_policy(RuleSetId::Quake3).1,
                        mode,
                    )
                    .unwrap(),
                expected,
            );
        }
    }
    assert_eq!(
        store
            .point_contents_body(
                Vec3::default(),
                CollisionShape::None,
                &body,
                Vec3::default(),
                ModelRules::default(),
                trace_policy(RuleSetId::Quake3).1,
                BodyTraceMode::Direct,
            )
            .unwrap(),
        Contents::EMPTY,
    );
}

#[test]
fn direct_temporary_contents_admit_finite_empty_plane_distances() {
    let store = CollisionStore::new();
    let body = Body {
        mins: Vec3([1.0, -1.0, -1.0]),
        maxs: Vec3([-1.0, 1.0, 1.0]),
        ..Body::default()
    };
    assert_eq!(
        store
            .point_contents_body(
                Vec3::default(),
                CollisionShape::Box,
                &body,
                Vec3::default(),
                ModelRules::default(),
                trace_policy(RuleSetId::Quake3).1,
                BodyTraceMode::Direct,
            )
            .unwrap(),
        Contents::EMPTY,
    );
}
