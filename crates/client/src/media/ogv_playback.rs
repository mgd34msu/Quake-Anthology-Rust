//! OGV playback: Theora video with a Vorbis soundtrack.
//!
//! Donor provenance: `src/media/ogv-playback.ts` (`OgvPlayback`).

use super::containers::{decode_ogg_movie, OggMovie};
use super::theora::{TheoraDecoder, TheoraPicture};
use super::types::{AudioSamples, CinematicAudio, CinematicAudioSink, CinematicFrame};
use super::vorbis::VorbisPcmStream;
use crate::ClientError;

/// Playback options (`OgvPlaybackOptions`, without host wiring).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct OgvPlaybackOptions {
    /// Loop.
    pub loop_playback: bool,
    /// Hold the last frame.
    pub hold: bool,
    /// Silent.
    pub silent: bool,
}

/// Resting playback state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OgvPlaybackState {
    /// Playing.
    Playing,
    /// Held.
    Held,
    /// Ended.
    Ended,
}

/// A playback tick status.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OgvPlaybackStatus {
    /// Playing.
    Playing,
    /// Looped (transient).
    Looped,
    /// Held.
    Held,
    /// Ended.
    Ended,
}

/// A playback frame update.
#[derive(Debug, Clone, PartialEq)]
pub enum OgvPlaybackUpdate {
    /// No new frame.
    Unchanged,
    /// New frame.
    Frame(Option<CinematicFrame>),
}

/// A playback tick.
#[derive(Debug, Clone, PartialEq)]
pub struct OgvPlaybackTick {
    /// Status.
    pub status: OgvPlaybackStatus,
    /// Update.
    pub update: OgvPlaybackUpdate,
}

/// OGV playback (`OgvPlayback`).
pub struct OgvPlayback {
    movie: OggMovie,
    decoder: TheoraDecoder,
    audio: Option<VorbisPcmStream>,
    epoch: f64,
    pass: usize,
    next_index: usize,
    picture: Option<CinematicFrame>,
    state: OgvPlaybackState,
    closed: bool,
    reset_audio: bool,
    closed_audio_position: Option<usize>,
    hold_playback: bool,
    loop_playback: bool,
    pending_audio: Vec<CinematicAudio>,
}

impl OgvPlayback {
    fn open(bytes: &[u8], options: OgvPlaybackOptions, clock_sample: f64) -> Result<Self, ClientError> {
        let movie = decode_ogg_movie(bytes)?;
        let decoder = TheoraDecoder::new(&movie.video[..3])?;
        let mut audio = None;
        if !options.silent {
            if let Some(audio_bytes) = &movie.audio {
                audio = Some(VorbisPcmStream::from_bytes(audio_bytes)?);
            }
        }
        Ok(Self {
            movie,
            decoder,
            audio,
            epoch: clock_sample,
            pass: 0,
            next_index: 0,
            picture: None,
            state: OgvPlaybackState::Playing,
            closed: false,
            reset_audio: true,
            closed_audio_position: None,
            hold_playback: options.hold,
            loop_playback: options.loop_playback,
            pending_audio: Vec::new(),
        })
    }

    /// Open playback over bytes (decodes the first frame and
    /// buffers initial audio for the first tick, since construction
    /// takes no host).
    pub fn new(bytes: &[u8], options: OgvPlaybackOptions, clock_sample: f64) -> Result<Self, ClientError> {
        let mut playback = Self::open(bytes, options, clock_sample)?;
        playback.decode_frame()?;
        playback.pending_audio = playback.take_audio(0.0)?;
        Ok(playback)
    }

    /// Open playback from a checkpoint (replays the exact packet and
    /// audio prefix without output callbacks).
    pub fn open_restored(
        bytes: &[u8],
        options: OgvPlaybackOptions,
        checkpoint: &OgvPlaybackCheckpoint,
    ) -> Result<Self, ClientError> {
        let mut playback = Self::open(bytes, options, checkpoint.epoch)?;
        playback.restore(checkpoint)?;
        Ok(playback)
    }

    /// Capture a checkpoint.
    pub fn capture_checkpoint(&self) -> Result<OgvPlaybackCheckpoint, ClientError> {
        Ok(OgvPlaybackCheckpoint {
            epoch: self.epoch,
            pass: self.pass,
            next_index: self.next_index,
            picture: self.picture.clone(),
            state: self.state,
            closed: self.closed,
            reset_audio: self.reset_audio,
            audio_position: if self.closed {
                self.closed_audio_position
            } else {
                self.audio_position()?
            },
            pending_audio: self.pending_audio.clone(),
        })
    }

    fn audio_position(&self) -> Result<Option<usize>, ClientError> {
        self.audio.as_ref().map(|audio| audio.position_frames()).transpose()
    }

    fn restore(&mut self, checkpoint: &OgvPlaybackCheckpoint) -> Result<(), ClientError> {
        if !checkpoint.epoch.is_finite() {
            return Err(ClientError::BadMedia("invalid OGV epoch".to_string()));
        }
        self.epoch = checkpoint.epoch;
        self.pass = checkpoint.pass;
        if checkpoint.next_index > self.movie.video.len() - 3 {
            return Err(ClientError::BadMedia("video packet cursor exceeds movie".to_string()));
        }
        // Opaque native state is reconstructed from the exact packet
        // prefix, without output callbacks.
        while self.next_index < checkpoint.next_index {
            self.decode_frame()?;
        }
        let actual = &self.picture;
        let expected = &checkpoint.picture;
        let differs = match (expected, actual) {
            (None, None) => false,
            (Some(a), Some(b)) => {
                a.index != b.index
                    || a.width != b.width
                    || a.height != b.height
                    || a.rgba.len() != b.rgba.len()
                    || a.rgba != b.rgba
            }
            _ => true,
        };
        if differs {
            return Err(ClientError::BadMedia(
                "reconstructed Theora frame differs from checkpoint".to_string(),
            ));
        }
        if (checkpoint.audio_position.is_none()) != self.audio.is_none() {
            return Err(ClientError::BadMedia("Vorbis stream ownership differs".to_string()));
        }
        if let (Some(audio), Some(position)) = (self.audio.as_mut(), checkpoint.audio_position) {
            if position > audio.frame_count() {
                return Err(ClientError::BadMedia("Vorbis sample cursor exceeds movie".to_string()));
            }
            while audio.position_frames()? < position {
                let remaining = position - audio.position_frames()?;
                if audio.read(remaining.min(4096))?.is_none() {
                    return Err(ClientError::BadMedia("Vorbis prefix ended early".to_string()));
                }
            }
        }
        self.picture = checkpoint.picture.clone();
        self.state = checkpoint.state;
        self.reset_audio = checkpoint.reset_audio;
        self.pending_audio = checkpoint.pending_audio.clone();
        if checkpoint.closed {
            self.close();
        }
        Ok(())
    }

    /// Dimensions.
    #[must_use]
    pub fn dimensions(&self) -> (usize, usize) {
        (self.decoder.width(), self.decoder.height())
    }

    /// Current frame.
    #[must_use]
    pub fn current_frame(&self) -> Option<CinematicFrame> {
        self.picture.clone()
    }

    fn decode_frame(&mut self) -> Result<(), ClientError> {
        let packet = self
            .movie
            .video
            .get(self.next_index + 3)
            .ok_or_else(|| ClientError::BadMedia("Missing Theora frame packet".to_string()))?;
        let TheoraPicture { width, height, rgba } = self.decoder.decode(packet)?;
        let source_time = self.next_index as f64 * self.decoder.frame_ms();
        self.picture = Some(CinematicFrame {
            rgba,
            width,
            height,
            index: self.next_index,
            pass: self.pass,
            source_time,
            time: self.epoch + source_time,
            decoded: true,
        });
        self.next_index += 1;
        Ok(())
    }

    fn take_audio(&mut self, elapsed: f64) -> Result<Vec<CinematicAudio>, ClientError> {
        let Some(audio) = self.audio.as_mut() else {
            return Ok(Vec::new());
        };
        let wanted = audio
            .frame_count()
            .min(((elapsed + 200.0) * f64::from(audio.sample_rate()) / 1000.0).ceil() as usize);
        let mut chunks = Vec::new();
        while audio.position_frames()? < wanted {
            let source_sample = audio.position_frames()?;
            let Some(chunk) = audio.read((wanted - source_sample).min(4096))? else {
                break;
            };
            let source_time = source_sample as f64 * 1000.0 / f64::from(chunk.sample_rate);
            chunks.push(CinematicAudio {
                samples: AudioSamples::I16(chunk.samples),
                channels: chunk.channels,
                sample_rate: chunk.sample_rate,
                source_sample,
                source_time,
                time: self.epoch + source_time,
                pass: self.pass,
                reset_stream: self.reset_audio,
            });
            self.reset_audio = false;
        }
        Ok(chunks)
    }

    fn queue_audio(&mut self, elapsed: f64, sink: &mut dyn CinematicAudioSink) -> Result<(), ClientError> {
        for audio in self.take_audio(elapsed)? {
            sink.on_audio(&audio);
        }
        Ok(())
    }

    fn flush_pending(&mut self, sink: &mut dyn CinematicAudioSink) {
        for audio in std::mem::take(&mut self.pending_audio) {
            sink.on_audio(&audio);
        }
    }

    /// Run playback (`run`).
    pub fn run(
        &mut self,
        clock_sample: f64,
        sink: &mut dyn CinematicAudioSink,
    ) -> Result<OgvPlaybackTick, ClientError> {
        if self.closed || self.state != OgvPlaybackState::Playing {
            return Ok(OgvPlaybackTick {
                status: match self.state {
                    OgvPlaybackState::Playing => OgvPlaybackStatus::Playing,
                    OgvPlaybackState::Held => OgvPlaybackStatus::Held,
                    OgvPlaybackState::Ended => OgvPlaybackStatus::Ended,
                },
                update: OgvPlaybackUpdate::Unchanged,
            });
        }
        let now = clock_sample;
        if !now.is_finite() || now < self.epoch {
            return Err(ClientError::BadMedia(
                "OGV clock must be finite and monotonic".to_string(),
            ));
        }
        self.flush_pending(sink);
        let count = self.movie.video.len() - 3;
        let duration = count as f64 * self.decoder.frame_ms();
        let mut elapsed = now - self.epoch;
        let mut looped = false;
        if elapsed >= duration {
            if self.hold_playback {
                while self.next_index < count {
                    self.decode_frame()?;
                }
                self.state = OgvPlaybackState::Held;
                return Ok(OgvPlaybackTick {
                    status: OgvPlaybackStatus::Held,
                    update: OgvPlaybackUpdate::Frame(self.picture.clone()),
                });
            }
            if !self.loop_playback {
                self.state = OgvPlaybackState::Ended;
                return Ok(OgvPlaybackTick {
                    status: OgvPlaybackStatus::Ended,
                    update: OgvPlaybackUpdate::Unchanged,
                });
            }
            let passes = (elapsed / duration).floor() as usize;
            let next = TheoraDecoder::new(&self.movie.video[..3])?;
            self.decoder.close();
            self.decoder = next;
            if let Some(audio) = self.audio.as_mut() {
                audio.seek(0)?;
            }
            self.pass += passes;
            self.epoch += passes as f64 * duration;
            elapsed = now - self.epoch;
            self.next_index = 0;
            self.reset_audio = true;
            looped = true;
        }
        let target = (count - 1).min((elapsed / self.decoder.frame_ms()).floor() as usize);
        let mut changed = false;
        while self.next_index <= target {
            self.decode_frame()?;
            changed = true;
        }
        self.queue_audio(elapsed, sink)?;
        Ok(OgvPlaybackTick {
            status: if looped {
                OgvPlaybackStatus::Looped
            } else {
                OgvPlaybackStatus::Playing
            },
            update: if changed {
                OgvPlaybackUpdate::Frame(self.picture.clone())
            } else {
                OgvPlaybackUpdate::Unchanged
            },
        })
    }

    /// Close playback.
    pub fn close(&mut self) {
        if self.closed {
            return;
        }
        self.closed_audio_position = self.audio.as_ref().and_then(|audio| audio.position_frames().ok());
        self.closed = true;
        self.state = OgvPlaybackState::Ended;
        if let Some(audio) = self.audio.as_mut() {
            audio.close();
        }
        self.decoder.close();
    }
}

/// An OGV playback checkpoint.
#[derive(Debug, Clone, PartialEq)]
pub struct OgvPlaybackCheckpoint {
    /// Epoch.
    pub epoch: f64,
    /// Pass.
    pub pass: usize,
    /// Next packet index.
    pub next_index: usize,
    /// Picture.
    pub picture: Option<CinematicFrame>,
    /// State.
    pub state: OgvPlaybackState,
    /// Closed.
    pub closed: bool,
    /// Reset audio on next queue.
    pub reset_audio: bool,
    /// Audio position.
    pub audio_position: Option<usize>,
    /// Buffered constructor audio.
    pub pending_audio: Vec<CinematicAudio>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn checksum_table() -> [u32; 256] {
        let mut table = [0u32; 256];
        for (index, slot) in table.iter_mut().enumerate() {
            let mut value = (index as u32) << 24;
            for _ in 0..8 {
                value = if value & 0x8000_0000 != 0 {
                    (value << 1) ^ 0x04c1_1db7
                } else {
                    value << 1
                };
            }
            *slot = value;
        }
        table
    }

    fn page(serial: u32, sequence: u32, flags: u8, granule: i64, packets: &[&[u8]]) -> Vec<u8> {
        let mut lacing = Vec::new();
        let mut payload = Vec::new();
        for packet in packets {
            lacing.push(packet.len() as u8);
            payload.extend_from_slice(packet);
        }
        let mut page = vec![0u8; 27 + lacing.len() + payload.len()];
        page[..4].copy_from_slice(b"OggS");
        page[4] = 0;
        page[5] = flags;
        page[6..14].copy_from_slice(&granule.to_le_bytes());
        page[14..18].copy_from_slice(&serial.to_le_bytes());
        page[18..22].copy_from_slice(&sequence.to_le_bytes());
        page[26] = lacing.len() as u8;
        page[27..27 + lacing.len()].copy_from_slice(&lacing);
        page[27 + lacing.len()..].copy_from_slice(&payload);
        let table = checksum_table();
        let mut crc = 0u32;
        for (index, byte) in page.iter().enumerate() {
            let byte = if (22..26).contains(&index) { 0 } else { *byte };
            crc = (crc << 8) ^ table[((crc >> 24) ^ u32::from(byte)) as usize & 255];
        }
        page[22..26].copy_from_slice(&crc.to_le_bytes());
        page
    }

    fn fake_movie() -> Vec<u8> {
        // Valid Ogg framing around Theora-labeled packets that carry
        // no real headers; the native decoder must reject them.
        let head = [vec![0x80], b"theora".to_vec()].concat();
        page(1, 0, 0x06, 0, &[&head, &[1, 2, 3], &[4, 5], &[6]])
    }

    #[test]
    fn framing_errors_surface() {
        assert!(OgvPlayback::new(&[0u8; 64], OgvPlaybackOptions::default(), 0.0).is_err());
        assert!(OgvPlayback::new(&[], OgvPlaybackOptions::default(), 0.0).is_err());
    }

    #[test]
    fn native_decoder_rejects_fake_headers() {
        // Success paths need real encoded streams; error paths do not.
        match OgvPlayback::new(&fake_movie(), OgvPlaybackOptions::default(), 0.0) {
            Err(error) => assert!(error.to_string().contains("Theora"), "unexpected {error}"),
            Ok(_) => panic!("expected Theora rejection"),
        }
    }
}
