//! Quake II mission-pack checkpoints ported from `src/persistence/q2-missionpacks.ts`.
//!
//! Rogue/Xatrix item and monster state, rogue entities, and the tag and
//! deathball mode checkpoints. Rogue hint paths decode through a
//! structural port of the donor hints checkpoint (chain indexes and the
//! 100-chain bound are enforced).

use qa_core::identity::SavedActorId;
use qa_world::save::records::{read_saved_actor, write_saved_actor};
use qa_world::save::value::{
    arr, boolean, decode_checkpoint_value, encode_checkpoint_value, int, num, obj, str, SaveJson, SaveReader,
};

use super::super::PersistenceError;

/// Mission-pack power timing.
#[derive(Debug, Clone, PartialEq)]
pub struct Q2MissionPackPower {
    /// Actor.
    pub actor: SavedActorId,
    /// Quad-fire expiry.
    pub quad_fire_until: f64,
    /// Double expiry.
    pub double_until: f64,
    /// IR expiry.
    pub ir_until: f64,
}

/// Mission-pack items checkpoint.
#[derive(Debug, Clone, PartialEq)]
pub struct Q2MissionPackItemsCheckpoint {
    /// Powers.
    pub powers: Vec<Q2MissionPackPower>,
}

/// Read a mission-pack items checkpoint.
pub fn read_q2_mission_pack_items_checkpoint(
    reader: SaveReader,
) -> Result<Q2MissionPackItemsCheckpoint, PersistenceError> {
    Ok(Q2MissionPackItemsCheckpoint {
        powers: reader
            .field("powers")
            .list(|value| -> Result<Q2MissionPackPower, PersistenceError> {
                let state = value.field("state");
                Ok(Q2MissionPackPower {
                    actor: read_saved_actor(value.field("actor"))?,
                    quad_fire_until: state.field("quadFireUntil").number()?,
                    double_until: state.field("doubleUntil").number()?,
                    ir_until: state.field("irUntil").number()?,
                })
            })?,
    })
}

/// Write a mission-pack items checkpoint.
#[must_use]
pub fn write_q2_mission_pack_items_checkpoint(checkpoint: &Q2MissionPackItemsCheckpoint) -> SaveJson {
    obj(vec![(
        "powers",
        arr(checkpoint
            .powers
            .iter()
            .map(|power| {
                obj(vec![
                    ("actor", write_saved_actor(power.actor)),
                    (
                        "state",
                        obj(vec![
                            ("quadFireUntil", num(power.quad_fire_until)),
                            ("doubleUntil", num(power.double_until)),
                            ("irUntil", num(power.ir_until)),
                        ]),
                    ),
                ])
            })
            .collect()),
    )])
}

/// Encode a mission-pack items checkpoint.
#[must_use]
pub fn encode_q2_mission_pack_items_checkpoint(checkpoint: &Q2MissionPackItemsCheckpoint) -> Vec<u8> {
    encode_checkpoint_value(&write_q2_mission_pack_items_checkpoint(checkpoint))
}

/// Decode a mission-pack items checkpoint.
pub fn decode_q2_mission_pack_items_checkpoint(bytes: &[u8]) -> Result<Q2MissionPackItemsCheckpoint, PersistenceError> {
    let payload = decode_checkpoint_value(bytes)?;
    read_q2_mission_pack_items_checkpoint(SaveReader::at(&payload, "q2-missionpack-items"))
}

/// Rogue hint node.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Q2HintNode {
    /// Actor.
    pub actor: SavedActorId,
    /// Chain index (-1 when unlinked).
    pub chain: i64,
    /// Next node.
    pub next: Option<SavedActorId>,
}

/// Rogue hint pursuer.
#[derive(Debug, Clone, PartialEq)]
pub struct Q2HintPursuer {
    /// Actor.
    pub actor: SavedActorId,
    /// Goal.
    pub goal: Option<SavedActorId>,
    /// Last time.
    pub last_time: f64,
}

/// Rogue hints checkpoint.
#[derive(Debug, Clone, PartialEq)]
pub struct Q2RogueHintsCheckpoint {
    /// Present flag.
    pub present: bool,
    /// Chain starts.
    pub starts: Vec<SavedActorId>,
    /// Nodes.
    pub nodes: Vec<Q2HintNode>,
    /// Pursuers.
    pub monsters: Vec<Q2HintPursuer>,
}

/// Maximum hint chains.
pub const MAX_HINT_CHAINS: usize = 100;

/// Read a rogue hints checkpoint.
pub fn read_q2_rogue_hints_checkpoint(reader: SaveReader) -> Result<Q2RogueHintsCheckpoint, PersistenceError> {
    reader.field("version").literal_i64(1)?;
    let checkpoint = Q2RogueHintsCheckpoint {
        present: reader.field("present").boolean()?,
        starts: reader
            .field("starts")
            .list(|value| read_saved_actor(value).map_err(PersistenceError::from))?,
        nodes: reader
            .field("nodes")
            .list(|node| -> Result<Q2HintNode, PersistenceError> {
                Ok(Q2HintNode {
                    actor: read_saved_actor(node.field("actor"))?,
                    chain: node.field("chain").integer(-1)?,
                    next: node
                        .field("next")
                        .nullable(|value| read_saved_actor(value).map_err(PersistenceError::from))?,
                })
            })?,
        monsters: reader
            .field("monsters")
            .list(|monster| -> Result<Q2HintPursuer, PersistenceError> {
                Ok(Q2HintPursuer {
                    actor: read_saved_actor(monster.field("actor"))?,
                    goal: monster
                        .field("goal")
                        .nullable(|value| read_saved_actor(value).map_err(PersistenceError::from))?,
                    last_time: monster.field("lastTime").finite()?,
                })
            })?,
    };
    if checkpoint.starts.len() > MAX_HINT_CHAINS
        || checkpoint
            .nodes
            .iter()
            .any(|node| node.chain >= checkpoint.starts.len() as i64)
    {
        return Err(PersistenceError::from(reader.fail("invalid hint chain index")));
    }
    Ok(checkpoint)
}

/// Write a rogue hints checkpoint.
#[must_use]
pub fn write_q2_rogue_hints_checkpoint(checkpoint: &Q2RogueHintsCheckpoint) -> SaveJson {
    obj(vec![
        ("version", int(1)),
        ("present", boolean(checkpoint.present)),
        (
            "starts",
            arr(checkpoint
                .starts
                .iter()
                .map(|actor| write_saved_actor(*actor))
                .collect()),
        ),
        (
            "nodes",
            arr(checkpoint
                .nodes
                .iter()
                .map(|node| {
                    obj(vec![
                        ("actor", write_saved_actor(node.actor)),
                        ("chain", int(node.chain)),
                        ("next", node.next.map_or(SaveJson::Null, write_saved_actor)),
                    ])
                })
                .collect()),
        ),
        (
            "monsters",
            arr(checkpoint
                .monsters
                .iter()
                .map(|monster| {
                    obj(vec![
                        ("actor", write_saved_actor(monster.actor)),
                        ("goal", monster.goal.map_or(SaveJson::Null, write_saved_actor)),
                        ("lastTime", num(monster.last_time)),
                    ])
                })
                .collect()),
        ),
    ])
}

/// Mission-pack monster state.
#[derive(Debug, Clone, PartialEq)]
pub struct Q2MissionPackMonsterState {
    /// Blocked flag.
    pub blocked: bool,
    /// Turret orientation.
    pub turret_orientation: f64,
    /// Healer.
    pub healer: Option<SavedActorId>,
    /// Bad medic 1.
    pub bad_medic1: Option<SavedActorId>,
    /// Bad medic 2.
    pub bad_medic2: Option<SavedActorId>,
    /// Medic tries.
    pub medic_tries: f64,
    /// Chosen reinforcements.
    pub chosen_reinforcements: Vec<u64>,
    /// React to damage time.
    pub react_to_damage_time: f64,
    /// Summon strength.
    pub summon_strength: f64,
    /// Last player enemy.
    pub last_player_enemy: Option<SavedActorId>,
    /// Bad area.
    pub bad_area: Option<SavedActorId>,
    /// Good guy flag.
    pub good_guy: bool,
    /// Widow quad expiry.
    pub widow_quad_until: f64,
    /// Widow double expiry.
    pub widow_double_until: f64,
    /// Widow invulnerability expiry.
    pub widow_invulnerable_until: f64,
}

/// Mission-pack monsters checkpoint.
#[derive(Debug, Clone, PartialEq)]
pub struct Q2MissionPackMonstersCheckpoint {
    /// Flyer next move.
    pub flyer_next_move: String,
    /// Rogue hints.
    pub hints: Option<Q2RogueHintsCheckpoint>,
    /// Widow shots fired.
    pub widow_shots_fired: f64,
    /// Widow damage multiplier.
    pub widow_damage_multiplier: i64,
    /// Actors.
    pub actors: Vec<(SavedActorId, Q2MissionPackMonsterState)>,
}

/// Read a mission-pack monsters checkpoint.
#[allow(clippy::cast_sign_loss)]
pub fn read_q2_mission_pack_monsters_checkpoint(
    reader: SaveReader,
) -> Result<Q2MissionPackMonstersCheckpoint, PersistenceError> {
    reader.field("version").literal_i64(1)?;
    Ok(Q2MissionPackMonstersCheckpoint {
        flyer_next_move: reader.field("flyerNextMove").choice_str(&["none", "run"])?,
        hints: reader.field("hints").nullable(read_q2_rogue_hints_checkpoint)?,
        widow_shots_fired: reader.field("widowShotsFired").number()?,
        widow_damage_multiplier: reader.field("widowDamageMultiplier").choice_i64(&[1, 2, 4])?,
        actors: reader.field("actors").list(
            |value| -> Result<(SavedActorId, Q2MissionPackMonsterState), PersistenceError> {
                let state = value.field("state");
                let saved = |name: &str| {
                    state
                        .field(name)
                        .nullable(|item| read_saved_actor(item).map_err(PersistenceError::from))
                };
                Ok((
                    read_saved_actor(value.field("actor"))?,
                    Q2MissionPackMonsterState {
                        blocked: state.field("blocked").boolean()?,
                        turret_orientation: state.field("turretOrientation").number()?,
                        healer: saved("healer")?,
                        bad_medic1: saved("badMedic1")?,
                        bad_medic2: saved("badMedic2")?,
                        medic_tries: state.field("medicTries").number()?,
                        chosen_reinforcements: state.field("chosenReinforcements").list(
                            |item| -> Result<u64, PersistenceError> {
                                let index = item.integer(0)?;
                                if index >= 255 {
                                    return Err(PersistenceError::from(
                                        item.fail("expected a reinforcement index below 255"),
                                    ));
                                }
                                Ok(index as u64)
                            },
                        )?,
                        react_to_damage_time: state.field("reactToDamageTime").finite()?,
                        summon_strength: state.field("summonStrength").number()?,
                        last_player_enemy: saved("lastPlayerEnemy")?,
                        bad_area: saved("badArea")?,
                        good_guy: state.field("goodGuy").boolean()?,
                        widow_quad_until: state.field("widowQuadUntil").number()?,
                        widow_double_until: state.field("widowDoubleUntil").number()?,
                        widow_invulnerable_until: state.field("widowInvulnerableUntil").number()?,
                    },
                ))
            },
        )?,
    })
}

/// Write a mission-pack monsters checkpoint.
#[must_use]
#[allow(clippy::cast_possible_wrap)]
pub fn write_q2_mission_pack_monsters_checkpoint(checkpoint: &Q2MissionPackMonstersCheckpoint) -> SaveJson {
    obj(vec![
        ("version", int(1)),
        ("flyerNextMove", str(&checkpoint.flyer_next_move)),
        (
            "hints",
            checkpoint
                .hints
                .as_ref()
                .map_or(SaveJson::Null, write_q2_rogue_hints_checkpoint),
        ),
        ("widowShotsFired", num(checkpoint.widow_shots_fired)),
        ("widowDamageMultiplier", int(checkpoint.widow_damage_multiplier)),
        (
            "actors",
            arr(checkpoint
                .actors
                .iter()
                .map(|(actor, state)| {
                    obj(vec![
                        ("actor", write_saved_actor(*actor)),
                        (
                            "state",
                            obj(vec![
                                ("blocked", boolean(state.blocked)),
                                ("turretOrientation", num(state.turret_orientation)),
                                ("healer", state.healer.map_or(SaveJson::Null, write_saved_actor)),
                                ("badMedic1", state.bad_medic1.map_or(SaveJson::Null, write_saved_actor)),
                                ("badMedic2", state.bad_medic2.map_or(SaveJson::Null, write_saved_actor)),
                                ("medicTries", num(state.medic_tries)),
                                (
                                    "chosenReinforcements",
                                    arr(state
                                        .chosen_reinforcements
                                        .iter()
                                        .map(|index| int(*index as i64))
                                        .collect()),
                                ),
                                ("reactToDamageTime", num(state.react_to_damage_time)),
                                ("summonStrength", num(state.summon_strength)),
                                (
                                    "lastPlayerEnemy",
                                    state.last_player_enemy.map_or(SaveJson::Null, write_saved_actor),
                                ),
                                ("badArea", state.bad_area.map_or(SaveJson::Null, write_saved_actor)),
                                ("goodGuy", boolean(state.good_guy)),
                                ("widowQuadUntil", num(state.widow_quad_until)),
                                ("widowDoubleUntil", num(state.widow_double_until)),
                                ("widowInvulnerableUntil", num(state.widow_invulnerable_until)),
                            ]),
                        ),
                    ])
                })
                .collect()),
        ),
    ])
}

/// Encode a mission-pack monsters checkpoint.
#[must_use]
pub fn encode_q2_mission_pack_monsters_checkpoint(checkpoint: &Q2MissionPackMonstersCheckpoint) -> Vec<u8> {
    encode_checkpoint_value(&write_q2_mission_pack_monsters_checkpoint(checkpoint))
}

/// Decode a mission-pack monsters checkpoint.
pub fn decode_q2_mission_pack_monsters_checkpoint(
    bytes: &[u8],
) -> Result<Q2MissionPackMonstersCheckpoint, PersistenceError> {
    let payload = decode_checkpoint_value(bytes)?;
    read_q2_mission_pack_monsters_checkpoint(SaveReader::at(&payload, "q2-missionpack-monsters"))
}

/// Rogue entities checkpoint.
#[derive(Debug, Clone, PartialEq)]
pub struct Q2RogueEntitiesCheckpoint {
    /// Steam id.
    pub steam_id: f64,
}

/// Read a rogue entities checkpoint.
pub fn read_q2_rogue_entities_checkpoint(reader: SaveReader) -> Result<Q2RogueEntitiesCheckpoint, PersistenceError> {
    Ok(Q2RogueEntitiesCheckpoint {
        steam_id: reader.field("steamId").number()?,
    })
}

/// Write a rogue entities checkpoint.
#[must_use]
pub fn write_q2_rogue_entities_checkpoint(checkpoint: &Q2RogueEntitiesCheckpoint) -> SaveJson {
    obj(vec![("steamId", num(checkpoint.steam_id))])
}

/// Encode a rogue entities checkpoint.
#[must_use]
pub fn encode_q2_rogue_entities_checkpoint(checkpoint: &Q2RogueEntitiesCheckpoint) -> Vec<u8> {
    encode_checkpoint_value(&write_q2_rogue_entities_checkpoint(checkpoint))
}

/// Decode a rogue entities checkpoint.
pub fn decode_q2_rogue_entities_checkpoint(bytes: &[u8]) -> Result<Q2RogueEntitiesCheckpoint, PersistenceError> {
    let payload = decode_checkpoint_value(bytes)?;
    read_q2_rogue_entities_checkpoint(SaveReader::at(&payload, "q2-rogue-entities"))
}

/// Tag checkpoint.
#[derive(Debug, Clone, PartialEq)]
pub struct Q2TagCheckpoint {
    /// Token.
    pub token: Option<SavedActorId>,
    /// Owner.
    pub owner: Option<SavedActorId>,
    /// Count.
    pub count: f64,
}

/// Read a tag checkpoint.
pub fn read_q2_tag_checkpoint(reader: SaveReader) -> Result<Q2TagCheckpoint, PersistenceError> {
    Ok(Q2TagCheckpoint {
        token: reader
            .field("token")
            .nullable(|value| read_saved_actor(value).map_err(PersistenceError::from))?,
        owner: reader
            .field("owner")
            .nullable(|value| read_saved_actor(value).map_err(PersistenceError::from))?,
        count: reader.field("count").number()?,
    })
}

/// Write a tag checkpoint.
#[must_use]
pub fn write_q2_tag_checkpoint(checkpoint: &Q2TagCheckpoint) -> SaveJson {
    obj(vec![
        ("token", checkpoint.token.map_or(SaveJson::Null, write_saved_actor)),
        ("owner", checkpoint.owner.map_or(SaveJson::Null, write_saved_actor)),
        ("count", num(checkpoint.count)),
    ])
}

/// Encode a tag checkpoint.
#[must_use]
pub fn encode_q2_tag_checkpoint(checkpoint: &Q2TagCheckpoint) -> Vec<u8> {
    encode_checkpoint_value(&write_q2_tag_checkpoint(checkpoint))
}

/// Decode a tag checkpoint.
pub fn decode_q2_tag_checkpoint(bytes: &[u8]) -> Result<Q2TagCheckpoint, PersistenceError> {
    let payload = decode_checkpoint_value(bytes)?;
    read_q2_tag_checkpoint(SaveReader::at(&payload, "q2-tag"))
}

/// Deathball checkpoint.
#[derive(Debug, Clone, PartialEq)]
pub struct Q2DeathBallCheckpoint {
    /// Ball.
    pub ball: Option<SavedActorId>,
    /// Starts.
    pub starts: f64,
    /// Team 1 score.
    pub team1_score: f64,
    /// Team 2 score.
    pub team2_score: f64,
}

/// Read a deathball checkpoint.
pub fn read_q2_death_ball_checkpoint(reader: SaveReader) -> Result<Q2DeathBallCheckpoint, PersistenceError> {
    Ok(Q2DeathBallCheckpoint {
        ball: reader
            .field("ball")
            .nullable(|value| read_saved_actor(value).map_err(PersistenceError::from))?,
        starts: reader.field("starts").number()?,
        team1_score: reader.field("team1Score").number()?,
        team2_score: reader.field("team2Score").number()?,
    })
}

/// Write a deathball checkpoint.
#[must_use]
pub fn write_q2_death_ball_checkpoint(checkpoint: &Q2DeathBallCheckpoint) -> SaveJson {
    obj(vec![
        ("ball", checkpoint.ball.map_or(SaveJson::Null, write_saved_actor)),
        ("starts", num(checkpoint.starts)),
        ("team1Score", num(checkpoint.team1_score)),
        ("team2Score", num(checkpoint.team2_score)),
    ])
}

/// Encode a deathball checkpoint.
#[must_use]
pub fn encode_q2_death_ball_checkpoint(checkpoint: &Q2DeathBallCheckpoint) -> Vec<u8> {
    encode_checkpoint_value(&write_q2_death_ball_checkpoint(checkpoint))
}

/// Decode a deathball checkpoint.
pub fn decode_q2_death_ball_checkpoint(bytes: &[u8]) -> Result<Q2DeathBallCheckpoint, PersistenceError> {
    let payload = decode_checkpoint_value(bytes)?;
    read_q2_death_ball_checkpoint(SaveReader::at(&payload, "q2-deathball"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mission_checkpoints_round_trip() {
        let items = Q2MissionPackItemsCheckpoint {
            powers: vec![Q2MissionPackPower {
                actor: SavedActorId { slot: 1, generation: 0 },
                quad_fire_until: 5.0,
                double_until: 0.0,
                ir_until: 0.0,
            }],
        };
        assert_eq!(
            decode_q2_mission_pack_items_checkpoint(&encode_q2_mission_pack_items_checkpoint(&items)).unwrap(),
            items
        );
        let monsters = Q2MissionPackMonstersCheckpoint {
            flyer_next_move: "none".to_string(),
            hints: Some(Q2RogueHintsCheckpoint {
                present: true,
                starts: vec![SavedActorId { slot: 1, generation: 0 }],
                nodes: vec![Q2HintNode {
                    actor: SavedActorId { slot: 1, generation: 0 },
                    chain: 0,
                    next: None,
                }],
                monsters: Vec::new(),
            }),
            widow_shots_fired: 0.0,
            widow_damage_multiplier: 2,
            actors: vec![(
                SavedActorId { slot: 2, generation: 0 },
                Q2MissionPackMonsterState {
                    blocked: false,
                    turret_orientation: 0.0,
                    healer: None,
                    bad_medic1: None,
                    bad_medic2: None,
                    medic_tries: 0.0,
                    chosen_reinforcements: vec![1, 2],
                    react_to_damage_time: 0.0,
                    summon_strength: 0.0,
                    last_player_enemy: None,
                    bad_area: None,
                    good_guy: false,
                    widow_quad_until: 0.0,
                    widow_double_until: 0.0,
                    widow_invulnerable_until: 0.0,
                },
            )],
        };
        assert_eq!(
            decode_q2_mission_pack_monsters_checkpoint(&encode_q2_mission_pack_monsters_checkpoint(&monsters)).unwrap(),
            monsters
        );
        let rogue = Q2RogueEntitiesCheckpoint { steam_id: 3.0 };
        assert_eq!(
            decode_q2_rogue_entities_checkpoint(&encode_q2_rogue_entities_checkpoint(&rogue)).unwrap(),
            rogue
        );
        let tag = Q2TagCheckpoint {
            token: None,
            owner: None,
            count: 1.0,
        };
        assert_eq!(decode_q2_tag_checkpoint(&encode_q2_tag_checkpoint(&tag)).unwrap(), tag);
        let ball = Q2DeathBallCheckpoint {
            ball: None,
            starts: 0.0,
            team1_score: 1.0,
            team2_score: 2.0,
        };
        assert_eq!(
            decode_q2_death_ball_checkpoint(&encode_q2_death_ball_checkpoint(&ball)).unwrap(),
            ball
        );
    }

    #[test]
    fn hint_chain_bounds_hold() {
        let bad = obj(vec![
            ("version", int(1)),
            ("present", boolean(true)),
            ("starts", arr(Vec::new())),
            (
                "nodes",
                arr(vec![obj(vec![
                    (
                        "actor",
                        SaveJson::Object(vec![("slot".to_string(), int(0)), ("generation".to_string(), int(0))]),
                    ),
                    ("chain", int(0)),
                    ("next", SaveJson::Null),
                ])]),
            ),
            ("monsters", arr(Vec::new())),
        ]);
        assert!(read_q2_rogue_hints_checkpoint(SaveReader::new(&bad)).is_err());
    }
}
