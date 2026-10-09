use qa_app::{
    Runtime,
    host::{FrameHost, FrameSource, Provider},
};
use qa_console::{commands::Console, views::Context};
use qa_core::{events::FrameEvent, primitives::*, sys_events::*};
use qa_session::{
    clients::{Connection, Server},
    timing::{Tick, TickRate},
};
use std::time::Duration;

#[derive(Default)]
struct Source {
    time: u64,
    sound_ids: [u32; 8],
    effect_ids: [u32; 8],
    sounds: usize,
    effects: usize,
}
impl FrameSource for Source {
    fn begin_frame(&mut self) -> EventTime {
        EventTime(self.time * 1_000_000)
    }
    fn poll_events(&mut self, queue: &mut SysEventQueue) {
        queue
            .push(SysEvent {
                time: EventTime(self.time * 1_000_000),
                kind: EventKind::Time,
            })
            .unwrap();
    }
    fn wait_time(&mut self, _: Duration) -> EventTime {
        panic!("uncapped fixture");
    }
    fn elapsed(&self) -> Duration {
        Duration::ZERO
    }
    fn present(&mut self) {}
    fn sound(&mut self, event: SoundEvent) -> bool {
        self.sound_ids[self.sounds] = event.sound.0;
        self.sounds += 1;
        true
    }
    fn effect(&mut self, event: EffectEvent) -> bool {
        self.effect_ids[self.effects] = event.effect.0;
        self.effects += 1;
        true
    }
}
fn emit(runtime: &mut Runtime, tick: Tick) {
    let id = tick.source_slot as u32;
    let _ = runtime.server.events.push(FrameEvent::Sound(SoundEvent {
        sound: SoundId(id),
        entity: None,
        channel: 1,
        position: Vec3::default(),
        volume: 1.0,
        attenuation: 1.0,
        action: SoundAction::Play,
    }));
    runtime.print_event(None, PrintKind::Center, format_args!("{id}"));
    let _ = runtime.server.events.push(FrameEvent::Effect(EffectEvent {
        effect: EffectId(id),
        position: Vec3::default(),
        direction: Vec3::default(),
        count: 1,
    }));
}
#[test]
fn mixed_provider_output_drains_once_and_routes_only_local_huds() {
    let mut runtime = Runtime::load(std::iter::empty()).unwrap();
    let first = runtime
        .server
        .connect(Connection::Local, ModuleId(1), PlayerTail::default(), None)
        .unwrap();
    let remote = runtime
        .server
        .connect(Connection::Remote, ModuleId(2), PlayerTail::default(), None)
        .unwrap();
    let second = runtime
        .server
        .connect(Connection::Local, ModuleId(3), PlayerTail::default(), None)
        .unwrap();
    let mut host = FrameHost::load(
        Console::new(Context::default()).unwrap(),
        runtime,
        TickRate::fixed(20).unwrap(),
        (1..=3)
            .map(|id| Provider {
                module: ModuleId(id),
                rate: TickRate::fixed(20).unwrap(),
                frame: emit,
                output: None,
            })
            .collect(),
    )
    .unwrap();
    host.local_clients[0] = Some(first);
    host.local_clients[1] = Some(second);
    let mut source = Source::default();
    host.frame(&mut source, true);
    source.time = 20;
    let result = host.frame(&mut source, true);
    assert_eq!(result.output_drains, 1);
    assert_eq!(
        (
            result.output.sounds,
            result.output.effects,
            result.output.prints
        ),
        (3, 3, 3)
    );
    assert_eq!(
        (
            result.output.unhandled_sounds,
            result.output.unhandled_effects
        ),
        (0, 0)
    );
    assert_eq!(source.sound_ids[..3], [1, 2, 3]);
    assert_eq!(source.effect_ids[..3], [1, 2, 3]);
    assert_eq!(host.runtime.server.events.len(), 9); // Remote has no native channel yet.
    let center = host.runtime.server.clients[first.0 as usize]
        .hud
        .centerprint
        .as_ref()
        .unwrap();
    assert_eq!(
        host.runtime.server.clients[second.0 as usize]
            .hud
            .centerprint
            .as_ref()
            .unwrap()
            .text,
        center.text
    );
    assert!(
        host.runtime.server.clients[remote.0 as usize]
            .hud
            .centerprint
            .is_none()
    );
    assert_eq!(center.started_at, 0.02);
    host.runtime
        .print_event(Some(second), PrintKind::Layout, format_args!("layout"));
    host.runtime
        .print_event(Some(remote), PrintKind::Notify, format_args!("remote"));
    source.time = 21;
    let result = host.frame(&mut source, true);
    assert_eq!(result.output.prints, 2);
    assert_eq!(result.output.sounds, 0);
    assert!(
        host.runtime.server.clients[first.0 as usize]
            .hud
            .layout_text
            .is_none()
    );
    assert!(
        host.runtime.server.clients[second.0 as usize]
            .hud
            .layout_text
            .is_some()
    );
    assert!(
        host.runtime.server.clients[first.0 as usize]
            .hud
            .notify
            .iter()
            .all(Option::is_none)
    );
    assert!(
        host.runtime.server.clients[second.0 as usize]
            .hud
            .notify
            .iter()
            .all(Option::is_none)
    );
}

#[test]
fn high_client_ids_route_to_local_huds_once_even_with_duplicate_seat_bindings() {
    let mut runtime = Runtime::load(std::iter::empty()).unwrap();
    runtime.server = Server::load(512, 1024, 1, 0, 0, 0).unwrap();
    for _ in 0..512 {
        runtime
            .server
            .connect(Connection::Local, ModuleId(1), PlayerTail::None, None)
            .unwrap();
    }
    let local = [
        Some(ClientId(64)),
        Some(ClientId(255)),
        Some(ClientId(511)),
        Some(ClientId(64)),
    ];
    runtime.print_event(None, PrintKind::Notify, format_args!("message\n"));
    let mut source = Source::default();
    let result = qa_app::output::dispatch(
        &mut runtime,
        &mut source,
        &local,
        EventTime(1_000_000_000),
        3.0,
        2.0,
    );
    assert_eq!(result.prints, 1);
    for slot in [64, 255, 511] {
        assert_eq!(
            runtime.server.clients[slot]
                .hud
                .notify
                .iter()
                .flatten()
                .count(),
            1
        );
    }
    assert_eq!(
        runtime.server.clients[0]
            .hud
            .notify
            .iter()
            .flatten()
            .count(),
        0
    );
}
#[test]
fn quit_flushes_console_output_once() {
    let mut host = FrameHost::load(
        Console::new(Context::default()).unwrap(),
        Runtime::load(std::iter::empty()).unwrap(),
        TickRate::FrameDriven,
        vec![],
    )
    .unwrap();
    host.runtime
        .print_event(None, PrintKind::Console, format_args!("1\n"));
    host.console
        .append_line("quit", Context::default())
        .unwrap();
    let result = host.frame(&mut Source::default(), true);
    assert!(host.runtime.quit);
    assert_eq!((result.output_drains, result.output.prints), (1, 1));
    assert!(host.runtime.server.events.is_empty());
}

fn consume_module(
    runtime: &mut Runtime,
    tick: Tick,
    record: qa_core::events::OutputRecord,
) -> qa_core::events::OutputSubmission {
    let FrameEvent::Print(print) = record.event else {
        return qa_core::events::OutputSubmission::BestEffort;
    };
    assert!(runtime.server.events.texts.get(print.text).is_some());
    // Unconnected fixture rows count delivery, never infer native entity numbers.
    runtime.server.clients[10 + tick.source_slot].player.health += 1;
    qa_core::events::OutputSubmission::BestEffort
}
fn emit_print(runtime: &mut Runtime, tick: Tick) {
    runtime.print_event(
        None,
        PrintKind::Center,
        format_args!("{}:{}", tick.source_slot, tick.index),
    );
}
struct PeerSource {
    source: Source,
    healthy: u64,
    reliable: u64,
    unsent: bool,
}
impl FrameSource for PeerSource {
    fn begin_frame(&mut self) -> EventTime {
        self.source.begin_frame()
    }
    fn poll_events(&mut self, q: &mut SysEventQueue) {
        self.source.poll_events(q);
    }
    fn wait_time(&mut self, d: Duration) -> EventTime {
        self.source.wait_time(d)
    }
    fn elapsed(&self) -> Duration {
        Duration::ZERO
    }
    fn present(&mut self) {}
    fn output(
        &mut self,
        client: ClientId,
        event: FrameEvent,
        text: Option<&[u8]>,
    ) -> qa_core::events::OutputSubmission {
        assert!(matches!(event, FrameEvent::Print(_)));
        assert!(text.is_some());
        if client == ClientId(0) {
            self.reliable += 1;
            if self.unsent {
                qa_core::events::OutputSubmission::Unsent
            } else {
                qa_core::events::OutputSubmission::Reliable(qa_core::events::NativeReceipt(7))
            }
        } else {
            self.healthy += 1;
            qa_core::events::OutputSubmission::BestEffort
        }
    }
}
fn stalled_peer_progress(unsent: bool) {
    use qa_core::events::{EventRing, NativeReceipt, OutputTarget};
    let mut runtime = Runtime::load(std::iter::empty()).unwrap();
    runtime.server.events = EventRing::load(32, 8, 64, 256).unwrap();
    runtime.server.presentation = runtime
        .server
        .events
        .bind(OutputTarget::Presentation)
        .unwrap();
    let stalled = runtime
        .server
        .connect(Connection::Remote, ModuleId(0), PlayerTail::None, None)
        .unwrap();
    let healthy = runtime
        .server
        .connect(Connection::Remote, ModuleId(0), PlayerTail::None, None)
        .unwrap();
    let local = runtime
        .server
        .connect(Connection::Local, ModuleId(0), PlayerTail::None, None)
        .unwrap();
    let stalled_cursor = runtime.server.clients[stalled.0 as usize].output.unwrap();
    let healthy_cursor = runtime.server.clients[healthy.0 as usize].output.unwrap();
    let mut host = FrameHost::load(
        Console::new(Context::default()).unwrap(),
        runtime,
        TickRate::fixed(25).unwrap(),
        [100, 50, 25]
            .into_iter()
            .enumerate()
            .map(|(slot, ms)| Provider {
                module: ModuleId(slot as u16 + 1),
                rate: TickRate::fixed(ms).unwrap(),
                frame: emit_print,
                output: Some(consume_module),
            })
            .collect(),
    )
    .unwrap();
    host.local_clients[0] = Some(local);
    let mut source = PeerSource {
        source: Source::default(),
        healthy: 0,
        reliable: 0,
        unsent,
    };
    let mut ticks = 0;
    for frame in 0..=400 {
        source.source.time = frame * 25;
        let before = source.healthy;
        let result = host.frame(&mut source, true);
        ticks += result.server_ticks;
        assert_eq!(host.runtime.server.world_frame, frame);
        assert_eq!(
            source.healthy - before,
            result.server_ticks.saturating_sub(1)
        );
        if frame > 0 {
            assert!(source.healthy > before);
        }
    }
    // Native 10/20/40-Hz modules and 40-Hz world progress without an ACK.
    assert_eq!(ticks, 1100);
    assert_eq!(host.runtime.server.world_frame, 400);
    assert_eq!(source.healthy, 700);
    if unsent {
        assert!(source.reliable > 0);
    } else {
        assert_eq!(source.reliable, 32);
    }
    let counters = host.runtime.server.events.counters(stalled_cursor).unwrap();
    assert_eq!(
        (
            counters.acknowledgements,
            counters.overflow,
            counters.resyncs
        ),
        (0, 1, 1)
    );
    assert_eq!(
        host.runtime
            .server
            .events
            .counters(healthy_cursor)
            .unwrap()
            .overflow,
        0
    );
    assert!(host.runtime.server.events.needs_resync(stalled_cursor));
    for slot in 11..14 {
        assert!(host.runtime.server.clients[slot].player.health > 600);
    }
    let line = host.runtime.server.clients[local.0 as usize]
        .hud
        .centerprint
        .as_ref()
        .unwrap();
    assert!(
        host.runtime
            .server
            .events
            .texts
            .get(line.text.id())
            .is_some()
    );
    assert_eq!(
        host.runtime
            .server
            .events
            .acknowledge(stalled_cursor, NativeReceipt(7)),
        0
    );
    assert!(host.runtime.server.disconnect(local)); // Explicitly retires display leases.
    assert!(host.runtime.server.disconnect(stalled));
    assert!(host.runtime.server.disconnect(healthy));
}

#[test]
fn stalled_reliable_peer_cannot_stop_healthy_delivery_or_mixed_rate_server_ticks() {
    stalled_peer_progress(false);
}

#[test]
fn unsent_peer_cannot_stop_healthy_delivery_or_mixed_rate_server_ticks() {
    stalled_peer_progress(true);
}
