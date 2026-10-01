//! NetQuake server network endpoint.
//!
//! Port of `src/app/bootstrap/network/q1.ts` (`Q1ServerNetwork`). The donor
//! is asynchronous; this port resolves every step inline over the
//! synchronous [`DatagramTransport`](qa_net::common::transport::DatagramTransport).
//! Connectionless control, channels, client decoding, message/entity
//! encoding, and command parsing reuse `qa-net`. Per-packet failures
//! disconnect the peer and print, exactly like the donor's `poll` catch;
//! transport-level failures reject the poll. Codec limits follow the donor
//! (`NQ15` 8000/1024, wide 64000/64000).

use std::collections::{HashMap, VecDeque};

use qa_core::identity::ClientId;
use qa_net::common::commands::ActorCommand;
use qa_net::common::endpoint::{address_key, NetworkAddress};
use qa_net::common::session::{WireAdmission, WireSelection};
use qa_net::common::transport::{DatagramTransport, ReceiveEvent, TransportError};
use qa_net::msg::MsgWriter;
use qa_net::protocol::ProtocolIdentity;
use qa_net::q1::{MAX_DATAGRAM as NQ15_MAX_DATAGRAM, MAX_MSGLEN as NQ15_MAX_MSGLEN};
use qa_net::q1_net::{
    answer_net_quake_control, decode_net_quake_client, quake_world_command_arguments, write_net_quake_entity,
    write_net_quake_message, NetQuakeChannel, NetQuakeClientMessage, NetQuakeConnectVerdict, NetQuakeConnectionHost,
    NetQuakeControl, NetQuakeMessage, NqNamedSlot, NqText, NqUnit, NqUserCommand, Q1NetError, Q1WireEntity,
    RereleaseMessages,
};
use qa_net::q1_wide::{nq_protocol_flags, NqProfile, WIDE_MAX_MSGLEN};
use qa_world::movement::types::Q1UserCommand;
use qa_world::session::SimulationOutput;
use thiserror::Error;

use super::q1_types::{
    Q1ApplicationAdmission, Q1ApplicationGameState, Q1ApplicationMessage, Q1ApplicationPlayer, Q1ApplicationServerHost,
    Q1ServerNetworkOptions,
};
use super::types::{
    ApplicationNetwork, ApplicationNetworkError, ApplicationNetworkPhase, ApplicationNetworkRole,
    NetworkPresentationEvent,
};

/// NetQuake server failure.
#[derive(Debug, Error)]
pub enum Q1ServerError {
    /// Policy or protocol failure with donor text.
    #[error("{0}")]
    Message(String),
    /// Wire failure.
    #[error(transparent)]
    Net(#[from] Q1NetError),
    /// Transport failure.
    #[error(transparent)]
    Transport(#[from] TransportError),
    /// Message buffer failure.
    #[error(transparent)]
    Msg(#[from] qa_net::msg::MsgError),
}

impl From<Q1ServerError> for ApplicationNetworkError {
    fn from(error: Q1ServerError) -> Self {
        Self::Message(error.to_string())
    }
}

/// Map a wire identity to its NetQuake profile (donor `Q1ProtocolIdentity`).
fn nq_profile(protocol: ProtocolIdentity) -> Result<NqProfile, Q1ServerError> {
    match protocol {
        ProtocolIdentity::Q1Netquake => Ok(NqProfile::Netquake),
        ProtocolIdentity::Q1Fitzquake => Ok(NqProfile::Fitzquake),
        ProtocolIdentity::Q1Rmq { flags } => Ok(NqProfile::Rmq { flags }),
        other => Err(Q1ServerError::Message(format!(
            "NetQuake server requires a Quake protocol, got version {}",
            other.version()
        ))),
    }
}

/// Codec limits for a profile (donor `createNetQuakeCodec`).
fn codec_limits(profile: NqProfile) -> (usize, usize) {
    match profile {
        NqProfile::Netquake => (NQ15_MAX_MSGLEN, NQ15_MAX_DATAGRAM),
        NqProfile::Fitzquake | NqProfile::Rmq { .. } => (WIDE_MAX_MSGLEN, WIDE_MAX_MSGLEN),
    }
}

/// One connected peer (donor `Peer`).
struct Q1Peer {
    remote: NetworkAddress,
    player: Q1ApplicationPlayer,
    channel: NetQuakeChannel,
    stage: u8,
    state: Q1ApplicationGameState,
    reliable: VecDeque<Vec<u8>>,
    last_received: u64,
    sequence: u32,
    pings: Vec<f64>,
}

/// NetQuake server network endpoint (`Q1ServerNetwork`).
pub struct Q1ServerNetwork<T, H> {
    transport: T,
    timeout_milliseconds: Option<u64>,
    host: H,
    profile: NqProfile,
    max_msglen: usize,
    max_datagram: usize,
    peers: HashMap<String, Q1Peer>,
    pending: Vec<ActorCommand>,
    source_seconds: f64,
    player_names: HashMap<u32, String>,
    ended: bool,
}

impl<T, H> Q1ServerNetwork<T, H>
where
    T: DatagramTransport<Address = NetworkAddress>,
    H: Q1ApplicationServerHost,
{
    /// Build a server endpoint.
    pub fn new(options: Q1ServerNetworkOptions<T, H>) -> Result<Self, Q1ServerError> {
        Self::validate(&options.host)?;
        let profile = nq_profile(options.host.protocol())?;
        let (max_msglen, max_datagram) = codec_limits(profile);
        Ok(Self {
            transport: options.transport,
            timeout_milliseconds: options.timeout_milliseconds,
            host: options.host,
            profile,
            max_msglen,
            max_datagram,
            peers: HashMap::new(),
            pending: Vec::new(),
            source_seconds: 0.0,
            player_names: HashMap::new(),
            ended: false,
        })
    }

    /// Reject hosts without a source wire binding.
    fn validate(host: &H) -> Result<(), Q1ServerError> {
        if let WireAdmission::Unsupported { reasons } = host.supports_source_wire() {
            return Err(Q1ServerError::Message(reasons.join("; ")));
        }
        Ok(())
    }

    /// Bound address.
    #[must_use]
    pub fn address(&self) -> NetworkAddress {
        self.transport.address()
    }

    /// Endpoint phase.
    #[must_use]
    pub fn phase(&self) -> ApplicationNetworkPhase {
        if self.ended {
            ApplicationNetworkPhase::Closed
        } else {
            ApplicationNetworkPhase::Active
        }
    }

    /// Connected players.
    #[must_use]
    pub fn clients(&self) -> Vec<Q1ApplicationPlayer> {
        self.peers.values().map(|peer| peer.player.clone()).collect()
    }

    /// Mean ping per client in milliseconds, truncated.
    #[must_use]
    pub fn client_pings(&self) -> HashMap<ClientId, u64> {
        self.peers
            .values()
            .map(|peer| {
                let mean = if peer.pings.is_empty() {
                    0.0
                } else {
                    peer.pings.iter().sum::<f64>() / peer.pings.len() as f64
                };
                (peer.player.client.clone(), mean.trunc() as u64)
            })
            .collect()
    }

    /// Encode messages, tracking player names (donor `bytes`).
    fn bytes(&mut self, messages: &[Q1ApplicationMessage]) -> Result<Vec<u8>, Q1ServerError> {
        for message in messages {
            if let NetQuakeMessage::NamedSlot {
                kind: NqNamedSlot::Name,
                slot,
                value,
            } = &message.message
            {
                self.player_names.insert(u32::from(*slot), value.clone());
            }
        }
        let mut writer = MsgWriter::new(self.max_msglen, false);
        for message in messages {
            write_net_quake_message(
                &mut writer,
                self.profile,
                &message.message,
                RereleaseMessages::KnownRetail,
                true,
            )?;
        }
        Ok(writer.bytes().to_vec())
    }

    /// Start signon for a peer.
    fn start(&mut self, peer: &mut Q1Peer) -> Result<(), Q1ServerError> {
        peer.stage = 1;
        let info = Q1ApplicationMessage {
            message: peer.state.info.message.clone(),
        };
        let view = Q1ApplicationMessage {
            message: NetQuakeMessage::SetView {
                entity: peer.player.source_entity as u16,
            },
        };
        let signon = Q1ApplicationMessage {
            message: NetQuakeMessage::Signon { stage: 1 },
        };
        let bytes = self.bytes(&[info, view, signon])?;
        peer.reliable.push_back(bytes);
        Ok(())
    }

    /// Carry peers across a world change (`changeWorld`).
    pub fn change_world(&mut self, host: H) -> Result<(), Q1ServerError> {
        Self::validate(&host)?;
        if !self.peers.is_empty()
            && (host.protocol().version() != self.host.protocol().version()
                || nq_protocol_flags(nq_profile(host.protocol())?) != nq_protocol_flags(self.profile))
        {
            return Err(Q1ServerError::Message(
                "Connected NetQuake peers require the same protocol across travel".to_string(),
            ));
        }
        self.host = host;
        self.profile = nq_profile(self.host.protocol())?;
        (self.max_msglen, self.max_datagram) = codec_limits(self.profile);
        self.pending.clear();
        let keys: Vec<String> = self.peers.keys().cloned().collect();
        for key in keys {
            let mut owned = self.peers.remove(&key).expect("peer present");
            owned.player = self.host.carried_player(&owned.player.client);
            owned.state = self.host.game_state(&owned.player);
            owned.reliable.clear();
            self.start(&mut owned)?;
            self.peers.insert(key, owned);
        }
        Ok(())
    }

    /// Disconnect a client.
    pub fn disconnect_client(&mut self, client: &ClientId, reason: &str) -> bool {
        let key = self
            .peers
            .iter()
            .find(|(_, peer)| peer.player.client == *client)
            .map(|(key, _)| key.clone());
        let Some(key) = key else { return false };
        if !self.peers.contains_key(&key) {
            return false;
        }
        let goodbye = Q1ApplicationMessage {
            message: NetQuakeMessage::Unit(NqUnit::Disconnect),
        };
        match self.bytes(std::slice::from_ref(&goodbye)) {
            Ok(bytes) => {
                if let Some(peer) = self.peers.get_mut(&key) {
                    if let Ok(packet) = peer.channel.unreliable(&bytes) {
                        let _ = self.transport.send(&peer.remote, &packet);
                    }
                }
            }
            Err(error) => self.host.print(&error.to_string()),
        }
        let Some(peer) = self.peers.remove(&key) else {
            return false;
        };
        self.pending.retain(|command| command.actor != peer.player.actor);
        self.host.disconnect(&peer.player, reason);
        true
    }

    /// Handle a client string command.
    fn command(&mut self, key: &str, text: &str) -> Result<(), Q1ServerError> {
        let mut parts = quake_world_command_arguments(text);
        if parts.is_empty() {
            return Ok(());
        }
        let name = parts.remove(0);
        if name == "ping" || name == "status" {
            let pings = self.client_pings();
            let peers: Vec<(ClientId, String, NetworkAddress)> = self
                .peers
                .values()
                .map(|other| {
                    let name = self
                        .player_names
                        .get(&other.player.client.slot())
                        .cloned()
                        .unwrap_or_else(|| "unconnected".to_string());
                    let name = if name.is_empty() {
                        "unconnected".to_string()
                    } else {
                        name
                    };
                    (other.player.client.clone(), name, other.remote.clone())
                })
                .collect();
            let text = if name == "ping" {
                let mut out = String::from("Client ping times:\n");
                for (client, name, _) in &peers {
                    out.push_str(&format!("{} {name}\n", pings.get(client).unwrap_or(&0)));
                }
                out
            } else {
                let mut out = format!(
                    "map: {}\nplayers: {} active ({} max)\n",
                    self.host.map_name(),
                    self.peers.len(),
                    self.host.max_clients()
                );
                for (client, name, remote) in &peers {
                    out.push_str(&format!(
                        "#{} {name} {}\n",
                        client.slot() + 1,
                        address_key(remote, true)
                    ));
                }
                out
            };
            let print = Q1ApplicationMessage {
                message: NetQuakeMessage::Text {
                    kind: NqText::Print,
                    text,
                },
            };
            let bytes = self.bytes(std::slice::from_ref(&print))?;
            if let Some(peer) = self.peers.get_mut(key) {
                peer.reliable.push_back(bytes);
            }
            return Ok(());
        }
        if name == "disconnect" {
            let client = self.peers.get(key).map(|peer| peer.player.client.clone());
            if let Some(client) = client {
                self.disconnect_client(&client, "Client disconnected");
            }
            return Ok(());
        }
        let stage = self.peers.get(key).map(|peer| peer.stage);
        if name == "prespawn" && stage == Some(1) {
            let Some(peer) = self.peers.get(key) else {
                return Ok(());
            };
            let mut messages = peer.state.signon.clone();
            let mut baselines: Vec<(&u32, &Q1WireEntity)> = peer.state.baselines.iter().collect();
            baselines.sort_by_key(|(number, _)| *number);
            messages.extend(baselines.into_iter().map(|(_, state)| Q1ApplicationMessage {
                message: NetQuakeMessage::Baseline { state: state.clone() },
            }));
            messages.push(Q1ApplicationMessage {
                message: NetQuakeMessage::Signon { stage: 2 },
            });
            let bytes = self.bytes(&messages)?;
            if let Some(peer) = self.peers.get_mut(key) {
                peer.reliable.push_back(bytes);
                peer.stage = 2;
            }
        } else if name == "spawn" && stage == Some(2) {
            let player = self.peers.get(key).map(|peer| peer.player.clone());
            let Some(player) = player else {
                return Ok(());
            };
            let mut messages = self.host.spawn(&player);
            messages.push(Q1ApplicationMessage {
                message: NetQuakeMessage::Signon { stage: 3 },
            });
            let bytes = self.bytes(&messages)?;
            if let Some(peer) = self.peers.get_mut(key) {
                peer.reliable.push_back(bytes);
                peer.stage = 3;
            }
        } else if name == "begin" && stage == Some(3) {
            if let Some(peer) = self.peers.get_mut(key) {
                peer.stage = 4;
            }
        } else if name != "prespawn" && name != "spawn" && name != "begin" {
            let player = self.peers.get(key).map(|peer| peer.player.clone());
            if let Some(player) = player {
                self.host.command(&player, &name, &parts);
            }
        }
        Ok(())
    }

    /// Handle one datagram (donor per-packet `try` body).
    fn handle_packet(&mut self, from: &NetworkAddress, payload: &[u8], now: u64) -> Result<(), Q1ServerError> {
        if payload.len() >= 4 && u32::from_be_bytes([payload[0], payload[1], payload[2], payload[3]]) >> 16 == 0x8000 {
            let mut control = ControlHost {
                server: self,
                error: None,
            };
            let response = answer_net_quake_control(payload, from, now as f64, &mut control)?;
            if let Some(error) = control.error {
                return Err(error);
            }
            if let Some(response) = response {
                control.server.transport.send(from, &response)?;
            }
            return Ok(());
        }
        let key = address_key(from, true);
        if !self.peers.contains_key(&key) {
            return Ok(());
        }
        let received = self
            .peers
            .get_mut(&key)
            .expect("peer checked")
            .channel
            .receive(payload, now as f64)?;
        if let Some(peer) = self.peers.get_mut(&key) {
            peer.last_received = now;
            for reply in &received.replies {
                self.transport.send(&peer.remote, reply)?;
            }
        }
        let Some(delivery) = received.delivery else {
            return Ok(());
        };
        let profile = self.profile;
        for message in decode_net_quake_client(&delivery.payload, profile)? {
            if !self.peers.contains_key(&key) {
                break;
            }
            match message {
                NetQuakeClientMessage::StringCommand { text } => self.command(&key, &text)?,
                NetQuakeClientMessage::Disconnect => {
                    let client = self.peers.get(&key).map(|peer| peer.player.client.clone());
                    if let Some(client) = client {
                        self.disconnect_client(&client, "Client disconnected");
                    }
                }
                NetQuakeClientMessage::Move { command } => {
                    let stage = self.peers.get(&key).map(|peer| peer.stage);
                    if stage != Some(4) {
                        continue;
                    }
                    let acknowledged = f64::from(command.acknowledged_server_time_seconds);
                    if let Some(peer) = self.peers.get_mut(&key) {
                        peer.pings.push((self.source_seconds - acknowledged).max(0.0) * 1000.0);
                        if peer.pings.len() > 16 {
                            peer.pings.remove(0);
                        }
                        let input = self
                            .host
                            .input(&peer.player.clone(), &to_q1_command(&command), peer.sequence);
                        peer.sequence += 1;
                        self.pending.push(input);
                    }
                }
                NetQuakeClientMessage::Nop => {}
            }
        }
        Ok(())
    }

    /// Poll the transport.
    pub fn poll(&mut self, now: u64) -> Result<Vec<ActorCommand>, Q1ServerError> {
        if self.ended {
            return Ok(Vec::new());
        }
        while let Some(packet) = self.transport.poll()? {
            let (from, payload) = match packet {
                ReceiveEvent::Packet { from, payload, .. } => (from, payload),
                ReceiveEvent::Error { error } => {
                    self.host.print(&error);
                    continue;
                }
                ReceiveEvent::Dropped { .. } => continue,
            };
            if self.host.rejects(&from) {
                continue;
            }
            if let Err(error) = self.handle_packet(&from, &payload, now) {
                let reason = error.to_string();
                let key = address_key(&from, true);
                let client = self.peers.get(&key).map(|peer| peer.player.client.clone());
                if let Some(client) = client {
                    self.disconnect_client(&client, &reason);
                }
                self.host.print(&reason);
            }
        }
        let keys: Vec<String> = self.peers.keys().cloned().collect();
        for key in keys {
            let timed_out = self.peers.get(&key).is_some_and(|peer| {
                now.saturating_sub(peer.last_received) > self.timeout_milliseconds.unwrap_or(65_000)
            });
            if timed_out {
                let client = self.peers.get(&key).map(|peer| peer.player.client.clone());
                if let Some(client) = client {
                    self.disconnect_client(&client, "Connection timed out");
                }
                continue;
            }
            let can_send = self.peers.get(&key).map(|peer| peer.channel.can_send_reliable());
            if can_send == Some(true) {
                let bytes = self.peers.get_mut(&key).and_then(|peer| peer.reliable.pop_front());
                if let Some(bytes) = bytes {
                    if let Some(peer) = self.peers.get_mut(&key) {
                        peer.channel.queue_reliable(&bytes)?;
                    }
                }
            }
            let packet = self
                .peers
                .get_mut(&key)
                .map(|peer| peer.channel.next(now as f64))
                .transpose()?
                .flatten();
            if let Some(packet) = packet {
                let remote = self.peers.get(&key).map(|peer| peer.remote.clone());
                if let Some(remote) = remote {
                    self.transport.send(&remote, &packet)?;
                }
            }
        }
        Ok(std::mem::take(&mut self.pending))
    }

    /// Server input must enter the application input batch.
    pub fn submit(&mut self, _commands: &[ActorCommand], _now: u64) -> Result<(), Q1ServerError> {
        Err(Q1ServerError::Message(
            "Server input must enter the application input batch".to_string(),
        ))
    }

    /// Publish a completed simulation step.
    pub fn publish(
        &mut self,
        output: &SimulationOutput,
        events: &[NetworkPresentationEvent],
        now: u64,
    ) -> Result<(), Q1ServerError> {
        if self.ended {
            return Ok(());
        }
        self.host.observe(output, events);
        self.source_seconds = output.snapshot.frame.time.as_seconds_f64();
        let keys: Vec<String> = self.peers.keys().cloned().collect();
        for key in keys {
            let player = self.peers.get(&key).map(|peer| peer.player.clone());
            let Some(player) = player else { continue };
            let frame = self.host.frame(&player, output);
            if !frame.reliable.is_empty() {
                let bytes = self.bytes(&frame.reliable)?;
                if let Some(peer) = self.peers.get_mut(&key) {
                    peer.reliable.push_back(bytes);
                }
            }
            let can_send = self
                .peers
                .get(&key)
                .map(|peer| peer.channel.can_send_reliable())
                .unwrap_or(false);
            if can_send {
                let bytes = self.peers.get_mut(&key).and_then(|peer| peer.reliable.pop_front());
                if let Some(bytes) = bytes {
                    if let Some(peer) = self.peers.get_mut(&key) {
                        peer.channel.queue_reliable(&bytes)?;
                    }
                }
            }
            let packet = self
                .peers
                .get_mut(&key)
                .map(|peer| peer.channel.next(now as f64))
                .transpose()?
                .flatten();
            if let Some(packet) = packet {
                let remote = self.peers.get(&key).map(|peer| peer.remote.clone());
                if let Some(remote) = remote {
                    self.transport.send(&remote, &packet)?;
                }
            }
            let stage = self.peers.get(&key).map(|peer| peer.stage);
            if stage != Some(4) {
                continue;
            }
            let max_datagram = self
                .max_datagram
                .min(self.transport.max_datagram_bytes().unwrap_or(65507).saturating_sub(8));
            let mut buffer = MsgWriter::new(max_datagram, false);
            write_net_quake_message(
                &mut buffer,
                self.profile,
                &NetQuakeMessage::Time {
                    seconds: frame.seconds as f32,
                },
                RereleaseMessages::KnownRetail,
                true,
            )?;
            for message in &frame.messages {
                write_net_quake_message(
                    &mut buffer,
                    self.profile,
                    &message.message,
                    RereleaseMessages::KnownRetail,
                    true,
                )?;
            }
            let baselines = self
                .peers
                .get(&key)
                .map(|peer| peer.state.baselines.clone())
                .unwrap_or_default();
            for state in &frame.entities {
                let mut encoded = MsgWriter::new(128, false);
                let baseline = baselines.get(&state.number).cloned().unwrap_or(Q1WireEntity {
                    number: state.number,
                    ..Q1WireEntity::default()
                });
                write_net_quake_entity(&mut encoded, self.profile, state, &baseline, frame.seconds)?;
                if buffer.cursize() + encoded.cursize() > max_datagram {
                    break;
                }
                buffer.write_bytes(encoded.bytes())?;
            }
            let mut datagram = MsgWriter::new(max_datagram, false);
            for message in &frame.datagram {
                let encoded = self.bytes(std::slice::from_ref(message))?;
                if datagram.cursize() + encoded.len() > max_datagram {
                    break;
                }
                datagram.write_bytes(&encoded)?;
            }
            if buffer.cursize() + datagram.cursize() <= max_datagram {
                buffer.write_bytes(datagram.bytes())?;
            }
            let remote = self.peers.get(&key).map(|peer| peer.remote.clone());
            if let Some(remote) = remote {
                let packet = self
                    .peers
                    .get_mut(&key)
                    .map(|peer| peer.channel.unreliable(buffer.bytes()))
                    .transpose()?;
                if let Some(packet) = packet {
                    self.transport.send(&remote, &packet)?;
                }
            }
        }
        Ok(())
    }

    /// Shut down.
    pub fn close(&mut self) {
        if self.ended {
            return;
        }
        for player in self.clients() {
            self.disconnect_client(&player.client, "Server shutdown");
        }
        self.ended = true;
        self.transport.close();
    }
}

/// Convert a wire move into an application command.
fn to_q1_command(command: &NqUserCommand) -> Q1UserCommand {
    Q1UserCommand {
        acknowledged_server_time_seconds: f64::from(command.acknowledged_server_time_seconds),
        view_angles: qa_core::math::Vec3 {
            x: command.view_angles[0] as f32,
            y: command.view_angles[1] as f32,
            z: command.view_angles[2] as f32,
        },
        forward_move: f64::from(command.forward_move),
        side_move: f64::from(command.side_move),
        up_move: f64::from(command.up_move),
        buttons: i32::from(command.buttons),
        impulse: i32::from(command.impulse),
    }
}

/// Connectionless control adapter (donor `answerNetQuakeControl` closures).
struct ControlHost<'a, T, H> {
    server: &'a mut Q1ServerNetwork<T, H>,
    error: Option<Q1ServerError>,
}

impl<T, H> NetQuakeConnectionHost for ControlHost<'_, T, H>
where
    T: DatagramTransport<Address = NetworkAddress>,
    H: Q1ApplicationServerHost,
{
    fn server_info(&self) -> NetQuakeControl {
        NetQuakeControl::ServerInfo {
            address: address_key(&self.server.address(), true),
            name: "Quake".to_string(),
            map: self.server.host.map_name(),
            players: self.server.peers.len().min(255) as u8,
            max_players: self.server.host.max_clients().min(255) as u8,
            version: 3,
        }
    }

    fn player_info(&self, _index: u8) -> Option<NetQuakeControl> {
        None
    }

    fn next_rule(&self, _previous: &str) -> Option<(String, String)> {
        None
    }

    fn connect(&mut self, from: &NetworkAddress, _now: f64) -> NetQuakeConnectVerdict {
        if self.server.address().kind() == "loopback" {
            return NetQuakeConnectVerdict::Rejected {
                reason: "NetQuake control requires an IP port".to_string(),
            };
        }
        if self.server.peers.contains_key(&address_key(from, true)) {
            return NetQuakeConnectVerdict::Accepted {
                port: self.server.address().port().unwrap_or(0) as i32,
            };
        }
        let admitted = self.server.host.admit(from);
        let Q1ApplicationAdmission::Accepted { player } = admitted else {
            let Q1ApplicationAdmission::Rejected { reason } = admitted else {
                unreachable!("admission is exhaustive");
            };
            return NetQuakeConnectVerdict::Rejected { reason };
        };
        let built = (|| -> Result<(), Q1ServerError> {
            let state = self.server.host.game_state(&player);
            let max_msglen = self.server.max_msglen;
            let mut peer = Q1Peer {
                remote: from.clone(),
                player,
                channel: NetQuakeChannel::new(max_msglen, 1024)?,
                stage: 1,
                state,
                reliable: VecDeque::new(),
                last_received: 0,
                sequence: 0,
                pings: Vec::new(),
            };
            // `start` borrows the server while `peer` is local: no alias.
            let info = Q1ApplicationMessage {
                message: peer.state.info.message.clone(),
            };
            let view = Q1ApplicationMessage {
                message: NetQuakeMessage::SetView {
                    entity: peer.player.source_entity as u16,
                },
            };
            let signon = Q1ApplicationMessage {
                message: NetQuakeMessage::Signon { stage: 1 },
            };
            let bytes = self.server.bytes(&[info, view, signon])?;
            peer.stage = 1;
            peer.reliable.push_back(bytes);
            self.server.peers.insert(address_key(from, true), peer);
            Ok(())
        })();
        match built {
            Ok(()) => NetQuakeConnectVerdict::Accepted {
                port: self.server.address().port().unwrap_or(0) as i32,
            },
            Err(error) => {
                // Mirror the donor: the peer never joined, so stash the
                // failure for the per-packet handler (which prints it)
                // instead of answering.
                self.error = Some(error);
                NetQuakeConnectVerdict::Retry
            }
        }
    }
}

impl<T, H> ApplicationNetwork for Q1ServerNetwork<T, H>
where
    T: DatagramTransport<Address = NetworkAddress>,
    H: Q1ApplicationServerHost,
{
    fn role(&self) -> ApplicationNetworkRole {
        ApplicationNetworkRole::Server
    }

    fn phase(&self) -> ApplicationNetworkPhase {
        self.phase()
    }

    fn wire(&self) -> WireSelection {
        WireSelection::Source {
            protocol: self.host.protocol(),
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
    use qa_core::identity::IdentityOwner;
    use qa_net::common::transport::{DatagramLimits, PacketQueue};
    use qa_net::q1_wide::NqProfile;
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
        owner: IdentityOwner,
        printed: Vec<String>,
    }

    impl Q1ApplicationServerHost for ScriptHost {
        fn protocol(&self) -> ProtocolIdentity {
            ProtocolIdentity::Q1Netquake
        }

        fn max_clients(&self) -> u32 {
            8
        }

        fn map_name(&self) -> String {
            "start".to_string()
        }

        fn supports_source_wire(&self) -> WireAdmission {
            WireAdmission::Supported
        }

        fn admit(&mut self, _from: &NetworkAddress) -> Q1ApplicationAdmission {
            Q1ApplicationAdmission::Accepted {
                player: Q1ApplicationPlayer {
                    client: self.owner.client(0, 0),
                    actor: self.owner.actor(1, 0),
                    source_entity: 1,
                },
            }
        }

        fn carried_player(&self, client: &ClientId) -> Q1ApplicationPlayer {
            Q1ApplicationPlayer {
                client: client.clone(),
                actor: self.owner.actor(1, 0),
                source_entity: 1,
            }
        }

        fn disconnect(&mut self, _player: &Q1ApplicationPlayer, _reason: &str) {}

        fn game_state(&self, _player: &Q1ApplicationPlayer) -> Q1ApplicationGameState {
            Q1ApplicationGameState {
                info: super::super::q1_types::Q1ServerInfo {
                    message: NetQuakeMessage::ServerInfo {
                        protocol: NqProfile::Netquake,
                        max_clients: 8,
                        game_type: 0,
                        level: "start".to_string(),
                        models: Vec::new(),
                        sounds: Vec::new(),
                    },
                },
                baselines: HashMap::new(),
                signon: Vec::new(),
            }
        }

        fn spawn(&mut self, _player: &Q1ApplicationPlayer) -> Vec<Q1ApplicationMessage> {
            Vec::new()
        }

        fn frame(
            &self,
            _player: &Q1ApplicationPlayer,
            _output: &SimulationOutput,
        ) -> super::super::q1_types::Q1ApplicationFrame {
            super::super::q1_types::Q1ApplicationFrame {
                seconds: 0.0,
                messages: Vec::new(),
                reliable: Vec::new(),
                datagram: Vec::new(),
                entities: Vec::new(),
            }
        }

        fn input(&mut self, player: &Q1ApplicationPlayer, command: &Q1UserCommand, sequence: u32) -> ActorCommand {
            ActorCommand {
                actor: player.actor.clone(),
                source: qa_net::common::commands::CommandSource::Remote {
                    client: player.client.clone(),
                },
                sequence: u64::from(sequence),
                command: qa_net::common::commands::UserCommand::Q1Netquake {
                    acknowledged_server_time_seconds: command.acknowledged_server_time_seconds,
                    view_angles: [
                        f64::from(command.view_angles.x),
                        f64::from(command.view_angles.y),
                        f64::from(command.view_angles.z),
                    ],
                    forward_move: command.forward_move,
                    side_move: command.side_move,
                    up_move: command.up_move,
                    buttons: f64::from(command.buttons),
                    impulse: f64::from(command.impulse),
                },
                arsenal: None,
            }
        }

        fn command(&mut self, _player: &Q1ApplicationPlayer, _name: &str, _args: &[String]) {}

        fn observe(&mut self, _output: &SimulationOutput, _events: &[NetworkPresentationEvent]) {}

        fn print(&mut self, text: &str) {
            self.printed.push(text.to_string());
        }
    }

    fn address() -> NetworkAddress {
        NetworkAddress::Ipv4 {
            host: [127, 0, 0, 1],
            port: 26000,
        }
    }

    fn server() -> Q1ServerNetwork<LoopTransport, ScriptHost> {
        let queue = PacketQueue::new(
            DatagramLimits {
                max_bytes: 65536,
                queue_packets: 16,
            },
            qa_net::common::transport::monotonic_clock(),
        )
        .expect("queue");
        Q1ServerNetwork::new(Q1ServerNetworkOptions {
            transport: LoopTransport {
                queue,
                address: address(),
            },
            host: ScriptHost {
                owner: IdentityOwner::create("q1-server-test").expect("owner"),
                printed: Vec::new(),
            },
            timeout_milliseconds: None,
        })
        .expect("server")
    }

    #[test]
    fn rejects_hosts_without_source_wire() {
        struct Refusing;
        impl Q1ApplicationServerHost for Refusing {
            fn protocol(&self) -> ProtocolIdentity {
                ProtocolIdentity::Q1Netquake
            }
            fn max_clients(&self) -> u32 {
                8
            }
            fn map_name(&self) -> String {
                "start".to_string()
            }
            fn supports_source_wire(&self) -> WireAdmission {
                WireAdmission::Unsupported {
                    reasons: vec!["nope".to_string()],
                }
            }
            fn admit(&mut self, _from: &NetworkAddress) -> Q1ApplicationAdmission {
                unreachable!()
            }
            fn carried_player(&self, _client: &ClientId) -> Q1ApplicationPlayer {
                unreachable!()
            }
            fn disconnect(&mut self, _player: &Q1ApplicationPlayer, _reason: &str) {}
            fn game_state(&self, _player: &Q1ApplicationPlayer) -> Q1ApplicationGameState {
                unreachable!()
            }
            fn spawn(&mut self, _player: &Q1ApplicationPlayer) -> Vec<Q1ApplicationMessage> {
                unreachable!()
            }
            fn frame(
                &self,
                _player: &Q1ApplicationPlayer,
                _output: &SimulationOutput,
            ) -> super::super::q1_types::Q1ApplicationFrame {
                unreachable!()
            }
            fn input(
                &mut self,
                _player: &Q1ApplicationPlayer,
                _command: &Q1UserCommand,
                _sequence: u32,
            ) -> ActorCommand {
                unreachable!()
            }
            fn command(&mut self, _player: &Q1ApplicationPlayer, _name: &str, _args: &[String]) {}
            fn observe(&mut self, _output: &SimulationOutput, _events: &[NetworkPresentationEvent]) {}
            fn print(&mut self, _text: &str) {}
        }
        let queue = PacketQueue::new(
            DatagramLimits {
                max_bytes: 65536,
                queue_packets: 16,
            },
            qa_net::common::transport::monotonic_clock(),
        )
        .expect("queue");
        let result = Q1ServerNetwork::new(Q1ServerNetworkOptions {
            transport: LoopTransport {
                queue,
                address: address(),
            },
            host: Refusing,
            timeout_milliseconds: None,
        });
        let Err(error) = result else {
            panic!("unsupported host must fail");
        };
        assert_eq!(error.to_string(), "nope");
    }

    #[test]
    fn profiles_select_codec_limits() {
        assert_eq!(codec_limits(NqProfile::Netquake), (8000, 1024));
        assert_eq!(codec_limits(NqProfile::Fitzquake), (64000, 64000));
        assert!(nq_profile(ProtocolIdentity::Q2Classic).is_err());
    }

    #[test]
    fn tracks_names_and_pings() {
        let mut server = server();
        let named = Q1ApplicationMessage {
            message: NetQuakeMessage::NamedSlot {
                kind: NqNamedSlot::Name,
                slot: 3,
                value: "alice".to_string(),
            },
        };
        server.bytes(std::slice::from_ref(&named)).expect("bytes");
        assert_eq!(server.player_names.get(&3).map(String::as_str), Some("alice"));
        assert!(server.client_pings().is_empty());
        assert_eq!(server.phase(), ApplicationNetworkPhase::Active);
        assert!(server.poll(0).expect("poll").is_empty());
        assert!(server.submit(&[], 0).is_err());
    }

    #[test]
    fn unknown_client_disconnect_is_quiet() {
        let mut server = server();
        let owner = IdentityOwner::create("q1-ghost").expect("owner");
        assert!(!server.disconnect_client(&owner.client(9, 0), "bye"));
    }
}
