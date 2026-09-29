//! Demo recording framing for NetQuake, QuakeWorld, Quake II, and Quake III.
//!
//! Donor provenance: `NetQuakeDemoReader`, `writeNetQuakeDemoRecord`,
//! `QuakeWorldDemoReader`, `writeQuakeWorldDemoRecord`, and
//! `QuakeWorldDemoPlayback` in `src/network/q1/demos.ts`; `readQ2Demo`,
//! `writeQ2DemoRecord`, and `finishQ2Demo` in `src/network/q2/demo.ts`;
//! `DemoReader`, `encodeDemo`, `encodeDemoMessage`, and `finishDemo` in
//! `src/network/q3/demo.ts`.
//!
//! Q2 demo preamble decoding (`readQ2DemoHeader`) needs the full
//! server-message session and is future work; the record framing here
//! round-trips complete demo files.

use thiserror::Error;

use crate::msg::{MsgError, MsgReader, MsgWriter};
use crate::protocol::qw as qw_protocol;
use crate::q3::MAX_MESSAGE_LENGTH;

/// Error for malformed demo streams.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum DemoError {
    /// The NetQuake track line is missing or malformed.
    #[error("invalid demo track line")]
    BadTrack,
    /// A demo message size is negative or over the limit.
    #[error("invalid demo message size {0}")]
    BadSize(i32),
    /// Unknown QuakeWorld demo record type.
    #[error("unknown QWD record {0}")]
    BadRecord(u8),
    /// A Q2 demo record header is truncated.
    #[error("truncated Q2 demo record header")]
    TruncatedHeader,
    /// A Q2 demo packet is truncated.
    #[error("truncated Q2 demo packet")]
    TruncatedPacket,
    /// A Q3 demo message length is invalid.
    #[error("invalid demo message length {0}")]
    BadLength(i32),
    /// A demo sequence or payload is out of range.
    #[error("invalid demo message")]
    BadMessage,
    /// Underlying message failure.
    #[error("{0}")]
    Msg(#[from] MsgError),
}

// ---------------------------------------------------------------------------
// NetQuake (.dem)
// ---------------------------------------------------------------------------

/// NetQuake demo record (`NetQuakeDemoRecord`).
#[derive(Debug, Clone, PartialEq)]
pub struct NqDemoRecord {
    /// View angles.
    pub view_angles: [f32; 3],
    /// Message bytes.
    pub message: Vec<u8>,
}

/// NetQuake demo reader (`NetQuakeDemoReader`).
#[derive(Debug)]
pub struct NqDemoReader<'a> {
    reader: MsgReader<'a>,
    /// Forced CD track from the header line.
    pub forced_track: i32,
    max_message_bytes: usize,
}

impl<'a> NqDemoReader<'a> {
    /// Open a demo, parsing the track line (`{track}\n`, ≤ 32 bytes).
    pub fn new(bytes: &'a [u8], max_message_bytes: usize) -> Result<Self, DemoError> {
        let end = bytes
            .iter()
            .position(|byte| *byte == b'\n')
            .ok_or(DemoError::BadTrack)?;
        if end > 31 {
            return Err(DemoError::BadTrack);
        }
        let mut track = String::new();
        for byte in &bytes[..end] {
            track.push(char::from(*byte));
        }
        if track.is_empty()
            || !track
                .bytes()
                .enumerate()
                .all(|(index, byte)| byte.is_ascii_digit() || (index == 0 && byte == b'-'))
            || track == "-"
        {
            return Err(DemoError::BadTrack);
        }
        let forced_track: i32 = track.parse().map_err(|_| DemoError::BadTrack)?;
        Ok(Self {
            reader: MsgReader::new(&bytes[end + 1..]),
            forced_track,
            max_message_bytes,
        })
    }

    /// Read the next record, or `None` at the end.
    pub fn next_record(&mut self) -> Result<Option<NqDemoRecord>, DemoError> {
        if self.reader.remaining() == 0 {
            return Ok(None);
        }
        let size = self.reader.long()?;
        if size < 0 || size as usize > self.max_message_bytes {
            return Err(DemoError::BadSize(size));
        }
        let view_angles = [self.reader.float()?, self.reader.float()?, self.reader.float()?];
        let message = self.reader.bytes(size as usize)?.to_vec();
        self.reader.finish()?;
        Ok(Some(NqDemoRecord { view_angles, message }))
    }
}

/// Write a NetQuake demo header line (`writeNetQuakeDemoHeader`).
#[must_use]
pub fn write_nq_demo_header(track: i32) -> Vec<u8> {
    format!("{track}\n").into_bytes()
}

/// Write a NetQuake demo record (`writeNetQuakeDemoRecord`).
pub fn write_nq_demo_record(record: &NqDemoRecord) -> Result<Vec<u8>, DemoError> {
    let mut writer = MsgWriter::new(record.message.len() + 16, false);
    writer.write_long(record.message.len() as i32)?;
    for angle in record.view_angles {
        writer.write_float(angle)?;
    }
    writer.write_bytes(&record.message)?;
    Ok(writer.bytes().to_vec())
}

// ---------------------------------------------------------------------------
// QuakeWorld (.qwd)
// ---------------------------------------------------------------------------

/// QuakeWorld demo user command (24-byte raw layout).
#[derive(Debug, Clone, PartialEq)]
pub struct QwDemoUserCommand {
    /// Milliseconds.
    pub milliseconds: u8,
    /// View angles.
    pub angles: [f32; 3],
    /// Forward move.
    pub forward_move: i16,
    /// Side move.
    pub side_move: i16,
    /// Up move.
    pub up_move: i16,
    /// Buttons.
    pub buttons: u8,
    /// Impulse.
    pub impulse: u8,
}

/// QuakeWorld demo record (`QuakeWorldDemoRecord`).
#[derive(Debug, Clone, PartialEq)]
pub enum QwDemoRecord {
    /// Input command with view angles.
    Command {
        /// Timestamp seconds.
        seconds: f32,
        /// Command.
        command: QwDemoUserCommand,
        /// View angles.
        view_angles: [f32; 3],
    },
    /// Network packet.
    Packet {
        /// Timestamp seconds.
        seconds: f32,
        /// Message bytes.
        message: Vec<u8>,
    },
    /// Sequence numbers.
    Sequences {
        /// Timestamp seconds.
        seconds: f32,
        /// Outgoing sequence.
        outgoing: i32,
        /// Incoming sequence.
        incoming: i32,
    },
}

impl QwDemoRecord {
    /// Record timestamp.
    #[must_use]
    pub fn seconds(&self) -> f32 {
        match *self {
            QwDemoRecord::Command { seconds, .. }
            | QwDemoRecord::Packet { seconds, .. }
            | QwDemoRecord::Sequences { seconds, .. } => seconds,
        }
    }
}

/// QuakeWorld demo reader (`QuakeWorldDemoReader`).
#[derive(Debug)]
pub struct QwDemoReader<'a> {
    reader: MsgReader<'a>,
    max_message_bytes: usize,
}

impl<'a> QwDemoReader<'a> {
    /// Open a demo stream.
    #[must_use]
    pub fn new(bytes: &'a [u8], max_message_bytes: usize) -> Self {
        Self {
            reader: MsgReader::new(bytes),
            max_message_bytes,
        }
    }

    /// Read the next record, or `None` at the end.
    pub fn next_record(&mut self) -> Result<Option<QwDemoRecord>, DemoError> {
        if self.reader.remaining() == 0 {
            return Ok(None);
        }
        let seconds = self.reader.float()?;
        let record_type = self.reader.byte()?;
        let record = match record_type {
            0 => {
                let raw = self.reader.bytes(24)?;
                let command = QwDemoUserCommand {
                    milliseconds: raw[0],
                    angles: [
                        f32::from_le_bytes([raw[4], raw[5], raw[6], raw[7]]),
                        f32::from_le_bytes([raw[8], raw[9], raw[10], raw[11]]),
                        f32::from_le_bytes([raw[12], raw[13], raw[14], raw[15]]),
                    ],
                    forward_move: i16::from_le_bytes([raw[16], raw[17]]),
                    side_move: i16::from_le_bytes([raw[18], raw[19]]),
                    up_move: i16::from_le_bytes([raw[20], raw[21]]),
                    buttons: raw[22],
                    impulse: raw[23],
                };
                let view_angles = [self.reader.float()?, self.reader.float()?, self.reader.float()?];
                QwDemoRecord::Command {
                    seconds,
                    command,
                    view_angles,
                }
            }
            1 => {
                let size = self.reader.long()?;
                if size < 0 || size as usize > self.max_message_bytes {
                    return Err(DemoError::BadSize(size));
                }
                let message = self.reader.bytes(size as usize)?.to_vec();
                QwDemoRecord::Packet { seconds, message }
            }
            2 => {
                let outgoing = self.reader.long()?;
                let incoming = self.reader.long()?;
                QwDemoRecord::Sequences {
                    seconds,
                    outgoing,
                    incoming,
                }
            }
            other => return Err(DemoError::BadRecord(other)),
        };
        self.reader.finish()?;
        Ok(Some(record))
    }
}

/// Write a QuakeWorld demo record (`writeQuakeWorldDemoRecord`).
pub fn write_qw_demo_record(record: &QwDemoRecord) -> Result<Vec<u8>, DemoError> {
    let capacity = match record {
        QwDemoRecord::Packet { message, .. } => message.len() + 9,
        _ => 41,
    };
    let mut writer = MsgWriter::new(capacity, false);
    writer.write_float(record.seconds())?;
    match record {
        QwDemoRecord::Command {
            command, view_angles, ..
        } => {
            writer.write_byte(0)?;
            let mut raw = [0u8; 24];
            raw[0] = command.milliseconds;
            raw[4..8].copy_from_slice(&command.angles[0].to_le_bytes());
            raw[8..12].copy_from_slice(&command.angles[1].to_le_bytes());
            raw[12..16].copy_from_slice(&command.angles[2].to_le_bytes());
            raw[16..18].copy_from_slice(&command.forward_move.to_le_bytes());
            raw[18..20].copy_from_slice(&command.side_move.to_le_bytes());
            raw[20..22].copy_from_slice(&command.up_move.to_le_bytes());
            raw[22] = command.buttons;
            raw[23] = command.impulse;
            writer.write_bytes(&raw)?;
            for angle in view_angles {
                writer.write_float(*angle)?;
            }
        }
        QwDemoRecord::Packet { message, .. } => {
            writer.write_byte(1)?;
            writer.write_long(message.len() as i32)?;
            writer.write_bytes(message)?;
        }
        QwDemoRecord::Sequences { outgoing, incoming, .. } => {
            writer.write_byte(2)?;
            writer.write_long(*outgoing)?;
            writer.write_long(*incoming)?;
        }
    }
    Ok(writer.bytes().to_vec())
}

/// QuakeWorld demo playback clock (`QuakeWorldDemoPlayback`).
#[derive(Debug)]
pub struct QwDemoPlayback<'a> {
    reader: QwDemoReader<'a>,
    pending: Option<QwDemoRecord>,
}

impl<'a> QwDemoPlayback<'a> {
    /// Open playback over a demo stream.
    pub fn new(bytes: &'a [u8]) -> Result<Self, DemoError> {
        let mut reader = QwDemoReader::new(bytes, qw_protocol::MAX_MSGLEN);
        let pending = reader.next_record()?;
        Ok(Self { reader, pending })
    }

    /// Drain records at or before `seconds` (`force` drains everything).
    pub fn drain(&mut self, seconds: f32, force: bool) -> Result<Vec<QwDemoRecord>, DemoError> {
        let mut records = Vec::new();
        while let Some(pending) = self.pending.take() {
            if !force && pending.seconds() > seconds {
                self.pending = Some(pending);
                break;
            }
            records.push(pending);
            self.pending = self.reader.next_record()?;
        }
        Ok(records)
    }

    /// Whether the stream is exhausted.
    #[must_use]
    pub fn ended(&self) -> bool {
        self.pending.is_none()
    }
}

// ---------------------------------------------------------------------------
// Quake II (.dm2)
// ---------------------------------------------------------------------------

/// Quake II demo record (`Q2DemoRecord`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Q2DemoRecord {
    /// File offset of the record header.
    pub offset: usize,
    /// Packet bytes.
    pub bytes: Vec<u8>,
}

/// Read Quake II demo framing (`readQ2Demo`).
///
/// A `-1` length terminates the stream; trailing bytes after it are ignored.
pub fn read_q2_demo(bytes: &[u8]) -> Result<Vec<Q2DemoRecord>, DemoError> {
    let mut records = Vec::new();
    let mut offset = 0;
    while offset < bytes.len() {
        if offset + 4 > bytes.len() {
            return Err(DemoError::TruncatedHeader);
        }
        let start = offset;
        let length = i32::from_le_bytes([bytes[offset], bytes[offset + 1], bytes[offset + 2], bytes[offset + 3]]);
        offset += 4;
        if length == -1 {
            return Ok(records);
        }
        if length < 0 || offset + length as usize > bytes.len() {
            return Err(DemoError::TruncatedPacket);
        }
        records.push(Q2DemoRecord {
            offset: start,
            bytes: bytes[offset..offset + length as usize].to_vec(),
        });
        offset += length as usize;
    }
    Ok(records)
}

/// Write one Quake II demo record (`writeQ2DemoRecord`).
#[must_use]
pub fn write_q2_demo_record(bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(bytes.len() + 4);
    out.extend_from_slice(&(bytes.len() as i32).to_le_bytes());
    out.extend_from_slice(bytes);
    out
}

/// Write the Quake II demo terminator (`finishQ2Demo`).
#[must_use]
pub fn finish_q2_demo() -> Vec<u8> {
    vec![255, 255, 255, 255]
}

// ---------------------------------------------------------------------------
// Quake III (.dm3)
// ---------------------------------------------------------------------------

/// Quake III demo message (`DemoMessage`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Q3DemoMessage {
    /// Sequence number.
    pub sequence: i32,
    /// Payload without the netchannel header.
    pub payload: Vec<u8>,
}

/// Quake III demo end reason (`DemoEnd`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Q3DemoEndReason {
    /// `-1, -1` terminator.
    Terminator,
    /// Clean end of file.
    Eof,
    /// Truncated sequence/length header.
    TruncatedHeader,
    /// Truncated payload.
    TruncatedPayload,
}

/// Quake III demo end marker (`DemoEnd`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Q3DemoEnd {
    /// End reason.
    pub reason: Q3DemoEndReason,
    /// Offset of the incomplete record.
    pub offset: usize,
}

/// Quake III demo read result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Q3DemoRead {
    /// A complete message.
    Message(Q3DemoMessage),
    /// End of stream.
    End(Q3DemoEnd),
}

/// Quake III demo reader (`DemoReader`).
#[derive(Debug)]
pub struct Q3DemoReader<'a> {
    bytes: &'a [u8],
    position: usize,
    ended: Option<Q3DemoEnd>,
}

impl<'a> Q3DemoReader<'a> {
    /// Open a demo stream.
    #[must_use]
    pub fn new(bytes: &'a [u8]) -> Self {
        Self {
            bytes,
            position: 0,
            ended: None,
        }
    }

    /// Current offset.
    #[must_use]
    pub fn offset(&self) -> usize {
        self.position
    }

    fn end(&mut self, reason: Q3DemoEndReason, offset: usize) -> Q3DemoRead {
        let end = Q3DemoEnd { reason, offset };
        self.ended = Some(end);
        Q3DemoRead::End(end)
    }

    /// Read the next message, publishing the sequence word through
    /// `on_sequence` before reading the length (`next`).
    pub fn next(&mut self, mut on_sequence: impl FnMut(i32)) -> Result<Q3DemoRead, DemoError> {
        if let Some(end) = self.ended {
            return Ok(Q3DemoRead::End(end));
        }
        let start = self.position;
        let remaining = self.bytes.len() - start;
        if remaining == 0 {
            return Ok(self.end(Q3DemoEndReason::Eof, start));
        }
        if remaining < 4 {
            self.position = self.bytes.len();
            return Ok(self.end(Q3DemoEndReason::TruncatedHeader, start));
        }
        let sequence = i32::from_le_bytes(self.bytes[start..start + 4].try_into().unwrap());
        self.position += 4;
        on_sequence(sequence);
        if remaining < 8 {
            self.position = self.bytes.len();
            return Ok(self.end(Q3DemoEndReason::TruncatedHeader, start));
        }
        let length = i32::from_le_bytes(self.bytes[start + 4..start + 8].try_into().unwrap());
        self.position += 4;
        if length == -1 {
            return Ok(self.end(Q3DemoEndReason::Terminator, start));
        }
        if length < 0 || length as usize > MAX_MESSAGE_LENGTH {
            return Err(DemoError::BadLength(length));
        }
        if length as usize > self.bytes.len() - self.position {
            self.position = self.bytes.len();
            return Ok(self.end(Q3DemoEndReason::TruncatedPayload, start));
        }
        let payload = self.bytes[self.position..self.position + length as usize].to_vec();
        self.position += length as usize;
        Ok(Q3DemoRead::Message(Q3DemoMessage { sequence, payload }))
    }
}

/// Encode demo messages with a terminator (`encodeDemo`).
pub fn encode_q3_demo(messages: &[Q3DemoMessage]) -> Result<Vec<u8>, DemoError> {
    let mut length: usize = 8;
    for message in messages {
        if message.payload.len() > MAX_MESSAGE_LENGTH {
            return Err(DemoError::BadMessage);
        }
        length = length
            .checked_add(8 + message.payload.len())
            .ok_or(DemoError::BadMessage)?;
    }
    let mut output = vec![0u8; length];
    let mut offset = 0;
    for message in messages {
        output[offset..offset + 4].copy_from_slice(&message.sequence.to_le_bytes());
        output[offset + 4..offset + 8].copy_from_slice(&(message.payload.len() as i32).to_le_bytes());
        output[offset + 8..offset + 8 + message.payload.len()].copy_from_slice(&message.payload);
        offset += 8 + message.payload.len();
    }
    output[offset..offset + 4].copy_from_slice(&(-1i32).to_le_bytes());
    output[offset + 4..offset + 8].copy_from_slice(&(-1i32).to_le_bytes());
    Ok(output)
}

/// Encode a single demo message without the terminator (`encodeDemoMessage`).
pub fn encode_q3_demo_message(message: &Q3DemoMessage) -> Result<Vec<u8>, DemoError> {
    let mut framed = encode_q3_demo(std::slice::from_ref(message))?;
    framed.truncate(framed.len() - 8);
    Ok(framed)
}

/// Write the Quake III demo terminator (`finishDemo`).
#[must_use]
pub fn finish_q3_demo() -> Vec<u8> {
    vec![255; 8]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn netquake_demo_round_trip() {
        let mut stream = write_nq_demo_header(-1);
        let record = NqDemoRecord {
            view_angles: [1.0, 2.0, 3.0],
            message: vec![11, 22, 33],
        };
        stream.extend(write_nq_demo_record(&record).unwrap());

        let mut reader = NqDemoReader::new(&stream, 64000).unwrap();
        assert_eq!(reader.forced_track, -1);
        let decoded = reader.next_record().unwrap().unwrap();
        assert_eq!(decoded, record);
        assert!(reader.next_record().unwrap().is_none());

        assert!(NqDemoReader::new(b"no-newline", 64000).is_err());
        assert!(NqDemoReader::new(b"abc\n", 64000).is_err());
        assert!(NqDemoReader::new(&[b'1'; 33], 64000).is_err());
    }

    #[test]
    fn quakeworld_demo_round_trip() {
        let records = [
            QwDemoRecord::Command {
                seconds: 1.5,
                command: QwDemoUserCommand {
                    milliseconds: 33,
                    angles: [10.0, 20.0, 30.0],
                    forward_move: 400,
                    side_move: -200,
                    up_move: 50,
                    buttons: 3,
                    impulse: 7,
                },
                view_angles: [0.0, 90.0, 0.0],
            },
            QwDemoRecord::Packet {
                seconds: 2.5,
                message: vec![9, 8, 7],
            },
            QwDemoRecord::Sequences {
                seconds: 3.5,
                outgoing: 100,
                incoming: 98,
            },
        ];
        let mut stream = Vec::new();
        for record in &records {
            stream.extend(write_qw_demo_record(record).unwrap());
        }
        let mut reader = QwDemoReader::new(&stream, qw_protocol::MAX_MSGLEN);
        for expected in &records {
            assert_eq!(&reader.next_record().unwrap().unwrap(), expected);
        }
        assert!(reader.next_record().unwrap().is_none());

        let mut playback = QwDemoPlayback::new(&stream).unwrap();
        assert!(!playback.ended());
        assert_eq!(playback.drain(2.0, false).unwrap().len(), 1);
        assert_eq!(playback.drain(99.0, false).unwrap().len(), 2);
        assert!(playback.ended());

        let mut bad = write_qw_demo_record(&records[1]).unwrap();
        bad[4] = 9;
        assert!(QwDemoReader::new(&bad, qw_protocol::MAX_MSGLEN).next_record().is_err());
    }

    #[test]
    fn quake2_demo_framing_round_trip() {
        let mut stream = write_q2_demo_record(&[1, 2, 3]);
        stream.extend(write_q2_demo_record(&[4, 5]));
        stream.extend(finish_q2_demo());
        let records = read_q2_demo(&stream).unwrap();
        assert_eq!(records.len(), 2);
        assert_eq!(records[0].offset, 0);
        assert_eq!(records[0].bytes, vec![1, 2, 3]);
        assert_eq!(records[1].offset, 7);
        assert_eq!(records[1].bytes, vec![4, 5]);

        assert!(read_q2_demo(&[1, 2, 3]).is_err());
        assert!(read_q2_demo(&[8, 0, 0, 0, 1]).is_err());
        assert!(read_q2_demo(&[]).unwrap().is_empty());
    }

    #[test]
    fn quake3_demo_round_trip() {
        let messages = [
            Q3DemoMessage {
                sequence: 100,
                payload: vec![1, 2, 3],
            },
            Q3DemoMessage {
                sequence: 101,
                payload: vec![],
            },
        ];
        let stream = encode_q3_demo(&messages).unwrap();
        assert_eq!(encode_q3_demo_message(&messages[0]).unwrap().len(), 11);

        let mut reader = Q3DemoReader::new(&stream);
        let mut sequences = Vec::new();
        let mut decoded = Vec::new();
        loop {
            match reader.next(|sequence| sequences.push(sequence)).unwrap() {
                Q3DemoRead::Message(message) => decoded.push(message),
                Q3DemoRead::End(end) => {
                    assert_eq!(end.reason, Q3DemoEndReason::Terminator);
                    break;
                }
            }
        }
        assert_eq!(decoded, messages);
        assert_eq!(sequences, vec![100, 101, -1]);

        let mut reader = Q3DemoReader::new(&stream[..stream.len() - 4]);
        loop {
            match reader.next(|_| {}).unwrap() {
                Q3DemoRead::Message(_) => {}
                Q3DemoRead::End(end) => {
                    assert_eq!(end.reason, Q3DemoEndReason::TruncatedHeader);
                    break;
                }
            }
        }
        let mut reader = Q3DemoReader::new(&[1, 2, 3]);
        assert!(matches!(
            reader.next(|_| {}).unwrap(),
            Q3DemoRead::End(Q3DemoEnd {
                reason: Q3DemoEndReason::TruncatedHeader,
                ..
            })
        ));
        let mut reader = Q3DemoReader::new(&[0, 0, 0, 0, 4, 0, 0, 0, 1]);
        assert!(matches!(
            reader.next(|_| {}).unwrap(),
            Q3DemoRead::End(Q3DemoEnd {
                reason: Q3DemoEndReason::TruncatedPayload,
                ..
            })
        ));
        assert_eq!(finish_q3_demo(), vec![255; 8]);
    }
}
