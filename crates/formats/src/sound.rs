//! Decode once at precache into the shared PCM representation.
use crate::{FormatError, read::Reader};
use qa_core::primitives::{Pcm, PcmChannels, PcmWidth};
use std::{io::Cursor, num::NonZeroU32};

const MAX_SAMPLES: usize = 256 * 1024 * 1024 / 2;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WavPolicy {
    Standard,
    Quake,
    Quake3,
    /// Admit full PCM once while retaining cue-aware native metadata.
    Shared,
}

pub struct Wav<'a> {
    pub rate: NonZeroU32,
    pub channels: PcmChannels,
    pub width: u8,
    pub data: &'a [u8],
    pub frames: usize,
    pub loop_start: Option<usize>,
    pub loop_end: Option<usize>,
    pub loop_invalid: bool,
    pub data_offset: usize,
    pub info_tail_clamped: bool,
    pub riff_length_ignored: bool,
    pub zero_tail_ignored: bool,
}
impl<'a> Wav<'a> {
    pub fn parse(bytes: &'a [u8], policy: WavPolicy) -> Result<Self, FormatError> {
        let mut r = Reader::new(bytes);
        if r.take(4)? != b"RIFF" {
            return Err(FormatError::Unsupported);
        }
        let length = r.u32()? as usize;
        let declared_end = 8usize
            .checked_add(length)
            .ok_or(FormatError::InvalidRange)?;
        let end = if policy == WavPolicy::Standard {
            if length < 4 {
                return Err(FormatError::InvalidRange);
            }
            if declared_end > bytes.len() {
                return Err(FormatError::Truncated);
            }
            declared_end
        } else {
            // GetWavinfo uses the physical file end, not RIFF's outer length.
            if length > i32::MAX as usize {
                return Err(FormatError::InvalidRange);
            }
            bytes.len()
        };
        if r.take(4)? != b"WAVE" {
            return Err(FormatError::Unsupported);
        }
        r.bytes = &bytes[..end];
        let mut format = None;
        let mut data = None;
        let mut cue = None;
        let mut sampler = None;
        let mut cue_seen = false;
        let mut loop_length = None;
        let mut info_tail_clamped = false;
        let mut zero_tail_ignored = false;
        let mut loop_invalid = false;
        while r.at < end {
            if end - r.at < 8
                && policy != WavPolicy::Standard
                && format.is_some()
                && data.is_some()
                && r.bytes[r.at..].iter().all(|&b| b == 0)
            {
                zero_tail_ignored = true;
                break;
            }
            let id = r.take(4)?;
            let length = r.u32()? as usize;
            if policy != WavPolicy::Standard && length > i32::MAX as usize {
                break;
            }
            let start = r.at;
            if length > end - start {
                // Native tools leave incomplete INFO after valid format/data.
                if policy != WavPolicy::Standard
                    && format.is_some()
                    && data.is_some()
                    && id == b"LIST"
                    && r.bytes.get(start..start + 4) == Some(b"INFO")
                {
                    info_tail_clamped = true;
                    break;
                }
                if policy == WavPolicy::Shared
                    && format.is_some()
                    && data.is_some()
                    && matches!(id, b"cue " | b"smpl" | b"LIST")
                {
                    // Q3's separate fmt/data scans do not read trailing loop
                    // metadata. Retain the PCM and its cue-aware rejection.
                    loop_invalid = true;
                    break;
                }
                return Err(FormatError::Truncated);
            }
            let chunk = r.take(length)?;
            let mut s = Reader::new(chunk);
            match id {
                b"fmt " if format.is_none() => {
                    if s.u16()? != 1 {
                        return Err(FormatError::Unsupported);
                    }
                    let channels = match s.u16()? {
                        1 => PcmChannels::Mono,
                        2 => PcmChannels::Stereo,
                        _ => return Err(FormatError::Unsupported),
                    };
                    let rate = NonZeroU32::new(s.u32()?).ok_or(FormatError::InvalidValue)?;
                    let byte_rate = s.u32()?;
                    let alignment = s.u16()?;
                    let width = match s.u16()? {
                        8 => 1,
                        16 => 2,
                        24 => 3,
                        _ => return Err(FormatError::Unsupported),
                    };
                    if alignment != channels as u16 * width as u16
                        || u64::from(byte_rate) != u64::from(rate.get()) * u64::from(alignment)
                    {
                        return Err(FormatError::InvalidRecordSize);
                    }
                    format = Some((rate, channels, width));
                }
                b"data" if data.is_none() => data = Some((chunk, start)),
                b"cue " if policy != WavPolicy::Quake3 => {
                    cue_seen = true;
                    if cue.is_none() {
                        let parsed = (|| {
                            let count = s.u32()? as usize;
                            s.remaining_records(count, 24)?;
                            if count == 0 {
                                return Ok(None);
                            }
                            s.take(20)?;
                            Ok(Some(s.u32()? as usize))
                        })();
                        match parsed {
                            Ok(start) => cue = start,
                            Err(_) if policy == WavPolicy::Shared => loop_invalid = true,
                            Err(error) => return Err(error),
                        }
                    }
                }
                b"smpl" if policy != WavPolicy::Quake3 && sampler.is_none() => {
                    let parsed = (|| {
                        s.take(28)?;
                        let count = s.u32()? as usize;
                        s.take(4)?;
                        s.remaining_records(count, 24)?;
                        if count == 0 {
                            return Ok(None);
                        }
                        s.take(4)?;
                        let kind = s.u32()?;
                        let begin = s.u32()?;
                        let finish = s.u32()?;
                        let ignored = matches!(policy, WavPolicy::Quake | WavPolicy::Shared)
                            && kind == 255
                            && begin == u32::MAX
                            && finish == u32::MAX;
                        Ok((!ignored).then_some(begin as usize))
                    })();
                    match parsed {
                        Ok(start) => sampler = start,
                        Err(_) if policy == WavPolicy::Shared => loop_invalid = true,
                        Err(error) => return Err(error),
                    }
                }
                b"LIST"
                    if matches!(policy, WavPolicy::Quake | WavPolicy::Shared)
                        && cue_seen
                        && loop_length.is_none()
                        && chunk.len() >= 24
                        && &chunk[20..24] == b"mark" =>
                {
                    s.take(16)?;
                    loop_length = Some(s.u32()? as usize);
                }
                _ => (),
            }
            if r.at < end {
                r.take(length & 1)?;
            }
        }
        let (rate, channels, width) = format.ok_or(FormatError::Truncated)?;
        let (data, data_offset) = data.ok_or(FormatError::Truncated)?;
        let alignment = channels as usize * width as usize;
        if data.len() % alignment != 0 {
            return Err(FormatError::InvalidRecordSize);
        }
        let mut frames = data.len() / alignment;
        let mut loop_start = cue.or(sampler);
        let mut loop_end = None;
        if let Some(start) = loop_start {
            if start >= frames {
                loop_invalid = true;
            } else if let Some(length) = loop_length {
                if length == 0 || length > frames - start {
                    loop_invalid = true;
                } else {
                    loop_end = Some(start + length);
                }
            }
        }
        if loop_invalid {
            if policy != WavPolicy::Shared {
                return Err(FormatError::InvalidRange);
            }
            loop_start = None;
            loop_end = None;
        }
        if policy != WavPolicy::Shared {
            frames = loop_end.unwrap_or(frames);
        }
        if frames * channels as usize > MAX_SAMPLES {
            return Err(FormatError::InvalidRange);
        }
        Ok(Self {
            rate,
            channels,
            width,
            data: &data[..frames * alignment],
            frames,
            loop_start,
            loop_end,
            loop_invalid,
            data_offset,
            info_tail_clamped,
            riff_length_ignored: declared_end != end,
            zero_tail_ignored,
        })
    }
    pub fn decode(&self) -> Pcm {
        let width = PcmWidth::from_bytes(self.width).unwrap_or(PcmWidth::ThreeBytes);
        let samples = self
            .data
            .chunks_exact(self.width as usize)
            .map(|s| width.sample(s, false))
            .collect();
        Pcm {
            rate: self.rate,
            channels: self.channels,
            samples,
            loop_start: self.loop_start,
            loop_end: self.loop_end,
            loop_invalid: self.loop_invalid,
        }
    }
}

pub fn decode(bytes: &[u8], policy: WavPolicy) -> Result<Pcm, FormatError> {
    if bytes.starts_with(b"OggS") {
        vorbis(bytes)
    } else {
        Ok(Wav::parse(bytes, policy)?.decode())
    }
}
pub fn vorbis(bytes: &[u8]) -> Result<Pcm, FormatError> {
    let mut reader = lewton::inside_ogg::OggStreamReader::new(Cursor::new(bytes))
        .map_err(|_| FormatError::InvalidValue)?;
    let channels = match reader.ident_hdr.audio_channels {
        1 => PcmChannels::Mono,
        2 => PcmChannels::Stereo,
        _ => return Err(FormatError::Unsupported),
    };
    let rate =
        NonZeroU32::new(reader.ident_hdr.audio_sample_rate).ok_or(FormatError::InvalidValue)?;
    let mut samples = Vec::new();
    while let Some(packet) = reader
        .read_dec_packet_itl()
        .map_err(|_| FormatError::InvalidValue)?
    {
        if reader.ident_hdr.audio_channels != channels as u8
            || reader.ident_hdr.audio_sample_rate != rate.get()
        {
            return Err(FormatError::Unsupported);
        }
        if packet.len() > MAX_SAMPLES - samples.len() || packet.len() % channels as usize != 0 {
            return Err(FormatError::InvalidRange);
        }
        samples.extend(packet);
    }
    Ok(Pcm {
        rate,
        channels,
        samples,
        loop_start: None,
        loop_end: None,
        loop_invalid: false,
    })
}
