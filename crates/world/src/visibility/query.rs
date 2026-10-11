use super::*;

fn remaining_planes(bounds: Bounds, frustum: &[Plane], mut mask: u8) -> Option<u8> {
    for (index, plane) in frustum.iter().enumerate() {
        let bit = 1 << index;
        if mask & bit == 0 {
            continue;
        }
        let positive = bounds.corner(plane.normal, true);
        if plane.signed_distance(positive) < 0.0 {
            return None;
        }
        let negative = bounds.corner(plane.normal, false);
        if plane.signed_distance(negative) >= 0.0 {
            mask &= !bit;
        }
    }
    Some(mask)
}

impl ViewVisibility {
    #[allow(clippy::too_many_arguments)]
    pub fn query(
        &mut self,
        world: &VisibilityWorld,
        origin: Vec3,
        primary: Option<u32>,
        secondary: Option<u32>,
        hidden_areas: &[u8],
        frustum: &[Plane],
    ) -> Result<(), VisibilityQueryError> {
        self.visible_count = 0;
        self.counters = VisibilityCounters::default();
        if origin.0.iter().any(|value| !value.is_finite()) {
            return Err(VisibilityQueryError::Origin);
        }
        if frustum.len() > 6
            || frustum.iter().any(|plane| {
                !plane.distance.is_finite()
                    || plane.normal.0.iter().any(|value| !value.is_finite())
                    || plane.normal == Vec3::default()
            })
        {
            return Err(VisibilityQueryError::Frustum);
        }
        if self.node_marks.len() != world.nodes.len()
            || self.leaf_marks.len() != world.leaves.len()
            || self.surface_marks.len() != world.surface_count()
            || self.primary.len() != world.pvs.row_bytes()
        {
            return Err(VisibilityQueryError::ScratchSize);
        }
        world.pvs.read_into(primary, &mut self.primary)?;
        let mut all_visible =
            primary.is_none_or(|selector| world.pvs.offsets[selector as usize].is_none());
        if let Some(secondary) = secondary {
            world.pvs.read_into(Some(secondary), &mut self.secondary)?;
            all_visible |= world.pvs.offsets[secondary as usize].is_none();
            for (primary, secondary) in self.primary.iter_mut().zip(self.secondary.iter()) {
                *primary |= secondary;
            }
        }
        self.node_marks.begin();
        self.leaf_marks.begin();
        self.surface_marks.begin();
        self.emitted_marks.begin();
        self.walk_marks.begin();
        let result = self.query_loaded(world, origin, all_visible, hidden_areas, frustum);
        if result.is_err() {
            self.visible_count = 0;
            self.counters.surfaces = 0;
        }
        result
    }

    fn mark_ancestors(
        &mut self,
        world: &VisibilityWorld,
        leaf: usize,
    ) -> Result<(), VisibilityQueryError> {
        let mut count = 0;
        let mut edge = world.parent_heads[world.nodes.len() + leaf];
        loop {
            while edge != u32::MAX {
                let parent = world.parent_edges[edge as usize];
                edge = parent.next;
                if self.node_marks.contains(parent.node as usize) {
                    continue;
                }
                let Some(slot) = self.ancestors.get_mut(count) else {
                    return Err(VisibilityQueryError::ScratchSize);
                };
                *slot = parent.node;
                count += 1;
                self.node_marks.mark(parent.node as usize);
                self.counters.nodes_marked += 1;
            }
            if count == 0 {
                break;
            }
            count -= 1;
            edge = world.parent_heads[self.ancestors[count] as usize];
        }
        Ok(())
    }

    fn push_step(&mut self, count: &mut usize, step: WalkStep) -> Result<(), VisibilityQueryError> {
        let Some(slot) = self.walk.get_mut(*count) else {
            return Err(VisibilityQueryError::ScratchSize);
        };
        *slot = step;
        *count += 1;
        Ok(())
    }

    fn emit_surface(&mut self, surface: u32, key: u32) -> Result<(), VisibilityQueryError> {
        let index = surface as usize;
        if !self.surface_marks.contains(index) || self.emitted_marks.contains(index) {
            return Ok(());
        }
        let Some(slot) = self.visible.get_mut(self.visible_count) else {
            return Err(VisibilityQueryError::ScratchSize);
        };
        *slot = surface;
        self.depth_keys[self.visible_count] = key;
        self.visible_count += 1;
        self.emitted_marks.mark(index);
        self.counters.surfaces += 1;
        Ok(())
    }

    fn query_loaded(
        &mut self,
        world: &VisibilityWorld,
        origin: Vec3,
        all_visible: bool,
        hidden_areas: &[u8],
        frustum: &[Plane],
    ) -> Result<(), VisibilityQueryError> {
        for (index, leaf) in world.leaves.iter().enumerate() {
            if leaf.solid
                || leaf.area.is_some_and(|area| {
                    hidden_areas
                        .get(area as usize / 8)
                        .is_some_and(|bits| bits & (1 << (area & 7)) != 0)
                })
                || !all_visible
                    && !leaf.selector.is_some_and(|selector| {
                        self.primary[selector as usize / 8] & (1 << (selector & 7)) != 0
                    })
            {
                continue;
            }
            self.leaf_marks.mark(index);
            let first = leaf.surfaces.first as usize;
            for &surface in &world.leaf_surfaces[first..first + leaf.surfaces.count as usize] {
                self.surface_marks.mark(surface as usize);
            }
            self.mark_ancestors(world, index)?;
        }

        let mut pending = 0;
        let mut key = 0u32;
        self.push_step(
            &mut pending,
            WalkStep {
                child: world.root,
                planes: (1 << frustum.len()) - 1,
                emit_node: false,
            },
        )?;
        while pending != 0 {
            pending -= 1;
            let step = self.walk[pending];
            if step.emit_node {
                let node = &world.nodes[step.child as usize];
                for surface in node.surfaces.first..node.surfaces.first + node.surfaces.count {
                    self.emit_surface(surface, key)?;
                }
                if node.surfaces.count != 0 {
                    key += 1;
                }
                continue;
            }
            if step.child >= 0 {
                let index = step.child as usize;
                if !self.node_marks.contains(index) || self.walk_marks.contains(index) {
                    continue;
                }
                let node = &world.nodes[index];
                let Some(planes) = remaining_planes(node.bounds, frustum, step.planes) else {
                    self.counters.bounds_rejected += 1;
                    continue;
                };
                self.walk_marks.mark(index);
                self.counters.nodes_visited += 1;
                let plane = world.planes[node.plane as usize];
                let front = usize::from(plane.signed_distance(origin) < 0.0);
                // Native r_bsp.c: near child, splitting-node surfaces, far child.
                self.push_step(
                    &mut pending,
                    WalkStep {
                        child: node.children[front ^ 1],
                        planes,
                        emit_node: false,
                    },
                )?;
                self.push_step(
                    &mut pending,
                    WalkStep {
                        child: step.child,
                        planes,
                        emit_node: true,
                    },
                )?;
                self.push_step(
                    &mut pending,
                    WalkStep {
                        child: node.children[front],
                        planes,
                        emit_node: false,
                    },
                )?;
            } else {
                let index = (-1 - i64::from(step.child)) as usize;
                let walk_index = world.nodes.len() + index;
                if !self.leaf_marks.contains(index) || self.walk_marks.contains(walk_index) {
                    continue;
                }
                let leaf = &world.leaves[index];
                if remaining_planes(leaf.bounds, frustum, step.planes).is_none() {
                    self.counters.bounds_rejected += 1;
                    continue;
                }
                self.walk_marks.mark(walk_index);
                self.counters.leaves_visited += 1;
                let first = leaf.surfaces.first as usize;
                for &surface in &world.leaf_surfaces[first..first + leaf.surfaces.count as usize] {
                    if world.surface_owners[surface as usize].is_none() {
                        self.emit_surface(surface, key)?;
                    }
                }
                key += 1;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn successive_queries_preserve_leaf_surface_deduplication() -> Result<(), VisibilityError> {
        let world = VisibilityWorld::load(
            vec![],
            vec![],
            vec![VisLeaf {
                selector: None,
                area: None,
                solid: false,
                bounds: Bounds::default(),
                surfaces: SurfaceSpan { first: 0, count: 2 },
            }],
            vec![0, 0],
            1,
            -1,
            PvsRows::all_visible(0),
        )?;
        let mut view = ViewVisibility::new(&world);
        for _ in 0..3 {
            assert_eq!(
                view.query(&world, Vec3::default(), None, None, &[], &[]),
                Ok(())
            );
            assert_eq!(view.visible_surfaces(), &[0]);
            assert_eq!(view.counters().leaves_visited, 1);
            assert_eq!(view.counters().surfaces, 1);
        }
        Ok(())
    }
}
