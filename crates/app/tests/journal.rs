use qa_app::{
    Runtime,
    host::{FrameHost, FrameSource},
};
use qa_console::{commands::Console, views::Context};
use qa_core::{primitives::RuleSetId, sys_events::*};
use qa_network::commands::packet::Protocol;
use qa_session::timing::TickRate;
use std::time::Duration;

struct Source {
    frame: u64,
    polls: u64,
    noisy: bool,
    sockets: bool,
}
impl FrameSource for Source {
    fn begin_frame(&mut self) -> Duration {
        self.frame += 1;
        Duration::from_millis(if self.noisy { 0 } else { self.frame * 20 })
    }
    fn poll_events(&mut self, queue: &mut SysEventQueue) {
        self.polls += 1;
        let time = EventTime::from_milliseconds(self.frame * 20);
        if self.noisy {
            queue
                .push(SysEvent {
                    time,
                    kind: EventKind::ConsoleLine("set sensitivity 99"),
                })
                .unwrap();
        } else if self.polls == 1 {
            queue
                .push(SysEvent {
                    time,
                    kind: EventKind::ConsoleLine("set sensitivity 2"),
                })
                .unwrap();
        }
        if self.polls % 2 == 1 {
            queue
                .push(SysEvent {
                    time,
                    kind: EventKind::Key {
                        device: DeviceId::Keyboard(0),
                        code: 26,
                        symbol: i32::from(b'w'),
                        down: self.frame < 3,
                        repeat: self.frame == 2,
                    },
                })
                .unwrap();
            queue
                .push(SysEvent {
                    time,
                    kind: EventKind::Mouse {
                        device: DeviceId::Mouse(0),
                        dx: self.frame as i32,
                        dy: 0,
                    },
                })
                .unwrap();
        }
        queue
            .push(SysEvent {
                time,
                kind: EventKind::Time,
            })
            .unwrap();
    }
    fn set_socket_submission_enabled(&mut self, enabled: bool) {
        self.sockets = enabled;
    }
    fn wait_time(&mut self, _: Duration) -> Duration {
        panic!("journal fixture needs no pacing wait")
    }
    fn elapsed(&self) -> Duration {
        Duration::ZERO
    }
    fn present(&mut self) {}
}
fn connected(protocol: Protocol) -> FrameHost {
    let mut host = FrameHost::load(
        Console::new(Context::default()).unwrap(),
        Runtime::load(2, []).unwrap(),
        TickRate::FrameDriven,
        vec![],
    )
    .unwrap();
    let client = host
        .runtime
        .connect_local(
            SeatId::FIRST,
            qa_app::map::SpawnAnchor {
                position: Default::default(),
                angles: Default::default(),
                entity: 0,
                fixture_fallback: false,
            },
            qa_app::client_policy::ClientPolicy::select(
                Some(RuleSetId::QuakeWorld),
                None,
                None,
                None,
            )
            .unwrap(),
            protocol,
            0,
        )
        .unwrap();
    host.local_clients[0] = Some(client);
    host.console
        .append("bind w +forward\n", Context::default())
        .unwrap();
    host.console.execute_frame(&mut host.runtime);
    host
}

#[test]
fn recorded_physical_inputs_replay_over_every_native_protocol_without_duplicate_loopback() {
    for (index, protocol) in [
        Protocol::NetQuake15,
        Protocol::QuakeWorld28,
        Protocol::Quake2_34,
        Protocol::Quake2Repro1038,
        Protocol::Quake3_68,
    ]
    .into_iter()
    .enumerate()
    {
        let path =
            std::env::temp_dir().join(format!("qa-host-journal-{}-{index}", std::process::id()));
        if path.exists() {
            qa_platform::files::remove_file(&path).unwrap();
        }
        let mut recording = connected(protocol);
        let mut live = Source {
            frame: 0,
            polls: 0,
            noisy: false,
            sockets: true,
        };
        recording.record_journal(&path).unwrap();
        let mut expected = Vec::new();
        for _ in 0..3 {
            let result = recording.frame(&mut live, true);
            expected.push((result.commands, result.events));
        }
        recording.flush_journal().unwrap();
        assert!(expected[0].0[0].movement[0] > 0.);
        let mut replay = connected(protocol);
        replay.replay_journal(&path).unwrap();
        let mut physical = Source {
            frame: 0,
            polls: 0,
            noisy: true,
            sockets: true,
        };
        for (commands, events) in expected {
            let result = replay.frame(&mut physical, false);
            for (actual, expected) in result.commands.into_iter().zip(commands) {
                assert_eq!(actual.duration_ms, expected.duration_ms);
                assert_eq!(actual.duration_ns, expected.duration_ns);
                assert_eq!(actual.server_time_ms, expected.server_time_ms);
                assert_eq!(
                    actual.view_angles.0.map(f32::to_bits),
                    expected.view_angles.0.map(f32::to_bits)
                );
                assert_eq!(
                    actual.movement.map(f32::to_bits),
                    expected.movement.map(f32::to_bits)
                );
                assert_eq!(actual.buttons, expected.buttons);
                assert_eq!(actual.impulse, expected.impulse);
                assert_eq!(actual.light_level, expected.light_level);
                assert_eq!(actual.weapon, expected.weapon);
            }
            assert_eq!(result.events, events, "{protocol:?}");
            assert_eq!(result.drains, 2);
            assert!(!physical.sockets);
            assert!(!replay.runtime.quit);
        }
        assert_eq!(recording.key_downs, replay.key_downs);
        assert_eq!(recording.key_repeats, replay.key_repeats);
        assert_eq!(recording.time, replay.time);
        replay.frame(&mut physical, false);
        assert!(replay.runtime.quit);
        assert!(replay.journal_error.is_none());
        qa_platform::files::remove_file(path).unwrap();
    }
}
