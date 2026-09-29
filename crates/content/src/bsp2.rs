//! Quake II BSP map (IBSP v38 / QBSP v38) parser with BSPX extensions.
//!
//! Donor provenance: `readQ2Bsp`, `decompressQ2Visibility`, and
//! `validateQ2Bsp` in `src/formats/q2-map/reader.ts`, the BSPX directory,
//! face normals, decoupled lightmaps, and lightgrid in
//! `src/formats/q2-map/bspx.ts`, and the world-geometry adapter in
//! `src/formats/q2-map/index.ts`.
//!
//! Lump payloads (entities, lighting, visibility, POP, BSPX lumps) are
//! borrowed from the input; decoded records are owned.

use qa_core::binary::{BinaryError, BinaryReader};

use crate::bsp::{Edge, IndexRange, Node, NodeChild, Plane};
use crate::common::Bounds;

/// IBSP magic (`0x50534249`, little-endian `"IBSP"`).
pub const Q2_MAGIC_IBSP: u32 = 0x5053_4249;
/// QBSP magic (`0x50534251`, little-endian `"QBSP"`, extended limits).
pub const Q2_MAGIC_QBSP: u32 = 0x5053_4251;
/// Q2 BSP version (`38`).
pub const Q2_BSP_VERSION: u32 = 38;

const LUMP_ENTITIES: usize = 0;
const LUMP_PLANES: usize = 1;
const LUMP_VERTICES: usize = 2;
const LUMP_VISIBILITY: usize = 3;
const LUMP_NODES: usize = 4;
const LUMP_TEXTURE_INFO: usize = 5;
const LUMP_FACES: usize = 6;
const LUMP_LIGHTING: usize = 7;
const LUMP_LEAVES: usize = 8;
const LUMP_LEAF_FACES: usize = 9;
const LUMP_LEAF_BRUSHES: usize = 10;
const LUMP_EDGES: usize = 11;
const LUMP_SURFACE_EDGES: usize = 12;
const LUMP_MODELS: usize = 13;
const LUMP_BRUSHES: usize = 14;
const LUMP_BRUSH_SIDES: usize = 15;
const LUMP_POP: usize = 16;
const LUMP_AREAS: usize = 17;
const LUMP_AREA_PORTALS: usize = 18;
const LUMP_COUNT: usize = 19;

/// Q2 BSP format (`"ibsp38" | "qbsp"`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Q2Format {
    /// Classic 16-bit limits.
    Ibsp38,
    /// Extended 32-bit limits.
    Qbsp,
}

/// Q2 lump directory entry (`Q2Lump`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Q2Lump {
    /// File offset.
    pub offset: u32,
    /// Length in bytes.
    pub length: u32,
}

/// Q2 raw face (`Q2RawFace`): on-disk draw flags, indices, and sentinels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Q2RawFace {
    /// Plane index.
    pub plane: u32,
    /// Draw flags (bit 0 selects the back side).
    pub draw_flags: u32,
    /// Surface-edge range.
    pub edges: IndexRange,
    /// Texture info index.
    pub texture_info: u32,
    /// Light styles.
    pub styles: [u8; 4],
    /// Lighting offset, or -1 for none.
    pub lighting_offset: i32,
}

/// Q2 raw texture info (`Q2RawTextureInfo`).
#[derive(Debug, Clone, PartialEq)]
pub struct Q2RawTextureInfo {
    /// S projection.
    pub projection_s: [f32; 4],
    /// T projection.
    pub projection_t: [f32; 4],
    /// Surface flags.
    pub flags: i32,
    /// Surface value.
    pub value: i32,
    /// Texture name.
    pub name: String,
    /// Next animation frame, or non-positive to end the chain.
    pub next: i32,
}

/// Q2 raw leaf (`Q2RawLeaf`, without merged contents).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Q2RawLeaf {
    /// Stored contents.
    pub contents: i32,
    /// Visibility cluster, or -1 for none.
    pub cluster: i32,
    /// Area index.
    pub area: u32,
    /// Bounds.
    pub bounds: Bounds,
    /// Leaf-face range.
    pub faces: IndexRange,
    /// Leaf-brush range.
    pub brushes: IndexRange,
}

/// Q2 brush (`Q2Brush`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Q2Brush {
    /// Brush-side range.
    pub sides: IndexRange,
    /// Contents.
    pub contents: i32,
}

/// Q2 brush side (`Q2BrushSide`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Q2BrushSide {
    /// Plane index.
    pub plane: u32,
    /// Texture info index, or -1 for none.
    pub texture_info: i32,
}

/// Q2 world model (`Q2WorldModel`, on-disk bounds).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Q2WorldModel {
    /// Bounds.
    pub bounds: Bounds,
    /// Origin.
    pub origin: [f32; 3],
    /// Head node (negative values address leaves).
    pub headnode: i32,
    /// Face range.
    pub faces: IndexRange,
}

/// Q2 area (`Q2Area`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Q2Area {
    /// Area-portal range.
    pub portals: IndexRange,
}

/// Q2 area portal (`Q2AreaPortal`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Q2AreaPortal {
    /// Portal index.
    pub portal: u32,
    /// Other area index.
    pub other_area: u32,
}

/// Q2 visibility cluster offsets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Q2VisCluster {
    /// Potentially-visible-set offset, or -1.
    pub pvs_offset: i32,
    /// Potentially-hearable-set offset, or -1.
    pub phs_offset: i32,
}

/// Q2 visibility (`Q2Visibility`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Q2Visibility<'a> {
    /// Per-cluster compressed-data offsets.
    pub clusters: Vec<Q2VisCluster>,
    /// Complete visibility lump, including its header.
    pub compressed: &'a [u8],
}

/// Parsed Q2 map (`Q2Bsp`): source records with on-disk bounds, indices,
/// light offsets, and animation sentinels.
#[derive(Debug, Clone, PartialEq)]
pub struct Q2Bsp<'a> {
    /// Source name.
    pub source: String,
    /// Format.
    pub format: Q2Format,
    /// Version (always 38).
    pub version: u32,
    /// Lump directory.
    pub lumps: Vec<Q2Lump>,
    /// Entity text.
    pub entities: String,
    /// Raw entity bytes.
    pub entity_bytes: &'a [u8],
    /// Planes.
    pub planes: Vec<Plane>,
    /// Vertices.
    pub vertices: Vec<[f32; 3]>,
    /// Edges.
    pub edges: Vec<Edge>,
    /// Surface edges.
    pub surface_edges: Vec<i32>,
    /// Nodes.
    pub nodes: Vec<Node>,
    /// Leaves.
    pub leaves: Vec<Q2RawLeaf>,
    /// Leaf faces.
    pub leaf_faces: Vec<u32>,
    /// Leaf brushes.
    pub leaf_brushes: Vec<u32>,
    /// Texture info.
    pub texture_info: Vec<Q2RawTextureInfo>,
    /// Faces.
    pub faces: Vec<Q2RawFace>,
    /// Brushes.
    pub brushes: Vec<Q2Brush>,
    /// Brush sides.
    pub brush_sides: Vec<Q2BrushSide>,
    /// Models.
    pub models: Vec<Q2WorldModel>,
    /// Areas.
    pub areas: Vec<Q2Area>,
    /// Area portals.
    pub area_portals: Vec<Q2AreaPortal>,
    /// Visibility, or `None` when the lump is empty.
    pub visibility: Option<Q2Visibility<'a>>,
    /// RGB lighting samples.
    pub lighting: &'a [u8],
    /// Cluster POP data.
    pub pop: &'a [u8],
    /// BSPX directory, or `None` when absent.
    pub bspx: Option<Q2BspxDirectory<'a>>,
    /// Loader diagnostics.
    pub diagnostics: Vec<String>,
}

fn vec3(reader: &mut BinaryReader<'_>) -> Result<[f32; 3], BinaryError> {
    Ok([reader.finite_f32()?, reader.finite_f32()?, reader.finite_f32()?])
}

fn short_vec3(reader: &mut BinaryReader<'_>) -> Result<[f32; 3], BinaryError> {
    Ok([
        f32::from(reader.i16()?),
        f32::from(reader.i16()?),
        f32::from(reader.i16()?),
    ])
}

fn projection(reader: &mut BinaryReader<'_>) -> Result<[f32; 4], BinaryError> {
    Ok([
        reader.finite_f32()?,
        reader.finite_f32()?,
        reader.finite_f32()?,
        reader.finite_f32()?,
    ])
}

fn bounds(reader: &mut BinaryReader<'_>, extended: bool) -> Result<Bounds, BinaryError> {
    if extended {
        Ok(Bounds {
            min: vec3(reader)?,
            max: vec3(reader)?,
        })
    } else {
        Ok(Bounds {
            min: short_vec3(reader)?,
            max: short_vec3(reader)?,
        })
    }
}

fn index(reader: &mut BinaryReader<'_>, extended: bool) -> Result<u32, BinaryError> {
    if extended {
        reader.u32()
    } else {
        Ok(u32::from(reader.u16()?))
    }
}

fn nullable_index(reader: &mut BinaryReader<'_>, extended: bool) -> Result<i32, BinaryError> {
    let value = index(reader, extended)?;
    if value == if extended { 0xffff_ffff } else { 0xffff } {
        return Ok(-1);
    }
    Ok(value as i32)
}

fn range(reader: &mut BinaryReader<'_>, extended: bool) -> Result<IndexRange, BinaryError> {
    Ok(IndexRange {
        first: index(reader, extended)?,
        count: index(reader, extended)?,
    })
}

fn child(value: i32) -> NodeChild {
    if value < 0 {
        NodeChild::Leaf((-1 - value) as u32)
    } else {
        NodeChild::Node(value as u32)
    }
}

fn read_visibility<'a>(
    reader: &BinaryReader<'a>,
    source: &str,
    lump: Q2Lump,
) -> Result<Option<Q2Visibility<'a>>, BinaryError> {
    if lump.length == 0 {
        return Ok(None);
    }
    let compressed = reader.view(lump.offset as usize, lump.length as usize)?;
    let mut rows = reader.section(lump.offset as usize, lump.length as usize)?;
    let count = rows.u32()?;
    rows.records(rows.offset(), count as usize * 8, 8)?;
    let mut clusters = Vec::with_capacity(count as usize);
    for _ in 0..count {
        let pvs_offset = rows.i32()?;
        let phs_offset = rows.i32()?;
        for offset in [pvs_offset, phs_offset] {
            let offset = i64::from(offset);
            if offset != -1 && (offset < 4 + i64::from(count) * 8 || offset >= i64::from(lump.length)) {
                return Err(BinaryError {
                    input: source.to_string(),
                    offset: rows.offset() - 8,
                    message: "visibility offset is outside compressed data".to_string(),
                });
            }
        }
        clusters.push(Q2VisCluster { pvs_offset, phs_offset });
    }
    Ok(Some(Q2Visibility { clusters, compressed }))
}

/// BSPX lump (`Q2BspxLump`): name, file offset, and borrowed bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Q2BspxLump<'a> {
    /// Lump name.
    pub name: String,
    /// File offset.
    pub offset: u32,
    /// Lump bytes.
    pub bytes: &'a [u8],
}

/// BSPX directory (`Q2BspxDirectory`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Q2BspxDirectory<'a> {
    /// Directory offset.
    pub offset: usize,
    /// Lumps in directory order.
    pub lumps: Vec<Q2BspxLump<'a>>,
    /// Loader diagnostics.
    pub diagnostics: Vec<String>,
}

impl<'a> Q2BspxDirectory<'a> {
    /// Look up a BSPX lump by name.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<&Q2BspxLump<'a>> {
        self.lumps.iter().find(|lump| lump.name == name)
    }
}

/// Read the BSPX directory after the lump data (`readQ2Bspx`).
pub fn read_q2_bspx<'a>(
    data: &'a [u8],
    source: &str,
    end_of_lumps: u32,
) -> Result<Option<Q2BspxDirectory<'a>>, BinaryError> {
    let offset = end_of_lumps.div_ceil(4) * 4;
    if data.len() < 8 || offset as usize > data.len() - 8 {
        return Ok(None);
    }
    let reader = BinaryReader::new(data, source);
    let mut directory = reader.section(offset as usize, data.len() - offset as usize)?;
    if directory.fixed_byte_string(4)? != "BSPX" {
        return Ok(None);
    }
    let count = directory.u32()?;
    let mut diagnostics = Vec::new();
    let mut lumps = Vec::new();
    if count as usize > directory.remaining() / 32 {
        return Ok(Some(Q2BspxDirectory {
            offset: offset as usize,
            lumps,
            diagnostics: vec!["Truncated BSPX directory".to_string()],
        }));
    }
    for _ in 0..count {
        let name = directory.fixed_byte_string(24)?;
        let file_offset = directory.u32()?;
        let length = directory.u32()?;
        if length == 0
            || length as usize > data.len()
            || file_offset as usize > data.len() - length as usize
            || lumps.iter().any(|lump: &Q2BspxLump<'_>| lump.name == name)
        {
            diagnostics.push(format!("Ignored empty, out-of-bounds, or duplicate BSPX lump {name}"));
            continue;
        }
        lumps.push(Q2BspxLump {
            name,
            offset: file_offset,
            bytes: reader.view(file_offset as usize, length as usize)?,
        });
    }
    Ok(Some(Q2BspxDirectory {
        offset: offset as usize,
        lumps,
        diagnostics,
    }))
}

/// BSPX face normals (`Q2FaceNormals`).
#[derive(Debug, Clone, PartialEq)]
pub struct Q2FaceNormals {
    /// Normal table.
    pub normals: Vec<[f32; 3]>,
    /// Three indices per face corner; consumers use the first.
    pub corner_indices: Vec<[u32; 3]>,
}

/// Read BSPX face normals (`readQ2FaceNormals`).
pub fn read_q2_face_normals(lump: &Q2BspxLump<'_>, corner_count: usize) -> Result<Q2FaceNormals, BinaryError> {
    let source = "BSPX FACENORMALS";
    let mut reader = BinaryReader::new(lump.bytes, source);
    let count = reader.u32()?;
    let total = count as u64 * 12 + corner_count as u64 * 12;
    reader.records(reader.offset(), total as usize, 12)?;
    let mut normals = Vec::with_capacity(count as usize);
    for _ in 0..count {
        normals.push(vec3(&mut reader)?);
    }
    let mut corner_indices = Vec::with_capacity(corner_count);
    for _ in 0..corner_count {
        let normal = reader.u32()?;
        if normal >= count {
            return Err(BinaryError {
                input: source.to_string(),
                offset: reader.offset() - 4,
                message: "invalid face normal index".to_string(),
            });
        }
        corner_indices.push([normal, reader.u32()?, reader.u32()?]);
    }
    Ok(Q2FaceNormals {
        normals,
        corner_indices,
    })
}

/// Decoupled lightmap face (`DecoupledLightmap`).
#[derive(Debug, Clone, PartialEq)]
pub struct DecoupledLightmap {
    /// Width in samples.
    pub width: u32,
    /// Height in samples.
    pub height: u32,
    /// Lighting offset, or `None` for none.
    pub lighting_offset: Option<u32>,
    /// Texture axes.
    pub axes: [[f32; 3]; 2],
    /// Texture offset.
    pub offset: [f32; 2],
}

/// BSPX decoupled lightmaps (`Q2DecoupledLightmaps`).
#[derive(Debug, Clone, PartialEq)]
pub struct Q2DecoupledLightmaps {
    /// Per-face lightmaps.
    pub faces: Vec<DecoupledLightmap>,
    /// Loader diagnostics.
    pub diagnostics: Vec<String>,
}

/// Read BSPX decoupled lightmaps (`readQ2DecoupledLightmaps`).
pub fn read_q2_decoupled_lightmaps(
    lump: &Q2BspxLump<'_>,
    face_count: usize,
    lighting_bytes: usize,
) -> Result<Q2DecoupledLightmaps, BinaryError> {
    let source = "BSPX DECOUPLED_LM";
    let mut reader = BinaryReader::new(lump.bytes, source);
    reader.records(0, reader.length(), 40)?;
    if face_count > reader.length() / 40 {
        return Err(BinaryError {
            input: source.to_string(),
            offset: 0,
            message: "fewer lightmaps than faces".to_string(),
        });
    }
    let mut faces = Vec::with_capacity(face_count);
    let mut diagnostics = Vec::new();
    for index in 0..face_count {
        let width = u32::from(reader.u16()?);
        let height = u32::from(reader.u16()?);
        let raw_offset = reader.u32()?;
        let mut lighting_offset = if raw_offset == 0xffff_ffff {
            None
        } else {
            Some(raw_offset)
        };
        if let Some(offset) = lighting_offset {
            if offset as usize >= lighting_bytes {
                diagnostics.push(format!(
                    "DECOUPLED_LM face {index} has invalid lighting offset {offset}"
                ));
                lighting_offset = None;
            }
        }
        let s = vec3(&mut reader)?;
        let x = reader.finite_f32()?;
        let t = vec3(&mut reader)?;
        let y = reader.finite_f32()?;
        faces.push(DecoupledLightmap {
            width,
            height,
            lighting_offset,
            axes: [s, t],
            offset: [x, y],
        });
    }
    Ok(Q2DecoupledLightmaps { faces, diagnostics })
}

/// Lightgrid octree child (`Q2LightgridChild`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Q2LightgridChild {
    /// Occluded cell.
    Occluded,
    /// Leaf index.
    Leaf(u32),
    /// Node index.
    Node(u32),
}

/// Lightgrid octree node (`Q2LightgridNode`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Q2LightgridNode {
    /// Split point in integer grid coordinates.
    pub point: [u32; 3],
    /// Octree children.
    pub children: [Q2LightgridChild; 8],
}

/// Lightgrid sample (`Q2LightgridSample`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Q2LightgridSample {
    /// Light style, or 255 for no sample.
    pub style: u8,
    /// RGB sample.
    pub rgb: [u8; 3],
}

/// Lightgrid leaf (`Q2LightgridLeaf`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Q2LightgridLeaf {
    /// Minimum corner in integer grid coordinates.
    pub min: [u32; 3],
    /// Size in grid points.
    pub size: [u32; 3],
    /// First sample index.
    pub first_sample: usize,
    /// Grid point count.
    pub point_count: u64,
}

/// Lightgrid octree (`Q2Lightgrid`).
#[derive(Debug, Clone, PartialEq)]
pub struct Q2Lightgrid {
    /// Grid spacing.
    pub spacing: [f32; 3],
    /// Reciprocal spacing.
    pub scale: [f32; 3],
    /// Minimum corner.
    pub min: [f32; 3],
    /// Grid size.
    pub size: [u32; 3],
    /// Styles per grid point.
    pub style_count: u8,
    /// Octree root.
    pub root: Q2LightgridChild,
    /// Octree nodes.
    pub nodes: Vec<Q2LightgridNode>,
    /// Leaves.
    pub leaves: Vec<Q2LightgridLeaf>,
    /// Samples: every grid point has `style_count` entries.
    pub samples: Vec<Q2LightgridSample>,
}

fn lightgrid_child(reader: &mut BinaryReader<'_>) -> Result<Q2LightgridChild, BinaryError> {
    let value = reader.u32()?;
    if value & 0x4000_0000 != 0 {
        return Ok(Q2LightgridChild::Occluded);
    }
    if value & 0x8000_0000 != 0 {
        return Ok(Q2LightgridChild::Leaf(value & 0x7fff_ffff));
    }
    Ok(Q2LightgridChild::Node(value))
}

fn integer_vec3(reader: &mut BinaryReader<'_>) -> Result<[u32; 3], BinaryError> {
    Ok([reader.u32()?, reader.u32()?, reader.u32()?])
}

const OCCLUDED_SAMPLE: Q2LightgridSample = Q2LightgridSample {
    style: 255,
    rgb: [255, 255, 255],
};

/// Read the BSPX lightgrid octree (`readQ2Lightgrid`).
///
/// The file header orders spacing, size, minimum, styles, root, and node
/// count.
pub fn read_q2_lightgrid(lump: &Q2BspxLump<'_>) -> Result<Q2Lightgrid, BinaryError> {
    let source = "BSPX LIGHTGRID_OCTREE";
    let fail = |offset: usize, message: &str| BinaryError {
        input: source.to_string(),
        offset,
        message: message.to_string(),
    };
    let mut reader = BinaryReader::new(lump.bytes, source);
    let spacing = vec3(&mut reader)?;
    if spacing[0] <= 0.0 || spacing[1] <= 0.0 || spacing[2] <= 0.0 {
        return Err(fail(0, "lightgrid spacing must be positive"));
    }
    let scale = [1.0 / spacing[0], 1.0 / spacing[1], 1.0 / spacing[2]];
    let size = integer_vec3(&mut reader)?;
    let min = vec3(&mut reader)?;
    let style_count = reader.u8()?;
    if !(1..=4).contains(&style_count) {
        return Err(fail(36, "invalid lightgrid style count"));
    }
    let root = lightgrid_child(&mut reader)?;
    let node_count = reader.u32()?;
    reader.records(reader.offset(), node_count as usize * 44, 44)?;
    let mut nodes = Vec::with_capacity(node_count as usize);
    for _ in 0..node_count {
        nodes.push(Q2LightgridNode {
            point: integer_vec3(&mut reader)?,
            children: [
                lightgrid_child(&mut reader)?,
                lightgrid_child(&mut reader)?,
                lightgrid_child(&mut reader)?,
                lightgrid_child(&mut reader)?,
                lightgrid_child(&mut reader)?,
                lightgrid_child(&mut reader)?,
                lightgrid_child(&mut reader)?,
                lightgrid_child(&mut reader)?,
            ],
        });
    }
    let leaf_count = reader.u32()?;
    if leaf_count as usize > reader.remaining() / 24 {
        return Err(fail(reader.offset() - 4, "invalid lightgrid leaf count"));
    }
    let mut leaves = Vec::with_capacity(leaf_count as usize);
    let mut samples = Vec::new();
    for _ in 0..leaf_count {
        let leaf_min = integer_vec3(&mut reader)?;
        let leaf_size = integer_vec3(&mut reader)?;
        let point_count = leaf_size[0] as u64 * leaf_size[1] as u64 * leaf_size[2] as u64;
        if point_count > reader.remaining() as u64 {
            return Err(fail(reader.offset(), "invalid lightgrid point count"));
        }
        let first_sample = samples.len();
        for _ in 0..point_count {
            let count = reader.u8()?;
            if count != 255 && count > style_count {
                return Err(fail(reader.offset() - 1, "too many sample styles"));
            }
            for style in 0..style_count {
                if count != 255 && style < count {
                    samples.push(Q2LightgridSample {
                        style: reader.u8()?,
                        rgb: [reader.u8()?, reader.u8()?, reader.u8()?],
                    });
                } else {
                    samples.push(OCCLUDED_SAMPLE);
                }
            }
        }
        leaves.push(Q2LightgridLeaf {
            min: leaf_min,
            size: leaf_size,
            first_sample,
            point_count,
        });
    }
    let mut pending = vec![root];
    let mut visited = vec![false; nodes.len()];
    while let Some(next) = pending.pop() {
        if leaves.is_empty() {
            break;
        }
        match next {
            Q2LightgridChild::Occluded => {}
            Q2LightgridChild::Leaf(index) => {
                if index as usize >= leaves.len() {
                    return Err(fail(37, "invalid lightgrid leaf reference"));
                }
            }
            Q2LightgridChild::Node(index) => {
                let slot = visited.get_mut(index as usize);
                match slot {
                    Some(seen) if !*seen => {
                        *seen = true;
                        let node = &nodes[index as usize];
                        pending.extend(node.children.iter().rev());
                    }
                    _ => return Err(fail(37, "invalid or repeated lightgrid node")),
                }
            }
        }
    }
    Ok(Q2Lightgrid {
        spacing,
        scale,
        min,
        size,
        style_count,
        root,
        nodes,
        leaves,
        samples,
    })
}

/// Look up lightgrid samples for an integer grid coordinate
/// (`lookupQ2Lightgrid`).
///
/// Callers convert world coordinates using the grid minimum and scale.
/// Non-integer points, occluded cells, and out-of-leaf points return
/// `None`.
#[must_use]
pub fn lookup_q2_lightgrid(grid: &Q2Lightgrid, point: [f32; 3]) -> Option<&[Q2LightgridSample]> {
    if point[0].fract() != 0.0 || point[1].fract() != 0.0 || point[2].fract() != 0.0 {
        return None;
    }
    let point = [point[0] as i64, point[1] as i64, point[2] as i64];
    let mut next = grid.root;
    while let Q2LightgridChild::Node(index) = next {
        let node = grid.nodes.get(index as usize)?;
        let octant = (usize::from(point[0] >= node.point[0] as i64) << 2)
            | (usize::from(point[1] >= node.point[1] as i64) << 1)
            | usize::from(point[2] >= node.point[2] as i64);
        next = node.children[octant];
    }
    if next == Q2LightgridChild::Occluded {
        return None;
    }
    let Q2LightgridChild::Leaf(index) = next else {
        return None;
    };
    let leaf = grid.leaves.get(index as usize)?;
    let local = [
        point[0] - leaf.min[0] as i64,
        point[1] - leaf.min[1] as i64,
        point[2] - leaf.min[2] as i64,
    ];
    if local[0] < 0
        || local[1] < 0
        || local[2] < 0
        || local[0] >= leaf.size[0] as i64
        || local[1] >= leaf.size[1] as i64
        || local[2] >= leaf.size[2] as i64
    {
        return None;
    }
    let first = leaf.first_sample
        + (leaf.size[0] as usize * (leaf.size[1] as usize * local[2] as usize + local[1] as usize) + local[0] as usize)
            * grid.style_count as usize;
    grid.samples.get(first..first + grid.style_count as usize)
}

/// Visibility table kind for [`decompress_q2_visibility`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Q2VisKind {
    /// Potentially visible set.
    Pvs,
    /// Potentially hearable set.
    Phs,
}

/// Decompress one visibility row (`decompressQ2Visibility`).
///
/// PVS and PHS offsets are relative to the complete visibility lump,
/// including its header.
pub fn decompress_q2_visibility(
    visibility: Option<&Q2Visibility<'_>>,
    cluster: i32,
    kind: Q2VisKind,
    cluster_count: usize,
) -> Result<Vec<u8>, BinaryError> {
    let count = visibility.map_or(cluster_count, |vis| vis.clusters.len());
    let mut row = vec![0u8; count.div_ceil(8)];
    if cluster == -1 {
        return Ok(row);
    }
    let Some(vis) = visibility else {
        row.fill(255);
        return Ok(row);
    };
    let entry = vis.clusters.get(cluster as usize).unwrap_or_else(|| {
        panic!("Invalid Q2 visibility cluster {cluster}");
    });
    let offset = match kind {
        Q2VisKind::Pvs => entry.pvs_offset,
        Q2VisKind::Phs => entry.phs_offset,
    };
    if offset == -1 {
        row.fill(255);
        return Ok(row);
    }
    let name = match kind {
        Q2VisKind::Pvs => "pvs",
        Q2VisKind::Phs => "phs",
    };
    let source = format!("Q2 {name} cluster {cluster}");
    let mut reader = BinaryReader::new(vis.compressed, &source);
    reader.seek(offset as usize)?;
    let mut output = 0;
    while output < row.len() {
        let value = reader.u8()?;
        if value != 0 {
            row[output] = value;
            output += 1;
            continue;
        }
        let run = reader.u8()?;
        if run == 0 || run as usize > row.len() - output {
            return Err(BinaryError {
                input: source,
                offset: reader.offset() - 1,
                message: "invalid visibility zero run".to_string(),
            });
        }
        output += run as usize;
    }
    Ok(row)
}

fn read_planes(reader: &mut BinaryReader<'_>) -> Result<Vec<Plane>, BinaryError> {
    let mut planes = Vec::new();
    while reader.remaining() > 0 {
        let normal = vec3(reader)?;
        let distance = reader.finite_f32()?;
        let plane_type = reader.i32()?;
        let signbits = u8::from(normal[0] < 0.0) | u8::from(normal[1] < 0.0) << 1 | u8::from(normal[2] < 0.0) << 2;
        planes.push(Plane {
            normal,
            distance,
            plane_type,
            signbits,
        });
    }
    Ok(planes)
}

/// Read a Q2 map (`readQ2Bsp`).
pub fn read_q2_bsp<'a>(data: &'a [u8], source: &str) -> Result<Q2Bsp<'a>, BinaryError> {
    let mut reader = BinaryReader::new(data, source);
    let magic = reader.fixed_byte_string(4)?;
    if magic != "IBSP" && magic != "QBSP" {
        return Err(BinaryError {
            input: source.to_string(),
            offset: 0,
            message: format!("unsupported Q2 map identifier {magic}"),
        });
    }
    let version = reader.u32()?;
    if version != Q2_BSP_VERSION {
        return Err(BinaryError {
            input: source.to_string(),
            offset: 4,
            message: format!("unsupported Q2 BSP version {version}"),
        });
    }
    let extended = magic == "QBSP";
    let mut lumps = Vec::with_capacity(LUMP_COUNT);
    let mut diagnostics = Vec::new();
    let mut end_of_lumps = 160u32;
    for number in 0..LUMP_COUNT {
        let offset = reader.u32()?;
        let mut length = reader.u32()?;
        // q2repro accepts the overlong entity lump produced by some older compilers.
        if number == LUMP_ENTITIES
            && (offset as usize) < reader.length()
            && length as usize > reader.length() - offset as usize
        {
            length = (reader.length() - offset as usize) as u32;
            diagnostics.push("Clamped entity lump to the end of the file".to_string());
        }
        reader.section(offset as usize, length as usize)?;
        lumps.push(Q2Lump { offset, length });
        end_of_lumps = end_of_lumps.max(offset.saturating_add(length));
    }
    let lump = |number: usize| lumps[number];
    let records = |number: usize, stride: usize| -> Result<BinaryReader<'a>, BinaryError> {
        let entry = lump(number);
        reader.records(entry.offset as usize, entry.length as usize, stride)
    };
    let entity_entry = lump(LUMP_ENTITIES);
    let entity_bytes = reader.view(entity_entry.offset as usize, entity_entry.length as usize)?;
    let entities =
        BinaryReader::new(entity_bytes, &format!("{source}:entities")).fixed_byte_string(entity_bytes.len())?;
    let planes = read_planes(&mut records(LUMP_PLANES, 20)?)?;
    let mut vertex_reader = records(LUMP_VERTICES, 12)?;
    let mut vertices = Vec::new();
    while vertex_reader.remaining() > 0 {
        vertices.push(vec3(&mut vertex_reader)?);
    }
    let visibility = read_visibility(&reader, source, lump(LUMP_VISIBILITY))?;
    let mut node_reader = records(LUMP_NODES, if extended { 44 } else { 28 })?;
    let mut nodes = Vec::new();
    while node_reader.remaining() > 0 {
        nodes.push(Node {
            plane: node_reader.u32()?,
            children: [child(node_reader.i32()?), child(node_reader.i32()?)],
            bounds: bounds(&mut node_reader, extended)?,
            faces: range(&mut node_reader, extended)?,
        });
    }
    let mut tex_reader = records(LUMP_TEXTURE_INFO, 76)?;
    let mut texture_info = Vec::new();
    while tex_reader.remaining() > 0 {
        texture_info.push(Q2RawTextureInfo {
            projection_s: projection(&mut tex_reader)?,
            projection_t: projection(&mut tex_reader)?,
            flags: tex_reader.i32()?,
            value: tex_reader.i32()?,
            name: tex_reader.fixed_byte_string(32)?,
            next: tex_reader.i32()?,
        });
    }
    let mut face_reader = records(LUMP_FACES, if extended { 28 } else { 20 })?;
    let mut faces = Vec::new();
    while face_reader.remaining() > 0 {
        let plane = index(&mut face_reader, extended)?;
        let draw_flags = index(&mut face_reader, extended)?;
        let first = face_reader.u32()?;
        let count = index(&mut face_reader, extended)?;
        let texture_info = index(&mut face_reader, extended)?;
        let styles = [
            face_reader.u8()?,
            face_reader.u8()?,
            face_reader.u8()?,
            face_reader.u8()?,
        ];
        let lighting_offset = face_reader.i32()?;
        faces.push(Q2RawFace {
            plane,
            draw_flags,
            edges: IndexRange { first, count },
            texture_info,
            styles,
            lighting_offset,
        });
    }
    let lighting_entry = lump(LUMP_LIGHTING);
    let lighting = reader.view(lighting_entry.offset as usize, lighting_entry.length as usize)?;
    let mut leaf_reader = records(LUMP_LEAVES, if extended { 52 } else { 28 })?;
    let mut leaves = Vec::new();
    while leaf_reader.remaining() > 0 {
        leaves.push(Q2RawLeaf {
            contents: leaf_reader.i32()?,
            cluster: nullable_index(&mut leaf_reader, extended)?,
            area: index(&mut leaf_reader, extended)?,
            bounds: bounds(&mut leaf_reader, extended)?,
            faces: range(&mut leaf_reader, extended)?,
            brushes: range(&mut leaf_reader, extended)?,
        });
    }
    let mut leaf_face_reader = records(LUMP_LEAF_FACES, if extended { 4 } else { 2 })?;
    let mut leaf_faces = Vec::new();
    while leaf_face_reader.remaining() > 0 {
        leaf_faces.push(index(&mut leaf_face_reader, extended)?);
    }
    let mut leaf_brush_reader = records(LUMP_LEAF_BRUSHES, if extended { 4 } else { 2 })?;
    let mut leaf_brushes = Vec::new();
    while leaf_brush_reader.remaining() > 0 {
        leaf_brushes.push(index(&mut leaf_brush_reader, extended)?);
    }
    let mut edge_reader = records(LUMP_EDGES, if extended { 8 } else { 4 })?;
    let mut edges = Vec::new();
    while edge_reader.remaining() > 0 {
        edges.push(Edge {
            vertices: [index(&mut edge_reader, extended)?, index(&mut edge_reader, extended)?],
        });
    }
    let mut surface_reader = records(LUMP_SURFACE_EDGES, 4)?;
    let mut surface_edges = Vec::new();
    while surface_reader.remaining() > 0 {
        surface_edges.push(surface_reader.i32()?);
    }
    let mut model_reader = records(LUMP_MODELS, 48)?;
    let mut models = Vec::new();
    while model_reader.remaining() > 0 {
        models.push(Q2WorldModel {
            bounds: bounds(&mut model_reader, true)?,
            origin: vec3(&mut model_reader)?,
            headnode: model_reader.i32()?,
            faces: range(&mut model_reader, true)?,
        });
    }
    let mut brush_reader = records(LUMP_BRUSHES, 12)?;
    let mut brushes = Vec::new();
    while brush_reader.remaining() > 0 {
        brushes.push(Q2Brush {
            sides: range(&mut brush_reader, true)?,
            contents: brush_reader.i32()?,
        });
    }
    let mut side_reader = records(LUMP_BRUSH_SIDES, if extended { 8 } else { 4 })?;
    let mut brush_sides = Vec::new();
    while side_reader.remaining() > 0 {
        brush_sides.push(Q2BrushSide {
            plane: index(&mut side_reader, extended)?,
            texture_info: nullable_index(&mut side_reader, extended)?,
        });
    }
    let pop_entry = lump(LUMP_POP);
    let pop = reader.view(pop_entry.offset as usize, pop_entry.length as usize)?;
    let mut area_reader = records(LUMP_AREAS, 8)?;
    let mut areas = Vec::new();
    while area_reader.remaining() > 0 {
        let count = area_reader.u32()?;
        let first = area_reader.u32()?;
        areas.push(Q2Area {
            portals: IndexRange { first, count },
        });
    }
    let mut portal_reader = records(LUMP_AREA_PORTALS, 8)?;
    let mut area_portals = Vec::new();
    while portal_reader.remaining() > 0 {
        area_portals.push(Q2AreaPortal {
            portal: portal_reader.u32()?,
            other_area: portal_reader.u32()?,
        });
    }
    let bspx = read_q2_bspx(data, source, end_of_lumps)?;
    let map = Q2Bsp {
        source: source.to_string(),
        format: if extended { Q2Format::Qbsp } else { Q2Format::Ibsp38 },
        version,
        lumps,
        entities,
        entity_bytes,
        planes,
        vertices,
        edges,
        surface_edges,
        nodes,
        leaves,
        leaf_faces,
        leaf_brushes,
        texture_info,
        faces,
        brushes,
        brush_sides,
        models,
        areas,
        area_portals,
        visibility,
        lighting,
        pop,
        bspx,
        diagnostics,
    };
    validate_q2_bsp(&map)?;
    Ok(map)
}

fn validate_q2_bsp(map: &Q2Bsp<'_>) -> Result<(), BinaryError> {
    let fail = |message: String| BinaryError {
        input: map.source.clone(),
        offset: 0,
        message,
    };
    let reference = |value: i64, count: usize, label: &str| -> Result<(), BinaryError> {
        if value < 0 || value >= count as i64 {
            return Err(fail(format!("Invalid {label} index {value} for {count} records")));
        }
        Ok(())
    };
    let span = |value: IndexRange, count: usize, label: &str| -> Result<(), BinaryError> {
        if value.first as u64 + value.count as u64 > count as u64 {
            return Err(fail(format!("Invalid {label} range")));
        }
        Ok(())
    };
    let node_reference = |value: NodeChild| -> Result<(), BinaryError> {
        match value {
            NodeChild::Node(index) => reference(i64::from(index), map.nodes.len(), "node"),
            NodeChild::Leaf(index) => reference(i64::from(index), map.leaves.len(), "leaf"),
        }
    };
    if map.models.is_empty() || map.nodes.is_empty() || map.leaves.is_empty() {
        return Err(fail("Map must contain models, nodes and leaves".to_string()));
    }
    if map.leaves[0].contents != 1 {
        return Err(fail("Map leaf 0 is not CONTENTS_SOLID".to_string()));
    }
    for edge in &map.edges {
        for vertex in edge.vertices {
            reference(i64::from(vertex), map.vertices.len(), "vertex")?;
        }
    }
    for edge in &map.surface_edges {
        reference((*edge as i64).abs(), map.edges.len(), "surface edge")?;
    }
    for face in &map.faces {
        reference(i64::from(face.plane), map.planes.len(), "face plane")?;
        reference(i64::from(face.texture_info), map.texture_info.len(), "face texture")?;
        span(face.edges, map.surface_edges.len(), "face edges")?;
        if face.edges.count < 3 {
            return Err(fail("Face has fewer than three edges".to_string()));
        }
        if face.lighting_offset < -1
            || (!map.lighting.is_empty() && face.lighting_offset as usize >= map.lighting.len())
        {
            return Err(fail("Invalid face lighting offset".to_string()));
        }
    }
    for texture in &map.texture_info {
        if texture.next > 0 {
            reference(i64::from(texture.next), map.texture_info.len(), "animated texture")?;
        }
    }
    for brush in &map.brushes {
        span(brush.sides, map.brush_sides.len(), "brush sides")?;
    }
    for side in &map.brush_sides {
        reference(i64::from(side.plane), map.planes.len(), "brush plane")?;
        if side.texture_info != -1 {
            reference(i64::from(side.texture_info), map.texture_info.len(), "brush texture")?;
        }
    }
    for brush in &map.leaf_brushes {
        reference(i64::from(*brush), map.brushes.len(), "leaf brush")?;
    }
    for face in &map.leaf_faces {
        reference(i64::from(*face), map.faces.len(), "leaf face")?;
    }
    for leaf in &map.leaves {
        span(leaf.faces, map.leaf_faces.len(), "leaf faces")?;
        span(leaf.brushes, map.leaf_brushes.len(), "leaf brushes")?;
        reference(i64::from(leaf.area), map.areas.len(), "leaf area")?;
        if map.visibility.is_some() && leaf.cluster != -1 {
            reference(
                i64::from(leaf.cluster),
                map.visibility.as_ref().map_or(0, |vis| vis.clusters.len()),
                "leaf cluster",
            )?;
        }
    }
    for node in &map.nodes {
        reference(i64::from(node.plane), map.planes.len(), "node plane")?;
        span(node.faces, map.faces.len(), "node faces")?;
        for value in node.children {
            node_reference(value)?;
        }
    }
    for model in &map.models {
        node_reference(child(model.headnode))?;
        span(model.faces, map.faces.len(), "model faces")?;
    }
    for area in &map.areas {
        span(area.portals, map.area_portals.len(), "area portals")?;
    }
    for portal in &map.area_portals {
        reference(i64::from(portal.other_area), map.areas.len(), "portal area")?;
        reference(i64::from(portal.portal), map.area_portals.len(), "portal")?;
    }
    // Iterative traversal also accepts the deep trees in Call of the Machine.
    let mut complete = vec![false; map.nodes.len()];
    let mut active = vec![false; map.nodes.len()];
    for model in &map.models {
        let mut pending = vec![(child(model.headnode), false)];
        while let Some((item, leaving)) = pending.pop() {
            let NodeChild::Node(number) = item else { continue };
            let number = number as usize;
            if leaving {
                active[number] = false;
                complete[number] = true;
                continue;
            }
            if active[number] {
                return Err(fail("Cycle in BSP nodes".to_string()));
            }
            if complete[number] {
                continue;
            }
            let Some(node) = map.nodes.get(number) else {
                return Err(fail("Invalid BSP node".to_string()));
            };
            active[number] = true;
            pending.push((item, true));
            pending.push((node.children[1], false));
            pending.push((node.children[0], false));
        }
    }
    Ok(())
}

/// Material resolver for world geometry (`Q2MapResources`).
///
/// Resolves `textures/<name>.mat` through the caller's content mount.
pub trait Q2MapResources {
    /// Read a material file, or return `None` when absent.
    fn read_material(&self, path: &str) -> Option<Vec<u8>>;
}

/// Decoded texture info (`Q2TextureInfo`).
#[derive(Debug, Clone, PartialEq)]
pub struct Q2TextureInfo {
    /// S projection.
    pub projection_s: [f32; 4],
    /// T projection.
    pub projection_t: [f32; 4],
    /// Surface flags.
    pub flags: i32,
    /// Surface value.
    pub value: i32,
    /// Texture name.
    pub name: String,
    /// Material name, or empty when absent or invalid.
    pub material: String,
    /// Next animation frame; zero and negative values end the chain.
    pub next: Option<u32>,
}

/// Decoded leaf (`Q2Leaf`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Q2Leaf {
    /// Stored contents.
    pub contents: i32,
    /// Contents merged with leaf brushes.
    pub merged_contents: i32,
    /// Visibility cluster, or -1 for none.
    pub cluster: i32,
    /// Area index.
    pub area: u32,
    /// Bounds.
    pub bounds: Bounds,
    /// Leaf-face range.
    pub faces: IndexRange,
    /// Leaf-brush range.
    pub brushes: IndexRange,
}

/// Decoded face (`BspFace`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Q2Face {
    /// Plane index.
    pub plane: u32,
    /// Back side.
    pub back: bool,
    /// Surface-edge range.
    pub edges: IndexRange,
    /// Texture info index.
    pub texture_info: u32,
    /// Light styles.
    pub styles: [u8; 4],
    /// Lighting offset, or `None` for none.
    pub lighting_offset: Option<u32>,
}

/// Decoded Q2 world geometry (`Q2DecodedMap`).
///
/// Source records stay available through [`Q2DecodedMap::map`]; shared
/// arrays (planes, nodes, brushes, visibility, lighting) are read from
/// there instead of being duplicated.
#[derive(Debug, Clone, PartialEq)]
pub struct Q2DecodedMap<'a> {
    /// Source records.
    pub map: Q2Bsp<'a>,
    /// Leaves with merged contents.
    pub leaves: Vec<Q2Leaf>,
    /// Faces with decoded sides and lighting offsets.
    pub faces: Vec<Q2Face>,
    /// Texture info with resolved materials.
    pub texture_info: Vec<Q2TextureInfo>,
    /// Models with bounds expanded by one unit.
    pub models: Vec<Q2WorldModel>,
    /// Decoupled lightmaps, or `None` when the BSPX lump is absent.
    pub decoupled_lightmaps: Option<Vec<DecoupledLightmap>>,
    /// Lightgrid, or `None` when the BSPX lump is absent or invalid.
    pub lightgrid: Option<Q2Lightgrid>,
    /// Face normals, or `None` when the BSPX lump is absent or invalid.
    pub face_normals: Option<Q2FaceNormals>,
    /// Loader diagnostics.
    pub diagnostics: Vec<String>,
}

fn resolve_materials(
    map: &Q2Bsp<'_>,
    resources: Option<&dyn Q2MapResources>,
    diagnostics: &mut Vec<String>,
) -> Vec<Q2TextureInfo> {
    use std::collections::HashMap;
    let mut materials: HashMap<String, String> = HashMap::new();
    map.texture_info
        .iter()
        .map(|texture| {
            let name: String = texture.name.chars().take(31).collect();
            let key = name.to_lowercase();
            let material = materials.entry(key).or_insert_with(|| {
                let path = format!("textures/{name}.mat");
                let bytes = resources.and_then(|resources| resources.read_material(&path));
                let material = bytes.map_or(String::new(), |bytes| {
                    let len = bytes.len().min(15);
                    BinaryReader::new(&bytes, &path)
                        .fixed_byte_string(len)
                        .unwrap_or_default()
                });
                if !material.is_empty()
                    && !material
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
                {
                    diagnostics.push(format!("Invalid material in {path}"));
                    return String::new();
                }
                material
            });
            Q2TextureInfo {
                projection_s: texture.projection_s,
                projection_t: texture.projection_t,
                flags: texture.flags,
                value: texture.value,
                name: texture.name.clone(),
                material: material.clone(),
                next: if texture.next > 0 {
                    Some(texture.next as u32)
                } else {
                    None
                },
            }
        })
        .collect()
}

/// Adapt source records to world geometry (`toQ2WorldGeometry`).
pub fn to_q2_world_geometry<'a>(
    map: Q2Bsp<'a>,
    resources: Option<&dyn Q2MapResources>,
) -> Result<Q2DecodedMap<'a>, BinaryError> {
    let mut diagnostics: Vec<String> = map.diagnostics.clone();
    if let Some(bspx) = &map.bspx {
        diagnostics.extend(bspx.diagnostics.iter().cloned());
    }
    let mut decoupled_lightmaps = None;
    let mut lightgrid = None;
    let mut face_normals = None;
    if let Some(bspx) = &map.bspx {
        if let Some(lump) = bspx.get("DECOUPLED_LM") {
            match read_q2_decoupled_lightmaps(lump, map.faces.len(), map.lighting.len()) {
                Ok(result) => {
                    diagnostics.extend(result.diagnostics);
                    decoupled_lightmaps = Some(result.faces);
                }
                Err(error) => diagnostics.push(error.message),
            }
        }
        if let Some(lump) = bspx.get("LIGHTGRID_OCTREE") {
            match read_q2_lightgrid(lump) {
                Ok(grid) => lightgrid = Some(grid),
                Err(error) => diagnostics.push(error.message),
            }
        }
        if let Some(lump) = bspx.get("FACENORMALS") {
            let corners: usize = map.faces.iter().map(|face| face.edges.count as usize).sum();
            match read_q2_face_normals(lump, corners) {
                Ok(normals) => face_normals = Some(normals),
                Err(error) => diagnostics.push(error.message),
            }
        }
    }
    let mut leaves = Vec::with_capacity(map.leaves.len());
    for (index, leaf) in map.leaves.iter().enumerate() {
        let mut merged_contents = leaf.contents;
        if index != 0 {
            let end = leaf.brushes.first as usize + leaf.brushes.count as usize;
            for offset in leaf.brushes.first as usize..end {
                let brush = map
                    .leaf_brushes
                    .get(offset)
                    .and_then(|brush| map.brushes.get(*brush as usize));
                let Some(brush) = brush else {
                    return Err(BinaryError {
                        input: map.source.clone(),
                        offset: 0,
                        message: "invalid leaf brush reference".to_string(),
                    });
                };
                merged_contents |= brush.contents;
            }
        }
        leaves.push(Q2Leaf {
            contents: leaf.contents,
            merged_contents,
            cluster: if leaf.cluster != -1 && map.visibility.is_none() {
                0
            } else {
                leaf.cluster
            },
            area: leaf.area,
            bounds: leaf.bounds,
            faces: leaf.faces,
            brushes: leaf.brushes,
        });
    }
    let faces = map
        .faces
        .iter()
        .enumerate()
        .map(|(face_index, face)| {
            let coupled = decoupled_lightmaps
                .as_ref()
                .and_then(|lightmaps| lightmaps.get(face_index))
                .map(|lightmap| lightmap.lighting_offset);
            let lighting_offset = match coupled {
                Some(offset) => offset,
                None if face.lighting_offset == -1 || map.lighting.is_empty() => None,
                None => Some(face.lighting_offset as u32),
            };
            Q2Face {
                plane: face.plane,
                back: face.draw_flags & 1 != 0,
                edges: face.edges,
                texture_info: face.texture_info,
                styles: face.styles,
                lighting_offset,
            }
        })
        .collect();
    let texture_info = resolve_materials(&map, resources, &mut diagnostics);
    // The loader expands submodel bounds by one unit, as both Q2 renderers and cmodel do.
    let models = map
        .models
        .iter()
        .map(|model| {
            let min = model.bounds.min;
            let max = model.bounds.max;
            Q2WorldModel {
                bounds: Bounds {
                    min: [min[0] - 1.0, min[1] - 1.0, min[2] - 1.0],
                    max: [max[0] + 1.0, max[1] + 1.0, max[2] + 1.0],
                },
                origin: model.origin,
                headnode: model.headnode,
                faces: model.faces,
            }
        })
        .collect();
    Ok(Q2DecodedMap {
        map,
        leaves,
        faces,
        texture_info,
        models,
        decoupled_lightmaps,
        lightgrid,
        face_normals,
        diagnostics,
    })
}

/// Read a Q2 map and adapt it to world geometry (`decodeQ2Map`).
pub fn decode_q2_map<'a>(
    data: &'a [u8],
    source: &str,
    resources: Option<&dyn Q2MapResources>,
) -> Result<Q2DecodedMap<'a>, BinaryError> {
    to_q2_world_geometry(read_q2_bsp(data, source)?, resources)
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_core::binary::BinaryWriter;

    struct Fixture {
        bytes: Vec<u8>,
    }

    fn fixture() -> Fixture {
        // Minimal valid map: 1 plane, 4 vertices, 4 edges, 4 surface edges,
        // 1 node, 2 leaves (solid + empty), 1 texture, 1 face, 1 model,
        // 1 brush + 1 side, 1 area + 1 portal, empty visibility/lighting.
        let mut lumps: Vec<Vec<u8>> = Vec::new();
        for _ in 0..LUMP_COUNT {
            lumps.push(Vec::new());
        }
        lumps[LUMP_ENTITIES] = b"{\"classname\" \"worldspawn\"}\n".to_vec();
        let mut writer = BinaryWriter::new(20);
        for value in [0.0f32, 0.0, 1.0, 8.0] {
            writer.f32(value).unwrap();
        }
        writer.i32(2).unwrap();
        lumps[LUMP_PLANES] = writer.finish();
        let mut writer = BinaryWriter::new(48);
        for vertex in [[0.0f32, 0.0, 0.0], [1.0, 0.0, 0.0], [1.0, 1.0, 0.0], [0.0, 1.0, 0.0]] {
            for component in vertex {
                writer.f32(component).unwrap();
            }
        }
        lumps[LUMP_VERTICES] = writer.finish();
        let mut writer = BinaryWriter::new(28);
        writer.u32(0).unwrap();
        writer.i32(-1).unwrap();
        writer.i32(-2).unwrap();
        for value in [0i16, 0, 0, 16, 16, 16] {
            writer.i16(value).unwrap();
        }
        writer.u16(0).unwrap();
        writer.u16(1).unwrap();
        lumps[LUMP_NODES] = writer.finish();
        let mut writer = BinaryWriter::new(76);
        for value in [1.0f32, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0] {
            writer.f32(value).unwrap();
        }
        writer.i32(0).unwrap();
        writer.i32(0).unwrap();
        let mut name = [0u8; 32];
        name[..4].copy_from_slice(b"wall");
        writer.bytes(&name).unwrap();
        writer.i32(0).unwrap();
        lumps[LUMP_TEXTURE_INFO] = writer.finish();
        let mut writer = BinaryWriter::new(20);
        writer.u16(0).unwrap();
        writer.u16(1).unwrap();
        writer.u32(0).unwrap();
        writer.u16(4).unwrap();
        writer.u16(0).unwrap();
        writer.bytes(&[0, 0, 0, 0]).unwrap();
        writer.i32(-1).unwrap();
        lumps[LUMP_FACES] = writer.finish();
        let mut writer = BinaryWriter::new(56);
        writer.i32(1).unwrap();
        writer.u16(0xffff).unwrap();
        writer.u16(0).unwrap();
        for value in [0i16, 0, 0, 0, 0, 0] {
            writer.i16(value).unwrap();
        }
        writer.u16(0).unwrap();
        writer.u16(0).unwrap();
        writer.u16(0).unwrap();
        writer.u16(0).unwrap();
        writer.i32(0).unwrap();
        writer.u16(0xffff).unwrap();
        writer.u16(0).unwrap();
        for value in [0i16, 0, 0, 16, 16, 16] {
            writer.i16(value).unwrap();
        }
        writer.u16(0).unwrap();
        writer.u16(1).unwrap();
        writer.u16(0).unwrap();
        writer.u16(1).unwrap();
        lumps[LUMP_LEAVES] = writer.finish();
        let mut writer = BinaryWriter::new(2);
        writer.u16(0).unwrap();
        lumps[LUMP_LEAF_FACES] = writer.finish();
        let mut writer = BinaryWriter::new(2);
        writer.u16(0).unwrap();
        lumps[LUMP_LEAF_BRUSHES] = writer.finish();
        let mut writer = BinaryWriter::new(16);
        for edge in [[0u16, 1], [1, 2], [2, 3], [3, 0]] {
            writer.u16(edge[0]).unwrap();
            writer.u16(edge[1]).unwrap();
        }
        lumps[LUMP_EDGES] = writer.finish();
        let mut writer = BinaryWriter::new(16);
        for edge in [0i32, 1, 2, 3] {
            writer.i32(edge).unwrap();
        }
        lumps[LUMP_SURFACE_EDGES] = writer.finish();
        let mut writer = BinaryWriter::new(48);
        for value in [0.0f32, 0.0, 0.0, 16.0, 16.0, 16.0, 0.0, 0.0, 0.0] {
            writer.f32(value).unwrap();
        }
        writer.i32(0).unwrap();
        writer.u32(0).unwrap();
        writer.u32(1).unwrap();
        lumps[LUMP_MODELS] = writer.finish();
        let mut writer = BinaryWriter::new(12);
        writer.u32(0).unwrap();
        writer.u32(1).unwrap();
        writer.i32(0).unwrap();
        lumps[LUMP_BRUSHES] = writer.finish();
        let mut writer = BinaryWriter::new(4);
        writer.u16(0).unwrap();
        writer.u16(0).unwrap();
        lumps[LUMP_BRUSH_SIDES] = writer.finish();
        let mut writer = BinaryWriter::new(8);
        writer.u32(1).unwrap();
        writer.u32(0).unwrap();
        lumps[LUMP_AREAS] = writer.finish();
        let mut writer = BinaryWriter::new(8);
        writer.u32(0).unwrap();
        writer.u32(0).unwrap();
        lumps[LUMP_AREA_PORTALS] = writer.finish();
        let mut file = BinaryWriter::new(4096);
        file.bytes(b"IBSP").unwrap();
        file.u32(Q2_BSP_VERSION).unwrap();
        let mut offset = 160u32;
        for lump in &lumps {
            file.u32(offset).unwrap();
            file.u32(lump.len() as u32).unwrap();
            offset += lump.len() as u32;
        }
        for lump in &lumps {
            file.bytes(lump).unwrap();
        }
        Fixture { bytes: file.finish() }
    }

    #[test]
    fn q2_round_trip() {
        let fixture = fixture();
        let map = read_q2_bsp(&fixture.bytes, "<test>").unwrap();
        assert_eq!(map.format, Q2Format::Ibsp38);
        assert_eq!(map.version, 38);
        assert_eq!(map.planes.len(), 1);
        assert_eq!(map.planes[0].signbits, 0);
        assert_eq!(map.vertices.len(), 4);
        assert_eq!(map.nodes.len(), 1);
        assert_eq!(map.texture_info[0].name, "wall");
        assert_eq!(map.faces[0].edges.count, 4);
        assert_eq!(map.leaves.len(), 2);
        assert_eq!(map.models.len(), 1);
        assert!(map.visibility.is_none());
        assert!(map.bspx.is_none());
        assert!(map.diagnostics.is_empty());

        struct NoResources;
        impl Q2MapResources for NoResources {
            fn read_material(&self, _path: &str) -> Option<Vec<u8>> {
                None
            }
        }
        let world = to_q2_world_geometry(map, Some(&NoResources)).unwrap();
        assert_eq!(world.faces[0].back, true);
        assert_eq!(world.faces[0].lighting_offset, None);
        assert_eq!(world.texture_info[0].material, "");
        assert_eq!(world.texture_info[0].next, None);
        assert_eq!(world.leaves[1].merged_contents, 0);
        assert_eq!(world.leaves[1].cluster, -1);
        assert_eq!(world.models[0].bounds.min, [-1.0, -1.0, -1.0]);
        assert_eq!(world.models[0].bounds.max, [17.0, 17.0, 17.0]);
        assert!(world.decoupled_lightmaps.is_none());
    }

    #[test]
    fn q2_rejects_bad_input() {
        let good = fixture().bytes;
        let mut bad_magic = good.clone();
        bad_magic[0] = b'X';
        let error = read_q2_bsp(&bad_magic, "<test>").unwrap_err();
        assert!(error.message.contains("unsupported Q2 map identifier"));
        let mut bad_version = good.clone();
        bad_version[7] = 1;
        let error = read_q2_bsp(&bad_version, "<test>").unwrap_err();
        assert!(error.message.contains("unsupported Q2 BSP version"));
        assert!(read_q2_bsp(&good[..100], "<test>").is_err());
        // Corrupt leaf 0 contents away from CONTENTS_SOLID.
        let mut bad_leaf = good.clone();
        let offset = u32::from_le_bytes([
            bad_leaf[8 + 8 * 8],
            bad_leaf[8 + 8 * 8 + 1],
            bad_leaf[8 + 8 * 8 + 2],
            bad_leaf[8 + 8 * 8 + 3],
        ]) as usize;
        bad_leaf[offset] = 0;
        let error = read_q2_bsp(&bad_leaf, "<test>").unwrap_err();
        assert_eq!(error.message, "Map leaf 0 is not CONTENTS_SOLID");
    }

    #[test]
    fn q2_visibility_decompression() {
        // One cluster; PVS row is a single set byte, PHS is absent.
        let lump = [1u8, 0, 0, 0, 12, 0, 0, 0, 255, 255, 255, 255, 1];
        let map_lump = Q2Lump {
            offset: 0,
            length: lump.len() as u32,
        };
        let reader = BinaryReader::new(&lump, "<test>");
        let vis = read_visibility(&reader, "<test>", map_lump).unwrap().unwrap();
        assert_eq!(vis.clusters.len(), 1);
        let row = decompress_q2_visibility(Some(&vis), 0, Q2VisKind::Pvs, 0).unwrap();
        assert_eq!(row, vec![1]);
        let row = decompress_q2_visibility(Some(&vis), 0, Q2VisKind::Phs, 0).unwrap();
        assert_eq!(row, vec![255]);
        let row = decompress_q2_visibility(Some(&vis), -1, Q2VisKind::Pvs, 0).unwrap();
        assert_eq!(row, vec![0]);
        let row = decompress_q2_visibility(None, 5, Q2VisKind::Pvs, 8).unwrap();
        assert_eq!(row, vec![255]);
    }
}
