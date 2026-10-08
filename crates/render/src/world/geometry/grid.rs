//! Cold axial polygon subdivision, Q1/WinQuake/gl_warp.c:55-137 and
//! Q2/ref_gl/gl_warp.c:54-153. Source boundaries remain the CPU polygons.
use super::{GeometryError, GeometryKind, WorldGeometry, WorldVertex, section, span};
use qa_core::primitives::Vec3;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GridFan {
    PolygonAnchor,
    CenterFan,
}

#[derive(Clone, Copy, Debug)]
pub struct GridOptions {
    pub spacing: f32,
    pub fan: GridFan,
    /// Subtracted once from appended GPU UVs. Original CPU UVs stay intact.
    pub texture_offset: [f32; 2],
    pub max_boundary_vertices: usize,
    pub max_fragments: usize,
    pub max_output_vertices: usize,
    pub max_split_depth: usize,
}
impl Default for GridOptions {
    fn default() -> Self {
        Self {
            spacing: 128.0,
            fan: GridFan::PolygonAnchor,
            texture_offset: [0.0; 2],
            max_boundary_vertices: 65 * 65,
            max_fragments: 65_536,
            max_output_vertices: 1_048_576,
            max_split_depth: 128,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GridStats {
    pub fragments: usize,
    pub splits: usize,
    pub appended_vertices: usize,
    pub appended_indices: usize,
}

/// Subdivide one convex, planar polygon at load time. Incoming UVs already
/// select the material's projection; interpolation retains those coordinates.
/// Failure leaves the geometry unchanged. Successful calls append GL geometry
/// and replace only the triangle range, keeping original CPU boundary ranges.
pub fn subdivide_surface(
    geometry: &mut WorldGeometry,
    surface: usize,
    options: GridOptions,
) -> Result<GridStats, GeometryError> {
    if !options.spacing.is_finite()
        || options.spacing <= 0.0
        || options.texture_offset.iter().any(|v| !v.is_finite())
        || options.max_boundary_vertices < 3
        || options.max_boundary_vertices > u32::MAX as usize - 2
        || options.max_fragments == 0
        || options.max_output_vertices < 3
        || options.max_output_vertices > u32::MAX as usize
        || options.max_split_depth == 0
    {
        return Err(GeometryError::InvalidOptions);
    }
    let id = u32::try_from(surface).map_err(|_| GeometryError::SizeLimit)?;
    let source = geometry
        .surfaces
        .get(surface)
        .ok_or(GeometryError::Reference("grid surface", id))?;
    if source.kind != GeometryKind::Polygon || source.boundaries.count != 1 {
        return Err(GeometryError::Reference("grid polygon", source.source_id));
    }
    let source_id = source.source_id;
    let boundary = *section(
        &geometry.boundaries,
        source.boundaries,
        "grid boundary",
        source_id,
    )?
    .first()
    .ok_or(GeometryError::EmptySurface(source_id))?;
    let indices = section(&geometry.indices, boundary, "grid indices", source_id)?;
    if indices.len() < 3 {
        return Err(GeometryError::EmptySurface(source_id));
    }
    if indices.len() > options.max_boundary_vertices {
        return Err(GeometryError::TooManySurfaceVertices(source_id));
    }
    let mut polygon = Vec::with_capacity(indices.len());
    for &index in indices {
        let point = *geometry
            .vertices
            .get(index as usize)
            .ok_or(GeometryError::Reference("grid vertex", source_id))?;
        if point
            .vertex
            .position
            .0
            .iter()
            .chain(point.normal.0.iter())
            .chain(point.vertex.texcoord.iter())
            .chain(point.vertex.lightmap_coord.iter())
            .any(|n| !n.is_finite())
        {
            return Err(GeometryError::NonFinite("grid vertex", source_id));
        }
        polygon.push(point);
    }
    let mut pending = vec![(polygon, 0usize)];
    let mut vertices = Vec::new();
    let mut triangles = Vec::new();
    let mut stats = GridStats::default();
    while let Some((polygon, depth)) = pending.pop() {
        if let Some((axis, middle)) = split_plane(&polygon, options.spacing, source_id)? {
            if depth >= options.max_split_depth {
                return Err(GeometryError::SizeLimit);
            }
            let (front, back) = split(&polygon, axis, middle);
            if front.len() < 3 || back.len() < 3 {
                return Err(GeometryError::Reference("grid split", source_id));
            }
            if front.len().max(back.len()) > options.max_boundary_vertices {
                return Err(GeometryError::TooManySurfaceVertices(source_id));
            }
            stats.splits += 1;
            // A binary subdivision with n splits has n+1 final fragments.
            if stats.splits >= options.max_fragments {
                return Err(GeometryError::SizeLimit);
            }
            pending.push((back, depth + 1));
            pending.push((front, depth + 1));
            continue;
        }
        let extra = usize::from(options.fan == GridFan::CenterFan) * 2;
        let end = vertices
            .len()
            .checked_add(polygon.len())
            .and_then(|n| n.checked_add(extra))
            .ok_or(GeometryError::SizeLimit)?;
        if end > options.max_output_vertices {
            return Err(GeometryError::SizeLimit);
        }
        let first = u32::try_from(vertices.len()).map_err(|_| GeometryError::SizeLimit)?;
        match options.fan {
            GridFan::PolygonAnchor => {
                for i in 2..polygon.len() as u32 {
                    triangles.extend_from_slice(&[first, first + i - 1, first + i]);
                }
                vertices.extend_from_slice(&polygon);
            }
            GridFan::CenterFan => {
                vertices.push(center(&polygon));
                vertices.extend_from_slice(&polygon);
                vertices.push(polygon[0]);
                for i in 0..polygon.len() as u32 {
                    triangles.extend_from_slice(&[first, first + i + 1, first + i + 2]);
                }
            }
        }
        stats.fragments += 1;
    }
    for vertex in &mut vertices {
        for axis in 0..2 {
            vertex.vertex.texcoord[axis] -= options.texture_offset[axis];
        }
        if vertex
            .vertex
            .position
            .0
            .iter()
            .chain(vertex.normal.0.iter())
            .chain(vertex.vertex.texcoord.iter())
            .chain(vertex.vertex.lightmap_coord.iter())
            .any(|v| !v.is_finite())
        {
            return Err(GeometryError::NonFinite("grid output", source_id));
        }
    }
    let appended = span(geometry.vertices.len(), vertices.len())?;
    let indices = span(geometry.indices.len(), triangles.len())?;
    for index in &mut triangles {
        *index = index
            .checked_add(appended.first)
            .ok_or(GeometryError::SizeLimit)?;
    }
    stats.appended_vertices = vertices.len();
    stats.appended_indices = triangles.len();
    geometry.vertices.extend_from_slice(&vertices);
    geometry.indices.extend_from_slice(&triangles);
    geometry.surfaces[surface].indices = indices;
    Ok(stats)
}

fn split_plane(
    points: &[WorldVertex],
    spacing: f32,
    id: u32,
) -> Result<Option<(usize, f32)>, GeometryError> {
    for axis in 0..3 {
        let mut minimum = points[0].vertex.position.0[axis];
        let mut maximum = minimum;
        for point in &points[1..] {
            let value = point.vertex.position.0[axis];
            minimum = minimum.min(value);
            maximum = maximum.max(value);
        }
        let middle = (minimum + maximum) * 0.5;
        // Native division is f32; its .5 and floor operation are double.
        let middle = (f64::from(spacing) * (f64::from(middle / spacing) + 0.5).floor()) as f32;
        if !middle.is_finite() {
            return Err(GeometryError::NonFinite("grid plane", id));
        }
        if maximum - middle < 8.0 || middle - minimum < 8.0 {
            continue;
        }
        return Ok(Some((axis, middle)));
    }
    Ok(None)
}

fn split(points: &[WorldVertex], axis: usize, middle: f32) -> (Vec<WorldVertex>, Vec<WorldVertex>) {
    let mut front = Vec::with_capacity(points.len() + 2);
    let mut back = Vec::with_capacity(points.len() + 2);
    for i in 0..points.len() {
        let point = points[i];
        let next = points[(i + 1) % points.len()];
        let distance = point.vertex.position.0[axis] - middle;
        let next_distance = next.vertex.position.0[axis] - middle;
        if distance >= 0.0 {
            front.push(point);
        }
        if distance <= 0.0 {
            back.push(point);
        }
        if distance == 0.0 || next_distance == 0.0 || (distance > 0.0) == (next_distance > 0.0) {
            continue;
        }
        let point = interpolate(point, next, distance / (distance - next_distance));
        front.push(point);
        back.push(point);
    }
    (front, back)
}

fn interpolate(a: WorldVertex, b: WorldVertex, fraction: f32) -> WorldVertex {
    let mut out = a;
    out.vertex.position = a.vertex.position.lerp(b.vertex.position, fraction);
    out.normal = a.normal.lerp(b.normal, fraction);
    out.vertex.normal = out.normal;
    out.vertex.texcoord = std::array::from_fn(|i| {
        a.vertex.texcoord[i] + fraction * (b.vertex.texcoord[i] - a.vertex.texcoord[i])
    });
    out.vertex.lightmap_coord = std::array::from_fn(|i| {
        a.vertex.lightmap_coord[i]
            + fraction * (b.vertex.lightmap_coord[i] - a.vertex.lightmap_coord[i])
    });
    out.vertex.color = std::array::from_fn(|i| {
        (f32::from(a.vertex.color[i])
            + fraction * (f32::from(b.vertex.color[i]) - f32::from(a.vertex.color[i])))
        .round() as u8
    });
    out
}

fn center(points: &[WorldVertex]) -> WorldVertex {
    let mut out = points[0];
    let mut position = [0.0; 3];
    let mut normal = [0.0; 3];
    let mut uv = [0.0; 2];
    let mut lightmap = [0.0; 2];
    let mut color = [0.0; 4];
    for point in points {
        for i in 0..3 {
            position[i] += point.vertex.position.0[i];
            normal[i] += point.normal.0[i];
        }
        for i in 0..2 {
            uv[i] += point.vertex.texcoord[i];
            lightmap[i] += point.vertex.lightmap_coord[i];
        }
        for (sum, value) in color.iter_mut().zip(point.vertex.color) {
            *sum += f32::from(value);
        }
    }
    // Q2 VectorScale(total, 1.0/numverts) multiplies by a double reciprocal.
    let inverse = 1.0 / points.len() as f64;
    out.vertex.position = Vec3(position.map(|n| (f64::from(n) * inverse) as f32));
    out.normal = Vec3(normal.map(|n| n / points.len() as f32));
    out.vertex.normal = out.normal;
    out.vertex.texcoord = uv.map(|n| n / points.len() as f32);
    out.vertex.lightmap_coord = lightmap.map(|n| n / points.len() as f32);
    out.vertex.color = color.map(|n| (n / points.len() as f32).round() as u8);
    out
}
