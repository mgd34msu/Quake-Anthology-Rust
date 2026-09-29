//! Cinematic types: clocks, targets, frames, timelines.
//!
//! Donor provenance: `src/media/types.ts`.

use qa_core::identity::SeatId;

/// A media clock (`MediaClock`, milliseconds).
pub trait MediaClock {
    /// Sample the clock.
    fn sample(&self) -> f64;
}

/// A cinematic target (`CinematicTarget`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CinematicTarget {
    /// Fullscreen seat.
    Seat(SeatId),
    /// Material id.
    Material(String),
}

/// A cinematic frame (`CinematicFrame`).
#[derive(Debug, Clone, PartialEq)]
pub struct CinematicFrame {
    /// RGBA pixels.
    pub rgba: Vec<u8>,
    /// Width.
    pub width: usize,
    /// Height.
    pub height: usize,
    /// Frame index.
    pub index: usize,
    /// Loop count (donor `loop`; named `pass` like the donor playbacks).
    pub pass: usize,
    /// Source time in milliseconds.
    pub source_time: f64,
    /// Presentation time in milliseconds.
    pub time: f64,
    /// Pixels decoded (false for timing-only container streams).
    pub decoded: bool,
}

/// Cinematic audio (`CinematicAudio`).
#[derive(Debug, Clone, PartialEq)]
pub struct CinematicAudio {
    /// Samples (16-bit or 8-bit).
    pub samples: AudioSamples,
    /// Channels.
    pub channels: u8,
    /// Sample rate.
    pub sample_rate: u32,
    /// Source sample.
    pub source_sample: usize,
    /// Source time in milliseconds.
    pub source_time: f64,
    /// Presentation time in milliseconds.
    pub time: f64,
    /// Loop count.
    pub pass: usize,
    /// Reset the stream.
    pub reset_stream: bool,
}

/// Audio samples.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AudioSamples {
    /// 16-bit samples.
    I16(Vec<i16>),
    /// 8-bit samples.
    U8(Vec<u8>),
}

impl AudioSamples {
    /// Sample count.
    #[must_use]
    pub fn len(&self) -> usize {
        match self {
            Self::I16(samples) => samples.len(),
            Self::U8(samples) => samples.len(),
        }
    }

    /// Whether no samples are present.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// Cinematic status (`CinematicStatus`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CinematicStatus {
    /// Playing.
    Playing,
    /// Paused.
    Paused,
    /// Held.
    Held,
    /// Ended.
    Ended,
    /// Stopped.
    Stopped,
}

/// Decoder status (`CinematicStatus | "looped"`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecoderStatus {
    /// Playing.
    Playing,
    /// Paused.
    Paused,
    /// Held.
    Held,
    /// Ended.
    Ended,
    /// Stopped.
    Stopped,
    /// Looped (transient decoder status).
    Looped,
}

impl DecoderStatus {
    /// Project to a cinematic status (loops report playing).
    #[must_use]
    pub const fn cinematic(&self) -> CinematicStatus {
        match self {
            Self::Playing | Self::Looped => CinematicStatus::Playing,
            Self::Paused => CinematicStatus::Paused,
            Self::Held => CinematicStatus::Held,
            Self::Ended => CinematicStatus::Ended,
            Self::Stopped => CinematicStatus::Stopped,
        }
    }
}

/// A cinematic timeline (`CinematicTimeline`).
#[derive(Debug, Clone, PartialEq)]
pub struct CinematicTimeline {
    /// Source name.
    pub source: String,
    /// Source time in milliseconds.
    pub source_time_ms: f64,
    /// Elapsed milliseconds.
    pub elapsed_ms: f64,
    /// Loop count.
    pub pass: usize,
    /// Status.
    pub status: CinematicStatus,
}

/// A cinematic end reason (`CinematicEndReason`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CinematicEndReason {
    /// Finished.
    Finished,
    /// Skipped.
    Skipped,
    /// Stopped.
    Stopped,
}

/// A cinematic tick (`CinematicTick`).
#[derive(Debug, Clone, PartialEq)]
pub struct CinematicTick {
    /// Status.
    pub status: CinematicStatus,
    /// Frame.
    pub frame: Option<CinematicFrame>,
    /// Frame changed.
    pub changed: bool,
}

/// Cinematic event host (`CinematicOptions` callbacks).
pub trait CinematicHost {
    /// Queue audio.
    fn on_audio(&mut self, audio: &CinematicAudio, target: &CinematicTarget);
    /// Reset audio.
    fn on_audio_reset(&mut self, target: &CinematicTarget);
    /// Pause audio.
    fn on_audio_pause(&mut self, paused: bool, target: &CinematicTarget);
    /// Complete.
    fn on_complete(&mut self, reason: CinematicEndReason, target: &CinematicTarget);
    /// Developer print.
    fn developer_print(&mut self, _message: &str) {}
}

/// Cinematic options (`CinematicOptions`).
pub struct CinematicOptions<'a> {
    /// Target.
    pub target: CinematicTarget,
    /// Loop.
    pub loop_playback: bool,
    /// Hold the last frame.
    pub hold: bool,
    /// Silent.
    pub silent: bool,
    /// Host.
    pub host: &'a mut dyn CinematicHost,
}

/// Audio sink for decoder playbacks (host plus target).
pub trait CinematicAudioSink {
    /// Queue audio.
    fn on_audio(&mut self, audio: &CinematicAudio);
    /// Reset the mixer lane (RoQ stereo before the first info).
    fn on_audio_reset(&mut self) {}
    /// Developer print.
    fn developer_print(&mut self, message: &str);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_clone_is_deep() {
        let frame = CinematicFrame {
            rgba: vec![1, 2, 3, 4],
            width: 1,
            height: 1,
            index: 0,
            pass: 0,
            source_time: 0.0,
            time: 0.0,
            decoded: true,
        };
        let mut copy = frame.clone();
        copy.rgba[0] = 9;
        assert_eq!(frame.rgba[0], 1);
    }
}
