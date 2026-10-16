//! One connection snapshot producer, with optional local CLIENT presentation.
use qa_core::{
    events::OutputConsumerId,
    primitives::{
        ClientId, EntityAppearance, EntityRecord, PlayerState, Q3StatLayout, ResourceNamespace,
        ThinkTime,
    },
    sys_events::SeatId,
};
use qa_network::{
    channel::Channel,
    commands::packet::{Error, Protocol},
    ingress::Incoming,
    message::{Encoding, ErrorKind, Writer},
    projection::{PlayerContext, PlayerProjection, PlayerViewSample},
    snapshots::ReceivedFrame,
    states,
};

mod entities;
mod feedback;
mod loops;
mod presentation;
pub use entities::EntitySource;
pub use presentation::{
    ClockError, ClockOptions, GunAnimation, Interpolation, PresentationClock, SnapshotClockSample,
};

fn player_context(
    player: &PlayerState,
    source: &EntitySource<'_>,
    protocol: Protocol,
    number: u32,
    body_yaw: f32,
    appearance: Option<EntityAppearance>,
    is_self: bool,
) -> PlayerContext {
    let tuning = qa_movement::effective_tuning(player.movement_rules, player.movement.tuning);
    let mut context = PlayerContext {
        namespace: source.native_module.map(ResourceNamespace::Module),
        client_number: Some(number),
        weapon_number: source
            .registry
            .native_weapon_number(protocol.rule_set(), player.inventory.weapon)
            .map(u32::from),
        gravity: tuning.gravity,
        speed: tuning.speed,
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

/// Physical native identity is supplied explicitly or assigned during startup.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NativePlayerBinding {
    Supplied(u32),
    Awaiting,
    Assigned(u32),
}

impl NativePlayerBinding {
    pub const fn number(self) -> Option<u32> {
        match self {
            Self::Supplied(number) | Self::Assigned(number) => Some(number),
            Self::Awaiting => None,
        }
    }
}

/// Connection policy and projection metadata; player storage remains in the
/// existing SERVER array and independently owned CLIENT prediction state.
pub struct SnapshotBinding {
    pub client: ClientId,
    pub output: OutputConsumerId,
    pub protocol: Protocol,
    pub applied: u64,
    pub rejected: u64,
    presentation_closed: bool,
    pub(crate) close_control_pending: bool,
    native_client: NativePlayerBinding,
    view_entity_override: Option<u16>,
    projection: PlayerProjection,
    player_stats: Q3StatLayout,
    particles: Option<qa_core::effects::NativeParticleBindings>,
    presentation: Option<ClientPresentation>,
    entities: entities::EntityScratch,
    last_sent: Option<u32>,
    pub pending: Option<u32>,
}

struct ClientPresentation {
    seat: SeatId,
    appearance: qa_gameplay::rules::appearance::AppearancePolicy,
    view_source: qa_core::primitives::RuleSetId,
    view_rules: qa_gameplay::rules::presentation::ViewRules,
    clock: PresentationClock,
    fixed_period: Option<u16>,
    negotiated_period: bool,
    sequence: Option<u32>,
    gun_animation: Option<GunAnimation>,
    view_height: Option<ViewHeightAnimation>,
    loops: Option<loops::LoopPublication>,
    feedback: feedback::FeedbackPublication,
}

struct ViewHeightAnimation {
    previous: f32,
    current: f32,
    changed_at: ThinkTime,
}
impl ViewHeightAnimation {
    fn observe(state: &mut Option<Self>, height: f32, time: ThinkTime) {
        match state {
            Some(state) if state.current != height => {
                state.previous = state.current;
                state.current = height;
                state.changed_at = time;
            }
            Some(_) => {}
            None => {
                *state = Some(Self {
                    previous: height,
                    current: height,
                    changed_at: time,
                });
            }
        }
    }
    fn sample(&self, time: ThinkTime) -> f32 {
        let elapsed = time
            .milliseconds()
            .saturating_sub(self.changed_at.milliseconds()) as f32;
        let remaining = 100.0 - elapsed.min(100.0);
        self.current + (self.previous - self.current) * remaining * 0.01
    }
}

impl SnapshotBinding {
    pub fn bind_particles(&mut self, catalog: &qa_core::effects::EffectCatalog) {
        self.particles = catalog.native_particles(self.protocol.rule_set());
    }
    pub fn particle_event(
        &self,
        particle: qa_network::outputs::Particle,
    ) -> Option<qa_core::primitives::EffectEvent> {
        Some(self.particles?.event(
            particle.position,
            particle.direction,
            particle.color,
            particle.count,
        ))
    }
    pub fn seat(&self) -> Option<SeatId> {
        (!self.presentation_closed)
            .then(|| self.presentation.as_ref().map(|client| client.seat))
            .flatten()
    }
    pub fn presentation_closed(&self) -> bool {
        self.presentation_closed
    }
    pub(crate) fn close_presentation(&mut self) -> bool {
        !std::mem::replace(&mut self.presentation_closed, true)
    }
    pub fn native_client(&self) -> Option<u32> {
        self.native_client.number()
    }
    pub(crate) fn assign_nq_player(&mut self, entity: u16, max_clients: u8) -> bool {
        if self.protocol != Protocol::NetQuake15
            || self.native_client != NativePlayerBinding::Awaiting
            || entity == 0
            || entity > u16::from(max_clients)
        {
            return false;
        }
        self.native_client = NativePlayerBinding::Assigned(u32::from(entity) - 1);
        true
    }
    pub(crate) fn receive_gamestate(
        &mut self,
        gamestate: &qa_network::channel::commands::Q3GameState<'_>,
    ) {
        self.native_client = NativePlayerBinding::Assigned(gamestate.client_num);
        self.reset_received();
    }
    pub(crate) fn receive_server_info(&mut self) {
        if matches!(self.native_client, NativePlayerBinding::Assigned(_)) {
            self.native_client = NativePlayerBinding::Awaiting;
        }
        self.reset_received();
    }
    fn reset_received(&mut self) {
        self.view_entity_override = None;
        if let Some(client) = self.presentation.as_mut() {
            client.clock.reset();
            client.sequence = None;
            client.gun_animation = None;
            client.view_height = None;
            if let Some(loops) = client.loops.as_mut() {
                loops.request_reset();
            }
        }
        self.reset_feedback();
    }
    /// Explicit native preset selected before the first snapshot publication.
    pub fn set_player_stat_layout(
        &mut self,
        layout: Q3StatLayout,
        values: &qa_ui::values::ValueLayout,
    ) -> bool {
        if layout == self.player_stats {
            return true;
        }
        if self.last_sent.is_some() || self.pending.is_some() || self.applied != 0 {
            return false;
        }
        let native = values.native(self.protocol.rule_set());
        if self.protocol == Protocol::Quake3_68 && !layout.fits(native.stats.len()) {
            return false;
        }
        self.projection = PlayerProjection::load_values(self.protocol, native.bindings(), layout);
        self.player_stats = layout;
        true
    }
    pub fn player_entity(&self, frame: Option<ReceivedFrame<'_>>) -> Option<u32> {
        let supplied = self.native_client()?;
        let client = frame
            .and_then(ReceivedFrame::player_words)
            .and_then(|words| self.projection.client_number(words))
            .unwrap_or(supplied);
        crate::output_namespace::native_client_entity(self.protocol, client)
    }
    pub fn view_entity(&self, frame: Option<ReceivedFrame<'_>>) -> Option<u32> {
        self.view_entity_override
            .map(u32::from)
            .or_else(|| self.player_entity(frame))
    }
    pub fn entity_projection(&self) -> &qa_network::projection::EntityProjection {
        self.entities.projection()
    }
    pub fn appearance_policy(&self) -> Option<qa_gameplay::rules::appearance::AppearancePolicy> {
        self.presentation.as_ref().map(|client| client.appearance)
    }
    pub fn player_projection(&self) -> &PlayerProjection {
        &self.projection
    }
    pub fn interpolation(&self) -> Option<Interpolation> {
        self.presentation
            .as_ref()
            .filter(|client| client.clock.is_ready())
            .map(|client| client.clock.sample())
    }
    pub fn view_source(&self) -> Option<qa_core::primitives::RuleSetId> {
        self.presentation.as_ref().map(|client| client.view_source)
    }
    /// Native kick fields remain borrowed from the receiving channel. They do
    /// not change accepted input angles or movement state.
    pub fn view_angles(
        &self,
        channel: &Channel,
        player: &PlayerState,
        kick_enabled: bool,
    ) -> qa_core::primitives::Vec3 {
        let Some(client) = self.presentation.as_ref() else {
            return player.view_angles;
        };
        let mut punch = player.punch_angles;
        let mut previous = None;
        if client.view_rules.interpolate_punch()
            && let Some(current) = channel.current_snapshot()
            && let Some(words) = current.player_words()
            && let Some(sample) = self.projection.punch_angles(words)
        {
            punch = sample;
            previous = self
                .previous_view_words(channel, current)
                .filter(|old| self.projection.can_interpolate_player(old, words))
                .and_then(|old| self.projection.punch_angles(old));
        }
        client.view_rules.angles(
            player.view_angles,
            punch,
            previous,
            client.clock.sample().fraction,
            kick_enabled,
        )
    }
    /// Camera fields borrow the same accepted frames and clock as native kick
    /// angles. Movement and SERVER state remain unchanged.
    pub fn view_presentation(
        &self,
        channel: &Channel,
        player: &PlayerState,
        entity_origin: impl FnOnce(u32) -> Option<qa_core::primitives::Vec3>,
    ) -> PlayerViewSample {
        let fallback = PlayerViewSample {
            origin: player.eye(),
            view: player.view,
        };
        if self.protocol == Protocol::NetQuake15 {
            let current = channel.current_snapshot();
            let view = self.view_entity(current);
            if view != self.player_entity(current)
                && let Some(origin) = view.and_then(entity_origin)
            {
                return PlayerViewSample {
                    origin: player.view.eye(origin, player.movement.view_height),
                    view: player.view,
                };
            }
        }
        let Some(client) = self.presentation.as_ref() else {
            return fallback;
        };
        let Some(current) = channel.current_snapshot() else {
            return fallback;
        };
        let Some(words) = current.player_words() else {
            return fallback;
        };
        let sample = client.clock.sample();
        let previous = self.previous_view_words(channel, current);
        self.projection
            .sample_view(
                previous,
                words,
                sample.fraction,
                player,
                client
                    .view_height
                    .as_ref()
                    .map(|height| height.sample(sample.time)),
            )
            .unwrap_or(fallback)
    }
    fn previous_view_words<'a>(
        &self,
        channel: &'a Channel,
        current: ReceivedFrame<'a>,
    ) -> Option<&'a [u32]> {
        let client = self.presentation.as_ref()?;
        crate::client_scene::player_event_interpolation(
            Some(current),
            self.view_entity(Some(current)),
            self.entity_projection(),
            client.appearance,
            self.client,
        )
        .then(|| channel.snapshot(current.sequence().wrapping_sub(1)))
        .flatten()
        .and_then(ReceivedFrame::player_words)
    }
    pub fn weapon_view_angles(
        &self,
        accepted: qa_core::primitives::Vec3,
        presented: qa_core::primitives::Vec3,
    ) -> qa_core::primitives::Vec3 {
        self.presentation.as_ref().map_or(accepted, |client| {
            client.view_rules.weapon_angles(accepted, presented)
        })
    }
    pub fn gun_animation(&self) -> Option<GunAnimation> {
        self.presentation
            .as_ref()
            .and_then(|client| client.gun_animation)
    }
    /// Called once at the CLIENT phase, after queued packets have been accepted.
    /// Retained player and weapon words stay in the channel's single ring.
    pub fn advance_presentation(
        &mut self,
        channel: &Channel,
        queued: qa_core::sys_events::EventTime,
        options: ClockOptions,
    ) -> Result<(), ClockError> {
        let Some(client) = self.presentation.as_mut() else {
            return Ok(());
        };
        let Some(current) = channel.current_snapshot() else {
            return Ok(());
        };
        let previous = channel.previous_snapshot();
        let period = if client.negotiated_period {
            channel.q2_repro_frame_ms()
        } else {
            client.fixed_period
        };
        client.clock.advance(
            queued,
            SnapshotClockSample {
                sequence: current.sequence(),
                time: current.time(),
                flags: match current {
                    ReceivedFrame::Quake3(frame) => frame.flags,
                    _ => 0,
                },
            },
            previous.map(ReceivedFrame::time),
            period,
            options,
        )?;
        if let Some(height) = current
            .player_words()
            .and_then(|words| self.projection.split_view_height(words))
        {
            ViewHeightAnimation::observe(
                &mut client.view_height,
                height,
                client.clock.sample().time,
            );
        }
        Ok(())
    }
    #[expect(
        clippy::too_many_arguments,
        reason = "Presentation borrows the connection's existing snapshot, resource and output owners"
    )]
    pub(crate) fn publish_loops(
        &mut self,
        player: Option<&PlayerState>,
        lifetime: qa_core::primitives::EntityId,
        owner: OutputConsumerId,
        events: &mut qa_core::events::EventRing,
        channel: &Channel,
        resources: &crate::client_resources::ResourceState,
        alias_roll: qa_gameplay::rules::appearance::AliasRoll,
        smooth_clients: bool,
        budget: &mut usize,
    ) {
        let projection = self.entities.projection();
        let Some(presentation) = self.presentation.as_mut() else {
            return;
        };
        let Some(loops) = presentation.loops.as_mut() else {
            return;
        };
        loops.publish(
            self.client,
            lifetime,
            owner,
            events,
            crate::client_scene::ClientEntities {
                client: self.client,
                player,
                current: channel.current_snapshot(),
                previous: channel.previous_snapshot(),
                projection,
                appearance: presentation.appearance.with_smooth_clients(smooth_clients),
                alias_roll,
                interpolation: presentation.clock.sample(),
            },
            resources,
            budget,
        );
    }
    pub(crate) fn loop_completed(
        &mut self,
        owner: OutputConsumerId,
        sequence: u64,
        event: qa_core::primitives::SoundEvent,
        accepted: bool,
    ) {
        if let Some(loops) = self
            .presentation
            .as_mut()
            .and_then(|row| row.loops.as_mut())
        {
            loops.completed(owner, sequence, event, accepted);
        }
    }
    /// Ownership cleanup needs no accepted snapshot or native configstrings.
    /// In particular, a foreign cgame over NQ must still retry its fences.
    pub(crate) fn maintain_client_audio(
        &mut self,
        lifetime: qa_core::primitives::EntityId,
        owner: OutputConsumerId,
        events: &mut qa_core::events::EventRing,
        budget: &mut usize,
    ) {
        if let Some(loops) = self
            .presentation
            .as_mut()
            .and_then(|row| row.loops.as_mut())
        {
            loops.maintain(self.client, lifetime, owner, events, budget);
        }
    }
    pub(crate) fn reset_module_audio(
        &mut self,
        module: qa_core::primitives::ModuleId,
        lifetime: qa_core::primitives::EntityId,
    ) {
        if let Some(loops) = self
            .presentation
            .as_mut()
            .and_then(|client| client.loops.as_mut())
        {
            loops.request_producer_reset(module, lifetime);
        }
    }

    pub(crate) fn owns_client_audio(&self) -> bool {
        self.presentation
            .as_ref()
            .and_then(|row| row.loops.as_ref())
            .is_some_and(loops::LoopPublication::owns_native)
    }
    pub(crate) fn attach_client_audio(
        &mut self,
        module: qa_core::primitives::ModuleId,
        lifetime: qa_core::primitives::EntityId,
        rules: qa_core::primitives::RuleSetId,
        events: &mut qa_core::events::EventRing,
        owner: OutputConsumerId,
    ) -> Result<(), qa_compat::services::CallError> {
        let owned = self.owns_client_audio();
        self.presentation
            .as_mut()
            .and_then(|row| row.loops.as_mut())
            .ok_or(qa_compat::services::CallError::Entity)?
            .attach(self.client, module, lifetime, rules, events, owner)?;
        if !owned {
            self.baseline_player_events();
        }
        Ok(())
    }
    pub(crate) fn detach_client_audio(
        &mut self,
        module: qa_core::primitives::ModuleId,
        lifetime: qa_core::primitives::EntityId,
        events: &mut qa_core::events::EventRing,
        owner: OutputConsumerId,
    ) {
        if let Some(loops) = self
            .presentation
            .as_mut()
            .and_then(|row| row.loops.as_mut())
        {
            loops.detach(self.client, module, lifetime, events, owner);
            self.baseline_player_events();
        }
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
        seat: Option<SeatId>,
        output: OutputConsumerId,
        protocol: Protocol,
        view_source: qa_core::primitives::RuleSetId,
        native_client: NativePlayerBinding,
        values: &qa_ui::values::ValueLayout,
    ) -> Self {
        let native = values.native(states::player_value_layout(protocol).0);
        Self {
            client,
            output,
            protocol,
            applied: 0,
            rejected: 0,
            presentation_closed: false,
            close_control_pending: false,
            native_client,
            view_entity_override: None,
            projection: PlayerProjection::load_values(
                protocol,
                native.bindings(),
                Q3StatLayout::BASE,
            ),
            player_stats: Q3StatLayout::BASE,
            particles: None,
            presentation: seat.map(|seat| ClientPresentation {
                seat,
                appearance: qa_gameplay::rules::appearance::policy(protocol.rule_set()),
                view_source,
                view_rules: qa_gameplay::rules::presentation::view_rules(view_source),
                // Native wire timestamps select their arithmetic at admission;
                // movement, map format and module ownership do not select it.
                clock: PresentationClock::load(qa_gameplay::rules::presentation::clock_rules(
                    protocol.rule_set(),
                )),
                fixed_period: (protocol == Protocol::Quake2_34).then_some(100),
                negotiated_period: protocol == Protocol::Quake2Repro1038,
                loops: Some(loops::LoopPublication::load(
                    qa_gameplay::rules::loop_rules(protocol.rule_set()),
                    qa_network::snapshots::snapshot_limits(protocol)
                        .native_numbers
                        .max(
                            qa_network::snapshots::snapshot_limits(Protocol::Quake3_68)
                                .native_numbers,
                        ),
                )),
                sequence: None,
                gun_animation: None,
                view_height: None,
                feedback: feedback::FeedbackPublication::load(protocol.rule_set()),
            }),
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
        numbers: &mut crate::output_namespace::NativeNumbers,
        sequence: u32,
        time: qa_core::sys_events::EventTime,
        delta: Option<u32>,
        bytes: &mut [u8],
    ) -> Result<usize, Error> {
        let native_client = self.native_client().ok_or(Error::Context)?;
        let time = if self.protocol == Protocol::NetQuake15 {
            ThinkTime::Seconds(0.0).at(time)
        } else if self.protocol == Protocol::Quake3_68
            && let Some(clock) = player.native_clock
        {
            if !clock.is_valid() {
                return Err(Error::Context);
            }
            ThinkTime::Milliseconds(clock.native.milliseconds())
        } else {
            ThinkTime::Milliseconds(0).at(time)
        };
        let mut words = [0; states::PLAYER_WORDS];
        self.entities.prepare(&mut source, player, numbers)?;
        // Native resources and entity numbers come from the connection's
        // projection, never common handle casts or the executing module.
        let mut context = player_context(
            player,
            &source,
            self.protocol,
            native_client,
            player.view_angles.0[1],
            None,
            true,
        );
        context.ground_number = player
            .movement
            .ground
            .filter(|_| player.movement.grounded)
            .and_then(|entity| numbers.number(entity));
        if !self.projection.reduce(
            player,
            &context,
            &mut words,
            &mut *source.resolve_model,
            &mut *source.resolve_sound,
        ) {
            return Err(Error::Context);
        }
        self.entities
            .prepare_qw_players(&mut source, &self.projection, numbers)?;
        let capacity = channel
            .native_payload_capacity()
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
                numbers.native_clients(),
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
                                numbers.native_clients(),
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
                                numbers.native_clients(),
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

    pub fn apply(
        &mut self,
        incoming: Incoming<'_>,
        player: &mut PlayerState,
        resolve_ground: impl FnMut(u32) -> Option<qa_core::primitives::EntityId>,
        hud: &qa_ui::hud::HudBindings,
        registry: &qa_gameplay::registry::Registry,
        time: qa_core::sys_events::EventTime,
    ) -> bool {
        if self.presentation_closed {
            return false;
        }
        if let Incoming::SetView { entity } = incoming {
            if self.protocol == Protocol::NetQuake15 {
                self.view_entity_override = Some(entity);
            }
            return false;
        }
        let Some(native_client) = self.native_client() else {
            return false;
        };
        let Some(client) = self.presentation.as_mut() else {
            return false;
        };
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
                    && u32::from(info.number) == native_client =>
            {
                (Some(&info.words), None)
            }
            _ => (None, None),
        };
        if let Some(words) = words {
            let clock = snapshot.map(|frame| qa_core::primitives::ClockObservation {
                native: frame.time(),
                queued: time,
            });
            if !hud.native_packet_fits(self.protocol.rule_set(), self.player_stats, player, clock) {
                self.rejected += 1;
                return false;
            }
            if let Some(frame) = snapshot
                && client.sequence != Some(frame.sequence())
            {
                // Capture each accepted transition before importing it into
                // CLIENT. Several snapshots may arrive in one physical drain.
                let previous = client.sequence.replace(frame.sequence());
                if let Some(weapon) = self
                    .projection
                    .view_weapon(words, ResourceNamespace::Client(self.client))
                    && (client.gun_animation.is_none()
                        || weapon.frame != player.view_weapon.frame
                        || weapon.model != player.view_weapon.model)
                {
                    client.gun_animation = Some(GunAnimation {
                        changed_at: frame.time(),
                        previous,
                    });
                }
            }
            // Native entity binding is deferred; an ordinal cannot resolve by
            // casting it to a common EntityId or selecting the executing module.
            let mut context = PlayerContext::default();
            if self.projection.apply(
                words,
                player,
                &mut context,
                ResourceNamespace::Client(self.client),
                resolve_ground,
            ) && hud.import_native_packet(
                self.protocol.rule_set(),
                self.player_stats,
                self.client,
                player,
                time,
                clock,
            ) {
                if let Some(frame @ ReceivedFrame::NetQuake(_)) = snapshot
                    && let Some(number) = self.player_entity(Some(frame))
                    && let Some((words, _)) =
                        crate::client_scene::ClientEntities::row(frame, number)
                {
                    let mut record = EntityRecord::default();
                    if self.entities.projection().apply(
                        words,
                        &mut record,
                        ResourceNamespace::Client(self.client),
                    ) {
                        player.body.position = record.position;
                    }
                }
                registry.project_player_weapon(player);
                player.native_clock = clock;
                self.applied += 1;
                return true;
            } else {
                self.rejected += 1;
            }
        }
        false
    }
}

impl qa_compat::services::ClientAudio for SnapshotBinding {
    fn publish(
        &mut self,
        events: &mut qa_core::events::EventRing,
        event: qa_core::primitives::SoundEvent,
        native_entity: Option<i32>,
        doppler_enabled: bool,
    ) -> Result<(), qa_compat::services::CallError> {
        self.presentation
            .as_mut()
            .and_then(|row| row.loops.as_mut())
            .ok_or(qa_compat::services::CallError::Entity)?
            .native_publish(self.client, events, event, native_entity, doppler_enabled)
    }
    fn respatialize(
        &mut self,
        events: &mut qa_core::events::EventRing,
        producer: qa_core::primitives::ModuleId,
        queued_at: qa_core::sys_events::EventTime,
        native_entity: u32,
        origin: qa_core::primitives::Vec3,
        right: qa_core::primitives::Vec3,
    ) -> Result<(), qa_compat::services::CallError> {
        self.presentation
            .as_mut()
            .and_then(|row| row.loops.as_mut())
            .ok_or(qa_compat::services::CallError::Entity)?
            .native_listener(
                self.client,
                events,
                producer,
                queued_at,
                native_entity,
                origin,
                right,
            )
    }
}
