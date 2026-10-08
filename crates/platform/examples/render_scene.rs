//! Private display fixture for THE-861 commands, not retail-map gameplay proof.
use qa_core::{
    primitives::Vec3,
    sys_events::{EventKind, SysEventQueue},
};
use qa_platform::{EventPump, Stopwatch, Window, pause};
use qa_render::{
    Assets, BackendStats, BlendPhase, Command, CommandList, Draw2d, FrontEnd, Light, Limits,
    MaterialId, ModelId, Refdef, SceneEntity, Vertex, Viewport,
    assets::{
        Cull, DepthFunc, MaterialSettings, Stage, StageTexture, TcGen,
        upload::{MipmapBuild, UploadParams},
    },
    cpu::CpuBackend,
    gl::GlBackend,
    shader::{BlendFactor, StageBlend},
};
use std::{io::Write, path::PathBuf, time::Duration};

const WIDTH: u32 = 640;
const HEIGHT: u32 = 400;
const WARMUP: usize = 60;
const FRAMES: usize = 600;
const CLEAR: [u8; 4] = [16, 24, 32, 255];

#[cfg(any(debug_assertions, feature = "allocation-tracking"))]
#[global_allocator]
static ALLOCATOR: qa_platform::allocations::CountingAllocator =
    qa_platform::allocations::CountingAllocator;

struct Fixture {
    model: ModelId,
    poly_material: MaterialId,
    poly: [Vertex; 3],
}
impl Fixture {
    fn load(assets: &mut Assets) -> Result<Self, &'static str> {
        let base = assets.register_image(2, 2, &[192, 128, 64, 255].repeat(4))?;
        assets.prepare_image(
            base,
            UploadParams {
                mipmaps: MipmapBuild::LegacyBox,
                ..UploadParams::default()
            },
        )?;
        let lightmap = assets.register_image(1, 1, &[128, 128, 128, 255])?;
        assets.prepare_image(lightmap, UploadParams::default())?;
        let material = assets.register_material(
            "fixture/base-times-lightmap",
            &[
                Stage {
                    texture: StageTexture::Image(base),
                    ..Stage::default()
                },
                Stage {
                    texture: StageTexture::Image(lightmap),
                    blend: Some(StageBlend {
                        source: BlendFactor::DestinationColor,
                        destination: BlendFactor::Zero,
                    }),
                    texgen: TcGen::Lightmap,
                    depth_func: DepthFunc::Equal,
                    depth_write: false,
                    ..Stage::default()
                },
            ],
            MaterialSettings {
                cull: Cull::None,
                sort: 0.0,
                ..MaterialSettings::default()
            },
        )?;
        let positions = [
            [16.0, 12.0, 12.0],
            [16.0, -12.0, 12.0],
            [16.0, -12.0, -12.0],
            [16.0, 12.0, -12.0],
        ];
        let coords = [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]];
        let mesh: [Vertex; 4] = std::array::from_fn(|i| Vertex {
            position: Vec3(positions[i]),
            texcoord: coords[i],
            lightmap_coord: coords[i],
            ..Vertex::default()
        });
        let model = assets.register_model(&mesh, &[0, 1, 2, 0, 2, 3], material)?;
        let image = assets.register_image(2, 2, &[32, 160, 224, 255].repeat(4))?;
        assets.prepare_image(
            image,
            UploadParams {
                mipmaps: MipmapBuild::Box,
                ..UploadParams::default()
            },
        )?;
        let poly_material = assets.register_material(
            "fixture/near-clipped-texture",
            &[Stage {
                texture: StageTexture::Image(image),
                ..Stage::default()
            }],
            MaterialSettings {
                cull: Cull::None,
                sort: 0.0,
                ..MaterialSettings::default()
            },
        )?;
        let positions = [[2.0, -0.5, 1.0], [8.0, -5.0, 5.0], [8.0, -5.0, 1.0]];
        let coords = [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0]];
        let poly = std::array::from_fn(|i| Vertex {
            position: Vec3(positions[i]),
            texcoord: coords[i],
            ..Vertex::default()
        });
        Ok(Self {
            model,
            poly_material,
            poly,
        })
    }

    fn packet(&self, front: &mut FrontEnd, assets: &Assets) -> Result<CommandList, &'static str> {
        let mut frame = front.begin_frame(CLEAR).ok_or("scene packet unavailable")?;
        for seat in 0..2 {
            frame.clear_scene();
            if !frame.add_entity(SceneEntity {
                model: self.model,
                ..SceneEntity::default()
            }) || !frame.add_poly(self.poly_material, &self.poly)
                || !frame.add_light(Light {
                    origin: Vec3([8.0, 0.0, 8.0]),
                    radius: 16.0,
                    color: [1.0, 0.5, 0.25],
                    additive: false,
                })
                || !frame.render_scene(
                    Refdef {
                        viewport: Viewport {
                            x: seat * 320,
                            y: 0,
                            width: 320,
                            height: HEIGHT,
                        },
                        fov: [90.0, 90.0],
                        near: 4.0,
                        far: 256.0,
                        blend: [0.0, 0.0, 1.0, 0.25],
                        blend_phase: if seat == 0 {
                            BlendPhase::AfterView
                        } else {
                            BlendPhase::FinalPalette
                        },
                        ..Refdef::default()
                    },
                    &[],
                    assets,
                )
            {
                return Err("fixture scene capacity");
            }
        }
        for seat in 0..2 {
            for (x, width, color) in [
                (24.0, 80.0, [220, 20, 40, 255]),
                (48.0, 24.0, [20, 220, 40, 255]),
            ] {
                if !frame.draw_2d(Draw2d {
                    rect: [x + seat as f32 * 320.0, 360.0, width, 20.0],
                    texcoords: [0.0, 0.0, 1.0, 1.0],
                    color,
                    material: MaterialId(0),
                }) {
                    return Err("fixture 2D capacity");
                }
            }
        }
        Ok(frame.finish())
    }
}

enum Backend {
    Cpu(CpuBackend),
    Gl(GlBackend),
}
impl Backend {
    fn render(&mut self, packet: &CommandList, assets: &Assets) -> BackendStats {
        match self {
            Self::Cpu(cpu) => cpu.render(packet, assets),
            Self::Gl(gl) => gl.render(packet, assets),
        }
    }
    fn present(&self, window: &mut Window) -> bool {
        match self {
            Self::Cpu(cpu) => window.present_pixels(WIDTH, HEIGHT, cpu.pixels()),
            Self::Gl(_) => window.present_gl(),
        }
    }
    fn readback(&self, out: &mut [u8]) -> bool {
        match self {
            Self::Cpu(cpu) => {
                for (pixel, bytes) in cpu.pixels().iter().zip(out.chunks_exact_mut(4)) {
                    bytes.copy_from_slice(&pixel.to_le_bytes());
                }
                true
            }
            Self::Gl(gl) => {
                if !gl.read_pixels(out) {
                    return false;
                }
                // GL back-buffer reads have their origin at the lower left.
                let stride = WIDTH as usize * 4;
                for y in 0..HEIGHT as usize / 2 {
                    let opposite = HEIGHT as usize - 1 - y;
                    let (first, rest) = out.split_at_mut(opposite * stride);
                    first[y * stride..(y + 1) * stride].swap_with_slice(&mut rest[..stride]);
                }
                true
            }
        }
    }
}

fn json_string(value: &str) {
    print!("\"");
    for c in value.chars() {
        match c {
            '"' => print!("\\\""),
            '\\' => print!("\\\\"),
            c if c.is_control() => print!("\\u{:04x}", c as u32),
            c => print!("{c}"),
        }
    }
    print!("\"");
}

fn stats_json(stats: BackendStats) {
    print!(
        "{{\"views\":{},\"triangles\":{},\"draws_2d\":{},\"rejected\":{},\"pending_lights\":{}}}",
        stats.views, stats.triangles, stats.draws_2d, stats.rejected, stats.pending_lights
    );
}

fn write_ppm(path: &PathBuf, rgba: &[u8]) -> std::io::Result<()> {
    let mut file = std::io::BufWriter::new(std::fs::File::create(path)?);
    write!(file, "P6\n{WIDTH} {HEIGHT}\n255\n")?;
    for pixel in rgba.chunks_exact(4) {
        file.write_all(&pixel[..3])?;
    }
    file.flush()
}

#[cfg(any(debug_assertions, feature = "allocation-tracking"))]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use qa_platform::allocations::{begin_frame, end_frame};
    let mut renderer = "cpu".to_owned();
    let mut output = None;
    let mut hold_ms = 1500;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--renderer" => renderer = args.next().ok_or("missing renderer")?,
            "--output" => output = Some(PathBuf::from(args.next().ok_or("missing output")?)),
            "--startup-hold-ms" => {
                hold_ms = args.next().ok_or("missing hold duration")?.parse::<u64>()?
            }
            _ => return Err("unknown fixture option".into()),
        }
    }
    if !matches!(renderer.as_str(), "cpu" | "gl") || hold_ms > 10_000 {
        return Err("invalid fixture options".into());
    }
    let mut assets = Assets::load();
    let fixture = Fixture::load(&mut assets)?;
    let mut front = FrontEnd::load(Limits::default())?;
    let mut window = if renderer == "gl" {
        Window::open_gl(WIDTH as i32, HEIGHT as i32)?
    } else {
        Window::open(WIDTH as i32, HEIGHT as i32)?
    };
    // Window outlives the GL backend. Only this main thread uses its context.
    let mut backend = if renderer == "gl" {
        Backend::Gl(unsafe {
            GlBackend::load(|name| window.gl_proc(name), &assets, WIDTH, HEIGHT, 65536)?
        })
    } else {
        Backend::Cpu(CpuBackend::load(WIDTH, HEIGHT)?)
    };
    let mut reference = CpuBackend::load(WIDTH, HEIGHT)?;
    let mut readback = vec![0u8; WIDTH as usize * HEIGHT as usize * 4];
    let mut samples = Vec::with_capacity(FRAMES);
    let mut stage_samples = [
        Vec::with_capacity(FRAMES),
        Vec::with_capacity(FRAMES),
        Vec::with_capacity(FRAMES),
        Vec::with_capacity(FRAMES),
    ];
    let mut pump = EventPump::new();
    let mut events = SysEventQueue::load(256, 8192).map_err(|_| "event queue capacity")?;
    let initial = fixture.packet(&mut front, &assets)?;
    backend.render(&initial, &assets);
    if !backend.present(&mut window) {
        return Err("initial fixture presentation failed".into());
    }
    if front.recycle(initial).is_err() {
        return Err("initial scene recycle failed".into());
    }
    print!("{{\"event\":\"window_ready\",\"gameplay\":false,\"renderer\":");
    json_string(&renderer);
    print!(",\"video_driver\":");
    json_string(window.video_driver());
    print!(",\"renderer_info\":");
    match &backend {
        Backend::Cpu(_) => print!("{{\"device\":\"shared CPU scene raster\",\"version\":null}}"),
        Backend::Gl(gl) => {
            let (device, version) = gl.renderer_info();
            print!("{{\"device\":");
            json_string(device);
            print!(",\"version\":");
            json_string(version);
            print!("}}");
        }
    }
    println!("}}");
    std::io::stdout().flush()?;
    pause(Duration::from_millis(hold_ms));
    let mut maximum_allocations = 0;
    let mut maximum_requested_bytes = 0;
    let mut allocations = 0u64;
    let mut requested_bytes = 0u64;
    let mut total_stats = BackendStats::default();
    let mut stable_stats = None;
    let mut stats_match = true;
    for index in 0..WARMUP + FRAMES {
        begin_frame();
        let total = Stopwatch::start();
        let result = (|| -> Result<([u64; 4], BackendStats), &'static str> {
            pump.poll_events(&mut window, &mut events);
            while let Some(event) = events.pop() {
                if matches!(event.kind, EventKind::Quit) {
                    return Err("private fixture window closed");
                }
            }
            let timer = Stopwatch::start();
            let packet = fixture.packet(&mut front, &assets)?;
            let frontend_ns = timer.elapsed().as_nanos() as u64;
            let timer = Stopwatch::start();
            let stats = backend.render(&packet, &assets);
            let backend_ns = timer.elapsed().as_nanos() as u64;
            let timer = Stopwatch::start();
            let presented = backend.present(&mut window);
            let present_ns = timer.elapsed().as_nanos() as u64;
            let recycled = front.recycle(packet).is_ok();
            if !presented || !recycled {
                return Err("frame presentation or recycle failed");
            }
            Ok((
                [
                    frontend_ns,
                    backend_ns,
                    present_ns,
                    total.elapsed().as_nanos() as u64,
                ],
                stats,
            ))
        })();
        let counts = end_frame();
        let (times, stats) = result?;
        if index >= WARMUP {
            samples.push(times);
            if let Some(expected) = stable_stats {
                stats_match &= stats == expected;
            } else {
                stable_stats = Some(stats);
            }
            total_stats.views += stats.views;
            total_stats.triangles += stats.triangles;
            total_stats.draws_2d += stats.draws_2d;
            total_stats.rejected += stats.rejected;
            total_stats.pending_lights += stats.pending_lights;
            let count = counts.allocations + counts.reallocations;
            maximum_allocations = maximum_allocations.max(count);
            maximum_requested_bytes = maximum_requested_bytes.max(counts.requested_bytes);
            allocations += count;
            requested_bytes += counts.requested_bytes;
        }
    }
    // Readback and reference comparison use a final packet outside measured frames.
    let packet = fixture.packet(&mut front, &assets)?;
    let command_count = packet.commands().len();
    let (mut entity_count, mut poly_count, mut vertex_count, mut light_count) = (0, 0, 0, 0);
    for command in packet.commands() {
        if let Command::View(view) = command {
            entity_count += view.scene.entities.count;
            poly_count += view.scene.polys.count;
            light_count += view.scene.lights.count;
            for poly in packet.polys(view.scene.polys) {
                vertex_count += poly.vertices.count;
            }
        }
    }
    let final_stats = backend.render(&packet, &assets);
    reference.render(&packet, &assets);
    if !backend.readback(&mut readback) {
        return Err("fixture readback failed".into());
    }
    let mut maximum_rgb_difference = 0u8;
    let mut different_pixels = 0usize;
    for (actual, expected) in readback.chunks_exact(4).zip(reference.pixels()) {
        let expected = expected.to_le_bytes();
        let difference = actual[0]
            .abs_diff(expected[0])
            .max(actual[1].abs_diff(expected[1]))
            .max(actual[2].abs_diff(expected[2]));
        maximum_rgb_difference = maximum_rgb_difference.max(difference);
        different_pixels += usize::from(difference > 2);
    }
    if !backend.present(&mut window) || front.recycle(packet).is_err() {
        return Err("final fixture presentation or recycle failed".into());
    }
    if let Some(path) = output {
        write_ppm(&path, &readback)?;
    }
    for sample in &samples {
        for (column, value) in stage_samples.iter_mut().zip(sample) {
            column.push(*value);
        }
    }
    for column in &mut stage_samples {
        column.sort_unstable();
    }
    print!(
        "{{\"event\":\"scene_timings\",\"scope\":\"shared_scene_fixture\",\"gameplay_qualified\":false,\"warmup\":{WARMUP},\"frames\":{FRAMES},\"vsync\":false,\"debugger\":false,\"samples_ns\":["
    );
    for (index, sample) in samples.iter().enumerate() {
        if index != 0 {
            print!(",");
        }
        print!("[{},{},{},{}]", sample[0], sample[1], sample[2], sample[3]);
    }
    print!("],\"stages\":{{");
    for (index, name) in ["frontend", "backend", "present", "total"]
        .iter()
        .enumerate()
    {
        if index != 0 {
            print!(",");
        }
        let column = &stage_samples[index];
        print!(
            "\"{name}\":{{\"median_ns\":{},\"p99_ns\":{}}}",
            column[299] as f64 * 0.5 + column[300] as f64 * 0.5,
            column[593]
        );
    }
    println!("}}}}");
    println!(
        "{{\"event\":\"allocation_gate\",\"frames\":{FRAMES},\"passed\":{},\"allocations\":{allocations},\"requested_bytes\":{requested_bytes},\"maximum_allocations\":{maximum_allocations},\"maximum_requested_bytes\":{maximum_requested_bytes},\"scope\":\"calling Rust thread; excludes SDL and driver allocations\"}}",
        allocations == 0 && requested_bytes == 0
    );
    print!(
        "{{\"event\":\"scene_fidelity\",\"scope\":\"fixed shared command fixture; no retail map or native palette proof\",\"commands\":{command_count},\"entities\":{entity_count},\"polys\":{poly_count},\"vertices\":{vertex_count},\"lights\":{light_count},\"stats_match\":{stats_match},\"frame_stats\":"
    );
    stats_json(final_stats);
    print!(",\"measured_stats\":");
    stats_json(total_stats);
    print!(
        ",\"reference_rgb\":{{\"tolerance\":2,\"maximum_difference\":{maximum_rgb_difference},\"pixels_over_tolerance\":{different_pixels},\"pixels\":{}}},\"probes\":[",
        WIDTH as usize * HEIGHT as usize
    );
    for (index, (name, x, y)) in [
        ("left_multiply", 160, 200),
        ("right_multiply", 480, 200),
        ("left_clipped", 225, 120),
        ("right_clipped", 545, 120),
        ("left_textured_poly", 250, 125),
        ("right_textured_poly", 570, 125),
        ("left_hud_red", 32, 370),
        ("right_hud_red", 352, 370),
        ("left_hud_overlap", 56, 370),
        ("right_hud_overlap", 376, 370),
        ("left_clear", 16, 16),
        ("right_clear", 336, 16),
    ]
    .iter()
    .enumerate()
    {
        if index != 0 {
            print!(",");
        }
        let offset = (*y as usize * WIDTH as usize + *x as usize) * 4;
        let color = &readback[offset..offset + 4];
        print!(
            "{{\"name\":\"{name}\",\"xy\":[{x},{y}],\"rgba\":[{},{},{},{}]}}",
            color[0], color[1], color[2], color[3]
        );
    }
    println!("]}}");
    if allocations != 0 || requested_bytes != 0 || !stats_match || total_stats.rejected != 0 {
        return Err("fixture allocation or command gate failed".into());
    }
    println!(
        "{{\"event\":\"normal_exit\",\"frames\":{FRAMES},\"warmup\":{WARMUP},\"gameplay\":false}}"
    );
    Ok(())
}

#[cfg(not(any(debug_assertions, feature = "allocation-tracking")))]
fn main() -> Result<(), &'static str> {
    Err("build this developer fixture with allocation-tracking")
}
