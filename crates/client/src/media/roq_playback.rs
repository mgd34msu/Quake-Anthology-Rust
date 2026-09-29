//! RoQ playback: frame scheduling over a chunk decoder.
//!
//! Donor provenance: `src/media/roq-playback.ts` (`RoqPlayback`,
//! `CIN_RunCinematic` and `RoQInterrupt` from id Software
//! `code/client/cl_cin.c`, Copyright (C) 1999-2005 Id Software, Inc.
//! GPL-2.0-or-later).
//!
//! The donor fires info/audio hooks inside chunk dispatch; this port
//! runs the same hook bodies after dispatch returns, when the decoded
//! event is known. Hook bodies only touch playback counters, so the
//! observable order is identical.

use super::containers::RoqEndPolicy;
use super::roq::{
    RoqAudioEvent, RoqChunkEvent, RoqChunkHooks, RoqDecoder, RoqDecoderCheckpoint, RoqDecoderOptions, RoqDecoderScratch,
};
use super::roq_stream::RoqStream;
use super::types::{AudioSamples, CinematicAudio, CinematicAudioSink};
use crate::ClientError;

/// A decoded playback frame (`RoqPlaybackFrame`).
#[derive(Debug, Clone, PartialEq)]
pub struct RoqPlaybackFrame {
    /// RGBA pixels.
    pub rgba: Vec<u8>,
    /// Width.
    pub width: usize,
    /// Height.
    pub height: usize,
    /// Frame index.
    pub index: i32,
    /// Loop count.
    pub pass: usize,
    /// Presentation deadline on the caller's clock; frame zero is due
    /// after one interval.
    pub time: f64,
    /// Source time in milliseconds.
    pub source_time: f64,
}

/// Playback audio (`RoqPlaybackAudio`).
#[derive(Debug, Clone, PartialEq)]
pub struct RoqPlaybackAudio {
    /// Samples.
    pub samples: Vec<i16>,
    /// Channels.
    pub channels: u8,
    /// Sample rate.
    pub sample_rate: u32,
    /// Loop count.
    pub pass: usize,
    /// Sample-frame offset, independent of interleaved channels.
    pub source_sample: usize,
    /// Source time in milliseconds.
    pub source_time: f64,
    /// Presentation time in milliseconds.
    pub time: f64,
    /// The raw stream resets before the first stereo audio of a pass.
    pub reset_stream: bool,
}

/// A physical frame pointer (`RoqFramePointer`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RoqFramePointer {
    /// Byte offset.
    pub offset: usize,
    /// Frame byte length.
    pub byte_length: usize,
}

/// Frame callback (`onFrame`).
pub type RoqFrameCallback = Box<dyn FnMut(&RoqPlaybackFrame, &RoqFramePointer)>;

/// Playback status.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RoqPlaybackStatus {
    /// Playing.
    Playing,
    /// Held.
    Held,
    /// Ended.
    Ended,
    /// Looped (transient).
    Looped,
}

/// A playback frame update.
#[derive(Debug, Clone, PartialEq)]
pub enum RoqPlaybackUpdate {
    /// No new frame.
    Unchanged,
    /// New frame.
    Frame(RoqPlaybackFrame),
}

/// A playback tick (`RoqPlaybackTick`).
#[derive(Debug, Clone, PartialEq)]
pub struct RoqPlaybackTick {
    /// Status.
    pub status: RoqPlaybackStatus,
    /// Update.
    pub update: RoqPlaybackUpdate,
}

/// Playback options (`RoqPlaybackOptions`).
///
/// The donor's `beforeRawStreamReset` hook always reset the mixer's
/// raw lane; here the audio sink owns that lane, so playback calls
/// [`CinematicAudioSink::on_audio_reset`] directly at the same point.
#[derive(Default)]
pub struct RoqPlaybackOptions {
    /// Loop.
    pub loop_playback: bool,
    /// Hold the last frame.
    pub hold: bool,
    /// Silent.
    pub silent: bool,
    /// Shader timing (epoch follows large gaps).
    pub shader: bool,
    /// Shared scratch.
    pub scratch: Option<RoqDecoderScratch>,
    /// Info callback.
    pub on_info: Option<Box<dyn FnMut(usize, usize)>>,
    /// Frame callback.
    pub on_frame: Option<RoqFrameCallback>,
}

fn clock_time(time: f64) -> Result<f64, ClientError> {
    if !time.is_finite() || time < 0.0 || (time as f32) > 0x7fff_ffff as f32 {
        return Err(ClientError::BadMedia(
            "Cinematic clock must fit finite nonnegative signed milliseconds".to_string(),
        ));
    }
    Ok(time)
}

/// RoQ playback (`RoqPlayback`).
pub struct RoqPlayback {
    decoder: RoqDecoder,
    bytes: Option<Vec<u8>>,
    source: String,
    loop_playback: bool,
    hold: bool,
    silent: bool,
    shader: bool,
    on_info: Option<Box<dyn FnMut(usize, usize)>>,
    on_frame: Option<RoqFrameCallback>,
    epoch: u32,
    last_time: i32,
    state: RoqPlaybackStatus,
    frame: Option<RoqPlaybackFrame>,
    pointer: Option<RoqFramePointer>,
    decoded_frames: i32,
    source_sample: usize,
    loop_index: usize,
}

impl RoqPlayback {
    fn bind(
        decoder: RoqDecoder,
        bytes: Option<Vec<u8>>,
        source: &str,
        options: RoqPlaybackOptions,
        clock_sample: f64,
    ) -> Result<Self, ClientError> {
        let epoch = clock_time(clock_sample)?.trunc() as u32;
        Ok(Self {
            decoder,
            bytes,
            source: source.to_string(),
            loop_playback: options.loop_playback,
            hold: options.hold,
            silent: options.silent,
            shader: options.shader,
            on_info: options.on_info,
            on_frame: options.on_frame,
            epoch,
            last_time: epoch as i32,
            state: RoqPlaybackStatus::Playing,
            frame: None,
            pointer: None,
            decoded_frames: -1,
            source_sample: 0,
            loop_index: 0,
        })
    }

    /// Play back bytes.
    pub fn from_bytes(
        bytes: Vec<u8>,
        source: &str,
        options: RoqPlaybackOptions,
        clock_sample: f64,
    ) -> Result<Self, ClientError> {
        let RoqPlaybackOptions {
            loop_playback,
            hold,
            silent,
            shader,
            scratch,
            on_info,
            on_frame,
        } = options;
        let decoder = RoqDecoder::from_bytes(
            bytes.clone(),
            source,
            RoqDecoderOptions {
                end_policy: RoqEndPolicy::CinematicLookahead,
                silent,
                scratch,
            },
        )?;
        Self::bind(
            decoder,
            Some(bytes),
            source,
            RoqPlaybackOptions {
                loop_playback,
                hold,
                silent,
                shader,
                scratch: None,
                on_info,
                on_frame,
            },
            clock_sample,
        )
    }

    /// Play back a stream.
    pub fn from_stream(
        stream: RoqStream,
        source: &str,
        options: RoqPlaybackOptions,
        clock_sample: f64,
    ) -> Result<Self, ClientError> {
        let RoqPlaybackOptions {
            loop_playback,
            hold,
            silent,
            shader,
            scratch,
            on_info,
            on_frame,
        } = options;
        let decoder = RoqDecoder::from_stream(
            stream,
            source,
            RoqDecoderOptions {
                end_policy: RoqEndPolicy::CinematicLookahead,
                silent,
                scratch,
            },
        )?;
        Self::bind(
            decoder,
            None,
            source,
            RoqPlaybackOptions {
                loop_playback,
                hold,
                silent,
                shader,
                scratch: None,
                on_info,
                on_frame,
            },
            clock_sample,
        )
    }

    /// `RoQReset` can initialize a retained file before playback has
    /// built its decoder (`fromReset`).
    pub fn from_reset(
        mut stream: RoqStream,
        source: &str,
        options: RoqPlaybackOptions,
        clock_sample: f64,
    ) -> Result<Self, ClientError> {
        stream.rewind()?;
        let mut playback = Self::from_stream(stream, source, options, clock_sample)?;
        playback.state = RoqPlaybackStatus::Looped;
        Ok(playback)
    }

    /// Capture a checkpoint.
    pub fn capture_checkpoint(&self) -> Result<RoqPlaybackCheckpoint, ClientError> {
        Ok(RoqPlaybackCheckpoint {
            decoder: self.decoder.capture_checkpoint()?,
            epoch: self.epoch,
            last_time: self.last_time,
            state: self.state,
            frame: self.frame.clone(),
            pointer: self.pointer,
            decoded_frames: self.decoded_frames,
            source_sample: self.source_sample,
            loop_index: self.loop_index,
        })
    }

    /// Restore a checkpoint.
    pub fn restore_checkpoint(&mut self, checkpoint: &RoqPlaybackCheckpoint) -> Result<(), ClientError> {
        self.decoder.restore_checkpoint(&checkpoint.decoder)?;
        if let Some(frame) = &checkpoint.frame {
            if frame.rgba.len() != frame.width * frame.height * 4 {
                return Err(ClientError::BadMedia("frame dimensions differ from pixels".to_string()));
            }
        }
        self.epoch = checkpoint.epoch;
        self.last_time = checkpoint.last_time;
        self.state = checkpoint.state;
        self.frame = checkpoint.frame.clone();
        self.pointer = checkpoint.pointer;
        self.decoded_frames = checkpoint.decoded_frames;
        self.source_sample = checkpoint.source_sample;
        self.loop_index = checkpoint.loop_index;
        Ok(())
    }

    /// Frame rate.
    #[must_use]
    pub fn frame_rate(&self) -> u32 {
        self.decoder.frame_rate()
    }

    /// Current frame.
    #[must_use]
    pub fn current_frame(&self) -> Option<RoqPlaybackFrame> {
        self.frame.clone()
    }

    /// Current pointer.
    #[must_use]
    pub const fn current_pointer(&self) -> Option<RoqFramePointer> {
        self.pointer
    }

    /// Dimensions.
    #[must_use]
    pub fn dimensions(&self) -> Option<(usize, usize)> {
        if self.decoder.width() == 0 {
            None
        } else {
            Some((self.decoder.width(), self.decoder.height()))
        }
    }

    /// Decoder scratch (presentation reads the live buffer).
    #[must_use]
    pub const fn scratch(&self) -> &RoqDecoderScratch {
        self.decoder.scratch()
    }

    /// Close the backing stream (`releaseInput`).
    pub fn close_input(&mut self) {
        self.decoder.close_stream();
    }

    /// Rewind the decoder and clear the published frame (`reset`).
    pub fn reset(&mut self, clock_sample: f64) -> Result<(), ClientError> {
        self.decoder.scratch_mut().clear();
        self.decoder.clear_stream_buffer();
        if let Some(bytes) = self.bytes.clone() {
            let scratch = std::mem::take(self.decoder.scratch_mut());
            let source = self.source.clone();
            self.decoder = RoqDecoder::from_bytes(
                bytes,
                &source,
                RoqDecoderOptions {
                    end_policy: RoqEndPolicy::CinematicLookahead,
                    silent: self.silent,
                    scratch: Some(scratch),
                },
            )?;
        } else {
            self.decoder.rebind()?;
        }
        self.frame = None;
        self.pointer = None;
        self.restart(clock_sample)?;
        self.state = RoqPlaybackStatus::Playing;
        Ok(())
    }

    /// Retain physical buffers, codebooks and the last published
    /// image (`restart`).
    pub fn restart(&mut self, clock_sample: f64) -> Result<(), ClientError> {
        self.decoder.rewind()?;
        self.epoch = clock_time(clock_sample)?.trunc() as u32;
        self.last_time = self.epoch as i32;
        self.state = RoqPlaybackStatus::Looped;
        self.decoded_frames = -1;
        self.source_sample = 0;
        self.loop_index = 0;
        Ok(())
    }

    fn reset_loop(&mut self, clock_sample: f64) -> Result<(), ClientError> {
        self.decoder.rewind()?;
        self.epoch = clock_time(clock_sample)?.trunc() as u32;
        self.last_time = self.epoch as i32;
        self.state = RoqPlaybackStatus::Looped;
        self.decoded_frames = -1;
        self.source_sample = 0;
        self.loop_index += 1;
        Ok(())
    }

    /// Run playback (`run`).
    pub fn run(
        &mut self,
        clock_sample: f64,
        sink: &mut dyn CinematicAudioSink,
    ) -> Result<RoqPlaybackTick, ClientError> {
        if self.state == RoqPlaybackStatus::Held || self.state == RoqPlaybackStatus::Ended {
            return Ok(RoqPlaybackTick {
                status: self.state,
                update: RoqPlaybackUpdate::Unchanged,
            });
        }
        clock_time(clock_sample)?;
        // `CIN_RunCinematic` uses 30 fps regardless of the header rate.
        let target_frames = |epoch: u32| (((clock_sample - f64::from(epoch)) * 3.0 / 100.0) as f32).trunc() as i64;
        // The x87 profile spills this signed conversion to float32.
        let this_time = (clock_sample as f32).trunc() as i64;
        let gap = this_time.wrapping_sub(i64::from(self.last_time)) as i32;
        if self.shader && (i64::from(gap)).abs() > 100 {
            self.epoch = self.epoch.wrapping_add(gap as u32);
        }
        let mut target = target_frames(self.epoch);
        let mut epoch_snapshot = self.epoch;
        let mut dirty: Option<RoqPlaybackFrame> = None;
        while self.state == RoqPlaybackStatus::Playing
            && (target != i64::from(self.decoded_frames)
                || self.decoder.in_packet()
                || self.decoder.has_invalid_lookahead())
        {
            let had_invalid = self.decoder.has_invalid_lookahead();
            let event = self.decoder.next_chunk(RoqChunkHooks::default())?;
            match &event {
                RoqChunkEvent::Info { width, height } => {
                    if self.decoded_frames == -1 {
                        if let Some(on_info) = self.on_info.as_deref_mut() {
                            on_info(*width, *height);
                        }
                        self.epoch = clock_time(clock_sample)?.trunc() as u32;
                        self.last_time = self.epoch as i32;
                    }
                    if self.decoded_frames != 1 {
                        self.decoded_frames = 0;
                    }
                }
                RoqChunkEvent::Audio(audio) => {
                    let reset_stream = audio.channels == 2 && self.decoded_frames == -1;
                    if reset_stream {
                        sink.on_audio_reset();
                    }
                    self.emit_audio(audio, reset_stream, sink);
                }
                RoqChunkEvent::Frame(frame) => {
                    self.decoded_frames += 1;
                    let picture = RoqPlaybackFrame {
                        rgba: frame.rgba.clone(),
                        width: self.decoder.width(),
                        height: self.decoder.height(),
                        index: frame.index,
                        pass: self.loop_index,
                        source_time: frame.time,
                        time: f64::from(self.epoch) + f64::from(self.decoded_frames) * 1000.0 / 30.0,
                    };
                    let byte_length = self.decoder.width() * self.decoder.height() * 4;
                    let pointer = RoqFramePointer {
                        offset: byte_length * (frame.index & 1) as usize,
                        byte_length,
                    };
                    self.pointer = Some(pointer);
                    if let Some(on_frame) = self.on_frame.as_deref_mut() {
                        on_frame(&picture, &pointer);
                    }
                    self.frame = Some(picture.clone());
                    dirty = Some(picture);
                }
                RoqChunkEvent::End | RoqChunkEvent::Metadata => {}
            }
            if !had_invalid && self.decoder.has_invalid_lookahead() && !self.decoder.reset_after_run() {
                sink.developer_print("roq_size>65536||roq_id==0x1084\n");
            }
            if matches!(event, RoqChunkEvent::End) {
                if self.hold && !self.decoder.has_invalid_lookahead() {
                    self.state = RoqPlaybackStatus::Held;
                } else if self.loop_playback && !self.decoder.reset_after_run() {
                    // `RoQReset` rebases `startTime` to the clock that
                    // observed EOF, even after a long stall.
                    self.reset_loop(clock_sample)?;
                } else {
                    self.state = RoqPlaybackStatus::Ended;
                }
            }
            // `RoQInterrupt` processes an entire packet before
            // `CIN_RunCinematic` checks `startTime`.
            if !self.decoder.in_packet() && epoch_snapshot != self.epoch {
                target = target_frames(self.epoch);
                epoch_snapshot = self.epoch;
            }
        }
        self.last_time = this_time as i32;
        if self.state == RoqPlaybackStatus::Looped {
            self.state = RoqPlaybackStatus::Playing;
        }
        if self.state == RoqPlaybackStatus::Ended && self.loop_playback {
            self.reset_loop(clock_sample)?;
        }
        Ok(RoqPlaybackTick {
            status: self.state,
            update: dirty
                .map(RoqPlaybackUpdate::Frame)
                .unwrap_or(RoqPlaybackUpdate::Unchanged),
        })
    }

    fn emit_audio(&mut self, audio: &RoqAudioEvent, reset_stream: bool, sink: &mut dyn CinematicAudioSink) {
        let source_time = self.source_sample as f64 * 1000.0 / f64::from(audio.sample_rate);
        sink.on_audio(&CinematicAudio {
            samples: AudioSamples::I16(audio.samples.clone()),
            channels: audio.channels,
            sample_rate: audio.sample_rate,
            source_sample: self.source_sample,
            source_time,
            time: f64::from(self.epoch) + source_time,
            pass: self.loop_index,
            reset_stream,
        });
        self.source_sample += audio.samples.len() / usize::from(audio.channels);
    }
}

/// A RoQ playback checkpoint.
#[derive(Debug, Clone, PartialEq)]
pub struct RoqPlaybackCheckpoint {
    /// Decoder.
    pub decoder: RoqDecoderCheckpoint,
    /// Epoch.
    pub epoch: u32,
    /// Last time.
    pub last_time: i32,
    /// State.
    pub state: RoqPlaybackStatus,
    /// Frame.
    pub frame: Option<RoqPlaybackFrame>,
    /// Pointer.
    pub pointer: Option<RoqFramePointer>,
    /// Decoded frames.
    pub decoded_frames: i32,
    /// Source sample.
    pub source_sample: usize,
    /// Loop index.
    pub loop_index: usize,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::media::types::CinematicAudio;

    struct Sink {
        audio: Vec<CinematicAudio>,
        prints: Vec<String>,
        resets: usize,
    }

    impl CinematicAudioSink for Sink {
        fn on_audio(&mut self, audio: &CinematicAudio) {
            self.audio.push(audio.clone());
        }

        fn on_audio_reset(&mut self) {
            self.resets += 1;
        }

        fn developer_print(&mut self, message: &str) {
            self.prints.push(message.to_string());
        }
    }

    fn chunk(id: u16, size: usize, flags: u16, payload: &[u8]) -> Vec<u8> {
        let mut out = vec![0u8; 8 + payload.len()];
        out[..2].copy_from_slice(&id.to_le_bytes());
        out[2..6].copy_from_slice(&(size as u32).to_le_bytes());
        out[6..8].copy_from_slice(&flags.to_le_bytes());
        out[8..].copy_from_slice(payload);
        out
    }

    fn movie() -> Vec<u8> {
        [
            vec![0x84, 0x10, 0, 0, 0, 0, 30, 0],
            chunk(0x1021, 4, 0x1234, &[1, 2, 3, 4]),
            chunk(0x1001, 8, 0, &[8, 0, 8, 0, 0, 0, 0, 0]),
            chunk(0x1002, 10, 0x0101, &[16, 32, 48, 64, 100, 200, 0, 0, 0, 0]),
            chunk(0x1011, 3, 0, &[0x00, 0x80, 0x00]),
            chunk(0x1011, 3, 0, &[0x00, 0x40, 0x88]),
        ]
        .concat()
    }

    fn run(playback: &mut RoqPlayback, sink: &mut Sink, time: f64) -> RoqPlaybackTick {
        playback.run(time, sink).unwrap()
    }

    #[test]
    fn timeline_matches_donor() {
        // Oracle ticks from the donor under bun.
        let mut sink = Sink {
            audio: Vec::new(),
            prints: Vec::new(),
            resets: 0,
        };
        let mut playback = RoqPlayback::from_bytes(movie(), "<test>", RoqPlaybackOptions::default(), 0.0).unwrap();
        assert_eq!(playback.frame_rate(), 30);
        let tick = run(&mut playback, &mut sink, 0.0);
        assert_eq!(tick.status, RoqPlaybackStatus::Playing);
        assert_eq!(tick.update, RoqPlaybackUpdate::Unchanged);
        let tick = run(&mut playback, &mut sink, 34.0);
        assert_eq!(tick.status, RoqPlaybackStatus::Playing);
        match tick.update {
            RoqPlaybackUpdate::Frame(frame) => {
                assert_eq!(frame.index, 0);
                assert_eq!(frame.pass, 0);
                assert_eq!(frame.time, 1000.0 / 30.0);
                assert_eq!(frame.source_time, 0.0);
                assert_eq!(frame.rgba.len(), 256);
            }
            RoqPlaybackUpdate::Unchanged => panic!("expected frame"),
        }
        assert_eq!(
            playback.current_pointer(),
            Some(RoqFramePointer {
                offset: 0,
                byte_length: 256
            })
        );
        // The lookahead names EOF before the trailing frame, so the
        // movie ends after one frame (donor behavior).
        let tick = run(&mut playback, &mut sink, 67.0);
        assert_eq!(tick.status, RoqPlaybackStatus::Ended);
        assert_eq!(tick.update, RoqPlaybackUpdate::Unchanged);
        assert_eq!(sink.audio.len(), 1);
        assert!(sink.audio[0].reset_stream);
        assert_eq!(sink.resets, 1);
        assert_eq!(sink.audio[0].source_sample, 0);
        assert!(sink.prints.is_empty());
    }

    #[test]
    fn loop_replays_with_pass_counter() {
        let mut sink = Sink {
            audio: Vec::new(),
            prints: Vec::new(),
            resets: 0,
        };
        let mut playback = RoqPlayback::from_bytes(
            movie(),
            "<test>",
            RoqPlaybackOptions {
                loop_playback: true,
                ..RoqPlaybackOptions::default()
            },
            0.0,
        )
        .unwrap();
        run(&mut playback, &mut sink, 0.0);
        run(&mut playback, &mut sink, 34.0);
        let tick = run(&mut playback, &mut sink, 67.0);
        assert_eq!(tick.status, RoqPlaybackStatus::Playing);
        let tick = run(&mut playback, &mut sink, 100.0);
        assert_eq!(tick.update, RoqPlaybackUpdate::Unchanged);
        let tick = run(&mut playback, &mut sink, 134.0);
        match tick.update {
            RoqPlaybackUpdate::Frame(frame) => {
                assert_eq!(frame.index, 0);
                assert_eq!(frame.pass, 1);
            }
            RoqPlaybackUpdate::Unchanged => panic!("expected looped frame"),
        }
    }

    #[test]
    fn hold_keeps_last_frame_and_shader_follows_gaps() {
        let mut sink = Sink {
            audio: Vec::new(),
            prints: Vec::new(),
            resets: 0,
        };
        let mut playback = RoqPlayback::from_bytes(
            movie(),
            "<test>",
            RoqPlaybackOptions {
                hold: true,
                ..RoqPlaybackOptions::default()
            },
            0.0,
        )
        .unwrap();
        run(&mut playback, &mut sink, 0.0);
        run(&mut playback, &mut sink, 34.0);
        let tick = run(&mut playback, &mut sink, 67.0);
        assert_eq!(tick.status, RoqPlaybackStatus::Held);
        assert!(playback.current_frame().is_some());

        let mut playback = RoqPlayback::from_bytes(
            movie(),
            "<test>",
            RoqPlaybackOptions {
                shader: true,
                ..RoqPlaybackOptions::default()
            },
            0.0,
        )
        .unwrap();
        run(&mut playback, &mut sink, 0.0);
        // A large wall gap rebases the shader epoch instead of
        // fast-forwarding through the movie.
        let tick = run(&mut playback, &mut sink, 5000.0);
        assert_eq!(tick.status, RoqPlaybackStatus::Playing);
        assert_eq!(tick.update, RoqPlaybackUpdate::Unchanged);
    }

    #[test]
    fn reset_restart_and_checkpoint() {
        let mut sink = Sink {
            audio: Vec::new(),
            prints: Vec::new(),
            resets: 0,
        };
        let mut playback = RoqPlayback::from_bytes(movie(), "<test>", RoqPlaybackOptions::default(), 0.0).unwrap();
        run(&mut playback, &mut sink, 0.0);
        run(&mut playback, &mut sink, 34.0);
        let checkpoint = playback.capture_checkpoint().unwrap();
        let mut revived = RoqPlayback::from_bytes(movie(), "<test>", RoqPlaybackOptions::default(), 0.0).unwrap();
        revived.restore_checkpoint(&checkpoint).unwrap();
        assert_eq!(revived.current_frame(), playback.current_frame());
        assert_eq!(revived.current_pointer(), playback.current_pointer());
        let (a, b) = (run(&mut playback, &mut sink, 67.0), run(&mut revived, &mut sink, 67.0));
        assert_eq!(a, b);

        revived.reset(1000.0).unwrap();
        assert!(revived.current_frame().is_none());
        let tick = run(&mut revived, &mut sink, 1000.0);
        assert_eq!(tick.status, RoqPlaybackStatus::Playing);

        let mut restarted = RoqPlayback::from_bytes(movie(), "<test>", RoqPlaybackOptions::default(), 0.0).unwrap();
        restarted.restart(500.0).unwrap();
        let tick = run(&mut restarted, &mut sink, 500.0);
        assert_eq!(tick.status, RoqPlaybackStatus::Playing);
        assert_eq!(tick.update, RoqPlaybackUpdate::Unchanged);
    }

    #[test]
    fn invalid_lookahead_prints_once() {
        let bytes = [
            vec![0x84, 0x10, 0, 0, 0, 0, 30, 0],
            chunk(0x1001, 8, 0, &[8, 0, 8, 0, 0, 0, 0, 0]),
            chunk(0x1002, 10, 0x0101, &[16, 32, 48, 64, 100, 200, 0, 0, 0, 0]),
            chunk(0x1011, 3, 0, &[0x00, 0x80, 0x00]),
            chunk(0x1011, 70000, 0, &[]),
        ]
        .concat();
        let mut sink = Sink {
            audio: Vec::new(),
            prints: Vec::new(),
            resets: 0,
        };
        let mut playback = RoqPlayback::from_bytes(bytes, "<test>", RoqPlaybackOptions::default(), 0.0).unwrap();
        run(&mut playback, &mut sink, 0.0);
        let tick = run(&mut playback, &mut sink, 34.0);
        assert_eq!(tick.status, RoqPlaybackStatus::Ended);
        assert!(matches!(tick.update, RoqPlaybackUpdate::Frame(_)));
        assert_eq!(sink.prints, vec!["roq_size>65536||roq_id==0x1084\n"]);
    }

    #[test]
    fn stream_backing_and_silent() {
        use super::super::roq_stream::RoqStream;
        let mut sink = Sink {
            audio: Vec::new(),
            prints: Vec::new(),
            resets: 0,
        };
        let stream = RoqStream::from_bytes(
            movie(),
            "<test>",
            super::super::containers::RoqEndPolicy::CinematicLookahead,
        );
        let mut playback = RoqPlayback::from_stream(stream, "<test>", RoqPlaybackOptions::default(), 0.0).unwrap();
        assert!(playback.dimensions().is_none());
        run(&mut playback, &mut sink, 0.0);
        assert_eq!(playback.dimensions(), Some((8, 8)));

        let mut quiet = Sink {
            audio: Vec::new(),
            prints: Vec::new(),
            resets: 0,
        };
        let mut silent = RoqPlayback::from_bytes(
            movie(),
            "<test>",
            RoqPlaybackOptions {
                silent: true,
                ..RoqPlaybackOptions::default()
            },
            0.0,
        )
        .unwrap();
        run(&mut silent, &mut quiet, 0.0);
        run(&mut silent, &mut quiet, 34.0);
        assert!(quiet.audio.is_empty());
    }
}
