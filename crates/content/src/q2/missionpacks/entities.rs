//! Mission-pack entities (`src/content/q2/missionpacks/entities`).

pub mod movers;
pub mod rogue;
pub mod types;
pub mod xatrix;

use qa_core::identity::ActorId;

use self::movers::{rogue_mover_callbacks, Q2RogueMovers};
use self::rogue::{rogue_callbacks, Q2RogueEntities, Q2RogueEntitiesCheckpoint};
use self::types::Q2MissionPackEntityHooks;
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
    Q2RogueEntities { hooks }.spawn(entity.clone(), game)
        || Q2RogueMovers { hooks }.spawn(entity, game)
}
