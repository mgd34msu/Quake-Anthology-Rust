//! Engine-facing audio contracts.
//!
//! Donor provenance: `src/audio/types.ts` (`PlaySound`, `LoopSound`,
//! `AudioListener`, `AudioAudience`, `AudioStreamTarget`, `StreamPcm`,
//! `SoundAsset`, `AudioVoiceEvent`, `AudioVoiceClock`).

use std::rc::Rc;

use qa_core::identity::{ActorId, ProviderId, SeatId};
use qa_core::math::{Axis, Vec3};

use super::wav::PcmSound;

/// Shared decoded sound.
pub type SharedPcm = Rc<PcmSound>;

/// Registered sound asset.
#[derive(Debug, Clone, PartialEq)]
pub struct SoundAsset {
    /// Content resource id.
    pub resource: String,
    /// Registered name.
    pub name: String,
    /// Decoded PCM.
    pub pcm: SharedPcm,
}

/// Where a sound plays.
#[derive(Debug, Clone, PartialEq)]
pub enum SoundOrigin {
    /// At the listener.
    Local,
    /// At a fixed position.
    Fixed {
        /// Position.
        position: Vec3,
    },
    /// Following an actor.
    Actor {
        /// Actor id.
        actor: ActorId,
    },
}

/// Who hears a sound.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AudioAudience {
    /// Every seat.
    World,
    /// One seat.
    Seat {
        /// Seat id.
        seat: SeatId,
    },
}

/// One-shot playback request.
#[derive(Debug, Clone, PartialEq)]
pub struct PlaySound {
    /// Game family.
    pub family: crate::audio::SoundFamily,
    /// Sound asset.
    pub sound: SoundAsset,
    /// Voice origin.
    pub origin: SoundOrigin,
    /// Owning actor, if any.
    pub actor: Option<ActorId>,
    /// Q3 guest owner, if any.
    pub owner: Option<ProviderId>,
    /// Game channel number.
    pub channel: i32,
    /// Volume 0..1.
    pub volume: f64,
    /// Attenuation.
    pub attenuation: f64,
    /// Audience.
    pub audience: AudioAudience,
    /// Q2 synchronized start delay in seconds.
    pub delay_seconds: Option<f64>,
    /// Q2 server clock in milliseconds.
    pub server_milliseconds: Option<f64>,
}

/// Looping playback request.
#[derive(Debug, Clone, PartialEq)]
pub struct LoopSound {
    /// Game family.
    pub family: crate::audio::SoundFamily,
    /// Sound asset.
    pub sound: SoundAsset,
    /// Voice origin.
    pub origin: SoundOrigin,
    /// Owning actor.
    pub actor: ActorId,
    /// Q3 guest owner, if any.
    pub owner: Option<ProviderId>,
    /// Frame velocity.
    pub velocity: Vec3,
    /// Frame number (Q3 loops).
    pub frame_number: i32,
    /// Volume 0..1.
    pub volume: f64,
    /// Attenuation.
    pub attenuation: f64,
    /// Lifetime.
    pub lifetime: LoopLifetime,
    /// Audience.
    pub audience: AudioAudience,
}

/// Loop lifetime.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoopLifetime {
    /// Cleared each loop frame.
    Frame,
    /// Survives loop frames.
    Persistent,
}

/// Streamed PCM samples: native shorts or unsigned bytes.
#[derive(Debug, Clone, PartialEq)]
pub enum StreamSamples {
    /// Signed 16-bit samples.
    S16(Vec<i16>),
    /// Unsigned 8-bit samples.
    U8(Vec<u8>),
}

/// Streamed PCM chunk.
#[derive(Debug, Clone, PartialEq)]
pub struct StreamPcm {
    /// Interleaved samples.
    pub samples: StreamSamples,
    /// Sample rate in Hz.
    pub sample_rate: u32,
    /// Channel count.
    pub channels: u8,
    /// Source sample cursor.
    pub source_sample: i64,
    /// Reset the stream before writing.
    pub reset_stream: bool,
}

/// Stream target.
#[derive(Debug, Clone, PartialEq)]
pub struct AudioStreamTarget {
    /// Stream lane id.
    pub id: String,
    /// Lane gain.
    pub gain: f64,
    /// Audience.
    pub audience: AudioAudience,
}

/// One listener.
#[derive(Debug, Clone, PartialEq)]
pub struct AudioListener {
    /// Seat id.
    pub seat: SeatId,
    /// Listener actor, if any.
    pub actor: Option<ActorId>,
    /// Listener origin.
    pub origin: Vec3,
    /// Listener axis.
    pub axis: Axis,
    /// Listener gain.
    pub gain: f64,
    /// Underwater filter active.
    pub underwater: bool,
}

/// Voice lifecycle event.
#[derive(Debug, Clone, PartialEq)]
pub enum AudioVoiceEvent {
    /// A voice started painting.
    Start {
        /// Seat id.
        seat: SeatId,
        /// Voice id.
        voice_id: u64,
        /// Sound asset.
        sound: SoundAsset,
        /// Output sample of the event.
        output_sample: i64,
        /// Output rate in Hz.
        sample_rate: u32,
        /// Source offset in seconds.
        source_offset_seconds: f64,
    },
    /// A voice stopped.
    Stop {
        /// Seat id.
        seat: SeatId,
        /// Voice id.
        voice_id: u64,
        /// Output sample of the event.
        output_sample: i64,
        /// Stop reason.
        reason: VoiceStopReason,
    },
}

/// Voice stop reason.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VoiceStopReason {
    /// Reached the end.
    Ended,
    /// Stopped explicitly.
    Stopped,
    /// Replaced by a channel steal.
    Replaced,
}

/// Voice clock snapshot.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AudioVoiceClock {
    /// Current output sample.
    pub output_sample: i64,
    /// Output rate in Hz.
    pub sample_rate: u32,
    /// Engine paused.
    pub paused: bool,
}
