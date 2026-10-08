use qa_console::{
    commands::{Console, Host},
    views::{Context, Source},
};
use qa_content::vfs::Vfs;
use qa_core::sys_events::{DeviceId, EventKind, EventTime, SeatId, SysEventQueue};
use qa_input::{Input, Target};
use qa_network::ingress::PacketReceiver;
use qa_platform::{EventPump, Window};
use std::time::Duration;

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
        network: PacketReceiver::default(),
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
    let mut input = Input::load();
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
                if !input.assign(DeviceId::Controller(id), seat) {
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
    let mut queue = SysEventQueue::load(1024, 256 * 1024).map_err(|e| format!("{e:?}"))?;
    let mut frame_time = EventTime::default();
    #[cfg(feature = "proof")]
    let mut script_start = None;
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
        #[cfg(feature = "proof")]
        if let Some(script) = &mut script {
            script.inject_due(
                Duration::from_nanos(
                    script_start
                        .map(|start| frame_time.since(start))
                        .unwrap_or(0),
                ),
                &mut window,
            );
        }
        pump.begin_frame(&mut window, &mut queue);
        let mut event_count = 0;
        // Exactly one FIFO drain. Consumers do not poll SDL, sockets or clocks.
        while let Some(event) = queue.pop() {
            event_count += 1;
            match event.kind {
                EventKind::Time => frame_time = event.time,
                EventKind::Quit => runtime.quit = true,
                EventKind::ConsoleLine(text) => {
                    let result = console.append(text, console.cvars.context());
                    if result.is_ok() {
                        let _ = console.append("\n", console.cvars.context());
                    }
                }
                EventKind::Packet {
                    socket,
                    from,
                    bytes,
                } => runtime.network.receive(socket, from, bytes, event.time),
                _ => input.dispatch(
                    event,
                    &mut ConsoleInput {
                        console: &mut console,
                    },
                ),
            }
            if let EventKind::Key {
                down: true, repeat, ..
            } = event.kind
            {
                key_downs += 1;
                key_repeats += u64::from(repeat);
            }
            if !matches!(event.kind, EventKind::Time | EventKind::Packet { .. }) {
                qa_console::logger::dev_print(
                    &console.cvars,
                    developer,
                    1,
                    format_args!(
                        "{{\"event\":\"input_diagnostic\",\"time_ns\":{},\"input\":\"{}\"}}",
                        event.time.0,
                        event_name(event.kind)
                    ),
                );
            }
        }
        #[cfg(feature = "proof")]
        script_start.get_or_insert(frame_time);
        console.execute_frame(&mut runtime);
        if runtime.quit {
            #[cfg(any(debug_assertions, feature = "allocation-tracking"))]
            let _ = qa_platform::allocations::end_frame();
            break;
        }
        // R2 routing proof uses normalized units. THE-735 supplies each seat's
        // selected movement speed/angle policy through cached cvar handles.
        let commands = input.build_frame(frame_time, [127; 3], [0.022; 2], [None; SeatId::COUNT]);
        qa_console::logger::dev_print(
            &console.cvars,
            developer,
            1,
            format_args!(
                "{{\"event\":\"system_event_frame\",\"scope\":\"window_shell\",\"frame\":{frame},\"time_ns\":{},\"events\":{event_count},\"queue_remaining\":{},\"rejected\":{},\"dropped_packets\":{},\"network_packets\":{},\"seat0_movement\":{:?},\"seat1_movement\":{:?}}}",
                frame_time.0,
                queue.len(),
                queue.rejected(),
                pump.dropped_packets(),
                runtime.network.packets,
                commands[0].movement,
                commands[1].movement
            ),
        );
        let input_ns = pump.elapsed().as_nanos() as u64;
        window.present();
        let total_ns = pump.elapsed().as_nanos() as u64;
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
            pump.pace(Duration::from_millis(16));
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

struct ConsoleInput<'a> {
    console: &'a mut Console<Runtime>,
}
impl Target for ConsoleInput<'_> {
    fn character(&mut self, _seat: SeatId, _value: char) {
        // THE-651 adds console/menu focus and editing here; never poll SDL there.
    }
    fn command(&mut self, _seat: SeatId, text: &str) {
        let context = self.console.cvars.context();
        let _ = self.console.append(text, context);
        let _ = self.console.append("\n", context);
    }
}
struct Runtime {
    vfs: Vfs,
    quit: bool,
    network: PacketReceiver,
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

fn event_name(kind: EventKind<'_>) -> &'static str {
    match kind {
        EventKind::Time => "time",
        EventKind::Key { .. } => "key",
        EventKind::Char { .. } => "char",
        EventKind::Mouse { .. } => "mouse",
        EventKind::MouseButton { .. } => "mouse_button",
        EventKind::MouseWheel { .. } => "mouse_wheel",
        EventKind::ControllerAxis { .. } => "controller_axis",
        EventKind::ControllerButton { .. } => "controller_button",
        EventKind::DeviceRemoved(_) => "device_removed",
        EventKind::Focus(_) => "focus",
        EventKind::Quit => "quit",
        EventKind::ConsoleLine(_) => "console_line",
        EventKind::Packet { .. } => "packet",
    }
}
fn main() {
    if let Err(message) = run() {
        qa_console::logger::error(&message);
        std::process::exit(1);
    }
}
