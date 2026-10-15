//! Client composition uses the same scene packets for either renderer.
pub mod console;
pub mod hud;
pub(crate) mod native;
pub mod text;
use crate::{
    client_scene::{ClientSceneInput, SceneCounts},
    host::ClientView,
};
use qa_core::{math::angle_vectors, sys_events::SeatId};
use qa_platform::{Stopwatch, Window, WorkerError, Workers};
use qa_render::{
    Assets, BackendStats, BlendPhase, FrontEnd, Limits, Refdef, Viewport,
    cpu::{CpuBackend, CpuLimits, RasterBands, RasterConfig, run_cpu_job},
    gl::{GlBackend, GlContext},
    material::world_load::LoadedWorld,
    world::WorldView,
};

#[cfg(any(debug_assertions, feature = "allocation-tracking"))]
use qa_platform::allocations::Counts;
#[cfg(any(debug_assertions, feature = "allocation-tracking"))]
use qa_render::cpu::MAX_BANDS;

pub fn parse_cpu_bands(value: &str) -> Result<RasterBands, &'static str> {
    value
        .parse()
        .ok()
        .and_then(RasterBands::from_count)
        .ok_or("CPU bands must be 1, 2, 4 or 8")
}

/// Resolve once at renderer load. A benchmark override takes precedence over
/// the shared cvar; None requests affinity-aware automatic selection.
pub fn cpu_band_setting(
    cvars: &qa_console::cvars::Cvars,
    benchmark: Option<RasterBands>,
) -> Result<(qa_core::primitives::CvarHandle, Option<RasterBands>), &'static str> {
    let handle = cvars.find("r_cpuBands").ok_or("missing CPU band setting")?;
    let selected = if benchmark.is_some() {
        benchmark
    } else {
        match cvars.integer(handle) {
            0 => None,
            value => Some(
                usize::try_from(value)
                    .ok()
                    .and_then(RasterBands::from_count)
                    .ok_or("r_cpuBands must be 0, 1, 2, 4 or 8")?,
            ),
        }
    };
    Ok((handle, selected))
}

/// The app owns worker scheduling; render lends scoped preparation or raster jobs.
/// Developer retail draws use this same dispatcher and allocation accounting.
pub struct CpuDispatch {
    workers: Workers,
    failure: Option<WorkerError>,
    #[cfg(any(debug_assertions, feature = "allocation-tracking"))]
    scratch: [Counts; MAX_BANDS],
    #[cfg(any(debug_assertions, feature = "allocation-tracking"))]
    pending: Counts,
}
impl CpuDispatch {
    pub fn load(bands: RasterBands) -> Result<Self, WorkerError> {
        let workers = Workers::load(bands.count() - 1)?;
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
        self.workers.count()
    }

    pub fn failure(&self) -> Option<WorkerError> {
        self.failure
    }

    pub fn dispatch<J: Send>(
        &mut self,
        jobs: &mut [J],
        work: fn(&mut J),
    ) -> Result<(), WorkerError> {
        let result = self.workers.dispatch_scoped(jobs, work);
        #[cfg(any(debug_assertions, feature = "allocation-tracking"))]
        let result = {
            // Capture each completed or rejected batch before another dispatch.
            // The caller's participating jobs remain in its outer frame count.
            let counts = &mut self.scratch[..self.workers.count()];
            let count_result = self.workers.allocation_counts(counts);
            if count_result.is_ok() {
                for count in counts {
                    self.pending += *count;
                }
            }
            result.and(count_result)
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
        caller += std::mem::take(&mut self.pending);
        caller
    }
}

/// Cache sizes describe the backend's load-time reservation, across all bands.
pub fn report_cpu_config(config: RasterConfig, workers: usize) {
    qa_console::logger::console(format_args!(
        "{{\"event\":\"cpu_raster_config\",\"bands\":{},\"workers\":{},\"total_cache_budget_bytes\":{},\"allocated_cache_bytes\":{},\"per_band_cache_bytes\":{},\"mandatory_cache_bytes\":{},\"loaded_mip_working_set_bytes\":{},\"bin_index_capacity_bytes\":{},\"mip_layout_metadata_bytes\":{},\"span_group_capacity_bytes\":{},\"preparation_capacity_bytes\":{},\"material_capacity_bytes\":{},\"view_effect_capacity_bytes\":{},\"prepare_minimum_primitives_per_job\":{},\"worker_affinity\":\"inherited_process_cpu_mask\",\"cpu_affinity_source\":\"private_harness_metadata\",\"individual_worker_pinning\":false}}\n",
        config.bands.count(),
        workers,
        config.total_cache_budget_bytes,
        config.allocated_cache_bytes,
        config.per_band_cache_bytes,
        config.mandatory_cache_bytes,
        config.loaded_mip_working_set_bytes,
        config.bin_index_capacity_bytes,
        config.mip_layout_metadata_bytes,
        config.span_group_capacity_bytes,
        config.preparation_capacity_bytes,
        config.material_capacity_bytes,
        config.view_effect_capacity_bytes,
        config.prepare_minimum_primitives_per_job,
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

/// Cold context and capability owner; it becomes the only live backend after
/// CLIENT registration finishes, without resolving the GL table a second time.
pub enum Context {
    Cpu(std::sync::Arc<qa_core::presentation::RendererConfiguration>),
    Gl(GlContext),
}
impl Context {
    pub fn load(kind: Kind, window: &Window) -> Result<Self, String> {
        let display = window.display_configuration()?;
        match kind {
            Kind::Cpu => {
                let mut configuration = qa_core::presentation::RendererConfiguration {
                    display,
                    max_texture_size: qa_render::assets::upload::MAX_DIMENSION,
                    max_active_textures: 1,
                    texture_env_add: true,
                    ..Default::default()
                };
                configuration
                    .renderer
                    .set("Quake Anthology CPU edge/span")
                    .map_err(|_| "CPU identity capacity")?;
                configuration
                    .vendor
                    .set("Quake Anthology")
                    .map_err(|_| "CPU identity capacity")?;
                configuration
                    .version
                    .set("CPU")
                    .map_err(|_| "CPU identity capacity")?;
                Ok(Self::Cpu(std::sync::Arc::new(configuration)))
            }
            Kind::Gl => {
                // SAFETY: Window owns this thread's current SDL GL context and
                // outlives Context and the consuming backend.
                Ok(Self::Gl(unsafe {
                    GlContext::load(|name| window.gl_proc(name), display)?
                }))
            }
        }
    }
    pub fn configuration(&self) -> &std::sync::Arc<qa_core::presentation::RendererConfiguration> {
        match self {
            Self::Cpu(configuration) => configuration,
            Self::Gl(context) => context.configuration(),
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
    pub client_scene: SceneCounts,
    pub console: console::ConsoleStats,
    pub hud: hud::HudStats,
}
#[expect(
    clippy::large_enum_variant,
    reason = "Keep load-owned backend layout inline; changing indirection requires separate measurement"
)]
enum Backend {
    Cpu {
        backend: CpuBackend,
        dispatch: CpuDispatch,
    },
    Gl(GlBackend),
}
struct WorldScene {
    views: [WorldView; SeatId::COUNT],
    presentations: Box<[qa_render::material::world_load::LoadedWorldPresentation]>,
}
#[allow(clippy::too_many_arguments)]
pub(crate) fn light_view_entities(
    frame: &mut qa_render::scene::Frame,
    assets: &Assets,
    world: Option<qa_render::world::WorldId>,
    view: &Refdef,
    use_world_lighting: bool,
    scales: [f32; 2],
    mut point_view: Option<&mut WorldView>,
    modulate: f32,
) {
    let world = world.and_then(|world| assets.world(world));
    let sun = world.map_or_else(qa_render::lighting::default_sun_direction, |world| {
        world.sun_direction()
    });
    let world = world.filter(|_| use_world_lighting);
    let grid = world.and_then(|world| world.light_grid());
    let unlit = qa_render::lighting::unlit_sample(view.identity_light, sun);
    frame.light_entities(view.identity_light, |entity| {
        if let Some(grid) = grid {
            return grid.sample(entity.light_origin(), scales[0], scales[1]);
        }
        world
            .zip(point_view.as_deref_mut())
            .and_then(|(world, scratch)| {
                scratch.point_lighting(world, entity.light_origin(), &view.lightstyles, modulate)
            })
            .unwrap_or(unlit)
    });
}
/// Each CLIENT supplies its selected presentation independently of the world.
pub fn view_refdef(client: ClientView, viewport: Viewport, kind: Kind) -> Refdef {
    let basis = angle_vectors(client.angles);
    let fov = qa_render::scene::fov_from_x(client.fov_x, viewport.width, viewport.height);
    let mut refdef = Refdef {
        viewport,
        origin: client.origin,
        axes: basis.axes(),
        fov,
        blend: client.screen_blend,
        damage_blend: client.damage_blend,
        flags: client.flags,
        time_ms: client.time_ms as i64,
        ..Refdef::default()
    };
    apply_presentation(&mut refdef, &client.presentation, kind);
    refdef
}
fn apply_presentation(
    refdef: &mut Refdef,
    presentation: &qa_render::material::world_load::PresentationDefaults,
    kind: Kind,
) {
    presentation.apply(refdef);
    if matches!(kind, Kind::Gl) {
        refdef.software_waterwarp = false;
        refdef.blend_phase = BlendPhase::AfterView;
        refdef.palette_transform = None;
    }
}
/// The session owns one front end and its view scratch across module dispatch
/// and built-in client composition.
pub struct SceneComposer {
    frontend: FrontEnd,
    assets: Assets,
    scenes: Box<[Option<WorldScene>]>,
    frame: Option<qa_render::scene::Frame>,
    publisher: Option<qa_core::primitives::ModuleId>,
    native_owned: [bool; SeatId::COUNT],
    native_axes: [Option<[qa_core::primitives::Vec3; 3]>; SeatId::COUNT],
    kind: Kind,
    beam: qa_render::MaterialId,
    beam_alpha: qa_render::MaterialId,
}

impl SceneComposer {
    pub fn load(
        mut limits: Limits,
        mut assets: Assets,
        worlds: Vec<LoadedWorld>,
    ) -> Result<(Self, Limits), String> {
        let mut scenes: Vec<Option<WorldScene>> =
            (0..assets.worlds().len()).map(|_| None).collect();
        let mut max_surfaces = 0usize;
        for loaded in worlds {
            let world = assets.world(loaded.world).ok_or("missing loaded world")?;
            let index = loaded.world.0 as usize;
            if scenes[index].is_some() {
                return Err("duplicate world geometry".into());
            }
            for diagnostic in &loaded.diagnostics {
                qa_console::logger::console(format_args!(
                    "world {}: {diagnostic}\n",
                    loaded.world.0
                ));
            }
            max_surfaces = max_surfaces.max(world.visibility().surface_count());
            scenes[index] = Some(WorldScene {
                views: std::array::from_fn(|_| WorldView::load(world)),
                presentations: loaded.presentations,
            });
        }
        if scenes.iter().any(Option::is_none) {
            return Err("missing loaded world geometry".into());
        }
        limits.surfaces = limits.surfaces.max(
            max_surfaces
                .checked_mul(SeatId::COUNT)
                .ok_or("scene surface capacity")?,
        );

        let frontend = FrontEnd::load(limits)?;
        let beam = qa_render::entities::register_beam_material(
            &mut assets,
            qa_render::shader::StageBlend {
                source: qa_render::shader::BlendFactor::One,
                destination: qa_render::shader::BlendFactor::One,
            },
        )
        .map_err(str::to_owned)?;
        let beam_alpha = qa_render::entities::register_beam_material(
            &mut assets,
            qa_render::shader::StageBlend::ALPHA,
        )
        .map_err(str::to_owned)?;
        Ok((
            Self {
                frontend,
                assets,
                scenes: scenes.into_boxed_slice(),
                frame: None,
                publisher: None,
                native_owned: [false; SeatId::COUNT],
                native_axes: [None; SeatId::COUNT],
                kind: Kind::Cpu,
                beam,
                beam_alpha,
            },
            limits,
        ))
    }

    pub fn frame_mut(&mut self) -> Option<&mut qa_render::scene::Frame> {
        self.frame.as_mut()
    }

    pub fn finish_frame(&mut self) -> Option<qa_render::scene::CommandList> {
        self.frame.take().map(qa_render::scene::Frame::finish)
    }

    pub fn recycle(
        &mut self,
        packet: qa_render::scene::CommandList,
    ) -> Result<(), qa_render::scene::CommandList> {
        self.frontend.recycle(packet)
    }

    pub fn begin_frame(&mut self) -> bool {
        self.discard_frame();
        self.publisher = None;
        self.native_axes.fill(None);
        self.frame = self.frontend.begin_frame([18, 26, 34, 255]);
        self.frame.is_some()
    }
    pub(crate) fn native_owners(&mut self, owners: [bool; SeatId::COUNT]) {
        self.native_owned = owners;
    }
    pub(crate) fn submitted_axes(&self, seat: usize) -> Option<[qa_core::primitives::Vec3; 3]> {
        self.native_axes[seat]
    }

    /// A headless client phase still retires its owned list before reuse.
    pub fn discard_frame(&mut self) {
        if let Some(frame) = self.frame.take() {
            let _ = self.frontend.recycle(frame.finish());
        }
    }
}

pub struct Renderer {
    effects: qa_render::effects::Effects,
    effect_clients: [Option<qa_render::effects::EffectClient>; SeatId::COUNT],
    effect_time: qa_core::sys_events::EventTime,
    effect_gravity: f32,
    backend: Backend,
    kind: Kind,
    pub sample: Sample,
    width: u32,
    height: u32,
    recorded_rendered_frames: u64,
    draw_failed: bool,
    console: console::ConsoleAssets,
    hud: [Option<hud::HudAssets>; 5],
    lightstyles: [hud::LightStyles; SeatId::COUNT],
}
impl Renderer {
    /// Assets are frozen while a backend and its packets reference them.
    pub fn load(
        context: Context,
        width: u32,
        height: u32,
        assets: Assets,
        worlds: Vec<LoadedWorld>,
        bands: Option<RasterBands>,
        console: console::ConsoleAssets,
        hud: [Option<hud::HudAssets>; 5],
        effects: qa_render::effects::Effects,
    ) -> Result<(Self, SceneComposer), String> {
        let kind = match &context {
            Context::Cpu(_) => Kind::Cpu,
            Context::Gl(_) => Kind::Gl,
        };
        let display = context.configuration().display;
        if display.width != width || display.height != height {
            return Err("renderer target changed during registration".into());
        }
        let mut limits = Limits::default();
        limits.particles = effects
            .capacity()
            .checked_mul(SeatId::COUNT)
            .ok_or("particle scene capacity")?;
        let hud_capacity = hud
            .iter()
            .flatten()
            .map(hud::HudAssets::command_capacity)
            .max()
            .unwrap_or(0)
            .checked_mul(SeatId::COUNT)
            .ok_or("HUD scene capacity")?;
        limits.commands = limits
            .commands
            .checked_add(console.command_capacity())
            .and_then(|count| count.checked_add(hud_capacity))
            .ok_or("console scene capacity")?;
        limits.vertices = limits
            .vertices
            .checked_add(
                console
                    .command_capacity()
                    .checked_add(hud_capacity)
                    .ok_or("HUD vertex capacity")?
                    .checked_mul(6)
                    .ok_or("console vertex capacity")?,
            )
            .ok_or("console vertex capacity")?;
        let (mut scene, limits) = SceneComposer::load(limits, assets, worlds)?;
        scene.kind = kind;
        let backend = match context {
            Context::Cpu(_) => {
                let physical_cores = qa_platform::physical_core_count();
                let backend = CpuBackend::load_with_limits(
                    width,
                    height,
                    &scene.assets,
                    CpuLimits {
                        scene: limits,
                        bands: bands.unwrap_or_else(|| RasterBands::at_most(physical_cores)),
                        auto_bands: bands.is_none(),
                        ..CpuLimits::default()
                    },
                )?;
                let dispatch = CpuDispatch::load(backend.raster_config().bands)
                    .map_err(|error| format!("CPU workers: {error}"))?;
                report_cpu_config(backend.raster_config(), dispatch.worker_count());
                qa_console::logger::console(format_args!(
                    "{{\"event\":\"cpu_band_selection\",\"automatic\":{},\"physical_cores\":{},\"bands\":{}}}\n",
                    bands.is_none(),
                    physical_cores,
                    backend.raster_config().bands.count()
                ));
                Backend::Cpu { backend, dispatch }
            }
            Context::Gl(context) => Backend::Gl(unsafe {
                GlBackend::load(context, &scene.assets, width, height, limits.vertices)?
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
        Ok((
            Self {
                effects,
                effect_clients: [None; SeatId::COUNT],
                effect_time: qa_core::sys_events::EventTime::default(),
                effect_gravity: 800.0,
                backend,
                kind,
                sample: Sample::default(),
                width,
                height,
                recorded_rendered_frames: 0,
                draw_failed: false,
                console,
                hud,
                lightstyles: std::array::from_fn(|_| Default::default()),
            },
            scene,
        ))
    }
    pub fn effect(&mut self, event: qa_core::primitives::EffectEvent) -> bool {
        self.effects.accept(event)
    }
    pub fn set_effect_clients(
        &mut self,
        clients: [Option<qa_render::effects::EffectClient>; SeatId::COUNT],
        time: qa_core::sys_events::EventTime,
        gravity: f32,
    ) {
        self.effects.synchronize(&clients);
        self.effect_clients = clients;
        self.effect_time = time;
        self.effect_gravity = gravity;
    }
    fn effect_phase(
        &mut self,
        action: fn(
            &mut qa_render::effects::Effects,
            qa_core::primitives::ClientId,
            qa_core::primitives::EntityId,
            qa_core::sys_events::EventTime,
            f32,
        ) -> bool,
    ) {
        for (seat, client) in self.effect_clients.iter().enumerate() {
            let Some(client) = client else { continue };
            if self.effect_clients[..seat].contains(&Some(*client)) {
                continue;
            }
            action(
                &mut self.effects,
                client.client,
                client.lifetime,
                self.effect_time,
                self.effect_gravity,
            );
        }
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
    pub fn frame(
        &mut self,
        composition: &mut SceneComposer,
        views: &[Option<ClientView>; SeatId::COUNT],
        scenes: &[Option<ClientSceneInput<'_>>; SeatId::COUNT],
        hud: &[Option<hud::HudView<'_>>; SeatId::COUNT],
        console: qa_console::presentation::ConsoleView<'_>,
    ) {
        self.sample = Sample::default();
        self.draw_failed = false;
        self.effect_phase(qa_render::effects::Effects::begin_frame);
        let timer = Stopwatch::start();
        if composition.frame.is_none() {
            composition.begin_frame();
        }
        let Some(mut frame) = composition.frame.take() else {
            self.sample.stats.rejected += 1;
            self.draw_failed = true;
            self.effect_phase(qa_render::effects::Effects::finish_frame);
            return;
        };
        let count = views.iter().filter(|view| view.is_some()).count();
        let mut view_index = 0;
        let mut rejected = 0;
        frame.append_client_effects(&mut self.effects, &composition.assets);
        frame.background(|frame| {
            for (seat, client) in views.iter().enumerate() {
                let Some(client) = client else {
                    continue;
                };
                let viewport = viewport(self.width, self.height, view_index, count);
                view_index += 1;
                if composition.native_owned[seat] {
                    rejected += u32::from(composition.native_axes[seat].is_none());
                    continue;
                }
                let Some(scene) = composition
                    .scenes
                    .get_mut(client.world.0 as usize)
                    .and_then(Option::as_mut)
                else {
                    rejected += 1;
                    continue;
                };
                let Some(world) = composition.assets.world(client.world) else {
                    rejected += 1;
                    continue;
                };
                let mut refdef = view_refdef(*client, viewport, self.kind);
                if let Some(hud) = hud[seat] {
                    self.sample.hud +=
                        hud.apply_lightstyles(&mut refdef, &mut self.lightstyles[seat]);
                }
                // Native snapshot areas use at most 255 bytes. This bounded stack
                // scratch is shared by visibility and the copied scene command.
                let mut area_mask = [0; 256];
                let hidden_areas = if let Some(client_scene) = &scenes[seat] {
                    let (mask, truncated) = client_scene.hidden_areas(&mut area_mask);
                    self.sample.client_scene.unsupported += u32::from(truncated);
                    mask
                } else {
                    &[]
                };
                let no_world = refdef
                    .flags
                    .contains(qa_core::primitives::PlayerViewFlags::NO_WORLD_MODEL);
                let surfaces = if no_world {
                    &[][..]
                } else {
                    let Ok(surfaces) = scene.views[seat].query(world, refdef, None, hidden_areas)
                    else {
                        rejected += 1;
                        continue;
                    };
                    surfaces
                };
                self.sample.visible_surfaces += surfaces.len() as u32;
                frame.clear_scene();
                let world_added =
                    no_world || frame.add_world(client.world, client.world_presentation, surfaces);
                if let Some(client_scene) = &scenes[seat] {
                    let mut random = || self.effects.random_word();
                    self.sample.client_scene += client_scene.append(
                        frame,
                        &composition.assets,
                        &refdef,
                        matches!(self.kind, Kind::Cpu),
                        Some(crate::client_scene::BeamInput {
                            material: composition.beam_alpha,
                            random: &mut random,
                        }),
                    );
                }
                if let Some(effect_client) = self.effect_clients[seat] {
                    self.effects
                        .append(frame, effect_client.client, effect_client.lifetime);
                }
                light_view_entities(
                    frame,
                    &composition.assets,
                    Some(client.world),
                    &refdef,
                    !no_world,
                    client.lighting_scales,
                    Some(&mut scene.views[seat]),
                    scene
                        .presentations
                        .iter()
                        .find(|entry| entry.id == client.world_presentation)
                        .map_or(1., |entry| entry.profile.q2_modulate),
                );
                let rendered = if no_world {
                    frame.render_scene(refdef, hidden_areas, &composition.assets)
                } else {
                    frame.render_world_scene(
                        client.world,
                        client.world_presentation,
                        refdef,
                        hidden_areas,
                        &mut scene.views[seat],
                        &composition.assets,
                    )
                };
                if !world_added || !rendered {
                    rejected += 1;
                }
                if let Some(hud) = hud[seat] {
                    if let Some(assets) = self.hud[hud.rules as usize].as_mut() {
                        self.sample.hud += assets.append(frame, &composition.assets, hud, refdef);
                    } else if hud.show_text {
                        self.sample.hud.missing_font += 1;
                    }
                }
            }
        });
        self.sample.console = self.console.append(
            &mut frame,
            &composition.assets,
            console,
            self.width,
            self.height,
            matches!(self.kind, Kind::Cpu),
        );
        rejected += self.sample.console.rejected;
        rejected += self.sample.hud.rejected;
        let packet = frame.finish();
        self.sample.frontend_ns = timer.elapsed().as_nanos() as u64;
        let timer = Stopwatch::start();
        let result = match &mut self.backend {
            Backend::Cpu { backend, dispatch } => {
                backend.render_with_dispatch(&packet, &composition.assets, |jobs| {
                    dispatch.dispatch(jobs, run_cpu_job)
                })
            }
            Backend::Gl(gl) => Ok(gl.render(&packet, &composition.assets)),
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
        if composition.recycle(packet).is_err() {
            self.sample.stats.rejected += 1;
        }
        self.effect_phase(qa_render::effects::Effects::finish_frame);
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

pub(crate) fn viewport(width: u32, height: u32, view: usize, count: usize) -> Viewport {
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
