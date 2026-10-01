//! Mission-pack entity hooks (`src/content/q2/missionpacks/entities/types.ts`).

use qa_core::identity::ActorId;
use qa_core::math::Vec3;

use crate::q2::foundation::host::Q2GameServices;
use crate::q2::foundation::movers::Q2MoverModule;

/// Mission-pack entity event (`Q2MissionPackEntityEvent`).
#[derive(Debug, Clone, PartialEq)]
pub enum Q2MissionPackEntityEvent {
    /// Steam jet.
    Steam {
        /// Steam id.
        id: i32,
        /// Origin.
        origin: Vec3,
        /// Direction.
        direction: Vec3,
        /// Count.
        count: i32,
        /// Color.
        color: i32,
        /// Speed.
        speed: i32,
        /// Milliseconds.
        milliseconds: i32,
    },
    /// Force wall.
    ForceWall {
        /// Start.
        start: Vec3,
        /// End.
        end: Vec3,
        /// Color.
        color: i32,
    },
}

/// Mission-pack entity hooks (`Q2MissionPackEntityHooks`).
///
/// The donor `weapons` ballistics and `projectiles` handles map to the
/// port's free ballistics functions and projectile driver, so the
/// handle carries only the mover module and the session callbacks.
#[derive(Debug, Clone, Copy)]
pub struct Q2MissionPackEntityHooks {
    /// Mover module.
    pub movers: Q2MoverModule,
    /// Teleport a player.
    pub teleport_player: fn(ActorId, &mut Q2GameServices, Vec3, Vec3),
    /// Anger a monster at a target.
    pub target_anger: fn(ActorId, ActorId, &mut Q2GameServices),
    /// Emit an entity event.
    pub emit: fn(&Q2GameServices, Q2MissionPackEntityEvent),
}

/// Session entity hooks.
pub fn mission_entity_hooks(game: &Q2GameServices) -> Q2MissionPackEntityHooks {
    game.mission_packs
        .entity_hooks
        .expect("Q2 mission-pack entities are not registered")
}
