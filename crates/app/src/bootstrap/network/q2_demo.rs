//! Quake II demo playback.
//!
//! Port of `/home/buzzkill/Projects/quake-typescript/src/app/bootstrap/network/q2-demo.ts`
//! (`readQ2PlaybackHeader`, `Q2MvdPresentation`, `Q2DemoAdvance`,
//! `Q2DemoPlayback`). Recorded server frames, not dm2 block boundaries,
//! determine playback time. dm2 framing and the recorded-preamble scan are
//! ported inline with the donor's error texts; MVD decoding reuses
//! [`MvdPlayback`](qa_net::q2_svc::MvdPlayback), and both flavors decode
//! through [`Q2ClientReceiver`](super::q2_client_receiver::Q2ClientReceiver).
//! The donor is asynchronous; this port resolves every step inline.

use qa_net::msg::{MsgError, MsgReader};
use qa_net::protocol::ProtocolIdentity;
use qa_net::q2::ServerData as VanillaServerData;
use qa_net::q2_net::{Q2NetError, Q2ServerData, Q2ServerEvent, Q2ServerRecord, Q2Wire, Q2WireFrame};
use qa_net::q2_svc::{mvd_magic, MvdPlayback, MvdVisibility, MVD_MAX_MESSAGE};
use qa_net::q2_variants::{
    read_mvd_header, MvdHeader, MvdProtocol, Q2ProServerData, RereleaseServerData, VariantError,
};
use thiserror::Error;

use super::q2_client_receiver::{Q2ClientReceiver, Q2ClientReceiverError, Q2ClientReceiverHost, Q2DemoReceiverSource};
use super::types::ApplicationNetworkPhase;
use crate::bootstrap::demo_recording::Q2ProtocolIdentity;

/// Demo playback failure.
#[derive(Debug, Error)]
pub enum Q2DemoError {
    /// Policy or protocol failure with donor text.
    #[error("{0}")]
    Message(String),
    /// Wire failure.
    #[error(transparent)]
    Net(#[from] Q2NetError),
    /// Receiver failure.
    #[error(transparent)]
    Receiver(#[from] Q2ClientReceiverError),
    /// Message failure.
    #[error(transparent)]
    Msg(#[from] MsgError),
    /// MVD profile failure.
    #[error(transparent)]
    Variant(#[from] VariantError),
}

/// Whether bytes hold an MVD stream (`isMvd`).
fn is_mvd(bytes: &[u8]) -> bool {
    bytes.len() >= 4 && bytes[0..4] == mvd_magic()
}

/// Lazy demo record cursor (`readQ2Demo` / `readMvdFile`).
enum Q2DemoCursor<'a> {
    /// dm2 framing.
    Dm2 {
        /// Source bytes.
        bytes: &'a [u8],
        /// Cursor offset.
        offset: usize,
    },
    /// MVD framing.
    Mvd {
        /// Source bytes.
        bytes: &'a [u8],
        /// Cursor offset.
        offset: usize,
    },
}

impl<'a> Q2DemoCursor<'a> {
    /// Open a cursor over demo bytes.
    fn new(bytes: &'a [u8]) -> Self {
        if is_mvd(bytes) {
            Self::Mvd { bytes, offset: 4 }
        } else {
            Self::Dm2 { bytes, offset: 0 }
        }
    }

    /// Read the next record header offset and payload.
    fn next_record(&mut self) -> Result<Option<(usize, &'a [u8])>, Q2DemoError> {
        match self {
            Self::Dm2 { bytes, offset } => {
                if *offset >= bytes.len() {
                    return Ok(None);
                }
                if *offset + 4 > bytes.len() {
                    return Err(Q2DemoError::Message("Truncated Q2 demo record header".to_string()));
                }
                let start = *offset;
                let length = i32::from_le_bytes([bytes[start], bytes[start + 1], bytes[start + 2], bytes[start + 3]]);
                *offset += 4;
                if length == -1 {
                    return Ok(None);
                }
                if length < 0 || *offset + length as usize > bytes.len() {
                    return Err(Q2DemoError::Message("Truncated Q2 demo packet".to_string()));
                }
                let end = *offset + length as usize;
                *offset = end;
                Ok(Some((start, &bytes[start + 4..end])))
            }
            Self::Mvd { bytes, offset } => {
                if *offset >= bytes.len() {
                    return Err(Q2DemoError::Message("MVD recording lacks its terminator".to_string()));
                }
                if *offset + 2 > bytes.len() {
                    return Err(Q2DemoError::Message("Truncated MVD record header".to_string()));
                }
                let start = *offset;
                let length = usize::from(u16::from_le_bytes([bytes[start], bytes[start + 1]]));
                *offset += 2;
                if length == 0 {
                    if *offset != bytes.len() {
                        return Err(Q2DemoError::Message("Data after MVD terminator".to_string()));
                    }
                    return Ok(None);
                }
                if length > MVD_MAX_MESSAGE || *offset + length > bytes.len() {
                    return Err(Q2DemoError::Message("Truncated or oversized MVD packet".to_string()));
                }
                let end = *offset + length;
                *offset = end;
                Ok(Some((start, &bytes[start + 2..end])))
            }
        }
    }
}

/// Map a recorded version to its wire protocol (`recordedProtocol`).
///
/// `tail` starts after the version long, where the donor's probe reads the
/// vanilla serverdata prefix to reach enhanced-protocol revisions.
fn recorded_protocol(version: i32, tail: &[u8]) -> Result<ProtocolIdentity, Q2DemoError> {
    match version {
        26 | 34 => Ok(ProtocolIdentity::Q2Classic),
        35 | 36 => {
            let mut probe = MsgReader::new(tail);
            probe.long()?;
            probe.byte()?;
            probe.string(2047);
            probe.short()?;
            probe.string(2047);
            if version == 35 {
                probe.byte()?;
            }
            let revision = probe.word()?;
            if probe.badread() {
                return Err(Q2DemoError::Message("Truncated Quake II message".to_string()));
            }
            if version == 35 {
                match revision {
                    1903..=1905 => Ok(ProtocolIdentity::Q2R1q2 {
                        revision: u32::from(revision),
                    }),
                    _ => Err(Q2DemoError::Message(format!(
                        "Unsupported recorded R1Q2 revision {revision}"
                    ))),
                }
            } else {
                match revision {
                    1015 | 1017 | 1018 | 1019 | 1020 | 1021 | 1022 | 1023 | 1024 | 1025 | 1026 => {
                        Ok(ProtocolIdentity::Q2Q2pro {
                            revision: u32::from(revision),
                        })
                    }
                    _ => Err(Q2DemoError::Message(format!(
                        "Unsupported recorded Q2PRO revision {revision}"
                    ))),
                }
            }
        }
        1038 => Ok(ProtocolIdentity::Q2Rerelease),
        4038 => Ok(ProtocolIdentity::Q2PrivateClassic),
        2022 => Ok(ProtocolIdentity::Q2KexDemo),
        2023 => Ok(ProtocolIdentity::Q2Kex),
        _ => Err(Q2DemoError::Message(format!(
            "Unsupported recorded Q2 protocol {version}"
        ))),
    }
}

/// Decode the recorded dm2 preamble (`readQ2DemoHeader`).
fn read_dm2_header(bytes: &[u8]) -> Result<(i32, ProtocolIdentity, Q2ServerData), Q2DemoError> {
    let mut cursor = Q2DemoCursor::new(bytes);
    while let Some((_, record)) = cursor.next_record()? {
        let mut message = MsgReader::new(record);
        while message.remaining() > 0 {
            let opcode = message.byte()?;
            match opcode {
                6 => {}
                10 => {
                    message.byte()?;
                    message.string(2047);
                }
                11 | 15 => {
                    message.string(2047);
                }
                12 => {
                    let recorded_version = message.long()?;
                    if message.badread() {
                        return Err(Q2DemoError::Message("Truncated Quake II message".to_string()));
                    }
                    let body = message.offset();
                    let protocol = recorded_protocol(recorded_version, &record[body..])?;
                    let mut wire = Q2Wire::new(protocol)?;
                    wire.begin(record);
                    wire.with_reader(|reader| {
                        reader.skip(body)?;
                        Ok(())
                    })?;
                    let data = wire.read_server_data()?;
                    if recorded_version == 34 && record.get(body + 4) == Some(&2) {
                        return Err(Q2DemoError::Message(
                            "Native serverrecord footage has no player view; use the Q2 server-demo reader, not ordinary demo playback".to_string(),
                        ));
                    }
                    wire.finish()?;
                    return Ok((recorded_version, protocol, data));
                }
                _ => {
                    return Err(Q2DemoError::Message(format!(
                        "Q2 demo preamble opcode {opcode} requires serverdata first"
                    )));
                }
            }
            if message.badread() {
                return Err(Q2DemoError::Message("Truncated Quake II message".to_string()));
            }
        }
    }
    Err(Q2DemoError::Message("Q2 demo has no serverdata".to_string()))
}

/// Map an MVD stream protocol to its wire identity.
fn mvd_wire_protocol(protocol: &MvdProtocol) -> ProtocolIdentity {
    match *protocol {
        MvdProtocol::Classic => ProtocolIdentity::Q2Classic,
        MvdProtocol::Q2Pro { revision } => ProtocolIdentity::Q2Q2pro {
            revision: u32::from(revision),
        },
        MvdProtocol::Rerelease => ProtocolIdentity::Q2Rerelease,
    }
}

/// Synthesize MVD header server data (selected client zero).
fn mvd_playback_data(header: &MvdHeader) -> Q2ServerData {
    match header.profile.protocol {
        MvdProtocol::Q2Pro { revision } => Q2ServerData::Q2Pro(Q2ProServerData {
            servercount: header.servercount,
            attractloop: true,
            gamedir: header.gamedir.clone(),
            clientnum: 0,
            levelname: header.levelname.clone(),
            version: revision,
            server_state: 2,
            wire_flags: if header.profile.v2 { 24 } else { 8 },
        }),
        MvdProtocol::Rerelease => Q2ServerData::Rerelease(RereleaseServerData {
            servercount: header.servercount,
            attractloop: true,
            gamedir: header.gamedir.clone(),
            clientnum: 0,
            levelname: header.levelname.clone(),
            protocol_revision: header.profile.revision,
            server_state: 2,
            wire_flags: 0,
            server_fps: 10,
        }),
        MvdProtocol::Classic => Q2ServerData::Vanilla(VanillaServerData {
            servercount: header.servercount,
            attractloop: true,
            gamedir: header.gamedir.clone(),
            clientnum: 0,
            levelname: header.levelname.clone(),
        }),
    }
}

/// Playback header (`Q2PlaybackHeader`).
#[derive(Debug, Clone, PartialEq)]
pub enum Q2PlaybackHeader {
    /// Client demo preamble.
    Dm2 {
        /// Recorded version.
        recorded_version: i32,
        /// Recorded protocol.
        protocol: ProtocolIdentity,
        /// Recorded server data.
        data: Q2ServerData,
    },
    /// Multiview gamestate.
    Mvd {
        /// Stream protocol.
        protocol: ProtocolIdentity,
        /// Header server data.
        data: Q2ServerData,
        /// Parsed header.
        profile: MvdHeader,
    },
}

/// Read a playback header (`readQ2PlaybackHeader`).
pub fn read_q2_playback_header(bytes: &[u8]) -> Result<Q2PlaybackHeader, Q2DemoError> {
    if !is_mvd(bytes) {
        let (recorded_version, protocol, data) = read_dm2_header(bytes)?;
        return Ok(Q2PlaybackHeader::Dm2 {
            recorded_version,
            protocol,
            data,
        });
    }
    let mut cursor = Q2DemoCursor::new(bytes);
    let Some((_, gamestate)) = cursor.next_record()? else {
        return Err(Q2DemoError::Message("MVD recording has no gamestate".to_string()));
    };
    let profile = read_mvd_header(&mut MsgReader::new(gamestate))?;
    let protocol = mvd_wire_protocol(&profile.profile.protocol);
    let data = mvd_playback_data(&profile);
    Ok(Q2PlaybackHeader::Mvd {
        protocol,
        data,
        profile,
    })
}

/// Multiview viewer binding (`Q2MvdPresentation['selectView']`).
pub trait Q2MvdView {
    /// Select the viewed client.
    fn select_view(&mut self, clientnum: u8);
}

/// Recorded viewpoint binding (`selectRecordedView`).
pub trait Q2RecordedView {
    /// Select the recorded viewpoint.
    fn select_recorded_view(&mut self, clientnum: i32);
}

impl Q2RecordedView for () {
    fn select_recorded_view(&mut self, _clientnum: i32) {}
}

impl Q2MvdView for () {
    fn select_view(&mut self, _clientnum: u8) {}
}

/// Multiview presentation (`Q2MvdPresentation`).
pub struct Q2MvdPresentation {
    /// Source BSP visibility.
    pub visibility: Box<dyn MvdVisibility>,
    /// Viewer binding.
    pub select_view: Box<dyn Q2MvdView>,
}

/// Owned visibility adapter for [`MvdPlayback`].
pub(crate) struct DelegatedVisibility(pub(crate) Box<dyn MvdVisibility>);

impl MvdVisibility for DelegatedVisibility {
    fn entities(
        &self,
        entities: &[qa_net::q2::EntityState],
        player: &qa_net::q2::PlayerState,
        portal_bits: &[u8],
    ) -> Vec<qa_net::q2::EntityState> {
        self.0.entities(entities, player, portal_bits)
    }

    fn visible(
        &self,
        leaf: u16,
        channel: qa_net::q2_svc::MvdChannel,
        player: &qa_net::q2::PlayerState,
        portal_bits: &[u8],
    ) -> bool {
        self.0.visible(leaf, channel, player, portal_bits)
    }

    fn area_bits(&self, player: &qa_net::q2::PlayerState, portal_bits: &[u8]) -> Vec<u8> {
        self.0.area_bits(player, portal_bits)
    }

    fn sound_audible(&self, origin: [f64; 3], player: &qa_net::q2::PlayerState, portal_bits: &[u8]) -> bool {
        self.0.sound_audible(origin, player, portal_bits)
    }

    fn sound_origin(&self, entity: &qa_net::q2::EntityState) -> [f64; 3] {
        self.0.sound_origin(entity)
    }
}

/// Demo advance outcome (`Q2DemoAdvance`).
#[derive(Debug, Clone, PartialEq)]
pub enum Q2DemoAdvance {
    /// A frame is ready at recorded time.
    Frame {
        /// Recorded time in milliseconds.
        time_milliseconds: f64,
    },
    /// End of recording.
    Eof,
    /// Recorded disconnect.
    Disconnected,
    /// Closed during advancement.
    Closed,
}

/// Terminal playback state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Q2DemoTerminal {
    /// End of recording.
    Eof,
    /// Recorded disconnect.
    Disconnected,
}

/// Viewpoint-selecting receiver host.
///
/// Public only to name [`Q2DemoPlayback::receiver`]; construct playback with
/// [`Q2DemoPlayback::new`].
pub struct DemoAdapter<H> {
    /// Wrapped host.
    inner: H,
    /// Whether the stream is multiview.
    mvd: bool,
    /// Recorded viewpoints.
    recorded_players: Vec<i16>,
    /// Selected viewpoint seat.
    selected_seat: usize,
    /// Recorded viewpoint binding.
    select_recorded_view: Option<Box<dyn Q2RecordedView>>,
}

impl<H: Q2ClientReceiverHost> DemoAdapter<H> {
    /// Select a dm2 viewpoint seat.
    fn select_dm2_player(&mut self, clientnum: i32) -> Result<(), Q2DemoError> {
        let Some(seat) = self
            .recorded_players
            .iter()
            .position(|player| i32::from(*player) == clientnum)
        else {
            return Err(Q2DemoError::Message(
                "Player has no viewpoint in this recording".to_string(),
            ));
        };
        self.selected_seat = seat;
        Ok(())
    }

    /// Filter seat-scoped records to the selected viewpoint (`viewRecords`).
    fn view_records(&self, records: &[Q2ServerRecord]) -> Vec<Q2ServerRecord> {
        if self.mvd {
            return records.to_vec();
        }
        records
            .iter()
            .filter(|record| match &record.event {
                Q2ServerEvent::Layout { .. }
                | Q2ServerEvent::Inventory { .. }
                | Q2ServerEvent::CenterPrint { .. }
                | Q2ServerEvent::Damage { .. }
                | Q2ServerEvent::Fog { .. }
                | Q2ServerEvent::Poi { .. }
                | Q2ServerEvent::HelpPath { .. }
                | Q2ServerEvent::Locprint { .. } => {
                    record.seat == 0 || usize::from(record.seat) == self.selected_seat + 1
                }
                _ => true,
            })
            .cloned()
            .collect()
    }
}

impl<H: Q2ClientReceiverHost> Q2ClientReceiverHost for DemoAdapter<H> {
    fn protocol(&self) -> Q2ProtocolIdentity {
        self.inner.protocol()
    }

    fn message_options(&self) -> qa_net::q2_net::Q2ServerMessageOptions {
        self.inner.message_options()
    }

    fn server_data(&mut self, data: &Q2ServerData, assert_current: &dyn Fn()) {
        self.recorded_players = match data {
            Q2ServerData::Kex(data) => data.clientnums.clone(),
            _ => vec![data.clientnum()],
        };
        if self.selected_seat >= self.recorded_players.len() {
            self.selected_seat = 0;
        }
        self.inner.server_data(data, assert_current);
    }

    fn game_state(&mut self, state: &super::types::Q2ApplicationGameState) {
        self.inner.game_state(state);
    }

    fn frame(&mut self, frame: &Q2WireFrame, records: &[Q2ServerRecord], now: u64) {
        if self.mvd {
            self.inner.frame(frame, records, now);
            return;
        }
        let Some(selected) = self.recorded_players.get(self.selected_seat) else {
            panic!("Recorded viewpoint has no source player");
        };
        let selected = *selected;
        let split = if self.selected_seat == 0 {
            None
        } else {
            match frame.split_players.get(self.selected_seat - 1) {
                Some(split) => Some(split),
                None => panic!("Recorded viewpoint has no player state"),
            }
        };
        if let Some(view) = self.select_recorded_view.as_mut() {
            view.select_recorded_view(i32::from(selected));
        }
        let filtered = self.view_records(records);
        match split {
            Some(split) => {
                let mut swapped = frame.clone();
                swapped.player = split.player.clone();
                swapped.area_bits.clone_from(&split.area_bits);
                self.inner.frame(&swapped, &filtered, now);
            }
            None => self.inner.frame(frame, &filtered, now),
        }
    }

    fn records(&mut self, records: &[Q2ServerRecord]) {
        let filtered = self.view_records(records);
        self.inner.records(&filtered);
    }

    fn disconnected(&mut self, reason: &str) {
        self.inner.disconnected(reason);
    }

    fn print(&mut self, text: &str) {
        self.inner.print(text);
    }
}

/// Quake II demo playback (`Q2DemoPlayback`).
pub struct Q2DemoPlayback<'a, H> {
    bytes: &'a [u8],
    cursor: Q2DemoCursor<'a>,
    receiver: Q2ClientReceiver<DemoAdapter<H>, Q2DemoReceiverSource>,
    mvd: Option<MvdPlayback>,
    mvd_view: Option<Box<dyn Q2MvdView>>,
    header_bytes: usize,
    terminal: Option<Q2DemoTerminal>,
    closed: bool,
    advancing: bool,
    failure: Option<String>,
    offset: usize,
}

impl<'a, H: Q2ClientReceiverHost> Q2DemoPlayback<'a, H> {
    /// Open playback over demo bytes.
    pub fn new(
        bytes: &'a [u8],
        host: H,
        presentation: Option<Q2MvdPresentation>,
        select_recorded_view: Option<Box<dyn Q2RecordedView>>,
    ) -> Result<Self, Q2DemoError> {
        let mvd_stream = is_mvd(bytes);
        let (mvd, mvd_view, header_bytes) = if mvd_stream {
            let Some(presentation) = presentation else {
                return Err(Q2DemoError::Message(
                    "MVD playback requires source BSP visibility and viewer binding".to_string(),
                ));
            };
            let decoder = MvdPlayback::new(DelegatedVisibility(presentation.visibility))?;
            (Some(decoder), Some(presentation.select_view), 2)
        } else {
            (None, None, 4)
        };
        let adapter = DemoAdapter {
            inner: host,
            mvd: mvd_stream,
            recorded_players: Vec::new(),
            selected_seat: 0,
            select_recorded_view,
        };
        Ok(Self {
            bytes,
            cursor: Q2DemoCursor::new(bytes),
            receiver: Q2ClientReceiver::new(adapter, Q2DemoReceiverSource)?,
            mvd,
            mvd_view,
            header_bytes,
            terminal: None,
            closed: false,
            advancing: false,
            failure: None,
            offset: 0,
        })
    }

    /// Borrow the receiver.
    #[must_use]
    pub fn receiver(&self) -> &Q2ClientReceiver<DemoAdapter<H>, Q2DemoReceiverSource> {
        &self.receiver
    }

    /// Select the viewed player (`selectPlayer`).
    pub fn select_player(&mut self, clientnum: i32) -> Result<(), Q2DemoError> {
        if let Some(mvd) = self.mvd.as_mut() {
            let number = u8::try_from(clientnum)
                .map_err(|_| Q2DemoError::Message("Player has no viewpoint in this recording".to_string()))?;
            mvd.select_player(number)?;
            return Ok(());
        }
        self.receiver.host_mut().select_dm2_player(clientnum)
    }

    /// Recorded time in milliseconds.
    #[must_use]
    pub fn recorded_time_milliseconds(&self) -> Option<f64> {
        self.receiver.recorded_time_milliseconds()
    }

    /// Consumed bytes.
    #[must_use]
    pub fn consumed_bytes(&self) -> usize {
        self.offset
    }

    /// Advance to a presentation time (`advance`).
    pub fn advance(&mut self, target_milliseconds: f64) -> Result<Q2DemoAdvance, Q2DemoError> {
        if !target_milliseconds.is_finite() || target_milliseconds < 0.0 {
            return Err(Q2DemoError::Message("Invalid Q2 demo presentation time".to_string()));
        }
        self.read_until(Some(target_milliseconds))
    }

    /// Advance one frame (`nextFrame`).
    pub fn next_frame(&mut self) -> Result<Q2DemoAdvance, Q2DemoError> {
        self.read_until(None)
    }

    /// Read until the target time or the next frame (`readUntil`).
    fn read_until(&mut self, target: Option<f64>) -> Result<Q2DemoAdvance, Q2DemoError> {
        if self.closed {
            return Ok(Q2DemoAdvance::Closed);
        }
        if let Some(failure) = self.failure.clone() {
            return Err(Q2DemoError::Message(failure));
        }
        if let Some(terminal) = self.terminal {
            return Ok(match terminal {
                Q2DemoTerminal::Eof => Q2DemoAdvance::Eof,
                Q2DemoTerminal::Disconnected => Q2DemoAdvance::Disconnected,
            });
        }
        if self.advancing {
            return Err(Q2DemoError::Message("Q2 demo advance already in progress".to_string()));
        }
        self.advancing = true;
        let outcome = self.read_until_inner(target);
        self.advancing = false;
        match outcome {
            Ok(advance) => Ok(advance),
            Err(error) => {
                self.failure = Some(error.to_string());
                Err(error)
            }
        }
    }

    /// Read loop body with failure capture.
    fn read_until_inner(&mut self, mut target: Option<f64>) -> Result<Q2DemoAdvance, Q2DemoError> {
        let mut delivered = false;
        loop {
            let time = self.recorded_time_milliseconds();
            if let Some(time) = time {
                let ready = match target {
                    None => delivered,
                    Some(target) => time > target,
                };
                if ready {
                    return Ok(Q2DemoAdvance::Frame {
                        time_milliseconds: time,
                    });
                }
            }
            let next = self.cursor.next_record()?;
            let Some((start, payload)) = next else {
                self.terminal = Some(Q2DemoTerminal::Eof);
                // A completed reader before physical EOF consumed the -1 header.
                if self.offset < self.bytes.len() {
                    self.offset += self.header_bytes;
                }
                // Let the final decoded frame be presented before the owner handles EOF.
                if let Some(time) = time {
                    if delivered {
                        return Ok(Q2DemoAdvance::Frame {
                            time_milliseconds: time,
                        });
                    }
                }
                return Ok(Q2DemoAdvance::Eof);
            };
            self.offset = start + self.header_bytes + payload.len();
            let generation = self.receiver.world_generation();
            if let Some(mvd) = self.mvd.as_mut() {
                let records = mvd.read(payload)?;
                for record in records {
                    if matches!(record.event, Q2ServerEvent::Frame { .. }) {
                        let selected = mvd.selected_player();
                        if let Some(view) = self.mvd_view.as_mut() {
                            view.select_view(selected);
                        }
                    }
                    self.receiver.receive_records(vec![record], 0, false)?;
                    if self.closed {
                        return Ok(Q2DemoAdvance::Closed);
                    }
                }
            } else {
                self.receiver.receive(payload, 0, false)?;
            }
            if time.is_some() && self.receiver.world_generation() != generation {
                target = None;
            }
            if self.closed {
                return Ok(Q2DemoAdvance::Closed);
            }
            let current = self.recorded_time_milliseconds();
            if current != time && current.is_some() {
                delivered = true;
            }
            if self.receiver.disconnected_demo() {
                self.terminal = Some(Q2DemoTerminal::Disconnected);
                if let Some(final_time) = self.recorded_time_milliseconds() {
                    if delivered {
                        return Ok(Q2DemoAdvance::Frame {
                            time_milliseconds: final_time,
                        });
                    }
                }
                return Ok(Q2DemoAdvance::Disconnected);
            }
            if self.receiver.phase() == ApplicationNetworkPhase::Closed {
                return Ok(Q2DemoAdvance::Closed);
            }
        }
    }

    /// Close playback (`close`).
    pub fn close(&mut self) {
        if self.closed {
            return;
        }
        self.closed = true;
        self.receiver.close();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_net::protocol::ProtocolIdentity;
    use qa_net::q2::{EntityState, PlayerState, PmoveState, MAX_STATS_STORAGE};
    use qa_net::q2_net::{encode_q2_frame, Q2ServerMessageOptions, Q2WireFrame};
    use qa_net::q2_svc::{frame_mvd_message, MvdCapture, MvdChannel, MvdEncoder};
    use std::collections::BTreeMap;

    use crate::bootstrap::demo_recording::Q2ProtocolIdentity as AppIdentity;

    struct DemoHost {
        frames: Vec<i32>,
    }

    impl Q2ClientReceiverHost for DemoHost {
        fn protocol(&self) -> AppIdentity {
            AppIdentity::Classic
        }

        fn message_options(&self) -> Q2ServerMessageOptions {
            Q2ServerMessageOptions::default()
        }

        fn server_data(&mut self, _data: &Q2ServerData, _assert_current: &dyn Fn()) {}

        fn game_state(&mut self, _state: &super::super::types::Q2ApplicationGameState) {}

        fn frame(&mut self, frame: &Q2WireFrame, _records: &[Q2ServerRecord], _now: u64) {
            self.frames.push(frame.server_frame);
        }

        fn records(&mut self, _records: &[Q2ServerRecord]) {}

        fn disconnected(&mut self, _reason: &str) {}

        fn print(&mut self, _text: &str) {}
    }

    fn player_state() -> PlayerState {
        PlayerState {
            clientnum: 0,
            pmove: PmoveState {
                pm_type: 0,
                origin: [0, 0, 0],
                velocity: [0, 0, 0],
                pm_flags: 0,
                pm_time: 0,
                gravity: 0,
                delta_angles: [0, 0, 0],
                viewheight: 22,
                origin_f: [0.0, 0.0, 0.0],
                velocity_f: [0.0, 0.0, 0.0],
                delta_angles_f: [0.0, 0.0, 0.0],
                delta_angle_float: false,
            },
            viewangles: [0.0, 0.0, 0.0],
            viewoffset: [0.0, 0.0, 22.0],
            kick_angles: [0.0, 0.0, 0.0],
            gunangles: [0.0, 0.0, 0.0],
            gunoffset: [0.0, 0.0, 0.0],
            gunindex: 0,
            gunskin: 0,
            gunframe: 0,
            gunrate: 0,
            blend: [0.0, 0.0, 0.0, 0.0],
            damage_blend: [0.0, 0.0, 0.0, 0.0],
            fov: 90,
            rdflags: 0,
            stats: [0; MAX_STATS_STORAGE],
            team_id: 0,
            fog: Default::default(),
        }
    }

    fn serverdata_message() -> Vec<u8> {
        let mut message = vec![12u8];
        message.extend_from_slice(&34i32.to_le_bytes());
        message.extend_from_slice(&7i32.to_le_bytes());
        message.push(0);
        message.extend_from_slice(b"baseq2\0");
        message.extend_from_slice(&0i16.to_le_bytes());
        message.extend_from_slice(b"base1\0");
        message
    }

    fn frame_message(server_frame: i32) -> Vec<u8> {
        let wire = Q2Wire::new(ProtocolIdentity::Q2Classic).expect("wire");
        let frame = Q2WireFrame {
            valid: true,
            server_frame,
            delta_frame: 0,
            suppressed_count: 0,
            area_bits: Vec::new(),
            player: player_state(),
            split_players: Vec::new(),
            entities: Vec::new(),
        };
        encode_q2_frame(&wire, &frame, None, &std::collections::HashMap::new(), 0).expect("frame")
    }

    fn dm2(records: &[Vec<u8>]) -> Vec<u8> {
        let mut bytes = Vec::new();
        for record in records {
            bytes.extend_from_slice(&(record.len() as i32).to_le_bytes());
            bytes.extend_from_slice(record);
        }
        bytes.extend_from_slice(&(-1i32).to_le_bytes());
        bytes
    }

    struct PassthroughVisibility;

    impl MvdVisibility for PassthroughVisibility {
        fn entities(&self, entities: &[EntityState], _player: &PlayerState, _portal_bits: &[u8]) -> Vec<EntityState> {
            entities.to_vec()
        }

        fn visible(&self, _leaf: u16, _channel: MvdChannel, _player: &PlayerState, _portal_bits: &[u8]) -> bool {
            true
        }

        fn area_bits(&self, _player: &PlayerState, _portal_bits: &[u8]) -> Vec<u8> {
            Vec::new()
        }

        fn sound_audible(&self, _origin: [f64; 3], _player: &PlayerState, _portal_bits: &[u8]) -> bool {
            true
        }

        fn sound_origin(&self, entity: &EntityState) -> [f64; 3] {
            entity.origin
        }
    }

    #[test]
    fn playback_header_decodes_dm2_preamble() {
        let bytes = dm2(&[serverdata_message()]);
        let header = read_q2_playback_header(&bytes).expect("header");
        match header {
            Q2PlaybackHeader::Dm2 {
                recorded_version,
                protocol,
                data,
            } => {
                assert_eq!(recorded_version, 34);
                assert_eq!(protocol, ProtocolIdentity::Q2Classic);
                assert_eq!(data.servercount(), 7);
            }
            Q2PlaybackHeader::Mvd { .. } => panic!("expected dm2"),
        }
    }

    #[test]
    fn playback_header_rejects_preamble_without_serverdata() {
        let bytes = dm2(&[vec![6u8]]);
        let error = read_q2_playback_header(&bytes).expect_err("no serverdata");
        assert_eq!(error.to_string(), "Q2 demo has no serverdata");
    }

    #[test]
    fn playback_header_rejects_serverrecord_footage() {
        let mut message = serverdata_message();
        // Attract-loop byte 2 marks serverrecord footage.
        message[1 + 4 + 4] = 2;
        let bytes = dm2(&[message]);
        let error = read_q2_playback_header(&bytes).expect_err("serverrecord");
        assert!(error
            .to_string()
            .contains("Native serverrecord footage has no player view"));
    }

    #[test]
    fn dm2_advance_presents_frames_by_recorded_time() {
        let bytes = dm2(&[serverdata_message(), frame_message(3), frame_message(5)]);
        let mut playback = Q2DemoPlayback::new(&bytes, DemoHost { frames: Vec::new() }, None, None).expect("playback");
        assert_eq!(
            playback.advance(0.0).expect("frame"),
            Q2DemoAdvance::Frame {
                time_milliseconds: 300.0
            }
        );
        assert_eq!(
            playback.advance(300.0).expect("frame"),
            Q2DemoAdvance::Frame {
                time_milliseconds: 500.0
            }
        );
        assert_eq!(playback.next_frame().expect("eof"), Q2DemoAdvance::Eof);
        assert_eq!(playback.next_frame().expect("sticky"), Q2DemoAdvance::Eof);
        assert_eq!(playback.receiver.host().inner.frames, vec![3, 5]);
    }

    #[test]
    fn dm2_advance_rejects_bad_clocks() {
        let bytes = dm2(&[serverdata_message()]);
        let mut playback = Q2DemoPlayback::new(&bytes, DemoHost { frames: Vec::new() }, None, None).expect("playback");
        let error = playback.advance(-1.0).expect_err("clock");
        assert_eq!(error.to_string(), "Invalid Q2 demo presentation time");
    }

    #[test]
    fn viewpoint_selection_requires_a_recorded_seat() {
        let bytes = dm2(&[serverdata_message()]);
        let mut playback = Q2DemoPlayback::new(&bytes, DemoHost { frames: Vec::new() }, None, None).expect("playback");
        playback.advance(1000.0).expect("advance");
        playback.select_player(0).expect("seat");
        let error = playback.select_player(9).expect_err("seat");
        assert_eq!(error.to_string(), "Player has no viewpoint in this recording");
    }

    #[test]
    fn mvd_requires_presentation_binding() {
        let mut bytes = mvd_magic().to_vec();
        bytes.extend_from_slice(&[0, 0]);
        let error = Q2DemoPlayback::new(&bytes, DemoHost { frames: Vec::new() }, None, None)
            .err()
            .expect("presentation");
        assert_eq!(
            error.to_string(),
            "MVD playback requires source BSP visibility and viewer binding"
        );
    }

    #[test]
    fn mvd_header_and_playback_roundtrip_encoder_output() {
        let mut encoder = MvdEncoder::new();
        let mut players = BTreeMap::new();
        players.insert(0u8, player_state());
        let capture = MvdCapture {
            revision: 2010,
            flags: 0,
            servercount: 2,
            gamedir: "baseq2".to_string(),
            dummy: 0,
            config_strings: BTreeMap::from([(0u16, "base1".to_string()), (30u16, "8".to_string())]),
            portal_bits: Vec::new(),
            players,
            entities: Vec::new(),
            messages: Vec::new(),
        };
        let mut bytes = mvd_magic().to_vec();
        for message in encoder.capture(&capture).expect("capture") {
            bytes.extend_from_slice(&frame_mvd_message(&message).expect("frame"));
        }
        bytes.extend_from_slice(&[0, 0]);
        let header = read_q2_playback_header(&bytes).expect("header");
        assert!(matches!(header, Q2PlaybackHeader::Mvd { .. }));
        let presentation = Q2MvdPresentation {
            visibility: Box::new(PassthroughVisibility),
            select_view: Box::new(()),
        };
        let mut playback =
            Q2DemoPlayback::new(&bytes, DemoHost { frames: Vec::new() }, Some(presentation), None).expect("playback");
        let advance = playback.next_frame().expect("advance");
        assert!(matches!(advance, Q2DemoAdvance::Frame { .. } | Q2DemoAdvance::Eof));
    }

    #[test]
    fn mvd_framing_errors_keep_donor_text() {
        let truncated = mvd_magic().to_vec();
        let error = read_q2_playback_header(&truncated).expect_err("truncated");
        assert_eq!(error.to_string(), "MVD recording lacks its terminator");
        let mut trailing = mvd_magic().to_vec();
        trailing.extend_from_slice(&[0, 0, 9]);
        let error = read_q2_playback_header(&trailing).expect_err("trailing");
        assert_eq!(error.to_string(), "Data after MVD terminator");
    }

    #[test]
    fn closed_playback_reports_closed() {
        let bytes = dm2(&[serverdata_message()]);
        let mut playback = Q2DemoPlayback::new(&bytes, DemoHost { frames: Vec::new() }, None, None).expect("playback");
        playback.close();
        assert_eq!(playback.next_frame().expect("closed"), Q2DemoAdvance::Closed);
    }
}
