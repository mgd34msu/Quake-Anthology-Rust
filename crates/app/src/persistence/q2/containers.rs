//! Quake II engine save metadata ported from `src/persistence/q2-containers.ts`.
//!
//! Engine-owned metadata from `sv_ccmds.c` and q2repro `src/server/save.c`:
//! classic fixed-width `server.ssv`/`level.sv2` pairs plus the rerelease
//! versioned containers. Reads and writes use the shared
//! [`qa_core::binary`] cursor over borrowed bytes.

use qa_core::binary::{BinaryReader, BinaryWriter};
use qa_world::save::bytes::{read_c_string, read_fixed, write_c_string, write_fixed};

use super::super::PersistenceError;

/// Saved console variable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SavedCvar {
    /// Name.
    pub name: String,
    /// Value.
    pub value: String,
}

/// Classic `server.ssv` metadata.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Q2ClassicServerSave {
    /// Comment (32 bytes).
    pub comment: String,
    /// Map command (128 bytes).
    pub map_command: String,
    /// Saved cvars.
    pub cvars: Vec<SavedCvar>,
}

/// Rerelease `server.ssv` metadata.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Q2RereleaseServerSave {
    /// Comment.
    pub comment: String,
    /// Map command.
    pub map_command: String,
    /// Saved cvars.
    pub cvars: Vec<SavedCvar>,
    /// Save timestamp.
    pub timestamp: u64,
    /// Save kind (0, 1, or 2).
    pub kind: u8,
}

/// Level metadata: sparse configstrings plus portal bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Q2LevelMetadata {
    /// Non-empty configstrings by index.
    pub configstrings: Vec<(u32, String)>,
    /// Portal bytes.
    pub portal_bytes: Vec<u8>,
}

/// Default classic configstring count.
pub const CLASSIC_CONFIGSTRING_COUNT: usize = 2080;
/// Default classic configstring width.
pub const CLASSIC_CONFIGSTRING_WIDTH: usize = 64;

const SSV2: u32 = 0x3255_5353;
const SAV2: u32 = 0x3256_4153;

fn save_error(message: &str) -> PersistenceError {
    PersistenceError::BadSave(format!("q2-rerelease: {message}"))
}

/// Decode a classic `server.ssv`.
pub fn decode_q2_classic_server(bytes: &[u8]) -> Result<Q2ClassicServerSave, PersistenceError> {
    let mut reader = BinaryReader::new(bytes, "q2/server.ssv");
    let comment = read_fixed(&mut reader, 32)?;
    let map_command = read_fixed(&mut reader, 128)?;
    let mut cvars = Vec::new();
    while reader.remaining() > 0 {
        cvars.push(SavedCvar {
            name: read_fixed(&mut reader, 128)?,
            value: read_fixed(&mut reader, 128)?,
        });
    }
    Ok(Q2ClassicServerSave {
        comment,
        map_command,
        cvars,
    })
}

/// Encode a classic `server.ssv`.
pub fn encode_q2_classic_server(save: &Q2ClassicServerSave) -> Result<Vec<u8>, PersistenceError> {
    let mut writer = BinaryWriter::new(160 + save.cvars.len() * 256);
    write_fixed(&mut writer, &save.comment, 32)?;
    write_fixed(&mut writer, &save.map_command, 128)?;
    for cvar in &save.cvars {
        write_fixed(&mut writer, &cvar.name, 128)?;
        write_fixed(&mut writer, &cvar.value, 128)?;
    }
    Ok(writer.finish())
}

/// Decode classic `level.sv2` metadata.
pub fn decode_q2_classic_level_metadata(
    bytes: &[u8],
    configstring_count: usize,
    width: usize,
) -> Result<Q2LevelMetadata, PersistenceError> {
    let mut reader = BinaryReader::new(bytes, "q2/level.sv2");
    let mut configstrings = Vec::new();
    for index in 0..configstring_count {
        let value = read_fixed(&mut reader, width)?;
        if !value.is_empty() {
            #[allow(clippy::cast_possible_truncation)]
            configstrings.push((index as u32, value));
        }
    }
    let portal_bytes = reader.bytes(reader.remaining())?;
    Ok(Q2LevelMetadata {
        configstrings,
        portal_bytes,
    })
}

/// Encode classic `level.sv2` metadata.
pub fn encode_q2_classic_level_metadata(
    save: &Q2LevelMetadata,
    configstring_count: usize,
    width: usize,
) -> Result<Vec<u8>, PersistenceError> {
    let mut writer = BinaryWriter::new(configstring_count * width + save.portal_bytes.len());
    let values: std::collections::HashMap<u32, &str> = save
        .configstrings
        .iter()
        .map(|(index, value)| (*index, value.as_str()))
        .collect();
    for index in 0..configstring_count {
        #[allow(clippy::cast_possible_truncation)]
        let value = values.get(&(index as u32)).copied().unwrap_or("");
        write_fixed(&mut writer, value, width)?;
    }
    writer.bytes(&save.portal_bytes)?;
    Ok(writer.finish())
}

/// Decode a rerelease `server.ssv`.
pub fn decode_q2_rerelease_server(bytes: &[u8]) -> Result<Q2RereleaseServerSave, PersistenceError> {
    let mut reader = BinaryReader::new(bytes, "q2-rerelease/server.ssv");
    if reader.u32()? != SSV2 || reader.u32()? != 1 {
        return Err(save_error("unsupported server save version"));
    }
    let low = reader.u32()?;
    let high = reader.u32()?;
    let timestamp = (u64::from(high) << 32) | u64::from(low);
    let kind = reader.u8()?;
    if kind > 2 {
        return Err(save_error("unknown save type"));
    }
    let comment = read_c_string(&mut reader)?;
    let map_command = read_c_string(&mut reader)?;
    let mut cvars = Vec::new();
    loop {
        let name = read_c_string(&mut reader)?;
        if name.is_empty() {
            break;
        }
        cvars.push(SavedCvar {
            name,
            value: read_c_string(&mut reader)?,
        });
    }
    if reader.remaining() != 0 {
        return Err(save_error("trailing server metadata"));
    }
    Ok(Q2RereleaseServerSave {
        comment,
        map_command,
        cvars,
        timestamp,
        kind,
    })
}

/// Encode a rerelease `server.ssv`.
pub fn encode_q2_rerelease_server(save: &Q2RereleaseServerSave) -> Result<Vec<u8>, PersistenceError> {
    if save.kind > 2 {
        return Err(save_error("unknown save type"));
    }
    let size = 20
        + save.comment.len()
        + save.map_command.len()
        + save
            .cvars
            .iter()
            .map(|cvar| cvar.name.len() + cvar.value.len() + 2)
            .sum::<usize>();
    let mut writer = BinaryWriter::new(size);
    writer.u32(SSV2)?;
    writer.u32(1)?;
    #[allow(clippy::cast_possible_truncation)]
    writer.u32(save.timestamp as u32)?;
    #[allow(clippy::cast_possible_truncation)]
    writer.u32((save.timestamp >> 32) as u32)?;
    writer.u8(save.kind)?;
    write_c_string(&mut writer, &save.comment)?;
    write_c_string(&mut writer, &save.map_command)?;
    for cvar in &save.cvars {
        write_c_string(&mut writer, &cvar.name)?;
        write_c_string(&mut writer, &cvar.value)?;
    }
    write_c_string(&mut writer, "")?;
    Ok(writer.finish())
}

/// Decode rerelease `level.sv2` metadata.
pub fn decode_q2_rerelease_level_metadata(
    bytes: &[u8],
    configstring_end: u32,
) -> Result<Q2LevelMetadata, PersistenceError> {
    let mut reader = BinaryReader::new(bytes, "q2-rerelease/level.sv2");
    if reader.u32()? != SAV2 || reader.u32()? != 1 {
        return Err(save_error("unsupported level save version"));
    }
    let mut configstrings = Vec::new();
    loop {
        let index = u32::from(reader.u16()?);
        if index == configstring_end {
            break;
        }
        if index > configstring_end {
            return Err(save_error("configstring outside source range"));
        }
        configstrings.push((index, read_c_string(&mut reader)?));
    }
    let portal_length = usize::from(reader.u8()?);
    let portal_bytes = reader.bytes(portal_length)?;
    if reader.remaining() != 0 {
        return Err(save_error("trailing level metadata"));
    }
    Ok(Q2LevelMetadata {
        configstrings,
        portal_bytes,
    })
}

/// Encode rerelease `level.sv2` metadata.
pub fn encode_q2_rerelease_level_metadata(
    save: &Q2LevelMetadata,
    configstring_end: u32,
) -> Result<Vec<u8>, PersistenceError> {
    for (index, _) in &save.configstrings {
        if *index >= configstring_end {
            return Err(save_error("configstring outside source range"));
        }
    }
    if save.portal_bytes.len() > 255 {
        return Err(save_error("trailing level metadata"));
    }
    let size = 11
        + save.portal_bytes.len()
        + save
            .configstrings
            .iter()
            .map(|(_, value)| value.len() + 3)
            .sum::<usize>();
    let mut writer = BinaryWriter::new(size);
    writer.u32(SAV2)?;
    writer.u32(1)?;
    for (index, value) in &save.configstrings {
        #[allow(clippy::cast_possible_truncation)]
        writer.u16(*index as u16)?;
        write_c_string(&mut writer, value)?;
    }
    #[allow(clippy::cast_possible_truncation)]
    writer.u16(configstring_end as u16)?;
    #[allow(clippy::cast_possible_truncation)]
    writer.u8(save.portal_bytes.len() as u8)?;
    writer.bytes(&save.portal_bytes)?;
    Ok(writer.finish())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classic_containers_round_trip() {
        let server = Q2ClassicServerSave {
            comment: "base1".to_string(),
            map_command: "map base1".to_string(),
            cvars: vec![SavedCvar {
                name: "skill".to_string(),
                value: "2".to_string(),
            }],
        };
        let bytes = encode_q2_classic_server(&server).unwrap();
        assert_eq!(bytes.len(), 160 + 256);
        assert_eq!(decode_q2_classic_server(&bytes).unwrap(), server);
        let level = Q2LevelMetadata {
            configstrings: vec![(0, "models/player.md2".to_string()), (37, "2".to_string())],
            portal_bytes: vec![1, 2, 3],
        };
        let bytes =
            encode_q2_classic_level_metadata(&level, CLASSIC_CONFIGSTRING_COUNT, CLASSIC_CONFIGSTRING_WIDTH).unwrap();
        assert_eq!(
            decode_q2_classic_level_metadata(&bytes, CLASSIC_CONFIGSTRING_COUNT, CLASSIC_CONFIGSTRING_WIDTH).unwrap(),
            level
        );
    }

    #[test]
    fn rerelease_containers_round_trip() {
        let server = Q2RereleaseServerSave {
            comment: "autosave".to_string(),
            map_command: "map base2".to_string(),
            cvars: vec![SavedCvar {
                name: "coop".to_string(),
                value: "1".to_string(),
            }],
            timestamp: 0x0102_0304_0506_0708,
            kind: 1,
        };
        let bytes = encode_q2_rerelease_server(&server).unwrap();
        assert_eq!(decode_q2_rerelease_server(&bytes).unwrap(), server);
        assert!(decode_q2_rerelease_server(&bytes[..bytes.len() - 1]).is_err());
        let level = Q2LevelMetadata {
            configstrings: vec![(5, "x".to_string())],
            portal_bytes: vec![9, 9],
        };
        let bytes = encode_q2_rerelease_level_metadata(&level, 2080).unwrap();
        assert_eq!(decode_q2_rerelease_level_metadata(&bytes, 2080).unwrap(), level);
        let bad = Q2LevelMetadata {
            configstrings: vec![(9999, "x".to_string())],
            portal_bytes: Vec::new(),
        };
        assert!(encode_q2_rerelease_level_metadata(&bad, 2080).is_err());
    }
}
