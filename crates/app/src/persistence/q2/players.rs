//! Quake II player checkpoint ported from `src/persistence/q2-players.ts`.

use qa_core::identity::SavedActorId;
use qa_core::math::Vec3;
use qa_world::combat::ArmorState;
use qa_world::inventory::InventoryEntry;
use qa_world::save::records::{
    read_armor, read_inventory_entry, read_saved_actor, write_armor, write_inventory_entry, write_saved_actor,
};
use qa_world::save::shared::{read_vector, write_vector};
use qa_world::save::value::{
    arr, boolean, decode_checkpoint_value, encode_checkpoint_value, int, namespaced, num, obj, str, SaveJson,
    SaveReader,
};

use super::super::PersistenceError;
use super::foundation::{read_q2_attack_checkpoint, write_q2_attack_checkpoint, Q2AttackCheckpoint};

/// Saved player carry (coop respawn / intermission).
#[derive(Debug, Clone, PartialEq)]
pub struct Q2PlayerCarry {
    /// Health.
    pub health: f64,
    /// Maximum health.
    pub maximum_health: f64,
    /// Armor.
    pub armor: ArmorState,
    /// Inventory.
    pub inventory: Vec<InventoryEntry>,
    /// Weapon.
    pub weapon: Option<String>,
    /// Selected item.
    pub selected_item: Option<String>,
    /// Score.
    pub score: f64,
    /// Flags.
    pub flags: f64,
    /// Power cubes.
    pub power_cubes: f64,
}

fn read_carry(reader: SaveReader) -> Result<Q2PlayerCarry, PersistenceError> {
    Ok(Q2PlayerCarry {
        health: reader.field("health").number()?,
        maximum_health: reader.field("maximumHealth").number()?,
        armor: read_armor(reader.field("armor"))?,
        inventory: reader
            .field("inventory")
            .list(|value| read_inventory_entry(value).map_err(PersistenceError::from))?,
        weapon: reader
            .field("weapon")
            .nullable(|value| value.string().map_err(PersistenceError::from))?,
        selected_item: reader
            .field("selectedItem")
            .nullable(|value| namespaced(value).map_err(PersistenceError::from))?,
        score: reader.field("score").number()?,
        flags: reader.field("flags").number()?,
        power_cubes: reader.field("powerCubes").number()?,
    })
}

fn write_carry(carry: &Q2PlayerCarry) -> SaveJson {
    obj(vec![
        ("health", num(carry.health)),
        ("maximumHealth", num(carry.maximum_health)),
        ("armor", write_armor(&carry.armor)),
        (
            "inventory",
            arr(carry.inventory.iter().map(write_inventory_entry).collect()),
        ),
        (
            "weapon",
            carry.weapon.as_ref().map_or(SaveJson::Null, |weapon| str(weapon)),
        ),
        (
            "selectedItem",
            carry.selected_item.as_ref().map_or(SaveJson::Null, |item| str(item)),
        ),
        ("score", num(carry.score)),
        ("flags", num(carry.flags)),
        ("powerCubes", num(carry.power_cubes)),
    ])
}

/// Saved player rules.
#[derive(Debug, Clone, PartialEq)]
pub struct Q2PlayerRules {
    /// Password.
    pub password: String,
    /// Spectator password.
    pub spectator_password: String,
    /// Maximum spectators.
    pub max_spectators: f64,
    /// Cheats enabled.
    pub cheats: bool,
    /// Time limit minutes.
    pub time_limit_minutes: f64,
    /// Frag limit.
    pub frag_limit: f64,
    /// Map list.
    pub map_list: Vec<String>,
    /// Map list shuffle.
    pub map_list_shuffle: bool,
    /// Next map.
    pub next_map: String,
    /// Spawn point.
    pub spawn_point: String,
    /// Flood messages.
    pub flood_messages: f64,
    /// Flood seconds.
    pub flood_seconds: f64,
    /// Flood wait seconds.
    pub flood_wait_seconds: f64,
    /// Roll speed.
    pub roll_speed: f64,
    /// Roll angle.
    pub roll_angle: f64,
    /// Run pitch.
    pub run_pitch: f64,
    /// Run roll.
    pub run_roll: f64,
    /// Bob up.
    pub bob_up: f64,
    /// Bob pitch.
    pub bob_pitch: f64,
    /// Bob roll.
    pub bob_roll: f64,
    /// Gun offset.
    pub gun_offset: Vec3,
}

fn read_rules(reader: SaveReader) -> Result<Q2PlayerRules, PersistenceError> {
    Ok(Q2PlayerRules {
        password: reader.field("password").string()?,
        spectator_password: reader.field("spectatorPassword").string()?,
        max_spectators: reader.field("maxSpectators").number()?,
        cheats: reader.field("cheats").boolean()?,
        time_limit_minutes: reader.field("timeLimitMinutes").number()?,
        frag_limit: reader.field("fragLimit").number()?,
        map_list: reader
            .field("mapList")
            .list(|value| value.string().map_err(PersistenceError::from))?,
        map_list_shuffle: if reader.field("mapListShuffle").is_missing() {
            false
        } else {
            reader.field("mapListShuffle").boolean()?
        },
        next_map: reader.field("nextMap").string()?,
        spawn_point: reader.field("spawnPoint").string()?,
        flood_messages: reader.field("floodMessages").number()?,
        flood_seconds: reader.field("floodSeconds").number()?,
        flood_wait_seconds: reader.field("floodWaitSeconds").number()?,
        roll_speed: reader.field("rollSpeed").number()?,
        roll_angle: reader.field("rollAngle").number()?,
        run_pitch: reader.field("runPitch").number()?,
        run_roll: reader.field("runRoll").number()?,
        bob_up: reader.field("bobUp").number()?,
        bob_pitch: reader.field("bobPitch").number()?,
        bob_roll: reader.field("bobRoll").number()?,
        gun_offset: read_vector(reader.field("gunOffset"))?,
    })
}

fn write_rules(rules: &Q2PlayerRules) -> SaveJson {
    obj(vec![
        ("password", str(&rules.password)),
        ("spectatorPassword", str(&rules.spectator_password)),
        ("maxSpectators", num(rules.max_spectators)),
        ("cheats", boolean(rules.cheats)),
        ("timeLimitMinutes", num(rules.time_limit_minutes)),
        ("fragLimit", num(rules.frag_limit)),
        ("mapList", arr(rules.map_list.iter().map(|map| str(map)).collect())),
        ("mapListShuffle", boolean(rules.map_list_shuffle)),
        ("nextMap", str(&rules.next_map)),
        ("spawnPoint", str(&rules.spawn_point)),
        ("floodMessages", num(rules.flood_messages)),
        ("floodSeconds", num(rules.flood_seconds)),
        ("floodWaitSeconds", num(rules.flood_wait_seconds)),
        ("rollSpeed", num(rules.roll_speed)),
        ("rollAngle", num(rules.roll_angle)),
        ("runPitch", num(rules.run_pitch)),
        ("runRoll", num(rules.run_roll)),
        ("bobUp", num(rules.bob_up)),
        ("bobPitch", num(rules.bob_pitch)),
        ("bobRoll", num(rules.bob_roll)),
        ("gunOffset", write_vector(rules.gun_offset)),
    ])
}

/// Saved intermission landmark.
#[derive(Debug, Clone, PartialEq)]
pub struct Q2IntermissionLandmark {
    /// Player.
    pub player: SavedActorId,
    /// Name.
    pub name: String,
    /// Relative origin.
    pub relative_origin: Vec3,
    /// Relative velocity.
    pub relative_velocity: Vec3,
    /// Relative view angles.
    pub relative_view_angles: Vec3,
}

/// Saved intermission checkpoint.
#[derive(Debug, Clone, PartialEq)]
pub enum Q2PlayerIntermission {
    /// Playing.
    Playing,
    /// Intermission.
    Intermission {
        /// Map.
        map: String,
        /// Started time.
        started: f64,
        /// Exit flag.
        exit: bool,
        /// Landmark.
        landmark: Option<Q2IntermissionLandmark>,
    },
}

fn read_intermission(reader: SaveReader) -> Result<Q2PlayerIntermission, PersistenceError> {
    if reader.field("kind").choice_str(&["playing", "intermission"])? == "playing" {
        return Ok(Q2PlayerIntermission::Playing);
    }
    Ok(Q2PlayerIntermission::Intermission {
        map: reader.field("map").string()?,
        started: reader.field("started").number()?,
        exit: reader.field("exit").boolean()?,
        landmark: reader
            .field("landmark")
            .nullable(|value| -> Result<Q2IntermissionLandmark, PersistenceError> {
                Ok(Q2IntermissionLandmark {
                    player: read_saved_actor(value.field("player"))?,
                    name: value.field("name").string()?,
                    relative_origin: read_vector(value.field("relativeOrigin"))?,
                    relative_velocity: read_vector(value.field("relativeVelocity"))?,
                    relative_view_angles: read_vector(value.field("relativeViewAngles"))?,
                })
            })?,
    })
}

fn write_intermission(intermission: &Q2PlayerIntermission) -> SaveJson {
    match intermission {
        Q2PlayerIntermission::Playing => obj(vec![("kind", str("playing"))]),
        Q2PlayerIntermission::Intermission {
            map,
            started,
            exit,
            landmark,
        } => obj(vec![
            ("kind", str("intermission")),
            ("map", str(map)),
            ("started", num(*started)),
            ("exit", boolean(*exit)),
            (
                "landmark",
                landmark.as_ref().map_or(SaveJson::Null, |landmark| {
                    obj(vec![
                        ("player", write_saved_actor(landmark.player)),
                        ("name", str(&landmark.name)),
                        ("relativeOrigin", write_vector(landmark.relative_origin)),
                        ("relativeVelocity", write_vector(landmark.relative_velocity)),
                        ("relativeViewAngles", write_vector(landmark.relative_view_angles)),
                    ])
                }),
            ),
        ]),
    }
}

/// Saved Q2 player state.
#[derive(Debug, Clone, PartialEq)]
pub struct Q2PlayerStateCheckpoint {
    /// Slot.
    pub slot: u64,
    /// Entered at.
    pub entered_at: f64,
    /// Use Q2 weapons.
    pub use_q2_weapons: bool,
    /// Use Q2 inventory.
    pub use_q2_inventory: bool,
    /// Spawn inventory.
    pub spawn_inventory: Vec<InventoryEntry>,
    /// Userinfo.
    pub userinfo: String,
    /// Name.
    pub name: String,
    /// Skin.
    pub skin: String,
    /// Gender.
    pub gender: String,
    /// Field of view.
    pub fov: f64,
    /// Hand.
    pub hand: String,
    /// Spectator flag.
    pub spectator: bool,
    /// Requested spectator flag.
    pub requested_spectator: bool,
    /// Connected flag.
    pub connected: bool,
    /// Dead flag.
    pub dead: bool,
    /// Gibbed flag.
    pub gibbed: bool,
    /// Noclip flag.
    pub noclip: bool,
    /// God flag.
    pub god: bool,
    /// Notarget flag.
    pub notarget: bool,
    /// Score.
    pub score: f64,
    /// Ping.
    pub ping: f64,
    /// Respawn time.
    pub respawn_time: f64,
    /// Air finished.
    pub air_finished: f64,
    /// Next drown time.
    pub next_drown_time: f64,
    /// Drown damage.
    pub drown_damage: f64,
    /// Old water level.
    pub old_water_level: f64,
    /// Breather sound.
    pub breather_sound: f64,
    /// Pain debounce.
    pub pain_debounce: f64,
    /// Damage blood.
    pub damage_blood: f64,
    /// Damage armor.
    pub damage_armor: f64,
    /// Damage power armor.
    pub damage_power_armor: f64,
    /// Damage knockback.
    pub damage_knockback: f64,
    /// Damage from.
    pub damage_from: Vec3,
    /// Damage blend.
    pub damage_blend: Vec3,
    /// Damage alpha.
    pub damage_alpha: f64,
    /// Bonus alpha.
    pub bonus_alpha: f64,
    /// Damage pitch.
    pub damage_pitch: f64,
    /// Damage roll.
    pub damage_roll: f64,
    /// Damage time.
    pub damage_time: f64,
    /// Power-armor time.
    pub power_armor_time: f64,
    /// Fall time.
    pub fall_time: f64,
    /// Fall value.
    pub fall_value: f64,
    /// Landmark free fall.
    pub landmark_free_fall: bool,
    /// Landmark noise time.
    pub landmark_noise_time: f64,
    /// Old velocity.
    pub old_velocity: Vec3,
    /// Old view angles.
    pub old_view_angles: Vec3,
    /// Killer yaw.
    pub killer_yaw: f64,
    /// Buttons.
    pub buttons: f64,
    /// Latched buttons.
    pub latched_buttons: f64,
    /// Weapon thunk.
    pub weapon_thunk: bool,
    /// Bob time.
    pub bob_time: f64,
    /// Bob move.
    pub bob_move: f64,
    /// Event.
    pub event: String,
    /// Animation priority.
    pub animation_priority: f64,
    /// Animation end.
    pub animation_end: f64,
    /// Animation duck.
    pub animation_duck: bool,
    /// Animation run.
    pub animation_run: bool,
    /// Loop sound.
    pub loop_sound: String,
    /// Selected item.
    pub selected_item: Option<String>,
    /// Show scores.
    pub show_scores: bool,
    /// Show inventory.
    pub show_inventory: bool,
    /// Show help.
    pub show_help: bool,
    /// Chase target.
    pub chase_target: Option<SavedActorId>,
    /// Coop respawn carry.
    pub coop_respawn: Option<Q2PlayerCarry>,
    /// Flood times.
    pub flood_times: Vec<f64>,
    /// Flood lock until.
    pub flood_lock_until: f64,
}

/// Read a Q2 player state checkpoint.
pub fn read_q2_player_state_checkpoint(reader: SaveReader) -> Result<Q2PlayerStateCheckpoint, PersistenceError> {
    let number = |name: &str| reader.field(name).number().map_err(PersistenceError::from);
    let text = |name: &str| reader.field(name).string().map_err(PersistenceError::from);
    let flag = |name: &str| reader.field(name).boolean().map_err(PersistenceError::from);
    let vector = |name: &str| read_vector(reader.field(name)).map_err(PersistenceError::from);
    let slot = reader.field("slot").integer(0)?;
    Ok(Q2PlayerStateCheckpoint {
        slot: u64::try_from(slot)
            .map_err(|_| PersistenceError::from(reader.field("slot").fail("expected an integer in range")))?,
        entered_at: number("enteredAt")?,
        use_q2_weapons: flag("useQ2Weapons")?,
        use_q2_inventory: flag("useQ2Inventory")?,
        spawn_inventory: reader
            .field("spawnInventory")
            .list(|value| read_inventory_entry(value).map_err(PersistenceError::from))?,
        userinfo: text("userinfo")?,
        name: text("name")?,
        skin: text("skin")?,
        gender: reader.field("gender").choice_str(&["male", "female", "neutral"])?,
        fov: number("fov")?,
        hand: reader.field("hand").choice_str(&["right", "left", "center"])?,
        spectator: flag("spectator")?,
        requested_spectator: flag("requestedSpectator")?,
        connected: flag("connected")?,
        dead: flag("dead")?,
        gibbed: flag("gibbed")?,
        noclip: flag("noclip")?,
        god: flag("god")?,
        notarget: flag("notarget")?,
        score: number("score")?,
        ping: number("ping")?,
        respawn_time: number("respawnTime")?,
        air_finished: number("airFinished")?,
        next_drown_time: number("nextDrownTime")?,
        drown_damage: number("drownDamage")?,
        old_water_level: number("oldWaterLevel")?,
        breather_sound: number("breatherSound")?,
        pain_debounce: number("painDebounce")?,
        damage_blood: number("damageBlood")?,
        damage_armor: number("damageArmor")?,
        damage_power_armor: number("damagePowerArmor")?,
        damage_knockback: number("damageKnockback")?,
        damage_from: vector("damageFrom")?,
        damage_blend: vector("damageBlend")?,
        damage_alpha: number("damageAlpha")?,
        bonus_alpha: number("bonusAlpha")?,
        damage_pitch: number("damagePitch")?,
        damage_roll: number("damageRoll")?,
        damage_time: number("damageTime")?,
        power_armor_time: number("powerArmorTime")?,
        fall_time: number("fallTime")?,
        fall_value: number("fallValue")?,
        landmark_free_fall: flag("landmarkFreeFall")?,
        landmark_noise_time: number("landmarkNoiseTime")?,
        old_velocity: vector("oldVelocity")?,
        old_view_angles: vector("oldViewAngles")?,
        killer_yaw: number("killerYaw")?,
        buttons: number("buttons")?,
        latched_buttons: number("latchedButtons")?,
        weapon_thunk: flag("weaponThunk")?,
        bob_time: number("bobTime")?,
        bob_move: number("bobMove")?,
        event: text("event")?,
        animation_priority: number("animationPriority")?,
        animation_end: number("animationEnd")?,
        animation_duck: flag("animationDuck")?,
        animation_run: flag("animationRun")?,
        loop_sound: text("loopSound")?,
        selected_item: reader
            .field("selectedItem")
            .nullable(|value| namespaced(value).map_err(PersistenceError::from))?,
        show_scores: flag("showScores")?,
        show_inventory: flag("showInventory")?,
        show_help: flag("showHelp")?,
        chase_target: reader
            .field("chaseTarget")
            .nullable(|value| read_saved_actor(value).map_err(PersistenceError::from))?,
        coop_respawn: reader.field("coopRespawn").nullable(read_carry)?,
        flood_times: reader
            .field("floodTimes")
            .list(|value| value.number().map_err(PersistenceError::from))?,
        flood_lock_until: number("floodLockUntil")?,
    })
}

/// Write a Q2 player state checkpoint.
#[must_use]
#[allow(clippy::too_many_lines)]
pub fn write_q2_player_state_checkpoint(state: &Q2PlayerStateCheckpoint) -> SaveJson {
    #[allow(clippy::cast_possible_wrap)]
    obj(vec![
        ("slot", int(state.slot as i64)),
        ("enteredAt", num(state.entered_at)),
        ("useQ2Weapons", boolean(state.use_q2_weapons)),
        ("useQ2Inventory", boolean(state.use_q2_inventory)),
        (
            "spawnInventory",
            arr(state.spawn_inventory.iter().map(write_inventory_entry).collect()),
        ),
        ("userinfo", str(&state.userinfo)),
        ("name", str(&state.name)),
        ("skin", str(&state.skin)),
        ("gender", str(&state.gender)),
        ("fov", num(state.fov)),
        ("hand", str(&state.hand)),
        ("spectator", boolean(state.spectator)),
        ("requestedSpectator", boolean(state.requested_spectator)),
        ("connected", boolean(state.connected)),
        ("dead", boolean(state.dead)),
        ("gibbed", boolean(state.gibbed)),
        ("noclip", boolean(state.noclip)),
        ("god", boolean(state.god)),
        ("notarget", boolean(state.notarget)),
        ("score", num(state.score)),
        ("ping", num(state.ping)),
        ("respawnTime", num(state.respawn_time)),
        ("airFinished", num(state.air_finished)),
        ("nextDrownTime", num(state.next_drown_time)),
        ("drownDamage", num(state.drown_damage)),
        ("oldWaterLevel", num(state.old_water_level)),
        ("breatherSound", num(state.breather_sound)),
        ("painDebounce", num(state.pain_debounce)),
        ("damageBlood", num(state.damage_blood)),
        ("damageArmor", num(state.damage_armor)),
        ("damagePowerArmor", num(state.damage_power_armor)),
        ("damageKnockback", num(state.damage_knockback)),
        ("damageFrom", write_vector(state.damage_from)),
        ("damageBlend", write_vector(state.damage_blend)),
        ("damageAlpha", num(state.damage_alpha)),
        ("bonusAlpha", num(state.bonus_alpha)),
        ("damagePitch", num(state.damage_pitch)),
        ("damageRoll", num(state.damage_roll)),
        ("damageTime", num(state.damage_time)),
        ("powerArmorTime", num(state.power_armor_time)),
        ("fallTime", num(state.fall_time)),
        ("fallValue", num(state.fall_value)),
        ("landmarkFreeFall", boolean(state.landmark_free_fall)),
        ("landmarkNoiseTime", num(state.landmark_noise_time)),
        ("oldVelocity", write_vector(state.old_velocity)),
        ("oldViewAngles", write_vector(state.old_view_angles)),
        ("killerYaw", num(state.killer_yaw)),
        ("buttons", num(state.buttons)),
        ("latchedButtons", num(state.latched_buttons)),
        ("weaponThunk", boolean(state.weapon_thunk)),
        ("bobTime", num(state.bob_time)),
        ("bobMove", num(state.bob_move)),
        ("event", str(&state.event)),
        ("animationPriority", num(state.animation_priority)),
        ("animationEnd", num(state.animation_end)),
        ("animationDuck", boolean(state.animation_duck)),
        ("animationRun", boolean(state.animation_run)),
        ("loopSound", str(&state.loop_sound)),
        (
            "selectedItem",
            state.selected_item.as_ref().map_or(SaveJson::Null, |item| str(item)),
        ),
        ("showScores", boolean(state.show_scores)),
        ("showInventory", boolean(state.show_inventory)),
        ("showHelp", boolean(state.show_help)),
        (
            "chaseTarget",
            state.chase_target.map_or(SaveJson::Null, write_saved_actor),
        ),
        (
            "coopRespawn",
            state.coop_respawn.as_ref().map_or(SaveJson::Null, write_carry),
        ),
        (
            "floodTimes",
            arr(state.flood_times.iter().map(|time| num(*time)).collect()),
        ),
        ("floodLockUntil", num(state.flood_lock_until)),
    ])
}

/// Q2 players checkpoint.
#[derive(Debug, Clone, PartialEq)]
pub struct Q2PlayersCheckpoint {
    /// Corpse index.
    pub corpse_index: u64,
    /// Death animation.
    pub death_animation: f64,
    /// Pain animation.
    pub pain_animation: f64,
    /// Rules.
    pub rules: Q2PlayerRules,
    /// Intermission.
    pub intermission: Q2PlayerIntermission,
    /// Players.
    pub players: Vec<(SavedActorId, Q2PlayerStateCheckpoint)>,
}

/// Read a Q2 players checkpoint.
pub fn read_q2_players_checkpoint(reader: SaveReader) -> Result<Q2PlayersCheckpoint, PersistenceError> {
    reader.field("version").literal_i64(1)?;
    let corpse_index = reader.field("corpseIndex").integer(0)?;
    Ok(Q2PlayersCheckpoint {
        corpse_index: u64::try_from(corpse_index)
            .map_err(|_| PersistenceError::from(reader.field("corpseIndex").fail("expected an integer in range")))?,
        death_animation: reader.field("deathAnimation").number()?,
        pain_animation: reader.field("painAnimation").number()?,
        rules: read_rules(reader.field("rules"))?,
        intermission: read_intermission(reader.field("intermission"))?,
        players: reader.field("players").list(
            |value| -> Result<(SavedActorId, Q2PlayerStateCheckpoint), PersistenceError> {
                Ok((
                    read_saved_actor(value.field("actor"))?,
                    read_q2_player_state_checkpoint(value.field("state"))?,
                ))
            },
        )?,
    })
}

/// Write a Q2 players checkpoint.
#[must_use]
pub fn write_q2_players_checkpoint(checkpoint: &Q2PlayersCheckpoint) -> SaveJson {
    #[allow(clippy::cast_possible_wrap)]
    obj(vec![
        ("version", int(1)),
        ("corpseIndex", int(checkpoint.corpse_index as i64)),
        ("deathAnimation", num(checkpoint.death_animation)),
        ("painAnimation", num(checkpoint.pain_animation)),
        ("rules", write_rules(&checkpoint.rules)),
        ("intermission", write_intermission(&checkpoint.intermission)),
        (
            "players",
            arr(checkpoint
                .players
                .iter()
                .map(|(actor, state)| {
                    obj(vec![
                        ("actor", write_saved_actor(*actor)),
                        ("state", write_q2_player_state_checkpoint(state)),
                    ])
                })
                .collect()),
        ),
    ])
}

/// Encode a Q2 players checkpoint.
#[must_use]
pub fn encode_q2_players_checkpoint(checkpoint: &Q2PlayersCheckpoint) -> Vec<u8> {
    encode_checkpoint_value(&write_q2_players_checkpoint(checkpoint))
}

/// Decode a Q2 players checkpoint.
pub fn decode_q2_players_checkpoint(bytes: &[u8]) -> Result<Q2PlayersCheckpoint, PersistenceError> {
    let payload = decode_checkpoint_value(bytes)?;
    read_q2_players_checkpoint(SaveReader::at(&payload, "q2-players"))
}

/// Saved Q2 character entity view.
#[derive(Debug, Clone, PartialEq)]
pub struct Q2CharacterEntity {
    /// Model.
    pub model: String,
    /// Model 2.
    pub model2: String,
    /// Model 3.
    pub model3: String,
    /// Model 4.
    pub model4: String,
    /// Skin.
    pub skin: f64,
    /// Frame.
    pub frame: f64,
    /// Old frame.
    pub old_frame: f64,
    /// Scale.
    pub scale: f64,
    /// Effects.
    pub effects: f64,
    /// Render flags.
    pub render_flags: f64,
    /// Flags.
    pub flags: f64,
    /// Server flags.
    pub server_flags: f64,
    /// View height.
    pub view_height: f64,
    /// Maximum health.
    pub max_health: f64,
    /// Sound.
    pub sound: String,
    /// Visible flag.
    pub visible: bool,
}

/// Q2 character checkpoint.
#[derive(Debug, Clone, PartialEq)]
pub struct Q2CharacterCheckpoint {
    /// Pain index.
    pub pain_index: f64,
    /// Death index.
    pub death_index: f64,
    /// State.
    pub state: Q2PlayerStateCheckpoint,
    /// Rules.
    pub rules: Q2PlayerRules,
    /// Last attack.
    pub last_attack: Option<Q2AttackCheckpoint>,
    /// Entity view.
    pub entity: Q2CharacterEntity,
}

/// Read a Q2 character checkpoint.
pub fn read_q2_character_checkpoint(reader: SaveReader) -> Result<Q2CharacterCheckpoint, PersistenceError> {
    reader.field("version").literal_i64(1)?;
    let entity = reader.field("entity");
    Ok(Q2CharacterCheckpoint {
        pain_index: reader.field("painIndex").number()?,
        death_index: reader.field("deathIndex").number()?,
        state: read_q2_player_state_checkpoint(reader.field("state"))?,
        rules: read_rules(reader.field("rules"))?,
        last_attack: reader.field("lastAttack").nullable(read_q2_attack_checkpoint)?,
        entity: Q2CharacterEntity {
            model: entity.field("model").string()?,
            model2: entity.field("model2").string()?,
            model3: entity.field("model3").string()?,
            model4: entity.field("model4").string()?,
            skin: entity.field("skin").number()?,
            frame: entity.field("frame").number()?,
            old_frame: entity.field("oldFrame").number()?,
            scale: entity.field("scale").number()?,
            effects: entity.field("effects").number()?,
            render_flags: entity.field("renderFlags").number()?,
            flags: entity.field("flags").number()?,
            server_flags: entity.field("serverFlags").number()?,
            view_height: entity.field("viewHeight").number()?,
            max_health: entity.field("maxHealth").number()?,
            sound: entity.field("sound").string()?,
            visible: entity.field("visible").boolean()?,
        },
    })
}

/// Write a Q2 character checkpoint.
#[must_use]
pub fn write_q2_character_checkpoint(checkpoint: &Q2CharacterCheckpoint) -> SaveJson {
    obj(vec![
        ("version", int(1)),
        ("painIndex", num(checkpoint.pain_index)),
        ("deathIndex", num(checkpoint.death_index)),
        ("state", write_q2_player_state_checkpoint(&checkpoint.state)),
        ("rules", write_rules(&checkpoint.rules)),
        (
            "lastAttack",
            checkpoint
                .last_attack
                .as_ref()
                .map_or(SaveJson::Null, write_q2_attack_checkpoint),
        ),
        (
            "entity",
            obj(vec![
                ("model", str(&checkpoint.entity.model)),
                ("model2", str(&checkpoint.entity.model2)),
                ("model3", str(&checkpoint.entity.model3)),
                ("model4", str(&checkpoint.entity.model4)),
                ("skin", num(checkpoint.entity.skin)),
                ("frame", num(checkpoint.entity.frame)),
                ("oldFrame", num(checkpoint.entity.old_frame)),
                ("scale", num(checkpoint.entity.scale)),
                ("effects", num(checkpoint.entity.effects)),
                ("renderFlags", num(checkpoint.entity.render_flags)),
                ("flags", num(checkpoint.entity.flags)),
                ("serverFlags", num(checkpoint.entity.server_flags)),
                ("viewHeight", num(checkpoint.entity.view_height)),
                ("maxHealth", num(checkpoint.entity.max_health)),
                ("sound", str(&checkpoint.entity.sound)),
                ("visible", boolean(checkpoint.entity.visible)),
            ]),
        ),
    ])
}

/// Encode a Q2 character checkpoint.
#[must_use]
pub fn encode_q2_character_checkpoint(checkpoint: &Q2CharacterCheckpoint) -> Vec<u8> {
    encode_checkpoint_value(&write_q2_character_checkpoint(checkpoint))
}

/// Decode a Q2 character checkpoint.
pub fn decode_q2_character_checkpoint(bytes: &[u8]) -> Result<Q2CharacterCheckpoint, PersistenceError> {
    let payload = decode_checkpoint_value(bytes)?;
    read_q2_character_checkpoint(SaveReader::at(&payload, "q2-character"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_world::combat::CombatState;

    fn zero() -> Vec3 {
        Vec3 { x: 0.0, y: 0.0, z: 0.0 }
    }

    fn rules() -> Q2PlayerRules {
        Q2PlayerRules {
            password: String::new(),
            spectator_password: String::new(),
            max_spectators: 4.0,
            cheats: false,
            time_limit_minutes: 0.0,
            frag_limit: 0.0,
            map_list: Vec::new(),
            map_list_shuffle: false,
            next_map: String::new(),
            spawn_point: String::new(),
            flood_messages: 4.0,
            flood_seconds: 4.0,
            flood_wait_seconds: 10.0,
            roll_speed: 200.0,
            roll_angle: 2.0,
            run_pitch: 0.002,
            run_roll: 0.005,
            bob_up: 0.005,
            bob_pitch: 0.002,
            bob_roll: 0.002,
            gun_offset: zero(),
        }
    }

    #[allow(clippy::too_many_lines)]
    fn state() -> Q2PlayerStateCheckpoint {
        Q2PlayerStateCheckpoint {
            slot: 0,
            entered_at: 1.0,
            use_q2_weapons: true,
            use_q2_inventory: true,
            spawn_inventory: Vec::new(),
            userinfo: "\\name\\player".to_string(),
            name: "player".to_string(),
            skin: "male/grunt".to_string(),
            gender: "male".to_string(),
            fov: 90.0,
            hand: "right".to_string(),
            spectator: false,
            requested_spectator: false,
            connected: true,
            dead: false,
            gibbed: false,
            noclip: false,
            god: false,
            notarget: false,
            score: 5.0,
            ping: 12.0,
            respawn_time: 0.0,
            air_finished: 0.0,
            next_drown_time: 0.0,
            drown_damage: 0.0,
            old_water_level: 0.0,
            breather_sound: 0.0,
            pain_debounce: 0.0,
            damage_blood: 0.0,
            damage_armor: 0.0,
            damage_power_armor: 0.0,
            damage_knockback: 0.0,
            damage_from: zero(),
            damage_blend: zero(),
            damage_alpha: 0.0,
            bonus_alpha: 0.0,
            damage_pitch: 0.0,
            damage_roll: 0.0,
            damage_time: 0.0,
            power_armor_time: 0.0,
            fall_time: 0.0,
            fall_value: 0.0,
            landmark_free_fall: false,
            landmark_noise_time: 0.0,
            old_velocity: zero(),
            old_view_angles: zero(),
            killer_yaw: 0.0,
            buttons: 0.0,
            latched_buttons: 0.0,
            weapon_thunk: false,
            bob_time: 0.0,
            bob_move: 0.0,
            event: String::new(),
            animation_priority: 0.0,
            animation_end: 0.0,
            animation_duck: false,
            animation_run: false,
            loop_sound: String::new(),
            selected_item: None,
            show_scores: false,
            show_inventory: false,
            show_help: false,
            chase_target: None,
            coop_respawn: Some(Q2PlayerCarry {
                health: 100.0,
                maximum_health: 100.0,
                armor: CombatState::default().armor,
                inventory: Vec::new(),
                weapon: Some("blaster".to_string()),
                selected_item: None,
                score: 0.0,
                flags: 0.0,
                power_cubes: 0.0,
            }),
            flood_times: vec![0.0, 0.0],
            flood_lock_until: 0.0,
        }
    }

    #[test]
    fn players_round_trip() {
        let checkpoint = Q2PlayersCheckpoint {
            corpse_index: 1,
            death_animation: 2.0,
            pain_animation: 1.0,
            rules: rules(),
            intermission: Q2PlayerIntermission::Playing,
            players: vec![(SavedActorId { slot: 0, generation: 0 }, state())],
        };
        let bytes = encode_q2_players_checkpoint(&checkpoint);
        assert_eq!(decode_q2_players_checkpoint(&bytes).unwrap(), checkpoint);
    }

    #[test]
    fn character_round_trip() {
        let checkpoint = Q2CharacterCheckpoint {
            pain_index: 1.0,
            death_index: 2.0,
            state: state(),
            rules: rules(),
            last_attack: None,
            entity: Q2CharacterEntity {
                model: "players/male/tris.md2".to_string(),
                model2: String::new(),
                model3: String::new(),
                model4: String::new(),
                skin: 0.0,
                frame: 0.0,
                old_frame: 0.0,
                scale: 1.0,
                effects: 0.0,
                render_flags: 0.0,
                flags: 0.0,
                server_flags: 0.0,
                view_height: 22.0,
                max_health: 100.0,
                sound: String::new(),
                visible: true,
            },
        };
        let bytes = encode_q2_character_checkpoint(&checkpoint);
        assert_eq!(decode_q2_character_checkpoint(&bytes).unwrap(), checkpoint);
    }
}
