use qa_console::{
    commands::{Console, Host},
    views::{Context, Source},
};
use qa_content::vfs::Vfs;
use qa_platform::{InputEvent, Window};
use std::time::{Duration, Instant};

#[cfg(any(debug_assertions, feature = "allocation-tracking"))]
#[global_allocator]
static ALLOCATOR: qa_platform::allocations::CountingAllocator =
    qa_platform::allocations::CountingAllocator;

#[cfg(feature = "proof")]
mod proof;

fn run() -> Result<(), String> {
    let mut console = Console::<Runtime>::new(Context::default());
    let mut runtime = Runtime {
        vfs: Vfs::default(),
        quit: false,
    };
    let developer = console
        .cvars
        .find("developer")
        .ok_or("developer cvar missing")?;
    let mut frames = 120u32;
    let mut width = 640i32;
    let mut height = 400i32;
    let mut warmup = 0u32;
    let mut timings = false;
    let mut uncapped = false;
    let mut startup_hold = 0u64;
    #[cfg(feature = "proof")]
    let mut script = None;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "+set" => {
                let cvars = &mut console.cvars;
                let name = args.next().ok_or("+set needs a cvar name")?;
                let value = args.next().ok_or("+set needs a value")?;
                let view = cvars
                    .bind(&name, cvars.context())
                    .ok_or_else(|| format!("unknown cvar: {name}"))?;
                cvars
                    .write(view, &value)
                    .map_err(|error| format!("cvar {name}: {error:?}"))?;
            }
            "--commands" => {
                let text = args.next().ok_or("--commands needs text")?;
                console
                    .append(&(text + "\n"), console.cvars.context())
                    .map_err(|e| format!("{e:?}"))?;
            }
            "--console-source" => {
                let name = args
                    .next()
                    .ok_or("--console-source needs q1/qw/q2/q2rr/q3")?;
                let source = [
                    ("q1", Source::Quake),
                    ("qw", Source::QuakeWorld),
                    ("q2", Source::Quake2),
                    ("q2rr", Source::Quake2Rerelease),
                    ("q3", Source::Quake3),
                ]
                .into_iter()
                .find(|(n, _)| *n == name)
                .map(|(_, s)| s)
                .ok_or("unknown console source")?;
                console.cvars.select_context(Context {
                    source,
                    ..console.cvars.context()
                });
            }
            "--content" => {
                let path = args.next().ok_or("--content needs a product directory")?;
                runtime
                    .vfs
                    .mount_product(std::path::Path::new(&path), 0)
                    .map_err(|e| format!("content: {e:?}"))?;
            }
            #[cfg(feature = "proof")]
            "--proof-script" => {
                script = Some(proof::Script::load(
                    &args.next().ok_or("--proof-script needs a path")?,
                )?)
            }
            "--frame-timings" => timings = true,
            "--uncapped" => uncapped = true,
            "--warmup" => {
                warmup = args
                    .next()
                    .ok_or("--warmup needs a number")?
                    .parse()
                    .map_err(|_| "invalid warm-up count")?
            }
            "--startup-hold-ms" => {
                startup_hold = args
                    .next()
                    .ok_or("--startup-hold-ms needs a number")?
                    .parse()
                    .map_err(|_| "invalid startup hold")?
            }
            "--build-info" => {
                println!(
                    "{{\"commit\":\"{}\",\"source_tree_dirty\":{},\"target_cpu\":\"{}\",\"proof\":{}}}",
                    option_env!("QA_BUILD_COMMIT").unwrap_or("unrecorded"),
                    option_env!("QA_BUILD_DIRTY").unwrap_or("true"),
                    option_env!("QA_TARGET_CPU").unwrap_or("baseline"),
                    cfg!(feature = "proof")
                );
                return Ok(());
            }
            "--version" => {
                println!("qa-rust {}", env!("CARGO_PKG_VERSION"));
                return Ok(());
            }
            "--frames" => {
                frames = args
                    .next()
                    .ok_or("--frames needs a number")?
                    .parse()
                    .map_err(|_| "invalid frame count")?
            }
            "--width" => {
                width = args
                    .next()
                    .ok_or("--width needs a number")?
                    .parse()
                    .map_err(|_| "invalid width")?
            }
            "--height" => {
                height = args
                    .next()
                    .ok_or("--height needs a number")?
                    .parse()
                    .map_err(|_| "invalid height")?
            }
            _ => return Err(format!("unknown option: {arg}")),
        }
    }
    if frames == 0 || width <= 0 || height <= 0 {
        return Err("frames and dimensions must be positive".into());
    }
    let mut window = Window::open(width, height)?;
    window.present();
    println!(
        "{{\"event\":\"window_ready\",\"gameplay\":false,\"video_driver\":\"{}\",\"wayland_display_present\":{}}}",
        window.video_driver(),
        std::env::var_os("WAYLAND_DISPLAY").is_some()
    );
    std::thread::sleep(Duration::from_millis(startup_hold));
    #[cfg(feature = "proof")]
    let script_start = Instant::now();
    let mut completed = 0;
    let mut key_downs = 0u64;
    let mut key_repeats = 0u64;
    let mut samples = if timings {
        Vec::with_capacity(frames as usize)
    } else {
        Vec::new()
    };
    for frame in 0..u64::from(frames) + u64::from(warmup) {
        #[cfg(any(debug_assertions, feature = "allocation-tracking"))]
        console.cvars.reset_lookup_count();
        #[cfg(any(debug_assertions, feature = "allocation-tracking"))]
        qa_platform::allocations::begin_frame();
        let start = Instant::now();
        console.execute_frame(&mut runtime);
        if runtime.quit {
            #[cfg(any(debug_assertions, feature = "allocation-tracking"))]
            let _ = qa_platform::allocations::end_frame();
            break;
        }
        #[cfg(feature = "proof")]
        if let Some(script) = &mut script {
            script.inject_due(script_start.elapsed(), &mut window);
        }
        let quit = window.poll(|event| {
            if let InputEvent::KeyDown { repeat, .. } = event {
                key_downs += 1;
                key_repeats += u64::from(repeat);
            }
            qa_console::logger::dev_print(
                &console.cvars,
                developer,
                1,
                format_args!("{{\"event\":\"input_diagnostic\",\"input\":\"{event:?}\"}}"),
            );
        });
        if quit {
            #[cfg(any(debug_assertions, feature = "allocation-tracking"))]
            let _ = qa_platform::allocations::end_frame();
            break;
        }
        let input_ns = start.elapsed().as_nanos() as u64;
        window.present();
        let total_ns = start.elapsed().as_nanos() as u64;
        if frame >= u64::from(warmup) {
            completed += 1;
            if timings {
                samples.push([input_ns, total_ns - input_ns, total_ns]);
            }
        }
        #[cfg(any(debug_assertions, feature = "allocation-tracking"))]
        {
            qa_console::logger::dev_print(
                &console.cvars,
                developer,
                1,
                format_args!(
                    "{{\"event\":\"frame_cvar_lookups\",\"scope\":\"window_shell\",\"frame\":{frame},\"lookups\":{}}}",
                    console.cvars.lookup_count()
                ),
            );
            let counts = qa_platform::allocations::end_frame();
            qa_console::logger::dev_print(
                &console.cvars,
                developer,
                1,
                format_args!(
                    "{{\"event\":\"frame_allocations\",\"scope\":\"window_shell_rust_thread\",\"frame\":{frame},\"allocations\":{},\"reallocations\":{},\"requested_bytes\":{}}}",
                    counts.allocations, counts.reallocations, counts.requested_bytes
                ),
            );
        }
        if !uncapped {
            std::thread::sleep(Duration::from_millis(16).saturating_sub(start.elapsed()));
        }
    }
    drop(window);
    if timings {
        println!(
            "{{\"event\":\"frame_timings\",\"scope\":\"window_shell\",\"warmup\":{warmup},\"frames\":{completed},\"vsync\":false,\"samples_ns\":{samples:?}}}"
        );
    }
    println!(
        "{{\"event\":\"normal_exit\",\"frames\":{completed},\"key_downs\":{key_downs},\"key_repeats\":{key_repeats}}}"
    );
    Ok(())
}

struct Runtime {
    vfs: Vfs,
    quit: bool,
}
impl Host for Runtime {
    fn print(&mut self, text: std::fmt::Arguments<'_>) {
        qa_console::logger::console(text);
    }
    fn read_script(&mut self, path: &str) -> Result<String, String> {
        let file = self
            .vfs
            .open(path.as_bytes())
            .ok_or_else(|| format!("script unavailable: {path}"))?;
        let length = self.vfs.length(file).map_err(|e| format!("{e:?}"))?;
        if length > 65535 {
            return Err("script exceeds command buffer capacity".into());
        }
        let mut bytes = vec![0; length as usize];
        self.vfs
            .read_at(file, 0, &mut bytes)
            .map_err(|e| format!("{e:?}"))?;
        String::from_utf8(bytes).map_err(|e| format!("script is not UTF-8: {e}"))
    }
    fn quit(&mut self) {
        self.quit = true;
    }
}

fn main() {
    if let Err(message) = run() {
        qa_console::logger::error(&message);
        std::process::exit(1);
    }
}
