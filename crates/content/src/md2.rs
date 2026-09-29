//! Quake II alias model (MD2 v8) parser with GL commands and animation.
//!
//! Donor provenance: `parseMd2` and `decodeMd2Commands` in
//! `src/formats/q12-model/md2.ts`, with shared readers from
//! `src/formats/q12-model/common.ts` (see [`crate::common`]), normals
//! from [`crate::normals`], and pose sampling, interpolation, and
//! geometry builders from `src/formats/q12-model/animation.ts`.
//!
//! Skins and decoded vertices are owned; sections are read sequentially
//! through sub-readers, borrowing nothing past validation.

use qa_core::binary::{BinaryError, BinaryReader};

use crate::common::{
    bounds_from_points, check_index, count, fail, packed_position, packed_vertex, union_bounds, vector, version,
    Bounds, PackedVertex, TimedFrames,
};
use crate::mdl::ModelVertex;
use crate::normals::alias_normal;

/// MD2 header size; all sections start at or after this offset.
const MD2_HEADER_SIZE: i64 = 68;

/// MD2 texture coordinate (integer skin pixels).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Md2TexCoord {
    /// S coordinate.
    pub x: i32,
    /// T coordinate.
    pub y: i32,
}

/// MD2 triangle with independent vertex and texture-coordinate indices.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Md2Triangle {
    /// Vertex indices.
    pub vertices: [u32; 3],
    /// Texture-coordinate indices.
    pub tex_coords: [u32; 3],
}

/// One MD2 frame (`Q2AliasFrame`).
#[derive(Debug, Clone, PartialEq)]
pub struct Md2Frame {
    /// Frame name.
    pub name: String,
    /// Vertex scale.
    pub scale: [f32; 3],
    /// Vertex translation.
    pub translation: [f32; 3],
    /// Frame origin (always zero).
    pub origin: [f32; 3],
    /// Decoded vertices.
    pub vertices: Vec<ModelVertex>,
    /// Compressed vertices.
    pub compressed_vertices: Vec<PackedVertex>,
    /// Bounds over decoded positions.
    pub bounds: Bounds,
}

/// Parsed MD2 model (`Q2AliasModel`).
#[derive(Debug, Clone, PartialEq)]
pub struct Md2Model {
    /// Skin width.
    pub skin_width: i32,
    /// Skin height.
    pub skin_height: i32,
    /// Skin names.
    pub skins: Vec<String>,
    /// Texture coordinates.
    pub texture_coordinates: Vec<Md2TexCoord>,
    /// Triangles.
    pub triangles: Vec<Md2Triangle>,
    /// Frames.
    pub frames: Vec<Md2Frame>,
    /// Raw GL command words (float UV bit patterns intact).
    pub gl_commands: Vec<i32>,
    /// Bounds.
    pub bounds: Bounds,
}

/// One GL command vertex (`Md2CommandVertex`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Md2CommandVertex {
    /// Texture coordinates.
    pub tex_coord: [f32; 2],
    /// Vertex index.
    pub vertex: u32,
}

/// GL triangle strip or fan (`Md2Command`).
#[derive(Debug, Clone, PartialEq)]
pub struct Md2Command {
    /// Whether the command is a strip (`true`) or a fan (`false`).
    pub strip: bool,
    /// Command vertices.
    pub vertices: Vec<Md2CommandVertex>,
}

fn read_commands(
    reader: &mut BinaryReader<'_>,
    source: &str,
    vertex_count: i32,
) -> Result<Vec<Md2Command>, BinaryError> {
    let mut result = Vec::new();
    if reader.length() == 0 {
        return Ok(result);
    }
    while reader.remaining() > 0 {
        let command = reader.i32()?;
        if command == 0 {
            return Ok(result);
        }
        let count = (command as i64).abs();
        if count < 3 {
            return fail(
                reader,
                source,
                "GL strip/fan must have at least three vertices".to_string(),
            );
        }
        reader.section(reader.offset(), count as usize * 12)?;
        let mut vertices = Vec::with_capacity(count as usize);
        for _ in 0..count {
            let tex_coord = [reader.finite_f32()?, reader.finite_f32()?];
            let vertex = reader.i32()?;
            vertices.push(Md2CommandVertex {
                tex_coord,
                vertex: check_index(reader, source, vertex, vertex_count, "GL vertex")?,
            });
        }
        result.push(Md2Command {
            strip: command > 0,
            vertices,
        });
    }
    fail(reader, source, "GL command list lacks terminator".to_string())
}

/// Decode the model's GL command words (`decodeMd2Commands`).
///
/// Float UV bit patterns remain intact in the round trip through bytes.
pub fn decode_md2_commands(model: &Md2Model) -> Result<Vec<Md2Command>, BinaryError> {
    let source = "<md2 GL commands>";
    let Some(frame) = model.frames.first() else {
        return Err(BinaryError {
            input: source.to_string(),
            offset: 0,
            message: "MD2 has no frames".to_string(),
        });
    };
    let mut data = Vec::with_capacity(model.gl_commands.len() * 4);
    for word in &model.gl_commands {
        data.extend_from_slice(&word.to_le_bytes());
    }
    read_commands(
        &mut BinaryReader::new(&data, source),
        source,
        frame.vertices.len() as i32,
    )
}

fn section<'a>(
    reader: &mut BinaryReader<'a>,
    source: &str,
    offset: i32,
    count: i32,
    stride: i64,
    end: i32,
) -> Result<BinaryReader<'a>, BinaryError> {
    let size = count as i64 * stride;
    if (offset as i64) < MD2_HEADER_SIZE || size > i64::from(end) || (offset as i64) > i64::from(end) - size {
        return fail(
            reader,
            source,
            format!("MD2 section {offset}+{size} exceeds header/model bounds"),
        );
    }
    reader.section(offset as usize, size as usize)
}

/// Parse an MD2 v8 model (`parseMd2`).
pub fn parse_md2(data: &[u8], source: &str) -> Result<Md2Model, BinaryError> {
    let mut reader = BinaryReader::new(data, source);
    reader.expect_magic("IDP2")?;
    version(&mut reader, source, 8)?;
    let skin_width = count(&mut reader, source, "skin width", 1)?;
    let skin_height = count(&mut reader, source, "skin height", 1)?;
    let frame_size = count(&mut reader, source, "frame size", 1)?;
    let skin_count = count(&mut reader, source, "skins", 0)?;
    let vertex_count = count(&mut reader, source, "vertices", 1)?;
    let coordinate_count = count(&mut reader, source, "texture coordinates", 1)?;
    let triangle_count = count(&mut reader, source, "triangles", 1)?;
    let command_count = count(&mut reader, source, "GL command words", 0)?;
    let frame_count = count(&mut reader, source, "frames", 1)?;
    let skin_offset = reader.i32()?;
    let coordinate_offset = reader.i32()?;
    let triangle_offset = reader.i32()?;
    let frame_offset = reader.i32()?;
    let command_offset = reader.i32()?;
    let end = reader.i32()?;
    if end < MD2_HEADER_SIZE as i32 || end as usize > reader.length() {
        return fail(&reader, source, format!("invalid model end {end}"));
    }
    if (frame_size as i64) < 40 + vertex_count as i64 * 4 {
        return fail(&reader, source, "frame size is smaller than its vertices".to_string());
    }
    let mut skin_reader = section(&mut reader, source, skin_offset, skin_count, 64, end)?;
    let mut coordinate_reader = section(&mut reader, source, coordinate_offset, coordinate_count, 4, end)?;
    let mut triangle_reader = section(&mut reader, source, triangle_offset, triangle_count, 12, end)?;
    let mut frame_reader = section(&mut reader, source, frame_offset, frame_count, frame_size as i64, end)?;
    let mut command_reader = section(&mut reader, source, command_offset, command_count, 4, end)?;
    let mut skins = Vec::with_capacity(skin_count as usize);
    for _ in 0..skin_count {
        skins.push(skin_reader.fixed_byte_string(64)?);
    }
    let mut texture_coordinates = Vec::with_capacity(coordinate_count as usize);
    for _ in 0..coordinate_count {
        texture_coordinates.push(Md2TexCoord {
            x: i32::from(coordinate_reader.i16()?),
            y: i32::from(coordinate_reader.i16()?),
        });
    }
    let mut triangles = Vec::with_capacity(triangle_count as usize);
    for _ in 0..triangle_count {
        let mut vertices = [0u32; 3];
        let mut tex_coords = [0u32; 3];
        for slot in &mut vertices {
            let value = triangle_reader.u16()?;
            *slot = check_index(&triangle_reader, source, i32::from(value), vertex_count, "vertex")?;
        }
        for slot in &mut tex_coords {
            let value = triangle_reader.u16()?;
            *slot = check_index(
                &triangle_reader,
                source,
                i32::from(value),
                coordinate_count,
                "texture coordinate",
            )?;
        }
        triangles.push(Md2Triangle { vertices, tex_coords });
    }
    let mut frames = Vec::with_capacity(frame_count as usize);
    for frame in 0..frame_count {
        frame_reader.seek(frame as usize * frame_size as usize)?;
        let scale = vector(&mut frame_reader)?;
        let translation = vector(&mut frame_reader)?;
        let name = frame_reader.fixed_byte_string(16)?;
        let mut vertices = Vec::with_capacity(vertex_count as usize);
        let mut compressed_vertices = Vec::with_capacity(vertex_count as usize);
        for _ in 0..vertex_count {
            let packed = packed_vertex(&mut frame_reader)?;
            check_index(&frame_reader, source, i32::from(packed.normal_index), 162, "normal")?;
            compressed_vertices.push(packed);
            vertices.push(ModelVertex {
                position: packed_position(packed, scale, translation),
                normal: alias_normal(packed.normal_index).unwrap_or([0.0, 0.0, 1.0]),
            });
        }
        let positions: Vec<[f32; 3]> = vertices.iter().map(|vertex| vertex.position).collect();
        frames.push(Md2Frame {
            name,
            scale,
            translation,
            origin: [0.0, 0.0, 0.0],
            vertices,
            compressed_vertices,
            bounds: bounds_from_points(&positions),
        });
    }
    read_commands(&mut command_reader, source, vertex_count)?;
    command_reader.seek(0)?;
    let mut gl_commands = Vec::with_capacity(command_count as usize);
    for _ in 0..command_count {
        gl_commands.push(command_reader.i32()?);
    }
    let bounds = union_bounds(&frames.iter().map(|frame| frame.bounds).collect::<Vec<_>>());
    Ok(Md2Model {
        skin_width,
        skin_height,
        skins,
        texture_coordinates,
        triangles,
        frames,
        gl_commands,
        bounds,
    })
}

fn at<'a, T>(values: &'a [T], index: usize, label: &str) -> &'a T {
    values.get(index).unwrap_or_else(|| panic!("Missing {label} {index}"))
}

/// Select the frame active at a time (`sampleTimedFrame`).
///
/// Q1 stores cumulative endpoints, not durations: a boundary selects the
/// next frame.
///
/// # Panics
///
/// Panics when either time argument is non-finite.
#[must_use]
pub fn sample_timed_frame<T>(frames: &TimedFrames<T>, time_seconds: f64, sync_base: f64) -> &T {
    if !time_seconds.is_finite() || !sync_base.is_finite() {
        panic!("Animation time must be finite");
    }
    match frames {
        TimedFrames::Single(frame) => frame,
        TimedFrames::Group(frames) => {
            let last = at(frames, frames.len() - 1, "group frame");
            let time = time_seconds + sync_base;
            let endpoint = f64::from(last.interval_seconds);
            let target = time - (time / endpoint).trunc() * endpoint;
            for item in frames {
                if f64::from(item.interval_seconds) > target {
                    return &item.frame;
                }
            }
            &last.frame
        }
    }
}

/// Blend two alias poses (`interpolateAliasFrames`).
///
/// Q1 and Q2 light the current pose's normal while interpolating vertex
/// positions.
///
/// # Panics
///
/// Panics when `back_lerp` is outside `0..=1` or the poses differ in
/// length.
#[must_use]
pub fn interpolate_alias_frames(
    current: &[ModelVertex],
    previous: &[ModelVertex],
    back_lerp: f32,
    previous_origin_delta: [f32; 3],
) -> Vec<ModelVertex> {
    if !back_lerp.is_finite() || back_lerp < 0.0 || back_lerp > 1.0 {
        panic!("backLerp must be in 0..1");
    }
    if current.len() != previous.len() {
        panic!("Alias poses have different vertex counts");
    }
    let back = f64::from(back_lerp);
    let front = 1.0 - back;
    current
        .iter()
        .zip(previous.iter())
        .map(|(vertex, old)| {
            let position = [
                (f64::from(vertex.position[0]) * front
                    + (f64::from(old.position[0]) + f64::from(previous_origin_delta[0])) * back) as f32,
                (f64::from(vertex.position[1]) * front
                    + (f64::from(old.position[1]) + f64::from(previous_origin_delta[1])) * back) as f32,
                (f64::from(vertex.position[2]) * front
                    + (f64::from(old.position[2]) + f64::from(previous_origin_delta[2])) * back) as f32,
            ];
            ModelVertex {
                position,
                normal: vertex.normal,
            }
        })
        .collect()
}

/// MD2 mesh vertex with resolved texture coordinates.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Md2MeshVertex {
    /// Position.
    pub position: [f32; 3],
    /// Normal.
    pub normal: [f32; 3],
    /// Texture coordinates.
    pub tex_coord: [f32; 2],
}

/// MD2 triangle geometry for one pose.
#[derive(Debug, Clone, PartialEq)]
pub struct Md2Geometry {
    /// Corner vertices.
    pub vertices: Vec<Md2MeshVertex>,
    /// Corner indices.
    pub indices: Vec<u32>,
}

fn md2_tex_coord(model: &Md2Model, coordinate: Md2TexCoord) -> [f32; 2] {
    [
        ((f64::from(coordinate.x) + 0.5) / f64::from(model.skin_width)) as f32,
        ((f64::from(coordinate.y) + 0.5) / f64::from(model.skin_height)) as f32,
    ]
}

/// Expand MD2 triangles into corners (`buildMd2Geometry`).
///
/// # Panics
///
/// Panics when the pose is shorter than the model's vertex count.
#[must_use]
pub fn build_md2_geometry(model: &Md2Model, pose: &[ModelVertex]) -> Md2Geometry {
    let mut vertices = Vec::with_capacity(model.triangles.len() * 3);
    let mut indices = Vec::with_capacity(model.triangles.len() * 3);
    for triangle in &model.triangles {
        for corner in 0..3 {
            let index = triangle.vertices[corner] as usize;
            let vertex = at(pose, index, "pose vertex");
            let coordinate = triangle.tex_coords[corner] as usize;
            let tex_coord = md2_tex_coord(model, *at(&model.texture_coordinates, coordinate, "texture coordinate"));
            indices.push(vertices.len() as u32);
            vertices.push(Md2MeshVertex {
                position: vertex.position,
                normal: vertex.normal,
                tex_coord,
            });
        }
    }
    Md2Geometry { vertices, indices }
}

/// Mapped MD2 geometry (`mapMd2Geometry` result).
#[derive(Debug, Clone, PartialEq)]
pub struct Md2MappedGeometry<T> {
    /// Mapped corner vertices.
    pub vertices: Vec<T>,
    /// Corner indices.
    pub indices: Vec<u32>,
}

/// Map MD2 corners into a consumer's geometry (`mapMd2Geometry`).
///
/// # Panics
///
/// Panics when the pose is shorter than the model's vertex count.
pub fn map_md2_geometry<T>(
    model: &Md2Model,
    pose: &[ModelVertex],
    map: impl Fn(&ModelVertex, [f32; 2], usize) -> T,
) -> Md2MappedGeometry<T> {
    let mut vertices = Vec::with_capacity(model.triangles.len() * 3);
    let mut indices = Vec::with_capacity(model.triangles.len() * 3);
    for triangle in &model.triangles {
        for corner in 0..3 {
            let vertex = at(pose, triangle.vertices[corner] as usize, "pose vertex");
            let coordinate = triangle.tex_coords[corner] as usize;
            let tex_coord = md2_tex_coord(model, *at(&model.texture_coordinates, coordinate, "texture coordinate"));
            indices.push(vertices.len() as u32);
            let corner_index = vertices.len();
            vertices.push(map(vertex, tex_coord, corner_index));
        }
    }
    Md2MappedGeometry { vertices, indices }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::{TimedFrame, TimedFrames};
    use qa_core::binary::BinaryWriter;

    fn fixture() -> Vec<u8> {
        let mut writer = BinaryWriter::new(512);
        writer.bytes(b"IDP2").unwrap();
        writer.i32(8).unwrap();
        writer.i32(64).unwrap(); // skin width
        writer.i32(64).unwrap(); // skin height
        writer.i32(52).unwrap(); // frame size
        writer.i32(1).unwrap(); // skins
        writer.i32(3).unwrap(); // vertices
        writer.i32(3).unwrap(); // texture coordinates
        writer.i32(1).unwrap(); // triangles
        writer.i32(11).unwrap(); // GL command words
        writer.i32(1).unwrap(); // frames
        writer.i32(68).unwrap(); // skins
        writer.i32(132).unwrap(); // coordinates
        writer.i32(144).unwrap(); // triangles
        writer.i32(156).unwrap(); // frames
        writer.i32(208).unwrap(); // commands
        writer.i32(252).unwrap(); // end
        let mut skin = [0u8; 64];
        skin[..8].copy_from_slice(b"skin.pcx");
        writer.bytes(&skin).unwrap();
        for coordinate in [[0i16, 0], [63, 0], [0, 63]] {
            writer.i16(coordinate[0]).unwrap();
            writer.i16(coordinate[1]).unwrap();
        }
        for index in [0u16, 1, 2, 0, 1, 2] {
            writer.u16(index).unwrap();
        }
        for value in [1.0f32, 1.0, 1.0, 10.0, 20.0, 30.0] {
            writer.f32(value).unwrap();
        }
        let mut name = [0u8; 16];
        name[..6].copy_from_slice(b"frame0");
        writer.bytes(&name).unwrap();
        writer.bytes(&[0, 0, 0, 5, 10, 0, 0, 5, 0, 10, 0, 5]).unwrap();
        writer.i32(3).unwrap();
        for (uv, vertex) in [([0.0f32, 0.0], 0i32), ([1.0, 0.0], 1), ([0.0, 1.0], 2)] {
            writer.f32(uv[0]).unwrap();
            writer.f32(uv[1]).unwrap();
            writer.i32(vertex).unwrap();
        }
        writer.i32(0).unwrap();
        writer.finish()
    }

    #[test]
    fn md2_round_trip() {
        let model = parse_md2(&fixture(), "<test>").unwrap();
        assert_eq!((model.skin_width, model.skin_height), (64, 64));
        assert_eq!(model.skins, vec!["skin.pcx".to_string()]);
        assert_eq!(model.texture_coordinates.len(), 3);
        assert_eq!(model.triangles[0].vertices, [0, 1, 2]);
        assert_eq!(model.triangles[0].tex_coords, [0, 1, 2]);
        assert_eq!(model.frames.len(), 1);
        let frame = &model.frames[0];
        assert_eq!(frame.name, "frame0");
        assert_eq!(frame.vertices[0].position, [10.0, 20.0, 30.0]);
        assert_eq!(frame.vertices[1].position, [20.0, 20.0, 30.0]);
        assert_eq!(frame.vertices[2].position, [10.0, 30.0, 30.0]);
        assert_eq!(frame.vertices[0].normal, alias_normal(5).unwrap());
        assert_eq!(frame.bounds.min, [10.0, 20.0, 30.0]);
        assert_eq!(frame.bounds.max, [20.0, 30.0, 30.0]);
        assert_eq!(model.bounds.min, [10.0, 20.0, 30.0]);
        assert_eq!(model.gl_commands.len(), 11);

        let commands = decode_md2_commands(&model).unwrap();
        assert_eq!(commands.len(), 1);
        assert!(commands[0].strip);
        assert_eq!(commands[0].vertices.len(), 3);
        assert_eq!(commands[0].vertices[1].tex_coord, [1.0, 0.0]);

        let geometry = build_md2_geometry(&model, &frame.vertices);
        assert_eq!(geometry.indices, vec![0, 1, 2]);
        assert_eq!(geometry.vertices.len(), 3);
        assert_eq!(geometry.vertices[0].position, [10.0, 20.0, 30.0]);
        assert_eq!(geometry.vertices[1].tex_coord[0], (63.0 + 0.5) / 64.0);

        let mapped = map_md2_geometry(&model, &frame.vertices, |vertex, tex_coord, corner| {
            (corner, vertex.position, tex_coord)
        });
        assert_eq!(mapped.indices, vec![0, 1, 2]);
        assert_eq!(mapped.vertices[2].0, 2);

        let blended = interpolate_alias_frames(&frame.vertices, &frame.vertices, 0.5, [0.0, 0.0, 0.0]);
        assert_eq!(blended[0].position, [10.0, 20.0, 30.0]);
        assert_eq!(blended[0].normal, frame.vertices[0].normal);
    }

    #[test]
    fn md2_samples_timed_frames() {
        let single = TimedFrames::Single("pose");
        assert_eq!(*sample_timed_frame(&single, 3.0, 0.0), "pose");
        let group = TimedFrames::Group(vec![
            TimedFrame {
                interval_seconds: 0.1,
                frame: "a",
            },
            TimedFrame {
                interval_seconds: 0.3,
                frame: "b",
            },
        ]);
        assert_eq!(*sample_timed_frame(&group, 0.0, 0.0), "a");
        // An exact endpoint boundary selects the next frame.
        assert_eq!(*sample_timed_frame(&group, f64::from(0.1f32), 0.0), "b");
        assert_eq!(*sample_timed_frame(&group, 0.15, 0.0), "b");
        // Time wraps past the final endpoint.
        assert_eq!(*sample_timed_frame(&group, 0.35, 0.0), "a");
    }

    #[test]
    fn md2_rejects_bad_input() {
        let good = fixture();
        let mut bad_magic = good.clone();
        bad_magic[0] = b'X';
        assert!(parse_md2(&bad_magic, "<test>").is_err());
        let mut bad_version = good.clone();
        bad_version[4] = 9;
        assert!(parse_md2(&bad_version, "<test>").is_err());
        assert!(parse_md2(&good[..67], "<test>").is_err());
        assert!(parse_md2(&good[..good.len() - 1], "<test>").is_err());
        // Corrupt the frame normal index past the table.
        let mut bad_normal = good.clone();
        bad_normal[156 + 40 + 11] = 200;
        assert!(parse_md2(&bad_normal, "<test>").is_err());
        // Unterminated GL commands.
        let mut bad_commands = good.clone();
        let last = bad_commands.len() - 4;
        bad_commands[last..].copy_from_slice(&1i32.to_le_bytes());
        assert!(parse_md2(&bad_commands, "<test>").is_err());
    }
}
