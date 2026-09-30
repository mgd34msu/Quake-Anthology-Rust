//! Q2 mission packs (`src/content/q2/missionpacks`).

pub mod damage;
pub mod doppleganger;
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

use self::items::{disconnected_item_hooks, Q2MissionPackItemHooks, Q2MissionPackPowerups};
use self::projectiles::disconnected_projectile_hooks;
use self::spheres::{disconnected_sphere_hooks, Q2SphereHooks};
use crate::q2::foundation::items::Q2ItemModule;

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
        }
    }
}
