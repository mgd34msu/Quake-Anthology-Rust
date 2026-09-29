//! AAS v4/v5 records and BSP sampling from id Software's `aasfile.h`,
//! `be_aas_file.c`, and `be_aas_sample.c`, ported from
//! `src/bots/navigation/aas.ts`.
//! Copyright (C) 1999-2005 Id Software, Inc.

use qa_core::binary::{BinaryError, BinaryReader};
use qa_core::math::{Bounds, Vec3};

use crate::error::BotsError;

/// AAS magic (`EAAS`).
const AAS_MAGIC: u32 = 0x5341_4145;
/// AAS header length in bytes.
const AAS_HEADER_LEN: usize = 124;
/// AAS lump count.
const AAS_LUMPS: usize = 14;

/// AAS lump span.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AasLump {
    /// File offset.
    pub offset: i32,
    /// Byte length.
    pub length: i32,
}

/// AAS presence bounding box.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AasBbox {
    /// Presence bits.
    pub presence: i32,
    /// Flags.
    pub flags: i32,
    /// Bounds.
    pub bounds: Bounds,
}

/// AAS plane.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AasPlane {
    /// Plane normal.
    pub normal: Vec3,
    /// Plane distance.
    pub distance: f32,
    /// Plane type.
    pub plane_type: i32,
}

/// AAS edge between two vertices.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AasEdge {
    /// Endpoint vertex indexes.
    pub vertices: [i32; 2],
}

/// AAS face.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AasFace {
    /// Plane index.
    pub plane: i32,
    /// Face flags.
    pub flags: i32,
    /// Edge count.
    pub edge_count: i32,
    /// First edge index.
    pub first_edge: i32,
    /// Front area.
    pub front_area: i32,
    /// Back area.
    pub back_area: i32,
}

/// AAS area.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AasArea {
    /// Area number; always its own ordinal.
    pub number: i32,
    /// Face count.
    pub face_count: i32,
    /// First face index.
    pub first_face: i32,
    /// Area bounds.
    pub bounds: Bounds,
    /// Area center.
    pub center: Vec3,
}

/// AAS area settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AasAreaSettings {
    /// Area contents.
    pub contents: i32,
    /// Area flags.
    pub flags: i32,
    /// Area presence.
    pub presence: i32,
    /// Cluster number, or the negated portal number.
    pub cluster: i32,
    /// Index within the cluster.
    pub cluster_area: i32,
    /// Reachability count.
    pub reach_count: i32,
    /// First reachability index.
    pub first_reach: i32,
}

/// AAS reachability record.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AasReachability {
    /// Destination area.
    pub area: i32,
    /// Source face.
    pub face: i32,
    /// Source edge.
    pub edge: i32,
    /// Traversal start.
    pub start: Vec3,
    /// Traversal end.
    pub end: Vec3,
    /// Travel type with team bits.
    pub travel_type: i32,
    /// Travel time in centiseconds.
    pub travel_time: u16,
    /// Padding.
    pub padding: u16,
}

/// AAS BSP node. Positive children are nodes, negative children are
/// areas, and zero is solid.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AasNode {
    /// Plane index.
    pub plane: i32,
    /// Child references.
    pub children: [i32; 2],
}

/// AAS cluster portal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AasPortal {
    /// Portal area.
    pub area: i32,
    /// Front cluster.
    pub front_cluster: i32,
    /// Back cluster.
    pub back_cluster: i32,
    /// Cluster-area numbers on each side.
    pub cluster_areas: [i32; 2],
}

/// AAS cluster.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AasCluster {
    /// Area count.
    pub area_count: i32,
    /// Reachability-area count.
    pub reachability_area_count: i32,
    /// Portal count.
    pub portal_count: i32,
    /// First portal index.
    pub first_portal: i32,
}

/// Parsed AAS asset.
#[derive(Debug, Clone, PartialEq)]
pub struct AasAsset {
    /// Source name.
    pub source: String,
    /// Format version.
    pub version: i32,
    /// BSP checksum this AAS was built against.
    pub bsp_checksum: i32,
    /// Lump spans.
    pub lumps: Vec<AasLump>,
    /// Presence bounding boxes.
    pub bboxes: Vec<AasBbox>,
    /// Vertices.
    pub vertices: Vec<Vec3>,
    /// Planes.
    pub planes: Vec<AasPlane>,
    /// Edges.
    pub edges: Vec<AasEdge>,
    /// Signed face edge indexes.
    pub edge_indexes: Vec<i32>,
    /// Faces.
    pub faces: Vec<AasFace>,
    /// Signed area face indexes.
    pub face_indexes: Vec<i32>,
    /// Areas.
    pub areas: Vec<AasArea>,
    /// Per-area settings.
    pub settings: Vec<AasAreaSettings>,
    /// Reachability records.
    pub reachability: Vec<AasReachability>,
    /// BSP nodes.
    pub nodes: Vec<AasNode>,
    /// Cluster portals.
    pub portals: Vec<AasPortal>,
    /// Per-cluster portal indexes.
    pub portal_indexes: Vec<i32>,
    /// Clusters.
    pub clusters: Vec<AasCluster>,
}

fn read_vector(reader: &mut BinaryReader<'_>) -> Result<Vec3, BinaryError> {
    Ok(Vec3 {
        x: reader.finite_f32()?,
        y: reader.finite_f32()?,
        z: reader.finite_f32()?,
    })
}

fn read_bounds(reader: &mut BinaryReader<'_>) -> Result<Bounds, BinaryError> {
    Ok(Bounds {
        min: read_vector(reader)?,
        max: read_vector(reader)?,
    })
}

fn read_pair(reader: &mut BinaryReader<'_>) -> Result<[i32; 2], BinaryError> {
    Ok([reader.i32()?, reader.i32()?])
}

fn check_index(value: i64, count: usize, name: &str) -> Result<(), BotsError> {
    if value < 0 || value as usize >= count {
        return Err(BotsError::OutOfRange {
            what: match name {
                "vertex" => "AAS vertex index",
                "edge" => "AAS edge index",
                "face plane" => "AAS face plane index",
                "front area" => "AAS front area index",
                "back area" => "AAS back area index",
                "face" => "AAS face index",
                "reachable area" => "AAS reachable area index",
                "node plane" => "AAS node plane index",
                "node child" => "AAS node child index",
                "portal area" => "AAS portal area index",
                "front cluster" => "AAS front cluster index",
                "back cluster" => "AAS back cluster index",
                "portal" => "AAS portal index",
                _ => "AAS index",
            },
            index: value,
            length: count,
        });
    }
    Ok(())
}

fn check_range(first: i32, count: i32, length: usize, name: &'static str) -> Result<(), BotsError> {
    if first < 0 || count < 0 || count as usize > length || first as usize > length - count as usize {
        return Err(BotsError::OutOfRange {
            what: name,
            index: i64::from(first),
            length,
        });
    }
    Ok(())
}

/// Parse AAS bytes, optionally verifying the expected BSP checksum.
pub fn parse_aas(bytes: &[u8], source: &str, expected_bsp_checksum: Option<i32>) -> Result<AasAsset, BotsError> {
    let mut reader = BinaryReader::new(bytes, source);
    if reader.u32()? != AAS_MAGIC {
        return Err(BinaryError::custom(source, 0, "expected EAAS magic").into());
    }
    let version = reader.i32()?;
    if version != 4 && version != 5 {
        return Err(BinaryError::custom(source, 4, format!("unsupported AAS version {version}")).into());
    }
    let mut decoded = reader.bytes(AAS_HEADER_LEN - 8)?;
    if version == 5 {
        for (offset, byte) in decoded.iter_mut().enumerate() {
            *byte ^= ((offset * 119) & 255) as u8;
        }
    }
    let mut header = BinaryReader::new(&decoded, &format!("{source}:header"));
    let bsp_checksum = header.i32()?;
    if let Some(expected) = expected_bsp_checksum {
        if bsp_checksum != expected {
            return Err(BinaryError::custom(source, 8, "AAS belongs to a different BSP checksum").into());
        }
    }
    let mut lumps = Vec::with_capacity(AAS_LUMPS);
    for lump in 0..AAS_LUMPS {
        let offset = header.i32()?;
        let length = header.i32()?;
        if length < 0
            || length > 0
                && (offset < AAS_HEADER_LEN as i32 || offset as usize > bytes.len().saturating_sub(length as usize))
        {
            return Err(BinaryError::custom(source, 12 + lump * 8, "invalid AAS lump range").into());
        }
        lumps.push(AasLump { offset, length });
    }
    let read_lump = |lump: usize, stride: usize| -> Result<BinaryReader<'_>, BotsError> {
        let span = lumps
            .get(lump)
            .ok_or_else(|| BotsError::Internal("Missing AAS lump descriptor".to_string()))?;
        Ok(reader.records(span.offset as usize, span.length as usize, stride)?)
    };
    let mut bboxes = Vec::new();
    let mut data = read_lump(0, 32)?;
    while data.remaining() > 0 {
        bboxes.push(AasBbox {
            presence: data.i32()?,
            flags: data.i32()?,
            bounds: read_bounds(&mut data)?,
        });
    }
    let mut vertices = Vec::new();
    let mut data = read_lump(1, 12)?;
    while data.remaining() > 0 {
        vertices.push(read_vector(&mut data)?);
    }
    let mut planes = Vec::new();
    let mut data = read_lump(2, 20)?;
    while data.remaining() > 0 {
        planes.push(AasPlane {
            normal: read_vector(&mut data)?,
            distance: data.finite_f32()?,
            plane_type: data.i32()?,
        });
    }
    let mut edges = Vec::new();
    let mut data = read_lump(3, 8)?;
    while data.remaining() > 0 {
        edges.push(AasEdge {
            vertices: read_pair(&mut data)?,
        });
    }
    let mut edge_indexes = Vec::new();
    let mut data = read_lump(4, 4)?;
    while data.remaining() > 0 {
        edge_indexes.push(data.i32()?);
    }
    let mut faces = Vec::new();
    let mut data = read_lump(5, 24)?;
    while data.remaining() > 0 {
        faces.push(AasFace {
            plane: data.i32()?,
            flags: data.i32()?,
            edge_count: data.i32()?,
            first_edge: data.i32()?,
            front_area: data.i32()?,
            back_area: data.i32()?,
        });
    }
    let mut face_indexes = Vec::new();
    let mut data = read_lump(6, 4)?;
    while data.remaining() > 0 {
        face_indexes.push(data.i32()?);
    }
    let mut areas = Vec::new();
    let mut data = read_lump(7, 48)?;
    while data.remaining() > 0 {
        areas.push(AasArea {
            number: data.i32()?,
            face_count: data.i32()?,
            first_face: data.i32()?,
            bounds: read_bounds(&mut data)?,
            center: read_vector(&mut data)?,
        });
    }
    let mut settings = Vec::new();
    let mut data = read_lump(8, 28)?;
    while data.remaining() > 0 {
        settings.push(AasAreaSettings {
            contents: data.i32()?,
            flags: data.i32()?,
            presence: data.i32()?,
            cluster: data.i32()?,
            cluster_area: data.i32()?,
            reach_count: data.i32()?,
            first_reach: data.i32()?,
        });
    }
    let mut reachability = Vec::new();
    let mut data = read_lump(9, 44)?;
    while data.remaining() > 0 {
        reachability.push(AasReachability {
            area: data.i32()?,
            face: data.i32()?,
            edge: data.i32()?,
            start: read_vector(&mut data)?,
            end: read_vector(&mut data)?,
            travel_type: data.i32()?,
            travel_time: data.u16()?,
            padding: data.u16()?,
        });
    }
    let mut nodes = Vec::new();
    let mut data = read_lump(10, 12)?;
    while data.remaining() > 0 {
        nodes.push(AasNode {
            plane: data.i32()?,
            children: read_pair(&mut data)?,
        });
    }
    let mut portals = Vec::new();
    let mut data = read_lump(11, 20)?;
    while data.remaining() > 0 {
        portals.push(AasPortal {
            area: data.i32()?,
            front_cluster: data.i32()?,
            back_cluster: data.i32()?,
            cluster_areas: read_pair(&mut data)?,
        });
    }
    let mut portal_indexes = Vec::new();
    let mut data = read_lump(12, 4)?;
    while data.remaining() > 0 {
        portal_indexes.push(data.i32()?);
    }
    let mut clusters = Vec::new();
    let mut data = read_lump(13, 16)?;
    while data.remaining() > 0 {
        clusters.push(AasCluster {
            area_count: data.i32()?,
            reachability_area_count: data.i32()?,
            portal_count: data.i32()?,
            first_portal: data.i32()?,
        });
    }
    if areas.len() != settings.len() {
        return Err(BinaryError::custom(source, 0, "AAS area/settings counts differ").into());
    }
    // AAS_OptimizeAlloc reserves a cleared edge zero even when all
    // vertices are pruned.
    for (number, edge) in edges.iter().enumerate() {
        if number == 0 && edge.vertices == [0, 0] {
            continue;
        }
        for vertex in edge.vertices {
            check_index(i64::from(vertex), vertices.len(), "vertex")?;
        }
    }
    for edge in &edge_indexes {
        check_index(edge.unsigned_abs() as i64, edges.len(), "edge")?;
    }
    for face in &faces {
        check_index(i64::from(face.plane), planes.len(), "face plane")?;
        check_range(
            face.first_edge,
            face.edge_count,
            edge_indexes.len(),
            "AAS face edges range",
        )?;
        check_index(i64::from(face.front_area), areas.len(), "front area")?;
        check_index(i64::from(face.back_area), areas.len(), "back area")?;
    }
    for face in &face_indexes {
        check_index(face.unsigned_abs() as i64, faces.len(), "face")?;
    }
    for (number, area) in areas.iter().enumerate() {
        if number as i32 != area.number {
            return Err(BinaryError::custom(source, 0, "AAS area ordinal differs from stored number").into());
        }
        check_range(
            area.first_face,
            area.face_count,
            face_indexes.len(),
            "AAS area faces range",
        )?;
    }
    for setting in &settings {
        check_range(
            setting.first_reach,
            setting.reach_count,
            reachability.len(),
            "AAS reachability range",
        )?;
    }
    for reach in &reachability {
        check_index(i64::from(reach.area), areas.len(), "reachable area")?;
    }
    for (number, node) in nodes.iter().enumerate() {
        if number == 0 {
            continue;
        }
        check_index(i64::from(node.plane), planes.len(), "node plane")?;
        for child in node.children {
            let count = if child > 0 { nodes.len() } else { areas.len() };
            check_index(child.unsigned_abs() as i64, count, "node child")?;
        }
    }
    for portal in &portals {
        check_index(i64::from(portal.area), areas.len(), "portal area")?;
        check_index(i64::from(portal.front_cluster), clusters.len(), "front cluster")?;
        check_index(i64::from(portal.back_cluster), clusters.len(), "back cluster")?;
    }
    for portal in &portal_indexes {
        check_index(i64::from(*portal), portals.len(), "portal")?;
    }
    for cluster in &clusters {
        check_range(
            cluster.first_portal,
            cluster.portal_count,
            portal_indexes.len(),
            "AAS cluster portals range",
        )?;
    }
    Ok(AasAsset {
        source: source.to_string(),
        version,
        bsp_checksum,
        lumps,
        bboxes,
        vertices,
        planes,
        edges,
        edge_indexes,
        faces,
        face_indexes,
        areas,
        settings,
        reachability,
        nodes,
        portals,
        portal_indexes,
        clusters,
    })
}

fn plane_distance(plane: &AasPlane, point: Vec3) -> f32 {
    (point.x * plane.normal.x + point.y * plane.normal.y) + point.z * plane.normal.z - plane.distance
}

fn bsp_node(asset: &AasAsset, number: i32) -> Result<(&AasNode, &AasPlane), BotsError> {
    let node = asset.nodes.get(number as usize).ok_or(BotsError::MissingBspNode)?;
    let plane = asset
        .planes
        .get(node.plane as usize)
        .filter(|_| node.plane >= 0)
        .ok_or(BotsError::MissingBspNode)?;
    Ok((node, plane))
}

/// Area containing a point, by descending the AAS BSP tree.
pub fn aas_point_area(asset: &AasAsset, point: Vec3) -> Result<i32, BotsError> {
    let mut number = 1i32;
    let mut visits = 0usize;
    while number > 0 {
        if visits >= asset.nodes.len() {
            return Err(BotsError::CyclicTree);
        }
        visits += 1;
        let (node, plane) = bsp_node(asset, number)?;
        number = if plane_distance(plane, point) > 0.0 {
            node.children[0]
        } else {
            node.children[1]
        };
    }
    Ok(-number)
}

/// Area crossing along a sweep.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AasAreaCrossing {
    /// Entered area.
    pub area: i32,
    /// Entry point.
    pub point: Vec3,
}

/// AAS_TraceAreas: split near side first; solid leaves do not stop area
/// enumeration.
pub fn aas_trace_areas(
    asset: &AasAsset,
    start: Vec3,
    end: Vec3,
    maximum: usize,
) -> Result<Vec<AasAreaCrossing>, BotsError> {
    let mut output = Vec::new();
    let mut stack = vec![(1i32, start, end, 0usize)];
    while let Some((node_number, start, end, depth)) = stack.pop() {
        if output.len() >= maximum {
            break;
        }
        if node_number <= 0 {
            if node_number < 0 {
                output.push(AasAreaCrossing {
                    area: -node_number,
                    point: start,
                });
            }
            continue;
        }
        if depth >= asset.nodes.len() {
            return Err(BotsError::CyclicTree);
        }
        let (node, plane) = bsp_node(asset, node_number)?;
        let front = plane_distance(plane, start);
        let back = plane_distance(plane, end);
        let depth = depth + 1;
        if front > 0.0 && back > 0.0 {
            stack.push((node.children[0], start, end, depth));
            continue;
        }
        if front <= 0.0 && back <= 0.0 {
            stack.push((node.children[1], start, end, depth));
            continue;
        }
        let fraction = (f64::from(front) / f64::from((f64::from(front) - f64::from(back)) as f32)) as f32;
        // Donor Math.max/Math.min order: NaN fractions pass through,
        // matching `clamp`, which returns NaN inputs unchanged.
        let fraction = fraction.clamp(0.0, 1.0);
        let middle = Vec3 {
            x: start.x + (end.x - start.x) * fraction,
            y: start.y + (end.y - start.y) * fraction,
            z: start.z + (end.z - start.z) * fraction,
        };
        let (near, far) = if front < 0.0 {
            (node.children[1], node.children[0])
        } else {
            (node.children[0], node.children[1])
        };
        stack.push((far, middle, end, depth));
        stack.push((near, start, middle, depth));
    }
    Ok(output)
}

/// AAS_BBoxAreas uses BSP planes, including oblique area boundaries; each
/// area appears once.
pub fn aas_bbox_areas(asset: &AasAsset, bounds: Bounds, maximum: usize) -> Result<Vec<i32>, BotsError> {
    let mut found: Vec<i32> = Vec::new();
    let mut stack = vec![(1i32, 0usize)];
    while let Some((node_number, depth)) = stack.pop() {
        if node_number <= 0 {
            if node_number < 0 && !found.contains(&-node_number) {
                found.push(-node_number);
            }
            continue;
        }
        if depth >= asset.nodes.len() {
            return Err(BotsError::CyclicTree);
        }
        let (node, plane) = bsp_node(asset, node_number)?;
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
        if plane_distance(plane, front) >= 0.0 {
            stack.push((node.children[0], depth + 1));
        }
        if plane_distance(plane, back) < 0.0 {
            stack.push((node.children[1], depth + 1));
        }
    }
    found.reverse();
    found.truncate(maximum);
    Ok(found)
}
