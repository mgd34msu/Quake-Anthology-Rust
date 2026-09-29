//! Quake I foundation checkpoint ported from `src/persistence/q1-foundation.ts`.

use qa_world::save::records::{read_saved_actor, write_saved_actor};
use qa_world::save::shared::{read_bounds, read_vector, write_bounds, write_vector};
use qa_world::save::value::{
    arr, boolean, decode_checkpoint_value, encode_checkpoint_value, int, namespaced, num, obj, str, SaveJson,
    SaveReader,
};

use super::super::PersistenceError;

/// Q1 weapon ids (base plus mission-pack extras).
pub const Q1_WEAPON_IDS: &[&str] = &[
    "axe",
    "shotgun",
    "supershotgun",
    "nailgun",
    "supernailgun",
    "grenadelauncher",
    "rocketlauncher",
    "lightning",
    "hipnotic:laser",
    "hipnotic:mjolnir",
    "hipnotic:proximity",
    "rogue:lava-nailgun",
    "rogue:lava-supernailgun",
    "rogue:multi-grenade",
    "rogue:multi-rocket",
    "rogue:plasma",
    "rogue:grapple",
    "mg3:laser",
    "mg3:mjolnir",
    "ctf:grapple",
];

/// Q1 powerup ids.
pub const Q1_POWERUP_IDS: &[&str] = &[
    "quad",
    "invulnerability",
    "invisibility",
    "suit",
    "hipnotic:wetsuit",
    "hipnotic:empathy",
    "rogue:shield",
    "rogue:antigrav",
    "mg3:lavasuit",
];

/// Q1 monster species.
pub const Q1_MONSTER_SPECIES: &[&str] = &[
    "army",
    "dog",
    "knight",
    "enforcer",
    "demon",
    "ogre",
    "hellknight",
    "shambler",
    "wizard",
    "shalrath",
    "tarbaby",
    "fish",
    "zombie",
    "boss",
    "oldone",
    "gremlin",
    "scourge",
    "armagon",
    "spikemine",
    "decoy",
    "eel",
    "sword",
    "wrath",
    "super-wrath",
    "mummy",
    "lava-man",
    "morph",
    "dragon",
];

/// Q1 entity source state.
#[derive(Debug, Clone, PartialEq)]
pub struct Q1EntitySourceState {
    /// Model path.
    pub model: String,
    /// Frame.
    pub frame: f64,
    /// Skin.
    pub skin: f64,
    /// Effects mask.
    pub effects: f64,
    /// Solidity.
    pub solid: String,
    /// Movement type.
    pub movement: String,
    /// Target name.
    pub target: String,
    /// Targetname.
    pub targetname: String,
    /// Kill target.
    pub killtarget: String,
    /// Message.
    pub message: String,
    /// Delay.
    pub delay: f64,
    /// Spawn flags.
    pub spawnflags: f64,
    /// Sounds.
    pub sounds: f64,
    /// Wait.
    pub wait: f64,
    /// Speed.
    pub speed: f64,
    /// Damage.
    pub damage: f64,
    /// Maximum health.
    pub max_health: f64,
    /// Aimed damage flag.
    pub aimed_damage: bool,
    /// Next think time.
    pub next_think: f64,
    /// Original model.
    pub original_model: String,
    /// Position 1.
    pub pos1: qa_core::math::Vec3,
    /// Position 2.
    pub pos2: qa_core::math::Vec3,
    /// Destination 1.
    pub dest1: qa_core::math::Vec3,
    /// Destination 2.
    pub dest2: qa_core::math::Vec3,
    /// Move direction.
    pub movedir: qa_core::math::Vec3,
    /// Mangle angles.
    pub mangle: qa_core::math::Vec3,
    /// Mover state.
    pub state: String,
    /// Trigger bounds.
    pub trigger_bounds: Option<qa_core::math::Bounds>,
    /// Attack finished time.
    pub attack_finished: f64,
    /// Count.
    pub count: f64,
    /// Activated flag.
    pub activated: bool,
    /// Projectile kind.
    pub projectile: Option<String>,
    /// Projectile weapon.
    pub projectile_weapon: Option<String>,
    /// Angular velocity.
    pub angular_velocity: qa_core::math::Vec3,
    /// Water level.
    pub water_level: f64,
    /// Water type.
    pub water_type: i64,
    /// Movement flags.
    pub movement_flags: f64,
    /// Ideal yaw.
    pub ideal_yaw: f64,
    /// Yaw speed.
    pub yaw_speed: f64,
    /// Attack state.
    pub attack_state: String,
}

fn read_source_state(reader: SaveReader) -> Result<Q1EntitySourceState, PersistenceError> {
    Ok(Q1EntitySourceState {
        model: reader.field("model").string()?,
        frame: reader.field("frame").number()?,
        skin: reader.field("skin").number()?,
        effects: reader.field("effects").number()?,
        solid: reader
            .field("solid")
            .choice_str(&["none", "trigger", "bbox", "slidebox", "bsp", "corpse"])?,
        movement: reader.field("movement").choice_str(&[
            "none",
            "push",
            "step",
            "toss",
            "bounce",
            "fly",
            "flymissile",
            "noclip",
            "gib",
        ])?,
        target: reader.field("target").string()?,
        targetname: reader.field("targetname").string()?,
        killtarget: reader.field("killtarget").string()?,
        message: reader.field("message").string()?,
        delay: reader.field("delay").number()?,
        spawnflags: reader.field("spawnflags").number()?,
        sounds: reader.field("sounds").number()?,
        wait: reader.field("wait").number()?,
        speed: reader.field("speed").number()?,
        damage: reader.field("damage").number()?,
        max_health: reader.field("maxHealth").number()?,
        aimed_damage: reader.field("aimedDamage").boolean()?,
        next_think: reader.field("nextThink").number()?,
        original_model: reader.field("originalModel").string()?,
        pos1: read_vector(reader.field("pos1"))?,
        pos2: read_vector(reader.field("pos2"))?,
        dest1: read_vector(reader.field("dest1"))?,
        dest2: read_vector(reader.field("dest2"))?,
        movedir: read_vector(reader.field("movedir"))?,
        mangle: read_vector(reader.field("mangle"))?,
        state: reader.field("state").choice_str(&["bottom", "up", "top", "down"])?,
        trigger_bounds: reader
            .field("triggerBounds")
            .nullable(|value| read_bounds(value).map_err(PersistenceError::from))?,
        attack_finished: reader.field("attackFinished").number()?,
        count: reader.field("count").number()?,
        activated: reader.field("activated").boolean()?,
        projectile: reader.field("projectile").nullable(|value| {
            value
                .choice_str(&["rocket", "grenade", "spike", "superspike"])
                .map_err(PersistenceError::from)
        })?,
        projectile_weapon: reader
            .field("projectileWeapon")
            .nullable(|value| value.choice_str(Q1_WEAPON_IDS).map_err(PersistenceError::from))?,
        angular_velocity: read_vector(reader.field("angularVelocity"))?,
        water_level: reader.field("waterLevel").number()?,
        water_type: reader.field("waterType").choice_i64(&[0, -1, -2, -3, -4, -5, -6])?,
        movement_flags: reader.field("movementFlags").number()?,
        ideal_yaw: reader.field("idealYaw").number()?,
        yaw_speed: reader.field("yawSpeed").number()?,
        attack_state: reader
            .field("attackState")
            .choice_str(&["straight", "melee", "missile", "dodging"])?,
    })
}

fn write_source_state(state: &Q1EntitySourceState) -> SaveJson {
    obj(vec![
        ("model", str(&state.model)),
        ("frame", num(state.frame)),
        ("skin", num(state.skin)),
        ("effects", num(state.effects)),
        ("solid", str(&state.solid)),
        ("movement", str(&state.movement)),
        ("target", str(&state.target)),
        ("targetname", str(&state.targetname)),
        ("killtarget", str(&state.killtarget)),
        ("message", str(&state.message)),
        ("delay", num(state.delay)),
        ("spawnflags", num(state.spawnflags)),
        ("sounds", num(state.sounds)),
        ("wait", num(state.wait)),
        ("speed", num(state.speed)),
        ("damage", num(state.damage)),
        ("maxHealth", num(state.max_health)),
        ("aimedDamage", boolean(state.aimed_damage)),
        ("nextThink", num(state.next_think)),
        ("originalModel", str(&state.original_model)),
        ("pos1", write_vector(state.pos1)),
        ("pos2", write_vector(state.pos2)),
        ("dest1", write_vector(state.dest1)),
        ("dest2", write_vector(state.dest2)),
        ("movedir", write_vector(state.movedir)),
        ("mangle", write_vector(state.mangle)),
        ("state", str(&state.state)),
        (
            "triggerBounds",
            state.trigger_bounds.map_or(SaveJson::Null, write_bounds),
        ),
        ("attackFinished", num(state.attack_finished)),
        ("count", num(state.count)),
        ("activated", boolean(state.activated)),
        (
            "projectile",
            state.projectile.as_ref().map_or(SaveJson::Null, |value| str(value)),
        ),
        (
            "projectileWeapon",
            state
                .projectile_weapon
                .as_ref()
                .map_or(SaveJson::Null, |value| str(value)),
        ),
        ("angularVelocity", write_vector(state.angular_velocity)),
        ("waterLevel", num(state.water_level)),
        ("waterType", int(state.water_type)),
        ("movementFlags", num(state.movement_flags)),
        ("idealYaw", num(state.ideal_yaw)),
        ("yawSpeed", num(state.yaw_speed)),
        ("attackState", str(&state.attack_state)),
    ])
}

/// Saved Q1 entity callbacks.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Q1EntityCallbacks {
    /// Think callback.
    pub think: Option<String>,
    /// Use callback.
    pub use_callback: Option<String>,
    /// Touch callback.
    pub touch: Option<String>,
    /// Pain callback.
    pub pain: Option<String>,
    /// Die callback.
    pub die: Option<String>,
    /// Blocked callback.
    pub blocked: Option<String>,
    /// Path-end callback.
    pub path_end: Option<String>,
}

/// Saved Q1 monster state.
#[derive(Debug, Clone, PartialEq)]
pub struct Q1MonsterState {
    /// Species.
    pub species: String,
    /// Mode.
    pub mode: String,
    /// Frame index.
    pub frame_index: f64,
    /// Animation sequence.
    pub sequence: Vec<f64>,
    /// First frame.
    pub first_frame: f64,
    /// Enemy.
    pub enemy: Option<qa_core::identity::SavedActorId>,
    /// Old enemy.
    pub old_enemy: Option<qa_core::identity::SavedActorId>,
    /// Path target.
    pub path: String,
    /// Pause until.
    pub pause_until: f64,
    /// Attack finished.
    pub attack_finished: f64,
    /// Pain finished.
    pub pain_finished: f64,
    /// Search until.
    pub search_until: f64,
    /// Death drop flag.
    pub death_drop: bool,
    /// Refired flag.
    pub refired: bool,
}

/// Saved Q1 entity.
#[derive(Debug, Clone, PartialEq)]
pub struct Q1SavedEntity {
    /// Actor.
    pub actor: qa_core::identity::SavedActorId,
    /// Actor provider.
    pub actor_provider: String,
    /// Source slot.
    pub source_slot: Option<i64>,
    /// Classname.
    pub classname: String,
    /// Source ordinal.
    pub source_ordinal: Option<i64>,
    /// Source state.
    pub state: Q1EntitySourceState,
    /// Spawn fields.
    pub fields: Vec<(String, String)>,
    /// Actor references.
    pub references: Vec<(String, Option<qa_core::identity::SavedActorId>)>,
    /// Owner.
    pub owner: Option<qa_core::identity::SavedActorId>,
    /// Activator.
    pub activator: Option<qa_core::identity::SavedActorId>,
    /// Door group.
    pub door_group: Vec<qa_core::identity::SavedActorId>,
    /// Monster state.
    pub monster: Option<Q1MonsterState>,
    /// Move destination.
    pub move_target: Option<(qa_core::math::Vec3, String)>,
    /// Callbacks.
    pub callbacks: Q1EntityCallbacks,
}

fn read_monster(reader: SaveReader) -> Result<Q1MonsterState, PersistenceError> {
    Ok(Q1MonsterState {
        species: reader.field("species").choice_str(Q1_MONSTER_SPECIES)?,
        mode: reader
            .field("mode")
            .choice_str(&["stand", "walk", "run", "attack", "leap", "pain", "death"])?,
        frame_index: reader.field("frameIndex").number()?,
        sequence: reader
            .field("sequence")
            .list(|frame| frame.number().map_err(PersistenceError::from))?,
        first_frame: reader.field("firstFrame").number()?,
        enemy: reader
            .field("enemy")
            .nullable(|value| read_saved_actor(value).map_err(PersistenceError::from))?,
        old_enemy: reader
            .field("oldEnemy")
            .nullable(|value| read_saved_actor(value).map_err(PersistenceError::from))?,
        path: reader.field("path").string()?,
        pause_until: reader.field("pauseUntil").number()?,
        attack_finished: reader.field("attackFinished").number()?,
        pain_finished: reader.field("painFinished").number()?,
        search_until: reader.field("searchUntil").number()?,
        death_drop: reader.field("deathDrop").boolean()?,
        refired: reader.field("refired").boolean()?,
    })
}

fn write_monster(state: &Q1MonsterState) -> SaveJson {
    obj(vec![
        ("species", str(&state.species)),
        ("mode", str(&state.mode)),
        ("frameIndex", num(state.frame_index)),
        (
            "sequence",
            arr(state.sequence.iter().map(|frame| num(*frame)).collect()),
        ),
        ("firstFrame", num(state.first_frame)),
        ("enemy", state.enemy.map_or(SaveJson::Null, write_saved_actor)),
        ("oldEnemy", state.old_enemy.map_or(SaveJson::Null, write_saved_actor)),
        ("path", str(&state.path)),
        ("pauseUntil", num(state.pause_until)),
        ("attackFinished", num(state.attack_finished)),
        ("painFinished", num(state.pain_finished)),
        ("searchUntil", num(state.search_until)),
        ("deathDrop", boolean(state.death_drop)),
        ("refired", boolean(state.refired)),
    ])
}

fn read_entity(reader: SaveReader) -> Result<Q1SavedEntity, PersistenceError> {
    let callbacks = reader.field("callbacks");
    let name = |key: &str| -> Result<Option<String>, PersistenceError> {
        callbacks
            .field(key)
            .nullable(|value| value.string().map_err(PersistenceError::from))
    };
    Ok(Q1SavedEntity {
        actor: read_saved_actor(reader.field("actor"))?,
        actor_provider: namespaced(reader.field("actorProvider"))?,
        source_slot: reader
            .field("sourceSlot")
            .nullable(|value| value.integer(0).map_err(PersistenceError::from))?,
        classname: reader.field("classname").string()?,
        source_ordinal: reader
            .field("sourceOrdinal")
            .nullable(|value| value.integer(i64::MIN).map_err(PersistenceError::from))?,
        state: read_source_state(reader.field("state"))?,
        fields: reader
            .field("fields")
            .list(|value| -> Result<(String, String), PersistenceError> {
                Ok((value.field("key").string()?, value.field("value").string()?))
            })?,
        references: reader.field("references").list(
            |value| -> Result<(String, Option<qa_core::identity::SavedActorId>), PersistenceError> {
                Ok((
                    value.field("key").string()?,
                    value
                        .field("actor")
                        .nullable(|actor| read_saved_actor(actor).map_err(PersistenceError::from))?,
                ))
            },
        )?,
        owner: reader
            .field("owner")
            .nullable(|value| read_saved_actor(value).map_err(PersistenceError::from))?,
        activator: reader
            .field("activator")
            .nullable(|value| read_saved_actor(value).map_err(PersistenceError::from))?,
        door_group: reader
            .field("doorGroup")
            .list(|value| read_saved_actor(value).map_err(PersistenceError::from))?,
        monster: reader.field("monster").nullable(read_monster)?,
        move_target: reader.field("move").nullable(
            |value| -> Result<(qa_core::math::Vec3, String), PersistenceError> {
                Ok((read_vector(value.field("destination"))?, value.field("done").string()?))
            },
        )?,
        callbacks: Q1EntityCallbacks {
            think: name("think")?,
            use_callback: name("use")?,
            touch: name("touch")?,
            pain: name("pain")?,
            die: name("die")?,
            blocked: name("blocked")?,
            path_end: name("pathEnd")?,
        },
    })
}

fn write_entity(entity: &Q1SavedEntity) -> SaveJson {
    obj(vec![
        ("actor", write_saved_actor(entity.actor)),
        ("actorProvider", str(&entity.actor_provider)),
        ("sourceSlot", entity.source_slot.map_or(SaveJson::Null, int)),
        ("classname", str(&entity.classname)),
        ("sourceOrdinal", entity.source_ordinal.map_or(SaveJson::Null, int)),
        ("state", write_source_state(&entity.state)),
        (
            "fields",
            arr(entity
                .fields
                .iter()
                .map(|(key, value)| obj(vec![("key", str(key)), ("value", str(value))]))
                .collect()),
        ),
        (
            "references",
            arr(entity
                .references
                .iter()
                .map(|(key, actor)| {
                    obj(vec![
                        ("key", str(key)),
                        ("actor", actor.map_or(SaveJson::Null, write_saved_actor)),
                    ])
                })
                .collect()),
        ),
        ("owner", entity.owner.map_or(SaveJson::Null, write_saved_actor)),
        ("activator", entity.activator.map_or(SaveJson::Null, write_saved_actor)),
        (
            "doorGroup",
            arr(entity
                .door_group
                .iter()
                .map(|actor| write_saved_actor(*actor))
                .collect()),
        ),
        ("monster", entity.monster.as_ref().map_or(SaveJson::Null, write_monster)),
        (
            "move",
            entity
                .move_target
                .as_ref()
                .map_or(SaveJson::Null, |(destination, done)| {
                    obj(vec![("destination", write_vector(*destination)), ("done", str(done))])
                }),
        ),
        (
            "callbacks",
            obj(vec![
                (
                    "think",
                    entity
                        .callbacks
                        .think
                        .as_ref()
                        .map_or(SaveJson::Null, |value| str(value)),
                ),
                (
                    "use",
                    entity
                        .callbacks
                        .use_callback
                        .as_ref()
                        .map_or(SaveJson::Null, |value| str(value)),
                ),
                (
                    "touch",
                    entity
                        .callbacks
                        .touch
                        .as_ref()
                        .map_or(SaveJson::Null, |value| str(value)),
                ),
                (
                    "pain",
                    entity
                        .callbacks
                        .pain
                        .as_ref()
                        .map_or(SaveJson::Null, |value| str(value)),
                ),
                (
                    "die",
                    entity.callbacks.die.as_ref().map_or(SaveJson::Null, |value| str(value)),
                ),
                (
                    "blocked",
                    entity
                        .callbacks
                        .blocked
                        .as_ref()
                        .map_or(SaveJson::Null, |value| str(value)),
                ),
                (
                    "pathEnd",
                    entity
                        .callbacks
                        .path_end
                        .as_ref()
                        .map_or(SaveJson::Null, |value| str(value)),
                ),
            ]),
        ),
    ])
}

/// Saved Q1 player state.
#[derive(Debug, Clone, PartialEq)]
pub struct Q1PlayerState {
    /// Alpha.
    pub alpha: f64,
    /// Scale.
    pub scale: f64,
    /// Weapon.
    pub weapon: String,
    /// Primary holstered.
    pub primary_holstered: bool,
    /// Attack held.
    pub attack_held: bool,
    /// Jump held.
    pub jump_held: bool,
    /// Teleport until.
    pub teleport_until: f64,
    /// Attack finished.
    pub attack_finished: f64,
    /// Weapon frame.
    pub weapon_frame: f64,
    /// Weapon animation at.
    pub weapon_animation_at: f64,
    /// Weapon animation base.
    pub weapon_animation_base: f64,
    /// Continuous firing.
    pub continuous_firing: bool,
    /// Next weapon frame.
    pub next_weapon_frame: f64,
    /// Lightning sound at.
    pub lightning_sound_at: f64,
    /// Punch angles.
    pub punch_angles: qa_core::math::Vec3,
    /// Nail side.
    pub nail_side: f64,
    /// Maximum health.
    pub max_health: f64,
    /// Mega rotation at.
    pub mega_rot_at: f64,
    /// Hostile until.
    pub hostile_until: f64,
    /// View angles.
    pub view_angles: qa_core::math::Vec3,
    /// Water level.
    pub water_level: f64,
    /// Air finished.
    pub air_finished: f64,
    /// Drown damage.
    pub drown_damage: f64,
    /// Drown at.
    pub drown_at: f64,
    /// Hazard at.
    pub hazard_at: f64,
    /// Auto switch.
    pub auto_switch: String,
}

/// Saved Q1 player.
#[derive(Debug, Clone, PartialEq)]
pub struct Q1SavedPlayer {
    /// Actor.
    pub actor: qa_core::identity::SavedActorId,
    /// Actor provider.
    pub actor_provider: String,
    /// State.
    pub state: Q1PlayerState,
    /// Powerups.
    pub powerups: Vec<(String, f64)>,
}

fn read_player(reader: SaveReader) -> Result<Q1SavedPlayer, PersistenceError> {
    let state = reader.field("state");
    Ok(Q1SavedPlayer {
        actor: read_saved_actor(reader.field("actor"))?,
        actor_provider: namespaced(reader.field("actorProvider"))?,
        state: Q1PlayerState {
            alpha: if state.field("alpha").is_missing() {
                0.0
            } else {
                state.field("alpha").number()?
            },
            scale: if state.field("scale").is_missing() {
                0.0
            } else {
                state.field("scale").number()?
            },
            weapon: state.field("weapon").choice_str(Q1_WEAPON_IDS)?,
            primary_holstered: state.field("primaryHolstered").boolean()?,
            attack_held: state.field("attackHeld").boolean()?,
            jump_held: state.field("jumpHeld").boolean()?,
            teleport_until: state.field("teleportUntil").number()?,
            attack_finished: state.field("attackFinished").number()?,
            weapon_frame: state.field("weaponFrame").number()?,
            weapon_animation_at: state.field("weaponAnimationAt").number()?,
            weapon_animation_base: state.field("weaponAnimationBase").number()?,
            continuous_firing: state.field("continuousFiring").boolean()?,
            next_weapon_frame: state.field("nextWeaponFrame").number()?,
            lightning_sound_at: state.field("lightningSoundAt").number()?,
            punch_angles: read_vector(state.field("punchAngles"))?,
            nail_side: state.field("nailSide").number()?,
            max_health: state.field("maxHealth").number()?,
            mega_rot_at: state.field("megaRotAt").number()?,
            hostile_until: state.field("hostileUntil").number()?,
            view_angles: read_vector(state.field("viewAngles"))?,
            water_level: state.field("waterLevel").number()?,
            air_finished: state.field("airFinished").number()?,
            drown_damage: state.field("drownDamage").number()?,
            drown_at: state.field("drownAt").number()?,
            hazard_at: state.field("hazardAt").number()?,
            auto_switch: state.field("autoSwitch").choice_str(&["always", "new", "never"])?,
        },
        powerups: reader
            .field("powerups")
            .list(|value| -> Result<(String, f64), PersistenceError> {
                Ok((
                    value.field("kind").choice_str(Q1_POWERUP_IDS)?,
                    value.field("expires").number()?,
                ))
            })?,
    })
}

fn write_player(player: &Q1SavedPlayer) -> SaveJson {
    obj(vec![
        ("actor", write_saved_actor(player.actor)),
        ("actorProvider", str(&player.actor_provider)),
        (
            "state",
            obj(vec![
                ("alpha", num(player.state.alpha)),
                ("scale", num(player.state.scale)),
                ("weapon", str(&player.state.weapon)),
                ("primaryHolstered", boolean(player.state.primary_holstered)),
                ("attackHeld", boolean(player.state.attack_held)),
                ("jumpHeld", boolean(player.state.jump_held)),
                ("teleportUntil", num(player.state.teleport_until)),
                ("attackFinished", num(player.state.attack_finished)),
                ("weaponFrame", num(player.state.weapon_frame)),
                ("weaponAnimationAt", num(player.state.weapon_animation_at)),
                ("weaponAnimationBase", num(player.state.weapon_animation_base)),
                ("continuousFiring", boolean(player.state.continuous_firing)),
                ("nextWeaponFrame", num(player.state.next_weapon_frame)),
                ("lightningSoundAt", num(player.state.lightning_sound_at)),
                ("punchAngles", write_vector(player.state.punch_angles)),
                ("nailSide", num(player.state.nail_side)),
                ("maxHealth", num(player.state.max_health)),
                ("megaRotAt", num(player.state.mega_rot_at)),
                ("hostileUntil", num(player.state.hostile_until)),
                ("viewAngles", write_vector(player.state.view_angles)),
                ("waterLevel", num(player.state.water_level)),
                ("airFinished", num(player.state.air_finished)),
                ("drownDamage", num(player.state.drown_damage)),
                ("drownAt", num(player.state.drown_at)),
                ("hazardAt", num(player.state.hazard_at)),
                ("autoSwitch", str(&player.state.auto_switch)),
            ]),
        ),
        (
            "powerups",
            arr(player
                .powerups
                .iter()
                .map(|(kind, expires)| obj(vec![("kind", str(kind)), ("expires", num(*expires))]))
                .collect()),
        ),
    ])
}

/// Q1 foundation checkpoint.
#[derive(Debug, Clone, PartialEq)]
pub struct Q1FoundationCheckpoint {
    /// Provider.
    pub provider: String,
    /// Precache phase.
    pub precache_phase: String,
    /// Precached models.
    pub precache_models: Vec<String>,
    /// Precached sounds.
    pub precache_sounds: Vec<String>,
    /// Edition.
    pub edition: String,
    /// Time.
    pub time: f64,
    /// Frame seconds.
    pub frame_seconds: f64,
    /// Force retouch.
    pub force_retouch: f64,
    /// Sequence.
    pub sequence: u64,
    /// Next dynamic slot.
    pub next_dynamic_slot: u64,
    /// Total secrets.
    pub total_secrets: f64,
    /// Found secrets.
    pub found_secrets: f64,
    /// Total monsters.
    pub total_monsters: f64,
    /// Killed monsters.
    pub killed_monsters: f64,
    /// World type.
    pub world_type: f64,
    /// Map name.
    pub map_name: String,
    /// Basis vectors.
    pub basis: (qa_core::math::Vec3, qa_core::math::Vec3, qa_core::math::Vec3),
    /// World actor.
    pub world: Option<qa_core::identity::SavedActorId>,
    /// Sight entity.
    pub sight_entity: Option<qa_core::identity::SavedActorId>,
    /// Sight time.
    pub sight_time: f64,
    /// Intermission state.
    pub intermission: Option<Q1Intermission>,
    /// Entities.
    pub entities: Vec<Q1SavedEntity>,
    /// Players.
    pub players: Vec<Q1SavedPlayer>,
    /// Extension blobs.
    pub extensions: Vec<(String, Vec<u8>)>,
}

/// Q1 intermission state.
#[derive(Debug, Clone, PartialEq)]
pub struct Q1Intermission {
    /// Map.
    pub map: String,
    /// Cause.
    pub cause: Option<qa_core::identity::SavedActorId>,
    /// Exit after.
    pub exit_after: f64,
}

/// Read a Q1 foundation checkpoint.
pub fn read_q1_foundation_checkpoint(reader: SaveReader) -> Result<Q1FoundationCheckpoint, PersistenceError> {
    let precaches = reader.field("precaches");
    let basis = reader.field("basis");
    let nonnegative = |reader: SaveReader| -> Result<u64, PersistenceError> {
        let value = reader.integer(0)?;
        u64::try_from(value).map_err(|_| PersistenceError::from(reader.fail("expected an integer in range")))
    };
    Ok(Q1FoundationCheckpoint {
        provider: namespaced(reader.field("provider"))?,
        precache_phase: {
            reader.field("format").literal_str("q1-foundation")?;
            reader.field("version").literal_i64(5)?;
            precaches.field("phase").choice_str(&["loading", "frozen"])?
        },
        precache_models: precaches
            .field("models")
            .list(|value| value.string().map_err(PersistenceError::from))?,
        precache_sounds: precaches
            .field("sounds")
            .list(|value| value.string().map_err(PersistenceError::from))?,
        edition: reader.field("edition").choice_str(&["classic", "rerelease"])?,
        time: reader.field("time").number()?,
        frame_seconds: reader.field("frameSeconds").number()?,
        force_retouch: reader.field("forceRetouch").number()?,
        sequence: nonnegative(reader.field("sequence"))?,
        next_dynamic_slot: nonnegative(reader.field("nextDynamicSlot"))?,
        total_secrets: reader.field("totalSecrets").number()?,
        found_secrets: reader.field("foundSecrets").number()?,
        total_monsters: reader.field("totalMonsters").number()?,
        killed_monsters: reader.field("killedMonsters").number()?,
        world_type: reader.field("worldType").number()?,
        map_name: reader.field("mapName").string()?,
        basis: (
            read_vector(basis.field("forward"))?,
            read_vector(basis.field("right"))?,
            read_vector(basis.field("up"))?,
        ),
        world: reader
            .field("world")
            .nullable(|value| read_saved_actor(value).map_err(PersistenceError::from))?,
        sight_entity: reader
            .field("sightEntity")
            .nullable(|value| read_saved_actor(value).map_err(PersistenceError::from))?,
        sight_time: reader.field("sightTime").number()?,
        intermission: reader
            .field("intermission")
            .nullable(|value| -> Result<Q1Intermission, PersistenceError> {
                Ok(Q1Intermission {
                    map: value.field("map").string()?,
                    cause: value
                        .field("cause")
                        .nullable(|cause| read_saved_actor(cause).map_err(PersistenceError::from))?,
                    exit_after: value.field("exitAfter").number()?,
                })
            })?,
        entities: reader.field("entities").list(read_entity)?,
        players: reader.field("players").list(read_player)?,
        extensions: reader
            .field("extensions")
            .list(|value| -> Result<(String, Vec<u8>), PersistenceError> {
                Ok((value.field("id").string()?, value.field("bytes").bytes()?))
            })?,
    })
}

/// Write a Q1 foundation checkpoint.
#[must_use]
#[allow(clippy::cast_possible_wrap)]
pub fn write_q1_foundation_checkpoint(checkpoint: &Q1FoundationCheckpoint) -> SaveJson {
    obj(vec![
        ("provider", str(&checkpoint.provider)),
        ("format", str("q1-foundation")),
        ("version", int(5)),
        (
            "precaches",
            obj(vec![
                ("phase", str(&checkpoint.precache_phase)),
                (
                    "models",
                    arr(checkpoint.precache_models.iter().map(|model| str(model)).collect()),
                ),
                (
                    "sounds",
                    arr(checkpoint.precache_sounds.iter().map(|sound| str(sound)).collect()),
                ),
            ]),
        ),
        ("edition", str(&checkpoint.edition)),
        ("time", num(checkpoint.time)),
        ("frameSeconds", num(checkpoint.frame_seconds)),
        ("forceRetouch", num(checkpoint.force_retouch)),
        ("sequence", int(checkpoint.sequence as i64)),
        ("nextDynamicSlot", int(checkpoint.next_dynamic_slot as i64)),
        ("totalSecrets", num(checkpoint.total_secrets)),
        ("foundSecrets", num(checkpoint.found_secrets)),
        ("totalMonsters", num(checkpoint.total_monsters)),
        ("killedMonsters", num(checkpoint.killed_monsters)),
        ("worldType", num(checkpoint.world_type)),
        ("mapName", str(&checkpoint.map_name)),
        (
            "basis",
            obj(vec![
                ("forward", write_vector(checkpoint.basis.0)),
                ("right", write_vector(checkpoint.basis.1)),
                ("up", write_vector(checkpoint.basis.2)),
            ]),
        ),
        ("world", checkpoint.world.map_or(SaveJson::Null, write_saved_actor)),
        (
            "sightEntity",
            checkpoint.sight_entity.map_or(SaveJson::Null, write_saved_actor),
        ),
        ("sightTime", num(checkpoint.sight_time)),
        (
            "intermission",
            checkpoint.intermission.as_ref().map_or(SaveJson::Null, |intermission| {
                obj(vec![
                    ("map", str(&intermission.map)),
                    ("cause", intermission.cause.map_or(SaveJson::Null, write_saved_actor)),
                    ("exitAfter", num(intermission.exit_after)),
                ])
            }),
        ),
        ("entities", arr(checkpoint.entities.iter().map(write_entity).collect())),
        ("players", arr(checkpoint.players.iter().map(write_player).collect())),
        (
            "extensions",
            arr(checkpoint
                .extensions
                .iter()
                .map(|(id, bytes)| obj(vec![("id", str(id)), ("bytes", SaveJson::Bytes(bytes.clone()))]))
                .collect()),
        ),
    ])
}

/// Encode a Q1 foundation checkpoint.
#[must_use]
pub fn encode_q1_foundation_checkpoint(checkpoint: &Q1FoundationCheckpoint) -> Vec<u8> {
    encode_checkpoint_value(&write_q1_foundation_checkpoint(checkpoint))
}

/// Decode a Q1 foundation checkpoint.
pub fn decode_q1_foundation_checkpoint(bytes: &[u8]) -> Result<Q1FoundationCheckpoint, PersistenceError> {
    let payload = decode_checkpoint_value(bytes).map_err(|error| PersistenceError::BadSave(error.to_string()))?;
    read_q1_foundation_checkpoint(SaveReader::at(&payload, "q1-foundation"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_core::identity::SavedActorId;
    use qa_core::math::Vec3;

    fn zero() -> Vec3 {
        Vec3 { x: 0.0, y: 0.0, z: 0.0 }
    }

    fn sample_state() -> Q1EntitySourceState {
        Q1EntitySourceState {
            model: "progs/ogre.mdl".to_string(),
            frame: 0.0,
            skin: 0.0,
            effects: 0.0,
            solid: "slidebox".to_string(),
            movement: "step".to_string(),
            target: String::new(),
            targetname: String::new(),
            killtarget: String::new(),
            message: String::new(),
            delay: 0.0,
            spawnflags: 0.0,
            sounds: 1.0,
            wait: 0.0,
            speed: 0.0,
            damage: 0.0,
            max_health: 200.0,
            aimed_damage: false,
            next_think: 0.0,
            original_model: String::new(),
            pos1: zero(),
            pos2: zero(),
            dest1: zero(),
            dest2: zero(),
            movedir: zero(),
            mangle: zero(),
            state: "bottom".to_string(),
            trigger_bounds: None,
            attack_finished: 0.0,
            count: 0.0,
            activated: false,
            projectile: None,
            projectile_weapon: None,
            angular_velocity: zero(),
            water_level: 0.0,
            water_type: 0,
            movement_flags: 0.0,
            ideal_yaw: 0.0,
            yaw_speed: 0.0,
            attack_state: "straight".to_string(),
        }
    }

    fn sample() -> Q1FoundationCheckpoint {
        Q1FoundationCheckpoint {
            provider: "q1:game".to_string(),
            precache_phase: "frozen".to_string(),
            precache_models: vec!["progs/ogre.mdl".to_string()],
            precache_sounds: vec!["ogre/ogdrag.wav".to_string()],
            edition: "classic".to_string(),
            time: 10.0,
            frame_seconds: 0.1,
            force_retouch: 2.0,
            sequence: 4,
            next_dynamic_slot: 8,
            total_secrets: 3.0,
            found_secrets: 1.0,
            total_monsters: 12.0,
            killed_monsters: 2.0,
            world_type: 0.0,
            map_name: "e1m1".to_string(),
            basis: (zero(), zero(), zero()),
            world: Some(SavedActorId { slot: 0, generation: 0 }),
            sight_entity: None,
            sight_time: 0.0,
            intermission: None,
            entities: vec![Q1SavedEntity {
                actor: SavedActorId { slot: 1, generation: 0 },
                actor_provider: "q1:game".to_string(),
                source_slot: Some(1),
                classname: "monster_ogre".to_string(),
                source_ordinal: Some(0),
                state: sample_state(),
                fields: vec![("targetname".to_string(), "ogre1".to_string())],
                references: Vec::new(),
                owner: None,
                activator: None,
                door_group: Vec::new(),
                monster: Some(Q1MonsterState {
                    species: "ogre".to_string(),
                    mode: "stand".to_string(),
                    frame_index: 0.0,
                    sequence: vec![0.0, 1.0],
                    first_frame: 0.0,
                    enemy: None,
                    old_enemy: None,
                    path: String::new(),
                    pause_until: 0.0,
                    attack_finished: 0.0,
                    pain_finished: 0.0,
                    search_until: 0.0,
                    death_drop: false,
                    refired: false,
                }),
                move_target: None,
                callbacks: Q1EntityCallbacks {
                    think: Some("ogre_stand".to_string()),
                    ..Q1EntityCallbacks::default()
                },
            }],
            players: vec![Q1SavedPlayer {
                actor: SavedActorId { slot: 2, generation: 0 },
                actor_provider: "q1:game".to_string(),
                state: Q1PlayerState {
                    alpha: 0.0,
                    scale: 0.0,
                    weapon: "shotgun".to_string(),
                    primary_holstered: false,
                    attack_held: false,
                    jump_held: false,
                    teleport_until: 0.0,
                    attack_finished: 0.0,
                    weapon_frame: 0.0,
                    weapon_animation_at: 0.0,
                    weapon_animation_base: 0.0,
                    continuous_firing: false,
                    next_weapon_frame: 0.0,
                    lightning_sound_at: 0.0,
                    punch_angles: zero(),
                    nail_side: 0.0,
                    max_health: 100.0,
                    mega_rot_at: 0.0,
                    hostile_until: 0.0,
                    view_angles: zero(),
                    water_level: 0.0,
                    air_finished: 0.0,
                    drown_damage: 0.0,
                    drown_at: 0.0,
                    hazard_at: 0.0,
                    auto_switch: "always".to_string(),
                },
                powerups: vec![("quad".to_string(), 30.0)],
            }],
            extensions: vec![("q1:extra".to_string(), vec![1, 2])],
        }
    }

    #[test]
    fn foundation_round_trip() {
        let checkpoint = sample();
        let bytes = encode_q1_foundation_checkpoint(&checkpoint);
        assert_eq!(decode_q1_foundation_checkpoint(&bytes).unwrap(), checkpoint);
    }

    #[test]
    fn foundation_rejects_bad_versions() {
        let mut json = write_q1_foundation_checkpoint(&sample());
        if let SaveJson::Object(members) = &mut json {
            for (key, value) in members.iter_mut() {
                if key == "version" {
                    *value = int(4);
                }
            }
        }
        assert!(read_q1_foundation_checkpoint(SaveReader::new(&json)).is_err());
    }
}
