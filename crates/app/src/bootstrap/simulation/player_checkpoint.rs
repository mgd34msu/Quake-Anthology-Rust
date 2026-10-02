//! Player checkpoint capture and readers.
//!
//! Absolute donor:
//! Port of Quake-Anthology-TS `src/app/bootstrap/simulation/player-checkpoint.ts`
//!
//! Covers the player save surface: trace-hit projection, movement/arsenal/
//! animation readers, movement-player capture/restore, and the Q3 character,
//! Q1 travel, and Q2 view readers. Movement states reuse the real world
//! family states behind the sibling [`MovementState`](super::player_input_application::MovementState)
//! union; weapon, arsenal, and animation states reuse
//! `qa_world::movement::types`; travel, view, and character records reuse the
//! real content types. Capture writes [`SaveJson`] directly, matching the
//! donor's plain-object saves; field names stay donor camelCase.

use qa_content::contract::{
    ArmorState as ContentArmorState, InventoryCountPolicy, InventoryEntry as ContentInventoryEntry, ItemId,
    PoweredProtectionState, RegularArmorState, SourceCounterArithmetic,
};
use qa_content::q1::base::travel::{Q1TravelExtension, Q1TravelState};
use qa_content::q1::foundation::types::Q1Weapon;
use qa_content::q2::base::player::types::{Q2PlayerTimer, Q2PlayerView};
use qa_content::q3::foundation::character::Q3CharacterCheckpoint;
use qa_core::identity::{ActorId, ClientId, OwnedActor, ProviderId, SavedActorId};
use qa_core::math::{Bounds, Vec3, Vec4};
use qa_world::combat::{ArmorState as WorldArmorState, PoweredProtection, RegularArmor};
use qa_world::inventory::{CountArithmetic, CountPolicy};
use qa_world::movement::q1::types::{Q1MovementState, QwMovementState};
use qa_world::movement::q2::types::{Q2MovementState, Q2RereleaseMovementState};
use qa_world::movement::q3::types::{Q3MovementState, Q3Product};
use qa_world::movement::types::{
    ActorAnimationState, AnimationState, ArsenalState, InventoryEntry as MovementInventoryEntry, Q1UserCommand,
    TraceHit, WeaponState,
};
use qa_world::save::records::{read_armor, read_inventory_entry, read_saved_actor, write_saved_actor};
use qa_world::save::shared::{read_bounds, read_time, read_vector, write_bounds, write_time, write_vector};
use qa_world::save::value::{arr, boolean, namespaced, num, obj, str, SaveJson, SaveReader};
use qa_world::WorldError;

use super::player_input_application::MovementState;

/// Checkpoint-facing surface of the donor `MovementPlayer` class.
///
/// Mirror of `MovementPlayer` from donor
/// `src/app/bootstrap/simulation/players.ts` (canonical home: the
/// players-lane port of that module); unify post-merge. Only the fields the
/// checkpoint capture reads are carried; live services, recipe, and host stay
/// with the owning lane.
#[derive(Debug, Clone, PartialEq)]
pub struct MovementPlayer {
    /// Owning actor handle.
    pub actor: OwnedActor,
    /// Owning client.
    pub client: ClientId,
    /// Movement state.
    pub state: MovementState,
    /// Arsenal snapshot.
    pub arsenal: ArsenalState,
    /// Animation snapshot.
    pub animation: ActorAnimationState,
    /// View angles.
    pub view_angles: Vec3,
    /// Command angles.
    pub command_angles: Vec3,
    /// View height.
    pub view_height: f64,
    /// Collision bounds.
    pub bounds: Bounds,
    /// Ground hit.
    pub ground: TraceHit,
    /// Water level.
    pub water_level: f64,
    /// Water contents type.
    pub water_type: f64,
    /// Intermission flag.
    pub intermission: bool,
    /// Cutscene camera, when active.
    pub cutscene: Option<PlayerCutscene>,
    /// Fixed-pose flag.
    pub fixed_pose_active: bool,
    /// Live body-shape base for NetQuake/QuakeWorld hulls.
    pub body_shape_base: Option<Bounds>,
    /// Gravity multiplier.
    pub gravity_multiplier: f64,
    /// Flight flag.
    pub flight: bool,
    /// World gravity.
    pub world_gravity: f64,
    /// Button bitmask.
    pub buttons: f64,
    /// Previous button bitmask.
    pub previous_buttons: f64,
    /// Last command sequence.
    pub last_sequence: i32,
    /// Pending NetQuake user command.
    pub net_quake_command: Option<Q1UserCommand>,
    /// Last weapon time in seconds.
    pub last_weapon_seconds: f64,
    /// Pending arsenal intent.
    pub arsenal_intent: Option<PlayerArsenalIntent>,
}

/// Cutscene camera pose (`MovementPlayer["cutscene"]`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlayerCutscene {
    /// Camera origin.
    pub origin: Vec3,
    /// Camera angles.
    pub angles: Vec3,
    /// View offset.
    pub view_offset: Vec3,
}

/// Weapon-selection intent with its optional source impulse byte.
///
/// Mirror of `ArsenalIntent` from donor `src/contracts/gameplay.ts`
/// (canonical home: `qa_net::common::commands::ArsenalIntent`); unify
/// post-merge. The network twin omits the optional impulse, so the exact
/// donor shape lives here until the twins merge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlayerArsenalIntent {
    /// Weapon provider.
    pub provider: ProviderId,
    /// Selected weapon, if any.
    pub weapon: Option<ItemId>,
    /// Holdable in use.
    pub use_holdable: bool,
    /// Source impulse byte.
    pub impulse: Option<u8>,
}

/// Restored movement-player record (`readMovementPlayer` result).
#[derive(Debug, Clone, PartialEq)]
pub struct SavedMovementPlayer {
    /// Live body-shape base.
    pub body_shape_base: Option<Bounds>,
    /// Fixed-pose flag.
    pub fixed_pose_active: bool,
    /// Pending arsenal intent.
    pub arsenal_intent: Option<PlayerArsenalIntent>,
    /// Pending NetQuake user command.
    pub net_quake_command: Option<Q1UserCommand>,
    /// Flight flag.
    pub flight: bool,
    /// Saved actor reference.
    pub actor: SavedActorId,
    /// Client slot.
    pub client_slot: u32,
    /// Movement state.
    pub state: MovementState,
    /// Arsenal snapshot.
    pub arsenal: ArsenalState,
    /// Animation snapshot.
    pub animation: ActorAnimationState,
    /// View angles.
    pub view_angles: Vec3,
    /// Command angles.
    pub command_angles: Vec3,
    /// View height.
    pub view_height: f64,
    /// Collision bounds.
    pub bounds: Bounds,
    /// Ground hit.
    pub ground: TraceHit,
    /// Water level.
    pub water_level: f64,
    /// Water contents type.
    pub water_type: f64,
    /// Intermission flag.
    pub intermission: bool,
    /// Gravity multiplier.
    pub gravity_multiplier: f64,
    /// World gravity.
    pub world_gravity: f64,
    /// Button bitmask.
    pub buttons: f64,
    /// Previous button bitmask.
    pub previous_buttons: f64,
    /// Last command sequence.
    pub last_sequence: i32,
    /// Last weapon time in seconds.
    pub last_weapon_seconds: f64,
    /// Cutscene camera, when active.
    pub cutscene: Option<PlayerCutscene>,
}

/// Live-actor resolver for checkpoint actor references.
pub type ActorReference<'a> = dyn Fn(SavedActorId) -> ActorId + 'a;

fn read_i32(reader: &SaveReader, minimum: i64) -> Result<i32, WorldError> {
    let value = reader.integer(minimum)?;
    i32::try_from(value).map_err(|_| reader.fail("integer out of range"))
}

fn read_u32(reader: &SaveReader, minimum: i64) -> Result<u32, WorldError> {
    let value = reader.integer(minimum)?;
    u32::try_from(value).map_err(|_| reader.fail("integer out of range"))
}

fn num_i32(value: i32) -> SaveJson {
    num(f64::from(value))
}

fn num_u32(value: u32) -> SaveJson {
    num(value as f64)
}

fn read_provider_id(reader: SaveReader) -> Result<ProviderId, WorldError> {
    let text = namespaced(reader.clone())?;
    let (namespace, name) = text
        .split_once(':')
        .ok_or_else(|| reader.fail("expected a namespaced identity"))?;
    Ok(ProviderId::new(namespace, name))
}

fn write_provider_id(provider: &ProviderId) -> SaveJson {
    str(&format!("{}:{}", provider.namespace, provider.name))
}

fn read_triple(reader: SaveReader) -> Result<[i32; 3], WorldError> {
    let values = reader.list(|value| read_i32(&value, i64::MIN))?;
    if values.len() != 3 {
        return Err(reader.fail("expected three numbers"));
    }
    Ok([values[0], values[1], values[2]])
}

fn write_triple(values: [i32; 3]) -> SaveJson {
    arr(values.into_iter().map(num_i32).collect())
}

/// Project a trace hit into its saved form (`saveHit`).
#[must_use]
pub fn save_hit(hit: &TraceHit) -> SaveJson {
    match hit {
        TraceHit::None => obj(vec![("kind", str("none"))]),
        TraceHit::World { model } => obj(vec![("kind", str("world")), ("model", num_u32(*model))]),
        TraceHit::Actor { actor } => obj(vec![
            ("kind", str("actor")),
            ("actor", write_saved_actor(SavedActorId::from(actor))),
        ]),
    }
}

/// Read a saved trace hit (`readHit`).
pub fn read_hit(reader: SaveReader, reference: &ActorReference<'_>) -> Result<TraceHit, WorldError> {
    match reader.field("kind").choice_str(&["none", "world", "actor"])?.as_str() {
        "none" => Ok(TraceHit::None),
        "world" => {
            let model = reader.field("model");
            Ok(TraceHit::World {
                model: read_u32(&model, 0)?,
            })
        }
        _ => Ok(TraceHit::Actor {
            actor: reference(read_saved_actor(reader.field("actor"))?),
        }),
    }
}

fn save_q1_netquake(state: &Q1MovementState, ground: SaveJson) -> SaveJson {
    obj(vec![
        ("kind", str("q1-netquake")),
        ("origin", write_vector(state.origin)),
        ("velocity", write_vector(state.velocity)),
        ("angles", write_vector(state.angles)),
        ("oldOrigin", write_vector(state.old_origin)),
        ("angularVelocity", write_vector(state.angular_velocity)),
        ("viewAngles", write_vector(state.view_angles)),
        ("punchAngles", write_vector(state.punch_angles)),
        ("moveType", num_i32(state.move_type)),
        ("flags", num_i32(state.flags)),
        ("ground", ground),
        ("waterLevel", num_i32(state.water_level)),
        ("waterType", num_i32(state.water_type)),
        ("teleportTimeSeconds", num(state.teleport_time_seconds)),
        ("waterJumpDirection", write_vector(state.water_jump_direction)),
        ("idealPitch", num(state.ideal_pitch)),
        ("fixAngle", boolean(state.fix_angle)),
        ("health", num(state.health)),
    ])
}

fn save_q1_quakeworld(state: &QwMovementState, ground: SaveJson) -> SaveJson {
    obj(vec![
        ("kind", str("q1-quakeworld")),
        ("origin", write_vector(state.origin)),
        ("velocity", write_vector(state.velocity)),
        ("angles", write_vector(state.angles)),
        ("oldButtons", num_i32(state.old_buttons)),
        ("waterJumpTimeSeconds", num(state.water_jump_time_seconds)),
        ("dead", boolean(state.dead)),
        ("spectator", num_i32(state.spectator)),
        ("ground", ground),
    ])
}

fn save_q2_classic(state: &Q2MovementState) -> SaveJson {
    obj(vec![
        ("kind", str("q2-classic")),
        ("type", num_i32(state.move_type)),
        ("originEighths", write_triple(state.origin_eighths)),
        ("velocityEighths", write_triple(state.velocity_eighths)),
        ("flags", num_i32(state.flags)),
        ("timeEightMilliseconds", num_i32(state.time_eight_milliseconds)),
        ("gravity", num(state.gravity)),
        ("deltaAngleShorts", write_triple(state.delta_angle_shorts)),
    ])
}

fn save_q2_rerelease(state: &Q2RereleaseMovementState) -> SaveJson {
    obj(vec![
        ("kind", str("q2-rerelease")),
        ("type", num_i32(state.move_type)),
        ("origin", write_vector(state.origin)),
        ("velocity", write_vector(state.velocity)),
        ("flags", num_i32(state.flags)),
        ("timeMilliseconds", num_i32(state.time_milliseconds)),
        ("gravity", num(state.gravity)),
        ("deltaAngles", write_vector(state.delta_angles)),
        ("viewHeight", num(state.view_height)),
    ])
}

#[allow(clippy::too_many_lines)]
fn save_q3(state: &Q3MovementState, ground: SaveJson) -> SaveJson {
    obj(vec![
        ("kind", str("q3")),
        ("commandTimeMilliseconds", num_i32(state.command_time_milliseconds)),
        ("movementType", num_i32(state.movement_type)),
        ("bobCycle", num_i32(state.bob_cycle)),
        ("movementFlags", num_i32(state.movement_flags)),
        ("movementTimeMilliseconds", num_i32(state.movement_time_milliseconds)),
        ("origin", write_vector(state.origin)),
        ("velocity", write_vector(state.velocity)),
        ("gravity", num(state.gravity)),
        ("speed", num(state.speed)),
        ("deltaAngleWords", write_triple(state.delta_angle_words)),
        ("movementDirection", num_i32(state.movement_direction)),
        ("grapplePoint", write_vector(state.grapple_point)),
        ("flags", num_i32(state.flags)),
        ("viewAngles", write_vector(state.view_angles)),
        ("viewHeight", num(state.view_height)),
        ("ground", ground),
        ("predictableEventSequence", num_i32(state.predictable_event_sequence)),
        (
            "jumpPad",
            state
                .jump_pad
                .as_ref()
                .map_or(SaveJson::Null, |pad| write_saved_actor(SavedActorId::from(pad))),
        ),
        ("movementFrame", num_i32(state.movement_frame)),
        ("jumpPadFrame", num_i32(state.jump_pad_frame)),
    ])
}

fn save_movement(state: &MovementState) -> SaveJson {
    match state {
        MovementState::Q1Netquake(inner) => save_q1_netquake(inner, save_hit(&inner.ground)),
        MovementState::Q1Quakeworld(inner) => save_q1_quakeworld(inner, save_hit(&inner.ground)),
        MovementState::Q2Classic(inner) => save_q2_classic(inner),
        MovementState::Q2Rerelease(inner) => save_q2_rerelease(inner),
        MovementState::Q3(inner) => save_q3(inner, save_hit(&inner.ground)),
    }
}

fn read_q1_netquake(reader: &SaveReader, reference: &ActorReference<'_>) -> Result<Q1MovementState, WorldError> {
    let v = |key: &str| read_vector(reader.field(key));
    let i = |key: &str| read_i32(&reader.field(key), i64::MIN);
    Ok(Q1MovementState {
        origin: v("origin")?,
        velocity: v("velocity")?,
        angles: v("angles")?,
        old_origin: v("oldOrigin")?,
        angular_velocity: v("angularVelocity")?,
        view_angles: v("viewAngles")?,
        punch_angles: v("punchAngles")?,
        move_type: i("moveType")?,
        flags: i("flags")?,
        ground: read_hit(reader.field("ground"), reference)?,
        water_level: i("waterLevel")?,
        water_type: i("waterType")?,
        teleport_time_seconds: reader.field("teleportTimeSeconds").number()?,
        water_jump_direction: v("waterJumpDirection")?,
        ideal_pitch: reader.field("idealPitch").number()?,
        fix_angle: reader.field("fixAngle").boolean()?,
        health: reader.field("health").number()?,
    })
}

fn read_q1_quakeworld(reader: &SaveReader, reference: &ActorReference<'_>) -> Result<QwMovementState, WorldError> {
    let v = |key: &str| read_vector(reader.field(key));
    Ok(QwMovementState {
        origin: v("origin")?,
        velocity: v("velocity")?,
        angles: v("angles")?,
        old_buttons: read_i32(&reader.field("oldButtons"), i64::MIN)?,
        water_jump_time_seconds: reader.field("waterJumpTimeSeconds").number()?,
        dead: reader.field("dead").boolean()?,
        spectator: read_i32(&reader.field("spectator"), i64::MIN)?,
        ground: read_hit(reader.field("ground"), reference)?,
    })
}

fn read_q2_classic(reader: &SaveReader) -> Result<Q2MovementState, WorldError> {
    Ok(Q2MovementState {
        move_type: read_i32(&reader.field("type"), i64::MIN)?,
        origin_eighths: read_triple(reader.field("originEighths"))?,
        velocity_eighths: read_triple(reader.field("velocityEighths"))?,
        flags: read_i32(&reader.field("flags"), i64::MIN)?,
        time_eight_milliseconds: read_i32(&reader.field("timeEightMilliseconds"), i64::MIN)?,
        gravity: reader.field("gravity").number()?,
        delta_angle_shorts: read_triple(reader.field("deltaAngleShorts"))?,
    })
}

fn read_q2_rerelease(reader: &SaveReader) -> Result<Q2RereleaseMovementState, WorldError> {
    Ok(Q2RereleaseMovementState {
        move_type: read_i32(&reader.field("type"), i64::MIN)?,
        origin: read_vector(reader.field("origin"))?,
        velocity: read_vector(reader.field("velocity"))?,
        flags: read_i32(&reader.field("flags"), i64::MIN)?,
        time_milliseconds: read_i32(&reader.field("timeMilliseconds"), i64::MIN)?,
        gravity: reader.field("gravity").number()?,
        delta_angles: read_vector(reader.field("deltaAngles"))?,
        view_height: reader.field("viewHeight").number()?,
    })
}

#[allow(clippy::too_many_lines)]
fn read_q3(reader: &SaveReader, reference: &ActorReference<'_>) -> Result<Q3MovementState, WorldError> {
    let v = |key: &str| read_vector(reader.field(key));
    let i = |key: &str| read_i32(&reader.field(key), i64::MIN);
    Ok(Q3MovementState {
        command_time_milliseconds: i("commandTimeMilliseconds")?,
        movement_type: i("movementType")?,
        bob_cycle: i("bobCycle")?,
        movement_flags: i("movementFlags")?,
        movement_time_milliseconds: i("movementTimeMilliseconds")?,
        origin: v("origin")?,
        velocity: v("velocity")?,
        gravity: reader.field("gravity").number()?,
        speed: reader.field("speed").number()?,
        delta_angle_words: read_triple(reader.field("deltaAngleWords"))?,
        movement_direction: i("movementDirection")?,
        grapple_point: v("grapplePoint")?,
        flags: i("flags")?,
        view_angles: v("viewAngles")?,
        view_height: reader.field("viewHeight").number()?,
        ground: read_hit(reader.field("ground"), reference)?,
        predictable_event_sequence: i("predictableEventSequence")?,
        jump_pad: reader
            .field("jumpPad")
            .nullable(|value| Ok(reference(read_saved_actor(value)?)))?,
        movement_frame: i("movementFrame")?,
        jump_pad_frame: i("jumpPadFrame")?,
    })
}

fn read_movement(reader: SaveReader, reference: &ActorReference<'_>) -> Result<MovementState, WorldError> {
    match reader
        .field("kind")
        .choice_str(&["q1-netquake", "q1-quakeworld", "q2-classic", "q2-rerelease", "q3"])?
        .as_str()
    {
        "q1-netquake" => Ok(MovementState::Q1Netquake(read_q1_netquake(&reader, reference)?)),
        "q1-quakeworld" => Ok(MovementState::Q1Quakeworld(read_q1_quakeworld(&reader, reference)?)),
        "q2-classic" => Ok(MovementState::Q2Classic(read_q2_classic(&reader)?)),
        "q2-rerelease" => Ok(MovementState::Q2Rerelease(read_q2_rerelease(&reader)?)),
        _ => Ok(MovementState::Q3(read_q3(&reader, reference)?)),
    }
}

fn read_weapon(reader: SaveReader) -> Result<WeaponState, WorldError> {
    match reader.field("kind").choice_str(&["q1", "q2", "q3"])?.as_str() {
        "q1" => Ok(WeaponState::Q1 {
            frame: read_i32(&reader.field("frame"), i64::MIN)?,
            attack_finished_seconds: reader.field("attackFinishedSeconds").number()?,
            source_weapon: read_i32(&reader.field("sourceWeapon"), i64::MIN)?,
        }),
        "q2" => Ok(WeaponState::Q2 {
            gun_frame: read_i32(&reader.field("gunFrame"), i64::MIN)?,
            state: read_i32(&reader.field("state"), i64::MIN)?,
            pending_weapon: reader.field("pendingWeapon").nullable(namespaced)?,
            machinegun_shots: read_i32(&reader.field("machinegunShots"), i64::MIN)?,
            grenade_time: read_time(reader.field("grenadeTime"))?,
            grenade_blew_up: reader.field("grenadeBlewUp").boolean()?,
        }),
        _ => Ok(WeaponState::Q3 {
            source_weapon: read_i32(&reader.field("sourceWeapon"), i64::MIN)?,
            state: read_i32(&reader.field("state"), i64::MIN)?,
            time_milliseconds: read_i32(&reader.field("timeMilliseconds"), i64::MIN)?,
        }),
    }
}

fn write_weapon(state: &WeaponState) -> SaveJson {
    match state {
        WeaponState::Q1 {
            frame,
            attack_finished_seconds,
            source_weapon,
        } => obj(vec![
            ("kind", str("q1")),
            ("frame", num_i32(*frame)),
            ("attackFinishedSeconds", num(*attack_finished_seconds)),
            ("sourceWeapon", num_i32(*source_weapon)),
        ]),
        WeaponState::Q2 {
            gun_frame,
            state,
            pending_weapon,
            machinegun_shots,
            grenade_time,
            grenade_blew_up,
        } => obj(vec![
            ("kind", str("q2")),
            ("gunFrame", num_i32(*gun_frame)),
            ("state", num_i32(*state)),
            (
                "pendingWeapon",
                pending_weapon.as_ref().map_or(SaveJson::Null, |weapon| str(weapon)),
            ),
            ("machinegunShots", num_i32(*machinegun_shots)),
            ("grenadeTime", write_time(*grenade_time)),
            ("grenadeBlewUp", boolean(*grenade_blew_up)),
        ]),
        WeaponState::Q3 {
            source_weapon,
            state,
            time_milliseconds,
        } => obj(vec![
            ("kind", str("q3")),
            ("sourceWeapon", num_i32(*source_weapon)),
            ("state", num_i32(*state)),
            ("timeMilliseconds", num_i32(*time_milliseconds)),
        ]),
    }
}

/// Read one arsenal ammo counter.
///
/// The movement-local ammo twin carries only item and count; capacity and
/// count policy stay with the content inventory lane. Donor saves may carry
/// the wider fields; the reader projects onto the twin. Unify post-merge by
/// widening `qa_world::movement::types::InventoryEntry`.
fn read_ammo_entry(reader: SaveReader) -> Result<MovementInventoryEntry, WorldError> {
    Ok(MovementInventoryEntry {
        item: namespaced(reader.field("item"))?,
        count: reader.field("count").number()?,
    })
}

fn write_ammo_entry(entry: &MovementInventoryEntry) -> SaveJson {
    obj(vec![("item", str(&entry.item)), ("count", num(entry.count))])
}

fn read_arsenal(reader: SaveReader) -> Result<ArsenalState, WorldError> {
    Ok(ArsenalState {
        provider: read_provider_id(reader.field("provider"))?,
        active_weapon: reader.field("activeWeapon").nullable(namespaced)?,
        state: read_weapon(reader.field("state"))?,
        ammo: reader.field("ammo").list(read_ammo_entry)?,
    })
}

fn write_arsenal(state: &ArsenalState) -> SaveJson {
    obj(vec![
        ("provider", write_provider_id(&state.provider)),
        (
            "activeWeapon",
            state
                .active_weapon
                .as_ref()
                .map_or(SaveJson::Null, |weapon| str(weapon)),
        ),
        ("state", write_weapon(&state.state)),
        ("ammo", arr(state.ammo.iter().map(write_ammo_entry).collect())),
    ])
}

/// Read an animation state (`readAnimation`).
pub fn read_animation(reader: SaveReader) -> Result<AnimationState, WorldError> {
    match reader.field("kind").choice_str(&["q1", "q2", "q3"])?.as_str() {
        "q1" => Ok(AnimationState::Q1 {
            frame: read_i32(&reader.field("frame"), i64::MIN)?,
            next_frame_seconds: reader.field("nextFrameSeconds").number()?,
        }),
        "q2" => Ok(AnimationState::Q2 {
            frame: read_i32(&reader.field("frame"), i64::MIN)?,
            end_frame: read_i32(&reader.field("endFrame"), i64::MIN)?,
            priority: read_i32(&reader.field("priority"), i64::MIN)?,
            duck: reader.field("duck").boolean()?,
            run: reader.field("run").boolean()?,
        }),
        _ => Ok(AnimationState::Q3 {
            legs: read_i32(&reader.field("legs"), i64::MIN)?,
            torso: read_i32(&reader.field("torso"), i64::MIN)?,
            legs_timer_milliseconds: read_i32(&reader.field("legsTimerMilliseconds"), i64::MIN)?,
            torso_timer_milliseconds: read_i32(&reader.field("torsoTimerMilliseconds"), i64::MIN)?,
        }),
    }
}

fn write_animation(state: &AnimationState) -> SaveJson {
    match state {
        AnimationState::Q1 {
            frame,
            next_frame_seconds,
        } => obj(vec![
            ("kind", str("q1")),
            ("frame", num_i32(*frame)),
            ("nextFrameSeconds", num(*next_frame_seconds)),
        ]),
        AnimationState::Q2 {
            frame,
            end_frame,
            priority,
            duck,
            run,
        } => obj(vec![
            ("kind", str("q2")),
            ("frame", num_i32(*frame)),
            ("endFrame", num_i32(*end_frame)),
            ("priority", num_i32(*priority)),
            ("duck", boolean(*duck)),
            ("run", boolean(*run)),
        ]),
        AnimationState::Q3 {
            legs,
            torso,
            legs_timer_milliseconds,
            torso_timer_milliseconds,
        } => obj(vec![
            ("kind", str("q3")),
            ("legs", num_i32(*legs)),
            ("torso", num_i32(*torso)),
            ("legsTimerMilliseconds", num_i32(*legs_timer_milliseconds)),
            ("torsoTimerMilliseconds", num_i32(*torso_timer_milliseconds)),
        ]),
    }
}

fn read_actor_animation(reader: SaveReader) -> Result<ActorAnimationState, WorldError> {
    Ok(ActorAnimationState {
        provider: read_provider_id(reader.field("provider"))?,
        state: read_animation(reader.field("state"))?,
    })
}

fn write_actor_animation(state: &ActorAnimationState) -> SaveJson {
    obj(vec![
        ("provider", write_provider_id(&state.provider)),
        ("state", write_animation(&state.state)),
    ])
}

fn write_user_command(command: &Q1UserCommand) -> SaveJson {
    obj(vec![
        ("kind", str("q1-netquake")),
        (
            "acknowledgedServerTimeSeconds",
            num(command.acknowledged_server_time_seconds),
        ),
        ("viewAngles", write_vector(command.view_angles)),
        ("forwardMove", num(command.forward_move)),
        ("sideMove", num(command.side_move)),
        ("upMove", num(command.up_move)),
        ("buttons", num_i32(command.buttons)),
        ("impulse", num_i32(command.impulse)),
    ])
}

fn read_user_command(reader: SaveReader) -> Result<Q1UserCommand, WorldError> {
    reader.field("kind").literal_str("q1-netquake")?;
    Ok(Q1UserCommand {
        acknowledged_server_time_seconds: reader.field("acknowledgedServerTimeSeconds").finite()?,
        view_angles: read_vector(reader.field("viewAngles"))?,
        forward_move: reader.field("forwardMove").finite()?,
        side_move: reader.field("sideMove").finite()?,
        up_move: reader.field("upMove").finite()?,
        buttons: read_i32(&reader.field("buttons"), 0)?,
        impulse: read_i32(&reader.field("impulse"), 0)?,
    })
}

fn write_cutscene(cutscene: &PlayerCutscene) -> SaveJson {
    obj(vec![
        ("origin", write_vector(cutscene.origin)),
        ("angles", write_vector(cutscene.angles)),
        ("viewOffset", write_vector(cutscene.view_offset)),
    ])
}

fn read_cutscene(reader: SaveReader) -> Result<PlayerCutscene, WorldError> {
    Ok(PlayerCutscene {
        origin: read_vector(reader.field("origin"))?,
        angles: read_vector(reader.field("angles"))?,
        view_offset: read_vector(reader.field("viewOffset"))?,
    })
}

fn write_arsenal_intent(intent: &PlayerArsenalIntent) -> SaveJson {
    obj(vec![
        ("provider", write_provider_id(&intent.provider)),
        ("weapon", SaveJson::Null),
        ("useHoldable", boolean(false)),
        ("impulse", num(f64::from(intent.impulse.unwrap_or(0)))),
    ])
}

fn read_arsenal_intent(reader: SaveReader) -> Result<PlayerArsenalIntent, WorldError> {
    let impulse_field = reader.field("impulse");
    let impulse = if impulse_field.is_missing() {
        None
    } else {
        let value = impulse_field.integer(0)?;
        if value > 255 {
            return Err(impulse_field.fail("source impulse exceeds one byte"));
        }
        Some(u8::try_from(value).map_err(|_| impulse_field.fail("source impulse exceeds one byte"))?)
    };
    Ok(PlayerArsenalIntent {
        provider: read_provider_id(reader.field("provider"))?,
        weapon: reader.field("weapon").nullable(namespaced)?,
        use_holdable: reader.field("useHoldable").boolean()?,
        impulse,
    })
}

/// Capture a movement player (`captureMovementPlayer`).
#[must_use]
#[allow(clippy::too_many_lines)]
pub fn capture_movement_player(player: &MovementPlayer) -> SaveJson {
    let intent = match player.arsenal_intent.as_ref() {
        Some(intent) if intent.impulse.is_some_and(|impulse| impulse != 0) => write_arsenal_intent(intent),
        _ => SaveJson::Null,
    };
    obj(vec![
        ("version", num(1.0)),
        (
            "bodyShapeBase",
            player.body_shape_base.map_or(SaveJson::Null, write_bounds),
        ),
        ("fixedPoseActive", boolean(player.fixed_pose_active)),
        ("arsenalIntent", intent),
        (
            "netQuakeCommand",
            player
                .net_quake_command
                .as_ref()
                .map_or(SaveJson::Null, write_user_command),
        ),
        ("actor", write_saved_actor(SavedActorId::from(player.actor.id()))),
        ("clientSlot", num_u32(player.client.slot())),
        ("state", save_movement(&player.state)),
        ("arsenal", write_arsenal(&player.arsenal)),
        ("animation", write_actor_animation(&player.animation)),
        ("viewAngles", write_vector(player.view_angles)),
        ("commandAngles", write_vector(player.command_angles)),
        ("viewHeight", num(player.view_height)),
        ("bounds", write_bounds(player.bounds)),
        ("ground", save_hit(&player.ground)),
        ("waterLevel", num(player.water_level)),
        ("waterType", num(player.water_type)),
        ("intermission", boolean(player.intermission)),
        (
            "cutscene",
            player.cutscene.as_ref().map_or(SaveJson::Null, write_cutscene),
        ),
        ("flight", boolean(player.flight)),
        ("gravityMultiplier", num(player.gravity_multiplier)),
        ("worldGravity", num(player.world_gravity)),
        ("buttons", num(player.buttons)),
        ("previousButtons", num(player.previous_buttons)),
        ("lastSequence", num_i32(player.last_sequence)),
        ("lastWeaponSeconds", num(player.last_weapon_seconds)),
    ])
}

/// Restore a movement player (`readMovementPlayer`).
#[allow(clippy::too_many_lines)]
pub fn read_movement_player(
    reader: SaveReader,
    reference: &ActorReference<'_>,
) -> Result<SavedMovementPlayer, WorldError> {
    reader.field("version").literal_i64(1)?;
    let saved_intent = reader.field("arsenalIntent");
    let arsenal_intent = if saved_intent.is_missing() {
        None
    } else {
        saved_intent.nullable(read_arsenal_intent)?
    };
    let body_shape_base = {
        let field = reader.field("bodyShapeBase");
        if field.is_missing() {
            None
        } else {
            field.nullable(read_bounds)?
        }
    };
    let fixed_pose_active = {
        let field = reader.field("fixedPoseActive");
        if field.is_missing() {
            false
        } else {
            field.boolean()?
        }
    };
    let flight = {
        let field = reader.field("flight");
        if field.is_missing() {
            false
        } else {
            field.boolean()?
        }
    };
    Ok(SavedMovementPlayer {
        body_shape_base,
        fixed_pose_active,
        arsenal_intent,
        net_quake_command: reader.field("netQuakeCommand").nullable(read_user_command)?,
        flight,
        actor: read_saved_actor(reader.field("actor"))?,
        client_slot: read_u32(&reader.field("clientSlot"), 0)?,
        state: read_movement(reader.field("state"), reference)?,
        arsenal: read_arsenal(reader.field("arsenal"))?,
        animation: read_actor_animation(reader.field("animation"))?,
        view_angles: read_vector(reader.field("viewAngles"))?,
        command_angles: read_vector(reader.field("commandAngles"))?,
        view_height: reader.field("viewHeight").number()?,
        bounds: read_bounds(reader.field("bounds"))?,
        ground: read_hit(reader.field("ground"), reference)?,
        water_level: reader.field("waterLevel").number()?,
        water_type: reader.field("waterType").number()?,
        intermission: reader.field("intermission").boolean()?,
        gravity_multiplier: reader.field("gravityMultiplier").number()?,
        world_gravity: reader.field("worldGravity").number()?,
        buttons: reader.field("buttons").number()?,
        previous_buttons: reader.field("previousButtons").number()?,
        last_sequence: read_i32(&reader.field("lastSequence"), -1)?,
        last_weapon_seconds: reader.field("lastWeaponSeconds").number()?,
        cutscene: reader.field("cutscene").nullable(read_cutscene)?,
    })
}

/// Restore a Q3 character checkpoint (`readQ3Character`).
pub fn read_q3_character(reader: SaveReader) -> Result<Q3CharacterCheckpoint, WorldError> {
    let animation = read_animation(reader.field("animation"))?;
    if !matches!(animation, AnimationState::Q3 { .. }) {
        return Err(reader.fail("Q3 character needs Q3 animation state"));
    }
    reader.field("version").literal_i64(1)?;
    let product = match reader.field("product").choice_str(&["baseq3", "missionpack"])?.as_str() {
        "baseq3" => Q3Product::BaseQ3,
        _ => Q3Product::MissionPack,
    };
    Ok(Q3CharacterCheckpoint {
        version: 1,
        product,
        animation,
        flags: read_i32(&reader.field("flags"), i64::MIN)?,
        event_sequence: read_i32(&reader.field("eventSequence"), 0)?,
        respawn_time: read_i32(&reader.field("respawnTime"), i64::MIN)?,
        spawn_count: read_i32(&reader.field("spawnCount"), 0)?,
        dead: reader.field("dead").boolean()?,
        gibbed: reader.field("gibbed").boolean()?,
        initialized: reader.field("initialized").boolean()?,
    })
}

fn convert_regular_armor(regular: RegularArmor) -> RegularArmorState {
    match regular {
        RegularArmor::None => RegularArmorState::None,
        RegularArmor::Q1 {
            points,
            absorption,
            item,
        } => RegularArmorState::Q1 {
            points,
            absorption,
            item,
        },
        RegularArmor::Q2 {
            points,
            normal_protection,
            energy_protection,
            item,
        } => RegularArmorState::Q2 {
            points,
            normal_protection,
            energy_protection,
            item,
        },
        RegularArmor::Q3 { points, protection } => RegularArmorState::Q3 { points, protection },
        RegularArmor::Source { points, item } => RegularArmorState::Source { points, item },
    }
}

fn convert_powered_protection(powered: PoweredProtection) -> PoweredProtectionState {
    match powered {
        PoweredProtection::None => PoweredProtectionState::None,
        PoweredProtection::Screen { cells } => PoweredProtectionState::Screen {
            cells: f64::from(cells),
        },
        PoweredProtection::Shield { cells } => PoweredProtectionState::Shield {
            cells: f64::from(cells),
        },
    }
}

fn convert_armor(armor: WorldArmorState) -> ContentArmorState {
    ContentArmorState {
        regular: convert_regular_armor(armor.regular),
        powered: convert_powered_protection(armor.powered),
    }
}

fn convert_arithmetic(arithmetic: CountArithmetic) -> SourceCounterArithmetic {
    match arithmetic {
        CountArithmetic::Binary32 => SourceCounterArithmetic::Binary32,
        CountArithmetic::Binary64 => SourceCounterArithmetic::Binary64,
        CountArithmetic::Int32 => SourceCounterArithmetic::Int32,
    }
}

fn convert_inventory_entry(entry: qa_world::inventory::InventoryEntry) -> ContentInventoryEntry {
    ContentInventoryEntry {
        item: entry.item,
        count: entry.count,
        capacity: entry.capacity,
        count_policy: entry.count_policy.map(|policy| match policy {
            CountPolicy::Stack => InventoryCountPolicy::Stack,
            CountPolicy::SourceCounter(arithmetic) => {
                InventoryCountPolicy::SourceCounter(convert_arithmetic(arithmetic))
            }
        }),
    }
}

/// Restore Q1 level-travel state (`readQ1Travel`).
pub fn read_q1_travel(reader: SaveReader) -> Result<Q1TravelState, WorldError> {
    let weapon_field = reader.field("weapon");
    let weapon_text = weapon_field.string()?;
    let weapon = Q1Weapon::parse(&weapon_text)
        .map_err(|_| weapon_field.fail(&format!("Unknown Q1 saved weapon {weapon_text}")))?;
    Ok(Q1TravelState {
        health: reader.field("health").number()?,
        max_health: reader.field("maxHealth").number()?,
        armor: convert_armor(read_armor(reader.field("armor"))?),
        inventory: reader
            .field("inventory")
            .list(read_inventory_entry)?
            .into_iter()
            .map(convert_inventory_entry)
            .collect(),
        weapon,
        extensions: reader.field("extensions").list(|value| {
            Ok(Q1TravelExtension {
                id: value.field("id").string()?,
                bytes: value.field("bytes").bytes()?,
            })
        })?,
    })
}

/// Restore a Q2 player view (`readQ2View`).
#[allow(clippy::cast_possible_truncation)]
pub fn read_q2_view(reader: SaveReader) -> Result<Q2PlayerView, WorldError> {
    let blend_field = reader.field("blend");
    let blend_w = blend_field.field("w").number()?;
    let blend_xyz = read_vector(blend_field)?;
    Ok(Q2PlayerView {
        angles: read_vector(reader.field("angles"))?,
        offset: read_vector(reader.field("offset"))?,
        kick_angles: read_vector(reader.field("kickAngles"))?,
        gun_angles: read_vector(reader.field("gunAngles"))?,
        gun_offset: read_vector(reader.field("gunOffset"))?,
        blend: Vec4 {
            x: blend_xyz.x,
            y: blend_xyz.y,
            z: blend_xyz.z,
            w: blend_w as f32,
        },
        fov: read_i32(&reader.field("fov"), i64::MIN)?,
        underwater: reader.field("underwater").boolean()?,
        flashes: read_i32(&reader.field("flashes"), i64::MIN)?,
        health: reader.field("health").number()?,
        armor: reader.field("armor").number()?,
        ammo: reader.field("ammo").number()?,
        score: read_i32(&reader.field("score"), i64::MIN)?,
        selected_item: reader.field("selectedItem").nullable(namespaced)?,
        timer: reader.field("timer").nullable(|value| {
            Ok(Q2PlayerTimer {
                item: namespaced(value.field("item"))?,
                seconds: read_i32(&value.field("seconds"), i64::MIN)?,
            })
        })?,
        spectator: reader.field("spectator").boolean()?,
        layouts: read_i32(&reader.field("layouts"), i64::MIN)?,
    })
}

#[cfg(test)]
mod tests {
    use qa_core::identity::IdentityOwner;
    use qa_core::math::vec3;
    use qa_core::time::SourceTime;

    use super::*;

    fn owner() -> IdentityOwner {
        IdentityOwner::create("player-checkpoint-test").expect("owner")
    }

    fn fixed_actor() -> (IdentityOwner, ActorId) {
        let owner = owner();
        let actor = owner.actor(3, 1);
        (owner, actor)
    }

    fn reference_for(actor: &ActorId) -> impl Fn(SavedActorId) -> ActorId + '_ {
        let actor = actor.clone();
        move |_| actor.clone()
    }

    fn provider() -> ProviderId {
        ProviderId::new("q1", "game")
    }

    fn bounds() -> Bounds {
        Bounds {
            min: vec3(-16.0, -16.0, -24.0),
            max: vec3(16.0, 16.0, 32.0),
        }
    }

    fn q1_state() -> Q1MovementState {
        Q1MovementState {
            origin: vec3(1.0, 2.0, 3.0),
            velocity: vec3(4.0, 5.0, 6.0),
            angles: vec3(0.0, 90.0, 0.0),
            old_origin: vec3(0.0, 0.0, 0.0),
            angular_velocity: vec3(0.0, 0.0, 10.0),
            view_angles: vec3(0.0, 90.0, 0.0),
            punch_angles: vec3(1.0, 0.0, 0.0),
            move_type: 3,
            flags: 1,
            ground: TraceHit::World { model: 2 },
            water_level: 1,
            water_type: -3,
            teleport_time_seconds: 12.5,
            water_jump_direction: vec3(0.0, 0.0, 1.0),
            ideal_pitch: 0.0,
            fix_angle: true,
            health: 100.0,
        }
    }

    fn arsenal() -> ArsenalState {
        ArsenalState {
            provider: provider(),
            active_weapon: Some("q1:weapon/shotgun".to_string()),
            state: WeaponState::Q1 {
                frame: 12,
                attack_finished_seconds: 4.5,
                source_weapon: 3,
            },
            ammo: vec![MovementInventoryEntry {
                item: "q1:ammo/shells".to_string(),
                count: 25.0,
            }],
        }
    }

    fn animation() -> ActorAnimationState {
        ActorAnimationState {
            provider: provider(),
            state: AnimationState::Q1 {
                frame: 12,
                next_frame_seconds: 0.5,
            },
        }
    }

    fn player() -> (IdentityOwner, MovementPlayer) {
        let owner = owner();
        let actor = owner.actor(3, 1);
        let owned = owner.owned_actor(&actor, provider()).expect("owned");
        let player = MovementPlayer {
            actor: owned,
            client: owner.client(0, 1),
            state: MovementState::Q1Netquake(q1_state()),
            arsenal: arsenal(),
            animation: animation(),
            view_angles: vec3(0.0, 90.0, 0.0),
            command_angles: vec3(0.0, 90.0, 0.0),
            view_height: 22.0,
            bounds: bounds(),
            ground: TraceHit::None,
            water_level: 0.0,
            water_type: 0.0,
            intermission: false,
            cutscene: None,
            fixed_pose_active: false,
            body_shape_base: None,
            gravity_multiplier: 1.0,
            flight: false,
            world_gravity: 800.0,
            buttons: 0.0,
            previous_buttons: 0.0,
            last_sequence: 41,
            net_quake_command: None,
            last_weapon_seconds: -1.0,
            arsenal_intent: Some(PlayerArsenalIntent {
                provider: provider(),
                weapon: Some("q1:weapon/shotgun".to_string()),
                use_holdable: true,
                impulse: Some(3),
            }),
        };
        (owner, player)
    }

    #[test]
    fn hit_round_trip() {
        let (owner, actor) = fixed_actor();
        let owned = owner.owned_actor(&actor, provider()).expect("owned");
        for hit in [
            TraceHit::None,
            TraceHit::World { model: 7 },
            TraceHit::Actor {
                actor: owned.id().clone(),
            },
        ] {
            let saved = save_hit(&hit);
            let restored = read_hit(SaveReader::new(&saved), &reference_for(owned.id())).expect("hit");
            assert_eq!(restored, hit);
        }
    }

    #[test]
    fn movement_round_trip_covers_all_dialects() {
        let (owner, actor) = fixed_actor();
        let states = vec![
            MovementState::Q1Netquake(q1_state()),
            MovementState::Q1Quakeworld(QwMovementState {
                origin: vec3(1.0, 0.0, 0.0),
                velocity: vec3(0.0, 1.0, 0.0),
                angles: vec3(0.0, 0.0, 0.0),
                old_buttons: 2,
                water_jump_time_seconds: 1.5,
                dead: false,
                spectator: 0,
                ground: TraceHit::Actor { actor: actor.clone() },
            }),
            MovementState::Q2Classic(Q2MovementState {
                move_type: 4,
                origin_eighths: [8, 16, 24],
                velocity_eighths: [1, 2, 3],
                flags: 0,
                time_eight_milliseconds: 100,
                gravity: 800.0,
                delta_angle_shorts: [0, 0, 0],
            }),
            MovementState::Q2Rerelease(Q2RereleaseMovementState {
                move_type: 4,
                origin: vec3(1.0, 2.0, 3.0),
                velocity: vec3(4.0, 5.0, 6.0),
                flags: 0,
                time_milliseconds: 800,
                gravity: 800.0,
                delta_angles: vec3(0.0, 0.0, 0.0),
                view_height: 22.0,
            }),
            MovementState::Q3(Q3MovementState {
                command_time_milliseconds: 100,
                movement_type: 2,
                bob_cycle: 0,
                movement_flags: 0,
                movement_time_milliseconds: 0,
                origin: vec3(1.0, 2.0, 3.0),
                velocity: vec3(0.0, 0.0, 0.0),
                gravity: 800.0,
                speed: 320.0,
                delta_angle_words: [0, 0, 0],
                movement_direction: 0,
                grapple_point: vec3(0.0, 0.0, 0.0),
                flags: 0,
                view_angles: vec3(0.0, 0.0, 0.0),
                view_height: 26.0,
                ground: TraceHit::World { model: 0 },
                predictable_event_sequence: 9,
                jump_pad: Some(actor.clone()),
                movement_frame: 3,
                jump_pad_frame: 1,
            }),
        ];
        for state in states {
            let saved = save_movement(&state);
            let restored = read_movement(SaveReader::new(&saved), &reference_for(&actor)).expect("state");
            assert_eq!(restored, state);
        }
        let _ = owner;
    }

    #[test]
    fn arsenal_animation_round_trip() {
        let saved = write_arsenal(&arsenal());
        let restored = read_arsenal(SaveReader::new(&saved)).expect("arsenal");
        assert_eq!(restored, arsenal());
        for state in [
            AnimationState::Q1 {
                frame: 1,
                next_frame_seconds: 0.5,
            },
            AnimationState::Q2 {
                frame: 2,
                end_frame: 39,
                priority: 1,
                duck: true,
                run: false,
            },
            AnimationState::Q3 {
                legs: 22,
                torso: 11,
                legs_timer_milliseconds: 5,
                torso_timer_milliseconds: 6,
            },
        ] {
            let saved = write_animation(&state);
            assert_eq!(read_animation(SaveReader::new(&saved)).expect("animation"), state);
        }
        let q2_weapon = WeaponState::Q2 {
            gun_frame: 7,
            state: 1,
            pending_weapon: Some("q2:weapon/railgun".to_string()),
            machinegun_shots: 3,
            grenade_time: SourceTime::Seconds(1.5),
            grenade_blew_up: false,
        };
        let saved = write_weapon(&q2_weapon);
        assert_eq!(read_weapon(SaveReader::new(&saved)).expect("weapon"), q2_weapon);
    }

    #[test]
    fn player_capture_restores() {
        let (owner, mut player) = player();
        player.cutscene = Some(PlayerCutscene {
            origin: vec3(1.0, 2.0, 3.0),
            angles: vec3(0.0, 0.0, 0.0),
            view_offset: vec3(0.0, 0.0, 8.0),
        });
        player.net_quake_command = Some(Q1UserCommand {
            acknowledged_server_time_seconds: 10.0,
            view_angles: vec3(0.0, 90.0, 0.0),
            forward_move: 400.0,
            side_move: 0.0,
            up_move: 0.0,
            buttons: 1,
            impulse: 0,
        });
        let actor = player.actor.id().clone();
        let saved = capture_movement_player(&player);
        let restored = read_movement_player(SaveReader::new(&saved), &reference_for(&actor)).expect("player");
        assert_eq!(restored.actor, SavedActorId::from(&actor));
        assert_eq!(restored.client_slot, 0);
        assert_eq!(restored.state, player.state);
        assert_eq!(restored.arsenal, player.arsenal);
        assert_eq!(restored.animation, player.animation);
        assert_eq!(restored.view_angles, player.view_angles);
        assert_eq!(restored.bounds, player.bounds);
        assert_eq!(restored.cutscene, player.cutscene);
        assert_eq!(restored.net_quake_command, player.net_quake_command);
        assert_eq!(restored.last_sequence, 41);
        // Capture keeps only provider and impulse; selection details reset.
        assert_eq!(
            restored.arsenal_intent,
            Some(PlayerArsenalIntent {
                provider: provider(),
                weapon: None,
                use_holdable: false,
                impulse: Some(3)
            })
        );
        let _ = owner;
    }

    #[test]
    fn player_capture_drops_empty_intent() {
        let (_owner, mut player) = player();
        player.arsenal_intent = Some(PlayerArsenalIntent {
            provider: provider(),
            weapon: None,
            use_holdable: false,
            impulse: Some(0),
        });
        let actor = player.actor.id().clone();
        let saved = capture_movement_player(&player);
        let restored = read_movement_player(SaveReader::new(&saved), &reference_for(&actor)).expect("player");
        assert_eq!(restored.arsenal_intent, None);
    }

    #[test]
    fn player_read_applies_legacy_defaults() {
        let (_owner, player) = player();
        let actor = player.actor.id().clone();
        let mut saved = capture_movement_player(&player);
        let SaveJson::Object(members) = &mut saved else {
            panic!("object")
        };
        members.retain(|(key, _)| {
            !matches!(
                key.as_str(),
                "bodyShapeBase" | "fixedPoseActive" | "flight" | "arsenalIntent"
            )
        });
        let restored = read_movement_player(SaveReader::new(&saved), &reference_for(&actor)).expect("player");
        assert_eq!(restored.body_shape_base, None);
        assert!(!restored.fixed_pose_active);
        assert!(!restored.flight);
        assert_eq!(restored.arsenal_intent, None);
    }

    #[test]
    fn player_read_rejects_wide_impulse() {
        let (_owner, player) = player();
        let actor = player.actor.id().clone();
        let saved = capture_movement_player(&player);
        let mut edited = match saved {
            SaveJson::Object(members) => members,
            _ => panic!("object"),
        };
        for (key, value) in &mut edited {
            if key == "arsenalIntent" {
                *value = obj(vec![
                    ("provider", str("q1:game")),
                    ("weapon", SaveJson::Null),
                    ("useHoldable", boolean(false)),
                    ("impulse", num(300.0)),
                ]);
            }
        }
        let saved = SaveJson::Object(edited);
        let error = read_movement_player(SaveReader::new(&saved), &reference_for(&actor)).expect_err("wide");
        assert!(error.to_string().contains("source impulse exceeds one byte"), "{error}");
    }

    #[test]
    fn q3_character_requires_q3_animation() {
        let good = obj(vec![
            ("version", num(1.0)),
            ("product", str("missionpack")),
            (
                "animation",
                obj(vec![
                    ("kind", str("q3")),
                    ("legs", num(22.0)),
                    ("torso", num(11.0)),
                    ("legsTimerMilliseconds", num(0.0)),
                    ("torsoTimerMilliseconds", num(0.0)),
                ]),
            ),
            ("flags", num(3.0)),
            ("eventSequence", num(9.0)),
            ("respawnTime", num(12.0)),
            ("spawnCount", num(2.0)),
            ("dead", boolean(false)),
            ("gibbed", boolean(false)),
            ("initialized", boolean(true)),
        ]);
        let restored = read_q3_character(SaveReader::new(&good)).expect("character");
        assert_eq!(restored.version, 1);
        assert_eq!(restored.product, Q3Product::MissionPack);
        assert_eq!(restored.spawn_count, 2);
        let bad = obj(vec![
            ("version", num(1.0)),
            ("product", str("baseq3")),
            (
                "animation",
                obj(vec![
                    ("kind", str("q1")),
                    ("frame", num(1.0)),
                    ("nextFrameSeconds", num(0.0)),
                ]),
            ),
            ("flags", num(0.0)),
            ("eventSequence", num(0.0)),
            ("respawnTime", num(0.0)),
            ("spawnCount", num(0.0)),
            ("dead", boolean(false)),
            ("gibbed", boolean(false)),
            ("initialized", boolean(false)),
        ]);
        let error = read_q3_character(SaveReader::new(&bad)).expect_err("q1 animation");
        assert!(
            error.to_string().contains("Q3 character needs Q3 animation state"),
            "{error}"
        );
    }

    #[test]
    fn q1_travel_round_trip_and_unknown_weapon() {
        let saved = obj(vec![
            ("health", num(100.0)),
            ("maxHealth", num(100.0)),
            (
                "armor",
                obj(vec![
                    ("regular", obj(vec![("kind", str("none"))])),
                    ("powered", obj(vec![("kind", str("none"))])),
                ]),
            ),
            (
                "inventory",
                arr(vec![obj(vec![
                    ("item", str("q1:ammo/shells")),
                    ("count", num(25.0)),
                    ("capacity", num(100.0)),
                ])]),
            ),
            ("weapon", str("rogue:plasma")),
            (
                "extensions",
                arr(vec![obj(vec![
                    ("id", str("x")),
                    ("bytes", SaveJson::Bytes(vec![1, 2])),
                ])]),
            ),
        ]);
        let restored = read_q1_travel(SaveReader::new(&saved)).expect("travel");
        assert_eq!(restored.weapon, Q1Weapon::RoguePlasma);
        assert_eq!(restored.inventory.len(), 1);
        assert_eq!(restored.inventory[0].capacity, 100.0);
        assert_eq!(restored.extensions[0].bytes, vec![1, 2]);
        let bad = obj(vec![
            ("health", num(100.0)),
            ("maxHealth", num(100.0)),
            (
                "armor",
                obj(vec![
                    ("regular", obj(vec![("kind", str("none"))])),
                    ("powered", obj(vec![("kind", str("none"))])),
                ]),
            ),
            ("inventory", arr(vec![])),
            ("weapon", str("nope:gun")),
            ("extensions", arr(vec![])),
        ]);
        let error = read_q1_travel(SaveReader::new(&bad)).expect_err("unknown weapon");
        assert!(
            error.to_string().contains("Unknown Q1 saved weapon nope:gun"),
            "{error}"
        );
    }

    #[test]
    fn q2_view_reads() {
        let vector = |x: f64, y: f64, z: f64| obj(vec![("x", num(x)), ("y", num(y)), ("z", num(z))]);
        let saved = obj(vec![
            ("angles", vector(0.0, 90.0, 0.0)),
            ("offset", vector(0.0, 0.0, 0.0)),
            ("kickAngles", vector(1.0, 0.0, 0.0)),
            ("gunAngles", vector(0.0, 0.0, 0.0)),
            ("gunOffset", vector(0.0, 0.0, 0.0)),
            (
                "blend",
                obj(vec![("x", num(0.0)), ("y", num(0.0)), ("z", num(0.0)), ("w", num(0.5))]),
            ),
            ("fov", num(90.0)),
            ("underwater", boolean(false)),
            ("flashes", num(0.0)),
            ("health", num(100.0)),
            ("armor", num(50.0)),
            ("ammo", num(10.0)),
            ("score", num(3.0)),
            ("selectedItem", str("q2:weapon/railgun")),
            (
                "timer",
                obj(vec![("item", str("q2:powerup/quad")), ("seconds", num(12.0))]),
            ),
            ("spectator", boolean(false)),
            ("layouts", num(1.0)),
        ]);
        let restored = read_q2_view(SaveReader::new(&saved)).expect("view");
        assert_eq!(restored.fov, 90);
        assert_eq!(restored.blend.w, 0.5);
        assert_eq!(restored.selected_item.as_deref(), Some("q2:weapon/railgun"));
        assert_eq!(restored.timer.as_ref().map(|timer| timer.seconds), Some(12));
    }
}
