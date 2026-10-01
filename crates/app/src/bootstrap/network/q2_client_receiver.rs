//! Quake II client record receiver.
//!
//! Port of `/home/buzzkill/Projects/quake-typescript/src/app/bootstrap/network/q2-client-receiver.ts`
//! (`Q2ClientReceiver`, `Q2ClientReceiverHost`, `Q2ClientReceiverSource`).
//! The donor is asynchronous; this port resolves every step inline. Server
//! messages decode through [`Q2ServerMessageReader`](qa_net::q2_net::Q2ServerMessageReader);
//! recording seeds re-encode through `qa-net` (`encodeQ2ServerEvent`,
//! `encodeQ2Frame`). The donor source's `command`/`resetCommands` callbacks
//! become returned [`Q2ReceiverActions`] the network owner drains in order,
//! and `closed()` becomes a `transport_closed` parameter, since a sync port
//! cannot reenter its owner mid-receive.

use std::collections::{HashMap, HashSet};

use qa_net::protocol::ProtocolIdentity;
use qa_net::q2::EntityState;
use qa_net::q2_net::{
    encode_q2_frame, parse_q2_token, ParseState, Q2ReadMode, Q2ServerData, Q2ServerEvent, Q2ServerMessageOptions,
    Q2ServerMessageReader, Q2ServerRecord, Q2WireFrame, Q2_TOKEN_MAX,
};
use qa_net::q2_svc::encode_q2_server_event;
use thiserror::Error;

use super::q2_downloads::{
    Q2ApplicationClientDownloads, Q2DownloadBlock, Q2DownloadError, Q2DownloadOutcome, Q2DownloadPreparation,
};
use super::types::{ApplicationNetworkPhase, Q2ApplicationClientHost, Q2ApplicationGameState};
use crate::bootstrap::demo_recording::{
    DemoRecordingIdentity, DemoRecordingPacket, DemoRecordingSeed, Q2ProRevision, Q2ProtocolIdentity, R1Q2Revision,
};

/// Client receiver failure.
#[derive(Debug, Error)]
pub enum Q2ClientReceiverError {
    /// Policy or protocol failure with donor text.
    #[error("{0}")]
    Message(String),
    /// Wire failure.
    #[error(transparent)]
    Net(#[from] qa_net::q2_net::Q2NetError),
    /// Download failure.
    #[error(transparent)]
    Download(#[from] Q2DownloadError),
}

/// Map an application protocol identity to its wire identity.
#[must_use]
pub fn q2_receiver_protocol(protocol: &Q2ProtocolIdentity) -> ProtocolIdentity {
    match *protocol {
        Q2ProtocolIdentity::Classic => ProtocolIdentity::Q2Classic,
        Q2ProtocolIdentity::R1Q2 { revision } => ProtocolIdentity::Q2R1q2 {
            revision: u32::from(revision.revision()),
        },
        Q2ProtocolIdentity::Q2Pro { revision } => ProtocolIdentity::Q2Q2pro {
            revision: u32::from(revision.revision()),
        },
        Q2ProtocolIdentity::Rerelease => ProtocolIdentity::Q2Rerelease,
        Q2ProtocolIdentity::Kex => ProtocolIdentity::Q2Kex,
        Q2ProtocolIdentity::KexDemo => ProtocolIdentity::Q2KexDemo,
    }
}

/// Map a wire identity back to its recording identity.
fn seed_protocol(protocol: &ProtocolIdentity) -> Result<Q2ProtocolIdentity, Q2ClientReceiverError> {
    let unsupported =
        |version: u32| Q2ClientReceiverError::Message(format!("Unsupported recorded Q2 protocol {version}"));
    match *protocol {
        ProtocolIdentity::Q2Classic => Ok(Q2ProtocolIdentity::Classic),
        ProtocolIdentity::Q2R1q2 { revision } => match revision {
            1903 => Ok(Q2ProtocolIdentity::R1Q2 {
                revision: R1Q2Revision::R1903,
            }),
            1904 => Ok(Q2ProtocolIdentity::R1Q2 {
                revision: R1Q2Revision::R1904,
            }),
            1905 => Ok(Q2ProtocolIdentity::R1Q2 {
                revision: R1Q2Revision::R1905,
            }),
            _ => Err(unsupported(35)),
        },
        ProtocolIdentity::Q2Q2pro { revision } => {
            let revision = match revision {
                1015 => Q2ProRevision::R1015,
                1016 => Q2ProRevision::R1016,
                1017 => Q2ProRevision::R1017,
                1018 => Q2ProRevision::R1018,
                1019 => Q2ProRevision::R1019,
                1020 => Q2ProRevision::R1020,
                1021 => Q2ProRevision::R1021,
                1022 => Q2ProRevision::R1022,
                1023 => Q2ProRevision::R1023,
                1024 => Q2ProRevision::R1024,
                1025 => Q2ProRevision::R1025,
                1026 => Q2ProRevision::R1026,
                _ => return Err(unsupported(36)),
            };
            Ok(Q2ProtocolIdentity::Q2Pro { revision })
        }
        ProtocolIdentity::Q2Rerelease => Ok(Q2ProtocolIdentity::Rerelease),
        ProtocolIdentity::Q2Kex => Ok(Q2ProtocolIdentity::Kex),
        ProtocolIdentity::Q2KexDemo => Ok(Q2ProtocolIdentity::KexDemo),
        // The Rust application identity has no private-classic variant; the
        // wire encoding is the classic one.
        ProtocolIdentity::Q2PrivateClassic => Ok(Q2ProtocolIdentity::Classic),
        other => Err(unsupported(other.version())),
    }
}

/// Server frames per second (`data.serverFps ?? 10`).
fn server_fps(data: &Q2ServerData) -> f64 {
    match data {
        Q2ServerData::Rerelease(data) => f64::from(data.server_fps),
        Q2ServerData::Kex(data) => f64::from(data.server_fps),
        _ => 10.0,
    }
}

/// Clone server data with the attract loop set (recording seeds).
fn seeded_server_data(data: &Q2ServerData) -> Q2ServerData {
    match data.clone() {
        Q2ServerData::Vanilla(mut inner) => {
            inner.attractloop = true;
            Q2ServerData::Vanilla(inner)
        }
        Q2ServerData::R1Q2(mut inner) => {
            inner.attractloop = true;
            Q2ServerData::R1Q2(inner)
        }
        Q2ServerData::Q2Pro(mut inner) => {
            inner.attractloop = true;
            Q2ServerData::Q2Pro(inner)
        }
        Q2ServerData::Rerelease(mut inner) => {
            inner.attractloop = true;
            Q2ServerData::Rerelease(inner)
        }
        Q2ServerData::Kex(mut inner) => {
            inner.attractloop = true;
            Q2ServerData::Kex(inner)
        }
    }
}

/// Client receiver host (`Q2ClientReceiverHost`).
pub trait Q2ClientReceiverHost {
    /// Wire protocol.
    fn protocol(&self) -> Q2ProtocolIdentity;
    /// Server message options.
    fn message_options(&self) -> Q2ServerMessageOptions;
    /// Handle server data (donor async; resolves inline here).
    fn server_data(&mut self, data: &Q2ServerData, assert_current: &dyn Fn());
    /// Resolve game state (donor async; resolves inline here).
    fn game_state(&mut self, state: &Q2ApplicationGameState);
    /// Publish a decoded frame.
    fn frame(&mut self, frame: &Q2WireFrame, records: &[Q2ServerRecord], now_milliseconds: u64);
    /// Handle raw server records.
    fn records(&mut self, records: &[Q2ServerRecord]);
    /// Handle a disconnect.
    fn disconnected(&mut self, reason: &str);
    /// Print client text.
    fn print(&mut self, text: &str);
}

impl<T: Q2ApplicationClientHost> Q2ClientReceiverHost for T {
    fn protocol(&self) -> Q2ProtocolIdentity {
        Q2ApplicationClientHost::protocol(self)
    }

    fn message_options(&self) -> Q2ServerMessageOptions {
        Q2ApplicationClientHost::message_options(self)
    }

    fn server_data(&mut self, data: &Q2ServerData, assert_current: &dyn Fn()) {
        Q2ApplicationClientHost::server_data(self, data, assert_current);
    }

    fn game_state(&mut self, state: &Q2ApplicationGameState) {
        Q2ApplicationClientHost::game_state(self, state);
    }

    fn frame(&mut self, frame: &Q2WireFrame, records: &[Q2ServerRecord], now_milliseconds: u64) {
        Q2ApplicationClientHost::frame(self, frame, records, now_milliseconds);
    }

    fn records(&mut self, records: &[Q2ServerRecord]) {
        Q2ApplicationClientHost::records(self, records);
    }

    fn disconnected(&mut self, reason: &str) {
        Q2ApplicationClientHost::disconnected(self, reason);
    }

    fn print(&mut self, text: &str) {
        Q2ApplicationClientHost::print(self, text);
    }
}

/// Client receiver source (`Q2ClientReceiverSource`).
///
/// The donor's `command`/`resetCommands` callbacks become returned actions
/// and `closed()` becomes a per-call parameter; only downloads stay behind
/// the source.
pub trait Q2ClientReceiverSource {
    /// Whether the source is a demo (no downloads, commands, or resets).
    fn is_demo(&self) -> bool;
    /// Client downloads, when the source accepts them.
    fn downloads(&mut self) -> Option<&mut dyn Q2ApplicationClientDownloads>;
}

/// Demo receiver source (`{ kind: 'demo' }`).
#[derive(Debug, Clone, Copy, Default)]
pub struct Q2DemoReceiverSource;

impl Q2ClientReceiverSource for Q2DemoReceiverSource {
    fn is_demo(&self) -> bool {
        true
    }

    fn downloads(&mut self) -> Option<&mut dyn Q2ApplicationClientDownloads> {
        None
    }
}

/// Network actions for the owner to drain in order.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Q2ReceiverActions {
    /// Reliable commands to send.
    pub commands: Vec<String>,
    /// Whether to reset pending move commands.
    pub reset_commands: bool,
}

/// Receive outcome: decoded records plus owner actions.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Q2ReceiverOutcome {
    /// Decoded records.
    pub records: Vec<Q2ServerRecord>,
    /// Owner actions.
    pub actions: Q2ReceiverActions,
}

/// Tokenize one command line (`tokens`).
fn tokenize(text: &str) -> Vec<String> {
    let mut cursor = ParseState::new(text);
    let mut result = Vec::new();
    while cursor.index < cursor.data.len() {
        let before = cursor.index;
        let value = parse_q2_token(&mut cursor, Q2_TOKEN_MAX);
        if cursor.index == before {
            break;
        }
        result.push(value);
    }
    result
}

/// Parse a signon number (`integer`).
fn signon_integer(text: Option<&str>) -> Result<i32, Q2ClientReceiverError> {
    let Some(text) = text else {
        return Err(Q2ClientReceiverError::Message("Invalid Q2 signon number".to_string()));
    };
    if text.is_empty()
        || !text
            .bytes()
            .enumerate()
            .all(|(index, byte)| byte.is_ascii_digit() || (index == 0 && byte == b'-'))
        || text == "-"
    {
        return Err(Q2ClientReceiverError::Message("Invalid Q2 signon number".to_string()));
    }
    text.parse::<i32>()
        .map_err(|_| Q2ClientReceiverError::Message("Q2 signon number outside range".to_string()))
}

/// Quake II client record receiver (`Q2ClientReceiver`).
pub struct Q2ClientReceiver<H, S> {
    host: H,
    source: S,
    /// Decoded server message reader.
    pub reader: Q2ServerMessageReader,
    state: ApplicationNetworkPhase,
    server_data: Option<Q2ServerData>,
    last_frame: i32,
    recorded_time: Option<f64>,
    demo_disconnected: bool,
    pending_game_state: Option<Q2ApplicationGameState>,
    loading_generation: u32,
}

impl<H: Q2ClientReceiverHost, S: Q2ClientReceiverSource> Q2ClientReceiver<H, S> {
    /// Build a receiver over a host and source.
    pub fn new(host: H, source: S) -> Result<Self, Q2ClientReceiverError> {
        let protocol = q2_receiver_protocol(&host.protocol());
        let mut options = host.message_options();
        options.read_mode = if source.is_demo() {
            Q2ReadMode::Demo
        } else {
            Q2ReadMode::Network
        };
        Ok(Self {
            host,
            source,
            reader: Q2ServerMessageReader::new(protocol, options, HashSet::new(), None)?,
            state: ApplicationNetworkPhase::Loading,
            server_data: None,
            last_frame: -1,
            recorded_time: None,
            demo_disconnected: false,
            pending_game_state: None,
            loading_generation: 0,
        })
    }

    /// Connection phase.
    #[must_use]
    pub fn phase(&self) -> ApplicationNetworkPhase {
        self.state
    }

    /// Acknowledged frame.
    #[must_use]
    pub fn acknowledged_frame(&self) -> i32 {
        self.last_frame
    }

    /// Loading generation.
    #[must_use]
    pub fn world_generation(&self) -> u32 {
        self.loading_generation
    }

    /// Recorded time in milliseconds, for demos.
    #[must_use]
    pub fn recorded_time_milliseconds(&self) -> Option<f64> {
        self.recorded_time
    }

    /// Whether a demo recorded a disconnect.
    #[must_use]
    pub fn disconnected_demo(&self) -> bool {
        self.demo_disconnected
    }

    /// Borrow the host.
    #[must_use]
    pub fn host(&self) -> &H {
        &self.host
    }

    /// Mutably borrow the host.
    pub fn host_mut(&mut self) -> &mut H {
        &mut self.host
    }

    /// Borrow the source.
    #[must_use]
    pub fn source(&self) -> &S {
        &self.source
    }

    /// Mutably borrow the source.
    pub fn source_mut(&mut self) -> &mut S {
        &mut self.source
    }

    /// Build a recording seed from the admitted game state (`seed`).
    pub fn seed(&mut self) -> Result<DemoRecordingSeed, Q2ClientReceiverError> {
        let Some(data) = self.server_data.clone() else {
            return Err(Q2ClientReceiverError::Message(
                "Recording requires an active Q2 server".to_string(),
            ));
        };
        if self.state != ApplicationNetworkPhase::Active {
            return Err(Q2ClientReceiverError::Message(
                "Recording requires an active Q2 server".to_string(),
            ));
        }
        let protocol = seed_protocol(&self.reader.wire.protocol())?;
        let mut packets = Vec::new();
        let mut push = |message: Vec<u8>| packets.push(DemoRecordingPacket::Q2 { message });
        push(encode_q2_server_event(
            &mut self.reader.wire,
            &Q2ServerEvent::ServerData {
                data: Box::new(seeded_server_data(&data)),
            },
        )?);
        let mut strings: Vec<(u16, String)> = self
            .reader
            .config_strings
            .iter()
            .map(|(index, value)| (*index, value.clone()))
            .collect();
        strings.sort_by_key(|(index, _)| *index);
        for (index, value) in strings {
            push(encode_q2_server_event(
                &mut self.reader.wire,
                &Q2ServerEvent::ConfigString { index, value },
            )?);
        }
        let seat = self.reader.seat();
        let baselines: HashMap<u16, EntityState> = self.reader.history(seat).baselines.clone();
        let mut entities: Vec<(u16, EntityState)> = baselines
            .iter()
            .map(|(number, entity)| (*number, entity.clone()))
            .collect();
        entities.sort_by_key(|(number, _)| *number);
        for (_, entity) in entities {
            push(encode_q2_server_event(
                &mut self.reader.wire,
                &Q2ServerEvent::Baseline { entity },
            )?);
        }
        push(encode_q2_server_event(
            &mut self.reader.wire,
            &Q2ServerEvent::CommandText {
                text: "precache\n".to_string(),
            },
        )?);
        let kex = matches!(
            self.reader.wire.protocol(),
            ProtocolIdentity::Q2Kex | ProtocolIdentity::Q2KexDemo
        );
        for (frame_seat, frame) in self.reader.latest_frames() {
            if kex {
                push(encode_q2_server_event(
                    &mut self.reader.wire,
                    &Q2ServerEvent::Seat { seat: frame_seat },
                )?);
            }
            push(encode_q2_frame(&self.reader.wire, &frame, None, &baselines, 0)?);
        }
        if kex {
            push(encode_q2_server_event(
                &mut self.reader.wire,
                &Q2ServerEvent::Seat { seat },
            )?);
        }
        Ok(DemoRecordingSeed {
            identity: DemoRecordingIdentity::Q2 { protocol },
            packets,
        })
    }

    /// Require the next full frame (`requestFullFrame`).
    pub fn request_full_frame(&mut self) {
        self.last_frame = -1;
    }

    /// Decode and handle one datagram (`receive`).
    pub fn receive(
        &mut self,
        bytes: &[u8],
        now_milliseconds: u64,
        transport_closed: bool,
    ) -> Result<Q2ReceiverOutcome, Q2ClientReceiverError> {
        if self.state == ApplicationNetworkPhase::Closed {
            return Ok(Q2ReceiverOutcome::default());
        }
        let records = self.reader.read(bytes)?;
        self.server_records(records, now_milliseconds, transport_closed)
    }

    /// Handle already decoded records (`receiveRecords`).
    pub fn receive_records(
        &mut self,
        records: Vec<Q2ServerRecord>,
        now_milliseconds: u64,
        transport_closed: bool,
    ) -> Result<Q2ReceiverOutcome, Q2ClientReceiverError> {
        if self.state == ApplicationNetworkPhase::Closed {
            return Ok(Q2ReceiverOutcome::default());
        }
        self.reader.accept_decoded(&records)?;
        self.server_records(records, now_milliseconds, transport_closed)
    }

    /// Close the receiver (`close`).
    pub fn close(&mut self) {
        self.cancel_loading();
        self.state = ApplicationNetworkPhase::Closed;
    }

    /// Retire the pending game state (`cancelLoading`).
    fn cancel_loading(&mut self) {
        self.loading_generation = self.loading_generation.wrapping_add(1);
        self.pending_game_state = None;
        if !self.source.is_demo() {
            if let Some(downloads) = self.source.downloads() {
                downloads.close();
            }
        }
    }

    /// Fail retired callbacks (`assertCurrent`).
    fn assert_current(&self, generation: u32, transport_closed: bool) -> Result<(), Q2ClientReceiverError> {
        if generation != self.loading_generation
            || self.state == ApplicationNetworkPhase::Closed
            || (!self.source.is_demo() && transport_closed)
        {
            return Err(Q2ClientReceiverError::Message(
                "Q2 server directory selection was retired".to_string(),
            ));
        }
        Ok(())
    }

    /// Prepare the pending game state (`prepareGameState`).
    pub fn prepare_game_state(&mut self, transport_closed: bool) -> Result<Q2ReceiverActions, Q2ClientReceiverError> {
        let actions = Q2ReceiverActions::default();
        let Some(state) = self.pending_game_state.clone() else {
            return Ok(actions);
        };
        let generation = self.loading_generation;
        let preparation = if self.source.is_demo() {
            Q2DownloadPreparation::Ready
        } else {
            match self.source.downloads() {
                Some(downloads) => downloads.prepare(&state)?,
                None => Q2DownloadPreparation::Ready,
            }
        };
        self.assert_current(generation, transport_closed)?;
        if self.pending_game_state.as_ref() != Some(&state) || preparation != Q2DownloadPreparation::Ready {
            return Ok(actions);
        }
        self.host.game_state(&state);
        self.assert_current(generation, transport_closed)?;
        if self.pending_game_state.as_ref() != Some(&state) {
            return Ok(actions);
        }
        self.pending_game_state = None;
        let mut actions = actions;
        if !self.source.is_demo() {
            actions.commands.push(format!("begin {}", state.data.servercount()));
        }
        self.state = ApplicationNetworkPhase::Active;
        Ok(actions)
    }

    /// Handle server command text (`serverCommands`).
    fn server_commands(
        &mut self,
        text: &str,
        transport_closed: bool,
        actions: &mut Q2ReceiverActions,
    ) -> Result<(), Q2ClientReceiverError> {
        for line in text.split(['\n', ';']) {
            let words = tokenize(line);
            let name = words.first().map(String::as_str);
            if name == Some("cmd")
                && (words.get(1).map(String::as_str) == Some("configstrings")
                    || words.get(1).map(String::as_str) == Some("baselines"))
            {
                if !self.source.is_demo() {
                    actions.commands.push(words[1..].join(" "));
                }
            } else if name == Some("precache") {
                let Some(data) = self.server_data.clone() else {
                    return Err(Q2ClientReceiverError::Message(
                        "Q2 precache refers to another server generation".to_string(),
                    ));
                };
                if (!self.source.is_demo() || words.get(1).is_some())
                    && signon_integer(words.get(1).map(String::as_str))? != data.servercount()
                {
                    return Err(Q2ClientReceiverError::Message(
                        "Q2 precache refers to another server generation".to_string(),
                    ));
                }
                self.cancel_loading();
                let seat = self.reader.seat();
                self.pending_game_state = Some(Q2ApplicationGameState {
                    data,
                    config_strings: self
                        .reader
                        .config_strings
                        .iter()
                        .map(|(index, value)| (u32::from(*index), value.clone()))
                        .collect(),
                    baselines: self
                        .reader
                        .history(seat)
                        .baselines
                        .iter()
                        .map(|(number, entity)| (u32::from(*number), entity.clone()))
                        .collect(),
                });
                let mut prepared = self.prepare_game_state(transport_closed)?;
                actions.commands.append(&mut prepared.commands);
                actions.reset_commands |= prepared.reset_commands;
            } else if name == Some("changing") {
                self.cancel_loading();
                self.last_frame = -1;
                self.state = ApplicationNetworkPhase::Loading;
            } else if !name.is_none_or(str::is_empty) {
                self.host
                    .print(&format!("Server command requires application binding: {line}\n"));
            }
        }
        Ok(())
    }

    /// Handle decoded server records (`serverRecords`).
    fn server_records(
        &mut self,
        records: Vec<Q2ServerRecord>,
        now: u64,
        transport_closed: bool,
    ) -> Result<Q2ReceiverOutcome, Q2ClientReceiverError> {
        let mut actions = Q2ReceiverActions::default();
        let is_demo = self.source.is_demo();
        for (index, record) in records.iter().enumerate() {
            match &record.event {
                Q2ServerEvent::ServerData { data } => {
                    self.cancel_loading();
                    let generation = self.loading_generation;
                    self.assert_current(generation, transport_closed)?;
                    // Sync hosts resolve inline, so retirement cannot
                    // interleave; the closure only carries the donor hook.
                    self.host.server_data(data, &|| {});
                    self.assert_current(generation, transport_closed)?;
                    self.server_data = Some((**data).clone());
                    self.last_frame = -1;
                    self.recorded_time = None;
                    if !is_demo {
                        actions.reset_commands = true;
                    }
                    self.state = ApplicationNetworkPhase::Loading;
                }
                Q2ServerEvent::CommandText { text } => {
                    let text = text.clone();
                    self.server_commands(&text, transport_closed, &mut actions)?;
                    if self.state == ApplicationNetworkPhase::Closed {
                        return Ok(Q2ReceiverOutcome { records, actions });
                    }
                }
                Q2ServerEvent::Frame { frame } => {
                    self.last_frame = if frame.valid { frame.server_frame } else { -1 };
                    if frame.valid {
                        let fps = self.server_data.as_ref().map_or(10.0, server_fps);
                        let time = f64::from(frame.server_frame) * (1000.0 / fps);
                        self.recorded_time = Some(time);
                        let now = if is_demo { time as u64 } else { now };
                        self.host.frame(frame, &records, now);
                    }
                }
                Q2ServerEvent::Disconnect => {
                    self.cancel_loading();
                    self.state = ApplicationNetworkPhase::Closed;
                    if is_demo {
                        self.demo_disconnected = true;
                        self.host.records(&records[..=index]);
                        return Ok(Q2ReceiverOutcome { records, actions });
                    }
                    self.host.disconnected("Server disconnected");
                }
                Q2ServerEvent::Reconnect => {
                    self.cancel_loading();
                    self.last_frame = -1;
                    self.state = ApplicationNetworkPhase::Loading;
                    if !is_demo {
                        actions.commands.push("new".to_string());
                    }
                }
                Q2ServerEvent::Print { text, .. } => {
                    self.host.print(text);
                }
                Q2ServerEvent::Download { percent, bytes } if !is_demo => {
                    let block = Q2DownloadBlock {
                        percent: *percent,
                        bytes: bytes.clone(),
                    };
                    let outcome = match self.source.downloads() {
                        Some(downloads) => downloads.receive(&block)?,
                        None => Q2DownloadOutcome::Waiting,
                    };
                    if outcome == Q2DownloadOutcome::Complete {
                        let mut prepared = self.prepare_game_state(transport_closed)?;
                        actions.commands.append(&mut prepared.commands);
                        actions.reset_commands |= prepared.reset_commands;
                    }
                }
                _ => {}
            }
        }
        self.host.records(&records);
        Ok(Q2ReceiverOutcome { records, actions })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct ReceiverHost {
        prints: Vec<String>,
        frames: Vec<i32>,
        record_batches: Vec<usize>,
        game_states: u32,
        disconnected: Vec<String>,
    }

    impl ReceiverHost {
        fn new() -> Self {
            Self {
                prints: Vec::new(),
                frames: Vec::new(),
                record_batches: Vec::new(),
                game_states: 0,
                disconnected: Vec::new(),
            }
        }
    }

    impl Q2ClientReceiverHost for ReceiverHost {
        fn protocol(&self) -> Q2ProtocolIdentity {
            Q2ProtocolIdentity::Classic
        }

        fn message_options(&self) -> Q2ServerMessageOptions {
            Q2ServerMessageOptions::default()
        }

        fn server_data(&mut self, _data: &Q2ServerData, _assert_current: &dyn Fn()) {}

        fn game_state(&mut self, _state: &Q2ApplicationGameState) {
            self.game_states += 1;
        }

        fn frame(&mut self, frame: &Q2WireFrame, _records: &[Q2ServerRecord], _now: u64) {
            self.frames.push(frame.server_frame);
        }

        fn records(&mut self, records: &[Q2ServerRecord]) {
            self.record_batches.push(records.len());
        }

        fn disconnected(&mut self, reason: &str) {
            self.disconnected.push(reason.to_string());
        }

        fn print(&mut self, text: &str) {
            self.prints.push(text.to_string());
        }
    }

    struct NetworkSource {
        downloads: Option<FakeDownloads>,
    }

    struct FakeDownloads {
        prepared: u32,
        outcome: Q2DownloadOutcome,
    }

    impl Q2ApplicationClientDownloads for FakeDownloads {
        fn set_http_server(&mut self, _server: Option<String>) {}

        fn prepare(&mut self, _state: &Q2ApplicationGameState) -> Result<Q2DownloadPreparation, Q2DownloadError> {
            self.prepared += 1;
            Ok(Q2DownloadPreparation::Ready)
        }

        fn receive(&mut self, _block: &Q2DownloadBlock) -> Result<Q2DownloadOutcome, Q2DownloadError> {
            Ok(self.outcome)
        }

        fn close(&mut self) {}
    }

    impl Q2ClientReceiverSource for NetworkSource {
        fn is_demo(&self) -> bool {
            false
        }

        fn downloads(&mut self) -> Option<&mut dyn Q2ApplicationClientDownloads> {
            self.downloads
                .as_mut()
                .map(|downloads| downloads as &mut dyn Q2ApplicationClientDownloads)
        }
    }

    fn record(event: Q2ServerEvent) -> Q2ServerRecord {
        Q2ServerRecord {
            seat: 0,
            opcode: 0,
            raw: Vec::new(),
            event,
        }
    }

    fn server_data(count: i32) -> Q2ServerData {
        Q2ServerData::Vanilla(qa_net::q2::ServerData {
            servercount: count,
            attractloop: false,
            gamedir: "baseq2".to_string(),
            clientnum: 0,
            levelname: "base1".to_string(),
        })
    }

    #[test]
    fn tokens_split_q2_command_lines() {
        assert_eq!(
            tokenize("cmd configstrings 1 0"),
            vec!["cmd", "configstrings", "1", "0"]
        );
        assert!(tokenize("").is_empty());
    }

    #[test]
    fn signon_integers_reject_garbage_with_donor_text() {
        assert_eq!(signon_integer(Some("12")).expect("int"), 12);
        assert_eq!(
            signon_integer(Some("x")).expect_err("bad").to_string(),
            "Invalid Q2 signon number"
        );
        assert_eq!(
            signon_integer(None).expect_err("missing").to_string(),
            "Invalid Q2 signon number"
        );
    }

    #[test]
    fn server_data_resets_and_requests_command_reset() {
        let mut receiver =
            Q2ClientReceiver::new(ReceiverHost::new(), NetworkSource { downloads: None }).expect("receiver");
        let outcome = receiver
            .receive_records(
                vec![record(Q2ServerEvent::ServerData {
                    data: Box::new(server_data(4)),
                })],
                100,
                false,
            )
            .expect("records");
        assert_eq!(outcome.records.len(), 1);
        assert!(outcome.actions.reset_commands);
        assert_eq!(receiver.phase(), ApplicationNetworkPhase::Loading);
        assert_eq!(receiver.acknowledged_frame(), -1);
    }

    #[test]
    fn precache_prepares_game_state_and_begins() {
        let mut receiver = Q2ClientReceiver::new(
            ReceiverHost::new(),
            NetworkSource {
                downloads: Some(FakeDownloads {
                    prepared: 0,
                    outcome: Q2DownloadOutcome::Waiting,
                }),
            },
        )
        .expect("receiver");
        receiver
            .receive_records(
                vec![record(Q2ServerEvent::ServerData {
                    data: Box::new(server_data(4)),
                })],
                100,
                false,
            )
            .expect("data");
        let outcome = receiver
            .receive_records(
                vec![record(Q2ServerEvent::CommandText {
                    text: "precache 4".to_string(),
                })],
                100,
                false,
            )
            .expect("precache");
        assert_eq!(outcome.actions.commands, vec!["begin 4".to_string()]);
        assert_eq!(receiver.phase(), ApplicationNetworkPhase::Active);
        assert_eq!(receiver.host().game_states, 1);
    }

    #[test]
    fn precache_rejects_other_generations() {
        let mut receiver = Q2ClientReceiver::new(ReceiverHost::new(), Q2DemoReceiverSource).expect("receiver");
        receiver
            .receive_records(
                vec![record(Q2ServerEvent::ServerData {
                    data: Box::new(server_data(4)),
                })],
                100,
                false,
            )
            .expect("data");
        let error = receiver
            .receive_records(
                vec![record(Q2ServerEvent::CommandText {
                    text: "precache 9".to_string(),
                })],
                100,
                false,
            )
            .expect_err("generation");
        assert_eq!(error.to_string(), "Q2 precache refers to another server generation");
    }

    #[test]
    fn changing_returns_to_loading_and_unhandled_commands_print() {
        let mut receiver = Q2ClientReceiver::new(ReceiverHost::new(), Q2DemoReceiverSource).expect("receiver");
        receiver
            .receive_records(
                vec![record(Q2ServerEvent::CommandText {
                    text: "changing;stuff".to_string(),
                })],
                100,
                false,
            )
            .expect("commands");
        assert_eq!(receiver.phase(), ApplicationNetworkPhase::Loading);
        assert!(receiver
            .host()
            .prints
            .iter()
            .any(|line| line.contains("Server command requires application binding")));
    }

    #[test]
    fn demo_disconnect_reports_partial_records() {
        let mut receiver = Q2ClientReceiver::new(ReceiverHost::new(), Q2DemoReceiverSource).expect("receiver");
        let outcome = receiver
            .receive_records(
                vec![
                    record(Q2ServerEvent::Print {
                        level: 0,
                        text: "hi\n".to_string(),
                    }),
                    record(Q2ServerEvent::Disconnect),
                ],
                100,
                false,
            )
            .expect("records");
        assert_eq!(outcome.records.len(), 2);
        assert!(receiver.disconnected_demo());
        assert_eq!(receiver.host().record_batches, vec![2]);
    }

    #[test]
    fn network_disconnect_and_reconnect_drive_host_and_commands() {
        let mut receiver =
            Q2ClientReceiver::new(ReceiverHost::new(), NetworkSource { downloads: None }).expect("receiver");
        let outcome = receiver
            .receive_records(vec![record(Q2ServerEvent::Reconnect)], 100, false)
            .expect("reconnect");
        assert_eq!(outcome.actions.commands, vec!["new".to_string()]);
        let mut receiver =
            Q2ClientReceiver::new(ReceiverHost::new(), NetworkSource { downloads: None }).expect("receiver");
        receiver
            .receive_records(vec![record(Q2ServerEvent::Disconnect)], 100, false)
            .expect("disconnect");
        assert_eq!(receiver.host().disconnected, vec!["Server disconnected".to_string()]);
    }

    #[test]
    fn complete_downloads_prepare_pending_state() {
        let mut receiver = Q2ClientReceiver::new(
            ReceiverHost::new(),
            NetworkSource {
                downloads: Some(FakeDownloads {
                    prepared: 0,
                    outcome: Q2DownloadOutcome::Complete,
                }),
            },
        )
        .expect("receiver");
        receiver
            .receive_records(
                vec![record(Q2ServerEvent::ServerData {
                    data: Box::new(server_data(4)),
                })],
                100,
                false,
            )
            .expect("data");
        receiver
            .receive_records(
                vec![record(Q2ServerEvent::CommandText {
                    text: "precache 4".to_string(),
                })],
                100,
                false,
            )
            .expect("precache");
        assert_eq!(receiver.phase(), ApplicationNetworkPhase::Active);
    }

    #[test]
    fn seed_requires_active_server() {
        let mut receiver = Q2ClientReceiver::new(ReceiverHost::new(), Q2DemoReceiverSource).expect("receiver");
        let error = receiver.seed().expect_err("seed");
        assert_eq!(error.to_string(), "Recording requires an active Q2 server");
    }

    #[test]
    fn closed_receivers_ignore_input() {
        let mut receiver = Q2ClientReceiver::new(ReceiverHost::new(), Q2DemoReceiverSource).expect("receiver");
        receiver.close();
        let outcome = receiver.receive(&[7], 100, false).expect("receive");
        assert!(outcome.records.is_empty());
    }
}
