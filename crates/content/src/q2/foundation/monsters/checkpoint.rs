//! Monster save state (`src/content/q2/foundation/monsters/checkpoint.ts`).

use qa_core::identity::{ActorId, SavedActorId};
use qa_core::math::Vec3;

use super::types::MonsterSoundTarget;
use crate::q2::foundation::checkpoint::Q2AttackCheckpoint;
use crate::q2::support::contracts::DeathReaction;

/// Saved monster state (`Q2MonsterStateCheckpoint`).
#[derive(Debug, Clone)]
pub struct Q2MonsterStateCheckpoint {
    /// Move name.
    pub movement: String,
    /// Next move name.
    pub next_move: Option<String>,
    /// Sound target.
    pub sound_target: Option<MonsterSoundTarget>,
    /// Old enemy.
    pub old_enemy: Option<SavedActorId>,
    /// Move target.
    pub move_target: Option<SavedActorId>,
    /// Commander.
    pub commander: Option<SavedActorId>,
    /// Remaining state fields.
    pub state: super::types::MonsterState,
}

/// Saved pending monster damage (`Q2MonsterDamageCheckpoint`).
#[derive(Debug, Clone)]
pub struct Q2MonsterDamageCheckpoint {
    /// Death reaction with saved actors.
    pub reaction: DeathReaction,
    /// Saved attacker.
    pub attacker: Option<SavedActorId>,
    /// Saved inflictor.
    pub inflictor: Option<SavedActorId>,
    /// Saved attack.
    pub attack: Option<Q2AttackCheckpoint>,
}

/// Saved sighting.
#[derive(Debug, Clone)]
pub struct SavedSighting {
    /// Sighted actor.
    pub actor: SavedActorId,
    /// Sight time.
    pub time: f64,
}

/// Saved noise.
#[derive(Debug, Clone)]
pub struct SavedNoise {
    /// Noise actor.
    pub actor: SavedActorId,
    /// Noise time.
    pub time: f64,
    /// Noise owner.
    pub owner: SavedActorId,
    /// Noise origin.
    pub origin: Vec3,
}

/// Saved perception (`Q2MonsterPerceptionCheckpoint`).
#[derive(Debug, Clone)]
pub struct Q2MonsterPerceptionCheckpoint {
    /// Sight client.
    pub sight_client: Option<SavedActorId>,
    /// Last sight.
    pub sight: Option<SavedSighting>,
    /// Alerted actors.
    pub alerted: Vec<(SavedActorId, SavedSighting)>,
    /// Primary noise.
    pub primary: Option<SavedNoise>,
    /// Secondary noise.
    pub secondary: Option<SavedNoise>,
    /// Noise pairs by owner.
    pub noises: Vec<(SavedActorId, SavedActorId, SavedActorId)>,
    /// Player trails.
    pub trails: Vec<(SavedActorId, Vec<super::perception::TrailPoint>)>,
    /// Player origins.
    pub player_origins: Vec<(SavedActorId, Vec3)>,
    /// Hostile timestamps.
    pub hostile: Vec<(SavedActorId, f64)>,
    /// Last frame time.
    pub last_frame: Option<f64>,
}

/// Saved monster actor.
#[derive(Debug, Clone)]
pub struct Q2MonsterActorCheckpoint {
    /// Actor.
    pub actor: SavedActorId,
    /// Definition classname.
    pub definition: String,
    /// Monster state.
    pub state: Q2MonsterStateCheckpoint,
    /// Pending damage.
    pub pending_damage: Option<Q2MonsterDamageCheckpoint>,
}

/// Saved monsters (`Q2MonstersCheckpoint`).
#[derive(Debug, Clone)]
pub struct Q2MonstersCheckpoint {
    /// Checkpoint version.
    pub version: u32,
    /// Monster actors.
    pub actors: Vec<Q2MonsterActorCheckpoint>,
    /// Perception.
    pub perception: Q2MonsterPerceptionCheckpoint,
}

/// Save an actor id (`saveQ2Actor` re-export for monster checkpoints).
pub fn save_actor(actor: Option<&ActorId>) -> Option<SavedActorId> {
    crate::q2::foundation::checkpoint::save_q2_actor(actor)
}
