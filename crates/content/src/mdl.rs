//! Quake alias model (MDL v6) parser.
//!
//! Donor provenance: `parseMdl` in `src/formats/q12-model/mdl.ts`, with
//! shared readers from `src/formats/q12-model/common.ts` (see
//! [`crate::common`]) and normals from [`crate::normals`].
//!
//! Skins and decoded vertices are owned; the table of contents is read
//! sequentially, borrowing nothing past validation.

use qa_core::binary::{BinaryError, BinaryReader};

use crate::common::{
    bounds_from_points, check_index, count, fail, group_type, intervals, packed_position, packed_vertex, read_timed,
    sync_type, union_bounds, vector, version, Bounds, GroupType, PackedVertex, SyncType, TimedFrame, TimedFrames,
};
use crate::normals::alias_normal;

/// Decoded alias vertex.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ModelVertex {
    /// Position.
    pub position: [f32; 3],
    /// Normal.
    pub normal: [f32; 3],
}

/// Texture coordinates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TextureCoordinate {
    /// Whether the vertex is on a seam.
    pub on_seam: bool,
    /// S coordinate.
    pub s: i32,
    /// T coordinate.
    pub t: i32,
}

/// Model triangle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Triangle {
    /// Whether the triangle faces forward.
    pub front: bool,
    /// Vertex indices.
    pub vertices: [u32; 3],
}

/// One alias frame.
#[derive(Debug, Clone, PartialEq)]
pub struct AliasFrame {
    /// Frame name.
    pub name: String,
    /// Compressed bounds.
    pub compressed_bounds: [PackedVertex; 2],
    /// Compressed vertices.
    pub compressed_vertices: Vec<PackedVertex>,
    /// Decoded vertices.
    pub vertices: Vec<ModelVertex>,
    /// Bounds.
    pub bounds: Bounds,
}

/// Single frame or timed frame group.
#[derive(Debug, Clone, PartialEq)]
pub enum FrameSet {
    /// One frame.
    Single(AliasFrame),
    /// Timed group.
    Group {
        /// Compressed bounds.
        compressed_bounds: [PackedVertex; 2],
        /// Bounds.
        bounds: Bounds,
        /// Frames.
        frames: Vec<TimedFrame<AliasFrame>>,
    },
}

impl FrameSet {
    /// Frame set bounds.
    #[must_use]
    pub fn bounds(&self) -> Bounds {
        match self {
            FrameSet::Single(frame) => frame.bounds,
            FrameSet::Group { bounds, .. } => *bounds,
        }
    }
}

/// Parsed alias model (`Q1AliasModel`).
#[derive(Debug, Clone, PartialEq)]
pub struct MdlModel {
    /// Vertex scale.
    pub scale: [f32; 3],
    /// Scale origin.
    pub scale_origin: [f32; 3],
    /// Bounding radius.
    pub bounding_radius: f32,
    /// Eye position.
    pub eye_position: [f32; 3],
    /// Model size.
    pub size: f32,
    /// Skin width.
    pub skin_width: i32,
    /// Skin height.
    pub skin_height: i32,
    /// Synchronization type.
    pub sync: SyncType,
    /// Flags.
    pub flags: i32,
    /// Skins.
    pub skins: Vec<TimedFrames<Vec<u8>>>,
    /// Texture coordinates.
    pub texture_coordinates: Vec<TextureCoordinate>,
    /// Triangles.
    pub triangles: Vec<Triangle>,
    /// Frames.
    pub frames: Vec<FrameSet>,
    /// Bounds.
    pub bounds: Bounds,
}

fn frame(
    reader: &mut BinaryReader<'_>,
    source: &str,
    vertex_count: usize,
    scale: [f32; 3],
    translate: [f32; 3],
) -> Result<AliasFrame, BinaryError> {
    let min = packed_vertex(reader)?;
    let max = packed_vertex(reader)?;
    let name = reader.fixed_byte_string(16)?;
    let mut compressed_vertices = Vec::with_capacity(vertex_count);
    let mut vertices = Vec::with_capacity(vertex_count);
    for _ in 0..vertex_count {
        let packed = packed_vertex(reader)?;
        check_index(reader, source, i32::from(packed.normal_index), 162, "normal")?;
        let normal = alias_normal(packed.normal_index).unwrap_or([0.0, 0.0, 1.0]);
        compressed_vertices.push(packed);
        vertices.push(ModelVertex {
            position: packed_position(packed, scale, translate),
            normal,
        });
    }
    let bounds = bounds_from_points(&[
        packed_position(min, scale, translate),
        packed_position(max, scale, translate),
    ]);
    Ok(AliasFrame {
        name,
        compressed_bounds: [min, max],
        compressed_vertices,
        vertices,
        bounds,
    })
}

fn frame_set(
    reader: &mut BinaryReader<'_>,
    source: &str,
    vertex_count: usize,
    scale: [f32; 3],
    translate: [f32; 3],
) -> Result<FrameSet, BinaryError> {
    if group_type(reader, source)? == GroupType::Single {
        return Ok(FrameSet::Single(frame(reader, source, vertex_count, scale, translate)?));
    }
    let frame_count = count(reader, source, "group frames", 1)?;
    let min = packed_vertex(reader)?;
    let max = packed_vertex(reader)?;
    let endpoints = intervals(reader, source, frame_count as usize)?;
    let mut frames = Vec::with_capacity(endpoints.len());
    for interval_seconds in endpoints {
        frames.push(TimedFrame {
            interval_seconds,
            frame: frame(reader, source, vertex_count, scale, translate)?,
        });
    }
    let bounds = bounds_from_points(&[
        packed_position(min, scale, translate),
        packed_position(max, scale, translate),
    ]);
    Ok(FrameSet::Group {
        compressed_bounds: [min, max],
        bounds,
        frames,
    })
}

/// Parse an MDL v6 model (`parseMdl`).
pub fn parse_mdl(data: &[u8], source: &str) -> Result<MdlModel, BinaryError> {
    let mut reader = BinaryReader::new(data, source);
    reader.expect_magic("IDPO")?;
    version(&mut reader, source, 6)?;
    let scale = vector(&mut reader)?;
    let scale_origin = vector(&mut reader)?;
    let bounding_radius = reader.finite_f32()?;
    let eye_position = vector(&mut reader)?;
    let skin_count = count(&mut reader, source, "skins", 1)?;
    let skin_width = count(&mut reader, source, "skin width", 1)?;
    let skin_height = count(&mut reader, source, "skin height", 1)?;
    let vertex_count = count(&mut reader, source, "vertices", 1)?;
    let triangle_count = count(&mut reader, source, "triangles", 1)?;
    let frame_count = count(&mut reader, source, "frames", 1)?;
    let sync = sync_type(&mut reader, source)?;
    let flags = reader.i32()?;
    let size = reader.finite_f32()?;
    if bounding_radius < 0.0 {
        return fail(&reader, source, "negative model radius".to_string());
    }
    let skin_len = skin_width as usize * skin_height as usize;
    let mut skins = Vec::with_capacity(skin_count as usize);
    for _ in 0..skin_count {
        skins.push(read_timed(&mut reader, source, |reader| reader.bytes(skin_len))?);
    }
    let mut texture_coordinates = Vec::with_capacity(vertex_count as usize);
    for _ in 0..vertex_count {
        texture_coordinates.push(TextureCoordinate {
            on_seam: reader.i32()? != 0,
            s: reader.i32()?,
            t: reader.i32()?,
        });
    }
    let mut triangles = Vec::with_capacity(triangle_count as usize);
    for _ in 0..triangle_count {
        let front = reader.i32()? != 0;
        let a = reader.i32()?;
        let b = reader.i32()?;
        let c = reader.i32()?;
        triangles.push(Triangle {
            front,
            vertices: [
                check_index(&reader, source, a, vertex_count, "vertex")?,
                check_index(&reader, source, b, vertex_count, "vertex")?,
                check_index(&reader, source, c, vertex_count, "vertex")?,
            ],
        });
    }
    let mut frames = Vec::with_capacity(frame_count as usize);
    for _ in 0..frame_count {
        frames.push(frame_set(
            &mut reader,
            source,
            vertex_count as usize,
            scale,
            scale_origin,
        )?);
    }
    let bounds = union_bounds(&frames.iter().map(FrameSet::bounds).collect::<Vec<_>>());
    Ok(MdlModel {
        scale,
        scale_origin,
        bounding_radius,
        eye_position,
        size,
        skin_width,
        skin_height,
        sync,
        flags,
        skins,
        texture_coordinates,
        triangles,
        frames,
        bounds,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_core::binary::BinaryWriter;

    fn fixture() -> Vec<u8> {
        let mut writer = BinaryWriter::new(1024);
        writer.bytes(b"IDPO").unwrap();
        writer.i32(6).unwrap();
        for value in [0.5f32, 0.5, 0.5] {
            writer.f32(value).unwrap();
        }
        for value in [1.0f32, 2.0, 3.0] {
            writer.f32(value).unwrap();
        }
        writer.f32(10.0).unwrap();
        for value in [0.0f32, 0.0, 0.0] {
            writer.f32(value).unwrap();
        }
        writer.i32(1).unwrap(); // skins
        writer.i32(4).unwrap(); // skin width
        writer.i32(4).unwrap(); // skin height
        writer.i32(3).unwrap(); // vertices
        writer.i32(1).unwrap(); // triangles
        writer.i32(2).unwrap(); // frames
        writer.i32(0).unwrap(); // sync
        writer.i32(0).unwrap(); // flags
        writer.f32(1.0).unwrap(); // size
        writer.i32(0).unwrap(); // single skin
        writer.bytes(&[7u8; 16]).unwrap();
        for seam in [0, 1, 0] {
            writer.i32(seam).unwrap();
            writer.i32(8).unwrap();
            writer.i32(12).unwrap();
        }
        writer.i32(1).unwrap(); // front
        writer.i32(0).unwrap();
        writer.i32(1).unwrap();
        writer.i32(2).unwrap();
        // Single frame.
        writer.i32(0).unwrap();
        writer.bytes(&[0, 0, 0, 0, 4, 4, 4, 0]).unwrap();
        let mut name = [0u8; 16];
        name[..5].copy_from_slice(b"frame");
        writer.bytes(&name).unwrap();
        writer.bytes(&[0, 0, 0, 5, 2, 2, 2, 5, 4, 4, 4, 5]).unwrap();
        // Group frame with one member (members carry no marker).
        writer.i32(1).unwrap();
        writer.i32(1).unwrap();
        writer.bytes(&[0, 0, 0, 0, 4, 4, 4, 0]).unwrap();
        writer.f32(0.1).unwrap();
        writer.bytes(&[0, 0, 0, 0, 4, 4, 4, 0]).unwrap();
        writer.bytes(&name).unwrap();
        writer.bytes(&[1, 1, 1, 5, 2, 2, 2, 5, 3, 3, 3, 5]).unwrap();
        writer.finish()
    }

    #[test]
    fn mdl_round_trip() {
        let model = parse_mdl(&fixture(), "<test>").unwrap();
        assert_eq!(model.scale, [0.5, 0.5, 0.5]);
        assert_eq!(model.skin_width, 4);
        assert_eq!(model.sync, SyncType::Synchronized);
        assert_eq!(model.skins.len(), 1);
        assert!(matches!(&model.skins[0], TimedFrames::Single(skin) if skin == &[7u8; 16]));
        assert!(model.texture_coordinates[1].on_seam);
        assert_eq!(model.triangles[0].vertices, [0, 1, 2]);
        assert_eq!(model.frames.len(), 2);
        let TimedFrames::Single(_) = &model.skins[0] else {
            unreachable!();
        };
        match &model.frames[0] {
            FrameSet::Single(frame) => {
                assert_eq!(frame.name, "frame");
                assert_eq!(frame.vertices.len(), 3);
                assert_eq!(frame.vertices[0].position, [1.0, 2.0, 3.0]);
                assert_eq!(frame.vertices[2].position, [3.0, 4.0, 5.0]);
                assert_eq!(frame.vertices[0].normal, alias_normal(5).unwrap());
            }
            FrameSet::Group { .. } => panic!("expected single frame"),
        }
        match &model.frames[1] {
            FrameSet::Group { frames, .. } => {
                assert_eq!(frames.len(), 1);
                assert_eq!(frames[0].interval_seconds, 0.1);
                assert_eq!(frames[0].frame.name, "frame");
                assert_eq!(frames[0].frame.vertices[0].position, [1.5, 2.5, 3.5]);
            }
            FrameSet::Single(_) => panic!("expected group frame"),
        }
        assert_eq!(model.bounds.min, [1.0, 2.0, 3.0]);
        assert_eq!(model.bounds.max, [3.0, 4.0, 5.0]);
    }

    #[test]
    fn mdl_rejects_bad_input() {
        let good = fixture();
        let mut bad_magic = good.clone();
        bad_magic[0] = b'X';
        assert!(parse_mdl(&bad_magic, "<test>").is_err());
        let mut bad_version = good.clone();
        bad_version[4] = 7;
        assert!(parse_mdl(&bad_version, "<test>").is_err());
        assert!(parse_mdl(&good[..good.len() - 4], "<test>").is_err());
        // Corrupt the final normal index past the table.
        let mut bad_normal = good.clone();
        let last = bad_normal.len() - 1;
        bad_normal[last] = 200;
        assert!(parse_mdl(&bad_normal, "<test>").is_err());
    }
}
