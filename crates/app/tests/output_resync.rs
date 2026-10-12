use qa_app::{
    Runtime,
    client_policy::ClientPolicy,
    host::{FrameHost, FrameResult, FrameSource},
    map::SpawnAnchor,
};
use qa_console::{commands::Console, views::Context};
use qa_core::{
    events::{EventRing, FrameEvent, OutputSubmission, OutputTarget},
    loopback::Endpoint,
    primitives::*,
    sys_events::*,
};
use qa_network::commands::packet::Protocol;
use qa_session::timing::TickRate;
use std::time::Duration;

#[derive(Default)]
struct Source(EventTime);
impl FrameSource for Source {
    fn begin_frame(&mut self) -> Duration {
        Duration::from_nanos(self.0.0)
    }
    fn poll_events(&mut self, queue: &mut SysEventQueue) {
        queue
            .push(SysEvent {
                time: self.0,
                kind: EventKind::Time,
            })
            .unwrap();
    }
    fn wait_time(&mut self, _: Duration) -> Duration {
        panic!("uncapped fixture")
    }
    fn elapsed(&self) -> Duration {
        Duration::ZERO
    }
    fn present(&mut self) {}
}

#[test]
fn local_resync_drains_native_flight_then_preserves_commands_snapshots_and_hud_leases() {
    for protocol in [
        Protocol::NetQuake15,
        Protocol::QuakeWorld28,
        Protocol::Quake2_34,
        Protocol::Quake2Repro1038,
        Protocol::Quake3_68,
    ] {
        let mut runtime = Runtime::load(2, []).unwrap();
        runtime.server.events = EventRing::load(8, 3, 40, 8192).unwrap();
        runtime.server.presentation = runtime
            .server
            .events
            .bind(OutputTarget::Presentation)
            .unwrap();
        let policy = ClientPolicy::select(Some(RuleSetId::Quake2), None, None, None).unwrap();
        let ids = std::array::from_fn::<_, 2, _>(|seat| {
            runtime
                .connect_local(
                    SeatId::new(seat as u8).unwrap(),
                    SpawnAnchor {
                        position: Default::default(),
                        angles: Default::default(),
                        entity: 0,
                        fixture_fallback: false,
                    },
                    policy,
                    protocol,
                    seat as u32,
                )
                .unwrap()
        });
        let [stalled, healthy] = ids;
        let old = runtime.server.clients[stalled.0 as usize].output.unwrap();
        let healthy_output = runtime.server.clients[healthy.0 as usize].output.unwrap();
        let mut host = FrameHost::load(
            Console::new(Context::default()).unwrap(),
            runtime,
            TickRate::fixed(50).unwrap(),
            vec![],
        )
        .unwrap();
        host.local_clients[..2].copy_from_slice(&ids.map(Some));
        let mut source = Source::default();
        host.runtime.prediction[0].player.inventory.item_counts[1] = 4;
        host.runtime
            .print_event(Some(stalled), PrintKind::Center, format_args!("leased"));
        host.frame(&mut source, true);
        // Admit the local command queued in CLIENT before blocking SERVER
        // transport, including QW's native request/reply grant.
        host.drain(&mut FrameResult::default());
        let lease = host.runtime.server.clients[stalled.0 as usize]
            .hud
            .centerprint
            .as_ref()
            .unwrap()
            .text
            .id();

        // Saturate only this client's SERVER transport, leaving the other seat
        // and the world clock free to progress. Keep a real reliable flight.
        host.runtime.loopback.clear_client(stalled);
        while host
            .runtime
            .loopback
            .send(Endpoint::Server, stalled, &[0])
            .is_ok()
        {}
        let mut bytes = [0; 128];
        let length = qa_network::outputs::print(
            protocol,
            PrintKind::Console,
            2,
            b"old flight",
            None,
            &mut bytes,
        )
        .unwrap();
        let receipt = host
            .runtime
            .network
            .get_mut(stalled, Endpoint::Server)
            .unwrap()
            .channel
            .queue_reliable(&bytes[..length])
            .unwrap();
        let retained = host
            .runtime
            .server
            .events
            .push_to(
                OutputTarget::Client(stalled),
                FrameEvent::Hud(HudEvent {
                    client: stalled,
                    change: HudChange::Collectibles(17),
                }),
            )
            .unwrap();
        assert!(host.runtime.server.events.submit(
            old,
            retained,
            OutputSubmission::Reliable(receipt)
        ));
        qa_app::output::dispatch(
            &mut host.runtime,
            &mut source,
            &mut host.console.scrollback,
            &host.local_clients,
            EventTime(1),
            3.0,
            2.0,
        );
        assert!(
            host.runtime
                .network
                .get(stalled, Endpoint::Server)
                .unwrap()
                .channel
                .pending_packet()
                .is_some()
        );
        for value in 0..9 {
            host.runtime
                .server
                .events
                .push_to(
                    OutputTarget::Client(stalled),
                    FrameEvent::Hud(HudEvent {
                        client: stalled,
                        change: HudChange::Collectibles(value),
                    }),
                )
                .unwrap();
        }
        assert!(host.runtime.server.events.needs_resync(old));
        qa_app::output::dispatch(
            &mut host.runtime,
            &mut source,
            &mut host.console.scrollback,
            &host.local_clients,
            EventTime(2),
            3.0,
            2.0,
        );
        assert_eq!(
            host.runtime.server.clients[stalled.0 as usize].output,
            Some(old)
        );
        assert_eq!(host.runtime.server.events.acknowledge(old, receipt), 0);
        host.runtime.loopback.clear_client(stalled);

        for frame in 1..=6 {
            source.0 = EventTime::from_milliseconds(frame * 50);
            host.frame(&mut source, true);
        }
        let new = host.runtime.server.clients[stalled.0 as usize]
            .output
            .unwrap();
        assert_ne!(old, new, "{protocol:?}");
        assert!(!host.runtime.server.events.needs_resync(new));
        assert_eq!(
            host.runtime.snapshots[stalled.0 as usize]
                .as_ref()
                .unwrap()
                .output,
            new
        );
        assert_eq!(
            host.runtime
                .network
                .get(stalled, Endpoint::Server)
                .unwrap()
                .output,
            Some(new)
        );
        assert_eq!(host.runtime.server.events.acknowledge(new, receipt), 0);
        assert_eq!(
            host.runtime.server.events.texts.get(lease),
            Some(&b"leased"[..])
        );
        assert_eq!(
            host.runtime.server.clients[stalled.0 as usize]
                .hud
                .inventory
                .item_counts[1],
            4
        );
        assert_eq!(
            host.runtime
                .server
                .events
                .counters(healthy_output)
                .unwrap()
                .overflow,
            0
        );
        assert!(host.runtime.server.world_frame >= 5);

        // Actual queued ingress must pass both generation guards after resync.
        let command = UserCmd {
            duration_ms: 16,
            server_time_ms: 400,
            movement: [77.0, 0.0, 0.0],
            ..Default::default()
        };
        assert!(host.runtime.send_local_command(stalled, &command, source.0));
        host.drain(&mut FrameResult::default());
        assert_eq!(
            host.runtime.server.clients[stalled.0 as usize]
                .command
                .movement[0],
            77.0
        );
        let before = host.runtime.snapshots[stalled.0 as usize]
            .as_ref()
            .unwrap()
            .applied;
        host.runtime.server.clients[stalled.0 as usize]
            .player
            .body
            .position = Vec3([12.0, 24.0, 48.0]);
        host.runtime.server.clients[stalled.0 as usize]
            .player
            .health = 137;
        assert!(
            host.runtime.send_snapshot(
                stalled,
                EventTime::from_milliseconds(500),
                None,
                &mut Source::default()
            ),
            "{protocol:?}"
        );
        host.drain(&mut FrameResult::default());
        assert!(
            host.runtime.snapshots[stalled.0 as usize]
                .as_ref()
                .unwrap()
                .applied
                > before
        );
        // NQ clientdata does not carry the world entity's origin; QW's own
        // playerinfo does not carry health. Check fields these messages own.
        if protocol != Protocol::NetQuake15 {
            assert_eq!(
                host.runtime.prediction[0].player.body.position,
                Vec3([12.0, 24.0, 48.0])
            );
        }
        if protocol != Protocol::QuakeWorld28 {
            assert_eq!(host.runtime.prediction[0].player.health, 137);
        }
    }
}
