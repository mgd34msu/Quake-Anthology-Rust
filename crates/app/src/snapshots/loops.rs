//! Numeric last-publication state for the connection's accepted CLIENT loops.
use crate::{client_resources::ResourceState, client_scene::ClientEntities};
use qa_core::{
    events::{EventRing, FrameEvent, OutputConsumerId, OutputTarget},
    primitives::{
        ClientId, EntityId, LoopLifetime, LoopPlayback, ResourceNamespace, SoundAction,
        SoundEmitter, SoundEvent, SoundId, SpatialPolicy,
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
}

/// This stores only successfully published numeric settings. The desired state,
/// pose and native words stay in the channel's one accepted snapshot ring.
pub(crate) struct LoopPublication {
    rules: LoopRules,
    rows: Box<[Row]>,
    present: StampSet,
    active: Vec<usize>,
    cursor: usize,
    lifetime: Option<EntityId>,
    owner: Option<OutputConsumerId>,
    reset_needed: bool,
    reset: Option<(OutputConsumerId, u64)>,
}
impl LoopPublication {
    pub(crate) fn load(rules: LoopRules, capacity: usize) -> Self {
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
        events.retained_for(owner, sequence).then_some(sequence)
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
        if self.lifetime != Some(lifetime) {
            // Disconnect already owns full-lifetime retirement. This arena is
            // retained only for its current canonical CLIENT connection.
            self.rows.fill(Row::default());
            self.active.clear();
            self.lifetime = Some(lifetime);
            self.owner = Some(owner);
            self.reset_needed = false;
            self.reset = None;
            self.cursor = 0;
        } else if self.owner != Some(owner) || events.needs_resync(owner) {
            self.reset_needed = true;
            self.owner = Some(owner);
        }
        if let Some((pending_owner, sequence)) = self.reset {
            if events.retained_for(pending_owner, sequence) {
                return;
            }
            self.reset = None;
            self.reset_needed = true;
        }
        if events.needs_resync(owner) || events.counters(owner).is_none() {
            return;
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
            }
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
    fn tracked(row: Row) -> bool {
        row.published.is_some()
            || row.stop_key.is_some()
            || row.event_once_seen
            || row.stopped_sequence.is_some()
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
        entities.visit(|number, pose| {
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
        let once = pose
            .as_ref()
            .is_some_and(|pose| self.rules.event_once(&pose.record.appearance.native));
        let seen = self.rows[index].event_once_seen;
        let stop = pose.as_ref().and_then(|pose| {
            let native = &pose.record.appearance.native;
            Some((
                self.rules.stop(native)?,
                self.rules
                    .stop_emitter(&pose.record.appearance, index as u32)?,
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
            let (playback, spatial) = self
                .rules
                .admit(&pose.record.appearance, resources.loop_spatial_policy())?;
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
            self.reset_needed = !accepted;
            if accepted {
                self.rows.fill(Row::default());
                self.active.clear();
                self.cursor = 0;
            }
        } else if event.looping.is_some() && event.action == SoundAction::StartLoop && !accepted {
            self.reset_needed = true;
        }
    }
}
