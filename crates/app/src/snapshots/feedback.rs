//! Player feedback derives from due CLIENT frames, never SERVER state.
use crate::client_resources::ResourceState;
use qa_core::{
    events::{EventRing, FrameEvent, OutputConsumerId, OutputTarget},
    primitives::{
        ClientId, ClockObservation, EntityId, HudState, ModuleId, PlayerSignals, ResourceNamespace,
        RuleSetId, SoundAction, SoundEmitter, SoundEvent, Vec3, ViewFeedbackEvent,
    },
    sys_events::EventTime,
};
use qa_gameplay::rules::feedback::{
    DamageFeedbackRules, PlayerEventRules, damage_feedback_rules, player_event_rules,
};
use qa_network::{channel::Channel, projection::PlayerFeedbackSample};

#[derive(Clone, Copy, PartialEq, Eq)]
struct Identity {
    lifetime: EntityId,
    namespace: ResourceNamespace,
    followed_client: u32,
}

pub(super) struct FeedbackPublication {
    rules: Option<DamageFeedbackRules>,
    event_rules: Option<PlayerEventRules>,
    identity: Option<Identity>,
    published: u32,
    signals: Option<PlayerSignals>,
    event_phase: u8,
    audio_owned: bool,
    event_drops: u64,
    view_baseline: bool,
    transition: Option<u32>,
    owner: Option<(ModuleId, EntityId)>,
    prior_axes: [Vec3; 3],
}

impl FeedbackPublication {
    pub(super) fn load(source: RuleSetId) -> Self {
        Self {
            rules: damage_feedback_rules(source),
            event_rules: player_event_rules(source),
            identity: None,
            published: 0,
            signals: None,
            event_phase: 0,
            audio_owned: false,
            event_drops: 0,
            view_baseline: true,
            transition: None,
            owner: None,
            prior_axes: [Vec3::default(); 3],
        }
    }
    fn reset(&mut self) {
        self.identity = None;
        self.transition = None;
        self.signals = None;
        self.event_phase = 0;
        self.reset_view();
    }
    fn reset_view(&mut self) {
        self.view_baseline = true;
        self.prior_axes = [Vec3::default(); 3];
    }
    #[expect(
        clippy::too_many_arguments,
        reason = "One publication borrows accepted CLIENT, HUD and ring owners"
    )]
    fn publish(
        &mut self,
        client: ClientId,
        lifetime: EntityId,
        sample: PlayerFeedbackSample,
        clock: ClockObservation,
        resources: Option<&ResourceState>,
        presentation: OutputConsumerId,
        audio_baseline: bool,
        hud: &mut HudState,
        consumer: OutputConsumerId,
        events: &mut EventRing,
    ) -> bool {
        let Some(rules) = self.rules else {
            return true;
        };
        let identity = Identity {
            lifetime,
            namespace: ResourceNamespace::Client(client),
            followed_client: sample.client_number,
        };
        let damage = sample.signals.damage;
        if self.identity != Some(identity) {
            self.identity = Some(identity);
            self.published = damage.sequence;
            self.signals = Some(sample.signals);
            self.event_phase = 0;
            hud.view_feedback = None;
            return true;
        }
        if self.view_baseline || self.owner.is_some() || damage.count == 0 {
            // FOLLOW/owner changes establish a native transition baseline.
            // Zero-count changes are observations, not new presentations.
            self.published = damage.sequence;
        } else if self.published != damage.sequence {
            if let Some(feedback) = rules.project(damage, sample.health, self.prior_axes, clock) {
                let Ok(sequence) = events.push_to(
                    OutputTarget::Local(client),
                    FrameEvent::ViewFeedback(ViewFeedbackEvent {
                        client,
                        client_lifetime: lifetime,
                        source: RuleSetId::Quake3,
                        namespace: identity.namespace,
                        native_client: identity.followed_client,
                        feedback,
                    }),
                ) else {
                    return false;
                };
                if !events.retained_for(consumer, sequence) {
                    events.reject_publication();
                    return false;
                }
            }
            self.published = damage.sequence;
        }
        self.publish_events(
            client,
            lifetime,
            sample,
            clock.queued,
            resources,
            presentation,
            audio_baseline,
            events,
        )
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "One producer borrows existing CLIENT resources and ring"
    )]
    fn publish_events(
        &mut self,
        client: ClientId,
        lifetime: EntityId,
        sample: PlayerFeedbackSample,
        queued: EventTime,
        resources: Option<&ResourceState>,
        presentation: OutputConsumerId,
        baseline: bool,
        events: &mut EventRing,
    ) -> bool {
        let Some(rules) = self.event_rules else {
            return true;
        };
        let Some(previous) = self.signals.filter(|_| !baseline) else {
            self.signals = Some(sample.signals);
            self.event_phase = 0;
            return true;
        };
        // The retained frame supplies desired data again on retry. Only this
        // three-phase ledger is kept, never a copied cue or second queue.
        while self.event_phase < 3 {
            let signal = if self.event_phase == 0 {
                (sample.signals.external.code != 0
                    && sample.signals.external.code != previous.external.code)
                    .then_some(sample.signals.external)
            } else {
                let ordinal = (sample.signals.sequence as i32)
                    .wrapping_sub(2)
                    .wrapping_add(i32::from(self.event_phase - 1));
                let index = ordinal as usize & 1;
                let old = previous.sequence as i32;
                (ordinal >= old
                    || (ordinal > old.wrapping_sub(2)
                        && sample.signals.events[index].code != previous.events[index].code))
                    .then_some(sample.signals.events[index])
            };
            if let Some(signal) = signal.filter(|signal| rules.code(signal.code) != 0) {
                let desired = rules.sound(signal).and_then(|cue| {
                    let resources = resources?;
                    Some(SoundEvent {
                        sound: resources.sound(cue.ordinal)?,
                        source_ordinal: Some(cue.ordinal),
                        spatial: resources.sound_policy(),
                        playback: rules.playback,
                        bus: cue.bus,
                        entity: Some(SoundEmitter::Native {
                            namespace: ResourceNamespace::Client(client),
                            slot: sample.client_number as i32,
                        }),
                        channel: cue.channel,
                        queued_at: queued,
                        volume: 1.0,
                        attenuation: 1.0,
                        action: SoundAction::Play,
                        client: Some(client),
                        client_lifetime: Some(lifetime),
                        ..Default::default()
                    })
                });
                if let Some(sound) = desired {
                    let Ok(sequence) =
                        events.push_to(OutputTarget::Presentation, FrameEvent::Sound(sound))
                    else {
                        return false;
                    };
                    if !events.retained_for(presentation, sequence) {
                        events.reject_publication();
                        return false;
                    }
                } else {
                    // Unsupported recipes and unavailable native resources are
                    // explicit semantic drops, not endless producer retries.
                    self.event_drops = self.event_drops.saturating_add(1);
                }
            }
            self.event_phase += 1;
        }
        self.signals = Some(sample.signals);
        self.event_phase = 0;
        true
    }
}

impl super::SnapshotBinding {
    pub fn player_event_drops(&self) -> u64 {
        self.presentation
            .as_ref()
            .map_or(0, |row| row.feedback.event_drops)
    }
    pub(super) fn baseline_player_events(&mut self) {
        if let Some(row) = self.presentation.as_mut() {
            row.feedback.signals = None;
            row.feedback.event_phase = 0;
        }
    }
    pub(crate) fn owns_client_view(&self) -> bool {
        self.presentation
            .as_ref()
            .is_some_and(|row| row.feedback.owner.is_some())
    }
    pub(crate) fn attach_client_view(&mut self, module: ModuleId, lifetime: EntityId) {
        if let Some(row) = self.presentation.as_mut() {
            row.feedback.owner = Some((module, lifetime));
            row.feedback.reset_view();
        }
    }
    pub(crate) fn detach_client_view(&mut self, module: ModuleId, lifetime: EntityId) {
        if let Some(row) = self.presentation.as_mut()
            && row.feedback.owner == Some((module, lifetime))
        {
            row.feedback.owner = None;
            row.feedback.reset_view();
        }
    }
    pub(crate) fn reset_feedback(&mut self) {
        if let Some(row) = self.presentation.as_mut() {
            row.feedback.reset();
        }
    }
    pub(crate) fn submitted_view(&mut self, axes: [Vec3; 3]) {
        if let Some(row) = self.presentation.as_mut() {
            row.feedback.prior_axes = axes;
        }
    }
    #[expect(
        clippy::too_many_arguments,
        reason = "One transition borrows existing connection, resources, HUD and ring owners"
    )]
    pub(crate) fn publish_feedback(
        &mut self,
        lifetime: EntityId,
        channel: &Channel,
        resources: Option<&ResourceState>,
        presentation: OutputConsumerId,
        hud: &mut HudState,
        events: &mut EventRing,
        queued: EventTime,
    ) {
        let audio_owned = self.owns_client_audio();
        if let Some(row) = self.presentation.as_mut() {
            // Audio and view ownership are independent. Baseline every due
            // frame during an audio handoff without replaying the backlog.
            let audio_baseline = audio_owned
                || audio_owned != row.feedback.audio_owned
                || row.feedback.signals.is_none();
            row.feedback.audio_owned = audio_owned;
            if row.feedback.rules.is_none() {
                return;
            }
            let presented = row.clock.sample().time;
            let due = |frame: &qa_network::snapshots::ReceivedFrame<'_>| {
                matches!((frame.time(), presented),
                    (qa_core::primitives::ThinkTime::Milliseconds(native),
                     qa_core::primitives::ThinkTime::Milliseconds(now)) if native <= now)
            };
            if row.feedback.transition.is_none() {
                // Initial binding, resync and owner release establish the
                // newest due baseline; canceled historical damage stays canceled.
                if let Some((frame, sample)) = channel
                    .retained_snapshots()
                    .take_while(due)
                    .filter_map(|frame| {
                        self.projection
                            .feedback_sample(frame.player_words()?)
                            .map(|sample| (frame, sample))
                    })
                    .last()
                {
                    row.feedback.publish(
                        self.client,
                        lifetime,
                        sample,
                        ClockObservation {
                            native: frame.time(),
                            queued,
                        },
                        resources,
                        presentation,
                        true,
                        hud,
                        self.output,
                        events,
                    );
                    row.feedback.transition = Some(frame.sequence());
                    row.feedback.view_baseline = false;
                }
                return;
            }
            for frame in channel.retained_snapshots() {
                if row
                    .feedback
                    .transition
                    .is_some_and(|old| frame.sequence() <= old)
                {
                    if audio_baseline
                        && due(&frame)
                        && let Some(sample) = frame
                            .player_words()
                            .and_then(|words| self.projection.feedback_sample(words))
                    {
                        // Owner changes may happen with no newer snapshot.
                        // Observe the latest due sample in this same traversal.
                        row.feedback.signals = Some(sample.signals);
                        row.feedback.event_phase = 0;
                    }
                    continue;
                }
                if !due(&frame) {
                    break;
                }
                let Some(sample) = frame
                    .player_words()
                    .and_then(|words| self.projection.feedback_sample(words))
                else {
                    row.feedback.transition = Some(frame.sequence());
                    continue;
                };
                if !row.feedback.publish(
                    self.client,
                    lifetime,
                    sample,
                    ClockObservation {
                        native: frame.time(),
                        queued,
                    },
                    resources,
                    presentation,
                    audio_baseline,
                    hud,
                    self.output,
                    events,
                ) {
                    return;
                }
                row.feedback.transition = Some(frame.sequence());
            }
            row.feedback.view_baseline = false;
        }
    }
    pub(crate) fn accepts_feedback(&self, event: ViewFeedbackEvent) -> bool {
        event.source == RuleSetId::Quake3
            && self.presentation.as_ref().is_some_and(|row| {
                row.feedback.owner.is_none()
                    && row.feedback.identity
                        == Some(Identity {
                            lifetime: event.client_lifetime,
                            namespace: event.namespace,
                            followed_client: event.native_client,
                        })
            })
    }
}
