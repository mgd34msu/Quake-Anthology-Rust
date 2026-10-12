use qa_core::bsp::{NodeError, node_depths};

#[test]
fn shared_children_and_disconnected_nodes_keep_raw_depth() {
    let nodes = [[-1, -2], [0, 0], [1, 0], [-3, -4], [3, 2]];
    assert_eq!(
        &*node_depths(nodes.len(), |n| nodes[n]).unwrap(),
        &[1, 2, 3, 1, 4]
    );
    assert!(node_depths(0, |_| unreachable!()).unwrap().is_empty());
    let mut nodes = Vec::new();
    let mut seed = 3367u32;
    let mut expected = Vec::<usize>::new();
    for index in 0..4096 {
        let children = std::array::from_fn(|_| {
            seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
            if index == 0 || seed & 3 == 0 {
                -1
            } else {
                (seed as usize % index) as i32
            }
        });
        expected.push(
            1 + children
                .iter()
                .filter(|&&c| c >= 0)
                .map(|&c| expected[c as usize])
                .max()
                .unwrap_or(0),
        );
        nodes.push(children);
    }
    assert_eq!(&*node_depths(nodes.len(), |n| nodes[n]).unwrap(), expected);
    let chain: Vec<_> = (0..4096).map(|n| [n - 1, -1]).collect();
    assert_eq!(node_depths(chain.len(), |n| chain[n]).unwrap()[4095], 4096);
}

#[test]
fn active_ancestor_and_disconnected_cycles_are_rejected() {
    for nodes in [
        vec![[0, -1]],
        vec![[1, -1], [0, -1]],
        vec![[-1, -2], [2, -1], [1, -2]],
    ] {
        assert_eq!(
            node_depths(nodes.len(), |n| nodes[n]),
            Err(NodeError::Cycle)
        );
    }
}
