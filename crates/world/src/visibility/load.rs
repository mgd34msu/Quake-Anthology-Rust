use super::*;

fn valid_child(child: i32, nodes: usize, leaves: usize) -> bool {
    if child >= 0 {
        (child as usize) < nodes
    } else {
        (qa_core::primitives::leaf_from_child(child) as usize) < leaves
    }
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
            if !plane.is_valid() {
                return Err(VisibilityError::Plane(index));
            }
        }
        let mut surface_owners = vec![None; surface_count];
        for (index, node) in nodes.iter().enumerate() {
            if node.plane as usize >= planes.len() {
                return Err(VisibilityError::Plane(index));
            }
            if !node.bounds.is_valid() {
                return Err(VisibilityError::Bounds(index));
            }
            if node
                .children
                .iter()
                .any(|&child| !valid_child(child, nodes.len(), leaves.len()))
            {
                return Err(VisibilityError::Child(index));
            }
            if !node.surfaces.fits(surface_count) {
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
            if !leaf.bounds.is_valid() {
                return Err(VisibilityError::Bounds(nodes.len() + index));
            }
            if !leaf.surfaces.fits(leaf_surfaces.len()) {
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
        crate::bsp::node_depths(nodes.len(), |index| nodes[index].children).map_err(|error| {
            match error {
                crate::bsp::NodeError::Cycle => VisibilityError::Cycle,
                crate::bsp::NodeError::Depth => VisibilityError::Size,
            }
        })?;

        // C port world.c:519: a validated BSP may share children; retain every parent.
        let mut parent_heads = vec![u32::MAX; nodes.len() + leaves.len()];
        let mut parent_edges = Vec::with_capacity(nodes.len() * 2);
        for (index, node) in nodes.iter().enumerate() {
            for &child in &node.children {
                let child_index = if child >= 0 {
                    child as usize
                } else {
                    nodes.len() + qa_core::primitives::leaf_from_child(child) as usize
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
