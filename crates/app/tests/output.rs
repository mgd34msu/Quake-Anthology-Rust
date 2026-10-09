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
    runtime.events.push(FrameEvent::Sound(SoundEvent {
        sound: SoundId(id),
        entity: None,
        channel: 1,
        position: Vec3::default(),
        volume: 1.0,
        attenuation: 1.0,
        action: SoundAction::Play,
    }));
    runtime.print_event(None, PrintKind::Center, format_args!("{id}"));
    runtime.events.push(FrameEvent::Effect(EffectEvent {
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
        Console::new(Context::default()),
        runtime,
        TickRate::fixed(20).unwrap(),
        (1..=3)
            .map(|id| Provider {
                module: ModuleId(id),
                rate: TickRate::fixed(20).unwrap(),
                frame: emit,
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
    assert!(host.runtime.events.is_empty());
    let center = host.runtime.server.clients[first.0 as usize]
        .hud
        .centerprint
        .unwrap();
    assert_eq!(
        host.runtime.server.clients[second.0 as usize]
            .hud
            .centerprint
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
        Console::new(Context::default()),
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
    assert!(host.runtime.events.is_empty());
}
