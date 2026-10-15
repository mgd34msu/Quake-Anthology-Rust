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

#[test]
fn visitor_walks_past_collection_limits_and_keeps_the_same_native_order() {
    use qa_core::primitives::child_from_leaf;
    const LEAVES: usize = 2048;
    let bounds = Bounds {
        mins: Vec3([-1.0; 3]),
        maxs: Vec3([1.0; 3]),
    };
    let node = |index: i32| {
        let children = [index as usize * 2 + 1, index as usize * 2 + 2].map(|child| {
            if child < LEAVES - 1 {
                child as i32
            } else {
                child_from_leaf((child - (LEAVES - 1)) as u32)
            }
        });
        (Plane::oriented(Vec3::axis(0, 1.0), 0.0), children)
    };
    let mut scratch = LeafScratch::new(12);
    let mut count = 0;
    let result = scratch
        .visit(0, bounds, node, |leaf| {
            assert_eq!(leaf as usize, count);
            count += 1;
            Ok::<_, ()>(true)
        })
        .unwrap()
        .unwrap();
    assert_eq!((result.count, result.top_node, count), (LEAVES, 0, LEAVES));
    let mut output = [u32::MAX; 1024];
    let collected = scratch.query(0, bounds, &mut output, node).unwrap();
    assert_eq!((collected.count, collected.top_node), (1024, 0));
    assert_eq!(output, std::array::from_fn(|i| i as u32));
    let mut seen = 0;
    let stopped = scratch
        .visit(0, bounds, node, |_| {
            seen += 1;
            Ok::<_, ()>(seen < 3)
        })
        .unwrap()
        .unwrap();
    assert_eq!((stopped.count, stopped.top_node, seen), (3, 0, 3));
    let empty = scratch
        .query(0, bounds, &mut [], |_| {
            panic!("empty collection must not walk")
        })
        .unwrap();
    assert_eq!((empty.count, empty.top_node), (0, -1));
}

#[test]
fn visitor_propagates_failure_and_stack_exhaustion_and_reuses_its_scratch() {
    let bounds = Bounds {
        mins: Vec3([-1.0; 3]),
        maxs: Vec3([1.0; 3]),
    };
    let node = |_| (Plane::oriented(Vec3::axis(0, 1.0), 0.0), [-1, -2]);
    let mut short = LeafScratch::new(1);
    assert_eq!(
        short.visit(0, bounds, node, |_| Ok::<_, u8>(true)),
        Ok(None)
    );
    let mut scratch = LeafScratch::new(2);
    assert_eq!(
        scratch.visit(0, bounds, node, |_| Err::<bool, _>(17u8)),
        Err(17)
    );
    let mut visited = Vec::new();
    let result = scratch
        .visit(0, bounds, node, |leaf| {
            visited.push(leaf);
            Ok::<_, u8>(true)
        })
        .unwrap()
        .unwrap();
    assert_eq!((result.count, result.top_node), (2, 0));
    assert_eq!(visited, [0, 1]);
}
