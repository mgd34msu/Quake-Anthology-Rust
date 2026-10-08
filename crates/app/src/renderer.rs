//! Client composition uses the same scene packets for either renderer.
use crate::host::ClientView;
use qa_core::{math::angle_vectors, sys_events::SeatId};
use qa_platform::{Stopwatch, Window, WorkerError, Workers};
use qa_render::{
    Assets, BackendStats, BlendPhase, FrontEnd, Limits, Refdef, Viewport,
    cpu::{CpuBackend, CpuLimits, RasterBands, RasterConfig, render_band},
    gl::GlBackend,
    material::world_load::{LoadedWorld, PresentationDefaults},
    world::WorldView,
};

#[cfg(any(debug_assertions, feature = "allocation-tracking"))]
use qa_platform::allocations::Counts;
#[cfg(any(debug_assertions, feature = "allocation-tracking"))]
use qa_render::cpu::MAX_BANDS;

pub fn parse_cpu_bands(value: &str) -> Result<RasterBands, &'static str> {
    match value {
        "1" => Ok(RasterBands::One),
        "2" => Ok(RasterBands::Two),
        "4" => Ok(RasterBands::Four),
        "8" => Ok(RasterBands::Eight),
        _ => Err("CPU bands must be 1, 2, 4 or 8"),
    }
}

/// The app owns worker scheduling; render only lends disjoint raster jobs.
/// Developer retail draws use this same dispatcher and allocation accounting.
pub struct CpuDispatch {
    workers: Option<Workers>,
    failure: Option<WorkerError>,
    #[cfg(any(debug_assertions, feature = "allocation-tracking"))]
    scratch: [Counts; MAX_BANDS],
    #[cfg(any(debug_assertions, feature = "allocation-tracking"))]
    pending: Counts,
}
impl CpuDispatch {
    pub fn load(bands: RasterBands) -> Result<Self, WorkerError> {
        let workers = if bands == RasterBands::One {
            None
        } else {
            Some(Workers::load(bands.count())?)
        };
        Ok(Self {
            workers,
            failure: None,
            #[cfg(any(debug_assertions, feature = "allocation-tracking"))]
            scratch: [Counts::default(); MAX_BANDS],
            #[cfg(any(debug_assertions, feature = "allocation-tracking"))]
            pending: Counts::default(),
        })
    }

    pub fn worker_count(&self) -> usize {
        self.workers.as_ref().map_or(0, Workers::count)
    }

    pub fn failure(&self) -> Option<WorkerError> {
        self.failure
    }

    pub fn dispatch<J: Send>(
        &mut self,
        jobs: &mut [J],
        work: fn(&mut J),
    ) -> Result<(), WorkerError> {
        let result = if let Some(workers) = &mut self.workers {
            let result = workers.dispatch_scoped(jobs, work);
            #[cfg(any(debug_assertions, feature = "allocation-tracking"))]
            let result = {
                // Counts belong to this completed or rejected dispatch, even
                // when the worker operation failed. Read before another batch.
                let counts = &mut self.scratch[..workers.count()];
                let count_result = workers.allocation_counts(counts);
                if count_result.is_ok() {
                    for count in counts {
                        add_counts(&mut self.pending, *count);
                    }
                }
                result.and(count_result)
            };
            result
        } else {
            for job in jobs {
                work(job);
            }
            Ok(())
        };
        if let Err(error) = result {
            self.failure.get_or_insert(error);
        }
        result
    }

    /// Consume every dispatch since the preceding merge, exactly once.
    /// Inline jobs already belong to the caller's thread-local counts.
    #[cfg(any(debug_assertions, feature = "allocation-tracking"))]
    pub fn merge_counts(&mut self, mut caller: Counts) -> Counts {
        add_counts(&mut caller, std::mem::take(&mut self.pending));
        caller
    }
}

#[cfg(any(debug_assertions, feature = "allocation-tracking"))]
fn add_counts(total: &mut Counts, counts: Counts) {
    total.allocations = total.allocations.saturating_add(counts.allocations);
    total.reallocations = total.reallocations.saturating_add(counts.reallocations);
    total.requested_bytes = total.requested_bytes.saturating_add(counts.requested_bytes);
}

/// Cache sizes describe the backend's load-time reservation, across all bands.
pub fn report_cpu_config(config: RasterConfig, workers: usize) {
    qa_console::logger::console(format_args!(
        "{{\"event\":\"cpu_raster_config\",\"bands\":{},\"workers\":{},\"total_cache_budget_bytes\":{},\"allocated_cache_bytes\":{},\"per_band_cache_bytes\":{},\"mandatory_cache_bytes\":{},\"bin_index_capacity_bytes\":{},\"worker_affinity\":\"inherited_process_cpu_mask\",\"cpu_affinity_source\":\"private_harness_metadata\",\"individual_worker_pinning\":false}}\n",
        config.bands.count(),
        workers,
        config.total_cache_budget_bytes,
        config.allocated_cache_bytes,
        config.per_band_cache_bytes,
        config.mandatory_cache_bytes,
        config.bin_index_capacity_bytes,
    ));
}

#[derive(Clone, Copy, Debug, Default)]
pub enum Kind {
    #[default]
    Cpu,
    Gl,
}
impl Kind {
    pub fn parse(value: &str) -> Result<Self, &'static str> {
        match value {
            "cpu" => Ok(Self::Cpu),
            "gl" => Ok(Self::Gl),
            _ => Err("renderer must be cpu or gl"),
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            Self::Cpu => "cpu",
            Self::Gl => "gl",
        }
    }
    pub fn open(self, width: i32, height: i32) -> Result<Window, String> {
        match self {
            Self::Cpu => Window::open(width, height),
            Self::Gl => Window::open_gl(width, height),
        }
    }
}
#[derive(Clone, Copy, Debug, Default)]
pub struct Sample {
    pub frontend_ns: u64,
    pub backend_ns: u64,
    pub present_ns: u64,
    pub stats: BackendStats,
    pub presented: bool,
    pub visible_surfaces: u32,
}
enum Backend {
    Cpu {
        backend: CpuBackend,
        dispatch: CpuDispatch,
    },
    Gl(GlBackend),
}
struct WorldScene {
    presentation: PresentationDefaults,
    views: [WorldView; SeatId::COUNT],
}
pub struct Renderer {
    frontend: FrontEnd,
    assets: Assets,
    backend: Backend,
    scenes: Box<[Option<WorldScene>]>,
    kind: Kind,
    pub sample: Sample,
    width: u32,
    height: u32,
    recorded_rendered_frames: u64,
    draw_failed: bool,
}
impl Renderer {
    /// Assets are frozen while a backend and its packets reference them.
    pub fn load(
        kind: Kind,
        window: &Window,
        width: u32,
        height: u32,
        assets: Assets,
        worlds: Vec<LoadedWorld>,
        bands: RasterBands,
    ) -> Result<Self, String> {
        let mut scenes: Vec<Option<WorldScene>> =
            (0..assets.worlds().len()).map(|_| None).collect();
        let mut max_surfaces = 0usize;
        for loaded in worlds {
            let world = assets.world(loaded.world).ok_or("missing loaded world")?;
            let index = loaded.world.0 as usize;
            if scenes[index].is_some() {
                return Err("duplicate world presentation".into());
            }
            for diagnostic in &loaded.diagnostics {
                qa_console::logger::console(format_args!(
                    "world {}: {diagnostic}\n",
                    loaded.world.0
                ));
            }
            max_surfaces = max_surfaces.max(world.visibility().surface_count());
            scenes[index] = Some(WorldScene {
                presentation: loaded.presentation,
                views: std::array::from_fn(|_| WorldView::load(world)),
            });
        }
        if scenes.iter().any(Option::is_none) {
            return Err("missing world presentation defaults".into());
        }
        let mut limits = Limits::default();
        limits.surfaces = limits.surfaces.max(
            max_surfaces
                .checked_mul(SeatId::COUNT)
                .ok_or("scene surface capacity")?,
        );
        let frontend = FrontEnd::load(limits)?;
        let backend = match kind {
            Kind::Cpu => {
                let backend = CpuBackend::load_with_limits(
                    width,
                    height,
                    &assets,
                    CpuLimits {
                        scene: limits,
                        bands,
                        ..CpuLimits::default()
                    },
                )?;
                let dispatch = CpuDispatch::load(backend.raster_config().bands)
                    .map_err(|error| format!("CPU workers: {error}"))?;
                report_cpu_config(backend.raster_config(), dispatch.worker_count());
                Backend::Cpu { backend, dispatch }
            }
            Kind::Gl => Backend::Gl(unsafe {
                GlBackend::load(
                    |name| window.gl_proc(name),
                    &assets,
                    width,
                    height,
                    limits.vertices,
                )?
            }),
        };
        if let Backend::Gl(gl) = &backend {
            let (renderer, version) = gl.renderer_info();
            qa_console::logger::console(format_args!(
                "{{\"event\":\"gl_context\",\"renderer\":{},\"version\":{}}}\n",
                serde_json::to_string(renderer).map_err(|e| format!("GL renderer report: {e}"))?,
                serde_json::to_string(version).map_err(|e| format!("GL version report: {e}"))?
            ));
        }
        Ok(Self {
            frontend,
            assets,
            backend,
            scenes: scenes.into_boxed_slice(),
            kind,
            sample: Sample::default(),
            width,
            height,
            recorded_rendered_frames: 0,
            draw_failed: false,
        })
    }
    /// Last backend-rendered frame workload, with lifetime cache counters.
    pub fn cpu_world_stats(&self) -> Option<qa_render::cpu::WorldStats> {
        match &self.backend {
            Backend::Cpu { backend, .. } => Some(backend.world_stats()),
            Backend::Gl(_) => None,
        }
    }
    /// Includes startup and warmup; failed front-end acquisitions do not count.
    pub fn recorded_rendered_frames(&self) -> u64 {
        self.recorded_rendered_frames
    }
    pub fn worker_count(&self) -> usize {
        match &self.backend {
            Backend::Cpu { dispatch, .. } => dispatch.worker_count(),
            Backend::Gl(_) => 0,
        }
    }
    pub fn worker_error(&self) -> Option<WorkerError> {
        match &self.backend {
            Backend::Cpu { dispatch, .. } => dispatch.failure(),
            Backend::Gl(_) => None,
        }
    }
    #[cfg(any(debug_assertions, feature = "allocation-tracking"))]
    pub fn merge_worker_counts(&mut self, caller: Counts) -> Counts {
        match &mut self.backend {
            Backend::Cpu { dispatch, .. } => dispatch.merge_counts(caller),
            Backend::Gl(_) => caller,
        }
    }
    pub fn frame(&mut self, views: &[Option<ClientView>; SeatId::COUNT]) {
        self.sample = Sample::default();
        self.draw_failed = false;
        let timer = Stopwatch::start();
        let Some(mut frame) = self.frontend.begin_frame([18, 26, 34, 255]) else {
            self.sample.stats.rejected += 1;
            self.draw_failed = true;
            return;
        };
        let count = views.iter().filter(|view| view.is_some()).count();
        let mut view_index = 0;
        let mut rejected = 0;
        for (seat, client) in views.iter().enumerate() {
            let Some(client) = client else {
                continue;
            };
            let viewport = viewport(self.width, self.height, view_index, count);
            view_index += 1;
            let Some(scene) = self
                .scenes
                .get_mut(client.world.0 as usize)
                .and_then(Option::as_mut)
            else {
                rejected += 1;
                continue;
            };
            let Some(world) = self.assets.world(client.world) else {
                rejected += 1;
                continue;
            };
            let basis = angle_vectors(client.angles);
            let fov_x = if client.fov_x.is_finite() {
                client.fov_x.clamp(1.0, 179.0)
            } else {
                90.0
            };
            let aspect = viewport.height as f32 / viewport.width.max(1) as f32;
            let fov_y = (2.0 * ((fov_x.to_radians() * 0.5).tan() * aspect).atan()).to_degrees();
            let mut refdef = Refdef {
                viewport,
                origin: client.origin,
                axes: [basis.forward, -basis.right, basis.up],
                fov: [fov_x, fov_y],
                time_ms: client.time_ms,
                ..Refdef::default()
            };
            scene.presentation.apply(&mut refdef);
            if matches!(self.kind, Kind::Gl) {
                refdef.blend_phase = BlendPhase::AfterView;
                refdef.palette_transform = None;
            }
            let Ok(surfaces) = scene.views[seat].query(world, refdef, None, &[]) else {
                rejected += 1;
                continue;
            };
            self.sample.visible_surfaces += surfaces.len() as u32;
            frame.clear_scene();
            if !frame.add_world(client.world, surfaces)
                || !frame.render_scene(refdef, &[], &self.assets)
            {
                rejected += 1;
            }
        }
        let packet = frame.finish();
        self.sample.frontend_ns = timer.elapsed().as_nanos() as u64;
        let timer = Stopwatch::start();
        let result = match &mut self.backend {
            Backend::Cpu { backend, dispatch } => {
                backend.render_with_dispatch(&packet, &self.assets, |jobs| {
                    dispatch.dispatch(jobs, render_band)
                })
            }
            Backend::Gl(gl) => Ok(gl.render(&packet, &self.assets)),
        };
        self.sample.backend_ns = timer.elapsed().as_nanos() as u64;
        match result {
            Ok(stats) => {
                self.sample.stats = stats;
                self.recorded_rendered_frames += 1;
            }
            Err(_) => {
                self.sample.stats.rejected += 1;
                self.draw_failed = true;
            }
        }
        self.sample.stats.rejected += rejected;
        if self.frontend.recycle(packet).is_err() {
            self.sample.stats.rejected += 1;
        }
    }
    pub fn present(&mut self, window: &mut Window) {
        if self.draw_failed {
            self.sample.presented = false;
            self.sample.present_ns = 0;
            return;
        }
        let timer = Stopwatch::start();
        self.sample.presented = match &self.backend {
            Backend::Cpu { backend, .. } => {
                window.present_pixels(self.width, self.height, backend.pixels())
            }
            Backend::Gl(_) => window.present_gl(),
        };
        self.sample.present_ns = timer.elapsed().as_nanos() as u64;
    }
}

fn viewport(width: u32, height: u32, view: usize, count: usize) -> Viewport {
    let columns = if count > 1 { 2 } else { 1 };
    let rows = if count > 2 { 2 } else { 1 };
    let x = view as u32 % columns;
    let y = view as u32 / columns;
    let left = width * x / columns;
    let top = height * y / rows;
    Viewport {
        x: left,
        y: top,
        width: width * (x + 1) / columns - left,
        height: height * (y + 1) / rows - top,
    }
}
