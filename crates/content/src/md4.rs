//! Quake III skeletal model (MD4 v1) parser and skinning.
//!
//! Donor provenance: `src/formats/q3-model/md4.ts` (loading from
//! `qfiles.h` and `tr_model.c`, bone indexing from `tr_animation.c`,
//! skinning from `RB_SurfaceAnim`). The dot-product order matches
//! [`qa_core::math::dot3`], which this port reuses.

use qa_core::binary::{BinaryError, BinaryReader};
use qa_core::math::{dot3, vec2, vec3, vec4, Bounds, Vec2, Vec3, Vec4};

use crate::model_text::at;

/// MD4 magic (`0x34504449`, little-endian `"IDP4"`).
pub const MD4_IDENT: u32 = 0x3450_4449;
/// MD4 version.
pub const MD4_VERSION: i32 = 1;

const HEADER_SIZE: i64 = 100;
const SURFACE_SIZE: i64 = 168;
const LOD_SIZE: i64 = 12;

/// MD4 bone (`Md4Bone`): three rotation rows plus translation.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Md4Bone {
    /// Matrix rows.
    pub matrix: [Vec4; 3],
}

/// MD4 frame (`Md4Frame`).
#[derive(Debug, Clone, PartialEq)]
pub struct Md4Frame {
    /// Bounds.
    pub bounds: Bounds,
    /// Local origin.
    pub local_origin: Vec3,
    /// Radius.
    pub radius: f32,
    /// Bones.
    pub bones: Vec<Md4Bone>,
}

/// MD4 vertex weight (`Md4Weight`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Md4Weight {
    /// Bone index.
    pub bone_index: u32,
    /// Bone weight.
    pub bone_weight: f32,
    /// Offset.
    pub offset: Vec3,
}

/// MD4 vertex (`Md4Vertex`).
#[derive(Debug, Clone, PartialEq)]
pub struct Md4Vertex {
    /// Normal.
    pub normal: Vec3,
    /// Texture coordinates.
    pub tex_coords: Vec2,
    /// Weights.
    pub weights: Vec<Md4Weight>,
}

/// MD4 triangle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Md4Triangle {
    /// Vertex indices.
    pub indices: [u32; 3],
}

/// MD4 surface (`Md4Surface`).
#[derive(Debug, Clone, PartialEq)]
pub struct Md4Surface {
    /// Name.
    pub name: String,
    /// Shader name.
    pub shader: String,
    /// Vertices.
    pub vertices: Vec<Md4Vertex>,
    /// Triangles.
    pub triangles: Vec<Md4Triangle>,
    /// Bone references.
    pub bone_references: Vec<u32>,
}

/// MD4 level of detail.
#[derive(Debug, Clone, PartialEq)]
pub struct Md4Lod {
    /// Surfaces.
    pub surfaces: Vec<Md4Surface>,
}

/// MD4 model (`Md4Model`).
#[derive(Debug, Clone, PartialEq)]
pub struct Md4Model {
    /// Version.
    pub version: i32,
    /// Name.
    pub name: String,
    /// Bone count.
    pub num_bones: u32,
    /// Model length.
    pub byte_length: usize,
    /// Frames.
    pub frames: Vec<Md4Frame>,
    /// Levels of detail.
    pub lods: Vec<Md4Lod>,
}

/// Decoded MD4 model with its source bytes (`DecodedMd4Model`).
#[derive(Debug, Clone, PartialEq)]
pub struct DecodedMd4Model {
    /// Source name.
    pub source: String,
    /// Model bytes through `byte_length`.
    pub bytes: Vec<u8>,
    /// Bone names, when the unused table is present.
    pub bone_names: Option<Vec<String>>,
    /// Model records.
    pub model: Md4Model,
}

/// Skinned MD4 vertex.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Md4SkinnedVertex {
    /// Position.
    pub position: Vec3,
    /// Normal.
    pub normal: Vec3,
}

fn md4_fail(source: &str, offset: usize, message: String) -> BinaryError {
    BinaryError {
        input: source.to_string(),
        offset,
        message,
    }
}

fn md4_range(source: &str, offset: i64, length: i64, start: i64, end: i64, field: usize) -> Result<(), BinaryError> {
    if offset < start || length < 0 || offset > end - length {
        return Err(md4_fail(
            source,
            field,
            format!("MD4 range {offset}+{length} exceeds {start}..{end}"),
        ));
    }
    Ok(())
}

fn md4_count(reader: &mut BinaryReader<'_>, source: &str, maximum: i32) -> Result<i32, BinaryError> {
    let offset = reader.offset();
    let value = reader.i32()?;
    if value < 0 || value > maximum {
        return Err(md4_fail(
            source,
            offset,
            format!("MD4 count {value} outside 0..{maximum}"),
        ));
    }
    Ok(value)
}

fn md4_index(reader: &mut BinaryReader<'_>, source: &str, maximum: i32, label: &str) -> Result<u32, BinaryError> {
    let offset = reader.offset();
    let value = reader.i32()?;
    if value < 0 || value >= maximum {
        return Err(md4_fail(
            source,
            offset,
            format!("{label} {value} outside 0..{}", maximum - 1),
        ));
    }
    Ok(value as u32)
}

fn md4_string(reader: &mut BinaryReader<'_>) -> Result<String, BinaryError> {
    let bytes = reader.bytes(64)?;
    let mut result = String::new();
    for byte in bytes {
        if byte == 0 {
            break;
        }
        result.push(char::from(byte));
    }
    Ok(result)
}

fn md4_vec3(reader: &mut BinaryReader<'_>) -> Result<Vec3, BinaryError> {
    Ok(vec3(reader.finite_f32()?, reader.finite_f32()?, reader.finite_f32()?))
}

fn md4_row(reader: &mut BinaryReader<'_>) -> Result<Vec4, BinaryError> {
    Ok(vec4(
        reader.finite_f32()?,
        reader.finite_f32()?,
        reader.finite_f32()?,
        reader.finite_f32()?,
    ))
}

fn parse_surface(
    reader: &mut BinaryReader<'_>,
    source: &str,
    surface_offset: i64,
    lod_end: i64,
    num_bones: i32,
) -> Result<(Md4Surface, i64), BinaryError> {
    md4_range(
        source,
        surface_offset,
        SURFACE_SIZE,
        HEADER_SIZE,
        lod_end,
        surface_offset as usize,
    )?;
    reader.seek(surface_offset as usize)?;
    reader.i32()?; // R_LoadMD4 replaces the disk ident with SF_MD4 without checking it.
    let name = md4_string(reader)?.to_ascii_lowercase();
    let shader = md4_string(reader)?;
    reader.i32()?; // Replaced by actual shader registration, outside the binary reader.
    let ofs_header = reader.i32()?;
    if surface_offset + i64::from(ofs_header) != 0 {
        return Err(md4_fail(
            source,
            (surface_offset + 136) as usize,
            "MD4 surface does not point back to its model header".to_string(),
        ));
    }
    let num_verts = md4_count(reader, source, 1000)?;
    let ofs_verts = reader.i32()?;
    let num_triangles = md4_count(reader, source, 2000)?;
    let ofs_triangles = reader.i32()?;
    let num_bone_references = md4_count(reader, source, i32::MAX)?;
    let ofs_bone_references = reader.i32()?;
    let ofs_end = reader.i32()?;
    md4_range(
        source,
        surface_offset,
        i64::from(ofs_end),
        surface_offset,
        lod_end,
        (surface_offset + 164) as usize,
    )?;
    if ofs_end < SURFACE_SIZE as i32 {
        return Err(md4_fail(
            source,
            (surface_offset + 164) as usize,
            "MD4 surface end precedes its header".to_string(),
        ));
    }
    let end = surface_offset + i64::from(ofs_end);
    if num_verts > 0 {
        md4_range(
            source,
            i64::from(ofs_verts),
            i64::from(num_verts) * 24,
            SURFACE_SIZE,
            i64::from(ofs_end),
            (surface_offset + 144) as usize,
        )?;
    }
    if num_triangles > 0 {
        md4_range(
            source,
            i64::from(ofs_triangles),
            i64::from(num_triangles) * 12,
            SURFACE_SIZE,
            i64::from(ofs_end),
            (surface_offset + 152) as usize,
        )?;
    }
    if num_bone_references > 0 {
        md4_range(
            source,
            i64::from(ofs_bone_references),
            i64::from(num_bone_references) * 4,
            SURFACE_SIZE,
            i64::from(ofs_end),
            (surface_offset + 160) as usize,
        )?;
    }

    let mut triangles = Vec::with_capacity(num_triangles as usize);
    if num_triangles > 0 {
        reader.seek((surface_offset + i64::from(ofs_triangles)) as usize)?;
    }
    for _ in 0..num_triangles {
        triangles.push(Md4Triangle {
            indices: [
                md4_index(reader, source, num_verts, "triangle vertex")?,
                md4_index(reader, source, num_verts, "triangle vertex")?,
                md4_index(reader, source, num_verts, "triangle vertex")?,
            ],
        });
    }
    let mut bone_references = Vec::with_capacity(num_bone_references as usize);
    if num_bone_references > 0 {
        reader.seek((surface_offset + i64::from(ofs_bone_references)) as usize)?;
    }
    for _ in 0..num_bone_references {
        bone_references.push(md4_index(reader, source, num_bones, "bone reference")?);
    }

    let mut vertices = Vec::with_capacity(num_verts as usize);
    if num_verts > 0 {
        reader.seek((surface_offset + i64::from(ofs_verts)) as usize)?;
    }
    for _ in 0..num_verts {
        md4_range(
            source,
            reader.offset() as i64,
            24,
            surface_offset + SURFACE_SIZE,
            end,
            reader.offset(),
        )?;
        let normal = md4_vec3(reader)?;
        let tex_coords = vec2(reader.finite_f32()?, reader.finite_f32()?);
        let num_weights = md4_count(reader, source, i32::MAX)?;
        md4_range(
            source,
            reader.offset() as i64,
            i64::from(num_weights) * 20,
            surface_offset + SURFACE_SIZE,
            end,
            reader.offset() - 4,
        )?;
        let mut weights = Vec::with_capacity(num_weights as usize);
        for _ in 0..num_weights {
            weights.push(Md4Weight {
                bone_index: md4_index(reader, source, num_bones, "weight bone")?,
                bone_weight: reader.finite_f32()?,
                offset: md4_vec3(reader)?,
            });
        }
        vertices.push(Md4Vertex {
            normal,
            tex_coords,
            weights,
        });
    }
    Ok((
        Md4Surface {
            name,
            shader,
            vertices,
            triangles,
            bone_references,
        },
        end,
    ))
}

/// Decode owned MD4 records (`parseMd4`).
///
/// Bytes after `ofsEnd` and unused bone-name metadata are ignored by the
/// source loader.
pub fn parse_md4(data: &[u8], source: &str) -> Result<DecodedMd4Model, BinaryError> {
    let mut reader = BinaryReader::new(data, source);
    md4_range(source, 0, HEADER_SIZE, 0, reader.length() as i64, 0)?;
    if reader.u32()? != MD4_IDENT {
        return Err(md4_fail(source, 0, "expected IDP4 model magic".to_string()));
    }
    let version = reader.i32()?;
    if version != MD4_VERSION {
        return Err(md4_fail(source, 4, format!("unsupported MD4 version {version}")));
    }
    let name = md4_string(&mut reader)?;
    let num_frames = md4_count(&mut reader, source, i32::MAX)?;
    if num_frames == 0 {
        return Err(md4_fail(source, 72, "MD4 model has no frames".to_string()));
    }
    let num_bones = md4_count(&mut reader, source, 128)?;
    let ofs_bone_names = reader.i32()?;
    let ofs_frames = reader.i32()?;
    let num_lods = md4_count(&mut reader, source, i32::MAX)?;
    let ofs_lods = reader.i32()?;
    let ofs_end = reader.i32()?;
    md4_range(source, 0, i64::from(ofs_end), 0, reader.length() as i64, 96)?;
    if ofs_end < HEADER_SIZE as i32 {
        return Err(md4_fail(source, 96, "MD4 model end precedes its header".to_string()));
    }
    let frame_size = 40 + i64::from(num_bones) * 48;
    md4_range(
        source,
        i64::from(ofs_frames),
        i64::from(num_frames) * frame_size,
        HEADER_SIZE,
        i64::from(ofs_end),
        84,
    )?;
    if num_lods > 0 {
        md4_range(
            source,
            i64::from(ofs_lods),
            i64::from(num_lods) * LOD_SIZE,
            HEADER_SIZE,
            i64::from(ofs_end),
            92,
        )?;
    }

    reader.seek(ofs_frames as usize)?;
    let mut frames = Vec::with_capacity(num_frames as usize);
    for _ in 0..num_frames {
        let bounds = Bounds {
            min: md4_vec3(&mut reader)?,
            max: md4_vec3(&mut reader)?,
        };
        let local_origin = md4_vec3(&mut reader)?;
        let radius = reader.finite_f32()?;
        let mut bones = Vec::with_capacity(num_bones as usize);
        for _ in 0..num_bones {
            bones.push(Md4Bone {
                matrix: [md4_row(&mut reader)?, md4_row(&mut reader)?, md4_row(&mut reader)?],
            });
        }
        frames.push(Md4Frame {
            bounds,
            local_origin,
            radius,
            bones,
        });
    }

    let mut lods = Vec::with_capacity(num_lods as usize);
    let mut lod_offset = i64::from(ofs_lods);
    for _ in 0..num_lods {
        md4_range(
            source,
            lod_offset,
            LOD_SIZE,
            HEADER_SIZE,
            i64::from(ofs_end),
            lod_offset as usize,
        )?;
        reader.seek(lod_offset as usize)?;
        let num_surfaces = md4_count(&mut reader, source, i32::MAX)?;
        let ofs_surfaces = reader.i32()?;
        let lod_length = reader.i32()?;
        md4_range(
            source,
            lod_offset,
            i64::from(lod_length),
            lod_offset,
            i64::from(ofs_end),
            (lod_offset + 8) as usize,
        )?;
        if lod_length < LOD_SIZE as i32 {
            return Err(md4_fail(
                source,
                (lod_offset + 8) as usize,
                "MD4 LOD end precedes its header".to_string(),
            ));
        }
        if num_surfaces > 0 {
            md4_range(
                source,
                i64::from(ofs_surfaces),
                i64::from(num_surfaces) * SURFACE_SIZE,
                LOD_SIZE,
                i64::from(lod_length),
                (lod_offset + 4) as usize,
            )?;
        }
        let mut surfaces = Vec::with_capacity(num_surfaces as usize);
        let mut surface_offset = lod_offset + i64::from(ofs_surfaces);
        for _ in 0..num_surfaces {
            let (surface, next) = parse_surface(
                &mut reader,
                source,
                surface_offset,
                lod_offset + i64::from(lod_length),
                num_bones,
            )?;
            surfaces.push(surface);
            surface_offset = next;
        }
        lods.push(Md4Lod { surfaces });
        lod_offset += i64::from(lod_length);
    }
    let mut bone_names = None;
    // The renderer ignores this field. Decode names only when the unused table is present.
    if i64::from(ofs_bone_names) >= HEADER_SIZE
        && i64::from(ofs_bone_names) <= i64::from(ofs_end) - i64::from(num_bones) * 64
    {
        reader.seek(ofs_bone_names as usize)?;
        let mut names = Vec::with_capacity(num_bones as usize);
        for _ in 0..num_bones {
            names.push(md4_string(&mut reader)?);
        }
        bone_names = Some(names);
    }
    let byte_length = ofs_end as usize;
    Ok(DecodedMd4Model {
        source: source.to_string(),
        bytes: data[..byte_length].to_vec(),
        bone_names,
        model: Md4Model {
            version,
            name,
            num_bones: num_bones as u32,
            byte_length,
            frames,
            lods,
        },
    })
}

/// Skin one MD4 surface (`skinMd4Surface`).
///
/// `RB_SurfaceAnim` uses global bone indices, including when
/// `bone_references` is sparse.
///
/// # Panics
///
/// Panics on invalid frame selections or short bone lists.
#[must_use]
pub fn skin_md4_surface(
    model: &Md4Model,
    surface: &Md4Surface,
    frame: i32,
    previous_frame: i32,
    back_lerp: f32,
) -> Vec<Md4SkinnedVertex> {
    if frame < 0 || previous_frame < 0 || !back_lerp.is_finite() {
        panic!("Invalid MD4 frame selection");
    }
    let current = at(&model.frames, frame as usize, "MD4 frame");
    let back = if frame == previous_frame { 0.0 } else { back_lerp };
    let old = if back == 0.0 {
        current
    } else {
        at(&model.frames, previous_frame as usize, "MD4 previous frame")
    };
    let front = 1.0 - back;
    let blend = |a: f32, b: f32| if back == 0.0 { a } else { front * a + back * b };
    let bones: Vec<Md4Bone> = current
        .bones
        .iter()
        .enumerate()
        .map(|(index, bone)| {
            let before = at(&old.bones, index, "MD4 bone");
            let row = |a: Vec4, b: Vec4| vec4(blend(a.x, b.x), blend(a.y, b.y), blend(a.z, b.z), blend(a.w, b.w));
            Md4Bone {
                matrix: [
                    row(bone.matrix[0], before.matrix[0]),
                    row(bone.matrix[1], before.matrix[1]),
                    row(bone.matrix[2], before.matrix[2]),
                ],
            }
        })
        .collect();
    let dot = |row: Vec4, value: Vec3| dot3(vec3(row.x, row.y, row.z), value);
    surface
        .vertices
        .iter()
        .map(|vertex| {
            let (mut x, mut y, mut z) = (0.0f32, 0.0f32, 0.0f32);
            let (mut nx, mut ny, mut nz) = (0.0f32, 0.0f32, 0.0f32);
            for weight in &vertex.weights {
                let bone = at(&bones, weight.bone_index as usize, "MD4 weight bone");
                let (a, b, c) = (bone.matrix[0], bone.matrix[1], bone.matrix[2]);
                x += weight.bone_weight * (dot(a, weight.offset) + a.w);
                y += weight.bone_weight * (dot(b, weight.offset) + b.w);
                z += weight.bone_weight * (dot(c, weight.offset) + c.w);
                nx += weight.bone_weight * dot(a, vertex.normal);
                ny += weight.bone_weight * dot(b, vertex.normal);
                nz += weight.bone_weight * dot(c, vertex.normal);
            }
            Md4SkinnedVertex {
                position: vec3(x, y, z),
                normal: vec3(nx, ny, nz),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_core::binary::BinaryWriter;

    fn fixture() -> Vec<u8> {
        let mut writer = BinaryWriter::new(512);
        writer.u32(MD4_IDENT).unwrap();
        writer.i32(MD4_VERSION).unwrap();
        let mut name = [0u8; 64];
        name[..4].copy_from_slice(b"test");
        writer.bytes(&name).unwrap();
        writer.i32(1).unwrap(); // frames
        writer.i32(1).unwrap(); // bones
        writer.i32(0).unwrap(); // bone names
        writer.i32(100).unwrap(); // frames
        writer.i32(1).unwrap(); // LODs
        writer.i32(188).unwrap(); // LOD offset
        writer.i32(428).unwrap(); // end
        for value in [-1.0f32, -1.0, -1.0, 1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 2.0] {
            writer.f32(value).unwrap();
        }
        for row in [[1.0f32, 0.0, 0.0, 10.0], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0]] {
            for component in row {
                writer.f32(component).unwrap();
            }
        }
        writer.i32(1).unwrap(); // surfaces
        writer.i32(12).unwrap(); // surfaces offset
        writer.i32(240).unwrap(); // LOD length
        writer.u32(MD4_IDENT).unwrap();
        let mut surface_name = [0u8; 64];
        surface_name[..5].copy_from_slice(b"Lower");
        writer.bytes(&surface_name).unwrap();
        let mut shader = [0u8; 64];
        shader[..7].copy_from_slice(b"shader0");
        writer.bytes(&shader).unwrap();
        writer.i32(0).unwrap();
        writer.i32(-200).unwrap();
        writer.i32(1).unwrap();
        writer.i32(168).unwrap();
        writer.i32(1).unwrap();
        writer.i32(212).unwrap();
        writer.i32(1).unwrap();
        writer.i32(224).unwrap();
        writer.i32(228).unwrap();
        for value in [0.0f32, 0.0, 1.0, 0.0, 0.0] {
            writer.f32(value).unwrap();
        }
        writer.i32(1).unwrap();
        writer.i32(0).unwrap();
        writer.f32(1.0).unwrap();
        for value in [1.0f32, 2.0, 3.0] {
            writer.f32(value).unwrap();
        }
        for index in [0i32, 0, 0] {
            writer.i32(index).unwrap();
        }
        writer.i32(0).unwrap();
        writer.finish()
    }

    #[test]
    fn md4_round_trip() {
        let decoded = parse_md4(&fixture(), "<test>").unwrap();
        assert_eq!(decoded.model.name, "test");
        assert_eq!(decoded.model.num_bones, 1);
        assert_eq!(decoded.model.frames.len(), 1);
        assert!(decoded.bone_names.is_none());
        let surface = &decoded.model.lods[0].surfaces[0];
        assert_eq!(surface.name, "lower");
        assert_eq!(surface.shader, "shader0");
        assert_eq!(surface.triangles[0].indices, [0, 0, 0]);
        assert_eq!(surface.bone_references, vec![0]);
        let skinned = skin_md4_surface(&decoded.model, surface, 0, 0, 0.0);
        assert_eq!(skinned.len(), 1);
        assert_eq!(skinned[0].position, vec3(11.0, 2.0, 3.0));
        assert_eq!(skinned[0].normal, vec3(0.0, 0.0, 1.0));
    }

    #[test]
    fn md4_rejects_bad_input() {
        let good = fixture();
        let mut bad_magic = good.clone();
        bad_magic[0] = b'X';
        let error = parse_md4(&bad_magic, "<test>").unwrap_err();
        assert_eq!(error.message, "expected IDP4 model magic");
        let mut bad_version = good.clone();
        bad_version[4] = 2;
        let error = parse_md4(&bad_version, "<test>").unwrap_err();
        assert!(error.message.contains("unsupported MD4 version"), "{}", error.message);
        assert!(parse_md4(&good[..99], "<test>").is_err());
        // Triangle vertex past the vertex count.
        let mut bad_triangle = good.clone();
        bad_triangle[200 + 212] = 5;
        let error = parse_md4(&bad_triangle, "<test>").unwrap_err();
        assert!(error.message.contains("triangle vertex"), "{}", error.message);
        // Surface header back-pointer broken.
        let mut bad_header = good.clone();
        bad_header[200 + 136] = 1;
        let error = parse_md4(&bad_header, "<test>").unwrap_err();
        assert!(error.message.contains("does not point back"), "{}", error.message);
    }
}
