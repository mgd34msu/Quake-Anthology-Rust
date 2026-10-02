//! Per-seat Quake fog with worldspawn defaults and transition events.
//!
//! Port of `/home/buzzkill/Projects/quake-typescript/src/app/bootstrap/q1-fog.ts`
//! (`Q1MapFog`). Fog state, worldspawn parsing, and the scene-fog value come from the
//! ported [`Q1FogState`](qa_client::materials::fog::Q1FogState),
//! [`q1_world_fog`](qa_client::materials::fog::q1_world_fog), and
//! [`SceneFog`](qa_client::render::types::SceneFog). The presentation-event stream
//! (`./simulation/types.ts`, out of scope) is shimmed minimally below: this class only
//! reads owner retirement plus the `q1-fog` owner/content/player/transition/sky-factor
//! fields. Construction reports [`ClientError`](qa_client::ClientError) because the merged
//! worldspawn parser is fallible where the donor's is total.

use std::collections::HashSet;

use qa_client::materials::fog::{q1_world_fog, Q1FogState, Q1FogTransition};
use qa_client::render::types::SceneFog;
use qa_client::ClientError;
use qa_content::contract::{same_presentation_owner, ContentId, PresentationOwner};
use qa_core::identity::ActorId;
use qa_core::math::vec3;

/// One `q1-fog` transition event (donor `SimulationPresentationEvent` subset).
#[derive(Debug, Clone, PartialEq)]
pub struct Q1FogTransitionEvent {
    /// Owning component activation, when component-owned.
    pub owner: Option<PresentationOwner>,
    /// Content the event belongs to.
    pub content: ContentId,
    /// Target player, or [`None`] for every seat.
    pub player: Option<ActorId>,
    /// Installed fog transition.
    pub transition: Q1FogTransition,
    /// Sky blend factor.
    pub sky_factor: f32,
}

/// Presentation input the map fog observes (donor event-stream subset).
#[derive(Debug, Clone, PartialEq)]
pub enum Q1MapFogEvent {
    /// A presentation owner retired.
    OwnerRetired {
        /// Retired activation.
        owner: PresentationOwner,
    },
    /// A fog transition.
    Transition(Q1FogTransitionEvent),
    /// Any other event, ignored.
    Other,
}

/// One seat in one published world (donor `Q1MapFog`).
///
/// Construction resets fades on world travel.
pub struct Q1MapFog {
    state: Q1FogState,
    sky_factor: f32,
    owner: Option<PresentationOwner>,
    initial: Q1FogTransition,
    initial_enabled: bool,
    contents: HashSet<ContentId>,
    actor: ActorId,
    enabled: bool,
}

impl Q1MapFog {
    /// Build the fog for one seat's worldspawn entities.
    pub fn new(
        entities: &str,
        contents: HashSet<ContentId>,
        actor: ActorId,
        enabled: bool,
    ) -> Result<Self, ClientError> {
        let initial = q1_world_fog(entities)?;
        let mut state = Q1FogState::new();
        state.install(initial);
        Ok(Self {
            state,
            sky_factor: 0.5,
            owner: None,
            initial,
            initial_enabled: enabled,
            contents,
            actor,
            enabled,
        })
    }

    /// Receive presentation events (donor `receive`).
    pub fn receive(&mut self, events: &[Q1MapFogEvent]) {
        for event in events {
            match event {
                Q1MapFogEvent::OwnerRetired { owner } => {
                    if same_presentation_owner(self.owner.as_ref(), owner) {
                        self.state.install(self.initial);
                        self.enabled = self.initial_enabled;
                        self.sky_factor = 0.5;
                        self.owner = None;
                    }
                }
                Q1MapFogEvent::Transition(transition) => {
                    if transition.owner.is_none() && !self.contents.contains(&transition.content) {
                        continue;
                    }
                    if transition.player.as_ref().is_some_and(|player| player != &self.actor) {
                        continue;
                    }
                    self.enabled = true;
                    self.owner = transition.owner.clone();
                    self.state.install(transition.transition);
                    self.sky_factor = transition.sky_factor;
                }
                Q1MapFogEvent::Other => {}
            }
        }
    }

    /// Whether fog is active.
    #[must_use]
    pub fn active(&self) -> bool {
        self.enabled
    }

    /// Sample the current fog (donor `current`).
    #[must_use]
    pub fn current(&self, seconds: f32) -> SceneFog {
        let value = self.state.sample(seconds);
        let byte = |component: f32| ((f64::from(component).clamp(0.0, 1.0) * 255.0).round() / 255.0) as f32;
        SceneFog::Q1 {
            density: value.density,
            color: vec3(byte(value.color.x), byte(value.color.y), byte(value.color.z)),
            sky_factor: self.sky_factor,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_client::materials::fog::{Q1Fog, Q1FogState};
    use qa_core::identity::IdentityOwner;
    use qa_core::math::vec3;

    fn fog() -> (Q1MapFog, ActorId, ContentId) {
        let owner = IdentityOwner::create("q1-fog").unwrap();
        let actor = owner.actor(0, 1);
        let content = ContentId("q1:classic:baseq3:1".to_string());
        let mut contents = HashSet::new();
        contents.insert(content.clone());
        let fog = Q1MapFog::new("", contents, actor.clone(), true).unwrap();
        (fog, actor, content)
    }

    fn transition(content: &ContentId, player: Option<ActorId>) -> Q1FogTransitionEvent {
        let mut state = Q1FogState::new();
        state.update(
            Q1Fog {
                density: 0.5,
                color: vec3(1.0, 0.0, 0.0),
            },
            0.0,
            0.0,
        );
        Q1FogTransitionEvent {
            owner: None,
            content: content.clone(),
            player,
            transition: state.capture(),
            sky_factor: 0.25,
        }
    }

    #[test]
    fn transition_installs_for_matching_seat() {
        let (mut fog, actor, content) = fog();
        fog.receive(&[Q1MapFogEvent::Transition(transition(&content, Some(actor)))]);
        assert!(fog.active());
        let SceneFog::Q1 {
            density,
            color,
            sky_factor,
        } = fog.current(1.0)
        else {
            panic!("expected Q1 fog");
        };
        assert_eq!(density, 0.5);
        assert_eq!(color, vec3(1.0, 0.0, 0.0));
        assert_eq!(sky_factor, 0.25);
    }

    #[test]
    fn foreign_player_and_content_are_ignored() {
        let (mut fog, _, content) = fog();
        let foreign_owner = IdentityOwner::create("q1-fog-foreign").unwrap();
        let foreign = foreign_owner.actor(0, 1);
        fog.receive(&[Q1MapFogEvent::Transition(transition(&content, Some(foreign)))]);
        let SceneFog::Q1 { density, .. } = fog.current(1.0) else {
            panic!("expected Q1 fog");
        };
        assert_eq!(density, 0.0);
        let mut stray = transition(&ContentId("q1:classic:other:1".to_string()), None);
        stray.owner = None;
        fog.receive(&[Q1MapFogEvent::Transition(stray)]);
        let SceneFog::Q1 { density, .. } = fog.current(1.0) else {
            panic!("expected Q1 fog");
        };
        assert_eq!(density, 0.0);
    }

    #[test]
    fn owner_retirement_resets_fades() {
        let (mut fog, actor, content) = fog();
        let provider = qa_core::identity::ProviderId::new("test", "fog");
        let mut owned = transition(&content, Some(actor));
        owned.owner = Some(PresentationOwner {
            provider,
            generation: 1,
        });
        fog.receive(&[Q1MapFogEvent::Transition(owned.clone())]);
        assert!(fog.active());
        fog.receive(&[Q1MapFogEvent::OwnerRetired {
            owner: owned.owner.clone().unwrap(),
        }]);
        let SceneFog::Q1 {
            density, sky_factor, ..
        } = fog.current(1.0)
        else {
            panic!("expected Q1 fog");
        };
        assert_eq!(density, 0.0);
        assert_eq!(sky_factor, 0.5);
    }
}
