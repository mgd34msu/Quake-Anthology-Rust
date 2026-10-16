use qa_app::{Runtime, client_policy::ClientPolicy, host::FrameSource, map::SpawnAnchor};
use qa_audio::{Bank, Listener, Mixer};
use qa_console::presentation::Scrollback;
use qa_core::{events::*, loopback::Endpoint, primitives::*, sys_events::*};
use qa_network::{
    channel::Channel,
    commands::{connection::Commands, packet::Protocol},
    ingress::{Connection as NativeConnection, Route},
};
use qa_session::clients::Connection;
use std::{num::NonZeroU32, time::Duration};

struct Source {
    mixer: Mixer,
    retired: Vec<EntityId>,
}
impl FrameSource for Source {
    fn begin_frame(&mut self) -> Duration {
        Duration::ZERO
    }
    fn poll_events(&mut self, _: &mut SysEventQueue) {}
    fn wait_time(&mut self, _: Duration) -> Duration {
        Duration::ZERO
    }
    fn elapsed(&self) -> Duration {
        Duration::ZERO
    }
    fn present(&mut self) {}
    fn sound(&mut self, _bank: Option<&qa_audio::Bank>, event: SoundEvent) -> bool {
        if event.action == SoundAction::RetireClient {
            self.retired.push(event.client_lifetime.unwrap());
        }
        self.mixer.sound(_bank.unwrap(), event)
    }
}
fn fixture(capacity: usize) -> (Runtime, Source) {
    let mut runtime = Runtime::load(1, []).unwrap();
    runtime.server.events = EventRing::load(capacity, 2, capacity + 1, 64).unwrap();
    runtime.server.presentation = runtime
        .server
        .events
        .bind(OutputTarget::Presentation)
        .unwrap();
    let rate = NonZeroU32::new(44100).unwrap();
    let bank = Bank::load(
        rate,
        vec![(
            "sound/loop.wav".into(),
            Pcm {
                rate,
                channels: PcmChannels::Mono,
                samples: vec![1000; 4],
                loop_start: Some(0),
                loop_end: None,
                loop_invalid: false,
            },
        )],
    )
    .unwrap();
    let mut mixer = Mixer::load(rate, 8, 1);
    mixer.listen(
        &[Some(Listener {
            negate_dot: false,
            client: ClientId(0),
            entity: None,
            origin: Vec3::default(),
            right: Vec3([1.0, 0.0, 0.0]),
        })],
        1.0,
    );
    runtime.sound_assets = Some(Box::new(qa_app::audio::SoundAssets::from_bank(bank)));
    (
        runtime,
        Source {
            mixer,
            retired: Vec::new(),
        },
    )
}
fn connect(runtime: &mut Runtime) -> Result<ClientId, String> {
    runtime.connect_local(
        SeatId::FIRST,
        SpawnAnchor {
            position: Vec3::default(),
            angles: Vec3::default(),
            entity: 0,
            fixture_fallback: false,
        },
        ClientPolicy {
            input: RuleSetId::QuakeWorld,
            client: RuleSetId::QuakeWorld,
            movement: RuleSetId::Quake2,
            trace: RuleSetId::Quake,
        },
        Protocol::QuakeWorld28,
        0,
    )
}
fn start(source: &mut Source, bank: &Bank, id: ClientId, lifetime: EntityId) {
    assert!(source.mixer.sound(
        bank,
        SoundEvent {
            sound: bank.find("sound/loop.wav").unwrap(),
            action: SoundAction::StartLoop,
            client: Some(id),
            client_lifetime: Some(lifetime),
            volume: 1.0,
            attenuation: 0.0,
            ..SoundEvent::default()
        }
    ));
}
fn audible(source: &mut Source, bank: &Bank) -> bool {
    let mut pcm = [0; 8];
    assert!(source.mixer.paint(bank, &mut pcm));
    pcm.iter().any(|sample| *sample != 0)
}
fn dispatch(runtime: &mut Runtime, source: &mut Source) {
    qa_app::output::dispatch(
        runtime,
        source,
        &mut Scrollback::default(),
        &[None; SeatId::COUNT],
        EventTime(0),
        3.0,
        2.0,
    );
}
fn effect() -> FrameEvent {
    FrameEvent::Effect(EffectEvent {
        effect: EffectId(0),
        position: Vec3::default(),
        direction: Vec3::default(),
        count: 1,
        ..EffectEvent::default()
    })
}

#[test]
fn canceled_retirement_retries_and_old_controls_cannot_stop_a_replacement() {
    let (mut runtime, mut source) = fixture(2);
    let client = connect(&mut runtime).unwrap();
    let old = runtime.server.clients[0].entity;
    start(&mut source, runtime.sound_bank().unwrap(), client, old);
    assert!(runtime.disconnect_client(client));
    runtime.publish_audio_retirements();
    let token = runtime.server.clients[0]
        .audio_retirement
        .unwrap()
        .publication
        .unwrap();
    assert!(runtime.server.events.retained_for(token.0, token.1));
    assert!(
        connect(&mut runtime).is_err(),
        "admission alone must not permit reuse"
    );
    for _ in 0..2 {
        runtime
            .server
            .events
            .push_to(OutputTarget::Presentation, effect())
            .unwrap();
    }
    assert!(runtime.server.events.needs_resync(token.0));
    assert!(!runtime.server.events.retained_for(token.0, token.1));
    assert!(
        audible(&mut source, runtime.sound_bank().unwrap()),
        "cancellation is not mixer consumption"
    );
    dispatch(&mut runtime, &mut source);
    assert_eq!(source.retired, [old]);
    assert!(runtime.server.clients[0].audio_retirement.is_none());
    assert!(!audible(&mut source, runtime.sound_bank().unwrap()));
    let replacement = connect(&mut runtime).unwrap();
    assert_eq!(replacement, client);
    let fresh = runtime.server.clients[0].entity;
    assert_ne!(fresh, old);
    start(&mut source, runtime.sound_bank().unwrap(), client, fresh);
    runtime
        .server
        .events
        .push_to(
            OutputTarget::Presentation,
            FrameEvent::Sound(SoundEvent::retire_client(client, old)),
        )
        .unwrap();
    dispatch(&mut runtime, &mut source);
    assert!(audible(&mut source, runtime.sound_bank().unwrap()));
    assert_eq!(
        runtime.server.clients[0].connection,
        Some(Connection::Local)
    );
}

#[test]
fn full_ring_resync_admits_cleanup_without_waiting_forever_on_a_stalled_owner() {
    let (mut runtime, mut source) = fixture(2);
    let client = connect(&mut runtime).unwrap();
    let old = runtime.server.clients[0].entity;
    start(&mut source, runtime.sound_bank().unwrap(), client, old);
    for _ in 0..2 {
        runtime
            .server
            .events
            .push_to(OutputTarget::Presentation, effect())
            .unwrap();
    }
    assert!(runtime.disconnect_client(client));
    runtime.publish_audio_retirements();
    let pending = runtime.server.clients[0].audio_retirement.unwrap();
    let (consumer, sequence) = pending.publication.unwrap();
    assert!(runtime.server.events.retained_for(consumer, sequence));
    assert!(runtime.server.events.counters(consumer).unwrap().overflow > 0);
    assert!(audible(&mut source, runtime.sound_bank().unwrap()));
    dispatch(&mut runtime, &mut source);
    assert!(runtime.server.clients[0].audio_retirement.is_none());
    assert!(!audible(&mut source, runtime.sound_bank().unwrap()));
    assert_eq!(source.retired, [old]);
}

#[test]
fn replacing_client_resources_tears_down_then_reconnects_after_consumption() {
    let (mut runtime, mut source) = fixture(4);
    let client = connect(&mut runtime).unwrap();
    let old = runtime.server.clients[0].entity;
    start(&mut source, runtime.sound_bank().unwrap(), client, old);
    let protocol = Protocol::QuakeWorld28;
    let replacement = NativeConnection {
        route: Route {
            socket: Endpoint::Server.socket(),
            peer: Peer::Loopback(client),
        },
        channel: Channel::load(protocol.channel(), Endpoint::Server, 8192, 16).unwrap(),
        output: None,
        pending_output: None,
        commands: Some(Commands::load(protocol, 8192)),
    };
    assert!(
        runtime
            .bind_server_connection(client, replacement, 0, Some(SeatId::FIRST))
            .is_err()
    );
    assert!(runtime.server.clients[0].connection.is_none());
    assert!(runtime.client_resources[0].is_none());
    assert!(runtime.snapshots[0].is_none());
    assert!(runtime.network.get(client, Endpoint::Client).is_none());
    assert!(runtime.network.get(client, Endpoint::Server).is_none());
    assert!(connect(&mut runtime).is_err());
    dispatch(&mut runtime, &mut source);
    assert!(!audible(&mut source, runtime.sound_bank().unwrap()));
    assert_eq!(connect(&mut runtime).unwrap(), client);
    assert_ne!(runtime.server.clients[0].entity, old);
}

#[test]
fn rejected_local_setup_and_headless_server_disconnect_do_not_leave_owned_audio() {
    let (mut runtime, _) = fixture(4);
    runtime.network = qa_network::ingress::Connections::load(0);
    assert!(connect(&mut runtime).is_err());
    assert!(runtime.server.clients[0].connection.is_none());
    assert!(runtime.server.clients[0].audio_retirement.is_none());
    runtime.network = qa_network::ingress::Connections::load(1);
    let id = runtime
        .server
        .connect(Connection::Remote, ModuleId(3), None)
        .unwrap();
    assert!(runtime.disconnect_client(id));
    assert!(runtime.server.clients[0].audio_retirement.is_none());
    assert_eq!(
        runtime
            .server
            .connect(Connection::Remote, ModuleId(3), None),
        Some(id)
    );
}
