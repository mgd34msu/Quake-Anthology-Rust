//! Q3 source player-state bridges between records and selected movement.
//!
//! Port of donor `/home/buzzkill/Projects/quake-typescript/src/app/bootstrap/simulation/q3/player-state.ts`
//! (`readQ3MovementState`, `writeQ3MovementState`,
//! `writeQ3CharacterAnimation`, `readQ3MovementEnvironment`,
//! `readQ3ArsenalRuntime`, `writeQ3ArsenalRuntime`).
//!
//! The donor works on game-layer entities; the Rust port works on the
//! records-layer entity refs whose client records own the shared player
//! state the donor reads and writes. Entity refs are shared handles, so the
//! readers take `&EntityRef` and the writers mutate through the handle.

use qa_content::q3::base::records::{EntityRef as RecordsEntityRef, Q3EntityRecords};
use qa_content::q3::base::shared::definitions::{stat_schema, MoveType, Powerup, Product, StatSchema, Team};
use qa_content::q3::base::shared::items::item_at;
use qa_content::q3::foundation::arsenal::Q3ArsenalRuntimeState;
use qa_world::movement::q3::constants::move_flags;
use qa_world::movement::q3::types::Q3MovementState;
use qa_world::movement::types::{ActorAnimationState, AnimationState, MovementEnvironment, TraceHit};

/// World slot (donor `1022`).
const WORLD_SLOT: usize = 1022;

/// Null slot (donor `1023`).
const NULL_SLOT: i32 = 1023;

pub(crate) fn schema_max_health(schema: StatSchema) -> usize {
    match schema {
        StatSchema::Base(layout) => layout.max_health as usize,
        StatSchema::Missionpack(layout) => layout.max_health as usize,
    }
}

pub(crate) fn schema_persistent_powerup(schema: StatSchema) -> Option<usize> {
    match schema {
        StatSchema::Base(_) => None,
        StatSchema::Missionpack(layout) => Some(layout.persistent_powerup as usize),
    }
}

pub(crate) fn item_tag(product: Product, index: i32) -> i32 {
    item_at(product, index)
        .unwrap_or_else(|_| panic!("Item index out of range: {index}"))
        .tag()
}

fn source_number(hit: &TraceHit, records: &Q3EntityRecords) -> i32 {
    match hit {
        TraceHit::World { .. } => WORLD_SLOT as i32,
        TraceHit::None => NULL_SLOT,
        TraceHit::Actor { actor } => records
            .native_by_actor(Some(actor))
            .map(|entity| entity.borrow().slot as i32)
            .unwrap_or(NULL_SLOT),
    }
}

/// Read immediately before selected Q3 PMove, after game-side triggers,
/// teleport and client policy.
#[must_use]
pub fn read_q3_movement_state(entity: &RecordsEntityRef, records: &Q3EntityRecords) -> Q3MovementState {
    let borrowed = entity.borrow();
    let client = borrowed
        .client
        .as_ref()
        .expect("Q3 player state requires an admitted client");
    let client = client.borrow();
    let ps = &client.ps;
    let jump_pad = if ps.jumppad_ent == 0 {
        None
    } else {
        records.get(ps.jumppad_ent as usize)
    };
    let ground = borrowed.binding.body().read().ground;
    let world = records.get(WORLD_SLOT);
    let ground = match ground {
        None => TraceHit::None,
        Some(actor) => {
            let is_world = world.as_ref().is_some_and(|world| {
                let world = world.borrow();
                world.inuse() && actor == *world.binding.actor().id()
            });
            if is_world {
                TraceHit::World { model: 0 }
            } else {
                TraceHit::Actor { actor }
            }
        }
    };
    Q3MovementState {
        command_time_milliseconds: ps.command_time,
        movement_type: ps.pm_type,
        bob_cycle: ps.bob_cycle,
        movement_flags: ps.pm_flags,
        movement_time_milliseconds: ps.pm_time,
        origin: ps.origin(),
        velocity: ps.velocity(),
        gravity: f64::from(ps.gravity),
        speed: f64::from(ps.speed),
        delta_angle_words: [
            ps.delta_angles.x as i32,
            ps.delta_angles.y as i32,
            ps.delta_angles.z as i32,
        ],
        ground,
        movement_direction: ps.movement_dir,
        grapple_point: ps.grapple_point,
        flags: ps.e_flags,
        view_angles: ps.viewangles,
        view_height: f64::from(ps.viewheight),
        predictable_event_sequence: ps.event_sequence,
        jump_pad: jump_pad.as_ref().and_then(|pad| {
            let pad = pad.borrow();
            if pad.inuse() {
                Some(pad.binding.actor().id().clone())
            } else {
                None
            }
        }),
        movement_frame: ps.pmove_framecount,
        jump_pad_frame: ps.jumppad_frame,
    }
}

/// PMove's scalar results return to the source record; body vectors already
/// belong to the shared body table.
pub fn write_q3_movement_state(entity: &RecordsEntityRef, state: &Q3MovementState, records: &Q3EntityRecords) {
    let ground_number = source_number(&state.ground, records);
    let jump_pad = state.jump_pad.as_ref().map_or(0, |actor| {
        records
            .native_by_actor(Some(actor))
            .map(|entity| entity.borrow().slot as i32)
            .unwrap_or(0)
    });
    let borrowed = entity.borrow();
    let client = borrowed
        .client
        .as_ref()
        .expect("Q3 player state requires an admitted client");
    let mut client = client.borrow_mut();
    let ps = &mut client.ps;
    ps.command_time = state.command_time_milliseconds;
    ps.pm_type = state.movement_type;
    ps.bob_cycle = state.bob_cycle;
    ps.pm_flags = state.movement_flags;
    ps.pm_time = state.movement_time_milliseconds;
    ps.gravity = state.gravity as i32;
    ps.speed = state.speed as i32;
    ps.delta_angles.x = state.delta_angle_words[0] as f32;
    ps.delta_angles.y = state.delta_angle_words[1] as f32;
    ps.delta_angles.z = state.delta_angle_words[2] as f32;
    ps.ground_entity_num = ground_number;
    ps.movement_dir = state.movement_direction;
    ps.grapple_point = state.grapple_point;
    ps.e_flags = state.flags;
    ps.viewangles = state.view_angles;
    ps.viewheight = state.view_height as i32;
    ps.pmove_framecount = state.movement_frame;
    ps.jumppad_frame = state.jump_pad_frame;
    ps.jumppad_ent = jump_pad;
    // The ordered effects append to ps.events at the caller, maintaining its
    // two-entry native ring.
}

/// Write selected character animation into the source record; non-Q3
/// animation states are ignored like the donor.
pub fn write_q3_character_animation(entity: &RecordsEntityRef, animation: &ActorAnimationState) {
    let AnimationState::Q3 {
        legs,
        torso,
        legs_timer_milliseconds,
        torso_timer_milliseconds,
    } = animation.state
    else {
        return;
    };
    let borrowed = entity.borrow();
    let client = borrowed
        .client
        .as_ref()
        .expect("Q3 player state requires an admitted client");
    let mut client = client.borrow_mut();
    let ps = &mut client.ps;
    ps.legs_anim = legs;
    ps.torso_anim = torso;
    ps.legs_timer = legs_timer_milliseconds;
    ps.torso_timer = torso_timer_milliseconds;
}

/// Layer source powerup words over a base movement environment.
#[must_use]
pub fn read_q3_movement_environment(entity: &RecordsEntityRef, base: &MovementEnvironment) -> MovementEnvironment {
    let borrowed = entity.borrow();
    let client = borrowed
        .client
        .as_ref()
        .expect("Q3 player state requires an admitted client");
    let client = client.borrow();
    let ps = &client.ps;
    MovementEnvironment {
        health: f64::from(ps.health()),
        flight: ps.powerups.get(Powerup::PwFlight as usize) != 0,
        haste: ps.powerups.get(Powerup::PwHaste as usize) != 0,
        invulnerable: base.invulnerable
            || ps.product() == Product::Missionpack && ps.powerups.get(Powerup::PwInvulnerability as usize) != 0,
        ..*base
    }
}

/// Item pickups and holdable use change these fields between weapon commands.
#[must_use]
pub fn read_q3_arsenal_runtime(entity: &RecordsEntityRef, previous: &Q3ArsenalRuntimeState) -> Q3ArsenalRuntimeState {
    let borrowed = entity.borrow();
    let client = borrowed
        .client
        .as_ref()
        .expect("Q3 player state requires an admitted client");
    let client = client.borrow();
    let ps = &client.ps;
    let schema = stat_schema(ps.product());
    let holdable_item = ps.stats.get(schema.holdable_item());
    Q3ArsenalRuntimeState {
        product: ps.product(),
        max_health: f64::from(ps.stats.get(schema_max_health(schema))),
        spectator: ps.pm_type == MoveType::PmSpectator as i32 || client.sess.session_team == Team::TeamSpectator,
        persistent_powerup_tag: schema_persistent_powerup(schema)
            .map(|slot| item_tag(ps.product(), ps.stats.get(slot)))
            .unwrap_or(0),
        holdable_item,
        holdable_tag: item_tag(ps.product(), holdable_item),
        respawned: ps.pm_flags & move_flags::RESPAWNED != 0,
        use_item_held: ps.pm_flags & move_flags::USE_ITEM_HELD != 0,
        event_sequence: ps.event_sequence,
        ..previous.clone()
    }
}

/// Write holdable and weapon-flag words back to the source record.
pub fn write_q3_arsenal_runtime(entity: &RecordsEntityRef, runtime: &Q3ArsenalRuntimeState) {
    let borrowed = entity.borrow();
    let client = borrowed
        .client
        .as_ref()
        .expect("Q3 player state requires an admitted client");
    let mut client = client.borrow_mut();
    let schema = stat_schema(client.ps.product());
    client.ps.stats.set(schema.holdable_item(), runtime.holdable_item);
    client.ps.pm_flags = (client.ps.pm_flags & !(move_flags::RESPAWNED | move_flags::USE_ITEM_HELD))
        | if runtime.respawned { move_flags::RESPAWNED } else { 0 }
        | if runtime.use_item_held {
            move_flags::USE_ITEM_HELD
        } else {
            0
        };
}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};
    use std::collections::HashMap;
    use std::rc::Rc;

    use qa_content::contract::ItemId;
    use qa_content::q3::base::records::{
        ActorCallbacks, CombatState, DamageAdmissionFn, DamageRequest, EntityRef, GameClient, Q3ActorCallbacks,
        Q3BaseError, Q3DamageCall, Q3EntityRecords, Q3RecordHost, Q3SessionActors, Q3SessionBodies, Q3SessionCombat,
        Q3SessionInventory,
    };
    use qa_content::q3::base::shared::definitions::Product;
    use qa_core::identity::{ActorId, IdentityOwner, OwnedActor, ProviderId};
    use qa_core::math::{vec3, Bounds, Vec3};
    use qa_world::body::BodyState;

    use super::*;

    struct FakeActors {
        owner: IdentityOwner,
        owned: RefCell<HashMap<ActorId, OwnedActor>>,
        live: RefCell<Vec<ActorId>>,
        next_generation: Cell<u32>,
    }

    impl FakeActors {
        fn new() -> Self {
            Self {
                owner: IdentityOwner::create("q3-player-state").unwrap(),
                owned: RefCell::new(HashMap::new()),
                live: RefCell::new(Vec::new()),
                next_generation: Cell::new(1),
            }
        }
    }

    impl Q3SessionActors for FakeActors {
        fn assert_owned(&self, actor: &OwnedActor) -> Result<(), Q3BaseError> {
            if self.owner.owns_owned(actor) {
                Ok(())
            } else {
                Err(Q3BaseError::Invalid("foreign actor".to_string()))
            }
        }

        fn allocate_at_source(&self, provider: &ProviderId, slot: usize, _definition: &str) -> OwnedActor {
            let generation = self.next_generation.get();
            self.next_generation.set(generation + 1);
            let id = self.owner.actor(slot as u32, generation);
            let owned = self.owner.owned_actor(&id, provider.clone()).unwrap();
            self.owned.borrow_mut().insert(id.clone(), owned.clone());
            self.live.borrow_mut().push(id);
            owned
        }

        fn is_live(&self, actor: &ActorId) -> bool {
            self.live.borrow().contains(actor)
        }

        fn on_release(&self, _callback: Box<dyn Fn(&OwnedActor)>) -> Box<dyn Fn()> {
            Box::new(|| {})
        }

        fn release(&self, actor: &OwnedActor) {
            self.live.borrow_mut().retain(|id| id != actor.id());
        }

        fn resolve_owned(&self, actor: &ActorId) -> Option<OwnedActor> {
            self.owned.borrow().get(actor).cloned()
        }
    }

    fn zero_body() -> BodyState {
        BodyState {
            origin: vec3(0.0, 0.0, 0.0),
            angles: vec3(0.0, 0.0, 0.0),
            velocity: vec3(0.0, 0.0, 0.0),
            bounds: Bounds {
                min: vec3(-15.0, -15.0, -24.0),
                max: vec3(15.0, 15.0, 32.0),
            },
            ground: None,
        }
    }

    struct FakeBodies {
        states: RefCell<HashMap<ActorId, BodyState>>,
    }

    impl Q3SessionBodies for FakeBodies {
        fn create(&self, actor: &OwnedActor, state: BodyState) {
            self.states.borrow_mut().insert(actor.id().clone(), state);
        }

        fn read(&self, actor: &ActorId) -> Option<BodyState> {
            self.states.borrow().get(actor).cloned()
        }

        fn write(&self, actor: &OwnedActor, state: BodyState) {
            self.states.borrow_mut().insert(actor.id().clone(), state);
        }

        fn linked(&self, _actor: &ActorId) -> Option<qa_world::body::LinkedBody> {
            None
        }

        fn link(&self, _actor: &OwnedActor, _origin: Option<Vec3>) {}

        fn unlink(&self, _actor: &OwnedActor) {}
    }

    struct FakeCombat {
        states: RefCell<HashMap<ActorId, CombatState>>,
    }

    impl Q3SessionCombat for FakeCombat {
        fn read(&self, actor: &ActorId) -> Option<CombatState> {
            self.states.borrow().get(actor).cloned()
        }

        fn create(&self, actor: &OwnedActor, initial: CombatState, _admit_damage: Option<DamageAdmissionFn>) {
            self.states.borrow_mut().insert(actor.id().clone(), initial);
        }

        fn set_health(&self, actor: &OwnedActor, health: i32) {
            if let Some(state) = self.states.borrow_mut().get_mut(actor.id()) {
                state.health = health;
            }
        }

        fn set_can_take_damage(&self, actor: &OwnedActor, can_take_damage: bool) {
            if let Some(state) = self.states.borrow_mut().get_mut(actor.id()) {
                state.can_take_damage = can_take_damage;
            }
        }

        fn set_regular_points(
            &self,
            _actor: &OwnedActor,
            _points: i32,
            _initial: qa_content::q3::base::records::RegularArmorState,
        ) {
        }

        fn bind_damage_admission(&self, _actor: &OwnedActor, _admit_damage: DamageAdmissionFn) {}

        fn apply(&self, request: DamageRequest) -> qa_content::q3::base::records::DamageOutcome {
            qa_content::q3::base::records::DamageOutcome::StaleTarget { request }
        }
    }

    struct FakeInventory;
    impl Q3SessionInventory for FakeInventory {
        fn has(&self, _actor: &ActorId) -> bool {
            true
        }

        fn create(&self, _actor: &OwnedActor, _entries: Vec<qa_content::q3::base::records::InventoryEntry>) {}

        fn count(&self, _actor: &ActorId, _item: &ItemId) -> i32 {
            0
        }

        fn configure(&self, _actor: &OwnedActor, _item: &ItemId, _count: i32, _capacity: i32) {}
    }

    struct FakeCallbacks;
    impl Q3ActorCallbacks for FakeCallbacks {
        fn bind(&self, _actor: &OwnedActor, _callbacks: ActorCallbacks) {}
    }

    struct FakeRecordHost {
        actors: Rc<FakeActors>,
        bodies: Rc<FakeBodies>,
        combat: Rc<FakeCombat>,
        inventory: Rc<FakeInventory>,
        callbacks: Rc<FakeCallbacks>,
    }

    impl Q3RecordHost for FakeRecordHost {
        fn actors(&self) -> Rc<dyn Q3SessionActors> {
            self.actors.clone()
        }

        fn bodies(&self) -> Rc<dyn Q3SessionBodies> {
            self.bodies.clone()
        }

        fn combat(&self) -> Rc<dyn Q3SessionCombat> {
            self.combat.clone()
        }

        fn inventory(&self) -> Rc<dyn Q3SessionInventory> {
            self.inventory.clone()
        }

        fn callbacks(&self) -> Rc<dyn Q3ActorCallbacks> {
            self.callbacks.clone()
        }

        fn schedule(&self, _actor: &OwnedActor, _due_milliseconds: Option<i32>) {}

        fn run_think(&self, _actor: &OwnedActor, _time_milliseconds: i32) {}

        fn damage_call(&self) -> Option<Q3DamageCall> {
            None
        }

        fn foreign(&self, _actor: &ActorId) -> Option<EntityRef> {
            None
        }

        fn is_player(&self, _actor: &ActorId) -> bool {
            true
        }
    }

    fn records() -> (Rc<FakeRecordHost>, Q3EntityRecords) {
        let host = Rc::new(FakeRecordHost {
            actors: Rc::new(FakeActors::new()),
            bodies: Rc::new(FakeBodies {
                states: RefCell::new(HashMap::new()),
            }),
            combat: Rc::new(FakeCombat {
                states: RefCell::new(HashMap::new()),
            }),
            inventory: Rc::new(FakeInventory),
            callbacks: Rc::new(FakeCallbacks),
        });
        let records = Q3EntityRecords::new(
            host.clone(),
            ProviderId::new("q3", "player-state-test"),
            Product::Baseq3,
        );
        (host, records)
    }

    fn player(host: &FakeRecordHost, records: &Q3EntityRecords, slot: usize) -> EntityRef {
        let actor = host
            .actors
            .allocate_at_source(&ProviderId::new("q3", "player-state-test"), slot, "test");
        let entity = records.attach(slot, actor, true).unwrap();
        entity.borrow_mut().client = Some(Rc::new(RefCell::new(GameClient::new(Product::Baseq3, None, None))));
        entity
    }

    #[test]
    fn movement_state_round_trip() {
        let (host, records) = records();
        let _ = records.activate(1022);
        let entity = player(&host, &records, 0);
        {
            let borrowed = entity.borrow();
            let mut client = borrowed.client.as_ref().unwrap().borrow_mut();
            let ps = &mut client.ps;
            ps.command_time = 111;
            ps.pm_type = MoveType::PmNormal as i32;
            ps.set_origin(vec3(1.0, 2.0, 3.0));
            ps.set_velocity(vec3(4.0, 5.0, 6.0));
            ps.gravity = 800;
            ps.speed = 320;
            ps.delta_angles = vec3(7.0, 8.0, 9.0);
            ps.movement_dir = 3;
            ps.grapple_point = vec3(0.0, 0.0, 1.0);
            ps.e_flags = 17;
            ps.viewangles = vec3(0.0, 90.0, 0.0);
            ps.viewheight = 26;
            ps.event_sequence = 41;
            ps.pmove_framecount = 400;
            ps.jumppad_frame = 401;
        }
        let state = read_q3_movement_state(&entity, &records);
        assert_eq!(state.command_time_milliseconds, 111);
        assert_eq!(state.gravity, 800.0);
        assert_eq!(state.delta_angle_words, [7, 8, 9]);
        assert_eq!(state.view_height, 26.0);
        assert!(matches!(state.ground, TraceHit::None));
        let mut updated = state;
        updated.command_time_milliseconds = 222;
        updated.view_height = 22.0;
        updated.movement_direction = 5;
        write_q3_movement_state(&entity, &updated, &records);
        let ps = entity.borrow().client.as_ref().unwrap().borrow().ps.command_time;
        assert_eq!(ps, 222);
        let reread = read_q3_movement_state(&entity, &records);
        assert_eq!(reread.view_height, 22.0);
        assert_eq!(reread.movement_direction, 5);
    }

    #[test]
    fn ground_and_jump_pad_resolve_through_records() {
        let (host, records) = records();
        let world = records.activate(1022);
        let world_actor = world.borrow().binding.actor().id().clone();
        let entity = player(&host, &records, 1);
        let pad = player(&host, &records, 5);
        let pad_actor = pad.borrow().binding.actor().id().clone();
        {
            let actor = entity.borrow().binding.actor().id().clone();
            let mut body = zero_body();
            body.ground = Some(world_actor);
            host.bodies.states.borrow_mut().insert(actor, body);
            entity.borrow().client.as_ref().unwrap().borrow_mut().ps.jumppad_ent = 5;
        }
        let state = read_q3_movement_state(&entity, &records);
        assert!(matches!(state.ground, TraceHit::World { model: 0 }));
        assert_eq!(state.jump_pad, Some(pad_actor));
        let mut updated = state;
        updated.ground = TraceHit::None;
        updated.jump_pad = None;
        write_q3_movement_state(&entity, &updated, &records);
        let ps = entity.borrow().client.as_ref().unwrap().borrow().ps.ground_entity_num;
        assert_eq!(ps, 1023);
    }

    #[test]
    fn animation_environment_and_arsenal_bridges() {
        let (host, records) = records();
        let entity = player(&host, &records, 2);
        {
            let borrowed = entity.borrow();
            let mut client = borrowed.client.as_ref().unwrap().borrow_mut();
            let ps = &mut client.ps;
            ps.stats.set(0, 80);
            ps.powerups.set(Powerup::PwFlight as usize, 9999);
        }
        write_q3_character_animation(
            &entity,
            &ActorAnimationState {
                provider: ProviderId::new("q3", "character"),
                state: AnimationState::Q3 {
                    legs: 3,
                    torso: 4,
                    legs_timer_milliseconds: 100,
                    torso_timer_milliseconds: 200,
                },
            },
        );
        {
            let borrowed = entity.borrow();
            let ps = borrowed.client.as_ref().unwrap().borrow().ps.legs_anim;
            let torso = borrowed.client.as_ref().unwrap().borrow().ps.torso_anim;
            assert_eq!((ps, torso), (3, 4));
            let timers = borrowed.client.as_ref().unwrap().borrow().ps.legs_timer;
            let torso_timer = borrowed.client.as_ref().unwrap().borrow().ps.torso_timer;
            assert_eq!((timers, torso_timer), (100, 200));
        }
        write_q3_character_animation(
            &entity,
            &ActorAnimationState {
                provider: ProviderId::new("q1", "character"),
                state: AnimationState::Q1 {
                    frame: 1,
                    next_frame_seconds: 0.1,
                },
            },
        );
        let environment = read_q3_movement_environment(&entity, &MovementEnvironment::default());
        assert_eq!(environment.health, 80.0);
        assert!(environment.flight);
        assert!(!environment.invulnerable);
        let previous = qa_content::q3::foundation::arsenal::q3_spawn_arsenal_runtime(Product::Baseq3, 100.0, 0);
        let runtime = read_q3_arsenal_runtime(&entity, &previous);
        assert!(!runtime.spectator);
        let mut flipped = runtime;
        flipped.respawned = true;
        flipped.holdable_item = 2;
        write_q3_arsenal_runtime(&entity, &flipped);
        let borrowed = entity.borrow();
        let ps = borrowed.client.as_ref().unwrap().borrow().ps.pm_flags;
        assert_ne!(ps & move_flags::RESPAWNED, 0);
    }

    #[test]
    #[should_panic(expected = "Q3 player state requires an admitted client")]
    fn missing_client_panics() {
        let (_host, records) = records();
        let entity = records.activate(3);
        let _ = read_q3_movement_state(&entity, &records);
    }
}
