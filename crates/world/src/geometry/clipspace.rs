//! Quake clip-space reconstruction: native hull bounds plus convex BSP
//! clipping of the id Software brush/winding representation.
//!
//! Donor provenance: `/home/buzzkill/Projects/quake-typescript/src/world/geometry/q1-solid/clipspace.ts`.

use crate::hull::{ClipChild, Hull};
use crate::WorldError;

use super::polyhedron::{
    add, box_cell, box_separating_planes, dot, scale, split_cell, sub, ConvexCell, DBounds, DPlane, DVec3, AXES,
};

/// Collision hull with the actor clip bounds the donor stores on `Q1Hull`.
#[derive(Debug, Clone)]
pub struct Q1ClipHull<'a> {
    /// Hull planes and clip tree.
    pub hull: &'a Hull,
    /// Actor box the hull was compiled for.
    pub clip_bounds: DBounds,
}

fn free_cells(hull: &Q1ClipHull, envelope: &DBounds) -> Result<Vec<ConvexCell>, WorldError> {
    let root = if hull.hull.first < 0 {
        ClipChild::Contents(hull.hull.first)
    } else {
        #[allow(clippy::cast_sign_loss)]
        ClipChild::Node(hull.hull.first as usize)
    };
    let mut cells = Vec::new();
    let mut stack = vec![(root, box_cell(envelope), 0usize)];
    while let Some((child, cell, depth)) = stack.pop() {
        match child {
            ClipChild::Contents(value) => {
                if value != -2 {
                    cells.push(cell);
                }
            }
            ClipChild::Node(index) => {
                if depth > hull.hull.clipnodes.len() {
                    return Err(WorldError::ClipCycle);
                }
                let node = hull.hull.clipnodes.get(index).ok_or(WorldError::BadClipNode)?;
                let plane = hull.hull.planes.get(node.plane).ok_or(WorldError::BadClipNode)?;
                let split = split_cell(
                    &cell,
                    DPlane {
                        normal: DVec3::from(plane.normal),
                        distance: f64::from(plane.distance),
                    },
                );
                if let Some(back) = split.back {
                    stack.push((node.children[1], back, depth + 1));
                }
                if let Some(front) = split.front {
                    stack.push((node.children[0], front, depth + 1));
                }
            }
        }
    }
    Ok(cells)
}

/// Subtract a convex region while retaining a convex decomposition of the rest.
fn subtract_region(cell: &ConvexCell, planes: &[DPlane]) -> Vec<ConvexCell> {
    let mut outside = Vec::new();
    let mut inside = Some(cell.clone());
    for plane in planes {
        let Some(current) = inside else {
            break;
        };
        let split = split_cell(&current, *plane);
        if let Some(remainder) = split.front {
            outside.push(remainder);
        }
        inside = split.back;
    }
    outside
}

/// A compiled hull S for actor box A represents solid expanded by -A.
/// S eroded by -A equals the complement of free(S) expanded by A.
/// This preserves compiler-removed clip solids for arbitrary future shapes.
/// Intersecting the two native reconstructions uses both available hulls.
/// The result is derived occupancy: lost narrow concavities can remain closed,
/// and compiler-specific bevels remain part of the available evidence.
pub fn derive_q1_clip_solids(hulls: &[Q1ClipHull], envelope: &DBounds) -> Result<Vec<ConvexCell>, WorldError> {
    let mut solids = vec![box_cell(envelope)];
    let mut used = false;
    for hull in hulls.iter().skip(1) {
        if hull.hull.first < 0 && hull.hull.first != -2 {
            continue;
        }
        used = true;
        let size = &hull.clip_bounds;
        let center = scale(add(size.min, size.max), 0.5);
        let extents = scale(sub(size.max, size.min), 0.5);
        let pad = DVec3 { x: 1.0, y: 1.0, z: 1.0 };
        let free_envelope = DBounds {
            min: sub(envelope.min, add(size.max, pad)),
            max: sub(envelope.max, sub(size.min, pad)),
        };
        for free in free_cells(hull, &free_envelope)? {
            let expanded: Vec<DPlane> = box_separating_planes(&free, &AXES)
                .into_iter()
                .map(|plane| DPlane {
                    normal: plane.normal,
                    distance: plane.distance
                        + dot(plane.normal, center)
                        + plane.normal.x.abs() * extents.x
                        + plane.normal.y.abs() * extents.y
                        + plane.normal.z.abs() * extents.z,
                })
                .collect();
            solids = solids
                .iter()
                .flat_map(|cell| subtract_region(cell, &expanded))
                .collect();
            if solids.is_empty() {
                return Ok(solids);
            }
        }
    }
    Ok(if used { solids } else { Vec::new() })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hull::{axis_box_hull, BspPlane, ClipNode};
    use qa_core::math::{vec3, Bounds};

    fn envelope() -> DBounds {
        DBounds::from(Bounds {
            min: vec3(-64.0, -64.0, -64.0),
            max: vec3(64.0, 64.0, 64.0),
        })
    }

    #[test]
    fn empty_hull_list_yields_no_solids() {
        assert!(derive_q1_clip_solids(&[], &envelope()).unwrap().is_empty());
    }

    #[test]
    fn open_hull_carves_everything() {
        // Hull 0 is the drawing hull and is always skipped; hull 1 rooted in
        // empty contents frees the whole envelope, leaving no solids.
        let drawing = axis_box_hull(&Bounds {
            min: vec3(-8.0, -8.0, -8.0),
            max: vec3(8.0, 8.0, 8.0),
        });
        let open = Hull {
            planes: Vec::new(),
            clipnodes: Vec::new(),
            first: -1,
            last: -1,
        };
        let hulls = [
            Q1ClipHull {
                hull: &drawing,
                clip_bounds: envelope(),
            },
            Q1ClipHull {
                hull: &open,
                clip_bounds: DBounds {
                    min: DVec3::from(vec3(-16.0, -16.0, -24.0)),
                    max: DVec3::from(vec3(16.0, 16.0, 32.0)),
                },
            },
        ];
        assert!(derive_q1_clip_solids(&hulls, &envelope()).unwrap().is_empty());
    }

    #[test]
    fn solid_hull_keeps_the_envelope() {
        let drawing = axis_box_hull(&Bounds {
            min: vec3(-8.0, -8.0, -8.0),
            max: vec3(8.0, 8.0, 8.0),
        });
        let solid = Hull {
            planes: Vec::new(),
            clipnodes: Vec::new(),
            first: -2,
            last: -2,
        };
        let zero = DBounds {
            min: DVec3::from(vec3(0.0, 0.0, 0.0)),
            max: DVec3::from(vec3(0.0, 0.0, 0.0)),
        };
        let hulls = [
            Q1ClipHull {
                hull: &drawing,
                clip_bounds: zero,
            },
            Q1ClipHull {
                hull: &solid,
                clip_bounds: zero,
            },
        ];
        let solids = derive_q1_clip_solids(&hulls, &envelope()).unwrap();
        assert_eq!(solids.len(), 1);
        assert_eq!(solids[0].faces.len(), 6);
    }

    #[test]
    fn invalid_clip_tree_errors() {
        let drawing = axis_box_hull(&Bounds {
            min: vec3(-8.0, -8.0, -8.0),
            max: vec3(8.0, 8.0, 8.0),
        });
        let broken = Hull {
            planes: vec![BspPlane {
                normal: vec3(1.0, 0.0, 0.0),
                distance: 0.0,
                plane_type: 0,
                signbits: 0,
            }],
            clipnodes: vec![ClipNode {
                plane: 7,
                children: [ClipChild::Contents(-1), ClipChild::Contents(-1)],
            }],
            first: 0,
            last: 0,
        };
        let zero = DBounds {
            min: DVec3::from(vec3(0.0, 0.0, 0.0)),
            max: DVec3::from(vec3(0.0, 0.0, 0.0)),
        };
        let hulls = [
            Q1ClipHull {
                hull: &drawing,
                clip_bounds: zero,
            },
            Q1ClipHull {
                hull: &broken,
                clip_bounds: zero,
            },
        ];
        assert_eq!(
            derive_q1_clip_solids(&hulls, &envelope()).unwrap_err(),
            WorldError::BadClipNode
        );
    }
}
