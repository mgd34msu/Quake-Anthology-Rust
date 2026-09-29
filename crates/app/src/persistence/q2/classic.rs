//! Original Quake II struct records ported from `src/persistence/q2-classic.ts`.
//!
//! `game/g_save.c` raw struct records with source-order appended strings.
//! Native layouts are build/ABI dependent, so the game module supplies
//! its verified [`Q2ClassicSaveLayout`]; string tails and pointer
//! references resolve through [`Q2ClassicRelocator`] on restore.

use qa_core::binary::{BinaryReader, BinaryWriter};
use qa_guest::checkpoint::ModuleIdentity;

use super::super::PersistenceError;

/// One saved field of a native struct.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Q2ClassicSaveField {
    /// Field name.
    pub name: String,
    /// Byte offset.
    pub offset: usize,
    /// Field kind.
    pub kind: Q2ClassicFieldKind,
}

/// Saved field kinds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Q2ClassicFieldKind {
    /// Appended NUL-terminated string.
    String,
    /// Entity index.
    Entity,
    /// Client index.
    Client,
    /// Item index.
    Item,
    /// Function offset.
    Function,
    /// Move offset.
    Move,
}

/// Layout of one native record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Q2ClassicRecordLayout {
    /// Struct byte length.
    pub byte_length: usize,
    /// Saved fields.
    pub fields: Vec<Q2ClassicSaveField>,
}

/// Verified native save layout from the game module.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Q2ClassicSaveLayout {
    /// Module identity.
    pub module: ModuleIdentity,
    /// Pointer width (4 or 8).
    pub pointer_bytes: u32,
    /// Game struct byte length.
    pub game_bytes: usize,
    /// Offset of the client count inside the game struct.
    pub client_count_offset: usize,
    /// Client record layout.
    pub client: Q2ClassicRecordLayout,
    /// Level record layout.
    pub level: Q2ClassicRecordLayout,
    /// Entity record layout.
    pub entity: Q2ClassicRecordLayout,
}

/// Saved string tail.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Q2ClassicString {
    /// Field name.
    pub field: String,
    /// NUL-terminated bytes (None for null).
    pub bytes: Option<Vec<u8>>,
}

/// Saved pointer reference.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Q2ClassicReference {
    /// Field name.
    pub field: String,
    /// Saved index.
    pub index: i32,
}

/// Saved native record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Q2ClassicRecord {
    /// Struct bytes (field slots hold lengths/indexes on the wire).
    pub bytes: Vec<u8>,
    /// String tails in field order.
    pub strings: Vec<Q2ClassicString>,
    /// References in field order.
    pub references: Vec<Q2ClassicReference>,
}

/// Saved `game.ssv`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Q2ClassicGameSave {
    /// Build date (16 bytes).
    pub build_date: Vec<u8>,
    /// Game struct bytes.
    pub game: Vec<u8>,
    /// Client records.
    pub clients: Vec<Q2ClassicRecord>,
}

/// Saved `level.sav`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Q2ClassicLevelSave {
    /// Function base address.
    pub function_base: u64,
    /// Level record.
    pub level: Q2ClassicRecord,
    /// Entity records by slot.
    pub entities: Vec<(i32, Q2ClassicRecord)>,
}

fn field_error(field: &str, message: &str) -> PersistenceError {
    PersistenceError::BadSave(format!("{field}: {message}"))
}

fn record_size(record: &Q2ClassicRecord) -> usize {
    record.bytes.len()
        + record
            .strings
            .iter()
            .map(|string| string.bytes.as_ref().map_or(0, Vec::len))
            .sum::<usize>()
}

fn check_field(bytes: &[u8], field: &Q2ClassicSaveField) -> Result<(), PersistenceError> {
    if field.offset.checked_add(4).is_some_and(|end| end <= bytes.len()) {
        Ok(())
    } else {
        Err(field_error(&field.name, "save field is outside its native struct"))
    }
}

fn field_i32(bytes: &[u8], field: &Q2ClassicSaveField) -> Result<i32, PersistenceError> {
    check_field(bytes, field)?;
    Ok(i32::from_le_bytes(
        bytes[field.offset..field.offset + 4].try_into().expect("checked range"),
    ))
}

fn set_field_i32(bytes: &mut [u8], field: &Q2ClassicSaveField, value: i32) -> Result<(), PersistenceError> {
    check_field(bytes, field)?;
    bytes[field.offset..field.offset + 4].copy_from_slice(&value.to_le_bytes());
    Ok(())
}

fn read_record(
    reader: &mut BinaryReader,
    source: &str,
    layout: &Q2ClassicRecordLayout,
) -> Result<Q2ClassicRecord, PersistenceError> {
    let bytes = reader.bytes(layout.byte_length)?;
    let mut strings = Vec::new();
    let mut references = Vec::new();
    for field in &layout.fields {
        let value = field_i32(&bytes, field)?;
        if field.kind == Q2ClassicFieldKind::String {
            if value == 0 {
                strings.push(Q2ClassicString {
                    field: field.name.clone(),
                    bytes: None,
                });
            } else {
                let length = usize::try_from(value).map_err(|_| {
                    PersistenceError::BadSave(format!(
                        "{source}:{}: range of {value} bytes exceeds {}-byte input",
                        reader.offset(),
                        reader.length()
                    ))
                })?;
                let string = reader.bytes(length)?;
                if string.last() != Some(&0) {
                    return Err(field_error(&field.name, "source string lacks a NUL terminator"));
                }
                strings.push(Q2ClassicString {
                    field: field.name.clone(),
                    bytes: Some(string),
                });
            }
        } else {
            references.push(Q2ClassicReference {
                field: field.name.clone(),
                index: value,
            });
        }
    }
    Ok(Q2ClassicRecord {
        bytes,
        strings,
        references,
    })
}

fn write_record(
    writer: &mut BinaryWriter,
    record: &Q2ClassicRecord,
    layout: &Q2ClassicRecordLayout,
) -> Result<(), PersistenceError> {
    if record.bytes.len() != layout.byte_length {
        return Err(field_error("q2-record", "native struct size mismatch"));
    }
    let mut bytes = record.bytes.clone();
    let mut tails: Vec<Vec<u8>> = Vec::new();
    for field in &layout.fields {
        if field.kind == Q2ClassicFieldKind::String {
            let string = record.strings.iter().find(|candidate| candidate.field == field.name);
            let Some(string) = string else {
                return Err(field_error(&field.name, "missing native saved string"));
            };
            if string.bytes.as_ref().is_some_and(|tail| tail.last() != Some(&0)) {
                return Err(field_error(&field.name, "source string lacks a NUL terminator"));
            }
            #[allow(clippy::cast_possible_wrap)]
            set_field_i32(&mut bytes, field, string.bytes.as_ref().map_or(0, Vec::len) as i32)?;
            if let Some(tail) = &string.bytes {
                tails.push(tail.clone());
            }
        } else {
            let reference = record.references.iter().find(|candidate| candidate.field == field.name);
            let Some(reference) = reference else {
                return Err(field_error(&field.name, "missing native saved reference"));
            };
            set_field_i32(&mut bytes, field, reference.index)?;
        }
    }
    writer.bytes(&bytes)?;
    for tail in &tails {
        writer.bytes(tail)?;
    }
    Ok(())
}

/// Decode a `game.ssv`.
pub fn decode_q2_classic_game(
    bytes: &[u8],
    layout: &Q2ClassicSaveLayout,
) -> Result<Q2ClassicGameSave, PersistenceError> {
    let source = format!("{}/game.ssv", layout.module.id);
    let mut reader = BinaryReader::new(bytes, &source);
    let build_date = reader.bytes(16)?;
    let game = reader.bytes(layout.game_bytes)?;
    let count_field = Q2ClassicSaveField {
        name: "game.maxclients".to_string(),
        offset: layout.client_count_offset,
        kind: Q2ClassicFieldKind::Client,
    };
    let count = field_i32(&game, &count_field)?;
    if count < 0 || count as usize > reader.remaining() / layout.client.byte_length {
        return Err(field_error("game.maxclients", "client count exceeds save bytes"));
    }
    let mut clients = Vec::new();
    for _ in 0..count {
        clients.push(read_record(&mut reader, &source, &layout.client)?);
    }
    if reader.remaining() != 0 {
        return Err(field_error(
            "game.ssv",
            "unconsumed bytes indicate a mismatched native layout",
        ));
    }
    Ok(Q2ClassicGameSave {
        build_date,
        game,
        clients,
    })
}

/// Encode a `game.ssv`.
pub fn encode_q2_classic_game(
    save: &Q2ClassicGameSave,
    layout: &Q2ClassicSaveLayout,
) -> Result<Vec<u8>, PersistenceError> {
    if save.build_date.len() != 16 || save.game.len() != layout.game_bytes {
        return Err(field_error("game.ssv", "native game header size mismatch"));
    }
    let mut game = save.game.clone();
    #[allow(clippy::cast_possible_wrap)]
    set_field_i32(
        &mut game,
        &Q2ClassicSaveField {
            name: "game.maxclients".to_string(),
            offset: layout.client_count_offset,
            kind: Q2ClassicFieldKind::Client,
        },
        save.clients.len() as i32,
    )?;
    let mut writer = BinaryWriter::new(16 + game.len() + save.clients.iter().map(record_size).sum::<usize>());
    writer.bytes(&save.build_date)?;
    writer.bytes(&game)?;
    for client in &save.clients {
        write_record(&mut writer, client, &layout.client)?;
    }
    Ok(writer.finish())
}

/// Decode a `level.sav`.
pub fn decode_q2_classic_level(
    bytes: &[u8],
    layout: &Q2ClassicSaveLayout,
) -> Result<Q2ClassicLevelSave, PersistenceError> {
    let source = format!("{}/level.sav", layout.module.id);
    let mut reader = BinaryReader::new(bytes, &source);
    #[allow(clippy::cast_possible_wrap)]
    if reader.i32()? != layout.entity.byte_length as i32 {
        return Err(field_error("level.sav", "native edict size mismatch"));
    }
    let low = reader.u32()?;
    let function_base = u64::from(low)
        | if layout.pointer_bytes == 8 {
            u64::from(reader.u32()?) << 32
        } else {
            0
        };
    let level = read_record(&mut reader, &source, &layout.level)?;
    let mut entities = Vec::new();
    loop {
        let slot = reader.i32()?;
        if slot == -1 {
            break;
        }
        if slot < 0 {
            return Err(field_error("level.sav", "invalid native edict slot"));
        }
        entities.push((slot, read_record(&mut reader, &source, &layout.entity)?));
    }
    if reader.remaining() != 0 {
        return Err(field_error(
            "level.sav",
            "unconsumed bytes indicate a mismatched native layout",
        ));
    }
    Ok(Q2ClassicLevelSave {
        function_base,
        level,
        entities,
    })
}

/// Encode a `level.sav`.
pub fn encode_q2_classic_level(
    save: &Q2ClassicLevelSave,
    layout: &Q2ClassicSaveLayout,
) -> Result<Vec<u8>, PersistenceError> {
    let mut writer = BinaryWriter::new(
        8 + layout.pointer_bytes as usize
            + record_size(&save.level)
            + save
                .entities
                .iter()
                .map(|(_, record)| 4 + record_size(record))
                .sum::<usize>(),
    );
    #[allow(clippy::cast_possible_wrap)]
    writer.i32(layout.entity.byte_length as i32)?;
    #[allow(clippy::cast_possible_truncation)]
    writer.u32(save.function_base as u32)?;
    if layout.pointer_bytes == 8 {
        #[allow(clippy::cast_possible_truncation)]
        writer.u32((save.function_base >> 32) as u32)?;
    }
    write_record(&mut writer, &save.level, &layout.level)?;
    for (slot, record) in &save.entities {
        writer.i32(*slot)?;
        write_record(&mut writer, record, &layout.entity)?;
    }
    writer.i32(-1)?;
    Ok(writer.finish())
}

/// Pointer relocations into the restoring guest address space.
pub trait Q2ClassicRelocator {
    /// Pointer width (4 or 8).
    fn pointer_bytes(&self) -> u32;
    /// Relocate a string tail.
    fn relocate_string(&self, field: &str, bytes: Option<&[u8]>) -> u64;
    /// Relocate a saved reference index.
    fn relocate_reference(&self, field: &Q2ClassicSaveField, index: i32) -> u64;
}

/// Resolve saved indexes into the restoring guest owner's address space.
pub fn restore_q2_classic_record(
    record: &Q2ClassicRecord,
    layout: &Q2ClassicRecordLayout,
    relocations: &impl Q2ClassicRelocator,
) -> Result<Vec<u8>, PersistenceError> {
    if record.bytes.len() != layout.byte_length {
        return Err(field_error("q2-record", "native struct size mismatch"));
    }
    let mut bytes = record.bytes.clone();
    for field in &layout.fields {
        let pointer = if field.kind == Q2ClassicFieldKind::String {
            let value = record.strings.iter().find(|string| string.field == field.name);
            let Some(value) = value else {
                return Err(field_error(&field.name, "missing source string relocation"));
            };
            relocations.relocate_string(&field.name, value.bytes.as_deref())
        } else {
            let value = record.references.iter().find(|reference| reference.field == field.name);
            let Some(value) = value else {
                return Err(field_error(&field.name, "missing source pointer relocation"));
            };
            relocations.relocate_reference(field, value.index)
        };
        let width = usize::try_from(relocations.pointer_bytes()).unwrap_or(4);
        if width != 4 && width != 8 {
            return Err(field_error(&field.name, "restored pointer exceeds guest ABI"));
        }
        if pointer >= (1u128 << (width * 8)) as u64 && width < 8 {
            return Err(field_error(&field.name, "restored pointer exceeds guest ABI"));
        }
        if field.offset + width > bytes.len() {
            return Err(field_error(&field.name, "save field is outside its native struct"));
        }
        if width == 4 {
            #[allow(clippy::cast_possible_truncation)]
            bytes[field.offset..field.offset + 4].copy_from_slice(&(pointer as u32).to_le_bytes());
        } else {
            bytes[field.offset..field.offset + 8].copy_from_slice(&pointer.to_le_bytes());
        }
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn module() -> ModuleIdentity {
        ModuleIdentity {
            id: "q2:game".to_string(),
            artifact_path: "gamex86.dll".to_string(),
            digest: format!("sha256:{}", "0".repeat(64)),
            revision: "1".to_string(),
        }
    }

    fn layout() -> Q2ClassicSaveLayout {
        Q2ClassicSaveLayout {
            module: module(),
            pointer_bytes: 4,
            game_bytes: 8,
            client_count_offset: 0,
            client: Q2ClassicRecordLayout {
                byte_length: 8,
                fields: vec![
                    Q2ClassicSaveField {
                        name: "name".to_string(),
                        offset: 0,
                        kind: Q2ClassicFieldKind::String,
                    },
                    Q2ClassicSaveField {
                        name: "edict".to_string(),
                        offset: 4,
                        kind: Q2ClassicFieldKind::Entity,
                    },
                ],
            },
            level: Q2ClassicRecordLayout {
                byte_length: 4,
                fields: Vec::new(),
            },
            entity: Q2ClassicRecordLayout {
                byte_length: 8,
                fields: vec![Q2ClassicSaveField {
                    name: "target".to_string(),
                    offset: 0,
                    kind: Q2ClassicFieldKind::String,
                }],
            },
        }
    }

    fn client_record() -> Q2ClassicRecord {
        Q2ClassicRecord {
            bytes: vec![0; 8],
            strings: vec![Q2ClassicString {
                field: "name".to_string(),
                bytes: Some(b"player\0".to_vec()),
            }],
            references: vec![Q2ClassicReference {
                field: "edict".to_string(),
                index: 1,
            }],
        }
    }

    #[test]
    fn game_and_level_round_trip() {
        let layout = layout();
        let game = Q2ClassicGameSave {
            build_date: vec![b'0'; 16],
            game: vec![0; 8],
            clients: vec![client_record(), client_record()],
        };
        let bytes = encode_q2_classic_game(&game, &layout).unwrap();
        let decoded = decode_q2_classic_game(&bytes, &layout).unwrap();
        assert_eq!(decoded.build_date, game.build_date);
        // Decoded struct bytes keep the wire slots (string length, reference index).
        let mut expected = client_record();
        expected.bytes = vec![7, 0, 0, 0, 1, 0, 0, 0];
        assert_eq!(decoded.clients, vec![expected.clone(), expected]);
        assert_eq!(i32::from_le_bytes(decoded.game[0..4].try_into().unwrap()), 2);
        let level = Q2ClassicLevelSave {
            function_base: 0x1000,
            level: Q2ClassicRecord {
                bytes: vec![7, 7, 7, 7],
                strings: Vec::new(),
                references: Vec::new(),
            },
            entities: vec![(
                3,
                Q2ClassicRecord {
                    bytes: vec![0; 8],
                    strings: vec![Q2ClassicString {
                        field: "target".to_string(),
                        bytes: Some(b"t0\0".to_vec()),
                    }],
                    references: Vec::new(),
                },
            )],
        };
        let bytes = encode_q2_classic_level(&level, &layout).unwrap();
        let decoded = decode_q2_classic_level(&bytes, &layout).unwrap();
        assert_eq!(decoded.function_base, level.function_base);
        assert_eq!(decoded.level, level.level);
        assert_eq!(decoded.entities.len(), 1);
        assert_eq!(decoded.entities[0].0, 3);
        // "t0\0" tail length is patched into the struct slot.
        assert_eq!(&decoded.entities[0].1.bytes[0..4], &[3, 0, 0, 0]);
        assert_eq!(decoded.entities[0].1.strings, level.entities[0].1.strings);
    }

    #[test]
    fn relocations_patch_pointers() {
        struct Relocator;
        impl Q2ClassicRelocator for Relocator {
            fn pointer_bytes(&self) -> u32 {
                4
            }
            fn relocate_string(&self, _field: &str, bytes: Option<&[u8]>) -> u64 {
                bytes.map_or(0, |tail| 0x2000 + tail.len() as u64)
            }
            fn relocate_reference(&self, _field: &Q2ClassicSaveField, index: i32) -> u64 {
                0x3000 + index as u64
            }
        }
        let layout = layout();
        let bytes = restore_q2_classic_record(&client_record(), &layout.client, &Relocator).unwrap();
        assert_eq!(u32::from_le_bytes(bytes[0..4].try_into().unwrap()), 0x2007);
        assert_eq!(u32::from_le_bytes(bytes[4..8].try_into().unwrap()), 0x3001);
    }
}
