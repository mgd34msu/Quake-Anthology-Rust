use super::{read::*, *};

fn group(
    r: &mut Reader<'_>,
    intervals: &mut Vec<f32>,
    frame_bounds: bool,
    scale: Vec3,
    translation: Vec3,
) -> Result<Group, FormatError> {
    let grouped = r.count(0, 1)? != 0;
    let count = if grouped {
        r.count(1, i32::MAX as usize)?
    } else {
        1
    };
    let bounds = if grouped && frame_bounds {
        packed_bounds(r, scale, translation)?
    } else {
        Bounds::default()
    };
    let first = intervals.len();
    if grouped {
        r.remaining_records(count, 4)?;
        reserve(intervals, count)?;
        for _ in 0..count {
            let value = r.float()?;
            if value <= 0.0 {
                return Err(FormatError::InvalidValue);
            }
            intervals.push(value);
        }
    }
    Ok(Group {
        members: 0..count,
        intervals: first..intervals.len(),
        bounds,
    })
}
fn packed_bounds(
    r: &mut Reader<'_>,
    scale: Vec3,
    translation: Vec3,
) -> Result<Bounds, FormatError> {
    let points = r.take(8)?;
    let mut bounds = empty_bounds();
    for p in points.as_chunks::<4>().0 {
        let point = Vec3(std::array::from_fn(|a| {
            f32::from(p[a]) * scale.0[a] + translation.0[a]
        }));
        finite_vec(point)?;
        add_point(&mut bounds, point);
    }
    Ok(bounds)
}
fn vertices(
    bytes: &[u8],
    scale: Vec3,
    translation: Vec3,
    out: &mut Vec<Vertex>,
    bounds: &mut Bounds,
) -> Result<(), FormatError> {
    if out
        .len()
        .checked_add(bytes.len() / 4)
        .ok_or(FormatError::InvalidRange)?
        > MAX_DECODED_BYTES / std::mem::size_of::<Vertex>()
    {
        return Err(FormatError::InvalidRange);
    }
    reserve(out, bytes.len() / 4)?;
    for p in bytes.as_chunks::<4>().0 {
        let normal = *normals::ALIAS
            .get(usize::from(p[3]))
            .ok_or(FormatError::InvalidReference("alias normal", p[3] as usize))?;
        let position = Vec3(std::array::from_fn(|a| {
            f32::from(p[a]) * scale.0[a] + translation.0[a]
        }));
        finite_vec(position)?;
        add_point(bounds, position);
        out.push(Vertex { position, normal });
    }
    Ok(())
}
fn half_texel(value: f32, dimension: usize) -> f32 {
    ((f64::from(value) + 0.5) / dimension as f64) as f32
}

pub(super) fn mdl<'a>(bytes: &'a [u8], mut m: Model<'a>) -> Result<Model<'a>, FormatError> {
    let mut r = Reader::new(bytes);
    r.take(4)?;
    if r.i32()? != 6 {
        return Err(FormatError::Unsupported);
    }
    let scale = r.vector()?;
    let translation = r.vector()?;
    m.radius = r.float()?;
    m.eye_position = r.vector()?;
    let skins = r.count(1, i32::MAX as usize)?;
    let width = r.count(1, i32::MAX as usize)?;
    let height = r.count(1, i32::MAX as usize)?;
    let vertex_count = r.count(1, i32::MAX as usize)?;
    let triangles = r.count(1, i32::MAX as usize)?;
    let groups = r.count(1, i32::MAX as usize)?;
    m.sync = r.i32()?;
    m.flags = r.i32()?;
    m.native_size = r.float()?;
    m.declared_skins = skins as u32;
    if m.radius < 0.0 {
        return Err(FormatError::InvalidValue);
    }
    let pixels = width.checked_mul(height).ok_or(FormatError::InvalidRange)?;
    r.remaining_records(
        skins,
        pixels.checked_add(4).ok_or(FormatError::InvalidRange)?,
    )?;
    for _ in 0..skins {
        let mut g = group(&mut r, &mut m.intervals, false, scale, translation)?;
        let count = g.members.len();
        r.remaining_records(count, pixels)?;
        g.members = m.skins.len()..m.skins.len() + count;
        reserve(&mut m.skins, count)?;
        for _ in 0..count {
            m.skins.push(Image::Indexed {
                width: width as u32,
                height: height as u32,
                pixels: r.take(pixels)?,
            });
        }
        m.skin_groups.push(g);
    }
    r.remaining_records(vertex_count, 12)?;
    let mut mesh = Mesh {
        vertices_per_frame: vertex_count,
        ..Mesh::default()
    };
    reserve(&mut mesh.texcoords, vertex_count)?;
    let mut back_coords = Vec::new();
    reserve(&mut back_coords, vertex_count)?;
    for _ in 0..vertex_count {
        let back = r.i32()? != 0;
        let s = r.i32()? as f32;
        let t = r.i32()? as f32;
        mesh.texcoords
            .push([half_texel(s, width), half_texel(t, height)]);
        back_coords.push(back.then_some([
            half_texel(s + (width / 2) as f32, width),
            half_texel(t, height),
        ]));
    }
    // Native MDL back-facing texture coordinates become ordinary UV indices.
    let mut back_indices = Vec::new();
    reserve(&mut back_indices, vertex_count)?;
    for (index, back) in back_coords.into_iter().enumerate() {
        back_indices.push(if let Some(uv) = back {
            let result = mesh.texcoords.len() as u32;
            mesh.texcoords.push(uv);
            result
        } else {
            index as u32
        });
    }
    r.remaining_records(triangles, 16)?;
    reserve(&mut mesh.triangles, triangles)?;
    for _ in 0..triangles {
        let front = r.i32()? != 0;
        let mut t = Triangle::default();
        for a in 0..3 {
            let index = r.count(0, vertex_count - 1)?;
            t.vertex[a] = index as u32;
            t.texcoord[a] = if front {
                index as u32
            } else {
                back_indices[index]
            };
        }
        mesh.triangles.push(t);
    }
    let frame_size = vertex_count
        .checked_mul(4)
        .and_then(|n| n.checked_add(24))
        .ok_or(FormatError::InvalidRange)?;
    r.remaining_records(
        groups,
        frame_size.checked_add(4).ok_or(FormatError::InvalidRange)?,
    )?;
    for _ in 0..groups {
        let mut g = group(&mut r, &mut m.intervals, true, scale, translation)?;
        let count = g.members.len();
        r.remaining_records(count, frame_size)?;
        g.members = m.frames.len()..m.frames.len() + count;
        reserve(&mut m.frames, count)?;
        for _ in 0..count {
            let bounds = packed_bounds(&mut r, scale, translation)?;
            let name = r.name(16)?;
            let packed_vertices = r.take(vertex_count * 4)?;
            vertices(
                packed_vertices,
                scale,
                translation,
                &mut mesh.vertices,
                &mut empty_bounds(),
            )?;
            m.frames.push(Frame {
                name,
                bounds,
                scale,
                translation,
                packed_vertices,
                ..Frame::default()
            });
        }
        if g.intervals.is_empty() {
            g.bounds = m.frames[g.members.start].bounds;
        }
        add_bounds(&mut m.bounds, g.bounds);
        m.frame_groups.push(g);
    }
    m.meshes.push(mesh);
    Ok(m)
}
pub(super) fn md2<'a>(bytes: &'a [u8], mut m: Model<'a>) -> Result<Model<'a>, FormatError> {
    let mut r = Reader::new(bytes);
    r.take(4)?;
    if r.i32()? != 8 {
        return Err(FormatError::Unsupported);
    }
    let width = r.count(1, i32::MAX as usize)?;
    let height = r.count(1, i32::MAX as usize)?;
    let stride = r.count(1, i32::MAX as usize)?;
    let skins = r.count(0, i32::MAX as usize)?;
    let vertex_count = r.count(1, i32::MAX as usize)?;
    let coords = r.count(1, i32::MAX as usize)?;
    let triangles = r.count(1, i32::MAX as usize)?;
    let commands = r.count(0, i32::MAX as usize)?;
    let frames = r.count(1, i32::MAX as usize)?;
    let skin_at = r.count(68, i32::MAX as usize)?;
    let uv_at = r.count(68, i32::MAX as usize)?;
    let tri_at = r.count(68, i32::MAX as usize)?;
    let frame_at = r.count(68, i32::MAX as usize)?;
    let command_at = r.count(68, i32::MAX as usize)?;
    let end = r.count(68, i32::MAX as usize)?;
    if stride
        < vertex_count
            .checked_mul(4)
            .and_then(|n| n.checked_add(40))
            .ok_or(FormatError::InvalidRange)?
    {
        return Err(FormatError::InvalidRecordSize);
    }
    let skin_data = r.section(skin_at, skins, 64, 68, end)?;
    let uv_data = r.section(uv_at, coords, 4, 68, end)?;
    let tri_data = r.section(tri_at, triangles, 12, 68, end)?;
    let frame_data = r.section(frame_at, frames, stride, 68, end)?;
    m.gl_commands = r.section(command_at, commands, 4, 68, end)?;
    m.declared_skins = skins as u32;
    let mut s = Reader::new(skin_data);
    for _ in 0..skins {
        m.skins.push(Image::External(s.name(64)?));
    }
    let mut mesh = Mesh {
        vertices_per_frame: vertex_count,
        ..Mesh::default()
    };
    let mut s = Reader::new(uv_data);
    reserve(&mut mesh.texcoords, coords)?;
    for _ in 0..coords {
        mesh.texcoords.push([
            half_texel(f32::from(s.i16()?), width),
            half_texel(f32::from(s.i16()?), height),
        ]);
    }
    let mut s = Reader::new(tri_data);
    reserve(&mut mesh.triangles, triangles)?;
    for _ in 0..triangles {
        let mut t = Triangle::default();
        for v in &mut t.vertex {
            *v = bounded_index(s.i16()? as u16 as usize, vertex_count)?;
        }
        for v in &mut t.texcoord {
            *v = bounded_index(s.i16()? as u16 as usize, coords)?;
        }
        mesh.triangles.push(t);
    }
    for data in frame_data.chunks_exact(stride) {
        let mut s = Reader::new(data);
        let scale = s.vector()?;
        let translation = s.vector()?;
        let name = s.name(16)?;
        let packed_vertices = s.take(vertex_count * 4)?;
        let mut bounds = empty_bounds();
        vertices(
            packed_vertices,
            scale,
            translation,
            &mut mesh.vertices,
            &mut bounds,
        )?;
        add_bounds(&mut m.bounds, bounds);
        m.frames.push(Frame {
            name,
            bounds,
            scale,
            translation,
            packed_vertices,
            ..Frame::default()
        });
    }
    let mut s = Reader::new(m.gl_commands);
    let mut terminated = commands == 0;
    while s.at < s.bytes.len() {
        let count = s.i32()?;
        if count == 0 {
            terminated = true;
            break;
        }
        let count = count.unsigned_abs() as usize;
        if count < 3 {
            return Err(FormatError::InvalidValue);
        }
        s.remaining_records(count, 12)?;
        for _ in 0..count {
            s.float()?;
            s.float()?;
            s.count(0, vertex_count - 1)?;
        }
    }
    if !terminated {
        return Err(FormatError::InvalidValue);
    }
    m.meshes.push(mesh);
    Ok(m)
}
pub(super) fn sprite<'a>(bytes: &'a [u8], mut m: Model<'a>) -> Result<Model<'a>, FormatError> {
    let external = m.format == ModelFormat::Sp2;
    let mut r = Reader::new(bytes);
    r.take(4)?;
    if r.i32()? != if external { 2 } else { 1 } {
        return Err(FormatError::Unsupported);
    }
    let (width, height) = if external {
        (0, 0)
    } else {
        m.orientation = r.count(0, 4)? as u32;
        m.radius = r.float()?;
        (
            r.count(1, i32::MAX as usize)?,
            r.count(1, i32::MAX as usize)?,
        )
    };
    let groups = r.count(1, i32::MAX as usize)?;
    if !external {
        m.beam_length = r.float()?;
        m.sync = r.i32()?;
        if m.radius < 0.0 {
            return Err(FormatError::InvalidValue);
        }
    }
    r.remaining_records(groups, if external { 80 } else { 21 })?;
    for _ in 0..groups {
        let mut g = if external {
            Group {
                members: 0..1,
                intervals: 0..0,
                bounds: Bounds::default(),
            }
        } else {
            group(
                &mut r,
                &mut m.intervals,
                false,
                Vec3::default(),
                Vec3::default(),
            )?
        };
        let count = g.members.len();
        r.remaining_records(count, if external { 80 } else { 17 })?;
        g.members = m.sprites.len()..m.sprites.len() + count;
        reserve(&mut m.sprites, count)?;
        for _ in 0..count {
            let mut origin = [0; 2];
            if !external {
                origin = [r.i32()?, r.i32()?];
            }
            let w = r.count(1, i32::MAX as usize)?;
            let h = r.count(1, i32::MAX as usize)?;
            let image = if external {
                origin = [r.i32()?, r.i32()?];
                let x = f64::from(origin[0])
                    .abs()
                    .max((w as f64 - f64::from(origin[0])).abs());
                let y = f64::from(origin[1])
                    .abs()
                    .max((h as f64 - f64::from(origin[1])).abs());
                let radius = x.hypot(y) as f32;
                add_point(&mut m.bounds, Vec3([-radius; 3]));
                add_point(&mut m.bounds, Vec3([radius; 3]));
                Image::External(r.name(64)?)
            } else {
                let length = w.checked_mul(h).ok_or(FormatError::InvalidRange)?;
                Image::Indexed {
                    width: w as u32,
                    height: h as u32,
                    pixels: r.take(length)?,
                }
            };
            m.sprites.push(Sprite {
                width: w as u32,
                height: h as u32,
                origin,
                image,
            });
        }
        m.frame_groups.push(g);
    }
    if !external {
        let maxs = Vec3([width as f32 * 0.5, width as f32 * 0.5, height as f32 * 0.5]);
        m.bounds = Bounds { mins: -maxs, maxs };
    }
    for g in &mut m.frame_groups {
        g.bounds = m.bounds;
    }
    Ok(m)
}
