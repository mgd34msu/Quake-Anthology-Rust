use qa_core::primitives::{Bounds, Plane, Vec3};
use qa_world::leaves::LeafScratch;

#[test]
fn bounded_box_walk_keeps_native_front_order_top_node_and_ties() {
    let planes = [
        Plane::oriented(Vec3([1.0, 0.0, 0.0]), 0.0),
        Plane::oriented(Vec3([0.0, -1.0, 0.0]), 0.0),
    ];
    let nodes = [(0, [1, 2]), (1, [-1, -2]), (1, [-3, -4])];
    let node = |child: i32| {
        let (plane, children) = nodes[child as usize];
        (planes[plane], children)
    };
    let mut scratch = LeafScratch::new(3);
    let mut output = [u32::MAX; 4];
    let bounds = Bounds {
        mins: Vec3([-1.0; 3]),
        maxs: Vec3([1.0; 3]),
    };
    let all = scratch.query(0, bounds, &mut output, node).unwrap();
    assert_eq!((all.count, all.top_node), (4, 0));
    assert_eq!(output, [0, 1, 2, 3]);
    let back = Bounds {
        maxs: Vec3([-0.5, 1.0, 1.0]),
        ..bounds
    };
    let result = scratch.query(0, back, &mut output, node).unwrap();
    assert_eq!((result.count, result.top_node), (2, 2));
    assert_eq!(&output[..2], &[2, 3]);
    let front_tie = Bounds {
        mins: Vec3([0.0; 3]),
        maxs: Vec3([0.0; 3]),
    };
    let result = scratch.query(0, front_tie, &mut output, node).unwrap();
    assert_eq!((result.count, result.top_node, output[0]), (1, -1, 0));
    let bounded = scratch.query(0, bounds, &mut output[..1], node).unwrap();
    assert_eq!((bounded.count, bounded.top_node, output[0]), (1, 0, 0));
    assert!(
        LeafScratch::new(1)
            .query(0, bounds, &mut output, node)
            .is_none()
    );
    let direct = scratch
        .query(-4, bounds, &mut output, |_| panic!("leaf is not a node"))
        .unwrap();
    assert_eq!((direct.count, direct.top_node, output[0]), (1, -1, 3));
}

#[test]
fn shared_leaf_references_retain_native_visit_order() {
    let mut scratch = LeafScratch::new(2);
    let mut output = [u32::MAX; 2];
    let result = scratch
        .query(
            0,
            Bounds {
                mins: Vec3([-2.0; 3]),
                maxs: Vec3([2.0; 3]),
            },
            &mut output,
            |_| (Plane::oriented(Vec3([1.0, 0.0, 0.0]), 0.0), [-8, -8]),
        )
        .unwrap();
    assert_eq!((result.count, result.top_node), (2, 0));
    assert_eq!(output, [7, 7]);
}
