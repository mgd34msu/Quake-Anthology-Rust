//! Q2 knowledge loader from `src/bots/behavior/rerelease/data/knowledge-q2.ts`.
//!
//! Q2 rerelease knowledge comes from mounted `bots/*.txt`, including
//! `dangers.txt`. The inventory-bit adapter maps KEX classnames to
//! synthetic `items` mask bits; these bits are not Q2 item IDs.

use crate::behavior::assets::BotSourceFiles;
use crate::behavior::rerelease::data::botdata::BotDataFormat;
use crate::behavior::rerelease::data::knowledge::{BotDataFilesT, BotGameModeT, BotKnowledge};
use crate::behavior::rerelease::data::source_files::read_bot_source_text;
use crate::error::BotsError;

/// Weapon binding: adapter label, synthetic bit, KEX classnames.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BotWeaponBindingT {
    /// Adapter label.
    pub name: &'static str,
    /// Synthetic items-mask bit.
    pub bit: i32,
    /// KEX item classname.
    pub classname: &'static str,
    /// KEX ammo classname, or empty.
    pub ammo_classname: &'static str,
}

/// Q2 weapon bindings.
pub const BOT_WEAPON_BINDINGS: &[BotWeaponBindingT] = &[
    BotWeaponBindingT {
        name: "blaster",
        bit: 1 << 0,
        classname: "weapon_blaster",
        ammo_classname: "",
    },
    BotWeaponBindingT {
        name: "chainfist",
        bit: 1 << 1,
        classname: "weapon_chainfist",
        ammo_classname: "",
    },
    BotWeaponBindingT {
        name: "shotgun",
        bit: 1 << 2,
        classname: "weapon_shotgun",
        ammo_classname: "ammo_shells",
    },
    BotWeaponBindingT {
        name: "super_shotgun",
        bit: 1 << 3,
        classname: "weapon_supershotgun",
        ammo_classname: "ammo_shells",
    },
    BotWeaponBindingT {
        name: "machinegun",
        bit: 1 << 4,
        classname: "weapon_machinegun",
        ammo_classname: "ammo_bullets",
    },
    BotWeaponBindingT {
        name: "chaingun",
        bit: 1 << 5,
        classname: "weapon_chaingun",
        ammo_classname: "ammo_bullets",
    },
    BotWeaponBindingT {
        name: "etf_rifle",
        bit: 1 << 6,
        classname: "weapon_etf_rifle",
        ammo_classname: "ammo_flechettes",
    },
    BotWeaponBindingT {
        name: "grenades",
        bit: 1 << 7,
        classname: "ammo_grenades",
        ammo_classname: "ammo_grenades",
    },
    BotWeaponBindingT {
        name: "grenade_launcher",
        bit: 1 << 8,
        classname: "weapon_grenadelauncher",
        ammo_classname: "ammo_grenades",
    },
    BotWeaponBindingT {
        name: "prox_launcher",
        bit: 1 << 9,
        classname: "weapon_proxlauncher",
        ammo_classname: "ammo_prox",
    },
    BotWeaponBindingT {
        name: "rocket_launcher",
        bit: 1 << 10,
        classname: "weapon_rocketlauncher",
        ammo_classname: "ammo_rockets",
    },
    BotWeaponBindingT {
        name: "hyperblaster",
        bit: 1 << 11,
        classname: "weapon_hyperblaster",
        ammo_classname: "ammo_cells",
    },
    BotWeaponBindingT {
        name: "boomer",
        bit: 1 << 12,
        classname: "weapon_boomer",
        ammo_classname: "ammo_cells",
    },
    BotWeaponBindingT {
        name: "plasmabeam",
        bit: 1 << 13,
        classname: "weapon_plasmabeam",
        ammo_classname: "ammo_cells",
    },
    BotWeaponBindingT {
        name: "railgun",
        bit: 1 << 14,
        classname: "weapon_railgun",
        ammo_classname: "ammo_slugs",
    },
    BotWeaponBindingT {
        name: "phalanx",
        bit: 1 << 15,
        classname: "weapon_phalanx",
        ammo_classname: "ammo_magslug",
    },
    BotWeaponBindingT {
        name: "bfg",
        bit: 1 << 16,
        classname: "weapon_bfg",
        ammo_classname: "ammo_cells",
    },
    BotWeaponBindingT {
        name: "disintegrator",
        bit: 1 << 17,
        classname: "weapon_disintegrator",
        ammo_classname: "ammo_disruptor",
    },
    BotWeaponBindingT {
        name: "tesla",
        bit: 1 << 18,
        classname: "ammo_tesla",
        ammo_classname: "ammo_tesla",
    },
    BotWeaponBindingT {
        name: "trap",
        bit: 1 << 19,
        classname: "ammo_trap",
        ammo_classname: "ammo_trap",
    },
    BotWeaponBindingT {
        name: "grapple",
        bit: 1 << 20,
        classname: "weapon_grapple",
        ammo_classname: "",
    },
];

/// Ammo classnames named by the bindings.
pub const BOT_AMMO_CLASSNAMES: &[&str] = &[
    "ammo_shells",
    "ammo_bullets",
    "ammo_cells",
    "ammo_rockets",
    "ammo_grenades",
    "ammo_slugs",
    "ammo_flechettes",
    "ammo_prox",
    "ammo_tesla",
    "ammo_trap",
    "ammo_magslug",
    "ammo_disruptor",
    "ammo_nuke",
];

/// Objective binding: classname plus owning team.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BotObjectiveBindingT {
    /// Classname.
    pub classname: &'static str,
    /// Team.
    pub team: i32,
}

/// Standard CTF objective classnames.
pub const BOT_OBJECTIVE_BINDINGS: &[BotObjectiveBindingT] = &[
    BotObjectiveBindingT {
        classname: "item_flag_team1",
        team: 1,
    },
    BotObjectiveBindingT {
        classname: "item_flag_team2",
        team: 2,
    },
];

/// Inventory bit for a KEX classname.
#[must_use]
pub fn weapon_number_for_classname(classname: &str) -> Option<i32> {
    BOT_WEAPON_BINDINGS
        .iter()
        .find(|binding| binding.classname == classname)
        .map(|binding| binding.bit)
}

/// Build Q2 knowledge from source files.
#[must_use]
pub fn bot_build_knowledge(files: &BotDataFilesT) -> BotKnowledge {
    BotKnowledge::new(files, BotDataFormat::Q2, Some(&weapon_number_for_classname))
}

/// Resolve the Q2 game mode from cvar values.
#[must_use]
pub fn bot_game_mode(knowledge: &BotKnowledge, cvar_value: &dyn Fn(&str) -> f64) -> BotGameModeT {
    knowledge.game_mode(cvar_value)
}

/// Settings platform.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum SettingsPlatform {
    /// PC.
    #[default]
    Pc,
    /// Consoles.
    Consoles,
    /// Nintendo.
    Nintendo,
}

impl SettingsPlatform {
    /// Settings file name.
    #[must_use]
    pub fn file_name(self) -> &'static str {
        match self {
            Self::Pc => "settings_PC",
            Self::Consoles => "settings_Consoles",
            Self::Nintendo => "settings_Nintendo",
        }
    }
}

/// Load Q2 knowledge from mounted sources.
pub fn bot_load_knowledge(files: &dyn BotSourceFiles, platform: SettingsPlatform) -> Result<BotKnowledge, BotsError> {
    let read = |name: &str| read_bot_source_text(files, &format!("bots/{name}.txt"));
    let weapons = read("weapons");
    let settings = read(platform.file_name());
    match (weapons, settings) {
        (Some(weapons), Some(settings)) => Ok(bot_build_knowledge(&BotDataFilesT::from_source_files(
            files, weapons, settings,
        ))),
        _ => Err(BotsError::BotScript(format!(
            "Q2 rerelease bot source data unavailable: weapons.txt and {}.txt are required",
            platform.file_name()
        ))),
    }
}
