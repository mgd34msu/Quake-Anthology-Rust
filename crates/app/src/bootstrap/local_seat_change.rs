//! Prepared local-seat join/drop changes with commit boundaries.
//!
//! Donor provenance: `/home/buzzkill/Projects/quake-typescript/src/app/bootstrap/local-seat-change.ts`
//! (`prepareLocalSeatChange`). `qa-world`'s `EngineSession` does not expose
//! client/seat preparation yet, so the session surface is the local
//! [`LocalSeatSession`] trait; the slot picking, phase machine, and error
//! texts are a direct port.

use qa_core::identity::SeatId;
use std::rc::Rc;
use thiserror::Error;

/// Failure of a local-seat change.
#[derive(Debug, Error)]
pub enum LocalSeatError {
    /// Invalid request, capacity, or phase.
    #[error("{0}")]
    Invalid(String),
    /// Seat preparation failed and client cleanup failed too.
    #[error("Local player preparation and cleanup failed: {prepare}; cleanup: {cleanup}")]
    PreparationCleanup {
        /// Preparation failure.
        prepare: String,
        /// Cleanup failure.
        cleanup: String,
    },
}

/// One local seat identity: client plus seat handles.
#[derive(Debug)]
pub struct LocalSeatIdentity<C, S> {
    /// Client handle.
    pub client: C,
    /// Seat handle.
    pub seat: S,
}

/// Join or drop request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LocalSeatRequest {
    /// Join a local seat.
    Join,
    /// Drop a local seat.
    Drop {
        /// Seat to drop.
        seat: SeatId,
    },
}

/// Seat and client capacity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SeatCapacity {
    /// Local seat slots.
    pub local_seats: u32,
    /// Server client slots.
    pub clients: u32,
}

/// Client/seat delta of one prepared change.
#[derive(Debug)]
pub struct SeatDelta<'a, C, S> {
    /// Added client, if joining.
    pub added_client: Option<&'a C>,
    /// Removed client, if dropping.
    pub removed_client: Option<&'a C>,
    /// Added seat, if joining.
    pub added_seat: Option<&'a S>,
    /// Removed seat, if dropping.
    pub removed_seat: Option<&'a S>,
}

/// Session surface for local-seat preparation.
pub trait LocalSeatSession {
    /// Client handle.
    type Client;
    /// Seat handle.
    type Seat;
    /// Seat presentation.
    type Presentation: Clone;
    /// Retired resource.
    type Resource;
    /// Whether the world is active (present and open).
    fn world_active(&self) -> bool;
    /// World generation for preparation-race detection.
    fn world_generation(&self) -> u64;
    /// Whether a client slot is occupied.
    fn client_occupied(&self, slot: u32) -> bool;
    /// Seat id of a seat handle.
    fn seat_id(&self, seat: &Self::Seat) -> SeatId;
    /// Seat index of a seat handle.
    fn seat_index(&self, seat: &Self::Seat) -> u32;
    /// Reserve a client slot.
    fn prepare_client(&mut self, slot: u32) -> Result<Self::Client, LocalSeatError>;
    /// Reserve a seat on a client.
    fn prepare_seat(&mut self, index: u32, client: &Self::Client) -> Result<Self::Seat, LocalSeatError>;
    /// Release a reserved client.
    fn close_client(&mut self, client: Self::Client) -> Result<(), LocalSeatError>;
    /// Validate the change against source admission.
    fn validate_local_seats(
        &mut self,
        presentations: &[Self::Presentation],
        delta: SeatDelta<'_, Self::Client, Self::Seat>,
    ) -> Result<(), LocalSeatError>;
    /// Publish the change, returning the retired resource.
    fn publish_local_seats(
        &mut self,
        presentations: Vec<Self::Presentation>,
        delta: SeatDelta<'_, Self::Client, Self::Seat>,
    ) -> Self::Resource;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ChangePhase {
    Prepared,
    Published,
    Discarded,
}

impl std::fmt::Display for ChangePhase {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ChangePhase::Prepared => f.write_str("prepared"),
            ChangePhase::Published => f.write_str("published"),
            ChangePhase::Discarded => f.write_str("discarded"),
        }
    }
}

/// A shared local-seat identity handle.
pub type SharedLocalSeat<C, S> = Rc<LocalSeatIdentity<C, S>>;

/// A prepared local-seat change awaiting validation and publication.
#[derive(Debug)]
pub struct PreparedLocalSeatChange<C, S> {
    added: Option<SharedLocalSeat<C, S>>,
    removed: Option<SharedLocalSeat<C, S>>,
    next: Vec<SharedLocalSeat<C, S>>,
    phase: ChangePhase,
    generation: u64,
}

impl<C, S> PreparedLocalSeatChange<C, S> {
    /// Added identity, if joining.
    #[must_use]
    pub fn added(&self) -> Option<&SharedLocalSeat<C, S>> {
        self.added.as_ref()
    }

    /// Removed identity, if dropping.
    #[must_use]
    pub fn removed(&self) -> Option<&SharedLocalSeat<C, S>> {
        self.removed.as_ref()
    }

    /// Seat list after the change, sorted by seat index.
    #[must_use]
    pub fn next(&self) -> &[SharedLocalSeat<C, S>] {
        &self.next
    }

    /// Whether the change was published.
    #[must_use]
    pub fn published(&self) -> bool {
        self.phase == ChangePhase::Published
    }

    /// Take the added identity out of the change.
    pub fn take_added(&mut self) -> Option<SharedLocalSeat<C, S>> {
        self.added.take()
    }

    /// Take the removed identity out of the change.
    pub fn take_removed(&mut self) -> Option<SharedLocalSeat<C, S>> {
        self.removed.take()
    }

    /// Take the post-change seat list out of the change.
    pub fn take_next(&mut self) -> Vec<SharedLocalSeat<C, S>> {
        std::mem::take(&mut self.next)
    }

    fn delta(&self) -> SeatDelta<'_, C, S> {
        SeatDelta {
            added_client: self.added.as_ref().map(|added| &added.client),
            removed_client: self.removed.as_ref().map(|removed| &removed.client),
            added_seat: self.added.as_ref().map(|added| &added.seat),
            removed_seat: self.removed.as_ref().map(|removed| &removed.seat),
        }
    }

    /// Validate the change against source admission.
    pub fn validate<Session: LocalSeatSession<Client = C, Seat = S>>(
        &self,
        session: &mut Session,
        presentations: &[Session::Presentation],
    ) -> Result<(), LocalSeatError> {
        if self.phase != ChangePhase::Prepared {
            return Err(LocalSeatError::Invalid(format!("Local seat change is {}", self.phase)));
        }
        if !session.world_active() || session.world_generation() != self.generation {
            return Err(LocalSeatError::Invalid(
                "Local player world changed during preparation".to_owned(),
            ));
        }
        session.validate_local_seats(presentations, self.delta())
    }

    /// Validate and publish the change, returning the retired resource.
    pub fn publish<Session: LocalSeatSession<Client = C, Seat = S>>(
        &mut self,
        session: &mut Session,
        presentations: Vec<Session::Presentation>,
    ) -> Result<Session::Resource, LocalSeatError> {
        self.validate(session, &presentations)?;
        let retired = session.publish_local_seats(presentations, self.delta());
        self.phase = ChangePhase::Published;
        Ok(retired)
    }

    /// Discard the change, releasing a reserved client.
    pub fn discard<Session: LocalSeatSession<Client = C, Seat = S>>(
        &mut self,
        session: &mut Session,
    ) -> Result<(), LocalSeatError> {
        if self.phase != ChangePhase::Prepared {
            return Ok(());
        }
        self.phase = ChangePhase::Discarded;
        if let Some(added) = self.added.take() {
            self.next.retain(|local| !Rc::ptr_eq(local, &added));
            match Rc::try_unwrap(added) {
                Ok(identity) => session.close_client(identity.client)?,
                Err(_) => {
                    return Err(LocalSeatError::Invalid("Local seat change is still shared".to_owned()));
                }
            }
        }
        Ok(())
    }
}

/// Reserve identities before preparing input/assets. Source admission belongs to the caller's commit boundary.
pub fn prepare_local_seat_change<Session: LocalSeatSession>(
    session: &mut Session,
    current: Vec<LocalSeatIdentity<Session::Client, Session::Seat>>,
    request: &LocalSeatRequest,
    capacity: &SeatCapacity,
) -> Result<PreparedLocalSeatChange<Session::Client, Session::Seat>, LocalSeatError> {
    if !session.world_active() {
        return Err(LocalSeatError::Invalid(
            "Local player changes require an active world".to_owned(),
        ));
    }
    let generation = session.world_generation();
    let mut added = None;
    let mut remaining = current;
    let mut removed = None;
    match request {
        LocalSeatRequest::Join => {
            let mut index = 0;
            while remaining.iter().any(|local| session.seat_index(&local.seat) == index) {
                index += 1;
            }
            if index >= capacity.local_seats {
                return Err(LocalSeatError::Invalid(
                    "All local player slots are occupied".to_owned(),
                ));
            }
            let mut slot = 0;
            while slot < capacity.clients && session.client_occupied(slot) {
                slot += 1;
            }
            if slot >= capacity.clients {
                return Err(LocalSeatError::Invalid(
                    "The server has no free player slots".to_owned(),
                ));
            }
            let client = session.prepare_client(slot)?;
            match session.prepare_seat(index, &client) {
                Ok(seat) => {
                    added = Some(LocalSeatIdentity { client, seat });
                }
                Err(error) => match session.close_client(client) {
                    Ok(()) => return Err(error),
                    Err(cleanup) => {
                        return Err(LocalSeatError::PreparationCleanup {
                            prepare: error.to_string(),
                            cleanup: cleanup.to_string(),
                        });
                    }
                },
            }
        }
        LocalSeatRequest::Drop { seat } => {
            let position = remaining.iter().position(|local| session.seat_id(&local.seat) == *seat);
            match position {
                Some(position) => {
                    removed = Some(remaining.remove(position));
                }
                None => {
                    return Err(LocalSeatError::Invalid("Local player is no longer active".to_owned()));
                }
            }
        }
    }
    let mut next: Vec<SharedLocalSeat<Session::Client, Session::Seat>> = remaining.into_iter().map(Rc::new).collect();
    let mut shared_added = None;
    if let Some(identity) = added {
        let shared = Rc::new(identity);
        next.push(Rc::clone(&shared));
        shared_added = Some(shared);
    }
    next.sort_by_key(|local| session.seat_index(&local.seat));
    let removed = removed.map(Rc::new);
    Ok(PreparedLocalSeatChange {
        added: shared_added,
        removed,
        next,
        phase: ChangePhase::Prepared,
        generation,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_core::identity::IdentityOwner;

    #[derive(Debug)]
    struct FakeClient {
        slot: u32,
        closed: bool,
    }

    #[derive(Debug)]
    struct FakeSeat {
        id: SeatId,
        index: u32,
    }

    struct FakeSession {
        owner: IdentityOwner,
        active: bool,
        generation: u64,
        occupied: Vec<u32>,
        fail_seat: bool,
        fail_close: bool,
        validated: usize,
        published: usize,
    }

    impl FakeSession {
        fn new() -> Self {
            Self {
                owner: IdentityOwner::create("seats").expect("owner"),
                active: true,
                generation: 3,
                occupied: Vec::new(),
                fail_seat: false,
                fail_close: false,
                validated: 0,
                published: 0,
            }
        }
    }

    impl LocalSeatSession for FakeSession {
        type Client = FakeClient;
        type Seat = FakeSeat;
        type Presentation = String;
        type Resource = String;

        fn world_active(&self) -> bool {
            self.active
        }

        fn world_generation(&self) -> u64 {
            self.generation
        }

        fn client_occupied(&self, slot: u32) -> bool {
            self.occupied.contains(&slot)
        }

        fn seat_id(&self, seat: &Self::Seat) -> SeatId {
            seat.id.clone()
        }

        fn seat_index(&self, seat: &Self::Seat) -> u32 {
            seat.index
        }

        fn prepare_client(&mut self, slot: u32) -> Result<Self::Client, LocalSeatError> {
            self.occupied.push(slot);
            Ok(FakeClient { slot, closed: false })
        }

        fn prepare_seat(&mut self, index: u32, _client: &Self::Client) -> Result<Self::Seat, LocalSeatError> {
            if self.fail_seat {
                return Err(LocalSeatError::Invalid("seat denied".to_owned()));
            }
            Ok(FakeSeat {
                id: self.owner.seat(index),
                index,
            })
        }

        fn close_client(&mut self, mut client: Self::Client) -> Result<(), LocalSeatError> {
            client.closed = true;
            self.occupied.retain(|slot| *slot != client.slot);
            if self.fail_close {
                return Err(LocalSeatError::Invalid("close denied".to_owned()));
            }
            Ok(())
        }

        fn validate_local_seats(
            &mut self,
            _presentations: &[Self::Presentation],
            _delta: SeatDelta<'_, Self::Client, Self::Seat>,
        ) -> Result<(), LocalSeatError> {
            self.validated += 1;
            Ok(())
        }

        fn publish_local_seats(
            &mut self,
            _presentations: Vec<Self::Presentation>,
            _delta: SeatDelta<'_, Self::Client, Self::Seat>,
        ) -> Self::Resource {
            self.published += 1;
            "retired".to_owned()
        }
    }

    fn capacity() -> SeatCapacity {
        SeatCapacity {
            local_seats: 2,
            clients: 4,
        }
    }

    #[test]
    fn join_publishes_lowest_free_slots() {
        let mut session = FakeSession::new();
        session.occupied.push(0);
        let mut change =
            prepare_local_seat_change(&mut session, Vec::new(), &LocalSeatRequest::Join, &capacity()).expect("prepare");
        assert_eq!(change.added().expect("added").client.slot, 1);
        assert_eq!(change.next().len(), 1);
        assert!(!change.published());
        let retired = change.publish(&mut session, Vec::new()).expect("publish");
        assert_eq!(retired, "retired");
        assert!(change.published());
        assert_eq!(session.validated, 1);
        assert_eq!(session.published, 1);
    }

    #[test]
    fn drop_releases_and_rejects_unknown_seats() {
        let mut session = FakeSession::new();
        let seat = session.owner.seat(0);
        let current = vec![LocalSeatIdentity {
            client: FakeClient { slot: 0, closed: false },
            seat: FakeSeat {
                id: seat.clone(),
                index: 0,
            },
        }];
        let mut change =
            prepare_local_seat_change(&mut session, current, &LocalSeatRequest::Drop { seat }, &capacity())
                .expect("prepare");
        assert!(change.removed().is_some());
        assert!(change.next().is_empty());
        change.publish(&mut session, Vec::new()).expect("publish");
        let unknown = session.owner.seat(1);
        let err = prepare_local_seat_change(
            &mut session,
            Vec::new(),
            &LocalSeatRequest::Drop { seat: unknown },
            &capacity(),
        )
        .expect_err("unknown seat");
        assert_eq!(err.to_string(), "Local player is no longer active");
    }

    #[test]
    fn capacity_world_and_phase_guards_fire() {
        let mut session = FakeSession::new();
        session.active = false;
        let err = prepare_local_seat_change(&mut session, Vec::new(), &LocalSeatRequest::Join, &capacity())
            .expect_err("inactive world");
        assert_eq!(err.to_string(), "Local player changes require an active world");
        session.active = true;
        let tight = SeatCapacity {
            local_seats: 0,
            clients: 4,
        };
        let err = prepare_local_seat_change(&mut session, Vec::new(), &LocalSeatRequest::Join, &tight)
            .expect_err("seat capacity");
        assert_eq!(err.to_string(), "All local player slots are occupied");
        let no_clients = SeatCapacity {
            local_seats: 2,
            clients: 0,
        };
        let err = prepare_local_seat_change(&mut session, Vec::new(), &LocalSeatRequest::Join, &no_clients)
            .expect_err("client capacity");
        assert_eq!(err.to_string(), "The server has no free player slots");
        let mut change =
            prepare_local_seat_change(&mut session, Vec::new(), &LocalSeatRequest::Join, &capacity()).expect("prepare");
        session.generation = 4;
        let err = change.validate(&mut session, &[]).expect_err("world race");
        assert_eq!(err.to_string(), "Local player world changed during preparation");
        session.generation = 3;
        change.discard(&mut session).expect("discard");
        assert!(session.occupied.is_empty());
        let err = change.publish(&mut session, Vec::new()).expect_err("published phase");
        assert_eq!(err.to_string(), "Local seat change is discarded");
    }

    #[test]
    fn seat_failure_combines_with_cleanup_failure() {
        let mut session = FakeSession::new();
        session.fail_seat = true;
        session.fail_close = true;
        let err = prepare_local_seat_change(&mut session, Vec::new(), &LocalSeatRequest::Join, &capacity())
            .expect_err("combined");
        assert!(matches!(err, LocalSeatError::PreparationCleanup { .. }));
    }
}
