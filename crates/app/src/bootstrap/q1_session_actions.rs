//! Presentation-event routing for Quake session transitions.
//!
//! Sync port of donor `src/app/bootstrap/q1-session-actions.ts`. Events are
//! the hub [`SimulationPresentationEvent`] from `./simulation/types.ts`.

use qa_content::q1::foundation::types::Q1Event;
use qa_content::q2::rerelease::types::Q2RereleaseEvent;
use qa_core::identity::{ActorId, SeatId};

use super::simulation::types::{SimulationPresentationEvent, SourcePresentationEvent};

/// Which game completed a level, as the donor's `'q1' | 'q2'` with `None`
/// for anything else.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LevelCompletionSource {
    /// Quake.
    Q1,
    /// Quake II rerelease.
    Q2,
}

impl LevelCompletionSource {
    /// Donor wire spelling.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Q1 => "q1",
            Self::Q2 => "q2",
        }
    }
}

/// One local seat with its owning actor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalSeatBinding {
    /// Owning actor.
    pub actor: ActorId,
    /// Local seat.
    pub seat: SeatId,
}

/// Map a source event to the family that completed a level, if any.
#[must_use]
pub fn source_level_completion(source: &SimulationPresentationEvent) -> Option<LevelCompletionSource> {
    match &source.event {
        SourcePresentationEvent::Q1(Q1Event::Intermission { .. }) | SourcePresentationEvent::Q1LevelCompleted => {
            Some(LevelCompletionSource::Q1)
        }
        SourcePresentationEvent::Q2Rerelease(Q2RereleaseEvent::EndOfUnit { .. }) => Some(LevelCompletionSource::Q2),
        _ => None,
    }
}

/// Seats whose actor must leave for the lobby: a `q1-session`
/// `back-to-lobby` event exists that either broadcasts or names the seat's
/// actor.
#[must_use]
pub fn q1_session_departures(events: &[SimulationPresentationEvent], seats: &[LocalSeatBinding]) -> Vec<SeatId> {
    seats
        .iter()
        .filter(|local| {
            events.iter().any(|source| {
                matches!(source.event, SourcePresentationEvent::Q1BackToLobby)
                    && source
                        .recipient
                        .as_ref()
                        .is_none_or(|recipient| recipient == &local.actor)
            })
        })
        .map(|local| local.seat.clone())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_content::contract::ContentId;
    use qa_core::identity::IdentityOwner;

    fn event(event: SourcePresentationEvent, recipient: Option<ActorId>) -> SimulationPresentationEvent {
        SimulationPresentationEvent {
            event,
            owner: None,
            recipient,
            sequence: 0,
            content: ContentId("test:session:actions:v1".to_string()),
            seconds: 0.0,
            source_entity: None,
        }
    }

    fn intermission() -> SourcePresentationEvent {
        SourcePresentationEvent::Q1(Q1Event::Intermission {
            origin: qa_core::math::vec3(0.0, 0.0, 0.0),
            angles: qa_core::math::vec3(0.0, 0.0, 0.0),
            map: "e1m1".to_string(),
            exit_after: 5.0,
            track: 2,
        })
    }

    fn end_of_unit() -> SourcePresentationEvent {
        SourcePresentationEvent::Q2Rerelease(Q2RereleaseEvent::EndOfUnit {
            levels: Vec::new(),
            button_time: 1.0,
        })
    }

    #[test]
    fn completion_sources() {
        assert_eq!(
            source_level_completion(&event(intermission(), None)),
            Some(LevelCompletionSource::Q1)
        );
        assert_eq!(
            source_level_completion(&event(SourcePresentationEvent::Q1LevelCompleted, None)),
            Some(LevelCompletionSource::Q1)
        );
        assert_eq!(
            source_level_completion(&event(end_of_unit(), None)),
            Some(LevelCompletionSource::Q2)
        );
        assert_eq!(LevelCompletionSource::Q1.as_str(), "q1");
        assert_eq!(LevelCompletionSource::Q2.as_str(), "q2");
    }

    #[test]
    fn completion_ignores_mismatches() {
        assert_eq!(
            source_level_completion(&event(SourcePresentationEvent::Q1BackToLobby, None)),
            None
        );
        assert_eq!(
            source_level_completion(&event(
                SourcePresentationEvent::Q2Rerelease(Q2RereleaseEvent::Alpha {
                    actor: IdentityOwner::create("q1-actions-mismatch").unwrap().actor(0, 0),
                    alpha: 1.0,
                }),
                None
            )),
            None
        );
        assert_eq!(
            source_level_completion(&event(SourcePresentationEvent::CdTrack { track: 3 }, None)),
            None
        );
    }

    #[test]
    fn departures_match_recipient_or_broadcast() {
        let owner = IdentityOwner::create("q1-actions").unwrap();
        let seats = vec![
            LocalSeatBinding {
                actor: owner.actor(0, 0),
                seat: owner.seat(0),
            },
            LocalSeatBinding {
                actor: owner.actor(1, 0),
                seat: owner.seat(1),
            },
        ];
        let targeted = vec![event(SourcePresentationEvent::Q1BackToLobby, Some(owner.actor(1, 0)))];
        assert_eq!(q1_session_departures(&targeted, &seats), vec![owner.seat(1)]);
        let broadcast = vec![event(SourcePresentationEvent::Q1BackToLobby, None)];
        assert_eq!(
            q1_session_departures(&broadcast, &seats),
            vec![owner.seat(0), owner.seat(1)]
        );
    }

    #[test]
    fn departures_ignore_other_events_and_actors() {
        let owner = IdentityOwner::create("q1-actions-quiet").unwrap();
        let seats = vec![LocalSeatBinding {
            actor: owner.actor(0, 0),
            seat: owner.seat(0),
        }];
        let foreign = vec![event(SourcePresentationEvent::Q1BackToLobby, Some(owner.actor(9, 0)))];
        assert!(q1_session_departures(&foreign, &seats).is_empty());
        let wrong_kind = vec![event(SourcePresentationEvent::Q1LevelCompleted, None)];
        assert!(q1_session_departures(&wrong_kind, &seats).is_empty());
        let wrong_family = vec![event(intermission(), None)];
        assert!(q1_session_departures(&wrong_family, &seats).is_empty());
        assert!(q1_session_departures(&[], &seats).is_empty());
    }
}
