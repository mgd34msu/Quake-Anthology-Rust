//! Common PCM readers for precached sounds and streamed music.
use crate::{FormatError, read::Reader};
use qa_core::primitives::{Pcm, PcmChannels, PcmFormat, PcmWidth};
use std::num::NonZeroU32;
pub mod ogg;
mod vorbis;
pub use vorbis::Vorbis;

const MAX_SAMPLES: usize = 256 * 1024 * 1024 / 2;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WavPolicy {
    Standard,
    Quake,
    Quake3,
    /// Admit full PCM once while retaining cue-aware native metadata.
    Shared,
}

#[derive(Clone, Copy, Debug)]
pub struct WavInfo {
    pub rate: NonZeroU32,
    pub channels: PcmChannels,
    pub width: u8,
    pub frames: usize,
    pub loop_start: Option<usize>,
    pub loop_end: Option<usize>,
    pub loop_invalid: bool,
    pub data_offset: usize,
    pub data_length: usize,
    pub info_tail_clamped: bool,
    pub riff_length_ignored: bool,
    pub zero_tail_ignored: bool,
}
pub struct Wav<'a> {
    pub info: WavInfo,
    pub data: &'a [u8],
}
impl std::ops::Deref for Wav<'_> {
    type Target = WavInfo;
    fn deref(&self) -> &Self::Target {
        &self.info
    }
}
impl WavInfo {
    pub fn format(self) -> PcmFormat {
        PcmFormat {
            rate: self.rate,
            channels: self.channels,
            width: PcmWidth::from_bytes(self.width).unwrap_or(PcmWidth::ThreeBytes),
        }
    }
    /// Native background streams stop their header scan at fmt/data and ignore
    /// cue metadata. The same scanner backs whole-file WAV admission below.
    pub fn read_source(
        physical: u64,
        read: impl FnMut(u64, &mut [u8]) -> Result<(), FormatError>,
    ) -> Result<Self, FormatError> {
        Self::scan(physical, WavPolicy::Quake3, true, read)
    }
    fn scan(
        physical: u64,
        policy: WavPolicy,
        header_only: bool,
        mut read: impl FnMut(u64, &mut [u8]) -> Result<(), FormatError>,
    ) -> Result<Self, FormatError> {
        let physical = usize::try_from(physical).map_err(|_| FormatError::InvalidRange)?;
        let mut header = [0; 12];
        read(0, &mut header[..4])?;
        if &header[..4] != b"RIFF" {
            return Err(FormatError::Unsupported);
        }
        read(4, &mut header[4..8])?;
        let length = u32::from_le_bytes([header[4], header[5], header[6], header[7]]) as usize;
        let declared_end = 8usize
            .checked_add(length)
            .ok_or(FormatError::InvalidRange)?;
        let end = if policy == WavPolicy::Standard {
            if length < 4 {
                return Err(FormatError::InvalidRange);
            }
            if declared_end > physical {
                return Err(FormatError::Truncated);
            }
            declared_end
        } else {
            // GetWavinfo uses the physical file end, not RIFF's outer length.
            if length > i32::MAX as usize {
                return Err(FormatError::InvalidRange);
            }
            physical
        };
        read(8, &mut header[8..12])?;
        if &header[8..12] != b"WAVE" {
            return Err(FormatError::Unsupported);
        }
        let mut at = 12;
        let mut format = None;
        let mut data = None;
        let mut cue = None;
        let mut sampler = None;
        let mut cue_seen = false;
        let mut loop_length = None;
        let mut info_tail_clamped = false;
        let mut zero_tail_ignored = false;
        let mut loop_invalid = false;
        while at < end {
            if end - at < 8
                && policy != WavPolicy::Standard
                && format.is_some()
                && data.is_some()
                && zero_tail(at, end, &mut read)?
            {
                zero_tail_ignored = true;
                break;
            }
            if end - at < 8 {
                return Err(FormatError::Truncated);
            }
            read(at as u64, &mut header[..8])?;
            let id = &header[..4];
            let length = u32::from_le_bytes([header[4], header[5], header[6], header[7]]) as usize;
            at += 8;
            if policy != WavPolicy::Standard && length > i32::MAX as usize {
                break;
            }
            let start = at;
            if length > end - start {
                // Native tools leave incomplete INFO after valid format/data.
                if policy != WavPolicy::Standard
                    && format.is_some()
                    && data.is_some()
                    && id == b"LIST"
                    && info_chunk(start, end, &mut read)?
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
            let prefix = match id {
                b"fmt " => 16,
                b"cue " if policy != WavPolicy::Quake3 => 28,
                b"smpl" if policy != WavPolicy::Quake3 => 60,
                b"LIST" if matches!(policy, WavPolicy::Quake | WavPolicy::Shared) => 24,
                _ => 0,
            }
            .min(length);
            let mut storage = [0; 60];
            if prefix != 0 {
                read(start as u64, &mut storage[..prefix])?;
            }
            let chunk = &storage[..prefix];
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
                b"data" if data.is_none() => data = Some((length, start)),
                b"cue " if policy != WavPolicy::Quake3 => {
                    cue_seen = true;
                    if cue.is_none() {
                        let parsed = (|| {
                            let count = s.u32()? as usize;
                            chunk_records(s.at, count, 24, length)?;
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
                        chunk_records(s.at, count, 24, length)?;
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
            if header_only && format.is_some() && data.is_some() {
                break;
            }
            at = start + length;
            if at < end {
                at = at
                    .checked_add(length & 1)
                    .filter(|&at| at <= end)
                    .ok_or(FormatError::Truncated)?;
            }
        }
        let (rate, channels, width) = format.ok_or(FormatError::Truncated)?;
        let (data_length, data_offset) = data.ok_or(FormatError::Truncated)?;
        let alignment = channels as usize * width as usize;
        if data_length % alignment != 0 {
            return Err(FormatError::InvalidRecordSize);
        }
        let mut frames = data_length / alignment;
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
        if !header_only && frames * channels as usize > MAX_SAMPLES {
            return Err(FormatError::InvalidRange);
        }
        Ok(Self {
            rate,
            channels,
            width,
            data_length: frames * alignment,
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
}
fn chunk_records(at: usize, count: usize, stride: usize, length: usize) -> Result<(), FormatError> {
    if count
        .checked_mul(stride)
        .and_then(|count| at.checked_add(count))
        .is_none_or(|end| end > length)
    {
        Err(FormatError::InvalidRange)
    } else {
        Ok(())
    }
}
fn zero_tail(
    at: usize,
    end: usize,
    read: &mut impl FnMut(u64, &mut [u8]) -> Result<(), FormatError>,
) -> Result<bool, FormatError> {
    let mut tail = [0; 8];
    read(at as u64, &mut tail[..end - at])?;
    Ok(tail[..end - at].iter().all(|&b| b == 0))
}
fn info_chunk(
    at: usize,
    end: usize,
    read: &mut impl FnMut(u64, &mut [u8]) -> Result<(), FormatError>,
) -> Result<bool, FormatError> {
    let mut bytes = [0; 4];
    if end - at < bytes.len() {
        return Ok(false);
    }
    read(at as u64, &mut bytes)?;
    Ok(&bytes == b"INFO")
}
impl<'a> Wav<'a> {
    pub fn parse(bytes: &'a [u8], policy: WavPolicy) -> Result<Self, FormatError> {
        let info = WavInfo::scan(bytes.len() as u64, policy, false, |at, out| {
            let at = usize::try_from(at).map_err(|_| FormatError::InvalidRange)?;
            let end = at.checked_add(out.len()).ok_or(FormatError::InvalidRange)?;
            out.copy_from_slice(bytes.get(at..end).ok_or(FormatError::Truncated)?);
            Ok(())
        })?;
        let data = &bytes[info.data_offset..info.data_offset + info.data_length];
        Ok(Self { info, data })
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
    let mut decoder = Vorbis::load(bytes.len() as u64, |at, out| {
        ogg::slice_read(bytes, at, out)
    })?;
    let format = decoder.format();
    let mut samples = Vec::new();
    while let Some(packet) = decoder.read_slice(bytes)? {
        if packet.len() > MAX_SAMPLES - samples.len() {
            return Err(FormatError::InvalidRange);
        }
        samples.extend_from_slice(packet);
    }
    Ok(Pcm {
        rate: format.rate,
        channels: format.channels,
        samples,
        loop_start: None,
        loop_end: None,
        loop_invalid: false,
    })
}
