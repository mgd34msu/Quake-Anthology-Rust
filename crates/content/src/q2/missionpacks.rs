//! Q2 mission packs (`src/content/q2/missionpacks`).

pub mod damage;
pub mod items;
pub mod modes;
pub mod monsters;
pub mod projectiles;
pub mod types;
pub mod weapons;

use types::Q2MissionPackProjectileHooks;

use self::projectiles::disconnected_projectile_hooks;

/// Arena runtime state for the mission packs.
#[derive(Debug, Clone, Copy)]
pub struct MissionPackRuntime {
    /// Session projectile hooks.
    pub projectile_hooks: Q2MissionPackProjectileHooks,
}

impl Default for MissionPackRuntime {
    fn default() -> Self {
        Self {
            projectile_hooks: disconnected_projectile_hooks(),
        }
    }
}
