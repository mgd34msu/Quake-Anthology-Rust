//! Per-seat sound captions following mixer voices.
//!
//! Port of Quake-Anthology-TS `src/app/bootstrap/sound-captions.ts`
//! (`SeatSoundCaptions`). The audio voice observer, seat identity, caption catalog, media
//! captions, and content-mount siblings are unported, so they are absorbed as local traits and
//! snapshot types. Sync port: the donor's async catalog reads become a caller-supplied sync
//! loader, and voice events arrive through explicit `note_*` calls instead of an observer
//! subscription. Source-authored sound sidecars follow the actual per-seat mixer voice,
//! including replacement.

use std::collections::HashMap;

/// Sound sidecar reference (donor `reference` with content, requested path, and seat).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SoundCaptionReference {
    /// Content identity owning the sidecar.
    pub content: String,
    /// Requested sound path.
    pub requested_path: String,
}

/// Started mixer voice (donor `start` voice event for this seat).
#[derive(Debug, Clone, PartialEq)]
pub struct SoundCaptionVoiceStart {
    /// Voice id.
    pub voice_id: u64,
    /// Output sample where the voice starts.
    pub output_sample: u64,
    /// Source offset in seconds.
    pub source_offset_seconds: f64,
    /// Voice sample rate.
    pub sample_rate: f64,
    /// Sidecar reference, when the sound carries one.
    pub reference: Option<SoundCaptionReference>,
}

/// Voice clock sample (donor `voiceClock`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VoiceClockSample {
    /// Current output sample.
    pub output_sample: u64,
    /// Whether playback is paused.
    pub paused: bool,
}

/// Caption preferences passed through to the catalog (donor `CaptionPreferences`).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SoundCaptionPreferences {
    /// Whether captions are enabled.
    pub enabled: bool,
}

/// One active caption (donor `ActiveCaption`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SoundActiveCaption {
    /// Caption source path.
    pub source: String,
    /// Caption text.
    pub text: String,
}

/// Absorbed per-sound media caption catalog (donor `SeatMediaCaptions`).
pub trait SoundMediaCaptions {
    /// Captions active at a source timestamp.
    fn active(
        &self,
        source: &str,
        source_time_milliseconds: f64,
        paused: bool,
        preferences: &SoundCaptionPreferences,
    ) -> Vec<SoundActiveCaption>;
    /// Release catalog resources (donor `clear`).
    fn clear(&mut self);
}

struct CaptionVoice<Catalog> {
    event: SoundCaptionVoiceStart,
    captions: Option<Catalog>,
    language: String,
    stop_sample: Option<u64>,
}

/// Per-seat sound captions (donor `SeatSoundCaptions`).
pub struct SeatSoundCaptions<Catalog> {
    voices: HashMap<u64, CaptionVoice<Catalog>>,
    catalogs: HashMap<String, Catalog>,
    closed: bool,
}

impl<Catalog> SeatSoundCaptions<Catalog> {
    /// Create empty per-seat captions.
    #[must_use]
    pub fn new() -> Self {
        Self {
            voices: HashMap::new(),
            catalogs: HashMap::new(),
            closed: false,
        }
    }

    /// Record a started voice for this seat (donor observer `start` branch).
    pub fn note_voice_start(&mut self, event: SoundCaptionVoiceStart) {
        if self.closed {
            return;
        }
        self.voices.insert(
            event.voice_id,
            CaptionVoice {
                event,
                captions: None,
                language: String::new(),
                stop_sample: None,
            },
        );
    }

    /// Record a stopped voice for this seat (donor observer stop branch).
    pub fn note_voice_stop(&mut self, voice_id: u64, output_sample: u64) {
        if let Some(voice) = self.voices.get_mut(&voice_id) {
            voice.stop_sample = Some(output_sample);
        }
    }

    /// Prepare catalogs for live voices, dropping finished ones.
    ///
    /// `load` builds the catalog for a sidecar reference and language; it replaces the donor's
    /// `content.forContent` plus `SeatMediaCaptions.prepare` chain.
    pub fn prepare(
        &mut self,
        clock: VoiceClockSample,
        language: &str,
        load: &mut dyn FnMut(&SoundCaptionReference, &str) -> Catalog,
    ) where
        Catalog: Clone,
    {
        if self.closed {
            return;
        }
        let ids: Vec<u64> = self.voices.keys().copied().collect();
        for id in ids {
            let expired = self
                .voices
                .get(&id)
                .and_then(|voice| voice.stop_sample)
                .is_some_and(|stop| clock.output_sample >= stop);
            if expired {
                self.voices.remove(&id);
                continue;
            }
            let reference = match self.voices.get(&id).and_then(|voice| voice.event.reference.clone()) {
                Some(reference) => reference,
                None => continue,
            };
            let fresh = self
                .voices
                .get(&id)
                .is_some_and(|voice| voice.captions.is_some() && voice.language == language);
            if fresh {
                continue;
            }
            let key = format!("{}:{}:{language}", reference.content, reference.requested_path);
            if !self.catalogs.contains_key(&key) {
                let catalog = load(&reference, language);
                if self.closed || !self.voices.contains_key(&id) {
                    continue;
                }
                self.catalogs.insert(key.clone(), catalog);
            }
            if self.closed || !self.voices.contains_key(&id) {
                continue;
            }
            if let Some(voice) = self.voices.get_mut(&id) {
                voice.language = language.to_string();
            }
            self.link_catalog(id, &key);
        }
    }

    fn link_catalog(&mut self, id: u64, key: &str)
    where
        Catalog: Clone,
    {
        // The donor shares one catalog object between the map and the voice; catalogs are
        // never mutated after prepare (only cleared at close), so a clone is equivalent.
        let catalog = self.catalogs.get(key).cloned();
        if let (Some(catalog), Some(voice)) = (catalog, self.voices.get_mut(&id)) {
            voice.captions = Some(catalog);
        }
    }

    /// Captions active at the current clock sample.
    pub fn active(&self, clock: VoiceClockSample, preferences: &SoundCaptionPreferences) -> Vec<SoundActiveCaption>
    where
        Catalog: SoundMediaCaptions,
    {
        let mut out = Vec::new();
        for voice in self.voices.values() {
            let Some(captions) = voice.captions.as_ref() else {
                continue;
            };
            if clock.output_sample < voice.event.output_sample {
                continue;
            }
            if voice.stop_sample.is_some_and(|stop| clock.output_sample >= stop) {
                continue;
            }
            let Some(reference) = voice.event.reference.as_ref() else {
                continue;
            };
            let source_time_milliseconds = voice.event.source_offset_seconds * 1000.0
                + (clock.output_sample - voice.event.output_sample) as f64 * 1000.0 / voice.event.sample_rate;
            out.extend(captions.active(
                &reference.requested_path,
                source_time_milliseconds,
                clock.paused,
                preferences,
            ));
        }
        out
    }

    /// Release every voice and catalog (donor `close`).
    pub fn close(&mut self)
    where
        Catalog: SoundMediaCaptions,
    {
        self.closed = true;
        for captions in self.catalogs.values_mut() {
            captions.clear();
        }
        self.catalogs.clear();
        self.voices.clear();
    }

    /// Whether the captions are closed.
    #[must_use]
    pub fn is_closed(&self) -> bool {
        self.closed
    }
}

impl<Catalog> Default for SeatSoundCaptions<Catalog> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Clone)]
    struct TestCatalog {
        cleared: bool,
    }

    impl SoundMediaCaptions for TestCatalog {
        fn active(
            &self,
            source: &str,
            source_time_milliseconds: f64,
            paused: bool,
            preferences: &SoundCaptionPreferences,
        ) -> Vec<SoundActiveCaption> {
            assert!(preferences.enabled);
            assert!(!paused);
            vec![SoundActiveCaption {
                source: source.to_string(),
                text: format!("caption@{source_time_milliseconds}"),
            }]
        }

        fn clear(&mut self) {
            self.cleared = true;
        }
    }

    fn start(reference: Option<SoundCaptionReference>) -> SoundCaptionVoiceStart {
        SoundCaptionVoiceStart {
            voice_id: 7,
            output_sample: 100,
            source_offset_seconds: 0.5,
            sample_rate: 1000.0,
            reference,
        }
    }

    #[test]
    fn prepare_links_catalog_and_active_reports_caption() {
        let mut captions = SeatSoundCaptions::new();
        captions.note_voice_start(start(Some(SoundCaptionReference {
            content: "base".to_string(),
            requested_path: "sfx/hit.wav".to_string(),
        })));
        let clock = VoiceClockSample {
            output_sample: 200,
            paused: false,
        };
        let mut loads = 0;
        captions.prepare(clock, "en", &mut |reference, language| {
            loads += 1;
            assert_eq!(reference.requested_path, "sfx/hit.wav");
            assert_eq!(language, "en");
            TestCatalog { cleared: false }
        });
        assert_eq!(loads, 1);
        let active = captions.active(clock, &SoundCaptionPreferences { enabled: true });
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].source, "sfx/hit.wav");
        assert_eq!(active[0].text, "caption@600");
    }

    #[test]
    fn prepare_skips_unreferenced_and_finished_voices() {
        let mut captions = SeatSoundCaptions::new();
        captions.note_voice_start(start(None));
        captions.note_voice_stop(7, 150);
        let clock = VoiceClockSample {
            output_sample: 200,
            paused: false,
        };
        captions.prepare(clock, "en", &mut |_, _| TestCatalog { cleared: false });
        assert!(captions
            .active(clock, &SoundCaptionPreferences { enabled: true })
            .is_empty());
    }

    #[test]
    fn prepare_ignores_future_voices_until_audible() {
        let mut captions = SeatSoundCaptions::new();
        let mut event = start(Some(SoundCaptionReference {
            content: "base".to_string(),
            requested_path: "sfx/hit.wav".to_string(),
        }));
        event.output_sample = 500;
        captions.note_voice_start(event);
        let clock = VoiceClockSample {
            output_sample: 200,
            paused: false,
        };
        captions.prepare(clock, "en", &mut |_, _| TestCatalog { cleared: false });
        assert!(captions
            .active(clock, &SoundCaptionPreferences { enabled: true })
            .is_empty());
    }

    #[test]
    fn close_releases_voices_and_catalogs() {
        let mut captions = SeatSoundCaptions::new();
        captions.note_voice_start(start(Some(SoundCaptionReference {
            content: "base".to_string(),
            requested_path: "sfx/hit.wav".to_string(),
        })));
        let clock = VoiceClockSample {
            output_sample: 200,
            paused: false,
        };
        captions.prepare(clock, "en", &mut |_, _| TestCatalog { cleared: false });
        captions.close();
        assert!(captions.is_closed());
        assert!(captions
            .active(clock, &SoundCaptionPreferences { enabled: true })
            .is_empty());
        captions.note_voice_start(start(None));
        assert!(captions
            .active(clock, &SoundCaptionPreferences { enabled: true })
            .is_empty());
    }
}
