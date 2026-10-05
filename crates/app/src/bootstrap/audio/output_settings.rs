//! Shared output-format cvars.
//!
//! Port of donor `src/app/bootstrap/audio/output-settings.ts`
//! (`registerAudioOutputCvars`, `readAudioOutputCvars`,
//! `writeAudioOutputCvars`, `audioOutputCvarNames`, the `s_khz` alias).
//! The merged [`CvarRegistry`](qa_core::cvar::CvarRegistry) has no binding,
//! documentation, or alias surface, so validators, documentation, and the
//! `s_khz` conversion are ported as explicit module items with the donor's
//! exact texts; registration installs the three variables.

use qa_client::audio::error::AudioError;
use qa_client::audio::output::{audio_khz_rate, audio_output_format, AudioOutputFormat};
use qa_core::cvar::{flags, CvarError, CvarRegistry};

/// Output-format cvar names in donor field order.
pub const AUDIO_OUTPUT_CVAR_NAMES: [&str; 3] = ["s_outputRate", "s_outputBits", "s_outputChannels"];

/// Shared output-format documentation summary.
pub const AUDIO_OUTPUT_DOC_SUMMARY: &str = "Shared output format; apply with snd_restart or the audio menu.";

/// `s_khz` documentation.
pub const S_KHZ_DOC_SUMMARY: &str = "Source sample-rate convention for the shared output; apply with snd_restart.";
/// `s_khz` usage line.
pub const S_KHZ_DOC_USAGE: &str = "s_khz <11|22|44|48>";
/// `s_khz` examples.
pub const S_KHZ_DOC_EXAMPLES: [&str; 1] = ["s_khz 44"];

/// One validated output-format field.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioOutputField {
    /// `s_outputRate` / `sampleRate`.
    SampleRate,
    /// `s_outputBits` / `sampleBits`.
    SampleBits,
    /// `s_outputChannels` / `channels`.
    Channels,
}

/// JavaScript `String.trim` character set (WhiteSpace plus LineTerminator).
pub(crate) fn is_js_trim(char: char) -> bool {
    matches!(
        char,
        '\u{9}'
            | '\u{a}'
            | '\u{b}'
            | '\u{c}'
            | '\u{d}'
            | '\u{20}'
            | '\u{a0}'
            | '\u{1680}'
            | '\u{2000}'
            | '\u{2001}'
            | '\u{2002}'
            | '\u{2003}'
            | '\u{2004}'
            | '\u{2005}'
            | '\u{2006}'
            | '\u{2007}'
            | '\u{2008}'
            | '\u{2009}'
            | '\u{200a}'
            | '\u{2028}'
            | '\u{2029}'
            | '\u{202f}'
            | '\u{205f}'
            | '\u{3000}'
            | '\u{feff}'
    )
}

/// JavaScript `String(number)` conversion.
pub(crate) fn js_number_string(value: f64) -> String {
    if value.is_nan() {
        return "NaN".to_string();
    }
    if value == f64::INFINITY {
        return "Infinity".to_string();
    }
    if value == f64::NEG_INFINITY {
        return "-Infinity".to_string();
    }
    if value == 0.0 {
        return "0".to_string();
    }
    let sign = if value < 0.0 { "-" } else { "" };
    let plain = format!("{}", value.abs());
    let (int_part, frac_part) = match plain.split_once('.') {
        Some((int, frac)) => (int, frac),
        None => (plain.as_str(), ""),
    };
    let digits = format!("{int_part}{frac_part}");
    let leading = digits.len() - digits.trim_start_matches('0').len();
    let significant = digits.trim_start_matches('0').trim_end_matches('0');
    let mut significant = significant.to_string();
    if significant.is_empty() {
        significant.push('0');
    }
    let k = significant.len() as i32;
    let n = int_part.len() as i32 - leading as i32;
    if k <= n && n <= 21 {
        return format!("{sign}{significant}{}", "0".repeat((n - k) as usize));
    }
    if 0 < n && n <= 21 {
        let split = n as usize;
        return format!("{sign}{}.{}", &significant[..split], &significant[split..]);
    }
    if -6 < n && n <= 0 {
        return format!("{sign}0.{}{significant}", "0".repeat(-n as usize));
    }
    let exponent = n - 1;
    let mantissa = if k > 1 {
        format!("{}.{}", &significant[..1], &significant[1..])
    } else {
        significant[..1].to_string()
    };
    let exp_sign = if exponent < 0 { "-" } else { "+" };
    format!("{sign}{mantissa}e{exp_sign}{}", exponent.abs())
}

/// Validate one output-format cvar value (`None` accepts).
#[must_use]
#[allow(clippy::cast_possible_truncation)]
pub fn validate_audio_output_value(defaults: AudioOutputFormat, field: AudioOutputField, text: &str) -> Option<String> {
    let trimmed = text.trim_matches(|char: char| char.is_ascii_whitespace());
    let digits = trimmed
        .strip_prefix('+')
        .or_else(|| trimmed.strip_prefix('-'))
        .unwrap_or(trimmed);
    if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return Some("Expected an integer audio format value".to_string());
    }
    let number = match trimmed.parse::<f64>() {
        Ok(number) if number.is_finite() => number,
        _ => return Some(AudioError::BadOutputFormat.to_string()),
    };
    let candidate = match field {
        AudioOutputField::SampleRate => {
            if !number.is_finite() || number.fract() != 0.0 || number.abs() > 9_007_199_254_740_991.0 {
                return Some(AudioError::BadOutputFormat.to_string());
            }
            audio_output_format(
                number as i64,
                i64::from(defaults.channels),
                i64::from(defaults.sample_bits),
            )
        }
        AudioOutputField::SampleBits => {
            if number != 8.0 && number != 16.0 {
                return Some(AudioError::BadOutputFormat.to_string());
            }
            audio_output_format(
                i64::from(defaults.sample_rate),
                i64::from(defaults.channels),
                number as i64,
            )
        }
        AudioOutputField::Channels => {
            if number != 1.0 && number != 2.0 {
                return Some(AudioError::BadOutputFormat.to_string());
            }
            audio_output_format(
                i64::from(defaults.sample_rate),
                number as i64,
                i64::from(defaults.sample_bits),
            )
        }
    };
    candidate.err().map(|error| error.to_string())
}

/// Register the shared output-format cvars.
pub fn register_audio_output_cvars(cvars: &mut CvarRegistry, defaults: AudioOutputFormat) -> Result<(), CvarError> {
    for (name, field) in [
        ("s_outputRate", AudioOutputField::SampleRate),
        ("s_outputBits", AudioOutputField::SampleBits),
        ("s_outputChannels", AudioOutputField::Channels),
    ] {
        let value = match field {
            AudioOutputField::SampleRate => i64::from(defaults.sample_rate),
            AudioOutputField::SampleBits => i64::from(defaults.sample_bits),
            AudioOutputField::Channels => i64::from(defaults.channels),
        };
        cvars.register(name, &value.to_string(), flags::ARCHIVE)?;
    }
    Ok(())
}

/// Read the shared output format, rejecting non-integral or out-of-range values.
pub fn read_audio_output_cvars(cvars: &CvarRegistry) -> Result<AudioOutputFormat, AudioError> {
    let mut values = [0i64; 3];
    for (index, name) in AUDIO_OUTPUT_CVAR_NAMES.iter().enumerate() {
        let value = cvars.variable_value(name);
        if !value.is_finite() || value.fract() != 0.0 || value.abs() > 9_007_199_254_740_991.0 {
            return audio_output_format(0, 1, 8);
        }
        #[allow(clippy::cast_possible_truncation)]
        {
            values[index] = value as i64;
        }
    }
    audio_output_format(values[0], values[2], values[1])
}

/// Write the shared output format.
pub fn write_audio_output_cvars(cvars: &mut CvarRegistry, format: AudioOutputFormat) -> Result<(), CvarError> {
    for (name, value) in [
        ("s_outputRate", i64::from(format.sample_rate)),
        ("s_outputBits", i64::from(format.sample_bits)),
        ("s_outputChannels", i64::from(format.channels)),
    ] {
        cvars.set(name, &value.to_string(), false)?;
    }
    Ok(())
}

/// Read the `s_khz` alias for an `s_outputRate` value: integer kHz.
#[must_use]
pub fn s_khz_read(value: &str) -> String {
    (qa_core::numeric::native_atoi(value) / 1000).to_string()
}

/// Write the `s_khz` alias to an `s_outputRate` value.
pub fn s_khz_write(value: &str) -> Result<String, &'static str> {
    audio_khz_rate(value).map_or(Err("Use s_khz 11, 22, 44 or 48"), |rate| Ok(rate.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_client::audio::output::DEFAULT_AUDIO_OUTPUT_FORMAT;
    use qa_core::cmd::Dialect;

    #[test]
    fn registers_defaults() {
        let mut cvars = CvarRegistry::new(Dialect::Q2Classic);
        register_audio_output_cvars(&mut cvars, DEFAULT_AUDIO_OUTPUT_FORMAT).unwrap();
        assert_eq!(cvars.get("s_outputRate").unwrap().value, "44100");
        assert_eq!(cvars.get("s_outputBits").unwrap().value, "16");
        assert_eq!(cvars.get("s_outputChannels").unwrap().value, "2");
        assert_eq!(
            AUDIO_OUTPUT_CVAR_NAMES,
            ["s_outputRate", "s_outputBits", "s_outputChannels"]
        );
    }

    #[test]
    fn validates_values() {
        let defaults = DEFAULT_AUDIO_OUTPUT_FORMAT;
        assert_eq!(
            validate_audio_output_value(defaults, AudioOutputField::SampleRate, "48000"),
            None
        );
        assert_eq!(
            validate_audio_output_value(defaults, AudioOutputField::SampleRate, "1e4"),
            Some("Expected an integer audio format value".to_string())
        );
        assert_eq!(
            validate_audio_output_value(defaults, AudioOutputField::SampleRate, "0xAC44"),
            Some("Expected an integer audio format value".to_string())
        );
        assert_eq!(
            validate_audio_output_value(defaults, AudioOutputField::SampleRate, ""),
            Some("Expected an integer audio format value".to_string())
        );
        assert_eq!(
            validate_audio_output_value(defaults, AudioOutputField::SampleRate, "12.5"),
            Some("Expected an integer audio format value".to_string())
        );
        assert_eq!(
            validate_audio_output_value(defaults, AudioOutputField::SampleRate, "7000"),
            Some("Audio output requires 8000–192000 Hz, 1 or 2 channels, and 8 or 16 bits".to_string())
        );
        assert_eq!(
            validate_audio_output_value(defaults, AudioOutputField::Channels, "1"),
            None
        );
        assert_eq!(
            validate_audio_output_value(defaults, AudioOutputField::Channels, "3"),
            Some("Audio output requires 8000–192000 Hz, 1 or 2 channels, and 8 or 16 bits".to_string())
        );
        assert_eq!(
            validate_audio_output_value(defaults, AudioOutputField::SampleBits, "8"),
            None
        );
        assert_eq!(
            validate_audio_output_value(defaults, AudioOutputField::SampleBits, "24"),
            Some("Audio output requires 8000–192000 Hz, 1 or 2 channels, and 8 or 16 bits".to_string())
        );
    }

    #[test]
    fn reads_and_writes_format() {
        let mut cvars = CvarRegistry::new(Dialect::Q2Classic);
        register_audio_output_cvars(&mut cvars, DEFAULT_AUDIO_OUTPUT_FORMAT).unwrap();
        assert_eq!(read_audio_output_cvars(&cvars).unwrap(), DEFAULT_AUDIO_OUTPUT_FORMAT);
        write_audio_output_cvars(
            &mut cvars,
            AudioOutputFormat {
                sample_rate: 22050,
                channels: 1,
                sample_bits: 8,
            },
        )
        .unwrap();
        assert_eq!(
            read_audio_output_cvars(&cvars).unwrap(),
            AudioOutputFormat {
                sample_rate: 22050,
                channels: 1,
                sample_bits: 8
            }
        );
        cvars.set("s_outputRate", "12.5", false).unwrap();
        assert!(read_audio_output_cvars(&cvars).is_err());
    }

    #[test]
    fn converts_s_khz() {
        assert_eq!(s_khz_read("11025"), "11");
        assert_eq!(s_khz_read("22050"), "22");
        assert_eq!(s_khz_read("44100"), "44");
        assert_eq!(s_khz_read("48000"), "48");
        assert_eq!(s_khz_read("12000"), "12");
        assert_eq!(s_khz_read("abc"), "0");
        assert_eq!(s_khz_write("44").unwrap(), "44100");
        assert_eq!(s_khz_write("11").unwrap(), "11025");
        assert_eq!(s_khz_write("12"), Err("Use s_khz 11, 22, 44 or 48"));
    }

    #[test]
    fn formats_js_numbers() {
        assert_eq!(js_number_string(44.1), "44.1");
        assert_eq!(js_number_string(12.0), "12");
        assert_eq!(js_number_string(0.000001), "0.000001");
        assert_eq!(js_number_string(0.0000001), "1e-7");
        assert_eq!(js_number_string(1e21), "1e+21");
        assert_eq!(js_number_string(-0.0), "0");
    }
}
