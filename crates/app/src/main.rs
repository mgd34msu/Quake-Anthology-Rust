use qa_console::cvars::Cvars;
use qa_platform::{InputEvent, Window};
use std::time::{Duration, Instant};

#[cfg(feature = "proof")]
mod proof;

fn run() -> Result<(), String> {
    let mut cvars = Cvars::new(qa_console::cvars_generated::DEFINITIONS);
    let developer = cvars.find("developer").ok_or("developer cvar missing")?;
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
                let name = args.next().ok_or("+set needs a cvar name")?;
                let value: f32 = args
                    .next()
                    .ok_or("+set needs a value")?
                    .parse()
                    .map_err(|_| "invalid numeric cvar value")?;
                if !value.is_finite() {
                    return Err("cvar value must be finite".into());
                }
                let handle = cvars
                    .find(&name)
                    .ok_or_else(|| format!("unknown cvar: {name}"))?;
                cvars.set(handle, value);
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
        let start = Instant::now();
        #[cfg(feature = "proof")]
        if let Some(script) = &mut script {
            script.inject_due(script_start.elapsed(), &mut window);
        }
        if window.poll(|event| {
            if let InputEvent::KeyDown { repeat, .. } = event {
                key_downs += 1;
                key_repeats += u64::from(repeat);
            }
            qa_console::logger::dev_print(
                &cvars,
                developer,
                1,
                format_args!("{{\"event\":\"input_diagnostic\",\"input\":\"{event:?}\"}}"),
            );
        }) {
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

fn main() {
    if let Err(message) = run() {
        qa_console::logger::error(&message);
        std::process::exit(1);
    }
}
