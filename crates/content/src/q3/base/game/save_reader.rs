//! Quake III base/game: save reader.
//!
//! Donor provenance: `src/content/q3/base/game/save-reader.ts`.

use crate::value::SaveJson;
use crate::value::SaveReader;
use qa_core::identity::SavedActorId;
use qa_core::math::Bounds;

// Intra-group imports: sibling modules split from the same flat port.
use crate::q3::base::game::mirrors_game_state::*;
use crate::q3::base::game::save_state::*;
use crate::q3::base::game::save_values::*;

/// Read a saved actor (`readQ3Actor`).
pub fn read_q3_actor(reader: &SaveReader) -> Result<SavedActorId, Q3GameError> {
    let slot = reader.field("slot").integer(0)?;
    let generation = reader.field("generation").integer(0)?;
    if slot > i64::from(u32::MAX) || generation > i64::from(u32::MAX) {
        return Err(reader.fail("Q3 source actor outside session range").into());
    }
    #[allow(clippy::cast_possible_truncation)]
    Ok(SavedActorId {
        slot: slot as u32,
        generation: generation as u32,
    })
}

pub(crate) fn read_limited_slot(reader: &SaveReader, maximum: i64) -> Result<usize, Q3GameError> {
    let value = reader.integer(0)?;
    if value >= maximum {
        return Err(reader.fail("Q3 source reference outside retained table").into());
    }
    Ok(value as usize)
}

pub(crate) fn read_trajectory(reader: &SaveReader) -> Result<SavedTrajectory, Q3GameError> {
    Ok(SavedTrajectory {
        trajectory_type: read_i32(reader, "type")?,
        time: read_i32(reader, "time")?,
        duration: read_i32(reader, "duration")?,
        base: read_vec(reader, "base")?,
        delta: read_vec(reader, "delta")?,
    })
}

pub(crate) fn read_collision_model(reader: &SaveReader) -> Result<Q3CollisionModel, Q3GameError> {
    let kind = reader.field("kind").choice_str(&["inline", "box", "capsule"])?;
    if kind == "inline" {
        #[allow(clippy::cast_possible_truncation)]
        Ok(Q3CollisionModel::Inline {
            index: reader.field("index").integer(0)? as i32,
        })
    } else if kind == "box" {
        Ok(Q3CollisionModel::Box)
    } else {
        Ok(Q3CollisionModel::Capsule)
    }
}

pub(crate) fn read_saved_body(reader: &SaveReader) -> Result<SavedBodyState, Q3GameError> {
    let bounds = reader.field("bounds");
    Ok(SavedBodyState {
        origin: read_vec(reader, "origin")?,
        angles: read_vec(reader, "angles")?,
        velocity: read_vec(reader, "velocity")?,
        bounds: Bounds {
            min: read_vec(&bounds, "min")?,
            max: read_vec(&bounds, "max")?,
        },
        ground: reader.field("ground").nullable(|value| read_q3_actor(&value))?,
    })
}

pub(crate) fn read_saved_link(reader: &SaveReader) -> Result<SavedLink, Q3GameError> {
    let bounds = reader.field("absoluteBounds");
    #[allow(clippy::cast_possible_truncation)]
    Ok(SavedLink {
        actor: read_q3_actor(&reader.field("actor"))?,
        link_count: reader.field("linkCount").integer(0)? as i32,
        absolute_bounds: Bounds {
            min: read_vec(&bounds, "min")?,
            max: read_vec(&bounds, "max")?,
        },
        state: read_saved_body(&reader.field("state"))?,
    })
}

pub(crate) fn read_graph_entity(reader: &SaveReader) -> Result<Q3GraphEntity, Q3GameError> {
    let shared = reader.field("shared");
    let model = shared.field("model");
    let private = reader.field("sharedPrivate");
    let classname = reader.field("classname");
    let name_kind = classname.field("kind").choice_str(&["value", "client-name"])?;
    let read_entity_slot = |reader: &SaveReader, field: &str| -> Result<Option<usize>, Q3GameError> {
        reader.field(field).nullable(|value| read_limited_slot(&value, 1024))
    };
    Ok(Q3GraphEntity {
        values: read_entity_values(&reader.field("values"))?,
        network: read_network_values(&reader.field("network"))?,
        pos: read_trajectory(&reader.field("pos"))?,
        apos: read_trajectory(&reader.field("apos"))?,
        shared: SavedShared {
            sv_flags: read_i32(&shared, "svFlags")?,
            single_client: read_i32(&shared, "singleClient")?,
            contents: read_i32(&shared, "contents")?,
            owner_num: read_i32(&shared, "ownerNum")?,
            model: read_collision_model(&model)?,
        },
        shared_private: SavedSharedPrivate {
            previous_link: private
                .field("previousLink")
                .nullable(|value| read_saved_link(&value))?,
            abs_min_override: private
                .field("absMinOverride")
                .nullable(|value| vec_from_reader(&value))?,
            abs_max_override: private
                .field("absMaxOverride")
                .nullable(|value| vec_from_reader(&value))?,
        },
        client: reader.field("client").nullable(|value| read_limited_slot(&value, 64))?,
        classname: if name_kind == "value" {
            SavedClassname::Value(classname.field("value").nullable(|value| value.string())?)
        } else {
            SavedClassname::ClientName(read_limited_slot(&classname.field("client"), 64)?)
        },
        parent: read_entity_slot(reader, "parent")?,
        next_train: read_entity_slot(reader, "nextTrain")?,
        prev_train: read_entity_slot(reader, "prevTrain")?,
        target_ent: read_entity_slot(reader, "targetEnt")?,
        chain: read_entity_slot(reader, "chain")?,
        enemy: read_entity_slot(reader, "enemy")?,
        activator: read_entity_slot(reader, "activator")?,
        teamchain: read_entity_slot(reader, "teamchain")?,
        teammaster: read_entity_slot(reader, "teammaster")?,
        activation: reader
            .field("activation")
            .nullable(|value| -> Result<SavedActivation, Q3GameError> {
                if value.field("kind").choice_str(&["entity", "actor"])? == "entity" {
                    Ok(SavedActivation::Entity(read_limited_slot(&value.field("slot"), 1024)?))
                } else {
                    Ok(SavedActivation::Actor(read_q3_actor(&value.field("actor"))?))
                }
            })?,
        item: reader.field("item").nullable(|value| -> Result<usize, Q3GameError> {
            #[allow(clippy::cast_possible_truncation)]
            Ok(value.integer(0)? as usize)
        })?,
        nextthink: read_i32(reader, "nextthink")?,
        think: reader.field("think").nullable(|value| value.string())?,
        reached: reader.field("reached").nullable(|value| value.string())?,
        blocked: reader.field("blocked").nullable(|value| value.string())?,
        touch: reader.field("touch").nullable(|value| value.string())?,
        use_callback: reader.field("use").nullable(|value| value.string())?,
        pain: reader.field("pain").nullable(|value| value.string())?,
        die: reader.field("die").nullable(|value| value.string())?,
    })
}

pub(crate) fn read_numbers(reader: &SaveReader) -> Result<Vec<i32>, Q3GameError> {
    reader.list(|value| -> Result<i32, Q3GameError> {
        #[allow(clippy::cast_possible_truncation)]
        Ok(value.number()? as i32)
    })
}

pub(crate) fn read_user_command(reader: &SaveReader) -> Result<Q3UserCommand, Q3GameError> {
    Ok(Q3UserCommand {
        server_time: read_i32(reader, "serverTime")?,
        angles: read_vec(reader, "angles")?,
        buttons: read_i32(reader, "buttons")?,
        weapon: read_i32(reader, "weapon")?,
        forwardmove: read_i32(reader, "forwardmove")?,
        rightmove: read_i32(reader, "rightmove")?,
        upmove: read_i32(reader, "upmove")?,
    })
}

pub(crate) fn read_graph_client(reader: &SaveReader) -> Result<Q3GraphClient, Q3GameError> {
    let backing = reader.field("backing");
    Ok(Q3GraphClient {
        values: read_client_values(&reader.field("values"))?,
        player: read_player_values(&reader.field("player"))?,
        persistant: read_persistant_values(&reader.field("persistant"))?,
        command: read_user_command(&reader.field("command"))?,
        team: read_team_values(&reader.field("team"))?,
        session: read_session_values(&reader.field("session"))?,
        events: read_numbers(&reader.field("events"))?,
        event_parms: read_numbers(&reader.field("eventParms"))?,
        persistant_slots: read_numbers(&reader.field("persistantSlots"))?,
        powerups: read_numbers(&reader.field("powerups"))?,
        ammo_times: read_numbers(&reader.field("ammoTimes"))?,
        backing: ClientBacking {
            source_stats: read_numbers(&backing.field("sourceStats"))?,
            special_ammo: read_numbers(&backing.field("specialAmmo"))?,
        },
        hook: reader.field("hook").nullable(|value| read_limited_slot(&value, 1024))?,
        persistant_powerup: reader
            .field("persistantPowerup")
            .nullable(|value| read_limited_slot(&value, 1024))?,
        areabits: reader.field("areabits").nullable(|value| value.bytes())?,
    })
}

/// Read a graph (`readQ3Graph`).
pub fn read_q3_graph(value: &SaveJson) -> Result<Q3Graph, Q3GameError> {
    let reader = SaveReader::at(value, "q3.graph");
    #[allow(clippy::cast_possible_truncation)]
    Ok(Q3Graph {
        ownership: reader
            .field("ownership")
            .list(|entry| -> Result<SavedOwnership, Q3GameError> {
                Ok(SavedOwnership {
                    actor: entry.field("actor").nullable(|value| read_q3_actor(&value))?,
                    active: entry.field("active").boolean()?,
                    borrowed: entry.field("borrowed").boolean()?,
                })
            })?,
        entities: reader.field("entities").list(|entry| read_graph_entity(&entry))?,
        clients: reader.field("clients").list(|entry| read_graph_client(&entry))?,
        num_entities: reader.field("numEntities").integer(64)? as usize,
        max_clients: reader.field("maxClients").integer(1)? as usize,
    })
}
