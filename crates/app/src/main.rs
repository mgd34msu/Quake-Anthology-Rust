use qa_platform::Window;
use std::time::{Duration, Instant};

fn run() -> Result<(), String> {
    let mut frames = 120u32;
    let mut width = 640i32;
    let mut height = 400i32;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--build-info" => {
                println!(
                    "{{\"commit\":\"{}\",\"source_tree_dirty\":{},\"target_cpu\":\"{}\"}}",
                    option_env!("QA_BUILD_COMMIT").unwrap_or("unrecorded"),
                    option_env!("QA_BUILD_DIRTY").unwrap_or("true"),
                    option_env!("QA_TARGET_CPU").unwrap_or("baseline")
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
    println!("{{\"event\":\"window_ready\",\"gameplay\":false}}");
    let mut completed = 0;
    for _ in 0..frames {
        let start = Instant::now();
        if window.poll_quit() {
            break;
        }
        window.present();
        completed += 1;
        std::thread::sleep(Duration::from_millis(16).saturating_sub(start.elapsed()));
    }
    drop(window);
    println!("{{\"event\":\"normal_exit\",\"frames\":{completed}}}");
    Ok(())
}

fn main() {
    if let Err(message) = run() {
        eprintln!("{message}");
        std::process::exit(1);
    }
}
