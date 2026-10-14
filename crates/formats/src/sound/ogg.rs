//! Reusable Ogg framing for sound codecs. Xiph framing version 0:
//! https://www.xiph.org/ogg/doc/framing.html
use crate::FormatError;

const HEADER: usize = 27;
const MAX_PAGE: usize = HEADER + 255 + 255 * 255;

#[derive(Clone, Copy, Default)]
struct Logical {
    serial: u32,
    next_sequence: u32,
    partial: bool,
    ended: bool,
    packets: u64,
    bytes: usize,
}
#[derive(Clone, Copy, Debug)]
pub struct Plan {
    pub streams: usize,
    pub packet_bytes: usize,
}
impl Plan {
    /// Cold inspection determines exact reusable packet-storage requirements.
    pub fn inspect(
        length: u64,
        mut read: impl FnMut(u64, &mut [u8]) -> Result<(), FormatError>,
    ) -> Result<Self, FormatError> {
        let mut page = vec![0; MAX_PAGE].into_boxed_slice();
        let mut logical: Vec<Logical> = Vec::new();
        let mut at = 0;
        let mut largest = 0;
        let mut active = 0;
        let mut streams = 0;
        while let Some(header) = page_at(length, &mut at, &mut page, &mut read)? {
            let slot = match logical
                .iter()
                .position(|state| state.serial == header.serial)
            {
                Some(_) if header.first => return Err(FormatError::InvalidValue),
                Some(slot) => slot,
                None => {
                    logical.push(Logical {
                        serial: header.serial,
                        ended: true,
                        ..Default::default()
                    });
                    logical.len() - 1
                }
            };
            let state = &mut logical[slot];
            if header.first {
                active += 1;
                streams = streams.max(active);
            }
            accept(state, header)?;
            for &lace in &page[HEADER..HEADER + header.segments] {
                state.bytes = state
                    .bytes
                    .checked_add(lace as usize)
                    .ok_or(FormatError::InvalidRange)?;
                largest = largest.max(state.bytes);
                state.partial = lace == 255;
                if !state.partial {
                    state.bytes = 0;
                    state.packets += 1;
                }
            }
            finish(state, header)?;
            if header.last {
                active -= 1;
            }
        }
        if logical.is_empty() || logical.iter().any(|state| state.partial) {
            return Err(FormatError::Truncated);
        }
        Ok(Self {
            streams,
            packet_bytes: largest.max(1),
        })
    }
}

#[derive(Clone, Copy)]
struct Page {
    first: bool,
    last: bool,
    continued: bool,
    granule: Option<u64>,
    serial: u32,
    sequence: u32,
    segments: usize,
}

fn page_at(
    length: u64,
    at: &mut u64,
    bytes: &mut [u8],
    read: &mut impl FnMut(u64, &mut [u8]) -> Result<(), FormatError>,
) -> Result<Option<Page>, FormatError> {
    if *at == length {
        return Ok(None);
    }
    if length.saturating_sub(*at) < HEADER as u64 {
        return Err(FormatError::Truncated);
    }
    read(*at, &mut bytes[..HEADER])?;
    if &bytes[..4] != b"OggS" {
        return Err(FormatError::InvalidValue);
    }
    if bytes[4] != 0 {
        return Err(FormatError::Unsupported);
    }
    if bytes[5] & !7 != 0 {
        return Err(FormatError::InvalidValue);
    }
    let segments = bytes[26] as usize;
    let header_end = HEADER + segments;
    if length - *at < header_end as u64 {
        return Err(FormatError::Truncated);
    }
    read(*at + HEADER as u64, &mut bytes[HEADER..header_end])?;
    let body = bytes[HEADER..header_end]
        .iter()
        .map(|&value| value as usize)
        .sum::<usize>();
    let end = header_end + body;
    if length - *at < end as u64 {
        return Err(FormatError::Truncated);
    }
    read(*at + header_end as u64, &mut bytes[header_end..end])?;
    let expected = word(bytes, 22);
    bytes[22..26].fill(0);
    if checksum(&bytes[..end]) != expected {
        return Err(FormatError::Checksum);
    }
    let granule = u64::from_le_bytes([
        bytes[6], bytes[7], bytes[8], bytes[9], bytes[10], bytes[11], bytes[12], bytes[13],
    ]);
    let page = Page {
        first: bytes[5] & 2 != 0,
        last: bytes[5] & 4 != 0,
        continued: bytes[5] & 1 != 0,
        granule: (granule != u64::MAX).then_some(granule),
        serial: word(bytes, 14),
        sequence: word(bytes, 18),
        segments,
    };
    *at += end as u64;
    Ok(Some(page))
}
fn word(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
}
fn accept(state: &mut Logical, page: Page) -> Result<(), FormatError> {
    if page.first {
        if !state.ended || page.continued || page.sequence != 0 {
            return Err(FormatError::InvalidValue);
        }
        *state = Logical {
            serial: page.serial,
            ..Default::default()
        };
    } else if state.ended {
        return Err(FormatError::InvalidValue);
    }
    if state.next_sequence != page.sequence || state.partial != page.continued {
        return Err(FormatError::InvalidValue);
    }
    state.next_sequence = state.next_sequence.wrapping_add(1);
    Ok(())
}
fn finish(state: &mut Logical, page: Page) -> Result<(), FormatError> {
    if page.last {
        if state.partial {
            return Err(FormatError::Truncated);
        }
        state.ended = true;
    }
    Ok(())
}

const fn crc_table() -> [u32; 256] {
    let mut table = [0; 256];
    let mut index = 0;
    while index < 256 {
        let mut value = (index as u32) << 24;
        let mut bit = 0;
        while bit < 8 {
            value = if value & 0x8000_0000 != 0 {
                (value << 1) ^ 0x04c1_1db7
            } else {
                value << 1
            };
            bit += 1;
        }
        table[index] = value;
        index += 1;
    }
    table
}
const CRC: [u32; 256] = crc_table();
/// Original Ogg page integrity checksum, never a resource identity.
pub fn checksum(bytes: &[u8]) -> u32 {
    bytes.iter().fold(0, |value, &byte| {
        (value << 8) ^ CRC[((value >> 24) as u8 ^ byte) as usize]
    })
}

#[derive(Debug)]
pub struct Packet<'a> {
    pub data: &'a [u8],
    pub serial: u32,
    pub granule: Option<u64>,
    pub first_in_stream: bool,
    pub last_in_stream: bool,
    pub last_in_page: bool,
}

pub struct Packets {
    page: Box<[u8]>,
    buffers: Box<[u8]>,
    logical: Box<[Logical]>,
    packet_bytes: usize,
    at: u64,
    current: Option<(Page, usize)>,
    segment: usize,
    body_at: usize,
    completed: Option<usize>,
}
impl Packets {
    pub fn load(plan: Plan) -> Result<Self, FormatError> {
        let bytes = plan
            .streams
            .checked_mul(plan.packet_bytes)
            .ok_or(FormatError::InvalidRange)?;
        if plan.streams == 0 || plan.packet_bytes == 0 || bytes > 256 * 1024 * 1024 {
            return Err(FormatError::InvalidRange);
        }
        Ok(Self {
            page: vec![0; MAX_PAGE].into_boxed_slice(),
            buffers: vec![0; bytes].into_boxed_slice(),
            logical: vec![
                Logical {
                    ended: true,
                    ..Default::default()
                };
                plan.streams
            ]
            .into_boxed_slice(),
            packet_bytes: plan.packet_bytes,
            at: 0,
            current: None,
            segment: 0,
            body_at: 0,
            completed: None,
        })
    }
    pub fn rewind(&mut self) {
        self.logical.fill(Logical {
            ended: true,
            ..Default::default()
        });
        self.at = 0;
        self.current = None;
        self.segment = 0;
        self.body_at = 0;
        self.completed = None;
    }
    pub fn position(&self) -> u64 {
        self.at
    }
    pub fn next_slice(&mut self, bytes: &[u8]) -> Result<Option<Packet<'_>>, FormatError> {
        self.next(bytes.len() as u64, |at, out| slice_read(bytes, at, out))
    }
    /// Returned bytes borrow this reader's storage until its next operation.
    pub fn next(
        &mut self,
        length: u64,
        mut read: impl FnMut(u64, &mut [u8]) -> Result<(), FormatError>,
    ) -> Result<Option<Packet<'_>>, FormatError> {
        if let Some(slot) = self.completed.take() {
            self.logical[slot].bytes = 0;
        }
        loop {
            if self.current.is_none() {
                let Some(page) = page_at(length, &mut self.at, &mut self.page, &mut read)? else {
                    if self.logical.iter().any(|state| state.partial) {
                        return Err(FormatError::Truncated);
                    }
                    return Ok(None);
                };
                let existing = self
                    .logical
                    .iter()
                    .position(|state| state.serial == page.serial && !state.ended);
                let slot = if page.first {
                    if existing.is_some() {
                        return Err(FormatError::InvalidValue);
                    }
                    self.logical
                        .iter()
                        .position(|state| state.ended)
                        .ok_or(FormatError::InvalidRange)?
                } else {
                    existing.ok_or(FormatError::InvalidValue)?
                };
                accept(&mut self.logical[slot], page)?;
                self.current = Some((page, slot));
                self.segment = 0;
                self.body_at = HEADER + page.segments;
            }
            let Some((page, slot)) = self.current else {
                return Err(FormatError::InvalidValue);
            };
            if self.segment == page.segments {
                finish(&mut self.logical[slot], page)?;
                self.current = None;
                continue;
            }
            let lace = self.page[HEADER + self.segment] as usize;
            self.segment += 1;
            let state = &mut self.logical[slot];
            let end = state
                .bytes
                .checked_add(lace)
                .filter(|&end| end <= self.packet_bytes)
                .ok_or(FormatError::InvalidRange)?;
            let base = slot * self.packet_bytes;
            self.buffers[base + state.bytes..base + end]
                .copy_from_slice(&self.page[self.body_at..self.body_at + lace]);
            self.body_at += lace;
            state.bytes = end;
            state.partial = lace == 255;
            if state.partial {
                continue;
            }
            let first = state.packets == 0;
            state.packets += 1;
            // The granule position applies to the last completed packet on
            // this page, even if a following packet continues on another page.
            let last_in_page = !self.page[HEADER + self.segment..HEADER + page.segments]
                .iter()
                .any(|&size| size < 255);
            let last_in_stream = page.last && self.segment == page.segments;
            self.completed = Some(slot);
            return Ok(Some(Packet {
                data: &self.buffers[base..base + end],
                serial: page.serial,
                granule: page.granule,
                first_in_stream: first,
                last_in_stream,
                last_in_page,
            }));
        }
    }
}
pub fn slice_read(bytes: &[u8], at: u64, out: &mut [u8]) -> Result<(), FormatError> {
    let at = usize::try_from(at).map_err(|_| FormatError::InvalidRange)?;
    out.copy_from_slice(crate::read::slice_at(bytes, at, out.len())?);
    Ok(())
}
