//! Quake III base-game root (`q3_base`) tests, awaiting distribution.
//!
//! Production unified into records.rs, combat_bridge.rs, and the merged
//! canonicals; these tests move to their canonical homes at file deletion.

#[cfg(test)]
mod tests {
    use crate::contract::{ItemId, ProtectionChannel};
    use qa_core::math::{vec3, Bounds, Vec3};
    use qa_world::combat::{Delivery, Reaction};
    use std::rc::Rc;

    use crate::q3::base::combat_bridge::*;
    use crate::q3::base::game::state::{GameFlags, MAX_CLIENTS, MAX_GENTITIES};
    use crate::q3::base::map_spawns::*;
    use crate::q3::base::records::*;
    use crate::q3::base::shared::definitions::*;
    use crate::q3::base::shared::entity_shared::*;
    use crate::q3::base::shared::entity_state::*;
    use crate::q3::base::shared::items::{
        can_item_be_grabbed, can_q3_armor_be_grabbed, find_item, find_item_for_holdable, find_item_for_powerup,
        find_item_for_weapon, item_at, item_list, player_touches_item, PickupEntity, PlayerInventory,
        Trajectory as ItemsTrajectory, TrajectoryType as ItemsTrajectoryType,
    };
    use crate::q3::base::shared::player_state::*;
    use crate::q3::base::world::*;
    use crate::q3::foundation::arsenal::q3_weapon_item;
    use crate::value::arr;
    use qa_core::cvar::CvarRegistry;
    use qa_core::identity::ActorId;
    use qa_core::identity::OwnedActor;
    use qa_core::identity::ProviderId;
    use qa_world::body::{BodyState, LinkedBody};

    use qa_core::time::SourceTime;
    use std::cell::{Cell, RefCell};
    use std::collections::HashMap;

    use crate::q3::base::bot_debug::*;

    use crate::q3::base::settings::*;

    use crate::q3::base::shared::direction_byte::*;

    use crate::q3::base::shared::jump_pad::*;

    use crate::q3::base::shared::snapshot_state::*;
    use crate::q3::base::shared::trajectory::*;

    use crate::q3::base::world_adapter::*;
    use crate::q3::product_restriction::*;

    use qa_core::cmd::Dialect;
    use qa_core::identity::IdentityOwner;

    fn test_owner() -> IdentityOwner {
        IdentityOwner::create("q3_base_test").expect("owner")
    }

    fn test_provider() -> ProviderId {
        ProviderId::new("q3", "test")
    }

    // -- definitions ------------------------------------------------------

    #[test]
    fn stat_schemas_match_source_slots() {
        let StatSchema::Base(base) = stat_schema(Product::Baseq3) else {
            panic!("base product must yield the base stat layout");
        };
        assert_eq!(base.product, Product::Baseq3);
        assert_eq!((base.health, base.weapons, base.armor, base.max_health), (0, 2, 3, 6));
        let StatSchema::Missionpack(pack) = stat_schema(Product::Missionpack) else {
            panic!("missionpack product must yield the missionpack stat layout");
        };
        assert_eq!(pack.product, Product::Missionpack);
        assert_eq!(pack.persistent_powerup, 2);
        assert_eq!((pack.health, pack.weapons, pack.armor, pack.max_health), (0, 3, 4, 7));
    }

    #[test]
    fn weapon_availability_follows_product() {
        assert_eq!(weapon_count(Product::Baseq3), 11);
        assert_eq!(weapon_count(Product::Missionpack), 14);
        assert!(weapon_available(Product::Baseq3, Weapon::WpGrapplingHook));
        assert!(!weapon_available(Product::Baseq3, Weapon::WpNailgun));
        assert!(weapon_available(Product::Missionpack, Weapon::WpChaingun));
        assert!(!weapon_available(Product::Missionpack, Weapon::WpNone));
        assert_eq!(GameType::GtTeam as i32, 3);
        assert_eq!(Team::TeamSpectator as i32, 3);
        assert_eq!(EntityType::EtEvents as i32, 13);
        assert_eq!(EV_EVENT_BITS, EV_EVENT_BIT1 | EV_EVENT_BIT2);
    }

    // -- trajectory -------------------------------------------------------

    fn linear_fixture() -> Trajectory {
        Trajectory {
            trajectory_type: TrajectoryType::TrLinear,
            time: 1000,
            duration: 0,
            base: vec3(1.0, 2.0, 3.0),
            delta: vec3(100.0, 0.0, -50.0),
        }
    }

    #[test]
    fn linear_trajectory_scales_by_seconds() {
        let at = evaluate_trajectory(&linear_fixture(), 1500);
        assert_eq!(at, vec3(51.0, 2.0, -22.0));
        assert_eq!(
            evaluate_trajectory_delta(&linear_fixture(), 9999),
            vec3(100.0, 0.0, -50.0)
        );
    }

    #[test]
    fn gravity_trajectory_falls_quadratically() {
        let tr = Trajectory {
            trajectory_type: TrajectoryType::TrGravity,
            ..linear_fixture()
        };
        let at = evaluate_trajectory(&tr, 2000);
        assert_eq!(at.x, 101.0);
        assert_eq!(at.z, 3.0 - 50.0 - 400.0);
        let delta = evaluate_trajectory_delta(&tr, 2000);
        assert_eq!(delta, vec3(100.0, 0.0, -850.0));
    }

    #[test]
    fn sine_and_stop_trajectories_match_source() {
        let sine = Trajectory {
            trajectory_type: TrajectoryType::TrSine,
            time: 0,
            duration: 1000,
            base: vec3(0.0, 0.0, 0.0),
            delta: vec3(0.0, 0.0, 10.0),
        };
        assert_eq!(evaluate_trajectory(&sine, 0), vec3(0.0, 0.0, 0.0));
        let stop = Trajectory {
            trajectory_type: TrajectoryType::TrLinearStop,
            time: 0,
            duration: 100,
            base: vec3(0.0, 0.0, 0.0),
            delta: vec3(10.0, 0.0, 0.0),
        };
        assert_eq!(evaluate_trajectory(&stop, 50), vec3(0.5, 0.0, 0.0));
        assert_eq!(evaluate_trajectory(&stop, 5000), vec3(1.0, 0.0, 0.0));
        assert_eq!(evaluate_trajectory_delta(&stop, 5000), vec3(0.0, 0.0, 0.0));
        let stationary = Trajectory::zero(TrajectoryType::TrStationary);
        assert_eq!(evaluate_trajectory(&stationary, 1234), vec3(0.0, 0.0, 0.0));
    }

    #[test]
    fn unknown_trajectory_tag_is_a_drop_error() {
        assert!(TrajectoryType::from_i32(5).is_ok());
        let error = TrajectoryType::from_i32(99).unwrap_err();
        assert!(matches!(error, Q3BaseError::Drop(_)));
    }

    // -- direction byte ---------------------------------------------------

    #[test]
    fn direction_table_round_trips() {
        assert_eq!(BYTE_DIRECTIONS.len(), NUM_VERTEX_NORMALS);
        assert_eq!(BYTE_DIRECTIONS[5], vec3(0.0, 0.0, 1.0));
        for (index, direction) in BYTE_DIRECTIONS.iter().enumerate() {
            assert_eq!(
                direction_to_byte(Some(*direction)),
                index,
                "entry {index} must win its own dot search"
            );
            assert_eq!(byte_to_direction(index as i32), *direction);
        }
    }

    #[test]
    fn direction_byte_edges_match_source() {
        assert_eq!(direction_to_byte(None), 0);
        assert_eq!(direction_to_byte(Some(vec3(0.0, 0.0, 0.0))), 0);
        assert_eq!(byte_to_direction(-1), ZERO_DIRECTION);
        assert_eq!(byte_to_direction(162), ZERO_DIRECTION);
        assert_eq!(byte_to_direction(999), ZERO_DIRECTION);
    }

    // -- entity state -----------------------------------------------------

    #[test]
    fn entity_state_copies_every_field() {
        let mut source = EntityState::new();
        source.number = 7;
        source.pos = linear_fixture();
        source.origin2 = vec3(1.0, 2.0, 3.0);
        let copy = source.copy();
        assert_eq!(copy, source);
        let mut target = EntityState::new();
        target.copy_from_state(&source);
        assert_eq!(target, source);
        assert_eq!(EntityState::default(), EntityState::new());
    }

    // -- player state -----------------------------------------------------

    #[test]
    fn player_state_defaults_match_retail() {
        let ps = create_player_state(Product::Baseq3, None);
        assert_eq!(ps.pm_type, MoveType::PmNormal as i32);
        assert_eq!(ps.weapon, Weapon::WpNone as i32);
        assert_eq!(ps.weapon_state, WeaponState::WeaponReady as i32);
        assert_eq!(ps.health(), 0);
    }

    #[test]
    fn player_health_uses_product_schema() {
        let mut base = create_player_state(Product::Baseq3, None);
        base.set_health(125);
        assert_eq!(base.stats.get(0), 125);
        let mut pack = create_player_state(Product::Missionpack, None);
        pack.set_health(200);
        assert_eq!(pack.health(), 200);
    }

    #[test]
    fn player_events_cycle_slots_and_sequence() {
        let mut ps = create_player_state(Product::Baseq3, None);
        let first = ps.add_event(13, 1);
        assert_eq!(
            first,
            PredictableEvent {
                sequence: 0,
                event: 13,
                parameter: 1
            }
        );
        let second = ps.add_event(14, 0);
        assert_eq!(second.sequence, 1);
        assert_eq!(ps.events.get(0), 13);
        assert_eq!(ps.event_parms.get(0), 1);
        assert_eq!(ps.events.get(1), 14);
        assert_eq!(ps.event_sequence, 2);
    }

    struct DebugSink {
        text: String,
        lines: RefCell<Vec<String>>,
    }

    impl PredictableEventDebug for DebugSink {
        fn module(&self) -> EventDebugModule {
            EventDebugModule::Game
        }

        fn show_events(&self) -> String {
            self.text.clone()
        }

        fn print(&self, message: &str) {
            self.lines.borrow_mut().push(message.to_string());
        }
    }

    #[test]
    fn player_event_debug_prints_source_line() {
        let mut ps = create_player_state(Product::Baseq3, None);
        let sink = Rc::new(DebugSink {
            text: "1".to_string(),
            lines: RefCell::new(Vec::new()),
        });
        ps.set_event_debug(Some(sink.clone()));
        ps.pmove_framecount = 41;
        ps.add_event(EntityEvent::EvJumpPad as i32, 1);
        let lines = sink.lines.borrow();
        assert_eq!(lines.len(), 1);
        assert!(lines[0].contains("EV_JUMP_PAD"), "{}", lines[0]);
        assert!(lines[0].contains("parm 1"), "{}", lines[0]);
    }

    #[test]
    fn player_copy_preserves_authority_words() {
        let mut ps = create_player_state(Product::Baseq3, None);
        ps.set_origin(vec3(1.0, 2.0, 3.0));
        ps.set_health(90);
        let copy = ps.copy();
        assert_eq!(copy.origin(), vec3(1.0, 2.0, 3.0));
        assert_eq!(copy.health(), 90);
        let mut other = create_player_state(Product::Missionpack, None);
        other.copy_from(&ps, AuthorityStores::ReplaceAuthority);
        assert_eq!(other.product(), Product::Baseq3);
        assert_eq!(other.origin(), vec3(1.0, 2.0, 3.0));
    }

    // -- items ------------------------------------------------------------

    struct FixedInventory {
        product: Product,
        health: i32,
        armor: i32,
        max_health: i32,
        holdable_item: i32,
        team: i32,
        ammo: i32,
        powerup: i32,
        persistent: i32,
    }

    impl PlayerInventory for FixedInventory {
        fn product(&self) -> Product {
            self.product
        }

        fn health(&self) -> i32 {
            self.health
        }

        fn armor(&self) -> i32 {
            self.armor
        }

        fn max_health(&self) -> i32 {
            self.max_health
        }

        fn holdable_item(&self) -> i32 {
            self.holdable_item
        }

        fn team(&self) -> i32 {
            self.team
        }

        fn ammo(&self, _weapon: Weapon) -> i32 {
            self.ammo
        }

        fn powerup(&self, _powerup: Powerup) -> i32 {
            self.powerup
        }

        fn persistent_powerup_index(&self) -> i32 {
            self.persistent
        }
    }

    fn base_inventory() -> FixedInventory {
        FixedInventory {
            product: Product::Baseq3,
            health: 100,
            armor: 0,
            max_health: 100,
            holdable_item: 0,
            team: Team::TeamFree as i32,
            ammo: 0,
            powerup: 0,
            persistent: 0,
        }
    }

    #[test]
    fn item_lists_split_at_the_missionpack_tail() {
        assert_eq!(item_list(Product::Baseq3).len(), 36);
        assert_eq!(item_list(Product::Missionpack).len(), 52);
        assert_eq!(item_at(Product::Baseq3, 0).unwrap().item_type(), ItemType::ItBad);
        assert_eq!(item_at(Product::Baseq3, 8).unwrap().pickup_name, Some("Gauntlet"));
        assert!(item_at(Product::Baseq3, 36).is_err());
        assert_eq!(item_at(Product::Missionpack, 51).unwrap().pickup_name, Some("Chaingun"));
    }

    #[test]
    fn item_finds_cover_names_tags_and_errors() {
        assert_eq!(
            find_item(Product::Baseq3, "quad damage").unwrap().class_name,
            Some("item_quad")
        );
        assert!(find_item(Product::Baseq3, "missing").is_none());
        assert_eq!(
            find_item_for_powerup(Product::Baseq3, Powerup::PwFlight)
                .unwrap()
                .class_name,
            Some("item_flight")
        );
        assert_eq!(
            find_item_for_holdable(Product::Baseq3, Holdable::HiMedkit)
                .unwrap()
                .class_name,
            Some("holdable_medkit")
        );
        assert!(find_item_for_holdable(Product::Baseq3, Holdable::HiNumHoldable).is_err());
        assert_eq!(
            find_item_for_weapon(Product::Baseq3, Weapon::WpShotgun)
                .unwrap()
                .class_name,
            Some("weapon_shotgun")
        );
        assert!(find_item_for_weapon(Product::Baseq3, Weapon::WpNone).is_err());
    }

    #[test]
    fn grab_rules_match_bg_canitemgrabbed() {
        let ps = base_inventory();
        let weapon = PickupEntity {
            model_index: 10,
            model_index2: 0,
            generic1: 0,
        };
        assert!(can_item_be_grabbed(0, &weapon, &ps).unwrap());
        let mut full_ammo = base_inventory();
        full_ammo.ammo = 200;
        let ammo = PickupEntity {
            model_index: 18,
            model_index2: 0,
            generic1: 0,
        };
        assert!(!can_item_be_grabbed(0, &ammo, &full_ammo).unwrap());
        let mut hurt = base_inventory();
        hurt.health = 50;
        let health = PickupEntity {
            model_index: 5,
            model_index2: 0,
            generic1: 0,
        };
        assert!(can_item_be_grabbed(0, &health, &hurt).unwrap());
        assert!(!can_item_be_grabbed(0, &health, &ps).unwrap());
        let mut red = base_inventory();
        red.team = Team::TeamRed as i32;
        let blue_flag = PickupEntity {
            model_index: 35,
            model_index2: 0,
            generic1: 0,
        };
        assert!(can_item_be_grabbed(GameType::GtCtf as i32, &blue_flag, &red).unwrap());
        let red_flag = PickupEntity {
            model_index: 34,
            model_index2: 0,
            generic1: 0,
        };
        assert!(!can_item_be_grabbed(GameType::GtCtf as i32, &red_flag, &red).unwrap());
        let mut holding = base_inventory();
        holding.holdable_item = 1;
        let teleporter = PickupEntity {
            model_index: 26,
            model_index2: 0,
            generic1: 0,
        };
        assert!(!can_item_be_grabbed(0, &teleporter, &holding).unwrap());
        let bad = PickupEntity {
            model_index: 0,
            model_index2: 0,
            generic1: 0,
        };
        assert!(can_item_be_grabbed(0, &bad, &ps).is_err());
    }

    #[test]
    fn armor_grab_rules_cover_scout_and_guard() {
        let mut scout = base_inventory();
        scout.product = Product::Missionpack;
        scout.persistent = 42;
        assert!(!can_q3_armor_be_grabbed(&scout).unwrap());
        let mut guard = base_inventory();
        guard.product = Product::Missionpack;
        guard.persistent = 43;
        guard.armor = 100;
        assert!(!can_q3_armor_be_grabbed(&guard).unwrap());
        guard.armor = 99;
        assert!(can_q3_armor_be_grabbed(&guard).unwrap());
        let mut plain = base_inventory();
        plain.armor = 199;
        assert!(can_q3_armor_be_grabbed(&plain).unwrap());
        plain.armor = 200;
        assert!(!can_q3_armor_be_grabbed(&plain).unwrap());
    }

    #[test]
    fn player_touch_uses_source_bounds() {
        let item = ItemsTrajectory {
            trajectory_type: ItemsTrajectoryType::TrStationary as i32,
            time: 0,
            duration: 0,
            base: vec3(0.0, 0.0, 0.0),
            delta: vec3(0.0, 0.0, 0.0),
        };
        assert!(player_touches_item(vec3(0.0, 0.0, 0.0), &item, 0).unwrap());
        assert!(!player_touches_item(vec3(45.0, 0.0, 0.0), &item, 0).unwrap());
        assert!(!player_touches_item(vec3(0.0, 37.0, 0.0), &item, 0).unwrap());
    }

    // -- jump pad / snapshot ----------------------------------------------

    #[test]
    fn jump_pad_applies_velocity_and_event() {
        let mut ps = create_player_state(Product::Baseq3, None);
        ps.pmove_framecount = 9;
        let mut pad = EntityState::new();
        pad.number = 12;
        pad.origin2 = vec3(0.0, 0.0, 700.0);
        touch_jump_pad(&mut ps, &pad);
        assert_eq!(ps.velocity(), vec3(0.0, 0.0, 700.0));
        assert_eq!(ps.jumppad_ent, 12);
        assert_eq!(ps.jumppad_frame, 9);
        assert_eq!(ps.events.get(0), EntityEvent::EvJumpPad as i32);
        assert_eq!(ps.event_parms.get(0), 1);
    }

    #[test]
    fn jump_pad_ignores_flight_and_dead() {
        let mut ps = create_player_state(Product::Baseq3, None);
        ps.powerups.set(Powerup::PwFlight as usize, 9999);
        let mut pad = EntityState::new();
        pad.origin2 = vec3(0.0, 0.0, 700.0);
        touch_jump_pad(&mut ps, &pad);
        assert_eq!(ps.velocity(), vec3(0.0, 0.0, 0.0));
        let mut dead = create_player_state(Product::Baseq3, None);
        dead.pm_type = MoveType::PmDead as i32;
        touch_jump_pad(&mut dead, &pad);
        assert_eq!(dead.jumppad_ent, 0);
    }

    #[test]
    fn snapshot_conversion_consumes_events_and_snaps() {
        let mut ps = create_player_state(Product::Baseq3, None);
        ps.client_num = 3;
        ps.set_origin(vec3(10.7, -4.2, 0.5));
        ps.set_health(100);
        ps.add_event(EntityEvent::EvJump as i32, 0);
        ps.powerups.set(Powerup::PwQuad as usize, 30);
        let mut entity = EntityState::new();
        player_state_to_entity_state(&mut ps, &mut entity, true);
        assert_eq!(entity.e_type, EntityType::EtPlayer as i32);
        assert_eq!(entity.number, 3);
        assert_eq!(entity.pos.base, vec3(10.0, -4.0, 0.0));
        assert_eq!(entity.event, EntityEvent::EvJump as i32);
        assert_eq!(entity.powerups, 1 << (Powerup::PwQuad as i32));
        assert_eq!(ps.entity_event_sequence, 1);
        let mut gibbed = create_player_state(Product::Baseq3, None);
        gibbed.set_health(GIB_HEALTH);
        let mut hidden = EntityState::new();
        player_state_to_entity_state(&mut gibbed, &mut hidden, false);
        assert_eq!(hidden.e_type, EntityType::EtInvisible as i32);
    }

    #[test]
    fn snapshot_extrapolation_uses_linear_stop() {
        let mut ps = create_player_state(Product::Baseq3, None);
        ps.set_origin(vec3(0.0, 0.0, 0.0));
        ps.set_velocity(vec3(100.0, 0.0, 0.0));
        ps.set_health(100);
        let mut entity = EntityState::new();
        player_state_to_entity_state_extra_polate(&mut ps, &mut entity, 500, false);
        assert_eq!(entity.pos.trajectory_type, TrajectoryType::TrLinearStop);
        assert_eq!(entity.pos.time, 500);
        assert_eq!(entity.pos.duration, 50);
        let moved: MovementTrace = ServerTraceResult {
            fraction: 1.0,
            end: vec3(0.0, 0.0, 0.0),
            entity_num: ENTITYNUM_NONE,
            solidity: TraceSolidity::Clear,
            contact: TraceContact::None,
            contents: 0,
            surface_flags: 0,
        };
        assert_eq!(moved.entity_num, ENTITYNUM_NONE);
    }

    // -- session fakes ------------------------------------------------------

    #[allow(clippy::type_complexity)]
    struct FakeActors {
        owner: IdentityOwner,
        owned: RefCell<HashMap<ActorId, OwnedActor>>,
        live: RefCell<Vec<ActorId>>,
        watchers: RefCell<Vec<Box<dyn Fn(&OwnedActor)>>>,
        next_generation: Cell<u32>,
    }

    impl FakeActors {
        fn new() -> Self {
            Self {
                owner: test_owner(),
                owned: RefCell::new(HashMap::new()),
                live: RefCell::new(Vec::new()),
                watchers: RefCell::new(Vec::new()),
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
            let owned = self.owner.owned_actor(&id, provider.clone()).expect("owned");
            self.owned.borrow_mut().insert(id.clone(), owned.clone());
            self.live.borrow_mut().push(id);
            owned
        }

        fn is_live(&self, actor: &ActorId) -> bool {
            self.live.borrow().contains(actor)
        }

        fn on_release(&self, callback: Box<dyn Fn(&OwnedActor)>) -> Box<dyn Fn()> {
            self.watchers.borrow_mut().push(callback);
            Box::new(|| {})
        }

        fn release(&self, actor: &OwnedActor) {
            self.live.borrow_mut().retain(|id| id != actor.id());
            for watcher in self.watchers.borrow().iter() {
                watcher(actor);
            }
        }

        fn resolve_owned(&self, actor: &ActorId) -> Option<OwnedActor> {
            self.owned.borrow().get(actor).cloned()
        }
    }

    struct FakeBodies {
        states: RefCell<HashMap<ActorId, BodyState>>,
        linked: RefCell<HashMap<ActorId, LinkedBody>>,
    }

    impl FakeBodies {
        fn new() -> Self {
            Self {
                states: RefCell::new(HashMap::new()),
                linked: RefCell::new(HashMap::new()),
            }
        }
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

        fn linked(&self, actor: &ActorId) -> Option<LinkedBody> {
            self.linked.borrow().get(actor).cloned()
        }

        fn link(&self, actor: &OwnedActor, origin: Option<Vec3>) {
            let mut state = self
                .states
                .borrow()
                .get(actor.id())
                .cloned()
                .unwrap_or_else(|| ZERO_BODY.clone());
            if let Some(origin) = origin {
                state.origin = origin;
            }
            let count = self
                .linked
                .borrow()
                .get(actor.id())
                .map_or(1, |linked| linked.link_count + 1);
            self.linked.borrow_mut().insert(
                actor.id().clone(),
                LinkedBody {
                    actor: actor.id().clone(),
                    state: state.clone(),
                    absolute_bounds: state.bounds,
                    link_count: count,
                },
            );
            self.states.borrow_mut().insert(actor.id().clone(), state);
        }

        fn unlink(&self, actor: &OwnedActor) {
            self.linked.borrow_mut().remove(actor.id());
        }
    }

    struct FakeCombat {
        states: RefCell<HashMap<ActorId, CombatState>>,
    }

    impl FakeCombat {
        fn new() -> Self {
            Self {
                states: RefCell::new(HashMap::new()),
            }
        }
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

        fn set_regular_points(&self, actor: &OwnedActor, points: i32, initial: RegularArmorState) {
            let mut states = self.states.borrow_mut();
            let state = states.get_mut(actor.id()).expect("combat state");
            state.armor.regular = match &state.armor.regular {
                RegularArmorState::None => initial,
                RegularArmorState::Q1 { absorption, item, .. } => RegularArmorState::Q1 {
                    points,
                    absorption: *absorption,
                    item: item.clone(),
                },
                RegularArmorState::Q2 {
                    normal_protection,
                    energy_protection,
                    item,
                    ..
                } => RegularArmorState::Q2 {
                    points,
                    normal_protection: *normal_protection,
                    energy_protection: *energy_protection,
                    item: item.clone(),
                },
                RegularArmorState::Q3 { protection, .. } => RegularArmorState::Q3 {
                    points,
                    protection: *protection,
                },
                RegularArmorState::Source { item, .. } => RegularArmorState::Source {
                    points,
                    item: item.clone(),
                },
            };
        }

        fn bind_damage_admission(&self, _actor: &OwnedActor, _admit_damage: DamageAdmissionFn) {}

        fn apply(&self, request: DamageRequest) -> DamageOutcome {
            DamageOutcome::StaleTarget { request }
        }
    }

    struct FakeInventory {
        entries: RefCell<HashMap<(ActorId, ItemId), (i32, i32)>>,
    }

    impl FakeInventory {
        fn new() -> Self {
            Self {
                entries: RefCell::new(HashMap::new()),
            }
        }
    }

    impl Q3SessionInventory for FakeInventory {
        fn has(&self, _actor: &ActorId) -> bool {
            true
        }

        fn create(&self, _actor: &OwnedActor, entries: Vec<InventoryEntry>) {
            for entry in entries {
                self.entries
                    .borrow_mut()
                    .insert((_actor.id().clone(), entry.item), (entry.count, entry.capacity));
            }
        }

        fn count(&self, actor: &ActorId, item: &ItemId) -> i32 {
            self.entries
                .borrow()
                .get(&(actor.clone(), item.clone()))
                .map_or(0, |(count, _)| *count)
        }

        fn configure(&self, actor: &OwnedActor, item: &ItemId, count: i32, capacity: i32) {
            self.entries
                .borrow_mut()
                .insert((actor.id().clone(), item.clone()), (count, capacity));
        }
    }

    struct FakeCallbacks {
        bound: RefCell<HashMap<ActorId, ActorCallbacks>>,
    }

    impl FakeCallbacks {
        fn new() -> Self {
            Self {
                bound: RefCell::new(HashMap::new()),
            }
        }
    }

    impl Q3ActorCallbacks for FakeCallbacks {
        fn bind(&self, actor: &OwnedActor, callbacks: ActorCallbacks) {
            self.bound.borrow_mut().insert(actor.id().clone(), callbacks);
        }
    }

    struct FakeRecordHost {
        actors: Rc<FakeActors>,
        bodies: Rc<FakeBodies>,
        combat: Rc<FakeCombat>,
        inventory: Rc<FakeInventory>,
        callbacks: Rc<FakeCallbacks>,
        scheduled: RefCell<Vec<(ActorId, Option<i32>)>>,
        foreign: RefCell<HashMap<ActorId, EntityRef>>,
        players: RefCell<Vec<ActorId>>,
        call: RefCell<Option<Q3DamageCall>>,
    }

    impl FakeRecordHost {
        fn new() -> Self {
            Self {
                actors: Rc::new(FakeActors::new()),
                bodies: Rc::new(FakeBodies::new()),
                combat: Rc::new(FakeCombat::new()),
                inventory: Rc::new(FakeInventory::new()),
                callbacks: Rc::new(FakeCallbacks::new()),
                scheduled: RefCell::new(Vec::new()),
                foreign: RefCell::new(HashMap::new()),
                players: RefCell::new(Vec::new()),
                call: RefCell::new(None),
            }
        }
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

        fn schedule(&self, actor: &OwnedActor, due_milliseconds: Option<i32>) {
            self.scheduled.borrow_mut().push((actor.id().clone(), due_milliseconds));
        }

        fn run_think(&self, _actor: &OwnedActor, _time_milliseconds: i32) {}

        fn damage_call(&self) -> Option<Q3DamageCall> {
            self.call.borrow().clone()
        }

        fn foreign(&self, actor: &ActorId) -> Option<EntityRef> {
            self.foreign.borrow().get(actor).cloned()
        }

        fn is_player(&self, actor: &ActorId) -> bool {
            self.players.borrow().contains(actor)
        }
    }

    fn test_records() -> (Rc<FakeRecordHost>, Q3EntityRecords) {
        let host = Rc::new(FakeRecordHost::new());
        let records = Q3EntityRecords::new(host.clone(), test_provider(), Product::Baseq3);
        (host, records)
    }

    // -- records ------------------------------------------------------------

    #[test]
    fn records_activate_and_mirror_stats() {
        let (host, records) = test_records();
        let entity = records.activate(3);
        assert!(entity.borrow().inuse());
        assert_eq!(entity.borrow().slot, 3);
        assert!(host.callbacks.bound.borrow().len() == 1);
        let client = records.client(3);
        client.borrow_mut().ps.stats.set(0, 120);
        let actor = entity.borrow().actor();
        assert_eq!(host.combat.read(actor.id()).unwrap().health, 120);
        client.borrow_mut().ps.stats.set(3, 45);
        assert!(matches!(
            host.combat.read(actor.id()).unwrap().armor.regular,
            RegularArmorState::Q3 { points: 45, .. }
        ));
        client
            .borrow_mut()
            .ps
            .stats
            .set(2, (1 << (Weapon::WpShotgun as i32)) | (1 << (Weapon::WpBfg as i32)));
        let shotgun = q3_weapon_item(Weapon::WpShotgun as i32).unwrap();
        let bfg = q3_weapon_item(Weapon::WpBfg as i32).unwrap();
        let machinegun = q3_weapon_item(Weapon::WpMachinegun as i32).unwrap();
        assert_eq!(host.inventory.count(actor.id(), &shotgun.item), 1);
        assert_eq!(host.inventory.count(actor.id(), &bfg.item), 1);
        assert_eq!(host.inventory.count(actor.id(), &machinegun.item), 0);
        client.borrow_mut().ps.ammo.set(Weapon::WpShotgun as usize, 12);
        assert_eq!(host.inventory.count(actor.id(), shotgun.ammo.as_ref().unwrap()), 12);
    }

    #[test]
    fn records_attach_adopt_and_release() {
        let (host, records) = test_records();
        let owned = host.actors.allocate_at_source(&test_provider(), 500, "q3:entity");
        records.host().bodies().create(&owned, ZERO_BODY.clone());
        let adopted = records.adopt(500, owned.clone()).expect("adopt");
        assert_eq!(adopted.borrow().s.number, 500);
        assert!(adopted.borrow().inuse());
        assert!(records.adopt(501, owned).is_err());

        let foreign_owned = host
            .actors
            .allocate_at_source(&ProviderId::new("other", "game"), 501, "q3:entity");
        let attached = records.attach(9, foreign_owned, true).expect("attach");
        assert!(attached.borrow().client.is_some());
        assert_eq!(attached.borrow().s.number, 9);

        records.release(adopted.clone());
        assert_eq!(adopted.borrow().s.number, 0);
        assert!(!adopted.borrow().inuse());
    }

    #[test]
    fn records_ownership_round_trips_into_fresh_records() {
        let (_host, records) = test_records();
        let _ = records.activate(1);
        let _ = records.activate(70);
        let ownership = records.capture_ownership();
        assert_eq!(ownership.len(), MAX_GENTITIES);
        assert!(ownership[1].active);
        assert!(ownership[70].active);
        assert!(!ownership[2].active);

        let (host2, fresh) = test_records();
        let _ = host2;
        // Ownership words reference foreign actors, so hydration fails
        // instead of aliasing another session's actors.
        assert!(fresh.restore_ownership(&ownership).is_err());
        assert!(fresh.restore_ownership(&ownership[..10]).is_err());

        let backing = records.capture_client_backing(1);
        assert_eq!(backing.source_stats, [0; 16]);
        fresh.restore_client_backing(1, &backing).expect("backing");
        assert!(fresh.restore_client_backing(99, &backing).is_err());
    }

    #[test]
    fn records_resolve_natives_foreigners_and_inflictors() {
        let (host, records) = test_records();
        let entity = records.activate(4);
        let actor = entity.borrow().actor().id().clone();
        assert!(records.native_by_actor(None).is_none());
        assert!(Rc::ptr_eq(&records.native_by_actor(Some(&actor)).unwrap(), &entity));
        assert!(records.by_actor(Some(&actor)).is_some());
        let owner = test_owner();
        let ghost = owner.actor(900, 1);
        assert!(records.by_actor(Some(&ghost)).is_none());
        host.foreign.borrow_mut().insert(ghost.clone(), entity.clone());
        assert!(records.by_actor(Some(&ghost)).is_some());
        let world = records.damage_inflictor(None);
        assert!(matches!(world, DamageParticipant::Native(_)));
        let foreign = records.damage_inflictor(Some(&ghost));
        assert!(matches!(foreign, DamageParticipant::SharedActor(_)));
        assert!(records.use_participant(None).is_none());
    }

    #[test]
    fn records_dispatch_bound_callbacks() {
        let (host, records) = test_records();
        let entity = records.activate(11);
        let actor = entity.borrow().actor().id().clone();
        let fired = Rc::new(RefCell::new(Vec::new()));
        let think_fired = fired.clone();
        entity.borrow_mut().think = Some(Rc::new(move |_| {
            think_fired.borrow_mut().push("think");
        }));
        let pain_fired = fired.clone();
        entity.borrow_mut().pain = Some(Rc::new(move |_, _, damage| {
            pain_fired.borrow_mut().push(if damage == 7 { "pain" } else { "bad" });
        }));
        let die_fired = fired.clone();
        entity.borrow_mut().die = Some(Rc::new(move |_, _, _, _, method| {
            die_fired.borrow_mut().push(if method == 9 { "die" } else { "bad" });
        }));
        let bound = host.callbacks.bound.borrow().get(&actor).expect("bound").clone();
        (bound.think)();
        (bound.pain)(&PainReaction {
            attack: None,
            attacker: None,
            damage: 7,
        });
        host.call.borrow_mut().replace(Q3DamageCall {
            target: entity.clone(),
            source: DamageParticipant::Native(entity.clone()),
            owner: DamageParticipant::Native(entity.clone()),
            direction: None,
            point: None,
            amount: 7.0,
            flags: 0,
            method_of_death: 9,
        });
        (bound.die)(&DeathReaction {
            attack: None,
            attacker: None,
            damage: 7,
            inflictor: None,
            point: vec3(0.0, 0.0, 0.0),
        });
        assert_eq!(*fired.borrow(), vec!["think", "pain", "die"]);
        assert_eq!(entity.borrow().nextthink(), 0);
        records.close();
    }

    // -- combat bridge --------------------------------------------------------

    struct FakePool {
        records: Q3EntityRecords,
        num: usize,
    }

    impl Q3EntityPool for FakePool {
        fn num_entities(&self) -> usize {
            self.num
        }

        fn entity_at(&self, index: usize) -> EntityRef {
            self.records.get(index).expect("pool entity")
        }
    }

    struct FakeBridgeHost {
        records_host: Rc<FakeRecordHost>,
        records: Q3EntityRecords,
        pool: EntityPoolRef,
        world: Rc<dyn Q3ServerWorld>,
        time: Cell<i32>,
        intermission: Cell<i32>,
        game_type: Cell<i32>,
        feedback: RefCell<Vec<String>>,
    }

    impl FakeBridgeHost {
        fn new(records_host: Rc<FakeRecordHost>, records: Q3EntityRecords) -> Rc<Self> {
            let world_host = Rc::new(FakeWorldHost::new());
            let world: Rc<dyn Q3ServerWorld> = Rc::new(Q3WorldAdapter::new(world_host, records.clone()));
            Rc::new(Self {
                records_host,
                pool: Rc::new(FakePool {
                    records: records.clone(),
                    num: MAX_CLIENTS,
                }),
                world,
                records,
                time: Cell::new(1000),
                intermission: Cell::new(0),
                game_type: Cell::new(0),
                feedback: RefCell::new(Vec::new()),
            })
        }
    }

    impl Q3CombatBridgeHost for FakeBridgeHost {
        fn authority(&self) -> Rc<dyn Q3SessionCombat> {
            self.records_host.combat.clone()
        }

        fn entities(&self) -> EntityPoolRef {
            self.pool.clone()
        }

        fn records(&self) -> Q3EntityRecords {
            self.records.clone()
        }

        fn world(&self) -> Rc<dyn Q3ServerWorld> {
            self.world.clone()
        }

        fn weapon_provider(&self) -> ProviderId {
            ProviderId::new("q3", "weapon")
        }

        fn combat_provider(&self) -> ProviderId {
            ProviderId::new("q3", "combat")
        }

        fn inventory_provider(&self) -> ProviderId {
            ProviderId::new("q3", "inventory")
        }

        fn movement_provider(&self) -> ProviderId {
            ProviderId::new("q3", "movement")
        }

        fn armor_context(&self, _request: &DamageRequest) -> VictimArmorContext {
            VictimArmorContext {
                screen_facing_dot: 0.0,
                arithmetic: VictimArithmetic::Binary32,
                q2: None,
            }
        }

        fn time(&self) -> i32 {
            self.time.get()
        }

        fn intermission_queued(&self) -> i32 {
            self.intermission.get()
        }

        fn game_type(&self) -> i32 {
            self.game_type.get()
        }

        fn friendly_fire(&self) -> bool {
            false
        }

        fn knockback(&self) -> f32 {
            1000.0
        }

        fn product(&self) -> Product {
            Product::Baseq3
        }

        fn check_hurt_carrier(&self, _target: EntityRef, _attacker: EntityRef) {}

        fn log_accuracy_hit(&self, _target: EntityRef, _attacker: EntityRef) -> bool {
            false
        }

        fn damage_feedback(&self, _call: &Q3DamageCall, _decision: &DamageDecision) {
            self.feedback.borrow_mut().push("damage".to_string());
        }

        fn foreign_damage_feedback(&self, _target: EntityRef, _owner: Option<EntityRef>, _decision: &DamageDecision) {
            self.feedback.borrow_mut().push("foreign".to_string());
        }
    }

    fn test_attack(target: &ActorId, attacker: Option<&ActorId>) -> AttackProvenance {
        let _ = target;
        AttackProvenance {
            sequence: 0,
            time: SourceTime::Milliseconds(1000),
            attacker: attacker.cloned(),
            inflictor: attacker.cloned(),
            originating_projectile: None,
            weapon: None,
            weapon_provider: ProviderId::new("q3", "weapon"),
            damage_powerup_owner: None,
            combat_provider: ProviderId::new("q3", "combat"),
            inventory_provider: ProviderId::new("q3", "inventory"),
            movement_provider: ProviderId::new("q3", "movement"),
            cause: AttackCause::Q3 {
                means_of_death: 7,
                damage_flags: 0,
            },
        }
    }

    fn test_request(target: ActorId, attacker: Option<ActorId>, amount: f32) -> DamageRequest {
        DamageRequest {
            attack: test_attack(&target, attacker.as_ref()),
            target,
            amount,
            knockback: amount,
            direction: vec3(1.0, 0.0, 0.0),
            point: vec3(0.0, 0.0, 0.0),
            normal: vec3(0.0, 0.0, 0.0),
            delivery: Delivery::Direct,
        }
    }

    fn test_combat_state(health: i32) -> CombatState {
        CombatState {
            health,
            armor: ArmorState {
                regular: RegularArmorState::Q3 {
                    points: 0,
                    protection: 0.66,
                },
                powered: PoweredProtectionState::None,
            },
            mass: 200,
            can_take_damage: true,
            invulnerable: false,
            no_knockback: false,
            team: None,
        }
    }

    struct FixedCurrent {
        target: Option<CombatState>,
        attacker: Option<CombatState>,
    }

    impl CurrentCombatState for FixedCurrent {
        fn target(&self) -> Option<CombatState> {
            self.target.clone()
        }

        fn attacker(&self) -> Option<CombatState> {
            self.attacker.clone()
        }
    }

    fn drive_progress(
        progress: CombatProgress,
        target: &CombatState,
        attacker: Option<&CombatState>,
    ) -> (CombatResult, Vec<DamageMutation>) {
        let mut progress = progress;
        let mut armor = target.armor.clone();
        let mut seen = Vec::new();
        loop {
            match progress {
                CombatProgress::Complete {
                    result, mut mutations, ..
                } => {
                    seen.append(&mut mutations);
                    return (result, seen);
                }
                CombatProgress::SourceContinuation {
                    mut mutations, resume, ..
                } => {
                    seen.append(&mut mutations);
                    let current = FixedCurrent {
                        target: Some(target.clone()),
                        attacker: attacker.cloned(),
                    };
                    progress = resume(&current);
                }
                CombatProgress::ArmorStage {
                    channel,
                    mut mutations,
                    resume,
                    fallback,
                    ..
                } => {
                    seen.append(&mut mutations);
                    let result = fallback(&armor);
                    armor = result.armor.clone();
                    let saved = match channel {
                        ProtectionChannel::Powered => result.power_saved,
                        ProtectionChannel::Regular => result.regular_saved,
                    };
                    let current = FixedCurrent {
                        target: Some(target.clone()),
                        attacker: attacker.cloned(),
                    };
                    progress = resume(ArmorStageResult { saved }, &current);
                }
            }
        }
    }

    #[test]
    fn bridge_captures_provenance_and_routes_feedback() {
        let (records_host, records) = test_records();
        let target = records.activate(5);
        target.borrow_mut().s.weapon = Weapon::WpShotgun as i32;
        let attacker = records.activate(6);
        let host = FakeBridgeHost::new(records_host, records.clone());
        let bridge = Q3CombatBridge::new(host.clone());

        let provenance = (bridge.context().attack)(
            &DamageParticipant::Native(target.clone()),
            &DamageParticipant::Native(attacker.clone()),
            None,
            7,
            0,
            None,
        );
        assert_eq!(provenance.sequence, 0);
        assert_eq!(provenance.weapon, Some("q3:weapon/shotgun".to_string()));
        let second = (bridge.context().attack)(
            &DamageParticipant::Native(target.clone()),
            &DamageParticipant::Native(attacker.clone()),
            Some("q3:weapon/bfg".to_string()),
            7,
            0,
            None,
        );
        assert_eq!(second.sequence, 1);
        assert_eq!(second.weapon, Some("q3:weapon/bfg".to_string()));

        let saved = bridge.capture_save_state();
        let bridge2 = Q3CombatBridge::new(host.clone());
        bridge2.restore_save_state(&saved).expect("restore");
        let third = (bridge2.context().attack)(
            &DamageParticipant::Native(target.clone()),
            &DamageParticipant::Native(attacker.clone()),
            None,
            7,
            0,
            None,
        );
        assert_eq!(third.sequence, 2);

        let target_id = target.borrow().actor().id().clone();
        let attacker_id = attacker.borrow().actor().id().clone();
        let decision = DamageDecision {
            request: test_request(target_id.clone(), Some(attacker_id), 40.0),
            mutations: Vec::new(),
            applied_damage: 40,
            reaction: Reaction::Pain,
            feedback: None,
        };
        let outcome = (bridge.context().dispatch)(
            Q3DamageCall {
                target: target.clone(),
                source: DamageParticipant::Native(attacker.clone()),
                owner: DamageParticipant::Native(attacker.clone()),
                direction: None,
                point: None,
                amount: 40.0,
                flags: 0,
                method_of_death: 7,
            },
            &|| DamageOutcome::StaleTarget {
                request: test_request(target_id.clone(), None, 0.0),
            },
        );
        assert!(matches!(outcome, DamageOutcome::StaleTarget { .. }));
        assert!(bridge.current_call().is_none());
        bridge.before_reaction(&DamageDecision {
            request: test_request(target_id, None, 10.0),
            ..decision.clone()
        });
        assert_eq!(*host.feedback.borrow(), vec!["foreign".to_string()]);
    }

    #[test]
    fn bridge_policy_decides_q3_damage_flow() {
        let (records_host, records) = test_records();
        let target = records.activate(5);
        target.borrow_mut().client = Some(records.client(5));
        let attacker = records.activate(6);
        attacker.borrow_mut().client = Some(records.client(6));
        attacker
            .borrow()
            .client
            .as_ref()
            .unwrap()
            .borrow_mut()
            .ps
            .stats
            .set(BaseStatIndex::StatMaxHealth as usize, 100);
        let host = FakeBridgeHost::new(records_host, records);
        let bridge = Q3CombatBridge::new(host.clone());
        let policy = bridge.policy();

        let target_id = target.borrow().actor().id().clone();
        let attacker_id = attacker.borrow().actor().id().clone();
        let request = test_request(target_id, Some(attacker_id), 50.0);
        let state = test_combat_state(100);
        let progress = (policy.decide)(&request, &state, Some(&state));
        let (result, mutations) = drive_progress(progress, &state, Some(&state));
        assert_eq!(result.applied_damage, 50);
        assert_eq!(result.reaction, Reaction::Pain);
        assert!(mutations
            .iter()
            .any(|mutation| matches!(mutation, DamageMutation::Health { before: 100, after: 50 })));
        assert!(mutations
            .iter()
            .any(|mutation| matches!(mutation, DamageMutation::Impulse { .. })));

        host.intermission.set(1);
        let held = (policy.decide)(&request, &state, Some(&state));
        let (held_result, _) = drive_progress(held, &state, Some(&state));
        assert_eq!(held_result.applied_damage, 0);
        assert_eq!(held_result.reaction, Reaction::None);
    }

    #[test]
    fn bridge_policy_blocks_godmode_targets() {
        let (records_host, records) = test_records();
        let target = records.activate(5);
        target.borrow_mut().flags |= GameFlags::GODMODE;
        let host = FakeBridgeHost::new(records_host, records);
        let bridge = Q3CombatBridge::new(host);
        let policy = bridge.policy();
        let target_id = target.borrow().actor().id().clone();
        let request = test_request(target_id, None, 50.0);
        let state = test_combat_state(100);
        let (result, _) = drive_progress((policy.decide)(&request, &state, None), &state, None);
        assert_eq!(result.applied_damage, 0);
    }

    #[test]
    fn native_armor_absorbs_q3_points() {
        let armor = ArmorState {
            regular: RegularArmorState::Q3 {
                points: 50,
                protection: 0.66,
            },
            powered: PoweredProtectionState::None,
        };
        let flags = ArmorDamageFlags {
            stage: None,
            no_armor: false,
            no_power_armor: false,
            no_regular_armor: false,
            energy: false,
            regular_protection_scale: Some(1.0),
        };
        let context = VictimArmorContext {
            screen_facing_dot: 0.0,
            arithmetic: VictimArithmetic::Binary32,
            q2: None,
        };
        let result = absorb_native_armor(&armor, 100, &flags, &context);
        assert_eq!(result.regular_saved, 50);
        assert_eq!(result.power_saved, 0);
    }

    // -- world adapter --------------------------------------------------------

    struct FakeWorldHost {
        bodies: FakeBodies,
        trace_result: RefCell<Q3TraceResult>,
        actors: RefCell<Vec<ActorId>>,
        collisions: RefCell<HashMap<ActorId, ActorCollision>>,
    }

    impl FakeWorldHost {
        fn new() -> Self {
            Self {
                bodies: FakeBodies::new(),
                trace_result: RefCell::new(Q3TraceResult {
                    fraction: 1.0,
                    end: vec3(0.0, 0.0, 0.0),
                    hit: Q3TraceHit::None,
                    contact: TraceContact::None,
                    start_solid: false,
                    all_solid: false,
                    contents: 0,
                    surface_flags: 0,
                }),
                actors: RefCell::new(Vec::new()),
                collisions: RefCell::new(HashMap::new()),
            }
        }
    }

    impl Q3WorldAdapterHost for FakeWorldHost {
        fn trace_scene(&self, _query: &Q3TraceQuery) -> Q3TraceResult {
            self.trace_result.borrow().clone()
        }

        fn point_contents_scene(&self, query: &Q3TraceQuery, _point: Vec3) -> i32 {
            assert_eq!(query.mask, -1);
            3
        }

        fn query_actors(&self, _bounds: Bounds) -> Vec<ActorId> {
            self.actors.borrow().clone()
        }

        fn spatial_collision(&self, actor: &ActorId) -> Option<ActorCollision> {
            self.collisions.borrow().get(actor).cloned()
        }

        fn body_state(&self, actor: &ActorId) -> Option<BodyState> {
            self.bodies.read(actor)
        }

        fn linked_body(&self, actor: &ActorId) -> Option<LinkedBody> {
            self.bodies.linked(actor)
        }

        fn set_collision(&self, actor: &OwnedActor, collision: ActorCollision) {
            self.collisions.borrow_mut().insert(actor.id().clone(), collision);
        }

        fn link_body(&self, actor: &OwnedActor, origin: Option<Vec3>) {
            self.bodies.link(actor, origin);
        }

        fn unlink_body(&self, actor: &OwnedActor) {
            self.bodies.unlink(actor);
        }

        fn curves(&self) -> bool {
            true
        }

        fn player_curve_clip(&self) -> bool {
            false
        }

        fn geometry_trace_start_solid(&self, _query: &Q3TraceQuery, _model: i32, _origin: Vec3, _angles: Vec3) -> bool {
            true
        }

        fn body_trace_start_solid(
            &self,
            _query: &Q3TraceQuery,
            _body: &BodyState,
            _collision: &ActorCollision,
        ) -> bool {
            false
        }
    }

    #[test]
    fn adapter_maps_hits_and_encodes_solid() {
        let (_records_host, records) = test_records();
        let entity = records.activate(20);
        let actor = entity.borrow().actor().id().clone();
        let host = Rc::new(FakeWorldHost::new());
        host.trace_result.borrow_mut().hit = Q3TraceHit::Actor { actor: actor.clone() };
        host.trace_result.borrow_mut().fraction = 0.5;
        let adapter = Q3WorldAdapter::new(host.clone(), records.clone());

        let result = adapter.trace(&ServerTraceQuery {
            start: vec3(0.0, 0.0, 0.0),
            end: vec3(0.0, 0.0, 10.0),
            shape: TraceShape::Point,
            pass_entity_num: ENTITYNUM_NONE,
            mask: 1,
        });
        assert_eq!(result.entity_num, 20);
        assert_eq!(result.fraction, 0.5);

        host.trace_result.borrow_mut().hit = Q3TraceHit::None;
        let clear = adapter.trace(&ServerTraceQuery {
            start: vec3(0.0, 0.0, 0.0),
            end: vec3(0.0, 0.0, 10.0),
            shape: TraceShape::Point,
            pass_entity_num: ENTITYNUM_NONE,
            mask: 1,
        });
        assert_eq!(clear.entity_num, ENTITYNUM_NONE);

        host.actors.borrow_mut().push(actor.clone());
        assert_eq!(
            adapter.area_entities(
                Bounds {
                    min: vec3(0.0, 0.0, 0.0),
                    max: vec3(1.0, 1.0, 1.0),
                },
                1024
            ),
            vec![20]
        );
        assert_eq!(adapter.point_contents(vec3(0.0, 0.0, 0.0), ENTITYNUM_NONE), 3);

        entity.borrow_mut().r.contents = 1;
        entity.borrow_mut().r.set_mins(vec3(-15.0, -15.0, -24.0));
        entity.borrow_mut().r.set_maxs(vec3(15.0, 15.0, 32.0));
        adapter.link(entity.clone());
        assert_eq!(entity.borrow().s.solid, (64 << 16) | (24 << 8) | 15);
        let stored = host.collisions.borrow().get(&actor).expect("collision").clone();
        assert_eq!(stored.role, ActorCollisionRole::Solid);
        assert!(adapter.link_state(20).is_some());
        assert!(adapter.link_state(21).is_none());
        let restore = adapter.unlink_actor(&actor).expect("restore");
        assert!(adapter.link_state(20).is_none());
        restore();
        assert!(adapter.link_state(20).is_some());
    }

    #[test]
    fn adapter_contact_uses_model_or_body_paths() {
        let (_records_host, records) = test_records();
        let entity = records.activate(30);
        let actor = entity.borrow().actor().id().clone();
        let host = Rc::new(FakeWorldHost::new());
        let adapter = Q3WorldAdapter::new(host.clone(), records.clone());
        let bounds = Bounds {
            min: vec3(0.0, 0.0, 0.0),
            max: vec3(1.0, 1.0, 1.0),
        };
        assert!(!adapter.contact_actor(bounds, &actor, false));
        let owned = entity.borrow().actor();
        host.bodies.create(&owned, ZERO_BODY.clone());
        host.collisions.borrow_mut().insert(
            actor.clone(),
            ActorCollision {
                shape: ActorCollisionShape::InlineModel { model: 2 },
                contents: 1,
                owner: None,
                role: ActorCollisionRole::Solid,
                monster: false,
                dead_monster: false,
            },
        );
        assert!(adapter.contact_actor(bounds, &actor, false));
        assert!(adapter.entity_contact(bounds, 30, false));
        assert!(!adapter.entity_contact(bounds, 31, false));
    }

    // -- map spawns -----------------------------------------------------------

    struct FakeSpawnHost {
        product: Product,
        obelisks: RefCell<Vec<String>>,
    }

    impl Q3SpawnHandlersHost for FakeSpawnHost {
        fn product(&self) -> Product {
            self.product
        }

        fn misc_handlers(&self) -> HashMap<String, SpawnHandler> {
            let mut handlers: HashMap<String, SpawnHandler> = HashMap::new();
            handlers.insert(
                "info_null".to_string(),
                Rc::new(|entity: EntityRef, _: &SpawnVariables| {
                    entity.borrow_mut().flags |= 1;
                }),
            );
            handlers
        }

        fn mover_handlers(&self) -> HashMap<String, SpawnHandler> {
            HashMap::new()
        }

        fn trigger_handlers(&self) -> HashMap<String, SpawnHandler> {
            HashMap::new()
        }

        fn target_handlers(&self) -> HashMap<String, SpawnHandler> {
            HashMap::new()
        }

        fn spawn_player_start(&self) -> SpawnHandler {
            Rc::new(|_, _| {})
        }

        fn spawn_deathmatch_point(&self) -> SpawnHandler {
            Rc::new(|_, _| {})
        }

        fn spawn_team_point(&self) -> SpawnHandler {
            Rc::new(|_, _| {})
        }

        fn spawn_team_obelisk(&self, _entity: EntityRef, team: Team) {
            self.obelisks.borrow_mut().push(format!("team:{}", team as i32));
        }

        fn spawn_neutral_obelisk(&self, _entity: EntityRef) {
            self.obelisks.borrow_mut().push("neutral".to_string());
        }
    }

    #[test]
    fn spawn_handlers_cover_routes_and_obelisks() {
        let host = Rc::new(FakeSpawnHost {
            product: Product::Missionpack,
            obelisks: RefCell::new(Vec::new()),
        });
        let handlers = create_q3_spawn_handlers(host.clone()).expect("handlers");
        assert!(handlers.contains_key("info_player_start"));
        assert!(handlers.contains_key("info_player_intermission"));
        assert!(handlers.contains_key("item_botroam"));
        assert!(handlers.contains_key("func_group"));
        assert!(handlers.contains_key("team_redobelisk"));
        let (_records_host, records) = test_records();
        let entity = records.activate(40);
        handlers["func_group"](entity.clone(), &SpawnVariables::default());
        assert_eq!(entity.borrow().flags, 1);
        handlers["team_redobelisk"](entity.clone(), &SpawnVariables::default());
        handlers["team_blueobelisk"](entity.clone(), &SpawnVariables::default());
        handlers["team_neutralobelisk"](entity, &SpawnVariables::default());
        assert_eq!(
            *host.obelisks.borrow(),
            vec![
                format!("team:{}", Team::TeamRed as i32),
                format!("team:{}", Team::TeamBlue as i32),
                "neutral".to_string()
            ]
        );

        let base = Rc::new(FakeSpawnHost {
            product: Product::Baseq3,
            obelisks: RefCell::new(Vec::new()),
        });
        let base_handlers = create_q3_spawn_handlers(base).expect("base");
        assert!(!base_handlers.contains_key("team_redobelisk"));
    }

    struct EmptySpawnHost;

    impl Q3SpawnHandlersHost for EmptySpawnHost {
        fn product(&self) -> Product {
            Product::Baseq3
        }

        fn misc_handlers(&self) -> HashMap<String, SpawnHandler> {
            HashMap::new()
        }

        fn mover_handlers(&self) -> HashMap<String, SpawnHandler> {
            HashMap::new()
        }

        fn trigger_handlers(&self) -> HashMap<String, SpawnHandler> {
            HashMap::new()
        }

        fn target_handlers(&self) -> HashMap<String, SpawnHandler> {
            HashMap::new()
        }

        fn spawn_player_start(&self) -> SpawnHandler {
            Rc::new(|_, _| {})
        }

        fn spawn_deathmatch_point(&self) -> SpawnHandler {
            Rc::new(|_, _| {})
        }

        fn spawn_team_point(&self) -> SpawnHandler {
            Rc::new(|_, _| {})
        }

        fn spawn_team_obelisk(&self, _entity: EntityRef, _team: Team) {}

        fn spawn_neutral_obelisk(&self, _entity: EntityRef) {}
    }

    #[test]
    fn spawn_handlers_require_info_null() {
        let host = Rc::new(EmptySpawnHost);
        assert!(create_q3_spawn_handlers(host).is_err());
    }

    struct DirectBinding {
        active: bool,
        actor: OwnedActor,
        body: Rc<FakeDirectBody>,
    }

    struct FakeDirectBody {
        state: RefCell<BodyState>,
    }

    impl EntityBodyBinding for FakeDirectBody {
        fn read(&self) -> BodyState {
            self.state.borrow().clone()
        }

        fn write(&self, value: BodyState) {
            *self.state.borrow_mut() = value;
        }

        fn linked(&self) -> Option<LinkedBody> {
            None
        }
    }

    impl GameEntityBinding for DirectBinding {
        fn body(&self) -> Rc<dyn EntityBodyBinding> {
            self.body.clone()
        }

        fn actor(&self) -> OwnedActor {
            self.actor.clone()
        }

        fn active(&self) -> bool {
            self.active
        }

        fn health(&self) -> i32 {
            0
        }

        fn set_health(&self, _value: i32) {}

        fn takes_damage(&self) -> bool {
            false
        }

        fn set_takes_damage(&self, _value: bool) {}

        fn schedule(&self, _nextthink: i32) {}

        fn run_think(&self, _time_milliseconds: i32) {}
    }

    struct VecPool {
        entities: Vec<EntityRef>,
    }

    impl Q3EntityPool for VecPool {
        fn num_entities(&self) -> usize {
            self.entities.len()
        }

        fn entity_at(&self, index: usize) -> EntityRef {
            self.entities[index].clone()
        }
    }

    fn team_entity(owner: &IdentityOwner, slot: usize, team: Option<&str>) -> EntityRef {
        let actor = owner
            .owned_actor(&owner.actor(slot as u32, 1), ProviderId::new("q3", "test"))
            .expect("owned");
        let binding = Rc::new(DirectBinding {
            active: true,
            actor,
            body: Rc::new(FakeDirectBody {
                state: RefCell::new(ZERO_BODY.clone()),
            }),
        });
        let entity = Rc::new(RefCell::new(GameEntity::new(slot, binding)));
        entity.borrow_mut().team = team.map(str::to_string);
        entity
    }

    #[test]
    fn entity_teams_chain_and_transfer_targetnames() {
        let owner = test_owner();
        let first = team_entity(&owner, 1, Some("alpha"));
        let second = team_entity(&owner, 2, Some("alpha"));
        second.borrow_mut().targetname = Some("slave-target".to_string());
        let third = team_entity(&owner, 3, Some("beta"));
        let pool = VecPool {
            entities: vec![
                team_entity(&owner, 0, None),
                first.clone(),
                second.clone(),
                third.clone(),
            ],
        };
        let counts = find_q3_entity_teams(&pool);
        assert_eq!(counts, EntityTeamCounts { teams: 2, entities: 3 });
        assert_eq!(first.borrow().targetname, Some("slave-target".to_string()));
        assert_eq!(second.borrow().targetname, None);
        assert_ne!(second.borrow().flags & GameFlags::TEAMSLAVE, 0);
        assert!(Rc::ptr_eq(first.borrow().teamchain.as_ref().unwrap(), &second));
        assert!(third.borrow().teamchain.is_none());
    }

    // -- settings ---------------------------------------------------------------

    struct FakeSettingsHost {
        cvars: Rc<RefCell<CvarRegistry>>,
        commands: RefCell<Vec<(i32, String)>>,
        remapped: Cell<bool>,
    }

    impl FakeSettingsHost {
        fn new() -> Self {
            Self {
                cvars: Rc::new(RefCell::new(CvarRegistry::new(Dialect::Q3))),
                commands: RefCell::new(Vec::new()),
                remapped: Cell::new(false),
            }
        }
    }

    impl Q3SettingsHost for FakeSettingsHost {
        fn cvars(&self) -> Rc<RefCell<CvarRegistry>> {
            self.cvars.clone()
        }

        fn send_server_command(&self, client: i32, command: String) {
            self.commands.borrow_mut().push((client, command));
        }

        fn remap_teams(&self) {
            self.remapped.set(true);
        }

        fn format_tracked_change(&self, name: &str, value: &str) -> String {
            format!("print \"Server: {name} changed to {value}\n\"")
        }
    }

    #[test]
    fn settings_register_update_and_save_round_trip() {
        let host = Rc::new(FakeSettingsHost::new());
        let settings = Q3GameSettings::new(host.clone(), Product::Baseq3);
        assert_eq!(settings.definitions().len(), 45);
        let pack = Q3GameSettings::new(host.clone(), Product::Missionpack);
        assert_eq!(pack.definitions().len(), 56);
        settings.register("2026-09-30");
        assert_eq!(settings.integer("fraglimit"), 20);
        assert_eq!(settings.number("g_speed"), 320.0);
        assert_eq!(settings.string("g_motd"), "");
        host.cvars.borrow_mut().set("fraglimit", "30", true).expect("set");
        settings.update();
        assert_eq!(settings.integer("fraglimit"), 30);
        let commands = host.commands.borrow();
        assert_eq!(commands.len(), 1);
        assert_eq!(commands[0].0, -1);
        assert!(commands[0].1.contains("fraglimit"), "{}", commands[0].1);
        assert!(commands[0].1.contains("30"), "{}", commands[0].1);
        drop(commands);

        let saved = settings.capture_save_state();
        let host2 = Rc::new(FakeSettingsHost::new());
        let restored = Q3GameSettings::new(host2.clone(), Product::Baseq3);
        restored.register("2026-09-30");
        restored.restore_save_state(&saved).expect("restore");
        assert_eq!(restored.integer("fraglimit"), 30);
        assert!(restored.restore_save_state(&arr(Vec::new())).is_err());
    }

    #[test]
    fn settings_remap_team_shaders_on_change() {
        let host = Rc::new(FakeSettingsHost::new());
        let settings = Q3GameSettings::new(host.clone(), Product::Missionpack);
        settings.register("2026-09-30");
        host.cvars.borrow_mut().set("g_redteam", "Rangers", true).expect("set");
        settings.update();
        assert!(host.remapped.get());
    }

    // -- product restriction ----------------------------------------------------

    fn valid_product_id() -> Vec<u8> {
        let mut seed: i32 = 5000;
        SCRAMBLED_PRODUCT_ID
            .iter()
            .map(|scrambled| {
                let byte = scrambled ^ ((seed & 255) as u8);
                seed = seed.wrapping_mul(69069).wrapping_add(1);
                byte
            })
            .collect()
    }

    #[test]
    fn mount_restriction_matches_fs_setrestrictions() {
        assert_eq!(
            q3_mount_restriction(Q3ProductPolicy::Retail, false),
            Q3MountRestriction::None
        );
        assert_eq!(
            q3_mount_restriction(Q3ProductPolicy::Retail, true),
            Q3MountRestriction::Demo {
                directory: "demota",
                pak_checksum: 437558517
            }
        );
        assert!(matches!(
            q3_mount_restriction(
                Q3ProductPolicy::PrereleaseDemo {
                    team_arena_ui: TeamArenaUi::Retail
                },
                false
            ),
            Q3MountRestriction::Demo { .. }
        ));
        let valid = valid_product_id();
        assert_eq!(
            resolve_q3_mount_restriction(Q3ProductPolicy::Retail, false, Some(&valid)).expect("valid"),
            Q3MountRestriction::None
        );
        assert!(matches!(
            resolve_q3_mount_restriction(Q3ProductPolicy::Retail, false, None).expect("missing"),
            Q3MountRestriction::Demo { .. }
        ));
        let mut corrupt = valid;
        corrupt[7] ^= 0xff;
        assert!(resolve_q3_mount_restriction(Q3ProductPolicy::Retail, false, Some(&corrupt)).is_err());
    }

    // -- bot debug ----------------------------------------------------------------

    struct FakePolygons {
        next: i32,
    }

    impl BotDebugPolygons for FakePolygons {
        fn create(&mut self, _color: i32, count: usize, points: &[Vec3]) -> i32 {
            assert_eq!(count, points.len());
            let id = self.next;
            self.next += 1;
            id
        }
    }

    #[test]
    fn bot_debug_polygons_allocate_ids() {
        let mut sink = FakePolygons { next: 4 };
        let id = sink.create(2, 1, &[vec3(1.0, 2.0, 3.0)]);
        assert_eq!(id, 4);
        assert_eq!(sink.create(2, 0, &[]), 5);
    }
}
