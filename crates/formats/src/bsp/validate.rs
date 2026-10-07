use super::*;

fn index(value: i64, length: usize, field: &'static str, record: usize) -> Result<(), FormatError> {
    if value < 0 || value as u64 >= length as u64 {
        return Err(FormatError::InvalidReference(field, record));
    }
    Ok(())
}
fn range(
    value: IndexRange,
    length: usize,
    field: &'static str,
    record: usize,
) -> Result<(), FormatError> {
    if u64::from(value.first) + u64::from(value.count) > length as u64 {
        return Err(FormatError::InvalidReference(field, record));
    }
    Ok(())
}
fn child(value: i32, map: &Map<'_>, record: usize) -> Result<(), FormatError> {
    if value >= 0 {
        index(i64::from(value), map.nodes.len(), "node child", record)
    } else {
        index(
            -1 - i64::from(value),
            map.leaves.len(),
            "leaf child",
            record,
        )
    }
}
fn trees(count: usize, children: impl Fn(usize) -> [i32; 2]) -> Result<(), FormatError> {
    let mut colors = vec![0u8; count];
    let mut stack = Vec::with_capacity(count);
    for root in 0..count {
        if colors[root] == 2 {
            continue;
        }
        stack.push((root, 0));
        colors[root] = 1;
        while let Some((node, next)) = stack.last_mut() {
            if *next == 2 {
                colors[*node] = 2;
                stack.pop();
                continue;
            }
            let value = children(*node)[*next];
            *next += 1;
            if value < 0 {
                continue;
            }
            let value = value as usize;
            match colors[value] {
                1 => return Err(FormatError::Cycle),
                0 => {
                    colors[value] = 1;
                    stack.push((value, 0));
                }
                _ => {}
            }
        }
    }
    Ok(())
}

pub(super) fn map(map: &Map<'_>) -> Result<(), FormatError> {
    let format = map.bsp.format;
    let family = format.family();
    if map.models.is_empty() {
        return Err(FormatError::InvalidReference("world model", 0));
    }
    if family >= 2 && map.nodes.is_empty() {
        return Err(FormatError::InvalidReference("world root", 0));
    }
    let faces = if family == 3 {
        map.surfaces.len()
    } else {
        map.faces.len()
    };
    for (i, edge) in map.edges.iter().enumerate() {
        for &vertex in edge {
            index(i64::from(vertex), map.vertices.len(), "edge vertex", i)?;
        }
    }
    for (i, &edge) in map.surface_edges.iter().enumerate() {
        index(i64::from(edge).abs(), map.edges.len(), "surface edge", i)?;
    }
    for (i, &face) in map.leaf_faces.iter().enumerate() {
        index(i64::from(face), faces, "leaf face", i)?;
    }
    for (i, &brush) in map.leaf_brushes.iter().enumerate() {
        index(i64::from(brush), map.brushes.len(), "leaf brush", i)?;
    }
    for (i, texture) in map.texture_info.iter().enumerate() {
        if family == 1 && !map.textures.is_empty() {
            index(
                i64::from(texture.texture),
                map.textures.len(),
                "mip texture",
                i,
            )?;
        }
        if family == 2 && texture.next > 0 {
            index(
                i64::from(texture.next),
                map.texture_info.len(),
                "animated texture",
                i,
            )?;
        }
    }
    let lighting = map.bsp.bytes(Lighting);
    for (i, face) in map.faces.iter().enumerate() {
        index(i64::from(face.plane), map.planes.len(), "face plane", i)?;
        index(
            i64::from(face.texture_info),
            map.texture_info.len(),
            "face texture",
            i,
        )?;
        range(face.edges, map.surface_edges.len(), "face edges", i)?;
        if face.edges.count < 3 || face.lighting_offset < -1 {
            return Err(FormatError::InvalidReference("face values", i));
        }
        let length = if format == BspFormat::Quake64 {
            lighting.len() / 2
        } else {
            lighting.len()
        };
        if face.lighting_offset >= 0 && length > 0 {
            index(i64::from(face.lighting_offset), length, "face lighting", i)?;
        }
    }
    let visibility = map.bsp.bytes(Visibility);
    let clusters = visibility_header(visibility, family)?;
    let pvs_bytes = map
        .models
        .first()
        .map_or(map.leaves.len().saturating_sub(1), |model| {
            model.visible_leaves.max(0) as usize
        })
        .div_ceil(8);
    for (i, leaf) in map.leaves.iter().enumerate() {
        range(leaf.faces, map.leaf_faces.len(), "leaf faces", i)?;
        range(leaf.brushes, map.leaf_brushes.len(), "leaf brushes", i)?;
        if family == 1 {
            if leaf.visibility_offset < -1 {
                return Err(FormatError::InvalidReference("leaf visibility", i));
            }
            if leaf.visibility_offset >= 0 && !visibility.is_empty() {
                validate_rle(visibility, leaf.visibility_offset as usize, pvs_bytes)?;
            }
        } else {
            if leaf.cluster < -1 {
                return Err(FormatError::InvalidReference("leaf cluster", i));
            }
            if leaf.cluster != -1
                && let Some(clusters) = clusters
            {
                index(leaf.cluster, clusters, "leaf cluster", i)?;
            }
            if family == 2 {
                index(leaf.area, map.areas.len(), "leaf area", i)?;
                if i == 0 && leaf.contents != 1 {
                    return Err(FormatError::InvalidValue);
                }
            } else if leaf.area < -1 {
                return Err(FormatError::InvalidValue);
            }
        }
    }
    for (i, node) in map.nodes.iter().enumerate() {
        index(i64::from(node.plane), map.planes.len(), "node plane", i)?;
        range(node.faces, map.faces.len(), "node faces", i)?;
        child(node.children[0], map, i)?;
        child(node.children[1], map, i)?;
    }
    for (i, node) in map.clipnodes.iter().enumerate() {
        index(i64::from(node.plane), map.planes.len(), "clipnode plane", i)?;
        for &value in &node.children {
            if value >= 0 {
                index(i64::from(value), map.clipnodes.len(), "clipnode child", i)?;
            }
        }
    }
    trees(map.nodes.len(), |at| map.nodes[at].children)?;
    trees(map.clipnodes.len(), |at| map.clipnodes[at].children)?;
    for (i, brush) in map.brushes.iter().enumerate() {
        range(brush.sides, map.brush_sides.len(), "brush sides", i)?;
        if let Some(shader) = brush.shader {
            index(i64::from(shader), map.shaders.len(), "brush shader", i)?;
        }
    }
    for (i, side) in map.brush_sides.iter().enumerate() {
        index(i64::from(side.plane), map.planes.len(), "brush plane", i)?;
        if let Some(texture) = side.texture_info {
            index(
                i64::from(texture),
                map.texture_info.len(),
                "side texture",
                i,
            )?;
        }
        if let Some(shader) = side.shader {
            index(i64::from(shader), map.shaders.len(), "side shader", i)?;
        }
    }
    let extra_brushes = map.extensions.iter().any(|e| e.name == b"BRUSHLIST");
    for (i, model) in map.models.iter().enumerate() {
        if !model.membership_from_tree {
            range(model.faces, faces, "model faces", i)?;
        }
        if format.modern_q3() {
            range(model.brushes, map.brushes.len(), "model brushes", i)?;
        } else {
            child(model.headnodes[0], map, i)?;
        }
        if family == 1 {
            if model.visible_leaves < 0
                || model.visible_leaves as usize > map.leaves.len().saturating_sub(1)
            {
                return Err(FormatError::InvalidReference("model visleafs", i));
            }
            for &root in &model.headnodes[1..] {
                if root >= 0 && (!map.clipnodes.is_empty() || !extra_brushes) {
                    index(i64::from(root), map.clipnodes.len(), "model clip root", i)?;
                }
            }
        }
    }
    for (i, area) in map.areas.iter().enumerate() {
        range(*area, map.area_portals.len(), "area portals", i)?;
    }
    for (i, portal) in map.area_portals.iter().enumerate() {
        index(
            i64::from(portal.portal),
            map.area_portals.len(),
            "portal number",
            i,
        )?;
        index(
            i64::from(portal.other_area),
            map.areas.len(),
            "portal area",
            i,
        )?;
    }
    for (i, fog) in map.fogs.iter().enumerate() {
        index(i64::from(fog.brush), map.brushes.len(), "fog brush", i)?;
        if fog.visible_side != -1 {
            index(
                i64::from(fog.visible_side),
                map.brushes[fog.brush as usize].sides.count as usize,
                "fog visible side",
                i,
            )?;
        }
    }
    for (i, surface) in map.surfaces.iter().enumerate() {
        if let Some(shader) = surface.shader {
            index(i64::from(shader), map.shaders.len(), "surface shader", i)?;
        }
        let retail_flare =
            surface.kind == SurfaceKind::Flare && surface.fog == 0 && map.fogs.is_empty();
        if surface.fog != -1 && !retail_flare {
            index(i64::from(surface.fog), map.fogs.len(), "surface fog", i)?;
        }
        if surface.brush_side != -1 {
            index(
                i64::from(surface.brush_side),
                map.brush_sides.len(),
                "surface brush side",
                i,
            )?;
        }
        range(surface.vertices, map.vertices.len(), "surface vertices", i)?;
        range(surface.indices, map.indices.len(), "surface indices", i)?;
        let planar = matches!(surface.kind, SurfaceKind::Planar | SurfaceKind::Patch);
        if planar && surface.lightmap < -4 {
            return Err(FormatError::InvalidReference("lightmap sentinel", i));
        }
        let embedded_lightmap =
            planar && surface.lightmap >= 0 && (surface.lightmap as usize) < lighting.len() / 49152;
        if surface.kind == SurfaceKind::Patch {
            let [width, height] = surface.patch;
            if width < 3
                || height < 3
                || width % 2 == 0
                || height % 2 == 0
                || i64::from(width) * i64::from(height) != i64::from(surface.vertices.count)
            {
                return Err(FormatError::InvalidReference("patch grid", i));
            }
        } else if surface.triangle_fan {
            if surface.vertices.count < 3 {
                return Err(FormatError::InvalidReference("polygon vertices", i));
            }
        } else if matches!(surface.kind, SurfaceKind::Planar | SurfaceKind::Triangles)
            && !surface.indices.count.is_multiple_of(3)
        {
            return Err(FormatError::InvalidReference("triangle count", i));
        }
        for at in surface.indices.indices() {
            index(
                i64::from(map.indices[at]),
                surface.vertices.count as usize,
                "triangle vertex",
                i,
            )?;
        }
        // Retail unlit vertices have unused NaN lightmap coordinates.
        if embedded_lightmap
            && map.vertices[surface.vertices.indices()]
                .iter()
                .any(|v| v.lightmap_coord.iter().any(|c| !c.is_finite()))
        {
            return Err(FormatError::InvalidReference("lit vertex coordinates", i));
        }
    }
    Ok(())
}

fn visibility_header(bytes: &[u8], family: u8) -> Result<Option<usize>, FormatError> {
    if family == 1 || bytes.is_empty() {
        return Ok(None);
    }
    let count = word(bytes, 0)? as usize;
    if family == 2 {
        if count > bytes.len().saturating_sub(4) / 8 {
            return Err(FormatError::InvalidRange);
        }
        for at in (4..4 + count * 8).step_by(4) {
            let offset = word(bytes, at)? as i32;
            if offset == -1 {
                continue;
            }
            if offset < 0 || (offset as usize) < 4 + count * 8 {
                return Err(FormatError::InvalidRange);
            }
            validate_rle(bytes, offset as usize, count.div_ceil(8))?;
        }
    } else {
        let row = word(bytes, 4)? as usize;
        if row < count.div_ceil(8)
            || count
                .checked_mul(row)
                .is_none_or(|length| length > bytes.len().saturating_sub(8))
        {
            return Err(FormatError::InvalidRange);
        }
    }
    Ok(Some(count))
}
fn validate_rle(bytes: &[u8], mut at: usize, length: usize) -> Result<(), FormatError> {
    let mut written = 0;
    while written < length {
        let byte = *bytes.get(at).ok_or(FormatError::Truncated)?;
        at += 1;
        let count = if byte != 0 {
            1
        } else {
            let run = *bytes.get(at).ok_or(FormatError::Truncated)?;
            at += 1;
            usize::from(run)
        };
        if count == 0 || count > length - written {
            return Err(FormatError::InvalidValue);
        }
        written += count;
    }
    Ok(())
}
