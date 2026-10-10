use qa_core::primitives::{Body, EntityId, Vec3};
use qa_world::collision::{Contents, TraceQuery, TraceRules, boxes::trace_box};

#[test]
fn temporary_box_planes_keep_native_type_and_sign_conventions() {
    use qa_core::primitives::RuleSetId;
    let entity = EntityId {
        slot: 2,
        generation: 7,
    };
    let body = Body {
        mins: Vec3([-10.0; 3]),
        maxs: Vec3([10.0; 3]),
        ..Body::default()
    };
    for id in [
        RuleSetId::Quake2,
        RuleSetId::Quake2Rerelease,
        RuleSetId::Quake3,
    ] {
        let (rules, entities) = qa_world::collision::trace_policy(id);
        for axis in 0..3 {
            for positive in [false, true] {
                let mut start = [0.0; 3];
                start[axis] = if positive { 100.0 } else { -100.0 };
                let query = TraceQuery {
                    mask: Contents::BODY,
                    ..TraceQuery::point(Vec3(start), Vec3::default(), rules, entities)
                };
                let hit = trace_box(query, &body, entity);
                assert!(hit.fraction < 1.0);
                assert_eq!(
                    hit.plane.type_sign(),
                    [
                        if positive { axis as u8 } else { 3 + axis as u8 },
                        if id == RuleSetId::Quake3 && !positive {
                            1 << axis
                        } else {
                            0
                        },
                    ]
                );
            }
        }
    }
}

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
            entity_rules: qa_world::collision::trace_policy(qa_core::primitives::RuleSetId::Quake)
                .1,
            pass: None,
            excluded: &[],
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
            entity_rules: qa_world::collision::trace_policy(qa_core::primitives::RuleSetId::Quake)
                .1,
            pass: None,
            excluded: &[],
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
            entity_rules: qa_world::collision::trace_policy(qa_core::primitives::RuleSetId::Quake)
                .1,
            pass: None,
            excluded: &[],
        },
        &body,
        entity,
    );
    assert!(embedded.start_solid && embedded.all_solid);
    assert_eq!(embedded.entity, Some(entity));
}

#[test]
fn q2_transformed_box_reconstructs_clear_and_embedded_endpoints() {
    let entity = EntityId {
        slot: 3,
        generation: 7,
    };
    let body = Body {
        mins: Vec3([-10000.0; 3]),
        maxs: Vec3([10000.0; 3]),
        ..Default::default()
    };
    let query = TraceQuery {
        mask: Contents::BODY,
        ..TraceQuery::point(
            Vec3([8192.0, 0.0, 0.0]),
            Vec3([0.0001, 0.0, 0.0]),
            TraceRules::LEGACY,
            qa_world::collision::trace_policy(qa_core::primitives::RuleSetId::Quake2).1,
        )
    };
    let enclosed = trace_box(query, &body, entity);
    assert!(enclosed.all_solid && enclosed.start_solid);
    assert_eq!(enclosed.fraction, 1.0);
    assert_eq!(enclosed.end.0[0].to_bits(), 0.0f32.to_bits());
    let clear = trace_box(
        TraceQuery {
            mask: Contents::SOLID,
            ..query
        },
        &body,
        entity,
    );
    assert!(!clear.all_solid && !clear.start_solid);
    assert_eq!(clear.fraction, 1.0);
    assert_eq!(clear.end.0[0].to_bits(), 0.0f32.to_bits());
}

#[test]
fn q3_transformed_box_centers_before_subtracting_a_large_origin() {
    let entity = EntityId {
        slot: 3,
        generation: 7,
    };
    let body = Body {
        position: Vec3([16_777_216.0, 0.0, 0.0]),
        mins: Vec3([-10.0; 3]),
        maxs: Vec3([0.0, 10.0, 10.0]),
        ..Default::default()
    };
    let query = TraceQuery {
        mins: Vec3::default(),
        maxs: Vec3([6.0, 0.0, 0.0]),
        mask: Contents::BODY,
        ..TraceQuery::point(
            body.position,
            body.position,
            TraceRules::ARENA,
            qa_world::collision::trace_policy(qa_core::primitives::RuleSetId::Quake3).1,
        )
    };
    let result = trace_box(query, &body, entity);
    assert_eq!(result.fraction, 1.0);
    assert!(!result.start_solid && !result.all_solid);
    assert_eq!(result.entity, None);
}
