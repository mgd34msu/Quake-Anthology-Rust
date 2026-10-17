//! Borrowed native file blocks. Transfer ownership and filesystem policy stay
//! with app callers; the packet decoder retains no extra byte queue.
use crate::{
    commands::packet::Protocol,
    message::{Encoding, Error as MessageError, ErrorKind, Reader, StringPolicy, Writer},
};

pub type Chunk<'a> = qa_core::primitives::FileChunk<&'a [u8]>;

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
            Chunk::Sequential {
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
        Chunk::Sequential {
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
    let encoding = if protocol == Protocol::Quake3_68 {
        Encoding::Q3
    } else {
        Encoding::Bytes
    };
    let mut writer = Writer::new(output, encoding);
    write_service(protocol, chunk, &mut writer)?;
    Ok(writer.size())
}

/// Append to the connection's existing MSG stream; Q3 shares Huffman state with
/// the surrounding server commands, snapshots and svc_EOF.
pub fn write_service(
    protocol: Protocol,
    chunk: Chunk<'_>,
    writer: &mut Writer<'_>,
) -> Result<(), Error> {
    match chunk {
        Chunk::Sequential { percent, bytes, .. } => {
            let policy = sequential(protocol).ok_or(Error::Unsupported)?;
            let size = if let Some(bytes) = bytes {
                if bytes.len() > policy.block_bytes {
                    return Err(Error::Size);
                }
                if percent > 100 {
                    return Err(Error::Percent);
                }
                bytes.len() as u16
            } else {
                u16::MAX
            };
            writer.write_bits(u32::from(policy.opcode), 8)?;
            writer.write_bits(u32::from(size), 16)?;
            writer.write_bits(u32::from(percent), 8)?;
            if let Some(bytes) = bytes {
                writer.write_data(bytes)?;
            }
        }
        Chunk::Block {
            number,
            total,
            bytes,
        } => {
            if protocol != Protocol::Quake3_68 {
                return Err(Error::Unsupported);
            }
            if (number == 0) != total.is_some()
                || total.is_some_and(|total| total > i32::MAX as u32)
                || bytes.len() > crate::commands::packet::Q3_MESSAGE_BYTES
            {
                return Err(Error::Size);
            }
            writer.write_bits(6, 8)?;
            writer.write_bits(u32::from(number as u16), 16)?;
            if let Some(total) = total {
                writer.write_bits(total, 32)?;
            }
            writer.write_bits(bytes.len() as u32, 16)?;
            writer.write_data(bytes)?;
        }
        Chunk::Rejected { size, message } => {
            if protocol != Protocol::Quake3_68 {
                return Err(Error::Unsupported);
            }
            if size >= 0 {
                return Err(Error::Size);
            }
            writer.write_bits(6, 8)?;
            writer.write_bits(0, 16)?;
            writer.write_bits(size as u32, 32)?;
            writer.write_string(StringPolicy::Q3, message)?;
        }
    }
    Ok(())
}

/// Q3's decoded bytes borrow the connection's load-sized scratch until the
/// synchronous consumer returns. They never create a second packet queue.
pub(crate) fn read_block<'a>(
    reader: &mut Reader<'_>,
    scratch: &'a mut [u8],
) -> Result<Chunk<'a>, Error> {
    let number = reader.read_signed(16)? as i16;
    let total = if number == 0 {
        let size = reader.read_signed(32)?;
        if size < 0 {
            let limit = scratch.len().min(crate::message::COMMAND_STRING_BYTES);
            let message = reader.read_string_into(StringPolicy::Q3, &mut scratch[..limit])?;
            return Ok(Chunk::Rejected { size, message });
        }
        Some(size as u32)
    } else {
        None
    };
    let length = reader.read_signed(16)?;
    if length < 0 || length as usize > crate::commands::packet::Q3_MESSAGE_BYTES {
        return Err(Error::Size);
    }
    let bytes = scratch.get_mut(..length as usize).ok_or(Error::Size)?;
    reader.read_data(bytes)?;
    Ok(Chunk::Block {
        number,
        total,
        bytes,
    })
}

/// Original Q3 base and Team Arena pak0..pak8 cannot be autodownloaded.
/// The result names the native product for its rejection message.
pub fn q3_stock_package(name: &[u8]) -> Option<&'static str> {
    let mut normalized = [0; 64];
    if name.len() > normalized.len() {
        return None;
    }
    for (to, from) in normalized.iter_mut().zip(name) {
        *to = if *from == b':' {
            b'/'
        } else {
            qa_core::names::path_byte(*from)
        };
    }
    let name = &normalized[..name.len()];
    for product in ["baseq3", "missionpack"] {
        let Some(name) = name
            .strip_prefix(product.as_bytes())
            .and_then(|name| name.strip_prefix(b"/pak"))
        else {
            continue;
        };
        if matches!(name, [b'0'..=b'8'] | [b'0'..=b'8', b'.', b'p', b'k', b'3']) {
            return Some(product);
        }
    }
    None
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
    if qa_formats::archive::ArchiveKind::from_path(name).is_some() {
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
