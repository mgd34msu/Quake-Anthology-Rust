//! Captions and subtitles (`CaptionTimeline`, SRT/WebVTT).
//!
//! Donor provenance: `src/text/captions.ts`.

use std::collections::BTreeMap;

use qa_core::identity::SeatId;

use super::localization::LocalizationCatalog;
use crate::ClientError;

/// A caption cue (`CaptionCue`).
#[derive(Debug, Clone, PartialEq)]
pub struct CaptionCue {
    /// Cue id.
    pub id: String,
    /// Cue kind.
    pub kind: CaptionKind,
    /// Start in milliseconds.
    pub start_ms: f64,
    /// Duration in milliseconds.
    pub duration_ms: f64,
    /// Text (localization key or literal).
    pub text: String,
    /// Speaker (or `None`).
    pub speaker: Option<String>,
    /// Format arguments.
    pub arguments: Vec<String>,
}

/// Cue kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaptionKind {
    /// Subtitle.
    Subtitle,
    /// Caption.
    Caption,
}

/// An active cue (`ActiveCaption`).
#[derive(Debug, Clone, PartialEq)]
pub struct ActiveCaption {
    /// Cue.
    pub cue: CaptionCue,
    /// Localized text.
    pub localized_text: String,
    /// Localized speaker.
    pub localized_speaker: Option<String>,
}

/// Caption preferences (`CaptionPreferences`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CaptionPreferences {
    /// Subtitles.
    pub subtitles: bool,
    /// Sound captions.
    pub sound_captions: bool,
    /// Speakers.
    pub speakers: bool,
}

impl Default for CaptionPreferences {
    fn default() -> Self {
        Self {
            subtitles: true,
            sound_captions: true,
            speakers: true,
        }
    }
}

fn validate_cue(cue: &CaptionCue) -> Result<(), ClientError> {
    if cue.id.is_empty()
        || !cue.start_ms.is_finite()
        || cue.start_ms < 0.0
        || !cue.duration_ms.is_finite()
        || cue.duration_ms < 0.0
        || !(cue.start_ms + cue.duration_ms).is_finite()
    {
        return Err(ClientError::BadText("Invalid caption cue interval".to_string()));
    }
    Ok(())
}

/// A caption timeline (`CaptionTimeline`).
#[derive(Debug)]
pub struct CaptionTimeline<'a> {
    cues: BTreeMap<String, CaptionCue>,
    /// Preferences.
    pub preferences: CaptionPreferences,
    seat: SeatId,
    localization: &'a LocalizationCatalog,
}

impl<'a> CaptionTimeline<'a> {
    /// New timeline.
    pub fn new(seat: SeatId, localization: &'a LocalizationCatalog) -> Result<Self, ClientError> {
        if seat != localization.seat {
            return Err(ClientError::BadText(
                "Caption localization belongs to a different seat".to_string(),
            ));
        }
        Ok(Self {
            cues: BTreeMap::new(),
            preferences: CaptionPreferences::default(),
            seat,
            localization,
        })
    }

    /// Replace cues.
    pub fn replace(&mut self, cues: Vec<CaptionCue>) -> Result<(), ClientError> {
        for cue in &cues {
            validate_cue(cue)?;
        }
        self.cues.clear();
        for cue in cues {
            self.cues.insert(cue.id.clone(), cue);
        }
        Ok(())
    }

    /// Add a cue.
    pub fn add(&mut self, cue: CaptionCue) -> Result<(), ClientError> {
        validate_cue(&cue)?;
        self.cues.insert(cue.id.clone(), cue);
        Ok(())
    }

    /// Remove a cue.
    pub fn remove(&mut self, id: &str) -> bool {
        self.cues.remove(id).is_some()
    }

    /// Clear cues.
    pub fn clear(&mut self) {
        self.cues.clear();
    }

    /// Active cues at a playback time (`activeAt`).
    pub fn active_at(&self, playback_ms: f64) -> Result<Vec<ActiveCaption>, ClientError> {
        if !playback_ms.is_finite() {
            return Err(ClientError::BadText("Caption time must be finite".to_string()));
        }
        let mut active = Vec::new();
        for cue in self.cues.values() {
            if cue.kind == CaptionKind::Subtitle {
                if !self.preferences.subtitles {
                    continue;
                }
            } else if !self.preferences.sound_captions {
                continue;
            }
            if playback_ms < cue.start_ms || playback_ms >= cue.start_ms + cue.duration_ms {
                continue;
            }
            active.push(ActiveCaption {
                localized_text: self.localization.localize(&cue.text, &cue.arguments),
                localized_speaker: if !self.preferences.speakers {
                    None
                } else {
                    cue.speaker
                        .as_ref()
                        .map(|speaker| self.localization.localize(speaker, &[]))
                },
                cue: cue.clone(),
            });
        }
        active.sort_by(|left, right| {
            left.cue
                .start_ms
                .partial_cmp(&right.cue.start_ms)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        Ok(active)
    }

    /// Owning seat.
    #[must_use]
    pub const fn seat(&self) -> &SeatId {
        &self.seat
    }
}

fn timestamp(value: &str) -> Result<f64, ClientError> {
    let invalid = || ClientError::BadText(format!("Invalid subtitle timestamp {value}"));
    let value = value.trim();
    let (hours, rest) = match value.rfind(':') {
        Some(_) => {
            let parts: Vec<&str> = value.split(':').collect();
            if parts.len() == 3 {
                (parts[0].parse::<f64>().map_err(|_| invalid())?, format!("{}:{}", parts[1], parts[2]))
            } else if parts.len() == 2 {
                (0.0, value.to_string())
            } else {
                return Err(invalid());
            }
        }
        None => return Err(invalid()),
    };
    let rest = rest;
    let dot = rest.find(['.', ',']).ok_or_else(invalid)?;
    let (clock, millis) = rest.split_at(dot);
    let millis = millis[1..].parse::<f64>().map_err(|_| invalid())?;
    if millis >= 1000.0 || millis.fract() != 0.0 {
        return Err(invalid());
    }
    let clock_parts: Vec<&str> = clock.split(':').collect();
    if clock_parts.len() != 2 || clock_parts[0].len() != 2 || clock_parts[1].len() != 2 {
        return Err(invalid());
    }
    let minutes = clock_parts[0].parse::<f64>().map_err(|_| invalid())?;
    let seconds = clock_parts[1].parse::<f64>().map_err(|_| invalid())?;
    if minutes >= 60.0 || seconds >= 60.0 {
        return Err(invalid());
    }
    if hours.fract() != 0.0 || !hours.is_finite() {
        return Err(invalid());
    }
    Ok(((hours * 60.0 + minutes) * 60.0 + seconds) * 1000.0 + millis)
}

/// Parse SRT/WebVTT text (`parseSubtitleText`).
pub fn parse_subtitle_text(text: &str, namespace: &str) -> Result<Vec<CaptionCue>, ClientError> {
    let normalized = text
        .strip_prefix('\u{feff}')
        .unwrap_or(text)
        .replace("\r\n", "\n")
        .replace('\r', "\n");
    // Split on blank lines.
    let mut blocks: Vec<String> = Vec::new();
    let mut current = String::new();
    for line in normalized.split('\n') {
        if line.trim().is_empty() {
            if !current.is_empty() {
                blocks.push(core::mem::take(&mut current));
            }
        } else {
            if !current.is_empty() {
                current.push('\n');
            }
            current.push_str(line);
        }
    }
    if !current.is_empty() {
        blocks.push(current);
    }
    let mut cues = Vec::new();
    for block in blocks {
        let lines: Vec<&str> = block.split('\n').collect();
        let Some(timing_index) = lines.iter().position(|line| line.contains("-->")) else {
            continue;
        };
        let timing = lines[timing_index];
        let arrow = timing.find("-->").ok_or_else(|| {
            ClientError::BadText("Invalid subtitle cue timing".to_string())
        })?;
        let start = timing[..arrow].trim();
        let end = timing[arrow + 3..]
            .split_whitespace()
            .next()
            .ok_or_else(|| ClientError::BadText("Invalid subtitle cue timing".to_string()))?;
        if start.is_empty() || end.is_empty() {
            return Err(ClientError::BadText("Invalid subtitle cue timing".to_string()));
        }
        let start_ms = timestamp(start)?;
        let end_ms = timestamp(end)?;
        let body = lines[timing_index + 1..].join("\n");
        let body = body.trim_end().to_string();
        let (speaker, body) = parse_voice(&body);
        let cue = CaptionCue {
            id: format!("{namespace}:{}", cues.len()),
            kind: CaptionKind::Subtitle,
            start_ms,
            duration_ms: end_ms - start_ms,
            text: body,
            speaker,
            arguments: Vec::new(),
        };
        validate_cue(&cue)?;
        cues.push(cue);
    }
    Ok(cues)
}

fn parse_voice(text: &str) -> (Option<String>, String) {
    let Some(rest) = text.strip_prefix("<v ") else {
        return (None, text.to_string());
    };
    let Some(end) = rest.find('>') else {
        return (None, text.to_string());
    };
    let speaker = rest[..end].to_string();
    let mut body = rest[end + 1..].to_string();
    if let Some(stripped) = body.strip_suffix("</v>") {
        body = stripped.to_string();
    }
    (Some(speaker), body)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::text::localization::{LocalizationCatalog, LocalizationProfile};
    use qa_core::identity::IdentityOwner;

    fn catalog() -> (IdentityOwner, LocalizationCatalog) {
        let owner = IdentityOwner::create("test").unwrap();
        let catalog = LocalizationCatalog::new(owner.seat(0), LocalizationProfile::Q1Rerelease);
        (owner, catalog)
    }

    #[test]
    fn parses_srt() {
        let cues = parse_subtitle_text(
            "1\n00:00:01,000 --> 00:00:02,000\nHello\n\n2\n00:00:03.500 --> 00:00:04,000\n<v Bob>Hi</v>\n",
            "movie",
        )
        .unwrap();
        assert_eq!(cues.len(), 2);
        assert_eq!(cues[0].start_ms, 1000.0);
        assert_eq!(cues[0].duration_ms, 1000.0);
        assert_eq!(cues[1].speaker.as_deref(), Some("Bob"));
        assert_eq!(cues[1].text, "Hi");
    }

    #[test]
    fn bad_timestamp_is_an_error() {
        assert!(parse_subtitle_text("1\nnope --> nope\nx\n", "m").is_err());
    }

    #[test]
    fn timeline_filters_and_sorts() {
        let (_owner, catalog) = catalog();
        let mut timeline = CaptionTimeline::new(catalog.seat.clone(), &catalog).unwrap();
        timeline
            .replace(vec![
                CaptionCue {
                    id: "b".to_string(),
                    kind: CaptionKind::Subtitle,
                    start_ms: 2000.0,
                    duration_ms: 1000.0,
                    text: "B".to_string(),
                    speaker: None,
                    arguments: Vec::new(),
                },
                CaptionCue {
                    id: "a".to_string(),
                    kind: CaptionKind::Caption,
                    start_ms: 0.0,
                    duration_ms: 5000.0,
                    text: "A".to_string(),
                    speaker: None,
                    arguments: Vec::new(),
                },
            ])
            .unwrap();
        let active = timeline.active_at(2500.0).unwrap();
        assert_eq!(active.len(), 2);
        assert_eq!(active[0].cue.id, "a");
        timeline.preferences.subtitles = false;
        assert_eq!(timeline.active_at(2500.0).unwrap().len(), 1);
    }

    #[test]
    fn bad_cue_is_an_error() {
        let (_owner, catalog) = catalog();
        let mut timeline = CaptionTimeline::new(catalog.seat.clone(), &catalog).unwrap();
        assert!(
            timeline
                .add(CaptionCue {
                    id: String::new(),
                    kind: CaptionKind::Subtitle,
                    start_ms: 0.0,
                    duration_ms: 1.0,
                    text: String::new(),
                    speaker: None,
                    arguments: Vec::new(),
                })
                .is_err()
        );
    }
}
