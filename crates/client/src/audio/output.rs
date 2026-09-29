//! Output format validation, PCM encoding, and queue resampling.
//!
//! Donor provenance: `src/audio/output.ts` (`audioOutputFormat`,
//! `audioKhzRate`, `encodeOutputPcm`, `resampleQueuedPcm`).

use std::borrow::Cow;

use super::error::AudioError;
use super::streams::RawAudioStream;
use super::types::{StreamPcm, StreamSamples};

/// Output device format.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AudioOutputFormat {
    /// Sample rate in Hz.
    pub sample_rate: u32,
    /// Channel count (1 or 2).
    pub channels: u8,
    /// Sample bits (8 or 16).
    pub sample_bits: u8,
}

/// Default output: 44100 Hz stereo 16-bit.
pub const DEFAULT_AUDIO_OUTPUT_FORMAT: AudioOutputFormat = AudioOutputFormat {
    sample_rate: 44100,
    channels: 2,
    sample_bits: 16,
};

/// Canonical output rates.
pub const AUDIO_OUTPUT_RATES: [u32; 4] = [11025, 22050, 44100, 48000];

/// Validate an output format.
pub fn audio_output_format(sample_rate: i64, channels: i64, sample_bits: i64) -> Result<AudioOutputFormat, AudioError> {
    if !(8000..=192000).contains(&sample_rate) || channels != 1 && channels != 2 || sample_bits != 8 && sample_bits != 16 {
        return Err(AudioError::BadOutputFormat);
    }
    Ok(AudioOutputFormat {
        sample_rate: sample_rate as u32,
        channels: channels as u8,
        sample_bits: sample_bits as u8,
    })
}

/// Parse a kHz shorthand to a sample rate.
#[must_use]
pub fn audio_khz_rate(value: &str) -> Option<u32> {
    if value.trim().is_empty() {
        return None;
    }
    let trimmed = value.trim();
    let number = trimmed.parse::<f64>().ok().or_else(|| {
        let hex = trimmed.strip_prefix("0x").or_else(|| trimmed.strip_prefix("0X"))?;
        i64::from_str_radix(hex, 16).ok().map(|value| value as f64)
    })?;
    match number {
        value if value == 11.0 => Some(11025),
        value if value == 22.0 => Some(22050),
        value if value == 44.0 => Some(44100),
        value if value == 48.0 => Some(48000),
        _ => None,
    }
}

/// Encoded output PCM.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EncodedPcm<'a> {
    /// 16-bit samples (borrowed when no conversion is needed).
    S16(Cow<'a, [i16]>),
    /// 8-bit samples.
    U8(Vec<u8>),
}

/// Encode stereo PCM for the output format.
pub fn encode_output_pcm<'a>(stereo: &'a [i16], format: &AudioOutputFormat) -> Result<EncodedPcm<'a>, AudioError> {
    if stereo.len() % 2 != 0 {
        return Err(AudioError::StereoFrames);
    }
    if format.channels == 2 && format.sample_bits == 16 {
        return Ok(EncodedPcm::S16(Cow::Borrowed(stereo)));
    }
    let frames = stereo.len() / 2;
    if format.sample_bits == 16 {
        let mut samples = vec![0i16; frames * usize::from(format.channels)];
        for frame in 0..frames {
            for channel in 0..format.channels {
                let value = if format.channels == 1 {
                    (i32::from(stereo[frame * 2]) + i32::from(stereo[frame * 2 + 1])) / 2
                } else if channel == 0 {
                    i32::from(stereo[frame * 2])
                } else {
                    i32::from(stereo[frame * 2 + 1])
                };
                samples[frame * usize::from(format.channels) + usize::from(channel)] = value as i16;
            }
        }
        return Ok(EncodedPcm::S16(Cow::Owned(samples)));
    }
    let mut samples = vec![0u8; frames * usize::from(format.channels)];
    for frame in 0..frames {
        for channel in 0..format.channels {
            let value = if format.channels == 1 {
                (i32::from(stereo[frame * 2]) + i32::from(stereo[frame * 2 + 1])) / 2
            } else if channel == 0 {
                i32::from(stereo[frame * 2])
            } else {
                i32::from(stereo[frame * 2 + 1])
            };
            samples[frame * usize::from(format.channels) + usize::from(channel)] = ((value + 32768) as u32 >> 8) as u8;
        }
    }
    Ok(EncodedPcm::U8(samples))
}

pub(crate) fn to_int16(value: f64) -> i16 {
    let mut wrapped = value.trunc() % 65536.0;
    if wrapped < 0.0 {
        wrapped += 65536.0;
    }
    if wrapped >= 32768.0 {
        wrapped -= 65536.0;
    }
    wrapped as i16
}

/// Resample queued PCM across an output change.
pub fn resample_queued_pcm(samples: &[i16], previous_rate: u32, next_rate: u32) -> Result<Vec<i16>, AudioError> {
    if previous_rate == next_rate || samples.is_empty() {
        return Ok(samples.to_vec());
    }
    let mut stream = RawAudioStream::new(next_rate);
    stream.queue(&StreamPcm {
        samples: StreamSamples::S16(samples.to_vec()),
        sample_rate: previous_rate,
        channels: 2,
        source_sample: 0,
        reset_stream: true,
    })?;
    let frames = (samples.len() as f64 / 2.0 * f64::from(next_rate) / f64::from(previous_rate)).ceil() as usize;
    Ok(stream.mix(frames, 1.0, None)?.iter().map(|sample| to_int16(*sample)).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_and_encodes() {
        let format = audio_output_format(44100, 2, 16).unwrap();
        assert_eq!(format, DEFAULT_AUDIO_OUTPUT_FORMAT);
        assert!(audio_output_format(4000, 2, 16).is_err());
        assert!(audio_output_format(44100, 3, 16).is_err());
        assert_eq!(audio_khz_rate("44"), Some(44100));
        assert_eq!(audio_khz_rate(" 22 "), Some(22050));
        assert_eq!(audio_khz_rate(""), None);
        assert_eq!(audio_khz_rate("96"), None);
        assert!(matches!(encode_output_pcm(&[1, 2], &format).unwrap(), EncodedPcm::S16(_)));
        let mono = audio_output_format(22050, 1, 16).unwrap();
        assert_eq!(
            encode_output_pcm(&[1000, 2000, -1000, -2000], &mono).unwrap(),
            EncodedPcm::S16(Cow::Owned(vec![1500, -1500]))
        );
        let eight = audio_output_format(11025, 1, 8).unwrap();
        assert_eq!(encode_output_pcm(&[0, 0], &eight).unwrap(), EncodedPcm::U8(vec![128]));
        assert!(encode_output_pcm(&[1], &format).is_err());
    }

    #[test]
    fn resamples_queues() {
        let samples = vec![1000i16, -1000, 2000, -2000];
        assert_eq!(resample_queued_pcm(&samples, 11025, 11025).unwrap(), samples);
        let up = resample_queued_pcm(&samples, 11025, 22050).unwrap();
        assert_eq!(up.len(), 8);
        assert_eq!(up[0], 1000);
        assert_eq!(up[1], -1000);
    }
}
