//! Seat media captions: sidecar selection and playback gating.
//!
//! Donor provenance: `src/text/media-captions.ts`. Sync port: reads are
//! caller-driven (`prepare` pulls sidecars through a sync reader).

use qa_core::identity::SeatId;

use super::captions::{parse_subtitle_text, ActiveCaption, CaptionKind, CaptionPreferences, CaptionTimeline};
use super::localization::{LocalizationCatalog, LocalizationProfile};
use super::resources::{load_localization_resources, LocalizationReader};
use crate::ClientError;

/// Subtitle sidecar suffixes by language.
fn language_suffix(language: &str) -> Option<&'static str> {
    match language {
        "french" => Some("fr"),
        "german" => Some("de"),
        "italian" => Some("it"),
        "spanish" => Some("es"),
        "russian" => Some("ru"),
        "polish" => Some("pl"),
        "portuguese" => Some("pt"),
        "japanese" => Some("ja"),
        "korean" => Some("ko"),
        "chinese" => Some("zh"),
        _ => None,
    }
}

/// Candidate sidecar paths (`subtitlePaths`).
#[must_use]
pub fn subtitle_paths(source: &str, language: &str) -> Vec<String> {
    let stem = match source.rfind('.') {
        Some(dot) if source[dot..].find('/').is_none() => &source[..dot],
        _ => source,
    };
    let mut paths = Vec::new();
    if let Some(suffix) = language_suffix(language) {
        paths.push(format!("{stem}_{suffix}.srt"));
        paths.push(format!("{stem}_{suffix}.vtt"));
    }
    paths.push(format!("{stem}.srt"));
    paths.push(format!("{stem}.vtt"));
    paths
}

/// Cinematic status for caption gating.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaptionPlaybackStatus {
    /// Playing.
    Playing,
    /// Paused.
    Paused,
    /// Held.
    Held,
    /// Ended.
    Ended,
    /// Stopped.
    Stopped,
}

/// Caption playback state (`CaptionPlaybackState`).
#[derive(Debug, Clone, PartialEq)]
pub struct CaptionPlaybackState {
    /// Source.
    pub source: String,
    /// Source time in milliseconds.
    pub source_time_ms: f64,
    /// Status.
    pub status: CaptionPlaybackStatus,
}

/// Seat media captions (`SeatMediaCaptions`, sync).
pub struct SeatMediaCaptions<'a> {
    seat: SeatId,
    localization: Option<LocalizationCatalog>,
    owned: Option<LocalizationCatalog>,
    timeline_cues: Vec<super::captions::CaptionCue>,
    prepared: String,
    kind: CaptionKind,
    profile: LocalizationProfile,
    reader: &'a mut dyn LocalizationReader,
}

impl<'a> SeatMediaCaptions<'a> {
    /// New captions.
    #[must_use]
    pub fn new(
        seat: SeatId,
        reader: &'a mut dyn LocalizationReader,
        localization: Option<LocalizationCatalog>,
        kind: CaptionKind,
        profile: LocalizationProfile,
    ) -> Self {
        Self {
            seat,
            localization,
            owned: None,
            timeline_cues: Vec::new(),
            prepared: String::new(),
            kind,
            profile,
            reader,
        }
    }

    fn catalog(&self) -> &LocalizationCatalog {
        self.localization
            .as_ref()
            .or(self.owned.as_ref())
            .expect("captions require a catalog after prepare")
    }

    /// Prepare sidecars for a source and language (`prepare`).
    pub fn prepare(&mut self, source: &str, language: &str) -> Result<(), ClientError> {
        let key = format!("{source}:{language}");
        if key == self.prepared {
            return Ok(());
        }
        self.prepared.clear();
        self.timeline_cues.clear();
        for path in subtitle_paths(source, language) {
            let bytes = self.reader.read(&path);
            let Some(bytes) = bytes else {
                continue;
            };
            if self.localization.is_none() && self.owned.is_none() {
                let seat = self.seat.clone();
                let profile = self.profile;
                let reader: &mut dyn LocalizationReader = &mut *self.reader;
                self.owned = Some(load_localization_resources(seat, language, reader, profile));
            }
            let cues = parse_subtitle_text(&String::from_utf8_lossy(&bytes), &path)?
                .into_iter()
                .map(|cue| super::captions::CaptionCue { kind: self.kind, ..cue })
                .collect();
            self.timeline_cues = cues;
            self.prepared = key;
            return Ok(());
        }
        self.prepared = key;
        Ok(())
    }

    /// Active captions (`active`).
    pub fn active(
        &self,
        state: &CaptionPlaybackState,
        preferences: CaptionPreferences,
    ) -> Result<Vec<ActiveCaption>, ClientError> {
        if !self.prepared.starts_with(&format!("{}:", state.source))
            || matches!(
                state.status,
                CaptionPlaybackStatus::Ended | CaptionPlaybackStatus::Stopped
            )
        {
            return Ok(Vec::new());
        }
        let mut timeline = CaptionTimeline::new(self.seat.clone(), self.catalog())?;
        timeline.preferences = preferences;
        timeline.replace(self.timeline_cues.clone())?;
        timeline.active_at(state.source_time_ms)
    }

    /// Clear prepared sidecars.
    pub fn clear(&mut self) {
        self.prepared.clear();
        self.timeline_cues.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_prefer_language_suffix() {
        assert_eq!(
            subtitle_paths("video/m.roq", "french"),
            vec![
                "video/m_fr.srt".to_string(),
                "video/m_fr.vtt".to_string(),
                "video/m.srt".to_string(),
                "video/m.vtt".to_string(),
            ]
        );
        assert_eq!(
            subtitle_paths("video/m.roq", "english"),
            vec!["video/m.srt".to_string(), "video/m.vtt".to_string(),]
        );
    }
}
