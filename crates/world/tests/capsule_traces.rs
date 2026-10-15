use qa_core::primitives::{Body, Bounds, Capsule, CollisionShape, EntityId, RuleSetId, Vec3};
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
fn capsule_targets_use_the_common_dispatch_with_native_startsolid_and_plane_fields() {
    let store = CollisionStore::new();
    let mut scratch = store.scratch();
    let body = Body {
        position: Vec3([8.0, -4.0, 2.0]),
        mins: Vec3([-15.0, -15.0, -24.0]),
        maxs: Vec3([15.0, 15.0, 32.0]),
        ..Body::default()
    };
    let id = EntityId {
        slot: 7,
        generation: 3,
    };
    for capsule in [false, true] {
        let mut moving = query(Vec3([108.0, -4.0, 6.0]), Vec3([8.0, -4.0, 6.0]));
        moving.mins = Vec3([-1.0; 3]);
        moving.maxs = Vec3([1.0; 3]);
        if capsule {
            moving = moving.with_capsule();
        }
        let hit = store
            .trace_body(
                moving,
                CollisionShape::Capsule,
                &body,
                Vec3([45.0, 90.0, 0.0]),
                Default::default(),
                BodyTraceMode::Transformed,
                Some(id),
                &mut scratch,
            )
            .unwrap();
        assert!(hit.fraction > 0.0 && hit.fraction < 1.0);
        assert_eq!(hit.entity, Some(id));
        assert_eq!(hit.contents, Contents::BODY);
        assert_eq!(hit.end, moving.start.lerp(moving.end, hit.fraction));
        assert!(!hit.brush_solid);
        // TranslationOnly remains an explicit body-role capability.
        let unrotated = store
            .trace_body(
                moving,
                CollisionShape::Capsule,
                &body,
                Vec3::default(),
                Default::default(),
                BodyTraceMode::Transformed,
                Some(id),
                &mut scratch,
            )
            .unwrap();
        assert_eq!(
            (
                hit.fraction.to_bits(),
                hit.end,
                hit.plane,
                hit.start_solid,
                hit.all_solid,
                hit.contents,
                hit.entity
            ),
            (
                unrotated.fraction.to_bits(),
                unrotated.end,
                unrotated.plane,
                unrotated.start_solid,
                unrotated.all_solid,
                unrotated.contents,
                unrotated.entity
            )
        );
    }
    let inside = query(body.position, body.position).with_capsule();
    // Native sphere-position tests keep their default contents/plane fields.
    let hit = store
        .trace_body(
            inside,
            CollisionShape::Capsule,
            &body,
            Vec3::default(),
            Default::default(),
            BodyTraceMode::Transformed,
            Some(id),
            &mut scratch,
        )
        .unwrap();
    assert!(hit.start_solid && hit.all_solid);
    assert_eq!(hit.fraction, 0.0);
    assert_eq!(hit.contents, Contents::EMPTY);
}

#[test]
fn capsule_point_contents_do_not_include_the_bounding_box_corners() {
    let bounds = Bounds {
        mins: Vec3([-15.0, -15.0, -24.0]),
        maxs: Vec3([15.0, 15.0, 32.0]),
    };
    let capsule = Capsule::from_bounds(bounds);
    let center = qa_core::math::midpoint(bounds.mins, bounds.maxs);
    assert!(capsule.contains(center, Vec3([0.0, 0.0, 32.0])));
    assert!(!capsule.contains(center, Vec3([15.0, 15.0, 0.0])));
    assert!(capsule.contains(center, Vec3([15.0, 0.0, 4.0])));
    assert!(!capsule.contains(center, Vec3([0.0, 0.0, 32.001])));
}

#[test]
fn transformed_capsule_target_uses_the_shared_rotation_and_restores_contact_normal() {
    use qa_core::primitives::{ModelRotation, ModelRules};
    let store = CollisionStore::new();
    let mut scratch = store.scratch();
    let body = Body {
        position: Vec3([10.0, 0.0, 0.0]),
        mins: Vec3([-1.0, -1.0, -4.0]),
        maxs: Vec3([1.0, 1.0, 4.0]),
        ..Body::default()
    };
    let rules = ModelRules {
        rotation: ModelRotation::TransposeBasis,
        ..ModelRules::default()
    };
    let horizontal = query(Vec3([2.0, 0.0, 3.0]), Vec3([18.0, 0.0, 3.0])).with_capsule();
    for (angles, hit) in [(Vec3::default(), true), (Vec3([90.0, 0.0, 0.0]), false)] {
        let trace = store
            .trace_body(
                horizontal,
                CollisionShape::Capsule,
                &body,
                angles,
                rules,
                BodyTraceMode::Transformed,
                None,
                &mut scratch,
            )
            .unwrap();
        assert_eq!(trace.fraction < 1.0, hit);
        assert_eq!(
            store
                .point_contents_body(
                    Vec3([10.0, 0.0, 3.0]),
                    CollisionShape::Capsule,
                    &body,
                    angles,
                    rules,
                    trace_policy(RuleSetId::Quake3).1,
                    BodyTraceMode::Transformed
                )
                .unwrap(),
            if hit { Contents::BODY } else { Contents::EMPTY }
        );
    }
    let downward = query(Vec3([10.0, 0.0, 8.0]), Vec3([10.0, 0.0, 0.0])).with_capsule();
    let hit = store
        .trace_body(
            downward,
            CollisionShape::Capsule,
            &body,
            Vec3([90.0, 0.0, 0.0]),
            rules,
            BodyTraceMode::Transformed,
            None,
            &mut scratch,
        )
        .unwrap();
    assert!(hit.fraction > 0.0 && hit.fraction < 1.0);
    assert!(hit.plane.normal.0[2] > 0.99);
    assert!(hit.plane.normal.0[0].abs() < 0.0001);
    assert_eq!(hit.end, downward.start.lerp(downward.end, hit.fraction));
    let direct = store
        .trace_body(
            query(Vec3([-8.0, 0.0, 3.0]), Vec3([8.0, 0.0, 3.0])).with_capsule(),
            CollisionShape::Capsule,
            &body,
            Vec3([90.0, 0.0, 0.0]),
            rules,
            BodyTraceMode::Direct,
            None,
            &mut scratch,
        )
        .unwrap();
    assert!(direct.fraction < 1.0); // Direct ignores supplied target pose.
}
