//! Solid BSP leaf cells: derived collision geometry, not recovered authored
//! brushes. Native clipnodes remain a separate source authority.
//!
//! Donor provenance: `/home/buzzkill/Projects/quake-typescript/src/world/geometry/q1-solid/index.ts`.

use crate::WorldError;

use super::polyhedron::{box_cell, split_cell, ConvexCell, DBounds, DPlane};

/// BSP child: another node or a leaf index.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SolidChild {
    /// Node index.
    Node(usize),
    /// Leaf index.
    Leaf(usize),
}

/// BSP node view for solid-space traversal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Q1SolidNode {
    /// Plane index.
    pub plane: usize,
    /// Front and back children.
    pub children: [SolidChild; 2],
}

/// BSP leaf view for solid-space traversal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Q1SolidLeaf {
    /// Leaf contents.
    pub contents: i32,
}

/// World model view: hull head nodes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Q1SolidModel {
    /// Head node per hull; index 0 is the drawing hull.
    pub headnodes: Vec<i32>,
}

/// World geometry view feeding [`Q1SolidSpace`]; the wiring layer converts
/// parsed BSP maps into this headless shape.
#[derive(Debug, Clone, PartialEq)]
pub struct Q1SolidGeometry {
    /// Models.
    pub models: Vec<Q1SolidModel>,
    /// BSP nodes.
    pub nodes: Vec<Q1SolidNode>,
    /// Planes.
    pub planes: Vec<DPlane>,
    /// Leaves.
    pub leaves: Vec<Q1SolidLeaf>,
}

/// One solid leaf cell.
#[derive(Debug, Clone, PartialEq)]
pub struct Q1SolidCell {
    /// Clipped convex cell.
    pub cell: ConvexCell,
    /// Leaf index.
    pub leaf: usize,
    /// Leaf contents.
    pub contents: i32,
}

/// Leaf-cell space over Q1 world geometry.
#[derive(Debug, Clone, PartialEq)]
pub struct Q1SolidSpace {
    /// World geometry.
    pub geometry: Q1SolidGeometry,
}

impl Q1SolidSpace {
    /// Representation tag.
    pub const REPRESENTATION: &'static str = "bsp-leaf-cells";

    /// Build a space over `geometry`.
    #[must_use]
    pub fn new(geometry: Q1SolidGeometry) -> Self {
        Self { geometry }
    }

    /// A bounded swept shape only needs the portions of infinite leaves inside
    /// its padded envelope. Repeated leaf zero references remain separate cells.
    pub fn cells(
        &self,
        envelope: &DBounds,
        model_index: usize,
        include: &dyn Fn(i32) -> bool,
    ) -> Result<Vec<Q1SolidCell>, WorldError> {
        let model = self
            .geometry
            .models
            .get(model_index)
            .ok_or(WorldError::UnknownQ1Model(model_index as i32))?;
        let root = model.headnodes.first().copied().ok_or(WorldError::MissingDrawingHull)?;
        let first = if root < 0 {
            #[allow(clippy::cast_sign_loss)]
            SolidChild::Leaf((-1 - root) as usize)
        } else {
            #[allow(clippy::cast_sign_loss)]
            SolidChild::Node(root as usize)
        };
        let mut found = Vec::new();
        let mut stack = vec![(first, box_cell(envelope), 0usize)];
        while let Some((child, cell, depth)) = stack.pop() {
            match child {
                SolidChild::Leaf(index) => {
                    let leaf = self.geometry.leaves.get(index).ok_or(WorldError::UnknownSolidLeaf)?;
                    if include(leaf.contents) {
                        found.push(Q1SolidCell {
                            cell,
                            leaf: index,
                            contents: leaf.contents,
                        });
                    }
                }
                SolidChild::Node(index) => {
                    if depth > self.geometry.nodes.len() {
                        return Err(WorldError::SolidCycle);
                    }
                    let node = self.geometry.nodes.get(index).ok_or(WorldError::BadSolidNode)?;
                    let plane = self.geometry.planes.get(node.plane).ok_or(WorldError::BadSolidNode)?;
                    let split = split_cell(&cell, *plane);
                    if let Some(back) = split.back {
                        stack.push((node.children[1], back, depth + 1));
                    }
                    if let Some(front) = split.front {
                        stack.push((node.children[0], front, depth + 1));
                    }
                }
            }
        }
        Ok(found)
    }

    /// Solid (`-2`) cells of a model.
    pub fn solid_cells(&self, envelope: &DBounds, model_index: usize) -> Result<Vec<Q1SolidCell>, WorldError> {
        self.cells(envelope, model_index, &|contents| contents == -2)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geometry::polyhedron::{dvec3, AXES};

    fn single_solid_leaf() -> Q1SolidGeometry {
        Q1SolidGeometry {
            models: vec![Q1SolidModel { headnodes: vec![-1] }],
            nodes: Vec::new(),
            planes: Vec::new(),
            leaves: vec![Q1SolidLeaf { contents: -2 }],
        }
    }

    fn envelope() -> DBounds {
        DBounds {
            min: dvec3(-16.0, -16.0, -16.0),
            max: dvec3(16.0, 16.0, 16.0),
        }
    }

    #[test]
    fn representation_tag_matches_donor() {
        assert_eq!(Q1SolidSpace::REPRESENTATION, "bsp-leaf-cells");
    }

    #[test]
    fn leaf_root_yields_filtered_cells() {
        let space = Q1SolidSpace::new(single_solid_leaf());
        let solids = space.solid_cells(&envelope(), 0).unwrap();
        assert_eq!(solids.len(), 1);
        assert_eq!(solids[0].leaf, 0);
        assert_eq!(solids[0].contents, -2);
        let empty = space.cells(&envelope(), 0, &|contents| contents == -1).unwrap();
        assert!(empty.is_empty());
    }

    #[test]
    fn node_split_clips_both_leaves() {
        let geometry = Q1SolidGeometry {
            models: vec![Q1SolidModel { headnodes: vec![0] }],
            nodes: vec![Q1SolidNode {
                plane: 0,
                children: [SolidChild::Leaf(0), SolidChild::Leaf(1)],
            }],
            planes: vec![super::DPlane {
                normal: AXES[0],
                distance: 0.0,
            }],
            leaves: vec![Q1SolidLeaf { contents: -2 }, Q1SolidLeaf { contents: -2 }],
        };
        let space = Q1SolidSpace::new(geometry);
        let solids = space.solid_cells(&envelope(), 0).unwrap();
        assert_eq!(solids.len(), 2);
    }

    #[test]
    fn bad_geometry_errors() {
        let space = Q1SolidSpace::new(single_solid_leaf());
        assert_eq!(
            space.solid_cells(&envelope(), 3).unwrap_err(),
            WorldError::UnknownQ1Model(3)
        );
        let no_hull = Q1SolidSpace::new(Q1SolidGeometry {
            models: vec![Q1SolidModel { headnodes: Vec::new() }],
            nodes: Vec::new(),
            planes: Vec::new(),
            leaves: Vec::new(),
        });
        assert_eq!(
            no_hull.solid_cells(&envelope(), 0).unwrap_err(),
            WorldError::MissingDrawingHull
        );
        let bad_leaf = Q1SolidSpace::new(Q1SolidGeometry {
            models: vec![Q1SolidModel { headnodes: vec![-9] }],
            nodes: Vec::new(),
            planes: Vec::new(),
            leaves: Vec::new(),
        });
        assert_eq!(
            bad_leaf.solid_cells(&envelope(), 0).unwrap_err(),
            WorldError::UnknownSolidLeaf
        );
    }
}
