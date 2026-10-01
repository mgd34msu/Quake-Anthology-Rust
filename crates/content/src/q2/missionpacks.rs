//! Q2 mission packs (`src/content/q2/missionpacks`).

pub mod damage;
pub mod doppleganger;
pub mod entities;
pub mod items;
pub mod modes;
pub mod monsters;
pub mod players;
pub mod projectiles;
pub mod random_items;
pub mod spheres;
pub mod types;
pub mod weapons;

use std::collections::HashMap;

use qa_core::identity::ActorId;

use types::{Q2MissionPack, Q2MissionPackProjectileHooks};

use self::doppleganger::{mission_doppleganger, Q2MissionPackDoppleganger};
use self::entities::types::Q2MissionPackEntityHooks;
use self::items::{disconnected_item_hooks, Q2MissionPackItemHooks, Q2MissionPackItems, Q2MissionPackPowerups};
use self::projectiles::{disconnected_projectile_hooks, mission_projectiles, Q2MissionPackProjectiles};
use self::spheres::{disconnected_sphere_hooks, mission_spheres, Q2MissionPackSpheres, Q2SphereHooks};
use self::weapons::player::Q2MissionPackWeapons;
use crate::q2::foundation::host::{Q2Edition, Q2GameServices};
use crate::q2::foundation::items::Q2ItemModule;
use crate::q2::foundation::weapons::player::set_fallback_order;
use crate::q2::foundation::weapons::types::Q2WeaponName;

/// Arena runtime state for the mission packs.
#[derive(Debug, Clone)]
pub struct MissionPackRuntime {
    /// Session projectile hooks.
    pub projectile_hooks: Q2MissionPackProjectileHooks,
    /// Session sphere hooks.
    pub sphere_hooks: Q2SphereHooks,
    /// Session item hooks.
    pub item_hooks: Q2MissionPackItemHooks,
    /// Registered items pack.
    pub items_pack: Option<Q2MissionPack>,
    /// Shared item module.
    pub shared_items: Option<Q2ItemModule>,
    /// Powerup timers by owner.
    pub item_powers: HashMap<ActorId, Q2MissionPackPowerups>,
    /// Session entity hooks.
    pub entity_hooks: Option<Q2MissionPackEntityHooks>,
    /// Rogue steam id.
    pub steam_id: i32,
}

impl Default for MissionPackRuntime {
    fn default() -> Self {
        Self {
            projectile_hooks: disconnected_projectile_hooks(),
            sphere_hooks: disconnected_sphere_hooks(),
            item_hooks: disconnected_item_hooks(),
            items_pack: None,
            shared_items: None,
            item_powers: HashMap::new(),
            entity_hooks: None,
            steam_id: 0,
        }
    }
}

/// Mission-pack armory hooks (`Q2MissionPackArmoryHooks`).
///
/// The donor `base` ballistics handle maps to the port's free
/// ballistics functions, so only the remaining hooks travel here.
#[derive(Debug, Clone, Copy)]
pub struct Q2MissionPackArmoryHooks {
    /// Projectile hooks.
    pub projectile_hooks: Q2MissionPackProjectileHooks,
    /// Sphere hooks.
    pub sphere_hooks: Q2SphereHooks,
    /// Item module.
    pub items: Q2ItemModule,
}

/// Installed mission-pack armory (`registerQ2MissionPackArmory` result).
#[derive(Debug, Clone, Copy)]
pub struct Q2MissionPackArmory {
    /// Projectile driver.
    pub projectiles: Q2MissionPackProjectiles,
    /// Weapons module.
    pub weapons: Q2MissionPackWeapons,
    /// Spheres module.
    pub spheres: Q2MissionPackSpheres,
    /// Items module.
    pub items: Q2MissionPackItems,
    /// Doppleganger module.
    pub doppleganger: Q2MissionPackDoppleganger,
}

/// Install the pack arsenal into the session authorities
/// (`registerQ2MissionPackArmory`).
pub fn register_q2_mission_pack_armory(
    game: &mut Q2GameServices,
    pack: Q2MissionPack,
    hooks: Q2MissionPackArmoryHooks,
    edition: Q2Edition,
) -> Q2MissionPackArmory {
    game.mission_packs.projectile_hooks = hooks.projectile_hooks;
    game.mission_packs.sphere_hooks = hooks.sphere_hooks;
    game.mission_packs.item_hooks = Q2MissionPackItemHooks {
        player_effect: hooks.projectile_hooks.player_effect,
    };
    let projectiles = mission_projectiles(game);
    let weapons = Q2MissionPackWeapons::new(projectiles);
    let spheres = mission_spheres(game);
    let doppleganger = mission_doppleganger(game);
    let items = Q2MissionPackItems {
        hooks: game.mission_packs.item_hooks,
        pack,
        shared_items: hooks.items,
    };
    let packs: &[Q2MissionPack] = if edition == Q2Edition::Rerelease {
        &[Q2MissionPack::Xatrix, Q2MissionPack::Rogue]
    } else {
        &[pack]
    };
    for selected in packs {
        weapons.register(game, *selected, edition);
        items.register(game, *selected, edition);
    }
    if edition == Q2Edition::Rerelease {
        set_fallback_order(
            game,
            [
                "disintegrator",
                "railgun",
                "heatbeam",
                "ionripper",
                "hyperblaster",
                "etf_rifle",
                "chaingun",
                "machinegun",
                "supershotgun",
                "shotgun",
                "phalanx",
                "rocketlauncher",
                "grenadelauncher",
                "proxlauncher",
                "chainfist",
                "blaster",
            ]
            .into_iter()
            .map(Q2WeaponName::from)
            .collect(),
        );
    }
    Q2MissionPackArmory {
        projectiles,
        weapons,
        spheres,
        items,
        doppleganger,
    }
}
