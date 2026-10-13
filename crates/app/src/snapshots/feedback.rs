//! Damage publications derive from accepted CLIENT state, never SERVER state.
use qa_core::{
    events::{EventRing, FrameEvent, OutputConsumerId, OutputTarget},
    primitives::{
        ClientId, ClockObservation, EntityId, HudState, ModuleId, ResourceNamespace, RuleSetId,
        Vec3, ViewFeedbackEvent,
    },
    sys_events::EventTime,
};
use qa_gameplay::rules::feedback::{DamageFeedbackRules, damage_feedback_rules};
use qa_network::{channel::Channel, projection::PlayerFeedbackSample};

#[derive(Clone, Copy, PartialEq, Eq)]
struct Identity {
    lifetime: EntityId,
    namespace: ResourceNamespace,
    followed_client: u32,
}

pub(super) struct FeedbackPublication {
    rules: Option<DamageFeedbackRules>,
    identity: Option<Identity>,
    published: u32,
    transition: Option<u32>,
    owner: Option<(ModuleId, EntityId)>,
    prior_axes: [Vec3; 3],
}

impl FeedbackPublication {
    pub(super) fn load(source: RuleSetId) -> Self {
        Self {
            rules: damage_feedback_rules(source),
            identity: None,
            published: 0,
            transition: None,
            owner: None,
            prior_axes: [Vec3::default(); 3],
        }
    }
    fn reset(&mut self) {
        self.identity = None;
        self.transition = None;
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
        let damage = sample.damage;
        if self.identity != Some(identity) {
            self.identity = Some(identity);
            self.published = damage.sequence;
            hud.view_feedback = None;
            return true;
        }
        if self.owner.is_some() || damage.count == 0 {
            // FOLLOW/owner changes establish a native transition baseline.
            // Zero-count changes are observations, not new presentations.
            self.published = damage.sequence;
            return true;
        }
        if self.published == damage.sequence {
            return true;
        }
        let Some(feedback) = rules.project(damage, sample.health, self.prior_axes, clock) else {
            self.published = damage.sequence;
            return true;
        };
        if let Ok(sequence) = events.push_to(
            OutputTarget::Local(client),
            FrameEvent::ViewFeedback(ViewFeedbackEvent {
                client,
                client_lifetime: lifetime,
                source: RuleSetId::Quake3,
                namespace: identity.namespace,
                native_client: identity.followed_client,
                feedback,
            }),
        ) {
            if events.retained_for(consumer, sequence) {
                self.published = damage.sequence;
                return true;
            } else {
                events.reject_publication();
            }
        }
        false
    }
}

impl super::SnapshotBinding {
    pub(crate) fn owns_client_view(&self) -> bool {
        self.presentation
            .as_ref()
            .is_some_and(|row| row.feedback.owner.is_some())
    }
    pub(crate) fn attach_client_view(&mut self, module: ModuleId, lifetime: EntityId) {
        if let Some(row) = self.presentation.as_mut() {
            row.feedback.owner = Some((module, lifetime));
            row.feedback.reset();
        }
    }
    pub(crate) fn detach_client_view(&mut self, module: ModuleId, lifetime: EntityId) {
        if let Some(row) = self.presentation.as_mut()
            && row.feedback.owner == Some((module, lifetime))
        {
            row.feedback.owner = None;
            row.feedback.reset();
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
    pub(crate) fn publish_feedback(
        &mut self,
        lifetime: EntityId,
        channel: &Channel,
        hud: &mut HudState,
        events: &mut EventRing,
        queued: EventTime,
    ) {
        if let Some(row) = self.presentation.as_mut() {
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
                            .damage_feedback(frame.player_words()?)
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
                        hud,
                        self.output,
                        events,
                    );
                    row.feedback.transition = Some(frame.sequence());
                }
                return;
            }
            for frame in channel.retained_snapshots() {
                if row
                    .feedback
                    .transition
                    .is_some_and(|old| frame.sequence() <= old)
                {
                    continue;
                }
                if !due(&frame) {
                    break;
                }
                let Some(sample) = frame
                    .player_words()
                    .and_then(|words| self.projection.damage_feedback(words))
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
                    hud,
                    self.output,
                    events,
                ) {
                    break;
                }
                row.feedback.transition = Some(frame.sequence());
            }
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
