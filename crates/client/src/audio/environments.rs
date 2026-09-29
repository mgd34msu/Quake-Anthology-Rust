//! Reverb environment selection from authored JSON.
//!
//! Donor provenance: `src/audio/environments.ts` (`parseEnvironments`,
//! `reverbPreset`, `EnvironmentReverb`, from `al.c`).

use qa_core::math::Vec3;

use super::error::AudioError;
use super::reverb_presets::{EfxReverbParams, REVERB_PRESET_NAMES, REVERB_PRESET_PLAIN, REVERB_PRESETS};

/// One material-to-preset rule.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReverbMaterial {
    /// Materials (lowercased at match time); `None` matches everything.
    pub materials: Option<Vec<String>>,
    /// Preset index.
    pub preset_index: usize,
}

/// One sized environment.
#[derive(Debug, Clone, PartialEq)]
pub struct ReverbEnvironment {
    /// Room size threshold.
    pub dimension: f64,
    /// Material rules.
    pub reverbs: Vec<ReverbMaterial>,
}

/// Trace hit.
#[derive(Debug, Clone, PartialEq)]
pub struct AudioTrace {
    /// Hit fraction.
    pub fraction: f64,
    /// Hit end.
    pub end: [f64; 3],
    /// Hit material.
    pub material: Option<String>,
    /// Hit sky.
    pub sky: bool,
}

/// World trace query.
pub type AudioTraceQuery = Box<dyn FnMut([f64; 3], [f64; 3], [f64; 3], [f64; 3]) -> AudioTrace>;

#[derive(Debug, Clone, PartialEq)]
enum Json {
    Null,
    Bool(bool),
    Num(f64),
    Str(String),
    Arr(Vec<Json>),
    Obj(Vec<(String, Json)>),
}

impl Json {
    fn get(&self, key: &str) -> Option<&Json> {
        match self {
            Json::Obj(entries) => entries.iter().find(|(name, _)| name == key).map(|(_, value)| value),
            _ => None,
        }
    }
}

struct JsonParser<'a> {
    bytes: &'a [u8],
    cursor: usize,
}

impl JsonParser<'_> {
    fn error(&self, message: &str) -> AudioError {
        AudioError::EnvironmentsJson(format!("{message} at byte {}", self.cursor))
    }

    fn whitespace(&mut self) {
        while self.cursor < self.bytes.len() && matches!(self.bytes[self.cursor], b' ' | b'\t' | b'\n' | b'\r') {
            self.cursor += 1;
        }
    }

    fn literal(&mut self, word: &str, value: Json) -> Result<Json, AudioError> {
        if self.bytes[self.cursor..].starts_with(word.as_bytes()) {
            self.cursor += word.len();
            Ok(value)
        } else {
            Err(self.error("invalid literal"))
        }
    }

    fn string(&mut self) -> Result<String, AudioError> {
        if self.bytes.get(self.cursor) != Some(&b'"') {
            return Err(self.error("expected string"));
        }
        self.cursor += 1;
        let mut text = String::new();
        loop {
            let Some(&byte) = self.bytes.get(self.cursor) else {
                return Err(self.error("unterminated string"));
            };
            self.cursor += 1;
            match byte {
                b'"' => return Ok(text),
                b'\\' => {
                    let Some(&escape) = self.bytes.get(self.cursor) else {
                        return Err(self.error("unterminated escape"));
                    };
                    self.cursor += 1;
                    match escape {
                        b'"' => text.push('"'),
                        b'\\' => text.push('\\'),
                        b'/' => text.push('/'),
                        b'b' => text.push('\u{8}'),
                        b'f' => text.push('\u{c}'),
                        b'n' => text.push('\n'),
                        b'r' => text.push('\r'),
                        b't' => text.push('\t'),
                        b'u' => {
                            let code = self.hex4()?;
                            let scalar = if (0xd800..0xdc00).contains(&code) && self.bytes.get(self.cursor) == Some(&b'\\') && self.bytes.get(self.cursor + 1) == Some(&b'u') {
                                self.cursor += 2;
                                let low = self.hex4()?;
                                if !(0xdc00..0xe000).contains(&low) {
                                    return Err(self.error("invalid surrogate"));
                                }
                                0x10000 + ((code - 0xd800) << 10) + (low - 0xdc00)
                            } else {
                                code
                            };
                            text.push(char::from_u32(scalar).ok_or_else(|| self.error("invalid code point"))?);
                        }
                        _ => return Err(self.error("invalid escape")),
                    }
                }
                _ => {
                    if byte < 0x20 {
                        return Err(self.error("unescaped control"));
                    }
                    // Multibyte UTF-8 continues verbatim.
                    let start = self.cursor - 1;
                    let width = if byte < 0x80 {
                        1
                    } else if byte >= 0xf0 {
                        4
                    } else if byte >= 0xe0 {
                        3
                    } else {
                        2
                    };
                    if start + width > self.bytes.len() {
                        return Err(self.error("truncated UTF-8"));
                    }
                    let slice = std::str::from_utf8(&self.bytes[start..start + width]).map_err(|_| self.error("invalid UTF-8"))?;
                    text.push_str(slice);
                    self.cursor = start + width;
                }
            }
        }
    }

    fn hex4(&mut self) -> Result<u32, AudioError> {
        if self.cursor + 4 > self.bytes.len() {
            return Err(self.error("truncated escape"));
        }
        let digits = std::str::from_utf8(&self.bytes[self.cursor..self.cursor + 4]).map_err(|_| self.error("invalid escape"))?;
        self.cursor += 4;
        u32::from_str_radix(digits, 16).map_err(|_| self.error("invalid escape"))
    }

    fn number(&mut self) -> Result<f64, AudioError> {
        let start = self.cursor;
        if self.bytes.get(self.cursor) == Some(&b'-') {
            self.cursor += 1;
        }
        match self.bytes.get(self.cursor) {
            Some(b'0') => self.cursor += 1,
            Some(b'1'..=b'9') => {
                while matches!(self.bytes.get(self.cursor), Some(b'0'..=b'9')) {
                    self.cursor += 1;
                }
            }
            _ => return Err(self.error("invalid number")),
        }
        if self.bytes.get(self.cursor) == Some(&b'.') {
            self.cursor += 1;
            if !matches!(self.bytes.get(self.cursor), Some(b'0'..=b'9')) {
                return Err(self.error("invalid number"));
            }
            while matches!(self.bytes.get(self.cursor), Some(b'0'..=b'9')) {
                self.cursor += 1;
            }
        }
        if matches!(self.bytes.get(self.cursor), Some(b'e' | b'E')) {
            self.cursor += 1;
            if matches!(self.bytes.get(self.cursor), Some(b'+' | b'-')) {
                self.cursor += 1;
            }
            if !matches!(self.bytes.get(self.cursor), Some(b'0'..=b'9')) {
                return Err(self.error("invalid number"));
            }
            while matches!(self.bytes.get(self.cursor), Some(b'0'..=b'9')) {
                self.cursor += 1;
            }
        }
        let text = std::str::from_utf8(&self.bytes[start..self.cursor]).map_err(|_| self.error("invalid number"))?;
        text.parse::<f64>().map_err(|_| self.error("invalid number"))
    }

    fn value(&mut self) -> Result<Json, AudioError> {
        self.whitespace();
        let Some(&byte) = self.bytes.get(self.cursor) else {
            return Err(self.error("unexpected end"));
        };
        match byte {
            b'{' => {
                self.cursor += 1;
                let mut entries = Vec::new();
                self.whitespace();
                if self.bytes.get(self.cursor) == Some(&b'}') {
                    self.cursor += 1;
                    return Ok(Json::Obj(entries));
                }
                loop {
                    self.whitespace();
                    let key = self.string()?;
                    self.whitespace();
                    if self.bytes.get(self.cursor) != Some(&b':') {
                        return Err(self.error("expected colon"));
                    }
                    self.cursor += 1;
                    entries.push((key, self.value()?));
                    self.whitespace();
                    match self.bytes.get(self.cursor) {
                        Some(b',') => self.cursor += 1,
                        Some(b'}') => {
                            self.cursor += 1;
                            return Ok(Json::Obj(entries));
                        }
                        _ => return Err(self.error("expected comma or brace")),
                    }
                }
            }
            b'[' => {
                self.cursor += 1;
                let mut items = Vec::new();
                self.whitespace();
                if self.bytes.get(self.cursor) == Some(&b']') {
                    self.cursor += 1;
                    return Ok(Json::Arr(items));
                }
                loop {
                    items.push(self.value()?);
                    self.whitespace();
                    match self.bytes.get(self.cursor) {
                        Some(b',') => self.cursor += 1,
                        Some(b']') => {
                            self.cursor += 1;
                            return Ok(Json::Arr(items));
                        }
                        _ => return Err(self.error("expected comma or bracket")),
                    }
                }
            }
            b'"' => Ok(Json::Str(self.string()?)),
            b't' => self.literal("true", Json::Bool(true)),
            b'f' => self.literal("false", Json::Bool(false)),
            b'n' => self.literal("null", Json::Null),
            _ => Ok(Json::Num(self.number()?)),
        }
    }

    fn document(&mut self) -> Result<Json, AudioError> {
        let value = self.value()?;
        self.whitespace();
        if self.cursor != self.bytes.len() {
            return Err(self.error("trailing bytes"));
        }
        Ok(value)
    }
}

/// Parse authored environments.
pub fn parse_environments(text: &str, warn: &mut dyn FnMut(&str)) -> Result<Vec<ReverbEnvironment>, AudioError> {
    let root = JsonParser {
        bytes: text.as_bytes(),
        cursor: 0,
    }
    .document()?;
    let Json::Obj(_) = root else {
        return Err(AudioError::EnvironmentsArray);
    };
    let Some(Json::Arr(environments)) = root.get("environments") else {
        return Err(AudioError::EnvironmentsArray);
    };
    let mut result = Vec::with_capacity(environments.len());
    for value in environments {
        let Json::Obj(_) = value else {
            return Err(AudioError::EnvironmentObject);
        };
        let dimension = match value.get("dimension") {
            None | Some(Json::Null) => 0.0,
            Some(Json::Num(dimension)) if dimension.is_finite() => *dimension,
            _ => return Err(AudioError::EnvironmentDimension),
        };
        let entries = match value.get("reverbs") {
            None => &[],
            Some(Json::Arr(entries)) => entries.as_slice(),
            _ => return Err(AudioError::EnvironmentReverbs),
        };
        let mut reverbs = Vec::with_capacity(entries.len());
        for entry in entries {
            let Json::Obj(_) = entry else {
                return Err(AudioError::ReverbObject);
            };
            let materials = match entry.get("materials") {
                None => None,
                Some(Json::Str(wildcard)) => {
                    if !wildcard.starts_with('*') {
                        return Err(AudioError::ReverbWildcard);
                    }
                    None
                }
                Some(Json::Arr(names)) => {
                    let mut materials = Vec::with_capacity(names.len());
                    for name in names {
                        let Json::Str(name) = name else {
                            return Err(AudioError::MaterialText);
                        };
                        materials.push(name.clone());
                    }
                    Some(materials)
                }
                _ => return Err(AudioError::ReverbMaterials),
            };
            let preset_index = match entry.get("preset") {
                None => 0,
                Some(Json::Str(name)) => match REVERB_PRESET_NAMES.iter().position(|preset| preset == name) {
                    Some(index) => index,
                    None => {
                        warn(&format!("Missing sound environment preset {name}"));
                        REVERB_PRESET_PLAIN
                    }
                },
                _ => return Err(AudioError::ReverbPresetText),
            };
            reverbs.push(ReverbMaterial {
                materials,
                preset_index,
            });
        }
        result.push(ReverbEnvironment { dimension, reverbs });
    }
    Ok(result)
}

/// Copy a preset by index.
pub fn reverb_preset(index: usize) -> Result<EfxReverbParams, AudioError> {
    REVERB_PRESETS.get(index).copied().ok_or(AudioError::UnknownPreset(index))
}

const PROBES: [[f64; 3]; 14] = [
    [0.0, 0.0, -1.0],
    [0.0, 0.0, 1.0],
    [0.707106769, 0.0, 0.707106769],
    [0.353553385, 0.612372458, 0.707106769],
    [-0.353553444, 0.612372458, 0.707106769],
    [-0.707106769, -6.18172393e-8, 0.707106769],
    [-0.353553325, -0.612372518, 0.707106769],
    [0.353553355, -0.612372458, 0.707106769],
    [1.0, 0.0, -4.37113883e-8],
    [0.49999997, 0.866025448, -4.37113883e-8],
    [-0.50000006, 0.866025388, -4.37113883e-8],
    [-1.0, -8.74227766e-8, -4.37113883e-8],
    [-0.499999911, -0.866025448, -4.37113883e-8],
    [0.499999911, -0.866025448, -4.37113883e-8],
];

fn interpolate(a: &EfxReverbParams, b: &EfxReverbParams, f: f64) -> EfxReverbParams {
    let lerp = |x: f64, y: f64| x + f * (y - x);
    EfxReverbParams {
        density: lerp(a.density, b.density),
        diffusion: lerp(a.diffusion, b.diffusion),
        gain: lerp(a.gain, b.gain),
        gain_hf: lerp(a.gain_hf, b.gain_hf),
        gain_lf: lerp(a.gain_lf, b.gain_lf),
        decay_time: lerp(a.decay_time, b.decay_time),
        decay_hf_ratio: lerp(a.decay_hf_ratio, b.decay_hf_ratio),
        decay_lf_ratio: lerp(a.decay_lf_ratio, b.decay_lf_ratio),
        reflections_gain: lerp(a.reflections_gain, b.reflections_gain),
        reflections_delay: lerp(a.reflections_delay, b.reflections_delay),
        late_reverb_gain: lerp(a.late_reverb_gain, b.late_reverb_gain),
        late_reverb_delay: lerp(a.late_reverb_delay, b.late_reverb_delay),
        echo_time: lerp(a.echo_time, b.echo_time),
        echo_depth: lerp(a.echo_depth, b.echo_depth),
        modulation_time: lerp(a.modulation_time, b.modulation_time),
        modulation_depth: lerp(a.modulation_depth, b.modulation_depth),
        air_absorption_gain_hf: lerp(a.air_absorption_gain_hf, b.air_absorption_gain_hf),
        hf_reference: lerp(a.hf_reference, b.hf_reference),
        lf_reference: lerp(a.lf_reference, b.lf_reference),
        room_rolloff_factor: lerp(a.room_rolloff_factor, b.room_rolloff_factor),
        decay_hf_limit: if f >= 0.5 { b.decay_hf_limit } else { a.decay_hf_limit },
    }
}

/// Per-listener environment selector with the repaired floor sweep.
pub struct EnvironmentReverb {
    environments: Vec<ReverbEnvironment>,
    trace: AudioTraceQuery,
    environment_index: usize,
    probe_index: usize,
    probe_time: f64,
    results: [[f64; 3]; 14],
    current_preset: usize,
    active: EfxReverbParams,
    from: EfxReverbParams,
    to: EfxReverbParams,
    lerp_start: f64,
    lerp_end: f64,
    /// Selector enabled.
    pub enabled: bool,
    /// Crossfade seconds.
    pub lerp_seconds: f64,
}

impl EnvironmentReverb {
    /// Selector over environments.
    #[must_use]
    pub fn new(environments: Vec<ReverbEnvironment>, trace: AudioTraceQuery) -> Self {
        let plain = REVERB_PRESETS[REVERB_PRESET_PLAIN];
        Self {
            environment_index: environments.len().saturating_sub(1),
            environments,
            trace,
            probe_index: 0,
            probe_time: 0.0,
            results: [[0.0; 3]; 14],
            current_preset: REVERB_PRESET_PLAIN,
            active: plain,
            from: plain,
            to: plain,
            lerp_start: 0.0,
            lerp_end: 0.0,
            enabled: true,
            lerp_seconds: 3.0,
        }
    }

    /// Active parameters, if any.
    #[must_use]
    pub fn params(&self) -> Option<EfxReverbParams> {
        (self.enabled && !self.environments.is_empty()).then_some(self.active)
    }

    /// Current preset index.
    #[must_use]
    pub const fn preset_index(&self) -> usize {
        self.current_preset
    }

    /// Update selection for a listener origin.
    pub fn update(&mut self, origin: Vec3, milliseconds: f64) -> Result<(), AudioError> {
        if !milliseconds.is_finite() {
            return Err(AudioError::ReverbTime);
        }
        if self.environments.is_empty() {
            return Ok(());
        }
        let origin = [f64::from(origin.x), f64::from(origin.y), f64::from(origin.z)];
        if milliseconds >= self.probe_time {
            self.probe_time = milliseconds + 13.0;
            let direction = PROBES.get(self.probe_index).ok_or(AudioError::ReverbProbe)?;
            let end = [
                origin[0] + 8192.0 * direction[0],
                origin[1] + 8192.0 * direction[1],
                origin[2] + 8192.0 * direction[2],
            ];
            let hit = (self.trace)(origin, end, [0.0; 3], [0.0; 3]);
            self.results[self.probe_index] = [
                hit.end[0] - origin[0],
                hit.end[1] - origin[1],
                hit.end[2] - origin[2] + if self.probe_index == 1 && hit.sky { 4096.0 } else { 0.0 },
            ];
            let span = |axis: usize| {
                let mut min = f64::INFINITY;
                let mut max = f64::NEG_INFINITY;
                for result in &self.results {
                    min = min.min(result[axis]);
                    max = max.max(result[axis]);
                }
                max - min
            };
            let average = (span(0) + span(1) + span(2)) / 3.0;
            let mut index = self.environment_index;
            while index < self.environments.len() - 1 && average > self.environments.get(index).map_or(f64::INFINITY, |environment| environment.dimension) {
                index += 1;
            }
            if index == self.environment_index {
                while index > 0 && average < self.environments.get(index - 1).map_or(f64::NEG_INFINITY, |environment| environment.dimension) {
                    index -= 1;
                }
            }
            self.environment_index = index;
            self.probe_index = (self.probe_index + 1) % PROBES.len();
        }
        let start = [origin[0], origin[1], origin[2] + 1.0];
        let floor = (self.trace)(start, [start[0], start[1], start[2] - 256.0], [-16.0, -16.0, 0.0], [16.0, 16.0, 0.0]);
        let mut selected = self.current_preset;
        if floor.fraction >= 1.0 || floor.sky {
            selected = REVERB_PRESET_PLAIN;
        } else {
            let environment = self.environments.get(self.environment_index).ok_or(AudioError::ReverbEnvironmentGone)?;
            let material = floor.material.as_ref().map(|material| material.to_lowercase());
            for entry in &environment.reverbs {
                let matches = entry.materials.is_none()
                    || entry.materials.as_ref().is_some_and(|materials| materials.iter().any(|name| Some(name.to_lowercase()) == material));
                if matches {
                    selected = entry.preset_index;
                    break;
                }
            }
        }
        if selected != self.current_preset {
            self.current_preset = selected;
            self.from = self.active;
            self.to = reverb_preset(selected)?;
            self.lerp_start = milliseconds;
            self.lerp_end = milliseconds + self.lerp_seconds.max(0.0) * 1000.0;
        }
        if milliseconds >= self.lerp_end {
            self.active = self.to;
        } else {
            let t = (milliseconds - self.lerp_start) / (self.lerp_end - self.lerp_start);
            let f = (1.0 - (1.0 - t).powi(3)).clamp(0.0, 1.0);
            self.active = interpolate(&self.from, &self.to, f);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_core::math::vec3;

    const JSON: &str = r#"{"environments": [
        {"dimension": 100, "reverbs": [{"materials": ["stone"], "preset": "cave"}, {"preset": "room"}]},
        {"dimension": 1000, "reverbs": [{"materials": "*", "preset": "plain"}]}
    ]}"#;

    #[test]
    fn parses_and_selects() {
        let mut warnings = Vec::new();
        let environments = parse_environments(JSON, &mut |message| warnings.push(message.to_string())).unwrap();
        assert_eq!(environments.len(), 2);
        assert_eq!(environments[0].reverbs[0].preset_index, 8);
        assert!(warnings.is_empty());
        let mut warnings = Vec::new();
        let fallback = parse_environments(r#"{"environments": [{"reverbs": [{"preset": "nope"}]}]}"#, &mut |message| {
            warnings.push(message.to_string())
        })
        .unwrap();
        assert_eq!(fallback[0].reverbs[0].preset_index, REVERB_PRESET_PLAIN);
        assert_eq!(warnings.len(), 1);
        assert!(parse_environments("{}", &mut |_| {}).is_err());
        assert!(parse_environments("{bad", &mut |_| {}).is_err());
        assert!(reverb_preset(26).is_err());
    }

    #[test]
    fn probes_and_crossfades() {
        let mut warnings = Vec::new();
        let environments = parse_environments(JSON, &mut |message| warnings.push(message.to_string())).unwrap();
        let mut selector = EnvironmentReverb::new(environments, Box::new(|start, end, mins, _| AudioTrace {
            // Probe sweeps hit at the origin (a small room); the floor sweep
            // reports stone.
            fraction: 0.5,
            end: if mins == [0.0; 3] { start } else { end },
            material: Some("stone".to_string()),
            sky: false,
        }));
        selector.lerp_seconds = 1.0;
        selector.update(vec3(0.0, 0.0, 0.0), 0.0).unwrap();
        assert_eq!(selector.preset_index(), 8);
        selector.update(vec3(0.0, 0.0, 0.0), 500.0).unwrap();
        let mid = selector.params().unwrap();
        assert!(mid.decay_time > REVERB_PRESETS[REVERB_PRESET_PLAIN].decay_time);
        selector.update(vec3(0.0, 0.0, 0.0), 1000.0).unwrap();
        assert_eq!(selector.params().unwrap(), REVERB_PRESETS[8]);
    }
}
