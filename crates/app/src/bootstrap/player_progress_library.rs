//! Library-menu view over retained player progress.
//!
//! Sync port of donor `src/app/bootstrap/player-progress-library.ts`.
//! Reads the retained profile's existing store through a caller-supplied
//! opener; never creates a second writer. The donor's async generation
//! counter collapses away (no await gap can reorder sync loads); the
//! participant stability re-check is mirrored.

use thiserror::Error;

use qa_client::ui::library::menu::{LibraryEntry, LibraryMenuService};

use super::player_progress::{PlayerProgressEvent, PlayerProgressStore};
use crate::settings::json::{stringify, Json};

/// Failure to open the retained progress store.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum PlayerProgressLibraryError {
    /// The store opener failed; the message is shown in the status line.
    #[error("{0}")]
    Store(String),
}

fn entry(event: &PlayerProgressEvent) -> LibraryEntry {
    let id = stringify(&Json::Array(vec![
        Json::String(event.source().as_str().to_string()),
        Json::String(event.participant().to_string()),
        Json::String(event.event().to_string()),
    ]));
    let family = event.source().as_str().to_uppercase();
    match event {
        PlayerProgressEvent::Achievement { award, .. } => LibraryEntry {
            id,
            label: award.clone(),
            detail: Some(format!("{family} · Achievement earned")),
            unavailable: None,
        },
        PlayerProgressEvent::LevelCompleted { map, .. } => LibraryEntry {
            id,
            label: map.clone(),
            detail: Some(format!("{family} · Level completed")),
            unavailable: None,
        },
        PlayerProgressEvent::MatchCompleted { map, score, .. } => LibraryEntry {
            id,
            label: map.clone(),
            detail: Some(format!("{family} · Match completed · Score {score}")),
            unavailable: None,
        },
    }
}

/// Library menu service over one participant's progress rows.
pub struct PlayerProgressLibrary<S, P> {
    open_store: S,
    participant: P,
    rows: Vec<LibraryEntry>,
    message: String,
}

impl<S, P> PlayerProgressLibrary<S, P>
where
    S: FnMut() -> Result<PlayerProgressStore, PlayerProgressLibraryError>,
    P: FnMut() -> String,
{
    /// Build over a store opener and the current participant selector.
    pub fn new(open_store: S, participant: P) -> Self {
        Self {
            open_store,
            participant,
            rows: Vec::new(),
            message: String::new(),
        }
    }

    /// Current rows.
    #[must_use]
    pub fn entries(&self) -> &[LibraryEntry] {
        &self.rows
    }

    /// Status line text.
    #[must_use]
    pub fn status(&self) -> &str {
        &self.message
    }

    /// Reload rows from the store.
    pub fn refresh(&mut self) {
        self.load();
    }

    /// Load rows; failures land in the status line, never propagate.
    pub fn load(&mut self) {
        let participant = (self.participant)();
        self.rows = Vec::new();
        self.message = "Loading progress...".to_string();
        match (self.open_store)() {
            Ok(store) => {
                if participant != (self.participant)() {
                    return;
                }
                self.rows = store.list(&participant).iter().map(entry).collect();
                self.message = if self.rows.is_empty() {
                    "No recorded achievements or completed levels for this player.".to_string()
                } else {
                    format!("{} progress records", self.rows.len())
                };
            }
            Err(error) => {
                if participant != (self.participant)() {
                    return;
                }
                self.rows = Vec::new();
                self.message = format!("Cannot read progress: {error}");
            }
        }
    }

    /// Show one row's label and detail in the status line.
    pub fn activate(&mut self, id: &str) {
        if let Some(row) = self.rows.iter().find(|entry| entry.id == id) {
            self.message = format!("{}: {}", row.label, row.detail.as_deref().unwrap_or(""));
        }
    }
}

impl<S, P> LibraryMenuService for PlayerProgressLibrary<S, P>
where
    S: FnMut() -> Result<PlayerProgressStore, PlayerProgressLibraryError>,
    P: FnMut() -> String,
{
    fn entries(&self) -> Vec<LibraryEntry> {
        self.rows.clone()
    }

    fn status(&self) -> String {
        self.message.clone()
    }

    fn refresh(&mut self) {
        PlayerProgressLibrary::load(self);
    }

    fn activate(&mut self, id: &str) {
        PlayerProgressLibrary::activate(self, id);
    }
}

#[cfg(test)]
mod tests {
    use super::super::player_progress::{PlayerProgressStore, ProgressSource};
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_TEMP: AtomicU64 = AtomicU64::new(1);

    fn seed_store() -> std::path::PathBuf {
        let id = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
        let file = std::env::temp_dir().join(format!("qa-progress-library-{}-{id}.json", std::process::id()));
        let _ = std::fs::remove_file(&file);
        let mut store = PlayerProgressStore::open(&file).unwrap();
        store
            .record(PlayerProgressEvent::Achievement {
                source: ProgressSource::Q1,
                participant: "player".to_string(),
                event: "e1".to_string(),
                award: "First Blood".to_string(),
            })
            .unwrap();
        store
            .record(PlayerProgressEvent::MatchCompleted {
                source: ProgressSource::Q3,
                participant: "player".to_string(),
                event: "m1".to_string(),
                map: "q3dm1".to_string(),
                score: 25.0,
            })
            .unwrap();
        file
    }

    #[test]
    fn loads_rows_and_status() {
        let file = seed_store();
        let mut view = PlayerProgressLibrary::new(
            || PlayerProgressStore::open(&file).map_err(|error| PlayerProgressLibraryError::Store(error.to_string())),
            || "player".to_string(),
        );
        view.refresh();
        assert_eq!(view.entries().len(), 2);
        assert_eq!(view.status(), "2 progress records");
        assert_eq!(view.entries()[0].label, "First Blood");
        assert_eq!(view.entries()[0].detail.as_deref(), Some("Q1 · Achievement earned"));
        assert_eq!(
            view.entries()[1].detail.as_deref(),
            Some("Q3 · Match completed · Score 25")
        );
        let id = view.entries()[0].id.clone();
        view.activate(&id);
        assert_eq!(view.status(), "First Blood: Q1 · Achievement earned");
        let _ = std::fs::remove_file(&file);
    }

    #[test]
    fn empty_and_error_states() {
        let file = seed_store();
        let mut view = PlayerProgressLibrary::new(
            || PlayerProgressStore::open(&file).map_err(|error| PlayerProgressLibraryError::Store(error.to_string())),
            || "nobody".to_string(),
        );
        view.load();
        assert!(view.entries().is_empty());
        assert_eq!(
            view.status(),
            "No recorded achievements or completed levels for this player."
        );
        let before = view.status().to_string();
        view.activate("missing");
        assert_eq!(view.status(), before);

        let mut failing = PlayerProgressLibrary::new(
            || Err::<PlayerProgressStore, _>(PlayerProgressLibraryError::Store("locked".to_string())),
            || "player".to_string(),
        );
        failing.load();
        assert!(failing.entries().is_empty());
        assert_eq!(failing.status(), "Cannot read progress: locked");
        let _ = std::fs::remove_file(&file);
    }

    #[test]
    fn serves_library_menu_contract() {
        let file = seed_store();
        let mut view = PlayerProgressLibrary::new(
            || PlayerProgressStore::open(&file).map_err(|error| PlayerProgressLibraryError::Store(error.to_string())),
            || "player".to_string(),
        );
        let service: &mut dyn LibraryMenuService = &mut view;
        service.refresh();
        assert_eq!(service.entries().len(), 2);
        assert_eq!(service.status(), "2 progress records");
        let id = service.entries()[0].id.clone();
        service.activate(&id);
        assert!(service.status().starts_with("First Blood: "));
        let _ = std::fs::remove_file(&file);
    }
}
