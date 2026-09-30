//! Quake III base/game: save values.
//!
//! Donor provenance: `src/content/q3/base/game/save-values.ts`.

use crate::value::boolean;
use crate::value::int;
use crate::value::num;
use crate::value::obj;
use crate::value::str;
use crate::value::SaveJson;
use crate::value::SaveReader;
use qa_core::math::Vec3;

// Intra-group imports: sibling modules split from the same flat port.
use crate::q3::base::game::mirrors_game_state::*;
use crate::q3::base::game::save_level::*;
use crate::q3::base::game::state::*;

// ---------------------------------------------------------------------------
// save-values.ts: value structs with capture/restore/read
// ---------------------------------------------------------------------------

pub(crate) fn vec_to_json(value: Vec3) -> SaveJson {
    obj(vec![
        ("x", num(f64::from(value.x))),
        ("y", num(f64::from(value.y))),
        ("z", num(f64::from(value.z))),
    ])
}

pub(crate) fn vec_from_reader(reader: &SaveReader) -> Result<Vec3, Q3GameError> {
    Ok(crate::value::read_vector(reader.clone())?)
}

pub(crate) fn opt_str_to_json(value: Option<&str>) -> SaveJson {
    value.map_or(SaveJson::Null, str)
}

pub(crate) fn num_i32(value: i32) -> SaveJson {
    int(i64::from(value))
}

pub(crate) fn num_f32(value: f32) -> SaveJson {
    num(f64::from(value))
}

pub(crate) fn read_i32(reader: &SaveReader, field: &str) -> Result<i32, Q3GameError> {
    #[allow(clippy::cast_possible_truncation)]
    Ok(reader.field(field).number()? as i32)
}

pub(crate) fn read_f32(reader: &SaveReader, field: &str) -> Result<f32, Q3GameError> {
    #[allow(clippy::cast_possible_truncation)]
    Ok(reader.field(field).number()? as f32)
}

pub(crate) fn read_opt_string(reader: &SaveReader, field: &str) -> Result<Option<String>, Q3GameError> {
    Ok(reader.field(field).nullable(|value| value.string())?)
}

pub(crate) fn read_vec(reader: &SaveReader, field: &str) -> Result<Vec3, Q3GameError> {
    vec_from_reader(&reader.field(field))
}

/// Entity values (`EntityValues`).
#[derive(Debug, Clone, PartialEq)]
pub struct EntityValues {
    /// Spawn flags.
    pub spawnflags: i32,
    /// Never free.
    pub never_free: bool,
    /// Flags.
    pub flags: i32,
    /// Model.
    pub model: Option<String>,
    /// Model 2.
    pub model2: Option<String>,
    /// Free time.
    pub freetime: i32,
    /// Event time.
    pub event_time: i32,
    /// Free after event.
    pub free_after_event: bool,
    /// Unlink after event.
    pub unlink_after_event: bool,
    /// Physics object.
    pub physics_object: bool,
    /// Physics bounce.
    pub physics_bounce: i32,
    /// Clip mask.
    pub clipmask: i32,
    /// Mover state.
    pub mover_state: i32,
    /// Sound position 1.
    pub sound_pos1: i32,
    /// Sound 1 to 2.
    pub sound1to2: i32,
    /// Sound 2 to 1.
    pub sound2to1: i32,
    /// Sound position 2.
    pub sound_pos2: i32,
    /// Sound loop.
    pub sound_loop: i32,
    /// Position 1.
    pub pos1: Vec3,
    /// Position 2.
    pub pos2: Vec3,
    /// Message.
    pub message: Option<String>,
    /// Timestamp.
    pub timestamp: i32,
    /// Angle.
    pub angle: f32,
    /// Target.
    pub target: Option<String>,
    /// Target name.
    pub targetname: Option<String>,
    /// Team.
    pub team: Option<String>,
    /// Target shader name.
    pub target_shader_name: Option<String>,
    /// Target shader new name.
    pub target_shader_new_name: Option<String>,
    /// Speed.
    pub speed: f32,
    /// Move direction.
    pub movedir: Vec3,
    /// Pain debounce time.
    pub pain_debounce_time: i32,
    /// Fly sound debounce time.
    pub fly_sound_debounce_time: i32,
    /// Last move time.
    pub last_move_time: i32,
    /// Damage.
    pub damage: i32,
    /// Splash damage.
    pub splash_damage: i32,
    /// Splash radius.
    pub splash_radius: i32,
    /// Means of death.
    pub method_of_death: i32,
    /// Splash means of death.
    pub splash_method_of_death: i32,
    /// Count.
    pub count: i32,
    /// Kamikaze time.
    pub kamikaze_time: i32,
    /// Kamikaze shock time.
    pub kamikaze_shock_time: i32,
    /// Water type.
    pub watertype: i32,
    /// Water level.
    pub waterlevel: i32,
    /// Noise index.
    pub noise_index: i32,
    /// Wait.
    pub wait: f32,
    /// Random.
    pub random: f32,
}

/// Capture entity values (`captureEntityValues`).
#[must_use]
pub fn capture_entity_values(source: &GameEntity) -> EntityValues {
    EntityValues {
        spawnflags: source.spawnflags,
        never_free: source.never_free,
        flags: source.flags,
        model: source.model.clone(),
        model2: source.model2.clone(),
        freetime: source.freetime,
        event_time: source.event_time,
        free_after_event: source.free_after_event,
        unlink_after_event: source.unlink_after_event,
        physics_object: source.physics_object,
        physics_bounce: source.physics_bounce,
        clipmask: source.clipmask,
        mover_state: source.mover_state,
        sound_pos1: source.sound_pos1,
        sound1to2: source.sound1to2,
        sound2to1: source.sound2to1,
        sound_pos2: source.sound_pos2,
        sound_loop: source.sound_loop,
        pos1: source.pos1,
        pos2: source.pos2,
        message: source.message.clone(),
        timestamp: source.timestamp,
        angle: source.angle,
        target: source.target.clone(),
        targetname: source.targetname.clone(),
        team: source.team.clone(),
        target_shader_name: source.target_shader_name.clone(),
        target_shader_new_name: source.target_shader_new_name.clone(),
        speed: source.speed,
        movedir: source.movedir,
        pain_debounce_time: source.pain_debounce_time,
        fly_sound_debounce_time: source.fly_sound_debounce_time,
        last_move_time: source.last_move_time,
        damage: source.damage,
        splash_damage: source.splash_damage,
        splash_radius: source.splash_radius,
        method_of_death: source.method_of_death,
        splash_method_of_death: source.splash_method_of_death,
        count: source.count,
        kamikaze_time: source.kamikaze_time,
        kamikaze_shock_time: source.kamikaze_shock_time,
        watertype: source.watertype,
        waterlevel: source.waterlevel,
        noise_index: source.noise_index,
        wait: source.wait,
        random: source.random,
    }
}

/// Restore entity values (`restoreEntityValues`).
pub fn restore_entity_values(target: &mut GameEntity, state: &EntityValues) {
    target.spawnflags = state.spawnflags;
    target.never_free = state.never_free;
    target.flags = state.flags;
    target.model = state.model.clone();
    target.model2 = state.model2.clone();
    target.freetime = state.freetime;
    target.event_time = state.event_time;
    target.free_after_event = state.free_after_event;
    target.unlink_after_event = state.unlink_after_event;
    target.physics_object = state.physics_object;
    target.physics_bounce = state.physics_bounce;
    target.clipmask = state.clipmask;
    target.mover_state = state.mover_state;
    target.sound_pos1 = state.sound_pos1;
    target.sound1to2 = state.sound1to2;
    target.sound2to1 = state.sound2to1;
    target.sound_pos2 = state.sound_pos2;
    target.sound_loop = state.sound_loop;
    target.pos1 = state.pos1;
    target.pos2 = state.pos2;
    target.message = state.message.clone();
    target.timestamp = state.timestamp;
    target.angle = state.angle;
    target.target = state.target.clone();
    target.targetname = state.targetname.clone();
    target.team = state.team.clone();
    target.target_shader_name = state.target_shader_name.clone();
    target.target_shader_new_name = state.target_shader_new_name.clone();
    target.speed = state.speed;
    target.movedir = state.movedir;
    target.pain_debounce_time = state.pain_debounce_time;
    target.fly_sound_debounce_time = state.fly_sound_debounce_time;
    target.last_move_time = state.last_move_time;
    target.damage = state.damage;
    target.splash_damage = state.splash_damage;
    target.splash_radius = state.splash_radius;
    target.method_of_death = state.method_of_death;
    target.splash_method_of_death = state.splash_method_of_death;
    target.count = state.count;
    target.kamikaze_time = state.kamikaze_time;
    target.kamikaze_shock_time = state.kamikaze_shock_time;
    target.watertype = state.watertype;
    target.waterlevel = state.waterlevel;
    target.noise_index = state.noise_index;
    target.wait = state.wait;
    target.random = state.random;
}

/// Encode entity values.
#[must_use]
pub fn entity_values_to_json(state: &EntityValues) -> SaveJson {
    obj(vec![
        ("spawnflags", num_i32(state.spawnflags)),
        ("neverFree", boolean(state.never_free)),
        ("flags", num_i32(state.flags)),
        ("model", opt_str_to_json(state.model.as_deref())),
        ("model2", opt_str_to_json(state.model2.as_deref())),
        ("freetime", num_i32(state.freetime)),
        ("eventTime", num_i32(state.event_time)),
        ("freeAfterEvent", boolean(state.free_after_event)),
        ("unlinkAfterEvent", boolean(state.unlink_after_event)),
        ("physicsObject", boolean(state.physics_object)),
        ("physicsBounce", num_i32(state.physics_bounce)),
        ("clipmask", num_i32(state.clipmask)),
        ("moverState", num_i32(state.mover_state)),
        ("soundPos1", num_i32(state.sound_pos1)),
        ("sound1to2", num_i32(state.sound1to2)),
        ("sound2to1", num_i32(state.sound2to1)),
        ("soundPos2", num_i32(state.sound_pos2)),
        ("soundLoop", num_i32(state.sound_loop)),
        ("pos1", vec_to_json(state.pos1)),
        ("pos2", vec_to_json(state.pos2)),
        ("message", opt_str_to_json(state.message.as_deref())),
        ("timestamp", num_i32(state.timestamp)),
        ("angle", num_f32(state.angle)),
        ("target", opt_str_to_json(state.target.as_deref())),
        ("targetname", opt_str_to_json(state.targetname.as_deref())),
        ("team", opt_str_to_json(state.team.as_deref())),
        ("targetShaderName", opt_str_to_json(state.target_shader_name.as_deref())),
        (
            "targetShaderNewName",
            opt_str_to_json(state.target_shader_new_name.as_deref()),
        ),
        ("speed", num_f32(state.speed)),
        ("movedir", vec_to_json(state.movedir)),
        ("painDebounceTime", num_i32(state.pain_debounce_time)),
        ("flySoundDebounceTime", num_i32(state.fly_sound_debounce_time)),
        ("lastMoveTime", num_i32(state.last_move_time)),
        ("damage", num_i32(state.damage)),
        ("splashDamage", num_i32(state.splash_damage)),
        ("splashRadius", num_i32(state.splash_radius)),
        ("methodOfDeath", num_i32(state.method_of_death)),
        ("splashMethodOfDeath", num_i32(state.splash_method_of_death)),
        ("count", num_i32(state.count)),
        ("kamikazeTime", num_i32(state.kamikaze_time)),
        ("kamikazeShockTime", num_i32(state.kamikaze_shock_time)),
        ("watertype", num_i32(state.watertype)),
        ("waterlevel", num_i32(state.waterlevel)),
        ("noiseIndex", num_i32(state.noise_index)),
        ("wait", num_f32(state.wait)),
        ("random", num_f32(state.random)),
    ])
}

/// Read entity values (`readEntityValues`).
pub fn read_entity_values(reader: &SaveReader) -> Result<EntityValues, Q3GameError> {
    Ok(EntityValues {
        spawnflags: read_i32(reader, "spawnflags")?,
        never_free: reader.field("neverFree").boolean()?,
        flags: read_i32(reader, "flags")?,
        model: read_opt_string(reader, "model")?,
        model2: read_opt_string(reader, "model2")?,
        freetime: read_i32(reader, "freetime")?,
        event_time: read_i32(reader, "eventTime")?,
        free_after_event: reader.field("freeAfterEvent").boolean()?,
        unlink_after_event: reader.field("unlinkAfterEvent").boolean()?,
        physics_object: reader.field("physicsObject").boolean()?,
        physics_bounce: read_i32(reader, "physicsBounce")?,
        clipmask: read_i32(reader, "clipmask")?,
        mover_state: read_i32(reader, "moverState")?,
        sound_pos1: read_i32(reader, "soundPos1")?,
        sound1to2: read_i32(reader, "sound1to2")?,
        sound2to1: read_i32(reader, "sound2to1")?,
        sound_pos2: read_i32(reader, "soundPos2")?,
        sound_loop: read_i32(reader, "soundLoop")?,
        pos1: read_vec(reader, "pos1")?,
        pos2: read_vec(reader, "pos2")?,
        message: read_opt_string(reader, "message")?,
        timestamp: read_i32(reader, "timestamp")?,
        angle: read_f32(reader, "angle")?,
        target: read_opt_string(reader, "target")?,
        targetname: read_opt_string(reader, "targetname")?,
        team: read_opt_string(reader, "team")?,
        target_shader_name: read_opt_string(reader, "targetShaderName")?,
        target_shader_new_name: read_opt_string(reader, "targetShaderNewName")?,
        speed: read_f32(reader, "speed")?,
        movedir: read_vec(reader, "movedir")?,
        pain_debounce_time: read_i32(reader, "painDebounceTime")?,
        fly_sound_debounce_time: read_i32(reader, "flySoundDebounceTime")?,
        last_move_time: read_i32(reader, "lastMoveTime")?,
        damage: read_i32(reader, "damage")?,
        splash_damage: read_i32(reader, "splashDamage")?,
        splash_radius: read_i32(reader, "splashRadius")?,
        method_of_death: read_i32(reader, "methodOfDeath")?,
        splash_method_of_death: read_i32(reader, "splashMethodOfDeath")?,
        count: read_i32(reader, "count")?,
        kamikaze_time: read_i32(reader, "kamikazeTime")?,
        kamikaze_shock_time: read_i32(reader, "kamikazeShockTime")?,
        watertype: read_i32(reader, "watertype")?,
        waterlevel: read_i32(reader, "waterlevel")?,
        noise_index: read_i32(reader, "noiseIndex")?,
        wait: read_f32(reader, "wait")?,
        random: read_f32(reader, "random")?,
    })
}

/// Client values (`ClientValues`).
#[derive(Debug, Clone, PartialEq)]
pub struct ClientValues {
    /// Ready to exit.
    pub ready_to_exit: bool,
    /// Noclip.
    pub noclip: bool,
    /// Last command time.
    pub last_cmd_time: i32,
    /// Buttons.
    pub buttons: i32,
    /// Old buttons.
    pub old_buttons: i32,
    /// Latched buttons.
    pub latched_buttons: i32,
    /// Old origin.
    pub old_origin: Vec3,
    /// Damage armor.
    pub damage_armor: i32,
    /// Damage blood.
    pub damage_blood: i32,
    /// Damage knockback.
    pub damage_knockback: i32,
    /// Damage from.
    pub damage_from: Vec3,
    /// Damage from world.
    pub damage_from_world: bool,
    /// Accurate count.
    pub accurate_count: i32,
    /// Accuracy shots.
    pub accuracy_shots: i32,
    /// Accuracy hits.
    pub accuracy_hits: i32,
    /// Last killed client.
    pub last_killed_client: i32,
    /// Last hurt client.
    pub last_hurt_client: i32,
    /// Last hurt means of death.
    pub last_hurt_mod: i32,
    /// Respawn time.
    pub respawn_time: i32,
    /// Inactivity time.
    pub inactivity_time: i32,
    /// Inactivity warning.
    pub inactivity_warning: bool,
    /// Reward time.
    pub reward_time: i32,
    /// Air out time.
    pub air_out_time: i32,
    /// Last kill time.
    pub last_kill_time: i32,
    /// Fire held.
    pub fire_held: bool,
    /// Switch team time.
    pub switch_team_time: i32,
    /// Time residual.
    pub time_residual: i32,
    /// Portal identifier.
    pub portal_id: i32,
    /// Invulnerability time.
    pub invulnerability_time: i32,
}

/// Capture client values (`captureClientValues`).
#[must_use]
pub fn capture_client_values(source: &GameClient) -> ClientValues {
    ClientValues {
        ready_to_exit: source.ready_to_exit,
        noclip: source.noclip,
        last_cmd_time: source.last_cmd_time,
        buttons: source.buttons,
        old_buttons: source.old_buttons,
        latched_buttons: source.latched_buttons,
        old_origin: source.old_origin,
        damage_armor: source.damage_armor,
        damage_blood: source.damage_blood,
        damage_knockback: source.damage_knockback,
        damage_from: source.damage_from,
        damage_from_world: source.damage_from_world,
        accurate_count: source.accurate_count,
        accuracy_shots: source.accuracy_shots,
        accuracy_hits: source.accuracy_hits,
        last_killed_client: source.last_killed_client,
        last_hurt_client: source.last_hurt_client,
        last_hurt_mod: source.last_hurt_mod,
        respawn_time: source.respawn_time,
        inactivity_time: source.inactivity_time,
        inactivity_warning: source.inactivity_warning,
        reward_time: source.reward_time,
        air_out_time: source.air_out_time,
        last_kill_time: source.last_kill_time,
        fire_held: source.fire_held,
        switch_team_time: source.switch_team_time,
        time_residual: source.time_residual,
        portal_id: source.portal_id,
        invulnerability_time: source.invulnerability_time,
    }
}

/// Restore client values (`restoreClientValues`).
pub fn restore_client_values(target: &mut GameClient, state: &ClientValues) {
    target.ready_to_exit = state.ready_to_exit;
    target.noclip = state.noclip;
    target.last_cmd_time = state.last_cmd_time;
    target.buttons = state.buttons;
    target.old_buttons = state.old_buttons;
    target.latched_buttons = state.latched_buttons;
    target.old_origin = state.old_origin;
    target.damage_armor = state.damage_armor;
    target.damage_blood = state.damage_blood;
    target.damage_knockback = state.damage_knockback;
    target.damage_from = state.damage_from;
    target.damage_from_world = state.damage_from_world;
    target.accurate_count = state.accurate_count;
    target.accuracy_shots = state.accuracy_shots;
    target.accuracy_hits = state.accuracy_hits;
    target.last_killed_client = state.last_killed_client;
    target.last_hurt_client = state.last_hurt_client;
    target.last_hurt_mod = state.last_hurt_mod;
    target.respawn_time = state.respawn_time;
    target.inactivity_time = state.inactivity_time;
    target.inactivity_warning = state.inactivity_warning;
    target.reward_time = state.reward_time;
    target.air_out_time = state.air_out_time;
    target.last_kill_time = state.last_kill_time;
    target.fire_held = state.fire_held;
    target.switch_team_time = state.switch_team_time;
    target.time_residual = state.time_residual;
    target.portal_id = state.portal_id;
    target.invulnerability_time = state.invulnerability_time;
}

/// Encode client values.
#[must_use]
pub fn client_values_to_json(state: &ClientValues) -> SaveJson {
    obj(vec![
        ("readyToExit", boolean(state.ready_to_exit)),
        ("noclip", boolean(state.noclip)),
        ("lastCmdTime", num_i32(state.last_cmd_time)),
        ("buttons", num_i32(state.buttons)),
        ("oldButtons", num_i32(state.old_buttons)),
        ("latchedButtons", num_i32(state.latched_buttons)),
        ("oldOrigin", vec_to_json(state.old_origin)),
        ("damageArmor", num_i32(state.damage_armor)),
        ("damageBlood", num_i32(state.damage_blood)),
        ("damageKnockback", num_i32(state.damage_knockback)),
        ("damageFrom", vec_to_json(state.damage_from)),
        ("damageFromWorld", boolean(state.damage_from_world)),
        ("accurateCount", num_i32(state.accurate_count)),
        ("accuracyShots", num_i32(state.accuracy_shots)),
        ("accuracyHits", num_i32(state.accuracy_hits)),
        ("lastKilledClient", num_i32(state.last_killed_client)),
        ("lastHurtClient", num_i32(state.last_hurt_client)),
        ("lastHurtMod", num_i32(state.last_hurt_mod)),
        ("respawnTime", num_i32(state.respawn_time)),
        ("inactivityTime", num_i32(state.inactivity_time)),
        ("inactivityWarning", boolean(state.inactivity_warning)),
        ("rewardTime", num_i32(state.reward_time)),
        ("airOutTime", num_i32(state.air_out_time)),
        ("lastKillTime", num_i32(state.last_kill_time)),
        ("fireHeld", boolean(state.fire_held)),
        ("switchTeamTime", num_i32(state.switch_team_time)),
        ("timeResidual", num_i32(state.time_residual)),
        ("portalID", num_i32(state.portal_id)),
        ("invulnerabilityTime", num_i32(state.invulnerability_time)),
    ])
}

/// Read client values (`readClientValues`).
pub fn read_client_values(reader: &SaveReader) -> Result<ClientValues, Q3GameError> {
    Ok(ClientValues {
        ready_to_exit: reader.field("readyToExit").boolean()?,
        noclip: reader.field("noclip").boolean()?,
        last_cmd_time: read_i32(reader, "lastCmdTime")?,
        buttons: read_i32(reader, "buttons")?,
        old_buttons: read_i32(reader, "oldButtons")?,
        latched_buttons: read_i32(reader, "latchedButtons")?,
        old_origin: read_vec(reader, "oldOrigin")?,
        damage_armor: read_i32(reader, "damageArmor")?,
        damage_blood: read_i32(reader, "damageBlood")?,
        damage_knockback: read_i32(reader, "damageKnockback")?,
        damage_from: read_vec(reader, "damageFrom")?,
        damage_from_world: reader.field("damageFromWorld").boolean()?,
        accurate_count: read_i32(reader, "accurateCount")?,
        accuracy_shots: read_i32(reader, "accuracyShots")?,
        accuracy_hits: read_i32(reader, "accuracyHits")?,
        last_killed_client: read_i32(reader, "lastKilledClient")?,
        last_hurt_client: read_i32(reader, "lastHurtClient")?,
        last_hurt_mod: read_i32(reader, "lastHurtMod")?,
        respawn_time: read_i32(reader, "respawnTime")?,
        inactivity_time: read_i32(reader, "inactivityTime")?,
        inactivity_warning: reader.field("inactivityWarning").boolean()?,
        reward_time: read_i32(reader, "rewardTime")?,
        air_out_time: read_i32(reader, "airOutTime")?,
        last_kill_time: read_i32(reader, "lastKillTime")?,
        fire_held: reader.field("fireHeld").boolean()?,
        switch_team_time: read_i32(reader, "switchTeamTime")?,
        time_residual: read_i32(reader, "timeResidual")?,
        portal_id: read_i32(reader, "portalID")?,
        invulnerability_time: read_i32(reader, "invulnerabilityTime")?,
    })
}

/// Player values (`PlayerValues`).
#[derive(Debug, Clone, PartialEq)]
pub struct PlayerValues {
    /// Command time.
    pub command_time: i32,
    /// Movement type.
    pub pm_type: i32,
    /// Bob cycle.
    pub bob_cycle: i32,
    /// Move flags.
    pub pm_flags: i32,
    /// Move time.
    pub pm_time: i32,
    /// Weapon time.
    pub weapon_time: i32,
    /// Gravity.
    pub gravity: i32,
    /// Speed.
    pub speed: i32,
    /// Delta angles.
    pub delta_angles: [i32; 3],
    /// Ground entity number.
    pub ground_entity_num: i32,
    /// Legs timer.
    pub legs_timer: i32,
    /// Legs animation.
    pub legs_anim: i32,
    /// Torso timer.
    pub torso_timer: i32,
    /// Torso animation.
    pub torso_anim: i32,
    /// Movement direction.
    pub movement_dir: i32,
    /// Grapple point.
    pub grapple_point: Vec3,
    /// Entity flags.
    pub e_flags: i32,
    /// Event sequence.
    pub event_sequence: i32,
    /// External event.
    pub external_event: i32,
    /// External event parameter.
    pub external_event_parm: i32,
    /// External event time.
    pub external_event_time: i32,
    /// Client number.
    pub client_num: i32,
    /// Weapon.
    pub weapon: i32,
    /// Weapon state.
    pub weapon_state: i32,
    /// View angles.
    pub viewangles: Vec3,
    /// View height.
    pub viewheight: f32,
    /// Damage event.
    pub damage_event: i32,
    /// Damage yaw.
    pub damage_yaw: i32,
    /// Damage pitch.
    pub damage_pitch: i32,
    /// Damage count.
    pub damage_count: i32,
    /// Generic 1.
    pub generic1: i32,
    /// Loop sound.
    pub loop_sound: i32,
    /// Jump pad entity.
    pub jumppad_ent: i32,
    /// Ping.
    pub ping: i32,
    /// Pmove frame count.
    pub pmove_framecount: i32,
    /// Jump pad frame.
    pub jumppad_frame: i32,
    /// Entity event sequence.
    pub entity_event_sequence: i32,
}

/// Capture player values (`capturePlayerValues`).
#[must_use]
pub fn capture_player_values(source: &Q3PlayerState) -> PlayerValues {
    PlayerValues {
        command_time: source.command_time,
        pm_type: source.pm_type,
        bob_cycle: source.bob_cycle,
        pm_flags: source.pm_flags,
        pm_time: source.pm_time,
        weapon_time: source.weapon_time,
        gravity: source.gravity,
        speed: source.speed,
        delta_angles: source.delta_angles,
        ground_entity_num: source.ground_entity_num,
        legs_timer: source.legs_timer,
        legs_anim: source.legs_anim,
        torso_timer: source.torso_timer,
        torso_anim: source.torso_anim,
        movement_dir: source.movement_dir,
        grapple_point: source.grapple_point,
        e_flags: source.e_flags,
        event_sequence: source.event_sequence,
        external_event: source.external_event,
        external_event_parm: source.external_event_parm,
        external_event_time: source.external_event_time,
        client_num: source.client_num,
        weapon: source.weapon,
        weapon_state: source.weapon_state,
        viewangles: source.viewangles,
        viewheight: source.viewheight,
        damage_event: source.damage_event,
        damage_yaw: source.damage_yaw,
        damage_pitch: source.damage_pitch,
        damage_count: source.damage_count,
        generic1: source.generic1,
        loop_sound: source.loop_sound,
        jumppad_ent: source.jumppad_ent,
        ping: source.ping,
        pmove_framecount: source.pmove_framecount,
        jumppad_frame: source.jumppad_frame,
        entity_event_sequence: source.entity_event_sequence,
    }
}

/// Restore player values (`restorePlayerValues`).
pub fn restore_player_values(target: &mut Q3PlayerState, state: &PlayerValues) {
    target.command_time = state.command_time;
    target.pm_type = state.pm_type;
    target.bob_cycle = state.bob_cycle;
    target.pm_flags = state.pm_flags;
    target.pm_time = state.pm_time;
    target.weapon_time = state.weapon_time;
    target.gravity = state.gravity;
    target.speed = state.speed;
    target.delta_angles = state.delta_angles;
    target.ground_entity_num = state.ground_entity_num;
    target.legs_timer = state.legs_timer;
    target.legs_anim = state.legs_anim;
    target.torso_timer = state.torso_timer;
    target.torso_anim = state.torso_anim;
    target.movement_dir = state.movement_dir;
    target.grapple_point = state.grapple_point;
    target.e_flags = state.e_flags;
    target.event_sequence = state.event_sequence;
    target.external_event = state.external_event;
    target.external_event_parm = state.external_event_parm;
    target.external_event_time = state.external_event_time;
    target.client_num = state.client_num;
    target.weapon = state.weapon;
    target.weapon_state = state.weapon_state;
    target.viewangles = state.viewangles;
    target.viewheight = state.viewheight;
    target.damage_event = state.damage_event;
    target.damage_yaw = state.damage_yaw;
    target.damage_pitch = state.damage_pitch;
    target.damage_count = state.damage_count;
    target.generic1 = state.generic1;
    target.loop_sound = state.loop_sound;
    target.jumppad_ent = state.jumppad_ent;
    target.ping = state.ping;
    target.pmove_framecount = state.pmove_framecount;
    target.jumppad_frame = state.jumppad_frame;
    target.entity_event_sequence = state.entity_event_sequence;
}

/// Encode player values.
#[must_use]
pub fn player_values_to_json(state: &PlayerValues) -> SaveJson {
    obj(vec![
        ("commandTime", num_i32(state.command_time)),
        ("pmType", num_i32(state.pm_type)),
        ("bobCycle", num_i32(state.bob_cycle)),
        ("pmFlags", num_i32(state.pm_flags)),
        ("pmTime", num_i32(state.pm_time)),
        ("weaponTime", num_i32(state.weapon_time)),
        ("gravity", num_i32(state.gravity)),
        ("speed", num_i32(state.speed)),
        (
            "deltaAngles",
            obj(vec![
                ("x", num_i32(state.delta_angles[0])),
                ("y", num_i32(state.delta_angles[1])),
                ("z", num_i32(state.delta_angles[2])),
            ]),
        ),
        ("groundEntityNum", num_i32(state.ground_entity_num)),
        ("legsTimer", num_i32(state.legs_timer)),
        ("legsAnim", num_i32(state.legs_anim)),
        ("torsoTimer", num_i32(state.torso_timer)),
        ("torsoAnim", num_i32(state.torso_anim)),
        ("movementDir", num_i32(state.movement_dir)),
        ("grapplePoint", vec_to_json(state.grapple_point)),
        ("eFlags", num_i32(state.e_flags)),
        ("eventSequence", num_i32(state.event_sequence)),
        ("externalEvent", num_i32(state.external_event)),
        ("externalEventParm", num_i32(state.external_event_parm)),
        ("externalEventTime", num_i32(state.external_event_time)),
        ("clientNum", num_i32(state.client_num)),
        ("weapon", num_i32(state.weapon)),
        ("weaponState", num_i32(state.weapon_state)),
        ("viewangles", vec_to_json(state.viewangles)),
        ("viewheight", num_f32(state.viewheight)),
        ("damageEvent", num_i32(state.damage_event)),
        ("damageYaw", num_i32(state.damage_yaw)),
        ("damagePitch", num_i32(state.damage_pitch)),
        ("damageCount", num_i32(state.damage_count)),
        ("generic1", num_i32(state.generic1)),
        ("loopSound", num_i32(state.loop_sound)),
        ("jumppadEnt", num_i32(state.jumppad_ent)),
        ("ping", num_i32(state.ping)),
        ("pmoveFramecount", num_i32(state.pmove_framecount)),
        ("jumppadFrame", num_i32(state.jumppad_frame)),
        ("entityEventSequence", num_i32(state.entity_event_sequence)),
    ])
}

/// Read player values (`readPlayerValues`).
pub fn read_player_values(reader: &SaveReader) -> Result<PlayerValues, Q3GameError> {
    Ok(PlayerValues {
        command_time: read_i32(reader, "commandTime")?,
        pm_type: read_i32(reader, "pmType")?,
        bob_cycle: read_i32(reader, "bobCycle")?,
        pm_flags: read_i32(reader, "pmFlags")?,
        pm_time: read_i32(reader, "pmTime")?,
        weapon_time: read_i32(reader, "weaponTime")?,
        gravity: read_i32(reader, "gravity")?,
        speed: read_i32(reader, "speed")?,
        delta_angles: {
            let angles = reader.field("deltaAngles");
            [
                read_i32(&angles, "x")?,
                read_i32(&angles, "y")?,
                read_i32(&angles, "z")?,
            ]
        },
        ground_entity_num: read_i32(reader, "groundEntityNum")?,
        legs_timer: read_i32(reader, "legsTimer")?,
        legs_anim: read_i32(reader, "legsAnim")?,
        torso_timer: read_i32(reader, "torsoTimer")?,
        torso_anim: read_i32(reader, "torsoAnim")?,
        movement_dir: read_i32(reader, "movementDir")?,
        grapple_point: read_vec(reader, "grapplePoint")?,
        e_flags: read_i32(reader, "eFlags")?,
        event_sequence: read_i32(reader, "eventSequence")?,
        external_event: read_i32(reader, "externalEvent")?,
        external_event_parm: read_i32(reader, "externalEventParm")?,
        external_event_time: read_i32(reader, "externalEventTime")?,
        client_num: read_i32(reader, "clientNum")?,
        weapon: read_i32(reader, "weapon")?,
        weapon_state: read_i32(reader, "weaponState")?,
        viewangles: read_vec(reader, "viewangles")?,
        viewheight: read_f32(reader, "viewheight")?,
        damage_event: read_i32(reader, "damageEvent")?,
        damage_yaw: read_i32(reader, "damageYaw")?,
        damage_pitch: read_i32(reader, "damagePitch")?,
        damage_count: read_i32(reader, "damageCount")?,
        generic1: read_i32(reader, "generic1")?,
        loop_sound: read_i32(reader, "loopSound")?,
        jumppad_ent: read_i32(reader, "jumppadEnt")?,
        ping: read_i32(reader, "ping")?,
        pmove_framecount: read_i32(reader, "pmoveFramecount")?,
        jumppad_frame: read_i32(reader, "jumppadFrame")?,
        entity_event_sequence: read_i32(reader, "entityEventSequence")?,
    })
}

/// Persistant values (`PersistantValues`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PersistantValues {
    /// Connected.
    pub connected: i32,
    /// Local client.
    pub local_client: bool,
    /// Initial spawn.
    pub initial_spawn: bool,
    /// Predict item pickup.
    pub predict_item_pickup: bool,
    /// Pmove fixed.
    pub pmove_fixed: bool,
    /// Net name.
    pub netname: String,
    /// Max health.
    pub max_health: i32,
    /// Enter time.
    pub enter_time: i32,
    /// Vote count.
    pub vote_count: i32,
    /// Team vote count.
    pub team_vote_count: i32,
    /// Team info.
    pub team_info: bool,
}

/// Capture persistant values (`capturePersistantValues`).
#[must_use]
pub fn capture_persistant_values(source: &ClientPersistant) -> PersistantValues {
    PersistantValues {
        connected: source.connected,
        local_client: source.local_client,
        initial_spawn: source.initial_spawn,
        predict_item_pickup: source.predict_item_pickup,
        pmove_fixed: source.pmove_fixed,
        netname: source.netname.clone(),
        max_health: source.max_health,
        enter_time: source.enter_time,
        vote_count: source.vote_count,
        team_vote_count: source.team_vote_count,
        team_info: source.team_info,
    }
}

/// Restore persistant values (`restorePersistantValues`).
pub fn restore_persistant_values(target: &mut ClientPersistant, state: &PersistantValues) {
    target.connected = state.connected;
    target.local_client = state.local_client;
    target.initial_spawn = state.initial_spawn;
    target.predict_item_pickup = state.predict_item_pickup;
    target.pmove_fixed = state.pmove_fixed;
    target.netname = state.netname.clone();
    target.max_health = state.max_health;
    target.enter_time = state.enter_time;
    target.vote_count = state.vote_count;
    target.team_vote_count = state.team_vote_count;
    target.team_info = state.team_info;
}

/// Encode persistant values.
#[must_use]
pub fn persistant_values_to_json(state: &PersistantValues) -> SaveJson {
    obj(vec![
        ("connected", num_i32(state.connected)),
        ("localClient", boolean(state.local_client)),
        ("initialSpawn", boolean(state.initial_spawn)),
        ("predictItemPickup", boolean(state.predict_item_pickup)),
        ("pmoveFixed", boolean(state.pmove_fixed)),
        ("netname", str(&state.netname)),
        ("maxHealth", num_i32(state.max_health)),
        ("enterTime", num_i32(state.enter_time)),
        ("voteCount", num_i32(state.vote_count)),
        ("teamVoteCount", num_i32(state.team_vote_count)),
        ("teamInfo", boolean(state.team_info)),
    ])
}

/// Read persistant values (`readPersistantValues`).
pub fn read_persistant_values(reader: &SaveReader) -> Result<PersistantValues, Q3GameError> {
    Ok(PersistantValues {
        connected: read_i32(reader, "connected")?,
        local_client: reader.field("localClient").boolean()?,
        initial_spawn: reader.field("initialSpawn").boolean()?,
        predict_item_pickup: reader.field("predictItemPickup").boolean()?,
        pmove_fixed: reader.field("pmoveFixed").boolean()?,
        netname: reader.field("netname").string()?,
        max_health: read_i32(reader, "maxHealth")?,
        enter_time: read_i32(reader, "enterTime")?,
        vote_count: read_i32(reader, "voteCount")?,
        team_vote_count: read_i32(reader, "teamVoteCount")?,
        team_info: reader.field("teamInfo").boolean()?,
    })
}

/// Team values (`TeamValues`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TeamValues {
    /// State.
    pub state: i32,
    /// Location.
    pub location: i32,
    /// Captures.
    pub captures: i32,
    /// Base defense.
    pub base_defense: i32,
    /// Carrier defense.
    pub carrier_defense: i32,
    /// Flag recovery.
    pub flag_recovery: i32,
    /// Frag carrier.
    pub frag_carrier: i32,
    /// Assists.
    pub assists: i32,
    /// Last hurt carrier.
    pub last_hurt_carrier: i32,
    /// Last returned flag.
    pub last_returned_flag: i32,
    /// Flag since.
    pub flag_since: i32,
    /// Last fragged carrier.
    pub last_fragged_carrier: i32,
}

/// Capture team values (`captureTeamValues`).
#[must_use]
pub fn capture_team_values(source: &PlayerTeamState) -> TeamValues {
    TeamValues {
        state: source.state,
        location: source.location,
        captures: source.captures,
        base_defense: source.base_defense,
        carrier_defense: source.carrier_defense,
        flag_recovery: source.flag_recovery,
        frag_carrier: source.frag_carrier,
        assists: source.assists,
        last_hurt_carrier: source.last_hurt_carrier,
        last_returned_flag: source.last_returned_flag,
        flag_since: source.flag_since,
        last_fragged_carrier: source.last_fragged_carrier,
    }
}

/// Restore team values (`restoreTeamValues`).
pub fn restore_team_values(target: &mut PlayerTeamState, state: &TeamValues) {
    target.state = state.state;
    target.location = state.location;
    target.captures = state.captures;
    target.base_defense = state.base_defense;
    target.carrier_defense = state.carrier_defense;
    target.flag_recovery = state.flag_recovery;
    target.frag_carrier = state.frag_carrier;
    target.assists = state.assists;
    target.last_hurt_carrier = state.last_hurt_carrier;
    target.last_returned_flag = state.last_returned_flag;
    target.flag_since = state.flag_since;
    target.last_fragged_carrier = state.last_fragged_carrier;
}

/// Encode team values.
#[must_use]
pub fn team_values_to_json(state: &TeamValues) -> SaveJson {
    obj(vec![
        ("state", num_i32(state.state)),
        ("location", num_i32(state.location)),
        ("captures", num_i32(state.captures)),
        ("baseDefense", num_i32(state.base_defense)),
        ("carrierDefense", num_i32(state.carrier_defense)),
        ("flagRecovery", num_i32(state.flag_recovery)),
        ("fragCarrier", num_i32(state.frag_carrier)),
        ("assists", num_i32(state.assists)),
        ("lastHurtCarrier", num_i32(state.last_hurt_carrier)),
        ("lastReturnedFlag", num_i32(state.last_returned_flag)),
        ("flagSince", num_i32(state.flag_since)),
        ("lastFraggedCarrier", num_i32(state.last_fragged_carrier)),
    ])
}

/// Read team values (`readTeamValues`).
pub fn read_team_values(reader: &SaveReader) -> Result<TeamValues, Q3GameError> {
    Ok(TeamValues {
        state: read_i32(reader, "state")?,
        location: read_i32(reader, "location")?,
        captures: read_i32(reader, "captures")?,
        base_defense: read_i32(reader, "baseDefense")?,
        carrier_defense: read_i32(reader, "carrierDefense")?,
        flag_recovery: read_i32(reader, "flagRecovery")?,
        frag_carrier: read_i32(reader, "fragCarrier")?,
        assists: read_i32(reader, "assists")?,
        last_hurt_carrier: read_i32(reader, "lastHurtCarrier")?,
        last_returned_flag: read_i32(reader, "lastReturnedFlag")?,
        flag_since: read_i32(reader, "flagSince")?,
        last_fragged_carrier: read_i32(reader, "lastFraggedCarrier")?,
    })
}

/// Session values (`SessionValues`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionValues {
    /// Session team.
    pub session_team: i32,
    /// Spectator time.
    pub spectator_time: i32,
    /// Spectator state.
    pub spectator_state: i32,
    /// Spectator client.
    pub spectator_client: i32,
    /// Wins.
    pub wins: i32,
    /// Losses.
    pub losses: i32,
    /// Team leader.
    pub team_leader: i32,
}

/// Capture session values (`captureSessionValues`).
#[must_use]
pub fn capture_session_values(source: &ClientSession) -> SessionValues {
    SessionValues {
        session_team: source.session_team,
        spectator_time: source.spectator_time,
        spectator_state: source.spectator_state,
        spectator_client: source.spectator_client,
        wins: source.wins,
        losses: source.losses,
        team_leader: source.team_leader,
    }
}

/// Restore session values (`restoreSessionValues`).
pub fn restore_session_values(target: &mut ClientSession, state: &SessionValues) {
    target.session_team = state.session_team;
    target.spectator_time = state.spectator_time;
    target.spectator_state = state.spectator_state;
    target.spectator_client = state.spectator_client;
    target.wins = state.wins;
    target.losses = state.losses;
    target.team_leader = state.team_leader;
}

/// Encode session values.
#[must_use]
pub fn session_values_to_json(state: &SessionValues) -> SaveJson {
    obj(vec![
        ("sessionTeam", num_i32(state.session_team)),
        ("spectatorTime", num_i32(state.spectator_time)),
        ("spectatorState", num_i32(state.spectator_state)),
        ("spectatorClient", num_i32(state.spectator_client)),
        ("wins", num_i32(state.wins)),
        ("losses", num_i32(state.losses)),
        ("teamLeader", num_i32(state.team_leader)),
    ])
}

/// Read session values (`readSessionValues`).
pub fn read_session_values(reader: &SaveReader) -> Result<SessionValues, Q3GameError> {
    Ok(SessionValues {
        session_team: read_i32(reader, "sessionTeam")?,
        spectator_time: read_i32(reader, "spectatorTime")?,
        spectator_state: read_i32(reader, "spectatorState")?,
        spectator_client: read_i32(reader, "spectatorClient")?,
        wins: read_i32(reader, "wins")?,
        losses: read_i32(reader, "losses")?,
        team_leader: read_i32(reader, "teamLeader")?,
    })
}

/// Network values (`NetworkValues`).
#[derive(Debug, Clone, PartialEq)]
pub struct NetworkValues {
    /// Number.
    pub number: i32,
    /// Entity type.
    pub e_type: i32,
    /// Flags.
    pub e_flags: i32,
    /// Time.
    pub time: i32,
    /// Time 2.
    pub time2: i32,
    /// Origin.
    pub origin: Vec3,
    /// Origin 2.
    pub origin2: Vec3,
    /// Angles.
    pub angles: Vec3,
    /// Angles 2.
    pub angles2: Vec3,
    /// Other entity number.
    pub other_entity_num: i32,
    /// Other entity number 2.
    pub other_entity_num2: i32,
    /// Ground entity number.
    pub ground_entity_num: i32,
    /// Constant light.
    pub constant_light: i32,
    /// Loop sound.
    pub loop_sound: i32,
    /// Model index.
    pub modelindex: i32,
    /// Model index 2.
    pub modelindex2: i32,
    /// Client number.
    pub client_num: i32,
    /// Frame.
    pub frame: i32,
    /// Solid.
    pub solid: i32,
    /// Event.
    pub event: i32,
    /// Event parameter.
    pub event_parm: i32,
    /// Powerups.
    pub powerups: i32,
    /// Weapon.
    pub weapon: i32,
    /// Legs animation.
    pub legs_anim: i32,
    /// Torso animation.
    pub torso_anim: i32,
    /// Generic 1.
    pub generic1: i32,
}

/// Capture network values (`captureNetworkValues`).
#[must_use]
pub fn capture_network_values(source: &Q3EntityState) -> NetworkValues {
    NetworkValues {
        number: source.number,
        e_type: source.e_type,
        e_flags: source.e_flags,
        time: source.time,
        time2: source.time2,
        origin: source.origin,
        origin2: source.origin2,
        angles: source.angles,
        angles2: source.angles2,
        other_entity_num: source.other_entity_num,
        other_entity_num2: source.other_entity_num2,
        ground_entity_num: source.ground_entity_num,
        constant_light: source.constant_light,
        loop_sound: source.loop_sound,
        modelindex: source.modelindex,
        modelindex2: source.modelindex2,
        client_num: source.client_num,
        frame: source.frame,
        solid: source.solid,
        event: source.event,
        event_parm: source.event_parm,
        powerups: source.powerups,
        weapon: source.weapon,
        legs_anim: source.legs_anim,
        torso_anim: source.torso_anim,
        generic1: source.generic1,
    }
}

/// Restore network values (`restoreNetworkValues`).
pub fn restore_network_values(target: &mut Q3EntityState, state: &NetworkValues) {
    target.number = state.number;
    target.e_type = state.e_type;
    target.e_flags = state.e_flags;
    target.time = state.time;
    target.time2 = state.time2;
    target.origin = state.origin;
    target.origin2 = state.origin2;
    target.angles = state.angles;
    target.angles2 = state.angles2;
    target.other_entity_num = state.other_entity_num;
    target.other_entity_num2 = state.other_entity_num2;
    target.ground_entity_num = state.ground_entity_num;
    target.constant_light = state.constant_light;
    target.loop_sound = state.loop_sound;
    target.modelindex = state.modelindex;
    target.modelindex2 = state.modelindex2;
    target.client_num = state.client_num;
    target.frame = state.frame;
    target.solid = state.solid;
    target.event = state.event;
    target.event_parm = state.event_parm;
    target.powerups = state.powerups;
    target.weapon = state.weapon;
    target.legs_anim = state.legs_anim;
    target.torso_anim = state.torso_anim;
    target.generic1 = state.generic1;
}

/// Encode network values.
#[must_use]
pub fn network_values_to_json(state: &NetworkValues) -> SaveJson {
    obj(vec![
        ("number", num_i32(state.number)),
        ("eType", num_i32(state.e_type)),
        ("eFlags", num_i32(state.e_flags)),
        ("time", num_i32(state.time)),
        ("time2", num_i32(state.time2)),
        ("origin", vec_to_json(state.origin)),
        ("origin2", vec_to_json(state.origin2)),
        ("angles", vec_to_json(state.angles)),
        ("angles2", vec_to_json(state.angles2)),
        ("otherEntityNum", num_i32(state.other_entity_num)),
        ("otherEntityNum2", num_i32(state.other_entity_num2)),
        ("groundEntityNum", num_i32(state.ground_entity_num)),
        ("constantLight", num_i32(state.constant_light)),
        ("loopSound", num_i32(state.loop_sound)),
        ("modelindex", num_i32(state.modelindex)),
        ("modelindex2", num_i32(state.modelindex2)),
        ("clientNum", num_i32(state.client_num)),
        ("frame", num_i32(state.frame)),
        ("solid", num_i32(state.solid)),
        ("event", num_i32(state.event)),
        ("eventParm", num_i32(state.event_parm)),
        ("powerups", num_i32(state.powerups)),
        ("weapon", num_i32(state.weapon)),
        ("legsAnim", num_i32(state.legs_anim)),
        ("torsoAnim", num_i32(state.torso_anim)),
        ("generic1", num_i32(state.generic1)),
    ])
}

/// Read network values (`readNetworkValues`).
pub fn read_network_values(reader: &SaveReader) -> Result<NetworkValues, Q3GameError> {
    Ok(NetworkValues {
        number: read_i32(reader, "number")?,
        e_type: read_i32(reader, "eType")?,
        e_flags: read_i32(reader, "eFlags")?,
        time: read_i32(reader, "time")?,
        time2: read_i32(reader, "time2")?,
        origin: read_vec(reader, "origin")?,
        origin2: read_vec(reader, "origin2")?,
        angles: read_vec(reader, "angles")?,
        angles2: read_vec(reader, "angles2")?,
        other_entity_num: read_i32(reader, "otherEntityNum")?,
        other_entity_num2: read_i32(reader, "otherEntityNum2")?,
        ground_entity_num: read_i32(reader, "groundEntityNum")?,
        constant_light: read_i32(reader, "constantLight")?,
        loop_sound: read_i32(reader, "loopSound")?,
        modelindex: read_i32(reader, "modelindex")?,
        modelindex2: read_i32(reader, "modelindex2")?,
        client_num: read_i32(reader, "clientNum")?,
        frame: read_i32(reader, "frame")?,
        solid: read_i32(reader, "solid")?,
        event: read_i32(reader, "event")?,
        event_parm: read_i32(reader, "eventParm")?,
        powerups: read_i32(reader, "powerups")?,
        weapon: read_i32(reader, "weapon")?,
        legs_anim: read_i32(reader, "legsAnim")?,
        torso_anim: read_i32(reader, "torsoAnim")?,
        generic1: read_i32(reader, "generic1")?,
    })
}

/// Level values (`LevelValues`).
#[derive(Debug, Clone, PartialEq)]
pub struct LevelValues {
    /// Time.
    pub time: i32,
    /// Start time.
    pub start_time: i32,
    /// Warmup time.
    pub warmup_time: i32,
    /// Warmup modification count.
    pub warmup_modification_count: i32,
    /// Restarted.
    pub restarted: bool,
    /// Connected clients.
    pub num_connected_clients: i32,
    /// Non-spectator clients.
    pub num_non_spectator_clients: i32,
    /// Playing clients.
    pub num_playing_clients: i32,
    /// Voting clients.
    pub num_voting_clients: i32,
    /// Follow 1.
    pub follow1: i32,
    /// Follow 2.
    pub follow2: i32,
    /// Intermission time.
    pub intermission_time: i32,
    /// Intermission queued.
    pub intermission_queued: i32,
    /// Intermission origin.
    pub intermission_origin: Vec3,
    /// Intermission angle.
    pub intermission_angle: Vec3,
    /// Change map.
    pub changemap: Option<String>,
    /// Ready to exit.
    pub ready_to_exit: bool,
    /// Exit time.
    pub exit_time: i32,
    /// Frame number.
    pub frame_num: i32,
    /// Previous time.
    pub previous_time: i32,
    /// New session.
    pub new_session: bool,
    /// Fry sound.
    pub fry_sound: i32,
}

/// Capture level values (`captureLevelValues`).
#[must_use]
pub fn capture_level_values(source: &Q3GameLevel) -> LevelValues {
    LevelValues {
        time: source.time,
        start_time: source.start_time,
        warmup_time: source.warmup_time,
        warmup_modification_count: source.warmup_modification_count,
        restarted: source.restarted,
        num_connected_clients: source.num_connected_clients,
        num_non_spectator_clients: source.num_non_spectator_clients,
        num_playing_clients: source.num_playing_clients,
        num_voting_clients: source.num_voting_clients,
        follow1: source.follow1,
        follow2: source.follow2,
        intermission_time: source.intermission_time,
        intermission_queued: source.intermission_queued,
        intermission_origin: source.intermission_origin,
        intermission_angle: source.intermission_angle,
        changemap: source.changemap.clone(),
        ready_to_exit: source.ready_to_exit,
        exit_time: source.exit_time,
        frame_num: source.frame_num,
        previous_time: source.previous_time,
        new_session: source.new_session,
        fry_sound: source.fry_sound,
    }
}

/// Restore level values (`restoreLevelValues`).
pub fn restore_level_values(target: &mut Q3GameLevel, state: &LevelValues) {
    target.time = state.time;
    target.start_time = state.start_time;
    target.warmup_time = state.warmup_time;
    target.warmup_modification_count = state.warmup_modification_count;
    target.restarted = state.restarted;
    target.num_connected_clients = state.num_connected_clients;
    target.num_non_spectator_clients = state.num_non_spectator_clients;
    target.num_playing_clients = state.num_playing_clients;
    target.num_voting_clients = state.num_voting_clients;
    target.follow1 = state.follow1;
    target.follow2 = state.follow2;
    target.intermission_time = state.intermission_time;
    target.intermission_queued = state.intermission_queued;
    target.intermission_origin = state.intermission_origin;
    target.intermission_angle = state.intermission_angle;
    target.changemap = state.changemap.clone();
    target.ready_to_exit = state.ready_to_exit;
    target.exit_time = state.exit_time;
    target.frame_num = state.frame_num;
    target.previous_time = state.previous_time;
    target.new_session = state.new_session;
    target.fry_sound = state.fry_sound;
}

/// Encode level values.
#[must_use]
pub fn level_values_to_json(state: &LevelValues) -> SaveJson {
    obj(vec![
        ("time", num_i32(state.time)),
        ("startTime", num_i32(state.start_time)),
        ("warmupTime", num_i32(state.warmup_time)),
        ("warmupModificationCount", num_i32(state.warmup_modification_count)),
        ("restarted", boolean(state.restarted)),
        ("numConnectedClients", num_i32(state.num_connected_clients)),
        ("numNonSpectatorClients", num_i32(state.num_non_spectator_clients)),
        ("numPlayingClients", num_i32(state.num_playing_clients)),
        ("numVotingClients", num_i32(state.num_voting_clients)),
        ("follow1", num_i32(state.follow1)),
        ("follow2", num_i32(state.follow2)),
        ("intermissionTime", num_i32(state.intermission_time)),
        ("intermissionQueued", num_i32(state.intermission_queued)),
        ("intermissionOrigin", vec_to_json(state.intermission_origin)),
        ("intermissionAngle", vec_to_json(state.intermission_angle)),
        ("changemap", opt_str_to_json(state.changemap.as_deref())),
        ("readyToExit", boolean(state.ready_to_exit)),
        ("exitTime", num_i32(state.exit_time)),
        ("frameNum", num_i32(state.frame_num)),
        ("previousTime", num_i32(state.previous_time)),
        ("newSession", boolean(state.new_session)),
        ("frySound", num_i32(state.fry_sound)),
    ])
}

/// Read level values (`readLevelValues`).
pub fn read_level_values(reader: &SaveReader) -> Result<LevelValues, Q3GameError> {
    Ok(LevelValues {
        time: read_i32(reader, "time")?,
        start_time: read_i32(reader, "startTime")?,
        warmup_time: read_i32(reader, "warmupTime")?,
        warmup_modification_count: read_i32(reader, "warmupModificationCount")?,
        restarted: reader.field("restarted").boolean()?,
        num_connected_clients: read_i32(reader, "numConnectedClients")?,
        num_non_spectator_clients: read_i32(reader, "numNonSpectatorClients")?,
        num_playing_clients: read_i32(reader, "numPlayingClients")?,
        num_voting_clients: read_i32(reader, "numVotingClients")?,
        follow1: read_i32(reader, "follow1")?,
        follow2: read_i32(reader, "follow2")?,
        intermission_time: read_i32(reader, "intermissionTime")?,
        intermission_queued: read_i32(reader, "intermissionQueued")?,
        intermission_origin: read_vec(reader, "intermissionOrigin")?,
        intermission_angle: read_vec(reader, "intermissionAngle")?,
        changemap: read_opt_string(reader, "changemap")?,
        ready_to_exit: reader.field("readyToExit").boolean()?,
        exit_time: read_i32(reader, "exitTime")?,
        frame_num: read_i32(reader, "frameNum")?,
        previous_time: read_i32(reader, "previousTime")?,
        new_session: reader.field("newSession").boolean()?,
        fry_sound: read_i32(reader, "frySound")?,
    })
}

/// Read a vector (`readVector`).
pub fn read_save_vector(reader: &SaveReader) -> Result<Vec3, Q3GameError> {
    vec_from_reader(reader)
}
