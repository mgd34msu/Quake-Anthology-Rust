//! Quake III base/game: entities.
//!
//! Donor provenance: `src/content/q3/base/game/entities.ts`.

use qa_core::identity::{ActorId, IdentityOwner};
use qa_core::math::{vec3, Plane, Vec3};
use qa_core::numeric::{q_rand, qvm_float_to_int};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

// Intra-group imports: sibling modules split from the same flat port.
use crate::q3::base::game::format::*;
use crate::q3::base::game::rankings::Q3RankingReports;
use crate::q3::base::game::state::{ConnectionState, MoverState, SpectatorState, MAX_CLIENTS, MAX_GENTITIES};
use crate::q3::base::shared::definitions::{
    stat_schema, weapon_count, EntityType, GameType, ItemType, MissionpackStatIndex, MoveType, PersistentIndex,
    Powerup, Product, StatSchema, Team, Weapon, WeaponState, EVENT_VALID_MSEC, EV_EVENT_BIT1, EV_EVENT_BITS,
};
use crate::q3::base::shared::entity_state::EntityState;
use crate::q3::base::shared::player_state::{EventDebugModule, PredictableEvent, ENTITYNUM_NONE, ENTITYNUM_WORLD};
use crate::q3::base::shared::trajectory::{Trajectory, TrajectoryType};
use crate::q3::base::world::{ActorTraceQuery, ActorTraceResult, ServerTraceQuery, ServerTraceResult};

// ---------------------------------------------------------------------------
// Entity pool (entities.ts).
// ---------------------------------------------------------------------------

/// Event-lifetime status (`EntityEventStatus`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntityEventStatus {
    /// Not in use.
    Inactive,
    /// Active.
    Active,
    /// Waiting for event expiry.
    Waiting,
    /// Freed after event.
    Freed,
}

/// Entity pool construction options (`EntityPoolOptions`).
#[derive(Clone)]
pub struct EntityPoolOptions {
    /// Product.
    pub product: Product,
    /// Configured clients.
    pub max_clients: usize,
    /// Map start time milliseconds.
    pub map_start_time: i32,
    /// Current time milliseconds.
    pub time: Rc<dyn Fn() -> i32>,
    /// Print sink.
    pub print: Rc<dyn Fn(String)>,
    /// Collision link.
    pub link: Rc<dyn Fn(EntityRef)>,
    /// Collision unlink.
    pub unlink: Rc<dyn Fn(EntityRef)>,
    /// Predictable-event debug sink.
    pub event_debug: Option<EventDebugMirror>,
}

/// Loaded-module entity storage (`EntityPool`).
pub struct EntityPool {
    /// Construction options.
    pub options: EntityPoolOptions,
    /// Ranking reports sink.
    pub rankings: RefCell<Q3RankingReports>,
    /// Native-callback catalog.
    pub callbacks: RefCell<Q3CallbackCatalog>,
    /// Utility scratch rings.
    pub utilities: RefCell<GameUtilityScratch>,
    /// Fixed slot records.
    entities: Vec<EntityRef>,
    /// Open entity count.
    num_entities: usize,
    /// Configured client count.
    max_clients: usize,
}

impl EntityPool {
    /// Open a pool (`new EntityPool(options)`).
    #[must_use]
    pub fn open(options: EntityPoolOptions) -> Self {
        if options.max_clients < 1 || options.max_clients > MAX_CLIENTS {
            panic!("Configured clients must be within 1..64");
        }
        let identity = IdentityOwner::create("q3-game-sim").expect("session identity needs a name");
        let entities: Vec<EntityRef> = (0..MAX_GENTITIES)
            .map(|slot| {
                Rc::new(RefCell::new(GameEntity::new(
                    slot,
                    OwnedActor {
                        id: identity.actor(slot as u32, 0),
                        owner: "q3:game".to_string(),
                    },
                )))
            })
            .collect();
        let mut pool = Self {
            utilities: RefCell::new(GameUtilityScratch::new(options.print.clone())),
            options,
            rankings: RefCell::new(Q3RankingReports::new()),
            callbacks: RefCell::new(Q3CallbackCatalog::new()),
            entities,
            num_entities: MAX_CLIENTS,
            max_clients: 0,
        };
        pool.max_clients = pool.options.max_clients;
        for index in 0..pool.max_clients {
            let mut entity = pool.entities[index].borrow_mut();
            let mut client = GameClient::new(pool.options.product);
            if let Some(debug) = pool.options.event_debug.clone() {
                client.ps.set_event_debug(Some(debug));
            }
            entity.client = Some(client);
        }
        pool
    }

    /// Open entity count (`numEntities`).
    #[must_use]
    pub fn num_entities(&self) -> usize {
        self.num_entities
    }

    /// Configured client count (`maxClients`).
    #[must_use]
    pub fn max_clients(&self) -> usize {
        self.max_clients
    }

    /// Restore saved counts (`restoreCounts`).
    pub fn restore_counts(&mut self, num_entities: usize, max_clients: usize) {
        if num_entities < MAX_CLIENTS
            || num_entities > ENTITYNUM_WORLD as usize
            || !(1..=MAX_CLIENTS).contains(&max_clients)
        {
            panic!("Invalid restored Q3 pool counts");
        }
        self.num_entities = num_entities;
        self.max_clients = max_clients;
    }

    /// Zero level counts (`clearLevel`).
    pub fn clear_level(&mut self) {
        self.num_entities = 0;
        self.max_clients = 0;
    }

    /// Release every in-use entity (`clearEntities`).
    pub fn clear_entities(&self) {
        for slot in 0..MAX_GENTITIES {
            if self.entities[slot].borrow().inuse {
                self.entities[slot].borrow_mut().reset();
            }
        }
    }

    /// Reset client records (`initializeClients`).
    pub fn initialize_clients(&mut self, max_clients: usize) {
        if !(1..=MAX_CLIENTS).contains(&max_clients) {
            panic!("Configured clients must be within 1..64");
        }
        self.max_clients = max_clients;
        for index in 0..MAX_CLIENTS {
            let mut entity = self.entities[index].borrow_mut();
            if index < max_clients {
                let mut client = GameClient::new(self.options.product);
                if let Some(debug) = self.options.event_debug.clone() {
                    client.ps.set_event_debug(Some(debug));
                }
                entity.client = Some(client);
            } else {
                entity.client = None;
            }
        }
        self.num_entities = MAX_CLIENTS;
    }

    /// Activate a client slot (`activateClient`).
    pub fn activate_client(&self, slot: i32) -> EntityRef {
        if slot < 0 || slot as usize >= self.max_clients {
            panic!("Client slot outside configured clients");
        }
        let entity = self.entities[slot as usize].clone();
        let mut client = GameClient::new(self.options.product);
        if let Some(debug) = self.options.event_debug.clone() {
            client.ps.set_event_debug(Some(debug));
        }
        entity.borrow_mut().client = Some(client);
        entity
    }

    /// Deactivate a client slot (`deactivateClient`).
    pub fn deactivate_client(&self, slot: i32) {
        self.at(slot).borrow_mut().client = None;
    }

    /// Entity record by slot (`get`).
    #[must_use]
    pub fn get(&self, number: i32) -> Option<EntityRef> {
        if number < 0 || number as usize >= MAX_GENTITIES {
            return None;
        }
        Some(self.entities[number as usize].clone())
    }

    /// Entity record by slot, panicking when unavailable (`at`).
    #[must_use]
    pub fn at(&self, number: i32) -> EntityRef {
        self.get(number)
            .unwrap_or_else(|| panic!("Game entity {number} is unavailable"))
    }

    /// Client record by slot (`clientAt`).
    #[must_use]
    pub fn client_at(&self, number: i32) -> GameClient {
        if number < 0 || number as usize >= MAX_CLIENTS {
            panic!("Game client {number} is unavailable");
        }
        self.entities[number as usize]
            .borrow()
            .client
            .clone()
            .unwrap_or_else(|| panic!("Game client {number} is unavailable"))
    }

    /// Entity by actor handle (`records.nativeByActor`).
    #[must_use]
    pub fn native_by_actor(&self, actor: &ActorId) -> Option<EntityRef> {
        self.entities
            .iter()
            .find(|entity| entity.borrow().actor.id == *actor)
            .cloned()
    }

    /// Panic unless the entity belongs to this pool (`owned`).
    fn owned(&self, entity: &EntityRef) {
        let slot = entity.borrow().slot;
        if slot >= MAX_GENTITIES || !Rc::ptr_eq(&self.entities[slot], entity) {
            panic!("Game entity does not belong to this pool");
        }
    }

    /// Spawn an entity (`spawn`, `G_Spawn`).
    pub fn spawn(&mut self) -> EntityRef {
        let now = (self.options.time)();
        for index in MAX_CLIENTS..self.num_entities {
            let entity = self.entities[index].clone();
            if entity.borrow().inuse {
                continue;
            }
            let freetime = entity.borrow().freetime;
            if freetime > self.options.map_start_time.wrapping_add(2000) && now.wrapping_sub(freetime) < 1000 {
                continue;
            }
            {
                let mut borrowed = entity.borrow_mut();
                borrowed.reset();
                borrowed.inuse = true;
            }
            init_game_entity(&entity);
            return entity;
        }
        if self.num_entities == ENTITYNUM_WORLD as usize {
            for index in 0..MAX_GENTITIES {
                let classname = self.entities[index].borrow().classname.clone();
                (self.options.print)(game_format(
                    "%4i: %s\n",
                    &[
                        GameFormatArgument::Int(index as i32),
                        classname.map_or(GameFormatArgument::Null, GameFormatArgument::Text),
                    ],
                ));
            }
            panic!("G_Spawn: no free entities");
        }
        let slot = self.num_entities;
        self.num_entities += 1;
        let entity = self.entities[slot].clone();
        {
            let mut borrowed = entity.borrow_mut();
            borrowed.reset();
            borrowed.inuse = true;
        }
        init_game_entity(&entity);
        entity
    }

    /// True when a non-client slot is free (`entitiesFree`).
    #[must_use]
    pub fn entities_free(&self) -> bool {
        (MAX_CLIENTS..self.num_entities).any(|index| !self.entities[index].borrow().inuse)
    }

    /// Free an entity (`free`, `G_FreeEntity`).
    pub fn free(&self, entity: &EntityRef) {
        self.owned(entity);
        (self.options.unlink)(entity.clone());
        if entity.borrow().never_free {
            return;
        }
        let now = (self.options.time)();
        let mut borrowed = entity.borrow_mut();
        borrowed.reset();
        borrowed.classname = Some("freed".to_string());
        borrowed.freetime = now;
    }

    /// Spawn a temporary event entity (`tempEntity`, `G_TempEntity`).
    pub fn temp_entity(&mut self, origin: Vec3, event: i32) -> EntityRef {
        let entity = self.spawn();
        {
            let mut borrowed = entity.borrow_mut();
            borrowed.s.e_type = EntityType::EtEvents as i32 + event;
            borrowed.classname = Some("tempEntity".to_string());
            borrowed.event_time = (self.options.time)();
            borrowed.free_after_event = true;
        }
        set_origin(
            &entity,
            vec3(
                qvm_float_to_int(origin.x) as f32,
                qvm_float_to_int(origin.y) as f32,
                qvm_float_to_int(origin.z) as f32,
            ),
        );
        (self.options.link)(entity.clone());
        entity
    }

    /// Queue a predictable event on a client entity (`addPredictableEvent`).
    pub fn add_predictable_event(&self, entity: &EntityRef, event: i32, parameter: i32) -> Option<PredictableEvent> {
        self.owned(entity);
        let mut borrowed = entity.borrow_mut();
        borrowed
            .client
            .as_mut()
            .map(|client| client.ps.add_event(event, parameter))
    }

    /// Queue an event on any entity (`addEvent`, `G_AddEvent`).
    pub fn add_event(&self, entity: &EntityRef, event: i32, parameter: i32) {
        self.owned(entity);
        if event == 0 {
            let number = entity.borrow().s.number;
            (self.options.print)(game_format(
                "G_AddEvent: zero event added for entity %i\n",
                &[GameFormatArgument::Int(number)],
            ));
            return;
        }
        let now = (self.options.time)();
        let mut borrowed = entity.borrow_mut();
        match borrowed.client.as_mut() {
            Some(client) => {
                let bits = ((client.ps.external_event & EV_EVENT_BITS) + EV_EVENT_BIT1) & EV_EVENT_BITS;
                client.ps.external_event = event | bits;
                client.ps.external_event_parm = parameter;
                client.ps.external_event_time = now;
            }
            None => {
                let bits = ((borrowed.s.event & EV_EVENT_BITS) + EV_EVENT_BIT1) & EV_EVENT_BITS;
                borrowed.s.event = event | bits;
                borrowed.s.event_parm = parameter;
            }
        }
        borrowed.event_time = now;
    }

    /// Expire entity events for a frame (`expireEvents`).
    pub fn expire_events(&self, entity: &EntityRef) -> EntityEventStatus {
        self.owned(entity);
        if !entity.borrow().inuse {
            return EntityEventStatus::Inactive;
        }
        let now = (self.options.time)();
        if now.wrapping_sub(entity.borrow().event_time) > EVENT_VALID_MSEC {
            {
                let mut borrowed = entity.borrow_mut();
                if borrowed.s.event != 0 {
                    borrowed.s.event = 0;
                    if let Some(client) = borrowed.client.as_mut() {
                        client.ps.external_event = 0;
                    }
                }
            }
            if entity.borrow().free_after_event {
                self.free(entity);
                return if entity.borrow().inuse {
                    EntityEventStatus::Waiting
                } else {
                    EntityEventStatus::Freed
                };
            }
            if entity.borrow().unlink_after_event {
                entity.borrow_mut().unlink_after_event = false;
                (self.options.unlink)(entity.clone());
            }
        }
        if entity.borrow().free_after_event {
            EntityEventStatus::Waiting
        } else {
            EntityEventStatus::Active
        }
    }
}

/// Initialize the four `G_InitGentity` fields (`initGameEntity`).
pub fn init_game_entity(entity: &EntityRef) {
    let mut borrowed = entity.borrow_mut();
    let slot = borrowed.slot as i32;
    borrowed.classname = Some("noclass".to_string());
    borrowed.s.number = slot;
    borrowed.r.owner_num = ENTITYNUM_NONE;
}

/// Store a fixed origin trajectory (`setOrigin`, `G_SetOrigin`).
pub fn set_origin(entity: &EntityRef, origin: Vec3) {
    let mut borrowed = entity.borrow_mut();
    borrowed.s.pos = Trajectory {
        trajectory_type: TrajectoryType::TrStationary,
        time: 0,
        duration: 0,
        base: vec3(origin.x, origin.y, origin.z),
        delta: vec3(0.0, 0.0, 0.0),
    };
    borrowed.r.current_origin = vec3(origin.x, origin.y, origin.z);
}

/// Run scheduled think (`runThink`, `G_RunThink`).
pub fn run_think(entity: &EntityRef, time: i32) {
    let think_time = entity.borrow().nextthink;
    if think_time <= 0 || think_time > time {
        return;
    }
    let think = entity.borrow().think.clone();
    entity.borrow_mut().nextthink = 0;
    if let Some(think) = think {
        think(entity.clone());
    }
}

/// String-valued entity search field (`EntityStringField`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntityStringField {
    /// Class name.
    Classname,
    /// Target name.
    Targetname,
    /// Target.
    Target,
    /// Team.
    Team,
}

/// Case-insensitive entity search (`findEntity`, `G_Find`).
#[must_use]
pub fn find_entity(
    pool: &EntityPool,
    after: Option<&EntityRef>,
    field: EntityStringField,
    text: &str,
) -> Option<EntityRef> {
    let fold = |value: &str| -> String {
        let end = value.find('\0').unwrap_or(value.len());
        value[..end].to_ascii_lowercase()
    };
    let folded = fold(text);
    let start = after.map_or(0, |entity| entity.borrow().slot + 1);
    for index in start..pool.num_entities() {
        let entity = pool.at(index as i32);
        let borrowed = entity.borrow();
        if !borrowed.inuse {
            continue;
        }
        let value = match field {
            EntityStringField::Classname => borrowed.classname.clone(),
            EntityStringField::Targetname => borrowed.targetname.clone(),
            EntityStringField::Target => borrowed.target.clone(),
            EntityStringField::Team => borrowed.team.clone(),
        };
        if value.as_ref().is_some_and(|value| fold(value) == folded) {
            return Some(entity.clone());
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Simulation shared core (unified from `mirrors_game_sim.rs`): entity records,
// clients, callbacks, items, random, scratch, and world handles. Donor
// provenance per item matches the owning `shared/*` or `game/*` module; these
// subset ports keep the pool/slot/callback shape the 1:1 canonicals cannot
// absorb without behavior changes.
// ---------------------------------------------------------------------------

/// Random operations used across the simulation.
pub trait SimRandom {
    /// `rand()` nonnegative `0..=0x7fff` draw.
    fn rand_value(&mut self) -> i32;
    /// `random()` unit draw.
    fn random_value(&mut self) -> f32;
    /// `crandom()` centered draw.
    fn crandom_value(&mut self) -> f32;
}

/// Instance-owned `bg_lib` rand/srand (`GameRandom`).
#[derive(Debug, Clone)]
pub struct GameRandomMirror {
    /// Current seed.
    seed: i32,
}

impl GameRandomMirror {
    /// Build with a seed (`new GameRandom(seed)`).
    #[must_use]
    pub fn new(seed: i32) -> Self {
        Self { seed }
    }

    /// Current seed.
    #[must_use]
    pub fn seed(&self) -> i32 {
        self.seed
    }

    /// Reset the seed (`reset`).
    pub fn reset(&mut self, seed: i32) {
        self.seed = seed;
    }

    /// Next `rand()` draw.
    pub fn rand_value(&mut self) -> i32 {
        self.seed = q_rand(self.seed);
        self.seed & 0x7fff
    }

    /// Next `random()` draw.
    pub fn random_value(&mut self) -> f32 {
        let draw = self.rand_value();
        (f64::from(draw) / f64::from(0x7fff)) as f32
    }

    /// Next `crandom()` draw.
    pub fn crandom_value(&mut self) -> f32 {
        let inner = (f64::from(self.random_value()) - 0.5) as f32;
        (2.0 * f64::from(inner)) as f32
    }
}

impl SimRandom for GameRandomMirror {
    fn rand_value(&mut self) -> i32 {
        GameRandomMirror::rand_value(self)
    }

    fn random_value(&mut self) -> f32 {
        GameRandomMirror::random_value(self)
    }

    fn crandom_value(&mut self) -> f32 {
        GameRandomMirror::crandom_value(self)
    }
}

// ---------------------------------------------------------------------------
// Trajectory, entity state, shared state (mirrors of `shared/trajectory.ts`,
// network `entity.ts`, `shared/entity-shared.ts`).
// ---------------------------------------------------------------------------

/// Server-side entity collision record (`EntityShared` plus body ground).
#[derive(Debug, Clone, PartialEq)]
pub struct EntityShared {
    /// Server flags.
    pub sv_flags: i32,
    /// Single-client target.
    pub single_client: i32,
    /// Contents mask.
    pub contents: i32,
    /// Owner entity number.
    pub owner_num: i32,
    /// Bounding minimum.
    pub mins: Vec3,
    /// Bounding maximum.
    pub maxs: Vec3,
    /// Current collision origin.
    pub current_origin: Vec3,
    /// Current collision angles.
    pub current_angles: Vec3,
    /// Supporting ground actor.
    pub ground: Option<ActorId>,
    /// Linked into collision.
    pub linked: bool,
    /// Link count.
    pub link_count: i32,
}

impl Default for EntityShared {
    fn default() -> Self {
        Self {
            sv_flags: 0,
            single_client: 0,
            contents: 0,
            owner_num: 0,
            mins: vec3(0.0, 0.0, 0.0),
            maxs: vec3(0.0, 0.0, 0.0),
            current_origin: vec3(0.0, 0.0, 0.0),
            current_angles: vec3(0.0, 0.0, 0.0),
            ground: None,
            linked: false,
            link_count: 0,
        }
    }
}

// ---------------------------------------------------------------------------
// Player state (mirror of `shared/player-state.ts`).
// ---------------------------------------------------------------------------

/// Integer slot vector (`PlayerStateSlots`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlayerStateSlots {
    /// Slot values.
    values: Vec<i32>,
}

impl PlayerStateSlots {
    /// Zeroed slots of a length.
    #[must_use]
    pub fn new(length: usize) -> Self {
        Self {
            values: vec![0; length],
        }
    }

    /// Slot count.
    #[must_use]
    pub fn len(&self) -> usize {
        self.values.len()
    }

    /// True when there are no slots.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    /// Read a slot (`get`).
    #[must_use]
    pub fn get(&self, index: i32) -> i32 {
        if index < 0 || index as usize >= self.values.len() {
            panic!("Player state slot {} outside {}", index, self.values.len());
        }
        self.values[index as usize]
    }

    /// Write a slot (`set`).
    pub fn set(&mut self, index: i32, value: i32) {
        if index < 0 || index as usize >= self.values.len() {
            panic!("Player state slot {} outside {}", index, self.values.len());
        }
        self.values[index as usize] = value;
    }

    /// Copy all slot values (`copy`).
    #[must_use]
    pub fn copy(&self) -> Vec<i32> {
        self.values.clone()
    }
}

/// Predictable-event debug sink (`PredictableEventDebug`).
#[derive(Clone)]
pub struct EventDebugMirror {
    /// Module side.
    pub module: EventDebugModule,
    /// `showEvents()` value reader.
    pub show_events: Rc<dyn Fn() -> String>,
    /// Debug printer.
    pub print: Rc<dyn Fn(String)>,
}

// `bg_misc.c:eventnames`, including its omitted `EV_OBELISKPAIN` entry.
pub(crate) const EVENT_NAMES: [&str; 76] = [
    "EV_NONE",
    "EV_FOOTSTEP",
    "EV_FOOTSTEP_METAL",
    "EV_FOOTSPLASH",
    "EV_FOOTWADE",
    "EV_SWIM",
    "EV_STEP_4",
    "EV_STEP_8",
    "EV_STEP_12",
    "EV_STEP_16",
    "EV_FALL_SHORT",
    "EV_FALL_MEDIUM",
    "EV_FALL_FAR",
    "EV_JUMP_PAD",
    "EV_JUMP",
    "EV_WATER_TOUCH",
    "EV_WATER_LEAVE",
    "EV_WATER_UNDER",
    "EV_WATER_CLEAR",
    "EV_ITEM_PICKUP",
    "EV_GLOBAL_ITEM_PICKUP",
    "EV_NOAMMO",
    "EV_CHANGE_WEAPON",
    "EV_FIRE_WEAPON",
    "EV_USE_ITEM0",
    "EV_USE_ITEM1",
    "EV_USE_ITEM2",
    "EV_USE_ITEM3",
    "EV_USE_ITEM4",
    "EV_USE_ITEM5",
    "EV_USE_ITEM6",
    "EV_USE_ITEM7",
    "EV_USE_ITEM8",
    "EV_USE_ITEM9",
    "EV_USE_ITEM10",
    "EV_USE_ITEM11",
    "EV_USE_ITEM12",
    "EV_USE_ITEM13",
    "EV_USE_ITEM14",
    "EV_USE_ITEM15",
    "EV_ITEM_RESPAWN",
    "EV_ITEM_POP",
    "EV_PLAYER_TELEPORT_IN",
    "EV_PLAYER_TELEPORT_OUT",
    "EV_GRENADE_BOUNCE",
    "EV_GENERAL_SOUND",
    "EV_GLOBAL_SOUND",
    "EV_GLOBAL_TEAM_SOUND",
    "EV_BULLET_HIT_FLESH",
    "EV_BULLET_HIT_WALL",
    "EV_MISSILE_HIT",
    "EV_MISSILE_MISS",
    "EV_MISSILE_MISS_METAL",
    "EV_RAILTRAIL",
    "EV_SHOTGUN",
    "EV_BULLET",
    "EV_PAIN",
    "EV_DEATH1",
    "EV_DEATH2",
    "EV_DEATH3",
    "EV_OBITUARY",
    "EV_POWERUP_QUAD",
    "EV_POWERUP_BATTLESUIT",
    "EV_POWERUP_REGEN",
    "EV_GIB_PLAYER",
    "EV_SCOREPLUM",
    "EV_PROXIMITY_MINE_STICK",
    "EV_PROXIMITY_MINE_TRIGGER",
    "EV_KAMIKAZE",
    "EV_OBELISKEXPLODE",
    "EV_INVUL_IMPACT",
    "EV_JUICED",
    "EV_LIGHTNINGBOLT",
    "EV_DEBUG_LINE",
    "EV_STOPLOOPINGSOUND",
    "EV_TAUNT",
];

/// Minimal leading-number scan matching `nativeAtof` for event-debug flags.
pub(crate) fn debug_flag(text: &str) -> f64 {
    let bytes = text.as_bytes();
    let mut index = 0;
    while index < bytes.len() && bytes[index].is_ascii_whitespace() {
        index += 1;
    }
    let mut sign = 1.0;
    if index < bytes.len() && (bytes[index] == b'+' || bytes[index] == b'-') {
        if bytes[index] == b'-' {
            sign = -1.0;
        }
        index += 1;
    }
    let mut value = 0.0;
    while index < bytes.len() && bytes[index].is_ascii_digit() {
        value = value * 10.0 + f64::from(bytes[index] - b'0');
        index += 1;
    }
    if index < bytes.len() && bytes[index] == b'.' {
        index += 1;
        let mut place = 0.1;
        while index < bytes.len() && bytes[index].is_ascii_digit() {
            value += f64::from(bytes[index] - b'0') * place;
            place *= 0.1;
            index += 1;
        }
    }
    sign * value
}

/// Owned `playerState_t` storage (`PlayerState`).
#[derive(Clone)]
pub struct PlayerState {
    /// Product.
    pub product: Product,
    /// Movement type.
    pub pm_type: MoveType,
    /// Movement flags.
    pub pm_flags: i32,
    /// Movement timer.
    pub pm_time: i32,
    /// Authoritative origin.
    pub origin: Vec3,
    /// Legs animation.
    pub legs_anim: i32,
    /// Torso animation.
    pub torso_anim: i32,
    /// Entity flags.
    pub e_flags: i32,
    /// Predictable-event sequence.
    pub event_sequence: i32,
    /// Predictable events ring.
    pub events: PlayerStateSlots,
    /// Predictable event parameters ring.
    pub event_parms: PlayerStateSlots,
    /// External event.
    pub external_event: i32,
    /// External event parameter.
    pub external_event_parm: i32,
    /// External event time.
    pub external_event_time: i32,
    /// Client number.
    pub client_num: i32,
    /// Current weapon.
    pub weapon: Weapon,
    /// Weapon state.
    pub weapon_state: WeaponState,
    /// View angles.
    pub viewangles: Vec3,
    /// Stats slots.
    pub stats: PlayerStateSlots,
    /// Persistent score slots.
    pub persistant: PlayerStateSlots,
    /// Powerup timer slots.
    pub powerups: PlayerStateSlots,
    /// Ammo slots.
    pub ammo: PlayerStateSlots,
    /// Generic 1.
    pub generic1: i32,
    /// Pmove frame count (event debug only).
    pub pmove_framecount: i32,
    /// Authoritative angle delta words (`ps.deltaAngles`; C7 needs input observation).
    pub delta_angles: [i32; 3],
    /// Event debug sink.
    event_debug: Option<EventDebugMirror>,
}

impl PlayerState {
    /// Zeroed player state for a product (`createPlayerState`).
    #[must_use]
    pub fn new(product: Product) -> Self {
        Self {
            product,
            pm_type: MoveType::PmNormal,
            pm_flags: 0,
            pm_time: 0,
            origin: vec3(0.0, 0.0, 0.0),
            legs_anim: 0,
            torso_anim: 0,
            e_flags: 0,
            event_sequence: 0,
            events: PlayerStateSlots::new(2),
            event_parms: PlayerStateSlots::new(2),
            external_event: 0,
            external_event_parm: 0,
            external_event_time: 0,
            client_num: 0,
            weapon: Weapon::WpNone,
            weapon_state: WeaponState::WeaponReady,
            viewangles: vec3(0.0, 0.0, 0.0),
            stats: PlayerStateSlots::new(16),
            persistant: PlayerStateSlots::new(16),
            powerups: PlayerStateSlots::new(16),
            ammo: PlayerStateSlots::new(16),
            generic1: 0,
            pmove_framecount: 0,
            delta_angles: [0; 3],
            event_debug: None,
        }
    }

    /// Install or clear the event-debug sink (`setEventDebug`).
    pub fn set_event_debug(&mut self, debug: Option<EventDebugMirror>) {
        self.event_debug = debug;
    }

    /// Queue a predictable event (`addEvent`).
    pub fn add_event(&mut self, event: i32, parameter: i32) -> PredictableEvent {
        if let Some(debug) = self.event_debug.clone() {
            let flag: String = (debug.show_events)().chars().take(255).collect();
            if debug_flag(&flag) != 0.0 {
                let name = if event >= 0 && (event as usize) < EVENT_NAMES.len() {
                    EVENT_NAMES[event as usize]
                } else {
                    panic!("bg_misc.c eventnames has no entry for {event}");
                };
                let label = match debug.module {
                    EventDebugModule::Game => " game",
                    EventDebugModule::Cgame => "Cgame",
                };
                (debug.print)(format!(
                    "{label} event svt {:>5} -> {:>5}: num = {:>20} parm {parameter}\n",
                    self.pmove_framecount, self.event_sequence, name
                ));
            }
        }
        let sequence = self.event_sequence;
        self.events.set(sequence & 1, event);
        self.event_parms.set(sequence & 1, parameter);
        self.event_sequence = sequence.wrapping_add(1);
        PredictableEvent {
            sequence,
            event,
            parameter,
        }
    }
}

// ---------------------------------------------------------------------------
// Client (mirror of `game/state.ts` `GameClient`).
// ---------------------------------------------------------------------------

/// User command, weapon selection only (`UserCommand`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct UserCommand {
    /// Selected weapon.
    pub weapon: i32,
}

/// Persistent client data (`ClientPersistant`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientPersistant {
    /// Connection state.
    pub connected: ConnectionState,
    /// Latest user command.
    pub cmd: UserCommand,
    /// Predict item pickups.
    pub predict_item_pickup: bool,
    /// Network name.
    pub netname: String,
}

impl Default for ClientPersistant {
    fn default() -> Self {
        Self {
            connected: ConnectionState::Disconnected,
            cmd: UserCommand::default(),
            predict_item_pickup: false,
            netname: String::new(),
        }
    }
}

/// Session data (`ClientSession`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClientSession {
    /// Session team.
    pub session_team: Team,
    /// Spectator state.
    pub spectator_state: SpectatorState,
    /// Followed client number.
    pub spectator_client: i32,
}

impl Default for ClientSession {
    fn default() -> Self {
        Self {
            session_team: Team::TeamFree,
            spectator_state: SpectatorState::Not,
            spectator_client: 0,
        }
    }
}

/// Source-zero game client (`GameClient`).
#[derive(Clone)]
pub struct GameClient {
    /// Player state.
    pub ps: PlayerState,
    /// Persistent data.
    pub pers: ClientPersistant,
    /// Session data.
    pub sess: ClientSession,
    /// Noclip cheat.
    pub noclip: bool,
    /// Absorbed armor damage accumulator.
    pub damage_armor: i32,
    /// Blood damage accumulator.
    pub damage_blood: i32,
    /// Knockback accumulator.
    pub damage_knockback: i32,
    /// Damage source direction.
    pub damage_from: Vec3,
    /// Damage came from the world.
    pub damage_from_world: bool,
    /// Last killed client.
    pub last_killed_client: i32,
    /// Last hurt client.
    pub last_hurt_client: i32,
    /// Last hurt means of death.
    pub last_hurt_mod: i32,
    /// Respawn time.
    pub respawn_time: i32,
    /// Reward expiry time.
    pub reward_time: i32,
    /// Last kill time.
    pub last_kill_time: i32,
    /// Active grapple hook.
    pub hook: Option<EntityRef>,
    /// Carried persistent powerup.
    pub persistant_powerup: Option<EntityRef>,
    /// Invulnerability expiry time.
    pub invulnerability_time: i32,
    /// Per-weapon ammo timers.
    pub ammo_times: PlayerStateSlots,
}

impl GameClient {
    /// Zeroed client for a product (`new GameClient(product)`).
    #[must_use]
    pub fn new(product: Product) -> Self {
        Self {
            ps: PlayerState::new(product),
            pers: ClientPersistant::default(),
            sess: ClientSession::default(),
            noclip: false,
            damage_armor: 0,
            damage_blood: 0,
            damage_knockback: 0,
            damage_from: vec3(0.0, 0.0, 0.0),
            damage_from_world: false,
            last_killed_client: 0,
            last_hurt_client: 0,
            last_hurt_mod: 0,
            respawn_time: 0,
            reward_time: 0,
            last_kill_time: 0,
            hook: None,
            persistant_powerup: None,
            invulnerability_time: 0,
            ammo_times: PlayerStateSlots::new(weapon_count(product) as usize),
        }
    }
}

// ---------------------------------------------------------------------------
// Identity and items (mirrors of `contracts/identity.ts`, `shared/items.ts`).
// ---------------------------------------------------------------------------

/// Provider identity (`ProviderId`).
pub type ProviderId = String;

/// Namespaced item identity (`ItemId`).
pub type ItemId = String;

/// Actor bound to its owning provider (`OwnedActor`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwnedActor {
    /// Actor handle.
    pub id: ActorId,
    /// Owning provider.
    pub owner: ProviderId,
}

/// Item definition (`ItemDefinition`).
#[derive(Debug, Clone, PartialEq)]
pub struct ItemDefinition {
    /// Spawn class name.
    pub class_name: Option<String>,
    /// Pickup display name.
    pub pickup_name: Option<String>,
    /// Default quantity.
    pub quantity: i32,
    /// Item type.
    pub item_type: ItemType,
    /// Type tag (weapon, powerup, or holdable id, else 0).
    pub tag: i32,
}

/// Product item table (`itemList` plus `BG_*` lookup helpers).
#[derive(Debug, Clone)]
pub struct Q3ItemTable {
    /// Table entries; index zero is the reserved empty item.
    pub items: Vec<ItemDefinition>,
}

impl Q3ItemTable {
    /// Build from entries.
    #[must_use]
    pub fn new(items: Vec<ItemDefinition>) -> Self {
        Self { items }
    }

    /// Entry count.
    #[must_use]
    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// True when the table is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// Entry by index (`itemAt`).
    #[must_use]
    pub fn item_at(&self, index: i32) -> &ItemDefinition {
        if index < 0 || index as usize >= self.items.len() {
            panic!("Item index out of range: {index}");
        }
        &self.items[index as usize]
    }

    /// Index of an entry (`indexOf`).
    #[must_use]
    pub fn index_of(&self, item: &ItemDefinition) -> Option<usize> {
        self.items.iter().position(|entry| entry == item)
    }

    /// Entry by pickup name, ASCII case-insensitive (`findItem`).
    #[must_use]
    pub fn find_item(&self, pickup_name: &str) -> Option<&ItemDefinition> {
        let folded = pickup_name.to_ascii_lowercase();
        self.items.iter().find(|item| {
            item.pickup_name
                .as_ref()
                .is_some_and(|name| name.to_ascii_lowercase() == folded)
        })
    }

    /// Entry granting a powerup (`findItemForPowerup`).
    #[must_use]
    pub fn find_item_for_powerup(&self, powerup: i32) -> Option<&ItemDefinition> {
        self.items.iter().find(|item| {
            matches!(
                item.item_type,
                ItemType::ItPowerup | ItemType::ItTeam | ItemType::ItPersistantPowerup
            ) && item.tag == powerup
        })
    }

    /// Entry granting a weapon (`findItemForWeapon`).
    #[must_use]
    pub fn find_item_for_weapon(&self, weapon: i32) -> &ItemDefinition {
        self.items
            .iter()
            .find(|item| item.item_type == ItemType::ItWeapon && item.tag == weapon)
            .unwrap_or_else(|| panic!("Couldn't find item for weapon {weapon}"))
    }

    /// Armor pickup eligibility (`canQ3ArmorBeGrabbed`).
    #[must_use]
    pub fn can_q3_armor_be_grabbed(&self, ps: &PlayerInventory) -> bool {
        if ps.product == Product::Missionpack {
            if self.item_at(ps.persistent_powerup_index).tag == Powerup::PwScout as i32 {
                return false;
            }
            let upper = if self.item_at(ps.persistent_powerup_index).tag == Powerup::PwGuard as i32 {
                ps.max_health
            } else {
                ps.max_health * 2
            };
            return ps.armor < upper;
        }
        ps.armor < ps.max_health * 2
    }

    /// Pickup eligibility (`canItemBeGrabbed`).
    #[must_use]
    pub fn can_item_be_grabbed(&self, gametype: i32, ent: &PickupEntity, ps: &PlayerInventory) -> bool {
        if ent.model_index < 1 || ent.model_index as usize >= self.items.len() {
            panic!("BG_CanItemBeGrabbed: index out of range");
        }
        let item = self.item_at(ent.model_index);
        match item.item_type {
            ItemType::ItWeapon => true,
            ItemType::ItAmmo => ps.ammo(item.tag) < 200,
            ItemType::ItArmor => self.can_q3_armor_be_grabbed(ps),
            ItemType::ItHealth => {
                if ps.product == Product::Missionpack
                    && self.item_at(ps.persistent_powerup_index).tag == Powerup::PwGuard as i32
                {
                    return ps.health < ps.max_health;
                }
                ps.health < ps.max_health * (i32::from(item.quantity == 5 || item.quantity == 100) + 1)
            }
            ItemType::ItPowerup => true,
            ItemType::ItPersistantPowerup => {
                if ps.product == Product::Baseq3 || ps.persistent_powerup_index != 0 {
                    return false;
                }
                if (ent.generic1 & 2) != 0 && ps.team != Team::TeamRed as i32 {
                    return false;
                }
                if (ent.generic1 & 4) != 0 && ps.team != Team::TeamBlue as i32 {
                    return false;
                }
                true
            }
            ItemType::ItTeam => {
                if ps.product == Product::Missionpack && gametype == GameType::Gt1fctf as i32 {
                    if item.tag == Powerup::PwNeutralflag as i32 {
                        return true;
                    }
                    if ps.team == Team::TeamRed as i32
                        && item.tag == Powerup::PwBlueflag as i32
                        && ps.powerup(Powerup::PwNeutralflag as i32) != 0
                    {
                        return true;
                    }
                    if ps.team == Team::TeamBlue as i32
                        && item.tag == Powerup::PwRedflag as i32
                        && ps.powerup(Powerup::PwNeutralflag as i32) != 0
                    {
                        return true;
                    }
                }
                if gametype == GameType::GtCtf as i32 {
                    if ps.team == Team::TeamRed as i32 {
                        return item.tag == Powerup::PwBlueflag as i32
                            || (item.tag == Powerup::PwRedflag as i32
                                && (ent.model_index2 != 0 || ps.powerup(Powerup::PwBlueflag as i32) != 0));
                    }
                    if ps.team == Team::TeamBlue as i32 {
                        return item.tag == Powerup::PwRedflag as i32
                            || (item.tag == Powerup::PwBlueflag as i32
                                && (ent.model_index2 != 0 || ps.powerup(Powerup::PwRedflag as i32) != 0));
                    }
                }
                ps.product == Product::Missionpack && gametype == GameType::GtHarvester as i32
            }
            ItemType::ItHoldable => ps.holdable_item == 0,
            ItemType::ItBad => panic!("BG_CanItemBeGrabbed: IT_BAD"),
        }
    }
}

/// Pickup eligibility entity fields (`PickupEntity`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PickupEntity {
    /// Item table model index.
    pub model_index: i32,
    /// Dropped marker.
    pub model_index2: i32,
    /// Generic flags.
    pub generic1: i32,
}

/// Player inventory view (`PlayerInventory`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlayerInventory {
    /// Product.
    pub product: Product,
    /// Health.
    pub health: i32,
    /// Armor.
    pub armor: i32,
    /// Maximum health.
    pub max_health: i32,
    /// Holdable item.
    pub holdable_item: i32,
    /// Team.
    pub team: i32,
    /// Ammo slots.
    pub ammo: Vec<i32>,
    /// Powerup timer slots.
    pub powerups: Vec<i32>,
    /// Persistent powerup item index (mission pack).
    pub persistent_powerup_index: i32,
}

impl PlayerInventory {
    /// Ammo for a weapon tag.
    #[must_use]
    pub fn ammo(&self, weapon: i32) -> i32 {
        self.ammo.get(weapon as usize).copied().unwrap_or(0)
    }

    /// Powerup timer for a powerup tag.
    #[must_use]
    pub fn powerup(&self, powerup: i32) -> i32 {
        self.powerups.get(powerup as usize).copied().unwrap_or(0)
    }
}

/// Inventory view of a client (`q3ItemInventory`).
#[must_use]
pub fn q3_item_inventory(client: &GameClient) -> PlayerInventory {
    let ps = &client.ps;
    let (health_slot, armor_slot, max_health_slot, holdable_item_slot) = match stat_schema(ps.product) {
        StatSchema::Base(layout) => (layout.health, layout.armor, layout.max_health, layout.holdable_item),
        StatSchema::Missionpack(layout) => (layout.health, layout.armor, layout.max_health, layout.holdable_item),
    };
    PlayerInventory {
        product: ps.product,
        health: ps.stats.get(health_slot),
        armor: ps.stats.get(armor_slot),
        max_health: ps.stats.get(max_health_slot),
        holdable_item: ps.stats.get(holdable_item_slot),
        team: ps.persistant.get(PersistentIndex::PersTeam as i32),
        ammo: ps.ammo.copy(),
        powerups: ps.powerups.copy(),
        persistent_powerup_index: if ps.product == Product::Missionpack {
            ps.stats.get(MissionpackStatIndex::StatPersistantPowerup as i32)
        } else {
            0
        },
    }
}

// ---------------------------------------------------------------------------
// Game entity, participants, callbacks (mirrors of `game/state.ts`,
// `game/use-participant.ts`, `game/save-callbacks.ts`).
// ---------------------------------------------------------------------------

/// Shared entity handle.
pub type EntityRef = Rc<RefCell<GameEntity>>;

/// Pool handle shared with saved callbacks.
pub type PoolHandle = Rc<RefCell<EntityPool>>;

/// Non-native damage participant (`{ kind: "shared-actor", ... }`).
#[derive(Debug, Clone, PartialEq)]
pub struct SharedActor {
    /// Actor handle.
    pub actor: ActorId,
    /// World origin, when known.
    pub origin: Option<Vec3>,
}

/// Damage/use participant (`DamageParticipant` / `UseParticipant`).
#[derive(Clone)]
pub enum DamageParticipant {
    /// Native game entity.
    Native(EntityRef),
    /// Shared actor outside the game DLL.
    Shared(SharedActor),
}

/// Actor handle of a participant (`useActor`).
#[must_use]
pub fn use_actor(participant: &DamageParticipant) -> ActorId {
    match participant {
        DamageParticipant::Native(entity) => entity.borrow().actor.id.clone(),
        DamageParticipant::Shared(shared) => shared.actor.clone(),
    }
}

/// Touch contact surface (`TouchContact.surface`).
#[derive(Debug, Clone, PartialEq)]
pub struct TouchSurfaceMirror {
    /// Surface name.
    pub name: String,
    /// Native flags.
    pub native_flags: i32,
    /// Native value.
    pub native_value: i32,
}

/// Touch contact (`TouchContact`).
#[derive(Debug, Clone, PartialEq)]
pub struct TouchContactMirror {
    /// Touching entity.
    pub this_actor: OwnedActor,
    /// Touched actor.
    pub other: ActorId,
    /// Contact plane.
    pub plane: Option<Plane>,
    /// Contact surface.
    pub surface: Option<TouchSurfaceMirror>,
}

/// Think callback (`EntityThink`).
pub type ThinkCallback = Rc<dyn Fn(EntityRef)>;

/// Reached callback (same shape as think).
pub type ReachedCallback = Rc<dyn Fn(EntityRef)>;

/// Blocked callback (`EntityBlocked`).
pub type BlockedCallback = Rc<dyn Fn(EntityRef, DamageParticipant)>;

/// Touch callback (`EntityTouch`).
pub type TouchCallback = Rc<dyn Fn(EntityRef, DamageParticipant, TouchContactMirror)>;

/// Use callback (`EntityUse`).
pub type UseCallback = Rc<dyn Fn(EntityRef, Option<DamageParticipant>, Option<DamageParticipant>)>;

/// Pain callback (`EntityPain`).
pub type PainCallback = Rc<dyn Fn(EntityRef, DamageParticipant, i32)>;

/// Die callback (`EntityDie`).
pub type DieCallback = Rc<dyn Fn(EntityRef, Option<DamageParticipant>, Option<DamageParticipant>, i32, i32)>;

/// Function identity for callback families.
pub trait CallbackIdentity {
    /// True when both handles name the same callback.
    fn same_callback(left: &Self, right: &Self) -> bool;
}

impl CallbackIdentity for ThinkCallback {
    fn same_callback(left: &Self, right: &Self) -> bool {
        Rc::ptr_eq(left, right)
    }
}

impl CallbackIdentity for BlockedCallback {
    fn same_callback(left: &Self, right: &Self) -> bool {
        Rc::ptr_eq(left, right)
    }
}

impl CallbackIdentity for TouchCallback {
    fn same_callback(left: &Self, right: &Self) -> bool {
        Rc::ptr_eq(left, right)
    }
}

impl CallbackIdentity for UseCallback {
    fn same_callback(left: &Self, right: &Self) -> bool {
        Rc::ptr_eq(left, right)
    }
}

impl CallbackIdentity for PainCallback {
    fn same_callback(left: &Self, right: &Self) -> bool {
        Rc::ptr_eq(left, right)
    }
}

impl CallbackIdentity for DieCallback {
    fn same_callback(left: &Self, right: &Self) -> bool {
        Rc::ptr_eq(left, right)
    }
}

/// Named native-callback family (`Q3CallbackFamily`).
#[derive(Debug, Clone)]
pub struct Q3CallbackFamily<F> {
    /// Callbacks by identity.
    by_id: HashMap<String, F>,
    /// Identities by callback.
    by_fn: Vec<(F, String)>,
}

impl<F: Clone + CallbackIdentity> Q3CallbackFamily<F> {
    /// Empty family.
    #[must_use]
    pub fn new() -> Self {
        Self {
            by_id: HashMap::new(),
            by_fn: Vec::new(),
        }
    }

    /// Register a callback identity (`register`).
    pub fn register(&mut self, id: &str, callback: F) -> F {
        if id.is_empty() {
            panic!("Q3 callback identity is empty");
        }
        if let Some(previous) = self.by_id.get(id) {
            if !F::same_callback(previous, &callback) {
                panic!("Duplicate Q3 callback identity {id}");
            }
        }
        for (known, identity) in &self.by_fn {
            if F::same_callback(known, &callback) && identity != id {
                panic!("Q3 callback has two identities: {identity}, {id}");
            }
        }
        self.by_id.insert(id.to_string(), callback.clone());
        if !self.by_fn.iter().any(|(known, _)| F::same_callback(known, &callback)) {
            self.by_fn.push((callback.clone(), id.to_string()));
        }
        callback
    }

    /// Register unless the identity already exists (`intern`).
    pub fn intern(&mut self, id: &str, callback: F) -> F {
        if let Some(existing) = self.by_id.get(id) {
            return existing.clone();
        }
        self.register(id, callback)
    }

    /// Identity of a callback (`capture`).
    #[must_use]
    pub fn capture(&self, callback: Option<&F>) -> Option<String> {
        let callback = callback?;
        for (known, identity) in &self.by_fn {
            if F::same_callback(known, callback) {
                return Some(identity.clone());
            }
        }
        panic!("Unregistered native Q3 callback cannot be saved");
    }

    /// Callback for an identity (`resolve`).
    #[must_use]
    pub fn resolve(&self, id: Option<&str>) -> Option<F> {
        let id = id?;
        match self.by_id.get(id) {
            Some(callback) => Some(callback.clone()),
            None => panic!("Unknown native Q3 callback {id}"),
        }
    }
}

impl<F: Clone + CallbackIdentity> Default for Q3CallbackFamily<F> {
    fn default() -> Self {
        Self::new()
    }
}

/// Native-callback catalog (`Q3CallbackCatalog`).
#[derive(Clone, Default)]
pub struct Q3CallbackCatalog {
    /// Think callbacks.
    pub think: Q3CallbackFamily<ThinkCallback>,
    /// Reached callbacks.
    pub reached: Q3CallbackFamily<ReachedCallback>,
    /// Blocked callbacks.
    pub blocked: Q3CallbackFamily<BlockedCallback>,
    /// Touch callbacks.
    pub touch: Q3CallbackFamily<TouchCallback>,
    /// Use callbacks.
    pub use_callbacks: Q3CallbackFamily<UseCallback>,
    /// Pain callbacks.
    pub pain: Q3CallbackFamily<PainCallback>,
    /// Die callbacks.
    pub die: Q3CallbackFamily<DieCallback>,
}

impl Q3CallbackCatalog {
    /// Empty catalog.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

/// Source-zero game entity (`GameEntity`).
pub struct GameEntity {
    /// Table slot.
    pub slot: usize,
    /// Owned actor handle.
    pub actor: OwnedActor,
    /// In-use flag.
    pub inuse: bool,
    /// Entity state.
    pub s: EntityState,
    /// Shared collision record.
    pub r: EntityShared,
    /// Client record for player entities.
    pub client: Option<GameClient>,
    /// Class name.
    pub classname: Option<String>,
    /// Spawn flags.
    pub spawnflags: i32,
    /// Never freed.
    pub never_free: bool,
    /// Game flags bitmask.
    pub flags: i32,
    /// Free time.
    pub freetime: i32,
    /// Event time.
    pub event_time: i32,
    /// Free after the event expires.
    pub free_after_event: bool,
    /// Unlink after the event expires.
    pub unlink_after_event: bool,
    /// Physics bounce factor.
    pub physics_bounce: f32,
    /// Collision mask.
    pub clipmask: i32,
    /// Mover state.
    pub mover_state: MoverState,
    /// Next think time.
    pub nextthink: i32,
    /// Think callback.
    pub think: Option<ThinkCallback>,
    /// Reached callback.
    pub reached: Option<ReachedCallback>,
    /// Blocked callback.
    pub blocked: Option<BlockedCallback>,
    /// Touch callback.
    pub touch: Option<TouchCallback>,
    /// Use callback.
    pub use_callback: Option<UseCallback>,
    /// Pain callback.
    pub pain: Option<PainCallback>,
    /// Die callback.
    pub die: Option<DieCallback>,
    /// Health.
    pub health: i32,
    /// Can take damage.
    pub takedamage: bool,
    /// Item count override.
    pub count: i32,
    /// Activating entity.
    pub activator: Option<EntityRef>,
    /// Team chain link.
    pub teamchain: Option<EntityRef>,
    /// Team master.
    pub teammaster: Option<EntityRef>,
    /// Team name.
    pub team: Option<String>,
    /// Target name.
    pub targetname: Option<String>,
    /// Target.
    pub target: Option<String>,
    /// Wait (respawn override seconds).
    pub wait: f32,
    /// Random respawn spread.
    pub random: f32,
    /// Contained item.
    pub item: Option<ItemDefinition>,
    /// Speed (or powerup global-sound suppression).
    pub speed: f32,
}

impl GameEntity {
    /// Fresh entity for a slot and actor (`new GameEntity(slot, binding)`).
    #[must_use]
    pub fn new(slot: usize, actor: OwnedActor) -> Self {
        assert!(slot < MAX_GENTITIES, "Game entity slot outside 0..1023");
        Self {
            slot,
            actor,
            inuse: false,
            s: EntityState::default(),
            r: EntityShared::default(),
            client: None,
            classname: None,
            spawnflags: 0,
            never_free: false,
            flags: 0,
            freetime: 0,
            event_time: 0,
            free_after_event: false,
            unlink_after_event: false,
            physics_bounce: 0.0,
            clipmask: 0,
            mover_state: MoverState::Pos1,
            nextthink: 0,
            think: None,
            reached: None,
            blocked: None,
            touch: None,
            use_callback: None,
            pain: None,
            die: None,
            health: 0,
            takedamage: false,
            count: 0,
            activator: None,
            teamchain: None,
            teammaster: None,
            team: None,
            targetname: None,
            target: None,
            wait: 0.0,
            random: 0.0,
            item: None,
            speed: 0.0,
        }
    }

    /// Clear all fields for release, keeping slot and actor identity.
    pub fn reset(&mut self) {
        let fresh = Self::new(self.slot, self.actor.clone());
        *self = fresh;
    }
}

// ---------------------------------------------------------------------------
// Rankings, utility scratch, spawn variables, death animation, arsenal
// (mirrors of `game/rankings.ts`, `game/utilities.ts`, `game/spawn.ts`,
// `foundation/character.ts`, `foundation/arsenal.ts`).
// ---------------------------------------------------------------------------

/// Temporary vector ring slot.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct TemporaryVector {
    /// X component.
    pub x: f32,
    /// Y component.
    pub y: f32,
    /// Z component.
    pub z: f32,
}

/// Fixed 32-byte game string allocation (`GameMemoryAllocation`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GameMemoryAllocation {
    /// Backing bytes.
    data: [u8; 32],
}

impl GameMemoryAllocation {
    /// Zeroed allocation.
    #[must_use]
    pub fn new() -> Self {
        Self { data: [0; 32] }
    }

    /// Write a string with NUL termination (`writeString`).
    pub fn write_string(&mut self, value: &str) {
        self.data = [0; 32];
        let bytes = value.as_bytes();
        let count = bytes.len().min(31);
        self.data[..count].copy_from_slice(&bytes[..count]);
    }

    /// Read up to the NUL terminator (`readString`).
    #[must_use]
    pub fn read_string(&self) -> String {
        self.data
            .iter()
            .take_while(|byte| **byte != 0)
            .map(|byte| char::from(*byte))
            .collect()
    }
}

impl Default for GameMemoryAllocation {
    fn default() -> Self {
        Self::new()
    }
}

/// Per-game `tv`/`vtos` scratch rings (`GameUtilityScratch`).
#[derive(Clone)]
pub struct GameUtilityScratch {
    /// Temporary vector ring.
    vectors: [TemporaryVector; 8],
    /// Vector string ring.
    strings: [GameMemoryAllocation; 8],
    /// Vector ring index.
    vector_index: usize,
    /// String ring index.
    string_index: usize,
    /// Overflow printer.
    print: Rc<dyn Fn(String)>,
}

impl GameUtilityScratch {
    /// Scratch rings with a printer.
    #[must_use]
    pub fn new(print: Rc<dyn Fn(String)>) -> Self {
        Self {
            vectors: [TemporaryVector::default(); 8],
            strings: [GameMemoryAllocation::new(); 8],
            vector_index: 0,
            string_index: 0,
            print,
        }
    }

    /// Next temporary vector (`tv`).
    pub fn tv(&mut self, x: f32, y: f32, z: f32) -> TemporaryVector {
        let vector = TemporaryVector { x, y, z };
        self.vectors[self.vector_index] = vector;
        self.vector_index = (self.vector_index + 1) & 7;
        vector
    }

    /// Next vector string (`vtos`).
    pub fn vtos(&mut self, vector: Vec3) -> GameMemoryAllocation {
        let value = game_format(
            "(%i %i %i)",
            &[
                GameFormatArgument::Int(qvm_float_to_int(vector.x)),
                GameFormatArgument::Int(qvm_float_to_int(vector.y)),
                GameFormatArgument::Int(qvm_float_to_int(vector.z)),
            ],
        );
        if value.chars().count() >= 32 {
            (self.print)(game_format(
                "Com_sprintf: overflow of %i in %i\n",
                &[
                    GameFormatArgument::Int(value.chars().count() as i32),
                    GameFormatArgument::Int(32),
                ],
            ));
        }
        let slot = &mut self.strings[self.string_index];
        self.string_index = (self.string_index + 1) & 7;
        let clipped: String = value.chars().take(31).collect();
        slot.write_string(&clipped);
        *slot
    }
}

/// Server world operations (`ServerWorld`).
#[derive(Clone)]
pub struct ServerWorldMirror {
    /// Entity-number trace (`trace`).
    pub trace: Rc<dyn Fn(&ServerTraceQuery) -> ServerTraceResult>,
    /// Actor trace (`traceActor`).
    pub trace_actor: Rc<dyn Fn(&ActorTraceQuery) -> ActorTraceResult>,
    /// Point contents (`pointContents`).
    pub point_contents: Rc<dyn Fn(Vec3, i32) -> i32>,
    /// Link an entity (`link`).
    pub link: Rc<dyn Fn(EntityRef)>,
    /// Unlink an entity number (`unlink`).
    pub unlink: Rc<dyn Fn(i32)>,
}

// ---------------------------------------------------------------------------
// Combat records (mirrors of `contracts/gameplay.ts` and
// `world/gameplay/damage-modifier.ts`).
// ---------------------------------------------------------------------------
