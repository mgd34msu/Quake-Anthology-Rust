//! Quake III base/game: save state.
//!
//! Donor provenance: `src/content/q3/base/game/save-state.ts`.

use crate::value::arr;
use crate::value::boolean;
use crate::value::int;
use crate::value::obj;
use crate::value::str;
use crate::value::SaveJson;
use qa_core::identity::ActorId;
use qa_core::identity::SavedActorId;
use qa_core::math::Bounds;
use qa_core::math::Vec3;

// Intra-group imports: sibling modules split from the same flat port.
use crate::q3::base::game::mirrors_game_state::*;
use crate::q3::base::game::save_values::*;
use crate::q3::base::game::state::*;

// ---------------------------------------------------------------------------
// save-state.ts / save-reader.ts: graph capture, restore, and reads
// ---------------------------------------------------------------------------

/// Ownership entry (`captureOwnership` row).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwnershipEntry {
    /// Actor.
    pub actor: Option<ActorId>,
    /// Active.
    pub active: bool,
    /// Borrowed.
    pub borrowed: bool,
}

/// Client backing storage (`captureClientBacking` row).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientBacking {
    /// Source stats.
    pub source_stats: Vec<i32>,
    /// Special ammo.
    pub special_ammo: Vec<i32>,
}

/// Entity records surface (`Q3EntityRecords`).
pub trait Q3EntityRecords {
    /// Product.
    fn product(&self) -> Q3Product;
    /// Item table length.
    fn item_count(&self) -> usize;
    /// Capture ownership.
    fn capture_ownership(&self) -> Vec<OwnershipEntry>;
    /// Restore ownership.
    fn restore_ownership(&mut self, entries: Vec<OwnershipEntry>);
    /// Capture client backing.
    fn capture_client_backing(&self, slot: usize) -> ClientBacking;
    /// Restore client backing.
    fn restore_client_backing(&mut self, slot: usize, backing: &ClientBacking);
    /// Damage inflictor for an actor (`damageInflictor`).
    fn damage_inflictor(&self, actor: &ActorId) -> Participant;
    /// Restore record callbacks (`restoreCallbacks`).
    fn restore_callbacks(&mut self);
}

/// Saved-actor registry surface (`SessionActorRegistry` picks).
pub trait Q3ActorRegistry {
    /// Resolve a saved actor (`resolveSaved`).
    fn resolve_saved(&self, saved: &SavedActorId) -> Option<ActorId>;
    /// Reference a saved actor (`referenceSaved`).
    fn reference_saved(&self, saved: &SavedActorId) -> ActorId;
}

pub(crate) fn saved_actor_to_json(actor: &SavedActorId) -> SaveJson {
    obj(vec![
        ("slot", int(i64::from(actor.slot))),
        ("generation", int(i64::from(actor.generation))),
    ])
}

/// Saved trajectory.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SavedTrajectory {
    /// Raw type number.
    pub trajectory_type: i32,
    /// Time.
    pub time: i32,
    /// Duration.
    pub duration: i32,
    /// Base.
    pub base: Vec3,
    /// Delta.
    pub delta: Vec3,
}

pub(crate) fn trajectory_to_json(trajectory: &SavedTrajectory) -> SaveJson {
    obj(vec![
        ("type", num_i32(trajectory.trajectory_type)),
        ("time", num_i32(trajectory.time)),
        ("duration", num_i32(trajectory.duration)),
        ("base", vec_to_json(trajectory.base)),
        ("delta", vec_to_json(trajectory.delta)),
    ])
}

/// Saved shared fields.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SavedShared {
    /// Server flags.
    pub sv_flags: i32,
    /// Single client.
    pub single_client: i32,
    /// Contents.
    pub contents: i32,
    /// Owner number.
    pub owner_num: i32,
    /// Collision model.
    pub model: Q3CollisionModel,
}

/// Saved body state (`SavedBodyState`).
#[derive(Debug, Clone, PartialEq)]
pub struct SavedBodyState {
    /// Origin.
    pub origin: Vec3,
    /// Angles.
    pub angles: Vec3,
    /// Velocity.
    pub velocity: Vec3,
    /// Bounds.
    pub bounds: Bounds,
    /// Ground.
    pub ground: Option<SavedActorId>,
}

/// Saved link snapshot.
#[derive(Debug, Clone, PartialEq)]
pub struct SavedLink {
    /// Actor.
    pub actor: SavedActorId,
    /// State.
    pub state: SavedBodyState,
    /// Absolute bounds.
    pub absolute_bounds: Bounds,
    /// Link count.
    pub link_count: i32,
}

/// Saved private shared state.
#[derive(Debug, Clone, PartialEq)]
pub struct SavedSharedPrivate {
    /// Previous link.
    pub previous_link: Option<SavedLink>,
    /// Absolute-min override.
    pub abs_min_override: Option<Vec3>,
    /// Absolute-max override.
    pub abs_max_override: Option<Vec3>,
}

/// Saved classname (`Q3EntityState["classname"]`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SavedClassname {
    /// Plain value.
    Value(Option<String>),
    /// Client name.
    ClientName(usize),
}

/// Saved activation (`Q3EntityState["activation"]`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SavedActivation {
    /// Entity slot.
    Entity(usize),
    /// Saved actor.
    Actor(SavedActorId),
}

/// Saved entity (`Q3EntityState`).
#[derive(Debug, Clone, PartialEq)]
pub struct Q3GraphEntity {
    /// Values.
    pub values: EntityValues,
    /// Network values.
    pub network: NetworkValues,
    /// Position trajectory.
    pub pos: SavedTrajectory,
    /// Angle trajectory.
    pub apos: SavedTrajectory,
    /// Shared fields.
    pub shared: SavedShared,
    /// Private shared state.
    pub shared_private: SavedSharedPrivate,
    /// Client slot.
    pub client: Option<usize>,
    /// Classname.
    pub classname: SavedClassname,
    /// Parent slot.
    pub parent: Option<usize>,
    /// Next train slot.
    pub next_train: Option<usize>,
    /// Previous train slot.
    pub prev_train: Option<usize>,
    /// Target entity slot.
    pub target_ent: Option<usize>,
    /// Chain slot.
    pub chain: Option<usize>,
    /// Enemy slot.
    pub enemy: Option<usize>,
    /// Activator slot.
    pub activator: Option<usize>,
    /// Team chain slot.
    pub teamchain: Option<usize>,
    /// Team master slot.
    pub teammaster: Option<usize>,
    /// Activation.
    pub activation: Option<SavedActivation>,
    /// Item index.
    pub item: Option<usize>,
    /// Next think.
    pub nextthink: i32,
    /// Think callback.
    pub think: Option<String>,
    /// Reached callback.
    pub reached: Option<String>,
    /// Blocked callback.
    pub blocked: Option<String>,
    /// Touch callback.
    pub touch: Option<String>,
    /// Use callback.
    pub use_callback: Option<String>,
    /// Pain callback.
    pub pain: Option<String>,
    /// Die callback.
    pub die: Option<String>,
}

/// Saved client (`Q3ClientState`).
#[derive(Debug, Clone, PartialEq)]
pub struct Q3GraphClient {
    /// Values.
    pub values: ClientValues,
    /// Player values.
    pub player: PlayerValues,
    /// Persistant values.
    pub persistant: PersistantValues,
    /// Command.
    pub command: Q3UserCommand,
    /// Team values.
    pub team: TeamValues,
    /// Session values.
    pub session: SessionValues,
    /// Events.
    pub events: Vec<i32>,
    /// Event parameters.
    pub event_parms: Vec<i32>,
    /// Persistant slots.
    pub persistant_slots: Vec<i32>,
    /// Powerups.
    pub powerups: Vec<i32>,
    /// Ammo times.
    pub ammo_times: Vec<i32>,
    /// Backing storage.
    pub backing: ClientBacking,
    /// Hook slot.
    pub hook: Option<usize>,
    /// Persistant powerup slot.
    pub persistant_powerup: Option<usize>,
    /// Area bits.
    pub areabits: Option<Vec<u8>>,
}

/// Saved ownership row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SavedOwnership {
    /// Actor.
    pub actor: Option<SavedActorId>,
    /// Active.
    pub active: bool,
    /// Borrowed.
    pub borrowed: bool,
}

/// Saved graph (`Q3GraphState`).
#[derive(Debug, Clone, PartialEq)]
pub struct Q3Graph {
    /// Ownership rows.
    pub ownership: Vec<SavedOwnership>,
    /// Entities.
    pub entities: Vec<Q3GraphEntity>,
    /// Clients.
    pub clients: Vec<Q3GraphClient>,
    /// Entity count.
    pub num_entities: usize,
    /// Maximum clients.
    pub max_clients: usize,
}

pub(crate) fn opt_slot_to_json(slot: Option<usize>) -> SaveJson {
    slot.map_or(SaveJson::Null, |slot| int(slot as i64))
}

pub(crate) fn opt_callback_to_json(id: Option<&str>) -> SaveJson {
    id.map_or(SaveJson::Null, str)
}

pub(crate) fn collision_model_to_json(model: &Q3CollisionModel) -> SaveJson {
    match model {
        Q3CollisionModel::Inline { index } => obj(vec![("kind", str("inline")), ("index", num_i32(*index))]),
        Q3CollisionModel::Box => obj(vec![("kind", str("box"))]),
        Q3CollisionModel::Capsule => obj(vec![("kind", str("capsule"))]),
    }
}

pub(crate) fn saved_body_to_json(state: &SavedBodyState) -> SaveJson {
    obj(vec![
        ("origin", vec_to_json(state.origin)),
        ("angles", vec_to_json(state.angles)),
        ("velocity", vec_to_json(state.velocity)),
        (
            "bounds",
            obj(vec![
                ("min", vec_to_json(state.bounds.min)),
                ("max", vec_to_json(state.bounds.max)),
            ]),
        ),
        (
            "ground",
            state.ground.as_ref().map_or(SaveJson::Null, saved_actor_to_json),
        ),
    ])
}

pub(crate) fn saved_link_to_json(link: &SavedLink) -> SaveJson {
    obj(vec![
        ("actor", saved_actor_to_json(&link.actor)),
        ("linkCount", num_i32(link.link_count)),
        (
            "absoluteBounds",
            obj(vec![
                ("min", vec_to_json(link.absolute_bounds.min)),
                ("max", vec_to_json(link.absolute_bounds.max)),
            ]),
        ),
        ("state", saved_body_to_json(&link.state)),
    ])
}

pub(crate) fn graph_entity_to_json(entity: &Q3GraphEntity) -> SaveJson {
    let classname = match &entity.classname {
        SavedClassname::Value(value) => obj(vec![
            ("kind", str("value")),
            ("value", opt_str_to_json(value.as_deref())),
        ]),
        SavedClassname::ClientName(client) => obj(vec![("kind", str("client-name")), ("client", int(*client as i64))]),
    };
    let activation = entity
        .activation
        .as_ref()
        .map_or(SaveJson::Null, |activation| match activation {
            SavedActivation::Entity(slot) => obj(vec![("kind", str("entity")), ("slot", int(*slot as i64))]),
            SavedActivation::Actor(actor) => obj(vec![("kind", str("actor")), ("actor", saved_actor_to_json(actor))]),
        });
    obj(vec![
        ("values", entity_values_to_json(&entity.values)),
        ("network", network_values_to_json(&entity.network)),
        ("pos", trajectory_to_json(&entity.pos)),
        ("apos", trajectory_to_json(&entity.apos)),
        (
            "shared",
            obj(vec![
                ("svFlags", num_i32(entity.shared.sv_flags)),
                ("singleClient", num_i32(entity.shared.single_client)),
                ("contents", num_i32(entity.shared.contents)),
                ("ownerNum", num_i32(entity.shared.owner_num)),
                ("model", collision_model_to_json(&entity.shared.model)),
            ]),
        ),
        (
            "sharedPrivate",
            obj(vec![
                (
                    "absMinOverride",
                    entity
                        .shared_private
                        .abs_min_override
                        .map_or(SaveJson::Null, vec_to_json),
                ),
                (
                    "absMaxOverride",
                    entity
                        .shared_private
                        .abs_max_override
                        .map_or(SaveJson::Null, vec_to_json),
                ),
                (
                    "previousLink",
                    entity
                        .shared_private
                        .previous_link
                        .as_ref()
                        .map_or(SaveJson::Null, saved_link_to_json),
                ),
            ]),
        ),
        ("client", opt_slot_to_json(entity.client)),
        ("classname", classname),
        ("parent", opt_slot_to_json(entity.parent)),
        ("nextTrain", opt_slot_to_json(entity.next_train)),
        ("prevTrain", opt_slot_to_json(entity.prev_train)),
        ("targetEnt", opt_slot_to_json(entity.target_ent)),
        ("chain", opt_slot_to_json(entity.chain)),
        ("enemy", opt_slot_to_json(entity.enemy)),
        ("activator", opt_slot_to_json(entity.activator)),
        ("teamchain", opt_slot_to_json(entity.teamchain)),
        ("teammaster", opt_slot_to_json(entity.teammaster)),
        ("activation", activation),
        ("item", opt_slot_to_json(entity.item)),
        ("nextthink", num_i32(entity.nextthink)),
        ("think", opt_callback_to_json(entity.think.as_deref())),
        ("reached", opt_callback_to_json(entity.reached.as_deref())),
        ("blocked", opt_callback_to_json(entity.blocked.as_deref())),
        ("touch", opt_callback_to_json(entity.touch.as_deref())),
        ("use", opt_callback_to_json(entity.use_callback.as_deref())),
        ("pain", opt_callback_to_json(entity.pain.as_deref())),
        ("die", opt_callback_to_json(entity.die.as_deref())),
    ])
}

pub(crate) fn numbers_to_json(values: &[i32]) -> SaveJson {
    arr(values.iter().map(|value| num_i32(*value)).collect())
}

pub(crate) fn user_command_to_json(command: &Q3UserCommand) -> SaveJson {
    obj(vec![
        ("serverTime", num_i32(command.server_time)),
        ("angles", vec_to_json(command.angles)),
        ("buttons", num_i32(command.buttons)),
        ("weapon", num_i32(command.weapon)),
        ("forwardmove", num_i32(command.forwardmove)),
        ("rightmove", num_i32(command.rightmove)),
        ("upmove", num_i32(command.upmove)),
    ])
}

pub(crate) fn graph_client_to_json(client: &Q3GraphClient) -> SaveJson {
    obj(vec![
        ("values", client_values_to_json(&client.values)),
        ("player", player_values_to_json(&client.player)),
        ("persistant", persistant_values_to_json(&client.persistant)),
        ("command", user_command_to_json(&client.command)),
        ("team", team_values_to_json(&client.team)),
        ("session", session_values_to_json(&client.session)),
        ("events", numbers_to_json(&client.events)),
        ("eventParms", numbers_to_json(&client.event_parms)),
        ("persistantSlots", numbers_to_json(&client.persistant_slots)),
        ("powerups", numbers_to_json(&client.powerups)),
        ("ammoTimes", numbers_to_json(&client.ammo_times)),
        (
            "backing",
            obj(vec![
                ("sourceStats", numbers_to_json(&client.backing.source_stats)),
                ("specialAmmo", numbers_to_json(&client.backing.special_ammo)),
            ]),
        ),
        ("hook", opt_slot_to_json(client.hook)),
        ("persistantPowerup", opt_slot_to_json(client.persistant_powerup)),
        (
            "areabits",
            client
                .areabits
                .as_ref()
                .map_or(SaveJson::Null, |bytes| SaveJson::Bytes(bytes.clone())),
        ),
    ])
}

/// Encode a graph.
#[must_use]
pub fn graph_to_json(graph: &Q3Graph) -> SaveJson {
    obj(vec![
        (
            "ownership",
            arr(graph
                .ownership
                .iter()
                .map(|entry| {
                    obj(vec![
                        (
                            "actor",
                            entry.actor.as_ref().map_or(SaveJson::Null, saved_actor_to_json),
                        ),
                        ("active", boolean(entry.active)),
                        ("borrowed", boolean(entry.borrowed)),
                    ])
                })
                .collect()),
        ),
        (
            "entities",
            arr(graph.entities.iter().map(graph_entity_to_json).collect()),
        ),
        ("clients", arr(graph.clients.iter().map(graph_client_to_json).collect())),
        ("numEntities", int(graph.num_entities as i64)),
        ("maxClients", int(graph.max_clients as i64)),
    ])
}

pub(crate) fn saved_trajectory(trajectory: &Q3Trajectory) -> SavedTrajectory {
    SavedTrajectory {
        trajectory_type: trajectory.trajectory_type as i32,
        time: trajectory.time,
        duration: trajectory.duration,
        base: trajectory.base,
        delta: trajectory.delta,
    }
}

pub(crate) fn check_graph_entity(pool: &dyn EntityPool, slot: Option<usize>) -> Result<Option<usize>, Q3GameError> {
    match slot {
        None => Ok(None),
        Some(slot) if pool.entity(slot).is_some() => Ok(Some(slot)),
        Some(_) => Err(failure("Q3 graph contains a foreign entity record")),
    }
}

pub(crate) fn check_graph_client(pool: &dyn EntityPool, slot: Option<usize>) -> Result<Option<usize>, Q3GameError> {
    match slot {
        None => Ok(None),
        Some(slot) if pool.client(slot).is_some() => Ok(Some(slot)),
        Some(_) => Err(failure("Q3 graph contains a foreign client record")),
    }
}

/// Capture the entity/client graph (`captureQ3Graph`).
pub fn capture_q3_graph(records: &dyn Q3EntityRecords, pool: &dyn EntityPool) -> Result<Q3Graph, Q3GameError> {
    let mut entities = Vec::with_capacity(MAX_GENTITIES);
    for slot in 0..MAX_GENTITIES {
        let Some(entity) = pool.entity(slot) else {
            return Err(failure(format!("Q3 graph capture is missing entity slot {slot}")));
        };
        let link = entity.r.previous_link.as_ref().map(|link| SavedLink {
            actor: SavedActorId::from(&link.actor),
            link_count: link.link_count,
            absolute_bounds: link.absolute_bounds,
            state: SavedBodyState {
                origin: link.state.origin,
                angles: link.state.angles,
                velocity: link.state.velocity,
                bounds: link.state.bounds,
                ground: link.state.ground.as_ref().map(SavedActorId::from),
            },
        });
        let classname = match entity.capture_classname() {
            ClassName::Value(value) => SavedClassname::Value(value),
            ClassName::ClientName(client) => {
                check_graph_client(pool, Some(client))?;
                SavedClassname::ClientName(client)
            }
        };
        if let Some(Participant::Entity(slot)) = &entity.activation {
            check_graph_entity(pool, Some(*slot))?;
        }
        if let Some(item) = entity.item {
            if item >= records.item_count() {
                return Err(failure("Q3 entity has an unknown item identity"));
            }
        }
        let callbacks = pool.callbacks();
        entities.push(Q3GraphEntity {
            values: capture_entity_values(entity),
            network: capture_network_values(&entity.s),
            pos: saved_trajectory(&entity.s.pos),
            apos: saved_trajectory(&entity.s.apos),
            shared: SavedShared {
                sv_flags: entity.r.sv_flags,
                single_client: entity.r.single_client,
                contents: entity.r.contents,
                owner_num: entity.r.owner_num,
                model: entity.r.model,
            },
            shared_private: SavedSharedPrivate {
                previous_link: link,
                abs_min_override: entity.r.absmin_override,
                abs_max_override: entity.r.absmax_override,
            },
            client: check_graph_client(pool, entity.client)?,
            classname,
            parent: check_graph_entity(pool, entity.parent)?,
            next_train: check_graph_entity(pool, entity.next_train)?,
            prev_train: check_graph_entity(pool, entity.prev_train)?,
            target_ent: check_graph_entity(pool, entity.target_ent)?,
            chain: check_graph_entity(pool, entity.chain)?,
            enemy: check_graph_entity(pool, entity.enemy)?,
            activator: check_graph_entity(pool, entity.activator)?,
            teamchain: check_graph_entity(pool, entity.teamchain)?,
            teammaster: check_graph_entity(pool, entity.teammaster)?,
            activation: entity.activation.as_ref().map(|activation| match activation {
                Participant::Entity(slot) => SavedActivation::Entity(*slot),
                Participant::SharedActor(actor) => SavedActivation::Actor(SavedActorId::from(actor)),
            }),
            item: entity.item,
            nextthink: entity.nextthink,
            think: callbacks.think.capture(entity.think.as_ref())?,
            reached: callbacks.reached.capture(entity.reached.as_ref())?,
            blocked: callbacks.blocked.capture(entity.blocked.as_ref())?,
            touch: callbacks.touch.capture(entity.touch.as_ref())?,
            use_callback: callbacks.use_callbacks.capture(entity.use_callback.as_ref())?,
            pain: callbacks.pain.capture(entity.pain.as_ref())?,
            die: callbacks.die.capture(entity.die.as_ref())?,
        });
    }
    let mut clients = Vec::with_capacity(MAX_CLIENTS);
    for slot in 0..MAX_CLIENTS {
        let Some(client) = pool.client(slot) else {
            return Err(failure(format!("Q3 graph capture is missing client slot {slot}")));
        };
        clients.push(Q3GraphClient {
            values: capture_client_values(client),
            player: capture_player_values(&client.ps),
            persistant: capture_persistant_values(&client.pers),
            command: client.pers.cmd,
            team: capture_team_values(&client.pers.team_state),
            session: capture_session_values(&client.sess),
            events: client.ps.events.copy_vec(),
            event_parms: client.ps.event_parms.copy_vec(),
            persistant_slots: client.ps.persistant.copy_vec(),
            powerups: client.ps.powerups.copy_vec(),
            ammo_times: client.ammo_times.copy_vec(),
            backing: records.capture_client_backing(slot),
            hook: check_graph_entity(pool, client.hook)?,
            persistant_powerup: check_graph_entity(pool, client.persistant_powerup)?,
            areabits: client.areabits.clone(),
        });
    }
    Ok(Q3Graph {
        ownership: records
            .capture_ownership()
            .into_iter()
            .map(|entry| SavedOwnership {
                actor: entry.actor.as_ref().map(SavedActorId::from),
                active: entry.active,
                borrowed: entry.borrowed,
            })
            .collect(),
        entities,
        clients,
        num_entities: pool.num_entities(),
        max_clients: pool.max_clients(),
    })
}

/// Prepare record storage for a graph (`prepareQ3Graph`).
pub fn prepare_q3_graph(
    records: &mut dyn Q3EntityRecords,
    state: &Q3Graph,
    actors: &dyn Q3ActorRegistry,
) -> Result<(), Q3GameError> {
    if state.entities.len() != MAX_GENTITIES || state.clients.len() != MAX_CLIENTS {
        return Err(failure("Q3 save must retain all entity and client slots"));
    }
    let mut ownership = Vec::with_capacity(state.ownership.len());
    for entry in &state.ownership {
        let actor = entry.actor.as_ref().map(|saved| actors.resolve_saved(saved));
        if entry.actor.is_some() && actor.as_ref().is_some_and(Option::is_none) {
            return Err(failure("Q3 saved ownership actor is not live"));
        }
        ownership.push(OwnershipEntry {
            actor: actor.flatten(),
            active: entry.active,
            borrowed: entry.borrowed,
        });
    }
    records.restore_ownership(ownership);
    for (slot, client) in state.clients.iter().enumerate() {
        records.restore_client_backing(slot, &client.backing);
    }
    Ok(())
}

pub(crate) fn restore_graph_entity(pool: &dyn EntityPool, slot: Option<usize>) -> Result<Option<usize>, Q3GameError> {
    match slot {
        None => Ok(None),
        Some(slot) if pool.entity(slot).is_some() => Ok(Some(slot)),
        Some(slot) => Err(failure(format!("Q3 entity {slot} outside 0..1023"))),
    }
}

pub(crate) fn restore_graph_client(pool: &dyn EntityPool, slot: usize) -> Result<usize, Q3GameError> {
    if pool.client(slot).is_some() {
        Ok(slot)
    } else {
        Err(failure(format!("Q3 client {slot} outside 0..63")))
    }
}

/// Restore the entity/client graph (`restoreQ3Graph`).
pub fn restore_q3_graph(
    records: &dyn Q3EntityRecords,
    pool: &mut dyn EntityPool,
    state: &Q3Graph,
    actors: &dyn Q3ActorRegistry,
) -> Result<(), Q3GameError> {
    if state.entities.len() != MAX_GENTITIES || state.clients.len() != MAX_CLIENTS {
        return Err(failure("Q3 save must retain all entity and client slots"));
    }
    for (slot, saved) in state.entities.iter().enumerate() {
        let target = pool
            .entity_mut(slot)
            .ok_or_else(|| failure(format!("Q3 graph restore is missing entity slot {slot}")))?;
        restore_entity_values(target, &saved.values);
        restore_network_values(&mut target.s, &saved.network);
        target.s.pos = Q3Trajectory {
            trajectory_type: TrajectoryType::from_i32(saved.pos.trajectory_type)?,
            time: saved.pos.time,
            duration: saved.pos.duration,
            base: saved.pos.base,
            delta: saved.pos.delta,
        };
        target.s.apos = Q3Trajectory {
            trajectory_type: TrajectoryType::from_i32(saved.apos.trajectory_type)?,
            time: saved.apos.time,
            duration: saved.apos.duration,
            base: saved.apos.base,
            delta: saved.apos.delta,
        };
        target.r.sv_flags = saved.shared.sv_flags;
        target.r.single_client = saved.shared.single_client;
        target.r.model = saved.shared.model;
        target.r.contents = saved.shared.contents;
        target.r.owner_num = saved.shared.owner_num;
        target.r.absmin_override = saved.shared_private.abs_min_override;
        target.r.absmax_override = saved.shared_private.abs_max_override;
        target.r.previous_link = saved.shared_private.previous_link.as_ref().map(|link| Q3LinkedBody {
            actor: actors.reference_saved(&link.actor),
            link_count: link.link_count,
            absolute_bounds: link.absolute_bounds,
            state: Q3BodyState {
                origin: link.state.origin,
                angles: link.state.angles,
                velocity: link.state.velocity,
                bounds: link.state.bounds,
                ground: link.state.ground.as_ref().map(|ground| actors.reference_saved(ground)),
            },
        });
    }
    for (slot, saved) in state.entities.iter().enumerate() {
        let client = saved
            .client
            .map(|client| restore_graph_client(pool, client))
            .transpose()?;
        let classname = match &saved.classname {
            SavedClassname::Value(value) => ClassName::Value(value.clone()),
            SavedClassname::ClientName(client) => ClassName::ClientName(restore_graph_client(pool, *client)?),
        };
        let parent = restore_graph_entity(pool, saved.parent)?;
        let next_train = restore_graph_entity(pool, saved.next_train)?;
        let prev_train = restore_graph_entity(pool, saved.prev_train)?;
        let target_ent = restore_graph_entity(pool, saved.target_ent)?;
        let chain = restore_graph_entity(pool, saved.chain)?;
        let enemy = restore_graph_entity(pool, saved.enemy)?;
        let activator = restore_graph_entity(pool, saved.activator)?;
        let teamchain = restore_graph_entity(pool, saved.teamchain)?;
        let teammaster = restore_graph_entity(pool, saved.teammaster)?;
        let activation = match saved.activation.as_ref() {
            None => None,
            Some(SavedActivation::Entity(slot)) => {
                let slot = *slot;
                restore_graph_entity(pool, Some(slot))?;
                Some(Participant::Entity(slot))
            }
            Some(SavedActivation::Actor(actor)) => Some(records.damage_inflictor(&actors.reference_saved(actor))),
        };
        let item = saved
            .item
            .map(|item| {
                if item < records.item_count() {
                    Ok(item)
                } else {
                    Err(failure(format!("Q3 saved item {item} is outside the retained table")))
                }
            })
            .transpose()?;
        let callbacks = pool.callbacks();
        let think = callbacks.think.resolve(saved.think.as_deref())?;
        let reached = callbacks.reached.resolve(saved.reached.as_deref())?;
        let blocked = callbacks.blocked.resolve(saved.blocked.as_deref())?;
        let touch = callbacks.touch.resolve(saved.touch.as_deref())?;
        let use_callback = callbacks.use_callbacks.resolve(saved.use_callback.as_deref())?;
        let pain = callbacks.pain.resolve(saved.pain.as_deref())?;
        let die = callbacks.die.resolve(saved.die.as_deref())?;
        let target = pool
            .entity_mut(slot)
            .ok_or_else(|| failure(format!("Q3 graph restore is missing entity slot {slot}")))?;
        target.client = client;
        target.set_classname(None);
        if let ClassName::ClientName(client) = classname {
            target.bind_client_name(client);
        } else if let ClassName::Value(value) = classname {
            target.set_classname(value);
        }
        target.parent = parent;
        target.next_train = next_train;
        target.prev_train = prev_train;
        target.target_ent = target_ent;
        target.chain = chain;
        target.enemy = enemy;
        target.activator = activator;
        target.teamchain = teamchain;
        target.teammaster = teammaster;
        target.activation = activation;
        target.item = item;
        target.nextthink = saved.nextthink;
        target.think = think;
        target.reached = reached;
        target.blocked = blocked;
        target.touch = touch;
        target.use_callback = use_callback;
        target.pain = pain;
        target.die = die;
    }
    for (slot, saved) in state.clients.iter().enumerate() {
        let hook = restore_graph_entity(pool, saved.hook)?;
        let persistant_powerup = restore_graph_entity(pool, saved.persistant_powerup)?;
        let target = pool
            .client_mut(slot)
            .ok_or_else(|| failure(format!("Q3 graph restore is missing client slot {slot}")))?;
        restore_client_values(target, &saved.values);
        restore_player_values(&mut target.ps, &saved.player);
        restore_persistant_values(&mut target.pers, &saved.persistant);
        target.pers.cmd = saved.command;
        restore_team_values(&mut target.pers.team_state, &saved.team);
        restore_session_values(&mut target.sess, &saved.session);
        target.ps.events.restore(&saved.events)?;
        target.ps.event_parms.restore(&saved.event_parms)?;
        target.ps.persistant.restore(&saved.persistant_slots)?;
        target.ps.powerups.restore(&saved.powerups)?;
        target.ammo_times.restore(&saved.ammo_times)?;
        target.hook = hook;
        target.persistant_powerup = persistant_powerup;
        target.areabits = saved.areabits.clone();
    }
    pool.restore_counts(state.num_entities, state.max_clients);
    Ok(())
}
