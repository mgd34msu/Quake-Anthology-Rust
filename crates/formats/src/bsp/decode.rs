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
    fn range(&mut self, wide: bool) -> Result<IndexSpan, FormatError> {
        Ok(IndexSpan {
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

pub(super) fn texture_info<'a>(
    r: &mut Row<'a>,
    rules: RuleSetId,
) -> Result<TextureInfo<'a>, FormatError> {
    let mut projection = [[0.0; 4]; 2];
    for axis in &mut projection {
        for v in axis {
            *v = r.float()?;
        }
    }
    let (texture, flags, value, name, next) = if matches!(rules, RuleSetId::Quake) {
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
}

pub(super) fn map<'a>(bsp: Bsp<'a>) -> Result<Map<'a>, FormatError> {
    let format = bsp.format;
    let rules = format.rule_set();
    let wide = format.wide();
    let mut planes = records(&bsp, Planes, |r| {
        let normal = r.vector()?;
        let distance = r.float()?;
        let plane_type = if bsp.stride(Planes) == 20 {
            r.i32()?
        } else {
            3
        };
        let mut plane = Plane::oriented(normal, distance);
        if !matches!(rules, RuleSetId::Quake3) && (0..3).contains(&plane_type) {
            plane.axis = Axis::from_index(plane_type as usize);
        }
        let [kind, signs] = plane.type_sign();
        plane.encoding = Some([
            if bsp.stride(Planes) == 20 {
                plane_type as u8
            } else {
                kind
            },
            signs,
        ]);
        Ok(plane)
    })?;
    let vertices = records(&bsp, Vertices, |r| {
        let position = r.vector()?;
        let (texcoord, lightmap_coord, normal, color) = if matches!(rules, RuleSetId::Quake3) {
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
        let bounds = r.bounds(if matches!(rules, RuleSetId::Quake3) {
            1
        } else if matches!(format, BspFormat::Bsp2 | BspFormat::Qbsp) {
            2
        } else {
            0
        })?;
        let faces = if matches!(rules, RuleSetId::Quake3) {
            IndexSpan::default()
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
            faces: IndexSpan::default(),
            brushes: IndexSpan::default(),
            ambient: [0; 4],
        };
        if matches!(rules, RuleSetId::Quake) {
            value.contents = r.i32()?;
            value.visibility_offset = r.i32()?;
            value.bounds = r.bounds(if format == BspFormat::Bsp2 { 2 } else { 0 })?;
            value.faces = r.range(wide)?;
            value.ambient = r.four_bytes()?;
        } else if matches!(rules, RuleSetId::Quake2) {
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
    let leaf_faces = records(&bsp, LeafFaces, |r| {
        r.index(wide || matches!(rules, RuleSetId::Quake3))
    })?;
    let leaf_brushes = records(&bsp, LeafBrushes, |r| {
        r.index(wide || matches!(rules, RuleSetId::Quake3))
    })?;
    let faces = records(&bsp, Faces, |r| {
        let plane = r.index(wide)?;
        let flags = r.index(wide)?;
        if matches!(rules, RuleSetId::Quake) && flags > 1 {
            return Err(FormatError::InvalidValue);
        }
        let edges = IndexSpan {
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
    let texture_rows = bsp.texture_info();
    let mut texture_info = Vec::with_capacity(texture_rows.len());
    for row in texture_rows {
        texture_info.push(row?);
    }
    let mut models = records(&bsp, Models, |r| {
        let bounds = r.bounds(2)?;
        let mut model = Model {
            bounds,
            origin: Vec3::default(),
            headnodes: [0; 4],
            visible_leaves: 0,
            faces: IndexSpan::default(),
            brushes: IndexSpan::default(),
            membership_from_tree: format == BspFormat::Quake3Test,
        };
        if format.modern_q3() {
            model.faces = r.range(true)?;
            model.brushes = r.range(true)?;
        } else {
            model.origin = r.vector()?;
            model.headnodes[0] = r.i32()?;
            if matches!(rules, RuleSetId::Quake) {
                for root in &mut model.headnodes[1..] {
                    *root = r.i32()?;
                }
                model.visible_leaves = r.i32()?;
            }
            model.faces = r.range(true)?;
        }
        Ok(model)
    })?;
    let mut brushes = records(&bsp, Brushes, |r| {
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
    let mut brush_sides = records(&bsp, BrushSides, |r| {
        let plane = r.index(if matches!(rules, RuleSetId::Quake2) {
            wide
        } else {
            true
        })?;
        let (texture_info, shader, flags) = if matches!(rules, RuleSetId::Quake2) {
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
        Ok(IndexSpan {
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
    let authored_brushes = matches!(rules, RuleSetId::Quake)
        && brush_list(
            &extensions,
            &mut models,
            &mut planes,
            &mut brushes,
            &mut brush_sides,
        )?;
    let decoupled_lightmaps = decoupled_lightmaps(&bsp, &faces, &extensions)?;
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
        authored_brushes,
        shaders,
        fogs,
        surfaces,
        indices,
        areas,
        area_portals,
        extensions,
        decoupled_lightmaps,
    })
}

#[derive(Default)]
struct BrushChunk {
    planes: Vec<Plane>,
    sides: Vec<BrushSide>,
    brushes: Vec<Brush>,
}
impl BrushChunk {
    fn plane(&mut self, plane: Plane) -> Result<(), FormatError> {
        if !plane.is_valid() {
            return Err(FormatError::InvalidValue);
        }
        let plane_index =
            u32::try_from(self.planes.len()).map_err(|_| FormatError::InvalidRange)?;
        self.planes.push(plane);
        self.sides.push(BrushSide {
            plane: plane_index,
            texture_info: None,
            shader: None,
            flags: 0,
        });
        Ok(())
    }
}

fn brush_chunk(
    mut row: Row<'_>,
    count: usize,
    mut remaining_planes: usize,
) -> Result<BrushChunk, FormatError> {
    let mut chunk = BrushChunk::default();
    for _ in 0..count {
        let bounds = row.bounds(2)?;
        if !bounds.is_valid() {
            return Err(FormatError::InvalidValue);
        }
        let contents = i32::from(row.i16()?);
        let planes = usize::from(row.u16()?);
        remaining_planes = remaining_planes
            .checked_sub(planes)
            .ok_or(FormatError::InvalidRange)?;
        let first = u32::try_from(chunk.sides.len()).map_err(|_| FormatError::InvalidRange)?;
        for _ in 0..planes {
            let normal = row.vector()?;
            let distance = row.float()?;
            chunk.plane(Plane::oriented(normal, distance))?;
        }
        // FTE Q1BSP_LoadBrushes appends all positive axes before negative axes.
        for axis in 0..3 {
            chunk.plane(Plane::oriented(Vec3::axis(axis, 1.0), bounds.maxs.0[axis]))?;
        }
        for axis in 0..3 {
            chunk.plane(Plane::oriented(
                Vec3::axis(axis, -1.0),
                -bounds.mins.0[axis],
            ))?;
        }
        chunk.brushes.push(Brush {
            sides: IndexSpan {
                first,
                count: u32::try_from(planes + 6).map_err(|_| FormatError::InvalidRange)?,
            },
            contents,
            shader: None,
        });
    }
    if remaining_planes != 0 || row.at != row.bytes.len() {
        return Err(FormatError::InvalidRecordSize);
    }
    Ok(chunk)
}

/// Decode into the same source records; repeated complete model chunks replace
/// their predecessor, including a later empty chunk (FTE q1bsp.c:1449-1507).
fn brush_list(
    extensions: &[Extension<'_>],
    models: &mut [Model],
    planes: &mut Vec<Plane>,
    brushes: &mut Vec<Brush>,
    sides: &mut Vec<BrushSide>,
) -> Result<bool, FormatError> {
    let Some(extension) = extensions.iter().find(|entry| entry.name == b"BRUSHLIST") else {
        return Ok(false);
    };
    let mut selected: Vec<Option<BrushChunk>> =
        std::iter::repeat_with(|| None).take(models.len()).collect();
    let mut row = Row::new(extension.bytes);
    while row.at < row.bytes.len() {
        if row.u32()? != 1 {
            return Err(FormatError::Unsupported);
        }
        let model = row.u32()? as usize;
        let count = row.u32()? as usize;
        let plane_count = row.u32()? as usize;
        let output = selected
            .get_mut(model)
            .ok_or(FormatError::InvalidReference("BRUSHLIST model", model))?;
        let length = count
            .checked_mul(28)
            .and_then(|bytes| {
                plane_count
                    .checked_mul(16)
                    .and_then(|planes| bytes.checked_add(planes))
            })
            .ok_or(FormatError::InvalidRange)?;
        let payload = row.take(length)?;
        *output = Some(brush_chunk(Row::new(payload), count, plane_count)?);
    }
    for (model, chunk) in models.iter_mut().zip(selected) {
        model.brushes = IndexSpan {
            first: u32::try_from(brushes.len()).map_err(|_| FormatError::InvalidRange)?,
            count: 0,
        };
        let Some(mut chunk) = chunk else {
            continue;
        };
        let plane_base = u32::try_from(planes.len()).map_err(|_| FormatError::InvalidRange)?;
        let side_base = u32::try_from(sides.len()).map_err(|_| FormatError::InvalidRange)?;
        u32::try_from(
            planes
                .len()
                .checked_add(chunk.planes.len())
                .ok_or(FormatError::InvalidRange)?,
        )
        .map_err(|_| FormatError::InvalidRange)?;
        u32::try_from(
            sides
                .len()
                .checked_add(chunk.sides.len())
                .ok_or(FormatError::InvalidRange)?,
        )
        .map_err(|_| FormatError::InvalidRange)?;
        u32::try_from(
            brushes
                .len()
                .checked_add(chunk.brushes.len())
                .ok_or(FormatError::InvalidRange)?,
        )
        .map_err(|_| FormatError::InvalidRange)?;
        for side in &mut chunk.sides {
            side.plane += plane_base;
        }
        for brush in &mut chunk.brushes {
            brush.sides.first += side_base;
        }
        model.brushes.count =
            u32::try_from(chunk.brushes.len()).map_err(|_| FormatError::InvalidRange)?;
        planes.extend(chunk.planes);
        sides.extend(chunk.sides);
        brushes.extend(chunk.brushes);
    }
    Ok(true)
}

/// FTE specs/bspx.txt DECOUPLED_LM and q2repro common/bsp.c:412-454.
/// Invalid whole-lump sizes leave ordinary face lighting untouched; bad sample
/// spans remain unlit while retaining the authored dimensions and projection.
fn decoupled_lightmaps(
    bsp: &Bsp<'_>,
    faces: &[Face],
    extensions: &[Extension<'_>],
) -> Result<Vec<Option<DecoupledLightmap>>, FormatError> {
    const STRIDE: usize = 40;
    let mut result = vec![None; faces.len()];
    let Some(extension) = extensions
        .iter()
        .find(|entry| entry.name == b"DECOUPLED_LM")
    else {
        return Ok(result);
    };
    if !extension.bytes.len().is_multiple_of(STRIDE) || extension.bytes.len() / STRIDE < faces.len()
    {
        return Ok(result);
    }
    let sample_bytes = match bsp.format {
        BspFormat::Quake64 => 2,
        BspFormat::HalfLife | BspFormat::Quake2 | BspFormat::Qbsp => 3,
        _ => 1,
    };
    let lighting = bsp.bytes(Lighting);
    for ((output, face), bytes) in result
        .iter_mut()
        .zip(faces)
        .zip(extension.bytes.chunks_exact(STRIDE))
    {
        let mut row = Row::new(bytes);
        let width = u32::from(row.u16()?);
        let height = u32::from(row.u16()?);
        let source_offset = row.u32()?;
        let mut projection = [[0.0; 4]; 2];
        for axis in &mut projection {
            for value in axis {
                *value = row.raw_float()?;
            }
        }
        if projection.iter().flatten().any(|value| !value.is_finite()) {
            continue;
        }
        let styles = face
            .styles
            .iter()
            .take_while(|&&style| style != 255)
            .count();
        let span = (width as usize)
            .checked_mul(height as usize)
            .and_then(|count| count.checked_mul(styles))
            .and_then(|count| count.checked_mul(sample_bytes))
            .and_then(|length| (source_offset as usize).checked_add(length));
        let offset = (source_offset != u32::MAX
            && width != 0
            && height != 0
            && styles != 0
            && face.styles[styles..].iter().all(|&style| style == 255)
            && span.is_some_and(|end| end <= lighting.len()))
        .then_some(source_offset);
        *output = Some(DecoupledLightmap {
            width,
            height,
            offset,
            projection,
        });
    }
    Ok(result)
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
    if matches!(bsp.format.rule_set(), RuleSetId::Quake3) {
        return Ok(Vec::new());
    }
    let normal = (bsp.end + 3) & !3;
    let legacy = matches!(bsp.format.rule_set(), RuleSetId::Quake)
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
            let name = qa_core::text::until_nul(&row[..24]);
            let bytes = span(bsp.source, word(row, 24)?, word(row, 28)?)?;
            result.push(Extension { name, bytes });
        }
        return Ok(result);
    }
    Ok(Vec::new())
}
