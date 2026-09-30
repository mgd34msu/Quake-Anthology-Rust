//! Presentation-event routing for Quake session transitions.
//!
//! Sync port of donor `src/app/bootstrap/q1-session-actions.ts`. The donor's
//! `SimulationPresentationEvent` (from `./simulation/types.ts`, out of scope)
//! is shimmed minimally below: these two functions only read the source
//! family, the inner event kind, and the optional recipient.

use qa_core::identity::{ActorId, SeatId};

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

/// Minimal source family of a presentation event (shimmed from the donor
/// `SourcePresentationEvent` union; only observed families are named).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionSourceFamily {
    /// `q1`.
    Q1,
    /// `q1-session`.
    Q1Session,
    /// `q2-rerelease`.
    Q2Rerelease,
    /// Any other family (never matches).
    Other,
}

/// Minimal inner event kind (shimmed; only observed kinds are named).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionSourceEvent {
    /// Q1 `intermission`.
    Intermission,
    /// Q1-session `level-completed`.
    LevelCompleted,
    /// Q1-session `back-to-lobby`.
    BackToLobby,
    /// Q2-rerelease `end-of-unit`.
    EndOfUnit,
    /// Any other kind (never matches).
    Other,
}

/// Minimal presentation event carrying exactly what session actions read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionPresentationEvent {
    /// Source family.
    pub family: SessionSourceFamily,
    /// Inner event kind.
    pub event: SessionSourceEvent,
    /// Optional per-actor recipient (`None` broadcasts).
    pub recipient: Option<ActorId>,
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
pub fn source_level_completion(
    source: &SessionPresentationEvent,
) -> Option<LevelCompletionSource> {
    match (&source.family, &source.event) {
        (
            SessionSourceFamily::Q1,
            SessionSourceEvent::Intermission,
        )
        | (
            SessionSourceFamily::Q1Session,
            SessionSourceEvent::LevelCompleted,
        ) => Some(LevelCompletionSource::Q1),
        (
            SessionSourceFamily::Q2Rerelease,
            SessionSourceEvent::EndOfUnit,
        ) => Some(LevelCompletionSource::Q2),
        _ => None,
    }
}

/// Seats whose actor must leave for the lobby: a `q1-session`
/// `back-to-lobby` event exists that either broadcasts or names the seat's
/// actor.
#[must_use]
pub fn q1_session_departures(
    events: &[SessionPresentationEvent],
    seats: &[LocalSeatBinding],
) -> Vec<SeatId> {
    seats
        .iter()
        .filter(|local| {
            events.iter().any(|source| {
                source.family == SessionSourceFamily::Q1Session
                    && source.event == SessionSourceEvent::BackToLobby
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
    use qa_core::identity::IdentityOwner;

    fn event(
        family: SessionSourceFamily,
        event: SessionSourceEvent,
        recipient: Option<ActorId>,
    ) -> SessionPresentationEvent {
        SessionPresentationEvent {
            family,
            event,
            recipient,
        }
    }

    #[test]
    fn completion_sources() {
        assert_eq!(
            source_level_completion(&event(
                SessionSourceFamily::Q1,
                SessionSourceEvent::Intermission,
                None
            )),
            Some(LevelCompletionSource::Q1)
        );
        assert_eq!(
            source_level_completion(&event(
                SessionSourceFamily::Q1Session,
                SessionSourceEvent::LevelCompleted,
                None
            )),
            Some(LevelCompletionSource::Q1)
        );
        assert_eq!(
            source_level_completion(&event(
                SessionSourceFamily::Q2Rerelease,
                SessionSourceEvent::EndOfUnit,
                None
            )),
            Some(LevelCompletionSource::Q2)
        );
        assert_eq!(LevelCompletionSource::Q1.as_str(), "q1");
        assert_eq!(LevelCompletionSource::Q2.as_str(), "q2");
    }

    #[test]
    fn completion_ignores_mismatches() {
        assert_eq!(
            source_level_completion(&event(
                SessionSourceFamily::Q1,
                SessionSourceEvent::LevelCompleted,
                None
            )),
            None
        );
        assert_eq!(
            source_level_completion(&event(
                SessionSourceFamily::Q1Session,
                SessionSourceEvent::Intermission,
                None
            )),
            None
        );
        assert_eq!(
            source_level_completion(&event(
                SessionSourceFamily::Other,
                SessionSourceEvent::EndOfUnit,
                None
            )),
            None
        );
        assert_eq!(
            source_level_completion(&event(
                SessionSourceFamily::Q2Rerelease,
                SessionSourceEvent::Other,
                None
            )),
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
        let targeted = vec![event(
            SessionSourceFamily::Q1Session,
            SessionSourceEvent::BackToLobby,
            Some(owner.actor(1, 0)),
        )];
        assert_eq!(q1_session_departures(&targeted, &seats), vec![owner.seat(1)]);
        let broadcast = vec![event(
            SessionSourceFamily::Q1Session,
            SessionSourceEvent::BackToLobby,
            None,
        )];
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
        let foreign = vec![event(
            SessionSourceFamily::Q1Session,
            SessionSourceEvent::BackToLobby,
            Some(owner.actor(9, 0)),
        )];
        assert!(q1_session_departures(&foreign, &seats).is_empty());
        let wrong_kind = vec![event(
            SessionSourceFamily::Q1Session,
            SessionSourceEvent::LevelCompleted,
            None,
        )];
        assert!(q1_session_departures(&wrong_kind, &seats).is_empty());
        let wrong_family = vec![event(
            SessionSourceFamily::Q1,
            SessionSourceEvent::BackToLobby,
            None,
        )];
        assert!(q1_session_departures(&wrong_family, &seats).is_empty());
        assert!(q1_session_departures(&[], &seats).is_empty());
    }
}
