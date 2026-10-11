//! One client-frame pass, with independent output ownership per consumer.
use crate::{Runtime, host::FrameSource};
use qa_core::{
    events::{FrameEvent, OutputSubmission},
    primitives::{ClientId, PrintKind},
    sys_events::{EventTime, SeatId},
};
use qa_session::clients::Connection;

mod native;

#[derive(Default)]
pub struct OutputCounts {
    pub sounds: u64,
    pub effects: u64,
    pub prints: u64,
    pub hud: u64,
    pub native_messages: u64,
    pub native_filtered: u64,
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

impl OutputCounts {
    fn add_native(&mut self, submitted: &crate::transport::Submission) {
        self.native_blocked += u64::from(submitted.blocked);
        self.native_packets += submitted.packets;
    }
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
    let now = time.seconds();
    let mut counts = OutputCounts::default();
    // In-process presentation resynchronization needs no native channel reset.
    if server.events.needs_resync(server.presentation)
        && let Some(id) = server.events.resume(server.presentation)
    {
        server.presentation = id;
    }
    qa_core::events::dispatch_consumer(
        &mut server.events,
        server.presentation,
        &mut |record: qa_core::events::OutputRecord, texts: &mut qa_core::events::TextStore| {
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
                    if let Some(text) = texts.get(event.text) {
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
                // Native messages have per-client channel delivery below;
                // there is no second presentation copy of their payload.
                FrameEvent::Native(_) => true,
                FrameEvent::Hud(_) => true,
            };
            if submitted {
                OutputSubmission::BestEffort
            } else {
                OutputSubmission::Unsent
            }
        },
    );
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
        if let Some(connection) = runtime
            .network
            .get_mut(client_id, qa_core::loopback::Endpoint::Server)
            && let Some(sequence) = connection.pending_output
        {
            let route = connection.route;
            let (disposition, submitted) = match native::transmit(connection, None, time, |bytes| {
                crate::transport::admit_output(&mut runtime.loopback, source, route, bytes)
            }) {
                Ok(submitted) => submitted,
                Err(_) => {
                    counts.native_unsupported += 1;
                    server
                        .events
                        .submit(id, sequence, OutputSubmission::BestEffort);
                    continue;
                }
            };
            counts.add_native(&submitted);
            let Some(disposition) = disposition else {
                continue;
            };
            server.events.submit(id, sequence, disposition);
        }
        let stopped = std::cell::Cell::new(false);
        qa_core::events::dispatch_consumer_until(
            &mut server.events,
            id,
            &mut |record: qa_core::events::OutputRecord, texts: &mut qa_core::events::TextStore| {
                let submission = if let FrameEvent::Native(message) = record.event {
                    let visible = match message.delivery.target {
                        qa_core::primitives::NativeTarget::All
                        | qa_core::primitives::NativeTarget::Client(_) => true,
                        target => runtime.collision.as_mut().is_some_and(|collision| {
                            let Some(world) = collision.visibility.as_deref() else {
                                return false;
                            };
                            let policy = qa_world::visibility::PointVisibility {
                                channel: if target == qa_core::primitives::NativeTarget::Hearing {
                                    qa_world::visibility::VisibilityChannel::Hearing
                                } else {
                                    qa_world::visibility::VisibilityChannel::Sight
                                },
                                missing_visible: false,
                                area_zero: true,
                            };
                            world
                                .test_points(
                                    message.delivery.origin,
                                    client.player.body.position,
                                    &mut collision.visibility_row,
                                    policy,
                                    collision
                                        .portals
                                        .as_ref()
                                        .filter(|_| message.delivery.check_portals),
                                )
                                .unwrap_or(false)
                        }),
                    };
                    if !visible {
                        counts.native_filtered += 1;
                        OutputSubmission::BestEffort
                    } else if let Some(bytes) =
                        texts.copy_payload(message.payload, &mut runtime.output_bytes)
                        && let Some(connection) = runtime
                            .network
                            .get_mut(client_id, qa_core::loopback::Endpoint::Server)
                    {
                        counts.native_messages += 1;
                        let route = connection.route;
                        match native::submit(
                            connection,
                            record.sequence,
                            message,
                            bytes,
                            time,
                            |bytes| {
                                crate::transport::admit_output(
                                    &mut runtime.loopback,
                                    source,
                                    route,
                                    bytes,
                                )
                            },
                        ) {
                            Ok((submission, submitted)) => {
                                counts.add_native(&submitted);
                                submission
                            }
                            Err(()) => {
                                counts.native_unsupported += 1;
                                OutputSubmission::BestEffort
                            }
                        }
                    } else {
                        counts.native_unsupported += 1;
                        OutputSubmission::BestEffort
                    }
                } else if is_local {
                    // Audio/particles have one presentation consumer. Each HUD
                    // independently leases its text before retiring its record.
                    let success = match record.event {
                        FrameEvent::Hud(event) => {
                            counts.hud += 1;
                            if !runtime.catalog.hud.apply(&mut client.hud, event.change) {
                                client.hud.clipped_values += 1;
                            }
                            true
                        }
                        FrameEvent::Print(event) => qa_ui::hud::print(
                            &mut client.hud,
                            texts,
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
                        FrameEvent::Print(p) => texts.get(p.text),
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
                                OutputSubmission::BestEffort
                            }
                        }
                    } else {
                        counts.native_unsupported += 1;
                        OutputSubmission::BestEffort
                    }
                } else {
                    // No presentation is bound to this client. Inapplicable
                    // records must not hold the shared publication head.
                    OutputSubmission::BestEffort
                };
                stopped.set(
                    runtime
                        .network
                        .get(client_id, qa_core::loopback::Endpoint::Server)
                        .is_some_and(|connection| connection.pending_output.is_some()),
                );
                submission
            },
            || stopped.get(),
        );
        if is_local {
            qa_ui::hud::expire_messages(&mut client.hud, &mut server.events.texts, now);
        }
        if (client.connection == Some(Connection::Remote) || is_local)
            && let Some(connection) = runtime
                .network
                .get_mut(client_id, qa_core::loopback::Endpoint::Server)
            && connection.pending_output.is_none()
        {
            let submitted = crate::transport::send(
                &mut connection.channel,
                time,
                &mut (),
                |channel, ()| {
                    channel.has_output()
                        && channel
                            .prepare_output(time)
                            .is_ok_and(|packet| packet.is_some())
                },
                |bytes| {
                    crate::transport::admit_output(
                        &mut runtime.loopback,
                        source,
                        connection.route,
                        bytes,
                    )
                },
                // One ordinary channel send per CLIENT output pass. Additional
                // sends here are only the already-queued native control records.
                |channel, (), _| channel.pending_controls() == 0,
            );
            counts.add_native(&submitted);
        }
    }
    counts
}
