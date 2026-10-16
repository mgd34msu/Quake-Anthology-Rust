#[path = "support/service_program.rs"]
mod service_program;

use qa_app::{
    Runtime,
    client_policy::ClientPolicy,
    host::{FrameHost, FrameSource},
    map::SpawnAnchor,
    modules::{Export, ModuleRequest, Phase, Program},
};
use qa_audio::{Bank, Mixer};
use qa_compat::{
    abi::{CallTable, Q3_CLIENT, Q3_UI},
    services::CallContext,
};
use qa_console::{commands::Console, views::Context};
use qa_core::{
    loopback::Endpoint,
    primitives::*,
    sys_events::{EventKind, EventTime, SeatId, SysEvent, SysEventQueue},
};
use qa_formats::program::qvm::Opcode::*;
use qa_session::timing::TickRate;
use qa_world::entities::AllocationPolicy;
use std::{num::NonZeroU32, time::Duration};

struct Source {
    mixer: Mixer,
    time: EventTime,
    last_sound: Option<SoundEvent>,
    last_loop: Option<SoundEvent>,
    pcm: [i16; 8],
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
    fn wait_time(&mut self, remaining: Duration) -> Duration {
        self.time.0 += remaining.as_nanos() as u64;
        Duration::from_nanos(self.time.0)
    }
    fn elapsed(&self) -> Duration {
        Duration::ZERO
    }
    fn present(&mut self) {}
    fn listeners(&mut self, listeners: &[Option<Listener>; SeatId::COUNT], volume: f32) {
        self.mixer.listen(listeners, volume);
    }
    fn sound(&mut self, _bank: Option<&qa_audio::Bank>, event: SoundEvent) -> bool {
        self.last_sound = Some(event);
        if event.action == SoundAction::StartLoop {
            self.last_loop = Some(event);
        }
        self.mixer.sound(_bank.unwrap(), event)
    }
    fn mix_audio(
        &mut self,
        _bank: Option<&qa_audio::Bank>,
        entities: &[Option<qa_app::client_scene::ClientEntities<'_>>; SeatId::COUNT],
    ) {
        qa_app::audio::respatialize(&mut self.mixer, entities);
        assert!(self.mixer.paint(_bank.unwrap(), &mut self.pcm));
    }
}

fn request(
    runtime: &mut Runtime,
    module: ModuleId,
    table: &'static CallTable,
    client: Option<ClientId>,
) -> ModuleRequest {
    let anchor = runtime
        .server
        .entities
        .allocate(ThinkTime::Milliseconds(0), module, AllocationPolicy::q3(0))
        .unwrap()
        .id;
    // vmMain receives an import ordinal and seven words. One callback invokes
    // that import; the frame callback is an independent no-op.
    let mut operations = vec![(Enter, 64), (Local, 72), (Load4, 0), (Const, 0), (Ne, 0)];
    for argument in 0..7 {
        operations.extend([
            (Local, 80 + argument * 4),
            (Load4, 0),
            (Arg, 8 + argument * 4),
        ]);
    }
    operations.extend([
        (Const, -1),
        (Local, 76),
        (Load4, 0),
        (Sub, 0),
        (Call, 0),
        (Leave, 64),
    ]);
    let frame = operations.len() as u32;
    operations[4].1 = frame as i32;
    operations.extend([(Const, 0), (Leave, 64)]);
    let mut data = [0; 256];
    for (offset, vector) in [
        (16, [0.0_f32, 0.0, 0.0]),
        (32, [1000.0, 0.0, 0.0]),
        (48, [0.0, 0.0, 0.0]),
        (64, [1.0, 0.0, 0.0]),
        (76, [0.0, 1.0, 0.0]),
        (88, [0.0, 0.0, 1.0]),
        (112, [1.0, 0.0, 0.0]),
        (128, [99.0, 0.0, 0.0]),
    ] {
        for (axis, value) in vector.into_iter().enumerate() {
            let at = offset + axis * 4;
            data[at..at + 4].copy_from_slice(&value.to_le_bytes());
        }
    }
    ModuleRequest {
        context: CallContext {
            module,
            native_rules: RuleSetId::Quake3,
            trace_rules: RuleSetId::QuakeWorld,
            sound_policy: qa_gameplay::rules::native_sound_spatial(RuleSetId::Quake3),
            clock: ThinkTime::Milliseconds(0),
            server_frame: 0,
            console: Context::default(),
            allocation: AllocationPolicy::q3(0),
            link_order: LinkOrder::Head,
        },
        phase: Phase::Client,
        timing_rules: RuleSetId::Quake3,
        rate: TickRate::FrameDriven,
        anchor,
        program: Program::Qvm {
            vm: Box::new(service_program::program(&operations, &data)),
            imports: table,
        },
        entries: vec![0, 1],
        frame: Export {
            client,
            ..Export::words(CallbackId(1), &[])
        },
        prepare: Vec::new(),
        initialize: Vec::new(),
        api: None,
        shutdown: Vec::new(),
        instruction_budget: 1000,
        configstrings: 0,
        files: 0,
        entity_source: None,
    }
}

fn fixture(
    protocol: Option<qa_network::commands::packet::Protocol>,
) -> (Box<FrameHost>, Source, i32) {
    let rate = NonZeroU32::new(44100).unwrap();
    let bank = Bank::load(
        rate,
        vec![(
            "sound/steady.wav".into(),
            Pcm {
                rate,
                channels: PcmChannels::Mono,
                samples: vec![3500; 256],
                loop_start: None,
                loop_end: None,
                loop_invalid: false,
            },
        )],
    )
    .unwrap();
    let handle = bank.registered_handle(b"sound/steady.wav").unwrap();
    let mut runtime = Runtime::load(1, []).unwrap();
    runtime.sound_assets = Some(Box::new(qa_app::audio::SoundAssets::from_bank(bank)));
    let client = protocol.map(|protocol| {
        runtime
            .connect_local(
                SeatId::FIRST,
                SpawnAnchor {
                    position: Vec3::default(),
                    angles: Vec3::default(),
                    entity: 0,
                    fixture_fallback: false,
                },
                ClientPolicy {
                    input: RuleSetId::Quake2,
                    client: RuleSetId::Quake2,
                    movement: RuleSetId::Quake,
                    trace: RuleSetId::QuakeWorld,
                },
                protocol,
                0,
            )
            .unwrap()
    });
    let imports = if client.is_some() { &Q3_CLIENT } else { &Q3_UI };
    let module = request(&mut runtime, ModuleId(1), imports, client);
    let mut console = Console::new(Context::default()).unwrap();
    let volume = console.cvars.bind("s_volume", Context::default()).unwrap();
    console.cvars.force_write(volume, "1").unwrap();
    let mut host =
        FrameHost::load_modules(console, runtime, TickRate::FrameDriven, vec![module]).unwrap();
    host.local_clients[0] = client;
    (
        Box::new(host),
        Source {
            mixer: Mixer::load(rate, 16, SeatId::COUNT),
            time: EventTime::from_milliseconds(100),
            last_sound: None,
            last_loop: None,
            pcm: [0; 8],
        },
        handle,
    )
}
fn call(host: &mut FrameHost, ordinal: u32, arguments: &[u64]) {
    let mut words = vec![u64::from(ordinal)];
    words.extend_from_slice(arguments);
    host.call_module_export(
        ModuleId(1),
        Export::words(CallbackId(0), &words),
        ThinkTime::Milliseconds(777_000),
    )
    .unwrap();
}
fn frame(host: &mut FrameHost, source: &mut Source) {
    source.time.0 += 100_000_000;
    host.frame(source, true);
}

fn damage_snapshot(host: &mut FrameHost, source: &mut Source, sequence: u32) {
    let client = host.local_clients[0].unwrap();
    let module = ModuleId(99);
    assert!(
        host.runtime
            .server
            .bind_client_native(client, NativeEntity { module, slot: 0 })
    );
    source.time = EventTime::from_milliseconds(u64::from(sequence) * 100);
    let player = &mut host.runtime.server.clients[client.0 as usize].player;
    player.health = 90;
    let mut tail = NativePlayerTail::new(RuleSetId::Quake3, ResourceNamespace::Module(module));
    tail.client_number = Some(0);
    tail.signals = Some(PlayerSignals {
        damage: PlayerDamageSample {
            sequence,
            yaw: 255,
            pitch: 255,
            count: 20,
        },
        ..Default::default()
    });
    player.native = Some(tail);
    player.native_clock = Some(ClockObservation {
        native: ThinkTime::Milliseconds(1000 + i64::from(sequence) * 100),
        queued: source.time,
    });
    assert!(host.runtime.send_snapshot(
        client,
        source.time,
        None,
        source,
        |_, _| None,
        |_, _, _, _| None
    ));
    host.queue
        .push(SysEvent {
            time: source.time,
            kind: EventKind::Time,
        })
        .unwrap();
    host.drain(&mut Default::default());
    advance_damage_view(host, source.time);
    host.publish_client_feedback();
}

fn advance_damage_view(host: &mut FrameHost, time: EventTime) {
    let client = host.local_clients[0].unwrap();
    let channel = &host
        .runtime
        .network
        .get(client, Endpoint::Client)
        .unwrap()
        .channel;
    host.runtime.snapshots[client.0 as usize]
        .as_mut()
        .unwrap()
        .advance_presentation(channel, time, Default::default())
        .unwrap();
}

#[test]
fn native_cgame_view_ownership_suppresses_fallback_and_release_baselines_without_replay() {
    assert!(Q3_CLIENT.produces_client_view());
    assert!(!Q3_UI.produces_client_view());
    assert!(!qa_compat::abi::Q3_SERVER.produces_client_view());
    let (mut host, mut source, _) =
        fixture(Some(qa_network::commands::packet::Protocol::Quake3_68));
    host.initialize_modules(Phase::Client);
    let client = host.local_clients[0].unwrap();
    let before = host.runtime.server.events.sequence_range().end;
    damage_snapshot(&mut host, &mut source, 1);
    damage_snapshot(&mut host, &mut source, 2);
    assert!(
        host.runtime.server.clients[client.0 as usize]
            .hud
            .view_feedback
            .is_none()
    );
    assert_eq!(host.runtime.server.events.sequence_range().end, before);
    // Actual module shutdown releases the view; retained suppressed frames
    // establish its baseline rather than replaying historical damage.
    assert!(host.shutdown_module(ModuleId(1)));
    let released = host.runtime.server.events.sequence_range().end;
    host.publish_client_feedback();
    assert_eq!(host.runtime.server.events.sequence_range().end, released);
    damage_snapshot(&mut host, &mut source, 3);
    damage_snapshot(&mut host, &mut source, 4);
    let snapshot = host.runtime.snapshots[client.0 as usize].as_ref().unwrap();
    assert_eq!(
        snapshot.interpolation().unwrap().time,
        ThinkTime::Milliseconds(1396)
    );
    // Native drift leaves the fourth transition in the future. A packetless
    // CLIENT advance makes it due without replaying the suppressed baseline.
    source.time = EventTime::from_milliseconds(500);
    host.queue
        .push(SysEvent {
            time: source.time,
            kind: EventKind::Time,
        })
        .unwrap();
    host.drain(&mut Default::default());
    advance_damage_view(&mut host, source.time);
    host.publish_client_feedback();
    let time = host.time;
    let counts = qa_app::output::dispatch(
        &mut host.runtime,
        &mut source,
        &mut host.console.scrollback,
        &host.local_clients,
        time,
        3.0,
        2.0,
    );
    assert_eq!(counts.view_feedback, 2);
}

fn listener(host: &mut FrameHost) {
    call(host, 33, &[0, 16, 64, 0]);
}
fn expected(master: i64) -> i16 {
    ((3500 * master * 255 >> 8) >> 8) as i16
}

#[test]
fn unbound_ui_local_sound_uses_the_ring_and_needs_no_fake_listener() {
    let (mut host, mut source, handle) = fixture(None);
    call(&mut host, 32, &[handle as u64, 7]);
    frame(&mut host, &mut source);
    assert_eq!(source.pcm, [expected(127); 8]);
    let event = source.last_sound.unwrap();
    assert_eq!(event.action, SoundAction::Play);
    assert_eq!(event.bus, SoundBus::Local);
    assert!(event.client.is_none());
    assert!(event.client_lifetime.is_none());
    assert!(event.entity.is_none());
    assert_eq!(event.queued_at, EventTime::default());
}

#[test]
fn cgame_loops_latch_clear_and_release_through_the_shared_host() {
    for protocol in [
        qa_network::commands::packet::Protocol::Quake3_68,
        qa_network::commands::packet::Protocol::Quake2_34,
        qa_network::commands::packet::Protocol::NetQuake15,
    ] {
        let (mut host, mut source, handle) = fixture(Some(protocol));
        frame(&mut host, &mut source);
        assert_eq!(source.pcm, [0; 8]);
        call(&mut host, 31, &[1, 16, 48, handle as u64]);
        call(&mut host, 80, &[2, 16, 48, handle as u64]);
        frame(&mut host, &mut source);
        assert_eq!(source.pcm, [0; 8], "Add must not rebuild the audible latch");
        listener(&mut host);
        frame(&mut host, &mut source);
        assert_eq!(source.pcm, [expected(108); 8]);
        host.runtime.prediction[0].player.body.position = Vec3([1_000_000.0, 0.0, 0.0]);
        frame(&mut host, &mut source);
        assert_eq!(
            source.pcm,
            [expected(108); 8],
            "host camera must not overwrite native listener"
        );
        call(&mut host, 30, &[0]);
        frame(&mut host, &mut source);
        assert_eq!(
            source.pcm, [0; 8],
            "Clear(false) mutes real loops until Respatialize too"
        );
        listener(&mut host);
        frame(&mut host, &mut source);
        assert_eq!(source.pcm, [expected(45); 8]);
        call(&mut host, 81, &[2]);
        frame(&mut host, &mut source);
        assert_eq!(
            source.pcm,
            [expected(45); 8],
            "Stop changes registration before the latch"
        );
        listener(&mut host);
        frame(&mut host, &mut source);
        assert_eq!(source.pcm, [0; 8]);
        call(&mut host, 80, &[2, 16, 48, handle as u64]);
        listener(&mut host);
        frame(&mut host, &mut source);
        assert_eq!(source.pcm, [expected(45); 8]);
        assert!(host.shutdown_module(ModuleId(1)));
        for _ in 0..3 {
            frame(&mut host, &mut source);
        }
        assert_eq!(source.pcm, [0; 8]);
        assert!(source.mixer.counts.stopped >= 2);
    }
}

#[test]
fn native_loop_velocity_reaches_the_ring_and_mixer_on_foreign_connections() {
    use qa_network::commands::packet::Protocol;
    for protocol in [
        Protocol::Quake3_68,
        Protocol::Quake2_34,
        Protocol::NetQuake15,
    ] {
        let (mut host, mut source, handle) = fixture(Some(protocol));
        frame(&mut host, &mut source);
        call(&mut host, 31, &[1, 112, 128, handle as u64]);
        listener(&mut host);
        frame(&mut host, &mut source);
        let event = source.last_loop.unwrap();
        assert_eq!(event.velocity, Vec3([99., 0., 0.]));
        assert_eq!(
            event.doppler,
            Some(LoopDoppler {
                scale: 100.,
                previous_scale: 1.
            })
        );
        assert_eq!(event.client, host.local_clients[0]);
        assert!(event.client_lifetime.is_some());
        let partial = (((1820 * 63 * 255) / 256) >> 8) as i16;
        assert_eq!(
            source.pcm,
            [
                expected(63),
                expected(63),
                expected(63),
                expected(63),
                partial,
                partial,
                0,
                0
            ]
        );
        let doppler = host
            .console
            .cvars
            .bind("s_doppler", Context::default())
            .unwrap();
        host.console.cvars.force_write(doppler, "0").unwrap();
        call(&mut host, 31, &[1, 112, 128, handle as u64]);
        listener(&mut host);
        frame(&mut host, &mut source);
        assert_eq!(source.last_loop.unwrap().doppler, None);
        assert_eq!(source.pcm, [expected(63); 8]);
        host.console.cvars.force_write(doppler, "1").unwrap();
        call(&mut host, 80, &[1, 112, 128, handle as u64]);
        listener(&mut host);
        frame(&mut host, &mut source);
        assert_eq!(source.last_loop.unwrap().velocity, Vec3([99., 0., 0.]));
        assert_eq!(source.last_loop.unwrap().doppler, None);
        assert_eq!(source.pcm, [expected(45); 8]);
    }
}
