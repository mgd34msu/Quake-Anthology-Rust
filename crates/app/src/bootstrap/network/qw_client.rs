//! QuakeWorld client network (donor `/home/buzzkill/Projects/quake-typescript/src/app/bootstrap/network/qw-client.ts`).
//!
//! The donor is `async`; this sync port resolves every host call inline.
//! Recording appends on the submit path keep the donor's deferred-failure
//! semantics: the first error is stored and rethrown at the next poll,
//! clearing only when a new sink attaches.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use qa_content::catalog::{remote_content_selection, CatalogError, RemoteContentBase};
use qa_net::common::commands::ActorCommand;
use qa_net::common::endpoint::{same_address, NetworkAddress};
use qa_net::common::session::WireSelection;
use qa_net::common::transport::{DatagramTransport, ReceiveEvent, TransportError};
use qa_net::demo::{QwDemoRecord, QwDemoUserCommand};
use qa_net::msg::{MsgError, MsgWriter};
use qa_net::protocol::ProtocolIdentity;
use qa_net::q1_net::{
    quake_world_command_arguments, quake_world_info, write_client_string_command, write_quake_world_move, Q1NetError,
    QuakeWorldChannel, QuakeWorldConnectClient, QuakeWorldConnectState, QuakeWorldDecoder, QuakeWorldMessage,
    QuakeWorldMove, QuakeWorldRecordingState, QuakeWorldSide, QwText, QwUnit,
};
use qa_net::q1_wide::QwProfile;
use qa_net::qw::QwUsercmd;
use qa_world::session::SimulationOutput;
use thiserror::Error;

use super::qw_types::{
    QwApplicationClientHost, QwDownloadCategory, QwDownloadReceive, QwDownloadRequest, QwServerData,
};
use super::types::{
    ApplicationNetwork, ApplicationNetworkError, ApplicationNetworkPhase, ApplicationNetworkRecording,
    ApplicationNetworkRole, NetworkPresentationEvent,
};
use crate::bootstrap::demo_recording::{
    DemoRecordingError, DemoRecordingIdentity, DemoRecordingPacket, DemoRecordingSeed, DemoRecordingSink,
};

/// Default connection timeout in milliseconds (donor `120000`).
const DEFAULT_TIMEOUT_MILLISECONDS: f64 = 120_000.0;

/// Maximum player slot (donor `32`).
const MAX_PLAYER_SLOT: u8 = 32;

/// QuakeWorld client network failure.
#[derive(Debug, Error)]
pub enum QwClientError {
    /// Donor failure with donor text.
    #[error("{0}")]
    Message(String),
    /// Wire failure.
    #[error(transparent)]
    Net(#[from] Q1NetError),
    /// Message coding failure.
    #[error(transparent)]
    Msg(#[from] MsgError),
    /// Transport failure.
    #[error(transparent)]
    Transport(#[from] TransportError),
    /// Recording failure.
    #[error(transparent)]
    Record(#[from] DemoRecordingError),
    /// Content failure.
    #[error(transparent)]
    Catalog(#[from] CatalogError),
}

impl QwClientError {
    /// Map into the application network error.
    fn into_network(self) -> ApplicationNetworkError {
        ApplicationNetworkError::Message(self.to_string())
    }
}

/// Attached recording sink with its detach identity.
struct QwRecordingOwner {
    /// Recording sink.
    sink: Box<dyn DemoRecordingSink>,
    /// Detach identity.
    id: u64,
}

/// Pending download queue (donor `QwClientNetwork['downloads']`).
struct QwDownloadQueue {
    /// Paths to fetch.
    paths: Vec<String>,
    /// Download category.
    category: QwDownloadCategory,
    /// Next path index.
    index: usize,
}

/// QuakeWorld client network options (donor `QwClientNetworkOptions`).
pub struct QwClientNetworkOptions<T, H> {
    /// Shared datagram transport.
    pub transport: T,
    /// Remote server address.
    pub remote: NetworkAddress,
    /// Application host.
    pub host: H,
    /// Client qport.
    pub qport: u16,
    /// Current userinfo.
    pub userinfo: Box<dyn Fn() -> String>,
    /// Connection timeout in milliseconds.
    pub timeout_milliseconds: Option<u64>,
}

/// QuakeWorld client network (`QwClientNetwork`).
pub struct QwClientNetwork<T: DatagramTransport<Address = NetworkAddress>, H> {
    transport: T,
    remote: NetworkAddress,
    host: H,
    handshake: QuakeWorldConnectClient,
    userinfo: String,
    userinfo_source: Box<dyn Fn() -> String>,
    qport: u16,
    timeout_milliseconds: Option<u64>,
    skin_pass_pending: bool,
    begun: bool,
    channel: QuakeWorldChannel,
    decoder: QuakeWorldDecoder,
    state: ApplicationNetworkPhase,
    connected: bool,
    last_received: Option<f64>,
    last_sent: f64,
    last_now: f64,
    data: Option<QwServerData>,
    models: Vec<String>,
    sounds: Vec<String>,
    downloads: Option<QwDownloadQueue>,
    last_delta: Option<u32>,
    oldest: QwUsercmd,
    previous: QwUsercmd,
    commands: HashMap<i64, QwUsercmd>,
    recording_state: QuakeWorldRecordingState,
    recording_sink: Rc<RefCell<Option<QwRecordingOwner>>>,
    recording_waiting: bool,
    recording_failure: Option<String>,
    detach_counter: u64,
}

impl<T, H> QwClientNetwork<T, H>
where
    T: DatagramTransport<Address = NetworkAddress>,
    H: QwApplicationClientHost,
{
    /// Build a client network.
    pub fn new(options: QwClientNetworkOptions<T, H>) -> Result<Self, QwClientError> {
        Ok(Self {
            transport: options.transport,
            remote: options.remote,
            host: options.host,
            handshake: QuakeWorldConnectClient::new(options.qport, ""),
            userinfo: String::new(),
            userinfo_source: options.userinfo,
            qport: options.qport,
            timeout_milliseconds: options.timeout_milliseconds,
            skin_pass_pending: false,
            begun: false,
            channel: QuakeWorldChannel::new(QuakeWorldSide::Client, u32::from(options.qport), 1450, 2500.0)?,
            decoder: QuakeWorldDecoder::new(QwProfile::Quakeworld),
            state: ApplicationNetworkPhase::Connecting,
            connected: false,
            last_received: None,
            last_sent: f64::NEG_INFINITY,
            last_now: 0.0,
            data: None,
            models: Vec::new(),
            sounds: Vec::new(),
            downloads: None,
            last_delta: None,
            oldest: QwUsercmd::default(),
            previous: QwUsercmd::default(),
            commands: HashMap::new(),
            recording_state: QuakeWorldRecordingState::default(),
            recording_sink: Rc::new(RefCell::new(None)),
            recording_waiting: false,
            recording_failure: None,
            detach_counter: 0,
        })
    }

    /// Queue a reliable string command (donor `command`).
    pub fn command(&mut self, text: &str) -> Result<(), QwClientError> {
        if !self.connected {
            return Err(QwClientError::Message("QuakeWorld client is not connected".to_string()));
        }
        let mut bytes = MsgWriter::new(1450, false);
        write_client_string_command(&mut bytes, text)?;
        self.channel.queue_reliable(bytes.bytes())?;
        Ok(())
    }

    /// Re-run the skin download pass (donor `refreshSkins`).
    pub fn refresh_skins(&mut self) -> Result<(), QwClientError> {
        self.skin_pass_pending = true;
        if self.downloads.is_some() || self.data.is_none() {
            return Ok(());
        }
        self.skin_pass_pending = false;
        if let Some(skins) = self.host.skins() {
            skins.loading(true);
        }
        let paths = self.host.skins().map_or_else(Vec::new, |skins| skins.names());
        self.downloads = Some(QwDownloadQueue {
            paths,
            category: QwDownloadCategory::Skin,
            index: 0,
        });
        self.resume_downloads()
    }

    /// Poll the transport and handshake (donor `poll`).
    fn poll_inner(&mut self, now_milliseconds: u64) -> Result<Vec<ActorCommand>, QwClientError> {
        let now = now_milliseconds as f64;
        self.last_now = now;
        if let Some(failure) = &self.recording_failure {
            return Err(QwClientError::Message(failure.clone()));
        }
        if matches!(
            self.state,
            ApplicationNetworkPhase::Closed | ApplicationNetworkPhase::Rejected
        ) {
            return Ok(Vec::new());
        }
        self.last_received.get_or_insert(now);
        self.sync_userinfo()?;
        if !self.connected {
            if let Some(bytes) = self.handshake.next(now) {
                self.transport.send(&self.remote, &bytes)?;
            }
        }
        loop {
            let event = self.transport.poll()?;
            let Some(event) = event else {
                break;
            };
            let ReceiveEvent::Packet { from, payload, .. } = event else {
                continue;
            };
            if !same_address(&from, &self.remote, true) {
                continue;
            }
            let oob = payload.len() >= 4 && i32::from_le_bytes([payload[0], payload[1], payload[2], payload[3]]) == -1;
            if !self.connected {
                if !oob {
                    continue;
                }
                self.handshake.receive(&payload)?;
                match &self.handshake.state {
                    QuakeWorldConnectState::Rejected { reason } => {
                        let reason = reason.clone();
                        self.state = ApplicationNetworkPhase::Rejected;
                        self.host.disconnected(&reason);
                        return Ok(Vec::new());
                    }
                    QuakeWorldConnectState::Connected => {
                        self.connected = true;
                        self.state = ApplicationNetworkPhase::Loading;
                        self.last_received = Some(now);
                        self.transmit(&[], now)?;
                        self.command("new")?;
                    }
                    _ => {}
                }
                continue;
            }
            if oob {
                continue;
            }
            let delivery = self.channel.receive(&payload, now)?;
            let Some(delivery) = delivery else {
                continue;
            };
            self.last_received = Some(now);
            if let Some(prediction) = self.host.prediction() {
                prediction.acknowledged(delivery.acknowledged, now);
            }
            let messages = self.decoder.decode(&delivery.payload, delivery.sequence)?;
            self.recording_state.observe(&messages);
            let closed = self.records(&messages, now)?;
            if messages.iter().any(|message| {
                matches!(
                    message,
                    QuakeWorldMessage::PacketEntities {
                        delta_sequence: None,
                        ..
                    }
                )
            }) {
                self.recording_waiting = false;
            }
            if !self.recording_waiting {
                if let Some(owner) = self.recording_sink.borrow_mut().as_mut() {
                    owner.sink.append(&DemoRecordingPacket::Qw {
                        record: QwDemoRecord::Packet {
                            seconds: (now / 1000.0) as f32,
                            message: payload.clone(),
                        },
                    })?;
                }
            }
            if closed {
                return Ok(Vec::new());
            }
        }
        let last_received = self.last_received.unwrap_or(now);
        if now - last_received
            > self
                .timeout_milliseconds
                .map_or(DEFAULT_TIMEOUT_MILLISECONDS, |timeout| timeout as f64)
        {
            self.state = ApplicationNetworkPhase::Rejected;
            self.host.disconnected("Connection timed out");
            return Ok(Vec::new());
        }
        if self.connected
            && self.channel.can_packet(now)
            && (self.state != ApplicationNetworkPhase::Active || now - self.last_sent >= 1000.0)
        {
            self.transmit(&[], now)?;
        }
        Ok(Vec::new())
    }

    /// Transmit one packet (donor `transmit`).
    fn transmit(&mut self, bytes: &[u8], now: f64) -> Result<(), QwClientError> {
        self.last_sent = now;
        let packet = self.channel.transmit(bytes, now, false)?;
        self.transport.send(&self.remote, &packet)?;
        Ok(())
    }

    /// Sync userinfo into the handshake or a live connection (donor
    /// `syncUserinfo`).
    fn sync_userinfo(&mut self) -> Result<(), QwClientError> {
        let text = (self.userinfo_source)();
        if text.bytes().any(|byte| byte == b'"' || byte == b'\n' || byte == b'\r') {
            return Err(QwClientError::Message("Invalid QW userinfo".to_string()));
        }
        if text == self.userinfo {
            return Ok(());
        }
        if self.connected {
            let previous = quake_world_info(&self.userinfo);
            let current = quake_world_info(&text);
            let mut keys: Vec<&String> = previous.keys().chain(current.keys()).collect();
            keys.sort();
            keys.dedup();
            for key in keys {
                let value = current.get(key).map_or("", String::as_str);
                if previous.get(key).map_or("", String::as_str) != value {
                    self.command(&format!("setinfo \"{key}\" \"{value}\""))?;
                }
            }
        } else {
            let state = self.handshake.state.clone();
            self.handshake = QuakeWorldConnectClient::new(self.qport, &text);
            self.handshake.state = state;
        }
        self.userinfo = text;
        Ok(())
    }

    /// Drain the download queue (donor `resumeDownloads`).
    fn resume_downloads(&mut self) -> Result<(), QwClientError> {
        let (category, server_count) = {
            let (Some(queue), Some(data)) = (self.downloads.as_ref(), self.data.as_ref()) else {
                return Ok(());
            };
            (queue.category, data.server_count)
        };
        loop {
            let next = {
                let Some(queue) = self.downloads.as_mut() else {
                    return Ok(());
                };
                if queue.index >= queue.paths.len() {
                    None
                } else {
                    let path = queue.paths[queue.index].clone();
                    queue.index += 1;
                    Some(path)
                }
            };
            let Some(path) = next else {
                break;
            };
            if path.starts_with('*') {
                continue;
            }
            let waiting = self
                .host
                .downloads()
                .is_some_and(|downloads| downloads.request(&path, category) == QwDownloadRequest::Waiting);
            if waiting {
                return Ok(());
            }
        }
        self.downloads = None;
        match category {
            QwDownloadCategory::Sound => {
                self.command(&format!("modellist {server_count} 0"))?;
            }
            QwDownloadCategory::Model => {
                let data = self.data.clone().expect("data checked above");
                let checksum = self.host.game_state(&data, &self.models.clone(), &self.sounds.clone());
                self.command(&format!("prespawn {server_count} 0 {checksum}"))?;
            }
            QwDownloadCategory::Skin => {
                if self.skin_pass_pending {
                    return self.refresh_skins();
                }
                if let Some(skins) = self.host.skins() {
                    skins.loading(false);
                }
                if let Some(skins) = self.host.skins() {
                    skins.prepare();
                }
                if !self.begun && self.state != ApplicationNetworkPhase::Active {
                    self.command(&format!("begin {server_count}"))?;
                    self.begun = true;
                }
            }
        }
        Ok(())
    }

    /// Fold server messages into client state (donor `records`); reports a
    /// closing disconnect.
    fn records(&mut self, messages: &[QuakeWorldMessage], now: f64) -> Result<bool, QwClientError> {
        for message in messages {
            match message {
                QuakeWorldMessage::ServerData {
                    protocol,
                    server_count,
                    game_directory,
                    player_slot,
                    spectator,
                    level,
                    move_variables,
                } => {
                    if *protocol != QwProfile::Quakeworld || *player_slot >= MAX_PLAYER_SLOT {
                        return Err(QwClientError::Message(
                            "Remote QW requires native protocol 28 and a valid player slot".to_string(),
                        ));
                    }
                    remote_content_selection(RemoteContentBase::Q1Quakeworld, game_directory)?;
                    if let Some(downloads) = self.host.downloads() {
                        downloads.close();
                    }
                    let data = QwServerData {
                        protocol: *protocol,
                        server_count: *server_count,
                        game_directory: game_directory.clone(),
                        player_slot: *player_slot,
                        spectator: *spectator,
                        level: level.clone(),
                        move_variables: move_variables.clone(),
                    };
                    self.host.server_data(&data);
                    self.data = Some(data);
                    self.models.clear();
                    self.sounds.clear();
                    self.skin_pass_pending = false;
                    self.begun = false;
                    self.last_delta = None;
                    self.commands.clear();
                    self.oldest = QwUsercmd::default();
                    self.previous = QwUsercmd::default();
                    self.state = ApplicationNetworkPhase::Loading;
                    self.downloads = None;
                    let server_count = self.data.as_ref().expect("data just set").server_count;
                    self.command(&format!("soundlist {server_count} 0"))?;
                }
                QuakeWorldMessage::SoundList { first, names, next }
                | QuakeWorldMessage::ModelList { first, names, next } => {
                    let sounds = matches!(message, QuakeWorldMessage::SoundList { .. });
                    let Some(data) = self.data.clone() else {
                        return Err(QwClientError::Message("QW list before serverdata".to_string()));
                    };
                    let list = if sounds { &mut self.sounds } else { &mut self.models };
                    if usize::from(*first) != list.len() {
                        return Err(QwClientError::Message("Non-contiguous QW precache list".to_string()));
                    }
                    list.extend(names.iter().cloned());
                    if *next != 0 {
                        let list = if sounds { "soundlist" } else { "modellist" };
                        self.command(&format!("{list} {} {next}", data.server_count))?;
                    } else {
                        let paths = if sounds {
                            list.iter().map(|name| format!("sound/{name}")).collect()
                        } else {
                            list.clone()
                        };
                        let category = if sounds {
                            QwDownloadCategory::Sound
                        } else {
                            QwDownloadCategory::Model
                        };
                        self.downloads = Some(QwDownloadQueue {
                            paths,
                            category,
                            index: 0,
                        });
                        self.resume_downloads()?;
                    }
                }
                QuakeWorldMessage::Download { result } => {
                    let verdict = match self.host.downloads() {
                        Some(downloads) => downloads.receive(result),
                        None => {
                            return Err(QwClientError::Message("Unsolicited QW download".to_string()));
                        }
                    };
                    if verdict != QwDownloadReceive::Waiting {
                        self.resume_downloads()?;
                    }
                }
                QuakeWorldMessage::Text {
                    kind: QwText::Stufftext,
                    text,
                } => {
                    for line in text.split('\n') {
                        self.stufftext_line(line)?;
                    }
                }
                QuakeWorldMessage::PacketEntities { sequence, .. } => {
                    self.last_delta = Some(*sequence);
                    self.state = ApplicationNetworkPhase::Active;
                }
                QuakeWorldMessage::InvalidDelta { .. } => {
                    self.last_delta = None;
                }
                QuakeWorldMessage::Unit(QwUnit::Disconnect) => {
                    self.state = ApplicationNetworkPhase::Closed;
                    self.host.disconnected("Server disconnected");
                }
                _ => {}
            }
        }
        self.host.receive(messages, now);
        if self.skin_pass_pending && self.downloads.is_none() {
            self.refresh_skins()?;
        }
        Ok(self.state == ApplicationNetworkPhase::Closed)
    }

    /// Run one stufftext line (donor `stufftext` branch).
    fn stufftext_line(&mut self, line: &str) -> Result<(), QwClientError> {
        let args = quake_world_command_arguments(line);
        let Some(name) = args.first() else {
            return Ok(());
        };
        if name == "cmd"
            && matches!(args.get(1).map(String::as_str), Some("prespawn" | "spawn"))
            && args[2..]
                .iter()
                .all(|value| !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit()))
            && args.get(2).is_some_and(|value| {
                // Donor `Number(...)`: compare in float64 exactly.
                value
                    .parse::<f64>()
                    .is_ok_and(|count| Some(count) == self.data.as_ref().map(|data| data.server_count as f64))
            })
        {
            self.command(&args[1..].join(" "))?;
        } else if name == "skins" {
            self.skin_pass_pending = true;
        } else if name == "reconnect" {
            self.state = ApplicationNetworkPhase::Loading;
            self.command("new")?;
        } else if name != "fullserverinfo" {
            self.host.print(&format!("Unhandled QW server command: {line}\n"));
        }
        Ok(())
    }

    /// Submit player commands (donor `submit`).
    fn submit_inner(&mut self, commands: &[ActorCommand], now_milliseconds: u64) -> Result<(), QwClientError> {
        if self.state != ApplicationNetworkPhase::Active {
            return Ok(());
        }
        if commands.len() > 1 {
            return Err(QwClientError::Message("A QW connection carries one player".to_string()));
        }
        let now = now_milliseconds as f64;
        self.last_now = now;
        for input in commands {
            let command = self.host.command(input);
            let sequence = self.channel.outgoing_sequence();
            if let Some(teleport) = self.host.take_spectator_teleport() {
                let mut reliable = MsgWriter::new(7, false);
                reliable.write_byte(6)?;
                reliable.write_coord(f64::from(teleport.x))?;
                reliable.write_coord(f64::from(teleport.y))?;
                reliable.write_coord(f64::from(teleport.z))?;
                self.channel.queue_reliable(reliable.bytes())?;
            }
            if let Some(owner) = self.recording_sink.borrow_mut().as_mut() {
                let record = DemoRecordingPacket::Qw {
                    record: QwDemoRecord::Command {
                        seconds: (now / 1000.0) as f32,
                        command: demo_user_command(&command),
                        view_angles: [
                            command.angles[0] as f32,
                            command.angles[1] as f32,
                            command.angles[2] as f32,
                        ],
                    },
                };
                if let Err(error) = owner.sink.append(&record) {
                    if self.recording_failure.is_none() {
                        self.recording_failure = Some(error.to_string());
                    }
                }
            }
            let mut bytes = MsgWriter::new(256, false);
            self.oldest = self.commands.get(&(sequence - 2)).cloned().unwrap_or_default();
            self.previous = self.commands.get(&(sequence - 1)).cloned().unwrap_or_default();
            write_quake_world_move(
                &mut bytes,
                &QuakeWorldMove {
                    oldest: self.oldest.clone(),
                    previous: self.previous.clone(),
                    current: command.clone(),
                    loss_percent: 0,
                },
                sequence as u32,
            )?;
            let delta = (!self.recording_waiting
                && self.last_delta.is_some_and(|delta| sequence - i64::from(delta) < 63))
            .then_some(self.last_delta)
            .flatten();
            if let Some(delta) = delta {
                bytes.write_byte(5)?;
                bytes.write_byte((delta & 255) as u8)?;
            }
            self.decoder.record_delta_request(sequence as u32, delta);
            self.commands.insert(sequence, command.clone());
            if let Some(prediction) = self.host.prediction() {
                prediction.sent(sequence as u32, &command, now);
            }
            self.commands.retain(|old, _| *old > sequence - 64);
            self.transmit(bytes.bytes(), now)?;
        }
        Ok(())
    }
}

/// Convert a live user command into its demo form.
fn demo_user_command(command: &QwUsercmd) -> QwDemoUserCommand {
    QwDemoUserCommand {
        milliseconds: command.msec,
        angles: [
            command.angles[0] as f32,
            command.angles[1] as f32,
            command.angles[2] as f32,
        ],
        forward_move: command.forwardmove,
        side_move: command.sidemove,
        up_move: command.upmove,
        buttons: command.buttons,
        impulse: command.impulse,
    }
}

impl<T, H> ApplicationNetworkRecording for QwClientNetwork<T, H>
where
    T: DatagramTransport<Address = NetworkAddress>,
    H: QwApplicationClientHost,
{
    fn seed(&self) -> Result<DemoRecordingSeed, ApplicationNetworkError> {
        let failed = |message: &str| ApplicationNetworkError::Message(message.to_string());
        if self.state != ApplicationNetworkPhase::Active {
            return Err(failed("Recording requires an active QuakeWorld connection"));
        }
        let Some(data) = self.data.as_ref() else {
            return Err(failed("Recording requires an active QuakeWorld connection"));
        };
        let message = QuakeWorldMessage::ServerData {
            protocol: data.protocol,
            server_count: data.server_count,
            game_directory: data.game_directory.clone(),
            player_slot: data.player_slot,
            spectator: data.spectator,
            level: data.level.clone(),
            move_variables: data.move_variables.clone(),
        };
        let seconds = (self.last_received.unwrap_or(0.0) / 1000.0) as f32;
        let outgoing = i32::try_from(self.channel.outgoing_sequence()).unwrap_or(i32::MAX);
        let incoming = i32::try_from(self.channel.incoming_sequence()).unwrap_or(i32::MAX);
        let records = self
            .recording_state
            .seed(&message, &self.models, &self.sounds, seconds, outgoing, incoming)
            .map_err(|error| failed(&error.to_string()))?;
        Ok(DemoRecordingSeed {
            identity: DemoRecordingIdentity::Qw,
            packets: records
                .into_iter()
                .map(|record| DemoRecordingPacket::Qw { record })
                .collect(),
        })
    }

    fn attach(&mut self, sink: Box<dyn DemoRecordingSink>) -> Result<Box<dyn FnOnce() + '_>, ApplicationNetworkError> {
        if self.recording_sink.borrow().is_some()
            || matches!(
                self.state,
                ApplicationNetworkPhase::Closed | ApplicationNetworkPhase::Rejected
            )
        {
            return Err(ApplicationNetworkError::Message(
                "QuakeWorld recording cannot attach".to_string(),
            ));
        }
        self.recording_waiting = self.state == ApplicationNetworkPhase::Active;
        self.last_delta = None;
        self.recording_failure = None;
        self.detach_counter += 1;
        let id = self.detach_counter;
        *self.recording_sink.borrow_mut() = Some(QwRecordingOwner { sink, id });
        let cell = self.recording_sink.clone();
        Ok(Box::new(move || {
            let matched = cell.borrow().as_ref().is_some_and(|owner| owner.id == id);
            if matched {
                *cell.borrow_mut() = None;
            }
        }))
    }
}

impl<T, H> ApplicationNetwork for QwClientNetwork<T, H>
where
    T: DatagramTransport<Address = NetworkAddress>,
    H: QwApplicationClientHost,
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
            protocol: ProtocolIdentity::Q1Quakeworld,
        }
    }

    fn poll(&mut self, now_milliseconds: u64) -> Result<Vec<ActorCommand>, ApplicationNetworkError> {
        self.poll_inner(now_milliseconds).map_err(QwClientError::into_network)
    }

    fn submit(&mut self, commands: &[ActorCommand], now_milliseconds: u64) -> Result<(), ApplicationNetworkError> {
        self.submit_inner(commands, now_milliseconds)
            .map_err(QwClientError::into_network)
    }

    fn publish(
        &mut self,
        _output: &SimulationOutput,
        _events: &[NetworkPresentationEvent],
        _now_milliseconds: u64,
    ) -> Result<(), ApplicationNetworkError> {
        Err(ApplicationNetworkError::Message(
            "Remote client cannot publish authoritative state".to_string(),
        ))
    }

    fn close(&mut self) {
        if self.transport.closed() {
            return;
        }
        if self.connected && self.state != ApplicationNetworkPhase::Closed {
            if let Err(error) = self.command("drop").and_then(|()| {
                let now = self.last_now;
                self.transmit(&[], now)
            }) {
                self.host.print(&error.to_string());
            }
        }
        if let Some(downloads) = self.host.downloads() {
            downloads.close();
        }
        self.state = ApplicationNetworkPhase::Closed;
        self.transport.close();
    }
}

#[cfg(test)]
mod tests {
    use super::super::qw_types::QwApplicationDownloads;
    use super::*;
    use qa_core::identity::IdentityOwner;
    use qa_net::common::commands::{CommandSource, UserCommand};
    use qa_net::q1_net::{
        quake_world_out_of_band, write_quake_world_entities, write_quake_world_message, QwDownload, QwMoveVariables,
    };
    use std::sync::Mutex;

    struct MockTransportInner {
        inbound: Vec<ReceiveEvent<NetworkAddress>>,
        sent: Vec<(NetworkAddress, Vec<u8>)>,
        closed: bool,
    }

    struct MockTransport {
        address: NetworkAddress,
        inner: Mutex<MockTransportInner>,
    }

    impl MockTransport {
        fn new(address: NetworkAddress) -> Self {
            Self {
                address,
                inner: Mutex::new(MockTransportInner {
                    inbound: Vec::new(),
                    sent: Vec::new(),
                    closed: false,
                }),
            }
        }

        fn feed(&self, from: NetworkAddress, payload: Vec<u8>) {
            self.inner
                .lock()
                .expect("transport")
                .inbound
                .push(ReceiveEvent::Packet {
                    from,
                    payload,
                    received_at: 0.0,
                });
        }

        fn sent(&self) -> Vec<(NetworkAddress, Vec<u8>)> {
            self.inner.lock().expect("transport").sent.clone()
        }

        fn take_sent(&self) -> Vec<(NetworkAddress, Vec<u8>)> {
            std::mem::take(&mut self.inner.lock().expect("transport").sent)
        }
    }

    impl DatagramTransport for MockTransport {
        type Address = NetworkAddress;

        fn address(&self) -> NetworkAddress {
            self.address.clone()
        }

        fn closed(&self) -> bool {
            self.inner.lock().expect("transport").closed
        }

        fn send(&self, to: &NetworkAddress, payload: &[u8]) -> Result<bool, TransportError> {
            self.inner
                .lock()
                .expect("transport")
                .sent
                .push((to.clone(), payload.to_vec()));
            Ok(true)
        }

        fn poll(&self) -> Result<Option<ReceiveEvent<NetworkAddress>>, TransportError> {
            let mut inner = self.inner.lock().expect("transport");
            if inner.inbound.is_empty() {
                return Ok(None);
            }
            Ok(Some(inner.inbound.remove(0)))
        }

        fn subscribe_readable(&self, _listener: std::sync::Arc<dyn Fn() + Send + Sync>) -> Result<u64, TransportError> {
            Ok(0)
        }

        fn unsubscribe(&self, _token: u64) {}

        fn close(&self) {
            self.inner.lock().expect("transport").closed = true;
        }
    }

    struct SharedTransport {
        inner: std::sync::Arc<MockTransport>,
    }

    impl DatagramTransport for SharedTransport {
        type Address = NetworkAddress;

        fn address(&self) -> NetworkAddress {
            self.inner.address()
        }

        fn closed(&self) -> bool {
            self.inner.closed()
        }

        fn send(&self, to: &NetworkAddress, payload: &[u8]) -> Result<bool, TransportError> {
            self.inner.send(to, payload)
        }

        fn poll(&self) -> Result<Option<ReceiveEvent<NetworkAddress>>, TransportError> {
            self.inner.poll()
        }

        fn subscribe_readable(&self, listener: std::sync::Arc<dyn Fn() + Send + Sync>) -> Result<u64, TransportError> {
            self.inner.subscribe_readable(listener)
        }

        fn unsubscribe(&self, token: u64) {
            self.inner.unsubscribe(token);
        }

        fn close(&self) {
            self.inner.close();
        }
    }

    struct MockDownloads {
        requests: Vec<(String, QwDownloadCategory)>,
        verdicts: Vec<QwDownloadRequest>,
        receives: Vec<String>,
        receive_verdict: QwDownloadReceive,
        closed: bool,
    }

    impl Default for MockDownloads {
        fn default() -> Self {
            Self {
                requests: Vec::new(),
                verdicts: Vec::new(),
                receives: Vec::new(),
                receive_verdict: QwDownloadReceive::Complete,
                closed: false,
            }
        }
    }

    impl QwApplicationDownloads for MockDownloads {
        fn request(&mut self, path: &str, category: QwDownloadCategory) -> QwDownloadRequest {
            self.requests.push((path.to_string(), category));
            if self.verdicts.is_empty() {
                return QwDownloadRequest::Available;
            }
            self.verdicts.remove(0)
        }

        fn receive(&mut self, result: &QwDownload) -> QwDownloadReceive {
            self.receives.push(format!("{result:?}"));
            self.receive_verdict
        }

        fn close(&mut self) {
            self.closed = true;
        }
    }

    #[derive(Default)]
    struct MockSkins {
        names: Vec<String>,
        loading: Vec<bool>,
        prepares: u32,
    }

    impl super::super::qw_types::QwApplicationSkins for MockSkins {
        fn names(&self) -> Vec<String> {
            self.names.clone()
        }

        fn loading(&mut self, value: bool) {
            self.loading.push(value);
        }

        fn prepare(&mut self) {
            self.prepares += 1;
        }
    }

    #[derive(Default)]
    struct MockPrediction {
        sent: Vec<(u32, f64)>,
        acknowledged: Vec<(u32, f64)>,
    }

    impl super::super::qw_types::QwApplicationPrediction for MockPrediction {
        fn sent(&mut self, sequence: u32, _command: &QwUsercmd, now_ms: f64) {
            self.sent.push((sequence, now_ms));
        }

        fn acknowledged(&mut self, sequence: u32, now_ms: f64) {
            self.acknowledged.push((sequence, now_ms));
        }
    }

    struct MockHost {
        downloads: Option<MockDownloads>,
        skins: Option<MockSkins>,
        prediction: Option<MockPrediction>,
        server_datas: Vec<String>,
        game_states: Vec<(Vec<String>, Vec<String>)>,
        checksum: i32,
        receives: Vec<usize>,
        usercmd: QwUsercmd,
        teleport: Option<qa_core::math::Vec3>,
        disconnects: Vec<String>,
        prints: Vec<String>,
    }

    impl MockHost {
        fn new() -> Self {
            Self {
                downloads: None,
                skins: None,
                prediction: None,
                server_datas: Vec::new(),
                game_states: Vec::new(),
                checksum: 1234,
                receives: Vec::new(),
                usercmd: QwUsercmd::default(),
                teleport: None,
                disconnects: Vec::new(),
                prints: Vec::new(),
            }
        }
    }

    impl QwApplicationClientHost for MockHost {
        fn downloads(&mut self) -> Option<&mut dyn super::super::qw_types::QwApplicationDownloads> {
            self.downloads.as_mut().map(|downloads| downloads as _)
        }

        fn skins(&mut self) -> Option<&mut dyn super::super::qw_types::QwApplicationSkins> {
            self.skins.as_mut().map(|skins| skins as _)
        }

        fn prediction(&mut self) -> Option<&mut dyn super::super::qw_types::QwApplicationPrediction> {
            self.prediction.as_mut().map(|prediction| prediction as _)
        }

        fn server_data(&mut self, data: &QwServerData) {
            self.server_datas.push(data.level.clone());
        }

        fn game_state(&mut self, _data: &QwServerData, models: &[String], sounds: &[String]) -> i32 {
            self.game_states.push((models.to_vec(), sounds.to_vec()));
            self.checksum
        }

        fn receive(&mut self, messages: &[QuakeWorldMessage], _now_ms: f64) {
            self.receives.push(messages.len());
        }

        fn command(&mut self, _command: &ActorCommand) -> QwUsercmd {
            self.usercmd.clone()
        }

        fn take_spectator_teleport(&mut self) -> Option<qa_core::math::Vec3> {
            self.teleport.take()
        }

        fn disconnected(&mut self, reason: &str) {
            self.disconnects.push(reason.to_string());
        }

        fn print(&mut self, text: &str) {
            self.prints.push(text.to_string());
        }
    }

    struct MockSink {
        packets: Vec<DemoRecordingPacket>,
        fail: bool,
    }

    impl DemoRecordingSink for MockSink {
        fn append(&mut self, packet: &DemoRecordingPacket) -> Result<(), DemoRecordingError> {
            if self.fail {
                return Err(DemoRecordingError::Stopped);
            }
            self.packets.push(packet.clone());
            Ok(())
        }
    }

    fn remote() -> NetworkAddress {
        NetworkAddress::Ipv4 {
            host: [127, 0, 0, 1],
            port: 27500,
        }
    }

    fn local() -> NetworkAddress {
        NetworkAddress::Ipv4 {
            host: [127, 0, 0, 1],
            port: 27501,
        }
    }

    fn client_pair(
        host: MockHost,
        userinfo: &str,
    ) -> (
        std::sync::Arc<MockTransport>,
        QwClientNetwork<SharedTransport, MockHost>,
    ) {
        let userinfo = userinfo.to_string();
        let transport = std::sync::Arc::new(MockTransport::new(local()));
        let network = QwClientNetwork::new(QwClientNetworkOptions {
            transport: SharedTransport {
                inner: transport.clone(),
            },
            remote: remote(),
            host,
            qport: 27501,
            userinfo: Box::new(move || userinfo.clone()),
            timeout_milliseconds: None,
        })
        .expect("client");
        (transport, network)
    }

    fn server_message(messages: &[QuakeWorldMessage]) -> Vec<u8> {
        let mut writer = MsgWriter::new(1450, false);
        for message in messages {
            // Packet entities are decode-only in the message writer; encode
            // an empty full frame instead.
            if matches!(message, QuakeWorldMessage::PacketEntities { .. }) {
                write_quake_world_entities(&mut writer, QwProfile::Quakeworld, &[], &HashMap::new(), None)
                    .expect("entities");
            } else {
                write_quake_world_message(&mut writer, QwProfile::Quakeworld, message).expect("write");
            }
        }
        writer.bytes().to_vec()
    }

    fn server_data_message() -> QuakeWorldMessage {
        QuakeWorldMessage::ServerData {
            protocol: QwProfile::Quakeworld,
            server_count: 5,
            game_directory: "qw".to_string(),
            player_slot: 1,
            spectator: false,
            level: "dm1".to_string(),
            move_variables: QwMoveVariables::default(),
        }
    }

    struct ServerLink {
        channel: QuakeWorldChannel,
    }

    impl ServerLink {
        fn new() -> Self {
            let mut channel =
                QuakeWorldChannel::new(QuakeWorldSide::Server, 27501, 1450, 2500.0).expect("server channel");
            // The toggle channel drops sequence 0 on both ends; skip it so
            // every fed packet is accepted.
            channel.transmit(&[], 0.0, false).expect("throwaway");
            Self { channel }
        }

        fn packet(&mut self, messages: &[QuakeWorldMessage], now: f64) -> Vec<u8> {
            self.channel
                .transmit(&server_message(messages), now, false)
                .expect("transmit")
        }
    }

    fn connect(transport: &std::sync::Arc<MockTransport>, network: &mut QwClientNetwork<SharedTransport, MockHost>) {
        network.poll_inner(0).expect("challenge");
        assert!(transport.take_sent()[0].1.ends_with(b"getchallenge\n"));
        transport.feed(remote(), quake_world_out_of_band("c1234", false));
        network.poll_inner(100).expect("challenged");
        network.poll_inner(5100).expect("connect");
        let sent = transport.take_sent();
        assert!(sent
            .iter()
            .any(|(_, payload)| payload.windows(7).any(|w| w == b"connect")));
        transport.feed(remote(), quake_world_out_of_band("j", false));
        network.poll_inner(5200).expect("connected");
        assert!(network.connected);
    }

    fn actor_command(sequence: u32) -> ActorCommand {
        let owner = IdentityOwner::create("qw-client-test").expect("owner");
        ActorCommand {
            actor: owner.actor(0, 1),
            source: CommandSource::LocalSeat { seat: owner.seat(0) },
            sequence: u64::from(sequence),
            command: UserCommand::Q1Quakeworld {
                milliseconds: 10.0,
                angles: [0.0, 0.0, 0.0],
                forward_move: 0.0,
                side_move: 0.0,
                up_move: 0.0,
                buttons: 0.0,
                impulse: 0.0,
            },
            arsenal: None,
        }
    }

    #[test]
    fn handshake_challenges_then_connects() {
        let (transport, mut network) = client_pair(MockHost::new(), "\\name\\t");
        connect(&transport, &mut network);
        assert_eq!(network.phase(), ApplicationNetworkPhase::Loading);
        assert_eq!(network.userinfo, "\\name\\t");
        // The connect poll transmitted an empty packet and queued `new`.
        assert!(network.channel.has_reliable());
    }

    #[test]
    fn handshake_rejection_disconnects() {
        let (transport, mut network) = client_pair(MockHost::new(), "");
        network.poll_inner(0).expect("challenge");
        transport.feed(remote(), quake_world_out_of_band("nServer is full", false));
        network.poll_inner(100).expect("rejected");
        assert_eq!(network.phase(), ApplicationNetworkPhase::Rejected);
        assert_eq!(network.host.disconnects, vec!["Server is full".to_string()]);
    }

    #[test]
    fn userinfo_validation_and_live_diff() {
        let (_transport, mut network) = client_pair(MockHost::new(), "bad\"quote");
        let error = network.poll_inner(0).expect_err("invalid userinfo");
        assert_eq!(error.to_string(), "Invalid QW userinfo");

        let (transport, mut network) = client_pair(MockHost::new(), "\\name\\t");
        connect(&transport, &mut network);
        network.userinfo_source = Box::new(|| "\\name\\t2\\rate\\5000".to_string());
        network.poll_inner(6000).expect("diff");
        assert!(network.channel.has_reliable());
    }

    #[test]
    fn server_data_resets_and_runs_precache() {
        let mut host = MockHost::new();
        host.downloads = Some(MockDownloads::default());
        let (transport, mut network) = client_pair(host, "");
        connect(&transport, &mut network);
        let mut server = ServerLink::new();
        transport.feed(remote(), server.packet(&[server_data_message()], 6000.0));
        network.poll_inner(6000).expect("serverdata");
        assert_eq!(network.host.server_datas, vec!["dm1".to_string()]);
        assert_eq!(network.phase(), ApplicationNetworkPhase::Loading);
        assert!(network.channel.has_reliable());

        // Sound list fans out to downloads, then requests the model list.
        let sounds = QuakeWorldMessage::SoundList {
            first: 0,
            names: vec!["a.wav".to_string()],
            next: 0,
        };
        transport.feed(remote(), server.packet(&[sounds], 6100.0));
        network.poll_inner(6100).expect("sounds");
        assert_eq!(network.sounds, vec!["a.wav".to_string()]);
        let downloads = network.host.downloads.as_ref().expect("downloads");
        assert_eq!(
            downloads.requests,
            vec![("sound/a.wav".to_string(), QwDownloadCategory::Sound)]
        );

        // Model list completes precache and prespawns with the checksum.
        let models = QuakeWorldMessage::ModelList {
            first: 0,
            names: vec!["progs/player.mdl".to_string()],
            next: 0,
        };
        transport.feed(remote(), server.packet(&[models], 6200.0));
        network.poll_inner(6200).expect("models");
        assert_eq!(network.host.game_states.len(), 1);
        assert_eq!(network.host.game_states[0].0, vec!["progs/player.mdl".to_string()]);
    }

    #[test]
    fn precache_guards() {
        let (transport, mut network) = client_pair(MockHost::new(), "");
        connect(&transport, &mut network);
        let mut server = ServerLink::new();
        // List before serverdata.
        let sounds = QuakeWorldMessage::SoundList {
            first: 0,
            names: Vec::new(),
            next: 0,
        };
        transport.feed(remote(), server.packet(&[sounds], 6000.0));
        let error = network.poll_inner(6000).expect_err("early list");
        assert_eq!(error.to_string(), "QW list before serverdata");

        transport.feed(remote(), server.packet(&[server_data_message()], 6100.0));
        network.poll_inner(6100).expect("serverdata");
        let gap = QuakeWorldMessage::SoundList {
            first: 2,
            names: vec!["b.wav".to_string()],
            next: 0,
        };
        transport.feed(remote(), server.packet(&[gap], 6200.0));
        let error = network.poll_inner(6200).expect_err("gap");
        assert_eq!(error.to_string(), "Non-contiguous QW precache list");

        // Bad protocol and bad slot.
        let mut wide = server_data_message();
        let QuakeWorldMessage::ServerData { protocol, .. } = &mut wide else {
            panic!("server data");
        };
        *protocol = QwProfile::Wide { flags: 0 };
        transport.feed(remote(), server.packet(&[wide], 6300.0));
        let error = network.poll_inner(6300).expect_err("wide");
        assert_eq!(
            error.to_string(),
            "Remote QW requires native protocol 28 and a valid player slot"
        );
    }

    #[test]
    fn stufftext_dispatch() {
        let (transport, mut network) = client_pair(MockHost::new(), "");
        connect(&transport, &mut network);
        let mut server = ServerLink::new();
        transport.feed(remote(), server.packet(&[server_data_message()], 6000.0));
        network.poll_inner(6000).expect("serverdata");
        let stuff = QuakeWorldMessage::Text {
            kind: qa_net::q1_net::QwText::Stufftext,
            text: "cmd prespawn 5 0\nskins\nreconnect\nfullserverinfo x\nbogus arg\n".to_string(),
        };
        transport.feed(remote(), server.packet(&[stuff], 6100.0));
        network.poll_inner(6100).expect("stufftext");
        // The skins line armed a pass that records() immediately consumed.
        assert!(!network.skin_pass_pending);
        assert!(network.begun);
        assert_eq!(network.phase(), ApplicationNetworkPhase::Loading);
        assert!(network
            .host
            .prints
            .iter()
            .any(|line| line.contains("Unhandled QW server command: bogus arg")));
        assert!(!network.host.prints.iter().any(|line| line.contains("fullserverinfo")));
    }

    #[test]
    fn packet_entities_activate_and_delta_flows() {
        let (transport, mut network) = client_pair(MockHost::new(), "");
        connect(&transport, &mut network);
        let mut server = ServerLink::new();
        transport.feed(remote(), server.packet(&[server_data_message()], 6000.0));
        network.poll_inner(6000).expect("serverdata");
        let entities = QuakeWorldMessage::PacketEntities {
            sequence: 7,
            delta_sequence: None,
            entities: Vec::new(),
        };
        transport.feed(remote(), server.packet(&[entities], 6100.0));
        network.poll_inner(6100).expect("entities");
        assert_eq!(network.phase(), ApplicationNetworkPhase::Active);
        let delta = network.last_delta.expect("delta base");

        network.submit_inner(&[actor_command(1)], 6200).expect("submit");
        let sent = transport.take_sent();
        let payload = sent.last().expect("move").1.clone();
        assert_eq!(&payload[payload.len() - 2..], &[5, (delta & 255) as u8]);

        let invalid = QuakeWorldMessage::InvalidDelta {
            sequence: 8,
            requested: 0,
        };
        network.records(&[invalid], 6300.0).expect("invalid delta");
        assert_eq!(network.last_delta, None);
    }

    #[test]
    fn download_flow_and_solicitation() {
        // Without a downloads handler the chunk is unsolicited.
        let (transport, mut network) = client_pair(MockHost::new(), "");
        connect(&transport, &mut network);
        let chunk = QuakeWorldMessage::Download {
            result: QwDownload::Missing,
        };
        let error = network.records(&[chunk], 6000.0).expect_err("unsolicited");
        assert_eq!(error.to_string(), "Unsolicited QW download");

        // A waiting verdict parks the queue; completion resumes it.
        let mut host = MockHost::new();
        let downloads = MockDownloads {
            verdicts: vec![QwDownloadRequest::Waiting],
            receive_verdict: QwDownloadReceive::Complete,
            ..Default::default()
        };
        host.downloads = Some(downloads);
        let (transport, mut network) = client_pair(host, "");
        connect(&transport, &mut network);
        let mut server = ServerLink::new();
        transport.feed(remote(), server.packet(&[server_data_message()], 6000.0));
        network.poll_inner(6000).expect("serverdata");
        let sounds = QuakeWorldMessage::SoundList {
            first: 0,
            names: vec!["a.wav".to_string(), "b.wav".to_string()],
            next: 0,
        };
        transport.feed(remote(), server.packet(&[sounds], 6100.0));
        network.poll_inner(6100).expect("parked");
        assert!(network.downloads.is_some());
        let chunk = QuakeWorldMessage::Download {
            result: QwDownload::Data {
                percent: 100,
                bytes: Vec::new(),
            },
        };
        network.records(&[chunk], 6200.0).expect("resumed");
        // The second path was requested after the first completed.
        assert_eq!(network.host.downloads.as_ref().expect("dl").requests.len(), 2);
    }

    #[test]
    fn submit_gates_and_prunes() {
        let (transport, mut network) = client_pair(MockHost::new(), "");
        // Silent before activation.
        network.submit_inner(&[actor_command(1)], 100).expect("silent");
        assert!(transport.sent().is_empty());
        connect(&transport, &mut network);

        // Activate through the wire, then submit with a teleport.
        let mut server = ServerLink::new();
        transport.feed(remote(), server.packet(&[server_data_message()], 6000.0));
        network.poll_inner(6000).expect("serverdata");
        let entities = QuakeWorldMessage::PacketEntities {
            sequence: 7,
            delta_sequence: None,
            entities: Vec::new(),
        };
        transport.feed(remote(), server.packet(&[entities], 6100.0));
        network.poll_inner(6100).expect("active");
        let error = network
            .submit_inner(&[actor_command(1), actor_command(2)], 6150)
            .expect_err("two players");
        assert_eq!(error.to_string(), "A QW connection carries one player");
        network.host.teleport = Some(qa_core::math::Vec3 { x: 1.0, y: 2.0, z: 3.0 });
        network.submit_inner(&[actor_command(1)], 6200).expect("submit");
        assert!(network.channel.has_reliable());
        for sequence in 2..80u32 {
            network
                .submit_inner(&[actor_command(sequence)], 6200 + u64::from(sequence))
                .expect("submit");
        }
        assert!(network.commands.len() <= 64);
    }

    #[test]
    fn recording_seed_attach_and_deferred_failure() {
        let (transport, mut network) = client_pair(MockHost::new(), "");
        let error = ApplicationNetworkRecording::seed(&network).expect_err("seed gate");
        assert_eq!(error.to_string(), "Recording requires an active QuakeWorld connection");
        ApplicationNetworkRecording::attach(
            &mut network,
            Box::new(MockSink {
                packets: Vec::new(),
                fail: false,
            }),
        )
        .expect("pre-active attach")();
        assert!(!network.recording_waiting);
        assert!(network.recording_sink.borrow().is_none());
        connect(&transport, &mut network);
        let mut server = ServerLink::new();
        transport.feed(remote(), server.packet(&[server_data_message()], 6000.0));
        network.poll_inner(6000).expect("serverdata");
        let entities = QuakeWorldMessage::PacketEntities {
            sequence: 7,
            delta_sequence: None,
            entities: Vec::new(),
        };
        transport.feed(remote(), server.packet(&[entities], 6100.0));
        network.poll_inner(6100).expect("active");
        let seed = ApplicationNetworkRecording::seed(&network).expect("seed");
        assert_eq!(seed.identity, DemoRecordingIdentity::Qw);
        assert!(!seed.packets.is_empty());
        // Dropping the detach closure without calling it leaves the sink
        // attached; the failing sink stores its error and the next poll
        // rethrows it.
        drop(
            ApplicationNetworkRecording::attach(
                &mut network,
                Box::new(MockSink {
                    packets: Vec::new(),
                    fail: true,
                }),
            )
            .expect("attach"),
        );
        assert!(network.recording_waiting);
        network.submit_inner(&[actor_command(1)], 6200).expect("submit stores");
        let error = network.poll_inner(6300).expect_err("deferred");
        assert_eq!(error.to_string(), "Recording is stopped");
        // Simulate the dropped detach, then reattaching clears the failure.
        *network.recording_sink.borrow_mut() = None;
        drop(
            ApplicationNetworkRecording::attach(
                &mut network,
                Box::new(MockSink {
                    packets: Vec::new(),
                    fail: false,
                }),
            )
            .expect("reattach"),
        );
        assert!(network.recording_failure.is_none());
        network.poll_inner(6400).expect("cleared");
    }

    #[test]
    fn timeout_disconnect_and_close() {
        let (transport, mut network) = client_pair(MockHost::new(), "");
        connect(&transport, &mut network);
        network.poll_inner(5200 + 120_001).expect("timeout");
        assert_eq!(network.phase(), ApplicationNetworkPhase::Rejected);
        assert_eq!(network.host.disconnects, vec!["Connection timed out".to_string()]);

        let (transport, mut network) = client_pair(MockHost::new(), "");
        connect(&transport, &mut network);
        let sent_before = transport.sent().len();
        ApplicationNetwork::close(&mut network);
        assert_eq!(network.phase(), ApplicationNetworkPhase::Closed);
        assert!(transport.sent().len() > sent_before);
        assert!(transport.closed());
        let sent_after_close = transport.sent().len();
        ApplicationNetwork::close(&mut network);
        assert_eq!(transport.sent().len(), sent_after_close);

        // Server disconnects close through records.
        let (transport, mut network) = client_pair(MockHost::new(), "");
        connect(&transport, &mut network);
        let mut server = ServerLink::new();
        let bye = QuakeWorldMessage::Unit(qa_net::q1_net::QwUnit::Disconnect);
        transport.feed(remote(), server.packet(&[bye], 6000.0));
        assert!(network.poll_inner(6000).expect("bye").is_empty());
        assert_eq!(network.phase(), ApplicationNetworkPhase::Closed);
        assert_eq!(network.host.disconnects, vec!["Server disconnected".to_string()]);
    }

    #[test]
    fn refresh_skins_flow() {
        let mut host = MockHost::new();
        let skins = MockSkins {
            names: vec!["skins/a.pcx".to_string()],
            ..Default::default()
        };
        host.skins = Some(skins);
        host.downloads = Some(MockDownloads::default());
        let (transport, mut network) = client_pair(host, "");
        // No server data yet: the pass parks.
        network.refresh_skins().expect("parked");
        assert!(network.skin_pass_pending);
        connect(&transport, &mut network);
        let mut server = ServerLink::new();
        transport.feed(remote(), server.packet(&[server_data_message()], 6000.0));
        network.poll_inner(6000).expect("serverdata");
        network.refresh_skins().expect("skins");
        let skins = network.host.skins.as_ref().expect("skins");
        assert_eq!(skins.loading, vec![true, false]);
        assert_eq!(skins.prepares, 1);
        assert!(network.begun);
        let downloads = network.host.downloads.as_ref().expect("downloads");
        assert_eq!(
            downloads.requests,
            vec![("skins/a.pcx".to_string(), QwDownloadCategory::Skin)]
        );
    }

    #[test]
    fn role_wire_and_publish() {
        let (_transport, mut network) = client_pair(MockHost::new(), "");
        assert_eq!(network.role(), ApplicationNetworkRole::Client);
        assert_eq!(
            network.wire(),
            WireSelection::Source {
                protocol: ProtocolIdentity::Q1Quakeworld
            }
        );
        assert!(network.recording().is_some());
    }
}
