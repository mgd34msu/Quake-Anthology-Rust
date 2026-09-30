//! Rogue random items (`src/content/q2/missionpacks/random-items.ts`).
//!
//! Rogue g_newdm.c and rerelease rogue/g_rogue_newdm.cpp item
//! substitution.

use qa_core::identity::ActorId;

use crate::q2::foundation::host::{Q2Edition, Q2GameServices};
use crate::q2::foundation::items::{Q2ItemDefinition, Q2ItemKind};

/// Random item settings (`Q2RandomItemSettings`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Q2RandomItemSettings {
    /// Whether random items are enabled.
    pub enabled: bool,
    /// Whether mines are excluded.
    pub no_mines: bool,
    /// Whether nukes are excluded.
    pub no_nukes: bool,
    /// Whether spheres are excluded.
    pub no_spheres: bool,
}

/// Classic item tables, in original itemlist order (`classic`).
const CLASSIC: [&[&str]; 4] = [
    &[
        "weapon_blaster",
        "weapon_shotgun",
        "weapon_supershotgun",
        "weapon_machinegun",
        "weapon_chaingun",
        "weapon_etf_rifle",
        "weapon_grenadelauncher",
        "weapon_proxlauncher",
        "weapon_rocketlauncher",
        "weapon_hyperblaster",
        "weapon_plasmabeam",
        "weapon_railgun",
        "weapon_bfg",
        "weapon_chainfist",
    ],
    &[
        "ammo_grenades",
        "ammo_shells",
        "ammo_bullets",
        "ammo_cells",
        "ammo_rockets",
        "ammo_slugs",
        "ammo_flechettes",
        "ammo_prox",
        "ammo_tesla",
    ],
    &[
        "ammo_nuke",
        "item_quad",
        "item_invulnerability",
        "item_silencer",
        "item_breather",
        "item_enviro",
        "item_ir_goggles",
        "item_double",
        "item_compass",
        "item_sphere_vengeance",
        "item_sphere_hunter",
        "item_sphere_defender",
        "item_doppleganger",
    ],
    &[
        "key_data_cd",
        "key_power_cube",
        "key_pyramid",
        "key_data_spinner",
        "key_pass",
        "key_blue_key",
        "key_red_key",
        "key_commander_head",
        "key_airstrike_target",
        "key_nuke_container",
        "key_nuke",
    ],
];

/// Rerelease item tables (`rerelease`).
const RERELEASE: [&[&str]; 4] = [
    &[
        "weapon_chainfist",
        "weapon_shotgun",
        "weapon_supershotgun",
        "weapon_machinegun",
        "weapon_etf_rifle",
        "weapon_chaingun",
        "weapon_grenadelauncher",
        "weapon_proxlauncher",
        "weapon_rocketlauncher",
        "weapon_hyperblaster",
        "weapon_boomer",
        "weapon_plasmabeam",
        "weapon_railgun",
        "weapon_phalanx",
        "weapon_bfg",
        "weapon_disintegrator",
    ],
    &[
        "ammo_grenades",
        "ammo_trap",
        "ammo_tesla",
        "ammo_shells",
        "ammo_bullets",
        "ammo_cells",
        "ammo_rockets",
        "ammo_slugs",
        "ammo_magslug",
        "ammo_flechettes",
        "ammo_prox",
        "ammo_disruptor",
    ],
    &[
        "ammo_nuke",
        "item_quad",
        "item_quadfire",
        "item_invulnerability",
        "item_invisibility",
        "item_silencer",
        "item_breather",
        "item_enviro",
        "item_adrenaline",
        "item_bandolier",
        "item_pack",
        "item_ir_goggles",
        "item_double",
        "item_sphere_vengeance",
        "item_sphere_hunter",
        "item_sphere_defender",
        "item_doppleganger",
        "item_health_mega",
    ],
    &[
        "key_data_cd",
        "key_power_cube",
        "key_explosive_charges",
        "key_yellow_key",
        "key_power_core",
        "key_pyramid",
        "key_data_spinner",
        "key_pass",
        "key_blue_key",
        "key_red_key",
        "key_green_key",
        "key_commander_head",
        "key_airstrike_target",
        "key_nuke_container",
        "key_nuke",
    ],
];

/// Find the category table holding a classname (`category`).
fn category(classname: &str, table: [&[&str]; 4]) -> Option<usize> {
    table
        .iter()
        .position(|choices| choices.contains(&classname))
}

/// Pick a rerelease random integer (`rereleaseRandom?.integer(n) ?? floor(random * n)`).
fn rerelease_integer(game: &mut Q2GameServices, count: usize) -> usize {
    if let Some(source) = game.host.rerelease_random() {
        return source.integer_max(count as i32).max(0) as usize;
    }
    (game.host.random() * count as f64).floor() as usize
}

/// Substitute a random item (`q2RandomItem`).
pub fn q2_random_item(
    entity: &ActorId,
    item: &Q2ItemDefinition,
    game: &mut Q2GameServices,
    settings: &Q2RandomItemSettings,
) -> Option<String> {
    if !settings.enabled {
        return None;
    }
    if game.options.edition == Q2Edition::Classic {
        if item.kind() == Q2ItemKind::PowerArmor {
            return None;
        }
        if item.kind() == Q2ItemKind::Health || item.classname == "item_adrenaline" {
            if game.require_entity(entity).classname == "item_health_small" {
                return None;
            }
            let chance = game.host.random();
            return Some(
                if chance < 0.6 {
                    "item_health"
                } else if chance < 0.9 {
                    "item_health_large"
                } else if chance < 0.99 {
                    "item_adrenaline"
                } else {
                    "item_health_mega"
                }
                .to_string(),
            );
        }
        if item.kind() == Q2ItemKind::Armor {
            let chance = game.host.random();
            return Some(
                if chance < 0.6 {
                    "item_armor_jacket"
                } else if chance < 0.9 {
                    "item_armor_combat"
                } else {
                    "item_armor_body"
                }
                .to_string(),
            );
        }
        if item.kind() == Q2ItemKind::Shard {
            return None;
        }
        let kind = category(&item.classname, CLASSIC)?;
        let classname = game.require_entity(entity).classname.clone();
        if settings.no_spheres
            && [
                "item_sphere_vengeance",
                "item_sphere_hunter",
                "item_spehre_defender",
            ]
            .contains(&classname.as_str())
            || settings.no_nukes && classname == "ammo_nuke"
            || settings.no_mines && ["ammo_prox", "ammo_tesla"].contains(&classname.as_str())
        {
            return None;
        }
        let choices = CLASSIC[kind];
        let pick = (game.host.random() * choices.len() as f64).ceil() as usize;
        return choices.get(pick.wrapping_sub(1)).map(|classname| classname.to_string());
    }
    if ["item_flag_team1", "item_flag_team2", "dm_tag_token"].contains(&item.classname.as_str()) {
        return None;
    }
    if ["item_health_small", "item_armor_shard"].contains(&item.classname.as_str()) {
        let choice = rerelease_integer(game, 2);
        return Some(if choice == 0 {
            "item_health_small"
        } else {
            "item_armor_shard"
        }
        .to_string());
    }
    if ["item_health", "item_health_large"].contains(&item.classname.as_str()) {
        return Some(if game.host.random() < 0.6 {
            "item_health"
        } else {
            "item_health_large"
        }
        .to_string());
    }
    if [
        "item_armor_jacket",
        "item_armor_combat",
        "item_armor_body",
        "item_power_screen",
        "item_power_shield",
    ]
    .contains(&item.classname.as_str())
    {
        let chance = game.host.random();
        return Some(
            if chance < 0.4 {
                "item_armor_jacket"
            } else if chance < 0.6 {
                "item_armor_combat"
            } else if chance < 0.8 {
                "item_armor_body"
            } else if chance < 0.9 {
                "item_power_screen"
            } else {
                "item_power_shield"
            }
            .to_string(),
        );
    }
    if item.classname == "item_ancient_head" || item.classname == "item_legacy_head" {
        let choices = ["item_health_small", "item_health", "item_health_large"];
        return choices
            .get(rerelease_integer(game, 3))
            .map(|classname| classname.to_string());
    }
    let kind = if item.classname == "weapon_blaster" || item.classname == "weapon_grapple" {
        0
    } else {
        category(&item.classname, RERELEASE)?
    };
    let choices: Vec<&&str> = RERELEASE[kind]
        .iter()
        .filter(|classname| {
            !(settings.no_spheres && classname.starts_with("item_sphere_")
                || settings.no_nukes && **classname == "ammo_nuke"
                || settings.no_mines
                    && [
                        "ammo_prox",
                        "ammo_tesla",
                        "ammo_trap",
                        "weapon_proxlauncher",
                    ]
                    .contains(*classname))
        })
        .collect();
    if choices.is_empty() {
        return None;
    }
    choices
        .get(rerelease_integer(game, choices.len()))
        .map(|classname| classname.to_string())
}
