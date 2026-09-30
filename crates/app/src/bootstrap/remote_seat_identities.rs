//! Frontend identity reservation for remote (multi-seat) play.
//!
//! Sync port of donor `src/app/bootstrap/remote-seat-identities.ts`.
//! `qa_world::session` currently ports only the headless simulation and has
//! no `EngineSession`/`SessionClient`/`SessionSeat`, so this module defines
//! the minimal session surface it needs as local traits (see
//! absorbed-contracts). Each native server still assigns its own wire
//! player number independently.

use std::fmt::Display;

use thiserror::Error;

/// Failure of remote seat identity preparation.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum RemoteSeatError {
    /// The requested player count is not one to four.
    #[error("Remote play requires one to four local players")]
    InvalidCount,
    /// Closing added clients failed; member messages are kept in `errors`
    /// (donor: `AggregateError`).
    #[error("Remote seat identity cleanup failed")]
    CleanupFailed {
        /// Member failure messages, in reverse-add order.
        errors: Vec<String>,
    },
    /// A seat reservation failed and its client cleanup failed too.
    #[error("Remote seat reservation failed")]
    ReservationFailed {
        /// Seat failure message.
        cause: String,
        /// Client cleanup failure message.
        cleanup: String,
    },
    /// Preparation failed and rolling back added clients failed too.
    #[error("Remote seat preparation failed")]
    PreparationFailed {
        /// Original failure message.
        cause: String,
        /// Rollback failure messages joined with `"; "`.
        cleanup: String,
    },
    /// `validate`/`publish` ran outside the prepared phase.
    #[error("Remote seat identities are {0}")]
    WrongPhase(String),
    /// The session backend failed.
    #[error("{0}")]
    Session(String),
}

/// Minimal client handle (absorbed `SessionClient` surface).
pub trait SeatClient {
    /// Close failure.
    type Error: Display;
    /// Client slot.
    fn slot(&self) -> u32;
    /// Release the prepared client.
    fn close(&mut self) -> Result<(), Self::Error>;
}

/// Minimal seat handle (absorbed `SessionSeat` surface).
pub trait LocalSeat {
    /// Seat index.
    fn index(&self) -> u32;
}

/// Retired seats published by the session (absorbed `SessionResource`).
pub trait PublishedSeats {
    /// Close failure.
    type Error: Display;
    /// Release retired seats.
    fn close(self) -> Result<(), Self::Error>;
}

/// One reserved frontend identity.
#[derive(Debug)]
pub struct RemoteSeatIdentity<C, S> {
    /// Prepared client.
    pub client: C,
    /// Prepared seat.
    pub seat: S,
}

/// Minimal session surface (absorbed `EngineSession` surface).
pub trait SeatSession {
    /// Client handle.
    type Client: SeatClient;
    /// Seat handle.
    type Seat: LocalSeat;
    /// Published retired seats.
    type Published: PublishedSeats;
    /// Backend failure.
    type Error: Display;
    /// The live client owning `slot`, if any.
    fn client_at(&self, slot: u32) -> Option<&Self::Client>;
    /// Prepare a client in a free slot.
    fn prepare_client(&mut self, slot: u32) -> Result<Self::Client, Self::Error>;
    /// Prepare a seat at a free index bound to `client`.
    fn prepare_seat(&mut self, index: u32, client: &Self::Client) -> Result<Self::Seat, Self::Error>;
    /// Validate publishing `added` seats.
    fn validate_local_seats(&self, added: &[RemoteSeatIdentity<Self::Client, Self::Seat>]) -> Result<(), Self::Error>;
    /// Publish `added` seats, returning retired seats to close.
    fn publish_local_seats(
        &mut self,
        added: &[RemoteSeatIdentity<Self::Client, Self::Seat>],
    ) -> Result<Self::Published, Self::Error>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SeatPhase {
    Prepared,
    Published,
    Discarded,
}

impl SeatPhase {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Prepared => "prepared",
            Self::Published => "published",
            Self::Discarded => "discarded",
        }
    }
}

fn close_added<C: SeatClient, S>(
    selected: &mut [RemoteSeatIdentity<C, S>],
    added_start: usize,
) -> Result<(), Vec<String>> {
    let mut failures = Vec::new();
    for entry in selected[added_start..].iter_mut().rev() {
        if let Err(error) = entry.client.close() {
            failures.push(error.to_string());
        }
    }
    if failures.is_empty() {
        Ok(())
    } else {
        Err(failures)
    }
}

/// A prepared seat set: retained identities plus newly added ones.
#[derive(Debug)]
pub struct RemoteSeatPreparation<'session, S: SeatSession + ?Sized> {
    session: &'session mut S,
    selected: Vec<RemoteSeatIdentity<S::Client, S::Seat>>,
    added_start: usize,
    phase: SeatPhase,
}

impl<S: SeatSession + ?Sized> RemoteSeatPreparation<'_, S> {
    /// Whether `publish` has run.
    #[must_use]
    pub fn published(&self) -> bool {
        self.phase == SeatPhase::Published
    }

    /// Retained plus added identities.
    #[must_use]
    pub fn selected(&self) -> &[RemoteSeatIdentity<S::Client, S::Seat>] {
        &self.selected
    }

    /// Newly added identities.
    #[must_use]
    pub fn added(&self) -> &[RemoteSeatIdentity<S::Client, S::Seat>] {
        &self.selected[self.added_start..]
    }

    /// Validate publishing the added seats.
    pub fn validate(&self) -> Result<(), RemoteSeatError> {
        if self.phase != SeatPhase::Prepared {
            return Err(RemoteSeatError::WrongPhase(self.phase.as_str().to_string()));
        }
        self.session
            .validate_local_seats(&self.selected[self.added_start..])
            .map_err(|error| RemoteSeatError::Session(error.to_string()))
    }

    /// Publish the added seats and close retired seats.
    pub fn publish(&mut self) -> Result<(), RemoteSeatError> {
        self.validate()?;
        let guard = self
            .session
            .publish_local_seats(&self.selected[self.added_start..])
            .map_err(|error| RemoteSeatError::Session(error.to_string()))?;
        guard
            .close()
            .map_err(|error| RemoteSeatError::Session(error.to_string()))?;
        self.phase = SeatPhase::Published;
        Ok(())
    }

    /// Release added clients; a no-op once published or discarded.
    pub fn discard(&mut self) -> Result<(), RemoteSeatError> {
        if self.phase != SeatPhase::Prepared {
            return Ok(());
        }
        self.phase = SeatPhase::Discarded;
        close_added(&mut self.selected, self.added_start).map_err(|errors| RemoteSeatError::CleanupFailed { errors })
    }
}

/// Reserve frontend identities: keep the first `count` retained identities
/// and prepare more until `count` seats are selected.
pub fn prepare_remote_seat_identities<S: SeatSession>(
    session: &mut S,
    retained: Vec<RemoteSeatIdentity<S::Client, S::Seat>>,
    count: i32,
) -> Result<RemoteSeatPreparation<'_, S>, RemoteSeatError> {
    if !(1..=4).contains(&count) {
        return Err(RemoteSeatError::InvalidCount);
    }
    let count = count as usize;
    let mut selected: Vec<RemoteSeatIdentity<S::Client, S::Seat>> = retained.into_iter().take(count).collect();
    let added_start = selected.len();
    while selected.len() < count {
        let mut slot = 0u32;
        while session.client_at(slot).is_some()
            || selected[added_start..].iter().any(|entry| entry.client.slot() == slot)
        {
            slot += 1;
        }
        let mut index = 0u32;
        while selected.iter().any(|entry| entry.seat.index() == index) {
            index += 1;
        }
        let client = match session.prepare_client(slot) {
            Ok(client) => client,
            Err(error) => {
                return match close_added(&mut selected, added_start) {
                    Ok(()) => Err(RemoteSeatError::Session(error.to_string())),
                    Err(failures) => Err(RemoteSeatError::PreparationFailed {
                        cause: error.to_string(),
                        cleanup: failures.join("; "),
                    }),
                };
            }
        };
        let seat = match session.prepare_seat(index, &client) {
            Ok(seat) => seat,
            Err(error) => {
                let mut client = client;
                let failure = match client.close() {
                    Ok(()) => RemoteSeatError::Session(error.to_string()),
                    Err(cleanup) => RemoteSeatError::ReservationFailed {
                        cause: error.to_string(),
                        cleanup: cleanup.to_string(),
                    },
                };
                return match close_added(&mut selected, added_start) {
                    Ok(()) => Err(failure),
                    Err(failures) => Err(RemoteSeatError::PreparationFailed {
                        cause: failure.to_string(),
                        cleanup: failures.join("; "),
                    }),
                };
            }
        };
        selected.push(RemoteSeatIdentity { client, seat });
    }
    Ok(RemoteSeatPreparation {
        session,
        selected,
        added_start,
        phase: SeatPhase::Prepared,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug)]
    struct FakeClient {
        slot: u32,
        closed: bool,
        fail_close: bool,
    }

    impl SeatClient for FakeClient {
        type Error = String;

        fn slot(&self) -> u32 {
            self.slot
        }

        fn close(&mut self) -> Result<(), String> {
            self.closed = true;
            if self.fail_close {
                return Err(format!("close slot {} failed", self.slot));
            }
            Ok(())
        }
    }

    #[derive(Debug)]
    struct FakeSeat {
        index: u32,
    }

    impl LocalSeat for FakeSeat {
        fn index(&self) -> u32 {
            self.index
        }
    }

    struct FakePublished {
        fail_close: bool,
    }

    impl PublishedSeats for FakePublished {
        type Error = String;

        fn close(self) -> Result<(), String> {
            if self.fail_close {
                return Err("retired close failed".to_string());
            }
            Ok(())
        }
    }

    #[derive(Debug, Default)]
    struct FakeSession {
        owned: Vec<FakeClient>,
        fail_client: bool,
        fail_seat: bool,
        fail_close_new: bool,
        fail_validate: bool,
        fail_publish: bool,
        fail_retired: bool,
        published: usize,
    }

    impl SeatSession for FakeSession {
        type Client = FakeClient;
        type Seat = FakeSeat;
        type Published = FakePublished;
        type Error = String;

        fn client_at(&self, slot: u32) -> Option<&FakeClient> {
            self.owned.iter().find(|client| client.slot == slot)
        }

        fn prepare_client(&mut self, slot: u32) -> Result<FakeClient, String> {
            if self.fail_client {
                return Err("no client".to_string());
            }
            Ok(FakeClient {
                slot,
                closed: false,
                fail_close: self.fail_close_new,
            })
        }

        fn prepare_seat(&mut self, index: u32, _client: &FakeClient) -> Result<FakeSeat, String> {
            if self.fail_seat {
                return Err("no seat".to_string());
            }
            Ok(FakeSeat { index })
        }

        fn validate_local_seats(&self, _added: &[RemoteSeatIdentity<FakeClient, FakeSeat>]) -> Result<(), String> {
            if self.fail_validate {
                return Err("bad seats".to_string());
            }
            Ok(())
        }

        fn publish_local_seats(
            &mut self,
            added: &[RemoteSeatIdentity<FakeClient, FakeSeat>],
        ) -> Result<FakePublished, String> {
            if self.fail_publish {
                return Err("no publish".to_string());
            }
            self.published = added.len();
            Ok(FakePublished {
                fail_close: self.fail_retired,
            })
        }
    }

    fn retained(slot: u32, index: u32) -> RemoteSeatIdentity<FakeClient, FakeSeat> {
        RemoteSeatIdentity {
            client: FakeClient {
                slot,
                closed: false,
                fail_close: false,
            },
            seat: FakeSeat { index },
        }
    }

    #[test]
    fn rejects_bad_counts() {
        let mut session = FakeSession::default();
        for count in [0, -1, 5] {
            assert_eq!(
                prepare_remote_seat_identities(&mut session, Vec::new(), count).unwrap_err(),
                RemoteSeatError::InvalidCount
            );
        }
    }

    #[test]
    fn reuses_retained_and_skips_occupied() {
        // Retained clients stay session-owned, so the owned set covers them.
        let mut session = FakeSession {
            owned: vec![
                FakeClient {
                    slot: 0,
                    closed: false,
                    fail_close: false,
                },
                FakeClient {
                    slot: 1,
                    closed: false,
                    fail_close: false,
                },
            ],
            ..FakeSession::default()
        };
        let mut prepared = prepare_remote_seat_identities(&mut session, vec![retained(1, 0)], 2).unwrap();
        assert_eq!(prepared.selected().len(), 2);
        assert_eq!(prepared.added().len(), 1);
        assert_eq!(prepared.added()[0].client.slot, 2);
        assert_eq!(prepared.added()[0].seat.index, 1);
        assert!(!prepared.published());
        prepared.validate().unwrap();
        prepared.publish().unwrap();
        assert!(prepared.published());
        assert!(prepared.discard().is_ok());
        assert_eq!(
            prepared.validate().unwrap_err(),
            RemoteSeatError::WrongPhase("published".to_string())
        );
    }

    #[test]
    fn discard_releases_added_in_reverse() {
        let mut session = FakeSession::default();
        let mut prepared = prepare_remote_seat_identities(&mut session, Vec::new(), 3).unwrap();
        prepared.discard().unwrap();
        assert!(prepared.selected().iter().all(|entry| entry.client.closed));
        assert!(prepared.discard().is_ok());
    }

    #[test]
    fn seat_failure_rolls_back_added_clients() {
        let mut session = FakeSession {
            fail_seat: true,
            ..FakeSession::default()
        };
        let error = prepare_remote_seat_identities(&mut session, Vec::new(), 1).unwrap_err();
        assert_eq!(error, RemoteSeatError::Session("no seat".to_string()));

        let mut session = FakeSession {
            fail_client: true,
            ..FakeSession::default()
        };
        let error = prepare_remote_seat_identities(&mut session, Vec::new(), 1).unwrap_err();
        assert_eq!(error, RemoteSeatError::Session("no client".to_string()));
    }

    #[test]
    fn cleanup_failures_aggregate() {
        let mut session = FakeSession {
            fail_client: true,
            ..FakeSession::default()
        };
        session.fail_client = false;
        let mut prepared = prepare_remote_seat_identities(&mut session, Vec::new(), 1).unwrap();
        prepared.selected[0].client.fail_close = true;
        let error = prepared.discard().unwrap_err();
        assert_eq!(error.to_string(), "Remote seat identity cleanup failed");
        assert!(matches!(
            error,
            RemoteSeatError::CleanupFailed { errors } if errors.len() == 1
        ));

        let mut session = FakeSession {
            fail_seat: true,
            fail_close_new: true,
            ..FakeSession::default()
        };
        let error = prepare_remote_seat_identities(&mut session, Vec::new(), 1).unwrap_err();
        assert_eq!(error.to_string(), "Remote seat reservation failed");
        assert!(matches!(error, RemoteSeatError::ReservationFailed { .. }));
    }

    #[test]
    fn publish_failures_propagate() {
        let mut session = FakeSession {
            fail_validate: true,
            ..FakeSession::default()
        };
        let mut prepared = prepare_remote_seat_identities(&mut session, Vec::new(), 1).unwrap();
        assert_eq!(
            prepared.publish().unwrap_err(),
            RemoteSeatError::Session("bad seats".to_string())
        );
    }
}
