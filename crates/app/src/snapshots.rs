//! Native local snapshot submission and CLIENT field application.
use qa_core::{
    events::OutputConsumerId,
    primitives::{ClientId, EntityAppearance, PlayerState, ResourceNamespace, ThinkTime},
    sys_events::SeatId,
};
use qa_network::{
    channel::Channel,
    commands::packet::{Error, Protocol},
    ingress::Incoming,
    message::{Encoding, ErrorKind, Writer},
    projection::{PlayerContext, PlayerProjection},
    snapshots::ReceivedFrame,
    states,
};

mod entities;
mod presentation;
pub use entities::EntitySource;
pub use presentation::{GunAnimation, Interpolation, PresentationClock};

fn player_context(
    player: &PlayerState,
    number: u32,
    body_yaw: f32,
    appearance: Option<EntityAppearance>,
    is_self: bool,
) -> PlayerContext {
    let mut context = PlayerContext {
        client_number: Some(number),
        body_yaw,
        appearance,
        ..Default::default()
    };
    // A model is explicit even when unresolved; using a connection's default
    // player model would silently bind a foreign resource ordinal.
    context.player_info_flags = 1 << 5;
    if !is_self {
        context.player_info_flags |= (1 << 0) | (1 << 1);
    }
    for (axis, value) in player.body.velocity.0.iter().enumerate() {
        if *value != 0. {
            context.player_info_flags |= 1 << (axis + 2);
        }
    }
    if let Some(appearance) = appearance {
        if appearance.skin != 0 {
            context.player_info_flags |= 1 << 6;
        }
        if appearance.native.effects != 0 {
            context.player_info_flags |= 1 << 7;
        }
    }
    if is_self && player.view_weapon.frame != 0 {
        context.player_info_flags |= 1 << 8;
    }
    context
}

/// Connection policy and projection metadata; player storage remains in the
/// existing SERVER array and independently owned CLIENT prediction state.
pub struct LocalSnapshot {
    pub client: ClientId,
    pub seat: SeatId,
    pub output: OutputConsumerId,
    pub protocol: Protocol,
    pub applied: u64,
    pub rejected: u64,
    native_client: u32,
    native_clients: u32,
    projection: PlayerProjection,
    presentation: PresentationClock,
    presentation_sequence: Option<u32>,
    gun_animation: Option<GunAnimation>,
    context: PlayerContext,
    entities: entities::EntityScratch,
    last_sent: Option<u32>,
    pub pending: Option<u32>,
}

impl LocalSnapshot {
    pub fn native_client(&self) -> u32 {
        self.native_client
    }
    pub fn native_clients(&self) -> u32 {
        self.native_clients
    }
    pub fn set_native_clients(&mut self, count: u32) {
        self.native_clients = count;
    }
    pub fn view_entity(&self) -> Option<u32> {
        self.entities.view_entity()
    }
    pub fn native_number(&self, entity: qa_core::primitives::EntityId) -> Option<u32> {
        self.entities.native_number(entity)
    }
    pub fn entity_projection(&self) -> &qa_network::projection::EntityProjection {
        self.entities.projection()
    }
    pub fn player_projection(&self) -> &PlayerProjection {
        &self.projection
    }
    pub fn interpolation(&self) -> Interpolation {
        self.presentation.sample()
    }
    pub fn gun_animation(&self) -> Option<GunAnimation> {
        self.gun_animation
    }
    /// Called once at the CLIENT phase, after queued packets have been accepted.
    /// Retained player and weapon words stay in the channel's single ring.
    pub fn advance_presentation(
        &mut self,
        channel: &Channel,
        queued: qa_core::sys_events::EventTime,
    ) {
        let Some(current) = channel.current_snapshot() else {
            return;
        };
        let previous = channel.previous_snapshot();
        let period = match self.protocol {
            Protocol::Quake2_34 => Some(100),
            Protocol::Quake2Repro1038 => channel.q2_repro_frame_ms(),
            _ => None,
        };
        self.presentation.advance(
            queued,
            current.time(),
            previous.map(ReceivedFrame::time),
            period,
            self.protocol,
        );
    }
    pub fn entity_rejections(&self) -> u64 {
        self.entities.rejected
    }
    pub fn conservative_leaf_queries(&self) -> u64 {
        self.entities.conservative_leaf_queries()
    }
    pub fn load_visibility(&mut self, world: &qa_world::visibility::VisibilityWorld) {
        self.entities.load_visibility(world);
    }
    pub fn load(
        client: ClientId,
        seat: SeatId,
        output: OutputConsumerId,
        protocol: Protocol,
        native_client: u32,
        values: &qa_ui::values::ValueLayout,
    ) -> Self {
        let native = values.native(states::player_value_layout(protocol).0);
        Self {
            client,
            seat,
            output,
            protocol,
            applied: 0,
            rejected: 0,
            native_client,
            native_clients: 1,
            projection: PlayerProjection::load_values(protocol, &native.stats, &native.persistent),
            presentation: PresentationClock::default(),
            presentation_sequence: None,
            gun_animation: None,
            context: PlayerContext {
                client_number: Some(native_client),
                ..Default::default()
            },
            entities: entities::EntityScratch::load(protocol),
            last_sent: None,
            pending: None,
        }
    }

    pub fn sequence(&self, channel: &Channel, time: qa_core::sys_events::EventTime) -> u32 {
        match self.protocol {
            Protocol::NetQuake15 => channel.send_state().datagram_sequence,
            // Protocol 34 derives time from serverframe * 100; publish only at
            // representable boundaries, independently of the world tick rate.
            Protocol::Quake2_34 => (time.milliseconds() / 100) as u32,
            Protocol::Quake2Repro1038 => channel.q2_repro_frame_ms().map_or(0, |duration| {
                (time.milliseconds() / u64::from(duration)) as u32
            }),
            _ => channel.send_state().sequence,
        }
    }

    pub fn encode(
        &mut self,
        channel: &mut Channel,
        player: &PlayerState,
        mut source: EntitySource<'_>,
        sequence: u32,
        time: qa_core::sys_events::EventTime,
        delta: Option<u32>,
        bytes: &mut [u8],
    ) -> Result<usize, Error> {
        let time = if self.protocol == Protocol::NetQuake15 {
            ThinkTime::Seconds(0.0).at(time)
        } else {
            ThinkTime::Milliseconds(0).at(time)
        };
        let mut words = [0; states::PLAYER_WORDS];
        self.entities
            .prepare(&mut source, player, self.native_client)?;
        // Native resources and entity numbers come from the connection's
        // projection, never common handle casts or the executing module.
        let mut context = player_context(
            player,
            self.native_client,
            player.view_angles.0[1],
            None,
            true,
        );
        context.ground_number = player
            .movement
            .ground
            .and_then(|entity| self.entities.native_number(entity));
        if !self
            .projection
            .reduce(player, &context, &mut words, &mut *source.resolve_model)
        {
            return Err(Error::Context);
        }
        self.entities
            .prepare_qw_players(&mut source, &self.projection)?;
        let capacity = channel
            .snapshot_payload_capacity()
            .map_err(|_| Error::Context)?;
        let length = bytes.len().min(capacity);
        let bytes = &mut bytes[..length];
        let full = self.entities.selection();
        let mut selection = full;
        let mut request = delta;
        let mut failed_entities: Option<usize> = None;
        loop {
            let frame = self
                .entities
                .frame(sequence, time, &words, selection.entities)?;
            match self.encode_candidate(
                channel,
                frame,
                selection,
                request,
                source.native_clients,
                bytes,
            ) {
                Ok(mut length) => {
                    // Full-baseline records add nonnegative byte cost. Delta
                    // prefixes are not monotonic: omitted old rows cost removals.
                    if request.is_none()
                        && let Some(failed) = failed_entities
                    {
                        let mut high = failed.saturating_sub(1);
                        let mut matches_buffer = true;
                        while selection.entities < high {
                            let count =
                                selection.entities + (high - selection.entities).div_ceil(2);
                            let trial = entities::Selection {
                                entities: count,
                                ..selection
                            };
                            let frame = self.entities.frame(sequence, time, &words, count)?;
                            match self.encode_candidate(
                                channel,
                                frame,
                                trial,
                                None,
                                source.native_clients,
                                bytes,
                            ) {
                                Ok(n) => {
                                    selection = trial;
                                    length = n;
                                    matches_buffer = true;
                                }
                                Err(Error::Message(error)) if error.kind == ErrorKind::Capacity => {
                                    high = count - 1;
                                    matches_buffer = false;
                                }
                                Err(error) => return Err(error),
                            }
                        }
                        // A failed last probe may have overwritten the buffer.
                        if !matches_buffer {
                            let frame =
                                self.entities
                                    .frame(sequence, time, &words, selection.entities)?;
                            length = self.encode_candidate(
                                channel,
                                frame,
                                selection,
                                None,
                                source.native_clients,
                                bytes,
                            )?;
                        }
                    }
                    channel.publish_written_snapshot(
                        self.entities
                            .frame(sequence, time, &words, selection.entities)?,
                        request,
                    )?;
                    self.entities.count_selection(selection);
                    return Ok(length);
                }
                Err(Error::Message(error)) if error.kind == ErrorKind::Capacity => {
                    if request.is_some() {
                        // The protocol's ordinary full snapshot can retire old
                        // entities without fitting a long sequence of removes.
                        request = None;
                        selection = full;
                        failed_entities = None;
                    } else {
                        failed_entities = Some(selection.entities);
                        let players = selection.players;
                        if !self.entities.reduce_selection(&mut selection) {
                            return Err(Error::Message(error));
                        }
                        if selection.players != players {
                            selection.entities = full.entities;
                            failed_entities = None;
                        }
                    }
                }
                Err(error) => return Err(error),
            }
        }
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "One boundary serializer probes native capacity and writes the selected frame"
    )]
    fn encode_candidate(
        &self,
        channel: &mut Channel,
        frame: ReceivedFrame<'_>,
        selection: entities::Selection,
        request: Option<u32>,
        native_clients: u32,
        bytes: &mut [u8],
    ) -> Result<usize, Error> {
        let body = |writer: &mut Writer<'_>, channel: &mut Channel| {
            if self.protocol == Protocol::QuakeWorld28 {
                self.entities.write_qw_players(writer, selection.players)?;
            }
            channel.write_snapshot_candidate(writer, frame, request, native_clients)
        };
        if self.protocol == Protocol::Quake3_68 {
            channel.encode_server_output(bytes, body)
        } else {
            let mut writer = Writer::new(bytes, Encoding::Bytes);
            body(&mut writer, channel)?;
            Ok(writer.size())
        }
    }

    pub fn needs_send(&self, channel: &Channel, sequence: u32) -> bool {
        // Original QW replies follow an admitted client sequence, never an
        // unsolicited frame that gets ahead of the client's first command.
        (self.protocol != Protocol::QuakeWorld28 || channel.state().sequence != 0)
            && self.last_sent != Some(sequence)
    }

    pub fn submitted(&mut self, sequence: u32) {
        self.last_sent = Some(sequence);
    }

    pub fn apply(&mut self, incoming: Incoming<'_>, player: &mut PlayerState) {
        let (words, snapshot): (Option<&[u32]>, Option<ReceivedFrame<'_>>) = match incoming {
            Incoming::Snapshot(frame) => match (self.protocol, frame) {
                (Protocol::NetQuake15, ReceivedFrame::NetQuake(_))
                | (Protocol::Quake2_34, ReceivedFrame::Quake2(_))
                | (Protocol::Quake2Repro1038, ReceivedFrame::Quake2Repro(_))
                | (Protocol::Quake3_68, ReceivedFrame::Quake3(_)) => {
                    (frame.player_words(), Some(frame))
                }
                _ => (None, None),
            },
            Incoming::PlayerInfo(ref info)
                if self.protocol == Protocol::QuakeWorld28
                    && u32::from(info.number) == self.native_client =>
            {
                (Some(&info.words), None)
            }
            _ => (None, None),
        };
        if let Some(words) = words {
            if let Some(frame) = snapshot
                && self.presentation_sequence != Some(frame.sequence())
            {
                // Capture each accepted transition before importing it into
                // CLIENT. Several snapshots may arrive in one physical drain.
                let previous = self.presentation_sequence.replace(frame.sequence());
                if let Some(weapon) = self
                    .projection
                    .view_weapon(words, ResourceNamespace::Client(self.client))
                    && (self.gun_animation.is_none()
                        || weapon.frame != player.view_weapon.frame
                        || weapon.model != player.view_weapon.model)
                {
                    self.gun_animation = Some(GunAnimation {
                        changed_at: frame.time(),
                        previous,
                    });
                }
            }
            // Native entity binding is deferred; an ordinal cannot resolve by
            // casting it to a common EntityId or selecting the executing module.
            if self.projection.apply(
                words,
                player,
                &mut self.context,
                ResourceNamespace::Client(self.client),
                |number| self.entities.entity(number),
            ) {
                self.applied += 1;
            } else {
                self.rejected += 1;
            }
        }
    }
}
