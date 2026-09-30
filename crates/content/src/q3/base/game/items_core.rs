//! Quake III base/game: items-group shared core.
//!
//! Donor provenance: shared support for `src/content/q3/base/game/item-motion.ts`,
//! `item-pickup.ts`, `level.ts`, `memory.ts`, `misc.ts`, `misc-spawn.ts`,
//! `missile.ts`, and `mover-spawn.ts`. Each item mirrors the donor module that
//! owns it; this module keeps the group-coupled subset ports (slot entity pool,
//! string-keyed callbacks, infallible text scans) whose pool/slot shape the 1:1
//! canonicals cannot absorb without behavior changes. Shared enums, constants,
//! and pure helpers live in their canonical homes and are imported here.

use std::collections::HashSet;

use qa_core::math::{vec3, Bounds, Vec3};
use qa_core::numeric::qvm_float_to_int;
use thiserror::Error;

use crate::q3::base::game::state::{ConnectionState, MoverState, MAX_CLIENTS, MAX_GENTITIES};
use crate::q3::base::game::utilities::EntityStringField;
use crate::q3::base::shared::definitions::{
    EntityEvent, EntityType, Holdable, ItemType, MoveType, Powerup, Product, Team, Weapon, EV_EVENT_BIT1,
    EV_EVENT_BITS, GIB_HEALTH,
};
use crate::q3::base::shared::player_state::{ENTITYNUM_NONE, ENTITYNUM_WORLD};
use crate::q3::base::shared::snapshot_state::copy_position;
use crate::q3::base::shared::trajectory::{Trajectory, TrajectoryType};
use crate::q3::base::world::{TraceShape, TraceSolidity};

/// Game-items failure: range/invalid/missing map the donor `RangeError` and
/// `Error`; `Drop` maps the donor `CommonError("drop", ...)`.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum Q3GameItemsError {
    /// Value outside its valid range (donor `RangeError`).
    #[error("range error: {0}")]
    Range(String),
    /// Invalid state or argument (donor `Error`).
    #[error("invalid: {0}")]
    Invalid(String),
    /// Missing required record (donor `Error` on absent lookups).
    #[error("missing: {0}")]
    Missing(String),
    /// Fatal game drop (donor `CommonError("drop", ...)`).
    #[error("drop: {0}")]
    Drop(String),
}

/// Game-items result.
pub type Q3GameItemsResult<T> = Result<T, Q3GameItemsError>;

pub(crate) fn range(message: impl Into<String>) -> Q3GameItemsError {
    Q3GameItemsError::Range(message.into())
}

pub(crate) fn invalid(message: impl Into<String>) -> Q3GameItemsError {
    Q3GameItemsError::Invalid(message.into())
}

pub(crate) fn missing(message: impl Into<String>) -> Q3GameItemsError {
    Q3GameItemsError::Missing(message.into())
}

pub(crate) fn drop_error(message: impl Into<String>) -> Q3GameItemsError {
    Q3GameItemsError::Drop(message.into())
}

/// `bg_lib` atoi (`gameAtoi`): wrapping decimal-prefix parse.
#[must_use]
pub fn game_atoi(text: &str) -> i32 {
    let bytes = text.as_bytes();
    let mut offset = 0;
    while offset < bytes.len() && matches!(bytes[offset], b'\t' | b'\n' | 0x0C | b'\r' | b' ') {
        offset += 1;
    }
    if offset >= bytes.len() || bytes[offset] == 0 {
        return 0;
    }
    let mut sign = 1i32;
    if bytes[offset] == b'+' || bytes[offset] == b'-' {
        if bytes[offset] == b'-' {
            sign = -1;
        }
        offset += 1;
    }
    let mut value = 0i32;
    while offset < bytes.len() {
        let byte = bytes[offset];
        if !byte.is_ascii_digit() {
            break;
        }
        value = value.wrapping_mul(10).wrapping_add(i32::from(byte - b'0'));
        offset += 1;
    }
    value.wrapping_mul(sign)
}

/// `bg_lib` atof (`gameAtof`): decimal prefix only, binary32 operations.
#[must_use]
pub fn game_atof(text: &str) -> f32 {
    let bytes = text.as_bytes();
    let mut offset = 0;
    while offset < bytes.len() && matches!(bytes[offset], b'\t' | b'\n' | 0x0C | b'\r' | b' ') {
        offset += 1;
    }
    if offset >= bytes.len() || bytes[offset] == 0 {
        return 0.0;
    }
    let mut sign = 1.0f32;
    if bytes[offset] == b'+' || bytes[offset] == b'-' {
        if bytes[offset] == b'-' {
            sign = -1.0;
        }
        offset += 1;
    }
    let mut value = 0.0f32;
    let mut character = if offset < bytes.len() { bytes[offset] } else { 0 };
    if character != b'.' {
        loop {
            character = if offset < bytes.len() { bytes[offset] } else { 0 };
            offset += 1;
            if !character.is_ascii_digit() {
                break;
            }
            value = value * 10.0 + f32::from(character - b'0');
        }
    } else {
        offset += 1;
    }
    if character == b'.' {
        let mut fraction = 0.1f32;
        loop {
            character = if offset < bytes.len() { bytes[offset] } else { 0 };
            offset += 1;
            if !character.is_ascii_digit() {
                break;
            }
            value += f32::from(character - b'0') * fraction;
            fraction *= 0.1;
        }
    }
    value * sign
}

/// Item kind: discriminated union of item type + tag (`ItemDefinition`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ItemKind {
    /// Invalid.
    Bad,
    /// Weapon with weapon tag.
    Weapon(Weapon),
    /// Ammo with weapon tag.
    Ammo(Weapon),
    /// Armor.
    Armor,
    /// Health.
    Health,
    /// Powerup with powerup tag.
    Powerup(Powerup),
    /// Holdable with holdable tag.
    Holdable(Holdable),
    /// Persistent powerup with powerup tag.
    PersistantPowerup(Powerup),
    /// Team objective with powerup tag.
    Team(Powerup),
}

impl ItemKind {
    /// Item type of this kind.
    #[must_use]
    pub fn item_type(self) -> ItemType {
        match self {
            ItemKind::Bad => ItemType::ItBad,
            ItemKind::Weapon(_) => ItemType::ItWeapon,
            ItemKind::Ammo(_) => ItemType::ItAmmo,
            ItemKind::Armor => ItemType::ItArmor,
            ItemKind::Health => ItemType::ItHealth,
            ItemKind::Powerup(_) => ItemType::ItPowerup,
            ItemKind::Holdable(_) => ItemType::ItHoldable,
            ItemKind::PersistantPowerup(_) => ItemType::ItPersistantPowerup,
            ItemKind::Team(_) => ItemType::ItTeam,
        }
    }
}

/// Item definition subset used by game item logic (`ItemDefinition`).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ItemDefinition {
    /// Entity class name.
    pub class_name: Option<String>,
    /// Default quantity.
    pub quantity: i32,
    /// Discriminated type + tag.
    pub kind: ItemKind,
}

impl ItemDefinition {
    /// Item type.
    #[must_use]
    pub fn item_type(&self) -> ItemType {
        self.kind.item_type()
    }

    /// Weapon tag; fails for non-weapon/ammo items.
    pub fn weapon_tag(&self) -> Q3GameItemsResult<Weapon> {
        match self.kind {
            ItemKind::Weapon(tag) | ItemKind::Ammo(tag) => Ok(tag),
            _ => Err(invalid("item has no weapon tag")),
        }
    }

    /// Powerup tag; fails for other items.
    pub fn powerup_tag(&self) -> Q3GameItemsResult<Powerup> {
        match self.kind {
            ItemKind::Powerup(tag) | ItemKind::PersistantPowerup(tag) | ItemKind::Team(tag) => Ok(tag),
            _ => Err(invalid("item has no powerup tag")),
        }
    }

    /// Holdable tag; fails for other items.
    pub fn holdable_tag(&self) -> Q3GameItemsResult<Holdable> {
        match self.kind {
            ItemKind::Holdable(tag) => Ok(tag),
            _ => Err(invalid("item has no holdable tag")),
        }
    }
}

/// Item table host (`itemList`/`itemAt`/`findItemForWeapon`).
pub trait ItemTable {
    /// Index of an item definition, or `None` when absent.
    fn index_of(&self, product: Product, item: &ItemDefinition) -> Option<usize>;
    /// Item at an index.
    fn item_at(&self, product: Product, index: usize) -> Q3GameItemsResult<ItemDefinition>;
    /// Item for a weapon tag.
    fn find_item_for_weapon(&self, product: Product, weapon: Weapon) -> Q3GameItemsResult<ItemDefinition>;
}

/// Item registration host (`ItemRegistry`).
pub trait ItemRegistry {
    /// Registry product.
    fn product(&self) -> Product;
    /// Register an item (precache).
    fn register(&mut self, item: &ItemDefinition);
}

/// Player-state integer slots (`PlayerStateSlots`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlayerStateSlots {
    values: Vec<i32>,
}

impl PlayerStateSlots {
    /// Zeroed slots.
    #[must_use]
    pub fn new(length: usize) -> PlayerStateSlots {
        PlayerStateSlots {
            values: vec![0; length],
        }
    }

    /// Slot count.
    #[must_use]
    pub fn len(&self) -> usize {
        self.values.len()
    }

    /// Whether there are no slots.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    /// Read a slot.
    pub fn get(&self, index: usize) -> Q3GameItemsResult<i32> {
        self.values
            .get(index)
            .copied()
            .ok_or_else(|| range(format!("player state slot {index} outside {}", self.values.len())))
    }

    /// Write a slot.
    pub fn set(&mut self, index: usize, value: i32) -> Q3GameItemsResult<()> {
        let len = self.values.len();
        let slot = self
            .values
            .get_mut(index)
            .ok_or_else(|| range(format!("player state slot {index} outside {len}")))?;
        *slot = value;
        Ok(())
    }

    /// Fill every slot.
    pub fn fill(&mut self, value: i32) {
        self.values.fill(value);
    }
}

/// Pool slot address of an entity.
pub type Slot = usize;

/// Actor identity (`ActorId`); the actor number is the pool slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ActorId(pub u32);

impl ActorId {
    /// Actor for a pool slot.
    #[must_use]
    pub fn from_slot(slot: Slot) -> ActorId {
        ActorId(slot as u32)
    }

    /// Pool slot, when in range.
    #[must_use]
    pub fn slot(self) -> Option<Slot> {
        let slot = self.0 as usize;
        (slot < MAX_GENTITIES).then_some(slot)
    }
}

/// Owned actor handle (`OwnedActor`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct OwnedActor {
    /// Actor identity.
    pub id: ActorId,
}

/// Event-bit constants (`EV_EVENT_BIT1`/`EV_EVENT_BITS`).
#[derive(Debug, Clone, PartialEq)]
pub struct EntityState {
    /// Entity type (raw; temp entities add the event, mirroring the donor).
    pub e_type: i32,
    /// Entity number.
    pub number: i32,
    /// Client number.
    pub client_num: i32,
    /// Spawn origin.
    pub origin: Vec3,
    /// Secondary origin (portals).
    pub origin2: Vec3,
    /// Angles.
    pub angles: Vec3,
    /// Secondary angles.
    pub angles2: Vec3,
    /// Position trajectory.
    pub pos: Trajectory,
    /// Angle trajectory.
    pub apos: Trajectory,
    /// Weapon.
    pub weapon: Weapon,
    /// Model index.
    pub modelindex: i32,
    /// Secondary model index.
    pub modelindex2: i32,
    /// Entity flags.
    pub e_flags: i32,
    /// Event.
    pub event: i32,
    /// Event parameter.
    pub event_parm: i32,
    /// Other entity number.
    pub other_entity_num: i32,
    /// Ground entity number.
    pub ground_entity_num: i32,
    /// Loop sound.
    pub loop_sound: i32,
    /// Generic 1.
    pub generic1: i32,
    /// Powerups bitmask.
    pub powerups: i32,
    /// Frame.
    pub frame: i32,
    /// Legs animation.
    pub legs_anim: i32,
    /// Torso animation.
    pub torso_anim: i32,
}

impl Default for EntityState {
    fn default() -> Self {
        EntityState {
            e_type: EntityType::EtGeneral as i32,
            number: 0,
            client_num: 0,
            origin: vec3(0.0, 0.0, 0.0),
            origin2: vec3(0.0, 0.0, 0.0),
            angles: vec3(0.0, 0.0, 0.0),
            angles2: vec3(0.0, 0.0, 0.0),
            pos: Trajectory::zero(TrajectoryType::TrStationary),
            apos: Trajectory::zero(TrajectoryType::TrStationary),
            weapon: Weapon::WpNone,
            modelindex: 0,
            modelindex2: 0,
            e_flags: 0,
            event: 0,
            event_parm: 0,
            other_entity_num: 0,
            ground_entity_num: ENTITYNUM_NONE,
            loop_sound: 0,
            generic1: 0,
            powerups: 0,
            frame: 0,
            legs_anim: 0,
            torso_anim: 0,
        }
    }
}

/// Shared entity subset (`EntityShared` / `r`).
#[derive(Debug, Clone, PartialEq)]
pub struct EntityShared {
    /// Current origin.
    pub current_origin: Vec3,
    /// Current angles.
    pub current_angles: Vec3,
    /// Bounds minimum.
    pub mins: Vec3,
    /// Bounds maximum.
    pub maxs: Vec3,
    /// Absolute bounds minimum.
    pub absmin: Vec3,
    /// Absolute bounds maximum.
    pub absmax: Vec3,
    /// Contents mask.
    pub contents: i32,
    /// Owner entity number.
    pub owner_num: i32,
    /// Server flags.
    pub sv_flags: i32,
}

impl Default for EntityShared {
    fn default() -> Self {
        EntityShared {
            current_origin: vec3(0.0, 0.0, 0.0),
            current_angles: vec3(0.0, 0.0, 0.0),
            mins: vec3(0.0, 0.0, 0.0),
            maxs: vec3(0.0, 0.0, 0.0),
            absmin: vec3(0.0, 0.0, 0.0),
            absmax: vec3(0.0, 0.0, 0.0),
            contents: 0,
            owner_num: ENTITYNUM_NONE,
            sv_flags: 0,
        }
    }
}

/// Player state subset (`PlayerState`).
#[derive(Debug, Clone, PartialEq)]
pub struct PlayerState {
    /// Product.
    pub product: Product,
    /// Client number.
    pub client_num: i32,
    /// Origin.
    pub origin: Vec3,
    /// Velocity.
    pub velocity: Vec3,
    /// View angles.
    pub viewangles: Vec3,
    /// Delta angles.
    pub delta_angles: [i32; 3],
    /// PM time.
    pub pm_time: i32,
    /// PM flags.
    pub pm_flags: i32,
    /// PM type.
    pub pm_type: MoveType,
    /// Entity flags.
    pub e_flags: i32,
    /// Health.
    pub health: i32,
    /// Stat slots.
    pub stats: PlayerStateSlots,
    /// Persistent slots.
    pub persistant: PlayerStateSlots,
    /// Powerup slots.
    pub powerups: PlayerStateSlots,
    /// Ammo slots indexed by weapon.
    pub ammo: PlayerStateSlots,
    /// Current weapon.
    pub weapon: Weapon,
    /// Legs animation.
    pub legs_anim: i32,
    /// Torso animation.
    pub torso_anim: i32,
    /// Movement direction.
    pub movement_dir: f32,
    /// Ground entity number.
    pub ground_entity_num: i32,
    /// Loop sound.
    pub loop_sound: i32,
    /// Generic 1.
    pub generic1: i32,
    /// External event.
    pub external_event: i32,
    /// External event parameter.
    pub external_event_parm: i32,
    /// External event time.
    pub external_event_time: i32,
    /// Event sequence.
    pub event_sequence: i32,
    /// Entity event sequence.
    pub entity_event_sequence: i32,
    /// Predictable events.
    pub events: PlayerStateSlots,
    /// Predictable event parameters.
    pub event_parms: PlayerStateSlots,
    /// Grapple point.
    pub grapple_point: Vec3,
}

impl PlayerState {
    /// Zero player state for a product.
    #[must_use]
    pub fn new(product: Product) -> PlayerState {
        PlayerState {
            product,
            client_num: 0,
            origin: vec3(0.0, 0.0, 0.0),
            velocity: vec3(0.0, 0.0, 0.0),
            viewangles: vec3(0.0, 0.0, 0.0),
            delta_angles: [0, 0, 0],
            pm_time: 0,
            pm_flags: 0,
            pm_type: MoveType::PmNormal,
            e_flags: 0,
            health: 0,
            stats: PlayerStateSlots::new(16),
            persistant: PlayerStateSlots::new(16),
            powerups: PlayerStateSlots::new(16),
            ammo: PlayerStateSlots::new(16),
            weapon: Weapon::WpNone,
            legs_anim: 0,
            torso_anim: 0,
            movement_dir: 0.0,
            ground_entity_num: ENTITYNUM_NONE,
            loop_sound: 0,
            generic1: 0,
            external_event: 0,
            external_event_parm: 0,
            external_event_time: 0,
            event_sequence: 0,
            entity_event_sequence: 0,
            events: PlayerStateSlots::new(2),
            event_parms: PlayerStateSlots::new(2),
            grapple_point: vec3(0.0, 0.0, 0.0),
        }
    }
}

/// Persistent client subset (`ClientPersistant`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientPersistant {
    /// Connection state.
    pub connected: ConnectionState,
    /// Command view angles.
    pub cmd_angles: [i32; 3],
    /// Maximum health.
    pub max_health: i32,
}

impl Default for ClientPersistant {
    fn default() -> Self {
        ClientPersistant {
            connected: ConnectionState::Disconnected,
            cmd_angles: [0, 0, 0],
            max_health: 0,
        }
    }
}

/// Client session subset (`ClientSession`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientSession {
    /// Session team.
    pub session_team: Team,
}

impl Default for ClientSession {
    fn default() -> Self {
        ClientSession {
            session_team: Team::TeamFree,
        }
    }
}

/// Game client subset (`GameClient`).
#[derive(Debug, Clone, PartialEq)]
pub struct GameClient {
    /// Player state.
    pub ps: PlayerState,
    /// Persistent data.
    pub pers: ClientPersistant,
    /// Session data.
    pub sess: ClientSession,
    /// Grapple hook entity.
    pub hook: Option<Slot>,
    /// Persistent powerup entity.
    pub persistant_powerup: Option<Slot>,
    /// Per-weapon ammo times.
    pub ammo_times: PlayerStateSlots,
    /// Invulnerability expiry.
    pub invulnerability_time: i32,
    /// Accuracy hits.
    pub accuracy_hits: i32,
}

impl GameClient {
    /// Zero client for a product.
    #[must_use]
    pub fn new(product: Product) -> GameClient {
        GameClient {
            ps: PlayerState::new(product),
            pers: ClientPersistant::default(),
            sess: ClientSession::default(),
            hook: None,
            persistant_powerup: None,
            ammo_times: PlayerStateSlots::new(14),
            invulnerability_time: 0,
            accuracy_hits: 0,
        }
    }
}

/// Save-callback name; the donor's string-keyed callback identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CallbackName(pub &'static str);

/// String-keyed callback registry (`intern`/`resolve` per donor catalogs).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CallbackTable {
    names: HashSet<&'static str>,
}

impl CallbackTable {
    /// Register a callback name.
    pub fn intern(&mut self, name: &'static str) {
        self.names.insert(name);
    }

    /// Resolve a registered name.
    pub fn resolve(&self, name: &'static str) -> Q3GameItemsResult<CallbackName> {
        if self.names.contains(name) {
            Ok(CallbackName(name))
        } else {
            Err(missing(format!("callback {name} is not registered")))
        }
    }

    /// Whether a name is registered.
    #[must_use]
    pub fn contains(&self, name: &'static str) -> bool {
        self.names.contains(name)
    }
}

/// Damage participant (`DamageParticipant`): a pooled entity or a shared actor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DamageParticipant {
    /// Pooled entity slot.
    Entity(Slot),
    /// Shared actor without a pooled record.
    SharedActor(ActorId),
}

/// Game entity subset (`GameEntity`).
#[derive(Debug, Clone, PartialEq)]
pub struct GameEntity {
    /// Pool slot.
    pub slot: Slot,
    /// Generation (bumped on free).
    pub generation: u32,
    /// In-use flag.
    pub inuse: bool,
    /// Linked flag.
    pub linked: bool,
    /// Class name.
    pub classname: Option<String>,
    /// Brush model name.
    pub model: Option<String>,
    /// Target name.
    pub target: Option<String>,
    /// Targetname.
    pub targetname: Option<String>,
    /// Item definition.
    pub item: Option<ItemDefinition>,
    /// Entity state.
    pub s: EntityState,
    /// Shared state.
    pub r: EntityShared,
    /// Client record.
    pub client: Option<GameClient>,
    /// Health.
    pub health: i32,
    /// Takes damage.
    pub takedamage: bool,
    /// Physics bounce factor.
    pub physics_bounce: f32,
    /// Game flags.
    pub flags: i32,
    /// Spawn flags.
    pub spawnflags: i32,
    /// Next think time.
    pub nextthink: i32,
    /// Think callback.
    pub think: Option<CallbackName>,
    /// Touch callback.
    pub touch: Option<CallbackName>,
    /// Use callback.
    pub use_cb: Option<CallbackName>,
    /// Die callback.
    pub die: Option<CallbackName>,
    /// Blocked callback.
    pub blocked: Option<CallbackName>,
    /// Reached callback.
    pub reached: Option<CallbackName>,
    /// Enemy slot.
    pub enemy: Option<Slot>,
    /// Parent slot.
    pub parent: Option<Slot>,
    /// Activator slot.
    pub activator: Option<Slot>,
    /// Target entity slot.
    pub target_ent: Option<Slot>,
    /// Team chain slot.
    pub teamchain: Option<Slot>,
    /// Next train slot.
    pub next_train: Option<Slot>,
    /// Count (quantity override / trigger axis / arming flag).
    pub count: i32,
    /// Random spread / misc float.
    pub random: f32,
    /// Move direction.
    pub movedir: Vec3,
    /// Speed.
    pub speed: f32,
    /// Wait.
    pub wait: f32,
    /// Damage.
    pub damage: i32,
    /// Splash damage.
    pub splash_damage: i32,
    /// Splash radius.
    pub splash_radius: f32,
    /// Means of death.
    pub method_of_death: i32,
    /// Splash means of death.
    pub splash_method_of_death: i32,
    /// Clip mask.
    pub clipmask: i32,
    /// Door/plat sounds.
    pub sound1to2: i32,
    /// Door/plat sounds.
    pub sound2to1: i32,
    /// Door/plat sounds.
    pub sound_pos1: i32,
    /// Door/plat sounds.
    pub sound_pos2: i32,
    /// Looping sound.
    pub sound_loop: i32,
    /// Mover position 1.
    pub pos1: Vec3,
    /// Mover position 2.
    pub pos2: Vec3,
    /// Mover state.
    pub mover_state: MoverState,
    /// Free after event.
    pub free_after_event: bool,
    /// Event time.
    pub event_time: i32,
    /// Ground actor (shared-body support mirror).
    pub ground: Option<ActorId>,
}

impl GameEntity {
    /// Fresh pooled record.
    #[must_use]
    pub fn new(slot: Slot, generation: u32) -> GameEntity {
        GameEntity {
            slot,
            generation,
            inuse: false,
            linked: false,
            classname: None,
            model: None,
            target: None,
            targetname: None,
            item: None,
            s: EntityState::default(),
            r: EntityShared::default(),
            client: None,
            health: 0,
            takedamage: false,
            physics_bounce: 0.0,
            flags: 0,
            spawnflags: 0,
            nextthink: 0,
            think: None,
            touch: None,
            use_cb: None,
            die: None,
            blocked: None,
            reached: None,
            enemy: None,
            parent: None,
            activator: None,
            target_ent: None,
            teamchain: None,
            next_train: None,
            count: 0,
            random: 0.0,
            movedir: vec3(0.0, 0.0, 0.0),
            speed: 0.0,
            wait: 0.0,
            damage: 0,
            splash_damage: 0,
            splash_radius: 0.0,
            method_of_death: 0,
            splash_method_of_death: 0,
            clipmask: 0,
            sound1to2: 0,
            sound2to1: 0,
            sound_pos1: 0,
            sound_pos2: 0,
            sound_loop: 0,
            pos1: vec3(0.0, 0.0, 0.0),
            pos2: vec3(0.0, 0.0, 0.0),
            mover_state: MoverState::Pos1,
            free_after_event: false,
            event_time: 0,
            ground: None,
        }
    }

    /// Client record or an error.
    pub fn client(&self) -> Q3GameItemsResult<&GameClient> {
        self.client
            .as_ref()
            .ok_or_else(|| invalid("entity has no client record"))
    }

    /// Mutable client record or an error.
    pub fn client_mut(&mut self) -> Q3GameItemsResult<&mut GameClient> {
        self.client
            .as_mut()
            .ok_or_else(|| invalid("entity has no client record"))
    }
}

/// Fired event record for host dispatch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FiredEvent {
    /// Entity slot.
    pub slot: Slot,
    /// Event.
    pub event: EntityEvent,
    /// Parameter.
    pub parm: i32,
    /// Time.
    pub time: i32,
}

/// Entity pool (`EntityPool` subset).
#[derive(Debug)]
pub struct EntityPool {
    pub(crate) product: Product,
    pub(crate) entities: Vec<GameEntity>,
    pub(crate) generations: Vec<u32>,
    pub(crate) high_water: usize,
    /// Current game time for event stamps (host sets per frame).
    pub time: i32,
    /// Think callbacks.
    pub think_cbs: CallbackTable,
    /// Touch callbacks.
    pub touch_cbs: CallbackTable,
    /// Use callbacks.
    pub use_cbs: CallbackTable,
    /// Die callbacks.
    pub die_cbs: CallbackTable,
    /// Blocked callbacks.
    pub blocked_cbs: CallbackTable,
    /// Reached callbacks.
    pub reached_cbs: CallbackTable,
    /// Fired non-client event log.
    pub events: Vec<FiredEvent>,
    /// Diagnostic prints (e.g. zero-event warnings).
    pub prints: Vec<String>,
}

impl EntityPool {
    /// Empty pool with a world entity at slot 1022.
    #[must_use]
    pub fn new(product: Product) -> EntityPool {
        let mut entities = Vec::with_capacity(MAX_GENTITIES);
        for slot in 0..MAX_GENTITIES {
            entities.push(GameEntity::new(slot, 0));
        }
        let mut pool = EntityPool {
            product,
            entities,
            generations: vec![0; MAX_GENTITIES],
            high_water: MAX_CLIENTS,
            time: 0,
            think_cbs: CallbackTable::default(),
            touch_cbs: CallbackTable::default(),
            use_cbs: CallbackTable::default(),
            die_cbs: CallbackTable::default(),
            blocked_cbs: CallbackTable::default(),
            reached_cbs: CallbackTable::default(),
            events: Vec::new(),
            prints: Vec::new(),
        };
        let world = pool.at_mut(ENTITYNUM_WORLD as usize).expect("world slot in range");
        world.inuse = true;
        world.classname = Some("worldspawn".to_string());
        world.s.number = ENTITYNUM_WORLD;
        pool
    }

    /// Pool product.
    #[must_use]
    pub fn product(&self) -> Product {
        self.product
    }

    /// Entity high-water mark (`numEntities`).
    #[must_use]
    pub fn num_entities(&self) -> usize {
        self.high_water
    }

    /// Record by slot regardless of use (`at`).
    pub fn at(&self, slot: Slot) -> Q3GameItemsResult<&GameEntity> {
        self.entities
            .get(slot)
            .ok_or_else(|| range(format!("entity slot {slot} outside {MAX_GENTITIES}")))
    }

    /// Mutable record by slot regardless of use.
    pub fn at_mut(&mut self, slot: Slot) -> Q3GameItemsResult<&mut GameEntity> {
        self.entities
            .get_mut(slot)
            .ok_or_else(|| range(format!("entity slot {slot} outside {MAX_GENTITIES}")))
    }

    /// Live record by slot (`get`).
    #[must_use]
    pub fn get(&self, slot: Slot) -> Option<&GameEntity> {
        self.entities.get(slot).filter(|entity| entity.inuse)
    }

    /// Live mutable record by slot.
    pub fn get_mut(&mut self, slot: Slot) -> Option<&mut GameEntity> {
        self.entities.get_mut(slot).filter(|entity| entity.inuse)
    }

    /// Generation counter for a slot.
    pub fn generation(&self, slot: Slot) -> Q3GameItemsResult<u32> {
        self.generations
            .get(slot)
            .copied()
            .ok_or_else(|| range(format!("entity slot {slot} outside {MAX_GENTITIES}")))
    }

    /// Ownership check: the slot must hold a live record.
    pub fn require_owned(&self, slot: Slot) -> Q3GameItemsResult<()> {
        if self.get(slot).is_some() {
            Ok(())
        } else {
            Err(invalid(format!(
                "entity {slot} does not belong to its pool or was freed"
            )))
        }
    }

    /// Spawn a record (`spawn`).
    pub fn spawn(&mut self) -> Q3GameItemsResult<Slot> {
        for slot in 0..MAX_GENTITIES {
            if !self.entities[slot].inuse {
                let generation = self.generations[slot];
                let mut entity = GameEntity::new(slot, generation);
                entity.inuse = true;
                entity.s.number = slot as i32;
                self.entities[slot] = entity;
                self.high_water = self.high_water.max(slot + 1);
                return Ok(slot);
            }
        }
        Err(drop_error("G_Spawn: no free entities"))
    }

    /// Free a record (`free`).
    pub fn free(&mut self, slot: Slot) -> Q3GameItemsResult<()> {
        self.require_owned(slot)?;
        self.generations[slot] = self.generations[slot].wrapping_add(1);
        let generation = self.generations[slot];
        self.entities[slot] = GameEntity::new(slot, generation);
        Ok(())
    }

    /// Live slot for an actor (`nativeByActor`).
    #[must_use]
    pub fn native_by_actor(&self, actor: ActorId) -> Option<Slot> {
        actor
            .slot()
            .filter(|slot| self.get(*slot).is_some_and(|entity| entity.inuse))
    }

    /// Mark a record linked (`options.link` pool half).
    pub fn link(&mut self, slot: Slot) -> Q3GameItemsResult<()> {
        self.require_owned(slot)?;
        self.entities[slot].linked = true;
        Ok(())
    }

    /// Mark a record unlinked.
    pub fn unlink(&mut self, slot: Slot) -> Q3GameItemsResult<()> {
        self.require_owned(slot)?;
        self.entities[slot].linked = false;
        Ok(())
    }

    /// Disjoint mutable pair.
    pub fn pair(&mut self, first: Slot, second: Slot) -> Q3GameItemsResult<(&mut GameEntity, &mut GameEntity)> {
        if first == second {
            return Err(invalid("entity pair slots must differ"));
        }
        if first >= MAX_GENTITIES || second >= MAX_GENTITIES {
            return Err(range("entity pair slot outside 1024"));
        }
        let (low, high) = if first < second {
            (first, second)
        } else {
            (second, first)
        };
        let (head, tail) = self.entities.split_at_mut(high);
        let low_ref = &mut head[low];
        let high_ref = &mut tail[0];
        if !low_ref.inuse || !high_ref.inuse {
            return Err(invalid("entity pair record was freed"));
        }
        if first < second {
            Ok((low_ref, high_ref))
        } else {
            Ok((high_ref, low_ref))
        }
    }

    /// Add an entity event (`addEvent`).
    pub fn add_event(&mut self, slot: Slot, event: EntityEvent, parameter: i32) -> Q3GameItemsResult<()> {
        self.require_owned(slot)?;
        if event == EntityEvent::EvNone {
            let number = self.entities[slot].s.number;
            self.prints
                .push(format!("G_AddEvent: zero event added for entity {number}\n"));
            return Ok(());
        }
        let now = self.time;
        let entity = &mut self.entities[slot];
        if let Some(client) = entity.client.as_mut() {
            let bits = ((client.ps.external_event & EV_EVENT_BITS) + EV_EVENT_BIT1) & EV_EVENT_BITS;
            client.ps.external_event = event as i32 | bits;
            client.ps.external_event_parm = parameter;
            client.ps.external_event_time = now;
        } else {
            let bits = ((entity.s.event & EV_EVENT_BITS) + EV_EVENT_BIT1) & EV_EVENT_BITS;
            entity.s.event = event as i32 | bits;
            entity.s.event_parm = parameter;
        }
        entity.event_time = now;
        self.events.push(FiredEvent {
            slot,
            event,
            parm: parameter,
            time: now,
        });
        Ok(())
    }

    /// Spawn a temporary event entity (`tempEntity`).
    pub fn temp_entity(
        &mut self,
        world: &mut dyn WorldOps,
        origin: Vec3,
        event: EntityEvent,
    ) -> Q3GameItemsResult<Slot> {
        let slot = self.spawn()?;
        {
            let entity = &mut self.entities[slot];
            entity.s.e_type = EntityType::EtEvents as i32 + event as i32;
            entity.classname = Some("tempEntity".to_string());
            entity.event_time = self.time;
            entity.free_after_event = true;
            let snapped = vec3(
                qvm_float_to_int(origin.x) as f32,
                qvm_float_to_int(origin.y) as f32,
                qvm_float_to_int(origin.z) as f32,
            );
            entity.s.pos = Trajectory {
                trajectory_type: TrajectoryType::TrStationary,
                time: 0,
                duration: 0,
                base: snapped,
                delta: vec3(0.0, 0.0, 0.0),
            };
            entity.r.current_origin = snapped;
        }
        world.link(self, slot)?;
        Ok(slot)
    }

    /// Format a vector (`vtos`).
    #[must_use]
    pub fn vtos(vector: Vec3) -> String {
        format!(
            "({} {} {})",
            qvm_float_to_int(vector.x),
            qvm_float_to_int(vector.y),
            qvm_float_to_int(vector.z)
        )
    }
}

/// Trace contact (`contact`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TraceContact {
    /// No contact.
    None,
    /// Plane contact.
    Plane {
        /// Plane normal.
        normal: Vec3,
    },
}

/// Trace hit identity (`hit`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TraceHit {
    /// No hit.
    None,
    /// World hit.
    World,
    /// Actor hit.
    Actor(ActorId),
}

/// Actor trace result (`ActorTraceResult`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ActorTraceResult {
    /// Fraction travelled.
    pub fraction: f32,
    /// End position.
    pub end: Vec3,
    /// Solidity.
    pub solidity: TraceSolidity,
    /// Contact.
    pub contact: TraceContact,
    /// Contents.
    pub contents: i32,
    /// Surface flags.
    pub surface_flags: i32,
    /// Hit identity.
    pub hit: TraceHit,
}

/// Server trace result (`ServerTraceResult`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ServerTraceResult {
    /// Fraction travelled.
    pub fraction: f32,
    /// End position.
    pub end: Vec3,
    /// Hit entity number.
    pub entity_num: i32,
    /// Solidity.
    pub solidity: TraceSolidity,
    /// Contact.
    pub contact: TraceContact,
    /// Contents.
    pub contents: i32,
    /// Surface flags.
    pub surface_flags: i32,
}

impl ServerTraceResult {
    /// Reinterpret as an actor trace against a resolved actor.
    #[must_use]
    pub fn with_actor_hit(self, actor: ActorId) -> ActorTraceResult {
        ActorTraceResult {
            fraction: self.fraction,
            end: self.end,
            solidity: self.solidity,
            contact: self.contact,
            contents: self.contents,
            surface_flags: self.surface_flags,
            hit: TraceHit::Actor(actor),
        }
    }
}

/// Actor trace query (`ActorTraceQuery`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ActorTraceQuery {
    /// Start.
    pub start: Vec3,
    /// End.
    pub end: Vec3,
    /// Shape.
    pub shape: TraceShape,
    /// Actor to ignore.
    pub pass_actor: Option<ActorId>,
    /// Contents mask.
    pub mask: i32,
}

/// Server world host (`ServerWorld` + `ActorSpatialQueries`).
pub trait WorldOps {
    /// Link an entity.
    fn link(&mut self, pool: &mut EntityPool, slot: Slot) -> Q3GameItemsResult<()>;
    /// Unlink an entity number.
    fn unlink(&mut self, pool: &mut EntityPool, number: i32) -> Q3GameItemsResult<()>;
    /// Trace an actor sweep.
    fn trace_actor(&mut self, pool: &EntityPool, query: &ActorTraceQuery) -> ActorTraceResult;
    /// Contents at a point.
    fn point_contents(&mut self, pool: &EntityPool, point: Vec3, pass_entity_num: i32) -> i32;
    /// Actors touching bounds.
    fn area_actors(&mut self, pool: &EntityPool, bounds: Bounds, maximum: usize) -> Vec<ActorId>;
}

/// Authority record for a damageable actor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AuthorityState {
    /// Can take damage.
    pub can_take_damage: bool,
    /// Health.
    pub health: i32,
    /// Team, when teamed.
    pub team: Option<Team>,
}

/// Accuracy-hit target description (`q3AccuracyHit` input).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AccuracyTarget {
    /// Actor.
    pub actor: ActorId,
    /// Damageable.
    pub damageable: bool,
    /// Player.
    pub player: bool,
    /// Health.
    pub health: i32,
    /// Team, when teamed.
    pub team: Option<Team>,
}

/// Combat host (`CombatContext` + damage/radius/accuracy surface).
pub trait CombatOps {
    /// Current time.
    fn time(&self) -> i32;
    /// Previous frame time.
    fn previous_time(&self) -> i32;
    /// Game type.
    fn game_type(&self) -> i32;
    /// Product.
    fn product(&self) -> Product;
    /// Spawn a temporary event entity.
    fn temp_entity(
        &mut self,
        pool: &mut EntityPool,
        world: &mut dyn WorldOps,
        origin: Vec3,
        event: EntityEvent,
    ) -> Q3GameItemsResult<Slot> {
        pool.temp_entity(world, origin, event)
    }
    /// Add an entity event.
    fn add_event(&mut self, pool: &mut EntityPool, slot: Slot, event: EntityEvent, parm: i32) -> Q3GameItemsResult<()> {
        pool.add_event(slot, event, parm)
    }
    /// Apply damage (`damage`).
    #[allow(clippy::too_many_arguments)]
    fn damage(
        &mut self,
        pool: &mut EntityPool,
        target: Slot,
        inflictor: DamageParticipant,
        attacker: DamageParticipant,
        direction: Option<Vec3>,
        point: Option<Vec3>,
        amount: i32,
        flags: i32,
        method: i32,
        projectile: Option<ActorId>,
    ) -> Q3GameItemsResult<()>;
    /// Line-of-damage check (`canDamage`).
    fn can_damage(&mut self, pool: &mut EntityPool, target: Slot, origin: Vec3) -> bool;
    /// Splash damage (`radiusDamage`).
    #[allow(clippy::too_many_arguments)]
    fn radius_damage(
        &mut self,
        pool: &mut EntityPool,
        origin: Vec3,
        attacker: Slot,
        damage: i32,
        radius: f32,
        ignore: Option<Slot>,
        method: i32,
        projectile: Option<ActorId>,
    ) -> bool;
    /// Accuracy attribution (`q3AccuracyHit`).
    fn accuracy_hit(&mut self, team_game: bool, target: &AccuracyTarget, attacker: &AccuracyTarget) -> bool;
    /// Whether an actor is a player.
    fn is_player(&self, pool: &EntityPool, actor: ActorId) -> bool {
        pool.native_by_actor(actor)
            .is_some_and(|slot| pool.get(slot).is_some_and(|entity| entity.client.is_some()))
    }
    /// Whether an actor is linked.
    fn linked_bounds(&self, pool: &EntityPool, actor: ActorId) -> bool {
        pool.native_by_actor(actor)
            .is_some_and(|slot| pool.get(slot).is_some_and(|entity| entity.linked))
    }
    /// Pooled participant for an actor.
    fn participant(&self, pool: &EntityPool, actor: ActorId) -> Option<Slot> {
        pool.native_by_actor(actor)
    }
    /// Authority record for an actor.
    fn authority(&self, pool: &EntityPool, actor: ActorId) -> Option<AuthorityState> {
        let slot = pool.native_by_actor(actor)?;
        let entity = pool.get(slot)?;
        Some(AuthorityState {
            can_take_damage: entity.takedamage,
            health: entity.health,
            team: entity.client.as_ref().map(|client| client.sess.session_team),
        })
    }
}

/// Game random host (`GameRandom`).
pub trait GameRandom {
    /// Nonnegative integer (`rand`).
    fn rand_int(&mut self) -> i32;
    /// Unit random (`random`).
    fn random(&mut self) -> f32;
    /// Signed unit random (`crandom`).
    fn crandom(&mut self) -> f32;
}

/// Mover core host (`MoverRuntime` surface used by mover-spawn).
pub trait MoverCore {
    /// Current time.
    fn time(&self) -> i32;
    /// Shared sound index.
    fn sound_index(&mut self, path: &str) -> i32;
    /// Use a binary mover (`useBinary`).
    fn use_binary(
        &mut self,
        pool: &mut EntityPool,
        entity: Slot,
        other: Option<DamageParticipant>,
        activator: Option<DamageParticipant>,
    ) -> Q3GameItemsResult<()>;
    /// Match a mover team to a state (`matchTeam`).
    fn match_team(
        &mut self,
        pool: &mut EntityPool,
        leader: Slot,
        state: MoverState,
        time: i32,
    ) -> Q3GameItemsResult<()>;
    /// Blocked-door handler (`blockedDoor`).
    fn blocked_door(&mut self, pool: &mut EntityPool, entity: Slot, other: DamageParticipant) -> Q3GameItemsResult<()>;
    /// Initialize a binary mover (`initializeBinary`).
    fn initialize_binary(
        &mut self,
        pool: &mut EntityPool,
        entity: Slot,
        variables: &SpawnVariables,
    ) -> Q3GameItemsResult<()>;
    /// Set mover state (`setState`).
    fn set_state(&mut self, pool: &mut EntityPool, entity: Slot, state: MoverState, time: i32)
        -> Q3GameItemsResult<()>;
}

/// Spawn variable value (`SpawnValue`).
#[derive(Debug, Clone, PartialEq)]
pub struct SpawnValue<T> {
    /// Whether the key was present.
    pub present: bool,
    /// Parsed value or parsed default.
    pub value: T,
}

/// Spawn variables (`SpawnVariables`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SpawnVariables {
    entries: Vec<(String, String)>,
}

impl SpawnVariables {
    /// Variables from key/value pairs.
    #[must_use]
    pub fn new(entries: Vec<(String, String)>) -> SpawnVariables {
        SpawnVariables { entries }
    }

    fn lookup(&self, key: &str) -> Option<&str> {
        let normalized = key.to_ascii_lowercase();
        self.entries
            .iter()
            .find(|pair| pair.0.to_ascii_lowercase() == normalized)
            .map(|pair| pair.1.as_str())
    }

    /// String value.
    #[must_use]
    pub fn string(&self, key: &str, default_value: &str) -> SpawnValue<String> {
        match self.lookup(key) {
            None => SpawnValue {
                present: false,
                value: default_value.to_string(),
            },
            Some(found) => SpawnValue {
                present: true,
                value: found.to_string(),
            },
        }
    }

    /// Integer value.
    #[must_use]
    pub fn int(&self, key: &str, default_value: &str) -> SpawnValue<i32> {
        let found = self.string(key, default_value);
        SpawnValue {
            present: found.present,
            value: game_atoi(&found.value),
        }
    }

    /// Float value.
    #[must_use]
    pub fn float(&self, key: &str, default_value: &str) -> SpawnValue<f32> {
        let found = self.string(key, default_value);
        SpawnValue {
            present: found.present,
            value: game_atof(&found.value),
        }
    }
}

/// Set an entity origin (`setOrigin`).
pub fn set_origin(pool: &mut EntityPool, slot: Slot, origin: Vec3) -> Q3GameItemsResult<()> {
    pool.require_owned(slot)?;
    let entity = &mut pool.entities[slot];
    entity.s.pos = Trajectory {
        trajectory_type: TrajectoryType::TrStationary,
        time: 0,
        duration: 0,
        base: origin,
        delta: vec3(0.0, 0.0, 0.0),
    };
    entity.r.current_origin = origin;
    Ok(())
}

/// Run a due think callback (`runThink`); dispatch runs the named callback.
pub fn run_think(
    pool: &mut EntityPool,
    slot: Slot,
    time: i32,
    dispatch: &mut dyn FnMut(&mut EntityPool, Slot, CallbackName) -> Q3GameItemsResult<()>,
) -> Q3GameItemsResult<()> {
    pool.require_owned(slot)?;
    let think_time = pool.entities[slot].nextthink;
    if think_time <= 0 || think_time > time {
        return Ok(());
    }
    pool.entities[slot].nextthink = 0;
    if let Some(name) = pool.entities[slot].think {
        dispatch(pool, slot, name)?;
    }
    Ok(())
}

/// Ground entity number (`groundNumber`).
#[must_use]
pub fn ground_number(ground: Option<ActorId>, pool: &EntityPool) -> i32 {
    match ground.and_then(|actor| pool.native_by_actor(actor)) {
        Some(slot) => slot as i32,
        None => ENTITYNUM_NONE,
    }
}

/// Record trace ground (`traceGround`).
pub fn trace_ground(pool: &mut EntityPool, slot: Slot, hit: TraceHit) -> Q3GameItemsResult<()> {
    pool.require_owned(slot)?;
    let ground = match hit {
        TraceHit::Actor(actor) => Some(actor),
        TraceHit::World => Some(ActorId::from_slot(ENTITYNUM_WORLD as usize)),
        TraceHit::None => None,
    };
    let number = ground_number(ground, pool);
    let entity = &mut pool.entities[slot];
    entity.ground = ground;
    entity.s.ground_entity_num = number;
    Ok(())
}

pub(crate) fn ascii_fold(value: &str) -> String {
    let cut = value.find('\0').map_or(value, |index| &value[..index]);
    cut.bytes()
        .map(|byte| {
            if byte.is_ascii_uppercase() {
                (byte + 32) as char
            } else {
                byte as char
            }
        })
        .collect()
}

pub(crate) fn string_field(entity: &GameEntity, field: EntityStringField) -> Option<&str> {
    match field {
        EntityStringField::Classname => entity.classname.as_deref(),
        EntityStringField::Target => entity.target.as_deref(),
        EntityStringField::Targetname => entity.targetname.as_deref(),
        EntityStringField::Model => entity.model.as_deref(),
        EntityStringField::Model2
        | EntityStringField::Message
        | EntityStringField::Team
        | EntityStringField::TargetShaderName
        | EntityStringField::TargetShaderNewName => None,
    }
}

/// Find an entity by string field (`findEntity`).
#[must_use]
pub fn find_entity(
    pool: &EntityPool,
    after: Option<Slot>,
    field: EntityStringField,
    value: Option<&str>,
) -> Option<Slot> {
    let wanted = ascii_fold(value?);
    let start = after.map_or(0, |slot| slot + 1);
    for index in start..pool.num_entities() {
        let Ok(entity) = pool.at(index) else {
            continue;
        };
        if !entity.inuse {
            continue;
        }
        if let Some(text) = string_field(entity, field) {
            if ascii_fold(text) == wanted {
                return Some(index);
            }
        }
    }
    None
}

/// Target selection services (`TargetSelectionContext` pool-free half).
pub struct TargetSelection<'a> {
    /// Nonnegative random integer.
    pub random_int: &'a mut dyn FnMut() -> i32,
    /// Warning sink.
    pub warn: &'a mut dyn FnMut(&str),
}

/// Pick a target by targetname (`pickTarget`).
pub fn pick_target(
    pool: &EntityPool,
    selection: &mut TargetSelection<'_>,
    target_name: Option<&str>,
) -> Q3GameItemsResult<Option<Slot>> {
    let Some(target_name) = target_name else {
        (selection.warn)("G_PickTarget called with NULL targetname\n");
        return Ok(None);
    };
    let mut choices = Vec::new();
    let mut found = None;
    while choices.len() < 32 {
        found = find_entity(pool, found, EntityStringField::Targetname, Some(target_name));
        let Some(slot) = found else {
            break;
        };
        choices.push(slot);
    }
    if choices.is_empty() {
        (selection.warn)(&format!("G_PickTarget: target {target_name} not found\n"));
        return Ok(None);
    }
    let random = (selection.random_int)();
    if random < 0 {
        return Err(range("game rand must return a nonnegative integer"));
    }
    let choice = choices[(random as usize) % choices.len()];
    Ok(Some(choice))
}

/// Publish player state to entity state (`playerStateToEntityState`).
pub fn player_state_to_entity_state(pool: &mut EntityPool, slot: Slot, snap: bool) -> Q3GameItemsResult<()> {
    pool.require_owned(slot)?;
    let entity = &mut pool.entities[slot];
    let (client_slot, s) = (&mut entity.client, &mut entity.s);
    let Some(client) = client_slot.as_mut() else {
        return Err(invalid("player state sync requires a client entity"));
    };
    let ps = &mut client.ps;
    s.e_type =
        if ps.pm_type == MoveType::PmIntermission || ps.pm_type == MoveType::PmSpectator || ps.health <= GIB_HEALTH {
            EntityType::EtInvisible as i32
        } else {
            EntityType::EtPlayer as i32
        };
    s.number = ps.client_num;
    s.pos = Trajectory {
        trajectory_type: TrajectoryType::TrInterpolate,
        time: s.pos.time,
        duration: s.pos.duration,
        base: copy_position(ps.origin, snap),
        delta: ps.velocity,
    };
    s.apos = Trajectory {
        trajectory_type: TrajectoryType::TrInterpolate,
        time: s.apos.time,
        duration: s.apos.duration,
        base: copy_position(ps.viewangles, snap),
        delta: s.apos.delta,
    };
    s.angles2 = vec3(s.angles2.x, ps.movement_dir, s.angles2.z);
    s.legs_anim = ps.legs_anim;
    s.torso_anim = ps.torso_anim;
    s.client_num = ps.client_num;
    s.e_flags = if ps.health <= 0 {
        ps.e_flags | 1
    } else {
        ps.e_flags & !1
    };
    if ps.external_event != 0 {
        s.event = ps.external_event;
        s.event_parm = ps.external_event_parm;
    } else if ps.entity_event_sequence < ps.event_sequence {
        let oldest = ps.event_sequence.wrapping_sub(2);
        if ps.entity_event_sequence < oldest {
            ps.entity_event_sequence = oldest;
        }
        let index = (ps.entity_event_sequence & 1) as usize;
        s.event = ps.events.get(index)? | ((ps.entity_event_sequence & 3) << 8);
        s.event_parm = ps.event_parms.get(index)?;
        ps.entity_event_sequence = ps.entity_event_sequence.wrapping_add(1);
    }
    s.weapon = ps.weapon;
    s.ground_entity_num = ps.ground_entity_num;
    s.powerups = 0;
    for index in 0..ps.powerups.len() {
        if ps.powerups.get(index)? != 0 {
            s.powerups |= 1 << index;
        }
    }
    s.loop_sound = ps.loop_sound;
    s.generic1 = ps.generic1;
    Ok(())
}

/// Set a client view angle (`setClientViewAngle`).
pub fn set_client_view_angle(pool: &mut EntityPool, slot: Slot, angles: Vec3) -> Q3GameItemsResult<()> {
    pool.require_owned(slot)?;
    let delta = |angle: f32, command: i32| -> i32 {
        let scaled = angle * 65536.0 / 360.0;
        let short = qvm_float_to_int(scaled) & 65535;
        short.wrapping_sub(command)
    };
    let entity = &mut pool.entities[slot];
    let Some(client) = entity.client.as_mut() else {
        return Err(invalid("view angle update requires a client entity"));
    };
    let command = client.pers.cmd_angles;
    client.ps.delta_angles = [
        delta(angles.x, command[0]),
        delta(angles.y, command[1]),
        delta(angles.z, command[2]),
    ];
    entity.s.angles = angles;
    if let Some(client) = entity.client.as_mut() {
        client.ps.viewangles = entity.s.angles;
    }
    Ok(())
}

#[cfg(test)]
pub(crate) mod test_support {
    use qa_core::math::{vec3, Bounds, Vec3};

    use super::*;
    use crate::q3::base::shared::definitions::{GameType, Holdable, Powerup, Product, Weapon};
    use crate::q3::base::world::TraceSolidity;

    pub(crate) fn item_def(class_name: &str, quantity: i32, kind: ItemKind) -> ItemDefinition {
        ItemDefinition {
            class_name: Some(class_name.to_string()),
            quantity,
            kind,
        }
    }

    pub(crate) struct TestItems {
        pub(crate) list: Vec<ItemDefinition>,
    }

    pub(crate) fn test_items() -> TestItems {
        TestItems {
            list: vec![
                ItemDefinition {
                    class_name: None,
                    quantity: 0,
                    kind: ItemKind::Bad,
                },
                item_def("weapon_rocketlauncher", 10, ItemKind::Weapon(Weapon::WpRocketLauncher)),
                item_def("weapon_plasmagun", 50, ItemKind::Weapon(Weapon::WpPlasmagun)),
                item_def("weapon_grenadelauncher", 5, ItemKind::Weapon(Weapon::WpGrenadeLauncher)),
                item_def("ammo_rockets", 5, ItemKind::Ammo(Weapon::WpRocketLauncher)),
                item_def("item_armor_shard", 5, ItemKind::Armor),
                item_def("item_health_small", 5, ItemKind::Health),
                item_def("item_health_mega", 100, ItemKind::Health),
                item_def("item_quad", 30, ItemKind::Powerup(Powerup::PwQuad)),
                item_def("holdable_kamikaze", 1, ItemKind::Holdable(Holdable::HiKamikaze)),
                item_def("item_guard", 1, ItemKind::PersistantPowerup(Powerup::PwGuard)),
                item_def("team_CTF_redflag", 0, ItemKind::Team(Powerup::PwRedflag)),
            ],
        }
    }

    impl ItemTable for TestItems {
        fn index_of(&self, _product: Product, item: &ItemDefinition) -> Option<usize> {
            self.list.iter().position(|candidate| candidate == item)
        }

        fn item_at(&self, _product: Product, index: usize) -> Q3GameItemsResult<ItemDefinition> {
            self.list
                .get(index)
                .cloned()
                .ok_or_else(|| range(format!("item index {index} out of range")))
        }

        fn find_item_for_weapon(&self, _product: Product, weapon: Weapon) -> Q3GameItemsResult<ItemDefinition> {
            self.list
                .iter()
                .find(|item| item.kind == ItemKind::Weapon(weapon))
                .cloned()
                .ok_or_else(|| drop_error(format!("couldn't find item for weapon {}", weapon as i32)))
        }
    }

    pub(crate) struct DamageCall {
        pub(crate) amount: i32,
        pub(crate) flags: i32,
        pub(crate) method: i32,
    }

    pub(crate) struct TestCombat {
        pub(crate) time: i32,
        pub(crate) previous_time: i32,
        pub(crate) game_type: i32,
        pub(crate) product: Product,
        pub(crate) damage_calls: Vec<DamageCall>,
        pub(crate) can_damage_value: bool,
        pub(crate) radius_value: bool,
        pub(crate) accuracy_value: bool,
    }

    impl TestCombat {
        pub(crate) fn new() -> TestCombat {
            TestCombat {
                time: 1000,
                previous_time: 900,
                game_type: GameType::GtFfa as i32,
                product: Product::Baseq3,
                damage_calls: Vec::new(),
                can_damage_value: true,
                radius_value: true,
                accuracy_value: true,
            }
        }
    }

    impl CombatOps for TestCombat {
        fn time(&self) -> i32 {
            self.time
        }

        fn previous_time(&self) -> i32 {
            self.previous_time
        }

        fn game_type(&self) -> i32 {
            self.game_type
        }

        fn product(&self) -> Product {
            self.product
        }

        fn damage(
            &mut self,
            _pool: &mut EntityPool,
            _target: Slot,
            _inflictor: DamageParticipant,
            _attacker: DamageParticipant,
            _direction: Option<Vec3>,
            _point: Option<Vec3>,
            amount: i32,
            flags: i32,
            method: i32,
            _projectile: Option<ActorId>,
        ) -> Q3GameItemsResult<()> {
            self.damage_calls.push(DamageCall { amount, flags, method });
            Ok(())
        }

        fn can_damage(&mut self, _pool: &mut EntityPool, _target: Slot, _origin: Vec3) -> bool {
            self.can_damage_value
        }

        fn radius_damage(
            &mut self,
            _pool: &mut EntityPool,
            _origin: Vec3,
            _attacker: Slot,
            _damage: i32,
            _radius: f32,
            _ignore: Option<Slot>,
            _method: i32,
            _projectile: Option<ActorId>,
        ) -> bool {
            self.radius_value
        }

        fn accuracy_hit(&mut self, _team_game: bool, _target: &AccuracyTarget, _attacker: &AccuracyTarget) -> bool {
            self.accuracy_value
        }
    }

    pub(crate) struct TestWorld {
        pub(crate) trace_result: ActorTraceResult,
        pub(crate) contents: i32,
        pub(crate) actors: Vec<ActorId>,
        pub(crate) link_log: Vec<Slot>,
        pub(crate) unlink_log: Vec<i32>,
    }

    impl TestWorld {
        pub(crate) fn clear_trace() -> ActorTraceResult {
            ActorTraceResult {
                fraction: 1.0,
                end: vec3(10.0, 20.0, 30.0),
                solidity: TraceSolidity::Clear,
                contact: TraceContact::None,
                contents: 0,
                surface_flags: 0,
                hit: TraceHit::None,
            }
        }

        pub(crate) fn new() -> TestWorld {
            TestWorld {
                trace_result: TestWorld::clear_trace(),
                contents: 0,
                actors: Vec::new(),
                link_log: Vec::new(),
                unlink_log: Vec::new(),
            }
        }
    }

    impl WorldOps for TestWorld {
        fn link(&mut self, pool: &mut EntityPool, slot: Slot) -> Q3GameItemsResult<()> {
            pool.link(slot)?;
            self.link_log.push(slot);
            Ok(())
        }

        fn unlink(&mut self, pool: &mut EntityPool, number: i32) -> Q3GameItemsResult<()> {
            if number >= 0 {
                pool.unlink(number as usize)?;
            }
            self.unlink_log.push(number);
            Ok(())
        }

        fn trace_actor(&mut self, _pool: &EntityPool, _query: &ActorTraceQuery) -> ActorTraceResult {
            self.trace_result
        }

        fn point_contents(&mut self, _pool: &EntityPool, _point: Vec3, _pass: i32) -> i32 {
            self.contents
        }

        fn area_actors(&mut self, _pool: &EntityPool, _bounds: Bounds, maximum: usize) -> Vec<ActorId> {
            self.actors.iter().take(maximum).copied().collect()
        }
    }

    pub(crate) struct TestRandom {
        pub(crate) int_value: i32,
        pub(crate) random_value: f32,
        pub(crate) crandom_value: f32,
    }

    impl GameRandom for TestRandom {
        fn rand_int(&mut self) -> i32 {
            self.int_value
        }

        fn random(&mut self) -> f32 {
            self.random_value
        }

        fn crandom(&mut self) -> f32 {
            self.crandom_value
        }
    }

    pub(crate) struct TestRegistry {
        pub(crate) product: Product,
        pub(crate) registered: Vec<ItemDefinition>,
    }

    impl ItemRegistry for TestRegistry {
        fn product(&self) -> Product {
            self.product
        }

        fn register(&mut self, item: &ItemDefinition) {
            self.registered.push(item.clone());
        }
    }

    pub(crate) fn player_slot(pool: &mut EntityPool, product: Product) -> Slot {
        let slot = pool.spawn().unwrap();
        pool.at_mut(slot).unwrap().client = Some(GameClient::new(product));
        pool.at_mut(slot).unwrap().health = 100;
        pool.at_mut(slot).unwrap().takedamage = true;
        slot
    }

    pub(crate) fn item_slot(pool: &mut EntityPool, items: &TestItems, index: usize) -> Slot {
        let slot = pool.spawn().unwrap();
        pool.at_mut(slot).unwrap().item = Some(items.list[index].clone());
        slot
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::*;
    use super::*;
    use crate::q3::base::game::item_motion::{bind_launch_save_callbacks, LAUNCH_ITEM_THINK};
    use crate::q3::base::game::utilities::EntityStringField;
    use crate::q3::base::shared::definitions::{EntityEvent, EntityType, Powerup, Product};

    #[test]
    fn game_number_parsing() {
        assert_eq!(game_atoi("  -12x"), -12);
        assert_eq!(game_atoi("+7"), 7);
        assert_eq!(game_atoi(""), 0);
        assert_eq!(game_atoi("abc"), 0);
        assert_eq!(game_atoi("9999999999"), 1_410_065_407);
        assert_eq!(game_atof("3.5"), 3.5);
        assert_eq!(game_atof(".5"), 0.5);
        assert_eq!(game_atof("-2.25 ignored"), -2.25);
        assert_eq!(game_atof(""), 0.0);
        assert_eq!(game_atof("10"), 10.0);
    }

    #[test]
    fn pool_events_think_and_targets() {
        let mut pool = EntityPool::new(Product::Baseq3);
        let slot = pool.spawn().unwrap();
        pool.add_event(slot, EntityEvent::EvNone, 0).unwrap();
        assert_eq!(pool.prints.len(), 1);
        pool.time = 777;
        pool.add_event(slot, EntityEvent::EvFireWeapon, 3).unwrap();
        assert_eq!(pool.at(slot).unwrap().s.event & 0xff, EntityEvent::EvFireWeapon as i32);
        assert_eq!(pool.at(slot).unwrap().event_time, 777);
        let mut world = TestWorld::new();
        let temp = pool
            .temp_entity(&mut world, vec3(1.9, 2.1, 3.5), EntityEvent::EvJump)
            .unwrap();
        assert_eq!(
            pool.at(temp).unwrap().s.e_type,
            EntityType::EtEvents as i32 + EntityEvent::EvJump as i32
        );
        assert_eq!(pool.at(temp).unwrap().r.current_origin, vec3(1.0, 2.0, 3.0));
        // Think dispatch clears the schedule first.
        bind_launch_save_callbacks(&mut pool);
        pool.at_mut(slot).unwrap().think = Some(CallbackName(LAUNCH_ITEM_THINK));
        pool.at_mut(slot).unwrap().nextthink = 100;
        let mut ran = Vec::new();
        run_think(&mut pool, slot, 1000, &mut |_pool, slot, name| {
            ran.push((slot, name));
            Ok(())
        })
        .unwrap();
        assert_eq!(pool.at(slot).unwrap().nextthink, 0);
        assert_eq!(ran.len(), 1);
        // Target selection warns and caps.
        let target = pool.spawn().unwrap();
        pool.at_mut(target).unwrap().targetname = Some("goal".to_string());
        let mut warnings = Vec::new();
        let mut rand = || 0;
        let mut selection = TargetSelection {
            random_int: &mut rand,
            warn: &mut |text: &str| warnings.push(text.to_string()),
        };
        assert_eq!(pick_target(&pool, &mut selection, Some("goal")).unwrap(), Some(target));
        assert_eq!(pick_target(&pool, &mut selection, None).unwrap(), None);
        assert_eq!(pick_target(&pool, &mut selection, Some("missing")).unwrap(), None);
        assert_eq!(warnings.len(), 2);
        assert_eq!(
            find_entity(&pool, None, EntityStringField::Targetname, Some("GOAL")),
            Some(target)
        );
        assert_eq!(EntityPool::vtos(vec3(1.9, -2.1, 0.0)), "(1 -2 0)");
        assert!(pool.think_cbs.resolve("unknown").is_err());
    }

    #[test]
    fn snapshot_sync_and_view_angles() {
        let mut pool = EntityPool::new(Product::Baseq3);
        let player = player_slot(&mut pool, Product::Baseq3);
        pool.at_mut(player).unwrap().client.as_mut().unwrap().ps.health = 100;
        pool.at_mut(player).unwrap().client.as_mut().unwrap().ps.origin = vec3(1.9, 2.1, 3.5);
        pool.at_mut(player)
            .unwrap()
            .client
            .as_mut()
            .unwrap()
            .ps
            .powerups
            .set(Powerup::PwQuad as usize, 5)
            .unwrap();
        player_state_to_entity_state(&mut pool, player, true).unwrap();
        assert_eq!(pool.at(player).unwrap().s.e_type, EntityType::EtPlayer as i32);
        assert_eq!(pool.at(player).unwrap().s.pos.base, vec3(1.0, 2.0, 3.0));
        assert_eq!(pool.at(player).unwrap().s.powerups, 1 << Powerup::PwQuad as i32);
        set_client_view_angle(&mut pool, player, vec3(0.0, 90.0, 0.0)).unwrap();
        let client = pool.at(player).unwrap().client.as_ref().unwrap();
        assert_eq!(client.ps.delta_angles[1], 16384);
        assert_eq!(client.ps.viewangles, vec3(0.0, 90.0, 0.0));
        pool.at_mut(player).unwrap().client.as_mut().unwrap().ps.health = -50;
        player_state_to_entity_state(&mut pool, player, false).unwrap();
        assert_eq!(pool.at(player).unwrap().s.e_type, EntityType::EtInvisible as i32);
    }
}
