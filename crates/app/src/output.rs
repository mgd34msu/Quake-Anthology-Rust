//! One client-frame pass, with independent output ownership per consumer.
use crate::{Runtime, host::FrameSource};
use qa_core::{
    events::{FrameEvent, OutputSubmission},
    primitives::{
        ClientId, ModuleId, NativeDelivery, NativeTarget, PrintKind, ResourceNamespace,
        SoundAction, SoundEmitter, Vec3,
    },
    sys_events::{EventTime, SeatId},
};
use qa_session::clients::Connection;

mod config;
pub mod duplicates;
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
    pub native_dropped: u64,
    pub native_disconnected: u64,
    pub native_overflow: u64,
    pub native_resync_retired: u64,
    pub native_dupe_suppressed: u64,
    pub native_dupe_overflow: u64,
}

impl OutputCounts {
    fn add_native(&mut self, submitted: &crate::transport::Submission) {
        self.native_blocked += u64::from(submitted.blocked);
        self.native_dropped += u64::from(submitted.dropped);
        self.native_packets += submitted.packets;
    }
}

fn complete_sound(
    clients: &mut [qa_session::clients::Client],
    snapshots: &mut [Option<crate::snapshots::SnapshotBinding>],
    owner: qa_core::events::OutputConsumerId,
    sequence: u64,
    event: qa_core::primitives::SoundEvent,
    accepted: bool,
) {
    let (Some(id), Some(lifetime)) = (event.client, event.client_lifetime) else {
        return;
    };
    if event.action == SoundAction::RetireClient {
        if let Some(client) = clients.get_mut(id.0 as usize)
            && client.audio_retirement.is_some_and(|pending| {
                pending.lifetime == lifetime && pending.publication == Some((owner, sequence))
            })
        {
            client.audio_retirement = None;
        }
    } else if let Some(snapshot) = snapshots.get_mut(id.0 as usize).and_then(Option::as_mut) {
        snapshot.loop_completed(owner, sequence, event, accepted);
    }
}

pub fn dispatch(
    runtime: &mut Runtime,
    source: &mut impl FrameSource,
    scrollback: &mut qa_console::presentation::Scrollback,
    local: &[Option<ClientId>; SeatId::COUNT],
    time: EventTime,
    notify_time: f64,
    center_time: f64,
) -> OutputCounts {
    let mut counts = OutputCounts::default();
    // Teardown must finish before borrowing SERVER state for the output pass.
    for slot in 0..runtime.server.clients.len() {
        let client = &runtime.server.clients[slot];
        if client.connection == Some(Connection::Remote)
            && let Some(output) = client.output
            && runtime.server.events.needs_resync(output)
        {
            if let Some(counters) = runtime.server.events.counters(output) {
                counts.native_overflow += counters.overflow;
                counts.native_resync_retired += counters.retired_on_resync;
            }
            runtime.disconnect_client(ClientId(slot as u32));
            counts.native_disconnected += 1;
        }
    }
    if runtime
        .server
        .events
        .resume_if_needed(&mut runtime.server.presentation)
    {
        runtime.publish_audio_retirements();
    }
    let server = &mut runtime.server;
    let snapshots = &mut runtime.snapshots;
    runtime
        .output_duplicates
        .sweep(&server.events, |client, owner, sequence, receipt| {
            runtime
                .network
                .get(client, qa_core::loopback::Endpoint::Server)
                .is_some_and(|connection| {
                    connection.output == Some(owner)
                        && (receipt.is_some()
                            || connection.pending_output == Some(sequence)
                                && (connection.channel.pending_packet().is_some()
                                    || connection.channel.pending_fragments()))
                })
        });
    let output_range = server.events.sequence_range();
    let now = time.seconds();
    // In-process presentation resynchronization needs no native channel reset.
    if server.events.resume_if_needed(&mut server.presentation) {
        qa_core::events::dispatch_consumer(
            &mut server.events,
            server.presentation,
            &mut |record: qa_core::events::OutputRecord, texts: &mut qa_core::events::TextStore| {
                let submitted = match record.event {
                    FrameEvent::Sound(event) => {
                        counts.sounds += 1;
                        let sent = source.sound(event);
                        counts.unhandled_sounds += u64::from(!sent);
                        complete_sound(
                            server.clients.as_mut(),
                            snapshots,
                            server.presentation,
                            record.sequence,
                            event,
                            sent,
                        );
                        // Ordinary missing/rejecting backends remain counted
                        // drops. Loop rejection invalidates only its producer's
                        // settled settings through the same completion.
                        true
                    }
                    FrameEvent::Effect(event) => {
                        counts.effects += 1;
                        let sent = source.effect(event);
                        counts.unhandled_effects += u64::from(!sent);
                        true
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
                                // Keep the original ring text alive for this
                                // independent display lifetime; no payload copy.
                                if !scrollback.append(texts, event.text) {
                                    counts.stale_texts += 1;
                                }
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
                    FrameEvent::ConfigString { .. } => true,
                    FrameEvent::StaticEntity(_) => true,
                };
                if submitted {
                    OutputSubmission::BestEffort
                } else {
                    OutputSubmission::Unsent
                }
            },
        );
    }
    let client_count = server.clients.len();
    for slot in 0..client_count {
        let client = &mut server.clients[slot];
        let Some(id) = client.output else {
            continue;
        };
        let client_id = ClientId(slot as u32);
        let is_local = local.contains(&Some(client_id));
        if let Some(protocol) = runtime
            .network
            .get(client_id, qa_core::loopback::Endpoint::Server)
            .and_then(|connection| connection.commands.as_ref())
            .map(|commands| commands.protocol)
        {
            let native_module = server.entities.columns.native_entity[client.entity.slot as usize]
                .map(|native| native.module);
            runtime.native_numbers[slot].refresh(&server.entities, native_module, protocol);
        }
        // A local client still has a native channel. During bounded resync,
        // finish its retained datagram/reliable flight under the old binding.
        // Commands and snapshots keep that binding until the flight is drained.
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
                    runtime.output_duplicates.submitted(
                        id,
                        sequence,
                        OutputSubmission::BestEffort,
                        false,
                        false,
                    );
                    server
                        .events
                        .submit(id, sequence, OutputSubmission::BestEffort);
                    continue;
                }
            };
            counts.add_native(&submitted);
            runtime.output_duplicates.submitted(
                id,
                sequence,
                disposition.unwrap_or(OutputSubmission::Unsent),
                submitted.complete,
                connection.pending_output == Some(sequence),
            );
            if let Some(disposition) = disposition {
                server.events.submit(id, sequence, disposition);
            } else if connection.pending_output.is_some() {
                continue;
            }
        }
        let stopped = std::cell::Cell::new(false);
        let mut waiting_resources = false;
        qa_core::events::dispatch_consumer_until(
            &mut server.events,
            id,
            &mut |record: qa_core::events::OutputRecord, texts: &mut qa_core::events::TextStore| {
                // Resource-dependent services keep their source order while
                // allowing later configstrings to publish their prerequisites.
                if waiting_resources && !matches!(record.event, FrameEvent::ConfigString { .. }) {
                    return OutputSubmission::Unsent;
                }
                let mut resource_pending = false;
                let submission = if let FrameEvent::ConfigString {
                    destination_index,
                    text,
                    ..
                } = record.event
                {
                    if let (Some(text), Some(connection)) = (
                        texts.get(text),
                        runtime
                            .network
                            .get_mut(client_id, qa_core::loopback::Endpoint::Server),
                    ) {
                        if let Some(commands) = &connection.commands {
                            match config::submit(
                                &mut connection.channel,
                                commands.protocol,
                                record.sequence,
                                destination_index,
                                text,
                                &mut runtime.config_publications[slot].pending,
                            ) {
                                Ok(submission) => submission,
                                Err(()) => {
                                    counts.native_unsupported += 1;
                                    OutputSubmission::BestEffort
                                }
                            }
                        } else {
                            OutputSubmission::BestEffort
                        }
                    } else {
                        counts.stale_texts += 1;
                        OutputSubmission::BestEffort
                    }
                } else if let Some(delivery) = record.delivery {
                    if !delivery_visible(
                        runtime.collision.as_mut(),
                        client.player.body.position,
                        delivery,
                    ) {
                        counts.native_filtered += 1;
                        OutputSubmission::BestEffort
                    } else if let Some(connection) = runtime
                        .network
                        .get_mut(client_id, qa_core::loopback::Endpoint::Server)
                    {
                        let route = connection.route;
                        let group = runtime
                            .output_duplicates
                            .group(client_id, client.connection == Some(Connection::Local));
                        match runtime.output_duplicates.admit(
                            record,
                            client_id,
                            id,
                            group,
                            output_range.clone(),
                        ) {
                            duplicates::Admission::Send => (),
                            duplicates::Admission::Pending => {
                                stopped.set(true);
                                return OutputSubmission::Unsent;
                            }
                            duplicates::Admission::Suppressed => {
                                counts.native_dupe_suppressed += 1;
                                return OutputSubmission::BestEffort;
                            }
                            duplicates::Admission::Full => {
                                counts.native_dupe_overflow += 1;
                                return OutputSubmission::BestEffort;
                            }
                        }
                        let mut completed = false;
                        let submission = (|| match record.event {
                            FrameEvent::Native(payload) => {
                                if let Some(bytes) =
                                    texts.copy_payload(payload, &mut runtime.output_bytes)
                                {
                                    counts.native_messages += 1;
                                    match native::submit(
                                        connection,
                                        record.sequence,
                                        delivery,
                                        &bytes[..payload.len()],
                                        &mut runtime.output_native_bytes,
                                        time,
                                        client.module == delivery.module,
                                        |cue, offset| {
                                            project_cue(
                                                cue,
                                                delivery.module,
                                                &server.entities,
                                                &mut runtime.native_numbers[slot],
                                                payload.cue(
                                                    bytes,
                                                    offset + delivery.component as usize,
                                                ),
                                            )
                                        },
                                        |bytes| {
                                            crate::transport::admit_output(
                                                &mut runtime.loopback,
                                                source,
                                                route,
                                                bytes,
                                            )
                                        },
                                    ) {
                                        Ok((submission, submitted, unsupported)) => {
                                            completed = submitted.complete;
                                            counts.native_unsupported += unsupported;
                                            counts.add_native(&submitted);
                                            submission
                                        }
                                        Err(()) => {
                                            counts.native_unsupported += 1;
                                            OutputSubmission::BestEffort
                                        }
                                    }
                                } else {
                                    counts.stale_texts += 1;
                                    OutputSubmission::BestEffort
                                }
                            }
                            FrameEvent::Print(print) => {
                                match native::submit_print(
                                    connection,
                                    record.sequence,
                                    Some(delivery),
                                    print,
                                    texts,
                                    &mut runtime.output_bytes,
                                    &mut runtime.output_native_bytes,
                                    time,
                                    client.module == delivery.module,
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
                                        completed = submitted.complete;
                                        counts.prints += 1;
                                        counts.add_native(&submitted);
                                        submission
                                    }
                                    Err(()) => {
                                        counts.native_unsupported += 1;
                                        OutputSubmission::BestEffort
                                    }
                                }
                            }
                            FrameEvent::Sound(_) | FrameEvent::StaticEntity(_) => {
                                let encoded = connection
                                    .commands
                                    .as_ref()
                                    .ok_or(qa_network::outputs::Error::Unsupported)
                                    .and_then(|commands| {
                                        encode_record(
                                            record.event,
                                            delivery,
                                            commands.protocol,
                                            client_id,
                                            client.module,
                                            &server.entities,
                                            &mut runtime.native_numbers[slot],
                                            &runtime.config_publications[slot],
                                            &mut runtime.output_native_bytes,
                                        )
                                    });
                                match encoded {
                                    Err(qa_network::outputs::Error::Pending) => {
                                        waiting_resources = true;
                                        resource_pending = true;
                                        OutputSubmission::Unsent
                                    }
                                    Err(_) => {
                                        counts.native_unsupported += 1;
                                        OutputSubmission::BestEffort
                                    }
                                    Ok(length) => match native::submit_encoded(
                                        connection,
                                        record.sequence,
                                        delivery,
                                        &runtime.output_native_bytes[..length],
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
                                            completed = submitted.complete;
                                            counts.add_native(&submitted);
                                            submission
                                        }
                                        Err(()) => {
                                            counts.native_unsupported += 1;
                                            OutputSubmission::BestEffort
                                        }
                                    },
                                }
                            }
                            _ => {
                                counts.native_unsupported += 1;
                                OutputSubmission::BestEffort
                            }
                        })();
                        runtime.output_duplicates.submitted(
                            id,
                            record.sequence,
                            submission,
                            completed,
                            connection.pending_output == Some(record.sequence),
                        );
                        submission
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
                    if let FrameEvent::Print(print) = record.event
                        && let Some(connection) = runtime
                            .network
                            .get_mut(client_id, qa_core::loopback::Endpoint::Server)
                    {
                        let route = connection.route;
                        match native::submit_print(
                            connection,
                            record.sequence,
                            None,
                            print,
                            texts,
                            &mut runtime.output_bytes,
                            &mut runtime.output_native_bytes,
                            time,
                            false,
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
                } else {
                    // No presentation is bound to this client. Inapplicable
                    // records must not hold the shared publication head.
                    OutputSubmission::BestEffort
                };
                stopped.set(
                    submission == OutputSubmission::Unsent && !resource_pending
                        || runtime
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
        if is_local
            && server.events.needs_resync(id)
            && runtime
                .network
                .get(client_id, qa_core::loopback::Endpoint::Server)
                .is_none_or(|connection| {
                    connection.pending_output.is_none() && !connection.channel.has_output()
                })
            && let Some(resumed) = server.events.resume(id)
        {
            // Keep channel sequence/receipt counters: an old ACK cannot name a
            // new receipt. Update every local lifetime guard in the same pass.
            client.output = Some(resumed);
            if let Some(connection) = runtime
                .network
                .get_mut(client_id, qa_core::loopback::Endpoint::Server)
            {
                connection.output = Some(resumed);
            }
            if let Some(snapshot) = runtime.snapshots[slot].as_mut() {
                snapshot.output = resumed;
            }
            // Rebuild discrete HUD state from CLIENT on its next publication.
            // Independently leased text stays valid through the resync.
            client.hud.reset_inventory();
            client.hud_publication.reset();
            runtime.config_publications[slot].reset();
            runtime.native_numbers[slot].reset();
        }
    }
    counts
}

fn delivery_visible(
    collision: Option<&mut crate::WorldCollision>,
    listener: Vec3,
    delivery: NativeDelivery,
) -> bool {
    match delivery.target {
        NativeTarget::All | NativeTarget::Client(_) => true,
        target => collision.is_some_and(|collision| {
            let Some(world) = collision.visibility.as_deref() else {
                return false;
            };
            let policy = qa_world::visibility::PointVisibility {
                channel: if target == NativeTarget::Hearing {
                    qa_world::visibility::VisibilityChannel::Hearing
                } else {
                    qa_world::visibility::VisibilityChannel::Sight
                },
                missing_visible: false,
                area_zero: true,
            };
            world
                .test_points(
                    delivery.origin,
                    listener,
                    &mut collision.visibility_row,
                    policy,
                    collision
                        .portals
                        .as_ref()
                        .filter(|_| delivery.check_portals),
                )
                .unwrap_or(false)
        }),
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "Native field projection uses the existing independently owned client/resource/entity boundaries."
)]
fn encode_record(
    event: FrameEvent,
    delivery: NativeDelivery,
    protocol: qa_network::commands::packet::Protocol,
    client: ClientId,
    client_module: ModuleId,
    entities: &qa_world::entities::EntityTable,
    numbers: &mut crate::output_namespace::NativeNumbers,
    publication: &crate::client_resources::ConfigPublication,
    output: &mut [u8],
) -> Result<usize, qa_network::outputs::Error> {
    use crate::client_resources::ResourceReadiness;
    use qa_network::{commands::packet::Protocol, outputs::Error};
    let (ordinal, readiness) = match event {
        FrameEvent::Sound(sound) => publication
            .project_sound(delivery.module, sound.source_ordinal, sound.sound)
            .ok_or(Error::Unsupported)?,
        FrameEvent::StaticEntity(record) => {
            if !matches!(protocol, Protocol::NetQuake15 | Protocol::QuakeWorld28) {
                return Err(Error::Unsupported);
            }
            let layout = protocol.config_layout().ok_or(Error::Unsupported)?;
            if let Some(reference) = record.appearance.models[0] {
                let ordinal = crate::client_resources::model_ordinal(
                    reference,
                    Some(client_module),
                    client,
                    layout.resources[0].count,
                )
                .ok_or(Error::Unsupported)?;
                (
                    ordinal,
                    publication.readiness(layout.resources[0].first as usize + ordinal as usize),
                )
            } else {
                (0, ResourceReadiness::Ready)
            }
        }
        _ => return Err(Error::Unsupported),
    };
    match readiness {
        ResourceReadiness::Pending => return Err(Error::Pending),
        ResourceReadiness::Unsupported => return Err(Error::Unsupported),
        ResourceReadiness::Ready => {}
    }
    match event {
        FrameEvent::Sound(sound) => {
            let entity = sound_entity(entities, numbers, sound, delivery.module);
            if sound.entity.is_some()
                && entity.is_none()
                && !matches!(sound.entity, Some(SoundEmitter::Static { .. }))
            {
                return Err(Error::Unsupported);
            }
            let positioned = delivery.reliable && protocol == Protocol::Quake2Repro1038;
            qa_network::outputs::sound(protocol, sound, ordinal, entity, positioned, output)
        }
        FrameEvent::StaticEntity(record) => {
            qa_network::outputs::static_entity(protocol, &record, |_| Some(ordinal), output)
        }
        _ => Err(Error::Unsupported),
    }
}

fn sound_entity(
    entities: &qa_world::entities::EntityTable,
    numbers: &mut crate::output_namespace::NativeNumbers,
    sound: qa_core::primitives::SoundEvent,
    module: ModuleId,
) -> Option<u32> {
    match sound.entity? {
        SoundEmitter::Entity(id) => {
            entities.resolve(id)?;
            if let Some(fields) = sound.native_fields
                && numbers.is_native_module(module)
            {
                Some(u32::from(fields.entity))
            } else {
                numbers.project(entities, id)
            }
        }
        SoundEmitter::Native {
            namespace: ResourceNamespace::Module(owner),
            slot,
        } if owner == module => {
            if let Some(fields) = sound.native_fields {
                numbers
                    .is_native_module(module)
                    .then_some(u32::from(fields.entity))
            } else {
                numbers.native(entities, owner, slot)
            }
        }
        _ => None,
    }
}
fn project_cue(
    cue: qa_network::outputs::Cue,
    module: ModuleId,
    entities: &qa_world::entities::EntityTable,
    numbers: &mut crate::output_namespace::NativeNumbers,
    captured: Option<qa_core::primitives::NativeCueBinding>,
) -> Result<qa_network::outputs::Cue, qa_network::outputs::Error> {
    use qa_network::outputs::{Cue, Error};
    let native_entity_namespace = numbers.is_native_module(module);
    Ok(match cue {
        Cue::Sound(_) | Cue::StaticSound(_) => return Err(Error::Unsupported),
        Cue::MuzzleFlash(mut flash) => {
            flash.entity = u16::try_from(if native_entity_namespace {
                u32::from(flash.entity)
            } else {
                numbers
                    .project(
                        entities,
                        captured
                            .and_then(|binding| binding.entity)
                            .ok_or(Error::Unsupported)?,
                    )
                    .ok_or(Error::Unsupported)?
            })
            .map_err(|_| Error::Unsupported)?;
            Cue::MuzzleFlash(flash)
        }
    })
}
