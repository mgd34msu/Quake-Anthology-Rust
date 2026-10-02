//! Unified remote client endpoint.
//!
//! Port of `/home/buzzkill/Projects/quake-typescript/src/app/bootstrap/network/unified-client.ts`
//! (`UnifiedClientNetwork`). The handshake, input batching, and
//! presentation plumbing around a [`UnifiedRemotePresentation`]. The
//! donor's clock reads collapse onto the poll timestamp, and the
//! donor's `submit` elapsed milliseconds derive from consecutive poll
//! timestamps because the sync [`ApplicationNetwork`] port passes the
//! current time rather than a measured delta.

use std::collections::BTreeMap;
use std::time::{SystemTime, UNIX_EPOCH};

use qa_content::contract::{ContentDigest, PresentationOwner};
use qa_content::hash::sha256_hex;
use qa_core::cmd::{tokenize_command, Dialect, TextMode};
use qa_core::identity::ProviderId;
use qa_net::common::commands::{ActorCommand, CommandSource, UserCommand};
use qa_net::common::endpoint::{same_address, NetworkAddress};
use qa_net::common::session::WireSelection;
use qa_net::common::transport::{DatagramTransport, ReceiveEvent};
use qa_net::unified::{decode_unified_packet, UnifiedChannel, UnifiedChannelLimits, UnifiedDelivery, UnifiedPacket};

use super::remote_unified::{UnifiedRemoteHost, UnifiedRemotePredictionCommand, UnifiedRemotePresentation};
use super::types::{ApplicationNetwork, ApplicationNetworkError, ApplicationNetworkPhase, ApplicationNetworkRole};
use super::unified_content::{UnifiedCompositionIdentity, UNIFIED_SNAPSHOT_SCHEMA};
use super::unified_control::{
    decode_unified_control, decode_unified_handshake, encode_unified_control, encode_unified_handshake,
    encode_unified_inputs, UnifiedControl, UnifiedHandshake, UnifiedInput, UnifiedInputBatch,
};

/// Fresh 32-hex-digit token (donor `randomBytes(16).toString('hex')`).
///
/// The workspace has no OS randomness source, so tokens hash the process
/// id, a per-process address salt, wall-clock nanoseconds, and a counter.
/// Tokens stay unpredictable across processes; document this if stronger
/// guarantees are ever required.
pub(crate) fn fresh_token(counter: u64) -> String {
    static SALT: u8 = 0;
    let salt = &SALT as *const u8 as u64;
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_nanos())
        .unwrap_or(0);
    let digest = sha256_hex(format!("{}:{salt}:{nanos}:{counter}", std::process::id()).as_bytes());
    digest[..32].to_string()
}

/// Client connection state (donor `ClientNetworkState`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ClientState {
    Handshake,
    Connecting,
    Loading,
    Active,
    Closed,
    Rejected,
}

/// Unified client options (donor `UnifiedClientOptions`).
pub struct UnifiedClientOptions<T, H: UnifiedRemoteHost, U> {
    /// Datagram transport.
    pub transport: T,
    /// Server address.
    pub remote: NetworkAddress,
    /// Remote presentation host.
    pub host: UnifiedRemotePresentation<H>,
    /// Fresh userinfo command.
    pub userinfo: U,
}

/// Unified remote client (donor `UnifiedClientNetwork`).
pub struct UnifiedClientNetwork<T, H: UnifiedRemoteHost, U> {
    transport: T,
    remote: NetworkAddress,
    host: UnifiedRemotePresentation<H>,
    userinfo_command: U,
    state: ClientState,
    nonce: String,
    token: Option<String>,
    composition: Option<UnifiedCompositionIdentity>,
    channel: Option<UnifiedChannel>,
    epoch: u64,
    inputs: BTreeMap<u64, UnifiedInput>,
    acknowledged: i64,
    command_time: Option<f64>,
    last_userinfo: String,
    handshake_started: Option<u64>,
    last_handshake: Option<u64>,
    last_received: Option<u64>,
    last_now: u64,
    submitted_at: Option<u64>,
}

fn network_error(message: impl Into<String>) -> ApplicationNetworkError {
    ApplicationNetworkError::Message(message.into())
}

/// Convert a composition digest onto the wire selection digest.
pub(crate) fn wire_digest(digest: &ContentDigest) -> qa_net::common::session::ContentDigest {
    let hex = digest.as_str().strip_prefix("sha256:").unwrap_or(digest.as_str());
    qa_net::common::session::ContentDigest::new(hex).expect("unified composition digest is sha256 hex")
}

impl<T, H, U> UnifiedClientNetwork<T, H, U>
where
    T: DatagramTransport<Address = NetworkAddress>,
    H: UnifiedRemoteHost + 'static,
    H::Content: 'static,
    H::Media: 'static,
    U: FnMut() -> String,
{
    /// Wrap client options.
    pub fn new(options: UnifiedClientOptions<T, H, U>) -> Self {
        Self {
            transport: options.transport,
            remote: options.remote,
            host: options.host,
            userinfo_command: options.userinfo,
            state: ClientState::Handshake,
            nonce: fresh_token(0),
            token: None,
            composition: None,
            channel: None,
            epoch: 0,
            inputs: BTreeMap::new(),
            acknowledged: -1,
            command_time: None,
            last_userinfo: String::new(),
            handshake_started: None,
            last_handshake: None,
            last_received: None,
            last_now: 0,
            submitted_at: None,
        }
    }

    /// Borrow the remote presentation host.
    #[must_use]
    pub fn host(&self) -> &UnifiedRemotePresentation<H> {
        &self.host
    }

    /// Mutably borrow the remote presentation host.
    pub fn host_mut(&mut self) -> &mut UnifiedRemotePresentation<H> {
        &mut self.host
    }

    fn send_control(&mut self, control: &UnifiedControl) -> Result<(), ApplicationNetworkError> {
        let channel = self
            .channel
            .as_mut()
            .ok_or_else(|| network_error("Unified connection has no channel"))?;
        let payload = encode_unified_control(control);
        channel
            .queue_reliable(&payload)
            .map_err(|error| network_error(error.to_string()))?;
        Ok(())
    }

    fn send_handshake(&mut self, handshake: &UnifiedHandshake) -> Result<(), ApplicationNetworkError> {
        let payload = encode_unified_handshake(handshake);
        self.transport
            .send(&self.remote, &payload)
            .map_err(|error| network_error(error.to_string()))?;
        self.last_handshake = Some(self.last_now);
        Ok(())
    }

    /// Run a console command (donor `command`).
    pub fn command(&mut self, text: &str) -> Result<(), ApplicationNetworkError> {
        let tokens =
            tokenize_command(text, Dialect::Q3, TextMode::Source).map_err(|error| network_error(error.to_string()))?;
        let mut argv = tokens.argv.into_iter();
        let Some(name) = argv.next() else {
            return Ok(());
        };
        let args: Vec<String> = argv.collect();
        self.player_command(&name, &args)
    }

    /// Send fresh userinfo (donor `userinfo`).
    pub fn userinfo(&mut self, value: &str) -> Result<(), ApplicationNetworkError> {
        if self.epoch == 0 {
            return Ok(());
        }
        self.send_control(&UnifiedControl::Userinfo {
            epoch: self.epoch,
            value: value.to_string(),
        })
    }

    /// Run a player command (donor `playerCommand`).
    pub fn player_command(&mut self, name: &str, args: &[String]) -> Result<(), ApplicationNetworkError> {
        if self.state != ClientState::Active {
            return Ok(());
        }
        self.send_control(&UnifiedControl::Command {
            epoch: self.epoch,
            name: name.to_string(),
            args: args.to_vec(),
        })
    }

    /// Run a component command (donor `componentCommand`).
    pub fn component_command(
        &mut self,
        owner: &PresentationOwner,
        generation: i64,
        args: &[String],
    ) -> Result<(), ApplicationNetworkError> {
        if self.state != ClientState::Active {
            return Err(network_error("Component command requires an active client"));
        }
        self.send_control(&UnifiedControl::ComponentCommand {
            epoch: self.epoch,
            owner: owner.clone(),
            generation,
            args: args.to_vec(),
        })
    }

    /// Submit timed input (donor `submitTimed`).
    pub fn submit_timed(
        &mut self,
        commands: &[ActorCommand],
        now: u64,
        elapsed_milliseconds: f64,
    ) -> Result<(), ApplicationNetworkError> {
        if !elapsed_milliseconds.is_finite() || elapsed_milliseconds < 0.0 {
            return Err(network_error("Unified input needs finite elapsed milliseconds"));
        }
        if self.state != ClientState::Active {
            return Ok(());
        }
        let player = self.host.player();
        let baseline = self.host.command_time_milliseconds();
        for command in commands {
            let local = matches!(command.source, CommandSource::LocalSeat { .. });
            let owned = player.as_ref().is_some_and(|player| player.actor == command.actor);
            if !local || !owned {
                return Err(network_error("Unified input belongs to another player"));
            }
            if (self.acknowledged >= 0 && command.sequence <= self.acknowledged as u64)
                || self.inputs.contains_key(&command.sequence)
            {
                continue;
            }
            let Some(baseline) = baseline else {
                return Err(network_error("Unified input has no authoritative clock"));
            };
            let duration = match &command.command {
                UserCommand::Q1Netquake { .. } | UserCommand::Q3 { .. } => elapsed_milliseconds,
                UserCommand::Q1Quakeworld { milliseconds, .. }
                | UserCommand::Q2Classic { milliseconds, .. }
                | UserCommand::Q2Rerelease { milliseconds, .. } => *milliseconds,
            };
            let start = self.command_time.unwrap_or(baseline).max(baseline);
            let time = start + duration;
            self.command_time = Some(time);
            self.submitted_at = Some(now);
            let timed = match &command.command {
                UserCommand::Q3 { .. } => {
                    let mut timed = command.command.clone();
                    if let UserCommand::Q3 {
                        server_time_milliseconds,
                        ..
                    } = &mut timed
                    {
                        *server_time_milliseconds = time.trunc();
                    }
                    timed
                }
                UserCommand::Q1Netquake { .. } => {
                    let mut timed = command.command.clone();
                    if let UserCommand::Q1Netquake {
                        acknowledged_server_time_seconds,
                        ..
                    } = &mut timed
                    {
                        *acknowledged_server_time_seconds = baseline / 1000.0;
                    }
                    timed
                }
                _ => command.command.clone(),
            };
            let arsenal = command
                .arsenal
                .as_ref()
                .map(|arsenal| super::unified_control::UnifiedArsenalIntent {
                    provider: arsenal.provider.clone(),
                    weapon: arsenal.weapon.clone(),
                    use_holdable: arsenal.use_holdable,
                    impulse: None,
                });
            let sequence = i64::try_from(command.sequence)
                .map_err(|_| network_error("Unified input sequence exceeds its range"))?;
            self.host.predict(&UnifiedRemotePredictionCommand {
                sequence,
                time_milliseconds: time,
                command: timed.clone(),
                arsenal: arsenal.clone(),
            });
            self.inputs.insert(
                command.sequence,
                UnifiedInput {
                    sequence,
                    command: timed,
                    arsenal,
                },
            );
            while self.inputs.len() > 64 {
                self.inputs.pop_first();
            }
        }
        Ok(())
    }
}

impl<T, H, U> UnifiedClientNetwork<T, H, U>
where
    T: DatagramTransport<Address = NetworkAddress>,
    H: UnifiedRemoteHost + 'static,
    H::Content: 'static,
    H::Media: 'static,
    U: FnMut() -> String,
{
    /// Poll the transport (donor `poll`).
    pub fn poll(&mut self, now: u64) -> Result<Vec<ActorCommand>, ApplicationNetworkError> {
        if self.state == ClientState::Closed || self.state == ClientState::Rejected {
            return Ok(Vec::new());
        }
        self.last_now = now;
        if self.handshake_started.is_none() {
            self.handshake_started = Some(now);
        }
        match self.poll_inner(now) {
            Ok(commands) => Ok(commands),
            Err(error) => {
                if self.state != ClientState::Closed {
                    self.state = ClientState::Rejected;
                    let message = error.to_string();
                    self.host.disconnected(&message);
                    self.close();
                }
                Err(error)
            }
        }
    }

    fn poll_inner(&mut self, now: u64) -> Result<Vec<ActorCommand>, ApplicationNetworkError> {
        if self.composition.is_none() && self.last_handshake.is_none_or(|last| now.saturating_sub(last) >= 1000) {
            let handshake = match self.token.clone() {
                None => UnifiedHandshake::Hello {
                    nonce: self.nonce.clone(),
                },
                Some(token) => UnifiedHandshake::Connect {
                    nonce: self.nonce.clone(),
                    token,
                },
            };
            self.send_handshake(&handshake)?;
        }
        loop {
            let event = self
                .transport
                .poll()
                .map_err(|error| network_error(error.to_string()))?;
            let Some(event) = event else {
                break;
            };
            match event {
                ReceiveEvent::Error { error } => return Err(network_error(error)),
                ReceiveEvent::Packet { from, payload, .. } => {
                    if !same_address(&from, &self.remote, true) {
                        continue;
                    }
                    if self.receive_handshake(&payload)? {
                        continue;
                    }
                    self.receive_packet(&payload, now)?;
                    if self.state == ClientState::Closed {
                        return Ok(Vec::new());
                    }
                }
                _ => {}
            }
        }
        let since = now.saturating_sub(self.last_received.or(self.handshake_started).unwrap_or(now));
        if since > 120000 {
            return Err(network_error("Unified connection timed out"));
        }
        if self.channel.is_some() {
            if self.state == ClientState::Active {
                let userinfo = (self.userinfo_command)();
                if userinfo != self.last_userinfo {
                    self.send_control(&UnifiedControl::Userinfo {
                        epoch: self.epoch,
                        value: userinfo.clone(),
                    })?;
                    self.last_userinfo = userinfo;
                }
                let batch = encode_unified_inputs(&UnifiedInputBatch {
                    epoch: self.epoch,
                    commands: self.inputs.values().cloned().collect(),
                });
                self.channel
                    .as_mut()
                    .expect("channel is present")
                    .queue_frame(&batch, 0)
                    .map_err(|error| network_error(error.to_string()))?;
            }
            let datagrams = self
                .channel
                .as_mut()
                .expect("channel is present")
                .flush(now as f64)
                .map_err(|error| network_error(error.to_string()))?;
            for bytes in datagrams {
                self.transport
                    .send(&self.remote, &bytes)
                    .map_err(|error| network_error(error.to_string()))?;
            }
        }
        Ok(Vec::new())
    }

    /// Handle a handshake datagram, returning whether it was one.
    fn receive_handshake(&mut self, payload: &[u8]) -> Result<bool, ApplicationNetworkError> {
        let Some(handshake) = decode_unified_handshake(payload) else {
            return Ok(false);
        };
        if let UnifiedHandshake::Challenge { nonce, token } = &handshake {
            if nonce == &self.nonce && self.composition.is_none() {
                if self.token.as_ref().is_some_and(|current| current != token) {
                    return Ok(true);
                }
                self.token = Some(token.clone());
                if self.channel.is_none() {
                    let ceiling = self.transport.max_datagram_bytes().unwrap_or(1200);
                    let channel = UnifiedChannel::new(
                        token,
                        UnifiedChannelLimits {
                            datagram_bytes: ceiling.min(1200),
                            ..Default::default()
                        },
                    )
                    .map_err(|error| network_error(error.to_string()))?;
                    self.channel = Some(channel);
                }
                self.state = ClientState::Connecting;
                self.last_handshake = Some(self.last_now);
                let payload = encode_unified_handshake(&UnifiedHandshake::Connect {
                    nonce: self.nonce.clone(),
                    token: token.clone(),
                });
                self.transport
                    .send(&self.remote, &payload)
                    .map_err(|error| network_error(error.to_string()))?;
            }
        }
        Ok(true)
    }

    /// Handle a channel datagram.
    fn receive_packet(&mut self, payload: &[u8], now: u64) -> Result<(), ApplicationNetworkError> {
        let token_matches = match (&self.channel, decode_unified_packet(payload)) {
            (Some(channel), Some(packet)) => packet_token(&packet) == channel.token(),
            _ => false,
        };
        if !token_matches {
            return Ok(());
        }
        self.last_received = Some(now);
        let deliveries = self
            .channel
            .as_mut()
            .expect("channel is present")
            .receive(payload, now as f64)
            .map_err(|error| network_error(error.to_string()))?;
        for delivery in deliveries {
            match delivery {
                UnifiedDelivery::Reliable { payload, .. } => {
                    if !self.receive_control(&payload)? {
                        return Ok(());
                    }
                }
                UnifiedDelivery::Frame { payload, .. } => {
                    let acknowledged = self
                        .host
                        .receive_frame(&payload)
                        .map_err(|error| network_error(error.to_string()))?;
                    if let Some(acknowledged) = acknowledged {
                        self.state = ClientState::Active;
                        self.acknowledged = self.acknowledged.max(acknowledged);
                        while self.inputs.first_key_value().is_some_and(|(sequence, _)| {
                            i64::try_from(*sequence).unwrap_or(i64::MAX) <= self.acknowledged
                        }) {
                            self.inputs.pop_first();
                        }
                        if self.inputs.is_empty() {
                            self.command_time = self.host.command_time_milliseconds();
                        }
                    }
                }
            }
        }
        Ok(())
    }

    /// Handle a reliable control, returning false when disconnected.
    fn receive_control(&mut self, payload: &[u8]) -> Result<bool, ApplicationNetworkError> {
        let control = decode_unified_control(payload).map_err(|error| network_error(error.to_string()))?;
        if let UnifiedControl::Disconnect { reason } = &control {
            self.state = ClientState::Rejected;
            self.host.disconnected(reason);
            self.close();
            return Ok(false);
        }
        if let UnifiedControl::Offer { epoch, composition, .. } = &control {
            if *epoch <= self.epoch {
                return Ok(true);
            }
            self.state = ClientState::Loading;
            self.epoch = *epoch;
            self.inputs.clear();
            self.acknowledged = -1;
            self.command_time = None;
            self.submitted_at = None;
            self.host
                .offer(&control)
                .map_err(|error| network_error(error.to_string()))?;
            self.composition = Some(composition.clone());
            let userinfo = (self.userinfo_command)();
            self.last_userinfo = userinfo.clone();
            self.send_control(&UnifiedControl::Ready {
                epoch: *epoch,
                composition: composition.digest.to_string(),
                userinfo,
            })?;
            return Ok(true);
        }
        let epoch_matches = match &control {
            UnifiedControl::Admitted { epoch, .. }
            | UnifiedControl::Resources { epoch, .. }
            | UnifiedControl::Components { epoch, .. }
            | UnifiedControl::Events { epoch, .. } => *epoch == self.epoch,
            _ => false,
        };
        if !epoch_matches {
            return Ok(true);
        }
        match &control {
            UnifiedControl::Admitted { .. } => self.host.admitted(&control),
            UnifiedControl::Resources { epoch, resources } => self
                .host
                .declare(*epoch, resources)
                .map_err(|error| network_error(error.to_string()))?,
            UnifiedControl::Components { epoch, update } => self
                .host
                .receive_components(*epoch, update)
                .map_err(|error| network_error(error.to_string()))?,
            UnifiedControl::Events {
                epoch,
                frame,
                payload,
                simulation,
            } => {
                let frame =
                    i32::try_from(*frame).map_err(|_| network_error("Unified event frame exceeds its range"))?;
                self.host
                    .receive_events(*epoch, frame, payload, simulation)
                    .map_err(|error| network_error(error.to_string()))?;
            }
            _ => return Err(network_error("Unexpected unified server control")),
        }
        Ok(true)
    }

    /// Close the client (donor `close`).
    pub fn close(&mut self) {
        if self.state == ClientState::Closed {
            return;
        }
        if self.channel.as_ref().is_some_and(|channel| !channel.closed()) && self.epoch != 0 {
            let _ = self.send_control(&UnifiedControl::Disconnect {
                reason: "Client disconnected".to_string(),
            });
            if let Some(channel) = self.channel.as_mut() {
                if let Ok(datagrams) = channel.flush(self.last_now as f64) {
                    for bytes in datagrams {
                        let _ = self.transport.send(&self.remote, &bytes);
                    }
                }
            }
        }
        self.state = ClientState::Closed;
        self.host.close();
        if let Some(channel) = self.channel.as_mut() {
            channel.close();
        }
        self.inputs.clear();
        self.transport.close();
    }
}

fn packet_token(packet: &UnifiedPacket) -> &str {
    match packet {
        UnifiedPacket::Ack { token, .. }
        | UnifiedPacket::Reliable { token, .. }
        | UnifiedPacket::Frame { token, .. } => token,
    }
}

impl<T, H, U> ApplicationNetwork for UnifiedClientNetwork<T, H, U>
where
    T: DatagramTransport<Address = NetworkAddress>,
    H: UnifiedRemoteHost + 'static,
    H::Content: 'static,
    H::Media: 'static,
    U: FnMut() -> String,
{
    fn role(&self) -> ApplicationNetworkRole {
        ApplicationNetworkRole::Client
    }

    fn phase(&self) -> ApplicationNetworkPhase {
        match self.state {
            ClientState::Handshake => ApplicationNetworkPhase::Challenging,
            ClientState::Connecting => ApplicationNetworkPhase::Connecting,
            ClientState::Loading => ApplicationNetworkPhase::Loading,
            ClientState::Active => ApplicationNetworkPhase::Active,
            ClientState::Closed => ApplicationNetworkPhase::Closed,
            ClientState::Rejected => ApplicationNetworkPhase::Rejected,
        }
    }

    fn wire(&self) -> WireSelection {
        let Some(composition) = self.composition.as_ref() else {
            panic!("Unified composition has not been negotiated");
        };
        let digest = wire_digest(&composition.digest);
        let (namespace, name) = UNIFIED_SNAPSHOT_SCHEMA.split_once(':').unwrap_or(("", ""));
        WireSelection::Unified {
            version: 1,
            composition: digest,
            snapshot_schema: ProviderId::new(namespace, name),
        }
    }

    fn poll(&mut self, now_milliseconds: u64) -> Result<Vec<ActorCommand>, ApplicationNetworkError> {
        UnifiedClientNetwork::poll(self, now_milliseconds)
    }

    fn submit(&mut self, commands: &[ActorCommand], now_milliseconds: u64) -> Result<(), ApplicationNetworkError> {
        let elapsed = self
            .submitted_at
            .map_or(0.0, |submitted| now_milliseconds.saturating_sub(submitted) as f64);
        self.submit_timed(commands, now_milliseconds, elapsed)
    }

    fn publish(
        &mut self,
        _output: &qa_world::session::SimulationOutput,
        _events: &[super::types::NetworkPresentationEvent],
        _now_milliseconds: u64,
    ) -> Result<(), ApplicationNetworkError> {
        Err(network_error("Unified client cannot publish authoritative state"))
    }

    fn close(&mut self) {
        UnifiedClientNetwork::close(self);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    use std::sync::{Arc, Mutex};

    use super::super::remote_unified::support as remote;
    use qa_net::common::transport::TransportError;

    const TOKEN: &str = "0123456789abcdef0123456789abcdef";

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
                id: "client".to_string(),
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

    fn server_address() -> NetworkAddress {
        NetworkAddress::Loopback {
            id: "server".to_string(),
        }
    }

    #[allow(clippy::type_complexity)]
    fn harness() -> (
        UnifiedClientNetwork<SharedTransport, remote::TestHost, impl FnMut() -> String>,
        Arc<TestTransport>,
        Arc<Mutex<String>>,
    ) {
        let transport = Arc::new(TestTransport::default());
        let (host, _) = remote::harness();
        let userinfo = Arc::new(Mutex::new("player".to_string()));
        let current = userinfo.clone();
        let client = UnifiedClientNetwork::new(UnifiedClientOptions {
            transport: SharedTransport(transport.clone()),
            remote: server_address(),
            host,
            userinfo: move || current.lock().unwrap().clone(),
        });
        (client, transport, userinfo)
    }

    fn feed(transport: &TestTransport, from: &NetworkAddress, payload: Vec<u8>) {
        transport.inbox.lock().unwrap().push_back(ReceiveEvent::Packet {
            from: from.clone(),
            payload,
            received_at: 0.0,
        });
    }

    fn server_channel() -> UnifiedChannel {
        UnifiedChannel::new(
            TOKEN,
            UnifiedChannelLimits {
                datagram_bytes: 1200,
                ..Default::default()
            },
        )
        .unwrap()
    }

    fn server_send(server: &mut UnifiedChannel, control: &UnifiedControl) -> Vec<Vec<u8>> {
        server.queue_reliable(&encode_unified_control(control)).unwrap();
        server.flush(0.0).unwrap()
    }

    /// Feed client datagrams into the server channel, returning deliveries.
    fn drain_to_server(server: &mut UnifiedChannel, transport: &TestTransport) -> Vec<UnifiedDelivery> {
        let mut deliveries = Vec::new();
        for (_, bytes) in transport.sent.lock().unwrap().drain(..) {
            if decode_unified_handshake(&bytes).is_some() {
                continue;
            }
            if decode_unified_packet(&bytes).is_some() {
                deliveries.extend(server.receive(&bytes, 1.0).unwrap());
            }
        }
        deliveries
    }

    #[test]
    fn hello_challenge_connect_handshake() {
        let (mut client, transport, _) = harness();
        client.poll(0).unwrap();
        let sent = transport.sent.lock().unwrap();
        assert_eq!(sent.len(), 1);
        let hello = decode_unified_handshake(&sent[0].1).expect("hello");
        let nonce = match hello {
            UnifiedHandshake::Hello { nonce } => nonce,
            _ => panic!("expected hello"),
        };
        assert_eq!(nonce.len(), 32);
        drop(sent);
        feed(
            &transport,
            &server_address(),
            encode_unified_handshake(&UnifiedHandshake::Challenge {
                nonce: nonce.clone(),
                token: TOKEN.to_string(),
            }),
        );
        client.poll(1).unwrap();
        assert_eq!(client.phase(), ApplicationNetworkPhase::Connecting);
        let sent = transport.sent.lock().unwrap();
        assert_eq!(sent.len(), 2);
        let connect = decode_unified_handshake(&sent[1].1).expect("connect");
        assert!(matches!(connect, UnifiedHandshake::Connect { .. }));
    }

    #[test]
    fn foreign_challenge_is_ignored() {
        let (mut client, transport, _) = harness();
        client.poll(0).unwrap();
        feed(
            &transport,
            &server_address(),
            encode_unified_handshake(&UnifiedHandshake::Challenge {
                nonce: "f".repeat(32),
                token: TOKEN.to_string(),
            }),
        );
        feed(
            &transport,
            &NetworkAddress::Loopback {
                id: "stranger".to_string(),
            },
            encode_unified_handshake(&UnifiedHandshake::Challenge {
                nonce: "0".repeat(32),
                token: TOKEN.to_string(),
            }),
        );
        client.poll(1).unwrap();
        assert_eq!(client.phase(), ApplicationNetworkPhase::Challenging);
        assert_eq!(transport.sent.lock().unwrap().len(), 1);
    }

    fn connect(
        client: &mut UnifiedClientNetwork<SharedTransport, remote::TestHost, impl FnMut() -> String>,
        transport: &TestTransport,
    ) {
        client.poll(0).unwrap();
        let nonce = {
            let sent = transport.sent.lock().unwrap();
            match decode_unified_handshake(&sent[0].1).expect("hello") {
                UnifiedHandshake::Hello { nonce } => nonce,
                _ => panic!("expected hello"),
            }
        };
        feed(
            transport,
            &server_address(),
            encode_unified_handshake(&UnifiedHandshake::Challenge {
                nonce,
                token: TOKEN.to_string(),
            }),
        );
        client.poll(1).unwrap();
        assert_eq!(client.phase(), ApplicationNetworkPhase::Connecting);
    }

    #[test]
    fn offer_admits_and_sends_ready() {
        let (mut client, transport, _) = harness();
        connect(&mut client, &transport);
        let mut server = server_channel();
        for bytes in server_send(&mut server, &remote::offer(3)) {
            feed(&transport, &server_address(), bytes);
        }
        client.poll(2).unwrap();
        assert_eq!(client.phase(), ApplicationNetworkPhase::Loading);
        assert_eq!(client.host().epoch(), 3);
        match client.wire() {
            WireSelection::Unified {
                version, composition, ..
            } => {
                assert_eq!(version, 1);
                assert_eq!(composition.to_string().len(), 64);
            }
            _ => panic!("expected unified wire"),
        }
        let deliveries = drain_to_server(&mut server, &transport);
        let ready = deliveries.iter().any(|delivery| match delivery {
            UnifiedDelivery::Reliable { payload, .. } => {
                matches!(
                    decode_unified_control(payload).unwrap(),
                    UnifiedControl::Ready { epoch: 3, .. }
                )
            }
            _ => false,
        });
        assert!(ready, "client sends ready");
    }

    #[test]
    fn frame_activates_client() {
        let (mut client, transport, _) = harness();
        connect(&mut client, &transport);
        let mut server = server_channel();
        for bytes in server_send(&mut server, &remote::offer(3)) {
            feed(&transport, &server_address(), bytes);
        }
        client.poll(2).unwrap();
        for bytes in server_send(&mut server, &remote::admitted(3)) {
            feed(&transport, &server_address(), bytes);
        }
        client.poll(3).unwrap();
        let frame = remote::frame_bytes(client.host(), 3);
        server.queue_frame(&frame, 0).unwrap();
        for bytes in server.flush(4.0).unwrap() {
            feed(&transport, &server_address(), bytes);
        }
        client.poll(4).unwrap();
        assert_eq!(client.phase(), ApplicationNetworkPhase::Active);
        assert!(client.host().player().is_some());
    }

    #[test]
    fn submit_queues_inputs_for_flush() {
        let (mut client, transport, _) = harness();
        connect(&mut client, &transport);
        let mut server = server_channel();
        for bytes in server_send(&mut server, &remote::offer(3)) {
            feed(&transport, &server_address(), bytes);
        }
        client.poll(2).unwrap();
        for bytes in server_send(&mut server, &remote::admitted(3)) {
            feed(&transport, &server_address(), bytes);
        }
        client.poll(3).unwrap();
        let frame = remote::frame_bytes(client.host(), 3);
        server.queue_frame(&frame, 0).unwrap();
        for bytes in server.flush(4.0).unwrap() {
            feed(&transport, &server_address(), bytes);
        }
        client.poll(4).unwrap();
        let actor = client.host().player().expect("player").actor;
        client
            .submit(
                &[ActorCommand {
                    actor,
                    source: CommandSource::LocalSeat {
                        seat: qa_core::identity::IdentityOwner::create("seat").unwrap().seat(0),
                    },
                    sequence: 2,
                    command: UserCommand::Q3 {
                        server_time_milliseconds: 0.0,
                        angle_words: [0.0, 0.0, 0.0],
                        buttons: 0.0,
                        weapon: 1.0,
                        forward_move: 0.0,
                        right_move: 0.0,
                        up_move: 0.0,
                    },
                    arsenal: None,
                }],
                100,
            )
            .unwrap();
        client.poll(101).unwrap();
        let deliveries = drain_to_server(&mut server, &transport);
        let mut batches = 0;
        for delivery in deliveries {
            if let UnifiedDelivery::Frame { payload, .. } = delivery {
                let batch = super::super::unified_control::decode_unified_inputs(&payload).unwrap();
                if !batch.commands.is_empty() {
                    batches += 1;
                    assert_eq!(batch.commands[0].sequence, 2);
                }
            }
        }
        assert_eq!(batches, 1);
    }

    #[test]
    fn foreign_input_is_rejected() {
        let (mut client, transport, _) = harness();
        connect(&mut client, &transport);
        let mut server = server_channel();
        for bytes in server_send(&mut server, &remote::offer(3)) {
            feed(&transport, &server_address(), bytes);
        }
        client.poll(2).unwrap();
        for bytes in server_send(&mut server, &remote::admitted(3)) {
            feed(&transport, &server_address(), bytes);
        }
        client.poll(3).unwrap();
        let frame = remote::frame_bytes(client.host(), 3);
        server.queue_frame(&frame, 0).unwrap();
        for bytes in server.flush(4.0).unwrap() {
            feed(&transport, &server_address(), bytes);
        }
        client.poll(4).unwrap();
        let foreign = qa_core::identity::IdentityOwner::create("foreign").unwrap().actor(9, 9);
        let error = client
            .submit(
                &[ActorCommand {
                    actor: foreign,
                    source: CommandSource::LocalSeat {
                        seat: qa_core::identity::IdentityOwner::create("seat").unwrap().seat(0),
                    },
                    sequence: 0,
                    command: UserCommand::Q3 {
                        server_time_milliseconds: 0.0,
                        angle_words: [0.0, 0.0, 0.0],
                        buttons: 0.0,
                        weapon: 1.0,
                        forward_move: 0.0,
                        right_move: 0.0,
                        up_move: 0.0,
                    },
                    arsenal: None,
                }],
                100,
            )
            .unwrap_err();
        assert_eq!(error.to_string(), "Unified input belongs to another player");
    }

    #[test]
    fn transport_error_rejects() {
        let (mut client, transport, _) = harness();
        connect(&mut client, &transport);
        transport.inbox.lock().unwrap().push_back(ReceiveEvent::Error {
            error: "boom".to_string(),
        });
        let error = client.poll(2).unwrap_err();
        assert_eq!(error.to_string(), "boom");
        assert_eq!(client.phase(), ApplicationNetworkPhase::Closed);
    }

    #[test]
    fn silence_times_out() {
        let (mut client, _, _) = harness();
        client.poll(0).unwrap();
        let error = client.poll(200_000).unwrap_err();
        assert_eq!(error.to_string(), "Unified connection timed out");
        assert_eq!(client.phase(), ApplicationNetworkPhase::Closed);
    }

    #[test]
    fn disconnect_control_closes_cleanly() {
        let (mut client, transport, _) = harness();
        connect(&mut client, &transport);
        let mut server = server_channel();
        for bytes in server_send(
            &mut server,
            &UnifiedControl::Disconnect {
                reason: "bye".to_string(),
            },
        ) {
            feed(&transport, &server_address(), bytes);
        }
        let commands = client.poll(2).unwrap();
        assert!(commands.is_empty());
        assert_eq!(client.phase(), ApplicationNetworkPhase::Closed);
    }

    #[test]
    fn userinfo_change_flushes() {
        let (mut client, transport, userinfo) = harness();
        connect(&mut client, &transport);
        let mut server = server_channel();
        for bytes in server_send(&mut server, &remote::offer(3)) {
            feed(&transport, &server_address(), bytes);
        }
        client.poll(2).unwrap();
        for bytes in server_send(&mut server, &remote::admitted(3)) {
            feed(&transport, &server_address(), bytes);
        }
        client.poll(3).unwrap();
        let frame = remote::frame_bytes(client.host(), 3);
        server.queue_frame(&frame, 0).unwrap();
        for bytes in server.flush(4.0).unwrap() {
            feed(&transport, &server_address(), bytes);
        }
        client.poll(4).unwrap();
        drain_to_server(&mut server, &transport);
        *userinfo.lock().unwrap() = "renamed".to_string();
        client.poll(5).unwrap();
        let deliveries = drain_to_server(&mut server, &transport);
        let updated = deliveries.iter().any(|delivery| match delivery {
            UnifiedDelivery::Reliable { payload, .. } => {
                matches!(
                    decode_unified_control(payload).unwrap(),
                    UnifiedControl::Userinfo { .. }
                )
            }
            _ => false,
        });
        assert!(updated, "client flushes userinfo change");
    }

    #[test]
    #[should_panic(expected = "Unified composition has not been negotiated")]
    fn wire_before_offer_panics() {
        let (client, _, _) = harness();
        let _ = client.wire();
    }
}
