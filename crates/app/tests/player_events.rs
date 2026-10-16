#[path = "support/service_program.rs"]
mod service_program;
#[path = "support/sound_registration.rs"]
mod sound_registration;

use qa_app::{
    Runtime,
    client_policy::ClientPolicy,
    host::{FrameHost, FrameSource},
    map::SpawnAnchor,
    modules::{Export, ModuleRequest, Phase, Program},
};
use qa_audio::{Bank, Mixer};
use qa_compat::{abi::Q3_SERVER, services::CallContext};
use qa_console::{commands::Console, views::Context};
use qa_core::{events::FrameEvent, loopback::Endpoint, primitives::*, sys_events::*};
use qa_formats::program::qvm::Opcode as Op;
use qa_network::commands::packet::Protocol;
use qa_session::timing::TickRate;
use qa_world::entities::AllocationPolicy;
use std::{num::NonZeroU32, time::Duration};

const MODULE: ModuleId = ModuleId(1);
const ENTITIES: i32 = 4096;
const PLAYERS: i32 = 16384;

static NEXT_MEDIA: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

struct MediaFiles(std::path::PathBuf);
impl MediaFiles {
    fn new(missing: &[&str]) -> Self {
        let root = std::env::temp_dir().join(format!(
            "qa-rust-player-event-media-{}-{}",
            std::process::id(),
            NEXT_MEDIA.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
        ));
        let wav = sound_registration::wav();
        for path in ["sound/feedback/hit.wav", "sound/one.wav", "sound/two.wav"]
            .into_iter()
            .chain(qa_gameplay::rules::feedback::PLAYER_SOUND_PATHS)
            .filter(|path| !missing.contains(path))
        {
            let path = root.join(path);
            qa_platform::files::create_dir_all(path.parent().unwrap()).unwrap();
            qa_platform::files::write(path, &wav).unwrap();
        }
        Self(root)
    }
}
impl Drop for MediaFiles {
    fn drop(&mut self) {
        let _ = qa_platform::files::remove_dir_all(&self.0);
    }
}

struct Source {
    mixer: Mixer,
    observations: Vec<(
        SoundId,
        i32,
        SoundBus,
        SpatialPolicy,
        Option<SoundPlayback>,
        Option<EntityId>,
        Option<SoundEmitter>,
        Option<u32>,
    )>,
    time: EventTime,
}
impl FrameSource for Source {
    fn begin_frame(&mut self) -> Duration {
        Duration::from_nanos(self.time.0)
    }
    fn poll_events(&mut self, queue: &mut SysEventQueue) {
        queue
            .push(SysEvent {
                time: self.time,
                kind: EventKind::Time,
            })
            .unwrap();
    }
    fn wait_time(&mut self, duration: Duration) -> Duration {
        duration
    }
    fn elapsed(&self) -> Duration {
        Duration::ZERO
    }
    fn present(&mut self) {}
    fn sound(&mut self, _bank: Option<&qa_audio::Bank>, event: SoundEvent) -> bool {
        self.observations.push((
            event.sound,
            event.channel,
            event.bus,
            event.spatial,
            event.playback,
            event.client_lifetime,
            event.entity,
            event.source_ordinal,
        ));
        self.mixer.sound(_bank.unwrap(), event)
    }
}

fn game() -> qa_compat::qvm::Vm {
    let mut code = vec![(Op::Enter, 64)];
    for export in [0, 10] {
        code.extend([
            (Op::Local, 72),
            (Op::Load4, 0),
            (Op::Const, export),
            (Op::Eq, 0),
        ]);
    }
    code.extend([(Op::Const, 0), (Op::Leave, 64)]);
    code[4].1 = code.len() as i32;
    for (arg, value) in [ENTITIES, 2, 608, PLAYERS, 512].into_iter().enumerate() {
        code.extend([(Op::Const, value), (Op::Arg, 8 + arg as i32 * 4)]);
    }
    code.extend([(Op::Const, -16), (Op::Call, 0), (Op::Pop, 0)]);
    let range = qa_core::configstrings::ConfigLayout::for_rules(RuleSetId::Quake3).resources[1];
    for (ordinal, address) in [(1, 128), (2, 160)] {
        code.extend([
            (Op::Const, (range.first + ordinal) as i32),
            (Op::Arg, 8),
            (Op::Const, address),
            (Op::Arg, 12),
            (Op::Const, -19),
            (Op::Call, 0),
            (Op::Pop, 0),
        ]);
    }
    code.extend([(Op::Const, 0), (Op::Leave, 64)]);
    code[8].1 = code.len() as i32;
    code.extend([
        (Op::Local, 76),
        (Op::Load4, 0),
        (Op::Local, 80),
        (Op::Load4, 0),
        (Op::Store4, 0),
        (Op::Const, 0),
        (Op::Leave, 64),
    ]);
    let mut data = vec![0; PLAYERS as usize + 1024];
    for (at, text) in [
        (128, b"sound/one.wav\0".as_slice()),
        (160, b"sound/two.wav\0"),
    ] {
        data[at..at + text.len()].copy_from_slice(text);
    }
    for row in 0..2 {
        let at = ENTITIES as usize + row * 608;
        data[at..at + 4].copy_from_slice(&(row as u32).to_le_bytes());
        data[at + 512..at + 516].copy_from_slice(&1023_u32.to_le_bytes());
    }
    for (offset, value) in [(4, 1_u32), (48, 800), (52, 320), (68, 1023), (184, 90)] {
        let at = PLAYERS as usize + offset;
        data[at..at + 4].copy_from_slice(&value.to_le_bytes());
    }
    let mut vm = service_program::program(&code, &data);
    vm.configure_q3_game(Q3StatLayout::BASE);
    vm
}

fn cgame() -> qa_compat::qvm::Vm {
    // Export zero is the ordinary frame. Export one selects an actually
    // received reliable command through CG_GETSERVERCOMMAND.
    service_program::program(
        &[
            (Op::Enter, 64),
            (Op::Local, 72),
            (Op::Load4, 0),
            (Op::Const, 1),
            (Op::Ne, 11),
            (Op::Local, 76),
            (Op::Load4, 0),
            (Op::Arg, 8),
            (Op::Const, -54),
            (Op::Call, 0),
            (Op::Pop, 0),
            (Op::Const, 0),
            (Op::Leave, 64),
        ],
        &[],
    )
}

fn fixture() -> (Box<FrameHost>, ClientId, Source) {
    fixture_owned(false)
}

fn fixture_owned(owned: bool) -> (Box<FrameHost>, ClientId, Source) {
    let rate = NonZeroU32::new(8000).unwrap();
    let bank = Bank::load(
        rate,
        ["sound/one.wav", "sound/two.wav"]
            .into_iter()
            .map(|path| {
                (
                    path.into(),
                    Pcm {
                        rate,
                        channels: PcmChannels::Mono,
                        samples: vec![12000; 128],
                        loop_start: None,
                        loop_end: None,
                        loop_invalid: false,
                    },
                )
            })
            .collect(),
    )
    .unwrap();
    let mut runtime = Runtime::load(2, []).unwrap();
    runtime.sound_assets = Some(Box::new(qa_app::audio::SoundAssets::from_bank(bank)));
    let client = runtime
        .connect_local(
            SeatId::FIRST,
            SpawnAnchor {
                position: Vec3::default(),
                angles: Vec3::default(),
                entity: 0,
                fixture_fallback: false,
            },
            ClientPolicy {
                input: RuleSetId::Quake3,
                client: RuleSetId::Quake3,
                movement: RuleSetId::QuakeWorld,
                trace: RuleSetId::Quake2,
            },
            Protocol::Quake3_68,
            0,
        )
        .unwrap();
    let context = Context {
        source: RuleSetId::Quake3,
        side: qa_console::catalog::Scope::Server,
        ..Default::default()
    };
    let mut console = Console::new(context).unwrap();
    console.append("sv_maxclients 2\n", context).unwrap();
    console.execute_frame(&mut runtime);
    let anchor = runtime
        .server
        .entities
        .allocate(ThinkTime::Milliseconds(0), MODULE, AllocationPolicy::q3(0))
        .unwrap()
        .id;
    let request = ModuleRequest {
        context: CallContext {
            module: MODULE,
            native_rules: RuleSetId::Quake3,
            trace_rules: RuleSetId::Quake2,
            sound_policy: qa_gameplay::rules::sound_policy(RuleSetId::Quake),
            clock: ThinkTime::Milliseconds(0),
            server_frame: 0,
            console: context,
            allocation: AllocationPolicy::q3(0),
            link_order: LinkOrder::Head,
        },
        timing_rules: RuleSetId::Quake3,
        phase: Phase::Server,
        rate: TickRate::fixed(50).unwrap(),
        anchor,
        program: Program::Qvm {
            vm: Box::new(game()),
            imports: &Q3_SERVER,
        },
        entries: (0..=10).collect(),
        frame: Export::clocked(CallbackId(8)),
        prepare: vec![],
        initialize: vec![Export::words(CallbackId(0), &[])],
        api: None,
        shutdown: vec![],
        instruction_budget: 2000,
        configstrings: qa_core::configstrings::ConfigLayout::for_rules(RuleSetId::Quake3).count,
        files: 0,
        entity_source: None,
    };
    let mut requests = vec![request];
    if owned {
        let mut client_context = requests[0].context;
        client_context.module = ModuleId(2);
        let anchor = runtime
            .server
            .entities
            .allocate(
                ThinkTime::Milliseconds(0),
                client_context.module,
                AllocationPolicy::q3(0),
            )
            .unwrap()
            .id;
        requests.push(ModuleRequest {
            context: client_context,
            timing_rules: RuleSetId::Quake3,
            phase: Phase::Client,
            rate: TickRate::FrameDriven,
            anchor,
            program: Program::Qvm {
                vm: Box::new(cgame()),
                imports: &qa_compat::abi::Q3_CLIENT,
            },
            entries: vec![0, 1],
            frame: Export {
                client: Some(client),
                ..Export::words(CallbackId(0), &[])
            },
            prepare: vec![],
            initialize: vec![],
            api: None,
            shutdown: vec![],
            instruction_budget: 1000,
            configstrings: 0,
            files: 0,
            entity_source: None,
        });
    }
    let mut host = Box::new(
        FrameHost::load_modules(console, runtime, TickRate::fixed(50).unwrap(), requests).unwrap(),
    );
    host.local_clients[0] = Some(client);
    host.initialize_modules(Phase::Server);
    assert_eq!(
        host.module_state(MODULE),
        Some(qa_app::modules::State::Running)
    );
    host.initialize_modules(Phase::Client);
    host.connect_module_client(MODULE, client, b"\\name\\events")
        .unwrap();
    host.bind_sounds().unwrap();
    let mut source = Source {
        mixer: Mixer::load(host.runtime.sound_bank().unwrap().rate, 16, SeatId::COUNT),
        observations: vec![],
        time: EventTime::from_milliseconds(100),
    };
    listen(&mut source, client);
    write(&mut host, 100, &[]);
    host.frame(&mut source, true);
    assert_eq!(host.module_counts(MODULE).unwrap().traps, 0);
    acknowledge_configs(&mut host, client, &mut source);
    let resources = &mut host.runtime.client_resources[client.0 as usize]
        .as_mut()
        .unwrap()
        .state;
    assert_eq!(
        resources.sound(1),
        host.runtime
            .sound_assets
            .as_deref()
            .unwrap()
            .bank()
            .find("sound/one.wav")
    );
    assert_eq!(
        resources.sound(2),
        host.runtime
            .sound_assets
            .as_deref()
            .unwrap()
            .bank()
            .find("sound/two.wav")
    );
    resources.set_sound_policy(
        qa_gameplay::rules::sound_policy(RuleSetId::Quake),
        qa_gameplay::rules::loop_spatial_policy(RuleSetId::Quake),
    );
    write(&mut host, 101, &[(108, 0), (128, 0)]);
    transmit(&mut host, client, &mut source, 101);
    due(&mut host, client, 101);
    host.publish_client_feedback();
    dispatch(&mut host, &mut source);
    source.observations.clear();
    (host, client, source)
}

fn listen(source: &mut Source, client: ClientId) {
    source.mixer.listen(
        &[
            Some(Listener {
                client,
                entity: Some(SoundEmitter::Native {
                    namespace: ResourceNamespace::Client(client),
                    slot: 0,
                }),
                origin: Vec3::default(),
                right: Vec3([0.0, 1.0, 0.0]),
                negate_dot: false,
            }),
            None,
            None,
            None,
        ],
        1.0,
    );
}

fn fixed_fixture(owned: bool, missing: &[&str]) -> (Box<FrameHost>, ClientId, Source, MediaFiles) {
    let (mut host, client, mut source) = fixture_owned(owned);
    let files = MediaFiles::new(missing);
    host.runtime.vfs.mount_directory(&files.0, 0).unwrap();
    // Exercise the actual cold registration call site before Bank freezes.
    host.runtime.sound_assets = None;
    qa_app::audio::begin_registration(&mut host, &[], RuleSetId::Quake3).unwrap();
    qa_app::audio::load_registered_sounds(&mut host, &[], RuleSetId::Quake3).unwrap();
    let counts = {
        let loader = host.runtime.sound_assets.as_ref().unwrap();
        (loader.reads, loader.decodes)
    };
    qa_app::audio::load_registered_sounds(&mut host, &[], RuleSetId::Quake3).unwrap();
    let loader = host.runtime.sound_assets.as_ref().unwrap();
    assert_eq!((loader.reads, loader.decodes), counts);
    qa_app::audio::finish_registration(&mut host).unwrap();
    source.mixer = Mixer::load(host.runtime.sound_bank().unwrap().rate, 16, SeatId::COUNT);
    listen(&mut source, client);
    acknowledge_configs(&mut host, client, &mut source);
    source.observations.clear();
    (host, client, source, files)
}

fn acknowledge_configs(host: &mut FrameHost, client: ClientId, source: &mut Source) {
    let milliseconds = host.time.milliseconds();
    for _ in 0..8 {
        dispatch(host, source);
        drain(host, milliseconds);
        assert!(host.runtime.send_client_command(
            client,
            &UserCmd::default(),
            EventTime::from_milliseconds(milliseconds),
            source
        ));
        drain(host, milliseconds);
    }
    dispatch(host, source);
}

fn dispatch(host: &mut FrameHost, source: &mut Source) -> qa_app::output::OutputCounts {
    qa_app::output::dispatch(
        &mut host.runtime,
        source,
        &mut host.console.scrollback,
        &host.local_clients,
        host.time,
        3.0,
        2.0,
    )
}
fn drain(host: &mut FrameHost, milliseconds: u64) {
    let time = EventTime::from_milliseconds(milliseconds);
    assert!(time >= host.time, "fixture realtime must remain monotonic");
    host.queue
        .push(SysEvent {
            time,
            kind: EventKind::Time,
        })
        .unwrap();
    host.drain(&mut Default::default());
    host.client_modules();
    if host.module_state(ModuleId(2)) == Some(qa_app::modules::State::Running) {
        let client = host.local_clients[0].unwrap();
        let channel = &host
            .runtime
            .network
            .get(client, Endpoint::Client)
            .unwrap()
            .channel;
        let received = channel.command_state().unwrap().received;
        let executed = channel.last_executed_server_command().unwrap();
        for number in executed + 1..=received {
            host.call_module_export(
                ModuleId(2),
                Export {
                    client: Some(client),
                    ..Export::words(CallbackId(1), &[u64::from(number)])
                },
                ThinkTime::Milliseconds(milliseconds as i64),
            )
            .unwrap();
        }
        assert_eq!(
            host.runtime
                .network
                .get(client, Endpoint::Client)
                .unwrap()
                .channel
                .last_executed_server_command()
                .unwrap(),
            received,
        );
    }
}
fn write(host: &mut FrameHost, native_ms: i64, fields: &[(i32, u32)]) {
    for &(offset, value) in fields {
        host.call_module_export(
            MODULE,
            Export::words(
                CallbackId(10),
                &[(PLAYERS + offset) as u64, u64::from(value)],
            ),
            ThinkTime::Milliseconds(native_ms),
        )
        .unwrap();
    }
    host.call_module_export(
        MODULE,
        Export::clocked(CallbackId(8)),
        ThinkTime::Milliseconds(native_ms),
    )
    .unwrap();
}
fn transmit(host: &mut FrameHost, client: ClientId, source: &mut Source, queued_ms: u64) {
    transmit_native(host, client, source, queued_ms, queued_ms);
}
fn transmit_native(
    host: &mut FrameHost,
    client: ClientId,
    source: &mut Source,
    native_ms: u64,
    queued_ms: u64,
) {
    let time = EventTime::from_milliseconds(native_ms);
    assert!(
        host.runtime
            .send_snapshot(client, time, None, source, |_, _| None, |_, _, _, _| None)
    );
    drain(host, queued_ms);
}
fn due(host: &mut FrameHost, client: ClientId, queued_ms: u64) {
    let mut queued_ms = queued_ms;
    // Q3 adjusts its delta after producing the current frame. Drive ordinary
    // packetless CLIENT frames until that native transition is actually due.
    for _ in 0..3 {
        drain(host, queued_ms);
        let channel = &host
            .runtime
            .network
            .get(client, Endpoint::Client)
            .unwrap()
            .channel;
        let current = channel.current_snapshot().unwrap().time();
        let snapshot = host.runtime.snapshots[client.0 as usize].as_mut().unwrap();
        snapshot
            .advance_presentation(
                channel,
                EventTime::from_milliseconds(queued_ms),
                Default::default(),
            )
            .unwrap();
        let presented = snapshot.interpolation().unwrap().time;
        if presented.milliseconds() >= current.milliseconds() {
            return;
        }
        queued_ms += u64::try_from(current.milliseconds() - presented.milliseconds()).unwrap();
    }
    panic!("CLIENT did not reach the retained native transition");
}
fn consume(host: &mut FrameHost, source: &mut Source) -> u64 {
    host.publish_client_feedback();
    dispatch(host, source).sounds
}

#[test]
fn native_events_cross_queued_snapshots_one_ring_and_real_pcm_with_independent_roles() {
    let (mut host, client, mut source) = fixture();
    write(&mut host, 200, &[(128, 45 | 0x100), (132, 1)]);
    transmit_native(&mut host, client, &mut source, 200, 150);
    // Receipt updates common raw state, but presentation has not reached this frame.
    assert_eq!(
        host.runtime.prediction[0]
            .player
            .native
            .unwrap()
            .signals
            .unwrap()
            .external
            .code,
        301
    );
    assert_eq!(consume(&mut host, &mut source), 0);
    due(&mut host, client, 200);
    assert_eq!(consume(&mut host, &mut source), 1);
    let (sound, channel, bus, spatial, playback, lifetime, emitter, _) = source.observations[0];
    assert_eq!(
        sound,
        host.runtime
            .sound_bank()
            .unwrap()
            .find("sound/one.wav")
            .unwrap()
    );
    assert_eq!((channel, bus), (3, SoundBus::Spatial));
    assert_eq!(spatial, qa_gameplay::rules::sound_policy(RuleSetId::Quake));
    assert_eq!(
        playback,
        qa_gameplay::rules::native_sound_playback(RuleSetId::Quake3)
    );
    assert_eq!(
        lifetime,
        Some(host.runtime.server.clients[client.0 as usize].entity)
    );
    assert_eq!(
        emitter,
        Some(SoundEmitter::Native {
            namespace: ResourceNamespace::Client(client),
            slot: 0
        })
    );
    let mut pcm = [0; 16];
    assert!(
        source
            .mixer
            .paint(host.runtime.sound_bank().unwrap(), &mut pcm)
    );
    assert!(pcm.iter().any(|&sample| sample != 0));
    assert_eq!(consume(&mut host, &mut source), 0);
    write(&mut host, 300, &[(128, 45 | 0x200), (132, 2)]);
    transmit(&mut host, client, &mut source, 300);
    due(&mut host, client, 300);
    assert_eq!(consume(&mut host, &mut source), 1);
    assert_eq!(
        source.observations[1].0,
        host.runtime
            .sound_bank()
            .unwrap()
            .find("sound/two.wav")
            .unwrap()
    );
    assert_eq!(
        host.runtime.prediction[0].player.movement_rules,
        RuleSetId::QuakeWorld
    );
    assert_eq!(
        host.runtime.prediction[0].player.trace_rules,
        RuleSetId::Quake2
    );
}

#[test]
fn predictable_corrections_compare_codes_and_preserve_external_first_order() {
    let (mut host, client, mut source) = fixture();
    write(
        &mut host,
        200,
        &[
            (108, 2),
            (112, 45),
            (120, 1),
            (116, 46),
            (124, 2),
            (128, 46),
            (132, 1),
        ],
    );
    transmit(&mut host, client, &mut source, 200);
    due(&mut host, client, 200);
    assert_eq!(consume(&mut host, &mut source), 3);
    assert_eq!(
        source
            .observations
            .iter()
            .map(|event| (event.1, event.7))
            .collect::<Vec<_>>(),
        vec![(0, Some(1)), (3, Some(1)), (0, Some(2))]
    );
    write(&mut host, 300, &[(120, 2), (124, 1), (132, 2)]);
    transmit(&mut host, client, &mut source, 300);
    due(&mut host, client, 300);
    assert_eq!(consume(&mut host, &mut source), 0);
    // Only i > oldSequence-2 is a corrected overlapping event in the native loop.
    write(&mut host, 400, &[(112, 46), (116, 45)]);
    transmit(&mut host, client, &mut source, 400);
    due(&mut host, client, 400);
    assert_eq!(consume(&mut host, &mut source), 1);
    assert_eq!(source.observations.last().unwrap().1, 3);
}

#[test]
fn unresolved_and_unsupported_events_drop_once_without_stalling_later_frames() {
    let (mut host, client, mut source) = fixture();
    write(
        &mut host,
        200,
        &[(108, 2), (112, 14), (120, 1), (116, 45), (124, 255)],
    );
    transmit(&mut host, client, &mut source, 200);
    due(&mut host, client, 200);
    assert_eq!(consume(&mut host, &mut source), 0);
    assert_eq!(
        host.runtime.snapshots[client.0 as usize]
            .as_ref()
            .unwrap()
            .player_event_drops(),
        2
    );
    assert_eq!(consume(&mut host, &mut source), 0);
    write(&mut host, 300, &[(108, 3), (112, 46), (120, 1)]);
    transmit(&mut host, client, &mut source, 300);
    due(&mut host, client, 300);
    assert_eq!(consume(&mut host, &mut source), 1);
    assert_eq!(
        host.runtime.snapshots[client.0 as usize]
            .as_ref()
            .unwrap()
            .player_event_drops(),
        2
    );
}

#[test]
fn full_presentation_retries_the_retained_transition_in_external_then_predictable_order() {
    let (mut host, client, mut source) = fixture();
    write(
        &mut host,
        200,
        &[
            (108, 2),
            (112, 45),
            (120, 1),
            (116, 46),
            (124, 2),
            (128, 46),
            (132, 1),
        ],
    );
    transmit(&mut host, client, &mut source, 200);
    due(&mut host, client, 200);
    let presentation = host.runtime.server.presentation;
    let capacity = host.runtime.server.events.capacity();
    for _ in 0..capacity {
        host.runtime
            .server
            .events
            .push_to(
                qa_core::events::OutputTarget::Presentation,
                FrameEvent::Effect(EffectEvent::default()),
            )
            .unwrap();
    }
    let rejected = host.runtime.server.events.rejected_publications();
    host.publish_client_feedback();
    assert!(host.runtime.server.events.needs_resync(presentation));
    assert!(host.runtime.server.events.rejected_publications() > rejected);
    assert_eq!(dispatch(&mut host, &mut source).sounds, 0);
    assert_eq!(consume(&mut host, &mut source), 3);
    assert_eq!(
        source
            .observations
            .iter()
            .map(|event| event.1)
            .collect::<Vec<_>>(),
        vec![0, 3, 0]
    );
    assert_eq!(consume(&mut host, &mut source), 0);
}

#[test]
fn follow_and_reused_connection_lifetimes_baseline_without_replaying_old_events() {
    let (mut host, client, mut source) = fixture();
    write(
        &mut host,
        200,
        &[
            (140, 1),
            (108, 2),
            (112, 45),
            (120, 1),
            (116, 46),
            (124, 2),
            (128, 45),
            (132, 1),
        ],
    );
    transmit(&mut host, client, &mut source, 200);
    due(&mut host, client, 200);
    assert_eq!(consume(&mut host, &mut source), 0);
    write(&mut host, 300, &[(128, 45 | 0x100), (132, 2)]);
    transmit(&mut host, client, &mut source, 300);
    due(&mut host, client, 300);
    assert_eq!(consume(&mut host, &mut source), 1);
    let old = host.runtime.server.clients[client.0 as usize].entity;
    assert!(host.runtime.disconnect_client(client));
    dispatch(&mut host, &mut source);
    let replacement = host
        .runtime
        .connect_local(
            SeatId::FIRST,
            SpawnAnchor {
                position: Vec3::default(),
                angles: Vec3::default(),
                entity: 0,
                fixture_fallback: false,
            },
            ClientPolicy {
                input: RuleSetId::Quake3,
                client: RuleSetId::Quake3,
                movement: RuleSetId::QuakeWorld,
                trace: RuleSetId::Quake2,
            },
            Protocol::Quake3_68,
            0,
        )
        .unwrap();
    assert_eq!(replacement, client);
    assert_ne!(host.runtime.server.clients[client.0 as usize].entity, old);
    assert_eq!(consume(&mut host, &mut source), 0);
    let mut pcm = [1; 16];
    assert!(
        source
            .mixer
            .paint(host.runtime.sound_bank().unwrap(), &mut pcm)
    );
    assert!(pcm.iter().all(|&sample| sample == 0));
}

#[test]
fn native_audio_ownership_suppresses_fallback_and_release_never_replays_suppressed_cues() {
    let (mut host, client, mut source) = fixture_owned(true);
    write(
        &mut host,
        200,
        &[
            (108, 2),
            (112, 45),
            (120, 1),
            (116, 46),
            (124, 2),
            (128, 45),
            (132, 1),
        ],
    );
    transmit(&mut host, client, &mut source, 200);
    due(&mut host, client, 200);
    assert_eq!(consume(&mut host, &mut source), 0);
    assert!(source.observations.is_empty());
    assert!(host.shutdown_module(ModuleId(2)));
    for _ in 0..3 {
        host.publish_client_loops();
        dispatch(&mut host, &mut source);
    }
    source.observations.clear();
    assert_eq!(consume(&mut host, &mut source), 0);
    write(&mut host, 300, &[(128, 45 | 0x100), (132, 2)]);
    transmit(&mut host, client, &mut source, 300);
    due(&mut host, client, 300);
    assert_eq!(consume(&mut host, &mut source), 1);
    assert_eq!(source.observations[0].7, Some(2));
}

#[test]
fn fixed_native_media_cross_the_queued_channel_and_mixer_with_independent_sound_roles() {
    // Literal CG_EntityEvent expectations, not the production media table.
    let recipes = [
        (10, "sound/player/land1.wav", 0),
        (15, "sound/player/watr_in.wav", 0),
        (16, "sound/player/watr_out.wav", 0),
        (17, "sound/player/watr_un.wav", 0),
        (22, "sound/weapons/change.wav", 0),
        (24, "sound/items/use_nothing.wav", 5),
        (26, "sound/items/use_medkit.wav", 5),
        (30, "sound/items/use_nothing.wav", 5),
        (31, "sound/items/use_nothing.wav", 5),
        (32, "sound/items/use_nothing.wav", 5),
        (33, "sound/items/use_nothing.wav", 5),
        (34, "sound/items/use_nothing.wav", 5),
        (35, "sound/items/use_nothing.wav", 5),
        (36, "sound/items/use_nothing.wav", 5),
        (37, "sound/items/use_nothing.wav", 5),
        (38, "sound/items/use_nothing.wav", 5),
        (40, "sound/items/respawn1.wav", 0),
        (41, "sound/items/respawn1.wav", 0),
        (42, "sound/world/telein.wav", 0),
        (43, "sound/world/teleout.wav", 0),
        (61, "sound/items/damage3.wav", 4),
        (62, "sound/items/protect3.wav", 4),
        (63, "sound/items/regen.wav", 4),
    ];
    for sound_role in [RuleSetId::Quake3, RuleSetId::Quake2] {
        let (mut host, client, mut source, _files) = fixed_fixture(false, &[]);
        let spatial = qa_gameplay::rules::sound_policy(sound_role);
        host.runtime.client_resources[client.0 as usize]
            .as_mut()
            .unwrap()
            .state
            .set_sound_policy(spatial, qa_gameplay::rules::loop_spatial_policy(sound_role));
        for (index, (code, path, expected_channel)) in recipes.into_iter().enumerate() {
            let milliseconds = 300 + index as u64 * 100;
            // Fixed recipes ignore eventParm, even when it is not a sound ordinal.
            write(
                &mut host,
                milliseconds as i64,
                &[(128, code | 0x100), (132, 255)],
            );
            transmit_native(
                &mut host,
                client,
                &mut source,
                milliseconds,
                milliseconds - 50,
            );
            assert_eq!(consume(&mut host, &mut source), 0);
            due(&mut host, client, milliseconds);
            assert_eq!(consume(&mut host, &mut source), 1);
            let event = source.observations.last().unwrap();
            assert_eq!(
                event.0,
                host.runtime.sound_bank().unwrap().find(path).unwrap()
            );
            assert_eq!((event.1, event.2), (expected_channel, SoundBus::Spatial));
            assert_eq!(event.3, spatial);
            assert_eq!(
                event.4,
                qa_gameplay::rules::native_sound_playback(RuleSetId::Quake3),
            );
            assert_eq!(
                event.7,
                Some(
                    host.runtime
                        .sound_bank()
                        .unwrap()
                        .registered_handle(path.as_bytes())
                        .unwrap() as u32
                ),
            );
            assert_eq!(
                event.6,
                Some(SoundEmitter::Native {
                    namespace: ResourceNamespace::Client(client),
                    slot: 0,
                }),
            );
            let mut pcm = [0; 16];
            assert!(
                source
                    .mixer
                    .paint(host.runtime.sound_bank().unwrap(), &mut pcm)
            );
            assert!(pcm.iter().any(|&sample| sample != 0));
            assert_eq!(consume(&mut host, &mut source), 0);
        }
        assert_eq!(
            host.runtime.prediction[0].player.movement_rules,
            RuleSetId::QuakeWorld,
        );
        assert_eq!(
            host.runtime.prediction[0].player.trace_rules,
            RuleSetId::Quake2,
        );
        assert_eq!(
            host.runtime.snapshots[client.0 as usize]
                .as_ref()
                .unwrap()
                .player_event_drops(),
            0,
        );
    }
}

#[test]
fn fixed_media_retry_keeps_external_then_predictable_order_without_repeating() {
    let (mut host, client, mut source, _files) = fixed_fixture(false, &[]);
    write(
        &mut host,
        300,
        &[
            (108, 2),
            (112, 15),
            (116, 61),
            (128, 10 | 0x100),
            (132, 255),
        ],
    );
    transmit(&mut host, client, &mut source, 300);
    due(&mut host, client, 300);
    let capacity = host.runtime.server.events.capacity();
    for _ in 0..capacity {
        host.runtime
            .server
            .events
            .push_to(
                qa_core::events::OutputTarget::Presentation,
                FrameEvent::Effect(EffectEvent::default()),
            )
            .unwrap();
    }
    host.publish_client_feedback();
    assert_eq!(dispatch(&mut host, &mut source).sounds, 0);
    assert_eq!(consume(&mut host, &mut source), 3);
    assert_eq!(
        source
            .observations
            .iter()
            .map(|event| (event.0, event.1))
            .collect::<Vec<_>>(),
        vec![
            (
                host.runtime
                    .sound_bank()
                    .unwrap()
                    .find("sound/player/land1.wav")
                    .unwrap(),
                0
            ),
            (
                host.runtime
                    .sound_bank()
                    .unwrap()
                    .find("sound/player/watr_in.wav")
                    .unwrap(),
                0
            ),
            (
                host.runtime
                    .sound_bank()
                    .unwrap()
                    .find("sound/items/damage3.wav")
                    .unwrap(),
                4
            ),
        ],
    );
    assert_eq!(consume(&mut host, &mut source), 0);
    let mut pcm = [0; 16];
    assert!(
        source
            .mixer
            .paint(host.runtime.sound_bank().unwrap(), &mut pcm)
    );
    assert!(pcm.iter().any(|&sample| sample != 0));
}

#[test]
fn missing_fixed_registration_uses_native_zero_without_reinterpreting_config_zero() {
    let (mut host, client, mut source, _files) = fixed_fixture(false, &["sound/player/land1.wav"]);
    assert_eq!(
        host.runtime
            .sound_bank()
            .unwrap()
            .find("sound/player/land1.wav"),
        None
    );
    assert_eq!(
        host.runtime
            .sound_bank()
            .unwrap()
            .registered_handle(b"sound/player/land1.wav"),
        Some(0)
    );
    assert_eq!(
        host.runtime.client_resources[client.0 as usize]
            .as_ref()
            .unwrap()
            .state
            .sound(0),
        None,
    );
    write(&mut host, 300, &[(128, 10), (132, 0)]);
    transmit(&mut host, client, &mut source, 300);
    due(&mut host, client, 300);
    assert_eq!(consume(&mut host, &mut source), 1);
    assert_eq!(
        source.observations[0].0,
        host.runtime
            .sound_bank()
            .unwrap()
            .registered_sound(0)
            .unwrap()
    );
    assert_eq!(source.observations[0].7, Some(0));
    let mut pcm = [0; 16];
    assert!(
        source
            .mixer
            .paint(host.runtime.sound_bank().unwrap(), &mut pcm)
    );
    assert!(pcm.iter().any(|&sample| sample != 0));
}

#[test]
fn missing_fixed_and_default_pcm_drops_once_while_later_fixed_media_continue() {
    let (mut host, client, mut source, _files) =
        fixed_fixture(false, &["sound/player/land1.wav", "sound/feedback/hit.wav"]);
    write(&mut host, 300, &[(128, 10)]);
    transmit(&mut host, client, &mut source, 300);
    due(&mut host, client, 300);
    assert_eq!(consume(&mut host, &mut source), 0);
    assert_eq!(consume(&mut host, &mut source), 0);
    assert_eq!(
        host.runtime.snapshots[client.0 as usize]
            .as_ref()
            .unwrap()
            .player_event_drops(),
        1,
    );
    write(&mut host, 400, &[(128, 15)]);
    transmit(&mut host, client, &mut source, 400);
    due(&mut host, client, 400);
    assert_eq!(consume(&mut host, &mut source), 1);
}

#[test]
fn native_cgame_owns_fixed_media_and_release_establishes_a_new_event_baseline() {
    let (mut host, client, mut source, _files) = fixed_fixture(true, &[]);
    write(&mut host, 300, &[(128, 10)]);
    transmit(&mut host, client, &mut source, 300);
    due(&mut host, client, 300);
    assert_eq!(consume(&mut host, &mut source), 0);
    assert!(host.shutdown_module(ModuleId(2)));
    for _ in 0..3 {
        host.publish_client_loops();
        dispatch(&mut host, &mut source);
    }
    source.observations.clear();
    assert_eq!(consume(&mut host, &mut source), 0);
    write(&mut host, 400, &[(128, 15)]);
    transmit(&mut host, client, &mut source, 400);
    due(&mut host, client, 400);
    assert_eq!(consume(&mut host, &mut source), 1);
    assert_eq!(
        source.observations[0].0,
        host.runtime
            .sound_bank()
            .unwrap()
            .find("sound/player/watr_in.wav")
            .unwrap(),
    );
}
