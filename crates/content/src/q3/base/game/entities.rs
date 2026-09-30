//! Quake III base/game: entities.
//!
//! Donor provenance: `src/content/q3/base/game/entities.ts`.

use qa_core::identity::{ActorId, IdentityOwner};
use qa_core::math::{vec3, Vec3};
use qa_core::numeric::qvm_float_to_int;
use std::cell::RefCell;
use std::rc::Rc;

// Intra-group imports: sibling modules split from the same flat port.
use crate::q3::base::game::format::*;
use crate::q3::base::game::mirrors_game_sim::*;

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
    pub product: Q3Product,
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
            borrowed.s.e_type = EntityType::Events as i32 + event;
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
        traj_type: TrajectoryType::Stationary,
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
