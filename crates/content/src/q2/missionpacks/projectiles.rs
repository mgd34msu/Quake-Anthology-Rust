//! Mission-pack projectiles (`src/content/q2/missionpacks/projectiles`).

pub mod bolts;
pub mod common;
pub mod mines;
pub mod nuke;
pub mod trap;

use qa_core::identity::ActorId;

use super::types::{Q2MissionPackPlayerEffect, Q2MissionPackProjectileHooks};
use crate::q2::foundation::host::Q2GameServices;
use crate::q2::foundation::monsters::types::MonsterContext;

/// Mission-pack projectile driver (`Q2MissionPackProjectiles`).
///
/// The donor chains bolt, mine, nuke, and trap classes through
/// inheritance; the port implements the same handle in each projectile
/// module (`bolts`, `mines`, `nuke`, `trap`) and merges the callback
/// tables from trap down through the shared base table.
#[derive(Debug, Clone, Copy)]
pub struct Q2MissionPackProjectiles {
    /// Session projectile hooks.
    pub hooks: Q2MissionPackProjectileHooks,
}

/// Disconnected monster hook (resolves nothing).
fn disconnected_monster(
    _actor: ActorId,
    _game: &mut Q2GameServices,
) -> Option<MonsterContext<'_>> {
    None
}

/// Disconnected player-effect hook (drops the effect).
fn disconnected_player_effect(_effect: Q2MissionPackPlayerEffect) {}

/// Projectile hooks used before the session installs its own.
pub fn disconnected_projectile_hooks() -> Q2MissionPackProjectileHooks {
    Q2MissionPackProjectileHooks {
        strong_mines: false,
        gravity: None,
        monster: disconnected_monster,
        player_effect: disconnected_player_effect,
    }
}

/// Session projectile hooks (`Q2MissionPackProjectileHooks`).
pub fn mission_hooks(game: &Q2GameServices) -> Q2MissionPackProjectileHooks {
    game.mission_packs.projectile_hooks
}

/// Projectile driver bound to the session hooks.
pub fn mission_projectiles(game: &Q2GameServices) -> Q2MissionPackProjectiles {
    Q2MissionPackProjectiles {
        hooks: mission_hooks(game),
    }
}
