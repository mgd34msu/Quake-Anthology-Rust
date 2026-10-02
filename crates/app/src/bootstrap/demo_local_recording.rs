//! Local demo recording borrowing the live source host.
//!
//! Port of donor `/home/buzzkill/Projects/quake-typescript/src/app/bootstrap/demo-local-recording.ts`
//! (`LocalDemoRecording`). Hosts arrive behind per-kind traits because the
//! network host lanes are unported; the Quake III path encodes through the
//! ported server-message codec while the Quake I/II/World paths take
//! host-encoded bytes (their ported codecs diverged or are missing). The port
//! is synchronous because the recording sink is synchronous. Worlds are
//! compared by host-issued generation tokens, and seed identities by debug
//! text instead of JSON.

use std::cell::RefCell;
use std::rc::Rc;

use qa_core::identity::ActorId;
use qa_core::math::Vec3;
use qa_net::demo::QwDemoRecord;
use qa_net::q3_net::{
    encode_server_message, q3_configstring_commands, Gamestate, GamestateEntry, Q3EntityState, Q3PlayerState,
    Q3Product, ServerMessageContext, ServerOperation, Snapshot, SnapshotValidity,
};
use qa_world::session::SimulationOutput;
use thiserror::Error;

use super::demo_recording::{
    DemoRecordingIdentity, DemoRecordingPacket, DemoRecordingSeed, DemoRecordingSink, Q1DemoProtocol,
    Q2ProtocolIdentity,
};

/// Local recording failure.
#[derive(Debug, Error)]
pub enum LocalRecordingError {
    /// Source wire unsupported.
    #[error("{0}")]
    Admission(String),
    /// QuakeWorld signon missing.
    #[error("QW source has no read-only recording signon")]
    MissingQwSignon,
    /// QWD message too large.
    #[error("QWD message exceeds native packet capacity")]
    QwdCapacity,
    /// Attach without a prepared seed, or double attach.
    #[error("Local recording requires an unattached prepared seed")]
    SeedRequired,
    /// Publish without a seed.
    #[error("Local recording lost its seed")]
    LostSeed,
    /// Reseed changed protocol.
    #[error("Local recording source protocol changed")]
    ProtocolChanged,
    /// Kind changed without a world boundary.
    #[error("Local recording source changed without a world boundary")]
    SourceChanged,
    /// Sink failure.
    #[error(transparent)]
    Sink(#[from] super::demo_recording::DemoRecordingError),
    /// Server message failure.
    #[error(transparent)]
    ServerMessage(#[from] qa_net::q3_net::Q3NetError),
}

/// Source wire admission (`supportsSourceWire`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WireAdmission {
    /// Supported.
    Supported,
    /// Unsupported with reasons.
    Unsupported {
        /// Reasons.
        reasons: Vec<String>,
    },
}

/// Source wire admission.
pub trait RecordingAdmission {
    /// Check source wire support.
    fn supports_source_wire(&self) -> WireAdmission;
}

/// Recording player identity.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RecordingPlayer {
    /// Player actor.
    pub actor: ActorId,
    /// Source entity number.
    pub source_entity: i32,
}

/// Quake III snapshot server bit (`0 | 4`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Q3SnapshotServerBit {
    /// Zero.
    Zero,
    /// Four.
    Four,
}

impl Q3SnapshotServerBit {
    /// Bit value.
    #[must_use]
    pub fn value(self) -> i32 {
        match self {
            Self::Zero => 0,
            Self::Four => 4,
        }
    }
}

/// Local recording source (`LocalRecordingSource`).
#[derive(Debug, Clone)]
pub enum LocalRecordingSource<Q1, Qw, Q2, Q3> {
    /// NetQuake source.
    Q1 {
        /// World generation.
        world: u64,
        /// Server host.
        host: Rc<RefCell<Q1>>,
        /// Recording player.
        player: RecordingPlayer,
        /// View angles.
        view_angles: Vec3,
    },
    /// QuakeWorld source.
    Qw {
        /// World generation.
        world: u64,
        /// Server host.
        host: Rc<RefCell<Qw>>,
        /// Recording player.
        player: RecordingPlayer,
        /// View angles.
        view_angles: Vec3,
        /// Timestamp seconds.
        seconds: f32,
    },
    /// Quake II source.
    Q2 {
        /// World generation.
        world: u64,
        /// Server host.
        host: Rc<RefCell<Q2>>,
        /// Recording player.
        player: RecordingPlayer,
        /// Protocol identity.
        protocol: Q2ProtocolIdentity,
    },
    /// Quake III source.
    Q3 {
        /// World generation.
        world: u64,
        /// Server host.
        host: Rc<RefCell<Q3>>,
        /// Recording player.
        player: RecordingPlayer,
        /// Server id.
        server_id: i32,
        /// Snapshot server bit.
        snapshot_server_bit: Q3SnapshotServerBit,
    },
}

impl<Q1, Qw, Q2, Q3> LocalRecordingSource<Q1, Qw, Q2, Q3> {
    /// World generation.
    #[must_use]
    pub fn world(&self) -> u64 {
        match self {
            Self::Q1 { world, .. } | Self::Qw { world, .. } | Self::Q2 { world, .. } | Self::Q3 { world, .. } => *world,
        }
    }
}

/// NetQuake recording host.
pub trait Q1RecordingHost: RecordingAdmission {
    /// Host failure.
    type Error;
    /// Demo protocol.
    fn demo_protocol(&self) -> Q1DemoProtocol;
    /// Encoded seed messages in packet order.
    fn seed_messages(&mut self, player: &RecordingPlayer, view_angles: Vec3) -> Result<Vec<Vec<u8>>, Self::Error>;
    /// Encoded frame message.
    fn frame_message(&mut self, player: &RecordingPlayer, output: &SimulationOutput) -> Result<Vec<u8>, Self::Error>;
}

/// QuakeWorld signon packets.
#[derive(Debug, Clone)]
pub struct QwSignonPackets {
    /// Seeded records; packet kinds become packets.
    pub initial: Vec<QwDemoRecord>,
    /// Signon buffers.
    pub signon_buffers: Vec<Vec<u8>>,
    /// Read-only recording signon.
    pub recording_signon: Option<Vec<Vec<u8>>>,
}

/// QuakeWorld recording host.
pub trait QwRecordingHost: RecordingAdmission {
    /// Host failure.
    type Error;
    /// Signon packets.
    fn signon_packets(&mut self, player: &RecordingPlayer, seconds: f32) -> Result<QwSignonPackets, Self::Error>;
    /// Encoded frame payloads in packet order.
    fn frame_packets(
        &mut self,
        player: &RecordingPlayer,
        output: &SimulationOutput,
        view_angles: Vec3,
        seconds: f32,
    ) -> Result<Vec<Vec<u8>>, Self::Error>;
}

/// Quake II recording host.
pub trait Q2RecordingHost: RecordingAdmission {
    /// Host failure.
    type Error;
    /// Encoded seed messages in packet order.
    fn seed_messages(
        &mut self,
        player: &RecordingPlayer,
        protocol: Q2ProtocolIdentity,
    ) -> Result<Vec<Vec<u8>>, Self::Error>;
    /// Encoded frame messages in packet order.
    fn frame_messages(
        &mut self,
        player: &RecordingPlayer,
        output: &SimulationOutput,
        events: &[LocalRecordingEvent],
        protocol: Q2ProtocolIdentity,
    ) -> Result<Vec<Vec<u8>>, Self::Error>;
}

/// Quake III recording snapshot.
#[derive(Debug, Clone)]
pub struct Q3RecordingSnapshot {
    /// Area mask.
    pub area_mask: Vec<u8>,
    /// Player state.
    pub player: Q3PlayerState,
    /// Entities.
    pub entities: Vec<Q3EntityState>,
}

/// Quake III recording host.
pub trait Q3RecordingHost: RecordingAdmission {
    /// Host failure.
    type Error;
    /// Product.
    fn product(&self) -> Q3Product;
    /// Server time.
    fn time(&self) -> i32;
    /// Gamestate for a player.
    fn game_state(&mut self, player: &RecordingPlayer, server_id: i32) -> Result<Gamestate, Self::Error>;
    /// Snapshot for a player.
    fn snapshot(&mut self, player: &RecordingPlayer) -> Result<Q3RecordingSnapshot, Self::Error>;
}

/// Presentation event surface used by local recording.
#[derive(Debug, Clone)]
pub enum LocalRecordingEvent {
    /// Quake III source event.
    Q3Source {
        /// Recipient actor, if directed.
        recipient: Option<ActorId>,
        /// Event.
        event: Q3RecordingEvent,
    },
    /// Any other event.
    Other,
}

/// Quake III recording event.
#[derive(Debug, Clone)]
pub enum Q3RecordingEvent {
    /// Server command.
    ServerCommand {
        /// Client number (`-1` broadcasts).
        client: i32,
        /// Command text.
        text: String,
    },
    /// Configstring.
    Configstring {
        /// Index.
        index: i32,
        /// Value.
        value: String,
    },
}

enum Prepared {
    Q1 {
        world: u64,
        seed: DemoRecordingSeed,
    },
    Qw {
        world: u64,
        seed: DemoRecordingSeed,
    },
    Q2 {
        world: u64,
        seed: DemoRecordingSeed,
    },
    Q3 {
        world: u64,
        seed: DemoRecordingSeed,
        state: Gamestate,
    },
}

impl Prepared {
    fn world(&self) -> u64 {
        match self {
            Self::Q1 { world, .. } | Self::Qw { world, .. } | Self::Q2 { world, .. } | Self::Q3 { world, .. } => *world,
        }
    }

    fn seed(&self) -> &DemoRecordingSeed {
        match self {
            Self::Q1 { seed, .. } | Self::Qw { seed, .. } | Self::Q2 { seed, .. } | Self::Q3 { seed, .. } => seed,
        }
    }
}

/// Shared demo recording sink cell.
type SharedRecordingSink = Rc<RefCell<Option<Rc<RefCell<dyn DemoRecordingSink>>>>>;

/// Local demo recording (`LocalDemoRecording`).
pub struct LocalDemoRecording<Q1, Qw, Q2, Q3> {
    read: Box<dyn FnMut() -> LocalRecordingSource<Q1, Qw, Q2, Q3>>,
    sink: SharedRecordingSink,
    prepared: Option<Prepared>,
    sequence: i32,
    command_sequence: i32,
}

impl<Q1, Qw, Q2, Q3> LocalDemoRecording<Q1, Qw, Q2, Q3>
where
    Q1: Q1RecordingHost + 'static,
    Qw: QwRecordingHost + 'static,
    Q2: Q2RecordingHost + 'static,
    Q3: Q3RecordingHost + 'static,
    Q1::Error: Into<LocalRecordingError>,
    Qw::Error: Into<LocalRecordingError>,
    Q2::Error: Into<LocalRecordingError>,
    Q3::Error: Into<LocalRecordingError>,
{
    /// Borrow a recording over a source reader.
    pub fn new(read: impl FnMut() -> LocalRecordingSource<Q1, Qw, Q2, Q3> + 'static) -> Self {
        Self {
            read: Box::new(read),
            sink: Rc::new(RefCell::new(None)),
            prepared: None,
            sequence: 0,
            command_sequence: 0,
        }
    }

    /// Seed the recording (`seed`).
    pub fn seed(&mut self) -> Result<DemoRecordingSeed, LocalRecordingError> {
        let source = (self.read)();
        let admission = match &source {
            LocalRecordingSource::Q1 { host, .. } => host.borrow().supports_source_wire(),
            LocalRecordingSource::Qw { host, .. } => host.borrow().supports_source_wire(),
            LocalRecordingSource::Q2 { host, .. } => host.borrow().supports_source_wire(),
            LocalRecordingSource::Q3 { host, .. } => host.borrow().supports_source_wire(),
        };
        if let WireAdmission::Unsupported { reasons } = admission {
            return Err(LocalRecordingError::Admission(reasons.join("; ")));
        }
        match source {
            LocalRecordingSource::Qw {
                host,
                player,
                view_angles,
                seconds,
                world,
            } => {
                let _ = view_angles;
                let signon = host.borrow_mut().signon_packets(&player, seconds).map_err(Into::into)?;
                let Some(recording) = signon.recording_signon else {
                    return Err(LocalRecordingError::MissingQwSignon);
                };
                let mut packets = vec![DemoRecordingPacket::Qw {
                    record: QwDemoRecord::Sequences {
                        seconds,
                        outgoing: 0,
                        incoming: 0,
                    },
                }];
                let mut count = 0;
                for record in signon.initial {
                    if matches!(record, QwDemoRecord::Packet { .. }) {
                        count += 1;
                        packets.push(DemoRecordingPacket::Qw { record });
                    }
                }
                self.sequence = count;
                for bytes in signon.signon_buffers.into_iter().chain(recording) {
                    packets.push(self.qw_packet(bytes, seconds)?);
                }
                let seed = DemoRecordingSeed {
                    identity: DemoRecordingIdentity::Qw,
                    packets,
                };
                self.prepared = Some(Prepared::Qw {
                    world,
                    seed: seed.clone(),
                });
                Ok(seed)
            }
            LocalRecordingSource::Q1 {
                host,
                player,
                view_angles,
                world,
            } => {
                let protocol = host.borrow().demo_protocol();
                let messages = host
                    .borrow_mut()
                    .seed_messages(&player, view_angles)
                    .map_err(Into::into)?;
                let packets = messages
                    .into_iter()
                    .map(|message| DemoRecordingPacket::Q1 { message, view_angles })
                    .collect();
                let seed = DemoRecordingSeed {
                    identity: DemoRecordingIdentity::Q1 { protocol, track: -1.0 },
                    packets,
                };
                self.prepared = Some(Prepared::Q1 {
                    world,
                    seed: seed.clone(),
                });
                Ok(seed)
            }
            LocalRecordingSource::Q2 {
                host,
                player,
                protocol,
                world,
            } => {
                let messages = host.borrow_mut().seed_messages(&player, protocol).map_err(Into::into)?;
                let packets = messages
                    .into_iter()
                    .map(|message| DemoRecordingPacket::Q2 { message })
                    .collect();
                let seed = DemoRecordingSeed {
                    identity: DemoRecordingIdentity::Q2 { protocol },
                    packets,
                };
                self.prepared = Some(Prepared::Q2 {
                    world,
                    seed: seed.clone(),
                });
                Ok(seed)
            }
            LocalRecordingSource::Q3 {
                host,
                player,
                server_id,
                snapshot_server_bit,
                world,
            } => {
                let _ = snapshot_server_bit;
                let state = host.borrow_mut().game_state(&player, server_id).map_err(Into::into)?;
                self.command_sequence = state.command_sequence;
                let product = host.borrow().product();
                let message = self.q3_message(&product, &state, std::slice::from_ref(&state_command(&state)))?;
                let sequence = self.sequence;
                self.sequence += 1;
                let seed = DemoRecordingSeed {
                    identity: DemoRecordingIdentity::Q3,
                    packets: vec![DemoRecordingPacket::Q3 { sequence, message }],
                };
                self.prepared = Some(Prepared::Q3 {
                    world,
                    seed: seed.clone(),
                    state,
                });
                Ok(seed)
            }
        }
    }

    /// Attach a sink (`attach`).
    pub fn attach(
        &mut self,
        sink: Rc<RefCell<dyn DemoRecordingSink>>,
    ) -> Result<Box<dyn FnOnce()>, LocalRecordingError> {
        if self.sink.borrow().is_some() || self.prepared.is_none() {
            return Err(LocalRecordingError::SeedRequired);
        }
        *self.sink.borrow_mut() = Some(Rc::clone(&sink));
        let cell = Rc::clone(&self.sink);
        Ok(Box::new(move || {
            let same = cell.borrow().as_ref().is_some_and(|stored| Rc::ptr_eq(stored, &sink));
            if same {
                cell.borrow_mut().take();
            }
        }))
    }

    /// Frame a QuakeWorld packet with a sequence prefix.
    fn qw_packet(&mut self, payload: Vec<u8>, seconds: f32) -> Result<DemoRecordingPacket, LocalRecordingError> {
        if payload.len() > 1442 {
            return Err(LocalRecordingError::QwdCapacity);
        }
        self.sequence += 1;
        let mut message = Vec::with_capacity(payload.len() + 8);
        message.extend_from_slice(&self.sequence.to_le_bytes());
        message.extend_from_slice(&[0u8; 4]);
        message.extend_from_slice(&payload);
        Ok(DemoRecordingPacket::Qw {
            record: QwDemoRecord::Packet { seconds, message },
        })
    }

    /// Encode a Quake III message against prepared baselines.
    fn q3_message(
        &self,
        product: &Q3Product,
        state: &Gamestate,
        operations: &[ServerOperation],
    ) -> Result<Vec<u8>, LocalRecordingError> {
        let baseline = |number: i32| {
            state.entries.iter().find_map(|entry| match entry {
                GamestateEntry::Baseline { number: found, entity } if *found == number => Some(entity.clone()),
                _ => None,
            })
        };
        let context = ServerMessageContext {
            product: *product,
            message_number: self.sequence,
            reliable_sequence: 0,
            server_command_sequence: self.command_sequence,
            parse_entities_number: 0,
            baseline: &baseline,
            history: &|_| None,
        };
        Ok(encode_server_message(0, operations, &context)?)
    }

    /// Publish one simulation step (`publish`).
    pub fn publish(
        &mut self,
        output: &SimulationOutput,
        events: &[LocalRecordingEvent],
    ) -> Result<(), LocalRecordingError> {
        let Some(sink) = self.sink.borrow().clone() else {
            return Ok(());
        };
        let source = (self.read)();
        let mut prepared = self.prepared.take().ok_or(LocalRecordingError::LostSeed)?;
        if prepared.world() != source.world() {
            let identity = format!("{:?}", prepared.seed().identity);
            let seed = self.seed()?;
            if format!("{:?}", seed.identity) != identity {
                return Err(LocalRecordingError::ProtocolChanged);
            }
            for packet in &seed.packets {
                sink.borrow_mut().append(packet)?;
            }
            prepared = self.prepared.take().ok_or(LocalRecordingError::LostSeed)?;
        }
        match (source, &prepared) {
            (
                LocalRecordingSource::Q1 {
                    host,
                    player,
                    view_angles,
                    ..
                },
                Prepared::Q1 { .. },
            ) => {
                let message = host.borrow_mut().frame_message(&player, output).map_err(Into::into)?;
                sink.borrow_mut()
                    .append(&DemoRecordingPacket::Q1 { message, view_angles })?;
            }
            (
                LocalRecordingSource::Qw {
                    host,
                    player,
                    view_angles,
                    seconds,
                    ..
                },
                Prepared::Qw { .. },
            ) => {
                let payloads = host
                    .borrow_mut()
                    .frame_packets(&player, output, view_angles, seconds)
                    .map_err(Into::into)?;
                for payload in payloads {
                    let packet = self.qw_packet(payload, seconds)?;
                    sink.borrow_mut().append(&packet)?;
                }
            }
            (
                LocalRecordingSource::Q2 {
                    host, player, protocol, ..
                },
                Prepared::Q2 { .. },
            ) => {
                let messages = host
                    .borrow_mut()
                    .frame_messages(&player, output, events, protocol)
                    .map_err(Into::into)?;
                for message in messages {
                    sink.borrow_mut().append(&DemoRecordingPacket::Q2 { message })?;
                }
            }
            (
                LocalRecordingSource::Q3 {
                    host,
                    player,
                    snapshot_server_bit,
                    ..
                },
                Prepared::Q3 { state, .. },
            ) => {
                let mut operations: Vec<ServerOperation> = Vec::new();
                for item in events {
                    let LocalRecordingEvent::Q3Source { recipient, event } = item else {
                        continue;
                    };
                    if recipient.as_ref().is_some_and(|recipient| *recipient != player.actor) {
                        continue;
                    }
                    let commands = match event {
                        Q3RecordingEvent::ServerCommand { client, text }
                            if *client == -1 || *client == player.source_entity =>
                        {
                            vec![text.clone()]
                        }
                        Q3RecordingEvent::Configstring { index, value } => q3_configstring_commands(*index, value),
                        _ => Vec::new(),
                    };
                    for text in commands {
                        self.command_sequence += 1;
                        operations.push(ServerOperation::Command {
                            sequence: self.command_sequence,
                            text,
                        });
                    }
                }
                let frame = host.borrow_mut().snapshot(&player).map_err(Into::into)?;
                let server_time = host.borrow().time();
                let product = host.borrow().product();
                operations.push(ServerOperation::Snapshot {
                    validity: SnapshotValidity::Valid,
                    snapshot: Box::new(Snapshot {
                        message_number: self.sequence,
                        server_time,
                        delta_number: -1,
                        flags: snapshot_server_bit.value(),
                        server_command_number: self.command_sequence,
                        parse_entities_number: 0,
                        area_mask: frame.area_mask,
                        player_state: frame.player,
                        entities: frame.entities,
                    }),
                });
                let message = self.q3_message(&product, state, &operations)?;
                let sequence = self.sequence;
                self.sequence += 1;
                sink.borrow_mut()
                    .append(&DemoRecordingPacket::Q3 { sequence, message })?;
            }
            _ => {
                self.prepared = Some(prepared);
                return Err(LocalRecordingError::SourceChanged);
            }
        }
        self.prepared = Some(prepared);
        Ok(())
    }
}

fn state_command(state: &Gamestate) -> ServerOperation {
    ServerOperation::Gamestate(Box::new(state.clone()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_core::identity::IdentityOwner;

    struct StubQ1;
    struct StubQw;
    struct StubQ2;
    struct StubQ3;

    impl RecordingAdmission for StubQ1 {
        fn supports_source_wire(&self) -> WireAdmission {
            WireAdmission::Supported
        }
    }

    impl RecordingAdmission for StubQw {
        fn supports_source_wire(&self) -> WireAdmission {
            WireAdmission::Supported
        }
    }

    impl RecordingAdmission for StubQ2 {
        fn supports_source_wire(&self) -> WireAdmission {
            WireAdmission::Supported
        }
    }

    impl RecordingAdmission for StubQ3 {
        fn supports_source_wire(&self) -> WireAdmission {
            WireAdmission::Supported
        }
    }

    impl Q1RecordingHost for StubQ1 {
        type Error = LocalRecordingError;

        fn demo_protocol(&self) -> Q1DemoProtocol {
            Q1DemoProtocol::V15
        }

        fn seed_messages(
            &mut self,
            _player: &RecordingPlayer,
            _view_angles: Vec3,
        ) -> Result<Vec<Vec<u8>>, LocalRecordingError> {
            Ok(vec![vec![1], vec![2]])
        }

        fn frame_message(
            &mut self,
            _player: &RecordingPlayer,
            _output: &SimulationOutput,
        ) -> Result<Vec<u8>, LocalRecordingError> {
            Ok(vec![3])
        }
    }

    impl QwRecordingHost for StubQw {
        type Error = LocalRecordingError;

        fn signon_packets(
            &mut self,
            _player: &RecordingPlayer,
            seconds: f32,
        ) -> Result<QwSignonPackets, LocalRecordingError> {
            Ok(QwSignonPackets {
                initial: vec![QwDemoRecord::Packet {
                    seconds,
                    message: vec![9],
                }],
                signon_buffers: vec![vec![7]],
                recording_signon: Some(vec![vec![8]]),
            })
        }

        fn frame_packets(
            &mut self,
            _player: &RecordingPlayer,
            _output: &SimulationOutput,
            _view_angles: Vec3,
            _seconds: f32,
        ) -> Result<Vec<Vec<u8>>, LocalRecordingError> {
            Ok(vec![vec![10]])
        }
    }

    impl Q2RecordingHost for StubQ2 {
        type Error = LocalRecordingError;

        fn seed_messages(
            &mut self,
            _player: &RecordingPlayer,
            _protocol: Q2ProtocolIdentity,
        ) -> Result<Vec<Vec<u8>>, LocalRecordingError> {
            Ok(vec![vec![4]])
        }

        fn frame_messages(
            &mut self,
            _player: &RecordingPlayer,
            _output: &SimulationOutput,
            _events: &[LocalRecordingEvent],
            _protocol: Q2ProtocolIdentity,
        ) -> Result<Vec<Vec<u8>>, LocalRecordingError> {
            Ok(vec![vec![5]])
        }
    }

    fn test_gamestate() -> Gamestate {
        Gamestate {
            command_sequence: 41,
            entries: Vec::new(),
            client_number: 0,
            checksum_feed: 0,
        }
    }

    impl Q3RecordingHost for StubQ3 {
        type Error = LocalRecordingError;

        fn product(&self) -> Q3Product {
            Q3Product::Base
        }

        fn time(&self) -> i32 {
            100
        }

        fn game_state(&mut self, _player: &RecordingPlayer, _server_id: i32) -> Result<Gamestate, LocalRecordingError> {
            Ok(test_gamestate())
        }

        fn snapshot(&mut self, _player: &RecordingPlayer) -> Result<Q3RecordingSnapshot, LocalRecordingError> {
            Ok(Q3RecordingSnapshot {
                area_mask: Vec::new(),
                player: Q3PlayerState::new(Q3Product::Base),
                entities: Vec::new(),
            })
        }
    }

    struct StubSink {
        packets: Vec<DemoRecordingPacket>,
    }

    impl DemoRecordingSink for StubSink {
        fn append(
            &mut self,
            packet: &DemoRecordingPacket,
        ) -> Result<(), super::super::demo_recording::DemoRecordingError> {
            self.packets.push(packet.clone());
            Ok(())
        }
    }

    fn player() -> RecordingPlayer {
        RecordingPlayer {
            actor: IdentityOwner::create("test").unwrap().actor(2, 0),
            source_entity: 2,
        }
    }

    #[test]
    fn q1_seed_and_publish_round_trip() {
        let host = Rc::new(RefCell::new(StubQ1));
        let host_reader = Rc::clone(&host);
        let mut recording: LocalDemoRecording<StubQ1, StubQw, StubQ2, StubQ3> =
            LocalDemoRecording::new(move || LocalRecordingSource::Q1 {
                world: 7,
                host: Rc::clone(&host_reader),
                player: player(),
                view_angles: Vec3::default(),
            });
        let seed = recording.seed().unwrap();
        assert_eq!(seed.packets.len(), 2);
        let stub = Rc::new(RefCell::new(StubSink { packets: Vec::new() }));
        let sink: Rc<RefCell<dyn DemoRecordingSink>> = stub.clone();
        let detach = recording.attach(sink).unwrap();
        let output = SimulationOutput {
            snapshot: qa_world::session::WorldSnapshot {
                frame: qa_core::time::FrameContext {
                    frame: 0,
                    time: qa_core::time::SourceTime::Milliseconds(0),
                    elapsed: qa_core::time::SourceTime::Milliseconds(16),
                    phase: qa_core::time::FramePhase::FrameEntry,
                },
                actors: Vec::new(),
                bodies: Vec::new(),
                inventories: Vec::new(),
            },
            events: Vec::new(),
        };
        recording.publish(&output, &[]).unwrap();
        assert_eq!(stub.borrow().packets.len(), 1);
        detach();
        recording.publish(&output, &[]).unwrap();
        assert_eq!(stub.borrow().packets.len(), 1);
    }

    #[test]
    fn qw_seed_frames_sequences() {
        let host = Rc::new(RefCell::new(StubQw));
        let host_reader = Rc::clone(&host);
        let mut recording: LocalDemoRecording<StubQ1, StubQw, StubQ2, StubQ3> =
            LocalDemoRecording::new(move || LocalRecordingSource::Qw {
                world: 3,
                host: Rc::clone(&host_reader),
                player: player(),
                view_angles: Vec3::default(),
                seconds: 1.5,
            });
        let seed = recording.seed().unwrap();
        assert_eq!(seed.packets.len(), 4);
        assert!(matches!(
            seed.packets[0],
            DemoRecordingPacket::Qw {
                record: QwDemoRecord::Sequences { .. }
            }
        ));
    }

    #[test]
    fn attach_requires_prepared_seed() {
        let host = Rc::new(RefCell::new(StubQ2));
        let host_reader = Rc::clone(&host);
        let mut recording: LocalDemoRecording<StubQ1, StubQw, StubQ2, StubQ3> =
            LocalDemoRecording::new(move || LocalRecordingSource::Q2 {
                world: 1,
                host: Rc::clone(&host_reader),
                player: player(),
                protocol: Q2ProtocolIdentity::Classic,
            });
        let sink: Rc<RefCell<dyn DemoRecordingSink>> = Rc::new(RefCell::new(StubSink { packets: Vec::new() }));
        let Err(error) = recording.attach(sink) else {
            panic!("attach without seed must fail");
        };
        assert!(matches!(error, LocalRecordingError::SeedRequired));
    }

    #[test]
    fn q3_seed_encodes_gamestate() {
        let host = Rc::new(RefCell::new(StubQ3));
        let host_reader = Rc::clone(&host);
        let mut recording: LocalDemoRecording<StubQ1, StubQw, StubQ2, StubQ3> =
            LocalDemoRecording::new(move || LocalRecordingSource::Q3 {
                world: 9,
                host: Rc::clone(&host_reader),
                player: player(),
                server_id: 1,
                snapshot_server_bit: Q3SnapshotServerBit::Four,
            });
        let seed = recording.seed().unwrap();
        assert_eq!(seed.packets.len(), 1);
        assert!(matches!(seed.packets[0], DemoRecordingPacket::Q3 { sequence: 0, .. }));
    }
}
