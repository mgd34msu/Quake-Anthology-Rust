//! Unified authoritative server endpoint.
//!
//! Port of `/home/buzzkill/Projects/quake-typescript/src/app/bootstrap/network/unified-server.ts`
//! (`UnifiedServerNetwork`). Admission, input collection with NetQuake
//! burst merging, and per-peer publication. The donor wraps host calls
//! so host failures abort the poll while channel failures drop the
//! peer; the sync port matches that by letting host panics propagate
//! (host bugs) while mapping every channel and decode failure onto
//! [`drop`](UnifiedServerNetwork::disconnect_client)-style peer drops.
//!
//! Two donor behaviors narrow to fit ported lane types. The NetQuake
//! burst merge keeps provider, weapon, and holdable selection but not
//! the session arsenal impulse, because the ported `ArsenalIntent` wire
//! format has no impulse field. Connection tokens hash process, time,
//! and counter material (see [`super::unified_client`]) instead of OS
//! randomness, which the workspace does not provide.

use std::collections::{HashMap, HashSet};

use qa_content::contract::PresentationOwner;
use qa_core::identity::{ActorId, ClientId};
use qa_net::common::commands::{ActorCommand, ArsenalIntent, UserCommand};
use qa_net::common::endpoint::{address_key, same_address, NetworkAddress};
use qa_net::common::session::WireSelection;
use qa_net::common::transport::{DatagramTransport, ReceiveEvent};
use qa_net::unified::{decode_unified_packet, UnifiedChannel, UnifiedChannelLimits, UnifiedDelivery};
use qa_world::session::SimulationOutput;

use super::types::{
    ApplicationNetwork, ApplicationNetworkError, ApplicationNetworkPhase, ApplicationNetworkRole,
    NetworkPresentationEvent,
};
use super::unified_client::wire_digest;
use super::unified_component_publication::UnifiedComponentPublisher;
use super::unified_components::UnifiedComponentPublication;
use super::unified_content::{unified_resource_id, UnifiedCompositionIdentity};
use super::unified_control::{
    decode_unified_control, decode_unified_handshake, decode_unified_inputs, encode_unified_control,
    encode_unified_handshake, UnifiedActorReference, UnifiedArsenalIntent, UnifiedControl, UnifiedHandshake,
    UnifiedServerMode,
};
use super::unified_event_codec::{
    encode_unified_presentation_events, write_unified_simulation_event, UnifiedPresentationEvent,
};
use super::unified_frame_codec::{encode_unified_frame, UnifiedResourceKey};
use super::unified_native_components::UnifiedNativePublication;
use qa_world::save::value::{arr, encode_checkpoint_value};

/// Authenticated server player (donor `UnifiedApplicationPlayer`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnifiedApplicationPlayer {
    /// Player actor.
    pub actor: ActorId,
    /// Owning client.
    pub client: ClientId,
    /// Source entity number.
    pub source_entity: i64,
}

/// Admission decision (donor `host.admit` result).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UnifiedAdmission {
    /// Admit with a bound player.
    Accepted(UnifiedApplicationPlayer),
    /// Reject with a reason.
    Rejected {
        /// Rejection reason.
        reason: String,
    },
}

/// Authoritative server host (donor `UnifiedApplicationServerHost`).
pub trait UnifiedServerHost {
    /// Server mode.
    fn mode(&self) -> UnifiedServerMode;
    /// Maximum clients.
    fn max_clients(&self) -> usize;
    /// Admit a ready peer.
    fn admit(&mut self, address: &NetworkAddress, userinfo: &str) -> UnifiedAdmission;
    /// Handle a disconnect.
    fn disconnect(&mut self, player: &UnifiedApplicationPlayer);
    /// Handle a userinfo update.
    fn userinfo(&mut self, player: &UnifiedApplicationPlayer, userinfo: &str);
    /// Initial presentation for an admitted player.
    fn initial_presentation(&mut self, player: &UnifiedApplicationPlayer) -> Vec<UnifiedPresentationEvent>;
    /// Convert one wire input into an actor command.
    fn input(
        &mut self,
        player: &UnifiedApplicationPlayer,
        sequence: i64,
        command: &UserCommand,
        arsenal: Option<&UnifiedArsenalIntent>,
    ) -> ActorCommand;
    /// Run a player command.
    fn command(&mut self, player: &UnifiedApplicationPlayer, name: &str, args: &[String]);
    /// Run a component command (`None` means unimplemented).
    fn component_command(
        &mut self,
        player: &UnifiedApplicationPlayer,
        owner: &PresentationOwner,
        generation: i64,
        args: &[String],
    ) -> Option<bool> {
        let _ = (player, owner, generation, args);
        None
    }
    /// Resource keys for a published output.
    fn resources(&mut self, player: &UnifiedApplicationPlayer, output: &SimulationOutput) -> Vec<UnifiedResourceKey>;
    /// Presentation events for a published output.
    fn presentation_events(
        &mut self,
        player: &UnifiedApplicationPlayer,
        events: &[NetworkPresentationEvent],
    ) -> Vec<UnifiedPresentationEvent>;
    /// Frame for a published output.
    fn frame(
        &mut self,
        player: &UnifiedApplicationPlayer,
        output: &SimulationOutput,
        epoch: u64,
        acknowledged: i64,
    ) -> super::unified_types::UnifiedPresentationFrame;
    /// Gameplay component publications.
    fn components(&mut self, player: &UnifiedApplicationPlayer) -> Vec<UnifiedComponentPublication> {
        let _ = player;
        Vec::new()
    }
    /// Native component publications.
    fn native_components(&mut self, player: &UnifiedApplicationPlayer) -> Vec<UnifiedNativePublication> {
        let _ = player;
        Vec::new()
    }
    /// Carry a player across a world change, if the client survives it.
    fn carried_player(&mut self, client: &ClientId) -> Option<UnifiedApplicationPlayer>;
}

/// Unified server options (donor `UnifiedServerOptions`).
pub struct UnifiedServerOptions<T, H, P> {
    /// Datagram transport.
    pub transport: T,
    /// Authoritative host.
    pub host: H,
    /// Served composition.
    pub composition: UnifiedCompositionIdentity,
    /// Log sink.
    pub print: P,
}

/// Handshake peer awaiting connect (donor `PendingPeer`).
struct PendingPeer {
    address: NetworkAddress,
    nonce: String,
    token: String,
    created: u64,
}

/// Established peer (donor `Peer`).
struct UnifiedPeer {
    address: NetworkAddress,
    token: String,
    nonce: String,
    channel: UnifiedChannel,
    resources: HashSet<qa_content::contract::ResourceId>,
    player: Option<UnifiedApplicationPlayer>,
    ready: bool,
    last_received: u64,
    last_queued_input: i64,
    last_consumed_input: i64,
    last_submitted_input: i64,
    required_reliable: u32,
    components: UnifiedComponentPublisher,
    closing: Option<u64>,
}

/// Unified authoritative server (donor `UnifiedServerNetwork`).
pub struct UnifiedServerNetwork<T, H, P> {
    transport: T,
    host: H,
    print: P,
    composition: UnifiedCompositionIdentity,
    epoch: u32,
    now: u64,
    ended: bool,
    token_counter: u64,
    pending: HashMap<String, PendingPeer>,
    peers: HashMap<String, UnifiedPeer>,
    commands: Vec<ActorCommand>,
}

fn network_error(message: impl Into<String>) -> ApplicationNetworkError {
    ApplicationNetworkError::Message(message.into())
}

impl<T, H, P> UnifiedServerNetwork<T, H, P>
where
    T: DatagramTransport<Address = NetworkAddress>,
    H: UnifiedServerHost,
    P: FnMut(&str),
{
    /// Wrap server options.
    pub fn new(options: UnifiedServerOptions<T, H, P>) -> Self {
        Self {
            transport: options.transport,
            host: options.host,
            print: options.print,
            composition: options.composition,
            epoch: 1,
            now: 0,
            ended: false,
            token_counter: 0,
            pending: HashMap::new(),
            peers: HashMap::new(),
            commands: Vec::new(),
        }
    }

    /// Borrow the authoritative host.
    #[must_use]
    pub fn host(&self) -> &H {
        &self.host
    }

    /// Admitted players.
    #[must_use]
    pub fn clients(&self) -> Vec<UnifiedApplicationPlayer> {
        self.peers.values().filter_map(|peer| peer.player.clone()).collect()
    }

    fn log(&mut self, text: &str) {
        (self.print)(text);
    }

    fn queue(peer: &mut UnifiedPeer, control: &UnifiedControl) -> Result<(), qa_net::unified::UnifiedError> {
        let sequence = peer.channel.queue_reliable(&encode_unified_control(control))?;
        peer.required_reliable = sequence;
        Ok(())
    }

    fn offer(&mut self, token: &str) -> Result<(), ApplicationNetworkError> {
        let Some(peer) = self.peers.get_mut(token) else {
            return Ok(());
        };
        peer.components = UnifiedComponentPublisher::default();
        peer.ready = false;
        peer.resources.clear();
        peer.last_queued_input = -1;
        peer.last_consumed_input = -1;
        peer.last_submitted_input = -1;
        let control = UnifiedControl::Offer {
            epoch: u64::from(self.epoch),
            composition: self.composition.clone(),
            mode: self.host.mode(),
            max_clients: self.host.max_clients() as i64,
        };
        if let Some(peer) = self.peers.get_mut(token) {
            Self::queue(peer, &control).map_err(|error| network_error(error.to_string()))?;
        }
        Ok(())
    }

    fn flush_peer(transport: &T, peer: &mut UnifiedPeer, now: u64) -> Result<(), ApplicationNetworkError> {
        let datagrams = peer
            .channel
            .flush(now as f64)
            .map_err(|error| network_error(error.to_string()))?;
        for bytes in datagrams {
            match transport.send(&peer.address, &bytes) {
                Ok(true) => {}
                Ok(false) => {
                    return Err(network_error("Unified transport rejected an outgoing datagram"));
                }
                Err(error) => return Err(network_error(error.to_string())),
            }
        }
        Ok(())
    }

    /// Drop a peer (donor `drop`).
    fn drop_peer(&mut self, token: &str, reason: &str, now: u64) {
        let closing = self.peers.get(token).and_then(|peer| peer.closing);
        if closing.is_some() {
            return;
        }
        let player = self.peers.get_mut(token).and_then(|peer| {
            peer.closing = Some(now);
            peer.ready = false;
            peer.player.take()
        });
        if let Some(player) = player {
            self.commands.retain(|command| command.actor != player.actor);
            self.host.disconnect(&player);
        }
        let address = self
            .peers
            .get(token)
            .map(|peer| address_key(&peer.address, true))
            .unwrap_or_default();
        self.log(&format!("Unified peer {address} disconnected: {reason}\n"));
        let control = UnifiedControl::Disconnect {
            reason: reason.chars().take(1024).collect(),
        };
        let flushed = self
            .peers
            .get_mut(token)
            .is_some_and(|peer| Self::queue(peer, &control).is_ok())
            && self
                .peers
                .get_mut(token)
                .is_some_and(|peer| Self::flush_peer(&self.transport, peer, now).is_ok());
        // Borrow the transport once for the flush above; on failure the
        // peer is closed and removed immediately.
        if !flushed {
            if let Some(peer) = self.peers.get_mut(token) {
                peer.channel.close();
            }
            self.peers.remove(token);
        }
    }

    /// Disconnect one client (donor `disconnectClient`).
    pub fn disconnect_client(&mut self, client: &ClientId, reason: &str) -> bool {
        let token = self
            .peers
            .values()
            .find(|peer| peer.player.as_ref().is_some_and(|player| player.client == *client))
            .map(|peer| peer.token.clone());
        match token {
            Some(token) => {
                let now = self.now;
                self.drop_peer(&token, reason, now);
                true
            }
            None => false,
        }
    }
}

impl<T, H, P> UnifiedServerNetwork<T, H, P>
where
    T: DatagramTransport<Address = NetworkAddress>,
    H: UnifiedServerHost,
    P: FnMut(&str),
{
    /// Handle one reliable control (donor `acceptControl`).
    fn accept_control(&mut self, token: &str, bytes: &[u8], now: u64) -> Result<(), ApplicationNetworkError> {
        let control = decode_unified_control(bytes).map_err(|error| network_error(error.to_string()))?;
        if let UnifiedControl::Disconnect { reason } = &control {
            self.drop_peer(token, reason, now);
            return Ok(());
        }
        if control_epoch(&control) != u64::from(self.epoch) {
            return Ok(());
        }
        if let UnifiedControl::Ready {
            composition, userinfo, ..
        } = &control
        {
            if *composition != self.composition.digest.to_string() {
                return Err(network_error("Client content composition differs from the server"));
            }
            let Self { peers, host, .. } = self;
            let Some(peer) = peers.get_mut(token) else {
                return Ok(());
            };
            if peer.ready {
                return Ok(());
            }
            if peer.player.is_none() {
                match host.admit(&peer.address, userinfo) {
                    UnifiedAdmission::Accepted(player) => peer.player = Some(player),
                    UnifiedAdmission::Rejected { reason } => return Err(network_error(reason)),
                }
            } else if let Some(player) = peer.player.clone() {
                host.userinfo(&player, userinfo);
            }
            let player = peer.player.clone().expect("player is admitted");
            Self::queue(
                peer,
                &UnifiedControl::Admitted {
                    epoch: control_epoch(&control),
                    client: UnifiedActorReference {
                        slot: i64::from(player.client.slot()),
                        generation: i64::from(player.client.generation()),
                    },
                    actor: UnifiedActorReference {
                        slot: i64::from(player.actor.slot()),
                        generation: i64::from(player.actor.generation()),
                    },
                    source_entity: player.source_entity,
                },
            )
            .map_err(|error| network_error(error.to_string()))?;
            let initial = host.initial_presentation(&player);
            if !initial.is_empty() {
                let payload =
                    encode_unified_presentation_events(&initial).map_err(|error| network_error(error.to_string()))?;
                Self::queue(
                    peer,
                    &UnifiedControl::Events {
                        epoch: control_epoch(&control),
                        frame: 0,
                        payload,
                        simulation: encode_checkpoint_value(&arr(Vec::new())),
                    },
                )
                .map_err(|error| network_error(error.to_string()))?;
            }
            peer.ready = true;
            return Ok(());
        }
        let admitted = self
            .peers
            .get(token)
            .is_some_and(|peer| peer.player.is_some() && peer.ready);
        if !admitted {
            return Err(network_error("Client command arrived before world admission"));
        }
        let Self { peers, host, .. } = self;
        let Some(peer) = peers.get_mut(token) else {
            return Ok(());
        };
        let player = peer.player.clone().expect("player is admitted");
        match &control {
            UnifiedControl::ComponentCommand {
                owner,
                generation,
                args,
                ..
            } => match host.component_command(&player, owner, *generation, args) {
                None => {
                    return Err(network_error("No component command receiver"));
                }
                Some(false) => {
                    return Err(network_error("Component command source or recipient is not admitted"));
                }
                Some(true) => {}
            },
            UnifiedControl::Command { name, args, .. } => host.command(&player, name, args),
            UnifiedControl::Userinfo { value, .. } => host.userinfo(&player, value),
            _ => {
                return Err(network_error("Client sent a server-only control message"));
            }
        }
        let _ = peer;
        Ok(())
    }

    /// Handle one input batch (donor `acceptInputs`).
    fn accept_inputs(&mut self, token: &str, bytes: &[u8]) {
        let batch = decode_unified_inputs(bytes);
        let Ok(batch) = batch else {
            return;
        };
        if batch.epoch != u64::from(self.epoch) {
            return;
        }
        let Self {
            peers, host, commands, ..
        } = self;
        let Some(peer) = peers.get_mut(token) else {
            return;
        };
        let Some(player) = peer.player.clone() else {
            return;
        };
        if !peer.ready {
            return;
        }
        for input in &batch.commands {
            if input.sequence <= peer.last_queued_input {
                continue;
            }
            let command = host.input(&player, input.sequence, &input.command, input.arsenal.as_ref());
            commands.push(command);
            peer.last_queued_input = input.sequence;
        }
    }

    /// Poll the transport (donor `poll`).
    pub fn poll(&mut self, now: u64) -> Result<Vec<ActorCommand>, ApplicationNetworkError> {
        if self.ended {
            return Ok(Vec::new());
        }
        self.now = now;
        self.pending
            .retain(|_, pending| now.saturating_sub(pending.created) <= 10000);
        for _ in 0..4096 {
            let event = self
                .transport
                .poll()
                .map_err(|error| network_error(error.to_string()))?;
            let Some(event) = event else {
                break;
            };
            match event {
                ReceiveEvent::Error { error } => {
                    self.log(&format!("Unified transport: {error}\n"));
                }
                ReceiveEvent::Packet { from, payload, .. } => {
                    self.receive_packet(&from, &payload, now)?;
                }
                _ => {}
            }
        }
        let tokens: Vec<String> = self.peers.keys().cloned().collect();
        for token in tokens {
            let Some(peer) = self.peers.get(&token) else {
                continue;
            };
            if let Some(closing) = peer.closing {
                if now.saturating_sub(closing) > 1000
                    || peer.channel.acknowledged_reliable_sequence() >= peer.required_reliable
                {
                    if let Some(peer) = self.peers.get_mut(&token) {
                        peer.channel.close();
                    }
                    self.peers.remove(&token);
                }
                continue;
            }
            if now.saturating_sub(peer.last_received) > 30000 {
                self.drop_peer(&token, "Connection timed out", now);
                continue;
            }
            let failed: Option<String> = match self.peers.get_mut(&token) {
                Some(peer) => match Self::flush_peer(&self.transport, peer, now) {
                    Ok(()) => None,
                    Err(error) => Some(error.to_string()),
                },
                None => None,
            };
            if let Some(reason) = failed {
                self.drop_peer(&token, &reason, now);
            }
        }
        Ok(self.drain_commands())
    }

    /// Handle one datagram (donor packet/handshake dispatch).
    fn receive_packet(
        &mut self,
        from: &NetworkAddress,
        payload: &[u8],
        now: u64,
    ) -> Result<(), ApplicationNetworkError> {
        if let Some(packet) = decode_unified_packet(payload) {
            let token = packet_token(&packet).to_string();
            let known = self
                .peers
                .get(&token)
                .is_some_and(|peer| same_address(&peer.address, from, true));
            if !known {
                return Ok(());
            }
            if let Some(peer) = self.peers.get_mut(&token) {
                peer.last_received = now;
            }
            let deliveries = self
                .peers
                .get_mut(&token)
                .expect("peer is known")
                .channel
                .receive(payload, now as f64);
            let deliveries = match deliveries {
                Ok(deliveries) => deliveries,
                Err(error) => {
                    self.drop_peer(&token, &error.to_string(), now);
                    return Ok(());
                }
            };
            for delivery in deliveries {
                let closing = self.peers.get(&token).and_then(|peer| peer.closing);
                if closing.is_some() {
                    break;
                }
                match delivery {
                    UnifiedDelivery::Reliable { payload, .. } => {
                        if let Err(error) = self.accept_control(&token, &payload, now) {
                            self.drop_peer(&token, &error.to_string(), now);
                        }
                    }
                    UnifiedDelivery::Frame { payload, .. } => {
                        self.accept_inputs(&token, &payload);
                    }
                }
            }
            return Ok(());
        }
        let Some(handshake) = decode_unified_handshake(payload) else {
            return Ok(());
        };
        match handshake {
            UnifiedHandshake::Challenge { .. } => {}
            UnifiedHandshake::Hello { nonce } => {
                let key = address_key(from, true);
                let known = self.pending.get(&key);
                if known.is_none() && self.pending.len() >= 256 {
                    return Ok(());
                }
                let reuse = known.is_some_and(|pending| pending.nonce == nonce);
                if !reuse {
                    let counter = self.token_counter;
                    self.token_counter += 1;
                    self.pending.insert(
                        key.clone(),
                        PendingPeer {
                            address: from.clone(),
                            nonce,
                            token: super::unified_client::fresh_token(counter),
                            created: now,
                        },
                    );
                }
                if let Some(pending) = self.pending.get(&key) {
                    let challenge = encode_unified_handshake(&UnifiedHandshake::Challenge {
                        nonce: pending.nonce.clone(),
                        token: pending.token.clone(),
                    });
                    let _ = self.transport.send(&pending.address, &challenge);
                }
            }
            UnifiedHandshake::Connect { nonce, token } => {
                let redial = self
                    .peers
                    .get(&token)
                    .is_some_and(|peer| peer.nonce == nonce && same_address(&peer.address, from, true));
                if redial {
                    if let Some(peer) = self.peers.get_mut(&token) {
                        let _ = Self::flush_peer(&self.transport, peer, now);
                    }
                    return Ok(());
                }
                let key = address_key(from, true);
                let admitted = self
                    .pending
                    .get(&key)
                    .is_some_and(|pending| pending.token == token && pending.nonce == nonce);
                if !admitted || self.peers.len() >= self.host.max_clients().saturating_add(8) {
                    return Ok(());
                }
                let pending = self.pending.remove(&key).expect("pending is admitted");
                let ceiling = self.transport.max_datagram_bytes().unwrap_or(1200);
                let channel = UnifiedChannel::new(
                    &pending.token,
                    UnifiedChannelLimits {
                        datagram_bytes: ceiling.min(1200),
                        ..Default::default()
                    },
                )
                .map_err(|error| network_error(error.to_string()))?;
                self.peers.insert(
                    pending.token.clone(),
                    UnifiedPeer {
                        address: from.clone(),
                        token: pending.token.clone(),
                        nonce: pending.nonce,
                        channel,
                        resources: HashSet::new(),
                        player: None,
                        ready: false,
                        last_received: now,
                        last_queued_input: -1,
                        last_consumed_input: -1,
                        last_submitted_input: -1,
                        required_reliable: 0,
                        components: UnifiedComponentPublisher::default(),
                        closing: None,
                    },
                );
                self.offer(&pending.token)?;
            }
        }
        Ok(())
    }

    /// Drain queued commands with NetQuake burst merging (donor poll tail).
    fn drain_commands(&mut self) -> Vec<ActorCommand> {
        let pending = std::mem::take(&mut self.commands);
        let mut latest: HashMap<u32, ActorCommand> = HashMap::new();
        for command in &pending {
            let slot = command.actor.slot();
            match latest.get(&slot) {
                Some(previous)
                    if matches!(command.command, UserCommand::Q1Netquake { .. })
                        && matches!(previous.command, UserCommand::Q1Netquake { .. }) =>
                {
                    let previous = previous.clone();
                    latest.insert(slot, merge_netquake(command, &previous));
                }
                _ => {
                    latest.insert(slot, command.clone());
                }
            }
        }
        let commands: Vec<ActorCommand> = pending
            .into_iter()
            .filter_map(|command| {
                if !matches!(command.command, UserCommand::Q1Netquake { .. }) {
                    return Some(command);
                }
                let selected = latest.get(&command.actor.slot())?;
                if selected.sequence == command.sequence {
                    Some(selected.clone())
                } else {
                    None
                }
            })
            .collect();
        for peer in self.peers.values_mut() {
            let submitted = peer.player.as_ref().and_then(|player| {
                latest.get(&player.actor.slot()).and_then(|command| {
                    if command.actor == player.actor {
                        i64::try_from(command.sequence).ok()
                    } else {
                        None
                    }
                })
            });
            if let Some(submitted) = submitted {
                peer.last_submitted_input = peer.last_submitted_input.max(submitted);
            }
        }
        commands
    }

    /// Publish after the simulation step (donor `publish`).
    pub fn publish_output(&mut self, output: &SimulationOutput, events: &[NetworkPresentationEvent], now: u64) {
        if self.ended {
            return;
        }
        let tokens: Vec<String> = self.peers.keys().cloned().collect();
        for token in tokens {
            let ready = self
                .peers
                .get(&token)
                .is_some_and(|peer| peer.player.is_some() && peer.ready && peer.closing.is_none());
            if !ready {
                continue;
            }
            if let Some(peer) = self.peers.get_mut(&token) {
                peer.last_consumed_input = peer.last_submitted_input;
            }
            if let Err(error) = self.publish_peer(&token, output, events, now) {
                self.drop_peer(&token, &error.to_string(), now);
            }
        }
    }

    /// Publish to one peer.
    fn publish_peer(
        &mut self,
        token: &str,
        output: &SimulationOutput,
        events: &[NetworkPresentationEvent],
        now: u64,
    ) -> Result<(), ApplicationNetworkError> {
        let player = self
            .peers
            .get(token)
            .and_then(|peer| peer.player.clone())
            .expect("peer is ready");
        let declared: Vec<UnifiedResourceKey> = {
            let resources = self.host.resources(&player, output);
            let peer = self.peers.get(token).expect("peer is ready");
            resources
                .into_iter()
                .filter(|resource| {
                    unified_resource_id(resource)
                        .map(|id| !peer.resources.contains(&id))
                        .unwrap_or(true)
                })
                .collect()
        };
        if !declared.is_empty() {
            let control = UnifiedControl::Resources {
                epoch: u64::from(self.epoch),
                resources: declared.clone(),
            };
            if let Some(peer) = self.peers.get_mut(token) {
                Self::queue(peer, &control).map_err(|error| network_error(error.to_string()))?;
            }
            for resource in &declared {
                let id = unified_resource_id(resource).map_err(|error| network_error(error.to_string()))?;
                if let Some(peer) = self.peers.get_mut(token) {
                    peer.resources.insert(id);
                }
            }
        }
        let presentation = self.host.presentation_events(&player, events);
        let consumed = self.peers.get(token).map(|peer| peer.last_consumed_input).unwrap_or(-1);
        let frame = self.host.frame(&player, output, u64::from(self.epoch), consumed);
        if !presentation.is_empty() || !frame.output.events.is_empty() {
            let payload =
                encode_unified_presentation_events(&presentation).map_err(|error| network_error(error.to_string()))?;
            let mut encoded = Vec::new();
            for event in &frame.output.events {
                encoded.push(write_unified_simulation_event(event).map_err(|error| network_error(error.to_string()))?);
            }
            let control = UnifiedControl::Events {
                epoch: u64::from(self.epoch),
                frame: frame.output.snapshot.frame.frame as i64,
                payload,
                simulation: encode_checkpoint_value(&arr(encoded)),
            };
            if let Some(peer) = self.peers.get_mut(token) {
                Self::queue(peer, &control).map_err(|error| network_error(error.to_string()))?;
            }
        }
        let sources = self.host.components(&player);
        let native = self.host.native_components(&player);
        let projected = self
            .peers
            .get_mut(token)
            .expect("peer is ready")
            .components
            .project(&sources, &native)
            .map_err(|error| network_error(error.to_string()))?;
        if let Some(update) = projected.update {
            let control = UnifiedControl::Components {
                epoch: u64::from(self.epoch),
                update,
            };
            if let Some(peer) = self.peers.get_mut(token) {
                Self::queue(peer, &control).map_err(|error| network_error(error.to_string()))?;
            }
        }
        let bytes = encode_unified_frame(&super::unified_types::UnifiedPresentationFrame {
            components: Some(projected.frame),
            output: super::unified_types::UnifiedOutput {
                snapshot: frame.output.snapshot.clone(),
                events: Vec::new(),
            },
            ..frame
        })
        .map_err(|error| network_error(error.to_string()))?;
        if let Some(peer) = self.peers.get_mut(token) {
            let required = peer.required_reliable;
            peer.channel
                .queue_frame(&bytes, required)
                .map_err(|error| network_error(error.to_string()))?;
        }
        if let Some(peer) = self.peers.get_mut(token) {
            Self::flush_peer(&self.transport, peer, now)?;
        }
        Ok(())
    }

    /// Swap the world and composition (donor `changeWorld`).
    pub fn change_world(
        &mut self,
        host: H,
        composition: UnifiedCompositionIdentity,
    ) -> Result<(), ApplicationNetworkError> {
        if self.ended {
            return Err(network_error("Unified server is closed"));
        }
        if self.epoch == u32::MAX {
            return Err(network_error("Unified world sequence exhausted"));
        }
        self.host = host;
        self.composition = composition;
        self.epoch += 1;
        self.commands.clear();
        let tokens: Vec<String> = self.peers.keys().cloned().collect();
        for token in tokens {
            if self.peers.get(&token).and_then(|peer| peer.closing).is_some() {
                continue;
            }
            let carried = self
                .peers
                .get(&token)
                .and_then(|peer| peer.player.clone())
                .and_then(|player| self.host.carried_player(&player.client));
            if let Some(peer) = self.peers.get_mut(&token) {
                peer.player = carried;
            }
            self.offer(&token)?;
        }
        Ok(())
    }

    /// Close the server (donor `close`).
    pub fn close(&mut self) {
        if self.ended {
            return;
        }
        self.ended = true;
        for peer in self.peers.values() {
            if let Some(player) = peer.player.clone() {
                self.host.disconnect(&player);
            }
            // Channel close needs a mutable peer; collected below.
        }
        for peer in self.peers.values_mut() {
            peer.channel.close();
        }
        self.peers.clear();
        self.pending.clear();
        self.commands.clear();
        self.transport.close();
    }
}

fn control_epoch(control: &UnifiedControl) -> u64 {
    match control {
        UnifiedControl::Offer { epoch, .. }
        | UnifiedControl::Ready { epoch, .. }
        | UnifiedControl::Admitted { epoch, .. }
        | UnifiedControl::Resources { epoch, .. }
        | UnifiedControl::Events { epoch, .. }
        | UnifiedControl::Components { epoch, .. }
        | UnifiedControl::ComponentCommand { epoch, .. }
        | UnifiedControl::Command { epoch, .. }
        | UnifiedControl::Userinfo { epoch, .. } => *epoch,
        UnifiedControl::Disconnect { .. } => 0,
    }
}

fn packet_token(packet: &qa_net::unified::UnifiedPacket) -> &str {
    match packet {
        qa_net::unified::UnifiedPacket::Ack { token, .. }
        | qa_net::unified::UnifiedPacket::Reliable { token, .. }
        | qa_net::unified::UnifiedPacket::Frame { token, .. } => token,
    }
}

/// Merge a NetQuake burst pair (donor poll merge).
fn merge_netquake(command: &ActorCommand, previous: &ActorCommand) -> ActorCommand {
    let arsenal = match (&command.arsenal, &previous.arsenal) {
        (None, None) => None,
        (arsenal, None) => arsenal.clone(),
        (None, Some(previous)) => Some(previous.clone()),
        (Some(current), Some(previous)) if current.provider != previous.provider => Some(current.clone()),
        (Some(current), Some(previous)) => Some(ArsenalIntent {
            provider: current.provider.clone(),
            weapon: current.weapon.clone().or_else(|| previous.weapon.clone()),
            use_holdable: current.use_holdable || previous.use_holdable,
        }),
    };
    let impulse = match (&command.command, &previous.command) {
        (
            UserCommand::Q1Netquake { impulse, .. },
            UserCommand::Q1Netquake {
                impulse: previous_impulse,
                ..
            },
        ) => {
            if *impulse != 0.0 {
                *impulse
            } else {
                *previous_impulse
            }
        }
        _ => 0.0,
    };
    let mut merged = command.command.clone();
    if let UserCommand::Q1Netquake { impulse: slot, .. } = &mut merged {
        *slot = impulse;
    }
    ActorCommand {
        command: merged,
        arsenal,
        ..command.clone()
    }
}

impl<T, H, P> ApplicationNetwork for UnifiedServerNetwork<T, H, P>
where
    T: DatagramTransport<Address = NetworkAddress>,
    H: UnifiedServerHost,
    P: FnMut(&str),
{
    fn role(&self) -> ApplicationNetworkRole {
        ApplicationNetworkRole::Server
    }

    fn phase(&self) -> ApplicationNetworkPhase {
        if self.ended {
            ApplicationNetworkPhase::Closed
        } else {
            ApplicationNetworkPhase::Active
        }
    }

    fn wire(&self) -> WireSelection {
        let (namespace, name) = super::unified_content::UNIFIED_SNAPSHOT_SCHEMA
            .split_once(':')
            .unwrap_or(("", ""));
        WireSelection::Unified {
            version: 1,
            composition: wire_digest(&self.composition.digest),
            snapshot_schema: qa_core::identity::ProviderId::new(namespace, name),
        }
    }

    fn poll(&mut self, now_milliseconds: u64) -> Result<Vec<ActorCommand>, ApplicationNetworkError> {
        UnifiedServerNetwork::poll(self, now_milliseconds)
    }

    fn submit(&mut self, _commands: &[ActorCommand], _now_milliseconds: u64) -> Result<(), ApplicationNetworkError> {
        Err(network_error("Server input belongs to its authoritative simulation"))
    }

    fn publish(
        &mut self,
        output: &SimulationOutput,
        events: &[NetworkPresentationEvent],
        now_milliseconds: u64,
    ) -> Result<(), ApplicationNetworkError> {
        self.publish_output(output, events, now_milliseconds);
        Ok(())
    }

    fn close(&mut self) {
        UnifiedServerNetwork::close(self);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    use std::sync::{Arc, Mutex};

    use super::super::unified_frame_codec::tests as frame_tests;
    use crate::persistence::recipe::fixture_recipe;
    use qa_core::identity::IdentityOwner;
    use qa_net::common::transport::TransportError;

    #[derive(Debug, Default)]
    struct TestTransport {
        inbox: Mutex<VecDeque<ReceiveEvent<NetworkAddress>>>,
        sent: Mutex<Vec<(NetworkAddress, Vec<u8>)>>,
        closed: Mutex<bool>,
    }

    impl DatagramTransport for TestTransport {
        type Address = NetworkAddress;

        fn address(&self) -> NetworkAddress {
            NetworkAddress::Loopback {
                id: "server".to_string(),
            }
        }

        fn closed(&self) -> bool {
            *self.closed.lock().unwrap()
        }

        fn send(&self, to: &NetworkAddress, payload: &[u8]) -> Result<bool, TransportError> {
            self.sent.lock().unwrap().push((to.clone(), payload.to_vec()));
            Ok(true)
        }

        fn poll(&self) -> Result<Option<ReceiveEvent<NetworkAddress>>, TransportError> {
            Ok(self.inbox.lock().unwrap().pop_front())
        }

        fn subscribe_readable(&self, _listener: Arc<dyn Fn() + Send + Sync>) -> Result<u64, TransportError> {
            Ok(0)
        }

        fn unsubscribe(&self, _token: u64) {}

        fn close(&self) {
            *self.closed.lock().unwrap() = true;
        }
    }

    #[derive(Clone)]
    struct SharedTransport(Arc<TestTransport>);

    impl DatagramTransport for SharedTransport {
        type Address = NetworkAddress;

        fn address(&self) -> NetworkAddress {
            self.0.address()
        }

        fn closed(&self) -> bool {
            self.0.closed()
        }

        fn max_datagram_bytes(&self) -> Option<usize> {
            self.0.max_datagram_bytes()
        }

        fn send(&self, to: &NetworkAddress, payload: &[u8]) -> Result<bool, TransportError> {
            self.0.send(to, payload)
        }

        fn poll(&self) -> Result<Option<ReceiveEvent<NetworkAddress>>, TransportError> {
            self.0.poll()
        }

        fn subscribe_readable(&self, listener: Arc<dyn Fn() + Send + Sync>) -> Result<u64, TransportError> {
            self.0.subscribe_readable(listener)
        }

        fn unsubscribe(&self, token: u64) {
            self.0.unsubscribe(token);
        }

        fn close(&self) {
            self.0.close();
        }
    }

    fn client_address() -> NetworkAddress {
        NetworkAddress::Loopback {
            id: "client".to_string(),
        }
    }

    fn composition() -> UnifiedCompositionIdentity {
        super::super::unified_content::create_unified_composition(&fixture_recipe(), &[]).expect("fixture composition")
    }

    #[derive(Debug, Default)]
    struct TestHostState {
        disconnects: Vec<UnifiedApplicationPlayer>,
        commands: Vec<(String, Vec<String>)>,
        inputs: Vec<i64>,
    }

    struct TestHost {
        identity: IdentityOwner,
        state: Arc<Mutex<TestHostState>>,
        reject: Option<String>,
    }

    impl TestHost {
        fn player(&self) -> UnifiedApplicationPlayer {
            UnifiedApplicationPlayer {
                actor: self.identity.actor(1, 1),
                client: self.identity.client(0, 1),
                source_entity: 1,
            }
        }
    }

    impl UnifiedServerHost for TestHost {
        fn mode(&self) -> UnifiedServerMode {
            UnifiedServerMode::Deathmatch
        }

        fn max_clients(&self) -> usize {
            8
        }

        fn admit(&mut self, _address: &NetworkAddress, _userinfo: &str) -> UnifiedAdmission {
            match self.reject.clone() {
                Some(reason) => UnifiedAdmission::Rejected { reason },
                None => UnifiedAdmission::Accepted(self.player()),
            }
        }

        fn disconnect(&mut self, player: &UnifiedApplicationPlayer) {
            self.state.lock().unwrap().disconnects.push(player.clone());
        }

        fn userinfo(&mut self, _player: &UnifiedApplicationPlayer, _userinfo: &str) {}

        fn initial_presentation(&mut self, _player: &UnifiedApplicationPlayer) -> Vec<UnifiedPresentationEvent> {
            Vec::new()
        }

        fn input(
            &mut self,
            player: &UnifiedApplicationPlayer,
            sequence: i64,
            command: &UserCommand,
            arsenal: Option<&UnifiedArsenalIntent>,
        ) -> ActorCommand {
            self.state.lock().unwrap().inputs.push(sequence);
            ActorCommand {
                actor: player.actor.clone(),
                source: qa_net::common::commands::CommandSource::Remote {
                    client: player.client.clone(),
                },
                sequence: sequence as u64,
                command: command.clone(),
                arsenal: arsenal.map(|arsenal| ArsenalIntent {
                    provider: arsenal.provider.clone(),
                    weapon: arsenal.weapon.clone(),
                    use_holdable: arsenal.use_holdable,
                }),
            }
        }

        fn command(&mut self, _player: &UnifiedApplicationPlayer, name: &str, args: &[String]) {
            self.state
                .lock()
                .unwrap()
                .commands
                .push((name.to_string(), args.to_vec()));
        }

        fn resources(
            &mut self,
            _player: &UnifiedApplicationPlayer,
            _output: &SimulationOutput,
        ) -> Vec<UnifiedResourceKey> {
            Vec::new()
        }

        fn presentation_events(
            &mut self,
            _player: &UnifiedApplicationPlayer,
            _events: &[NetworkPresentationEvent],
        ) -> Vec<UnifiedPresentationEvent> {
            Vec::new()
        }

        fn frame(
            &mut self,
            _player: &UnifiedApplicationPlayer,
            _output: &SimulationOutput,
            _epoch: u64,
            _acknowledged: i64,
        ) -> super::super::unified_types::UnifiedPresentationFrame {
            let ledger = frame_tests::ledger();
            let mut frame = frame_tests::frame(&ledger);
            let actor = self.identity.actor(1, 1);
            frame.player.actor = actor.clone();
            frame.prediction.actor = actor;
            frame.prediction.sequence = frame.acknowledged_input;
            frame.epoch = 1;
            frame
        }

        fn carried_player(&mut self, _client: &ClientId) -> Option<UnifiedApplicationPlayer> {
            Some(self.player())
        }
    }

    #[allow(clippy::type_complexity)]
    fn harness() -> (
        UnifiedServerNetwork<SharedTransport, TestHost, impl FnMut(&str)>,
        Arc<TestTransport>,
        Arc<Mutex<TestHostState>>,
        Arc<Mutex<Vec<String>>>,
    ) {
        let transport = Arc::new(TestTransport::default());
        let state = Arc::new(Mutex::new(TestHostState::default()));
        let logs = Arc::new(Mutex::new(Vec::new()));
        let sink = logs.clone();
        let server = UnifiedServerNetwork::new(UnifiedServerOptions {
            transport: SharedTransport(transport.clone()),
            host: TestHost {
                identity: IdentityOwner::create("server").unwrap(),
                state: state.clone(),
                reject: None,
            },
            composition: composition(),
            print: move |text: &str| sink.lock().unwrap().push(text.to_string()),
        });
        (server, transport, state, logs)
    }

    fn feed(transport: &TestTransport, payload: Vec<u8>) {
        transport.inbox.lock().unwrap().push_back(ReceiveEvent::Packet {
            from: client_address(),
            payload,
            received_at: 0.0,
        });
    }

    fn client_channel(token: &str) -> UnifiedChannel {
        UnifiedChannel::new(
            token,
            UnifiedChannelLimits {
                datagram_bytes: 1200,
                ..Default::default()
            },
        )
        .unwrap()
    }

    /// Feed server datagrams into a client channel, returning deliveries.
    fn drain_to_client(client: &mut UnifiedChannel, transport: &TestTransport) -> Vec<UnifiedDelivery> {
        let mut deliveries = Vec::new();
        for (_, bytes) in transport.sent.lock().unwrap().drain(..) {
            if decode_unified_handshake(&bytes).is_some() {
                continue;
            }
            if decode_unified_packet(&bytes).is_some() {
                deliveries.extend(client.receive(&bytes, 1.0).unwrap());
            }
        }
        deliveries
    }

    fn handshake(
        server: &mut UnifiedServerNetwork<SharedTransport, TestHost, impl FnMut(&str)>,
        transport: &TestTransport,
    ) -> (UnifiedChannel, String) {
        feed(
            transport,
            encode_unified_handshake(&UnifiedHandshake::Hello { nonce: "ab".repeat(16) }),
        );
        server.poll(0).unwrap();
        let challenge = {
            let sent = transport.sent.lock().unwrap();
            assert_eq!(sent.len(), 1);
            decode_unified_handshake(&sent[0].1).expect("challenge")
        };
        let (nonce, token) = match challenge {
            UnifiedHandshake::Challenge { nonce, token } => (nonce, token),
            _ => panic!("expected challenge"),
        };
        assert_eq!(token.len(), 32);
        feed(
            transport,
            encode_unified_handshake(&UnifiedHandshake::Connect {
                nonce,
                token: token.clone(),
            }),
        );
        server.poll(1).unwrap();
        (client_channel(&token), token)
    }

    fn admit(
        server: &mut UnifiedServerNetwork<SharedTransport, TestHost, impl FnMut(&str)>,
        transport: &TestTransport,
        client: &mut UnifiedChannel,
    ) {
        for delivery in drain_to_client(client, transport) {
            if let UnifiedDelivery::Reliable { payload, .. } = delivery {
                assert!(matches!(
                    decode_unified_control(&payload).unwrap(),
                    UnifiedControl::Offer { .. }
                ));
            }
        }
        client
            .queue_reliable(&encode_unified_control(&UnifiedControl::Ready {
                epoch: 1,
                composition: composition().digest.to_string(),
                userinfo: "player".to_string(),
            }))
            .unwrap();
        for bytes in client.flush(2.0).unwrap() {
            feed(transport, bytes);
        }
        server.poll(2).unwrap();
        assert_eq!(server.clients().len(), 1);
    }

    fn q1_command(impulse: f64) -> UserCommand {
        UserCommand::Q1Netquake {
            acknowledged_server_time_seconds: 0.0,
            view_angles: [0.0, 0.0, 0.0],
            forward_move: 0.0,
            side_move: 0.0,
            up_move: 0.0,
            buttons: 0.0,
            impulse,
        }
    }

    #[test]
    fn hello_challenge_connect_offers() {
        let (mut server, transport, _, _) = harness();
        let (mut client, _) = handshake(&mut server, &transport);
        let deliveries = drain_to_client(&mut client, &transport);
        assert!(deliveries.iter().any(|delivery| match delivery {
            UnifiedDelivery::Reliable { payload, .. } => matches!(
                decode_unified_control(payload).unwrap(),
                UnifiedControl::Offer { epoch: 1, .. }
            ),
            _ => false,
        }));
    }

    #[test]
    fn duplicate_hello_reuses_token() {
        let (mut server, transport, _, _) = harness();
        for _ in 0..2 {
            feed(
                &transport,
                encode_unified_handshake(&UnifiedHandshake::Hello { nonce: "ab".repeat(16) }),
            );
            server.poll(0).unwrap();
        }
        let sent = transport.sent.lock().unwrap();
        assert_eq!(sent.len(), 2);
        let first = decode_unified_handshake(&sent[0].1).expect("challenge");
        let second = decode_unified_handshake(&sent[1].1).expect("challenge");
        assert_eq!(first, second);
    }

    #[test]
    fn stray_connect_is_ignored() {
        let (mut server, transport, _, _) = harness();
        feed(
            &transport,
            encode_unified_handshake(&UnifiedHandshake::Connect {
                nonce: "ab".repeat(16),
                token: "0".repeat(32),
            }),
        );
        server.poll(0).unwrap();
        assert!(transport.sent.lock().unwrap().is_empty());
        assert!(server.clients().is_empty());
    }

    #[test]
    fn ready_admits_player() {
        let (mut server, transport, _, _) = harness();
        let (mut client, _) = handshake(&mut server, &transport);
        admit(&mut server, &transport, &mut client);
        let deliveries = drain_to_client(&mut client, &transport);
        assert!(deliveries.iter().any(|delivery| match delivery {
            UnifiedDelivery::Reliable { payload, .. } => matches!(
                decode_unified_control(payload).unwrap(),
                UnifiedControl::Admitted { .. }
            ),
            _ => false,
        }));
    }

    #[test]
    fn input_batch_queues_commands() {
        let (mut server, transport, _, _) = harness();
        let (mut client, _) = handshake(&mut server, &transport);
        admit(&mut server, &transport, &mut client);
        client
            .queue_frame(
                &super::super::unified_control::encode_unified_inputs(
                    &super::super::unified_control::UnifiedInputBatch {
                        epoch: 1,
                        commands: vec![super::super::unified_control::UnifiedInput {
                            sequence: 0,
                            command: q1_command(0.0),
                            arsenal: None,
                        }],
                    },
                ),
                0,
            )
            .unwrap();
        for bytes in client.flush(3.0).unwrap() {
            feed(&transport, bytes);
        }
        let commands = server.poll(3).unwrap();
        assert_eq!(commands.len(), 1);
        assert_eq!(commands[0].sequence, 0);
    }

    #[test]
    fn netquake_burst_merges_impulse() {
        let (mut server, transport, state, _) = harness();
        let (mut client, _) = handshake(&mut server, &transport);
        admit(&mut server, &transport, &mut client);
        client
            .queue_frame(
                &super::super::unified_control::encode_unified_inputs(
                    &super::super::unified_control::UnifiedInputBatch {
                        epoch: 1,
                        commands: vec![
                            super::super::unified_control::UnifiedInput {
                                sequence: 0,
                                command: q1_command(4.0),
                                arsenal: Some(UnifiedArsenalIntent {
                                    provider: "q1:arsenal".to_string(),
                                    weapon: None,
                                    use_holdable: false,
                                    impulse: None,
                                }),
                            },
                            super::super::unified_control::UnifiedInput {
                                sequence: 1,
                                command: q1_command(0.0),
                                arsenal: Some(UnifiedArsenalIntent {
                                    provider: "q1:arsenal".to_string(),
                                    weapon: Some("q1:weapon/shotgun".to_string()),
                                    use_holdable: true,
                                    impulse: None,
                                }),
                            },
                        ],
                    },
                ),
                0,
            )
            .unwrap();
        for bytes in client.flush(3.0).unwrap() {
            feed(&transport, bytes);
        }
        let commands = server.poll(3).unwrap();
        assert_eq!(commands.len(), 1);
        assert_eq!(commands[0].sequence, 1);
        match &commands[0].command {
            UserCommand::Q1Netquake { impulse, .. } => assert_eq!(*impulse, 4.0),
            _ => panic!("expected netquake command"),
        }
        let arsenal = commands[0].arsenal.as_ref().expect("arsenal");
        assert_eq!(arsenal.weapon.as_deref(), Some("q1:weapon/shotgun"));
        assert!(arsenal.use_holdable);
        assert_eq!(state.lock().unwrap().inputs, vec![0, 1]);
    }

    #[test]
    fn mismatched_composition_drops() {
        let (mut server, transport, _, _) = harness();
        let (mut client, _) = handshake(&mut server, &transport);
        drain_to_client(&mut client, &transport);
        client
            .queue_reliable(&encode_unified_control(&UnifiedControl::Ready {
                epoch: 1,
                composition: "sha256:wrong".to_string(),
                userinfo: "player".to_string(),
            }))
            .unwrap();
        for bytes in client.flush(2.0).unwrap() {
            feed(&transport, bytes);
        }
        server.poll(2).unwrap();
        assert!(server.clients().is_empty());
    }

    #[test]
    fn command_before_admission_drops() {
        let (mut server, transport, _, logs) = harness();
        let (mut client, _) = handshake(&mut server, &transport);
        drain_to_client(&mut client, &transport);
        client
            .queue_reliable(&encode_unified_control(&UnifiedControl::Command {
                epoch: 1,
                name: "say".to_string(),
                args: Vec::new(),
            }))
            .unwrap();
        for bytes in client.flush(2.0).unwrap() {
            feed(&transport, bytes);
        }
        server.poll(2).unwrap();
        assert!(server.clients().is_empty());
        assert!(logs.lock().unwrap().iter().any(|line| line.contains("disconnected")));
    }

    #[test]
    fn player_command_routes_to_host() {
        let (mut server, transport, state, _) = harness();
        let (mut client, _) = handshake(&mut server, &transport);
        admit(&mut server, &transport, &mut client);
        client
            .queue_reliable(&encode_unified_control(&UnifiedControl::Command {
                epoch: 1,
                name: "say".to_string(),
                args: vec!["hi".to_string()],
            }))
            .unwrap();
        for bytes in client.flush(3.0).unwrap() {
            feed(&transport, bytes);
        }
        server.poll(3).unwrap();
        assert_eq!(
            state.lock().unwrap().commands,
            vec![("say".to_string(), vec!["hi".to_string()])]
        );
        assert_eq!(server.clients().len(), 1);
    }

    #[test]
    fn component_command_without_receiver_drops() {
        let (mut server, transport, _, _) = harness();
        let (mut client, _) = handshake(&mut server, &transport);
        admit(&mut server, &transport, &mut client);
        drain_to_client(&mut client, &transport);
        client
            .queue_reliable(&encode_unified_control(&UnifiedControl::ComponentCommand {
                epoch: 1,
                owner: qa_content::contract::PresentationOwner {
                    provider: qa_core::identity::ProviderId::new("q1", "mod"),
                    generation: 1,
                },
                generation: 0,
                args: vec!["ping".to_string()],
            }))
            .unwrap();
        for bytes in client.flush(3.0).unwrap() {
            feed(&transport, bytes);
        }
        server.poll(3).unwrap();
        assert!(server.clients().is_empty());
        let deliveries = drain_to_client(&mut client, &transport);
        assert!(deliveries.iter().any(|delivery| match delivery {
            UnifiedDelivery::Reliable { payload, .. } => match decode_unified_control(payload) {
                Ok(UnifiedControl::Disconnect { reason }) => reason == "No component command receiver",
                _ => false,
            },
            _ => false,
        }));
    }

    #[test]
    fn publish_sends_frame() {
        use qa_core::time::{FrameContext, FramePhase, SourceTime};
        let (mut server, transport, _, _) = harness();
        let (mut client, _) = handshake(&mut server, &transport);
        admit(&mut server, &transport, &mut client);
        drain_to_client(&mut client, &transport);
        let output = SimulationOutput {
            snapshot: qa_world::session::WorldSnapshot {
                frame: FrameContext {
                    frame: 9,
                    time: SourceTime::Seconds(1.0),
                    elapsed: SourceTime::Seconds(0.1),
                    phase: FramePhase::FrameExit,
                },
                actors: Vec::new(),
                bodies: Vec::new(),
                inventories: Vec::new(),
            },
            events: Vec::new(),
        };
        server.publish_output(&output, &[], 10);
        let deliveries = drain_to_client(&mut client, &transport);
        assert!(deliveries
            .iter()
            .any(|delivery| matches!(delivery, UnifiedDelivery::Frame { .. })));
    }

    #[test]
    fn change_world_reoffers() {
        let (mut server, transport, _, _) = harness();
        let (mut client, _) = handshake(&mut server, &transport);
        admit(&mut server, &transport, &mut client);
        drain_to_client(&mut client, &transport);
        server
            .change_world(
                TestHost {
                    identity: IdentityOwner::create("server").unwrap(),
                    state: Arc::new(Mutex::new(TestHostState::default())),
                    reject: None,
                },
                composition(),
            )
            .unwrap();
        server.poll(10).unwrap();
        let deliveries = drain_to_client(&mut client, &transport);
        assert!(deliveries.iter().any(|delivery| match delivery {
            UnifiedDelivery::Reliable { payload, .. } => matches!(
                decode_unified_control(payload).unwrap(),
                UnifiedControl::Offer { epoch: 2, .. }
            ),
            _ => false,
        }));
    }

    #[test]
    fn idle_peer_times_out() {
        let (mut server, transport, _, logs) = harness();
        let (mut client, _) = handshake(&mut server, &transport);
        admit(&mut server, &transport, &mut client);
        server.poll(60_000).unwrap();
        assert!(server.clients().is_empty());
        assert!(logs
            .lock()
            .unwrap()
            .iter()
            .any(|line| line.contains("Connection timed out")));
    }

    #[test]
    fn disconnect_client_drops_peer() {
        let (mut server, transport, state, _) = harness();
        let (mut client, _) = handshake(&mut server, &transport);
        admit(&mut server, &transport, &mut client);
        let player = server.clients().pop().expect("player");
        assert!(server.disconnect_client(&player.client, "kicked"));
        assert!(server.clients().is_empty());
        assert_eq!(state.lock().unwrap().disconnects.len(), 1);
        let unknown = IdentityOwner::create("other").unwrap().client(0, 1);
        assert!(!server.disconnect_client(&unknown, "kicked"));
    }

    #[test]
    fn close_disconnects_all() {
        let (mut server, transport, state, _) = harness();
        let (mut client, _) = handshake(&mut server, &transport);
        admit(&mut server, &transport, &mut client);
        server.close();
        assert_eq!(state.lock().unwrap().disconnects.len(), 1);
        assert!(transport.closed());
        assert_eq!(server.phase(), ApplicationNetworkPhase::Closed);
        assert!(server.poll(99).unwrap().is_empty());
        assert!(server.submit(&[], 99).is_err());
    }
}
