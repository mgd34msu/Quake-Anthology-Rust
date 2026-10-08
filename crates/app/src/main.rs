use qa_app::{
    Runtime,
    host::{FrameHost, LiveFrame},
};
use qa_console::{
    commands::Console,
    views::{Context, Source},
};
use qa_core::sys_events::{DeviceId, SeatId};
use qa_platform::{EventPump, Window};
use qa_session::timing::TickRate;
use std::time::Duration;

#[cfg(any(debug_assertions, feature = "allocation-tracking"))]
#[global_allocator]
static ALLOCATOR: qa_platform::allocations::CountingAllocator =
    qa_platform::allocations::CountingAllocator;

#[cfg(feature = "proof")]
mod proof;

#[cfg(feature = "allocation-tracking")]
mod allocation_gate;

fn run() -> Result<(), String> {
    let mut console = Console::<Runtime>::new(Context::default());
    let mut runtime = Runtime::load()?;
    let mut frames = 120u32;
    let mut width = 640i32;
    let mut height = 400i32;
    let mut warmup = 0u32;
    let mut timings = false;
    let mut uncapped = false;
    let mut startup_hold = 0u64;
    #[cfg(feature = "proof")]
    let mut script = None;
    let mut pump = EventPump::new();
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
            "--udp-listen" => {
                let address = args
                    .next()
                    .ok_or("--udp-listen needs an address")?
                    .parse()
                    .map_err(|_| "invalid UDP address")?;
                let (socket, address) = pump.bind_udp(address).map_err(|e| e.to_string())?;
                println!(
                    "{{\"event\":\"udp_listen\",\"socket\":{socket},\"address\":\"{address}\"}}"
                );
            }
            "--controller-seat" => {
                let value = args.next().ok_or("--controller-seat needs instance:seat")?;
                let (device, seat) = value
                    .split_once(':')
                    .ok_or("--controller-seat needs instance:seat")?;
                let id = device.parse().map_err(|_| "invalid controller instance")?;
                let seat = SeatId::new(seat.parse().map_err(|_| "invalid seat")?)
                    .ok_or("seat outside 0..4")?;
                if !runtime.input.assign(DeviceId::Controller(id), seat) {
                    return Err("device table full".into());
                }
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
    qa_platform::pause(Duration::from_millis(startup_hold));
    let mut host = FrameHost::load(console, runtime, TickRate::FrameDriven, Vec::new())?;
    #[cfg(feature = "proof")]
    let mut script_start = None;
    let mut completed = 0;
    #[cfg(feature = "allocation-tracking")]
    let mut allocation_gate = allocation_gate::Gate::default();
    let mut samples = if timings {
        Vec::with_capacity(frames as usize)
    } else {
        Vec::new()
    };
    for frame in 0..u64::from(frames) + u64::from(warmup) {
        #[cfg(any(debug_assertions, feature = "allocation-tracking"))]
        host.console.cvars.reset_lookup_count();
        #[cfg(any(debug_assertions, feature = "allocation-tracking"))]
        qa_platform::allocations::begin_frame();
        #[cfg(feature = "proof")]
        if let Some(script) = &mut script {
            script.inject_due(
                Duration::from_nanos(
                    script_start
                        .map(|start| host.time.since(start))
                        .unwrap_or(0),
                ),
                &mut window,
            );
        }
        let result = host.frame(
            &mut LiveFrame {
                pump: &mut pump,
                window: &mut window,
            },
            uncapped,
        );
        #[cfg(feature = "proof")]
        script_start.get_or_insert(host.time);
        if host.runtime.quit {
            #[cfg(any(debug_assertions, feature = "allocation-tracking"))]
            {
                let counts = qa_platform::allocations::end_frame();
                #[cfg(feature = "allocation-tracking")]
                allocation_gate.observe(frame, warmup, counts);
                #[cfg(not(feature = "allocation-tracking"))]
                let _ = counts;
            }
            break;
        }
        qa_console::logger::dev_print(
            &host.console.cvars,
            host.developer,
            1,
            format_args!(
                "{{\"event\":\"system_event_frame\",\"scope\":\"window_shell\",\"frame\":{frame},\"time_ns\":{},\"events\":{},\"drains\":{},\"server_ticks\":{},\"world_frames\":{},\"client_frame\":true,\"queue_remaining\":{},\"rejected\":{},\"dropped_packets\":{},\"network_packets\":{},\"seat0_movement\":{:?},\"seat1_movement\":{:?},\"seat0_duration_ms\":{},\"seat1_duration_ms\":{},\"command_server_time_ms\":{}}}",
                host.time.0,
                result.events,
                result.drains,
                result.server_ticks,
                host.runtime.server.world_frame,
                host.queue.len(),
                host.queue.rejected(),
                pump.dropped_packets(),
                host.runtime.network.packets,
                result.commands[0].movement,
                result.commands[1].movement,
                result.commands[0].duration_ms,
                result.commands[1].duration_ms,
                result.commands[0].server_time_ms
            ),
        );
        if frame >= u64::from(warmup) {
            qa_console::logger::dev_print(
                &host.console.cvars,
                host.developer,
                1,
                format_args!(
                    "{{\"event\":\"stdin_frame\",\"frame\":{frame},\"lines\":{},\"discarded\":{},\"errors\":{}}}",
                    pump.console_lines(),
                    pump.discarded_console_lines(),
                    pump.console_errors()
                ),
            );
            qa_console::logger::dev_print(
                &host.console.cvars,
                host.developer,
                1,
                format_args!(
                    "{{\"event\":\"output_frame\",\"frame\":{frame},\"drains\":{},\"remaining\":{},\"sounds\":{},\"effects\":{},\"prints\":{},\"unhandled_sounds\":{},\"unhandled_effects\":{},\"stale_texts\":{}}}",
                    result.output_drains,
                    host.runtime.events.len(),
                    result.output.sounds,
                    result.output.effects,
                    result.output.prints,
                    result.output.unhandled_sounds,
                    result.output.unhandled_effects,
                    result.output.stale_texts
                ),
            );
            completed += 1;
            if timings {
                samples.push([
                    result.input_ns,
                    result.total_ns - result.input_ns,
                    result.total_ns,
                ]);
            }
        }
        #[cfg(any(debug_assertions, feature = "allocation-tracking"))]
        {
            qa_console::logger::dev_print(
                &host.console.cvars,
                host.developer,
                1,
                format_args!(
                    "{{\"event\":\"frame_cvar_lookups\",\"scope\":\"window_shell\",\"frame\":{frame},\"lookups\":{}}}",
                    host.console.cvars.lookup_count()
                ),
            );
            let counts = qa_platform::allocations::end_frame();
            #[cfg(feature = "allocation-tracking")]
            allocation_gate.observe(frame, warmup, counts);
            qa_console::logger::dev_print(
                &host.console.cvars,
                host.developer,
                1,
                format_args!(
                    "{{\"event\":\"frame_allocations\",\"scope\":\"window_shell_rust_thread\",\"frame\":{frame},\"allocations\":{},\"reallocations\":{},\"requested_bytes\":{}}}",
                    counts.allocations, counts.reallocations, counts.requested_bytes
                ),
            );
        }
    }
    drop(window);
    #[cfg(feature = "allocation-tracking")]
    allocation_gate.finish()?;
    if timings {
        println!(
            "{{\"event\":\"frame_timings\",\"scope\":\"window_shell\",\"warmup\":{warmup},\"frames\":{completed},\"vsync\":false,\"samples_ns\":{samples:?}}}"
        );
    }
    println!(
        "{{\"event\":\"normal_exit\",\"frames\":{completed},\"key_downs\":{},\"key_repeats\":{}}}",
        host.key_downs, host.key_repeats
    );
    Ok(())
}

fn main() {
    if let Err(message) = run() {
        qa_console::logger::error(&message);
        std::process::exit(1);
    }
}
