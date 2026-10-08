use super::*;
use qa_core::primitives::Axis;

type Row<'a> = crate::read::Reader<'a>;
impl Row<'_> {
    fn index(&mut self, wide: bool) -> Result<u32, FormatError> {
        if wide {
            self.u32()
        } else {
            self.u16().map(u32::from)
        }
    }
    fn range(&mut self, wide: bool) -> Result<IndexRange, FormatError> {
        Ok(IndexRange {
            first: self.index(wide)?,
            count: self.index(wide)?,
        })
    }
    fn bounds(&mut self, format: u8) -> Result<Bounds, FormatError> {
        let mut coordinates = [0.0; 6];
        for value in &mut coordinates {
            *value = match format {
                0 => self.u16()? as i16 as f32,
                1 => self.i32()? as f32,
                _ => self.float()?,
            };
        }
        Ok(Bounds {
            mins: Vec3(
                coordinates[..3]
                    .try_into()
                    .map_err(|_| FormatError::Truncated)?,
            ),
            maxs: Vec3(
                coordinates[3..]
                    .try_into()
                    .map_err(|_| FormatError::Truncated)?,
            ),
        })
    }
    fn four_bytes(&mut self) -> Result<[u8; 4], FormatError> {
        self.take(4)?.try_into().map_err(|_| FormatError::Truncated)
    }
}
fn cstring(bytes: &[u8]) -> &[u8] {
    &bytes[..bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len())]
}
fn records<'a, T>(
    bsp: &Bsp<'a>,
    kind: Lump,
    mut read: impl FnMut(&mut Row<'a>) -> Result<T, FormatError>,
) -> Result<Vec<T>, FormatError> {
    let bytes = bsp.bytes(kind);
    let stride = bsp.stride(kind);
    let mut result = Vec::with_capacity(bytes.len() / stride);
    for data in bytes.chunks_exact(stride) {
        result.push(read(&mut Row { bytes: data, at: 0 })?);
    }
    Ok(result)
}
fn short_child(row: &mut Row<'_>, count: usize) -> Result<i32, FormatError> {
    let value = i32::from(row.u16()?);
    Ok(if (value as usize) < count {
        value
    } else {
        value - 65536
    })
}
fn nullable(row: &mut Row<'_>, wide: bool) -> Result<i64, FormatError> {
    let value = row.index(wide)?;
    Ok(if value == if wide { u32::MAX } else { u16::MAX as u32 } {
        -1
    } else {
        i64::from(value)
    })
}

pub(super) fn map<'a>(bsp: Bsp<'a>) -> Result<Map<'a>, FormatError> {
    let format = bsp.format;
    let family = format.family();
    let wide = format.wide();
    let planes = records(&bsp, Planes, |r| {
        let normal = r.vector()?;
        let distance = r.float()?;
        let plane_type = if bsp.stride(Planes) == 20 {
            r.i32()?
        } else {
            3
        };
        let axis = if family != 3 && (0..3).contains(&plane_type) {
            Some([Axis::X, Axis::Y, Axis::Z][plane_type as usize])
        } else {
            (0..3)
                .find(|&a| normal.0[a] == 1.0 && (0..3).all(|b| b == a || normal.0[b] == 0.0))
                .map(|a| [Axis::X, Axis::Y, Axis::Z][a])
        };
        Ok(Plane {
            normal,
            distance,
            axis,
        })
    })?;
    let vertices = records(&bsp, Vertices, |r| {
        let position = r.vector()?;
        let (texcoord, lightmap_coord, normal, color) = if family == 3 {
            (
                [r.float()?, r.float()?],
                [r.raw_float()?, r.raw_float()?],
                r.vector()?,
                r.four_bytes()?,
            )
        } else {
            ([0.0; 2], [0.0; 2], Vec3::default(), [255; 4])
        };
        Ok(Vertex {
            position,
            texcoord,
            lightmap_coord,
            normal,
            color,
        })
    })?;
    let node_count = bsp.bytes(Nodes).len() / bsp.stride(Nodes);
    let nodes = records(&bsp, Nodes, |r| {
        let plane = r.u32()?;
        let children = if format.narrow_q1() {
            [short_child(r, node_count)?, short_child(r, node_count)?]
        } else {
            [r.i32()?, r.i32()?]
        };
        let bounds = r.bounds(if family == 3 {
            1
        } else if matches!(format, BspFormat::Bsp2 | BspFormat::Qbsp) {
            2
        } else {
            0
        })?;
        let faces = if family == 3 {
            IndexRange::default()
        } else {
            r.range(wide)?
        };
        Ok(Node {
            plane,
            children,
            bounds,
            faces,
        })
    })?;
    let leaves = records(&bsp, Leaves, |r| {
        let mut value = Leaf {
            contents: 0,
            cluster: -1,
            area: -1,
            visibility_offset: -1,
            bounds: Bounds::default(),
            faces: IndexRange::default(),
            brushes: IndexRange::default(),
            ambient: [0; 4],
        };
        if family == 1 {
            value.contents = r.i32()?;
            value.visibility_offset = r.i32()?;
            value.bounds = r.bounds(if format == BspFormat::Bsp2 { 2 } else { 0 })?;
            value.faces = r.range(wide)?;
            value.ambient = r.four_bytes()?;
        } else if family == 2 {
            value.contents = r.i32()?;
            value.cluster = nullable(r, wide)?;
            value.area = i64::from(r.index(wide)?);
            value.bounds = r.bounds(if wide { 2 } else { 0 })?;
            value.faces = r.range(wide)?;
            value.brushes = r.range(wide)?;
        } else {
            value.cluster = i64::from(r.i32()?);
            value.area = i64::from(r.i32()?);
            value.bounds = r.bounds(1)?;
            value.faces = r.range(true)?;
            value.brushes = r.range(true)?;
        }
        Ok(value)
    })?;
    let edges = records(&bsp, Edges, |r| Ok([r.index(wide)?, r.index(wide)?]))?;
    let surface_edges = records(&bsp, SurfEdges, Row::i32)?;
    let leaf_faces = records(&bsp, LeafFaces, |r| r.index(wide || family == 3))?;
    let leaf_brushes = records(&bsp, LeafBrushes, |r| r.index(wide || family == 3))?;
    let faces = records(&bsp, Faces, |r| {
        let plane = r.index(wide)?;
        let flags = r.index(wide)?;
        if family == 1 && flags > 1 {
            return Err(FormatError::InvalidValue);
        }
        let edges = IndexRange {
            first: r.u32()?,
            count: r.index(wide)?,
        };
        let texture_info = r.index(wide)?;
        let styles = r.four_bytes()?;
        let mut lighting_offset = r.i32()?;
        if format == BspFormat::Quake64 && lighting_offset != -1 {
            if lighting_offset % 2 != 0 {
                return Err(FormatError::InvalidValue);
            }
            lighting_offset /= 2;
        }
        Ok(Face {
            plane,
            flags,
            edges,
            texture_info,
            styles,
            lighting_offset,
        })
    })?;
    let clips = bsp.bytes(ClipNodes).len() / bsp.stride(ClipNodes);
    let clipnodes = records(&bsp, ClipNodes, |r| {
        Ok(ClipNode {
            plane: r.u32()?,
            children: if format.narrow_q1() {
                [short_child(r, clips)?, short_child(r, clips)?]
            } else {
                [r.i32()?, r.i32()?]
            },
        })
    })?;
    let texture_info = records(&bsp, TexInfo, |r| {
        let mut projection = [[0.0; 4]; 2];
        for axis in &mut projection {
            for v in axis {
                *v = r.float()?;
            }
        }
        let (texture, flags, value, name, next) = if family == 1 {
            (r.i32()?, r.i32()?, 0, &[][..], -1)
        } else {
            (-1, r.i32()?, r.i32()?, r.name(32)?, r.i32()?)
        };
        Ok(TextureInfo {
            projection,
            texture,
            flags,
            value,
            name,
            next,
        })
    })?;
    let models = records(&bsp, Models, |r| {
        let bounds = r.bounds(2)?;
        let mut model = Model {
            bounds,
            origin: Vec3::default(),
            headnodes: [0; 4],
            visible_leaves: 0,
            faces: IndexRange::default(),
            brushes: IndexRange::default(),
            membership_from_tree: format == BspFormat::Quake3Test,
        };
        if format.modern_q3() {
            model.faces = r.range(true)?;
            model.brushes = r.range(true)?;
        } else {
            model.origin = r.vector()?;
            model.headnodes[0] = r.i32()?;
            if family == 1 {
                for root in &mut model.headnodes[1..] {
                    *root = r.i32()?;
                }
                model.visible_leaves = r.i32()?;
            }
            model.faces = r.range(true)?;
        }
        Ok(model)
    })?;
    let brushes = records(&bsp, Brushes, |r| {
        let sides = r.range(true)?;
        let (contents, shader) = if format.modern_q3() {
            (0, Some(r.u32()?))
        } else {
            (r.i32()?, None)
        };
        Ok(Brush {
            sides,
            contents,
            shader,
        })
    })?;
    let brush_sides = records(&bsp, BrushSides, |r| {
        let plane = r.index(if family == 2 { wide } else { true })?;
        let (texture_info, shader, flags) = if family == 2 {
            let info = nullable(r, wide)?;
            ((info != -1).then_some(info as u32), None, 0)
        } else if format == BspFormat::Quake3Test {
            (None, None, r.i32()?)
        } else {
            (None, Some(r.u32()?), 0)
        };
        Ok(BrushSide {
            plane,
            texture_info,
            shader,
            flags,
        })
    })?;
    let shaders = records(&bsp, Shaders, |r| {
        Ok(Shader {
            name: r.name(64)?,
            surface_flags: r.i32()?,
            content_flags: r.i32()?,
        })
    })?;
    let fogs = records(&bsp, Fogs, |r| {
        Ok(Fog {
            name: r.name(64)?,
            brush: r.i32()?,
            visible_side: if format == BspFormat::Quake3Test {
                -1
            } else {
                r.i32()?
            },
        })
    })?;
    let surfaces = records(&bsp, Surfaces, |r| surface(r, format))?;
    let indices = records(&bsp, Indices, Row::i32)?;
    let areas = records(&bsp, Areas, |r| {
        let count = r.u32()?;
        Ok(IndexRange {
            first: r.u32()?,
            count,
        })
    })?;
    let area_portals = records(&bsp, AreaPortals, |r| {
        Ok(AreaPortal {
            portal: r.u32()?,
            other_area: r.u32()?,
        })
    })?;
    let textures = textures(&bsp)?;
    let extensions = extensions(&bsp)?;
    Ok(Map {
        bsp,
        planes,
        vertices,
        nodes,
        leaves,
        edges,
        surface_edges,
        faces,
        leaf_faces,
        leaf_brushes,
        clipnodes,
        texture_info,
        textures,
        models,
        brushes,
        brush_sides,
        shaders,
        fogs,
        surfaces,
        indices,
        areas,
        area_portals,
        extensions,
    })
}

fn surface<'a>(r: &mut Row<'a>, format: BspFormat) -> Result<Surface<'a>, FormatError> {
    let test = format == BspFormat::Quake3Test;
    let (shader, shader_name, fog, brush_side, kind) = if test {
        (None, r.name(64)?, r.i32()?, r.i32()?, SurfaceKind::Planar)
    } else {
        let shader = r.u32()?;
        let fog = r.i32()?;
        let kind = match r.i32()? {
            1 => SurfaceKind::Planar,
            2 => SurfaceKind::Patch,
            3 => SurfaceKind::Triangles,
            4 => SurfaceKind::Flare,
            _ => return Err(FormatError::InvalidValue),
        };
        (Some(shader), &[][..], fog, -1, kind)
    };
    let vertices = r.range(true)?;
    let indices = r.range(true)?;
    let mut patch = if test { [r.i32()?, r.i32()?] } else { [0; 2] };
    let kind = if test {
        if patch[0] > 0 && patch[1] > 0 {
            SurfaceKind::Patch
        } else if indices.count > 0 {
            SurfaceKind::Triangles
        } else {
            SurfaceKind::Planar
        }
    } else {
        kind
    };
    let lightmap = r.i32()?;
    let lightmap_rect = [r.i32()?, r.i32()?, r.i32()?, r.i32()?];
    let lightmap_origin = r.vector()?;
    let lightmap_vectors = [r.vector()?, r.vector()?, r.vector()?];
    if !test {
        patch = [r.i32()?, r.i32()?];
    }
    Ok(Surface {
        kind,
        shader,
        shader_name,
        fog,
        brush_side,
        vertices,
        indices,
        patch,
        lightmap,
        lightmap_rect,
        lightmap_origin,
        lightmap_vectors,
        triangle_fan: test && kind == SurfaceKind::Planar,
    })
}

fn textures<'a>(bsp: &Bsp<'a>) -> Result<Vec<Option<MipTexture<'a>>>, FormatError> {
    let bytes = bsp.bytes(Textures);
    if bytes.is_empty() {
        return Ok(Vec::new());
    }
    let count = word(bytes, 0)? as usize;
    if count > (bytes.len().saturating_sub(4)) / 4 {
        return Err(FormatError::InvalidRange);
    }
    let mut result = Vec::with_capacity(count);
    for index in 0..count {
        let offset = word(bytes, 4 + index * 4)? as i32;
        if offset == -1 {
            result.push(None);
            continue;
        }
        if offset < 0 || (offset as usize) < 4 + count * 4 {
            return Err(FormatError::InvalidRange);
        }
        let row = bytes
            .get(offset as usize..)
            .ok_or(FormatError::InvalidRange)?;
        let format = if bsp.format == BspFormat::Quake64 {
            crate::image::MipFormat::Quake64
        } else {
            crate::image::MipFormat::Quake
        };
        result.push(Some(MipTexture::parse(row, format)?));
    }
    Ok(result)
}

fn extensions<'a>(bsp: &Bsp<'a>) -> Result<Vec<Extension<'a>>, FormatError> {
    if bsp.format.family() == 3 {
        return Ok(Vec::new());
    }
    let normal = (bsp.end + 3) & !3;
    let legacy = bsp.format.family() == 1
        && !bsp.lumps.iter().any(|bytes| {
            !bytes.is_empty() && (bytes.as_ptr() as usize - bsp.source.as_ptr() as usize) < 132
        });
    for offset in [Some(normal), legacy.then_some(124)].into_iter().flatten() {
        if bsp.source.get(offset..offset + 4) != Some(b"BSPX") {
            continue;
        }
        let count = word(bsp.source, offset + 4)? as usize;
        if count > bsp.source.len().saturating_sub(offset + 8) / 32 {
            return Err(FormatError::Truncated);
        }
        let mut result = Vec::with_capacity(count);
        for index in 0..count {
            let row = &bsp.source[offset + 8 + index * 32..offset + 8 + (index + 1) * 32];
            let name = cstring(&row[..24]);
            let bytes = span(bsp.source, word(row, 24)?, word(row, 28)?)?;
            result.push(Extension { name, bytes });
        }
        return Ok(result);
    }
    Ok(Vec::new())
}
