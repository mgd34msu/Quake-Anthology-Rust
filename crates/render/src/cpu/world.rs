//! The single world edge/span consumer. Map topology never falls back to mesh
//! triangles; polygon and tessellated-patch boundaries enter the same scanner.
use super::{Camera, ClipVertex, ScreenVertex};
use crate::assets::{
    Assets, DepthFunc, ImageId, Material, MaterialId, MaterialSettings, Sky, Stage, StageTexture,
    Vertex,
};
use crate::edges::{DepthPolicy, Edges, ProjectedVertex, Span};
use crate::scene::{CommandList, CpuPresentation, DrawItem, DrawKind, SceneRanges, SurfaceRef};
use crate::shader::{AlphaFunc, AlphaGen, BlendFactor, Cull, RgbGen, TexCoordGen};
use crate::stage::{DeformOp, DrawInputs, PreparedStage, StageEvaluator, alpha_pass};
use crate::surface_cache::{BuildState, CacheStats, LightGrid, SurfaceCache, SurfaceSource};
use crate::world::WorldId;
use crate::world::geometry::{GeometryPartition, TextureCoordinates};
use std::sync::Arc;

mod band;
mod bins;
mod clip;
mod jobs;
mod packed;
use bins::{CoverageBins, RasterSelection};
pub use jobs::{BandJob, render_band};

pub const MAX_BANDS: usize = 8;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RasterBands {
    #[default]
    One,
    Two,
    Four,
    Eight,
}
impl RasterBands {
    pub const fn count(self) -> usize {
        match self {
            Self::One => 1,
            Self::Two => 2,
            Self::Four => 4,
            Self::Eight => 8,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RasterConfig {
    pub bands: RasterBands,
    pub total_cache_budget_bytes: usize,
    pub allocated_cache_bytes: usize,
    pub per_band_cache_bytes: usize,
    pub mandatory_cache_bytes: usize,
    /// Load-owned ordered u32 index payload; separate from the rover budget.
    pub bin_index_capacity_bytes: usize,
    /// Shared mip-layout slot payload, counted once rather than per band.
    pub mip_layout_metadata_bytes: usize,
}

#[derive(Clone, Copy, Debug)]
pub struct CpuLimits {
    pub cache_bytes: usize,
    pub max_spans: usize,
    pub scene: crate::scene::Limits,
    pub bands: RasterBands,
}
impl Default for CpuLimits {
    fn default() -> Self {
        Self {
            cache_bytes: 32 * 1024 * 1024,
            max_spans: 4096,
            scene: crate::scene::Limits::default(),
            bands: RasterBands::One,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WorldStats {
    /// Accepted clipped boundary descriptors, counted once during preparation.
    pub polygons: u64,
    pub spans: u64,
    pub pixels: u64,
    /// Overlapping patch subset of accepted prepared boundaries.
    pub patch_polygons: u64,
    /// Kernel span calls and successful pixel writes, including sky stages.
    pub sky_spans: u64,
    pub sky_pixels: u64,
    /// Non-sky generic stage calls. A multi-stage span contributes per stage.
    pub stage_spans: u64,
    pub stage_pixels: u64,
    /// Overlapping subsets of non-sky generic stage calls and writes.
    pub curve_spans: u64,
    pub curve_pixels: u64,
    pub multistage_spans: u64,
    pub multistage_pixels: u64,
    /// Native indexed-cache kernel calls and successful pixel writes.
    pub indexed_spans: u64,
    pub indexed_pixels: u64,
    /// Precombined RGB cache calls and successful writes.
    pub rgba_spans: u64,
    pub rgba_pixels: u64,
    pub rgba_hits: u64,
    pub rgba_fills: u64,
    pub rgba_evictions: u64,
    pub rgba_rejected: u64,
    pub rgba_minified_spans: u64,
    /// Static pairs independently sample two native factors. Pixels count the
    /// final combined writes; cache counts are actual shared-rover deltas.
    pub factor_spans: u64,
    pub factor_pixels: u64,
    pub factor_hits: u64,
    pub factor_fills: u64,
    pub factor_evictions: u64,
    pub factor_rejected: u64,
    pub factor_fallback_spans: u64,
    pub factor_minified_spans: u64,
    /// Overlapping subset of factor calls covering tessellated patch boundaries.
    pub factor_curve_spans: u64,
    pub factor_curve_pixels: u64,
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
    fn derivatives(self, x: f32, y: f32) -> [[f32; 2]; 2] {
        let zi = self.inverse_depth.at(x, y);
        let depth = [self.inverse_depth.x, self.inverse_depth.y];
        std::array::from_fn(|axis| {
            std::array::from_fn(|coordinate| {
                let p = self.texture[coordinate];
                let gradient = [p.x, p.y];
                (gradient[axis] * zi - p.at(x, y) * depth[axis]) / (zi * zi)
            })
        })
    }
    fn mip(
        self,
        image: &crate::assets::Image,
        sampler: crate::assets::Sampler,
        x: f32,
        y: f32,
    ) -> u8 {
        super::image_mip(image, sampler, self.derivatives(x, y))
    }
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

#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum PrimitiveDomain {
    #[default]
    WorldSurface,
    GeneratedSky,
}

#[derive(Clone, Copy, Default)]
struct Primitive {
    domain: PrimitiveDomain,
    world: WorldId,
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
    first_coverage: usize,
    coverage_count: usize,
    depth_override: Option<f32>,
    patch: bool,
    rgba: Option<usize>,
}

#[derive(Clone, Copy, Default)]
struct StagePlanes {
    prepared: Option<PreparedStage>,
    planes: Planes,
    sampler: Option<super::ShadeFn>,
}

struct WorldCatalog {
    edge_capacity: usize,
    primitive_capacity: usize,
    mandatory_cache_bytes: usize,
    offsets: Box<[usize]>,
    surfaces: Box<[SurfaceInfo]>,
    boundary_offsets: Box<[usize]>,
    rgba: Box<[Option<super::rgba::Recipe>]>,
    factors: Box<[super::rgba::Factor]>,
    skies: Box<[super::sky::SkyDefinition]>,
    surfaces_cache: Arc<crate::surface_cache::SurfaceCatalog>,
}

#[derive(Clone, Copy, Default)]
enum PreparedDraw {
    #[default]
    External,
    Surface([usize; 2]),
    Sky {
        boxes: [usize; 2],
        clouds: [usize; 2],
    },
    Skip,
}

struct WorldPrepare {
    catalog: Arc<WorldCatalog>,
    rgba_prepared: Box<[Option<super::rgba::Prepared>]>,
    rgba_colors: Box<[super::rgba::ProductColorCache]>,
    primitives: Box<[Primitive]>,
    primitive_count: usize,
    stages: Box<[StagePlanes]>,
    stage_count: usize,
    clip: clip::ClipGraph,
    screen: Box<[ScreenVertex]>,
    projected: Box<[ProjectedVertex]>,
    coverage: Box<[ProjectedVertex]>,
    coverage_count: usize,
    sky_states: Box<[super::sky::SkyState]>,
    sky_stages: Box<[Option<PreparedStage>]>,
    draws: Box<[PreparedDraw]>,
    surface_draws: Box<[PreparedDraw]>,
    reference_base: u32,
    draw_count: usize,
    opaque_count: usize,
    policy: DepthPolicy,
    background: Option<super::sky::BackgroundDraw>,
    stats: WorldStats,
    bins: CoverageBins,
}

struct WorldBand {
    width: u32,
    catalog: Arc<WorldCatalog>,
    edges: Edges,
    cache: SurfaceCache,
    stats: WorldStats,
    selection: RasterSelection,
}

pub(super) struct WorldRaster {
    prepare: WorldPrepare,
    bands: Box<[WorldBand]>,
    config: RasterConfig,
}

pub(super) struct Buffers<'a> {
    pub first_row: u32,
    pub frame_height: u32,
    pub pixels: &'a mut [u32],
    pub inverse_depth: &'a mut [f32],
    pub depth_ranks: &'a mut [u32],
    pub indices: &'a mut [u8],
    pub palettes: &'a mut [u32],
}

impl<'a> Buffers<'a> {
    fn reborrow(&mut self) -> Buffers<'_> {
        Buffers {
            first_row: self.first_row,
            frame_height: self.frame_height,
            pixels: &mut *self.pixels,
            inverse_depth: &mut *self.inverse_depth,
            depth_ranks: &mut *self.depth_ranks,
            indices: &mut *self.indices,
            palettes: &mut *self.palettes,
        }
    }

    fn split_rows(self, width: u32, rows: u32) -> (Self, Self) {
        let count = width as usize * rows as usize;
        let (pixels, remaining_pixels) = self.pixels.split_at_mut(count);
        let (depth, remaining_depth) = self.inverse_depth.split_at_mut(count);
        let (ranks, remaining_ranks) = self.depth_ranks.split_at_mut(count);
        let (indices, remaining_indices) = self.indices.split_at_mut(count);
        let (palettes, remaining_palettes) = self.palettes.split_at_mut(count);
        (
            Self {
                first_row: self.first_row,
                frame_height: self.frame_height,
                pixels,
                inverse_depth: depth,
                depth_ranks: ranks,
                indices,
                palettes,
            },
            Self {
                first_row: self.first_row + rows,
                frame_height: self.frame_height,
                pixels: remaining_pixels,
                inverse_depth: remaining_depth,
                depth_ranks: remaining_ranks,
                indices: remaining_indices,
                palettes: remaining_palettes,
            },
        )
    }

    pub(super) fn rows(
        &self,
        width: u32,
        viewport: crate::scene::Viewport,
    ) -> Option<std::ops::Range<u32>> {
        let start = self.first_row.max(viewport.y);
        let end =
            (self.first_row + self.pixels.len() as u32 / width).min(viewport.y + viewport.height);
        (start < end).then_some(start..end)
    }
    pub(super) fn offset(&self, width: u32, x: u32, y: u32) -> usize {
        ((y - self.first_row) * width + x) as usize
    }
}

impl WorldRaster {
    pub(super) fn load(
        width: u32,
        height: u32,
        assets: &Assets,
        limits: CpuLimits,
        evaluator: &StageEvaluator,
    ) -> Result<Self, &'static str> {
        let band_count = limits.bands.count();
        if band_count > height as usize {
            return Err("CPU band count exceeds framebuffer rows");
        }
        let share = (limits.cache_bytes / 8 / band_count) * 8;
        let mut offsets = Vec::with_capacity(assets.worlds().len());
        let mut sources = Vec::new();
        let mut surfaces = Vec::new();
        let mut boundary_offsets = Vec::with_capacity(assets.worlds().len());
        let mut rgba = Vec::new();
        let mut factors = Vec::new();
        let mut boundaries = 0usize;
        let mut max_vertices = 3usize;
        let mut max_edges = 0usize;
        let mut stage_capacity = 0usize;
        let skies: Vec<_> = assets
            .materials()
            .iter()
            .map(super::sky::SkyDefinition::load)
            .collect::<Result<_, _>>()?;
        let max_sky_stages = assets
            .materials()
            .iter()
            .zip(&skies)
            .filter(|(_, batch)| batch.enabled)
            .map(|(material, _)| material.stages.len())
            .max()
            .unwrap_or(0);
        let sky_count = skies.iter().filter(|sky| sky.enabled).count();
        let has_sky = sky_count != 0;
        let draw_capacity = limits
            .scene
            .draw_capacity()
            .ok_or("CPU scene draw count overflow")?;
        if draw_capacity == 0
            || draw_capacity > u32::MAX as usize
            || limits.scene.surfaces == 0
            || limits.scene.surfaces > u32::MAX as usize
        {
            return Err("invalid CPU scene draw capacity");
        }
        for world in assets.worlds() {
            offsets.push(surfaces.len());
            let geometry = world.geometry();
            let boundary_offset = rgba.len();
            boundary_offsets.push(boundary_offset);
            rgba.resize(boundary_offset + geometry.boundaries.len(), None);
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
                for (loop_index, boundary) in loops.iter().enumerate() {
                    let boundary_index = surface.boundaries.first as usize + loop_index;
                    if let Some(recipe) = super::rgba::Recipe::load(
                        geometry,
                        boundary_index,
                        *binding,
                        material,
                        assets,
                        evaluator,
                        &mut sources,
                        &mut factors,
                        limits.cache_bytes,
                    )? {
                        rgba[boundary_offset + boundary_index] = Some(recipe);
                    }
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
        let sky_boundaries = sky_count
            .checked_mul(super::sky::CLOUD_BOUNDARIES + super::sky::BOX_BOUNDARIES)
            .ok_or("sky polygon count overflow")?;
        boundaries = boundaries
            .checked_add(sky_boundaries)
            .ok_or("sky polygon count overflow")?;
        for (material, sky) in assets.materials().iter().zip(&skies) {
            if sky.enabled {
                stage_capacity = stage_capacity
                    .checked_add(
                        super::sky::CLOUD_BOUNDARIES
                            .checked_mul(material.stages.len())
                            .and_then(|count| count.checked_add(super::sky::BOX_BOUNDARIES))
                            .ok_or("sky stage count overflow")?,
                    )
                    .ok_or("sky stage count overflow")?;
            }
        }
        let coverage_capacity = max_edges
            .checked_add(
                sky_count
                    .checked_mul(super::sky::SKY_EDGES)
                    .ok_or("sky coverage count overflow")?,
            )
            .ok_or("world coverage count overflow")?;
        if has_sky {
            max_vertices = max_vertices.max(10);
            max_edges = max_edges.max(super::sky::SKY_EDGES);
        }
        let surfaces_cache = crate::surface_cache::SurfaceCatalog::load(sources)?;
        let mut mandatory_cache_bytes = 0usize;
        for id in surfaces
            .iter()
            .filter(|surface| surface.cache_supported && surface.sky.is_none())
            .map(|surface| surface.cache)
            .chain(rgba.iter().flatten().filter_map(|recipe| match recipe {
                super::rgba::Recipe::Product(product) => Some(product.cache),
                super::rgba::Recipe::Pair(_) => None,
            }))
        {
            let source = surfaces_cache
                .surface(id)
                .ok_or("missing mandatory cache surface")?;
            for mip in 0..source.mip_count() {
                mandatory_cache_bytes = mandatory_cache_bytes.max(
                    source
                        .reservation_bytes(mip)
                        .ok_or("mandatory cache reservation overflow")?,
                );
            }
        }
        if mandatory_cache_bytes > share {
            return Err("CPU cache cannot hold a mandatory surface");
        }
        let rgba_count = rgba.len();
        let sky_state_count = skies.len();
        let catalog = Arc::new(WorldCatalog {
            edge_capacity: max_edges.max(3),
            primitive_capacity: boundaries.max(1),
            mandatory_cache_bytes,
            offsets: offsets.into_boxed_slice(),
            surfaces: surfaces.into_boxed_slice(),
            boundary_offsets: boundary_offsets.into_boxed_slice(),
            rgba: rgba.into_boxed_slice(),
            factors: factors.into_boxed_slice(),
            skies: skies.into_boxed_slice(),
            surfaces_cache,
        });
        let bins = CoverageBins::load(height, band_count, catalog.primitive_capacity)?;
        let bin_index_capacity_bytes = bins.capacity_bytes();
        let bands = (0..band_count)
            .map(|id| {
                WorldBand::load(
                    width,
                    height,
                    Arc::clone(&catalog),
                    share,
                    limits.max_spans,
                    RasterSelection::Band(id),
                )
            })
            .collect::<Result<Vec<_>, _>>()?
            .into_boxed_slice();
        Ok(Self {
            prepare: WorldPrepare {
                catalog: Arc::clone(&catalog),
                rgba_prepared: vec![None; rgba_count].into_boxed_slice(),
                rgba_colors: vec![super::rgba::ProductColorCache::default(); rgba_count]
                    .into_boxed_slice(),
                primitives: vec![Primitive::default(); boundaries.max(1)].into_boxed_slice(),
                primitive_count: 0,
                stages: vec![StagePlanes::default(); stage_capacity.max(1)].into_boxed_slice(),
                stage_count: 0,
                clip: clip::ClipGraph::load(max_vertices)?,
                screen: vec![ScreenVertex::default(); max_vertices].into_boxed_slice(),
                projected: vec![ProjectedVertex::default(); max_vertices].into_boxed_slice(),
                coverage: vec![ProjectedVertex::default(); coverage_capacity.max(3)]
                    .into_boxed_slice(),
                coverage_count: 0,
                sky_states: (0..sky_state_count)
                    .map(|_| super::sky::SkyState::default())
                    .collect::<Vec<_>>()
                    .into_boxed_slice(),
                sky_stages: vec![None; max_sky_stages.max(1)].into_boxed_slice(),
                draws: vec![PreparedDraw::default(); draw_capacity].into_boxed_slice(),
                surface_draws: vec![PreparedDraw::Skip; limits.scene.surfaces].into_boxed_slice(),
                reference_base: 0,
                draw_count: 0,
                opaque_count: 0,
                policy: DepthPolicy::PlaneDepth,
                background: None,
                stats: WorldStats::default(),
                bins,
            },
            bands,
            config: RasterConfig {
                bands: limits.bands,
                total_cache_budget_bytes: limits.cache_bytes,
                allocated_cache_bytes: share * band_count,
                per_band_cache_bytes: share,
                mandatory_cache_bytes,
                bin_index_capacity_bytes,
                mip_layout_metadata_bytes: catalog.surfaces_cache.mip_metadata_bytes(),
            },
        })
    }

    pub(super) fn stats(&self) -> WorldStats {
        let mut total = self.prepare.stats;
        let mut cache = CacheStats::default();
        for band in &self.bands {
            total = merge_stats(total, band.stats);
            let current = band.cache.stats();
            cache.hits = cache.hits.saturating_add(current.hits);
            cache.fills = cache.fills.saturating_add(current.fills);
            cache.evictions = cache.evictions.saturating_add(current.evictions);
            cache.rejected = cache.rejected.saturating_add(current.rejected);
        }
        WorldStats { cache, ..total }
    }
    pub(super) fn config(&self) -> RasterConfig {
        self.config
    }
    pub(super) fn band_stats(&self, output: &mut [WorldStats; MAX_BANDS]) -> usize {
        output.fill(WorldStats::default());
        for (out, band) in output.iter_mut().zip(&self.bands) {
            *out = WorldStats {
                cache: band.cache.stats(),
                ..band.stats
            };
        }
        self.bands.len()
    }
    pub(super) fn reset_stats(&mut self) {
        self.prepare.stats = WorldStats::default();
        for band in &mut self.bands {
            band.stats = WorldStats::default();
        }
    }
    pub(super) fn prepare_view(
        &mut self,
        camera: &Camera,
        list: &CommandList,
        scene: SceneRanges,
        assets: &Assets,
        evaluator: &StageEvaluator,
        stats: &mut crate::BackendStats,
    ) -> bool {
        self.prepare
            .prepare_view(camera, list, scene, assets, evaluator, stats)
    }

    fn draw(&self, rank: usize) -> PreparedDraw {
        self.prepare
            .draws
            .get(rank)
            .filter(|_| rank < self.prepare.draw_count)
            .copied()
            .unwrap_or(PreparedDraw::External)
    }
}

impl WorldPrepare {
    fn prepare_view(
        &mut self,
        camera: &Camera,
        list: &CommandList,
        scene: SceneRanges,
        assets: &Assets,
        evaluator: &StageEvaluator,
        stats: &mut crate::BackendStats,
    ) -> bool {
        self.primitive_count = 0;
        self.stage_count = 0;
        self.coverage_count = 0;
        self.opaque_count = 0;
        self.draw_count = 0;
        self.bins.clear();
        self.background = None;
        if list.draws(scene.draws).len() > self.draws.len() {
            self.reject(stats);
            return false;
        }
        self.collect_skies(camera, list, scene, assets, stats);
        let references = list.surfaces(scene.surfaces);
        let reference_base = scene.surfaces.first;
        self.reference_base = reference_base;
        if references.len() > self.surface_draws.len() {
            self.reject(stats);
            return false;
        }
        self.surface_draws[..references.len()].fill(PreparedDraw::Skip);
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
                .catalog
                .offsets
                .get(reference.world.0 as usize)
                .and_then(|offset| {
                    self.catalog
                        .surfaces
                        .get(offset + reference.surface as usize)
                })
                .copied()
            else {
                self.reject(stats);
                continue;
            };
            if info.raster_empty {
                continue;
            }
            if material.settings.sky.is_some() {
                if self
                    .catalog
                    .skies
                    .get(binding.material.0 as usize)
                    .is_some_and(|batch| batch.enabled)
                    && matches!(camera.refdef.cpu_presentation, CpuPresentation::Rgb)
                {
                    stats.surfaces = stats.surfaces.saturating_add(1);
                    continue;
                }
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
                    camera, *reference, source, material, assets, evaluator, stats,
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
                    patch: surface.patch.is_some(),
                    rgba: if native.is_none() {
                        self.catalog
                            .boundary_offsets
                            .get(reference.world.0 as usize)
                            .map(|offset| offset + boundary)
                            .filter(|&index| {
                                self.catalog.rgba[index]
                                    .as_ref()
                                    .is_some_and(|recipe| recipe.current(assets))
                            })
                    } else {
                        None
                    },
                    ..Primitive::default()
                };
                if let Some(index) = primitive.rgba {
                    let prepared = self.catalog.rgba[index].as_ref().and_then(|recipe| {
                        recipe.prepare_cached(
                            &camera.refdef,
                            evaluator,
                            &mut self.rgba_colors[index],
                        )
                    });
                    if let Some(prepared) = prepared {
                        self.rgba_prepared[index] = Some(prepared);
                    } else {
                        primitive.rgba = None;
                    }
                }
                let product = primitive.rgba.is_some_and(|index| {
                    self.catalog.rgba[index]
                        .as_ref()
                        .is_some_and(|recipe| recipe.product())
                });
                let Some(count) = self.prepare_clip(
                    camera,
                    assets,
                    primitive,
                    if native.is_some() || product {
                        None
                    } else {
                        Some(base)
                    },
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
                    let Some((planes, adjust)) = native_planes(camera, surface, primitive.mip)
                    else {
                        self.reject(stats);
                        continue;
                    };
                    primitive.planes = planes;
                    primitive.fixed_adjust = adjust;
                } else if product {
                    let Some(planes) = Planes::load(&self.screen[..count]) else {
                        self.reject(stats);
                        continue;
                    };
                    primitive.planes = planes;
                    stats.stages = stats.stages.saturating_add(2);
                } else {
                    if self.stage_count + material.stages.len() > self.stages.len() {
                        self.reject(stats);
                        continue;
                    }
                    let mut valid = true;
                    for (stage_index, stage) in material.stages.iter().enumerate() {
                        let Ok(prepared) = evaluator.prepare(stage, material.settings, inputs)
                        else {
                            valid = false;
                            break;
                        };
                        let Some(image) = assets.image(prepared.image) else {
                            valid = false;
                            break;
                        };
                        let stage_count = if stage_index == 0 {
                            Some(count)
                        } else {
                            self.clip
                                .project_stage(prepared, evaluator, &mut self.screen)
                        };
                        let Some(stage_count) = stage_count else {
                            valid = false;
                            break;
                        };
                        let Some(planes) = Planes::load(&self.screen[..stage_count]) else {
                            valid = false;
                            break;
                        };
                        self.stages[self.stage_count] = StagePlanes {
                            prepared: Some(prepared),
                            planes,
                            sampler: Some(super::stage_sampler(image, prepared.stage.sampler)),
                        };
                        self.stage_count += 1;
                        primitive.stages += 1;
                    }
                    if !valid {
                        self.stage_count = primitive.first_stage;
                        self.reject(stats);
                        continue;
                    }
                    primitive.planes = self.stages[primitive.first_stage].planes;
                }
                if !self.keep_coverage(&mut primitive, count) {
                    self.stage_count = primitive.first_stage;
                    self.reject(stats);
                    continue;
                }
                self.primitives[self.primitive_count] = primitive;
                self.primitive_count += 1;
            }
            if self.primitive_count > before {
                stats.surfaces = stats.surfaces.saturating_add(1);
                if self.primitives[before..self.primitive_count]
                    .iter()
                    .any(|p| p.overlay)
                {
                    self.surface_draws[reference_index] =
                        PreparedDraw::Surface([before, self.primitive_count]);
                }
            }
        }
        self.policy = if certified {
            DepthPolicy::BspKeys
        } else {
            DepthPolicy::PlaneDepth
        };
        self.background = background
            .and_then(|source| super::sky::BackgroundDraw::prepare(source, camera, assets));
        self.opaque_count = self.primitive_count;
        for (rank, &item) in list.draws(scene.draws).iter().enumerate() {
            self.draws[rank] =
                self.prepare_draw(camera, item, rank as u32, list, assets, evaluator, stats);
        }
        self.draw_count = list.draws(scene.draws).len();
        self.bins.rebuild(
            camera.refdef.viewport,
            self.policy,
            self.opaque_count,
            &self.primitives[..self.primitive_count],
            &self.coverage[..self.coverage_count],
        );
        self.stats.polygons = self
            .stats
            .polygons
            .saturating_add(self.primitive_count as u64);
        self.stats.patch_polygons = self.stats.patch_polygons.saturating_add(
            self.primitives[..self.primitive_count]
                .iter()
                .filter(|p| p.patch)
                .count() as u64,
        );
        stats.stages = stats
            .stages
            .saturating_add(self.stage_count.min(u32::MAX as usize) as u32);
        true
    }

    fn reject(&mut self, stats: &mut crate::BackendStats) {
        self.stats.rejected = self.stats.rejected.saturating_add(1);
        stats.rejected = stats.rejected.saturating_add(1);
    }

    fn collect_skies(
        &mut self,
        camera: &Camera,
        list: &CommandList,
        scene: SceneRanges,
        assets: &Assets,
        stats: &mut crate::BackendStats,
    ) {
        for batch in &mut self.sky_states {
            batch.clear();
        }
        if !matches!(camera.refdef.cpu_presentation, CpuPresentation::Rgb) {
            return;
        }
        for &item in list.draws(scene.draws) {
            let Some(material) = sky_item_material(item, list, assets) else {
                continue;
            };
            if !self
                .catalog
                .skies
                .get(material.0 as usize)
                .is_some_and(|batch| batch.enabled)
            {
                continue;
            }
            match item.kind {
                DrawKind::Surface => {
                    let reference = list.surface(item.index);
                    let Some(world) = assets.world(reference.world) else {
                        continue;
                    };
                    let Some(surface) = world.geometry().surfaces.get(reference.surface as usize)
                    else {
                        continue;
                    };
                    if self
                        .catalog
                        .offsets
                        .get(reference.world.0 as usize)
                        .and_then(|offset| {
                            self.catalog
                                .surfaces
                                .get(offset + reference.surface as usize)
                        })
                        .is_some_and(|info| info.raster_empty)
                    {
                        continue;
                    }
                    for triangle in
                        world.geometry().indices[surface.indices.indices()].chunks_exact(3)
                    {
                        let points = [triangle[0], triangle[1], triangle[2]]
                            .map(|index| world.geometry().vertices[index as usize].vertex.position);
                        self.add_sky_polygon(material, points, camera, stats);
                    }
                }
                DrawKind::Entity => {
                    let entity = list.entity(item.index);
                    let Some(model) = assets.model(entity.model) else {
                        continue;
                    };
                    for triangle in model.indices.chunks_exact(3) {
                        let points = [triangle[0], triangle[1], triangle[2]].map(|index| {
                            let position = model.vertices[index as usize].position;
                            qa_core::primitives::Vec3(std::array::from_fn(|axis| {
                                entity.origin.0[axis]
                                    + entity.axes[0].0[axis] * position.0[0]
                                    + entity.axes[1].0[axis] * position.0[1]
                                    + entity.axes[2].0[axis] * position.0[2]
                            }))
                        });
                        self.add_sky_polygon(material, points, camera, stats);
                    }
                }
                DrawKind::Poly => {
                    let vertices = list.vertices(list.poly(item.index).vertices);
                    for triangle in vertices[1..].windows(2) {
                        self.add_sky_polygon(
                            material,
                            [
                                vertices[0].position,
                                triangle[0].position,
                                triangle[1].position,
                            ],
                            camera,
                            stats,
                        );
                    }
                }
            }
        }
    }

    fn add_sky_polygon(
        &mut self,
        material: MaterialId,
        points: [qa_core::primitives::Vec3; 3],
        camera: &Camera,
        stats: &mut crate::BackendStats,
    ) {
        if !self.sky_states[material.0 as usize]
            .clip
            .add_polygon(&points, camera.refdef.origin)
        {
            self.reject(stats);
        }
    }

    /// The shared draw list determines when the material's single sky draw
    /// occurs. Source polygons from all worlds/entities/polys have already
    /// contributed to its clip, matching native RB_StageIteratorSky.
    fn prepare_draw(
        &mut self,
        camera: &Camera,
        item: DrawItem,
        rank: u32,
        list: &CommandList,
        assets: &Assets,
        evaluator: &StageEvaluator,
        stats: &mut crate::BackendStats,
    ) -> PreparedDraw {
        let external = match item.kind {
            DrawKind::Surface => item
                .index
                .checked_sub(self.reference_base)
                .and_then(|index| self.surface_draws.get(index as usize))
                .copied()
                .unwrap_or(PreparedDraw::Skip),
            _ => PreparedDraw::External,
        };
        let Some(id) = sky_item_material(item, list, assets) else {
            return external;
        };
        if !self
            .catalog
            .skies
            .get(id.0 as usize)
            .is_some_and(|sky| sky.enabled)
            || !matches!(camera.refdef.cpu_presentation, CpuPresentation::Rgb)
        {
            return external;
        }
        let Some(batch) = self.sky_states.get_mut(id.0 as usize) else {
            return external;
        };
        if batch.prepared {
            return PreparedDraw::Skip;
        }
        batch.prepared = true;
        let mut bounds = *batch.clip.bounds();
        if !bounds.iter().any(|bound| bound.visible()) {
            return PreparedDraw::Skip;
        }
        let Some(material) = assets.material(id) else {
            self.reject(stats);
            return PreparedDraw::Skip;
        };
        let Some(Sky::Cube {
            outer_box,
            rotation,
            params,
            ..
        }) = material.settings.sky
        else {
            return external;
        };
        if rotation.is_some_and(|rotation| rotation.degrees_per_second != 0.0) {
            bounds.fill(crate::sky::FaceBounds {
                mins: [-1.0; 2],
                maxs: [1.0; 2],
            });
        }
        let inputs = DrawInputs {
            time_ms: camera.refdef.time_ms,
            view_origin: camera.refdef.origin,
            identity_light: camera.refdef.identity_light,
            ..DrawInputs::default()
        };
        let old_primitive_count = self.primitive_count;
        let depth_override = params.far_depth.then_some(1.0 / camera.refdef.far);
        if let Some(images) = outer_box {
            let settings = MaterialSettings {
                cull: Cull::None,
                ..MaterialSettings::default()
            };
            for face in crate::sky::CubeFace::ALL {
                let bound = bounds[face.index()];
                if !bound.visible() {
                    continue;
                }
                let (mins, maxs) = if params.snap_bounds {
                    let Some([mins, maxs]) = bound.grid_bounds() else {
                        continue;
                    };
                    (
                        mins.map(|v| (v as f32 - 4.0) / 4.0),
                        maxs.map(|v| (v as f32 - 4.0) / 4.0),
                    )
                } else {
                    (bound.mins, bound.maxs)
                };
                let vertices = [
                    [mins[0], mins[1]],
                    [mins[0], maxs[1]],
                    [maxs[0], maxs[1]],
                    [maxs[0], mins[1]],
                ]
                .map(|st| {
                    let cube = crate::sky::cube_vertex(
                        face,
                        st,
                        params.distance.value(camera.refdef.far),
                        params.texcoord_range,
                    );
                    let direction = if params.cpu_rotation {
                        rotation.map_or(cube.direction, |rotation| {
                            crate::sky::unrotate(
                                cube.direction,
                                crate::sky::Rotation {
                                    degrees_per_second: -rotation.degrees_per_second,
                                    ..rotation
                                },
                                inputs.time_ms as f32 * 0.001,
                            )
                        })
                    } else {
                        cube.direction
                    };
                    Vertex {
                        position: inputs.view_origin + direction,
                        texcoord: cube.uv,
                        ..Vertex::default()
                    }
                });
                let stage = Stage {
                    texture: StageTexture::Image(images[face.index()]),
                    sampler: params.sampler,
                    rgb_gen: RgbGen::IdentityLighting,
                    depth_write: !params.far_depth,
                    ..Stage::default()
                };
                let Ok(prepared) = evaluator.prepare(&stage, settings, inputs) else {
                    self.reject(stats);
                    continue;
                };
                self.sky_stages[0] = Some(prepared);
                self.add_generated_sky(
                    camera,
                    &vertices,
                    1,
                    [DeformOp::None; 3],
                    Cull::None,
                    rank,
                    depth_override,
                    assets,
                    evaluator,
                    stats,
                );
            }
        }
        let boxes = [old_primitive_count, self.primitive_count];
        let first_cloud = self.primitive_count;
        if !material.stages.is_empty() {
            let Ok(deforms) = evaluator.prepare_deforms(&material.settings, &inputs) else {
                self.reject(stats);
                return PreparedDraw::Sky {
                    boxes,
                    clouds: [first_cloud, first_cloud],
                };
            };
            let mut valid = true;
            for (index, &stage) in material.stages.iter().enumerate() {
                let stage = if matches!(stage.texgen, TexCoordGen::CloudSky { .. }) {
                    Stage {
                        texgen: TexCoordGen::Texture,
                        ..stage
                    }
                } else {
                    stage
                };
                match evaluator.prepare(&stage, material.settings, inputs) {
                    Ok(prepared) if assets.image(prepared.image).is_some() => {
                        self.sky_stages[index] = Some(prepared)
                    }
                    _ => {
                        valid = false;
                        break;
                    }
                }
            }
            if valid {
                for face in crate::sky::CubeFace::ALL {
                    if face == crate::sky::CubeFace::NegativeZ {
                        continue;
                    }
                    let Some([mins, maxs]) = bounds[face.index()].grid_bounds() else {
                        continue;
                    };
                    for t in mins[1]..maxs[1] {
                        for s in mins[0]..maxs[0] {
                            // FillCloudySkySide's native diagonal and vertex order.
                            for cell in [
                                [[s, t], [s, t + 1], [s + 1, t]],
                                [[s, t + 1], [s + 1, t + 1], [s + 1, t]],
                            ] {
                                let Some(grid) = self.catalog.skies[id.0 as usize].cloud.as_ref()
                                else {
                                    self.reject(stats);
                                    continue;
                                };
                                let vertices = cell.map(|[s, t]| {
                                    let st = [(s as f32 - 4.0) / 4.0, (t as f32 - 4.0) / 4.0];
                                    let direction = crate::sky::cube_vertex(
                                        face,
                                        st,
                                        params.distance.value(camera.refdef.far),
                                        [0.0, 1.0],
                                    )
                                    .direction;
                                    Vertex {
                                        position: inputs.view_origin + direction,
                                        texcoord: grid.uv[face.index()][t][s],
                                        ..Vertex::default()
                                    }
                                });
                                self.add_generated_sky(
                                    camera,
                                    &vertices,
                                    material.stages.len(),
                                    deforms,
                                    material.settings.cull,
                                    rank,
                                    depth_override,
                                    assets,
                                    evaluator,
                                    stats,
                                );
                            }
                        }
                    }
                }
            } else {
                self.reject(stats);
            }
        }
        PreparedDraw::Sky {
            boxes,
            clouds: [first_cloud, self.primitive_count],
        }
    }

    fn add_generated_sky(
        &mut self,
        camera: &Camera,
        vertices: &[Vertex],
        stages: usize,
        deforms: [DeformOp; 3],
        cull: Cull,
        rank: u32,
        depth_override: Option<f32>,
        assets: &Assets,
        evaluator: &StageEvaluator,
        stats: &mut crate::BackendStats,
    ) {
        if self.primitive_count == self.primitives.len()
            || self.stage_count + stages > self.stages.len()
        {
            self.reject(stats);
            return;
        }
        let mut primitive = Primitive {
            domain: PrimitiveDomain::GeneratedSky,
            first_stage: self.stage_count,
            draw_rank: rank,
            deforms,
            depth_override,
            ..Primitive::default()
        };
        let Some(base) = self.sky_stages.first().copied().flatten() else {
            self.reject(stats);
            return;
        };
        let Some(source) = self.clip.sources(vertices.len()) else {
            self.reject(stats);
            return;
        };
        source.copy_from_slice(vertices);
        let Some(count) = self
            .clip
            .build(camera, vertices.len(), deforms, Some(base), evaluator)
            .and_then(|_| {
                self.clip
                    .project_base(camera, &mut self.screen, &mut self.projected)
            })
        else {
            self.reject(stats);
            return;
        };
        if count < 3 {
            return;
        }
        let area = polygon_area(&self.screen[..count]);
        if area == 0.0
            || match cull {
                Cull::Front => area < 0.0,
                Cull::Back => area > 0.0,
                Cull::None => false,
            }
        {
            return;
        }
        for stage in 0..stages {
            let Some(prepared) = self.sky_stages[stage] else {
                self.stage_count = primitive.first_stage;
                self.reject(stats);
                return;
            };
            let Some(image) = assets.image(prepared.image) else {
                self.stage_count = primitive.first_stage;
                self.reject(stats);
                return;
            };
            if stage != 0
                && self
                    .clip
                    .project_stage(prepared, evaluator, &mut self.screen)
                    .is_none()
            {
                self.stage_count = primitive.first_stage;
                self.reject(stats);
                return;
            }
            let Some(planes) = Planes::load(&self.screen[..count]) else {
                self.stage_count = primitive.first_stage;
                self.reject(stats);
                return;
            };
            self.stages[self.stage_count] = StagePlanes {
                prepared: Some(prepared),
                planes,
                sampler: Some(super::stage_sampler(image, prepared.stage.sampler)),
            };
            self.stage_count += 1;
            primitive.stages += 1;
        }
        primitive.planes = self.stages[primitive.first_stage].planes;
        if !self.keep_coverage(&mut primitive, count) {
            self.stage_count = primitive.first_stage;
            self.reject(stats);
            return;
        }
        self.primitives[self.primitive_count] = primitive;
        self.primitive_count += 1;
    }

    fn add_sky(
        &mut self,
        camera: &Camera,
        reference: SurfaceRef,
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
                depth_key: reference.depth_key,
                draw_rank: reference.draw_rank,
                sky: Some(source),
                ..Primitive::default()
            };
            let Some(count) = self.prepare_clip(camera, assets, primitive, None, evaluator) else {
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
            if !self.keep_coverage(&mut primitive, count) {
                self.reject(stats);
                continue;
            }
            self.primitives[self.primitive_count] = primitive;
            self.primitive_count += 1;
            visible = true;
        }
        visible
    }

    fn prepare_clip(
        &mut self,
        camera: &Camera,
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
        let vertices = self.clip.sources(indices.len())?;
        // Recipe ids map to this immutable boundary, whose source corner order
        // is the same order used at recipe registration. Intersections still
        // receive the original ClipGraph interpolation after these sources.
        for (corner, (output, &index)) in vertices.iter_mut().zip(indices).enumerate() {
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
            } else if let Some(index) = primitive.rgba
                && let Some(coordinate) = self.catalog.rgba[index]
                    .as_ref()?
                    .coordinate_corner(corner)
                    .or_else(|| {
                        self.catalog.rgba[index]
                            .as_ref()?
                            .coordinate(vertex.position)
                    })
            {
                vertex.texcoord = coordinate;
            }
            *output = vertex;
        }
        self.clip.build(
            camera,
            indices.len(),
            primitive.deforms,
            prepared,
            evaluator,
        )?;
        self.clip
            .project_base(camera, &mut self.screen, &mut self.projected)
    }

    fn keep_coverage(&mut self, primitive: &mut Primitive, count: usize) -> bool {
        let Some(end) = self.coverage_count.checked_add(count) else {
            return false;
        };
        let Some(target) = self.coverage.get_mut(self.coverage_count..end) else {
            return false;
        };
        target.copy_from_slice(&self.projected[..count]);
        if let Some(depth) = primitive.depth_override {
            for vertex in target {
                vertex.inverse_depth = depth;
            }
        }
        primitive.first_coverage = self.coverage_count;
        primitive.coverage_count = count;
        self.coverage_count = end;
        true
    }
}

fn evaluated_vertex(
    vertex: Vertex,
    deforms: [DeformOp; 3],
    prepared: Option<PreparedStage>,
    evaluator: &StageEvaluator,
) -> Vertex {
    let Some(prepared) = prepared else {
        return vertex;
    };
    let vertex = evaluator.apply_deforms(deforms, vertex);
    let evaluated = evaluator.evaluate(&prepared, &vertex);
    Vertex {
        texcoord: evaluated.texcoord,
        color: evaluated.color,
        position: evaluated.position,
        ..vertex
    }
}

fn sky_item_material(item: DrawItem, list: &CommandList, assets: &Assets) -> Option<MaterialId> {
    match item.kind {
        DrawKind::Surface => {
            let reference = list.surface(item.index);
            Some(
                assets
                    .world(reference.world)?
                    .bindings()
                    .get(reference.surface as usize)?
                    .material,
            )
        }
        DrawKind::Entity => {
            let entity = list.entity(item.index);
            Some(
                entity
                    .material
                    .unwrap_or(assets.model(entity.model)?.material),
            )
        }
        DrawKind::Poly => Some(list.poly(item.index).material),
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
    camera: &Camera,
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

fn merge_stats(a: WorldStats, b: WorldStats) -> WorldStats {
    WorldStats {
        polygons: a.polygons.saturating_add(b.polygons),
        spans: a.spans.saturating_add(b.spans),
        pixels: a.pixels.saturating_add(b.pixels),
        patch_polygons: a.patch_polygons.saturating_add(b.patch_polygons),
        sky_spans: a.sky_spans.saturating_add(b.sky_spans),
        sky_pixels: a.sky_pixels.saturating_add(b.sky_pixels),
        stage_spans: a.stage_spans.saturating_add(b.stage_spans),
        stage_pixels: a.stage_pixels.saturating_add(b.stage_pixels),
        curve_spans: a.curve_spans.saturating_add(b.curve_spans),
        curve_pixels: a.curve_pixels.saturating_add(b.curve_pixels),
        multistage_spans: a.multistage_spans.saturating_add(b.multistage_spans),
        multistage_pixels: a.multistage_pixels.saturating_add(b.multistage_pixels),
        indexed_spans: a.indexed_spans.saturating_add(b.indexed_spans),
        indexed_pixels: a.indexed_pixels.saturating_add(b.indexed_pixels),
        rgba_spans: a.rgba_spans.saturating_add(b.rgba_spans),
        rgba_pixels: a.rgba_pixels.saturating_add(b.rgba_pixels),
        rgba_hits: a.rgba_hits.saturating_add(b.rgba_hits),
        rgba_fills: a.rgba_fills.saturating_add(b.rgba_fills),
        rgba_evictions: a.rgba_evictions.saturating_add(b.rgba_evictions),
        rgba_rejected: a.rgba_rejected.saturating_add(b.rgba_rejected),
        rgba_minified_spans: a.rgba_minified_spans.saturating_add(b.rgba_minified_spans),
        factor_spans: a.factor_spans.saturating_add(b.factor_spans),
        factor_pixels: a.factor_pixels.saturating_add(b.factor_pixels),
        factor_hits: a.factor_hits.saturating_add(b.factor_hits),
        factor_fills: a.factor_fills.saturating_add(b.factor_fills),
        factor_evictions: a.factor_evictions.saturating_add(b.factor_evictions),
        factor_rejected: a.factor_rejected.saturating_add(b.factor_rejected),
        factor_fallback_spans: a
            .factor_fallback_spans
            .saturating_add(b.factor_fallback_spans),
        factor_minified_spans: a
            .factor_minified_spans
            .saturating_add(b.factor_minified_spans),
        factor_curve_spans: a.factor_curve_spans.saturating_add(b.factor_curve_spans),
        factor_curve_pixels: a.factor_curve_pixels.saturating_add(b.factor_curve_pixels),
        rejected: a.rejected.saturating_add(b.rejected),
        cache: CacheStats::default(),
    }
}

fn add_edge_stats(stats: &mut WorldStats, edges: crate::edges::Stats) {
    stats.spans = stats.spans.saturating_add(edges.spans);
    stats.rejected = stats.rejected.saturating_add(edges.rejected);
}

fn consume_span(
    width: u32,
    span: Span,
    primitive: Primitive,
    stages: &[StagePlanes],
    cache: &mut SurfaceCache,
    rgba: &[Option<super::rgba::Recipe>],
    rgba_prepared: &[Option<super::rgba::Prepared>],
    factors: &[super::rgba::Factor],
    assets: &Assets,
    camera: &Camera,
    buffers: &mut Buffers<'_>,
    stats: &mut WorldStats,
) {
    let refdef = &camera.refdef;
    if let Some(source) = primitive.sky {
        let depth = primitive.planes.inverse_depth;
        super::sky::layered_span(
            width,
            buffers.frame_height,
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
                let before = stats.pixels;
                stats.indexed_spans = stats.indexed_spans.saturating_add(1);
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
                stats.indexed_pixels = stats
                    .indexed_pixels
                    .saturating_add(stats.pixels.saturating_sub(before));
            } else {
                stats.rejected += 1;
            }
        } else {
            stats.rejected += 1;
        }
        cache.end_batch();
    } else if let Some(index) = primitive.rgba {
        let (Some(recipe), Some(prepared)) = (rgba[index].as_ref(), rgba_prepared[index].as_ref())
        else {
            stats.rejected += 1;
            return;
        };
        match (recipe, prepared) {
            (super::rgba::Recipe::Product(recipe), super::rgba::Prepared::Product(prepared)) => {
                rgba_span(
                    width, span, primitive, recipe, prepared, cache, assets, buffers, stats,
                );
            }
            (super::rgba::Recipe::Pair(pair), super::rgba::Prepared::Pair) => {
                factor_span(
                    width, span, primitive, pair, stages, factors, cache, assets, buffers, stats,
                );
            }
            _ => stats.rejected = stats.rejected.saturating_add(1),
        }
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
            let midpoint = span.x as f32 + span.count as f32 * 0.5;
            let mip = stage
                .planes
                .mip(image, prepared.stage.sampler, midpoint, span.y as f32);
            let mut texels = super::TexelView::image(image, mip, prepared.stage.texture_intensity);
            if prepared.stage.texture == StageTexture::Lightmap
                && primitive.domain == PrimitiveDomain::WorldSurface
            {
                let Some(bounded) = texels.region(
                    assets
                        .world(primitive.world)
                        .and_then(|world| world.bindings().get(primitive.surface as usize))
                        .and_then(|binding| binding.lightmap_region),
                ) else {
                    stats.rejected += 1;
                    continue;
                };
                texels = bounded;
            }
            let before = stats.pixels;
            for x in span.x..span.x + span.count {
                let px = x as f32;
                let py = span.y as f32;
                let zi = stage.planes.inverse_depth.at(px, py);
                if zi <= 0.0 || !zi.is_finite() {
                    continue;
                }
                let index = buffers.offset(width, x, span.y);
                let depth = primitive.depth_override.unwrap_or(zi);
                if !super::depth_passes(
                    prepared.stage.depth_func,
                    depth,
                    buffers.inverse_depth[index],
                    primitive.draw_rank,
                    buffers.depth_ranks[index],
                ) {
                    continue;
                }
                let z = 1.0 / zi;
                let uv = stage.planes.texture.map(|plane| plane.at(px, py) * z);
                let color = stage.planes.color.map(|plane| plane.at(px, py) * z);
                let source = sampler(texels, uv, color);
                if !alpha_pass(prepared.stage.alpha_test, source[3]) {
                    continue;
                }
                buffers.pixels[index] =
                    super::composite(buffers.pixels[index], source, prepared.stage.blend);
                buffers.palettes[index] = u32::MAX;
                if prepared.stage.depth_write {
                    buffers.inverse_depth[index] = depth;
                    buffers.depth_ranks[index] = primitive.draw_rank;
                }
                stats.pixels = stats.pixels.saturating_add(1);
            }
            let written = stats.pixels.saturating_sub(before);
            if primitive.domain == PrimitiveDomain::GeneratedSky {
                stats.sky_spans = stats.sky_spans.saturating_add(1);
                stats.sky_pixels = stats.sky_pixels.saturating_add(written);
            } else {
                stats.stage_spans = stats.stage_spans.saturating_add(1);
                stats.stage_pixels = stats.stage_pixels.saturating_add(written);
                if primitive.patch {
                    stats.curve_spans = stats.curve_spans.saturating_add(1);
                    stats.curve_pixels = stats.curve_pixels.saturating_add(written);
                }
                if primitive.stages > 1 {
                    stats.multistage_spans = stats.multistage_spans.saturating_add(1);
                    stats.multistage_pixels = stats.multistage_pixels.saturating_add(written);
                }
            }
        }
    }
}

fn factor_span(
    width: u32,
    span: Span,
    primitive: Primitive,
    pair: &super::rgba::Pair,
    stages: &[StagePlanes],
    factors: &[super::rgba::Factor],
    cache: &mut SurfaceCache,
    assets: &Assets,
    buffers: &mut Buffers<'_>,
    stats: &mut WorldStats,
) {
    let a = stages[primitive.first_stage];
    let b = stages[primitive.first_stage + 1];
    let (Some(first), Some(second), Some(sample_a), Some(sample_b)) =
        (a.prepared, b.prepared, a.sampler, b.sampler)
    else {
        stats.rejected = stats.rejected.saturating_add(1);
        return;
    };
    let (Some(image_a), Some(image_b)) = (assets.image(first.image), assets.image(second.image))
    else {
        stats.rejected = stats.rejected.saturating_add(1);
        return;
    };
    let midpoint = span.x as f32 + span.count as f32 * 0.5;
    let mips = [
        a.planes
            .mip(image_a, first.stage.sampler, midpoint, span.y as f32),
        b.planes
            .mip(image_b, second.stage.sampler, midpoint, span.y as f32),
    ];
    let region = assets
        .world(primitive.world)
        .and_then(|world| world.bindings().get(primitive.surface as usize))
        .and_then(|binding| binding.lightmap_region);
    let raw_a = super::TexelView::image(image_a, mips[0], first.stage.texture_intensity).region(
        (first.stage.texture == StageTexture::Lightmap)
            .then_some(region)
            .flatten(),
    );
    let raw_b = super::TexelView::image(image_b, mips[1], second.stage.texture_intensity).region(
        (second.stage.texture == StageTexture::Lightmap)
            .then_some(region)
            .flatten(),
    );
    let (Some(raw_a), Some(raw_b)) = (raw_a, raw_b) else {
        stats.rejected = stats.rejected.saturating_add(1);
        return;
    };
    let before_cache = cache.stats();
    let active = cache.begin_batch();
    let cached = if active {
        [
            factors[pair.factors[0]].prepare(mips[0], assets, cache),
            factors[pair.factors[1]].prepare(mips[1], assets, cache),
        ]
    } else {
        [None; 2]
    };
    let after_cache = cache.stats();
    stats.factor_hits = stats
        .factor_hits
        .saturating_add(after_cache.hits.saturating_sub(before_cache.hits));
    stats.factor_fills = stats
        .factor_fills
        .saturating_add(after_cache.fills.saturating_sub(before_cache.fills));
    stats.factor_evictions = stats
        .factor_evictions
        .saturating_add(after_cache.evictions.saturating_sub(before_cache.evictions));
    stats.factor_rejected = stats
        .factor_rejected
        .saturating_add(after_cache.rejected.saturating_sub(before_cache.rejected));
    let copied_a = cached[0]
        .and_then(|block| cache.rgba_pixels(block))
        .and_then(|pixels| raw_a.copied(pixels));
    let copied_b = cached[1]
        .and_then(|block| cache.rgba_pixels(block))
        .and_then(|pixels| raw_b.copied(pixels));
    if copied_a.is_none() || copied_b.is_none() {
        stats.factor_fallback_spans = stats.factor_fallback_spans.saturating_add(1);
    }
    let texels_a = copied_a.unwrap_or(raw_a);
    let texels_b = copied_b.unwrap_or(raw_b);
    let before = stats.pixels;
    for x in span.x..span.x + span.count {
        let px = x as f32;
        let py = span.y as f32;
        let zi_a = a.planes.inverse_depth.at(px, py);
        if zi_a <= 0.0 || !zi_a.is_finite() {
            continue;
        }
        let index = buffers.offset(width, x, span.y);
        if !super::depth_passes(
            first.stage.depth_func,
            zi_a,
            buffers.inverse_depth[index],
            primitive.draw_rank,
            buffers.depth_ranks[index],
        ) {
            continue;
        }
        let z_a = 1.0 / zi_a;
        let source_a = sample_a(
            texels_a,
            a.planes.texture.map(|plane| plane.at(px, py) * z_a),
            a.planes.color.map(|plane| plane.at(px, py) * z_a),
        );
        let first_pixel =
            u32::from_le_bytes(source_a.map(|value| (value.clamp(0.0, 1.0) * 255.0).round() as u8));
        let zi_b = b.planes.inverse_depth.at(px, py);
        let color = if zi_b > 0.0
            && zi_b.is_finite()
            && super::depth_passes(
                second.stage.depth_func,
                zi_b,
                zi_a,
                primitive.draw_rank,
                primitive.draw_rank,
            ) {
            let z_b = 1.0 / zi_b;
            let source_b = sample_b(
                texels_b,
                b.planes.texture.map(|plane| plane.at(px, py) * z_b),
                b.planes.color.map(|plane| plane.at(px, py) * z_b),
            );
            super::rgba::multiply_pixel(first_pixel, source_b)
        } else {
            first_pixel
        };
        buffers.pixels[index] = color;
        buffers.palettes[index] = u32::MAX;
        buffers.inverse_depth[index] = zi_a;
        buffers.depth_ranks[index] = primitive.draw_rank;
        stats.pixels = stats.pixels.saturating_add(1);
    }
    stats.factor_spans = stats.factor_spans.saturating_add(1);
    let written = stats.pixels.saturating_sub(before);
    stats.factor_pixels = stats.factor_pixels.saturating_add(written);
    if primitive.patch {
        stats.factor_curve_spans = stats.factor_curve_spans.saturating_add(1);
        stats.factor_curve_pixels = stats.factor_curve_pixels.saturating_add(written);
    }
    if mips != [0; 2] {
        stats.factor_minified_spans = stats.factor_minified_spans.saturating_add(1);
    }
    if active {
        cache.end_batch();
    }
}

fn rgba_span(
    width: u32,
    span: Span,
    primitive: Primitive,
    recipe: &super::rgba::Product,
    prepared: &super::rgba::ProductPrepared,
    cache: &mut SurfaceCache,
    assets: &Assets,
    buffers: &mut Buffers<'_>,
    stats: &mut WorldStats,
) {
    let Some(base) = assets.image(recipe.base) else {
        stats.rejected += 1;
        return;
    };
    let chart_derivatives = primitive
        .planes
        .derivatives(span.x as f32 + span.count as f32 * 0.5, span.y as f32);
    let derivatives = chart_derivatives.map(|d| recipe.texture.map(|p| p[0] * d[0] + p[1] * d[1]));
    let mip = super::image_mip(base, recipe.base_sampler(), derivatives);
    if !cache.begin_batch() {
        stats.rejected += 1;
        stats.rgba_rejected = stats.rgba_rejected.saturating_add(1);
        return;
    }
    let before_cache = cache.stats();
    let block = cache.prepare_rgba(recipe.cache, mip, prepared.state, |destination| {
        recipe.fill(*prepared, mip, assets, destination);
    });
    let after_cache = cache.stats();
    stats.rgba_hits = stats
        .rgba_hits
        .saturating_add(after_cache.hits.saturating_sub(before_cache.hits));
    stats.rgba_fills = stats
        .rgba_fills
        .saturating_add(after_cache.fills.saturating_sub(before_cache.fills));
    stats.rgba_evictions = stats
        .rgba_evictions
        .saturating_add(after_cache.evictions.saturating_sub(before_cache.evictions));
    stats.rgba_rejected = stats
        .rgba_rejected
        .saturating_add(after_cache.rejected.saturating_sub(before_cache.rejected));
    if let Some(block) = block {
        if let Some(pixels) = cache.rgba_pixels(block) {
            let before = stats.pixels;
            stats.pixels = stats.pixels.saturating_add(packed::cached_rgba_span(
                width,
                span,
                &primitive.planes,
                primitive.draw_rank,
                block,
                pixels,
                buffers,
            ) as u64);
            stats.rgba_spans = stats.rgba_spans.saturating_add(1);
            if mip != 0 {
                stats.rgba_minified_spans = stats.rgba_minified_spans.saturating_add(1);
            }
            stats.rgba_pixels = stats
                .rgba_pixels
                .saturating_add(stats.pixels.saturating_sub(before));
        } else {
            stats.rejected += 1;
        }
    } else {
        stats.rejected += 1;
    }
    cache.end_batch();
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
            let index = buffers.offset(width, px, span.y);
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
#[path = "../../tests/cpu_prepared/world.rs"]
mod prepared_tests;

#[cfg(test)]
#[path = "../../tests/cpu_prepared/clip.rs"]
mod clipping_tests;

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
