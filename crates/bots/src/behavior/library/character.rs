//! Bot character profiles from `src/bots/behavior/library/character.ts`
//! (`be_ai_char.c`, `chars.h`: `BotLoadCharacterSkill`,
//! `Characteristic_Float`/`Characteristic_String`/`Characteristic_BFloat`).
//!
//! A character file declares one `skill` block per level with named
//! characteristics. Loading interpolates between the bracketing skill
//! blocks for fractional skill; every float carries min/max bounds and
//! every lookup clamps into them.

use std::collections::HashMap;

use crate::behavior::assets::BotSourceFiles;
use crate::error::BotsError;

/// Character file search path.
pub const DEFAULT_CHARACTER_PATH: &str = "botfiles/bots.c";

/// Characteristic index (`chars.h`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Characteristic;

impl Characteristic {
    /// Bot name.
    pub const NAME: i32 = 0;
    /// Gender string.
    pub const GENDER: i32 = 1;
    /// Attack skill 0-1.
    pub const ATTACK_SKILL: i32 = 2;
    /// Weapon weights file.
    pub const WEAPON_WEIGHTS: i32 = 3;
    /// View turning factor.
    pub const VIEW_FACTOR: i32 = 4;
    /// View turning cap.
    pub const VIEW_MAX_CHANGE: i32 = 5;
    /// Reaction time seconds.
    pub const REACTION_TIME: i32 = 6;
    /// Aim accuracy 0-1.
    pub const AIM_ACCURACY: i32 = 7;
    /// Machinegun aim accuracy.
    pub const AIM_ACCURACY_MACHINEGUN: i32 = 8;
    /// Shotgun aim accuracy.
    pub const AIM_ACCURACY_SHOTGUN: i32 = 9;
    /// Rocket launcher aim accuracy.
    pub const AIM_ACCURACY_ROCKET_LAUNCHER: i32 = 10;
    /// Grenade launcher aim accuracy.
    pub const AIM_ACCURACY_GRENADE_LAUNCHER: i32 = 11;
    /// Lightning aim accuracy.
    pub const AIM_ACCURACY_LIGHTNING: i32 = 12;
    /// Plasmagun aim accuracy.
    pub const AIM_ACCURACY_PLASMA_GUN: i32 = 13;
    /// Railgun aim accuracy.
    pub const AIM_ACCURACY_RAILGUN: i32 = 14;
    /// BFG aim accuracy.
    pub const AIM_ACCURACY_BFG10K: i32 = 15;
    /// Aim skill 0-1.
    pub const AIM_SKILL: i32 = 16;
    /// Rocket launcher aim skill.
    pub const AIM_SKILL_ROCKET_LAUNCHER: i32 = 17;
    /// Grenade launcher aim skill.
    pub const AIM_SKILL_GRENADE_LAUNCHER: i32 = 18;
    /// Plasmagun aim skill.
    pub const AIM_SKILL_PLASMA_GUN: i32 = 19;
    /// BFG aim skill.
    pub const AIM_SKILL_BFG10K: i32 = 20;
    /// Chat file.
    pub const CHAT_FILE: i32 = 21;
    /// Chat name.
    pub const CHAT_NAME: i32 = 22;
    /// Chat characters per minute.
    pub const CHAT_CPM: i32 = 23;
    /// Insult chattiness.
    pub const CHAT_INSULT: i32 = 24;
    /// Misc chattiness.
    pub const CHAT_MISC: i32 = 25;
    /// Start/end-level chattiness.
    pub const CHAT_START_END_LEVEL: i32 = 26;
    /// Enter/exit-game chattiness.
    pub const CHAT_ENTER_EXIT_GAME: i32 = 27;
    /// Kill chattiness.
    pub const CHAT_KILL: i32 = 28;
    /// Death chattiness.
    pub const CHAT_DEATH: i32 = 29;
    /// Enemy-suicide chattiness.
    pub const CHAT_ENEMY_SUICIDE: i32 = 30;
    /// Hit-talking chattiness.
    pub const CHAT_HIT_TALKING: i32 = 31;
    /// Hit-no-death chattiness.
    pub const CHAT_HIT_NO_DEATH: i32 = 32;
    /// Hit-no-kill chattiness.
    pub const CHAT_HIT_NO_KILL: i32 = 33;
    /// Random chattiness.
    pub const CHAT_RANDOM: i32 = 34;
    /// Reply chattiness.
    pub const CHAT_REPLY: i32 = 35;
    /// Crouch tendency.
    pub const CROUCHER: i32 = 36;
    /// Jump tendency.
    pub const JUMPER: i32 = 37;
    /// Weapon-jump tendency.
    pub const WEAPON_JUMPING: i32 = 38;
    /// Grapple use.
    pub const GRAPPLE_USER: i32 = 39;
    /// Item weights file.
    pub const ITEM_WEIGHTS: i32 = 40;
    /// Aggression 0-1.
    pub const AGGRESSION: i32 = 41;
    /// Self-preservation 0-1.
    pub const SELF_PRESERVATION: i32 = 42;
    /// Vengefulness 0-1.
    pub const VENGEFULNESS: i32 = 43;
    /// Camping tendency.
    pub const CAMPER: i32 = 44;
    /// Easy-frag targeting.
    pub const EASY_FRAGGER: i32 = 45;
    /// Alertness 0-1.
    pub const ALERTNESS: i32 = 46;
    /// Fire throttle 0-1.
    pub const FIRE_THROTTLE: i32 = 47;
    /// Walk tendency.
    pub const WALKER: i32 = 48;
    /// Characteristic count.
    pub const COUNT: usize = 49;
}

/// Characteristic name table in index order.
pub const CHARACTERISTIC_NAMES: [&str; Characteristic::COUNT] = [
    "name",
    "gender",
    "attack_skill",
    "weaponweights",
    "view_factor",
    "view_maxchange",
    "reactiontime",
    "aim_accuracy",
    "aim_accuracy_machinegun",
    "aim_accuracy_shotgun",
    "aim_accuracy_rocketlauncher",
    "aim_accuracy_grenadelauncher",
    "aim_accuracy_lightning",
    "aim_accuracy_plasmagun",
    "aim_accuracy_railgun",
    "aim_accuracy_bfg10k",
    "aim_skill",
    "aim_skill_rocketlauncher",
    "aim_skill_grenadelauncher",
    "aim_skill_plasmagun",
    "aim_skill_bfg10k",
    "chat_file",
    "chat_name",
    "chat_cpm",
    "chat_insult",
    "chat_misc",
    "chat_startendlevel",
    "chat_enterexitgame",
    "chat_kill",
    "chat_death",
    "chat_enemysuicide",
    "chat_hittalking",
    "chat_hitnodeath",
    "chat_hitnokill",
    "chat_random",
    "chat_reply",
    "croucher",
    "jumper",
    "weaponjumping",
    "grapple_user",
    "itemweights",
    "aggression",
    "selfpreservation",
    "vengefulness",
    "camper",
    "easyfragger",
    "alertness",
    "firethrottle",
    "walker",
];

/// Index of a characteristic name, or `None`.
#[must_use]
pub fn characteristic_index(name: &str) -> Option<i32> {
    CHARACTERISTIC_NAMES
        .iter()
        .position(|candidate| candidate.eq_ignore_ascii_case(name))
        .map(|index| index as i32)
}

/// One characteristic value: bounded float or string.
#[derive(Debug, Clone, PartialEq)]
pub enum CharacteristicValue {
    /// Bounded float.
    Float {
        /// Value.
        value: f32,
        /// Minimum.
        min: f32,
        /// Maximum.
        max: f32,
    },
    /// String.
    String(String),
}

impl CharacteristicValue {
    /// Float value clamped into bounds.
    #[must_use]
    pub fn float(&self) -> Option<f32> {
        match self {
            CharacteristicValue::Float { value, min, max } => Some(value.clamp(*min, *max)),
            CharacteristicValue::String(_) => None,
        }
    }

    /// String value.
    #[must_use]
    pub fn string(&self) -> Option<&str> {
        match self {
            CharacteristicValue::String(text) => Some(text),
            CharacteristicValue::Float { .. } => None,
        }
    }
}

/// One `skill` block: values per characteristic index.
#[derive(Debug, Clone, Default)]
pub struct CharacterSkill {
    /// Skill level.
    pub skill: f32,
    /// Values by characteristic index.
    pub values: HashMap<i32, CharacteristicValue>,
}

/// Parsed character file: skill blocks plus defaults.
#[derive(Debug, Clone, Default)]
pub struct CharacterFile {
    /// Skill blocks sorted by skill.
    pub skills: Vec<CharacterSkill>,
}

impl CharacterFile {
    /// Parse `skill N { name value ... }` blocks.
    pub fn parse(text: &str) -> Result<Self, BotsError> {
        let tokens = super::structure::tokenize_bot_script(text);
        let mut skills = Vec::new();
        let mut index = 0;
        while index < tokens.len() {
            if !tokens[index].eq_ignore_ascii_case("skill") {
                return Err(BotsError::BotScript(format!(
                    "expected 'skill' at token {index}, found '{}'",
                    tokens[index]
                )));
            }
            index += 1;
            if index >= tokens.len() {
                return Err(BotsError::BotScript("skill block lacks a level".to_owned()));
            }
            let skill: f32 = tokens[index]
                .parse()
                .map_err(|_| BotsError::BotScript(format!("bad skill level '{}'", tokens[index])))?;
            index += 1;
            if index >= tokens.len() || tokens[index] != "{" {
                return Err(BotsError::BotScript("skill block lacks '{'".to_owned()));
            }
            index += 1;
            let mut values = HashMap::new();
            while index < tokens.len() && tokens[index] != "}" {
                let name = tokens[index].clone();
                index += 1;
                if index >= tokens.len() {
                    return Err(BotsError::BotScript("characteristic lacks a value".to_owned()));
                }
                let raw = tokens[index].clone();
                index += 1;
                let Some(characteristic) = characteristic_index(&name) else {
                    continue;
                };
                if let Ok(number) = raw.parse::<f32>() {
                    let (min, max) = if index + 1 < tokens.len()
                        && tokens[index] != "}"
                        && !characteristic_index(&tokens[index]).is_some()
                        && tokens[index].parse::<f32>().is_ok()
                        && tokens[index + 1].parse::<f32>().is_ok()
                    {
                        let min = tokens[index].parse::<f32>().unwrap_or(number);
                        let max = tokens[index + 1].parse::<f32>().unwrap_or(number);
                        index += 2;
                        (min, max)
                    } else {
                        (number, number)
                    };
                    values.insert(
                        characteristic,
                        CharacteristicValue::Float {
                            value: number,
                            min: min.min(max),
                            max: min.max(max),
                        },
                    );
                } else {
                    values.insert(characteristic, CharacteristicValue::String(raw));
                }
            }
            if index >= tokens.len() {
                return Err(BotsError::BotScript("unterminated skill block".to_owned()));
            }
            index += 1;
            skills.push(CharacterSkill { skill, values });
        }
        if skills.is_empty() {
            return Err(BotsError::BotScript("character file has no skill blocks".to_owned()));
        }
        skills.sort_by(|a, b| a.skill.total_cmp(&b.skill));
        Ok(Self { skills })
    }

    /// Interpolate values for a fractional skill.
    #[must_use]
    pub fn interpolate(&self, desired_skill: f32) -> HashMap<i32, CharacteristicValue> {
        let mut lower = &self.skills[0];
        let mut upper = &self.skills[self.skills.len() - 1];
        for window in self.skills.windows(2) {
            if desired_skill >= window[0].skill && desired_skill <= window[1].skill {
                lower = &window[0];
                upper = &window[1];
                break;
            }
        }
        if desired_skill <= self.skills[0].skill {
            return self.skills[0].values.clone();
        }
        if desired_skill >= upper.skill || (upper.skill - lower.skill).abs() < f32::EPSILON {
            return upper.values.clone();
        }
        let fraction = (desired_skill - lower.skill) / (upper.skill - lower.skill);
        let mut values = HashMap::new();
        for (index, value) in &lower.values {
            let other = upper.values.get(index);
            match (value, other) {
                (
                    CharacteristicValue::Float { value: a, min, max },
                    Some(CharacteristicValue::Float { value: b, .. }),
                ) => {
                    values.insert(
                        *index,
                        CharacteristicValue::Float {
                            value: a + (b - a) * fraction,
                            min: *min,
                            max: *max,
                        },
                    );
                }
                _ => {
                    values.insert(*index, value.clone());
                }
            }
        }
        for (index, value) in &upper.values {
            values.entry(*index).or_insert_with(|| value.clone());
        }
        values
    }
}

/// Loaded character handle.
#[derive(Debug, Clone)]
pub struct CharacterProfile {
    /// Source file.
    pub filename: String,
    /// Requested skill.
    pub skill: f32,
    /// Interpolated values.
    pub values: HashMap<i32, CharacteristicValue>,
}

/// Character diagnostic.
#[derive(Debug, Clone, PartialEq)]
pub struct CharacterDiagnostic {
    /// Severity.
    pub severity: CharacterSeverity,
    /// Message.
    pub message: String,
    /// Source file.
    pub source: String,
}

/// Character diagnostic severity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CharacterSeverity {
    /// Informational.
    Info,
    /// Warning.
    Warning,
    /// Error.
    Error,
    /// Fatal.
    Fatal,
}

/// Bot character library (`BotLoadCharacterSkill` and friends).
#[derive(Debug, Default)]
pub struct BotCharacterLibrary {
    profiles: Vec<Option<CharacterProfile>>,
    files: HashMap<String, CharacterFile>,
    diagnostics: Vec<CharacterDiagnostic>,
}

impl BotCharacterLibrary {
    /// Empty library (4 handle slots like the donor's initial table).
    #[must_use]
    pub fn new() -> Self {
        Self {
            profiles: vec![None, None, None, None],
            files: HashMap::new(),
            diagnostics: Vec::new(),
        }
    }

    /// Reported diagnostics.
    #[must_use]
    pub fn diagnostics(&self) -> &[CharacterDiagnostic] {
        &self.diagnostics
    }

    /// Load a character file from prepared sources.
    pub fn load_file(&mut self, files: &dyn BotSourceFiles, path: &str) -> Result<(), BotsError> {
        let bytes = files
            .read(path)
            .ok_or_else(|| BotsError::BotScript(format!("missing character file {path}")))?;
        let text = String::from_utf8_lossy(&bytes);
        let file = CharacterFile::parse(&text)?;
        self.files.insert(path.to_lowercase(), file);
        Ok(())
    }

    /// Load a character at a skill level; returns the handle.
    pub fn load_character_skill(
        &mut self,
        files: &dyn BotSourceFiles,
        path: &str,
        skill: f32,
    ) -> Result<i32, BotsError> {
        let key = path.to_lowercase();
        if !self.files.contains_key(&key) {
            self.load_file(files, path)?;
        }
        let file = self.files.get(&key).expect("character file just loaded");
        let values = file.interpolate(skill.clamp(1.0, 5.0));
        let handle = self.free_handle();
        self.profiles[handle] = Some(CharacterProfile {
            filename: path.to_owned(),
            skill,
            values,
        });
        self.diagnostics.push(CharacterDiagnostic {
            severity: CharacterSeverity::Info,
            message: format!("loaded {path} skill {skill}"),
            source: path.to_owned(),
        });
        Ok(handle as i32)
    }

    /// Free a character handle.
    pub fn free_character(&mut self, handle: i32) {
        if let Some(slot) = self.profiles.get_mut(handle as usize) {
            *slot = None;
        }
    }

    fn profile(&self, handle: i32) -> Result<&CharacterProfile, BotsError> {
        self.profiles
            .get(handle as usize)
            .and_then(Option::as_ref)
            .ok_or_else(|| BotsError::BotScript(format!("invalid character handle {handle}")))
    }

    /// Float characteristic clamped into bounds (`Characteristic_Float`).
    pub fn float(&self, handle: i32, index: i32) -> Result<f32, BotsError> {
        match self.profile(handle)?.values.get(&index) {
            Some(value) => value
                .float()
                .ok_or_else(|| BotsError::BotScript(format!("characteristic {index} is not a float"))),
            None => Ok(0.0),
        }
    }

    /// Bounded float (`Characteristic_BFloat`): clamped into `[min, max]`.
    pub fn bounded_float(&self, handle: i32, index: i32, min: f32, max: f32) -> f32 {
        self.float(handle, index).unwrap_or(min).clamp(min, max)
    }

    /// String characteristic (`Characteristic_String`).
    pub fn string(&self, handle: i32, index: i32) -> Result<String, BotsError> {
        match self.profile(handle)?.values.get(&index) {
            Some(value) => Ok(value.string().unwrap_or_default().to_owned()),
            None => Ok(String::new()),
        }
    }

    /// Dump a character for diagnostics (`BotDumpCharacter`).
    pub fn dump(&self, handle: i32) -> Result<Vec<(String, String)>, BotsError> {
        let profile = self.profile(handle)?;
        let mut entries = Vec::new();
        for (index, name) in CHARACTERISTIC_NAMES.iter().enumerate() {
            let text = match profile.values.get(&(index as i32)) {
                Some(CharacteristicValue::Float { value, .. }) => format!("{value}"),
                Some(CharacteristicValue::String(text)) => text.clone(),
                None => String::new(),
            };
            entries.push(((*name).to_owned(), text));
        }
        Ok(entries)
    }

    fn free_handle(&mut self) -> usize {
        if let Some(index) = self.profiles.iter().position(Option::is_none) {
            return index;
        }
        self.profiles.push(None);
        self.profiles.len() - 1
    }
}
