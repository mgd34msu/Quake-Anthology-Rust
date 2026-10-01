//! Mission-pack monster state (`src/content/q2/missionpacks/monsters/state.ts`).

use std::collections::HashMap;

use qa_core::identity::{ActorId, SavedActorId};

use crate::q2::foundation::checkpoint::{restore_q2_actor, save_q2_actor};
use crate::q2::foundation::host::Q2GameServices;

/// Rogue follow-up move (`flyerNextMove`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum RogueFlyerNext {
    /// No follow-up.
    #[default]
    None,
    /// Run.
    Run,
}

/// Rogue monster state (`RogueMonsterState`).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct RogueMonsterState {
    /// Blocked.
    pub blocked: bool,
    /// Turret orientation.
    pub turret_orientation: f64,
    /// Healer.
    pub healer: Option<ActorId>,
    /// First bad medic.
    pub bad_medic1: Option<ActorId>,
    /// Second bad medic.
    pub bad_medic2: Option<ActorId>,
    /// Medic tries.
    pub medic_tries: i32,
    /// Chosen reinforcements.
    pub chosen_reinforcements: Vec<i32>,
    /// React-to-damage time.
    pub react_to_damage_time: f64,
    /// Summon strength.
    pub summon_strength: i32,
    /// Last player enemy.
    pub last_player_enemy: Option<ActorId>,
    /// Bad area.
    pub bad_area: Option<ActorId>,
    /// Good guy.
    pub good_guy: bool,
    /// Widow quad expiry.
    pub widow_quad_until: f64,
    /// Widow double expiry.
    pub widow_double_until: f64,
    /// Widow invulnerability expiry.
    pub widow_invulnerable_until: f64,
}

/// Saved rogue monster state.
#[derive(Debug, Clone, PartialEq)]
pub struct SavedRogueMonsterState {
    /// Actor.
    pub actor: SavedActorId,
    /// Blocked.
    pub blocked: bool,
    /// Turret orientation.
    pub turret_orientation: f64,
    /// Healer.
    pub healer: Option<SavedActorId>,
    /// First bad medic.
    pub bad_medic1: Option<SavedActorId>,
    /// Second bad medic.
    pub bad_medic2: Option<SavedActorId>,
    /// Medic tries.
    pub medic_tries: i32,
    /// Chosen reinforcements.
    pub chosen_reinforcements: Vec<i32>,
    /// React-to-damage time.
    pub react_to_damage_time: f64,
    /// Summon strength.
    pub summon_strength: i32,
    /// Last player enemy.
    pub last_player_enemy: Option<SavedActorId>,
    /// Bad area.
    pub bad_area: Option<SavedActorId>,
    /// Good guy.
    pub good_guy: bool,
    /// Widow quad expiry.
    pub widow_quad_until: f64,
    /// Widow double expiry.
    pub widow_double_until: f64,
    /// Widow invulnerability expiry.
    pub widow_invulnerable_until: f64,
}

/// Mission-pack monsters checkpoint (`Q2MissionPackMonstersCheckpoint`).
#[derive(Debug, Clone, PartialEq)]
pub struct MissionPackMonstersCheckpoint {
    /// Version.
    pub version: i32,
    /// Rogue follow-up move.
    pub flyer_next_move: RogueFlyerNext,
    /// Widow shots fired.
    pub widow_shots_fired: i32,
    /// Widow damage multiplier.
    pub widow_damage_multiplier: u8,
    /// Rogue hint-path snapshot (the donor capture always records null).
    pub hints: Option<super::hints::RogueHintsCheckpoint>,
    /// Actor states.
    pub actors: Vec<SavedRogueMonsterState>,
}

/// Require rogue state for an actor (`Q2MissionPackMonsterState.get`).
pub fn rogue_state<'a>(game: &'a mut Q2GameServices, actor: &ActorId) -> &'a mut RogueMonsterState {
    game.mission_monsters.rogue.entry(actor.clone()).or_default()
}

/// Capture mission-pack monster state (`capture`).
pub fn capture_mission_monsters(game: &mut Q2GameServices) -> MissionPackMonstersCheckpoint {
    let mut actors = Vec::new();
    let mut live: Vec<(ActorId, RogueMonsterState)> = game
        .mission_monsters
        .rogue
        .iter()
        .filter(|(actor, _)| game.host.actors().is_live(actor))
        .map(|(actor, state)| (actor.clone(), state.clone()))
        .collect();
    live.sort_by(|a, b| (a.0.slot(), a.0.generation()).cmp(&(b.0.slot(), b.0.generation())));
    for (actor, state) in live {
        let Some(saved) = save_q2_actor(Some(&actor)) else {
            continue;
        };
        actors.push(SavedRogueMonsterState {
            actor: saved,
            blocked: state.blocked,
            turret_orientation: state.turret_orientation,
            healer: save_q2_actor(state.healer.as_ref()),
            bad_medic1: save_q2_actor(state.bad_medic1.as_ref()),
            bad_medic2: save_q2_actor(state.bad_medic2.as_ref()),
            medic_tries: state.medic_tries,
            chosen_reinforcements: state.chosen_reinforcements.clone(),
            react_to_damage_time: state.react_to_damage_time,
            summon_strength: state.summon_strength,
            last_player_enemy: save_q2_actor(state.last_player_enemy.as_ref()),
            bad_area: save_q2_actor(state.bad_area.as_ref()),
            good_guy: state.good_guy,
            widow_quad_until: state.widow_quad_until,
            widow_double_until: state.widow_double_until,
            widow_invulnerable_until: state.widow_invulnerable_until,
        });
    }
    MissionPackMonstersCheckpoint {
        version: 1,
        flyer_next_move: game.mission_monsters.flyer_next_move,
        widow_shots_fired: game.mission_monsters.widow_shots_fired,
        widow_damage_multiplier: game.mission_monsters.widow_damage_multiplier,
        hints: None,
        actors,
    }
}

/// Restore mission-pack monster state (`restore`).
pub fn restore_mission_monsters(game: &mut Q2GameServices, checkpoint: &MissionPackMonstersCheckpoint) {
    game.mission_monsters.rogue = HashMap::new();
    game.mission_monsters.flyer_next_move = checkpoint.flyer_next_move;
    game.mission_monsters.widow_shots_fired = checkpoint.widow_shots_fired;
    game.mission_monsters.widow_damage_multiplier = checkpoint.widow_damage_multiplier;
    for saved in &checkpoint.actors {
        let actor = restore_q2_actor(game, saved.actor.clone()).id().clone();
        let healer = saved.healer.clone().map(|id| game.host.actors().reference_saved(id));
        let bad_medic1 = saved
            .bad_medic1
            .clone()
            .map(|id| game.host.actors().reference_saved(id));
        let bad_medic2 = saved
            .bad_medic2
            .clone()
            .map(|id| game.host.actors().reference_saved(id));
        let last_player_enemy = saved
            .last_player_enemy
            .clone()
            .map(|id| game.host.actors().reference_saved(id));
        let bad_area = saved.bad_area.clone().map(|id| game.host.actors().reference_saved(id));
        game.mission_monsters.rogue.insert(
            actor,
            RogueMonsterState {
                blocked: saved.blocked,
                turret_orientation: saved.turret_orientation,
                healer,
                bad_medic1,
                bad_medic2,
                medic_tries: saved.medic_tries,
                chosen_reinforcements: saved.chosen_reinforcements.clone(),
                react_to_damage_time: saved.react_to_damage_time,
                summon_strength: saved.summon_strength,
                last_player_enemy,
                bad_area,
                good_guy: saved.good_guy,
                widow_quad_until: saved.widow_quad_until,
                widow_double_until: saved.widow_double_until,
                widow_invulnerable_until: saved.widow_invulnerable_until,
            },
        );
    }
}
