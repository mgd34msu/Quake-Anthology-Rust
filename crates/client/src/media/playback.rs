//! Playback timing: clocks, decoder orchestration, hold/loop/ended.
//!
//! Donor provenance: `src/media/playback.ts` (`CinematicPlayback`,
//! `PlaybackClock`) over the `CinPlayback`, `OgvPlayback` and
//! `RoqPlayback` decoders.

use super::cin_playback::CinPlaybackCheckpoint;
use super::cin_playback::{CinPlayback, CinPlaybackOptions, CinPlaybackStatus, CinPlaybackUpdate};
use super::ogv_playback::OgvPlaybackCheckpoint;
use super::ogv_playback::{OgvPlayback, OgvPlaybackOptions, OgvPlaybackStatus, OgvPlaybackUpdate};
use super::roq::RoqDecoderScratch;
use super::roq_playback::RoqPlaybackCheckpoint;
use super::roq_playback::{RoqPlayback, RoqPlaybackOptions, RoqPlaybackStatus, RoqPlaybackUpdate};
use super::types::{
    CinematicAudio, CinematicAudioSink, CinematicEndReason, CinematicFrame, CinematicHost, CinematicStatus,
    CinematicTarget, CinematicTick, CinematicTimeline, DecoderStatus,
};
use crate::ClientError;

/// A RoQ frame pointer (`RoqFramePointer`, page-flip offset).
pub use super::roq_playback::RoqFramePointer;

/// A playback clock (`PlaybackClock`, milliseconds).
#[derive(Debug, Clone, PartialEq)]
pub struct PlaybackClock {
    start: f64,
    paused_at: Option<f64>,
    paused_duration: f64,
    offset: f64,
}

impl PlaybackClock {
    /// New clock over a wall sample.
    pub fn new(wall_now: f64) -> Result<Self, ClientError> {
        check_wall(wall_now)?;
        Ok(Self {
            start: wall_now,
            paused_at: None,
            paused_duration: 0.0,
            offset: 0.0,
        })
    }

    /// Sample the clock.
    pub fn sample(&self, wall_now: f64) -> Result<f64, ClientError> {
        check_wall(wall_now)?;
        Ok((self.offset + self.paused_at.unwrap_or(wall_now) - self.start - self.paused_duration).max(0.0))
    }

    /// Pause or resume.
    pub fn pause(&mut self, paused: bool, wall_now: f64) -> Result<(), ClientError> {
        check_wall(wall_now)?;
        if paused && self.paused_at.is_none() {
            self.paused_at = Some(wall_now);
        } else if !paused && self.paused_at.is_some() {
            self.paused_duration += wall_now - self.paused_at.take().unwrap_or(wall_now);
        }
        Ok(())
    }

    /// Capture a checkpoint.
    pub fn capture(&self, wall_now: f64) -> Result<ClockCheckpoint, ClientError> {
        Ok(ClockCheckpoint {
            elapsed: self.sample(wall_now)?,
            paused: self.paused_at.is_some(),
        })
    }

    /// Restore a checkpoint.
    pub fn restore(wall_now: f64, checkpoint: &ClockCheckpoint) -> Result<Self, ClientError> {
        check_wall(wall_now)?;
        if checkpoint.elapsed < 0.0 {
            return Err(ClientError::BadMedia("negative elapsed time".to_string()));
        }
        Ok(Self {
            start: wall_now,
            paused_at: if checkpoint.paused { Some(wall_now) } else { None },
            paused_duration: 0.0,
            offset: checkpoint.elapsed,
        })
    }
}

fn check_wall(time: f64) -> Result<(), ClientError> {
    if !time.is_finite() || time < 0.0 {
        return Err(ClientError::BadMedia(
            "Media clock must be finite and nonnegative".to_string(),
        ));
    }
    Ok(())
}

/// A clock checkpoint.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ClockCheckpoint {
    /// Elapsed milliseconds.
    pub elapsed: f64,
    /// Paused.
    pub paused: bool,
}

/// Playback focus (`CinPlayback::run` focus).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlaybackFocus {
    /// Game.
    Game,
    /// Console.
    Console,
    /// Menu.
    Menu,
}

/// A cinematic source (`CinematicSource`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CinematicSource {
    /// RoQ bytes.
    Roq {
        /// Source name.
        source: String,
        /// Bytes.
        bytes: Vec<u8>,
    },
    /// CIN bytes.
    Cin {
        /// Source name.
        source: String,
        /// Bytes.
        bytes: Vec<u8>,
    },
    /// OGV bytes.
    Ogv {
        /// Source name.
        source: String,
        /// Bytes.
        bytes: Vec<u8>,
    },
    /// Still image.
    Image {
        /// Source name.
        source: String,
        /// Width.
        width: usize,
        /// Height.
        height: usize,
        /// RGBA pixels.
        rgba: Vec<u8>,
    },
}

/// Build a byte source (`cinematicBytes`).
#[must_use]
pub fn cinematic_bytes(format: &str, bytes: Vec<u8>, source: &str) -> Option<CinematicSource> {
    match format {
        "roq" => Some(CinematicSource::Roq {
            source: source.to_string(),
            bytes,
        }),
        "cin" => Some(CinematicSource::Cin {
            source: source.to_string(),
            bytes,
        }),
        "ogv" => Some(CinematicSource::Ogv {
            source: source.to_string(),
            bytes,
        }),
        _ => None,
    }
}

impl CinematicSource {
    /// Source name.
    #[must_use]
    pub fn source(&self) -> &str {
        match self {
            Self::Roq { source, .. }
            | Self::Cin { source, .. }
            | Self::Ogv { source, .. }
            | Self::Image { source, .. } => source,
        }
    }

    /// Format name.
    #[must_use]
    pub const fn format(&self) -> &'static str {
        match self {
            Self::Roq { .. } => "roq",
            Self::Cin { .. } => "cin",
            Self::Ogv { .. } => "ogv",
            Self::Image { .. } => "image",
        }
    }
}

enum Movie {
    Roq(RoqPlayback),
    Cin(CinPlayback),
    Ogv(OgvPlayback),
    Image,
}

struct HostSink<'a> {
    host: &'a mut dyn CinematicHost,
    target: &'a CinematicTarget,
}

impl CinematicAudioSink for HostSink<'_> {
    fn on_audio(&mut self, audio: &CinematicAudio) {
        self.host.on_audio(audio, self.target);
    }

    fn on_audio_reset(&mut self) {
        self.host.on_audio_reset(self.target);
    }

    fn developer_print(&mut self, message: &str) {
        self.host.developer_print(message);
    }
}

/// A playback checkpoint.
#[derive(Debug, Clone, PartialEq)]
pub struct PlaybackCheckpoint {
    /// Version (1).
    pub version: u32,
    /// Source name.
    pub source: String,
    /// Format.
    pub format: String,
    /// Loop.
    pub loop_playback: bool,
    /// Hold.
    pub hold: bool,
    /// Silent.
    pub silent: bool,
    /// Shader target.
    pub shader: bool,
    /// Clock.
    pub clock: ClockCheckpoint,
    /// State.
    pub state: CinematicStatus,
    /// Decoder status.
    pub decoder_status: DecoderStatus,
    /// Picture.
    pub picture: Option<CinematicFrame>,
    /// Dirty.
    pub dirty: bool,
    /// Frame revision.
    pub frame_revision: usize,
    /// Completed.
    pub completed: bool,
    /// Closed.
    pub closed: bool,
    /// Decoder checkpoints.
    pub stream: StreamCheckpoint,
}

/// Decoder checkpoints by format.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct StreamCheckpoint {
    /// RoQ.
    pub roq: Option<RoqPlaybackCheckpoint>,
    /// CIN.
    pub cin: Option<CinPlaybackCheckpoint>,
    /// OGV.
    pub ogv: Option<OgvPlaybackCheckpoint>,
}

/// Cinematic playback (`CinematicPlayback`).
pub struct CinematicPlayback {
    target: CinematicTarget,
    clock: PlaybackClock,
    movie: Movie,
    state: CinematicStatus,
    decoder_status: DecoderStatus,
    picture: Option<CinematicFrame>,
    dirty: bool,
    frame_revision: usize,
    completed: bool,
    closed: bool,
    source_name: String,
    format: &'static str,
    loop_playback: bool,
    hold: bool,
    silent: bool,
    last_wall: f64,
}

impl CinematicPlayback {
    /// New playback.
    pub fn open(
        source: &CinematicSource,
        target: CinematicTarget,
        wall_now: f64,
        loop_playback: bool,
        hold: bool,
        silent: bool,
    ) -> Result<Self, ClientError> {
        let clock = PlaybackClock::new(wall_now)?;
        let shader = matches!(target, CinematicTarget::Material(_));
        let format = source.format();
        let source_name = source.source().to_string();
        match source {
            CinematicSource::Image {
                width, height, rgba, ..
            } => {
                if *width == 0 || *height == 0 || rgba.len() != width * height * 4 {
                    return Err(ClientError::BadMedia("Invalid cinematic still image".to_string()));
                }
                let frame = CinematicFrame {
                    rgba: rgba.clone(),
                    width: *width,
                    height: *height,
                    index: 0,
                    pass: 0,
                    source_time: 0.0,
                    time: 0.0,
                    decoded: true,
                };
                let mut clock = clock;
                clock.pause(true, wall_now)?;
                Ok(Self {
                    target,
                    clock,
                    movie: Movie::Image,
                    state: CinematicStatus::Held,
                    decoder_status: DecoderStatus::Held,
                    picture: Some(frame),
                    dirty: true,
                    frame_revision: 0,
                    completed: false,
                    closed: false,
                    source_name,
                    format,
                    loop_playback,
                    hold,
                    silent,
                    last_wall: wall_now,
                })
            }
            CinematicSource::Roq { source, bytes } => {
                if bytes.is_empty() {
                    return Err(ClientError::BadMedia(format!("Empty cinematic: {source}")));
                }
                let playback = RoqPlayback::from_bytes(
                    bytes.clone(),
                    source,
                    RoqPlaybackOptions {
                        loop_playback,
                        hold,
                        silent,
                        shader,
                        scratch: Some(RoqDecoderScratch::new()),
                        on_info: None,
                        on_frame: None,
                    },
                    wall_now,
                )?;
                Ok(Self::streaming(
                    target,
                    clock,
                    Movie::Roq(playback),
                    None,
                    false,
                    source_name,
                    format,
                    loop_playback,
                    hold,
                    silent,
                    wall_now,
                ))
            }
            CinematicSource::Cin { source, bytes } => {
                let playback = CinPlayback::from_bytes(
                    bytes.clone(),
                    source,
                    CinPlaybackOptions {
                        loop_playback,
                        hold,
                        silent,
                    },
                    wall_now,
                )?;
                let picture = playback.current_frame()?;
                Ok(Self::streaming(
                    target,
                    clock,
                    Movie::Cin(playback),
                    picture,
                    true,
                    source_name,
                    format,
                    loop_playback,
                    hold,
                    silent,
                    wall_now,
                ))
            }
            CinematicSource::Ogv { bytes, .. } => {
                let playback = OgvPlayback::new(
                    bytes,
                    OgvPlaybackOptions {
                        loop_playback,
                        hold,
                        silent,
                    },
                    wall_now,
                )?;
                let picture = playback.current_frame();
                Ok(Self::streaming(
                    target,
                    clock,
                    Movie::Ogv(playback),
                    picture,
                    true,
                    source_name,
                    format,
                    loop_playback,
                    hold,
                    silent,
                    wall_now,
                ))
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn streaming(
        target: CinematicTarget,
        clock: PlaybackClock,
        movie: Movie,
        picture: Option<CinematicFrame>,
        dirty: bool,
        source_name: String,
        format: &'static str,
        loop_playback: bool,
        hold: bool,
        silent: bool,
        wall_now: f64,
    ) -> Self {
        Self {
            target,
            clock,
            movie,
            state: CinematicStatus::Playing,
            decoder_status: DecoderStatus::Playing,
            picture,
            dirty,
            frame_revision: 0,
            completed: false,
            closed: false,
            source_name,
            format,
            loop_playback,
            hold,
            silent,
            last_wall: wall_now,
        }
    }

    /// Open playback from a checkpoint.
    pub fn open_restored(
        source: &CinematicSource,
        target: CinematicTarget,
        wall_now: f64,
        loop_playback: bool,
        hold: bool,
        silent: bool,
        checkpoint: &PlaybackCheckpoint,
    ) -> Result<Self, ClientError> {
        if checkpoint.version != 1 {
            return Err(ClientError::BadMedia("unsupported cinematic checkpoint".to_string()));
        }
        if checkpoint.source != source.source()
            || checkpoint.format != source.format()
            || checkpoint.loop_playback != loop_playback
            || checkpoint.hold != hold
            || checkpoint.silent != silent
            || checkpoint.shader != matches!(target, CinematicTarget::Material(_))
        {
            return Err(ClientError::BadMedia(
                "cinematic checkpoint identity differs".to_string(),
            ));
        }
        let clock = PlaybackClock::restore(wall_now, &checkpoint.clock)?;
        let format = source.format();
        let source_name = source.source().to_string();
        let movie = match source {
            CinematicSource::Image {
                width, height, rgba, ..
            } => {
                if *width == 0 || *height == 0 || rgba.len() != width * height * 4 {
                    return Err(ClientError::BadMedia("Invalid cinematic still image".to_string()));
                }
                Movie::Image
            }
            CinematicSource::Roq { source, bytes } => {
                let mut playback = RoqPlayback::from_bytes(
                    bytes.clone(),
                    source,
                    RoqPlaybackOptions {
                        loop_playback,
                        hold,
                        silent,
                        shader: matches!(target, CinematicTarget::Material(_)),
                        scratch: Some(RoqDecoderScratch::new()),
                        on_info: None,
                        on_frame: None,
                    },
                    wall_now,
                )?;
                playback.restore_checkpoint(
                    checkpoint
                        .stream
                        .roq
                        .as_ref()
                        .ok_or_else(|| ClientError::BadMedia("missing RoQ checkpoint".to_string()))?,
                )?;
                Movie::Roq(playback)
            }
            CinematicSource::Cin { source, bytes } => {
                let mut playback = CinPlayback::from_bytes(
                    bytes.clone(),
                    source,
                    CinPlaybackOptions {
                        loop_playback,
                        hold,
                        silent,
                    },
                    wall_now,
                )?;
                playback.restore_checkpoint(
                    checkpoint
                        .stream
                        .cin
                        .as_ref()
                        .ok_or_else(|| ClientError::BadMedia("missing CIN checkpoint".to_string()))?,
                )?;
                Movie::Cin(playback)
            }
            CinematicSource::Ogv { bytes, .. } => Movie::Ogv(OgvPlayback::open_restored(
                bytes,
                OgvPlaybackOptions {
                    loop_playback,
                    hold,
                    silent,
                },
                checkpoint
                    .stream
                    .ogv
                    .as_ref()
                    .ok_or_else(|| ClientError::BadMedia("missing OGV checkpoint".to_string()))?,
            )?),
        };
        Ok(Self {
            target,
            clock,
            movie,
            state: checkpoint.state,
            decoder_status: checkpoint.decoder_status,
            picture: checkpoint.picture.clone(),
            dirty: checkpoint.dirty,
            frame_revision: checkpoint.frame_revision,
            completed: checkpoint.completed,
            closed: checkpoint.closed,
            source_name,
            format,
            loop_playback,
            hold,
            silent,
            last_wall: wall_now,
        })
    }

    /// Target.
    #[must_use]
    pub const fn target(&self) -> &CinematicTarget {
        &self.target
    }

    /// Status.
    #[must_use]
    pub const fn status(&self) -> CinematicStatus {
        self.state
    }

    /// Decoder status.
    #[must_use]
    pub const fn decoder_status(&self) -> DecoderStatus {
        self.decoder_status
    }

    /// Source status (`sourceStatus`, may be looped while playing).
    #[must_use]
    pub const fn source_status(&self) -> DecoderStatus {
        if matches!(self.state, CinematicStatus::Playing) {
            self.decoder_status
        } else {
            match self.state {
                CinematicStatus::Playing => DecoderStatus::Playing,
                CinematicStatus::Paused => DecoderStatus::Paused,
                CinematicStatus::Held => DecoderStatus::Held,
                CinematicStatus::Ended => DecoderStatus::Ended,
                CinematicStatus::Stopped => DecoderStatus::Stopped,
            }
        }
    }

    /// Frame revision.
    #[must_use]
    pub const fn revision(&self) -> usize {
        self.frame_revision
    }

    /// Playback time.
    pub fn playback_time(&self, wall_now: f64) -> Result<f64, ClientError> {
        self.clock.sample(wall_now)
    }

    /// Timeline.
    pub fn timeline(&self, wall_now: f64) -> Result<CinematicTimeline, ClientError> {
        let elapsed = self.clock.sample(wall_now)?;
        Ok(CinematicTimeline {
            source: self.source_name.clone(),
            source_time_ms: match &self.picture {
                Some(frame) => frame.source_time + (elapsed - frame.time).max(0.0),
                None => elapsed,
            },
            elapsed_ms: elapsed,
            pass: self.picture.as_ref().map_or(0, |frame| frame.pass),
            status: self.state,
        })
    }

    /// Current frame.
    #[must_use]
    pub fn current_frame(&self) -> Option<CinematicFrame> {
        self.picture.clone()
    }

    /// Tick (`tick`, wall time in milliseconds).
    pub fn tick(&mut self, wall_now: f64, host: &mut dyn CinematicHost) -> Result<CinematicTick, ClientError> {
        self.tick_with_focus(wall_now, host, PlaybackFocus::Game)
    }

    /// Tick with an explicit focus.
    pub fn tick_with_focus(
        &mut self,
        wall_now: f64,
        host: &mut dyn CinematicHost,
        focus: PlaybackFocus,
    ) -> Result<CinematicTick, ClientError> {
        self.last_wall = wall_now;
        let mut changed = self.dirty;
        self.dirty = false;
        if self.state == CinematicStatus::Playing && !matches!(self.movie, Movie::Image) {
            let now = self.clock.sample(wall_now)?;
            let mut sink = HostSink {
                host: &mut *host,
                target: &self.target,
            };
            match &mut self.movie {
                Movie::Roq(playback) => {
                    let tick = playback.run(now, &mut sink)?;
                    self.decoder_status = match tick.status {
                        RoqPlaybackStatus::Playing => DecoderStatus::Playing,
                        RoqPlaybackStatus::Held => DecoderStatus::Held,
                        RoqPlaybackStatus::Ended => DecoderStatus::Ended,
                        RoqPlaybackStatus::Looped => DecoderStatus::Looped,
                    };
                    if let RoqPlaybackUpdate::Frame(frame) = tick.update {
                        self.picture = Some(CinematicFrame {
                            rgba: frame.rgba,
                            width: frame.width,
                            height: frame.height,
                            index: frame.index as usize,
                            pass: frame.pass,
                            source_time: frame.source_time,
                            time: frame.time,
                            decoded: true,
                        });
                        self.frame_revision += 1;
                        changed = true;
                    }
                }
                Movie::Cin(playback) => {
                    let tick = playback.run(now, focus, &mut sink)?;
                    self.decoder_status = match tick.status {
                        CinPlaybackStatus::Playing => DecoderStatus::Playing,
                        CinPlaybackStatus::Held => DecoderStatus::Held,
                        CinPlaybackStatus::Ended => DecoderStatus::Ended,
                        CinPlaybackStatus::Looped => DecoderStatus::Looped,
                    };
                    if let CinPlaybackUpdate::Frame(frame) = tick.update {
                        self.picture = frame;
                        self.frame_revision += 1;
                        changed = true;
                    }
                }
                Movie::Ogv(playback) => {
                    let tick = playback.run(now, &mut sink)?;
                    self.decoder_status = match tick.status {
                        OgvPlaybackStatus::Playing => DecoderStatus::Playing,
                        OgvPlaybackStatus::Held => DecoderStatus::Held,
                        OgvPlaybackStatus::Ended => DecoderStatus::Ended,
                        OgvPlaybackStatus::Looped => DecoderStatus::Looped,
                    };
                    if let OgvPlaybackUpdate::Frame(frame) = tick.update {
                        self.picture = frame;
                        self.frame_revision += 1;
                        changed = true;
                    }
                }
                Movie::Image => {}
            }
            if self.decoder_status == DecoderStatus::Held {
                self.state = CinematicStatus::Held;
                self.clock.pause(true, wall_now)?;
            } else if self.decoder_status == DecoderStatus::Ended {
                self.state = CinematicStatus::Ended;
                self.finish(CinematicEndReason::Finished, host)?;
            }
        }
        Ok(CinematicTick {
            status: self.state,
            frame: self.picture.clone(),
            changed,
        })
    }

    /// Pause or resume.
    pub fn pause(&mut self, paused: bool, wall_now: f64, host: &mut dyn CinematicHost) -> Result<(), ClientError> {
        self.last_wall = wall_now;
        if (paused && self.state != CinematicStatus::Playing) || (!paused && self.state != CinematicStatus::Paused) {
            return Ok(());
        }
        self.clock.pause(paused, wall_now)?;
        self.state = if paused {
            CinematicStatus::Paused
        } else {
            CinematicStatus::Playing
        };
        host.on_audio_pause(paused, &self.target);
        Ok(())
    }

    /// Skip.
    pub fn skip(&mut self, host: &mut dyn CinematicHost) -> Result<(), ClientError> {
        if self.completed {
            return Ok(());
        }
        self.state = CinematicStatus::Ended;
        self.finish(CinematicEndReason::Skipped, host)
    }

    /// Stop.
    pub fn stop(&mut self, host: &mut dyn CinematicHost) -> Result<(), ClientError> {
        if self.completed {
            return Ok(());
        }
        self.state = CinematicStatus::Stopped;
        self.finish(CinematicEndReason::Stopped, host)
    }

    fn finish(&mut self, reason: CinematicEndReason, host: &mut dyn CinematicHost) -> Result<(), ClientError> {
        if self.completed {
            return Ok(());
        }
        self.completed = true;
        self.clock.pause(true, self.last_wall)?;
        self.release_input();
        host.on_audio_reset(&self.target);
        host.on_complete(reason, &self.target);
        Ok(())
    }

    fn release_input(&mut self) {
        if self.closed {
            return;
        }
        self.closed = true;
        match &mut self.movie {
            Movie::Roq(playback) => playback.close_input(),
            Movie::Cin(playback) => playback.close(),
            Movie::Ogv(playback) => playback.close(),
            Movie::Image => {}
        }
    }

    /// Close.
    pub fn close(&mut self, host: &mut dyn CinematicHost) -> Result<(), ClientError> {
        self.stop(host)?;
        self.release_input();
        Ok(())
    }

    /// Current RoQ frame pointer.
    #[must_use]
    pub fn current_pointer(&self) -> Option<RoqFramePointer> {
        match &self.movie {
            Movie::Roq(playback) => playback.current_pointer(),
            _ => None,
        }
    }

    /// Live RoQ scratch and pointer for presentation.
    #[must_use]
    pub fn roq_presentation(&self) -> Option<(&RoqDecoderScratch, RoqFramePointer)> {
        match &self.movie {
            Movie::Roq(playback) => playback.current_pointer().map(|pointer| (playback.scratch(), pointer)),
            _ => None,
        }
    }

    /// Capture a checkpoint.
    pub fn capture(&self, wall_now: f64) -> Result<PlaybackCheckpoint, ClientError> {
        let stream = match &self.movie {
            Movie::Roq(playback) => StreamCheckpoint {
                roq: Some(playback.capture_checkpoint()?),
                cin: None,
                ogv: None,
            },
            Movie::Cin(playback) => StreamCheckpoint {
                roq: None,
                cin: Some(playback.capture_checkpoint()),
                ogv: None,
            },
            Movie::Ogv(playback) => StreamCheckpoint {
                roq: None,
                cin: None,
                ogv: Some(playback.capture_checkpoint()?),
            },
            Movie::Image => StreamCheckpoint::default(),
        };
        Ok(PlaybackCheckpoint {
            version: 1,
            source: self.source_name.clone(),
            format: self.format.to_string(),
            loop_playback: self.loop_playback,
            hold: self.hold,
            silent: self.silent,
            shader: matches!(self.target, CinematicTarget::Material(_)),
            clock: self.clock.capture(wall_now)?,
            state: self.state,
            decoder_status: self.decoder_status,
            picture: self.picture.clone(),
            dirty: self.dirty,
            frame_revision: self.frame_revision,
            completed: self.completed,
            closed: self.closed,
            stream,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::media::types::CinematicAudio;

    struct Recorder {
        audio: Vec<CinematicAudio>,
        resets: usize,
        pauses: Vec<bool>,
        completions: Vec<CinematicEndReason>,
        prints: Vec<String>,
    }

    impl CinematicHost for Recorder {
        fn on_audio(&mut self, audio: &CinematicAudio, _target: &CinematicTarget) {
            self.audio.push(audio.clone());
        }

        fn on_audio_reset(&mut self, _target: &CinematicTarget) {
            self.resets += 1;
        }

        fn on_audio_pause(&mut self, paused: bool, _target: &CinematicTarget) {
            self.pauses.push(paused);
        }

        fn on_complete(&mut self, reason: CinematicEndReason, _target: &CinematicTarget) {
            self.completions.push(reason);
        }

        fn developer_print(&mut self, message: &str) {
            self.prints.push(message.to_string());
        }
    }

    fn recorder() -> Recorder {
        Recorder {
            audio: Vec::new(),
            resets: 0,
            pauses: Vec::new(),
            completions: Vec::new(),
            prints: Vec::new(),
        }
    }

    fn cin_bytes(frames: usize) -> Vec<u8> {
        let mut header = vec![0u8; 20];
        header[..4].copy_from_slice(&4i32.to_le_bytes());
        header[4..8].copy_from_slice(&4i32.to_le_bytes());
        let mut bytes = [header, vec![0u8; 65536]].concat();
        for _ in 0..frames {
            // Command 0, size 4, trivial Huffman payload (count only).
            bytes.extend_from_slice(&0i32.to_le_bytes());
            bytes.extend_from_slice(&4i32.to_le_bytes());
            bytes.extend_from_slice(&16i32.to_le_bytes());
        }
        bytes.extend_from_slice(&2i32.to_le_bytes());
        bytes
    }

    fn roq_chunk(id: u16, size: usize, flags: u16, payload: &[u8]) -> Vec<u8> {
        let mut out = vec![0u8; 8 + payload.len()];
        out[..2].copy_from_slice(&id.to_le_bytes());
        out[2..6].copy_from_slice(&(size as u32).to_le_bytes());
        out[6..8].copy_from_slice(&flags.to_le_bytes());
        out[8..].copy_from_slice(payload);
        out
    }

    fn roq_bytes() -> Vec<u8> {
        [
            vec![0x84, 0x10, 0, 0, 0, 0, 30, 0],
            roq_chunk(0x1021, 4, 0x1234, &[1, 2, 3, 4]),
            roq_chunk(0x1001, 8, 0, &[8, 0, 8, 0, 0, 0, 0, 0]),
            roq_chunk(0x1002, 10, 0x0101, &[16, 32, 48, 64, 100, 200, 0, 0, 0, 0]),
            roq_chunk(0x1011, 3, 0, &[0x00, 0x80, 0x00]),
            roq_chunk(0x1011, 3, 0, &[0x00, 0x40, 0x88]),
        ]
        .concat()
    }

    fn target() -> CinematicTarget {
        CinematicTarget::Material("cin".to_string())
    }

    #[test]
    fn cin_advances_and_ends() {
        let source = CinematicSource::Cin {
            source: "test.cin".to_string(),
            bytes: cin_bytes(4),
        };
        let mut host = recorder();
        let mut playback = CinematicPlayback::open(&source, target(), 0.0, false, false, false).unwrap();
        assert_eq!(playback.status(), CinematicStatus::Playing);
        assert_eq!(playback.timeline(0.0).unwrap().pass, 0);
        // Donor prefetch order: the prefetched frame first, a blank
        // on first advance, then index 1.
        let tick = playback.tick(0.0, &mut host).unwrap();
        assert!(tick.changed);
        assert_eq!(tick.frame.as_ref().unwrap().index, 0);
        assert!(tick.frame.as_ref().unwrap().decoded);
        let tick = playback.tick(143.0, &mut host).unwrap();
        assert!(tick.frame.is_none());
        let tick = playback.tick(215.0, &mut host).unwrap();
        assert_eq!(tick.frame.as_ref().unwrap().index, 1);
        // One frame per tick like the donor: the drop rebases the
        // epoch, so the drain resumes from the decoded position.
        let tick = playback.tick(10000.0, &mut host).unwrap();
        assert_eq!(tick.status, CinematicStatus::Playing);
        assert_eq!(tick.frame.as_ref().unwrap().index, 2);
        let tick = playback.tick(10001.0, &mut host).unwrap();
        assert_eq!(tick.status, CinematicStatus::Playing);
        let tick = playback.tick(10200.0, &mut host).unwrap();
        assert_eq!(tick.status, CinematicStatus::Ended);
        assert!(tick.frame.is_none());
        assert_eq!(host.completions, vec![CinematicEndReason::Finished]);
        assert_eq!(host.resets, 1);
    }

    #[test]
    fn cin_loops() {
        let source = CinematicSource::Cin {
            source: "test.cin".to_string(),
            bytes: cin_bytes(2),
        };
        let mut host = recorder();
        let mut playback = CinematicPlayback::open(&source, target(), 0.0, true, false, false).unwrap();
        playback.tick(80.0, &mut host).unwrap();
        let tick = playback.tick(1000.0, &mut host).unwrap();
        assert_eq!(tick.status, CinematicStatus::Playing);
        // The drop rebases the epoch; the loop trips once the
        // rebased frame passes the last decoded frame.
        let tick = playback.tick(1200.0, &mut host).unwrap();
        assert_eq!(tick.status, CinematicStatus::Playing);
        assert_eq!(playback.source_status(), DecoderStatus::Looped);
    }

    #[test]
    fn cin_empty_ends_on_first_tick() {
        let source = CinematicSource::Cin {
            source: "empty.cin".to_string(),
            bytes: cin_bytes(0),
        };
        let mut host = recorder();
        let mut playback = CinematicPlayback::open(&source, target(), 0.0, false, false, false).unwrap();
        assert_eq!(playback.status(), CinematicStatus::Playing);
        let tick = playback.tick(0.0, &mut host).unwrap();
        assert_eq!(tick.status, CinematicStatus::Ended);
    }

    #[test]
    fn roq_plays_real_frames_and_audio() {
        let source = CinematicSource::Roq {
            source: "test.roq".to_string(),
            bytes: roq_bytes(),
        };
        let mut host = recorder();
        let mut playback = CinematicPlayback::open(&source, target(), 0.0, false, false, false).unwrap();
        // RoQ starts blank like the donor (no prefetch).
        let tick = playback.tick(0.0, &mut host).unwrap();
        assert!(!tick.changed);
        assert!(tick.frame.is_none());
        // Early stereo audio resets the mixer lane once.
        assert_eq!(host.audio.len(), 1);
        assert!(host.audio[0].reset_stream);
        let tick = playback.tick(34.0, &mut host).unwrap();
        assert!(tick.changed);
        let frame = tick.frame.as_ref().unwrap();
        assert_eq!((frame.width, frame.height), (8, 8));
        assert_eq!(frame.rgba.len(), 256);
        assert!(frame.decoded);
        assert!(playback.current_pointer().is_some());
        assert!(playback.roq_presentation().is_some());
        let tick = playback.tick(67.0, &mut host).unwrap();
        assert_eq!(tick.status, CinematicStatus::Ended);
        assert_eq!(host.completions, vec![CinematicEndReason::Finished]);
        assert_eq!(host.resets, 2);
    }

    #[test]
    fn roq_hold_and_empty() {
        let source = CinematicSource::Roq {
            source: "test.roq".to_string(),
            bytes: roq_bytes(),
        };
        let mut host = recorder();
        let mut playback = CinematicPlayback::open(&source, target(), 0.0, false, true, false).unwrap();
        playback.tick(0.0, &mut host).unwrap();
        playback.tick(34.0, &mut host).unwrap();
        let tick = playback.tick(67.0, &mut host).unwrap();
        assert_eq!(tick.status, CinematicStatus::Held);
        let empty = CinematicSource::Roq {
            source: "empty.roq".to_string(),
            bytes: Vec::new(),
        };
        assert!(CinematicPlayback::open(&empty, target(), 0.0, false, false, false).is_err());
    }

    #[test]
    fn ogv_rejects_fake_movie() {
        let source = CinematicSource::Ogv {
            source: "fake.ogv".to_string(),
            bytes: vec![0u8; 64],
        };
        assert!(CinematicPlayback::open(&source, target(), 0.0, false, false, false).is_err());
    }

    #[test]
    fn pause_freezes_time() {
        let source = CinematicSource::Cin {
            source: "test.cin".to_string(),
            bytes: cin_bytes(3),
        };
        let mut host = recorder();
        let mut playback = CinematicPlayback::open(&source, target(), 0.0, false, false, false).unwrap();
        playback.pause(true, 10.0, &mut host).unwrap();
        assert_eq!(playback.status(), CinematicStatus::Paused);
        assert_eq!(playback.playback_time(1000.0).unwrap(), 10.0);
        playback.pause(false, 1000.0, &mut host).unwrap();
        assert_eq!(playback.playback_time(1010.0).unwrap(), 20.0);
        assert_eq!(host.pauses, vec![true, false]);
        playback.skip(&mut host).unwrap();
        assert_eq!(host.completions, vec![CinematicEndReason::Skipped]);
    }

    #[test]
    fn image_is_held() {
        let source = CinematicSource::Image {
            source: "poster".to_string(),
            width: 2,
            height: 1,
            rgba: vec![0u8; 8],
        };
        let mut host = recorder();
        let mut playback = CinematicPlayback::open(&source, target(), 0.0, false, false, true).unwrap();
        assert_eq!(playback.status(), CinematicStatus::Held);
        let tick = playback.tick(100.0, &mut host).unwrap();
        assert!(tick.frame.as_ref().unwrap().decoded);
    }

    #[test]
    fn checkpoint_round_trip() {
        let source = CinematicSource::Cin {
            source: "test.cin".to_string(),
            bytes: cin_bytes(4),
        };
        let mut host = recorder();
        let mut playback = CinematicPlayback::open(&source, target(), 0.0, false, false, false).unwrap();
        playback.tick(0.0, &mut host).unwrap();
        playback.tick(143.0, &mut host).unwrap();
        let checkpoint = playback.capture(143.0).unwrap();
        assert_eq!(checkpoint.version, 1);
        let mut revived =
            CinematicPlayback::open_restored(&source, target(), 143.0, false, false, false, &checkpoint).unwrap();
        let (a, b) = (
            playback.tick(215.0, &mut host).unwrap(),
            revived.tick(215.0, &mut host).unwrap(),
        );
        assert_eq!(a, b);
        assert_eq!(a.frame.as_ref().unwrap().index, 1);
        let mut bad = checkpoint;
        bad.source = "other.cin".to_string();
        assert!(CinematicPlayback::open_restored(&source, target(), 143.0, false, false, false, &bad).is_err());
    }

    #[test]
    fn focus_rebases_without_advancing() {
        let source = CinematicSource::Cin {
            source: "test.cin".to_string(),
            bytes: cin_bytes(4),
        };
        let mut host = recorder();
        let mut playback = CinematicPlayback::open(&source, target(), 0.0, false, false, false).unwrap();
        let tick = playback.tick_with_focus(0.0, &mut host, PlaybackFocus::Game).unwrap();
        assert_eq!(tick.frame.as_ref().unwrap().index, 0);
        let tick = playback
            .tick_with_focus(5000.0, &mut host, PlaybackFocus::Console)
            .unwrap();
        assert!(!tick.changed);
        assert_eq!(tick.frame.as_ref().unwrap().index, 0);
        let tick = playback
            .tick_with_focus(5000.0, &mut host, PlaybackFocus::Game)
            .unwrap();
        assert!(!tick.changed);
        assert_eq!(tick.frame.as_ref().unwrap().index, 0);
    }

    #[test]
    fn clock_checkpoint_round_trips() {
        let clock = PlaybackClock::new(100.0).unwrap();
        let checkpoint = clock.capture(150.0).unwrap();
        assert_eq!(checkpoint.elapsed, 50.0);
        let restored = PlaybackClock::restore(1000.0, &checkpoint).unwrap();
        assert_eq!(restored.sample(1100.0).unwrap(), 150.0);
    }

    #[test]
    fn byte_sources_cover_formats() {
        let bytes = vec![1, 2, 3];
        assert!(cinematic_bytes("roq", bytes.clone(), "a").is_some());
        assert!(cinematic_bytes("cin", bytes.clone(), "a").is_some());
        assert!(cinematic_bytes("ogv", bytes.clone(), "a").is_some());
        assert!(cinematic_bytes("png", bytes, "a").is_none());
    }
}
