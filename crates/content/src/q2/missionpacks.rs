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

use types::Q2MissionPackProjectileHooks;

use self::projectiles::disconnected_projectile_hooks;
use self::spheres::{disconnected_sphere_hooks, Q2SphereHooks};

/// Arena runtime state for the mission packs.
#[derive(Debug, Clone, Copy)]
pub struct MissionPackRuntime {
    /// Session projectile hooks.
    pub projectile_hooks: Q2MissionPackProjectileHooks,
    /// Session sphere hooks.
    pub sphere_hooks: Q2SphereHooks,
}

impl Default for MissionPackRuntime {
    fn default() -> Self {
        Self {
            projectile_hooks: disconnected_projectile_hooks(),
            sphere_hooks: disconnected_sphere_hooks(),
        }
    }
}
