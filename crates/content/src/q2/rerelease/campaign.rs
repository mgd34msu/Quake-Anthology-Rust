//! Q2 rerelease campaign (`src/content/q2/rerelease/campaign.ts`).
//!
//! Gameplay logic adapted from id Software's Quake II game and the
//! rerelease game DLL (GPL-2.0-or-later).

use std::collections::{HashMap, HashSet};

use crate::q2::foundation::fields::number_field;
use crate::q2::foundation::host::{Q2GameServices, Q2Mode};

/// Maximum tracked levels per unit.
const MAX_LEVELS: usize = 8;

/// Rerelease level entry (`Q2RereleaseLevelEntry`).
#[derive(Debug, Clone, PartialEq)]
pub struct Q2RereleaseLevelEntry {
    /// Map name.
    pub map: String,
    /// Level name.
    pub name: String,
    /// Visit order.
    pub visit_order: i32,
    /// Total secrets.
    pub total_secrets: i32,
    /// Found secrets.
    pub found_secrets: i32,
    /// Total monsters.
    pub total_monsters: i32,
    /// Killed monsters.
    pub killed_monsters: i32,
    /// Level time.
    pub time: f64,
}

/// Rerelease mission objectives (`Q2RereleaseCampaignState["mission"]`).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Q2RereleaseMission {
    /// Primary objective.
    pub primary: String,
    /// Secondary objective.
    pub secondary: String,
    /// Primary change counter.
    pub primary_changes: i32,
    /// Secondary change counter.
    pub secondary_changes: i32,
}

/// Rerelease campaign state (`Q2RereleaseCampaignState`).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Q2RereleaseCampaignState {
    /// Cross-unit flags.
    pub cross_unit_flags: i32,
    /// Visited maps.
    pub visited_maps: HashSet<String>,
    /// Level entries by map.
    pub levels: HashMap<String, Q2RereleaseLevelEntry>,
    /// Mission objectives.
    pub mission: Q2RereleaseMission,
}

/// Create rerelease campaign state (`createQ2RereleaseCampaignState`).
pub fn create_q2_rerelease_campaign_state() -> Q2RereleaseCampaignState {
    Q2RereleaseCampaignState::default()
}

/// Add a level entry, or return none when the unit already tracks eight maps.
fn add_level<'a>(campaign: &'a mut Q2RereleaseCampaignState, map: &str) -> Option<&'a mut Q2RereleaseLevelEntry> {
    if !campaign.levels.contains_key(map) {
        if campaign.levels.len() >= MAX_LEVELS {
            return None;
        }
        campaign.levels.insert(
            map.to_string(),
            Q2RereleaseLevelEntry {
                map: map.to_string(),
                name: String::new(),
                visit_order: 0,
                total_secrets: 0,
                found_secrets: 0,
                total_monsters: 0,
                killed_monsters: 0,
                time: 0.0,
            },
        );
    }
    campaign.levels.get_mut(map)
}

/// Enter a rerelease level (`enterQ2RereleaseLevel`).
pub fn enter_q2_rerelease_level(game: &mut Q2GameServices) {
    if game.options.mode == Q2Mode::Deathmatch {
        return;
    }
    let world_actor = game.host.world_actor();
    let hub = game
        .entity(&world_actor)
        .is_some_and(|world| number_field(&world.spawn, "hub_map", 0.0) != 0.0);
    if hub {
        return;
    }
    let map_name = game.options.map_name.clone();
    let world_message = game.entity(&world_actor).map(|world| world.message.clone());
    if add_level(&mut game.rerelease.campaign, &map_name).is_none() {
        game.host
            .diagnostic("More than 8 maps in unit; cannot track remaining levels");
        return;
    }
    let unnamed = game
        .rerelease
        .campaign
        .levels
        .get(&map_name)
        .is_some_and(|entry| entry.name.is_empty());
    if unnamed {
        let visit_order = game
            .rerelease
            .campaign
            .levels
            .values()
            .map(|entry| entry.visit_order)
            .max()
            .unwrap_or(0)
            .max(0)
            + 1;
        let name = world_message.unwrap_or_default();
        let entry = game
            .rerelease
            .campaign
            .levels
            .get_mut(&map_name)
            .expect("Q2 rerelease level entry is missing");
        entry.name = if name.is_empty() { map_name.clone() } else { name };
        entry.visit_order = visit_order;
        game.rerelease.campaign.visited_maps.insert(map_name.clone());
        if game.rerelease.options.coop_lives {
            let coop_num_lives = game.rerelease.options.coop_num_lives;
            for extra in game.rerelease.states.values_mut() {
                extra.lives = (coop_num_lives + 1).min(extra.lives + 1);
            }
        }
    }
    let destinations: Vec<String> = game
        .entities
        .values()
        .filter(|entity| {
            entity.classname == "target_changelevel" && !entity.map.is_empty() && !entity.map.contains('*')
        })
        .filter_map(|entity| {
            let destination = entity
                .map
                .find('+')
                .map_or(entity.map.as_str(), |index| &entity.map[index + 1..]);
            if destination.contains(".cin") || destination.contains(".pcx") {
                return None;
            }
            let map = destination.split('$').next().unwrap_or("");
            if map.is_empty() {
                None
            } else {
                Some(map.to_string())
            }
        })
        .collect();
    for map in destinations {
        if add_level(&mut game.rerelease.campaign, &map).is_none() {
            game.host
                .diagnostic("More than 8 maps in unit; cannot track remaining levels");
            return;
        }
    }
}

/// Update the current level entry (`updateQ2RereleaseLevel`).
pub fn update_q2_rerelease_level(game: &mut Q2GameServices) {
    let counters = game.counters;
    if let Some(entry) = game.rerelease.campaign.levels.get_mut(&game.options.map_name.clone()) {
        entry.found_secrets = counters.found_secrets;
        entry.total_secrets = counters.total_secrets;
        entry.killed_monsters = counters.killed_monsters;
        entry.total_monsters = counters.total_monsters;
    }
}

/// Report unit levels in visit order (`q2RereleaseUnitReport`).
pub fn q2_rerelease_unit_report(game: &Q2GameServices) -> Vec<Q2RereleaseLevelEntry> {
    let order = |entry: &Q2RereleaseLevelEntry| {
        if entry.visit_order != 0 {
            entry.visit_order
        } else if entry.name.is_empty() {
            10
        } else {
            9
        }
    };
    let mut levels: Vec<Q2RereleaseLevelEntry> = game.rerelease.campaign.levels.values().cloned().collect();
    levels.sort_by(|left, right| order(left).cmp(&order(right)).then(left.map.cmp(&right.map)));
    levels
}
