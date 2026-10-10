use qa_core::primitives::{Axis, Bounds, GeometryId, Plane, Vec3};
use qa_world::collision::{
    CollisionStore, Contents, StoreError, TraceQuery, TraceRules,
    hulls::{ClipNode, HullError, HullModel},
};

struct Case {
    store: CollisionStore,
    geometry: GeometryId,
}
fn load(
    planes: Vec<Plane>,
    drawing: Vec<ClipNode>,
    clips: Vec<ClipNode>,
    models: Vec<HullModel>,
) -> Result<Case, StoreError> {
    let bounds = vec![
        Bounds {
            mins: Vec3([-32768.0; 3]),
            maxs: Vec3([32768.0; 3])
        };
        models.len()
    ];
    let mut store = CollisionStore::new();
    let geometry = store.load_hulls(planes, drawing, clips, models, bounds)?;
    Ok(Case { store, geometry })
}

fn wall() -> (Vec<Plane>, Vec<ClipNode>, Vec<HullModel>) {
    (
        vec![Plane {
            encoding: None,
            normal: Vec3([1.0, 0.0, 0.0]),
            distance: 0.0,
            axis: Some(Axis::X),
        }],
        vec![ClipNode {
            plane: 0,
            children: [-1, -2],
        }],
        vec![HullModel { roots: [0; 3] }],
    )
}

#[test]
fn native_hull_epsilon_and_startsolid_follow_world_c() {
    let (planes, nodes, models) = wall();
    let hulls = load(planes, nodes.clone(), nodes, models).unwrap();
    let mut scratch = hulls.store.scratch();
    let result = hulls.store.trace_model(
        hulls.geometry,
        0,
        TraceQuery::point(
            Vec3([1.0, 0.0, 0.0]),
            Vec3([-1.0, 0.0, 0.0]),
            TraceRules::LEGACY,
            qa_world::collision::trace_policy(qa_core::primitives::RuleSetId::Quake).1,
        ),
        &mut scratch,
    );
    assert_eq!(result.fraction, 0.484375);
    assert_eq!(result.end.0[0], 0.03125);
    assert_eq!(result.plane.normal, Vec3([1.0, 0.0, 0.0]));
    assert!(result.in_open && !result.start_solid && !result.all_solid);
    let embedded = hulls.store.trace_model(
        hulls.geometry,
        0,
        TraceQuery::point(
            Vec3([-1.0, 0.0, 0.0]),
            Vec3([-2.0, 0.0, 0.0]),
            TraceRules::LEGACY,
            qa_world::collision::trace_policy(qa_core::primitives::RuleSetId::Quake).1,
        ),
        &mut scratch,
    );
    assert!(embedded.start_solid && embedded.all_solid);
    assert_eq!(embedded.fraction, 1.0);
}

#[test]
fn foreign_caller_epsilon_is_used_on_compiled_hull_topology() {
    let (planes, nodes, models) = wall();
    let hulls = load(planes, nodes.clone(), nodes, models).unwrap();
    let mut scratch = hulls.store.scratch();
    let query = TraceQuery::point(
        Vec3([1.0, 0.0, 0.0]),
        Vec3([-1.0, 0.0, 0.0]),
        TraceRules::ARENA,
        qa_world::collision::trace_policy(qa_core::primitives::RuleSetId::Quake).1,
    );
    let contact = hulls
        .store
        .trace_model(hulls.geometry, 0, query, &mut scratch);
    assert_eq!(contact.fraction, 0.4375);
    assert_eq!(contact.end.0[0], 0.125);
    let embedded = hulls.store.trace_model(
        hulls.geometry,
        0,
        TraceQuery {
            start: Vec3([-1.0, 0.0, 0.0]),
            end: Vec3([-2.0, 0.0, 0.0]),
            ..query
        },
        &mut scratch,
    );
    assert!(embedded.start_solid && embedded.all_solid);
    assert_eq!(embedded.fraction, 0.0);
    assert_eq!(embedded.end, Vec3([-1.0, 0.0, 0.0]));
    assert_eq!(embedded.contents, Contents::SOLID);
}

#[test]
fn independent_callers_reuse_scratch_over_one_immutable_hull() {
    let (planes, nodes, models) = wall();
    let hulls = load(planes, nodes.clone(), nodes, models).unwrap();
    let geometry = &hulls;
    let mut first = geometry.store.scratch();
    let mut second = geometry.store.scratch();
    let crossing = TraceQuery::point(
        Vec3([1.0, 0.0, 0.0]),
        Vec3([-1.0, 0.0, 0.0]),
        TraceRules::LEGACY,
        qa_world::collision::trace_policy(qa_core::primitives::RuleSetId::Quake).1,
    );
    let embedded = TraceQuery::point(
        Vec3([-1.0, 0.0, 0.0]),
        Vec3([-2.0, 0.0, 0.0]),
        TraceRules::ARENA,
        qa_world::collision::trace_policy(qa_core::primitives::RuleSetId::Quake).1,
    );
    for _ in 0..32 {
        let hit = geometry
            .store
            .trace_model(geometry.geometry, 0, crossing, &mut first);
        let inside = geometry
            .store
            .trace_model(geometry.geometry, 0, embedded, &mut second);
        assert_eq!(hit.fraction.to_bits(), 0.484375f32.to_bits());
        assert_eq!(hit.end.0.map(f32::to_bits), [0.03125f32.to_bits(), 0, 0]);
        assert!(hit.in_open && !hit.start_solid && !hit.all_solid);
        assert_eq!(inside.fraction.to_bits(), 0.0f32.to_bits());
        assert_eq!(inside.end, embedded.start);
        assert!(inside.start_solid && inside.all_solid);
        assert_eq!(inside.contents, Contents::SOLID);
        let swapped_hit = geometry
            .store
            .trace_model(geometry.geometry, 0, crossing, &mut second);
        let swapped_inside = geometry
            .store
            .trace_model(geometry.geometry, 0, embedded, &mut first);
        assert_eq!(swapped_hit.fraction.to_bits(), hit.fraction.to_bits());
        assert_eq!(
            swapped_hit.end.0.map(f32::to_bits),
            hit.end.0.map(f32::to_bits)
        );
        assert_eq!(swapped_inside.fraction.to_bits(), inside.fraction.to_bits());
        assert_eq!(swapped_inside.end, inside.end);
    }
}

#[test]
fn compiled_hull_height_is_not_silently_claimed_to_fit_a_foreign_crouch() {
    let hulls = load(
        vec![
            Plane {
                encoding: None,
                normal: Vec3([0.0, 0.0, 1.0]),
                distance: 0.0,
                axis: Some(Axis::Z),
            },
            Plane {
                encoding: None,
                normal: Vec3([0.0, 0.0, 1.0]),
                distance: -32.0,
                axis: Some(Axis::Z),
            },
        ],
        vec![ClipNode {
            plane: 0,
            children: [-2, -1],
        }],
        vec![ClipNode {
            plane: 1,
            children: [-2, -1],
        }],
        vec![HullModel { roots: [0; 3] }],
    )
    .unwrap();
    let mut scratch = hulls.store.scratch();
    let standing = TraceQuery {
        mins: Vec3([-16.0, -16.0, -24.0]),
        maxs: Vec3([16.0, 16.0, 32.0]),
        ..TraceQuery::point(
            Vec3([0.0, 0.0, -100.0]),
            Vec3::default(),
            TraceRules::LEGACY,
            qa_world::collision::trace_policy(qa_core::primitives::RuleSetId::Quake).1,
        )
    };
    let stock = hulls
        .store
        .trace_model(hulls.geometry, 0, standing, &mut scratch);
    assert_eq!(stock.end.0[2], -32.03125);
    let crouched = hulls.store.trace_model(
        hulls.geometry,
        0,
        TraceQuery {
            maxs: Vec3([16.0, 16.0, 4.0]),
            ..standing
        },
        &mut scratch,
    );
    // Native SV_HullForEntity chooses by X width, so the authored standing
    // ceiling remains. Rebuilding a four-unit-high hull is separate work.
    assert_eq!(crouched.end, stock.end);
    let foreign = hulls.store.trace_model(
        hulls.geometry,
        0,
        TraceQuery {
            maxs: Vec3([16.0, 16.0, 4.0]),
            rules: TraceRules::ARENA,
            ..standing
        },
        &mut scratch,
    );
    assert_eq!(foreign.end.0[2], -32.125);
}

#[test]
fn malformed_hulls_are_rejected_once_at_load() {
    let (planes, mut nodes, models) = wall();
    nodes[0].children[0] = 0;
    assert!(matches!(
        load(planes, nodes.clone(), nodes, models),
        Err(StoreError::Hull(HullError::Cycle))
    ));
    let (planes, mut nodes, models) = wall();
    nodes[0].children[0] = 12;
    assert!(matches!(
        load(planes, nodes.clone(), nodes, models),
        Err(StoreError::Hull(HullError::Node))
    ));
}
