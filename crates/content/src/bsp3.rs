//! Quake III BSP map (IBSP v46, with v44 normalization) parser.
//!
//! Donor provenance: `parseQ3Bsp` in `src/formats/q3-map/decode.ts`, the
//! entity tokenizer in `src/formats/q3-map/entities.ts`, the v44 layout
//! decoder in `src/formats/q3-map/ibsp44.ts`, and the scene adapter in
//! `src/formats/q3-map/world.ts`.
//!
//! Unlike the Q1/Q2 readers, lightmap and visibility payloads are owned:
//! IBSP44 normalization rebuilds the file, so borrowed slices could not
//! outlive the call. Decoded records are owned.

use std::collections::{BTreeSet, HashMap, HashSet};

use qa_core::binary::{BinaryError, BinaryReader, BinaryWriter};

use crate::bsp::{IndexRange, NodeChild};
use crate::common::Bounds;

/// IBSP magic (`0x50534249`, little-endian `"IBSP"`).
pub const Q3_MAGIC_IBSP: u32 = 0x5053_4249;
/// Q3 test-disk BSP version (normalized to v46 on load).
pub const Q3_BSP_VERSION_44: i32 = 44;
/// Q3 retail BSP version.
pub const Q3_BSP_VERSION_46: i32 = 46;
/// Header size: magic, version, and 17 lump ranges.
pub const Q3_HEADER_SIZE: usize = 144;
/// Lightmap width in samples.
pub const Q3_LIGHTMAP_WIDTH: usize = 128;
/// Lightmap height in samples.
pub const Q3_LIGHTMAP_HEIGHT: usize = 128;
/// Lightmap size in bytes (`128 * 128 * 3`).
pub const Q3_LIGHTMAP_BYTES: usize = Q3_LIGHTMAP_WIDTH * Q3_LIGHTMAP_HEIGHT * 3;
/// Entity token limit (`TOKEN_MAX`); tokens may hold 1023 characters.
pub const Q3_TOKEN_MAX: usize = 1024;

/// Q3 entity: ordered properties with source duplicate precedence
/// (later writes win, like the donor `Map`).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Q3Entity {
    /// Properties in first-seen order.
    pub properties: Vec<(String, String)>,
}

impl Q3Entity {
    /// Look up a property value.
    #[must_use]
    pub fn get(&self, key: &str) -> Option<&str> {
        self.properties
            .iter()
            .rev()
            .find(|property| property.0 == key)
            .map(|property| property.1.as_str())
    }
}

struct EntityToken {
    value: String,
    line: usize,
    column: usize,
}

struct EntityTokenizer {
    chars: Vec<char>,
    position: usize,
    line: usize,
    column: usize,
}

impl EntityTokenizer {
    fn advance(&mut self) {
        let Some(character) = self.chars.get(self.position).copied() else {
            return;
        };
        if character == '\r' {
            self.position += 1;
            if self.chars.get(self.position) == Some(&'\n') {
                self.position += 1;
            }
            self.line += 1;
            self.column = 1;
            return;
        }
        self.position += 1;
        if character == '\n' {
            self.line += 1;
            self.column = 1;
        } else {
            self.column += 1;
        }
    }

    fn starts_with(&self, text: &str) -> bool {
        let pattern: Vec<char> = text.chars().collect();
        self.chars[self.position..].starts_with(&pattern)
    }

    fn next(&mut self, source: &str, allow_line_breaks: bool) -> Result<Option<EntityToken>, BinaryError> {
        let mut crossed_line = false;
        loop {
            while self.position < self.chars.len() {
                let character = self.chars[self.position];
                if (character as u32) > 32 {
                    break;
                }
                if character == '\n' || character == '\r' {
                    crossed_line = true;
                }
                self.advance();
            }
            if self.starts_with("//") {
                self.advance();
                self.advance();
                while self.position < self.chars.len() {
                    let character = self.chars[self.position];
                    if character == '\n' || character == '\r' {
                        break;
                    }
                    self.advance();
                }
                continue;
            }
            if self.starts_with("/*") {
                self.advance();
                self.advance();
                while self.position < self.chars.len() && !self.starts_with("*/") {
                    let character = self.chars[self.position];
                    if character == '\n' || character == '\r' {
                        crossed_line = true;
                    }
                    self.advance();
                }
                if self.starts_with("*/") {
                    self.advance();
                    self.advance();
                }
                continue;
            }
            break;
        }
        if !allow_line_breaks && crossed_line {
            return Ok(None);
        }
        if self.position >= self.chars.len() {
            return Ok(None);
        }
        let line = self.line;
        let column = self.column;
        let quoted = self.chars[self.position] == '"';
        let mut value = String::new();
        if quoted {
            self.advance();
            while self.position < self.chars.len() {
                let character = self.chars[self.position];
                if character == '"' {
                    break;
                }
                value.push(character);
                self.advance();
            }
            if self.chars.get(self.position) == Some(&'"') {
                self.advance();
            }
        } else {
            while self.position < self.chars.len() {
                let character = self.chars[self.position];
                if (character as u32) <= 32 {
                    break;
                }
                value.push(character);
                self.advance();
            }
        }
        if value.chars().count() >= Q3_TOKEN_MAX {
            return Err(BinaryError {
                input: source.to_string(),
                offset: self.position,
                message: format!("{line}:{column}: token is limited to {} characters", Q3_TOKEN_MAX - 1),
            });
        }
        Ok(Some(EntityToken { value, line, column }))
    }
}

/// Parse Q3 entity text (`parseEntities`).
///
/// The first record is the worldspawn contract: callers read its
/// `classname` (`worldspawn`) plus global keys. Escapes stay literal and
/// later duplicate keys win.
pub fn parse_q3_entities(text: &str, source: &str) -> Result<Vec<Q3Entity>, BinaryError> {
    let mut tokenizer = EntityTokenizer {
        chars: text.chars().collect(),
        position: 0,
        line: 1,
        column: 1,
    };
    let mut entities = Vec::new();
    loop {
        let Some(opening) = tokenizer.next(source, true)? else {
            return Ok(entities);
        };
        if opening.value != "{" {
            return Err(BinaryError {
                input: source.to_string(),
                offset: tokenizer.position,
                message: format!("{}:{}: expected \"{{\"", opening.line, opening.column),
            });
        }
        let mut entity = Q3Entity::default();
        loop {
            let Some(key) = tokenizer.next(source, true)? else {
                return Err(BinaryError {
                    input: source.to_string(),
                    offset: tokenizer.position,
                    message: format!("{}:{}: expected key or \"}}\"", tokenizer.line, tokenizer.column),
                });
            };
            if key.value == "}" {
                entities.push(entity);
                break;
            }
            let value = tokenizer.next(source, false)?;
            match value {
                None => {
                    return Err(BinaryError {
                        input: source.to_string(),
                        offset: tokenizer.position,
                        message: format!(
                            "{}:{}: missing value for entity key \"{}\"",
                            tokenizer.line, tokenizer.column, key.value
                        ),
                    });
                }
                Some(value) if value.value == "}" => {
                    return Err(BinaryError {
                        input: source.to_string(),
                        offset: tokenizer.position,
                        message: format!(
                            "{}:{}: missing value for entity key \"{}\"",
                            value.line, value.column, key.value
                        ),
                    });
                }
                Some(value) => {
                    if let Some(slot) = entity.properties.iter_mut().find(|property| property.0 == key.value) {
                        slot.1 = value.value;
                    } else {
                        entity.properties.push((key.value, value.value));
                    }
                }
            }
        }
    }
}

fn rd_i32(bytes: &[u8], offset: usize) -> i32 {
    i32::from_le_bytes([bytes[offset], bytes[offset + 1], bytes[offset + 2], bytes[offset + 3]])
}

fn at_44<'b>(items: &[&'b [u8]], index: i32, source: &str) -> Result<&'b [u8], BinaryError> {
    if index < 0 {
        return Err(BinaryError {
            input: source.to_string(),
            offset: 0,
            message: format!("invalid IBSP44 reference {index}"),
        });
    }
    match items.get(index as usize) {
        Some(record) => Ok(record),
        None => Err(BinaryError {
            input: source.to_string(),
            offset: 0,
            message: format!("invalid IBSP44 reference {index}"),
        }),
    }
}

/// Decode the Q3 test-disk layout into the shared IBSP46 loading contract
/// (`normalizeQ3Bsp`).
///
/// Returns `None` when the input is not IBSP version 44, leaving the
/// caller's bytes in place.
pub fn normalize_q3_bsp(data: &[u8], source: &str) -> Result<Option<Vec<u8>>, BinaryError> {
    if data.len() < 8 {
        return Ok(None);
    }
    let magic = u32::from_le_bytes([data[0], data[1], data[2], data[3]]);
    if magic != Q3_MAGIC_IBSP || rd_i32(data, 4) != Q3_BSP_VERSION_44 {
        return Ok(None);
    }
    let fail = |offset: usize, message: String| BinaryError {
        input: source.to_string(),
        offset,
        message,
    };
    if data.len() < 128 {
        return Err(fail(8, "truncated IBSP44 directory".to_string()));
    }
    let mut sections: Vec<&[u8]> = Vec::with_capacity(15);
    for index in 0..15 {
        let offset = rd_i32(data, 8 + index * 8);
        let length = rd_i32(data, 12 + index * 8);
        if offset < 0
            || length < 0
            || length as usize > data.len()
            || offset as usize > data.len() - length as usize
            || (length > 0 && offset < 128)
        {
            return Err(fail(8 + index * 8, "invalid IBSP44 lump range".to_string()));
        }
        sections.push(&data[offset as usize..offset as usize + length as usize]);
    }
    let section = |index: usize| -> Result<&[u8], BinaryError> {
        sections
            .get(index)
            .copied()
            .ok_or_else(|| fail(8, "missing IBSP44 lump".to_string()))
    };
    let records = |index: usize, stride: usize| -> Result<Vec<&[u8]>, BinaryError> {
        let bytes = section(index)?;
        if !bytes.len().is_multiple_of(stride) {
            return Err(fail(8 + index * 8, format!("invalid IBSP44 record size {stride}")));
        }
        Ok(bytes.chunks(stride).collect())
    };
    let check_range = |first: i32, count: i32, length: usize| -> Result<(), BinaryError> {
        if first < 0 || count < 0 || count as usize > length || first as usize > length - count as usize {
            return Err(fail(0, "invalid IBSP44 record range".to_string()));
        }
        Ok(())
    };
    let planes = records(1, 20)?;
    let nodes = records(2, 36)?;
    let leaves = records(3, 48)?;
    let leaf_surfaces = records(4, 4)?;
    let leaf_brushes = records(5, 4)?;
    let models = records(6, 48)?;
    let brushes = records(7, 12)?;
    let sides = records(8, 8)?;
    let surfaces = records(12, 164)?;
    let fogs = records(13, 68)?;
    let original_indices: Vec<i32> = records(14, 4)?.iter().map(|record| rd_i32(record, 0)).collect();
    let mut indices = original_indices.clone();
    let mut side_names: HashMap<i32, Vec<u8>> = HashMap::new();
    for surface in &surfaces {
        let side = rd_i32(surface, 68);
        if side >= 0 {
            at_44(&sides, side, source)?;
            side_names.insert(side, surface[0..64].to_vec());
        }
    }
    let mut side_contents: HashMap<i32, i32> = HashMap::new();
    for brush in &brushes {
        let first = rd_i32(brush, 0);
        let count = rd_i32(brush, 4);
        check_range(first, count, sides.len())?;
        for side in first..first + count {
            side_contents.insert(side, rd_i32(brush, 8));
        }
    }
    let mut shader_records: Vec<Vec<u8>> = Vec::new();
    let mut shader_ids: HashMap<Vec<u8>, u32> = HashMap::new();
    let mut shader = |name: &[u8], flags: i32, contents: i32| -> u32 {
        let mut key = name.to_vec();
        key.extend_from_slice(&flags.to_le_bytes());
        key.extend_from_slice(&contents.to_le_bytes());
        if let Some(existing) = shader_ids.get(&key) {
            return *existing;
        }
        let mut record = Vec::with_capacity(72);
        record.extend_from_slice(name);
        record.extend_from_slice(&flags.to_le_bytes());
        record.extend_from_slice(&contents.to_le_bytes());
        let id = shader_records.len() as u32;
        shader_records.push(record);
        shader_ids.insert(key, id);
        id
    };
    let empty_name = vec![0u8; 64];
    let mut brush_records: Vec<Vec<u8>> = Vec::with_capacity(brushes.len());
    for brush in &brushes {
        let first = rd_i32(brush, 0);
        let name = side_names.get(&first).map_or(empty_name.as_slice(), Vec::as_slice);
        let id = shader(name, 0, rd_i32(brush, 8));
        let mut record = Vec::with_capacity(12);
        record.extend_from_slice(&first.to_le_bytes());
        record.extend_from_slice(&rd_i32(brush, 4).to_le_bytes());
        record.extend_from_slice(&id.to_le_bytes());
        brush_records.push(record);
    }
    let mut side_records: Vec<Vec<u8>> = Vec::with_capacity(sides.len());
    for (side_index, side) in sides.iter().enumerate() {
        let side_index = side_index as i32;
        let name = side_names.get(&side_index).map_or(empty_name.as_slice(), Vec::as_slice);
        let id = shader(
            name,
            rd_i32(side, 4),
            side_contents.get(&side_index).copied().unwrap_or(0),
        );
        let mut record = Vec::with_capacity(8);
        record.extend_from_slice(&rd_i32(side, 0).to_le_bytes());
        record.extend_from_slice(&id.to_le_bytes());
        side_records.push(record);
    }
    let mut surface_records: Vec<Vec<u8>> = Vec::with_capacity(surfaces.len());
    for surface in &surfaces {
        let side_index = rd_i32(surface, 68);
        let flags = if side_index < 0 {
            0
        } else {
            rd_i32(at_44(&sides, side_index, source)?, 4)
        };
        let patch_width = rd_i32(surface, 88);
        let patch_height = rd_i32(surface, 92);
        let vertex_count = rd_i32(surface, 76);
        let mut first_index = rd_i32(surface, 80);
        let mut index_count = rd_i32(surface, 84);
        check_range(first_index, index_count, original_indices.len())?;
        let surface_type: i32 = if patch_width > 0 && patch_height > 0 {
            2
        } else if index_count > 0 {
            3
        } else {
            1
        };
        if surface_type == 1 {
            if vertex_count < 3 {
                return Err(fail(0, "IBSP44 polygon has fewer than three vertices".to_string()));
            }
            first_index = indices.len() as i32;
            for corner in 1..vertex_count - 1 {
                indices.extend_from_slice(&[0, corner, corner + 1]);
            }
            index_count = indices.len() as i32 - first_index;
        }
        let name = &surface[0..64];
        let id = shader(name, flags, side_contents.get(&side_index).copied().unwrap_or(0));
        let mut record = Vec::with_capacity(104);
        record.extend_from_slice(&id.to_le_bytes());
        record.extend_from_slice(&rd_i32(surface, 64).to_le_bytes());
        record.extend_from_slice(&surface_type.to_le_bytes());
        record.extend_from_slice(&rd_i32(surface, 72).to_le_bytes());
        record.extend_from_slice(&vertex_count.to_le_bytes());
        record.extend_from_slice(&first_index.to_le_bytes());
        record.extend_from_slice(&index_count.to_le_bytes());
        record.extend_from_slice(&surface[96..96 + 68]);
        record.extend_from_slice(&patch_width.to_le_bytes());
        record.extend_from_slice(&patch_height.to_le_bytes());
        surface_records.push(record);
    }
    let mut model_records: Vec<Vec<u8>> = Vec::with_capacity(models.len());
    for model in &models {
        let mut surface_ids = BTreeSet::new();
        let mut brush_ids = BTreeSet::new();
        let mut visited = HashSet::new();
        let mut pending = vec![rd_i32(model, 36)];
        while let Some(node_id) = pending.pop() {
            if !visited.insert(node_id) {
                continue;
            }
            if node_id >= 0 {
                let node = at_44(&nodes, node_id, source)?;
                pending.push(rd_i32(node, 4));
                pending.push(rd_i32(node, 8));
                continue;
            }
            let leaf = at_44(&leaves, -node_id - 1, source)?;
            let first_surface = rd_i32(leaf, 32);
            let surface_count = rd_i32(leaf, 36);
            let first_brush = rd_i32(leaf, 40);
            let brush_count = rd_i32(leaf, 44);
            check_range(first_surface, surface_count, leaf_surfaces.len())?;
            check_range(first_brush, brush_count, leaf_brushes.len())?;
            for member in first_surface..first_surface + surface_count {
                surface_ids.insert(rd_i32(at_44(&leaf_surfaces, member, source)?, 0));
            }
            for member in first_brush..first_brush + brush_count {
                brush_ids.insert(rd_i32(at_44(&leaf_brushes, member, source)?, 0));
            }
        }
        let model_range =
            |ids: &BTreeSet<i32>, output: &mut Vec<Vec<u8>>, originals: usize| -> Result<(i32, i32), BinaryError> {
                let sorted: Vec<i32> = ids.iter().copied().collect();
                for id in &sorted {
                    if *id < 0 || *id as usize >= originals {
                        return Err(fail(0, "invalid IBSP44 model member".to_string()));
                    }
                }
                let first = sorted.first().copied().unwrap_or(0);
                if sorted
                    .iter()
                    .enumerate()
                    .all(|(position, id)| *id == first + position as i32)
                {
                    return Ok((first, sorted.len() as i32));
                }
                let start = output.len() as i32;
                for id in &sorted {
                    let record = output
                        .get(*id as usize)
                        .ok_or_else(|| fail(0, "missing IBSP44 model member".to_string()))?;
                    let record = record.clone();
                    output.push(record);
                }
                Ok((start, sorted.len() as i32))
            };
        let (first_surface, surface_count) = model_range(&surface_ids, &mut surface_records, surfaces.len())?;
        let (first_brush, brush_count) = model_range(&brush_ids, &mut brush_records, brushes.len())?;
        let mut record = Vec::with_capacity(40);
        record.extend_from_slice(&model[0..24]);
        record.extend_from_slice(&first_surface.to_le_bytes());
        record.extend_from_slice(&surface_count.to_le_bytes());
        record.extend_from_slice(&first_brush.to_le_bytes());
        record.extend_from_slice(&brush_count.to_le_bytes());
        model_records.push(record);
    }
    let join = |parts: &[Vec<u8>]| -> Vec<u8> {
        let mut joined = Vec::with_capacity(parts.iter().map(Vec::len).sum());
        for part in parts {
            joined.extend_from_slice(part);
        }
        joined
    };
    let mut index_bytes = Vec::with_capacity(indices.len() * 4);
    for index in &indices {
        index_bytes.extend_from_slice(&index.to_le_bytes());
    }
    let converted_fogs: Vec<Vec<u8>> = fogs
        .iter()
        .map(|fog| {
            let mut record = Vec::with_capacity(72);
            record.extend_from_slice(&fog[0..68]);
            record.extend_from_slice(&(-1i32).to_le_bytes());
            record
        })
        .collect();
    let plane_bytes = join(&planes.iter().map(|plane| plane[0..16].to_vec()).collect::<Vec<_>>());
    let output: Vec<Vec<u8>> = vec![
        section(0)?.to_vec(),
        join(&shader_records),
        plane_bytes,
        section(2)?.to_vec(),
        section(3)?.to_vec(),
        section(4)?.to_vec(),
        section(5)?.to_vec(),
        join(&model_records),
        join(&brush_records),
        join(&side_records),
        section(11)?.to_vec(),
        index_bytes,
        join(&converted_fogs),
        join(&surface_records),
        section(9)?.to_vec(),
        Vec::new(),
        section(10)?.to_vec(),
    ];
    let total: usize = Q3_HEADER_SIZE + output.iter().map(Vec::len).sum::<usize>();
    let mut writer = BinaryWriter::new(total);
    writer.u32(Q3_MAGIC_IBSP)?;
    writer.i32(Q3_BSP_VERSION_46)?;
    let mut offset = Q3_HEADER_SIZE as i32;
    for part in &output {
        writer.i32(offset)?;
        writer.i32(part.len() as i32)?;
        offset += part.len() as i32;
    }
    for part in &output {
        writer.bytes(part)?;
    }
    Ok(Some(writer.finish()))
}

/// Integer bounds for Q3 nodes and leaves.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IntBounds {
    /// Minimum corner.
    pub min: [i32; 3],
    /// Maximum corner.
    pub max: [i32; 3],
}

/// Q3 shader (`BspShader`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Q3Shader {
    /// Shader name.
    pub name: String,
    /// Surface flags.
    pub surface_flags: i32,
    /// Content flags.
    pub content_flags: i32,
}

/// Q3 plane (`BspPlane`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Q3Plane {
    /// Normal.
    pub normal: [f32; 3],
    /// Distance.
    pub distance: f32,
}

/// Q3 node (`BspNode`, with raw child numbers).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Q3Node {
    /// Plane index.
    pub plane: i32,
    /// Children: non-negative nodes, negative leaves (`-leaf - 1`).
    pub children: [i32; 2],
    /// Bounds.
    pub bounds: IntBounds,
}

/// Q3 leaf (`BspLeaf`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Q3Leaf {
    /// Visibility cluster, or -1.
    pub cluster: i32,
    /// Area, or -1.
    pub area: i32,
    /// Bounds.
    pub bounds: IntBounds,
    /// First leaf surface.
    pub first_surface: i32,
    /// Surface count.
    pub surface_count: i32,
    /// First leaf brush.
    pub first_brush: i32,
    /// Brush count.
    pub brush_count: i32,
}

/// Q3 model (`BspModel`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Q3Model {
    /// Bounds.
    pub bounds: Bounds,
    /// First surface.
    pub first_surface: i32,
    /// Surface count.
    pub surface_count: i32,
    /// First brush.
    pub first_brush: i32,
    /// Brush count.
    pub brush_count: i32,
}

/// Q3 brush (`BspBrush`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Q3Brush {
    /// First side.
    pub first_side: i32,
    /// Side count.
    pub side_count: i32,
    /// Shader index.
    pub shader: i32,
}

/// Q3 brush side (`BspBrushSide`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Q3BrushSide {
    /// Plane index.
    pub plane: i32,
    /// Shader index.
    pub shader: i32,
}

/// Q3 vertex (`BspVertex`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Q3Vertex {
    /// Position.
    pub position: [f32; 3],
    /// Texture coordinates.
    pub tex_coord: [f32; 2],
    /// Lightmap coordinates (may be non-finite on unlit patches).
    pub lightmap_coord: [f32; 2],
    /// Normal.
    pub normal: [f32; 3],
    /// Color.
    pub color: [u8; 4],
}

/// Q3 fog (`BspFog`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Q3Fog {
    /// Shader name.
    pub shader: String,
    /// Brush index.
    pub brush: i32,
    /// Visible side, or -1.
    pub visible_side: i32,
}

/// Q3 surface type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Q3SurfaceType {
    /// Planar polygon.
    Planar,
    /// Bezier patch.
    Patch,
    /// Triangle soup.
    Triangles,
    /// Flare.
    Flare,
}

/// Q3 surface (`BspSurface`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Q3Surface {
    /// Surface type.
    pub surface_type: Q3SurfaceType,
    /// Shader index.
    pub shader: i32,
    /// Fog index, or -1.
    pub fog: i32,
    /// First vertex.
    pub first_vertex: i32,
    /// Vertex count.
    pub vertex_count: i32,
    /// First index.
    pub first_index: i32,
    /// Index count.
    pub index_count: i32,
    /// Lightmap image, or a negative sentinel.
    pub lightmap: i32,
    /// Lightmap origin rectangle.
    pub lightmap_rect: [i32; 4],
    /// Lightmap origin.
    pub lightmap_origin: [f32; 3],
    /// Lightmap vectors.
    pub lightmap_vectors: [[f32; 3]; 3],
    /// Patch width (patches only).
    pub patch_width: i32,
    /// Patch height (patches only).
    pub patch_height: i32,
}

/// Q3 light-grid point (`BspLightGridPoint`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Q3LightGridPoint {
    /// Ambient color.
    pub ambient: [u8; 3],
    /// Directed color.
    pub directed: [u8; 3],
    /// Latitude/longitude.
    pub lat_long: [u8; 2],
}

/// Q3 visibility (`BspVisibility`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Q3Visibility {
    /// Cluster count.
    pub cluster_count: i32,
    /// Bytes per cluster.
    pub bytes_per_cluster: i32,
    /// Visibility bits.
    pub bits: Vec<u8>,
}

/// Parsed Q3 map (`BspMap`): stored values exactly as loaded.
#[derive(Debug, Clone, PartialEq)]
pub struct Q3Map {
    /// Entity text.
    pub entities: String,
    /// Parsed entity records.
    pub entity_records: Vec<Q3Entity>,
    /// Shaders.
    pub shaders: Vec<Q3Shader>,
    /// Planes.
    pub planes: Vec<Q3Plane>,
    /// Nodes.
    pub nodes: Vec<Q3Node>,
    /// Leaves.
    pub leaves: Vec<Q3Leaf>,
    /// Leaf surfaces.
    pub leaf_surfaces: Vec<i32>,
    /// Leaf brushes.
    pub leaf_brushes: Vec<i32>,
    /// Models.
    pub models: Vec<Q3Model>,
    /// Brushes.
    pub brushes: Vec<Q3Brush>,
    /// Brush sides.
    pub brush_sides: Vec<Q3BrushSide>,
    /// Vertices.
    pub vertices: Vec<Q3Vertex>,
    /// Indices (local to each surface's vertex range).
    pub indices: Vec<i32>,
    /// Fogs.
    pub fogs: Vec<Q3Fog>,
    /// Surfaces.
    pub surfaces: Vec<Q3Surface>,
    /// Lightmaps.
    pub lightmaps: Vec<Vec<u8>>,
    /// Light grid.
    pub light_grid: Vec<Q3LightGridPoint>,
    /// Visibility, or `None` when the lump is empty.
    pub visibility: Option<Q3Visibility>,
}

fn q3_vec2(reader: &mut BinaryReader<'_>) -> Result<[f32; 2], BinaryError> {
    Ok([reader.finite_f32()?, reader.finite_f32()?])
}

fn q3_vec3(reader: &mut BinaryReader<'_>) -> Result<[f32; 3], BinaryError> {
    Ok([reader.finite_f32()?, reader.finite_f32()?, reader.finite_f32()?])
}

fn q3_int_vec3(reader: &mut BinaryReader<'_>) -> Result<[i32; 3], BinaryError> {
    Ok([reader.i32()?, reader.i32()?, reader.i32()?])
}

/// Parse the stored map data (`parseQ3Bsp`).
///
/// Renderer overbright shifts and collision bounds expansion are separate
/// operations.
pub fn parse_q3_bsp(data: &[u8], source: &str) -> Result<Q3Map, BinaryError> {
    let normalized = normalize_q3_bsp(data, source)?;
    let bytes: &[u8] = normalized.as_deref().unwrap_or(data);
    let mut reader = BinaryReader::new(bytes, source);
    if reader.u32()? != Q3_MAGIC_IBSP {
        return Err(BinaryError {
            input: source.to_string(),
            offset: 0,
            message: "expected IBSP magic".to_string(),
        });
    }
    if reader.i32()? != Q3_BSP_VERSION_46 {
        return Err(BinaryError {
            input: source.to_string(),
            offset: 4,
            message: "expected BSP version 46".to_string(),
        });
    }
    let mut lumps = Vec::with_capacity(17);
    for index in 0..17 {
        let offset = reader.i32()?;
        let length = reader.i32()?;
        if offset < 0
            || length < 0
            || length as usize > bytes.len()
            || offset as usize > bytes.len() - length as usize
            || (length > 0 && offset < Q3_HEADER_SIZE as i32)
        {
            return Err(BinaryError {
                input: source.to_string(),
                offset: 8 + index * 8,
                message: format!("invalid lump {index} range {offset}+{length}"),
            });
        }
        lumps.push((offset as usize, length as usize));
    }
    let lump = |index: usize| lumps[index];
    let mut entities_reader = BinaryReader::new(bytes, source);
    entities_reader.seek(lump(0).0)?;
    let entities = entities_reader.fixed_byte_string(lump(0).1)?;
    let mut read_records = |index: usize,
                            stride: usize,
                            read: &mut dyn FnMut(&mut BinaryReader<'_>) -> Result<(), BinaryError>|
     -> Result<(), BinaryError> {
        let (offset, length) = lump(index);
        if !length.is_multiple_of(stride) {
            return Err(BinaryError {
                input: source.to_string(),
                offset,
                message: format!("lump {index} length {length} is not a multiple of {stride}"),
            });
        }
        reader.seek(offset)?;
        for _ in 0..length / stride {
            read(&mut reader)?;
        }
        Ok(())
    };
    let mut shaders = Vec::new();
    read_records(1, 72, &mut |row| {
        shaders.push(Q3Shader {
            name: row.fixed_byte_string(64)?,
            surface_flags: row.i32()?,
            content_flags: row.i32()?,
        });
        Ok(())
    })?;
    let mut planes = Vec::new();
    read_records(2, 16, &mut |row| {
        planes.push(Q3Plane {
            normal: q3_vec3(row)?,
            distance: row.finite_f32()?,
        });
        Ok(())
    })?;
    let mut nodes = Vec::new();
    read_records(3, 36, &mut |row| {
        nodes.push(Q3Node {
            plane: row.i32()?,
            children: [row.i32()?, row.i32()?],
            bounds: IntBounds {
                min: q3_int_vec3(row)?,
                max: q3_int_vec3(row)?,
            },
        });
        Ok(())
    })?;
    let mut leaves = Vec::new();
    read_records(4, 48, &mut |row| {
        leaves.push(Q3Leaf {
            cluster: row.i32()?,
            area: row.i32()?,
            bounds: IntBounds {
                min: q3_int_vec3(row)?,
                max: q3_int_vec3(row)?,
            },
            first_surface: row.i32()?,
            surface_count: row.i32()?,
            first_brush: row.i32()?,
            brush_count: row.i32()?,
        });
        Ok(())
    })?;
    let mut leaf_surfaces = Vec::new();
    read_records(5, 4, &mut |row| {
        leaf_surfaces.push(row.i32()?);
        Ok(())
    })?;
    let mut leaf_brushes = Vec::new();
    read_records(6, 4, &mut |row| {
        leaf_brushes.push(row.i32()?);
        Ok(())
    })?;
    let mut models = Vec::new();
    read_records(7, 40, &mut |row| {
        models.push(Q3Model {
            bounds: Bounds {
                min: q3_vec3(row)?,
                max: q3_vec3(row)?,
            },
            first_surface: row.i32()?,
            surface_count: row.i32()?,
            first_brush: row.i32()?,
            brush_count: row.i32()?,
        });
        Ok(())
    })?;
    let mut brushes = Vec::new();
    read_records(8, 12, &mut |row| {
        brushes.push(Q3Brush {
            first_side: row.i32()?,
            side_count: row.i32()?,
            shader: row.i32()?,
        });
        Ok(())
    })?;
    let mut brush_sides = Vec::new();
    read_records(9, 8, &mut |row| {
        brush_sides.push(Q3BrushSide {
            plane: row.i32()?,
            shader: row.i32()?,
        });
        Ok(())
    })?;
    let mut vertices = Vec::new();
    read_records(10, 44, &mut |row| {
        vertices.push(Q3Vertex {
            position: q3_vec3(row)?,
            tex_coord: q3_vec2(row)?,
            lightmap_coord: [row.f32()?, row.f32()?],
            normal: q3_vec3(row)?,
            color: [row.u8()?, row.u8()?, row.u8()?, row.u8()?],
        });
        Ok(())
    })?;
    let mut indices = Vec::new();
    read_records(11, 4, &mut |row| {
        indices.push(row.i32()?);
        Ok(())
    })?;
    let mut fogs = Vec::new();
    read_records(12, 72, &mut |row| {
        fogs.push(Q3Fog {
            shader: row.fixed_byte_string(64)?,
            brush: row.i32()?,
            visible_side: row.i32()?,
        });
        Ok(())
    })?;
    let mut surfaces = Vec::new();
    let surface_source = source.to_string();
    read_records(13, 104, &mut |row| {
        surfaces.push(read_q3_surface(row, &surface_source)?);
        Ok(())
    })?;
    let mut lightmaps = Vec::new();
    read_records(14, Q3_LIGHTMAP_BYTES, &mut |row| {
        lightmaps.push(row.bytes(Q3_LIGHTMAP_BYTES)?);
        Ok(())
    })?;
    let mut light_grid = Vec::new();
    read_records(15, 8, &mut |row| {
        light_grid.push(Q3LightGridPoint {
            ambient: [row.u8()?, row.u8()?, row.u8()?],
            directed: [row.u8()?, row.u8()?, row.u8()?],
            lat_long: [row.u8()?, row.u8()?],
        });
        Ok(())
    })?;
    let visibility_lump = lump(16);
    let mut visibility = None;
    if visibility_lump.1 != 0 {
        reader.seek(visibility_lump.0)?;
        if visibility_lump.1 < 8 {
            return Err(BinaryError {
                input: source.to_string(),
                offset: reader.offset(),
                message: "truncated visibility header".to_string(),
            });
        }
        let cluster_count = reader.i32()?;
        let bytes_per_cluster = reader.i32()?;
        if cluster_count < 0
            || i64::from(bytes_per_cluster) < (i64::from(cluster_count) + 7) / 8
            || cluster_count as usize * bytes_per_cluster as usize != visibility_lump.1 - 8
        {
            return Err(BinaryError {
                input: source.to_string(),
                offset: visibility_lump.0,
                message: "invalid visibility dimensions".to_string(),
            });
        }
        visibility = Some(Q3Visibility {
            cluster_count,
            bytes_per_cluster,
            bits: reader.bytes(visibility_lump.1 - 8)?,
        });
    }
    let map = Q3Map {
        entities,
        entity_records: Vec::new(),
        shaders,
        planes,
        nodes,
        leaves,
        leaf_surfaces,
        leaf_brushes,
        models,
        brushes,
        brush_sides,
        vertices,
        indices,
        fogs,
        surfaces,
        lightmaps,
        light_grid,
        visibility,
    };
    validate_q3_bsp(&map, &lumps, source)?;
    let entity_records = parse_q3_entities(&map.entities, &format!("{source}:entities"))?;
    Ok(Q3Map { entity_records, ..map })
}

fn read_q3_surface(reader: &mut BinaryReader<'_>, source: &str) -> Result<Q3Surface, BinaryError> {
    let shader = reader.i32()?;
    let fog = reader.i32()?;
    let type_offset = reader.offset();
    let surface_type = reader.i32()?;
    let surface = Q3Surface {
        surface_type: Q3SurfaceType::Planar,
        shader,
        fog,
        first_vertex: reader.i32()?,
        vertex_count: reader.i32()?,
        first_index: reader.i32()?,
        index_count: reader.i32()?,
        lightmap: reader.i32()?,
        lightmap_rect: [reader.i32()?, reader.i32()?, reader.i32()?, reader.i32()?],
        lightmap_origin: q3_vec3(reader)?,
        lightmap_vectors: [q3_vec3(reader)?, q3_vec3(reader)?, q3_vec3(reader)?],
        patch_width: reader.i32()?,
        patch_height: reader.i32()?,
    };
    let surface_type = match surface_type {
        1 => Q3SurfaceType::Planar,
        2 => Q3SurfaceType::Patch,
        3 => Q3SurfaceType::Triangles,
        4 => Q3SurfaceType::Flare,
        _ => {
            return Err(BinaryError {
                input: source.to_string(),
                offset: type_offset,
                message: format!("unknown BSP surface type {surface_type}"),
            });
        }
    };
    Ok(Q3Surface {
        surface_type,
        ..surface
    })
}

fn validate_q3_bsp(map: &Q3Map, lumps: &[(usize, usize)], source: &str) -> Result<(), BinaryError> {
    let fail = |offset: usize, message: String| BinaryError {
        input: source.to_string(),
        offset,
        message,
    };
    let reference = |value: i32, count: usize, offset: usize, name: &str| -> Result<(), BinaryError> {
        if value < 0 || value as usize >= count {
            return Err(fail(
                offset,
                format!("{name} index {value} outside 0..{}", count as i64 - 1),
            ));
        }
        Ok(())
    };
    let range = |first: i32, count: i32, total: usize, offset: usize, name: &str| -> Result<(), BinaryError> {
        if first < 0 || count < 0 || count as usize > total || first as usize > total - count as usize {
            return Err(fail(offset, format!("{name} range {first}+{count} exceeds {total}")));
        }
        Ok(())
    };
    for (index, node) in map.nodes.iter().enumerate() {
        let offset = lumps[3].0 + index * 36;
        reference(node.plane, map.planes.len(), offset, "node plane")?;
        for (side, child) in node.children.iter().enumerate() {
            let (target, count) = if *child < 0 {
                (-(i64::from(*child)) - 1, map.leaves.len())
            } else {
                (i64::from(*child), map.nodes.len())
            };
            if target < 0 || target >= count as i64 {
                return Err(fail(
                    offset + 4 + side * 4,
                    format!("node child index {target} outside 0..{}", count as i64 - 1),
                ));
            }
        }
    }
    // Iterative depth-first traversal checks disconnected nodes too, without a stack-depth limit.
    let mut visited = vec![0u8; map.nodes.len()];
    let mut stack: Vec<(usize, bool)> = Vec::new();
    for root in 0..map.nodes.len() {
        if visited[root] == 2 {
            continue;
        }
        stack.push((root, false));
        while let Some((current, exiting)) = stack.pop() {
            if exiting {
                visited[current] = 2;
                continue;
            }
            if visited[current] == 1 {
                return Err(fail(lumps[3].0 + current * 36, "cycle in BSP nodes".to_string()));
            }
            if visited[current] == 2 {
                continue;
            }
            visited[current] = 1;
            let Some(node) = map.nodes.get(current) else {
                return Err(fail(lumps[3].0, "missing BSP node".to_string()));
            };
            stack.push((current, true));
            for child in node.children {
                if child >= 0 {
                    stack.push((child as usize, false));
                }
            }
        }
    }
    for (index, leaf) in map.leaves.iter().enumerate() {
        let offset = lumps[4].0 + index * 48;
        if leaf.cluster < -1
            || map
                .visibility
                .as_ref()
                .is_some_and(|vis| leaf.cluster >= vis.cluster_count)
        {
            return Err(fail(offset, "invalid leaf cluster".to_string()));
        }
        if leaf.area < -1 {
            return Err(fail(offset + 4, "invalid leaf area".to_string()));
        }
        range(
            leaf.first_surface,
            leaf.surface_count,
            map.leaf_surfaces.len(),
            offset + 32,
            "leaf surfaces",
        )?;
        range(
            leaf.first_brush,
            leaf.brush_count,
            map.leaf_brushes.len(),
            offset + 40,
            "leaf brushes",
        )?;
    }
    for (index, value) in map.leaf_surfaces.iter().enumerate() {
        reference(*value, map.surfaces.len(), lumps[5].0 + index * 4, "leaf surface")?;
    }
    for (index, value) in map.leaf_brushes.iter().enumerate() {
        reference(*value, map.brushes.len(), lumps[6].0 + index * 4, "leaf brush")?;
    }
    for (index, model) in map.models.iter().enumerate() {
        let offset = lumps[7].0 + index * 40;
        range(
            model.first_surface,
            model.surface_count,
            map.surfaces.len(),
            offset + 24,
            "model surfaces",
        )?;
        range(
            model.first_brush,
            model.brush_count,
            map.brushes.len(),
            offset + 32,
            "model brushes",
        )?;
    }
    for (index, brush) in map.brushes.iter().enumerate() {
        let offset = lumps[8].0 + index * 12;
        range(
            brush.first_side,
            brush.side_count,
            map.brush_sides.len(),
            offset,
            "brush sides",
        )?;
        reference(brush.shader, map.shaders.len(), offset + 8, "brush shader")?;
    }
    for (index, side) in map.brush_sides.iter().enumerate() {
        reference(side.plane, map.planes.len(), lumps[9].0 + index * 8, "brush side plane")?;
        reference(
            side.shader,
            map.shaders.len(),
            lumps[9].0 + index * 8 + 4,
            "brush side shader",
        )?;
    }
    for (index, fog) in map.fogs.iter().enumerate() {
        let offset = lumps[12].0 + index * 72;
        reference(fog.brush, map.brushes.len(), offset + 64, "fog brush")?;
        let Some(brush) = map.brushes.get(fog.brush as usize) else {
            return Err(fail(offset + 64, "missing fog brush".to_string()));
        };
        if fog.visible_side != -1 {
            reference(
                fog.visible_side,
                brush.side_count as usize,
                offset + 68,
                "fog visible side",
            )?;
        }
    }
    let mut lightmap_use = vec![0u8; map.vertices.len()];
    for (index, item) in map.surfaces.iter().enumerate() {
        let offset = lumps[13].0 + index * 104;
        reference(item.shader, map.shaders.len(), offset, "surface shader")?;
        // Retail mpteam1 stores zero in 277 flare fog fields despite an empty fog lump.
        let retail_flare_fog = item.surface_type == Q3SurfaceType::Flare && item.fog == 0 && map.fogs.is_empty();
        if item.fog != -1 && !retail_flare_fog {
            reference(item.fog, map.fogs.len(), offset + 4, "surface fog")?;
        }
        range(
            item.first_vertex,
            item.vertex_count,
            map.vertices.len(),
            offset + 12,
            "surface vertices",
        )?;
        range(
            item.first_index,
            item.index_count,
            map.indices.len(),
            offset + 20,
            "surface indices",
        )?;
        // R_FindShader falls back to vertex lighting for missing lightmaps, including retail texturegrab.
        let uses_lightmap = matches!(item.surface_type, Q3SurfaceType::Planar | Q3SurfaceType::Patch)
            && item.lightmap >= 0
            && (item.lightmap as usize) < map.lightmaps.len();
        if matches!(item.surface_type, Q3SurfaceType::Planar | Q3SurfaceType::Patch) && item.lightmap < -4 {
            return Err(fail(offset + 28, "invalid lightmap sentinel".to_string()));
        }
        for vertex in item.first_vertex..item.first_vertex + item.vertex_count {
            let slot = &mut lightmap_use[vertex as usize];
            if uses_lightmap {
                *slot = 2;
            } else if *slot != 2 {
                *slot = 1;
            }
        }
        if item.surface_type == Q3SurfaceType::Patch
            && (item.patch_width < 3
                || item.patch_height < 3
                || item.patch_width % 2 != 1
                || item.patch_height % 2 != 1
                || i64::from(item.patch_width) * i64::from(item.patch_height) != i64::from(item.vertex_count))
        {
            return Err(fail(offset + 96, "invalid patch control grid".to_string()));
        }
        if matches!(item.surface_type, Q3SurfaceType::Planar | Q3SurfaceType::Triangles) && item.index_count % 3 != 0 {
            return Err(fail(
                offset + 24,
                "triangle index count is not a multiple of 3".to_string(),
            ));
        }
        for local in item.first_index..item.first_index + item.index_count {
            let Some(value) = map.indices.get(local as usize) else {
                return Err(fail(
                    lumps[11].0 + local as usize * 4,
                    "missing surface index".to_string(),
                ));
            };
            reference(
                *value,
                item.vertex_count as usize,
                lumps[11].0 + local as usize * 4,
                "surface local vertex",
            )?;
        }
    }
    // mpterra3 has NaN lightmap UVs on patches with lightmap=-1. Preserve unused
    // bytes, while rejecting non-finite coordinates used for actual lightmap sampling.
    for (index, vertex) in map.vertices.iter().enumerate() {
        if lightmap_use[index] == 1 {
            continue;
        }
        if !vertex.lightmap_coord[0].is_finite() {
            return Err(fail(
                lumps[10].0 + index * 44 + 20,
                "non-finite lightmap coordinate".to_string(),
            ));
        }
        if !vertex.lightmap_coord[1].is_finite() {
            return Err(fail(
                lumps[10].0 + index * 44 + 24,
                "non-finite lightmap coordinate".to_string(),
            ));
        }
    }
    Ok(())
}

/// Q3 world plane with derived type and sign bits.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Q3WorldPlane {
    /// Normal.
    pub normal: [f32; 3],
    /// Distance.
    pub distance: f32,
    /// Axial type (0/1/2) or 3 for non-axial.
    pub plane_type: i32,
    /// Sign bits.
    pub signbits: u8,
}

/// Q3 world node (without face ranges).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Q3WorldNode {
    /// Plane index.
    pub plane: i32,
    /// Children.
    pub children: [NodeChild; 2],
    /// Bounds.
    pub bounds: IntBounds,
}

/// Q3 world leaf.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Q3WorldLeaf {
    /// Visibility cluster.
    pub cluster: i32,
    /// Area.
    pub area: i32,
    /// Bounds.
    pub bounds: IntBounds,
    /// Surface range.
    pub surfaces: IndexRange,
    /// Brush range.
    pub brushes: IndexRange,
}

/// Q3 world model.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Q3WorldModel {
    /// Bounds.
    pub bounds: Bounds,
    /// Surface range.
    pub surfaces: IndexRange,
    /// Brush range.
    pub brushes: IndexRange,
}

/// Q3 world brush.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Q3WorldBrush {
    /// Side range.
    pub sides: IndexRange,
    /// Shader index.
    pub shader: i32,
}

/// Q3 surface lightmap reference.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Q3SurfaceLightmap {
    /// Lightmap image.
    pub image: i32,
    /// Rectangle origin and size.
    pub rect: [i32; 4],
    /// Lightmap origin.
    pub origin: [f32; 3],
    /// Lightmap vectors.
    pub vectors: [[f32; 3]; 3],
}

/// Q3 world surface kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Q3WorldSurfaceKind {
    /// Planar polygon.
    Planar,
    /// Triangle soup.
    Triangles,
    /// Flare.
    Flare,
    /// Bezier patch with control dimensions.
    Patch {
        /// Control width.
        width: i32,
        /// Control height.
        height: i32,
    },
}

/// Q3 world surface.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Q3WorldSurface {
    /// Surface kind.
    pub kind: Q3WorldSurfaceKind,
    /// Shader index.
    pub shader: i32,
    /// Fog index.
    pub fog: i32,
    /// Vertex range.
    pub vertices: IndexRange,
    /// Index range (local to the vertex range).
    pub indices: IndexRange,
    /// Lightmap reference.
    pub lightmap: Q3SurfaceLightmap,
}

/// Adapted Q3 world geometry (`Q3WorldGeometry`).
///
/// Source units, local draw indices, byte RGBA, and patch LOD bounds are
/// unchanged. Shared arrays (shaders, vertices, lightmaps, visibility)
/// stay on [`Q3DecodedWorld::map`].
#[derive(Debug, Clone, PartialEq)]
pub struct Q3DecodedWorld {
    /// Source records.
    pub map: Q3Map,
    /// Planes with derived type and sign bits.
    pub planes: Vec<Q3WorldPlane>,
    /// Nodes with decoded children.
    pub nodes: Vec<Q3WorldNode>,
    /// Leaves with surface/brush ranges.
    pub leaves: Vec<Q3WorldLeaf>,
    /// Models with surface/brush ranges.
    pub models: Vec<Q3WorldModel>,
    /// Brushes with side ranges.
    pub brushes: Vec<Q3WorldBrush>,
    /// Surfaces with decoded kinds.
    pub surfaces: Vec<Q3WorldSurface>,
}

/// Adapt stored records to world geometry (`adaptQ3Bsp`).
#[must_use]
pub fn adapt_q3_bsp(map: Q3Map) -> Q3DecodedWorld {
    fn q3_child(value: i32) -> NodeChild {
        if value < 0 {
            NodeChild::Leaf((-value - 1) as u32)
        } else {
            NodeChild::Node(value as u32)
        }
    }
    let planes = map
        .planes
        .iter()
        .map(|plane| {
            let normal = plane.normal;
            let plane_type = if normal[0] == 1.0 {
                0
            } else if normal[1] == 1.0 {
                1
            } else if normal[2] == 1.0 {
                2
            } else {
                3
            };
            Q3WorldPlane {
                normal,
                distance: plane.distance,
                plane_type,
                signbits: u8::from(normal[0] < 0.0) | u8::from(normal[1] < 0.0) << 1 | u8::from(normal[2] < 0.0) << 2,
            }
        })
        .collect();
    let nodes = map
        .nodes
        .iter()
        .map(|node| Q3WorldNode {
            plane: node.plane,
            children: [q3_child(node.children[0]), q3_child(node.children[1])],
            bounds: node.bounds,
        })
        .collect();
    let range = |first: i32, count: i32| IndexRange {
        first: first as u32,
        count: count as u32,
    };
    let leaves = map
        .leaves
        .iter()
        .map(|leaf| Q3WorldLeaf {
            cluster: leaf.cluster,
            area: leaf.area,
            bounds: leaf.bounds,
            surfaces: range(leaf.first_surface, leaf.surface_count),
            brushes: range(leaf.first_brush, leaf.brush_count),
        })
        .collect();
    let models = map
        .models
        .iter()
        .map(|model| Q3WorldModel {
            bounds: model.bounds,
            surfaces: range(model.first_surface, model.surface_count),
            brushes: range(model.first_brush, model.brush_count),
        })
        .collect();
    let brushes = map
        .brushes
        .iter()
        .map(|brush| Q3WorldBrush {
            sides: range(brush.first_side, brush.side_count),
            shader: brush.shader,
        })
        .collect();
    let surfaces = map
        .surfaces
        .iter()
        .map(|surface| Q3WorldSurface {
            kind: match surface.surface_type {
                Q3SurfaceType::Planar => Q3WorldSurfaceKind::Planar,
                Q3SurfaceType::Triangles => Q3WorldSurfaceKind::Triangles,
                Q3SurfaceType::Flare => Q3WorldSurfaceKind::Flare,
                Q3SurfaceType::Patch => Q3WorldSurfaceKind::Patch {
                    width: surface.patch_width,
                    height: surface.patch_height,
                },
            },
            shader: surface.shader,
            fog: surface.fog,
            vertices: range(surface.first_vertex, surface.vertex_count),
            indices: range(surface.first_index, surface.index_count),
            lightmap: Q3SurfaceLightmap {
                image: surface.lightmap,
                rect: surface.lightmap_rect,
                origin: surface.lightmap_origin,
                vectors: surface.lightmap_vectors,
            },
        })
        .collect();
    Q3DecodedWorld {
        map,
        planes,
        nodes,
        leaves,
        models,
        brushes,
        surfaces,
    }
}

/// Parse a Q3 map and adapt it to world geometry (`decodeQ3World`).
pub fn decode_q3_world(data: &[u8], source: &str) -> Result<Q3DecodedWorld, BinaryError> {
    Ok(adapt_q3_bsp(parse_q3_bsp(data, source)?))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_vertex(writer: &mut BinaryWriter, position: [f32; 3]) {
        for component in position {
            writer.f32(component).unwrap();
        }
        writer.f32(0.25).unwrap();
        writer.f32(0.75).unwrap();
        writer.f32(0.125).unwrap();
        writer.f32(0.875).unwrap();
        for component in [0.0f32, 0.0, 1.0] {
            writer.f32(component).unwrap();
        }
        writer.bytes(&[12, 34, 56, 255]).unwrap();
    }

    fn fixture_46() -> Vec<u8> {
        let mut lumps: Vec<Vec<u8>> = vec![Vec::new(); 17];
        lumps[0] = b"{ \"classname\" \"worldspawn\" \"message\" \"BSP test\" }\n".to_vec();
        let mut writer = BinaryWriter::new(72);
        let mut name = [0u8; 64];
        name[..18].copy_from_slice(b"textures/test/wall");
        writer.bytes(&name).unwrap();
        writer.i32(128).unwrap();
        writer.i32(1).unwrap();
        lumps[1] = writer.finish();
        let mut writer = BinaryWriter::new(16);
        for value in [1.0f32, 0.0, 0.0, 16.0] {
            writer.f32(value).unwrap();
        }
        lumps[2] = writer.finish();
        let mut writer = BinaryWriter::new(36);
        writer.i32(0).unwrap();
        writer.i32(-1).unwrap();
        writer.i32(-1).unwrap();
        for value in [-32i32, -32, -32, 32, 32, 32] {
            writer.i32(value).unwrap();
        }
        lumps[3] = writer.finish();
        let mut writer = BinaryWriter::new(48);
        writer.i32(0).unwrap();
        writer.i32(0).unwrap();
        for value in [-32i32, -32, -32, 32, 32, 32] {
            writer.i32(value).unwrap();
        }
        writer.i32(0).unwrap();
        writer.i32(1).unwrap();
        writer.i32(0).unwrap();
        writer.i32(1).unwrap();
        lumps[4] = writer.finish();
        let mut writer = BinaryWriter::new(4);
        writer.i32(0).unwrap();
        lumps[5] = writer.finish();
        let mut writer = BinaryWriter::new(4);
        writer.i32(0).unwrap();
        lumps[6] = writer.finish();
        let mut writer = BinaryWriter::new(40);
        for value in [0.0f32, 0.0, 0.0, 32.0, 32.0, 32.0] {
            writer.f32(value).unwrap();
        }
        writer.i32(0).unwrap();
        writer.i32(1).unwrap();
        writer.i32(0).unwrap();
        writer.i32(1).unwrap();
        lumps[7] = writer.finish();
        let mut writer = BinaryWriter::new(12);
        writer.i32(0).unwrap();
        writer.i32(1).unwrap();
        writer.i32(0).unwrap();
        lumps[8] = writer.finish();
        let mut writer = BinaryWriter::new(8);
        writer.i32(0).unwrap();
        writer.i32(0).unwrap();
        lumps[9] = writer.finish();
        let mut writer = BinaryWriter::new(132);
        write_vertex(&mut writer, [0.0, 0.0, 0.0]);
        write_vertex(&mut writer, [1.0, 1.5, -1.0]);
        write_vertex(&mut writer, [2.0, 0.0, 0.0]);
        lumps[10] = writer.finish();
        let mut writer = BinaryWriter::new(12);
        for index in [0i32, 1, 2] {
            writer.i32(index).unwrap();
        }
        lumps[11] = writer.finish();
        let mut writer = BinaryWriter::new(104);
        writer.i32(0).unwrap();
        writer.i32(-1).unwrap();
        writer.i32(1).unwrap();
        writer.i32(0).unwrap();
        writer.i32(3).unwrap();
        writer.i32(0).unwrap();
        writer.i32(3).unwrap();
        writer.i32(0).unwrap();
        for value in [0i32, 0, 128, 128] {
            writer.i32(value).unwrap();
        }
        for value in [0.5f32, 1.5, 2.5, 3.5, 4.5, 5.5, 6.5, 7.5, 8.5, 9.5, 10.5, 11.5] {
            writer.f32(value).unwrap();
        }
        writer.i32(0).unwrap();
        writer.i32(0).unwrap();
        lumps[13] = writer.finish();
        lumps[14] = vec![127u8; Q3_LIGHTMAP_BYTES];
        let mut writer = BinaryWriter::new(8);
        writer.bytes(&[1, 2, 3, 4, 5, 6, 7, 8]).unwrap();
        lumps[15] = writer.finish();
        let mut writer = BinaryWriter::new(9);
        writer.i32(1).unwrap();
        writer.i32(1).unwrap();
        writer.u8(1).unwrap();
        lumps[16] = writer.finish();
        let mut file = BinaryWriter::new(65536);
        file.u32(Q3_MAGIC_IBSP).unwrap();
        file.i32(Q3_BSP_VERSION_46).unwrap();
        let mut offset = Q3_HEADER_SIZE as i32;
        for lump in &lumps {
            file.i32(offset).unwrap();
            file.i32(lump.len() as i32).unwrap();
            offset += lump.len() as i32;
        }
        for lump in &lumps {
            file.bytes(lump).unwrap();
        }
        file.finish()
    }

    #[test]
    fn q3_v46_round_trip() {
        let map = parse_q3_bsp(&fixture_46(), "<test>").unwrap();
        assert_eq!(map.entity_records[0].get("classname"), Some("worldspawn"));
        assert!(map.entities.contains("\"message\" \"BSP test\""));
        assert_eq!(map.shaders.len(), 1);
        assert_eq!(map.shaders[0].name, "textures/test/wall");
        assert_eq!((map.shaders[0].surface_flags, map.shaders[0].content_flags), (128, 1));
        assert_eq!(map.planes[0].normal, [1.0, 0.0, 0.0]);
        assert_eq!(map.nodes[0].children, [-1, -1]);
        assert_eq!(map.leaves[0].cluster, 0);
        assert_eq!(map.leaf_surfaces, vec![0]);
        assert_eq!(map.leaf_brushes, vec![0]);
        assert_eq!(map.vertices[1].position, [1.0, 1.5, -1.0]);
        assert_eq!(map.vertices[1].color, [12, 34, 56, 255]);
        assert_eq!(map.indices, vec![0, 1, 2]);
        assert_eq!(map.surfaces.len(), 1);
        assert_eq!(map.surfaces[0].surface_type, Q3SurfaceType::Planar);
        assert_eq!(map.lightmaps.len(), 1);
        assert_eq!(map.lightmaps[0].len(), Q3_LIGHTMAP_BYTES);
        assert_eq!(map.light_grid[0].ambient, [1, 2, 3]);
        let visibility = map.visibility.as_ref().unwrap();
        assert_eq!((visibility.cluster_count, visibility.bytes_per_cluster), (1, 1));
        assert_eq!(visibility.bits, vec![1]);

        let world = adapt_q3_bsp(map);
        assert_eq!(world.planes[0].plane_type, 0);
        assert_eq!(world.planes[0].signbits, 0);
        assert_eq!(world.nodes[0].children, [NodeChild::Leaf(0), NodeChild::Leaf(0)]);
        assert_eq!(world.models[0].surfaces, IndexRange { first: 0, count: 1 });
        assert_eq!(world.brushes[0].sides, IndexRange { first: 0, count: 1 });
        assert_eq!(world.surfaces[0].kind, Q3WorldSurfaceKind::Planar);
        assert_eq!(world.map.vertices.len(), 3);
    }

    #[test]
    fn q3_entities_keep_contract() {
        let records = parse_q3_entities(
            "// header\n{ \"classname\" \"worldspawn\" /* comment */ \"key\" \"old\" \"key\" \"new\" \"message\" \"caf\u{e9}\\nnext\" }",
            "<test>",
        )
        .unwrap();
        assert_eq!(records[0].get("classname"), Some("worldspawn"));
        assert_eq!(records[0].get("key"), Some("new"));
        assert_eq!(records[0].get("message"), Some("caf\u{e9}\\nnext"));
        let error = parse_q3_entities("{ \"key\" }", "<test>").unwrap_err();
        assert!(error.message.contains("missing value"), "{}", error.message);
        let error = parse_q3_entities("\"x\"", "<test>").unwrap_err();
        assert!(error.message.contains("expected \"{\""), "{}", error.message);
        let long = format!("{{ \"key\" \"{}\" }}", "a".repeat(1024));
        let error = parse_q3_entities(&long, "<test>").unwrap_err();
        assert!(
            error.message.contains("token is limited to 1023 characters"),
            "{}",
            error.message
        );
    }

    #[test]
    fn q3_rejects_bad_input() {
        let good = fixture_46();
        assert!(parse_q3_bsp(&good[..100], "<test>").is_err());
        assert!(parse_q3_bsp(&good[..good.len() - 1], "<test>").is_err());
        let mut bad_version = good.clone();
        bad_version[4] = 45;
        let error = parse_q3_bsp(&bad_version, "<test>").unwrap_err();
        assert!(error.message.contains("version 46"), "{}", error.message);
        let mut bad_lump = good.clone();
        bad_lump[8..12].copy_from_slice(&(-1i32).to_le_bytes());
        let error = parse_q3_bsp(&bad_lump, "<test>").unwrap_err();
        assert!(error.message.contains("lump 0 range"), "{}", error.message);
        // Surface index 12 exceeds the 3-vertex surface range.
        let mut bad_index = good.clone();
        let lump11 = u32::from_le_bytes([
            good[8 + 11 * 8],
            good[8 + 11 * 8 + 1],
            good[8 + 11 * 8 + 2],
            good[8 + 11 * 8 + 3],
        ]) as usize;
        bad_index[lump11] = 12;
        let error = parse_q3_bsp(&bad_index, "<test>").unwrap_err();
        assert!(error.message.contains("surface local vertex"), "{}", error.message);
        // Unknown surface type.
        let mut bad_type = good.clone();
        let lump13 = u32::from_le_bytes([
            good[8 + 13 * 8],
            good[8 + 13 * 8 + 1],
            good[8 + 13 * 8 + 2],
            good[8 + 13 * 8 + 3],
        ]) as usize;
        bad_type[lump13 + 8] = 9;
        let error = parse_q3_bsp(&bad_type, "<test>").unwrap_err();
        assert!(error.message.contains("unknown BSP surface type"), "{}", error.message);
    }

    #[test]
    fn q3_v44_normalizes() {
        let mut sections: Vec<Vec<u8>> = vec![Vec::new(); 15];
        sections[0] = b"{ \"classname\" \"worldspawn\" }\n".to_vec();
        let mut writer = BinaryWriter::new(20);
        for value in [1.0f32, 0.0, 0.0, 16.0] {
            writer.f32(value).unwrap();
        }
        writer.i32(0).unwrap();
        sections[1] = writer.finish();
        let mut writer = BinaryWriter::new(36);
        writer.i32(0).unwrap();
        writer.i32(-1).unwrap();
        writer.i32(-1).unwrap();
        for _ in 0..6 {
            writer.i32(0).unwrap();
        }
        sections[2] = writer.finish();
        let mut writer = BinaryWriter::new(48);
        writer.i32(-1).unwrap();
        writer.i32(0).unwrap();
        for _ in 0..6 {
            writer.i32(0).unwrap();
        }
        writer.i32(0).unwrap();
        writer.i32(1).unwrap();
        writer.i32(0).unwrap();
        writer.i32(1).unwrap();
        sections[3] = writer.finish();
        let mut writer = BinaryWriter::new(4);
        writer.i32(0).unwrap();
        sections[4] = writer.finish();
        let mut writer = BinaryWriter::new(4);
        writer.i32(0).unwrap();
        sections[5] = writer.finish();
        let mut writer = BinaryWriter::new(48);
        for _ in 0..9 {
            writer.f32(0.0).unwrap();
        }
        writer.i32(0).unwrap();
        writer.i32(0).unwrap();
        writer.i32(0).unwrap();
        sections[6] = writer.finish();
        let mut writer = BinaryWriter::new(12);
        writer.i32(0).unwrap();
        writer.i32(1).unwrap();
        writer.i32(1).unwrap();
        sections[7] = writer.finish();
        let mut writer = BinaryWriter::new(8);
        writer.i32(0).unwrap();
        writer.i32(128).unwrap();
        sections[8] = writer.finish();
        let mut writer = BinaryWriter::new(132);
        write_vertex(&mut writer, [0.0, 0.0, 0.0]);
        write_vertex(&mut writer, [1.0, 0.0, 0.0]);
        write_vertex(&mut writer, [0.0, 1.0, 0.0]);
        sections[11] = writer.finish();
        let mut writer = BinaryWriter::new(164);
        let mut name = [0u8; 64];
        name[..4].copy_from_slice(b"wall");
        writer.bytes(&name).unwrap();
        writer.i32(-1).unwrap();
        writer.i32(0).unwrap();
        writer.i32(0).unwrap();
        writer.i32(3).unwrap();
        writer.i32(0).unwrap();
        writer.i32(3).unwrap();
        writer.i32(0).unwrap();
        writer.i32(0).unwrap();
        writer.i32(-1).unwrap();
        for _ in 0..4 {
            writer.i32(0).unwrap();
        }
        for _ in 0..12 {
            writer.f32(0.0).unwrap();
        }
        sections[12] = writer.finish();
        let mut writer = BinaryWriter::new(12);
        for index in [0i32, 1, 2] {
            writer.i32(index).unwrap();
        }
        sections[14] = writer.finish();
        let mut file = BinaryWriter::new(4096);
        file.u32(Q3_MAGIC_IBSP).unwrap();
        file.i32(Q3_BSP_VERSION_44).unwrap();
        let mut offset = 128i32;
        for section in &sections {
            file.i32(offset).unwrap();
            file.i32(section.len() as i32).unwrap();
            offset += section.len() as i32;
        }
        for section in &sections {
            file.bytes(section).unwrap();
        }
        let bytes = file.finish();
        assert_eq!(crate::classify_bsp(&bytes, "<test>").unwrap(), crate::BspKind::Q3);
        let map = parse_q3_bsp(&bytes, "<test>").unwrap();
        assert_eq!(map.surfaces.len(), 1);
        assert_eq!(map.surfaces[0].surface_type, Q3SurfaceType::Triangles);
        assert_eq!(map.indices, vec![0, 1, 2]);
        assert_eq!(map.models[0].surface_count, 1);
        assert_eq!(map.models[0].brush_count, 1);
        let brush_shader = &map.shaders[map.brushes[0].shader as usize];
        assert_eq!(brush_shader.content_flags, 1);
        let side_shader = &map.shaders[map.brush_sides[0].shader as usize];
        assert_eq!(side_shader.surface_flags, 128);
        assert_eq!(side_shader.name, "wall");
    }
}
