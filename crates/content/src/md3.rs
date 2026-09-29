//! Quake III mesh model (MD3 v15) parser, interpolation, and skins.
//!
//! Donor provenance: `src/formats/q3-model/md3.ts` (loading and
//! interpolation from `qfiles.h`, `tr_model.c`, `tr_surface.c`), with the
//! normal decode table from `src/core/renderer-math.ts` (`tr_init.c`,
//! `q_math.c`). The sine table, inverse square root, and fast normalize
//! live here until `qa-core` adopts the renderer-math module.
//!
//! Tag and surface interpolation reuse [`qa_core::math`]; out-of-range
//! frame selections panic with the donor `RangeError` messages.

use std::sync::OnceLock;

use qa_core::binary::{BinaryError, BinaryReader};
use qa_core::math::{dot3, normalize3, scale3, vec2, vec3, Axis, Bounds, Vec2, Vec3};

/// MD3 magic (`0x33504449`, little-endian `"IDP3"`).
pub const MD3_IDENT: u32 = 0x3350_4449;
/// MD3 version.
pub const MD3_VERSION: i32 = 15;

const HEADER_SIZE: i64 = 108;
const FRAME_SIZE: usize = 56;
const TAG_SIZE: usize = 112;
const SURFACE_HEADER_SIZE: i64 = 108;
const SHADER_SIZE: usize = 68;
const TRIANGLE_SIZE: usize = 12;
const TEX_COORD_SIZE: usize = 8;
const VERTEX_SIZE: usize = 8;
const XYZ_SCALE: f32 = 1.0 / 64.0;
const FUNCTION_TABLE_SIZE: u32 = 1024;

const MAX_FRAMES: i32 = 1024;
const MAX_TAGS: i32 = 16;
const MAX_SURFACES: i32 = 32;
const MAX_SHADERS: i32 = 256;
const MAX_VERTICES: i32 = 4096;
const MAX_TRIANGLES: i32 = 8192;

/// MD3 frame (`Md3Frame`).
#[derive(Debug, Clone, PartialEq)]
pub struct Md3Frame {
    /// Bounds.
    pub bounds: Bounds,
    /// Origin.
    pub origin: Vec3,
    /// Radius.
    pub radius: f32,
    /// Name.
    pub name: String,
}

/// MD3 tag (`Md3Tag`).
#[derive(Debug, Clone, PartialEq)]
pub struct Md3Tag {
    /// Name.
    pub name: String,
    /// Origin.
    pub origin: Vec3,
    /// Axes.
    pub axes: Axis,
}

/// MD3 shader (`Md3Shader`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Md3Shader {
    /// Name.
    pub name: String,
    /// Shader index.
    pub index: i32,
}

/// MD3 triangle (`Md3Triangle`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Md3Triangle {
    /// Vertex indices.
    pub indices: [u32; 3],
}

/// MD3 vertex (`Md3Vertex`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Md3Vertex {
    /// Position.
    pub position: Vec3,
    /// Normal.
    pub normal: Vec3,
}

/// MD3 surface (`Md3Surface`).
#[derive(Debug, Clone, PartialEq)]
pub struct Md3Surface {
    /// Name with any LOD suffix stripped.
    pub name: String,
    /// Flags.
    pub flags: i32,
    /// Shaders.
    pub shaders: Vec<Md3Shader>,
    /// Triangles.
    pub triangles: Vec<Md3Triangle>,
    /// Texture coordinates.
    pub tex_coords: Vec<Vec2>,
    /// Per-frame vertices.
    pub frames: Vec<Vec<Md3Vertex>>,
}

/// MD3 model (`Md3Model`).
#[derive(Debug, Clone, PartialEq)]
pub struct Md3Model {
    /// Name.
    pub name: String,
    /// Flags.
    pub flags: i32,
    /// Skin count.
    pub skin_count: i32,
    /// Frames.
    pub frames: Vec<Md3Frame>,
    /// Per-frame tags.
    pub tags: Vec<Vec<Md3Tag>>,
    /// Surfaces.
    pub surfaces: Vec<Md3Surface>,
}

/// Decoded MD3 model with its source bytes (`DecodedMd3Model`).
#[derive(Debug, Clone, PartialEq)]
pub struct DecodedMd3Model {
    /// Source name.
    pub source: String,
    /// Model bytes through `byte_length`.
    pub bytes: Vec<u8>,
    /// Model length.
    pub byte_length: usize,
    /// Model records.
    pub model: Md3Model,
}

/// Skin surface mapping (`SkinSurface`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkinSurface {
    /// Surface name.
    pub name: String,
    /// Shader name.
    pub shader: String,
}

fn c_string(reader: &mut BinaryReader<'_>, length: usize) -> Result<String, BinaryError> {
    let bytes = reader.bytes(length)?;
    let mut result = String::new();
    for byte in bytes {
        if byte == 0 {
            break;
        }
        result.push(char::from(byte));
    }
    Ok(result)
}

fn md3_vec3(reader: &mut BinaryReader<'_>) -> Result<Vec3, BinaryError> {
    Ok(vec3(reader.finite_f32()?, reader.finite_f32()?, reader.finite_f32()?))
}

fn md3_count(value: i32, maximum: i32, source: &str, offset: usize, name: &str) -> Result<i32, BinaryError> {
    if value < 0 || value > maximum {
        return Err(BinaryError {
            input: source.to_string(),
            offset,
            message: format!("{name} count {value} outside 0..{maximum}"),
        });
    }
    Ok(value)
}

fn md3_range(
    offset: i64,
    length: i64,
    start: i64,
    end: i64,
    source: &str,
    field_offset: usize,
    name: &str,
) -> Result<(), BinaryError> {
    if offset < start || length < 0 || offset > end - length {
        return Err(BinaryError {
            input: source.to_string(),
            offset: field_offset,
            message: format!("{name} range {offset}+{length} exceeds {start}..{end}"),
        });
    }
    Ok(())
}

fn read_frame(reader: &mut BinaryReader<'_>) -> Result<Md3Frame, BinaryError> {
    let minimum = md3_vec3(reader)?;
    let maximum = md3_vec3(reader)?;
    let origin = md3_vec3(reader)?;
    let radius = reader.finite_f32()?;
    let name = c_string(reader, 16)?;
    Ok(Md3Frame {
        bounds: Bounds {
            min: minimum,
            max: maximum,
        },
        origin,
        radius,
        name,
    })
}

fn read_tag(reader: &mut BinaryReader<'_>) -> Result<Md3Tag, BinaryError> {
    Ok(Md3Tag {
        name: c_string(reader, 64)?,
        origin: md3_vec3(reader)?,
        axes: [md3_vec3(reader)?, md3_vec3(reader)?, md3_vec3(reader)?],
    })
}

fn sine_table() -> &'static [f32; 1024] {
    static TABLE: OnceLock<[f32; 1024]> = OnceLock::new();
    TABLE.get_or_init(|| {
        let mut table = [0.0f32; 1024];
        for (index, slot) in table.iter_mut().enumerate() {
            let degrees = (index as f64 * 360.0 / (FUNCTION_TABLE_SIZE as f64 - 1.0)) as f32;
            *slot = (f64::from(degrees) * std::f64::consts::PI / 180.0).sin() as f32;
        }
        table
    })
}

/// Read the renderer sine table (`rendererSine`).
#[must_use]
pub fn renderer_sine(index: i32) -> f32 {
    sine_table()[(index & 1023) as usize]
}

/// `q_math.c` `Q_rsqrt` with one refinement (`inverseSqrt32`).
#[must_use]
pub fn inverse_sqrt32(square: f32) -> f32 {
    let approximate = 0x5f37_59df_i32.wrapping_sub((square.to_bits() as i32) >> 1);
    let mut inverse = f32::from_bits(approximate as u32);
    let half_square = square * 0.5;
    inverse *= 1.5 - (half_square * inverse) * inverse;
    inverse
}

/// `q_math.c` `VectorNormalizeFast` (`normalizeFast3`).
#[must_use]
pub fn normalize_fast3(value: Vec3) -> Vec3 {
    scale3(value, inverse_sqrt32(dot3(value, value)))
}

/// Decode a packed lat-long normal (`decodeMd3Normal`).
#[must_use]
pub fn decode_md3_normal(packed: u16) -> Vec3 {
    let latitude = u32::from(packed >> 8) * (FUNCTION_TABLE_SIZE / 256);
    let longitude = u32::from(packed & 0xff) * (FUNCTION_TABLE_SIZE / 256);
    let eval = |index: u32| renderer_sine(index as i32);
    vec3(
        eval(latitude + FUNCTION_TABLE_SIZE / 4) * eval(longitude),
        eval(latitude) * eval(longitude),
        eval(longitude + FUNCTION_TABLE_SIZE / 4),
    )
}

fn surface_name(name: &str) -> String {
    let lower = name.to_lowercase();
    let chars: Vec<char> = lower.chars().collect();
    if chars.len() > 2 && chars[chars.len() - 2] == '_' {
        chars[..chars.len() - 2].iter().collect()
    } else {
        lower
    }
}

fn parse_surface(
    reader: &mut BinaryReader<'_>,
    source: &str,
    surface_offset: i64,
    model_end: i64,
    model_frame_count: i32,
) -> Result<(Md3Surface, i64), BinaryError> {
    md3_range(
        surface_offset,
        SURFACE_HEADER_SIZE,
        HEADER_SIZE,
        model_end,
        source,
        surface_offset as usize,
        "surface header",
    )?;
    reader.seek(surface_offset as usize)?;
    if reader.u32()? != MD3_IDENT {
        return Err(BinaryError {
            input: source.to_string(),
            offset: surface_offset as usize,
            message: "expected IDP3 surface magic".to_string(),
        });
    }
    let name = surface_name(&c_string(reader, 64)?);
    let flags = reader.i32()?;
    let frame_count = md3_count(
        reader.i32()?,
        MAX_FRAMES,
        source,
        (surface_offset + 72) as usize,
        "surface frame",
    )?;
    let shader_count = md3_count(
        reader.i32()?,
        MAX_SHADERS,
        source,
        (surface_offset + 76) as usize,
        "surface shader",
    )?;
    let vertex_count = md3_count(
        reader.i32()?,
        MAX_VERTICES,
        source,
        (surface_offset + 80) as usize,
        "surface vertex",
    )?;
    let triangle_count = md3_count(
        reader.i32()?,
        MAX_TRIANGLES,
        source,
        (surface_offset + 84) as usize,
        "surface triangle",
    )?;
    let triangles_offset = reader.i32()?;
    let shaders_offset = reader.i32()?;
    let tex_coords_offset = reader.i32()?;
    let vertices_offset = reader.i32()?;
    let surface_length = reader.i32()?;
    if frame_count != model_frame_count {
        return Err(BinaryError {
            input: source.to_string(),
            offset: (surface_offset + 72) as usize,
            message: format!("surface has {frame_count} frames, model has {model_frame_count}"),
        });
    }
    if surface_length < SURFACE_HEADER_SIZE as i32 || surface_offset > model_end - i64::from(surface_length) {
        return Err(BinaryError {
            input: source.to_string(),
            offset: (surface_offset + 104) as usize,
            message: format!("surface end {surface_length} exceeds model end {model_end}"),
        });
    }
    let surface_end = surface_offset + i64::from(surface_length);
    let shader_bytes = i64::from(shader_count) * SHADER_SIZE as i64;
    let triangle_bytes = i64::from(triangle_count) * TRIANGLE_SIZE as i64;
    let tex_coord_bytes = i64::from(vertex_count) * TEX_COORD_SIZE as i64;
    let vertex_bytes = i64::from(frame_count) * i64::from(vertex_count) * VERTEX_SIZE as i64;
    md3_range(
        i64::from(shaders_offset),
        shader_bytes,
        SURFACE_HEADER_SIZE,
        i64::from(surface_length),
        source,
        (surface_offset + 92) as usize,
        "shaders",
    )?;
    md3_range(
        i64::from(triangles_offset),
        triangle_bytes,
        SURFACE_HEADER_SIZE,
        i64::from(surface_length),
        source,
        (surface_offset + 88) as usize,
        "triangles",
    )?;
    md3_range(
        i64::from(tex_coords_offset),
        tex_coord_bytes,
        SURFACE_HEADER_SIZE,
        i64::from(surface_length),
        source,
        (surface_offset + 96) as usize,
        "texture coordinates",
    )?;
    md3_range(
        i64::from(vertices_offset),
        vertex_bytes,
        SURFACE_HEADER_SIZE,
        i64::from(surface_length),
        source,
        (surface_offset + 100) as usize,
        "vertices",
    )?;

    reader.seek((surface_offset + i64::from(shaders_offset)) as usize)?;
    let mut shaders = Vec::with_capacity(shader_count as usize);
    for _ in 0..shader_count {
        shaders.push(Md3Shader {
            name: c_string(reader, 64)?,
            index: reader.i32()?,
        });
    }

    reader.seek((surface_offset + i64::from(triangles_offset)) as usize)?;
    let mut triangles = Vec::with_capacity(triangle_count as usize);
    for index in 0..triangle_count {
        let triangle_offset = surface_offset + i64::from(triangles_offset) + i64::from(index) * TRIANGLE_SIZE as i64;
        let indices = [reader.i32()?, reader.i32()?, reader.i32()?];
        for vertex in indices {
            if vertex < 0 || vertex >= vertex_count {
                return Err(BinaryError {
                    input: source.to_string(),
                    offset: triangle_offset as usize,
                    message: format!("triangle vertex {vertex} outside 0..{}", vertex_count - 1),
                });
            }
        }
        triangles.push(Md3Triangle {
            indices: [indices[0] as u32, indices[1] as u32, indices[2] as u32],
        });
    }

    reader.seek((surface_offset + i64::from(tex_coords_offset)) as usize)?;
    let mut tex_coords = Vec::with_capacity(vertex_count as usize);
    for _ in 0..vertex_count {
        tex_coords.push(vec2(reader.finite_f32()?, reader.finite_f32()?));
    }

    reader.seek((surface_offset + i64::from(vertices_offset)) as usize)?;
    let mut frames = Vec::with_capacity(frame_count as usize);
    for _ in 0..frame_count {
        let mut vertices = Vec::with_capacity(vertex_count as usize);
        for _ in 0..vertex_count {
            let position = vec3(
                f32::from(reader.i16()?) * XYZ_SCALE,
                f32::from(reader.i16()?) * XYZ_SCALE,
                f32::from(reader.i16()?) * XYZ_SCALE,
            );
            vertices.push(Md3Vertex {
                position,
                normal: decode_md3_normal(reader.u16()?),
            });
        }
        frames.push(vertices);
    }
    Ok((
        Md3Surface {
            name,
            flags,
            shaders,
            triangles,
            tex_coords,
            frames,
        },
        surface_end,
    ))
}

/// Parse a complete little-endian MD3 version 15 model (`parseMd3`).
pub fn parse_md3(data: &[u8], source: &str) -> Result<DecodedMd3Model, BinaryError> {
    let mut reader = BinaryReader::new(data, source);
    let fail = |offset: usize, message: String| BinaryError {
        input: source.to_string(),
        offset,
        message,
    };
    if reader.u32()? != MD3_IDENT {
        return Err(fail(0, "expected IDP3 magic".to_string()));
    }
    if reader.i32()? != MD3_VERSION {
        return Err(fail(4, "expected MD3 version 15".to_string()));
    }
    let name = c_string(&mut reader, 64)?;
    let flags = reader.i32()?;
    let frame_count = md3_count(reader.i32()?, MAX_FRAMES, source, 76, "frame")?;
    let tag_count = md3_count(reader.i32()?, MAX_TAGS, source, 80, "tag")?;
    let surface_count = md3_count(reader.i32()?, MAX_SURFACES, source, 84, "surface")?;
    let skin_count = reader.i32()?;
    let frames_offset = reader.i32()?;
    let tags_offset = reader.i32()?;
    let surfaces_offset = reader.i32()?;
    let model_end = reader.i32()?;
    if frame_count < 1 {
        return Err(fail(76, "MD3 has no frames".to_string()));
    }
    if skin_count < 0 {
        return Err(fail(88, format!("negative skin count {skin_count}")));
    }
    if model_end < HEADER_SIZE as i32 || model_end as usize > data.len() {
        return Err(fail(
            104,
            format!("model end {model_end} outside {}..{}", HEADER_SIZE, data.len()),
        ));
    }
    let model_end = i64::from(model_end);
    let frame_bytes = i64::from(frame_count) * FRAME_SIZE as i64;
    let tag_bytes = i64::from(frame_count) * i64::from(tag_count) * TAG_SIZE as i64;
    md3_range(
        i64::from(frames_offset),
        frame_bytes,
        HEADER_SIZE,
        model_end,
        source,
        92,
        "frames",
    )?;
    md3_range(
        i64::from(tags_offset),
        tag_bytes,
        HEADER_SIZE,
        model_end,
        source,
        96,
        "tags",
    )?;
    md3_range(
        i64::from(surfaces_offset),
        0,
        HEADER_SIZE,
        model_end,
        source,
        100,
        "surfaces",
    )?;

    reader.seek(frames_offset as usize)?;
    let mut frames = Vec::with_capacity(frame_count as usize);
    for _ in 0..frame_count {
        frames.push(read_frame(&mut reader)?);
    }

    reader.seek(tags_offset as usize)?;
    let mut tags = Vec::with_capacity(frame_count as usize);
    for _ in 0..frame_count {
        let mut frame_tags = Vec::with_capacity(tag_count as usize);
        for _ in 0..tag_count {
            frame_tags.push(read_tag(&mut reader)?);
        }
        tags.push(frame_tags);
    }

    let mut surfaces = Vec::with_capacity(surface_count as usize);
    let mut surface_offset = i64::from(surfaces_offset);
    for _ in 0..surface_count {
        let (surface, next) = parse_surface(&mut reader, source, surface_offset, model_end, frame_count)?;
        surfaces.push(surface);
        surface_offset = next;
    }
    if surface_offset != model_end {
        return Err(fail(
            surface_offset as usize,
            format!("surface chain ends at {surface_offset}, model ends at {model_end}"),
        ));
    }
    let byte_length = model_end as usize;
    Ok(DecodedMd3Model {
        source: source.to_string(),
        bytes: data[..byte_length].to_vec(),
        byte_length,
        model: Md3Model {
            name,
            flags,
            skin_count,
            frames,
            tags,
            surfaces,
        },
    })
}

fn frame_at<'a>(surface: &'a Md3Surface, index: i32, name: &str) -> &'a [Md3Vertex] {
    if index < 0 || index as usize >= surface.frames.len() {
        panic!(
            "{name} MD3 frame {index} outside 0..{}",
            surface.frames.len().saturating_sub(1)
        );
    }
    match surface.frames.get(index as usize) {
        Some(frame) => frame,
        None => panic!("missing {name} MD3 frame {index}"),
    }
}

/// Interpolate a surface with the renderer's old-frame backlerp convention
/// (`interpolateSurface`).
///
/// # Panics
///
/// Panics when the backlerp is non-finite or a frame is out of range.
#[must_use]
pub fn interpolate_surface(surface: &Md3Surface, frame: i32, old_frame: i32, back_lerp: f32) -> Vec<Md3Vertex> {
    if !back_lerp.is_finite() {
        panic!("MD3 backlerp must be finite");
    }
    let current = frame_at(surface, frame, "current");
    if back_lerp == 0.0 || frame == old_frame {
        return current.to_vec();
    }
    let previous = frame_at(surface, old_frame, "old");
    interpolate_md3_frames(current, previous, back_lerp)
}

/// Shared interpolation arithmetic (`interpolateMd3Frames`).
///
/// # Panics
///
/// Panics when the poses differ in length.
#[must_use]
pub fn interpolate_md3_frames(current: &[Md3Vertex], previous: &[Md3Vertex], back_lerp: f32) -> Vec<Md3Vertex> {
    let old_scale = back_lerp;
    let new_scale = 1.0 - old_scale;
    let old_xyz_scale = XYZ_SCALE * old_scale;
    let new_xyz_scale = XYZ_SCALE * new_scale;
    let position_component = |old_value: f32, new_value: f32| {
        let old_xyz = old_value * 64.0;
        let new_xyz = new_value * 64.0;
        old_xyz * old_xyz_scale + new_xyz * new_xyz_scale
    };
    let normal_component = |old_value: f32, new_value: f32| old_value * old_scale + new_value * new_scale;
    current
        .iter()
        .enumerate()
        .map(|(index, current_vertex)| {
            let previous_vertex = previous
                .get(index)
                .unwrap_or_else(|| panic!("missing MD3 surface vertex {index}"));
            Md3Vertex {
                position: vec3(
                    position_component(previous_vertex.position.x, current_vertex.position.x),
                    position_component(previous_vertex.position.y, current_vertex.position.y),
                    position_component(previous_vertex.position.z, current_vertex.position.z),
                ),
                normal: normalize_fast3(vec3(
                    normal_component(previous_vertex.normal.x, current_vertex.normal.x),
                    normal_component(previous_vertex.normal.y, current_vertex.normal.y),
                    normal_component(previous_vertex.normal.z, current_vertex.normal.z),
                )),
            }
        })
        .collect()
}

fn tag_at<'a>(model: &'a Md3Model, frame: i32, name: &str) -> Option<&'a Md3Tag> {
    if frame < 0 {
        panic!("MD3 tag frame {frame} must be non-negative");
    }
    if model.frames.is_empty() {
        return None;
    }
    let clamped = (frame as usize).min(model.frames.len() - 1);
    model.tags.get(clamped)?.iter().find(|tag| tag.name == name)
}

/// Interpolate a named tag (`lerpTag`).
///
/// Oversized frame numbers clamp as `R_GetTag` does.
///
/// # Panics
///
/// Panics when a frame number is negative.
#[must_use]
pub fn lerp_tag(model: &Md3Model, name: &str, start_frame: i32, end_frame: i32, fraction: f32) -> Option<Md3Tag> {
    let start = tag_at(model, start_frame, name)?;
    let end = tag_at(model, end_frame, name)?;
    Some(interpolate_md3_tags(start, end, name, fraction))
}

/// `R_LerpTag` arithmetic after tag resolution (`interpolateMd3Tags`).
///
/// # Panics
///
/// Panics when the fraction is non-finite.
#[must_use]
pub fn interpolate_md3_tags(start: &Md3Tag, end: &Md3Tag, name: &str, fraction: f32) -> Md3Tag {
    if !fraction.is_finite() {
        panic!("MD3 tag fraction must be finite");
    }
    let front_lerp = fraction;
    let back_lerp = 1.0 - front_lerp;
    let interpolate = |from: Vec3, to: Vec3| {
        vec3(
            from.x * back_lerp + to.x * front_lerp,
            from.y * back_lerp + to.y * front_lerp,
            from.z * back_lerp + to.z * front_lerp,
        )
    };
    Md3Tag {
        name: name.to_string(),
        origin: interpolate(start.origin, end.origin),
        axes: [
            normalize3(interpolate(start.axes[0], end.axes[0])),
            normalize3(interpolate(start.axes[1], end.axes[1])),
            normalize3(interpolate(start.axes[2], end.axes[2])),
        ],
    }
}

struct SkinTokenizer {
    chars: Vec<char>,
    offset: usize,
}

impl SkinTokenizer {
    fn new(text: &str) -> Result<Self, BinaryError> {
        let terminator = text.find('\0').unwrap_or(text.len());
        let text = &text[..terminator];
        if text.chars().any(|character| character as u32 > 255) {
            return Err(BinaryError {
                input: "<skin>".to_string(),
                offset: 0,
                message: "CommaParse requires source byte text".to_string(),
            });
        }
        Ok(Self {
            chars: text.chars().collect(),
            offset: 0,
        })
    }

    fn skip_comma(&mut self) {
        if self.chars.get(self.offset) == Some(&',') {
            self.offset += 1;
        }
    }

    fn next(&mut self) -> Result<Option<String>, BinaryError> {
        loop {
            while self.offset < self.chars.len() {
                let code = self.chars[self.offset] as u32;
                if code > 32 && code < 128 {
                    break;
                }
                self.offset += 1;
            }
            if self.starts_with("//") {
                while self.offset < self.chars.len() && self.chars[self.offset] != '\n' {
                    self.offset += 1;
                }
                continue;
            }
            if self.starts_with("/*") {
                self.offset += 2;
                while self.offset < self.chars.len() && !self.starts_with("*/") {
                    self.offset += 1;
                }
                if self.starts_with("*/") {
                    self.offset += 2;
                }
                continue;
            }
            break;
        }
        let Some(first) = self.chars.get(self.offset).copied() else {
            return Ok(None);
        };
        if first == '"' {
            self.offset += 1;
            let mut result = String::new();
            while self.offset < self.chars.len() {
                let character = self.chars[self.offset];
                if character == '"' {
                    break;
                }
                result.push(character);
                self.offset += 1;
            }
            if self.chars.get(self.offset) == Some(&'"') {
                self.offset += 1;
            }
            if result.len() >= 1024 {
                return Err(BinaryError {
                    input: "<skin>".to_string(),
                    offset: self.offset,
                    message: "CommaParse quoted token exceeds its source allocation".to_string(),
                });
            }
            return Ok(Some(result));
        }
        // CommaParse always consumes the first byte, even when it is a comma.
        let mut result = String::from(first);
        self.offset += 1;
        while self.offset < self.chars.len() {
            let character = self.chars[self.offset];
            let code = character as u32;
            if character == ',' || code <= 32 || code >= 128 {
                break;
            }
            if result.len() < 1024 {
                result.push(character);
            }
            self.offset += 1;
        }
        if result.len() == 1024 {
            return Ok(Some(String::new()));
        }
        Ok(Some(result))
    }

    fn starts_with(&self, text: &str) -> bool {
        let pattern: Vec<char> = text.chars().collect();
        self.chars[self.offset..].starts_with(&pattern)
    }
}

/// Comma-separated skin mappings (`iterateSkinSurfaces`).
pub struct SkinSurfaceIter {
    tokenizer: SkinTokenizer,
}

impl SkinSurfaceIter {
    fn next_surface(&mut self) -> Result<Option<SkinSurface>, BinaryError> {
        loop {
            let name = self.tokenizer.next()?;
            let Some(name) = name else { return Ok(None) };
            if name.is_empty() {
                return Ok(None);
            }
            self.tokenizer.skip_comma();
            if name.contains("tag_") {
                continue;
            }
            let shader = self.tokenizer.next()?.unwrap_or_default();
            let short: String = name.chars().take(63).collect();
            return Ok(Some(SkinSurface {
                name: short.to_ascii_lowercase(),
                shader,
            }));
        }
    }
}

impl Iterator for SkinSurfaceIter {
    type Item = Result<SkinSurface, BinaryError>;

    fn next(&mut self) -> Option<Self::Item> {
        self.next_surface().transpose()
    }
}

/// Iterate the renderer's comma-separated skin mappings
/// (`iterateSkinSurfaces`).
pub fn iterate_skin_surfaces(text: &str) -> Result<SkinSurfaceIter, BinaryError> {
    Ok(SkinSurfaceIter {
        tokenizer: SkinTokenizer::new(text)?,
    })
}

/// Parse the renderer's skin mappings (`parseSkin`).
pub fn parse_skin(text: &str) -> Result<Vec<SkinSurface>, BinaryError> {
    iterate_skin_surfaces(text)?.collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_core::binary::BinaryWriter;

    fn fixture() -> Vec<u8> {
        // 1 frame, 1 tag, 1 surface (1 shader, 1 triangle, 3 vertices).
        let mut writer = BinaryWriter::new(1024);
        writer.u32(MD3_IDENT).unwrap();
        writer.i32(MD3_VERSION).unwrap();
        let mut name = [0u8; 64];
        name[..4].copy_from_slice(b"test");
        writer.bytes(&name).unwrap();
        writer.i32(0).unwrap(); // flags
        writer.i32(1).unwrap(); // frames
        writer.i32(1).unwrap(); // tags
        writer.i32(1).unwrap(); // surfaces
        writer.i32(0).unwrap(); // skins
        writer.i32(108).unwrap(); // frames offset
        writer.i32(164).unwrap(); // tags offset
        writer.i32(276).unwrap(); // surfaces offset
        let surface_length = 108 + 68 + 12 + 24 + 24;
        writer.i32(276 + surface_length).unwrap(); // end
        for value in [-1.0f32, -1.0, -1.0, 1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 2.0] {
            writer.f32(value).unwrap();
        }
        let mut frame_name = [0u8; 16];
        frame_name[..6].copy_from_slice(b"frame0");
        writer.bytes(&frame_name).unwrap();
        let mut tag_name = [0u8; 64];
        tag_name[..9].copy_from_slice(b"tag_torso");
        writer.bytes(&tag_name).unwrap();
        for value in [1.0f32, 2.0, 3.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0] {
            writer.f32(value).unwrap();
        }
        writer.u32(MD3_IDENT).unwrap();
        let mut surface_name = [0u8; 64];
        surface_name[..6].copy_from_slice(b"head_1");
        writer.bytes(&surface_name).unwrap();
        writer.i32(0).unwrap();
        writer.i32(1).unwrap();
        writer.i32(1).unwrap();
        writer.i32(3).unwrap();
        writer.i32(1).unwrap();
        writer.i32(108 + 68).unwrap(); // triangles
        writer.i32(108).unwrap(); // shaders
        writer.i32(108 + 68 + 12).unwrap(); // tex coords
        writer.i32(108 + 68 + 12 + 24).unwrap(); // vertices
        writer.i32(surface_length).unwrap();
        let mut shader = [0u8; 64];
        shader[..7].copy_from_slice(b"shader0");
        writer.bytes(&shader).unwrap();
        writer.i32(0).unwrap();
        for index in [0i32, 1, 2] {
            writer.i32(index).unwrap();
        }
        for uv in [[0.0f32, 0.0], [1.0, 0.0], [0.0, 1.0]] {
            writer.f32(uv[0]).unwrap();
            writer.f32(uv[1]).unwrap();
        }
        for position in [[0i16, 0, 0], [64, 0, 0], [0, 64, 0]] {
            writer.i16(position[0]).unwrap();
            writer.i16(position[1]).unwrap();
            writer.i16(position[2]).unwrap();
            writer.u16(0).unwrap();
        }
        writer.finish()
    }

    #[test]
    fn md3_round_trip() {
        let decoded = parse_md3(&fixture(), "<test>").unwrap();
        assert_eq!(decoded.model.name, "test");
        assert_eq!(decoded.model.frames.len(), 1);
        assert_eq!(decoded.model.frames[0].name, "frame0");
        assert_eq!(decoded.model.tags[0][0].name, "tag_torso");
        let surface = &decoded.model.surfaces[0];
        assert_eq!(surface.name, "head");
        assert_eq!(surface.shaders[0].name, "shader0");
        assert_eq!(surface.triangles[0].indices, [0, 1, 2]);
        assert_eq!(surface.tex_coords[1], vec2(1.0, 0.0));
        assert_eq!(surface.frames[0][1].position, vec3(1.0, 0.0, 0.0));
        assert_eq!(decoded.byte_length, decoded.bytes.len());

        let tag = lerp_tag(&decoded.model, "tag_torso", 0, 0, 0.5).unwrap();
        assert_eq!(tag.origin, vec3(1.0, 2.0, 3.0));
        assert!(lerp_tag(&decoded.model, "missing", 0, 0, 0.0).is_none());
        let interpolated = interpolate_surface(surface, 0, 0, 0.5);
        assert_eq!(interpolated[1].position, vec3(1.0, 0.0, 0.0));
    }

    #[test]
    fn md3_normal_table() {
        // Packed zero points along +Z through the sine table.
        let normal = decode_md3_normal(0);
        assert!((normal.z - 1.0).abs() < 0.01, "{normal:?}");
        assert!(normal.x.abs() < 0.01 && normal.y.abs() < 0.01);
        // The fast normalize matches the single-iteration Q_rsqrt path.
        let fast = normalize_fast3(vec3(3.0, 0.0, 4.0));
        assert!((fast.x - 0.6).abs() < 0.002, "{fast:?}");
        assert!((fast.z - 0.8).abs() < 0.002, "{fast:?}");
    }

    #[test]
    fn md3_skins() {
        let surfaces = parse_skin("Head,skins/head.tga\n\"Legs\",\"skins/legs.tga\"\n").unwrap();
        assert_eq!(surfaces.len(), 2);
        assert_eq!(
            surfaces[0],
            SkinSurface {
                name: "head".to_string(),
                shader: "skins/head.tga".to_string()
            }
        );
        assert_eq!(
            surfaces[1],
            SkinSurface {
                name: "legs".to_string(),
                shader: "skins/legs.tga".to_string()
            }
        );
        // A skipped tag_ line leaves its shader token to be read as a name.
        let surfaces = parse_skin("tag_head,foo\n").unwrap();
        assert_eq!(
            surfaces,
            vec![SkinSurface {
                name: "foo".to_string(),
                shader: String::new()
            }]
        );
        assert!(parse_skin("head,\u{100}").is_err());
    }

    #[test]
    fn md3_rejects_bad_input() {
        let good = fixture();
        let mut bad_magic = good.clone();
        bad_magic[0] = b'X';
        let error = parse_md3(&bad_magic, "<test>").unwrap_err();
        assert_eq!(error.message, "expected IDP3 magic");
        let mut bad_version = good.clone();
        bad_version[4] = 14;
        let error = parse_md3(&bad_version, "<test>").unwrap_err();
        assert_eq!(error.message, "expected MD3 version 15");
        assert!(parse_md3(&good[..100], "<test>").is_err());
        // Triangle vertex past the vertex count.
        let mut bad_triangle = good.clone();
        let triangle_at = 276 + 108 + 68;
        bad_triangle[triangle_at] = 9;
        let error = parse_md3(&bad_triangle, "<test>").unwrap_err();
        assert!(error.message.contains("triangle vertex"), "{}", error.message);
    }
}
