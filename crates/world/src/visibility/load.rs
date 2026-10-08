use super::*;

fn valid_bounds(bounds: Bounds) -> bool {
    (0..3).all(|axis| {
        bounds.mins.0[axis].is_finite()
            && bounds.maxs.0[axis].is_finite()
            && bounds.mins.0[axis] <= bounds.maxs.0[axis]
    })
}

fn valid_child(child: i32, nodes: usize, leaves: usize) -> bool {
    if child >= 0 {
        (child as usize) < nodes
    } else {
        (-1 - i64::from(child)) < leaves as i64
    }
}

fn valid_span(span: SurfaceSpan, count: usize) -> bool {
    (span.first as usize)
        .checked_add(span.count as usize)
        .is_some_and(|end| end <= count)
}

fn validate_cycles(nodes: &[VisNode]) -> Result<(), VisibilityError> {
    let mut marks = vec![0u8; nodes.len()];
    let mut stack = Vec::with_capacity(nodes.len());
    for root in 0..nodes.len() {
        if marks[root] == 2 {
            continue;
        }
        marks[root] = 1;
        stack.push((root, 0));
        while let Some((node, side)) = stack.last_mut() {
            if *side == 2 {
                marks[*node] = 2;
                stack.pop();
                continue;
            }
            let child = nodes[*node].children[*side];
            *side += 1;
            if child < 0 {
                continue;
            }
            let child = child as usize;
            match marks[child] {
                1 => return Err(VisibilityError::Cycle),
                0 => {
                    marks[child] = 1;
                    stack.push((child, 0));
                }
                _ => {}
            }
        }
    }
    Ok(())
}

impl VisibilityWorld {
    #[allow(clippy::too_many_arguments)]
    pub fn load(
        planes: Vec<Plane>,
        nodes: Vec<VisNode>,
        leaves: Vec<VisLeaf>,
        leaf_surfaces: Vec<u32>,
        surface_count: usize,
        root: i32,
        pvs: PvsRows,
    ) -> Result<Self, VisibilityError> {
        if nodes.len() > i32::MAX as usize
            || leaves.len() > i32::MAX as usize
            || surface_count > u32::MAX as usize
            || planes.len() > u32::MAX as usize
            || leaf_surfaces.len() > u32::MAX as usize
            || nodes.len().checked_mul(2).is_none()
            || nodes.len().checked_add(leaves.len()).is_none()
            || nodes.len() + leaves.len() > u32::MAX as usize
            || pvs.selector_count() > u32::MAX as usize
        {
            return Err(VisibilityError::Size);
        }
        if !valid_child(root, nodes.len(), leaves.len()) {
            return Err(VisibilityError::Root);
        }
        for (index, plane) in planes.iter().enumerate() {
            if !plane.distance.is_finite()
                || plane.normal.0.iter().any(|value| !value.is_finite())
                || plane.normal == Vec3::default()
            {
                return Err(VisibilityError::Plane(index));
            }
        }
        let mut surface_owners = vec![None; surface_count];
        for (index, node) in nodes.iter().enumerate() {
            if node.plane as usize >= planes.len() {
                return Err(VisibilityError::Plane(index));
            }
            if !valid_bounds(node.bounds) {
                return Err(VisibilityError::Bounds(index));
            }
            if node
                .children
                .iter()
                .any(|&child| !valid_child(child, nodes.len(), leaves.len()))
            {
                return Err(VisibilityError::Child(index));
            }
            if !valid_span(node.surfaces, surface_count) {
                return Err(VisibilityError::SurfaceRange(index));
            }
            let first = node.surfaces.first as usize;
            for (offset, owner) in surface_owners[first..first + node.surfaces.count as usize]
                .iter_mut()
                .enumerate()
            {
                if owner.is_some() {
                    return Err(VisibilityError::SurfaceOwnership((first + offset) as u32));
                }
                *owner = Some(index as u32);
            }
        }
        for (index, leaf) in leaves.iter().enumerate() {
            if !valid_bounds(leaf.bounds) {
                return Err(VisibilityError::Bounds(nodes.len() + index));
            }
            if !valid_span(leaf.surfaces, leaf_surfaces.len()) {
                return Err(VisibilityError::SurfaceRange(nodes.len() + index));
            }
            if leaf
                .selector
                .is_some_and(|selector| selector as usize >= pvs.selector_count())
            {
                return Err(VisibilityError::Selector(index));
            }
        }
        for (index, &surface) in leaf_surfaces.iter().enumerate() {
            if surface as usize >= surface_count {
                return Err(VisibilityError::SurfaceReference(index));
            }
        }
        validate_cycles(&nodes)?;

        // C port world.c:519: a validated BSP may share children; retain every parent.
        let mut parent_heads = vec![u32::MAX; nodes.len() + leaves.len()];
        let mut parent_edges = Vec::with_capacity(nodes.len() * 2);
        for (index, node) in nodes.iter().enumerate() {
            for &child in &node.children {
                let child_index = if child >= 0 {
                    child as usize
                } else {
                    nodes.len() + (-1 - i64::from(child)) as usize
                };
                let edge = parent_edges.len() as u32;
                parent_edges.push(ParentEdge {
                    node: index as u32,
                    next: parent_heads[child_index],
                });
                parent_heads[child_index] = edge;
            }
        }
        Ok(Self {
            planes: planes.into_boxed_slice(),
            nodes: nodes.into_boxed_slice(),
            leaves: leaves.into_boxed_slice(),
            leaf_surfaces: leaf_surfaces.into_boxed_slice(),
            surface_owners: surface_owners.into_boxed_slice(),
            parent_heads: parent_heads.into_boxed_slice(),
            parent_edges: parent_edges.into_boxed_slice(),
            root,
            pvs,
        })
    }
}
