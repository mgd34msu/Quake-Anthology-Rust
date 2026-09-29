//! Source PCM streams and the resampling stream bus.
//!
//! Donor provenance: `src/audio/streams.ts` (`MemoryPcmStream`,
//! `VorbisPcmStream`, `openPcmBytes`, `decodeSoundBytes`,
//! `RawAudioStream`).

use std::sync::atomic::{AtomicU64, Ordering};

use super::error::AudioError;
use super::types::{StreamPcm, StreamSamples};
use super::wav::{decode_wav, PcmSound};

/// Seekable decoded stream.
pub trait PcmStream {
    /// Sample rate in Hz.
    fn sample_rate(&self) -> u32;
    /// Channel count.
    fn channels(&self) -> u8;
    /// Total frames.
    fn frame_count(&self) -> u64;
    /// Read cursor in frames.
    fn position_frames(&self) -> Result<u64, AudioError>;
    /// Read up to `max_frames`; `None` at the end.
    fn read(&mut self, max_frames: u32) -> Result<Option<PcmSound>, AudioError>;
    /// Seek to a frame.
    fn seek(&mut self, frame: u64) -> Result<(), AudioError>;
    /// Close the stream.
    fn close(&mut self);
}

/// Stream over decoded memory.
pub struct MemoryPcmStream {
    pcm: PcmSound,
    position: usize,
    closed: bool,
}

impl MemoryPcmStream {
    /// Stream over PCM.
    #[must_use]
    pub const fn new(pcm: PcmSound) -> Self {
        Self {
            pcm,
            position: 0,
            closed: false,
        }
    }

    fn check(&self) -> Result<(), AudioError> {
        if self.closed {
            return Err(AudioError::StreamClosed);
        }
        Ok(())
    }
}

impl PcmStream for MemoryPcmStream {
    fn sample_rate(&self) -> u32 {
        self.pcm.sample_rate
    }

    fn channels(&self) -> u8 {
        self.pcm.channels
    }

    fn frame_count(&self) -> u64 {
        self.pcm.frame_count as u64
    }

    fn position_frames(&self) -> Result<u64, AudioError> {
        Ok(self.position as u64)
    }

    fn read(&mut self, max_frames: u32) -> Result<Option<PcmSound>, AudioError> {
        self.check()?;
        if max_frames < 1 {
            return Err(AudioError::BadFrameRequest);
        }
        let count = (max_frames as usize).min(self.pcm.frame_count - self.position);
        if count == 0 {
            return Ok(None);
        }
        let channels = usize::from(self.pcm.channels);
        let samples = self.pcm.samples[self.position * channels..(self.position + count) * channels].to_vec();
        self.position += count;
        Ok(Some(PcmSound {
            sample_rate: self.pcm.sample_rate,
            channels: self.pcm.channels,
            samples,
            frame_count: count,
            loop_start: None,
        }))
    }

    fn seek(&mut self, frame: u64) -> Result<(), AudioError> {
        self.check()?;
        if frame > self.pcm.frame_count as u64 {
            return Err(AudioError::StreamSeek);
        }
        self.position = frame as usize;
        Ok(())
    }

    fn close(&mut self) {
        self.closed = true;
    }
}

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Stream over a platform Vorbis decoder.
pub struct VorbisPcmStream {
    decoder: qa_platform::vorbis::VorbisDecoder,
    temporary_directory: Option<std::path::PathBuf>,
    closed: bool,
}

impl VorbisPcmStream {
    /// Open a file path.
    pub fn open(path: &str) -> Result<Self, AudioError> {
        Ok(Self {
            decoder: qa_platform::vorbis::VorbisDecoder::open(path).map_err(AudioError::from)?,
            temporary_directory: None,
            closed: false,
        })
    }

    /// Decode archive bytes through an owned temporary file.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, AudioError> {
        let directory = std::env::temp_dir().join(format!(
            "quake-audio-{}-{}",
            std::process::id(),
            TEMP_COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&directory).map_err(|error| AudioError::StreamIo(error.to_string()))?;
        let result = (|| -> Result<Self, AudioError> {
            let path = directory.join("source.ogg");
            std::fs::write(&path, bytes).map_err(|error| AudioError::StreamIo(error.to_string()))?;
            Ok(Self {
                decoder: qa_platform::vorbis::VorbisDecoder::open(path.to_string_lossy().as_ref())
                    .map_err(AudioError::from)?,
                temporary_directory: Some(directory.clone()),
                closed: false,
            })
        })();
        if result.is_err() {
            let _ = std::fs::remove_dir_all(&directory);
        }
        result
    }
}

impl Drop for VorbisPcmStream {
    fn drop(&mut self) {
        self.close();
    }
}

impl PcmStream for VorbisPcmStream {
    fn sample_rate(&self) -> u32 {
        self.decoder.metadata().sample_rate
    }

    fn channels(&self) -> u8 {
        self.decoder.metadata().channels
    }

    fn frame_count(&self) -> u64 {
        self.decoder.metadata().total_frames
    }

    fn position_frames(&self) -> Result<u64, AudioError> {
        self.decoder.position_frames().map_err(AudioError::from)
    }

    fn read(&mut self, max_frames: u32) -> Result<Option<PcmSound>, AudioError> {
        let chunk = self.decoder.read(max_frames).map_err(AudioError::from)?;
        Ok(chunk.map(|chunk| PcmSound {
            sample_rate: chunk.sample_rate,
            channels: chunk.channels,
            samples: chunk.samples,
            frame_count: chunk.frames as usize,
            loop_start: None,
        }))
    }

    fn seek(&mut self, frame: u64) -> Result<(), AudioError> {
        self.decoder.seek(frame).map_err(AudioError::from)
    }

    fn close(&mut self) {
        if self.closed {
            return;
        }
        self.closed = true;
        let _ = self.decoder.close();
        if let Some(directory) = self.temporary_directory.take() {
            let _ = std::fs::remove_dir_all(directory);
        }
    }
}

fn is_ogg(bytes: &[u8]) -> bool {
    bytes.len() >= 4 && bytes[0] == 79 && bytes[1] == 103 && bytes[2] == 103 && bytes[3] == 83
}

/// Open bytes as a stream: Ogg decodes through Vorbis, else WAV memory.
pub fn open_pcm_bytes(bytes: &[u8], source: &str) -> Result<Box<dyn PcmStream>, AudioError> {
    if is_ogg(bytes) {
        return Ok(Box::new(VorbisPcmStream::from_bytes(bytes)?));
    }
    Ok(Box::new(MemoryPcmStream::new(decode_wav(bytes, source)?.pcm)))
}

/// Fully decode bytes to PCM.
pub fn decode_sound_bytes(bytes: &[u8], source: &str) -> Result<PcmSound, AudioError> {
    if !is_ogg(bytes) {
        return Ok(decode_wav(bytes, source)?.pcm);
    }
    let mut stream = VorbisPcmStream::from_bytes(bytes)?;
    let mut samples = Vec::new();
    while let Some(chunk) = stream.read(16384)? {
        samples.extend_from_slice(&chunk.samples);
    }
    let channels = stream.channels();
    let sample_rate = stream.sample_rate();
    stream.close();
    Ok(PcmSound {
        sample_rate,
        channels,
        frame_count: samples.len() / usize::from(channels),
        samples,
        loop_start: None,
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Segment {
    begin: i64,
    end: i64,
    samples: Vec<i16>,
}

/// Checkpoint segment.
#[derive(Debug, Clone, PartialEq)]
pub struct RawSegmentCheckpoint {
    /// First source frame.
    pub begin: i64,
    /// One past the last source frame.
    pub end: i64,
    /// Interleaved samples.
    pub samples: Vec<i32>,
}

/// Raw stream checkpoint.
#[derive(Debug, Clone, PartialEq)]
pub struct RawCheckpoint {
    /// Output rate in Hz.
    pub output_rate: i64,
    /// Input rate in Hz (0 when uninitialized).
    pub input_rate: i64,
    /// Channel count.
    pub channels: i64,
    /// Fractional origin.
    pub origin: f64,
    /// Mixed output frames.
    pub output_frames: i64,
    /// Queued end frame.
    pub end: i64,
    /// Paused.
    pub paused: bool,
    /// Segments.
    pub segments: Vec<RawSegmentCheckpoint>,
}

/// Resampling stream bus: chunk boundaries never reset phase.
pub struct RawAudioStream {
    segments: Vec<Segment>,
    input_rate: u32,
    channels: u8,
    origin: f64,
    output_frames: i64,
    end: i64,
    /// Paused streams mix silence.
    pub paused: bool,
    output_rate: u32,
}

impl RawAudioStream {
    /// Stream bus at an output rate.
    #[must_use]
    pub const fn new(output_rate: u32) -> Self {
        Self {
            segments: Vec::new(),
            input_rate: 0,
            channels: 2,
            origin: 0.0,
            output_frames: 0,
            end: 0,
            paused: false,
            output_rate,
        }
    }

    /// Capture a checkpoint.
    #[must_use]
    pub fn capture_checkpoint(&self) -> RawCheckpoint {
        RawCheckpoint {
            output_rate: i64::from(self.output_rate),
            input_rate: i64::from(self.input_rate),
            channels: i64::from(self.channels),
            origin: self.origin,
            output_frames: self.output_frames,
            end: self.end,
            paused: self.paused,
            segments: self
                .segments
                .iter()
                .map(|segment| RawSegmentCheckpoint {
                    begin: segment.begin,
                    end: segment.end,
                    samples: segment.samples.iter().map(|sample| i32::from(*sample)).collect(),
                })
                .collect(),
        }
    }

    /// Restore a checkpoint, retargeting the output rate.
    pub fn restore_checkpoint(value: &RawCheckpoint, output_rate: u32) -> Result<Self, AudioError> {
        if value.output_rate < 1 {
            return Err(AudioError::BadCheckpointRate);
        }
        if value.input_rate < 0 {
            return Err(AudioError::BadCheckpointField("inputRate".to_string()));
        }
        if value.channels != 1 && value.channels != 2 {
            return Err(AudioError::BadCheckpointField("channels".to_string()));
        }
        if !value.origin.is_finite() {
            return Err(AudioError::BadCheckpointField("origin".to_string()));
        }
        if value.output_frames < 0 || value.end < 0 {
            return Err(AudioError::BadCheckpointField("cursor".to_string()));
        }
        let channels = value.channels as u8;
        let mut segments = Vec::with_capacity(value.segments.len());
        for segment in &value.segments {
            if segment.begin < 0 || segment.end < segment.begin {
                return Err(AudioError::BadCheckpointField("segment".to_string()));
            }
            for sample in &segment.samples {
                if *sample < -32768 || *sample > 32767 {
                    return Err(AudioError::CheckpointSample);
                }
            }
            if segment.samples.len() != (segment.end - segment.begin) as usize * usize::from(channels)
                || segment.begin == segment.end
            {
                return Err(AudioError::CheckpointExtent);
            }
            segments.push(Segment {
                begin: segment.begin,
                end: segment.end,
                samples: segment.samples.iter().map(|sample| *sample as i16).collect(),
            });
        }
        for window in segments.windows(2) {
            if window[0].end != window[1].begin {
                return Err(AudioError::CheckpointDiscontinuity);
            }
        }
        let stream = Self {
            segments,
            input_rate: value.input_rate as u32,
            channels,
            origin: value.origin,
            output_frames: value.output_frames,
            end: value.end,
            paused: value.paused,
            output_rate: value.output_rate as u32,
        };
        if stream.origin < 0.0
            || stream
                .segments
                .first()
                .is_some_and(|first| stream.source_position() < first.begin)
            || stream.segments.last().is_some_and(|last| last.end != stream.end)
            || stream.input_rate == 0 && !stream.segments.is_empty()
        {
            return Err(AudioError::CheckpointCursor);
        }
        if output_rate < 1 {
            return Err(AudioError::BadPcmRate);
        }
        Ok(if stream.output_rate == output_rate {
            stream
        } else {
            stream.with_output_rate(output_rate)
        })
    }

    /// Whether a chunk started the resampling phase.
    #[must_use]
    pub const fn initialized(&self) -> bool {
        self.input_rate != 0
    }

    /// Current source frame.
    #[must_use]
    pub fn source_position(&self) -> i64 {
        (self.origin + self.output_frames as f64 * f64::from(self.input_rate) / f64::from(self.output_rate)).floor()
            as i64
    }

    /// Queued source frames.
    #[must_use]
    pub fn queued_source_frames(&self) -> i64 {
        (self.end - self.source_position()).max(0)
    }

    /// Retarget the output rate, retaining queue and phase.
    #[must_use]
    pub fn with_output_rate(&self, output_rate: u32) -> Self {
        Self {
            segments: self.segments.clone(),
            input_rate: self.input_rate,
            channels: self.channels,
            origin: self.origin + self.output_frames as f64 * f64::from(self.input_rate) / f64::from(self.output_rate),
            output_frames: 0,
            end: self.end,
            paused: self.paused,
            output_rate,
        }
    }

    /// Queue a chunk.
    pub fn queue(&mut self, chunk: &StreamPcm) -> Result<(), AudioError> {
        let sample_count = match &chunk.samples {
            StreamSamples::S16(samples) => samples.len(),
            StreamSamples::U8(samples) => samples.len(),
        };
        if chunk.sample_rate < 1
            || chunk.source_sample < 0
            || chunk.channels == 0
            || sample_count % usize::from(chunk.channels) != 0
        {
            return Err(AudioError::BadStreamedPcm);
        }
        if chunk.reset_stream || self.input_rate == 0 {
            self.segments.clear();
            self.input_rate = chunk.sample_rate;
            self.channels = chunk.channels;
            self.origin = chunk.source_sample as f64;
            self.output_frames = 0;
            self.end = chunk.source_sample;
        }
        if chunk.sample_rate != self.input_rate || chunk.channels != self.channels {
            return Err(AudioError::StreamFormatChanged);
        }
        if chunk.source_sample != self.end {
            return Err(AudioError::StreamDiscontinuity {
                expected: self.end,
                received: chunk.source_sample,
            });
        }
        let samples: Vec<i16> = match &chunk.samples {
            StreamSamples::S16(samples) => samples.clone(),
            StreamSamples::U8(samples) => samples
                .iter()
                .map(|value| ((i32::from(*value) - 128) << 8) as i16)
                .collect(),
        };
        let end = chunk.source_sample + samples.len() as i64 / i64::from(chunk.channels);
        if end > chunk.source_sample {
            self.segments.push(Segment {
                begin: chunk.source_sample,
                end,
                samples,
            });
        }
        self.end = end;
        Ok(())
    }

    /// Mix frames with an optional refill.
    pub fn mix(
        &mut self,
        frames: usize,
        gain: f64,
        mut refill: Option<&mut dyn FnMut() -> Result<Option<StreamPcm>, AudioError>>,
    ) -> Result<Vec<f64>, AudioError> {
        let mut output = vec![0.0; frames * 2];
        if self.paused {
            return Ok(output);
        }
        for frame in 0..frames {
            loop {
                let position = self.source_position();
                let covered = self.segments.first().is_some_and(|segment| position < segment.end);
                if covered {
                    break;
                }
                if !self.segments.is_empty() {
                    self.segments.remove(0);
                }
                if self.segments.is_empty() {
                    let chunk = match refill.as_deref_mut() {
                        None => return Ok(output),
                        Some(refill) => refill()?,
                    };
                    match chunk {
                        None => return Ok(output),
                        Some(chunk) => self.queue(&chunk)?,
                    }
                }
            }
            let segment = self.segments.first().expect("covered");
            let index = (self.source_position() - segment.begin) as usize * usize::from(self.channels);
            let left = segment.samples.get(index).copied().ok_or(AudioError::StreamPosition)?;
            let right = if self.channels == 1 {
                left
            } else {
                segment
                    .samples
                    .get(index + 1)
                    .copied()
                    .ok_or(AudioError::StreamPosition)?
            };
            output[frame * 2] = f64::from(left) * gain;
            output[frame * 2 + 1] = f64::from(right) * gain;
            self.output_frames += 1;
        }
        Ok(output)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chunk(samples: Vec<i16>, source_sample: i64, reset: bool) -> StreamPcm {
        StreamPcm {
            samples: StreamSamples::S16(samples),
            sample_rate: 11025,
            channels: 1,
            source_sample,
            reset_stream: reset,
        }
    }

    #[test]
    fn queues_and_mixes_mono() {
        let mut stream = RawAudioStream::new(11025);
        assert!(!stream.initialized());
        stream.queue(&chunk(vec![1000, 2000, 3000, 4000], 0, true)).unwrap();
        stream.queue(&chunk(vec![5000, 6000], 4, false)).unwrap();
        assert_eq!(stream.queued_source_frames(), 6);
        let mixed = stream.mix(3, 1.0, None).unwrap();
        assert_eq!(mixed, vec![1000.0, 1000.0, 2000.0, 2000.0, 3000.0, 3000.0]);
        let checkpoint = stream.capture_checkpoint();
        let restored = RawAudioStream::restore_checkpoint(&checkpoint, 11025).unwrap();
        assert_eq!(restored.capture_checkpoint(), checkpoint);
        let retargeted = RawAudioStream::restore_checkpoint(&checkpoint, 22050).unwrap();
        assert_eq!(retargeted.source_position(), stream.source_position());
    }

    #[test]
    fn memory_streams_read_and_seek() {
        let pcm = PcmSound {
            sample_rate: 8000,
            channels: 1,
            samples: vec![1, 2, 3, 4],
            frame_count: 4,
            loop_start: None,
        };
        let mut stream = MemoryPcmStream::new(pcm);
        assert_eq!(stream.read(2).unwrap().unwrap().samples, vec![1, 2]);
        stream.seek(3).unwrap();
        assert_eq!(stream.read(8).unwrap().unwrap().samples, vec![4]);
        assert!(stream.read(8).unwrap().is_none());
        assert!(stream.seek(9).is_err());
        stream.close();
        assert!(stream.read(1).is_err());
    }
}
