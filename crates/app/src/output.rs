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
mod peers;

#[derive(Default)]
pub struct OutputCounts {
    pub sounds: u64,
    pub music: u64,
    pub unhandled_music: u64,
    pub effects: u64,
    pub prints: u64,
    pub hud: u64,
    pub view_feedback: u64,
    pub presentation_resets: u64,
    pub unhandled_presentation_resets: u64,
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
    let counts = retire_resync_clients(runtime, &mut crate::modules::Driver::cache_only());
    dispatch_frame(
        runtime,
        source,
        scrollback,
        local,
        time,
        notify_time,
        center_time,
        counts,
        |_, _, _, _| None,
    )
}

pub(crate) trait RetirementOwner {
    fn runtime(&mut self) -> &mut Runtime;
    fn retire(&mut self, client: ClientId, driver: &mut crate::modules::Driver<'_>);
}
impl RetirementOwner for Runtime {
    fn runtime(&mut self) -> &mut Runtime {
        self
    }
    fn retire(&mut self, client: ClientId, _driver: &mut crate::modules::Driver<'_>) {
        self.disconnect_client(client);
    }
}
impl RetirementOwner for crate::host::FrameHost {
    fn runtime(&mut self) -> &mut Runtime {
        &mut self.runtime
    }
    fn retire(&mut self, client: ClientId, driver: &mut crate::modules::Driver<'_>) {
        self.disconnect_client_with_driver(client, driver);
    }
}

pub(crate) fn retire_resync_clients(
    owner: &mut impl RetirementOwner,
    driver: &mut crate::modules::Driver<'_>,
) -> OutputCounts {
    let mut counts = OutputCounts::default();
    // Teardown must finish before borrowing SERVER state for the output pass.
    let clients = owner.runtime().server.clients.len();
    for slot in 0..clients {
        let retire = {
            let runtime = owner.runtime();
            let client = &runtime.server.clients[slot];
            if client.is_active()
                && client.connection == Some(Connection::Remote)
                && let Some(output) = client.output
                && runtime.server.events.needs_resync(output)
            {
                if let Some(counters) = runtime.server.events.counters(output) {
                    counts.native_overflow += counters.overflow;
                    counts.native_resync_retired += counters.retired_on_resync;
                }
                true
            } else {
                false
            }
        };
        if retire {
            owner.retire(ClientId(slot as u32), driver);
            counts.native_disconnected += 1;
        }
    }
    counts
}

pub(crate) fn dispatch_frame(
    runtime: &mut Runtime,
    source: &mut impl FrameSource,
    scrollback: &mut qa_console::presentation::Scrollback,
    local: &[Option<ClientId>; SeatId::COUNT],
    time: EventTime,
    notify_time: f64,
    center_time: f64,
    mut counts: OutputCounts,
    mut bound_model: impl FnMut(
        ModuleId,
        u32,
        Option<qa_core::primitives::GeometryId>,
        qa_core::primitives::ModelUse,
    ) -> Option<crate::client_resources::ModelName>,
) -> OutputCounts {
    if runtime
        .server
        .events
        .resume_if_needed(&mut runtime.server.presentation)
    {
        runtime.publish_audio_retirements();
    }
    runtime.publish_presentation_resets();
    runtime.publish_userinfos();
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
    if source.presentation_ready() && server.events.resume_if_needed(&mut server.presentation) {
        loop {
            let responded = std::cell::Cell::new(false);
            let mut reply = None;
            qa_core::events::dispatch_consumer_until(
                &mut server.events,
                server.presentation,
                &mut |record: qa_core::events::OutputRecord,
                      texts: &mut qa_core::events::TextStore| {
                    let submitted = match record.event {
                        FrameEvent::ResetPresentation { client, lifetime } => {
                            let slot = client.0 as usize;
                            if let Some(row) = server.clients.get_mut(slot)
                                && row.connection == Some(Connection::Local)
                                && row.entity == lifetime
                                && let Some(resources) = runtime
                                    .client_resources
                                    .get_mut(slot)
                                    .and_then(Option::as_mut)
                                && resources.state.complete_presentation_reset(
                                    lifetime,
                                    server.presentation,
                                    record.sequence,
                                )
                            {
                                row.hud.reset(texts);
                                if let Some(seat) = snapshots
                                    .get(slot)
                                    .and_then(Option::as_ref)
                                    .and_then(crate::snapshots::SnapshotBinding::seat)
                                {
                                    runtime.catalog.hud.project_stats(
                                        &runtime.prediction[seat.index()].player,
                                        &mut row.hud,
                                    );
                                }
                                row.hud_publication.reset();
                                row.value_publication.clear();
                                counts.presentation_resets += 1;
                                if !source.reset_presentation(client, lifetime) {
                                    resources.state.request_presentation_reset(lifetime);
                                    counts.unhandled_presentation_resets += 1;
                                }
                            }
                            true
                        }
                        FrameEvent::Music(control) => {
                            let client = match control {
                                qa_core::primitives::MusicControl::Start { owner, .. }
                                | qa_core::primitives::MusicControl::Track { owner, .. } => {
                                    owner.client
                                }
                                _ => None,
                            };
                            if client.is_some_and(|(client, _)| {
                                runtime
                                    .client_resources
                                    .get(client.0 as usize)
                                    .and_then(Option::as_ref)
                                    .is_some_and(|resources| {
                                        !resources.state.presentation_after_reset(record.sequence)
                                    })
                            }) {
                                counts.unhandled_music += 1;
                                return OutputSubmission::BestEffort;
                            }
                            counts.music += 1;
                            if let qa_core::primitives::MusicControl::Report(query) = control {
                                reply = source
                                    .music_report(&runtime.vfs, query, texts)
                                    .map(|lease| (record.sequence, lease));
                                responded.set(reply.is_some());
                                counts.unhandled_music += u64::from(reply.is_none());
                                return if reply.is_some() {
                                    OutputSubmission::Unsent
                                } else {
                                    OutputSubmission::BestEffort
                                };
                            }
                            let paths = match control {
                                qa_core::primitives::MusicControl::Start {
                                    intro,
                                    loop_track,
                                    ..
                                } => (texts.get(intro), texts.get(loop_track)),
                                qa_core::primitives::MusicControl::Remap { entries } => {
                                    (texts.get(entries), None)
                                }
                                _ => (None, None),
                            };
                            counts.unhandled_music +=
                                u64::from(!source.music(&runtime.vfs, control, paths.0, paths.1));
                            true
                        }
                        FrameEvent::Sound(event) => {
                            counts.sounds += 1;
                            let creates_voice =
                                matches!(event.action, SoundAction::Play | SoundAction::StartLoop);
                            let ready = !creates_voice
                                || event.client.is_none_or(|client| {
                                    runtime
                                        .client_resources
                                        .get(client.0 as usize)
                                        .and_then(Option::as_ref)
                                        .is_none_or(|resources| {
                                            resources
                                                .state
                                                .presentation_after_reset(record.sequence)
                                        })
                                });
                            let sent = ready
                                && source.sound(
                                    runtime
                                        .sound_assets
                                        .as_deref()
                                        .map(crate::audio::SoundAssets::bank),
                                    event,
                                );
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
                            let ready = event.client.is_none_or(|client| {
                                runtime
                                    .client_resources
                                    .get(client.0 as usize)
                                    .and_then(Option::as_ref)
                                    .is_none_or(|resources| {
                                        resources.state.presentation_after_reset(record.sequence)
                                    })
                            });
                            let palette = runtime
                                .catalog
                                .registry
                                .effects
                                .definition(event.effect)
                                .filter(|definition| {
                                    definition.source == qa_core::primitives::RuleSetId::Quake
                                })
                                .and_then(|definition| {
                                    runtime
                                        .model_assets
                                        .as_ref()?
                                        .published_palette(definition.source)
                                });
                            let sent = ready && source.effect(palette, event);
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
                        FrameEvent::Native(_)
                        | FrameEvent::GameState { .. }
                        | FrameEvent::PeerInfo { .. }
                        | FrameEvent::FileChunk { .. } => true,
                        FrameEvent::Hud(_) => true,
                        FrameEvent::ViewFeedback(_) => true,
                        FrameEvent::ConfigString { .. } => true,
                        FrameEvent::ServerCommand { .. } => true,
                        FrameEvent::StaticEntity(_) => true,
                        FrameEvent::PlayerValue { .. } | FrameEvent::SetAngles(_) => true,
                    };
                    if submitted {
                        OutputSubmission::BestEffort
                    } else {
                        OutputSubmission::Unsent
                    }
                },
                || responded.get(),
            );
            let Some((sequence, reply)) = reply else {
                break;
            };
            // Fill the original query slot after the consumer borrow ends.
            // Its unchanged sequence keeps it ahead of later console prints.
            if !server
                .events
                .console_response(server.presentation, sequence, reply)
            {
                counts.stale_texts += 1;
                server
                    .events
                    .submit(server.presentation, sequence, OutputSubmission::BestEffort);
            }
        }
    }
    let mut peer_resync = false;
    let client_count = server.clients.len();
    for slot in 0..client_count {
        let Some(id) = server.clients[slot].output else {
            continue;
        };
        let client_id = ClientId(slot as u32);
        let is_local = local.contains(&Some(client_id));
        let client = &mut server.clients[slot];
        // A local client still has a native channel. During bounded resync,
        // finish its retained datagram/reliable flight under the old binding.
        // Commands and snapshots keep that binding until the flight is drained.
        if let Some(connection) = runtime
            .network
            .get_mut(client_id, qa_core::loopback::Endpoint::Server)
            && let Some(sequence) = connection.pending_output
        {
            let route = connection.route;
            let cancelled_download = !runtime.server_downloads[slot].accepts_record(sequence)
                && server.events.batch(id).is_some_and(|mut batch| {
                    while let Some(record) = server.events.next(&mut batch) {
                        if record.sequence == sequence {
                            return matches!(record.event, FrameEvent::FileChunk { .. });
                        }
                    }
                    false
                })
                && connection.channel.cancel_server_message();
            let (disposition, submitted) = if cancelled_download {
                connection.pending_output = None;
                (
                    Some(OutputSubmission::BestEffort),
                    crate::transport::Submission {
                        dropped: true,
                        ..Default::default()
                    },
                )
            } else {
                match native::transmit(connection, None, time, |bytes| {
                    crate::transport::admit_output(
                        &mut runtime.loopback,
                        &mut |socket, to, bytes| source.send_packet(socket, to, bytes),
                        qa_core::loopback::Endpoint::Server,
                        route,
                        bytes,
                    )
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
                if !client.is_active()
                    && !matches!(
                        record.event,
                        FrameEvent::ServerCommand { .. }
                            | FrameEvent::ConfigString { .. }
                            | FrameEvent::GameState { .. }
                            | FrameEvent::FileChunk { .. }
                    )
                {
                    counts.native_unsupported += 1;
                    return OutputSubmission::BestEffort;
                }
                // Resource-dependent services keep their source order while
                // allowing later configstrings to publish their prerequisites.
                if waiting_resources
                    && !matches!(
                        record.event,
                        FrameEvent::ConfigString { .. } | FrameEvent::FileChunk { .. }
                    )
                {
                    return OutputSubmission::Unsent;
                }
                let mut resource_pending = false;
                let submission = if let FrameEvent::FileChunk { receiver, chunk } = record.event {
                    if receiver != client.entity
                        || !runtime.server_downloads[slot].accepts_record(record.sequence)
                    {
                        return OutputSubmission::BestEffort;
                    }
                    let stored = if let Some(payload) = chunk.payload() {
                        let Some(stored) = texts.copy_payload(*payload, &mut runtime.output_bytes)
                        else {
                            counts.stale_texts += 1;
                            runtime.server_downloads[slot].reset();
                            return OutputSubmission::BestEffort;
                        };
                        stored
                    } else {
                        &[]
                    };
                    let chunk = chunk.map(|payload| &stored[..payload.len()]);
                    let Some(connection) = runtime
                        .network
                        .get_mut(client_id, qa_core::loopback::Endpoint::Server)
                    else {
                        runtime.server_downloads[slot].reset();
                        return OutputSubmission::BestEffort;
                    };
                    let Some(protocol) = connection
                        .commands
                        .as_ref()
                        .map(|commands| commands.protocol)
                    else {
                        runtime.server_downloads[slot].reset();
                        return OutputSubmission::BestEffort;
                    };
                    if protocol == qa_network::commands::packet::Protocol::Quake3_68 {
                        let length = connection.channel.encode_server_output(
                            &mut runtime.output_native_bytes,
                            |writer, _| {
                                qa_network::downloads::write_service(protocol, chunk, writer)
                                    .map_err(|_| qa_network::commands::packet::Error::Context)
                            },
                        );
                        let Ok(length) = length else {
                            counts.native_unsupported += 1;
                            runtime.server_downloads[slot].reset();
                            return OutputSubmission::BestEffort;
                        };
                        let route = connection.route;
                        match native::transmit(
                            connection,
                            Some((record.sequence, &runtime.output_native_bytes[..length])),
                            time,
                            |bytes| {
                                crate::transport::admit_output(
                                    &mut runtime.loopback,
                                    &mut |socket, to, bytes| source.send_packet(socket, to, bytes),
                                    qa_core::loopback::Endpoint::Server,
                                    route,
                                    bytes,
                                )
                            },
                        ) {
                            Ok((submission, sent)) => {
                                counts.add_native(&sent);
                                counts.native_messages += u64::from(sent.complete);
                                submission.unwrap_or(OutputSubmission::Unsent)
                            }
                            Err(_) => {
                                counts.native_unsupported += 1;
                                runtime.server_downloads[slot].reset();
                                OutputSubmission::BestEffort
                            }
                        }
                    } else {
                        let Ok(length) = qa_network::downloads::write_chunk(
                            protocol,
                            chunk,
                            &mut runtime.output_native_bytes,
                        ) else {
                            counts.native_unsupported += 1;
                            runtime.server_downloads[slot].reset();
                            return OutputSubmission::BestEffort;
                        };
                        match native::reliable(
                            &mut connection.channel,
                            &runtime.output_native_bytes[..length],
                        ) {
                            Ok(submission) => {
                                counts.native_messages +=
                                    u64::from(submission != OutputSubmission::Unsent);
                                submission
                            }
                            Err(()) => {
                                counts.native_unsupported += 1;
                                runtime.server_downloads[slot].reset();
                                OutputSubmission::BestEffort
                            }
                        }
                    }
                } else if let FrameEvent::PeerInfo { user_id, payload } = record.event {
                    let Some(stored) = texts.copy_payload(payload, &mut runtime.output_bytes)
                    else {
                        counts.stale_texts += 1;
                        return OutputSubmission::BestEffort;
                    };
                    let Some(route) = payload.peer(stored, client.entity) else {
                        return OutputSubmission::BestEffort;
                    };
                    let Some(connection) = runtime
                        .network
                        .get_mut(client_id, qa_core::loopback::Endpoint::Server)
                    else {
                        return OutputSubmission::BestEffort;
                    };
                    let Some(protocol) = connection
                        .commands
                        .as_ref()
                        .map(|commands| commands.protocol)
                    else {
                        return OutputSubmission::BestEffort;
                    };
                    let Ok(length) = peers::encode(
                        protocol,
                        route,
                        user_id,
                        &stored[..payload.len()],
                        &mut runtime.output_native_bytes,
                    ) else {
                        counts.native_unsupported += 1;
                        return OutputSubmission::BestEffort;
                    };
                    match native::reliable(
                        &mut connection.channel,
                        &runtime.output_native_bytes[..length],
                    ) {
                        Ok(submission) => {
                            if submission != OutputSubmission::Unsent {
                                counts.native_messages += 1;
                            }
                            submission
                        }
                        Err(()) => {
                            counts.native_unsupported += 1;
                            OutputSubmission::BestEffort
                        }
                    }
                } else if let FrameEvent::ConfigString {
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
                } else if let FrameEvent::ServerCommand { source, text, .. } = record.event {
                    let Some(text) = texts.get(text) else {
                        counts.stale_texts += 1;
                        return OutputSubmission::BestEffort;
                    };
                    let Some(connection) = runtime
                        .network
                        .get_mut(client_id, qa_core::loopback::Endpoint::Server)
                    else {
                        counts.native_unsupported += 1;
                        return OutputSubmission::BestEffort;
                    };
                    let Some(protocol) = connection
                        .commands
                        .as_ref()
                        .map(|commands| commands.protocol)
                    else {
                        counts.native_unsupported += 1;
                        return OutputSubmission::BestEffort;
                    };
                    let final_close = matches!(
                        client.phase,
                        qa_session::clients::ClientPhase::Closing(closing)
                            if closing.final_output == Some((id, record.sequence))
                    );
                    if source != qa_core::primitives::RuleSetId::Quake3 {
                        counts.native_unsupported += 1;
                        OutputSubmission::BestEffort
                    } else {
                        let text = if protocol == qa_network::commands::packet::Protocol::Quake3_68
                        {
                            text
                        } else if final_close {
                            let Ok(length) = qa_network::outputs::disconnect(
                                protocol,
                                &mut runtime.output_native_bytes,
                            ) else {
                                counts.native_unsupported += 1;
                                return OutputSubmission::BestEffort;
                            };
                            &runtime.output_native_bytes[..length]
                        } else {
                            counts.native_unsupported += 1;
                            return OutputSubmission::BestEffort;
                        };
                        match native::reliable(&mut connection.channel, text) {
                            Ok(submission) => submission,
                            Err(()) => {
                                counts.native_unsupported += 1;
                                OutputSubmission::BestEffort
                            }
                        }
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
                            FrameEvent::GameState {
                                payload,
                                command_sequence,
                                body_bits,
                            } => {
                                if delivery.source != qa_core::primitives::RuleSetId::Quake3
                                    || !delivery.reliable
                                {
                                    counts.native_unsupported += 1;
                                    return OutputSubmission::BestEffort;
                                }
                                let Some(bytes) =
                                    texts.copy_payload(payload, &mut runtime.output_bytes)
                                else {
                                    counts.stale_texts += 1;
                                    return OutputSubmission::BestEffort;
                                };
                                match connection.channel.queue_server_state(
                                    &bytes[..payload.len()],
                                    body_bits,
                                    command_sequence,
                                ) {
                                    Ok(receipt) => {
                                        counts.native_messages += 1;
                                        OutputSubmission::Reliable(receipt)
                                    }
                                    Err(qa_network::channel::TransmitError::Full) => {
                                        OutputSubmission::Unsent
                                    }
                                    Err(qa_network::channel::TransmitError::PendingPacket) => {
                                        stopped.set(true);
                                        OutputSubmission::Unsent
                                    }
                                    Err(_) => {
                                        counts.native_unsupported += 1;
                                        OutputSubmission::BestEffort
                                    }
                                }
                            }
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
                                                &mut |socket, to, bytes| {
                                                    source.send_packet(socket, to, bytes)
                                                },
                                                qa_core::loopback::Endpoint::Server,
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
                                            &mut |socket, to, bytes| {
                                                source.send_packet(socket, to, bytes)
                                            },
                                            qa_core::loopback::Endpoint::Server,
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
                            FrameEvent::Sound(_)
                            | FrameEvent::Effect(_)
                            | FrameEvent::StaticEntity(_)
                            | FrameEvent::PlayerValue { .. }
                            | FrameEvent::SetAngles(_) => {
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
                                            &server.entities,
                                            &mut runtime.native_numbers[slot],
                                            &runtime.config_publications[slot],
                                            crate::client_resources::model_world(
                                                client.connection,
                                                runtime.client_resources[slot].as_ref(),
                                                runtime
                                                    .collision
                                                    .as_ref()
                                                    .map(|world| world.geometry),
                                            ),
                                            &mut bound_model,
                                            runtime.catalog.hud.values.native(delivery.source),
                                            &runtime.catalog.registry.effects,
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
                                                &mut |socket, to, bytes| {
                                                    source.send_packet(socket, to, bytes)
                                                },
                                                qa_core::loopback::Endpoint::Server,
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
                    let resources = runtime.client_resources.get(slot).and_then(Option::as_ref);
                    if resources
                        .is_some_and(|resources| resources.state.presentation_reset_pending())
                    {
                        // Preserve this CLIENT's new display records on its
                        // existing cursor until its reset fence is consumed.
                        return OutputSubmission::Unsent;
                    }
                    let ready = resources.is_none_or(|resources| {
                        resources.state.presentation_after_reset(record.sequence)
                    });
                    let success = match record.event {
                        _ if !ready => true,
                        FrameEvent::Hud(event) => {
                            counts.hud += 1;
                            if !runtime.catalog.hud.apply(&mut client.hud, event.change) {
                                client.hud.clipped_values += 1;
                            }
                            true
                        }
                        FrameEvent::ViewFeedback(event) => {
                            if event.client == client_id
                                && event.client_lifetime == client.entity
                                && event.namespace
                                    == qa_core::primitives::ResourceNamespace::Client(client_id)
                                && runtime.snapshots[slot]
                                    .as_ref()
                                    .is_some_and(|snapshot| snapshot.accepts_feedback(event))
                            {
                                client.hud.view_feedback = Some(event.feedback);
                                counts.view_feedback += 1;
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
                                    &mut |socket, to, bytes| source.send_packet(socket, to, bytes),
                                    qa_core::loopback::Endpoint::Server,
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
        if is_local && client.is_active() {
            qa_ui::hud::expire_messages(&mut client.hud, &mut server.events.texts, now);
        }
        if (client.connection == Some(Connection::Remote) || is_local)
            && let Some(connection) = runtime
                .network
                .get_mut(client_id, qa_core::loopback::Endpoint::Server)
            && connection.pending_output.is_none()
        {
            let route = connection.route;
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
                        &mut |socket, to, bytes| source.send_packet(socket, to, bytes),
                        qa_core::loopback::Endpoint::Server,
                        route,
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
            && client.is_active()
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
            runtime.server_downloads[slot].reset();
            if let Some(connection) = runtime
                .network
                .get_mut(client_id, qa_core::loopback::Endpoint::Server)
            {
                connection.output = Some(resumed);
            }
            if let Some(snapshot) = runtime.snapshots[slot].as_mut() {
                snapshot.output = resumed;
                snapshot.reset_feedback();
            }
            // Rebuild discrete HUD state from CLIENT on its next publication.
            // Independently leased text stays valid through the resync.
            client.hud.reset_inventory();
            client.hud.view_feedback = None;
            client.hud_publication.reset();
            // Separate native stat services rebuild from the producer after a
            // cancelled delivery, just like the discrete inventory above.
            client.value_publication.clear();
            if let Some(snapshot) = runtime.snapshots[slot].as_ref()
                && let Some(seat) = snapshot.seat()
                && qa_network::outputs::separate_player_values(snapshot.protocol)
            {
                let rules = snapshot.protocol.rule_set();
                for index in 0..runtime.catalog.hud.values.native(rules).stats.len() {
                    runtime.catalog.hud.import_native_stat(
                        rules,
                        client_id,
                        index,
                        0,
                        &mut runtime.prediction[seat.index()].player,
                        time,
                    );
                }
            }
            runtime.config_publications[slot].reset();
            runtime.native_numbers[slot].reset();
            peer_resync = true;
        }
    }
    if peer_resync {
        server.invalidate_userinfo_publications();
    }
    counts
}

fn delivery_visible(
    collision: Option<&mut crate::WorldCollision>,
    listener: Vec3,
    delivery: NativeDelivery,
) -> bool {
    let nearby_hearing = delivery.target == NativeTarget::Hearing
        && delivery.source == qa_core::primitives::RuleSetId::QuakeWorld
        && qa_core::math::length(delivery.origin - listener) <= 1024.0;
    match delivery.target {
        NativeTarget::All | NativeTarget::Client(_) => true,
        NativeTarget::Hearing if nearby_hearing && !delivery.check_portals => true,
        target => collision.is_some_and(|collision| {
            let Some(world) = collision.visibility.as_deref() else {
                return false;
            };
            let policy = qa_world::visibility::PointVisibility {
                leaf_rules: qa_gameplay::rules::point_leaf_rules(delivery.source),
                channel: if nearby_hearing {
                    qa_world::visibility::VisibilityChannel::All
                } else if target == NativeTarget::Hearing {
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
    entities: &qa_world::entities::EntityTable,
    numbers: &mut crate::output_namespace::NativeNumbers,
    publication: &crate::client_resources::ConfigPublication,
    model_world: Option<qa_core::primitives::GeometryId>,
    bound_model: &mut impl FnMut(
        ModuleId,
        u32,
        Option<qa_core::primitives::GeometryId>,
        qa_core::primitives::ModelUse,
    ) -> Option<crate::client_resources::ModelName>,
    source_values: &qa_ui::values::NativeValues,
    effects: &qa_core::effects::EffectCatalog,
    output: &mut [u8],
) -> Result<usize, qa_network::outputs::Error> {
    use crate::client_resources::ResourceReadiness;
    use qa_network::{commands::packet::Protocol, outputs::Error};
    match event {
        FrameEvent::Effect(event) => {
            let count = effects
                .native_particles(delivery.source)
                .and_then(|binding| binding.wire_count(event))
                .ok_or(Error::Unsupported)?;
            return qa_network::outputs::particle(
                protocol,
                qa_network::outputs::Particle {
                    position: event.position,
                    direction: event.direction,
                    color: event.color,
                    count,
                },
                output,
            );
        }
        FrameEvent::PlayerValue { id, value } => {
            let index = source_values
                .stats
                .iter()
                .position(|binding| binding.id == id)
                .and_then(|index| u8::try_from(index).ok())
                .ok_or(Error::Unsupported)?;
            return qa_network::outputs::player_value(protocol, index, value.as_integer(), output);
        }
        FrameEvent::SetAngles(angles) => {
            return qa_network::outputs::set_angles(protocol, angles, output);
        }
        _ => {}
    }
    let (ordinal, readiness) = match event {
        FrameEvent::Sound(sound) => publication
            .project_sound(delivery.module, sound.source_ordinal, sound.sound)
            .ok_or(Error::Unsupported)?,
        FrameEvent::StaticEntity(record) => {
            if !matches!(protocol, Protocol::NetQuake15 | Protocol::QuakeWorld28) {
                return Err(Error::Unsupported);
            }
            if let Some(reference) = record.appearance.models[0] {
                match publication.project_model_ref(
                    reference,
                    qa_core::primitives::ModelUse::Entity(0),
                    client,
                    |module, ordinal, usage| bound_model(module, ordinal, model_world, usage),
                ) {
                    crate::client_resources::ModelProjection::Ready(ordinal) => {
                        (ordinal, ResourceReadiness::Ready)
                    }
                    crate::client_resources::ModelProjection::Pending => {
                        return Err(Error::Pending);
                    }
                    crate::client_resources::ModelProjection::Unsupported => {
                        return Err(Error::Unsupported);
                    }
                }
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
                && numbers.is_identity_namespace(module)
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
            if let Some(fields) = sound.native_fields
                && numbers.is_identity_namespace(module)
            {
                Some(u32::from(fields.entity))
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
    let native_entity_namespace = numbers.is_identity_namespace(module);
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
