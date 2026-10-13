use qa_app::{
    Runtime, WorldCollision,
    client_policy::ClientPolicy,
    host::{FrameHost, FrameResult, FrameSource},
    map::SpawnAnchor,
};
use qa_console::{commands::Console, views::Context};
use qa_core::{
    loopback::Endpoint,
    primitives::{Bounds, ClientId, MovementMode, RuleSetId, UserCmd, Vec3},
    sys_events::{EventKind, EventTime, Peer, SeatId, SysEvent, SysEventQueue},
};
use qa_network::commands::{
    Q3Cmd,
    packet::{self, Move, Protocol, ZERO_Q3},
};
use qa_session::timing::TickRate;
use qa_world::collision::{brushes::BrushTree, surfaces::SurfaceTable};
use std::time::Duration;

struct Clock(EventTime);
impl FrameSource for Clock {
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

fn host(protocol: Protocol) -> (FrameHost, ClientId) {
    let mut runtime = Runtime::load(1, []).unwrap();
    let geometry = runtime
        .geometry
        .load_brushes(
            vec![],
            vec![],
            SurfaceTable::flags(vec![]),
            BrushTree::direct(0).unwrap(),
            vec![Bounds {
                mins: Vec3([-4096.0; 3]),
                maxs: Vec3([4096.0; 3]),
            }],
        )
        .unwrap();
    runtime.collision = Some(WorldCollision::new(&runtime.geometry, geometry, 0));
    let client = runtime
        .connect_local(
            SeatId::FIRST,
            SpawnAnchor {
                position: Vec3::default(),
                angles: Vec3::default(),
                entity: 0,
                fixture_fallback: false,
            },
            ClientPolicy::select(Some(RuleSetId::Quake3), None, None, None).unwrap(),
            protocol,
            0,
        )
        .unwrap();
    runtime.server.clients[client.0 as usize]
        .player
        .movement
        .mode = MovementMode::Noclip;
    (
        FrameHost::load(
            Console::new(Context::default()).unwrap(),
            runtime,
            TickRate::FrameDriven,
            vec![],
        )
        .unwrap(),
        client,
    )
}

fn command(time: i32, movement: [f32; 3]) -> UserCmd {
    UserCmd {
        duration_ms: 16,
        server_time_ms: time,
        movement,
        ..Default::default()
    }
}

#[test]
fn two_native_packets_in_one_drain_move_the_engine_client_twice_without_a_pending_overwrite() {
    for protocol in [
        Protocol::Quake2_34,
        Protocol::Quake2Repro1038,
        Protocol::Quake3_68,
    ] {
        let (mut host, client) = host(protocol);
        host.time = EventTime::from_milliseconds(32);
        for command in [
            command(16, [127.0, 0.0, 0.0]),
            command(32, [0.0, 127.0, 0.0]),
        ] {
            assert!(host.runtime.send_local_command(client, &command, host.time));
        }
        host.drain(&mut FrameResult::default());
        assert_eq!(host.runtime.network.commands, 2, "{protocol:?}");
        let player = &host.runtime.server.clients[client.0 as usize].player;
        assert!(
            player.body.position.0[0] > 0.0,
            "first command lost: {protocol:?}"
        );
        assert!(
            player.body.position.0[1] < 0.0,
            "second command lost: {protocol:?}"
        );
        let position = player.body.position;
        host.frame(&mut Clock(host.time), true);
        assert_eq!(
            host.runtime.server.clients[client.0 as usize]
                .player
                .body
                .position,
            position
        );
    }
}

#[test]
fn every_new_q3_command_inside_one_packet_reaches_engine_movement_in_order() {
    let (mut host, client) = host(Protocol::Quake3_68);
    let mut native = [ZERO_Q3; 32];
    native[0] = Q3Cmd {
        server_time: 16,
        movement: [127, 0, 0],
        ..ZERO_Q3
    };
    native[1] = Q3Cmd {
        server_time: 32,
        movement: [0, 127, 0],
        ..ZERO_Q3
    };
    let movement = Move::Quake3 {
        commands: native,
        count: 2,
        delta: false,
    };
    let connection = host
        .runtime
        .network
        .get_mut(client, Endpoint::Client)
        .unwrap();
    let sequence = connection.channel.send_state().sequence;
    let mut payload = [0; 1400];
    let length = packet::write(
        &mut payload,
        &movement,
        sequence,
        connection.channel.command_key(None).unwrap(),
    )
    .unwrap();
    let packet = connection
        .channel
        .prepare_move(&payload[..length], EventTime::default(), None, None)
        .unwrap()
        .unwrap();
    host.runtime
        .loopback
        .send(Endpoint::Client, client, packet.bytes)
        .unwrap();
    connection.channel.submitted(EventTime::default()).unwrap();
    host.time = EventTime::from_milliseconds(32);
    host.drain(&mut FrameResult::default());
    assert_eq!(host.runtime.network.commands, 2);
    let player = &host.runtime.server.clients[client.0 as usize].player;
    assert!(player.body.position.0[0] > 0.0, "intermediate command lost");
    assert!(player.body.position.0[1] < 0.0);
    assert_eq!(player.movement.command_time_ms, 32);
    let entity = host.runtime.server.clients[client.0 as usize].entity;
    let stored = host
        .runtime
        .server
        .entities
        .columns
        .body(entity.slot as usize);
    assert_eq!(
        (stored.position, stored.velocity, stored.mins, stored.maxs),
        (
            player.body.position,
            player.body.velocity,
            player.body.mins,
            player.body.maxs
        )
    );
}

#[test]
fn netquake_packets_keep_only_the_latest_move_and_apply_one_server_frame() {
    let (mut host, client) = host(Protocol::NetQuake15);
    host.time = EventTime::from_milliseconds(16);
    for command in [
        command(16, [127.0, 0.0, 0.0]),
        command(16, [0.0, 127.0, 0.0]),
    ] {
        assert!(host.runtime.send_local_command(client, &command, host.time));
    }
    host.drain(&mut FrameResult::default());
    assert_eq!(host.runtime.network.commands, 2);
    assert_eq!(
        host.runtime.server.clients[client.0 as usize]
            .player
            .body
            .position,
        Vec3::default()
    );
    host.frame(&mut Clock(host.time), true);
    let player = &host.runtime.server.clients[client.0 as usize].player;
    assert_eq!(player.body.position.0[0], 0.0);
    assert!(player.body.position.0[1] < 0.0);
    let position = player.body.position;
    host.frame(&mut Clock(host.time), true);
    assert_eq!(
        host.runtime.server.clients[client.0 as usize]
            .player
            .body
            .position,
        position
    );
}

#[test]
fn netquake_command_from_second_intake_uses_the_next_queued_server_frame_span() {
    struct SecondIntake {
        packet: Vec<u8>,
        client: ClientId,
        polls: u32,
    }
    impl FrameSource for SecondIntake {
        fn begin_frame(&mut self) -> Duration {
            Duration::from_secs(999)
        }
        fn poll_events(&mut self, queue: &mut SysEventQueue) {
            self.polls += 1;
            let time = EventTime::from_milliseconds(match self.polls {
                1 => 50,
                2 => 52,
                _ => 70,
            });
            if self.polls == 2 {
                queue
                    .push(SysEvent {
                        time,
                        kind: EventKind::Packet {
                            socket: Endpoint::Server.socket(),
                            from: Peer::Loopback(self.client),
                            bytes: &self.packet,
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
        fn wait_time(&mut self, _: Duration) -> Duration {
            panic!("uncapped fixture")
        }
        fn elapsed(&self) -> Duration {
            Duration::ZERO
        }
        fn present(&mut self) {}
    }
    let (mut host, client) = host(Protocol::NetQuake15);
    let connection = host
        .runtime
        .network
        .get_mut(client, Endpoint::Client)
        .unwrap();
    let mut payload = [0; 1400];
    let codec = connection.commands.as_ref().unwrap();
    let length = codec
        .encode(
            &command(52, [127.0, 0.0, 0.0]),
            0,
            &connection.channel,
            &mut payload,
        )
        .unwrap();
    let packet = connection
        .channel
        .prepare_move(&payload[..length], EventTime::default(), None, None)
        .unwrap()
        .unwrap()
        .bytes
        .to_vec();
    connection.channel.submitted(EventTime::default()).unwrap();
    let mut source = SecondIntake {
        packet,
        client,
        polls: 0,
    };
    host.frame(&mut source, true);
    assert_eq!(
        host.runtime.server.clients[client.0 as usize]
            .player
            .body
            .position,
        Vec3::default()
    );
    host.frame(&mut source, true);
    let client = &host.runtime.server.clients[client.0 as usize];
    assert_eq!(client.command.duration_ms, 20);
    assert_eq!(client.command.duration_ns, 20_000_000);
    assert_eq!(client.command.server_time_ms, 70);
    assert!(client.player.body.position.0[0] > 0.0);
}
