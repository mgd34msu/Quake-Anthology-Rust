//! Borrowed native file blocks. Transfer ownership and filesystem policy stay
//! with the receiving CLIENT; the packet decoder retains no extra byte queue.
use crate::{
    commands::packet::Protocol,
    message::{Encoding, Error as MessageError, ErrorKind, Reader, Writer},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Chunk<'a> {
    pub percent: u8,
    /// Native signed size -1 means unavailable; an empty slice is a real block.
    pub bytes: Option<&'a [u8]>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Sequential {
    opcode: u8,
    block_bytes: usize,
    empty_completes: bool,
}
impl Sequential {
    pub const fn block_bytes(self) -> usize {
        self.block_bytes
    }
    pub const fn empty_completes(self) -> bool {
        self.empty_completes
    }
}

/// The two sequential native services share their framing, while retaining
/// the original SERVER block size and CLIENT empty-file completion rule.
pub const fn sequential(protocol: Protocol) -> Option<Sequential> {
    match protocol {
        Protocol::QuakeWorld28 => Some(Sequential {
            opcode: 41,
            block_bytes: 768,
            empty_completes: false,
        }),
        Protocol::Quake2_34 | Protocol::Quake2Repro1038 => Some(Sequential {
            opcode: 16,
            block_bytes: 1024,
            empty_completes: true,
        }),
        Protocol::NetQuake15 | Protocol::Quake3_68 => None,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Message(MessageError),
    Unsupported,
    Opcode,
    Size,
    Percent,
}
impl From<MessageError> for Error {
    fn from(error: MessageError) -> Self {
        Self::Message(error)
    }
}

pub fn read_chunk(protocol: Protocol, bytes: &[u8]) -> Result<(Chunk<'_>, usize), Error> {
    let policy = sequential(protocol).ok_or(Error::Unsupported)?;
    let mut reader = Reader::new(bytes, Encoding::Bytes);
    if reader.read_bits(8)? != u32::from(policy.opcode) {
        return Err(Error::Opcode);
    }
    let size = reader.read_signed(16)?;
    let percent = reader.read_bits(8)? as u8;
    if size == -1 {
        return Ok((
            Chunk {
                percent,
                bytes: None,
            },
            reader.byte_position(),
        ));
    }
    if size < 0 {
        return Err(Error::Size);
    }
    if percent > 100 {
        return Err(Error::Percent);
    }
    // Unlike the SERVER's preferred chunk size, the signed wire width is a
    // receive boundary. Larger positive blocks remain bounded by this packet.
    let first = reader.byte_position();
    let end = first + size as usize;
    let data = bytes
        .get(first..end)
        .ok_or_else(|| reader.error(ErrorKind::Truncated))?;
    Ok((
        Chunk {
            percent,
            bytes: Some(data),
        },
        end,
    ))
}

pub fn write_chunk(
    protocol: Protocol,
    chunk: Chunk<'_>,
    output: &mut [u8],
) -> Result<usize, Error> {
    let policy = sequential(protocol).ok_or(Error::Unsupported)?;
    let size = if let Some(bytes) = chunk.bytes {
        if bytes.len() > policy.block_bytes {
            return Err(Error::Size);
        }
        if chunk.percent > 100 {
            return Err(Error::Percent);
        }
        bytes.len() as u16
    } else {
        u16::MAX
    };
    let mut writer = Writer::new(output, Encoding::Bytes);
    writer.write_bits(u32::from(policy.opcode), 8)?;
    writer.write_bits(u32::from(size), 16)?;
    writer.write_bits(u32::from(chunk.percent), 8)?;
    if let Some(bytes) = chunk.bytes {
        writer.write_data(bytes)?;
    }
    Ok(writer.size())
}
