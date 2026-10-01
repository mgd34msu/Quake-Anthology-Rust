//! NetQuake client network endpoint.
//!
//! Port of `src/app/bootstrap/network/q1-client.ts` (`Q1ClientNetwork`),
//! following NetQuake `net_dgrm.c`/`cl_main.c` client progression. The
//! donor is asynchronous; this port resolves every step inline over the
//! synchronous [`DatagramTransport`](qa_net::common::transport::DatagramTransport).
//! Handshake, channel, decoding, signon, and recording reuse `qa-net`
//! (`NetQuakeConnectClient`, `NetQuakeChannel`, `NetQuakeDecoder`,
//! `NetQuakeSignon`, `NetQuakeRecordingState`); failures surface as
//! [`Q1ClientError`] and cross the [`ApplicationNetwork`] boundary as
//! [`ApplicationNetworkError`] with the donor message text intact.

use std::collections::VecDeque;

use qa_core::math::Vec3;
use qa_net::common::commands::ActorCommand;
use qa_net::common::endpoint::{same_address, NetworkAddress};
use qa_net::common::session::WireSelection;
use qa_net::common::transport::{DatagramTransport, ReceiveEvent, TransportError};
use qa_net::protocol::ProtocolIdentity;
use qa_net::q1_net::{
    write_client_string_command, write_net_quake_move, NetQuakeChannel, NetQuakeConnectClient, NetQuakeConnectState,
    NetQuakeDecoder, NetQuakeMessage, NetQuakeRecordingState, NetQuakeSeatIdentity, NetQuakeSignon, NqUnit,
    NqUserCommand, Q1NetError, RereleaseMessages,
};
use qa_net::q1_wide::{NqProfile, WIDE_MAX_MSGLEN};
use qa_world::movement::types::Q1UserCommand;
use qa_world::session::SimulationOutput;
use thiserror::Error;

use super::types::{
    ApplicationNetwork, ApplicationNetworkError, ApplicationNetworkPhase, ApplicationNetworkRecording,
    ApplicationNetworkRole, NetworkPresentationEvent,
};
use crate::bootstrap::demo_recording::{
    DemoRecordingError, DemoRecordingIdentity, DemoRecordingPacket, DemoRecordingSeed, DemoRecordingSink,
    Q1DemoProtocol,
};
use qa_net::msg::MsgWriter;

/// NetQuake client failure.
#[derive(Debug, Error)]
pub enum Q1ClientError {
    /// Policy or protocol failure with donor text.
    #[error("{0}")]
    Message(String),
    /// Wire failure.
    #[error(transparent)]
    Net(#[from] Q1NetError),
    /// Transport failure.
    #[error(transparent)]
    Transport(#[from] TransportError),
    /// Recording failure.
    #[error(transparent)]
    Recording(#[from] DemoRecordingError),
}

impl From<Q1ClientError> for ApplicationNetworkError {
    fn from(error: Q1ClientError) -> Self {
        Self::Message(error.to_string())
    }
}

/// Quake application client host (`Q1ApplicationClientHost`).
pub trait Q1ApplicationClientHost {
    /// Receive decoded messages (donor async; resolves inline here).
    fn receive(&mut self, messages: &[NetQuakeMessage], now_milliseconds: u64);
    /// Convert an actor command into a wire command.
    fn command(&self, command: &ActorCommand) -> Q1UserCommand;
    /// Handle a disconnect.
    fn disconnected(&mut self, reason: &str);
}

/// Quake client network options (`Q1ClientNetworkOptions`).
pub struct Q1ClientNetworkOptions<T, H> {
    /// Datagram transport.
    pub transport: T,
    /// Remote address.
    pub remote: NetworkAddress,
    /// Client host.
    pub host: H,
    /// Seat identity.
    pub seat: NetQuakeSeatIdentity,
    /// Idle timeout in milliseconds.
    pub timeout_milliseconds: Option<u64>,
}

/// NetQuake client network endpoint (`Q1ClientNetwork`).
pub struct Q1ClientNetwork<T, H> {
    options: Q1ClientNetworkOptions<T, H>,
    recording_state: NetQuakeRecordingState,
    recording_sink: Option<Box<dyn DemoRecordingSink>>,
    view_angles: Vec3,
    handshake: NetQuakeConnectClient,
    channel: NetQuakeChannel,
    decoder: NetQuakeDecoder,
    signon: NetQuakeSignon,
    state: ApplicationNetworkPhase,
    peer: Option<NetworkAddress>,
    last_received: u64,
    movement_messages: u32,
    reliable: VecDeque<Vec<u8>>,
}

impl<T, H> Q1ClientNetwork<T, H>
where
    T: DatagramTransport<Address = NetworkAddress>,
    H: Q1ApplicationClientHost,
{
    /// Build a client endpoint.
    pub fn new(options: Q1ClientNetworkOptions<T, H>) -> Result<Self, Q1ClientError> {
        if options.seat.extension_flags.is_some() {
            return Err(Q1ClientError::Message(
                "Native NetQuake does not negotiate private extensions".to_string(),
            ));
        }
        Ok(Self {
            signon: NetQuakeSignon::new(options.seat.clone()),
            options,
            recording_state: NetQuakeRecordingState::default(),
            recording_sink: None,
            view_angles: Vec3::default(),
            handshake: NetQuakeConnectClient::new(),
            channel: NetQuakeChannel::new(WIDE_MAX_MSGLEN, 1024)?,
            decoder: NetQuakeDecoder::new(NqProfile::Netquake, RereleaseMessages::KnownRetail, true),
            state: ApplicationNetworkPhase::Connecting,
            peer: None,
            last_received: 0,
            movement_messages: 0,
            reliable: VecDeque::new(),
        })
    }

    /// Connection phase.
    #[must_use]
    pub fn phase(&self) -> ApplicationNetworkPhase {
        self.state
    }

    /// Connected server address.
    #[must_use]
    pub fn server_address(&self) -> Option<&NetworkAddress> {
        self.peer.as_ref()
    }

    /// Queue a reliable string command (`command`).
    pub fn command(&mut self, text: &str) -> Result<(), Q1ClientError> {
        if self.peer.is_none() {
            return Err(Q1ClientError::Message("NetQuake client is not connected".to_string()));
        }
        let mut writer = MsgWriter::new(8000, false);
        write_client_string_command(&mut writer, text)?;
        self.reliable.push_back(writer.bytes().to_vec());
        Ok(())
    }

    /// Reject the connection.
    fn reject(&mut self, reason: &str) {
        self.state = ApplicationNetworkPhase::Rejected;
        self.options.host.disconnected(reason);
    }

    /// Poll the transport, returning actor commands (always empty: the
    /// client never steps authority).
    pub fn poll(&mut self, now: u64) -> Result<Vec<ActorCommand>, Q1ClientError> {
        if self.state == ApplicationNetworkPhase::Closed || self.state == ApplicationNetworkPhase::Rejected {
            return Ok(Vec::new());
        }
        let now_ms = now as f64;
        if self.peer.is_none() {
            if let Some(request) = self.handshake.next(now_ms)? {
                self.options.transport.send(&self.options.remote, &request)?;
            }
            if let NetQuakeConnectState::Rejected { reason } = &self.handshake.state {
                let reason = reason.clone();
                self.reject(&reason);
                return Ok(Vec::new());
            }
        }
        while let Some(packet) = self.options.transport.poll()? {
            let ReceiveEvent::Packet { from, payload, .. } = packet else {
                continue;
            };
            let expected = self.peer.as_ref().unwrap_or(&self.options.remote);
            if !same_address(&from, expected, true) {
                continue;
            }
            if self.peer.is_none() {
                self.handshake.receive(&payload)?;
                match &self.handshake.state {
                    NetQuakeConnectState::Rejected { reason } => {
                        let reason = reason.clone();
                        self.reject(&reason);
                        break;
                    }
                    NetQuakeConnectState::Connected { port } => {
                        let port = *port as u16;
                        self.peer = Some(with_port(&self.options.remote, port));
                        self.state = ApplicationNetworkPhase::Loading;
                        self.last_received = now;
                    }
                    NetQuakeConnectState::Waiting { .. } => {}
                }
                continue;
            }
            let received = self.channel.receive(&payload, now_ms)?;
            self.last_received = now;
            let peer = self.peer.clone().expect("peer checked");
            for reply in &received.replies {
                self.options.transport.send(&peer, reply)?;
            }
            let Some(delivery) = received.delivery else {
                continue;
            };
            let messages = self.decoder.decode(&delivery.payload)?;
            self.recording_state.observe(&messages);
            for message in &messages {
                if let NetQuakeMessage::ServerInfo { max_clients, .. } = message {
                    if *max_clients < 1 || *max_clients > 16 {
                        return Err(Q1ClientError::Message(
                            "Native NetQuake requires 1–16 scoreboard slots".to_string(),
                        ));
                    }
                    self.signon.stage = 0;
                    self.movement_messages = 0;
                    self.state = ApplicationNetworkPhase::Loading;
                }
            }
            self.options.host.receive(&messages, now);
            for message in &messages {
                if let NetQuakeMessage::SetAngle { angles } = message {
                    self.view_angles = Vec3 {
                        x: angles[0] as f32,
                        y: angles[1] as f32,
                        z: angles[2] as f32,
                    };
                }
            }
            if let Some(sink) = self.recording_sink.as_mut() {
                sink.append(&DemoRecordingPacket::Q1 {
                    message: delivery.payload.clone(),
                    view_angles: self.view_angles,
                })?;
            }
            for message in &messages {
                match message {
                    NetQuakeMessage::Signon { stage } => {
                        let response = self.signon.receive(*stage)?;
                        if !response.is_empty() {
                            self.reliable.push_back(response);
                        }
                    }
                    NetQuakeMessage::Entity { .. } => self.signon.first_entity(),
                    NetQuakeMessage::Unit(NqUnit::Disconnect) => {
                        self.state = ApplicationNetworkPhase::Closed;
                        self.options.host.disconnected("Server disconnected");
                        return Ok(Vec::new());
                    }
                    _ => {}
                }
            }
            if self.signon.active() {
                self.state = ApplicationNetworkPhase::Active;
            }
        }
        if self.peer.is_some() {
            if now.saturating_sub(self.last_received) > self.options.timeout_milliseconds.unwrap_or(120_000) {
                self.reject("Connection timed out");
                return Ok(Vec::new());
            }
            if self.channel.can_send_reliable() {
                if let Some(bytes) = self.reliable.pop_front() {
                    self.channel.queue_reliable(&bytes)?;
                }
            }
            if let Some(packet) = self.channel.next(now_ms)? {
                let peer = self.peer.clone().expect("peer checked");
                self.options.transport.send(&peer, &packet)?;
            }
        }
        Ok(Vec::new())
    }

    /// Submit local input.
    pub fn submit(&mut self, commands: &[ActorCommand], _now: u64) -> Result<(), Q1ClientError> {
        if self.state != ApplicationNetworkPhase::Active || self.peer.is_none() {
            return Ok(());
        }
        if commands.len() > 1 {
            return Err(Q1ClientError::Message(
                "A native NetQuake connection carries one player".to_string(),
            ));
        }
        for command in commands {
            let walker = self.options.host.command(command);
            self.view_angles = walker.view_angles;
            self.movement_messages += 1;
            if self.movement_messages <= 2 {
                continue;
            }
            let mut writer = MsgWriter::new(128, false);
            write_net_quake_move(
                &mut writer,
                &NqUserCommand {
                    acknowledged_server_time_seconds: self.decoder.time_seconds,
                    view_angles: [
                        f64::from(walker.view_angles.x),
                        f64::from(walker.view_angles.y),
                        f64::from(walker.view_angles.z),
                    ],
                    forward_move: walker.forward_move as i16,
                    side_move: walker.side_move as i16,
                    up_move: walker.up_move as i16,
                    buttons: walker.buttons as u8,
                    impulse: walker.impulse as u8,
                },
                self.decoder.protocol,
            )?;
            let peer = self.peer.clone().expect("peer checked");
            let packet = self.channel.unreliable(writer.bytes())?;
            self.options.transport.send(&peer, &packet)?;
        }
        Ok(())
    }

    /// Remote clients cannot publish authoritative state.
    pub fn publish(
        &mut self,
        _output: &SimulationOutput,
        _events: &[NetworkPresentationEvent],
        _now: u64,
    ) -> Result<(), Q1ClientError> {
        Err(Q1ClientError::Message(
            "Remote client cannot publish authoritative state".to_string(),
        ))
    }

    /// Close the endpoint.
    pub fn close(&mut self) {
        if self.options.transport.closed() {
            return;
        }
        if self.peer.is_some() && self.state != ApplicationNetworkPhase::Closed {
            let peer = self.peer.clone().expect("peer checked");
            if let Ok(packet) = self.channel.unreliable(&[2]) {
                let _ = self.options.transport.send(&peer, &packet);
            }
        }
        self.state = ApplicationNetworkPhase::Closed;
        self.options.transport.close();
    }
}

/// Copy an address with a replaced port (donor `{ ...remote, port }`).
fn with_port(address: &NetworkAddress, port: u16) -> NetworkAddress {
    match address {
        NetworkAddress::Ipv4 { host, .. } => NetworkAddress::Ipv4 { host: *host, port },
        NetworkAddress::Ipv6 { host, .. } => NetworkAddress::Ipv6 {
            host: host.clone(),
            port,
        },
        NetworkAddress::Ipx { network, node, .. } => NetworkAddress::Ipx {
            network: *network,
            node: *node,
            port,
        },
        // The donor only dials IP/IPX remotes; a loopback remote has no
        // port to replace, so it survives unchanged.
        NetworkAddress::Loopback { .. } => address.clone(),
    }
}

impl<T, H> ApplicationNetworkRecording for Q1ClientNetwork<T, H>
where
    T: DatagramTransport<Address = NetworkAddress>,
    H: Q1ApplicationClientHost,
{
    fn seed(&self) -> Result<DemoRecordingSeed, ApplicationNetworkError> {
        if self.state != ApplicationNetworkPhase::Active {
            return Err(ApplicationNetworkError::Message(
                "Recording requires an active NetQuake connection".to_string(),
            ));
        }
        let protocol = match self.decoder.protocol {
            NqProfile::Netquake => Q1DemoProtocol::V15,
            NqProfile::Fitzquake => Q1DemoProtocol::V666,
            NqProfile::Rmq { .. } => Q1DemoProtocol::V999,
        };
        let packets = self
            .recording_state
            .seed(
                self.decoder.protocol,
                self.decoder.rerelease_messages,
                self.decoder.standard_quake,
            )
            .map_err(|error| ApplicationNetworkError::Message(error.to_string()))?;
        Ok(DemoRecordingSeed {
            identity: DemoRecordingIdentity::Q1 { protocol, track: -1.0 },
            packets: packets
                .into_iter()
                .map(|message| DemoRecordingPacket::Q1 {
                    message,
                    view_angles: self.view_angles,
                })
                .collect(),
        })
    }

    fn attach(&mut self, sink: Box<dyn DemoRecordingSink>) -> Result<Box<dyn FnOnce() + '_>, ApplicationNetworkError> {
        if self.recording_sink.is_some() || self.state != ApplicationNetworkPhase::Active {
            return Err(ApplicationNetworkError::Message(
                "NetQuake recording cannot attach".to_string(),
            ));
        }
        self.recording_sink = Some(sink);
        // Attaching while attached throws, so the slot always holds this
        // sink; an unconditional take matches the donor's identity check.
        Ok(Box::new(|| {
            self.recording_sink.take();
        }))
    }
}

impl<T, H> ApplicationNetwork for Q1ClientNetwork<T, H>
where
    T: DatagramTransport<Address = NetworkAddress>,
    H: Q1ApplicationClientHost,
{
    fn recording(&mut self) -> Option<&mut dyn ApplicationNetworkRecording> {
        Some(self)
    }

    fn role(&self) -> ApplicationNetworkRole {
        ApplicationNetworkRole::Client
    }

    fn phase(&self) -> ApplicationNetworkPhase {
        self.state
    }

    fn wire(&self) -> WireSelection {
        WireSelection::Source {
            protocol: match self.decoder.protocol {
                NqProfile::Netquake => ProtocolIdentity::Q1Netquake,
                NqProfile::Fitzquake => ProtocolIdentity::Q1Fitzquake,
                NqProfile::Rmq { flags } => ProtocolIdentity::Q1Rmq { flags },
            },
        }
    }

    fn poll(&mut self, now_milliseconds: u64) -> Result<Vec<ActorCommand>, ApplicationNetworkError> {
        self.poll(now_milliseconds).map_err(ApplicationNetworkError::from)
    }

    fn submit(&mut self, commands: &[ActorCommand], now_milliseconds: u64) -> Result<(), ApplicationNetworkError> {
        self.submit(commands, now_milliseconds)
            .map_err(ApplicationNetworkError::from)
    }

    fn publish(
        &mut self,
        output: &SimulationOutput,
        events: &[NetworkPresentationEvent],
        now_milliseconds: u64,
    ) -> Result<(), ApplicationNetworkError> {
        self.publish(output, events, now_milliseconds)
            .map_err(ApplicationNetworkError::from)
    }

    fn close(&mut self) {
        self.close();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_net::common::transport::{DatagramLimits, PacketQueue};
    use std::sync::Arc;

    struct LoopTransport {
        queue: PacketQueue<NetworkAddress>,
        address: NetworkAddress,
    }

    impl DatagramTransport for LoopTransport {
        type Address = NetworkAddress;

        fn address(&self) -> NetworkAddress {
            self.address.clone()
        }

        fn closed(&self) -> bool {
            false
        }

        fn send(&self, _to: &NetworkAddress, _payload: &[u8]) -> Result<bool, TransportError> {
            Ok(true)
        }

        fn poll(&self) -> Result<Option<ReceiveEvent<NetworkAddress>>, TransportError> {
            Ok(self.queue.poll())
        }

        fn subscribe_readable(&self, _listener: Arc<dyn Fn() + Send + Sync>) -> Result<u64, TransportError> {
            Ok(0)
        }

        fn unsubscribe(&self, _token: u64) {}

        fn close(&self) {}
    }

    struct ScriptHost {
        disconnected: Vec<String>,
    }

    impl Q1ApplicationClientHost for ScriptHost {
        fn receive(&mut self, _messages: &[NetQuakeMessage], _now_milliseconds: u64) {}

        fn command(&self, _command: &ActorCommand) -> Q1UserCommand {
            Q1UserCommand {
                acknowledged_server_time_seconds: 0.0,
                view_angles: Vec3::default(),
                forward_move: 0.0,
                side_move: 0.0,
                up_move: 0.0,
                buttons: 0,
                impulse: 0,
            }
        }

        fn disconnected(&mut self, reason: &str) {
            self.disconnected.push(reason.to_string());
        }
    }

    fn address() -> NetworkAddress {
        NetworkAddress::Ipv4 {
            host: [127, 0, 0, 1],
            port: 26000,
        }
    }

    fn client() -> Q1ClientNetwork<LoopTransport, ScriptHost> {
        let queue = PacketQueue::new(
            DatagramLimits {
                max_bytes: 65536,
                queue_packets: 16,
            },
            qa_net::common::transport::monotonic_clock(),
        )
        .expect("queue");
        Q1ClientNetwork::new(Q1ClientNetworkOptions {
            transport: LoopTransport {
                queue,
                address: address(),
            },
            remote: address(),
            host: ScriptHost {
                disconnected: Vec::new(),
            },
            seat: NetQuakeSeatIdentity {
                name: "player".to_string(),
                color: 0,
                spawn_parameters: String::new(),
                extension_flags: None,
            },
            timeout_milliseconds: None,
        })
        .expect("client")
    }

    #[test]
    fn rejects_private_extensions() {
        let queue = PacketQueue::new(
            DatagramLimits {
                max_bytes: 65536,
                queue_packets: 16,
            },
            qa_net::common::transport::monotonic_clock(),
        )
        .expect("queue");
        let result = Q1ClientNetwork::new(Q1ClientNetworkOptions {
            transport: LoopTransport {
                queue,
                address: address(),
            },
            remote: address(),
            host: ScriptHost {
                disconnected: Vec::new(),
            },
            seat: NetQuakeSeatIdentity {
                name: "player".to_string(),
                color: 0,
                spawn_parameters: String::new(),
                extension_flags: Some(1),
            },
            timeout_milliseconds: None,
        });
        let Err(error) = result else {
            panic!("extensions must fail");
        };
        assert_eq!(
            error.to_string(),
            "Native NetQuake does not negotiate private extensions"
        );
    }

    #[test]
    fn handshake_times_out_without_replies() {
        let mut client = client();
        assert_eq!(client.phase(), ApplicationNetworkPhase::Connecting);
        assert!(client.poll(0).expect("poll").is_empty());
        assert!(client.poll(3000).expect("poll").is_empty());
        assert!(client.poll(6000).expect("poll").is_empty());
        assert!(client.poll(9000).expect("poll").is_empty());
        assert_eq!(client.phase(), ApplicationNetworkPhase::Rejected);
        assert!(client.poll(12000).expect("poll").is_empty());
    }

    #[test]
    fn idle_client_refuses_commands_and_publish() {
        let mut client = client();
        assert_eq!(
            client.command("status").expect_err("command").to_string(),
            "NetQuake client is not connected"
        );
        assert_eq!(
            ApplicationNetworkRecording::seed(&client)
                .expect_err("seed")
                .to_string(),
            "Recording requires an active NetQuake connection"
        );
    }
}
