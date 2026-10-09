use qa_app::map;
use qa_app::renderer::{Kind, Renderer, cpu_band_setting, parse_cpu_bands};
use qa_app::{
    Runtime,
    client_policy::ClientPolicy,
    host::{FrameHost, LiveFrame},
};
use qa_console::{
    commands::Console,
    views::{Context, RuleSetId},
};
use qa_core::sys_events::{DeviceId, SeatId};
use qa_platform::EventPump;
use qa_render::{Assets, material::world_load::WorldLoadOptions};
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
    let mut console = Console::<Runtime>::new(Context::default()).map_err(|e| e.to_string())?;
    let mut vfs = qa_content::vfs::Vfs::default();
    let mut device_assignments = Vec::new();
    let mut frames = 120u32;
    let mut width = 640i32;
    let mut height = 400i32;
    let mut warmup = 0u32;
    let mut timings = false;
    let mut uncapped = false;
    let mut startup_hold = 0u64;
    let mut renderer_kind = Kind::Cpu;
    let mut cpu_bands = None;
    let mut map_name = None;
    let mut movement_rules = None;
    let mut trace_rules = None;
    let mut client_module = None;
    let mut seat_policies = [None; SeatId::COUNT];
    // The built-in walk-through host has no native module/sign-on yet. Its
    // explicit transport default is independent of map and movement identity.
    let mut local_protocol = qa_network::commands::packet::Protocol::QuakeWorld28;
    let mut seat_protocols = [None; SeatId::COUNT];
    let mut seat_count = 1;
    let mut max_clients = 64usize;
    let mut console_source_explicit = false;
    let mut content_priority = 0i32;
    let mut startup_sets = Vec::new();
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
                startup_sets.push((name, value, cvars.context()));
            }
            "--commands" => {
                let text = args.next().ok_or("--commands needs text")?;
                console
                    .append(&(text + "\n"), console.cvars.context())
                    .map_err(|e| format!("{e:?}"))?;
            }
            "--console-source" => {
                console_source_explicit = true;
                let name = args
                    .next()
                    .ok_or("--console-source needs q1/qw/q2/q2rr/q3")?;
                let source = RuleSetId::parse(&name).ok_or("unknown console source")?;
                console.cvars.select_context(Context {
                    source,
                    ..console.cvars.context()
                });
            }
            "--content" => {
                let path = args.next().ok_or("--content needs a product directory")?;
                vfs.mount_product(std::path::Path::new(&path), content_priority)
                    .map_err(|e| format!("content: {e:?}"))?;
                content_priority = content_priority
                    .checked_add(100_000)
                    .ok_or("too many content mounts")?;
            }
            "--map" => map_name = Some(args.next().ok_or("--map needs a virtual map name")?),
            "--movement" => {
                movement_rules = Some(
                    RuleSetId::parse(&args.next().ok_or("--movement needs q1/qw/q2/q2rr/q3")?)
                        .ok_or("unknown movement rule set")?,
                );
            }
            "--client-module" => {
                client_module = Some(
                    RuleSetId::parse(
                        &args
                            .next()
                            .ok_or("--client-module needs q1/qw/q2/q2rr/q3")?,
                    )
                    .ok_or("unknown client module preset")?,
                );
            }
            "--trace-rules" => {
                trace_rules = Some(
                    RuleSetId::parse(&args.next().ok_or("--trace-rules needs q1/qw/q2/q2rr/q3")?)
                        .ok_or("unknown trace rule set")?,
                );
            }
            "--seat-policy" => {
                let (seat, policy) = ClientPolicy::parse_seat(
                    &args
                        .next()
                        .ok_or("--seat-policy needs seat:client:movement:trace")?,
                )?;
                if seat_policies[seat.index()].replace(policy).is_some() {
                    return Err("seat-policy was supplied more than once for a seat".into());
                }
                seat_count = seat_count.max(seat.index() + 1);
            }
            "--local-protocol" => {
                local_protocol = qa_network::commands::packet::Protocol::parse(
                    &args.next().ok_or("--local-protocol needs 15/28/34/68")?,
                )
                .ok_or("unsupported local protocol")?;
            }
            "--seat-protocol" => {
                let value = args
                    .next()
                    .ok_or("--seat-protocol needs seat:15/28/34/68")?;
                let (seat, protocol) = value
                    .split_once(':')
                    .ok_or("--seat-protocol needs seat:15/28/34/68")?;
                let seat = seat
                    .parse()
                    .ok()
                    .and_then(SeatId::new)
                    .ok_or("invalid protocol seat")?;
                let protocol = qa_network::commands::packet::Protocol::parse(protocol)
                    .ok_or("unsupported local protocol")?;
                if seat_protocols[seat.index()].replace(protocol).is_some() {
                    return Err("duplicate seat-protocol".into());
                }
                seat_count = seat_count.max(seat.index() + 1);
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
            "--controller-seat" | "--mouse-seat" => {
                let value = args.next().ok_or("device assignment needs instance:seat")?;
                let (device, seat) = value
                    .split_once(':')
                    .ok_or("device assignment needs instance:seat")?;
                let id = if arg == "--mouse-seat" {
                    DeviceId::Mouse(device.parse().map_err(|_| "invalid mouse instance")?)
                } else {
                    DeviceId::Controller(device.parse().map_err(|_| "invalid controller instance")?)
                };
                let seat = SeatId::new(seat.parse().map_err(|_| "invalid seat")?)
                    .ok_or("seat outside 0..4")?;
                device_assignments.push((id, seat));
                seat_count = seat_count.max(seat.index() + 1);
            }
            "--frame-timings" => timings = true,
            "--renderer" => {
                renderer_kind = Kind::parse(&args.next().ok_or("--renderer needs cpu or gl")?)?
            }
            "--cpu-bands" => {
                if cpu_bands.is_some() {
                    return Err("--cpu-bands was supplied more than once".into());
                }
                cpu_bands = Some(parse_cpu_bands(
                    &args.next().ok_or("--cpu-bands needs 1, 2, 4 or 8")?,
                )?);
            }
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
            "--max-clients" => {
                max_clients = args
                    .next()
                    .ok_or("--max-clients needs a number")?
                    .parse()
                    .map_err(|_| "invalid client capacity")?;
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
    if cpu_bands.is_some() && matches!(renderer_kind, Kind::Gl) {
        return Err("--cpu-bands requires --renderer cpu".into());
    }
    if (movement_rules.is_some()
        || trace_rules.is_some()
        || client_module.is_some()
        || seat_policies.iter().any(Option::is_some))
        && map_name.is_none()
    {
        return Err("movement and trace roles need a loaded --map".into());
    }
    if max_clients < seat_count {
        return Err("client capacity must cover the local seats".into());
    }
    let staged_map = if let Some(name) = map_name {
        let input = map::read(&vfs, &name)?;
        let policy = if let Some(policy) = seat_policies[0] {
            policy
        } else {
            ClientPolicy::select(
                client_module,
                input.client_rules,
                movement_rules,
                trace_rules,
            )?
        };
        Some((input, policy))
    } else {
        None
    };
    let names = staged_map
        .as_ref()
        .map(|(input, _)| input.catalog_names())
        .transpose()?
        .unwrap_or_default();
    let mut runtime = Runtime::load(max_clients, names.iter().map(|name| name.as_ref()))?;
    drop(names);
    runtime.vfs = vfs;
    for (device, seat) in device_assignments {
        if !runtime.input.assign(device, seat) {
            return Err("device table full".into());
        }
        println!(
            "{{\"event\":\"device_assignment\",\"device\":{},\"seat\":{}}}",
            json_string(&format!("{device:?}")),
            seat.index()
        );
    }
    let mut loaded_world = None;
    let mut local_clients = [None; SeatId::COUNT];
    let mut world_rate = TickRate::FrameDriven;
    let mut map_path = None;
    let mut imported_profile = qa_app::profile::Import::default();
    if let Some((input, policy)) = &staged_map {
        if !console_source_explicit {
            console.cvars.select_context(Context {
                source: policy.client,
                ..console.cvars.context()
            });
        }
        imported_profile = qa_app::profile::load(
            &mut console,
            &mut runtime,
            &input.profile_product(policy.client),
            policy.client,
            policy.movement,
        )?;
        println!(
            "{{\"event\":\"profile_import\",\"consumed\":{},\"files\":{},\"applied_cvars\":{},\"applied_bindings\":{},\"unsupported_settings\":{},\"active_saved_seats\":1,\"connected_seats\":{seat_count},\"history_imported\":false,\"settings_only\":true}}",
            imported_profile.consumed(),
            serde_json::to_string(&imported_profile.files)
                .map_err(|e| format!("profile report: {e}"))?,
            imported_profile.cvars,
            imported_profile.bindings,
            imported_profile.unsupported
        );
    }
    // Saved settings are already in the one table. Explicit CLI values win
    // before image/material registration consumes its cold settings.
    for (name, value, context) in &startup_sets {
        let view = console
            .cvars
            .bind(name, *context)
            .ok_or_else(|| format!("unknown startup cvar {name}"))?;
        console
            .cvars
            .write(view, value)
            .map_err(|e| format!("startup cvar {name}: {e:?}"))?;
    }
    let mut assets = Assets::load().map_err(|e| e.to_string())?;
    if let Some((input, policy)) = staged_map {
        let image_settings =
            qa_app::render_settings::image_settings(&console.cvars, input.native_source)?;
        let loaded = input.load(
            &runtime.vfs,
            &mut assets,
            &mut runtime.geometry,
            WorldLoadOptions {
                image_settings: Some(image_settings),
                ..WorldLoadOptions::default()
            },
        )?;
        world_rate = policy.tick_rate(&mut console.cvars)?;
        let tick_ms = match world_rate {
            TickRate::FrameDriven => None,
            TickRate::FixedMilliseconds(period) => Some(period.get()),
        };
        println!(
            "{{\"event\":\"client_policy\",\"client_module\":\"{}\",\"module_loaded\":false,\"tick_rules\":\"{}\",\"tick_ms\":{},\"link_rules\":\"{}\",\"link_first\":{}}}",
            policy.client.name(),
            policy.client.name(),
            serde_json::to_string(&tick_ms).map_err(|e| format!("client period report: {e}"))?,
            policy.client.name(),
            policy.link_order() == qa_world::area::LinkOrder::Head,
        );
        runtime.entity_sources.push(loaded.entity_source);
        runtime.server.area = qa_world::area::AreaGrid::load(
            runtime.server.entities.capacity(),
            loaded.collision_bounds,
        )
        .map_err(|e| format!("world area: {e:?}"))?;
        runtime.collision = Some(qa_app::WorldCollision::new(
            &runtime.geometry,
            loaded.collision,
            0,
        ));
        if loaded.spawns.len() < seat_count {
            return Err(format!(
                "map has {} distinct spawn anchors for {seat_count} local seats",
                loaded.spawns.len()
            ));
        }
        for (index, slot) in local_clients.iter_mut().enumerate().take(seat_count) {
            let seat = SeatId::new(index as u8).ok_or("invalid local seat")?;
            let policy = seat_policies[index].unwrap_or(policy);
            let rules = policy.movement;
            let traces = policy.trace;
            let spawn = loaded.spawns[index];
            let protocol = seat_protocols[index].unwrap_or(local_protocol);
            let client = runtime.connect_local(seat, spawn, policy, protocol)?;
            println!(
                "{{\"event\":\"local_channel\",\"seat\":{index},\"client\":{},\"protocol\":{},\"native_signon\":false}}",
                client.0,
                protocol.number()
            );
            let player = &runtime.server.clients[client.0 as usize].player;
            println!(
                "{{\"event\":\"map_loaded\",\"scope\":\"retail_map_walk_integration\",\"gameplay\":false,\"map\":{},\"movement\":\"{}\",\"trace_rules\":\"{}\",\"world\":{},\"client\":{},\"parsed_entities\":{},\"seat\":{index},\"spawned_clients\":{seat_count},\"module_entities_spawned\":0,\"collision_brushes\":{},\"spawn_entity\":{},\"spawn_fixture_fallback\":{},\"position\":{:?},\"angles\":{:?},\"mins\":{:?},\"maxs\":{:?},\"foreign_q1_box_limitation\":{},\"profile_consumed\":{},\"native_input_policy\":false}}",
                json_string(&loaded.virtual_path),
                rules.name(),
                traces.name(),
                loaded.render.world.0,
                client.0,
                loaded.entity_count,
                loaded.collision_brushes,
                spawn.entity,
                spawn.fixture_fallback,
                player.body.position.0,
                player.view_angles.0,
                player.body.mins.0,
                player.body.maxs.0,
                loaded.native_source == RuleSetId::Quake
                    && rules != qa_core::primitives::RuleSetId::Quake,
                imported_profile.consumed()
            );
            *slot = Some(client);
        }
        map_path = Some(loaded.virtual_path);
        loaded_world = Some(loaded.render);
    }
    let render_world = loaded_world.as_ref().map(|world| world.world);
    let mut host = FrameHost::load(console, runtime, world_rate, Vec::new())?;
    host.local_clients = local_clients;
    for (world, client) in host.local_worlds.iter_mut().zip(local_clients) {
        *world = client.and(render_world);
    }
    let (band_cvar, cpu_bands) = if matches!(renderer_kind, Kind::Cpu) {
        cpu_band_setting(&host.console.cvars, cpu_bands)?
    } else {
        (
            host.console
                .cvars
                .find("r_cpuBands")
                .ok_or("missing CPU band setting")?,
            None,
        )
    };
    let mut window = renderer_kind.open(width, height)?;
    let mut renderer = Renderer::load(
        renderer_kind,
        &window,
        width as u32,
        height as u32,
        assets,
        loaded_world.into_iter().collect(),
        cpu_bands,
    )?;
    if matches!(renderer_kind, Kind::Cpu) {
        host.console.cvars.set_latch_active(band_cvar, true);
    }
    renderer.frame(&host.client_views());
    #[cfg(any(debug_assertions, feature = "allocation-tracking"))]
    let _ = renderer.merge_worker_counts(qa_platform::allocations::Counts::default());
    renderer.present(&mut window);
    if !renderer.sample.presented {
        return Err("initial frame presentation failed".into());
    }
    if let Some(path) = &map_path {
        println!(
            "{{\"event\":\"world_frame_presented\",\"gameplay\":false,\"map\":{},\"renderer\":\"{}\",\"client_connected\":{},\"views\":{},\"visible_surfaces\":{},\"surfaces\":{},\"triangles\":{},\"rejected\":{},\"profile_consumed\":{},\"native_input_policy\":{}}}",
            json_string(path),
            renderer_kind.name(),
            local_clients[0].is_some(),
            renderer.sample.stats.views,
            renderer.sample.visible_surfaces,
            renderer.sample.stats.surfaces,
            renderer.sample.stats.triangles,
            renderer.sample.stats.rejected,
            imported_profile.consumed(),
            host.native_input_policy_active()
        );
    }
    println!(
        "{{\"event\":\"window_ready\",\"gameplay\":false,\"video_driver\":\"{}\",\"wayland_display_present\":{}}}",
        window.video_driver(),
        std::env::var_os("WAYLAND_DISPLAY").is_some()
    );
    qa_platform::pause(Duration::from_millis(startup_hold));
    let scope = if map_path.is_some() {
        "retail_map_walk_integration"
    } else {
        "window_shell"
    };
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
    let mut stage_samples = if timings {
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
                renderer: &mut renderer,
            },
            uncapped,
        );
        #[cfg(feature = "proof")]
        script_start.get_or_insert(host.time);
        if host.runtime.quit {
            #[cfg(any(debug_assertions, feature = "allocation-tracking"))]
            {
                let caller = qa_platform::allocations::end_frame();
                let counts = renderer.merge_worker_counts(caller);
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
                "{{\"event\":\"system_event_frame\",\"scope\":\"{scope}\",\"frame\":{frame},\"time_ns\":{},\"events\":{},\"drains\":{},\"server_ticks\":{},\"world_frames\":{},\"client_frame\":true,\"queue_remaining\":{},\"rejected\":{},\"dropped_packets\":{},\"network_packets\":{},\"seat0_movement\":{:?},\"seat1_movement\":{:?},\"seat0_buttons\":{},\"seat0_view_angles\":{:?},\"seat0_duration_ms\":{},\"seat1_duration_ms\":{},\"command_server_time_ms\":{}}}",
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
                result.commands[0].buttons,
                result.commands[0].view_angles.0,
                result.commands[0].duration_ms,
                result.commands[1].duration_ms,
                result.commands[0].server_time_ms
            ),
        );
        for (index, id) in local_clients.iter().enumerate() {
            let Some(id) = *id else { continue };
            let authoritative = &host.runtime.server.clients[id.0 as usize].player;
            let predicted = &host.runtime.prediction[index].player;
            qa_console::logger::dev_print(
                &host.console.cvars,
                host.developer,
                1,
                format_args!(
                    "{{\"event\":\"walk_frame\",\"scope\":\"{scope}\",\"frame\":{frame},\"seat\":{index},\"client\":{},\"movement\":\"{}\",\"trace_rules\":\"{}\",\"command_movement\":{:?},\"position\":{:?},\"velocity\":{:?},\"predicted_position\":{:?},\"view_angles\":{:?},\"grounded\":{},\"ground_normal\":{:?},\"ducked\":{},\"water_level\":{},\"simulation_ns\":{},\"client_ns\":{}}}",
                    id.0,
                    authoritative.movement_rules.name(),
                    authoritative.trace_rules.name(),
                    result.commands[index].movement,
                    authoritative.body.position.0,
                    authoritative.body.velocity.0,
                    predicted.body.position.0,
                    predicted.view_angles.0,
                    authoritative.movement.grounded,
                    authoritative.movement.ground_normal.0,
                    authoritative.movement.ducked,
                    authoritative.movement.water_level,
                    result.simulation_ns,
                    result.client_ns
                ),
            );
        }
        if frame >= u64::from(warmup) {
            if timings {
                qa_console::logger::dev_print(
                    &host.console.cvars,
                    host.developer,
                    1,
                    format_args!(
                        "{{\"event\":\"render_frame\",\"scope\":\"{scope}\",\"renderer\":\"{}\",\"frame\":{frame},\"frontend_ns\":{},\"backend_ns\":{},\"present_ns\":{},\"presented\":{},\"views\":{},\"surfaces\":{},\"triangles\":{},\"stages\":{},\"rejected\":{}}}",
                        renderer_kind.name(),
                        renderer.sample.frontend_ns,
                        renderer.sample.backend_ns,
                        renderer.sample.present_ns,
                        renderer.sample.presented,
                        renderer.sample.stats.views,
                        renderer.sample.stats.surfaces,
                        renderer.sample.stats.triangles,
                        renderer.sample.stats.stages,
                        renderer.sample.stats.rejected
                    ),
                );
            }
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
                    host.runtime.server.events.len(),
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
                stage_samples.push([
                    result.simulation_ns,
                    result.client_ns,
                    renderer.sample.frontend_ns,
                    renderer.sample.backend_ns,
                    renderer.sample.present_ns,
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
                    "{{\"event\":\"frame_cvar_lookups\",\"scope\":\"{scope}\",\"frame\":{frame},\"lookups\":{}}}",
                    host.console.cvars.lookup_count()
                ),
            );
            let caller = qa_platform::allocations::end_frame();
            let counts = renderer.merge_worker_counts(caller);
            #[cfg(feature = "allocation-tracking")]
            allocation_gate.observe(frame, warmup, counts);
            qa_console::logger::dev_print(
                &host.console.cvars,
                host.developer,
                1,
                format_args!(
                    "{{\"event\":\"frame_allocations\",\"scope\":\"{scope}_all_instrumented_rust_threads\",\"frame\":{frame},\"allocations\":{},\"reallocations\":{},\"requested_bytes\":{}}}",
                    counts.allocations, counts.reallocations, counts.requested_bytes
                ),
            );
        }
    }
    if timings && let Some(stats) = renderer.cpu_world_stats() {
        qa_console::logger::console(format_args!(
            "{{\"event\":\"cpu_draw_profile\",\"scope\":\"{scope}\",\"gameplay\":false,\"width\":{width},\"height\":{height},\"recorded_rendered_frames\":{},\"includes_startup\":true,\"includes_warmup\":true,\"requested_warmup_frames\":{warmup},\"measured_host_frames\":{completed},\"final_rendered_frame\":{{\"polygons\":{},\"patch_polygons\":{},\"spans\":{},\"pixels\":{},\"sky_spans\":{},\"sky_pixels\":{},\"stage_spans\":{},\"stage_pixels\":{},\"curve_spans\":{},\"curve_pixels\":{},\"multistage_spans\":{},\"multistage_pixels\":{},\"indexed_spans\":{},\"indexed_pixels\":{},\"rgba_spans\":{},\"rgba_pixels\":{},\"rgba_hits\":{},\"rgba_fills\":{},\"rgba_evictions\":{},\"rgba_rejected\":{},\"rgba_minified_spans\":{},\"factor_spans\":{},\"factor_pixels\":{},\"factor_hits\":{},\"factor_fills\":{},\"factor_evictions\":{},\"factor_rejected\":{},\"factor_fallback_spans\":{},\"factor_minified_spans\":{},\"factor_curve_spans\":{},\"factor_curve_pixels\":{},\"rejected\":{}}},\"cache_cumulative\":{{\"hits\":{},\"fills\":{},\"evictions\":{},\"rejected\":{}}}}}\n",
            renderer.recorded_rendered_frames(),
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
    if timings && let Some(stats) = renderer.cpu_world_stats() {
        let cache = stats.cache;
        qa_console::logger::console(format_args!(
            "{{\"event\":\"cpu_cache_diagnostics\",\"includes_startup_and_warmup\":true,\"nonresident_fills\":{},\"changed_state_fills\":{},\"filled_payload_bytes\":{},\"evicted_payload_bytes\":{},\"resident_payload_bytes\":{},\"sum_band_peak_resident_payload_bytes\":{}}}\n",
            cache.nonresident_fills,
            cache.state_fills,
            cache.fill_bytes,
            cache.evicted_bytes,
            cache.resident_bytes,
            cache.peak_resident_bytes,
        ));
    }
    let worker_error = renderer.worker_error();
    #[cfg(feature = "allocation-tracking")]
    let worker_threads = renderer.worker_count();
    drop(renderer);
    drop(window);
    if let Some(error) = worker_error {
        return Err(format!("CPU worker qualification failed: {error}"));
    }
    #[cfg(feature = "allocation-tracking")]
    allocation_gate.finish(scope, worker_threads)?;
    if timings {
        println!(
            "{{\"event\":\"frame_timings\",\"scope\":\"{scope}\",\"warmup\":{warmup},\"frames\":{completed},\"vsync\":false,\"samples_ns\":{samples:?},\"stage_columns\":[\"simulation\",\"client\",\"scene\",\"draw\",\"present\"],\"stage_samples_ns\":{stage_samples:?},\"audio_measured\":false}}"
        );
    }
    println!(
        "{{\"event\":\"normal_exit\",\"frames\":{completed},\"key_downs\":{},\"key_repeats\":{}}}",
        host.key_downs, host.key_repeats
    );
    Ok(())
}

fn json_string(value: &str) -> String {
    use std::fmt::Write;
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for ch in value.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c <= '\u{1f}' => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn main() {
    if let Err(message) = run() {
        qa_console::logger::error(&message);
        std::process::exit(1);
    }
}
