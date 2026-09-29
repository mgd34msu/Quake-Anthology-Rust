//! Q3 game AI integration tests: setup, frames, orders, chat, catalog.

#[path = "common/behavior.rs"]
mod fixtures;

use fixtures::{behavior_files, FakeNav};
use qa_bots::behavior::library::genetic::{BotRandom, Xorshift32};
use qa_bots::behavior::q3::ai_chat::{bot_chat_kill, bot_initial_chat};
use qa_bots::behavior::q3::ai_command::{bot_match_message, ConsoleMessageQueue};
use qa_bots::behavior::q3::ai_context::GameAiContext;
use qa_bots::behavior::q3::ai_definitions::{BotLongTermGoal, BotMessage};
use qa_bots::behavior::q3::ai_input::{bot_angle_difference, bot_change_view_angle};
use qa_bots::behavior::q3::ai_main::GameAi;
use qa_bots::behavior::q3::ai_orders::{bot_order_ltg, bot_teammates, client_name, same_team};
use qa_bots::behavior::q3::ai_state::{AiNode, BotSettings};
use qa_bots::behavior::q3::ai_team::{
    bot_client_from_name, bot_elect_team_leader, bot_is_team_leader, bot_set_team_mate_task_preference,
};
use qa_bots::behavior::q3::ai_voice::bot_voice_chat_command;
use qa_bots::behavior::q3::catalog::{GameBotCatalog, GameBotCatalogHost};
use qa_bots::behavior::q3::game_host::{BotObservedEntity, BotObservedPlayer, BotProduct};
use qa_bots::behavior::q3::info::{info_set_value_for_key, info_value_for_key};
use qa_bots::behavior::q3::library::BotLibrary;
use qa_bots::behavior::q3::navigation_types::TravelType;
use qa_bots::behavior::q3::source_game::ScriptedBotGame;
use qa_core::math::vec3;

fn named_player(name: &str, team: i32, health: i32) -> BotObservedEntity {
    let mut entity = BotObservedEntity::absent();
    entity.generation = 1;
    entity.present = true;
    entity.linked = true;
    entity.origin = vec3(0.0, 0.0, 0.0);
    entity.player = Some(BotObservedPlayer {
        health,
        connected: true,
        team,
        name: name.to_owned(),
        last_hurt_client: -1,
        last_hurt_mod: 0,
        weapon: 2,
        powerups: [0; 16],
    });
    entity
}

fn scripted_game() -> ScriptedBotGame {
    let mut game = ScriptedBotGame::new(BotProduct::BaseQ3, 4, 4);
    game.set_entity(0, named_player("Grunt", 1, 100));
    game.set_entity(1, named_player("Major", 1, 100));
    game.set_entity(2, named_player("Enemy", 2, 100));
    game
}

#[test]
fn game_ai_thinks_and_commands() {
    let files = behavior_files();
    let mut game = scripted_game();
    let mut nav = FakeNav::new();
    let mut ai = GameAi::new(&files, 4, false);
    ai.setup(false).unwrap();
    ai.load_map("botfiles/items.c", 0, false).unwrap();
    ai.setup_client(
        0,
        BotSettings {
            characterfile: "botfiles/bots.c".to_owned(),
            skill: 3.0,
            team: String::new(),
        },
        &files,
    )
    .unwrap();
    let mut rng = Xorshift32::new(42);
    for tick in 1..=5 {
        ai.start_frame(&mut game, &mut nav, tick * 100, &mut rng, || 0.5)
            .unwrap();
    }
    let commands = ai.drain_commands();
    assert!(!commands.is_empty());
    assert_eq!(commands[0].0, 0);
    // Node switches are tracked every frame.
    assert!(!ai.context.node_switches.is_empty());
    ai.shutdown_client(0);
    assert_eq!(ai.context.num_bots, 0);
}

#[test]
fn respawn_presses_attack_until_alive() {
    let files = behavior_files();
    let mut game = scripted_game();
    game.set_entity(0, named_player("Grunt", 1, 0));
    let mut nav = FakeNav::new();
    let mut ai = GameAi::new(&files, 4, false);
    ai.setup(false).unwrap();
    ai.load_map("botfiles/items.c", 0, false).unwrap();
    ai.setup_client(
        0,
        BotSettings {
            characterfile: "botfiles/bots.c".to_owned(),
            skill: 2.0,
            team: String::new(),
        },
        &files,
    )
    .unwrap();
    let mut rng = Xorshift32::new(1);
    ai.start_frame(&mut game, &mut nav, 100, &mut rng, || 0.5).unwrap();
    let node = ai.context.states.get(0).unwrap().ai_node;
    assert_eq!(node, Some(AiNode::Respawn));
    let commands = ai.drain_commands();
    assert_ne!(commands[0].1.buttons & 1, 0);
}

#[test]
fn team_queries_and_orders() {
    let game = scripted_game();
    let mut context = GameAiContext::new(4);
    for client in 0..3 {
        let state = context.states.acquire(client);
        state.inuse = true;
        state.team_leader = String::new();
    }
    assert_eq!(client_name(&game, 0), "Grunt");
    assert!(same_team(&game, 0, 1));
    assert!(!same_team(&game, 0, 2));
    let state = context.states.get(0).unwrap();
    assert_eq!(bot_teammates(&game, state), vec![1]);
    assert_eq!(bot_client_from_name(&game, &context, "major"), 1);
    bot_elect_team_leader(&game, &mut context, 1);
    assert_eq!(context.states.get(1).unwrap().team_leader, "Grunt");
    bot_elect_team_leader(&game, &mut context, 0);
    assert!(bot_is_team_leader(&context, &game, 0));
    assert!(!bot_is_team_leader(&context, &game, 1));
    bot_order_ltg(&mut context, 1, 0, BotLongTermGoal::DefendKeyArea, 600.0);
    let ordered = context.states.get(1).unwrap();
    assert!(ordered.ordered);
    assert_eq!(ordered.last_goal_ltg_type, BotLongTermGoal::DefendKeyArea as i32);
    assert!(bot_voice_chat_command(&mut context, &game, 1, 0, "followme"));
    assert!(!bot_voice_chat_command(&mut context, &game, 1, 0, "nonsense"));
    bot_set_team_mate_task_preference(&mut context, "Major", 2);
}

#[test]
fn console_messages_dispatch_orders() {
    let game = scripted_game();
    let mut context = GameAiContext::new(4);
    let state = context.states.acquire(1);
    state.inuse = true;
    let mut queue = ConsoleMessageQueue::new();
    queue.queue(qa_bots::behavior::q3::ai_command::QueuedConsoleMessage {
        client: 1,
        sender: 0,
        addressee: 1,
        message: "defend the base".to_owned(),
        time: 0.0,
    });
    assert_eq!(queue.count_for(1), 1);
    assert_eq!(bot_match_message("defend the base"), Some(BotMessage::DEFENDKEYAREA));
    let mut rng = Xorshift32::new(9);
    qa_bots::behavior::q3::ai_command::bot_process_console_messages(&mut context, &game, &mut queue, 1, &mut rng);
    assert_eq!(
        context.states.get(1).unwrap().ltg_type,
        BotLongTermGoal::DefendKeyArea as i32
    );
}

#[test]
fn chat_helpers_gate_on_chattiness() {
    let files = behavior_files();
    let mut library = BotLibrary::new(&files, 4, false);
    library.setup().unwrap();
    library.chat.load_chat_file("botfiles/chat.c").unwrap();
    let mut context = GameAiContext::new(4);
    let state = context.states.acquire(0);
    state.inuse = true;
    state.cs = library.chat.alloc_chat_state().unwrap();
    state.character = library
        .characters
        .load_character_skill(&files, "botfiles/bots.c", 3.0)
        .unwrap();
    let mut rng = Xorshift32::new(5);
    // Missing chattiness characteristics default to 0: no chat.
    assert!(!bot_initial_chat(
        &mut context,
        &mut library.chat,
        &library.characters,
        0,
        "random",
        34,
        &mut rng
    ));
    assert!(!bot_chat_kill(
        &mut context,
        &mut library.chat,
        &library.characters,
        0,
        "Major",
        &mut rng
    ));
    let _ = rng.next_unit();
}

#[test]
fn view_angle_math_wraps() {
    assert!((bot_change_view_angle(350.0, 10.0, 5.0) - 355.0).abs() < 0.01);
    assert!((bot_angle_difference(10.0, 350.0) - 20.0).abs() < 0.01);
    assert_eq!(TravelType::MASK, 0x00ff_ffff);
}

#[test]
fn info_strings_roundtrip() {
    let info = info_set_value_for_key("", "name", "Grunt");
    assert_eq!(info_value_for_key(&info, "name"), "Grunt");
    let info = info_set_value_for_key(&info, "name", "Major");
    assert_eq!(info_value_for_key(&info, "name"), "Major");
    assert_eq!(info_value_for_key(&info, "missing"), "");
}

struct CatalogHost {
    clients: Vec<i32>,
    time: i32,
    printed: Vec<String>,
}

impl GameBotCatalogHost for CatalogHost {
    fn allocate_client(&mut self) -> Option<i32> {
        let client = self.clients.len() as i32;
        self.clients.push(client);
        Some(client)
    }

    fn setup_client(&mut self, _client: i32, _settings: BotSettings, _restart: bool) -> bool {
        true
    }

    fn shutdown_client(&mut self, _client: i32, _restart: bool) {}

    fn time_ms(&self) -> i32 {
        self.time
    }

    fn print(&mut self, text: &str) {
        self.printed.push(text.to_owned());
    }

    fn name_in_use(&self, _name: &str) -> bool {
        false
    }

    fn team_counts(&self) -> Vec<(i32, i32, i32)> {
        vec![(1, 1, 1)]
    }
}

#[test]
fn catalog_adds_and_spawns_bots() {
    let files = behavior_files();
    let mut catalog = GameBotCatalog::new();
    catalog.load(&files);
    assert_eq!(catalog.num_bots(), 2);
    assert!(catalog.bot_info_by_name("grunt").is_some());
    let mut host = CatalogHost {
        clients: Vec::new(),
        time: 0,
        printed: Vec::new(),
    };
    assert!(catalog.console_command(
        &mut host,
        &[
            "addbot".to_owned(),
            "Grunt".to_owned(),
            "3".to_owned(),
            "red".to_owned()
        ]
    ));
    assert_eq!(catalog.queued(), 1);
    host.time = 1000;
    assert_eq!(catalog.check_spawn(&mut host), vec![0]);
    catalog.check_minimum_players(&mut host, 0);
    assert_eq!(catalog.count_human_players(&host, -1), 1);
    assert_eq!(catalog.count_bot_players(&host, -1), 1);
}
