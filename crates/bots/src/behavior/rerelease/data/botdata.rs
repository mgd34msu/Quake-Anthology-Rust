//! Rerelease bot data from `src/bots/behavior/rerelease/data/botdata.ts`.
//!
//! Parsers for the shipped `bots/*.txt` family: characters, weapons,
//! items, monsters, interactables, game rules, teams, chats, skill
//! settings, and dangers. Unknown keys record errors without
//! dropping the entry.

use std::collections::HashMap;

use crate::behavior::rerelease::data::blockparse::{
    block_field, field_bool, field_flag_list, field_number, field_string, parse_blocks, Block, BlockField,
};

/// Bot data format.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum BotDataFormat {
    /// Quake 1.
    #[default]
    Q1,
    /// Quake 2.
    Q2,
}

/// Source entry base: block plus unknown fields.
#[derive(Debug, Clone, Default)]
pub struct BotSourceEntry {
    /// Source block.
    pub source: Option<Block>,
    /// Unknown fields.
    pub unknown: Vec<BlockField>,
}

fn unknown_key(errors: &mut Vec<String>, key: &str, line: usize) {
    errors.push(format!("line {line}: unknown key \"{key}\""));
}

fn expect_string(errors: &mut Vec<String>, key: &str, values: &[String], line: usize) -> Option<String> {
    match field_string(values) {
        Some(value) => Some(value),
        None => {
            errors.push(format!("line {line}: \"{key}\" expects a single string"));
            None
        }
    }
}

fn expect_number(errors: &mut Vec<String>, key: &str, values: &[String], line: usize) -> Option<f64> {
    match field_number(values) {
        Some(value) => Some(value),
        None => {
            errors.push(format!("line {line}: \"{key}\" expects a single number"));
            None
        }
    }
}

fn expect_bool(errors: &mut Vec<String>, key: &str, values: &[String], line: usize) -> Option<bool> {
    match field_bool(values) {
        Some(value) => Some(value),
        None => {
            errors.push(format!("line {line}: \"{key}\" expects \"true\" or \"false\""));
            None
        }
    }
}

/// Character entry (`characters.txt`).
#[derive(Debug, Clone, Default)]
pub struct CharacterEntry {
    /// Base entry.
    pub base: BotSourceEntry,
    /// Fun name.
    pub fun_name: String,
    /// Name.
    pub name: String,
    /// Skin.
    pub skin: String,
    /// Dogtag.
    pub dogtag: String,
    /// Shirt color.
    pub shirt_color: f64,
    /// Pants color.
    pub pants_color: f64,
}

/// Parse characters.
#[must_use]
pub fn parse_characters(text: &str) -> (Vec<CharacterEntry>, Vec<String>) {
    let parsed = parse_blocks(text);
    let mut errors: Vec<String> = parsed
        .errors
        .iter()
        .map(|error| format!("line {}: {}", error.line, error.message))
        .collect();
    let mut entries = Vec::new();
    for block in &parsed.blocks {
        let mut entry = CharacterEntry::default();
        entry.base.source = Some(block.clone());
        for field in &block.fields {
            match field.key.as_str() {
                "fun_name" => {
                    if let Some(value) = expect_string(&mut errors, &field.key, &field.values, field.line) {
                        entry.fun_name = value;
                    }
                }
                "name" => {
                    if let Some(value) = expect_string(&mut errors, &field.key, &field.values, field.line) {
                        entry.name = value;
                    }
                }
                "skin" => {
                    if let Some(value) = expect_string(&mut errors, &field.key, &field.values, field.line) {
                        entry.skin = value;
                    }
                }
                "dogtag" => {
                    if let Some(value) = expect_string(&mut errors, &field.key, &field.values, field.line) {
                        entry.dogtag = value;
                    }
                }
                "shirt_color" => {
                    if let Some(value) = expect_number(&mut errors, &field.key, &field.values, field.line) {
                        entry.shirt_color = value;
                    }
                }
                "pants_color" => {
                    if let Some(value) = expect_number(&mut errors, &field.key, &field.values, field.line) {
                        entry.pants_color = value;
                    }
                }
                _ => {
                    entry.base.unknown.push(field.clone());
                    unknown_key(&mut errors, &field.key, field.line);
                }
            }
        }
        entries.push(entry);
    }
    (entries, errors)
}

/// Weapon identity.
#[derive(Debug, Clone, PartialEq)]
pub enum BotWeaponIdentity {
    /// Q1 items bit.
    Q1Bit {
        /// Bit number.
        bit: i32,
    },
    /// Q2 classname.
    Q2Classname {
        /// Classname.
        classname: String,
    },
}

/// Trigger type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum TriggerType {
    /// Continuous.
    #[default]
    Continuous,
    /// Hold and release.
    HoldAndRelease,
}

/// Weapon entry (`weapons.txt`).
#[derive(Debug, Clone)]
pub struct WeaponEntry {
    /// Base entry.
    pub base: BotSourceEntry,
    /// Identity.
    pub identity: BotWeaponIdentity,
    /// Projectile speed.
    pub speed: f64,
    /// Ideal field of view.
    pub ideal_fov: f64,
    /// Trigger type.
    pub trigger_type: TriggerType,
    /// Trigger hold seconds.
    pub trigger_hold: f64,
    /// Trigger cooldown seconds.
    pub trigger_cooldown: f64,
    /// Name.
    pub name: String,
    /// Weapon number.
    pub number: i32,
    /// Damage.
    pub damage: f64,
    /// Minimum range.
    pub min_range: f64,
    /// Maximum range.
    pub max_range: f64,
    /// Minimum height.
    pub min_height: f64,
    /// Maximum height.
    pub max_height: f64,
    /// Priority.
    pub priority: f64,
    /// Ammo item.
    pub ammo: String,
    /// Ammo name.
    pub ammo_name: String,
    /// Minimum ammo.
    pub min_ammo: f64,
    /// Maximum ammo.
    pub max_ammo: f64,
    /// Flags.
    pub flags: Vec<String>,
    /// Aim point.
    pub aim_point: String,
}

impl Default for WeaponEntry {
    fn default() -> Self {
        Self {
            base: BotSourceEntry::default(),
            identity: BotWeaponIdentity::Q1Bit { bit: 0 },
            speed: 0.0,
            ideal_fov: 0.0,
            trigger_type: TriggerType::Continuous,
            trigger_hold: 0.0,
            trigger_cooldown: 0.0,
            name: String::new(),
            number: 0,
            damage: 0.0,
            min_range: 0.0,
            max_range: 0.0,
            min_height: 0.0,
            max_height: 0.0,
            priority: 0.0,
            ammo: String::new(),
            ammo_name: String::new(),
            min_ammo: 0.0,
            max_ammo: 0.0,
            flags: Vec::new(),
            aim_point: String::new(),
        }
    }
}

/// Parse weapons.
#[must_use]
pub fn parse_weapons(text: &str, format: BotDataFormat) -> (Vec<WeaponEntry>, Vec<String>) {
    let parsed = parse_blocks(text);
    let mut errors: Vec<String> = parsed
        .errors
        .iter()
        .map(|error| format!("line {}: {}", error.line, error.message))
        .collect();
    let mut entries = Vec::new();
    for block in &parsed.blocks {
        let mut entry = WeaponEntry::default();
        entry.base.source = Some(block.clone());
        for field in &block.fields {
            match field.key.as_str() {
                "name" => {
                    if let Some(value) = expect_string(&mut errors, &field.key, &field.values, field.line) {
                        entry.name = value;
                    }
                }
                "speed" => {
                    if let Some(value) = expect_number(&mut errors, &field.key, &field.values, field.line) {
                        entry.speed = value;
                    }
                }
                "ideal_fov" => {
                    if let Some(value) = expect_number(&mut errors, &field.key, &field.values, field.line) {
                        entry.ideal_fov = value;
                    }
                }
                "trigger_hold" => {
                    if let Some(value) = expect_number(&mut errors, &field.key, &field.values, field.line) {
                        entry.trigger_hold = value;
                    }
                }
                "trigger_cooldown" => {
                    if let Some(value) = expect_number(&mut errors, &field.key, &field.values, field.line) {
                        entry.trigger_cooldown = value;
                    }
                }
                "trigger_type" => {
                    if let Some(value) = expect_string(&mut errors, &field.key, &field.values, field.line) {
                        match value.as_str() {
                            "continuous" => entry.trigger_type = TriggerType::Continuous,
                            "hold_and_release" => entry.trigger_type = TriggerType::HoldAndRelease,
                            _ => errors.push(format!("line {}: unsupported trigger_type \"{}\"", field.line, value)),
                        }
                    }
                }
                "number" => {
                    if let Some(value) = expect_number(&mut errors, &field.key, &field.values, field.line) {
                        entry.number = value as i32;
                    }
                }
                "damage" => {
                    if let Some(value) = expect_number(&mut errors, &field.key, &field.values, field.line) {
                        entry.damage = value;
                    }
                }
                "min_range" => {
                    if let Some(value) = expect_number(&mut errors, &field.key, &field.values, field.line) {
                        entry.min_range = value;
                    }
                }
                "max_range" => {
                    if let Some(value) = expect_number(&mut errors, &field.key, &field.values, field.line) {
                        entry.max_range = value;
                    }
                }
                "min_height" => {
                    if let Some(value) = expect_number(&mut errors, &field.key, &field.values, field.line) {
                        entry.min_height = value;
                    }
                }
                "max_height" => {
                    if let Some(value) = expect_number(&mut errors, &field.key, &field.values, field.line) {
                        entry.max_height = value;
                    }
                }
                "priority" => {
                    if let Some(value) = expect_number(&mut errors, &field.key, &field.values, field.line) {
                        entry.priority = value;
                    }
                }
                "ammo" => {
                    if let Some(value) = expect_string(&mut errors, &field.key, &field.values, field.line) {
                        entry.ammo = value;
                    }
                }
                "ammo_name" => {
                    if let Some(value) = expect_string(&mut errors, &field.key, &field.values, field.line) {
                        entry.ammo_name = value;
                    }
                }
                "min_ammo" => {
                    if let Some(value) = expect_number(&mut errors, &field.key, &field.values, field.line) {
                        entry.min_ammo = value;
                    }
                }
                "max_ammo" => {
                    if let Some(value) = expect_number(&mut errors, &field.key, &field.values, field.line) {
                        entry.max_ammo = value;
                    }
                }
                "flags" => entry.flags = field_flag_list(&field.values),
                "aim_point" => {
                    if let Some(value) = expect_string(&mut errors, &field.key, &field.values, field.line) {
                        entry.aim_point = value;
                    }
                }
                _ => {
                    entry.base.unknown.push(field.clone());
                    unknown_key(&mut errors, &field.key, field.line);
                }
            }
        }
        entry.identity = match format {
            BotDataFormat::Q1 => BotWeaponIdentity::Q1Bit { bit: entry.number },
            BotDataFormat::Q2 => BotWeaponIdentity::Q2Classname {
                classname: entry.name.clone(),
            },
        };
        entries.push(entry);
    }
    (entries, errors)
}

/// One `spawnflags BIT = name` pair.
#[derive(Debug, Clone, Default)]
pub struct ItemSpawnflag {
    /// Bit.
    pub bit: i32,
    /// Name.
    pub name: String,
}

/// Item entry (`items.txt`).
#[derive(Debug, Clone)]
pub struct ItemEntry {
    /// Base entry.
    pub base: BotSourceEntry,
    /// Sight distance (FLT_MAX default).
    pub sight_dist: f64,
    /// Name.
    pub name: String,
    /// Spawnflags.
    pub spawnflags: Vec<ItemSpawnflag>,
    /// Flags.
    pub flags: Vec<String>,
    /// Team (CTF flags only).
    pub team: Option<f64>,
}

impl Default for ItemEntry {
    fn default() -> Self {
        Self {
            base: BotSourceEntry::default(),
            sight_dist: f64::from(f32::MAX),
            name: String::new(),
            spawnflags: Vec::new(),
            flags: Vec::new(),
            team: None,
        }
    }
}

/// Parse items.
#[must_use]
pub fn parse_items(text: &str) -> (Vec<ItemEntry>, Vec<String>) {
    let parsed = parse_blocks(text);
    let mut errors: Vec<String> = parsed
        .errors
        .iter()
        .map(|error| format!("line {}: {}", error.line, error.message))
        .collect();
    let mut entries = Vec::new();
    for block in &parsed.blocks {
        let mut entry = ItemEntry::default();
        entry.base.source = Some(block.clone());
        for field in &block.fields {
            match field.key.as_str() {
                "name" => {
                    if let Some(value) = expect_string(&mut errors, &field.key, &field.values, field.line) {
                        entry.name = value;
                    }
                }
                "sight_dist" => {
                    if let Some(value) = expect_number(&mut errors, &field.key, &field.values, field.line) {
                        entry.sight_dist = value;
                    }
                }
                "flags" => entry.flags = field_flag_list(&field.values),
                "spawnflags" => {
                    if field.values.len() == 3 && field.values[1] == "=" {
                        match field.values[0].parse::<i32>() {
                            Ok(bit) => entry.spawnflags.push(ItemSpawnflag {
                                bit,
                                name: field.values[2].clone(),
                            }),
                            Err(_) => errors.push(format!(
                                "line {}: \"spawnflags\" bit \"{}\" is not an integer",
                                field.line, field.values[0]
                            )),
                        }
                    } else {
                        errors.push(format!("line {}: \"spawnflags\" expects \"BIT = name\"", field.line));
                    }
                }
                "team" => {
                    if let Some(value) = expect_number(&mut errors, &field.key, &field.values, field.line) {
                        entry.team = Some(value);
                    }
                }
                _ => {
                    entry.base.unknown.push(field.clone());
                    unknown_key(&mut errors, &field.key, field.line);
                }
            }
        }
        entries.push(entry);
    }
    (entries, errors)
}

/// Monster entry (`monsters.txt`).
#[derive(Debug, Clone, Default)]
pub struct MonsterEntry {
    /// Base entry.
    pub base: BotSourceEntry,
    /// Classname.
    pub classname: String,
    /// Flags.
    pub flags: Vec<String>,
}

/// Parse monsters.
#[must_use]
pub fn parse_monsters(text: &str) -> (Vec<MonsterEntry>, Vec<String>) {
    let parsed = parse_blocks(text);
    let mut errors: Vec<String> = parsed
        .errors
        .iter()
        .map(|error| format!("line {}: {}", error.line, error.message))
        .collect();
    let mut entries = Vec::new();
    for block in &parsed.blocks {
        let mut entry = MonsterEntry::default();
        entry.base.source = Some(block.clone());
        for field in &block.fields {
            match field.key.as_str() {
                "classname" => {
                    if let Some(value) = expect_string(&mut errors, &field.key, &field.values, field.line) {
                        entry.classname = value;
                    }
                }
                "flags" => entry.flags = field_flag_list(&field.values),
                _ => {
                    entry.base.unknown.push(field.clone());
                    unknown_key(&mut errors, &field.key, field.line);
                }
            }
        }
        entries.push(entry);
    }
    (entries, errors)
}

/// Interactable bounds.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BotInteractableBounds {
    /// Mins.
    pub mins: [f64; 3],
    /// Maxs.
    pub maxs: [f64; 3],
}

/// Interactable entry (`interactables.txt`).
#[derive(Debug, Clone, Default)]
pub struct InteractableEntry {
    /// Base entry.
    pub base: BotSourceEntry,
    /// Bounds.
    pub bounds: Option<BotInteractableBounds>,
    /// Name.
    pub name: String,
    /// Interaction.
    pub interaction: String,
    /// Plain integer spawnflags.
    pub spawnflags: Option<f64>,
    /// Health condition.
    pub health: Option<bool>,
    /// Targetname condition.
    pub targetname: Option<bool>,
    /// Logic op.
    pub logic_op: Option<String>,
}

/// Parse interactables.
#[must_use]
pub fn parse_interactables(text: &str) -> (Vec<InteractableEntry>, Vec<String>) {
    let parsed = parse_blocks(text);
    let mut errors: Vec<String> = parsed
        .errors
        .iter()
        .map(|error| format!("line {}: {}", error.line, error.message))
        .collect();
    let mut entries = Vec::new();
    for block in &parsed.blocks {
        let mut entry = InteractableEntry::default();
        entry.base.source = Some(block.clone());
        for field in &block.fields {
            match field.key.as_str() {
                "name" => {
                    if let Some(value) = expect_string(&mut errors, &field.key, &field.values, field.line) {
                        entry.name = value;
                    }
                }
                "interaction" => {
                    if let Some(value) = expect_string(&mut errors, &field.key, &field.values, field.line) {
                        entry.interaction = value;
                    }
                }
                "spawnflags" => {
                    if let Some(value) = expect_number(&mut errors, &field.key, &field.values, field.line) {
                        entry.spawnflags = Some(value);
                    }
                }
                "health" => {
                    if let Some(value) = expect_bool(&mut errors, &field.key, &field.values, field.line) {
                        entry.health = Some(value);
                    }
                }
                "targetname" => {
                    if let Some(value) = expect_bool(&mut errors, &field.key, &field.values, field.line) {
                        entry.targetname = Some(value);
                    }
                }
                "bounds" => {
                    let values = &field.values;
                    let numbers: Vec<f64> = [1, 2, 3, 6, 7, 8]
                        .iter()
                        .filter_map(|index| values.get(*index)?.parse::<f64>().ok())
                        .collect();
                    if values.len() == 10
                        && values[0] == "{"
                        && values[4] == "}"
                        && values[5] == "{"
                        && values[9] == "}"
                        && numbers.len() == 6
                    {
                        entry.bounds = Some(BotInteractableBounds {
                            mins: [numbers[0], numbers[1], numbers[2]],
                            maxs: [numbers[3], numbers[4], numbers[5]],
                        });
                    } else {
                        errors.push(format!(
                            "line {}: \"bounds\" expects {{ x y z }} {{ x y z }}",
                            field.line
                        ));
                    }
                }
                "logic_op" => {
                    if let Some(value) = expect_string(&mut errors, &field.key, &field.values, field.line) {
                        entry.logic_op = Some(value);
                    }
                }
                _ => {
                    entry.base.unknown.push(field.clone());
                    unknown_key(&mut errors, &field.key, field.line);
                }
            }
        }
        entries.push(entry);
    }
    (entries, errors)
}

/// Rule condition.
#[derive(Debug, Clone, PartialEq)]
pub struct BotRuleCondition {
    /// Cvar.
    pub cvar: String,
    /// Value.
    pub value: f64,
}

/// Game rule entry (`game_rules.txt`).
#[derive(Debug, Clone)]
pub struct GameRuleEntry {
    /// Base entry.
    pub base: BotSourceEntry,
    /// Conditions.
    pub conditions: Vec<BotRuleCondition>,
    /// Only game type.
    pub only_game_type: Option<String>,
    /// Not game type.
    pub not_game_type: Option<String>,
    /// Logic op.
    pub logic_op: String,
    /// Has teams.
    pub has_teams: Option<bool>,
    /// Team damage.
    pub team_damage: Option<bool>,
    /// Weapon stay specified.
    pub weapon_stay_specified: bool,
    /// Cvar.
    pub cvar: String,
    /// Value.
    pub value: f64,
    /// Weapon stay.
    pub weapon_stay: bool,
    /// Game type.
    pub game_type: String,
}

impl Default for GameRuleEntry {
    fn default() -> Self {
        Self {
            base: BotSourceEntry::default(),
            conditions: Vec::new(),
            only_game_type: None,
            not_game_type: None,
            logic_op: "or".to_owned(),
            has_teams: None,
            team_damage: None,
            weapon_stay_specified: false,
            cvar: String::new(),
            value: 0.0,
            weapon_stay: false,
            game_type: String::new(),
        }
    }
}

/// Parse game rules.
#[must_use]
pub fn parse_game_rules(text: &str) -> (Vec<GameRuleEntry>, Vec<String>) {
    let parsed = parse_blocks(text);
    let mut errors: Vec<String> = parsed
        .errors
        .iter()
        .map(|error| format!("line {}: {}", error.line, error.message))
        .collect();
    let mut entries = Vec::new();
    for block in &parsed.blocks {
        let mut entry = GameRuleEntry::default();
        entry.base.source = Some(block.clone());
        let mut cvars = Vec::new();
        let mut values = Vec::new();
        for field in &block.fields {
            match field.key.as_str() {
                "cvar" => {
                    if let Some(value) = expect_string(&mut errors, &field.key, &field.values, field.line) {
                        entry.cvar = value.clone();
                        cvars.push(value);
                    }
                }
                "value" => {
                    if let Some(value) = expect_number(&mut errors, &field.key, &field.values, field.line) {
                        entry.value = value;
                        values.push(value);
                    }
                }
                "weapon_stay" => {
                    if let Some(value) = expect_bool(&mut errors, &field.key, &field.values, field.line) {
                        entry.weapon_stay = value;
                        entry.weapon_stay_specified = true;
                    }
                }
                "has_teams" => {
                    if let Some(value) = expect_bool(&mut errors, &field.key, &field.values, field.line) {
                        entry.has_teams = Some(value);
                    }
                }
                "team_damage" => {
                    if let Some(value) = expect_bool(&mut errors, &field.key, &field.values, field.line) {
                        entry.team_damage = Some(value);
                    }
                }
                "only_gametype" => {
                    if let Some(value) = expect_string(&mut errors, &field.key, &field.values, field.line) {
                        entry.only_game_type = Some(value);
                    }
                }
                "not_gametype" => {
                    if let Some(value) = expect_string(&mut errors, &field.key, &field.values, field.line) {
                        entry.not_game_type = Some(value);
                    }
                }
                "logic_op" => {
                    if let Some(value) = expect_string(&mut errors, &field.key, &field.values, field.line) {
                        if value == "and" || value == "or" {
                            entry.logic_op = value;
                        } else {
                            errors.push(format!("line {}: unsupported logic_op \"{}\"", field.line, value));
                        }
                    }
                }
                "game_type" => {
                    if let Some(value) = expect_string(&mut errors, &field.key, &field.values, field.line) {
                        entry.game_type = value;
                    }
                }
                _ => {
                    entry.base.unknown.push(field.clone());
                    unknown_key(&mut errors, &field.key, field.line);
                }
            }
        }
        if cvars.len() != values.len() || cvars.len() > 2 {
            errors.push(format!(
                "line {}: game rule expects one or two paired cvar/value entries",
                block.line
            ));
        }
        for (index, cvar) in cvars.iter().enumerate() {
            if let Some(value) = values.get(index) {
                entry.conditions.push(BotRuleCondition {
                    cvar: cvar.clone(),
                    value: *value,
                });
            }
        }
        entries.push(entry);
    }
    (entries, errors)
}

/// Team entry (`teams.txt`).
#[derive(Debug, Clone, Default)]
pub struct TeamEntry {
    /// Base entry.
    pub base: BotSourceEntry,
    /// Value.
    pub value: f64,
    /// Name.
    pub name: String,
}

/// Parse teams.
#[must_use]
pub fn parse_teams(text: &str) -> (Vec<TeamEntry>, Vec<String>) {
    let parsed = parse_blocks(text);
    let mut errors: Vec<String> = parsed
        .errors
        .iter()
        .map(|error| format!("line {}: {}", error.line, error.message))
        .collect();
    let mut entries = Vec::new();
    for block in &parsed.blocks {
        let mut entry = TeamEntry::default();
        entry.base.source = Some(block.clone());
        for field in &block.fields {
            match field.key.as_str() {
                "value" => {
                    if let Some(value) = expect_number(&mut errors, &field.key, &field.values, field.line) {
                        entry.value = value;
                    }
                }
                "name" => {
                    if let Some(value) = expect_string(&mut errors, &field.key, &field.values, field.line) {
                        entry.name = value;
                    }
                }
                _ => {
                    entry.base.unknown.push(field.clone());
                    unknown_key(&mut errors, &field.key, field.line);
                }
            }
        }
        entries.push(entry);
    }
    (entries, errors)
}

/// Chat entry (`chats.txt`).
#[derive(Debug, Clone, Default)]
pub struct ChatEntry {
    /// Base entry.
    pub base: BotSourceEntry,
    /// Locstring.
    pub locstring: String,
    /// Type.
    pub chat_type: String,
    /// Delay milliseconds.
    pub time: f64,
    /// Chance.
    pub chance: f64,
    /// Team only.
    pub team: bool,
}

/// Parse chats.
#[must_use]
pub fn parse_chats(text: &str) -> (Vec<ChatEntry>, Vec<String>) {
    let parsed = parse_blocks(text);
    let mut errors: Vec<String> = parsed
        .errors
        .iter()
        .map(|error| format!("line {}: {}", error.line, error.message))
        .collect();
    let mut entries = Vec::new();
    for block in &parsed.blocks {
        let mut entry = ChatEntry::default();
        entry.base.source = Some(block.clone());
        for field in &block.fields {
            match field.key.as_str() {
                "locstring" => {
                    if let Some(value) = expect_string(&mut errors, &field.key, &field.values, field.line) {
                        entry.locstring = value;
                    }
                }
                "type" => {
                    if let Some(value) = expect_string(&mut errors, &field.key, &field.values, field.line) {
                        entry.chat_type = value;
                    }
                }
                "time" => {
                    if let Some(value) = expect_number(&mut errors, &field.key, &field.values, field.line) {
                        entry.time = value;
                    }
                }
                "chance" => {
                    if let Some(value) = expect_number(&mut errors, &field.key, &field.values, field.line) {
                        entry.chance = value;
                    }
                }
                "team" => {
                    if let Some(value) = expect_bool(&mut errors, &field.key, &field.values, field.line) {
                        entry.team = value;
                    }
                }
                _ => {
                    entry.base.unknown.push(field.clone());
                    unknown_key(&mut errors, &field.key, field.line);
                }
            }
        }
        entries.push(entry);
    }
    (entries, errors)
}

/// Aiming settings.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct BotAimingSettings {
    /// Maximum acceleration (read as speed clamp).
    pub max_acceleration: f32,
    /// Spring stiffness.
    pub spring_stiffness: f32,
    /// Damping.
    pub damping: f32,
    /// Velocity offset seconds.
    pub velocity_offset: f32,
    /// Lead targets.
    pub lead_targets: bool,
    /// Modifier max angle.
    pub modifier_max_angle: f32,
    /// Modifier apply time.
    pub modifier_apply_time: f32,
    /// Modifier accel scalar.
    pub modifier_accel_scalar: f32,
    /// Modifier spring scalar.
    pub modifier_spring_scalar: f32,
    /// Modifier damping scalar.
    pub modifier_damping_scalar: f32,
}

/// Behavior settings.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BotBehaviorSettings {
    /// Combat max item distance.
    pub combat_max_item_dist: f32,
    /// Combat minimum health percent.
    pub combat_min_health_pct: f32,
    /// Combat minimum ammo percent.
    pub combat_min_ammo_pct: f32,
    /// Combat minimum armor percent.
    pub combat_min_armor_pct: f32,
    /// Combat grab weapons.
    pub combat_grab_weapons: bool,
    /// React to dangers.
    pub react_to_dangers: bool,
    /// Allow combat.
    pub allow_combat: bool,
    /// Allow grab items in combat.
    pub allow_grab_items_in_combat: bool,
    /// Allow melee.
    pub allow_melee: bool,
    /// Allow check-six.
    pub allow_check_six: bool,
    /// Allow grab items.
    pub allow_grab_items: bool,
    /// Allow grab power items.
    pub allow_grab_power_items: bool,
    /// Defer power items to humans.
    pub defer_power_items_to_humans: bool,
    /// Minimum respawn time.
    pub min_respawn_time: f32,
    /// Maximum respawn time.
    pub max_respawn_time: f32,
}

impl Default for BotBehaviorSettings {
    fn default() -> Self {
        Self {
            combat_max_item_dist: 512.0,
            combat_min_health_pct: 100.0,
            combat_min_ammo_pct: 100.0,
            combat_min_armor_pct: 100.0,
            combat_grab_weapons: true,
            react_to_dangers: false,
            allow_combat: false,
            allow_grab_items_in_combat: false,
            allow_melee: false,
            allow_check_six: false,
            allow_grab_items: false,
            allow_grab_power_items: false,
            defer_power_items_to_humans: false,
            min_respawn_time: 0.0,
            max_respawn_time: 0.0,
        }
    }
}

/// Movement settings.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct BotMovementSettings {
    /// Allow jumping in combat.
    pub allow_jumping_in_combat: bool,
    /// Jump chance.
    pub jump_chance: f32,
    /// Jump cooldown.
    pub jump_cooldown: f32,
    /// Walk only.
    pub walk_only: bool,
    /// Allow rocket jumping.
    pub allow_rocket_jumping: bool,
}

/// Senses settings.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct BotSensesSettings {
    /// Sight fill time.
    pub sight_time: f32,
    /// Sight decay time.
    pub sight_decay_time: f32,
    /// Invisible enemy sight scalar.
    pub invis_enemy_sight_scalar: f32,
    /// Maximum invisible sight distance.
    pub max_invis_enemy_sight_dist: f32,
    /// Field of view angle.
    pub fov_angle: f32,
    /// Forget non-visible enemy time.
    pub forget_non_vis_enemy_time: f32,
    /// Sound range.
    pub sound_range: f32,
    /// Sound fill time.
    pub sound_time: f32,
    /// Sound decay time.
    pub sound_decay_time: f32,
    /// Sound persist time.
    pub sound_persist_time: f32,
}

/// Weapon sense settings.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct BotWeaponSenseSettings {
    /// FOV scalar.
    pub fov_scalar: f32,
    /// Decay time.
    pub decay_time: f32,
    /// FOV angle.
    pub fov_angle: f32,
    /// Sight fill time.
    pub sight_time: f32,
}

/// Skill settings: one `skill <name>` block.
#[derive(Debug, Clone, Default)]
pub struct BotSkillSettings {
    /// Skill name.
    pub skill: String,
    /// Aiming.
    pub aiming: BotAimingSettings,
    /// Behaviors.
    pub behaviors: BotBehaviorSettings,
    /// Movement.
    pub movement: BotMovementSettings,
    /// Senses.
    pub senses: BotSensesSettings,
    /// Weapons.
    pub weapons: BotWeaponSenseSettings,
    /// Unknown dotted keys kept verbatim.
    pub unknown: HashMap<String, Vec<String>>,
}

/// Parse skill settings.
#[must_use]
pub fn parse_bot_settings(text: &str) -> (Vec<BotSkillSettings>, Vec<String>) {
    let parsed = parse_blocks(text);
    let mut errors: Vec<String> = parsed
        .errors
        .iter()
        .map(|error| format!("line {}: {}", error.line, error.message))
        .collect();
    let mut skills = Vec::new();
    for block in &parsed.blocks {
        if block.header.first().is_some_and(|head| head == "skill") {
            let mut settings = BotSkillSettings {
                skill: block.header.get(1).cloned().unwrap_or_default(),
                ..BotSkillSettings::default()
            };
            parse_skill_block(block, &mut settings, &mut errors);
            skills.push(settings);
        } else {
            errors.push(format!("line {}: expected 'skill <name>' block", block.line));
        }
    }
    (skills, errors)
}

fn num(block: &Block, key: &str) -> Option<f32> {
    block_field(block, key)
        .and_then(|field| field_number(&field.values))
        .map(|value| value as f32)
}

fn boolean(block: &Block, key: &str) -> Option<bool> {
    block_field(block, key).and_then(|field| field_bool(&field.values))
}

fn parse_skill_block(block: &Block, settings: &mut BotSkillSettings, errors: &mut Vec<String>) {
    let aiming = &mut settings.aiming;
    if let Some(value) = num(block, "aiming.max_acceleration") {
        aiming.max_acceleration = value;
    }
    if let Some(value) = num(block, "aiming.spring_stiffness") {
        aiming.spring_stiffness = value;
    }
    if let Some(value) = num(block, "aiming.damping") {
        aiming.damping = value;
    }
    if let Some(value) = num(block, "aiming.velocity_offset") {
        aiming.velocity_offset = value;
    }
    if let Some(value) = boolean(block, "aiming.lead_targets") {
        aiming.lead_targets = value;
    }
    if let Some(value) = num(block, "aiming.modifier.max_angle") {
        aiming.modifier_max_angle = value;
    }
    if let Some(value) = num(block, "aiming.modifier.apply_time") {
        aiming.modifier_apply_time = value;
    }
    if let Some(value) = num(block, "aiming.modifier.accel_scalar") {
        aiming.modifier_accel_scalar = value;
    }
    if let Some(value) = num(block, "aiming.modifier.spring_scalar") {
        aiming.modifier_spring_scalar = value;
    }
    if let Some(value) = num(block, "aiming.modifier.damping_scalar") {
        aiming.modifier_damping_scalar = value;
    }
    let behaviors = &mut settings.behaviors;
    if let Some(value) = num(block, "behaviors.combat.max_item_dist") {
        behaviors.combat_max_item_dist = value;
    }
    if let Some(value) = num(block, "behaviors.combat.min_health_pct") {
        behaviors.combat_min_health_pct = value;
    }
    if let Some(value) = num(block, "behaviors.combat.min_ammo_pct") {
        behaviors.combat_min_ammo_pct = value;
    }
    if let Some(value) = num(block, "behaviors.combat.min_armor_pct") {
        behaviors.combat_min_armor_pct = value;
    }
    if let Some(value) = boolean(block, "behaviors.combat.grab_weapons") {
        behaviors.combat_grab_weapons = value;
    }
    if let Some(value) = boolean(block, "behaviors.react_to_dangers") {
        behaviors.react_to_dangers = value;
    }
    if let Some(value) = boolean(block, "behaviors.allow_combat") {
        behaviors.allow_combat = value;
    }
    if let Some(value) = boolean(block, "behaviors.combat.allow_grab_items")
        .or_else(|| boolean(block, "behaviors.allow_grab_items_in_combat"))
    {
        behaviors.allow_grab_items_in_combat = value;
    }
    if let Some(value) =
        boolean(block, "behaviors.combat.allow_melee").or_else(|| boolean(block, "behaviors.allow_melee"))
    {
        behaviors.allow_melee = value;
    }
    if let Some(value) = boolean(block, "behaviors.allow_check_six") {
        behaviors.allow_check_six = value;
    }
    if let Some(value) = boolean(block, "behaviors.allow_grab_items") {
        behaviors.allow_grab_items = value;
    }
    if let Some(value) = boolean(block, "behaviors.allow_grab_power_items") {
        behaviors.allow_grab_power_items = value;
    }
    if let Some(value) = boolean(block, "behaviors.defer_power_items_to_humans") {
        behaviors.defer_power_items_to_humans = value;
    }
    if let Some(value) = num(block, "behaviors.min_respawn_time") {
        behaviors.min_respawn_time = value;
    }
    if let Some(value) = num(block, "behaviors.max_respawn_time") {
        behaviors.max_respawn_time = value;
    }
    let movement = &mut settings.movement;
    if let Some(value) = boolean(block, "movement.allow_jumping_in_combat") {
        movement.allow_jumping_in_combat = value;
    }
    if let Some(value) = num(block, "movement.jump_chance") {
        movement.jump_chance = value;
    }
    if let Some(value) = num(block, "movement.jump_cooldown") {
        movement.jump_cooldown = value;
    }
    if let Some(value) = boolean(block, "movement.walk_only") {
        movement.walk_only = value;
    }
    if let Some(value) = boolean(block, "movement.allow_rocket_jumping") {
        movement.allow_rocket_jumping = value;
    }
    let senses = &mut settings.senses;
    if let Some(value) = num(block, "senses.sight_time") {
        senses.sight_time = value;
    }
    if let Some(value) = num(block, "senses.sight_decay_time") {
        senses.sight_decay_time = value;
    }
    if let Some(value) = num(block, "senses.invis_enemy_sight_scalar") {
        senses.invis_enemy_sight_scalar = value;
    }
    if let Some(value) = num(block, "senses.max_invis_enemy_sight_dist") {
        senses.max_invis_enemy_sight_dist = value;
    }
    if let Some(value) = num(block, "senses.fov_angle") {
        senses.fov_angle = value;
    }
    if let Some(value) = num(block, "senses.forget_non_vis_enemy_time") {
        senses.forget_non_vis_enemy_time = value;
    }
    if let Some(value) = num(block, "senses.sound_range") {
        senses.sound_range = value;
    }
    if let Some(value) = num(block, "senses.sound_time") {
        senses.sound_time = value;
    }
    if let Some(value) = num(block, "senses.sound_decay_time") {
        senses.sound_decay_time = value;
    }
    if let Some(value) = num(block, "senses.sound_persist_time") {
        senses.sound_persist_time = value;
    }
    let weapons = &mut settings.weapons;
    if let Some(value) = num(block, "weapons.fov_scalar") {
        weapons.fov_scalar = value;
    }
    if let Some(value) = num(block, "weapons.decay_time") {
        weapons.decay_time = value;
    }
    if let Some(value) = num(block, "weapons.fov_angle") {
        weapons.fov_angle = value;
    }
    if let Some(value) = num(block, "weapons.sight_time") {
        weapons.sight_time = value;
    }
    const KNOWN: &[&str] = &[
        "aiming.lead_targets",
        "behaviors.combat.max_item_dist",
        "behaviors.combat.min_health_pct",
        "behaviors.combat.min_ammo_pct",
        "behaviors.combat.min_armor_pct",
        "behaviors.combat.grab_weapons",
        "behaviors.react_to_dangers",
        "movement.allow_rocket_jumping",
        "weapons.fov_scalar",
        "aiming.max_acceleration",
        "aiming.spring_stiffness",
        "aiming.damping",
        "aiming.velocity_offset",
        "aiming.modifier.max_angle",
        "aiming.modifier.apply_time",
        "aiming.modifier.accel_scalar",
        "aiming.modifier.spring_scalar",
        "aiming.modifier.damping_scalar",
        "behaviors.allow_combat",
        "behaviors.combat.allow_grab_items",
        "behaviors.allow_grab_items_in_combat",
        "behaviors.combat.allow_melee",
        "behaviors.allow_melee",
        "behaviors.allow_check_six",
        "behaviors.allow_grab_items",
        "behaviors.allow_grab_power_items",
        "behaviors.defer_power_items_to_humans",
        "behaviors.min_respawn_time",
        "behaviors.max_respawn_time",
        "movement.allow_jumping_in_combat",
        "movement.jump_chance",
        "movement.jump_cooldown",
        "movement.walk_only",
        "senses.sight_time",
        "senses.sight_decay_time",
        "senses.invis_enemy_sight_scalar",
        "senses.max_invis_enemy_sight_dist",
        "senses.fov_angle",
        "senses.forget_non_vis_enemy_time",
        "senses.sound_range",
        "senses.sound_time",
        "senses.sound_decay_time",
        "senses.sound_persist_time",
        "weapons.decay_time",
        "weapons.fov_angle",
        "weapons.sight_time",
    ];
    for field in &block.fields {
        if !KNOWN.contains(&field.key.as_str()) {
            settings.unknown.insert(field.key.clone(), field.values.clone());
            errors.push(format!("line {}: unknown settings key \"{}\"", field.line, field.key));
        }
    }
}

/// Danger entry (Q2 rerelease danger family).
#[derive(Debug, Clone)]
pub struct DangerEntry {
    /// Base entry.
    pub base: BotSourceEntry,
    /// Name.
    pub name: String,
    /// Sight distance.
    pub sight_dist: f64,
    /// Flags.
    pub flags: Vec<String>,
}

impl Default for DangerEntry {
    fn default() -> Self {
        Self {
            base: BotSourceEntry::default(),
            name: String::new(),
            sight_dist: f64::from(f32::MAX),
            flags: Vec::new(),
        }
    }
}

/// Parse dangers.
#[must_use]
pub fn parse_dangers(text: &str) -> (Vec<DangerEntry>, Vec<String>) {
    let parsed = parse_blocks(text);
    let mut errors: Vec<String> = parsed
        .errors
        .iter()
        .map(|error| format!("line {}: {}", error.line, error.message))
        .collect();
    let mut entries = Vec::new();
    for block in &parsed.blocks {
        let mut entry = DangerEntry::default();
        entry.base.source = Some(block.clone());
        for field in &block.fields {
            match field.key.as_str() {
                "name" => {
                    if let Some(value) = expect_string(&mut errors, &field.key, &field.values, field.line) {
                        entry.name = value;
                    }
                }
                "sight_dist" => {
                    if let Some(value) = expect_number(&mut errors, &field.key, &field.values, field.line) {
                        entry.sight_dist = value;
                    }
                }
                "flags" => entry.flags = field_flag_list(&field.values),
                _ => {
                    entry.base.unknown.push(field.clone());
                    unknown_key(&mut errors, &field.key, field.line);
                }
            }
        }
        entries.push(entry);
    }
    (entries, errors)
}
