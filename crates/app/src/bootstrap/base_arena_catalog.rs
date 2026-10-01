//! Base single-player arena catalog.
//!
//! Donor: `/home/buzzkill/Projects/quake-typescript/src/app/bootstrap/base-arena-catalog.ts`
//! (`parseBaseArenaCatalog`, `readBaseArenaCatalog`, `baseArenaForMap`).
//! Async mounts access becomes sync reads through [`ArenaCatalogMounts`];
//! the `.arena` tokenizer is a local `COM_Parse` following
//! `src/core/common-parse.ts` (`parse` with and without line breaks).

use std::collections::HashMap;

use qa_content::mounts::{MountError, MountedContent};
use thiserror::Error;

/// Base arena catalog failure.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum BaseArenaCatalogError {
    /// Parse source is not Latin-1 bytes.
    #[error("COM_Parse source is not a Latin-1 byte string")]
    NonByteSource,
    /// Quoted token terminator overflows the 1024-byte token.
    #[error("COM_Parse quoted token terminator exceeds 1024-byte storage")]
    QuotedTerminatorOverflow,
    /// Expected `{` before an arena definition.
    #[error("Malformed base arena definition")]
    Malformed,
    /// Arena definition ends before `}`.
    #[error("Unterminated base arena definition")]
    Unterminated,
    /// Arena `map` field fails the `q3` map-name shape.
    #[error("Invalid base arena map name")]
    InvalidMapName,
    /// No authored single-player arena matches the map.
    #[error("No authored single-player arena for {0}")]
    UnknownMap(String),
    /// Mount listing or open failure.
    #[error(transparent)]
    Mount(#[from] MountError),
}

/// One authored single-player arena (`BaseArena`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BaseArena {
    /// UI_LoadArenas numbering (regulars first, specials after).
    pub number: i32,
    /// BSP path (`maps/<name>.bsp`).
    pub map: String,
    /// Display title (`longname`, else the bare map name).
    pub title: String,
    /// Bot names.
    pub bots: Vec<String>,
    /// Special slot (`""`, `training`, or `final`).
    pub special: String,
    /// Selection order (`training` sorts first, `final` last).
    pub selection: i32,
    /// Frag limit (defaults to 10 when both limits are zero).
    pub frag_limit: i64,
    /// Time limit in minutes.
    pub time_limit: i64,
}

/// Parsed arena catalog (`BaseArenaCatalog`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BaseArenaCatalog {
    /// Single-player arenas in file order.
    pub arenas: Vec<BaseArena>,
    /// Regular (non-special) arena count, rounded down to a tier of four.
    pub regular_count: i32,
    /// Regular tier count (`regularCount / 4`).
    pub tier_count: i32,
}

/// Mount reads backing [`read_base_arena_catalog`].
pub trait ArenaCatalogMounts {
    /// List one directory by extension.
    fn list_files(&self, directory: &str, extension: &str) -> Result<Vec<String>, MountError>;
    /// Open one resource path; `None` means absent.
    fn open(&self, path: &str) -> Result<Option<Vec<u8>>, MountError>;
}

impl ArenaCatalogMounts for MountedContent {
    fn list_files(&self, directory: &str, extension: &str) -> Result<Vec<String>, MountError> {
        MountedContent::list_files(self, directory, extension)
    }

    fn open(&self, path: &str) -> Result<Option<Vec<u8>>, MountError> {
        Ok(MountedContent::open(self, path, |_| true)?.map(|found| found.bytes))
    }
}

const MAX_TOKEN_CHARS: usize = 1024;

struct ParseCursor {
    bytes: Vec<u8>,
    offset: Option<usize>,
}

impl ParseCursor {
    fn new(source: &str) -> Result<Self, BaseArenaCatalogError> {
        if source.chars().any(|c| c as u32 > 255) {
            return Err(BaseArenaCatalogError::NonByteSource);
        }
        Ok(Self {
            bytes: source.as_bytes().to_vec(),
            offset: Some(0),
        })
    }

    fn signed_byte(&self, offset: usize) -> i32 {
        self.bytes.get(offset).map_or(0, |b| i32::from(*b as i8))
    }

    fn char_at(&self, offset: usize) -> char {
        self.bytes.get(offset).map_or('\0', |b| *b as char)
    }
}

#[derive(Debug, Default)]
struct ParseState {
    token: String,
    line: i32,
}

impl ParseState {
    fn parse(&mut self, cursor: &mut ParseCursor, allow_line_breaks: bool) -> Result<String, BaseArenaCatalogError> {
        let Some(mut data) = cursor.offset else {
            self.token.clear();
            return Ok(String::new());
        };
        self.token.clear();
        let mut has_new_lines = false;
        let mut c: i32;
        loop {
            loop {
                c = cursor.signed_byte(data);
                if c > 32 {
                    break;
                }
                if c == 0 {
                    cursor.offset = None;
                    return Ok(String::new());
                }
                if c == 10 {
                    self.line = self.line.wrapping_add(1);
                    has_new_lines = true;
                }
                data += 1;
            }
            if has_new_lines && !allow_line_breaks {
                cursor.offset = Some(data);
                return Ok(String::new());
            }
            if c == 47 && cursor.signed_byte(data + 1) == 47 {
                data += 2;
                while {
                    c = cursor.signed_byte(data);
                    c != 0 && c != 10
                } {
                    data += 1;
                }
            } else if c == 47 && cursor.signed_byte(data + 1) == 42 {
                data += 2;
                while cursor.signed_byte(data) != 0
                    && (cursor.signed_byte(data) != 42 || cursor.signed_byte(data + 1) != 47)
                {
                    data += 1;
                }
                if cursor.signed_byte(data) != 0 {
                    data += 2;
                }
            } else {
                break;
            }
        }
        if c == 34 {
            data += 1;
            loop {
                c = cursor.signed_byte(data);
                data += 1;
                if c == 34 || c == 0 {
                    if self.token.chars().count() == MAX_TOKEN_CHARS {
                        return Err(BaseArenaCatalogError::QuotedTerminatorOverflow);
                    }
                    cursor.offset = if c == 0 { None } else { Some(data) };
                    return Ok(std::mem::take(&mut self.token));
                }
                if self.token.chars().count() < MAX_TOKEN_CHARS {
                    self.token.push(cursor.char_at(data - 1));
                }
            }
        }
        loop {
            if self.token.chars().count() < MAX_TOKEN_CHARS {
                self.token.push(cursor.char_at(data));
            }
            data += 1;
            c = cursor.signed_byte(data);
            if c == 10 {
                self.line = self.line.wrapping_add(1);
            }
            if c <= 32 {
                break;
            }
        }
        if self.token.chars().count() == MAX_TOKEN_CHARS {
            self.token.clear();
        }
        cursor.offset = Some(data);
        Ok(std::mem::take(&mut self.token))
    }
}

/// JavaScript `Number.parseInt(text, 10)`: leading-whitespace skip, optional
/// sign, decimal-digit prefix; no digits means `None` (the donor's `NaN`).
/// Saturates at the `i64` bounds; source limits are small game values.
fn parse_int_prefix(text: &str) -> Option<i64> {
    let bytes = text.as_bytes();
    let mut offset = 0;
    while offset < bytes.len() && bytes[offset].is_ascii_whitespace() {
        offset += 1;
    }
    let mut negative = false;
    if offset < bytes.len() && (bytes[offset] == b'+' || bytes[offset] == b'-') {
        negative = bytes[offset] == b'-';
        offset += 1;
    }
    let mut value: i64 = 0;
    let mut digits = 0u32;
    while offset < bytes.len() && bytes[offset].is_ascii_digit() {
        value = value.saturating_mul(10).saturating_add(i64::from(bytes[offset] - b'0'));
        offset += 1;
        digits += 1;
    }
    if digits == 0 {
        return None;
    }
    Some(if negative { value.saturating_neg() } else { value })
}

fn valid_map_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '/' | '-'))
}

/// Parse `.arena` texts into a catalog (`parseBaseArenaCatalog`).
pub fn parse_base_arena_catalog(texts: &[&str]) -> Result<BaseArenaCatalog, BaseArenaCatalogError> {
    let mut infos: Vec<HashMap<String, String>> = Vec::new();
    for text in texts {
        let mut parser = ParseState::default();
        let mut cursor = ParseCursor::new(text)?;
        loop {
            let token = parser.parse(&mut cursor, true)?;
            if token.is_empty() {
                break;
            }
            if token != "{" {
                return Err(BaseArenaCatalogError::Malformed);
            }
            let mut fields = HashMap::new();
            loop {
                let key = parser.parse(&mut cursor, true)?;
                if key == "}" {
                    break;
                }
                if key.is_empty() {
                    return Err(BaseArenaCatalogError::Unterminated);
                }
                let value = parser.parse(&mut cursor, false)?;
                fields.insert(key, if value.is_empty() { "<NULL>".to_string() } else { value });
            }
            infos.push(fields);
        }
    }
    let singles: Vec<&HashMap<String, String>> = infos
        .iter()
        .filter(|row| row.get("type").is_some_and(|kind| kind.contains("single")))
        .collect();
    let regular_count = singles.iter().filter(|row| row.get("special").is_none()).count() as i32 / 4 * 4;
    let mut single = 0i32;
    let mut special_number = regular_count;
    let mut arenas = Vec::with_capacity(singles.len());
    for row in singles {
        let special = row.get("special").cloned().unwrap_or_default();
        let number = if special.is_empty() {
            let number = single;
            single += 1;
            number
        } else {
            let number = special_number;
            special_number += 1;
            number
        };
        let name = row.get("map").cloned().unwrap_or_default();
        if !valid_map_name(&name) {
            return Err(BaseArenaCatalogError::InvalidMapName);
        }
        let frag = row
            .get("fraglimit")
            .and_then(|text| parse_int_prefix(text))
            .unwrap_or(0);
        let time = row
            .get("timelimit")
            .and_then(|text| parse_int_prefix(text))
            .unwrap_or(0);
        let lowered = special.to_lowercase();
        arenas.push(BaseArena {
            number,
            map: format!("maps/{name}.bsp"),
            title: row.get("longname").cloned().unwrap_or(name),
            special,
            selection: if lowered == "training" {
                -4
            } else if lowered == "final" {
                regular_count
            } else {
                number
            },
            bots: row
                .get("bots")
                .map(|bots| bots.split_whitespace().map(str::to_string).collect::<Vec<_>>())
                .unwrap_or_default(),
            frag_limit: if frag == 0 && time == 0 { 10 } else { frag },
            time_limit: time,
        });
    }
    Ok(BaseArenaCatalog {
        arenas,
        regular_count,
        tier_count: regular_count / 4,
    })
}

/// Read `scripts/arenas.txt` plus every `scripts/*.arena` and parse the
/// catalog (`readBaseArenaCatalog`). Missing files are skipped.
pub fn read_base_arena_catalog(mounts: &impl ArenaCatalogMounts) -> Result<BaseArenaCatalog, BaseArenaCatalogError> {
    let mut files = vec!["scripts/arenas.txt".to_string()];
    files.extend(
        mounts
            .list_files("scripts", ".arena")?
            .into_iter()
            .map(|name| format!("scripts/{name}")),
    );
    let mut texts = Vec::new();
    for path in &files {
        if let Some(bytes) = mounts.open(path)? {
            texts.push(bytes.iter().map(|b| *b as char).collect::<String>());
        }
    }
    let borrowed: Vec<&str> = texts.iter().map(String::as_str).collect();
    parse_base_arena_catalog(&borrowed)
}

/// Find the authored arena for a map path (`baseArenaForMap`).
pub fn base_arena_for_map(catalog: &BaseArenaCatalog, map: &str) -> Result<BaseArena, BaseArenaCatalogError> {
    let lowered = map.to_lowercase();
    let without_prefix = lowered.strip_prefix("maps/").unwrap_or(&lowered);
    let stripped = without_prefix.strip_suffix(".bsp").unwrap_or(without_prefix);
    let wanted = format!("maps/{stripped}.bsp");
    catalog
        .arenas
        .iter()
        .find(|row| row.map.to_lowercase() == wanted)
        .cloned()
        .ok_or_else(|| BaseArenaCatalogError::UnknownMap(map.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    struct StubMounts {
        files: HashMap<String, Vec<u8>>,
        arena_names: Vec<String>,
    }

    impl ArenaCatalogMounts for StubMounts {
        fn list_files(&self, directory: &str, extension: &str) -> Result<Vec<String>, MountError> {
            assert_eq!(directory, "scripts");
            assert_eq!(extension, ".arena");
            Ok(self.arena_names.clone())
        }

        fn open(&self, path: &str) -> Result<Option<Vec<u8>>, MountError> {
            Ok(self.files.get(path).cloned())
        }
    }

    fn arena_block(fields: &[(&str, &str)]) -> String {
        let mut text = String::from("{\n");
        for (key, value) in fields {
            text.push_str(&format!("  {key} \"{value}\"\n"));
        }
        text.push_str("}\n");
        text
    }

    fn regular_arena(index: i32) -> String {
        arena_block(&[
            ("map", &format!("q3dm{index}")),
            ("longname", &format!("Arena {index}")),
            ("type", "single"),
            ("bots", "sarge major"),
        ])
    }

    #[test]
    fn parses_regular_and_special_arenas() {
        let mut text = String::new();
        for index in 1..=8 {
            text.push_str(&regular_arena(index));
        }
        text.push_str(&arena_block(&[
            ("map", "training"),
            ("longname", "Training"),
            ("type", "single"),
            ("special", "training"),
        ]));
        text.push_str(&arena_block(&[
            ("map", "final"),
            ("longname", "Final"),
            ("type", "single"),
            ("special", "final"),
            ("fraglimit", "20"),
            ("timelimit", "15"),
        ]));
        text.push_str(&arena_block(&[("map", "q3dm0"), ("type", "ffa")]));
        let catalog = parse_base_arena_catalog(&[text.as_str()]).unwrap();
        assert_eq!(catalog.regular_count, 8);
        assert_eq!(catalog.tier_count, 2);
        assert_eq!(catalog.arenas.len(), 10);
        let first = &catalog.arenas[0];
        assert_eq!(first.number, 0);
        assert_eq!(first.map, "maps/q3dm1.bsp");
        assert_eq!(first.title, "Arena 1");
        assert_eq!(first.bots, vec!["sarge".to_string(), "major".to_string()]);
        assert_eq!(first.selection, 0);
        assert_eq!(first.frag_limit, 10);
        assert_eq!(first.time_limit, 0);
        let training = &catalog.arenas[8];
        assert_eq!(training.number, 8);
        assert_eq!(training.selection, -4);
        let final_arena = &catalog.arenas[9];
        assert_eq!(final_arena.number, 9);
        assert_eq!(final_arena.selection, 8);
        assert_eq!(final_arena.frag_limit, 20);
        assert_eq!(final_arena.time_limit, 15);
    }

    #[test]
    fn regular_count_rounds_down_to_tier() {
        let mut text = String::new();
        for index in 1..=6 {
            text.push_str(&regular_arena(index));
        }
        let catalog = parse_base_arena_catalog(&[text.as_str()]).unwrap();
        assert_eq!(catalog.regular_count, 4);
        assert_eq!(catalog.tier_count, 1);
    }

    #[test]
    fn missing_value_becomes_null_placeholder() {
        let text = "{\n  map \"q3dm1\"\n  longname\n  type \"single\"\n}\n";
        let catalog = parse_base_arena_catalog(&[text]).unwrap();
        assert_eq!(catalog.arenas.len(), 1);
        assert_eq!(catalog.arenas[0].title, "<NULL>");
    }

    #[test]
    fn rejects_malformed_and_bad_names() {
        assert_eq!(
            parse_base_arena_catalog(&["map"]).unwrap_err(),
            BaseArenaCatalogError::Malformed
        );
        assert_eq!(
            parse_base_arena_catalog(&["{"]).unwrap_err(),
            BaseArenaCatalogError::Unterminated
        );
        let bad = arena_block(&[("map", "q3dm1;drop"), ("type", "single")]);
        assert_eq!(
            parse_base_arena_catalog(&[bad.as_str()]).unwrap_err(),
            BaseArenaCatalogError::InvalidMapName
        );
        let missing = arena_block(&[("type", "single")]);
        assert_eq!(
            parse_base_arena_catalog(&[missing.as_str()]).unwrap_err(),
            BaseArenaCatalogError::InvalidMapName
        );
    }

    #[test]
    fn reads_catalog_from_mounts_skipping_missing() {
        let mut files = HashMap::new();
        files.insert(
            "scripts/arenas.txt".to_string(),
            regular_arena(1).repeat(4).into_bytes(),
        );
        files.insert(
            "scripts/extra.arena".to_string(),
            regular_arena(5).repeat(4).into_bytes(),
        );
        let mounts = StubMounts {
            files,
            arena_names: vec!["extra.arena".to_string(), "missing.arena".to_string()],
        };
        let catalog = read_base_arena_catalog(&mounts).unwrap();
        assert_eq!(catalog.arenas.len(), 8);
        assert_eq!(catalog.regular_count, 8);
    }

    #[test]
    fn finds_arena_for_map_variants() {
        let catalog = parse_base_arena_catalog(&[regular_arena(1).repeat(4).as_str()]).unwrap();
        let arena = base_arena_for_map(&catalog, "Q3DM1").unwrap();
        assert_eq!(arena.map, "maps/q3dm1.bsp");
        let arena = base_arena_for_map(&catalog, "maps/q3dm1.bsp").unwrap();
        assert_eq!(arena.number, 0);
        assert_eq!(
            base_arena_for_map(&catalog, "maps/unknown.bsp").unwrap_err(),
            BaseArenaCatalogError::UnknownMap("maps/unknown.bsp".to_string())
        );
    }
}
