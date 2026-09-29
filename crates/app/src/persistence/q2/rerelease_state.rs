//! Quake II rerelease state checkpoints ported from `src/persistence/q2-rerelease-state.ts`.

use qa_core::identity::SavedActorId;
use qa_core::math::Vec3;
use qa_world::save::records::{read_saved_actor, write_saved_actor};
use qa_world::save::shared::{read_vector, write_vector};
use qa_world::save::value::{
    arr, boolean, decode_checkpoint_value, encode_checkpoint_value, int, num, obj, str, SaveJson, SaveReader,
};

use super::super::PersistenceError;

/// Rerelease player module state.
#[derive(Debug, Clone, PartialEq)]
pub struct Q2RereleasePlayerModule {
    /// Invisible total.
    pub invisible_total: f64,
    /// Invisible maximum.
    pub invisible_maximum: f64,
    /// Explosion time.
    pub explosion_time: f64,
    /// Explosion frames.
    pub explosion_frames: Vec<f64>,
    /// Explosion count.
    pub explosion_count: f64,
    /// Explosion base.
    pub explosion_base: Vec3,
    /// Explosion angles.
    pub explosion_angles: Vec3,
    /// Explosion damage.
    pub explosion_damage: f64,
    /// Explosion radius.
    pub explosion_radius: f64,
    /// Regeneration pool.
    pub regeneration_pool: f64,
}

/// Read a rerelease player module checkpoint.
pub fn read_q2_rerelease_player_module_checkpoint(
    reader: SaveReader,
) -> Result<Vec<(SavedActorId, Q2RereleasePlayerModule)>, PersistenceError> {
    reader.field("actors").list(
        |value| -> Result<(SavedActorId, Q2RereleasePlayerModule), PersistenceError> {
            let state = value.field("state");
            Ok((
                read_saved_actor(value.field("actor"))?,
                Q2RereleasePlayerModule {
                    invisible_total: state.field("invisibleTotal").number()?,
                    invisible_maximum: state.field("invisibleMaximum").number()?,
                    explosion_time: state.field("explosionTime").number()?,
                    explosion_frames: state
                        .field("explosionFrames")
                        .list(|frame| frame.number().map_err(PersistenceError::from))?,
                    explosion_count: state.field("explosionCount").number()?,
                    explosion_base: read_vector(state.field("explosionBase"))?,
                    explosion_angles: read_vector(state.field("explosionAngles"))?,
                    explosion_damage: state.field("explosionDamage").number()?,
                    explosion_radius: state.field("explosionRadius").number()?,
                    regeneration_pool: state.field("regenerationPool").number()?,
                },
            ))
        },
    )
}

/// Write a rerelease player module checkpoint.
#[must_use]
pub fn write_q2_rerelease_player_module_checkpoint(entries: &[(SavedActorId, Q2RereleasePlayerModule)]) -> SaveJson {
    obj(vec![(
        "actors",
        arr(entries
            .iter()
            .map(|(actor, state)| {
                obj(vec![
                    ("actor", write_saved_actor(*actor)),
                    (
                        "state",
                        obj(vec![
                            ("invisibleTotal", num(state.invisible_total)),
                            ("invisibleMaximum", num(state.invisible_maximum)),
                            ("explosionTime", num(state.explosion_time)),
                            (
                                "explosionFrames",
                                arr(state.explosion_frames.iter().map(|frame| num(*frame)).collect()),
                            ),
                            ("explosionCount", num(state.explosion_count)),
                            ("explosionBase", write_vector(state.explosion_base)),
                            ("explosionAngles", write_vector(state.explosion_angles)),
                            ("explosionDamage", num(state.explosion_damage)),
                            ("explosionRadius", num(state.explosion_radius)),
                            ("regenerationPool", num(state.regeneration_pool)),
                        ]),
                    ),
                ])
            })
            .collect()),
    )])
}

/// Q64 checkpoint.
#[derive(Debug, Clone, PartialEq)]
pub struct Q2Q64Checkpoint {
    /// Frag counts.
    pub frag_counts: Vec<(SavedActorId, f64)>,
    /// Team sizes.
    pub team_sizes: Vec<f64>,
    /// Ghosts.
    pub ghosts: Vec<(SavedActorId, SavedActorId)>,
    /// Slowprint.
    pub slowprint: String,
    /// Emotes.
    pub emotes: Vec<(SavedActorId, f64)>,
}

/// Read a Q64 checkpoint.
pub fn read_q2_q64_checkpoint(reader: SaveReader) -> Result<Q2Q64Checkpoint, PersistenceError> {
    reader.field("version").literal_i64(1)?;
    Ok(Q2Q64Checkpoint {
        frag_counts: reader
            .field("fragCounts")
            .list(|value| -> Result<(SavedActorId, f64), PersistenceError> {
                Ok((read_saved_actor(value.field("actor"))?, value.field("count").number()?))
            })?,
        team_sizes: reader
            .field("teamSizes")
            .list(|value| value.number().map_err(PersistenceError::from))?,
        ghosts: reader
            .field("ghosts")
            .list(|value| -> Result<(SavedActorId, SavedActorId), PersistenceError> {
                Ok((
                    read_saved_actor(value.field("actor"))?,
                    read_saved_actor(value.field("owner"))?,
                ))
            })?,
        slowprint: reader.field("slowprint").string()?,
        emotes: reader
            .field("emotes")
            .list(|value| -> Result<(SavedActorId, f64), PersistenceError> {
                Ok((read_saved_actor(value.field("actor"))?, value.field("until").number()?))
            })?,
    })
}

/// Write a Q64 checkpoint.
#[must_use]
pub fn write_q2_q64_checkpoint(checkpoint: &Q2Q64Checkpoint) -> SaveJson {
    obj(vec![
        ("version", int(1)),
        (
            "fragCounts",
            arr(checkpoint
                .frag_counts
                .iter()
                .map(|(actor, count)| obj(vec![("actor", write_saved_actor(*actor)), ("count", num(*count))]))
                .collect()),
        ),
        (
            "teamSizes",
            arr(checkpoint.team_sizes.iter().map(|size| num(*size)).collect()),
        ),
        (
            "ghosts",
            arr(checkpoint
                .ghosts
                .iter()
                .map(|(actor, owner)| {
                    obj(vec![
                        ("actor", write_saved_actor(*actor)),
                        ("owner", write_saved_actor(*owner)),
                    ])
                })
                .collect()),
        ),
        ("slowprint", str(&checkpoint.slowprint)),
        (
            "emotes",
            arr(checkpoint
                .emotes
                .iter()
                .map(|(actor, until)| obj(vec![("actor", write_saved_actor(*actor)), ("until", num(*until))]))
                .collect()),
        ),
    ])
}

/// Encode a Q64 checkpoint.
#[must_use]
pub fn encode_q2_q64_checkpoint(checkpoint: &Q2Q64Checkpoint) -> Vec<u8> {
    encode_checkpoint_value(&write_q2_q64_checkpoint(checkpoint))
}

/// Decode a Q64 checkpoint.
pub fn decode_q2_q64_checkpoint(bytes: &[u8]) -> Result<Q2Q64Checkpoint, PersistenceError> {
    let payload = decode_checkpoint_value(bytes)?;
    read_q2_q64_checkpoint(SaveReader::at(&payload, "q2-q64"))
}

/// Team info checkpoint.
#[derive(Debug, Clone, PartialEq)]
pub struct Q2TeamInfoCheckpoint {
    /// Scores.
    pub scores: Vec<(f64, f64)>,
    /// Teamplay.
    pub teamplay: bool,
}

/// Read a team info checkpoint.
pub fn read_q2_team_info_checkpoint(reader: SaveReader) -> Result<Q2TeamInfoCheckpoint, PersistenceError> {
    reader.field("version").literal_i64(1)?;
    Ok(Q2TeamInfoCheckpoint {
        scores: reader
            .field("scores")
            .list(|value| -> Result<(f64, f64), PersistenceError> {
                Ok((value.field("wins").number()?, value.field("losses").number()?))
            })?,
        teamplay: reader.field("teamplay").boolean()?,
    })
}

/// Write a team info checkpoint.
#[must_use]
pub fn write_q2_team_info_checkpoint(checkpoint: &Q2TeamInfoCheckpoint) -> SaveJson {
    obj(vec![
        ("version", int(1)),
        (
            "scores",
            arr(checkpoint
                .scores
                .iter()
                .map(|(wins, losses)| obj(vec![("wins", num(*wins)), ("losses", num(*losses))]))
                .collect()),
        ),
        ("teamplay", boolean(checkpoint.teamplay)),
    ])
}

/// Encode a team info checkpoint.
#[must_use]
pub fn encode_q2_team_info_checkpoint(checkpoint: &Q2TeamInfoCheckpoint) -> Vec<u8> {
    encode_checkpoint_value(&write_q2_team_info_checkpoint(checkpoint))
}

/// Decode a team info checkpoint.
pub fn decode_q2_team_info_checkpoint(bytes: &[u8]) -> Result<Q2TeamInfoCheckpoint, PersistenceError> {
    let payload = decode_checkpoint_value(bytes)?;
    read_q2_team_info_checkpoint(SaveReader::at(&payload, "q2-teaminfo"))
}

/// Rerelease fog overlay checkpoint.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Q2RereleaseFogCheckpoint {
    /// Color.
    pub color: Option<Vec3>,
    /// Density.
    pub density: Option<f64>,
    /// Height falloff.
    pub height_falloff: Option<f64>,
    /// Height density.
    pub height_density: Option<f64>,
    /// Height start color.
    pub height_start_color: Option<Vec3>,
    /// Height start distance.
    pub height_start_distance: Option<f64>,
    /// Height end color.
    pub height_end_color: Option<Vec3>,
    /// Height end distance.
    pub height_end_distance: Option<f64>,
}

/// Read a rerelease fog overlay checkpoint.
pub fn read_q2_rerelease_fog_checkpoint(reader: SaveReader) -> Result<Q2RereleaseFogCheckpoint, PersistenceError> {
    Ok(Q2RereleaseFogCheckpoint {
        color: reader
            .field("color")
            .nullable(|value| read_vector(value).map_err(PersistenceError::from))?,
        density: reader
            .field("density")
            .nullable(|value| value.number().map_err(PersistenceError::from))?,
        height_falloff: reader
            .field("heightFalloff")
            .nullable(|value| value.number().map_err(PersistenceError::from))?,
        height_density: reader
            .field("heightDensity")
            .nullable(|value| value.number().map_err(PersistenceError::from))?,
        height_start_color: reader
            .field("heightStartColor")
            .nullable(|value| read_vector(value).map_err(PersistenceError::from))?,
        height_start_distance: reader
            .field("heightStartDistance")
            .nullable(|value| value.number().map_err(PersistenceError::from))?,
        height_end_color: reader
            .field("heightEndColor")
            .nullable(|value| read_vector(value).map_err(PersistenceError::from))?,
        height_end_distance: reader
            .field("heightEndDistance")
            .nullable(|value| value.number().map_err(PersistenceError::from))?,
    })
}

/// Write a rerelease fog overlay checkpoint.
#[must_use]
pub fn write_q2_rerelease_fog_checkpoint(checkpoint: &Q2RereleaseFogCheckpoint) -> SaveJson {
    obj(vec![
        ("color", checkpoint.color.map_or(SaveJson::Null, write_vector)),
        ("density", checkpoint.density.map_or(SaveJson::Null, num)),
        ("heightFalloff", checkpoint.height_falloff.map_or(SaveJson::Null, num)),
        ("heightDensity", checkpoint.height_density.map_or(SaveJson::Null, num)),
        (
            "heightStartColor",
            checkpoint.height_start_color.map_or(SaveJson::Null, write_vector),
        ),
        (
            "heightStartDistance",
            checkpoint.height_start_distance.map_or(SaveJson::Null, num),
        ),
        (
            "heightEndColor",
            checkpoint.height_end_color.map_or(SaveJson::Null, write_vector),
        ),
        (
            "heightEndDistance",
            checkpoint.height_end_distance.map_or(SaveJson::Null, num),
        ),
    ])
}

/// Encode a rerelease fog overlay checkpoint.
#[must_use]
pub fn encode_q2_rerelease_fog_checkpoint(checkpoint: &Q2RereleaseFogCheckpoint) -> Vec<u8> {
    encode_checkpoint_value(&write_q2_rerelease_fog_checkpoint(checkpoint))
}

/// Decode a rerelease fog overlay checkpoint.
pub fn decode_q2_rerelease_fog_checkpoint(bytes: &[u8]) -> Result<Q2RereleaseFogCheckpoint, PersistenceError> {
    let payload = decode_checkpoint_value(bytes)?;
    read_q2_rerelease_fog_checkpoint(SaveReader::at(&payload, "q2-rerelease-fog"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rerelease_state_round_trip() {
        let zero = Vec3 { x: 0.0, y: 0.0, z: 0.0 };
        let modules = vec![(
            SavedActorId { slot: 1, generation: 0 },
            Q2RereleasePlayerModule {
                invisible_total: 0.0,
                invisible_maximum: 30.0,
                explosion_time: 0.0,
                explosion_frames: vec![1.0, 2.0],
                explosion_count: 0.0,
                explosion_base: zero,
                explosion_angles: zero,
                explosion_damage: 120.0,
                explosion_radius: 160.0,
                regeneration_pool: 0.0,
            },
        )];
        let json = write_q2_rerelease_player_module_checkpoint(&modules);
        let bytes = encode_checkpoint_value(&json);
        let back = decode_checkpoint_value(&bytes).unwrap();
        assert_eq!(
            read_q2_rerelease_player_module_checkpoint(SaveReader::at(&back, "m")).unwrap(),
            modules
        );
        let q64 = Q2Q64Checkpoint {
            frag_counts: vec![(SavedActorId { slot: 1, generation: 0 }, 3.0)],
            team_sizes: vec![2.0, 2.0],
            ghosts: Vec::new(),
            slowprint: "welcome".to_string(),
            emotes: Vec::new(),
        };
        assert_eq!(decode_q2_q64_checkpoint(&encode_q2_q64_checkpoint(&q64)).unwrap(), q64);
        let team = Q2TeamInfoCheckpoint {
            scores: vec![(3.0, 1.0)],
            teamplay: true,
        };
        assert_eq!(
            decode_q2_team_info_checkpoint(&encode_q2_team_info_checkpoint(&team)).unwrap(),
            team
        );
        let fog = Q2RereleaseFogCheckpoint {
            color: Some(zero),
            density: Some(0.01),
            ..Q2RereleaseFogCheckpoint::default()
        };
        assert_eq!(
            decode_q2_rerelease_fog_checkpoint(&encode_q2_rerelease_fog_checkpoint(&fog)).unwrap(),
            fog
        );
    }
}
