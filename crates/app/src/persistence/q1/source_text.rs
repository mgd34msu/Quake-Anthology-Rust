//! Original Quake save text ported from `src/persistence/q1.ts`.
//!
//! `Host_Savegame_f` / `Host_Loadgame_f` version 5, plus Ironwail's KEX
//! version 6 header (`gameDirectories`). Array order of `entities` is the
//! original edict number; an empty record is a free slot.
//!
//! Numeric fields are decimal literals (the writer emits `toFixed(6)` for
//! header floats and bare integers elsewhere); the donor's `Number()`
//! also accepts hexadecimal spellings, which this port rejects.

use qa_world::save::bytes::{byte_text, text_bytes};

use super::super::PersistenceError;

fn save_error(path: &str, message: &str) -> PersistenceError {
    PersistenceError::BadSave(format!("{path}: {message}"))
}

/// Save format header.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Q1SaveFormat {
    /// Original version 5.
    V5,
    /// KEX version 6 with game directories.
    V6 {
        /// Semicolon-separated game directories.
        game_directories: String,
    },
}

/// One QuakeC text pair.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QcTextPair {
    /// Key.
    pub key: String,
    /// Value.
    pub value: String,
}

/// Decoded original Quake save.
#[derive(Debug, Clone, PartialEq)]
pub struct Q1SaveData {
    /// Format header.
    pub format: Q1SaveFormat,
    /// Comment token.
    pub comment: String,
    /// Spawn parameters (16).
    pub spawn_parameters: Vec<f64>,
    /// Skill level.
    pub skill: i32,
    /// Map name.
    pub map: String,
    /// Server time.
    pub time: f64,
    /// Light styles (64).
    pub light_styles: Vec<String>,
    /// Global pairs.
    pub globals: Vec<QcTextPair>,
    /// Entity records by edict number.
    pub entities: Vec<Vec<QcTextPair>>,
    /// Trailing extension text.
    pub extension_text: String,
}

struct Scanner {
    text: Vec<char>,
    offset: usize,
}

impl Scanner {
    fn skip(&mut self) {
        loop {
            while self.offset < self.text.len() && self.text[self.offset] as u32 <= 32 {
                self.offset += 1;
            }
            if self.offset + 1 < self.text.len() && self.text[self.offset] == '/' && self.text[self.offset + 1] == '/' {
                while self.offset < self.text.len() && self.text[self.offset] != '\n' {
                    self.offset += 1;
                }
                if self.offset < self.text.len() {
                    self.offset += 1;
                }
            } else {
                return;
            }
        }
    }

    fn token(&mut self) -> Result<String, PersistenceError> {
        self.skip();
        if self.offset >= self.text.len() {
            return Err(save_error(&format!("q1:{}", self.offset), "truncated save"));
        }
        let first = self.text[self.offset];
        self.offset += 1;
        if first == '{' || first == '}' {
            return Ok(first.to_string());
        }
        if first == '"' {
            let start = self.offset;
            while self.offset < self.text.len() && self.text[self.offset] != '"' {
                self.offset += 1;
            }
            if self.offset >= self.text.len() {
                return Err(save_error(&format!("q1:{}", self.offset), "unterminated quoted field"));
            }
            let token: String = self.text[start..self.offset].iter().collect();
            self.offset += 1;
            return Ok(token);
        }
        let start = self.offset - 1;
        while self.offset < self.text.len() {
            let character = self.text[self.offset];
            if character as u32 <= 32 || character == '{' || character == '}' {
                break;
            }
            self.offset += 1;
        }
        Ok(self.text[start..self.offset].iter().collect())
    }

    fn number(&mut self) -> Result<f64, PersistenceError> {
        let text = self.token()?;
        match text.parse::<f64>() {
            Ok(value) if value.is_finite() => Ok(value),
            _ => Err(save_error(
                &format!("q1:{}", self.offset),
                &format!("invalid numeric field {text}"),
            )),
        }
    }

    fn record(&mut self) -> Result<Vec<QcTextPair>, PersistenceError> {
        if self.token()? != "{" {
            return Err(save_error(&format!("q1:{}", self.offset), "expected opening brace"));
        }
        let mut pairs = Vec::new();
        loop {
            let key = self.token()?;
            if key == "}" {
                return Ok(pairs);
            }
            let value = self.token()?;
            if value == "}" {
                return Err(save_error(&format!("q1:{}", self.offset), "field has no value"));
            }
            pairs.push(QcTextPair { key, value });
        }
    }
}

/// Decode an original Quake save.
pub fn decode_q1_save(bytes: &[u8]) -> Result<Q1SaveData, PersistenceError> {
    let mut scanner = Scanner {
        text: byte_text(bytes).chars().collect(),
        offset: 0,
    };
    let version = scanner.number()?;
    let format = if version == 5.0 {
        Q1SaveFormat::V5
    } else if version == 6.0 {
        Q1SaveFormat::V6 {
            game_directories: scanner.token()?,
        }
    } else {
        return Err(save_error("q1", &format!("unsupported save version {version}")));
    };
    let comment = scanner.token()?;
    let mut spawn_parameters = Vec::with_capacity(16);
    for _ in 0..16 {
        spawn_parameters.push(scanner.number()?);
    }
    let skill = (scanner.number()? + 0.1).trunc();
    #[allow(clippy::cast_possible_truncation)]
    let skill = skill as i32;
    let map = scanner.token()?;
    let time = scanner.number()?;
    let mut light_styles = Vec::with_capacity(64);
    for _ in 0..64 {
        light_styles.push(scanner.token()?);
    }
    let globals = scanner.record()?;
    let mut entities = Vec::new();
    loop {
        while scanner.offset < scanner.text.len() && scanner.text[scanner.offset] as u32 <= 32 {
            scanner.offset += 1;
        }
        let extension_start = scanner.offset;
        scanner.skip();
        if scanner.offset >= scanner.text.len() || scanner.text[scanner.offset] != '{' {
            scanner.offset = extension_start;
            break;
        }
        entities.push(scanner.record()?);
    }
    Ok(Q1SaveData {
        format,
        comment,
        spawn_parameters,
        skill,
        map,
        time,
        light_styles,
        globals,
        entities,
        extension_text: scanner.text[scanner.offset..].iter().collect(),
    })
}

fn header_token(value: &str) -> Result<&str, PersistenceError> {
    if value.is_empty()
        || value
            .chars()
            .any(|character| character.is_whitespace() || character == '\0' || character == '"')
    {
        return Err(save_error("q1", "invalid save header token"));
    }
    Ok(value)
}

fn decimal(value: f64) -> Result<String, PersistenceError> {
    if !value.is_finite() {
        return Err(save_error("q1", "non-finite header number"));
    }
    Ok(format!("{value:.6}"))
}

/// Encode an original Quake save.
pub fn encode_q1_save(save: &Q1SaveData) -> Result<Vec<u8>, PersistenceError> {
    if save.spawn_parameters.len() != 16 || save.light_styles.len() != 64 {
        return Err(save_error(
            "q1",
            "save requires 16 spawn parameters and 64 light styles",
        ));
    }
    let mut lines = match &save.format {
        Q1SaveFormat::V5 => vec!["5".to_string()],
        Q1SaveFormat::V6 { game_directories } => {
            vec!["6".to_string(), header_token(game_directories)?.to_string()]
        }
    };
    lines.push(header_token(&save.comment)?.to_string());
    for parameter in &save.spawn_parameters {
        lines.push(decimal(*parameter)?);
    }
    lines.push(format!("{}", save.skill));
    lines.push(header_token(&save.map)?.to_string());
    lines.push(decimal(save.time)?);
    for style in &save.light_styles {
        let style = if style.is_empty() { "m" } else { style };
        lines.push(header_token(style)?.to_string());
    }
    let mut record = |pairs: &[QcTextPair]| -> Result<(), PersistenceError> {
        lines.push("{".to_string());
        for pair in pairs {
            if pair.key.chars().any(|character| character == '"' || character == '\0')
                || pair
                    .value
                    .chars()
                    .any(|character| character == '"' || character == '\0')
            {
                return Err(save_error(
                    "q1",
                    "source QuakeC save strings cannot contain quotes or NUL",
                ));
            }
            lines.push(format!("\"{}\" \"{}\"", pair.key, pair.value));
        }
        lines.push("}".to_string());
        Ok(())
    };
    record(&save.globals)?;
    for entity in &save.entities {
        record(entity)?;
    }
    text_bytes(&format!("{}\n{}", lines.join("\n"), save.extension_text))
        .map_err(|_| save_error("q1", "invalid save text"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Q1SaveData {
        Q1SaveData {
            format: Q1SaveFormat::V6 {
                game_directories: "id1;rogue".to_string(),
            },
            comment: "test_save".to_string(),
            spawn_parameters: vec![0.5; 16],
            skill: 2,
            map: "e1m1".to_string(),
            time: 12.5,
            light_styles: vec!["m".to_string(); 64],
            globals: vec![QcTextPair {
                key: "time".to_string(),
                value: "12.5".to_string(),
            }],
            entities: vec![
                vec![QcTextPair {
                    key: "classname".to_string(),
                    value: "worldspawn".to_string(),
                }],
                vec![QcTextPair {
                    key: "classname".to_string(),
                    value: "player".to_string(),
                }],
                Vec::new(),
            ],
            extension_text: "// extension\n".to_string(),
        }
    }

    #[test]
    fn saves_round_trip() {
        let save = sample();
        let bytes = encode_q1_save(&save).unwrap();
        let text = String::from_utf8(bytes.clone()).unwrap();
        assert!(text.starts_with("6\nid1;rogue\ntest_save\n"));
        assert!(text.contains("\"classname\" \"player\""));
        let decoded = decode_q1_save(&bytes).unwrap();
        assert_eq!(decoded.format, save.format);
        assert_eq!(decoded.comment, save.comment);
        assert_eq!(decoded.skill, save.skill);
        assert_eq!(decoded.map, save.map);
        assert_eq!(decoded.globals, save.globals);
        assert_eq!(decoded.entities, save.entities);
        assert_eq!(decoded.extension_text, save.extension_text);
        for (left, right) in decoded.spawn_parameters.iter().zip(save.spawn_parameters.iter()) {
            assert!((left - right).abs() < 1e-6);
        }
        // Comments and blank lines decode around records.
        let commented = format!("{}\n// note\n", String::from_utf8(bytes).unwrap());
        let decoded = decode_q1_save(commented.as_bytes()).unwrap();
        assert_eq!(decoded.entities.len(), 3);
    }

    #[test]
    fn malformed_saves_fail() {
        assert!(decode_q1_save(b"7\ncomment").is_err());
        assert!(decode_q1_save(b"5\ncomment\n1 2").is_err());
        assert!(decode_q1_save(b"5\n\"unterminated").is_err());
        let mut save = sample();
        save.spawn_parameters.pop();
        assert!(encode_q1_save(&save).is_err());
        let mut save = sample();
        save.map = "has space".to_string();
        assert!(encode_q1_save(&save).is_err());
    }
}
