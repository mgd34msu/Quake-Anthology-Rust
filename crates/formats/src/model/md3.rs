use super::{read::*, *};
use qa_core::math::{angle_vectors_radians, radians_from_degrees_f32};

pub(super) fn load<'a>(bytes: &'a [u8], mut m: Model<'a>) -> Result<Model<'a>, FormatError> {
    let compressed = m.format == ModelFormat::Mdc;
    let header = if compressed { 112 } else { 108 };
    let mut r = Reader::new(bytes);
    r.take(4)?;
    if r.i32()? != if compressed { 2 } else { 15 } {
        return Err(FormatError::Unsupported);
    }
    m.name = r.name(64)?;
    m.flags = r.i32()?;
    let frames = r.count(1, 1024)?;
    let tags = r.count(0, 16)?;
    let surfaces = r.count(0, 32)?;
    m.declared_skins = r.count(0, i32::MAX as usize)? as u32;
    let frame_at = r.count(header, i32::MAX as usize)?;
    let tag_names_at = if compressed {
        r.count(header, i32::MAX as usize)?
    } else {
        header
    };
    let tag_at = r.count(header, i32::MAX as usize)?;
    let mut surface_at = r.count(header, i32::MAX as usize)?;
    let end = r.count(header, i32::MAX as usize)?;
    let frame_data = r.section(frame_at, frames, 56, header, end)?;
    let tag_data = r.section(
        tag_at,
        frames * tags,
        if compressed { 12 } else { 112 },
        header,
        end,
    )?;
    let tag_names = r.section(
        tag_names_at,
        if compressed { tags } else { 0 },
        64,
        header,
        end,
    )?;
    r.section(
        surface_at,
        surfaces,
        if compressed { 124 } else { 108 },
        header,
        end,
    )?;
    reserve(&mut m.frames, frames)?;
    for data in frame_data.as_chunks::<56>().0 {
        let mut s = Reader::new(data);
        let mut bounds = Bounds {
            mins: s.vector()?,
            maxs: s.vector()?,
        };
        if (0..3).any(|a| bounds.mins.0[a] > bounds.maxs.0[a]) {
            // Retail *_hand files contain tags and the exporter's empty-box
            // sentinel. There is no geometry to bound in those models.
            if surfaces != 0 {
                return Err(FormatError::InvalidRange);
            }
            bounds = Bounds::default();
        }
        let origin = s.vector()?;
        let radius = s.float()?;
        if radius < 0.0 {
            return Err(FormatError::InvalidValue);
        }
        let name = s.name(16)?;
        m.bounds.add_bounds(bounds);
        m.frames.push(Frame {
            bounds,
            origin,
            radius,
            name,
            ..Frame::default()
        });
    }
    m.tags_per_frame = tags;
    reserve(&mut m.tags, frames * tags)?;
    for (index, data) in tag_data
        .chunks_exact(if compressed { 12 } else { 112 })
        .enumerate()
    {
        let mut s = Reader::new(data);
        let (name, origin, axes) = if compressed {
            let name = Reader::new(&tag_names[(index % tags) * 64..]).name(64)?;
            let origin = Vec3([
                f32::from(s.i16()?) / 64.0,
                f32::from(s.i16()?) / 64.0,
                f32::from(s.i16()?) / 64.0,
            ]);
            let angles = Vec3(
                [s.i16()? as f32, s.i16()? as f32, s.i16()? as f32]
                    .map(|v| (f64::from(v) * (360.0 / 32700.0)) as f32),
            );
            let basis = angle_vectors_radians(radians_from_degrees_f32(angles));
            (name, origin, [basis.forward, -basis.right, basis.up])
        } else {
            (
                s.name(64)?,
                s.vector()?,
                [s.vector()?, s.vector()?, s.vector()?],
            )
        };
        m.tags.push(Tag { name, origin, axes });
    }
    let mut expanded_vertices = 0usize;
    for _ in 0..surfaces {
        let surface_header = if compressed { 124 } else { 108 };
        r.section(surface_at, 1, surface_header, header, end)?;
        let mut s = Reader::new(&bytes[surface_at..end]);
        if s.take(4)? != if compressed { b"IDPC" } else { b"IDP3" } {
            return Err(FormatError::InvalidValue);
        }
        let name = s.name(64)?;
        let flags = s.i32()?;
        let comp_frames = if compressed { s.count(0, frames)? } else { 0 };
        let base_frames = s.count(1, frames)?;
        if !compressed && base_frames != frames {
            return Err(FormatError::InvalidValue);
        }
        let shaders = s.count(0, 256)?;
        let vertices = s.count(0, 4096)?;
        let triangles = s.count(0, 8192)?;
        let tri_at = s.count(surface_header, i32::MAX as usize)?;
        let shader_at = s.count(surface_header, i32::MAX as usize)?;
        let uv_at = s.count(surface_header, i32::MAX as usize)?;
        let vert_at = s.count(surface_header, i32::MAX as usize)?;
        let comp_at = if compressed {
            s.count(surface_header, i32::MAX as usize)?
        } else {
            surface_header
        };
        let base_index_at = if compressed {
            s.count(surface_header, i32::MAX as usize)?
        } else {
            surface_header
        };
        let comp_index_at = if compressed {
            s.count(surface_header, i32::MAX as usize)?
        } else {
            surface_header
        };
        let length = s.count(surface_header, i32::MAX as usize)?;
        r.section(surface_at, length, 1, header, end)?;
        let tri_data = s.section(tri_at, triangles, 12, surface_header, length)?;
        let shader_data = s.section(shader_at, shaders, 68, surface_header, length)?;
        let uv_data = s.section(uv_at, vertices, 8, surface_header, length)?;
        let vert_data = s.section(vert_at, base_frames * vertices, 8, surface_header, length)?;
        let comp_data = s.section(comp_at, comp_frames * vertices, 4, surface_header, length)?;
        let base_indices = s.section(
            base_index_at,
            if compressed { frames } else { 0 },
            2,
            surface_header,
            length,
        )?;
        let comp_indices = s.section(
            comp_index_at,
            if compressed { frames } else { 0 },
            2,
            surface_header,
            length,
        )?;
        expanded_vertices += frames * vertices;
        // Compressed inputs can expand many times. Bound the whole asset before
        // allocating frame arrays; ordinary models remain below this limit.
        if expanded_vertices > MAX_DECODED_BYTES / std::mem::size_of::<Vertex>() {
            return Err(FormatError::InvalidRange);
        }
        let mut mesh = Mesh {
            name,
            flags,
            vertices_per_frame: vertices,
            ..Mesh::default()
        };
        reserve(&mut mesh.triangles, triangles)?;
        for data in tri_data.as_chunks::<12>().0 {
            let mut t = Reader::new(data);
            let mut indices = [0; 3];
            for index in &mut indices {
                *index = bounded_index(t.count(0, i32::MAX as usize)?, vertices)?;
            }
            mesh.triangles.push(Triangle {
                vertex: indices,
                texcoord: indices,
            });
        }
        reserve(&mut mesh.shaders, shaders)?;
        for data in shader_data.as_chunks::<68>().0 {
            let mut t = Reader::new(data);
            mesh.shaders.push(Shader {
                name: t.name(64)?,
                native_index: t.i32()?,
            });
        }
        reserve(&mut mesh.texcoords, vertices)?;
        for data in uv_data.as_chunks::<8>().0 {
            let mut t = Reader::new(data);
            mesh.texcoords.push([t.float()?, t.float()?]);
        }
        reserve(&mut mesh.vertices, frames * vertices)?;
        let mut base_ids = Reader::new(base_indices);
        let mut comp_ids = Reader::new(comp_indices);
        for frame in 0..frames {
            let base = if compressed {
                bounded_index(base_ids.i16()? as usize, base_frames)? as usize
            } else {
                frame
            };
            let comp = if compressed { comp_ids.i16()? } else { -1 };
            if comp < -1 || (comp >= 0 && comp as usize >= comp_frames) {
                return Err(FormatError::InvalidReference(
                    "MDC compressed frame",
                    comp as usize,
                ));
            }
            for vertex in 0..vertices {
                let at = (base * vertices + vertex) * 8;
                let mut t = Reader::new(&vert_data[at..at + 8]);
                let mut position = Vec3([
                    f32::from(t.i16()?) / 64.0,
                    f32::from(t.i16()?) / 64.0,
                    f32::from(t.i16()?) / 64.0,
                ]);
                let mut normal = normals::packed(t.i16()? as u16);
                if comp >= 0 {
                    let at = (comp as usize * vertices + vertex) * 4;
                    let packed = &comp_data[at..at + 4];
                    for (a, value) in packed[..3].iter().enumerate() {
                        position.0[a] += (f32::from(*value) - 127.0) * 0.05;
                    }
                    normal = normals::COMPRESSED[usize::from(packed[3])];
                }
                mesh.vertices.push(Vertex { position, normal });
            }
        }
        m.meshes.push(mesh);
        surface_at += length;
    }
    if surface_at != end {
        return Err(FormatError::InvalidRange);
    }
    Ok(m)
}
