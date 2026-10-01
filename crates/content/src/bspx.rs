//! Quake BSPX extensions and map queries.
//!
//! Donor provenance: `src/formats/q1-map/extensions.ts` (BSPX directory,
//! decoupled lightmaps, brush lists, per-face styles, vertex/face normals)
//! and `src/formats/q1-map/queries.ts` (PVS decompression, leaf lookup,
//! face traversal). Layouts adapted from FTE specs/bspx.txt and
//! q2repro/common/bsp.c. Model traversal adapted from id Software
//! model.c.
//!
//! The auditor's `src/content/bsp/{q12,q1x,q2x}.ts` paths do not exist in
//! the donor; Q1 BSPX geometry plus the map queries are the actual
//! content. Q2 BSPX already lives in [`crate::bsp2`]; q12-model formats
//! (md2/mdl/sprite) already live in their own modules.

use qa_core::binary::{BinaryError, BinaryReader};

use crate::bsp::{Lump, Plane, Q1Map};
use crate::bsp2::DecoupledLightmap;
use crate::common::Bounds;

fn optional_offset_u32(value: u32) -> Option<u32> {
    if value == u32::MAX {
        None
    } else {
        Some(value)
    }
}

/// Q1 BSPX lump: name, file offset, and copied bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Q1BspxLump {
    /// Lump name.
    pub name: String,
    /// File offset.
    pub offset: u32,
    /// Length.
    pub length: u32,
    /// Lump bytes.
    pub data: Vec<u8>,
}

/// Face normals: shared vectors plus per-corner indices.
#[derive(Debug, Clone, PartialEq)]
pub struct Q1FaceNormals {
    /// Shared normal/tangent/bitangent vectors.
    pub vectors: Vec<[f32; 3]>,
    /// Per-corner vector indices.
    pub indices: Vec<[u32; 3]>,
}

/// Parsed BSPX metadata.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Q1BspxMetadata {
    /// Per-face lightmap shifts.
    pub lightmap_shifts: Option<Vec<u8>>,
    /// Per-face lightmap offsets.
    pub lightmap_offsets: Option<Vec<Option<u32>>>,
    /// Per-face light styles.
    pub lightmap_styles: Option<Vec<Vec<u16>>>,
    /// Per-face wide light styles.
    pub lightmap_styles16: Option<Vec<Vec<u16>>>,
    /// RGB lighting samples.
    pub rgb_lighting: Option<Vec<u8>>,
    /// HDR lighting samples.
    pub hdr_lighting: Option<Vec<u32>>,
    /// Lighting directions.
    pub lighting_directions: Option<Vec<u8>>,
    /// Vertex normals.
    pub vertex_normals: Option<Vec<[f32; 3]>>,
    /// Face normals.
    pub face_normals: Option<Q1FaceNormals>,
}

/// One brush of a brush-list model.
#[derive(Debug, Clone, PartialEq)]
pub struct Q1Brush {
    /// Brush bounds.
    pub bounds: Bounds,
    /// Contents.
    pub contents: i16,
    /// Authored non-axial planes; collision adds bounds planes later.
    pub planes: Vec<Plane>,
}

/// Brush list of one model.
#[derive(Debug, Clone, PartialEq)]
pub struct Q1BrushListModel {
    /// Model index.
    pub model: u32,
    /// Brushes.
    pub brushes: Vec<Q1Brush>,
}

/// Read the BSPX directory after the lump data (`readBspx`).
pub fn read_bspx(data: &[u8], source: &str, lumps: &[Lump]) -> Result<Vec<Q1BspxLump>, BinaryError> {
    let reader = BinaryReader::new(data, source);
    let mut end: i64 = 124;
    for lump in lumps {
        end = end.max(i64::from(lump.offset) + i64::from(lump.length));
    }
    let mut candidates = vec![(end + 3) / 4 * 4];
    if !lumps
        .iter()
        .any(|lump| lump.length > 0 && lump.offset < 132 && lump.offset + lump.length > 124)
    {
        candidates.push(124);
    }
    for offset in candidates {
        if offset < 0 || offset as usize > data.len().saturating_sub(8) {
            continue;
        }
        let mut head = reader.section(offset as usize, 4)?;
        if head.fixed_byte_string(4)? != "BSPX" {
            continue;
        }
        let mut directory = reader.section(offset as usize + 4, data.len() - offset as usize - 4)?;
        let count = directory.u32()?;
        if count as usize > directory.remaining() / 32 {
            return Err(BinaryError::custom(
                source,
                offset as usize,
                format!("invalid BSPX count {count}"),
            ));
        }
        let mut result = Vec::with_capacity(count as usize);
        for _ in 0..count {
            let name = directory.fixed_byte_string(24)?;
            let lump_offset = directory.u32()?;
            let length = directory.u32()?;
            let mut section = reader.section(lump_offset as usize, length as usize)?;
            let bytes = section.bytes(length as usize)?;
            result.push(Q1BspxLump {
                name,
                offset: lump_offset,
                length,
                data: bytes,
            });
        }
        return Ok(result);
    }
    Ok(Vec::new())
}

/// Find a BSPX lump payload by name (`bspxData`).
#[must_use]
pub fn bspx_data<'a>(lumps: &'a [Q1BspxLump], name: &str) -> Option<&'a [u8]> {
    lumps
        .iter()
        .find(|lump| lump.name == name)
        .map(|lump| lump.data.as_slice())
}

/// Read decoupled lightmaps (`readDecoupledLightmaps`).
pub fn read_decoupled_lightmaps(
    data: Option<&[u8]>,
    face_count: usize,
) -> Result<Option<Vec<DecoupledLightmap>>, BinaryError> {
    let Some(data) = data else {
        return Ok(None);
    };
    let mut reader = BinaryReader::new(data, "BSPX DECOUPLED_LM");
    if data.len() != face_count * 40 {
        return Err(BinaryError::custom(
            "BSPX DECOUPLED_LM",
            0,
            "record count does not match faces",
        ));
    }
    let mut faces = Vec::with_capacity(face_count);
    for _ in 0..face_count {
        let width = u32::from(reader.u16()?);
        let height = u32::from(reader.u16()?);
        let lighting_offset = optional_offset_u32(reader.u32()?);
        let s = [
            reader.finite_f32()?,
            reader.finite_f32()?,
            reader.finite_f32()?,
            reader.finite_f32()?,
        ];
        let t = [
            reader.finite_f32()?,
            reader.finite_f32()?,
            reader.finite_f32()?,
            reader.finite_f32()?,
        ];
        faces.push(DecoupledLightmap {
            width,
            height,
            lighting_offset,
            axes: [[s[0], s[1], s[2]], [t[0], t[1], t[2]]],
            offset: [s[3], t[3]],
        });
    }
    Ok(Some(faces))
}

/// Read brush lists (`readBrushList`). Unknown versions yield `None`.
pub fn read_brush_list(data: Option<&[u8]>, model_count: usize) -> Result<Option<Vec<Q1BrushListModel>>, BinaryError> {
    let Some(data) = data else {
        return Ok(None);
    };
    let mut reader = BinaryReader::new(data, "BSPX BRUSHLIST");
    let mut models = Vec::new();
    while reader.remaining() > 0 {
        let version = reader.u32()?;
        if version != 1 {
            return Ok(None);
        }
        let model = reader.u32()?;
        check_index(model, model_count, "BSPX BRUSHLIST")?;
        let count = reader.u32()?;
        let expected_planes = reader.u32()?;
        if count as usize > reader.remaining() / 28
            || expected_planes as usize > reader.remaining().saturating_sub(count as usize * 28) / 16
        {
            return Err(BinaryError::custom(
                "BSPX BRUSHLIST",
                reader.offset(),
                "brushes or planes exceed lump",
            ));
        }
        let mut brushes = Vec::with_capacity(count as usize);
        let mut plane_total = 0u32;
        for _ in 0..count {
            let bounds = Bounds {
                min: [reader.finite_f32()?, reader.finite_f32()?, reader.finite_f32()?],
                max: [reader.finite_f32()?, reader.finite_f32()?, reader.finite_f32()?],
            };
            let contents = reader.i16()?;
            let plane_count = u32::from(reader.u16()?);
            plane_total += plane_count;
            if plane_total > expected_planes {
                return Err(BinaryError::custom(
                    "BSPX BRUSHLIST",
                    reader.offset(),
                    "brush plane count exceeds model total",
                ));
            }
            let mut planes = Vec::with_capacity(plane_count as usize);
            for _ in 0..plane_count {
                planes.push(Plane {
                    normal: [reader.finite_f32()?, reader.finite_f32()?, reader.finite_f32()?],
                    distance: reader.finite_f32()?,
                    plane_type: 0,
                    signbits: 0,
                });
            }
            brushes.push(Q1Brush {
                bounds,
                contents,
                planes,
            });
        }
        if plane_total != expected_planes {
            return Err(BinaryError::custom(
                "BSPX BRUSHLIST",
                reader.offset(),
                "brush plane count differs from model total",
            ));
        }
        models.push(Q1BrushListModel { model, brushes });
    }
    Ok(Some(models))
}

fn check_index(value: u32, count: usize, source: &str) -> Result<(), BinaryError> {
    if value as usize >= count {
        return Err(BinaryError::custom(source, 0, format!("index {value} exceeds {count}")));
    }
    Ok(())
}

fn per_face_styles(data: Option<&[u8]>, face_count: usize, wide: bool) -> Result<Option<Vec<Vec<u16>>>, BinaryError> {
    let Some(data) = data else {
        return Ok(None);
    };
    let source = if wide { "BSPX LMSTYLE16" } else { "BSPX LMSTYLE" };
    let mut reader = BinaryReader::new(data, source);
    let width = if wide { 2 } else { 1 };
    if face_count == 0 {
        if !data.is_empty() {
            return Err(BinaryError::custom(source, 0, "styles with no faces"));
        }
        return Ok(Some(Vec::new()));
    }
    if data.len() % (width * face_count) != 0 {
        return Err(BinaryError::custom(source, 0, "style count does not match faces"));
    }
    let count = data.len() / (width * face_count);
    let mut result = Vec::with_capacity(face_count);
    for _ in 0..face_count {
        let mut styles = Vec::with_capacity(count);
        for _ in 0..count {
            styles.push(if wide { reader.u16()? } else { u16::from(reader.u8()?) });
        }
        result.push(styles);
    }
    Ok(Some(result))
}

fn read_face_normals(data: Option<&[u8]>, corner_count: usize) -> Result<Option<Q1FaceNormals>, BinaryError> {
    let Some(data) = data else {
        return Ok(None);
    };
    let mut reader = BinaryReader::new(data, "BSPX FACENORMALS");
    let count = reader.u32()?;
    let mut vectors = Vec::with_capacity(count as usize);
    for _ in 0..count {
        vectors.push([reader.finite_f32()?, reader.finite_f32()?, reader.finite_f32()?]);
    }
    let mut indices = Vec::with_capacity(corner_count);
    for _ in 0..corner_count {
        let normal = reader.u32()?;
        let tangent = reader.u32()?;
        let bitangent = reader.u32()?;
        check_index(normal, count as usize, "BSPX FACENORMALS")?;
        check_index(tangent, count as usize, "BSPX FACENORMALS")?;
        check_index(bitangent, count as usize, "BSPX FACENORMALS")?;
        indices.push([normal, tangent, bitangent]);
    }
    Ok(Some(Q1FaceNormals { vectors, indices }))
}

/// Read parsed BSPX metadata (`readBspxMetadata`).
pub fn read_bspx_metadata(
    lumps: &[Q1BspxLump],
    face_count: usize,
    vertex_count: usize,
    corner_count: usize,
) -> Result<Q1BspxMetadata, BinaryError> {
    let exact = |name: &str, length: usize| -> Result<Option<&[u8]>, BinaryError> {
        let data = bspx_data(lumps, name);
        if let Some(data) = data {
            if data.len() != length {
                let label = format!("BSPX {name}");
                return Err(BinaryError::custom(
                    &label,
                    0,
                    format!("expected {length} bytes, got {}", data.len()),
                ));
            }
        }
        Ok(data)
    };
    let offsets = exact("LMOFFSET", face_count * 4)?;
    let hdr = bspx_data(lumps, "LIGHTING_E5BGR9");
    let normals = exact("VERTEXNORMALS", vertex_count * 12)?;
    let mut lightmap_offsets = None;
    if let Some(offsets) = offsets {
        let mut reader = BinaryReader::new(offsets, "BSPX LMOFFSET");
        let mut values = Vec::with_capacity(face_count);
        for _ in 0..face_count {
            let raw = reader.u32()?;
            values.push(if raw == u32::MAX { None } else { Some(raw) });
        }
        lightmap_offsets = Some(values);
    }
    let mut hdr_lighting = None;
    if let Some(hdr) = hdr {
        let mut reader = BinaryReader::new(hdr, "BSPX LIGHTING_E5BGR9");
        let mut values = Vec::with_capacity(hdr.len() / 4);
        while reader.remaining() > 0 {
            values.push(reader.u32()?);
        }
        hdr_lighting = Some(values);
    }
    let mut vertex_normals = None;
    if let Some(normals) = normals {
        let mut reader = BinaryReader::new(normals, "BSPX VERTEXNORMALS");
        let mut values = Vec::with_capacity(vertex_count);
        for _ in 0..vertex_count {
            values.push([reader.finite_f32()?, reader.finite_f32()?, reader.finite_f32()?]);
        }
        vertex_normals = Some(values);
    }
    Ok(Q1BspxMetadata {
        lightmap_shifts: exact("LMSHIFT", face_count)?.map(<[u8]>::to_vec),
        lightmap_offsets,
        lightmap_styles: per_face_styles(bspx_data(lumps, "LMSTYLE"), face_count, false)?,
        lightmap_styles16: per_face_styles(bspx_data(lumps, "LMSTYLE16"), face_count, true)?,
        rgb_lighting: bspx_data(lumps, "RGBLIGHTING").map(<[u8]>::to_vec),
        hdr_lighting,
        lighting_directions: bspx_data(lumps, "LIGHTINGDIR").map(<[u8]>::to_vec),
        vertex_normals,
        face_normals: read_face_normals(bspx_data(lumps, "FACENORMALS"), corner_count)?,
    })
}

/// Decompress Quake PVS RLE (`decompressQ1Pvs`).
pub fn decompress_q1_pvs(data: &[u8], offset: Option<u32>, visible_leaves: usize) -> Result<Vec<u8>, BinaryError> {
    let mut output = vec![0u8; visible_leaves.div_ceil(8)];
    if offset.is_none() || data.is_empty() {
        output.fill(255);
        return Ok(output);
    }
    let mut reader = BinaryReader::new(data, "Quake PVS");
    reader.seek(offset.unwrap_or(0) as usize)?;
    let mut position = 0;
    while position < output.len() {
        let value = reader.u8()?;
        if value != 0 {
            output[position] = value;
            position += 1;
            continue;
        }
        let count = reader.u8()?;
        if count == 0 || count as usize > output.len() - position {
            return Err(BinaryError::custom(
                "Quake PVS",
                reader.offset() - 1,
                format!("invalid zero run {count}"),
            ));
        }
        position += count as usize;
    }
    Ok(output)
}

/// PVS bits for a leaf. PVS bit zero denotes leaf one; the shared solid
/// leaf zero sees every leaf (`q1LeafPvs`).
pub fn q1_leaf_pvs(map: &Q1Map, leaf_index: usize) -> Result<Vec<u8>, BinaryError> {
    if leaf_index >= map.leaves.len() {
        return Err(BinaryError::custom("Quake PVS leaf", 0, "Missing Quake leaf"));
    }
    let leaf = &map.leaves[leaf_index];
    let count = map.models.first().map_or(map.leaves.len().saturating_sub(1), |world| {
        world.visible_leaves.max(0) as usize
    });
    let offset = if leaf_index == 0 { None } else { leaf.visibility_offset };
    decompress_q1_pvs(map.visibility, offset, count)
}

/// Find the leaf containing a point (`findQ1Leaf`).
pub fn find_q1_leaf(map: &Q1Map, point: [f32; 3], model_index: usize) -> Result<usize, BinaryError> {
    if model_index >= map.models.len() {
        return Err(BinaryError::custom("Quake model", 0, "index exceeds models"));
    }
    let mut next = map.models[model_index].headnodes[0];
    for _ in 0..=map.nodes.len() {
        if next < 0 {
            let leaf = (-1 - next) as usize;
            if leaf >= map.leaves.len() {
                return Err(BinaryError::custom("Quake leaf", 0, "Missing Quake leaf"));
            }
            return Ok(leaf);
        }
        let node = map
            .nodes
            .get(next as usize)
            .ok_or_else(|| BinaryError::custom("Quake BSP nodes", 0, format!("Missing Quake node {next}")))?;
        let plane = map
            .planes
            .get(node.plane as usize)
            .ok_or_else(|| BinaryError::custom("Quake BSP nodes", 0, format!("Missing Quake plane {}", node.plane)))?;
        let distance =
            point[0] * plane.normal[0] + point[1] * plane.normal[1] + point[2] * plane.normal[2] - plane.distance;
        let child = node.children[usize::from(distance <= 0.0)];
        match child {
            crate::bsp::NodeChild::Leaf(index) => return Ok(index as usize),
            crate::bsp::NodeChild::Node(index) => next = index as i32,
        }
    }
    Err(BinaryError::custom("Quake BSP nodes", 0, "Cycle in Quake BSP nodes"))
}

/// Vertices of a face (`q1FaceVertices`).
pub fn q1_face_vertices(map: &Q1Map, face_index: usize) -> Result<Vec<[f32; 3]>, BinaryError> {
    let face = map
        .faces
        .get(face_index)
        .ok_or_else(|| BinaryError::custom("Quake face", 0, "Missing Quake face"))?;
    let mut vertices = Vec::with_capacity(face.edge_count as usize);
    for i in 0..face.edge_count {
        let edge_index = map
            .surface_edges
            .get((face.edge_first + i as i32) as usize)
            .copied()
            .ok_or_else(|| BinaryError::custom("Quake face", 0, "Missing Quake surface edge"))?;
        let edge = map
            .edges
            .get(edge_index.unsigned_abs() as usize)
            .ok_or_else(|| BinaryError::custom("Quake face", 0, "Missing Quake edge"))?;
        let vertex = map
            .vertices
            .get(edge.vertices[usize::from(edge_index < 0)] as usize)
            .copied()
            .ok_or_else(|| BinaryError::custom("Quake face", 0, "Missing Quake vertex"))?;
        vertices.push(vertex);
    }
    Ok(vertices)
}

/// Identify an authored face at an existing hull-zero contact, without
/// retracing (`q1FaceAtContact`).
pub fn q1_face_at_contact(
    map: &Q1Map,
    candidates: &[usize],
    point: [f32; 3],
    plane: &Plane,
) -> Result<Option<usize>, BinaryError> {
    let normal = plane.normal;
    let norm_squared = normal[0] * normal[0] + normal[1] * normal[1] + normal[2] * normal[2];
    let distance = (point[0] * normal[0] + point[1] * normal[1] + point[2] * normal[2] - plane.distance) / norm_squared;
    let projected = [
        point[0] - normal[0] * distance,
        point[1] - normal[1] * distance,
        point[2] - normal[2] * distance,
    ];
    for face_index in candidates {
        let vertices = q1_face_vertices(map, *face_index)?;
        let mut positive = false;
        let mut negative = false;
        for (i, a) in vertices.iter().enumerate() {
            let b = vertices[(i + 1) % vertices.len()];
            let edge = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
            let offset = [projected[0] - a[0], projected[1] - a[1], projected[2] - a[2]];
            let side = (edge[1] * offset[2] - edge[2] * offset[1]) * normal[0]
                + (edge[2] * offset[0] - edge[0] * offset[2]) * normal[1]
                + (edge[0] * offset[1] - edge[1] * offset[0]) * normal[2];
            positive |= side > 0.0;
            negative |= side < 0.0;
            if positive && negative {
                break;
            }
        }
        if vertices.len() >= 3 && !(positive && negative) {
            return Ok(Some(*face_index));
        }
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bsp::{BspLighting, Edge, Face, IndexRange, Leaf, Node, NodeChild, WorldModel};

    fn empty_map() -> Q1Map<'static> {
        Q1Map {
            format: crate::bsp::BspFormat::Bsp29,
            source: String::new(),
            version: 29,
            data: &[],
            lumps: Vec::new(),
            entities: String::new(),
            entity_list: Vec::new(),
            planes: Vec::new(),
            vertices: Vec::new(),
            textures: Vec::new(),
            texture_offsets: Vec::new(),
            mip_offsets: Vec::new(),
            texture_info: Vec::new(),
            faces: Vec::new(),
            models: Vec::new(),
            nodes: Vec::new(),
            leaves: Vec::new(),
            edges: Vec::new(),
            clipnodes: Vec::new(),
            surface_edges: Vec::new(),
            leaf_faces: Vec::new(),
            visibility: &[],
            monochrome_lighting: &[],
            lighting: BspLighting::Luminance8 { samples: &[] },
        }
    }

    fn u32_bytes(value: u32) -> [u8; 4] {
        value.to_le_bytes()
    }

    #[test]
    fn bspx_directory_parses_and_queries() {
        // Minimal file: 124-byte header, BSPX magic, one lump entry.
        let mut file = vec![0u8; 124];
        file.extend_from_slice(b"BSPX");
        file.extend_from_slice(&u32_bytes(1));
        let mut name = [0u8; 24];
        name[..8].copy_from_slice(b"RGBTEST\x00");
        file.extend_from_slice(&name);
        file.extend_from_slice(&u32_bytes(200));
        file.extend_from_slice(&u32_bytes(4));
        while file.len() < 200 {
            file.push(0);
        }
        file.extend_from_slice(&[9, 8, 7, 6]);
        let lumps = vec![Lump {
            name: "entities",
            offset: 0,
            length: 0,
        }];
        let found = read_bspx(&file, "test", &lumps).unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(bspx_data(&found, "RGBTEST"), Some([9, 8, 7, 6].as_slice()));
        assert_eq!(bspx_data(&found, "MISSING"), None);
        assert!(read_bspx(&[0u8; 64], "short", &[]).unwrap().is_empty());
    }

    #[test]
    fn metadata_reads_styles_and_normals() {
        let lumps = vec![
            Q1BspxLump {
                name: "LMSTYLE".to_string(),
                offset: 0,
                length: 4,
                data: vec![0, 1, 2, 3],
            },
            Q1BspxLump {
                name: "FACENORMALS".to_string(),
                offset: 0,
                length: 0,
                data: [
                    u32_bytes(2).as_slice(),
                    &[0u8; 24],
                    &u32_bytes(0),
                    &u32_bytes(1),
                    &u32_bytes(0),
                ]
                .concat(),
            },
        ];
        let metadata = read_bspx_metadata(&lumps, 2, 0, 1).unwrap();
        assert_eq!(metadata.lightmap_styles, Some(vec![vec![0, 1], vec![2, 3]]));
        let normals = metadata.face_normals.expect("normals");
        assert_eq!(normals.vectors.len(), 2);
        assert_eq!(normals.indices, vec![[0, 1, 0]]);
        assert!(metadata.rgb_lighting.is_none());
    }

    #[test]
    fn brush_list_round_trips_one_model() {
        let mut data = Vec::new();
        data.extend_from_slice(&u32_bytes(1));
        data.extend_from_slice(&u32_bytes(0));
        data.extend_from_slice(&u32_bytes(1));
        data.extend_from_slice(&u32_bytes(6));
        data.extend_from_slice(&[0u8; 24]);
        data.extend_from_slice(&(-2i16).to_le_bytes());
        data.extend_from_slice(&6u16.to_le_bytes());
        for _ in 0..6 {
            data.extend_from_slice(&1f32.to_le_bytes());
            data.extend_from_slice(&[0u8; 12]);
        }
        let models = read_brush_list(Some(&data), 1).unwrap().expect("models");
        assert_eq!(models.len(), 1);
        assert_eq!(models[0].brushes.len(), 1);
        assert_eq!(models[0].brushes[0].contents, -2);
        assert_eq!(models[0].brushes[0].planes.len(), 6);
        assert!(read_brush_list(None, 1).unwrap().is_none());
        let mut bad_version = data.clone();
        bad_version[0] = 2;
        assert!(read_brush_list(Some(&bad_version), 1).unwrap().is_none());
    }

    #[test]
    fn decoupled_lightmaps_need_exact_faces() {
        assert!(read_decoupled_lightmaps(None, 3).unwrap().is_none());
        assert!(read_decoupled_lightmaps(Some(&[0u8; 10]), 1).is_err());
        let faces = read_decoupled_lightmaps(Some(&[0u8; 40]), 1).unwrap().expect("faces");
        assert_eq!(faces.len(), 1);
        assert_eq!(faces[0].width, 0);
    }

    #[test]
    fn pvs_decompresses_runs_and_defaults() {
        assert_eq!(decompress_q1_pvs(&[], None, 10).unwrap(), vec![255, 255]);
        assert_eq!(decompress_q1_pvs(&[0x03], Some(0), 8).unwrap(), vec![0x03]);
        assert_eq!(decompress_q1_pvs(&[0x00, 0x02], Some(0), 16).unwrap(), vec![0, 0]);
        assert!(decompress_q1_pvs(&[0x00, 0x00], Some(0), 8).is_err());
    }

    #[test]
    fn leaf_queries_traverse_nodes_and_faces() {
        let mut map = empty_map();
        map.planes.push(Plane {
            normal: [1.0, 0.0, 0.0],
            distance: 0.0,
            plane_type: 0,
            signbits: 0,
        });
        map.nodes.push(Node {
            plane: 0,
            children: [NodeChild::Leaf(1), NodeChild::Leaf(2)],
            bounds: Bounds {
                min: [0.0; 3],
                max: [0.0; 3],
            },
            faces: IndexRange { first: 0, count: 0 },
        });
        for _ in 0..3 {
            map.leaves.push(Leaf {
                contents: -1,
                visibility_offset: None,
                bounds: Bounds {
                    min: [0.0; 3],
                    max: [0.0; 3],
                },
                faces: IndexRange { first: 0, count: 0 },
                ambient_sound: [0; 4],
            });
        }
        map.models.push(WorldModel {
            bounds: Bounds {
                min: [0.0; 3],
                max: [0.0; 3],
            },
            origin: [0.0; 3],
            headnodes: [0, -1, -1, -1],
            visible_leaves: 2,
            face_first: 0,
            face_count: 0,
        });
        assert_eq!(find_q1_leaf(&map, [4.0, 0.0, 0.0], 0).unwrap(), 1);
        assert_eq!(find_q1_leaf(&map, [-4.0, 0.0, 0.0], 0).unwrap(), 2);
        assert_eq!(q1_leaf_pvs(&map, 0).unwrap(), vec![255]);
        assert_eq!(q1_leaf_pvs(&map, 1).unwrap(), vec![255]);

        map.vertices.push([0.0, 0.0, 0.0]);
        map.vertices.push([8.0, 0.0, 0.0]);
        map.vertices.push([8.0, 8.0, 0.0]);
        map.edges.push(Edge { vertices: [0, 1] });
        map.edges.push(Edge { vertices: [1, 2] });
        map.edges.push(Edge { vertices: [2, 0] });
        map.surface_edges.extend([0, 1, 2]);
        map.faces.push(Face {
            plane: 0,
            back: false,
            edge_first: 0,
            edge_count: 3,
            texture_info: 0,
            styles: [0; 4],
            lighting_offset: None,
        });
        let vertices = q1_face_vertices(&map, 0).unwrap();
        assert_eq!(vertices.len(), 3);
        let plane = Plane {
            normal: [0.0, 0.0, 1.0],
            distance: 0.0,
            plane_type: 0,
            signbits: 0,
        };
        assert_eq!(
            q1_face_at_contact(&map, &[0], [4.0, 1.0, 2.0], &plane).unwrap(),
            Some(0)
        );
        assert_eq!(q1_face_at_contact(&map, &[0], [40.0, 40.0, 2.0], &plane).unwrap(), None);
    }
}
