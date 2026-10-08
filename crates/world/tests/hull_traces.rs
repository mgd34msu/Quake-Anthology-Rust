use qa_core::primitives::{Axis, Plane, Vec3};
use qa_world::collision::{
    Contents, TraceQuery, TraceRules,
    hulls::{ClipNode, HullError, HullModel, Q1Hulls},
};

fn wall() -> (Vec<Plane>, Vec<ClipNode>, Vec<HullModel>) {
    (
        vec![Plane {
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
    let mut hulls = Q1Hulls::load(planes, nodes.clone(), nodes, models).unwrap();
    let result = hulls.trace(TraceQuery {
        start: Vec3([1.0, 0.0, 0.0]),
        end: Vec3([-1.0, 0.0, 0.0]),
        mins: Vec3::default(),
        maxs: Vec3::default(),
        mask: Contents::SOLID,
        rules: TraceRules::LEGACY,
    });
    assert_eq!(result.fraction, 0.484375);
    assert_eq!(result.end.0[0], 0.03125);
    assert_eq!(result.plane.normal, Vec3([1.0, 0.0, 0.0]));
    assert!(result.in_open && !result.start_solid && !result.all_solid);
    let embedded = hulls.trace(TraceQuery {
        start: Vec3([-1.0, 0.0, 0.0]),
        end: Vec3([-2.0, 0.0, 0.0]),
        mins: Vec3::default(),
        maxs: Vec3::default(),
        mask: Contents::SOLID,
        rules: TraceRules::LEGACY,
    });
    assert!(embedded.start_solid && embedded.all_solid);
    assert_eq!(embedded.fraction, 1.0);
}

#[test]
fn foreign_caller_epsilon_is_used_on_compiled_hull_topology() {
    let (planes, nodes, models) = wall();
    let mut hulls = Q1Hulls::load(planes, nodes.clone(), nodes, models).unwrap();
    let query = TraceQuery {
        start: Vec3([1.0, 0.0, 0.0]),
        end: Vec3([-1.0, 0.0, 0.0]),
        mins: Vec3::default(),
        maxs: Vec3::default(),
        mask: Contents::SOLID,
        rules: TraceRules::ARENA,
    };
    let contact = hulls.trace(query);
    assert_eq!(contact.fraction, 0.4375);
    assert_eq!(contact.end.0[0], 0.125);
    let embedded = hulls.trace(TraceQuery {
        start: Vec3([-1.0, 0.0, 0.0]),
        end: Vec3([-2.0, 0.0, 0.0]),
        ..query
    });
    assert!(embedded.start_solid && embedded.all_solid);
    assert_eq!(embedded.fraction, 0.0);
    assert_eq!(embedded.end, Vec3([-1.0, 0.0, 0.0]));
    assert_eq!(embedded.contents, Contents::SOLID);
}

#[test]
fn compiled_hull_height_is_not_silently_claimed_to_fit_a_foreign_crouch() {
    let mut hulls = Q1Hulls::load(
        vec![
            Plane {
                normal: Vec3([0.0, 0.0, 1.0]),
                distance: 0.0,
                axis: Some(Axis::Z),
            },
            Plane {
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
    let standing = TraceQuery {
        start: Vec3([0.0, 0.0, -100.0]),
        end: Vec3::default(),
        mins: Vec3([-16.0, -16.0, -24.0]),
        maxs: Vec3([16.0, 16.0, 32.0]),
        mask: Contents::SOLID,
        rules: TraceRules::LEGACY,
    };
    let stock = hulls.trace(standing);
    assert_eq!(stock.end.0[2], -32.03125);
    let crouched = hulls.trace(TraceQuery {
        maxs: Vec3([16.0, 16.0, 4.0]),
        ..standing
    });
    // Native SV_HullForEntity chooses by X width, so the authored standing
    // ceiling remains. Rebuilding a four-unit-high hull is separate work.
    assert_eq!(crouched.end, stock.end);
    let foreign = hulls.trace(TraceQuery {
        maxs: Vec3([16.0, 16.0, 4.0]),
        rules: TraceRules::ARENA,
        ..standing
    });
    assert_eq!(foreign.end.0[2], -32.125);
}

#[test]
fn malformed_hulls_are_rejected_once_at_load() {
    let (planes, mut nodes, models) = wall();
    nodes[0].children[0] = 0;
    assert!(matches!(
        Q1Hulls::load(planes, nodes.clone(), nodes, models),
        Err(HullError::Cycle)
    ));
    let (planes, mut nodes, models) = wall();
    nodes[0].children[0] = 12;
    assert!(matches!(
        Q1Hulls::load(planes, nodes.clone(), nodes, models),
        Err(HullError::Node)
    ));
}
