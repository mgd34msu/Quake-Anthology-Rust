//! Native per-seat remote channels over admitted session clients.
//!
//! Port of Quake-Anthology-TS `src/app/bootstrap/remote-seats.ts`
//! (`RemoteSeatServices`, `RemoteSeatChannel`, `prepareRemoteSeatChannels`).
//! One native channel per admitted local player; no authority or platform
//! input owner. Session seats, clients, and connections
//! ([`EngineSession`](qa_world::session::EngineSession) surface) arrive as
//! local traits following the [`super::remote_seat_identities`]
//! absorbed-contract pattern; the network and presentation reuse
//! [`super::remote_seat_source`], the frame clock reuses
//! [`super::frame_clock`], and frame-time controls
//! ([`FrameTimeControls`](super::frame_time::FrameTimeControls),
//! `frame-time.ts` port) arrive through [`RemoteFrameTimer`]. Sync port: the
//! donor's async poll/close/prepare become sync calls with the same
//! aggregate cleanup texts.

use qa_core::cmd_buffer::CommandOrigin;
use qa_core::cvar::CvarRegistry;
use qa_core::identity::ClientId;
use qa_core::identity::SeatId;
use qa_core::identity::SessionId;
use thiserror::Error;

use super::frame_clock::PresentationTime;
use super::remote_seat_source::RemoteActorCommand;
use super::remote_seat_source::RemoteFrameTimer;
use super::remote_seat_source::RemoteInboundOutput;
use super::remote_seat_source::RemoteSeatNetwork;
use super::remote_seat_source::RemoteSeatPresentation;
use super::remote_seat_source::RemoteSimulationOutput;

/// Remote seat channel failure.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum RemoteSeatChannelError {
    /// Presentation belongs to another session client.
    #[error("Remote channel belongs to another session client")]
    ForeignClient,
    /// Adopted registry belongs to another session.
    #[error("Remote clock registry belongs to another session")]
    ForeignSession,
    /// Seat or client retired.
    #[error("Remote seat has retired")]
    Retired,
    /// Connection already published.
    #[error("Remote seat connection is already published")]
    AlreadyPublished,
    /// Frame on a closed channel.
    #[error("Remote channel is closed")]
    Closed,
    /// Command from another seat.
    #[error("Remote command belongs to another seat")]
    ForeignSeat,
    /// Command for another admitted actor.
    #[error("Remote command belongs to another admitted actor")]
    ForeignActor,
    /// Command without an active connection.
    #[error("Remote command has no active connection")]
    Inactive,
    /// Snapshot from another session.
    #[error("Remote snapshot belongs to another session")]
    ForeignSnapshot,
    /// Seat count outside one to four.
    #[error("Remote play requires one to four local seats")]
    BadSeatCount,
    /// Seats or clients not distinct and live.
    #[error("Remote channels require distinct live seats and clients")]
    BadSeats,
    /// Channel cleanup failure (donor `AggregateError` member texts).
    #[error("Remote seat channel cleanup failed")]
    CleanupFailed {
        /// Member failure messages.
        errors: Vec<String>,
    },
    /// Construction cleanup failure (donor `AggregateError` member texts).
    #[error("Remote seat construction cleanup failed")]
    ConstructionCleanupFailed {
        /// Member failure messages.
        errors: Vec<String>,
    },
    /// Preparation cleanup failure (donor `AggregateError` member texts).
    #[error("Remote channel preparation failed")]
    PreparationFailed {
        /// Member failure messages.
        errors: Vec<String>,
    },
    /// Backend failure.
    #[error("{0}")]
    Backend(String),
}

/// Absorbed session connection (donor `SessionConnection`).
pub trait RemoteChannelConnection: PartialEq {
    /// Connection kind marker.
    fn kind(&self) -> &str;
}

/// Replacement produced by `replaceConnection` (donor `{ connection, retired }`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplacedRemoteConnection<Connection> {
    /// Replacement connection.
    pub connection: Connection,
    /// Retired connection, if any.
    pub retired: Option<Connection>,
}

/// Absorbed session client (donor `SessionClient` surface).
pub trait RemoteChannelClient {
    /// Connection handle.
    type Connection: RemoteChannelConnection;
    /// Backend failure.
    type Error: std::fmt::Display;
    /// Client id.
    fn id(&self) -> ClientId;
    /// Whether the client is closed.
    fn is_closed(&self) -> bool;
    /// Current connection, if any.
    fn connection(&self) -> Option<&Self::Connection>;
    /// Replace the connection, retiring the current one.
    fn replace_connection(&mut self, kind: &str) -> Result<ReplacedRemoteConnection<Self::Connection>, Self::Error>;
    /// Drop the current connection.
    fn disconnect(&mut self) -> Result<(), Self::Error>;
}

/// Absorbed session seat (donor `SessionSeat` surface).
pub trait RemoteChannelSeat {
    /// Client handle.
    type Client: RemoteChannelClient;
    /// Seat id.
    fn id(&self) -> SeatId;
    /// Owning session.
    fn session(&self) -> SessionId;
    /// Seat client.
    fn client(&self) -> &Self::Client;
    /// Mutable seat client.
    fn client_mut(&mut self) -> &mut Self::Client;
    /// Whether the seat is closed.
    fn is_closed(&self) -> bool;
    /// Deliver seat-audience events.
    fn receive(&mut self, events: Vec<super::remote_seat_source::RemoteSeatEvent>);
}

/// Channel services (donor `RemoteSeatServices`).
pub struct RemoteSeatServices<Network, Presentation> {
    /// Client network.
    pub network: Network,
    /// Remote presentation.
    pub remote: Presentation,
    /// Source cvars.
    pub cvars: CvarRegistry,
}

/// One native channel per admitted local player.
pub struct RemoteSeatChannel<Seat, Network, Presentation, Timer>
where
    Seat: RemoteChannelSeat,
{
    seat: Seat,
    clock: PresentationTime,
    services: RemoteSeatServices<Network, Presentation>,
    timer: Timer,
    source_session: SessionId,
    closed: bool,
    connection: Option<<Seat::Client as RemoteChannelClient>::Connection>,
    source_elapsed_ms: f64,
    source_frame_elapsed_ms: f64,
    frame_number: u64,
}

impl<Seat, Network, Presentation, Timer> RemoteSeatChannel<Seat, Network, Presentation, Timer>
where
    Seat: RemoteChannelSeat,
    Network: RemoteSeatNetwork,
    Presentation: RemoteSeatPresentation,
    Timer: RemoteFrameTimer,
{
    /// Open a channel over an admitted seat.
    pub fn open(
        seat: Seat,
        clock: PresentationTime,
        services: RemoteSeatServices<Network, Presentation>,
        timer: Timer,
    ) -> Result<Self, RemoteSeatChannelError> {
        if services.remote.client_id() != seat.client().id() {
            return Err(RemoteSeatChannelError::ForeignClient);
        }
        let source_session = seat.session();
        Ok(Self {
            seat,
            clock,
            services,
            timer,
            source_session,
            closed: false,
            connection: None,
            source_elapsed_ms: 0.0,
            source_frame_elapsed_ms: 0.0,
            frame_number: 0,
        })
    }

    /// Admitted seat.
    #[must_use]
    pub fn seat(&self) -> &Seat {
        &self.seat
    }

    /// Presentation clock.
    #[must_use]
    pub fn clock(&self) -> &PresentationTime {
        &self.clock
    }

    /// Accumulated source milliseconds.
    #[must_use]
    pub fn elapsed_milliseconds(&self) -> f64 {
        self.source_elapsed_ms
    }

    /// Current frame milliseconds.
    #[must_use]
    pub fn frame_milliseconds(&self) -> f64 {
        self.source_frame_elapsed_ms
    }

    /// Committed frame count.
    #[must_use]
    pub fn frames(&self) -> u64 {
        self.frame_number
    }

    /// Whether the channel can submit commands.
    #[must_use]
    pub fn active(&self) -> bool {
        !self.closed && !self.seat.is_closed() && self.services.network.phase() == "active"
    }

    /// Adopt a new clock registry from the same session.
    pub fn adopt_cvars(&mut self, cvars: CvarRegistry, session: &SessionId) -> Result<(), RemoteSeatChannelError> {
        if session != &self.source_session {
            return Err(RemoteSeatChannelError::ForeignSession);
        }
        self.services.cvars = cvars;
        Ok(())
    }

    /// Publish the remote connection, returning the retired one.
    pub fn publish_connection(
        &mut self,
    ) -> Result<Option<<Seat::Client as RemoteChannelClient>::Connection>, RemoteSeatChannelError> {
        if self.closed || self.seat.is_closed() || self.seat.client().is_closed() {
            return Err(RemoteSeatChannelError::Retired);
        }
        if self.connection.is_some() {
            return Err(RemoteSeatChannelError::AlreadyPublished);
        }
        let replacement = self
            .seat
            .client_mut()
            .replace_connection("remote")
            .map_err(|error| RemoteSeatChannelError::Backend(error.to_string()))?;
        self.connection = Some(replacement.connection);
        Ok(replacement.retired)
    }

    /// Begin a frame, advancing source and presentation clocks.
    pub fn begin_frame(&mut self, wall_now_ms: f64, wall_elapsed_ms: f64) -> Result<(), RemoteSeatChannelError> {
        if self.closed {
            return Err(RemoteSeatChannelError::Closed);
        }
        let frame =
            self.timer
                .source_frame_milliseconds(self.services.cvars.dialect(), wall_elapsed_ms, &self.services.cvars);
        self.source_frame_elapsed_ms = frame;
        self.source_elapsed_ms += frame;
        self.clock.advance(wall_now_ms, wall_elapsed_ms, frame);
        self.frame_number += 1;
        Ok(())
    }

    /// Poll the network unless closed.
    pub fn poll(&mut self, wall_now_ms: f64) {
        if self.closed {
            return;
        }
        self.services.network.poll(wall_now_ms);
    }

    /// Submit one actor command.
    pub fn submit(&mut self, command: &RemoteActorCommand, wall_now_ms: f64) -> Result<(), RemoteSeatChannelError> {
        let belongs = match &command.source {
            CommandOrigin::LocalSeat { seat, client } => seat == &self.seat.id() && client == &self.seat.client().id(),
            _ => false,
        };
        if !belongs {
            return Err(RemoteSeatChannelError::ForeignSeat);
        }
        let admitted = self
            .services
            .remote
            .player()
            .is_some_and(|player| player.actor == command.actor);
        if !admitted {
            return Err(RemoteSeatChannelError::ForeignActor);
        }
        if !self.active() {
            return Err(RemoteSeatChannelError::Inactive);
        }
        let frame = if self.services.network.timed_submission() {
            self.source_frame_elapsed_ms
        } else {
            0.0
        };
        self.services
            .network
            .submit(std::slice::from_ref(command), wall_now_ms, frame);
        Ok(())
    }

    /// Filter inbound output to this seat and deliver it.
    pub fn receive(&mut self, output: &RemoteInboundOutput) -> Result<RemoteSimulationOutput, RemoteSeatChannelError> {
        if output.snapshot.session != self.source_session {
            return Err(RemoteSeatChannelError::ForeignSnapshot);
        }
        let events: Vec<_> = output
            .events
            .iter()
            .filter(|event| event.targets_seat(&self.seat.id(), &self.seat.client().id()))
            .map(|event| event.for_seat(&self.seat.id()))
            .collect();
        self.seat.receive(events.clone());
        Ok(RemoteSimulationOutput {
            snapshot: output.snapshot.clone(),
            events,
        })
    }

    /// Close the channel and drop a still-current published connection.
    pub fn close(&mut self) -> Result<(), RemoteSeatChannelError> {
        if self.closed {
            return Ok(());
        }
        self.closed = true;
        let mut errors = Vec::new();
        if let Err(error) = self.services.network.close() {
            errors.push(error);
        }
        if let Some(connection) = self.connection.take() {
            let current = self.seat.client().connection().is_some_and(|live| *live == connection);
            if current {
                if let Err(error) = self.seat.client_mut().disconnect() {
                    errors.push(error.to_string());
                }
            }
        }
        if errors.is_empty() {
            Ok(())
        } else {
            Err(RemoteSeatChannelError::CleanupFailed { errors })
        }
    }
}

/// Prepare one channel per seat; candidate resources retire on failure.
pub fn prepare_remote_seat_channels<Seat, Network, Presentation, Timer>(
    seats: Vec<Seat>,
    mut create: impl FnMut(
        &Seat,
        PresentationTime,
    ) -> Result<
        (RemoteSeatServices<Network, Presentation>, PresentationTime, Timer),
        RemoteSeatChannelError,
    >,
) -> Result<Vec<RemoteSeatChannel<Seat, Network, Presentation, Timer>>, RemoteSeatChannelError>
where
    Seat: RemoteChannelSeat,
    Network: RemoteSeatNetwork,
    Presentation: RemoteSeatPresentation,
    Timer: RemoteFrameTimer,
{
    if seats.is_empty() || seats.len() > 4 {
        return Err(RemoteSeatChannelError::BadSeatCount);
    }
    for (index, seat) in seats.iter().enumerate() {
        let duplicate = seats[..index]
            .iter()
            .any(|previous: &Seat| previous.id() == seat.id() || previous.client().id() == seat.client().id());
        if seat.is_closed() || seat.client().is_closed() || duplicate {
            return Err(RemoteSeatChannelError::BadSeats);
        }
    }
    let mut channels = Vec::with_capacity(seats.len());
    let mut pending = seats;
    let mut first_error: Option<RemoteSeatChannelError> = None;
    for seat in pending.drain(..) {
        if first_error.is_some() {
            break;
        }
        let clock = PresentationTime::new();
        match create(&seat, clock) {
            Ok((mut services, clock, timer)) => {
                // Pre-check the client match so a failed construction still
                // closes its candidate network (donor construction cleanup).
                if services.remote.client_id() != seat.client().id() {
                    let error = RemoteSeatChannelError::ForeignClient;
                    if let Err(cleanup) = services.network.close() {
                        first_error = Some(RemoteSeatChannelError::ConstructionCleanupFailed {
                            errors: vec![error.to_string(), cleanup],
                        });
                    } else {
                        first_error = Some(error);
                    }
                    break;
                }
                match RemoteSeatChannel::open(seat, clock, services, timer) {
                    Ok(channel) => channels.push(channel),
                    Err(error) => {
                        first_error = Some(error);
                        break;
                    }
                }
            }
            Err(error) => {
                first_error = Some(error);
                break;
            }
        }
    }
    if let Some(error) = first_error {
        let mut errors = vec![error.to_string()];
        for mut channel in channels.into_iter().rev() {
            if let Err(cleanup) = channel.close() {
                errors.push(cleanup.to_string());
            }
        }
        if errors.len() > 1 {
            return Err(RemoteSeatChannelError::PreparationFailed { errors });
        }
        return Err(error);
    }
    Ok(channels)
}

#[cfg(test)]
mod tests {
    use qa_core::cmd::Dialect;
    use qa_core::identity::IdentityOwner;

    use super::super::remote_seat_source::RemoteActorId;
    use super::super::remote_seat_source::RemoteInboundEvent;
    use super::super::remote_seat_source::RemotePlayer;
    use super::super::remote_seat_source::RemoteSceneHandle;
    use super::super::remote_seat_source::RemoteSeatEvent;
    use super::super::remote_seat_source::RemoteSeatFamily;
    use super::super::remote_seat_source::RemoteSnapshot;
    use super::*;

    #[derive(Debug, Clone, PartialEq)]
    struct FakeConnection {
        kind: String,
    }

    impl RemoteChannelConnection for FakeConnection {
        fn kind(&self) -> &str {
            &self.kind
        }
    }

    #[derive(Debug)]
    struct FakeClient {
        id: ClientId,
        closed: bool,
        connection: Option<FakeConnection>,
        fail_disconnect: bool,
    }

    impl RemoteChannelClient for FakeClient {
        type Connection = FakeConnection;
        type Error = String;

        fn id(&self) -> ClientId {
            self.id.clone()
        }

        fn is_closed(&self) -> bool {
            self.closed
        }

        fn connection(&self) -> Option<&FakeConnection> {
            self.connection.as_ref()
        }

        fn replace_connection(&mut self, kind: &str) -> Result<ReplacedRemoteConnection<FakeConnection>, String> {
            let retired = self.connection.take();
            let connection = FakeConnection { kind: kind.to_string() };
            self.connection = Some(connection.clone());
            Ok(ReplacedRemoteConnection { connection, retired })
        }

        fn disconnect(&mut self) -> Result<(), String> {
            if self.fail_disconnect {
                return Err("disconnect failed".to_string());
            }
            self.connection = None;
            Ok(())
        }
    }

    #[derive(Debug)]
    struct FakeSeat {
        id: SeatId,
        session: SessionId,
        client: FakeClient,
        closed: bool,
        received: Vec<RemoteSeatEvent>,
    }

    impl RemoteChannelSeat for FakeSeat {
        type Client = FakeClient;

        fn id(&self) -> SeatId {
            self.id.clone()
        }

        fn session(&self) -> SessionId {
            self.session.clone()
        }

        fn client(&self) -> &FakeClient {
            &self.client
        }

        fn client_mut(&mut self) -> &mut FakeClient {
            &mut self.client
        }

        fn is_closed(&self) -> bool {
            self.closed
        }

        fn receive(&mut self, events: Vec<RemoteSeatEvent>) {
            self.received.extend(events);
        }
    }

    struct FakeNetwork {
        phase: String,
        timed: bool,
        submitted: Vec<(usize, f64, f64)>,
        polls: usize,
        fail_close: bool,
    }

    impl RemoteSeatNetwork for FakeNetwork {
        fn phase(&self) -> String {
            self.phase.clone()
        }

        fn timed_submission(&self) -> bool {
            self.timed
        }

        fn poll(&mut self, _wall_now_ms: f64) {
            self.polls += 1;
        }

        fn submit(&mut self, commands: &[RemoteActorCommand], wall_now_ms: f64, frame_elapsed_ms: f64) {
            self.submitted.push((commands.len(), wall_now_ms, frame_elapsed_ms));
        }

        fn command(&mut self, _text: &str) {}

        fn close(&mut self) -> Result<(), String> {
            if self.fail_close {
                return Err("network close failed".to_string());
            }
            Ok(())
        }
    }

    struct FakePresentation {
        client: ClientId,
        player: Option<RemotePlayer>,
    }

    impl RemoteSeatPresentation for FakePresentation {
        fn client_id(&self) -> ClientId {
            self.client.clone()
        }

        fn player(&self) -> Option<RemotePlayer> {
            self.player
        }

        fn is_player(&self, actor: &RemoteActorId) -> bool {
            self.player.is_some_and(|player| player.actor == *actor)
        }

        fn scene(&self) -> RemoteSceneHandle {
            RemoteSceneHandle { generation: 0 }
        }

        fn output_frame(&self) -> Option<u64> {
            None
        }

        fn family(&self) -> RemoteSeatFamily {
            RemoteSeatFamily::Unified
        }
    }

    struct FixedTimer(f64);

    impl RemoteFrameTimer for FixedTimer {
        fn source_frame_milliseconds(&self, _dialect: Dialect, _wall_elapsed_ms: f64, _cvars: &CvarRegistry) -> f64 {
            self.0
        }
    }

    fn owner() -> IdentityOwner {
        IdentityOwner::create("remote-seats-test").unwrap()
    }

    fn seat(owner: &IdentityOwner, index: u32) -> FakeSeat {
        FakeSeat {
            id: owner.seat(index),
            session: owner.session().clone(),
            client: FakeClient {
                id: owner.client(index, 0),
                closed: false,
                connection: None,
                fail_disconnect: false,
            },
            closed: false,
            received: Vec::new(),
        }
    }

    fn services(
        owner: &IdentityOwner,
        index: u32,
        player: Option<RemotePlayer>,
        timed: bool,
    ) -> RemoteSeatServices<FakeNetwork, FakePresentation> {
        RemoteSeatServices {
            network: FakeNetwork {
                phase: "active".to_string(),
                timed,
                submitted: Vec::new(),
                polls: 0,
                fail_close: false,
            },
            remote: FakePresentation {
                client: owner.client(index, 0),
                player,
            },
            cvars: CvarRegistry::new(Dialect::Q3),
        }
    }

    fn actor() -> RemoteActorId {
        RemoteActorId {
            session: 1,
            slot: 2,
            generation: 3,
        }
    }

    #[test]
    fn open_rejects_foreign_client() {
        let owner = owner();
        let seat = seat(&owner, 0);
        let bad = services(&owner, 1, None, false);
        assert_eq!(
            RemoteSeatChannel::open(seat, PresentationTime::new(), bad, FixedTimer(16.0))
                .err()
                .unwrap(),
            RemoteSeatChannelError::ForeignClient
        );
    }

    #[test]
    fn frames_advance_clocks_and_counts() {
        let owner = owner();
        let mut channel = RemoteSeatChannel::open(
            seat(&owner, 0),
            PresentationTime::new(),
            services(&owner, 0, None, false),
            FixedTimer(16.0),
        )
        .unwrap();
        assert!(channel.active());
        channel.begin_frame(1000.0, 16.0).unwrap();
        channel.begin_frame(1016.0, 16.0).unwrap();
        assert_eq!(channel.frames(), 2);
        assert_eq!(channel.frame_milliseconds(), 16.0);
        assert_eq!(channel.elapsed_milliseconds(), 32.0);
        assert_eq!(channel.clock().milliseconds().unwrap(), 1016.0);
        channel.poll(1016.0);
    }

    #[test]
    fn publish_connection_replaces_and_close_disconnects() {
        let owner = owner();
        let mut channel = RemoteSeatChannel::open(
            seat(&owner, 0),
            PresentationTime::new(),
            services(&owner, 0, None, false),
            FixedTimer(16.0),
        )
        .unwrap();
        assert_eq!(channel.publish_connection().unwrap(), None);
        assert_eq!(
            channel.publish_connection(),
            Err(RemoteSeatChannelError::AlreadyPublished)
        );
        channel.close().unwrap();
        assert!(!channel.active());
        assert_eq!(channel.begin_frame(0.0, 0.0), Err(RemoteSeatChannelError::Closed));
        channel.poll(0.0);
        channel.close().unwrap();
    }

    #[test]
    fn submit_validates_seat_actor_and_phase() {
        let owner = owner();
        let mut channel = RemoteSeatChannel::open(
            seat(&owner, 0),
            PresentationTime::new(),
            services(&owner, 0, Some(RemotePlayer { actor: actor() }), true),
            FixedTimer(16.0),
        )
        .unwrap();
        channel.begin_frame(1000.0, 16.0).unwrap();
        let local = CommandOrigin::LocalSeat {
            seat: owner.seat(0),
            client: owner.client(0, 0),
        };
        channel
            .submit(
                &RemoteActorCommand {
                    source: local.clone(),
                    actor: actor(),
                },
                1000.0,
            )
            .unwrap();
        let foreign = RemoteActorCommand {
            source: CommandOrigin::LocalSeat {
                seat: owner.seat(1),
                client: owner.client(1, 0),
            },
            actor: actor(),
        };
        assert_eq!(
            channel.submit(&foreign, 1000.0),
            Err(RemoteSeatChannelError::ForeignSeat)
        );
        let stranger = RemoteActorCommand {
            source: local,
            actor: RemoteActorId {
                session: 9,
                slot: 9,
                generation: 9,
            },
        };
        assert_eq!(
            channel.submit(&stranger, 1000.0),
            Err(RemoteSeatChannelError::ForeignActor)
        );
    }

    #[test]
    fn receive_filters_and_retargets_events() {
        let owner = owner();
        let mut channel = RemoteSeatChannel::open(
            seat(&owner, 0),
            PresentationTime::new(),
            services(&owner, 0, None, false),
            FixedTimer(16.0),
        )
        .unwrap();
        let output = RemoteInboundOutput {
            snapshot: RemoteSnapshot {
                session: owner.session().clone(),
                frame: 3,
            },
            events: vec![
                RemoteInboundEvent {
                    name: "mine".to_string(),
                    seat: Some(owner.seat(0)),
                    client: Some(owner.client(0, 0)),
                    broadcast: false,
                },
                RemoteInboundEvent {
                    name: "theirs".to_string(),
                    seat: Some(owner.seat(1)),
                    client: Some(owner.client(1, 0)),
                    broadcast: false,
                },
            ],
        };
        let filtered = channel.receive(&output).unwrap();
        assert_eq!(filtered.events.len(), 1);
        assert_eq!(filtered.events[0].seat, owner.seat(0));
        assert_eq!(channel.seat().received.len(), 1);
        let foreign = RemoteInboundOutput {
            snapshot: RemoteSnapshot {
                session: IdentityOwner::create("foreign").unwrap().session().clone(),
                frame: 4,
            },
            events: Vec::new(),
        };
        assert_eq!(channel.receive(&foreign), Err(RemoteSeatChannelError::ForeignSnapshot));
    }

    #[test]
    fn prepare_validates_and_rolls_back() {
        let owner = owner();
        assert_eq!(
            prepare_remote_seat_channels(Vec::new(), |_: &FakeSeat, clock| {
                Ok((services(&owner, 0, None, false), clock, FixedTimer(16.0)))
            })
            .err()
            .unwrap(),
            RemoteSeatChannelError::BadSeatCount
        );
        let duplicate = vec![seat(&owner, 0), seat(&owner, 0)];
        assert_eq!(
            prepare_remote_seat_channels(duplicate, |seat: &FakeSeat, clock| {
                let index = seat.id.index();
                Ok((services(&owner, index, None, false), clock, FixedTimer(16.0)))
            })
            .err()
            .unwrap(),
            RemoteSeatChannelError::BadSeats
        );
        let seats = vec![seat(&owner, 0), seat(&owner, 1)];
        let channels = prepare_remote_seat_channels(seats, |seat: &FakeSeat, clock| {
            let index = seat.id.index();
            Ok((services(&owner, index, None, false), clock, FixedTimer(16.0)))
        })
        .unwrap();
        assert_eq!(channels.len(), 2);

        let seats = vec![seat(&owner, 0), seat(&owner, 1)];
        let error = prepare_remote_seat_channels(seats, |seat: &FakeSeat, clock| {
            if seat.id.index() == 1 {
                return Err(RemoteSeatChannelError::Backend("create failed".to_string()));
            }
            Ok((services(&owner, 0, None, false), clock, FixedTimer(16.0)))
        })
        .err()
        .unwrap();
        assert_eq!(error, RemoteSeatChannelError::Backend("create failed".to_string()));
    }
}
