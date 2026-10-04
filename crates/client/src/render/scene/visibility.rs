//! BSP visibility and source traversal.
//!
//! Donor provenance: `src/render/scene/visibility.ts` plus the Q1 RLE in
//! `src/formats/q1-map/queries.ts`. The unified [`WorldMap`] carries the
//! traversal subset (nodes, leaves, planes, leaf surfaces) of every BSP
//! family; family PVS payloads travel in [`WorldVisibility`].

use std::collections::{HashMap, HashSet};

use qa_core::math::{dot3, Bounds, Plane, Vec3};

use crate::materials::dlight::split_dlight_mask;
use crate::materials::q3_lighting::DynamicLight;
use crate::render::RenderError;
use crate::view::{bounds_in_frustum, camera_frustum, SceneCamera};

/// BSP family under traversal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorldKind {
    /// Quake 1.
    Q1,
    /// Quake 2.
    Q2,
    /// Quake 3.
    Q3,
}

/// One BSP tree child.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BspChild {
    /// Interior node index.
    Node(usize),
    /// Leaf index.
    Leaf(usize),
}

/// One interior BSP node.
#[derive(Debug, Clone, PartialEq)]
pub struct WorldNode {
    /// Splitting plane index.
    pub plane: usize,
    /// Front/back children.
    pub children: [BspChild; 2],
    /// Node bounds.
    pub bounds: Bounds,
}

/// One BSP leaf.
#[derive(Debug, Clone, PartialEq)]
pub struct WorldLeaf {
    /// Leaf bounds.
    pub bounds: Bounds,
    /// Visibility cluster (-1 when outside).
    pub cluster: i32,
    /// Area index.
    pub area: i32,
    /// Contents flags.
    pub contents: i32,
    /// First leaf surface.
    pub first_surface: usize,
    /// Leaf surface count.
    pub surface_count: usize,
    /// Q1 visibility lump offset (None sees everything).
    pub visibility_offset: Option<i32>,
    /// Q1 visible-leaf count for PVS row length.
    pub visible_leaves: usize,
}

/// Q2 per-cluster visibility offsets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Q2ClusterVis {
    /// PVS lump offset (negative sees everything).
    pub pvs_offset: i32,
}

/// Family visibility payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorldVisibility {
    /// No visibility data; everything is visible.
    None,
    /// Q1 compressed visibility lump.
    Q1 {
        /// Compressed bytes.
        data: Vec<u8>,
    },
    /// Q2 compressed visibility.
    Q2 {
        /// Compressed bytes.
        compressed: Vec<u8>,
        /// Per-cluster offsets.
        clusters: Vec<Q2ClusterVis>,
    },
    /// Q3 unpacked cluster bits.
    Q3 {
        /// Row-major cluster bits.
        bits: Vec<u8>,
        /// Cluster count.
        cluster_count: usize,
        /// Bytes per cluster row.
        bytes_per_cluster: usize,
    },
}

/// Traversal subset of a decoded BSP map.
#[derive(Debug, Clone, PartialEq)]
pub struct WorldMap {
    /// BSP family.
    pub kind: WorldKind,
    /// Interior nodes.
    pub nodes: Vec<WorldNode>,
    /// Leaves.
    pub leaves: Vec<WorldLeaf>,
    /// Splitting planes.
    pub planes: Vec<Plane>,
    /// Leaf surface membership.
    pub leaf_surfaces: Vec<usize>,
}

/// Visibility query options.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct WorldVisibilityOptions {
    /// PVS origin override (portal views).
    pub pvs_origin: Option<Vec3>,
    /// Ignore visibility data.
    pub no_vis: bool,
    /// Ignore frustum culling.
    pub no_cull: bool,
    /// Adapters normalize Q2 visible bits and Q3 excluded bits to this set.
    pub visible_areas: Option<HashSet<i32>>,
    /// Explicit Q2 secondary cluster.
    pub secondary_cluster: Option<i32>,
    /// Q3 dynamic lights for first-encounter masks.
    pub q3_lights: Vec<DynamicLight>,
}

/// Visible leaves, surfaces, and first-encounter light masks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VisibleWorld {
    /// Eye leaf (-1 when outside).
    pub leaf: i32,
    /// Visible leaves.
    pub leaves: Vec<usize>,
    /// Visible surfaces in first-encounter order.
    pub surfaces: Vec<usize>,
    /// First-encounter dlight mask per surface.
    pub surface_dlight_masks: HashMap<usize, u32>,
}

fn at<'a, T>(items: &'a [T], index: usize, what: &str) -> Result<&'a T, RenderError> {
    items.get(index).ok_or_else(|| RenderError::BadBatch {
        index,
        detail: format!("World visibility {what} {index} outside {}", items.len()),
    })
}

/// Leaf containing a point by BSP descent (-1 when the map has no leaves).
pub fn world_point_leaf(map: &WorldMap, point: Vec3) -> Result<i32, RenderError> {
    if map.nodes.is_empty() {
        return Ok(if map.leaves.is_empty() { -1 } else { 0 });
    }
    let mut child = BspChild::Node(0);
    while let BspChild::Node(index) = child {
        let node = at(&map.nodes, index, "node")?;
        let plane = at(&map.planes, node.plane, "plane")?;
        child = node.children[usize::from(dot3(point, plane.normal) <= plane.distance)];
    }
    match child {
        BspChild::Leaf(index) => Ok(index as i32),
        BspChild::Node(_) => unreachable!(),
    }
}

/// PVS family selecting the shared RLE core's error strings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PvsFamily {
    /// Quake 1.
    Q1,
    /// Quake 2.
    Q2,
}

/// Shared PVS RLE core: expand `output_len` bytes from `data` at `read.
///
/// Both families share the literal/zero-run encoding; only the row length
/// source and error strings differ, selected here by `family`.
fn decompress_pvs_row(data: &[u8], read: usize, output_len: usize, family: PvsFamily) -> Result<Vec<u8>, RenderError> {
    let truncated = || {
        RenderError::BadWire(if family == PvsFamily::Q1 {
            "Truncated Q1 PVS".to_string()
        } else {
            "Truncated Q2 PVS".to_string()
        })
    };
    let invalid = || {
        RenderError::BadWire(if family == PvsFamily::Q1 {
            "Invalid Q1 PVS run".to_string()
        } else {
            "Invalid Q2 PVS run".to_string()
        })
    };
    let mut output = vec![0u8; output_len];
    let mut read = read;
    let mut write = 0;
    while write < output.len() {
        let value = *data.get(read).ok_or_else(truncated)?;
        read += 1;
        if value != 0 {
            output[write] = value;
            write += 1;
            continue;
        }
        let count = *data.get(read).ok_or_else(truncated)?;
        read += 1;
        if count == 0 || write + count as usize > output.len() {
            return Err(invalid());
        }
        write += count as usize;
    }
    Ok(output)
}

/// Decompress one Q1 PVS row; null offsets and empty lumps see everything.
pub fn decompress_q1_pvs(data: &[u8], offset: Option<i32>, visible_leaves: usize) -> Result<Vec<u8>, RenderError> {
    let output_len = visible_leaves.div_ceil(8);
    if offset.is_none() || data.is_empty() {
        return Ok(vec![255; output_len]);
    }
    let offset = offset.unwrap_or(0);
    if offset < 0 {
        return Err(RenderError::BadWire("Truncated Q1 PVS".to_string()));
    }
    decompress_pvs_row(data, offset as usize, output_len, PvsFamily::Q1)
}

fn q2_pvs(compressed: &[u8], clusters: &[Q2ClusterVis], cluster: i32) -> Result<Option<Vec<u8>>, RenderError> {
    if cluster < 0 {
        return Ok(None);
    }
    let entry = at(clusters, cluster as usize, "cluster")?;
    if entry.pvs_offset < 0 {
        return Ok(None);
    }
    decompress_pvs_row(
        compressed,
        entry.pvs_offset as usize,
        clusters.len().div_ceil(8),
        PvsFamily::Q2,
    )
    .map(Some)
}

fn remaining_frustum_planes(bounds: &Bounds, planes: &[Plane], mut bits: u32) -> Option<u32> {
    for (index, plane) in planes.iter().enumerate() {
        let bit = 1u32 << index;
        if bits & bit == 0 {
            continue;
        }
        let front = Vec3 {
            x: if plane.normal.x < 0.0 {
                bounds.min.x
            } else {
                bounds.max.x
            },
            y: if plane.normal.y < 0.0 {
                bounds.min.y
            } else {
                bounds.max.y
            },
            z: if plane.normal.z < 0.0 {
                bounds.min.z
            } else {
                bounds.max.z
            },
        };
        if dot3(front, plane.normal) - plane.distance < 0.0 {
            return None;
        }
        let back = Vec3 {
            x: if plane.normal.x < 0.0 {
                bounds.max.x
            } else {
                bounds.min.x
            },
            y: if plane.normal.y < 0.0 {
                bounds.max.y
            } else {
                bounds.min.y
            },
            z: if plane.normal.z < 0.0 {
                bounds.max.z
            } else {
                bounds.min.z
            },
        };
        if dot3(back, plane.normal) - plane.distance >= 0.0 {
            bits &= !bit;
        }
    }
    Some(bits)
}

/// Compute the visible leaves and surfaces for a camera.
pub fn visible_world(
    map: &WorldMap,
    visibility: &WorldVisibility,
    camera: &SceneCamera,
    options: &WorldVisibilityOptions,
) -> Result<VisibleWorld, RenderError> {
    let origin = options.pvs_origin.unwrap_or(camera.origin);
    let eye = world_point_leaf(map, origin)?;
    let empty = VisibleWorld {
        leaf: eye,
        leaves: Vec::new(),
        surfaces: Vec::new(),
        surface_dlight_masks: HashMap::new(),
    };
    if eye < 0 {
        return Ok(empty);
    }
    let eye_index = eye as usize;
    let mut pvs: Option<Vec<u8>> = None;
    let mut from_cluster = -1;
    if !options.no_vis {
        match (&map.kind, visibility) {
            (WorldKind::Q1, WorldVisibility::Q1 { data }) => {
                let leaf = at(&map.leaves, eye_index, "leaf")?;
                pvs = Some(decompress_q1_pvs(
                    data,
                    if eye_index == 0 { None } else { leaf.visibility_offset },
                    leaf.visible_leaves,
                )?);
            }
            (WorldKind::Q2, WorldVisibility::Q2 { compressed, clusters }) => {
                from_cluster = at(&map.leaves, eye_index, "leaf")?.cluster;
                pvs = q2_pvs(compressed, clusters, from_cluster)?;
                let mut second = options.secondary_cluster;
                if second.is_none() {
                    let leaf = at(&map.leaves, eye_index, "leaf")?;
                    let probe = world_point_leaf(
                        map,
                        Vec3 {
                            x: origin.x,
                            y: origin.y,
                            z: origin.z + if leaf.contents == 0 { -16.0 } else { 16.0 },
                        },
                    )?;
                    if probe >= 0 {
                        let other = at(&map.leaves, probe as usize, "leaf")?;
                        if other.contents & 1 == 0 && other.cluster != from_cluster {
                            second = Some(other.cluster);
                        }
                    }
                }
                if let Some(second) = second {
                    if second >= 0 && pvs.is_some() {
                        match q2_pvs(compressed, clusters, second)? {
                            None => pvs = None,
                            Some(other) => {
                                if let Some(pvs) = pvs.as_mut() {
                                    for (index, byte) in other.iter().enumerate() {
                                        pvs[index] |= byte;
                                    }
                                }
                            }
                        }
                    }
                }
            }
            (
                WorldKind::Q3,
                WorldVisibility::Q3 {
                    bits,
                    cluster_count,
                    bytes_per_cluster,
                },
            ) => {
                from_cluster = at(&map.leaves, eye_index, "leaf")?.cluster;
                if from_cluster >= 0 && (from_cluster as usize) < *cluster_count {
                    let first = from_cluster as usize * bytes_per_cluster;
                    pvs = Some(bits[first..first + bytes_per_cluster].to_vec());
                }
            }
            _ => {}
        }
    }
    let frustum = camera_frustum(camera);
    let planes: &[Plane] = if options.no_cull {
        &[]
    } else if map.kind == WorldKind::Q3 {
        &frustum[..frustum.len().min(4)]
    } else {
        &frustum
    };
    let leaf_visible = |index: usize| -> Result<bool, RenderError> {
        if map.kind == WorldKind::Q1 {
            if index == 0 {
                return Ok(false);
            }
            return Ok(match &pvs {
                None => true,
                Some(pvs) => pvs[(index - 1) >> 3] & (1 << ((index - 1) & 7)) != 0,
            });
        }
        let leaf = at(&map.leaves, index, "leaf")?;
        if let Some(areas) = &options.visible_areas {
            if !areas.contains(&leaf.area) {
                return Ok(false);
            }
        }
        if map.kind == WorldKind::Q3 && from_cluster < 0 {
            return Ok(leaf.cluster != -1);
        }
        Ok(match &pvs {
            None => true,
            Some(pvs) => {
                leaf.cluster >= 0 && pvs[(leaf.cluster as usize) >> 3] & (1 << ((leaf.cluster as usize) & 7)) != 0
            }
        })
    };
    let lights = &options.q3_lights;
    if lights.len() > 32 {
        return Err(RenderError::Backend(
            "Q3 world lighting supports the source 32-light mask".to_string(),
        ));
    }
    let initial_mask = if lights.len() == 32 {
        u32::MAX
    } else {
        (1u32 << lights.len()) - 1
    };
    let mut leaves = Vec::new();
    let mut surfaces = Vec::new();
    let mut seen = HashSet::new();
    let mut surface_dlight_masks = HashMap::new();
    let root = if map.nodes.is_empty() {
        BspChild::Leaf(0)
    } else {
        BspChild::Node(0)
    };
    let mut pending = vec![(
        root,
        initial_mask,
        if planes.is_empty() {
            0
        } else {
            (1u32 << planes.len()) - 1
        },
    )];
    while let Some((child, mask, plane_bits)) = pending.pop() {
        match child {
            BspChild::Node(index) => {
                let node = at(&map.nodes, index, "node")?;
                let remaining = if map.kind == WorldKind::Q3 {
                    remaining_frustum_planes(&node.bounds, planes, plane_bits)
                } else if bounds_in_frustum(&node.bounds, planes) {
                    Some(plane_bits)
                } else {
                    None
                };
                let Some(remaining) = remaining else { continue };
                if map.kind == WorldKind::Q3 {
                    let plane = at(&map.planes, node.plane, "plane")?;
                    let masks = split_dlight_mask(lights, mask, plane);
                    pending.push((node.children[1], masks.1, remaining));
                    pending.push((node.children[0], masks.0, remaining));
                } else {
                    let plane = at(&map.planes, node.plane, "plane")?;
                    let front = usize::from(dot3(camera.origin, plane.normal) < plane.distance);
                    pending.push((node.children[1 - front], mask, remaining));
                    pending.push((node.children[front], mask, remaining));
                }
            }
            BspChild::Leaf(index) => {
                if !leaf_visible(index)? {
                    continue;
                }
                let leaf = at(&map.leaves, index, "leaf")?;
                let culled = if map.kind == WorldKind::Q3 {
                    remaining_frustum_planes(&leaf.bounds, planes, plane_bits).is_none()
                } else {
                    !bounds_in_frustum(&leaf.bounds, planes)
                };
                if culled {
                    continue;
                }
                leaves.push(index);
                for i in 0..leaf.surface_count {
                    let surface = *at(&map.leaf_surfaces, leaf.first_surface + i, "leaf surface")?;
                    if seen.insert(surface) {
                        surfaces.push(surface);
                        surface_dlight_masks.insert(surface, mask);
                    }
                }
            }
        }
    }
    Ok(VisibleWorld {
        leaf: eye,
        leaves,
        surfaces,
        surface_dlight_masks,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::view::{CameraClip, Rect};
    use qa_core::math::vec3;

    fn camera_at(origin: Vec3) -> SceneCamera {
        SceneCamera {
            origin,
            axis: [vec3(1.0, 0.0, 0.0), vec3(0.0, 1.0, 0.0), vec3(0.0, 0.0, 1.0)],
            projection: [
                1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, -1.0, -1.0, 0.0, 0.0, -8.0, 0.0,
            ],
            viewport: Rect {
                x: 0,
                y: 0,
                width: 64,
                height: 64,
            },
            clip: CameraClip::None,
        }
    }

    fn two_leaf_map(kind: WorldKind) -> WorldMap {
        WorldMap {
            kind,
            nodes: vec![WorldNode {
                plane: 0,
                children: [BspChild::Leaf(1), BspChild::Leaf(2)],
                bounds: Bounds {
                    min: vec3(-64.0, -64.0, -64.0),
                    max: vec3(64.0, 64.0, 64.0),
                },
            }],
            leaves: vec![
                WorldLeaf {
                    bounds: Bounds {
                        min: vec3(0.0, 0.0, 0.0),
                        max: vec3(0.0, 0.0, 0.0),
                    },
                    cluster: -1,
                    area: 0,
                    contents: 1,
                    first_surface: 0,
                    surface_count: 0,
                    visibility_offset: None,
                    visible_leaves: 2,
                },
                WorldLeaf {
                    bounds: Bounds {
                        min: vec3(0.0, -64.0, -64.0),
                        max: vec3(64.0, 64.0, 64.0),
                    },
                    cluster: 0,
                    area: 1,
                    contents: 0,
                    first_surface: 0,
                    surface_count: 1,
                    visibility_offset: Some(0),
                    visible_leaves: 2,
                },
                WorldLeaf {
                    bounds: Bounds {
                        min: vec3(-64.0, -64.0, -64.0),
                        max: vec3(0.0, 64.0, 64.0),
                    },
                    cluster: 1,
                    area: 1,
                    contents: 0,
                    first_surface: 1,
                    surface_count: 1,
                    visibility_offset: Some(1),
                    visible_leaves: 2,
                },
            ],
            planes: vec![Plane {
                normal: vec3(1.0, 0.0, 0.0),
                distance: 0.0,
            }],
            leaf_surfaces: vec![10, 11],
        }
    }

    #[test]
    fn point_leaf_descends_by_plane_side() {
        let map = two_leaf_map(WorldKind::Q3);
        assert_eq!(world_point_leaf(&map, vec3(5.0, 0.0, 0.0)).unwrap(), 1);
        assert_eq!(world_point_leaf(&map, vec3(-5.0, 0.0, 0.0)).unwrap(), 2);
        assert_eq!(world_point_leaf(&map, vec3(0.0, 0.0, 0.0)).unwrap(), 2);
    }

    #[test]
    fn empty_tree_returns_solid_or_outside() {
        let map = WorldMap {
            kind: WorldKind::Q1,
            nodes: Vec::new(),
            leaves: Vec::new(),
            planes: Vec::new(),
            leaf_surfaces: Vec::new(),
        };
        assert_eq!(world_point_leaf(&map, vec3(0.0, 0.0, 0.0)).unwrap(), -1);
    }

    #[test]
    fn q1_pvs_gates_leaves() {
        let map = two_leaf_map(WorldKind::Q1);
        let camera = camera_at(vec3(5.0, 0.0, 0.0));
        let visibility = WorldVisibility::Q1 { data: vec![0b01, 0b10] };
        let options = WorldVisibilityOptions {
            no_cull: true,
            ..Default::default()
        };
        let visible = visible_world(&map, &visibility, &camera, &options).unwrap();
        assert_eq!(visible.leaf, 1);
        assert_eq!(visible.leaves, vec![1]);
        assert_eq!(visible.surfaces, vec![10]);
        assert_eq!(visible.surface_dlight_masks.get(&10), Some(&0));
    }

    #[test]
    fn q2_pvs_merges_secondary_cluster() {
        let map = two_leaf_map(WorldKind::Q2);
        let camera = camera_at(vec3(5.0, 0.0, 0.0));
        let visibility = WorldVisibility::Q2 {
            compressed: vec![0b01, 0b10],
            clusters: vec![Q2ClusterVis { pvs_offset: 0 }, Q2ClusterVis { pvs_offset: 1 }],
        };
        let options = WorldVisibilityOptions {
            no_cull: true,
            secondary_cluster: Some(1),
            ..Default::default()
        };
        let visible = visible_world(&map, &visibility, &camera, &options).unwrap();
        assert_eq!(visible.leaf, 1);
        assert!(visible.leaves.contains(&1));
        assert!(visible.leaves.contains(&2));
        assert_eq!(visible.surfaces.len(), 2);
    }

    #[test]
    fn q3_area_filter_and_cluster_bits_apply() {
        let map = two_leaf_map(WorldKind::Q3);
        let camera = camera_at(vec3(5.0, 0.0, 0.0));
        let visibility = WorldVisibility::Q3 {
            bits: vec![0b01, 0b11],
            cluster_count: 2,
            bytes_per_cluster: 1,
        };
        let mut areas = HashSet::new();
        areas.insert(1);
        let options = WorldVisibilityOptions {
            no_cull: true,
            visible_areas: Some(areas),
            ..Default::default()
        };
        let visible = visible_world(&map, &visibility, &camera, &options).unwrap();
        assert_eq!(visible.surfaces, vec![10]);
        let blocked = WorldVisibilityOptions {
            no_cull: true,
            visible_areas: Some(HashSet::new()),
            ..Default::default()
        };
        let visible = visible_world(&map, &visibility, &camera, &blocked).unwrap();
        assert!(visible.surfaces.is_empty());
    }

    #[test]
    fn frustum_cull_excludes_side_leaves() {
        let mut map = two_leaf_map(WorldKind::Q3);
        map.leaves[2].bounds.min.x = -64.0;
        map.leaves[2].bounds.max.x = -60.0;
        let camera = camera_at(vec3(5.0, 0.0, 0.0));
        let visible = visible_world(
            &map,
            &WorldVisibility::None,
            &camera,
            &WorldVisibilityOptions::default(),
        )
        .unwrap();
        assert!(visible.leaves.contains(&1));
        assert!(!visible.leaves.contains(&2));
    }

    #[test]
    fn too_many_q3_lights_fail() {
        let map = two_leaf_map(WorldKind::Q3);
        let camera = camera_at(vec3(5.0, 0.0, 0.0));
        let lights = vec![
            DynamicLight {
                origin: vec3(0.0, 0.0, 0.0),
                radius: 10.0,
                color: vec3(1.0, 1.0, 1.0),
                additive: false,
            };
            33
        ];
        let options = WorldVisibilityOptions {
            q3_lights: lights,
            ..Default::default()
        };
        assert!(visible_world(&map, &WorldVisibility::None, &camera, &options).is_err());
    }

    #[test]
    fn q1_rle_round_trip_and_errors() {
        assert_eq!(decompress_q1_pvs(&[], None, 9).unwrap(), vec![255, 255]);
        assert_eq!(
            decompress_q1_pvs(&[0x05, 0x00, 0x01], Some(0), 16).unwrap(),
            vec![0x05, 0x00]
        );
        assert!(decompress_q1_pvs(&[0x01], Some(0), 16).is_err());
        assert!(decompress_q1_pvs(&[0x00, 0x00], Some(0), 8).is_err());
    }

    #[test]
    fn shared_pvs_core_matches_both_families() {
        let data = [0x05, 0x00, 0x01, 0xFF];
        assert_eq!(
            decompress_pvs_row(&data, 0, 2, PvsFamily::Q1).unwrap(),
            decompress_pvs_row(&data, 0, 2, PvsFamily::Q2).unwrap()
        );
        assert_eq!(decompress_q1_pvs(&data, Some(0), 16).unwrap(), vec![0x05, 0x00]);
        let clusters = vec![Q2ClusterVis { pvs_offset: 0 }];
        assert_eq!(
            q2_pvs(&data, &clusters, 0).unwrap(),
            Some(decompress_q1_pvs(&data, Some(0), 8).unwrap())
        );
        assert_eq!(q2_pvs(&data, &clusters, -1).unwrap(), None);
        let wide = vec![Q2ClusterVis { pvs_offset: 0 }; 9];
        assert!(q2_pvs(&[0x01], &wide, 0).is_err());
        let q1_err = decompress_pvs_row(&[0x01], 0, 2, PvsFamily::Q1).expect_err("q1 truncates");
        let q2_err = decompress_pvs_row(&[0x01], 0, 2, PvsFamily::Q2).expect_err("q2 truncates");
        assert_eq!(q1_err, RenderError::BadWire("Truncated Q1 PVS".to_string()));
        assert_eq!(q2_err, RenderError::BadWire("Truncated Q2 PVS".to_string()));
    }
}
