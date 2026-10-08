//! The single world edge/span consumer. Map topology never falls back to mesh
//! triangles; polygon and tessellated-patch boundaries enter the same scanner.
use super::{Camera, ClipVertex, ScreenVertex};
use crate::assets::{Assets, DepthFunc, ImageId, Material, StageTexture, Vertex};
use crate::edges::{DepthPolicy, Edges, ProjectedVertex, Span};
use crate::scene::{CpuPresentation, SurfaceRef};
use crate::shader::{AlphaFunc, AlphaGen, BlendFactor, Cull, RgbGen, TexCoordGen};
use crate::stage::{DeformOp, DrawInputs, PreparedStage, StageEvaluator, alpha_pass};
use crate::surface_cache::{BuildState, CacheStats, LightGrid, SurfaceCache, SurfaceSource};
use crate::world::WorldId;
use crate::world::geometry::{GeometryPartition, TextureCoordinates};

#[derive(Clone, Copy, Debug)]
pub struct CpuLimits {
    pub cache_bytes: usize,
    pub max_spans: usize,
}
impl Default for CpuLimits {
    fn default() -> Self {
        Self {
            cache_bytes: 32 * 1024 * 1024,
            max_spans: 4096,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WorldStats {
    pub polygons: u64,
    pub spans: u64,
    pub pixels: u64,
    pub rejected: u64,
    pub cache: CacheStats,
}

#[derive(Clone, Copy, Default)]
struct SurfaceInfo {
    cache: u32,
    cache_supported: bool,
    mip_adjust: f32,
    sky: Option<super::sky::Source>,
    raster_empty: bool,
}

#[derive(Clone, Copy, Default)]
struct Affine {
    x: f32,
    y: f32,
    origin: f32,
}
impl Affine {
    fn at(self, x: f32, y: f32) -> f32 {
        self.origin + self.y * y + self.x * x
    }
    fn load(vertices: [ScreenVertex; 3], values: [f32; 3], area: f64) -> Self {
        let [a, b, c] = vertices.map(|v| v.xy.map(|value| f64::from(value - 0.5)));
        let ab = [b[0] - a[0], b[1] - a[1]];
        let ac = [c[0] - a[0], c[1] - a[1]];
        let vb = f64::from(values[1]) - f64::from(values[0]);
        let vc = f64::from(values[2]) - f64::from(values[0]);
        let x = (vb * ac[1] - vc * ab[1]) / area;
        let y = (ab[0] * vc - ac[0] * vb) / area;
        Self {
            x: x as f32,
            y: y as f32,
            origin: (f64::from(values[0]) - x * a[0] - y * a[1]) as f32,
        }
    }
}

#[derive(Clone, Copy, Default)]
struct Planes {
    inverse_depth: Affine,
    texture: [Affine; 2],
    lightmap: [Affine; 2],
    color: [Affine; 4],
}
impl Planes {
    fn load(vertices: &[ScreenVertex]) -> Option<Self> {
        let a = vertices[0];
        let mut largest = 0.0_f64;
        let mut basis = None;
        for pair in vertices[1..].windows(2) {
            let ab = [
                f64::from(pair[0].xy[0] - 0.5) - f64::from(a.xy[0] - 0.5),
                f64::from(pair[0].xy[1] - 0.5) - f64::from(a.xy[1] - 0.5),
            ];
            let ac = [
                f64::from(pair[1].xy[0] - 0.5) - f64::from(a.xy[0] - 0.5),
                f64::from(pair[1].xy[1] - 0.5) - f64::from(a.xy[1] - 0.5),
            ];
            let area = ab[0] * ac[1] - ac[0] * ab[1];
            if area.abs() > largest {
                largest = area.abs();
                basis = Some(([a, pair[0], pair[1]], area));
            }
        }
        let (triangle, area) = basis?;
        let planes = Self {
            inverse_depth: Affine::load(triangle, triangle.map(|v| v.inverse_depth), area),
            texture: std::array::from_fn(|axis| {
                Affine::load(
                    triangle,
                    triangle.map(|v| v.texcoord_over_depth[axis]),
                    area,
                )
            }),
            lightmap: std::array::from_fn(|axis| {
                Affine::load(
                    triangle,
                    triangle.map(|v| v.lightmap_over_depth[axis]),
                    area,
                )
            }),
            color: std::array::from_fn(|axis| {
                Affine::load(triangle, triangle.map(|v| v.color_over_depth[axis]), area)
            }),
        };
        planes.finite().then_some(planes)
    }
    fn finite(self) -> bool {
        std::iter::once(self.inverse_depth)
            .chain(self.texture)
            .chain(self.lightmap)
            .chain(self.color)
            .all(|p| [p.x, p.y, p.origin].iter().all(|v| v.is_finite()))
    }
}

#[derive(Clone, Copy, Default)]
struct Primitive {
    world: WorldId,
    reference: u32,
    surface: u32,
    boundary: u32,
    depth_key: u32,
    draw_rank: u32,
    cache: u32,
    cache_image: Option<ImageId>,
    sky: Option<super::sky::Source>,
    mip: u8,
    fixed_adjust: [i64; 2],
    planes: Planes,
    overlay: bool,
    first_stage: usize,
    stages: usize,
    deforms: [DeformOp; 3],
}

#[derive(Clone, Copy, Default)]
struct StagePlanes {
    prepared: Option<PreparedStage>,
    planes: Planes,
    sampler: Option<super::ShadeFn>,
}

pub(super) struct WorldRaster {
    width: u32,
    offsets: Box<[usize]>,
    surfaces: Box<[SurfaceInfo]>,
    primitives: Box<[Primitive]>,
    primitive_count: usize,
    stages: Box<[StagePlanes]>,
    stage_count: usize,
    clip_a: Box<[ClipVertex]>,
    clip_b: Box<[ClipVertex]>,
    screen: Box<[ScreenVertex]>,
    projected: Box<[ProjectedVertex]>,
    edges: Edges,
    cache: SurfaceCache,
    stats: WorldStats,
}

pub(super) struct Buffers<'a> {
    pub pixels: &'a mut [u32],
    pub inverse_depth: &'a mut [f32],
    pub depth_ranks: &'a mut [u32],
    pub indices: &'a mut [u8],
    pub palettes: &'a mut [u32],
}

impl WorldRaster {
    pub(super) fn load(
        width: u32,
        height: u32,
        assets: &Assets,
        limits: CpuLimits,
    ) -> Result<Self, &'static str> {
        let mut offsets = Vec::with_capacity(assets.worlds().len());
        let mut sources = Vec::new();
        let mut surfaces = Vec::new();
        let mut boundaries = 0usize;
        let mut max_vertices = 3usize;
        let mut max_edges = 0usize;
        let mut stage_capacity = 0usize;
        for world in assets.worlds() {
            offsets.push(sources.len());
            let geometry = world.geometry();
            for (surface_index, surface) in geometry.surfaces.iter().enumerate() {
                let cache_supported = surface.texture_coordinates == TextureCoordinates::Texels
                    && surface
                        .texture_extents
                        .iter()
                        .all(|&e| e > 0 && e <= 8192 && e.is_multiple_of(16));
                let lightmap = if cache_supported && surface.light_samples.count != 0 {
                    let samples = geometry
                        .light_samples
                        .get(surface.light_samples.indices())
                        .ok_or("invalid world light sample range")?;
                    Some(LightGrid::rgb(
                        surface.lightmap_grid[0],
                        surface.lightmap_grid[1],
                        surface.styles,
                        samples.as_flattened(),
                    )?)
                } else {
                    None
                };
                let source = SurfaceSource::load(
                    surface.texture_minima,
                    if cache_supported {
                        surface.texture_extents
                    } else {
                        [16; 2]
                    },
                    lightmap,
                    geometry.world_has_lightdata,
                )?;
                let cache = u32::try_from(sources.len()).map_err(|_| "too many world surfaces")?;
                sources.push(source);
                surfaces.push(SurfaceInfo {
                    cache,
                    cache_supported,
                    mip_adjust: mip_adjust(surface.texture_projection),
                    sky: None,
                    raster_empty: false,
                });
                let loops = geometry
                    .boundaries
                    .get(surface.boundaries.indices())
                    .ok_or("invalid world polygon boundary range")?;
                let binding = world
                    .bindings()
                    .get(surface_index)
                    .ok_or("invalid world surface binding")?;
                let material = assets
                    .material(binding.material)
                    .ok_or("invalid world material")?;
                if let Some(info) = surfaces.last_mut() {
                    info.sky = super::sky::Source::load(material, assets);
                }
                stage_capacity = stage_capacity
                    .checked_add(
                        loops
                            .len()
                            .checked_mul(material.stages.len())
                            .ok_or("world stage count overflow")?,
                    )
                    .ok_or("world stage count overflow")?;
                boundaries = boundaries
                    .checked_add(loops.len())
                    .ok_or("world polygon count overflow")?;
                let mut raster_empty = material.settings.deforms.iter().all(Option::is_none);
                for boundary in loops {
                    let indices = geometry
                        .indices
                        .get(boundary.indices())
                        .ok_or("invalid world polygon index range")?;
                    if indices.len() < 3
                        || indices
                            .iter()
                            .any(|&i| i as usize >= geometry.vertices.len())
                    {
                        return Err("invalid world polygon vertices");
                    }
                    raster_empty &= boundary_is_collinear(&geometry.vertices, indices);
                    let clipped = indices
                        .len()
                        .checked_add(6)
                        .ok_or("world clip count overflow")?;
                    max_vertices = max_vertices.max(clipped);
                    max_edges = max_edges
                        .checked_add(clipped)
                        .ok_or("world edge count overflow")?;
                }
                if let Some(info) = surfaces.last_mut() {
                    info.raster_empty = raster_empty;
                }
            }
        }
        Ok(Self {
            width,
            offsets: offsets.into_boxed_slice(),
            surfaces: surfaces.into_boxed_slice(),
            primitives: vec![Primitive::default(); boundaries.max(1)].into_boxed_slice(),
            primitive_count: 0,
            stages: vec![StagePlanes::default(); stage_capacity.max(1)].into_boxed_slice(),
            stage_count: 0,
            clip_a: vec![ClipVertex::default(); max_vertices].into_boxed_slice(),
            clip_b: vec![ClipVertex::default(); max_vertices].into_boxed_slice(),
            screen: vec![ScreenVertex::default(); max_vertices].into_boxed_slice(),
            projected: vec![ProjectedVertex::default(); max_vertices].into_boxed_slice(),
            edges: Edges::load(
                width,
                height,
                max_edges.max(3),
                boundaries.max(1),
                limits.max_spans,
            )?,
            cache: SurfaceCache::load(sources, limits.cache_bytes)?,
            stats: WorldStats::default(),
        })
    }

    pub(super) fn stats(&self) -> WorldStats {
        WorldStats {
            cache: self.cache.stats(),
            ..self.stats
        }
    }

    pub(super) fn reset_stats(&mut self) {
        self.stats = WorldStats::default();
    }

    pub(super) fn render_opaque(
        &mut self,
        camera: Camera,
        references: &[SurfaceRef],
        reference_base: u32,
        assets: &Assets,
        evaluator: &StageEvaluator,
        mut buffers: Buffers<'_>,
        stats: &mut crate::BackendStats,
    ) {
        let old_rejected = self.stats.rejected;
        let backend_rejected = stats.rejected;
        self.primitive_count = 0;
        self.stage_count = 0;
        let mut background = None;
        let first_world = references.first().map(|r| r.world);
        let mut certified = first_world.is_some_and(|id| {
            assets
                .world(id)
                .is_some_and(|world| world.geometry().partition == GeometryPartition::SplitBsp)
        });
        for (reference_index, reference) in references.iter().enumerate() {
            certified &= Some(reference.world) == first_world;
            let Some(world) = assets.world(reference.world) else {
                self.reject(stats);
                continue;
            };
            let Some(surface) = world.geometry().surfaces.get(reference.surface as usize) else {
                self.reject(stats);
                continue;
            };
            let Some(binding) = world.bindings().get(reference.surface as usize) else {
                self.reject(stats);
                continue;
            };
            let Some(material) = assets.material(binding.material) else {
                self.reject(stats);
                continue;
            };
            let Some(info) = self
                .offsets
                .get(reference.world.0 as usize)
                .and_then(|offset| self.surfaces.get(offset + reference.surface as usize))
                .copied()
            else {
                self.reject(stats);
                continue;
            };
            if info.raster_empty {
                continue;
            }
            if material.settings.sky.is_some() {
                let Some(source) = info.sky else {
                    self.reject(stats);
                    continue;
                };
                if !matches!(camera.refdef.cpu_presentation, CpuPresentation::Indexed { palette, .. } if assets.palette(palette).is_some())
                {
                    self.reject(stats);
                    continue;
                }
                if self.add_sky(
                    camera,
                    *reference,
                    reference_base + reference_index as u32,
                    source,
                    material,
                    assets,
                    evaluator,
                    stats,
                ) {
                    stats.surfaces = stats.surfaces.saturating_add(1);
                    if matches!(source, super::sky::Source::BackgroundCube { .. }) {
                        if background.is_some_and(|old| old != source) {
                            // Multiple independently selected full-view backgrounds
                            // need a combined-game policy, not a backend sorter.
                            self.reject(stats);
                        } else {
                            background = Some(source);
                        }
                    }
                }
                continue;
            }
            if material.stages.is_empty()
                || material.settings.fog.is_some()
                || material.settings.portal
                || material.settings.polygon_offset
            {
                self.reject(stats);
                continue;
            }
            certified &= material.settings.deforms.iter().all(Option::is_none);
            let inputs = DrawInputs {
                time_ms: camera.refdef.time_ms,
                view_origin: camera.refdef.origin,
                identity_light: camera.refdef.identity_light,
                lightmap: binding.lightmap,
                texture_scale: binding.texture_scale,
                ..DrawInputs::default()
            };
            let Ok(base) = evaluator.prepare(&material.stages[0], material.settings, inputs) else {
                self.reject(stats);
                continue;
            };
            let Ok(deforms) = evaluator.prepare_deforms(&material.settings, &inputs) else {
                self.reject(stats);
                continue;
            };
            let native = match camera.refdef.cpu_presentation {
                CpuPresentation::Rgb => None,
                CpuPresentation::Indexed { palette, .. } => {
                    if assets.palette(palette).is_none()
                        || !info.cache_supported
                        || !cached_material(material)
                        || assets
                            .image(base.image)
                            .is_none_or(|image| image.indexed.is_none())
                    {
                        self.reject(stats);
                        continue;
                    }
                    Some(base.image)
                }
            };
            let overlay = material.stages[0].blend.is_some()
                || material.stages[0].alpha_test != AlphaFunc::None
                || !material.stages[0].depth_write
                || material.stages[0].depth_func != DepthFunc::Lequal
                || native.is_some_and(|image| {
                    assets
                        .image(image)
                        .and_then(|i| i.indexed.as_ref())
                        .is_some_and(|t| t.cutout())
                });
            let loops = surface.boundaries.indices();
            if loops.is_empty() {
                self.reject(stats);
                continue;
            }
            let before = self.primitive_count;
            for boundary in loops {
                if self.primitive_count == self.primitives.len() {
                    self.reject(stats);
                    break;
                }
                let mut primitive = Primitive {
                    world: reference.world,
                    reference: reference_base + reference_index as u32,
                    surface: reference.surface,
                    boundary: boundary as u32,
                    depth_key: reference.depth_key,
                    draw_rank: reference.draw_rank,
                    cache: info.cache,
                    cache_image: native,
                    sky: None,
                    mip: 0,
                    fixed_adjust: [0; 2],
                    planes: Planes::default(),
                    overlay,
                    first_stage: self.stage_count,
                    stages: 0,
                    deforms,
                };
                let Some(count) = self.clip(
                    camera,
                    assets,
                    primitive,
                    if native.is_some() { None } else { Some(base) },
                    evaluator,
                ) else {
                    self.reject(stats);
                    continue;
                };
                if count < 3 {
                    continue;
                }
                let area = polygon_area(&self.screen[..count]);
                if area == 0.0
                    || match material.settings.cull {
                        Cull::Front => area < 0.0,
                        Cull::Back => area > 0.0,
                        Cull::None => false,
                    }
                {
                    continue;
                }
                if native.is_some() {
                    let nearzi = self.screen[..count]
                        .iter()
                        .map(|v| v.inverse_depth)
                        .fold(0.0, f32::max);
                    let scale = (camera.refdef.viewport.width as f32 / (2.0 * camera.tangent[0]))
                        .max(camera.refdef.viewport.height as f32 / (2.0 * camera.tangent[1]));
                    let distance = nearzi * scale * info.mip_adjust;
                    primitive.mip = if distance >= 1.0 {
                        0
                    } else if distance >= 0.4 {
                        1
                    } else if distance >= 0.2 {
                        2
                    } else {
                        3
                    };
                    if primitive.mip != 0 {
                        let Some(next) = self.clip(camera, assets, primitive, None, evaluator)
                        else {
                            self.reject(stats);
                            continue;
                        };
                        if next < 3 {
                            continue;
                        }
                    }
                    let Some((planes, adjust)) = native_planes(camera, surface, primitive.mip)
                    else {
                        self.reject(stats);
                        continue;
                    };
                    primitive.planes = planes;
                    primitive.fixed_adjust = adjust;
                } else {
                    if self.stage_count + material.stages.len() > self.stages.len() {
                        self.reject(stats);
                        continue;
                    }
                    let mut valid = true;
                    for stage in material.stages.iter() {
                        let Ok(prepared) = evaluator.prepare(stage, material.settings, inputs)
                        else {
                            valid = false;
                            break;
                        };
                        if assets.image(prepared.image).is_none() {
                            valid = false;
                            break;
                        }
                        let Some(stage_count) =
                            self.clip(camera, assets, primitive, Some(prepared), evaluator)
                        else {
                            valid = false;
                            break;
                        };
                        if stage_count < 3 {
                            valid = false;
                            break;
                        }
                        let Some(planes) = Planes::load(&self.screen[..stage_count]) else {
                            valid = false;
                            break;
                        };
                        self.stages[self.stage_count] = StagePlanes {
                            prepared: Some(prepared),
                            planes,
                            sampler: Some(super::stage_sampler(prepared.stage.sampler)),
                        };
                        self.stage_count += 1;
                        primitive.stages += 1;
                    }
                    if !valid {
                        self.stage_count = primitive.first_stage;
                        self.reject(stats);
                        continue;
                    }
                    // Return coverage coordinates to the first stage. Its
                    // deformation is identical for every stage of this draw.
                    let Some(first_count) =
                        self.clip(camera, assets, primitive, Some(base), evaluator)
                    else {
                        self.stage_count = primitive.first_stage;
                        self.reject(stats);
                        continue;
                    };
                    if first_count < 3 {
                        self.stage_count = primitive.first_stage;
                        continue;
                    }
                    primitive.planes = self.stages[primitive.first_stage].planes;
                }
                self.primitives[self.primitive_count] = primitive;
                self.primitive_count += 1;
            }
            if self.primitive_count > before {
                stats.surfaces = stats.surfaces.saturating_add(1);
            }
        }
        let policy = if certified {
            DepthPolicy::BspKeys
        } else {
            DepthPolicy::PlaneDepth
        };
        if !self.edges.begin_with_policy(camera.refdef.viewport, policy) {
            self.reject(stats);
            return;
        }
        if let Some(source) = background {
            super::sky::background(
                self.width,
                source,
                camera,
                assets,
                &mut buffers,
                &mut self.stats,
            );
        }
        for index in 0..self.primitive_count {
            let primitive = self.primitives[index];
            if primitive.overlay {
                continue;
            }
            let prepared = if primitive.cache_image.is_some() || primitive.sky.is_some() {
                None
            } else {
                self.stages[primitive.first_stage].prepared
            };
            if let Some(count) = self.clip(camera, assets, primitive, prepared, evaluator) {
                if count >= 3
                    && !self.edges.add_polygon(
                        index as u32,
                        primitive.depth_key,
                        primitive.draw_rank,
                        &self.projected[..count],
                    )
                {
                    self.reject(stats);
                }
            } else {
                self.reject(stats);
            }
        }
        let width = self.width;
        let primitives = &self.primitives;
        let stages = &self.stages;
        let cache = &mut self.cache;
        let counters = &mut self.stats;
        let edge_stats = self.edges.scan(|spans| {
            for &span in spans {
                consume_span(
                    width,
                    span,
                    primitives[span.surface as usize],
                    stages,
                    cache,
                    assets,
                    camera,
                    &mut buffers,
                    counters,
                );
            }
        });
        add_edge_stats(&mut self.stats, edge_stats);
        stats.stages = stats
            .stages
            .saturating_add(self.stage_count.min(u32::MAX as usize) as u32);
        stats.rejected = backend_rejected
            .saturating_add((self.stats.rejected - old_rejected).min(u32::MAX as u64) as u32);
    }

    /// Only the frontend chooses material/entity ordering. Deferred boundary
    /// spans consume that order while the opaque GET retains its BSP ordering.
    pub(super) fn draw_surface(
        &mut self,
        camera: Camera,
        reference: u32,
        assets: &Assets,
        evaluator: &StageEvaluator,
        mut buffers: Buffers<'_>,
        stats: &mut crate::BackendStats,
    ) {
        let old_rejected = self.stats.rejected;
        let backend_rejected = stats.rejected;
        let width = self.width;
        for index in 0..self.primitive_count {
            if self.primitives[index].reference != reference || !self.primitives[index].overlay {
                continue;
            }
            let primitive = self.primitives[index];
            let prepared = if primitive.cache_image.is_some() || primitive.sky.is_some() {
                None
            } else {
                self.stages[primitive.first_stage].prepared
            };
            let Some(count) = self.clip(camera, assets, primitive, prepared, evaluator) else {
                self.reject(stats);
                continue;
            };
            if count < 3 {
                continue;
            }
            if !self
                .edges
                .begin_with_policy(camera.refdef.viewport, DepthPolicy::PlaneDepth)
                || !self.edges.add_polygon(
                    index as u32,
                    primitive.depth_key,
                    primitive.draw_rank,
                    &self.projected[..count],
                )
            {
                self.reject(stats);
                continue;
            }
            let stages = &self.stages;
            let cache = &mut self.cache;
            let counters = &mut self.stats;
            let edge_stats = self.edges.scan(|spans| {
                for &span in spans {
                    consume_span(
                        width,
                        span,
                        primitive,
                        stages,
                        cache,
                        assets,
                        camera,
                        &mut buffers,
                        counters,
                    );
                }
            });
            add_edge_stats(&mut self.stats, edge_stats);
        }
        stats.rejected = backend_rejected
            .saturating_add((self.stats.rejected - old_rejected).min(u32::MAX as u64) as u32);
    }

    fn reject(&mut self, stats: &mut crate::BackendStats) {
        self.stats.rejected = self.stats.rejected.saturating_add(1);
        stats.rejected = stats.rejected.saturating_add(1);
    }

    fn add_sky(
        &mut self,
        camera: Camera,
        reference: SurfaceRef,
        reference_index: u32,
        source: super::sky::Source,
        material: &Material,
        assets: &Assets,
        evaluator: &StageEvaluator,
        stats: &mut crate::BackendStats,
    ) -> bool {
        let Some(world) = assets.world(reference.world) else {
            self.reject(stats);
            return false;
        };
        let surface = &world.geometry().surfaces[reference.surface as usize];
        let mut visible = false;
        for boundary in surface.boundaries.indices() {
            let mut primitive = Primitive {
                world: reference.world,
                surface: reference.surface,
                boundary: boundary as u32,
                reference: reference_index,
                depth_key: reference.depth_key,
                draw_rank: reference.draw_rank,
                sky: Some(source),
                ..Primitive::default()
            };
            let Some(count) = self.clip(camera, assets, primitive, None, evaluator) else {
                self.reject(stats);
                continue;
            };
            if count < 3 {
                continue;
            }
            let area = polygon_area(&self.screen[..count]);
            if area == 0.0
                || match material.settings.cull {
                    Cull::Front => area < 0.0,
                    Cull::Back => area > 0.0,
                    Cull::None => false,
                }
            {
                continue;
            }
            if matches!(source, super::sky::Source::BackgroundCube { .. }) {
                visible = true;
                continue;
            }
            let Some(planes) = Planes::load(&self.screen[..count]) else {
                self.reject(stats);
                continue;
            };
            if self.primitive_count == self.primitives.len() {
                self.reject(stats);
                break;
            }
            primitive.planes = planes;
            self.primitives[self.primitive_count] = primitive;
            self.primitive_count += 1;
            visible = true;
        }
        visible
    }

    fn clip(
        &mut self,
        camera: Camera,
        assets: &Assets,
        primitive: Primitive,
        prepared: Option<PreparedStage>,
        evaluator: &StageEvaluator,
    ) -> Option<usize> {
        let world = assets.world(primitive.world)?;
        let geometry = world.geometry();
        let surface = &geometry.surfaces[primitive.surface as usize];
        let boundary = geometry.boundaries[primitive.boundary as usize];
        let indices = &geometry.indices[boundary.indices()];
        let mut count = indices.len();
        if count > self.clip_a.len() {
            return None;
        }
        for (output, &index) in self.clip_a.iter_mut().zip(indices) {
            let loaded = geometry.vertices[index as usize];
            let mut vertex = Vertex {
                normal: loaded.normal,
                ..loaded.vertex
            };
            if primitive.cache_image.is_some() {
                vertex.texcoord = std::array::from_fn(|axis| {
                    let projection = surface.texture_projection[axis];
                    (vertex.position.0[0] * projection[0]
                        + vertex.position.0[1] * projection[1]
                        + vertex.position.0[2] * projection[2]
                        + projection[3]
                        - surface.texture_minima[axis] as f32)
                        / (1u32 << primitive.mip) as f32
                });
            }
            if let Some(prepared) = prepared {
                vertex = evaluator.apply_deforms(primitive.deforms, vertex);
                let evaluated = evaluator.evaluate(&prepared, &vertex);
                vertex = Vertex {
                    texcoord: evaluated.texcoord,
                    color: evaluated.color,
                    position: evaluated.position,
                    ..vertex
                };
            }
            *output = camera.vertex(vertex, vertex.position);
            if !output.finite() {
                return None;
            }
        }
        for plane in 0..6 {
            count = clip_world_plane(camera, &self.clip_a[..count], &mut self.clip_b, plane)?;
            if count < 3 {
                return Some(0);
            }
            std::mem::swap(&mut self.clip_a, &mut self.clip_b);
        }
        for i in 0..count {
            let screen = camera.project(self.clip_a[i]);
            if !screen.finite() {
                return None;
            }
            self.screen[i] = screen;
            self.projected[i] = ProjectedVertex {
                xy: screen.xy.map(|v| v - 0.5),
                inverse_depth: screen.inverse_depth,
                texcoord_over_depth: screen.texcoord_over_depth,
            };
        }
        Some(count)
    }
}

/// Used only for materials without positional deforms. Retail PVS lists can
/// retain repeated/collinear faces. A zero texture extent does not prove zero area.
fn boundary_is_collinear(
    vertices: &[crate::world::geometry::WorldVertex],
    indices: &[u32],
) -> bool {
    let origin = vertices[indices[0] as usize].vertex.position.0;
    let Some(end) = indices
        .iter()
        .map(|&index| vertices[index as usize].vertex.position.0)
        .find(|&position| position != origin)
    else {
        return true;
    };
    let Some(direction) = exact_difference(end, origin) else {
        return false;
    };
    for &index in indices {
        let Some(delta) = exact_difference(vertices[index as usize].vertex.position.0, origin)
        else {
            return false;
        };
        for axis in 0..3 {
            let next = (axis + 1) % 3;
            let left = direction[axis] * delta[next];
            let right = direction[next] * delta[axis];
            if left != right
                || direction[axis].mul_add(delta[next], -left)
                    != direction[next].mul_add(delta[axis], -right)
            {
                return false;
            }
        }
    }
    true
}

/// TwoDiff exposes any lost low bits. Uncertain extreme-range geometry stays
/// drawable; f32 differences/products cannot underflow or overflow f64 here.
fn exact_difference(point: [f32; 3], origin: [f32; 3]) -> Option<[f64; 3]> {
    let mut delta = [0.0; 3];
    for axis in 0..3 {
        let a = f64::from(point[axis]);
        let b = f64::from(origin[axis]);
        let difference = a - b;
        let b_virtual = a - difference;
        let a_virtual = difference + b_virtual;
        if (a - a_virtual) + (b_virtual - b) != 0.0 {
            return None;
        }
        delta[axis] = difference;
    }
    Some(delta)
}

/// WinQuake D_CalcGradients and ref_soft D_CalcGradients keep camera-space
/// texture gradients separate from their fixed-point texture-minimum offset.
fn native_planes(
    camera: Camera,
    surface: &crate::world::geometry::WorldSurface,
    mip: u8,
) -> Option<(Planes, [i64; 2])> {
    let plane = surface.plane?;
    let scale = [
        camera.refdef.viewport.width as f32 / (2.0 * camera.tangent[0]),
        camera.refdef.viewport.height as f32 / (2.0 * camera.tangent[1]),
    ];
    let center = [
        camera.refdef.viewport.x as f32 + camera.refdef.viewport.width as f32 * 0.5 - 0.5,
        camera.refdef.viewport.y as f32 + camera.refdef.viewport.height as f32 * 0.5 - 0.5,
    ];
    let right = qa_core::primitives::Vec3(camera.refdef.axes[1].0.map(|value| -value));
    let inverse_distance = 1.0 / (plane.distance - plane.normal.dot(camera.refdef.origin));
    if !inverse_distance.is_finite() {
        return None;
    }
    let x = plane.normal.dot(right) * inverse_distance / scale[0];
    let y = -plane.normal.dot(camera.refdef.axes[2]) * inverse_distance / scale[1];
    let inverse_depth = Affine {
        x,
        y,
        origin: plane.normal.dot(camera.refdef.axes[0]) * inverse_distance
            - center[0] * x
            - center[1] * y,
    };
    let mip_scale = 1.0 / (1u32 << mip) as f32;
    let axes = surface.texture_projection.map(|projection| {
        let axis = qa_core::primitives::Vec3([projection[0], projection[1], projection[2]]);
        qa_core::primitives::Vec3([
            axis.dot(right),
            axis.dot(camera.refdef.axes[2]),
            axis.dot(camera.refdef.axes[0]),
        ])
    });
    // Preserve D_CalcGradients' division/multiply order, transformed model
    // origin and post-transform mip scaling. Rotated views can otherwise
    // cross a fixed16 texel boundary despite mathematical equivalence.
    let texture = axes.map(|axis| {
        let x = axis.0[0] * ((1.0 / scale[0]) * mip_scale);
        let y = -axis.0[1] * ((1.0 / scale[1]) * mip_scale);
        Affine {
            x,
            y,
            origin: axis.0[2] * mip_scale - center[0] * x - center[1] * y,
        }
    });
    let transformed_origin = qa_core::primitives::Vec3([
        camera.refdef.origin.dot(right),
        camera.refdef.origin.dot(camera.refdef.axes[2]),
        camera.refdef.origin.dot(camera.refdef.axes[0]),
    ]);
    let scaled_origin =
        qa_core::primitives::Vec3(transformed_origin.0.map(|value| value * mip_scale));
    let adjust = std::array::from_fn(|i| {
        let projection = surface.texture_projection[i];
        let camera_fixed = (scaled_origin.dot(axes[i]) * 65536.0 + 0.5) as i64;
        let minimum = (i64::from(surface.texture_minima[i]) << 16) >> mip;
        ((camera_fixed - minimum) as f32 + projection[3] * (65536.0 * mip_scale)) as i64
    });
    let planes = Planes {
        inverse_depth,
        texture,
        ..Planes::default()
    };
    planes.finite().then_some((planes, adjust))
}

fn polygon_area(vertices: &[ScreenVertex]) -> f32 {
    let a = vertices[0];
    vertices[1..]
        .windows(2)
        .map(|pair| super::edge(a.xy, pair[0].xy, pair[1].xy))
        .sum()
}

fn cached_material(material: &Material) -> bool {
    if material.settings.deforms.iter().any(Option::is_some) {
        return false;
    }
    let base = &material.stages[0];
    let simple = base.blend.is_none()
        && base.depth_write
        && base.depth_func == DepthFunc::Lequal
        && base.rgb_gen == RgbGen::Identity
        && base.alpha_gen == AlphaGen::Identity
        && base.texgen == TexCoordGen::Texture
        && base.tcmods.iter().all(Option::is_none)
        && base.sampler.wrap == crate::assets::Wrap::Repeat
        && !matches!(base.texture, StageTexture::Lightmap)
        && matches!(
            base.alpha_test,
            AlphaFunc::None | AlphaFunc::GreaterZero | AlphaFunc::AtLeastHalf
        );
    if !simple {
        return false;
    }
    match &*material.stages {
        [_] => true,
        [_, light] => {
            matches!(light.texture, StageTexture::Lightmap)
                && light.texgen == TexCoordGen::Lightmap
                && light.rgb_gen == RgbGen::Identity
                && light.alpha_gen == AlphaGen::Identity
                && light.tcmods.iter().all(Option::is_none)
                && light.alpha_test == AlphaFunc::None
                && light.depth_func == DepthFunc::Equal
                && !light.depth_write
                && light.blend.is_some_and(|blend| {
                    blend.source == BlendFactor::DestinationColor
                        && blend.destination == BlendFactor::Zero
                })
        }
        _ => false,
    }
}

fn add_edge_stats(stats: &mut WorldStats, edges: crate::edges::Stats) {
    stats.polygons = stats.polygons.saturating_add(edges.polygons);
    stats.spans = stats.spans.saturating_add(edges.spans);
    stats.rejected = stats.rejected.saturating_add(edges.rejected);
}

fn consume_span(
    width: u32,
    span: Span,
    primitive: Primitive,
    stages: &[StagePlanes],
    cache: &mut SurfaceCache,
    assets: &Assets,
    camera: Camera,
    buffers: &mut Buffers<'_>,
    stats: &mut WorldStats,
) {
    let refdef = camera.refdef;
    if let Some(source) = primitive.sky {
        let depth = primitive.planes.inverse_depth;
        super::sky::layered_span(
            width,
            (buffers.pixels.len() / width as usize) as u32,
            span,
            source,
            camera,
            assets,
            [depth.x, depth.y, depth.origin],
            primitive.draw_rank,
            buffers,
            stats,
        );
        return;
    }
    if let Some(image) = primitive.cache_image {
        let CpuPresentation::Indexed {
            palette,
            lighting,
            ambient,
            fullbright,
        } = refdef.cpu_presentation
        else {
            stats.rejected += 1;
            return;
        };
        let Some(texture) = assets.image(image).and_then(|image| image.indexed.as_ref()) else {
            stats.rejected += 1;
            return;
        };
        let Some(palette_resource) = assets.palette(palette) else {
            stats.rejected += 1;
            return;
        };
        let Some(source) = cache.surface(primitive.cache) else {
            stats.rejected += 1;
            return;
        };
        let styles = source.lightmap().map_or([255; 4], |grid| grid.styles());
        let state = BuildState {
            texture_id: image.0,
            style_scales: styles.map(|style| {
                if style == 255 {
                    0
                } else {
                    refdef.lightstyles[style as usize].indexed_scale
                }
            }),
            ambient,
            fullbright,
            lighting,
            ..BuildState::default()
        };
        if !cache.begin_batch() {
            stats.rejected += 1;
            return;
        }
        let cached = cache.prepare(
            primitive.cache,
            primitive.mip,
            texture,
            palette_resource,
            state,
        );
        if let Some(cached) = cached {
            if let Some(texels) = cache.pixels(cached) {
                native_span(
                    width,
                    span,
                    primitive.planes,
                    primitive.fixed_adjust,
                    primitive.draw_rank,
                    cached.width,
                    cached.height,
                    cached.transparent_index,
                    texels,
                    palette_resource,
                    palette.0,
                    refdef.perspective_step.pixels(),
                    buffers,
                    stats,
                );
            } else {
                stats.rejected += 1;
            }
        } else {
            stats.rejected += 1;
        }
        cache.end_batch();
    } else {
        for stage in &stages[primitive.first_stage..primitive.first_stage + primitive.stages] {
            let Some(prepared) = stage.prepared else {
                stats.rejected += 1;
                continue;
            };
            let Some(image) = assets.image(prepared.image) else {
                stats.rejected += 1;
                continue;
            };
            let Some(sampler) = stage.sampler else {
                stats.rejected += 1;
                continue;
            };
            for x in span.x..span.x + span.count {
                let px = x as f32;
                let py = span.y as f32;
                let zi = stage.planes.inverse_depth.at(px, py);
                if zi <= 0.0 || !zi.is_finite() {
                    continue;
                }
                let index = span.y as usize * width as usize + x as usize;
                if !super::depth_passes(
                    prepared.stage.depth_func,
                    zi,
                    buffers.inverse_depth[index],
                    primitive.draw_rank,
                    buffers.depth_ranks[index],
                ) {
                    continue;
                }
                let z = 1.0 / zi;
                let uv = stage.planes.texture.map(|plane| plane.at(px, py) * z);
                let color = stage.planes.color.map(|plane| plane.at(px, py) * z);
                let source = sampler(image, uv, color);
                if !alpha_pass(prepared.stage.alpha_test, source[3]) {
                    continue;
                }
                buffers.pixels[index] =
                    super::composite(buffers.pixels[index], source, prepared.stage.blend);
                buffers.palettes[index] = u32::MAX;
                if prepared.stage.depth_write {
                    buffers.inverse_depth[index] = zi;
                    buffers.depth_ranks[index] = primitive.draw_rank;
                }
                stats.pixels = stats.pixels.saturating_add(1);
            }
        }
    }
}

/// Original D_DrawSpans8/16 fixed texture stepping: perspective correction at
/// each bounded chunk, arithmetic shifts for complete chunks and division by
/// count-1 at the final endpoint. 1/Z remains affine at each covered pixel.
fn native_span(
    width: u32,
    span: Span,
    planes: Planes,
    fixed_adjust: [i64; 2],
    draw_rank: u32,
    texture_width: u32,
    texture_height: u32,
    transparent_index: Option<u8>,
    texels: &[u8],
    palette: &crate::surface_cache::PaletteLighting,
    palette_id: u32,
    chunk: u32,
    buffers: &mut Buffers<'_>,
    stats: &mut WorldStats,
) {
    let extents = [
        ((texture_width as i64) << 16) - 1,
        ((texture_height as i64) << 16) - 1,
    ];
    let mut x = span.x;
    let end = span.x + span.count;
    let mut zi = planes.inverse_depth.at(x as f32, span.y as f32);
    let mut divided = planes
        .texture
        .map(|plane| plane.at(x as f32, span.y as f32));
    let endpoint = |divided: [f32; 2], zi: f32, minimum: i64| {
        let z = 65536.0 / zi;
        std::array::from_fn::<_, 2, _>(|axis| {
            ((divided[axis] * z) as i64 + fixed_adjust[axis]).clamp(minimum, extents[axis])
        })
    };
    let mut current = endpoint(divided, zi, 0);
    while x < end {
        let count = chunk.min(end - x);
        let complete = x + count < end;
        let distance = if complete { chunk } else { count - 1 } as f32;
        divided[0] += planes.texture[0].x * distance;
        divided[1] += planes.texture[1].x * distance;
        zi += planes.inverse_depth.x * distance;
        let next = endpoint(divided, zi, chunk as i64);
        let step = if complete {
            std::array::from_fn(|i| (next[i] - current[i]) >> chunk.trailing_zeros())
        } else if count > 1 {
            std::array::from_fn(|i| (next[i] - current[i]) / (count - 1) as i64)
        } else {
            [0; 2]
        };
        for offset in 0..count {
            let px = x + offset;
            let zi = planes.inverse_depth.at(px as f32, span.y as f32);
            let index = span.y as usize * width as usize + px as usize;
            let sx = (current[0] >> 16).clamp(0, texture_width as i64 - 1) as usize;
            let sy = (current[1] >> 16).clamp(0, texture_height as i64 - 1) as usize;
            let color = texels[sy * texture_width as usize + sx];
            if zi.is_finite()
                && zi > 0.0
                && super::depth_passes(
                    DepthFunc::Lequal,
                    zi,
                    buffers.inverse_depth[index],
                    draw_rank,
                    buffers.depth_ranks[index],
                )
                && Some(color) != transparent_index
            {
                buffers.pixels[index] = palette.color(color);
                buffers.indices[index] = color;
                buffers.palettes[index] = palette_id;
                buffers.inverse_depth[index] = zi;
                buffers.depth_ranks[index] = draw_rank;
                stats.pixels = stats.pixels.saturating_add(1);
            }
            current[0] += step[0];
            current[1] += step[1];
        }
        x += count;
        current = next;
    }
}

fn clip_world_plane(
    camera: Camera,
    input: &[ClipVertex],
    output: &mut [ClipVertex],
    plane: usize,
) -> Option<usize> {
    let mut count = 0;
    let mut previous = input[input.len() - 1];
    let mut previous_distance = camera.distance(previous, plane);
    for &current in input {
        let distance = camera.distance(current, plane);
        if !distance.is_finite() || !previous_distance.is_finite() {
            return None;
        }
        let inside = distance >= 0.0;
        let previous_inside = previous_distance >= 0.0;
        if inside != previous_inside {
            let denominator = previous_distance - distance;
            if !denominator.is_finite() || count == output.len() {
                return None;
            }
            let vertex = previous.lerp(current, previous_distance / denominator);
            if !vertex.finite() {
                return None;
            }
            output[count] = vertex;
            count += 1;
        }
        if inside {
            if count == output.len() {
                return None;
            }
            output[count] = current;
            count += 1;
        }
        previous = current;
        previous_distance = distance;
    }
    Some(count)
}

fn mip_adjust(projection: [[f32; 4]; 2]) -> f32 {
    let lengths = projection.map(|p| (p[0] * p[0] + p[1] * p[1] + p[2] * p[2]).sqrt());
    let average = (lengths[0] + lengths[1]) * 0.5;
    if average < 0.32 {
        4.0
    } else if average < 0.49 {
        3.0
    } else if average < 0.99 {
        2.0
    } else {
        1.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn collinearity_proof_preserves_even_subnormal_world_area() {
        use qa_core::primitives::Vec3;
        let loaded = |points: [[f32; 3]; 3]| {
            points.map(|point| crate::world::geometry::WorldVertex {
                vertex: Vertex {
                    position: Vec3(point),
                    ..Vertex::default()
                },
                normal: Vec3::default(),
            })
        };
        let line = loaded([[0.0; 3], [1.0, 2.0, 3.0], [2.0, 4.0, 6.0]]);
        assert!(boundary_is_collinear(&line, &[0, 1, 2]));
        let thin = loaded([[0.0; 3], [1.0, 1.0, 0.0], [2.0, 2.0, f32::from_bits(1)]]);
        assert!(!boundary_is_collinear(&thin, &[0, 1, 2]));
        let wide = loaded([
            [1.0, 1.0, 0.0],
            [f32::MAX, f32::MAX, 0.0],
            [f32::MAX, f32::MAX, f32::from_bits(1)],
        ]);
        assert!(!boundary_is_collinear(&wide, &[0, 1, 2]));
        let repeated = loaded([[4.0, 8.0, 12.0]; 3]);
        assert!(boundary_is_collinear(&repeated, &[0, 1, 2]));
    }
    #[test]
    fn widest_basis_preserves_depth_and_uv_after_nearly_collinear_vertices() {
        let points = [
            [0.0, 0.0],
            [1.0, 0.0],
            [2.0, 2.0_f32.powi(-24)],
            [4.0, 4.0],
            [0.0, 4.0],
        ];
        let vertices = points.map(|point| {
            let zi = 0.125 * point[0] + 0.25 * point[1] + 0.25;
            ScreenVertex {
                xy: point.map(|value| value + 0.5),
                inverse_depth: zi,
                texcoord_over_depth: [
                    2.0 * point[0] + 4.0 * point[1] + 1.0,
                    -0.25 * point[0] + 0.5 * point[1] + 0.25,
                ],
                color_over_depth: [zi, zi * 0.5, zi * 0.25, zi],
                ..ScreenVertex::default()
            }
        });
        assert_eq!(vertices[2].inverse_depth, 0.5);
        let result = Planes::load(&vertices);
        assert!(result.is_some());
        let Some(planes) = result else {
            return;
        };
        assert_eq!(planes.inverse_depth.at(1.0, 1.0), 0.625);
        assert_eq!(planes.texture[0].at(1.0, 1.0), 7.0);
        assert_eq!(planes.texture[1].at(1.0, 1.0), 0.5);
        assert_eq!(planes.color[0].at(1.0, 1.0), 0.625);
        assert_eq!(planes.color[1].at(1.0, 1.0), 0.3125);
    }
}
