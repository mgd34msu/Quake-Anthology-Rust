//! Quake II foundation checkpoint ported from `src/persistence/q2-foundation.ts`.

use qa_core::identity::SavedActorId;
use qa_core::math::Vec3;
use qa_core::time::SourceTime;
use qa_world::save::records::{read_saved_actor, write_saved_actor};
use qa_world::save::shared::{read_time, read_vector, write_time, write_vector};
use qa_world::save::value::{
    arr, boolean, decode_checkpoint_value, encode_checkpoint_value, int, namespaced, num, obj, str, SaveJson,
    SaveReader,
};

use super::super::PersistenceError;

fn nonnegative(reader: SaveReader) -> Result<u64, PersistenceError> {
    let value = reader.integer(0)?;
    u64::try_from(value).map_err(|_| PersistenceError::from(reader.fail("expected an integer in range")))
}

/// Native damage cause.
#[derive(Debug, Clone, PartialEq)]
pub enum Q2NativeCause {
    /// Classic mod/game cause.
    Classic {
        /// Game.
        game: String,
        /// Value.
        value: i64,
    },
    /// Rerelease cause id.
    Rerelease {
        /// Cause id.
        id: u64,
        /// Friendly fire.
        friendly_fire: bool,
        /// No point loss.
        no_point_loss: bool,
    },
}

fn read_native_cause(reader: SaveReader) -> Result<Q2NativeCause, PersistenceError> {
    if reader.field("edition").choice_str(&["classic", "rerelease"])? == "classic" {
        Ok(Q2NativeCause::Classic {
            game: reader.field("game").choice_str(&["base", "xatrix", "rogue", "ctf"])?,
            value: reader.field("value").integer(i64::MIN)?,
        })
    } else {
        Ok(Q2NativeCause::Rerelease {
            id: nonnegative(reader.field("id"))?,
            friendly_fire: reader.field("friendlyFire").boolean()?,
            no_point_loss: reader.field("noPointLoss").boolean()?,
        })
    }
}

fn write_native_cause(cause: &Q2NativeCause) -> SaveJson {
    match cause {
        Q2NativeCause::Classic { game, value } => obj(vec![
            ("edition", str("classic")),
            ("game", str(game)),
            ("value", int(*value)),
        ]),
        #[allow(clippy::cast_possible_wrap)]
        Q2NativeCause::Rerelease {
            id,
            friendly_fire,
            no_point_loss,
        } => obj(vec![
            ("edition", str("rerelease")),
            ("id", int(*id as i64)),
            ("friendlyFire", boolean(*friendly_fire)),
            ("noPointLoss", boolean(*no_point_loss)),
        ]),
    }
}

/// Attack provenance cause.
#[derive(Debug, Clone, PartialEq)]
pub enum Q2AttackCause {
    /// Quake I death type.
    Q1 {
        /// Death type.
        death_type: String,
        /// Armor effect.
        armor_effect: Option<String>,
    },
    /// Quake II means/damage flags.
    Q2 {
        /// Means of death.
        means_of_death: i64,
        /// Damage flags.
        damage_flags: i64,
        /// Native cause.
        native: Option<Q2NativeCause>,
    },
    /// Quake III means/damage flags.
    Q3 {
        /// Means of death.
        means_of_death: i64,
        /// Damage flags.
        damage_flags: i64,
    },
    /// Environmental hazard.
    Environment {
        /// Hazard.
        hazard: String,
    },
}

fn read_cause(reader: SaveReader) -> Result<Q2AttackCause, PersistenceError> {
    match reader
        .field("kind")
        .choice_str(&["q1", "q2", "q3", "environment"])?
        .as_str()
    {
        "q1" => Ok(Q2AttackCause::Q1 {
            death_type: reader.field("deathType").string()?,
            armor_effect: if reader.field("armorEffect").is_missing() {
                None
            } else {
                Some(
                    reader
                        .field("armorEffect")
                        .choice_str(&["bypass", "half-effectiveness"])?,
                )
            },
        }),
        "q2" => Ok(Q2AttackCause::Q2 {
            means_of_death: reader.field("meansOfDeath").integer(i64::MIN)?,
            damage_flags: reader.field("damageFlags").integer(i64::MIN)?,
            native: if reader.field("native").is_missing() {
                None
            } else {
                Some(read_native_cause(reader.field("native"))?)
            },
        }),
        "q3" => Ok(Q2AttackCause::Q3 {
            means_of_death: reader.field("meansOfDeath").integer(i64::MIN)?,
            damage_flags: reader.field("damageFlags").integer(i64::MIN)?,
        }),
        _ => Ok(Q2AttackCause::Environment {
            hazard: reader
                .field("hazard")
                .choice_str(&["fall", "drown", "lava", "slime", "crush", "trigger"])?,
        }),
    }
}

fn write_cause(cause: &Q2AttackCause) -> SaveJson {
    match cause {
        Q2AttackCause::Q1 {
            death_type,
            armor_effect,
        } => {
            let mut members = vec![("kind", str("q1")), ("deathType", str(death_type))];
            if let Some(effect) = armor_effect {
                members.push(("armorEffect", str(effect)));
            }
            obj(members)
        }
        Q2AttackCause::Q2 {
            means_of_death,
            damage_flags,
            native,
        } => {
            let mut members = vec![
                ("kind", str("q2")),
                ("meansOfDeath", int(*means_of_death)),
                ("damageFlags", int(*damage_flags)),
            ];
            if let Some(native) = native {
                members.push(("native", write_native_cause(native)));
            }
            obj(members)
        }
        Q2AttackCause::Q3 {
            means_of_death,
            damage_flags,
        } => obj(vec![
            ("kind", str("q3")),
            ("meansOfDeath", int(*means_of_death)),
            ("damageFlags", int(*damage_flags)),
        ]),
        Q2AttackCause::Environment { hazard } => obj(vec![("kind", str("environment")), ("hazard", str(hazard))]),
    }
}

/// Saved Q2 attack checkpoint.
#[derive(Debug, Clone, PartialEq)]
pub struct Q2AttackCheckpoint {
    /// Sequence.
    pub sequence: u64,
    /// Time.
    pub time: SourceTime,
    /// Attacker.
    pub attacker: Option<SavedActorId>,
    /// Inflictor.
    pub inflictor: Option<SavedActorId>,
    /// Originating projectile.
    pub originating_projectile: Option<SavedActorId>,
    /// Damage powerup owner.
    pub damage_powerup_owner: Option<String>,
    /// Weapon.
    pub weapon: Option<String>,
    /// Weapon provider.
    pub weapon_provider: String,
    /// Combat provider.
    pub combat_provider: String,
    /// Inventory provider.
    pub inventory_provider: String,
    /// Movement provider.
    pub movement_provider: String,
    /// Cause.
    pub cause: Q2AttackCause,
}

/// Read a Q2 attack checkpoint.
pub fn read_q2_attack_checkpoint(reader: SaveReader) -> Result<Q2AttackCheckpoint, PersistenceError> {
    Ok(Q2AttackCheckpoint {
        sequence: nonnegative(reader.field("sequence"))?,
        time: read_time(reader.field("time"))?,
        attacker: reader
            .field("attacker")
            .nullable(|value| read_saved_actor(value).map_err(PersistenceError::from))?,
        inflictor: reader
            .field("inflictor")
            .nullable(|value| read_saved_actor(value).map_err(PersistenceError::from))?,
        originating_projectile: if reader.field("originatingProjectile").is_missing() {
            None
        } else {
            Some(read_saved_actor(reader.field("originatingProjectile"))?)
        },
        damage_powerup_owner: if reader.field("damagePowerupOwner").is_missing() {
            None
        } else {
            Some(namespaced(reader.field("damagePowerupOwner"))?)
        },
        weapon: reader
            .field("weapon")
            .nullable(|value| value.string().map_err(PersistenceError::from))?,
        weapon_provider: namespaced(reader.field("weaponProvider"))?,
        combat_provider: namespaced(reader.field("combatProvider"))?,
        inventory_provider: namespaced(reader.field("inventoryProvider"))?,
        movement_provider: namespaced(reader.field("movementProvider"))?,
        cause: read_cause(reader.field("cause"))?,
    })
}

/// Write a Q2 attack checkpoint.
#[must_use]
pub fn write_q2_attack_checkpoint(attack: &Q2AttackCheckpoint) -> SaveJson {
    #[allow(clippy::cast_possible_wrap)]
    let mut members = vec![
        ("sequence", int(attack.sequence as i64)),
        ("time", write_time(attack.time)),
        ("attacker", attack.attacker.map_or(SaveJson::Null, write_saved_actor)),
        ("inflictor", attack.inflictor.map_or(SaveJson::Null, write_saved_actor)),
    ];
    if let Some(projectile) = attack.originating_projectile {
        members.push(("originatingProjectile", write_saved_actor(projectile)));
    }
    if let Some(owner) = &attack.damage_powerup_owner {
        members.push(("damagePowerupOwner", str(owner)));
    }
    members.extend([
        (
            "weapon",
            attack.weapon.as_ref().map_or(SaveJson::Null, |weapon| str(weapon)),
        ),
        ("weaponProvider", str(&attack.weapon_provider)),
        ("combatProvider", str(&attack.combat_provider)),
        ("inventoryProvider", str(&attack.inventory_provider)),
        ("movementProvider", str(&attack.movement_provider)),
        ("cause", write_cause(&attack.cause)),
    ]);
    obj(members)
}

/// Q2 entity values.
#[derive(Debug, Clone, PartialEq)]
pub struct Q2EntityValues {
    /// Classname.
    pub classname: String,
    /// Target.
    pub target: String,
    /// Targetname.
    pub targetname: String,
    /// Kill target.
    pub killtarget: String,
    /// Combat target.
    pub combat_target: String,
    /// Death target.
    pub death_target: String,
    /// Health target.
    pub health_target: String,
    /// Item target.
    pub item_target: String,
    /// Message.
    pub message: String,
    /// Models.
    pub model: String,
    /// Model 2.
    pub model2: String,
    /// Model 3.
    pub model3: String,
    /// Model 4.
    pub model4: String,
    /// Spawn flags.
    pub spawnflags: f64,
    /// Delay.
    pub delay: f64,
    /// Wait.
    pub wait: f64,
    /// Speed.
    pub speed: f64,
    /// Acceleration.
    pub accel: f64,
    /// Deceleration.
    pub decel: f64,
    /// Damage.
    pub damage: f64,
    /// Damage radius.
    pub damage_radius: f64,
    /// Radius damage.
    pub radius_damage: f64,
    /// Count.
    pub count: f64,
    /// Maximum health.
    pub max_health: f64,
    /// View height.
    pub view_height: f64,
    /// Frame.
    pub frame: f64,
    /// Old frame.
    pub old_frame: f64,
    /// Scale.
    pub scale: f64,
    /// Alpha.
    pub alpha: f64,
    /// Skin.
    pub skin: f64,
    /// Effects.
    pub effects: f64,
    /// Render flags.
    pub render_flags: f64,
    /// Flags.
    pub flags: f64,
    /// Server flags.
    pub server_flags: f64,
    /// Light level.
    pub light_level: f64,
    /// Power cubes.
    pub power_cubes: f64,
    /// Timestamp.
    pub timestamp: f64,
    /// Noise.
    pub noise: String,
    /// Sound.
    pub sound: String,
    /// Volume.
    pub volume: f64,
    /// Attenuation.
    pub attenuation: f64,
    /// Random.
    pub random: f64,
    /// Map.
    pub map: String,
    /// Style.
    pub style: f64,
    /// Transition started.
    pub transition_started: bool,
    /// Clip mask.
    pub clip_mask: f64,
    /// Projectile flag.
    pub projectile: bool,
    /// Dodgeable flag.
    pub dodgeable: bool,
    /// Laser immune flag.
    pub laser_immune: bool,
    /// Damageable target flag.
    pub damageable_target: bool,
    /// Visible flag.
    pub visible: bool,
    /// Solidity.
    pub solid: String,
    /// Motion.
    pub motion: String,
    /// Gravity.
    pub gravity: f64,
    /// Gravity vector.
    pub gravity_vector: Vec3,
    /// Angular velocity.
    pub angular_velocity: Vec3,
    /// Move direction.
    pub movedir: Vec3,
    /// Position 1.
    pub pos1: Vec3,
    /// Position 2.
    pub pos2: Vec3,
    /// Next think.
    pub next_think: Option<f64>,
}

fn read_values(reader: SaveReader, spawn_values: &[(String, String)]) -> Result<Q2EntityValues, PersistenceError> {
    let value = reader.field("values");
    let number = |name: &str| value.field(name).number().map_err(PersistenceError::from);
    let text = |name: &str| value.field(name).string().map_err(PersistenceError::from);
    let flag = |name: &str| value.field(name).boolean().map_err(PersistenceError::from);
    let spawn_default = |key: &str| {
        spawn_values
            .iter()
            .find(|(name, _)| name == key)
            .map_or("", |(_, v)| v)
            .to_string()
    };
    Ok(Q2EntityValues {
        classname: text("classname")?,
        target: text("target")?,
        targetname: text("targetname")?,
        killtarget: text("killtarget")?,
        combat_target: text("combatTarget")?,
        death_target: text("deathTarget")?,
        health_target: if value.field("healthTarget").is_missing() {
            spawn_default("healthtarget")
        } else {
            text("healthTarget")?
        },
        item_target: if value.field("itemTarget").is_missing() {
            spawn_default("itemtarget")
        } else {
            text("itemTarget")?
        },
        message: text("message")?,
        model: text("model")?,
        model2: text("model2")?,
        model3: text("model3")?,
        model4: text("model4")?,
        spawnflags: number("spawnflags")?,
        delay: number("delay")?,
        wait: number("wait")?,
        speed: number("speed")?,
        accel: number("accel")?,
        decel: number("decel")?,
        damage: number("damage")?,
        damage_radius: number("damageRadius")?,
        radius_damage: number("radiusDamage")?,
        count: number("count")?,
        max_health: number("maxHealth")?,
        view_height: number("viewHeight")?,
        frame: number("frame")?,
        old_frame: number("oldFrame")?,
        scale: number("scale")?,
        alpha: if value.field("alpha").is_missing() {
            1.0
        } else {
            number("alpha")?
        },
        skin: number("skin")?,
        effects: number("effects")?,
        render_flags: number("renderFlags")?,
        flags: number("flags")?,
        server_flags: number("serverFlags")?,
        light_level: number("lightLevel")?,
        power_cubes: number("powerCubes")?,
        timestamp: number("timestamp")?,
        noise: text("noise")?,
        sound: text("sound")?,
        volume: number("volume")?,
        attenuation: number("attenuation")?,
        random: number("random")?,
        map: text("map")?,
        style: number("style")?,
        transition_started: flag("transitionStarted")?,
        clip_mask: number("clipMask")?,
        projectile: flag("projectile")?,
        dodgeable: flag("dodgeable")?,
        laser_immune: flag("laserImmune")?,
        damageable_target: flag("damageableTarget")?,
        visible: flag("visible")?,
        solid: value.field("solid").choice_str(&["none", "trigger", "box", "brush"])?,
        motion: value.field("motion").choice_str(&[
            "stationary",
            "push",
            "stop",
            "toss",
            "new-toss",
            "bounce",
            "wall-bounce",
            "fly-missile",
            "fly",
            "step",
        ])?,
        gravity: number("gravity")?,
        gravity_vector: read_vector(value.field("gravityVector"))?,
        angular_velocity: read_vector(value.field("angularVelocity"))?,
        movedir: read_vector(value.field("movedir"))?,
        pos1: read_vector(value.field("pos1"))?,
        pos2: read_vector(value.field("pos2"))?,
        next_think: value
            .field("nextThink")
            .nullable(|item| item.number().map_err(PersistenceError::from))?,
    })
}

#[allow(clippy::too_many_lines)]
fn write_values(values: &Q2EntityValues) -> SaveJson {
    obj(vec![
        ("classname", str(&values.classname)),
        ("target", str(&values.target)),
        ("targetname", str(&values.targetname)),
        ("killtarget", str(&values.killtarget)),
        ("combatTarget", str(&values.combat_target)),
        ("deathTarget", str(&values.death_target)),
        ("healthTarget", str(&values.health_target)),
        ("itemTarget", str(&values.item_target)),
        ("message", str(&values.message)),
        ("model", str(&values.model)),
        ("model2", str(&values.model2)),
        ("model3", str(&values.model3)),
        ("model4", str(&values.model4)),
        ("spawnflags", num(values.spawnflags)),
        ("delay", num(values.delay)),
        ("wait", num(values.wait)),
        ("speed", num(values.speed)),
        ("accel", num(values.accel)),
        ("decel", num(values.decel)),
        ("damage", num(values.damage)),
        ("damageRadius", num(values.damage_radius)),
        ("radiusDamage", num(values.radius_damage)),
        ("count", num(values.count)),
        ("maxHealth", num(values.max_health)),
        ("viewHeight", num(values.view_height)),
        ("frame", num(values.frame)),
        ("oldFrame", num(values.old_frame)),
        ("scale", num(values.scale)),
        ("alpha", num(values.alpha)),
        ("skin", num(values.skin)),
        ("effects", num(values.effects)),
        ("renderFlags", num(values.render_flags)),
        ("flags", num(values.flags)),
        ("serverFlags", num(values.server_flags)),
        ("lightLevel", num(values.light_level)),
        ("powerCubes", num(values.power_cubes)),
        ("timestamp", num(values.timestamp)),
        ("noise", str(&values.noise)),
        ("sound", str(&values.sound)),
        ("volume", num(values.volume)),
        ("attenuation", num(values.attenuation)),
        ("random", num(values.random)),
        ("map", str(&values.map)),
        ("style", num(values.style)),
        ("transitionStarted", boolean(values.transition_started)),
        ("clipMask", num(values.clip_mask)),
        ("projectile", boolean(values.projectile)),
        ("dodgeable", boolean(values.dodgeable)),
        ("laserImmune", boolean(values.laser_immune)),
        ("damageableTarget", boolean(values.damageable_target)),
        ("visible", boolean(values.visible)),
        ("solid", str(&values.solid)),
        ("motion", str(&values.motion)),
        ("gravity", num(values.gravity)),
        ("gravityVector", write_vector(values.gravity_vector)),
        ("angularVelocity", write_vector(values.angular_velocity)),
        ("movedir", write_vector(values.movedir)),
        ("pos1", write_vector(values.pos1)),
        ("pos2", write_vector(values.pos2)),
        ("nextThink", values.next_think.map_or(SaveJson::Null, num)),
    ])
}

/// Q2 entity links.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Q2EntityLinks {
    /// Activator.
    pub activator: Option<SavedActorId>,
    /// Enemy.
    pub enemy: Option<SavedActorId>,
    /// Owner.
    pub owner: Option<SavedActorId>,
    /// Goal.
    pub goal: Option<SavedActorId>,
    /// Team master.
    pub team_master: Option<SavedActorId>,
    /// Team chain.
    pub team_chain: Option<SavedActorId>,
    /// Chain.
    pub chain: Option<SavedActorId>,
    /// Beam.
    pub beam: Option<SavedActorId>,
    /// Second beam.
    pub beam2: Option<SavedActorId>,
    /// Proboscus.
    pub proboscus: Option<SavedActorId>,
}

/// Q2 entity callbacks.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Q2EntityCallbacks {
    /// Think.
    pub think: Option<String>,
    /// Prethink.
    pub prethink: Option<String>,
    /// Postthink.
    pub postthink: Option<String>,
    /// Use.
    pub use_callback: Option<String>,
    /// Touch.
    pub touch: Option<String>,
    /// Pain.
    pub pain: Option<String>,
    /// Die.
    pub die: Option<String>,
    /// Blocked.
    pub blocked: Option<String>,
}

/// Saved Q2 entity checkpoint.
#[derive(Debug, Clone, PartialEq)]
pub struct Q2EntityCheckpoint {
    /// Actor.
    pub actor: SavedActorId,
    /// Source slot.
    pub source_slot: Option<u64>,
    /// Spawn classname.
    pub spawn_classname: String,
    /// Spawn ordinal.
    pub spawn_ordinal: i64,
    /// Spawn values.
    pub spawn_values: Vec<(String, String)>,
    /// Values.
    pub values: Q2EntityValues,
    /// Links.
    pub links: Q2EntityLinks,
    /// Last attack.
    pub last_attack: Option<Q2AttackCheckpoint>,
    /// Callbacks.
    pub callbacks: Q2EntityCallbacks,
}

fn read_entity(reader: SaveReader) -> Result<Q2EntityCheckpoint, PersistenceError> {
    let spawn = reader.field("spawn");
    let links = reader.field("links");
    let callbacks = reader.field("callbacks");
    let spawn_values: Vec<(String, String)> =
        spawn
            .field("values")
            .list(|value| -> Result<(String, String), PersistenceError> {
                Ok((value.field("key").string()?, value.field("value").string()?))
            })?;
    let link = |name: &str| {
        links
            .field(name)
            .nullable(|value| read_saved_actor(value).map_err(PersistenceError::from))
    };
    let callback = |name: &str| -> Result<Option<String>, PersistenceError> {
        callbacks
            .field(name)
            .nullable(|value| value.string().map_err(PersistenceError::from))
    };
    Ok(Q2EntityCheckpoint {
        actor: read_saved_actor(reader.field("actor"))?,
        source_slot: reader.field("sourceSlot").nullable(nonnegative)?,
        spawn_classname: spawn.field("classname").string()?,
        spawn_ordinal: spawn.field("ordinal").integer(i64::MIN)?,
        values: read_values(reader.clone(), &spawn_values)?,
        spawn_values,
        links: Q2EntityLinks {
            activator: link("activator")?,
            enemy: link("enemy")?,
            owner: link("owner")?,
            goal: link("goal")?,
            team_master: link("teamMaster")?,
            team_chain: link("teamChain")?,
            chain: link("chain")?,
            beam: link("beam")?,
            beam2: link("beam2")?,
            proboscus: link("proboscus")?,
        },
        last_attack: reader.field("lastAttack").nullable(read_q2_attack_checkpoint)?,
        callbacks: Q2EntityCallbacks {
            think: callback("think")?,
            prethink: callback("prethink")?,
            postthink: callback("postthink")?,
            use_callback: callback("use")?,
            touch: callback("touch")?,
            pain: callback("pain")?,
            die: callback("die")?,
            blocked: callback("blocked")?,
        },
    })
}

fn write_entity(entity: &Q2EntityCheckpoint) -> SaveJson {
    let link = |actor: &Option<SavedActorId>| actor.map_or(SaveJson::Null, write_saved_actor);
    let callback = |value: &Option<String>| value.as_ref().map_or(SaveJson::Null, |name| str(name));
    obj(vec![
        ("actor", write_saved_actor(entity.actor)),
        (
            "sourceSlot",
            entity.source_slot.map_or(SaveJson::Null, |slot| {
                #[allow(clippy::cast_possible_wrap)]
                int(slot as i64)
            }),
        ),
        (
            "spawn",
            obj(vec![
                ("classname", str(&entity.spawn_classname)),
                ("ordinal", int(entity.spawn_ordinal)),
                (
                    "values",
                    arr(entity
                        .spawn_values
                        .iter()
                        .map(|(key, value)| obj(vec![("key", str(key)), ("value", str(value))]))
                        .collect()),
                ),
            ]),
        ),
        ("values", write_values(&entity.values)),
        (
            "links",
            obj(vec![
                ("activator", link(&entity.links.activator)),
                ("enemy", link(&entity.links.enemy)),
                ("owner", link(&entity.links.owner)),
                ("goal", link(&entity.links.goal)),
                ("teamMaster", link(&entity.links.team_master)),
                ("teamChain", link(&entity.links.team_chain)),
                ("chain", link(&entity.links.chain)),
                ("beam", link(&entity.links.beam)),
                ("beam2", link(&entity.links.beam2)),
                ("proboscus", link(&entity.links.proboscus)),
            ]),
        ),
        (
            "lastAttack",
            entity
                .last_attack
                .as_ref()
                .map_or(SaveJson::Null, write_q2_attack_checkpoint),
        ),
        (
            "callbacks",
            obj(vec![
                ("think", callback(&entity.callbacks.think)),
                ("prethink", callback(&entity.callbacks.prethink)),
                ("postthink", callback(&entity.callbacks.postthink)),
                ("use", callback(&entity.callbacks.use_callback)),
                ("touch", callback(&entity.callbacks.touch)),
                ("pain", callback(&entity.callbacks.pain)),
                ("die", callback(&entity.callbacks.die)),
                ("blocked", callback(&entity.callbacks.blocked)),
            ]),
        ),
    ])
}

/// Q2 foundation counters.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Q2FoundationCounters {
    /// Total secrets.
    pub total_secrets: f64,
    /// Found secrets.
    pub found_secrets: f64,
    /// Total goals.
    pub total_goals: f64,
    /// Found goals.
    pub found_goals: f64,
    /// Total monsters.
    pub total_monsters: f64,
    /// Killed monsters.
    pub killed_monsters: f64,
    /// Server flags.
    pub server_flags: f64,
}

/// Q2 foundation checkpoint.
#[derive(Debug, Clone, PartialEq)]
pub struct Q2FoundationCheckpoint {
    /// Next source slot.
    pub next_source_slot: u64,
    /// Sequence.
    pub sequence: u64,
    /// Freed slots.
    pub freed_slots: Vec<(u64, f64)>,
    /// Counters.
    pub counters: Q2FoundationCounters,
    /// Entities.
    pub entities: Vec<Q2EntityCheckpoint>,
}

/// Read a Q2 foundation checkpoint.
pub fn read_q2_foundation_checkpoint(reader: SaveReader) -> Result<Q2FoundationCheckpoint, PersistenceError> {
    reader.field("version").literal_i64(1)?;
    let counters = reader.field("counters");
    Ok(Q2FoundationCheckpoint {
        next_source_slot: nonnegative(reader.field("nextSourceSlot"))?,
        sequence: nonnegative(reader.field("sequence"))?,
        freed_slots: reader
            .field("freedSlots")
            .list(|value| -> Result<(u64, f64), PersistenceError> {
                Ok((nonnegative(value.field("slot"))?, value.field("time").number()?))
            })?,
        counters: Q2FoundationCounters {
            total_secrets: counters.field("totalSecrets").number()?,
            found_secrets: counters.field("foundSecrets").number()?,
            total_goals: counters.field("totalGoals").number()?,
            found_goals: counters.field("foundGoals").number()?,
            total_monsters: counters.field("totalMonsters").number()?,
            killed_monsters: counters.field("killedMonsters").number()?,
            server_flags: counters.field("serverFlags").number()?,
        },
        entities: reader.field("entities").list(read_entity)?,
    })
}

/// Write a Q2 foundation checkpoint.
#[must_use]
pub fn write_q2_foundation_checkpoint(checkpoint: &Q2FoundationCheckpoint) -> SaveJson {
    #[allow(clippy::cast_possible_wrap)]
    obj(vec![
        ("version", int(1)),
        ("nextSourceSlot", int(checkpoint.next_source_slot as i64)),
        ("sequence", int(checkpoint.sequence as i64)),
        (
            "freedSlots",
            arr(checkpoint
                .freed_slots
                .iter()
                .map(|(slot, time)| obj(vec![("slot", int(*slot as i64)), ("time", num(*time))]))
                .collect()),
        ),
        (
            "counters",
            obj(vec![
                ("totalSecrets", num(checkpoint.counters.total_secrets)),
                ("foundSecrets", num(checkpoint.counters.found_secrets)),
                ("totalGoals", num(checkpoint.counters.total_goals)),
                ("foundGoals", num(checkpoint.counters.found_goals)),
                ("totalMonsters", num(checkpoint.counters.total_monsters)),
                ("killedMonsters", num(checkpoint.counters.killed_monsters)),
                ("serverFlags", num(checkpoint.counters.server_flags)),
            ]),
        ),
        ("entities", arr(checkpoint.entities.iter().map(write_entity).collect())),
    ])
}

/// Encode a Q2 foundation checkpoint.
#[must_use]
pub fn encode_q2_foundation_checkpoint(checkpoint: &Q2FoundationCheckpoint) -> Vec<u8> {
    encode_checkpoint_value(&write_q2_foundation_checkpoint(checkpoint))
}

/// Decode a Q2 foundation checkpoint.
pub fn decode_q2_foundation_checkpoint(bytes: &[u8]) -> Result<Q2FoundationCheckpoint, PersistenceError> {
    let payload = decode_checkpoint_value(bytes)?;
    read_q2_foundation_checkpoint(SaveReader::at(&payload, "q2-foundation"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_values() -> Q2EntityValues {
        Q2EntityValues {
            classname: "monster_soldier".to_string(),
            target: String::new(),
            targetname: "soldier1".to_string(),
            killtarget: String::new(),
            combat_target: String::new(),
            death_target: String::new(),
            health_target: String::new(),
            item_target: String::new(),
            message: String::new(),
            model: "models/monsters/soldier/tris.md2".to_string(),
            model2: String::new(),
            model3: String::new(),
            model4: String::new(),
            spawnflags: 0.0,
            delay: 0.0,
            wait: 0.0,
            speed: 0.0,
            accel: 0.0,
            decel: 0.0,
            damage: 0.0,
            damage_radius: 0.0,
            radius_damage: 0.0,
            count: 0.0,
            max_health: 30.0,
            view_height: 22.0,
            frame: 0.0,
            old_frame: 0.0,
            scale: 1.0,
            alpha: 1.0,
            skin: 0.0,
            effects: 0.0,
            render_flags: 0.0,
            flags: 0.0,
            server_flags: 0.0,
            light_level: 0.0,
            power_cubes: 0.0,
            timestamp: 0.0,
            noise: String::new(),
            sound: String::new(),
            volume: 1.0,
            attenuation: 1.0,
            random: 0.0,
            map: String::new(),
            style: 0.0,
            transition_started: false,
            clip_mask: 0.0,
            projectile: false,
            dodgeable: false,
            laser_immune: false,
            damageable_target: false,
            visible: true,
            solid: "box".to_string(),
            motion: "step".to_string(),
            gravity: 800.0,
            gravity_vector: Vec3 {
                x: 0.0,
                y: 0.0,
                z: -1.0,
            },
            angular_velocity: Vec3 { x: 0.0, y: 0.0, z: 0.0 },
            movedir: Vec3 { x: 0.0, y: 0.0, z: 0.0 },
            pos1: Vec3 { x: 0.0, y: 0.0, z: 0.0 },
            pos2: Vec3 { x: 0.0, y: 0.0, z: 0.0 },
            next_think: None,
        }
    }

    fn sample() -> Q2FoundationCheckpoint {
        Q2FoundationCheckpoint {
            next_source_slot: 4,
            sequence: 7,
            freed_slots: vec![(2, 1.5)],
            counters: Q2FoundationCounters {
                total_secrets: 2.0,
                found_secrets: 1.0,
                ..Q2FoundationCounters::default()
            },
            entities: vec![Q2EntityCheckpoint {
                actor: SavedActorId { slot: 1, generation: 0 },
                source_slot: Some(1),
                spawn_classname: "monster_soldier".to_string(),
                spawn_ordinal: 0,
                spawn_values: vec![("targetname".to_string(), "soldier1".to_string())],
                values: sample_values(),
                links: Q2EntityLinks::default(),
                last_attack: Some(Q2AttackCheckpoint {
                    sequence: 1,
                    time: SourceTime::Milliseconds(100),
                    attacker: None,
                    inflictor: None,
                    originating_projectile: None,
                    damage_powerup_owner: None,
                    weapon: Some("q2:blaster".to_string()),
                    weapon_provider: "q2:game".to_string(),
                    combat_provider: "q2:game".to_string(),
                    inventory_provider: "q2:game".to_string(),
                    movement_provider: "q2:game".to_string(),
                    cause: Q2AttackCause::Q2 {
                        means_of_death: 1,
                        damage_flags: 0,
                        native: Some(Q2NativeCause::Classic {
                            game: "base".to_string(),
                            value: 3,
                        }),
                    },
                }),
                callbacks: Q2EntityCallbacks {
                    think: Some("soldier_think".to_string()),
                    ..Q2EntityCallbacks::default()
                },
            }],
        }
    }

    #[test]
    fn foundation_round_trip() {
        let checkpoint = sample();
        let bytes = encode_q2_foundation_checkpoint(&checkpoint);
        assert_eq!(decode_q2_foundation_checkpoint(&bytes).unwrap(), checkpoint);
    }

    #[test]
    fn spawn_defaults_apply() {
        let mut checkpoint = sample();
        checkpoint.entities[0].values.health_target = "medic1".to_string();
        checkpoint.entities[0]
            .spawn_values
            .push(("healthtarget".to_string(), "medic1".to_string()));
        let mut json = write_q2_foundation_checkpoint(&checkpoint);
        // Drop the explicit value: the spawn table supplies the default.
        if let SaveJson::Object(members) = &mut json {
            for (key, value) in members.iter_mut() {
                if key == "entities" {
                    if let SaveJson::Array(entities) = value {
                        if let SaveJson::Object(fields) = &mut entities[0] {
                            for (name, record) in fields.iter_mut() {
                                if name == "values" {
                                    if let SaveJson::Object(values) = record {
                                        values.retain(|(name, _)| name != "healthTarget");
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        let decoded = read_q2_foundation_checkpoint(SaveReader::new(&json)).unwrap();
        assert_eq!(decoded.entities[0].values.health_target, "medic1");
    }
}
