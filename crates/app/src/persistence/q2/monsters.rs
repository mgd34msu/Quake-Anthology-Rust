//! Quake II monster checkpoint ported from `src/persistence/q2-monsters.ts`.

use qa_core::identity::SavedActorId;
use qa_core::math::Vec3;
use qa_world::save::records::{read_saved_actor, write_saved_actor};
use qa_world::save::shared::{read_vector, write_vector};
use qa_world::save::value::{
    arr, boolean, decode_checkpoint_value, encode_checkpoint_value, int, num, obj, str, SaveJson, SaveReader,
};

use super::super::PersistenceError;
use super::foundation::{read_q2_attack_checkpoint, write_q2_attack_checkpoint, Q2AttackCheckpoint};

/// Alternate fly state.
#[derive(Debug, Clone, PartialEq)]
pub struct Q2AlternateFlyState {
    /// Alternate fly mode.
    pub alternate_fly: bool,
    /// Minimum fly distance.
    pub fly_min_distance: f64,
    /// Maximum fly distance.
    pub fly_max_distance: f64,
    /// Fly acceleration.
    pub fly_acceleration: f64,
    /// Fly speed.
    pub fly_speed: f64,
    /// Ideal fly position.
    pub fly_ideal_position: Vec3,
    /// Fly position time.
    pub fly_position_time: f64,
    /// Buzzard behavior.
    pub fly_buzzard: bool,
    /// Fly above target.
    pub fly_above: bool,
    /// Pinned in place.
    pub fly_pinned: bool,
    /// Thruster flight.
    pub fly_thrusters: bool,
    /// Recovery time.
    pub fly_recovery_time: f64,
    /// Recovery direction.
    pub fly_recovery_direction: Vec3,
    /// Hint path mode.
    pub hint_path: bool,
    /// Pathing state.
    pub pathing: Option<Q2FlyPathing>,
}

/// Fly pathing state.
#[derive(Debug, Clone, PartialEq)]
pub struct Q2FlyPathing {
    /// First move point.
    pub first_move_point: Vec3,
    /// Second move point.
    pub second_move_point: Vec3,
    /// Traversal pending.
    pub traversal_pending: bool,
}

fn read_alternate_fly(reader: SaveReader) -> Result<Q2AlternateFlyState, PersistenceError> {
    Ok(Q2AlternateFlyState {
        alternate_fly: reader.field("alternateFly").boolean()?,
        fly_min_distance: reader.field("flyMinDistance").number()?,
        fly_max_distance: reader.field("flyMaxDistance").number()?,
        fly_acceleration: reader.field("flyAcceleration").number()?,
        fly_speed: reader.field("flySpeed").number()?,
        fly_ideal_position: read_vector(reader.field("flyIdealPosition"))?,
        fly_position_time: reader.field("flyPositionTime").number()?,
        fly_buzzard: reader.field("flyBuzzard").boolean()?,
        fly_above: reader.field("flyAbove").boolean()?,
        fly_pinned: reader.field("flyPinned").boolean()?,
        fly_thrusters: reader.field("flyThrusters").boolean()?,
        fly_recovery_time: reader.field("flyRecoveryTime").number()?,
        fly_recovery_direction: read_vector(reader.field("flyRecoveryDirection"))?,
        hint_path: reader.field("hintPath").boolean()?,
        pathing: reader
            .field("pathing")
            .nullable(|value| -> Result<Q2FlyPathing, PersistenceError> {
                Ok(Q2FlyPathing {
                    first_move_point: read_vector(value.field("firstMovePoint"))?,
                    second_move_point: read_vector(value.field("secondMovePoint"))?,
                    traversal_pending: value.field("traversalPending").boolean()?,
                })
            })?,
    })
}

fn write_alternate_fly(state: &Q2AlternateFlyState) -> Vec<(&'static str, SaveJson)> {
    vec![
        ("alternateFly", boolean(state.alternate_fly)),
        ("flyMinDistance", num(state.fly_min_distance)),
        ("flyMaxDistance", num(state.fly_max_distance)),
        ("flyAcceleration", num(state.fly_acceleration)),
        ("flySpeed", num(state.fly_speed)),
        ("flyIdealPosition", write_vector(state.fly_ideal_position)),
        ("flyPositionTime", num(state.fly_position_time)),
        ("flyBuzzard", boolean(state.fly_buzzard)),
        ("flyAbove", boolean(state.fly_above)),
        ("flyPinned", boolean(state.fly_pinned)),
        ("flyThrusters", boolean(state.fly_thrusters)),
        ("flyRecoveryTime", num(state.fly_recovery_time)),
        ("flyRecoveryDirection", write_vector(state.fly_recovery_direction)),
        ("hintPath", boolean(state.hint_path)),
        (
            "pathing",
            state.pathing.as_ref().map_or(SaveJson::Null, |pathing| {
                obj(vec![
                    ("firstMovePoint", write_vector(pathing.first_move_point)),
                    ("secondMovePoint", write_vector(pathing.second_move_point)),
                    ("traversalPending", boolean(pathing.traversal_pending)),
                ])
            }),
        ),
    ]
}

/// Q2 monster state checkpoint.
#[derive(Debug, Clone, PartialEq)]
pub struct Q2MonsterStateCheckpoint {
    /// Initial power-armor type.
    pub initial_power_armor_type: String,
    /// Maximum power-armor power.
    pub max_power_armor_power: f64,
    /// Base health.
    pub base_health: f64,
    /// Health scaling.
    pub health_scaling: i64,
    /// Fly state.
    pub fly: Q2AlternateFlyState,
    /// Kind.
    pub kind: String,
    /// Weapon.
    pub weapon: String,
    /// Locomotion.
    pub locomotion: String,
    /// Melee capability.
    pub has_melee: bool,
    /// Ranged attack capability.
    pub has_ranged_attack: bool,
    /// Idle capability.
    pub has_idle: bool,
    /// Search capability.
    pub has_search: bool,
    /// Blind fire capability.
    pub blind_fire: bool,
    /// Good-guy flag.
    pub good_guy: bool,
    /// Target anger flag.
    pub target_anger: bool,
    /// Ignore shots flag.
    pub ignore_shots: bool,
    /// Do-not-count flag.
    pub do_not_count: bool,
    /// Spawner.
    pub spawned_by: String,
    /// Commander.
    pub commander: Option<SavedActorId>,
    /// Monster slots.
    pub monster_slots: f64,
    /// Monster used.
    pub monster_used: f64,
    /// Brutal flag.
    pub brutal: bool,
    /// Medic flag.
    pub medic: bool,
    /// Resurrecting flag.
    pub resurrecting: bool,
    /// Current move.
    pub move_name: String,
    /// Next move.
    pub next_move: Option<String>,
    /// Next frame.
    pub next_frame: f64,
    /// Next move time.
    pub next_move_time: f64,
    /// Scale.
    pub scale: f64,
    /// Gib health.
    pub gib_health: f64,
    /// Can take damage.
    pub can_take_damage: bool,
    /// Dead flag.
    pub dead: bool,
    /// Corpse flag.
    pub corpse: bool,
    /// Gibbed flag.
    pub gibbed: bool,
    /// Stand ground flag.
    pub stand_ground: bool,
    /// Temporary stand ground.
    pub temporary_stand_ground: bool,
    /// Hold frame flag.
    pub hold_frame: bool,
    /// Ducked flag.
    pub ducked: bool,
    /// Dodging flag.
    pub dodging: bool,
    /// Charging flag.
    pub charging: bool,
    /// Manual steering flag.
    pub manual_steering: bool,
    /// Combat point flag.
    pub combat_point: bool,
    /// Attack state.
    pub attack_state: String,
    /// Lefty flag.
    pub lefty: bool,
    /// Ideal yaw.
    pub ideal_yaw: f64,
    /// Yaw speed.
    pub yaw_speed: f64,
    /// Pause time.
    pub pause_time: f64,
    /// Idle time.
    pub idle_time: f64,
    /// Pain time.
    pub pain_time: f64,
    /// Fire wait.
    pub fire_wait: f64,
    /// Duck wait.
    pub duck_wait: f64,
    /// Next duck time.
    pub next_duck_time: f64,
    /// Dodge time.
    pub dodge_time: f64,
    /// Attack finished.
    pub attack_finished: f64,
    /// Check attack time.
    pub check_attack_time: f64,
    /// Strafe time.
    pub strafe_time: f64,
    /// Had visibility flag.
    pub had_visibility: bool,
    /// Close sight tripped flag.
    pub close_sight_tripped: bool,
    /// Melee time.
    pub melee_time: f64,
    /// Search time.
    pub search_time: f64,
    /// Trail time.
    pub trail_time: f64,
    /// Show hostile time.
    pub show_hostile: f64,
    /// Last sighting.
    pub last_sighting: Vec3,
    /// Saved goal.
    pub saved_goal: Option<Vec3>,
    /// Lost sight flag.
    pub lost_sight: bool,
    /// Pursue next flag.
    pub pursue_next: bool,
    /// Pursue temporary flag.
    pub pursue_temporary: bool,
    /// Pursuit last seen flag.
    pub pursuit_last_seen: bool,
    /// Blind-fire target.
    pub blind_fire_target: Vec3,
    /// Blind-fire delay.
    pub blind_fire_delay: f64,
    /// Sound target.
    pub sound_target: Option<Q2SoundTarget>,
    /// Old enemy.
    pub old_enemy: Option<SavedActorId>,
    /// Move target.
    pub move_target: Option<SavedActorId>,
    /// Combat target.
    pub combat_target: String,
    /// Cocked flag.
    pub cocked: bool,
    /// Force refire flag.
    pub force_refire: bool,
    /// Normal height.
    pub normal_height: f64,
    /// Air finished.
    pub air_finished: f64,
    /// Environmental damage time.
    pub environmental_damage_time: f64,
    /// Water level.
    pub water_level: i64,
    /// Water type.
    pub water_type: f64,
    /// Last link count.
    pub last_link_count: f64,
    /// Jump time.
    pub jump_time: f64,
    /// Flies time.
    pub flies_time: Option<f64>,
}

/// Monster sound target.
#[derive(Debug, Clone, PartialEq)]
pub struct Q2SoundTarget {
    /// Actor.
    pub actor: SavedActorId,
    /// Owner.
    pub owner: SavedActorId,
    /// Origin.
    pub origin: Vec3,
    /// Time.
    pub time: f64,
}

fn read_state(reader: SaveReader) -> Result<Q2MonsterStateCheckpoint, PersistenceError> {
    let number = |name: &str| reader.field(name).number().map_err(PersistenceError::from);
    let flag = |name: &str| reader.field(name).boolean().map_err(PersistenceError::from);
    Ok(Q2MonsterStateCheckpoint {
        initial_power_armor_type: reader
            .field("initialPowerArmorType")
            .choice_str(&["none", "screen", "shield"])?,
        max_power_armor_power: reader.field("maxPowerArmorPower").finite()?,
        base_health: reader.field("baseHealth").finite()?,
        health_scaling: reader.field("healthScaling").integer(1)?,
        fly: read_alternate_fly(reader.clone())?,
        kind: reader.field("kind").string()?,
        weapon: reader
            .field("weapon")
            .choice_str(&["blaster", "shotgun", "machinegun"])?,
        locomotion: reader
            .field("locomotion")
            .choice_str(&["walk", "fly", "swim", "stationary"])?,
        has_melee: flag("hasMelee")?,
        has_ranged_attack: flag("hasRangedAttack")?,
        has_idle: flag("hasIdle")?,
        has_search: flag("hasSearch")?,
        blind_fire: flag("blindFire")?,
        good_guy: flag("goodGuy")?,
        target_anger: flag("targetAnger")?,
        ignore_shots: flag("ignoreShots")?,
        do_not_count: flag("doNotCount")?,
        spawned_by: reader
            .field("spawnedBy")
            .choice_str(&["none", "carrier", "medic", "widow"])?,
        commander: reader
            .field("commander")
            .nullable(|value| read_saved_actor(value).map_err(PersistenceError::from))?,
        monster_slots: number("monsterSlots")?,
        monster_used: number("monsterUsed")?,
        brutal: flag("brutal")?,
        medic: flag("medic")?,
        resurrecting: flag("resurrecting")?,
        move_name: reader.field("move").string()?,
        next_move: reader
            .field("nextMove")
            .nullable(|value| value.string().map_err(PersistenceError::from))?,
        next_frame: number("nextFrame")?,
        next_move_time: number("nextMoveTime")?,
        scale: number("scale")?,
        gib_health: number("gibHealth")?,
        can_take_damage: flag("canTakeDamage")?,
        dead: flag("dead")?,
        corpse: flag("corpse")?,
        gibbed: flag("gibbed")?,
        stand_ground: flag("standGround")?,
        temporary_stand_ground: flag("temporaryStandGround")?,
        hold_frame: flag("holdFrame")?,
        ducked: flag("ducked")?,
        dodging: flag("dodging")?,
        charging: flag("charging")?,
        manual_steering: flag("manualSteering")?,
        combat_point: flag("combatPoint")?,
        attack_state: reader
            .field("attackState")
            .choice_str(&["straight", "sliding", "melee", "missile", "blind"])?,
        lefty: flag("lefty")?,
        ideal_yaw: number("idealYaw")?,
        yaw_speed: number("yawSpeed")?,
        pause_time: number("pauseTime")?,
        idle_time: number("idleTime")?,
        pain_time: number("painTime")?,
        fire_wait: number("fireWait")?,
        duck_wait: number("duckWait")?,
        next_duck_time: number("nextDuckTime")?,
        dodge_time: number("dodgeTime")?,
        attack_finished: number("attackFinished")?,
        check_attack_time: number("checkAttackTime")?,
        strafe_time: number("strafeTime")?,
        had_visibility: flag("hadVisibility")?,
        close_sight_tripped: flag("closeSightTripped")?,
        melee_time: number("meleeTime")?,
        search_time: number("searchTime")?,
        trail_time: number("trailTime")?,
        show_hostile: number("showHostile")?,
        last_sighting: read_vector(reader.field("lastSighting"))?,
        saved_goal: reader
            .field("savedGoal")
            .nullable(|value| read_vector(value).map_err(PersistenceError::from))?,
        lost_sight: flag("lostSight")?,
        pursue_next: flag("pursueNext")?,
        pursue_temporary: flag("pursueTemporary")?,
        pursuit_last_seen: flag("pursuitLastSeen")?,
        blind_fire_target: read_vector(reader.field("blindFireTarget"))?,
        blind_fire_delay: number("blindFireDelay")?,
        sound_target: reader
            .field("soundTarget")
            .nullable(|value| -> Result<Q2SoundTarget, PersistenceError> {
                Ok(Q2SoundTarget {
                    actor: read_saved_actor(value.field("actor"))?,
                    owner: read_saved_actor(value.field("owner"))?,
                    origin: read_vector(value.field("origin"))?,
                    time: value.field("time").number()?,
                })
            })?,
        old_enemy: reader
            .field("oldEnemy")
            .nullable(|value| read_saved_actor(value).map_err(PersistenceError::from))?,
        move_target: reader
            .field("moveTarget")
            .nullable(|value| read_saved_actor(value).map_err(PersistenceError::from))?,
        combat_target: reader.field("combatTarget").string()?,
        cocked: flag("cocked")?,
        force_refire: flag("forceRefire")?,
        normal_height: number("normalHeight")?,
        air_finished: number("airFinished")?,
        environmental_damage_time: number("environmentalDamageTime")?,
        water_level: reader.field("waterLevel").choice_i64(&[0, 1, 2, 3])?,
        water_type: number("waterType")?,
        last_link_count: number("lastLinkCount")?,
        jump_time: number("jumpTime")?,
        flies_time: reader
            .field("fliesTime")
            .nullable(|value| value.number().map_err(PersistenceError::from))?,
    })
}

#[allow(clippy::too_many_lines)]
fn write_state(state: &Q2MonsterStateCheckpoint) -> SaveJson {
    let mut members = vec![
        ("initialPowerArmorType", str(&state.initial_power_armor_type)),
        ("maxPowerArmorPower", num(state.max_power_armor_power)),
        ("baseHealth", num(state.base_health)),
        ("healthScaling", int(state.health_scaling)),
    ];
    members.extend(write_alternate_fly(&state.fly));
    members.extend(vec![
        ("kind", str(&state.kind)),
        ("weapon", str(&state.weapon)),
        ("locomotion", str(&state.locomotion)),
        ("hasMelee", boolean(state.has_melee)),
        ("hasRangedAttack", boolean(state.has_ranged_attack)),
        ("hasIdle", boolean(state.has_idle)),
        ("hasSearch", boolean(state.has_search)),
        ("blindFire", boolean(state.blind_fire)),
        ("goodGuy", boolean(state.good_guy)),
        ("targetAnger", boolean(state.target_anger)),
        ("ignoreShots", boolean(state.ignore_shots)),
        ("doNotCount", boolean(state.do_not_count)),
        ("spawnedBy", str(&state.spawned_by)),
        ("commander", state.commander.map_or(SaveJson::Null, write_saved_actor)),
        ("monsterSlots", num(state.monster_slots)),
        ("monsterUsed", num(state.monster_used)),
        ("brutal", boolean(state.brutal)),
        ("medic", boolean(state.medic)),
        ("resurrecting", boolean(state.resurrecting)),
        ("move", str(&state.move_name)),
        (
            "nextMove",
            state.next_move.as_ref().map_or(SaveJson::Null, |value| str(value)),
        ),
        ("nextFrame", num(state.next_frame)),
        ("nextMoveTime", num(state.next_move_time)),
        ("scale", num(state.scale)),
        ("gibHealth", num(state.gib_health)),
        ("canTakeDamage", boolean(state.can_take_damage)),
        ("dead", boolean(state.dead)),
        ("corpse", boolean(state.corpse)),
        ("gibbed", boolean(state.gibbed)),
        ("standGround", boolean(state.stand_ground)),
        ("temporaryStandGround", boolean(state.temporary_stand_ground)),
        ("holdFrame", boolean(state.hold_frame)),
        ("ducked", boolean(state.ducked)),
        ("dodging", boolean(state.dodging)),
        ("charging", boolean(state.charging)),
        ("manualSteering", boolean(state.manual_steering)),
        ("combatPoint", boolean(state.combat_point)),
        ("attackState", str(&state.attack_state)),
        ("lefty", boolean(state.lefty)),
        ("idealYaw", num(state.ideal_yaw)),
        ("yawSpeed", num(state.yaw_speed)),
        ("pauseTime", num(state.pause_time)),
        ("idleTime", num(state.idle_time)),
        ("painTime", num(state.pain_time)),
        ("fireWait", num(state.fire_wait)),
        ("duckWait", num(state.duck_wait)),
        ("nextDuckTime", num(state.next_duck_time)),
        ("dodgeTime", num(state.dodge_time)),
        ("attackFinished", num(state.attack_finished)),
        ("checkAttackTime", num(state.check_attack_time)),
        ("strafeTime", num(state.strafe_time)),
        ("hadVisibility", boolean(state.had_visibility)),
        ("closeSightTripped", boolean(state.close_sight_tripped)),
        ("meleeTime", num(state.melee_time)),
        ("searchTime", num(state.search_time)),
        ("trailTime", num(state.trail_time)),
        ("showHostile", num(state.show_hostile)),
        ("lastSighting", write_vector(state.last_sighting)),
        ("savedGoal", state.saved_goal.map_or(SaveJson::Null, write_vector)),
        ("lostSight", boolean(state.lost_sight)),
        ("pursueNext", boolean(state.pursue_next)),
        ("pursueTemporary", boolean(state.pursue_temporary)),
        ("pursuitLastSeen", boolean(state.pursuit_last_seen)),
        ("blindFireTarget", write_vector(state.blind_fire_target)),
        ("blindFireDelay", num(state.blind_fire_delay)),
        (
            "soundTarget",
            state.sound_target.as_ref().map_or(SaveJson::Null, |target| {
                obj(vec![
                    ("actor", write_saved_actor(target.actor)),
                    ("owner", write_saved_actor(target.owner)),
                    ("origin", write_vector(target.origin)),
                    ("time", num(target.time)),
                ])
            }),
        ),
        ("oldEnemy", state.old_enemy.map_or(SaveJson::Null, write_saved_actor)),
        (
            "moveTarget",
            state.move_target.map_or(SaveJson::Null, write_saved_actor),
        ),
        ("combatTarget", str(&state.combat_target)),
        ("cocked", boolean(state.cocked)),
        ("forceRefire", boolean(state.force_refire)),
        ("normalHeight", num(state.normal_height)),
        ("airFinished", num(state.air_finished)),
        ("environmentalDamageTime", num(state.environmental_damage_time)),
        ("waterLevel", int(state.water_level)),
        ("waterType", num(state.water_type)),
        ("lastLinkCount", num(state.last_link_count)),
        ("jumpTime", num(state.jump_time)),
        ("fliesTime", state.flies_time.map_or(SaveJson::Null, num)),
    ]);
    obj(members)
}

/// Pending monster damage.
#[derive(Debug, Clone, PartialEq)]
pub struct Q2PendingDamage {
    /// Damage.
    pub damage: f64,
    /// Kick.
    pub kick: f64,
    /// Point.
    pub point: Vec3,
    /// Attacker.
    pub attacker: Option<SavedActorId>,
    /// Inflictor.
    pub inflictor: Option<SavedActorId>,
    /// Attack.
    pub attack: Option<Q2AttackCheckpoint>,
}

/// Saved monster actor.
#[derive(Debug, Clone, PartialEq)]
pub struct Q2MonsterActor {
    /// Actor.
    pub actor: SavedActorId,
    /// Definition.
    pub definition: String,
    /// State.
    pub state: Q2MonsterStateCheckpoint,
    /// Pending damage.
    pub pending_damage: Option<Q2PendingDamage>,
}

/// Sighting record.
#[derive(Debug, Clone, PartialEq)]
pub struct Q2Sighting {
    /// Actor.
    pub actor: SavedActorId,
    /// Time.
    pub time: f64,
}

/// Noise record.
#[derive(Debug, Clone, PartialEq)]
pub struct Q2NoiseSighting {
    /// Actor.
    pub actor: SavedActorId,
    /// Time.
    pub time: f64,
    /// Owner.
    pub owner: SavedActorId,
    /// Origin.
    pub origin: Vec3,
}

/// One monster trail point: origin, time, yaw.
pub type Q2TrailPoint = (Vec3, f64, f64);
/// One monster trail: actor plus points.
pub type Q2MonsterTrail = (SavedActorId, Vec<Q2TrailPoint>);

/// Monster perception checkpoint.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Q2MonsterPerception {
    /// Sight client.
    pub sight_client: Option<SavedActorId>,
    /// Sight.
    pub sight: Option<Q2Sighting>,
    /// Alerted actors.
    pub alerted: Vec<(SavedActorId, Q2Sighting)>,
    /// Primary noise.
    pub primary: Option<Q2NoiseSighting>,
    /// Secondary noise.
    pub secondary: Option<Q2NoiseSighting>,
    /// Noises.
    pub noises: Vec<(SavedActorId, SavedActorId, SavedActorId)>,
    /// Trails.
    pub trails: Vec<Q2MonsterTrail>,
    /// Player origins.
    pub player_origins: Vec<(SavedActorId, Vec3)>,
    /// Hostile sightings.
    pub hostile: Vec<Q2Sighting>,
    /// Last frame.
    pub last_frame: Option<f64>,
}

fn read_sighting(reader: SaveReader) -> Result<Q2Sighting, PersistenceError> {
    Ok(Q2Sighting {
        actor: read_saved_actor(reader.field("actor"))?,
        time: reader.field("time").number()?,
    })
}

fn write_sighting(sighting: &Q2Sighting) -> SaveJson {
    obj(vec![
        ("actor", write_saved_actor(sighting.actor)),
        ("time", num(sighting.time)),
    ])
}

fn read_noise(reader: SaveReader) -> Result<Q2NoiseSighting, PersistenceError> {
    Ok(Q2NoiseSighting {
        actor: read_saved_actor(reader.field("actor"))?,
        time: reader.field("time").number()?,
        owner: read_saved_actor(reader.field("owner"))?,
        origin: read_vector(reader.field("origin"))?,
    })
}

fn write_noise(noise: &Q2NoiseSighting) -> SaveJson {
    obj(vec![
        ("actor", write_saved_actor(noise.actor)),
        ("time", num(noise.time)),
        ("owner", write_saved_actor(noise.owner)),
        ("origin", write_vector(noise.origin)),
    ])
}

fn read_perception(reader: SaveReader) -> Result<Q2MonsterPerception, PersistenceError> {
    Ok(Q2MonsterPerception {
        sight_client: reader
            .field("sightClient")
            .nullable(|value| read_saved_actor(value).map_err(PersistenceError::from))?,
        sight: reader.field("sight").nullable(read_sighting)?,
        alerted: reader
            .field("alerted")
            .list(|value| -> Result<(SavedActorId, Q2Sighting), PersistenceError> {
                Ok((
                    read_saved_actor(value.field("actor"))?,
                    read_sighting(value.field("sighting"))?,
                ))
            })?,
        primary: reader.field("primary").nullable(read_noise)?,
        secondary: reader.field("secondary").nullable(read_noise)?,
        noises: reader.field("noises").list(
            |value| -> Result<(SavedActorId, SavedActorId, SavedActorId), PersistenceError> {
                Ok((
                    read_saved_actor(value.field("actor"))?,
                    read_saved_actor(value.field("primary"))?,
                    read_saved_actor(value.field("secondary"))?,
                ))
            },
        )?,
        trails: reader
            .field("trails")
            .list(|value| -> Result<Q2MonsterTrail, PersistenceError> {
                Ok((
                    read_saved_actor(value.field("actor"))?,
                    value
                        .field("points")
                        .list(|point| -> Result<Q2TrailPoint, PersistenceError> {
                            Ok((
                                read_vector(point.field("origin"))?,
                                point.field("time").number()?,
                                point.field("yaw").number()?,
                            ))
                        })?,
                ))
            })?,
        player_origins: reader.field("playerOrigins").list(
            |value| -> Result<(SavedActorId, Vec3), PersistenceError> {
                Ok((
                    read_saved_actor(value.field("actor"))?,
                    read_vector(value.field("origin"))?,
                ))
            },
        )?,
        hostile: reader.field("hostile").list(read_sighting)?,
        last_frame: reader
            .field("lastFrame")
            .nullable(|value| value.number().map_err(PersistenceError::from))?,
    })
}

fn write_perception(perception: &Q2MonsterPerception) -> SaveJson {
    obj(vec![
        (
            "sightClient",
            perception.sight_client.map_or(SaveJson::Null, write_saved_actor),
        ),
        (
            "sight",
            perception.sight.as_ref().map_or(SaveJson::Null, write_sighting),
        ),
        (
            "alerted",
            arr(perception
                .alerted
                .iter()
                .map(|(actor, sighting)| {
                    obj(vec![
                        ("actor", write_saved_actor(*actor)),
                        ("sighting", write_sighting(sighting)),
                    ])
                })
                .collect()),
        ),
        (
            "primary",
            perception.primary.as_ref().map_or(SaveJson::Null, write_noise),
        ),
        (
            "secondary",
            perception.secondary.as_ref().map_or(SaveJson::Null, write_noise),
        ),
        (
            "noises",
            arr(perception
                .noises
                .iter()
                .map(|(actor, primary, secondary)| {
                    obj(vec![
                        ("actor", write_saved_actor(*actor)),
                        ("primary", write_saved_actor(*primary)),
                        ("secondary", write_saved_actor(*secondary)),
                    ])
                })
                .collect()),
        ),
        (
            "trails",
            arr(perception
                .trails
                .iter()
                .map(|(actor, points)| {
                    obj(vec![
                        ("actor", write_saved_actor(*actor)),
                        (
                            "points",
                            arr(points
                                .iter()
                                .map(|(origin, time, yaw)| {
                                    obj(vec![
                                        ("origin", write_vector(*origin)),
                                        ("time", num(*time)),
                                        ("yaw", num(*yaw)),
                                    ])
                                })
                                .collect()),
                        ),
                    ])
                })
                .collect()),
        ),
        (
            "playerOrigins",
            arr(perception
                .player_origins
                .iter()
                .map(|(actor, origin)| {
                    obj(vec![
                        ("actor", write_saved_actor(*actor)),
                        ("origin", write_vector(*origin)),
                    ])
                })
                .collect()),
        ),
        ("hostile", arr(perception.hostile.iter().map(write_sighting).collect())),
        ("lastFrame", perception.last_frame.map_or(SaveJson::Null, num)),
    ])
}

/// Q2 monsters checkpoint.
#[derive(Debug, Clone, PartialEq)]
pub struct Q2MonstersCheckpoint {
    /// Actors.
    pub actors: Vec<Q2MonsterActor>,
    /// Perception.
    pub perception: Q2MonsterPerception,
}

/// Read a Q2 monsters checkpoint.
pub fn read_q2_monsters_checkpoint(reader: SaveReader) -> Result<Q2MonstersCheckpoint, PersistenceError> {
    reader.field("version").literal_i64(1)?;
    Ok(Q2MonstersCheckpoint {
        actors: reader
            .field("actors")
            .list(|value| -> Result<Q2MonsterActor, PersistenceError> {
                Ok(Q2MonsterActor {
                    actor: read_saved_actor(value.field("actor"))?,
                    definition: value.field("definition").string()?,
                    state: read_state(value.field("state"))?,
                    pending_damage: value.field("pendingDamage").nullable(
                        |pending| -> Result<Q2PendingDamage, PersistenceError> {
                            let reaction = pending.field("reaction");
                            Ok(Q2PendingDamage {
                                damage: reaction.field("damage").number()?,
                                kick: reaction.field("kick").number()?,
                                point: read_vector(reaction.field("point"))?,
                                attacker: reaction
                                    .field("attacker")
                                    .nullable(|item| read_saved_actor(item).map_err(PersistenceError::from))?,
                                inflictor: reaction
                                    .field("inflictor")
                                    .nullable(|item| read_saved_actor(item).map_err(PersistenceError::from))?,
                                attack: pending.field("attack").nullable(read_q2_attack_checkpoint)?,
                            })
                        },
                    )?,
                })
            })?,
        perception: read_perception(reader.field("perception"))?,
    })
}

/// Write a Q2 monsters checkpoint.
#[must_use]
pub fn write_q2_monsters_checkpoint(checkpoint: &Q2MonstersCheckpoint) -> SaveJson {
    obj(vec![
        ("version", int(1)),
        (
            "actors",
            arr(checkpoint
                .actors
                .iter()
                .map(|actor| {
                    obj(vec![
                        ("actor", write_saved_actor(actor.actor)),
                        ("definition", str(&actor.definition)),
                        ("state", write_state(&actor.state)),
                        (
                            "pendingDamage",
                            actor.pending_damage.as_ref().map_or(SaveJson::Null, |pending| {
                                obj(vec![
                                    (
                                        "reaction",
                                        obj(vec![
                                            ("damage", num(pending.damage)),
                                            ("kick", num(pending.kick)),
                                            ("point", write_vector(pending.point)),
                                            ("attacker", pending.attacker.map_or(SaveJson::Null, write_saved_actor)),
                                            ("inflictor", pending.inflictor.map_or(SaveJson::Null, write_saved_actor)),
                                        ]),
                                    ),
                                    (
                                        "attack",
                                        pending
                                            .attack
                                            .as_ref()
                                            .map_or(SaveJson::Null, write_q2_attack_checkpoint),
                                    ),
                                ])
                            }),
                        ),
                    ])
                })
                .collect()),
        ),
        ("perception", write_perception(&checkpoint.perception)),
    ])
}

/// Encode a Q2 monsters checkpoint.
#[must_use]
pub fn encode_q2_monsters_checkpoint(checkpoint: &Q2MonstersCheckpoint) -> Vec<u8> {
    encode_checkpoint_value(&write_q2_monsters_checkpoint(checkpoint))
}

/// Decode a Q2 monsters checkpoint.
pub fn decode_q2_monsters_checkpoint(bytes: &[u8]) -> Result<Q2MonstersCheckpoint, PersistenceError> {
    let payload = decode_checkpoint_value(bytes)?;
    read_q2_monsters_checkpoint(SaveReader::at(&payload, "q2-monsters"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn zero() -> Vec3 {
        Vec3 { x: 0.0, y: 0.0, z: 0.0 }
    }

    fn fly() -> Q2AlternateFlyState {
        Q2AlternateFlyState {
            alternate_fly: false,
            fly_min_distance: 0.0,
            fly_max_distance: 0.0,
            fly_acceleration: 0.0,
            fly_speed: 0.0,
            fly_ideal_position: zero(),
            fly_position_time: 0.0,
            fly_buzzard: false,
            fly_above: false,
            fly_pinned: false,
            fly_thrusters: false,
            fly_recovery_time: 0.0,
            fly_recovery_direction: zero(),
            hint_path: false,
            pathing: None,
        }
    }

    #[allow(clippy::too_many_lines)]
    fn state() -> Q2MonsterStateCheckpoint {
        Q2MonsterStateCheckpoint {
            initial_power_armor_type: "none".to_string(),
            max_power_armor_power: 0.0,
            base_health: 30.0,
            health_scaling: 1,
            fly: fly(),
            kind: "soldier".to_string(),
            weapon: "blaster".to_string(),
            locomotion: "walk".to_string(),
            has_melee: true,
            has_ranged_attack: true,
            has_idle: true,
            has_search: true,
            blind_fire: false,
            good_guy: false,
            target_anger: false,
            ignore_shots: false,
            do_not_count: false,
            spawned_by: "none".to_string(),
            commander: None,
            monster_slots: 0.0,
            monster_used: 0.0,
            brutal: false,
            medic: false,
            resurrecting: false,
            move_name: "stand".to_string(),
            next_move: None,
            next_frame: 0.0,
            next_move_time: 0.0,
            scale: 1.0,
            gib_health: -30.0,
            can_take_damage: true,
            dead: false,
            corpse: false,
            gibbed: false,
            stand_ground: false,
            temporary_stand_ground: false,
            hold_frame: false,
            ducked: false,
            dodging: false,
            charging: false,
            manual_steering: false,
            combat_point: false,
            attack_state: "straight".to_string(),
            lefty: false,
            ideal_yaw: 0.0,
            yaw_speed: 0.0,
            pause_time: 0.0,
            idle_time: 0.0,
            pain_time: 0.0,
            fire_wait: 0.0,
            duck_wait: 0.0,
            next_duck_time: 0.0,
            dodge_time: 0.0,
            attack_finished: 0.0,
            check_attack_time: 0.0,
            strafe_time: 0.0,
            had_visibility: false,
            close_sight_tripped: false,
            melee_time: 0.0,
            search_time: 0.0,
            trail_time: 0.0,
            show_hostile: 0.0,
            last_sighting: zero(),
            saved_goal: None,
            lost_sight: false,
            pursue_next: false,
            pursue_temporary: false,
            pursuit_last_seen: false,
            blind_fire_target: zero(),
            blind_fire_delay: 0.0,
            sound_target: None,
            old_enemy: None,
            move_target: None,
            combat_target: String::new(),
            cocked: false,
            force_refire: false,
            normal_height: 0.0,
            air_finished: 0.0,
            environmental_damage_time: 0.0,
            water_level: 0,
            water_type: 0.0,
            last_link_count: 0.0,
            jump_time: 0.0,
            flies_time: None,
        }
    }

    #[test]
    fn monsters_round_trip() {
        let checkpoint = Q2MonstersCheckpoint {
            actors: vec![Q2MonsterActor {
                actor: SavedActorId { slot: 1, generation: 0 },
                definition: "q2:soldier".to_string(),
                state: state(),
                pending_damage: Some(Q2PendingDamage {
                    damage: 10.0,
                    kick: 20.0,
                    point: zero(),
                    attacker: None,
                    inflictor: None,
                    attack: None,
                }),
            }],
            perception: Q2MonsterPerception {
                hostile: vec![Q2Sighting {
                    actor: SavedActorId { slot: 2, generation: 0 },
                    time: 4.0,
                }],
                ..Q2MonsterPerception::default()
            },
        };
        let bytes = encode_q2_monsters_checkpoint(&checkpoint);
        assert_eq!(decode_q2_monsters_checkpoint(&bytes).unwrap(), checkpoint);
    }
}
