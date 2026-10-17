//! Borrowed native file blocks. Transfer ownership and filesystem policy stay
//! with app callers; the packet decoder retains no extra byte queue.
use crate::{
    commands::packet::Protocol,
    message::{Encoding, Error as MessageError, ErrorKind, Reader, Writer},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Chunk<'a> {
    pub percent: u8,
    /// Native signed size -1 means unavailable; an empty slice is a real block.
    pub bytes: Option<&'a [u8]>,
    /// DEFLATE input can advance while producing an empty decoded block.
    pub compressed: bool,
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
    Path,
    Compression,
}
impl From<MessageError> for Error {
    fn from(error: MessageError) -> Self {
        Self::Message(error)
    }
}

pub fn read_chunk(protocol: Protocol, bytes: &[u8]) -> Result<(Chunk<'_>, usize), Error> {
    let policy = sequential(protocol).ok_or(Error::Unsupported)?;
    read_chunk_service(policy.opcode, bytes)
}

pub(crate) fn read_chunk_service(opcode: u8, bytes: &[u8]) -> Result<(Chunk<'_>, usize), Error> {
    let mut reader = Reader::new(bytes, Encoding::Bytes);
    if reader.read_bits(8)? != u32::from(opcode) {
        return Err(Error::Opcode);
    }
    let size = reader.read_signed(16)?;
    let percent = reader.read_bits(8)? as u8;
    if size == -1 {
        return Ok((
            Chunk {
                percent,
                bytes: None,
                compressed: false,
            },
            reader.byte_position(),
        ));
    }
    if size < 0 {
        return Err(Error::Size);
    }
    // Unlike the SERVER's preferred chunk size, the signed wire width is a
    // receive boundary. Larger positive blocks remain bounded by this packet.
    let first = reader.byte_position();
    let end = first + size as usize;
    let data = bytes
        .get(first..end)
        .ok_or_else(|| reader.error(ErrorKind::Truncated))?;
    if percent > 100 {
        return Err(Error::Percent);
    }
    Ok((
        Chunk {
            percent,
            bytes: Some(data),
            compressed: false,
        },
        end,
    ))
}

/// Writes an ordinary service, including when forwarding decoded DEFLATE data.
/// The provenance flag never selects a compressed wire encoding.
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

/// Native downloads name a bounded relative content path, never an OS path.
pub fn validate_path(name: &[u8]) -> Result<(), Error> {
    if name.is_empty()
        || name.len() >= 64
        || !name.contains(&b'/')
        || name.windows(2).any(|pair| pair == b"..")
        || name
            .iter()
            .any(|byte| !byte.is_ascii_alphanumeric() && !b"_+./-".contains(byte))
        || name
            .split(|byte| *byte == b'/')
            .any(|part| part.is_empty() || part == b"." || part == b"..")
        || name.first() == Some(&b'.')
    {
        return Err(Error::Path);
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AssetCategory {
    Package,
    Map,
    Texture,
    Skin,
    Player,
    Model,
    Sound,
    Picture,
    Other,
}

/// Classify the already canonical content path for native permission cvars.
pub fn category(protocol: Protocol, name: &[u8]) -> AssetCategory {
    if [b".pak".as_slice(), b".pkz", b".pk3"]
        .iter()
        .any(|suffix| name.ends_with(suffix))
    {
        AssetCategory::Package
    } else {
        content_category(protocol, name)
    }
}

/// Native SERVER permissions follow the directory even for package suffixes.
pub fn content_category(protocol: Protocol, name: &[u8]) -> AssetCategory {
    if name.starts_with(b"maps/") {
        AssetCategory::Map
    } else if name.starts_with(b"textures/") || name.starts_with(b"env/") {
        AssetCategory::Texture
    } else if name.starts_with(b"skins/") {
        AssetCategory::Skin
    } else if name.starts_with(b"players/") {
        AssetCategory::Player
    } else if name.starts_with(b"models/")
        || name.starts_with(b"progs/")
        || protocol == Protocol::Quake2Repro1038 && name.starts_with(b"sprites/")
    {
        AssetCategory::Model
    } else if name.starts_with(b"sound/") {
        AssetCategory::Sound
    } else if name.starts_with(b"pics/") {
        AssetCategory::Picture
    } else {
        AssetCategory::Other
    }
}
