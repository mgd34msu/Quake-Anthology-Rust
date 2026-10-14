//! One source-driven Vorbis reader for precached sounds and streamed music.
use super::ogg::{self, Packets, Plan};
use crate::FormatError;
use lewton::{
    audio::{PreviousWindowRight, read_audio_packet_generic},
    header::{
        CommentHeader, IdentHeader, SetupHeader, read_header_comment, read_header_ident,
        read_header_setup,
    },
    samples::InterleavedSamples,
};
use qa_core::primitives::{PcmChannels, PcmFormat, PcmWidth};
use std::num::NonZeroU32;

struct Pending {
    serial: u32,
    ident: IdentHeader,
    comment: Option<CommentHeader>,
    setup: Option<SetupHeader>,
}
struct Link {
    serial: u32,
    ident: IdentHeader,
    comment: CommentHeader,
    setup: SetupHeader,
}

struct VorbisState {
    active: usize,
    headers_left: u8,
    ended: bool,
    previous: PreviousWindowRight,
    granule: Option<u64>,
}
impl VorbisState {
    fn new(active: usize) -> Self {
        Self {
            active,
            headers_left: 3,
            ended: false,
            previous: PreviousWindowRight::new(),
            granule: Some(0),
        }
    }
}

pub struct Vorbis {
    packets: Packets,
    links: Box<[Link]>,
    first: usize,
    state: VorbisState,
    samples: Vec<i16>,
    length: u64,
    format: PcmFormat,
}
impl Vorbis {
    /// Parses all chained setup headers at load. Packet DSP remains in the
    /// existing decoder; this API does not yet promise allocation-free reads.
    pub fn load(
        length: u64,
        mut read: impl FnMut(u64, &mut [u8]) -> Result<(), FormatError>,
    ) -> Result<Self, FormatError> {
        let plan = Plan::inspect(length, &mut read)?;
        let mut packets = Packets::load(plan)?;
        let mut pending: Vec<Pending> = Vec::new();
        while let Some(packet) = packets.next(length, &mut read)? {
            if packet.first_in_stream && packet.data.starts_with(b"\x01vorbis") {
                pending.push(Pending {
                    serial: packet.serial,
                    ident: read_header_ident(packet.data).map_err(|_| FormatError::InvalidValue)?,
                    comment: None,
                    setup: None,
                });
                continue;
            }
            let Some(link) = pending.iter_mut().find(|link| link.serial == packet.serial) else {
                continue;
            };
            if link.comment.is_none() {
                link.comment =
                    Some(read_header_comment(packet.data).map_err(|_| FormatError::InvalidValue)?);
            } else if link.setup.is_none() {
                link.setup = Some(
                    read_header_setup(
                        packet.data,
                        link.ident.audio_channels,
                        (link.ident.blocksize_0, link.ident.blocksize_1),
                    )
                    .map_err(|_| FormatError::InvalidValue)?,
                );
            }
        }
        let first_serial = pending.first().ok_or(FormatError::Unsupported)?.serial;
        let mut links = pending
            .into_iter()
            .map(|link| {
                Ok(Link {
                    serial: link.serial,
                    ident: link.ident,
                    comment: link.comment.ok_or(FormatError::Truncated)?,
                    setup: link.setup.ok_or(FormatError::Truncated)?,
                })
            })
            .collect::<Result<Vec<_>, FormatError>>()?;
        links.sort_unstable_by_key(|link| link.serial);
        let first = links
            .binary_search_by_key(&first_serial, |link| link.serial)
            .map_err(|_| FormatError::InvalidValue)?;
        let format = Self::pcm_format(&links[first].ident)?;
        packets.rewind();
        Ok(Self {
            packets,
            links: links.into_boxed_slice(),
            first,
            state: VorbisState::new(first),
            samples: Vec::new(),
            length,
            format,
        })
    }
    fn pcm_format(ident: &IdentHeader) -> Result<PcmFormat, FormatError> {
        Ok(PcmFormat {
            rate: NonZeroU32::new(ident.audio_sample_rate).ok_or(FormatError::InvalidValue)?,
            channels: match ident.audio_channels {
                1 => PcmChannels::Mono,
                2 => PcmChannels::Stereo,
                _ => return Err(FormatError::Unsupported),
            },
            width: PcmWidth::Short,
        })
    }
    pub fn format(&self) -> PcmFormat {
        self.format
    }
    pub fn comments(&self) -> &CommentHeader {
        &self.links[self.state.active].comment
    }
    pub fn logical_streams(&self) -> usize {
        self.links.len()
    }
    pub fn source_position(&self) -> u64 {
        self.packets.position()
    }
    pub fn rewind(&mut self) {
        self.packets.rewind();
        self.state = VorbisState::new(self.first);
        self.samples.clear();
    }
    /// Returns the next nonempty decoded packet, borrowed until another read
    /// or rewind. The callback reads only from the already selected VFS source.
    pub fn read_packet(
        &mut self,
        mut read: impl FnMut(u64, &mut [u8]) -> Result<(), FormatError>,
    ) -> Result<Option<&[i16]>, FormatError> {
        while let Some(packet) = self.packets.next(self.length, &mut read)? {
            if packet.serial != self.links[self.state.active].serial {
                if !self.state.ended || !packet.first_in_stream {
                    continue;
                }
                let Ok(active) = self
                    .links
                    .binary_search_by_key(&packet.serial, |link| link.serial)
                else {
                    continue;
                };
                if Self::pcm_format(&self.links[active].ident)? != self.format {
                    return Err(FormatError::Unsupported);
                }
                self.state = VorbisState::new(active);
            }
            if self.state.headers_left != 0 {
                self.state.headers_left -= 1;
                continue;
            }
            let link = &self.links[self.state.active];
            let mut decoded: InterleavedSamples<i16> = read_audio_packet_generic(
                &link.ident,
                &link.setup,
                packet.data,
                &mut self.state.previous,
            )
            .map_err(|_| FormatError::InvalidValue)?;
            if packet.last_in_stream
                && let (Some(before), Some(end)) = (self.state.granule, packet.granule)
            {
                let frames = usize::try_from(end.saturating_sub(before)).unwrap_or(usize::MAX);
                decoded
                    .samples
                    .truncate(frames.saturating_mul(self.format.channels as usize));
            }
            if decoded.samples.len() % self.format.channels as usize != 0 {
                return Err(FormatError::InvalidRange);
            }
            if packet.last_in_page {
                self.state.granule = packet.granule;
            } else if let Some(before) = &mut self.state.granule {
                *before = before
                    .checked_add((decoded.samples.len() / self.format.channels as usize) as u64)
                    .ok_or(FormatError::InvalidRange)?;
            }
            self.state.ended = packet.last_in_stream;
            if decoded.samples.is_empty() {
                continue;
            }
            self.samples = decoded.samples;
            return Ok(Some(&self.samples));
        }
        Ok(None)
    }
    pub fn read_slice(&mut self, bytes: &[u8]) -> Result<Option<&[i16]>, FormatError> {
        self.read_packet(|at, out| ogg::slice_read(bytes, at, out))
    }
}
