//! One client-frame pass, with independent output ownership per consumer.
use crate::{Runtime, host::FrameSource};
use qa_core::{
    events::{FrameEvent, OutputSubmission},
    primitives::{ClientId, PrintKind},
    sys_events::{EventTime, SeatId},
};
use qa_session::clients::Connection;

#[derive(Default)]
pub struct OutputCounts {
    pub sounds: u64,
    pub effects: u64,
    pub prints: u64,
    pub unhandled_sounds: u64,
    pub unhandled_effects: u64,
    pub stale_texts: u64,
    pub native_unsupported: u64,
    pub native_packets: u64,
    pub native_blocked: u64,
    pub native_disconnected: u64,
    pub native_overflow: u64,
    pub native_resync_retired: u64,
}

pub fn dispatch(
    runtime: &mut Runtime,
    source: &mut impl FrameSource,
    local: &[Option<ClientId>; SeatId::COUNT],
    time: EventTime,
    notify_time: f64,
    center_time: f64,
) -> OutputCounts {
    let server = &mut runtime.server;
    let now = time.0 as f64 * 1e-9;
    let mut counts = OutputCounts::default();
    // In-process presentation resynchronization needs no native channel reset.
    if server.events.needs_resync(server.presentation)
        && let Some(id) = server.events.resume(server.presentation)
    {
        server.presentation = id;
    }
    if let Some(mut batch) = server.events.batch(server.presentation) {
        while let Some(record) = server.events.next(&mut batch) {
            let submitted = match record.event {
                FrameEvent::Sound(event) => {
                    counts.sounds += 1;
                    let sent = source.sound(event);
                    counts.unhandled_sounds += u64::from(!sent);
                    sent
                }
                FrameEvent::Effect(event) => {
                    counts.effects += 1;
                    let sent = source.effect(event);
                    counts.unhandled_effects += u64::from(!sent);
                    sent
                }
                FrameEvent::Print(event) => {
                    counts.prints += 1;
                    if let Some(text) = server.events.texts.get(event.text) {
                        if event.client.is_none_or(|id| local.contains(&Some(id)))
                            && matches!(
                                event.kind,
                                PrintKind::Console | PrintKind::Notify | PrintKind::Chat
                            )
                        {
                            qa_console::logger::console_bytes(text);
                        }
                    } else {
                        counts.stale_texts += 1;
                    }
                    true
                }
            };
            server.events.submit(
                server.presentation,
                record.sequence,
                if submitted {
                    OutputSubmission::BestEffort
                } else {
                    OutputSubmission::Unsent
                },
            );
        }
    }
    for slot in 0..server.clients.len() {
        let client = &mut server.clients[slot];
        let Some(mut id) = client.output else {
            continue;
        };
        let client_id = ClientId(slot as u32);
        let is_local = local.contains(&Some(client_id));
        if server.events.needs_resync(id) && client.connection == Some(Connection::Remote) {
            // A slow peer cannot hold publication or another peer's native ACK.
            // Drop its connection rather than invent a successful native reset.
            if let Some(counters) = server.events.counters(id) {
                counts.native_overflow += counters.overflow;
                counts.native_resync_retired += counters.retired_on_resync;
            }
            runtime.network.unbind(client_id);
            runtime.loopback.clear_client(client_id);
            server.disconnect(client_id);
            counts.native_disconnected += 1;
            continue;
        }
        if server.events.needs_resync(id) && is_local {
            let Some(resumed) = server.events.resume(id) else {
                continue;
            };
            client.output = Some(resumed);
            id = resumed;
        }
        if let Some(mut batch) = server.events.batch(id) {
            while let Some(record) = server.events.next(&mut batch) {
                let submission = if is_local {
                    // Audio/particles have one presentation consumer. Each HUD
                    // independently leases its text before retiring its record.
                    let success = match record.event {
                        FrameEvent::Print(event) => qa_ui::hud::print(
                            &mut client.hud,
                            &mut server.events.texts,
                            event,
                            now,
                            notify_time,
                            center_time,
                        ),
                        _ => true,
                    };
                    if success {
                        OutputSubmission::BestEffort
                    } else {
                        OutputSubmission::Unsent
                    }
                } else if client.connection == Some(Connection::Remote) {
                    let text = match record.event {
                        FrameEvent::Print(p) => server.events.texts.get(p.text),
                        _ => None,
                    };
                    if let FrameEvent::Print(print) = record.event
                        && let Some(text) = text
                        && let Some(connection) = runtime
                            .network
                            .get_mut(client_id, qa_core::loopback::Endpoint::Server)
                        && let Some(commands) = &connection.commands
                    {
                        let mut bytes = [0; 8192];
                        match qa_network::outputs::print(
                            commands.protocol,
                            print.kind,
                            print.level,
                            text,
                            &mut bytes,
                        ) {
                            Ok(n) => match connection.channel.queue_reliable(&bytes[..n]) {
                                Ok(receipt) => OutputSubmission::Reliable(receipt),
                                Err(_) => OutputSubmission::Unsent,
                            },
                            Err(_) => {
                                counts.native_unsupported += 1;
                                OutputSubmission::Unsent
                            }
                        }
                    } else {
                        counts.native_unsupported += 1;
                        OutputSubmission::Unsent
                    }
                } else {
                    // A local client without a bound seat is not delivered yet.
                    OutputSubmission::Unsent
                };
                server.events.submit(id, record.sequence, submission);
            }
        }
        if is_local {
            qa_ui::hud::expire_messages(&mut client.hud, &mut server.events.texts, now);
        }
        if client.connection == Some(Connection::Remote)
            && let Some(connection) = runtime
                .network
                .get_mut(client_id, qa_core::loopback::Endpoint::Server)
        {
            // Controls/queued fragments are bounded at load. No physical intake
            // occurs here; rejected bytes retain their prepared channel state.
            for _ in 0..17 {
                if !connection.channel.has_output() {
                    break;
                }
                if connection.channel.pending_packet().is_none() {
                    let Ok(Some(_)) = connection.channel.prepare_output(time) else {
                        break;
                    };
                }
                let Ok(Some(_)) =
                    connection
                        .channel
                        .submit_with(time, |bytes| match connection.route.peer {
                            qa_core::sys_events::Peer::Loopback(client) => runtime
                                .loopback
                                .send(qa_core::loopback::Endpoint::Server, client, bytes)
                                .is_ok(),
                            qa_core::sys_events::Peer::Socket(to) => {
                                source.send_packet(connection.route.socket, to, bytes)
                            }
                        })
                else {
                    counts.native_blocked += 1;
                    break;
                };
                counts.native_packets += 1;
                // One ordinary channel send per CLIENT output pass. Additional
                // sends here are only the already-queued native control records.
                if connection.channel.pending_controls() == 0 {
                    break;
                }
            }
        }
    }
    counts
}
