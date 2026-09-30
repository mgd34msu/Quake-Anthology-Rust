//! Quake III team-arena runtime support.
//!
//! Donor provenance: the team-arena port (`src/content/q3/team-arena`) plus
//! the sibling donors named on each item (`src/content/q3/base/shared`,
//! `src/content/q3/base/game`, `src/core`). This module owns the runtime the
//! team rules execute against: entity/client records, the entity pool, engine
//! host contracts, and the small value, codec, slot, and random helpers those
//! records use. Leaf value types (products, codes, trajectories, commands,
//! formats, spawn variables, session records) come from their canonical ports.

use qa_core::cmd::ascii_fold;
use qa_core::identity::{ActorId, IdentityOwner};
use qa_core::math::{vec3, Bounds, Vec3};
use qa_core::numeric::{q_rand, qvm_float_to_int};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use crate::q3::base::game::format::{game_format_bounded, GameFormatArgument, BIG_BUFFER_BYTES};
use crate::q3::base::game::spawn::SpawnVariables;
use crate::q3::base::game::state::{ClientSession, ConnectionState, MAX_CLIENTS, MAX_GENTITIES};
use crate::q3::base::shared::definitions::{
    stat_schema, weapon_count, EntityType, MoveType, Product, StatSchema, Weapon, WeaponState, EV_EVENT_BIT1,
    EV_EVENT_BITS, GIB_HEALTH,
};
use crate::q3::base::shared::player_state::{UserCommand, ENTITYNUM_NONE, ENTITYNUM_WORLD};
use crate::q3::base::shared::trajectory::{Trajectory, TrajectoryType};

/// Damage flag bits (`DamageFlags`).
pub mod damage_flags {
    pub const RADIUS: i32 = 0x1;
    pub const NO_ARMOR: i32 = 0x2;
    pub const NO_KNOCKBACK: i32 = 0x4;
    pub const NO_PROTECTION: i32 = 0x8;
    pub const NO_TEAM_PROTECTION: i32 = 0x10;
}

/// Fixed source arrays with checked access (`PlayerStateSlots`).
#[derive(Debug, Clone)]
pub struct SlotArray {
    values: RefCell<Vec<i32>>,
}

impl SlotArray {
    /// Zeroed slots of the given length.
    pub fn new(length: usize) -> Self {
        Self {
            values: RefCell::new(vec![0; length]),
        }
    }

    /// Slot count.
    #[must_use]
    pub fn len(&self) -> usize {
        self.values.borrow().len()
    }

    /// Whether there are no slots.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.values.borrow().is_empty()
    }

    /// Checked read.
    #[must_use]
    pub fn get(&self, index: usize) -> i32 {
        let values = self.values.borrow();
        match values.get(index) {
            Some(value) => *value,
            None => panic!("Player state slot {index} outside {}", values.len()),
        }
    }

    /// Checked write.
    pub fn set(&self, index: usize, value: i32) {
        let mut values = self.values.borrow_mut();
        if index >= values.len() {
            panic!("Player state slot {index} outside {}", values.len());
        }
        values[index] = value;
    }

    /// Snapshot copy (`copy`).
    #[must_use]
    pub fn copy(&self) -> Vec<i32> {
        self.values.borrow().clone()
    }
}

/// Team scores and other stores shared across modules by reference.
pub type SharedSlots = Rc<SlotArray>;

/// Instance-owned `bg_lib` rand/srand and game random (`GameRandom`).
#[derive(Debug)]
pub struct GameRandom {
    seed: Cell<i32>,
}

impl GameRandom {
    /// Fresh generator.
    pub fn new(seed: i32) -> Self {
        Self { seed: Cell::new(seed) }
    }

    /// Current seed.
    #[must_use]
    pub fn seed(&self) -> i32 {
        self.seed.get()
    }

    /// Reset the seed.
    pub fn reset(&self, seed: i32) {
        self.seed.set(seed);
    }

    /// Low-15-bit game rand.
    pub fn rand(&self) -> i32 {
        let next = q_rand(self.seed.get());
        self.seed.set(next);
        next & 0x7fff
    }

    /// `[0, 1]` game random.
    pub fn random(&self) -> f32 {
        self.rand() as f32 / 0x7fff as f32
    }

    /// `[-1, 1]` game random.
    pub fn crandom(&self) -> f32 {
        2.0 * (self.random() - 0.5)
    }
}

/// Checkpoint value (`unknown` JSON-ish save payloads).
#[derive(Debug, Clone, PartialEq)]
pub enum SaveValue {
    /// Null.
    Null,
    /// Boolean.
    Bool(bool),
    /// Integer.
    Int(i64),
    /// String.
    Str(String),
    /// Array.
    List(Vec<SaveValue>),
    /// Record.
    Map(HashMap<String, SaveValue>),
}

impl SaveValue {
    /// Build a record from pairs.
    pub fn map(pairs: Vec<(&str, SaveValue)>) -> Self {
        SaveValue::Map(pairs.into_iter().map(|(key, value)| (key.to_string(), value)).collect())
    }
}

/// Checkpoint reader (`SaveReader`).
#[derive(Debug, Clone)]
pub struct SaveReader<'a> {
    value: &'a SaveValue,
    path: String,
}

impl<'a> SaveReader<'a> {
    /// Root reader.
    pub fn new(value: &'a SaveValue, path: &str) -> Self {
        Self {
            value,
            path: path.to_string(),
        }
    }

    /// Field reader.
    #[must_use]
    pub fn field(&self, name: &str) -> SaveReader<'a> {
        match self.value {
            SaveValue::Map(map) => match map.get(name) {
                Some(value) => SaveReader {
                    value,
                    path: format!("{}.{}", self.path, name),
                },
                None => self.fail("expected a record"),
            },
            _ => self.fail("expected a record"),
        }
    }

    /// String value.
    #[must_use]
    pub fn string(&self) -> String {
        match self.value {
            SaveValue::Str(value) => value.clone(),
            _ => self.fail("expected a string"),
        }
    }

    /// Boolean value.
    #[must_use]
    pub fn boolean(&self) -> bool {
        match self.value {
            SaveValue::Bool(value) => *value,
            _ => self.fail("expected a boolean"),
        }
    }

    /// Integer value with a minimum.
    #[must_use]
    pub fn integer(&self, minimum: i64) -> i64 {
        match self.value {
            SaveValue::Int(value) if *value >= minimum => *value,
            _ => self.fail("expected an integer in range"),
        }
    }

    /// Value restricted to the given choices.
    #[must_use]
    pub fn choice(&self, choices: &[i64]) -> i64 {
        match self.value {
            SaveValue::Int(value) if choices.contains(value) => *value,
            _ => self.fail("expected a listed choice"),
        }
    }

    /// Array value.
    pub fn list<T>(&self, read: impl Fn(&SaveReader) -> T) -> Vec<T> {
        match self.value {
            SaveValue::List(entries) => entries
                .iter()
                .enumerate()
                .map(|(index, entry)| {
                    read(&SaveReader {
                        value: entry,
                        path: format!("{}[{index}]", self.path),
                    })
                })
                .collect(),
            _ => self.fail("expected an array"),
        }
    }

    /// Nullable value.
    pub fn nullable<T>(&self, read: impl Fn(&SaveReader) -> T) -> Option<T> {
        match self.value {
            SaveValue::Null => None,
            _ => Some(read(self)),
        }
    }

    /// Fail with a save-format error.
    pub fn fail(&self, message: &str) -> ! {
        panic!("save format error at {}: {message}", self.path);
    }
}

/// Item definition subset used by team rules (`ItemDefinition`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ItemDefinition {
    /// Item tag (powerup/weapon code).
    pub tag: i32,
    /// Entity class name.
    pub class_name: String,
    /// Pickup display name.
    pub pickup_name: Option<String>,
}

/// Set a `key\value` info pair, Q3 dialect (`setInfoValue`).
pub fn set_info_value(
    input: &str,
    key_input: &str,
    value_input: &str,
    maximum_length: usize,
    print: &dyn Fn(&str),
) -> String {
    let info = input.to_string();
    let key = key_input.to_string();
    let value = value_input.to_string();
    if info.len() >= maximum_length {
        panic!("Info_SetValueForKey: oversize infostring");
    }
    if key.contains('\\') || value.contains('\\') {
        print("Can't use keys or values with a \\\n");
        return info;
    }
    if key.contains(';') || value.contains(';') {
        print("Can't use keys or values with a semicolon\n");
        return info;
    }
    if key.contains('"') || value.contains('"') {
        print("Can't use keys or values with a \"\n");
        return info;
    }
    let mut result = info.clone();
    let mut cursor = 0;
    while cursor < info.len() {
        let start = cursor;
        if info.as_bytes().get(cursor) == Some(&b'\\') {
            cursor += 1;
        }
        let rest = &info[cursor..];
        let Some(separator) = rest.find('\\') else { break };
        let separator = cursor + separator;
        let after = &info[separator + 1..];
        let end = match after.find('\\') {
            Some(next) => separator + 1 + next,
            None => info.len(),
        };
        if info[cursor..separator] == key {
            result = format!("{}{}", &info[..start], &info[end..]);
            break;
        }
        cursor = end;
    }
    if value.is_empty() {
        return result;
    }
    let mut pair = format!("\\{key}\\{value}");
    if pair.len() >= maximum_length {
        pair = pair.chars().take(maximum_length - 1).collect();
    }
    if pair.len() + result.len() > maximum_length {
        print("Info string length exceeded\n");
        return result;
    }
    format!("{result}{pair}")
}

/// Shared entity handle (aliases its pool slot).
pub type EntityRef = Rc<RefCell<GameEntity>>;

/// Shared client handle (aliases its entity record).
pub type ClientRef = Rc<RefCell<GameClient>>;

/// Damage participant: an entity or a foreign shared actor
/// (`DamageParticipant`).
#[derive(Clone)]
pub enum DamageParticipant {
    /// Pool entity.
    Entity(EntityRef),
    /// Foreign shared actor.
    SharedActor(ActorId),
}

impl DamageParticipant {
    /// Borrow as an entity when applicable.
    #[must_use]
    pub fn as_entity(&self) -> Option<&EntityRef> {
        match self {
            DamageParticipant::Entity(entity) => Some(entity),
            DamageParticipant::SharedActor(_) => None,
        }
    }
}

/// Touch contact placeholder (unused by team rules).
#[derive(Debug, Clone, Copy, Default)]
pub struct TouchContact;

/// Think callback (`EntityThink`).
pub type ThinkCallback = Rc<dyn Fn(&EntityRef)>;

/// Pain callback (`EntityPain`).
pub type PainCallback = Rc<dyn Fn(&EntityRef, &DamageParticipant, f32)>;

/// Touch callback (`EntityTouch`).
pub type TouchCallback = Rc<dyn Fn(&EntityRef, &DamageParticipant, &TouchContact)>;

/// Death callback (`EntityDie`).
pub type DieCallback = Rc<dyn Fn(&EntityRef, &DamageParticipant, &DamageParticipant, i32, i32)>;

/// Entity state (`EntityState`; fields touched by team rules).
#[derive(Debug, Clone, PartialEq)]
pub struct EntityState {
    /// Entity number.
    pub number: i32,
    /// Entity type code.
    pub e_type: i32,
    /// Entity flag bits.
    pub e_flags: i32,
    /// Position trajectory.
    pub pos: Trajectory,
    /// Angle trajectory.
    pub apos: Trajectory,
    /// Current origin.
    pub origin: Vec3,
    /// Current angles.
    pub angles: Vec3,
    /// Secondary angles.
    pub angles2: Vec3,
    /// Other entity number.
    pub other_entity_num: i32,
    /// Ground entity number.
    pub ground_entity_num: i32,
    /// Looping sound index.
    pub loop_sound: i32,
    /// Model index.
    pub modelindex: i32,
    /// Secondary model index.
    pub modelindex2: i32,
    /// Client number.
    pub client_num: i32,
    /// Animation frame.
    pub frame: i32,
    /// Current event.
    pub event: i32,
    /// Event parameter.
    pub event_parm: i32,
    /// Active powerup bits.
    pub powerups: i32,
    /// Current weapon.
    pub weapon: i32,
    /// Legs animation.
    pub legs_anim: i32,
    /// Torso animation.
    pub torso_anim: i32,
    /// Generic counter.
    pub generic1: i32,
}

impl Default for EntityState {
    fn default() -> Self {
        Self {
            number: 0,
            e_type: EntityType::EtGeneral as i32,
            e_flags: 0,
            pos: Trajectory::zero(TrajectoryType::TrStationary),
            apos: Trajectory::zero(TrajectoryType::TrStationary),
            origin: vec3(0.0, 0.0, 0.0),
            angles: vec3(0.0, 0.0, 0.0),
            angles2: vec3(0.0, 0.0, 0.0),
            other_entity_num: 0,
            ground_entity_num: 0,
            loop_sound: 0,
            modelindex: 0,
            modelindex2: 0,
            client_num: 0,
            frame: 0,
            event: 0,
            event_parm: 0,
            powerups: 0,
            weapon: Weapon::WpNone as i32,
            legs_anim: 0,
            torso_anim: 0,
            generic1: 0,
        }
    }
}

/// Collision model selector (`EntityCollisionModel`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CollisionModel {
    /// Box hull.
    Box,
    /// Capsule hull.
    Capsule,
}

/// Server-side shared entity data (`EntityShared`).
#[derive(Debug, Clone)]
pub struct EntityShared {
    /// Server flags.
    pub sv_flags: i32,
    /// Single-client target.
    pub single_client: i32,
    /// Collision model.
    pub model: CollisionModel,
    /// Contents mask.
    pub contents: i32,
    /// Owner number.
    pub owner_num: i32,
    /// Collision minimums.
    pub mins: Vec3,
    /// Collision maximums.
    pub maxs: Vec3,
    /// Current collision origin.
    pub stored_origin: Vec3,
    origin_view: Rc<RefCell<Option<Vec3>>>,
}

impl EntityShared {
    /// Fresh shared record.
    pub fn new() -> Self {
        Self {
            sv_flags: 0,
            single_client: 0,
            model: CollisionModel::Box,
            contents: 0,
            owner_num: ENTITYNUM_NONE,
            mins: vec3(0.0, 0.0, 0.0),
            maxs: vec3(0.0, 0.0, 0.0),
            stored_origin: vec3(0.0, 0.0, 0.0),
            origin_view: Rc::new(RefCell::new(None)),
        }
    }

    /// Current origin, honoring a temporary origin view.
    #[must_use]
    pub fn current_origin(&self) -> Vec3 {
        self.origin_view.borrow().unwrap_or(self.stored_origin)
    }

    /// Store the current origin.
    pub fn set_current_origin(&mut self, origin: Vec3) {
        if self.origin_view.borrow().is_some() {
            *self.origin_view.borrow_mut() = Some(origin);
        } else {
            self.stored_origin = origin;
        }
    }
}

impl Default for EntityShared {
    fn default() -> Self {
        Self::new()
    }
}

/// Run a closure with a temporary current origin (`withCurrentOrigin`).
pub fn with_entity_origin(entity: &EntityRef, origin: Vec3, run: impl FnOnce()) {
    let view = entity.borrow().r.origin_view.clone();
    *view.borrow_mut() = Some(origin);
    run();
    *view.borrow_mut() = None;
}

/// Classname storage, including client-name borrowing (`GameEntity`).
#[derive(Clone)]
pub enum Classname {
    /// Plain value.
    Value(Option<String>),
    /// Borrowed from a client's netname.
    ClientName(ClientRef),
}

/// Game entity (`GameEntity`; fields touched by team rules).
#[derive(Clone)]
pub struct GameEntity {
    /// Pool slot.
    pub slot: usize,
    /// Actor handle.
    pub actor: ActorId,
    /// Whether the slot is live.
    pub inuse: bool,
    /// Never recycled by `free`.
    pub never_free: bool,
    /// Class name.
    pub classname: Classname,
    /// Entity state.
    pub s: EntityState,
    /// Shared collision data.
    pub r: EntityShared,
    /// Owning client record.
    pub client: Option<ClientRef>,
    /// Health.
    pub health: i32,
    /// Takes damage.
    pub takedamage: bool,
    /// Scratch damage accumulator.
    pub damage: i32,
    /// Entity flag bits.
    pub flags: i32,
    /// Physics object.
    pub physics_object: bool,
    /// Physics bounce fraction.
    pub physics_bounce: i32,
    /// Timestamp.
    pub timestamp: i32,
    /// Next think time.
    pub nextthink: i32,
    /// Think callback.
    pub think: Option<ThinkCallback>,
    /// Touch callback.
    pub touch: Option<TouchCallback>,
    /// Pain callback.
    pub pain: Option<PainCallback>,
    /// Death callback.
    pub die: Option<DieCallback>,
    /// Pain debounce time.
    pub pain_debounce_time: i32,
    /// Last event time.
    pub event_time: i32,
    /// Free after the event expires.
    pub free_after_event: bool,
    /// Last free time.
    pub freetime: i32,
    /// Spawn flags.
    pub spawnflags: i32,
    /// Collision mask.
    pub clipmask: i32,
    /// Water type.
    pub watertype: i32,
    /// Water level.
    pub waterlevel: i32,
    /// Generic counter.
    pub count: i32,
    /// Location message.
    pub message: Option<String>,
    /// Target name reference.
    pub target: Option<String>,
    /// Own targetname.
    pub targetname: Option<String>,
    /// Current enemy.
    pub enemy: Option<EntityRef>,
    /// Activator entity.
    pub activator: Option<EntityRef>,
    /// Next entity in the location chain.
    pub next_train: Option<EntityRef>,
    /// Carried item definition.
    pub item: Option<ItemDefinition>,
    /// Ground actor.
    pub ground: Option<ActorId>,
}

impl GameEntity {
    /// Fresh entity for a slot.
    pub fn new(slot: usize, actor: ActorId) -> Self {
        Self {
            slot,
            actor,
            inuse: false,
            never_free: false,
            classname: Classname::Value(None),
            s: EntityState::default(),
            r: EntityShared::new(),
            client: None,
            health: 0,
            takedamage: false,
            damage: 0,
            flags: 0,
            physics_object: false,
            physics_bounce: 0,
            timestamp: 0,
            nextthink: 0,
            think: None,
            touch: None,
            pain: None,
            die: None,
            pain_debounce_time: 0,
            event_time: 0,
            free_after_event: false,
            freetime: 0,
            spawnflags: 0,
            clipmask: 0,
            watertype: 0,
            waterlevel: 0,
            count: 0,
            message: None,
            target: None,
            targetname: None,
            enemy: None,
            activator: None,
            next_train: None,
            item: None,
            ground: None,
        }
    }

    /// Read the class name.
    #[must_use]
    pub fn classname(&self) -> Option<String> {
        match &self.classname {
            Classname::Value(value) => value.clone(),
            Classname::ClientName(client) => Some(client.borrow().pers.netname.clone()),
        }
    }

    /// Write the class name.
    pub fn set_classname(&mut self, value: Option<String>) {
        self.classname = Classname::Value(value);
    }

    /// Borrow the class name from a client record.
    pub fn bind_client_name(&mut self, client: &ClientRef) {
        self.classname = Classname::ClientName(client.clone());
    }
}

/// Authority-copy mode for `copy_from` (no bindings exist in this mirror,
/// so both modes copy every field).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthorityCopy {
    /// Keep authority-owned stores.
    PreserveAuthority,
    /// Replace authority-owned stores.
    ReplaceAuthority,
}

/// Player state (`PlayerState`).
#[derive(Debug, Clone)]
pub struct PlayerState {
    /// Product.
    pub product: Product,
    /// Last command time.
    pub command_time: i32,
    /// Movement type code.
    pub pm_type: i32,
    /// Bob cycle.
    pub bob_cycle: i32,
    /// Movement flag bits.
    pub pm_flags: i32,
    /// Movement timer.
    pub pm_time: i32,
    /// Origin.
    pub origin: Vec3,
    /// Velocity.
    pub velocity: Vec3,
    /// Weapon time.
    pub weapon_time: i32,
    /// Gravity.
    pub gravity: i32,
    /// Speed.
    pub speed: i32,
    /// Delta angles.
    pub delta_angles: Vec3,
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
    /// Entity flag bits.
    pub e_flags: i32,
    /// Predictable event sequence.
    pub event_sequence: i32,
    /// Predictable event ring.
    pub events: SlotArray,
    /// Predictable event parameters.
    pub event_parms: SlotArray,
    /// External event.
    pub external_event: i32,
    /// External event parameter.
    pub external_event_parm: i32,
    /// External event time.
    pub external_event_time: i32,
    /// Client number.
    pub client_num: i32,
    /// Current weapon.
    pub weapon: i32,
    /// Weapon state.
    pub weapon_state: i32,
    /// View angles.
    pub viewangles: Vec3,
    /// View height.
    pub viewheight: i32,
    /// Damage event counter.
    pub damage_event: i32,
    /// Damage yaw.
    pub damage_yaw: i32,
    /// Damage pitch.
    pub damage_pitch: i32,
    /// Damage count.
    pub damage_count: i32,
    /// Stat slots.
    pub stats: SlotArray,
    /// Persistent slots.
    pub persistant: SlotArray,
    /// Powerup slots.
    pub powerups: SlotArray,
    /// Ammo slots.
    pub ammo: SlotArray,
    /// Generic counter.
    pub generic1: i32,
    /// Looping sound.
    pub loop_sound: i32,
    /// Jump-pad entity.
    pub jumppad_ent: i32,
    /// Ping.
    pub ping: i32,
    /// Pmove frame count.
    pub pmove_framecount: i32,
    /// Jump-pad frame.
    pub jumppad_frame: i32,
    /// Consumed event sequence.
    pub entity_event_sequence: i32,
}

impl PlayerState {
    /// Source-zero player state.
    pub fn new(product: Product) -> Self {
        Self {
            product,
            command_time: 0,
            pm_type: MoveType::PmNormal as i32,
            bob_cycle: 0,
            pm_flags: 0,
            pm_time: 0,
            origin: vec3(0.0, 0.0, 0.0),
            velocity: vec3(0.0, 0.0, 0.0),
            weapon_time: 0,
            gravity: 0,
            speed: 0,
            delta_angles: vec3(0.0, 0.0, 0.0),
            ground_entity_num: 0,
            legs_timer: 0,
            legs_anim: 0,
            torso_timer: 0,
            torso_anim: 0,
            movement_dir: 0,
            grapple_point: vec3(0.0, 0.0, 0.0),
            e_flags: 0,
            event_sequence: 0,
            events: SlotArray::new(2),
            event_parms: SlotArray::new(2),
            external_event: 0,
            external_event_parm: 0,
            external_event_time: 0,
            client_num: 0,
            weapon: Weapon::WpNone as i32,
            weapon_state: WeaponState::WeaponReady as i32,
            viewangles: vec3(0.0, 0.0, 0.0),
            viewheight: 0,
            damage_event: 0,
            damage_yaw: 0,
            damage_pitch: 0,
            damage_count: 0,
            stats: SlotArray::new(16),
            persistant: SlotArray::new(16),
            powerups: SlotArray::new(16),
            ammo: SlotArray::new(16),
            generic1: 0,
            loop_sound: 0,
            jumppad_ent: 0,
            ping: 0,
            pmove_framecount: 0,
            jumppad_frame: 0,
            entity_event_sequence: 0,
        }
    }

    /// Health (`stats[STAT_HEALTH]`).
    #[must_use]
    pub fn health(&self) -> i32 {
        let slot = match stat_schema(self.product) {
            StatSchema::Base(layout) => layout.health,
            StatSchema::Missionpack(layout) => layout.health,
        };
        self.stats.get(slot as usize)
    }

    /// Write health.
    pub fn set_health(&mut self, value: i32) {
        let slot = match stat_schema(self.product) {
            StatSchema::Base(layout) => layout.health,
            StatSchema::Missionpack(layout) => layout.health,
        };
        self.stats.set(slot as usize, value);
    }

    /// Full copy (`copyFrom`).
    pub fn copy_from(&mut self, source: &PlayerState, _mode: AuthorityCopy) {
        self.product = source.product;
        self.command_time = source.command_time;
        self.pm_type = source.pm_type;
        self.bob_cycle = source.bob_cycle;
        self.pm_flags = source.pm_flags;
        self.pm_time = source.pm_time;
        self.origin = source.origin;
        self.velocity = source.velocity;
        self.weapon_time = source.weapon_time;
        self.gravity = source.gravity;
        self.speed = source.speed;
        self.delta_angles = source.delta_angles;
        self.ground_entity_num = source.ground_entity_num;
        self.legs_timer = source.legs_timer;
        self.legs_anim = source.legs_anim;
        self.torso_timer = source.torso_timer;
        self.torso_anim = source.torso_anim;
        self.movement_dir = source.movement_dir;
        self.grapple_point = source.grapple_point;
        self.e_flags = source.e_flags;
        self.event_sequence = source.event_sequence;
        copy_slots(&self.events, &source.events);
        copy_slots(&self.event_parms, &source.event_parms);
        self.external_event = source.external_event;
        self.external_event_parm = source.external_event_parm;
        self.external_event_time = source.external_event_time;
        self.client_num = source.client_num;
        self.weapon = source.weapon;
        self.weapon_state = source.weapon_state;
        self.viewangles = source.viewangles;
        self.viewheight = source.viewheight;
        self.damage_event = source.damage_event;
        self.damage_yaw = source.damage_yaw;
        self.damage_pitch = source.damage_pitch;
        self.damage_count = source.damage_count;
        copy_slots(&self.stats, &source.stats);
        copy_slots(&self.persistant, &source.persistant);
        copy_slots(&self.powerups, &source.powerups);
        copy_slots(&self.ammo, &source.ammo);
        self.generic1 = source.generic1;
        self.loop_sound = source.loop_sound;
        self.jumppad_ent = source.jumppad_ent;
        self.ping = source.ping;
        self.pmove_framecount = source.pmove_framecount;
        self.jumppad_frame = source.jumppad_frame;
        self.entity_event_sequence = source.entity_event_sequence;
    }
}

fn copy_slots(target: &SlotArray, source: &SlotArray) {
    for index in 0..target.len() {
        target.set(index, source.get(index));
    }
}

/// Persistent team state (`PlayerTeamState`).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct PlayerTeamState {
    /// Team state code.
    pub state: i32,
    /// Location marker.
    pub location: i32,
    /// Captures.
    pub captures: i32,
    /// Base defenses.
    pub base_defense: i32,
    /// Carrier defenses.
    pub carrier_defense: i32,
    /// Flag recoveries.
    pub flag_recovery: i32,
    /// Carrier frags.
    pub frag_carrier: i32,
    /// Assists.
    pub assists: i32,
    /// Last time this client hurt a carrier.
    pub last_hurt_carrier: f32,
    /// Last flag-return time.
    pub last_returned_flag: f32,
    /// Flag-hold start time.
    pub flag_since: f32,
    /// Last carrier-frag time.
    pub last_fragged_carrier: f32,
}

/// Persistent client record (`ClientPersistant`).
#[derive(Debug, Clone, PartialEq)]
pub struct ClientPersistant {
    /// Connection state code.
    pub connected: i32,
    /// Latest command.
    pub cmd: UserCommand,
    /// Local client.
    pub local_client: bool,
    /// Initial spawn consumed.
    pub initial_spawn: bool,
    /// Predict item pickup.
    pub predict_item_pickup: bool,
    /// Fixed pmove.
    pub pmove_fixed: bool,
    /// Net name.
    pub netname: String,
    /// Maximum health.
    pub max_health: i32,
    /// Enter time.
    pub enter_time: i32,
    /// Team state.
    pub team_state: PlayerTeamState,
    /// Votes called.
    pub vote_count: i32,
    /// Team votes called.
    pub team_vote_count: i32,
    /// Receives team info.
    pub team_info: bool,
}

impl Default for ClientPersistant {
    fn default() -> Self {
        Self {
            connected: ConnectionState::Disconnected as i32,
            cmd: UserCommand {
                server_time: 0,
                angles: vec3(0.0, 0.0, 0.0),
                buttons: 0,
                weapon: Weapon::WpNone as i32,
                forwardmove: 0,
                rightmove: 0,
                upmove: 0,
            },
            local_client: false,
            initial_spawn: false,
            predict_item_pickup: false,
            pmove_fixed: false,
            netname: String::new(),
            max_health: 0,
            enter_time: 0,
            team_state: PlayerTeamState::default(),
            vote_count: 0,
            team_vote_count: 0,
            team_info: false,
        }
    }
}

/// Game client (`GameClient`; fields touched by team rules).
#[derive(Clone)]
pub struct GameClient {
    /// Player state.
    pub ps: PlayerState,
    /// Persistent record.
    pub pers: ClientPersistant,
    /// Session record.
    pub sess: ClientSession,
    /// Ready to exit intermission.
    pub ready_to_exit: bool,
    /// Noclip cheat.
    pub noclip: bool,
    /// Last command time.
    pub last_cmd_time: i32,
    /// Buttons.
    pub buttons: i32,
    /// Previous buttons.
    pub old_buttons: i32,
    /// Latched buttons.
    pub latched_buttons: i32,
    /// Previous origin.
    pub old_origin: Vec3,
    /// Absorbed armor damage.
    pub damage_armor: i32,
    /// Absorbed health damage.
    pub damage_blood: i32,
    /// Damage knockback.
    pub damage_knockback: i32,
    /// Damage direction source.
    pub damage_from: Vec3,
    /// Damage came from the world.
    pub damage_from_world: bool,
    /// Accuracy shots.
    pub accuracy_shots: i32,
    /// Accuracy hits.
    pub accuracy_hits: i32,
    /// Last killed client.
    pub last_killed_client: i32,
    /// Last hurt means of death.
    pub last_hurt_mod: i32,
    /// Respawn time.
    pub respawn_time: i32,
    /// Inactivity deadline.
    pub inactivity_time: i32,
    /// Inactivity warned.
    pub inactivity_warning: bool,
    /// Reward expiry.
    pub reward_time: i32,
    /// Air deadline.
    pub air_out_time: i32,
    /// Fire held.
    pub fire_held: bool,
    /// Grapple hook.
    pub hook: Option<EntityRef>,
    /// Team-switch throttle.
    pub switch_team_time: i32,
    /// Timer residual.
    pub time_residual: i32,
    /// Portal id.
    pub portal_id: i32,
    /// Ammo-regeneration timers.
    pub ammo_times: SlotArray,
    /// Invulnerability deadline.
    pub invulnerability_time: i32,
}

impl GameClient {
    /// Source-zero client.
    pub fn new(product: Product) -> Self {
        Self {
            ps: PlayerState::new(product),
            pers: ClientPersistant::default(),
            sess: ClientSession::default(),
            ready_to_exit: false,
            noclip: false,
            last_cmd_time: 0,
            buttons: 0,
            old_buttons: 0,
            latched_buttons: 0,
            old_origin: vec3(0.0, 0.0, 0.0),
            damage_armor: 0,
            damage_blood: 0,
            damage_knockback: 0,
            damage_from: vec3(0.0, 0.0, 0.0),
            damage_from_world: false,
            accuracy_shots: 0,
            accuracy_hits: 0,
            last_killed_client: 0,
            last_hurt_mod: 0,
            respawn_time: 0,
            inactivity_time: 0,
            inactivity_warning: false,
            reward_time: 0,
            air_out_time: 0,
            fire_held: false,
            hook: None,
            switch_team_time: 0,
            time_residual: 0,
            portal_id: 0,
            ammo_times: SlotArray::new(weapon_count(product) as usize),
            invulnerability_time: 0,
        }
    }
}

/// Pool engine hooks (`EntityPoolOptions` subset).
pub struct PoolHooks {
    /// Current game time.
    pub time: Rc<dyn Fn() -> i32>,
    /// Map start time.
    pub map_start_time: i32,
    /// Link an entity.
    pub link: Rc<dyn Fn(&EntityRef)>,
    /// Unlink an entity.
    pub unlink: Rc<dyn Fn(&EntityRef)>,
    /// Engine print.
    pub print: Rc<dyn Fn(&str)>,
}

/// Save-callback registries (`pool.callbacks`).
#[derive(Default)]
pub struct CallbackRegistries {
    think: RefCell<HashMap<String, ThinkCallback>>,
    pain: RefCell<HashMap<String, PainCallback>>,
    die: RefCell<HashMap<String, DieCallback>>,
    touch: RefCell<HashMap<String, TouchCallback>>,
}

impl CallbackRegistries {
    /// Register a think callback.
    pub fn intern_think(&self, name: &str, callback: ThinkCallback) {
        self.think.borrow_mut().insert(name.to_string(), callback);
    }

    /// Resolve a think callback.
    #[must_use]
    pub fn resolve_think(&self, name: &str) -> ThinkCallback {
        self.think
            .borrow()
            .get(name)
            .cloned()
            .unwrap_or_else(|| panic!("unknown think callback {name}"))
    }

    /// Register a pain callback.
    pub fn intern_pain(&self, name: &str, callback: PainCallback) {
        self.pain.borrow_mut().insert(name.to_string(), callback);
    }

    /// Resolve a pain callback.
    #[must_use]
    pub fn resolve_pain(&self, name: &str) -> PainCallback {
        self.pain
            .borrow()
            .get(name)
            .cloned()
            .unwrap_or_else(|| panic!("unknown pain callback {name}"))
    }

    /// Register a death callback.
    pub fn intern_die(&self, name: &str, callback: DieCallback) {
        self.die.borrow_mut().insert(name.to_string(), callback);
    }

    /// Resolve a death callback.
    #[must_use]
    pub fn resolve_die(&self, name: &str) -> DieCallback {
        self.die
            .borrow()
            .get(name)
            .cloned()
            .unwrap_or_else(|| panic!("unknown die callback {name}"))
    }

    /// Register a touch callback.
    pub fn intern_touch(&self, name: &str, callback: TouchCallback) {
        self.touch.borrow_mut().insert(name.to_string(), callback);
    }

    /// Resolve a touch callback.
    #[must_use]
    pub fn resolve_touch(&self, name: &str) -> TouchCallback {
        self.touch
            .borrow()
            .get(name)
            .cloned()
            .unwrap_or_else(|| panic!("unknown touch callback {name}"))
    }
}

/// Rankings reporting (`pool.rankings`).
pub trait RankingsHost {
    /// Report a holdable use.
    fn use_holdable(&self, slot: usize, holdable: i32);
    /// Report a capture.
    fn capture(&self, slot: usize);
    /// Report a powerup pickup.
    fn pickup_powerup(&self, slot: usize, powerup: i32);
}

/// Shared entity pool.
pub type PoolRef = Rc<EntityPool>;

/// Loaded-module entity/client storage (`EntityPool`).
pub struct EntityPool {
    entities: RefCell<Vec<EntityRef>>,
    num_entities: Cell<usize>,
    clients: Vec<ClientRef>,
    max_clients: usize,
    product: Product,
    hooks: PoolHooks,
    /// Save-callback registries.
    pub callbacks: CallbackRegistries,
    /// Rankings reporting.
    pub rankings: Box<dyn RankingsHost>,
}

impl EntityPool {
    /// Fresh pool with linked client records.
    pub fn new(product: Product, max_clients: usize, hooks: PoolHooks, rankings: Box<dyn RankingsHost>) -> Self {
        if !(1..=MAX_CLIENTS).contains(&max_clients) {
            panic!("Entity pool maxClients outside 1..64");
        }
        let identity =
            IdentityOwner::create("q3-team-arena").unwrap_or_else(|_| panic!("Entity pool identity must have a name"));
        let entities: Vec<EntityRef> = (0..MAX_GENTITIES)
            .map(|slot| Rc::new(RefCell::new(GameEntity::new(slot, identity.actor(slot as u32, 0)))))
            .collect();
        let clients: Vec<ClientRef> = (0..MAX_CLIENTS)
            .map(|_| Rc::new(RefCell::new(GameClient::new(product))))
            .collect();
        for index in 0..max_clients {
            entities[index].borrow_mut().client = Some(clients[index].clone());
        }
        Self {
            entities: RefCell::new(entities),
            num_entities: Cell::new(MAX_CLIENTS),
            clients,
            max_clients,
            product,
            hooks,
            callbacks: CallbackRegistries::default(),
            rankings,
        }
    }

    /// Pool product.
    #[must_use]
    pub fn product(&self) -> Product {
        self.product
    }

    /// Configured client count.
    #[must_use]
    pub fn max_clients(&self) -> usize {
        self.max_clients
    }

    /// Open entity high-water mark.
    #[must_use]
    pub fn num_entities(&self) -> usize {
        self.num_entities.get()
    }

    /// All client records.
    #[must_use]
    pub fn clients(&self) -> &[ClientRef] {
        &self.clients
    }

    /// Optional slot lookup.
    #[must_use]
    pub fn get(&self, slot: usize) -> Option<EntityRef> {
        self.entities.borrow().get(slot).cloned()
    }

    /// Checked slot lookup.
    #[must_use]
    pub fn at(&self, slot: usize) -> EntityRef {
        match self.get(slot) {
            Some(entity) => entity,
            None => panic!("Game entity {slot} is unavailable"),
        }
    }

    /// Checked client lookup.
    #[must_use]
    pub fn client_at(&self, slot: usize) -> ClientRef {
        match self.clients.get(slot) {
            Some(client) => client.clone(),
            None => panic!("Game client {slot} is unavailable"),
        }
    }

    /// Pool index of a client record.
    #[must_use]
    pub fn client_index(&self, client: &ClientRef) -> Option<usize> {
        self.clients.iter().position(|entry| Rc::ptr_eq(entry, client))
    }

    /// Whether an entity belongs to this pool.
    fn owned(&self, entity: &EntityRef) {
        match self.get(entity.borrow().slot) {
            Some(owned) if Rc::ptr_eq(&owned, entity) => {}
            _ => panic!("Game entity does not belong to this pool"),
        }
    }

    fn initialize_slot(&self, slot: usize) -> EntityRef {
        let entity = self.at(slot);
        {
            let mut body = entity.borrow_mut();
            let actor = body.actor.clone();
            *body = GameEntity::new(slot, actor);
            body.inuse = true;
            body.client = if slot < self.max_clients {
                Some(self.client_at(slot))
            } else {
                None
            };
        }
        init_game_entity(&entity);
        entity
    }

    /// Allocate an entity (`spawn`).
    pub fn spawn(&self) -> EntityRef {
        let now = (self.hooks.time)();
        for index in MAX_CLIENTS..self.num_entities.get() {
            let entity = self.at(index);
            let body = entity.borrow();
            if body.inuse {
                continue;
            }
            if body.freetime > self.hooks.map_start_time.wrapping_add(2000) && now.wrapping_sub(body.freetime) < 1000 {
                continue;
            }
            drop(body);
            return self.initialize_slot(index);
        }
        if self.num_entities.get() == (ENTITYNUM_WORLD as usize) {
            for index in 0..MAX_GENTITIES {
                let name = self.at(index).borrow().classname();
                (self.hooks.print)(&game_format_bounded(
                    "%4i: %s\n",
                    &[
                        GameFormatArgument::Int(index as i32),
                        match name {
                            Some(text) => GameFormatArgument::Text(text),
                            None => GameFormatArgument::Null,
                        },
                    ],
                    BIG_BUFFER_BYTES,
                ));
            }
            panic!("G_Spawn: no free entities");
        }
        let slot = self.num_entities.get();
        self.num_entities.set(slot + 1);
        self.initialize_slot(slot)
    }

    /// Release an entity (`free`).
    pub fn free(&self, entity: &EntityRef) {
        self.owned(entity);
        (self.hooks.unlink)(entity);
        if entity.borrow().never_free {
            return;
        }
        let now = (self.hooks.time)();
        {
            let mut body = entity.borrow_mut();
            let slot = body.slot;
            let actor = body.actor.clone();
            *body = GameEntity::new(slot, actor);
            body.set_classname(Some("freed".to_string()));
            body.freetime = now;
        }
    }

    /// Spawn a temporary event entity (`tempEntity`).
    pub fn temp_entity(&self, origin: Vec3, event: i32) -> EntityRef {
        let entity = self.spawn();
        {
            let mut body = entity.borrow_mut();
            body.s.e_type = EntityType::EtEvents as i32 + event;
            body.set_classname(Some("tempEntity".to_string()));
            body.event_time = (self.hooks.time)();
            body.free_after_event = true;
        }
        set_origin(
            &entity,
            vec3(
                qvm_float_to_int(origin.x) as f32,
                qvm_float_to_int(origin.y) as f32,
                qvm_float_to_int(origin.z) as f32,
            ),
        );
        (self.hooks.link)(&entity);
        entity
    }

    /// Publish an entity event (`addEvent`).
    pub fn add_event(&self, entity: &EntityRef, event: i32, parameter: i32) {
        self.owned(entity);
        if event == 0 {
            let number = entity.borrow().s.number;
            (self.hooks.print)(&game_format_bounded(
                "G_AddEvent: zero event added for entity %i\n",
                &[GameFormatArgument::Int(number)],
                BIG_BUFFER_BYTES,
            ));
            return;
        }
        let now = (self.hooks.time)();
        let client = entity.borrow().client.clone();
        match client {
            Some(client) => {
                let mut record = client.borrow_mut();
                let bits = ((record.ps.external_event & EV_EVENT_BITS) + EV_EVENT_BIT1) & EV_EVENT_BITS;
                record.ps.external_event = event | bits;
                record.ps.external_event_parm = parameter;
                record.ps.external_event_time = now;
            }
            None => {
                let mut body = entity.borrow_mut();
                let bits = ((body.s.event & EV_EVENT_BITS) + EV_EVENT_BIT1) & EV_EVENT_BITS;
                body.s.event = event | bits;
                body.s.event_parm = parameter;
            }
        }
        entity.borrow_mut().event_time = now;
    }

    /// Activate a client slot.
    pub fn activate_client(&self, slot: usize) -> EntityRef {
        let entity = self.at(slot);
        entity.borrow_mut().inuse = true;
        entity.borrow_mut().client = Some(self.client_at(slot));
        entity
    }

    /// Deactivate a client slot. The client record stays linked: team rules
    /// assume every client slot always resolves a client.
    pub fn deactivate_client(&self, slot: usize) {
        self.at(slot).borrow_mut().inuse = false;
    }

    /// Find an entity by actor handle.
    #[must_use]
    pub fn native_by_actor(&self, actor: &ActorId) -> Option<EntityRef> {
        self.entities
            .borrow()
            .iter()
            .find(|entity| entity.borrow().actor == *actor)
            .cloned()
    }

    /// Format a vector (`utilities.vtos`).
    #[must_use]
    pub fn vtos(&self, vector: Vec3) -> String {
        let value = game_format_bounded(
            "(%i %i %i)",
            &[
                GameFormatArgument::Int(qvm_float_to_int(vector.x)),
                GameFormatArgument::Int(qvm_float_to_int(vector.y)),
                GameFormatArgument::Int(qvm_float_to_int(vector.z)),
            ],
            BIG_BUFFER_BYTES,
        );
        if value.len() >= 32 {
            (self.hooks.print)(&game_format_bounded(
                "Com_sprintf: overflow of %i in %i\n",
                &[GameFormatArgument::Int(value.len() as i32), GameFormatArgument::Int(32)],
                BIG_BUFFER_BYTES,
            ));
        }
        value.chars().take(31).collect()
    }
}

/// Reset per-spawn entity fields (`initGameEntity`).
pub fn init_game_entity(entity: &EntityRef) {
    let mut body = entity.borrow_mut();
    body.set_classname(Some("noclass".to_string()));
    body.s.number = body.slot as i32;
    body.r.owner_num = ENTITYNUM_NONE;
}

/// Store a fixed origin trajectory (`setOrigin`).
pub fn set_origin(entity: &EntityRef, origin: Vec3) {
    let mut body = entity.borrow_mut();
    body.s.pos = Trajectory {
        trajectory_type: TrajectoryType::TrStationary,
        time: 0,
        duration: 0,
        base: vec3(origin.x, origin.y, origin.z),
        delta: vec3(0.0, 0.0, 0.0),
    };
    body.r.set_current_origin(vec3(origin.x, origin.y, origin.z));
}

/// Entity string fields searchable by [`find_entity`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntityStringField {
    /// Class name.
    Classname,
    /// Targetname.
    Targetname,
}

/// Case-insensitive entity search (`findEntity`).
#[must_use]
pub fn find_entity(
    pool: &EntityPool,
    after: Option<&EntityRef>,
    field: EntityStringField,
    value: Option<&str>,
) -> Option<EntityRef> {
    let wanted = value?;
    let folded = ascii_fold(wanted);
    let start = after.map_or(0, |entity| entity.borrow().slot + 1);
    for index in start..pool.num_entities() {
        let entity = pool.at(index);
        let body = entity.borrow();
        if !body.inuse {
            continue;
        }
        let current = match field {
            EntityStringField::Classname => body.classname(),
            EntityStringField::Targetname => body.targetname.clone(),
        };
        match current {
            Some(current) if ascii_fold(&current) == folded => return Some(entity.clone()),
            _ => {}
        }
    }
    None
}

/// Pick a random target by name (`pickTarget`).
pub fn pick_target(
    pool: &EntityPool,
    random_int: &dyn Fn() -> i32,
    warn: &dyn Fn(&str),
    target_name: Option<&str>,
) -> Option<EntityRef> {
    let Some(name) = target_name else {
        warn("G_PickTarget called with NULL targetname\n");
        return None;
    };
    let mut choices = Vec::new();
    let mut found: Option<EntityRef> = None;
    while choices.len() < 32 {
        found = find_entity(pool, found.as_ref(), EntityStringField::Targetname, Some(name));
        match found.clone() {
            Some(entity) => choices.push(entity),
            None => break,
        }
    }
    if choices.is_empty() {
        warn(&format!("G_PickTarget: target {name} not found\n"));
        return None;
    }
    let random = random_int();
    if random < 0 {
        panic!("Game rand must return a nonnegative integer");
    }
    let choice = choices.get((random as usize) % choices.len()).cloned();
    match choice {
        Some(entity) => Some(entity),
        None => panic!("Target selection index invariant"),
    }
}

/// Resolve an actor ground reference to an entity number.
#[must_use]
pub fn ground_number(ground: Option<&ActorId>, pool: &EntityPool) -> i32 {
    match ground {
        None => ENTITYNUM_NONE,
        Some(actor) => pool
            .native_by_actor(actor)
            .map_or(ENTITYNUM_NONE, |entity| entity.borrow().slot as i32),
    }
}

/// Write an entity's ground reference (`writeGround`).
pub fn write_ground(entity: &EntityRef, ground: Option<ActorId>, pool: &EntityPool) {
    let number = ground_number(ground.as_ref(), pool);
    let mut body = entity.borrow_mut();
    body.ground = ground;
    body.s.ground_entity_num = number;
}

/// Trace-hit ground reference (`traceGround`).
pub fn trace_ground(entity: &EntityRef, hit: &TraceHit, pool: &EntityPool) {
    let ground = match hit {
        TraceHit::Actor(actor) => Some(actor.clone()),
        TraceHit::World => Some(pool.at(ENTITYNUM_WORLD as usize).borrow().actor.clone()),
        TraceHit::None => None,
    };
    write_ground(entity, ground, pool);
}

/// Resolve a checkpoint entity reference (`readModuleEntity`).
#[must_use]
pub fn read_module_entity(reader: &SaveReader, pool: &EntityPool) -> EntityRef {
    let slot = reader.integer(0);
    if slot > 1023 {
        reader.fail("module source entity slot exceeds table extent");
    }
    match pool.get(slot as usize) {
        Some(entity) => entity,
        None => reader.fail("module references an absent source entity slot"),
    }
}

pub(crate) fn snap_component(component: f32) -> f32 {
    if (-2_147_483_648.0..2_147_483_648.0).contains(&component) {
        component.trunc()
    } else {
        -2_147_483_648.0
    }
}

pub(crate) fn copy_position(value: Vec3, snap: bool) -> Vec3 {
    if snap {
        vec3(
            snap_component(value.x),
            snap_component(value.y),
            snap_component(value.z),
        )
    } else {
        value
    }
}

pub(crate) fn convert_player_state(
    ps: &mut PlayerState,
    state: &mut EntityState,
    snap: bool,
    extrapolation_time: Option<i32>,
) {
    state.e_type = if ps.pm_type == MoveType::PmIntermission as i32
        || ps.pm_type == MoveType::PmSpectator as i32
        || ps.health() <= GIB_HEALTH
    {
        EntityType::EtInvisible as i32
    } else {
        EntityType::EtPlayer as i32
    };
    state.number = ps.client_num;
    state.pos = Trajectory {
        trajectory_type: if extrapolation_time.is_none() {
            TrajectoryType::TrInterpolate
        } else {
            TrajectoryType::TrLinearStop
        },
        base: copy_position(ps.origin, snap),
        delta: ps.velocity,
        time: extrapolation_time.unwrap_or(state.pos.time),
        duration: extrapolation_time.map_or(state.pos.duration, |_| 50),
    };
    state.apos.trajectory_type = TrajectoryType::TrInterpolate;
    state.apos.base = copy_position(ps.viewangles, snap);
    state.angles2.y = ps.movement_dir as f32;
    state.legs_anim = ps.legs_anim;
    state.torso_anim = ps.torso_anim;
    state.client_num = ps.client_num;
    state.e_flags = if ps.health() <= 0 {
        ps.e_flags | 1
    } else {
        ps.e_flags & !1
    };
    if ps.external_event != 0 {
        state.event = ps.external_event;
        state.event_parm = ps.external_event_parm;
    } else if ps.entity_event_sequence < ps.event_sequence {
        let oldest = ps.event_sequence.wrapping_sub(2);
        if ps.entity_event_sequence < oldest {
            ps.entity_event_sequence = oldest;
        }
        let slot = (ps.entity_event_sequence & 1) as usize;
        state.event = ps.events.get(slot) | ((ps.entity_event_sequence & 3) << 8);
        state.event_parm = ps.event_parms.get(slot);
        ps.entity_event_sequence = ps.entity_event_sequence.wrapping_add(1);
    }
    state.weapon = ps.weapon;
    state.ground_entity_num = ps.ground_entity_num;
    state.powerups = 0;
    for index in 0..ps.powerups.len() {
        if ps.powerups.get(index) != 0 {
            state.powerups |= 1 << index;
        }
    }
    state.loop_sound = ps.loop_sound;
    state.generic1 = ps.generic1;
}

/// Map a player state onto an entity state (`playerStateToEntityState`).
pub fn player_state_to_entity_state(ps: &mut PlayerState, state: &mut EntityState, snap: bool) {
    convert_player_state(ps, state, snap, None);
}

/// Map with linear extrapolation (`playerStateToEntityStateExtraPolate`).
pub fn player_state_to_entity_state_extrapolate(ps: &mut PlayerState, state: &mut EntityState, time: i32, snap: bool) {
    convert_player_state(ps, state, snap, Some(time));
}

/// Trace shape (`TraceShape`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TraceShape {
    /// Point trace.
    Point,
    /// Box trace.
    Box {
        /// Minimums.
        mins: Vec3,
        /// Maximums.
        maxs: Vec3,
    },
    /// Capsule trace.
    Capsule {
        /// Minimums.
        mins: Vec3,
        /// Maximums.
        maxs: Vec3,
    },
}

/// Trace solidity (`solidity`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TraceSolidity {
    /// Clear path.
    Clear,
    /// Starts inside solid.
    StartSolid,
    /// Entirely inside solid.
    AllSolid,
}

/// Trace hit record (`hit`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TraceHit {
    /// No hit.
    None,
    /// World hit.
    World,
    /// Actor hit.
    Actor(ActorId),
}

/// Actor trace query (`ActorTraceQuery`).
#[derive(Debug, Clone, PartialEq)]
pub struct ActorTraceQuery {
    /// Start point.
    pub start: Vec3,
    /// End point.
    pub end: Vec3,
    /// Trace shape.
    pub shape: TraceShape,
    /// Actor to ignore.
    pub pass_actor: Option<ActorId>,
    /// Contents mask.
    pub mask: i32,
}

/// Actor trace result (`ActorTraceResult`).
#[derive(Debug, Clone, PartialEq)]
pub struct ActorTraceResult {
    /// Stopped end point.
    pub end: Vec3,
    /// Path solidity.
    pub solidity: TraceSolidity,
    /// Hit record.
    pub hit: TraceHit,
}

/// Link state (`LinkState`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LinkState {
    /// Absolute bounds.
    pub absbounds: Bounds,
    /// Whether linked.
    pub linked: bool,
    /// Link count.
    pub linkcount: i32,
}

/// Server world plus spatial queries (`ServerWorld` + `ActorSpatialQueries`).
pub trait Q3World {
    /// Link an entity.
    fn link(&self, entity: &EntityRef);
    /// Unlink an entity number.
    fn unlink(&self, number: i32);
    /// Link state for an entity number.
    fn link_state(&self, number: i32) -> Option<LinkState>;
    /// Contents at a point.
    fn point_contents(&self, point: Vec3, pass_entity: i32) -> i32;
    /// Trace against actors.
    fn trace_actor(&self, query: &ActorTraceQuery) -> ActorTraceResult;
    /// Actors overlapping bounds.
    fn area_actors(&self, bounds: &Bounds, maximum: usize) -> Vec<ActorId>;
    /// Bounds/actor contact test.
    fn contact_actor(&self, bounds: &Bounds, actor: &ActorId) -> bool;
}

/// Shared world handle.
pub type WorldRef = Rc<dyn Q3World>;

/// Combat services used by team rules (`CombatContext` subset).
pub trait Combat {
    /// Product.
    fn product(&self) -> Product;
    /// Current time.
    fn time(&self) -> i32;
    /// Game type code.
    fn game_type(&self) -> i32;
    /// Queued intermission time.
    fn intermission_queued(&self) -> i32;
    /// Entity pool.
    fn pool(&self) -> PoolRef;
    /// Apply damage (`damage`).
    #[allow(clippy::too_many_arguments)]
    fn damage(
        &self,
        target: &EntityRef,
        inflictor: Option<&DamageParticipant>,
        attacker: Option<&DamageParticipant>,
        direction: Option<Vec3>,
        point: Option<Vec3>,
        amount: i32,
        flags: i32,
        method: i32,
    );
}

/// Shared combat handle.
pub type CombatRef = Rc<dyn Combat>;

/// Weapon services (`WeaponRuntime` subset).
pub trait WeaponHost {
    /// Fire the current weapon.
    fn fire(&self, entity: &EntityRef);
    /// Start a kamikaze countdown.
    fn start_kamikaze(&self, entity: &EntityRef);
}

/// Personal-portal services (`PersonalPortalRuntime` subset).
pub trait PortalHost {
    /// Drop a portal source.
    fn drop_portal_source(&self, entity: &EntityRef);
    /// Drop a portal destination.
    fn drop_portal_destination(&self, entity: &EntityRef);
}

/// Item-drop services (`DropItemContext`).
pub trait DropHost {
    /// Drop an item near an entity.
    fn drop_item(&self, entity: &EntityRef, item: &ItemDefinition, angle: i32) -> EntityRef;
}

/// Death services (`DeathRuntime` subset).
pub trait DeathHost {
    /// Kill a player.
    fn player_die(
        &self,
        target: &EntityRef,
        inflictor: Option<&DamageParticipant>,
        attacker: Option<&DamageParticipant>,
        damage: i32,
        method: i32,
    );
    /// Toss a client's items.
    fn toss_client_items(&self, entity: &EntityRef);
    /// Toss persistent powerups.
    fn toss_client_persistant_powerups(&self, entity: &EntityRef);
    /// Toss harvester cubes.
    fn toss_client_cubes(&self, entity: &EntityRef);
}

/// Item services (`items.ts` + `ItemLifecycleContext` subsets).
pub trait ItemHost {
    /// Item by product and index.
    fn item_at(&self, product: Product, index: usize) -> ItemDefinition;
    /// Item by pickup name.
    fn find_item(&self, product: Product, name: &str) -> Option<ItemDefinition>;
    /// Item carrying a powerup.
    fn find_item_for_powerup(&self, product: Product, powerup: i32) -> Option<ItemDefinition>;
    /// Spawn an item entity (`spawnItem`).
    fn spawn_item(&self, entity: &EntityRef, item: &ItemDefinition, vars: &SpawnVariables, disabled: bool);
    /// Finish spawning an item (`finishSpawningItem`).
    fn finish_spawning_item(&self, entity: &EntityRef);
    /// Touch an item (`touchItem`).
    fn touch_item(&self, entity: &EntityRef, other: &DamageParticipant, contact: &TouchContact);
}

/// Target services (`TargetUseContext` subset).
pub trait TargetsHost {
    /// Fire an entity's targets (`useTargets`).
    fn use_targets(&self, used: Option<&EntityRef>, activator: Option<&DamageParticipant>);
}

/// Teleport services (`TeleportContext`).
pub trait TeleportHost {
    /// Teleport a player (`teleportPlayer`).
    fn teleport_player(&self, entity: &EntityRef, origin: Vec3, angles: Vec3);
}

/// Cvar value snapshot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CvarValue {
    /// String value.
    pub value: String,
    /// Integer value.
    pub integer_value: i32,
}

/// VM cvar snapshot (`CvarSnapshot` subset).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CvarSnapshot {
    /// String value.
    pub value: String,
    /// Integer value.
    pub integer_value: i32,
    /// Modification count.
    pub modification_count: i32,
}

/// Cvar registry (`CvarRegistry` subset).
pub trait CvarRegistry {
    /// Read a cvar.
    fn get(&self, name: &str) -> Option<CvarValue>;
    /// Write a cvar.
    fn set(&self, name: &str, value: &str, force: bool);
}

/// Config-string services (`ConfigStringRegistry` subset).
pub trait ConfigStrings {
    /// Model index for a path.
    fn model_index(&self, name: &str) -> i32;
}

/// Session cvar name (`SessionCvarName`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SessionCvarName {
    /// World session record.
    Session,
    /// Per-client session record.
    SessionN(usize),
}

impl SessionCvarName {
    /// Cvar spelling.
    #[must_use]
    pub fn as_string(&self) -> String {
        match self {
            SessionCvarName::Session => "session".to_string(),
            SessionCvarName::SessionN(index) => format!("session{index}"),
        }
    }
}

/// Session cvar storage (`SessionCvarService`).
pub trait SessionCvarService {
    /// Read a session cvar.
    fn get(&self, name: &SessionCvarName) -> String;
    /// Write a session cvar.
    fn set(&self, name: &SessionCvarName, value: &str);
}

/// Session services (`SessionServices`).
pub trait SessionServices {
    /// Engine print.
    fn print(&self, message: &str);
    /// Announce a team change.
    fn broadcast_team_change(&self, client_num: usize, old_team: i32);
}
