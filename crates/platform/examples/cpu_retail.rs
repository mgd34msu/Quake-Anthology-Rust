//! Fixed-time retail CPU draw benchmark. Run only through the private harness.
//! This developer example neither drives input nor qualifies gameplay/installation.
use qa_app::{Runtime, map, profile, render_settings};
use qa_console::{
    commands::Console,
    logger,
    views::{Context, Source},
};
use qa_core::{math::angle_vectors, sys_events::SeatId};
use qa_platform::{Stopwatch, Window, pause};
use qa_render::{
    Assets, BackendStats, CpuPresentation, FrontEnd, Limits, Refdef, Viewport,
    cpu::{CpuBackend, WorldStats},
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
    hold_ms: u64,
}
impl Options {
    fn read() -> Result<Self, String> {
        let (mut content, mut map, mut output) = (None, None, None);
        let mut hold_ms = 1500;
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
        if !content.is_absolute() || !output.is_absolute() {
            return Err("content and output paths must be absolute".into());
        }
        Ok(Self {
            content,
            map,
            output,
            hold_ms,
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
    // Fields shared with e9e6211d. Stage/cache categories may change between
    // kernels; the immutable packet and raw pixels define comparison fidelity.
    logger::console(format_args!(
        "{{\"polygons\":{},\"patch_polygons\":{},\"spans\":{},\"pixels\":{},\"sky_spans\":{},\"sky_pixels\":{},\"stage_spans\":{},\"stage_pixels\":{},\"indexed_spans\":{},\"indexed_pixels\":{},\"rejected\":{},\"cache_cumulative\":{{\"hits\":{},\"fills\":{},\"evictions\":{},\"rejected\":{}}}}}",
        stats.polygons,
        stats.patch_polygons,
        stats.spans,
        stats.pixels,
        stats.sky_spans,
        stats.sky_pixels,
        stats.stage_spans,
        stats.stage_pixels,
        stats.indexed_spans,
        stats.indexed_pixels,
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
    let mut runtime = Runtime::load()?;
    runtime
        .vfs
        .mount_product(&content, 0)
        .map_err(|e| format!("content mount: {e:?}"))?;
    let input = map::read(&runtime.vfs, &options.map)?;
    let source = input.native_source;
    let rules = map::native_movement(source);
    let mut console = Console::<Runtime>::new(Context {
        source,
        ..Context::default()
    });
    let imported = profile::load(&mut console, &mut runtime, &input.profile_product, rules)?;
    if !imported.consumed() {
        return Err("copied saved profile contained no applicable settings".into());
    }
    let image_settings = render_settings::image_settings(&console.cvars, source)?;
    let mut assets = Assets::load();
    let loaded = input.load(
        &runtime.vfs,
        &mut assets,
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
    let client = runtime.connect_local(SeatId::FIRST, loaded.spawn, rules)?;
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
    let mut cpu = CpuBackend::load_with_assets(WIDTH, HEIGHT, &assets)?;
    let initial = cpu.render(&packet, &assets);
    if initial.rejected != 0 || !window.present_pixels(WIDTH, HEIGHT, cpu.pixels()) {
        return Err("initial retail draw or presentation failed".into());
    }
    logger::console(format_args!(
        "{{\"event\":\"window_ready\",\"gameplay\":false,\"renderer\":\"cpu\",\"video_driver\":\"x11\",\"scope\":\"fixed_time_retail_draw\"}}\n"
    ));
    std::io::stdout()
        .flush()
        .map_err(|e| format!("ready report: {e}"))?;
    pause(Duration::from_millis(options.hold_ms));

    let mut samples = [0u64; FRAMES];
    let mut allocations = 0u64;
    let mut reallocations = 0u64;
    let mut requested_bytes = 0u64;
    let mut maximum_allocations = 0u64;
    let mut maximum_requested_bytes = 0u64;
    let mut stable_stats = true;
    let mut rejected = 0u64;
    let mut final_stats = initial;
    for _ in 0..WARMUP {
        let timer = Stopwatch::start();
        final_stats = cpu.render(&packet, &assets);
        let _ = timer.elapsed();
        rejected += u64::from(final_stats.rejected);
    }
    let expected_stats = final_stats;
    let warm_cache = cpu.world_stats().cache;
    // Only the direct CPU draw and platform stopwatch are inside this measured
    // allocation scope. No scene construction, SDL presentation or I/O occurs.
    for sample in &mut samples {
        begin_frame();
        let timer = Stopwatch::start();
        final_stats = cpu.render(&packet, &assets);
        *sample = timer.elapsed().as_nanos() as u64;
        let counts = end_frame();
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
    json_string(&loaded.profile_product);
    logger::console(format_args!(",\"native_source\":"));
    json_string(match source {
        Source::Quake => "q1",
        Source::QuakeWorld => "qw",
        Source::Quake2 => "q2",
        Source::Quake2Rerelease => "q2rr",
        Source::Quake3 => "q3",
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
        loaded.spawn.entity,
        loaded.spawn.fixture_fallback,
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
        "{{\"event\":\"retail_draw_timings\",\"scope\":\"direct_cpu_backend_only\",\"warmup\":{WARMUP},\"frames\":{FRAMES},\"debug_build\":{},\"median_ns\":{},\"p99_ns\":{},\"samples_ns\":[",
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
    logger::console(format_args!(
        "{{\"event\":\"allocation_gate\",\"scope\":\"direct CPU draw calling Rust thread; excludes SDL/driver allocations and presentation\",\"frames\":{FRAMES},\"allocations\":{allocations},\"reallocations\":{reallocations},\"requested_bytes\":{requested_bytes},\"maximum_allocations\":{maximum_allocations},\"maximum_requested_bytes\":{maximum_requested_bytes},\"passed\":{}}}\n",
        allocations == 0 && reallocations == 0 && requested_bytes == 0
    ));
    logger::console(format_args!(
        "{{\"event\":\"retail_draw_pixels\",\"format\":\"RGBA8\",\"row_order\":\"top_to_bottom\",\"width\":{WIDTH},\"height\":{HEIGHT},\"bytes\":{},\"depth_output_available\":false,\"path\":",
        cpu.pixels().len() * 4
    ));
    json_string(&output.to_string_lossy());
    logger::console(format_args!("}}\n"));
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
