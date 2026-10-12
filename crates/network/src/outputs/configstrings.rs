//! Native configstring payloads and Q3's existing reliable-command syntax.
use super::{Error, MessageError, Protocol};
use crate::message::{Encoding, ErrorKind, Writer};
use qa_core::{configstrings::ConfigLayout, text::FixedText};
use std::fmt::Write;

// Q3 sv_init.c uses MAX_STRING_CHARS - 24 and copies one fewer payload byte.
const CONFIG_CHUNK_BYTES: usize = 999;
const BIG_CONFIG_BYTES: usize = 8192;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ConfigString<'a> {
    pub index: u32,
    pub text: &'a [u8],
}

pub fn configstring(
    protocol: Protocol,
    index: u32,
    text: &[u8],
    output: &mut [u8],
) -> Result<usize, Error> {
    if protocol == Protocol::Quake3_68 {
        if text.len() > CONFIG_CHUNK_BYTES {
            return Err(Error::Unsupported);
        }
        return Ok(config_command(index, text, 0, output)?.length);
    }
    if protocol == Protocol::QuakeWorld28 {
        return qw_configstring(index, text, output);
    }
    if !matches!(protocol, Protocol::Quake2_34 | Protocol::Quake2Repro1038) {
        return Err(Error::Unsupported);
    }
    let index = u16::try_from(index).map_err(|_| {
        Error::Message(MessageError {
            byte: 1,
            kind: ErrorKind::Width,
        })
    })?;
    if text.contains(&0) {
        return Err(Error::Opcode);
    }
    let need = text
        .len()
        .checked_add(4)
        .ok_or(Error::Message(MessageError {
            byte: 3,
            kind: ErrorKind::Capacity,
        }))?;
    let size = output.len();
    let bytes = output.get_mut(..need).ok_or(Error::Message(MessageError {
        byte: size,
        kind: ErrorKind::Capacity,
    }))?;
    let mut writer = Writer::new(bytes, Encoding::Bytes);
    writer.write_bits(13, 8)?;
    writer.write_bits(u32::from(index), 16)?;
    writer.write_data(text)?;
    writer.write_bits(0, 8)?;
    Ok(writer.size())
}

pub fn read_configstring(
    protocol: Protocol,
    bytes: &[u8],
) -> Result<(ConfigString<'_>, usize), Error> {
    if protocol == Protocol::QuakeWorld28 {
        let header = bytes.get(..2).ok_or(Error::Truncated)?;
        if header[0] != 12 {
            return Err(Error::Opcode);
        }
        let index = ConfigLayout::QUAKEWORLD
            .style_index(usize::from(header[1]))
            .ok_or(Error::Opcode)? as u32;
        let text = qa_core::text::terminated(&bytes[2..]).ok_or(Error::Truncated)?;
        if text.len()
            > protocol
                .config_capacity(index as usize)
                .ok_or(Error::Unsupported)?
        {
            return Err(Error::Unsupported);
        }
        return Ok((ConfigString { index, text }, text.len() + 3));
    }
    if !matches!(protocol, Protocol::Quake2_34 | Protocol::Quake2Repro1038) {
        return Err(Error::Unsupported);
    }
    let header = bytes.get(..3).ok_or(Error::Truncated)?;
    if header[0] != 13 {
        return Err(Error::Opcode);
    }
    let index = u16::from_le_bytes([header[1], header[2]]).into();
    let text = qa_core::text::terminated(&bytes[3..]).ok_or(Error::Truncated)?;
    Ok((ConfigString { index, text }, text.len() + 4))
}

/// QW retail list chunks contain several consecutive resource rows. Borrow
/// and publish each accepted row in wire order, without a second resource list.
pub fn read_configstrings<'a>(
    protocol: Protocol,
    bytes: &'a [u8],
    mut consume: impl FnMut(ConfigString<'a>),
) -> Result<usize, Error> {
    if protocol != Protocol::QuakeWorld28 || !matches!(bytes.first(), Some(45 | 46)) {
        let (row, length) = read_configstring(protocol, bytes)?;
        consume(row);
        return Ok(length);
    }
    let header = bytes.get(..2).ok_or(Error::Truncated)?;
    let range = ConfigLayout::QUAKEWORLD.resources[usize::from(header[0] == 46)];
    let mut ordinal = u32::from(header[1]);
    let mut at = 2;
    loop {
        let text = qa_core::text::terminated(&bytes[at..]).ok_or(Error::Truncated)?;
        at += text.len() + 1;
        if text.is_empty() {
            // Original CL_Parse{Model,Sound}list reads the continuation byte
            // after the empty terminator, even on the final zero continuation.
            bytes.get(at).ok_or(Error::Truncated)?;
            return Ok(at + 1);
        }
        ordinal += 1;
        let index = range.get(ordinal as usize).ok_or(Error::Unsupported)? as u32;
        if text.len()
            > protocol
                .config_capacity(index as usize)
                .ok_or(Error::Unsupported)?
        {
            return Err(Error::Unsupported);
        }
        consume(ConfigString { index, text });
    }
}

fn qw_configstring(index: u32, text: &[u8], output: &mut [u8]) -> Result<usize, Error> {
    let layout = ConfigLayout::QUAKEWORLD;
    let index = index as usize;
    if text.contains(&0)
        || text.len()
            > Protocol::QuakeWorld28
                .config_capacity(index)
                .ok_or(Error::Unsupported)?
    {
        return Err(Error::Unsupported);
    }
    let (opcode, ordinal, list) = if layout.lights.contains(index) {
        (12, index - layout.lights.first as usize, false)
    } else {
        let (kind, range) = layout.resources[..2]
            .iter()
            .copied()
            .enumerate()
            .find(|(_, range)| range.contains(index))
            .ok_or(Error::Unsupported)?;
        let ordinal = index - range.first as usize;
        // A zero/empty resource is a native list terminator, not a row update.
        if ordinal == 0 || text.is_empty() {
            return Err(Error::Unsupported);
        }
        (45 + kind as u32, ordinal - 1, true)
    };
    let need = text.len() + 3 + 2 * usize::from(list);
    let capacity = output.len();
    let bytes = output.get_mut(..need).ok_or(Error::Message(MessageError {
        byte: capacity,
        kind: ErrorKind::Capacity,
    }))?;
    let mut writer = Writer::new(bytes, Encoding::Bytes);
    writer.write_bits(opcode, 8)?;
    writer.write_bits(ordinal as u32, 8)?;
    writer.write_data(text)?;
    writer.write_bits(0, 8)?;
    if list {
        writer.write_bits(0, 8)?; // empty resource terminates this chunk
        writer.write_bits(0, 8)?; // no further native list chunk requested
    }
    Ok(writer.size())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ConfigChunk {
    pub length: usize,
    pub next: usize,
    pub complete: bool,
}
/// `sent` belongs to the retained output record. Queue every generated command
/// on the existing Q3 channel and retire the record on the final native receipt.
pub fn config_command(
    index: u32,
    text: &[u8],
    sent: usize,
    output: &mut [u8],
) -> Result<ConfigChunk, Error> {
    if index as usize >= ConfigLayout::QUAKE3.count || text.contains(&0) || sent > text.len() {
        return Err(Error::Opcode);
    }
    if text.len()
        > ConfigLayout::QUAKE3
            .capacity(index as usize)
            .ok_or(Error::Opcode)?
    {
        return Err(Error::Message(MessageError {
            byte: text.len(),
            kind: ErrorKind::Capacity,
        }));
    }
    let chunked = text.len() > CONFIG_CHUNK_BYTES;
    if chunked {
        let prefix = config_prefix("cs", index)?;
        if text.len() + prefix.as_bytes().len() + 1 >= BIG_CONFIG_BYTES {
            return Err(Error::Message(MessageError {
                byte: text.len(),
                kind: ErrorKind::Capacity,
            }));
        }
    }
    if (!chunked && sent != 0)
        || (chunked && (sent == text.len() || sent % CONFIG_CHUNK_BYTES != 0))
    {
        return Err(Error::Opcode);
    }
    let remaining = text.len() - sent;
    let count = if chunked {
        remaining.min(CONFIG_CHUNK_BYTES)
    } else {
        remaining
    };
    let verb = if !chunked {
        "cs"
    } else if sent == 0 {
        "bcs0"
    } else if remaining <= CONFIG_CHUNK_BYTES {
        "bcs2"
    } else {
        "bcs1"
    };
    let prefix = config_prefix(verb, index)?;
    let need = prefix.as_bytes().len() + count + 3;
    let size = output.len();
    let out = output.get_mut(..need).ok_or(Error::Message(MessageError {
        byte: size,
        kind: ErrorKind::Capacity,
    }))?;
    let first = prefix.as_bytes().len();
    out[..first].copy_from_slice(prefix.as_bytes());
    out[first..first + count].copy_from_slice(&text[sent..sent + count]);
    out[need - 3..].copy_from_slice(b"\"\n\0");
    Ok(ConfigChunk {
        length: need,
        next: sent + count,
        complete: sent + count == text.len(),
    })
}

/// Bounded bcs0/1/2 assembly; ordinary commands borrow their incoming payload.
/// This is native reliable-message decoding, not a second command queue.
pub struct ConfigCommandReader {
    text: Box<[u8]>,
    length: usize,
    index: Option<u32>,
}
impl ConfigCommandReader {
    pub fn load() -> Self {
        Self {
            text: vec![0; BIG_CONFIG_BYTES].into_boxed_slice(),
            length: 0,
            index: None,
        }
    }
    pub fn reset(&mut self) {
        self.length = 0;
        self.index = None;
    }
    pub fn read<'a>(&'a mut self, command: &'a [u8]) -> Result<Option<ConfigString<'a>>, Error> {
        let command = qa_core::text::until_nul(command);
        let Some(first) = command.iter().position(|b| b.is_ascii_whitespace()) else {
            return Ok(None);
        };
        let verb = &command[..first];
        if !matches!(verb, b"cs" | b"bcs0" | b"bcs1" | b"bcs2") {
            return Ok(None);
        }
        let rest = trim(&command[first..]);
        let (index, used) = qa_core::text::integer_prefix(rest);
        if used == 0
            || index < 0
            || index as usize >= ConfigLayout::QUAKE3.count
            || rest.get(used).is_some_and(|b| !b.is_ascii_whitespace())
        {
            self.reset();
            return Err(Error::Opcode);
        }
        let index = index as u32;
        let value = trim(&rest[used..]);
        let text = if let Some(quoted) = value.strip_prefix(b"\"") {
            let end = quoted
                .iter()
                .position(|&b| b == b'"')
                .ok_or(Error::Truncated)?;
            &quoted[..end]
        } else {
            let end = value
                .iter()
                .position(|b| b.is_ascii_whitespace())
                .unwrap_or(value.len());
            &value[..end]
        };
        if verb == b"cs" {
            return Ok(Some(ConfigString { index, text }));
        }
        if verb == b"bcs0" {
            self.reset();
            self.index = Some(index);
        } else if self.index != Some(index) {
            self.reset();
            return Err(Error::Opcode);
        }
        let next = self
            .length
            .checked_add(text.len())
            .ok_or(Error::Truncated)?;
        // Original BIG_INFO_STRING also includes `cs <index> "` and, on bcs2,
        // the closing quote. Preserve that boundary rather than silently crop.
        let prefix = config_prefix("cs", index)?;
        if next + prefix.as_bytes().len() + usize::from(verb == b"bcs2") >= self.text.len() {
            self.reset();
            return Err(Error::Message(MessageError {
                byte: next,
                kind: ErrorKind::Capacity,
            }));
        }
        self.text[self.length..next].copy_from_slice(text);
        self.length = next;
        if verb != b"bcs2" {
            return Ok(None);
        }
        self.index = None;
        Ok(Some(ConfigString {
            index,
            text: &self.text[..self.length],
        }))
    }
}
fn trim(bytes: &[u8]) -> &[u8] {
    &bytes[bytes
        .iter()
        .position(|b| !b.is_ascii_whitespace())
        .unwrap_or(bytes.len())..]
}
fn config_prefix(verb: &str, index: u32) -> Result<FixedText<32>, Error> {
    let mut prefix = FixedText::default();
    write!(&mut prefix, "{verb} {index} \"").map_err(|_| Error::Opcode)?;
    Ok(prefix)
}
