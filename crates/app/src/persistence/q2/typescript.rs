//! TypeScript-donor Quake II saves ported from `src/persistence/q2-typescript.ts`.
//!
//! The donor's classic TypeScript game uses this JSON format, distinct
//! from retail native structs. Records stay as lossless [`SourceJson`]
//! objects; this module owns the stamp, client/entity arrays, and slot
//! validation.

use qa_world::save::json::{
    parse_source_json, source_number, source_object, write_source_json, SaveNumber, SourceJson,
};

use super::super::PersistenceError;

fn save_error(path: &str, message: &str) -> PersistenceError {
    PersistenceError::BadSave(format!("{path}: {message}"))
}

/// TypeScript game save.
#[derive(Debug, Clone, PartialEq)]
pub struct Q2TypeScriptGameSave {
    /// Root record.
    pub root: SourceJson,
    /// Client records.
    pub clients: Vec<SourceJson>,
}

/// TypeScript level save.
#[derive(Debug, Clone, PartialEq)]
pub struct Q2TypeScriptLevelSave {
    /// Root record.
    pub root: SourceJson,
    /// Level record.
    pub level: SourceJson,
    /// Entity records by slot.
    pub edicts: Vec<(u32, SourceJson)>,
}

fn root_object(text: &str) -> Result<SourceJson, PersistenceError> {
    let parsed = parse_source_json(text)?;
    source_object(Some(&parsed), "q2-typescript")?;
    Ok(parsed)
}

/// Decode a TypeScript game save.
pub fn decode_q2_typescript_game(text: &str) -> Result<Q2TypeScriptGameSave, PersistenceError> {
    let root = root_object(text)?;
    if root.get("stamp") != Some(&SourceJson::String("quake-2-ts:g_save:v1".to_string())) {
        return Err(save_error("stamp", "unsupported TypeScript donor save version"));
    }
    let clients = match root.get("clients") {
        Some(SourceJson::Array(clients)) => clients
            .iter()
            .enumerate()
            .map(|(index, client)| {
                let path = format!("clients[{index}]");
                source_object(Some(client), &path)?;
                Ok(client.clone())
            })
            .collect::<Result<Vec<_>, PersistenceError>>()?,
        _ => return Err(save_error("clients", "expected source client array")),
    };
    Ok(Q2TypeScriptGameSave { root, clients })
}

/// Encode a TypeScript game save.
pub fn encode_q2_typescript_game(save: &Q2TypeScriptGameSave) -> String {
    let mut root = save.root.clone();
    let _ = root.set("clients", SourceJson::Array(save.clients.clone()));
    write_source_json(&root)
}

/// Decode a TypeScript level save.
pub fn decode_q2_typescript_level(text: &str) -> Result<Q2TypeScriptLevelSave, PersistenceError> {
    let root = root_object(text)?;
    let level = source_object(root.get("level"), "level")?.clone();
    let edicts = match root.get("edicts") {
        Some(SourceJson::Array(edicts)) => edicts
            .iter()
            .enumerate()
            .map(|(offset, value)| {
                let record = source_object(Some(value), &format!("edicts[{offset}]"))?.clone();
                if record.get("index").is_none() {
                    return Err(save_error("edicts.index", "missing source entity slot"));
                }
                let index = source_number(record.get("index"), &format!("edicts[{offset}].index"), 0.0)?;
                if !index.is_finite() || index.trunc() != index || index < 0.0 || index > f64::from(u32::MAX) {
                    return Err(save_error("edicts.index", "invalid source entity slot"));
                }
                #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                let slot = index as u32;
                let data = source_object(record.get("data"), &format!("edicts[{offset}].data"))?.clone();
                Ok((slot, data))
            })
            .collect::<Result<Vec<_>, PersistenceError>>()?,
        _ => return Err(save_error("edicts", "expected source entity array")),
    };
    Ok(Q2TypeScriptLevelSave { root, level, edicts })
}

/// Encode a TypeScript level save.
pub fn encode_q2_typescript_level(save: &Q2TypeScriptLevelSave) -> String {
    let mut root = save.root.clone();
    let _ = root.set("level", save.level.clone());
    let edicts = save
        .edicts
        .iter()
        .map(|(index, data)| {
            SourceJson::Object(vec![
                (
                    "index".to_string(),
                    SourceJson::Number(SaveNumber::from_i64(i64::from(*index))),
                ),
                ("data".to_string(), data.clone()),
            ])
        })
        .collect();
    let _ = root.set("edicts", SourceJson::Array(edicts));
    write_source_json(&root)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn game_saves_round_trip() {
        let text = r#"{"stamp":"quake-2-ts:g_save:v1","clients":[{"name":"p0"},{"name":"p1"}]}"#;
        let save = decode_q2_typescript_game(text).unwrap();
        assert_eq!(save.clients.len(), 2);
        assert_eq!(encode_q2_typescript_game(&save), text);
        assert!(decode_q2_typescript_game(r#"{"stamp":"other","clients":[]}"#).is_err());
        assert!(decode_q2_typescript_game(r#"{"stamp":"quake-2-ts:g_save:v1"}"#).is_err());
    }

    #[test]
    fn level_saves_round_trip() {
        let text = r#"{"level":{"map":"base1"},"edicts":[{"index":0,"data":{"classname":"worldspawn"}},{"index":3,"data":{}}]}"#;
        let save = decode_q2_typescript_level(text).unwrap();
        assert_eq!(save.edicts.len(), 2);
        assert_eq!(save.edicts[1].0, 3);
        assert_eq!(encode_q2_typescript_level(&save), text);
        assert!(decode_q2_typescript_level(r#"{"level":{},"edicts":[{"data":{}}]}"#).is_err());
        assert!(decode_q2_typescript_level(r#"{"level":{},"edicts":[{"index":-1,"data":{}}]}"#).is_err());
    }
}
