//! Mission-pack entities barrel (`src/content/q2/missionpacks/entities/index.ts`).
//!
//! Selects the Xatrix or Rogue entity source behind `Q2MissionPackEntities`
//! with Rogue mover composition and steam-id checkpointing.

pub mod movers;
pub mod rogue;
pub mod types;
pub mod xatrix;

use qa_core::identity::ActorId;

use self::movers::{rogue_mover_callbacks, Q2RogueMovers};
pub use self::rogue::Q2RogueEntitiesCheckpoint;
use self::rogue::{rogue_callbacks, Q2RogueEntities};
pub use self::types::{Q2MissionPackEntityEvent, Q2MissionPackEntityHooks};
use self::xatrix::{xatrix_callbacks, Q2XatrixEntities};
use super::types::Q2MissionPack;
use crate::q2::foundation::host::{Q2GameServices, Q2SpawnFn, SpawnModule};

/// Mission-pack entities (`Q2MissionPackEntities`).
#[derive(Debug, Clone, Copy)]
pub struct Q2MissionPackEntities {
    /// Mission pack.
    pub pack: Q2MissionPack,
    /// Entity hooks.
    pub hooks: Q2MissionPackEntityHooks,
}

impl Q2MissionPackEntities {
    /// Register the pack entities and build the spawn module.
    pub fn register(&self, game: &mut Q2GameServices) -> SpawnModule {
        game.mission_packs.entity_hooks = Some(self.hooks);
        let mut callbacks = match self.pack {
            Q2MissionPack::Xatrix => xatrix_callbacks(),
            Q2MissionPack::Rogue => rogue_callbacks(),
        };
        if self.pack == Q2MissionPack::Rogue {
            let movers = rogue_mover_callbacks();
            callbacks.think.extend(movers.think);
            callbacks.use_.extend(movers.use_);
            callbacks.touch.extend(movers.touch);
            callbacks.die.extend(movers.die);
            callbacks.blocked.extend(movers.blocked);
            callbacks.trajectory.extend(movers.trajectory);
        }
        let spawn: Q2SpawnFn = match self.pack {
            Q2MissionPack::Xatrix => xatrix_spawn,
            Q2MissionPack::Rogue => rogue_spawn,
        };
        SpawnModule {
            spawn,
            item_name: |_| None,
            callbacks,
        }
    }

    /// Spawn a pack entity (`spawn`).
    pub fn spawn(&self, entity: ActorId, game: &mut Q2GameServices) -> bool {
        match self.pack {
            Q2MissionPack::Xatrix => Q2XatrixEntities { hooks: self.hooks }.spawn(entity, game),
            Q2MissionPack::Rogue => {
                Q2RogueEntities { hooks: self.hooks }.spawn(entity.clone(), game)
                    || Q2RogueMovers { hooks: self.hooks }.spawn(entity, game)
            }
        }
    }

    /// Capture Rogue entity state (`capture`).
    pub fn capture(&self, game: &Q2GameServices) -> Q2RogueEntitiesCheckpoint {
        match self.pack {
            Q2MissionPack::Xatrix => Q2RogueEntitiesCheckpoint { steam_id: 0 },
            Q2MissionPack::Rogue => Q2RogueEntities { hooks: self.hooks }.capture(game),
        }
    }

    /// Restore Rogue entity state (`restore`).
    pub fn restore(&self, game: &mut Q2GameServices, state: Q2RogueEntitiesCheckpoint) {
        if self.pack == Q2MissionPack::Rogue {
            Q2RogueEntities { hooks: self.hooks }.restore(game, state);
        }
    }
}

/// Xatrix spawn entry.
fn xatrix_spawn(entity: ActorId, game: &mut Q2GameServices) -> bool {
    let hooks = game
        .mission_packs
        .entity_hooks
        .expect("Q2 mission-pack entities are not registered");
    Q2XatrixEntities { hooks }.spawn(entity, game)
}

/// Rogue spawn entry (entities, then movers).
fn rogue_spawn(entity: ActorId, game: &mut Q2GameServices) -> bool {
    let hooks = game
        .mission_packs
        .entity_hooks
        .expect("Q2 mission-pack entities are not registered");
    Q2RogueEntities { hooks }.spawn(entity.clone(), game) || Q2RogueMovers { hooks }.spawn(entity, game)
}

#[cfg(test)]
mod tests {
    use qa_core::math::vec3;

    use super::*;
    use crate::q2::foundation::movers::{create_q2_mover_module, Q2MoverHooks};

    fn path_hook(_corner: ActorId, _game: &mut Q2GameServices, _other: ActorId) {}

    fn teleport(_actor: ActorId, _origin: qa_core::math::Vec3, _angles: qa_core::math::Vec3) {}

    fn anger(_actor: ActorId, _target: ActorId, _game: &mut Q2GameServices) {}

    fn emit(_event: Q2MissionPackEntityEvent) {}

    fn hooks() -> Q2MissionPackEntityHooks {
        Q2MissionPackEntityHooks {
            movers: create_q2_mover_module(Q2MoverHooks {
                path_corner: path_hook,
                combat_point: path_hook,
            }),
            teleport_player: teleport,
            target_anger: anger,
            emit,
        }
    }

    #[test]
    fn modules_carry_their_pack() {
        let xatrix = Q2MissionPackEntities {
            pack: Q2MissionPack::Xatrix,
            hooks: hooks(),
        };
        let rogue = Q2MissionPackEntities {
            pack: Q2MissionPack::Rogue,
            hooks: hooks(),
        };
        assert_eq!(xatrix.pack, Q2MissionPack::Xatrix);
        assert_eq!(rogue.pack, Q2MissionPack::Rogue);
    }

    #[test]
    fn pack_callbacks_cover_source_names() {
        assert!(xatrix_callbacks().think.contains_key("rotating_light_alarm"));
        let rogue = rogue_callbacks();
        assert!(rogue.think.contains_key("target_steam_start"));
        assert!(rogue.touch.contains_key("trigger_teleport_touch"));
        assert!(rogue.use_.contains_key("use_target_steam"));
    }

    #[test]
    fn rogue_checkpoint_defaults_to_zero_steam_id() {
        let checkpoint = Q2RogueEntitiesCheckpoint { steam_id: 0 };
        assert_eq!(checkpoint.steam_id, 0);
        let event = Q2MissionPackEntityEvent::Steam {
            id: 1,
            origin: vec3(0.0, 0.0, 0.0),
            direction: vec3(0.0, 0.0, 1.0),
            count: 8,
            color: 0,
            speed: 100,
            milliseconds: 500,
        };
        assert!(matches!(event, Q2MissionPackEntityEvent::Steam { id: 1, .. }));
    }
}
