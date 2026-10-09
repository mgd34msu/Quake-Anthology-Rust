//! Fixed-time retail CPU draw benchmark. Run only through the private harness.
//! This developer example neither drives input nor qualifies gameplay/installation.
//!
//! Both the example and shared app dispatcher need allocation tracking:
//! ```sh
//! cargo build --release -p qa-platform --example cpu_retail \
//!   --features allocation-tracking,qa-app/allocation-tracking
//! ```
//! Select `--cpu-bands 1`, `2`, `4` or `8`; the harness records the inherited
//! process CPU mask. `--depth-output` writes final depth scores after measurement.
//! `--stage-timings` adds diagnostic callback timers. Keep those rows separate
//! from the normal benchmark; the residual estimates serial/other wall cost.
use qa_app::{
    Runtime, map, profile, render_settings,
    renderer::{CpuDispatch, cpu_band_setting, parse_cpu_bands, report_cpu_config},
};
use qa_console::{
    commands::Console,
    logger,
    views::{Context, RuleSetId},
};
use qa_core::{math::angle_vectors, sys_events::SeatId};
use qa_platform::{Stopwatch, Window, pause};
use qa_render::{
    Assets, BackendStats, CpuPresentation, FrontEnd, Limits, Refdef, Viewport,
    cpu::{
        CpuBackend, CpuLimits, JobKind, MAX_BANDS, PreparePoint, RasterBands, WorldStats,
        run_cpu_job,
    },
    material::world_load::WorldLoadOptions,
    world::WorldView,
};
use std::{
    fs::OpenOptions,
    io::{BufWriter, Write},
    path::{Path, PathBuf},
    time::Duration,
};

const WIDTH: u32 = 640;
const HEIGHT: u32 = 400;
const WARMUP: usize = 60;
const FRAMES: usize = 600;

#[cfg(any(debug_assertions, feature = "allocation-tracking"))]
#[global_allocator]
static ALLOCATOR: qa_platform::allocations::CountingAllocator =
    qa_platform::allocations::CountingAllocator;

struct Options {
    content: PathBuf,
    map: String,
    output: PathBuf,
    depth_output: Option<PathBuf>,
    hold_ms: u64,
    bands: Option<RasterBands>,
    stage_timings: bool,
}
impl Options {
    fn read() -> Result<Self, String> {
        let (mut content, mut map, mut output) = (None, None, None);
        let mut depth_output = None;
        let mut bands = None;
        let mut hold_ms = 1500;
        let mut stage_timings = false;
        let mut args = std::env::args().skip(1);
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--content" if content.is_none() => {
                    content = Some(PathBuf::from(
                        args.next().ok_or("--content needs a product directory")?,
                    ));
                }
                "--map" if map.is_none() => {
                    map = Some(args.next().ok_or("--map needs a virtual map name")?)
                }
                "--output" if output.is_none() => {
                    output = Some(PathBuf::from(
                        args.next().ok_or("--output needs a raw RGBA path")?,
                    ));
                }
                "--depth-output" if depth_output.is_none() => {
                    depth_output = Some(PathBuf::from(
                        args.next()
                            .ok_or("--depth-output needs an inverse-depth path")?,
                    ));
                }
                "--cpu-bands" if bands.is_none() => {
                    bands = Some(parse_cpu_bands(
                        &args.next().ok_or("--cpu-bands needs 1, 2, 4 or 8")?,
                    )?);
                }
                "--stage-timings" if !stage_timings => stage_timings = true,
                "--startup-hold-ms" => {
                    hold_ms = args
                        .next()
                        .ok_or("--startup-hold-ms needs a duration")?
                        .parse()
                        .map_err(|_| "invalid startup hold duration")?;
                }
                _ => return Err(format!("unknown or duplicate benchmark option: {arg}")),
            }
        }
        if hold_ms > 10_000 {
            return Err("startup hold exceeds 10000 ms".into());
        }
        let content = content.ok_or("--content is required")?;
        let map = map.ok_or("--map is required")?;
        let output = output.ok_or("--output is required")?;
        if !content.is_absolute()
            || !output.is_absolute()
            || depth_output
                .as_ref()
                .is_some_and(|path| !path.is_absolute())
        {
            return Err("content and output paths must be absolute".into());
        }
        Ok(Self {
            content,
            map,
            output,
            depth_output,
            hold_ms,
            bands,
            stage_timings,
        })
    }
}

fn copied_profile() -> Result<(PathBuf, PathBuf), String> {
    let home = PathBuf::from(std::env::var_os("HOME").ok_or("copied HOME is required")?);
    if !home.is_absolute() {
        return Err("copied HOME must be absolute".into());
    }
    let home = home
        .canonicalize()
        .map_err(|e| format!("copied HOME: {e}"))?;
    let root = qa_platform::saved_profile_root()
        .ok_or("copied saved profile is required")?
        .canonicalize()
        .map_err(|e| format!("copied saved profile: {e}"))?;
    if !root.starts_with(&home) {
        return Err("saved profile must be inside the copied HOME".into());
    }
    Ok((home, root))
}

fn private_environment() -> Result<(), String> {
    if std::env::var_os("WAYLAND_DISPLAY").is_some()
        || std::env::var("SDL_VIDEODRIVER").as_deref() != Ok("x11")
        || std::env::var("SDL_VIDEO_DRIVER").as_deref() != Ok("x11")
        || std::env::var_os("DISPLAY").is_none()
    {
        return Err("use the private X11 harness with WAYLAND_DISPLAY unset and both SDL video drivers forced to x11".into());
    }
    let legacy =
        std::env::var("SDL_AUDIODRIVER").map_err(|_| "private SDL audio driver is required")?;
    let current =
        std::env::var("SDL_AUDIO_DRIVER").map_err(|_| "private SDL3 audio driver is required")?;
    if legacy != current || !matches!(current.as_str(), "disk" | "dummy") {
        return Err(
            "both SDL audio drivers must select the same private disk or dummy sink".into(),
        );
    }
    Ok(())
}

fn output_path(path: &Path, content: &Path, profile: &Path) -> Result<PathBuf, String> {
    let parent = path
        .parent()
        .ok_or("output needs a parent directory")?
        .canonicalize()
        .map_err(|e| format!("output directory: {e}"))?;
    let output = parent.join(path.file_name().ok_or("output needs a filename")?);
    if output.starts_with(content)
        || output.starts_with(profile)
        || output.components().any(|part| part.as_os_str() == "qfiles")
    {
        return Err("pixel evidence must be outside product/profile directories and qfiles".into());
    }
    if output.exists() {
        return Err("pixel evidence output already exists".into());
    }
    Ok(output)
}

fn json_string(value: &str) {
    logger::console(format_args!("\""));
    for character in value.chars() {
        match character {
            '"' => logger::console(format_args!("\\\"")),
            '\\' => logger::console(format_args!("\\\\")),
            c if c.is_control() => logger::console(format_args!("\\u{:04x}", c as u32)),
            c => logger::console(format_args!("{c}")),
        }
    }
    logger::console(format_args!("\""));
}

fn backend_stats(stats: BackendStats) {
    logger::console(format_args!(
        "{{\"views\":{},\"surfaces\":{},\"triangles\":{},\"stages\":{},\"draws_2d\":{},\"rejected\":{},\"pending_lights\":{}}}",
        stats.views,
        stats.surfaces,
        stats.triangles,
        stats.stages,
        stats.draws_2d,
        stats.rejected,
        stats.pending_lights
    ));
}

fn world_stats(stats: WorldStats) {
    // Cache/raster categories can change with partitioning. The immutable
    // packet, final pixels and depth scores define comparison fidelity.
    logger::console(format_args!(
        "{{\"polygons\":{},\"patch_polygons\":{},\"spans\":{},\"pixels\":{},\"sky_spans\":{},\"sky_pixels\":{},\"stage_spans\":{},\"stage_pixels\":{},\"curve_spans\":{},\"curve_pixels\":{},\"multistage_spans\":{},\"multistage_pixels\":{},\"indexed_spans\":{},\"indexed_pixels\":{},\"rgba_spans\":{},\"rgba_pixels\":{},\"rgba_hits\":{},\"rgba_fills\":{},\"rgba_evictions\":{},\"rgba_rejected\":{},\"rgba_minified_spans\":{},\"factor_spans\":{},\"factor_pixels\":{},\"factor_hits\":{},\"factor_fills\":{},\"factor_evictions\":{},\"factor_rejected\":{},\"factor_fallback_spans\":{},\"factor_minified_spans\":{},\"factor_curve_spans\":{},\"factor_curve_pixels\":{},\"rejected\":{},\"cache_cumulative\":{{\"hits\":{},\"fills\":{},\"evictions\":{},\"rejected\":{}}}}}",
        stats.polygons,
        stats.patch_polygons,
        stats.spans,
        stats.pixels,
        stats.sky_spans,
        stats.sky_pixels,
        stats.stage_spans,
        stats.stage_pixels,
        stats.curve_spans,
        stats.curve_pixels,
        stats.multistage_spans,
        stats.multistage_pixels,
        stats.indexed_spans,
        stats.indexed_pixels,
        stats.rgba_spans,
        stats.rgba_pixels,
        stats.rgba_hits,
        stats.rgba_fills,
        stats.rgba_evictions,
        stats.rgba_rejected,
        stats.rgba_minified_spans,
        stats.factor_spans,
        stats.factor_pixels,
        stats.factor_hits,
        stats.factor_fills,
        stats.factor_evictions,
        stats.factor_rejected,
        stats.factor_fallback_spans,
        stats.factor_minified_spans,
        stats.factor_curve_spans,
        stats.factor_curve_pixels,
        stats.rejected,
        stats.cache.hits,
        stats.cache.fills,
        stats.cache.evictions,
        stats.cache.rejected
    ));
}

fn write_rgba(path: &Path, pixels: &[u32]) -> std::io::Result<()> {
    let file = OpenOptions::new().write(true).create_new(true).open(path)?;
    let mut file = BufWriter::new(file);
    for pixel in pixels {
        file.write_all(&pixel.to_le_bytes())?;
    }
    file.flush()
}

fn write_depth(path: &Path, depth: &[f32]) -> std::io::Result<()> {
    let file = OpenOptions::new().write(true).create_new(true).open(path)?;
    let mut file = BufWriter::new(file);
    for value in depth {
        file.write_all(&value.to_bits().to_le_bytes())?;
    }
    file.flush()
}

#[derive(Clone, Copy, Default)]
struct DispatchTimings {
    prepare_ns: u64,
    prepare_dispatches: u64,
    prepare_jobs: u64,
    first_ns: u64,
    subsequent_ns: u64,
    count: u64,
    maximum_ns: u64,
}
impl DispatchTimings {
    fn observe(&mut self, elapsed_ns: u64) {
        if self.count == 0 {
            self.first_ns = elapsed_ns;
        } else {
            self.subsequent_ns = self.subsequent_ns.saturating_add(elapsed_ns);
        }
        self.count += 1;
        self.maximum_ns = self.maximum_ns.max(elapsed_ns);
    }
    fn waited_ns(self) -> u64 {
        self.first_ns.saturating_add(self.subsequent_ns)
    }
}

fn draw(
    cpu: &mut CpuBackend,
    dispatch: &mut CpuDispatch,
    packet: &qa_render::scene::CommandList,
    assets: &Assets,
    timings: Option<&mut DispatchTimings>,
) -> Result<BackendStats, qa_platform::WorkerError> {
    if let Some(timings) = timings {
        let mut prepare_timer = None;
        let mut dispatch_timings = DispatchTimings::default();
        let result = cpu.render_observed(
            packet,
            assets,
            |jobs| {
                let kind = jobs.first().map(|job| job.kind());
                let timer = Stopwatch::start();
                let result = dispatch.dispatch(jobs, run_cpu_job);
                if kind == Some(JobKind::Raster) {
                    dispatch_timings.observe(timer.elapsed().as_nanos() as u64);
                } else {
                    dispatch_timings.prepare_dispatches += 1;
                    dispatch_timings.prepare_jobs += jobs.len() as u64;
                }
                result
            },
            |point| match point {
                PreparePoint::Begin => prepare_timer = Some(Stopwatch::start()),
                PreparePoint::End => {
                    if let Some(timer) = prepare_timer.take() {
                        timings.prepare_ns = timings
                            .prepare_ns
                            .saturating_add(timer.elapsed().as_nanos() as u64);
                    }
                }
            },
        );
        dispatch_timings.prepare_ns = timings.prepare_ns;
        *timings = dispatch_timings;
        result
    } else {
        cpu.render_with_dispatch(packet, assets, |jobs| dispatch.dispatch(jobs, run_cpu_job))
    }
}

fn timing_summary(name: &str, unit: &str, mut values: [u64; FRAMES]) {
    values.sort_unstable();
    logger::console(format_args!(
        "\"{name}\":{{\"unit\":\"{unit}\",\"median\":{},\"p99\":{},\"minimum\":{},\"maximum\":{}}}",
        values[299] as f64 * 0.5 + values[300] as f64 * 0.5,
        values[593],
        values[0],
        values[FRAMES - 1],
    ));
}

fn report_stage_timings(
    frames: &[DispatchTimings; FRAMES],
    total: &[u64; FRAMES],
    bands: usize,
    workers: usize,
) {
    let consistent = frames
        .iter()
        .zip(total)
        .all(|(frame, total)| frame.prepare_ns.saturating_add(frame.waited_ns()) <= *total);
    logger::console(format_args!(
        "{{\"event\":\"retail_draw_stage_timings\",\"scope\":\"diagnostic_dispatch_wall_times\",\"diagnostic_instrumentation\":true,\"timing_qualified\":false,\"cpu_time_measured\":false,\"bands\":{bands},\"workers\":{workers},\"warmup\":{WARMUP},\"frames\":{FRAMES},\"first_raster_dispatch_role\":\"opaque for this single-view retail world packet\",\"waited_scope\":\"dispatch, completion barrier and allocation count collection\",\"residual_semantics\":\"direct total minus view preparation and waited raster dispatches; other wall cost including instrumentation overhead\",\"nested_times_within_total\":{consistent},\"summaries\":{{"
    ));
    timing_summary(
        "view_prepare",
        "nanoseconds",
        std::array::from_fn(|index| frames[index].prepare_ns),
    );
    logger::console(format_args!(","));
    timing_summary(
        "preparation_dispatches",
        "count",
        std::array::from_fn(|index| frames[index].prepare_dispatches),
    );
    logger::console(format_args!(","));
    timing_summary(
        "preparation_jobs",
        "count",
        std::array::from_fn(|index| frames[index].prepare_jobs),
    );
    logger::console(format_args!(","));
    timing_summary(
        "first_dispatch",
        "nanoseconds",
        std::array::from_fn(|index| frames[index].first_ns),
    );
    logger::console(format_args!(","));
    timing_summary(
        "subsequent_dispatches",
        "nanoseconds",
        std::array::from_fn(|index| frames[index].subsequent_ns),
    );
    logger::console(format_args!(","));
    timing_summary(
        "dispatches",
        "count",
        std::array::from_fn(|index| frames[index].count),
    );
    logger::console(format_args!(","));
    timing_summary(
        "maximum_dispatch",
        "nanoseconds",
        std::array::from_fn(|index| frames[index].maximum_ns),
    );
    logger::console(format_args!(","));
    timing_summary(
        "other_wall",
        "nanoseconds",
        std::array::from_fn(|index| {
            total[index].saturating_sub(
                frames[index]
                    .prepare_ns
                    .saturating_add(frames[index].waited_ns()),
            )
        }),
    );
    logger::console(format_args!(
        "}},\"sample_columns\":[\"view_prepare_ns\",\"preparation_dispatch_count\",\"preparation_job_count\",\"first_dispatch_ns\",\"subsequent_dispatches_ns\",\"dispatch_count\",\"maximum_dispatch_ns\",\"other_wall_ns\"],\"samples\":["
    ));
    for (index, frame) in frames.iter().enumerate() {
        if index != 0 {
            logger::console(format_args!(","));
        }
        logger::console(format_args!(
            "[{},{},{},{},{},{},{},{}]",
            frame.prepare_ns,
            frame.prepare_dispatches,
            frame.prepare_jobs,
            frame.first_ns,
            frame.subsequent_ns,
            frame.count,
            frame.maximum_ns,
            total[index].saturating_sub(frame.prepare_ns.saturating_add(frame.waited_ns())),
        ));
    }
    logger::console(format_args!("]}}\n"));
}

#[cfg(any(debug_assertions, feature = "allocation-tracking"))]
fn run() -> Result<(), String> {
    use qa_platform::allocations::{begin_frame, end_frame};
    let options = Options::read()?;
    private_environment()?;
    let (home, profile_root) = copied_profile()?;
    let content = options
        .content
        .canonicalize()
        .map_err(|e| format!("content directory: {e}"))?;
    let output = output_path(&options.output, &content, &profile_root)?;
    let depth_output = options
        .depth_output
        .as_ref()
        .map(|path| output_path(path, &content, &profile_root))
        .transpose()?;
    if depth_output.as_ref() == Some(&output) {
        return Err("pixel and inverse-depth evidence paths must differ".into());
    }
    let mut vfs = qa_content::vfs::Vfs::default();
    vfs.mount_product(&content, 0)
        .map_err(|e| format!("content mount: {e:?}"))?;
    let input = map::read(&vfs, &options.map)?;
    let names = input.catalog_names()?;
    let mut runtime = Runtime::load(64, names.iter().map(|name| name.as_ref()))?;
    drop(names);
    runtime.vfs = vfs;
    let source = input.native_source;
    let policy = qa_app::client_policy::ClientPolicy::select(None, input.client_rules, None, None)?;
    let rules = policy.movement;
    let mut console = Console::<Runtime>::new(Context {
        source,
        ..Context::default()
    })
    .map_err(|e| e.to_string())?;
    let profile_product = input.profile_product(policy.client).into_owned();
    let imported = profile::load(
        &mut console,
        &mut runtime,
        &profile_product,
        policy.client,
        rules,
    )?;
    if !imported.consumed() {
        return Err("copied saved profile contained no applicable settings".into());
    }
    let image_settings = render_settings::image_settings(&console.cvars, source)?;
    let mut assets = Assets::load().map_err(|e| e.to_string())?;
    let loaded = input.load(
        &runtime.vfs,
        &mut assets,
        &mut runtime.geometry,
        WorldLoadOptions {
            image_settings: Some(image_settings),
            ..WorldLoadOptions::default()
        },
    )?;
    for diagnostic in &loaded.render.diagnostics {
        logger::console(format_args!(
            "world {}: {diagnostic}\n",
            loaded.render.world.0
        ));
    }
    runtime.entity_sources.push(loaded.entity_source);
    let client = runtime.connect_local(
        SeatId::FIRST,
        loaded.spawns[0],
        policy,
        qa_network::commands::packet::Protocol::QuakeWorld28,
    )?;
    let player = &runtime.server.clients[client.0 as usize].player;
    let basis = angle_vectors(player.view_angles);
    let fov = console.cvars.find("cg_fov").ok_or("missing cg_fov")?;
    let fov_x = console.cvars.value(fov);
    let fov_x = if fov_x.is_finite() {
        fov_x.clamp(1.0, 179.0)
    } else {
        90.0
    };
    let aspect = HEIGHT as f32 / WIDTH as f32;
    let fov_y = (2.0 * ((fov_x.to_radians() * 0.5).tan() * aspect).atan()).to_degrees();
    let mut refdef = Refdef {
        viewport: Viewport {
            x: 0,
            y: 0,
            width: WIDTH,
            height: HEIGHT,
        },
        origin: player.body.position + player.view_offset,
        axes: [basis.forward, -basis.right, basis.up],
        fov: [fov_x, fov_y],
        time_ms: 0,
        ..Refdef::default()
    };
    loaded.render.presentation.apply(&mut refdef);
    let world = assets
        .world(loaded.render.world)
        .ok_or("missing loaded world")?;
    let mut view = WorldView::load(world);
    let surfaces = view
        .query(world, refdef, None, &[])
        .map_err(|e| format!("world visibility: {e:?}"))?;
    if surfaces.is_empty() {
        return Err("native spawn camera has no visible world surfaces".into());
    }
    let mut limits = Limits::default();
    limits.surfaces = limits.surfaces.max(world.visibility().surface_count());
    let mut front = FrontEnd::load(limits)?;
    let mut frame = front
        .begin_frame([18, 26, 34, 255])
        .ok_or("scene packet unavailable")?;
    frame.clear_scene();
    if !frame.add_world(loaded.render.world, surfaces) || !frame.render_scene(refdef, &[], &assets)
    {
        return Err("fixed retail scene capacity".into());
    }
    let packet = frame.finish();
    let mut window = Window::open(WIDTH as i32, HEIGHT as i32)?;
    if window.video_driver() != "x11" {
        return Err("private benchmark did not select X11".into());
    }
    let (_, selected_bands) = cpu_band_setting(&console.cvars, options.bands)?;
    let mut cpu = CpuBackend::load_with_limits(
        WIDTH,
        HEIGHT,
        &assets,
        CpuLimits {
            scene: limits,
            bands: selected_bands
                .unwrap_or_else(|| RasterBands::at_most(qa_platform::physical_core_count())),
            auto_bands: selected_bands.is_none(),
            ..CpuLimits::default()
        },
    )?;
    let config = cpu.raster_config();
    let mut dispatch =
        CpuDispatch::load(config.bands).map_err(|e| format!("retail CPU workers: {e}"))?;
    report_cpu_config(config, dispatch.worker_count());
    let mut initial_stages = DispatchTimings::default();
    let initial = draw(
        &mut cpu,
        &mut dispatch,
        &packet,
        &assets,
        options.stage_timings.then_some(&mut initial_stages),
    );
    let _ = dispatch.merge_counts(qa_platform::allocations::Counts::default());
    let initial = initial.map_err(|e| format!("initial retail CPU dispatch: {e}"))?;
    if initial.rejected != 0 {
        let mut diagnosis = FrontEnd::load(limits)?;
        for surface in surfaces {
            let mut isolated = diagnosis
                .begin_frame([18, 26, 34, 255])
                .ok_or("diagnostic scene packet unavailable")?;
            isolated.clear_scene();
            if !isolated.add_world(loaded.render.world, std::slice::from_ref(surface))
                || !isolated.render_scene(refdef, &[], &assets)
            {
                return Err("diagnostic scene capacity".into());
            }
            let packet = isolated.finish();
            let result = cpu.render(&packet, &assets);
            if result.rejected != 0 {
                logger::console(format_args!(
                    "{{\"event\":\"retail_surface_rejected\",\"surface\":{},\"rejected\":{},\"material\":{}}}\n",
                    surface.surface,
                    result.rejected,
                    world.bindings()[surface.surface as usize].material.0,
                ));
            }
            diagnosis
                .recycle(packet)
                .map_err(|_| "diagnostic packet ownership")?;
        }
        return Err(format!(
            "initial retail draw rejected {} submissions",
            initial.rejected
        ));
    }
    if !window.present_pixels(WIDTH, HEIGHT, cpu.pixels()) {
        return Err("initial retail presentation failed".into());
    }
    logger::console(format_args!(
        "{{\"event\":\"window_ready\",\"gameplay\":false,\"renderer\":\"cpu\",\"video_driver\":\"x11\",\"scope\":\"fixed_time_retail_draw\"}}\n"
    ));
    std::io::stdout()
        .flush()
        .map_err(|e| format!("ready report: {e}"))?;
    pause(Duration::from_millis(options.hold_ms));

    let mut samples = [0u64; FRAMES];
    let mut stage_samples = [DispatchTimings::default(); FRAMES];
    let mut allocations = 0u64;
    let mut reallocations = 0u64;
    let mut requested_bytes = 0u64;
    let mut maximum_allocations = 0u64;
    let mut maximum_requested_bytes = 0u64;
    let mut stable_stats = true;
    let mut rejected = 0u64;
    let mut final_stats = initial;
    for _ in 0..WARMUP {
        let mut stages = DispatchTimings::default();
        let timer = Stopwatch::start();
        let result = draw(
            &mut cpu,
            &mut dispatch,
            &packet,
            &assets,
            options.stage_timings.then_some(&mut stages),
        );
        let _ = timer.elapsed();
        let _ = dispatch.merge_counts(qa_platform::allocations::Counts::default());
        final_stats = result.map_err(|e| format!("warmup retail CPU dispatch: {e}"))?;
        rejected += u64::from(final_stats.rejected);
    }
    let expected_stats = final_stats;
    let warm_cache = cpu.world_stats().cache;
    // CPU preparation, all raster dispatches/barriers and the stopwatch are
    // inside this scope. Each sample merges caller and every worker's counts.
    // No scene construction, SDL presentation or I/O occurs inside it.
    for (sample, stages) in samples.iter_mut().zip(&mut stage_samples) {
        begin_frame();
        let timer = Stopwatch::start();
        let result = draw(
            &mut cpu,
            &mut dispatch,
            &packet,
            &assets,
            options.stage_timings.then_some(stages),
        );
        *sample = timer.elapsed().as_nanos() as u64;
        let counts = dispatch.merge_counts(end_frame());
        final_stats = result.map_err(|e| format!("measured retail CPU dispatch: {e}"))?;
        allocations += counts.allocations;
        reallocations += counts.reallocations;
        requested_bytes += counts.requested_bytes;
        maximum_allocations = maximum_allocations.max(counts.allocations + counts.reallocations);
        maximum_requested_bytes = maximum_requested_bytes.max(counts.requested_bytes);
        stable_stats &= final_stats == expected_stats;
        rejected += u64::from(final_stats.rejected);
    }
    let final_world = cpu.world_stats();
    if !window.present_pixels(WIDTH, HEIGHT, cpu.pixels()) {
        return Err("final retail presentation failed".into());
    }
    write_rgba(&output, cpu.pixels()).map_err(|e| format!("raw RGBA evidence: {e}"))?;
    if let Some(path) = &depth_output {
        write_depth(path, cpu.inverse_depth())
            .map_err(|e| format!("inverse-depth evidence: {e}"))?;
    }

    logger::console(format_args!(
        "{{\"event\":\"retail_draw_workload\",\"scope\":\"fixed_time_retail_draw\",\"gameplay_qualified\":false,\"host_loop_qualified\":false,\"width\":{WIDTH},\"height\":{HEIGHT},\"time_ms\":0,\"home\":"
    ));
    json_string(&home.to_string_lossy());
    logger::console(format_args!(",\"profile_root\":"));
    json_string(&profile_root.to_string_lossy());
    logger::console(format_args!(",\"content\":"));
    json_string(&content.to_string_lossy());
    logger::console(format_args!(",\"map\":"));
    json_string(&loaded.virtual_path);
    logger::console(format_args!(",\"profile_product\":"));
    json_string(&profile_product);
    logger::console(format_args!(",\"native_source\":"));
    json_string(match source {
        RuleSetId::Quake => "q1",
        RuleSetId::QuakeWorld => "qw",
        RuleSetId::Quake2 => "q2",
        RuleSetId::Quake2Rerelease => "q2rr",
        RuleSetId::Quake3 => "q3",
    });
    logger::console(format_args!(
        ",\"presentation\":\"{}\",\"origin\":{:?},\"angles\":{:?},\"axes\":[{:?},{:?},{:?}],\"fov\":{:?},\"near\":{},\"far\":{},\"spawn_entity\":{},\"spawn_fixture_fallback\":{},\"profile_consumed\":true,\"applied_cvars\":{},\"applied_bindings\":{},\"unsupported_settings\":{},\"profile_files\":[",
        if matches!(refdef.cpu_presentation, CpuPresentation::Rgb) {
            "rgb"
        } else {
            "indexed"
        },
        refdef.origin.0,
        player.view_angles.0,
        refdef.axes[0].0,
        refdef.axes[1].0,
        refdef.axes[2].0,
        refdef.fov,
        refdef.near,
        refdef.far,
        loaded.spawns[0].entity,
        loaded.spawns[0].fixture_fallback,
        imported.cvars,
        imported.bindings,
        imported.unsupported
    ));
    for (index, path) in imported.files.iter().enumerate() {
        if index != 0 {
            logger::console(format_args!(","));
        }
        json_string(path);
    }
    logger::console(format_args!(
        "],\"commands\":{},\"visible_surfaces\":{},\"world_surfaces\":{},\"world_vertices\":{},\"world_indices\":{},\"images\":{},\"materials\":{},\"models\":{},\"surface_order\":[",
        packet.commands().len(),
        surfaces.len(),
        world.geometry().surfaces.len(),
        world.geometry().vertices.len(),
        world.geometry().indices.len(),
        assets.images().len(),
        assets.materials().len(),
        assets.models().len()
    ));
    for (index, surface) in surfaces.iter().enumerate() {
        if index != 0 {
            logger::console(format_args!(","));
        }
        logger::console(format_args!("[{},{}]", surface.surface, surface.depth_key));
    }
    logger::console(format_args!("]}}\n"));
    let mut sorted = samples;
    sorted.sort_unstable();
    logger::console(format_args!(
        "{{\"event\":\"retail_draw_timings\",\"scope\":\"cpu_prepare_raster_dispatch_and_barriers\",\"diagnostic_instrumentation\":{},\"bands\":{},\"workers\":{},\"warmup\":{WARMUP},\"frames\":{FRAMES},\"debug_build\":{},\"median_ns\":{},\"p99_ns\":{},\"samples_ns\":[",
        options.stage_timings,
        config.bands.count(),
        dispatch.worker_count(),
        cfg!(debug_assertions),
        sorted[299] as f64 * 0.5 + sorted[300] as f64 * 0.5,
        sorted[593]
    ));
    for (index, sample) in samples.iter().enumerate() {
        if index != 0 {
            logger::console(format_args!(","));
        }
        logger::console(format_args!("{sample}"));
    }
    logger::console(format_args!("]}}\n"));
    if options.stage_timings {
        report_stage_timings(
            &stage_samples,
            &samples,
            config.bands.count(),
            dispatch.worker_count(),
        );
    }
    logger::console(format_args!(
        "{{\"event\":\"retail_draw_stats\",\"within_run_backend_stats_match\":{stable_stats},\"backend\":"
    ));
    backend_stats(final_stats);
    logger::console(format_args!(",\"world\":"));
    world_stats(final_world);
    logger::console(format_args!(
        ",\"measured_cache_delta\":{{\"hits\":{},\"fills\":{},\"evictions\":{},\"rejected\":{}}}}}\n",
        final_world.cache.hits.saturating_sub(warm_cache.hits),
        final_world.cache.fills.saturating_sub(warm_cache.fills),
        final_world
            .cache
            .evictions
            .saturating_sub(warm_cache.evictions),
        final_world
            .cache
            .rejected
            .saturating_sub(warm_cache.rejected)
    ));
    let mut bands = [WorldStats::default(); MAX_BANDS];
    let band_count = cpu.band_stats(&mut bands);
    logger::console(format_args!(
        "{{\"event\":\"retail_draw_bands\",\"bands\":["
    ));
    for (index, band) in bands[..band_count].iter().enumerate() {
        if index != 0 {
            logger::console(format_args!(","));
        }
        logger::console(format_args!(
            "{{\"band\":{index},\"cache_capacity_bytes\":{},\"world\":",
            config.per_band_cache_bytes
        ));
        world_stats(*band);
        logger::console(format_args!("}}"));
    }
    logger::console(format_args!("]}}\n"));
    logger::console(format_args!(
        "{{\"event\":\"allocation_gate\",\"scope\":\"cpu_draw_all_instrumented_rust_threads\",\"calling_threads\":1,\"worker_threads\":{},\"native_heap_measured\":false,\"presentation_measured\":false,\"frames\":{FRAMES},\"allocations\":{allocations},\"reallocations\":{reallocations},\"requested_bytes\":{requested_bytes},\"maximum_allocations\":{maximum_allocations},\"maximum_requested_bytes\":{maximum_requested_bytes},\"passed\":{}}}\n",
        dispatch.worker_count(),
        allocations == 0 && reallocations == 0 && requested_bytes == 0
    ));
    logger::console(format_args!(
        "{{\"event\":\"retail_draw_pixels\",\"format\":\"RGBA8\",\"row_order\":\"top_to_bottom\",\"width\":{WIDTH},\"height\":{HEIGHT},\"bytes\":{},\"depth_output_available\":{},\"path\":",
        cpu.pixels().len() * 4,
        depth_output.is_some()
    ));
    json_string(&output.to_string_lossy());
    logger::console(format_args!("}}\n"));
    if let Some(path) = &depth_output {
        logger::console(format_args!(
            "{{\"event\":\"retail_draw_depth\",\"format\":\"f32_le_inverse_depth_scores\",\"includes_depth_range_adjustment\":true,\"row_order\":\"top_to_bottom\",\"width\":{WIDTH},\"height\":{HEIGHT},\"bytes\":{},\"path\":",
            cpu.inverse_depth().len() * 4
        ));
        json_string(&path.to_string_lossy());
        logger::console(format_args!("}}\n"));
    }
    std::io::stdout()
        .flush()
        .map_err(|e| format!("final capture report: {e}"))?;
    pause(Duration::from_millis(options.hold_ms));
    if front.recycle(packet).is_err() {
        return Err("fixed scene recycle failed".into());
    }
    if allocations != 0
        || reallocations != 0
        || requested_bytes != 0
        || !stable_stats
        || rejected != 0
        || final_world.rejected != 0
    {
        return Err("retail draw allocation/stability/rejection gate failed".into());
    }
    logger::console(format_args!(
        "{{\"event\":\"normal_exit\",\"frames\":{FRAMES},\"warmup\":{WARMUP},\"gameplay\":false,\"scope\":\"fixed_time_retail_draw\"}}\n"
    ));
    Ok(())
}

#[cfg(any(debug_assertions, feature = "allocation-tracking"))]
fn main() {
    if let Err(message) = run() {
        logger::error(&message);
        std::process::exit(1);
    }
}

#[cfg(not(any(debug_assertions, feature = "allocation-tracking")))]
fn main() {
    logger::error("build this developer benchmark with allocation-tracking");
    std::process::exit(1);
}
