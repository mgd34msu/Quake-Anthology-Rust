use qa_app::{
    Runtime,
    host::{FrameHost, FrameSource},
};
use qa_console::{commands::Console, views::Context};
use qa_core::{
    events::OutputSubmission,
    loopback::Endpoint,
    primitives::{ClientId, ModuleId, PlayerTail, PrintKind},
    sys_events::{EventKind, EventTime, Peer, SysEvent, SysEventQueue},
};
use qa_network::{
    channel::{self, Channel, Policy},
    headers,
    ingress::{Connection as ChannelConnection, Route},
};
use qa_session::{clients::Connection, timing::TickRate};
use std::time::Duration;

#[derive(Default)]
struct Clock {
    polls: u64,
    time: u64,
}
impl FrameSource for Clock {
    fn begin_frame(&mut self) -> EventTime {
        self.time += 10_000_000;
        EventTime(self.time)
    }
    fn poll_events(&mut self, queue: &mut SysEventQueue) {
        self.polls += 1;
        assert!(
            queue
                .push(SysEvent {
                    time: EventTime(self.time),
                    kind: EventKind::Time
                })
                .is_ok()
        );
    }
    fn wait_time(&mut self, _: Duration) -> EventTime {
        EventTime(self.time)
    }
    fn elapsed(&self) -> Duration {
        Duration::ZERO
    }
    fn present(&mut self) {}
}
fn host(policy: Policy) -> Result<(FrameHost, [ClientId; 2]), String> {
    let mut runtime = Runtime::load(2, [])?;
    let mut clients = [ClientId(0); 2];
    for id in &mut clients {
        *id = runtime
            .server
            .connect(Connection::Remote, ModuleId(0), PlayerTail::None, None)
            .ok_or("client")?;
        for endpoint in [Endpoint::Client, Endpoint::Server] {
            runtime
                .network
                .bind(
                    *id,
                    endpoint,
                    ChannelConnection {
                        route: Route {
                            socket: endpoint.socket(),
                            peer: Peer::Loopback(*id),
                        },
                        channel: Channel::load(policy, endpoint, 8192, 16)
                            .map_err(|e| e.to_string())?,
                        output: (endpoint == Endpoint::Server)
                            .then_some(runtime.server.clients[id.0 as usize].output)
                            .flatten(),
                    },
                )
                .map_err(|e| format!("bind {e:?}"))?;
        }
    }
    Ok((
        FrameHost::load(
            Console::new(Context::default()).map_err(|e| format!("console {e:?}"))?,
            runtime,
            TickRate::FrameDriven,
            vec![],
        )?,
        clients,
    ))
}
fn publish(
    host: &mut FrameHost,
    client: ClientId,
) -> Result<qa_core::events::NativeReceipt, String> {
    let sequence = host
        .runtime
        .server
        .events
        .print(Some(client), PrintKind::Center, format_args!("retained"))
        .map_err(|e| format!("print {e:?}"))?;
    let output = host.runtime.server.clients[client.0 as usize]
        .output
        .ok_or("output")?;
    let channel = &mut host
        .runtime
        .network
        .get_mut(client, Endpoint::Server)
        .ok_or("channel")?
        .channel;
    let receipt = channel
        .queue_reliable(b"\x1aretained\0")
        .map_err(|e| e.to_string())?;
    assert!(host.runtime.server.events.submit(
        output,
        sequence,
        OutputSubmission::Reliable(receipt)
    ));
    Ok(receipt)
}
fn send(host: &mut FrameHost, client: ClientId, endpoint: Endpoint) -> Result<(), String> {
    let mut packet = [0; 1400];
    let channel = &mut host
        .runtime
        .network
        .get_mut(client, endpoint)
        .ok_or("channel")?
        .channel;
    let prepared = channel
        .prepare(None, host.time)
        .map_err(|e| e.to_string())?
        .ok_or("packet")?;
    let n = prepared.bytes.len();
    packet[..n].copy_from_slice(prepared.bytes);
    host.runtime
        .loopback
        .send(endpoint, client, &packet[..n])
        .map_err(|e| format!("send {e:?}"))?;
    channel.submitted(host.time).map_err(|e| e.to_string())
}
fn acks(host: &FrameHost, client: ClientId) -> Result<u64, String> {
    let output = host.runtime.server.clients[client.0 as usize]
        .output
        .ok_or("output")?;
    Ok(host
        .runtime
        .server
        .events
        .counters(output)
        .ok_or("counters")?
        .acknowledged_records)
}

#[test]
fn native_acks_traverse_the_host_queue_and_only_retire_their_peer() -> Result<(), String> {
    for policy in [
        channel::NETQUAKE,
        channel::QUAKEWORLD,
        channel::QUAKE2,
        channel::q2_new(true),
    ] {
        let (mut host, clients) = host(policy)?;
        let mut clock = Clock::default();
        for client in clients {
            publish(&mut host, client)?;
            send(&mut host, client, Endpoint::Server)?;
        }
        host.frame(&mut clock, true);
        assert_eq!(acks(&host, clients[0])?, 0);
        assert_eq!(acks(&host, clients[1])?, 0);
        // Only the healthy peer submits its native acknowledgement.
        send(&mut host, clients[1], Endpoint::Client)?;
        host.frame(&mut clock, true);
        assert_eq!(acks(&host, clients[0])?, 0);
        assert_eq!(acks(&host, clients[1])?, 1);
        send(&mut host, clients[0], Endpoint::Client)?;
        host.frame(&mut clock, true);
        assert_eq!(acks(&host, clients[0])?, 1);
        assert_eq!(host.runtime.network.acknowledged, 2);
        assert_eq!(clock.polls, 6);
        assert!(host.runtime.server.events.is_empty());
    }
    Ok(())
}

#[test]
fn unsent_and_duplicate_acks_cannot_retire_output() -> Result<(), String> {
    let (mut host, clients) = host(channel::QUAKEWORLD)?;
    let client = clients[0];
    publish(&mut host, client)?;
    let mut clock = Clock::default();
    let mut packet = [0; 1400];
    let n = headers::encode(
        headers::QUAKEWORLD,
        headers::Direction::ToServer,
        headers::Header {
            sequence: 1,
            acknowledgement: 1,
            reliable_ack: true,
            ..headers::Header::default()
        },
        &[],
        &mut packet,
    )
    .map_err(|e| e.to_string())?;
    host.runtime
        .loopback
        .send(Endpoint::Client, client, &packet[..n])
        .map_err(|e| format!("send {e:?}"))?;
    host.frame(&mut clock, true);
    assert_eq!(acks(&host, client)?, 0);
    send(&mut host, client, Endpoint::Server)?;
    host.frame(&mut clock, true);
    // The first accepted sequence already advanced the server receive state.
    let n = headers::encode(
        headers::QUAKEWORLD,
        headers::Direction::ToServer,
        headers::Header {
            sequence: 2,
            acknowledgement: 1,
            reliable_ack: true,
            ..headers::Header::default()
        },
        &[],
        &mut packet,
    )
    .map_err(|e| e.to_string())?;
    for _ in 0..2 {
        host.runtime
            .loopback
            .send(Endpoint::Client, client, &packet[..n])
            .map_err(|e| format!("send {e:?}"))?;
        host.frame(&mut clock, true);
        assert_eq!(acks(&host, client)?, 1);
    }
    assert_eq!(host.runtime.network.acknowledged, 1);
    Ok(())
}

#[test]
fn delayed_ack_keeps_the_original_consumer_generation_on_slot_reuse() -> Result<(), String> {
    let (mut host, clients) = host(channel::QUAKEWORLD)?;
    let client = clients[0];
    let receipt = publish(&mut host, client)?;
    send(&mut host, client, Endpoint::Server)?;
    let mut clock = Clock::default();
    host.frame(&mut clock, true);
    assert!(host.runtime.server.disconnect(client));
    let replacement = host
        .runtime
        .server
        .connect(Connection::Remote, ModuleId(0), PlayerTail::None, None)
        .ok_or("replacement")?;
    assert_eq!(client, replacement);
    let output = host.runtime.server.clients[client.0 as usize]
        .output
        .ok_or("output")?;
    let sequence = host
        .runtime
        .server
        .events
        .print(Some(client), PrintKind::Center, format_args!("new life"))
        .map_err(|e| format!("print {e:?}"))?;
    assert!(host.runtime.server.events.submit(
        output,
        sequence,
        OutputSubmission::Reliable(receipt)
    ));
    send(&mut host, client, Endpoint::Client)?;
    host.frame(&mut clock, true);
    assert_eq!(host.runtime.network.acknowledged, 1);
    assert_eq!(acks(&host, client)?, 0);
    assert_eq!(host.runtime.server.events.len(), 1);
    Ok(())
}
