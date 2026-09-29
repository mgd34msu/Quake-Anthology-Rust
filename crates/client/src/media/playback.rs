//! Playback timing: clocks, frame scheduling, hold/loop/ended.
//!
//! Donor provenance: `src/media/playback.ts` (`CinematicPlayback`,
//! `PlaybackClock`), `src/media/cin-playback.ts` (`CinPlayback`),
//! `src/media/ogv-playback.ts` (`OgvPlayback`), and
//! `src/media/roq-playback.ts` (`RoqPlayback`).
//!
//! Timing model over parsed containers: frame scheduling, drop-frame
//! detection, epoch rebasing, pause clocks, and hold/loop/ended
//! transitions match the donor. Frame pixels and audio samples need
//! codec engines (deferred); container streams emit metadata frames
//! (`decoded: false`) with real dimensions, indices, and times.
//! Stills emit decoded frames.

use super::containers::{
    cin_sample_range, decode_ogg_movie, parse_cin_header, parse_roq_chunk_header, parse_roq_header, parse_theora_ident,
    read_cin_chunk, CinChunk, RoqChunkHeader, CIN_FRAME_RATE, ROQ_AUDIO_MONO, ROQ_AUDIO_STEREO, ROQ_CODEBOOK,
    ROQ_FRAME, ROQ_HANG, ROQ_INFO, ROQ_PACKET, ROQ_QUAD_JPEG,
};
use super::source::MemMedia;
use super::types::{
    CinematicEndReason, CinematicFrame, CinematicHost, CinematicStatus, CinematicTarget, CinematicTick,
    CinematicTimeline, DecoderStatus,
};
use crate::ClientError;

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

/// A stream frame time.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StreamFrameTime {
    /// Frame index.
    pub index: usize,
    /// Source time in milliseconds.
    pub source_time: f64,
}

/// A stream timeline (timing only).
#[derive(Debug, Clone, PartialEq)]
pub struct StreamTimeline {
    /// Width.
    pub width: usize,
    /// Height.
    pub height: usize,
    /// Frame times.
    pub frames: Vec<StreamFrameTime>,
    /// Audio format.
    pub audio: Option<StreamAudioFormat>,
}

/// Stream audio format (timing only).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StreamAudioFormat {
    /// Sample rate.
    pub sample_rate: u32,
    /// Channels.
    pub channels: u8,
}

/// Build a CIN timeline.
pub fn build_cin_timeline(bytes: &[u8], source: &str) -> Result<StreamTimeline, ClientError> {
    let mut input = MemMedia::new(bytes.to_vec(), source);
    let header = parse_cin_header(&mut input)?;
    let mut frames = Vec::new();
    let mut offset = 20 + 65536;
    let mut index = 0usize;
    // Guard against pathological container loops.
    for _ in 0..1_000_000 {
        if offset >= bytes.len() {
            break;
        }
        let (chunk, _) = read_cin_chunk(&mut input, offset)?;
        match chunk {
            CinChunk::End => break,
            CinChunk::Frame {
                size, offset: payload, ..
            } => {
                frames.push(StreamFrameTime {
                    index,
                    source_time: index as f64 * 1000.0 / CIN_FRAME_RATE as f64,
                });
                let audio = audio_bytes(&header.audio, index, payload + size, bytes.len(), source)?;
                index += 1;
                offset = payload + size + audio;
            }
        }
    }
    Ok(StreamTimeline {
        width: header.width as usize,
        height: header.height as usize,
        frames,
        audio: header.audio.map(|audio| StreamAudioFormat {
            sample_rate: audio.sample_rate as u32,
            channels: audio.channels,
        }),
    })
}

fn audio_bytes(
    audio: &Option<super::containers::CinAudioFormat>,
    frame: usize,
    offset: usize,
    length: usize,
    source: &str,
) -> Result<usize, ClientError> {
    let Some(audio) = audio else {
        return Ok(0);
    };
    let (from, to) = cin_sample_range(frame as i64, audio.sample_rate as i64)?;
    let bytes = (to - from) as usize * usize::from(audio.channels) * usize::from(audio.sample_bytes);
    // Validate the audio range fits; payload decode is deferred.
    if offset + bytes > length {
        return Err(ClientError::BadMedia(format!("{source}:{offset}: truncated CIN audio")));
    }
    Ok(bytes)
}

/// Build a RoQ timeline (cinematic lookahead).
pub fn build_roq_timeline(bytes: &[u8], source: &str) -> Result<StreamTimeline, ClientError> {
    let (header, mut offset) = parse_roq_header(bytes, source)?;
    let mut frames = Vec::new();
    let mut dimensions: Option<(usize, usize)> = None;
    let mut index = 0usize;
    let mut invalid_lookahead = false;
    for _ in 0..1_000_000 {
        if offset >= bytes.len() {
            break;
        }
        if offset + 8 > bytes.len() {
            break;
        }
        let chunk: RoqChunkHeader = parse_roq_chunk_header(bytes, offset, source)?;
        if chunk.size > 65536 || chunk.id == super::containers::ROQ_MAGIC {
            invalid_lookahead = true;
            break;
        }
        let payload = offset + 8;
        let header_only = matches!(chunk.id, ROQ_PACKET | ROQ_HANG);
        if !header_only && payload + chunk.size > bytes.len() {
            break;
        }
        match chunk.id {
            ROQ_INFO => {
                if payload + 8 > bytes.len() {
                    return Err(ClientError::BadMedia(format!(
                        "{source}:{payload}: truncated RoQ payload"
                    )));
                }
                let width = u16::from_le_bytes([bytes[payload], bytes[payload + 1]]);
                let height = u16::from_le_bytes([bytes[payload + 2], bytes[payload + 3]]);
                if dimensions.is_none() {
                    if width == 0
                        || height == 0
                        || !width.is_multiple_of(8)
                        || !height.is_multiple_of(8)
                        || u32::from(width) * u32::from(height) > 512 * 512
                    {
                        return Err(ClientError::BadMedia(format!(
                            "{source}:{payload}: invalid RoQ quad dimensions"
                        )));
                    }
                    dimensions = Some((width as usize, height as usize));
                }
            }
            ROQ_FRAME => {
                if dimensions.is_none() {
                    return Err(ClientError::BadMedia(format!(
                        "{source}:{payload}: RoQ frame precedes quad info"
                    )));
                }
                frames.push(StreamFrameTime {
                    index,
                    source_time: index as f64 * 1000.0 / f64::from(header.frame_rate),
                });
                index += 1;
            }
            ROQ_CODEBOOK | ROQ_QUAD_JPEG | ROQ_HANG | ROQ_PACKET | ROQ_AUDIO_MONO | ROQ_AUDIO_STEREO => {}
            _ => break,
        }
        offset = payload + if header_only { 0 } else { chunk.size };
    }
    let _ = invalid_lookahead;
    let Some((width, height)) = dimensions else {
        return Err(ClientError::BadMedia(format!(
            "{source}:0: RoQ stream has no quad info"
        )));
    };
    Ok(StreamTimeline {
        width,
        height,
        frames,
        audio: None,
    })
}

/// Build an OGV timeline.
pub fn build_ogv_timeline(bytes: &[u8]) -> Result<StreamTimeline, ClientError> {
    let movie = decode_ogg_movie(bytes)?;
    let Some(first) = movie.video.first() else {
        return Err(ClientError::BadMedia(
            "Ogg movie has no complete Theora video".to_string(),
        ));
    };
    let ident = parse_theora_ident(first)?;
    let count = movie.video.len() - 3;
    Ok(StreamTimeline {
        width: ident.width,
        height: ident.height,
        frames: (0..count)
            .map(|index| StreamFrameTime {
                index,
                source_time: index as f64 * ident.frame_ms,
            })
            .collect(),
        audio: None,
    })
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StreamState {
    Playing,
    Held,
    Ended,
    Looped,
}

#[derive(Debug, Clone)]
struct StreamRunner {
    timeline: StreamTimeline,
    format: &'static str,
    epoch: f64,
    last_time: f64,
    decoded: i64,
    next_index: usize,
    picture: Option<CinematicFrame>,
    pending: Option<CinematicFrame>,
    pass: usize,
    state: StreamState,
    loop_playback: bool,
    hold: bool,
    shader: bool,
}

impl StreamRunner {
    fn frame(&self, index: usize, time: f64) -> Option<CinematicFrame> {
        let frame = self.timeline.frames.get(index)?;
        Some(CinematicFrame {
            rgba: Vec::new(),
            width: self.timeline.width,
            height: self.timeline.height,
            index: frame.index,
            pass: self.pass,
            source_time: frame.source_time,
            time,
            decoded: false,
        })
    }

    /// Read the next frame; `decoded` counts frames read like CIN's
    /// `nextFrameIndex`.
    fn read(&mut self) -> Option<CinematicFrame> {
        let index = self.decoded.max(0) as usize;
        let frame = self.frame(index, self.epoch + self.source_time(index))?;
        self.decoded += 1;
        Some(frame)
    }

    fn source_time(&self, index: usize) -> f64 {
        self.timeline.frames.get(index).map_or(0.0, |frame| frame.source_time)
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
    /// Stream cursor.
    pub stream: StreamCheckpoint,
}

/// A stream checkpoint.
#[derive(Debug, Clone, PartialEq)]
pub struct StreamCheckpoint {
    /// Epoch.
    pub epoch: f64,
    /// Decoded frames.
    pub decoded: i64,
    /// Next index.
    pub next_index: usize,
    /// Picture.
    pub picture: Option<CinematicFrame>,
    /// Pending.
    pub pending: Option<CinematicFrame>,
    /// Pass.
    pub pass: usize,
    /// State name.
    pub state: String,
    /// Last time.
    pub last_time: f64,
}

/// Cinematic playback (`CinematicPlayback`, timing model).
pub struct CinematicPlayback {
    target: CinematicTarget,
    clock: PlaybackClock,
    runner: Option<StreamRunner>,
    still: Option<CinematicFrame>,
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
                    runner: None,
                    still: Some(frame.clone()),
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
            _ => {
                if matches!(source, CinematicSource::Roq { bytes, .. } if bytes.is_empty()) {
                    return Err(ClientError::BadMedia(format!("Empty cinematic: {}", source.source())));
                }
                let timeline = match source {
                    CinematicSource::Roq { source, bytes } => build_roq_timeline(bytes, source)?,
                    CinematicSource::Cin { source, bytes } => build_cin_timeline(bytes, source)?,
                    CinematicSource::Ogv { bytes, .. } => build_ogv_timeline(bytes)?,
                    CinematicSource::Image { .. } => unreachable!(),
                };
                let mut runner = StreamRunner {
                    timeline,
                    format,
                    epoch: wall_now,
                    last_time: wall_now,
                    decoded: 0,
                    next_index: 0,
                    picture: None,
                    pending: None,
                    pass: 0,
                    state: StreamState::Playing,
                    loop_playback,
                    hold,
                    shader,
                };
                let mut state = CinematicStatus::Playing;
                // Prefetch like the donor constructors: CIN reads one
                // frame, OGV decodes frame zero, RoQ starts blank.
                let picture = match format {
                    "cin" => {
                        let picture = runner.read();
                        if picture.is_none() {
                            state = CinematicStatus::Ended;
                            runner.state = StreamState::Ended;
                        } else {
                            runner.picture.clone_from(&picture);
                        }
                        picture
                    }
                    "ogv" => {
                        let picture = runner.frame(0, wall_now);
                        if picture.is_none() {
                            state = CinematicStatus::Ended;
                            runner.state = StreamState::Ended;
                        } else {
                            runner.picture.clone_from(&picture);
                            runner.next_index = 1;
                        }
                        picture
                    }
                    _ => {
                        runner.decoded = -1;
                        None
                    }
                };
                let decoder_status = if state == CinematicStatus::Ended {
                    DecoderStatus::Ended
                } else {
                    DecoderStatus::Playing
                };
                Ok(Self {
                    target,
                    clock,
                    runner: Some(runner),
                    still: None,
                    state,
                    decoder_status,
                    picture,
                    // RoQ starts blank like the donor (no prefetch).
                    dirty: format != "roq",
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
        }
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

    /// Tick with an explicit focus (superset for direct CIN focus timing).
    pub fn tick_with_focus(
        &mut self,
        wall_now: f64,
        host: &mut dyn CinematicHost,
        focus: PlaybackFocus,
    ) -> Result<CinematicTick, ClientError> {
        self.last_wall = wall_now;
        let mut changed = self.dirty;
        self.dirty = false;
        if self.state == CinematicStatus::Playing {
            if let Some(runner) = self.runner.as_mut() {
                let now = self.clock.sample(wall_now)?;
                let (status, update) = run_stream(runner, now, focus, host, &self.target)?;
                self.decoder_status = status;
                if let Some(frame) = update {
                    self.picture = frame;
                    self.frame_revision += 1;
                    changed = true;
                }
                if status == DecoderStatus::Held {
                    self.state = CinematicStatus::Held;
                    self.clock.pause(true, wall_now)?;
                } else if status == DecoderStatus::Ended {
                    self.state = CinematicStatus::Ended;
                    self.finish(CinematicEndReason::Finished, host)?;
                }
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
        host.on_audio_reset(&self.target);
        host.on_complete(reason, &self.target);
        Ok(())
    }

    /// Close.
    pub fn close(&mut self, host: &mut dyn CinematicHost) -> Result<(), ClientError> {
        self.stop(host)?;
        self.closed = true;
        Ok(())
    }

    /// Current RoQ frame pointer (`currentPointer`, timing metadata).
    #[must_use]
    pub fn current_pointer(&self) -> Option<RoqFramePointer> {
        let runner = self.runner.as_ref()?;
        if runner.format != "roq" {
            return None;
        }
        let frame = self.picture.as_ref()?;
        let byte_length = frame.width * frame.height * 4;
        Some(RoqFramePointer {
            offset: byte_length * (frame.index & 1),
            byte_length,
        })
    }

    /// Capture a checkpoint.
    pub fn capture(&self, wall_now: f64) -> Result<PlaybackCheckpoint, ClientError> {
        let stream = match &self.runner {
            Some(runner) => StreamCheckpoint {
                epoch: runner.epoch,
                decoded: runner.decoded,
                next_index: runner.next_index,
                picture: runner.picture.clone(),
                pending: runner.pending.clone(),
                pass: runner.pass,
                state: match runner.state {
                    StreamState::Playing => "playing".to_string(),
                    StreamState::Held => "held".to_string(),
                    StreamState::Ended => "ended".to_string(),
                    StreamState::Looped => "looped".to_string(),
                },
                last_time: runner.last_time,
            },
            None => StreamCheckpoint {
                epoch: 0.0,
                decoded: -1,
                next_index: 0,
                picture: self.still.clone(),
                pending: None,
                pass: 0,
                state: "held".to_string(),
                last_time: 0.0,
            },
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

/// A RoQ frame pointer (`RoqPlaybackFrame["pointer"]`, page-flip offset).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RoqFramePointer {
    /// Byte offset.
    pub offset: usize,
    /// Frame byte length.
    pub byte_length: usize,
}

fn run_stream(
    runner: &mut StreamRunner,
    now: f64,
    focus: PlaybackFocus,
    host: &mut dyn CinematicHost,
    _target: &CinematicTarget,
) -> Result<(DecoderStatus, Option<Option<CinematicFrame>>), ClientError> {
    match runner.format {
        "cin" => run_cin(runner, now, focus, host),
        "roq" => run_roq(runner, now),
        _ => run_ogv(runner, now),
    }
}

fn run_cin(
    runner: &mut StreamRunner,
    now: f64,
    focus: PlaybackFocus,
    host: &mut dyn CinematicHost,
) -> Result<(DecoderStatus, Option<Option<CinematicFrame>>), ClientError> {
    if runner.state == StreamState::Ended || runner.state == StreamState::Held {
        return Ok((stream_status(runner.state), None));
    }
    if !now.is_finite() || now < 0.0 || now > f64::from(i32::MAX) {
        return Err(ClientError::BadMedia(
            "CIN clock requires nonnegative signed milliseconds".to_string(),
        ));
    }
    let now = now.trunc();
    let decoded = runner.decoded;
    if focus != PlaybackFocus::Game {
        runner.epoch = now - (decoded as f64 * 1000.0 / CIN_FRAME_RATE as f64).trunc();
        return Ok((stream_status(runner.state), None));
    }
    let frame = ((now - runner.epoch) * f64::from(CIN_FRAME_RATE) / 1000.0).trunc() as i64;
    if frame <= decoded {
        return Ok((stream_status(runner.state), None));
    }
    if frame > decoded + 1 {
        host.developer_print(&format!("Dropped frame: {frame} > {}\n", decoded + 1));
        runner.epoch = now - (decoded as f64 * 1000.0 / CIN_FRAME_RATE as f64).trunc();
    }
    let previous = runner.picture.clone();
    runner.picture.clone_from(&runner.pending);
    runner.pending = runner.read();
    if runner.pending.is_none() {
        if runner.hold {
            if runner.picture.is_none() {
                runner.picture = previous;
            }
            runner.state = StreamState::Held;
        } else if runner.loop_playback {
            runner.pass += 1;
            runner.decoded = 0;
            runner.epoch = now;
            runner.pending = None;
            runner.picture = runner.read();
            if runner.picture.is_none() {
                runner.state = StreamState::Ended;
            } else {
                runner.state = StreamState::Looped;
            }
        } else {
            runner.picture = None;
            runner.state = StreamState::Ended;
        }
    } else {
        runner.state = StreamState::Playing;
    }
    let status = stream_status(runner.state);
    Ok((status, Some(runner.picture.clone())))
}

fn stream_status(state: StreamState) -> DecoderStatus {
    match state {
        StreamState::Playing => DecoderStatus::Playing,
        StreamState::Held => DecoderStatus::Held,
        StreamState::Ended => DecoderStatus::Ended,
        StreamState::Looped => DecoderStatus::Looped,
    }
}

/// Unsigned 32-bit wrap (`>>> 0`).
fn uint32(value: f64) -> f64 {
    value.rem_euclid(4294967296.0)
}

fn run_roq(
    runner: &mut StreamRunner,
    now: f64,
) -> Result<(DecoderStatus, Option<Option<CinematicFrame>>), ClientError> {
    if runner.state == StreamState::Held || runner.state == StreamState::Ended {
        return Ok((stream_status(runner.state), None));
    }
    let clock_time = now as f32;
    if !now.is_finite() || clock_time > 0x7fff_ffff as f32 {
        return Err(ClientError::BadMedia(
            "Cinematic clock must fit finite nonnegative signed milliseconds".to_string(),
        ));
    }
    // CIN_RunCinematic uses 30 fps regardless of the RoQ header's rate.
    let target_frame = |epoch: f64| ((now - epoch) as f32 * 3.0 / 100.0).trunc() as i64;
    let this_time = clock_time.trunc() as i64;
    let gap = this_time.wrapping_sub(runner.last_time as i64) as i32;
    if runner.shader && gap.abs() > 100 {
        runner.epoch = uint32(runner.epoch + f64::from(gap));
    }
    let mut target = target_frame(runner.epoch);
    let mut dirty: Option<CinematicFrame> = None;
    while runner.state == StreamState::Playing && target != runner.decoded {
        runner.decoded += 1;
        let index = runner.decoded.max(0) as usize;
        if index >= runner.timeline.frames.len() {
            if runner.hold {
                runner.state = StreamState::Held;
            } else if runner.loop_playback {
                runner.epoch = now;
                runner.last_time = now;
                runner.state = StreamState::Looped;
                runner.decoded = -1;
                runner.pass += 1;
            } else {
                runner.state = StreamState::Ended;
            }
            break;
        }
        let frame = runner.frame(index, runner.epoch + runner.decoded as f64 * 1000.0 / 30.0);
        runner.picture.clone_from(&frame);
        dirty.clone_from(&frame);
        target = target_frame(runner.epoch);
    }
    runner.last_time = this_time as f64;
    if runner.state == StreamState::Looped {
        runner.state = StreamState::Playing;
    }
    if runner.state == StreamState::Ended && runner.loop_playback {
        runner.epoch = now;
        runner.last_time = now;
        runner.state = StreamState::Looped;
        runner.decoded = -1;
        runner.pass += 1;
    }
    let status = stream_status(runner.state);
    Ok((status, dirty.map(Some)))
}

fn run_ogv(
    runner: &mut StreamRunner,
    now: f64,
) -> Result<(DecoderStatus, Option<Option<CinematicFrame>>), ClientError> {
    if runner.state != StreamState::Playing {
        return Ok((stream_status(runner.state), None));
    }
    if !now.is_finite() || now < runner.epoch {
        return Err(ClientError::BadMedia(
            "OGV clock must be finite and monotonic".to_string(),
        ));
    }
    let count = runner.timeline.frames.len();
    if count == 0 {
        runner.state = StreamState::Ended;
        return Ok((DecoderStatus::Ended, None));
    }
    let frame_ms = runner
        .timeline
        .frames
        .get(1)
        .map_or_else(|| runner.source_time(0).max(1000.0 / 30.0), |frame| frame.source_time);
    let frame_ms = if frame_ms > 0.0 { frame_ms } else { 1000.0 / 30.0 };
    let duration = count as f64 * frame_ms;
    let mut elapsed = now - runner.epoch;
    let mut looped = false;
    if elapsed >= duration {
        if runner.hold {
            runner.next_index = count;
            runner.picture = runner.frame(count - 1, now);
            runner.state = StreamState::Held;
            return Ok((DecoderStatus::Held, Some(runner.picture.clone())));
        }
        if !runner.loop_playback {
            runner.state = StreamState::Ended;
            return Ok((DecoderStatus::Ended, None));
        }
        let passes = (elapsed / duration).floor() as usize;
        runner.pass += passes;
        runner.epoch += passes as f64 * duration;
        elapsed = now - runner.epoch;
        runner.next_index = 0;
        looped = true;
    }
    let target = (count - 1).min((elapsed / frame_ms).floor() as usize);
    let mut changed = false;
    while runner.next_index <= target {
        let index = runner.next_index;
        runner.picture = runner.frame(index, now);
        runner.next_index += 1;
        changed = true;
    }
    let status = if looped {
        runner.state = StreamState::Playing;
        DecoderStatus::Looped
    } else {
        DecoderStatus::Playing
    };
    Ok((status, changed.then(|| runner.picture.clone())))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::media::types::{CinematicAudio, CinematicTarget};

    struct NullHost;

    impl CinematicHost for NullHost {
        fn on_audio(&mut self, _audio: &CinematicAudio, _target: &CinematicTarget) {}
        fn on_audio_reset(&mut self, _target: &CinematicTarget) {}
        fn on_audio_pause(&mut self, _paused: bool, _target: &CinematicTarget) {}
        fn on_complete(&mut self, _reason: CinematicEndReason, _target: &CinematicTarget) {}
    }

    fn cin_bytes(frames: usize) -> Vec<u8> {
        let mut bytes = vec![0u8; 20 + 65536];
        bytes[0..4].copy_from_slice(&64i32.to_le_bytes());
        bytes[4..8].copy_from_slice(&64i32.to_le_bytes());
        for _ in 0..frames {
            // command 0, size 4, 4 payload bytes.
            bytes.extend_from_slice(&0i32.to_le_bytes());
            bytes.extend_from_slice(&4i32.to_le_bytes());
            bytes.extend_from_slice(&[1, 2, 3, 4]);
        }
        bytes.extend_from_slice(&2i32.to_le_bytes());
        bytes
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
        let mut host = NullHost;
        let mut playback = CinematicPlayback::open(&source, target(), 0.0, false, false, true).unwrap();
        assert_eq!(playback.status(), CinematicStatus::Playing);
        assert_eq!(playback.timeline(0.0).unwrap().pass, 0);
        // Donor prefetch order: unchanged at frame 1, blank on first
        // advance, then index 1.
        let tick = playback.tick(142.0, &mut host).unwrap();
        assert!(tick.changed);
        assert_eq!(tick.frame.as_ref().unwrap().index, 0);
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
    }

    #[test]
    fn cin_loops() {
        let source = CinematicSource::Cin {
            source: "test.cin".to_string(),
            bytes: cin_bytes(2),
        };
        let mut host = NullHost;
        let mut playback = CinematicPlayback::open(&source, target(), 0.0, true, false, true).unwrap();
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
    fn pause_freezes_time() {
        let source = CinematicSource::Cin {
            source: "test.cin".to_string(),
            bytes: cin_bytes(3),
        };
        let mut host = NullHost;
        let mut playback = CinematicPlayback::open(&source, target(), 0.0, false, false, true).unwrap();
        playback.pause(true, 10.0, &mut host).unwrap();
        assert_eq!(playback.status(), CinematicStatus::Paused);
        assert_eq!(playback.playback_time(1000.0).unwrap(), 10.0);
        playback.pause(false, 1000.0, &mut host).unwrap();
        assert_eq!(playback.playback_time(1010.0).unwrap(), 20.0);
    }

    #[test]
    fn image_is_held() {
        let source = CinematicSource::Image {
            source: "poster".to_string(),
            width: 2,
            height: 1,
            rgba: vec![0u8; 8],
        };
        let mut host = NullHost;
        let mut playback = CinematicPlayback::open(&source, target(), 0.0, false, false, true).unwrap();
        assert_eq!(playback.status(), CinematicStatus::Held);
        let tick = playback.tick(100.0, &mut host).unwrap();
        assert!(tick.frame.as_ref().unwrap().decoded);
    }

    #[test]
    fn clock_checkpoint_round_trips() {
        let clock = PlaybackClock::new(100.0).unwrap();
        let checkpoint = clock.capture(150.0).unwrap();
        assert_eq!(checkpoint.elapsed, 50.0);
        let restored = PlaybackClock::restore(1000.0, &checkpoint).unwrap();
        assert_eq!(restored.sample(1100.0).unwrap(), 150.0);
    }
}
