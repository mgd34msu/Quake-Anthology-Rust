//! Bot arsenal knowledge from `src/bots/behavior/q3/arsenal-knowledge.ts`.
//!
//! Default weapon selection over the weapon config: owned weapons
//! rank by fuzzy weight for the sensed range, melee maps to the
//! gauntlet role, splash maps to rocket/grenade roles, and hitscan
//! maps to machinegun/shotgun/lightning/railgun by spread and cycle.

use crate::behavior::library::character::BotCharacterLibrary;
use crate::behavior::library::genetic::BotRandom;
use crate::behavior::library::weapons::{WeaponAi, DAMAGE_TYPE_RADIAL};
use crate::behavior::q3::ai_definitions::BotInventory;
use crate::behavior::q3::ai_state::BotState;
use crate::behavior::q3::game_host::{BotArsenalKnowledge, BotObservedPickup, BotWeaponTactics};

/// Weapon numbers (`Weapon`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Weapon;

impl Weapon {
    /// None.
    pub const NONE: i32 = 0;
    /// Gauntlet.
    pub const GAUNTLET: i32 = 1;
    /// Machinegun.
    pub const MACHINEGUN: i32 = 2;
    /// Shotgun.
    pub const SHOTGUN: i32 = 3;
    /// Grenade launcher.
    pub const GRENADE_LAUNCHER: i32 = 4;
    /// Rocket launcher.
    pub const ROCKET_LAUNCHER: i32 = 5;
    /// Lightning.
    pub const LIGHTNING: i32 = 6;
    /// Railgun.
    pub const RAILGUN: i32 = 7;
    /// Plasmagun.
    pub const PLASMAGUN: i32 = 8;
    /// BFG.
    pub const BFG: i32 = 9;
    /// Grappling hook.
    pub const GRAPPLING_HOOK: i32 = 10;
    /// Nailgun.
    pub const NAILGUN: i32 = 11;
    /// Prox launcher.
    pub const PROX_LAUNCHER: i32 = 12;
    /// Chaingun.
    pub const CHAINGUN: i32 = 13;
}

/// Personality role for a weapon number.
#[must_use]
pub fn personality_role(
    _weapon: i32,
    melee: bool,
    gravity: f32,
    speed: f32,
    damage_type: i32,
    projectile_count: i32,
    spread: bool,
    reload: f32,
) -> i32 {
    if melee {
        return Weapon::GAUNTLET;
    }
    if gravity > 0.0 {
        return Weapon::GRENADE_LAUNCHER;
    }
    if speed > 0.0 {
        return if damage_type & DAMAGE_TYPE_RADIAL != 0 {
            Weapon::ROCKET_LAUNCHER
        } else {
            Weapon::PLASMAGUN
        };
    }
    if projectile_count >= 4 {
        return Weapon::SHOTGUN;
    }
    if spread {
        return Weapon::MACHINEGUN;
    }
    if reload <= 0.2 {
        Weapon::LIGHTNING
    } else {
        Weapon::RAILGUN
    }
}

/// Default arsenal knowledge over a weapon config.
pub struct DefaultArsenalKnowledge {
    melee_range: f32,
}

impl DefaultArsenalKnowledge {
    /// New default knowledge.
    #[must_use]
    pub fn new() -> Self {
        Self { melee_range: 60.0 }
    }
}

impl Default for DefaultArsenalKnowledge {
    fn default() -> Self {
        Self::new()
    }
}

impl BotArsenalKnowledge for DefaultArsenalKnowledge {
    fn pickup_utility(&self, _characters: &BotCharacterLibrary, state: &BotState, pickup: &BotObservedPickup) -> f32 {
        if !pickup.eligible {
            return 0.0;
        }
        let health = state.inventory.get(BotInventory::HEALTH).copied().unwrap_or(100) as f32;
        let name = pickup.name.to_lowercase();
        if name.contains("health") {
            return if health < 100.0 {
                (100.0 - health).max(10.0)
            } else {
                0.0
            };
        }
        if name.contains("armor") {
            let armor = state.inventory.get(BotInventory::ARMOR).copied().unwrap_or(0) as f32;
            return if armor < 100.0 { (100.0 - armor).max(10.0) } else { 0.0 };
        }
        if name.contains("weapon") {
            return 50.0;
        }
        if name.contains("ammo") {
            return 20.0;
        }
        10.0
    }

    fn choose_weapon(&self, _characters: &BotCharacterLibrary, state: &BotState) -> i32 {
        // Highest owned weapon number wins; the weapon AI refines this
        // with fuzzy weights when a config is loaded.
        let mut best = Weapon::GAUNTLET;
        for weapon in Weapon::GAUNTLET..=Weapon::BFG {
            let slot = match weapon {
                Weapon::GAUNTLET => BotInventory::GAUNTLET,
                Weapon::MACHINEGUN => BotInventory::MACHINEGUN,
                Weapon::SHOTGUN => BotInventory::SHOTGUN,
                Weapon::GRENADE_LAUNCHER => BotInventory::GRENADELAUNCHER,
                Weapon::ROCKET_LAUNCHER => BotInventory::ROCKETLAUNCHER,
                Weapon::LIGHTNING => BotInventory::LIGHTNING,
                Weapon::RAILGUN => BotInventory::RAILGUN,
                Weapon::PLASMAGUN => BotInventory::PLASMAGUN,
                Weapon::BFG => BotInventory::BFG10K,
                _ => continue,
            };
            if state.inventory.get(slot).copied().unwrap_or(0) > 0 {
                best = weapon;
            }
        }
        best
    }

    fn activation_weapon(&self, _characters: &BotCharacterLibrary, state: &BotState) -> i32 {
        if state.inventory.get(BotInventory::ROCKETLAUNCHER).copied().unwrap_or(0) > 0 {
            Weapon::ROCKET_LAUNCHER
        } else if state.inventory.get(BotInventory::PLASMAGUN).copied().unwrap_or(0) > 0 {
            Weapon::PLASMAGUN
        } else {
            Weapon::MACHINEGUN
        }
    }

    fn tactics(&self, weapon: i32) -> BotWeaponTactics {
        match weapon {
            Weapon::GAUNTLET => BotWeaponTactics {
                melee: true,
                maximum_range: Some(self.melee_range),
                ..BotWeaponTactics::default()
            },
            Weapon::ROCKET_LAUNCHER | Weapon::GRENADE_LAUNCHER => BotWeaponTactics {
                predict_occluded_splash: true,
                ..BotWeaponTactics::default()
            },
            _ => BotWeaponTactics::default(),
        }
    }

    fn aggression(&self, state: &BotState) -> f32 {
        let health = state.inventory.get(BotInventory::HEALTH).copied().unwrap_or(100) as f32;
        (health / 100.0).clamp(0.1, 1.0)
    }

    fn update_inventory(&mut self, state: &mut BotState) {
        super::ai_combat::update_bot_inventory(state);
        super::ai_combat::update_bot_item_inventory(state);
    }
}

/// Choose the best fight weapon through the weapon AI when available.
pub fn choose_fight_weapon(
    weapons: &WeaponAi<'_>,
    handle: i32,
    state: &BotState,
    range: f32,
    random: &mut dyn BotRandom,
) -> i32 {
    let picked = weapons.choose_best_weapon(handle, &state.inventory, range, random);
    if picked > 0 {
        picked
    } else {
        DefaultArsenalKnowledge::new().choose_weapon(&BotCharacterLibrary::new(), state)
    }
}
