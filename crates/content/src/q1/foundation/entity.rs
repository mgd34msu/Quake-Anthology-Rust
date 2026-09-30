//! Q1 source entity records (`src/content/q1/foundation/entity.ts`).
//!
//! Copyright (C) 1996-2022 id Software LLC. GPL-2.0-or-later.
//!
//! Source continuations (`think`, `use`, `touch`, `pain`, `die`,
//! `blocked`, `pathEnd`, and mover completion) are stored as registered
//! callback names and dispatched through
//! [`Q1EntityServices`](super::entity_services::Q1EntityServices); the
//! donor's per-entity closures all originate from the named registry,
//! so the name preserves the full save/clone behavior. Actor links use
//! ids; live records resolve through the services object.

use std::collections::HashMap;

use qa_core::identity::{ActorId, OwnedActor};
use qa_core::math::{Bounds, Vec3};

use crate::bsp::{q1_entity_value, Q1Entity};
use crate::q1::Q1Error;

use super::entity_services::Q1EntityServices;
use super::text::q1_entity_string;
use super::types::{vectors, Q1MoveType, Q1Solid, Q1Weapon, ZERO};

/// Monster species (`Q1MonsterSpecies`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Q1MonsterSpecies {
    /// Grunt.
    Army,
    /// Rottweiler.
    Dog,
    /// Knight.
    Knight,
    /// Enforcer.
    Enforcer,
    /// Fiend.
    Demon,
    /// Ogre.
    Ogre,
    /// Death knight.
    Hellknight,
    /// Shambler.
    Shambler,
    /// Scrag.
    Wizard,
    /// Vore.
    Shalrath,
    /// Spawn.
    Tarbaby,
    /// Rotfish.
    Fish,
    /// Zombie.
    Zombie,
    /// Chthon.
    Boss,
    /// Shub-Niggurath.
    Oldone,
    /// Gremlin.
    Gremlin,
    /// Scourge.
    Scourge,
    /// Armagon.
    Armagon,
    /// Spike mine.
    Spikemine,
    /// Decoy.
    Decoy,
    /// Electric eel.
    Eel,
    /// Sword.
    Sword,
    /// Wrath.
    Wrath,
    /// Super wrath.
    SuperWrath,
    /// Mummy.
    Mummy,
    /// Lava man.
    LavaMan,
    /// Morph.
    Morph,
    /// Dragon.
    Dragon,
}

impl Q1MonsterSpecies {
    /// Donor species text.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Q1MonsterSpecies::Army => "army",
            Q1MonsterSpecies::Dog => "dog",
            Q1MonsterSpecies::Knight => "knight",
            Q1MonsterSpecies::Enforcer => "enforcer",
            Q1MonsterSpecies::Demon => "demon",
            Q1MonsterSpecies::Ogre => "ogre",
            Q1MonsterSpecies::Hellknight => "hellknight",
            Q1MonsterSpecies::Shambler => "shambler",
            Q1MonsterSpecies::Wizard => "wizard",
            Q1MonsterSpecies::Shalrath => "shalrath",
            Q1MonsterSpecies::Tarbaby => "tarbaby",
            Q1MonsterSpecies::Fish => "fish",
            Q1MonsterSpecies::Zombie => "zombie",
            Q1MonsterSpecies::Boss => "boss",
            Q1MonsterSpecies::Oldone => "oldone",
            Q1MonsterSpecies::Gremlin => "gremlin",
            Q1MonsterSpecies::Scourge => "scourge",
            Q1MonsterSpecies::Armagon => "armagon",
            Q1MonsterSpecies::Spikemine => "spikemine",
            Q1MonsterSpecies::Decoy => "decoy",
            Q1MonsterSpecies::Eel => "eel",
            Q1MonsterSpecies::Sword => "sword",
            Q1MonsterSpecies::Wrath => "wrath",
            Q1MonsterSpecies::SuperWrath => "super-wrath",
            Q1MonsterSpecies::Mummy => "mummy",
            Q1MonsterSpecies::LavaMan => "lava-man",
            Q1MonsterSpecies::Morph => "morph",
            Q1MonsterSpecies::Dragon => "dragon",
        }
    }

    /// Parse donor species text.
    pub fn parse(text: &str) -> Result<Self, Q1Error> {
        for species in Q1_MONSTER_SPECIES {
            if species.as_str() == text {
                return Ok(species);
            }
        }
        Err(Q1Error::Message(format!("Unknown Q1 monster species: {text}")))
    }
}

/// Monster species roster in donor order (`Q1_MONSTER_SPECIES`).
pub const Q1_MONSTER_SPECIES: [Q1MonsterSpecies; 28] = [
    Q1MonsterSpecies::Army,
    Q1MonsterSpecies::Dog,
    Q1MonsterSpecies::Knight,
    Q1MonsterSpecies::Enforcer,
    Q1MonsterSpecies::Demon,
    Q1MonsterSpecies::Ogre,
    Q1MonsterSpecies::Hellknight,
    Q1MonsterSpecies::Shambler,
    Q1MonsterSpecies::Wizard,
    Q1MonsterSpecies::Shalrath,
    Q1MonsterSpecies::Tarbaby,
    Q1MonsterSpecies::Fish,
    Q1MonsterSpecies::Zombie,
    Q1MonsterSpecies::Boss,
    Q1MonsterSpecies::Oldone,
    Q1MonsterSpecies::Gremlin,
    Q1MonsterSpecies::Scourge,
    Q1MonsterSpecies::Armagon,
    Q1MonsterSpecies::Spikemine,
    Q1MonsterSpecies::Decoy,
    Q1MonsterSpecies::Eel,
    Q1MonsterSpecies::Sword,
    Q1MonsterSpecies::Wrath,
    Q1MonsterSpecies::SuperWrath,
    Q1MonsterSpecies::Mummy,
    Q1MonsterSpecies::LavaMan,
    Q1MonsterSpecies::Morph,
    Q1MonsterSpecies::Dragon,
];

/// Monster behavior mode (`Q1Monster["mode"]`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Q1MonsterMode {
    /// Standing.
    Stand,
    /// Walking.
    Walk,
    /// Running.
    Run,
    /// Attacking.
    Attack,
    /// Leaping.
    Leap,
    /// In pain.
    Pain,
    /// Dying.
    Death,
}

impl Q1MonsterMode {
    /// Donor mode text.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Q1MonsterMode::Stand => "stand",
            Q1MonsterMode::Walk => "walk",
            Q1MonsterMode::Run => "run",
            Q1MonsterMode::Attack => "attack",
            Q1MonsterMode::Leap => "leap",
            Q1MonsterMode::Pain => "pain",
            Q1MonsterMode::Death => "death",
        }
    }

    /// Parse donor mode text.
    pub fn parse(text: &str) -> Result<Self, Q1Error> {
        match text {
            "stand" => Ok(Q1MonsterMode::Stand),
            "walk" => Ok(Q1MonsterMode::Walk),
            "run" => Ok(Q1MonsterMode::Run),
            "attack" => Ok(Q1MonsterMode::Attack),
            "leap" => Ok(Q1MonsterMode::Leap),
            "pain" => Ok(Q1MonsterMode::Pain),
            "death" => Ok(Q1MonsterMode::Death),
            _ => Err(Q1Error::Message(format!("Unknown Q1 monster mode: {text}"))),
        }
    }
}

/// Monster behavior state (`Q1Monster`).
#[derive(Debug, Clone, PartialEq)]
pub struct Q1Monster {
    /// Species.
    pub species: Q1MonsterSpecies,
    /// Behavior mode.
    pub mode: Q1MonsterMode,
    /// Frame index within the sequence.
    pub frame_index: usize,
    /// Step distances per frame.
    pub sequence: Vec<f64>,
    /// First model frame of the sequence.
    pub first_frame: i32,
    /// Current enemy.
    pub enemy: Option<ActorId>,
    /// Previous enemy.
    pub old_enemy: Option<ActorId>,
    /// Patrol path name.
    pub path: String,
    /// Patrol pause expiry in seconds.
    pub pause_until: f64,
    /// Next allowed attack time in seconds.
    pub attack_finished: f64,
    /// Pain cooldown expiry in seconds.
    pub pain_finished: f64,
    /// Target search expiry in seconds.
    pub search_until: f64,
    /// Whether the death drop ran.
    pub death_drop: bool,
    /// Whether the attack refired.
    pub refired: bool,
}

/// Pending mover completion (`Q1Move`). The completion callback is a
/// registered action name.
#[derive(Debug, Clone, PartialEq)]
pub struct Q1Move {
    /// Move destination.
    pub destination: Vec3,
    /// Completion action name.
    pub done: String,
}

/// Mover position state (`Q1Actor["state"]`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Q1MoverState {
    /// At the bottom.
    #[default]
    Bottom,
    /// Moving up.
    Up,
    /// At the top.
    Top,
    /// Moving down.
    Down,
}

impl Q1MoverState {
    /// Donor state text.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Q1MoverState::Bottom => "bottom",
            Q1MoverState::Up => "up",
            Q1MoverState::Top => "top",
            Q1MoverState::Down => "down",
        }
    }

    /// Parse donor state text.
    pub fn parse(text: &str) -> Result<Self, Q1Error> {
        match text {
            "bottom" => Ok(Q1MoverState::Bottom),
            "up" => Ok(Q1MoverState::Up),
            "top" => Ok(Q1MoverState::Top),
            "down" => Ok(Q1MoverState::Down),
            _ => Err(Q1Error::Message(format!("Unknown Q1 mover state: {text}"))),
        }
    }
}

/// Combat attack state (`Q1Actor["attackState"]`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Q1AttackState {
    /// No attack.
    #[default]
    Straight,
    /// Melee attack.
    Melee,
    /// Missile attack.
    Missile,
    /// Dodging.
    Dodging,
}

impl Q1AttackState {
    /// Donor attack-state text.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Q1AttackState::Straight => "straight",
            Q1AttackState::Melee => "melee",
            Q1AttackState::Missile => "missile",
            Q1AttackState::Dodging => "dodging",
        }
    }

    /// Parse donor attack-state text.
    pub fn parse(text: &str) -> Result<Self, Q1Error> {
        match text {
            "straight" => Ok(Q1AttackState::Straight),
            "melee" => Ok(Q1AttackState::Melee),
            "missile" => Ok(Q1AttackState::Missile),
            "dodging" => Ok(Q1AttackState::Dodging),
            _ => Err(Q1Error::Message(format!("Unknown Q1 attack state: {text}"))),
        }
    }
}

/// Source projectile kind (`Q1Actor["projectile"]`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Q1ProjectileKind {
    /// Rocket.
    Rocket,
    /// Grenade.
    Grenade,
    /// Nail spike.
    Spike,
    /// Super nail spike.
    Superspike,
}

impl Q1ProjectileKind {
    /// Donor projectile text.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Q1ProjectileKind::Rocket => "rocket",
            Q1ProjectileKind::Grenade => "grenade",
            Q1ProjectileKind::Spike => "spike",
            Q1ProjectileKind::Superspike => "superspike",
        }
    }

    /// Parse donor projectile text.
    pub fn parse(text: &str) -> Result<Self, Q1Error> {
        match text {
            "rocket" => Ok(Q1ProjectileKind::Rocket),
            "grenade" => Ok(Q1ProjectileKind::Grenade),
            "spike" => Ok(Q1ProjectileKind::Spike),
            "superspike" => Ok(Q1ProjectileKind::Superspike),
            _ => Err(Q1Error::Message(format!("Unknown Q1 projectile: {text}"))),
        }
    }
}

/// Q1 source entity record (`Q1Actor`).
#[derive(Debug, Clone, PartialEq)]
pub struct Q1Actor {
    /// Owning actor handle.
    pub actor: OwnedActor,
    /// Entity classname.
    pub classname: String,
    /// Source entity ordinal.
    pub source_ordinal: Option<i32>,
    /// Model path.
    pub model: String,
    /// Model frame.
    pub frame: i32,
    /// Model skin.
    pub skin: i32,
    /// Effect flags.
    pub effects: i32,
    /// Solidity.
    pub solid: Q1Solid,
    /// Movement type.
    pub movement: Q1MoveType,
    /// Entity string fields.
    pub fields: HashMap<String, String>,
    /// Named actor references.
    pub references: HashMap<String, Option<ActorId>>,
    /// Trigger target.
    pub target: String,
    /// Target name.
    pub targetname: String,
    /// Kill target.
    pub killtarget: String,
    /// Center-print message.
    pub message: String,
    /// Trigger delay in seconds.
    pub delay: f64,
    /// Spawn flags.
    pub spawnflags: i32,
    /// Sound set selector.
    pub sounds: i32,
    /// Wait interval in seconds.
    pub wait: f64,
    /// Movement speed.
    pub speed: f64,
    /// Damage value.
    pub damage: f64,
    /// Maximum health.
    pub max_health: f64,
    /// Whether the entity takes aimed damage.
    pub aimed_damage: bool,
    /// Next think time in seconds (`-1` when idle).
    pub next_think: f64,
    /// Scheduled think action name.
    pub think: Option<String>,
    /// Use callback name.
    pub use_callback: Option<String>,
    /// Touch callback name.
    pub touch: Option<String>,
    /// Pain callback name.
    pub pain: Option<String>,
    /// Death callback name.
    pub die: Option<String>,
    /// Blocked callback name.
    pub blocked: Option<String>,
    /// Pending mover completion.
    pub move_completion: Option<Q1Move>,
    /// Owning actor, if any.
    pub owner: Option<ActorId>,
    /// Activating actor, if any.
    pub activator: Option<ActorId>,
    /// Original model path.
    pub original_model: String,
    /// Saved position 1.
    pub pos1: Vec3,
    /// Saved position 2.
    pub pos2: Vec3,
    /// Saved destination 1.
    pub dest1: Vec3,
    /// Saved destination 2.
    pub dest2: Vec3,
    /// Move direction.
    pub movedir: Vec3,
    /// Saved mangle angles.
    pub mangle: Vec3,
    /// Mover position state.
    pub state: Q1MoverState,
    /// Linked door group.
    pub door_group: Vec<ActorId>,
    /// Trigger bounds override.
    pub trigger_bounds: Option<Bounds>,
    /// Next allowed attack time in seconds.
    pub attack_finished: f64,
    /// Counter value.
    pub count: f64,
    /// Whether the entity activated.
    pub activated: bool,
    /// Monster state, if any.
    pub monster: Option<Q1Monster>,
    /// Projectile kind, if any.
    pub projectile: Option<Q1ProjectileKind>,
    /// Projectile weapon, if any.
    pub projectile_weapon: Option<Q1Weapon>,
    /// Angular velocity.
    pub angular_velocity: Vec3,
    /// Water depth level.
    pub water_level: i32,
    /// Water contents type.
    pub water_type: i32,
    /// Movement flags.
    pub movement_flags: i32,
    /// Ideal yaw.
    pub ideal_yaw: f64,
    /// Yaw speed.
    pub yaw_speed: f64,
    /// Combat attack state.
    pub attack_state: Q1AttackState,
    /// Path-end action name.
    pub path_end: Option<String>,
}

impl Q1Actor {
    /// Admit a source entity record, decoding entity strings once
    /// (`ED_NewString` semantics live in
    /// [`q1_entity_string`](super::text::q1_entity_string)).
    pub fn new(actor: OwnedActor, classname: &str, source_ordinal: Option<i32>, source: Option<&Q1Entity>) -> Self {
        let mut entity = Self {
            actor,
            classname: classname.to_string(),
            source_ordinal,
            model: String::new(),
            frame: 0,
            skin: 0,
            effects: 0,
            solid: Q1Solid::None,
            movement: Q1MoveType::None,
            fields: HashMap::new(),
            references: HashMap::new(),
            target: String::new(),
            targetname: String::new(),
            killtarget: String::new(),
            message: String::new(),
            delay: 0.0,
            spawnflags: 0,
            sounds: 0,
            wait: 0.0,
            speed: 0.0,
            damage: 0.0,
            max_health: 0.0,
            aimed_damage: false,
            next_think: -1.0,
            think: None,
            use_callback: None,
            touch: None,
            pain: None,
            die: None,
            blocked: None,
            move_completion: None,
            owner: None,
            activator: None,
            original_model: String::new(),
            pos1: ZERO,
            pos2: ZERO,
            dest1: ZERO,
            dest2: ZERO,
            movedir: ZERO,
            mangle: ZERO,
            state: Q1MoverState::Bottom,
            door_group: Vec::new(),
            trigger_bounds: None,
            attack_finished: 0.0,
            count: 0.0,
            activated: false,
            monster: None,
            projectile: None,
            projectile_weapon: None,
            angular_velocity: ZERO,
            water_level: 0,
            water_type: 0,
            movement_flags: 0,
            ideal_yaw: 0.0,
            yaw_speed: 20.0,
            attack_state: Q1AttackState::Straight,
            path_end: None,
        };
        if let Some(source) = source {
            for (key, value) in &source.properties {
                entity.fields.insert(key.clone(), q1_entity_string(value));
            }
        }
        entity.model = entity.text("model");
        entity.original_model = entity.model.clone();
        entity.target = entity.text("target");
        entity.targetname = entity.text("targetname");
        entity.killtarget = entity.text("killtarget");
        entity.message = entity.text("message");
        entity.delay = entity.number("delay");
        entity.spawnflags = entity.number("spawnflags") as i32;
        entity.sounds = entity.number("sounds") as i32;
        entity.wait = entity.number("wait");
        entity.speed = entity.number("speed");
        entity.damage = entity.number("dmg");
        entity.max_health = entity.number("health");
        entity.count = entity.number("count");
        entity
    }

    /// Read a string field (`""` when absent).
    #[must_use]
    pub fn text(&self, key: &str) -> String {
        self.fields.get(key).cloned().unwrap_or_default()
    }

    /// Read a binary32 numeric field (`fallback` when absent or not
    /// finite).
    #[must_use]
    pub fn number(&self, key: &str) -> f64 {
        self.number_or(key, 0.0)
    }

    /// Read a binary32 numeric field with an explicit fallback for
    /// absent or non-finite values.
    #[must_use]
    pub fn number_or(&self, key: &str, fallback: f64) -> f64 {
        let text = self.text(key);
        if text.is_empty() {
            return fallback;
        }
        match text.parse::<f64>() {
            Ok(value) if value.is_finite() => f64::from(value as f32),
            _ => fallback,
        }
    }

    /// Read a numeric field with a fallback applied to zero as well as
    /// absent values (donor `entity.number(key) || fallback`).
    #[must_use]
    pub fn number_fallback(&self, key: &str, fallback: f64) -> f64 {
        let value = self.number(key);
        if value == 0.0 {
            fallback
        } else {
            value
        }
    }

    /// Read a vector field.
    #[must_use]
    pub fn vector(&self, key: &str) -> Vec3 {
        parse_vector(&self.text(key))
    }
}

/// Parse a source vector (`parseVector`). Non-finite components become
/// zero; missing components default to zero.
#[must_use]
pub fn parse_vector(text: &str) -> Vec3 {
    let mut numbers = text
        .split_whitespace()
        .map(|part| part.parse::<f64>().unwrap_or(f64::NAN));
    let convert = |value: Option<f64>| -> f32 {
        match value {
            Some(value) if value.is_finite() => value as f32,
            _ => 0.0,
        }
    };
    Vec3 {
        x: convert(numbers.next()),
        y: convert(numbers.next()),
        z: convert(numbers.next()),
    }
}

/// Spawn angles for a map entity (`sourceAngles`).
#[must_use]
pub fn source_angles(source: &Q1Entity) -> Vec3 {
    if let Some(angles) = q1_entity_value(source, "angles") {
        return parse_vector(angles);
    }
    let angle = q1_entity_value(source, "angle")
        .and_then(|text| text.parse::<f64>().ok())
        .unwrap_or(0.0);
    Vec3 {
        x: 0.0,
        y: angle as f32,
        z: 0.0,
    }
}

/// Move direction for spawn angles, honoring the vertical shortcuts
/// (`moveDirection`). The services argument selects the saved QC basis,
/// exactly like the donor.
pub fn move_direction(angles: Vec3, game: Option<&mut Q1EntityServices>) -> Vec3 {
    if angles.y == -1.0 && angles.x == 0.0 && angles.z == 0.0 {
        return Vec3 { x: 0.0, y: 0.0, z: 1.0 };
    }
    if angles.y == -2.0 && angles.x == 0.0 && angles.z == 0.0 {
        return Vec3 {
            x: 0.0,
            y: 0.0,
            z: -1.0,
        };
    }
    match game {
        Some(game) => game.make_vectors(angles).forward,
        None => vectors(angles).forward,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entity_fields_decode_once() {
        let source = Q1Entity {
            properties: vec![
                ("classname".to_string(), "light".to_string()),
                ("message".to_string(), "line\\none".to_string()),
                ("wait".to_string(), "2.5".to_string()),
                ("spawnflags".to_string(), "3".to_string()),
            ],
        };
        let owner = qa_core::identity::IdentityOwner::create("test").expect("owner");
        let id = owner.actor(1, 0);
        let owned = owner
            .owned_actor(&id, qa_core::identity::ProviderId::new("q1", "official"))
            .expect("owned");
        let entity = Q1Actor::new(owned, "light", Some(4), Some(&source));
        assert_eq!(entity.message, "line\none");
        assert_eq!(entity.wait, 2.5);
        assert_eq!(entity.spawnflags, 3);
        assert_eq!(entity.number("missing"), 0.0);
        assert_eq!(entity.number_fallback("missing", 8.0), 8.0);
    }

    #[test]
    fn vector_parsing_tolerates_garbage() {
        assert_eq!(parse_vector("1 2 3"), Vec3 { x: 1.0, y: 2.0, z: 3.0 });
        assert_eq!(parse_vector("1 bad"), Vec3 { x: 1.0, y: 0.0, z: 0.0 });
        assert_eq!(
            move_direction(
                Vec3 {
                    x: 0.0,
                    y: -1.0,
                    z: 0.0
                },
                None
            ),
            Vec3 { x: 0.0, y: 0.0, z: 1.0 }
        );
    }
}
