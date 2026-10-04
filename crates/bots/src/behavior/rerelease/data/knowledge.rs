//! Rerelease knowledge from `src/bots/behavior/rerelease/data/knowledge.ts`.
//!
//! Every `bots/*.txt` file indexed and shared by all bots: weapons
//! with compiled flag tests, items with mega-spawnflag tests,
//! monsters, interactables, game rules, teams, chats, skills, and
//! dangers. Selection follows each shipped file's comments:
//! `choose_weapon` runs weapons.txt's five-step rule, `item_value`
//! scores pickups by need, and `game_mode` resolves game_rules.txt.

use std::collections::HashMap;

use crate::behavior::assets::BotSourceFiles;
use crate::behavior::rerelease::data::botdata::{
    parse_bot_settings, parse_characters, parse_chats, parse_dangers, parse_game_rules, parse_interactables,
    parse_items, parse_monsters, parse_teams, parse_weapons, BotDataFormat, BotSkillSettings, BotSourceEntry,
    BotWeaponIdentity, CharacterEntry, ChatEntry, DangerEntry, GameRuleEntry, InteractableEntry, ItemEntry,
    MonsterEntry, TeamEntry, WeaponEntry,
};
use crate::behavior::rerelease::data::source_files::read_bot_source_text;

/// Item flags.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ItemFlag;

impl ItemFlag {
    /// Health.
    pub const HEALTH: &'static str = "health";
    /// Ammo.
    pub const AMMO: &'static str = "ammo";
    /// Armor.
    pub const ARMOR: &'static str = "armor";
    /// Weapon.
    pub const WEAPON: &'static str = "weapon";
    /// Powerup.
    pub const POWERUP: &'static str = "powerup";
    /// Dropped.
    pub const DROPPED: &'static str = "dropped";
    /// Mega item.
    pub const MEGA_ITEM: &'static str = "mega_item";
    /// Backpack.
    pub const BACKPACK: &'static str = "backpack";
    /// Zap proof.
    pub const ZAP_PROOF: &'static str = "zap_proof";
    /// Check items.
    pub const CHECK_ITEMS: &'static str = "check_items";
    /// Objective.
    pub const OBJECTIVE: &'static str = "objective";
    /// Rune.
    pub const RUNE: &'static str = "rune";
    /// Key.
    pub const KEY: &'static str = "key";
    /// Usable.
    pub const USABLE: &'static str = "usable";
    /// Ignore limits.
    pub const IGNORE_LIMITS: &'static str = "ignore_limits";
}

/// Weapon flags.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct WeaponFlag;

impl WeaponFlag {
    /// Melee.
    pub const MELEE: &'static str = "melee";
    /// Hitscan.
    pub const HITSCAN: &'static str = "hitscan";
    /// Projectile.
    pub const PROJECTILE: &'static str = "projectile";
    /// Parabolic.
    pub const PARABOLIC: &'static str = "parabolic";
    /// Explosive.
    pub const EXPLOSIVE: &'static str = "explosive";
    /// Sticky.
    pub const STICKY: &'static str = "sticky";
    /// Electric.
    pub const ELECTRIC: &'static str = "electric";
    /// Starting.
    pub const STARTING: &'static str = "starting";
    /// Initial.
    pub const INITIAL: &'static str = "initial";
    /// Suitable.
    pub const SUITABLE: &'static str = "suitable";
    /// Rocket jump.
    pub const ROCKET_JUMP: &'static str = "rocket_jump";
}

/// Monster flags.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MonsterFlag;

impl MonsterFlag {
    /// Melee.
    pub const MELEE: &'static str = "melee";
    /// Ranged.
    pub const RANGED: &'static str = "ranged";
    /// Aquatic.
    pub const AQUATIC: &'static str = "aquatic";
    /// Flyer.
    pub const FLYER: &'static str = "flyer";
    /// Tank.
    pub const TANK: &'static str = "tank";
    /// Sponge.
    pub const SPONGE: &'static str = "sponge";
    /// Turret.
    pub const TURRET: &'static str = "turret";
    /// Neutral.
    pub const NEUTRAL: &'static str = "neutral";
}

/// Danger flags.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DangerFlag;

impl DangerFlag {
    /// Takes damage.
    pub const TAKES_DAMAGE: &'static str = "takes_damage";
    /// Ignore if team.
    pub const IGNORE_IF_TEAM: &'static str = "ignore_if_team";
    /// Ignore if coop.
    pub const IGNORE_IF_COOP: &'static str = "ignore_if_coop";
    /// Ignore if no team damage.
    pub const IGNORE_IF_NO_TEAM_DMG: &'static str = "ignore_if_no_team_dmg";
    /// Timer based.
    pub const TIMER_BASED: &'static str = "timer_based";
    /// Nav hazard.
    pub const NAV_HAZARD: &'static str = "nav_hazard";
    /// Check velocity.
    pub const CHECK_VELOCITY: &'static str = "check_velocity";
    /// Check beam.
    pub const CHECK_BEAM: &'static str = "check_beam";
    /// Exploding barrel.
    pub const EXPLODING_BARREL: &'static str = "exploding_barrel";
    /// Update PVS.
    pub const UPDATE_PVS: &'static str = "update_pvs";
}

/// Interactions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Interaction;

impl Interaction {
    /// Push.
    pub const PUSH: &'static str = "push";
    /// Shoot.
    pub const SHOOT: &'static str = "shoot";
    /// Use.
    pub const USE: &'static str = "use";
    /// Ride.
    pub const RIDE: &'static str = "ride";
    /// Touch.
    pub const TOUCH: &'static str = "touch";
}

/// Source data files.
#[derive(Debug, Clone, Default)]
pub struct BotDataFilesT {
    /// characters.txt.
    pub characters: String,
    /// weapons.txt.
    pub weapons: String,
    /// items.txt.
    pub items: String,
    /// monsters.txt.
    pub monsters: String,
    /// interactables.txt.
    pub interactables: String,
    /// game_rules.txt.
    pub game_rules: String,
    /// teams.txt.
    pub teams: String,
    /// chats.txt.
    pub chats: String,
    /// settings file.
    pub settings: String,
    /// dangers.txt.
    pub dangers: Option<String>,
}

impl BotDataFilesT {
    /// Assemble the shared `bots/*.txt` payload around already-resolved
    /// weapons and settings text. Settings resolution (Q1 platform fallback
    /// versus Q2 explicit platform) stays with the callers; the remaining
    /// eight files read in the same order for both.
    #[must_use]
    pub fn from_source_files(files: &dyn BotSourceFiles, weapons: String, settings: String) -> Self {
        Self {
            weapons,
            settings,
            characters: read_bot_source_text(files, "bots/characters.txt").unwrap_or_default(),
            items: read_bot_source_text(files, "bots/items.txt").unwrap_or_default(),
            monsters: read_bot_source_text(files, "bots/monsters.txt").unwrap_or_default(),
            interactables: read_bot_source_text(files, "bots/interactables.txt").unwrap_or_default(),
            game_rules: read_bot_source_text(files, "bots/game_rules.txt").unwrap_or_default(),
            teams: read_bot_source_text(files, "bots/teams.txt").unwrap_or_default(),
            chats: read_bot_source_text(files, "bots/chats.txt").unwrap_or_default(),
            dangers: read_bot_source_text(files, "bots/dangers.txt"),
        }
    }
}

/// Compiled weapon with hoisted flag tests.
#[derive(Debug, Clone)]
pub struct BotWeaponT {
    /// Entry.
    pub entry: WeaponEntry,
    /// Melee.
    pub is_melee: bool,
    /// Electric.
    pub is_electric: bool,
    /// Needs ammo.
    pub needs_ammo: bool,
}

/// Compiled item with hoisted flag tests.
#[derive(Debug, Clone)]
pub struct BotItemT {
    /// Entry.
    pub entry: ItemEntry,
    /// Health.
    pub is_health: bool,
    /// Armor.
    pub is_armor: bool,
    /// Ammo.
    pub is_ammo: bool,
    /// Weapon.
    pub is_weapon: bool,
    /// Powerup.
    pub is_powerup: bool,
    /// Unconditional mega item.
    pub is_mega: bool,
    /// Spawnflag bit making this entity a mega item.
    pub mega_spawnflag: i32,
}

/// Game types.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BotGameType;

impl BotGameType {
    /// Deathmatch.
    pub const DEATHMATCH: &'static str = "deathmatch";
    /// Team deathmatch.
    pub const TEAM_DEATHMATCH: &'static str = "tdm";
    /// Coop.
    pub const COOP: &'static str = "coop";
    /// Horde.
    pub const HORDE: &'static str = "horde";
    /// CTF.
    pub const CTF: &'static str = "ctf";
}

/// Game mode.
#[derive(Debug, Clone, PartialEq)]
pub struct BotGameModeT {
    /// Game type.
    pub game_type: String,
    /// Weapon stay.
    pub weapon_stay: bool,
    /// Has teams.
    pub has_teams: Option<bool>,
    /// Team damage.
    pub team_damage: Option<bool>,
}

/// Bot knowledge: every data file indexed.
#[derive(Debug, Clone, Default)]
pub struct BotKnowledge {
    /// Format.
    pub format: BotDataFormat,
    /// Dangers.
    pub dangers: Vec<DangerEntry>,
    /// Characters.
    pub characters: Vec<CharacterEntry>,
    /// Weapons.
    pub weapons: Vec<BotWeaponT>,
    /// Items.
    pub items: Vec<BotItemT>,
    /// Monsters.
    pub monsters: Vec<MonsterEntry>,
    /// Interactables.
    pub interactables: Vec<InteractableEntry>,
    /// Game rules.
    pub game_rules: Vec<GameRuleEntry>,
    /// Teams.
    pub teams: Vec<TeamEntry>,
    /// Chats.
    pub chats: Vec<ChatEntry>,
    /// Skills.
    pub skills: Vec<BotSkillSettings>,
    /// Parse errors across all files.
    pub errors: Vec<String>,
    items_by_name: HashMap<String, usize>,
    monsters_by_class: HashMap<String, usize>,
    skills_by_name: HashMap<String, usize>,
    weapons_by_name: HashMap<String, usize>,
    weapons_by_number: HashMap<i32, usize>,
    dangers_by_name: HashMap<String, usize>,
}

/// Weapon-number resolver for knowledge builds.
type WeaponNumber<'a> = Option<&'a dyn Fn(&str) -> Option<i32>>;

impl BotKnowledge {
    /// Build knowledge from source files.
    #[must_use]
    pub fn new(files: &BotDataFilesT, format: BotDataFormat, weapon_number: WeaponNumber<'_>) -> Self {
        let mut knowledge = Self {
            format,
            ..Self::default()
        };
        let (characters, errors) = parse_characters(&files.characters);
        knowledge.take("characters.txt", &errors);
        knowledge.characters = characters;
        let (weapons, errors) = parse_weapons(&files.weapons, format);
        knowledge.take("weapons.txt", &errors);
        for mut entry in weapons {
            if matches!(entry.identity, BotWeaponIdentity::Q2Classname { .. }) {
                let classname = match &entry.identity {
                    BotWeaponIdentity::Q2Classname { classname } => classname.clone(),
                    _ => String::new(),
                };
                let bit = weapon_number.and_then(|lookup| lookup(&classname));
                if bit.is_none() {
                    knowledge.errors.push(format!(
                        "weapons.txt: no inventory binding for Q2 classname \"{classname}\""
                    ));
                }
                entry.number = bit.unwrap_or(0);
            }
            let flags: std::collections::HashSet<&str> = entry.flags.iter().map(String::as_str).collect();
            let compiled = BotWeaponT {
                is_melee: flags.contains(WeaponFlag::MELEE),
                is_electric: flags.contains(WeaponFlag::ELECTRIC),
                needs_ammo: if matches!(entry.identity, BotWeaponIdentity::Q2Classname { .. }) {
                    !entry.ammo_name.is_empty() && entry.ammo_name != "none"
                } else {
                    !entry.ammo.is_empty() && entry.ammo != "none"
                },
                entry,
            };
            if compiled.entry.number != 0 {
                knowledge
                    .weapons_by_number
                    .insert(compiled.entry.number, knowledge.weapons.len());
            }
            knowledge
                .weapons_by_name
                .insert(compiled.entry.name.clone(), knowledge.weapons.len());
            knowledge.weapons.push(compiled);
        }
        let (items, errors) = parse_items(&files.items);
        knowledge.take("items.txt", &errors);
        for entry in items {
            let flags: std::collections::HashSet<&str> = entry.flags.iter().map(String::as_str).collect();
            let compiled = BotItemT {
                is_health: flags.contains(ItemFlag::HEALTH),
                is_armor: flags.contains(ItemFlag::ARMOR),
                is_ammo: flags.contains(ItemFlag::AMMO),
                is_weapon: flags.contains(ItemFlag::WEAPON),
                is_powerup: flags.contains(ItemFlag::POWERUP),
                is_mega: flags.contains(ItemFlag::MEGA_ITEM),
                mega_spawnflag: entry
                    .spawnflags
                    .iter()
                    .find(|flag| flag.name == ItemFlag::MEGA_ITEM)
                    .map_or(0, |flag| flag.bit),
                entry,
            };
            knowledge
                .items_by_name
                .insert(compiled.entry.name.clone(), knowledge.items.len());
            knowledge.items.push(compiled);
        }
        let (dangers, errors) = parse_dangers(files.dangers.as_deref().unwrap_or(""));
        knowledge.take("dangers.txt", &errors);
        for (index, danger) in dangers.iter().enumerate() {
            knowledge.dangers_by_name.insert(danger.name.clone(), index);
        }
        knowledge.dangers = dangers;
        let (monsters, errors) = parse_monsters(&files.monsters);
        knowledge.take("monsters.txt", &errors);
        for (index, monster) in monsters.iter().enumerate() {
            knowledge.monsters_by_class.insert(monster.classname.clone(), index);
        }
        knowledge.monsters = monsters;
        let (interactables, errors) = parse_interactables(&files.interactables);
        knowledge.take("interactables.txt", &errors);
        knowledge.interactables = interactables;
        let (rules, errors) = parse_game_rules(&files.game_rules);
        knowledge.take("game_rules.txt", &errors);
        knowledge.game_rules = rules;
        let (teams, errors) = parse_teams(&files.teams);
        knowledge.take("teams.txt", &errors);
        knowledge.teams = teams;
        let (chats, errors) = parse_chats(&files.chats);
        knowledge.take("chats.txt", &errors);
        knowledge.chats = chats;
        let (skills, errors) = parse_bot_settings(&files.settings);
        knowledge.take("settings.txt", &errors);
        for (index, skill) in skills.iter().enumerate() {
            knowledge.skills_by_name.insert(skill.skill.clone(), index);
        }
        knowledge.skills = skills;
        knowledge
    }

    fn take(&mut self, label: &str, errors: &[String]) {
        for error in errors {
            self.errors.push(format!("{label}: {error}"));
        }
    }

    /// Replace the weapon table.
    pub fn replace_weapons(&mut self, weapons: Vec<BotWeaponT>) {
        self.weapons_by_number.clear();
        self.weapons_by_name.clear();
        for (index, weapon) in weapons.iter().enumerate() {
            if weapon.entry.number != 0 {
                self.weapons_by_number.insert(weapon.entry.number, index);
            }
            self.weapons_by_name.insert(weapon.entry.name.clone(), index);
        }
        self.weapons = weapons;
    }

    /// Skill by name.
    #[must_use]
    pub fn skill(&self, name: &str) -> Option<&BotSkillSettings> {
        self.skills_by_name.get(name).and_then(|index| self.skills.get(*index))
    }

    /// Skill names in file order.
    #[must_use]
    pub fn skill_names(&self) -> Vec<&str> {
        self.skills.iter().map(|skill| skill.skill.as_str()).collect()
    }

    /// Item by classname.
    #[must_use]
    pub fn item(&self, classname: &str) -> Option<&BotItemT> {
        self.items_by_name
            .get(classname)
            .and_then(|index| self.items.get(*index))
    }

    /// Monster by classname.
    #[must_use]
    pub fn monster(&self, classname: &str) -> Option<&MonsterEntry> {
        self.monsters_by_class
            .get(classname)
            .and_then(|index| self.monsters.get(*index))
    }

    /// Weapon by name.
    #[must_use]
    pub fn weapon_by_name(&self, name: &str) -> Option<&BotWeaponT> {
        self.weapons_by_name
            .get(name)
            .and_then(|index| self.weapons.get(*index))
    }

    /// Weapon by number.
    #[must_use]
    pub fn weapon_by_number(&self, number: i32) -> Option<&BotWeaponT> {
        self.weapons_by_number
            .get(&number)
            .and_then(|index| self.weapons.get(*index))
    }

    /// Danger by classname.
    #[must_use]
    pub fn danger(&self, classname: &str) -> Option<&DangerEntry> {
        self.dangers_by_name
            .get(classname)
            .and_then(|index| self.dangers.get(*index))
    }

    /// Character by name (case-insensitive).
    #[must_use]
    pub fn character(&self, name: &str) -> Option<&CharacterEntry> {
        self.characters
            .iter()
            .find(|character| character.name.eq_ignore_ascii_case(name))
    }

    /// teams.txt translation of a QuakeC team value to a 0-based index.
    #[must_use]
    pub fn team_index(&self, quakec_team_value: f64) -> i32 {
        let cap = if self.format == BotDataFormat::Q2 { 2 } else { 4 };
        for (index, team) in self.teams.iter().enumerate().take(cap) {
            if team.value == quakec_team_value {
                return index as i32;
            }
        }
        -1
    }

    /// Chats of a type.
    #[must_use]
    pub fn chats_of_type(&self, chat_type: &str) -> Vec<&ChatEntry> {
        self.chats.iter().filter(|chat| chat.chat_type == chat_type).collect()
    }

    /// Resolve the game mode from cvar values.
    #[must_use]
    pub fn game_mode(&self, cvar_value: &dyn Fn(&str) -> f64) -> BotGameModeT {
        if self.format == BotDataFormat::Q2 {
            let mut mode = BotGameModeT {
                game_type: "dm".to_owned(),
                weapon_stay: false,
                has_teams: Some(false),
                team_damage: Some(false),
            };
            let mut assigned = std::collections::HashSet::new();
            for rule in &self.game_rules {
                if let Some(only) = &rule.only_game_type {
                    if mode.game_type != *only {
                        continue;
                    }
                }
                if let Some(not) = &rule.not_game_type {
                    if mode.game_type == *not {
                        continue;
                    }
                }
                let matches: Vec<bool> = rule
                    .conditions
                    .iter()
                    .map(|condition| cvar_value(&condition.cvar) == condition.value)
                    .collect();
                if matches.is_empty()
                    || !(if rule.logic_op == "and" {
                        matches.iter().all(|value| *value)
                    } else {
                        matches.iter().any(|value| *value)
                    })
                {
                    continue;
                }
                if !rule.game_type.is_empty() && !assigned.contains("gameType") {
                    mode.game_type = rule.game_type.clone();
                    assigned.insert("gameType");
                }
                if rule.weapon_stay_specified && !assigned.contains("weaponStay") {
                    mode.weapon_stay = rule.weapon_stay;
                    assigned.insert("weaponStay");
                }
                if rule.has_teams.is_some() && !assigned.contains("hasTeams") {
                    mode.has_teams = rule.has_teams;
                    assigned.insert("hasTeams");
                }
                if rule.team_damage.is_some() && !assigned.contains("teamDamage") {
                    mode.team_damage = rule.team_damage;
                    assigned.insert("teamDamage");
                }
            }
            return mode;
        }
        for rule in &self.game_rules {
            if rule.cvar.is_empty() || cvar_value(&rule.cvar) != rule.value {
                continue;
            }
            let mut game_type = rule.game_type.clone();
            if game_type == BotGameType::DEATHMATCH && cvar_value("teamplay") == 1.0 {
                game_type = BotGameType::TEAM_DEATHMATCH.to_owned();
            }
            return BotGameModeT {
                game_type,
                weapon_stay: rule.weapon_stay,
                has_teams: None,
                team_damage: None,
            };
        }
        BotGameModeT {
            game_type: BotGameType::DEATHMATCH.to_owned(),
            weapon_stay: false,
            has_teams: None,
            team_damage: None,
        }
    }

    /// Interaction for an entity per interactables.txt.
    #[must_use]
    pub fn interaction_for(
        &self,
        classname: &str,
        spawnflags: i32,
        has_health: bool,
        has_targetname: bool,
    ) -> Option<&str> {
        for entry in self.interactables.iter().filter(|entry| entry.name == classname) {
            let mut conditions = Vec::new();
            if let Some(mask) = entry.spawnflags {
                conditions.push((f64::from(spawnflags) as i64 & mask as i64) == mask as i64);
            }
            if let Some(health) = entry.health {
                conditions.push(health == has_health);
            }
            if let Some(targetname) = entry.targetname {
                conditions.push(targetname == has_targetname);
            }
            if conditions.is_empty() {
                return Some(&entry.interaction);
            }
            let ok = if entry.logic_op.as_deref() == Some("or") {
                conditions.iter().any(|value| *value)
            } else {
                conditions.iter().all(|value| *value)
            };
            if ok {
                return Some(&entry.interaction);
            }
        }
        None
    }

    /// Look up a source entry (tests).
    #[must_use]
    pub fn base_entry(&self) -> BotSourceEntry {
        BotSourceEntry::default()
    }
}

/// Weapon pick.
#[derive(Debug, Clone, PartialEq)]
pub struct BotWeaponPickT {
    /// Weapon index.
    pub index: usize,
    /// Selection number.
    pub number: i32,
}

/// Weapon selection context.
#[derive(Debug, Clone, PartialEq)]
pub struct BotWeaponContextT {
    /// Items bitmask.
    pub items: i32,
    /// Ammo by name.
    pub ammo: HashMap<String, i32>,
    /// Range to target.
    pub range: f32,
    /// Height delta (target - bot).
    pub height_delta: f32,
    /// Bot in water.
    pub in_water: bool,
    /// Bot protected.
    pub has_protection: bool,
    /// Target in water.
    pub target_in_water: bool,
    /// Allow melee.
    pub allow_melee: bool,
}

/// weapons.txt's five-step rule. Returns `None` when nothing in
/// inventory is valid (keep what is in hand).
#[must_use]
pub fn choose_weapon(weapons: &[BotWeaponT], ctx: &BotWeaponContextT) -> Option<BotWeaponPickT> {
    let mut best: Option<usize> = None;
    for (index, weapon) in weapons.iter().enumerate() {
        let entry = &weapon.entry;
        if entry.priority <= 0.0 {
            continue;
        }
        if weapon.is_melee && !ctx.allow_melee {
            continue;
        }
        if ctx.items & entry.number == 0 {
            continue;
        }
        if weapon.needs_ammo {
            let have = ctx.ammo.get(&entry.ammo_name).copied().unwrap_or(0) as f64;
            if have < entry.min_ammo.max(1.0) {
                continue;
            }
        }
        if f64::from(ctx.range) < entry.min_range {
            continue;
        }
        if entry.max_range > 0.0 && f64::from(ctx.range) > entry.max_range {
            continue;
        }
        if entry.max_height > 0.0 && f64::from(ctx.height_delta) > entry.max_height {
            continue;
        }
        if entry.min_height > 0.0 && f64::from(-ctx.height_delta) > entry.min_height {
            continue;
        }
        if weapon.is_electric && ctx.in_water && !(ctx.has_protection && ctx.target_in_water) {
            continue;
        }
        if best.is_none_or(|best_index| entry.priority > weapons[best_index].entry.priority) {
            best = Some(index);
        }
    }
    best.map(|index| BotWeaponPickT {
        index,
        number: weapons[index].entry.number,
    })
}

/// Item scoring context.
#[derive(Debug, Clone, PartialEq)]
pub struct BotItemContextT {
    /// Pickup spawnflags.
    pub spawnflags: i32,
    /// Health.
    pub health: f32,
    /// Maximum health.
    pub max_health: f32,
    /// Armor.
    pub armor: f32,
    /// Items bitmask.
    pub items: i32,
    /// Ammo by name.
    pub ammo: HashMap<String, i32>,
    /// Weapon stay.
    pub weapon_stay: bool,
    /// Allow power items.
    pub allow_power_items: bool,
    /// Bot team.
    pub team: i32,
    /// Item team.
    pub item_team: i32,
    /// Objective at home.
    pub objective_at_home: bool,
}

/// Item value in arbitrary units; `<= 0` means walk past.
#[must_use]
pub fn item_value(item: &BotItemT, ctx: &BotItemContextT, weapons: &[BotWeaponT]) -> f32 {
    let mega =
        item.is_mega || (item.mega_spawnflag != 0 && ctx.spawnflags & item.mega_spawnflag == item.mega_spawnflag);
    if item.is_powerup || mega {
        if !ctx.allow_power_items {
            return 0.0;
        }
        return if item.is_powerup { 1000.0 } else { 700.0 };
    }
    if item.is_weapon {
        let weapon = weapon_for_item(weapons, &item.entry.name);
        let number = weapon.map_or(0, |weapon| weapons[weapon].entry.number);
        let owned = number != 0 && ctx.items & number != 0;
        if owned && !ctx.weapon_stay {
            return 200.0;
        }
        if owned {
            return 0.0;
        }
        return 600.0 + weapon.map_or(0.0, |weapon| weapons[weapon].entry.priority as f32) * 10.0;
    }
    if item.is_health {
        let missing = (ctx.max_health - ctx.health).max(0.0);
        if missing <= 0.0 {
            return 0.0;
        }
        return 100.0 + missing * 4.0;
    }
    if item.is_armor {
        if ctx.armor >= 200.0 {
            return 0.0;
        }
        return 300.0 + (200.0 - ctx.armor);
    }
    if item.is_ammo {
        let mut want: f32 = 0.0;
        for weapon in weapons {
            if !weapon.needs_ammo {
                continue;
            }
            let entry = &weapon.entry;
            let relevant = if matches!(entry.identity, BotWeaponIdentity::Q2Classname { .. }) {
                item.entry.name == entry.ammo_name
            } else {
                item.entry.flags.iter().any(|flag| flag == &entry.ammo)
            };
            if !relevant {
                continue;
            }
            let have = ctx.ammo.get(&entry.ammo_name).copied().unwrap_or(0) as f64;
            let room = (entry.max_ammo - have).max(0.0);
            let owned = ctx.items & entry.number != 0;
            let score = (if owned { 150.0 } else { 40.0 })
                * if entry.max_ammo > 0.0 {
                    room / entry.max_ammo
                } else {
                    0.0
                };
            want = want.max(score as f32);
        }
        return want;
    }
    if item.entry.flags.iter().any(|flag| flag == ItemFlag::BACKPACK) {
        return 120.0;
    }
    if item.entry.flags.iter().any(|flag| flag == ItemFlag::OBJECTIVE) {
        if ctx.item_team <= 0 || ctx.team <= 0 {
            return 900.0;
        }
        if ctx.item_team != ctx.team {
            return 900.0;
        }
        return if ctx.objective_at_home { 0.0 } else { 950.0 };
    }
    if item.entry.flags.iter().any(|flag| flag == ItemFlag::RUNE) {
        return 900.0;
    }
    0.0
}

/// weapons.txt entry granted by a `weapon_*` pickup: longest
/// prefix-compatible normalized name wins.
#[must_use]
pub fn weapon_for_item(weapons: &[BotWeaponT], item_name: &str) -> Option<usize> {
    if let Some(index) = weapons.iter().position(|weapon| weapon.entry.name == item_name) {
        return Some(index);
    }
    let key: String = item_name
        .strip_prefix("weapon_")
        .unwrap_or(item_name)
        .chars()
        .filter(|c| *c != '_')
        .collect();
    let mut best: Option<usize> = None;
    for (index, weapon) in weapons.iter().enumerate() {
        let entry = &weapon.entry;
        if matches!(entry.identity, BotWeaponIdentity::Q2Classname { .. }) || entry.name.is_empty() {
            continue;
        }
        let name: String = entry.name.chars().filter(|c| *c != '_').collect();
        if !key.starts_with(&name) && !name.starts_with(&key) {
            continue;
        }
        let longer = best.is_none_or(|best_index| {
            let best_name: String = weapons[best_index].entry.name.chars().filter(|c| *c != '_').collect();
            name.len() > best_name.len()
        });
        if longer {
            best = Some(index);
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::collections::HashMap;

    use super::*;

    struct RecordingFiles {
        files: HashMap<String, Vec<u8>>,
        reads: RefCell<Vec<String>>,
    }

    impl BotSourceFiles for RecordingFiles {
        fn read(&self, path: &str) -> Option<Vec<u8>> {
            self.reads.borrow_mut().push(path.to_owned());
            self.files.get(path).cloned()
        }

        fn list(&self, _directory: &str, _extension: &str) -> Vec<String> {
            Vec::new()
        }
    }

    #[test]
    fn shared_payload_matches_inline_assembly() {
        let mut files = HashMap::new();
        files.insert("bots/characters.txt".to_owned(), b"chars".to_vec());
        files.insert("bots/dangers.txt".to_owned(), b"danger".to_vec());
        let sources = RecordingFiles {
            files,
            reads: RefCell::new(Vec::new()),
        };
        let payload = BotDataFilesT::from_source_files(&sources, "weapons".to_owned(), "settings".to_owned());
        assert_eq!(payload.weapons, "weapons");
        assert_eq!(payload.settings, "settings");
        assert_eq!(payload.characters, "chars");
        assert_eq!(payload.items, String::new());
        assert_eq!(payload.monsters, String::new());
        assert_eq!(payload.interactables, String::new());
        assert_eq!(payload.game_rules, String::new());
        assert_eq!(payload.teams, String::new());
        assert_eq!(payload.chats, String::new());
        assert_eq!(payload.dangers, Some("danger".to_owned()));
        // Missing text files default; only dangers stays optional, and the
        // eight reads keep the previous inline order.
        assert_eq!(
            *sources.reads.borrow(),
            [
                "bots/characters.txt",
                "bots/items.txt",
                "bots/monsters.txt",
                "bots/interactables.txt",
                "bots/game_rules.txt",
                "bots/teams.txt",
                "bots/chats.txt",
                "bots/dangers.txt",
            ]
            .into_iter()
            .map(str::to_owned)
            .collect::<Vec<_>>()
        );
    }

    fn recording(pairs: &[(&str, &str)]) -> RecordingFiles {
        RecordingFiles {
            files: pairs
                .iter()
                .map(|(path, text)| ((*path).to_owned(), text.as_bytes().to_vec()))
                .collect(),
            reads: RefCell::new(Vec::new()),
        }
    }

    #[test]
    fn q1_loader_keeps_absence_and_fallback() {
        use crate::behavior::rerelease::data::knowledge_q1::load_quake1_knowledge;
        // Unmounted weapons stay `None` without further reads.
        let empty = recording(&[]);
        assert!(load_quake1_knowledge(&empty).expect("ok").is_none());
        assert_eq!(*empty.reads.borrow(), vec!["bots/weapons.txt".to_owned()]);
        // Settings resolve PC, then Consoles, then Nintendo.
        let consoles = recording(&[("bots/weapons.txt", ""), ("bots/settings_Consoles.txt", "")]);
        let knowledge = load_quake1_knowledge(&consoles).expect("ok").expect("knowledge");
        assert_eq!(knowledge.format, BotDataFormat::Q1);
        assert_eq!(
            consoles.reads.borrow()[..3],
            [
                "bots/weapons.txt".to_owned(),
                "bots/settings_PC.txt".to_owned(),
                "bots/settings_Consoles.txt".to_owned(),
            ]
        );
        // Mounted weapons without any settings stay an error.
        let bare = recording(&[("bots/weapons.txt", "")]);
        assert!(load_quake1_knowledge(&bare).is_err());
    }

    #[test]
    fn q2_loader_keeps_platform_policy() {
        use crate::behavior::rerelease::data::knowledge_q2::{bot_load_knowledge, SettingsPlatform};
        let full = recording(&[("bots/weapons.txt", ""), ("bots/settings_PC.txt", "")]);
        let knowledge = bot_load_knowledge(&full, SettingsPlatform::Pc).expect("knowledge");
        assert_eq!(knowledge.format, BotDataFormat::Q2);
        assert_eq!(
            full.reads.borrow()[..2],
            ["bots/weapons.txt".to_owned(), "bots/settings_PC.txt".to_owned()]
        );
        let missing = recording(&[("bots/weapons.txt", "")]);
        let error = bot_load_knowledge(&missing, SettingsPlatform::Pc).expect_err("missing settings");
        assert!(error.to_string().contains("settings_PC.txt"), "{error}");
    }
}
