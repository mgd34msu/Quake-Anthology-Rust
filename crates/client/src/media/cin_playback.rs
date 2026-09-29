//! CIN playback: prefetch scheduling over a Huffman decoder.
//!
//! Donor provenance: `src/media/cin-playback.ts` (`CinPlayback`,
//! Quake II `SCR_PlayCinematic`/`SCR_RunCinematic`/`SCR_ReadNextFrame`,
//! Copyright (C) 1997-2001 Id Software, Inc. GPL-2.0-or-later).
//!
//! The donor emits prefetch audio synchronously during construction;
//! this port buffers constructor audio and flushes it on the first
//! tick, since construction takes no host. Chunk content and order
//! are identical.

use super::cin::{CinDecoder, CinFrame, CinNext};
use super::containers::CIN_FRAME_RATE;
use super::playback::PlaybackFocus;
use super::types::{CinematicAudio, CinematicAudioSink, CinematicFrame};
use crate::ClientError;

/// Playback options (`CinPlaybackOptions`, without host wiring).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CinPlaybackOptions {
    /// Loop.
    pub loop_playback: bool,
    /// Hold the last frame.
    pub hold: bool,
    /// Silent.
    pub silent: bool,
}

/// Playback status.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CinPlaybackStatus {
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
pub enum CinPlaybackUpdate {
    /// No new frame.
    Unchanged,
    /// New frame (blank while prefetching).
    Frame(Option<CinematicFrame>),
}

/// A playback tick (`CinPlaybackTick`).
#[derive(Debug, Clone, PartialEq)]
pub struct CinPlaybackTick {
    /// Status.
    pub status: CinPlaybackStatus,
    /// Update.
    pub update: CinPlaybackUpdate,
}

fn milliseconds(clock_sample: f64) -> Result<i64, ClientError> {
    if !clock_sample.is_finite() || clock_sample < 0.0 || clock_sample > f64::from(i32::MAX) {
        return Err(ClientError::BadMedia(
            "CIN clock requires nonnegative signed milliseconds".to_string(),
        ));
    }
    Ok(clock_sample.trunc() as i64)
}

/// CIN playback (`CinPlayback`).
pub struct CinPlayback {
    decoder: CinDecoder,
    epoch: i64,
    picture: Option<CinFrame>,
    pending: Option<CinFrame>,
    state: CinPlaybackStatus,
    pass: usize,
    loop_playback: bool,
    hold: bool,
    silent: bool,
    pending_audio: Vec<CinematicAudio>,
}

impl CinPlayback {
    /// Open playback over bytes.
    pub fn from_bytes(
        bytes: Vec<u8>,
        source: &str,
        options: CinPlaybackOptions,
        clock_sample: f64,
    ) -> Result<Self, ClientError> {
        let decoder = CinDecoder::from_bytes(bytes, source)?;
        let mut playback = Self {
            decoder,
            epoch: milliseconds(clock_sample)?,
            picture: None,
            pending: None,
            state: CinPlaybackStatus::Playing,
            pass: 0,
            loop_playback: options.loop_playback,
            hold: options.hold,
            silent: options.silent,
            pending_audio: Vec::new(),
        };
        playback.picture = playback.read()?;
        if playback.picture.is_none() {
            playback.state = CinPlaybackStatus::Ended;
        }
        Ok(playback)
    }

    /// Capture a checkpoint.
    #[must_use]
    pub fn capture_checkpoint(&self) -> CinPlaybackCheckpoint {
        CinPlaybackCheckpoint {
            decoder: self.decoder.capture_checkpoint(),
            epoch: self.epoch,
            picture: self.picture.clone(),
            pending: self.pending.clone(),
            state: self.state,
            pass: self.pass,
            pending_audio: self.pending_audio.clone(),
        }
    }

    /// Restore a checkpoint.
    pub fn restore_checkpoint(&mut self, checkpoint: &CinPlaybackCheckpoint) -> Result<(), ClientError> {
        self.decoder.restore_checkpoint(&checkpoint.decoder)?;
        let pixels = self.decoder.width() * self.decoder.height();
        for frame in [&checkpoint.picture, &checkpoint.pending].into_iter().flatten() {
            if frame.pixels.len() != pixels || frame.palette.len() != 768 {
                return Err(ClientError::BadMedia("CIN frame size differs".to_string()));
            }
            if let Some(audio) = &frame.audio {
                if audio.samples.len() % usize::from(audio.channels) != 0 {
                    return Err(ClientError::BadMedia("partial PCM frame".to_string()));
                }
            }
        }
        self.epoch = checkpoint.epoch;
        self.picture = checkpoint.picture.clone();
        self.pending = checkpoint.pending.clone();
        self.state = checkpoint.state;
        self.pass = checkpoint.pass;
        self.pending_audio = checkpoint.pending_audio.clone();
        Ok(())
    }

    /// Dimensions.
    #[must_use]
    pub fn dimensions(&self) -> (usize, usize) {
        (self.decoder.width(), self.decoder.height())
    }

    /// Current frame.
    pub fn current_frame(&self) -> Result<Option<CinematicFrame>, ClientError> {
        self.presentation()
    }

    fn presentation(&self) -> Result<Option<CinematicFrame>, ClientError> {
        let Some(picture) = &self.picture else {
            return Ok(None);
        };
        // The global palette updates while prefetching, before the
        // current indices draw.
        Ok(Some(CinematicFrame {
            rgba: CinDecoder::rgba(&picture.pixels, &self.decoder.palette())?,
            width: self.decoder.width(),
            height: self.decoder.height(),
            index: picture.index,
            pass: self.pass,
            source_time: picture.time,
            time: self.epoch as f64 + picture.time,
            decoded: true,
        }))
    }

    fn read(&mut self) -> Result<Option<CinFrame>, ClientError> {
        let frame = match self.decoder.next()? {
            CinNext::End => return Ok(None),
            CinNext::Frame(frame) => frame,
        };
        if let Some(audio) = &frame.audio {
            if !self.silent {
                let source_time = audio.source_sample as f64 * 1000.0 / f64::from(audio.sample_rate);
                self.pending_audio.push(CinematicAudio {
                    samples: audio.samples.clone(),
                    channels: audio.channels,
                    sample_rate: audio.sample_rate,
                    source_sample: audio.source_sample,
                    source_time,
                    time: self.epoch as f64 + source_time,
                    pass: self.pass,
                    reset_stream: audio.source_sample == 0,
                });
            }
        }
        Ok(Some(frame))
    }

    /// Restart the movie (`restart`).
    pub fn restart(&mut self, clock_sample: f64) -> Result<(), ClientError> {
        self.decoder.rewind()?;
        self.epoch = milliseconds(clock_sample)?;
        self.pending = None;
        self.state = CinPlaybackStatus::Playing;
        self.picture = self.read()?;
        if self.picture.is_none() {
            self.state = CinPlaybackStatus::Ended;
        }
        Ok(())
    }

    /// Run playback (`run`).
    pub fn run(
        &mut self,
        clock_sample: f64,
        focus: PlaybackFocus,
        sink: &mut dyn CinematicAudioSink,
    ) -> Result<CinPlaybackTick, ClientError> {
        if self.state == CinPlaybackStatus::Ended || self.state == CinPlaybackStatus::Held {
            return Ok(CinPlaybackTick {
                status: self.state,
                update: CinPlaybackUpdate::Unchanged,
            });
        }
        for audio in self.pending_audio.drain(..) {
            sink.on_audio(&audio);
        }
        let time = milliseconds(clock_sample)?;
        let decoded = self.decoder.next_frame_index() as i64;
        if focus != PlaybackFocus::Game {
            self.epoch = time - (decoded * 1000 / i64::from(CIN_FRAME_RATE));
            return Ok(CinPlaybackTick {
                status: self.state,
                update: CinPlaybackUpdate::Unchanged,
            });
        }
        let frame = (time - self.epoch) * i64::from(CIN_FRAME_RATE) / 1000;
        if frame <= decoded {
            return Ok(CinPlaybackTick {
                status: self.state,
                update: CinPlaybackUpdate::Unchanged,
            });
        }
        if frame > decoded + 1 {
            sink.developer_print(&format!("Dropped frame: {frame} > {}\n", decoded + 1));
            self.epoch = time - (decoded * 1000 / i64::from(CIN_FRAME_RATE));
        }
        let previous = self.picture.take();
        self.picture = self.pending.take();
        self.pending = self.read()?;
        if self.pending.is_none() {
            if self.hold {
                if self.picture.is_none() {
                    self.picture = previous;
                }
                self.state = CinPlaybackStatus::Held;
            } else if self.loop_playback {
                self.pass += 1;
                self.restart(clock_sample)?;
                self.state = CinPlaybackStatus::Looped;
            } else {
                self.picture = None;
                self.state = CinPlaybackStatus::Ended;
            }
        } else {
            self.state = CinPlaybackStatus::Playing;
        }
        Ok(CinPlaybackTick {
            status: self.state,
            update: CinPlaybackUpdate::Frame(self.presentation()?),
        })
    }

    /// Close playback.
    pub fn close(&mut self) {
        self.state = CinPlaybackStatus::Ended;
        self.decoder.close();
    }
}

/// A CIN playback checkpoint.
#[derive(Debug, Clone, PartialEq)]
pub struct CinPlaybackCheckpoint {
    /// Decoder.
    pub decoder: super::cin::CinDecoderCheckpoint,
    /// Epoch.
    pub epoch: i64,
    /// Picture.
    pub picture: Option<CinFrame>,
    /// Pending.
    pub pending: Option<CinFrame>,
    /// State.
    pub state: CinPlaybackStatus,
    /// Pass.
    pub pass: usize,
    /// Buffered prefetch audio.
    pub pending_audio: Vec<CinematicAudio>,
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Sink {
        audio: Vec<CinematicAudio>,
        prints: Vec<String>,
    }

    impl CinematicAudioSink for Sink {
        fn on_audio(&mut self, audio: &CinematicAudio) {
            self.audio.push(audio.clone());
        }

        fn developer_print(&mut self, message: &str) {
            self.prints.push(message.to_string());
        }
    }

    fn movie(frames: usize) -> Vec<u8> {
        let mut header = vec![0u8; 20];
        header[..4].copy_from_slice(&4i32.to_le_bytes());
        header[4..8].copy_from_slice(&4i32.to_le_bytes());
        header[8..12].copy_from_slice(&14i32.to_le_bytes());
        header[12..16].copy_from_slice(&1i32.to_le_bytes());
        header[16..20].copy_from_slice(&1i32.to_le_bytes());
        let mut bytes = [header, vec![0u8; 65536]].concat();
        for frame in 0..frames {
            bytes.extend_from_slice(&0i32.to_le_bytes());
            bytes.extend_from_slice(&4i32.to_le_bytes());
            bytes.extend_from_slice(&16i32.to_le_bytes());
            bytes.push(frame as u8);
        }
        bytes.extend_from_slice(&2i32.to_le_bytes());
        bytes
    }

    fn sink() -> Sink {
        Sink {
            audio: Vec::new(),
            prints: Vec::new(),
        }
    }

    #[test]
    fn prefetch_advance_and_end() {
        let mut sync = sink();
        let mut playback = CinPlayback::from_bytes(movie(3), "<test>", CinPlaybackOptions::default(), 0.0).unwrap();
        assert_eq!(playback.dimensions(), (4, 4));
        let frame = playback.current_frame().unwrap().unwrap();
        assert_eq!(frame.index, 0);
        assert!(frame.decoded);
        // Prefetch audio flushes on the first tick.
        let tick = playback.run(0.0, PlaybackFocus::Game, &mut sync).unwrap();
        assert_eq!(tick.status, CinPlaybackStatus::Playing);
        assert_eq!(tick.update, CinPlaybackUpdate::Unchanged);
        assert_eq!(sync.audio.len(), 1);
        assert!(sync.audio[0].reset_stream);
        // First advance shows a blank prefetch frame.
        let tick = playback.run(143.0, PlaybackFocus::Game, &mut sync).unwrap();
        assert_eq!(tick.update, CinPlaybackUpdate::Frame(None));
        let tick = playback.run(215.0, PlaybackFocus::Game, &mut sync).unwrap();
        match tick.update {
            CinPlaybackUpdate::Frame(Some(frame)) => assert_eq!(frame.index, 1),
            update => panic!("expected frame 1, got {update:?}"),
        }
        let tick = playback.run(300.0, PlaybackFocus::Game, &mut sync).unwrap();
        assert_eq!(tick.status, CinPlaybackStatus::Ended);
        assert_eq!(tick.update, CinPlaybackUpdate::Frame(None));
    }

    #[test]
    fn hold_loop_focus_and_drops() {
        let mut sync = sink();
        let mut playback = CinPlayback::from_bytes(
            movie(1),
            "<test>",
            CinPlaybackOptions {
                hold: true,
                ..CinPlaybackOptions::default()
            },
            0.0,
        )
        .unwrap();
        playback.run(0.0, PlaybackFocus::Game, &mut sync).unwrap();
        let tick = playback.run(143.0, PlaybackFocus::Game, &mut sync).unwrap();
        assert_eq!(tick.status, CinPlaybackStatus::Held);
        match tick.update {
            CinPlaybackUpdate::Frame(Some(frame)) => assert_eq!(frame.index, 0),
            update => panic!("expected held frame, got {update:?}"),
        }

        let mut playback = CinPlayback::from_bytes(
            movie(1),
            "<test>",
            CinPlaybackOptions {
                loop_playback: true,
                ..CinPlaybackOptions::default()
            },
            0.0,
        )
        .unwrap();
        playback.run(0.0, PlaybackFocus::Game, &mut sync).unwrap();
        let tick = playback.run(143.0, PlaybackFocus::Game, &mut sync).unwrap();
        assert_eq!(tick.status, CinPlaybackStatus::Looped);

        // Losing focus rebases the epoch without advancing.
        let mut playback = CinPlayback::from_bytes(movie(2), "<test>", CinPlaybackOptions::default(), 0.0).unwrap();
        playback.run(0.0, PlaybackFocus::Game, &mut sync).unwrap();
        let tick = playback.run(5000.0, PlaybackFocus::Console, &mut sync).unwrap();
        assert_eq!(tick.update, CinPlaybackUpdate::Unchanged);
        let tick = playback.run(5000.0, PlaybackFocus::Game, &mut sync).unwrap();
        assert_eq!(tick.update, CinPlaybackUpdate::Unchanged);

        // Skipping frames prints and rebases.
        let mut playback = CinPlayback::from_bytes(movie(4), "<test>", CinPlaybackOptions::default(), 0.0).unwrap();
        playback.run(0.0, PlaybackFocus::Game, &mut sync).unwrap();
        playback.run(10000.0, PlaybackFocus::Game, &mut sync).unwrap();
        assert_eq!(sync.prints.len(), 1);
    }

    #[test]
    fn checkpoint_and_silent() {
        let mut sync = sink();
        let mut playback = CinPlayback::from_bytes(movie(2), "<test>", CinPlaybackOptions::default(), 0.0).unwrap();
        playback.run(0.0, PlaybackFocus::Game, &mut sync).unwrap();
        let checkpoint = playback.capture_checkpoint();
        let mut revived = CinPlayback::from_bytes(movie(2), "<test>", CinPlaybackOptions::default(), 0.0).unwrap();
        revived.restore_checkpoint(&checkpoint).unwrap();
        assert_eq!(revived.current_frame().unwrap(), playback.current_frame().unwrap());

        let mut quiet = sink();
        let mut playback = CinPlayback::from_bytes(
            movie(2),
            "<test>",
            CinPlaybackOptions {
                silent: true,
                ..CinPlaybackOptions::default()
            },
            0.0,
        )
        .unwrap();
        playback.run(143.0, PlaybackFocus::Game, &mut quiet).unwrap();
        assert!(quiet.audio.is_empty());
    }
}
