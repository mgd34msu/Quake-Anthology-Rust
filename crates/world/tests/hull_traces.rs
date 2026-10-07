use qa_core::primitives::{Axis, Plane, Vec3};
use qa_world::collision::{
    Contents,
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
    let result = hulls.trace(
        Vec3([1.0, 0.0, 0.0]),
        Vec3([-1.0, 0.0, 0.0]),
        Vec3::default(),
        Vec3::default(),
        Contents::SOLID,
    );
    assert_eq!(result.fraction, 0.484375);
    assert_eq!(result.end.0[0], 0.03125);
    assert_eq!(result.plane.normal, Vec3([1.0, 0.0, 0.0]));
    assert!(result.in_open && !result.start_solid && !result.all_solid);
    let embedded = hulls.trace(
        Vec3([-1.0, 0.0, 0.0]),
        Vec3([-2.0, 0.0, 0.0]),
        Vec3::default(),
        Vec3::default(),
        Contents::SOLID,
    );
    assert!(embedded.start_solid && embedded.all_solid);
    assert_eq!(embedded.fraction, 1.0);
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
