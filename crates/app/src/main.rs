use qa_app::map;
use qa_app::options::{insert_seat_value, parse_seat, parse_seat_value};
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
use qa_render::Assets;
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

fn json(value: &str) -> Result<String, String> {
    serde_json::to_string(value).map_err(|error| format!("JSON report: {error}"))
}

fn startup_server_client(
    host: &FrameHost,
    seat: usize,
    original: Option<(
        qa_core::primitives::ClientId,
        qa_core::primitives::EntityId,
        RuleSetId,
    )>,
) -> Option<qa_core::primitives::ClientId> {
    let (id, lifetime, _) = original?;
    let row = host.runtime.server.clients.get(id.0 as usize)?;
    (host.local_clients[seat] == Some(id)
        && row.entity == lifetime
        && row.connection.is_some()
        && row.server_authoritative())
    .then_some(id)
}

fn run() -> Result<(), String> {
    let mut console = Console::<Runtime>::new(Context::default()).map_err(|e| e.to_string())?;
    let mut vfs = qa_content::vfs::Vfs::default();
    let mut device_assignments = Vec::new();
    let mut frames: Option<u32> = None;
    let mut width: Option<i32> = None;
    let mut height: Option<i32> = None;
    let mut warmup = 0u32;
    let mut timings = false;
    let mut uncapped = false;
    let mut startup_hold = 0u64;
    let mut renderer_kind = Kind::Cpu;
    let mut cpu_bands = None;
    let mut map_name = None;
    let mut movement_rules = None;
    let mut input_rules = None;
    let mut seat_input_rules = [None; SeatId::COUNT];
    let mut presentation_rules = None;
    let mut seat_presentations = [None; SeatId::COUNT];
    let mut trace_rules = None;
    let mut client_module = None;
    let mut server_rules = None;
    let mut auto_modules = true;
    let mut seat_policies = [None; SeatId::COUNT];
    let mut local_protocol = None;
    let mut seat_protocols = [None; SeatId::COUNT];
    let mut seat_game_modules = [None; SeatId::COUNT];
    let mut seat_count = 1;
    let mut max_clients = 64usize;
    let mut console_source_explicit = false;
    let mut content_priority = 0i32;
    let mut download_root = None;
    let mut write_root = None;
    let mut skin_download_root = None;
    let mut startup_sets = Vec::new();
    let mut precache_sounds = Vec::new();
    let mut quakec_modules = Vec::new();
    let mut q2_modules = Vec::new();
    let mut q3_modules = Vec::new();
    let mut journal: Option<qa_content::journal::Mode<std::path::PathBuf, std::path::PathBuf>> =
        None;
    #[cfg(feature = "proof")]
    let mut script = None;
    let mut pump = EventPump::new();
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--manual-modules" => auto_modules = false,
            "--server-rules" => {
                server_rules = Some(
                    RuleSetId::parse(&args.next().ok_or("--server-rules needs q1/qw/q2/q2rr/q3")?)
                        .ok_or("unknown SERVER rules")?,
                );
            }
            "--record-journal" | "--play-journal" => {
                if journal.is_some() {
                    return Err("select one journal operation".into());
                }
                let path = std::path::PathBuf::from(
                    args.next().ok_or("journal option needs a file path")?,
                );
                journal = Some(if arg == "--record-journal" {
                    qa_content::journal::Mode::Recording(path)
                } else {
                    qa_content::journal::Mode::Replay(path)
                });
            }
            "--q2-module" => {
                q2_modules.push(qa_app::modules::Q2Spec::parse(
                    &args
                        .next()
                        .ok_or("--q2-module needs q2|q2rr:virtual-file")?,
                )?);
            }
            "--qvm-module" | "--q3-module" => {
                q3_modules.push(qa_app::modules::Q3Spec::parse(
                    &args
                        .next()
                        .ok_or("Q3 module needs game:path, cgame:seat:path or ui:path")?,
                )?);
            }
            "--quakec-module" => {
                quakec_modules.push(qa_app::modules::QuakeCSpec::parse(
                    &args
                        .next()
                        .ok_or("--quakec-module needs q1|qw:virtual-file")?,
                )?);
            }
            "--precache-sound" => {
                precache_sounds.push(args.next().ok_or("--precache-sound needs a virtual path")?);
            }
            "+set" => {
                let cvars = &mut console.cvars;
                let name = args.next().ok_or("+set needs a cvar name")?;
                let value = args.next().ok_or("+set needs a value")?;
                let context = cvars.context();
                cvars
                    .set_user(&name, &value, context, None, 0)
                    .map_err(|error| format!("cvar {name}: {error:?}"))?;
                startup_sets.push((name, value, context));
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
            "--download-root" => {
                download_root = Some(std::path::PathBuf::from(
                    args.next()
                        .ok_or("--download-root needs an existing writable content directory")?,
                ))
            }
            "--write-root" => {
                write_root = Some(std::path::PathBuf::from(
                    args.next()
                        .ok_or("--write-root needs a writable game directory")?,
                ));
            }
            "--skin-download-root" => {
                skin_download_root =
                    Some(std::path::PathBuf::from(args.next().ok_or(
                        "--skin-download-root needs an existing writable content directory",
                    )?))
            }
            "--map" => map_name = Some(args.next().ok_or("--map needs a virtual map name")?),
            "--movement" => {
                movement_rules = Some(
                    RuleSetId::parse(&args.next().ok_or("--movement needs q1/qw/q2/q2rr/q3")?)
                        .ok_or("unknown movement rule set")?,
                );
            }
            "--input-rules" => {
                input_rules = Some(
                    RuleSetId::parse(&args.next().ok_or("--input-rules needs q1/qw/q2/q2rr/q3")?)
                        .ok_or("unknown input rule set")?,
                );
            }
            "--seat-input-rules" => {
                let value = args
                    .next()
                    .ok_or("--seat-input-rules needs seat:q1/qw/q2/q2rr/q3")?;
                let (seat, rules) = parse_seat_value(&value, RuleSetId::parse)?;
                insert_seat_value(&mut seat_input_rules, &mut seat_count, seat, rules)?;
            }
            "--presentation" => {
                presentation_rules = Some(
                    RuleSetId::parse(&args.next().ok_or("--presentation needs q1/qw/q2/q2rr/q3")?)
                        .ok_or("unknown presentation rule set")?,
                );
            }
            "--seat-presentation" => {
                let value = args
                    .next()
                    .ok_or("--seat-presentation needs seat:q1/qw/q2/q2rr/q3")?;
                let (seat, rules) = parse_seat_value(&value, RuleSetId::parse)?;
                insert_seat_value(&mut seat_presentations, &mut seat_count, seat, rules)?;
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
                let (seat, policy) = qa_app::client_policy::parse_seat(
                    &args
                        .next()
                        .ok_or("--seat-policy needs seat:client:movement:trace[:input]")?,
                )?;
                insert_seat_value(&mut seat_policies, &mut seat_count, seat, policy)?;
            }
            "--seat-game-module" => {
                let value = args
                    .next()
                    .ok_or("--seat-game-module needs seat:module-number")?;
                let (seat, module) = parse_seat_value(&value, |module| {
                    module
                        .parse::<u16>()
                        .ok()
                        .filter(|&id| id != 0)
                        .map(qa_core::primitives::ModuleId)
                })?;
                insert_seat_value(&mut seat_game_modules, &mut seat_count, seat, module)?;
            }
            "--local-protocol" => {
                local_protocol = Some(
                    qa_network::commands::packet::Protocol::parse(
                        &args
                            .next()
                            .ok_or("--local-protocol needs 15/28/34/1038/68")?,
                    )
                    .ok_or("unsupported local protocol")?,
                );
            }
            "--seat-protocol" => {
                let value = args
                    .next()
                    .ok_or("--seat-protocol needs seat:15/28/34/1038/68")?;
                let (seat, protocol) =
                    parse_seat_value(&value, qa_network::commands::packet::Protocol::parse)?;
                insert_seat_value(&mut seat_protocols, &mut seat_count, seat, protocol)?;
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
            "--controller-seat" | "--mouse-seat" | "--keyboard-seat" => {
                let value = args.next().ok_or("device assignment needs instance:seat")?;
                let (device, seat) = value
                    .split_once(':')
                    .ok_or("device assignment needs instance:seat")?;
                let id = if arg == "--mouse-seat" {
                    DeviceId::Mouse(device.parse().map_err(|_| "invalid mouse instance")?)
                } else if arg == "--keyboard-seat" {
                    DeviceId::Keyboard(device.parse().map_err(|_| "invalid keyboard instance")?)
                } else {
                    DeviceId::Controller(device.parse().map_err(|_| "invalid controller instance")?)
                };
                let seat = parse_seat(seat).ok_or("seat outside 0..4")?;
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
                let count = args
                    .next()
                    .ok_or("--frames needs a number")?
                    .parse()
                    .map_err(|_| "invalid frame count")?;
                frames = (count != 0).then_some(count);
            }
            "--width" => {
                width = Some(
                    args.next()
                        .ok_or("--width needs a number")?
                        .parse()
                        .map_err(|_| "invalid width")?,
                )
            }
            "--height" => {
                height = Some(
                    args.next()
                        .ok_or("--height needs a number")?
                        .parse()
                        .map_err(|_| "invalid height")?,
                )
            }
            _ => return Err(format!("unknown option: {arg}")),
        }
    }
    if width.is_some_and(|value| value <= 0) || height.is_some_and(|value| value <= 0) {
        return Err("dimensions must be positive".into());
    }
    if timings && frames.is_none() {
        return Err("--timings needs a positive --frames limit".into());
    }
    if map_name.is_none() && frames.is_none() {
        let roots = qa_platform::content_roots().map_err(|e| format!("content locations: {e}"))?;
        let game =
            qa_app::startup::default_game(&mut vfs, roots.iter().map(|path| path.as_path()))?;
        println!(
            "{{\"event\":\"default_game\",\"product\":{},\"map\":{},\"client_rules\":\"{}\"}}",
            json(game.product)?,
            json(game.map)?,
            game.rules.name()
        );
        map_name = Some(game.map.into());
        client_module.get_or_insert(game.rules);
    }
    if cpu_bands.is_some() && matches!(renderer_kind, Kind::Gl) {
        return Err("--cpu-bands requires --renderer cpu".into());
    }
    if (movement_rules.is_some()
        || input_rules.is_some()
        || seat_input_rules.iter().any(Option::is_some)
        || trace_rules.is_some()
        || client_module.is_some()
        || seat_policies.iter().any(Option::is_some))
        && map_name.is_none()
    {
        return Err("input, movement, trace and client roles need a loaded --map".into());
    }
    if max_clients < seat_count {
        return Err("client capacity must cover the local seats".into());
    }
    let download_mounts = if let Some(root) = download_root {
        let priority = vfs
            .mounts()
            .map(|mount| mount.priority)
            .max()
            .unwrap_or(0)
            .checked_add(1)
            .ok_or("download mount priority")?;
        let game = vfs
            .mount_directory(&root, priority)
            .map_err(|error| format!("download root: {error}"))?;
        let skins = if let Some(skins) = skin_download_root {
            if skins == root {
                game
            } else {
                vfs.mount_directory(&skins, priority)
                    .map_err(|error| format!("skin download root: {error}"))?
            }
        } else {
            game
        };
        Some((game, skins))
    } else {
        if skin_download_root.is_some() {
            return Err("--skin-download-root requires --download-root".into());
        }
        None
    };
    let staged_map = if let Some(name) = map_name {
        let input = map::read(vfs.read_view(None), &name)?;
        let policy = if let Some(policy) = seat_policies[0] {
            policy
        } else {
            ClientPolicy::select(
                client_module,
                input.client_rules,
                movement_rules,
                trace_rules,
                input_rules,
            )?
        };
        Some((input, policy))
    } else {
        None
    };
    let stock_server_rules = server_rules.or_else(|| {
        staged_map
            .as_ref()
            .and_then(|(input, _)| input.client_rules)
    });
    let stock_q3_stats = if staged_map
        .as_ref()
        .is_some_and(|(input, _)| input.profile_product(RuleSetId::Quake3) == "q3a/missionpack")
    {
        qa_core::primitives::Q3StatLayout::TEAM_ARENA
    } else {
        qa_core::primitives::Q3StatLayout::BASE
    };
    let names = staged_map
        .as_ref()
        .map(|(input, _)| input.catalog_names())
        .transpose()?
        .unwrap_or_default();
    let file_rules = staged_map
        .as_ref()
        .map_or(console.cvars.context().source, |(_, policy)| policy.client);
    let write_root = write_root
        .or_else(|| qa_platform::user_files_root(file_rules.name()))
        .ok_or("missing user file directory")?;
    qa_platform::files::create_dir_all(&write_root)
        .map_err(|error| format!("user files: {error}"))?;
    let write_mount = vfs
        .mount_directory(&write_root, i32::MAX)
        .map_err(|error| format!("user files mount: {error}"))?;
    vfs.set_write_directory(write_mount)
        .map_err(|error| format!("user files authority: {error}"))?;
    let mut runtime = Runtime::load(max_clients, names.iter().map(|name| name.as_ref()))?;
    drop(names);
    runtime.vfs = vfs;
    for (device, seat) in device_assignments {
        if !runtime.input.assign(device, seat) {
            return Err("device table full".into());
        }
        println!(
            "{{\"event\":\"device_assignment\",\"device\":{},\"seat\":{}}}",
            json(&format!("{device:?}"))?,
            seat.index()
        );
    }
    let mut loaded_world = None;
    let mut local_clients = [None; SeatId::COUNT];
    let mut world_rate = TickRate::FrameDriven;
    let mut map_path = None;
    let mut native_map_checksums = None;
    let mut native_entity_source = None;
    console
        .cvars
        .set_video_policy(
            std::sync::Arc::new(qa_platform::Window::query_video_capabilities()?),
            qa_core::presentation::DisplayMode {
                width: 640,
                height: 400,
                ..Default::default()
            },
        )
        .map_err(|error| format!("video capabilities: {error:?}"))?;
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
        console
            .cvars
            .set_user(name, value, *context, None, 0)
            .map_err(|e| format!("startup cvar {name}: {e:?}"))?;
    }
    let mut assets = Assets::load().map_err(|e| e.to_string())?;
    assets.set_image_search_order(qa_app::render_settings::image_search(
        &console.cvars,
        qa_app::render_settings::image_search_view(&console.cvars)?,
    )?);
    let mut palettes = qa_app::render_settings::PaletteRegistrations::new(&runtime.vfs);
    if let Some((input, policy)) = staged_map {
        let virtual_name = input
            .virtual_path
            .strip_prefix("maps/")
            .unwrap_or(&input.virtual_path);
        let mapname = virtual_name.strip_suffix(".bsp").unwrap_or(virtual_name);
        let map_view = console
            .cvars
            .bind(
                "mapname",
                Context {
                    source: RuleSetId::Quake3,
                    side: qa_console::catalog::Scope::Server,
                    role: qa_console::views::Role::Engine,
                    ..Context::default()
                },
            )
            .ok_or("missing native mapname cvar")?;
        console
            .cvars
            .force_write(map_view, mapname)
            .map_err(|error| format!("native mapname: {error:?}"))?;
        let source_palette = qa_app::render_settings::palette(
            runtime.vfs.read_view(None),
            &mut assets,
            input.native_source,
            &mut palettes,
        )?;
        let presentations = seat_presentations[..seat_count]
            .iter()
            .map(|rules| {
                qa_app::render_settings::world_profile(
                    runtime.vfs.read_view(None),
                    &mut assets,
                    &console.cvars,
                    rules.or(presentation_rules).unwrap_or(input.native_source),
                    &mut palettes,
                )
                .map(Into::into)
            })
            .collect::<Result<Vec<_>, _>>()?;
        let world_options = qa_app::render_settings::world_load_options(
            &console.cvars,
            runtime.renderer_configuration.as_deref(),
        )?;
        let loaded = input.load(
            &runtime.vfs,
            &mut assets,
            &mut runtime.geometry,
            source_palette,
            &presentations,
            world_options,
            false,
        )?;
        world_rate = qa_app::client_policy::tick_rate(policy.client, &mut console.cvars)?;
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
            qa_gameplay::rules::link_order(policy.client) == qa_world::area::LinkOrder::Head,
        );
        native_entity_source = Some(runtime.entity_sources.len());
        runtime.entity_sources.push(loaded.entity_source);
        runtime.server.area = qa_world::area::AreaGrid::load(
            runtime.server.entities.capacity(),
            loaded.collision_bounds,
        )
        .map_err(|e| format!("world area: {e:?}"))?;
        runtime.collision = Some(
            qa_app::WorldCollision::new(&runtime.geometry, loaded.collision, 0)
                .with_visibility(
                    assets
                        .world(loaded.render.world)
                        .ok_or("missing loaded world")?
                        .shared_visibility(),
                )
                .with_portals(loaded.portals),
        );
        if loaded.spawns.len() < seat_count {
            return Err(format!(
                "map has {} distinct spawn anchors for {seat_count} local seats",
                loaded.spawns.len()
            ));
        }
        for (index, slot) in local_clients.iter_mut().enumerate().take(seat_count) {
            let seat = SeatId::new(index as u8).ok_or("invalid local seat")?;
            let mut policy = seat_policies[index].unwrap_or(policy);
            policy.input = seat_input_rules[index].unwrap_or(policy.input);
            let rules = policy.movement;
            let traces = policy.trace;
            let spawn = loaded.spawns[index];
            let protocol = seat_protocols[index]
                .or(local_protocol)
                .unwrap_or_else(|| qa_network::commands::packet::Protocol::native(policy.client));
            let client = runtime.connect_local(seat, spawn, policy, protocol, index as u32)?;
            println!(
                "{{\"event\":\"local_channel\",\"seat\":{index},\"client\":{},\"protocol\":{},\"native_signon\":false}}",
                client.0,
                protocol.number()
            );
            let player = &runtime.server.clients[client.0 as usize].player;
            println!(
                "{{\"event\":\"map_loaded\",\"scope\":\"retail_map_walk_integration\",\"gameplay\":false,\"map\":{},\"movement\":\"{}\",\"trace_rules\":\"{}\",\"world\":{},\"client\":{},\"parsed_entities\":{},\"seat\":{index},\"spawned_clients\":{seat_count},\"module_entities_spawned\":0,\"collision_brushes\":{},\"spawn_entity\":{},\"spawn_fixture_fallback\":{},\"position\":{:?},\"angles\":{:?},\"mins\":{:?},\"maxs\":{:?},\"foreign_q1_box_limitation\":{},\"profile_consumed\":{},\"native_input_policy\":false}}",
                json(&loaded.virtual_path)?,
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
        native_map_checksums = Some(loaded.native_checksums);
        map_path = Some(loaded.virtual_path);
        loaded_world = Some(loaded.render);
    }
    let render_world = loaded_world.as_ref().map(|world| world.world);
    if auto_modules && map_path.is_some() {
        qa_app::modules::complete_stock_server(
            runtime.vfs.read_view(None),
            &console.cvars,
            stock_server_rules,
            stock_q3_stats,
            &mut quakec_modules,
            &mut q2_modules,
            &mut q3_modules,
        )?;
    }
    let mut module_requests = qa_app::modules::load_quakec(
        &mut runtime,
        &quakec_modules,
        map_path.as_deref().zip(native_entity_source),
    )?;
    let q2_clients = if !q2_modules.is_empty() {
        qa_app::modules::load_q2(
            &mut runtime,
            &q2_modules,
            &mut module_requests,
            map_path.as_deref().ok_or("Q2 module needs a loaded map")?,
            native_entity_source.ok_or("Q2 module needs a native entity source")?,
        )?
    } else {
        Vec::new()
    };
    if auto_modules {
        qa_app::modules::complete_q3_clients(
            &runtime,
            &console.cvars,
            &mut q3_modules,
            &local_clients,
        )?;
    }
    if !q3_modules.is_empty() {
        let rate = qa_app::client_policy::tick_rate(RuleSetId::Quake3, &mut console.cvars)?;
        qa_app::modules::load_q3(
            &mut runtime,
            &q3_modules,
            &mut module_requests,
            rate,
            native_entity_source,
        )?;
    }
    let module_count = module_requests.len();
    let model_sources: Vec<_> = module_requests
        .iter()
        .map(|request| request.context.native_rules)
        .collect();
    let mut host = FrameHost::load_modules(console, runtime, world_rate, module_requests)?;
    if let Some((game, skins)) = download_mounts {
        host.configure_download_roots(game, skins, 64 * 1024 * 1024, 8192)?;
    }
    if render_world.is_some() {
        host.set_renderer_fullscreen_ui(false);
    }
    host.local_clients = local_clients;
    host.local_presentation_rules = std::array::from_fn(|seat| {
        seat_presentations[seat].or(presentation_rules).or_else(|| {
            loaded_world
                .as_ref()?
                .presentations
                .get(seat)
                .map(|presentation| presentation.profile.rules)
        })
    });
    let startup_clients = host.local_clients.map(|id| {
        let id = id?;
        let row = &host.runtime.server.clients[id.0 as usize];
        Some((id, row.entity, row.client_rules))
    });
    match journal {
        Some(qa_content::journal::Mode::Recording(path)) => host
            .record_journal(&path)
            .map_err(|error| format!("record journal: {error:?}"))?,
        Some(qa_content::journal::Mode::Replay(path)) => host
            .replay_journal(&path)
            .map_err(|error| format!("play journal: {error:?}"))?,
        None => {}
    }
    for api in &q2_clients {
        host.register_q2_client_api(*api)?;
    }
    for (index, spec) in quakec_modules.iter().enumerate() {
        println!(
            "{{\"event\":\"module_loaded\",\"module\":{},\"file\":{},\"rules\":\"{}\",\"entry\":\"StartFrame\",\"scope\":\"map_spawn_and_frames\",\"gameplay\":false}}",
            index + 1,
            json(&spec.path)?,
            spec.rules.name()
        );
    }
    for (index, spec) in q3_modules.iter().enumerate() {
        println!(
            "{{\"event\":\"module_loaded\",\"module\":{},\"file\":{},\"rules\":\"q3\",\"role\":\"{:?}\",\"scope\":\"module_entries_only\",\"gameplay\":false}}",
            quakec_modules.len() + q2_modules.len() + index + 1,
            json(&spec.path)?,
            spec.role,
        );
    }
    for (index, spec) in q2_modules.iter().enumerate() {
        println!(
            "{{\"event\":\"module_loaded\",\"module\":{},\"file\":{},\"rules\":\"{}\",\"role\":\"game\",\"scope\":\"module_entries_only\",\"gameplay\":false}}",
            quakec_modules.len() + index + 1,
            json(&spec.path)?,
            spec.rules.name(),
        );
    }
    host.execute_commands_with_driver(&mut qa_app::modules::Driver::with_connector(
        None, &mut pump,
    ));
    // Synchronous native precache owns the same assets and metadata that CLIENT
    // will read after startup. Configure every module's independent profile first.
    let mut profiles = [None; 5];
    let mut seat_model_profiles = [[None; 5]; SeatId::COUNT];
    let mut configured_profiles = Vec::new();
    for source_rules in RuleSetId::ALL {
        if !model_sources.contains(&source_rules) {
            continue;
        }
        let source_palette = qa_app::render_settings::palette(
            host.runtime.vfs.read_view(None),
            &mut assets,
            source_rules,
            &mut palettes,
        )?;
        let default = qa_app::render_settings::world_profile(
            host.runtime.vfs.read_view(None),
            &mut assets,
            &host.console.cvars,
            presentation_rules.unwrap_or(source_rules),
            &mut palettes,
        )?;
        let local = loaded_world
            .as_ref()
            .into_iter()
            .flat_map(|world| world.presentations.iter().enumerate())
            .map(|(seat, presentation)| (Some(seat), presentation.profile));
        for (seat, presentation) in std::iter::once((None, default)).chain(local) {
            let profile = qa_app::model_assets::ModelProfile {
                source_rules,
                rules: presentation.rules,
                source_palette,
                palette: presentation.indexed_palette,
                image_settings: presentation.image_settings,
                finish: presentation.finish,
            };
            if let Some(seat) = seat {
                seat_model_profiles[seat][source_rules as usize] = Some(profile);
            } else {
                profiles[source_rules as usize] = Some(profile);
            }
            configured_profiles.push(profile);
        }
    }
    host.runtime.model_assets = Some(qa_app::model_assets::ModelAssets::new(
        &host.runtime.vfs,
        assets,
        configured_profiles,
        render_world,
        Default::default(),
    )?);
    host.refresh_resource_settings()?;
    host.runtime
        .model_assets
        .as_mut()
        .ok_or("missing model assets")?
        .adopt_palettes(palettes)?;
    if let Some(path) = map_path.as_deref() {
        let world = render_world.ok_or("loaded map needs a render world")?;
        let collision = host
            .runtime
            .collision
            .as_ref()
            .ok_or("loaded map needs collision")?;
        if collision.index != 0 {
            return Err("loaded map needs collision model0".to_owned());
        }
        let bounds = host
            .runtime
            .geometry
            .model_bounds(collision.geometry, collision.index)
            .ok_or("loaded map collision model0 bounds")?;
        host.runtime
            .model_assets
            .as_mut()
            .ok_or("missing model loader")?
            .bind_loaded_world(
                &host.runtime.vfs,
                path,
                world,
                Some(collision.geometry),
                bounds,
                native_map_checksums,
                None,
            )?;
    }
    let sound_rules = host
        .local_clients
        .iter()
        .flatten()
        .next()
        .map_or(client_module.unwrap_or(RuleSetId::Quake3), |id| {
            host.runtime.server.clients[id.0 as usize].client_rules
        });
    qa_app::audio::begin_registration(&mut host, &precache_sounds, sound_rules)?;
    // SERVER and CLIENT initialization register into the same load-owned bank.
    // Com_Frame reuses these once-only lifecycle entries on subsequent frames.
    if !host.runtime.quit {
        host.initialize_modules(qa_app::modules::Phase::Server);
        qa_app::output::flush_console(
            &mut host.runtime,
            &mut host.console.scrollback,
            &host.local_clients,
        );
        host.registration_pending(qa_app::modules::Phase::Server)?;
    }
    for (seat, original) in startup_clients.into_iter().enumerate() {
        let Some(id) = startup_server_client(&host, seat, original) else {
            continue;
        };
        // A sole provider is unambiguous. Combined providers require the
        // connection's explicit module number, independent of map/physics.
        let selected = seat_game_modules[seat].or_else(|| {
            let mut available = (1..=module_count).filter_map(|number| {
                let module = qa_core::primitives::ModuleId(number as u16);
                host.client_info(module).map(|_| module)
            });
            let only = available.next()?;
            available.next().is_none().then_some(only)
        });
        let Some(module) = selected else {
            continue;
        };
        let api = host
            .client_info(module)
            .ok_or("seat-game-module needs a loaded supported client lifecycle")?;
        let mut info = vec![0; api.userinfo_bytes];
        let length = host
            .console
            .cvars
            .info_string(
                Context {
                    source: api.rules,
                    seat: SeatId::new(seat as u8).ok_or("invalid local seat")?,
                    ..Context::default()
                },
                2,
                qa_compat::services::userinfo_rules(api.rules),
                &mut info,
            )
            .map_err(|e| format!("native userinfo: {e:?}"))?;
        let native_client = host.connect_module_client(module, id, &info[..length])?;
        println!(
            "{{\"event\":\"module_client_connected\",\"seat\":{seat},\"module\":{},\"native_client\":{native_client},\"native_signon\":false,\"gameplay_accepted\":false}}",
            module.0
        );
    }
    // Resolve initial SERVER model precaches at registration. CLIENT uses
    // received configstrings to bind assets at numeric native indices.
    let precaches: Vec<_> = host
        .model_precaches()
        .into_iter()
        .map(|(source, name, search)| (source, name.to_vec(), search.cloned()))
        .collect();
    for (source_rules, name, search) in precaches {
        let path =
            std::str::from_utf8(&name).map_err(|error| format!("model precache: {error}"))?;
        let profile = profiles[source_rules as usize].ok_or("missing model profile")?;
        host.runtime
            .model_assets
            .as_mut()
            .ok_or("missing model loader")?
            .register_request(
                &host.runtime.vfs,
                qa_app::model_assets::ModelRequest {
                    path,
                    search: search.as_ref(),
                    source_rules: profile.source_rules,
                    rules: profile.rules,
                    source_palette: profile.source_palette,
                    palette: profile.palette,
                    image_settings: profile.image_settings,
                    finish: profile.finish,
                    required: false,
                },
            )?;
    }
    let client_profiles: Vec<_> = host
        .client_model_sources()
        .into_iter()
        .filter_map(|(client, source)| {
            let seat = host.local_clients.iter().position(|id| *id == Some(client));
            let profile = seat.map_or(profiles[source as usize], |seat| {
                seat_model_profiles[seat][source as usize]
            })?;
            let world = seat.and_then(|seat| {
                if startup_server_client(&host, seat, startup_clients[seat]) != Some(client) {
                    return None;
                }
                let loaded = loaded_world.as_ref()?;
                Some((loaded.world, loaded.presentations.get(seat)?.id))
            });
            Some((client, profile, world))
        })
        .collect();
    qa_app::audio::load_registered_sounds(&mut host, &precache_sounds, sound_rules)?;
    let window_settings =
        qa_app::window_settings::startup(&mut host.console.cvars, renderer_kind, width, height)?;
    let mut window = renderer_kind.open(window_settings.request())?;
    let renderer_context = qa_app::renderer::Context::load(renderer_kind, &window)?;
    host.runtime.renderer_configuration =
        Some(std::sync::Arc::clone(renderer_context.configuration()));
    let width = i32::try_from(renderer_context.configuration().display.width)
        .map_err(|_| "renderer width")?;
    let height = i32::try_from(renderer_context.configuration().display.height)
        .map_err(|_| "renderer height")?;
    if let Some(world) = &loaded_world {
        for (seat, presentation) in world.presentations.iter().enumerate() {
            if startup_server_client(&host, seat, startup_clients[seat]).is_some() {
                host.local_worlds[seat] = Some((world.world, presentation.id));
                host.local_presentations[seat] = presentation.defaults;
            }
        }
    }
    host.configure_native_scenes(&profiles, &seat_model_profiles, width as u32, height as u32)?;
    host.register_client_model_assets(None);
    let models = host
        .runtime
        .model_assets
        .as_ref()
        .ok_or("missing model assets")?
        .catalog();
    let diagnostics: Vec<_> = models
        .diagnostics
        .iter()
        .map(|diagnostic| {
            format!(
                "model precache {}: {}\n",
                models.name(diagnostic.name).unwrap_or("<unknown>"),
                diagnostic.error
            )
        })
        .collect();
    for diagnostic in diagnostics {
        host.runtime.print_event(
            None,
            qa_core::primitives::PrintKind::Console,
            format_args!("{diagnostic}"),
        );
    }
    host.bind_client_model_assets(client_profiles);
    qa_app::audio::finish_registration(&mut host)?;
    let rate = host
        .runtime
        .sound_bank()
        .ok_or("sound assets unavailable")?
        .rate;
    let mut audio = qa_app::audio::Output::open(rate, &host.runtime.vfs)?;
    host.set_cpu_band_override(cpu_bands);
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
    let console_rules = host.console.cvars.context().source;
    let compile_policy = qa_app::render_settings::material_finish(&host.console.cvars)?
        .compile(host.renderer_fullscreen_ui());
    let owner = host
        .runtime
        .model_assets
        .as_mut()
        .ok_or("missing model assets")?;
    let console_assets = qa_app::renderer::console::ConsoleAssets::load(
        &host.runtime.vfs,
        owner,
        console_rules,
        qa_app::render_settings::image_settings(&host.console.cvars, console_rules)?,
        compile_policy,
        width as u32,
        height as u32,
    )?;
    let mut hud_assets = std::array::from_fn(|_| None);
    for rules in host
        .local_clients
        .iter()
        .flatten()
        .map(|id| host.runtime.server.clients[id.0 as usize].client_rules)
        .chain(startup_clients.iter().flatten().map(|(_, _, rules)| *rules))
    {
        if hud_assets[rules as usize].is_some() {
            continue;
        }
        let font = if rules == console_rules {
            console_assets.font_assets()
        } else {
            qa_app::renderer::console::FontAssets::load(
                &host.runtime.vfs,
                owner,
                rules,
                qa_app::render_settings::image_settings(&host.console.cvars, rules)?,
                compile_policy,
                None,
            )?
        };
        hud_assets[rules as usize] = Some(qa_app::renderer::hud::HudAssets::load(
            &host.runtime.vfs,
            owner,
            rules,
            font,
            width as u32,
        )?);
    }
    let effect_catalog = std::sync::Arc::clone(&host.runtime.catalog.registry.effects);
    let effect_bindings = vec![None; effect_catalog.len() + 1];
    let (assets, palettes) = owner.unpublished_assets()?;
    let _ = qa_app::render_settings::palette(
        host.runtime.vfs.read_view(None),
        assets,
        qa_core::primitives::RuleSetId::Quake,
        palettes,
    );
    let effects = qa_render::effects::Effects::load(
        effect_catalog,
        2048,
        host.runtime.server.clients.len(),
        &effect_bindings,
    )?;
    let model_choices = host
        .runtime
        .model_assets
        .as_ref()
        .ok_or("missing model assets")?
        .catalog()
        .max_choice_nodes();
    let (assets, graphics) = host.stage_engine_graphics(&console_assets, &hud_assets)?;
    let (mut renderer, scene) = Renderer::load(
        renderer_context,
        width as u32,
        height as u32,
        assets,
        loaded_world.into_iter().collect(),
        cpu_bands,
        console_assets,
        hud_assets,
        effects,
        model_choices,
    )?;
    graphics.commit();
    host.publish_model_admission(
        renderer
            .admitted_assets()
            .ok_or("missing renderer admission")?,
    )
    .map_err(|error| format!("renderer admission: {error:?}"))?;
    host.runtime.scene = Some(scene);
    window_settings.validate(&host.console.cvars)?;
    window_settings
        .apply(
            &mut host.console.cvars,
            &host
                .runtime
                .renderer_configuration
                .as_ref()
                .ok_or("missing renderer configuration")?
                .display,
        )
        .map_err(|error| format!("window latch publication: {error:?}"))?;
    window_settings.report_outcome(Some(window.request()), &mut host.runtime.server.events);
    // Startup commands can retire a seat before Com_Frame begins. Complete
    // those real Presentation records before a deferred connect may retry.
    host.dispatch_output(
        &mut LiveFrame {
            pump: &mut pump,
            window: &mut window,
            renderer: Some(&mut renderer),
            audio: Some(&mut audio),
        },
        &mut Default::default(),
    );
    if !host.runtime.quit {
        // Queued CLIENT startup may introduce a world and native precaches.
        // Their cold transactions use the same live backend as later frames.
        host.register_client_modules(
            &mut LiveFrame {
                pump: &mut pump,
                window: &mut window,
                renderer: Some(&mut renderer),
                audio: Some(&mut audio),
            },
            Duration::from_secs(10),
        )?;
    }
    if matches!(renderer_kind, Kind::Cpu) {
        host.console.cvars.set_latch_active(band_cvar, true);
    }
    host.capture_graphics_owners();
    let mut scene = host.runtime.scene.take().ok_or("missing scene composer")?;
    renderer.frame(
        &mut scene,
        host.runtime
            .model_assets
            .as_ref()
            .ok_or("missing model assets")?
            .view(),
        host.native_shaders(),
        &host.client_views(),
        &host.client_scene_inputs(),
        &host.client_hud_inputs(),
        qa_console::presentation::ConsoleView {
            editor: &host.console.editor,
            scrollback: &host.console.scrollback,
            texts: &host.runtime.server.events.texts,
            rules: console_rules,
            time: host.time,
        },
    );
    host.runtime.scene = Some(scene);
    #[cfg(any(debug_assertions, feature = "allocation-tracking"))]
    let _ = renderer.merge_worker_counts(qa_platform::allocations::Counts::default());
    renderer.present(&mut window);
    if !renderer.sample.presented {
        return Err("initial frame presentation failed".into());
    }
    if let Some(path) = &map_path {
        println!(
            "{{\"event\":\"world_frame_presented\",\"gameplay\":false,\"map\":{},\"renderer\":\"{}\",\"client_connected\":{},\"views\":{},\"visible_surfaces\":{},\"surfaces\":{},\"triangles\":{},\"rejected\":{},\"profile_consumed\":{},\"native_input_policy\":{}}}",
            json(path)?,
            renderer_kind.name(),
            host.local_clients[0].is_some(),
            renderer.sample.stats.views,
            renderer.sample.visible_surfaces,
            renderer.sample.stats.surfaces,
            renderer.sample.stats.triangles,
            renderer.sample.stats.rejected,
            imported_profile.consumed(),
            host.local_clients.iter().any(Option::is_some)
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
    let mut completed = 0u64;
    #[cfg(feature = "allocation-tracking")]
    let mut allocation_gate = allocation_gate::Gate::default();
    let mut samples = if timings {
        Vec::with_capacity(frames.unwrap_or(0) as usize)
    } else {
        Vec::new()
    };
    let mut stage_samples = if timings {
        Vec::with_capacity(frames.unwrap_or(0) as usize)
    } else {
        Vec::new()
    };
    let limit = frames.map(|count| u64::from(count) + u64::from(warmup));
    let mut frame = 0u64;
    loop {
        if limit.is_some_and(|limit| frame >= limit) {
            break;
        }
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
                renderer: Some(&mut renderer),
                audio: Some(&mut audio),
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
        for (seat, input) in host.client_hud_inputs().into_iter().enumerate() {
            let Some(input) = input else { continue };
            qa_console::logger::dev_print(
                &host.console.cvars,
                host.developer,
                1,
                format_args!(
                    "{{\"event\":\"hud_frame\",\"frame\":{frame},\"seat\":{seat},\"statusbar_bytes\":{},\"health\":{},\"frame_glyphs\":{},\"frame_pictures\":{},\"frame_missing_font\":{},\"frame_missing_picture\":{},\"frame_unsupported\":{},\"frame_rejected\":{}}}",
                    input
                        .layout
                        .statusbar()
                        .and_then(|index| input.configs.config(index))
                        .map_or(0, |text| text.text.len()),
                    input.stat(1).unwrap_or_default(),
                    renderer.sample.hud.glyphs,
                    renderer.sample.hud.pictures,
                    renderer.sample.hud.missing_font,
                    renderer.sample.hud.missing_picture,
                    renderer.sample.hud.unsupported,
                    renderer.sample.hud.rejected
                ),
            );
        }
        for (index, id) in host.local_clients.iter().enumerate() {
            let Some(id) = *id else { continue };
            let row = &host.runtime.server.clients[id.0 as usize];
            if !row.server_authoritative() {
                continue;
            }
            let authoritative = &row.player;
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
        frame += 1;
    }
    host.shutdown_modules_with_driver(&mut qa_app::modules::Driver::new(Some(&mut renderer)));
    host.flush_journal()
        .map_err(|error| format!("flush journal: {error:?}"))?;
    if let Some(error) = host.journal_error.as_ref() {
        return Err(format!("event journal: {error:?}"));
    }
    for index in 0..module_count {
        if let Some(counts) = host.module_counts(qa_core::primitives::ModuleId((index + 1) as u16))
        {
            println!(
                "{{\"event\":\"module_calls\",\"module\":{},\"calls\":{},\"traps\":{},\"rejected\":{},\"scope\":\"module_entries_only\",\"gameplay\":false}}",
                index + 1,
                counts.calls,
                counts.traps,
                counts.rejected
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
    println!(
        "{{\"event\":\"audio_output\",\"started\":{},\"stopped\":{},\"invalid\":{},\"full\":{},\"mixed_frames\":{},\"submissions\":{},\"blocked\":{}}}",
        audio.mixer.counts.started,
        audio.mixer.counts.stopped,
        audio.mixer.counts.invalid,
        audio.mixer.counts.full,
        audio.mixer.counts.frames,
        audio.submissions,
        audio.blocked,
    );
    println!(
        "{{\"event\":\"music_output\",\"started\":{},\"stopped\":{},\"loops\":{},\"invalid\":{},\"source_bytes\":{},\"queued_frames\":{}}}",
        audio.music.counts.started,
        audio.music.counts.stopped,
        audio.music.counts.loops,
        audio.music.counts.invalid,
        audio.music.counts.source_bytes,
        audio.music.counts.queued_frames,
    );
    Ok(())
}

fn main() {
    if let Some(status) = qa_platform::native::native_child_bootstrap() {
        std::process::exit(status);
    }
    if let Err(message) = run() {
        qa_console::logger::error(&message);
        std::process::exit(1);
    }
}
