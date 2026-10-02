//! Quake III team-arena group tests.
//!
//! Donor provenance: `src/content/q3/team-arena` (behavioral coverage for the
//! team rules, arenas, match, client, command, and session modules).

use qa_core::cmd::Dialect;
use qa_core::cvar::{set_info_value, InfoOptions, InfoTarget};
use qa_core::math::{vec3, Bounds, Vec3};

use crate::q3::base::game::combat::DamageFlags;
use crate::q3::base::game::entities::ItemDefinition;
use crate::q3::base::game::format::{game_format, game_format_bounded, GameFormatArgument};
use crate::q3::base::game::numeric::{game_atof, game_atoi};
use crate::q3::base::game::save_module_values::Q3CvarSnapshot;
use crate::q3::base::game::spawn::SpawnVariables;
use crate::q3::base::game::state::{ClientSession, ConnectionState, GameFlags, MAX_CLIENTS, MAX_GENTITIES};
use crate::q3::base::game::state::{SpectatorState, TeamState};
use crate::q3::base::shared::definitions::*;
use crate::q3::base::shared::entity_shared::ServerEntityFlags;
use crate::q3::base::shared::entity_state::EntityState;
use crate::q3::base::shared::items::{player_touches_item, Trajectory as ItemsTrajectory};
use crate::q3::base::shared::player_state::{
    CommandButtons, MoveFlags, PlayerAnimation, UserCommand, ENTITYNUM_NONE, ENTITYNUM_WORLD,
};
use crate::q3::base::shared::trajectory::{evaluate_trajectory, Trajectory, TrajectoryType};
use crate::q3::base::world::{
    ActorTraceHit, ActorTraceQuery, ActorTraceResult, LinkState, TraceContact, TraceSolidity,
};
use crate::q3::team_arena::client_admission::*;
use crate::q3::team_arena::client_policy::*;
use crate::q3::team_arena::client_spawn::*;
use crate::q3::team_arena::client_think::*;
use crate::q3::team_arena::commands::*;
use crate::q3::team_arena::r#match::*;
use crate::q3::team_arena::server_commands::*;
use crate::q3::team_arena::support::*;
use crate::q3::team_arena::team::*;

use qa_core::identity::{ActorId, IdentityOwner};

use crate::q3::team_arena::arenas::*;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use crate::q3::team_arena::client_effects::*;
use crate::q3::team_arena::client_events::*;

use crate::q3::team_arena::foreign_objectives::*;
use crate::q3::team_arena::movement_host::*;
use crate::q3::team_arena::objective_placement::*;

use crate::q3::team_arena::session::*;

struct TestRankings {
    calls: RefCell<Vec<String>>,
}

impl RankingsHost for TestRankings {
    fn use_holdable(&self, slot: usize, holdable: i32) {
        self.calls.borrow_mut().push(format!("holdable {slot} {holdable}"));
    }

    fn capture(&self, slot: usize) {
        self.calls.borrow_mut().push(format!("capture {slot}"));
    }

    fn pickup_powerup(&self, slot: usize, powerup: i32) {
        self.calls.borrow_mut().push(format!("powerup {slot} {powerup}"));
    }
}

fn base_max_health_slot() -> usize {
    let StatSchema::Base(base) = stat_schema(Product::Baseq3) else {
        panic!("baseq3 must use the base schema");
    };
    base.max_health as usize
}

fn test_pool(product: Product, max_clients: usize, time: Rc<Cell<i32>>, log: Rc<RefCell<Vec<String>>>) -> PoolRef {
    let time_hook: Rc<dyn Fn() -> i32> = Rc::new({
        let time = time.clone();
        move || time.get()
    });
    let link_log = log.clone();
    let unlink_log = log.clone();
    let print_log = log.clone();
    Rc::new(EntityPool::new(
        product,
        max_clients,
        PoolHooks {
            time: time_hook,
            map_start_time: 0,
            link: Rc::new(move |entity: &EntityRef| {
                link_log.borrow_mut().push(format!("link {}", entity.borrow().slot));
            }),
            unlink: Rc::new(move |entity: &EntityRef| {
                unlink_log.borrow_mut().push(format!("unlink {}", entity.borrow().slot));
            }),
            print: Rc::new(move |text: &str| {
                print_log.borrow_mut().push(format!("print {text}"));
            }),
        },
        Box::new(TestRankings {
            calls: RefCell::new(Vec::new()),
        }),
    ))
}

struct StubWorld {
    links: RefCell<Vec<usize>>,
    unlinks: RefCell<Vec<i32>>,
    states: RefCell<HashMap<i32, LinkState>>,
    contents: Cell<i32>,
    trace: RefCell<ActorTraceResult>,
    area: RefCell<Vec<ActorId>>,
    contact: Cell<bool>,
}

impl StubWorld {
    fn new() -> Self {
        Self {
            links: RefCell::new(Vec::new()),
            unlinks: RefCell::new(Vec::new()),
            states: RefCell::new(HashMap::new()),
            contents: Cell::new(0),
            trace: RefCell::new(ActorTraceResult {
                fraction: 1.0,
                end: vec3(0.0, 0.0, 0.0),
                hit: ActorTraceHit::None,
                contact: TraceContact::None,
                solidity: TraceSolidity::Clear,
                contents: 0,
                surface_flags: 0,
            }),
            area: RefCell::new(Vec::new()),
            contact: Cell::new(false),
        }
    }
}

impl Q3World for StubWorld {
    fn link(&self, entity: &EntityRef) {
        self.links.borrow_mut().push(entity.borrow().slot);
    }

    fn unlink(&self, number: i32) {
        self.unlinks.borrow_mut().push(number);
    }

    fn link_state(&self, number: i32) -> Option<LinkState> {
        self.states.borrow().get(&number).cloned()
    }

    fn point_contents(&self, _point: Vec3, _pass_entity: i32) -> i32 {
        self.contents.get()
    }

    fn trace_actor(&self, _query: &ActorTraceQuery) -> ActorTraceResult {
        self.trace.borrow().clone()
    }

    fn area_actors(&self, _bounds: &Bounds, _maximum: usize) -> Vec<ActorId> {
        self.area.borrow().clone()
    }

    fn contact_actor(&self, _bounds: &Bounds, _actor: &ActorId) -> bool {
        self.contact.get()
    }
}

struct StubCombat {
    product: Product,
    time: Cell<i32>,
    game_type: Cell<i32>,
    intermission_queued: Cell<i32>,
    pool: PoolRef,
    damage_calls: RefCell<Vec<(usize, i32, i32)>>,
}

impl Combat for StubCombat {
    fn product(&self) -> Product {
        self.product
    }

    fn time(&self) -> i32 {
        self.time.get()
    }

    fn game_type(&self) -> i32 {
        self.game_type.get()
    }

    fn intermission_queued(&self) -> i32 {
        self.intermission_queued.get()
    }

    fn pool(&self) -> PoolRef {
        self.pool.clone()
    }

    fn damage(
        &self,
        target: &EntityRef,
        _inflictor: Option<&DamageParticipant>,
        _attacker: Option<&DamageParticipant>,
        _direction: Option<Vec3>,
        _point: Option<Vec3>,
        amount: i32,
        flags: i32,
        method: i32,
    ) {
        let _ = flags;
        self.damage_calls
            .borrow_mut()
            .push((target.borrow().slot, amount, method));
    }
}

struct StubItems {
    tags: RefCell<HashMap<usize, i32>>,
    by_name: RefCell<HashMap<String, ItemDefinition>>,
    by_powerup: RefCell<HashMap<i32, ItemDefinition>>,
    log: RefCell<Vec<String>>,
}

impl StubItems {
    fn new() -> Self {
        Self {
            tags: RefCell::new(HashMap::new()),
            by_name: RefCell::new(HashMap::new()),
            by_powerup: RefCell::new(HashMap::new()),
            log: RefCell::new(Vec::new()),
        }
    }
}

impl ItemHost for StubItems {
    fn item_at(&self, _product: Product, index: usize) -> ItemDefinition {
        ItemDefinition {
            class_name: Some(format!("item{index}")),
            pickup_name: None,
            quantity: 0,
            item_type: ItemType::ItBad,
            tag: self.tags.borrow().get(&index).copied().unwrap_or(index as i32),
        }
    }

    fn find_item(&self, _product: Product, name: &str) -> Option<ItemDefinition> {
        self.by_name.borrow().get(name).cloned()
    }

    fn find_item_for_powerup(&self, _product: Product, powerup: i32) -> Option<ItemDefinition> {
        self.by_powerup.borrow().get(&powerup).cloned()
    }

    fn spawn_item(&self, entity: &EntityRef, item: &ItemDefinition, _vars: &SpawnVariables, disabled: bool) {
        entity.borrow_mut().item = Some(item.clone());
        self.log.borrow_mut().push(format!("spawn disabled={disabled}"));
    }

    fn finish_spawning_item(&self, _entity: &EntityRef) {
        self.log.borrow_mut().push("finish".to_string());
    }

    fn touch_item(&self, entity: &EntityRef, _other: &DamageParticipant, _contact: &TouchContact) {
        self.log.borrow_mut().push(format!("touch {}", entity.borrow().slot));
    }
}

struct StubEffects {
    combat: CombatRef,
    items: Rc<StubItems>,
    intermission_time: Cell<i32>,
    smooth: Cell<bool>,
    fry: Cell<i32>,
    random: Cell<i32>,
    sounds: RefCell<Vec<(usize, i32, i32)>>,
    indexes: RefCell<Vec<String>>,
    spectator_frames: RefCell<Vec<usize>>,
}

impl EffectsCore for StubEffects {
    fn combat(&self) -> CombatRef {
        self.combat.clone()
    }

    fn items(&self) -> Rc<dyn ItemHost> {
        self.items.clone()
    }
}

impl EffectsHost for StubEffects {
    fn intermission_time(&self) -> i32 {
        self.intermission_time.get()
    }

    fn smooth_clients(&self) -> bool {
        self.smooth.get()
    }

    fn fry_sound(&self) -> i32 {
        self.fry.get()
    }

    fn random_int(&self) -> i32 {
        self.random.get()
    }

    fn sound_index(&self, path: &str) -> i32 {
        self.indexes.borrow_mut().push(path.to_string());
        self.indexes.borrow().len() as i32
    }

    fn sound(&self, entity: &EntityRef, channel: i32, sound_index: i32) {
        self.sounds
            .borrow_mut()
            .push((entity.borrow().slot, channel, sound_index));
    }

    fn spectator_end_frame(&self, entity: &EntityRef) {
        self.spectator_frames.borrow_mut().push(entity.borrow().slot);
    }
}

struct StubCvars {
    values: RefCell<HashMap<String, (String, i32)>>,
    sets: RefCell<Vec<(String, String)>>,
}

impl CvarRegistry for StubCvars {
    fn get(&self, name: &str) -> Option<Q3CvarSnapshot> {
        self.values.borrow().get(name).map(|(value, integer)| Q3CvarSnapshot {
            name: name.to_string(),
            value: value.clone(),
            reset_value: String::new(),
            latched_value: None,
            flags: 0,
            modified: false,
            modification_count: 0,
            numeric_value: f64::from(*integer),
            integer_value: *integer,
        })
    }

    fn set(&self, name: &str, value: &str, _force: bool) {
        self.sets.borrow_mut().push((name.to_string(), value.to_string()));
    }
}

#[test]
fn codes_match_donor_values() {
    assert_eq!(GameType::Gt1fctf as i32, 5);
    assert_eq!(GameType::GtHarvester as i32, 7);
    assert_eq!(Team::TeamNumTeams as i32, 4);
    assert_eq!(MoveType::PmSpintermission as i32, 6);
    assert_eq!(Powerup::PwInvulnerability as i32, 14);
    assert_eq!(Weapon::WpChaingun as i32, 13);
    assert_eq!(EntityEvent::EvTauntPatrol as i32, 82);
    assert_eq!(EntityEvent::EvKamikaze as i32, 68);
    assert_eq!(EntityType::EtEvents as i32, 13);
    assert_eq!(PersistentIndex::PersCaptures as i32, 14);
    assert_eq!(TrajectoryType::TrGravity as i32, 5);
    assert_eq!(PlayerAnimation::FlagStand2Run as i32, 36);
    assert_eq!(MoveFlags::InvulExpand as i32, 16384);
    assert_eq!(CommandButtons::Any as i32, 2048);
    assert_eq!(ConnectionState::Connected as i32, 2);
    assert_eq!(SpectatorState::Scoreboard as i32, 3);
    assert_eq!(TeamState::Active as i32, 1);
    assert_eq!(GameFlags::FORCE_GESTURE, 0x8000);
    assert_eq!(ServerEntityFlags::Notsingleclient as i32, 2048);
    assert_eq!(DamageFlags::NO_TEAM_PROTECTION, 0x10);
    assert_eq!(ENTITYNUM_WORLD, 1022);
    assert_eq!(ENTITYNUM_NONE, 1023);
    assert_eq!(MAX_CLIENTS, 64);
    assert_eq!(MAX_GENTITIES, 1024);
    assert_eq!(GIB_HEALTH, -40);
    assert_eq!(EV_EVENT_BITS, 0x300);
    assert_eq!(flag_status::DROPPED, 4);
    assert_eq!(global_team_sound::KAMIKAZE, 13);
    let StatSchema::Base(base) = stat_schema(Product::Baseq3) else {
        panic!("baseq3 must use the base schema");
    };
    assert_eq!((base.health, base.weapons, base.max_health), (0, 2, 6));
    let StatSchema::Missionpack(mp) = stat_schema(Product::Missionpack) else {
        panic!("missionpack must use the missionpack schema");
    };
    assert_eq!((mp.weapons, mp.max_health), (3, 7));
    assert_eq!(mp.persistent_powerup, 2);
    assert_eq!(weapon_count(Product::Baseq3), 11);
    assert_eq!(weapon_count(Product::Missionpack), 14);
}

#[test]
fn game_atoi_wraps_and_skips() {
    assert_eq!(game_atoi("  -42x").unwrap(), -42);
    assert_eq!(game_atoi("+7").unwrap(), 7);
    assert_eq!(game_atoi("").unwrap(), 0);
    assert_eq!(game_atoi("   ").unwrap(), 0);
    assert_eq!(game_atoi("2147483648").unwrap(), -2147483648);
    assert_eq!(game_atoi("9999999999").unwrap(), 1410065407);
    assert_eq!(game_atoi("12abc34").unwrap(), 12);
}

#[test]
fn game_atof_reads_decimal_prefix() {
    assert_eq!(game_atof("1.5").unwrap(), 1.5);
    assert_eq!(game_atof("-2.25x").unwrap(), -2.25);
    assert_eq!(game_atof("abc").unwrap(), 0.0);
    assert_eq!(game_atof(".5").unwrap(), 0.5);
    assert_eq!(game_atof("3.").unwrap(), 3.0);
    assert_eq!(game_atof("  10").unwrap(), 10.0);
}

#[test]
fn game_random_is_deterministic() {
    let first = GameRandom::new(1);
    let second = GameRandom::new(1);
    for _ in 0..8 {
        assert_eq!(first.rand(), second.rand());
    }
    assert_eq!(GameRandom::new(0).rand(), 1);
    let fraction = GameRandom::new(42).random();
    assert!((0.0..=1.0).contains(&fraction));
}

#[test]
fn game_format_covers_used_specifiers() {
    let args = |values: Vec<GameFormatArgument>| values;
    assert_eq!(game_format("%i", &args(vec![42.into()])), "42");
    assert_eq!(game_format("%3i:", &args(vec![5.into()])), "  5:");
    assert_eq!(game_format("%-3i:", &args(vec![5.into()])), "5  :");
    assert_eq!(game_format("%03i", &args(vec![5.into()])), "005");
    assert_eq!(game_format("%d", &args(vec![(-7).into()])), "-7");
    assert_eq!(game_format("%s", &args(vec!["hi".into()])), "hi");
    assert_eq!(game_format("%s", &args(vec![GameFormatArgument::Null])), "(null)");
    assert_eq!(game_format("%c", &args(vec![94.into()])), "^");
    assert_eq!(game_format("%%", &args(vec![])), "%");
    assert_eq!(
        game_format("n\\%s\\t\\%i", &args(vec!["bob".into(), 1.into()])),
        "n\\bob\\t\\1"
    );
    assert_eq!(game_format_bounded("ab\0cd", &args(vec![]), 32), "ab");
    assert_eq!(
        game_format_bounded("%s%s", &args(vec!["aa".into(), "bb".into()]), 3),
        "aa"
    );
    assert_eq!(game_format_bounded("%.2s", &args(vec!["abcdef".into()]), 32), "ab");
}

#[test]
fn trajectories_evaluate() {
    let linear = Trajectory {
        trajectory_type: TrajectoryType::TrLinear,
        time: 1000,
        duration: 0,
        base: vec3(1.0, 2.0, 3.0),
        delta: vec3(10.0, 0.0, 0.0),
    };
    assert_eq!(evaluate_trajectory(&linear, 2000), vec3(11.0, 2.0, 3.0));
    let gravity = Trajectory {
        trajectory_type: TrajectoryType::TrGravity,
        time: 0,
        duration: 0,
        base: vec3(0.0, 0.0, 100.0),
        delta: vec3(0.0, 0.0, 0.0),
    };
    assert_eq!(evaluate_trajectory(&gravity, 1000), vec3(0.0, 0.0, -300.0));
    let stopped = Trajectory {
        trajectory_type: TrajectoryType::TrLinearStop,
        time: 0,
        duration: 500,
        base: vec3(0.0, 0.0, 0.0),
        delta: vec3(2.0, 0.0, 0.0),
    };
    assert_eq!(evaluate_trajectory(&stopped, 5000), vec3(1.0, 0.0, 0.0));
    let sine = Trajectory {
        trajectory_type: TrajectoryType::TrSine,
        time: 0,
        duration: 1000,
        base: vec3(5.0, 0.0, 0.0),
        delta: vec3(2.0, 0.0, 0.0),
    };
    let at = evaluate_trajectory(&sine, 250);
    assert!((at.x - 7.0).abs() < 1e-5);
    let resting = ItemsTrajectory {
        trajectory_type: TrajectoryType::TrStationary as i32,
        time: 0,
        duration: 0,
        base: vec3(0.0, 0.0, 0.0),
        delta: vec3(0.0, 0.0, 0.0),
    };
    assert!(!player_touches_item(vec3(100.0, 0.0, 0.0), &resting, 0).unwrap());
    assert!(player_touches_item(vec3(0.0, 0.0, 0.0), &resting, 0).unwrap());
}

#[test]
fn info_values_round_trip() {
    let printed = Rc::new(RefCell::new(Vec::new()));
    let mut print = {
        let printed = printed.clone();
        move |text: &str| printed.borrow_mut().push(text.to_string())
    };
    let options = InfoOptions {
        dialect: Dialect::Q3,
        maximum_length: 1024,
        target: InfoTarget::ClientUserinfo,
        server_high_characters: false,
    };
    let info = set_info_value("", "name", "bob", options, &mut print).unwrap();
    assert_eq!(info, "\\name\\bob");
    let info = set_info_value(&info, "name", "al", options, &mut print).unwrap();
    assert_eq!(info, "\\name\\al");
    let ordered = set_info_value("\\a\\b", "c", "d", options, &mut print).unwrap();
    assert_eq!(ordered, "\\c\\d\\a\\b");
    let info = set_info_value(&info, "name", "", options, &mut print).unwrap();
    assert_eq!(info, "");
    let before = "\\a\\b".to_string();
    assert_eq!(
        set_info_value(&before, "k;ey", "v", options, &mut print).unwrap(),
        before
    );
    assert!(!printed.borrow().is_empty());
}

#[test]
fn slots_check_bounds() {
    let slots = SlotArray::new(4);
    slots.set(3, 9);
    assert_eq!(slots.get(3), 9);
    assert_eq!(slots.copy(), vec![0, 0, 0, 9]);
}

#[test]
#[should_panic(expected = "outside 4")]
fn slots_reject_overflow() {
    let _ = SlotArray::new(4).get(4);
}

#[test]
fn pool_spawns_and_recycles() {
    let time = Rc::new(Cell::new(5000));
    let log = Rc::new(RefCell::new(Vec::new()));
    let pool = test_pool(Product::Baseq3, 2, time.clone(), log);
    assert_eq!(pool.num_entities(), MAX_CLIENTS);
    let first = pool.spawn();
    assert_eq!(first.borrow().slot, MAX_CLIENTS);
    assert_eq!(first.borrow().s.number, MAX_CLIENTS as i32);
    let temp = pool.temp_entity(vec3(1.5, 2.5, 3.5), EntityEvent::EvRailtrail as i32);
    assert_eq!(
        temp.borrow().s.e_type,
        EntityType::EtEvents as i32 + EntityEvent::EvRailtrail as i32
    );
    assert_eq!(temp.borrow().s.pos.base, vec3(1.0, 2.0, 3.0));
    assert!(temp.borrow().free_after_event);
    pool.free(&first);
    time.set(5500);
    let reused = pool.spawn();
    // Freed less than a second ago, so the pool extends instead.
    assert_ne!(reused.borrow().slot, first.borrow().slot);
    pool.free(&reused);
    pool.free(&temp);
    time.set(9000);
    let recycled = pool.spawn();
    assert_eq!(recycled.borrow().slot, MAX_CLIENTS);
    assert_eq!(recycled.borrow().classname(), Some("noclass".to_string()));
}

#[test]
fn pool_events_set_bits() {
    let time = Rc::new(Cell::new(100));
    let log = Rc::new(RefCell::new(Vec::new()));
    let pool = test_pool(Product::Baseq3, 2, time, log);
    let entity = pool.at(0);
    pool.add_event(&entity, EntityEvent::EvPain as i32, 55);
    let record = entity.borrow().client.clone().unwrap();
    assert_eq!(
        record.borrow().ps.external_event,
        EntityEvent::EvPain as i32 | EV_EVENT_BIT1
    );
    assert_eq!(record.borrow().ps.external_event_parm, 55);
    let temp = pool.temp_entity(vec3(0.0, 0.0, 0.0), EntityEvent::EvBullet as i32);
    pool.add_event(&temp, EntityEvent::EvBulletHitWall as i32, 3);
    assert_eq!(
        temp.borrow().s.event,
        EntityEvent::EvBulletHitWall as i32 | EV_EVENT_BIT1
    );
    assert_eq!(pool.vtos(vec3(1.2, -3.7, 4.0)), "(1 -3 4)");
}

#[test]
fn entity_search_is_case_insensitive() {
    let time = Rc::new(Cell::new(0));
    let log = Rc::new(RefCell::new(Vec::new()));
    let pool = test_pool(Product::Baseq3, 1, time, log);
    let entity = pool.spawn();
    entity
        .borrow_mut()
        .set_classname(Some("Info_Player_Deathmatch".to_string()));
    let found = find_entity(
        &pool,
        None,
        EntityStringField::Classname,
        Some("info_player_deathmatch"),
    );
    assert!(found.map(|entry| Rc::ptr_eq(&entry, &entity)).unwrap_or(false));
    assert!(find_entity(
        &pool,
        Some(&entity),
        EntityStringField::Classname,
        Some("info_player_deathmatch")
    )
    .is_none());
}

#[test]
fn target_pick_warns_and_selects() {
    let time = Rc::new(Cell::new(0));
    let log = Rc::new(RefCell::new(Vec::new()));
    let pool = test_pool(Product::Baseq3, 1, time, log);
    let warnings = Rc::new(RefCell::new(Vec::new()));
    let warn = {
        let warnings = warnings.clone();
        move |text: &str| warnings.borrow_mut().push(text.to_string())
    };
    assert!(pick_target(&pool, &|| 0, &warn, None).is_none());
    assert!(pick_target(&pool, &|| 0, &warn, Some("missing")).is_none());
    assert_eq!(warnings.borrow().len(), 2);
    for name in ["t1", "t1"] {
        let entity = pool.spawn();
        entity.borrow_mut().targetname = Some(name.to_string());
    }
    let picked = pick_target(&pool, &|| 1, &warn, Some("t1")).unwrap();
    assert_eq!(picked.borrow().targetname, Some("t1".to_string()));
}

#[test]
fn snapshot_mapping_marks_dead_and_events() {
    let mut ps = PlayerState::new(Product::Baseq3);
    ps.client_num = 3;
    ps.origin = vec3(10.0, 20.0, 30.0);
    ps.velocity = vec3(1.0, 0.0, 0.0);
    ps.viewangles = vec3(0.0, 90.0, 0.0);
    ps.movement_dir = 2;
    ps.set_health(-50);
    ps.events.set(0, EntityEvent::EvJump as i32);
    ps.event_sequence = 1;
    ps.powerups.set(2, 999);
    let mut state = EntityState::default();
    player_state_to_entity_state(&mut ps, &mut state, true);
    assert_eq!(state.e_type, EntityType::EtInvisible as i32);
    assert_eq!(state.number, 3);
    assert_eq!(state.event, EntityEvent::EvJump as i32);
    assert_eq!(state.powerups, 1 << 2);
    assert_eq!(ps.entity_event_sequence, 1);
    player_state_to_entity_state_extrapolate(&mut ps, &mut state, 500, true);
    assert_eq!(state.pos.trajectory_type, TrajectoryType::TrLinearStop);
    assert_eq!(state.pos.time, 500);
    assert_eq!(state.pos.duration, 50);
}

struct StubSessionCvars {
    values: RefCell<HashMap<String, String>>,
}

impl SessionCvarService for StubSessionCvars {
    fn get(&self, name: &SessionCvarName) -> String {
        self.values.borrow().get(&name.as_string()).cloned().unwrap_or_default()
    }

    fn set(&self, name: &SessionCvarName, value: &str) {
        self.values.borrow_mut().insert(name.as_string(), value.to_string());
    }
}

struct StubSessionServices {
    prints: RefCell<Vec<String>>,
    teams: RefCell<Vec<(usize, i32)>>,
}

impl SessionServices for StubSessionServices {
    fn print(&self, message: &str) {
        self.prints.borrow_mut().push(message.to_string());
    }

    fn broadcast_team_change(&self, client_num: usize, old_team: i32) {
        self.teams.borrow_mut().push((client_num, old_team));
    }
}

struct TeamUserinfo {
    team: String,
}

impl SessionUserinfo for TeamUserinfo {
    fn value_for_key(&self, _key: &str) -> String {
        self.team.clone()
    }
}

fn session_fixture(game_type: i32) -> (Rc<SessionWorld>, Rc<GameSessionManager>, Rc<StubSessionCvars>) {
    let time = Rc::new(Cell::new(1000));
    let log = Rc::new(RefCell::new(Vec::new()));
    let pool = test_pool(Product::Baseq3, 4, time, log);
    let world = Rc::new(SessionWorld {
        clients: pool.clients()[..4].to_vec(),
        max_clients: 4,
        team_scores: Rc::new(SlotArray::new(4)),
        game_type: Cell::new(game_type),
        team_auto_join: Cell::new(false),
        max_game_clients: Cell::new(0),
        time: Cell::new(1000),
        num_non_spectator_clients: Cell::new(0),
        new_session: Cell::new(false),
    });
    let cvars = Rc::new(StubSessionCvars {
        values: RefCell::new(HashMap::new()),
    });
    let manager = Rc::new(GameSessionManager::new(
        world.clone(),
        Rc::new(StubSessionServices {
            prints: RefCell::new(Vec::new()),
            teams: RefCell::new(Vec::new()),
        }),
        cvars.clone(),
    ));
    (world, manager, cvars)
}

#[test]
fn session_write_read_round_trip() {
    let (world, manager, cvars) = session_fixture(GameType::GtFfa as i32);
    {
        let client = &world.clients[1];
        let mut record = client.borrow_mut();
        record.sess.session_team = Team::TeamRed as i32;
        record.sess.spectator_time = 11;
        record.sess.spectator_state = SpectatorState::Follow as i32;
        record.sess.spectator_client = 2;
        record.sess.wins = 3;
        record.sess.losses = 4;
        record.sess.team_leader = 1;
    }
    manager.write_client(1);
    assert_eq!(cvars.get(&SessionCvarName::SessionN(1)), "1 11 2 2 3 4 1");
    world.clients[1].borrow_mut().sess = ClientSession::default();
    manager.read_client(1);
    let record = world.clients[1].borrow();
    assert_eq!(record.sess.session_team, Team::TeamRed as i32);
    assert_eq!(
        (record.sess.wins, record.sess.losses, record.sess.team_leader),
        (3, 4, 1)
    );
}

#[test]
fn session_initialize_assigns_teams() {
    let (world, manager, _) = session_fixture(GameType::GtFfa as i32);
    manager.initialize_client(0, &TeamUserinfo { team: "s".to_string() });
    assert_eq!(world.clients[0].borrow().sess.session_team, Team::TeamSpectator as i32);
    manager.initialize_client(1, &TeamUserinfo { team: "".to_string() });
    assert_eq!(world.clients[1].borrow().sess.session_team, Team::TeamFree as i32);

    let (world, manager, _) = session_fixture(GameType::GtTournament as i32);
    world.num_non_spectator_clients.set(2);
    manager.initialize_client(0, &TeamUserinfo { team: "".to_string() });
    assert_eq!(world.clients[0].borrow().sess.session_team, Team::TeamSpectator as i32);

    let (world, manager, _) = session_fixture(GameType::GtTeam as i32);
    world.team_auto_join.set(true);
    world.clients[0].borrow_mut().pers.connected = ConnectionState::Connected as i32;
    world.clients[0].borrow_mut().sess.session_team = Team::TeamRed as i32;
    manager.initialize_client(1, &TeamUserinfo { team: "".to_string() });
    assert_eq!(world.clients[1].borrow().sess.session_team, Team::TeamBlue as i32);

    let (world, manager, _) = session_fixture(GameType::GtCtf as i32);
    manager.initialize_world();
    assert!(world.new_session.get());
    manager.write_world();
}

#[test]
fn placements_cover_modes() {
    let placed = |names: &[&str]| -> Vec<ObjectivePlacement> {
        names
            .iter()
            .map(|name| ObjectivePlacement {
                classname: Some(name.to_string()),
            })
            .collect()
    };
    assert_eq!(
        check_objective_placements(Product::Baseq3, GameType::GtFfa as i32, &[]),
        ObjectivePlacementResult::Ready
    );
    assert_eq!(
        check_objective_placements(
            Product::Baseq3,
            GameType::GtCtf as i32,
            &placed(&["team_CTF_redflag", "team_CTF_blueflag"]),
        ),
        ObjectivePlacementResult::Ready
    );
    assert_eq!(
        check_objective_placements(Product::Baseq3, GameType::GtCtf as i32, &placed(&["team_CTF_redflag"])),
        ObjectivePlacementResult::MissingObjectives {
            classnames: vec![ObjectiveClassname::BlueFlag],
        }
    );
    assert_eq!(
        check_objective_placements(Product::Baseq3, GameType::Gt1fctf as i32, &[]),
        ObjectivePlacementResult::UnsupportedMode {
            product: Product::Baseq3,
            game_type: GameType::Gt1fctf as i32,
        }
    );
    assert_eq!(
        check_objective_placements(
            Product::Missionpack,
            GameType::GtHarvester as i32,
            &placed(&["team_redobelisk", "team_blueobelisk", "team_neutralobelisk"]),
        ),
        ObjectivePlacementResult::Ready
    );
}

#[test]
fn foreign_objectives_translate_and_place() {
    let q3 = ForeignWorld {
        kind: WorldKind::Q3Bsp,
        entities: "{\n\"classname\" \"team_CTF_redflag\"\n}\n{\n\"classname\" \"team_CTF_blueflag\"\n}\n{\n\"classname\" \"info_player_deathmatch\"\n}\n".to_string(),
    };
    match adapt_foreign_q3_objectives(&q3, Product::Baseq3, GameType::GtCtf as i32, &[]) {
        ForeignObjectiveAdaptation::Ready { translated, .. } => assert_eq!(translated, 0),
        other => panic!("unexpected {other:?}"),
    }
    match adapt_foreign_q3_objectives(&q3, Product::Baseq3, GameType::Gt1fctf as i32, &[]) {
        ForeignObjectiveAdaptation::UnsupportedMode { .. } => {}
        other => panic!("unexpected {other:?}"),
    }
    let q1 = ForeignWorld {
        kind: WorldKind::Q1Bsp,
        entities: "{\n\"classname\" \"item_flag_team1\"\n}\n{\n\"classname\" \"item_flag_team2\"\n}\n{\n\"classname\" \"info_player_start\"\n}\n".to_string(),
    };
    match adapt_foreign_q3_objectives(&q1, Product::Baseq3, GameType::GtCtf as i32, &[]) {
        ForeignObjectiveAdaptation::Ready { entities, translated } => {
            assert_eq!(translated, 3);
            assert!(entities.contains("\"team_CTF_redflag\""));
            assert!(entities.contains("\"info_player_deathmatch\""));
        }
        other => panic!("unexpected {other:?}"),
    }
    let bare = ForeignWorld {
        kind: WorldKind::Q1Bsp,
        entities: "{\n\"classname\" \"info_player_start\"\n}\n".to_string(),
    };
    let explicit = vec![
        ExplicitObjectivePlacement {
            classname: ObjectiveClassname::RedFlag,
            origin: vec3(1.0, 2.0, 3.0),
        },
        ExplicitObjectivePlacement {
            classname: ObjectiveClassname::BlueFlag,
            origin: vec3(4.0, 5.0, 6.0),
        },
    ];
    match adapt_foreign_q3_objectives(&bare, Product::Baseq3, GameType::GtCtf as i32, &explicit) {
        ForeignObjectiveAdaptation::Ready { entities, .. } => {
            assert!(entities.contains("\"1 2 3\""));
        }
        other => panic!("unexpected {other:?}"),
    }
}

#[test]
#[should_panic(expected = "authored player spawn")]
fn foreign_objectives_require_spawn() {
    let world = ForeignWorld {
        kind: WorldKind::Q1Bsp,
        entities: "{\n\"classname\" \"item_flag_team1\"\n}\n{\n\"classname\" \"item_flag_team2\"\n}\n".to_string(),
    };
    let _ = adapt_foreign_q3_objectives(&world, Product::Baseq3, GameType::GtCtf as i32, &[]);
}

#[test]
fn admission_names_and_configs() {
    assert_eq!(clean_client_name("  ^1Bob  ^2"), "^1Bob  ^2");
    assert_eq!(clean_client_name(""), "UnnamedPlayer");
    assert_eq!(clean_client_name("^0^1"), "UnnamedPlayer");
    assert_eq!(clean_client_name("a    b"), "a   b");
    assert_eq!(client_info_value("\\name\\bob\\TEAM\\red", "team"), "red");
    assert_eq!(client_info_value("name\\bob", "missing"), "");
    let time = Rc::new(Cell::new(0));
    let log = Rc::new(RefCell::new(Vec::new()));
    let pool = test_pool(Product::Baseq3, 1, time, log);
    let client = pool.client_at(0);
    client.borrow_mut().pers.netname = "bob".to_string();
    client.borrow_mut().sess.session_team = Team::TeamRed as i32;
    let config = client_presentation_config(
        &client,
        "\\model\\sarge\\headmodel\\sarge\\color1\\4\\color2\\5",
        GameType::GtFfa as i32,
        None,
    );
    assert!(config.starts_with("n\\bob\\t\\1\\model\\sarge"));
    assert!(config.contains("\\c1\\4\\c2\\5"));
}

struct StubServerHost {
    cvars: RefCell<HashMap<String, Q3CvarSnapshot>>,
    prints: RefCell<Vec<String>>,
    commands: RefCell<Vec<(i32, String)>>,
    console: RefCell<Vec<String>>,
    teams: RefCell<Vec<(usize, String)>>,
}

impl GameServerCommandHost for StubServerHost {
    fn read_vm_cvar(&self, name: ServerCommandCvar) -> Q3CvarSnapshot {
        self.cvars
            .borrow()
            .get(name.as_str())
            .cloned()
            .unwrap_or(Q3CvarSnapshot {
                name: String::new(),
                value: String::new(),
                reset_value: String::new(),
                latched_value: None,
                flags: 0,
                modified: false,
                modification_count: 0,
                numeric_value: 0.0,
                integer_value: 0,
            })
    }

    fn print(&self, text: &str) {
        self.prints.borrow_mut().push(text.to_string());
    }

    fn send_server_command(&self, client_num: i32, text: &str) {
        self.commands.borrow_mut().push((client_num, text.to_string()));
    }

    fn execute_console_now(&self, text: &str) {
        self.console.borrow_mut().push(text.to_string());
    }

    fn set_team(&self, entity: &EntityRef, team: &str) {
        self.teams.borrow_mut().push((entity.borrow().slot, team.to_string()));
    }

    fn bots(&self) -> ServerCommandCapability {
        ServerCommandCapability::Unavailable {
            reason: "no bots".to_string(),
        }
    }

    fn memory(&self) -> ServerCommandCapability {
        ServerCommandCapability::Available { run: Rc::new(|_| {}) }
    }

    fn podium(&self) -> ServerCommandCapability {
        ServerCommandCapability::Available { run: Rc::new(|_| {}) }
    }
}

fn server_fixture() -> (GameServerCommandRuntime, Rc<StubServerHost>, Rc<StubCvars>, PoolRef) {
    let time = Rc::new(Cell::new(0));
    let log = Rc::new(RefCell::new(Vec::new()));
    let pool = test_pool(Product::Baseq3, 4, time, log);
    let cvars = Rc::new(StubCvars {
        values: RefCell::new(HashMap::new()),
        sets: RefCell::new(Vec::new()),
    });
    let host = Rc::new(StubServerHost {
        cvars: RefCell::new(HashMap::new()),
        prints: RefCell::new(Vec::new()),
        commands: RefCell::new(Vec::new()),
        console: RefCell::new(Vec::new()),
        teams: RefCell::new(Vec::new()),
    });
    let runtime = GameServerCommandRuntime::new(
        pool.clone(),
        cvars.clone(),
        host.clone(),
        GameServerCommandState::default(),
    );
    (runtime, host, cvars, pool)
}

fn argv(words: &[&str]) -> Vec<String> {
    words.iter().map(|word| word.to_string()).collect()
}

#[test]
fn server_ip_filters_round_trip() {
    let (runtime, host, cvars, _) = server_fixture();
    assert!(runtime.console_command(&argv(&["addip", "192.168.1.*"])));
    assert_eq!(
        cvars.sets.borrow().last(),
        Some(&("g_banIPs".to_string(), "192.168.1.* ".to_string()))
    );
    host.cvars.borrow_mut().insert(
        "g_filterBan".to_string(),
        Q3CvarSnapshot {
            name: "g_filterBan".to_string(),
            value: "1".to_string(),
            reset_value: String::new(),
            latched_value: None,
            flags: 0,
            modified: false,
            modification_count: 0,
            numeric_value: 1.0,
            integer_value: 1,
        },
    );
    assert!(runtime.filter_packet("192.168.1.7"));
    assert!(!runtime.filter_packet("10.0.0.1"));
    assert!(runtime.console_command(&argv(&["removeip", "192.168.1.*"])));
    assert!(host.prints.borrow().iter().any(|line| line == "Removed.\n"));
    assert!(!runtime.filter_packet("192.168.1.7"));
    assert!(runtime.console_command(&argv(&["removeip", "10.0.0.*"])));
    assert!(host.prints.borrow().iter().any(|line| line.contains("Didn't find")));
}

#[test]
fn server_bans_reload_and_list() {
    let (runtime, host, _, _) = server_fixture();
    host.cvars.borrow_mut().insert(
        "g_banIPs".to_string(),
        Q3CvarSnapshot {
            name: "g_banIPs".to_string(),
            value: "10.1.1.1  ".to_string(),
            reset_value: String::new(),
            latched_value: None,
            flags: 0,
            modified: false,
            modification_count: 3,
            numeric_value: 0.0,
            integer_value: 0,
        },
    );
    host.cvars.borrow_mut().insert(
        "g_filterBan".to_string(),
        Q3CvarSnapshot {
            name: "g_filterBan".to_string(),
            value: "1".to_string(),
            reset_value: String::new(),
            latched_value: None,
            flags: 0,
            modified: false,
            modification_count: 0,
            numeric_value: 1.0,
            integer_value: 1,
        },
    );
    runtime.process_ip_bans();
    assert!(runtime.filter_packet("10.1.1.1"));
    assert!(runtime.console_command(&argv(&["listip"])));
    assert_eq!(host.console.borrow().as_slice(), ["g_banIPs\n"]);
    let saved = runtime.capture_save_state();
    runtime.restore_save_state(&saved);
    assert!(runtime.filter_packet("10.1.1.1"));
}

#[test]
fn server_console_lists_forces_and_chats() {
    let (runtime, host, _, pool) = server_fixture();
    let entity = pool.at(0);
    entity.borrow_mut().inuse = true;
    entity.borrow_mut().set_classname(Some("player".to_string()));
    entity.borrow_mut().client.clone().unwrap().borrow_mut().pers.connected = ConnectionState::Connected as i32;
    // The listing skips slot 0 (world), so the listed player lives on slot 1.
    pool.at(1).borrow_mut().inuse = true;
    pool.at(1).borrow_mut().set_classname(Some("player".to_string()));
    assert!(runtime.console_command(&argv(&["entitylist"])));
    assert!(host.prints.borrow().iter().any(|line| line.contains("player")));
    assert!(runtime.console_command(&argv(&["forceteam", "0", "red"])));
    assert_eq!(host.teams.borrow().as_slice(), [(0, "red".to_string())]);
    host.cvars.borrow_mut().insert(
        "dedicated".to_string(),
        Q3CvarSnapshot {
            name: "dedicated".to_string(),
            value: "1".to_string(),
            reset_value: String::new(),
            latched_value: None,
            flags: 0,
            modified: false,
            modification_count: 0,
            numeric_value: 1.0,
            integer_value: 1,
        },
    );
    assert!(runtime.console_command(&argv(&["say", "hello", "there"])));
    assert!(host
        .commands
        .borrow()
        .iter()
        .any(|(_, text)| text.contains("hello there")));
    assert!(runtime.console_command(&argv(&["game_memory"])));
}

#[test]
#[should_panic(expected = "addbot unavailable")]
fn server_missing_capability_panics() {
    let (runtime, _, _, _) = server_fixture();
    runtime.console_command(&argv(&["addbot"]));
}

struct StubSpawnShort {
    respawns: RefCell<Vec<usize>>,
}

impl SpawnShort for StubSpawnShort {
    fn select_spawn_point(&self, _avoid: Vec3) -> SpawnPoint {
        panic!("unexpected spawn selection");
    }

    fn respawn(&self, entity: &EntityRef) {
        self.respawns.borrow_mut().push(entity.borrow().slot);
    }
}

struct StubMatchHost {
    product: Product,
    state: MatchStateRef,
    pool: PoolRef,
    team_scores: SharedSlots,
    random: GameRandom,
    spawn: Rc<StubSpawnShort>,
    settings: RefCell<MatchSettings>,
    log: RefCell<Vec<String>>,
    configstrings: RefCell<HashMap<i32, String>>,
    cvars: RefCell<HashMap<String, String>>,
    single_player: Cell<bool>,
}

impl MatchHost for StubMatchHost {
    fn product(&self) -> Product {
        self.product
    }

    fn state(&self) -> &MatchStateRef {
        &self.state
    }

    fn pool(&self) -> PoolRef {
        self.pool.clone()
    }

    fn team_scores(&self) -> SharedSlots {
        self.team_scores.clone()
    }

    fn random(&self) -> &GameRandom {
        &self.random
    }

    fn spawn(&self) -> Rc<dyn SpawnShort> {
        self.spawn.clone()
    }

    fn settings(&self) -> MatchSettings {
        self.settings.borrow().clone()
    }

    fn set_team(&self, entity: &EntityRef, team: &str) {
        self.log
            .borrow_mut()
            .push(format!("setteam {} {team}", entity.borrow().slot));
    }

    fn stop_following(&self, entity: &EntityRef) {
        self.log.borrow_mut().push(format!("stop {}", entity.borrow().slot));
    }

    fn send_scoreboard(&self, entity: &EntityRef) {
        self.log
            .borrow_mut()
            .push(format!("scoreboard {}", entity.borrow().slot));
    }

    fn client_userinfo_changed(&self, client_num: usize) {
        self.log.borrow_mut().push(format!("userinfo {client_num}"));
    }

    fn write_session_data(&self) {
        self.log.borrow_mut().push("session".to_string());
    }

    fn append_console_command(&self, text: &str) {
        self.log.borrow_mut().push(format!("console {text}"));
    }

    fn send_server_command(&self, client_num: i32, text: &str) {
        self.log.borrow_mut().push(format!("server {client_num} {text}"));
    }

    fn set_configstring(&self, index: i32, text: &str) {
        self.configstrings.borrow_mut().insert(index, text.to_string());
    }

    fn set_cvar(&self, name: &str, value: &str) {
        self.cvars.borrow_mut().insert(name.to_string(), value.to_string());
    }

    fn log(&self, text: &str) {
        self.log.borrow_mut().push(format!("log {text}"));
    }

    fn warn(&self, text: &str) {
        self.log.borrow_mut().push(format!("warn {text}"));
    }

    fn bot_interbreed_end_match(&self) {
        self.log.borrow_mut().push("bots".to_string());
    }

    fn update_tournament_info(&self) {
        self.log.borrow_mut().push("tourney".to_string());
    }

    fn spawn_models_on_victory_pads(&self) {
        self.log.borrow_mut().push("podium".to_string());
    }

    fn single_player(&self) -> bool {
        self.single_player.get()
    }
}

fn match_settings(game_type: i32) -> MatchSettings {
    MatchSettings {
        game_type,
        time_limit: 0,
        frag_limit: 0,
        capture_limit: 0,
        warmup_seconds: 0,
        warmup_modification_count: 0,
        password: String::new(),
        password_modification_count: 0,
    }
}

fn match_fixture(game_type: i32) -> (Rc<MatchRuntime>, Rc<StubMatchHost>) {
    let time = Rc::new(Cell::new(0));
    let log = Rc::new(RefCell::new(Vec::new()));
    let pool = test_pool(Product::Baseq3, 4, time, log);
    let host = Rc::new(StubMatchHost {
        product: Product::Baseq3,
        state: Rc::new(RefCell::new(MatchState::default())),
        pool,
        team_scores: Rc::new(SlotArray::new(4)),
        random: GameRandom::new(7),
        spawn: Rc::new(StubSpawnShort {
            respawns: RefCell::new(Vec::new()),
        }),
        settings: RefCell::new(match_settings(game_type)),
        log: RefCell::new(Vec::new()),
        configstrings: RefCell::new(HashMap::new()),
        cvars: RefCell::new(HashMap::new()),
        single_player: Cell::new(false),
    });
    let runtime = Rc::new(MatchRuntime::new(host.clone(), MatchModuleState::new()));
    (runtime, host)
}

fn connect_client(host: &StubMatchHost, slot: usize, team_code: i32, score: i32) {
    let client = host.pool.client_at(slot);
    let mut record = client.borrow_mut();
    record.pers.connected = ConnectionState::Connected as i32;
    record.sess.session_team = team_code;
    record.ps.persistant.set(PersistentIndex::PersScore as usize, score);
    drop(record);
    host.pool.at(slot).borrow_mut().inuse = true;
}

#[test]
fn match_ranks_and_scores() {
    let (runtime, host) = match_fixture(GameType::GtFfa as i32);
    connect_client(&host, 0, Team::TeamFree as i32, 5);
    connect_client(&host, 1, Team::TeamFree as i32, 9);
    connect_client(&host, 2, Team::TeamSpectator as i32, 0);
    runtime.calculate_ranks();
    let state = host.state.borrow();
    assert_eq!(state.num_connected_clients, 3);
    assert_eq!(state.num_playing_clients, 2);
    assert_eq!(state.sorted_clients[0], 1);
    assert_eq!(state.sorted_clients[1], 0);
    drop(state);
    assert_eq!(
        host.pool
            .client_at(1)
            .borrow()
            .ps
            .persistant
            .get(PersistentIndex::PersRank as usize),
        0
    );
    assert_eq!(host.configstrings.borrow().get(&6), Some(&"9".to_string()));
    assert_eq!(host.configstrings.borrow().get(&7), Some(&"5".to_string()));
    assert!(!runtime.score_is_tied());
    host.pool
        .client_at(0)
        .borrow_mut()
        .ps
        .persistant
        .set(PersistentIndex::PersScore as usize, 9);
    runtime.calculate_ranks();
    assert!(runtime.score_is_tied());
}

#[test]
fn match_votes_and_cvars() {
    let (runtime, host) = match_fixture(GameType::GtFfa as i32);
    connect_client(&host, 0, Team::TeamFree as i32, 0);
    connect_client(&host, 1, Team::TeamFree as i32, 0);
    runtime.calculate_ranks();
    host.state.borrow_mut().vote.time = 1000;
    host.state.borrow_mut().vote.yes = 2;
    host.state.borrow_mut().time = 2000;
    runtime.check_vote();
    assert!(host.log.borrow().iter().any(|line| line.contains("Vote passed")));
    assert_eq!(host.state.borrow().vote.execute_time, 5000);
    host.state.borrow_mut().time = 6000;
    runtime.check_vote();
    assert!(host.log.borrow().iter().any(|line| line.starts_with("console")));
    host.settings.borrow_mut().password = "secret".to_string();
    host.settings.borrow_mut().password_modification_count = 1;
    runtime.check_cvars();
    assert_eq!(host.cvars.borrow().get("g_needpass"), Some(&"1".to_string()));
    runtime.check_cvars();
    assert_eq!(host.cvars.borrow().len(), 1);
}

#[test]
fn match_team_votes_and_leaders() {
    let (runtime, host) = match_fixture(GameType::GtTeam as i32);
    connect_client(&host, 0, Team::TeamRed as i32, 0);
    connect_client(&host, 1, Team::TeamRed as i32, 0);
    runtime.calculate_ranks();
    host.state.borrow_mut().team_votes[0].time = 100;
    host.state.borrow_mut().team_votes[0].string = "leader 1".to_string();
    host.state.borrow_mut().team_votes[0].yes = 2;
    host.state.borrow_mut().time = 200;
    runtime.check_team_vote(Team::TeamRed as i32);
    assert_eq!(host.pool.client_at(1).borrow().sess.team_leader, 1);
    host.pool.client_at(1).borrow_mut().sess.team_leader = 0;
    runtime.check_team_leader(Team::TeamRed as i32);
    assert_eq!(host.pool.client_at(0).borrow().sess.team_leader, 1);
    runtime.print_team(Team::TeamRed as i32, "hello");
    assert_eq!(
        host.log.borrow().iter().filter(|line| line.contains("hello")).count(),
        2
    );
}

#[test]
fn match_exit_rules_and_intermission() {
    let (runtime, host) = match_fixture(GameType::GtFfa as i32);
    connect_client(&host, 0, Team::TeamFree as i32, 3);
    connect_client(&host, 1, Team::TeamFree as i32, 1);
    host.pool.at(2).borrow_mut().inuse = true;
    host.pool
        .at(2)
        .borrow_mut()
        .set_classname(Some("info_player_intermission".to_string()));
    runtime.calculate_ranks();
    host.settings.borrow_mut().time_limit = 10;
    host.state.borrow_mut().time = 10 * 60_000 + 1;
    runtime.check_exit_rules();
    assert_ne!(host.state.borrow().intermission_queued, 0);
    host.state.borrow_mut().time += 2000;
    runtime.check_exit_rules();
    assert_ne!(host.state.borrow().intermission_time, 0);
    assert!(host.log.borrow().iter().any(|line| line.contains("Timelimit")));
    let saved = runtime.capture_save_state();
    runtime.restore_save_state(&saved);
    runtime.move_client_to_intermission(&host.pool.at(0));
    assert_eq!(host.pool.at(0).borrow().s.e_type, EntityType::EtGeneral as i32);
}

struct StubTeamHost {
    product: Product,
    pool: PoolRef,
    world: Rc<StubWorld>,
    game_type: Cell<i32>,
    time: Cell<i32>,
    team_scores: SharedSlots,
    sorted: RefCell<Vec<i32>>,
    location_head: RefCell<Option<EntityRef>>,
    obelisk: Cell<Option<ObeliskSettings>>,
    log: RefCell<Vec<String>>,
    configstrings: RefCell<HashMap<i32, String>>,
    scores: RefCell<Vec<(usize, i32)>>,
    ranks: Cell<i32>,
    respawned: RefCell<Vec<usize>>,
    pvs: Cell<bool>,
}

impl TeamHost for StubTeamHost {
    fn product(&self) -> Product {
        self.product
    }

    fn pool(&self) -> PoolRef {
        self.pool.clone()
    }

    fn world(&self) -> WorldRef {
        self.world.clone()
    }

    fn game_type(&self) -> i32 {
        self.game_type.get()
    }

    fn time(&self) -> i32 {
        self.time.get()
    }

    fn team_scores(&self) -> SharedSlots {
        self.team_scores.clone()
    }

    fn sorted_clients(&self) -> Vec<i32> {
        self.sorted.borrow().clone()
    }

    fn location_head(&self) -> Option<EntityRef> {
        self.location_head.borrow().clone()
    }

    fn obelisk_settings(&self) -> Option<ObeliskSettings> {
        self.obelisk.get()
    }

    fn send_server_command(&self, client_num: i32, text: &str) {
        self.log.borrow_mut().push(format!("server {client_num} {text}"));
    }

    fn set_configstring(&self, index: i32, text: &str) {
        self.configstrings.borrow_mut().insert(index, text.to_string());
    }

    fn warn(&self, text: &str) {
        self.log.borrow_mut().push(format!("warn {text}"));
    }

    fn add_score(&self, player: &EntityRef, _origin: Vec3, score: i32) {
        self.scores.borrow_mut().push((player.borrow().slot, score));
    }

    fn calculate_ranks(&self) {
        self.ranks.set(self.ranks.get() + 1);
    }

    fn respawn_item(&self, item: &EntityRef) {
        self.respawned.borrow_mut().push(item.borrow().slot);
    }

    fn in_pvs(&self, _first: Vec3, _second: Vec3) -> bool {
        self.pvs.get()
    }
}

fn team_fixture(product: Product, game_type: i32) -> (TeamRuntime, Rc<StubTeamHost>) {
    // Mid-game clock so freed-slot protection applies (map start is time 0).
    let time = Rc::new(Cell::new(3000));
    let log = Rc::new(RefCell::new(Vec::new()));
    let pool = test_pool(product, 4, time, log);
    for slot in 0..2 {
        pool.at(slot).borrow_mut().inuse = true;
        pool.client_at(slot).borrow_mut().pers.connected = ConnectionState::Connected as i32;
    }
    let host = Rc::new(StubTeamHost {
        product,
        pool,
        world: Rc::new(StubWorld::new()),
        game_type: Cell::new(game_type),
        time: Cell::new(3000),
        team_scores: Rc::new(SlotArray::new(4)),
        sorted: RefCell::new(vec![0, 1]),
        location_head: RefCell::new(None),
        obelisk: Cell::new(None),
        log: RefCell::new(Vec::new()),
        configstrings: RefCell::new(HashMap::new()),
        scores: RefCell::new(Vec::new()),
        ranks: Cell::new(0),
        respawned: RefCell::new(Vec::new()),
        pvs: Cell::new(true),
    });
    let runtime = TeamRuntime::new(host.clone());
    (runtime, host)
}

fn flag_base(host: &StubTeamHost, classname: &str) -> EntityRef {
    let entity = host.pool.spawn();
    entity.borrow_mut().set_classname(Some(classname.to_string()));
    entity.borrow_mut().item = Some(ItemDefinition {
        class_name: Some(classname.to_string()),
        pickup_name: None,
        quantity: 0,
        item_type: ItemType::ItTeam,
        tag: if classname.contains("red") {
            Powerup::PwRedflag as i32
        } else {
            Powerup::PwBlueflag as i32
        },
    });
    entity
}

#[test]
fn team_helpers_cover_codes() {
    assert_eq!(other_team(Team::TeamRed as i32), Team::TeamBlue as i32);
    assert_eq!(other_team(Team::TeamFree as i32), Team::TeamFree as i32);
    assert_eq!(team_name(Team::TeamSpectator as i32), "SPECTATOR");
    assert_eq!(other_team_name(Team::TeamBlue as i32), "RED");
    assert_eq!(team_color_string(Team::TeamRed as i32), "^1");
    assert_eq!(team_color_string(9), "^7");
    let (runtime, host) = team_fixture(Product::Baseq3, GameType::GtCtf as i32);
    host.pool.client_at(0).borrow_mut().sess.session_team = Team::TeamRed as i32;
    host.pool.client_at(1).borrow_mut().sess.session_team = Team::TeamRed as i32;
    assert!(on_same_team(GameType::GtCtf as i32, &host.pool.at(0), &host.pool.at(1)));
    assert!(!on_same_team(
        GameType::GtFfa as i32,
        &host.pool.at(0),
        &host.pool.at(1)
    ));
    runtime.print_message(None, "hi \"there\"\n");
    assert!(host.log.borrow().iter().any(|line| line.contains("hi 'there'")));
    spawn_team_point(&host.pool.at(0));
}

#[test]
fn team_flag_take_and_capture() {
    let (runtime, host) = team_fixture(Product::Baseq3, GameType::GtCtf as i32);
    runtime.init_game();
    host.pool.client_at(0).borrow_mut().sess.session_team = Team::TeamRed as i32;
    host.pool.client_at(0).borrow_mut().pers.netname = "red".to_string();
    host.pool.client_at(1).borrow_mut().sess.session_team = Team::TeamBlue as i32;
    host.pool.client_at(1).borrow_mut().pers.netname = "blue".to_string();
    let red_base = flag_base(&host, "team_CTF_redflag");
    let blue_base = flag_base(&host, "team_CTF_blueflag");
    assert_eq!(
        runtime.touch_enemy_flag(&blue_base, &host.pool.at(0), Team::TeamBlue as i32),
        -1
    );
    assert_ne!(
        host.pool
            .client_at(0)
            .borrow()
            .ps
            .powerups
            .get(Powerup::PwBlueflag as usize),
        0
    );
    assert_eq!(host.configstrings.borrow().get(&23), Some(&"01".to_string()));
    assert_eq!(
        runtime.touch_our_flag(&red_base, &host.pool.at(0), Team::TeamRed as i32),
        0
    );
    assert_eq!(host.team_scores.get(Team::TeamRed as usize), 1);
    assert_eq!(
        host.pool
            .client_at(0)
            .borrow()
            .ps
            .persistant
            .get(PersistentIndex::PersCaptures as usize),
        1
    );
    assert_eq!(host.ranks.get(), 1);
}

#[test]
fn team_flag_return_and_frag_bonus() {
    let (runtime, host) = team_fixture(Product::Baseq3, GameType::GtCtf as i32);
    host.pool.client_at(0).borrow_mut().sess.session_team = Team::TeamRed as i32;
    host.pool.client_at(0).borrow_mut().pers.netname = "red".to_string();
    host.pool.client_at(1).borrow_mut().sess.session_team = Team::TeamBlue as i32;
    host.pool.client_at(1).borrow_mut().pers.netname = "blue".to_string();
    let red_base = flag_base(&host, "team_CTF_redflag");
    flag_base(&host, "team_CTF_blueflag");
    let dropped = host.pool.spawn();
    dropped.borrow_mut().set_classname(Some("team_CTF_redflag".to_string()));
    dropped.borrow_mut().flags |= GameFlags::DROPPED_ITEM;
    dropped.borrow_mut().item = Some(ItemDefinition {
        class_name: Some("team_CTF_redflag".to_string()),
        pickup_name: None,
        quantity: 0,
        item_type: ItemType::ItTeam,
        tag: Powerup::PwRedflag as i32,
    });
    runtime.check_dropped_item(&dropped);
    runtime.return_flag(Team::TeamRed as i32);
    assert!(!dropped.borrow().inuse);
    assert!(host.respawned.borrow().contains(&red_base.borrow().slot));
    host.pool
        .client_at(1)
        .borrow_mut()
        .ps
        .powerups
        .set(Powerup::PwRedflag as usize, 99999);
    runtime.frag_bonuses(&host.pool.at(1), Some(&host.pool.at(0)));
    assert!(host
        .scores
        .borrow()
        .iter()
        .any(|(slot, score)| *slot == 0 && *score == 2));
    assert_eq!(host.pool.client_at(0).borrow().pers.team_state.frag_carrier, 1);
}

#[test]
fn team_locations_and_status() {
    let (runtime, host) = team_fixture(Product::Baseq3, GameType::GtCtf as i32);
    let marker = host.pool.spawn();
    marker.borrow_mut().count = 3;
    marker.borrow_mut().message = Some("Base".to_string());
    marker.borrow_mut().health = 7;
    host.location_head.borrow_mut().replace(marker);
    host.pool.client_at(0).borrow_mut().sess.session_team = Team::TeamRed as i32;
    assert_eq!(
        runtime.get_location_message(&host.pool.at(0), 64),
        Some("^3Base^7".to_string())
    );
    host.time.set(5000);
    runtime.check_team_status();
    assert_eq!(host.pool.client_at(0).borrow().pers.team_state.location, 7);
    let saved = runtime.capture_save_state();
    runtime.restore_save_state(&saved);
}

#[test]
fn team_obelisk_lifecycle() {
    let (runtime, host) = team_fixture(Product::Missionpack, GameType::GtObelisk as i32);
    host.obelisk.set(Some(ObeliskSettings {
        health: 100,
        regen_period_seconds: 1,
        regen_amount: 25,
        respawn_delay_seconds: 5,
    }));
    let marker = host.pool.spawn();
    marker.borrow_mut().set_classname(Some("team_redobelisk".to_string()));
    marker.borrow_mut().s.origin = vec3(0.0, 0.0, 10.0);
    host.world.trace.borrow_mut().end = vec3(0.0, 0.0, 0.0);
    runtime.spawn_team_obelisk(&marker, Team::TeamRed as i32);
    assert_eq!(marker.borrow().s.e_type, EntityType::EtTeam as i32);
    let obelisk = find_entity(&host.pool, None, EntityStringField::Classname, Some("noclass")).unwrap();
    assert_eq!(obelisk.borrow().health, 100);
    host.pool.client_at(0).borrow_mut().sess.session_team = Team::TeamRed as i32;
    assert!(runtime.check_obelisk_attack(&obelisk, &host.pool.at(0)));
    host.pool.client_at(1).borrow_mut().sess.session_team = Team::TeamBlue as i32;
    host.time.set(30_000);
    let before = host.pool.num_entities();
    assert!(!runtime.check_obelisk_attack(&obelisk, &host.pool.at(1)));
    // The attacked announcement spawns a broadcast temp entity.
    assert_eq!(host.pool.num_entities(), before + 1);
}

fn effects_fixture(product: Product) -> (PoolRef, Rc<StubCombat>, Rc<StubItems>, Rc<StubEffects>) {
    let time = Rc::new(Cell::new(5000));
    let log = Rc::new(RefCell::new(Vec::new()));
    let pool = test_pool(product, 2, time, log);
    let items = Rc::new(StubItems::new());
    let combat = Rc::new(StubCombat {
        product,
        time: Cell::new(5000),
        game_type: Cell::new(GameType::GtFfa as i32),
        intermission_queued: Cell::new(0),
        pool: pool.clone(),
        damage_calls: RefCell::new(Vec::new()),
    });
    let effects = Rc::new(StubEffects {
        combat: combat.clone() as CombatRef,
        items: items.clone(),
        intermission_time: Cell::new(0),
        smooth: Cell::new(false),
        fry: Cell::new(9),
        random: Cell::new(0),
        sounds: RefCell::new(Vec::new()),
        indexes: RefCell::new(Vec::new()),
        spectator_frames: RefCell::new(Vec::new()),
    });
    (pool, combat, items, effects)
}

#[test]
fn effect_feedback_and_timers() {
    let (pool, _combat, _items, effects) = effects_fixture(Product::Baseq3);
    let entity = pool.at(0);
    let client = pool.client_at(0);
    client.borrow_mut().damage_blood = 30;
    client.borrow_mut().damage_armor = 10;
    client.borrow_mut().damage_from = vec3(1.0, 0.0, 0.0);
    damage_feedback(effects.as_ref(), &entity);
    assert_eq!(client.borrow().ps.damage_count, 40);
    assert_eq!(client.borrow().damage_blood, 0);
    assert_eq!(client.borrow().ps.damage_event, 1);
    assert_eq!(entity.borrow().pain_debounce_time, 5700);

    let rule = q3_ammo_regeneration_rule(Weapon::WpRocketLauncher as i32);
    assert_eq!((rule.max, rule.increment, rule.time), (10, 1, 1750));
    assert_eq!(step_q3_ammo_regeneration(&rule, 5, 1000, 800), (50, Some(6)));
    assert_eq!(step_q3_ammo_regeneration(&rule, 10, 9999, 10), (0, None));

    entity.borrow_mut().health = 80;
    client.borrow_mut().ps.stats.set(base_max_health_slot(), 100);
    client.borrow_mut().ps.powerups.set(Powerup::PwRegen as usize, 99999);
    client_timer_actions(effects.as_ref(), &entity, 1000, None);
    assert_eq!(entity.borrow().health, 95);
    client.borrow_mut().ps.powerups.set(Powerup::PwRegen as usize, 0);
    entity.borrow_mut().health = 150;
    client_timer_actions(effects.as_ref(), &entity, 1000, None);
    assert_eq!(entity.borrow().health, 149);
}

#[test]
fn effect_speed_powerups_and_frames() {
    let (pool, _combat, items, effects) = effects_fixture(Product::Missionpack);
    let entity = pool.at(0);
    let client = pool.client_at(0);
    client.borrow_mut().ps.stats.set(2, 2);
    items.tags.borrow_mut().insert(2, Powerup::PwScout as i32);
    assert_eq!(client_speed_multiplier(items.as_ref(), &client.borrow().ps), 1.5);
    items.tags.borrow_mut().insert(2, Powerup::PwNone as i32);
    client.borrow_mut().ps.powerups.set(Powerup::PwHaste as usize, 99999);
    assert_eq!(client_speed_multiplier(items.as_ref(), &client.borrow().ps), 1.3);
    client.borrow_mut().ps.powerups.set(4, 100);
    update_q3_client_powerups(effects.as_ref(), &client);
    assert_eq!(client.borrow().ps.powerups.get(4), 0);
    client.borrow_mut().invulnerability_time = 99999;
    update_q3_client_powerups(effects.as_ref(), &client);
    assert_eq!(
        client.borrow().ps.powerups.get(Powerup::PwInvulnerability as usize),
        5000
    );

    client.borrow_mut().sess.session_team = Team::TeamSpectator as i32;
    client_end_frame(effects.as_ref(), &entity);
    assert_eq!(effects.spectator_frames.borrow().as_slice(), [0]);
    client.borrow_mut().sess.session_team = Team::TeamFree as i32;
    client.borrow_mut().pers.connected = ConnectionState::Connected as i32;
    entity.borrow_mut().health = 88;
    client_end_frame(effects.as_ref(), &entity);
    assert_eq!(client.borrow().ps.health(), 88);
    assert_eq!(entity.borrow().s.pos.trajectory_type, TrajectoryType::TrInterpolate);
}

struct StubWeapons {
    fires: RefCell<Vec<usize>>,
    kamikaze: RefCell<Vec<usize>>,
}

impl WeaponHost for StubWeapons {
    fn fire(&self, entity: &EntityRef) {
        self.fires.borrow_mut().push(entity.borrow().slot);
    }

    fn start_kamikaze(&self, entity: &EntityRef) {
        self.kamikaze.borrow_mut().push(entity.borrow().slot);
    }
}

struct StubDrops {
    pool: PoolRef,
    calls: RefCell<Vec<(usize, i32)>>,
}

impl DropHost for StubDrops {
    fn drop_item(&self, entity: &EntityRef, item: &ItemDefinition, angle: i32) -> EntityRef {
        self.calls.borrow_mut().push((entity.borrow().slot, angle));
        let dropped = self.pool.spawn();
        dropped.borrow_mut().item = Some(item.clone());
        dropped
    }
}

struct StubTeleport {
    calls: RefCell<Vec<(usize, Vec3, Vec3)>>,
}

impl TeleportHost for StubTeleport {
    fn teleport_player(&self, entity: &EntityRef, origin: Vec3, angles: Vec3) {
        self.calls.borrow_mut().push((entity.borrow().slot, origin, angles));
    }
}

struct StubSelector {
    pose: SpawnPose,
}

impl SpawnSelector for StubSelector {
    fn select_spawn_point(&self, _avoid: Vec3) -> SpawnPoint {
        SpawnPoint {
            origin: self.pose.origin,
            angles: self.pose.angles,
            entity: Rc::new(RefCell::new(GameEntity::new(0, ActorIdPlaceholder::actor()))),
        }
    }
}

struct ActorIdPlaceholder;

impl ActorIdPlaceholder {
    fn actor() -> ActorId {
        IdentityOwner::create("test").unwrap().actor(0, 0)
    }
}

#[test]
fn client_events_dispatch() {
    let (pool, combat, items, _) = effects_fixture(Product::Baseq3);
    let weapons = Rc::new(StubWeapons {
        fires: RefCell::new(Vec::new()),
        kamikaze: RefCell::new(Vec::new()),
    });
    let drops = Rc::new(StubDrops {
        pool: pool.clone(),
        calls: RefCell::new(Vec::new()),
    });
    let teleport = Rc::new(StubTeleport {
        calls: RefCell::new(Vec::new()),
    });
    let entity = pool.at(0);
    entity.borrow_mut().s.e_type = EntityType::EtPlayer as i32;
    let client = pool.client_at(0);
    client.borrow_mut().ps.event_sequence = 1;
    client.borrow_mut().ps.events.set(0, EntityEvent::EvFallFar as i32);
    let context = ClientEvents {
        world: Rc::new(StubWorld::new()),
        weapons: weapons.clone(),
        spawns: Rc::new(StubSelector {
            pose: SpawnPose {
                origin: vec3(7.0, 8.0, 9.0),
                angles: vec3(0.0, 45.0, 0.0),
            },
        }),
        drops: drops.clone(),
        items: items.clone(),
        teleport: teleport.clone(),
        dmflags: 0,
        primary_attack_allowed: None,
        product: Product::Baseq3,
        combat: combat.clone() as CombatRef,
        personal_portal: None,
    };
    client_events(&context, &entity, 0);
    assert_eq!(combat.damage_calls.borrow().as_slice(), [(0, 10, 19)]);
    assert_eq!(entity.borrow().pain_debounce_time, 5200);

    client.borrow_mut().ps.events.set(0, EntityEvent::EvFireWeapon as i32);
    client_events(&context, &entity, 0);
    assert_eq!(weapons.fires.borrow().as_slice(), [0]);

    client.borrow_mut().ps.events.set(0, EntityEvent::EvUseItem2 as i32);
    client.borrow_mut().ps.stats.set(base_max_health_slot(), 100);
    client_events(&context, &entity, 0);
    assert_eq!(entity.borrow().health, 125);

    items.by_powerup.borrow_mut().insert(
        Powerup::PwRedflag as i32,
        ItemDefinition {
            class_name: Some("team_CTF_redflag".to_string()),
            pickup_name: None,
            quantity: 0,
            item_type: ItemType::ItTeam,
            tag: Powerup::PwRedflag as i32,
        },
    );
    client.borrow_mut().ps.powerups.set(Powerup::PwRedflag as usize, 10_000);
    client.borrow_mut().ps.events.set(0, EntityEvent::EvUseItem1 as i32);
    client_events(&context, &entity, 0);
    assert_eq!(client.borrow().ps.powerups.get(Powerup::PwRedflag as usize), 0);
    assert_eq!(drops.calls.borrow().len(), 1);
    assert_eq!(teleport.calls.borrow().len(), 1);
    assert_eq!(teleport.calls.borrow()[0].1, vec3(7.0, 8.0, 9.0));
}

struct StubPolicyHost {
    pool: PoolRef,
    world: Rc<StubWorld>,
    time: Cell<i32>,
    inactivity: Cell<i32>,
    follow1: Cell<i32>,
    follow2: Cell<i32>,
    moves: RefCell<Vec<(usize, i32)>>,
    movement: RefCell<ClientMovementResult>,
    touched: RefCell<Vec<usize>>,
    cycles: RefCell<Vec<(usize, i32)>>,
    begins: RefCell<Vec<usize>>,
    drops: RefCell<Vec<(usize, String)>>,
    commands: RefCell<Vec<(i32, String)>>,
}

impl MovementHost for StubPolicyHost {
    fn move_client(
        &self,
        entity: &EntityRef,
        _command: &UserCommand,
        options: &ClientMovementOptions,
    ) -> ClientMovementResult {
        self.moves.borrow_mut().push((entity.borrow().slot, options.trace_mask));
        self.movement.borrow().clone()
    }
}

impl ClientPolicyHost for StubPolicyHost {
    fn pool(&self) -> PoolRef {
        self.pool.clone()
    }

    fn world(&self) -> WorldRef {
        self.world.clone()
    }

    fn time(&self) -> i32 {
        self.time.get()
    }

    fn inactivity_seconds(&self) -> i32 {
        self.inactivity.get()
    }

    fn follow1(&self) -> i32 {
        self.follow1.get()
    }

    fn follow2(&self) -> i32 {
        self.follow2.get()
    }

    fn touch_triggers(&self, entity: &EntityRef) {
        self.touched.borrow_mut().push(entity.borrow().slot);
    }

    fn follow_cycle(&self, entity: &EntityRef, direction: i32) {
        self.cycles.borrow_mut().push((entity.borrow().slot, direction));
    }

    fn client_begin(&self, client_num: usize) {
        self.begins.borrow_mut().push(client_num);
    }

    fn drop_client(&self, client_num: usize, reason: &str) {
        self.drops.borrow_mut().push((client_num, reason.to_string()));
    }

    fn send_server_command(&self, client_num: i32, text: &str) {
        self.commands.borrow_mut().push((client_num, text.to_string()));
    }
}

fn policy_fixture() -> (PoolRef, Rc<StubPolicyHost>) {
    let time = Rc::new(Cell::new(60_000));
    let log = Rc::new(RefCell::new(Vec::new()));
    let pool = test_pool(Product::Baseq3, 2, time, log);
    let host = Rc::new(StubPolicyHost {
        pool: pool.clone(),
        world: Rc::new(StubWorld::new()),
        time: Cell::new(60_000),
        inactivity: Cell::new(60),
        follow1: Cell::new(0),
        follow2: Cell::new(0),
        moves: RefCell::new(Vec::new()),
        movement: RefCell::new(ClientMovementResult {
            contacts: Vec::new(),
            bounds: Bounds {
                min: vec3(-15.0, -15.0, -24.0),
                max: vec3(15.0, 15.0, 32.0),
            },
            waterlevel: 0,
            watertype: 0,
            xyspeed: 0.0,
        }),
        touched: RefCell::new(Vec::new()),
        cycles: RefCell::new(Vec::new()),
        begins: RefCell::new(Vec::new()),
        drops: RefCell::new(Vec::new()),
        commands: RefCell::new(Vec::new()),
    });
    (pool, host)
}

#[test]
fn policy_spectator_moves_and_cycles() {
    let (pool, host) = policy_fixture();
    let entity = pool.at(0);
    pool.client_at(0).borrow_mut().sess.session_team = Team::TeamSpectator as i32;
    let command = UserCommand {
        buttons: CommandButtons::Attack as i32,
        server_time: 0,
        angles: vec3(0.0, 0.0, 0.0),
        weapon: Weapon::WpNone as i32,
        forwardmove: 0,
        rightmove: 0,
        upmove: 0,
    };
    spectator_think(host.as_ref(), &entity, &command);
    assert_eq!(pool.client_at(0).borrow().ps.pm_type, MoveType::PmSpectator as i32);
    assert_eq!(pool.client_at(0).borrow().ps.speed, 400);
    assert_eq!(host.moves.borrow().as_slice(), [(0, 1 | 0x10000)]);
    assert_eq!(host.touched.borrow().as_slice(), [0]);
    assert!(host.world.unlinks.borrow().contains(&0));
    assert_eq!(host.cycles.borrow().as_slice(), [(0, 1)]);

    pool.client_at(1).borrow_mut().pers.connected = ConnectionState::Connected as i32;
    pool.client_at(1).borrow_mut().sess.session_team = Team::TeamRed as i32;
    pool.client_at(1).borrow_mut().ps.origin = vec3(5.0, 6.0, 7.0);
    pool.client_at(0).borrow_mut().sess.spectator_state = SpectatorState::Follow as i32;
    pool.client_at(0).borrow_mut().sess.spectator_client = 1;
    spectator_client_end_frame(host.as_ref(), &entity);
    assert_eq!(pool.client_at(0).borrow().ps.origin, vec3(5.0, 6.0, 7.0));
    assert_ne!(pool.client_at(0).borrow().ps.pm_flags & MoveFlags::Follow as i32, 0);
}

#[test]
fn policy_inactivity_and_intermission() {
    let (pool, host) = policy_fixture();
    let client = pool.client_at(0);
    client.borrow_mut().inactivity_time = 1000;
    assert!(!client_inactivity_timer(host.as_ref(), &client));
    assert_eq!(host.drops.borrow().len(), 1);
    client.borrow_mut().inactivity_time = 65_000;
    client.borrow_mut().inactivity_warning = false;
    assert!(client_inactivity_timer(host.as_ref(), &client));
    assert!(client.borrow().inactivity_warning);
    assert!(host
        .commands
        .borrow()
        .iter()
        .any(|(_, text)| text.contains("inactivity drop")));
    host.inactivity.set(0);
    assert!(client_inactivity_timer(host.as_ref(), &client));
    assert_eq!(client.borrow().inactivity_time, 120_000);

    client.borrow_mut().pers.cmd.buttons = CommandButtons::Attack as i32;
    client.borrow_mut().buttons = 0;
    client_intermission_think(&client);
    assert!(client.borrow().ready_to_exit);
}

struct StubTouches {
    native: RefCell<HashMap<ActorId, EntityRef>>,
    triggers: RefCell<Vec<ActorId>>,
    calls: RefCell<Vec<(ActorId, ActorId)>>,
}

impl TouchAccess for StubTouches {
    fn native(&self, actor: &ActorId) -> Option<EntityRef> {
        self.native.borrow().get(actor).cloned()
    }

    fn is_trigger(&self, actor: &ActorId) -> bool {
        self.triggers.borrow().contains(actor)
    }

    fn touch(&self, this: &ActorId, other: &ActorId) {
        self.calls.borrow_mut().push((this.clone(), other.clone()));
    }
}

struct StubThinkHost {
    pool: PoolRef,
    world: Rc<StubWorld>,
    touches: Rc<StubTouches>,
    effects: Rc<StubEffects>,
    items: Rc<StubItems>,
    frame: Cell<ClientThinkFrame>,
    settings: RefCell<ClientThinkSettings>,
    moves: RefCell<Vec<usize>>,
    movement: RefCell<ClientMovementResult>,
    pmove_msec: Cell<i32>,
    intermissions: RefCell<Vec<usize>>,
    spectators: RefCell<Vec<usize>>,
    inactivity: Cell<bool>,
    hooks: RefCell<Vec<usize>>,
    gauntlet: Cell<bool>,
    events: RefCell<Vec<(usize, i32)>>,
    respawns: RefCell<Vec<usize>>,
    console: RefCell<Vec<String>>,
    door_trigger: Cell<bool>,
    aas: RefCell<Vec<Vec3>>,
}

fn think_settings() -> ClientThinkSettings {
    ClientThinkSettings {
        debug_move: 0,
        synchronous_clients: false,
        pmove_fixed: false,
        pmove_msec: 8,
        gravity: 800.0,
        speed: 320.0,
        dmflags: 0,
        smooth_clients: false,
        force_respawn_seconds: 0,
        single_player: false,
    }
}

fn movement_result() -> ClientMovementResult {
    ClientMovementResult {
        contacts: Vec::new(),
        bounds: Bounds {
            min: vec3(-15.0, -15.0, -24.0),
            max: vec3(15.0, 15.0, 32.0),
        },
        waterlevel: 0,
        watertype: 0,
        xyspeed: 0.0,
    }
}

impl MovementHost for StubThinkHost {
    fn move_client(
        &self,
        entity: &EntityRef,
        _command: &UserCommand,
        _options: &ClientMovementOptions,
    ) -> ClientMovementResult {
        self.moves.borrow_mut().push(entity.borrow().slot);
        self.movement.borrow().clone()
    }
}

impl ClientThinkHost for StubThinkHost {
    fn pool(&self) -> PoolRef {
        self.pool.clone()
    }

    fn world(&self) -> WorldRef {
        self.world.clone()
    }

    fn touches(&self) -> Rc<dyn TouchAccess> {
        self.touches.clone()
    }

    fn effects(&self) -> EffectsCoreRef {
        self.effects.clone()
    }

    fn items(&self) -> Rc<dyn ItemHost> {
        self.items.clone()
    }

    fn timer_ownership(&self, _actor: &ActorId) -> Option<ClientTimerOwnership> {
        None
    }

    fn speed_multiplier(&self, _actor: &ActorId) -> Option<f32> {
        None
    }

    fn frame(&self) -> ClientThinkFrame {
        self.frame.get()
    }

    fn settings(&self) -> ClientThinkSettings {
        *self.settings.borrow()
    }

    fn set_pmove_msec(&self, ms: i32) {
        self.pmove_msec.set(ms);
    }

    fn intermission_think(&self, client: &ClientRef) {
        self.intermissions
            .borrow_mut()
            .push(self.pool.client_index(client).unwrap());
    }

    fn spectator_think(&self, entity: &EntityRef, _command: &UserCommand) {
        self.spectators.borrow_mut().push(entity.borrow().slot);
    }

    fn check_inactivity(&self, _client: &ClientRef) -> bool {
        self.inactivity.get()
    }

    fn free_hook(&self, hook: &EntityRef) {
        self.hooks.borrow_mut().push(hook.borrow().slot);
    }

    fn check_gauntlet_attack(&self, _entity: &EntityRef) -> bool {
        self.gauntlet.get()
    }

    fn client_events(&self, entity: &EntityRef, old_sequence: i32) {
        self.events.borrow_mut().push((entity.borrow().slot, old_sequence));
    }

    fn respawn(&self, entity: &EntityRef) {
        self.respawns.borrow_mut().push(entity.borrow().slot);
    }

    fn append_console_command(&self, command: &str) {
        self.console.borrow_mut().push(command.to_string());
    }

    fn is_door_trigger(&self, _entity: &EntityRef) -> bool {
        self.door_trigger.get()
    }

    fn bot_test_aas(&self, origin: Vec3) {
        self.aas.borrow_mut().push(origin);
    }
}

fn think_fixture(product: Product) -> (ClientThinkRuntime, Rc<StubThinkHost>) {
    let (pool, combat, items, effects) = effects_fixture(product);
    let host = Rc::new(StubThinkHost {
        pool: pool.clone(),
        world: Rc::new(StubWorld::new()),
        touches: Rc::new(StubTouches {
            native: RefCell::new(HashMap::new()),
            triggers: RefCell::new(Vec::new()),
            calls: RefCell::new(Vec::new()),
        }),
        effects,
        items,
        frame: Cell::new(ClientThinkFrame {
            time: 1000,
            intermission_time: 0,
            intermission_queued: 0,
        }),
        settings: RefCell::new(think_settings()),
        moves: RefCell::new(Vec::new()),
        movement: RefCell::new(movement_result()),
        pmove_msec: Cell::new(8),
        intermissions: RefCell::new(Vec::new()),
        spectators: RefCell::new(Vec::new()),
        inactivity: Cell::new(true),
        hooks: RefCell::new(Vec::new()),
        gauntlet: Cell::new(false),
        events: RefCell::new(Vec::new()),
        respawns: RefCell::new(Vec::new()),
        console: RefCell::new(Vec::new()),
        door_trigger: Cell::new(false),
        aas: RefCell::new(Vec::new()),
    });
    let _ = combat;
    (ClientThinkRuntime::new(host.clone()), host)
}

#[test]
fn think_runs_moves_and_branches() {
    let (runtime, host) = think_fixture(Product::Baseq3);
    let entity = host.pool.at(0);
    let client = host.pool.client_at(0);
    client.borrow_mut().pers.connected = ConnectionState::Connected as i32;
    client.borrow_mut().ps.set_health(100);
    client.borrow_mut().pers.cmd.server_time = 100;
    runtime.client_think_real(&entity);
    assert_eq!(host.moves.borrow().as_slice(), [0]);
    assert_eq!(client.borrow().ps.pm_type, MoveType::PmNormal as i32);
    assert_eq!(client.borrow().ps.gravity, 800);
    assert_eq!(entity.borrow().r.mins, vec3(-15.0, -15.0, -24.0));
    assert_eq!(host.events.borrow().as_slice(), [(0, 0)]);
    assert!(host.world.links.borrow().contains(&0));

    client.borrow_mut().ps.set_health(0);
    client.borrow_mut().respawn_time = 500;
    runtime.client_think_real(&entity);
    assert_eq!(client.borrow().ps.pm_type, MoveType::PmDead as i32);

    client.borrow_mut().sess.session_team = Team::TeamSpectator as i32;
    client.borrow_mut().sess.spectator_state = SpectatorState::Free as i32;
    runtime.client_think_real(&entity);
    assert_eq!(host.spectators.borrow().as_slice(), [0]);

    client.borrow_mut().sess.session_team = Team::TeamFree as i32;
    host.frame.set(ClientThinkFrame {
        time: 1000,
        intermission_time: 900,
        intermission_queued: 0,
    });
    runtime.client_think_real(&entity);
    assert_eq!(host.intermissions.borrow().as_slice(), [0]);
}

#[test]
fn think_triggers_and_invulnerability() {
    let (runtime, host) = think_fixture(Product::Missionpack);
    let entity = host.pool.at(0);
    let client = host.pool.client_at(0);
    client.borrow_mut().ps.set_health(100);
    let item = host.pool.spawn();
    item.borrow_mut().s.e_type = EntityType::EtItem as i32;
    item.borrow_mut().r.contents = 0x40000000;
    item.borrow_mut().touch = Some(Rc::new(|_, _, _| {}));
    host.world.area.borrow_mut().push(item.borrow().actor.clone());
    host.touches
        .native
        .borrow_mut()
        .insert(item.borrow().actor.clone(), item.clone());
    runtime.touch_triggers(&entity);
    assert_eq!(host.touches.calls.borrow().len(), 1);

    client
        .borrow_mut()
        .ps
        .powerups
        .set(Powerup::PwInvulnerability as usize, 99999);
    expand_q3_invulnerability(&host.pool, host.world.as_ref(), &entity);
    assert_ne!(client.borrow().ps.pm_flags & MoveFlags::InvulExpand as i32, 0);
    assert_eq!(host.world.links.borrow().len(), 2);
}

struct StubTargets {
    calls: RefCell<Vec<(Option<usize>, bool)>>,
}

impl TargetsHost for StubTargets {
    fn use_targets(&self, used: Option<&EntityRef>, activator: Option<&DamageParticipant>) {
        self.calls
            .borrow_mut()
            .push((used.map(|entity| entity.borrow().slot), activator.is_some()));
    }
}

struct StubSpawnHost {
    pool: PoolRef,
    world: Rc<StubWorld>,
    is_player: Cell<bool>,
    random: GameRandom,
    think: Rc<ClientThinkRuntime>,
    frame: Cell<ClientSpawnFrame>,
    command: RefCell<UserCommand>,
    handicap: RefCell<String>,
    intermission: Cell<SpawnPose>,
    moved: RefCell<Vec<usize>>,
    killed: RefCell<Vec<usize>>,
    player_die_cb: DieCallback,
    body_die_cb: DieCallback,
    selected: Cell<bool>,
    selected_calls: RefCell<Vec<usize>>,
    effects: EffectsRef,
    targets: Rc<StubTargets>,
}

impl ClientSpawnHost for StubSpawnHost {
    fn pool(&self) -> PoolRef {
        self.pool.clone()
    }

    fn world(&self) -> WorldRef {
        self.world.clone()
    }

    fn is_player(&self, _actor: &ActorId) -> bool {
        self.is_player.get()
    }

    fn random(&self) -> &GameRandom {
        &self.random
    }

    fn think_runtime(&self) -> Rc<ClientThinkRuntime> {
        self.think.clone()
    }

    fn frame(&self) -> ClientSpawnFrame {
        self.frame.get()
    }

    fn user_command(&self, _client_num: usize) -> UserCommand {
        *self.command.borrow()
    }

    fn handicap(&self, _client_num: usize) -> String {
        self.handicap.borrow().clone()
    }

    fn find_intermission_point(&self) -> SpawnPose {
        self.intermission.get()
    }

    fn move_to_intermission(&self, entity: &EntityRef) {
        self.moved.borrow_mut().push(entity.borrow().slot);
    }

    fn kill_box(&self, entity: &EntityRef) {
        self.killed.borrow_mut().push(entity.borrow().slot);
    }

    fn player_die(&self) -> DieCallback {
        self.player_die_cb.clone()
    }

    fn body_die(&self) -> DieCallback {
        self.body_die_cb.clone()
    }

    fn has_selected_player(&self) -> bool {
        self.selected.get()
    }

    fn selected_player(&self, entity: &EntityRef, _pose: &SpawnPose) {
        self.selected_calls.borrow_mut().push(entity.borrow().slot);
    }

    fn effects(&self) -> EffectsRef {
        self.effects.clone()
    }

    fn targets(&self) -> Rc<dyn TargetsHost> {
        self.targets.clone()
    }
}

fn spawn_fixture() -> (ClientSpawnRuntime, Rc<StubSpawnHost>, Rc<StubThinkHost>) {
    let (pool, _combat, items, effects) = effects_fixture(Product::Baseq3);
    let world = Rc::new(StubWorld::new());
    let think_host = Rc::new(StubThinkHost {
        pool: pool.clone(),
        world: world.clone(),
        touches: Rc::new(StubTouches {
            native: RefCell::new(HashMap::new()),
            triggers: RefCell::new(Vec::new()),
            calls: RefCell::new(Vec::new()),
        }),
        effects: effects.clone(),
        items,
        frame: Cell::new(ClientThinkFrame {
            time: 1000,
            intermission_time: 0,
            intermission_queued: 0,
        }),
        settings: RefCell::new(think_settings()),
        moves: RefCell::new(Vec::new()),
        movement: RefCell::new(movement_result()),
        pmove_msec: Cell::new(8),
        intermissions: RefCell::new(Vec::new()),
        spectators: RefCell::new(Vec::new()),
        inactivity: Cell::new(true),
        hooks: RefCell::new(Vec::new()),
        gauntlet: Cell::new(false),
        events: RefCell::new(Vec::new()),
        respawns: RefCell::new(Vec::new()),
        console: RefCell::new(Vec::new()),
        door_trigger: Cell::new(false),
        aas: RefCell::new(Vec::new()),
    });
    let think = Rc::new(ClientThinkRuntime::new(think_host.clone()));
    let host = Rc::new(StubSpawnHost {
        pool: pool.clone(),
        world,
        is_player: Cell::new(false),
        random: GameRandom::new(0),
        think,
        frame: Cell::new(ClientSpawnFrame {
            time: 1000,
            game_type: GameType::GtFfa as i32,
            inactivity_seconds: 0,
            intermission_time: 0,
        }),
        command: RefCell::new(UserCommand {
            server_time: 0,
            angles: vec3(0.0, 0.0, 0.0),
            buttons: 0,
            weapon: Weapon::WpNone as i32,
            forwardmove: 0,
            rightmove: 0,
            upmove: 0,
        }),
        handicap: RefCell::new(String::new()),
        intermission: Cell::new(SpawnPose {
            origin: vec3(0.0, 0.0, 0.0),
            angles: vec3(0.0, 0.0, 0.0),
        }),
        moved: RefCell::new(Vec::new()),
        killed: RefCell::new(Vec::new()),
        player_die_cb: Rc::new(|_, _, _, _, _| {}),
        body_die_cb: Rc::new(|_, _, _, _, _| {}),
        selected: Cell::new(false),
        selected_calls: RefCell::new(Vec::new()),
        effects: effects.clone() as EffectsRef,
        targets: Rc::new(StubTargets {
            calls: RefCell::new(Vec::new()),
        }),
    });
    let runtime = ClientSpawnRuntime::new(host.clone(), ClientSpawnState::default());
    (runtime, host, think_host)
}

fn deathmatch_point(host: &StubSpawnHost, origin: Vec3) -> EntityRef {
    let entity = host.pool.spawn();
    entity
        .borrow_mut()
        .set_classname(Some("info_player_deathmatch".to_string()));
    entity.borrow_mut().s.origin = origin;
    entity
}

#[test]
fn spawn_view_angle_and_selection() {
    let (runtime, host, _) = spawn_fixture();
    let entity = host.pool.at(0);
    set_client_view_angle(&entity, vec3(0.0, 90.0, 0.0));
    assert_eq!(host.pool.client_at(0).borrow().ps.delta_angles.y, 16384.0);
    assert_eq!(host.pool.client_at(0).borrow().ps.viewangles, vec3(0.0, 90.0, 0.0));

    let near = deathmatch_point(&host, vec3(0.0, 0.0, 0.0));
    deathmatch_point(&host, vec3(100.0, 0.0, 0.0));
    let far = deathmatch_point(&host, vec3(200.0, 0.0, 0.0));
    host.is_player.set(true);
    host.world.area.borrow_mut().push(near.borrow().actor.clone());
    assert!(runtime.spot_would_telefrag(&near));
    host.world.area.borrow_mut().clear();
    assert!(!runtime.spot_would_telefrag(&near));
    let selected = runtime.select_spawn_point(vec3(0.0, 0.0, 0.0));
    assert_eq!(selected.origin.x, 200.0);
    assert_eq!(selected.origin.z, 9.0);
    assert!(Rc::ptr_eq(&selected.entity, &far));
    let nearest = runtime
        .select_nearest_deathmatch_spawn_point(vec3(90.0, 0.0, 0.0))
        .unwrap();
    assert_eq!(nearest.borrow().s.origin.x, 100.0);
}

#[test]
fn spawn_body_queue_rotates() {
    let (runtime, host, _) = spawn_fixture();
    runtime.init_body_queue();
    let entity = host.pool.at(0);
    entity.borrow_mut().inuse = true;
    entity.borrow_mut().health = 100;
    let first = runtime.copy_to_body_queue(&entity).unwrap();
    assert_eq!(first.borrow().s.e_flags, 1);
    assert_eq!(first.borrow().s.number, first.borrow().slot as i32);
    let second = runtime.copy_to_body_queue(&entity).unwrap();
    assert!(!Rc::ptr_eq(&first, &second));
    let saved = runtime.capture_save_state();
    runtime.restore_save_state(&saved);
}

#[test]
fn spawn_client_runs_full_path() {
    let (runtime, host, think) = spawn_fixture();
    deathmatch_point(&host, vec3(10.0, 20.0, 30.0));
    let entity = host.pool.at(0);
    entity.borrow_mut().inuse = true;
    host.pool.client_at(0).borrow_mut().pers.connected = ConnectionState::Connected as i32;
    runtime.client_spawn(&entity);
    let client = host.pool.client_at(0);
    assert_eq!(client.borrow().pers.team_state.state, TeamState::Active as i32);
    assert_eq!(client.borrow().ps.weapon, Weapon::WpMachinegun as i32);
    assert_eq!(entity.borrow().health, 125);
    assert_ne!(client.borrow().ps.pm_flags & MoveFlags::Respawned as i32, 0);
    assert_eq!(client.borrow().ps.command_time, 900);
    assert_eq!(client.borrow().last_cmd_time, 1000);
    assert_eq!(host.killed.borrow().as_slice(), [0]);
    assert!(!host.targets.calls.borrow().is_empty());
    assert!(think.world.links.borrow().contains(&0));
}

struct StubImports {
    commands: RefCell<Vec<(i32, String)>>,
    configstrings: RefCell<HashMap<i32, String>>,
    console: RefCell<Vec<String>>,
    cvars: RefCell<HashMap<String, String>>,
    userinfo: RefCell<HashMap<usize, String>>,
    logs: RefCell<Vec<String>>,
    prints: RefCell<Vec<String>>,
}

impl CommandImports for StubImports {
    fn send_server_command(&self, client_num: i32, text: &str) {
        self.commands.borrow_mut().push((client_num, text.to_string()));
    }

    fn set_configstring(&self, index: i32, text: &str) {
        self.configstrings.borrow_mut().insert(index, text.to_string());
    }

    fn append_console_command(&self, text: &str) {
        self.console.borrow_mut().push(text.to_string());
    }

    fn get_cvar(&self, name: &str) -> String {
        self.cvars.borrow().get(name).cloned().unwrap_or_default()
    }

    fn get_userinfo(&self, client_num: usize) -> String {
        self.userinfo.borrow().get(&client_num).cloned().unwrap_or_default()
    }

    fn set_userinfo(&self, client_num: usize, text: &str) {
        self.userinfo.borrow_mut().insert(client_num, text.to_string());
    }

    fn log(&self, text: &str) {
        self.logs.borrow_mut().push(text.to_string());
    }

    fn print(&self, text: &str) {
        self.prints.borrow_mut().push(text.to_string());
    }
}

struct StubDeath {
    dies: RefCell<Vec<(usize, i32, i32)>>,
    tossed: RefCell<Vec<usize>>,
    tossed_pp: RefCell<Vec<usize>>,
    tossed_cubes: RefCell<Vec<usize>>,
}

impl DeathHost for StubDeath {
    fn player_die(
        &self,
        target: &EntityRef,
        _inflictor: Option<&DamageParticipant>,
        _attacker: Option<&DamageParticipant>,
        damage: i32,
        method: i32,
    ) {
        self.dies.borrow_mut().push((target.borrow().slot, damage, method));
    }

    fn toss_client_items(&self, entity: &EntityRef) {
        self.tossed.borrow_mut().push(entity.borrow().slot);
    }

    fn toss_client_persistant_powerups(&self, entity: &EntityRef) {
        self.tossed_pp.borrow_mut().push(entity.borrow().slot);
    }

    fn toss_client_cubes(&self, entity: &EntityRef) {
        self.tossed_cubes.borrow_mut().push(entity.borrow().slot);
    }
}

struct StubCommandHost {
    pool: PoolRef,
    match_state: MatchStateRef,
    team_scores: SharedSlots,
    settings: RefCell<CommandSettings>,
    imports: Rc<StubImports>,
    locations: RefCell<HashMap<usize, String>>,
    death: Rc<StubDeath>,
    bodies: RefCell<Vec<usize>>,
    begins: RefCell<Vec<usize>>,
    userinfos: RefCell<Vec<usize>>,
    intermission: Cell<bool>,
    leaders: RefCell<Vec<(i32, usize)>>,
    checked_leaders: RefCell<Vec<i32>>,
    items: Rc<StubItems>,
    teleport: Rc<StubTeleport>,
}

impl GameCommandHost for StubCommandHost {
    fn pool(&self) -> PoolRef {
        self.pool.clone()
    }

    fn match_state(&self) -> &MatchStateRef {
        &self.match_state
    }

    fn team_scores(&self) -> SharedSlots {
        self.team_scores.clone()
    }

    fn settings(&self) -> CommandSettings {
        self.settings.borrow().clone()
    }

    fn imports(&self) -> Rc<dyn CommandImports> {
        self.imports.clone()
    }

    fn team_location_message(&self, entity: &EntityRef, _capacity: usize) -> Option<String> {
        self.locations.borrow().get(&entity.borrow().slot).cloned()
    }

    fn death(&self) -> Rc<dyn DeathHost> {
        self.death.clone()
    }

    fn copy_to_body_queue(&self, entity: &EntityRef) -> Option<EntityRef> {
        self.bodies.borrow_mut().push(entity.borrow().slot);
        None
    }

    fn admission_begin(&self, client_num: usize) {
        self.begins.borrow_mut().push(client_num);
    }

    fn admission_userinfo_changed(&self, client_num: usize) {
        self.userinfos.borrow_mut().push(client_num);
    }

    fn match_begin_intermission(&self) {
        self.intermission.set(true);
    }

    fn match_set_leader(&self, team_code: i32, client_num: usize) {
        self.leaders.borrow_mut().push((team_code, client_num));
    }

    fn match_check_team_leader(&self, team_code: i32) {
        self.checked_leaders.borrow_mut().push(team_code);
    }

    fn items(&self) -> Rc<dyn ItemHost> {
        self.items.clone()
    }

    fn teleport(&self) -> Rc<dyn TeleportHost> {
        self.teleport.clone()
    }
}

fn command_fixture() -> (GameCommandRuntime, Rc<StubCommandHost>) {
    let time = Rc::new(Cell::new(0));
    let log = Rc::new(RefCell::new(Vec::new()));
    let pool = test_pool(Product::Baseq3, 4, time, log);
    let host = Rc::new(StubCommandHost {
        pool,
        match_state: Rc::new(RefCell::new(MatchState::default())),
        team_scores: Rc::new(SlotArray::new(4)),
        settings: RefCell::new(CommandSettings {
            game_type: GameType::GtFfa as i32,
            cheats: true,
            team_force_balance: false,
            max_game_clients: 0,
            dedicated: false,
            allow_vote: true,
        }),
        imports: Rc::new(StubImports {
            commands: RefCell::new(Vec::new()),
            configstrings: RefCell::new(HashMap::new()),
            console: RefCell::new(Vec::new()),
            cvars: RefCell::new(HashMap::new()),
            userinfo: RefCell::new(HashMap::new()),
            logs: RefCell::new(Vec::new()),
            prints: RefCell::new(Vec::new()),
        }),
        locations: RefCell::new(HashMap::new()),
        death: Rc::new(StubDeath {
            dies: RefCell::new(Vec::new()),
            tossed: RefCell::new(Vec::new()),
            tossed_pp: RefCell::new(Vec::new()),
            tossed_cubes: RefCell::new(Vec::new()),
        }),
        bodies: RefCell::new(Vec::new()),
        begins: RefCell::new(Vec::new()),
        userinfos: RefCell::new(Vec::new()),
        intermission: Cell::new(false),
        leaders: RefCell::new(Vec::new()),
        checked_leaders: RefCell::new(Vec::new()),
        items: Rc::new(StubItems::new()),
        teleport: Rc::new(StubTeleport {
            calls: RefCell::new(Vec::new()),
        }),
    });
    let runtime = GameCommandRuntime::new(host.clone());
    (runtime, host)
}

#[test]
fn command_argument_helpers() {
    let args = CommandArguments::new(&argv(&["say", "hello", "world"]));
    assert_eq!(args.concat(1), "hello world");
    assert_eq!(args.at(9, 64), "");
    assert_eq!(concat_command_args(&argv(&["say", "a", "b"]), 1), "a b");
    assert_eq!(sanitize("A\x1b[B"), "ab");
    assert_eq!(clean_name("^1Bob^^"), "bob^^");
}

#[test]
fn command_dispatch_score_team_vote_kill() {
    let (runtime, host) = command_fixture();
    host.pool.at(0).borrow_mut().inuse = true;
    host.pool.client_at(0).borrow_mut().pers.connected = ConnectionState::Connected as i32;
    host.pool.client_at(0).borrow_mut().pers.netname = "bob".to_string();
    host.match_state.borrow_mut().num_connected_clients = 1;
    host.match_state.borrow_mut().sorted_clients[0] = 0;
    runtime.dispatch(0, &argv(&["score"]));
    assert!(host
        .imports
        .commands
        .borrow()
        .iter()
        .any(|(_, text)| text.starts_with("scores 1 0 0")));
    runtime.dispatch(0, &argv(&["frobnicate"]));
    assert!(host
        .imports
        .commands
        .borrow()
        .iter()
        .any(|(_, text)| text.contains("unknown cmd frobnicate")));

    host.pool.at(0).borrow_mut().health = 100;
    runtime.dispatch(0, &argv(&["kill"]));
    assert_eq!(host.pool.at(0).borrow().health, -999);
    assert_eq!(host.death.dies.borrow().as_slice(), [(0, 100_000, 20)]);

    host.match_state.borrow_mut().time = 5000;
    runtime.dispatch(0, &argv(&["callvote", "map", "q3dm1"]));
    assert_eq!(host.match_state.borrow().vote.time, 5000);
    assert_eq!(host.match_state.borrow().vote.string, "map q3dm1");
    assert_eq!(host.imports.configstrings.borrow().get(&10), Some(&"1".to_string()));
    host.pool.at(1).borrow_mut().inuse = true;
    host.pool.client_at(1).borrow_mut().pers.connected = ConnectionState::Connected as i32;
    runtime.dispatch(1, &argv(&["vote", "yes"]));
    assert_eq!(host.match_state.borrow().vote.yes, 2);
    runtime.dispatch(1, &argv(&["vote", "no"]));
    assert!(host
        .imports
        .commands
        .borrow()
        .iter()
        .any(|(_, text)| text.contains("already cast")));
}

#[test]
fn command_set_team_give_say_task() {
    let (runtime, host) = command_fixture();
    host.pool.at(0).borrow_mut().inuse = true;
    host.pool.at(0).borrow_mut().health = 100;
    host.pool.client_at(0).borrow_mut().pers.connected = ConnectionState::Connected as i32;
    runtime.set_team(&host.pool.at(0), "spectator");
    assert_eq!(
        host.pool.client_at(0).borrow().sess.session_team,
        Team::TeamSpectator as i32
    );
    assert_eq!(host.death.dies.borrow().len(), 1);
    assert_eq!(host.begins.borrow().as_slice(), [0]);
    assert_eq!(host.userinfos.borrow().as_slice(), [0]);

    host.pool.at(0).borrow_mut().health = 100;
    runtime.dispatch(0, &argv(&["give", "all"]));
    let StatSchema::Base(base) = stat_schema(Product::Baseq3) else {
        panic!("baseq3 must use the base schema");
    };
    assert_eq!(
        host.pool.client_at(0).borrow().ps.stats.get(base.weapons as usize),
        1022
    );
    assert_eq!(host.pool.client_at(0).borrow().ps.ammo.get(2), 999);
    assert_eq!(host.pool.client_at(0).borrow().ps.stats.get(base.armor as usize), 200);

    host.settings.borrow_mut().dedicated = true;
    runtime.dispatch(0, &argv(&["say", "hi"]));
    assert!(host.imports.prints.borrow().iter().any(|line| line.contains("hi")));

    runtime.dispatch(0, &argv(&["teamtask", "3"]));
    assert!(host
        .imports
        .userinfo
        .borrow()
        .get(&0)
        .map(|info| info.contains("teamtask"))
        .unwrap_or(false));
    assert!(host.userinfos.borrow().contains(&0));
}

struct StubAdmissionCommands {
    teams: RefCell<Vec<(usize, i32)>>,
    stops: RefCell<Vec<usize>>,
}

impl ClientAdmissionCommands for StubAdmissionCommands {
    fn broadcast_team_change(&self, client_num: usize, old_team: i32) {
        self.teams.borrow_mut().push((client_num, old_team));
    }

    fn stop_following(&self, entity: &EntityRef) {
        self.stops.borrow_mut().push(entity.borrow().slot);
    }
}

struct StubAdmissionHost {
    product: Product,
    pool: PoolRef,
    team_scores: SharedSlots,
    world: Rc<StubWorld>,
    match_state: MatchStateRef,
    new_session: Cell<bool>,
    session: Rc<GameSessionManager>,
    spawns: RefCell<Vec<usize>>,
    death: Rc<StubDeath>,
    ranks: Cell<i32>,
    commands: Rc<StubAdmissionCommands>,
    settings: RefCell<ClientAdmissionSettings>,
    userinfo: RefCell<HashMap<usize, String>>,
    configstrings: RefCell<HashMap<i32, String>>,
    commands_log: RefCell<Vec<(i32, String)>>,
    logs: RefCell<Vec<String>>,
    banned: Cell<bool>,
}

impl ClientAdmissionHost for StubAdmissionHost {
    fn product(&self) -> Product {
        self.product
    }

    fn pool(&self) -> PoolRef {
        self.pool.clone()
    }

    fn team_scores(&self) -> SharedSlots {
        self.team_scores.clone()
    }

    fn world(&self) -> WorldRef {
        self.world.clone()
    }

    fn match_state(&self) -> MatchStateRef {
        self.match_state.clone()
    }

    fn new_session(&self) -> bool {
        self.new_session.get()
    }

    fn session(&self) -> Rc<GameSessionManager> {
        self.session.clone()
    }

    fn spawn_client(&self, entity: &EntityRef) {
        self.spawns.borrow_mut().push(entity.borrow().slot);
    }

    fn death(&self) -> Rc<dyn DeathHost> {
        self.death.clone()
    }

    fn calculate_ranks(&self) {
        self.ranks.set(self.ranks.get() + 1);
    }

    fn commands(&self) -> Rc<dyn ClientAdmissionCommands> {
        self.commands.clone()
    }

    fn bots(&self) -> ClientBotServices<'_> {
        ClientBotServices::Available {
            remove_queued_begin: Rc::new(|_| {}),
            connect: Rc::new(|_, _| true),
            shutdown_client: Rc::new(|_, _| {}),
        }
    }

    fn settings(&self) -> ClientAdmissionSettings {
        self.settings.borrow().clone()
    }

    fn get_userinfo(&self, client_num: usize) -> String {
        self.userinfo.borrow().get(&client_num).cloned().unwrap_or_default()
    }

    fn set_configstring(&self, index: i32, value: &str) {
        self.configstrings.borrow_mut().insert(index, value.to_string());
    }

    fn send_server_command(&self, client_num: i32, value: &str) {
        self.commands_log.borrow_mut().push((client_num, value.to_string()));
    }

    fn log(&self, value: &str) {
        self.logs.borrow_mut().push(value.to_string());
    }

    fn filter_packet(&self, _address: &str) -> bool {
        self.banned.get()
    }
}

fn admission_fixture() -> (ClientAdmissionRuntime, Rc<StubAdmissionHost>, Rc<StubSessionCvars>) {
    let time = Rc::new(Cell::new(0));
    let log = Rc::new(RefCell::new(Vec::new()));
    let pool = test_pool(Product::Baseq3, 4, time, log);
    let world = Rc::new(SessionWorld {
        clients: pool.clients()[..4].to_vec(),
        max_clients: 4,
        team_scores: Rc::new(SlotArray::new(4)),
        game_type: Cell::new(GameType::GtFfa as i32),
        team_auto_join: Cell::new(false),
        max_game_clients: Cell::new(0),
        time: Cell::new(0),
        num_non_spectator_clients: Cell::new(0),
        new_session: Cell::new(false),
    });
    let cvars = Rc::new(StubSessionCvars {
        values: RefCell::new(HashMap::new()),
    });
    let session = Rc::new(GameSessionManager::new(
        world,
        Rc::new(StubSessionServices {
            prints: RefCell::new(Vec::new()),
            teams: RefCell::new(Vec::new()),
        }),
        cvars.clone(),
    ));
    let host = Rc::new(StubAdmissionHost {
        product: Product::Baseq3,
        pool: pool.clone(),
        team_scores: Rc::new(SlotArray::new(4)),
        world: Rc::new(StubWorld::new()),
        match_state: Rc::new(RefCell::new(MatchState::default())),
        new_session: Cell::new(true),
        session,
        spawns: RefCell::new(Vec::new()),
        death: Rc::new(StubDeath {
            dies: RefCell::new(Vec::new()),
            tossed: RefCell::new(Vec::new()),
            tossed_pp: RefCell::new(Vec::new()),
            tossed_cubes: RefCell::new(Vec::new()),
        }),
        ranks: Cell::new(0),
        commands: Rc::new(StubAdmissionCommands {
            teams: RefCell::new(Vec::new()),
            stops: RefCell::new(Vec::new()),
        }),
        settings: RefCell::new(ClientAdmissionSettings {
            game_type: GameType::GtFfa as i32,
            password: String::new(),
        }),
        userinfo: RefCell::new(HashMap::new()),
        configstrings: RefCell::new(HashMap::new()),
        commands_log: RefCell::new(Vec::new()),
        logs: RefCell::new(Vec::new()),
        banned: Cell::new(false),
    });
    (ClientAdmissionRuntime::new(host.clone()), host, cvars)
}

#[test]
fn admission_connect_begin_disconnect() {
    let (runtime, host, cvars) = admission_fixture();
    host.userinfo.borrow_mut().insert(0, "\\name\\bob".to_string());
    host.banned.set(true);
    assert_eq!(
        runtime.connect(0, true, false),
        Some("You are banned from this server.".to_string())
    );
    host.banned.set(false);
    host.settings.borrow_mut().password = "pw".to_string();
    assert_eq!(runtime.connect(0, true, false), Some("Invalid password".to_string()));
    host.settings.borrow_mut().password = String::new();

    assert_eq!(runtime.connect(0, true, false), None);
    assert_eq!(
        host.pool.client_at(0).borrow().pers.connected,
        ConnectionState::Connecting as i32
    );
    assert_eq!(host.pool.client_at(0).borrow().sess.session_team, Team::TeamFree as i32);
    assert!(cvars.values.borrow().contains_key("session0"));
    assert!(host.logs.borrow().iter().any(|line| line.contains("ClientConnect")));

    runtime.begin(0);
    assert_eq!(
        host.pool.client_at(0).borrow().pers.connected,
        ConnectionState::Connected as i32
    );
    assert_eq!(host.spawns.borrow().as_slice(), [0]);
    assert!(host.logs.borrow().iter().any(|line| line.contains("ClientBegin")));

    host.pool.at(0).borrow_mut().inuse = true;
    host.pool.client_at(0).borrow_mut().ps.set_health(100);
    runtime.disconnect(0);
    assert_eq!(
        host.pool.client_at(0).borrow().pers.connected,
        ConnectionState::Disconnected as i32
    );
    assert_eq!(host.configstrings.borrow().get(&544), Some(&String::new()));
    assert!(host.logs.borrow().iter().any(|line| line.contains("ClientDisconnect")));
    assert_eq!(host.death.tossed.borrow().as_slice(), [0]);
}

struct StubConfig {
    indexes: RefCell<Vec<String>>,
}

impl ConfigStrings for StubConfig {
    fn model_index(&self, name: &str) -> i32 {
        self.indexes.borrow_mut().push(name.to_string());
        7
    }
}

struct StubArenaHost {
    match_runtime: Rc<MatchRuntime>,
    world: Rc<StubWorld>,
    cvars: Rc<StubCvars>,
    config: Rc<StubConfig>,
}

impl ArenaHost for StubArenaHost {
    fn match_runtime(&self) -> Rc<MatchRuntime> {
        self.match_runtime.clone()
    }

    fn world(&self) -> WorldRef {
        self.world.clone()
    }

    fn cvars(&self) -> Rc<dyn CvarRegistry> {
        self.cvars.clone()
    }

    fn config(&self) -> Rc<dyn ConfigStrings> {
        self.config.clone()
    }
}

#[test]
fn arena_postgame_podium_abort() {
    let (match_runtime, match_host) = match_fixture(GameType::GtFfa as i32);
    connect_client(&match_host, 0, Team::TeamFree as i32, 5);
    connect_client(&match_host, 1, Team::TeamFree as i32, 9);
    match_host.pool.at(1).borrow_mut().r.sv_flags |= ServerEntityFlags::Bot as i32;
    match_runtime.calculate_ranks();
    let host = Rc::new(StubArenaHost {
        match_runtime: match_runtime.clone(),
        world: Rc::new(StubWorld::new()),
        cvars: Rc::new(StubCvars {
            values: RefCell::new(HashMap::from([
                ("g_podiumDist".to_string(), ("80".to_string(), 80)),
                ("g_podiumDrop".to_string(), ("70".to_string(), 70)),
            ])),
            sets: RefCell::new(Vec::new()),
        }),
        config: Rc::new(StubConfig {
            indexes: RefCell::new(Vec::new()),
        }),
    });
    let runtime = ArenaRuntime::new(host.clone());
    runtime.update_tournament_info();
    assert!(match_host.log.borrow().iter().any(|line| line.contains("postgame 2 0")));
    runtime.spawn_models_on_victory_pads();
    assert!(host.config.indexes.borrow().iter().any(|name| name.contains("podium4")));
    let saved = runtime.capture_save_state();
    runtime.restore_save_state(&saved);
    let before = host.world.links.borrow().len();
    runtime.abort_podium();
    assert_eq!(host.world.links.borrow().len(), before);
    match_host.settings.borrow_mut().game_type = GameType::GtSinglePlayer as i32;
    runtime.abort_podium();
}
