//! Weapon, item, goal, chat, senses, and travel selection tests.

#[path = "common/behavior.rs"]
mod fixtures;

use fixtures::{
    behavior_files, TEST_CHARACTER, TEST_CHAT_FILE, TEST_ITEM_CONFIG, TEST_WEAPON_CONFIG, TEST_WEIGHT_CONFIG,
};
use qa_bots::behavior::library::character::{characteristic_index, BotCharacterLibrary};
use qa_bots::behavior::library::chat::BotChatLibrary;
use qa_bots::behavior::library::chat_data::ChatData;
use qa_bots::behavior::library::genetic::Xorshift32;
use qa_bots::behavior::library::goals::{touching_goal, BotGoal, BotGoalLibrary, GoalFlags};
use qa_bots::behavior::library::structure::{read_structure_definitions, StructureReader};
use qa_bots::behavior::library::weapons::{WeaponAi, WeaponConfig, WeaponLoadResult};
use qa_bots::behavior::library::weights::{WeightConfig, WeightConfigStore};
use qa_bots::behavior::q3::ai_definitions::BotInventory;
use qa_bots::behavior::q3::movement_state::{BotMoveResult, BotMoveStateStore};
use qa_bots::behavior::q3::travel::controller::{move_to_goal, TravelOutcome, TravelStep};
use qa_bots::behavior::q3::travel::ground::GroundReachability;
use qa_bots::behavior::q3::travel::special::MoverObservation;
use qa_core::math::vec3;

#[test]
fn structure_reader_parses_item_blocks() {
    let definitions = read_structure_definitions(TEST_ITEM_CONFIG).unwrap();
    assert_eq!(definitions.len(), 2);
    assert_eq!(definitions[0].type_name.as_deref(), Some("iteminfo"));
    assert_eq!(definitions[0].string("classname"), Some("weapon_rocketlauncher"));
    assert_eq!(definitions[0].number("respawntime"), Some(30.0));
    let mut reader = StructureReader::new();
    assert_eq!(reader.load(TEST_ITEM_CONFIG).unwrap(), 2);
    assert_eq!(reader.definitions_of_type("iteminfo").len(), 2);
}

#[test]
fn character_interpolates_skills() {
    let files = behavior_files();
    let mut library = BotCharacterLibrary::new();
    let mid = library.load_character_skill(&files, "botfiles/bots.c", 3.0).unwrap();
    let aggression = characteristic_index("aggression").unwrap();
    let value = library.float(mid, aggression).unwrap();
    assert!((value - 0.6).abs() < 0.01, "mid skill interpolates, got {value}");
    assert_eq!(
        library.string(mid, characteristic_index("name").unwrap()).unwrap(),
        "Grunt"
    );
    assert_eq!(library.bounded_float(mid, aggression, 0.0, 0.5), 0.5);
    library.free_character(mid);
}

#[test]
fn fuzzy_weights_score_by_inventory() {
    let config = WeightConfig::parse("botfiles/fw.c", TEST_WEIGHT_CONFIG).unwrap();
    assert_eq!(config.names(), vec!["health"]);
    let mut rng = Xorshift32::new(7);
    let low = [0i32; 64];
    let mut high = [0i32; 64];
    high[BotInventory::HEALTH] = 100;
    // value 0 < threshold 50 branch weight... evaluate both ends.
    let empty: [i32; 64] = [0; 64];
    let w_empty = config.fuzzy_weight(&empty.as_slice(), "health", &mut rng);
    let w_full = config.fuzzy_weight(&high.as_slice(), "health", &mut rng);
    assert!(w_empty > w_full, "low health must weigh more ({w_empty} vs {w_full})");
    let _ = low;
}

#[test]
fn weight_store_caches_configs() {
    let files = behavior_files();
    let mut store = WeightConfigStore::new(&files);
    assert!(store.read_config("botfiles/fw.c").is_ok());
    assert_eq!(store.len(), 1);
    store.free_config("botfiles/fw.c");
    assert!(store.is_empty());
}

#[test]
fn weapon_config_parses_and_selects() {
    let config = WeaponConfig::parse("botfiles/weapons.c", TEST_WEAPON_CONFIG).unwrap();
    assert_eq!(config.weapons.len(), 2);
    assert_eq!(config.projectiles.len(), 2);
    let rocket = config.weapon_info(5).unwrap();
    assert_eq!(rocket.projectile_info.damage, 100.0);

    let files = behavior_files();
    let mut ai = WeaponAi::new(&files);
    assert_eq!(ai.load_weapons("botfiles/weapons.c"), WeaponLoadResult::NoError);
    let handle = ai.alloc_state().unwrap();
    ai.set_owned(handle, 5, true);
    ai.set_owned(handle, 2, true);
    let mut inventory = [0i32; 64];
    inventory[23] = 10;
    inventory[19] = 100;
    let mut rng = Xorshift32::new(11);
    // Mid-range favors the rocket launcher's splash DPS.
    assert_eq!(ai.choose_best_weapon(handle, &inventory, 600.0, &mut rng), 5);
    // Without rockets, the machinegun wins.
    inventory[23] = 0;
    assert_eq!(ai.choose_best_weapon(handle, &inventory, 600.0, &mut rng), 2);
    ai.free_state(handle);
}

#[test]
fn goal_stack_and_avoid_goals() {
    let files = behavior_files();
    let mut library = BotGoalLibrary::new(&files);
    assert_eq!(library.load_item_config("botfiles/items.c"), 0);
    let handle = library.alloc_goal_state().unwrap();
    let goal = BotGoal {
        origin: vec3(100.0, 0.0, 0.0),
        area: 2,
        flags: GoalFlags::ITEM,
        ..BotGoal::default()
    };
    library.push_goal(handle, goal);
    library.push_goal(handle, BotGoal::default());
    assert_eq!(library.stack_depth(handle), 2);
    assert_eq!(library.top_goal(handle).unwrap().area, 0);
    assert_eq!(library.second_goal(handle).unwrap().area, 2);
    library.pop_goal(handle);
    assert_eq!(library.top_goal(handle).unwrap().area, 2);
    library.add_avoid_goal(handle, 9, 10.0);
    assert!(library.is_avoided(handle, 9, 5.0));
    assert!(!library.is_avoided(handle, 9, 11.0));
    library.remove_avoid_goal(handle, 9);
    assert!(!library.is_avoided(handle, 9, 5.0));
    library.empty_goal_stack(handle);
    assert_eq!(library.stack_depth(handle), 0);
}

#[test]
fn ltg_choice_skips_avoided_and_timed_out_items() {
    let files = behavior_files();
    let mut library = BotGoalLibrary::new(&files);
    library.load_item_config("botfiles/items.c");
    let handle = library.alloc_goal_state().unwrap();
    let first = library.add_level_item(0, 10, vec3(100.0, 0.0, 0.0), 2, 0);
    let second = library.add_level_item(1, 11, vec3(200.0, 0.0, 0.0), 3, 0);
    let weights = |_: &qa_bots::behavior::library::goals::LevelItem| 50.0;
    let travel = |from: i32, to: i32| if from == to { 1 } else { 100 };
    let picked = library
        .choose_ltg_item(handle, vec3(0.0, 0.0, 0.0), &weights, &travel, 1, 0.0)
        .unwrap();
    assert!(picked.number == first || picked.number == second);
    library.add_avoid_goal(handle, first, 100.0);
    library.add_avoid_goal(handle, second, 100.0);
    assert!(library
        .choose_ltg_item(handle, vec3(0.0, 0.0, 0.0), &weights, &travel, 1, 0.0)
        .is_none());
}

#[test]
fn touching_goal_uses_interaction_radius() {
    let goal = BotGoal {
        origin: vec3(0.0, 0.0, 0.0),
        ..BotGoal::default()
    };
    assert!(touching_goal(vec3(0.0, 0.0, 0.0), &goal));
    assert!(!touching_goal(vec3(100.0, 0.0, 0.0), &goal));
}

#[test]
fn chat_matches_and_replies() {
    let data = ChatData::parse(TEST_CHAT_FILE).unwrap();
    assert_eq!(data.counts(), (0, 1, 1, 1));
    let files = behavior_files();
    let mut library = BotChatLibrary::new(&files);
    library.load_chat_file("botfiles/chat.c").unwrap();
    let handle = library.alloc_chat_state().unwrap();
    let matched = library.chat_test("hello world").unwrap();
    assert_eq!(matched.variables[0].as_deref(), Some("world"));
    let mut rng = Xorshift32::new(3);
    assert!(library.reply_chat(
        handle,
        "hello world",
        qa_bots::behavior::library::chat::ChatDestination::All,
        &matched.variables,
        1.0,
        &mut rng
    ));
    assert_eq!(library.outgoing(handle).len(), 1);
    assert!(library.initial_chat(
        handle,
        "kill",
        &[],
        qa_bots::behavior::library::chat::ChatDestination::All,
        2.0,
        &mut rng
    ));
    assert_eq!(library.num_initial_chats("kill"), 1);
    assert!(library.chat_length(handle, "hello") > 0.0);
}

#[test]
fn travel_controller_arrives_and_blocks() {
    let mut states = BotMoveStateStore::new();
    let handle = states.alloc().unwrap();
    let goal = BotGoal {
        origin: vec3(100.0, 0.0, 0.0),
        area: 2,
        ..BotGoal::default()
    };
    let mover = MoverObservation {
        model: None,
        on_mover: false,
        mover_down: false,
    };
    let mut result = BotMoveResult::default();
    // Straight steering fallback with no routed step.
    let outcome = move_to_goal(
        states.get_mut(handle).unwrap(),
        &goal,
        None,
        &mover,
        || 0,
        &[],
        0.0,
        &mut result,
    );
    assert_eq!(
        outcome,
        qa_bots::behavior::q3::travel::controller::TravelOutcome::Moving
    );
    assert!(result.move_direction.x > 0.9);
    // Routed walk step.
    let step = TravelStep {
        travel_type: qa_bots::behavior::TravelType::WALK,
        start: vec3(0.0, 0.0, 0.0),
        end: vec3(100.0, 0.0, 0.0),
        number: 4,
        entity: 0,
        travel_time: 100,
    };
    let outcome = move_to_goal(
        states.get_mut(handle).unwrap(),
        &goal,
        Some(step),
        &mover,
        || 0,
        &[],
        0.0,
        &mut result,
    );
    assert_eq!(
        outcome,
        qa_bots::behavior::q3::travel::controller::TravelOutcome::Moving
    );
    // Blocked reachability.
    states.get_mut(handle).unwrap().avoid_reach[0] = 4;
    let outcome = move_to_goal(
        states.get_mut(handle).unwrap(),
        &goal,
        Some(step),
        &mover,
        || 0,
        &[],
        0.0,
        &mut result,
    );
    assert_eq!(
        outcome,
        qa_bots::behavior::q3::travel::controller::TravelOutcome::Blocked
    );
    // Arrival at the goal.
    states.get_mut(handle).unwrap().origin = vec3(100.0, 0.0, 0.0);
    states.get_mut(handle).unwrap().avoid_reach[0] = 0;
    let outcome = move_to_goal(
        states.get_mut(handle).unwrap(),
        &goal,
        Some(step),
        &mover,
        || 0,
        &[],
        0.0,
        &mut result,
    );
    assert_eq!(
        outcome,
        qa_bots::behavior::q3::travel::controller::TravelOutcome::Arrived
    );
}

#[test]
fn ground_followers_set_travel_types() {
    use qa_bots::behavior::q3::movement_state::BotInitMove;
    use qa_bots::behavior::q3::travel::ground::travel_ground;

    let mut states = BotMoveStateStore::new();
    let handle = states.alloc().unwrap();
    states.init(
        handle,
        &BotInitMove {
            origin: vec3(0.0, 0.0, 0.0),
            velocity: vec3(0.0, 0.0, 0.0),
            view_offset: vec3(0.0, 0.0, 0.0),
            entity_num: 0,
            client: 0,
            think_time: 0.1,
            presence_type: 0,
            view_angles: vec3(0.0, 0.0, 0.0),
            or_move_flags: 0,
        },
    );
    let reach = GroundReachability {
        travel_type: qa_bots::behavior::TravelType::LADDER,
        start: vec3(0.0, 0.0, 0.0),
        end: vec3(0.0, 0.0, 100.0),
        travel_time: 50,
        entity: 0,
    };
    let mut result = BotMoveResult::default();
    assert!(travel_ground(states.get(handle).unwrap(), &reach, &mut result));
    assert_eq!(result.travel_type, qa_bots::behavior::TravelType::LADDER);
    assert!(result.move_direction.z > 0.9);
}

#[test]
fn character_file_parses() {
    let _ = TEST_CHARACTER;
    let _ = TEST_CHAT_FILE;
}
