//! Numeric last-publication state for the connection's accepted CLIENT loops.
use crate::{client_resources::ResourceState, client_scene::ClientEntities};
use qa_compat::services::CallError;
use qa_core::sys_events::EventTime;
use qa_core::{
    events::{EventRing, FrameEvent, OutputConsumerId, OutputTarget},
    primitives::{
        ClientId, EntityId, Listener, LoopLifetime, LoopPlayback, ModuleId, ResourceNamespace,
        RuleSetId, SoundAction, SoundEmitter, SoundEvent, SoundId, SpatialPolicy, Vec3,
    },
    stamps::StampSet,
};
use qa_gameplay::rules::LoopRules;

#[derive(Clone, Copy, PartialEq)]
struct Settings {
    sound: SoundId,
    playback: LoopPlayback,
    spatial: SpatialPolicy,
}
#[derive(Clone, Copy, Default)]
struct Row {
    published: Option<Settings>,
    stop_key: Option<u32>,
    stopped_sequence: Option<u32>,
    event_once_seen: bool,
    origin: Option<Vec3>,
    last_publication: Option<u64>,
}

/// Successfully published numeric settings and native CLIENT emitter origins.
/// Snapshot fallback still reads desired settings from the accepted channel;
/// native imports use these same rows, with no second loop publication queue.
pub(crate) struct LoopPublication {
    rules: Option<LoopRules>,
    rows: Box<[Row]>,
    present: StampSet,
    active: Vec<usize>,
    cursor: usize,
    lifetime: Option<EntityId>,
    owner: Option<OutputConsumerId>,
    reset_needed: bool,
    reset: Option<(OutputConsumerId, u64)>,
    producer: Option<ModuleId>,
    native_rules: Option<LoopRules>,
    releasing: bool,
    release: Option<(OutputConsumerId, u64)>,
    listener: u32,
}
impl LoopPublication {
    pub(crate) fn load(rules: Option<LoopRules>, capacity: usize) -> Self {
        Self {
            rules,
            rows: vec![Row::default(); capacity].into_boxed_slice(),
            present: StampSet::new(capacity),
            active: Vec::with_capacity(capacity),
            cursor: 0,
            lifetime: None,
            owner: None,
            reset_needed: false,
            reset: None,
            producer: None,
            native_rules: None,
            releasing: false,
            release: None,
            listener: 0,
        }
    }
    fn retained_push(
        events: &mut EventRing,
        owner: OutputConsumerId,
        event: SoundEvent,
    ) -> Option<u64> {
        let sequence = events
            .push_to(OutputTarget::Presentation, FrameEvent::Sound(event))
            .ok()?;
        if events.retained_for(owner, sequence) {
            Some(sequence)
        } else {
            events.reject_publication();
            None
        }
    }
    #[expect(
        clippy::too_many_arguments,
        reason = "The publication borrows disjoint connection, ring, accepted snapshot and resource owners"
    )]
    pub(crate) fn publish(
        &mut self,
        client: ClientId,
        lifetime: EntityId,
        owner: OutputConsumerId,
        events: &mut EventRing,
        entities: ClientEntities<'_>,
        resources: &ResourceState,
        budget: &mut usize,
    ) {
        if !self.maintain(client, lifetime, owner, events, budget) || self.owns_native() {
            return;
        }
        if self.rules.is_none() {
            return;
        }
        let Some(current) = entities.current else {
            return;
        };
        self.present.begin();
        let start = self.cursor;
        self.walk_range(
            start..self.rows.len(),
            client,
            lifetime,
            owner,
            events,
            entities,
            resources,
            current.sequence(),
            budget,
        );
        if *budget != 0 && !self.reset_needed {
            self.walk_range(
                0..start,
                client,
                lifetime,
                owner,
                events,
                entities,
                resources,
                current.sequence(),
                budget,
            );
        }
    }
    pub(crate) fn maintain(
        &mut self,
        client: ClientId,
        lifetime: EntityId,
        owner: OutputConsumerId,
        events: &mut EventRing,
        budget: &mut usize,
    ) -> bool {
        self.bind_scope(lifetime, owner);
        self.cleanup(client, lifetime, owner, events, budget)
    }
    fn tracked(row: Row) -> bool {
        row.published.is_some()
            || row.stop_key.is_some()
            || row.event_once_seen
            || row.stopped_sequence.is_some()
            || row.origin.is_some()
            || row.last_publication.is_some()
    }
    fn write_row(&mut self, index: usize, row: Row) {
        let old = Self::tracked(self.rows[index]);
        let new = Self::tracked(row);
        if old != new {
            let position = self.active.binary_search(&index);
            match (new, position) {
                (true, Err(position)) => self.active.insert(position, index),
                (false, Ok(position)) => {
                    self.active.remove(position);
                }
                _ => {}
            }
        }
        self.rows[index] = row;
    }
    fn rewrite_active(&mut self, mut rewrite: impl FnMut(Row) -> Row) {
        let mut position = 0;
        while let Some(index) = self.active.get(position).copied() {
            self.write_row(index, rewrite(self.rows[index]));
            if self.active.get(position) == Some(&index) {
                position += 1;
            }
        }
    }
    fn settle(&mut self, index: usize, desired: Option<Settings>) {
        self.write_row(
            index,
            Row {
                published: desired,
                ..self.rows[index]
            },
        );
    }
    #[expect(
        clippy::too_many_arguments,
        reason = "The sparse walk shares the same borrowed publication owners and bounded budget"
    )]
    fn walk_range(
        &mut self,
        range: std::ops::Range<usize>,
        client: ClientId,
        lifetime: EntityId,
        owner: OutputConsumerId,
        events: &mut EventRing,
        entities: ClientEntities<'_>,
        resources: &ResourceState,
        sequence: u32,
        budget: &mut usize,
    ) {
        let mut active = self.active.partition_point(|&index| index < range.start);
        entities.visit_loops(|number, pose| {
            let index = number as usize;
            if !range.contains(&index) || index >= self.rows.len() {
                return;
            }
            while active < self.active.len() && self.active[active] < index {
                let missing = self.active[active];
                self.update(
                    missing, None, client, lifetime, owner, events, resources, sequence, budget,
                );
                if self.active.get(active) == Some(&missing) {
                    active += 1;
                }
            }
            if !self.present.test_and_set(index) {
                self.update(
                    index, pose, client, lifetime, owner, events, resources, sequence, budget,
                );
            }
            active = self.active.partition_point(|&number| number <= index);
        });
        while active < self.active.len() && range.contains(&self.active[active]) {
            let missing = self.active[active];
            if !self.present.contains(missing) {
                self.update(
                    missing, None, client, lifetime, owner, events, resources, sequence, budget,
                );
            }
            if self.active.get(active) == Some(&missing) {
                active += 1;
            }
        }
    }
    #[expect(
        clippy::too_many_arguments,
        reason = "One accepted row updates its numeric last-publication state against the common ring"
    )]
    fn update(
        &mut self,
        index: usize,
        pose: Option<crate::client_scene::PresentedEntity>,
        client: ClientId,
        lifetime: EntityId,
        owner: OutputConsumerId,
        events: &mut EventRing,
        resources: &ResourceState,
        sequence: u32,
        budget: &mut usize,
    ) {
        if *budget == 0 || self.reset_needed {
            return;
        }
        let Some(rules) = self.rules else {
            return;
        };
        let once = pose
            .as_ref()
            .is_some_and(|pose| rules.event_once(&pose.record.appearance.native));
        let seen = self.rows[index].event_once_seen;
        let stop = pose.as_ref().and_then(|pose| {
            let native = &pose.record.appearance.native;
            Some((
                rules.stop(native)?,
                rules.stop_emitter(&pose.record.appearance, index as u32)?,
            ))
        });
        if let Some((key, target)) = stop {
            if self.rows[index].stop_key != Some(key)
                && (!once || !seen)
                && (target as usize) < self.rows.len()
            {
                let event = Self::event(client, lifetime, target as usize, SoundAction::StopLoop);
                if !self.push(index, owner, events, event, budget) {
                    return;
                }
                self.settle(target as usize, None);
                self.write_row(
                    index,
                    Row {
                        stop_key: Some(key),
                        ..self.rows[index]
                    },
                );
                self.write_row(
                    target as usize,
                    Row {
                        stopped_sequence: Some(sequence),
                        ..self.rows[target as usize]
                    },
                );
            }
        } else {
            self.write_row(
                index,
                Row {
                    stop_key: None,
                    ..self.rows[index]
                },
            );
        }
        self.write_row(
            index,
            Row {
                event_once_seen: once,
                ..self.rows[index]
            },
        );
        if self.rows[index].stopped_sequence == Some(sequence) {
            return;
        }
        self.write_row(
            index,
            Row {
                stopped_sequence: None,
                ..self.rows[index]
            },
        );
        if *budget == 0 {
            return;
        }
        let desired = pose.as_ref().and_then(|pose| {
            let reference = pose.record.appearance.loop_sound?;
            if reference.namespace != ResourceNamespace::Client(client) {
                return None;
            }
            let sound = resources.sound(reference.ordinal)?;
            let (playback, spatial) =
                rules.admit(&pose.record.appearance, resources.loop_spatial_policy())?;
            Some(Settings {
                sound,
                playback,
                spatial,
            })
        });
        let published = self.rows[index].published;
        if desired == published {
            return;
        }
        // Native real-speaker loops survive ordinary missing/zero rows.
        if desired.is_none()
            && published.is_some_and(|old| old.playback.lifetime == LoopLifetime::Persistent)
        {
            return;
        }
        let action = if desired.is_some() {
            SoundAction::StartLoop
        } else {
            SoundAction::StopLoop
        };
        let event = SoundEvent {
            sound: desired.map_or(SoundId(0), |settings| settings.sound),
            looping: desired.map(|settings| settings.playback),
            spatial: desired.map_or_else(SpatialPolicy::default, |settings| settings.spatial),
            position: pose
                .as_ref()
                .map_or_else(Default::default, |pose| pose.record.position),
            volume: 1.0,
            attenuation: 1.0,
            ..Self::event(client, lifetime, index, action)
        };
        if self.push(index, owner, events, event, budget) {
            self.settle(index, desired);
        }
    }
    fn event(
        client: ClientId,
        lifetime: EntityId,
        index: usize,
        action: SoundAction,
    ) -> SoundEvent {
        SoundEvent {
            client: Some(client),
            client_lifetime: Some(lifetime),
            entity: Some(SoundEmitter::Native {
                namespace: ResourceNamespace::Client(client),
                slot: index as i32,
            }),
            action,
            ..SoundEvent::default()
        }
    }
    fn push(
        &mut self,
        index: usize,
        owner: OutputConsumerId,
        events: &mut EventRing,
        event: SoundEvent,
        budget: &mut usize,
    ) -> bool {
        *budget -= 1;
        self.cursor = (index + 1) % self.rows.len();
        if Self::retained_push(events, owner, event).is_some() {
            true
        } else {
            self.reset_needed = true;
            false
        }
    }
    fn bind_scope(&mut self, lifetime: EntityId, owner: OutputConsumerId) {
        if self.lifetime != Some(lifetime) {
            self.rows.fill(Row::default());
            self.active.clear();
            self.lifetime = Some(lifetime);
            self.owner = Some(owner);
            self.reset_needed = false;
            self.reset = None;
            self.producer = None;
            self.native_rules = None;
            self.releasing = false;
            self.release = None;
            self.listener = 0;
            self.cursor = 0;
        } else if self.owner != Some(owner) {
            self.owner = Some(owner);
            self.reset_needed = true;
            self.reset = None;
            self.release = None;
        }
    }
    pub(crate) fn owns_native(&self) -> bool {
        self.producer.is_some()
    }
    pub(crate) fn request_reset(&mut self) {
        // Keep any older fence tracked. Cleanup serializes its completion
        // before publishing the reset required by the new gamestate.
        self.reset_needed = true;
    }
    pub(crate) fn attach(
        &mut self,
        client: ClientId,
        module: ModuleId,
        lifetime: EntityId,
        rules: RuleSetId,
        events: &mut EventRing,
        owner: OutputConsumerId,
    ) -> Result<(), CallError> {
        if self.producer.is_some() && self.lifetime != Some(lifetime) {
            return Err(CallError::Entity);
        }
        self.bind_scope(lifetime, owner);
        if let Some(current) = self.producer {
            return if current == module && !self.releasing {
                Ok(())
            } else {
                Err(CallError::Entity)
            };
        }
        self.producer = Some(module);
        self.native_rules = qa_gameplay::rules::loop_rules(rules);
        self.reset_needed = true;
        // The retained reset is ordered before Init's imports. Its later
        // receipt must never erase rows published after this fence.
        let mut budget = 1;
        self.cleanup(client, lifetime, owner, events, &mut budget);
        Ok(())
    }
    pub(crate) fn detach(
        &mut self,
        client: ClientId,
        module: ModuleId,
        lifetime: EntityId,
        events: &mut EventRing,
        owner: OutputConsumerId,
    ) {
        if self.lifetime != Some(lifetime) || self.producer != Some(module) || self.releasing {
            return;
        }
        self.releasing = true;
        // An attach fence may precede newly registered loops. Detach requires
        // another fence after them, not the earlier pending attach receipt.
        self.reset = None;
        self.reset_needed = true;
        self.release = None;
        let mut budget = 1;
        self.cleanup(client, lifetime, owner, events, &mut budget);
    }
    /// Reset/release use the existing ring's actual presentation receipts.
    /// Ownership remains until both controls have been accepted, including
    /// consumer overflow and backend rejection retries.
    fn cleanup(
        &mut self,
        client: ClientId,
        lifetime: EntityId,
        owner: OutputConsumerId,
        events: &mut EventRing,
        budget: &mut usize,
    ) -> bool {
        if events.needs_resync(owner) {
            self.reset_needed = true;
        }
        if let Some((pending_owner, sequence)) = self.reset {
            if events.retained_for(pending_owner, sequence) {
                return false;
            }
            self.reset = None;
            self.reset_needed = true;
        }
        if let Some((pending_owner, sequence)) = self.release {
            if events.retained_for(pending_owner, sequence) {
                return false;
            }
            self.release = None;
            self.reset_needed = true;
        }
        if events.needs_resync(owner) || events.counters(owner).is_none() {
            return false;
        }
        if self.reset_needed {
            if *budget != 0 {
                *budget -= 1;
                self.reset = Self::retained_push(
                    events,
                    owner,
                    SoundEvent::reset_client_loops(client, lifetime),
                )
                .map(|sequence| (owner, sequence));
                if self.reset.is_some() {
                    self.reset_needed = false;
                }
            }
            return false;
        }
        if self.releasing {
            if *budget != 0 {
                *budget -= 1;
                self.release = Self::retained_push(
                    events,
                    owner,
                    SoundEvent {
                        action: SoundAction::ReleaseListener,
                        client: Some(client),
                        client_lifetime: Some(lifetime),
                        listener_owner: self.producer,
                        ..SoundEvent::default()
                    },
                )
                .map(|sequence| (owner, sequence));
            }
            return false;
        }
        true
    }
    fn native_scope(
        &self,
        producer: Option<ModuleId>,
    ) -> Result<(EntityId, OutputConsumerId), CallError> {
        if self.producer.is_none() || self.producer != producer || self.releasing {
            return Err(CallError::Entity);
        }
        Ok((
            self.lifetime.ok_or(CallError::Entity)?,
            self.owner.ok_or(CallError::Entity)?,
        ))
    }
    pub(crate) fn native_publish(
        &mut self,
        client: ClientId,
        events: &mut EventRing,
        mut event: SoundEvent,
        native_entity: Option<i32>,
        doppler_enabled: bool,
    ) -> Result<(), CallError> {
        let (lifetime, owner) = self.native_scope(event.listener_owner)?;
        if event.client.is_some_and(|recipient| recipient != client)
            || event
                .client_lifetime
                .is_some_and(|previous| previous != lifetime)
        {
            return Err(CallError::Entity);
        }
        // Native imports may follow a retained attach fence before the mixer
        // drains it. If no fence fit yet, retry it before admitting new work.
        if self.reset_needed {
            let mut budget = 1;
            self.cleanup(client, lifetime, owner, events, &mut budget);
            if self.reset_needed || self.reset.is_none() {
                events.reject_publication();
                return Err(CallError::Capacity);
            }
        }
        let positioned = event.action == SoundAction::Play && event.positioned;
        let index = if let Some(number) = native_entity {
            if positioned {
                None
            } else {
                Some(
                    usize::try_from(number)
                        .ok()
                        .filter(|&number| number < self.rows.len())
                        .ok_or(CallError::Entity)?,
                )
            }
        } else if event.action == SoundAction::Play {
            Some(self.listener as usize)
        } else {
            None
        };
        if matches!(
            event.action,
            SoundAction::StartLoop | SoundAction::StopLoop | SoundAction::UpdateEmitterPosition
        ) && index.is_none()
        {
            return Err(CallError::Entity);
        }
        event.client = Some(client);
        event.client_lifetime = Some(lifetime);
        event.entity = native_entity
            .or_else(|| index.map(|index| index as i32))
            .map(|slot| SoundEmitter::Native {
                namespace: ResourceNamespace::Client(client),
                slot,
            });
        if let Some(index) = index {
            if event.action == SoundAction::Play && !event.positioned {
                event.position = self.rows[index].origin.unwrap_or_default();
            }
        }
        if matches!(
            event.action,
            SoundAction::StartLoop | SoundAction::UpdateEmitterPosition
        ) && (!event.position.is_finite() || !event.velocity.is_finite())
        {
            return Err(CallError::Geometry);
        }
        if event.action == SoundAction::StartLoop {
            let supplied = event.looping.ok_or(CallError::Unsupported)?;
            event.looping = Some(self.native_rules.map_or(supplied, |rules| {
                rules.native(supplied.lifetime == LoopLifetime::Persistent)
            }));
        }
        if event.action == SoundAction::StartLoop {
            let playback = event.looping.ok_or(CallError::Unsupported)?;
            event.doppler = if doppler_enabled {
                self.native_rules.and_then(|rules| {
                    rules.doppler(
                        playback,
                        self.rows
                            .get(self.listener as usize)
                            .and_then(|row| row.origin)
                            .unwrap_or_default(),
                        event.position,
                        event.velocity,
                    )
                })
            } else {
                None
            };
        }
        let sequence = Self::retained_push(events, owner, event).ok_or_else(|| {
            self.reset_needed = true;
            CallError::Capacity
        })?;
        if let Some(index) = index {
            let mut row = self.rows[index];
            row.last_publication = Some(sequence);
            match event.action {
                SoundAction::StartLoop => {
                    row.published = event.looping.map(|playback| Settings {
                        sound: event.sound,
                        playback,
                        spatial: event.spatial,
                    });
                    row.origin = Some(event.position);
                }
                SoundAction::StopLoop => row.published = None,
                SoundAction::UpdateEmitterPosition => row.origin = Some(event.position),
                _ => {}
            }
            self.write_row(index, row);
        }
        if matches!(
            event.action,
            SoundAction::ClearTemporaryLoops | SoundAction::ClearClientLoops
        ) {
            self.rewrite_active(|mut row| {
                if event.action == SoundAction::ClearClientLoops
                    || row
                        .published
                        .is_some_and(|p| p.playback.lifetime == LoopLifetime::Temporary)
                {
                    row.published = None;
                }
                row.last_publication = Some(sequence);
                row
            });
        }
        Ok(())
    }
    pub(crate) fn native_listener(
        &mut self,
        client: ClientId,
        events: &mut EventRing,
        producer: ModuleId,
        queued_at: EventTime,
        native_entity: u32,
        origin: Vec3,
        right: Vec3,
    ) -> Result<(), CallError> {
        if native_entity as usize >= self.rows.len() || !origin.is_finite() || !right.is_finite() {
            return Err(CallError::Geometry);
        }
        self.native_publish(
            client,
            events,
            SoundEvent {
                action: SoundAction::Respatialize,
                queued_at,
                listener_owner: Some(producer),
                listener: Some(Listener {
                    client,
                    entity: Some(SoundEmitter::Native {
                        namespace: ResourceNamespace::Client(client),
                        slot: native_entity as i32,
                    }),
                    origin,
                    right,
                    negate_dot: true,
                }),
                ..SoundEvent::default()
            },
            None,
            false,
        )?;
        self.listener = native_entity;
        Ok(())
    }
    pub(crate) fn completed(
        &mut self,
        owner: OutputConsumerId,
        sequence: u64,
        event: SoundEvent,
        accepted: bool,
    ) {
        if self.lifetime != event.client_lifetime {
            return;
        }
        if event.action == SoundAction::ResetClientLoops && self.reset == Some((owner, sequence)) {
            self.reset = None;
            self.reset_needed |= !accepted;
            if accepted {
                self.rewrite_active(|row| {
                    if row.last_publication.is_none_or(|newer| newer <= sequence) {
                        Row::default()
                    } else {
                        row
                    }
                });
                self.cursor = 0;
            }
        } else if event.action == SoundAction::ReleaseListener
            && self.release == Some((owner, sequence))
        {
            self.release = None;
            if accepted {
                self.producer = None;
                self.native_rules = None;
                self.releasing = false;
                self.listener = 0;
            } else {
                self.reset_needed = true;
            }
        } else if !accepted && self.producer.is_some() {
            self.reset_needed = true;
        } else if event.looping.is_some() && event.action == SoundAction::StartLoop && !accepted {
            self.reset_needed = true;
        }
    }
}
