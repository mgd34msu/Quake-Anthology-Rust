//! Load-time BSP geometry conversion. Runtime drawing has no BSP-family branch.
//! Legacy extents follow WinQuake/model.c and Q2/ref_gl/gl_model.c;
//! patch subdivision follows Q3/renderer/tr_curve.c.
use crate::assets::Vertex;
use qa_core::primitives::{Axis, Bounds, Plane, Vec3};
use qa_formats::bsp::{BspFormat, IndexRange, LightmapSource, Lump, Map, SurfaceKind};

pub mod grid;
mod patch;

#[derive(Clone, Copy, Debug)]
pub struct GeometryOptions {
    /// Native r_subdivisions curvature tolerance, not a uniform segment count.
    pub patch_subdivisions: f32,
    pub max_surface_vertices: usize,
    pub max_light_samples: usize,
    /// Q3 map-overbright bits minus output-overbright bits. Zero retains raw
    /// colors; the stock presentation loader supplies its cached native value.
    pub vertex_color_shift: i8,
}
impl Default for GeometryOptions {
    fn default() -> Self {
        Self {
            patch_subdivisions: 4.0,
            max_surface_vertices: 65 * 65,
            max_light_samples: 16 * 1024 * 1024,
            vertex_color_shift: 0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GeometryError {
    InvalidOptions,
    Reference(&'static str, u32),
    NonFinite(&'static str, u32),
    EmptySurface(u32),
    TooManySurfaceVertices(u32),
    LightSpan(u32),
    InvalidPatch(u32),
    SizeLimit,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WorldVertex {
    pub vertex: Vertex,
    pub normal: Vec3,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GeometryKind {
    Polygon,
    Triangles,
    Patch,
    Flare,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextureCoordinates {
    /// Resolve texture dimensions at material registration, then normalize once.
    Texels,
    Normalized,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LightEncoding {
    Luminance,
    Rgb,
    PackedRgb,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LightSource {
    None,
    Vertex,
    White,
    TwoDimensional,
    Samples,
    /// Authored page shared by every surface referring to its numeric index.
    Page(u32),
    ExternalPage(u32),
}

/// The full pre-tessellated grid is stored in the shared vertex/index arrays.
/// Load-time stitching and shared LOD-error propagation update the full grid.
/// View-dependent grid simplification remains a frontend operation.
#[derive(Clone, Debug)]
pub struct PatchGrid {
    pub control_vertices: IndexRange,
    pub control_dimensions: [u16; 2],
    pub dimensions: [u16; 2],
    pub width_lod_error: Box<[f32]>,
    pub height_lod_error: Box<[f32]>,
    pub lod_origin: Vec3,
    pub lod_radius: f32,
}

#[derive(Clone, Debug)]
pub struct WorldSurface {
    /// Equal to the original BSP face/surface index, including non-drawing ones.
    pub source_id: u32,
    pub kind: GeometryKind,
    pub vertices: IndexRange,
    /// Triangle draw indices in WorldGeometry::indices, all globally addressed.
    pub indices: IndexRange,
    /// Entries in WorldGeometry::boundaries. Polygon loops or individual
    /// triangle loops keep the topology needed by the CPU edge renderer.
    pub boundaries: IndexRange,
    pub plane: Option<Plane>,
    pub bounds: Bounds,
    pub texture_coordinates: TextureCoordinates,
    pub texture_projection: [[f32; 4]; 2],
    pub texture_minima: [i32; 2],
    pub texture_extents: [u32; 2],
    pub lightmap_grid: [u32; 2],
    pub styles: [u8; 4],
    pub light_source: LightSource,
    pub light_encoding: LightEncoding,
    /// Samples are RGB triples; each active legacy style has a full grid.
    pub light_samples: IndexRange,
    pub source_texture: Option<u32>,
    pub source_texture_info: Option<u32>,
    pub source_shader: Option<u32>,
    pub source_flags: u32,
    pub source_contents: i32,
    pub no_draw: bool,
    pub source_fog: i32,
    pub source_brush_side: i32,
    pub source_lightmap: i32,
    pub lightmap_rect: [i32; 4],
    pub lightmap_origin: Vec3,
    pub lightmap_vectors: [Vec3; 3],
    pub patch: Option<PatchGrid>,
}

#[derive(Clone, Copy, Debug)]
pub struct WorldModel {
    pub bounds: Bounds,
    pub origin: Vec3,
    pub surfaces: IndexRange,
}
pub struct WorldGeometry {
    /// Only split, node-owned surfaces provide a BSP depth certificate. Other
    /// primitives use the same edge scanner's plane-depth ordering.
    pub partition: GeometryPartition,
    pub world_has_lightdata: bool,
    pub vertices: Vec<WorldVertex>,
    pub indices: Vec<u32>,
    /// Each range indexes an ordered loop in `indices`.
    pub boundaries: Vec<IndexRange>,
    pub surfaces: Vec<WorldSurface>,
    pub light_samples: Vec<[u8; 3]>,
    pub models: Vec<WorldModel>,
    pub patch_stats: PatchStats,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PatchStats {
    pub insertions: usize,
    pub lod_copies: usize,
    pub changed_grids: usize,
    pub reverse_endpoint_fixes: usize,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GeometryPartition {
    SplitBsp,
    Unpartitioned,
}

pub fn load_geometry(
    map: &Map<'_>,
    options: GeometryOptions,
) -> Result<WorldGeometry, GeometryError> {
    if !options.patch_subdivisions.is_finite()
        || options.patch_subdivisions <= 0.0
        || options.max_surface_vertices < 3
        || options.max_surface_vertices > u32::MAX as usize
        || !(-8..=8).contains(&options.vertex_color_shift)
    {
        return Err(GeometryError::InvalidOptions);
    }
    let mut geometry = WorldGeometry {
        partition: if map.bsp.format.family() == 3 {
            GeometryPartition::Unpartitioned
        } else {
            GeometryPartition::SplitBsp
        },
        world_has_lightdata: !map.bsp.bytes(Lump::Lighting).is_empty(),
        vertices: Vec::new(),
        indices: Vec::new(),
        boundaries: Vec::new(),
        surfaces: Vec::new(),
        light_samples: Vec::new(),
        models: map
            .models
            .iter()
            .map(|model| WorldModel {
                bounds: model.bounds,
                origin: model.origin,
                surfaces: model.faces,
            })
            .collect(),
        patch_stats: PatchStats::default(),
    };
    if map.bsp.format.family() == 3 {
        load_modern(map, options, &mut geometry)?;
    } else {
        load_legacy(map, options, &mut geometry)?;
    }
    geometry.patch_stats = prepare_patches(&mut geometry, options)?;
    Ok(geometry)
}

/// Load-only preparation, also usable by callers constructing owned geometry.
/// Inline-model ownership separates coincident groups that move independently.
pub fn prepare_patches(
    geometry: &mut WorldGeometry,
    options: GeometryOptions,
) -> Result<PatchStats, GeometryError> {
    let mut partitions = vec![0u32; geometry.surfaces.len()];
    for (index, model) in geometry.models.iter().enumerate().skip(1) {
        let owner = u32::try_from(index).map_err(|_| GeometryError::SizeLimit)?;
        for at in model.surfaces.indices() {
            let partition = partitions
                .get_mut(at)
                .ok_or(GeometryError::Reference("model surfaces", owner))?;
            if *partition != 0 {
                return Err(GeometryError::Reference(
                    "overlapping model surfaces",
                    at as u32,
                ));
            }
            *partition = owner;
        }
    }
    patch::finish_world_patches(geometry, options, &partitions)
}

fn load_legacy(
    map: &Map<'_>,
    options: GeometryOptions,
    out: &mut WorldGeometry,
) -> Result<(), GeometryError> {
    let family = map.bsp.format.family();
    for (id, face) in map.faces.iter().enumerate() {
        let id = u32::try_from(id).map_err(|_| GeometryError::SizeLimit)?;
        let count = face.edges.count as usize;
        if count < 3 {
            return Err(GeometryError::EmptySurface(id));
        }
        if count > options.max_surface_vertices {
            return Err(GeometryError::TooManySurfaceVertices(id));
        }
        let info = map
            .texture_info
            .get(face.texture_info as usize)
            .ok_or(GeometryError::Reference("texture info", id))?;
        let source_plane = *map
            .planes
            .get(face.plane as usize)
            .ok_or(GeometryError::Reference("plane", id))?;
        if source_plane.normal.0.iter().any(|v| !v.is_finite())
            || !source_plane.distance.is_finite()
        {
            return Err(GeometryError::NonFinite("plane", id));
        }
        let plane = if face.flags == 0 {
            source_plane
        } else {
            oriented_plane(scale(source_plane.normal, -1.0), -source_plane.distance)
        };
        let edges = section(&map.surface_edges, face.edges, "surface edges", id)?;
        let vertex_start = out.vertices.len();
        let mut uv_min = [f32::INFINITY; 2];
        let mut uv_max = [f32::NEG_INFINITY; 2];
        for &signed_edge in edges {
            let edge = map
                .edges
                .get(i64::from(signed_edge).unsigned_abs() as usize)
                .ok_or(GeometryError::Reference("edge", id))?;
            let index = edge[usize::from(signed_edge < 0)];
            let point = map
                .vertices
                .get(index as usize)
                .ok_or(GeometryError::Reference("vertex", id))?
                .position;
            let mut uv = [0.0; 2];
            for axis in 0..2 {
                let p = info.projection[axis];
                uv[axis] = point.0[0] * p[0] + point.0[1] * p[1] + point.0[2] * p[2] + p[3];
                if !uv[axis].is_finite() || point.0.iter().any(|v| !v.is_finite()) {
                    return Err(GeometryError::NonFinite("texture projection", id));
                }
                uv_min[axis] = uv_min[axis].min(uv[axis]);
                uv_max[axis] = uv_max[axis].max(uv[axis]);
            }
            out.vertices.push(WorldVertex {
                vertex: Vertex {
                    position: point,
                    normal: plane.normal,
                    texcoord: uv,
                    lightmap_coord: [0.0; 2],
                    color: [255; 4],
                },
                normal: plane.normal,
            });
        }
        let mut minima = [0i32; 2];
        let mut extents = [0u32; 2];
        for axis in 0..2 {
            let low = (uv_min[axis] / 16.0).floor() as f64 * 16.0;
            let high = (uv_max[axis] / 16.0).ceil() as f64 * 16.0;
            if low < i32::MIN as f64 || low > i32::MAX as f64 || high - low > u32::MAX as f64 {
                return Err(GeometryError::SizeLimit);
            }
            minima[axis] = low as i32;
            extents[axis] = (high - low) as u32;
        }
        let grid = [extents[0] / 16 + 1, extents[1] / 16 + 1];
        for value in &mut out.vertices[vertex_start..] {
            value.vertex.lightmap_coord = std::array::from_fn(|axis| {
                (value.vertex.texcoord[axis] - minima[axis] as f32 + 8.0)
                    / (grid[axis] as f32 * 16.0)
            });
        }
        let vertices = span(vertex_start, count)?;
        let boundary_start = out.boundaries.len();
        let loop_start = out.indices.len();
        out.indices
            .extend(vertices.indices().map(|value| value as u32));
        out.boundaries.push(span(loop_start, count)?);
        let index_start = out.indices.len();
        for at in 1..count - 1 {
            out.indices.extend_from_slice(&[
                vertices.first,
                vertices.first + at as u32,
                vertices.first + at as u32 + 1,
            ]);
        }
        let light_encoding = match map.bsp.format {
            BspFormat::Quake64 => LightEncoding::PackedRgb,
            BspFormat::HalfLife => LightEncoding::Rgb,
            _ if family == 2 => LightEncoding::Rgb,
            _ => LightEncoding::Luminance,
        };
        let samples = legacy_samples(
            map.bsp.bytes(Lump::Lighting),
            face.lighting_offset,
            face.styles,
            grid,
            light_encoding,
            id,
            options,
            &mut out.light_samples,
        )?;
        out.surfaces.push(WorldSurface {
            source_id: id,
            kind: GeometryKind::Polygon,
            vertices,
            indices: span(index_start, out.indices.len() - index_start)?,
            boundaries: span(boundary_start, 1)?,
            plane: Some(plane),
            bounds: vertex_bounds(&out.vertices[vertices.indices()])?,
            texture_coordinates: TextureCoordinates::Texels,
            texture_projection: info.projection,
            texture_minima: minima,
            texture_extents: extents,
            lightmap_grid: grid,
            styles: face.styles,
            light_source: if samples.count > 0 {
                LightSource::Samples
            } else {
                LightSource::None
            },
            light_encoding,
            light_samples: samples,
            source_texture: if family == 1 {
                u32::try_from(info.texture).ok()
            } else {
                None
            },
            source_texture_info: Some(face.texture_info),
            source_shader: None,
            source_flags: info.flags as u32,
            source_contents: 0,
            no_draw: family == 2 && info.flags & 0x80 != 0,
            source_fog: -1,
            source_brush_side: -1,
            source_lightmap: face.lighting_offset,
            lightmap_rect: [0, 0, grid[0] as i32, grid[1] as i32],
            lightmap_origin: Vec3::default(),
            lightmap_vectors: [Vec3::default(); 3],
            patch: None,
        });
    }
    Ok(())
}

fn legacy_samples(
    bytes: &[u8],
    offset: i32,
    styles: [u8; 4],
    grid: [u32; 2],
    encoding: LightEncoding,
    id: u32,
    options: GeometryOptions,
    out: &mut Vec<[u8; 3]>,
) -> Result<IndexRange, GeometryError> {
    if offset < 0 || bytes.is_empty() {
        return Ok(IndexRange::default());
    }
    let style_count = styles.iter().take_while(|&&style| style != 255).count();
    let count = (grid[0] as usize)
        .checked_mul(grid[1] as usize)
        .and_then(|v| v.checked_mul(style_count))
        .ok_or(GeometryError::LightSpan(id))?;
    let stride = match encoding {
        LightEncoding::Luminance => 1,
        LightEncoding::Rgb => 3,
        LightEncoding::PackedRgb => 2,
    };
    // Q64's reader already converts its byte offset to a sample offset.
    let start = (offset as usize)
        .checked_mul(if encoding == LightEncoding::PackedRgb {
            2
        } else {
            1
        })
        .ok_or(GeometryError::LightSpan(id))?;
    let end = count
        .checked_mul(stride)
        .and_then(|n| start.checked_add(n))
        .ok_or(GeometryError::LightSpan(id))?;
    let source = bytes.get(start..end).ok_or(GeometryError::LightSpan(id))?;
    let first = out.len();
    if first
        .checked_add(count)
        .is_none_or(|n| n > options.max_light_samples)
    {
        return Err(GeometryError::SizeLimit);
    }
    for sample in source.chunks_exact(stride) {
        out.push(match encoding {
            LightEncoding::Luminance => [sample[0]; 3],
            LightEncoding::Rgb => [sample[0], sample[1], sample[2]],
            // Quakespasm gl_model.c:1124-1140, retained by the C port reader.
            LightEncoding::PackedRgb => [
                sample[0] & 0xf8,
                ((sample[0] & 7) << 5) | ((sample[1] & 0xc0) >> 5),
                (sample[1] & 0x3f) << 2,
            ],
        });
    }
    span(first, count)
}

fn load_modern(
    map: &Map<'_>,
    options: GeometryOptions,
    out: &mut WorldGeometry,
) -> Result<(), GeometryError> {
    out.vertices.reserve(map.vertices.len());
    for (id, source) in map.vertices.iter().enumerate() {
        if source
            .position
            .0
            .iter()
            .chain(source.normal.0.iter())
            .chain(source.texcoord.iter())
            .any(|v| !v.is_finite())
        {
            return Err(GeometryError::NonFinite("vertex", id as u32));
        }
        out.vertices.push(WorldVertex {
            vertex: Vertex {
                position: source.position,
                normal: source.normal,
                texcoord: source.texcoord,
                // Retail unlit vertices can contain unused NaN page coordinates.
                lightmap_coord: source
                    .lightmap_coord
                    .map(|v| if v.is_finite() { v } else { 0.0 }),
                color: color_shift(source.color, options.vertex_color_shift),
            },
            normal: source.normal,
        });
    }
    let lighting = map.bsp.bytes(Lump::Lighting);
    if !lighting.len().is_multiple_of(128 * 128 * 3)
        || lighting.len() / 3 > options.max_light_samples
    {
        return Err(GeometryError::SizeLimit);
    }
    out.light_samples
        .extend(lighting.chunks_exact(3).map(|p| [p[0], p[1], p[2]]));
    for (id, source) in map.surfaces.iter().enumerate() {
        let id = u32::try_from(id).map_err(|_| GeometryError::SizeLimit)?;
        let controls = section(&out.vertices, source.vertices, "vertices", id)?;
        if controls.len() > options.max_surface_vertices {
            return Err(GeometryError::TooManySurfaceVertices(id));
        }
        if matches!(map.surface_lightmap(source), LightmapSource::Embedded(_))
            && section(&map.vertices, source.vertices, "source vertices", id)?
                .iter()
                .any(|v| v.lightmap_coord.iter().any(|c| !c.is_finite()))
        {
            return Err(GeometryError::NonFinite("lightmap coordinates", id));
        }
        let (flags, contents) = match source.shader {
            Some(index) => {
                let shader = map
                    .shaders
                    .get(index as usize)
                    .ok_or(GeometryError::Reference("shader", id))?;
                (shader.surface_flags as u32, shader.content_flags)
            }
            None => (0, 0),
        };
        let no_draw = flags & 0x80 != 0;
        let boundary_start = out.boundaries.len();
        let index_start = out.indices.len();
        let mut vertices = source.vertices;
        let mut patch = None;
        let kind = match source.kind {
            SurfaceKind::Patch => {
                let grid =
                    patch::subdivide(controls, source.patch, options.patch_subdivisions, id)?;
                if grid.vertices.len() > options.max_surface_vertices {
                    return Err(GeometryError::TooManySurfaceVertices(id));
                }
                vertices = span(out.vertices.len(), grid.vertices.len())?;
                out.vertices.extend_from_slice(&grid.vertices);
                let [width, height] = grid.dimensions;
                if !no_draw {
                    for row in 0..height as u32 - 1 {
                        for column in 0..width as u32 - 1 {
                            let a = vertices.first + row * width as u32 + column;
                            let b = a + width as u32;
                            push_triangle(out, [a, b, a + 1])?;
                            push_triangle(out, [a + 1, b, b + 1])?;
                        }
                    }
                }
                let origin = scale(
                    add(source.lightmap_vectors[0], source.lightmap_vectors[1]),
                    0.5,
                );
                let radius = length(sub(source.lightmap_vectors[0], origin));
                if !radius.is_finite() {
                    return Err(GeometryError::NonFinite("patch LOD", id));
                }
                patch = Some(PatchGrid {
                    control_vertices: source.vertices,
                    control_dimensions: [source.patch[0] as u16, source.patch[1] as u16],
                    dimensions: grid.dimensions,
                    width_lod_error: grid.width_lod_error,
                    height_lod_error: grid.height_lod_error,
                    lod_origin: origin,
                    lod_radius: radius,
                });
                GeometryKind::Patch
            }
            SurfaceKind::Flare => GeometryKind::Flare,
            _ if source.triangle_fan => {
                if controls.len() < 3 {
                    return Err(GeometryError::EmptySurface(id));
                }
                for at in 1..controls.len() as u32 - 1 {
                    out.indices.extend_from_slice(&[
                        vertices.first,
                        vertices.first + at,
                        vertices.first + at + 1,
                    ]);
                }
                let boundary_first = out.indices.len();
                out.indices.extend(vertices.indices().map(|v| v as u32));
                out.boundaries
                    .push(span(boundary_first, vertices.count as usize)?);
                GeometryKind::Polygon
            }
            _ => {
                for triangle in
                    section(&map.indices, source.indices, "indices", id)?.chunks_exact(3)
                {
                    let mut indices = [0; 3];
                    for axis in 0..3 {
                        let local = u32::try_from(triangle[axis])
                            .map_err(|_| GeometryError::Reference("triangle vertex", id))?;
                        if local >= vertices.count {
                            return Err(GeometryError::Reference("triangle vertex", id));
                        }
                        indices[axis] = vertices.first + local;
                    }
                    push_triangle(out, indices)?;
                }
                if source.kind == SurfaceKind::Planar {
                    GeometryKind::Polygon
                } else {
                    GeometryKind::Triangles
                }
            }
        };
        let vertex_data = section(&out.vertices, vertices, "vertices", id)?;
        let plane = if source.kind == SurfaceKind::Planar {
            let first = vertex_data.first().ok_or(GeometryError::EmptySurface(id))?;
            let normal = source.lightmap_vectors[2];
            let distance = normal.dot(first.vertex.position);
            if normal.0.iter().any(|v| !v.is_finite()) || !distance.is_finite() {
                return Err(GeometryError::NonFinite("plane", id));
            }
            Some(oriented_plane(normal, distance))
        } else {
            None
        };
        let bounds = if vertex_data.is_empty() {
            Bounds {
                mins: source.lightmap_origin,
                maxs: source.lightmap_origin,
            }
        } else {
            vertex_bounds(vertex_data)?
        };
        let (light_source, samples) = match map.surface_lightmap(source) {
            LightmapSource::Embedded(_) => (
                LightSource::Page(source.lightmap as u32),
                span(source.lightmap as usize * 128 * 128, 128 * 128)?,
            ),
            LightmapSource::External(page) => {
                (LightSource::ExternalPage(page), IndexRange::default())
            }
            LightmapSource::Vertex => (LightSource::Vertex, IndexRange::default()),
            LightmapSource::White => (LightSource::White, IndexRange::default()),
            LightmapSource::TwoDimensional => (LightSource::TwoDimensional, IndexRange::default()),
            LightmapSource::None => (LightSource::None, IndexRange::default()),
        };
        // Polygon fan boundaries were appended after triangles; draw only triangles.
        let index_count = if source.triangle_fan {
            vertices.count.saturating_sub(2) as usize * 3
        } else {
            out.indices.len() - index_start
        };
        out.surfaces.push(WorldSurface {
            source_id: id,
            kind,
            vertices,
            indices: span(index_start, index_count)?,
            boundaries: span(boundary_start, out.boundaries.len() - boundary_start)?,
            plane,
            bounds,
            texture_coordinates: TextureCoordinates::Normalized,
            texture_projection: [[0.0; 4]; 2],
            texture_minima: [0; 2],
            texture_extents: [0; 2],
            lightmap_grid: [128; 2],
            styles: [0, 255, 255, 255],
            light_source,
            light_encoding: LightEncoding::Rgb,
            light_samples: samples,
            source_texture: None,
            source_texture_info: None,
            source_shader: source.shader,
            source_flags: flags,
            source_contents: contents,
            no_draw,
            source_fog: source.fog,
            source_brush_side: source.brush_side,
            source_lightmap: source.lightmap,
            lightmap_rect: source.lightmap_rect,
            lightmap_origin: source.lightmap_origin,
            lightmap_vectors: source.lightmap_vectors,
            patch,
        });
    }
    Ok(())
}

fn push_triangle(out: &mut WorldGeometry, vertices: [u32; 3]) -> Result<(), GeometryError> {
    let at = out.indices.len();
    out.indices.extend_from_slice(&vertices);
    out.boundaries.push(span(at, 3)?);
    Ok(())
}
fn span(first: usize, count: usize) -> Result<IndexRange, GeometryError> {
    if first
        .checked_add(count)
        .is_none_or(|n| n > u32::MAX as usize)
    {
        return Err(GeometryError::SizeLimit);
    }
    Ok(IndexRange {
        first: first as u32,
        count: count as u32,
    })
}
fn section<'a, T>(
    values: &'a [T],
    range: IndexRange,
    field: &'static str,
    id: u32,
) -> Result<&'a [T], GeometryError> {
    let end = (range.first as usize)
        .checked_add(range.count as usize)
        .ok_or(GeometryError::Reference(field, id))?;
    values
        .get(range.first as usize..end)
        .ok_or(GeometryError::Reference(field, id))
}
fn vertex_bounds(vertices: &[WorldVertex]) -> Result<Bounds, GeometryError> {
    let first = vertices
        .first()
        .ok_or(GeometryError::SizeLimit)?
        .vertex
        .position;
    let mut bounds = Bounds {
        mins: first,
        maxs: first,
    };
    for vertex in vertices {
        for axis in 0..3 {
            let value = vertex.vertex.position.0[axis];
            if !value.is_finite() {
                return Err(GeometryError::SizeLimit);
            }
            bounds.mins.0[axis] = bounds.mins.0[axis].min(value);
            bounds.maxs.0[axis] = bounds.maxs.0[axis].max(value);
        }
    }
    Ok(bounds)
}
fn oriented_plane(normal: Vec3, distance: f32) -> Plane {
    let axis = (0..3)
        .find(|&a| normal.0[a] == 1.0 && (0..3).all(|b| b == a || normal.0[b] == 0.0))
        .map(|a| [Axis::X, Axis::Y, Axis::Z][a]);
    Plane {
        normal,
        distance,
        axis,
    }
}
fn color_shift(color: [u8; 4], shift: i8) -> [u8; 4] {
    // Q3 tr_bsp.c R_ColorShiftLightingBytes. Apply before patch interpolation.
    let mut rgb: [u32; 3] = std::array::from_fn(|axis| {
        if shift >= 0 {
            u32::from(color[axis]) << shift as u32
        } else {
            u32::from(color[axis]) >> shift.unsigned_abs() as u32
        }
    });
    let highest = rgb[0].max(rgb[1]).max(rgb[2]);
    if highest > 255 {
        for channel in &mut rgb {
            *channel = *channel * 255 / highest;
        }
    }
    [rgb[0] as u8, rgb[1] as u8, rgb[2] as u8, color[3]]
}
fn add(a: Vec3, b: Vec3) -> Vec3 {
    Vec3(std::array::from_fn(|i| a.0[i] + b.0[i]))
}
fn sub(a: Vec3, b: Vec3) -> Vec3 {
    Vec3(std::array::from_fn(|i| a.0[i] - b.0[i]))
}
fn scale(v: Vec3, scale: f32) -> Vec3 {
    Vec3(v.0.map(|v| v * scale))
}
fn length(v: Vec3) -> f32 {
    v.dot(v).sqrt()
}
