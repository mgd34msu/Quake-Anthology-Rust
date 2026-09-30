//! Ordered demo recording to disk.
//!
//! Donor provenance: `src/app/bootstrap/demo-recording.ts` (`DemoRecording`,
//! `recordingPath`, `DemoRecordingPacket`, `DemoRecordingIdentity`).
//!
//! Sync port: the donor chains async file writes through a promise queue; the
//! sync port writes inline, keeping the same guards ("Recording is stopped",
//! "Recording source protocol changed", "Recording write made no progress"),
//! the same "wx" exclusive create, and the same per-family footers. A stored
//! write failure still poisons later writes, and repeat `stop`/`abort` calls
//! replay the first terminal outcome by message (donor: the same promise).
//! Record framing is reused from [`qa_net::demo`] (NetQuake, QuakeWorld, Quake
//! II, Quake III) and [`qa_net::q2_svc`] (MVD magic and framing).

use std::fs::{create_dir_all, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use qa_content::paths::{normalize_resource_path, PathError};
use qa_core::math::Vec3;
use qa_net::demo::{
    encode_q3_demo_message, finish_q2_demo, finish_q3_demo, write_nq_demo_header, write_nq_demo_record,
    write_q2_demo_record, write_qw_demo_record, DemoError, NqDemoRecord, Q3DemoMessage, QwDemoRecord,
};
use qa_net::q2_svc::{frame_mvd_message, mvd_magic};
use thiserror::Error;

/// Failure of a demo-recording operation.
#[derive(Debug, Error)]
pub enum DemoRecordingError {
    /// The seed was empty or mixed families.
    #[error("Recording requires matching initial source state")]
    BadSeed,
    /// The recording is stopped.
    #[error("Recording is stopped")]
    Stopped,
    /// A packet family differs from the recording identity.
    #[error("Recording source protocol changed")]
    ProtocolChanged,
    /// A write call made no progress.
    #[error("Recording write made no progress")]
    NoProgress,
    /// A repeat stop/abort replays the first terminal failure by message.
    #[error("{0}")]
    Settled(String),
    /// A demo name was not a valid resource path.
    #[error(transparent)]
    Path(#[from] PathError),
    /// A filesystem operation failed.
    #[error(transparent)]
    Io(#[from] std::io::Error),
    /// A demo codec operation failed.
    #[error(transparent)]
    Demo(#[from] DemoError),
    /// MVD framing failed.
    #[error("mvd framing failed: {0}")]
    Mvd(String),
}

/// NetQuake demo protocol versions (donor `15 | 666 | 999`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Q1DemoProtocol {
    /// NetQuake protocol 15.
    V15,
    /// FitzQuake protocol 666.
    V666,
    /// RMQ protocol 999.
    V999,
}

impl Q1DemoProtocol {
    /// Protocol number.
    #[must_use]
    pub fn version(self) -> u16 {
        match self {
            Self::V15 => 15,
            Self::V666 => 666,
            Self::V999 => 999,
        }
    }
}

/// MVD revisions (donor `2009 | 2010 | 2011 | 2012 | 2013 | 3038`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MvdRevision {
    /// Revision 2009.
    R2009,
    /// Revision 2010.
    R2010,
    /// Revision 2011.
    R2011,
    /// Revision 2012.
    R2012,
    /// Revision 2013.
    R2013,
    /// Revision 3038.
    R3038,
}

impl MvdRevision {
    /// Revision number.
    #[must_use]
    pub fn revision(self) -> u16 {
        match self {
            Self::R2009 => 2009,
            Self::R2010 => 2010,
            Self::R2011 => 2011,
            Self::R2012 => 2012,
            Self::R2013 => 2013,
            Self::R3038 => 3038,
        }
    }
}

/// R1Q2 protocol revisions (donor `1903 | 1904 | 1905`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum R1Q2Revision {
    /// Revision 1903.
    R1903,
    /// Revision 1904.
    R1904,
    /// Revision 1905.
    R1905,
}

impl R1Q2Revision {
    /// Revision number.
    #[must_use]
    pub fn revision(self) -> u16 {
        match self {
            Self::R1903 => 1903,
            Self::R1904 => 1904,
            Self::R1905 => 1905,
        }
    }
}

/// Q2Pro protocol revisions (donor `Q2ProRevision`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Q2ProRevision {
    /// Revision 1015.
    R1015,
    /// Revision 1016.
    R1016,
    /// Revision 1017.
    R1017,
    /// Revision 1018.
    R1018,
    /// Revision 1019.
    R1019,
    /// Revision 1020.
    R1020,
    /// Revision 1021.
    R1021,
    /// Revision 1022.
    R1022,
    /// Revision 1023.
    R1023,
    /// Revision 1024.
    R1024,
    /// Revision 1025.
    R1025,
    /// Revision 1026.
    R1026,
}

impl Q2ProRevision {
    /// Revision number.
    #[must_use]
    pub fn revision(self) -> u16 {
        match self {
            Self::R1015 => 1015,
            Self::R1016 => 1016,
            Self::R1017 => 1017,
            Self::R1018 => 1018,
            Self::R1019 => 1019,
            Self::R1020 => 1020,
            Self::R1021 => 1021,
            Self::R1022 => 1022,
            Self::R1023 => 1023,
            Self::R1024 => 1024,
            Self::R1025 => 1025,
            Self::R1026 => 1026,
        }
    }
}

/// Quake II protocol identity (donor `Q2ProtocolIdentity`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Q2ProtocolIdentity {
    /// Classic protocol 34.
    Classic,
    /// R1Q2 protocol 35.
    R1Q2 {
        /// Protocol revision.
        revision: R1Q2Revision,
    },
    /// Q2Pro protocol 36.
    Q2Pro {
        /// Protocol revision.
        revision: Q2ProRevision,
    },
    /// Rerelease protocol 1038.
    Rerelease,
    /// KEX protocol 2023.
    Kex,
    /// KEX demo protocol 2022.
    KexDemo,
}

impl Q2ProtocolIdentity {
    /// Protocol version.
    #[must_use]
    pub fn version(self) -> u16 {
        match self {
            Self::Classic => 34,
            Self::R1Q2 { .. } => 35,
            Self::Q2Pro { .. } => 36,
            Self::Rerelease => 1038,
            Self::Kex => 2023,
            Self::KexDemo => 2022,
        }
    }
}

/// Recording family discriminator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecordingKind {
    /// Quake II server demo.
    Q2Server,
    /// Multi-view demo.
    Mvd,
    /// NetQuake demo.
    Q1,
    /// QuakeWorld demo.
    Qw,
    /// Quake II client demo.
    Q2,
    /// Quake III demo.
    Q3,
}

/// Recording identity (donor `DemoRecordingIdentity`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum DemoRecordingIdentity {
    /// Quake II server demo (protocol 34).
    Q2Server,
    /// Multi-view demo.
    Mvd {
        /// MVD revision.
        revision: MvdRevision,
    },
    /// NetQuake demo.
    Q1 {
        /// Demo protocol.
        protocol: Q1DemoProtocol,
        /// Forced CD track (truncated on write, as the donor does).
        track: f64,
    },
    /// QuakeWorld demo (protocol 28).
    Qw,
    /// Quake II client demo.
    Q2 {
        /// Protocol identity.
        protocol: Q2ProtocolIdentity,
    },
    /// Quake III demo (protocol 68).
    Q3,
}

impl DemoRecordingIdentity {
    /// Family discriminator.
    #[must_use]
    pub fn kind(&self) -> RecordingKind {
        match self {
            Self::Q2Server => RecordingKind::Q2Server,
            Self::Mvd { .. } => RecordingKind::Mvd,
            Self::Q1 { .. } => RecordingKind::Q1,
            Self::Qw => RecordingKind::Qw,
            Self::Q2 { .. } => RecordingKind::Q2,
            Self::Q3 => RecordingKind::Q3,
        }
    }
}

/// One recordable packet (donor `DemoRecordingPacket`).
#[derive(Debug, Clone, PartialEq)]
pub enum DemoRecordingPacket {
    /// Quake II server message.
    Q2Server {
        /// Message bytes.
        message: Vec<u8>,
    },
    /// MVD message.
    Mvd {
        /// Message bytes.
        message: Vec<u8>,
    },
    /// NetQuake message with view angles.
    Q1 {
        /// Message bytes.
        message: Vec<u8>,
        /// View angles.
        view_angles: Vec3,
    },
    /// QuakeWorld record.
    Qw {
        /// Record.
        record: QwDemoRecord,
    },
    /// Quake II client message.
    Q2 {
        /// Message bytes.
        message: Vec<u8>,
    },
    /// Quake III sequenced message.
    Q3 {
        /// Sequence number.
        sequence: i32,
        /// Message bytes.
        message: Vec<u8>,
    },
}

impl DemoRecordingPacket {
    /// Family discriminator.
    #[must_use]
    pub fn kind(&self) -> RecordingKind {
        match self {
            Self::Q2Server { .. } => RecordingKind::Q2Server,
            Self::Mvd { .. } => RecordingKind::Mvd,
            Self::Q1 { .. } => RecordingKind::Q1,
            Self::Qw { .. } => RecordingKind::Qw,
            Self::Q2 { .. } => RecordingKind::Q2,
            Self::Q3 { .. } => RecordingKind::Q3,
        }
    }
}

/// Initial source state (donor `DemoRecordingSeed`).
#[derive(Debug, Clone, PartialEq)]
pub struct DemoRecordingSeed {
    /// Recording identity.
    pub identity: DemoRecordingIdentity,
    /// Complete signon/gamestate and baselines.
    pub packets: Vec<DemoRecordingPacket>,
}

/// Append-only sink (donor `DemoRecordingSink`).
pub trait DemoRecordingSink {
    /// Append one packet.
    fn append(&mut self, packet: &DemoRecordingPacket) -> Result<(), DemoRecordingError>;
}

/// Resolve the on-disk recording path for a name and identity.
pub fn recording_path(
    root: &Path,
    name: &str,
    identity: &DemoRecordingIdentity,
) -> Result<PathBuf, DemoRecordingError> {
    let normalized = normalize_resource_path(name)?;
    let extension = match identity.kind() {
        RecordingKind::Mvd => ".mvd",
        RecordingKind::Q1 => ".dem",
        RecordingKind::Qw => ".qwd",
        RecordingKind::Q2 | RecordingKind::Q2Server => ".dm2",
        RecordingKind::Q3 => ".dm_68",
    };
    let filename = if normalized.to_lowercase().ends_with(extension) {
        normalized
    } else {
        format!("{normalized}{extension}")
    };
    let prefixed = matches!(
        identity.kind(),
        RecordingKind::Mvd | RecordingKind::Q2Server | RecordingKind::Q2 | RecordingKind::Q3
    ) && !filename.to_lowercase().starts_with("demos/");
    if prefixed {
        Ok(root.join("demos").join(filename))
    } else {
        Ok(root.join(filename))
    }
}

/// Ordered disk recording over accepted protocol messages.
#[derive(Debug)]
pub struct DemoRecording {
    path: PathBuf,
    identity: DemoRecordingIdentity,
    file: Option<File>,
    failure: Option<String>,
    finishing: bool,
    settled: Option<String>,
    accepting: bool,
    angles: Vec3,
    seconds: f32,
}

impl DemoRecording {
    /// Open an exclusive recording and write the seed.
    pub fn open(root: &Path, name: &str, seed: &DemoRecordingSeed) -> Result<Self, DemoRecordingError> {
        if seed.packets.is_empty() || seed.packets.iter().any(|packet| packet.kind() != seed.identity.kind()) {
            return Err(DemoRecordingError::BadSeed);
        }
        let path = recording_path(root, name, &seed.identity)?;
        if let Some(parent) = path.parent() {
            create_dir_all(parent)?;
        }
        let file = OpenOptions::new().write(true).create_new(true).open(&path)?;
        let mut recording = Self {
            path,
            identity: seed.identity,
            file: Some(file),
            failure: None,
            finishing: false,
            settled: None,
            accepting: true,
            angles: Vec3::default(),
            seconds: 0.0,
        };
        if recording.identity.kind() == RecordingKind::Mvd {
            recording.write(&mvd_magic())?;
        }
        if let DemoRecordingIdentity::Q1 { track, .. } = recording.identity {
            recording.write(&write_nq_demo_header(track.trunc() as i32))?;
        }
        for packet in &seed.packets {
            recording.append(packet)?;
        }
        Ok(recording)
    }

    /// Recording path.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Recording identity.
    #[must_use]
    pub fn identity(&self) -> &DemoRecordingIdentity {
        &self.identity
    }

    /// Append one packet of the recording family.
    pub fn append(&mut self, packet: &DemoRecordingPacket) -> Result<(), DemoRecordingError> {
        if !self.accepting {
            return Err(DemoRecordingError::Stopped);
        }
        if packet.kind() != self.identity.kind() {
            return Err(DemoRecordingError::ProtocolChanged);
        }
        let bytes = match packet {
            DemoRecordingPacket::Mvd { message } => {
                frame_mvd_message(message).map_err(|error| DemoRecordingError::Mvd(error.to_string()))?
            }
            DemoRecordingPacket::Q1 { message, view_angles } => {
                self.angles = *view_angles;
                write_nq_demo_record(&NqDemoRecord {
                    view_angles: [view_angles.x, view_angles.y, view_angles.z],
                    message: message.clone(),
                })?
            }
            DemoRecordingPacket::Qw { record } => {
                self.seconds = record.seconds();
                write_qw_demo_record(record)?
            }
            DemoRecordingPacket::Q2Server { message } | DemoRecordingPacket::Q2 { message } => {
                write_q2_demo_record(message)
            }
            DemoRecordingPacket::Q3 { sequence, message } => encode_q3_demo_message(&Q3DemoMessage {
                sequence: *sequence,
                payload: message.clone(),
            })?,
        };
        self.write(&bytes)
    }

    fn write(&mut self, bytes: &[u8]) -> Result<(), DemoRecordingError> {
        if let Some(failure) = self.failure.clone() {
            return Err(DemoRecordingError::Settled(failure));
        }
        let outcome: Result<(), DemoRecordingError> = (|| {
            let mut offset = 0;
            while offset < bytes.len() {
                let file = self.file.as_mut().ok_or(DemoRecordingError::Stopped)?;
                let written = file.write(&bytes[offset..])?;
                if written == 0 {
                    return Err(DemoRecordingError::NoProgress);
                }
                offset += written;
            }
            Ok(())
        })();
        if let Err(error) = &outcome {
            if self.failure.is_none() {
                self.failure = Some(error.to_string());
            }
        }
        outcome
    }

    fn settled_result(&self) -> Result<(), DemoRecordingError> {
        match &self.settled {
            Some(message) => Err(DemoRecordingError::Settled(message.clone())),
            None => Ok(()),
        }
    }

    /// Write the footer, sync, and close.
    pub fn stop(&mut self) -> Result<(), DemoRecordingError> {
        if self.finishing {
            return self.settled_result();
        }
        self.accepting = false;
        let footer = match &self.identity {
            DemoRecordingIdentity::Q2Server => Vec::new(),
            DemoRecordingIdentity::Mvd { .. } => vec![0, 0],
            DemoRecordingIdentity::Q1 { .. } => write_nq_demo_record(&NqDemoRecord {
                view_angles: [self.angles.x, self.angles.y, self.angles.z],
                message: vec![2],
            })?,
            DemoRecordingIdentity::Qw => {
                let mut message = vec![255, 255, 255, 255, 2];
                message.extend_from_slice(b"EndOfDemo");
                message.push(0);
                write_qw_demo_record(&QwDemoRecord::Packet {
                    seconds: self.seconds,
                    message,
                })?
            }
            DemoRecordingIdentity::Q2 { .. } => finish_q2_demo(),
            DemoRecordingIdentity::Q3 => finish_q3_demo(),
        };
        self.finishing = true;
        let outcome = match self.write(&footer) {
            Ok(()) => match self.file.as_mut() {
                Some(file) => file.sync_all().map_err(DemoRecordingError::from),
                None => Ok(()),
            },
            Err(error) => Err(error),
        };
        self.file.take();
        if let Err(error) = &outcome {
            self.settled = Some(error.to_string());
        }
        outcome
    }

    /// Close without a footer.
    pub fn abort(&mut self) -> Result<(), DemoRecordingError> {
        if self.finishing {
            return self.settled_result();
        }
        self.accepting = false;
        self.finishing = true;
        self.file.take();
        Ok(())
    }
}

impl DemoRecordingSink for DemoRecording {
    fn append(&mut self, packet: &DemoRecordingPacket) -> Result<(), DemoRecordingError> {
        DemoRecording::append(self, packet)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn root(name: &str) -> PathBuf {
        let root: PathBuf = std::env::temp_dir().join(format!("qa-demo-recording-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        root
    }

    fn q1_seed() -> DemoRecordingSeed {
        DemoRecordingSeed {
            identity: DemoRecordingIdentity::Q1 {
                protocol: Q1DemoProtocol::V15,
                track: 2.0,
            },
            packets: vec![DemoRecordingPacket::Q1 {
                message: vec![9, 9],
                view_angles: Vec3 { x: 1.0, y: 2.0, z: 3.0 },
            }],
        }
    }

    #[test]
    fn recording_paths() {
        let root = PathBuf::from("/root");
        let q1 = DemoRecordingIdentity::Q1 {
            protocol: Q1DemoProtocol::V666,
            track: -1.0,
        };
        assert_eq!(
            recording_path(&root, "foo", &q1).unwrap(),
            PathBuf::from("/root/foo.dem")
        );
        assert_eq!(
            recording_path(&root, "foo.DEM", &q1).unwrap(),
            PathBuf::from("/root/foo.DEM")
        );
        assert_eq!(
            recording_path(&root, "bar", &DemoRecordingIdentity::Q3).unwrap(),
            PathBuf::from("/root/demos/bar.dm_68")
        );
        assert_eq!(
            recording_path(
                &root,
                "demos/x",
                &DemoRecordingIdentity::Q2 {
                    protocol: Q2ProtocolIdentity::Classic
                }
            )
            .unwrap(),
            PathBuf::from("/root/demos/x.dm2")
        );
        assert!(recording_path(&root, "../escape", &q1).is_err());
    }

    #[test]
    fn bad_seeds_fail() {
        let root = root("seed");
        let empty = DemoRecordingSeed {
            identity: DemoRecordingIdentity::Qw,
            packets: Vec::new(),
        };
        assert!(matches!(
            DemoRecording::open(&root, "x", &empty).unwrap_err(),
            DemoRecordingError::BadSeed
        ));
        let mut mixed = q1_seed();
        mixed.packets.push(DemoRecordingPacket::Q2 { message: vec![1] });
        assert!(matches!(
            DemoRecording::open(&root, "x", &mixed).unwrap_err(),
            DemoRecordingError::BadSeed
        ));
    }

    #[test]
    fn q1_records_header_packets_and_footer() {
        let root = root("q1");
        let mut recording = DemoRecording::open(&root, "run", &q1_seed()).unwrap();
        assert_eq!(recording.path(), root.join("run.dem"));
        recording
            .append(&DemoRecordingPacket::Q1 {
                message: vec![5],
                view_angles: Vec3 { x: 4.0, y: 5.0, z: 6.0 },
            })
            .unwrap();
        assert!(matches!(
            recording
                .append(&DemoRecordingPacket::Q2 { message: vec![1] })
                .unwrap_err(),
            DemoRecordingError::ProtocolChanged
        ));
        recording.stop().unwrap();
        recording.stop().unwrap();
        assert!(matches!(
            recording.append(&q1_seed().packets[0]).unwrap_err(),
            DemoRecordingError::Stopped
        ));
        let bytes = std::fs::read(root.join("run.dem")).unwrap();
        assert!(bytes.starts_with(b"2\n"));
        assert_eq!(bytes.last(), Some(&2));
    }

    #[test]
    fn exclusive_create_and_mvd_footer() {
        let root = root("mvd");
        let seed = DemoRecordingSeed {
            identity: DemoRecordingIdentity::Mvd {
                revision: MvdRevision::R2013,
            },
            packets: vec![DemoRecordingPacket::Mvd { message: vec![1, 2] }],
        };
        let mut first = DemoRecording::open(&root, "tv", &seed).unwrap();
        assert!(DemoRecording::open(&root, "tv", &seed).is_err());
        first.stop().unwrap();
        let bytes = std::fs::read(root.join("demos/tv.mvd")).unwrap();
        assert!(bytes.ends_with(&[0, 0]));
    }

    #[test]
    fn abort_skips_footer() {
        let root = root("abort");
        let mut recording = DemoRecording::open(&root, "q", &q1_seed()).unwrap();
        recording.abort().unwrap();
        recording.stop().unwrap();
        let bytes = std::fs::read(root.join("q.dem")).unwrap();
        assert_ne!(bytes.last(), Some(&2));
    }

    #[test]
    fn identity_versions() {
        assert_eq!(Q1DemoProtocol::V999.version(), 999);
        assert_eq!(MvdRevision::R3038.revision(), 3038);
        assert_eq!(R1Q2Revision::R1905.revision(), 1905);
        assert_eq!(Q2ProRevision::R1026.revision(), 1026);
        assert_eq!(Q2ProtocolIdentity::Rerelease.version(), 1038);
        assert_eq!(
            DemoRecordingIdentity::Q2 {
                protocol: Q2ProtocolIdentity::KexDemo
            }
            .kind(),
            RecordingKind::Q2
        );
    }
}
