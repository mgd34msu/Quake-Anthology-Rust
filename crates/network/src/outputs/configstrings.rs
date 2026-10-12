//! Native configstring payloads and Q3's existing reliable-command syntax.
use super::{Error, MessageError, Protocol, read_text, string_error};
use crate::message::{Encoding, ErrorKind, Reader, Writer};
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
    let text = protocol.string_policy().payload(text);
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
    writer.write_string(protocol.string_policy(), text)?;
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
        let mut reader = Reader::new(&bytes[2..], Encoding::Bytes);
        let text = read_text(protocol, &mut reader)?;
        if text.len()
            > protocol
                .config_capacity(index as usize)
                .ok_or(Error::Unsupported)?
        {
            return Err(Error::Unsupported);
        }
        return Ok((ConfigString { index, text }, reader.byte_position() + 2));
    }
    if !matches!(protocol, Protocol::Quake2_34 | Protocol::Quake2Repro1038) {
        return Err(Error::Unsupported);
    }
    let header = bytes.get(..3).ok_or(Error::Truncated)?;
    if header[0] != 13 {
        return Err(Error::Opcode);
    }
    let index = u16::from_le_bytes([header[1], header[2]]).into();
    let mut reader = Reader::new(&bytes[3..], Encoding::Bytes);
    let text = read_text(protocol, &mut reader)?;
    Ok((ConfigString { index, text }, reader.byte_position() + 3))
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
    let mut reader = Reader::new(&bytes[2..], Encoding::Bytes);
    loop {
        let text = read_text(protocol, &mut reader)?;
        if text.is_empty() {
            // Original CL_Parse{Model,Sound}list reads the continuation byte
            // after the empty terminator, even on the final zero continuation.
            reader.read_bits(8).map_err(string_error)?;
            return Ok(reader.byte_position() + 2);
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
    let text = Protocol::QuakeWorld28.string_policy().payload(text);
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
    writer.write_string(Protocol::QuakeWorld28.string_policy(), text)?;
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

/// Native selection result; lexical parsing belongs to the shared tokenizer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConfigCommandStep<'a> {
    Pass,
    Pending,
    Rescan(&'a [u8]),
}

/// One native bcs0/1/2 assembly, not another command queue or tokenizer.
pub struct ConfigCommandReader {
    text: FixedText<BIG_CONFIG_BYTES>,
    active: bool,
}
impl ConfigCommandReader {
    pub fn load() -> Self {
        Self {
            text: FixedText::default(),
            active: false,
        }
    }
    pub fn reset(&mut self) {
        self.text.clear();
        self.active = false;
    }
    pub fn read<'a>(
        &'a mut self,
        verb: &[u8],
        index_text: &[u8],
        fragment: &[u8],
    ) -> Result<ConfigCommandStep<'a>, Error> {
        if !matches!(verb, b"bcs0" | b"bcs1" | b"bcs2") {
            return Ok(ConfigCommandStep::Pass);
        }
        if verb == b"bcs0" {
            self.reset();
            for part in [b"cs ".as_slice(), index_text, b" \"".as_slice()] {
                if self.text.append_bytes(part).is_err() {
                    self.reset();
                    return Err(Error::Opcode);
                }
            }
            self.active = true;
        } else if !self.active {
            return Err(Error::Opcode);
        }
        // Native bcs1/2 ignore argv1. Only bcs0 owns the reconstructed index.
        let next = self
            .text
            .as_bytes()
            .len()
            .checked_add(fragment.len())
            .and_then(|n| n.checked_add(usize::from(verb == b"bcs2")))
            .ok_or(Error::Truncated)?;
        if next >= BIG_CONFIG_BYTES {
            self.reset();
            return Err(Error::Message(MessageError {
                byte: next,
                kind: ErrorKind::Capacity,
            }));
        }
        if self.text.append_bytes(fragment).is_err() {
            self.reset();
            return Err(Error::Truncated);
        }
        if verb != b"bcs2" {
            return Ok(ConfigCommandStep::Pending);
        }
        self.active = false;
        self.text
            .append_bytes(b"\"")
            .map_err(|_| Error::Truncated)?;
        Ok(ConfigCommandStep::Rescan(self.text.as_bytes()))
    }
}
fn config_prefix(verb: &str, index: u32) -> Result<FixedText<32>, Error> {
    let mut prefix = FixedText::default();
    write!(&mut prefix, "{verb} {index} \"").map_err(|_| Error::Opcode)?;
    Ok(prefix)
}
