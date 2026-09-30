//! Audio preferences load/save.
//!
//! Donor provenance: `src/app/bootstrap/audio-settings.ts`
//! (`loadAudioSettings`, `saveAudioSettings`, `AudioPreferences`).
//!
//! Sync port: the donor's async `ConfigStore` calls become the existing sync
//! [`crate::settings::config::ConfigStore`] methods. `MusicPreferences` and
//! `valid_menu_track` come from the sibling
//! [`crate::bootstrap::audio::playlist_settings`] module. A missing file
//! yields `Ok(None)` (the donor's `{}` empty partial); every other rule —
//! version literal, device-name shape, `0..=1` volumes, music validation,
//! output-format fallback — matches the donor exactly.

use qa_client::audio::error::AudioError;
use qa_client::audio::output::{audio_output_format, AudioOutputFormat, DEFAULT_AUDIO_OUTPUT_FORMAT};
use thiserror::Error;

use crate::bootstrap::audio::playlist_settings::{valid_menu_track, MusicPreferences};
use crate::settings::config::ConfigStore;
use crate::settings::json::{parse_json, stringify, Json};
use crate::settings::SettingsError;

/// Failure of an audio-settings operation.
#[derive(Debug, Error)]
pub enum AudioSettingsError {
    /// The stored document failed audio validation.
    #[error("Invalid audio preferences")]
    BadAudio,
    /// The stored music fields failed validation.
    #[error("Invalid music preferences")]
    BadMusic,
    /// Settings store or JSON failure.
    #[error(transparent)]
    Settings(#[from] SettingsError),
    /// Output-format validation failure.
    #[error(transparent)]
    Audio(#[from] AudioError),
}

/// Validated audio preferences (donor `AudioPreferences`).
///
/// The music fields stay optional exactly as in the donor: they are present
/// only when the stored document carries them.
#[derive(Debug, Clone, PartialEq)]
pub struct AudioPreferences {
    /// Shuffle mounted gameplay music.
    pub music_shuffle: Option<bool>,
    /// Menu track selection.
    pub menu_track: Option<String>,
    /// Selected output device name, or [`None`] for the default.
    pub device_name: Option<String>,
    /// Output sample format.
    pub output_format: AudioOutputFormat,
    /// Effects volume (`0..=1`).
    pub effects_volume: f64,
    /// Music volume (`0..=1`).
    pub music_volume: f64,
}

/// Save request (donor `saveAudioSettings` argument).
#[derive(Debug, Clone, PartialEq)]
pub struct AudioSaveRequest {
    /// Explicit music preferences; when [`None`], the stored file is read.
    pub music_preferences: Option<MusicPreferences>,
    /// Output format override; defaults when [`None`].
    pub output_format: Option<AudioOutputFormat>,
    /// Selected output device, or [`None`] for the default.
    pub selected_output: Option<String>,
    /// Effects volume (`0..=1`).
    pub effects_volume: f64,
    /// Music volume (`0..=1`).
    pub music_volume: f64,
}

fn json_int(value: &Json) -> Option<i64> {
    match value {
        Json::Number(n) if n.fract() == 0.0 && *n >= i64::MIN as f64 && *n <= i64::MAX as f64 => Some(*n as i64),
        _ => None,
    }
}

fn json_volume(value: &Json) -> Option<f64> {
    match value {
        Json::Number(n) if n.is_finite() && (0.0..=1.0).contains(n) => Some(*n),
        _ => None,
    }
}

fn output_format(value: &Json) -> Result<AudioOutputFormat, AudioSettingsError> {
    let parsed = value.get("sampleRate").and_then(json_int).and_then(|rate| {
        value.get("channels").and_then(json_int).and_then(|channels| {
            value
                .get("sampleBits")
                .and_then(json_int)
                .map(|bits| (rate, channels, bits))
        })
    });
    match parsed {
        Some((rate, channels, bits)) => Ok(audio_output_format(rate, channels, bits)?),
        None => Err(AudioSettingsError::Audio(AudioError::BadOutputFormat)),
    }
}

/// Validate a stored document (donor `preferences`).
fn preferences(value: &Json) -> Result<AudioPreferences, AudioSettingsError> {
    if !matches!(value.get("version"), Some(Json::Number(n)) if *n == 1.0) {
        return Err(AudioSettingsError::BadAudio);
    }
    let device_name = match value.get("deviceName") {
        Some(Json::Null) => None,
        Some(Json::String(name)) if !name.is_empty() && !name.contains('\0') => Some(name.clone()),
        _ => return Err(AudioSettingsError::BadAudio),
    };
    let Some(effects_volume) = value.get("effectsVolume").and_then(json_volume) else {
        return Err(AudioSettingsError::BadAudio);
    };
    let Some(music_volume) = value.get("musicVolume").and_then(json_volume) else {
        return Err(AudioSettingsError::BadAudio);
    };
    let music_shuffle = match value.get("musicShuffle") {
        None => None,
        Some(Json::Bool(shuffle)) => Some(*shuffle),
        Some(_) => return Err(AudioSettingsError::BadMusic),
    };
    let menu_track = match value.get("menuTrack") {
        None => None,
        Some(Json::String(track)) if valid_menu_track(track) => Some(track.clone()),
        Some(_) => return Err(AudioSettingsError::BadMusic),
    };
    let format = match value.get("outputFormat") {
        None => DEFAULT_AUDIO_OUTPUT_FORMAT,
        Some(format) => output_format(format)?,
    };
    Ok(AudioPreferences {
        music_shuffle,
        menu_track,
        device_name,
        output_format: format,
        effects_volume,
        music_volume,
    })
}

fn output_format_json(format: &AudioOutputFormat) -> Json {
    Json::Object(vec![
        ("sampleRate".to_string(), Json::Number(f64::from(format.sample_rate))),
        ("channels".to_string(), Json::Number(f64::from(format.channels))),
        ("sampleBits".to_string(), Json::Number(f64::from(format.sample_bits))),
    ])
}

/// Load stored preferences, or [`None`] when `audio.json` is absent.
pub fn load_audio_settings(store: &ConfigStore) -> Result<Option<AudioPreferences>, AudioSettingsError> {
    let Some(text) = store.load_text("audio.json")? else {
        return Ok(None);
    };
    preferences(&parse_json(&text)?).map(Some)
}

/// Save preferences, filling music fields from the store when omitted.
pub fn save_audio_settings(store: &ConfigStore, request: &AudioSaveRequest) -> Result<(), AudioSettingsError> {
    let (shuffle, track) = match &request.music_preferences {
        Some(music) => (music.music_shuffle, music.menu_track.clone()),
        None => match load_audio_settings(store)? {
            Some(stored) => (
                stored.music_shuffle.unwrap_or(false),
                stored.menu_track.unwrap_or_else(|| "auto".to_string()),
            ),
            None => (false, "auto".to_string()),
        },
    };
    let document = Json::Object(vec![
        ("version".to_string(), Json::Number(1.0)),
        ("musicShuffle".to_string(), Json::Bool(shuffle)),
        ("menuTrack".to_string(), Json::String(track)),
        (
            "outputFormat".to_string(),
            output_format_json(&request.output_format.unwrap_or(DEFAULT_AUDIO_OUTPUT_FORMAT)),
        ),
        (
            "deviceName".to_string(),
            match &request.selected_output {
                Some(name) => Json::String(name.clone()),
                None => Json::Null,
            },
        ),
        ("effectsVolume".to_string(), Json::Number(request.effects_volume)),
        ("musicVolume".to_string(), Json::Number(request.music_volume)),
    ]);
    let saved = preferences(&document)?;
    let mut members = vec![("version".to_string(), Json::Number(1.0))];
    if let Some(shuffle) = saved.music_shuffle {
        members.push(("musicShuffle".to_string(), Json::Bool(shuffle)));
    }
    if let Some(track) = saved.menu_track {
        members.push(("menuTrack".to_string(), Json::String(track)));
    }
    members.push(("outputFormat".to_string(), output_format_json(&saved.output_format)));
    members.push((
        "deviceName".to_string(),
        match saved.device_name {
            Some(name) => Json::String(name),
            None => Json::Null,
        },
    ));
    members.push(("effectsVolume".to_string(), Json::Number(saved.effects_volume)));
    members.push(("musicVolume".to_string(), Json::Number(saved.music_volume)));
    store.dump("audio.json", &format!("{}\n", stringify(&Json::Object(members))))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn store(name: &str) -> ConfigStore {
        let root: PathBuf = std::env::temp_dir().join(format!("qa-audio-settings-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        ConfigStore::new(root)
    }

    fn request() -> AudioSaveRequest {
        AudioSaveRequest {
            music_preferences: Some(MusicPreferences {
                music_shuffle: true,
                menu_track: "auto".to_string(),
            }),
            output_format: None,
            selected_output: None,
            effects_volume: 0.8,
            music_volume: 0.6,
        }
    }

    #[test]
    fn missing_file_loads_none() {
        assert_eq!(load_audio_settings(&store("missing")).unwrap(), None);
    }

    #[test]
    fn save_then_load_roundtrip() {
        let store = store("roundtrip");
        save_audio_settings(&store, &request()).unwrap();
        let loaded = load_audio_settings(&store).unwrap().unwrap();
        assert_eq!(loaded.music_shuffle, Some(true));
        assert_eq!(loaded.menu_track.as_deref(), Some("auto"));
        assert_eq!(loaded.device_name, None);
        assert_eq!(loaded.output_format, DEFAULT_AUDIO_OUTPUT_FORMAT);
        assert_eq!(loaded.effects_volume, 0.8);
        assert_eq!(loaded.music_volume, 0.6);
        let text = store.load_text("audio.json").unwrap().unwrap();
        assert!(text.ends_with('\n'));
        assert!(text.starts_with("{\"version\":1"));
    }

    #[test]
    fn save_reuses_stored_music_when_omitted() {
        let store = store("reuse");
        save_audio_settings(&store, &request()).unwrap();
        let mut next = request();
        next.music_preferences = None;
        next.effects_volume = 0.1;
        save_audio_settings(&store, &next).unwrap();
        let loaded = load_audio_settings(&store).unwrap().unwrap();
        assert_eq!(loaded.music_shuffle, Some(true));
        assert_eq!(loaded.menu_track.as_deref(), Some("auto"));
        assert_eq!(loaded.effects_volume, 0.1);
    }

    #[test]
    fn save_without_music_or_file_uses_defaults() {
        let store = store("defaults");
        let mut next = request();
        next.music_preferences = None;
        save_audio_settings(&store, &next).unwrap();
        let loaded = load_audio_settings(&store).unwrap().unwrap();
        assert_eq!(loaded.music_shuffle, Some(false));
        assert_eq!(loaded.menu_track.as_deref(), Some("auto"));
    }

    #[test]
    fn invalid_documents_fail() {
        let store = store("invalid");
        for (name, document) in [
            (
                "version",
                r#"{"version":2,"deviceName":null,"effectsVolume":0.5,"musicVolume":0.5}"#,
            ),
            (
                "device",
                r#"{"version":1,"deviceName":"","effectsVolume":0.5,"musicVolume":0.5}"#,
            ),
            (
                "volume",
                r#"{"version":1,"deviceName":null,"effectsVolume":2,"musicVolume":0.5}"#,
            ),
            (
                "shuffle",
                r#"{"version":1,"deviceName":null,"effectsVolume":0.5,"musicVolume":0.5,"musicShuffle":"yes"}"#,
            ),
            (
                "track",
                r#"{"version":1,"deviceName":null,"effectsVolume":0.5,"musicVolume":0.5,"menuTrack":"  spaced  "}"#,
            ),
            (
                "format",
                r#"{"version":1,"deviceName":null,"effectsVolume":0.5,"musicVolume":0.5,"outputFormat":{"sampleRate":44100,"channels":3,"sampleBits":16}}"#,
            ),
        ] {
            store.dump("audio.json", document).unwrap();
            let error = load_audio_settings(&store).unwrap_err();
            assert!(
                matches!(
                    error,
                    AudioSettingsError::BadAudio | AudioSettingsError::BadMusic | AudioSettingsError::Audio(_)
                ),
                "{name}: {error}"
            );
        }
        store.dump("audio.json", "not json").unwrap();
        assert!(matches!(
            load_audio_settings(&store).unwrap_err(),
            AudioSettingsError::Settings(_)
        ));
    }

    #[test]
    fn save_validates_volumes_and_device() {
        let store = store("save-invalid");
        let mut bad = request();
        bad.effects_volume = f64::NAN;
        assert!(matches!(
            save_audio_settings(&store, &bad).unwrap_err(),
            AudioSettingsError::BadAudio
        ));
        let mut bad = request();
        bad.selected_output = Some(String::new());
        assert!(matches!(
            save_audio_settings(&store, &bad).unwrap_err(),
            AudioSettingsError::BadAudio
        ));
    }
}
