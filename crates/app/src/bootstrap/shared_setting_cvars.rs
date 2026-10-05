//! Shared client settings cvars.
//!
//! Port of `/home/buzzkill/Projects/quake-typescript/src/app/bootstrap/shared-setting-cvars.ts`
//! (`validateFieldOfView`, `registerRunCvar`, `bindRunCvar`, `registerSharedClientSettings`,
//! `applyAudioOutputSettings`). The merged [`CvarRegistry`](qa_core::cvar::CvarRegistry) has no
//! binding, documentation, or alias surface, so validators, documentation, and the gamma
//! conversion are ported as explicit module items with the donor's exact texts. The
//! `InputCommandBuilder` live always-run binding is absorbed as a one-shot sync because the
//! ported builder exposes tuning only through getters and setters.

use qa_client::audio::error::AudioError;
use qa_client::audio::output::AudioOutputFormat;
use qa_client::input::default_tuning;
use qa_core::cmd::Dialect;
use qa_core::cvar::{cvar_value_text, flags, quake_atof, CvarError, CvarRegistry};
use qa_core::numeric::native_atof;
use qa_world::client::ClientFamily;

use super::render_settings::register_render_settings;
use crate::bootstrap::audio::output_settings::{read_audio_output_cvars, register_audio_output_cvars};
use crate::bootstrap::audio::playlist_settings::register_music_settings;

/// `cl_run` documentation summary.
pub const CL_RUN_DOC_SUMMARY: &str =
    "Always run for this player; the speed key reverses run and walk. Uses the Always run menu preference.";
/// `cl_run` usage line.
pub const CL_RUN_DOC_USAGE: &str = "cl_run [0|1]";
/// `cl_run` examples.
pub const CL_RUN_DOC_EXAMPLES: [&str; 2] = ["cl_run 1", "set cl_run 0"];
/// `cl_run` allowed values.
pub const CL_RUN_ALLOWED_VALUES: [&str; 2] = ["0", "1"];

/// `s_geometryAcoustics` documentation summary.
pub const S_GEOMETRY_ACOUSTICS_DOC_SUMMARY: &str = "Enable shared geometry sound obstruction. Off preserves native attenuation; this is a functional A3D replacement, not native numerical emulation.";
/// `s_geometryAcoustics` usage line.
pub const S_GEOMETRY_ACOUSTICS_DOC_USAGE: &str = "s_geometryAcoustics <0|1>";
/// `s_geometryAcoustics` examples.
pub const S_GEOMETRY_ACOUSTICS_DOC_EXAMPLES: [&str; 2] = ["s_geometryAcoustics 1", "s_geometryAcoustics 0"];
/// `s_geometryAcoustics` allowed values.
pub const S_GEOMETRY_ACOUSTICS_ALLOWED_VALUES: [&str; 2] = ["0", "1"];

/// `r_saveFontData` documentation summary.
pub const R_SAVE_FONT_DATA_DOC_SUMMARY: &str =
    "Export generated Q3 font atlases and DAT records to this content's user directory.";
/// `r_saveFontData` usage line.
pub const R_SAVE_FONT_DATA_DOC_USAGE: &str = "r_saveFontData <0|1>";
/// `r_saveFontData` examples.
pub const R_SAVE_FONT_DATA_DOC_EXAMPLES: [&str; 1] = ["r_saveFontData 1"];

/// `volume` documentation summary.
pub const VOLUME_DOC_SUMMARY: &str = "Effects volume shared with the audio menu. Output clamps to 0 through 1.";
/// `volume` usage line.
pub const VOLUME_DOC_USAGE: &str = "volume <0..1>";
/// `volume` examples.
pub const VOLUME_DOC_EXAMPLES: [&str; 2] = ["volume 0.7", "volume 0"];

/// `bgmvolume` documentation summary.
pub const BGMVOLUME_DOC_SUMMARY: &str = "Music volume shared with the audio menu. Output clamps to 0 through 1.";
/// `bgmvolume` usage line.
pub const BGMVOLUME_DOC_USAGE: &str = "bgmvolume <0..1>";
/// `bgmvolume` examples.
pub const BGMVOLUME_DOC_EXAMPLES: [&str; 2] = ["bgmvolume 0.5", "bgmvolume 0"];

/// `gamma` alias documentation (target `r_gamma`, `gamma = 1 / r_gamma`).
pub const GAMMA_ALIAS_SUMMARY: &str =
    "Quake brightness convention: lower values brighten. Alias of r_gamma with gamma = 1 / r_gamma.";
/// `gamma` alias usage line.
pub const GAMMA_ALIAS_USAGE: &str = "gamma <1/3..2>";
/// `gamma` alias examples.
pub const GAMMA_ALIAS_EXAMPLES: [&str; 1] = ["gamma 0.5"];

/// `vid_gamma` alias documentation (target `r_gamma`, `vid_gamma = 1 / r_gamma`).
pub const VID_GAMMA_ALIAS_SUMMARY: &str =
    "Quake II brightness convention: lower values brighten. Alias of r_gamma with vid_gamma = 1 / r_gamma.";
/// `vid_gamma` alias usage line.
pub const VID_GAMMA_ALIAS_USAGE: &str = "vid_gamma <1/3..2>";
/// `vid_gamma` alias examples.
pub const VID_GAMMA_ALIAS_EXAMPLES: [&str; 1] = ["vid_gamma 0.5"];

/// `s_volume` alias documentation (identity alias of `volume`).
pub const S_VOLUME_ALIAS_SUMMARY: &str = "Alias of volume. Changes the same effects gain and audio menu preference.";
/// `s_volume` alias usage line.
pub const S_VOLUME_ALIAS_USAGE: &str = "s_volume <0..1>";
/// `s_volume` alias examples.
pub const S_VOLUME_ALIAS_EXAMPLES: [&str; 1] = ["s_volume 0.7"];

/// `s_musicvolume` alias documentation (identity alias of `bgmvolume`).
pub const S_MUSICVOLUME_ALIAS_SUMMARY: &str =
    "Alias of bgmvolume. Changes the same music gain and audio menu preference.";
/// `s_musicvolume` alias usage line.
pub const S_MUSICVOLUME_ALIAS_USAGE: &str = "s_musicvolume <0..1>";
/// `s_musicvolume` alias examples.
pub const S_MUSICVOLUME_ALIAS_EXAMPLES: [&str; 1] = ["s_musicvolume 0.5"];

/// `ogg_volume` alias documentation (identity alias of `bgmvolume`).
pub const OGG_VOLUME_ALIAS_SUMMARY: &str = "Quake II music volume. Alias of bgmvolume and the shared music gain.";
/// `ogg_volume` alias usage line.
pub const OGG_VOLUME_ALIAS_USAGE: &str = "ogg_volume <0..1>";
/// `ogg_volume` alias examples.
pub const OGG_VOLUME_ALIAS_EXAMPLES: [&str; 1] = ["ogg_volume 0.5"];

/// Validate a field-of-view preference (`None` accepts; donor text otherwise).
#[must_use]
pub fn validate_field_of_view(text: &str) -> Option<String> {
    let value = text.parse::<f64>().unwrap_or(f64::NAN);
    if !matches_fov_number(text) || !value.is_finite() || value < 60.0 || value > 160.0 {
        Some("Field of view must be between 60 and 160 degrees".to_string())
    } else {
        None
    }
}

/// Donor `/^[+-]?(?:\d+(?:\.\d*)?|\.\d+)$/` shape check.
fn matches_fov_number(text: &str) -> bool {
    let rest = text
        .strip_prefix('+')
        .or_else(|| text.strip_prefix('-'))
        .unwrap_or(text);
    if rest.is_empty() {
        return false;
    }
    match rest.find('.') {
        None => rest.chars().all(|char| char.is_ascii_digit()),
        Some(dot) => {
            let (head, tail) = rest.split_at(dot);
            let tail = &tail[1..];
            if tail.contains('.') || !tail.chars().all(|char| char.is_ascii_digit()) {
                return false;
            }
            if head.is_empty() {
                !tail.is_empty()
            } else {
                head.chars().all(|char| char.is_ascii_digit())
            }
        }
    }
}

/// Default always-run tuning for a movement dialect (donor `defaultViewInputTuning`).
#[must_use]
pub const fn default_always_run(movement_dialect: Dialect) -> bool {
    default_tuning(match movement_dialect {
        Dialect::Q1Netquake => ClientFamily::Q1Netquake,
        Dialect::Q1Quakeworld => ClientFamily::Q1Quakeworld,
        Dialect::Q2Classic => ClientFamily::Q2Classic,
        Dialect::Q2Rerelease => ClientFamily::Q2Rerelease,
        Dialect::Q3 => ClientFamily::Q3,
    })
    .always_run
}

/// Register the shared `cl_run` cvar.
pub fn register_run_cvar(cvars: &mut CvarRegistry, movement_dialect: Dialect) -> Result<(), CvarError> {
    let default_value = if default_always_run(movement_dialect) { "1" } else { "0" };
    let stored = cvars.get("cl_run").is_some();
    if !stored || !cvars.dialect().is_q1() || cvars.is_console_created("cl_run") {
        cvars.register("cl_run", default_value, flags::ARCHIVE)?;
    } else {
        cvars.add_flags("cl_run", flags::ARCHIVE)?;
    }
    Ok(())
}

/// Validate a `cl_run` value (`None` accepts; donor text otherwise).
#[must_use]
pub fn validate_run_cvar(text: &str) -> Option<String> {
    if text == "0" || text == "1" {
        None
    } else {
        Some("Always run must be 0 or 1".to_string())
    }
}

/// Register `cl_run` and sync it one-shot with the caller's tuning.
///
/// Returns the effective always-run state (`cl_run != 0`) so the caller can store it back into
/// its tuning; the donor's live builder binding has no ported builder surface.
pub fn bind_run_cvar(
    cvars: &mut CvarRegistry,
    movement_dialect: Dialect,
    tuning_always_run: bool,
) -> Result<bool, CvarError> {
    let stored = cvars.get("cl_run").is_some();
    register_run_cvar(cvars, movement_dialect)?;
    if !stored {
        cvars.set("cl_run", if tuning_always_run { "1" } else { "0" }, false)?;
    }
    if validate_run_cvar(&cvars.variable_string("cl_run")).is_some() {
        cvars.set("cl_run", if tuning_always_run { "1" } else { "0" }, true)?;
    }
    Ok(cvars.variable_value("cl_run") != 0.0)
}

/// Parse cvar text with the dialect's `atof`: Quake `Q_atof` for Q1,
/// `strtod` for the others.
#[must_use]
pub fn dialect_number(text: &str, dialect: Dialect) -> f64 {
    if dialect.is_q1() {
        quake_atof(text)
    } else {
        native_atof(text)
    }
}

/// Format a number the way `Cvar_SetValue` does: `%f`, with the `%i`
/// shortcut for integral values outside Q1. Non-finite values render the
/// way C `%f` renders them.
#[must_use]
pub fn number_text(value: f64, dialect: Dialect) -> String {
    cvar_value_text(value, !dialect.is_q1()).unwrap_or_else(|_| {
        if value.is_nan() {
            "nan".to_string()
        } else if value < 0.0 {
            "-inf".to_string()
        } else {
            "inf".to_string()
        }
    })
}

/// Whether text starts a C number after ASCII whitespace: what the
/// dialect `atof` consumes, so `"1x"` parses (value 1) and `"abc"` does not.
fn starts_number(text: &str) -> bool {
    let trimmed = text.trim_start_matches(|char: char| char.is_ascii_whitespace());
    let rest = trimmed
        .strip_prefix('+')
        .or_else(|| trimmed.strip_prefix('-'))
        .unwrap_or(trimmed);
    let mut chars = rest.chars();
    match chars.next() {
        Some('0'..='9') => true,
        Some('.') => matches!(chars.next(), Some('0'..='9')),
        _ => false,
    }
}

/// Validate a finite-number cvar value (`None` accepts; donor text otherwise).
#[must_use]
pub fn validate_finite_number(text: &str, dialect: Dialect) -> Option<String> {
    if starts_number(text) && dialect_number(text, dialect).is_finite() {
        None
    } else {
        Some("Expected a finite number".to_string())
    }
}

/// Validate an `r_gamma` value (`None` accepts; donor text otherwise).
#[must_use]
pub fn validate_r_gamma(text: &str, dialect: Dialect) -> Option<String> {
    if validate_finite_number(text, dialect).is_some() {
        return validate_finite_number(text, dialect);
    }
    let value = dialect_number(text, dialect);
    if (0.5..=3.0).contains(&value) {
        None
    } else {
        Some("Brightness must be between 0.5 and 3".to_string())
    }
}

/// Validate an `s_geometryAcoustics` value (`None` accepts; donor text otherwise).
#[must_use]
pub fn validate_geometry_acoustics(text: &str) -> Option<String> {
    if text == "0" || text == "1" {
        None
    } else {
        Some("Use 0 or 1".to_string())
    }
}

/// Gamma alias read conversion (`gamma = 1 / r_gamma`), formatted the
/// way `Cvar_SetValue` formats.
#[must_use]
pub fn gamma_alias_read(value: &str, dialect: Dialect) -> String {
    number_text(1.0 / dialect_number(value, dialect), dialect)
}

/// Gamma alias write conversion (donor range `1/3..=2`, exact error text).
pub fn gamma_alias_write(value: &str, dialect: Dialect) -> Result<String, &'static str> {
    if validate_finite_number(value, dialect).is_some() {
        return Err("Gamma must be between 1/3 and 2");
    }
    let number = dialect_number(value, dialect);
    if number < 1.0 / 3.0 || number > 2.0 {
        return Err("Gamma must be between 1/3 and 2");
    }
    Ok(number_text(1.0 / number, dialect))
}

/// Register every shared client setting (aliases exist before source configuration executes).
pub fn register_shared_client_settings(
    cvars: &mut CvarRegistry,
    output_format: AudioOutputFormat,
) -> Result<(), CvarError> {
    register_render_settings(cvars)?;
    qa_client::input::device::register_input_device_cvars(cvars)?;
    register_audio_output_cvars(cvars, output_format)?;
    register_music_settings(cvars)?;
    cvars.register("s_geometryAcoustics", "0", flags::ARCHIVE)?;
    cvars.register("r_saveFontData", "0", flags::NONE)?;
    let q3 = cvars.dialect() == Dialect::Q3;
    cvars.register("volume", if q3 { "0.8" } else { "0.7" }, flags::ARCHIVE)?;
    cvars.register("bgmvolume", if q3 { "0.25" } else { "1" }, flags::ARCHIVE)?;
    Ok(())
}

/// Audio owner backing [`apply_audio_output_settings`].
pub trait AudioOutputSink {
    /// Current output format.
    fn output_format(&self) -> AudioOutputFormat;
    /// Select a new output format.
    fn select_output_format(&mut self, format: AudioOutputFormat);
}

/// Select the cvar output format when it differs from the audio owner's format.
pub fn apply_audio_output_settings<Sink: AudioOutputSink + ?Sized>(
    cvars: &CvarRegistry,
    audio: &mut Sink,
) -> Result<(), AudioError> {
    let desired = read_audio_output_cvars(cvars)?;
    let current = audio.output_format();
    if desired.sample_rate != current.sample_rate
        || desired.sample_bits != current.sample_bits
        || desired.channels != current.channels
    {
        audio.select_output_format(desired);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use qa_client::audio::output::DEFAULT_AUDIO_OUTPUT_FORMAT;

    use super::*;

    #[test]
    fn field_of_view_validation_matches_donor() {
        for text in ["60", "90", "160", "90.5", "+120", "60.", ".75"] {
            if text == ".75" {
                assert_eq!(
                    validate_field_of_view(text),
                    Some("Field of view must be between 60 and 160 degrees".to_string()),
                    "{text}"
                );
            } else {
                assert_eq!(validate_field_of_view(text), None, "{text}");
            }
        }
        for text in ["", "59", "161", "abc", "90deg", "1e2", " 90", "90 ", "--90", ".", "+"] {
            assert_eq!(
                validate_field_of_view(text),
                Some("Field of view must be between 60 and 160 degrees".to_string()),
                "{text}"
            );
        }
    }

    #[test]
    fn run_cvar_defaults_follow_movement_dialect() {
        let mut cvars = CvarRegistry::new(Dialect::Q1Netquake);
        register_run_cvar(&mut cvars, Dialect::Q1Netquake).unwrap();
        assert_eq!(cvars.variable_string("cl_run"), "0");
        let mut cvars = CvarRegistry::new(Dialect::Q3);
        register_run_cvar(&mut cvars, Dialect::Q3).unwrap();
        assert_eq!(cvars.variable_string("cl_run"), "1");
        assert_eq!(validate_run_cvar("2"), Some("Always run must be 0 or 1".to_string()));
        assert_eq!(validate_run_cvar("1"), None);
    }

    #[test]
    fn bind_run_cvar_coerces_invalid_stored_value() {
        let mut cvars = CvarRegistry::new(Dialect::Q2Classic);
        cvars.register("cl_run", "7", flags::NONE).unwrap();
        let effective = bind_run_cvar(&mut cvars, Dialect::Q2Classic, true).unwrap();
        assert!(effective);
        assert_eq!(cvars.variable_string("cl_run"), "1");
    }

    #[test]
    fn shared_settings_install_dialect_volumes() {
        let mut cvars = CvarRegistry::new(Dialect::Q3);
        register_shared_client_settings(&mut cvars, DEFAULT_AUDIO_OUTPUT_FORMAT).unwrap();
        assert_eq!(cvars.variable_string("volume"), "0.8");
        assert_eq!(cvars.variable_string("bgmvolume"), "0.25");
        assert_eq!(cvars.variable_string("r_shadows"), "0");
        assert_eq!(cvars.variable_string("s_geometryAcoustics"), "0");
        assert_eq!(cvars.variable_string("r_saveFontData"), "0");
        let mut cvars = CvarRegistry::new(Dialect::Q1Netquake);
        register_shared_client_settings(&mut cvars, DEFAULT_AUDIO_OUTPUT_FORMAT).unwrap();
        assert_eq!(cvars.variable_string("volume"), "0.7");
        assert_eq!(cvars.variable_string("bgmvolume"), "1");
    }

    #[test]
    fn gamma_alias_conversion_uses_cvar_set_value_text() {
        for dialect in [Dialect::Q2Classic, Dialect::Q2Rerelease, Dialect::Q3] {
            assert_eq!(gamma_alias_read("0.5", dialect), "2");
            assert_eq!(gamma_alias_write("0.5", dialect).unwrap(), "2");
            assert_eq!(gamma_alias_read("0.8", dialect), "1.250000");
            assert_eq!(
                gamma_alias_write("abc", dialect).unwrap_err(),
                "Gamma must be between 1/3 and 2"
            );
            assert_eq!(
                gamma_alias_write("3", dialect).unwrap_err(),
                "Gamma must be between 1/3 and 2"
            );
            assert_eq!(
                validate_r_gamma("0.4", dialect),
                Some("Brightness must be between 0.5 and 3".to_string())
            );
            assert_eq!(validate_r_gamma("1", dialect), None);
        }
        for dialect in [Dialect::Q1Netquake, Dialect::Q1Quakeworld] {
            assert_eq!(gamma_alias_read("0.5", dialect), "2.000000");
            assert_eq!(gamma_alias_write("0.5", dialect).unwrap(), "2.000000");
            assert_eq!(validate_finite_number("1x", dialect), None);
            assert_eq!(
                validate_finite_number("abc", dialect),
                Some("Expected a finite number".to_string())
            );
        }
        assert_eq!(validate_geometry_acoustics("2"), Some("Use 0 or 1".to_string()));
    }

    #[test]
    fn apply_output_settings_selects_on_difference() {
        struct Sink {
            format: AudioOutputFormat,
            selected: Option<AudioOutputFormat>,
        }
        impl AudioOutputSink for Sink {
            fn output_format(&self) -> AudioOutputFormat {
                self.format
            }
            fn select_output_format(&mut self, format: AudioOutputFormat) {
                self.selected = Some(format);
            }
        }
        let mut cvars = CvarRegistry::new(Dialect::Q3);
        register_shared_client_settings(&mut cvars, DEFAULT_AUDIO_OUTPUT_FORMAT).unwrap();
        let mut sink = Sink {
            format: DEFAULT_AUDIO_OUTPUT_FORMAT,
            selected: None,
        };
        apply_audio_output_settings(&cvars, &mut sink).unwrap();
        assert!(sink.selected.is_none());
        cvars.set("s_outputRate", "22050", true).unwrap();
        apply_audio_output_settings(&cvars, &mut sink).unwrap();
        assert_eq!(sink.selected.unwrap().sample_rate, 22050);
    }
}
