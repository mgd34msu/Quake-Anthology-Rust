//! Durable per-player progress records.
//!
//! Sync port of donor `src/app/bootstrap/player-progress.ts`: one
//! application-owned writer persists achievement, level-completed, and
//! match-completed events to `{ version: 1, events: [...] }` JSON. The
//! donor's promise tail serializing concurrent `record` calls collapses to
//! `&mut self` in the sync port; `flush` is a no-op kept for API parity.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use thiserror::Error;

use crate::settings::json::{parse_json, stringify, Json};
use crate::settings::SettingsError;

/// Failure of a player-progress operation.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum PlayerProgressError {
    /// Identity fields (source/participant/event) are missing or empty.
    #[error("Invalid player progress identity")]
    BadIdentity,
    /// A level/match record has a missing or empty map.
    #[error("Invalid player progress map")]
    BadMap,
    /// Kind-specific fields (award/score) are missing or invalid.
    #[error("Invalid player progress event")]
    BadEvent,
    /// The progress file is not `{ version: 1, events: [...] }`.
    #[error("Invalid player progress file")]
    BadFile,
    /// Two records share one `[source, participant, event]` identity.
    #[error("Duplicate player progress event")]
    Duplicate,
    /// The file is not valid JSON (donor: raw `JSON.parse` throw).
    #[error(transparent)]
    Json(#[from] SettingsError),
    /// A file operation failed (donor: raw fs throw).
    #[error("{0}")]
    Io(String),
}

impl From<std::io::Error> for PlayerProgressError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error.to_string())
    }
}

/// Game family a progress event belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProgressSource {
    /// Quake.
    Q1,
    /// Quake II.
    Q2,
    /// Quake III (match-completed only).
    Q3,
}

impl ProgressSource {
    /// Donor wire spelling.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Q1 => "q1",
            Self::Q2 => "q2",
            Self::Q3 => "q3",
        }
    }

    fn parse(text: &str) -> Option<Self> {
        match text {
            "q1" => Some(Self::Q1),
            "q2" => Some(Self::Q2),
            "q3" => Some(Self::Q3),
            _ => None,
        }
    }
}

/// One durable progress event (`PlayerProgressEvent`).
#[derive(Debug, Clone, PartialEq)]
pub enum PlayerProgressEvent {
    /// An earned award (`source` is q1/q2 only).
    Achievement {
        /// Game family.
        source: ProgressSource,
        /// Owning participant.
        participant: String,
        /// Stable event identity within the participant.
        event: String,
        /// Award label.
        award: String,
    },
    /// A completed level (`source` is q1/q2 only).
    LevelCompleted {
        /// Game family.
        source: ProgressSource,
        /// Owning participant.
        participant: String,
        /// Stable event identity within the participant.
        event: String,
        /// Completed map.
        map: String,
    },
    /// A completed match (any family).
    MatchCompleted {
        /// Game family.
        source: ProgressSource,
        /// Owning participant.
        participant: String,
        /// Stable event identity within the participant.
        event: String,
        /// Played map.
        map: String,
        /// Final score (finite).
        score: f64,
    },
}

impl PlayerProgressEvent {
    /// Donor `kind` discriminator.
    #[must_use]
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Achievement { .. } => "achievement",
            Self::LevelCompleted { .. } => "level-completed",
            Self::MatchCompleted { .. } => "match-completed",
        }
    }

    /// Game family.
    #[must_use]
    pub fn source(&self) -> ProgressSource {
        match self {
            Self::Achievement { source, .. }
            | Self::LevelCompleted { source, .. }
            | Self::MatchCompleted { source, .. } => *source,
        }
    }

    /// Owning participant.
    #[must_use]
    pub fn participant(&self) -> &str {
        match self {
            Self::Achievement { participant, .. }
            | Self::LevelCompleted { participant, .. }
            | Self::MatchCompleted { participant, .. } => participant,
        }
    }

    /// Stable event identity.
    #[must_use]
    pub fn event(&self) -> &str {
        match self {
            Self::Achievement { event, .. }
            | Self::LevelCompleted { event, .. }
            | Self::MatchCompleted { event, .. } => event,
        }
    }

    /// Mirror the donor `decode` checks for an already-typed event (the
    /// donor `record` re-decodes its typed input). Order matches the donor
    /// exactly, including its fall-through: an achievement that fails the
    /// award branch reaches the map check and reports `BadMap`.
    fn check(&self) -> Result<(), PlayerProgressError> {
        if self.participant().is_empty() || self.event().is_empty() {
            return Err(PlayerProgressError::BadIdentity);
        }
        match self {
            Self::Achievement { source, award, .. } => {
                if *source != ProgressSource::Q3 && !award.is_empty() {
                    return Ok(());
                }
                Err(PlayerProgressError::BadMap)
            }
            Self::LevelCompleted { source, map, .. } => {
                if map.is_empty() {
                    return Err(PlayerProgressError::BadMap);
                }
                if *source != ProgressSource::Q3 {
                    return Ok(());
                }
                Err(PlayerProgressError::BadEvent)
            }
            Self::MatchCompleted { map, score, .. } => {
                if map.is_empty() {
                    return Err(PlayerProgressError::BadMap);
                }
                if score.is_finite() {
                    return Ok(());
                }
                Err(PlayerProgressError::BadEvent)
            }
        }
    }
}

/// Durable identity: `JSON.stringify([source, participant, event])`.
fn identity(event: &PlayerProgressEvent) -> String {
    stringify(&Json::Array(vec![
        Json::String(event.source().as_str().to_string()),
        Json::String(event.participant().to_string()),
        Json::String(event.event().to_string()),
    ]))
}

fn decode_json(value: &Json) -> Result<PlayerProgressEvent, PlayerProgressError> {
    let source = match value.get("source") {
        Some(Json::String(text)) => ProgressSource::parse(text),
        _ => None,
    };
    let participant = match value.get("participant") {
        Some(Json::String(text)) => Some(text.as_str()),
        _ => None,
    };
    let event = match value.get("event") {
        Some(Json::String(text)) => Some(text.as_str()),
        _ => None,
    };
    let (Some(source), Some(participant), Some(event)) = (source, participant, event) else {
        return Err(PlayerProgressError::BadIdentity);
    };
    if participant.is_empty() || event.is_empty() {
        return Err(PlayerProgressError::BadIdentity);
    }
    let kind = match value.get("kind") {
        Some(Json::String(kind)) => Some(kind.as_str()),
        _ => None,
    };
    if kind == Some("achievement") && source != ProgressSource::Q3 {
        if let Some(Json::String(award)) = value.get("award") {
            if !award.is_empty() {
                return Ok(PlayerProgressEvent::Achievement {
                    source,
                    participant: participant.to_string(),
                    event: event.to_string(),
                    award: award.clone(),
                });
            }
        }
    }
    let map = match value.get("map") {
        Some(Json::String(map)) if !map.is_empty() => map.clone(),
        _ => return Err(PlayerProgressError::BadMap),
    };
    if kind == Some("level-completed") && source != ProgressSource::Q3 {
        return Ok(PlayerProgressEvent::LevelCompleted {
            source,
            participant: participant.to_string(),
            event: event.to_string(),
            map,
        });
    }
    if kind == Some("match-completed") {
        if let Some(Json::Number(score)) = value.get("score") {
            if score.is_finite() {
                return Ok(PlayerProgressEvent::MatchCompleted {
                    source,
                    participant: participant.to_string(),
                    event: event.to_string(),
                    map,
                    score: *score,
                });
            }
        }
    }
    Err(PlayerProgressError::BadEvent)
}

fn encode(event: &PlayerProgressEvent) -> Json {
    let mut members = vec![
        ("kind".to_string(), Json::String(event.kind().to_string())),
        ("source".to_string(), Json::String(event.source().as_str().to_string())),
        ("participant".to_string(), Json::String(event.participant().to_string())),
        ("event".to_string(), Json::String(event.event().to_string())),
    ];
    match event {
        PlayerProgressEvent::Achievement { award, .. } => {
            members.push(("award".to_string(), Json::String(award.clone())));
        }
        PlayerProgressEvent::LevelCompleted { map, .. } => {
            members.push(("map".to_string(), Json::String(map.clone())));
        }
        PlayerProgressEvent::MatchCompleted { map, score, .. } => {
            members.push(("map".to_string(), Json::String(map.clone())));
            members.push(("score".to_string(), Json::Number(*score)));
        }
    }
    Json::Object(members)
}

/// One application-owned writer over a progress file.
#[derive(Debug)]
pub struct PlayerProgressStore {
    file: PathBuf,
    events: Vec<PlayerProgressEvent>,
    index: HashMap<String, usize>,
}

impl PlayerProgressStore {
    /// Open (or create on first write) the store at `file`. A missing file
    /// yields an empty store; any other failure propagates.
    pub fn open(file: &Path) -> Result<Self, PlayerProgressError> {
        let mut store = Self {
            file: file.to_path_buf(),
            events: Vec::new(),
            index: HashMap::new(),
        };
        let text = match std::fs::read_to_string(file) {
            Ok(text) => text,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(store),
            Err(error) => return Err(error.into()),
        };
        let value = parse_json(&text)?;
        let Json::Object(_) = value else {
            return Err(PlayerProgressError::BadFile);
        };
        if !matches!(value.get("version"), Some(Json::Number(version)) if *version == 1.0) {
            return Err(PlayerProgressError::BadFile);
        }
        let Some(Json::Array(items)) = value.get("events") else {
            return Err(PlayerProgressError::BadFile);
        };
        for item in items {
            let event = decode_json(item)?;
            let key = identity(&event);
            if store.index.contains_key(&key) {
                return Err(PlayerProgressError::Duplicate);
            }
            store.index.insert(key, store.events.len());
            store.events.push(event);
        }
        Ok(store)
    }

    /// Events owned by one participant, in record order.
    #[must_use]
    pub fn list(&self, participant: &str) -> Vec<PlayerProgressEvent> {
        self.events
            .iter()
            .filter(|event| event.participant() == participant)
            .cloned()
            .collect()
    }

    /// Durably record one event. Returns `false` when the identity already
    /// exists (the original durable result is retained).
    pub fn record(&mut self, event: PlayerProgressEvent) -> Result<bool, PlayerProgressError> {
        event.check()?;
        let key = identity(&event);
        if self.index.contains_key(&key) {
            return Ok(false);
        }
        let mut pending = self.events.clone();
        pending.push(event.clone());
        if let Some(parent) = self.file.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)?;
            }
        }
        let mut temporary = self.file.as_os_str().to_owned();
        temporary.push(".pending");
        let payload = stringify(&Json::Object(vec![
            ("version".to_string(), Json::Number(1.0)),
            ("events".to_string(), Json::Array(pending.iter().map(encode).collect())),
        ])) + "\n";
        std::fs::write(&temporary, payload)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&temporary, std::fs::Permissions::from_mode(0o600))?;
        }
        std::fs::rename(&temporary, &self.file)?;
        self.index.insert(key, self.events.len());
        self.events.push(event);
        Ok(true)
    }

    /// No-op: sync writes are durable on return.
    pub fn flush(&self) {}
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_TEMP: AtomicU64 = AtomicU64::new(1);

    fn temp_file(name: &str) -> PathBuf {
        let id = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!("qa-player-progress-{name}-{}-{id}.json", std::process::id()))
    }

    fn achievement() -> PlayerProgressEvent {
        PlayerProgressEvent::Achievement {
            source: ProgressSource::Q1,
            participant: "player".to_string(),
            event: "first-blood".to_string(),
            award: "First Blood".to_string(),
        }
    }

    #[test]
    fn missing_file_opens_empty() {
        let file = temp_file("missing");
        let _ = std::fs::remove_file(&file);
        let store = PlayerProgressStore::open(&file).unwrap();
        assert!(store.list("player").is_empty());
    }

    #[test]
    fn record_lists_and_deduplicates() {
        let file = temp_file("record");
        let _ = std::fs::remove_file(&file);
        let mut store = PlayerProgressStore::open(&file).unwrap();
        assert!(store.record(achievement()).unwrap());
        assert!(!store.record(achievement()).unwrap());
        let rows = store.list("player");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].kind(), "achievement");
        assert!(store.list("other").is_empty());
        store.flush();
        let reopened = PlayerProgressStore::open(&file).unwrap();
        assert_eq!(reopened.list("player"), rows);
        let _ = std::fs::remove_file(&file);
    }

    #[test]
    fn record_rejects_invalid_typed_input() {
        let file = temp_file("invalid");
        let _ = std::fs::remove_file(&file);
        let mut store = PlayerProgressStore::open(&file).unwrap();
        let empty_participant = PlayerProgressEvent::LevelCompleted {
            source: ProgressSource::Q2,
            participant: String::new(),
            event: "e1m1".to_string(),
            map: "e1m1".to_string(),
        };
        assert_eq!(
            store.record(empty_participant).unwrap_err(),
            PlayerProgressError::BadIdentity
        );
        let q3_achievement = PlayerProgressEvent::Achievement {
            source: ProgressSource::Q3,
            participant: "player".to_string(),
            event: "x".to_string(),
            award: "X".to_string(),
        };
        assert_eq!(store.record(q3_achievement).unwrap_err(), PlayerProgressError::BadMap);
        let nan_score = PlayerProgressEvent::MatchCompleted {
            source: ProgressSource::Q3,
            participant: "player".to_string(),
            event: "m".to_string(),
            map: "q3dm1".to_string(),
            score: f64::NAN,
        };
        assert_eq!(store.record(nan_score).unwrap_err(), PlayerProgressError::BadEvent);
        assert!(!file.exists());
    }

    #[test]
    fn open_rejects_bad_files() {
        for (name, text, expected) in [
            ("scalar", "42", PlayerProgressError::BadFile),
            ("version", r#"{"version":2,"events":[]}"#, PlayerProgressError::BadFile),
            ("events", r#"{"version":1,"events":{}}"#, PlayerProgressError::BadFile),
            (
                "entry",
                r#"{"version":1,"events":[{"kind":"achievement"}]}"#,
                PlayerProgressError::BadIdentity,
            ),
            (
                "duplicate",
                r#"{"version":1,"events":[
                  {"kind":"achievement","source":"q1","participant":"p","event":"e","award":"a"},
                  {"kind":"achievement","source":"q1","participant":"p","event":"e","award":"a"}]}"#,
                PlayerProgressError::Duplicate,
            ),
        ] {
            let file = temp_file(name);
            std::fs::write(&file, text).unwrap();
            assert_eq!(PlayerProgressStore::open(&file).unwrap_err(), expected, "{name}");
            let _ = std::fs::remove_file(&file);
        }
        let file = temp_file("json");
        std::fs::write(&file, "{oops").unwrap();
        assert!(matches!(
            PlayerProgressStore::open(&file).unwrap_err(),
            PlayerProgressError::Json(_)
        ));
        let _ = std::fs::remove_file(&file);
    }
}
